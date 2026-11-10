// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{
        movement::{Memory, Unbalance},
        Movement,
    },
    grid::{Grid, COLS},
    targeting,
};
use crate::deterministic::math::hypot;
impl Host {
    pub(super) fn enemy_can_cast(
        &mut self,
        w: &mut World,
        id: Id,
        silence: bool,
    ) -> Result<bool, String> {
        let time = w.time;
        let u = w.unit_mut(id)?;
        let alive = u.stats.alive;
        let flags = &u.stats.stats().map_err(str::to_string)?.flags;
        Ok(alive
            && !u.hidden
            && !flags.contains("stun")
            && !(silence && flags.contains("silence"))
            && time >= u.motion.unbalance_until - 1e-9)
    }
    fn enemy_float(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        on: bool,
    ) -> Result<(), String> {
        if !on {
            r.remove(w, self, id, "ab:float")?;
            return Ok(());
        }
        let mut b = buff("ab:float", &[]);
        b.runtime.persist = true;
        b.flags
            .extend(["unblockable", "float", "noDisplace"].map(str::to_string));
        self.enemy_buff(w, r, id, b)?;
        blocking::unblock(
            w,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            id,
        )
    }
    pub(in crate::battle::runtime) fn enemy_movement_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &status::Context,
    ) -> Result<(), String> {
        if !matches!(c.key.as_str(), "stun" | "freeze" | "sleep" | "groundbind")
            || w.unit(c.target)?.side != health::Side::Enemy
        {
            return Ok(());
        }
        let id = c.target;
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Movement(Movement::Syufo { .. })
                if matches!(c.key.as_str(), "stun" | "freeze" | "sleep" | "groundbind") =>
            {
                if !matches!(
                    self.content.as_ref().unwrap().movement(id),
                    Memory::Syufo { crawl: false }
                ) {
                    return Ok(());
                }
                self.content
                    .as_mut()
                    .unwrap()
                    .set_movement(id, Memory::Syufo { crawl: true });
                self.enemy_float(w, r, id, false)?;
                self.apply_status(
                    w,
                    r,
                    id,
                    "stun",
                    status::Options {
                        duration: 0.5,
                        ..Default::default()
                    },
                )?;
            }
            Kit::Movement(Movement::Parrot { down: duration, .. }) => {
                let Memory::Parrot {
                    down,
                    mut chill,
                    ran,
                } = self.content.as_ref().unwrap().movement(id)
                else {
                    unreachable!()
                };
                if c.key == "freeze" {
                    chill = true;
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_movement(id, Memory::Parrot { down, chill, ran });
                    return Ok(());
                }
                if down || !matches!(c.key.as_str(), "stun" | "sleep" | "groundbind") {
                    return Ok(());
                }
                self.content.as_mut().unwrap().set_movement(
                    id,
                    Memory::Parrot {
                        down: true,
                        chill,
                        ran,
                    },
                );
                self.enemy_float(w, r, id, false)?;
                r.remove(w, self, id, "ab:parrotRun")?;
                let mut b = buff("ab:parrotDown", &[]);
                b.flags.insert("unblockable".into());
                b.runtime.persist = true;
                self.enemy_buff(w, r, id, b)?;
                if duration > 0. {
                    self.apply_status(
                        w,
                        r,
                        id,
                        "stun",
                        status::Options {
                            duration,
                            ..Default::default()
                        },
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_movement_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if let Kit::Movement(Movement::Parrot {
            speed, duration, ..
        }) = self.content.as_ref().unwrap().kit(id)
        {
            if let Memory::Parrot {
                down: false,
                chill,
                ran: false,
            } = self.content.as_ref().unwrap().movement(id)
            {
                self.content.as_mut().unwrap().set_movement(
                    id,
                    Memory::Parrot {
                        down: false,
                        chill,
                        ran: true,
                    },
                );
                let mut b = buff("ab:parrotRun", &[("moveMul", speed)]);
                b.runtime.duration = duration;
                self.enemy_buff(w, r, id, b)?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_targets_near(
        &mut self,
        w: &mut World,
        id: Id,
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        let candidates = enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?;
        let mut out = vec![];
        for target in candidates {
            if targeting::can_target_ally(w, id, target, true, Default::default())? {
                out.push(target);
            }
        }
        Ok(out)
    }
    pub(super) fn enemy_movement_attack(
        &mut self,
        w: &mut World,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        if event != enemy::Event::BeforeAttack
            || !matches!(
                self.content.as_ref().unwrap().kit(c.enemy),
                Kit::Movement(Movement::BoneSpike { .. })
            )
        {
            return Ok(());
        }
        if !targeting::Queries::enemy_stealthed(self, c.enemy, w)? {
            return Ok(());
        }
        let radius = w.unit(c.enemy)?.stats.base_number("rangeRadius") + 0.25;
        let mut targets = self.enemy_targets_near(w, c.enemy, radius)?;
        targets.retain(|&id| w.unit(id).is_ok_and(|u| !u.motion_fly));
        targeting::sort_allies(
            w,
            c.enemy,
            &mut targets,
            targeting::Profile {
                block_free: true,
                ..Default::default()
            },
        )?;
        targets.truncate(3);
        if !targets.is_empty() {
            c.targets = targets;
        }
        Ok(())
    }
    pub(super) fn enemy_movement_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Movement,
        dt: f64,
    ) -> Result<(), String> {
        match kit {
            Movement::Parrot { .. } => {
                let Memory::Parrot { down, chill, ran } =
                    self.content.as_ref().unwrap().movement(id)
                else {
                    unreachable!()
                };
                let flags = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .clone();
                if chill && !flags.contains("freeze") {
                    self.content.as_mut().unwrap().set_movement(
                        id,
                        Memory::Parrot {
                            down,
                            chill: false,
                            ran,
                        },
                    );
                    self.apply_status(
                        w,
                        r,
                        id,
                        "stun",
                        status::Options {
                            duration: 0.05,
                            ..Default::default()
                        },
                    )?;
                }
                let Memory::Parrot { down, chill, ran } =
                    self.content.as_ref().unwrap().movement(id)
                else {
                    unreachable!()
                };
                let flags = &w.unit_mut(id)?.stats.stats().map_err(str::to_string)?.flags;
                if down && !flags.contains("stun") && !flags.contains("groundbind") {
                    self.content.as_mut().unwrap().set_movement(
                        id,
                        Memory::Parrot {
                            down: false,
                            chill,
                            ran,
                        },
                    );
                    r.remove(w, self, id, "ab:parrotDown")?;
                    r.remove(w, self, id, "ab:parrotRun")?;
                    self.enemy_float(w, r, id, true)?;
                }
            }
            Movement::Unbalanced(Unbalance::Bleed { amount, interval }) => {
                let Memory::Bleed { mut elapsed } = self.content.as_ref().unwrap().movement(id)
                else {
                    unreachable!()
                };
                if amount <= 0.
                    || interval <= 0.
                    || w.time >= w.unit(id)?.motion.unbalance_until - 1e-9
                {
                    elapsed = 0.;
                } else {
                    elapsed += dt;
                    let mut count = 0;
                    while elapsed >= interval - 1e-9 && w.unit(id)?.stats.alive && count < 8 {
                        elapsed -= interval;
                        count += 1;
                        let mut hit = ability_damage(None, id, amount, DamageType::True);
                        hit.is_skill = false;
                        hit.attack_type = hit::AttackType::Buff;
                        hit.tags = vec!["dot".into(), "periodic".into(), "unbalanced".into()];
                        self.enemy_damage(w, r, &mut hit)?;
                    }
                }
                if w.unit(id)?.stats.alive {
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_movement(id, Memory::Bleed { elapsed });
                }
            }
            Movement::Unbalanced(react) => {
                let Memory::Travel {
                    previous,
                    hidden,
                    done,
                } = self.content.as_ref().unwrap().movement(id)
                else {
                    unreachable!()
                };
                let position = w.unit(id)?.position;
                let now_hidden = w.unit(id)?.hidden;
                self.content.as_mut().unwrap().set_movement(
                    id,
                    Memory::Travel {
                        previous: Some(position),
                        hidden: now_hidden,
                        done,
                    },
                );
                let Some(old) = previous.filter(|_| !hidden && !now_hidden) else {
                    return Ok(());
                };
                let dy = position[0] - old[0];
                let dx = position[1] - old[1];
                let moved = hypot(dx, dy);
                let own = Self::tac_stat(w, id, "moveSpeed")? * 0.5 * dt * 1.5 + 1e-3;
                if moved - own <= 0.05 {
                    return Ok(());
                }
                match react {
                    Unbalance::Stun(duration) if duration > 0. => {
                        self.apply_status(
                            w,
                            r,
                            id,
                            "stun",
                            status::Options {
                                duration,
                                ..Default::default()
                            },
                        )?;
                    }
                    Unbalance::LoseEgg(speed) if !done => {
                        self.content.as_mut().unwrap().set_movement(
                            id,
                            Memory::Travel {
                                previous: Some(position),
                                hidden: now_hidden,
                                done: true,
                            },
                        );
                        let mut b = buff("ab:noEgg", &[("moveMul", 1. + speed)]);
                        b.runtime.persist = true;
                        b.runtime.visible = true;
                        b.flags.insert("unblockable".into());
                        self.enemy_buff(w, r, id, b)?;
                    }
                    Unbalance::Wall(amount) => {
                        let point = [
                            (position[0] + dy / moved * 0.6 + 0.5).floor() as i32,
                            (position[1] + dx / moved * 0.6 + 0.5).floor() as i32,
                        ];
                        let grid = self.grid.as_ref().ok_or("Missing collision query grid")?;
                        if Grid::in_bounds(point)
                            && grid.tiles()[(point[0] * COLS as i32 + point[1]) as usize].height
                                == "LOW"
                            && grid.ground_passable(point, false)
                        {
                            return Ok(());
                        }
                        let mut hit = ability_damage(None, id, amount, DamageType::True);
                        hit.tags.push("wallHit".into());
                        self.enemy_damage(w, r, &mut hit)?;
                    }
                    _ => {}
                }
            }
            Movement::Run {
                skill: Some(skill),
                speed,
                duration,
            } => {
                let Memory::Cast { mut left } = self.content.as_ref().unwrap().movement(id) else {
                    unreachable!()
                };
                let hp = w.unit(id)?.stats.hp;
                let ready =
                    self.enemy_can_cast(w, id, true)? && hp / Self::tac_stat(w, id, "maxHp")? < 0.5;
                let cast = skill.advance(&mut left, dt, ready);
                self.content
                    .as_mut()
                    .unwrap()
                    .set_movement(id, Memory::Cast { left });
                if cast {
                    let mut b = buff("ab:run", &[("moveMul", 1. + speed)]);
                    b.runtime.duration = duration;
                    b.runtime.visible = true;
                    b.flags.insert("unblockable".into());
                    self.enemy_buff(w, r, id, b)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_movement_profile(
        &self,
        id: Id,
        kit: &Kit,
        profile: &mut enemy::Profile,
    ) {
        match kit {
            Kit::Movement(Movement::Syufo { .. }) => {
                profile.melee = matches!(
                    self.content.as_ref().unwrap().movement(id),
                    Memory::Syufo { crawl: true }
                );
            }
            Kit::Movement(Movement::Parrot { .. }) => profile.no_attack = true,
            Kit::Movement(Movement::Unbalanced(Unbalance::LoseEgg(_))) => {
                profile.no_attack |= matches!(
                    self.content.as_ref().unwrap().movement(id),
                    Memory::Travel { done: true, .. }
                );
            }
            Kit::Movement(Movement::BoneSpike { .. }) => profile.selection.block_free = true,
            _ => {}
        }
    }
}
