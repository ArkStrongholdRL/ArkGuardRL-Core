// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::bosses::{
        gun::{Charge, Dash},
        Gun, SpringKind,
    },
    element::Element,
};
use crate::deterministic::math::hypot;
mod springs;
impl Host {
    fn gun_companions(&self, w: &World, id: Id, springs_only: bool) -> Vec<Id> {
        self.spawns
            .as_ref()
            .unwrap()
            .live()
            .iter()
            .copied()
            .filter(|&t| {
                t != id
                    && w.unit(t).is_ok_and(|u| u.stats.alive)
                    && match self.boss_config(t) {
                        Some(Boss::Spring(_)) => true,
                        Some(Boss::Gun(_)) => !springs_only,
                        _ => false,
                    }
            })
            .collect()
    }
    fn gun_defense_targets(
        &mut self,
        w: &mut World,
        id: Id,
        ground: bool,
    ) -> Result<Vec<Id>, String> {
        let radius = w.unit(id)?.stats.base_number("rangeRadius");
        let mut targets = self.boss_targets(
            w,
            id,
            if ground {
                f64::INFINITY
            } else if radius != 0. {
                radius
            } else {
                2.
            },
        )?;
        if ground {
            targets.retain(|&t| w.unit(t).unwrap().ground);
        }
        targeting::sort_allies(w, id, &mut targets, Default::default())?;
        let mut keyed = vec![];
        for t in targets {
            keyed.push((t, Self::tac_stat(w, t, "def")?));
        }
        keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(keyed.into_iter().map(|x| x.0).collect())
    }
    pub(super) fn gun_condition(
        &mut self,
        w: &mut World,
        id: Id,
        spell: Spell,
    ) -> Result<Option<bool>, String> {
        Ok(Some(match spell {
            Spell::GunCharge => {
                matches!(self.content.as_ref().unwrap().boss(id), Memory::Gun(s) if s.charge.is_none())
                    && !self.gun_defense_targets(w, id, true)?.is_empty()
            }
            Spell::SpringDash { .. } => !self.gun_companions(w, id, true).is_empty(),
            _ => return Ok(None),
        }))
    }
    pub(super) fn gun_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
    ) -> Result<(), String> {
        match spell {
            Spell::GunCharge => {
                let Some(Boss::Gun(config)) = self.boss_config(id) else {
                    unreachable!()
                };
                if let Some(&target) = self.gun_defense_targets(w, id, true)?.first() {
                    let Memory::Gun(mut state) = self.content.as_ref().unwrap().boss(id) else {
                        unreachable!()
                    };
                    state.charge = Some(Charge {
                        target: w.unit(target)?.position,
                        until: w.time + config.duration,
                        hit: vec![],
                    });
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_boss(id, Memory::Gun(state));
                    let mut b = buff("boss:charge", &[]);
                    b.runtime.duration = config.duration;
                    b.runtime.visible = true;
                    b.flags.extend(["noMove", "disarm"].map(str::to_string));
                    self.enemy_buff(w, r, id, b)?;
                }
            }
            Spell::SpringDash { duration, mul } => {
                for t in self.gun_companions(w, id, true) {
                    let Memory::Spring(mut state) = self.content.as_ref().unwrap().boss(t) else {
                        unreachable!()
                    };
                    state.dash = Some(Dash {
                        gun: id,
                        until: w.time + duration,
                        mul,
                        hit: vec![],
                    });
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_boss(t, Memory::Spring(state));
                    let mut b = buff("boss:dash", &[]);
                    b.runtime.duration = duration;
                    b.runtime.visible = true;
                    b.flags.extend(
                        ["invulnerable", "noMove", "unblockable", "disarm"].map(str::to_string),
                    );
                    self.enemy_buff(w, r, t, b)?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn gun_aspd(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Gun,
        stacks: u32,
    ) -> Result<(), String> {
        if config.raw_aspd {
            let mut b = buff(
                "boss:aspd",
                &[(
                    "aspd",
                    (config.step * stacks as f64).max(20.) - w.unit(id)?.stats.base_number("aspd"),
                )],
            );
            b.runtime.persist = true;
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn gun_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        if let Some(Boss::Spring(config)) = self.boss_config(id) {
            return self.spring_attack(w, r, c, event, config);
        }
        let Some(Boss::Gun(config)) = self.boss_config(id) else {
            return Ok(());
        };
        if event == enemy::Event::BeforeAttack {
            if let Some(&t) = self.gun_defense_targets(w, id, false)?.first() {
                c.targets = vec![t];
            }
        } else {
            let Memory::Gun(mut state) = self.content.as_ref().unwrap().boss(id) else {
                unreachable!()
            };
            let target = c.targets.first().copied();
            state.stacks = if target == state.last {
                state.stacks.saturating_add(1).min(config.max)
            } else {
                0
            };
            state.last = target;
            self.gun_aspd(w, r, id, config, state.stacks)?;
            self.content
                .as_mut()
                .unwrap()
                .set_boss(id, Memory::Gun(state));
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn gun_confession(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(Boss::Gun(config)) = self
            .boss_config(id)
            .filter(|b| matches!(b, Boss::Gun(Gun { hidden: true, .. })))
        else {
            return Ok(());
        };
        if self.gun_companions(w, id, false).is_empty() {
            r.remove(w, self, id, "boss:confession")?;
        } else {
            let mut b = buff(
                "boss:confession",
                &[
                    ("physTakenMul", config.confession),
                    ("artsTakenMul", config.confession),
                ],
            );
            b.runtime.duration = 0.35;
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    fn gun_step(w: &mut World, id: Id, target: [f64; 2], speed: f64) -> Result<bool, String> {
        let origin = w.unit(id)?.position;
        let d = [target[0] - origin[0], target[1] - origin[1]];
        let len = hypot(d[1], d[0]);
        if len <= speed + 1e-9 {
            w.unit_mut(id)?.position = target;
            return Ok(true);
        }
        w.unit_mut(id)?.position = [
            origin[0] + d[0] / len * speed,
            origin[1] + d[1] / len * speed,
        ];
        Ok(false)
    }
    fn gun_trample(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        hit: &mut Vec<Id>,
        scale: f64,
    ) -> Result<(), String> {
        let origin = w.unit(id)?.position;
        for t in self.boss_area(w, id)? {
            let pos = w.unit(t)?.position;
            if hypot(pos[1] - origin[1], pos[0] - origin[0]) <= 0.35 && !hit.contains(&t) {
                if hit.len() >= w.unit_capacity() {
                    return Err("Trample target capacity exceeded".into());
                }
                hit.push(t);
                let mut q = ability_damage(
                    Some(id),
                    t,
                    Self::tac_stat(w, id, "atk")? * scale,
                    DamageType::Physical,
                );
                self.enemy_damage(w, r, &mut q)?;
            }
        }
        Ok(())
    }
    pub(super) fn gun_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Gun,
        dt: f64,
    ) -> Result<(), String> {
        let Memory::Gun(mut state) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        if w.time >= w.unit(id)?.motion.unbalance_until - 1e-9 {
            if let Some(mut charge) = state.charge.take() {
                let old = w.unit(id)?.position;
                let speed = (Self::tac_stat(w, id, "moveSpeed")? + config.speed) * 0.5 * dt;
                let arrived = Self::gun_step(w, id, charge.target, speed)?;
                let pos = w.unit(id)?.position.map(|v| (v + 0.5).floor() as i32);
                if !self
                    .grid
                    .as_ref()
                    .ok_or("Missing charge grid")?
                    .ground_passable(pos, false)
                {
                    w.unit_mut(id)?.position = old;
                }
                self.gun_trample(w, r, id, &mut charge.hit, config.scale)?;
                if arrived || w.time >= charge.until || w.unit(id)?.position == old {
                    r.remove(w, self, id, "boss:charge")?;
                    if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                        route.points = None;
                    }
                } else {
                    state.charge = Some(charge);
                }
            }
        }
        if config.hidden && self.content.as_mut().unwrap().pulses(id, dt, 0.25) > 0 {
            self.gun_confession(w, r, id)?;
        }
        if let Some([interval, damage]) = config.link {
            let mut pulses = 1;
            if interval > 0. {
                state.link_acc += dt;
                pulses = 0;
                while state.link_acc >= interval - 1e-9 && pulses < 4 {
                    state.link_acc -= interval;
                    pulses += 1;
                }
            }
            for _ in 0..pulses {
                let start = w.unit(id)?.position;
                for t in self.gun_companions(w, id, true) {
                    let end = w.unit(t)?.position;
                    for a in self.roster.as_ref().unwrap().ids().to_vec() {
                        let u = w.unit(a)?;
                        if !u.stats.alive || !u.deployed {
                            continue;
                        }
                        let p = u.position;
                        let d = [end[0] - start[0], end[1] - start[1]];
                        let len = d[0] * d[0] + d[1] * d[1];
                        let along = if len == 0. {
                            0.
                        } else {
                            ((p[0] - start[0]) * d[0] + (p[1] - start[1]) * d[1]) / len
                        }
                        .clamp(0., 1.);
                        if hypot(
                            p[1] - start[1] - along * d[1],
                            p[0] - start[0] - along * d[0],
                        ) <= 0.5
                        {
                            let mut q = ability_damage(Some(id), a, damage, DamageType::Physical);
                            q.ignore_select = true;
                            q.tags.push("faithLink".into());
                            self.enemy_damage(w, r, &mut q)?;
                        }
                    }
                }
            }
        }
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Gun(state));
        self.rebuild_index(w)?;
        self.enemy_caster_tick(w, r, id, config.caster, dt)
    }
    pub(super) fn gun_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
    ) -> Result<(), String> {
        if let Some(Boss::Gun(config)) = self.boss_config(id) {
            let amount = Self::tac_stat(w, id, "atk")? * config.erosion;
            self.enemy_element(w, r, id, target, Element::Erosion, amount)?;
        }
        Ok(())
    }
}
