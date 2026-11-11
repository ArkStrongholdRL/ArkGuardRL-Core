// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "bard",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_cetsyr_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SP_FULL",
            attack_override: false,
        },
        Skill {
            id: "skchr_cetsyr_1",
            kind: Kind::Toggle,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_cetsyr_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SP_FULL",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_cetsyr_1" => Mode::First,
        "skchr_cetsyr_2" => Mode::Second,
        "skchr_cetsyr_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.no_attack = true;
    s.combat.projectile = None;
    if mode == Mode::First {
        s.rule_override = Some(skills::Rule::SpFull);
    }
    skill_range(raw, s)?;
    if n(t, "cnt", 3.) > 29. {
        return Err("Mote capacity exceeded".into());
    }
    s.extra.tactics = Tactics::Cetsyr(Cetsyr {
        mode: mode,
        base_motes: n(t, "cnt", 3.).floor().max(0.) as usize,
        mote_radius: n(t, "range_radius", 1.15),
        mote_duration: n(t, "talent_duration", 6.),
        mote_cooldown: n(t, "cooldown", 6.),
        mote_multiplier: n(t, "attack@trait_mul", 1.5),
        skill_cooldown: n(bb, "talent_cool_down", 3.),
        ratio: n(tb, "attack@atk_to_hp_recovery_ratio", 0.1),
        skill_ratio: n(
            bb,
            "attack@atk_to_hp_recovery_ratio",
            n(tb, "attack@atk_to_hp_recovery_ratio", 0.1),
        ),
        outside: n(bb, "outside_radius", 2.),
        mote_damage: n(bb, "atk_scale", 2.2),
        bind: n(bb, "unmoveable_duration", 3.),
        inspire: n(bb, "attack@atk", 0.65),
        hp_inspire: n(bb, "max_hp", 0.65),
        redistribute: n(bb, "attack@cetsyr_s_3[cal_hp_ratio].interval", 2.).max(0.5),
        cut: n(t1, "damage_resistance", 0.1),
        module_atk: n(tm, "atk", 0.),
        module_count: n(tm, "cnt", 2.),
        base_grid: tactical_grid(&raw["rangeGrid"], false)?,
    });
    Ok(())
}
pub(crate) struct Cetsyr {
    pub mode: Mode,
    pub base_motes: usize,
    pub mote_radius: f64,
    pub mote_duration: f64,
    pub mote_cooldown: f64,
    pub mote_multiplier: f64,
    pub skill_cooldown: f64,
    pub ratio: f64,
    pub skill_ratio: f64,
    pub outside: f64,
    pub mote_damage: f64,
    pub bind: f64,
    pub inspire: f64,
    pub hp_inspire: f64,
    pub redistribute: f64,
    pub cut: f64,
    pub module_atk: f64,
    pub module_count: f64,
    pub base_grid: Vec<[i32; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub motes: Vec<f64>,
    pub hit_at: BTreeMap<Id, f64>,
    pub redistribute: f64,
}

impl Cetsyr {
    fn take_mote(&self, k: &mut KitContext<'_>) -> Result<bool, String> {
        let cooldown = if self.mode == Mode::First && k.active()? {
            self.skill_cooldown
        } else {
            self.mote_cooldown
        };
        let now = k.world.time;
        let state = k.world.unit_mut(k.unit)?.operator.tactical.cetsyr_mut()?;
        if let Some(at) = state.motes.iter_mut().find(|at| **at <= now + 1e-9) {
            *at = now + cooldown;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
impl Behavior for Cetsyr {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.cetsyr_mut()?;
                state.redistribute = 0.;
                if self.mode == Mode::Second {
                    state.motes = vec![f64::NEG_INFINITY; self.base_motes + 3];
                    state.hit_at.clear();
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode != Mode::First => {
                let keys = k.keys()?;
                let allies = k.allies(&keys)?;
                let value = if self.mode == Mode::Third {
                    k.stat(id, "maxHp")? * self.hp_inspire
                } else {
                    k.stat(id, "atk")? * self.inspire
                };
                for &target in &allies {
                    if target != id {
                        self.inspire(k, target, value, self.mode == Mode::Third)?;
                    }
                }
                if self.mode == Mode::Third {
                    let due = {
                        let state = k.world.unit_mut(id)?.operator.tactical.cetsyr_mut()?;
                        state.redistribute += context.dt;
                        if state.redistribute + 1e-9 >= self.redistribute {
                            state.redistribute -= self.redistribute;
                            true
                        } else {
                            false
                        }
                    };
                    if due {
                        let mut pool = Vec::new();
                        let mut hp = 0.;
                        let mut maximum = 0.;
                        for target in allies {
                            if target != id
                                && (k.flag(target, "noHeal")?
                                    || k.host.tac_def(target)?.extra.no_heal)
                            {
                                continue;
                            }
                            let max = k.stat(target, "maxHp")?;
                            hp += k.world.unit(target)?.stats.hp;
                            maximum += max;
                            pool.push((target, max));
                        }
                        if pool.len() > 1 && maximum > 0. {
                            let ratio = (hp / maximum).min(1.);
                            for (target, max) in pool {
                                k.world.unit_mut(target)?.stats.hp = (max * ratio).min(max).max(1.);
                            }
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let state = k.world.unit_mut(id)?.operator.tactical.cetsyr_mut()?;
                state.motes.sort_by(f64::total_cmp);
                state.motes.truncate(self.base_motes);
                state.hit_at.clear();
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if k.world.unit(request.hit.target)?.side == health::Side::Ally => {
                if let Some(source) = request.hit.source {
                    if k.world.unit(source)?.side == health::Side::Enemy {
                        let key = &k.host.spawns.as_ref().unwrap().definition(source)?.key;
                        let tagged = k
                            .host
                            .data
                            .table("enemies")
                            .and_then(|v| v.get(key))
                            .and_then(|v| v["tags"].as_array())
                            .is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some("sarkaz")));
                        if tagged {
                            request.damage.mul *= 1. - self.cut;
                        }
                    }
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                if k.world
                    .unit(id)?
                    .operator
                    .tactical
                    .cetsyr()?
                    .motes
                    .is_empty()
                {
                    k.world.unit_mut(id)?.operator.tactical.cetsyr_mut()?.motes =
                        vec![f64::NEG_INFINITY; self.base_motes];
                }
                let active = k.active()?;
                let keys = k.keys()?;
                let attack = k.stat(id, "atk")?;
                let ratio = if active && self.mode != Mode::Second {
                    self.skill_ratio
                } else {
                    self.ratio
                };
                for target in k.allies(&keys)? {
                    let multiplier = if k.has(target, &format!("cetsyr:mote:{id}")) {
                        self.mote_multiplier
                    } else {
                        1.
                    };
                    k.buff(
                        target,
                        &format!("trait:bard:{id}"),
                        [("hpRegen".into(), attack * ratio * multiplier)].into(),
                        0.5,
                    )?;
                }
                if active && self.mode == Mode::Second {
                    let at = k.world.unit(id)?.position;
                    let now = k.world.time;
                    let mut targets = k.radius(at, self.outside)?;
                    targets.sort_unstable();
                    for target in targets {
                        if k.world
                            .unit(id)?
                            .operator
                            .tactical
                            .cetsyr()?
                            .hit_at
                            .get(&target)
                            .is_some_and(|t| *t > now + 1e-9)
                        {
                            continue;
                        }
                        if !self.take_mote(k)? {
                            break;
                        }
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .cetsyr_mut()?
                            .hit_at
                            .insert(target, now + 0.5);
                        k.damage(
                            target,
                            attack * self.mote_damage,
                            DamageType::True,
                            true,
                            "mote",
                        )?;
                        if k.world.unit(target)?.stats.alive {
                            k.status(target, "bind", self.bind, None)?;
                        }
                    }
                } else {
                    let at = k.world.unit(id)?.position;
                    for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if target == id
                            || !Host::tac_up(k.world, target)
                            || body::distance(k.world.unit(target)?, at) > self.mote_radius + 1e-9
                            || k.has(target, &format!("cetsyr:mote:{id}"))
                        {
                            continue;
                        }
                        if !(active && self.mode == Mode::Third) && !self.take_mote(k)? {
                            break;
                        }
                        k.buff(
                            target,
                            &format!("cetsyr:mote:{id}"),
                            Numbers::new(),
                            self.mote_duration,
                        )?;
                    }
                }
                let valid: BTreeSet<_> = k.world.unit_ids().collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .cetsyr_mut()?
                    .hit_at
                    .retain(|target, _| valid.contains(target));
            }
            _ => {}
        }
        // The bard's regeneration timer is registered before the module poll.
        if let KitEvent::Scheduled(due) = event {
            if due[1] && k.up() {
                if self.module_atk != 0. {
                    let base = k.range(&self.base_grid)?;
                    let count = k
                        .allies(&base)?
                        .into_iter()
                        .filter(|&target| target != id)
                        .count();
                    if count as f64 >= self.module_count {
                        k.buff(
                            id,
                            "cetsyr:module",
                            [("atkPct".into(), self.module_atk)].into(),
                            f64::INFINITY,
                        )?;
                    } else {
                        k.remove(id, "cetsyr:module")?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        let mut state = Self::default();
        if let Some(previous) = previous {
            state.motes = previous.motes.clone();
        }
        state
    }
}

impl Cetsyr {
    fn inspire(
        &self,
        k: &mut KitContext<'_>,
        target: Id,
        value: f64,
        hp: bool,
    ) -> Result<(), String> {
        if !(value > 0.)
            || !k.world.unit(target)?.stats.alive
            || matches!(
                k.host.tac_def(target)?.extra.tactics,
                Tactics::Cetsyr(_) | Tactics::Skadi2(_)
            )
        {
            return Ok(());
        }
        let key = if hp { "inspire:hp" } else { "inspire" };
        if let Some(old) = k
            .world
            .unit(target)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some(key))
        {
            if old.runtime.source != Some(k.unit)
                && old.runtime.tick_payload > value
                && old.runtime.time_left > 1e-6
            {
                return Ok(());
            }
        }
        let modifiers = crate::battle::stats::Modifiers::aggregate_except(
            k.world.unit(target)?.stats.buffs(),
            key,
        )
        .map_err(str::to_string)?;
        let (pct, mul, flat) = if hp {
            ("hpPct", "hpMul", "hpFlat")
        } else {
            ("atkPct", "atkMul", "atkFlat")
        };
        let factor = (1. + modifiers.add.get(pct).copied().unwrap_or(0.)).max(0.)
            * modifiers.mul.get(mul).copied().unwrap_or(1.);
        let amount = if factor > 1e-6 { value / factor } else { value };
        let mut buff = Buff {
            key: Some(key.into()),
            mods: [(flat.into(), amount)].into(),
            ..Default::default()
        };
        buff.runtime.duration = 0.25;
        buff.runtime.tick_payload = value;
        k.add(target, buff).map(|_| ())
    }
}
