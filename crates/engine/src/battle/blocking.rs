// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol battle/blocking.js contact and relationship bookkeeping.
use super::{
    deploy::{Deployment, Kind},
    grid::{Grid, COLS, ROWS},
    health::{Id, World},
    stats::Stat,
};
pub trait Hooks {
    fn unblock_with_buffs(
        &mut self,
        w: &mut World,
        _r: &mut super::buffs::Runtime,
        id: Id,
    ) -> Result<(), String>
    where
        Self: Sized,
    {
        unblock(w, self, id)
    }
    fn release_with_buffs(
        &mut self,
        w: &mut World,
        _r: &mut super::buffs::Runtime,
        id: Id,
    ) -> Result<(), String>
    where
        Self: Sized,
    {
        release(w, self, id)
    }
    fn blocked(&mut self, blocker: Id, enemy: Id, w: &mut World) -> Result<(), String>;
    /// Content host updates stealth restoration after a block ends.
    fn ended(&mut self, enemy: Id, w: &mut World) -> Result<(), String>;
}
/// Called by the concrete host's `ended` hook, once the relationship is cleared.
/// Each concealment source has an independent restoration timer.
pub fn restore_stealth<H: super::buffs::Hooks>(
    w: &mut World,
    runtime: &mut super::buffs::Runtime,
    h: &mut H,
    id: Id,
) -> Result<(), String> {
    runtime.call(|runtime| {
        let u = w.unit_mut(id)?;
        if u.side != super::health::Side::Enemy
            || !u.stats.alive
            || !u
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("stealth")
        {
            return Ok(());
        }
        let sources: Vec<_> = u
            .stats
            .buffs()
            .iter()
            .filter(|b| b.flags.contains("stealth"))
            .map(|b| {
                (
                    format!("stealthOff:{}", b.key.as_deref().unwrap_or("null")),
                    b.runtime
                        .stealth_restore
                        .filter(|n| n.is_finite())
                        .unwrap_or(3.),
                )
            })
            .collect();
        for (key, duration) in sources {
            if duration > 0. {
                let mut b = super::stats::Buff::default();
                b.key = Some(key);
                b.flags.insert("stealthOff".into());
                b.runtime.duration = duration;
                runtime.add(
                    w,
                    h,
                    id,
                    super::buffs::Spec {
                        buff: b,
                        flags_present: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    })
}
pub fn unblock<H: Hooks>(w: &mut World, h: &mut H, enemy: Id) -> Result<(), String> {
    let Some(blocker) = w.unit(enemy)?.blocked_by else {
        return Ok(());
    };
    w.unit_mut(blocker)?.blocking.retain(|&id| id != enemy);
    w.unit_mut(enemy)?.blocked_by = None;
    h.ended(enemy, w)
}
pub fn release<H: Hooks>(w: &mut World, h: &mut H, blocker: Id) -> Result<(), String> {
    let held = std::mem::take(&mut w.unit_mut(blocker)?.blocking);
    for id in held {
        if w.unit(id)?.blocked_by == Some(blocker) {
            w.unit_mut(id)?.blocked_by = None;
            h.ended(id, w)?;
        }
    }
    Ok(())
}
/// ai.js enforceBlockCapacity: release the newest relationship first. Capacity
/// depends on the current stats and becomes zero for an undeployed/dead blocker.
pub fn enforce_capacity<H: Hooks>(w: &mut World, h: &mut H, blocker: Id) -> Result<(), String> {
    let u = w.unit_mut(blocker)?;
    let cap = if u.stats.alive && u.deployed {
        u.stats.stats().map_err(str::to_string)?.get(Stat::BlockCnt)
    } else {
        0.0
    };
    let mut used = 0.0;
    for &id in w.unit(blocker)?.blocking() {
        used += w.unit(id)?.block_weight;
    }
    while used > cap {
        let Some(id) = w.unit_mut(blocker)?.blocking.pop() else {
            break;
        };
        if w.unit(id)?.blocked_by() == Some(blocker) {
            w.unit_mut(id)?.blocked_by = None;
            h.ended(id, w)?;
        }
        used -= w.unit(id)?.block_weight;
    }
    Ok(())
}
pub fn blocker_for(
    w: &mut World,
    g: &Grid,
    blocker: Id,
    weight: f64,
    fly: bool,
) -> Result<bool, String> {
    let u = w.unit_mut(blocker)?;
    if !u.stats.alive || !u.deployed || u.hidden {
        return Ok(false);
    }
    let s = u.stats.stats().map_err(str::to_string)?;
    let f = &s.flags;
    if f.contains("noBlock") || f.contains("sleep") || f.contains("noNewBlock") {
        return Ok(false);
    }
    if fly && !(f.contains("blockFly") || u.profile_block_fly) {
        return Ok(false);
    }
    let tile = u.deployment.tile;
    if !fly
        && (!u.ground
            || f.contains("liftoff")
            || !Grid::in_bounds(tile)
            || g.tiles()[tile[0] as usize * COLS + tile[1] as usize].pass != "ALL")
    {
        return Ok(false);
    }
    let cap = s.get(Stat::BlockCnt);
    if cap <= 0.0 {
        return Ok(false);
    }
    let mut used = 0.0;
    for &id in &w.unit(blocker)?.blocking {
        used += w.unit(id)?.block_weight;
    }
    Ok(used + weight <= cap)
}
fn round(x: f64) -> i32 {
    let f = x.floor();
    (if x - f < 0.5 { f } else { f + 1.0 }) as i32
}
pub fn check<H: Hooks>(
    w: &mut World,
    d: &Deployment,
    g: &Grid,
    h: &mut H,
    enemy: Id,
) -> Result<bool, String> {
    let e = w.unit_mut(enemy)?;
    if e.blocked_by.is_some() || e.hidden || !e.stats.alive {
        return Ok(e.blocked_by.is_some());
    }
    let s = e.stats.stats().map_err(str::to_string)?;
    if ["unblockable", "levitate", "fear", "sleep"]
        .iter()
        .any(|k| s.flags.contains(*k))
    {
        return Ok(false);
    }
    let [y, x] = e.position;
    if !y.is_finite() || !x.is_finite() {
        return Err("Nonfinite enemy position".into());
    }
    let weight = e.block_weight;
    let fly = e
        .stats
        .is_flying(true, e.motion_fly)
        .map_err(str::to_string)?;
    let r0 = round(y);
    let c0 = round(x);
    let mut best = None;
    let mut distance = f64::INFINITY;
    for r in r0.saturating_sub(1)..=r0.saturating_add(1) {
        if r < 0 || r >= ROWS as i32 {
            continue;
        }
        for c in c0.saturating_sub(1)..=c0.saturating_add(1) {
            if c < 0 || c >= COLS as i32 || (!fly && r != r0 && c != c0) {
                continue;
            }
            let Some(id) = d.occupant([r, c]) else {
                continue;
            };
            if !blocker_for(w, g, id, weight, fly)? {
                continue;
            }
            let d2 = (x - c as f64) * (x - c as f64) + (y - r as f64) * (y - r as f64);
            let u = w.unit_mut(id)?;
            let k = if fly {
                1.0 + u
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .get(Stat::BlockRadiusScale)
            } else {
                1.0
            };
            let r2 = if u.deployment.kind == Kind::Device {
                0.4472 * 0.4472
            } else if fly {
                0.79995137 * k * k
            } else {
                0.49999037
            };
            if d2 < r2 && d2 < distance {
                best = Some(id);
                distance = d2;
            }
        }
    }
    let Some(id) = best else {
        return Ok(false);
    };
    w.unit_mut(enemy)?.blocked_by = Some(id);
    w.unit_mut(id)?.blocking.push(enemy);
    w.unit_mut(enemy)?.moving = false;
    h.blocked(id, enemy, w)?;
    Ok(true)
}
