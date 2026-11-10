//! Native decision history: the `arkguardrl.candidates.DecisionHistory` ledger.
//!
//! The history is a view of the public decision context, not a memory of hidden
//! state. It binds the exact public progress stream, groups attempted intents by
//! `decision_id`, masks rejected intents only at the decision that rejected them,
//! and counts PREP attempts so the candidate builder can stop offering prep
//! actions after the native 64-action recovery limit.
//!
//! Python's `_sync` mutates the ledger: a round/phase change clears both the
//! attempt groups and `prep_used`. `blocked`/`snapshot` therefore take `&mut self`
//! here, exactly like the Python methods they replace; taking `&self` would keep
//! attempts from an older context alive after a newer snapshot and diverge on a
//! later record at that older context. `blocked` returns the set of rejected
//! `action_id`s (the Python method returns a `frozenset`) because a boolean can
//! not express which of several candidate intents is masked.

use crate::identity::{action_id, decision_id, fingerprint};
use crate::progress::{EXECUTION_SCHEMA, PROGRESS_SCHEMA};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const HISTORY_SCHEMA: &str = "arkguardrl/native-context-attempts-feedback-intent-prep64-v4";
pub const PREP_ACTION_LIMIT: u64 = 64;

type Result<T> = std::result::Result<T, String>;

fn missing(message: &str) -> String {
    message.to_string()
}

/// JSON integers only: Python's `type(value) is int` excludes booleans and floats.
fn integer(value: &Value) -> Option<i128> {
    match value {
        Value::Number(number) if !number.is_f64() => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        _ => None,
    }
}

/// The numeric value Python's `<`/`<=` comparisons accept for receipt sequences.
fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct DecisionHistory {
    /// Last observed `(round, phase)`; groups are discarded when this changes.
    context: Option<(Value, Value)>,
    /// Attempted `action_id`s by `decision_id` of the public decision context.
    attempts: BTreeMap<String, BTreeSet<String>>,
    prep_used: u64,
    progress: Option<Value>,
    state: Option<Arc<Value>>,
    cached_decision: Option<(Arc<Value>, String)>,
}

impl DecisionHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the history to the real reset observation and its public progress.
    ///
    /// Python also rejects a step result here through `result.accepted is not
    /// None`. The state/progress signature cannot see that flag, but every real
    /// step progress carries `last_execution`, which is checked instead.
    pub fn initialize(&mut self, result_state: Arc<Value>, progress: &Value) -> Result<()> {
        let initialized = self.progress.is_some()
            || progress
                .get("last_execution")
                .is_some_and(|value| !value.is_null());
        if initialized {
            return Err("Initialize history exactly once from a real reset result".into());
        }
        self.observe(result_state, progress)
    }

    /// Bind the public progress stream to this exact observation.
    ///
    /// A failure leaves the ledger unchanged, so a rejected receipt can be
    /// reported without corrupting the stream.
    fn observe(&mut self, state: Arc<Value>, progress: &Value) -> Result<()> {
        let map = progress.as_object().ok_or_else(|| {
            missing("Exact public progress stream required; legacy history is incompatible")
        })?;
        let observation_id = self.observation_id(&state)?;
        if map.get("schema").and_then(Value::as_str) != Some(PROGRESS_SCHEMA)
            || map.get("observation_id").and_then(Value::as_str) != Some(&observation_id)
        {
            return Err(
                "Exact public progress stream required; legacy history is incompatible".into(),
            );
        }
        let stream_id = map.get("stream_id").and_then(Value::as_str);
        let sequence = map.get("sequence").and_then(integer);
        let ordered = stream_id.is_some_and(|id| id.chars().count() == 32)
            && sequence.is_some_and(|sequence| sequence >= 1);
        if !ordered {
            return Err("Public progress stream identity/order is missing".into());
        }
        let (stream_id, sequence) = (stream_id.unwrap(), sequence.unwrap());
        if let Some(previous) = self.progress.as_ref() {
            let previous_stream = previous.get("stream_id").and_then(Value::as_str);
            let previous_sequence = previous
                .get("sequence")
                .and_then(integer)
                .ok_or_else(|| missing("Public progress stream identity/order is missing"))?;
            if previous_stream != Some(stream_id) || sequence <= previous_sequence {
                return Err("Public progress crossed streams or repeated an observation".into());
            }
        }
        self.progress = Some(progress.clone());
        self.state = Some(state);
        Ok(())
    }

    fn observation_id(&self, state: &Value) -> Result<String> {
        let bound_id = self
            .progress
            .as_ref()
            .and_then(|progress| progress.get("observation_id"))
            .and_then(Value::as_str);
        match (self.state.as_deref(), bound_id) {
            // Retaining an Arc prevents this exact allocation from being mutated.
            (Some(owned), Some(id)) if std::ptr::eq(owned, state) => Ok(id.to_owned()),
            _ => fingerprint(state),
        }
    }

    /// Record one real attempted step, accepted or rejected, after the fact.
    ///
    /// `before` is the state the intent was chosen at, `after` is
    /// `result.observation`, `receipt` is `result.execution`, `result_progress`
    /// is `result.progress` and `accepted` is `result.accepted`. Python also
    /// compares `result.action_id`, which the transport derives as
    /// `action_id(action)`; recomputing it below is the same identity.
    pub fn record(
        &mut self,
        before: Arc<Value>,
        after: Arc<Value>,
        action: &Value,
        receipt: &Value,
        result_progress: &Value,
        accepted: bool,
    ) -> Result<()> {
        let bound = "History requires the bound real execution receipt";
        let report = receipt.as_object().ok_or_else(|| missing(bound))?;
        if report.get("schema").and_then(Value::as_str) != Some(EXECUTION_SCHEMA) {
            return Err(bound.into());
        }
        // A receipt is boolean about acceptance; `1`/`true` must not alias.
        if report.get("accepted") != Some(&Value::Bool(accepted)) {
            return Err(bound.into());
        }
        let aid = action_id(action)?;
        if report.get("action_id").and_then(Value::as_str) != Some(&aid) {
            return Err(bound.into());
        }
        if report.get("intent") != Some(action) {
            return Err(bound.into());
        }
        let before_id = self.observation_id(&before)?;
        if report.get("before_id").and_then(Value::as_str) != Some(&before_id) {
            return Err(bound.into());
        }
        if !result_progress.is_object() || result_progress.get("last_execution") != Some(receipt) {
            return Err(bound.into());
        }
        let previous = self
            .progress
            .as_ref()
            .ok_or_else(|| missing("History execution stream is disconnected"))?;
        if previous.get("observation_id").and_then(Value::as_str) != Some(&before_id) {
            return Err("History execution stream is disconnected".into());
        }
        let previous_sequence = previous.get("sequence").and_then(integer).ok_or_else(|| {
            missing("Execution receipt does not continue the current public stream")
        })?;
        // Python's `report.get("sequence", 0)` only defaults when the key is absent.
        let receipt_sequence = match report.get("sequence") {
            None => 0.0,
            Some(value) => numeric(value).ok_or_else(|| {
                missing("Execution receipt does not continue the current public stream")
            })?,
        };
        let result_sequence = result_progress
            .get("sequence")
            .and_then(numeric)
            .ok_or_else(|| {
                missing("Execution receipt does not continue the current public stream")
            })?;
        let continuing = previous.get("stream_id").and_then(Value::as_str)
            == report.get("stream_id").and_then(Value::as_str)
            && (previous_sequence as f64) < receipt_sequence
            && receipt_sequence <= result_sequence;
        if !continuing {
            return Err("Execution receipt does not continue the current public stream".into());
        }
        self.observe(after, result_progress)?;
        let key = self.sync(&before)?;
        self.attempts.entry(key).or_default().insert(aid);
        // Only a real PREP action consumes the recovery budget; a mandatory
        // personal choice is not a prep action.
        let prep = before.get("phase").and_then(Value::as_str) == Some("PREP");
        let choice = action.get("t").and_then(Value::as_str) == Some("g.choice");
        if prep && !choice {
            self.prep_used += 1;
        }
        Ok(())
    }

    /// Rejected `action_id`s at this exact public decision context.
    pub fn blocked(&mut self, state: &Value) -> Result<BTreeSet<String>> {
        let key = self.sync(state)?;
        Ok(self.attempts.get(&key).cloned().unwrap_or_default())
    }

    /// The v4 snapshot consumed by feature encoding.
    pub fn snapshot(&mut self, state: &Value) -> Result<Value> {
        let observation_id = self.observation_id(state)?;
        let bound = self
            .progress
            .as_ref()
            .and_then(|progress| progress.get("observation_id"))
            .and_then(Value::as_str)
            == Some(observation_id.as_str());
        if !bound {
            return Err(
                "Initialize/update history with the real environment result before encoding".into(),
            );
        }
        let key = self.sync(state)?;
        let blocked_count = self.attempts.get(&key).map_or(0, BTreeSet::len);
        Ok(json!({
            "schema": HISTORY_SCHEMA,
            "progress": self.progress.clone().unwrap_or(Value::Null),
            "used": self.prep_used,
            "limit": PREP_ACTION_LIMIT,
            "blocked_count": blocked_count,
        }))
    }

    /// Python `_sync`: a new round/phase discards every group and the prep count.
    fn sync(&mut self, state: &Value) -> Result<String> {
        let round = state
            .get("round")
            .cloned()
            .ok_or_else(|| missing("Decision state has no round"))?;
        let phase = state
            .get("phase")
            .cloned()
            .ok_or_else(|| missing("Decision state has no phase"))?;
        let context = (round, phase);
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.attempts.clear();
            self.prep_used = 0;
        }
        if let Some((owned, key)) = &self.cached_decision {
            if std::ptr::eq(owned.as_ref(), state) {
                return Ok(key.clone());
            }
        }
        let key = decision_id(state)?;
        if let Some(owned) = self
            .state
            .as_ref()
            .filter(|owned| std::ptr::eq(owned.as_ref(), state))
        {
            self.cached_decision = Some((Arc::clone(owned), key.clone()));
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initialized() -> (DecisionHistory, Arc<Value>) {
        let state = Arc::new(json!({
            "schema": "test", "round": 1, "phase": "PREP", "public": {}, "own": {},
            "personalEnded": false, "gameEnded": false,
        }));
        let progress = json!({
            "schema": PROGRESS_SCHEMA, "observation_id": fingerprint(&state).unwrap(),
            "stream_id": "0".repeat(32), "sequence": 1,
        });
        let mut history = DecisionHistory::new();
        history.initialize(state.clone(), &progress).unwrap();
        history.snapshot(&state).unwrap();
        (history, state)
    }

    #[test]
    fn cached_identity_rejects_a_changed_copy() {
        let (mut history, mut state) = initialized();
        let original = state.clone();
        let key = decision_id(&original).unwrap();
        history
            .attempts
            .insert(key, BTreeSet::from(["attempt".into()]));
        Arc::make_mut(&mut state)["own"]["funds"] = json!(10);
        assert!(history.snapshot(&state).is_err());
        assert!(history.blocked(&state).unwrap().is_empty());
        assert!(history.blocked(&original).unwrap().contains("attempt"));
        assert!(history.snapshot(&original).is_ok());
    }

    #[test]
    fn cached_decision_still_resets_context() {
        let (mut history, state) = initialized();
        let key = decision_id(&state).unwrap();
        history
            .attempts
            .insert(key, BTreeSet::from(["attempt".into()]));
        let mut next_round = state.as_ref().clone();
        next_round["round"] = json!(2);
        assert!(history.blocked(&next_round).unwrap().is_empty());
        assert!(history.blocked(&state).unwrap().is_empty());
    }
}
