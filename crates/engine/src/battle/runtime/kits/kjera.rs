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
            id: "skchr_kjera_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_atk_up[3]",
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
        "skcom_atk_up[3]" => Mode::First,
        "skchr_kjera_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    s.extra.tactics = Tactics::Kjera(Kjera {
        mode: mode,
        funnel: funnel::Parameters::read(raw),
        drones: 1 + n(bb, "attack@cnt", 0.).max(0.) as usize,
        prob: n(bb, "attack@prob", 0.),
        cold: n(bb, "attack@cold", 0.),
        low_count: n(t, "cnt", 2.),
        atk: n(t, "atk", 0.),
        low_atk: n(t, "kjera_t_1[high].atk", n(t, "atk", 0.)),
        base_grid: tactical_grid(&raw["rangeGrid"], false)?,
    });
    Ok(())
}
pub(crate) struct Kjera {
    pub mode: Mode,
    pub funnel: funnel::Parameters,
    pub drones: usize,
    pub prob: f64,
    pub cold: f64,
    pub low_count: f64,
    pub atk: f64,
    pub low_atk: f64,
    pub base_grid: Vec<[i32; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub funnel: funnel::State,
    pub released: funnel::State,
    pub drones: Vec<Option<Id>>,
    pub cooldown: f64,
}
impl Behavior for Kjera {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Before(_)) {
            -100
        } else {
            0
        }
    }

    fn payload(
        &self,
        world: &World,
        id: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        if !(self.mode == Mode::Second && world.unit(id)?.operator.active_skill) {
            payload.attachment = Some(self.funnel.attachment(payload.hits, true));
            payload.hits = 1;
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Before(context) if context.attacker == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .kjera_mut()?
                    .funnel
                    .map
                    .retain(|target, _| context.targets.contains(target));
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                let keys = k.range(&self.base_grid)?;
                let grid = k.host.grid.as_ref().ok_or("Kjera deployment needs grid")?;
                let low = keys
                    .into_iter()
                    .filter(|&key| grid.tiles()[key].height == "LOW")
                    .count();
                let attack = if low as f64 >= self.low_count {
                    self.low_atk
                } else {
                    self.atk
                };
                k.buff(
                    id,
                    "talent:kjera_brow",
                    [("atkPct".into(), attack)].into(),
                    f64::INFINITY,
                )?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                if self.drones > crate::battle::operators::Combat::MAX_HITS as usize {
                    return Err("Released drone capacity exceeded".into());
                }
                let s = k.world.unit_mut(id)?.operator.tactical.kjera_mut()?;
                s.released = s.funnel.clone();
                s.drones = vec![None; self.drones];
                s.cooldown = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let valid: BTreeSet<_> = k
                    .world
                    .unit_ids()
                    .filter(|&target| k.world.unit(target).is_ok_and(|u| u.stats.alive))
                    .collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .kjera_mut()?
                    .released
                    .map
                    .retain(|target, _| valid.contains(target));
                let mut locks = k.world.unit(id)?.operator.tactical.kjera()?.drones.clone();
                let candidates = k.enemies(None, true)?;
                for lock in &mut locks {
                    if let Some(target) = *lock {
                        if !Host::tac_up(k.world, target)
                            || !targeting::can_target_enemy(
                                k.world,
                                k.host,
                                Some(id),
                                target,
                                targeting::Profile {
                                    can_hit_fly: true,
                                    ..Default::default()
                                },
                            )?
                        {
                            *lock = None;
                        }
                    }
                }
                for index in 0..locks.len() {
                    if locks[index].is_some() {
                        continue;
                    }
                    locks[index] = candidates
                        .iter()
                        .copied()
                        .find(|e| !locks.contains(&Some(*e)))
                        .or_else(|| candidates.first().copied())
                        .or_else(|| locks.iter().flatten().copied().next());
                }
                let due = {
                    let s = k.world.unit_mut(id)?.operator.tactical.kjera_mut()?;
                    s.drones = locks.clone();
                    s.cooldown = (s.cooldown - context.dt).max(0.);
                    s.cooldown <= 1e-9 && locks.iter().any(Option::is_some)
                };
                if due {
                    let interval = k.stat(id, "interval")?;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .kjera_mut()?
                        .cooldown = interval;
                    for target in locks.into_iter().flatten() {
                        if !Host::tac_up(k.world, target) {
                            continue;
                        }
                        let time = k.world.time;
                        let scale = k
                            .world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .kjera_mut()?
                            .released
                            .step(target, time, self.funnel, true);
                        let amount = k.stat(id, "atk")? * k.stat(id, "atkScaleMul")? * scale;
                        k.damage(target, amount, DamageType::Arts, true, "kjera:drone")?;
                        if Host::tac_up(k.world, target)
                            && self.cold > 0.
                            && k.host.rng.chance(self.prob)
                        {
                            k.status(target, "cold", self.cold, None)?;
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let s = k.world.unit_mut(id)?.operator.tactical.kjera_mut()?;
                s.funnel = std::mem::take(&mut s.released);
                s.drones.clear();
            }
            KitEvent::Landed {
                event: landed,
                request,
            } if landed.source == Some(id)
                && self.mode == Mode::Second
                && k.active()?
                && request.is_attack
                && !request.is_skill
                && landed.target != id =>
            {
                if self.cold > 0.
                    && Host::tac_up(k.world, landed.target)
                    && k.host.rng.chance(self.prob)
                {
                    k.status(landed.target, "cold", self.cold, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
