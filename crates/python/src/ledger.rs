//! One public ledger per native decision session, including automatic combat.
use arkguard_engine::{
    history::DecisionHistory, ledger_reward::NativeProgressCore, progress::PublicProgress,
    solo::session::Session,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

pub struct DecisionLedger {
    pub state: Arc<Value>,
    history: DecisionHistory,
    pub policy_history: Value,
    pub blocked: BTreeSet<String>,
    progress: PublicProgress,
    reward: NativeProgressCore,
    feedback: Value,
}

impl DecisionLedger {
    pub fn new(state: Arc<Value>, stream_id: String) -> Result<Self, String> {
        let mut progress = PublicProgress::new(stream_id)?;
        progress.observe(Arc::clone(&state), None, None)?;
        let mut history = DecisionHistory::new();
        history.initialize(Arc::clone(&state), &progress.snapshot(&state)?)?;
        let mut reward = NativeProgressCore::new();
        reward.reset(&state)?;
        Ok(Self {
            state,
            history,
            policy_history: Value::Null,
            blocked: BTreeSet::new(),
            progress,
            reward,
            feedback: Value::Null,
        })
    }

    pub fn snapshot(
        &mut self,
        execution: Option<Value>,
        reward: Option<Value>,
        advances: usize,
    ) -> Result<String, String> {
        self.policy_history = self.history.snapshot(&self.state)?;
        self.blocked = self.history.blocked(&self.state)?;
        #[derive(serde::Serialize)]
        struct Snapshot<'a> {
            observation: &'a Value,
            feedback: &'a Value,
            history: &'a Value,
            blocked: &'a BTreeSet<String>,
            execution: Option<Value>,
            reward: Option<Value>,
            advances: usize,
        }
        serde_json::to_string(&Snapshot {
            observation: &self.state,
            feedback: &self.feedback,
            history: &self.policy_history,
            blocked: &self.blocked,
            execution,
            reward,
            advances,
        })
        .map_err(|error| error.to_string())
    }

    pub fn step(
        &mut self,
        session: &mut Session,
        action: &Value,
        tick_budget: usize,
        max_advances: usize,
    ) -> Result<String, String> {
        let before = Arc::clone(&self.state);
        // Preserve the immediate receipt before advancing the resulting battle.
        let mut transport = session.step(action, 0)?;
        let mut receipts = vec![transport["feedback"].clone()];
        let accepted = transport["accepted"]
            .as_bool()
            .ok_or("Missing action acceptance")?;
        let mut reply = json!({"accepted": accepted, "result": {"ok": accepted}});
        if !accepted {
            reply["result"]["error"] = transport["rejection"].clone();
        }
        let immediate = Arc::new(session.observation()?);
        let execution = self
            .progress
            .observe(immediate.clone(), Some(action), Some(&reply))?
            .ok_or("Missing execution report")?;
        let mut after = immediate;
        let mut advances = 0;
        while transport["budgetExhausted"] == true {
            if advances >= max_advances {
                return Err("Combat budget exhausted before a decision boundary".into());
            }
            transport = session.advance(tick_budget)?;
            receipts.push(transport["feedback"].clone());
            advances += 1;
            if transport["budgetExhausted"] != true {
                after = Arc::new(session.observation()?);
                self.progress.observe(after.clone(), None, None)?;
            }
        }
        let progress = self.progress.snapshot(&after)?;
        self.history.record(
            before.clone(),
            after.clone(),
            action,
            &execution,
            &progress,
            accepted,
        )?;
        let reward = self.reward.step(&before, &after)?;
        self.feedback = json!({"schema": "arkguard.decision-feedback.v1",
            "decisionRevision": before["revision"], "resultRevision": after["revision"],
            "receipts": receipts});
        self.state = after;
        self.snapshot(Some(execution), Some(reward), advances)
    }
}

/// One bounded current domain. The binding keeps its token counter across resets.
pub struct PolicyDomain {
    pub revision: u64,
    pub token: u64,
    pub actions: Vec<Value>,
}
impl PolicyDomain {
    pub fn selected(&self, revision: u64, token: u64, index: usize) -> Result<Value, String> {
        if self.revision != revision || self.token != token {
            return Err("Stale native candidate domain".into());
        }
        self.actions
            .get(index)
            .cloned()
            .ok_or_else(|| "Select a current unpadded candidate index".into())
    }
}
