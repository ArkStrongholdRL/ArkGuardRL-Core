// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "funnel",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_rockr_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_magic_rage[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_magic_rage[3]" => Mode::First,
        "skchr_rockr_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.modifiers = [("aspd".into(), n(bb, "attack_speed", 0.))].into();
    s.extra.tactics = Tactics::Rockr(Rockr {
        mode: mode,
        duration: n(&raw["skill"], "duration", 0.),
        funnel: funnel::Parameters::read(raw),
        over_atk: n(bb, "atk", 0.),
        over_scale: n(bb, "scale", 1.),
        rock_period: n(t, "interval", 0.),
        rock_stacks: n(t, "max_stack_cnt", 0.).floor().max(0.) as u32,
        rock_atk: n(t, "atk", 0.),
    });
    Ok(())
}
pub(crate) struct Rockr {
    pub mode: Mode,
    pub duration: f64,
    pub funnel: funnel::Parameters,
    pub over_atk: f64,
    pub over_scale: f64,
    pub rock_period: f64,
    pub rock_stacks: u32,
    pub rock_atk: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub funnel: funnel::State,
    pub drone_target: Option<Id>,
    pub drone_previous: Option<Id>,
    pub drone_scale: f64,
    pub drone_cooldown: f64,
    pub over_at: f64,
    pub overloaded: bool,
}
impl Rockr {
    fn first_target(k: &mut KitContext<'_>) -> Result<Option<Id>, String> {
        let profile = k.host.tac_def(k.unit)?.combat.profile;
        let mut enemies = k.enemies(None, true)?;
        for enemy in k.world.unit(k.unit)?.blocking().to_vec() {
            if !enemies.contains(&enemy)
                && targeting::can_target_enemy(
                    k.world,
                    k.host,
                    Some(k.unit),
                    enemy,
                    profile.selection,
                )?
            {
                enemies.push(enemy);
            }
        }
        targeting::sort_enemies(k.world, k.host, k.unit, &mut enemies, profile.priority)?;
        Ok(enemies.first().copied())
    }
}
impl Behavior for Rockr {
    fn payload(
        &self,
        world: &World,
        id: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        if !(self.mode == Mode::Second && world.unit(id)?.operator.active_skill) {
            payload.attachment = Some(self.funnel.attachment(payload.hits, false));
            payload.hits = 1;
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Tick if k.up() && self.rock_period > 0. && self.rock_stacks > 0 => {
                let stacks = ((k.world.time - k.world.unit(id)?.deployment.deployed_at + 1e-9)
                    / self.rock_period)
                    .floor()
                    .max(0.) as u32;
                let stacks = stacks.min(self.rock_stacks);
                let old = k
                    .world
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("rockr:rock"))
                    .map_or(0, |b| b.stacks);
                if stacks != old && stacks > 0 {
                    let mut b = Buff {
                        key: Some("rockr:rock".into()),
                        mods: [("atkPct".into(), self.rock_atk)].into(),
                        stacks,
                        ..Default::default()
                    };
                    b.runtime.max_stacks = self.rock_stacks;
                    k.add(id, b)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let target = Self::first_target(k)?;
                let now = k.world.time;
                let s = k.world.unit_mut(id)?.operator.tactical.rockr_mut()?;
                s.over_at = now + self.duration / 2.;
                s.overloaded = false;
                s.drone_cooldown = 0.;
                s.drone_target = target;
                s.drone_scale = s.funnel.scale;
                s.drone_previous = s.funnel.target;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let now = k.world.time;
                let over = {
                    let s = k.world.unit_mut(id)?.operator.tactical.rockr_mut()?;
                    s.drone_cooldown = (s.drone_cooldown - context.dt).max(0.);
                    !s.overloaded && now + 1e-9 >= s.over_at
                };
                if over {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .rockr_mut()?
                        .overloaded = true;
                    k.buff(
                        id,
                        "rockr:overload",
                        [("atkPct".into(), self.over_atk)].into(),
                        f64::INFINITY,
                    )?;
                }
                let old = k.world.unit(id)?.operator.tactical.rockr()?.drone_target;
                let target = if let Some(t) = old.filter(|&t| Host::tac_up(k.world, t)) {
                    if targeting::can_target_enemy(
                        k.world,
                        k.host,
                        Some(id),
                        t,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                    )? {
                        Some(t)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let target = match target {
                    Some(t) => Some(t),
                    None => Self::first_target(k)?,
                };
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .rockr_mut()?
                    .drone_target = target;
                if let Some(target) = target {
                    if k.world.unit(id)?.operator.tactical.rockr()?.drone_cooldown <= 1e-9 {
                        let interval = k.stat(id, "interval")?;
                        let state = k.world.unit_mut(id)?.operator.tactical.rockr_mut()?;
                        let cap = self.funnel.cap
                            * if state.overloaded {
                                self.over_scale
                            } else {
                                1.
                            };
                        let scale = if state.drone_previous == Some(target) {
                            (state.drone_scale + self.funnel.delta).min(cap)
                        } else {
                            self.funnel.initial
                        };
                        state.drone_previous = Some(target);
                        state.drone_scale = scale;
                        state.drone_cooldown = interval;
                        let amount = k.stat(id, "atk")? * k.stat(id, "atkScaleMul")? * scale;
                        k.damage(target, amount, DamageType::Arts, true, "rockr:drone")?;
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let now = k.world.time;
                let over = {
                    let s = k.world.unit_mut(id)?.operator.tactical.rockr_mut()?;
                    s.funnel.scale = s.drone_scale;
                    s.funnel.target = s.drone_previous;
                    s.drone_target = None;
                    let over = if s.overloaded { now - s.over_at } else { 0. };
                    s.overloaded = false;
                    over
                };
                k.remove(id, "rockr:overload")?;
                if over > 0. && context.reason != "death" && k.up() {
                    k.status(id, "stun", over, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
