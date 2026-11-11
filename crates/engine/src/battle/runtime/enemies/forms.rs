// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::content::enemies::forms::{HuskPhase, State, StonePhase};

#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum FormEvent {
    HuskEnter,
    HuskRevive,
    StoneFlight,
}
impl Host {
    pub(super) fn enemy_cleanse_rebirth(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let mut remove = vec![];
        for b in w.unit(id)?.stats.buffs() {
            if b.runtime.persist
                || b.runtime
                    .source
                    .is_some_and(|s| w.unit(s).is_ok_and(|u| u.side == health::Side::Enemy))
            {
                continue;
            }
            if b.runtime.source.is_some()
                || b.runtime.status.is_some()
                || b.key.as_deref() == Some("terrain:infection")
            {
                remove.push(b.runtime.id());
            }
        }
        for buff in remove {
            r.remove_id(w, self, id, buff)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_knockout(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<bool, String> {
        if self.enemy_special_knockout(w, id)? {
            return Ok(true);
        }
        if self.enemy_reborn_knockout(w, r, id)? {
            return Ok(true);
        }
        let Some(form) = self.content.as_ref().unwrap().form(id) else {
            return Ok(false);
        };
        match form {
            State::Husk {
                phase: HuskPhase::Rebirth { until, .. },
                ..
            } if w.time < until => {
                let max = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("maxHp");
                w.unit_mut(id)?.stats.hp = max;
                Ok(true)
            }
            State::Husk {
                config,
                phase: HuskPhase::Living { original },
            } if config.hits > 0. && config.delay > 0. => {
                let original = original.unwrap_or(w.unit(id)?.stats.base_number("maxHp"));
                self.content.as_mut().unwrap().set_form(
                    id,
                    State::Husk {
                        config,
                        phase: HuskPhase::Rebirth {
                            original,
                            until: w.time + 1.,
                        },
                    },
                );
                w.unit_mut(id)?.motion.unbalance_until = f64::NEG_INFINITY;
                self.enemy_cleanse_rebirth(w, r, id)?;
                let hits = (config.hits + 0.5).floor().max(1.);
                let u = w.unit_mut(id)?;
                u.stats.edit_base(|b| {
                    b.0.insert("maxHp".into(), hits);
                });
                u.stats.stats().map_err(str::to_string)?;
                u.stats.hp = hits;
                u.last_attack_at = f64::NEG_INFINITY;
                let mut count = buff("ab:hitCount", &[]);
                count.flags.insert("hitCount".into());
                count.runtime.persist = true;
                self.enemy_buff(w, r, id, count)?;
                let mut husk = buff(config.key(), &[]);
                husk.runtime.persist = true;
                husk.runtime.visible = true;
                husk.flags.insert("disarm".into());
                husk.flags.insert(
                    if config.regen {
                        "unblockable"
                    } else {
                        "stealth"
                    }
                    .into(),
                );
                self.enemy_buff(w, r, id, husk)?;
                let mut reborn = buff(&format!("{}:reborn", config.key()), &[]);
                reborn.runtime.duration = 1.;
                reborn.flags = [
                    "invulnerable",
                    "untargetable",
                    "unblockable",
                    "noMove",
                    "noDisplace",
                ]
                .map(str::to_string)
                .into();
                self.enemy_buff(w, r, id, reborn)?;
                blocking::unblock(
                    w,
                    &mut allies::Blocking {
                        host: self,
                        buffs: r,
                    },
                    id,
                )?;
                if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                    route.points = None;
                }
                self.enemy_schedule(
                    w,
                    Some(id),
                    1.,
                    0.,
                    1,
                    EnemyTask::Form(FormEvent::HuskEnter),
                )?;
                self.enemy_schedule(
                    w,
                    Some(id),
                    1. + config.delay,
                    0.,
                    1,
                    EnemyTask::Form(FormEvent::HuskRevive),
                )?;
                Ok(true)
            }
            State::Stone {
                config,
                phase: StonePhase::Ground,
            } if config.duration > 0. => {
                self.content.as_mut().unwrap().set_form(
                    id,
                    State::Stone {
                        config,
                        phase: StonePhase::Stone,
                    },
                );
                self.enemy_cleanse_rebirth(w, r, id)?;
                let u = w.unit_mut(id)?;
                u.stats.hp = u.stats.stats().map_err(str::to_string)?.n("maxHp");
                u.immune.insert("levitate".into());
                let mut stone = buff(
                    "ab:stone",
                    &[("defFlat", config.def), ("resFlat", config.stone_res)],
                );
                stone.runtime.duration = config.duration;
                stone.runtime.visible = true;
                stone.flags = ["noMove", "selfBound", "unblockable", "noDisplace"]
                    .map(str::to_string)
                    .into();
                self.enemy_buff(w, r, id, stone)?;
                self.enemy_schedule(
                    w,
                    Some(id),
                    config.duration,
                    0.,
                    1,
                    EnemyTask::Form(FormEvent::StoneFlight),
                )?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn enemy_form_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        event: FormEvent,
    ) -> Result<(), String> {
        let Some(state) = self.content.as_ref().unwrap().form(id) else {
            return Ok(());
        };
        match (state, event) {
            (
                State::Husk {
                    config,
                    phase: HuskPhase::Rebirth { original, .. },
                },
                FormEvent::HuskEnter,
            ) => {
                self.content.as_mut().unwrap().set_form(
                    id,
                    State::Husk {
                        config,
                        phase: HuskPhase::Husk { original },
                    },
                );
                if config.shield > 0 {
                    for other in self.enemy_nearby(w, id, 1.8)? {
                        if other == id {
                            continue;
                        }
                        let mut shield = buff("ab:hitShield", &[]);
                        shield.runtime.shield_hits = config.shield;
                        shield.runtime.refresh = buffs::Refresh::Extend;
                        shield.runtime.injury.shield_types =
                            Some(vec![DamageType::Physical, DamageType::Arts]);
                        self.enemy_buff(w, r, other, shield)?;
                    }
                }
            }
            (
                State::Husk {
                    config,
                    phase: HuskPhase::Husk { original },
                },
                FormEvent::HuskRevive,
            ) => {
                self.content.as_mut().unwrap().set_form(
                    id,
                    State::Husk {
                        config,
                        phase: HuskPhase::Living {
                            original: Some(original),
                        },
                    },
                );
                r.remove(w, self, id, "ab:hitCount")?;
                let u = w.unit_mut(id)?;
                u.stats.edit_base(|b| {
                    b.0.insert("maxHp".into(), original);
                });
                u.stats.hp = u.stats.stats().map_err(str::to_string)?.n("maxHp");
                r.remove(w, self, id, config.key())?;
                r.remove(w, self, id, &format!("{}:reborn", config.key()))?;
                if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                    route.points = None;
                }
                w.unit_mut(id)?.cadence.cooldown = 0.;
            }
            (
                State::Stone {
                    config,
                    phase: StonePhase::Stone,
                },
                FormEvent::StoneFlight,
            ) => {
                self.content.as_mut().unwrap().set_form(
                    id,
                    State::Stone {
                        config,
                        phase: StonePhase::Flight,
                    },
                );
                w.unit_mut(id)?.motion_fly = true;
                let mut flight = buff("ab:flight", &[]);
                flight.runtime.persist = true;
                flight.flags.insert("noDisplace".into());
                self.enemy_buff(w, r, id, flight)?;
                if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                    route.points = None;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_form_profile(
        &self,
        id: Id,
        profile: &mut enemy::Profile,
    ) {
        match self.content.as_ref().unwrap().form(id) {
            Some(State::Husk {
                phase: HuskPhase::Rebirth { .. } | HuskPhase::Husk { .. },
                ..
            }) => profile.no_attack = true,
            Some(State::Stone { phase, .. }) => {
                profile.melee = phase != StonePhase::Flight;
                profile.no_attack |= phase == StonePhase::Stone;
                if phase == StonePhase::Flight {
                    profile.mode = enemy::Mode::Damage(DamageType::Arts);
                }
            }
            _ => {}
        }
    }
    pub(in crate::battle::runtime) fn enemy_cancel_form_timers(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        let mut removed = 0;
        self.tactical_timers.retain(|t| {
            let remove = t.source == Some(id)
                && matches!(
                    t.kind,
                    super::super::tactics::Task::Enemy(EnemyTask::Form(_))
                );
            if remove {
                removed += 1;
            }
            !remove
        });
        self.tactical_slots -= removed;
        for _ in 0..removed {
            w.unpin_unit(id)?;
        }
        Ok(())
    }
}
