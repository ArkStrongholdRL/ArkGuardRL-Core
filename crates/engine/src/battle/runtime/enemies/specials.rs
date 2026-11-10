// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::specials::{BombPhase, Charge, Form, Memory, Special, Translation},
    element, projectiles, targeting,
};
use crate::deterministic::math::hypot;
mod tasks;
pub(in crate::battle::runtime) use tasks::{Task, PARASITE_TICK, TRANSLATOR_FINISH};
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) struct BombShot {
    pub source: Id,
    pub attack: f64,
}
impl Host {
    fn special_config(&self, id: Id) -> Option<Special> {
        match self.content.as_ref()?.kit(id) {
            Kit::Special(c) => Some(c),
            _ => None,
        }
    }
    pub(in crate::battle::runtime) fn enemy_special_spawn(
        &mut self,
        w: &mut World,
        id: Id,
        config: Special,
    ) -> Result<(), String> {
        if let Special::Armor { appear, .. } = config {
            self.enemy_schedule(
                w,
                Some(id),
                0.5,
                0.,
                1,
                EnemyTask::Special(Task::Entrance { hits: appear }),
            )?;
        }
        Ok(())
    }
    fn special_targets(&mut self, w: &mut World, id: Id, radius: f64) -> Result<Vec<Id>, String> {
        let mut out = if radius.is_infinite() {
            let mut v = vec![];
            for t in self.roster.as_ref().unwrap().ids() {
                if targeting::can_target_ally(w, id, *t, true, Default::default())? {
                    v.push(*t);
                }
            }
            v
        } else {
            self.enemy_targets_near(w, id, radius)?
        };
        if !radius.is_infinite() {
            targeting::sort_allies(w, id, &mut out, Default::default())?;
        }
        Ok(out)
    }
    fn cloud_prey(&mut self, w: &mut World, id: Id, radius: f64) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for t in self.enemy_nearby(w, id, radius)? {
            if t == id
                || w.unit(t)?.boss
                || self.data.table("enemies").unwrap()
                    [&self.spawns.as_ref().unwrap().definition(t)?.key]["rank"]
                    .as_str()
                    .unwrap_or("NORMAL")
                    != "NORMAL"
                || !targeting::flying(w, t)?
                || w.unit(t)?
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some("ab:devoured"))
            {
                continue;
            }
            out.push(t);
        }
        let p = w.unit(id)?.position;
        out.sort_by(|&a, &b| {
            let a = w.unit(a).unwrap().position;
            let b = w.unit(b).unwrap().position;
            hypot(a[0] - p[0], a[1] - p[1]).total_cmp(&hypot(b[0] - p[0], b[1] - p[1]))
        });
        Ok(out)
    }
    pub(in crate::battle::runtime) fn enemy_special_profile(
        &self,
        id: Id,
        kit: &Kit,
        p: &mut enemy::Profile,
    ) {
        let Kit::Special(config) = kit else {
            return;
        };
        match config {
            Special::Catapult { .. } => {
                p.deferred = true;
                p.selection.block_free = true;
            }
            Special::Bomb { .. } => p.no_attack = true,
            Special::Translator { .. } => match self.content.as_ref().unwrap().special(id) {
                Memory::Translator {
                    phase: Translation::Done(form),
                    ..
                } => {
                    p.no_attack = form == Form::Youling;
                    p.melee = form == Form::Fuchou;
                    p.mode = enemy::Mode::Damage(if form == Form::Shushi {
                        DamageType::Arts
                    } else {
                        DamageType::Physical
                    });
                    p.max_targets = if form == Form::Shushi { 2. } else { 1. };
                }
                _ => p.no_attack = true,
            },
            _ => {}
        }
    }
    pub(in crate::battle::runtime) fn enemy_special_before_status(&self, c: &mut status::Context) {
        if !matches!(
            c.key.as_str(),
            "stun" | "sleep" | "cold" | "freeze" | "levitate" | "fear"
        ) {
            return;
        }
        if let Some(content) = &self.content {
            if matches!(
                content.special(c.target),
                Memory::Translator {
                    phase: Translation::Origin | Translation::Pending(_) | Translation::Changing(_),
                    ..
                }
            ) {
                c.cancelled = true;
            }
        }
    }
    pub(in crate::battle::runtime) fn enemy_special_blocked(&mut self, id: Id) {
        if let Some(content) = &mut self.content {
            if let Memory::Translator {
                phase: Translation::Origin,
                counts,
                anger,
            } = content.special(id)
            {
                content.set_special(
                    id,
                    Memory::Translator {
                        phase: Translation::Pending(Form::Youling),
                        counts,
                        anger,
                    },
                );
            }
        }
    }
    pub(in crate::battle::runtime) fn enemy_special_flush(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if let Memory::Translator {
            phase: Translation::Pending(form),
            counts,
            anger,
        } = self.content.as_ref().unwrap().special(id)
        {
            self.content.as_mut().unwrap().set_special(
                id,
                Memory::Translator {
                    phase: Translation::Changing(form),
                    counts,
                    anger,
                },
            );
            let mut b = buff("ab:change", &[]);
            b.runtime.duration = 2.;
            b.runtime.persist = true;
            b.flags.insert("noMove".into());
            b.runtime.on_expire = Some(TRANSLATOR_FINISH);
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_special_in(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        let id = q.hit.target;
        let Some(Special::Translator { counts: limits, .. }) = self.special_config(id) else {
            return Ok(());
        };
        let Memory::Translator {
            mut phase,
            mut counts,
            anger,
        } = self.content.as_ref().unwrap().special(id)
        else {
            unreachable!()
        };
        if matches!(phase, Translation::Done(_)) {
            return Ok(());
        }
        if phase == Translation::Origin
            && q.hit
                .source
                .or(q.hit.credit)
                .is_some_and(|s| w.unit(s).is_ok_and(|u| u.side == health::Side::Ally))
        {
            let kind = match q.hit.damage_type {
                DamageType::Physical => Some((0, Form::Fuchou)),
                DamageType::Arts => Some((1, Form::Shushi)),
                _ => None,
            };
            if let Some((i, form)) = kind {
                counts[i] = counts[i].saturating_add(1);
                if counts[i] >= limits[i] {
                    phase = Translation::Pending(form);
                }
            }
        }
        self.content.as_mut().unwrap().set_special(
            id,
            Memory::Translator {
                phase,
                counts,
                anger,
            },
        );
        self.enemy_special_flush(w, r, id)?;
        q.cancelled = true;
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_special_knockout(
        &self,
        w: &mut World,
        id: Id,
    ) -> Result<bool, String> {
        if matches!(
            self.content.as_ref().unwrap().special(id),
            Memory::Translator {
                phase: Translation::Origin | Translation::Pending(_) | Translation::Changing(_),
                ..
            }
        ) {
            w.unit_mut(id)?.stats.hp = w.unit(id)?.stats.hp.max(1.);
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn enemy_special_hit(
        &self,
        w: &World,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        if let Some(Special::Armor { ranged, .. }) = self.special_config(id) {
            if w.unit(id)?.blocked_by() != Some(q.hit.target) {
                q.damage.amount *= ranged;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_special_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<bool, String> {
        let id = c.enemy;
        let Some(Special::Cloud {
            fire: Some(fire),
            cost,
            scale,
            ..
        }) = self.special_config(id)
        else {
            return Ok(false);
        };
        let Memory::Cloud { mut left, ammo } = self.content.as_ref().unwrap().special(id) else {
            unreachable!()
        };
        if event != enemy::Event::BeforeAttack
            || left[1] > 1e-9
            || ammo < cost
            || !self.enemy_can_cast(w, id, true)?
            || self.special_targets(w, id, f64::INFINITY)?.is_empty()
        {
            return Ok(false);
        }
        left[1] = (fire.period + 2.).max(DT);
        self.content
            .as_mut()
            .unwrap()
            .set_special(id, Memory::Cloud { left, ammo: 0. });
        for _ in 0..ammo.floor() as usize {
            let targets = self.special_targets(w, id, f64::INFINITY)?;
            if let Some(&t) = self.rng.pick(&targets) {
                let amount = Self::tac_stat(w, id, "atk")? * scale;
                let mut q = ability_damage(Some(id), t, amount, DamageType::Physical);
                self.enemy_damage(w, r, &mut q)?;
            } else {
                break;
            }
        }
        w.unit_mut(id)?.cadence.cooldown = Self::tac_stat(w, id, "interval")?.max(2.);
        w.unit_mut(id)?.motion.attack_stand_until = f64::NEG_INFINITY;
        c.targets.clear();
        Ok(true)
    }
    pub(super) fn enemy_special_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        if let Memory::Armor {
            left,
            combo,
            appeared,
        } = self.content.as_ref().unwrap().special(id)
        {
            if combo > 1 {
                self.content.as_mut().unwrap().set_special(
                    id,
                    Memory::Armor {
                        left,
                        combo: 0,
                        appeared,
                    },
                );
                for _ in 1..combo {
                    let amount = Self::tac_stat(w, id, "atk")?;
                    let mut hit =
                        ability_damage(Some(id), q.hit.target, amount, DamageType::Physical);
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_special_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Special,
        dt: f64,
    ) -> Result<(), String> {
        self.enemy_special_flush(w, r, id)?;
        match (config, self.content.as_ref().unwrap().special(id)) {
            (
                Special::Bomb {
                    skill: Some(skill), ..
                },
                Memory::Bomb { phase, mut left },
            ) => {
                if let BombPhase::Charging { .. } = phase {
                    if w.unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("stun")
                    {
                        left = skill.period.max(DT) - dt;
                        self.content.as_mut().unwrap().set_special(
                            id,
                            Memory::Bomb {
                                phase: BombPhase::Ready,
                                left,
                            },
                        );
                        w.unit_mut(id)?.enemy.pause_until = w.time;
                    }
                    return Ok(());
                }
                if !matches!(phase, BombPhase::Ready) {
                    return Ok(());
                }
                left -= dt;
                if left <= 1e-9 && self.enemy_can_cast(w, id, false)? {
                    let radius = w.unit(id)?.stats.base_number("rangeRadius");
                    if let Some(&target) = self
                        .special_targets(w, id, if radius == 0. { 2. } else { radius })?
                        .first()
                    {
                        let release = w.time + 8. / 30.;
                        self.content.as_mut().unwrap().set_special(
                            id,
                            Memory::Bomb {
                                phase: BombPhase::Charging { release },
                                left: f64::INFINITY,
                            },
                        );
                        w.unit_mut(id)?.enemy.pause_until = w.time + 8. / 30. + 10. + 0.667;
                        self.enemy_schedule(
                            w,
                            Some(id),
                            8. / 30.,
                            0.,
                            1,
                            EnemyTask::Special(Task::BombRelease { target, release }),
                        )?;
                        return Ok(());
                    }
                }
                self.content
                    .as_mut()
                    .unwrap()
                    .set_special(id, Memory::Bomb { phase, left });
            }
            (
                Special::Cloud {
                    grab,
                    fire,
                    radius,
                    duration,
                    ..
                },
                Memory::Cloud { mut left, ammo },
            ) => {
                left[0] -= dt;
                left[1] -= dt;
                self.content
                    .as_mut()
                    .unwrap()
                    .set_special(id, Memory::Cloud { left, ammo });
                if let Some(grab) = grab {
                    if left[0] <= 1e-9 && self.enemy_can_cast(w, id, true)? {
                        let targets = self.cloud_prey(w, id, radius)?;
                        if !targets.is_empty() {
                            left[0] = grab.period.max(DT);
                            self.content
                                .as_mut()
                                .unwrap()
                                .set_special(id, Memory::Cloud { left, ammo });
                            let mut b = buff("ab:grabbing", &[]);
                            b.runtime.duration = duration;
                            b.flags.extend(["bind", "noMove"].map(str::to_string));
                            self.enemy_buff(w, r, id, b)?;
                            for &target in targets.iter().take(3) {
                                self.enemy_schedule(
                                    w,
                                    Some(id),
                                    0.5,
                                    0.,
                                    1,
                                    EnemyTask::Special(Task::Devour {
                                        target,
                                        finish: false,
                                    }),
                                )?;
                            }
                        }
                    }
                }
                if fire.is_some() && !w.unit(id)?.enemy.swing && w.unit(id)?.cadence.cooldown <= 0.
                {
                    let flags = &w.unit_mut(id)?.stats.stats().map_err(str::to_string)?.flags;
                    if !flags.contains("fear")
                        && !flags.contains("disarm")
                        && !(flags.contains("tremble") && w.unit(id)?.blocked_by().is_some())
                    {
                        let mut c = enemy::Context {
                            enemy: id,
                            targets: vec![],
                            deployments: vec![],
                            palsy: None,
                            keep: false,
                            attack_id: None,
                        };
                        self.enemy_special_attack(w, r, &mut c, enemy::Event::BeforeAttack)?;
                    }
                }
            }
            (
                Special::Armor {
                    skill,
                    appear: _,
                    times,
                    ..
                },
                Memory::Armor {
                    mut left,
                    mut combo,
                    appeared,
                },
            ) => {
                if !appeared {
                    self.enemy_special_spawn(w, id, config)?;
                }
                if let Some(skill) = skill {
                    if skill.advance(&mut left, dt, self.enemy_can_cast(w, id, false)?) {
                        combo = times;
                    }
                }
                self.content.as_mut().unwrap().set_special(
                    id,
                    Memory::Armor {
                        left,
                        combo,
                        appeared: true,
                    },
                );
            }
            (
                Special::Translator { anger: atk, .. },
                Memory::Translator {
                    phase: Translation::Done(Form::Fuchou),
                    counts,
                    anger,
                },
            ) => {
                let on = w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? < 0.5;
                if on != anger {
                    self.content.as_mut().unwrap().set_special(
                        id,
                        Memory::Translator {
                            phase: Translation::Done(Form::Fuchou),
                            counts,
                            anger: on,
                        },
                    );
                    if on {
                        let mut b = buff("ab:anger", &[("atkPct", atk)]);
                        b.runtime.persist = true;
                        b.runtime.visible = true;
                        self.enemy_buff(w, r, id, b)?;
                    } else {
                        r.remove(w, self, id, "ab:anger")?;
                    }
                }
            }
            (
                Special::Elk {
                    skill,
                    duration,
                    scale,
                    stun,
                },
                Memory::Elk {
                    mut left,
                    mut charge,
                    previous,
                    hidden,
                },
            ) => {
                let u = w.unit(id)?;
                let p = u.position;
                let was_hidden = hidden;
                let hidden = u.hidden;
                let own_speed = Self::tac_stat(w, id, "moveSpeed")?;
                let forced = previous.is_some_and(|old| {
                    !was_hidden
                        && !hidden
                        && hypot(p[0] - old[0], p[1] - old[1]) - own_speed * 0.5 * dt * 1.5 - 0.001
                            > 0.05
                });
                if let Some(ch) = charge {
                    let flags = w
                        .unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .clone();
                    if flags.contains("stun")
                        || !w.unit(ch.target)?.stats.alive
                        || w.unit(id)?.blocked_by() != Some(ch.target)
                    {
                        charge = None;
                        r.remove(w, self, id, "ab:elkCharge")?;
                    } else if w.time + 1e-9 >= ch.until
                        && w.time >= w.unit(id)?.motion.unbalance_until
                    {
                        charge = None;
                        r.remove(w, self, id, "ab:elkCharge")?;
                        let amount = Self::tac_stat(w, id, "atk")? * scale;
                        let mut q =
                            ability_damage(Some(id), ch.target, amount, DamageType::Physical);
                        self.enemy_damage(w, r, &mut q)?;
                        if stun > 0. && w.unit(ch.target)?.stats.alive {
                            self.apply_status(
                                w,
                                r,
                                ch.target,
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
                if forced && charge.is_some() {
                    charge = None;
                    r.remove(w, self, id, "ab:elkCharge")?;
                }
                if let Some(skill) = skill {
                    if skill.advance(
                        &mut left,
                        dt,
                        charge.is_none()
                            && w.unit(id)?.blocked_by().is_some()
                            && self.enemy_can_cast(w, id, false)?,
                    ) {
                        let target = w.unit(id)?.blocked_by().unwrap();
                        charge = Some(Charge {
                            target,
                            until: w.time + duration,
                        });
                        let mut b = buff("ab:elkCharge", &[]);
                        b.runtime.duration = duration + 0.1;
                        b.runtime.visible = true;
                        b.flags.insert("disarm".into());
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
                self.content.as_mut().unwrap().set_special(
                    id,
                    Memory::Elk {
                        left,
                        charge,
                        previous: Some(p),
                        hidden,
                    },
                );
            }
            (Special::Parasite { .. }, Memory::Parasite { .. }) => {
                let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.2);
                for _ in 0..pulses {
                    let Memory::Parasite { host } = self.content.as_ref().unwrap().special(id)
                    else {
                        unreachable!()
                    };
                    let next = w
                        .unit(id)?
                        .blocked_by()
                        .filter(|&t| w.unit(t).is_ok_and(|u| u.stats.alive));
                    if host != next {
                        self.parasite_detach(w, r, id)?;
                        if let Some(target) = next {
                            w.pin_unit(target)?;
                            self.content
                                .as_mut()
                                .unwrap()
                                .set_special(id, Memory::Parasite { host: Some(target) });
                            w.unit_mut(id)?.position =
                                w.unit(target)?.deployment.tile.map(f64::from);
                            if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                                route.points = None;
                            }
                            let mut b = buff("ab:parasiteAttached", &[]);
                            b.flags.extend(["noMove", "selfBound"].map(str::to_string));
                            self.enemy_buff(w, r, id, b)?;
                            let mut b = buff("ab:parasite", &[]);
                            b.runtime.visible = true;
                            b.runtime.interval = 1.;
                            b.runtime.on_tick = Some(PARASITE_TICK);
                            r.add(
                                w,
                                self,
                                target,
                                buffs::Spec {
                                    buff: b,
                                    ..Default::default()
                                }
                                .retaining_unit(id),
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn parasite_detach(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if let Memory::Parasite { host: Some(target) } = self.content.as_ref().unwrap().special(id)
        {
            self.content
                .as_mut()
                .unwrap()
                .set_special(id, Memory::Parasite { host: None });
            r.remove(w, self, target, "ab:parasite")?;
            r.remove(w, self, id, "ab:parasiteAttached")?;
            w.unpin_unit(target)?;
        }
        Ok(())
    }
}
