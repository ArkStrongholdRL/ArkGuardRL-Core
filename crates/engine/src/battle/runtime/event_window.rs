//! Quiet-state projection. Event frames execute through the normal clock phases.
use super::*;
use crate::battle::{body, cadence, grid::N, search::AttackProfile, skills, stats::Stat};

pub(super) fn quiet_enemy(kit: &content::Kit) -> bool {
    use content::Ability;
    matches!(
        kit,
        content::Kit::Plain
            | content::Kit::Zone(_)
            | content::Kit::Reaction(
                content::Reaction::ExposeDeath { .. }
                    | content::Reaction::HealDeath { .. }
                    | content::Reaction::Barrier { .. }
                    | content::Reaction::Double { .. }
                    | content::Reaction::FirstSplash { .. }
                    | content::Reaction::Fear(_)
                    | content::Reaction::Shell
            )
            | content::Kit::Ability(
                Ability::Unblockable
                    | Ability::Bleed { .. }
                    | Ability::Expose { .. }
                    | Ability::Tide { .. }
                    | Ability::Taunt(_)
                    | Ability::HitShield(_)
                    | Ability::MaxTargets(_)
                    | Ability::LowHp(_)
                    | Ability::Run(_)
                    | Ability::DefDecay { .. }
                    | Ability::FrozenPower(_)
                    | Ability::Stun { .. }
                    | Ability::Power { .. }
                    | Ability::Song { .. }
                    | Ability::DeathBoom { .. }
            )
    )
}

struct QuietHooks {
    profile: AttackProfile,
    event: bool,
}
impl skills::Hooks for QuietHooks {
    fn can_activate(&mut self, _: &skills::Runtime, _: &mut World) -> Result<bool, String> {
        self.event = true;
        Err("Quiet interval reached a skill event".into())
    }
    fn event(
        &mut self,
        _: skills::Event,
        _: &mut skills::Context,
        _: &mut skills::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        self.event = true;
        Err("Quiet interval reached a skill event".into())
    }
    fn condition(
        &mut self,
        _: skills::Condition,
        _: &skills::Runtime,
        _: &mut World,
    ) -> Result<bool, String> {
        Ok(false)
    }
    fn profile(&mut self, _: Id, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(self.profile))
    }
    fn has_sp_hook(&self) -> bool {
        false
    }
}

struct Ally {
    id: Id,
    skill: skills::Runtime,
    next_skill: skills::Runtime,
    cadence: cadence::State,
    next_cadence: cadence::State,
    profile: AttackProfile,
    base_profile: AttackProfile,
    store: bool,
}
struct Timer {
    unit: Id,
    buff: buffs::BuffId,
    left: f64,
    accumulator: f64,
    interval: f64,
    periodic: bool,
    next_left: f64,
    next_accumulator: f64,
}
struct Regen {
    unit: Id,
    hp: f64,
    max_hp: f64,
    rate: f64,
    accumulator: f64,
    next_accumulator: f64,
}

pub(super) struct Window {
    allies: Vec<Ally>,
    timers: Vec<Timer>,
    regeneration: Vec<Regen>,
    deadline: f64,
    trigger_range: [bool; N],
}
impl Window {
    pub(super) fn prepare(h: &mut Host, w: &mut World) -> Result<Option<Self>, String> {
        use crate::battle::content::operators::Tactics;
        let Some(deadline) = h.roster.as_ref().unwrap().quiet_deadline(w)? else {
            return Ok(None);
        };
        let time = w.time;
        let mut out = Self {
            allies: Vec::new(),
            timers: Vec::new(),
            regeneration: Vec::new(),
            deadline: deadline.min(h.movement_bond_deadline()),
            trigger_range: [false; N],
        };
        let ids = h.roster.as_ref().unwrap().ids().to_vec();
        let mut injured_ally = false;
        let mut healer = false;
        for &id in &ids {
            let roster = h.roster.as_ref().unwrap();
            let entry = roster.entry(id)?;
            let def = Arc::clone(&entry.definition);
            let skill = entry.skill()?.clone();
            if skill.pending
                || skill.spec.trigger_allies
                || matches!(
                    skill.spec.rule,
                    skills::Rule::Global | skills::Rule::AutoStop
                )
            {
                return Ok(None);
            }
            let u = w.unit_mut(id)?;
            if !u.stats.alive
                || !u.stats.cached_clean()
                || !u.deployed
                || u.hidden
                || u.removed
                || !u.blocking().is_empty()
                || u.element_buildup.iter().any(|&n| n != 0.)
                || !h.terrain.stationary_ground_clear(u)
            {
                return Ok(None);
            }
            let (flags_empty, max_hp) = {
                let stats = u.stats.stats().map_err(str::to_string)?;
                (stats.flags.is_empty(), stats.get(Stat::MaxHp))
            };
            injured_ally |= u.stats.hp < max_hp;
            healer |= skill.spec.heal_skill
                || def.combat.heal
                || def.combat.skill_heal
                || def.extra.support.auto_heal;
            if !flags_empty {
                return Ok(None);
            }
            if u.operator.support.quiet_until > time {
                out.deadline = out.deadline.min(u.operator.support.quiet_until);
            }
            let store = match &def.extra.tactics {
                Tactics::Indigo(kit) => u.operator.tactical.indigo()?.stored < kit.capacity,
                _ => false,
            };
            for grid in def.trigger_range.iter().chain(def.extra.skill_range.iter()) {
                for key in crate::battle::targeting::absolute_range(
                    grid,
                    u.deployment.tile,
                    entry.direction,
                    0,
                ) {
                    out.trigger_range[key] = true;
                }
            }
            let cadence = u.cadence.clone();
            let profile = crate::battle::ally::Host::effective_profile(h, id, Some(&skill), w)?
                .unwrap_or(def.combat.profile);
            out.allies.push(Ally {
                id,
                next_skill: skill.clone(),
                skill,
                next_cadence: cadence.clone(),
                cadence,
                profile,
                base_profile: def.combat.profile,
                store,
            });
        }
        if healer && injured_ally || !h.choices_clock_quiet(w)? {
            return Ok(None);
        }
        for id in ids
            .into_iter()
            .chain(h.spawns.as_ref().unwrap().live().iter().copied())
        {
            let u = w.unit_mut(id)?;
            let (max_hp, rate) = {
                let stats = u.stats.stats().map_err(str::to_string)?;
                (stats.get(Stat::MaxHp), stats.n("hpRegen"))
            };
            let hp = u.stats.hp;
            if u.pool.is_none() && hp < max_hp && rate > 0. {
                out.regeneration.push(Regen {
                    unit: id,
                    hp,
                    max_hp,
                    rate,
                    accumulator: u.regen_acc,
                    next_accumulator: u.regen_acc,
                });
            }
            for buff in u.stats.buffs() {
                let b = &buff.runtime;
                if b.id() == 0 {
                    return Ok(None);
                }
                let periodic = b.on_tick.is_some()
                    || b.resist_decay
                    || b.element_burst.is_some_and(|b| b.periodic());
                if periodic || b.time_left != f64::INFINITY {
                    out.timers.push(Timer {
                        unit: id,
                        buff: b.id(),
                        left: b.time_left,
                        accumulator: b.accumulator,
                        interval: b.interval,
                        periodic,
                        next_left: b.time_left,
                        next_accumulator: b.accumulator,
                    });
                }
            }
        }
        Ok(Some(out))
    }

    pub(super) fn clear_at(&self, position: [f64; 2]) -> bool {
        let mut clear = true;
        body::enemy_collider_keys(position, |key| clear &= !self.trigger_range[key]);
        clear
    }

    pub(super) fn project(&mut self, w: &mut World, time: f64) -> Result<bool, String> {
        if time + 1e-9 >= self.deadline {
            return Ok(false);
        }
        for regen in &mut self.regeneration {
            regen.next_accumulator = regen.accumulator + regen.rate * DT;
            if regen.next_accumulator >= 1. || regen.hp + regen.next_accumulator >= regen.max_hp {
                return Ok(false);
            }
        }
        for timer in &mut self.timers {
            timer.next_left = timer.left;
            timer.next_accumulator = timer.accumulator;
            if timer.periodic {
                if timer.interval <= 0. {
                    return Ok(false);
                }
                timer.next_accumulator += DT;
                if timer.next_accumulator >= timer.interval - 1e-9 {
                    return Ok(false);
                }
            }
            if timer.left != f64::INFINITY {
                timer.next_left -= DT;
                if timer.next_left <= 1e-9 {
                    return Ok(false);
                }
            }
        }
        let saved_time = w.time;
        w.time = time;
        let result = (|| {
            for ally in &mut self.allies {
                ally.next_skill.clone_from(&ally.skill);
                let mut hooks = QuietHooks {
                    profile: ally.base_profile,
                    event: false,
                };
                if let Err(error) = ally.next_skill.tick(w, &mut hooks, DT) {
                    if hooks.event {
                        return Ok(false);
                    }
                    return Err(error);
                }
                if ally.next_skill.bank.charges != ally.skill.bank.charges {
                    return Ok(false);
                }
                ally.next_cadence.clone_from(&ally.cadence);
                ally.next_cadence.tick(DT, true);
                if ally.next_cadence.cooldown <= 0.
                    && !ally.profile.no_attack
                    && (!ally.profile.only_while_skill || ally.next_skill.bank.active)
                {
                    if ally.store {
                        return Ok(false);
                    }
                    ally.next_cadence.had_target = Some(false);
                }
            }
            Ok(true)
        })();
        w.time = saved_time;
        result
    }

    pub(super) fn accept(&mut self) {
        for regen in &mut self.regeneration {
            regen.accumulator = regen.next_accumulator;
        }
        for ally in &mut self.allies {
            std::mem::swap(&mut ally.skill, &mut ally.next_skill);
            std::mem::swap(&mut ally.cadence, &mut ally.next_cadence);
        }
        for timer in &mut self.timers {
            timer.left = timer.next_left;
            timer.accumulator = timer.next_accumulator;
        }
    }

    pub(super) fn commit(self, h: &mut Host, w: &mut World) -> Result<(), String> {
        for regen in self.regeneration {
            w.unit_mut(regen.unit)?.regen_acc = regen.accumulator;
        }
        for timer in self.timers {
            h.buffs
                .as_mut()
                .unwrap()
                .edit_timer(w, timer.unit, timer.buff, |b| {
                    b.runtime.time_left = timer.left;
                    b.runtime.accumulator = timer.accumulator;
                })?;
        }
        for ally in self.allies {
            let roster = h.roster.as_mut().unwrap();
            roster.take_skill(ally.id)?;
            roster.restore_skill(ally.id, ally.skill)?;
            w.unit_mut(ally.id)?.cadence = ally.cadence;
        }
        Ok(())
    }
}
