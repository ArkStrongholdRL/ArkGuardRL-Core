// Public PREP action enumeration. This does not predict engine acceptance.
use pyo3::{
    exceptions::PyValueError,
    intern,
    prelude::*,
    types::{PyDict, PyList},
};

// UID, area (board/hand/temp), kind (chess/token/equip/magic), cell,
// direction, permitted terrain/token cells, equipped item UIDs.
type Piece = (u64, u8, u8, i8, i8, u64, Vec<u64>);
const MASK: u64 = (1 << 36) - 1;

enum Action {
    Sell(u64),
    Destroy(u64),
    Hand(u64, usize),
    Board(u64, usize, usize),
    Equip(u64, u64, Option<u64>),
    Art(u64, usize, usize),
}

fn enumerate(
    pieces: &[Piece],
    empty_hand: u16,
    full: bool,
    recovery: bool,
    ready: bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    if recovery && ready {
        return actions;
    }
    let chess_cells = pieces
        .iter()
        .filter(|p| p.1 == 0 && p.2 == 0)
        .fold(0_u64, |mask, p| mask | (1 << p.3));
    for &(uid, area, kind, cell, direction, allowed, _) in pieces {
        if kind == 0 {
            actions.push(Action::Sell(uid));
        } else if kind >= 2 && (!recovery || area == 2) {
            actions.push(Action::Destroy(uid));
        }
        if recovery && area != 2 {
            continue;
        }
        if area != 1 {
            for slot in 0..10 {
                if !recovery || empty_hand & (1 << slot) != 0 {
                    actions.push(Action::Hand(uid, slot));
                }
            }
        }
        if kind <= 1 {
            for target in 0..36 {
                if allowed & (1 << target) == 0 {
                    continue;
                }
                if area != 0 && kind == 0 && full && chess_cells & (1 << target) == 0 {
                    continue;
                }
                for facing in 0..4 {
                    if area == 0 && target == cell as usize && facing == direction as usize {
                        continue;
                    }
                    actions.push(Action::Board(uid, target, facing));
                }
            }
        } else if kind == 2 {
            for target in pieces.iter().filter(|p| p.2 == 0) {
                actions.push(Action::Equip(uid, target.0, None));
                if target.6.len() >= 2 {
                    for &old in &target.6 {
                        actions.push(Action::Equip(uid, target.0, Some(old)));
                    }
                }
            }
        } else {
            // Magic targets the whole protocol field, including nondeployable cells.
            for target in 0..36 {
                for facing in 0..4 {
                    actions.push(Action::Art(uid, target, facing));
                }
            }
        }
    }
    actions
}

#[pyfunction]
pub fn prep_piece_actions<'py>(
    py: Python<'py>,
    pieces: Vec<Piece>,
    empty_hand: u16,
    capacity_full: bool,
    recovery: bool,
    can_ready: bool,
) -> PyResult<Bound<'py, PyList>> {
    if empty_hand >= 1 << 10
        || pieces.iter().any(|p| {
            p.0 == 0
                || p.1 > 2
                || p.2 > 3
                || p.5 & !MASK != 0
                || (p.1 == 0 && (!(0..36).contains(&p.3) || !(0..4).contains(&p.4)))
        })
    {
        return Err(PyValueError::new_err(
            "Invalid PREP piece or board coordinate",
        ));
    }
    let actions = py.detach(|| enumerate(&pieces, empty_hand, capacity_full, recovery, can_ready));
    let output = PyList::empty(py);
    let directions = [
        intern!(py, "RIGHT"),
        intern!(py, "UP"),
        intern!(py, "DOWN"),
        intern!(py, "LEFT"),
    ];
    for action in actions {
        let row = PyDict::new(py);
        match action {
            Action::Sell(uid) | Action::Destroy(uid) => {
                row.set_item(
                    intern!(py, "t"),
                    if matches!(action, Action::Sell(_)) {
                        intern!(py, "g.sell")
                    } else {
                        intern!(py, "g.destroy")
                    },
                )?;
                row.set_item(intern!(py, "uid"), uid)?;
            }
            Action::Hand(uid, slot) => {
                row.set_item(intern!(py, "t"), intern!(py, "g.move"))?;
                row.set_item(intern!(py, "uid"), uid)?;
                let to = PyDict::new(py);
                to.set_item(intern!(py, "area"), intern!(py, "hand"))?;
                to.set_item(intern!(py, "idx"), slot)?;
                row.set_item(intern!(py, "to"), to)?;
            }
            Action::Board(uid, cell, direction) => {
                row.set_item(intern!(py, "t"), intern!(py, "g.move"))?;
                row.set_item(intern!(py, "uid"), uid)?;
                let to = PyDict::new(py);
                to.set_item(intern!(py, "area"), intern!(py, "board"))?;
                to.set_item(intern!(py, "row"), 9 + cell / 9)?;
                to.set_item(intern!(py, "col"), 2 + cell % 9)?;
                row.set_item(intern!(py, "to"), to)?;
                row.set_item(intern!(py, "dir"), directions[direction])?;
            }
            Action::Equip(uid, target, old) => {
                row.set_item(intern!(py, "t"), intern!(py, "g.equip"))?;
                row.set_item(intern!(py, "itemUid"), uid)?;
                row.set_item(intern!(py, "targetUid"), target)?;
                if let Some(old) = old {
                    row.set_item(intern!(py, "replaceUid"), old)?;
                }
            }
            Action::Art(uid, cell, direction) => {
                row.set_item(intern!(py, "t"), intern!(py, "g.art"))?;
                row.set_item(intern!(py, "itemUid"), uid)?;
                row.set_item(intern!(py, "row"), 9 + cell / 9)?;
                row.set_item(intern!(py, "col"), 2 + cell % 9)?;
                row.set_item(intern!(py, "dir"), directions[direction])?;
            }
        }
        output.append(row)?;
    }
    Ok(output)
}
