// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "pioneer",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_vulpis_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_vulpis_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_vulpis_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
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
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_vulpis_1" => Mode::First,
        "skchr_vulpis_2" => Mode::Second,
        "skchr_vulpis_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    if mode == Mode::Third {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
        let mut p = s.combat.profile;
        p.hit_all_blocked = true;
        s.combat.skill_profile = Some(p);
        s.extra.skill_status = Some(OnHitStatus {
            key: "stun",
            duration: n(bb, "attack@stun", 0.2),
            chance: None,
        });
    }
    skill_range(raw, s)?;
    s.extra.tactics = Tactics::Vulpis(Vulpis {
        mode: mode,
        dp: n(bb, "cost", 0.),
        extra: n(bb, "extra_damage_ratio", 0.),
        scale: n(bb, "atk_scale", 1.),
        targets: n(bb, "max_target", 6.).max(1.) as usize,
        grid: tactical_grid(&raw["skill"]["rangeGrid"], false)?,
        sluggish: n(bb, "sluggish", 0.),
        stun: n(bb, "stun", 0.),
        aspd: n(bb, "attack_speed", 0.),
        hunt_scale: n(t, "atk_scale", 0.),
        hunt_duration: n(t, "interval", 10.),
        dp_bonus: n(t1, "delta_cost_increase_time", 1.) - 1.,
        quiet: n(t1, "interval", 4.),
        regen: n(
            t1,
            "vulpis_t_2[heal][interval].hp_recovery_per_sec_by_max_hp_ratio",
            0.,
        ),
        block_atk: n(tb, "atk", 0.),
        block_def: n(tb, "def", 0.),
    });
    Ok(())
}
pub(crate) struct Vulpis {
    pub mode: Mode,
    pub dp: f64,
    pub extra: f64,
    pub scale: f64,
    pub targets: usize,
    pub grid: Vec<[i32; 2]>,
    pub sluggish: f64,
    pub stun: f64,
    pub aspd: f64,
    pub hunt_scale: f64,
    pub hunt_duration: f64,
    pub dp_bonus: f64,
    pub quiet: f64,
    pub regen: f64,
    pub block_atk: f64,
    pub block_def: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub duration: f64,
    pub marks: BTreeMap<Id, f64>,
    pub killed: bool,
    pub busy: bool,
}
impl Behavior for Vulpis {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                if self.mode == Mode::Third {
                    k.remove(id, "vulpis:camou")?;
                    let state = k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?;
                    state.killed = false;
                    state.duration = skill.time_left;
                    k.dp(self.dp)?;
                    k.buff(
                        id,
                        "skill:vulpis_aspd",
                        [("aspd".into(), self.aspd)].into(),
                        f64::INFINITY,
                    )?;
                }
                if self.mode == Mode::Second {
                    k.dp(self.dp)?;
                    let keys = k.range(&self.grid)?;
                    let targets = k.enemies(Some(&keys), true)?;
                    for target in targets.into_iter().take(self.targets) {
                        let slow = k.has(target, "sluggish");
                        let amount = k.stat(id, "atk")? * self.scale;
                        k.damage(target, amount, DamageType::Arts, true, "vulpisTorture")?;
                        if Host::tac_up(k.world, target) {
                            k.status(target, "sluggish", self.sluggish, None)?;
                            if slow {
                                k.status(target, "stun", self.stun, None)?;
                            }
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                skill,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                let duration = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .vulpis()?
                    .duration
                    .max(DT);
                let aspd = self.aspd * skill.time_left.max(0.) / duration;
                if let Some(buff) = k
                    .world
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("skill:vulpis_aspd"))
                    .map(|b| b.runtime.id())
                {
                    k.buffs.edit(k.world, id, buff, |b| {
                        b.mods = [("aspd".into(), aspd)].into()
                    })?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                k.remove(id, "skill:vulpis_aspd")?;
                if context.reason != "death"
                    && k.up()
                    && k.world.unit(id)?.operator.tactical.vulpis()?.killed
                {
                    let mut buff = Buff {
                        key: Some("vulpis:camou".into()),
                        flags: ["camou".into()].into(),
                        ..Default::default()
                    };
                    buff.runtime.status = Some("camou".into());
                    k.add(id, buff)?;
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && self.mode == Mode::First
                    && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target.filter(|&t| Host::tac_up(k.world, t)) {
                    let amount = k.stat(id, "atk")? * self.extra;
                    k.damage(target, amount, DamageType::Arts, true, "vulpisPunish")?;
                }
                k.dp(self.dp)?;
            }
            KitEvent::Landed {
                event: landed,
                request,
            } if landed.source == Some(id)
                && k.world.unit(landed.target)?.side == health::Side::Enemy
                && request.hit.damage_type != DamageType::Elemental
                && !request.tags.iter().any(|t| t == "vulpisHunt") =>
            {
                if k.world.unit(id)?.operator.tactical.vulpis()?.busy {
                    return Ok(());
                }
                let now = k.world.time;
                let started = *k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .vulpis_mut()?
                    .marks
                    .entry(landed.target)
                    .or_insert(now);
                if now - started <= self.hunt_duration + 1e-9
                    && Host::tac_up(k.world, landed.target)
                {
                    k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?.busy = true;
                    let amount = k.stat(id, "atk")? * self.hunt_scale;
                    let result =
                        k.damage(landed.target, amount, DamageType::Arts, false, "vulpisHunt");
                    k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?.busy = false;
                    result?;
                }
            }
            KitEvent::Death {
                credit: Some(source),
                ..
            } if *source == id && k.active()? => {
                k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?.killed = true;
            }
            KitEvent::Scheduled(_) => {
                let mut marks = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?.marks,
                );
                marks.retain(|target, _| k.world.unit(*target).is_ok());
                k.world.unit_mut(id)?.operator.tactical.vulpis_mut()?.marks = marks;
            }
            KitEvent::Tick if k.up() => {
                if self.dp_bonus > 0. && k.host.tac_leader(k.world, id)? {
                    k.dp(DT * self.dp_bonus)?;
                }
                let on =
                    k.world.time - k.world.unit(id)?.last_hit_at >= self.quiet && self.regen > 0.;
                if on && !k.has(id, "talent:vulpis_regen") {
                    k.buff(
                        id,
                        "talent:vulpis_regen",
                        [("hpRegenRatio".into(), self.regen)].into(),
                        f64::INFINITY,
                    )?;
                } else if !on {
                    k.remove(id, "talent:vulpis_regen")?;
                }
                if self.block_atk > 0. || self.block_def > 0. {
                    let blocked = !k.world.unit(id)?.blocking().is_empty();
                    k.host.advanced_toggle(
                        k.world,
                        k.buffs,
                        id,
                        "trait:vulpis_block",
                        blocked,
                        [
                            ("atkPct".into(), self.block_atk),
                            ("defPct".into(), self.block_def),
                        ]
                        .into(),
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        let mut state = Self::default();
        if let Some(previous) = previous {
            state.marks = previous.marks.clone();
        }
        state
    }
}
