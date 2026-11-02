// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol server/sim/dir.js and battle/players.js.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}
impl Direction {
    pub const ALL: [Self; 4] = [Self::Up, Self::Right, Self::Down, Self::Left];
    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Right => "RIGHT",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
        }
    }
    pub fn vector(self) -> [i32; 2] {
        self.rotate([0, 1])
    }
    pub fn rotate(self, [r, c]: [i32; 2]) -> [i32; 2] {
        match self {
            Self::Right => [r, c],
            Self::Up => [c, -r],
            Self::Left => [-r, -c],
            Self::Down => [-c, r],
        }
    }
    pub fn local(self, [r, c]: [i32; 2]) -> [i32; 2] {
        match self {
            Self::Right => [r, c],
            Self::Up => [-c, r],
            Self::Left => [-r, -c],
            Self::Down => [c, -r],
        }
    }
    pub fn mirror(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            _ => self,
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }
    pub fn from_delta(r: f64, c: f64, fallback: Self) -> Self {
        let r = if r.is_nan() { 0.0 } else { r };
        let c = if c.is_nan() { 0.0 } else { c };
        if r == 0.0 && c == 0.0 {
            fallback
        } else if c.abs() >= r.abs() {
            if c > 0.0 {
                Self::Right
            } else {
                Self::Left
            }
        } else if r > 0.0 {
            Self::Up
        } else {
            Self::Down
        }
    }
}
/// Mapping is independent of player coordinate input mode: board ownership always
/// refers to the original board, whereas input tiles may already be absolute.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub boss_like: bool,
    pub right_side: bool,
    pub field_coords: bool,
    pub row_offset: Option<i32>,
    pub col_offset: i32,
}
impl Placement {
    pub fn board_tile(self, [r, c]: [i32; 2]) -> [i32; 2] {
        let r = if let Some(offset) = self.row_offset {
            r + offset
        } else if self.boss_like && r >= 7 {
            r - 7
        } else {
            r
        };
        let c = if self.boss_like && self.right_side {
            if c <= 10 {
                20 - c
            } else {
                c
            }
        } else {
            c + self.col_offset
        };
        [r, c]
    }
    pub fn map_tile(self, tile: [i32; 2], absolute: bool) -> [i32; 2] {
        if absolute || self.field_coords {
            tile
        } else {
            self.board_tile(tile)
        }
    }
    pub fn map_dir(self, d: Direction, absolute: bool) -> Direction {
        if self.boss_like && self.right_side && !absolute && !self.field_coords {
            d.mirror()
        } else {
            d
        }
    }
    pub fn on_own_board(self, tile: [i32; 2]) -> bool {
        (9..=12).any(|r| (2..=10).any(|c| self.board_tile([r, c]) == tile))
    }
}
