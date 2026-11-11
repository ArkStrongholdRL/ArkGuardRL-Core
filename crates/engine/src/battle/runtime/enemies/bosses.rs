// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{
        bosses::{Lion, Memory},
        casters::{BuffEffect, Spell},
        Boss,
    },
    targeting,
};
mod ensemble;
mod gun;
mod quintus;
mod spawning;
mod tasks;
pub(in crate::battle::runtime) use tasks::BossTask;
mod helm;
pub(in crate::battle::runtime) use helm::{BLADE_RETURN, PART_DOT};
pub(in crate::battle::runtime) const SURGE_TICK: u64 = 1712;
const EQUIPMENT: [&str; 3] = [
    "enemy_10028_vtswd",
    "enemy_10029_vtshld",
    "enemy_10030_vtwand",
];
impl Host {
    fn boss_config(&self, id: Id) -> Option<Boss> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Boss(b) => Some(b),
            _ => None,
        }
    }
    fn boss_targets(&mut self, w: &mut World, id: Id, radius: f64) -> Result<Vec<Id>, String> {
        let mut targets = if radius.is_infinite() {
            self.roster.as_ref().unwrap().ids().to_vec()
        } else {
            enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?
        };
        let mut out = vec![];
        for t in targets.drain(..) {
            if targeting::can_target_ally(w, id, t, true, Default::default())? {
                out.push(t);
            }
        }
        Ok(out)
    }
    fn boss_area(&mut self, w: &mut World, id: Id) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for t in self.roster.as_ref().unwrap().ids().to_vec() {
            if targeting::area_selectable(w, Some(id), t)? {
                out.push(t);
            }
        }
        Ok(out)
    }
    fn lion_range(w: &World, id: Id) -> Result<f64, String> {
        let radius = w.unit(id)?.stats.base_number("rangeRadius");
        Ok(if radius != 0. { radius } else { 3.4 })
    }
    pub(super) fn boss_condition(
        &mut self,
        w: &mut World,
        id: Id,
        spell: Spell,
    ) -> Result<Option<bool>, String> {
        if let Some(ready) = self.ensemble_condition(w, spell) {
            return Ok(Some(ready));
        }
        if let Some(ready) = self.quintus_condition(id, spell) {
            return Ok(Some(ready));
        }
        if let Some(ready) = self.gun_condition(w, id, spell)? {
            return Ok(Some(ready));
        }
        if let Some(ready) = self.helm_condition(w, id, spell)? {
            return Ok(Some(ready));
        }
        Ok(Some(match spell {
            Spell::Decree { .. } => {
                let Some(Boss::Lion(lion)) = self.boss_config(id) else {
                    return Err("Decree requires a lion kit".into());
                };
                let Memory::Lion { sp, .. } = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                sp >= lion.cost
                    && !self
                        .boss_targets(w, id, Self::lion_range(w, id)?)?
                        .is_empty()
            }
            Spell::Surge { .. } => !self.boss_targets(w, id, f64::INFINITY)?.is_empty(),
            Spell::Repel { .. } => true,
            _ => return Ok(None),
        }))
    }
    pub(super) fn boss_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
        grid: Option<&Grid>,
    ) -> Result<(), String> {
        match spell {
            spell @ (Spell::PipeStrike { .. } | Spell::Ensemble) => {
                self.ensemble_fire(w, r, id, spell)?
            }
            spell @ (Spell::Tide { .. }
            | Spell::Rockfall { .. }
            | Spell::Tentacles { .. }
            | Spell::Species { .. }) => self.quintus_fire(w, r, id, spell)?,
            spell @ (Spell::GunCharge | Spell::SpringDash { .. }) => {
                self.gun_fire(w, r, id, spell)?
            }
            spell @ (Spell::HelmShell | Spell::HelmDrones { .. } | Spell::BladeDive) => {
                self.helm_fire(w, r, id, spell)?
            }
            Spell::Decree { scales } => self.lion_decree(w, r, id, scales, grid)?,
            Spell::Repel { scale, loss } => {
                for t in self.boss_area(w, id)? {
                    let amount = Self::tac_stat(w, id, "atk")? * scale;
                    let mut q = ability_damage(Some(id), t, amount, DamageType::Arts);
                    self.enemy_damage(w, r, &mut q)?;
                }
                self.lose_lp(loss)?;
            }
            Spell::Surge {
                max,
                duration,
                scale,
            } => {
                let Some(Boss::Deer(deer)) = self.boss_config(id) else {
                    return Err("Surge requires a deer kit".into());
                };
                let Memory::Deer { low, .. } = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                let mut targets = self.boss_targets(w, id, f64::INFINITY)?;
                targeting::sort_allies(w, id, &mut targets, Default::default())?;
                for t in targets.into_iter().take(if low { deer.surge } else { max }) {
                    self.apply_status(
                        w,
                        r,
                        t,
                        "stun",
                        status::Options {
                            duration,
                            source: Some(id),
                            ..Default::default()
                        },
                    )?;
                    let mut b = buff("boss:surge", &[]);
                    b.runtime.duration = duration;
                    b.runtime.interval = 1.;
                    b.runtime.visible = true;
                    b.runtime.source = Some(id);
                    b.runtime.tick_payload = scale;
                    b.runtime.on_tick = Some(SURGE_TICK);
                    r.add(
                        w,
                        self,
                        t,
                        buffs::Spec {
                            buff: b,
                            ..Default::default()
                        }
                        .retaining_unit(id),
                    )?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn lion_equipment(&self, w: &World) -> Result<Vec<Id>, String> {
        let spawns = self.spawns.as_ref().unwrap();
        let mut out = vec![];
        for &t in spawns.live() {
            if w.unit(t)?.stats.alive && EQUIPMENT.contains(&spawns.definition(t)?.key.as_str()) {
                out.push(t);
            }
        }
        out.sort_by_key(|&t| w.unit(t).unwrap().targeting.spawn_seq);
        Ok(out)
    }
    fn lion_decree(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        scales: [f64; 2],
        grid: Option<&Grid>,
    ) -> Result<(), String> {
        let targets = self.boss_targets(w, id, Self::lion_range(w, id)?)?;
        let Some(target) = self.rng.pick(&targets).copied() else {
            return Ok(());
        };
        let Memory::Lion {
            advanced,
            next_anim,
            wand,
            ..
        } = self.content.as_ref().unwrap().boss(id)
        else {
            unreachable!()
        };
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Lion {
                sp: 0.,
                advanced,
                next_anim,
                wand,
            },
        );
        let tile = w.unit(target)?.deployment.tile;
        for t in self.caster_cross_area(w, id, tile)? {
            let amount = Self::tac_stat(w, id, "atk")? * scales[usize::from(advanced)];
            let mut q = ability_damage(Some(id), t, amount, DamageType::Physical);
            self.enemy_damage(w, r, &mut q)?;
        }
        let equipment = self.lion_equipment(w)?;
        let tiles = [
            tile,
            [tile[0] + 1, tile[1]],
            [tile[0] - 1, tile[1]],
            [tile[0], tile[1] + 1],
            [tile[0], tile[1] - 1],
        ];
        let grid = grid
            .or(self.grid.as_deref())
            .ok_or("Missing equipment grid")?;
        let mut free = vec![];
        for p in tiles {
            if !inside(grid.rect(), p)
                || grid.tiles()[p[0] as usize * crate::battle::grid::COLS + p[1] as usize].height
                    != "LOW"
                || !grid.ground_passable(p, false)
                || self
                    .roster
                    .as_ref()
                    .unwrap()
                    .deployment()
                    .occupant(p)
                    .is_some()
            {
                continue;
            }
            if equipment.iter().any(|&t| {
                w.unit(t)
                    .is_ok_and(|u| u.position.map(|v| (v + 0.5).floor() as i32) == p)
            }) {
                continue;
            }
            free.push(p);
        }
        if let Some(position) = self.rng.pick(&free).copied() {
            let key = *self.rng.pick(&EQUIPMENT).unwrap();
            self.with_spawns(w, r, |spawns, context, world| {
                spawns.spawn_part(world, context, key, position.map(|v| v as f64))
            })?;
            self.rebuild_index(w)?;
        }
        Ok(())
    }
    fn lion_animate(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        lion: Lion,
    ) -> Result<(), String> {
        let Memory::Lion { advanced, .. } = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        let store = lion.stores[usize::from(advanced)];
        let equipment = self.lion_equipment(w)?;
        let count = if store.count == 0 {
            equipment.len()
        } else {
            store.count
        };
        for &item in equipment.iter().skip(equipment.len().saturating_sub(count)) {
            match self.spawns.as_ref().unwrap().definition(item)?.key.as_str() {
                "enemy_10028_vtswd" => {
                    let targets = self.boss_targets(w, id, Self::lion_range(w, id)?)?;
                    if let Some(t) = self.rng.pick(&targets).copied() {
                        let amount = Self::tac_stat(w, id, "atk")? * store.scale;
                        let mut q = ability_damage(Some(id), t, amount, DamageType::Physical);
                        self.enemy_damage(w, r, &mut q)?;
                    }
                }
                "enemy_10029_vtshld" => {
                    let dx = w.unit(item)?.position[1] - w.unit(id)?.position[1];
                    let side = if dx == 0. { -1. } else { dx.signum() };
                    let mut b = buff("boss:vest", &[]);
                    b.shield = store.shield;
                    b.runtime.persist = true;
                    b.runtime.visible = true;
                    b.runtime.enemy_effect = Some(BuffEffect::Vest {
                        side,
                        cut: store.cut,
                    });
                    self.enemy_buff(w, r, id, b)?;
                }
                "enemy_10030_vtwand" => {
                    let Memory::Lion {
                        sp,
                        advanced,
                        next_anim,
                        ..
                    } = self.content.as_ref().unwrap().boss(id)
                    else {
                        unreachable!()
                    };
                    self.content.as_mut().unwrap().set_boss(
                        id,
                        Memory::Lion {
                            sp,
                            advanced,
                            next_anim,
                            wand: true,
                        },
                    );
                }
                _ => unreachable!(),
            }
            self.kill_enemy(item, None, r, w)?;
        }
        Ok(())
    }
    pub(super) fn enemy_boss_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        boss: Boss,
        dt: f64,
    ) -> Result<(), String> {
        match boss {
            Boss::Pipe(config) => self.pipe_tick(w, r, id, config, dt)?,
            Boss::Echo(config) => self.enemy_caster_tick(w, r, id, config.caster, dt)?,
            Boss::Quintus(config) => self.quintus_tick(w, r, id, config, dt)?,
            Boss::Gun(config) => self.gun_tick(w, r, id, config, dt)?,
            Boss::Spring(config) => self.spring_tick(w, r, id, config, dt)?,
            Boss::Helm(_) | Boss::Blade(_) | Boss::Shell { .. } => {
                self.helm_tick(w, r, id, boss, dt)?
            }
            Boss::Equipment => {}
            Boss::Lion(lion) => {
                let Memory::Lion {
                    mut sp,
                    mut advanced,
                    mut next_anim,
                    wand,
                } = self.content.as_ref().unwrap().boss(id)
                else {
                    unreachable!()
                };
                if !advanced
                    && w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? <= lion.advance
                {
                    advanced = true;
                    let mut b = buff(
                        "boss:advance",
                        &[("defPct", lion.defense), ("resFlat", lion.resistance)],
                    );
                    b.runtime.persist = true;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, id, b)?;
                }
                if self.enemy_can_cast(w, id, false)? {
                    sp += dt;
                    if self.lion_equipment(w)?.is_empty() {
                        next_anim = None;
                    } else {
                        let deadline = *next_anim.get_or_insert(w.time + 2.);
                        if w.time >= deadline {
                            self.content.as_mut().unwrap().set_boss(
                                id,
                                Memory::Lion {
                                    sp,
                                    advanced,
                                    next_anim,
                                    wand,
                                },
                            );
                            self.lion_animate(w, r, id, lion)?;
                            let Memory::Lion { wand: armed, .. } =
                                self.content.as_ref().unwrap().boss(id)
                            else {
                                unreachable!()
                            };
                            next_anim =
                                Some(w.time + if lion.cost.is_finite() { lion.cost } else { 2. });
                            self.content.as_mut().unwrap().set_boss(
                                id,
                                Memory::Lion {
                                    sp,
                                    advanced,
                                    next_anim,
                                    wand: armed,
                                },
                            );
                            return self.enemy_caster_tick(w, r, id, lion.caster, dt);
                        }
                    }
                }
                self.content.as_mut().unwrap().set_boss(
                    id,
                    Memory::Lion {
                        sp,
                        advanced,
                        next_anim,
                        wand,
                    },
                );
                self.enemy_caster_tick(w, r, id, lion.caster, dt)?;
            }
            Boss::Deer(deer) => {
                let Memory::Deer {
                    mut low,
                    mut acc,
                    held_until,
                } = self.content.as_ref().unwrap().boss(id)
                else {
                    unreachable!()
                };
                if !low && w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? < 0.5 {
                    low = true;
                    let mut b = buff(
                        "boss:madness",
                        &[
                            ("physTakenMul", 1. - deer.resistance),
                            ("artsTakenMul", 1. - deer.resistance),
                        ],
                    );
                    b.runtime.persist = true;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, id, b)?;
                }
                if self.enemy_can_cast(w, id, false)? {
                    acc += dt;
                    if acc >= Self::tac_stat(w, id, "interval")? && w.time >= held_until {
                        let mut targets = self.boss_targets(w, id, f64::INFINITY)?;
                        targeting::sort_allies(w, id, &mut targets, Default::default())?;
                        if !targets.is_empty() {
                            acc = 0.;
                            self.content.as_mut().unwrap().set_boss(
                                id,
                                Memory::Deer {
                                    low,
                                    acc,
                                    held_until,
                                },
                            );
                            if let Some(clip) = self.caster_turn(w, r, id, false, None)? {
                                self.content.as_mut().unwrap().set_boss(
                                    id,
                                    Memory::Deer {
                                        low,
                                        acc,
                                        held_until: w.time + clip,
                                    },
                                );
                                return self.enemy_caster_tick(w, r, id, deer.caster, dt);
                            }
                            let mut columns = vec![];
                            for t in targets {
                                let col = w.unit(t)?.deployment.tile[1];
                                if !columns.contains(&col) {
                                    columns.push(col);
                                }
                                if columns.len() >= if low { deer.columns } else { 1 } {
                                    break;
                                }
                            }
                            for col in columns {
                                for t in self.boss_area(w, id)? {
                                    if w.unit(t)?.deployment.tile[1] != col {
                                        continue;
                                    }
                                    let amount = Self::tac_stat(w, id, "atk")?;
                                    let mut q =
                                        ability_damage(Some(id), t, amount, DamageType::Physical);
                                    q.is_attack = true;
                                    q.is_skill = false;
                                    q.dodge_mode = crate::battle::injury::DodgeMode::Normal;
                                    q.tags.clear();
                                    self.enemy_damage(w, r, &mut q)?;
                                }
                            }
                            let now = w.time;
                            let u = w.unit_mut(id)?;
                            u.last_attack_at = now;
                            u.attacks += 1;
                        }
                    }
                }
                self.content.as_mut().unwrap().set_boss(
                    id,
                    Memory::Deer {
                        low,
                        acc,
                        held_until,
                    },
                );
                self.enemy_caster_tick(w, r, id, deer.caster, dt)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_boss_profile(&self, kit: &Kit, p: &mut enemy::Profile) {
        self.helm_profile(kit, p);
        match kit {
            Kit::Boss(Boss::Quintus(_)) => p.max_targets = 2.,
            Kit::Boss(Boss::Deer(_) | Boss::Pipe(_)) => p.no_attack = true,
            _ => {}
        }
    }
    pub(super) fn enemy_boss_hit(&self, id: Id, q: &mut hit::Request) {
        if matches!(self.boss_config(id), Some(Boss::Helm(_)))
            && q.hit.damage_type != DamageType::Elemental
        {
            q.hit.damage_type = DamageType::Arts;
        }
        if matches!(
            self.content.as_ref().unwrap().boss(id),
            Memory::Lion { wand: true, .. }
        ) {
            q.hit.damage_type = DamageType::Arts;
        }
    }
    pub(in crate::battle::runtime) fn enemy_boss_in(
        &self,
        w: &World,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        if let Some(source) = q.hit.source {
            if let Some(b) = w
                .unit(q.hit.target)?
                .stats
                .buffs()
                .iter()
                .find(|b| b.key.as_deref() == Some("boss:vest") && b.shield > 0.)
            {
                if let Some(BuffEffect::Vest { side, cut }) = b.runtime.enemy_effect {
                    let dx = w.unit(source)?.position[1] - w.unit(q.hit.target)?.position[1];
                    if dx != 0. && dx.signum() == side {
                        q.damage.mul *= 1. - cut;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_boss_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
    ) -> Result<(), String> {
        self.gun_dealt(w, r, id, target)?;
        self.quintus_dealt(w, r, id, target)?;
        let Memory::Lion {
            sp,
            advanced,
            next_anim,
            wand: true,
        } = self.content.as_ref().unwrap().boss(id)
        else {
            return Ok(());
        };
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Lion {
                sp,
                advanced,
                next_anim,
                wand: false,
            },
        );
        let raw = &self.data.table("enemies").unwrap()["enemy_10030_vtwand"];
        let duration = raw["talents"]["bb"]["1.disarmed_duration"]
            .as_f64()
            .or_else(|| raw["talent"]["1.disarmed_duration"].as_f64())
            .unwrap_or(0.);
        if duration > 0. {
            self.apply_status(
                w,
                r,
                target,
                "disarm",
                status::Options {
                    duration,
                    source: Some(id),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_surge_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let source = b.runtime.source.ok_or("Missing surge source")?;
        let scale = b.runtime.tick_payload;
        let amount = Self::tac_stat(w, source, "atk")? * scale;
        let mut q = ability_damage(Some(source), c.unit, amount, DamageType::Arts);
        q.is_skill = false;
        q.ignore_select = true;
        self.enemy_damage(w, r, &mut q)
    }
}
