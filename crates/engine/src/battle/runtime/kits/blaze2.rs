// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::element;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "primcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[Skill {
        id: "skchr_blaze2_3",
        kind: Kind::Ammo,
        sp_type: SpType::Time,
        rule: "DEFAULT",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    if raw["skill"]["skillId"] != "skchr_blaze2_3" {
        return Err("Unknown selected skill".into());
    }
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        (
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        ),
    ]
    .into();
    skill_range(raw, s)?;
    s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 18.);
    s.combat.skill_area.splash = Some(Splash {
        radius: 1.7,
        scale: 1.,
        hits_fly: true,
        fortress_melee: false,
    });
    s.extra.tactics = Tactics::Blaze2(Blaze2 {
        loss: n(bb, "lose_hp_scale", 0.),
        burst_damage: n(t, "ep_damage_scale", 0.),
        burst_heal: n(t, "hp_ratio", 0.),
        shield: n(t1, "dynamic", 0.),
        regen: n(t1, "hp_recovery_per_sec_by_max_hp_ratio", 0.),
        stun: n(t1, "stun", 0.),
        burst_attack: n(bb, "attack@atk_scale", 0.),
        ammo_refill: n(bb, "ammo_recover", 0.),
        ammo_cap: n(bb, "attack@trigger_time", 18.),
        burst_mul: n(tb, "damage_scale", 1.),
        sp: n(tm, "sp_recovery_per_sec", 0.),
    });
    Ok(())
}
pub(crate) struct Blaze2 {
    pub loss: f64,
    pub burst_damage: f64,
    pub burst_heal: f64,
    pub shield: f64,
    pub regen: f64,
    pub stun: f64,
    pub burst_attack: f64,
    pub ammo_refill: f64,
    pub ammo_cap: f64,
    pub burst_mul: f64,
    pub sp: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub loss_acc: f64,
    pub downed: bool,
    pub pending_end: bool,
    pub pending_ammo: f64,
}

const DOWNED: u64 = 1519;
impl Blaze2 {
    fn resume(k: &mut KitContext<'_>, skill: &mut skills::Runtime) -> Result<(), String> {
        let state = k.world.unit_mut(k.unit)?.operator.tactical.blaze2_mut()?;
        let end = std::mem::take(&mut state.pending_end);
        let ammo = std::mem::take(&mut state.pending_ammo);
        if end {
            k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                skill.end(world, host, "downed")
            })?;
        } else if skill.bank.active {
            let def = k.host.tac_def(k.unit)?;
            let Tactics::Blaze2(p) = &def.extra.tactics else {
                unreachable!()
            };
            skill.add_ammo(ammo.min((p.ammo_cap - skill.ammo_left).max(0.)));
        }
        Ok(())
    }
}
impl Behavior for Blaze2 {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            -50
        } else {
            0
        }
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
                    .blaze2_mut()?
                    .loss_acc = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .blaze2_mut()?
                    .loss_acc += context.dt;
                for _ in 0..30 {
                    if k.world.unit(id)?.operator.tactical.blaze2()?.loss_acc < 1.
                        || !k.world.unit(id)?.stats.alive
                    {
                        break;
                    }
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .blaze2_mut()?
                        .loss_acc -= 1.;
                    let loss = k.stat(id, "maxHp")? * self.loss;
                    k.lose_hp(loss)?;
                }
            }
            KitEvent::Attack { context, skill } if context.attacker == id => {
                if let Some(skill) = skill {
                    Self::resume(k, skill)?;
                }
            }
            KitEvent::Resume { unit, skill } if *unit == id => {
                Self::resume(k, skill)?;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id) => {
                if k.flag(request.hit.target, "burstLock")? {
                    request.damage.mul *= self.burst_mul;
                }
                if request.is_attack && request.is_skill && k.has(request.hit.target, "burnBurst") {
                    let amount = k.stat(id, "atk")? * self.burst_attack;
                    k.element_damage(
                        request.hit.target,
                        amount,
                        element::Element::Burn,
                        "blazeBurn",
                    )?;
                }
            }
            KitEvent::Element {
                phase: element::Phase::Burst,
                request,
            } if request.damage.element == Some(element::Element::Burn) && k.up() => {
                if k.world.unit(request.damage.hit.target)?.side == health::Side::Enemy {
                    let amount = k.stat(id, "atk")? * self.burst_damage;
                    k.element_damage(
                        request.damage.hit.target,
                        amount,
                        element::Element::Burn,
                        "blazeMelt",
                    )?;
                    if !k.world.unit(id)?.operator.tactical.blaze2()?.downed {
                        let amount = k.stat(id, "maxHp")? * self.burst_heal;
                        k.heal(
                            id,
                            amount,
                            health::HealOptions {
                                self_heal: true,
                                ..Default::default()
                            },
                        )?;
                    }
                }
                if k.active()? {
                    if k.host.roster.as_ref().unwrap().entry(id)?.skill().is_ok() {
                        k.skill(|skill, _, _| {
                            skill.add_ammo(
                                self.ammo_refill
                                    .min((self.ammo_cap - skill.ammo_left).max(0.)),
                            );
                            Ok(())
                        })?;
                    } else {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .blaze2_mut()?
                            .pending_ammo += self.ammo_refill;
                    }
                }
            }
            KitEvent::Fatal(event)
                if event.target == id
                    && !event.prevented
                    && !k.world.unit(id)?.operator.tactical.blaze2()?.downed =>
            {
                event.prevented = true;
                k.world.unit_mut(id)?.operator.tactical.blaze2_mut()?.downed = true;
                if !k.end_skill("downed")? {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .blaze2_mut()?
                        .pending_end = true;
                }
                k.release()?;
                let mut buff = Buff {
                    key: Some("blaze2:downed".into()),
                    mods: [
                        ("hpRegenRatio".into(), self.regen),
                        ("blockCnt".into(), -99.),
                    ]
                    .into(),
                    flags: ["disarm".into(), "noHeal".into()].into(),
                    ..Default::default()
                };
                buff.shield = self.shield;
                buff.runtime.interval = 0.2;
                buff.runtime.on_tick = Some(DOWNED);
                k.add(id, buff)?;
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == DOWNED => {
                if k.buffs
                    .get(k.world, context.unit, context.buff)?
                    .runtime
                    .source
                    != Some(id)
                {
                    return Ok(());
                }
                if k.world.unit(id)?.stats.hp >= k.stat(id, "maxHp")? - 1e-6 {
                    k.buffs.remove_id(k.world, k.host, id, context.buff)?;
                    k.world.unit_mut(id)?.operator.tactical.blaze2_mut()?.downed = false;
                    let at = k.world.unit(id)?.position;
                    for target in k.radius(at, 1.7)? {
                        k.status(target, "stun", self.stun, None)?;
                    }
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                let keys = k.keys()?;
                if self.sp != 0. {
                    let mut found = false;
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        if k.world.unit(target)?.stats.alive
                            && body::in_range(k.world.unit(target)?, &keys)
                            && k.flag(target, "burstLock")?
                        {
                            found = true;
                            break;
                        }
                    }
                    if found {
                        k.buff(
                            id,
                            "blaze2:module",
                            [("spRecoveryFlat".into(), self.sp)].into(),
                            0.5,
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
