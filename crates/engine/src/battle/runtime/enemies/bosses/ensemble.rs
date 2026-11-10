// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::bosses::{ensemble, EchoForm, Pipe},
    element::Element,
};
use crate::deterministic::math::hypot;
#[derive(Clone, Copy)]
pub(super) struct AreaStrike {
    pub position: [f64; 2],
    pub radius: f64,
    pub scale: f64,
    pub element: Element,
    pub ratio: f64,
    pub attack: Option<f64>,
}
impl Host {
    pub(super) fn boss_arts_area(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        strike: AreaStrike,
    ) -> Result<(), String> {
        for target in self.boss_area(w, id)? {
            let p = w.unit(target)?.position;
            if hypot(p[1] - strike.position[1], p[0] - strike.position[0]) > strike.radius {
                continue;
            }
            let atk = match strike.attack {
                Some(atk) => atk,
                None => Self::tac_stat(w, id, "atk")?,
            };
            let mut q = ability_damage(Some(id), target, atk * strike.scale, DamageType::Arts);
            self.enemy_damage(w, r, &mut q)?;
            let atk = match strike.attack {
                Some(atk) => atk,
                None => Self::tac_stat(w, id, "atk")?,
            };
            self.enemy_element(w, r, id, target, strike.element, atk * strike.ratio)?;
        }
        Ok(())
    }
    fn echoes(&self, w: &World, form: EchoForm) -> Vec<Id> {
        self.spawns.as_ref().unwrap().live().iter().copied().filter(|&target| {
            w.unit(target).is_ok_and(|unit|unit.stats.alive)
                && matches!(self.content.as_ref().unwrap().boss(target),Memory::Echo(state) if state.form==form)
        }).collect()
    }
    pub(in crate::battle::runtime) fn echo_form(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        form: EchoForm,
    ) -> Result<(), String> {
        let Some(Boss::Echo(config)) = self.boss_config(id) else {
            return Ok(());
        };
        let Memory::Echo(mut state) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        state.form = form;
        state.strikes = 0;
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Echo(state));
        self.enemy_buff(w, r, id, ensemble::form_buff(config, form))
    }
    pub(in crate::battle::runtime) fn echo_hit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(Boss::Echo(config)) = self.boss_config(id) else {
            return Ok(());
        };
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        let Memory::Echo(mut state) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        if state.pulses.len() >= w.unit_capacity() {
            return Err("Echo pulse capacity exceeded".into());
        }
        state.pulses.push_back(Self::tac_stat(w, id, "atk")?);
        let active = state.pulsing;
        state.pulsing = true;
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Echo(state));
        if active {
            return Ok(());
        }
        let result = (|| {
            loop {
                let Memory::Echo(mut state) = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                let Some(atk) = state.pulses.pop_front() else {
                    break;
                };
                self.content
                    .as_mut()
                    .unwrap()
                    .set_boss(id, Memory::Echo(state));
                self.boss_arts_area(
                    w,
                    r,
                    id,
                    AreaStrike {
                        position: w.unit(id)?.position,
                        radius: 1.6,
                        scale: config.pulse,
                        element: Element::Apoptosis,
                        ratio: config.element,
                        attack: Some(atk),
                    },
                )?;
                let Memory::Echo(mut state) = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                state.strikes = state
                    .strikes
                    .checked_add(1)
                    .ok_or("Echo strike count exhausted")?;
                let need = config.strikes[usize::from(state.form == EchoForm::Gold)];
                let switch = w.unit(id)?.stats.alive && need > 0. && state.strikes as f64 >= need;
                let next = if state.form == EchoForm::Gold {
                    EchoForm::Dark
                } else {
                    EchoForm::Gold
                };
                self.content
                    .as_mut()
                    .unwrap()
                    .set_boss(id, Memory::Echo(state));
                if switch {
                    self.echo_form(w, r, id, next)?;
                }
            }
            Ok(())
        })();
        let Memory::Echo(mut state) = self.content.as_ref().unwrap().boss(id) else {
            unreachable!()
        };
        state.pulsing = false;
        state.pulses.clear();
        self.content
            .as_mut()
            .unwrap()
            .set_boss(id, Memory::Echo(state));
        result
    }
    pub(in crate::battle::runtime) fn echo_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        source: Option<Id>,
        credit: Option<Id>,
    ) -> Result<(), String> {
        if source
            .or(credit)
            .is_some_and(|t| w.unit(t).is_ok_and(|u| u.side == health::Side::Ally))
        {
            self.echo_hit(w, r, id)?;
        }
        Ok(())
    }
    pub(super) fn ensemble_condition(&self, w: &World, spell: Spell) -> Option<bool> {
        match spell {
            Spell::PipeStrike { gold, .. } => Some(
                !self
                    .echoes(w, if gold { EchoForm::Gold } else { EchoForm::Dark })
                    .is_empty(),
            ),
            Spell::Ensemble => Some(true),
            _ => None,
        }
    }
    pub(super) fn ensemble_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
    ) -> Result<(), String> {
        match spell {
            Spell::Ensemble => {
                let Some(Boss::Echo(config)) = self.boss_config(id) else {
                    unreachable!()
                };
                let Memory::Echo(state) = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                self.boss_arts_area(
                    w,
                    r,
                    id,
                    AreaStrike {
                        position: w.unit(id)?.position,
                        radius: if state.form == EchoForm::Gold {
                            0.8
                        } else {
                            1.6
                        },
                        scale: 1.,
                        element: Element::Apoptosis,
                        ratio: config.ensemble[usize::from(state.form == EchoForm::Gold)],
                        attack: None,
                    },
                )?;
            }
            Spell::PipeStrike {
                gold,
                radius,
                element,
            } => {
                for echo in self.echoes(w, if gold { EchoForm::Gold } else { EchoForm::Dark }) {
                    if gold {
                        self.boss_arts_area(
                            w,
                            r,
                            id,
                            AreaStrike {
                                position: w.unit(echo)?.position,
                                radius,
                                scale: 1.,
                                element: Element::Apoptosis,
                                ratio: element,
                                attack: None,
                            },
                        )?;
                        self.echo_form(w, r, echo, EchoForm::Dark)?;
                    } else {
                        for k in 0..3 {
                            self.enemy_schedule(
                                w,
                                Some(id),
                                k as f64 * 0.4,
                                0.,
                                1,
                                EnemyTask::Boss(BossTask::Pipe {
                                    echo,
                                    radius,
                                    element,
                                }),
                            )?;
                        }
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    pub(super) fn pipe_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Pipe,
        dt: f64,
    ) -> Result<(), String> {
        let Memory::Pipe {
            mut acc,
            mut attack,
            mut held_until,
        } = self.content.as_ref().unwrap().boss(id)
        else {
            unreachable!()
        };
        acc += dt;
        let low = w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? < config.threshold;
        if acc >= config.intervals[usize::from(low)] {
            acc = 0.;
            let key = self
                .content
                .as_ref()
                .unwrap()
                .child(id)
                .unwrap_or_else(|| Arc::from("enemy_9023_acdums"));
            for t in self.boss_spawn_branch(w, r, id, "summon_enemy", &key, Value::Null, "WALK")? {
                if config.gold {
                    self.echo_form(w, r, t, EchoForm::Gold)?;
                }
            }
        }
        if self.enemy_can_cast(w, id, false)? {
            attack += dt;
            if attack >= Self::tac_stat(w, id, "interval")? && w.time >= held_until {
                let form = if config.gold {
                    EchoForm::Gold
                } else {
                    EchoForm::Dark
                };
                let targets = self.echoes(w, form);
                if let Some(&target) = self.rng.pick(&targets) {
                    attack = 0.;
                    self.content.as_mut().unwrap().set_boss(
                        id,
                        Memory::Pipe {
                            acc,
                            attack,
                            held_until,
                        },
                    );
                    if let Some(clip) = self.caster_turn(w, r, id, false, None)? {
                        held_until = w.time + clip;
                    } else {
                        let now = w.time;
                        let u = w.unit_mut(id)?;
                        u.last_attack_at = now;
                        u.attacks += 1;
                        self.echo_hit(w, r, target)?;
                    }
                }
            }
        }
        self.content.as_mut().unwrap().set_boss(
            id,
            Memory::Pipe {
                acc,
                attack,
                held_until,
            },
        );
        self.enemy_caster_tick(w, r, id, config.caster, dt)
    }
    pub(in crate::battle::runtime::enemies) fn pipe_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if matches!(
            self.boss_config(id),
            Some(Boss::Pipe(Pipe { gold: false, .. }))
        ) {
            let ids = self.spawns.as_ref().unwrap().live().to_vec();
            for t in ids {
                if w.unit(t)?.stats.alive
                    && matches!(
                        self.boss_config(t),
                        Some(Boss::Pipe(Pipe { gold: true, .. }))
                    )
                {
                    self.kill_enemy(t, None, r, w)?;
                }
            }
        }
        Ok(())
    }
}
