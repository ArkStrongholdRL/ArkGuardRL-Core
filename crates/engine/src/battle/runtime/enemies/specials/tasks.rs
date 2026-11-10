// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(in crate::battle::runtime) const PARASITE_TICK: u64 = 1707;
pub(in crate::battle::runtime) const TRANSLATOR_FINISH: u64 = 1708;
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum Task {
    BombRelease { target: Id, release: f64 },
    BombWait,
    Devour { target: Id, finish: bool },
    Entrance { hits: u32 },
}
impl Task {
    pub(in crate::battle::runtime) fn target(self) -> Option<Id> {
        match self {
            Self::BombRelease { target, .. } | Self::Devour { target, .. } => Some(target),
            _ => None,
        }
    }
}
impl Host {
    fn bomb_finish(&mut self, w: &mut World, r: &mut buffs::Runtime, id: Id) -> Result<(), String> {
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        if let Some(Special::Bomb { speed, .. }) = self.special_config(id) {
            self.content.as_mut().unwrap().set_special(
                id,
                Memory::Bomb {
                    phase: BombPhase::Done,
                    left: f64::INFINITY,
                },
            );
            w.unit_mut(id)?.enemy.pause_until = w.time;
            if speed > 0. {
                let mut b = buff("ab:bombRun", &[("moveMul", speed)]);
                b.runtime.persist = true;
                self.enemy_buff(w, r, id, b)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_special_task(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        task: Task,
    ) -> Result<bool, String> {
        if !w.unit(id)?.stats.alive {
            return Ok(false);
        }
        match task {
            Task::BombRelease { target, release } => {
                if !matches!(self.content.as_ref().unwrap().special(id),Memory::Bomb{phase:BombPhase::Charging{release:held},..} if held==release)
                {
                    return Ok(false);
                }
                if w.unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("stun")
                {
                    let left = match self.special_config(id) {
                        Some(Special::Bomb { skill: Some(s), .. }) => s.period.max(DT),
                        _ => 1.,
                    };
                    self.content.as_mut().unwrap().set_special(
                        id,
                        Memory::Bomb {
                            phase: BombPhase::Ready,
                            left,
                        },
                    );
                    w.unit_mut(id)?.enemy.pause_until = w.time;
                    return Ok(false);
                }
                self.content.as_mut().unwrap().set_special(
                    id,
                    Memory::Bomb {
                        phase: BombPhase::Dropped {
                            waited: false,
                            landed: false,
                        },
                        left: f64::INFINITY,
                    },
                );
                let attack = Self::tac_stat(w, id, "atk")?;
                self.flights
                    .as_mut()
                    .ok_or("Missing bomb projectiles")?
                    .add(
                        w,
                        projectiles::Launch {
                            from: w.unit(id)?.position,
                            source: Some(id),
                            target: Some(target),
                            to: None,
                            speed: 5.,
                            hit_dead: true,
                            max_age: 10.,
                            on_hit: true,
                            visual: "droneBomb".into(),
                            speed_changeable: None,
                            removable: None,
                            payload: allies::Shot::Bomb(BombShot { source: id, attack }),
                        },
                    )?;
                self.enemy_schedule(
                    w,
                    Some(id),
                    0.667,
                    0.,
                    1,
                    EnemyTask::Special(Task::BombWait),
                )?;
            }
            Task::BombWait => {
                if let Memory::Bomb {
                    phase: BombPhase::Dropped { landed, .. },
                    ..
                } = self.content.as_ref().unwrap().special(id)
                {
                    if landed {
                        self.bomb_finish(w, r, id)?;
                    } else {
                        self.content.as_mut().unwrap().set_special(
                            id,
                            Memory::Bomb {
                                phase: BombPhase::Dropped {
                                    waited: true,
                                    landed,
                                },
                                left: f64::INFINITY,
                            },
                        );
                    }
                }
            }
            Task::Devour { target, finish } => {
                if !w.unit(target)?.stats.alive {
                    return Ok(false);
                }
                if let Some(Special::Cloud {
                    duration,
                    gain,
                    max,
                    ..
                }) = self.special_config(id)
                {
                    if finish {
                        if let Memory::Cloud { left, ammo } =
                            self.content.as_ref().unwrap().special(id)
                        {
                            self.content.as_mut().unwrap().set_special(
                                id,
                                Memory::Cloud {
                                    left,
                                    ammo: (ammo + gain).min(max),
                                },
                            );
                        }
                        self.kill_enemy(target, None, r, w)?;
                    } else {
                        let mut b = buff("ab:devoured", &[]);
                        b.runtime.duration = duration;
                        b.runtime.visible = true;
                        b.flags
                            .extend(["bind", "noMove", "disarm"].map(str::to_string));
                        self.enemy_buff(w, r, target, b)?;
                        self.enemy_schedule(
                            w,
                            Some(id),
                            duration.max(0.),
                            0.,
                            1,
                            EnemyTask::Special(Task::Devour {
                                target,
                                finish: true,
                            }),
                        )?;
                    }
                }
            }
            Task::Entrance { hits } => {
                let mut all = self.special_targets(w, id, f64::INFINITY)?;
                all.sort_by(|&a, &b| {
                    w.unit(b)
                        .unwrap()
                        .stats
                        .hp
                        .total_cmp(&w.unit(a).unwrap().stats.hp)
                });
                if let Some(&target) = all.first() {
                    let tile = w.unit(target)?.deployment.tile;
                    for _ in 0..hits {
                        let mut list = vec![];
                        for t in self.enemy_tile_allies(w, tile, 1)? {
                            if targeting::area_selectable(w, Some(id), t)? {
                                list.push(t);
                            }
                        }
                        list.sort_by(|&a, &b| {
                            w.unit(b)
                                .unwrap()
                                .stats
                                .hp
                                .total_cmp(&w.unit(a).unwrap().stats.hp)
                        });
                        if let Some(&t) = list.first() {
                            let amount = Self::tac_stat(w, id, "atk")?;
                            let mut q = ability_damage(Some(id), t, amount, DamageType::Physical);
                            self.enemy_damage(w, r, &mut q)?;
                        }
                    }
                }
            }
        }
        Ok(true)
    }
    pub(in crate::battle::runtime) fn enemy_bomb_impact(
        &mut self,
        w: &mut World,
        shot: BombShot,
        target: Option<Id>,
        position: [f64; 2],
    ) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing bomb impact buffs")?;
        let result = (|| {
            let tile = if let Some(t) = target {
                w.unit(t)?.deployment.tile
            } else {
                position.map(|v| (v + 0.5).floor() as i32)
            };
            if let Some(t) = target {
                let mut q = ability_damage(Some(shot.source), t, shot.attack, DamageType::Physical);
                self.enemy_damage(w, &mut r, &mut q)?;
            }
            for t in self.enemy_tile_allies(w, tile, 1)? {
                if Some(t) != target && targeting::area_selectable(w, Some(shot.source), t)? {
                    let mut q =
                        ability_damage(Some(shot.source), t, shot.attack, DamageType::Physical);
                    q.tags.push("splash".into());
                    self.enemy_damage(w, &mut r, &mut q)?;
                }
            }
            if let Memory::Bomb {
                phase: BombPhase::Dropped { waited, .. },
                ..
            } = self.content.as_ref().unwrap().special(shot.source)
            {
                if waited {
                    self.bomb_finish(w, &mut r, shot.source)?;
                } else {
                    self.content.as_mut().unwrap().set_special(
                        shot.source,
                        Memory::Bomb {
                            phase: BombPhase::Dropped {
                                waited,
                                landed: true,
                            },
                            left: f64::INFINITY,
                        },
                    );
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime) fn enemy_translator_finish(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(Special::Translator { mods, .. }) = self.special_config(id) else {
            return Ok(());
        };
        let Memory::Translator {
            phase: Translation::Changing(form),
            counts,
            anger,
        } = self.content.as_ref().unwrap().special(id)
        else {
            return Ok(());
        };
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        self.content.as_mut().unwrap().set_special(
            id,
            Memory::Translator {
                phase: Translation::Done(form),
                counts,
                anger,
            },
        );
        r.remove(w, self, id, "ab:origin")?;
        let i = match form {
            Form::Fuchou => 0,
            Form::Youling => 1,
            Form::Shushi => 2,
        };
        let values = mods[i];
        let mut b = buff(
            "ab:form",
            &[
                ("hpFlat", values[0]),
                ("atkFlat", values[1]),
                ("defFlat", values[2]),
                ("resFlat", values[3]),
                ("moveMul", values[4]),
                ("batPct", values[5]),
                ("massFlat", values[6]),
            ],
        );
        b.runtime.persist = true;
        b.runtime.visible = true;
        if form == Form::Youling {
            b.flags.insert("unblockable".into());
        }
        self.enemy_buff(w, r, id, b)?;
        if form != Form::Youling {
            w.unit_mut(id)?.cadence.cooldown = 0.;
        }
        if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
            route.points = None;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_parasite_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let Some(source) = r.get(w, c.unit, c.buff)?.runtime.retained_unit() else {
            return Ok(());
        };
        if let Some(Special::Parasite { scale, .. }) = self.special_config(source) {
            if w.unit(source)?.stats.alive {
                let amount = Self::tac_stat(w, source, "atk")? * scale;
                let mut q = ability_damage(Some(source), c.unit, amount, DamageType::Arts);
                q.ignore_select = true;
                q.is_skill = false;
                q.tags.push("parasite".into());
                self.enemy_damage(w, r, &mut q)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_parasite_element(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        phase: element::Phase,
        q: &mut element::Request,
    ) -> Result<(), String> {
        let target = q.damage.hit.target;
        let source = w
            .unit(target)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some("ab:parasite"))
            .and_then(|b| b.runtime.retained_unit());
        let Some(source) = source else {
            return Ok(());
        };
        match phase {
            element::Phase::Hit => {
                if let Some(Special::Parasite { element, .. }) = self.special_config(source) {
                    if element > 0. && element != 1. {
                        q.damage.damage.mul *= element;
                    }
                }
            }
            element::Phase::Burst => {
                if let Some(el) = q.damage.element {
                    let tile = w.unit(target)?.deployment.tile;
                    for t in self.roster.as_ref().unwrap().ids().to_vec() {
                        let u = w.unit(t)?;
                        if t != target
                            && u.stats.alive
                            && u.deployed
                            && !u.hidden
                            && (u.deployment.tile[0] - tile[0]).abs()
                                + (u.deployment.tile[1] - tile[1]).abs()
                                <= 1
                        {
                            let mut hit =
                                element::Request::new(Some(source), Some(source), t, el, 1000.);
                            hit.damage.ignore_select = true;
                            hit.damage.attack_type = hit::AttackType::None;
                            hit.damage.tags = vec!["enemyAbility".into()];
                            element::apply(w, self, r, &mut hit)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
