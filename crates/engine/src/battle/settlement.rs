// SPDX-License-Identifier: GPL-3.0-or-later
// HP settlement backed by the unit's canonical buffs; no mirrored live effect pool.
use super::{
    buffs::{self, BuffId, Runtime},
    damage::DamageType,
    health::{self, Id, World},
    hit::{self, Phase, Request},
    injury::{
        self, Dispatch, DodgeMode, Effect, EffectId, Effects, HookResult, InjuryEvent, Notice,
        Observation, Store,
    },
};
use crate::deterministic::rng::Rng;
pub trait Hooks: buffs::Hooks + Sized {
    fn phase_enabled(&self, _phase: Phase) -> bool {
        false
    }
    fn damage_phase(
        &mut self,
        _phase: Phase,
        _request: &mut Request,
        _amount: &mut f64,
        _r: &mut Runtime,
        _w: &mut World,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn injury(_token: u64, _event: &mut InjuryEvent, _c: &mut Context<'_, Self>) -> HookResult {
        Err("Injury callback handler not installed".into())
    }
    fn observe(_token: u64, _event: Observation, _c: &mut Context<'_, Self>) -> HookResult {
        Err("Injury observer handler not installed".into())
    }
    fn damage_notice(
        &mut self,
        _notice: Notice,
        _expected: f64,
        _request: &mut Request,
        _r: &mut Runtime,
        _w: &mut World,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn damage_cleanup(&mut self, _token: u64, _r: &mut Runtime, _w: &mut World) -> HookResult {
        Err("Deferred injury cleanup handler not installed".into())
    }
    fn health_event(
        &mut self,
        event: &mut health::Event,
        _r: &mut Runtime,
        w: &mut World,
    ) -> HookResult {
        self.emit(event, w)
    }
    fn damage_transfers(
        &mut self,
        _request: &Request,
        _r: &mut Runtime,
        _w: &mut World,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
}
pub struct Context<'a, H: Hooks> {
    pub host: &'a mut H,
    pub world: &'a mut World,
    pub runtime: &'a mut Runtime,
    pub rng: &'a mut Rng,
    pub request: &'a mut Request,
    pub buff: BuffId,
    pub expected: f64,
}
impl<H: Hooks> Context<'_, H> {
    pub fn block(
        &mut self,
        event: &mut InjuryEvent,
        probability: Option<f64>,
    ) -> Result<bool, String> {
        let mut store = Buffs {
            w: self.world,
            r: self.runtime,
            unit: self.request.hit.target,
        };
        let mut dispatch = Callbacks {
            h: self.host,
            request: self.request,
            expected: self.expected,
        };
        event.block(probability, &mut dispatch, &mut store, self.rng)
    }
    pub fn dodge(
        &mut self,
        event: &mut InjuryEvent,
        probability: Option<f64>,
    ) -> Result<bool, String> {
        let mut store = Buffs {
            w: self.world,
            r: self.runtime,
            unit: self.request.hit.target,
        };
        let mut dispatch = Callbacks {
            h: self.host,
            request: self.request,
            expected: self.expected,
        };
        event.dodge(probability, &mut dispatch, &mut store, self.rng)
    }
    pub fn resolve(&mut self, request: &mut Request) -> Result<f64, String> {
        resolve(self.world, self.runtime, self.rng, self.host, request)
    }
}
struct Buffs<'a> {
    w: &'a mut World,
    r: &'a mut Runtime,
    unit: Id,
}
fn effect(b: &super::stats::Buff) -> Effect {
    let p = &b.runtime.injury;
    Effect {
        key: b.key.clone().unwrap_or_else(|| "buff".into()),
        reduction_group: p.reduction_group.clone(),
        priority: p.priority,
        stacks: b.stacks,
        mods: if b.runtime.mods_present || !b.mods.is_empty() {
            Some(b.mods.clone())
        } else {
            None
        },
        has_flags: b.runtime.flags_present || !b.flags.is_empty(),
        reduction: p.reduction,
        shield: b.shield,
        shield_hits: b.runtime.shield_hits,
        shield_types: p.shield_types.clone(),
        keep_empty_shield: p.keep_empty_shield,
        injury: p.on_injury.is_some(),
        observer: p.observe.is_some(),
    }
}
impl Store for Buffs<'_> {
    fn order(&self, kind: DamageType) -> Result<Vec<(EffectId, f64)>, String> {
        Ok(self
            .w
            .unit(self.unit)?
            .stats
            .buffs()
            .iter()
            .filter(|b| effect(b).participates(kind))
            .map(|b| (b.runtime.id(), b.runtime.injury.priority))
            .collect())
    }
    fn read(&self, id: EffectId, retained: bool) -> Result<Option<Effect>, String> {
        if !retained
            && !self
                .w
                .unit(self.unit)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.runtime.id() == id)
        {
            return Ok(None);
        }
        Ok(Some(effect(self.r.get(self.w, self.unit, id)?)))
    }
    fn write_shields(&mut self, id: EffectId, shield: f64, hits: u32) -> HookResult {
        self.r.edit_timer(self.w, self.unit, id, |b| {
            b.shield = shield;
            b.runtime.shield_hits = hits;
        })
    }
    fn remove(&mut self, _: EffectId) -> HookResult {
        Err("Canonical effect removal requires buff callbacks".into())
    }
    fn mark_dirty(&mut self) -> HookResult {
        self.w.unit_mut(self.unit)?.stats.mark_dirty();
        Ok(())
    }
    fn deferred_limit(&self) -> usize {
        self.r.deferred_limit()
    }
}
struct Callbacks<'a, H> {
    h: &'a mut H,
    request: &'a mut Request,
    expected: f64,
}
impl<H: Hooks> Dispatch<Buffs<'_>> for Callbacks<'_, H> {
    fn injury(
        &mut self,
        id: EffectId,
        event: &mut InjuryEvent,
        s: &mut Buffs<'_>,
        rng: &mut Rng,
    ) -> HookResult {
        let token =
            s.r.get(s.w, s.unit, id)?
                .runtime
                .injury
                .on_injury
                .ok_or("Missing injury callback")?;
        H::injury(
            token,
            event,
            &mut Context {
                host: self.h,
                world: s.w,
                runtime: s.r,
                rng,
                request: self.request,
                buff: id,
                expected: self.expected,
            },
        )
    }
    fn observe(
        &mut self,
        id: EffectId,
        event: Observation,
        s: &mut Buffs<'_>,
        rng: &mut Rng,
    ) -> HookResult {
        let token =
            s.r.get(s.w, s.unit, id)?
                .runtime
                .injury
                .observe
                .ok_or("Missing injury observer")?;
        H::observe(
            token,
            event,
            &mut Context {
                host: self.h,
                world: s.w,
                runtime: s.r,
                rng,
                request: self.request,
                buff: id,
                expected: self.expected,
            },
        )
    }
    fn notice(&mut self, notice: Notice, s: &mut Buffs<'_>, rng: &mut Rng) -> HookResult {
        self.h
            .damage_notice(notice, self.expected, self.request, s.r, s.w, rng)
    }
    fn remove(&mut self, id: EffectId, s: &mut Buffs<'_>, _: &mut Rng) -> HookResult {
        s.r.remove_id(s.w, self.h, s.unit, id)?;
        Ok(())
    }
}
struct Pipeline<'a, H> {
    h: &'a mut H,
    r: &'a mut Runtime,
}
impl<H: Hooks> health::Hooks for Pipeline<'_, H> {
    fn emit(&mut self, event: &mut health::Event, w: &mut World) -> HookResult {
        self.h.health_event(event, self.r, w)
    }
    fn number(&mut self, heal: bool, target: Id, amount: f64) -> HookResult {
        self.h.number(heal, target, amount)
    }
    fn hit_cap(&mut self, target: Id, amount: f64) -> HookResult {
        self.h.hit_cap(target, amount)
    }
    fn kill(&mut self, target: Id, credit: Option<Id>, w: &mut World) -> HookResult {
        self.h.kill(target, credit, w)
    }
    fn heal_through(&mut self, source: Id, target: Id, w: &World) -> bool {
        self.h.heal_through(source, target, w)
    }
    fn overheal(
        &mut self,
        target: Id,
        excess: f64,
        max_hp: f64,
        duration: Option<f64>,
        w: &mut World,
    ) -> HookResult {
        self.h.overheal(target, excess, max_hp, duration, w)
    }
}
impl<H: Hooks> hit::Hooks for Pipeline<'_, H> {
    fn enabled(&self, phase: Phase) -> bool {
        self.h.phase_enabled(phase)
    }
    fn phase(
        &mut self,
        p: Phase,
        r: &mut Request,
        amount: &mut f64,
        w: &mut World,
        _: &mut Effects,
        rng: &mut Rng,
    ) -> HookResult {
        self.h.damage_phase(p, r, amount, self.r, w, rng)
    }
    fn resolve_injury(
        &mut self,
        w: &mut World,
        _: &mut Effects,
        rng: &mut Rng,
        request: &mut Request,
        amount: f64,
        kind: DamageType,
        mode: DodgeMode,
        ignore_block: bool,
        expected: f64,
    ) -> Result<injury::InjuryResult, String> {
        self.r.adopt(w, request.hit.target)?;
        let mut store = Buffs {
            unit: request.hit.target,
            w,
            r: self.r,
        };
        injury::run(
            &mut store,
            rng,
            &mut Callbacks {
                h: self.h,
                request,
                expected,
            },
            amount,
            kind,
            mode,
            ignore_block,
        )
    }
    fn cleanup(&mut self, token: u64, w: &mut World) -> HookResult {
        self.h.damage_cleanup(token, self.r, w)
    }
    fn transfers(
        &mut self,
        request: &Request,
        w: &mut World,
        _: &mut Effects,
        rng: &mut Rng,
    ) -> HookResult {
        self.h.damage_transfers(request, self.r, w, rng)
    }
}
pub fn resolve<H: Hooks>(
    w: &mut World,
    r: &mut Runtime,
    rng: &mut Rng,
    h: &mut H,
    request: &mut Request,
) -> Result<f64, String> {
    r.call(|r| {
        // Empty legacy callback argument: the canonical backend never reads or writes it.
        // BTreeMap::new allocates no pool; all effects remain in World.unit.stats.buffs.
        let mut unused = Effects::new(1)?;
        hit::resolve(w, &mut unused, rng, &mut Pipeline { h, r }, request)
    })
}
