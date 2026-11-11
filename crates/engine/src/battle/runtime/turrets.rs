// SPDX-License-Identifier: GPL-3.0-or-later
//! Map turrets share targeting, projectile retention and damage settlement.
use super::devices::{device_position, num, Hooks};
use super::*;
use crate::battle::{
    cadence,
    damage::{Damage, DamageType},
    deploy, projectiles,
    stats::{Base, UnitStats},
    targeting,
};

pub(super) struct Turret {
    id: Id,
    alias: String,
    keys: Vec<usize>,
    base_aspd: f64,
    aspd_per: f64,
    aspd_max: f64,
    fragile_per: f64,
    fragile_max: f64,
    fragile_duration: f64,
    cooldown: f64,
}
#[derive(Clone, Copy)]
pub(super) struct Shot {
    source: Id,
    attack: u64,
    fragile: f64,
    duration: f64,
}
impl Core {
    pub(super) fn spawn_map_unit(
        &mut self,
        tile: [i32; 2],
        stats: &Value,
        crate_hp: Option<f64>,
    ) -> Result<Id, String> {
        let base = Base(
            [
                (
                    "maxHp",
                    crate_hp
                        .unwrap_or_else(|| num(&stats["maxHp"], 100.))
                        .max(1.),
                ),
                ("atk", num(&stats["atk"], 0.).max(0.)),
                ("def", num(&stats["def"], 0.).max(0.)),
                ("res", num(&stats["res"], 0.).clamp(0., 100.)),
                ("aspd", num(&stats["aspd"], 100.).max(1.)),
                ("bat", num(&stats["bat"], 1.).max(0.001)),
                ("blockCnt", if crate_hp.is_some() { 99. } else { 0. }),
                ("cost", 0.),
                ("spRecovery", 0.),
                ("tauntLevel", -1.),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect(),
        );
        let id = self.world.next_unit_id()?;
        let mut u = health::Unit::new(UnitStats::new(base), health::Side::Ally);
        u.stats.alive = false;
        u.ground = true;
        u.owner = if crate_hp.is_none() { Some(1) } else { None };
        u.deployment.kind = deploy::Kind::Device;
        u.deployment.home = tile;
        self.world.insert_unit(id, u)?;
        let mut roster = self.host.roster.take().unwrap();
        let mut buffs = self.host.buffs.take().unwrap();
        let result = (|| {
            roster.deployment_mut().register(id, &self.world)?;
            roster.deployment_mut().deploy(
                &mut self.world,
                &mut Hooks {
                    host: &mut self.host,
                    buffs: &mut buffs,
                },
                id,
                deploy::Options::default(),
            )
        })();
        self.host.roster = Some(roster);
        self.host.buffs = Some(buffs);
        result?;
        self.host.devices.ids.push(id);
        if crate_hp.is_some() {
            self.host
                .grid
                .as_mut()
                .unwrap()
                .set_obstacle(tile, true, super::super::grid::CRATE)?;
        }
        Ok(id)
    }
    fn turret_tile(&self, origin: [i32; 2]) -> Result<Option<[i32; 2]>, String> {
        let grid = self.host.grid.as_ref().unwrap();
        let mut best: Option<([f64; 4], [i32; 2])> = None;
        let rect = grid.rect();
        for row in rect.r0..=rect.r1 {
            for col in rect.c0..=rect.c1 {
                let at = [row, col];
                if self
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .deployment()
                    .reserved(&self.world, at)?
                {
                    continue;
                }
                let exact = at == origin;
                let tile = &grid.tiles()[row as usize * COLS + col as usize];
                if !exact && tile.build != "NONE" {
                    continue;
                }
                let score = [
                    if exact { 0. } else { 1. },
                    if !exact && grid.ground_passable(at, true) {
                        1.
                    } else {
                        0.
                    },
                    crate::deterministic::math::hypot(
                        (row - origin[0]) as f64,
                        (col - origin[1]) as f64,
                    ),
                    (row as usize * COLS + col as usize) as f64,
                ];
                let less = best.as_ref().is_none_or(|(b, _)| {
                    for i in 0..4 {
                        if score[i] < b[i] - 1e-9 {
                            return true;
                        }
                        if score[i] > b[i] + 1e-9 {
                            return false;
                        }
                    }
                    false
                });
                if less {
                    best = Some((score, at));
                }
            }
        }
        Ok(best.map(|(_, at)| at))
    }
    pub(super) fn start_map_devices(&mut self) -> Result<(), String> {
        for (tile, hp) in self.host.devices.added.clone() {
            if self
                .host
                .roster
                .as_ref()
                .unwrap()
                .deployment()
                .occupant(tile)
                .is_none()
            {
                self.spawn_map_unit(tile, &Value::Null, Some(hp))?;
            }
        }
        let stage = self.host.devices.stage.clone();
        for d in stage["devices"].as_array().into_iter().flatten() {
            if d["role"] != "turret" && d["key"] != "trap_1104_aclasert" {
                continue;
            }
            let origin = device_position(d);
            let alias = d["alias"].as_str().map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{}@{},{}",
                    d["key"].as_str().unwrap_or(""),
                    origin[0],
                    origin[1]
                )
            });
            let on = self
                .host
                .devices
                .config
                .device_overrides
                .get(&alias)
                .copied()
                .unwrap_or_else(|| d["active"].as_bool().unwrap_or(d["hidden"] != true));
            if !on || self.host.devices.turrets.iter().any(|t| t.alias == alias) {
                continue;
            }
            let raw = d.get("raw").unwrap_or(d);
            let mut keys = Vec::new();
            for p in raw["rangeTiles"].as_array().into_iter().flatten() {
                let at = [
                    p[0].as_i64().unwrap_or(-1) as i32,
                    p[1].as_i64().unwrap_or(-1) as i32,
                ];
                if self.host.grid.as_ref().unwrap().in_rect(at) {
                    let key = at[0] as usize * COLS + at[1] as usize;
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
            if keys.is_empty() {
                continue;
            }
            let Some(tile) = self.turret_tile(origin)? else {
                continue;
            };
            let stats = &raw["stats"];
            let id = self.spawn_map_unit(tile, stats, None)?;
            let skill = &raw["skill"];
            let bb = &skill["bb"];
            let text = skill["desc"]
                .as_str()
                .or(skill["description"].as_str())
                .unwrap_or("");
            let duration = text
                .split_once("持续")
                .and_then(|(_, s)| s.split_once('秒'))
                .and_then(|(s, _)| s.parse::<f64>().ok())
                .unwrap_or(0.);
            self.host.devices.turrets.push(Turret {
                id,
                alias,
                keys,
                base_aspd: self.world.unit(id)?.stats.base_number("aspd"),
                aspd_per: num(&bb["attack_speed_per_stack"], 0.),
                aspd_max: num(&bb["max_attack_speed"], f64::INFINITY),
                fragile_per: num(&bb["damage_scale_per_stack"], 0.),
                fragile_max: num(&bb["max_damage_scale"], f64::INFINITY),
                fragile_duration: duration,
                cooldown: 0.,
            });
        }
        Ok(())
    }
    pub(super) fn tick_turrets(&mut self, dt: f64) -> Result<(), String> {
        for i in 0..self.host.devices.turrets.len() {
            let t = &mut self.host.devices.turrets[i];
            let id = t.id;
            if !self.world.unit(id)?.stats.alive {
                continue;
            }
            t.cooldown = cadence::countdown(t.cooldown, dt);
            if t.cooldown > 0.
                || self
                    .world
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("stun")
            {
                continue;
            }
            let layers = self.host.devices.config.top_layers;
            let aspd = t.base_aspd + (layers * t.aspd_per).min(t.aspd_max);
            if (self.world.unit(id)?.stats.base_number("aspd") - aspd).abs() > 1e-9 {
                self.world.unit_mut(id)?.stats.edit_base(|b| {
                    b.0.insert("aspd".into(), aspd);
                });
            }
            let keys = t.keys.clone();
            let mut targets = Vec::new();
            for enemy in self.host.spawns.as_ref().unwrap().live().to_vec() {
                if super::super::body::in_range(self.world.unit(enemy)?, &keys)
                    && targeting::can_target_enemy(
                        &mut self.world,
                        &mut self.host,
                        Some(id),
                        enemy,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                    )?
                {
                    targets.push(enemy);
                }
            }
            targeting::sort_enemies(
                &mut self.world,
                &mut self.host,
                id,
                &mut targets,
                targeting::Priority::None,
            )?;
            let Some(&target) = targets.first() else {
                self.host.devices.turrets[i].cooldown = 0.;
                continue;
            };
            let attack = self.world.next_attack_id()?;
            let time = self.world.time;
            let u = self.world.unit_mut(id)?;
            u.attacks = u
                .attacks
                .checked_add(1)
                .ok_or("Turret attack count exhausted")?;
            u.last_attack_at = time;
            u.engaged = true;
            let t = &mut self.host.devices.turrets[i];
            let payload = Shot {
                source: id,
                attack,
                fragile: (1. + layers * t.fragile_per).min(t.fragile_max) - 1.,
                duration: t.fragile_duration,
            };
            t.cooldown = self
                .world
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("interval");
            let from = self.world.unit(id)?.position;
            self.host.flights.as_mut().unwrap().add(
                &mut self.world,
                projectiles::Launch {
                    from,
                    source: Some(id),
                    target: Some(target),
                    to: None,
                    speed: 11.,
                    hit_dead: false,
                    max_age: 10.,
                    on_hit: true,
                    visual: "bolt".into(),
                    speed_changeable: None,
                    removable: None,
                    payload: allies::Shot::Turret(payload),
                },
            )?;
        }
        Ok(())
    }
}
impl Host {
    pub(super) fn turret_impact(
        &mut self,
        w: &mut World,
        shot: Shot,
        target: Option<Id>,
    ) -> Result<(), String> {
        let Some(target) = target else { return Ok(()) };
        if !w.unit(target)?.stats.alive {
            return Ok(());
        }
        let amount = w
            .unit_mut(shot.source)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("atk");
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(shot.source),
                credit: Some(shot.source),
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
        q.is_attack = true;
        q.attack_id = Some(shot.attack);
        let mut buffs = self.buffs.take().unwrap();
        let result = (|| {
            self.enemy_damage(w, &mut buffs, &mut q)?;
            if shot.fragile > 0. && shot.duration > 0. && w.unit(target)?.stats.alive {
                self.apply_status(
                    w,
                    &mut buffs,
                    target,
                    "fragile",
                    super::super::status::Options {
                        source: Some(shot.source),
                        duration: shot.duration,
                        value: Some(shot.fragile),
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        })();
        self.buffs = Some(buffs);
        result
    }
}
