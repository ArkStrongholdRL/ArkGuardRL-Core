// SPDX-License-Identifier: GPL-3.0-or-later
//! Tactical Kit callbacks. Timers and projectile payloads obey Field capacities.
use super::super::{
    attacks, body,
    content::operators::*,
    damage::{Damage, DamageType},
    hit,
    operators::Definition,
    projectiles, search, targeting,
};
use super::*;
pub(super) const OBSERVE: u64 = 1401;
pub(super) const WITHER: u64 = 1402;
pub(super) const RAIN: u64 = 1403;
pub(super) const DRIZZLE: u64 = 1404;
pub(super) const SHIELD_LOST: u64 = 1405;
pub(super) const ABNORMAL: &[&str] = &[
    "stun", "freeze", "cold", "sleep", "silence", "fear", "attract", "tremble", "bind", "levitate",
    "disarm", "palsy", "sluggish", "slow",
];
#[derive(Clone, Copy)]
pub(super) enum ArrowKind {
    Snow,
    Pursuit,
}
#[derive(Clone, Copy)]
pub(super) struct Arrow {
    pub source: Id,
    pub kind: ArrowKind,
    pub scale: f64,
    pub hits: u32,
    pub seen: [Id; 5],
    pub count: usize,
}
pub(super) struct Timer {
    pub source: Option<Id>,
    pub seq: u64,
    pub at: f64,
    pub interval: f64,
    pub left: u32,
    pub kind: Task,
}
pub(super) enum Task {
    Enemy(super::enemies::EnemyTask),
    Callback(fn(&mut kits::dispatch::Context<'_>) -> Result<(), String>),
    KeyedCallback {
        key: u64,
        callback: fn(&mut kits::dispatch::Context<'_>, u64) -> Result<(), String>,
    },
    AreaCallback {
        position: [f64; 2],
        attack: f64,
        survives_retreat: bool,
        callback: fn(&mut kits::dispatch::Context<'_>, [f64; 2], f64) -> Result<(), String>,
    },
    Zone {
        position: [f64; 2],
        end: f64,
        radius: f64,
        attack: f64,
        regen: f64,
        weaken: f64,
        wither: f64,
        key: String,
        held: std::collections::BTreeMap<Id, u64>,
        pulse: u32,
    },
    Storm {
        position: [i32; 2],
        scale: f64,
        chain: attacks::Chain,
    },
    Icicle {
        position: [f64; 2],
        scale: f64,
        cold: f64,
    },
    Blast {
        target: Id,
        scale: f64,
        element: f64,
    },
    Explosion {
        target: Id,
        position: [f64; 2],
        scale: f64,
    },
    Gap {
        target: Id,
        buff: u64,
    },
}

impl Host {
    pub(super) fn tac_def(&self, id: Id) -> Result<Arc<Definition>, String> {
        Ok(Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing tactical roster")?
                .entry(id)?
                .definition,
        ))
    }
    pub(super) fn tac_stat(w: &mut World, id: Id, k: &str) -> Result<f64, String> {
        Self::advanced_stat(w, id, k)
    }
    pub(super) fn tac_up(w: &World, id: Id) -> bool {
        w.unit(id).is_ok_and(|u| u.stats.alive && u.deployed)
    }
    pub(super) fn tac_has(w: &World, id: Id, k: &str) -> bool {
        Self::advanced_has(w, id, k)
    }
    pub(super) fn tac_abnormal(w: &World, id: Id) -> bool {
        w.unit(id).is_ok_and(|u| {
            u.stats.buffs().iter().any(|b| {
                b.runtime
                    .status
                    .as_deref()
                    .is_some_and(|k| ABNORMAL.contains(&k))
            })
        })
    }
    pub(super) fn tac_keys(
        &self,
        w: &World,
        id: Id,
        grid: &[[i32; 2]],
    ) -> Result<Vec<usize>, String> {
        Ok(targeting::absolute_range(
            grid,
            w.unit(id)?.deployment.tile,
            self.roster.as_ref().unwrap().entry(id)?.direction,
            0,
        ))
    }
    pub(super) fn tac_enemies(
        &mut self,
        w: &mut World,
        id: Id,
        keys: Option<&[usize]>,
        fly: bool,
    ) -> Result<Vec<Id>, String> {
        let range = match keys {
            Some(k) => k.to_vec(),
            None => self.roster.as_ref().unwrap().range(w, id)?,
        };
        let mut ids = search::Host::enemies(
            self,
            id,
            &range,
            targeting::Profile {
                can_hit_fly: fly,
                hit_sleep: self.tac_def(id)?.combat.profile.selection.hit_sleep,
                ..Default::default()
            },
            w,
            w.unit_capacity(),
        )?;
        targeting::sort_enemies(
            w,
            self,
            id,
            &mut ids,
            self.tac_def(id)?.combat.profile.priority,
        )?;
        Ok(ids)
    }
    pub(super) fn tac_radius(
        &mut self,
        w: &mut World,
        position: [f64; 2],
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        attacks::Hooks::radius_targets(self, position, radius, false, w, w.unit_capacity())
    }
    pub(super) fn tac_selectable(
        &mut self,
        w: &mut World,
        id: Id,
        targets: Vec<Id>,
    ) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for a in targets {
            if targeting::can_target_enemy(
                w,
                self,
                Some(id),
                a,
                targeting::Profile {
                    can_hit_fly: true,
                    ..Default::default()
                },
            )? {
                out.push(a);
            }
        }
        Ok(out)
    }
    pub(super) fn tac_heal_targets(
        &self,
        w: &mut World,
        id: Id,
        keys: &[usize],
        injured: bool,
    ) -> Result<Vec<Id>, String> {
        let mut ids = vec![];
        for a in self.tac_heal_targets_in_order(w, id, keys, injured)? {
            let max = Self::tac_stat(w, a, "maxHp")?;
            ids.push((
                a,
                w.unit(a)?.stats.hp / max,
                w.unit(a)?.deployment.deploy_seq,
            ));
        }
        ids.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
        Ok(ids.into_iter().map(|x| x.0).collect())
    }
    pub(super) fn tac_heal_targets_in_order(
        &self,
        w: &mut World,
        id: Id,
        keys: &[usize],
        injured: bool,
    ) -> Result<Vec<Id>, String> {
        let mut ids = vec![];
        for &a in self.roster.as_ref().unwrap().ids() {
            if !targeting::friendly_selectable(w, id, a)? || !body::in_range(w.unit(a)?, keys) {
                continue;
            }
            let u = w.unit(a)?;
            if a != id && (Self::tac_has(w, a, "noHeal") || u.profile_no_heal) {
                continue;
            }
            let max = Self::tac_stat(w, a, "maxHp")?;
            if injured && w.unit(a)?.stats.hp >= max - 1e-6 {
                continue;
            }
            ids.push(a);
        }
        Ok(ids)
    }
    pub(super) fn tac_heal(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        amount: f64,
    ) -> Result<f64, String> {
        self.support_heal(
            w,
            r,
            Some(id),
            target,
            amount,
            health::HealOptions::default(),
        )
    }
    pub(super) fn tac_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        amount: f64,
        kind: DamageType,
        attack: bool,
        tag: &str,
    ) -> Result<f64, String> {
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(id),
                credit: Some(id),
                target,
                damage_type: kind,
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
        q.is_skill = true;
        q.is_attack = attack;
        if tag == "iceSpike" {
            q.is_splash = true;
            q.attack_type = hit::AttackType::Splash;
        }
        q.tags.push(tag.into());
        if ["zone", "gap", "lordAddition", "mark", "markOwn"].contains(&tag) {
            q.calculation = hit::Calculation::Precomputed;
            q.attack_type = if tag == "zone" {
                hit::AttackType::Buff
            } else {
                hit::AttackType::Addition
            };
        }
        if tag == "zone" {
            q.hit.source = None;
        }
        self.resolve_hit(w, r, &mut q)
    }
    pub(super) fn tac_cleanse(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let buffs: Vec<_> = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .filter(|b| {
                b.runtime
                    .status
                    .as_deref()
                    .is_some_and(|k| ABNORMAL.contains(&k))
            })
            .map(|b| b.runtime.id())
            .collect();
        for buff in buffs {
            r.remove_id(w, self, id, buff)?;
        }
        Ok(())
    }
    pub(super) fn tac_timer(
        &mut self,
        w: &World,
        id: Id,
        at: f64,
        interval: f64,
        left: u32,
        kind: Task,
    ) -> Result<(), String> {
        self.enqueue_timer(Timer {
            source: Some(id),
            seq: w.unit(id)?.deployment.deploy_seq,
            at,
            interval,
            left,
            kind,
        })
    }
    pub(super) fn enqueue_timer(&mut self, timer: Timer) -> Result<(), String> {
        if self.tactical_slots >= self.tactical_limit {
            return Err("Tactical timer capacity exceeded".into());
        }
        self.tactical_slots += 1;
        self.tactical_timers.push(timer);
        Ok(())
    }
    pub(super) fn tac_arrow_launch(
        w: &mut World,
        flights: &mut projectiles::System<allies::Shot>,
        from: [f64; 2],
        target: Id,
        arrow: Arrow,
    ) -> Result<(), String> {
        flights.add(
            w,
            projectiles::Launch {
                from,
                source: Some(arrow.source),
                target: Some(target),
                to: None,
                speed: 16.,
                hit_dead: matches!(arrow.kind, ArrowKind::Pursuit),
                max_age: 10.,
                on_hit: true,
                visual: "arrow".into(),
                speed_changeable: None,
                removable: None,
                payload: allies::Shot::Tactical(arrow),
            },
        )?;
        Ok(())
    }
    pub(super) fn tac_fire(
        &mut self,
        w: &mut World,
        id: Id,
        target: Id,
        kind: ArrowKind,
        scale: f64,
        hits: u32,
    ) -> Result<(), String> {
        let mut seen = [0; 5];
        seen[0] = target;
        Self::tac_arrow_launch(
            w,
            self.flights
                .as_mut()
                .ok_or("Missing tactical projectiles")?,
            w.unit(id)?.position,
            target,
            Arrow {
                source: id,
                kind,
                scale,
                hits,
                seen,
                count: 1,
            },
        )
    }
    pub(super) fn tactical_arrow(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        arrow: Arrow,
        target: Option<Id>,
        position: [f64; 2],
    ) -> Result<(), String> {
        let def = self.tac_def(arrow.source)?;
        let mut scale = arrow.scale;
        if let Tactics::Snhunt(p) = &def.extra.tactics {
            scale = self.snhunt_arrow_scale(w, target, scale, p)?;
        }
        if let Some(t) = target {
            for _ in 0..arrow.hits {
                if !w.unit(t)?.stats.alive {
                    break;
                }
                let amount = Self::tac_stat(w, arrow.source, "atk")? * scale;
                self.tac_damage(
                    w,
                    r,
                    arrow.source,
                    t,
                    amount,
                    DamageType::Physical,
                    false,
                    "specialArrow",
                )?;
            }
        }
        if matches!(arrow.kind, ArrowKind::Pursuit) && arrow.hits > 1 {
            self.archet_continue_arrow(w, arrow, position)?;
        }
        Ok(())
    }

    pub(super) fn tactical_scheduled(&mut self, w: &mut World) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing tactical buffs")?;
        let mut ids = std::mem::take(&mut self.scratch);
        let result: Result<(), String> = (|| {
            self.flush_kit_events(w, &mut r)?;
            ids.clear();
            ids.extend_from_slice(self.roster.as_ref().unwrap().ids());
            enum Due {
                Timer(Timer),
                KitAura(Id),
                Periodic(Id, super::advanced::Periodic),
                Equipment(Id, super::super::equipment::Timer),
                Bond(super::bonds::Periodic),
            }
            // Fixed operator periods and dynamically created timers share deadline order.
            // Periodic registrations precede combat-created timers when deadlines are equal.
            let mut due: Vec<_> = self
                .advanced_deadlines(w)?
                .into_iter()
                .map(|(at, id, kind)| (at, Due::Periodic(id, kind)))
                .collect();
            for &id in &ids {
                let definition = &self.roster.as_ref().unwrap().entry(id)?.definition;
                if matches!(definition.extra.tactics, Tactics::Ines(_)) {
                    let at = w.unit(id)?.operator.tactical.next[1].max(0.2);
                    if at <= w.time + 1e-9 {
                        due.push((at, Due::KitAura(id)));
                    }
                }
            }
            due.extend(
                self.bond_deadlines(w)
                    .into_iter()
                    .map(|(at, kind)| (at, Due::Bond(kind))),
            );
            due.extend(
                self.equipment_deadlines(w)
                    .into_iter()
                    .map(|(at, unit, index)| (at, Due::Equipment(unit, index))),
            );
            due.extend(
                std::mem::take(&mut self.tactical_timers)
                    .into_iter()
                    .map(|timer| (timer.at, Due::Timer(timer))),
            );
            due.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (_, event) in due {
                let mut timer = match event {
                    Due::Timer(timer) => timer,
                    Due::KitAura(id) => {
                        let next = &mut w.unit_mut(id)?.operator.tactical.next[1];
                        *next = next.max(0.2) + 0.2;
                        self.kit_event_for(
                            w,
                            &mut r,
                            id,
                            &mut kits::dispatch::Event::Scheduled([
                                false, true, false, false, false,
                            ]),
                        )?;
                        continue;
                    }
                    Due::Periodic(id, kind) => {
                        self.advanced_periodic(w, &mut r, &ids, id, kind)?;
                        continue;
                    }
                    Due::Bond(kind) => {
                        self.bond_periodic(w, &mut r, kind)?;
                        continue;
                    }
                    Due::Equipment(unit, index) => {
                        self.equipment_periodic(w, &mut r, unit, index)?;
                        continue;
                    }
                };
                if w.time + 1e-9 < timer.at {
                    self.tactical_timers.push(timer);
                    continue;
                }
                let mut complete = true;
                if self.tac_run_timer(w, &mut r, &mut timer)? {
                    timer.left -= 1;
                    timer.at += timer.interval;
                    complete = timer.left == 0;
                }
                if complete {
                    self.tactical_slots -= 1;
                    if let Task::Enemy(task) = timer.kind {
                        for reference in
                            [timer.source, task.retained_target()].into_iter().flatten()
                        {
                            w.unpin_unit(reference)?;
                        }
                    }
                } else {
                    self.tactical_timers.push(timer);
                }
            }
            self.tactical_sync_zones(w, &mut r)?;
            self.dispatch_scheduled_kits(w, &mut r)?;
            self.tactical_sync_injury(w, &mut r)?;
            Ok(())
        })();
        self.scratch = ids;
        self.buffs = Some(r);
        result
    }
    pub(super) fn tac_leader(&self, w: &World, id: Id) -> Result<bool, String> {
        let character = self.tac_def(id)?.char_id.clone();
        Ok(self
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .find(|&a| Self::tac_up(w, a) && self.tac_def(a).is_ok_and(|d| d.char_id == character))
            == Some(id))
    }

    pub(super) fn tac_run_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        if let Task::Enemy(task) = &mut t.kind {
            return self.enemy_task(w, r, t.source, task, t.left == 1);
        }
        let id = t.source.ok_or("Operator timer requires a source")?;
        if !matches!(
            t.kind,
            Task::Zone { .. }
                | Task::Blast { .. }
                | Task::Gap { .. }
                | Task::AreaCallback {
                    survives_retreat: true,
                    ..
                }
        ) && (!Self::tac_up(w, id) || w.unit(id)?.deployment.deploy_seq != t.seq)
        {
            return Ok(false);
        }
        match &mut t.kind {
            Task::Enemy(_) => unreachable!("Enemy timers are handled before operator dispatch"),
            Task::KeyedCallback { key, callback } => {
                callback(
                    &mut kits::dispatch::Context {
                        host: self,
                        world: w,
                        buffs: r,
                        unit: id,
                    },
                    *key,
                )?;
                Ok(true)
            }
            Task::AreaCallback {
                position,
                attack,
                callback,
                ..
            } => {
                callback(
                    &mut kits::dispatch::Context {
                        host: self,
                        world: w,
                        buffs: r,
                        unit: id,
                    },
                    *position,
                    *attack,
                )?;
                Ok(true)
            }
            Task::Callback(callback) => {
                callback(&mut kits::dispatch::Context {
                    host: self,
                    world: w,
                    buffs: r,
                    unit: id,
                })?;
                Ok(true)
            }
            Task::Zone { .. } => self.tinman_zone_timer(w, r, t),
            Task::Storm { .. } => self.pasngr_storm_timer(w, r, t),
            Task::Icicle { .. } => self.sntlla_icicle_timer(w, r, t),
            Task::Blast { .. } => self.philae_blast_timer(w, r, t),
            Task::Explosion { .. } => self.qiubai_explosion_timer(w, r, t),
            Task::Gap { .. } => self.qiubai_gap_timer(w, r, t),
        }
    }

    pub(super) fn tac_faction(&self, id: Id, faction: &str) -> Result<bool, String> {
        let def = self.tac_def(id)?;
        Ok(self.data.table("chess").unwrap()[&def.key]["nationId"] == faction)
    }
}
pub(super) const GAP_FINISH: u64 = 1406;
