// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::deploy;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SPECIAL",
    branch: "geek",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_angel2_2",
        kind: crate::battle::sp::Kind::Ammo,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_angel2_2" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    s.modifiers = [(
        "batPct".into(),
        flat_bat(raw, n(bb, "base_attack_time", 0.)),
    )]
    .into();
    s.combat.skill_scale = n(bb, "attack@atk_scale", 1.);
    s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 10.).floor().max(1.);
    s.extra.tactics = Tactics::Angel2(Angel2 {
        drain: n(tb, "hp_ratio", 0.03),
        steal: n(bb, "steal", 0.),
        extra: n(bb, "addtional_ammo_each", 0.).floor(),
        shield: n(bb, "shield_max_hp_ratio", 0.),
        shield_duration: n(bb, "shield_max_duration", 0.),
        heal: n(t, "hp_ratio", 0.),
        probability: n(t, "prob", 0.),
        blast: n(t, "aoe_atk_scale", n(t, "damage_scale", 0.)),
        covenant: n(t1, "atk", 0.),
        laterano: n(t1, "mult", 1.),
        sp: suffix_number(tb, "sp_recovery_per_sec", 0.),
        threshold: n(tb, "angel2_tr[e].hp_ratio", f64::INFINITY),
    });
    Ok(())
}
pub(crate) struct Angel2 {
    drain: f64,
    steal: f64,
    extra: f64,
    shield: f64,
    shield_duration: f64,
    heal: f64,
    probability: f64,
    blast: f64,
    covenant: f64,
    laterano: f64,
    sp: f64,
    threshold: f64,
}
struct Round {
    time: f64,
    target: Id,
    position: [f64; 2],
    amount: f64,
    count: u32,
    pending: u32,
}
#[derive(Default)]
pub(crate) struct State {
    victim: Option<Id>,
    rounds: BTreeMap<u64, Round>,
    current: BTreeMap<Id, u64>,
    next: u64,
}
const SHIELD: u64 = 1522;
impl Angel2 {
    fn shield(&self, k: &mut KitContext<'_>, target: Id) -> Result<(), String> {
        let total = k.stat(target, "maxHp")? * self.shield;
        if total <= 0. {
            return Ok(());
        }
        let mut buff = Buff {
            key: Some("angel2:barrier".into()),
            shield: total,
            ..Default::default()
        };
        if self.shield_duration > 0. {
            buff.runtime.duration = self.shield_duration;
            buff.runtime.interval = 0.5;
            buff.runtime.tick_payload = total * 0.5 / self.shield_duration;
            buff.runtime.on_tick = Some(SHIELD);
        }
        k.add(target, buff)?;
        Ok(())
    }
    fn bomb(k: &mut KitContext<'_>, key: u64) -> Result<(), String> {
        let target = match k
            .world
            .unit(k.unit)?
            .operator
            .tactical
            .angel2()?
            .rounds
            .get(&key)
        {
            Some(round) => round.target,
            None => return Ok(()),
        };
        if let Ok(target) = k.world.unit(target) {
            if target.stats.alive {
                let position = target.position;
                k.world
                    .unit_mut(k.unit)?
                    .operator
                    .tactical
                    .angel2_mut()?
                    .rounds
                    .get_mut(&key)
                    .unwrap()
                    .position = position;
            }
        }
        let round = &k.world.unit(k.unit)?.operator.tactical.angel2()?.rounds[&key];
        let position = round.position;
        let amount = round.amount;
        let targets = attacks::Hooks::radius_targets(
            k.host,
            position,
            1.3,
            true,
            k.world,
            k.world.unit_capacity(),
        )?;
        for target in targets {
            let mut request = hit::Request::new(
                health::Hit {
                    source: Some(k.unit),
                    credit: Some(k.unit),
                    target,
                    damage_type: DamageType::Physical,
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
            request.tags.extend(["talent".into(), "airstrike".into()]);
            k.deal(&mut request)?;
        }
        if let Some(round) = k
            .world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .angel2_mut()?
            .rounds
            .get_mut(&key)
        {
            round.pending = round.pending.saturating_sub(1);
        }
        Ok(())
    }
    fn ammo(&self, k: &mut KitContext<'_>, consumer: Id) -> Result<(), String> {
        if self.heal > 0. {
            let amount = k.stat(k.unit, "maxHp")? * self.heal;
            k.heal(
                k.unit,
                amount,
                health::HealOptions {
                    self_heal: true,
                    silent: true,
                    ..Default::default()
                },
            )?;
        }
        if self.probability <= 0. || self.blast <= 0. || k.host.rng.next_f64() >= self.probability {
            return Ok(());
        }
        let now = k.world.time;
        let current = k
            .world
            .unit(k.unit)?
            .operator
            .tactical
            .angel2()?
            .current
            .get(&consumer)
            .copied()
            .filter(|key| {
                k.world
                    .unit(k.unit)
                    .unwrap()
                    .operator
                    .tactical
                    .angel2()
                    .unwrap()
                    .rounds
                    .get(key)
                    .is_some_and(|r| r.time == now)
            });
        let key = if let Some(key) = current {
            key
        } else {
            let keys = k
                .host
                .roster
                .as_ref()
                .unwrap()
                .base_range(k.world, consumer)?;
            let targets = k.enemies(Some(&keys), true)?;
            let Some(&target) = targets.first() else {
                return Ok(());
            };
            let position = k.world.unit(target)?.position;
            let amount = k.stat(k.unit, "atk")? * self.blast;
            let limit = k.host.tactical_limit;
            let state = k.world.unit_mut(k.unit)?.operator.tactical.angel2_mut()?;
            if state.rounds.len() >= limit {
                return Err("Angel2 bombing round capacity exceeded".into());
            }
            state.next = state
                .next
                .checked_add(1)
                .ok_or("Angel2 bombing identity overflow")?;
            let key = state.next;
            state.rounds.insert(
                key,
                Round {
                    time: now,
                    target,
                    position,
                    amount,
                    count: 0,
                    pending: 0,
                },
            );
            state.current.insert(consumer, key);
            key
        };
        let round = k
            .world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .angel2_mut()?
            .rounds
            .get_mut(&key)
            .unwrap();
        let index = round.count;
        round.count += 1;
        round.pending += 1;
        if index == 0 {
            Self::bomb(k, key)?;
        } else {
            k.later_key(index as f64 * 0.1, key, Self::bomb)?;
        }
        Ok(())
    }
}
impl Behavior for Angel2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                let keys = k.keys()?;
                let mut candidates: Vec<_> = k
                    .allies(&keys)?
                    .into_iter()
                    .filter(|&a| {
                        a != id
                            && k.world
                                .unit(a)
                                .is_ok_and(|u| u.deployment.kind == deploy::Kind::Operator)
                    })
                    .collect();
                targeting::sort_allies(k.world, id, &mut candidates, Default::default())?;
                let victim = if self.steal > 0. {
                    candidates.first().copied()
                } else {
                    None
                };
                k.world.unit_mut(id)?.operator.tactical.angel2_mut()?.victim = victim;
                self.shield(k, id)?;
                if let Some(victim) = victim {
                    k.buff(
                        victim,
                        "angel2:stolen",
                        [("aspd".into(), -self.steal)].into(),
                        f64::INFINITY,
                    )?;
                    k.buff(
                        id,
                        "angel2:steal",
                        [("aspd".into(), self.steal)].into(),
                        f64::INFINITY,
                    )?;
                    self.shield(k, victim)?;
                    if self.extra > 0. {
                        skill.add_ammo(self.extra);
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let victim = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .angel2_mut()?
                    .victim
                    .take();
                if let Some(victim) = victim.filter(|a| k.world.unit(*a).is_ok()) {
                    k.remove(victim, "angel2:stolen")?;
                }
                k.remove(id, "angel2:steal")?;
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::AmmoUsed,
                ..
            } if k.up() && k.world.unit(*unit)?.side == health::Side::Ally => {
                self.ammo(k, *unit)?;
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == SHIELD => {
                let buff = k.buffs.get(k.world, context.unit, context.buff)?;
                if buff.runtime.source == Some(id) {
                    k.buffs.edit(k.world, context.unit, context.buff, |b| {
                        b.shield = (b.shield - b.runtime.tick_payload).max(0.);
                    })?;
                }
            }
            KitEvent::Tick => {
                let now = k.world.time;
                let state = k.world.unit_mut(id)?.operator.tactical.angel2_mut()?;
                state.rounds.retain(|_, r| r.time == now || r.pending > 0);
                state
                    .current
                    .retain(|_, key| state.rounds.contains_key(key));
            }
            KitEvent::Scheduled(due) if k.up() => {
                if due[4] && self.drain > 0. {
                    let loss = k.stat(id, "maxHp")? * self.drain;
                    let hp = k.world.unit(id)?.stats.hp;
                    if hp - loss >= 1. {
                        k.lose_hp(loss)?;
                    } else {
                        k.world.unit_mut(id)?.stats.hp = hp.min(1.);
                    }
                }
                if due[3] && self.covenant != 0. {
                    for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if Host::tac_up(k.world, target)
                            && k.world.unit(target)?.deployment.kind == deploy::Kind::Operator
                            && k.host.tac_def(target)?.skill_kind() == crate::battle::sp::Kind::Ammo
                        {
                            let factor = if k.faction(target, "lateranoShip", &["laterano"])? {
                                self.laterano
                            } else {
                                1.
                            };
                            k.buff(
                                target,
                                "angel2:covenant",
                                [("atkPct".into(), self.covenant * factor)].into(),
                                0.75,
                            )?;
                        }
                    }
                }
                if due[2]
                    && self.sp > 0.
                    && k.world.unit(id)?.stats.hp / k.stat(id, "maxHp")? > self.threshold
                {
                    k.buff(
                        id,
                        "angel2:calm",
                        [("spRecoveryFlat".into(), self.sp)].into(),
                        0.4,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
