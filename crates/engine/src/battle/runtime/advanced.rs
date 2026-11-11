// SPDX-License-Identifier: GPL-3.0-or-later
//! Lifecycle callbacks for the second-half operator batch.
use super::super::{
    attacks, body,
    content::operators::*,
    damage::{Damage, DamageType},
    direction::Direction,
    displacement,
    health::HealOptions,
    hit, injury,
    operators::Definition,
    search, skills,
    stats::{Buff, Numbers},
    targeting,
};
use super::*;
use crate::deterministic::math::hypot;
mod combat;

pub(super) const FLAME_EVADE: u64 = 1301;
#[derive(Clone, Copy)]
pub(super) enum Periodic {
    Frame,
    Fast,
    Aura(f64),
    Layer(f64),
}
impl Host {
    pub(super) fn advanced_stat(w: &mut World, id: Id, stat: &str) -> Result<f64, String> {
        Ok(w.unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n(stat))
    }
    fn advanced_flag(w: &mut World, id: Id, flag: &str) -> Result<bool, String> {
        Ok(w.unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains(flag))
    }
    pub(super) fn advanced_has(w: &World, id: Id, key: &str) -> bool {
        w.unit(id).is_ok_and(|u| {
            u.stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(key))
        })
    }
    pub(super) fn advanced_enemies(&self, w: &World, id: Id) -> Result<Vec<Id>, String> {
        let keys = self
            .roster
            .as_ref()
            .ok_or("Missing range context")?
            .range(w, id)?;
        Ok(self
            .spawns
            .as_ref()
            .ok_or("Missing enemy context")?
            .live()
            .iter()
            .copied()
            .filter(|&e| {
                w.unit(e)
                    .is_ok_and(|u| u.stats.alive && !u.hidden && body::in_range(u, &keys))
            })
            .collect())
    }
    fn advanced_allies(&self, w: &mut World, id: Id) -> Result<Vec<Id>, String> {
        let keys = self
            .roster
            .as_ref()
            .ok_or("Missing allied range context")?
            .range(w, id)?;
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut out = vec![];
        for target in ids {
            if targeting::friendly_selectable(w, id, target)?
                && body::in_range(w.unit(target)?, &keys)
            {
                out.push(target);
            }
        }
        Ok(out)
    }
    pub(super) fn advanced_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        key: &str,
        duration: f64,
        value: Option<f64>,
    ) -> Result<(), String> {
        self.apply_status(
            w,
            r,
            target,
            key,
            super::super::status::Options {
                duration,
                source: Some(source),
                value,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    pub(super) fn advanced_toggle(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
        on: bool,
        mods: Numbers,
    ) -> Result<(), String> {
        if on {
            if !w
                .unit(id)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(key) && b.mods == mods)
            {
                self.component_buff(w, r, id, key, mods, f64::INFINITY, id)?;
            }
        } else {
            r.remove(w, self, id, key)?;
        }
        Ok(())
    }
    fn advanced_hit_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        amount: f64,
        kind: DamageType,
        skill: bool,
        attack: bool,
        tag: &str,
    ) -> Result<f64, String> {
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(source),
                credit: Some(source),
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
        q.is_skill = skill;
        q.is_attack = attack;
        q.tags.push(tag.into());
        self.resolve_hit(w, r, &mut q)
    }
    pub(super) fn advanced_grant_sp(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        amount: f64,
    ) -> Result<(), String> {
        if amount <= 0. {
            return Ok(());
        }
        if self
            .roster
            .as_ref()
            .ok_or("Missing SP recipient")?
            .entry(id)?
            .skill()
            .is_err()
        {
            let limit = w
                .unit_capacity()
                .checked_mul(self.roster.as_ref().unwrap().ids().len())
                .ok_or("SP capacity overflow")?;
            let pending = &mut w.unit_mut(id)?.operator.support.sp_changes;
            if pending.len() >= limit {
                return Err("Pending SP capacity exceeded".into());
            }
            pending.push(crate::battle::content::operators::SpChange::Gain(amount));
            return Ok(());
        }
        let mut roster = self.roster.take().unwrap();
        let result = roster.grant_sp(w, r, self, id, amount);
        self.roster = Some(roster);
        result.map(|_| ())
    }
    fn advanced_abyssal(&self, id: Id) -> Result<bool, String> {
        Ok(matches!(
            self.roster
                .as_ref()
                .unwrap()
                .entry(id)?
                .definition
                .char_id
                .as_str(),
            "char_263_skadi"
                | "char_143_ghost"
                | "char_1023_ghost2"
                | "char_474_glady"
                | "char_4145_ulpia"
        ))
    }
    fn advanced_pull(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        p: &Glady,
        point: Option<[f64; 2]>,
    ) -> Result<(), String> {
        if !w.unit(target)?.stats.alive {
            return Ok(());
        }
        let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
        let origin = w.unit(id)?.position;
        let at = w.unit(target)?.position;
        let force = p.force
            + if point.is_none()
                && p.far_radius > 0.
                && hypot(at[0] - origin[0], at[1] - origin[1]) > p.far_radius + 1e-9
            {
                p.far_force
            } else {
                0.
            };
        let grid = self.grid.take().ok_or("Missing pull grid")?;
        let mut host = allies::Blocking {
            host: self,
            buffs: r,
        };
        let result = if let Some(to) = point {
            displacement::pull(
                w,
                &grid,
                &mut host,
                target,
                force,
                displacement::Pull {
                    to,
                    center: None,
                    stop: 0.05,
                },
            )
        } else {
            displacement::pull_to_front(w, &grid, &mut host, target, id, dir, force)
        };
        self.grid = Some(grid);
        let moved = result?;
        if moved > 0. && p.drag > 0. && w.unit(target)?.stats.alive {
            self.advanced_hit_damage(
                w,
                r,
                id,
                target,
                p.drag * moved / p.drag_distance,
                DamageType::Arts,
                true,
                false,
                "drag",
            )?;
        }
        Ok(())
    }
    fn advanced_layers(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        n: u32,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.advanced.layers = n;
        if n == 0 {
            r.remove(w, self, id, "mudrok:layers")?;
            return Ok(());
        }
        let mut b = Buff {
            key: Some("mudrok:layers".into()),
            ..Default::default()
        };
        b.runtime.shield_hits = n;
        r.add(
            w,
            self,
            id,
            buffs::Spec {
                buff: b,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    fn advanced_weightless(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        left: f64,
    ) -> Result<(), String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        for target in ids {
            if w.unit(target)?.stats.alive && !Self::advanced_has(w, target, "aglina:weightless") {
                let mut b = Buff {
                    key: Some("aglina:weightless".into()),
                    mods: [("massFlat".into(), -1.)].into(),
                    ..Default::default()
                };
                b.runtime.duration = left.max(0.1);
                b.runtime.source = Some(id);
                b.runtime.status = Some("weightless".into());
                r.add(
                    w,
                    self,
                    target,
                    buffs::Spec {
                        buff: b,
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn advanced_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        dir: Direction,
        def: &Definition,
        event: skills::Event,
        c: &skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        let id = s.unit();
        w.unit_mut(id)?.operator.advanced.skill_left = s.time_left;
        if event == skills::Event::Start {
            let m = &mut w.unit_mut(id)?.operator.advanced;
            m.elapsed = 0.;
            m.accumulator = 0.;
            m.aura_accumulator = 0.;
            m.dp_given = 0;
            m.locked.clear();
            m.charged =
                s.bank.max_charges() > 1 && s.bank.charges + 1. >= f64::from(s.bank.max_charges());
            if def.extra.advanced_start_heal > 0. {
                let amount = Self::advanced_stat(w, id, "maxHp")? * def.extra.advanced_start_heal;
                self.support_heal(
                    w,
                    r,
                    Some(id),
                    id,
                    amount,
                    HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
            match &def.extra.advanced {
                Advanced::Mostma(_) => {}
                Advanced::Billro(p) => match p.mode {
                    Mode::First => {
                        if w.unit(id)?.operator.advanced.charged {
                            self.component_buff(
                                w,
                                r,
                                id,
                                "billro:s1guard",
                                p.guard.clone(),
                                f64::INFINITY,
                                id,
                            )?;
                        }
                    }
                    Mode::Second => {
                        if w.unit(id)?.operator.advanced.charged {
                            self.component_buff(
                                w,
                                r,
                                id,
                                "billro:charged",
                                [("atkPct".into(), p.charged_atk)].into(),
                                f64::INFINITY,
                                id,
                            )?;
                        }
                    }
                    Mode::Third => {
                        self.component_buff(
                            w,
                            r,
                            id,
                            "billro:s3atk",
                            [("atkPct".into(), 0.)].into(),
                            f64::INFINITY,
                            id,
                        )?;
                    }
                },
                Advanced::Mudrok(p) if p.mode == Mode::Third => {
                    let mut b = Buff {
                        key: Some("mudrok:dormant".into()),
                        flags: ["invulnerable".into(), "disarm".into(), "noBlock".into()].into(),
                        ..Default::default()
                    };
                    b.runtime.duration = p.sleep + 1.;
                    r.add(
                        w,
                        self,
                        id,
                        buffs::Spec {
                            buff: b,
                            flags_present: true,
                            ..Default::default()
                        },
                    )?;
                    blocking::release(
                        w,
                        &mut allies::Blocking {
                            host: self,
                            buffs: r,
                        },
                        id,
                    )?;
                }
                Advanced::Flamtl(p) => {
                    if p.mode == Mode::First {
                        self.roster.as_mut().unwrap().add_dp(p.cost)?;
                        let mut b = Buff {
                            key: Some("flamtl:evade".into()),
                            ..Default::default()
                        };
                        b.runtime.injury.on_injury = Some(FLAME_EVADE);
                        r.add(
                            w,
                            self,
                            id,
                            buffs::Spec {
                                buff: b,
                                ..Default::default()
                            },
                        )?;
                    }
                    if p.mode == Mode::Second {
                        let keys =
                            targeting::absolute_range(&p.grid, w.unit(id)?.deployment.tile, dir, 0);
                        let ids = self.roster.as_ref().unwrap().ids().to_vec();
                        for a in ids {
                            if targeting::friendly_selectable(w, id, a)?
                                && body::in_range(w.unit(a)?, &keys)
                            {
                                self.component_buff(
                                    w,
                                    r,
                                    a,
                                    "flamtl:redPine",
                                    [("dodgePhys".into(), p.ally_dodge)].into(),
                                    p.ally_duration,
                                    id,
                                )?;
                            }
                        }
                    }
                }
                Advanced::Lisa(p) if p.fox => {
                    w.unit_mut(id)?.operator.advanced.aura_accumulator = 0.25;
                }
                Advanced::Surtr(p) if p.mode == Mode::Third => {
                    let amount = Self::advanced_stat(w, id, "maxHp")?;
                    self.support_heal(
                        w,
                        r,
                        Some(id),
                        id,
                        amount,
                        HealOptions {
                            self_heal: true,
                            ignore_heal_free: true,
                            ..Default::default()
                        },
                    )?;
                }
                Advanced::Aglina(p) if p.weightless => {
                    self.advanced_weightless(w, r, id, s.time_left)?
                }
                Advanced::Glady(p) if p.mode == Mode::Third => {
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    let mut foes = search::Host::enemies(
                        self,
                        id,
                        &keys,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                        w,
                        w.unit_capacity(),
                    )?;
                    let origin = w.unit(id)?.position;
                    foes.sort_by(|a, b| {
                        body::distance(w.unit(*b).unwrap(), origin)
                            .total_cmp(&body::distance(w.unit(*a).unwrap(), origin))
                            .then(a.cmp(b))
                    });
                    w.unit_mut(id)?.operator.advanced.tornado =
                        foes.first().map(|&t| w.unit(t).unwrap().position);
                    if let Some(&t) = foes.first() {
                        self.advanced_status(w, r, id, t, "bind", s.spec.duration, None)?;
                    }
                }
                Advanced::Gnosis(p) if p.mode == Mode::Second => {
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    let mut foes = search::Host::enemies(
                        self,
                        id,
                        &keys,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                        w,
                        w.unit_capacity(),
                    )?;
                    targeting::sort_enemies(w, self, id, &mut foes, def.combat.profile.priority)?;
                    for target in foes {
                        let amount = Self::advanced_stat(w, id, "atk")? * p.burst_scale;
                        self.advanced_hit_damage(
                            w,
                            r,
                            id,
                            target,
                            amount,
                            DamageType::Arts,
                            true,
                            false,
                            "burst",
                        )?;
                        if w.unit(target)?.stats.alive {
                            self.advanced_status(w, r, id, target, "cold", p.burst_cold, None)?;
                            if w.unit(id)?.operator.advanced.charged {
                                self.advanced_status(w, r, id, target, "cold", p.burst_cold, None)?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if event == skills::Event::SkillStart {
            if let Advanced::Billro(p) = &def.extra.advanced {
                r.remove(w, self, id, "trait:phalanxGuard")?;
                let ratio = if w.unit(id)?.operator.advanced.charged {
                    p.charged_heal
                } else {
                    p.heal
                };
                let amount = Self::advanced_stat(w, id, "maxHp")? * ratio;
                self.support_heal(
                    w,
                    r,
                    Some(id),
                    id,
                    amount,
                    HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
                if !(p.mode == Mode::First && w.unit(id)?.operator.advanced.charged)
                    && p.keep.values().any(|v| *v != 0.)
                {
                    self.component_buff(
                        w,
                        r,
                        id,
                        "billro:keep",
                        p.keep.clone(),
                        f64::INFINITY,
                        id,
                    )?;
                }
            }
            if self.roster.is_some() {
                self.advanced_learn(w, r, id, None)?;
            } else {
                if self.pending_skill_starts.len() >= w.unit_capacity() {
                    return Err("Skill notification capacity exceeded".into());
                }
                self.pending_skill_starts.push((
                    id,
                    w.unit_ids()
                        .filter(|&a| {
                            w.unit(a).is_ok_and(|u| {
                                u.side == health::Side::Ally && u.stats.alive && u.deployed
                            })
                        })
                        .collect(),
                ));
            }
        }
        if event == skills::Event::Tick {
            self.advanced_skill_tick(w, r, id, dir, def, s, c.dt)?;
        }
        if event == skills::Event::End {
            match &def.extra.advanced {
                Advanced::Billro(_) => {
                    for key in ["billro:s1guard", "billro:charged", "billro:s3atk"] {
                        r.remove(w, self, id, key)?;
                    }
                    self.advanced_clear_marks(w, r, id, "billro:mark")?;
                    w.unit_mut(id)?.operator.advanced.charged = false;
                }
                Advanced::Mudrok(_) => {
                    r.remove(w, self, id, "mudrok:dormant")?;
                    r.remove(w, self, id, "mudrok:awake")?;
                }
                Advanced::Flamtl(p) if p.mode == Mode::Third && c.reason == "duration" => {
                    let left = p
                        .total
                        .saturating_sub(w.unit(id)?.operator.advanced.dp_given);
                    self.roster
                        .as_mut()
                        .unwrap()
                        .add_dp(f64::from(left) * p.cost)?;
                }
                Advanced::Fartth(_) => w.unit_mut(id)?.operator.advanced.extra_range.clear(),
                Advanced::Lisa(_) => {
                    let ids: Vec<_> = w.unit_ids().collect();
                    for a in ids {
                        r.remove(w, self, a, &format!("lisa:fox:{id}"))?;
                    }
                }
                Advanced::Aglina(p) if p.weightless => {
                    self.advanced_clear_marks(w, r, id, "aglina:weightless")?
                }
                Advanced::Glady(p) if p.mode == Mode::Third => {
                    let pos = w.unit_mut(id)?.operator.advanced.tornado.take();
                    if c.reason != "death" && w.unit(id)?.stats.alive {
                        if let Some(pos) = pos {
                            let foes = self.advanced_radius(w, pos, 1.)?;
                            for target in foes {
                                self.advanced_pull(w, r, id, target, p, None)?;
                            }
                        }
                    }
                }
                Advanced::Gnosis(p) if p.mode == Mode::Third => {
                    if c.reason != "death" && w.unit(id)?.stats.alive {
                        let mut list = self.advanced_enemies(w, id)?;
                        let held = w.unit(id)?.operator.advanced.locked.clone();
                        for a in held {
                            if w.unit(a).is_ok_and(|u| u.stats.alive) && !list.contains(&a) {
                                list.push(a);
                            }
                        }
                        for target in list {
                            if Self::advanced_has(w, target, "freeze") {
                                let amount = Self::advanced_stat(w, id, "atk")? * p.burst_scale;
                                self.advanced_hit_damage(
                                    w,
                                    r,
                                    id,
                                    target,
                                    amount,
                                    DamageType::Arts,
                                    true,
                                    false,
                                    "shatter",
                                )?;
                                r.remove(w, self, target, "freeze")?;
                            }
                        }
                    }
                    w.unit_mut(id)?.operator.advanced.locked.clear();
                }
                _ => {}
            }
        }
        if event == skills::Event::SkillEnd {
            if let Advanced::Billro(p) = &def.extra.advanced {
                if w.unit(id)?.stats.alive {
                    self.component_buff(
                        w,
                        r,
                        id,
                        "trait:phalanxGuard",
                        p.guard.clone(),
                        f64::INFINITY,
                        id,
                    )?;
                }
                r.remove(w, self, id, "billro:keep")?;
            }
        }
        Ok(())
    }
    fn advanced_radius(
        &mut self,
        w: &mut World,
        pos: [f64; 2],
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        spatial::enemies_in_radius(
            w,
            self,
            &ids,
            pos,
            radius,
            spatial::RadiusRule::Body,
            true,
            w.unit_capacity(),
        )
    }
    fn advanced_clear_marks(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
    ) -> Result<(), String> {
        let ids: Vec<_> = w.unit_ids().collect();
        for a in ids {
            if w.unit(a)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(key) && b.runtime.source == Some(id))
            {
                r.remove(w, self, a, key)?;
            }
        }
        Ok(())
    }
    pub(super) fn advanced_flush_starts(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        self.flush_kit_events(w, r)?;
        let pending = std::mem::take(&mut self.pending_skill_starts);
        for (id, eligible) in pending {
            self.advanced_learn(w, r, id, Some(&eligible))?;
        }
        Ok(())
    }
    fn advanced_learn(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        eligible: Option<&[Id]>,
    ) -> Result<(), String> {
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        for id in ids {
            if id == source
                || eligible.is_some_and(|a| !a.contains(&id))
                || !w.unit(id)?.stats.alive
                || !w.unit(id)?.deployed
            {
                continue;
            }
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
            let Advanced::Reckpr(p) = &def.extra.advanced else {
                continue;
            };
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            if !body::in_range(w.unit(source)?, &keys)
                || p.probability < 1. && !self.rng.chance(p.probability)
            {
                continue;
            }
            self.advanced_grant_sp(w, r, id, p.sp)?;
            if p.aspd != 0. {
                self.component_buff(
                    w,
                    r,
                    id,
                    "reckpr:learn",
                    [("aspd".into(), p.aspd)].into(),
                    p.learn_duration,
                    id,
                )?;
            }
        }
        Ok(())
    }
    fn advanced_skill_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        dir: Direction,
        def: &Definition,
        s: &mut skills::Runtime,
        dt: f64,
    ) -> Result<(), String> {
        {
            let m = &mut w.unit_mut(id)?.operator.advanced;
            m.elapsed += dt;
            m.accumulator += dt;
            m.aura_accumulator += dt;
        }
        match &def.extra.advanced {
            Advanced::Mostma(p) if p.lock => {
                let targets = self.advanced_enemies(w, id)?;
                for &target in &targets {
                    if w.unit_mut(id)?.operator.advanced.locked.insert(target) {
                        self.advanced_status(
                            w,
                            r,
                            id,
                            target,
                            "stun",
                            s.time_left.max(0.01),
                            None,
                        )?;
                    }
                }
                if w.unit(id)?.operator.advanced.accumulator + 1e-9 >= 1. {
                    w.unit_mut(id)?.operator.advanced.accumulator -= 1.;
                    for target in targets {
                        let damage = Self::advanced_stat(w, id, "atk")? * p.lock_scale;
                        self.advanced_hit_damage(
                            w,
                            r,
                            id,
                            target,
                            damage,
                            DamageType::Arts,
                            true,
                            false,
                            "timeLock",
                        )?;
                    }
                }
            }
            Advanced::Billro(p) if p.mode == Mode::Third => {
                let value = p.ramp_atk
                    * (w.unit(id)?.operator.advanced.elapsed + 1e-9)
                        .floor()
                        .min(20.)
                    / 20.;
                self.advanced_edit_mod(w, r, id, "billro:s3atk", "atkPct", value)?;
            }
            Advanced::Mudrok(p)
                if p.mode == Mode::Third && !Self::advanced_has(w, id, "mudrok:awake") =>
            {
                let keys = targeting::absolute_range(&p.grid, w.unit(id)?.deployment.tile, dir, 0);
                let targets = self.spawns.as_ref().unwrap().live().to_vec();
                let mut nearby = vec![];
                for a in targets {
                    if w.unit(a)?.stats.alive
                        && !w.unit(a)?.hidden
                        && body::in_range(w.unit(a)?, &keys)
                        && targeting::can_target_enemy(
                            w,
                            self,
                            Some(id),
                            a,
                            targeting::Profile {
                                can_hit_fly: true,
                                ..Default::default()
                            },
                        )?
                    {
                        nearby.push(a);
                    }
                }
                for &a in &nearby {
                    self.component_buff(
                        w,
                        r,
                        a,
                        &format!("mudrok:slow:{id}"),
                        [("moveMul".into(), (1. + p.slow).max(0.))].into(),
                        0.25,
                        id,
                    )?;
                }
                if w.unit(id)?.operator.advanced.elapsed + 1e-9 >= p.sleep {
                    r.remove(w, self, id, "mudrok:dormant")?;
                    for a in nearby {
                        if !targeting::flying(w, a)? {
                            self.advanced_status(w, r, id, a, "stun", p.stun, None)?;
                        }
                    }
                    self.component_buff(
                        w,
                        r,
                        id,
                        "mudrok:awake",
                        p.awake.clone(),
                        f64::INFINITY,
                        id,
                    )?;
                }
            }
            Advanced::Flamtl(p) if p.mode == Mode::Third => {
                let step = (s.spec.duration / f64::from(p.total.max(1))).max(DT);
                while w.unit(id)?.operator.advanced.accumulator + 1e-9 >= step
                    && w.unit(id)?.operator.advanced.dp_given < p.total
                {
                    let m = &mut w.unit_mut(id)?.operator.advanced;
                    m.accumulator -= step;
                    m.dp_given += 1;
                    self.roster.as_mut().unwrap().add_dp(p.cost)?;
                }
            }
            Advanced::Fartth(p) if p.mode == Mode::Second => {
                let ids = self.spawns.as_ref().unwrap().live();
                let mut keys: Vec<_> = ids
                    .iter()
                    .filter_map(|&a| {
                        let u = w.unit(a).ok()?;
                        if !u.stats.alive
                            || u.hidden
                            || !u.blocked_by().is_some_and(|b| {
                                w.unit(b)
                                    .is_ok_and(|b| b.stats.alive && b.side == health::Side::Ally)
                            })
                        {
                            return None;
                        }
                        Some(
                            (u.position[0].round() as usize) * super::super::grid::COLS
                                + u.position[1].round() as usize,
                        )
                    })
                    .collect();
                keys.sort_unstable();
                w.unit_mut(id)?.operator.advanced.extra_range = keys;
            }
            Advanced::Lisa(p) if p.fox => {
                if w.unit(id)?.operator.advanced.aura_accumulator >= 0.25 {
                    w.unit_mut(id)?.operator.advanced.aura_accumulator = 0.;
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    let targets = search::Host::enemies(
                        self,
                        id,
                        &keys,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                        w,
                        w.unit_capacity(),
                    )?;
                    for a in targets {
                        self.advanced_status(w, r, id, a, "sluggish", 0.5, None)?;
                    }
                }
                if w.unit(id)?.operator.advanced.accumulator >= 1. {
                    w.unit_mut(id)?.operator.advanced.accumulator -= 1.;
                    let value = Self::advanced_stat(w, id, "atk")? * p.regen;
                    for a in self.advanced_allies(w, id)? {
                        self.component_buff(
                            w,
                            r,
                            a,
                            &format!("lisa:fox:{id}"),
                            [("hpRegen".into(), value)].into(),
                            1.25,
                            id,
                        )?;
                    }
                }
            }
            Advanced::Surtr(p) if p.mode == Mode::Third && p.drain_peak > 0. => {
                while w.unit(id)?.operator.advanced.accumulator >= p.drain_interval
                    && w.unit(id)?.stats.alive
                {
                    w.unit_mut(id)?.operator.advanced.accumulator -= p.drain_interval;
                    let rate = p.drain_peak
                        * (w.unit(id)?.operator.advanced.elapsed / p.drain_ramp).min(1.);
                    let amount = Self::advanced_stat(w, id, "maxHp")? * rate * p.drain_interval;
                    self.support_lose_hp(w, r, id, amount, Some(id), true)?;
                }
            }
            Advanced::Aglina(p)
                if p.weightless && w.unit(id)?.operator.advanced.aura_accumulator >= 0.25 =>
            {
                w.unit_mut(id)?.operator.advanced.aura_accumulator = 0.;
                self.advanced_weightless(w, r, id, s.time_left)?;
            }
            Advanced::Glady(p) if p.mode == Mode::Third => {
                if let Some(pos) = w.unit(id)?.operator.advanced.tornado {
                    let targets = self.advanced_radius(w, pos, 1.5)?;
                    for &a in &targets {
                        self.component_buff(
                            w,
                            r,
                            a,
                            &format!("glady:slow:{id}"),
                            [("moveMul".into(), (1. + p.slow).max(0.))].into(),
                            0.25,
                            id,
                        )?;
                    }
                    if w.unit(id)?.operator.advanced.accumulator + 1e-9 >= p.interval {
                        w.unit_mut(id)?.operator.advanced.accumulator -= p.interval;
                        for a in targets {
                            if w.unit(a)?.stats.alive && !Self::advanced_flag(w, a, "untargetable")?
                            {
                                let amount = Self::advanced_stat(w, id, "atk")? * p.scale;
                                self.advanced_hit_damage(
                                    w,
                                    r,
                                    id,
                                    a,
                                    amount,
                                    DamageType::Arts,
                                    true,
                                    false,
                                    "tornado",
                                )?;
                                self.advanced_pull(w, r, id, a, p, Some(pos))?;
                            }
                        }
                    }
                }
            }
            Advanced::Gnosis(p) if p.mode == Mode::Third => {
                for a in self.advanced_enemies(w, id)? {
                    if let Some(b) = w
                        .unit(a)?
                        .stats
                        .buffs()
                        .iter()
                        .find(|b| b.key.as_deref() == Some("freeze"))
                    {
                        let bid = b.runtime.id();
                        if b.runtime.time_left < s.time_left {
                            r.edit(w, a, bid, |b| {
                                b.runtime.time_left = s.time_left;
                                b.runtime.duration = b.runtime.duration.max(s.time_left);
                            })?;
                        }
                        w.unit_mut(id)?.operator.advanced.locked.insert(a);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn advanced_edit_mod(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
        stat: &str,
        value: f64,
    ) -> Result<(), String> {
        if let Some(b) = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some(key))
        {
            let bid = b.runtime.id();
            if b.mods.get(stat).copied() != Some(value) {
                r.edit(w, id, bid, |b| {
                    b.mods.insert(stat.into(), value);
                })?;
            }
        }
        Ok(())
    }
}
impl Host {
    pub(super) fn advanced_scheduled(&mut self, w: &mut World) -> Result<(), String> {
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut r = self.buffs.take().ok_or("Missing periodic buffs")?;
        let result = (|| {
            self.advanced_flush_starts(w, &mut r)?;
            for &id in &ids {
                if matches!(
                    self.roster
                        .as_ref()
                        .unwrap()
                        .entry(id)?
                        .definition
                        .extra
                        .advanced,
                    Advanced::None
                ) && w.unit(id)?.operator.advanced.locked.is_empty()
                {
                    continue;
                }
                let mut locked = std::mem::take(&mut w.unit_mut(id)?.operator.advanced.locked);
                locked.retain(|a| w.unit(*a).is_ok_and(|u| u.stats.alive));
                w.unit_mut(id)?.operator.advanced.locked = locked;
                self.advanced_periodic(w, &mut r, &ids, id, Periodic::Frame)?;
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn advanced_deadlines(&self, w: &World) -> Result<Vec<(f64, Id, Periodic)>, String> {
        let mut due = vec![];
        for &id in self.roster.as_ref().unwrap().ids() {
            let def = &self.roster.as_ref().unwrap().entry(id)?.definition;
            if matches!(def.extra.advanced, Advanced::None) {
                continue;
            }
            let state = &w.unit(id)?.operator.advanced;
            let period = if matches!(def.extra.advanced, Advanced::Lisa(_) | Advanced::Aglina(_)) {
                0.25
            } else {
                0.2
            };
            for (at, kind) in [
                (state.next_fast.max(0.1), Periodic::Fast),
                (state.next_aura.max(period), Periodic::Aura(period)),
            ] {
                if at <= w.time + 1e-9 {
                    due.push((at, id, kind));
                }
            }
            if let Advanced::Mudrok(p) = &def.extra.advanced {
                if state.next_layer <= w.time + 1e-9 && Self::tac_up(w, id) {
                    due.push((state.next_layer, id, Periodic::Layer(p.layer_period)));
                }
            }
        }
        Ok(due)
    }
    pub(super) fn advanced_periodic(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        ids: &[Id],
        id: Id,
        event: Periodic,
    ) -> Result<(), String> {
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let state = &mut w.unit_mut(id)?.operator.advanced;
        match event {
            Periodic::Fast => state.next_fast = state.next_fast.max(0.1) + 0.1,
            Periodic::Aura(period) => state.next_aura = state.next_aura.max(period) + period,
            Periodic::Layer(period) => state.next_layer += period,
            Periodic::Frame => {}
        }
        if !Self::tac_up(w, id) {
            return Ok(());
        }
        let now = w.time;
        let active = w.unit(id)?.operator.active_skill;
        let fast = matches!(event, Periodic::Fast);
        let aura = matches!(event, Periodic::Aura(_));
        let layer = matches!(event, Periodic::Layer(_));
        let frame = matches!(event, Periodic::Frame);
        match &def.extra.advanced {
            Advanced::Mostma(p) if aura => {
                let slow = p.slow * if p.ripple && active { p.slow_scale } else { 1. };
                for a in self.advanced_enemies(w, id)? {
                    self.component_buff(
                        w,
                        r,
                        a,
                        &format!("mostma:slow:{id}"),
                        [("moveMul".into(), (1. + slow).max(0.))].into(),
                        0.25,
                        id,
                    )?;
                }
            }
            Advanced::Mizuki(p) => {
                if fast {
                    let targets = self.advanced_enemies(w, id)?;
                    let mut on = false;
                    for a in targets {
                        if w.unit(a)?.stats.hp / Self::advanced_stat(w, a, "maxHp")? < p.threshold {
                            on = true;
                            break;
                        }
                    }
                    self.advanced_toggle(
                        w,
                        r,
                        id,
                        "mizuki:t2",
                        on,
                        [("atkPct".into(), p.attack)].into(),
                    )?;
                }
                if aura && p.slow != 0. {
                    for a in self.advanced_enemies(w, id)? {
                        self.component_buff(
                            w,
                            r,
                            a,
                            &format!("mizuki:slow:{id}"),
                            [("moveMul".into(), (1. + p.slow).max(0.))].into(),
                            0.25,
                            id,
                        )?;
                    }
                }
            }
            Advanced::Aroma(p) if frame => {
                let pending = w.unit(id)?.operator.advanced.floating.clone();
                for a in pending {
                    if !w.unit(a).is_ok_and(|u| u.stats.alive) {
                        w.unit_mut(id)?.operator.advanced.floating.remove(&a);
                        continue;
                    }
                    if !Self::advanced_has(w, a, "levitate") {
                        w.unit_mut(id)?.operator.advanced.floating.remove(&a);
                        if active && p.landing {
                            let keys = self.roster.as_ref().unwrap().range(w, id)?;
                            if body::in_range(w.unit(a)?, &keys) {
                                let amount = Self::advanced_stat(w, id, "atk")? * p.land_scale;
                                self.advanced_hit_damage(
                                    w,
                                    r,
                                    id,
                                    a,
                                    amount,
                                    DamageType::Arts,
                                    true,
                                    false,
                                    "landing",
                                )?;
                            }
                        }
                    }
                }
            }
            Advanced::Billro(p) if fast => {
                let ready = self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(id)?
                    .skill()?
                    .bank
                    .charges
                    >= 1.;
                self.advanced_toggle(
                    w,
                    r,
                    id,
                    "billro:t2",
                    ready && !active,
                    [("spRecoveryFlat".into(), p.recovery)].into(),
                )?;
            }
            Advanced::Mudrok(p) => {
                if layer {
                    let layers = w.unit(id)?.operator.advanced.layers;
                    if layers < p.layer_cap {
                        self.advanced_layers(
                            w,
                            r,
                            id,
                            layers.saturating_add(p.layer_add).min(p.layer_cap),
                        )?;
                    }
                }
                if aura && p.module.values().any(|v| *v != 0.) {
                    let home = w.unit(id)?.deployment.tile;
                    let mut lonely = true;
                    for &a in ids {
                        let u = w.unit(a)?;
                        if a != id
                            && u.stats.alive
                            && u.deployed
                            && (u.deployment.tile[0] - home[0]).abs() <= 1
                            && (u.deployment.tile[1] - home[1]).abs() <= 1
                        {
                            lonely = false;
                            break;
                        }
                    }
                    self.advanced_toggle(w, r, id, "mudrok:module", lonely, p.module.clone())?;
                }
            }
            Advanced::Flamtl(p) => {
                if fast && p.module.values().any(|v| *v != 0.) {
                    self.advanced_toggle(
                        w,
                        r,
                        id,
                        "flamtl:module",
                        !w.unit(id)?.blocking().is_empty(),
                        p.module.clone(),
                    )?;
                }
                if aura {
                    for &a in ids {
                        let d = &self.roster.as_ref().unwrap().entry(a)?.definition;
                        let key = &d.key;
                        if self.data.table("chess").unwrap()[key]["nationId"] == "kazimierz"
                            && w.unit(a)?.stats.alive
                            && w.unit(a)?.deployed
                        {
                            self.component_buff(
                                w,
                                r,
                                a,
                                "flamtl:dodge",
                                [("dodgePhys".into(), p.faction_dodge)].into(),
                                0.25,
                                id,
                            )?;
                        }
                    }
                }
            }
            Advanced::Fartth(p) if fast => {
                self.advanced_toggle(
                    w,
                    r,
                    id,
                    "fartth:focus",
                    now - w.unit(id)?.operator.advanced.last_hurt >= p.delay - 1e-9,
                    [("atkPct".into(), p.focus)].into(),
                )?;
            }
            Advanced::Lisa(p) if aura => {
                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                let targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    targeting::Profile {
                        can_hit_fly: true,
                        ..Default::default()
                    },
                    w,
                    w.unit_capacity(),
                )?;
                if p.module_sp > 0. && !targets.is_empty() {
                    self.component_buff(
                        w,
                        r,
                        id,
                        "lisa:module",
                        [("spRecoveryFlat".into(), p.module_sp)].into(),
                        0.5,
                        id,
                    )?;
                }
                for a in targets {
                    if Self::advanced_has(w, a, "sluggish") {
                        self.advanced_status(
                            w,
                            r,
                            id,
                            a,
                            "fragile",
                            0.5,
                            Some(p.fragile * if active { p.boost } else { 1. }),
                        )?;
                    }
                }
            }
            Advanced::Mountain(p) if aura && p.module_aspd != 0. => {
                if w.unit(id)?.stats.hp / Self::advanced_stat(w, id, "maxHp")? > 0.5 {
                    self.component_buff(
                        w,
                        r,
                        id,
                        "f12yin:module",
                        [("aspd".into(), p.module_aspd)].into(),
                        0.3,
                        id,
                    )?;
                }
            }
            Advanced::Surtr(p) if aura => {
                if p.module_aspd != 0. {
                    if w.unit(id)?.blocking().is_empty() {
                        self.component_buff(
                            w,
                            r,
                            id,
                            "surtr:module",
                            [("aspd".into(), p.module_aspd)].into(),
                            0.3,
                            id,
                        )?;
                    } else {
                        r.remove(w, self, id, "surtr:module")?;
                    }
                }
                if p.module_fragile > 0. {
                    let held = w.unit(id)?.blocking().to_vec();
                    for a in held {
                        if w.unit(a)?.stats.alive {
                            self.advanced_status(
                                w,
                                r,
                                id,
                                a,
                                "artsFragile",
                                0.3,
                                Some(p.module_fragile),
                            )?;
                        }
                    }
                }
            }
            Advanced::Aglina(p) if aura => {
                for &a in ids {
                    if targeting::friendly_selectable(w, id, a)? {
                        if p.aspd != 0. {
                            self.component_buff(
                                w,
                                r,
                                a,
                                "aglina:field",
                                [("aspd".into(), p.aspd)].into(),
                                0.5,
                                id,
                            )?;
                        }
                        if !active && p.regen > 0. {
                            self.component_buff(
                                w,
                                r,
                                a,
                                "aglina:parttime",
                                [("hpRegen".into(), p.regen)].into(),
                                0.5,
                                id,
                            )?;
                        }
                    }
                }
                if p.module_sp > 0. {
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    if !search::Host::enemies(
                        self,
                        id,
                        &keys,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                        w,
                        w.unit_capacity(),
                    )?
                    .is_empty()
                    {
                        self.component_buff(
                            w,
                            r,
                            id,
                            "aglina:module",
                            [("spRecoveryFlat".into(), p.module_sp)].into(),
                            0.5,
                            id,
                        )?;
                    }
                }
            }
            Advanced::Glady(p) if aura => {
                for &a in ids {
                    if self.advanced_abyssal(a)? && w.unit(a)?.stats.alive && w.unit(a)?.deployed {
                        self.component_buff(
                            w,
                            r,
                            a,
                            "glady:tide",
                            [("hpRegenRatio".into(), p.regen)].into(),
                            0.25,
                            id,
                        )?;
                    }
                }
            }
            Advanced::Gnosis(p) => {
                if fast {
                    let targets = self.advanced_enemies(w, id)?;
                    let elite = targets
                        .iter()
                        .any(|&a| w.unit(a).is_ok_and(|u| u.boss || u.targeting.elite));
                    if p.module_sp > 0. {
                        self.advanced_toggle(
                            w,
                            r,
                            id,
                            "gnosis:module",
                            elite,
                            [("spRecoveryFlat".into(), p.module_sp)].into(),
                        )?;
                    }
                    for a in targets {
                        let scale = if Self::advanced_flag(w, a, "freeze")? {
                            p.fragile_frozen
                        } else if Self::advanced_flag(w, a, "cold")? {
                            p.fragile_cold
                        } else {
                            0.
                        };
                        if scale > 0. {
                            super::super::valued::apply(
                                w,
                                self,
                                r,
                                a,
                                super::super::valued::Request {
                                    key: "gnosis:fragile",
                                    kind: super::super::valued::Kind::Custom(|v| {
                                        [("dmgTakenMul".into(), v)].into()
                                    }),
                                    plain: true,
                                    duration: 0.15,
                                    value: scale,
                                    source: Some(id),
                                    stack_as: None,
                                },
                            )?;
                        }
                    }
                }
                if aura
                    && now - w.unit(id)?.deployment.deployed_at >= p.resist_delay - 1e-9
                    && p.resist > 0.
                {
                    for &a in ids {
                        let key = &self.roster.as_ref().unwrap().entry(a)?.definition.key;
                        if self.data.table("chess").unwrap()[key]["nationId"] == "kjerag"
                            && w.unit(a)?.stats.alive
                            && w.unit(a)?.deployed
                        {
                            self.advanced_status(w, r, id, a, "resist", 0.25, Some(p.resist))?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
