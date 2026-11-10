// SPDX-License-Identifier: GPL-3.0-or-later
//! Battle host adapters for damage settlement, status and elemental effects.
use super::*;

impl Host {
    pub(super) fn resolve_hit(
        &mut self,
        world: &mut World,
        buffs: &mut buffs::Runtime,
        request: &mut hit::Request,
    ) -> Result<f64, String> {
        let mut rng = self.rng.clone();
        let result = settlement::resolve(world, buffs, &mut rng, self, request);
        // Publish draws consumed before an error as well as successful draws.
        self.rng = rng;
        result
    }
}

impl settlement::Hooks for Host {
    fn damage_cleanup(
        &mut self,
        token: u64,
        target: Id,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        r.remove_id(w, self, target, token)?;
        Ok(())
    }

    fn injury(
        token: u64,
        e: &mut super::super::injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if token == super::super::equipment::FIXED_REDUCTION {
            e.amount = (e.amount
                - c.runtime
                    .get(c.world, c.request.hit.target, c.buff)?
                    .runtime
                    .tick_payload)
                .max(0.);
            return Ok(());
        }
        if token == super::super::content::enemies::bosses::gun::SPRING_SHIELD {
            return Self::spring_injury(e, c);
        }
        if token == enemies::ARTS_BARRIER {
            return Self::enemy_barrier_injury(e, c);
        }
        if token == kits::excu2::DODGE {
            return Self::excu2_injury(c, e);
        }
        if token == kits::tippi::EVADE {
            return Self::tippi_injury(e, c);
        }
        if token == tactics::WITHER {
            return Self::tactical_wither(e, c);
        }
        if token == advanced::FLAME_EVADE {
            return Self::advanced_evade(e, c);
        }
        if token != super::super::content::operators::ARMOR_BLOCK {
            return Err("Unknown operator injury callback".into());
        }
        let probability = c
            .runtime
            .get(c.world, c.request.hit.target, c.buff)?
            .runtime
            .tick_payload;
        c.block(e, Some(probability))?;
        Ok(())
    }
    fn observe(
        token: u64,
        event: super::super::injury::Observation,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if matches!(
            token,
            super::super::content::enemies::bosses::gun::SPRING_SHIELD
                | super::super::content::enemies::bosses::gun::SPRING_SHARE
        ) {
            return Self::spring_observe(token, event, c);
        }
        if token == kits::tippi::OBSERVER {
            return Self::tippi_observe(c);
        }
        if token == tactics::OBSERVE {
            return Self::kit_observe(c);
        }
        if token == super::super::content::operators::PROTEGE_COUNTER {
            Self::support_counter(c)
        } else {
            Self::component_observe(c, token)
        }
    }

    fn damage_notice(
        &mut self,
        notice: super::super::injury::Notice,
        _expected: f64,
        q: &mut hit::Request,
        _r: &mut buffs::Runtime,
        w: &mut World,
        _rng: &mut Rng,
    ) -> Result<(), String> {
        self.advanced_notice(notice, q, w)
    }
    fn phase_enabled(&self, p: hit::Phase) -> bool {
        matches!(
            p,
            hit::Phase::Hit | hit::Phase::Calculate | hit::Phase::Output | hit::Phase::Applying
        ) || p == hit::Phase::BeforeApply && self.equipment.pegasus_damage
            || p == hit::Phase::HpDamage && self.bonds_has_share()
    }
    fn damage_phase(
        &mut self,
        p: hit::Phase,
        q: &mut hit::Request,
        amount: &mut f64,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if p == hit::Phase::Hit {
            self.bonds_hit(w, q)?;
            self.growth_hit(w, q)?;
        }
        if p == hit::Phase::HpDamage {
            self.rng = rng.clone();
            let result = self.bond_share(w, r, q, amount);
            *rng = self.rng.clone();
            return result;
        }
        if p == hit::Phase::BeforeApply {
            self.rng = rng.clone();
            let result = self.equipment_before_apply(w, r, q);
            *rng = self.rng.clone();
            return result;
        }
        if p == hit::Phase::Hit {
            // Profession attack scales form the input to weakness conversion.
            if q.is_attack && w.unit(q.hit.target)?.side == health::Side::Enemy {
                if let Some(id) = q.hit.source {
                    if w.unit(id)?.has_operator_kit() {
                        let entry = self.roster.as_ref().unwrap().entry(id)?;
                        if matches!(
                            entry.definition.hit,
                            super::super::content::operators::HitHandler::Blocked { .. }
                        ) {
                            entry.definition.hit.apply(w, id, entry.direction, q, rng)?;
                        }
                    }
                }
            }
            self.equipment_retype(w, q)?;
            self.rng = rng.clone();
            self.enemy_content_hit(w, q, r)?;
            *rng = self.rng.clone();
        }
        self.advanced_phase(p, q, r, w)?;
        self.rng = rng.clone();
        self.kit_hit_phase(w, r, p, q)?;
        *rng = self.rng.clone();
        self.support_phase(p, q, *amount, r, w, rng)?;
        self.component_phase(p, q, r, w, rng)?;
        if p == hit::Phase::Hit {
            self.equipment_hit(w, q)?;
        }
        if p == hit::Phase::Hit
            && w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != super::super::deploy::Kind::Device
        {
            let def = Arc::clone(
                &self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(q.hit.target)?
                    .definition,
            );
            def.extra.protect(w, r, self, q.hit.target)?;
        }
        if let Some(source) = q
            .hit
            .source
            .filter(|&id| w.unit(id).is_ok_and(|u| u.has_operator_kit()))
        {
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
            if p == hit::Phase::Hit
                && q.is_attack
                && def.extra.skill_arts
                && q.hit.damage_type == super::super::damage::DamageType::Physical
                && w.unit(source)?
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some(format!("skill:{source}").as_str()))
            {
                q.hit.damage_type = super::super::damage::DamageType::Arts;
            }
            if p == hit::Phase::Hit
                && w.unit(q.hit.target)?.side == health::Side::Enemy
                && q.hit.damage_type != super::super::damage::DamageType::Elemental
                && !q.tags.iter().any(|t| t == "dot")
                && !def.extra.tag_scale.is_empty()
            {
                let key = &self.spawns.as_ref().unwrap().definition(q.hit.target)?.key;
                let tags = &self.data.table("enemies").unwrap()[key]["tags"];
                for (tag, scale) in &def.extra.tag_scale {
                    if tags.as_array().is_some_and(|a| a.iter().any(|x| x == tag)) {
                        q.damage.amount *= scale;
                    }
                }
            }
            if p == hit::Phase::Calculate
                && def.extra.weakness
                && w.unit(q.hit.target)?.side == health::Side::Enemy
                && matches!(
                    q.hit.damage_type,
                    super::super::damage::DamageType::Physical
                        | super::super::damage::DamageType::Arts
                )
            {
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
                let mut ign = q.damage.penetration;
                ign.def_flat += ss.n("defIgnoreFlat");
                ign.def_pct += ss.n("defIgnorePct");
                ign.res_flat += ss.n("resIgnoreFlat");
                ign.res_pct += ss.n("resIgnorePct");
                let ph = super::super::damage::mitigate(
                    super::super::damage::attack_input(
                        q.damage.amount,
                        super::super::damage::DamageType::Physical,
                        &ts,
                    ),
                    super::super::damage::DamageType::Physical,
                    &ts,
                    ign,
                    0.,
                );
                let ar = super::super::damage::mitigate(
                    super::super::damage::attack_input(
                        q.damage.amount,
                        super::super::damage::DamageType::Arts,
                        &ts,
                    ),
                    super::super::damage::DamageType::Arts,
                    &ts,
                    ign,
                    0.,
                );
                if ar > ph + 1e-9 {
                    q.hit.damage_type = super::super::damage::DamageType::Arts;
                } else if ph > ar + 1e-9 {
                    q.hit.damage_type = super::super::damage::DamageType::Physical;
                }
            }
            if p == hit::Phase::Calculate
                && w.unit(q.hit.target)?.stats.alive
                && w.unit(q.hit.target)?.stats.hp > 0.
                && w.unit(q.hit.target)?.side == health::Side::Enemy
            {
                if let Some(lord) = def
                    .extra
                    .lord
                    .filter(|l| l.arts > 0. && !q.tags.iter().any(|t| t == "ayerBlade"))
                {
                    let value = w
                        .unit_mut(source)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .n("atk")
                        * lord.arts
                        * if q.is_skill || w.unit(source)?.operator.active_skill {
                            def.extra.effects.lord_skill_scale
                        } else {
                            1.
                        }
                        * if w.unit(q.hit.target)?.boss || w.unit(q.hit.target)?.targeting.elite {
                            def.extra.support.elite_scale.unwrap_or(1.)
                        } else {
                            1.
                        };
                    let mut child = hit::Request::new(
                        health::Hit {
                            source: Some(source),
                            credit: Some(source),
                            target: q.hit.target,
                            damage_type: super::super::damage::DamageType::Arts,
                            no_sp: false,
                            silent: false,
                            hp_loss: false,
                            on_applied: false,
                        },
                        super::super::damage::Damage {
                            amount: value,
                            mul: 1.,
                            resolved: false,
                            penetration: Default::default(),
                        },
                    );
                    child.calculation = hit::Calculation::Precomputed;
                    child.attack_type = hit::AttackType::Addition;
                    child.tags = vec!["module".into(), "addition".into()];
                    settlement::resolve(w, r, rng, self, &mut child)?;
                }
            }
            if p == hit::Phase::Output
                && *amount > 0.
                && def.extra.self_heal > 0.
                && w.unit(source)?.stats.alive
                && w.unit(q.hit.target)?.side == health::Side::Enemy
                && !q.hit.hp_loss
                && q.hit.damage_type != super::super::damage::DamageType::Elemental
            {
                health::heal(
                    w,
                    self,
                    Some(source),
                    source,
                    def.extra.self_heal,
                    health::HealOptions {
                        self_heal: true,
                        ignore_heal_free: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        if p == hit::Phase::Hit
            && w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != super::super::deploy::Kind::Device
        {
            let effect = self
                .roster
                .as_ref()
                .ok_or("Missing defensive content context")?
                .entry(q.hit.target)?
                .definition
                .incoming_hit;
            if let Some(effect) = effect {
                effect.apply(w, q)?;
            }
        }
        if p == hit::Phase::Hit && q.is_attack && w.unit(q.hit.target)?.side == health::Side::Enemy
        {
            if let Some(id) = q.hit.source {
                if w.unit(id)?.has_operator_kit() {
                    let entry = self
                        .roster
                        .as_ref()
                        .ok_or("Missing operator hit context")?
                        .entry(id)?;
                    if !matches!(
                        entry.definition.hit,
                        super::super::content::operators::HitHandler::Blocked { .. }
                    ) {
                        entry.definition.hit.apply(w, id, entry.direction, q, rng)?;
                    }
                }
            }
        }
        if p == hit::Phase::Hit {
            self.enemy_chalice_share(w, r, q)?;
            self.gun_confession(w, r, q.hit.target)?;
            self.enemy_boss_in(w, q)?;
            self.enemy_reaction_in(w, q)?;
            self.enemy_terrain_in(q);
            self.enemy_special_in(w, r, q)?;
        }
        Ok(())
    }
    fn damage_kill(
        &mut self,
        target: Id,
        credit: Option<Id>,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        self.rng = rng.clone();
        let result = self.health_kill(target, credit, r, w);
        *rng = self.rng.clone();
        result
    }
    fn health_kill(
        &mut self,
        id: Id,
        credit: Option<Id>,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if w.unit(id)?.deployment.kind == super::super::deploy::Kind::Device {
            return self.kill_device(w, r, id, credit);
        }
        if w.unit(id)?.side == health::Side::Enemy {
            return self.kill_enemy(id, credit, r, w);
        }
        let mut roster = self.roster.take().ok_or("Missing operator death context")?;
        let result = roster.kill(w, r, self, id, credit);
        self.roster = Some(roster);
        result?;
        self.bonds_aura(w, r)?;
        self.enemy_other_death(w, r, id)?;
        self.kit_event(w, r, &mut kits::dispatch::Event::Death { unit: id, credit })?;
        self.flush_kit_events(w, r)
    }
    fn health_event(
        &mut self,
        e: &mut health::Event,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if w.unit(e.target)?.deployment.kind == super::super::deploy::Kind::Device {
            return Ok(());
        }
        self.component_fatal(e, r, w)?;
        self.advanced_fatal(e, r, w)?;
        if e.stage == health::Stage::ModifierApplied {
            self.spring_modifier(w, r, e.target, e.source, e.credit, e.amount)?;
        }
        if e.stage == health::Stage::Fatal
            && w.unit(e.target)?.side == health::Side::Ally
            && w.unit(e.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = &self
                .roster
                .as_ref()
                .ok_or("Missing fatal content context")?
                .entry(e.target)?
                .definition;
            if def.undying
                && w.unit(e.target)?
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some(format!("skill:{}", e.target).as_str()))
            {
                e.prevented = true;
            }
        }
        if e.stage == health::Stage::Fatal {
            self.kit_unit_event(
                w,
                r,
                e.target,
                kits::dispatch::Role::Target,
                &mut kits::dispatch::Event::Fatal(e),
            )?;
            self.kit_event(w, r, &mut kits::dispatch::Event::Fatal(e))?;
        }
        if e.stage == health::Stage::SkillDamaged {
            let mut roster = self
                .roster
                .take()
                .ok_or("Missing operator damage context")?;
            let result = roster.damaged(w, r, self, e.target);
            self.roster = Some(roster);
            result?;
            self.advanced_flush_starts(w, r)?;
        }
        Ok(())
    }
    fn damage_health_event(
        &mut self,
        e: &mut health::Event,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if e.stage == health::Stage::OnApplied {
            self.spring_counter_applied(w, r, q)?;
        }
        if e.stage == health::Stage::DamageApplied {
            self.support_damage_applied(e, q, r, w)?;
            self.kit_damage_applied(w, r, e, q)?;
        }
        if e.stage == health::Stage::Damaged {
            self.rng = rng.clone();
            self.enemy_content_taken(w, r, q, e.amount)?;
            self.enemy_content_dealt(w, r, q)?;
            *rng = self.rng.clone();
            let result = self.advanced_landed(e, q, r, w);
            *rng = self.rng.clone();
            result?;
            self.support_landed(e, q, r, w)?;
            self.rng = rng.clone();
            let result = self.kit_landed(w, r, e, q);
            *rng = self.rng.clone();
            result?;
            self.component_landed(e, q, r, w)?;
            self.rng = rng.clone();
            let result = self.bonds_landed(w, r, q);
            *rng = self.rng.clone();
            result?;
            self.rng = rng.clone();
            let result = self.equipment_landed(w, r, e, q);
            *rng = self.rng.clone();
            result?;
            self.choices_damaged(w, r, e, q)?;
            if e.amount > 0.
                && !q.hit.no_sp
                && q.hit.damage_type != super::super::damage::DamageType::Elemental
                && w.unit(e.target)?.side == health::Side::Ally
                && w.unit(e.target)?.deployment.kind != crate::battle::deploy::Kind::Device
                && w.unit(e.target)?.stats.alive
                && w.unit(e.target)?.deployed
            {
                let def = Arc::clone(&self.roster.as_ref().unwrap().entry(e.target)?.definition);
                if let Some(talent) = &def.extra.sp_hurt {
                    let source = e.target;
                    let entry = self.roster.as_ref().unwrap().entry(source)?;
                    let keys = super::super::targeting::absolute_range(
                        &talent.grid,
                        w.unit(source)?.deployment.tile,
                        entry.direction,
                        0,
                    );
                    let ids = self.roster.as_ref().unwrap().ids().to_vec();
                    let mut mates = Vec::new();
                    for id in ids {
                        if id != source
                            && super::super::targeting::aura_selectable(w, id)?
                            && super::super::body::in_range(w.unit(id)?, &keys)
                            && !self
                                .roster
                                .as_ref()
                                .unwrap()
                                .entry(id)?
                                .skill()?
                                .bank
                                .no_skill
                        {
                            mates.push(id);
                        }
                    }
                    let mut roster = self.roster.take().unwrap();
                    let result = roster.grant_sp(w, r, self, source, talent.amount);
                    self.roster = Some(roster);
                    result?;
                    if let Some(&mate) = rng.pick(&mates) {
                        let mut roster = self.roster.take().unwrap();
                        let result = roster.grant_sp(w, r, self, mate, talent.amount);
                        self.roster = Some(roster);
                        result?;
                    }
                }
            }
            if q.is_attack
                && w.unit(e.target)?.side == health::Side::Enemy
                && w.unit(e.target)?.stats.alive
            {
                if let Some(source) = q
                    .hit
                    .source
                    .filter(|&s| w.unit(s).is_ok_and(|u| u.has_operator_kit()))
                {
                    if let Some(dot) = self
                        .roster
                        .as_ref()
                        .unwrap()
                        .entry(source)?
                        .definition
                        .extra
                        .dot
                        .filter(|d| d.duration > 0. && d.amount > 0.)
                    {
                        let key = &self.spawns.as_ref().unwrap().definition(e.target)?.key;
                        let sea = self.data.table("enemies").unwrap()[key]["tags"]
                            .as_array()
                            .is_some_and(|t| t.iter().any(|x| x == "seamonster"));
                        let mut b = super::super::stats::Buff {
                            key: Some(format!("udflow:dot:{source}")),
                            ..Default::default()
                        };
                        b.runtime.duration = dot.duration;
                        b.runtime.interval = dot.interval;
                        b.runtime.source = Some(source);
                        b.runtime.refresh = buffs::Refresh::Extend;
                        b.runtime.on_tick = Some(super::super::content::operators::DOT);
                        b.runtime.tick_payload = if sea { dot.sea } else { dot.amount };
                        r.add(
                            w,
                            self,
                            e.target,
                            buffs::Spec {
                                buff: b,
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }

            if e.hp_removed > 0.
                && w.unit(e.target)?.side == health::Side::Enemy
                && w.unit(e.target)?.stats.hp > 0.
                && q.hit.damage_type != super::super::damage::DamageType::Elemental
                && !q.tags.iter().any(|t| t == "burst")
            {
                if let Some(source) = q.hit.source {
                    if w.unit(source)?.has_operator_kit() {
                        let attach = self
                            .roster
                            .as_ref()
                            .ok_or("Missing elemental talent context")?
                            .entry(source)?
                            .definition
                            .element_attach;
                        if let Some(attach) = attach {
                            let ratio = if w.unit(e.target)?.element_leader {
                                attach.leader
                            } else {
                                attach.normal
                            };
                            let amount = w
                                .unit_mut(source)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("atk")
                                * ratio;
                            for element in [
                                super::super::element::Element::Neural,
                                super::super::element::Element::Burn,
                                super::super::element::Element::Apoptosis,
                            ] {
                                let mut effect = super::super::element::Request::new(
                                    Some(source),
                                    Some(source),
                                    e.target,
                                    element,
                                    amount,
                                );
                                effect.damage.tags = vec!["talent".into(), "pithst".into()];
                                self.rng = rng.clone();
                                let result = super::super::element::apply(w, self, r, &mut effect);
                                *rng = self.rng.clone();
                                result?;
                            }
                        }
                    }
                }
            }
            if let Some(mut effect) = self.content.as_ref().unwrap().damaged(w, q)? {
                self.rng = rng.clone();
                let result = super::super::element::apply(w, self, r, &mut effect);
                *rng = self.rng.clone();
                result?;
            }
        }
        self.rng = rng.clone();
        let result = self.health_event(e, r, w);
        *rng = self.rng.clone();
        result
    }
}

impl super::super::status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        true
    }
    fn before_status(
        &mut self,
        context: &mut super::super::status::Context,
        buffs: &mut buffs::Runtime,
        world: &mut World,
    ) -> Result<(), String> {
        self.enemy_special_before_status(context);
        self.enemy_caster_before_status(context);
        self.kit_event(
            world,
            buffs,
            &mut kits::dispatch::Event::BeforeStatus(context),
        )?;
        self.equipment_before_status(world, context)
    }
    fn has_status_applied(&self) -> bool {
        true
    }
    fn status_applied(
        &mut self,
        c: &super::super::status::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        self.growth_status(w, c)?;
        self.enemy_movement_status(w, r, c)?;
        self.kit_event(w, r, &mut kits::dispatch::Event::Status(c))?;
        self.kit_event(w, r, &mut kits::dispatch::Event::StatusFinished(c))?;
        self.advanced_status_applied(c, w)
    }
}

impl super::super::element::Hooks for Host {
    fn battle_rect(&self) -> Rect {
        self.rect
    }
    fn element_enabled(&self, phase: super::super::element::Phase) -> bool {
        matches!(
            phase,
            super::super::element::Phase::Hit
                | super::super::element::Phase::Burst
                | super::super::element::Phase::ModifierApplied
        )
    }
    fn element_event(
        &mut self,
        phase: super::super::element::Phase,
        request: &mut super::super::element::Request,
        amount: f64,
        _: f64,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if phase == super::super::element::Phase::ModifierApplied {
            return self.spring_modifier(
                w,
                r,
                request.damage.hit.target,
                request.damage.hit.source,
                request.damage.hit.credit,
                amount,
            );
        }
        if phase == super::super::element::Phase::Burst {
            self.spring_element_burst(w, r, request.damage.hit.target)?;
        }
        if phase == super::super::element::Phase::Hit
            && request.damage.element == Some(super::super::element::Element::Erosion)
            && w.unit(request.damage.hit.target)?.boss
        {
            request.damage.cancelled = true;
        }
        self.enemy_parasite_element(w, r, phase, request)?;
        self.kit_event(w, r, &mut kits::dispatch::Event::Element { phase, request })?;
        if phase != super::super::element::Phase::Hit {
            return Ok(());
        }
        self.kit_element(w, r, request)?;
        self.advanced_element(request, w)?;
        self.kit_event(w, r, &mut kits::dispatch::Event::ElementLate(request))
    }
    fn element_number(
        &mut self,
        _: Id,
        _: f64,
        _: super::super::element::Element,
    ) -> Result<(), String> {
        Ok(())
    }
    fn burst_visual(
        &mut self,
        _: Id,
        _: super::super::element::Element,
        _: &World,
    ) -> Result<(), String> {
        Ok(())
    }
}
