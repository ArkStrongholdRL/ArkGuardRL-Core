// SPDX-License-Identifier: GPL-3.0-or-later
//! Equipped effects use canonical damage, healing, buffs and status lifetimes.
use super::super::{
    attacks,
    damage::{Damage, DamageType},
    equipment::Effect,
    search, skills,
    stats::{Buff, Numbers},
    targeting,
};
use super::*;
use crate::deterministic::math::hypot;

impl Host {
    pub(super) fn equipment_install(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let roster = self.roster.as_ref().unwrap();
        let loadouts: Vec<_> = roster
            .ids()
            .iter()
            .map(|&id| Ok((id, Arc::clone(&roster.entry(id)?.definition))))
            .collect::<Result<_, String>>()?;
        for (unit, definition) in loadouts {
            for grant in &definition.equipment {
                for buff in &grant.buffs {
                    r.add(
                        w,
                        self,
                        unit,
                        buffs::Spec {
                            buff: buff.clone(),
                            allow_dead: true,
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
                for effect in &grant.effects {
                    match effect {
                        Effect::Cauldron { speed, cap }
                            if self.equipment_member(unit, "yanShip") =>
                        {
                            let count = self.equipment.gained_chess.min(*cap as u64);
                            if count > 0 {
                                self.equipment_stat(
                                    w,
                                    r,
                                    unit,
                                    format!("{}:yan", grant.tag),
                                    [("aspd".into(), speed * count as f64)].into(),
                                )?;
                            }
                        }
                        Effect::Hammer(hammer) => {
                            let hammers = self
                                .equipment
                                .units
                                .get_mut(&unit)
                                .unwrap()
                                .hammers
                                .as_mut()
                                .unwrap();
                            let index = hammer.index();
                            hammers.own[index] += 1;
                            hammers.params[index].get_or_insert(*hammer);
                            self.hammer_refresh(w, r, unit)?;
                        }
                        Effect::Steam(params) => {
                            self.equipment
                                .units
                                .get_mut(&unit)
                                .unwrap()
                                .hammers
                                .as_mut()
                                .unwrap()
                                .steam
                                .get_or_insert(*params);
                            self.hammer_refresh(w, r, unit)?;
                        }
                        _ => (),
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn equipment_member(&self, unit: Id, bond: &str) -> bool {
        self.equipment.units.get(&unit).is_some_and(|loadout| {
            loadout.bonds.iter().any(|b| b == bond)
                || (super::bonds::core_bond(bond)
                    && loadout.bonds.iter().any(|b| b == "maniShip")
                    && self.growth.live.get("maniShip").is_some_and(|b| b.tier > 0))
        })
    }
    pub(super) fn equipment_partner(
        &self,
        unit: Id,
        instance: &super::super::equipment::Instance,
    ) -> bool {
        instance.partner.as_deref().is_some_and(|partner| {
            self.equipment.units[&unit].items.iter().any(|tag| {
                let id = tag.strip_prefix("item:").unwrap();
                let family = id
                    .strip_suffix("_a")
                    .or_else(|| id.strip_suffix("_b"))
                    .unwrap_or(id);
                family
                    == partner
                        .strip_suffix("_a")
                        .or_else(|| partner.strip_suffix("_b"))
                        .unwrap_or(partner)
            })
        })
    }
    pub(super) fn equipment_targets(
        &mut self,
        w: &mut World,
        unit: Id,
        profile: targeting::Profile,
    ) -> Result<Vec<Id>, String> {
        let keys = self.roster.as_ref().unwrap().range(w, unit)?;
        search::Host::enemies(self, unit, &keys, profile, w, w.unit_capacity())
    }
    pub(super) fn equipment_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        scale: f64,
        kind: DamageType,
        tag: &str,
        dodge: bool,
    ) -> Result<(), String> {
        let amount = Self::tac_stat(w, source, "atk")? * scale;
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
                penetration: Default::default(),
                resolved: false,
            },
        );
        q.tags = vec!["item".into(), format!("item:{tag}")];
        if !dodge {
            q.dodge_mode = super::super::injury::DodgeMode::Ignore;
        }
        self.enemy_damage(w, r, &mut q)?;
        Ok(())
    }
    pub(super) fn equipment_chance(&mut self, probability: f64) -> bool {
        probability > 0. && (probability >= 1. || self.rng.next_f64() < probability)
    }
    pub(super) fn equipment_skill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: skills::Event,
        context: &skills::Context,
        skill: &mut skills::Runtime,
    ) -> Result<(), String> {
        let unit = skill.unit();
        let initial = w.time == 0. && w.unit(unit)?.operator.deployments == 1;
        let ammo_left = skill.ammo_left;
        if event == skills::Event::Deploy {
            self.equipment.fields.clear();
        }
        if !matches!(
            event,
            skills::Event::Deploy
                | skills::Event::SkillStart
                | skills::Event::SkillEnd
                | skills::Event::AmmoUsed
        ) {
            return Ok(());
        }
        let direction = self.roster.as_ref().unwrap().entry(unit)?.direction;
        // Dispatch in grant order so mixed loadouts observe the same SP and buff state.
        for (index, instance) in self.equipment.effects(unit) {
            match instance.effect {
                Effect::Chivalry { .. } => {
                    self.chivalry_skill(w, r, unit, index, &instance, event, context, skill)?
                }
                Effect::Compass {
                    initial,
                    end,
                    team,
                    spent,
                } => {
                    if event == skills::Event::Deploy {
                        self.with_kit_buffs(w, r, |host, world| {
                            skill.gain(world, host, initial, "init", false, false)
                        })?;
                    }
                    if !self.equipment_member(unit, "sargonShip") {
                        continue;
                    }
                    if event == skills::Event::SkillEnd && !spent && context.reason != "death" {
                        self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                            Effect::Compass {
                                initial,
                                end,
                                team,
                                spent: true,
                            };
                        if w.unit(unit)?.stats.alive {
                            self.with_kit_buffs(w, r, |host, world| {
                                skill.gain(world, host, end, "item", false, true)
                            })?;
                        }
                    }
                    if event == skills::Event::SkillStart
                        && team > 0.
                        && self.equipment_partner(unit, &instance)
                    {
                        let owner = w.unit(unit)?.owner;
                        for target in self.roster.as_ref().unwrap().ids().to_vec() {
                            if w.unit(target)?.owner != owner
                                || !Self::tac_up(w, target)
                                || !self.equipment_member(target, "sargonShip")
                            {
                                continue;
                            }
                            if target == unit {
                                self.with_kit_buffs(w, r, |host, world| {
                                    skill.gain(world, host, team, "item", false, true)
                                })?;
                            } else {
                                let mut recipient =
                                    self.roster.as_mut().unwrap().take_skill(target)?;
                                let result = self.with_kit_buffs(w, r, |host, world| {
                                    recipient.gain(world, host, team, "item", false, true)
                                });
                                self.roster
                                    .as_mut()
                                    .unwrap()
                                    .restore_skill(target, recipient)?;
                                result?;
                            }
                        }
                    }
                }
                Effect::DeploySp(each) if event == skills::Event::Deploy => {
                    let mine = &self.equipment.units[&unit].bonds;
                    let mut count = 1;
                    for (&other, loadout) in &self.equipment.units {
                        let a = w.unit(other)?;
                        if other != unit
                            && a.owner == w.unit(unit)?.owner
                            && !a.removed
                            && (initial || Self::tac_up(w, other))
                            && loadout.bonds.iter().any(|bond| mine.contains(bond))
                        {
                            count += 1;
                        }
                    }
                    self.with_kit_buffs(w, r, |host, world| {
                        skill.gain(world, host, each * count as f64, "item", false, false)
                    })?;
                }
                Effect::FrontHealth(extra) if event == skills::Event::Deploy => {
                    let front = direction.vector();
                    let tile = w.unit(unit)?.deployment.tile;
                    let front = [tile[0] + front[0], tile[1] + front[1]];
                    let occupied = w.unit_ids().any(|other| {
                        let a = w.unit(other).unwrap();
                        other != unit
                            && a.side == health::Side::Ally
                            && a.deployment.kind == super::super::deploy::Kind::Operator
                            && !a.removed
                            && if initial {
                                a.deployment.home == front
                            } else {
                                Self::tac_up(w, other) && a.deployment.tile == front
                            }
                    });
                    let key = format!("{}:front", instance.tag);
                    if occupied {
                        r.remove(w, self, unit, &key)?;
                    } else {
                        r.add(
                            w,
                            self,
                            unit,
                            buffs::Spec {
                                buff: Buff {
                                    key: Some(key),
                                    mods: [("hpPct".into(), extra)].into(),
                                    permanent: true,
                                    ..Default::default()
                                },
                                mods_present: true,
                                ..Default::default()
                            },
                        )?;
                    }
                }
                Effect::AmmoClip {
                    probability,
                    ratio,
                    cap,
                    mut used,
                    mut deployment,
                } if event == skills::Event::AmmoUsed && ammo_left == 1. => {
                    let current = w.unit(unit)?.deployment.deploy_seq;
                    if current != deployment {
                        used = 0;
                        deployment = current;
                    }
                    if used < cap && self.equipment_chance(probability) {
                        used += 1;
                        skill.add_ammo((ratio * skill.spec.ammo - 1e-9).ceil().max(1.));
                    }
                    self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                        Effect::AmmoClip {
                            probability,
                            ratio,
                            cap,
                            used,
                            deployment,
                        };
                }
                Effect::SkillAttack {
                    attack,
                    cap,
                    probability,
                    stacks,
                } if event == skills::Event::SkillStart => {
                    if stacks >= cap || !self.equipment_chance(probability) {
                        continue;
                    }
                    self.equipment.units.get_mut(&unit).unwrap().effects[index].effect =
                        Effect::SkillAttack {
                            attack,
                            cap,
                            probability,
                            stacks: stacks + 1,
                        };
                    self.equipment_stat(
                        w,
                        r,
                        unit,
                        format!("{}:stacks", instance.tag),
                        [("atkPct".into(), attack * (stacks + 1) as f64)].into(),
                    )?;
                }
                _ => (),
            }
        }
        Ok(())
    }
    pub(super) fn equipment_stat(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
        key: String,
        mods: Numbers,
    ) -> Result<(), String> {
        let mut buff = Buff {
            key: Some(key),
            mods,
            permanent: true,
            ..Default::default()
        };
        buff.runtime.persist = true;
        r.add(
            w,
            self,
            unit,
            buffs::Spec {
                buff,
                mods_present: true,
                allow_dead: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    pub(super) fn equipment_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        attack: &attacks::Context,
    ) -> Result<(), String> {
        let unit = attack.attacker;
        for (index, instance) in self.equipment.effects(unit) {
            if self.equipment.units[&unit]
                .hammers
                .as_ref()
                .is_some_and(|hammers| hammers.order == index)
            {
                self.hammer_attack(w, r, attack)?;
            }
            match instance.effect {
                Effect::GunKnight {
                    probability,
                    scale,
                    combo,
                } => {
                    if !attack
                        .targets
                        .iter()
                        .any(|&id| w.unit(id).is_ok_and(|u| u.side == health::Side::Enemy))
                        || !self.equipment_member(unit, "lateranoShip")
                        || !self.equipment_chance(probability)
                    {
                        continue;
                    }
                    let combat = self.tac_def(unit)?.combat;
                    let profile = if w.unit(unit)?.operator.active_skill {
                        combat.skill_profile.unwrap_or(combat.profile)
                    } else {
                        combat.profile
                    }
                    .selection;
                    let targets = self.equipment_targets(w, unit, profile)?;
                    if let Some(target) = self.rng.pick(&targets).copied() {
                        let scale = if self.equipment_partner(unit, &instance) {
                            combo
                        } else {
                            scale
                        };
                        self.equipment_damage(
                            w,
                            r,
                            unit,
                            target,
                            scale,
                            DamageType::Physical,
                            "gunknight",
                            true,
                        )?;
                    }
                }
                Effect::Accelerator { speed, cap } => {
                    let mut buff = Buff {
                        key: Some(format!("{}:stacks", instance.tag)),
                        mods: [("aspd".into(), speed)].into(),
                        permanent: true,
                        ..Default::default()
                    };
                    buff.runtime.persist = true;
                    buff.runtime.refresh = buffs::Refresh::Stack;
                    buff.runtime.max_stacks = cap;
                    r.add(
                        w,
                        self,
                        unit,
                        buffs::Spec {
                            buff,
                            mods_present: true,
                            allow_dead: true,
                            ..Default::default()
                        },
                    )?;
                }
                Effect::Silence(duration) => {
                    for &target in &attack.targets {
                        if w.unit(target)?.side == health::Side::Enemy
                            && w.unit(target)?.stats.alive
                        {
                            self.apply_status(
                                w,
                                r,
                                target,
                                "silence",
                                super::super::status::Options {
                                    duration,
                                    source: Some(unit),
                                    ..Default::default()
                                },
                            )?;
                        }
                    }
                }
                Effect::Grenade { window, duration } => {
                    if w.time - w.unit(unit)?.deployment.deployed_at <= window + 1e-9 {
                        for &target in &attack.targets {
                            if w.unit(target)?.side == health::Side::Enemy
                                && w.unit(target)?.stats.alive
                            {
                                self.apply_status(
                                    w,
                                    r,
                                    target,
                                    "stun",
                                    super::super::status::Options {
                                        duration,
                                        source: Some(unit),
                                        ..Default::default()
                                    },
                                )?;
                            }
                        }
                    }
                }
                Effect::AttackHeal(ratio) => {
                    for &target in &attack.targets {
                        if w.unit(target)?.side == health::Side::Enemy
                            && w.unit(target)?.stats.alive
                        {
                            let max_hp = w
                                .unit_mut(unit)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("maxHp");
                            self.support_heal(
                                w,
                                r,
                                Some(unit),
                                unit,
                                max_hp * ratio,
                                health::HealOptions {
                                    self_heal: true,
                                    ..Default::default()
                                },
                            )?;
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
    pub(super) fn equipment_retype(
        &self,
        w: &mut World,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        let Some(source) = q.hit.source else {
            return Ok(());
        };
        if !matches!(q.hit.damage_type, DamageType::Physical | DamageType::Arts)
            || !(self.core_weakness(w, source, q.hit.target)
                || self
                    .equipment
                    .effects(source)
                    .iter()
                    .any(|(_, instance)| matches!(instance.effect, Effect::Weakness)))
        {
            return Ok(());
        }
        let ss = w
            .unit_mut(source)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .clone();
        let ts = w
            .unit_mut(q.hit.target)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .clone();
        let penetration = super::super::damage::Penetration {
            def_flat: q.damage.penetration.def_flat + ss.n("defIgnoreFlat"),
            def_pct: q.damage.penetration.def_pct + ss.n("defIgnorePct"),
            res_flat: q.damage.penetration.res_flat + ss.n("resIgnoreFlat"),
            res_pct: q.damage.penetration.res_pct + ss.n("resIgnorePct"),
        };
        let physical = super::super::damage::mitigate(
            q.damage.amount,
            DamageType::Physical,
            &ts,
            penetration,
            0.,
        );
        let arts =
            super::super::damage::mitigate(q.damage.amount, DamageType::Arts, &ts, penetration, 0.);
        if arts > physical + 1e-9 {
            q.hit.damage_type = DamageType::Arts;
        } else if physical > arts + 1e-9 {
            q.hit.damage_type = DamageType::Physical;
        }
        Ok(())
    }
    pub(super) fn equipment_deadlines(
        &self,
        w: &World,
    ) -> Vec<(f64, Id, super::super::equipment::Timer)> {
        use super::super::equipment::Timer;
        let mut due = Vec::new();
        for (&unit, loadout) in &self.equipment.units {
            for (index, instance) in loadout.effects.iter().enumerate() {
                if let Effect::SideAura { next, .. }
                | Effect::Tears { next, .. }
                | Effect::Chivalry { next, .. }
                | Effect::Crest { next, .. } = instance.effect
                {
                    if next <= w.time + 1e-9 {
                        due.push((next, unit, Timer::Effect(index)));
                    }
                }
                if let Some(hammers) = &loadout.hammers {
                    if hammers.order == index && hammers.next <= w.time + 1e-9 {
                        due.push((hammers.next, unit, Timer::Hammers));
                    }
                }
            }
        }
        due
    }
    pub(super) fn equipment_periodic(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
        timer: super::super::equipment::Timer,
    ) -> Result<(), String> {
        let index = match timer {
            super::super::equipment::Timer::Effect(index) => index,
            super::super::equipment::Timer::Hammers => {
                self.equipment
                    .units
                    .get_mut(&unit)
                    .unwrap()
                    .hammers
                    .as_mut()
                    .unwrap()
                    .next += 0.5;
                return self.hammer_refresh(w, r, unit);
            }
        };
        let instance = self.equipment.units[&unit].effects[index].clone();
        if matches!(
            instance.effect,
            Effect::Crest { .. } | Effect::Chivalry { .. }
        ) {
            return self.equipment_special_periodic(w, r, unit, index, &instance);
        }

        if let Effect::Tears {
            interval,
            next,
            scale,
            combo,
        } = instance.effect
        {
            self.equipment.units.get_mut(&unit).unwrap().effects[index].effect = Effect::Tears {
                interval,
                next: next + interval,
                scale,
                combo,
            };
            if Self::tac_up(w, unit) && self.equipment_member(unit, "kjeragShip") {
                let scale = if self.equipment_partner(unit, &instance) {
                    combo
                } else {
                    scale
                };
                let targets = self.equipment_targets(
                    w,
                    unit,
                    targeting::Profile {
                        can_hit_fly: true,
                        ..Default::default()
                    },
                )?;
                for target in targets {
                    let stats = w.unit_mut(target)?.stats.stats().map_err(str::to_string)?;
                    if stats.flags.contains("cold") || stats.flags.contains("freeze") {
                        self.equipment_damage(
                            w,
                            r,
                            unit,
                            target,
                            scale,
                            DamageType::Arts,
                            "tears",
                            false,
                        )?;
                    }
                }
            }
        }
        if let Effect::SideAura { speed, next } = instance.effect {
            self.equipment.units.get_mut(&unit).unwrap().effects[index].effect = Effect::SideAura {
                speed,
                next: next + 0.5,
            };
            if !Self::tac_up(w, unit) {
                return Ok(());
            }
            let direction = self.roster.as_ref().unwrap().entry(unit)?.direction;
            let tile = w.unit(unit)?.deployment.tile;
            let owner = w.unit(unit)?.owner;
            for side in [[1, 0], [-1, 0]] {
                let delta = direction.rotate(side);
                let at = [tile[0] + delta[0], tile[1] + delta[1]];
                for other in self.roster.as_ref().unwrap().ids().to_vec() {
                    let a = w.unit(other)?;
                    if a.owner != owner || !Self::tac_up(w, other) || a.deployment.tile != at {
                        continue;
                    }
                    let mut buff = Buff {
                        key: Some(format!("{}:side", instance.tag)),
                        mods: [("aspd".into(), speed)].into(),
                        ..Default::default()
                    };
                    buff.runtime.duration = 0.6;
                    r.add(
                        w,
                        self,
                        other,
                        buffs::Spec {
                            buff,
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn equipment_hit(
        &mut self,
        w: &mut World,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        if q.hit.damage_type == DamageType::Elemental {
            return Ok(());
        }
        if let Some(unit) = q.hit.source {
            for (_, instance) in self.equipment.effects(unit) {
                match instance.effect {
                    Effect::Sniper { radius, scale }
                        if w.unit(q.hit.target)?.side == health::Side::Enemy =>
                    {
                        let a = w.unit(unit)?.position;
                        let b = w.unit(q.hit.target)?.position;
                        if hypot(b[0] - a[0], b[1] - a[1]) >= radius - 1e-6 {
                            q.damage.mul *= scale;
                        }
                    }
                    Effect::Flag {
                        scale,
                        window,
                        interval,
                        decay,
                    } => {
                        let t = w.time - w.unit(unit)?.deployment.deployed_at;
                        let m = if t > window + 1e-9 {
                            if decay < 0. {
                                (scale + decay * ((t - window) / interval - 1e-9).ceil()).max(1.)
                            } else {
                                1.
                            }
                        } else {
                            scale
                        };
                        if m != 1. {
                            q.damage.mul *= m;
                        }
                    }
                    _ => (),
                }
            }
        }
        let target = w.unit(q.hit.target)?;
        if !target.blocking.is_empty()
            && q.hit
                .source
                .is_some_and(|source| source != q.hit.target && !target.blocking.contains(&source))
        {
            for (_, instance) in self.equipment.effects(q.hit.target) {
                if let Effect::RiotShield(scale) = instance.effect {
                    q.damage.mul *= scale;
                }
            }
        }
        Ok(())
    }
    pub(super) fn equipment_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: &health::Event,
        q: &hit::Request,
    ) -> Result<(), String> {
        let target = event.target;
        if !q.tags.iter().any(|tag| tag == "item")
            && self.equipment_member(target, "egirShip")
            && w.unit(target)?.stats.alive
        {
            if let Some(source) = event.source.filter(|&source| {
                w.unit(source)
                    .is_ok_and(|unit| unit.side == health::Side::Enemy && unit.stats.alive)
            }) {
                for (index, instance) in self.equipment.effects(target) {
                    if let Effect::Trench {
                        cooldown,
                        scale,
                        ready,
                    } = instance.effect
                    {
                        if w.time < ready {
                            continue;
                        }
                        self.equipment.units.get_mut(&target).unwrap().effects[index].effect =
                            Effect::Trench {
                                cooldown,
                                scale,
                                ready: w.time + cooldown,
                            };
                        let hits = if self.equipment_partner(target, &instance) {
                            2
                        } else {
                            1
                        };
                        for _ in 0..hits {
                            if w.unit(source)?.stats.alive {
                                self.equipment_damage(
                                    w,
                                    r,
                                    target,
                                    source,
                                    scale,
                                    DamageType::Arts,
                                    "trench",
                                    true,
                                )?;
                            }
                        }
                    }
                }
            }
        }
        self.equipment_crest_landed(w, r, event, q)?;
        if event.amount <= 0. || q.hit.hp_loss {
            return Ok(());
        }
        for (index, instance) in self.equipment.effects(target) {
            if let Effect::Camouflage {
                duration,
                used: false,
            } = instance.effect
            {
                if !w.unit(target)?.stats.alive {
                    continue;
                }
                self.equipment.units.get_mut(&target).unwrap().effects[index].effect =
                    Effect::Camouflage {
                        duration,
                        used: true,
                    };
                self.apply_status(
                    w,
                    r,
                    target,
                    "stealth",
                    super::super::status::Options {
                        duration,
                        source: Some(target),
                        ..Default::default()
                    },
                )?;
            }
        }
        if q.hit.damage_type == DamageType::Elemental
            || w.unit(target)?.side != health::Side::Enemy
            || !w.unit(target)?.stats.alive
            || w.unit(target)?.stats.hp <= 0.
        {
            return Ok(());
        }
        if let Some(source) = event.source {
            for (index, instance) in self.equipment.effects(source) {
                if self.equipment.units[&source]
                    .hammers
                    .as_ref()
                    .is_some_and(|hammers| hammers.order == index)
                {
                    self.hammer_landed(w, r, event, q)?;
                }
                if let Effect::DamageStatus {
                    key,
                    probability,
                    duration,
                } = instance.effect
                {
                    if self.equipment_chance(probability) {
                        self.apply_status(
                            w,
                            r,
                            target,
                            key,
                            super::super::status::Options {
                                duration,
                                value: (key == "palsy").then_some(1.),
                                source: Some(source),
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn equipment_before_status(
        &self,
        w: &mut World,
        c: &mut super::super::status::Context,
    ) -> Result<(), String> {
        if !matches!(
            c.key.as_str(),
            "stun" | "freeze" | "sleep" | "levitate" | "palsy"
        ) {
            return Ok(());
        }
        for (_, instance) in self.equipment.effects(c.target) {
            if let Effect::StatusImmunity(threshold) = instance.effect {
                let stats = &mut w.unit_mut(c.target)?.stats;
                let max = stats.stats().map_err(str::to_string)?.n("maxHp");
                if stats.hp / max > threshold {
                    c.cancelled = true;
                }
            }
        }
        Ok(())
    }
    pub(super) fn equipment_heal(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        options: health::HealOptions,
    ) -> Result<(), String> {
        if e.amount <= 0. || options.regen || options.self_heal || !w.unit(e.target)?.stats.alive {
            return Ok(());
        }
        let Some(source) = e.source else {
            return Ok(());
        };
        for (_, instance) in self.equipment.effects(source) {
            if let Effect::HealShield { probability, cap } = instance.effect {
                let have = w
                    .unit(e.target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("item:shield_drone"))
                    .map_or(0, |b| b.runtime.shield_hits);
                if have >= cap || !self.equipment_chance(probability) {
                    continue;
                }
                let mut buff = Buff {
                    key: Some("item:shield_drone".into()),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.shield_hits = have + 1;
                buff.runtime.visible = true;
                r.add(
                    w,
                    self,
                    e.target,
                    buffs::Spec {
                        buff,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn equipment_pegasus_heal(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        if !w.unit(c.unit)?.deployed {
            return Ok(());
        }
        let buff = r.get(w, c.unit, c.buff)?;
        let ratio = buff.runtime.tick_payload;
        let key = buff.key.as_deref().unwrap_or("");
        let instance = self
            .equipment
            .effects(c.unit)
            .into_iter()
            .find(|(_, instance)| {
                matches!(instance.effect, Effect::PegasusHeal)
                    && key.strip_suffix(":set") == Some(instance.tag.as_ref())
            });
        if let Some((_, instance)) = instance {
            if self.equipment_partner(c.unit, &instance) {
                let amount = Self::tac_stat(w, c.unit, "maxHp")? * ratio;
                self.support_heal(
                    w,
                    r,
                    Some(c.unit),
                    c.unit,
                    amount,
                    health::HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn equipment_before_apply(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        parent: &hit::Request,
    ) -> Result<(), String> {
        let Some(source) = parent.hit.source else {
            return Ok(());
        };
        let target = parent.hit.target;
        if parent.attack_type == hit::AttackType::Addition
            || w.unit(target)?.side != health::Side::Enemy
            || !w.unit(target)?.stats.alive
            || w.unit(target)?.stats.hp <= 0.
        {
            return Ok(());
        }
        for (_, instance) in self.equipment.effects(source) {
            if let Effect::PegasusDamage(scale) = instance.effect {
                if !self.equipment_partner(source, &instance) {
                    continue;
                }
                let mut q = hit::Request::new(
                    health::Hit {
                        source: None,
                        credit: Some(source),
                        target,
                        damage_type: DamageType::True,
                        no_sp: false,
                        silent: false,
                        hp_loss: false,
                        on_applied: false,
                    },
                    Damage {
                        amount: parent.damage.amount * scale,
                        mul: 1.,
                        resolved: false,
                        penetration: Default::default(),
                    },
                );
                q.attack_type = hit::AttackType::Addition;
                q.calculation = hit::Calculation::Precomputed;
                q.tags = vec!["item".into(), "item:pegasus".into()];
                self.enemy_damage(w, r, &mut q)?;
            }
        }
        Ok(())
    }
    pub(super) fn equipment_drain(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        if !w.unit(c.unit)?.deployed {
            return Ok(());
        }
        let amount = r.get(w, c.unit, c.buff)?.runtime.tick_payload;
        let mut q = hit::Request::new(
            health::Hit {
                source: None,
                credit: Some(c.unit),
                target: c.unit,
                damage_type: DamageType::True,
                no_sp: false,
                silent: false,
                hp_loss: false,
                on_applied: false,
            },
            Damage {
                amount,
                mul: 1.,
                penetration: Default::default(),
                resolved: false,
            },
        );
        q.dodge_mode = super::super::injury::DodgeMode::Ignore;
        q.attack_type = hit::AttackType::Buff;
        q.tags = vec!["dot".into(), "periodic".into()];
        self.enemy_damage(w, r, &mut q)?;
        Ok(())
    }
}
