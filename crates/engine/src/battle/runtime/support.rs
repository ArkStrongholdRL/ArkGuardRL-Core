// SPDX-License-Identifier: GPL-3.0-or-later
//! Native execution of healer and support components, sharing canonical buffs.
use super::super::{
    attacks, body,
    content::operators::*,
    damage::{Damage, DamageType},
    direction::Direction,
    displacement,
    health::HealOptions,
    hit,
    operators::Definition,
    search, skills,
    stats::{Buff, Numbers},
    targeting,
};
use super::*;
use crate::deterministic::math::hypot;
pub(super) struct ZoneState {
    source: Id,
    position: [f64; 2],
    damage: f64,
    left: u32,
    next: f64,
}
struct Healing<'a> {
    options: HealOptions,
    host: &'a mut Host,
    buffs: &'a mut buffs::Runtime,
}
impl health::Hooks for Healing<'_> {
    fn adjust_heal(
        &mut self,
        source: Option<Id>,
        target: Id,
        amount: f64,
        options: HealOptions,
        w: &mut World,
    ) -> Result<f64, String> {
        self.host
            .support_heal_amount(w, source, target, amount, options)
    }
    fn emit(&mut self, e: &mut health::Event, w: &mut World) -> Result<(), String> {
        if e.stage == health::Stage::Heal {
            self.host.kit_heal_event(w, self.buffs, e, self.options)?;
            self.host
                .support_overflow(w, self.buffs, e.target, e.amount)?;
            if e.amount > 0. && !self.options.regen {
                self.buffs.remove(w, self.host, e.target, "ab:bleed")?;
            }
        }
        if e.stage == health::Stage::Damaged && e.kind == health::Kind::HpLoss {
            self.host
                .helm_taken(w, self.buffs, e.target, e.source, e.credit, e.amount)?;
            self.host
                .echo_taken(w, self.buffs, e.target, e.source, e.credit)?;
        }
        settlement::Hooks::health_event(self.host, e, self.buffs, w)
    }
    fn kill(&mut self, target: Id, credit: Option<Id>, w: &mut World) -> Result<(), String> {
        settlement::Hooks::health_kill(self.host, target, credit, self.buffs, w)
    }
}
impl spatial::Healing for Host {
    fn heal_through(&mut self, _: Id, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
}
impl Host {
    pub(super) fn support_trigger_range(
        &self,
        w: &World,
        id: Id,
        def: &Definition,
    ) -> Result<Vec<usize>, String> {
        if let Some(grid) = &def.trigger_range {
            let dir = self
                .roster
                .as_ref()
                .ok_or("Missing trigger roster")?
                .entry(id)?
                .direction;
            Ok(targeting::absolute_range(
                grid,
                w.unit(id)?.deployment.tile,
                dir,
                0,
            ))
        } else {
            self.roster
                .as_ref()
                .ok_or("Missing trigger roster")?
                .range(w, id)
        }
    }
    fn support_allies(&self, w: &mut World, source: Id) -> Result<Vec<Id>, String> {
        let roster = self.roster.as_ref().ok_or("Missing support roster")?;
        let keys = roster.range(w, source)?;
        let mut ids = Vec::new();
        for &id in roster.ids() {
            if targeting::friendly_selectable(w, source, id)? && body::in_range(w.unit(id)?, &keys)
            {
                ids.push(id);
            }
        }
        Ok(ids)
    }
    fn support_top(&self, w: &mut World, source: Id) -> Result<Option<Id>, String> {
        let mut candidates = Vec::new();
        for id in self.support_allies(w, source)? {
            let u = w.unit_mut(id)?;
            candidates.push((
                id,
                u.stats.stats().map_err(str::to_string)?.n("maxHp"),
                u.deployment.deploy_seq,
            ));
        }
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
        Ok(candidates.first().map(|x| x.0))
    }
    pub(super) fn support_heal_amount(
        &self,
        w: &mut World,
        source: Option<Id>,
        target: Id,
        mut amount: f64,
        options: HealOptions,
    ) -> Result<f64, String> {
        if let Some(source) = source.filter(|&id| w.unit(id).is_ok_and(|u| u.has_operator_kit())) {
            let def = self.tac_def(source)?;
            if let Some(kit) = def.extra.tactics.behavior() {
                amount *= kit.heal_multiplier(w, target)?;
            }
        }
        if options.regen || options.restore {
            return Ok(amount);
        }
        let roster = self.roster.as_ref().ok_or("Missing heal content context")?;
        let max = w
            .unit_mut(target)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("maxHp");
        let ratio = if max > 0. {
            w.unit(target)?.stats.hp / max
        } else {
            0.
        };
        if w.unit(target)?.side == health::Side::Ally {
            let support = &roster.entry(target)?.definition.extra.support;
            if let Some(bonus) = &support.received_heal {
                amount *= if ratio < bonus.below {
                    bonus.low
                } else {
                    bonus.normal
                };
            }
        }
        // Installation order governs multiple copies of the same external modifier.
        for &id in roster.ids() {
            let u = w.unit(id)?;
            let support = &roster.entry(id)?.definition.extra.support;
            if u.stats.alive
                && u.deployed
                && support.top_heal != 0.
                && support.top_heal != 1.
                && self.support_top(w, id)? == Some(target)
            {
                amount *= support.top_heal;
            }
            if source == Some(id) {
                // Installed module heal modifiers also apply to lingering off-field heals.
                if let Some(bonus) = support.heal_bonus {
                    let active = w.unit(id)?.operator.active_skill;
                    let guardian = roster.entry(id)?.definition.profession == "TANK";
                    if (!guardian || active && !options.self_heal)
                        && (ratio < bonus.below || bonus.inclusive && ratio <= bonus.below)
                    {
                        amount *= bonus.scale;
                    }
                }
            }
        }
        self.kit_heal_amount(w, source, target, amount, options)
    }
    fn support_overflow(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        amount: f64,
    ) -> Result<(), String> {
        if w.unit(id)?.side != health::Side::Ally {
            return Ok(());
        }
        let ratio = self
            .roster
            .as_ref()
            .ok_or("Missing overflow roster")?
            .entry(id)?
            .definition
            .extra
            .support
            .overflow;
        if ratio <= 0. || amount <= 0. {
            return Ok(());
        }
        let max = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("maxHp");
        let excess = amount - (max - w.unit(id)?.stats.hp);
        if excess <= 0. {
            return Ok(());
        }
        let have = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some("humus:recycle"))
            .map(|b| b.shield)
            .unwrap_or(0.);
        let value = (have + excess).min(max * ratio);
        if value <= have + 1e-6 {
            return Ok(());
        }
        let mut b = Buff {
            key: Some("humus:recycle".into()),
            shield: value,
            ..Default::default()
        };
        b.runtime.source = Some(id);
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
    pub(super) fn support_heal(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Option<Id>,
        target: Id,
        amount: f64,
        opts: HealOptions,
    ) -> Result<f64, String> {
        health::heal(
            w,
            &mut Healing {
                options: opts,
                host: self,
                buffs: r,
            },
            source,
            target,
            amount,
            opts,
        )
    }
    pub(super) fn lose_hp_with_credit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        hit: health::Hit,
        amount: f64,
        no_limit: bool,
    ) -> Result<f64, String> {
        health::lose_hp(
            w,
            &mut Healing {
                options: HealOptions::default(),
                host: self,
                buffs: r,
            },
            hit,
            amount,
            Default::default(),
            no_limit,
        )
    }
    pub(super) fn support_lose_hp(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        amount: f64,
        source: Option<Id>,
        silent: bool,
    ) -> Result<(), String> {
        self.lose_hp_with_credit(
            w,
            r,
            health::Hit {
                source,
                credit: source,
                target: id,
                damage_type: DamageType::True,
                no_sp: true,
                silent,
                hp_loss: true,
                on_applied: false,
            },
            amount,
            false,
        )?;
        Ok(())
    }
    pub(super) fn support_drain(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
        token: u64,
    ) -> Result<(), String> {
        let mut amount = r.get(w, c.unit, c.buff)?.runtime.tick_payload;
        if token == PLASMA_DRAIN {
            amount *= w
                .unit_mut(c.unit)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("maxHp");
        }
        health::lose_hp(
            w,
            &mut Healing {
                options: HealOptions::default(),
                host: self,
                buffs: r,
            },
            health::Hit {
                source: None,
                credit: None,
                target: c.unit,
                damage_type: DamageType::True,
                no_sp: true,
                silent: false,
                hp_loss: true,
                on_applied: false,
            },
            amount,
            Default::default(),
            false,
        )?;
        Ok(())
    }
    fn support_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        key: &str,
        duration: f64,
        value: Option<f64>,
    ) -> Result<(), String> {
        if w.unit(target)?.stats.alive && duration > 0. {
            self.apply_status(
                w,
                r,
                target,
                key,
                super::super::status::Options {
                    duration,
                    value,
                    source: Some(source),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    fn support_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        amount: f64,
        tag: &str,
    ) -> Result<f64, String> {
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(source),
                credit: Some(source),
                target,
                damage_type: DamageType::Arts,
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
        q.tags = vec!["skill".into(), tag.into()];
        if tag == "zone" {
            q.dodge_mode = super::super::injury::DodgeMode::Ignore;
            q.tags.push("dot".into());
        }
        self.resolve_hit(w, r, &mut q)
    }
    fn support_enemy_aura(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        a: &EnemyAura,
    ) -> Result<(), String> {
        let up = w.unit(id)?.stats.alive && w.unit(id)?.deployed;
        let active = w.unit(id)?.operator.active_skill;
        let on = up && (!a.active_only || active);
        let keys = if on {
            self.roster
                .as_ref()
                .ok_or("Missing aura roster")?
                .range(w, id)?
        } else {
            vec![]
        };
        let all: Vec<_> = w.unit_ids().collect();
        for target in all {
            if w.unit(target)?.side != health::Side::Enemy {
                continue;
            }
            let u = w.unit_mut(target)?;
            let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
            let filter = match a.filter {
                AuraFilter::All => true,
                AuraFilter::Normal => !u.boss && !u.targeting.elite,
                AuraFilter::LowHp(h) => max > 0. && u.stats.hp / max < h,
            };
            if on && u.stats.alive && !u.hidden && filter && body::in_range(u, &keys) {
                let mut mods = a.mods.clone();
                if a.growth.is_some() {
                    if let Some(value) = mods.get_mut("dmgTakenMul") {
                        *value = 1. + (*value - 1.) * w.unit(id)?.operator.support.fragile_scale;
                    }
                }
                let mut b = Buff {
                    key: Some(a.key.into()),
                    mods,
                    ..Default::default()
                };
                b.runtime.duration = (a.period * 2.).max(0.1) + 0.05;
                b.runtime.refresh = buffs::Refresh::Extend;
                b.runtime.source = Some(id);
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
            } else if w
                .unit(target)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(a.key) && b.runtime.source == Some(id))
            {
                r.remove(w, self, target, a.key)?;
            }
        }
        Ok(())
    }
    fn support_phalanx(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Phalanx,
        active: bool,
    ) -> Result<(), String> {
        if active {
            if p.keep.values().any(|v| *v != 0.) {
                self.component_buff(
                    w,
                    r,
                    id,
                    "trait:mint_keep",
                    p.keep.clone(),
                    f64::INFINITY,
                    id,
                )?;
            }
            self.component_buff(
                w,
                r,
                id,
                "talent:mint_taunt",
                [("taunt".into(), p.taunt)].into(),
                f64::INFINITY,
                id,
            )?;
        } else {
            r.remove(w, self, id, "trait:mint_keep")?;
            r.remove(w, self, id, "talent:mint_taunt")?;
        }
        self.support_def_aura(w, r, id, &p.aura, !active)
    }
    fn support_def_aura(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        a: &StatAura,
        on: bool,
    ) -> Result<(), String> {
        let up = on && w.unit(id)?.stats.alive && w.unit(id)?.deployed;
        let keys = if up {
            let dir = self
                .roster
                .as_ref()
                .ok_or("Missing ally aura context")?
                .entry(id)?
                .direction;
            targeting::absolute_range(
                a.grid.as_ref().ok_or("Missing ally aura grid")?,
                w.unit(id)?.deployment.tile,
                dir,
                0,
            )
        } else {
            vec![]
        };
        let ids: Vec<_> = w.unit_ids().collect();
        for target in ids {
            if w.unit(target)?.side != health::Side::Ally {
                continue;
            }
            if up
                && targeting::friendly_selectable(w, id, target)?
                && body::in_range(w.unit(target)?, &keys)
            {
                let mut b = Buff {
                    key: Some(a.key.into()),
                    mods: a.mods.clone(),
                    ..Default::default()
                };
                b.runtime.duration = 0.4 + 0.05;
                b.runtime.refresh = buffs::Refresh::Extend;
                b.runtime.source = Some(id);
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
            } else if w
                .unit(target)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(a.key) && b.runtime.source == Some(id))
            {
                r.remove(w, self, target, a.key)?;
            }
        }
        Ok(())
    }
    fn support_peaks(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        peaks: &[[f64; 2]],
    ) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        let ratio = if max > 0. { u.stats.hp / max } else { 0. };
        let value = peaks
            .iter()
            .find(|p| ratio > p[0])
            .map(|p| p[1])
            .unwrap_or(0.);
        let old = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some("humus:peak"))
            .and_then(|b| b.mods.get("atkPct"))
            .copied()
            .unwrap_or(0.);
        if old != value {
            if value > 0. {
                self.component_buff(
                    w,
                    r,
                    id,
                    "humus:peak",
                    [("atkPct".into(), value)].into(),
                    f64::INFINITY,
                    id,
                )?;
            } else {
                r.remove(w, self, id, "humus:peak")?;
            }
        }
        Ok(())
    }
    pub(super) fn support_skill(
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
        let cfg = &def.extra.support;
        if let Some(p) = &cfg.phalanx {
            // Profession hooks run after the Kit's start/end callbacks.
            if event == skills::Event::SkillStart {
                r.remove(w, self, id, "trait:phalanxGuard")?;
            } else if event == skills::Event::SkillEnd && w.unit(id)?.stats.alive {
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
        }
        if event == skills::Event::Start {
            if let Some(cook) = &cfg.cook {
                let mut b = Buff {
                    key: Some("sunbr:cook".into()),
                    mods: [("defPct".into(), cook.defence)].into(),
                    flags: ["disarm".into()].into(),
                    ..Default::default()
                };
                b.runtime.duration = cook.duration;
                r.add(
                    w,
                    self,
                    id,
                    buffs::Spec {
                        buff: b,
                        mods_present: true,
                        flags_present: true,
                        ..Default::default()
                    },
                )?;
            }
            if let Some(q) = &cfg.quiet {
                w.unit_mut(id)?.operator.support.quiet_until = w.time + q.duration;
                let keys = self.support_trigger_range(w, id, def)?;
                let mut targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    def.combat.profile.selection,
                    w,
                    w.unit_capacity(),
                )?;
                targeting::sort_enemies(w, self, id, &mut targets, def.combat.profile.priority)?;
                targets.truncate(q.count);
                for target in targets {
                    self.support_status(w, r, id, target, "sleep", q.duration, None)?;
                }
            }
            if let Some(p) = &cfg.protege {
                let target = self.support_top(w, id)?;
                w.unit_mut(id)?.operator.support.protege = target;
                if let Some(target) = target {
                    self.component_buff(
                        w,
                        r,
                        target,
                        "vendla:taunt",
                        [("taunt".into(), p.taunt)].into(),
                        s.spec.duration + 0.1,
                        id,
                    )?;
                    let mut b = Buff {
                        key: Some(format!("vendla:counter:{id}")),
                        ..Default::default()
                    };
                    b.runtime.duration = s.spec.duration + 0.1;
                    b.runtime.source = Some(id);
                    b.runtime.injury.observe = Some(PROTEGE_COUNTER);
                    r.add(
                        w,
                        self,
                        target,
                        buffs::Spec {
                            buff: b,
                            ..Default::default()
                        },
                    )?;
                }
            }
            if let Some(p) = &cfg.plasma {
                let mut targets = vec![id];
                let others: Vec<_> = self
                    .support_allies(w, id)?
                    .into_iter()
                    .filter(|&a| a != id)
                    .collect();
                if let Some(&other) = self.rng.pick(&others) {
                    targets.push(other);
                }
                for target in targets {
                    let mut b = Buff {
                        key: Some("bldsk:plasma".into()),
                        mods: [("atkPct".into(), p.attack)].into(),
                        ..Default::default()
                    };
                    b.runtime.duration = p.duration + 1e-6;
                    b.runtime.interval = p.interval;
                    b.runtime.on_tick = Some(PLASMA_DRAIN);
                    b.runtime.tick_payload = p.ratio;
                    b.runtime.source = Some(id);
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
            if let Some(zone) = &cfg.zone {
                if self.zones.len() >= w.unit_capacity() {
                    return Err("Lingering area capacity exceeded".into());
                }
                let keys = self
                    .roster
                    .as_ref()
                    .ok_or("Missing zone roster")?
                    .range(w, id)?;
                let mut targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    def.combat.profile.selection,
                    w,
                    w.unit_capacity(),
                )?;
                targeting::sort_enemies(w, self, id, &mut targets, def.combat.profile.priority)?;
                let position = if let Some(&target) = targets.first() {
                    w.unit(target)?.position
                } else {
                    let a = w.unit(id)?.position;
                    let f = dir.vector();
                    [a[0] + f64::from(f[0]), a[1] + f64::from(f[1])]
                };
                let damage = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("atk")
                    * zone.scale;
                let pulses = zone.duration.round().max(1.);
                if pulses > u32::MAX as f64 {
                    return Err("Lingering area pulse count exceeded".into());
                }
                let state = ZoneState {
                    source: id,
                    position,
                    damage,
                    left: pulses as u32,
                    next: w.time,
                };
                // An immediate timer first runs in the next scheduler phase.
                self.zones.push(state);
            }
            if let Some(push) = &cfg.push {
                self.support_push_cast(w, r, id, dir, push)?;
            }
            if !cfg.peaks.is_empty() {
                self.support_peaks(w, r, id, &cfg.peaks)?;
            }
            if cfg.deferred.is_some() {
                w.unit_mut(id)?.operator.support.debt = 0.;
            }
            if let Some(p) = &cfg.phalanx {
                self.support_phalanx(w, r, id, p, true)?;
            }
            w.unit_mut(id)?.operator.support.fragile_scale = 1.;
            for aura in cfg.enemy_auras.iter().filter(|a| a.active_only) {
                self.support_enemy_aura(w, r, id, aura)?;
            }
        }
        if event == skills::Event::Tick {
            if cfg.cook.is_some()
                && !w.unit(id)?.stats.buffs().iter().any(|b| {
                    b.key.as_deref() == Some("sunbr:cook")
                        || b.key.as_deref() == Some("sunbr:serve")
                })
            {
                self.component_buff(
                    w,
                    r,
                    id,
                    "sunbr:serve",
                    [("atkPct".into(), cfg.cook.as_ref().unwrap().attack)].into(),
                    f64::INFINITY,
                    id,
                )?;
            }
            if !cfg.peaks.is_empty() {
                self.support_peaks(w, r, id, &cfg.peaks)?;
            }
            for aura in cfg.enemy_auras.iter().filter(|a| a.active_only) {
                self.support_enemy_aura(w, r, id, aura)?;
            }
        }
        if event == skills::Event::End {
            if cfg.cook.is_some() {
                r.remove(w, self, id, "sunbr:cook")?;
                r.remove(w, self, id, "sunbr:serve")?;
            }
            if let Some(target) = w.unit_mut(id)?.operator.support.protege.take() {
                if w.unit(target)?.stats.buffs().iter().any(|b| {
                    b.key.as_deref() == Some("vendla:taunt") && b.runtime.source == Some(id)
                }) {
                    r.remove(w, self, target, "vendla:taunt")?;
                }
                r.remove(w, self, target, &format!("vendla:counter:{id}"))?;
            }
            if !cfg.peaks.is_empty() {
                r.remove(w, self, id, "humus:peak")?;
            }
            if let Some(p) = &cfg.phalanx {
                if c.reason != "death" && w.unit(id)?.stats.alive && p.burst > 0. {
                    let keys = self
                        .roster
                        .as_ref()
                        .ok_or("Missing phalanx roster")?
                        .range(w, id)?;
                    let mut targets = search::Host::enemies(
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
                    targeting::sort_enemies(
                        w,
                        self,
                        id,
                        &mut targets,
                        def.combat.profile.priority,
                    )?;
                    for target in targets {
                        let amount = w
                            .unit_mut(id)?
                            .stats
                            .stats()
                            .map_err(str::to_string)?
                            .n("atk")
                            * p.burst;
                        self.support_damage(w, r, id, target, amount, "burst")?;
                    }
                }
                self.support_phalanx(w, r, id, p, false)?;
            }
            if let Some(d) = &cfg.deferred {
                let debt = std::mem::take(&mut w.unit_mut(id)?.operator.support.debt);
                if c.reason != "death" && w.unit(id)?.stats.alive && debt > 0. {
                    let mut b = Buff {
                        key: Some("gvial:bleed".into()),
                        ..Default::default()
                    };
                    b.runtime.duration = d.duration + 1e-6;
                    b.runtime.interval = d.interval;
                    b.runtime.on_tick = Some(HP_DRAIN);
                    b.runtime.tick_payload = debt * d.interval / d.duration;
                    b.runtime.refresh = buffs::Refresh::Independent;
                    b.runtime.max_stacks = 10;
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
            }
            w.unit_mut(id)?.operator.support.fragile_scale = 1.;
            for aura in cfg.enemy_auras.iter().filter(|a| a.active_only) {
                self.support_enemy_aura(w, r, id, aura)?;
            }
        }
        Ok(())
    }
    fn support_zone_pulse(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        z: &mut ZoneState,
    ) -> Result<(), String> {
        let ids = self
            .spawns
            .as_ref()
            .ok_or("Missing zone targets")?
            .live()
            .to_vec();
        let targets = spatial::enemies_in_radius(
            w,
            self,
            &ids,
            z.position,
            0.9,
            spatial::RadiusRule::Body,
            true,
            w.unit_capacity(),
        )?;
        for target in targets {
            self.support_status(w, r, z.source, target, "sluggish", 1.05, None)?;
            self.support_status(w, r, z.source, target, "silence", 1.05, None)?;
            self.support_damage(w, r, z.source, target, z.damage, "zone")?;
        }
        z.left -= 1;
        z.next += 1.;
        Ok(())
    }
    fn support_push_cast(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        dir: Direction,
        p: &PushCast,
    ) -> Result<(), String> {
        let keys = targeting::absolute_range(&p.grid, w.unit(id)?.deployment.tile, dir, 0);
        let all = self
            .spawns
            .as_ref()
            .ok_or("Missing push targets")?
            .live()
            .to_vec();
        let mut targets = Vec::new();
        for &target in &all {
            let u = w.unit_mut(target)?;
            if u.stats.alive
                && !u.hidden
                && !u
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("untargetable")
                && body::in_range(u, &keys)
            {
                targets.push(target);
            }
        }
        let mut pushed: std::collections::BTreeSet<_> = targets.iter().copied().collect();
        for &target in &targets {
            let a = w.unit(id)?.position;
            let b = w.unit(target)?.position;
            let near = hypot(b[0] - a[0], b[1] - a[1]) < 0.25;
            let expect = if displacement::displaceable(w, target)? {
                let level =
                    displacement::force_level(w, target, p.force - if near { 2. } else { 0. })?;
                displacement::push_tiles(level, true)
            } else {
                0.
            };
            let options = displacement::Push {
                from: Some(displacement::Origin::unit(w, id, dir)?),
                direction: Some(dir.vector().map(f64::from)),
                fixed_angle: true,
                effect: true,
                ..Default::default()
            };
            let grid = self.grid.take().ok_or("Missing push grid")?;
            let result = displacement::push(
                w,
                &grid,
                &mut allies::Blocking {
                    host: self,
                    buffs: r,
                },
                target,
                p.force,
                options,
            );
            self.grid = Some(grid);
            let moved = result?;
            self.support_status(
                w,
                r,
                id,
                target,
                "stun",
                if expect > 0. && moved + 0.05 < expect {
                    p.wall
                } else {
                    p.direct
                },
                None,
            )?;
        }
        for target in targets {
            let others = spatial::enemies_in_radius(
                w,
                self,
                &all,
                w.unit(target)?.position,
                0.6,
                spatial::RadiusRule::Body,
                false,
                w.unit_capacity(),
            )?;
            for other in others {
                if pushed.insert(other) {
                    self.support_status(w, r, id, other, "stun", p.collision, None)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn support_counter(c: &mut settlement::Context<'_, Self>) -> Result<(), String> {
        let buff = c.runtime.get(c.world, c.request.hit.target, c.buff)?;
        let source = buff.runtime.source.ok_or("Missing protege source")?;
        let target = c.request.hit.target;
        let Some(enemy) = c.request.hit.source.filter(|&e| {
            c.world
                .unit(e)
                .is_ok_and(|u| u.side == health::Side::Enemy && u.stats.alive)
        }) else {
            return Ok(());
        };
        let u = c.world.unit(source)?;
        if !u.stats.alive
            || !u.deployed
            || !u.operator.active_skill
            || u.operator.support.protege != Some(target)
        {
            return Ok(());
        }
        let def = Arc::clone(
            &c.host
                .roster
                .as_ref()
                .ok_or("Missing protege roster")?
                .entry(source)?
                .definition,
        );
        let scale = def
            .extra
            .support
            .protege
            .as_ref()
            .ok_or("Missing protege configuration")?
            .scale;
        let amount = c
            .world
            .unit_mut(source)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("atk")
            * scale;
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(source),
                credit: Some(source),
                target: enemy,
                damage_type: DamageType::Arts,
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
        q.calculation = hit::Calculation::Precomputed;
        q.is_skill = true;
        q.ignore_select = true;
        q.tags.push("counter".into());
        q.trait_ally = Some(target);
        c.resolve(&mut q)?;
        Ok(())
    }
    pub(super) fn support_scheduled(&mut self, w: &mut World) -> Result<(), String> {
        let roster = self
            .roster
            .as_ref()
            .ok_or("Missing support scheduler roster")?;
        let mut r = self.buffs.take().ok_or("Missing support scheduler buffs")?;
        let mut ids = std::mem::take(&mut self.scratch);
        ids.clear();
        ids.extend_from_slice(roster.ids());
        let result = (|| {
            for &id in &ids {
                let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
                let cfg = &def.extra.support;
                let up = w.unit(id)?.stats.alive && w.unit(id)?.deployed;
                let now = w.time;
                if cfg.lavender > 0. {
                    let next = &mut w.unit_mut(id)?.operator.support.next_lavender;
                    if *next == 0. {
                        *next = 0.25;
                    }
                    if now + 1e-9 >= *next {
                        *next += 0.25;
                        if up {
                            let value = w
                                .unit_mut(id)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("atk")
                                * cfg.lavender;
                            for &target in &ids {
                                if targeting::friendly_selectable(w, id, target)? {
                                    self.component_buff(
                                        w,
                                        &mut r,
                                        target,
                                        &format!("flower:lavender:{id}"),
                                        [("hpRegen".into(), value)].into(),
                                        0.5,
                                        id,
                                    )?;
                                }
                            }
                        }
                    }
                }
                if cfg.sp_aura > 0. || cfg.reveal {
                    let period = cfg.sp_period.unwrap_or(0.2);
                    let next = &mut w.unit_mut(id)?.operator.support.next_sp_aura;
                    if *next == 0. {
                        *next = period;
                    }
                    if now + 1e-9 >= *next {
                        *next += period;
                        if up {
                            if cfg.sp_aura > 0. {
                                for &target in &ids {
                                    if cfg.sp_profession.is_some_and(|p| {
                                        self.roster
                                            .as_ref()
                                            .unwrap()
                                            .entry(target)
                                            .is_ok_and(|e| e.definition.profession != p)
                                    }) || !targeting::friendly_selectable(w, id, target)?
                                    {
                                        continue;
                                    }
                                    let stronger = w.unit(target)?.stats.buffs().iter().any(|b| {
                                        b.key.as_deref() == Some("aura:spRecovery")
                                            && b.runtime.source != Some(id)
                                            && b.runtime.source.is_some_and(|s| {
                                                w.unit(s).is_ok_and(|u| u.stats.alive)
                                            })
                                            && b.mods.get("spRecoveryFlat").copied().unwrap_or(0.)
                                                > cfg.sp_aura
                                    });
                                    if !stronger {
                                        self.component_buff(
                                            w,
                                            &mut r,
                                            target,
                                            "aura:spRecovery",
                                            [("spRecoveryFlat".into(), cfg.sp_aura)].into(),
                                            cfg.sp_duration.unwrap_or(0.25),
                                            id,
                                        )?;
                                    }
                                }
                            }
                            if cfg.reveal {
                                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                                let enemies = self.spawns.as_ref().unwrap().live().to_vec();
                                for target in enemies {
                                    let u = w.unit_mut(target)?;
                                    if u.stats.alive
                                        && !u.hidden
                                        && body::in_range(u, &keys)
                                        && u.stats
                                            .stats()
                                            .map_err(str::to_string)?
                                            .flags
                                            .contains("stealth")
                                    {
                                        let mut b = Buff {
                                            key: Some("aura:reveal".into()),
                                            flags: ["reveal".into()].into(),
                                            ..Default::default()
                                        };
                                        b.runtime.duration = 0.25;
                                        r.add(
                                            w,
                                            self,
                                            target,
                                            buffs::Spec {
                                                buff: b,
                                                flags_present: true,
                                                ..Default::default()
                                            },
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                }
                if cfg.support_atk != 0. {
                    let next = &mut w.unit_mut(id)?.operator.support.next_support_aura;
                    if now + 1e-9 >= *next {
                        *next += 0.5;
                        if up {
                            for &target in &ids {
                                if self
                                    .roster
                                    .as_ref()
                                    .unwrap()
                                    .entry(target)?
                                    .definition
                                    .profession
                                    != "SUPPORT"
                                    || !targeting::friendly_selectable(w, id, target)?
                                {
                                    continue;
                                }
                                let stronger = w.unit(target)?.stats.buffs().iter().any(|b| {
                                    b.key.as_deref() == Some("talent:podego")
                                        && b.runtime.source != Some(id)
                                        && b.mods.get("atkPct").copied().unwrap_or(0.)
                                            > cfg.support_atk
                                        && b.runtime.time_left > 0.05
                                });
                                if !stronger {
                                    self.component_buff(
                                        w,
                                        &mut r,
                                        target,
                                        "talent:podego",
                                        [("atkPct".into(), cfg.support_atk)].into(),
                                        0.6,
                                        id,
                                    )?;
                                }
                            }
                        }
                    }
                }
                if w.unit(id)?.operator.support.next_aura.len() != cfg.enemy_auras.len() {
                    w.unit_mut(id)?.operator.support.next_aura = vec![0.; cfg.enemy_auras.len()];
                }
                for (index, aura) in cfg
                    .enemy_auras
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| !a.active_only)
                {
                    let next = &mut w.unit_mut(id)?.operator.support.next_aura[index];
                    if now + 1e-9 >= *next {
                        *next += aura.period;
                        self.support_enemy_aura(w, &mut r, id, aura)?;
                    }
                }
                if cfg.reaper > 0. {
                    if !up {
                        w.unit_mut(id)?.operator.support.reaper_queue.clear();
                    } else if now + 1e-9 >= w.unit(id)?.operator.support.next_reaper {
                        if let Some(amount) =
                            w.unit_mut(id)?.operator.support.reaper_queue.pop_front()
                        {
                            w.unit_mut(id)?.operator.support.next_reaper = now + 0.12;
                            self.support_heal(
                                w,
                                &mut r,
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
                    }
                }
                if let Some(p) = &cfg.phalanx {
                    if now + 1e-9 >= w.unit(id)?.operator.support.next_phalanx {
                        w.unit_mut(id)?.operator.support.next_phalanx += 0.2;
                        self.support_def_aura(
                            w,
                            &mut r,
                            id,
                            &p.aura,
                            !w.unit(id)?.operator.active_skill,
                        )?;
                    }
                }
                if let Some(values) = cfg.block_stats {
                    if now + 1e-9 >= w.unit(id)?.operator.support.next_block_stats.max(0.1) {
                        w.unit_mut(id)?.operator.support.next_block_stats =
                            w.unit(id)?.operator.support.next_block_stats.max(0.1) + 0.1;
                        if up {
                            let extra = w.unit(id)?.blocking().len().saturating_sub(1) as f64;
                            let mods: Numbers = [
                                ("atkPct".into(), values[0] + values[2] * extra),
                                ("defPct".into(), values[1] + values[3] * extra),
                            ]
                            .into();
                            if !w
                                .unit(id)?
                                .stats
                                .buffs()
                                .iter()
                                .any(|b| b.key.as_deref() == Some("gvial:t1") && b.mods == mods)
                            {
                                self.component_buff(
                                    w,
                                    &mut r,
                                    id,
                                    "gvial:t1",
                                    mods,
                                    f64::INFINITY,
                                    id,
                                )?;
                            }
                        }
                    }
                }
                if cfg.forge.is_some()
                    && now + 1e-9 >= w.unit(id)?.operator.support.next_forge.max(0.2)
                {
                    w.unit_mut(id)?.operator.support.next_forge =
                        w.unit(id)?.operator.support.next_forge.max(0.2) + 0.2;
                    if up && w.unit(id)?.operator.active_skill {
                        // The source's own passive modifiers are installed by SkillRuntime.
                        for &target in &ids {
                            if target != id
                                && targeting::friendly_selectable(w, id, target)?
                                && w.unit(target)?
                                    .stats
                                    .buffs()
                                    .iter()
                                    .any(|b| b.key.as_deref() == Some("cathy:shield"))
                            {
                                self.component_buff(
                                    w,
                                    &mut r,
                                    target,
                                    &format!("cathy:forge:{id}"),
                                    cfg.forge.as_ref().unwrap().clone(),
                                    0.25,
                                    id,
                                )?;
                            }
                        }
                    }
                }
            }
            let mut zones = std::mem::take(&mut self.zones);
            let pulses = (|| {
                for zone in &mut zones {
                    if zone.left > 0 && w.time + 1e-9 >= zone.next {
                        self.support_zone_pulse(w, &mut r, zone)?;
                    }
                }
                Ok(())
            })();
            zones.retain(|z| z.left > 0);
            self.zones = zones;
            pulses
        })();
        self.scratch = ids;
        self.buffs = Some(r);
        result
    }
    pub(super) fn refresh_support_casts(&mut self, w: &mut World) -> Result<(), String> {
        let ids = self
            .roster
            .as_ref()
            .ok_or("Missing support tick roster")?
            .ids()
            .to_vec();
        for id in ids {
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
            if !def.extra.support.auto_heal || !skills::can_act(w, id)? {
                continue;
            }
            let entry = self.roster.as_ref().unwrap().entry(id)?;
            let s = entry.skill()?;
            if !s.bank.ready()
                || s.bank.active
                || s.op_cooling(w.time)
                || w.unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("silence")
            {
                continue;
            }
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            if search::Host::injured_allies(self, id, &keys, false, w, w.unit_capacity())?
                .is_empty()
            {
                continue;
            }
            let mut skill = self.roster.as_mut().unwrap().take_skill(id)?;
            let result = skill.activate(w, self, "DEFAULT", false);
            self.roster.as_mut().unwrap().restore_skill(id, skill)?;
            result?;
        }
        Ok(())
    }
    pub(super) fn support_before_attack(
        &mut self,
        w: &mut World,
        id: Id,
        targets: &[Id],
        skill: Option<&mut skills::Runtime>,
    ) -> Result<(), String> {
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        if def.extra.support.bandage.is_none() {
            return Ok(());
        }
        let (Some(&target), Some(skill)) = (targets.first(), skill) else {
            return Ok(());
        };
        if w.unit(target)?.side != health::Side::Ally
            || !skill.bank.ready()
            || w.unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("silence")
        {
            return Ok(());
        }
        let u = w.unit_mut(target)?;
        let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        if max <= 0. || u.stats.hp / max >= 0.5 {
            return Ok(());
        }
        if skill.activate(w, self, "DEFAULT", false)? {
            w.unit_mut(id)?.operator.support.bandage = Some(target);
        }
        Ok(())
    }
    pub(super) fn support_attack(
        &mut self,
        c: &attacks::Context,
        w: &mut World,
    ) -> Result<(), String> {
        let id = c.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let cfg = &def.extra.support;
        let bandage = w.unit_mut(id)?.operator.support.bandage.take();
        if cfg.skill_heal <= 0. && bandage.is_none() {
            return Ok(());
        }
        let mut r = self.buffs.take().ok_or("Missing attack heal context")?;
        let result = (|| {
            if cfg.skill_heal > 0. && c.is_skill && w.unit(id)?.stats.alive {
                self.support_heal(
                    w,
                    &mut r,
                    Some(id),
                    id,
                    cfg.skill_heal,
                    HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
            if let Some(target) = bandage {
                if w.unit(target)?.stats.alive {
                    let amount = w
                        .unit_mut(target)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .n("maxHp")
                        * cfg.bandage.unwrap_or(0.);
                    self.support_heal(w, &mut r, Some(id), target, amount, HealOptions::default())?;
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn support_hit(
        &mut self,
        impact: attacks::Impact,
        w: &mut World,
    ) -> Result<(), String> {
        if impact.heal {
            return Ok(());
        }
        let id = impact.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let cfg = &def.extra.support;
        let mut r = self.buffs.take().ok_or("Missing support hit buffs")?;
        let result = (|| {
            if cfg.skill_drain_heal > 0.
                && w.unit(id)?.operator.active_skill
                && impact.amount > 0.
                && w.unit(id)?.stats.alive
            {
                self.support_heal(
                    w,
                    &mut r,
                    Some(id),
                    id,
                    impact.amount * cfg.skill_drain_heal,
                    HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
            let Some(target) = impact.target.filter(|&t| {
                w.unit(t)
                    .is_ok_and(|u| u.stats.alive && u.side == health::Side::Enemy)
            }) else {
                return Ok(());
            };
            if let Some([duration, bonus]) = cfg.sluggish {
                let key = &self.spawns.as_ref().unwrap().definition(target)?.key;
                let beast = self.data.table("enemies").unwrap()[key]["tags"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|t| t == "infection"));
                self.support_status(
                    w,
                    &mut r,
                    id,
                    target,
                    "sluggish",
                    duration + if beast { bonus } else { 0. },
                    None,
                )?;
            }
            if let Some([value, duration]) = cfg.weaken {
                self.support_status(w, &mut r, id, target, "weaken", duration, Some(value))?;
            }
            if let Some(force) = cfg.radial_push.filter(|_| impact.is_skill) {
                let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
                let grid = self.grid.take().ok_or("Missing radial push grid")?;
                let result = displacement::push(
                    w,
                    &grid,
                    &mut allies::Blocking {
                        host: self,
                        buffs: &mut r,
                    },
                    target,
                    force,
                    displacement::Push {
                        from: Some(displacement::Origin::unit(w, id, dir)?),
                        ..Default::default()
                    },
                );
                self.grid = Some(grid);
                result?;
            }
            if let Some(force) = cfg
                .inward
                .filter(|_| w.unit(id).is_ok_and(|u| u.operator.active_skill))
            {
                let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
                let options = displacement::Push {
                    from: Some(displacement::Origin::unit(w, id, dir)?),
                    inward: true,
                    ..Default::default()
                };
                let grid = self.grid.take().ok_or("Missing inward push grid")?;
                let result = displacement::push(
                    w,
                    &grid,
                    &mut allies::Blocking {
                        host: self,
                        buffs: &mut r,
                    },
                    target,
                    force,
                    options,
                );
                self.grid = Some(grid);
                result?;
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn support_phase(
        &mut self,
        p: hit::Phase,
        q: &mut hit::Request,
        amount: f64,
        r: &mut buffs::Runtime,
        w: &mut World,
        _rng: &mut Rng,
    ) -> Result<(), String> {
        if p == hit::Phase::Hit
            && w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = Arc::clone(
                &self
                    .roster
                    .as_ref()
                    .ok_or("Missing incoming support roster")?
                    .entry(q.hit.target)?
                    .definition,
            );
            if let Some(d) = def
                .extra
                .support
                .deferred
                .as_ref()
                .filter(|_| w.unit(q.hit.target).is_ok_and(|u| u.operator.active_skill))
            {
                q.damage.mul *= 1. - d.resistance;
                q.tags.push("gvialDeferred".into());
            }
        }
        let Some(source) = q
            .hit
            .source
            .filter(|&s| w.unit(s).is_ok_and(|u| u.has_operator_kit()))
        else {
            return Ok(());
        };
        let def = Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing outgoing support roster")?
                .entry(source)?
                .definition,
        );
        let cfg = &def.extra.support;
        if p == hit::Phase::Hit && w.unit(q.hit.target)?.side == health::Side::Enemy {
            if let Some([weight, penetration]) = cfg.heavy_penetration {
                if w.unit_mut(q.hit.target)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("massLevel")
                    >= weight
                {
                    q.damage.penetration.def_flat += penetration;
                }
            }
        }
        if p == hit::Phase::Calculate
            && (w.unit(q.hit.target)?.boss || w.unit(q.hit.target)?.targeting.elite)
        {
            if let Some(scale) = cfg.elite_scale {
                q.damage.amount *= scale;
            }
        }
        if p == hit::Phase::Output
            && amount > 0.
            && cfg.reaper > 0.
            && def
                .extra
                .tactics
                .behavior()
                .is_none_or(|kit| kit.reaper_eligible(q))
            && w.unit(source)?.stats.alive
            && w.unit(source)?.deployed
            && w.unit(q.hit.target)?.side == health::Side::Enemy
            && !q.hit.hp_loss
            && q.hit.damage_type != DamageType::Elemental
        {
            let now = w.time;
            let limit = w
                .unit_mut(source)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("blockCnt")
                .floor()
                .max(0.) as usize;
            let max_pending = w.unit_capacity();
            let heal = cfg.reaper
                * self
                    .tac_def(source)?
                    .extra
                    .tactics
                    .behavior()
                    .map(|kit| kit.reaper_multiplier(w, source))
                    .transpose()?
                    .unwrap_or(1.);
            let m = &mut w.unit_mut(source)?.operator.support;
            if now + 1e-9 >= m.reaper_window {
                m.reaper_window = now + 0.05;
                m.reaper_accepted = 0;
                m.reaper_limit = limit;
            }
            if m.reaper_accepted < m.reaper_limit {
                if m.reaper_queue.len() >= max_pending {
                    return Err("Reaper heal queue capacity exceeded".into());
                }
                m.reaper_accepted += 1;
                m.reaper_queue.push_back(heal);
            }
        }
        let _ = r;
        Ok(())
    }
    pub(super) fn support_landed(
        &mut self,
        e: &health::Event,
        q: &hit::Request,
        _r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if q.tags.iter().any(|t| t == "gvialDeferred")
            && e.amount > 0.
            && w.unit(e.target)?.side == health::Side::Ally
            && w.unit(e.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = Arc::clone(
                &self
                    .roster
                    .as_ref()
                    .ok_or("Missing deferred damage roster")?
                    .entry(e.target)?
                    .definition,
            );
            if let Some(d) = &def.extra.support.deferred {
                w.unit_mut(e.target)?.operator.support.debt +=
                    e.amount * d.resistance / (1. - d.resistance);
            }
        }
        Ok(())
    }
    pub(super) fn support_damage_applied(
        &mut self,
        e: &health::Event,
        q: &hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let Some(source) = e.source.filter(|&s| {
            w.unit(s)
                .is_ok_and(|u| u.has_operator_kit() && u.stats.alive)
        }) else {
            return Ok(());
        };
        if w.unit(e.target)?.side != health::Side::Enemy {
            return Ok(());
        }
        let def = Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing incantation roster")?
                .entry(source)?
                .definition,
        );
        if def.extra.support.incantation <= 0. {
            return Ok(());
        }
        let target = if let Some(target) = q.trait_ally {
            Some(target)
        } else {
            let keys = self.roster.as_ref().unwrap().range(w, source)?;
            search::Host::injured_allies(self, source, &keys, false, w, w.unit_capacity())?
                .first()
                .copied()
        };
        if let Some(target) = target {
            self.support_heal(
                w,
                r,
                Some(source),
                target,
                e.amount * def.extra.support.incantation,
                HealOptions::default(),
            )?;
        }
        Ok(())
    }
    pub(super) fn support_enemy_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        victim: Id,
    ) -> Result<(), String> {
        let ids = self
            .roster
            .as_ref()
            .ok_or("Missing death support roster")?
            .ids()
            .to_vec();
        for &id in &ids {
            if !w.unit(id)?.stats.alive || !w.unit(id)?.deployed {
                continue;
            }
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
            let cfg = &def.extra.support;
            if cfg.death_sp.is_none() && !cfg.enemy_auras.iter().any(|a| a.growth.is_some()) {
                continue;
            }
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            if !body::in_range(w.unit(victim)?, &keys) {
                continue;
            }
            if w.unit(id)?.operator.active_skill
                && !w.unit(victim)?.boss
                && !w.unit(victim)?.targeting.elite
            {
                for a in &cfg.enemy_auras {
                    if let Some([step, cap]) = a.growth {
                        let m = &mut w.unit_mut(id)?.operator.support;
                        m.fragile_scale = (m.fragile_scale + step).min(cap);
                    }
                }
            }
            if let Some([own, other]) = cfg.death_sp {
                let mut roster = self.roster.take().unwrap();
                let result = roster.grant_sp(w, r, self, id, own);
                self.roster = Some(roster);
                result?;
                let mut candidates = vec![];
                for target in self.support_allies(w, id)? {
                    let entry = self.roster.as_ref().unwrap().entry(target)?;
                    if target != id
                        && entry.talent_sp_eligible(w.unit(target)?.operator.active_skill)
                    {
                        candidates.push(target);
                    }
                }
                if let Some(&target) = self.rng.pick(&candidates) {
                    if self.roster.as_ref().unwrap().entry(target)?.skill().is_ok() {
                        let mut roster = self.roster.take().unwrap();
                        let result = roster.grant_sp(w, r, self, target, other);
                        self.roster = Some(roster);
                        result?;
                    } else {
                        // The attacker owns its SkillRuntime until this callback returns.
                        let limit = w
                            .unit_capacity()
                            .checked_mul(ids.len())
                            .ok_or("SP grant capacity overflow")?;
                        let pending = &mut w.unit_mut(target)?.operator.support.sp_changes;
                        if pending.len() >= limit {
                            return Err("Pending SP grant capacity exceeded".into());
                        }
                        pending.push(crate::battle::content::operators::SpChange::Gain(other));
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn support_flush_sp(
        &mut self,
        w: &mut World,
        skill: &mut skills::Runtime,
    ) -> Result<(), String> {
        let changes = std::mem::take(&mut w.unit_mut(skill.unit())?.operator.support.sp_changes);
        for change in changes {
            match change {
                crate::battle::content::operators::SpChange::Damaged => {
                    skill.on_damaged(w, self)?
                }
                crate::battle::content::operators::SpChange::Gain(amount) => {
                    skill.gain(w, self, amount, "talent", false, false)?;
                }
                crate::battle::content::operators::SpChange::Bond(amount) => {
                    skill.gain(w, self, amount, "bond", false, false)?;
                }
                crate::battle::content::operators::SpChange::Clear => {
                    skill.bank.clear_inactive(w)?
                }
            }
        }
        Ok(())
    }
    pub(super) fn support_sp_gain(
        &mut self,
        w: &mut World,
        id: Id,
        c: &mut skills::Context,
    ) -> Result<(), String> {
        if c.reason != "time" {
            return Ok(());
        }
        let def = Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing SP modifier roster")?
                .entry(id)?
                .definition,
        );
        if def.extra.support.sp_near <= 0. {
            return Ok(());
        }
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
            c.amount += def.extra.support.sp_near * DT;
        }
        Ok(())
    }
}
