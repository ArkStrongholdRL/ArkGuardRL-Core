// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "incantationmedic",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_reed2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_reed2_3" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let self_attack = bb
        .as_object()
        .into_iter()
        .flatten()
        .find(|(key, _)| {
            !key.starts_with("talent@") && (key.as_str() == "atk" || key.ends_with(".atk"))
        })
        .map_or(0., |(_, value)| {
            value.as_f64().filter(|v| v.is_finite()).unwrap_or(0.)
        });
    s.modifiers = [("atkPct".into(), self_attack)].into();
    s.extra.support.incantation = n(&raw["trait"]["bb"], "scale", 0.5);
    let mut profile = s.combat.profile;
    profile.max_targets = n(bb, "max_target", 1.).floor().max(1.);
    s.combat.skill_profile = Some(profile);
    s.extra.tactics = Tactics::Reed2(Reed2 {
        probability: n(t, "prob", 0.),
        skill_probability: n(bb, "talent@prob", n(t, "prob", 0.)),
        scorch_atk: n(t, "atk", 0.),
        fragile: n(t, "damage_scale", 1.) - 1.,
        duration: n(t, "duration", 6.),
        dot: n(bb, "talent@s3_atk_scale", 0.),
        explosion: n(bb, "talent@aoe_scale", 0.),
        radius: n(bb, "talent@range_radius", 1.7),
        share: n(t1, "scale", 0.),
        heal_boost: n(t1, "heal_scale", 1.),
        module_damage: n(talent_at(raw, -1), "damage_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Reed2 {
    probability: f64,
    skill_probability: f64,
    scorch_atk: f64,
    fragile: f64,
    duration: f64,
    dot: f64,
    explosion: f64,
    radius: f64,
    share: f64,
    heal_boost: f64,
    module_damage: f64,
}
#[derive(Default)]
pub(crate) struct State {
    accumulator: f64,
}
impl Reed2 {
    fn scorch(&self, k: &mut KitContext<'_>, target: Id) -> Result<(), String> {
        if !Host::tac_up(k.world, target) {
            return Ok(());
        }
        let duration = if k.active()? {
            k.host
                .roster
                .as_ref()
                .unwrap()
                .entry(k.unit)?
                .skill()
                .map_or(self.duration, |s| s.time_left.max(0.1))
        } else {
            self.duration
        };
        let mut buff = Buff {
            key: Some("reed2:scorch".into()),
            ..Default::default()
        };
        if self.scorch_atk != 0. {
            buff.mods.insert("atkPct".into(), self.scorch_atk);
        }
        buff.runtime.duration = duration;
        buff.runtime.refresh = buffs::Refresh::Extend;
        k.add(target, buff)?;
        if self.fragile > 0. {
            k.status(target, "artsFragile", duration, Some(self.fragile))?;
        }
        Ok(())
    }
    fn explosion(k: &mut KitContext<'_>, position: [f64; 2], _: f64) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Reed2(p) = &def.extra.tactics else {
            return Err("Invalid reed2 kit".into());
        };
        let targets = attacks::Hooks::radius_targets(
            k.host,
            position,
            p.radius,
            true,
            k.world,
            k.world.unit_capacity(),
        )?;
        for target in targets {
            let amount = k.stat(k.unit, "atk")? * p.explosion;
            let mut request = hit::Request::new(
                health::Hit {
                    source: Some(k.unit),
                    credit: Some(k.unit),
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
            request.is_splash = true;
            request.dodge_mode = injury::DodgeMode::Ignore;
            request.tags.extend(["skill".into(), "scorch".into()]);
            k.deal(&mut request)?;
            p.scorch(k, target)?;
        }
        Ok(())
    }
}
impl Behavior for Reed2 {
    fn heal_amount(
        &self,
        _: &World,
        unit: Id,
        target: Id,
        amount: f64,
        _: health::HealOptions,
    ) -> Result<f64, String> {
        Ok(if unit == target {
            amount
        } else {
            amount * self.heal_boost
        })
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
                    .reed2_mut()?
                    .accumulator = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.reed2_mut()?;
                state.accumulator += context.dt;
                if state.accumulator + 1e-9 < 1. {
                    return Ok(());
                }
                state.accumulator -= 1.;
                if self.dot > 0. {
                    let amount = k.stat(id, "atk")? * self.dot;
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        if Host::tac_up(k.world, target)
                            && !k.world.unit(target)?.hidden
                            && k.has(target, "reed2:scorch")
                        {
                            k.damage(target, amount, DamageType::Arts, true, "scorch")?;
                        }
                    }
                }
            }
            KitEvent::Landed { event, request }
                if event.source == Some(id)
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && k.world.unit(event.target)?.stats.alive
                    && !request.tags.iter().any(|tag| tag == "scorch") =>
            {
                let probability = if k.active()? {
                    self.skill_probability
                } else {
                    self.probability
                };
                if probability >= 1. || probability > 0. && k.host.rng.next_f64() < probability {
                    self.scorch(k, event.target)?;
                }
            }
            KitEvent::Death { unit, .. }
                if k.up()
                    && k.active()?
                    && self.explosion > 0.
                    && k.world.unit(*unit)?.side == health::Side::Enemy
                    && k.has(*unit, "reed2:scorch") =>
            {
                k.later_at(
                    0.,
                    k.world.unit(*unit)?.position,
                    0.,
                    false,
                    Self::explosion,
                )?;
            }
            KitEvent::Heal { event, .. }
                if event.source == Some(id)
                    && event.target != id
                    && self.share > 0.
                    && event.amount > 0. =>
            {
                k.heal(
                    id,
                    event.amount * self.share,
                    health::HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
            KitEvent::Scheduled(due) if due[2] && k.up() && self.module_damage > 1. => {
                let keys = k.keys()?;
                if search::Host::injured_allies(
                    k.host,
                    id,
                    &keys,
                    false,
                    k.world,
                    k.world.unit_capacity(),
                )?
                .iter()
                .any(|target| *target != id)
                {
                    k.buff(
                        id,
                        "reed2:corner",
                        [("dmgDealtMul".into(), self.module_damage)].into(),
                        0.4,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
