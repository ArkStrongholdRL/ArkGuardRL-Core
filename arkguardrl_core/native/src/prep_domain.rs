// Public PREP intent domain shared by the Python kernel and native session.
// UID, area (board/hand/temp), kind (chess/token/equip/magic), cell,
// direction, permitted terrain/token cells, equipped item UIDs.
pub type Piece = (u64, u8, u8, i8, i8, u64, Vec<u64>);
pub const MASK: u64 = (1 << 36) - 1;

pub enum Action {
    Sell(u64),
    Destroy(u64),
    Hand(u64, usize),
    Board(u64, usize, usize),
    Equip(u64, u64, Option<u64>),
    Art(u64, usize, usize),
}

pub fn enumerate(
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
