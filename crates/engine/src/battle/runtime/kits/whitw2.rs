// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::deterministic::math::{atan2, cos, hypot, sin};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "funnel",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_whitw2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "GDGLOW_SKILL_2",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let additional_drones = n(bb, "attack@cnt", 0.).floor().max(0.);
    if additional_drones > 63. {
        return Err("Released drone count exceeds capacity".into());
    }
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    let mut profile = s.combat.profile;
    profile.no_attack = true;
    s.combat.skill_profile = Some(profile);
    s.extra.tactics = Tactics::Whitw2(Whitw2 {
        funnel: funnel::Parameters::read(raw),
        spread: n(bb, "attack@times", 1.3).max(0.),
        radius: n(bb, "attack@range_radius", 0.9),
        damage: n(bb, "attack@magic_atk_scale", 1.),
        fear: n(bb, "attack@fear", 0.),
        slow: n(bb, "attack@move_speed", 0.),
        drones: additional_drones as usize + 1,
        interval: n(t, "interval", 20.).max(1.),
        cap: n(t, "scale", 1.),
        silence: n(t, "attack@silence_duration", 0.),
        sp: n(t1, "sp", 0.),
        speed: n(t1, "attack_speed", 0.),
    });
    Ok(())
}
pub(crate) struct Whitw2 {
    funnel: funnel::Parameters,
    spread: f64,
    radius: f64,
    damage: f64,
    fear: f64,
    slow: f64,
    drones: usize,
    interval: f64,
    cap: f64,
    silence: f64,
    sp: f64,
    speed: f64,
}
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Spread,
    Seek,
    Chase,
    Lock,
}
struct Drone {
    position: [f64; 2],
    heading: [f64; 2],
    speed: f64,
    age: f64,
    phase: Phase,
    target: Option<Id>,
    cooldown: f64,
    ramp_target: Option<Id>,
    ramp: f64,
    orbit: Option<[f64; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub funnel: funnel::State,
    stage: u32,
    drones: Vec<Drone>,
    accumulator: f64,
    honor: BTreeMap<Id, u64>,
}
impl Whitw2 {
    fn release(k: &mut KitContext<'_>, count: usize) -> Result<Vec<Drone>, String> {
        let unit = k.world.unit(k.unit)?;
        let dir = k
            .host
            .roster
            .as_ref()
            .unwrap()
            .entry(k.unit)?
            .direction
            .vector();
        let angle = atan2(dir[0] as f64, dir[1] as f64);
        Ok((0..count)
            .map(|index| {
                let a = angle + (2. * std::f64::consts::PI * index as f64) / count as f64;
                Drone {
                    position: unit.position,
                    heading: [sin(a), cos(a)],
                    speed: 0.1,
                    age: 0.,
                    phase: Phase::Spread,
                    target: None,
                    cooldown: 0.,
                    ramp_target: None,
                    ramp: 0.,
                    orbit: None,
                }
            })
            .collect())
    }
    fn accelerate(drone: &mut Drone, acceleration: f64, maximum: f64, dt: f64) -> f64 {
        let next = (drone.speed + acceleration * dt).min(maximum);
        let distance = ((drone.speed + next) / 2.) * dt;
        drone.speed = next;
        distance
    }
    fn selectable(k: &mut KitContext<'_>, id: Id) -> Result<bool, String> {
        if !k
            .world
            .unit(id)
            .is_ok_and(|u| u.stats.alive && !u.hidden && u.side == health::Side::Enemy)
        {
            return Ok(false);
        }
        targeting::can_target_enemy(
            k.world,
            k.host,
            Some(k.unit),
            id,
            targeting::Profile {
                can_hit_fly: true,
                ..Default::default()
            },
        )
    }
    fn fly(&self, k: &mut KitContext<'_>, drone: &mut Drone, dt: f64) -> Result<(), String> {
        drone.cooldown = (drone.cooldown - dt).max(0.);
        if drone.phase == Phase::Spread {
            let distance = Self::accelerate(drone, 1.9, 2., dt);
            for axis in 0..2 {
                drone.position[axis] += drone.heading[axis] * distance;
            }
            drone.age += dt;
            if drone.age + 1e-9 < self.spread {
                return Ok(());
            }
            drone.phase = Phase::Seek;
        }
        if matches!(drone.phase, Phase::Lock | Phase::Chase)
            && !Self::selectable(k, drone.target.unwrap())?
        {
            if drone.phase == Phase::Lock {
                let local = [
                    (k.host.rng.next_f64() * 2. - 1.) * 0.75,
                    (k.host.rng.next_f64() * 2. - 1.) * 0.75,
                ];
                let direction = k.host.roster.as_ref().unwrap().entry(k.unit)?.direction;
                let offset = match direction {
                    Direction::Right => local,
                    Direction::Left => [-local[0], -local[1]],
                    Direction::Up => [local[1], -local[0]],
                    Direction::Down => [-local[1], local[0]],
                };
                let target = k.world.unit(drone.target.unwrap())?;
                drone.position = [
                    target.position[0] + offset[0],
                    target.position[1] + offset[1],
                ];
            }
            k.world.unpin_unit(drone.target.take().unwrap())?;
            drone.phase = Phase::Seek;
        }
        if drone.phase == Phase::Seek {
            let mut best = None;
            let mut distance = f64::INFINITY;
            let mut home_distance = f64::INFINITY;
            let own = k.world.unit(k.unit)?.position;
            for target in k.world.unit_ids().collect::<Vec<_>>() {
                if !Self::selectable(k, target)? {
                    continue;
                }
                let point = k.world.unit(target)?.position;
                let d = hypot(point[0] - drone.position[0], point[1] - drone.position[1]);
                let h = hypot(point[0] - own[0], point[1] - own[1]);
                if d < distance - 1e-9 || (d <= distance + 1e-9 && h < home_distance - 1e-9) {
                    best = Some(target);
                    distance = d;
                    home_distance = h;
                }
            }
            let Some(target) = best else {
                let center = *drone.orbit.get_or_insert([
                    drone.position[0] + drone.heading[1] * 0.9,
                    drone.position[1] - drone.heading[0] * 0.9,
                ]);
                let a =
                    atan2(drone.position[0] - center[0], drone.position[1] - center[1]) + dt / 0.9;
                drone.position = [center[0] + 0.9 * sin(a), center[1] + 0.9 * cos(a)];
                drone.heading = [cos(a), -sin(a)];
                return Ok(());
            };
            // Keep the final position available until the drone leaves its target.
            k.world.pin_unit(target)?;
            drone.target = Some(target);
            drone.phase = Phase::Chase;
            drone.speed = 2.;
            drone.orbit = None;
        }
        let target = drone.target.ok_or("Drone lost its chase target")?;
        if drone.phase == Phase::Chase {
            let distance = Self::accelerate(drone, 1., 4., dt);
            if body::distance(k.world.unit(target)?, drone.position) > distance + 1e-9 {
                let point = k.world.unit(target)?.position;
                let delta = [point[0] - drone.position[0], point[1] - drone.position[1]];
                let length = hypot(delta[0], delta[1]);
                if length > 1e-9 {
                    drone.heading = [delta[0] / length, delta[1] / length];
                }
                for axis in 0..2 {
                    drone.position[axis] += drone.heading[axis] * distance;
                }
                return Ok(());
            }
            drone.phase = Phase::Lock;
            if self.fear > 0. {
                k.status(target, "fear", self.fear, None)?;
            }
        }
        drone.position = k.world.unit(target)?.position;
        if drone.cooldown > 1e-9 || !k.up() || !Self::selectable(k, target)? {
            return Ok(());
        }
        drone.cooldown = k.stat(k.unit, "interval")?;
        let stage = k.world.unit(k.unit)?.operator.tactical.whitw2()?.stage;
        let cap = self.funnel.cap * if stage >= 1 { self.cap } else { 1. };
        drone.ramp = if drone.ramp_target == Some(target) {
            (drone.ramp + self.funnel.delta).min(cap)
        } else {
            self.funnel.initial
        };
        drone.ramp_target = Some(target);
        let amount = k.stat(k.unit, "atk")? * k.stat(k.unit, "atkScaleMul")? * drone.ramp;
        k.damage(target, amount, DamageType::Arts, false, "droneAttack")?;
        Ok(())
    }
}
impl Behavior for Whitw2 {
    fn payload(
        &self,
        w: &World,
        id: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        if !w.unit(id)?.operator.active_skill {
            let stage = w.unit(id)?.operator.tactical.whitw2()?.stage;
            let mut parameters = self.funnel;
            if stage >= 1 {
                parameters.cap *= self.cap;
            }
            payload.attachment = Some(parameters.attachment(if stage >= 3 { 2 } else { 1 }, false));
            payload.hits = 1;
        }
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
                let count = self.drones
                    + usize::from(k.world.unit(id)?.operator.tactical.whitw2()?.stage >= 3);
                let drones = Self::release(k, count)?;
                let state = k.world.unit_mut(id)?.operator.tactical.whitw2_mut()?;
                state.drones = drones;
                state.accumulator = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let drones = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.whitw2_mut()?.drones,
                );
                for drone in drones {
                    if let Some(target) = drone.target {
                        k.world.unpin_unit(target)?;
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && k.up() && k.active()? => {
                let mut drones = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.whitw2_mut()?.drones,
                );
                let result = (|| -> Result<bool, String> {
                    let count = self.drones
                        + usize::from(k.world.unit(id)?.operator.tactical.whitw2()?.stage >= 3);
                    if drones.len() < count {
                        drones.extend(Self::release(k, count - drones.len())?);
                    }
                    for drone in &mut drones {
                        if !k.up() || !k.active()? {
                            return Ok(false);
                        }
                        self.fly(k, drone, context.dt)?;
                    }
                    if !k.up() || !k.active()? {
                        return Ok(false);
                    }
                    let mut near = BTreeSet::new();
                    for drone in &drones {
                        for enemy in k.radius(drone.position, self.radius)? {
                            if Self::selectable(k, enemy)? {
                                near.insert(enemy);
                            }
                        }
                    }
                    if self.slow != 0. {
                        for &enemy in &near {
                            k.buff(
                                enemy,
                                "whitw2:slow",
                                [("moveMul".into(), (1. + self.slow).max(0.))].into(),
                                0.2,
                            )?;
                        }
                    }
                    let accumulator =
                        k.world.unit(id)?.operator.tactical.whitw2()?.accumulator + context.dt;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .whitw2_mut()?
                        .accumulator = accumulator;
                    if accumulator + 1e-9 >= 1. {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .whitw2_mut()?
                            .accumulator -= 1.;
                        for enemy in near {
                            if !k.up() || !k.active()? {
                                return Ok(false);
                            }
                            let amount = k.stat(id, "atk")? * self.damage;
                            k.damage(enemy, amount, DamageType::Arts, true, "dot")?;
                        }
                    }
                    Ok(k.up() && k.active()?)
                })();
                if matches!(result, Ok(true)) {
                    k.world.unit_mut(id)?.operator.tactical.whitw2_mut()?.drones = drones;
                } else {
                    for drone in drones {
                        if let Some(target) = drone.target {
                            k.world.unpin_unit(target)?;
                        }
                    }
                }
                result?;
            }
            KitEvent::Scheduled(due) if due[4] && k.up() => {
                let stage = ((k.world.time - k.world.unit(id)?.deployment.deployed_at + 1e-6)
                    / self.interval)
                    .floor()
                    .clamp(0., 3.) as u32;
                k.world.unit_mut(id)?.operator.tactical.whitw2_mut()?.stage = stage;
            }
            KitEvent::Landed { event, request }
                if event.source == Some(id)
                    && self.silence > 0.
                    && k.world.unit(id)?.operator.tactical.whitw2()?.stage >= 2
                    && request.hit.damage_type != DamageType::Elemental
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && Host::tac_up(k.world, event.target) =>
            {
                k.status(event.target, "silence", self.silence, None)?;
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::Deploy,
                skill,
            } if self.sp > 0.
                && k.world.unit(*unit)?.deployment.kind
                    == crate::battle::deploy::Kind::Operator
                && k.world.unit(*unit)?.owner == k.world.unit(id)?.owner
                && k.faction(*unit, "siracusaShip", &[])? =>
            {
                let amount = self.sp;
                k.host.with_kit_buffs(k.world, k.buffs, |host, w| {
                    skill.gain(w, host, amount, "talent", false, false)
                })?;
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::SkillStart,
                ..
            } if self.speed != 0.
                && k.world.unit(*unit)?.deployment.kind
                    == crate::battle::deploy::Kind::Operator
                && k.world.unit(*unit)?.owner == k.world.unit(id)?.owner
                && k.faction(*unit, "siracusaShip", &[])? =>
            {
                let sequence = k.world.unit(*unit)?.deployment.deploy_seq;
                let previous = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .whitw2_mut()?
                    .honor
                    .insert(*unit, sequence);
                if previous != Some(sequence) {
                    k.buff(
                        *unit,
                        "whitw2:honor",
                        [("aspd".into(), self.speed)].into(),
                        f64::INFINITY,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
