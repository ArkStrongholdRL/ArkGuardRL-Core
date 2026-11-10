// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed Kit callbacks and bounded access to the canonical battle mechanisms.
use super::*;
use crate::battle::{element, status};
mod context;
pub(crate) use context::Context;

/// Identifies the actor that caused or received a combat event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    Source,
    Target,
}

pub(crate) enum Event<'a> {
    BattleStart,
    /// Initialization uses dt = 0, and frame refresh uses the simulation step.
    Refresh {
        skill: &'a mut skills::Runtime,
        dt: f64,
    },
    /// The deferred callback's skill has been restored to the roster.
    Restored(skills::Event),
    /// Reconcile effects after draining the deferred lifecycle queue.
    Flush,
    Resume {
        unit: Id,
        skill: &'a mut skills::Runtime,
    },
    Skill {
        unit: Id,
        event: skills::Event,
        context: &'a mut skills::Context,
        skill: &'a mut skills::Runtime,
    },
    SkillObserved {
        unit: Id,
        event: skills::Event,
        skill: &'a mut skills::Runtime,
    },
    Scheduled([bool; 5]),
    /// Emitted once per simulation step by clock::Phase::TickHook.
    Tick,
    Before(&'a mut attacks::Context),
    Attack {
        context: &'a mut attacks::Context,
        skill: Option<&'a mut skills::Runtime>,
    },
    /// Attack observers have run, and the skill has yet to consume ammunition.
    AttackFinished {
        context: &'a attacks::Context,
        skill: Option<&'a mut skills::Runtime>,
    },
    Impact(attacks::Impact),
    Hit {
        phase: hit::Phase,
        request: &'a mut hit::Request,
    },
    Landed {
        event: &'a health::Event,
        request: &'a hit::Request,
    },
    Heal {
        event: &'a health::Event,
        options: health::HealOptions,
    },
    Fatal(&'a mut health::Event),
    DamageApplied {
        event: &'a health::Event,
        request: &'a hit::Request,
    },
    ProjectileStep {
        id: u64,
        source: Option<Id>,
        position: [f64; 2],
        speed: f64,
        removable: bool,
        cancelled: &'a mut bool,
    },
    Element {
        phase: element::Phase,
        request: &'a mut element::Request,
    },
    ElementLate(&'a mut element::Request),
    BeforeStatus(&'a mut status::Context),
    Status(&'a status::Context),
    /// Status reactions have completed, before advanced support reactions.
    StatusFinished(&'a status::Context),
    /// Enemy kill, before removal; callbacks may reenter damage.
    Kill {
        victim: Id,
    },
    Death {
        unit: Id,
        credit: Option<Id>,
    },
    Buff {
        context: buffs::Context,
        event: buffs::Event,
        token: u64,
    },
}

impl Event<'_> {
    pub(super) fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::Skill { .. }
                | Self::Resume { .. }
                | Self::Refresh { .. }
                | Self::Restored(_)
                | Self::Flush
        )
    }
}

pub(crate) trait Behavior {
    fn can_activate(&self, _host: &Host, _world: &mut World, _unit: Id) -> Result<bool, String> {
        Ok(true)
    }
    fn condition(
        &self,
        _host: &Host,
        _world: &mut World,
        _unit: Id,
        _condition: skills::Condition,
    ) -> Result<Option<bool>, String> {
        Ok(None)
    }
    fn sp_gain(&self, _host: &Host, _unit: Id, _context: &mut skills::Context) {}
    fn observe_injury(
        &self,
        _context: &mut settlement::Context<'_, Host>,
        _key: &str,
        _buff: &Buff,
    ) -> Result<(), String> {
        Ok(())
    }
    fn heal_amount(
        &self,
        _world: &World,
        _unit: Id,
        _target: Id,
        amount: f64,
        _options: health::HealOptions,
    ) -> Result<f64, String> {
        Ok(amount)
    }
    /// Deliver a combat callback to its source or target Kit.
    fn handle_unit(
        &self,
        _context: &mut Context<'_>,
        _role: Role,
        _event: &mut Event<'_>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn attack_heal(
        &self,
        _world: &World,
        _unit: Id,
        _target: Id,
        amount: f64,
    ) -> Result<f64, String> {
        Ok(amount)
    }
    fn observes(&self, event: &Event<'_>) -> bool {
        !matches!(
            event,
            Event::AttackFinished { .. } | Event::StatusFinished(_)
        )
    }
    fn refreshes_skill(&self) -> bool {
        false
    }
    fn priority(&self, _event: &Event<'_>, _world: &World, _unit: Id) -> i32 {
        0
    }
    fn handle(&self, context: &mut Context<'_>, event: &mut Event<'_>) -> Result<(), String>;
    fn profile(
        &self,
        _world: &World,
        _unit: Id,
        _skill: Option<&skills::Runtime>,
        _profile: &mut search::AttackProfile,
    ) -> Result<(), String> {
        Ok(())
    }
    fn payload(
        &self,
        _world: &World,
        _unit: Id,
        _payload: &mut attacks::Direct,
        _area: &mut attacks::Area,
    ) -> Result<(), String> {
        Ok(())
    }
    fn reaper_eligible(&self, _request: &hit::Request) -> bool {
        true
    }
    fn reaper_multiplier(&self, _world: &World, _unit: Id) -> Result<f64, String> {
        Ok(1.)
    }
    fn heal_multiplier(&self, _world: &World, _target: Id) -> Result<f64, String> {
        Ok(1.)
    }
    fn can_attack(&self, _host: &Host, _world: &World, _unit: Id) -> Result<bool, String> {
        Ok(true)
    }
    fn enemy_filter(&self) -> Option<targeting::EnemyFilter> {
        None
    }
}

impl Host {
    pub(in crate::battle::runtime) fn kit_event(
        &mut self,
        world: &mut World,
        buffs: &mut buffs::Runtime,
        event: &mut Event<'_>,
    ) -> Result<(), String> {
        let Some(roster) = self.roster.as_ref() else {
            return Ok(());
        };
        enum Observer {
            Kit(Id, Arc<Definition>),
            HeldUndying,
            HammerLock,
            Chivalry,
            Cocoon,
            Egir,
            Indom(Option<String>),
            GrowthDeath,
            DeathTarget(Id),
        }
        let mut handlers = Vec::new();
        if let Event::Death { unit, .. } = event {
            let u = world.unit(*unit)?;
            if u.has_operator_kit() {
                // Capture the departing unit's effects before any revival.
                handlers.push((20, Observer::DeathTarget(*unit)));
            }
            if u.side == health::Side::Ally
                && u.deployment.remove_reason.as_deref() == Some("killed")
            {
                handlers.push((0, Observer::GrowthDeath));
            }
        }
        for (unit, def) in roster.kit_observers(matches!(event, Event::Tick)) {
            let unit = *unit;
            if let Some(kit) = def.extra.tactics.behavior() {
                if !kit.observes(event) {
                    continue;
                }
                let priority = kit.priority(event, world, unit);
                handlers.push((priority, Observer::Kit(unit, Arc::clone(def))));
            }
        }
        if matches!(event, Event::Fatal(_))
            && self
                .equipment
                .units
                .values()
                .any(|loadout| loadout.hammers.is_some())
        {
            handlers.push((-99, Observer::HeldUndying));
            handlers.push((-100, Observer::HammerLock));
        }
        if matches!(event, Event::Fatal(_)) {
            handlers.push((20, Observer::Chivalry));
        }
        if let Event::Death { unit, .. } = event {
            handlers.push((13, Observer::Cocoon));
            handlers.push((11, Observer::Egir));
            handlers.push((
                10,
                Observer::Indom(world.unit(*unit)?.deployment.remove_reason.clone()),
            ));
        }
        handlers.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, observer) in handlers {
            match observer {
                Observer::Chivalry => {
                    if let Event::Fatal(e) = event {
                        self.chivalry_fatal(e)?;
                    }
                }
                Observer::Egir => {
                    if let Event::Death { unit, .. } = event {
                        self.egir_death(world, buffs, *unit)?;
                    }
                }
                Observer::Cocoon => {
                    if let Event::Death { unit, .. } = event {
                        self.equipment_death(world, buffs, *unit)?;
                    }
                }
                Observer::DeathTarget(unit) => {
                    self.kit_unit_event(world, buffs, unit, Role::Target, event)?;
                }
                Observer::GrowthDeath => {
                    if let Event::Death { unit, credit } = event {
                        self.growth_death(world, *unit, *credit)?;
                    }
                }
                Observer::Indom(reason) => {
                    if let Event::Death { unit, .. } = event {
                        self.bond_indom_death(world, buffs, *unit, reason.as_deref())?;
                    }
                }
                Observer::HeldUndying => {
                    if let Event::Fatal(e) = event {
                        if !e.prevented && self.holds_undying(world, e.target)? {
                            e.prevented = true;
                        }
                    }
                }
                Observer::HammerLock => {
                    if let Event::Fatal(e) = event {
                        self.hammer_fatal(world, e)?;
                    }
                }
                Observer::Kit(unit, def) => def.extra.tactics.behavior().unwrap().handle(
                    &mut Context {
                        host: self,
                        world,
                        buffs,
                        unit,
                    },
                    event,
                )?,
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_event_for(
        &mut self,
        world: &mut World,
        buffs: &mut buffs::Runtime,
        unit: Id,
        event: &mut Event<'_>,
    ) -> Result<(), String> {
        let def = self.tac_def(unit)?;
        if let Some(behavior) = def
            .extra
            .tactics
            .behavior()
            .filter(|kit| kit.observes(event))
        {
            behavior.handle(
                &mut Context {
                    host: self,
                    world,
                    buffs,
                    unit,
                },
                event,
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_event_with_buffs(
        &mut self,
        world: &mut World,
        event: &mut Event<'_>,
    ) -> Result<(), String> {
        let mut buffs = self.buffs.take().ok_or("Missing Kit buff context")?;
        let result = self.kit_event(world, &mut buffs, event);
        self.buffs = Some(buffs);
        result
    }
}

pub(in crate::battle::runtime) struct DeferredEvent {
    pub unit: Id,
    pub deployment: u64,
    pub event: skills::Event,
    pub context: skills::Context,
}
impl Host {
    pub(in crate::battle::runtime) fn kit_skill_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        _def: &Definition,
        event: skills::Event,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        let id = s.unit();
        if self.roster.is_none() {
            if matches!(
                event,
                skills::Event::Deploy
                    | skills::Event::Start
                    | skills::Event::SkillStart
                    | skills::Event::End
                    | skills::Event::SkillEnd
                    | skills::Event::AmmoUsed
            ) {
                if self.deferred_kit_events.len() >= self.kit_event_limit {
                    return Err("Deferred kit event capacity exceeded".into());
                }
                self.deferred_kit_events.push_back(DeferredEvent {
                    unit: id,
                    deployment: w.unit(id)?.deployment.deploy_seq,
                    event,
                    context: c.clone(),
                });
            }
            return Ok(());
        }
        self.kit_event_for(
            w,
            r,
            id,
            &mut Event::Skill {
                unit: id,
                event,
                context: c,
                skill: s,
            },
        )?;
        if matches!(
            event,
            skills::Event::Deploy
                | skills::Event::SkillStart
                | skills::Event::AmmoUsed
                | skills::Event::SkillEnd
        ) {
            self.kit_event(
                w,
                r,
                &mut Event::SkillObserved {
                    unit: id,
                    event,
                    skill: s,
                },
            )?;
        }
        self.bonds_skill(w, r, event, c, s)?;
        self.growth_skill(w, id, event)?;

        self.equipment_skill(w, r, event, c, s)?;
        if event == skills::Event::Deploy
            && w.unit(id)?.deployment.kind == crate::battle::deploy::Kind::Operator
        {
            self.last_operator = Some(id);
        }

        Ok(())
    }
    pub(in crate::battle::runtime) fn flush_kit_events(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if self.roster.is_none() || self.flushing_kit_events {
            return Ok(());
        }
        self.flushing_kit_events = true;
        let result = (|| {
            while let Some(pending) = self.deferred_kit_events.front() {
                let id = pending.unit;
                // A nested callback may still own this skill. Its enclosing entry point flushes again.
                if self.roster.as_ref().unwrap().entry(id)?.skill().is_err() {
                    break;
                }
                let mut pending = self.deferred_kit_events.pop_front().unwrap();
                if w.unit(id)?.deployment.deploy_seq != pending.deployment
                    || (pending.event != skills::Event::End && !Self::tac_up(w, id))
                {
                    continue;
                }
                let def = self.tac_def(id)?;
                let mut skill = self.roster.as_mut().unwrap().take_skill(id)?;
                let result = self.kit_skill_event(
                    w,
                    r,
                    &def,
                    pending.event,
                    &mut pending.context,
                    &mut skill,
                );
                let restore = self.roster.as_mut().unwrap().restore_skill(id, skill);
                result.and(restore)?;
                self.kit_event_for(w, r, id, &mut Event::Restored(pending.event))?;
            }
            for id in self.roster.as_ref().unwrap().ids().to_vec() {
                self.kit_event_for(w, r, id, &mut Event::Flush)?;
            }
            self.growth_flush_onstart(w, r)?;
            self.bonds_flush(w, r)?;
            Ok(())
        })();
        self.flushing_kit_events = false;
        result
    }
    pub(in crate::battle::runtime) fn with_kit_buffs<T>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        f: impl FnOnce(&mut Self, &mut World) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.buffs.is_some() {
            return Err("Kit callback already owns buffs".into());
        }
        self.buffs = Some(std::mem::replace(r, buffs::Runtime::new(1, 1, 1)?));
        let result = f(self, w);
        *r = self.buffs.take().ok_or("Kit callback lost buffs")?;
        result
    }
    pub(in crate::battle::runtime) fn resume_kit_skill(
        &mut self,
        world: &mut World,
        skill: &mut skills::Runtime,
    ) -> Result<(), String> {
        let id = skill.unit();
        let def = self.tac_def(id)?;
        if def.extra.tactics.behavior().is_some() {
            let mut buffs = self.buffs.take().ok_or("Missing resumed Kit buffs")?;
            let result = self.kit_event_for(
                world,
                &mut buffs,
                id,
                &mut kits::dispatch::Event::Resume { unit: id, skill },
            );
            self.buffs = Some(buffs);
            result?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_profile(
        &self,
        w: &World,
        id: Id,
        skill: Option<&skills::Runtime>,
        p: &mut search::AttackProfile,
    ) -> Result<(), String> {
        let def = self.tac_def(id)?;
        if let Some(kit) = def.extra.tactics.behavior() {
            kit.profile(w, id, skill, p)?;
        }

        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_payload(
        &self,
        w: &World,
        id: Id,
        p: &mut attacks::Direct,
        area: &mut attacks::Area,
    ) -> Result<(), String> {
        let def = self.tac_def(id)?;
        if let Some(kit) = def.extra.tactics.behavior() {
            kit.payload(w, id, p, area)?;
        }

        Ok(())
    }
    pub(in crate::battle::runtime) fn refresh_kit_effects(
        &mut self,
        w: &mut World,
        dt: f64,
    ) -> Result<(), String> {
        self.refresh_component_conditions(w)?;
        self.refresh_support_casts(w)?;
        for timer in &mut self.tactical_timers {
            if let Task::Explosion {
                target, position, ..
            } = &mut timer.kind
            {
                if let Ok(unit) = w.unit(*target) {
                    *position = unit.position;
                }
            }
        }
        for id in self.roster.as_ref().unwrap().ids().to_vec() {
            if !Self::tac_up(w, id) {
                continue;
            }
            let def = self.tac_def(id)?;
            if !def
                .extra
                .tactics
                .behavior()
                .is_some_and(|kit| kit.refreshes_skill())
            {
                continue;
            }
            let mut buffs = self.buffs.take().ok_or("Missing Kit refresh buffs")?;
            let mut skill = match self.roster.as_mut().unwrap().take_skill(id) {
                Ok(skill) => skill,
                Err(error) => {
                    self.buffs = Some(buffs);
                    return Err(error);
                }
            };
            let result = self.kit_event_for(
                w,
                &mut buffs,
                id,
                &mut Event::Refresh {
                    skill: &mut skill,
                    dt,
                },
            );
            self.buffs = Some(buffs);
            let result = result.and_then(|_| self.support_flush_sp(w, &mut skill));
            let restored = self.roster.as_mut().unwrap().restore_skill(id, skill);
            result.and(restored)?;
        }
        Ok(())
    }
}

impl Host {
    pub(in crate::battle::runtime) fn dispatch_scheduled_kits(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let ids: Vec<_> = self
            .roster
            .as_ref()
            .unwrap()
            .kit_observers(false)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let mut due = [false; 5];
            for (i, iv) in [0.1, 0.2, 0.25, 0.5, 1.].into_iter().enumerate() {
                let at = w.unit(id)?.operator.tactical.next[i].max(iv);
                if w.time + 1e-9 >= at {
                    due[i] = true;
                    w.unit_mut(id)?.operator.tactical.next[i] = at + iv;
                }
            }
            self.kit_event_for(w, r, id, &mut Event::Scheduled(due))?;
        }
        Ok(())
    }
}

impl Host {
    pub(in crate::battle::runtime) fn kit_unit_event(
        &mut self,
        world: &mut World,
        buffs: &mut buffs::Runtime,
        unit: Id,
        role: Role,
        event: &mut Event<'_>,
    ) -> Result<(), String> {
        if self.roster.is_none()
            || world.unit(unit)?.side != health::Side::Ally
            || world.unit(unit)?.deployment.kind == crate::battle::deploy::Kind::Device
        {
            return Ok(());
        }
        let definition = self.tac_def(unit)?;
        if let Some(kit) = definition.extra.tactics.behavior() {
            kit.handle_unit(
                &mut Context {
                    host: self,
                    world,
                    buffs,
                    unit,
                },
                role,
                event,
            )?;
        }
        Ok(())
    }
}

impl Host {
    pub(in crate::battle::runtime) fn kit_before(
        &mut self,
        w: &mut World,
        c: &mut attacks::Context,
    ) -> Result<(), String> {
        let mut r = self
            .buffs
            .take()
            .ok_or("Missing attack preparation buffs")?;
        let result = (|| {
            self.kit_event(w, &mut r, &mut Event::Before(c))?;
            self.kit_unit_event(w, &mut r, c.attacker, Role::Source, &mut Event::Before(c))
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime) fn kit_attack(
        &mut self,
        w: &mut World,
        c: &mut attacks::Context,
        mut skill: Option<&mut skills::Runtime>,
    ) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing attack callback buffs")?;
        let result = (|| {
            self.kit_unit_event(
                w,
                &mut r,
                c.attacker,
                Role::Source,
                &mut Event::Attack {
                    context: c,
                    skill: skill.as_deref_mut(),
                },
            )?;
            self.kit_event(
                w,
                &mut r,
                &mut Event::Attack {
                    context: c,
                    skill: skill.as_deref_mut(),
                },
            )?;
            self.kit_event(w, &mut r, &mut Event::AttackFinished { context: c, skill })
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime) fn kit_impact(
        &mut self,
        w: &mut World,
        impact: attacks::Impact,
    ) -> Result<(), String> {
        if impact.target.is_none() {
            return Ok(());
        }
        let mut r = self.buffs.take().ok_or("Missing impact callback buffs")?;
        let result = (|| {
            self.kit_event(w, &mut r, &mut Event::Impact(impact))?;
            self.kit_unit_event(
                w,
                &mut r,
                impact.attacker,
                Role::Source,
                &mut Event::Impact(impact),
            )
        })();
        self.buffs = Some(r);
        result
    }
    pub(in crate::battle::runtime) fn kit_heal_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        options: health::HealOptions,
    ) -> Result<(), String> {
        let mut event = Event::Heal { event: e, options };
        self.kit_event(w, r, &mut event)?;
        let source = match e.source {
            Some(source) if self.roster.is_some() && w.unit(source)?.side == health::Side::Ally => {
                Some(source)
            }
            _ => None,
        };
        let after_items = if let Some(source) = source {
            self.tac_def(source)?
                .extra
                .tactics
                .behavior()
                .is_some_and(|behavior| behavior.priority(&event, w, source) < 0)
        } else {
            false
        };
        if after_items {
            self.equipment_heal(w, r, e, options)?;
        }
        if let Some(source) = source {
            self.kit_unit_event(w, r, source, Role::Source, &mut event)?;
        }
        if !after_items {
            self.equipment_heal(w, r, e, options)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_hit_phase(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        phase: hit::Phase,
        request: &mut hit::Request,
    ) -> Result<(), String> {
        // Broadcast reactions precede effect synchronization and target/source callbacks.
        self.kit_event(w, r, &mut Event::Hit { phase, request })?;
        if self.roster.is_none() {
            return Ok(());
        }
        if phase == hit::Phase::Hit {
            self.tactical_sync_zones(w, r)?;
            self.tactical_sync_injury(w, r)?;
            self.kit_unit_event(
                w,
                r,
                request.hit.target,
                Role::Target,
                &mut Event::Hit { phase, request },
            )?;
        }
        if let Some(source) = request.hit.source {
            self.kit_unit_event(
                w,
                r,
                source,
                Role::Source,
                &mut Event::Hit { phase, request },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: &health::Event,
        request: &hit::Request,
    ) -> Result<(), String> {
        self.kit_event(w, r, &mut Event::Landed { event, request })?;
        if let Some(source) = request.hit.source {
            self.kit_unit_event(
                w,
                r,
                source,
                Role::Source,
                &mut Event::Landed { event, request },
            )?;
        }
        self.kit_unit_event(
            w,
            r,
            event.target,
            Role::Target,
            &mut Event::Landed { event, request },
        )
    }
    pub(in crate::battle::runtime) fn kit_kill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        victim: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        if let Some(source) = credit {
            self.kit_unit_event(w, r, source, Role::Source, &mut Event::Kill { victim })?;
        }
        self.kit_event(w, r, &mut Event::Kill { victim })
    }
    pub(in crate::battle::runtime) fn kit_attack_heal(
        &self,
        world: &World,
        unit: Id,
        target: Id,
        amount: f64,
    ) -> Result<f64, String> {
        let definition = self.tac_def(unit)?;
        match definition.extra.tactics.behavior() {
            Some(kit) => kit.attack_heal(world, unit, target, amount),
            None => Ok(amount),
        }
    }
}

impl Host {
    pub(in crate::battle::runtime) fn kit_element(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        request: &mut element::Request,
    ) -> Result<(), String> {
        let phase = element::Phase::Hit;
        self.kit_unit_event(
            w,
            r,
            request.damage.hit.target,
            Role::Target,
            &mut Event::Element { phase, request },
        )?;
        if let Some(source) = request.damage.hit.source {
            self.kit_unit_event(
                w,
                r,
                source,
                Role::Source,
                &mut Event::Element { phase, request },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_heal_amount(
        &self,
        w: &World,
        source: Option<Id>,
        target: Id,
        amount: f64,
        options: health::HealOptions,
    ) -> Result<f64, String> {
        let Some(source) =
            source.filter(|&unit| w.unit(unit).is_ok_and(|u| u.side == health::Side::Ally))
        else {
            return Ok(amount);
        };
        let definition = self.tac_def(source)?;
        match definition.extra.tactics.behavior() {
            Some(kit) => kit.heal_amount(w, source, target, amount, options),
            None => Ok(amount),
        }
    }
    pub(in crate::battle::runtime) fn kit_observe(
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if c.request.hit.hp_loss {
            return Ok(());
        }
        let target = c.request.hit.target;
        let buff = c.runtime.get(c.world, target, c.buff)?.clone();
        let owner = buff.runtime.source.unwrap_or(target);
        let definition = c.host.tac_def(owner)?;
        c.host.rng = c.rng.clone();
        let result = match definition.extra.tactics.behavior() {
            Some(kit) => kit.observe_injury(c, buff.key.as_deref().unwrap_or(""), &buff),
            None => Ok(()),
        };
        *c.rng = c.host.rng.clone();
        result
    }
    pub(in crate::battle::runtime) fn kit_buff_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        context: buffs::Context,
        event: buffs::Event,
        token: u64,
    ) -> Result<(), String> {
        let owner = r
            .get(w, context.unit, context.buff)?
            .runtime
            .source
            .unwrap_or(context.unit);
        self.kit_unit_event(
            w,
            r,
            owner,
            Role::Source,
            &mut Event::Buff {
                context,
                event,
                token,
            },
        )
    }
}

impl Host {
    pub(in crate::battle::runtime) fn kit_can_activate(
        &self,
        world: &mut World,
        unit: Id,
    ) -> Result<bool, String> {
        if self.roster.is_none() {
            return Ok(true);
        }
        let definition = self.tac_def(unit)?;
        match definition.extra.tactics.behavior() {
            Some(kit) => kit.can_activate(self, world, unit),
            None => Ok(true),
        }
    }
    pub(in crate::battle::runtime) fn kit_condition(
        &self,
        world: &mut World,
        unit: Id,
        condition: skills::Condition,
    ) -> Result<Option<bool>, String> {
        let definition = self.tac_def(unit)?;
        match definition.extra.tactics.behavior() {
            Some(kit) => kit.condition(self, world, unit, condition),
            None => Ok(None),
        }
    }
    pub(in crate::battle::runtime) fn kit_sp_gain(
        &self,
        unit: Id,
        context: &mut skills::Context,
    ) -> Result<(), String> {
        let definition = self.tac_def(unit)?;
        if let Some(kit) = definition.extra.tactics.behavior() {
            kit.sp_gain(self, unit, context);
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn kit_damage_applied(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: &health::Event,
        request: &hit::Request,
    ) -> Result<(), String> {
        self.kit_event(w, r, &mut Event::DamageApplied { event, request })?;
        if let Some(source) = event.source {
            self.kit_unit_event(
                w,
                r,
                source,
                Role::Source,
                &mut Event::DamageApplied { event, request },
            )?;
        }
        Ok(())
    }
}
