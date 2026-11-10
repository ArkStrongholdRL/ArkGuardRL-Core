// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "mystic",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_indigo_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_indigo_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_indigo_1" => Mode::First,
        "skchr_indigo_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.modifiers = [(
        "batPct".into(),
        if mode == Mode::First {
            n(bb, "base_attack_time", 0.).clamp(-0.95, 0.)
        } else {
            n(bb, "base_attack_time", 1.) - 1.
        },
    )]
    .into();
    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "attack@atk_scale", 1.);
        skill_range(raw, s)?;
    }
    if n(tb, "times", 3.) > 4. {
        return Err("Stored attack capacity exceeded".into());
    }
    s.extra.tactics = Tactics::Indigo(Indigo {
        mode: mode,
        prob: n(t, "prob", 0.),
        bind: n(t, "duration", 0.),
        talent_scale: n(bb, "talent_scale", 1.),
        dot: n(bb, "indigo_s_2[damage].atk_scale", 0.),
        interval: n(bb, "indigo_s_2[damage].interval", 0.5).max(DT),
        capacity: n(tb, "times", 3.).floor().max(0.) as u32,
    });
    Ok(())
}
pub(crate) struct Indigo {
    pub mode: Mode,
    pub prob: f64,
    pub bind: f64,
    pub talent_scale: f64,
    pub dot: f64,
    pub interval: f64,
    pub capacity: u32,
}
#[derive(Default)]
pub(crate) struct State {
    pub stored: u32,
    pub accumulator: f64,
}
impl Behavior for Indigo {
    fn enemy_filter(&self) -> Option<targeting::EnemyFilter> {
        Some(bound_target)
    }
    fn payload(
        &self,
        world: &World,
        id: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        payload.hits = 1 + world.unit(id)?.operator.tactical.indigo()?.stored;
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .indigo_mut()?
                    .accumulator = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let ticks = {
                    let s = k.world.unit_mut(id)?.operator.tactical.indigo_mut()?;
                    s.accumulator += context.dt;
                    let n = ((s.accumulator + 1e-9) / self.interval).floor().max(0.) as usize;
                    s.accumulator -= n as f64 * self.interval;
                    n
                };
                if ticks > 30 {
                    return Err("Indigo periodic payload capacity exceeded".into());
                }
                for _ in 0..ticks {
                    let keys = k.keys()?;
                    let enemies = k.host.spawns.as_ref().unwrap().live().to_vec();
                    for target in enemies {
                        if Host::tac_up(k.world, target)
                            && body::in_range(k.world.unit(target)?, &keys)
                            && k.flag(target, "bind")?
                        {
                            let amount = k.stat(id, "atk")? * self.dot;
                            let mut request = hit::Request::new(
                                health::Hit {
                                    source: Some(id),
                                    credit: Some(id),
                                    target,
                                    damage_type: DamageType::Arts,
                                    no_sp: false,
                                    silent: false,
                                    hp_loss: false,
                                    on_applied: false,
                                },
                                Damage {
                                    amount,
                                    mul: 1.,
                                    resolved: false,
                                    penetration: Default::default(),
                                },
                            );
                            request.is_skill = true;
                            request.dodge_mode = injury::DodgeMode::Ignore;
                            request.tags.push("dot".into());
                            k.deal(&mut request)?;
                        }
                    }
                }
            }
            KitEvent::Attack { context, .. } if context.attacker == id => {
                k.world.unit_mut(id)?.operator.tactical.indigo_mut()?.stored = 0;
            }
            KitEvent::Impact(impact)
                if impact.attacker == id && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target.filter(|&t| Host::tac_up(k.world, t)) {
                    let probability = self.prob * if k.active()? { self.talent_scale } else { 1. };
                    if probability > 0. && k.host.rng.chance(probability.min(1.)) {
                        k.status(target, "bind", self.bind, None)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn bound_target(world: &mut World, _: Id, target: Id) -> Result<bool, String> {
    Ok(world
        .unit_mut(target)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("bind"))
}
