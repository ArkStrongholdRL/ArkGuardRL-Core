// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "skywalker",
    ranged: false,
    can_hit_fly: true,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_tippi_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_tippi_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_tippi_1" => Mode::First,
        "skchr_tippi_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.selection.can_hit_fly = true;
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.skill_area = s.combat.area;
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    s.skill_flags = ["liftoff".into(), "blockFly".into()].into();
    skill_range(raw, s)?;
    s.extra.release_on_start = true;
    if mode == Mode::Second {
        s.rule_override = Some(skills::Rule::Never);
        s.combat.skill_hits = 3;
    }
    let mut observer = persistent("tippi:timerObserver", Numbers::new());
    observer.runtime.injury.observe = Some(OBSERVER);
    s.extra.effects.permanent.push(observer);
    s.extra.tactics = Tactics::Tippi(Tippi {
        mode: mode,
        prob: n(t, "prob", 1.),
        wait: n(t, "stack_time", 0.).ceil(),
    });
    Ok(())
}
pub(crate) struct Tippi {
    pub mode: Mode,
    pub prob: f64,
    pub wait: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub ready_at: f64,
}
pub(in crate::battle::runtime) const EVADE: u64 = 1500;
pub(in crate::battle::runtime) const OBSERVER: u64 = 1501;
impl Behavior for Tippi {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .tippi_mut()?
                    .ready_at = k.world.time + self.wait;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                buffs::Hooks::operator_release(k.host, k.world, k.buffs, id)?;
            }
            KitEvent::Tick if k.up() && self.wait > 0. && self.prob > 0. => {
                if !k.has(id, "tippi:evade")
                    && k.world.time + 1e-9 >= k.world.unit(id)?.operator.tactical.tippi()?.ready_at
                {
                    let mut buff = Buff {
                        key: Some("tippi:evade".into()),
                        ..Default::default()
                    };
                    buff.runtime.injury.on_injury = Some(EVADE);
                    k.add(id, buff)?;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Applying,
                request,
            } if request.hit.target == id
                && self.mode == Mode::Second
                && !request.cancelled
                && k.up() =>
            {
                if k.flag(id, "silence")? || !skills::can_act(k.world, id)? {
                    return Ok(());
                }
                let dodgeable = matches!(
                    request.hit.damage_type,
                    DamageType::Physical | DamageType::Arts
                ) && request.dodge_mode != injury::DodgeMode::Ignore;
                let activated = k.skill(|skill, host, world| {
                    if skill.bank.ready() && !skill.bank.active {
                        skill.activate(world, host, "TAKE_DAMAGE", false)
                    } else {
                        Ok(false)
                    }
                })?;
                if activated && dodgeable {
                    request.cancelled = true;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
impl Host {
    pub(in crate::battle::runtime) fn tippi_injury(
        event: &mut injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if !matches!(
            c.request.hit.damage_type,
            DamageType::Physical | DamageType::Arts
        ) {
            return Ok(());
        }
        let id = c.request.hit.target;
        let def = c.host.tac_def(id)?;
        let Tactics::Tippi(p) = &def.extra.tactics else {
            return Err("Invalid Tippi injury owner".into());
        };
        if c.dodge(event, if p.prob >= 1. { None } else { Some(p.prob) })? {
            c.runtime.remove_id(c.world, c.host, id, c.buff)?;
            c.world
                .unit_mut(id)?
                .operator
                .tactical
                .tippi_mut()?
                .ready_at = c.world.time + p.wait;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn tippi_observe(
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        let id = c.request.hit.target;
        let def = c.host.tac_def(id)?;
        let Tactics::Tippi(p) = &def.extra.tactics else {
            return Err("Invalid Tippi observer owner".into());
        };
        let ready = if Self::tac_up(c.world, id) && !Self::tac_has(c.world, id, "tippi:evade") {
            c.world.time + p.wait
        } else {
            f64::INFINITY
        };
        c.world
            .unit_mut(id)?
            .operator
            .tactical
            .tippi_mut()?
            .ready_at = ready;
        Ok(())
    }
}
