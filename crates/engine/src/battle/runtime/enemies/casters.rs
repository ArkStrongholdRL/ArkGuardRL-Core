// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::casters::{Caster, Passive, Skill, Spell},
    motion::Leg,
    targeting,
};
use crate::deterministic::math::hypot;
mod champions;
mod tasks;
pub(in crate::battle::runtime) use champions::{FEAR_CAGE, WOLF_REBORN, XI_SHIELD};
pub(in crate::battle::runtime) use tasks::{
    SkillShot, SpellTask, ARTS_BARRIER, BURN_TICK, SAND_TICK,
};
impl Host {
    fn caster_config(&self, id: Id) -> Option<&Caster> {
        let kit = self.content.as_ref().unwrap().kit_ref(id);
        self.caster_config_from(id, kit)
    }
    fn caster_config_from<'a>(&'a self, id: Id, kit: &'a Kit) -> Option<&'a Caster> {
        match kit {
            Kit::Caster(c) => Some(c),
            Kit::Boss(content::Boss::Quintus(_)) => {
                self.content.as_ref().unwrap().quintus_caster_ref(id)
            }
            Kit::Boss(c) => Some(c.caster_ref().unwrap_or(&content::casters::EMPTY)),
            _ => None,
        }
    }
    pub(super) fn caster_targets(
        &mut self,
        w: &mut World,
        id: Id,
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        let mut targets = if radius.is_infinite() {
            self.roster.as_ref().unwrap().ids().to_vec()
        } else {
            enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?
        };
        let mut out = vec![];
        for target in targets.drain(..) {
            if targeting::can_target_ally(w, id, target, true, Default::default())? {
                out.push(target);
            }
        }
        Ok(out)
    }
    fn cross_targets(
        &mut self,
        w: &mut World,
        id: Id,
        direction: Option<[i32; 2]>,
    ) -> Result<Vec<Id>, String> {
        let origin = w.unit(id)?.position;
        let mut targets = self.caster_targets(w, id, 6.)?;
        targets.retain(|&t| {
            let p = w.unit(t).unwrap().position;
            let delta = [p[0] - origin[0], p[1] - origin[1]];
            let sign = delta.map(|x| ((x + 0.5).floor() as i32).signum());
            (delta[0].abs() < 0.5 || delta[1].abs() < 0.5) && direction.is_none_or(|d| d == sign)
        });
        targets.sort_by(|&a, &b| {
            let a = w.unit(a).unwrap().position;
            let b = w.unit(b).unwrap().position;
            hypot(a[1] - origin[1], a[0] - origin[0])
                .total_cmp(&hypot(b[1] - origin[1], b[0] - origin[0]))
        });
        Ok(targets)
    }
    fn cast_targets(&mut self, w: &mut World, id: Id, spell: Spell) -> Result<Vec<Id>, String> {
        let mut targets = match spell {
            Spell::Charge { .. } => {
                return Ok(w
                    .unit(id)?
                    .blocked_by()
                    .into_iter()
                    .filter(|&t| w.unit(t).is_ok_and(|u| u.stats.alive))
                    .collect())
            }
            Spell::Cross { .. } => return self.cross_targets(w, id, None),
            Spell::DeathEye { .. } => {
                let radius = w.unit(id)?.stats.base_number("rangeRadius");
                self.caster_targets(w, id, if radius == 0. { 2.5 } else { radius })?
            }
            Spell::Triple { radius, .. }
            | Spell::Jazz { radius, .. }
            | Spell::C4 { radius, .. }
            | Spell::Rock { radius, .. } => self.caster_targets(w, id, radius)?,
            Spell::Cannon { .. } => {
                self.caster_targets(w, id, w.unit(id)?.stats.base_number("rangeRadius") + 0.25)?
            }
            Spell::Drift { .. } | Spell::Sand { .. } => {
                self.caster_targets(w, id, f64::INFINITY)?
            }
            Spell::Chain { radius, .. } => {
                let mut targets = self.enemy_nearby(w, id, radius)?;
                targets.retain(|&t| t != id);
                return Ok(targets);
            }
            _ => return Ok(vec![]),
        };
        if matches!(spell, Spell::Rock { .. }) {
            let mut selected = vec![];
            for target in targets {
                if !w
                    .unit_mut(target)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("stun")
                {
                    selected.push(target);
                }
            }
            targets = selected;
        }
        if matches!(spell, Spell::Cannon { .. }) {
            let mut keyed = vec![];
            for target in targets {
                keyed.push((target, Self::tac_stat(w, target, "maxHp")?));
            }
            keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
            return Ok(keyed.into_iter().map(|x| x.0).collect());
        }
        targeting::sort_allies(
            w,
            id,
            &mut targets,
            targeting::Profile {
                block_free: matches!(spell, Spell::Drift { .. } | Spell::Sand { .. }),
                ..Default::default()
            },
        )?;
        if matches!(spell, Spell::Drift { .. } | Spell::Sand { .. }) {
            let mut keyed = vec![];
            for target in targets {
                keyed.push((target, Self::tac_stat(w, target, "maxHp")?));
            }
            keyed.sort_by(|a, b| {
                if matches!(spell, Spell::Drift { .. }) {
                    b.1.total_cmp(&a.1)
                } else {
                    a.1.total_cmp(&b.1)
                }
            });
            targets = keyed.into_iter().map(|x| x.0).collect();
        }
        Ok(targets)
    }
    fn caster_condition(&mut self, w: &mut World, id: Id, spell: Spell) -> Result<bool, String> {
        if let Some(ready) = self.boss_condition(w, id, spell)? {
            return Ok(ready);
        }
        if let Some(ready) = self.champion_condition(w, id, spell)? {
            return Ok(ready);
        }
        Ok(match spell {
            Spell::Shield { .. } | Spell::TombShield { .. } => true,
            Spell::Blink { .. } => w.unit(id)?.blocked_by().is_some(),
            Spell::Jazz { .. } => {
                !self.content.as_ref().unwrap().caster(id).channel
                    && !Queries::enemy_stealthed(self, id, w)?
                    && !self.cast_targets(w, id, spell)?.is_empty()
            }
            _ => !self.cast_targets(w, id, spell)?.is_empty(),
        })
    }
    fn caster_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        slot: usize,
        skill: Skill,
        grid: Option<&Grid>,
    ) -> Result<(), String> {
        let mut state = self.content.as_ref().unwrap().caster(id);
        state.left[slot] = (skill.clock.period + skill.clip.max(0.)).max(DT);
        self.content.as_mut().unwrap().set_caster(id, state);
        let targets = self.cast_targets(w, id, skill.spell)?;
        match skill.spell {
            spell @ (Spell::PipeStrike { .. }
            | Spell::Ensemble
            | Spell::Tide { .. }
            | Spell::Rockfall { .. }
            | Spell::Tentacles { .. }
            | Spell::Species { .. }
            | Spell::GunCharge
            | Spell::SpringDash { .. }
            | Spell::HelmShell
            | Spell::HelmDrones { .. }
            | Spell::BladeDive
            | Spell::Decree { .. }
            | Spell::Repel { .. }
            | Spell::Surge { .. }) => self.boss_fire(w, r, id, spell, grid)?,
            spell @ (Spell::XiShield { .. }
            | Spell::XiCross { .. }
            | Spell::MaceBlink { .. }
            | Spell::MaceCircle { .. }
            | Spell::Fear { .. }
            | Spell::LucienAoe { .. }
            | Spell::Phantom { .. }) => self.champion_fire(w, r, id, spell)?,
            Spell::Charge { duration, scale } => {
                if let Some(&target) = targets.first() {
                    let mut b = buff("ab:charging", &[]);
                    b.runtime.duration = duration;
                    b.runtime.visible = true;
                    b.flags.insert("disarm".into());
                    self.enemy_buff(w, r, id, b)?;
                    self.enemy_schedule(
                        w,
                        Some(id),
                        duration,
                        0.,
                        1,
                        EnemyTask::Spell(SpellTask::Knight {
                            target: None,
                            tile: w.unit(target)?.deployment.tile,
                            scale,
                            charge: true,
                        }),
                    )?;
                }
            }
            Spell::Triple { delay, scale, .. } => {
                for &target in targets.iter().take(3) {
                    self.enemy_schedule(
                        w,
                        Some(id),
                        delay,
                        0.,
                        1,
                        EnemyTask::Spell(SpellTask::Knight {
                            target: Some(target),
                            tile: w.unit(target)?.deployment.tile,
                            scale,
                            charge: false,
                        }),
                    )?;
                }
            }
            Spell::Cross { scale, stun } => {
                let Some(&target) = targets.first() else {
                    return Ok(());
                };
                let a = w.unit(id)?.position;
                let b = w.unit(target)?.position;
                let direction =
                    [b[0] - a[0], b[1] - a[1]].map(|x| ((x + 0.5).floor() as i32).signum());
                for (key, flags) in [
                    ("ab:revealed", vec!["reveal"]),
                    ("ab:aim", vec!["noMove", "disarm"]),
                ] {
                    let mut b = buff(key, &[]);
                    b.runtime.duration = 2.5;
                    b.flags.extend(flags.into_iter().map(str::to_string));
                    self.enemy_buff(w, r, id, b)?;
                }
                self.enemy_schedule(
                    w,
                    Some(id),
                    1.4,
                    0.,
                    1,
                    EnemyTask::Spell(SpellTask::Cross {
                        direction,
                        scale,
                        stun,
                    }),
                )?;
            }
            Spell::DeathEye {
                duration,
                scale,
                element,
            } => {
                if let Some(&target) = targets.first() {
                    let mut b = buff("ab:channel", &[]);
                    b.runtime.duration = duration;
                    b.flags.insert("disarm".into());
                    self.enemy_buff(w, r, id, b)?;
                    self.enemy_schedule(
                        w,
                        Some(id),
                        1.,
                        1.,
                        Self::spell_count(duration.ceil().max(1.))?,
                        EnemyTask::Spell(SpellTask::Channel {
                            target,
                            jazz: false,
                            scale,
                            element,
                        }),
                    )?;
                }
            }
            Spell::Jazz {
                duration,
                interval,
                scale,
                element,
                ..
            } => {
                let Some(&target) = targets.first() else {
                    return Ok(());
                };
                if duration <= 0. || interval <= 0. {
                    return Ok(());
                }
                let mut b = buff("ab:channel", &[]);
                b.runtime.duration = duration;
                b.flags.insert("disarm".into());
                self.enemy_buff(w, r, id, b)?;
                let mut state = self.content.as_ref().unwrap().caster(id);
                state.channel = true;
                self.content.as_mut().unwrap().set_caster(id, state);
                self.enemy_schedule(
                    w,
                    Some(id),
                    interval.max(DT),
                    interval.max(DT),
                    Self::spell_count((duration / interval + 1e-9).floor().max(1.))?,
                    EnemyTask::Spell(SpellTask::Channel {
                        target,
                        jazz: true,
                        scale,
                        element,
                    }),
                )?;
            }
            Spell::Cannon { scale } => {
                if let Some(&target) = targets.first() {
                    let tile = w.unit(target)?.position.map(|x| (x + 0.5).floor() as i32);
                    let mut b = buff("ab:cannon", &[]);
                    b.runtime.duration = 5.;
                    b.flags.insert("noDisplace".into());
                    self.enemy_buff(w, r, id, b)?;
                    let mut state = self.content.as_ref().unwrap().caster(id);
                    state.cannon = true;
                    self.content.as_mut().unwrap().set_caster(id, state);
                    self.enemy_schedule(
                        w,
                        Some(id),
                        0.5,
                        0.5,
                        10,
                        EnemyTask::Spell(SpellTask::Cannon { tile, scale }),
                    )?;
                }
            }
            Spell::C4 { scale, .. } => {
                let hp = w.unit(id)?.stats.hp;
                let count = if hp / Self::tac_stat(w, id, "maxHp")? < 0.5 {
                    2
                } else {
                    1
                };
                for &target in targets.iter().take(count) {
                    self.enemy_schedule(
                        w,
                        Some(id),
                        1.,
                        0.,
                        1,
                        EnemyTask::Spell(SpellTask::C4 {
                            target,
                            position: w.unit(target)?.position,
                            scale,
                        }),
                    )?;
                }
            }
            Spell::Drift { damage } => {
                if let Some(&target) = targets.first() {
                    let tile = w.unit(target)?.deployment.tile;
                    for t in self.caster_cross_area(w, id, tile)? {
                        let mut hit = ability_damage(Some(id), t, damage, DamageType::Physical);
                        self.enemy_damage(w, r, &mut hit)?;
                    }
                }
            }
            Spell::Sand {
                duration,
                damage,
                attack,
            } => {
                if let Some(&target) = targets.first() {
                    let ids = spatial::allies_in_radius(
                        w,
                        self.roster.as_ref().unwrap().ids(),
                        w.unit(target)?.position,
                        1.,
                        None,
                        false,
                        w.unit_capacity(),
                    )?;
                    for t in ids {
                        if !targeting::area_selectable(w, Some(id), t)? {
                            continue;
                        }
                        let mut b = buff("ab:sandStorm", &[("atkPct", attack)]);
                        b.runtime.duration = duration;
                        b.runtime.interval = 1.;
                        b.runtime.visible = true;
                        b.runtime.source = Some(id);
                        b.runtime.tick_payload = damage;
                        b.runtime.on_tick = Some(SAND_TICK);
                        r.add(
                            w,
                            self,
                            t,
                            buffs::Spec {
                                buff: b,
                                mods_present: true,
                                ..Default::default()
                            }
                            .retaining_unit(id),
                        )?;
                    }
                }
            }
            Spell::Rock { scale, stun, .. } => {
                if let Some(&target) = targets.first() {
                    self.caster_projectile(w, id, target, 8., DamageType::Physical, scale, stun)?;
                }
            }
            Spell::Chain {
                radius,
                jump,
                max,
                duration,
                speed,
                aspd,
            } => {
                let mut seen = vec![id];
                let mut position = w.unit(id)?.position;
                let mut reach = radius;
                while seen.len() - 1 < max as usize {
                    let ids = self.spawns.as_ref().unwrap().live().to_vec();
                    let mut near = spatial::enemies_in_radius(
                        w,
                        self,
                        &ids,
                        position,
                        reach,
                        RadiusRule::Body,
                        false,
                        w.unit_capacity(),
                    )?;
                    near.retain(|t| !seen.contains(t));
                    near.sort_by(|&a, &b| {
                        let a = w.unit(a).unwrap().position;
                        let b = w.unit(b).unwrap().position;
                        hypot(a[1] - position[1], a[0] - position[0])
                            .total_cmp(&hypot(b[1] - position[1], b[0] - position[0]))
                    });
                    let Some(&target) = near.first() else {
                        break;
                    };
                    seen.push(target);
                    let mut b = buff("ab:chainBuff", &[("moveMul", 1. + speed), ("aspd", aspd)]);
                    b.runtime.duration = duration;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, target, b)?;
                    position = w.unit(target)?.position;
                    reach = jump;
                }
            }
            Spell::Shield { amount, hp, aspd } => self.enemy_buff(
                w,
                r,
                id,
                Caster::barrier("ab:rockPower", amount, &[("hpPct", hp), ("aspd", aspd)]),
            )?,
            Spell::TombShield { ratio } => {
                let amount = Self::tac_stat(w, id, "maxHp")? * ratio;
                let mut b = buff("ab:tombShield", &[]);
                b.shield = amount;
                b.runtime.persist = true;
                self.enemy_buff(w, r, id, b)?;
            }
            Spell::Blink { distance } => {
                self.caster_blink(w, r, id, distance)?;
            }
        }
        Ok(())
    }
    pub(super) fn caster_turn(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        own: bool,
        grid: Option<&Grid>,
    ) -> Result<Option<f64>, String> {
        let Some(skills) = self.caster_config(id).map(|c| c.skills) else {
            return Ok(None);
        };
        let state = self.content.as_ref().unwrap().caster(id);
        let mut ready = vec![];
        let mut top = f64::NEG_INFINITY;
        for (slot, skill) in skills.into_iter().enumerate() {
            let Some(skill) = skill else {
                continue;
            };
            if !skill.turn
                || own && !skill.own
                || state.left[slot] > 1e-9
                || !self.enemy_can_cast(w, id, skill.silence)?
                || !self.caster_condition(w, id, skill.spell)?
            {
                continue;
            }
            if skill.priority > top {
                ready.clear();
                top = skill.priority;
            }
            if skill.priority == top {
                ready.push((slot, skill));
            }
        }
        if ready.is_empty() {
            return Ok(None);
        }
        let pick = if ready.len() > 1 {
            self.rng.int(ready.len() as u32) as usize
        } else {
            0
        };
        let (slot, skill) = ready[pick];
        self.caster_fire(w, r, id, slot, skill, grid)?;
        Ok(Some(skill.clip))
    }
    pub(super) fn enemy_caster_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &mut enemy::Context,
        event: enemy::Event,
        grid: Option<&Grid>,
    ) -> Result<bool, String> {
        if event == enemy::Event::BeforeAttack {
            if let Some(clip) = self.caster_turn(w, r, c.enemy, false, grid)? {
                Self::caster_restart_attack(w, c.enemy, clip)?;
                c.targets.clear();
                return Ok(true);
            }
        }
        let Some(config) = self.caster_config(c.enemy) else {
            return Ok(false);
        };
        let passive = config.passive;
        match passive {
            Passive::Knight { ranged: false, .. } if event == enemy::Event::BeforeAttack => {
                if let Some(&target) = c.targets.first() {
                    for t in self.caster_cross_area(w, c.enemy, w.unit(target)?.deployment.tile)? {
                        if !c.targets.contains(&t) {
                            c.targets.push(t);
                        }
                    }
                }
            }
            Passive::Mudrock { atk, .. } if event == enemy::Event::Attack => {
                let mut b = buff("ab:rockCharge", &[("atkPct", atk)]);
                b.runtime.persist = true;
                b.runtime.refresh = buffs::Refresh::Stack;
                b.runtime.max_stacks = 6;
                self.enemy_buff(w, r, c.enemy, b)?;
            }
            Passive::Tomb { .. } if event == enemy::Event::BeforeAttack => {
                let mut state = self.content.as_ref().unwrap().caster(c.enemy);
                state.splash = w.unit(c.enemy)?.blocked_by().is_none();
                self.content.as_mut().unwrap().set_caster(c.enemy, state);
            }
            _ => {}
        }
        Ok(false)
    }
    pub(super) fn enemy_caster_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Caster,
        dt: f64,
    ) -> Result<(), String> {
        self.champion_tick(w, r, id, config, dt)?;
        if let Passive::Pompeii {
            blast,
            blast_interval,
            ..
        } = config.passive
        {
            let blocked = w.unit(id)?.blocked_by().is_some();
            let mut state = self.content.as_ref().unwrap().caster(id);
            if !blocked {
                state.held = 0.;
            } else if !w.unit(id)?.hidden
                && !w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("stun")
            {
                state.held += dt;
                if state.held + 1e-9 >= blast_interval {
                    state.held = 0.;
                    for target in
                        enemy::Hooks::allies_in_radius(self, id, 1.4, w, w.unit_capacity())?
                    {
                        if !w.unit(target)?.motion_fly
                            && targeting::area_selectable(w, Some(id), target)?
                        {
                            let mut q = ability_damage(Some(id), target, blast, DamageType::Arts);
                            q.tags.push("splash".into());
                            self.enemy_damage(w, r, &mut q)?;
                        }
                    }
                }
            }
            self.content.as_mut().unwrap().set_caster(id, state);
        }
        if matches!(config.passive, Passive::Stealth { jazz: true }) {
            let hidden = Queries::enemy_stealthed(self, id, w)?;
            let mut state = self.content.as_ref().unwrap().caster(id);
            state.no_attack = hidden;
            self.content.as_mut().unwrap().set_caster(id, state);
        }
        if matches!(config.passive, Passive::Tomb { .. }) {
            let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.5);
            for _ in 0..pulses {
                for t in self.roster.as_ref().unwrap().ids().to_vec() {
                    let u = w.unit(t)?;
                    if !u.stats.alive || !u.deployed || u.hidden {
                        continue;
                    }
                    let mut b = buff("ab:tombstone", &[("atkMul", 0.5), ("defMul", 0.5)]);
                    b.runtime.duration = 0.6;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, t, b)?;
                    let mut b = buff("ab:tombRedeploy", &[("redeployMul", 2.)]);
                    b.runtime.duration = 0.6;
                    b.runtime.persist = true;
                    self.enemy_buff(w, r, t, b)?;
                }
            }
            self.roster.as_mut().unwrap().add_dp(-0.5 * dt)?;
        }
        for (slot, skill) in config.skills.into_iter().enumerate() {
            let Some(skill) = skill else {
                continue;
            };
            let mut state = self.content.as_ref().unwrap().caster(id);
            state.left[slot] -= dt;
            self.content.as_mut().unwrap().set_caster(id, state);
            if !skill.turn
                && state.left[slot] <= 1e-9
                && self.enemy_can_cast(w, id, skill.silence)?
                && self.caster_condition(w, id, skill.spell)?
            {
                self.caster_fire(w, r, id, slot, skill, None)?;
            }
        }
        let u = w.unit_mut(id)?;
        let blocked = u.blocked_by().is_some();
        let flags = &u.stats.stats().map_err(str::to_string)?.flags;
        let disabled = flags.contains("fear")
            || flags.contains("disarm")
            || flags.contains("tremble") && blocked;
        if !u.enemy.swing && u.cadence.cooldown <= 0. && !disabled {
            if let Some(clip) = self.caster_turn(w, r, id, true, None)? {
                Self::caster_restart_attack(w, id, clip)?;
            }
        }
        Ok(())
    }
    fn caster_restart_attack(w: &mut World, id: Id, clip: f64) -> Result<(), String> {
        w.unit_mut(id)?.cadence.cooldown = Self::tac_stat(w, id, "interval")?.max(clip);
        w.unit_mut(id)?.motion.attack_stand_until = f64::NEG_INFINITY;
        Ok(())
    }
    fn spell_count(count: f64) -> Result<u32, String> {
        if !count.is_finite() || count < 1. || count > i32::MAX as f64 {
            return Err("Enemy skill pulse count outside positive i32 domain".into());
        }
        Ok(count as u32)
    }
    pub(super) fn caster_cross_area(
        &mut self,
        w: &mut World,
        id: Id,
        tile: [i32; 2],
    ) -> Result<Vec<Id>, String> {
        let mut ids = vec![];
        for t in self.enemy_tile_allies(w, tile, 1)? {
            let p = w.unit(t)?.deployment.tile;
            if (p[0] - tile[0]).abs() + (p[1] - tile[1]).abs() <= 1
                && targeting::area_selectable(w, Some(id), t)?
            {
                ids.push(t);
            }
        }
        Ok(ids)
    }
    pub(super) fn caster_blink(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        distance: f64,
    ) -> Result<Option<[f64; 2]>, String> {
        let origin = w.unit(id)?.position;
        let mut at = origin;
        let route = w
            .unit(id)?
            .motion
            .route
            .as_ref()
            .ok_or("Missing blink route")?;
        if let Some(points) = route.points.as_ref().filter(|p| route.point < p.len()) {
            let mut left = distance;
            for p in points.iter().skip(route.point) {
                if left <= 1e-9 {
                    break;
                }
                let d = hypot(p[1] - at[1], p[0] - at[0]);
                if d <= left {
                    at = *p;
                    left -= d;
                } else {
                    at[0] += (p[0] - at[0]) / d * left;
                    at[1] += (p[1] - at[1]) / d * left;
                    break;
                }
            }
        } else if let Some(Leg::Move { tile, .. } | Leg::Appear(tile)) = route.legs().get(route.leg)
        {
            let p = tile.map(|x| x as f64);
            let d = hypot(p[1] - at[1], p[0] - at[0]);
            if d <= distance + 1e-9 {
                at = p;
            } else if d > 0. {
                at[0] += (p[0] - at[0]) / d * distance;
                at[1] += (p[1] - at[1]) / d * distance;
            }
        }
        let tile = at.map(|x| (x + 0.5).floor() as i32);
        if !w.unit(id)?.motion_fly
            && !self
                .grid
                .as_ref()
                .ok_or("Missing blink grid")?
                .ground_passable(tile, false)
        {
            return Ok(None);
        }
        w.unit_mut(id)?.position = at;
        let mut b = buff("ab:blink", &[]);
        b.runtime.duration = 1.;
        b.flags
            .extend(["invulnerable", "unblockable"].map(str::to_string));
        self.enemy_buff(w, r, id, b)?;
        if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
            route.points = None;
        }
        Ok(Some(origin))
    }
}
