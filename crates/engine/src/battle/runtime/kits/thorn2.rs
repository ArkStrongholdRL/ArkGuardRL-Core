// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{deploy, spatial};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SPECIAL",
    branch: "alchemist",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_thorn2_2",
        kind: crate::battle::sp::Kind::Charges,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_thorn2_2" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    s.extra.effects.permanent.push(persistent(
        "thorn2:mind",
        [("atkPct".into(), n(t, "atk", 0.))].into(),
    ));
    s.extra.tactics = Tactics::Thorn2(Thorn2 {
        duration: n(bb, "projectile_delay_time", n(bb, "remaining_time", 12.)),
        extend: n(t, "projectile_extend", 0.).min(n(t, "projectile_extend_max", f64::INFINITY)),
        radius: n(bb, "projectile_range", 1.1),
        grow: n(bb, "value", 0.),
        speed: n(bb, "projectile_move_speed", 0.),
        healing: n(bb, "heal_scale", 1.).max(0.),
        scale: n(bb, "atk_scale", 0.),
        regen: n(bb, "hp_recovery_per_sec_ratio_chr", 0.),
        road: n(t1, "cnt", 6.),
        ally: n(t1, "attack_speed_ally", 0.),
        ally_extra: n(t1, "attack_speed_ally_extra", 0.),
        enemy: n(t1, "attack_speed_enemy", 0.),
        enemy_extra: n(t1, "attack_speed_enemy_extra", 0.),
        sp: if raw["isGolden"] == true {
            n(tm, "sp_recovery_per_sec", 0.)
        } else {
            0.
        },
    });
    Ok(())
}
pub(crate) struct Thorn2 {
    duration: f64,
    extend: f64,
    radius: f64,
    grow: f64,
    speed: f64,
    healing: f64,
    scale: f64,
    regen: f64,
    road: f64,
    ally: f64,
    ally_extra: f64,
    enemy: f64,
    enemy_extra: f64,
    sp: f64,
}
#[derive(Clone)]
struct Zone {
    position: [f64; 2],
    velocity: [f64; 2],
    age: f64,
    accumulator: f64,
    duration: f64,
    key: String,
}
#[derive(Default, Clone)]
pub(crate) struct State {
    zones: Vec<Zone>,
    sequence: u64,
    roads: BTreeMap<usize, usize>,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        previous.cloned().unwrap_or_default()
    }
}
impl Thorn2 {
    fn road(k: &mut KitContext<'_>, tile: [i32; 2]) -> Result<usize, String> {
        if !Grid::in_bounds(tile) {
            return Ok(0);
        }
        let key = tile[0] as usize * COLS + tile[1] as usize;
        if let Some(&length) = k
            .world
            .unit(k.unit)?
            .operator
            .tactical
            .thorn2()?
            .roads
            .get(&key)
        {
            return Ok(length);
        }
        let grid = k.host.grid.as_ref().unwrap();
        let rect = grid.rect();
        let length = if !grid.ground_passable(tile, true) {
            0
        } else {
            let mut horizontal = 1;
            let mut vertical = 1;
            for sign in [-1, 1] {
                for distance in 1..COLS as i32 {
                    let p = [tile[0], tile[1] + sign * distance];
                    if p[1] < rect.c0 || p[1] > rect.c1 || !grid.ground_passable(p, true) {
                        break;
                    }
                    horizontal += 1;
                }
                for distance in 1..crate::battle::grid::ROWS as i32 {
                    let p = [tile[0] + sign * distance, tile[1]];
                    if p[0] < rect.r0 || p[0] > rect.r1 || !grid.ground_passable(p, true) {
                        break;
                    }
                    vertical += 1;
                }
            }
            horizontal.max(vertical)
        };
        k.world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .thorn2_mut()?
            .roads
            .insert(key, length);
        Ok(length)
    }
}
impl Behavior for Thorn2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let targets = k.enemies(None, true)?;
                let mut chosen = None;
                for &target in &targets {
                    if !targeting::flying(k.world, target)? {
                        chosen = Some(target);
                        break;
                    }
                }
                let position = if let Some(target) = chosen.or_else(|| targets.first().copied()) {
                    k.world.unit(target)?.position
                } else {
                    let home = k.world.unit(id)?.deployment.tile;
                    let direction = k.host.roster.as_ref().unwrap().entry(id)?.direction;
                    let mut best = None;
                    for key in k.keys()? {
                        let tile = [(key / COLS) as i32, (key % COLS) as i32];
                        let [lateral, forward] =
                            direction.local([tile[0] - home[0], tile[1] - home[1]]);
                        if lateral == 0
                            && forward > 0
                            && best.is_none_or(|(distance, _)| forward > distance)
                        {
                            best = Some((forward, tile));
                        }
                    }
                    let Some((_, tile)) = best else {
                        return Ok(());
                    };
                    tile.map(|v| v as f64)
                };
                let keys = k.keys()?;
                let extended = k.allies(&keys)?.into_iter().any(|a| {
                    a != id
                        && k.world
                            .unit(a)
                            .is_ok_and(|u| u.deployment.kind == deploy::Kind::Operator)
                });
                let source = k.world.unit(id)?.position;
                let delta = [position[0] - source[0], position[1] - source[1]];
                let length = crate::deterministic::math::hypot(delta[0], delta[1]);
                let length = if length == 0. { 1. } else { length };
                let velocity = delta.map(|d| d / length * self.speed);
                let limit = k.host.tactical_limit;
                let state = k.world.unit_mut(id)?.operator.tactical.thorn2_mut()?;
                if state.zones.len() >= limit {
                    return Err("Thorn2 zone capacity exceeded".into());
                }
                state.sequence = state
                    .sequence
                    .checked_add(1)
                    .ok_or("Thorn2 zone identity overflow")?;
                state.zones.push(Zone {
                    position,
                    velocity,
                    age: 0.,
                    accumulator: 0.,
                    duration: self.duration + if extended { self.extend } else { 0. },
                    key: format!("thorn2:{id}:{}", state.sequence),
                });
            }
            KitEvent::Scheduled(due) if due[2] => {
                if k.up() {
                    for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if !Host::tac_up(k.world, target) {
                            continue;
                        }
                        let value = self.ally
                            + if Self::road(k, k.world.unit(target)?.deployment.tile)? as f64
                                >= self.road
                            {
                                self.ally_extra
                            } else {
                                0.
                            };
                        if value != 0. {
                            k.buff(
                                target,
                                "thorn2:vision",
                                [("aspd".into(), value)].into(),
                                0.5,
                            )?;
                        }
                    }
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        if !Host::tac_up(k.world, target) || k.world.unit(target)?.hidden {
                            continue;
                        }
                        let tile = k
                            .world
                            .unit(target)?
                            .position
                            .map(|v| (v + 0.5).floor() as i32);
                        let value = self.enemy
                            + if Self::road(k, tile)? as f64 >= self.road {
                                self.enemy_extra
                            } else {
                                0.
                            };
                        if value != 0. {
                            k.buff(
                                target,
                                "thorn2:vision",
                                [("aspd".into(), value)].into(),
                                0.5,
                            )?;
                        }
                    }
                }
                let mut zones = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.thorn2_mut()?.zones,
                );
                let result: Result<(), String> = (|| {
                    for zone in &mut zones {
                        zone.age += 0.25;
                        zone.accumulator += 0.25;
                        let rect = k.host.grid.as_ref().unwrap().rect();
                        zone.position[0] = (zone.position[0] + zone.velocity[0] * 0.25)
                            .clamp(rect.r0 as f64, rect.r1 as f64);
                        zone.position[1] = (zone.position[1] + zone.velocity[1] * 0.25)
                            .clamp(rect.c0 as f64, rect.c1 as f64);
                        let radius = self.radius + self.grow * zone.age;
                        let mut foes = Vec::new();
                        for target in k.radius(zone.position, radius)? {
                            if !targeting::flying(k.world, target)? {
                                foes.push(target);
                            }
                        }
                        for &target in &foes {
                            k.buff(
                                target,
                                "thorn2:rot",
                                [("healingTakenMul".into(), self.healing)].into(),
                                0.5,
                            )?;
                        }
                        if zone.accumulator + 1e-9 >= 1. {
                            zone.accumulator -= 1.;
                            let attack = k.stat(id, "atk")?;
                            for target in foes {
                                k.damage(
                                    target,
                                    attack * self.scale,
                                    DamageType::Arts,
                                    true,
                                    "alchemy",
                                )?;
                            }
                            if self.regen > 0. {
                                let allies = k.host.roster.as_ref().unwrap().ids().to_vec();
                                for target in spatial::allies_in_radius(
                                    k.world,
                                    &allies,
                                    zone.position,
                                    radius,
                                    None,
                                    false,
                                    k.world.unit_capacity(),
                                )? {
                                    if !k.flag(target, "isolated")? || target == id {
                                        k.buff(
                                            target,
                                            &format!("{}:regen", zone.key),
                                            [("hpRegen".into(), attack * self.regen)].into(),
                                            1. + 2. * DT,
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                    Ok(())
                })();
                zones.retain(|zone| zone.age < zone.duration - 1e-9);
                k.world.unit_mut(id)?.operator.tactical.thorn2_mut()?.zones = zones;
                result?;
                if self.sp > 0.
                    && k.up()
                    && !k
                        .world
                        .unit(id)?
                        .operator
                        .tactical
                        .thorn2()?
                        .zones
                        .is_empty()
                {
                    k.buff(
                        id,
                        "thorn2:module",
                        [("spRecoveryFlat".into(), self.sp)].into(),
                        0.5,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
