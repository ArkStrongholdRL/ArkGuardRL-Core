// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 sim/body.js: distinct tile, collider and radius rules.
use super::{
    grid::{COLS, ROWS},
    health::{Side, Unit},
};
use crate::deterministic::math::{hypot, round_half_even};
#[derive(Clone, Copy, Debug)]
pub struct HitArea {
    pub width: f64,
    pub height: f64,
    pub dx: f64,
    pub dy: f64,
}
impl HitArea {
    pub fn new(width: f64, height: f64, dx: f64, dy: f64) -> Option<Self> {
        if width > 0.0 && height > 0.0 && [width, height, dx, dy].iter().all(|x| x.is_finite()) {
            Some(Self {
                width,
                height,
                dx,
                dy,
            })
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
}
pub fn rectangle(u: &Unit) -> Option<Rect> {
    let a = u.hit_area?;
    let cx = u.position[1] + a.dx;
    let cy = u.position[0] + a.dy;
    Some(Rect {
        x0: cx - a.width / 2.0,
        x1: cx + a.width / 2.0,
        y0: cy - a.height / 2.0,
        y1: cy + a.height / 2.0,
    })
}
pub fn position_key(u: &Unit) -> Option<usize> {
    let r = round_half_even(u.position[0]);
    let c = round_half_even(u.position[1]);
    if !r.is_finite()
        || !c.is_finite()
        || r < 0.0
        || r >= ROWS as f64
        || c < 0.0
        || c >= COLS as f64
    {
        None
    } else {
        Some(r as usize * COLS + c as usize)
    }
}
pub fn keys(u: &Unit) -> Vec<usize> {
    let mut out = Vec::new();
    keys_into(u, &mut out);
    out
}
/// Reuse query storage while preserving the exact tile enumeration order.
pub(crate) fn keys_into(u: &Unit, out: &mut Vec<usize>) {
    out.clear();
    let Some(rect) = rectangle(u) else {
        out.extend(position_key(u));
        return;
    };
    if !u.position.iter().all(|x| x.is_finite()) {
        return;
    }
    let r0 = (rect.y0 + 0.5 + 1e-9).floor().max(0.0) as i32;
    let r1 = (rect.y1 - 0.5 - 1e-9).ceil().min(ROWS as f64 - 1.0) as i32;
    let c0 = (rect.x0 + 0.5 + 1e-9).floor().max(0.0) as i32;
    let c1 = (rect.x1 - 0.5 - 1e-9).ceil().min(COLS as f64 - 1.0) as i32;
    for r in r0..=r1 {
        for c in c0..=c1 {
            out.push(r as usize * COLS + c as usize);
        }
    }
}
pub fn on_tile(u: &Unit, r: i32, c: i32) -> bool {
    if let Some(rect) = rectangle(u) {
        c as f64 + 0.5 > rect.x0 + 1e-9
            && c as f64 - 0.5 < rect.x1 - 1e-9
            && r as f64 + 0.5 > rect.y0 + 1e-9
            && r as f64 - 0.5 < rect.y1 - 1e-9
    } else {
        round_half_even(u.position[0]) == r as f64 && round_half_even(u.position[1]) == c as f64
    }
}
pub fn collider_keys(u: &Unit) -> Vec<usize> {
    let mut out = Vec::new();
    collider_keys_into(u, &mut out);
    out
}
pub(crate) fn collider_keys_into(u: &Unit, out: &mut Vec<usize>) {
    out.clear();
    if u.hit_area.is_some() || u.side == Side::Ally {
        keys_into(u, out);
        return;
    }
    enemy_collider_keys(u.position, |key| out.push(key));
}
/// Ordinary enemy colliders touch at most four tiles; callers may avoid a Vec.
pub(crate) fn enemy_collider_keys([y, x]: [f64; 2], mut visit: impl FnMut(usize)) {
    if !y.is_finite() || !x.is_finite() {
        return;
    }
    let rho = 0.1;
    let r0 = (y - rho + 0.5 + 1e-9).floor().max(0.0) as i32;
    let r1 = (y + rho - 0.5 - 1e-9).ceil().min(ROWS as f64 - 1.0) as i32;
    let c0 = (x - rho + 0.5 + 1e-9).floor().max(0.0) as i32;
    let c1 = (x + rho - 0.5 - 1e-9).ceil().min(COLS as f64 - 1.0) as i32;
    for r in r0..=r1 {
        let dy = if y < r as f64 - 0.5 {
            r as f64 - 0.5 - y
        } else if y > r as f64 + 0.5 {
            y - r as f64 - 0.5
        } else {
            0.0
        };
        for c in c0..=c1 {
            let dx = if x < c as f64 - 0.5 {
                c as f64 - 0.5 - x
            } else if x > c as f64 + 0.5 {
                x - c as f64 - 0.5
            } else {
                0.0
            };
            if dx > 0.0 && dy > 0.0 && dx * dx + dy * dy >= rho * rho {
                continue;
            }
            visit(r as usize * COLS + c as usize);
        }
    }
}
pub fn in_keys(u: &Unit, tiles: &[usize]) -> bool {
    keys(u).iter().any(|k| tiles.contains(k))
}
pub fn in_range(u: &Unit, tiles: &[usize]) -> bool {
    collider_keys(u).iter().any(|k| tiles.contains(k))
}
pub fn distance(u: &Unit, origin: [f64; 2]) -> f64 {
    let [y, x] = origin;
    if let Some(r) = rectangle(u) {
        let dx = if x < r.x0 {
            r.x0 - x
        } else if x > r.x1 {
            x - r.x1
        } else {
            0.0
        };
        let dy = if y < r.y0 {
            r.y0 - y
        } else if y > r.y1 {
            y - r.y1
        } else {
            0.0
        };
        hypot(dx, dy)
    } else {
        hypot(u.position[1] - x, u.position[0] - y)
    }
}
pub fn in_radius(u: &Unit, origin: [f64; 2], radius: f64) -> bool {
    distance(u, origin) <= radius + 1e-9
}
pub fn tile_reach(u: &Unit, r: i32, c: i32) -> f64 {
    if u.hit_area.is_none() {
        let dy = (round_half_even(u.position[0]) - r as f64).abs();
        let dx = (round_half_even(u.position[1]) - c as f64).abs();
        if dy.is_nan() || dx.is_nan() {
            f64::NAN
        } else {
            dy.max(dx)
        }
    } else {
        keys(u)
            .into_iter()
            .map(|k| {
                (((k / COLS) as i32 - r)
                    .abs()
                    .max(((k % COLS) as i32 - c).abs())) as f64
            })
            .fold(f64::INFINITY, f64::min)
    }
}
