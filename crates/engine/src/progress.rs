//! Public-observation execution receipts and per-round progress (`feedback.py`).
//! No simulator counters or queries are used. Observe the immediate command result
//! with action/reply, then each distinct automatic/final observation without them.
use crate::identity::{action_id, decision_id, fingerprint};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, sync::Arc};

pub const EXECUTION_SCHEMA: &str = "arkguardrl/public-execution-report-v2";
pub const PROGRESS_SCHEMA: &str = "arkguardrl/public-observed-progress-v2";
const ERRORS: &[&str] = &[
    "NO_FUNDS",
    "HAND_FULL",
    "BOARD_FULL",
    "BAD_TILE",
    "BAD_TARGET",
    "SOLD_OUT",
    "MAX_LEVEL",
    "WRONG_PHASE",
    "NOT_YOUR_TURN",
    "ALREADY",
    "TEMP_NOT_EMPTY",
    "ELIMINATED",
    "NOT_READY",
    "BAD_MSG",
    "OTHER",
];
type Result<T> = std::result::Result<T, String>;

// JSON integer and string identities never alias, even if str(uid) is equal.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Uid {
    Integer(i128),
    Text(String),
}
impl Uid {
    fn parse(v: &Value) -> Result<Self> {
        if let Some(s) = v.as_str() {
            return Ok(Self::Text(s.into()));
        }
        Ok(Self::Integer(integer(v, "piece UID")?))
    }
    fn text(&self) -> String {
        match self {
            Self::Integer(n) => n.to_string(),
            Self::Text(s) => s.clone(),
        }
    }
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.as_object()
        .and_then(|m| m.get(key))
        .ok_or_else(|| format!("Missing public field: {key}"))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    field(v, key)?
        .as_array()
        .ok_or_else(|| format!("Expected public array: {key}"))
}
fn integer(v: &Value, name: &str) -> Result<i128> {
    match v {
        Value::Number(n) if !n.is_f64() => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from)),
        _ => None,
    }
    .ok_or_else(|| format!("Expected public integer: {name}"))
}
fn nonnegative(v: &Value, name: &str) -> Result<u64> {
    let n = integer(v, name)?;
    u64::try_from(n).map_err(|_| format!("Missing nonnegative public cumulative counter: {name}"))
}
fn counter(state: &Value, name: &str) -> Result<u64> {
    nonnegative(field(field(field(state, "own")?, "stats")?, name)?, name)
}
fn int_value(n: i128) -> Result<Value> {
    if let Ok(n) = i64::try_from(n) {
        Ok(json!(n))
    } else if let Ok(n) = u64::try_from(n) {
        Ok(json!(n))
    } else {
        Err("Public numeric delta exceeds JSON integer range".into())
    }
}
fn delta(a: &Value, b: &Value) -> Result<Value> {
    if let (Ok(a), Ok(b)) = (integer(a, "delta"), integer(b, "delta")) {
        return int_value(b - a);
    }
    let (a, b) = (
        a.as_f64().ok_or("Expected numeric public delta")?,
        b.as_f64().ok_or("Expected numeric public delta")?,
    );
    let n = b - a;
    if !n.is_finite() {
        return Err("Nonfinite public numeric delta".into());
    }
    Ok(json!(n))
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| "Public progress counter overflow".into())
}

// Keep insertion order: Python's owned() overwrites duplicate identities in place,
// and snapshot stringification may map distinct typed identities to the same key.
fn owned(state: &Value, equipped: bool) -> Result<Vec<(Uid, String, Value)>> {
    let own = field(state, "own")?;
    let mut out: Vec<(Uid, String, Value)> = Vec::new();
    for area in ["board", "hand", "temp"] {
        for piece in array(own, area)? {
            if piece.is_null() {
                continue;
            }
            piece.as_object().ok_or("Expected public piece object")?;
            let uid = Uid::parse(field(piece, "uid")?)?;
            put_owned(&mut out, uid, area.into(), piece.clone());
            if equipped {
                if let Some(items) = piece.get("items") {
                    for item in items.as_array().ok_or("Expected equipment array")? {
                        let mut row = Map::new();
                        row.insert("kind".into(), json!("item"));
                        row.extend(item.as_object().ok_or("Expected equipment object")?.clone());
                        put_owned(
                            &mut out,
                            Uid::parse(field(item, "uid")?)?,
                            "equipped".into(),
                            Value::Object(row),
                        );
                    }
                }
            }
        }
    }
    Ok(out)
}
fn put_owned(out: &mut Vec<(Uid, String, Value)>, uid: Uid, area: String, piece: Value) {
    if let Some(row) = out.iter_mut().find(|r| r.0 == uid) {
        *row = (uid, area, piece);
    } else {
        out.push((uid, area, piece));
    }
}
fn descriptor(piece: &Value, area: &str) -> Value {
    let mut out = Map::new();
    for key in ["kind", "id", "uid", "row", "col", "dir"] {
        if let Some(v) = piece.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    out.insert("area".into(), json!(area));
    Value::Object(out)
}
fn lookup(rows: &[(Uid, String, Value)], uid: Option<&Value>) -> Result<Value> {
    let Some(uid) = uid.filter(|v| !v.is_null()) else {
        return Ok(Value::Null);
    };
    let uid = Uid::parse(uid)?;
    Ok(rows
        .iter()
        .find(|r| r.0 == uid)
        .map(|r| descriptor(&r.2, &r.1))
        .unwrap_or(Value::Null))
}
fn index<'a>(rows: &'a [Value], v: &Value) -> Result<Option<&'a Value>> {
    let i = integer(v, "action index")?;
    Ok(usize::try_from(i)
        .ok()
        .and_then(|i| rows.get(i))
        .filter(|v| !v.is_null()))
}
fn description(before: &Value, action: &Value) -> Result<(String, Value, Value, Value)> {
    let tag = field(action, "t")?
        .as_str()
        .ok_or("Expected action type string")?;
    let kind: String = tag.chars().skip(2).collect();
    let rows = owned(before, true)?;
    // Python's get(key, default) only falls back when the key is absent.
    let uid = if action.get("uid").is_some() {
        action.get("uid")
    } else {
        action.get("itemUid")
    };
    let mut source = lookup(&rows, uid)?;
    let mut target = lookup(&rows, action.get("targetUid"))?;
    let replaced = lookup(&rows, action.get("replaceUid"))?;
    let own = field(before, "own")?;
    let shop = field(own, "shop")?;
    if matches!(kind.as_str(), "buy" | "reward" | "choice") {
        let empty = Vec::new();
        let (slots, idx, area) = match kind.as_str() {
            "buy" => (array(shop, "slots")?, field(action, "slot")?, "shop"),
            "reward" => (
                match shop.get("rewardOffer").filter(|v| !v.is_null()) {
                    Some(o) => o
                        .get("slots")
                        .map(|v| v.as_array().ok_or("Expected reward slots"))
                        .transpose()?
                        .unwrap_or(&empty),
                    None => &empty,
                },
                field(action, "idx")?,
                "offer",
            ),
            _ => {
                let choice = if field(before, "phase")? == "PREP" {
                    own.get("personalChoice")
                } else {
                    field(before, "public")?.get("sp")
                };
                (
                    match choice.filter(|v| !v.is_null()) {
                        Some(c) => c
                            .get("cards")
                            .map(|v| v.as_array().ok_or("Expected choice cards"))
                            .transpose()?
                            .unwrap_or(&empty),
                        None => &empty,
                    },
                    field(action, "idx")?,
                    "offer",
                )
            }
        };
        source = match index(slots, idx)?.filter(|s| s.as_object().is_none_or(|m| !m.is_empty())) {
            Some(s) => {
                json!({"kind":field(s,"kind")?,"id":s.get("id").unwrap_or(&Value::Null),"area":area})
            }
            None => Value::Null,
        };
    } else if kind == "band" {
        source = json!({"kind":"band","id":field(action,"bandId")?,"area":"offer"});
    }
    if kind == "move" {
        let empty = json!({});
        let to = action.get("to").unwrap_or(&empty);
        match to.get("area").and_then(Value::as_str) {
            Some("board") => {
                let (row, col) = (
                    to.get("row").unwrap_or(&Value::Null),
                    to.get("col").unwrap_or(&Value::Null),
                );
                target = array(own, "board")?
                    .iter()
                    .find(|p| {
                        p.get("row").unwrap_or(&Value::Null) == row
                            && p.get("col").unwrap_or(&Value::Null) == col
                    })
                    .map(|p| descriptor(p, "board"))
                    .unwrap_or(Value::Null);
            }
            Some("hand") => {
                target = index(array(own, "hand")?, to.get("idx").unwrap_or(&Value::Null))?
                    .map(|p| descriptor(p, "hand"))
                    .unwrap_or(Value::Null);
            }
            _ => {}
        }
    } else if ![
        "infoReady",
        "band",
        "choice",
        "buy",
        "reward",
        "refresh",
        "freeze",
        "levelUp",
        "sell",
        "equip",
        "art",
        "destroy",
        "ready",
    ]
    .contains(&kind.as_str())
    {
        return Err("Unknown native action feature identity".into());
    }
    Ok((kind, source, target, replaced))
}

/// A v2 receipt for the immediate reply, before automatic progression.
pub fn execution_report(
    before: &Value,
    action: &Value,
    after: &Value,
    reply: &Value,
) -> Result<Value> {
    execution_report_inner(before, action, after, reply, None)
}

fn execution_report_inner(
    before: &Value,
    action: &Value,
    after: &Value,
    reply: &Value,
    before_id: Option<&str>,
) -> Result<Value> {
    let accepted = field(reply, "accepted")?
        .as_bool()
        .ok_or("Real native acceptance/result required")?;
    let result = field(reply, "result")?;
    result
        .as_object()
        .ok_or("Real native acceptance/result required")?;
    if accepted != (result.get("ok") == Some(&Value::Bool(true))) {
        return Err("Native result/acceptance disagree".into());
    }
    let error = result.get("error").unwrap_or(&Value::Null);
    if error == "INTERNAL" {
        return Err("Native INTERNAL is a technical failure, not policy rejection".into());
    }
    if !accepted && error.as_str().is_none_or(|s| s.is_empty()) {
        return Err("Native rejection omitted its error code".into());
    }
    if accepted && truthy(error) {
        return Err("Accepted native result contains an error".into());
    }
    let (kind, source, target, replaced) = description(before, action)?;
    let a = field(before, "own")?;
    let b = field(after, "own")?;
    let mut changes = Map::new();
    changes.insert(
        "funds_delta".into(),
        delta(field(a, "funds")?, field(b, "funds")?)?,
    );
    for (name, key) in [
        ("spent_delta", "gold"),
        ("refreshes_delta", "refreshes"),
        ("merges_delta", "merges"),
    ] {
        changes.insert(
            name.into(),
            int_value(i128::from(counter(after, key)?) - i128::from(counter(before, key)?))?,
        );
    }
    changes.insert(
        "level_delta".into(),
        delta(
            field(field(a, "shop")?, "level")?,
            field(field(b, "shop")?, "level")?,
        )?,
    );
    changes.insert(
        "deploy_delta".into(),
        delta(field(a, "deployCount")?, field(b, "deployCount")?)?,
    );
    for (name, key, null) in [
        ("hand_free_delta", "hand", true),
        ("temp_used_delta", "temp", false),
    ] {
        let count = |v: &Value| -> Result<i128> {
            Ok(array(v, key)?
                .iter()
                .filter(|v| v.is_null() == null)
                .count() as i128)
        };
        changes.insert(name.into(), int_value(count(b)? - count(a)?)?);
    }
    let layers = |own: &Value| -> Result<BTreeMap<String, Value>> {
        let mut out = BTreeMap::new();
        for row in array(own, "bonds")? {
            out.insert(
                field(row, "bondId")?
                    .as_str()
                    .ok_or("Expected bond ID")?
                    .into(),
                row.get("layers").cloned().unwrap_or(json!(0)),
            );
        }
        Ok(out)
    };
    let la = layers(a)?;
    let lb = layers(b)?;
    let mut bonds = Map::new();
    let mut keys: Vec<_> = la.keys().chain(lb.keys()).collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let av = la.get(key).cloned().unwrap_or(json!(0));
        let bv = lb.get(key).cloned().unwrap_or(json!(0));
        let d = delta(&av, &bv)?;
        if d.as_f64() != Some(0.) {
            bonds.insert(key.clone(), d);
        }
    }
    let class = if accepted {
        Value::Null
    } else {
        json!(if ERRORS.contains(&error.as_str().unwrap()) {
            error.as_str().unwrap()
        } else {
            "OTHER"
        })
    };
    Ok(
        json!({"schema":EXECUTION_SCHEMA,"action_id":action_id(action)?,"action_type":kind,
        "decision_id":decision_id(before)?,"before_id":match before_id {
            Some(id) => id.to_owned(),
            None => fingerprint(before)?,
        },"immediate_after_id":fingerprint(after)?,
        "round":field(before,"round")?,"phase":field(before,"phase")?,"accepted":accepted,
        "error":if accepted {&Value::Null} else {error},"error_class":class,"source":source,"target":target,
        "changes":changes,"bond_layer_deltas":bonds,"intent":action,"replaced":replaced}),
    )
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(n) => n.as_f64() != Some(0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// One ordered stream of trusted, internally owned public JSON snapshots.
#[derive(Clone)]
pub struct PublicProgress {
    stream_id: String,
    state: Option<Arc<Value>>,
    observation_id: Option<String>,
    sequence: u64,
    spent: u64,
    refreshes: u64,
    direct_gains: u64,
    spent_known: bool,
    refreshes_known: bool,
    gains_complete: bool,
    exposures: BTreeMap<Uid, (u64, bool)>,
    last_execution: Option<Arc<Value>>,
}
impl PublicProgress {
    /// Internal constructor: callers supply Python uuid4().hex on every reset.
    /// Lowercase hex is an internal constraint, not a public history-schema rule.
    pub fn new(stream_id: String) -> Result<Self> {
        if stream_id.len() != 32
            || !stream_id
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("Expected a lowercase 32-hex stream ID".into());
        }
        Ok(Self {
            stream_id,
            state: None,
            observation_id: None,
            sequence: 0,
            spent: 0,
            refreshes: 0,
            direct_gains: 0,
            spent_known: false,
            refreshes_known: false,
            gains_complete: false,
            exposures: BTreeMap::new(),
            last_execution: None,
        })
    }
    /// Errors leave the entire ledger unchanged, including observation binding.
    pub fn observe(
        &mut self,
        state: Arc<Value>,
        action: Option<&Value>,
        reply: Option<&Value>,
    ) -> Result<Option<Value>> {
        let mut next = self.clone();
        let receipt = next.observe_inner(state, action, reply)?;
        *self = next;
        Ok(receipt)
    }
    fn observe_inner(
        &mut self,
        state: Arc<Value>,
        action: Option<&Value>,
        reply: Option<&Value>,
    ) -> Result<Option<Value>> {
        let sequence = add(self.sequence, 1)?;
        let mut receipt = match action {
            Some(action) => Some(execution_report_inner(
                self.state
                    .as_deref()
                    .ok_or("Execution observation needs a preceding state and native reply")?,
                action,
                &state,
                reply.ok_or("Execution observation needs a preceding state and native reply")?,
                // The retained Arc binds this identity to the immutable preceding state.
                self.observation_id.as_deref(),
            )?),
            None => {
                if reply.is_some() {
                    return Err("Reply without an executed intent".into());
                }
                None
            }
        };
        if let Some(r) = receipt.as_mut() {
            r["stream_id"] = json!(self.stream_id);
            r["sequence"] = json!(sequence);
        }
        // feedback.py compares these as ordered integers and does not re-run schema checks.
        let round = integer(field(&state, "round")?, "round")?;
        let revision = integer(field(&state, "revision")?, "revision")?;
        let gold = counter(&state, "gold")?;
        let refreshes = counter(&state, "refreshes")?;
        let mut dg = 0;
        let mut dr = 0;
        let mut adjacent = false;
        let changed_round = match self.state.as_deref() {
            Some(before) => {
                if revision <= integer(field(before, "revision")?, "revision")? {
                    return Err(
                        "Public progress requires strictly ordered native observations".into(),
                    );
                }
                dg = gold
                    .checked_sub(counter(before, "gold")?)
                    .ok_or("Public cumulative counters regressed")?;
                dr = refreshes
                    .checked_sub(counter(before, "refreshes")?)
                    .ok_or("Public cumulative counters regressed")?;
                let old_round = integer(field(before, "round")?, "round")?;
                if round < old_round {
                    return Err("Cross-episode observation requires a new PublicProgress".into());
                }
                adjacent = old_round.checked_add(1) == Some(round);
                round != old_round
            }
            None => true,
        };
        if changed_round {
            self.spent = 0;
            self.refreshes = 0;
            self.direct_gains = 0;
            self.spent_known = gold == 0 || adjacent && dg == 0;
            self.refreshes_known = refreshes == 0 || adjacent && dr == 0;
            self.gains_complete = adjacent;
            self.exposures.clear();
        } else {
            self.spent = add(self.spent, dg)?;
            self.refreshes = add(self.refreshes, dr)?;
        }
        for (uid, _, piece) in owned(&state, false)? {
            if field(&piece, "kind")? == "chess" {
                self.exposures.entry(uid).or_insert((
                    0,
                    if changed_round {
                        self.refreshes_known
                    } else {
                        self.state.is_some()
                    },
                ));
            }
        }
        if let Some(r) = receipt.as_ref() {
            self.last_execution = Some(Arc::new(r.clone()));
            if r["accepted"] == true && !changed_round {
                if r["action_type"] == "refresh" {
                    for (uid, area, piece) in owned(self.state.as_deref().unwrap(), false)? {
                        if field(&piece, "kind")? == "chess"
                            && matches!(area.as_str(), "board" | "hand")
                        {
                            let exposure = self.exposures.entry(uid).or_insert((0, false));
                            exposure.0 = add(exposure.0, 1)?;
                        }
                    }
                }
                if matches!(r["action_type"].as_str(), Some("buy" | "reward"))
                    && r["source"]["kind"] == "chess"
                {
                    self.direct_gains = add(self.direct_gains, 1)?;
                }
            }
        }
        if self.exposures.len() > 4096 {
            return Err("Public per-round progress identity capacity exceeded".into());
        }
        self.observation_id = Some(match receipt.as_ref() {
            Some(report) => report["immediate_after_id"]
                .as_str()
                .ok_or("Missing immediate observation identity")?
                .to_owned(),
            None => fingerprint(&state)?,
        });
        self.state = Some(state);
        self.sequence = sequence;
        Ok(receipt)
    }
    pub fn snapshot(&self, state: &Value) -> Result<Value> {
        let observation_id = match self.state.as_deref() {
            Some(owned) if std::ptr::eq(owned, state) => self
                .observation_id
                .clone()
                .ok_or("Progress is not bound to this exact public observation")?,
            _ => fingerprint(state)?,
        };
        if self.observation_id.as_deref() != Some(&observation_id) {
            return Err("Progress is not bound to this exact public observation".into());
        }
        let mut pieces = Map::new();
        for (uid, _, piece) in owned(state, false)? {
            if field(&piece, "kind")? == "chess" {
                let (count, complete) = self.exposures.get(&uid).copied().unwrap_or((0, false));
                pieces.insert(
                    uid.text(),
                    json!({"visible_pre_refreshes":count,"history_complete":complete}),
                );
            }
        }
        Ok(
            json!({"schema":PROGRESS_SCHEMA,"observation_id":observation_id,"stream_id":self.stream_id,"sequence":self.sequence,
            "round":field(state,"round")?,"spent":self.spent,"spent_known":self.spent_known,"refreshes":self.refreshes,
            "refreshes_known":self.refreshes_known,"direct_chess_gains":self.direct_gains,"direct_gains_complete":self.gains_complete,
            "pieces":pieces,"last_execution":self.last_execution.as_deref()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_snapshot_rejects_a_changed_copy() {
        let mut state = Arc::new(json!({
            "round": 1, "revision": 0,
            "own": {"stats": {"gold": 0, "refreshes": 0}, "board": [], "hand": [], "temp": []},
        }));
        let original = state.clone();
        let mut progress = PublicProgress::new("0".repeat(32)).unwrap();
        progress.observe(state.clone(), None, None).unwrap();
        let expected = progress.snapshot(&original).unwrap();
        Arc::make_mut(&mut state)["own"]["funds"] = json!(10);
        assert!(progress.snapshot(&state).is_err());
        assert_eq!(progress.snapshot(&original).unwrap(), expected);
    }
}
