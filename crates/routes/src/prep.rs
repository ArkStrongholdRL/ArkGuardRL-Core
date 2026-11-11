// Public PREP action enumeration. This does not predict engine acceptance.
use pyo3::{
    exceptions::PyValueError,
    intern,
    prelude::*,
    types::{PyDict, PyList},
};

#[path = "prep_domain.rs"]
mod domain;
use domain::{enumerate, Action, Piece, MASK};

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
