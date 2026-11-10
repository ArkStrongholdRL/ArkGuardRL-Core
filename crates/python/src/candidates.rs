//! Complete candidate filtering and encoding share the owned native decision.
#[path = "../../../arkguardrl_core/native/src/prep_domain.rs"]
mod domain;
use arkguard_engine::identity::{action_id, canonical};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

pub type Prepared = (Vec<domain::Piece>, u16, bool, bool, bool);
pub(crate) const CELL_MASK: u64 = domain::MASK;

const DIRECTIONS: [&str; 4] = ["RIGHT", "UP", "DOWN", "LEFT"];

fn intent(action: domain::Action) -> Value {
    use domain::Action::*;
    match action {
        Sell(uid) => json!({"t": "g.sell", "uid": uid}),
        Destroy(uid) => json!({"t": "g.destroy", "uid": uid}),
        Hand(uid, idx) => json!({"t": "g.move", "uid": uid, "to": {"area": "hand", "idx": idx}}),
        Board(uid, cell, dir) => json!({"t": "g.move", "uid": uid,
            "to": {"area": "board", "row": 9 + cell / 9, "col": 2 + cell % 9}, "dir": DIRECTIONS[dir]}),
        Equip(uid, target, old) => {
            let mut action = json!({"t": "g.equip", "itemUid": uid, "targetUid": target});
            if let Some(old) = old {
                action["replaceUid"] = json!(old);
            }
            action
        }
        Art(uid, cell, dir) => json!({"t": "g.art", "itemUid": uid,
            "row": 9 + cell / 9, "col": 2 + cell % 9, "dir": DIRECTIONS[dir]}),
    }
}

pub fn build(
    prefix: Vec<Value>,
    prepared: Option<Prepared>,
    blocked: &BTreeSet<String>,
    terminal: bool,
) -> Result<Vec<Value>, String> {
    let pieces = match prepared {
        Some((pieces, empty, full, recovery, ready)) => {
            if pieces.len() > 128
                || empty >= 1 << 10
                || pieces.iter().any(|p| {
                    p.0 == 0
                        || p.1 > 2
                        || p.2 > 3
                        || p.5 & !domain::MASK != 0
                        || (p.1 == 0 && (!(0..36).contains(&p.3) || !(0..4).contains(&p.4)))
                })
            {
                return Err("Invalid PREP piece or board coordinate".into());
            }
            domain::enumerate(&pieces, empty, full, recovery, ready)
        }
        None => Vec::new(),
    };
    let mut actions = Vec::new();
    let mut seen = HashSet::new();
    for action in prefix.into_iter().chain(pieces.into_iter().map(intent)) {
        if !seen.insert(canonical(&action)?)
            || (!blocked.is_empty() && blocked.contains(&action_id(&action)?))
        {
            continue;
        }
        actions.push(action);
        if actions.len() > 16384 {
            return Err("Native candidate hard cap exceeded; domain was not sliced".into());
        }
    }
    if actions.is_empty() && !terminal {
        return Err("History/public domain exhausted; no fallback wait or forced Ready".into());
    }
    Ok(actions)
}
