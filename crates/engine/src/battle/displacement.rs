// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/displacement.js, computation only.
// Display-facing, interpolation durations and fx payloads are intentionally absent.
use super::{
    blocking,
    direction::Direction,
    grid::{Grid, COLS},
    health::{Id, Side, World},
};
use crate::deterministic::math::hypot;
pub const STOP_RADIUS: f64 = 0.6708;
const MIN_HOLD: f64 = 0.1;
const PUSH: [f64; 6] = [0.12, 0.44, 1.7, 2.14, 2.96, 3.53];
const EFFECT: [f64; 6] = [0.085, 0.374, 1.562, 1.987, 2.773, 3.331];
const HOLD: [f64; 6] = [0.2, 0.4, 0.8, 0.9, 32. / 30., 35. / 30.];
fn finite(n: f64, fallback: f64) -> f64 {
    if n.is_finite() {
        n
    } else {
        fallback
    }
}
fn round(n: f64) -> f64 {
    let f = n.floor();
    let r = if n - f < 0.5 { f } else { f + 1. };
    if r == 0. && n < 0. {
        -0.
    } else {
        r
    }
}
fn row(level: f64) -> Option<usize> {
    let n = round(finite(level, -99.));
    if n <= -3. {
        None
    } else {
        Some((n.min(3.) + 2.) as usize)
    }
}
pub fn push_tiles(level: f64, effect: bool) -> f64 {
    row(level)
        .map(|i| if effect { EFFECT[i] } else { PUSH[i] })
        .unwrap_or(0.)
}
pub fn push_hold(level: f64) -> f64 {
    row(level).map(|i| HOLD[i]).unwrap_or(0.)
}
pub fn pull_hold(level: f64) -> f64 {
    let n = round(finite(level, -99.));
    if n <= -3. {
        0.
    } else if n < -1. {
        0.5
    } else {
        1.
    }
}
pub fn force_level(w: &mut World, id: Id, force: f64) -> Result<f64, String> {
    Ok(round(finite(force, 0.))
        - w.unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("massLevel"))
}
pub fn displaceable(w: &mut World, id: Id) -> Result<bool, String> {
    let u = w.unit_mut(id)?;
    if !u.stats.alive || u.side != Side::Enemy || u.boss {
        return Ok(false);
    }
    Ok(!u
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("noDisplace")
        && !u.static_body)
}
pub fn unbalance(w: &mut World, id: Id, duration: f64) -> Result<(), String> {
    if !(duration > 0.) || !w.unit(id)?.stats.alive {
        return Ok(());
    }
    let until = w.time + duration;
    let u = w.unit_mut(id)?;
    if !(u.motion.unbalance_until >= until) {
        u.motion.unbalance_until = until;
    }
    u.motion.attack_stand_until = f64::NEG_INFINITY;
    Ok(())
}
fn static_force(w: &mut World, id: Id, force: f64, pull: bool) -> Result<(), String> {
    let u = w.unit_mut(id)?;
    if !u.stats.alive
        || u.side != Side::Enemy
        || u.boss
        || !u.static_body
        || u.stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains("noDisplace")
    {
        return Ok(());
    }
    let level = force_level(w, id, force)?;
    if level >= -2. {
        unbalance(
            w,
            id,
            if pull {
                MIN_HOLD.max(pull_hold(level))
            } else {
                MIN_HOLD
            },
        )?;
    }
    Ok(())
}
/// [row, column] vector, bounded to 42 tiles in 0.1-tile collision steps.
pub fn displace<H: blocking::Hooks>(
    w: &mut World,
    g: &Grid,
    h: &mut H,
    id: Id,
    direction: [f64; 2],
    distance: f64,
) -> Result<f64, String> {
    if !displaceable(w, id)? {
        return Ok(0.);
    }
    let dx = finite(direction[1], 0.);
    let dy = finite(direction[0], 0.);
    let len = hypot(dx, dy);
    if !(len > 0.) {
        return Ok(0.);
    }
    let eff = finite(distance, 0.).min(2. * COLS as f64);
    if !(eff > 0.) {
        return Ok(0.);
    }
    let ux = dx / len;
    let uy = dy / len;
    let mut moved = 0.;
    while moved + 1e-9 < eff {
        let s: f64 = 0.1_f64.min(eff - moved);
        let u = w.unit(id)?;
        let nx = u.position[1] + ux * s;
        let ny = u.position[0] + uy * s;
        let r = round(ny) as i32;
        let c = round(nx) as i32;
        // A flyer crosses any in-field tile; hovering follows its ground motion.
        let rect = g.rect();
        let ok = if u.motion_fly {
            r >= rect.r0 && r <= rect.r1 && c >= rect.c0 && c <= rect.c1
        } else {
            g.ground_passable([r, c], false)
        };
        if !ok {
            break;
        }
        w.unit_mut(id)?.position = [ny, nx];
        moved += s;
    }
    if moved > 0. {
        blocking::unblock(w, h, id)?;
        let u = w.unit_mut(id)?;
        u.motion.attack_stand_until = f64::NEG_INFINITY;
        if let Some(route) = u.motion.route.as_mut() {
            route.points = None;
        }
    }
    Ok(moved)
}
#[derive(Clone, Copy, Debug)]
pub struct Origin {
    pub position: [f64; 2],
    pub forward: Option<[f64; 2]>,
    pub unit: Option<Id>,
}
impl Origin {
    pub fn point(position: [f64; 2]) -> Self {
        Self {
            position,
            forward: None,
            unit: None,
        }
    }
    pub fn unit(w: &World, id: Id, direction: Direction) -> Result<Self, String> {
        Ok(Self {
            position: w.unit(id)?.position,
            forward: Some(direction.vector().map(|n| n as f64)),
            unit: Some(id),
        })
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub struct Push {
    pub from: Option<Origin>,
    pub direction: Option<[f64; 2]>,
    pub fixed: bool,
    pub fixed_angle: bool,
    pub inward: bool,
    pub effect: bool,
}
pub fn push<H: blocking::Hooks>(
    w: &mut World,
    g: &Grid,
    h: &mut H,
    id: Id,
    force: f64,
    options: Push,
) -> Result<f64, String> {
    if !displaceable(w, id)? {
        static_force(w, id, force, false)?;
        return Ok(0.);
    }
    let mut level = force_level(w, id, force)?;
    let p = w.unit(id)?.position;
    let origin = options.from.map(|o| o.position).unwrap_or(p);
    let vx = p[1] - finite(origin[1], p[1]);
    let vy = p[0] - finite(origin[0], p[0]);
    let d = hypot(vx, vy);
    let dir = options.direction.unwrap_or([0.; 2]);
    let dir_x = finite(dir[1], 0.);
    let dir_y = finite(dir[0], 0.);
    let dl = hypot(dir_x, dir_y);
    let (mut ux, mut uy);
    if dl > 0. {
        ux = dir_x / dl;
        uy = dir_y / dl;
        if options.from.is_some()
            && !options.fixed
            && (d < 0.25
                || !options.fixed_angle && vx * ux + vy * uy < d * std::f64::consts::FRAC_1_SQRT_2)
        {
            level -= 2.;
            if d > 1e-6 {
                ux = vx / d;
                uy = vy / d;
            }
        }
    } else if d > 1e-6 {
        ux = vx / d;
        uy = vy / d;
    } else if !options.inward && options.from.and_then(|o| o.forward).is_some() {
        let f = options.from.unwrap().forward.unwrap();
        ux = f[1];
        uy = f[0];
    } else {
        return Ok(0.);
    }
    let mut distance = push_tiles(level, options.effect);
    if options.inward && !(dl > 0.) {
        ux = -ux;
        uy = -uy;
        distance = distance.min((d - STOP_RADIUS).max(0.));
    }
    let hold = push_hold(level);
    let moved = displace(w, g, h, id, [uy, ux], distance)?;
    if hold > 0. {
        unbalance(w, id, if moved > 0. { hold } else { MIN_HOLD })?;
    }
    Ok(moved)
}
#[derive(Clone, Copy, Debug)]
pub struct Pull {
    pub to: [f64; 2],
    pub center: Option<Origin>,
    pub stop: f64,
}
pub fn pull<H: blocking::Hooks>(
    w: &mut World,
    g: &Grid,
    h: &mut H,
    id: Id,
    force: f64,
    options: Pull,
) -> Result<f64, String> {
    if let Some(origin) = options.center {
        if let Some(center) = origin.unit {
            if w.unit(center)?.side == Side::Ally && w.unit(id)?.blocked_by() == Some(center) {
                return Ok(0.);
            }
        }
    }
    if !displaceable(w, id)? {
        static_force(w, id, force, true)?;
        return Ok(0.);
    }
    let level = force_level(w, id, force)?;
    let hold = pull_hold(level);
    if !(hold > 0.) {
        return Ok(0.);
    }
    let p = w.unit(id)?.position;
    let tx = finite(options.to[1], p[1]);
    let ty = finite(options.to[0], p[0]);
    let dx = tx - p[1];
    let dy = ty - p[0];
    let d0 = hypot(dx, dy);
    if !(d0 > 1e-6) {
        unbalance(w, id, hold)?;
        return Ok(0.);
    }
    let ux = dx / d0;
    let uy = dy / d0;
    let mut full = d0;
    let center = options.center.map(|o| o.position).unwrap_or([ty, tx]);
    let cx = finite(center[1], tx);
    let cy = finite(center[0], ty);
    let r = finite(options.stop, 0.).max(0.);
    let wx = p[1] - cx;
    let wy = p[0] - cy;
    let wu = wx * ux + wy * uy;
    let w2 = wx * wx + wy * wy;
    if w2 <= r * r {
        full = 0.;
    } else {
        let disc = wu * wu - w2 + r * r;
        if disc >= 0. {
            let t = -wu - disc.sqrt();
            if t >= 0. {
                full = full.min(t);
            }
        }
    }
    let distance = if level >= 0. {
        full
    } else if level == -1. {
        full.min(0.35 * d0)
    } else {
        full.min(0.03)
    };
    let moved = if distance > 1e-6 {
        displace(w, g, h, id, [uy, ux], distance)?
    } else {
        0.
    };
    unbalance(w, id, hold)?;
    Ok(moved)
}
pub fn push_distance(w: &mut World, id: Id, force: f64, effect: bool) -> Result<f64, String> {
    if displaceable(w, id)? {
        Ok(push_tiles(force_level(w, id, force)?, effect))
    } else {
        Ok(0.)
    }
}
pub fn pull_to_front<H: blocking::Hooks>(
    w: &mut World,
    g: &Grid,
    h: &mut H,
    id: Id,
    source: Id,
    direction: Direction,
    force: f64,
) -> Result<f64, String> {
    let center = Origin::unit(w, source, direction)?;
    let f = center.forward.unwrap();
    pull(
        w,
        g,
        h,
        id,
        force,
        Pull {
            to: [
                center.position[0] + f[0] * 0.5,
                center.position[1] + f[1] * 0.5,
            ],
            center: Some(center),
            stop: STOP_RADIUS,
        },
    )
}
