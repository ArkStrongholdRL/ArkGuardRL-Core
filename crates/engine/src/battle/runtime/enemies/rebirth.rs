// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{
        lifecycle::{Memory, RebornPhase},
        Lifecycle,
    },
    targeting,
};
use crate::deterministic::math::hypot;
pub(in crate::battle::runtime) const PILOT_BURN: u64 = 1706;
impl Host {
    pub(in crate::battle::runtime::enemies) fn enemy_rebirth_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Lifecycle,
    ) -> Result<(), String> {
        match kit {
            Lifecycle::Warden { .. } => {
                for other in self.spawns.as_ref().unwrap().live().to_vec() {
                    self.liberate_prisoner(w, r, other)?;
                }
            }
            Lifecycle::Ugly { bomb, stun, .. } => {
                self.enemy_schedule(
                    w,
                    Some(id),
                    2.17,
                    0.,
                    1,
                    EnemyTask::Blast {
                        position: w.unit(id)?.position,
                        scale: bomb,
                        stun,
                    },
                )?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_rebirth_finish(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Lifecycle,
    ) -> Result<(), String> {
        match kit {
            Lifecycle::Warden {
                atk,
                defense,
                bat,
                speed,
                ..
            } => {
                self.content.as_mut().unwrap().set_lifecycle(
                    id,
                    Memory::Warden {
                        phase: RebornPhase::Revived,
                        count: 0,
                        power: false,
                    },
                );
                r.remove(w, self, id, "ab:warden")?;
                let mut b = buff(
                    "ab:killer",
                    &[
                        ("atkFlat", atk),
                        ("defFlat", defense),
                        (
                            "batPct",
                            bat / w.unit(id)?.stats.base_number("bat").max(1e-9),
                        ),
                        ("moveFlat", speed),
                    ],
                );
                b.runtime.persist = true;
                b.runtime.visible = true;
                self.enemy_buff(w, r, id, b)?;
            }
            Lifecycle::Ugly {
                speed,
                defense,
                res,
                burn,
                ..
            } => {
                let mut b = buff(
                    "ab:pilot",
                    &[
                        ("moveMul", 1. + speed),
                        (
                            "defFlat",
                            defense
                                .map(|v| v - w.unit(id).unwrap().stats.base_number("def"))
                                .unwrap_or(0.),
                        ),
                        (
                            "resFlat",
                            res.map(|v| v - w.unit(id).unwrap().stats.base_number("res"))
                                .unwrap_or(0.),
                        ),
                    ],
                );
                b.flags.insert("unblockable".into());
                b.runtime.persist = true;
                b.runtime.visible = true;
                b.runtime.interval = 1.;
                b.runtime.tick_payload = burn;
                b.runtime.on_tick = Some(PILOT_BURN);
                self.enemy_buff(w, r, id, b)?;
            }
            Lifecycle::Reid { skill, .. } => {
                if let Memory::Reid { phase, enraged, .. } =
                    self.content.as_ref().unwrap().lifecycle(id)
                {
                    self.content.as_mut().unwrap().set_lifecycle(
                        id,
                        Memory::Reid {
                            phase,
                            enraged,
                            left: skill.map(|s| s.initial).unwrap_or(0.),
                        },
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_pilot_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let amount = r.get(w, c.unit, c.buff)?.runtime.tick_payload;
        let mut q = ability_damage(None, c.unit, amount, DamageType::True);
        q.is_skill = false;
        q.tags.push("pilotBurn".into());
        self.enemy_damage(w, r, &mut q)
    }
    pub(super) fn enemy_rebirth_hit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match (
            self.content.as_ref().unwrap().kit(id),
            self.content.as_ref().unwrap().lifecycle(id),
        ) {
            (
                Kit::Lifecycle(Lifecycle::Warden { penetration, .. }),
                Memory::Warden { phase, power, .. },
            ) => {
                q.hit.damage_type = if phase == RebornPhase::Revived {
                    DamageType::Physical
                } else {
                    DamageType::Arts
                };
                if phase == RebornPhase::Revived && power {
                    q.damage.penetration.def_pct =
                        (q.damage.penetration.def_pct + penetration).min(1.);
                }
            }
            (Kit::Lifecycle(Lifecycle::Ugly { .. }), Memory::Ugly { splash: true, .. }) => {
                let amount = Self::tac_stat(w, id, "atk")?;
                for t in self.enemy_tile_allies(w, w.unit(q.hit.target)?.deployment.tile, 1)? {
                    if t != q.hit.target && targeting::area_selectable(w, Some(id), t)? {
                        let mut h = ability_damage(Some(id), t, amount, DamageType::Physical);
                        h.tags.push("splash".into());
                        self.enemy_damage(w, r, &mut h)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_rebirth_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        match (
            self.content.as_ref().unwrap().kit(id),
            self.content.as_ref().unwrap().lifecycle(id),
        ) {
            (
                Kit::Lifecycle(Lifecycle::Warden {
                    every,
                    stun,
                    targets,
                    ..
                }),
                Memory::Warden {
                    phase,
                    count,
                    power,
                },
            ) => {
                if event == enemy::Event::BeforeAttack {
                    let n = every[usize::from(phase == RebornPhase::Revived)];
                    self.content.as_mut().unwrap().set_lifecycle(
                        id,
                        Memory::Warden {
                            phase,
                            count,
                            power: n > 0 && (count + 1) % n == 0,
                        },
                    );
                }
                if event == enemy::Event::Attack {
                    self.content.as_mut().unwrap().set_lifecycle(
                        id,
                        Memory::Warden {
                            phase,
                            count: count.saturating_add(1),
                            power,
                        },
                    );
                    if phase != RebornPhase::Revived && power {
                        let radius = w.unit(id)?.stats.base_number("rangeRadius");
                        let mut list =
                            self.enemy_targets_near(w, id, if radius == 0. { 2. } else { radius })?;
                        targeting::sort_allies(w, id, &mut list, Default::default())?;
                        for &t in &c.targets {
                            if !list.contains(&t) {
                                list.insert(0, t);
                            }
                        }
                        for t in list.into_iter().take(targets) {
                            if stun > 0. && w.unit(t)?.stats.alive {
                                self.apply_status(
                                    w,
                                    r,
                                    t,
                                    "stun",
                                    status::Options {
                                        source: Some(id),
                                        duration: stun,
                                        ..Default::default()
                                    },
                                )?;
                            }
                        }
                    }
                }
            }
            (Kit::Lifecycle(Lifecycle::Ugly { .. }), Memory::Ugly { phase, .. })
                if event == enemy::Event::BeforeAttack =>
            {
                self.content.as_mut().unwrap().set_lifecycle(
                    id,
                    Memory::Ugly {
                        phase,
                        splash: w.unit(id)?.blocked_by().is_none(),
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_rebirth_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        if let (
            Kit::Lifecycle(Lifecycle::Warden {
                times, penetration, ..
            }),
            Memory::Warden {
                phase: RebornPhase::Revived,
                power: true,
                ..
            },
        ) = (
            self.content.as_ref().unwrap().kit(id),
            self.content.as_ref().unwrap().lifecycle(id),
        ) {
            for _ in 1..times {
                if !w.unit(q.hit.target)?.stats.alive {
                    break;
                }
                let amount = Self::tac_stat(w, id, "atk")?;
                let mut h = ability_damage(Some(id), q.hit.target, amount, DamageType::Physical);
                h.damage.penetration.def_pct = penetration;
                h.dodge_mode = crate::battle::injury::DodgeMode::Normal;
                h.tags.push("armorPiercing".into());
                self.enemy_damage(w, r, &mut h)?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_rebirth_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if let (
            Kit::Lifecycle(Lifecycle::Reid { ratio, atk, .. }),
            Memory::Reid {
                phase,
                left,
                enraged: false,
            },
        ) = (
            self.content.as_ref().unwrap().kit(id),
            self.content.as_ref().unwrap().lifecycle(id),
        ) {
            if w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? < ratio {
                self.content.as_mut().unwrap().set_lifecycle(
                    id,
                    Memory::Reid {
                        phase,
                        left,
                        enraged: true,
                    },
                );
                let mut b = buff("ab:lowhp", &[("atkPct", atk)]);
                b.runtime.persist = true;
                b.runtime.visible = true;
                self.enemy_buff(w, r, id, b)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_reid_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Lifecycle,
        dt: f64,
    ) -> Result<(), String> {
        let Lifecycle::Reid {
            skill: Some(skill),
            speed,
            rush,
            ..
        } = kit
        else {
            return Ok(());
        };
        let Memory::Reid {
            phase,
            mut left,
            enraged,
        } = self.content.as_ref().unwrap().lifecycle(id)
        else {
            return Ok(());
        };
        let mut path = std::collections::BTreeSet::new();
        let origin = w.unit(id)?.position;
        path.insert(origin.map(|v| (v + 0.5).floor() as i32));
        if let Some(route) = w.unit(id)?.motion.route.as_ref() {
            if let Some(points) = &route.points {
                let mut from = origin;
                for &p in points.iter().skip(route.point) {
                    let n = (hypot(p[0] - from[0], p[1] - from[1]) * 4.).ceil().max(1.) as usize;
                    for k in 1..=n {
                        path.insert(
                            [
                                from[0] + (p[0] - from[0]) * k as f64 / n as f64,
                                from[1] + (p[1] - from[1]) * k as f64 / n as f64,
                            ]
                            .map(|v| (v + 0.5).floor() as i32),
                        );
                    }
                    from = p;
                }
            }
        }
        let mut has = false;
        for t in self.enemy_targets_near(w, id, 3.)? {
            has |= w.unit(t)?.ground && path.contains(&w.unit(t)?.deployment.tile);
        }
        let fire = skill.advance(
            &mut left,
            dt,
            w.unit(id)?.blocked_by().is_none() && has && self.enemy_can_cast(w, id, false)?,
        );
        self.content.as_mut().unwrap().set_lifecycle(
            id,
            Memory::Reid {
                phase,
                left,
                enraged,
            },
        );
        if fire {
            let mut b = buff("ab:rush", &[("moveMul", 1. + speed)]);
            b.runtime.duration = rush;
            b.runtime.visible = true;
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
}
