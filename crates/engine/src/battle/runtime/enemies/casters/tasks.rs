// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{element::Element, injury, projectiles};
use crate::deterministic::math::powi;
pub(in crate::battle::runtime) const ARTS_BARRIER: u64 = 1703;
pub(in crate::battle::runtime) const SAND_TICK: u64 = 1704;
pub(in crate::battle::runtime) const BURN_TICK: u64 = 1705;
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) struct SkillShot {
    pub source: Id,
    pub kind: DamageType,
    pub scale: f64,
    pub stun: f64,
}
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum SpellTask {
    Knight {
        target: Option<Id>,
        tile: [i32; 2],
        scale: f64,
        charge: bool,
    },
    Cross {
        direction: [i32; 2],
        scale: f64,
        stun: f64,
    },
    Channel {
        target: Id,
        jazz: bool,
        scale: f64,
        element: f64,
    },
    Cannon {
        tile: [i32; 2],
        scale: f64,
    },
    C4 {
        target: Id,
        position: [f64; 2],
        scale: f64,
    },
}
impl SpellTask {
    pub(in crate::battle::runtime) fn target(self) -> Option<Id> {
        match self {
            Self::Knight { target, .. } => target,
            Self::Channel { target, .. } | Self::C4 { target, .. } => Some(target),
            _ => None,
        }
    }
}
impl Host {
    pub(super) fn caster_projectile(
        &mut self,
        w: &mut World,
        source: Id,
        target: Id,
        speed: f64,
        kind: DamageType,
        scale: f64,
        stun: f64,
    ) -> Result<(), String> {
        let from = w.unit(source)?.position;
        self.flights
            .as_mut()
            .ok_or("Missing enemy skill projectiles")?
            .add(
                w,
                projectiles::Launch {
                    from,
                    source: Some(source),
                    target: Some(target),
                    to: None,
                    speed,
                    hit_dead: false,
                    max_age: 10.,
                    on_hit: true,
                    visual: if speed == 8. { "lob" } else { "enemy" }.into(),
                    speed_changeable: None,
                    removable: None,
                    payload: allies::Shot::Skill(SkillShot {
                        source,
                        kind,
                        scale,
                        stun,
                    }),
                },
            )?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_skill_impact(
        &mut self,
        w: &mut World,
        shot: SkillShot,
        target: Option<Id>,
    ) -> Result<(), String> {
        let mut r = self
            .buffs
            .take()
            .ok_or("Missing enemy skill impact buffs")?;
        let result = (|| {
            if let Some(target) = target {
                let amount = Self::tac_stat(w, shot.source, "atk")? * shot.scale;
                let mut hit = ability_damage(Some(shot.source), target, amount, shot.kind);
                self.enemy_damage(w, &mut r, &mut hit)?;
                if shot.stun > 0. && w.unit(target)?.stats.alive {
                    self.apply_status(
                        w,
                        &mut r,
                        target,
                        "stun",
                        status::Options {
                            source: Some(shot.source),
                            duration: shot.stun,
                            ..Default::default()
                        },
                    )?;
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime::enemies) fn enemy_spell_task(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Option<Id>,
        task: SpellTask,
        last: bool,
    ) -> Result<bool, String> {
        let id = source.ok_or("Missing enemy spell source")?;
        if !matches!(task, SpellTask::C4 { .. }) && !w.unit(id)?.stats.alive {
            return Ok(false);
        }
        match task {
            SpellTask::Knight {
                target,
                tile,
                scale,
                charge,
            } => {
                if charge
                    && w.unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("stun")
                {
                    return Ok(false);
                }
                let mut targets = vec![];
                if let Some(target) = target {
                    if w.unit(target)?.stats.alive && w.unit(target)?.deployed {
                        targets.push(target);
                    }
                }
                for t in self.caster_cross_area(w, id, tile)? {
                    if !targets.contains(&t) {
                        targets.push(t);
                    }
                }
                for t in targets {
                    let amount = Self::tac_stat(w, id, "atk")? * scale;
                    let mut q = ability_damage(
                        Some(id),
                        t,
                        amount,
                        if charge {
                            DamageType::Physical
                        } else {
                            DamageType::Arts
                        },
                    );
                    self.enemy_damage(w, r, &mut q)?;
                }
            }
            SpellTask::Cross {
                direction,
                scale,
                stun,
            } => {
                let u = w.unit_mut(id)?;
                let flags = &u.stats.stats().map_err(str::to_string)?.flags;
                if u.hidden || flags.contains("stun") || flags.contains("silence") {
                    return Ok(false);
                }
                if let Some(&target) = self.cross_targets(w, id, Some(direction))?.first() {
                    self.caster_projectile(w, id, target, 12., DamageType::Arts, scale, stun)?;
                }
            }
            SpellTask::Channel {
                target,
                jazz,
                scale,
                element,
            } => {
                let flags = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .clone();
                if flags.contains("silence")
                    || (!jazz && flags.contains("stun"))
                    || !w.unit(target)?.stats.alive
                    || (jazz && !w.unit(target)?.deployed)
                {
                    self.caster_end_channel(w, r, id, jazz)?;
                    return Ok(false);
                }
                let amount = Self::tac_stat(w, id, "atk")? * scale;
                let mut hit = ability_damage(Some(id), target, amount, DamageType::Arts);
                self.enemy_damage(w, r, &mut hit)?;
                if jazz {
                    let amount = Self::tac_stat(w, id, "atk")? * element;
                    self.enemy_element(w, r, id, target, Element::Burn, amount)?;
                } else if last {
                    let mut targets = vec![];
                    if w.unit(target)?.stats.alive && w.unit(target)?.deployed {
                        targets.push(target);
                    }
                    for t in spatial::allies_in_radius(
                        w,
                        self.roster.as_ref().unwrap().ids(),
                        w.unit(target)?.position,
                        1.,
                        None,
                        false,
                        w.unit_capacity(),
                    )? {
                        if t != target && targeting::area_selectable(w, Some(id), t)? {
                            targets.push(t);
                        }
                    }
                    for t in targets {
                        let amount = Self::tac_stat(w, id, "atk")? * element;
                        self.enemy_element(w, r, id, t, Element::Apoptosis, amount)?;
                    }
                }
                if last && jazz {
                    self.caster_end_channel(w, r, id, true)?;
                }
            }
            SpellTask::Cannon { tile, scale } => {
                if last {
                    let mut state = self.content.as_ref().unwrap().caster(id);
                    state.cannon = false;
                    self.content.as_mut().unwrap().set_caster(id, state);
                }
                let mut keyed = vec![];
                for target in self.enemy_tile_allies(w, tile, 1)? {
                    if !targeting::area_selectable(w, Some(id), target)? {
                        continue;
                    }
                    let hp = w.unit(target)?.stats.hp;
                    keyed.push((target, hp / Self::tac_stat(w, target, "maxHp")?));
                }
                keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
                if let Some(&(target, _)) = keyed.first() {
                    let amount = Self::tac_stat(w, id, "atk")? * scale;
                    let mut hit = ability_damage(Some(id), target, amount, DamageType::Arts);
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
            SpellTask::C4 {
                target,
                position,
                scale,
            } => {
                let mut targets = vec![];
                if w.unit(target)?.stats.alive && w.unit(target)?.deployed {
                    targets.push(target);
                }
                for t in spatial::allies_in_radius(
                    w,
                    self.roster.as_ref().unwrap().ids(),
                    position,
                    1.,
                    None,
                    false,
                    w.unit_capacity(),
                )? {
                    if t != target && targeting::area_selectable(w, Some(id), t)? {
                        targets.push(t);
                    }
                }
                for t in targets {
                    let amount = Self::tac_stat(w, id, "atk")? * scale;
                    let mut hit = ability_damage(Some(id), t, amount, DamageType::Physical);
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
        }
        Ok(true)
    }
    fn caster_end_channel(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        jazz: bool,
    ) -> Result<(), String> {
        if jazz {
            let mut state = self.content.as_ref().unwrap().caster(id);
            state.channel = false;
            self.content.as_mut().unwrap().set_caster(id, state);
        }
        r.remove(w, self, id, "ab:channel")?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_sand_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let amount = b.runtime.tick_payload;
        let source = b.runtime.source;
        let mut hit = ability_damage(source, c.unit, amount, DamageType::Arts);
        hit.is_skill = false;
        hit.ignore_select = true;
        hit.tags.push("sandStorm".into());
        self.enemy_damage(w, r, &mut hit)
    }
    pub(in crate::battle::runtime) fn enemy_burn_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let mut q = ability_damage(
            b.runtime.source,
            c.unit,
            b.runtime.tick_payload,
            DamageType::Arts,
        );
        q.is_skill = false;
        q.ignore_select = true;
        self.enemy_damage(w, r, &mut q)
    }
    pub(in crate::battle::runtime) fn enemy_barrier_injury(
        event: &mut injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if c.request.hit.damage_type != DamageType::Arts {
            return Ok(());
        }
        let target = c.request.hit.target;
        let b = c.runtime.get(c.world, target, c.buff)?;
        let left = b.runtime.tick_payload;
        if left <= 0. {
            return Ok(());
        }
        let take = left.min(event.amount);
        let rest = (left - take).max(0.);
        event.amount -= take;
        c.runtime
            .edit(c.world, target, c.buff, |b| b.runtime.tick_payload = rest)?;
        if rest <= 1e-6 {
            event.defer(c.buff)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_caster_hit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        self.champion_hit(w, id, q)?;
        let tomb = self.caster_config(id).and_then(|c| match c.passive {
            Passive::Tomb { scale } => Some(scale),
            _ => None,
        });
        if let Some(scale) = tomb {
            if self.content.as_ref().unwrap().caster(id).splash {
                let amount = Self::tac_stat(w, id, "atk")? * scale;
                for t in self.enemy_tile_allies(w, w.unit(q.hit.target)?.deployment.tile, 1)? {
                    if t == q.hit.target || !targeting::area_selectable(w, Some(id), t)? {
                        continue;
                    }
                    let mut hit = ability_damage(Some(id), t, amount, DamageType::Physical);
                    hit.tags.push("splash".into());
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_caster_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        self.champion_dealt(w, r, id, q)?;
        let pompeii = self.caster_config(id).and_then(|c| match &c.passive {
            Passive::Pompeii {
                dot,
                interval,
                duration,
                ..
            } => Some((*dot, *interval, *duration)),
            _ => None,
        });
        if let Some((dot, interval, duration)) = pompeii {
            if duration > 0. {
                let mut b = buff("ab:burnDot", &[]);
                b.runtime.duration = duration;
                b.runtime.interval = interval;
                b.runtime.source = Some(id);
                b.runtime.tick_payload = dot;
                b.runtime.visible = true;
                b.runtime.on_tick = Some(BURN_TICK);
                r.add(
                    w,
                    self,
                    q.hit.target,
                    buffs::Spec {
                        buff: b,
                        ..Default::default()
                    }
                    .retaining_unit(id),
                )?;
            }
        }
        let Some((cold, max, fall, jump)) = self.caster_config(id).and_then(|c| match &c.passive {
            Passive::Snow {
                cold,
                max,
                fall,
                jump,
            } => Some((*cold, *max, *fall, *jump)),
            _ => None,
        }) else {
            return Ok(());
        };
        self.caster_cold(w, r, id, q.hit.target, cold)?;
        let mut previous = q.hit.target;
        let mut seen = vec![previous];
        for k in 1..max {
            let mut next = None;
            for t in spatial::allies_in_radius(
                w,
                self.roster.as_ref().unwrap().ids(),
                w.unit(previous)?.position,
                jump,
                None,
                false,
                w.unit_capacity(),
            )? {
                if !seen.contains(&t) && targeting::area_selectable(w, Some(id), t)? {
                    next = Some(t);
                    break;
                }
            }
            let Some(target) = next else {
                break;
            };
            seen.push(target);
            let amount =
                Self::tac_stat(w, id, "atk")? * powi(fall, k as i32).map_err(str::to_string)?;
            let mut hit = ability_damage(Some(id), target, amount, DamageType::Arts);
            self.enemy_damage(w, r, &mut hit)?;
            self.caster_cold(w, r, id, target, cold)?;
            previous = target;
        }
        Ok(())
    }
    fn caster_cold(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        duration: f64,
    ) -> Result<(), String> {
        if duration > 0. && w.unit(target)?.stats.alive {
            self.apply_status(
                w,
                r,
                target,
                "cold",
                status::Options {
                    duration,
                    source: Some(id),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn enemy_caster_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        self.champion_taken(w, r, id)?;
        let (ratio, mods) = match self.caster_config(id).map(|c| &c.passive) {
            Some(Passive::Mouse { ratio, scale, .. }) => (*ratio, vec![("dmgDealtMul", *scale)]),
            Some(Passive::Pompeii { ratio, aspd, .. }) => (*ratio, vec![("aspd", *aspd)]),
            _ => return Ok(()),
        };
        let mut state = self.content.as_ref().unwrap().caster(id);
        let hp = w.unit(id)?.stats.hp;
        if !state.enraged && hp / Self::tac_stat(w, id, "maxHp")? < ratio {
            state.enraged = true;
            self.content.as_mut().unwrap().set_caster(id, state);
            let mut b = buff("ab:lowhp", &mods);
            b.runtime.persist = true;
            b.runtime.visible = true;
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_caster_profile(
        &self,
        id: Id,
        kit: &Kit,
        profile: &mut enemy::Profile,
    ) {
        let state = self.content.as_ref().unwrap().caster(id);
        profile.no_attack |=
            state.phase == Some(crate::battle::content::enemies::lifecycle::RebornPhase::Rebirth);
        match self.caster_config_from(id, kit).map(|c| &c.passive) {
            Some(Passive::Wolf { .. })
                if state.phase
                    == Some(crate::battle::content::enemies::lifecycle::RebornPhase::Revived) =>
            {
                profile.melee = false
            }
            Some(Passive::Pompeii { .. }) => profile.max_targets = 4.,
            Some(Passive::Knight { ranged: true, .. }) => profile.max_targets = 2.,
            Some(Passive::Stealth { jazz: true }) => {
                profile.melee = true;
                profile.no_attack = state.no_attack;
            }
            Some(Passive::Tomb { scale }) => {
                profile.atk_scale = if state.splash { *scale } else { 1. }
            }
            _ => {}
        }
    }
    pub(in crate::battle::runtime) fn enemy_caster_before_status(&self, c: &mut status::Context) {
        if !matches!(
            c.key.as_str(),
            "stun" | "freeze" | "cold" | "levitate" | "sleep"
        ) {
            return;
        }
        if self
            .content
            .as_ref()
            .is_some_and(|r| r.caster(c.target).cannon)
        {
            c.cancelled = true;
        }
    }
}
