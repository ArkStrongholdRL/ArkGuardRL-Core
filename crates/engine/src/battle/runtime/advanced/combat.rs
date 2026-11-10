// SPDX-License-Identifier: GPL-3.0-or-later
//! Continuous-combat callbacks for legacy operator components.
use super::*;

impl Host {
    pub(in crate::battle::runtime) fn advanced_before(
        &mut self,
        c: &mut attacks::Context,
        w: &mut World,
    ) -> Result<(), String> {
        let id = c.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let active = w.unit(id)?.operator.active_skill;
        match &def.extra.advanced {
            Advanced::Harold(p) if p.severe && active => {
                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                let mut candidates =
                    search::Host::injured_allies(self, id, &keys, true, w, w.unit_capacity())?;
                let mut order = vec![];
                for a in candidates.drain(..) {
                    let load = w
                        .unit(a)?
                        .element_buildup
                        .iter()
                        .copied()
                        .fold(0., f64::max);
                    let ratio = w.unit(a)?.stats.hp / Self::advanced_stat(w, a, "maxHp")?;
                    order.push((a, load, ratio, w.unit(a)?.deployment.deploy_seq));
                }
                order.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then(a.2.total_cmp(&b.2))
                        .then(a.3.cmp(&b.3))
                });
                c.targets = order
                    .into_iter()
                    .take(c.targets.len().max(1))
                    .map(|x| x.0)
                    .collect();
                w.unit_mut(id)?.operator.advanced.harold_half = c
                    .targets
                    .iter()
                    .copied()
                    .filter(|&a| w.unit(a).unwrap().element_buildup.iter().any(|&v| v > 500.))
                    .collect();
            }
            Advanced::Mountain(p) => {
                let probability = if p.mode == Mode::Third && active {
                    p.active_probability
                } else {
                    p.probability
                };
                w.unit_mut(id)?.operator.advanced.critical =
                    probability > 0. && self.rng.chance(probability);
            }
            Advanced::Surtr(p) if p.mode == Mode::Second => {
                w.unit_mut(id)?.operator.advanced.solo = active && c.targets.len() == 1;
            }
            Advanced::Flamtl(_) => {
                if w.unit(id)?.operator.advanced.riposte {
                    let m = &mut w.unit_mut(id)?.operator.advanced;
                    m.riposte = false;
                    m.riposte_now = true;
                    let held = w.unit(id)?.blocking().to_vec();
                    for a in held {
                        if !c.targets.contains(&a)
                            && targeting::can_target_enemy(
                                w,
                                self,
                                Some(id),
                                a,
                                def.combat.profile.selection,
                            )?
                        {
                            c.targets.push(a);
                        }
                    }
                }
            }
            Advanced::Fartth(p) if p.mode == Mode::Second && active => {
                let own = targeting::absolute_range(
                    &p.base_grid,
                    w.unit(id)?.deployment.tile,
                    self.roster.as_ref().unwrap().entry(id)?.direction,
                    0,
                );
                let eligible = |w: &World, a: Id| {
                    w.unit(a).is_ok_and(|u| {
                        body::in_range(u, &own)
                            || u.blocked_by().is_some_and(|b| {
                                w.unit(b)
                                    .is_ok_and(|u| u.stats.alive && u.side == health::Side::Ally)
                            })
                    })
                };
                if c.targets.iter().any(|&a| !eligible(w, a)) {
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    let mut targets = search::Host::enemies(
                        self,
                        id,
                        &keys,
                        def.combat.profile.selection,
                        w,
                        w.unit_capacity(),
                    )?;
                    for &a in w.unit(id)?.blocking() {
                        if !targets.contains(&a) {
                            targets.push(a);
                        }
                    }
                    targets.retain(|&a| eligible(w, a));
                    targeting::sort_enemies(
                        w,
                        self,
                        id,
                        &mut targets,
                        def.combat.profile.priority,
                    )?;
                    targets.truncate(c.targets.len().max(1));
                    c.targets = targets;
                }
            }
            Advanced::Gnosis(p) if p.mode == Mode::Third && active => {
                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                let mut targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    def.combat.profile.selection,
                    w,
                    w.unit_capacity(),
                )?;
                for &a in w.unit(id)?.blocking() {
                    if !targets.contains(&a) {
                        targets.push(a);
                    }
                }
                if targets.len() > 1 {
                    targeting::sort_enemies(
                        w,
                        self,
                        id,
                        &mut targets,
                        def.combat.profile.priority,
                    )?;
                    targets.sort_by_key(|&a| Self::advanced_has(w, a, "freeze"));
                    targets.truncate(c.targets.len().max(1));
                    c.targets = targets;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_attack(
        &mut self,
        c: &attacks::Context,
        w: &mut World,
    ) -> Result<(), String> {
        let id = c.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let active = w.unit(id)?.operator.active_skill;
        let mut r = self.buffs.take().ok_or("Missing attack callback buffs")?;
        let result = (|| {
            match &def.extra.advanced {
                Advanced::Mizuki(p) if w.unit(id)?.stats.alive => {
                    if active
                        && p.mode == Mode::Third
                        && c.targets
                            .iter()
                            .filter(|&&a| w.unit(a).is_ok_and(|u| u.side == health::Side::Enemy))
                            .count()
                            < 3
                    {
                        let amount = Self::advanced_stat(w, id, "maxHp")? * p.loss;
                        self.support_lose_hp(w, &mut r, id, amount, None, false)?;
                    }
                    if !w.unit(id)?.stats.alive {
                        return Ok(());
                    }
                    let mut targets: Vec<_> = c
                        .targets
                        .iter()
                        .copied()
                        .filter(|&a| {
                            w.unit(a)
                                .is_ok_and(|u| u.stats.alive && u.side == health::Side::Enemy)
                        })
                        .collect();
                    targets.sort_by(|a, b| {
                        w.unit(*a)
                            .unwrap()
                            .stats
                            .hp
                            .total_cmp(&w.unit(*b).unwrap().stats.hp)
                            .then(a.cmp(b))
                    });
                    targets.truncate(
                        p.count
                            + if active && p.mode != Mode::First {
                                p.extra
                            } else {
                                0
                            },
                    );
                    for a in targets {
                        let amount = Self::advanced_stat(w, id, "atk")?
                            * p.scale
                            * if p.mode == Mode::First && c.is_skill {
                                p.skill_scale
                            } else {
                                1.
                            };
                        self.advanced_hit_damage(
                            w,
                            &mut r,
                            id,
                            a,
                            amount,
                            DamageType::Arts,
                            false,
                            false,
                            "talent",
                        )?;
                        if active && p.mode != Mode::First && w.unit(a)?.stats.alive {
                            self.advanced_status(
                                w,
                                &mut r,
                                id,
                                a,
                                if p.mode == Mode::Second {
                                    "bind"
                                } else {
                                    "stun"
                                },
                                p.status,
                                None,
                            )?;
                        }
                    }
                }
                Advanced::Flamtl(_) => {
                    if std::mem::take(&mut w.unit_mut(id)?.operator.advanced.riposte_now) {
                        for &a in &c.targets {
                            if w.unit(a)?.stats.alive {
                                let amount = Self::advanced_stat(w, id, "atk")?
                                    * Self::advanced_stat(w, id, "atkScaleMul")?;
                                self.advanced_hit_damage(
                                    w,
                                    &mut r,
                                    id,
                                    a,
                                    amount,
                                    DamageType::Physical,
                                    false,
                                    true,
                                    "riposte",
                                )?;
                            }
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime) fn advanced_impact(
        &mut self,
        impact: attacks::Impact,
        skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        let id = impact.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        let active = w.unit(id)?.operator.active_skill;
        let mut r = self.buffs.take().ok_or("Missing impact buffs")?;
        let result = (|| {
            if impact.heal {
                if let Some(target) = impact.target {
                    match &def.extra.advanced {
                        Advanced::Harold(p) => {
                            let mut amount = Self::advanced_stat(w, id, "atk")? * p.element_ratio;
                            if p.severe
                                && active
                                && w.unit(id)?.operator.advanced.harold_half.contains(&target)
                            {
                                amount *= p.extra_scale;
                            }
                            if !crate::battle::element::locked(w, target)? {
                                for v in &mut w.unit_mut(target)?.element_buildup {
                                    *v -= v.min(amount);
                                }
                            }
                        }
                        Advanced::Reckpr(p)
                            if p.guard && active && p.heal > 0. && w.unit(target)?.stats.alive =>
                        {
                            let mut b = Buff {
                                key: Some("reckpr:guard".into()),
                                ..Default::default()
                            };
                            b.runtime.duration = p.duration;
                            b.runtime.source = Some(id);
                            b.runtime.tick_payload = p.heal;
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
                        _ => {}
                    }
                }
                return Ok(());
            }
            let Some(target) = impact.target else {
                return Ok(());
            };
            if let Advanced::Surtr(p) = &def.extra.advanced {
                if p.mode == Mode::First && impact.is_skill && !w.unit(target)?.stats.alive {
                    if let Some(skill) = skill {
                        let cost = skill.bank.cost(w)?;
                        skill.gain(w, self, cost, "skill", false, false)?;
                    }
                }
            }
            if !w.unit(target)?.stats.alive {
                return Ok(());
            }
            match &def.extra.advanced {
                Advanced::Mountain(p)
                    if w.unit(id)?.operator.advanced.critical && p.weaken != 0. =>
                {
                    self.component_buff(
                        w,
                        &mut r,
                        target,
                        "f12yin:punch",
                        [("atkPct".into(), p.weaken)].into(),
                        p.duration,
                        id,
                    )?;
                }
                Advanced::Aroma(p)
                    if !p.landing && impact.is_skill && targeting::flying(w, target)? =>
                {
                    let amount = Self::advanced_stat(w, id, "atk")? * p.fly_scale;
                    self.advanced_hit_damage(
                        w,
                        &mut r,
                        id,
                        target,
                        amount,
                        DamageType::Arts,
                        true,
                        false,
                        "antiAir",
                    )?;
                }
                Advanced::Glady(p) if p.mode != Mode::Third && impact.is_skill => {
                    self.advanced_pull(w, &mut r, id, target, p, None)?;
                }
                Advanced::Mostma(p) if p.ripple && active && impact.is_skill => {
                    let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
                    let grid = self.grid.take().ok_or("Missing ripple grid")?;
                    let result = displacement::push(
                        w,
                        &grid,
                        &mut allies::Blocking {
                            host: self,
                            buffs: &mut r,
                        },
                        target,
                        p.force,
                        displacement::Push {
                            from: Some(displacement::Origin::unit(w, id, dir)?),
                            ..Default::default()
                        },
                    );
                    self.grid = Some(grid);
                    result?;
                }
                _ => {}
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
}
impl Host {
    fn advanced_enemy_tag(&self, id: Id, tag: &str) -> bool {
        self.spawns
            .as_ref()
            .and_then(|s| s.definition(id).ok())
            .is_some_and(|d| {
                self.data.table("enemies").unwrap()[&d.key]["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|t| t == tag))
            })
    }
    pub(in crate::battle::runtime) fn advanced_phase(
        &mut self,
        p: hit::Phase,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if p != hit::Phase::Hit {
            return Ok(());
        }
        if w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = Arc::clone(
                &self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(q.hit.target)?
                    .definition,
            );
            if let Advanced::Mudrok(m) = &def.extra.advanced {
                if q.hit
                    .source
                    .is_some_and(|a| self.advanced_enemy_tag(a, "sarkaz"))
                {
                    q.damage.mul *= 1. - m.sarkaz_resistance;
                }
            }
            if matches!(q.hit.damage_type, DamageType::Physical | DamageType::Arts)
                && Self::advanced_has(w, q.hit.target, "glady:tide")
                && q.hit
                    .source
                    .is_some_and(|a| self.advanced_enemy_tag(a, "seamonster"))
            {
                let ids = self.roster.as_ref().unwrap().ids().to_vec();
                for id in ids {
                    if let Advanced::Glady(g) = &self
                        .roster
                        .as_ref()
                        .unwrap()
                        .entry(id)?
                        .definition
                        .extra
                        .advanced
                    {
                        q.damage.mul *= 1. - g.resistance;
                    }
                }
            }
        }
        let Some(source) = q
            .hit
            .source
            .filter(|&a| w.unit(a).is_ok_and(|u| u.has_operator_kit()))
        else {
            return Ok(());
        };
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
        let active = w.unit(source)?.operator.active_skill;
        match &def.extra.advanced {
            Advanced::Mountain(m) if q.is_attack && w.unit(source)?.operator.advanced.critical => {
                q.damage.mul *= m.scale
            }
            Advanced::Surtr(m)
                if q.is_attack
                    && !q.is_splash
                    && active
                    && w.unit(source)?.operator.advanced.solo =>
            {
                q.damage.amount *= m.solo
            }
            Advanced::Aroma(a)
                if q.is_attack
                    && w.unit(q.hit.target)?.side == health::Side::Enemy
                    && !Self::advanced_has(w, q.hit.target, "aroma:bubbled") =>
            {
                self.component_buff(
                    w,
                    r,
                    q.hit.target,
                    "aroma:bubbled",
                    Numbers::new(),
                    f64::INFINITY,
                    source,
                )?;
                q.damage.mul *= a.first_scale;
                self.advanced_status(w, r, source, q.hit.target, "levitate", a.levitate, None)?;
            }
            Advanced::Billro(b) if w.unit(q.hit.target)?.side == health::Side::Enemy => {
                if b.per_enemy > 0. {
                    let n = self.advanced_enemies(w, source)?.len() as f64;
                    q.damage.mul *= 1. + b.per_enemy * n.min(b.enemy_cap);
                }
                if b.mode == Mode::Third {
                    if q.is_attack && active && w.unit(source)?.operator.advanced.charged {
                        let old = w
                            .unit(q.hit.target)?
                            .stats
                            .buffs()
                            .iter()
                            .find(|b| b.key.as_deref() == Some("billro:mark"))
                            .map(|b| (b.runtime.time_left, b.runtime.source));
                        let mut mark = Buff {
                            key: Some("billro:mark".into()),
                            ..Default::default()
                        };
                        mark.runtime.duration = old.map(|x| x.0).unwrap_or_else(|| {
                            (self
                                .roster
                                .as_ref()
                                .unwrap()
                                .entry(source)
                                .unwrap()
                                .skill()
                                .map(|s| s.time_left)
                                .unwrap_or(w.unit(source).unwrap().operator.advanced.skill_left)
                                + 0.1)
                                .max(0.1)
                        });
                        mark.runtime.source = old.and_then(|x| x.1).or(Some(source));
                        mark.runtime.refresh = buffs::Refresh::Stack;
                        mark.runtime.max_stacks = 5;
                        r.add(
                            w,
                            self,
                            q.hit.target,
                            buffs::Spec {
                                buff: mark,
                                ..Default::default()
                            },
                        )?;
                    }
                    if let Some(mark) = w.unit(q.hit.target)?.stats.buffs().iter().find(|b| {
                        b.key.as_deref() == Some("billro:mark") && b.runtime.source == Some(source)
                    }) {
                        q.damage.mul *= 1. + b.mark * f64::from(mark.stacks);
                    }
                }
            }
            Advanced::Fartth(f) => {
                if active && q.hit.damage_type == DamageType::Physical {
                    q.dodge_mode = injury::DodgeMode::Ignore;
                }
                if f.mode == Mode::Third && active && q.is_attack {
                    let keys = targeting::absolute_range(
                        &f.base_grid,
                        w.unit(source)?.deployment.tile,
                        self.roster.as_ref().unwrap().entry(source)?.direction,
                        0,
                    );
                    if !body::in_range(w.unit(q.hit.target)?, &keys) {
                        q.damage.mul *= f.far_scale;
                    }
                }
            }
            Advanced::Glady(g)
                if q.is_attack && w.unit(q.hit.target)?.side == health::Side::Enemy =>
            {
                if Self::advanced_stat(w, q.hit.target, "massLevel")? <= g.max_weight {
                    q.damage.mul *= g.light_scale;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_landed(
        &mut self,
        e: &health::Event,
        q: &hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if w.unit(e.target)?.side == health::Side::Ally
            && w.unit(e.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(e.target)?.definition);
            if let Advanced::Fartth(_) = &def.extra.advanced {
                if e.amount > 0. && !q.hit.hp_loss {
                    w.unit_mut(e.target)?.operator.advanced.last_hurt = w.time;
                }
            }
            if let Advanced::Mudrok(p) = &def.extra.advanced {
                let left = w
                    .unit(e.target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("mudrok:layers"))
                    .map(|b| b.runtime.shield_hits)
                    .unwrap_or(0);
                let broken = w
                    .unit(e.target)?
                    .operator
                    .advanced
                    .layers
                    .saturating_sub(left);
                if broken > 0 && w.unit(e.target)?.stats.alive {
                    w.unit_mut(e.target)?.operator.advanced.layers = left;
                    let amount = Self::advanced_stat(w, e.target, "maxHp")?
                        * p.layer_heal
                        * f64::from(broken);
                    self.support_heal(
                        w,
                        r,
                        Some(e.target),
                        e.target,
                        amount,
                        HealOptions {
                            self_heal: true,
                            ..Default::default()
                        },
                    )?;
                    self.advanced_grant_sp(w, r, e.target, p.layer_sp * f64::from(broken))?;
                }
            }
            if e.amount > 0.
                && !q.hit.hp_loss
                && q.hit.damage_type != DamageType::Elemental
                && w.unit(e.target)?.stats.hp > 0.
            {
                let guard = w
                    .unit(e.target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("reckpr:guard"))
                    .map(|b| (b.runtime.source, b.runtime.tick_payload));
                if let Some((Some(source), amount)) = guard {
                    self.support_heal(
                        w,
                        r,
                        Some(source),
                        e.target,
                        amount,
                        HealOptions::default(),
                    )?;
                }
            }
        }
        let Some(source) = q
            .hit
            .source
            .filter(|&a| w.unit(a).is_ok_and(|u| u.has_operator_kit()))
        else {
            return Ok(());
        };
        if w.unit(e.target)?.side != health::Side::Enemy {
            return Ok(());
        }
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
        match &def.extra.advanced {
            Advanced::Billro(b)
                if b.mode == Mode::Second
                    && q.is_attack
                    && w.unit(source)?.operator.active_skill
                    && w.unit(e.target)?.stats.alive =>
            {
                let charged = w.unit(source)?.operator.advanced.charged;
                self.advanced_status(
                    w,
                    r,
                    source,
                    e.target,
                    if charged { "bind" } else { "sluggish" },
                    if charged { b.bind } else { b.sluggish },
                    None,
                )?;
            }
            Advanced::Gnosis(g) if q.is_attack && !q.is_splash && w.unit(e.target)?.stats.alive => {
                self.advanced_status(w, r, source, e.target, "cold", g.cold, None)?;
            }
            Advanced::Fartth(f)
                if q.is_attack
                    && !q.is_splash
                    && f.module_sp > 0.
                    && w.unit(e.target)?.stats.alive
                    && w.unit(e.target)?.stats.hp > 0. =>
            {
                self.advanced_grant_sp(w, r, source, f.module_sp)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_fatal(
        &mut self,
        e: &mut health::Event,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if e.stage != health::Stage::Fatal
            || e.prevented
            || w.unit(e.target)?.side != health::Side::Ally
        {
            return Ok(());
        }
        let id = e.target;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        if let Advanced::Surtr(p) = &def.extra.advanced {
            e.prevented = true;
            if !w.unit(id)?.operator.advanced.ember {
                let now = w.time;
                let m = &mut w.unit_mut(id)?.operator.advanced;
                m.ember = true;
                self.tac_timer(
                    w,
                    id,
                    now + p.withdraw,
                    0.,
                    1,
                    crate::battle::runtime::tactics::Task::Callback(|k| k.retreat()),
                )?;
                let mut b = Buff {
                    key: Some("surtr:ember".into()),
                    flags: ["noHeal".into(), "healFree".into()].into(),
                    ..Default::default()
                };
                b.runtime.status = Some("healFree".into());
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
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_removed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        self.advanced_clear_marks(w, r, id, "aroma:bubbled")?;
        w.unit_mut(id)?.operator.advanced.riposte = false;
        w.unit_mut(id)?.operator.advanced.riposte_now = false;
        w.unit_mut(id)?.operator.advanced.extra_range.clear();
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_notice(
        &mut self,
        notice: injury::Notice,
        q: &hit::Request,
        w: &mut World,
    ) -> Result<(), String> {
        if matches!(notice, injury::Notice::Dodge { .. })
            && w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            if matches!(
                self.roster
                    .as_ref()
                    .unwrap()
                    .entry(q.hit.target)?
                    .definition
                    .extra
                    .advanced,
                Advanced::Flamtl(_)
            ) {
                w.unit_mut(q.hit.target)?.operator.advanced.riposte = true;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_evade(
        e: &mut injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if c.request.hit.damage_type == DamageType::Physical
            && c.request.is_attack
            && c.request
                .hit
                .source
                .is_some_and(|a| c.world.unit(a).is_ok_and(|u| u.side == health::Side::Enemy))
            && c.dodge(e, None)?
        {
            c.runtime
                .remove_id(c.world, c.host, c.request.hit.target, c.buff)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_status_applied(
        &mut self,
        status: &crate::battle::status::Context,
        w: &mut World,
    ) -> Result<(), String> {
        if status.key != "levitate" || w.unit(status.target)?.side != health::Side::Enemy {
            return Ok(());
        }
        if let Some(roster) = &self.roster {
            for &id in roster.ids() {
                if matches!(
                    roster.entry(id)?.definition.extra.advanced,
                    Advanced::Aroma(_)
                ) {
                    w.unit_mut(id)?
                        .operator
                        .advanced
                        .floating
                        .insert(status.target);
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn advanced_element(
        &mut self,
        q: &mut crate::battle::element::Request,
        w: &mut World,
    ) -> Result<(), String> {
        let target = q.damage.hit.target;
        if w.unit(target)?.side != health::Side::Ally
            || !w.unit(target)?.element_buildup.iter().any(|&v| v > 500.)
        {
            return Ok(());
        }
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut cut: f64 = 0.;
        for id in ids {
            let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
            if let Advanced::Harold(h) = &def.extra.advanced {
                if w.unit(id)?.stats.alive && w.unit(id)?.deployed {
                    let keys = self.roster.as_ref().unwrap().range(w, id)?;
                    if body::in_range(w.unit(target)?, &keys) {
                        cut = cut.max(h.resistance);
                    }
                }
            }
        }
        q.damage.damage.mul *= 1. - cut;
        Ok(())
    }
}

impl Host {
    pub(in crate::battle::runtime) fn advanced_finish_skill(
        &mut self,
        w: &mut World,
        skill: &mut skills::Runtime,
    ) -> Result<(), String> {
        if std::mem::take(&mut w.unit_mut(skill.unit())?.operator.skill_death_pending) {
            skill.end(w, self, "death")?;
        }
        Ok(())
    }
}
