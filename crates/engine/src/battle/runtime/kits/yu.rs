// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{deploy, element, valued};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "primprotector",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_yu_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "CUSTOM_RANGE",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_yu_3" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    s.trigger_range = Some(tactical_grid(
        &raw["skill"]["trigger"]["customRangeGrid"],
        false,
    )?);
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    s.modifiers = [
        ("hpPct".into(), n(bb, "max_hp", 0.)),
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("defPct".into(), n(bb, "def", 0.)),
    ]
    .into();
    s.extra.tactics = Tactics::Yu(Yu {
        protection: n(t, "damage_resistance", 0.),
        dot: suffix_number(t, "atk_scale", 0.),
        burn: suffix_number(t, "ep_damage_ratio", 0.),
        dot_interval: suffix_number(t, "interval", 1.).max(0.1),
        heal_count: n(t1, "cnt", 4.),
        heal: n(t1, "hp_recovery_per_sec_by_max_hp_ratio", 0.),
        element_heal: n(t1, "ep_heal_ratio", 0.),
        heal_interval: n(t1, "interval", 1.).max(0.1),
        wall_burn: n(bb, "ep_damage_ratio", 0.),
        wall_probability: n(bb, "prob", 0.),
        element_scale: n(&raw["trait"]["bb"], "ep_damage_scale", 1.),
        module_count: n(tm, "cnt", f64::INFINITY),
        module_damage: n(tm, "damage_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Yu {
    protection: f64,
    dot: f64,
    burn: f64,
    dot_interval: f64,
    heal_count: f64,
    heal: f64,
    element_heal: f64,
    heal_interval: f64,
    wall_burn: f64,
    wall_probability: f64,
    element_scale: f64,
    module_count: f64,
    module_damage: f64,
}
#[derive(Default)]
pub(crate) struct State {
    wall: Option<(usize, f64)>,
    touching: BTreeSet<u64>,
    next_dot: f64,
    next_heal: f64,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            next_dot: previous.map_or(0., |s| s.next_dot),
            next_heal: previous.map_or(0., |s| s.next_heal),
            ..Default::default()
        }
    }
}
impl Yu {
    fn operators(k: &KitContext<'_>) -> Result<Vec<Id>, String> {
        Ok(k.host
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .filter(|&a| {
                k.world.unit(a).is_ok_and(|u| {
                    Host::tac_up(k.world, a)
                        && u.deployment.kind == deploy::Kind::Operator
                        && u.owner == k.world.unit(k.unit).unwrap().owner
                })
            })
            .collect())
    }
}
impl Behavior for Yu {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let direction = k
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(id)?
                    .direction
                    .vector();
                let tile = k.world.unit(id)?.deployment.tile;
                let axis = if direction[0] == 0 { 1 } else { 0 };
                let state = k.world.unit_mut(id)?.operator.tactical.yu_mut()?;
                state.wall = Some((axis, tile[axis] as f64 + 0.5 * direction[axis] as f64));
                state.touching.clear();
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.yu_mut()?;
                state.wall = None;
                state.touching.clear();
            }
            KitEvent::DamageApplied { event, request }
                if self.wall_burn > 0.
                    && k.up()
                    && event.amount > 0.
                    && request.hit.damage_type == DamageType::Arts
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && k.world.unit(event.target)?.stats.alive =>
            {
                if let (Some(source), Some((axis, value))) = (
                    event.source.filter(|&a| {
                        a != id && k.world.unit(a).is_ok_and(|u| u.side == health::Side::Ally)
                    }),
                    k.world.unit(id)?.operator.tactical.yu()?.wall,
                ) {
                    if (k.world.unit(source)?.position[axis] - value)
                        * (k.world.unit(event.target)?.position[axis] - value)
                        < 0.
                    {
                        let amount = k.stat(id, "atk")? * self.wall_burn;
                        k.elemental(event.target, amount, element::Element::Burn)?;
                    }
                }
            }
            KitEvent::ProjectileStep {
                id: projectile,
                source,
                position,
                speed,
                removable,
                cancelled,
            } if !**cancelled
                && self.wall_probability > 0.
                && k.up()
                && *removable
                && *speed > 0.
                && source.is_some_and(|a| {
                    k.world.unit(a).is_ok_and(|u| u.side == health::Side::Enemy)
                }) =>
            {
                if let Some((axis, value)) = k.world.unit(id)?.operator.tactical.yu()?.wall {
                    let rect = k.host.grid.as_ref().unwrap().rect();
                    let inside = if axis == 1 {
                        position[0] >= rect.r0 as f64 - 0.5 && position[0] <= rect.r1 as f64 + 0.5
                    } else {
                        position[1] >= rect.c0 as f64 - 0.5 && position[1] <= rect.c1 as f64 + 0.5
                    };
                    let state = k.world.unit_mut(id)?.operator.tactical.yu_mut()?;
                    if !inside || (position[axis] - value).abs() > 0.15 + 1e-9 {
                        state.touching.remove(projectile);
                    } else if state.touching.insert(*projectile) {
                        if state.touching.len() > k.host.tactical_limit {
                            return Err("Yu projectile contact capacity exceeded".into());
                        }
                        if k.host.rng.next_f64() < self.wall_probability {
                            **cancelled = true;
                        }
                    }
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.damage_type == DamageType::Arts
                && self.module_damage > 1.
                && k.has(request.hit.target, "burnBurst") =>
            {
                if let Some(source) = request.hit.source {
                    let shares = source == id
                        || k.up()
                            && k.active()?
                            && k.world.unit(source)?.deployment.kind == deploy::Kind::Operator
                            && k.world.unit(source)?.owner == k.world.unit(id)?.owner;
                    if shares && Self::operators(k)?.len() as f64 >= self.module_count {
                        request.damage.mul *= self.module_damage;
                    }
                }
            }
            KitEvent::Element {
                phase: element::Phase::Hit,
                request,
            } if request.damage.hit.source == Some(id)
                && k.up()
                && !k.world.unit(id)?.blocking().is_empty() =>
            {
                request.damage.damage.mul *= self.element_scale;
            }
            KitEvent::Tick => {
                let active: BTreeSet<_> = k
                    .host
                    .flights
                    .as_ref()
                    .unwrap()
                    .list()
                    .iter()
                    .map(|p| p.id)
                    .collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .yu_mut()?
                    .touching
                    .retain(|p| active.contains(p));
            }
            KitEvent::Scheduled(due) => {
                let now = k.world.time;
                let state = k.world.unit_mut(id)?.operator.tactical.yu_mut()?;
                let dot_at = state.next_dot.max(self.dot_interval);
                let heal_at = state.next_heal.max(self.heal_interval);
                let dot = now + 1e-9 >= dot_at;
                let heal = now + 1e-9 >= heal_at;
                if dot {
                    state.next_dot = dot_at + self.dot_interval;
                }
                if heal {
                    state.next_heal = heal_at + self.heal_interval;
                }
                if !k.up() {
                    return Ok(());
                }
                if due[0] && self.protection > 0. && !k.world.unit(id)?.blocking().is_empty() {
                    valued::apply(
                        k.world,
                        k.host,
                        k.buffs,
                        id,
                        valued::Request {
                            key: "protect",
                            kind: valued::Kind::Custom(|v| {
                                [
                                    ("physTakenMul".into(), 1. - v),
                                    ("artsTakenMul".into(), 1. - v),
                                ]
                                .into()
                            }),
                            plain: false,
                            duration: 0.2,
                            value: self.protection,
                            source: Some(id),
                            stack_as: None,
                        },
                    )?;
                }
                if dot {
                    for enemy in k.world.unit(id)?.blocking().to_vec() {
                        if !Host::tac_up(k.world, enemy) {
                            continue;
                        }
                        let attack = k.stat(id, "atk")?;
                        if self.burn > 0. {
                            k.elemental(enemy, attack * self.burn, element::Element::Burn)?;
                        }
                        if self.dot > 0. && Host::tac_up(k.world, enemy) {
                            let mut request = hit::Request::new(
                                health::Hit {
                                    target: enemy,
                                    source: Some(id),
                                    credit: Some(id),
                                    damage_type: DamageType::Arts,
                                    no_sp: false,
                                    silent: false,
                                    hp_loss: false,
                                    on_applied: false,
                                },
                                Damage {
                                    amount: attack * self.dot,
                                    mul: 1.,
                                    resolved: false,
                                    penetration: Default::default(),
                                },
                            );
                            request.attack_type = hit::AttackType::Buff;
                            request.calculation = hit::Calculation::Precomputed;
                            request.tags.extend(["talent".into(), "dot".into()]);
                            k.deal(&mut request)?;
                        }
                    }
                }
                if heal {
                    let operators = Self::operators(k)?;
                    if operators.len() as f64 >= self.heal_count {
                        let targets = if k.active()? { operators } else { vec![id] };
                        for target in targets {
                            let hp = k.stat(target, "maxHp")?;
                            if self.heal > 0. {
                                k.heal(
                                    target,
                                    hp * self.heal * self.heal_interval,
                                    health::HealOptions {
                                        self_heal: true,
                                        silent: true,
                                        ..Default::default()
                                    },
                                )?;
                            }
                            if self.element_heal > 0. {
                                element::heal(
                                    k.world,
                                    target,
                                    hp * self.element_heal * self.heal_interval,
                                    None,
                                )?;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
