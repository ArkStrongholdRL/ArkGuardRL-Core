// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 sim/targeting.js and ai.js targetCount.
use super::{
    deploy::Kind,
    direction::Direction,
    grid::{COLS, N, ROWS},
    health::{Id, Side, World},
    stats::Stats,
};
use std::{cmp::Ordering, collections::BTreeMap};
#[derive(Clone, Copy, Debug, Default)]
pub struct Profile {
    pub hit_sleep: bool,
    pub can_hit_fly: bool,
    pub doll_hits_fly: bool,
    pub ground_only: bool,
    pub block_free: bool,
}
#[derive(Clone, Debug, Default)]
pub struct State {
    pub doll: bool,
    pub elite: bool,
    pub ranged_apply_melee: bool,
    pub spawn_seq: u64,
}
/// Content-specific exclusion, per-source stealth restoration and geometric route
/// distances are resolved by the concrete battle host, without executing actions.
pub trait Queries {
    fn enemy_stealthed(&mut self, id: Id, w: &mut World) -> Result<bool, String>;
    fn skip_enemy(
        &mut self,
        attacker: Option<Id>,
        enemy: Id,
        w: &mut World,
    ) -> Result<bool, String>;
    fn remaining_distance(&mut self, id: Id, w: &mut World) -> Result<f64, String>;
    fn body_distance(&mut self, id: Id, origin: [f64; 2], w: &mut World) -> Result<f64, String>;
}
fn snapshot(w: &mut World, id: Id) -> Result<Stats, String> {
    w.unit_mut(id)?
        .stats
        .stats()
        .cloned()
        .map_err(str::to_string)
}
fn flying(w: &mut World, id: Id) -> Result<bool, String> {
    let u = w.unit_mut(id)?;
    u.stats
        .is_flying(u.side == Side::Enemy, u.motion_fly)
        .map_err(str::to_string)
}
pub fn can_target_enemy<Q: Queries>(
    w: &mut World,
    q: &mut Q,
    attacker: Option<Id>,
    enemy: Id,
    p: Profile,
) -> Result<bool, String> {
    let e = w.unit(enemy)?;
    if !e.stats.alive || e.hidden || !e.deployed {
        return Ok(false);
    }
    let flags = snapshot(w, enemy)?.flags;
    if flags.contains("untargetable") || (flags.contains("sleep") && !p.hit_sleep) {
        return Ok(false);
    }
    if flags.contains("stealth") && q.enemy_stealthed(enemy, w)? {
        return Ok(false);
    }
    let doll = attacker
        .map(|id| w.unit(id).map(|u| u.targeting.doll))
        .transpose()?
        .unwrap_or(false);
    if flying(w, enemy)? && !(p.can_hit_fly || p.doll_hits_fly && doll) {
        return Ok(false);
    }
    if p.ground_only && flying(w, enemy)? {
        return Ok(false);
    }
    if q.skip_enemy(attacker, enemy, w)? {
        return Ok(false);
    }
    Ok(true)
}
pub fn evades_ground(w: &mut World, source: Option<Id>, target: Id) -> Result<bool, String> {
    if !snapshot(w, target)?.flags.contains("liftoff") {
        return Ok(false);
    }
    if let Some(id) = source {
        if w.unit(id)?.side == Side::Enemy && !flying(w, id)? {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn can_target_ally(
    w: &mut World,
    enemy: Id,
    ally: Id,
    ranged: bool,
    enemy_profile: Profile,
) -> Result<bool, String> {
    let a = w.unit(ally)?;
    if !a.stats.alive || !a.deployed || a.hidden || a.deployment.kind == Kind::Device {
        return Ok(false);
    }
    let f = snapshot(w, ally)?.flags;
    if f.contains("untargetable") || f.contains("sleep") {
        return Ok(false);
    }
    if ranged
        && (f.contains("stealth") || f.contains("camou"))
        && (w.unit(enemy)?.blocked_by() != Some(ally) || enemy_profile.block_free)
    {
        return Ok(false);
    }
    Ok(!evades_ground(w, Some(enemy), ally)?)
}
pub fn aura_selectable(w: &mut World, ally: Id) -> Result<bool, String> {
    let a = w.unit(ally)?;
    if !a.stats.alive || !a.deployed || a.hidden || a.deployment.kind == Kind::Device {
        return Ok(false);
    }
    let f = snapshot(w, ally)?.flags;
    Ok(!["untargetable", "sleep", "stealth"]
        .iter()
        .any(|k| f.contains(*k)))
}
pub fn area_selectable(w: &mut World, source: Option<Id>, ally: Id) -> Result<bool, String> {
    Ok(aura_selectable(w, ally)? && !evades_ground(w, source, ally)?)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    None,
    Fly,
    LowDef,
    HighDef,
    Ranged,
    LowestHp,
    HighestHp,
    LowestHpRatio,
    HighestAtk,
    Boss,
    Elite,
    NotBurst,
    Ground,
    Heaviest,
    Nearest,
    Farthest,
}
// JS comparator's `difference || next` treats NaN and signed zero as ties.
fn compare(a: &[f64], b: &[f64]) -> Ordering {
    for (&a, &b) in a.iter().zip(b) {
        let d = a - b;
        if d.is_nan() || d == 0.0 {
            continue;
        }
        return if d < 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    Ordering::Equal
}
pub fn sort_enemies<Q: Queries>(
    w: &mut World,
    q: &mut Q,
    attacker: Id,
    candidates: &mut [Id],
    priority: Priority,
) -> Result<(), String> {
    if candidates.len() <= 1 {
        return Ok(());
    }
    let origin = w.unit(attacker)?.position;
    let mut keyed = Vec::with_capacity(candidates.len());
    for &id in candidates.iter() {
        let b = if w.unit(id)?.blocked_by() == Some(attacker) {
            0.0
        } else {
            1.0
        };
        let p = match priority {
            Priority::Fly => {
                if flying(w, id)? {
                    0.0
                } else {
                    1.0
                }
            }
            Priority::Ground => {
                if flying(w, id)? {
                    1.0
                } else {
                    0.0
                }
            }
            Priority::LowDef => snapshot(w, id)?.n("def"),
            Priority::HighDef => -snapshot(w, id)?.n("def"),
            Priority::LowestHp => w.unit(id)?.stats.hp,
            Priority::HighestHp => -w.unit(id)?.stats.hp,
            Priority::LowestHpRatio => {
                let max = snapshot(w, id)?.n("maxHp");
                if max > 0.0 {
                    w.unit(id)?.stats.hp / max
                } else {
                    0.0
                }
            }
            Priority::HighestAtk => -snapshot(w, id)?.n("atk"),
            Priority::Boss => {
                if w.unit(id)?.boss {
                    0.0
                } else {
                    1.0
                }
            }
            Priority::Elite => {
                if w.unit(id)?.boss || w.unit(id)?.targeting.elite {
                    0.0
                } else {
                    1.0
                }
            }
            Priority::NotBurst => {
                if snapshot(w, id)?.flags.contains("burstLock") {
                    1.0
                } else {
                    0.0
                }
            }
            Priority::Heaviest => -snapshot(w, id)?.n("massLevel"),
            Priority::Ranged => {
                let u = w.unit(id)?;
                if u.stats.base_number("rangeRadius") > 0.0 && !u.targeting.ranged_apply_melee {
                    0.0
                } else {
                    1.0
                }
            }
            Priority::Nearest => q.body_distance(id, origin, w)?,
            Priority::Farthest => -q.body_distance(id, origin, w)?,
            Priority::None => 0.0,
        };
        let taunt = snapshot(w, id)?.n("taunt");
        let t = if taunt.is_nan() || taunt == 0.0 {
            0.0
        } else {
            -taunt
        };
        let d = q.remaining_distance(id, w)?;
        let seq = w.unit(id)?.targeting.spawn_seq as f64;
        keyed.push((
            id,
            [
                if priority == Priority::Fly { p } else { 0.0 },
                b,
                p,
                t,
                d,
                seq,
            ],
        ));
    }
    keyed.sort_by(|a, b| compare(&a.1, &b.1));
    for (id, (sorted, _)) in candidates.iter_mut().zip(keyed) {
        *id = sorted;
    }
    Ok(())
}
pub fn sort_allies(
    w: &mut World,
    enemy: Id,
    candidates: &mut [Id],
    profile: Profile,
) -> Result<(), String> {
    if candidates.len() <= 1 {
        return Ok(());
    }
    let blocker = if profile.block_free {
        None
    } else {
        w.unit(enemy)?.blocked_by()
    };
    let mut keys = Vec::with_capacity(candidates.len());
    for &id in candidates.iter() {
        let t = snapshot(w, id)?.n("taunt");
        let t = if t.is_nan() || t == 0.0 { 0.0 } else { -t };
        let u = w.unit(id)?;
        let seq = if u.deployment.aggro_seq != 0 {
            u.deployment.aggro_seq
        } else {
            u.deployment.deploy_seq
        };
        keys.push((
            id,
            [
                if blocker == Some(id) { 0.0 } else { 1.0 },
                t,
                -(seq as f64),
            ],
        ));
    }
    keys.sort_by(|a, b| compare(&a.1, &b.1));
    for (id, (sorted, _)) in candidates.iter_mut().zip(keys) {
        *id = sorted;
    }
    Ok(())
}
pub fn absolute_range(
    grid: &[[i32; 2]],
    origin: [i32; 2],
    dir: Direction,
    extend: u32,
) -> Vec<usize> {
    let mut seen = [false; N];
    let mut out = Vec::new();
    let mut add = |p: [i32; 2]| {
        let [r, c] = p.map(i64::from);
        let [r, c] = match dir {
            Direction::Right => [r, c],
            Direction::Up => [c, -r],
            Direction::Down => [-c, r],
            Direction::Left => [-r, -c],
        };
        let rr = origin[0] as i64 + r;
        let cc = origin[1] as i64 + c;
        if rr < 0 || rr >= ROWS as i64 || cc < 0 || cc >= COLS as i64 {
            return;
        }
        let k = rr as usize * COLS + cc as usize;
        if !seen[k] {
            seen[k] = true;
            out.push(k);
        }
    };
    for &p in grid {
        add(p);
    }
    let mut rows = Vec::new();
    let mut max = BTreeMap::new();
    for &[r, c] in grid {
        if !max.contains_key(&r) {
            rows.push(r);
        }
        let entry = max.entry(r).or_insert(c);
        *entry = (*entry).max(c);
    }
    for r in rows {
        for k in 1..=extend.min(COLS as u32) {
            if let Some(c) = max[&r].checked_add(k as i32) {
                add([r, c]);
            }
        }
    }
    out
}
pub fn target_count(
    stats: &Stats,
    hit_all_blocked: bool,
    max_targets: f64,
) -> Result<usize, String> {
    let base = if hit_all_blocked {
        stats.n("blockCnt").floor().max(1.0)
    } else if max_targets == 0.0 || max_targets.is_nan() {
        1.0
    } else {
        max_targets
    };
    let n = (base + stats.n("maxTargets")).floor().max(1.0);
    if !n.is_finite() || n >= usize::MAX as f64 {
        return Err("Target count exceeds native capacity".into());
    }
    Ok(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::stats::{Base, Buff, Cultivation, Modifiers};
    #[test]
    fn attack_target_count_uses_capacity_plus_modifiers_not_current_blocked_count() {
        let mut b = Base::default();
        b.0.insert("blockCnt".into(), 3.0);
        let m = Modifiers::aggregate(&[Buff {
            mods: [("maxTargets".into(), 2.0)].into_iter().collect(),
            ..Buff::default()
        }])
        .unwrap();
        let s = Stats::calculate(&b, &m, Cultivation::default(), 0.0);
        assert_eq!(target_count(&s, true, 1.0).unwrap(), 5);
        assert_eq!(target_count(&s, false, 2.0).unwrap(), 4);
        assert_eq!(target_count(&s, false, 0.0).unwrap(), 3);
    }
    #[test]
    fn range_extension_preserves_input_row_order_and_deduplicates() {
        assert_eq!(
            absolute_range(&[[1, 0], [0, 0], [1, 0]], [10, 4], Direction::Right, 1),
            vec![11 * 21 + 4, 10 * 21 + 4, 11 * 21 + 5, 10 * 21 + 5]
        );
        assert!(absolute_range(&[[0, 0]], [-1, 0], Direction::Right, 0).is_empty());
    }
}
