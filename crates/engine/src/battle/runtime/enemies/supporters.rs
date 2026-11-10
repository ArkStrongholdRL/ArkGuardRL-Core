// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::content::enemies::{supporters::Memory, Supporter};
use crate::deterministic::math::hypot;
impl Host {
    pub(super) fn enemy_supporter_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Kit::Supporter(Supporter::Roar { fear, .. }) = self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        let Memory::Roar { left, feared } = self.content.as_ref().unwrap().supporter(id) else {
            unreachable!()
        };
        let hp = w.unit(id)?.stats.hp;
        if feared || hp / Self::tac_stat(w, id, "maxHp")? >= 0.5 {
            return Ok(());
        }
        self.content
            .as_mut()
            .unwrap()
            .set_supporter(id, Memory::Roar { left, feared: true });
        if fear.duration > 0. {
            self.apply_status(
                w,
                r,
                id,
                "fear",
                status::Options {
                    source: Some(id),
                    duration: fear.duration,
                    ..Default::default()
                },
            )?;
        }
        if fear.speed > 0. && fear.speed_duration > 0. {
            let mut b = buff("ab:fearRun", &[("moveMul", fear.speed)]);
            b.runtime.duration = fear.speed_duration;
            b.flags.insert("unblockable".into());
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(super) fn enemy_supporter_hit(
        &mut self,
        w: &World,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Supporter(Supporter::Torturer { ranged, .. })
                if w.unit(id)?.blocked_by() != Some(q.hit.target) =>
            {
                q.damage.amount *= ranged
            }
            Kit::Supporter(Supporter::Barrel { scale, .. })
                if !w
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.flags.contains("silence"))
                    && matches!(
                        self.content.as_ref().unwrap().supporter(id),
                        Memory::Barrel { done: false }
                    ) =>
            {
                q.damage.amount *= scale
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_supporter_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        event: enemy::Event,
    ) -> Result<(), String> {
        let Kit::Supporter(Supporter::Barrel {
            life, aspd, dodge, ..
        }) = self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        if event != enemy::Event::Attack
            || silenced(w, id)?
            || !matches!(
                self.content.as_ref().unwrap().supporter(id),
                Memory::Barrel { done: false }
            )
        {
            return Ok(());
        }
        self.content
            .as_mut()
            .unwrap()
            .set_supporter(id, Memory::Barrel { done: true });
        r.remove(w, self, id, "ab:barrel")?;
        let count = ((life - 1e-6) / 0.5).ceil().max(1.);
        if !count.is_finite() || count > i32::MAX as f64 {
            return Err("Barrel pulse count outside positive i32 domain".into());
        }
        self.enemy_schedule(
            w,
            None,
            0.5,
            0.5,
            count as u32,
            EnemyTask::Barrel {
                position: w.unit(id)?.position,
                aspd,
                dodge,
            },
        )
    }
    pub(super) fn enemy_barrel_pulse(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        position: [f64; 2],
        aspd: f64,
        dodge: f64,
    ) -> Result<(), String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        let targets = spatial::enemies_in_radius(
            w,
            self,
            &ids,
            position,
            1.5,
            RadiusRule::Body,
            false,
            w.unit_capacity(),
        )?;
        for target in targets {
            let mut b = buff("ab:barrelZone", &[("aspd", aspd), ("dodgePhys", dodge)]);
            b.runtime.duration = 0.6;
            self.enemy_buff(w, r, target, b)?;
        }
        Ok(())
    }
    pub(super) fn enemy_supporter_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Supporter,
        dt: f64,
    ) -> Result<(), String> {
        match kit {
            Supporter::Roar {
                skill: Some(skill),
                duration,
                aspd,
                ..
            } => {
                let Memory::Roar { mut left, feared } =
                    self.content.as_ref().unwrap().supporter(id)
                else {
                    unreachable!()
                };
                let castable = self.enemy_can_cast(w, id, true)?;
                let cast = skill.advance(&mut left, dt, castable);
                self.content
                    .as_mut()
                    .unwrap()
                    .set_supporter(id, Memory::Roar { left, feared });
                if cast {
                    for target in self.roster.as_ref().unwrap().ids().to_vec() {
                        if w.unit(target)?.motion_fly
                            || !crate::battle::targeting::area_selectable(w, Some(id), target)?
                        {
                            continue;
                        }
                        let mut b = buff("ab:roar", &[("aspd", aspd)]);
                        b.runtime.duration = duration;
                        b.runtime.visible = true;
                        b.runtime.refresh = buffs::Refresh::Extend;
                        self.enemy_buff(w, r, target, b)?;
                    }
                }
            }
            Supporter::Carrier { max, camel, speed } => {
                let Memory::Carrier {
                    mut passengers,
                    seats,
                } = self.content.as_ref().unwrap().supporter(id)
                else {
                    unreachable!()
                };
                let mut keep = vec![];
                for p in passengers {
                    let u = w.unit(p)?;
                    if u.stats.alive && u.enemy.carrier == Some(id) {
                        keep.push(p);
                    } else {
                        if u.enemy.carrier == Some(id) {
                            w.unit_mut(p)?.enemy.carrier = None;
                        }
                        w.unpin_unit(p)?;
                    }
                }
                passengers = keep;
                let picking = self.enemy_can_cast(w, id, false)?;
                if picking {
                    for other in self.spawns.as_ref().unwrap().live().to_vec() {
                        if passengers.len() >= max {
                            break;
                        }
                        let u = w.unit(other)?;
                        if other == id
                            || !u.stats.alive
                            || !u.deployed
                            || u.hidden
                            || u.enemy.carrier.is_some()
                            || u.boss
                            || u.motion_fly
                        {
                            continue;
                        }
                        let def = self.spawns.as_ref().unwrap().definition(other)?;
                        if def.leader || def.machine {
                            continue;
                        }
                        let at = w.unit(id)?.position;
                        if hypot(u.position[1] - at[1], u.position[0] - at[0]) > 0.5 + 1e-9 {
                            continue;
                        }
                        enemy::clear(w, other)?;
                        let u = w.unit_mut(other)?;
                        u.enemy.carrier = Some(id);
                        u.hidden = true;
                        u.motion.attack_stand_until = f64::NEG_INFINITY;
                        w.pin_unit(other)?;
                        blocking::unblock(
                            w,
                            &mut allies::Blocking {
                                host: self,
                                buffs: r,
                            },
                            other,
                        )?;
                        passengers.push(other);
                    }
                }
                for &other in &passengers {
                    let position = w.unit(id)?.position;
                    let u = w.unit_mut(other)?;
                    u.position = position;
                    u.moving = false;
                }
                let now = passengers.len();
                self.content.as_mut().unwrap().set_supporter(
                    id,
                    Memory::Carrier {
                        passengers,
                        seats: if camel { now } else { seats },
                    },
                );
                if camel && seats != now {
                    let mut b = buff(
                        "ab:busSpeed",
                        &[("moveMul", (1. + speed * now as f64).max(0.))],
                    );
                    b.runtime.persist = true;
                    self.enemy_buff(w, r, id, b)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_other_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        victim: Id,
    ) -> Result<(), String> {
        if w.unit(victim)?.stats.alive {
            return Ok(());
        }
        let position = w.unit(victim)?.position;
        let knight = w.unit(victim)?.side == health::Side::Enemy
            && self
                .spawns
                .as_ref()
                .unwrap()
                .definition(victim)?
                .key
                .contains("enemy_1513_dekght");
        for id in self.spawns.as_ref().unwrap().live().to_vec() {
            if id == victim || !w.unit(id)?.stats.alive {
                continue;
            }
            if knight {
                if let Kit::Caster(content::Caster {
                    passive:
                        content::casters::Passive::Knight {
                            atk, aspd, speed, ..
                        },
                    ..
                }) = self.content.as_ref().unwrap().kit(id)
                {
                    let mut state = self.content.as_ref().unwrap().caster(id);
                    if !state.enraged {
                        state.enraged = true;
                        self.content.as_mut().unwrap().set_caster(id, state);
                        let mut b = buff(
                            "ab:rage",
                            &[("atkPct", atk), ("aspd", aspd), ("moveMul", 1. + speed)],
                        );
                        b.runtime.persist = true;
                        b.runtime.visible = true;
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
            }
            let Kit::Supporter(Supporter::Torturer {
                radius,
                heal,
                atk,
                max,
                ..
            }) = self.content.as_ref().unwrap().kit(id)
            else {
                continue;
            };
            let at = w.unit(id)?.position;
            if hypot(at[1] - position[1], at[0] - position[0]) > radius + 1e-9 {
                continue;
            }
            let amount = Self::tac_stat(w, id, "maxHp")? * heal;
            self.support_heal(
                w,
                r,
                Some(id),
                id,
                amount,
                health::HealOptions {
                    self_heal: true,
                    ..Default::default()
                },
            )?;
            let mut b = buff("ab:torture", &[("atkPct", atk)]);
            b.runtime.persist = true;
            b.runtime.refresh = buffs::Refresh::Stack;
            b.runtime.max_stacks = max;
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_torturer_children(
        &mut self,
        w: &World,
        id: Id,
    ) -> Result<Option<super::splits::Children>, String> {
        let Kit::Supporter(Supporter::Torturer { count, .. }) =
            self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(None);
        };
        let Some(key) = self
            .content
            .as_ref()
            .unwrap()
            .child(id)
            .filter(|_| count > 0)
        else {
            return Ok(None);
        };
        let mut children = self.enemy_children_spec(w, id, key, count)?;
        children.make_stationary(w, id, self.spawns.as_ref().unwrap().rect())?;
        Ok(Some(children))
    }
    pub(in crate::battle::runtime) fn enemy_unload(
        &mut self,
        w: &mut World,
        id: Id,
        rng: &mut Rng,
    ) -> Result<(), String> {
        let Kit::Supporter(Supporter::Carrier { camel, .. }) =
            self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        let Memory::Carrier { passengers, .. } = self.content.as_ref().unwrap().supporter(id)
        else {
            unreachable!()
        };
        let origin = w.unit(id)?.position;
        let rect = self.spawns.as_ref().unwrap().rect();
        for other in passengers {
            if w.unit(other)?.enemy.carrier == Some(id) {
                w.unit_mut(other)?.enemy.carrier = None;
            }
            if !w.unit(other)?.stats.alive {
                w.unpin_unit(other)?;
                continue;
            }
            let position = if camel {
                let x = (origin[1] + rng.range(-0.2, 0.2)).clamp(rect.c0 as f64, rect.c1 as f64);
                let y = (origin[0] + rng.range(-0.2, 0.2)).clamp(rect.r0 as f64, rect.r1 as f64);
                [y, x]
            } else {
                origin.map(|x| (x + 0.5).floor())
            };
            let route = w
                .unit(if camel { id } else { other })?
                .motion
                .route
                .as_ref()
                .ok_or("Missing passenger unload route")?
                .after_wait(position, rect)?;
            enemy::clear(w, other)?;
            let u = w.unit_mut(other)?;
            u.position = position;
            u.motion.route = Some(route);
            u.motion.attack_stand_until = f64::NEG_INFINITY;
            u.hidden = false;
            w.unpin_unit(other)?;
        }
        self.content.as_mut().unwrap().set_supporter(
            id,
            Memory::Carrier {
                passengers: vec![],
                seats: 0,
            },
        );
        Ok(())
    }
}
