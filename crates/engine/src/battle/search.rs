// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 ai.js acquireTargets/noteAttackCommit/committedAttackTargets.
use super::{
    grid::COLS,
    health::{Id, World},
    stats::Stat,
    targeting::{self, Priority, Profile, Queries},
};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug)]
pub struct Heal {
    pub multi: bool,
    pub count: f64,
    pub hp_at_most: f64,
    pub element_heal: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct AttackProfile {
    pub selection: Profile,
    pub priority: Priority,
    pub heal: Option<Heal>,
    pub heal_attack: bool,
    pub fortress: bool,
    pub all_in_range: bool,
    pub range_aoe: bool,
    pub hit_all_blocked: bool,
    pub max_targets: f64,
    pub no_attack: bool,
    pub only_while_skill: bool,
}
impl Default for AttackProfile {
    fn default() -> Self {
        Self {
            selection: Profile::default(),
            priority: Priority::None,
            heal: None,
            heal_attack: false,
            fortress: false,
            all_in_range: false,
            range_aoe: false,
            hit_all_blocked: false,
            max_targets: 0.0,
            no_attack: false,
            only_while_skill: false,
        }
    }
}
impl AttackProfile {
    fn reselects(self) -> bool {
        self.range_aoe || self.all_in_range || self.heal.is_some() && self.heal_attack
    }
}
/// Geometric/range queries supply already eligible targets in upstream order.
/// Implementors must honour `limit` without clipping: overflow is an error.
pub trait Host: Queries {
    fn enemies(
        &mut self,
        id: Id,
        keys: &[usize],
        p: Profile,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn injured_allies(
        &mut self,
        id: Id,
        keys: &[usize],
        element_heal: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn has_ally_targets(&self) -> bool;
    fn ally_targets(
        &mut self,
        id: Id,
        keys: &[usize],
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn profile(&mut self, id: Id, w: &mut World) -> Result<Option<AttackProfile>, String>;
    fn range(&mut self, id: Id, w: &World) -> Result<Vec<usize>, String>;
    fn skill_active(&self, id: Id, w: &World) -> Result<bool, String>;
    fn can_attack(&mut self, id: Id, p: AttackProfile, w: &mut World) -> Result<bool, String>;
}
#[derive(Debug, PartialEq)]
pub struct Selection {
    pub targets: Vec<Id>,
    pub fortress_melee: Option<bool>,
}
fn bounded(ids: Vec<Id>, limit: usize) -> Result<Vec<Id>, String> {
    if ids.len() > limit {
        Err("Target domain capacity exceeded; no targets dropped".into())
    } else {
        Ok(ids)
    }
}
fn concat(a: &mut Vec<Id>, b: Vec<Id>, limit: usize) -> Result<(), String> {
    if b.len() > limit.saturating_sub(a.len()) {
        return Err("Target domain capacity exceeded; no targets dropped".into());
    }
    a.extend(b);
    Ok(())
}
fn take(mut ids: Vec<Id>, n: f64) -> Vec<Id> {
    let end = if n.is_nan() || n <= 0.0 {
        0
    } else {
        n.min(ids.len() as f64).trunc() as usize
    };
    ids.truncate(end);
    ids
}
fn number(w: &mut World, id: Id, stat: Stat) -> Result<f64, String> {
    w.unit_mut(id)?
        .stats
        .stats()
        .map(|s| s.get(stat))
        .map_err(str::to_string)
}
pub fn acquire<H: Host>(
    w: &mut World,
    h: &mut H,
    id: Id,
    keys: &[usize],
    p: AttackProfile,
    limit: usize,
) -> Result<Selection, String> {
    if limit == 0 {
        return Err("Target capacity must be positive".into());
    }
    if let Some(heal) = p.heal.filter(|_| p.heal_attack) {
        let mut c = bounded(
            h.injured_allies(id, keys, heal.element_heal, w, limit)?,
            limit,
        )?;
        if heal.hp_at_most > 0.0 {
            let mut eligible = Vec::with_capacity(c.len());
            for target in c {
                let max = number(w, target, Stat::MaxHp)?;
                let hp = w.unit(target)?.stats.hp;
                let ratio = if max > 0.0 { hp / max } else { 0.0 };
                if ratio <= heal.hp_at_most + 1e-9 {
                    eligible.push(target);
                }
            }
            c = eligible;
        }
        if c.is_empty() {
            return Ok(Selection {
                targets: c,
                fortress_melee: None,
            });
        }
        let mut n = if heal.multi {
            if heal.count == 0.0 || heal.count.is_nan() {
                3.0
            } else {
                heal.count.max(1.0)
            }
        } else {
            1.0
        };
        if p.max_targets > n {
            n = p.max_targets.floor();
        }
        let extra = number(w, id, Stat::MaxTargets)?.floor().max(0.0);
        return Ok(Selection {
            targets: take(c, n + extra),
            fortress_melee: None,
        });
    }
    let mut melee = None;
    if p.fortress {
        let held = w.unit(id)?.blocking().to_vec();
        if !held.is_empty() {
            let mut c = vec![];
            for target in held {
                if targeting::can_target_enemy(w, h, Some(id), target, Profile::default())? {
                    c.push(target);
                }
            }
            let mut c = bounded(c, limit)?;
            c.truncate(1);
            return Ok(Selection {
                targets: c,
                fortress_melee: Some(true),
            });
        }
        melee = Some(false);
    }
    let mut c = bounded(h.enemies(id, keys, p.selection, w, limit)?, limit)?;
    for target in w.unit(id)?.blocking().to_vec() {
        if targeting::can_target_enemy(w, h, Some(id), target, p.selection)? && !c.contains(&target)
        {
            concat(&mut c, vec![target], limit)?;
        }
    }
    let extra = if h.has_ally_targets() {
        bounded(h.ally_targets(id, keys, w, limit)?, limit)?
    } else {
        vec![]
    };
    if p.fortress && c.is_empty() && extra.is_empty() {
        let [r, col] = w.unit(id)?.deployment.tile;
        let here = [r as usize * COLS + col as usize];
        let mut near = bounded(h.enemies(id, &here, p.selection, w, limit)?, limit)?;
        if near.is_empty() && h.has_ally_targets() {
            concat(&mut near, h.ally_targets(id, &here, w, limit)?, limit)?;
        }
        melee = Some(!near.is_empty());
        targeting::sort_enemies(w, h, id, &mut near, p.priority)?;
        near.truncate(1);
        return Ok(Selection {
            targets: near,
            fortress_melee: melee,
        });
    }
    let has_extra = !extra.is_empty();
    if has_extra {
        if !p.all_in_range {
            targeting::sort_enemies(w, h, id, &mut c, p.priority)?;
        }
        concat(&mut c, extra, limit)?;
    } else if c.is_empty() {
        return Ok(Selection {
            targets: c,
            fortress_melee: melee,
        });
    }
    if p.all_in_range {
        return Ok(Selection {
            targets: c,
            fortress_melee: melee,
        });
    }
    if !has_extra {
        targeting::sort_enemies(w, h, id, &mut c, p.priority)?;
    }
    let n = {
        let u = w.unit_mut(id)?;
        targeting::target_count(
            u.stats.stats().map_err(str::to_string)?,
            p.hit_all_blocked,
            p.max_targets,
        )?
    };
    c.truncate(n);
    Ok(Selection {
        targets: c,
        fortress_melee: melee,
    })
}
/// Each pre-movement capture replaces the complete cache. Failure invalidates the
/// previous snapshot so callers cannot accidentally attack using a prior tick.
pub struct Commitments {
    entries: BTreeMap<Id, Vec<Id>>,
    max_units: usize,
    max_targets: usize,
    max_total: usize,
}
impl Commitments {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn new(max_units: usize, max_targets: usize, max_total: usize) -> Result<Self, String> {
        if [max_units, max_targets, max_total].contains(&0) {
            return Err("Commitment limits must be positive".into());
        }
        Ok(Self {
            entries: BTreeMap::new(),
            max_units,
            max_targets,
            max_total,
        })
    }
    pub fn capture<H: Host>(
        &mut self,
        w: &mut World,
        h: &mut H,
        allies: &[Id],
        dt: f64,
    ) -> Result<(), String> {
        self.entries.clear();
        if allies.len() > self.max_units {
            return Err("Attack commitment unit capacity exceeded".into());
        }
        let mut next = BTreeMap::new();
        let mut total = 0;
        for &id in allies {
            let u = w.unit_mut(id)?;
            if !u.stats.alive || !u.deployed || u.hidden {
                continue;
            }
            let flags = &u.stats.stats().map_err(str::to_string)?.flags;
            if flags.contains("stun") || flags.contains("disarm") {
                continue;
            }
            if u.cadence.cooldown > dt + 1e-9
                || (u.cadence.cooldown <= 0.0 && u.cadence.had_target == Some(false))
            {
                continue;
            }
            let Some(p) = h.profile(id, w)? else {
                continue;
            };
            if p.no_attack || (p.only_while_skill && !h.skill_active(id, w)?) || p.reselects() {
                continue;
            }
            if !h.can_attack(id, p, w)? {
                continue;
            }
            let keys = h.range(id, w)?;
            let raw = acquire(w, h, id, &keys, p, self.max_targets)?.targets;
            if raw.is_empty() {
                continue;
            }
            let mut alive = Vec::new();
            for target in raw {
                if w.unit(target)?.stats.alive {
                    alive.push(target);
                }
            }
            if alive.len() > self.max_total.saturating_sub(total) {
                return Err("Attack commitment row capacity exceeded".into());
            }
            total += alive.len();
            if next.insert(id, alive).is_some() {
                return Err("Duplicate ally in attack commitment capture".into());
            }
        }
        self.entries = next;
        Ok(())
    }
    pub fn targets<H: Host>(
        &self,
        w: &mut World,
        h: &mut H,
        id: Id,
        p: AttackProfile,
    ) -> Result<Vec<Id>, String> {
        if p.reselects() {
            return Ok(vec![]);
        }
        let Some(raw) = self.entries.get(&id) else {
            return Ok(vec![]);
        };
        let mut out = Vec::with_capacity(raw.len());
        for &target in raw {
            if targeting::can_target_enemy(w, h, Some(id), target, p.selection)? {
                out.push(target);
            }
        }
        Ok(out)
    }
}
