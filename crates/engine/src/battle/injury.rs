// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol attack/effects.js. Stable effect identities preserve the
// snapshot order even when callbacks remove effects or trigger nested damage.
use super::{damage::DamageType, stats::Numbers};
use crate::deterministic::rng::Rng;
use std::collections::{BTreeMap, BTreeSet};
pub type EffectId = u64;
pub type HookResult = Result<(), String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DodgeMode {
    Ignore,
    Normal,
    Uncancellable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Zero,
    Blocked,
    Dodged,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Notice {
    Block { cancelled: bool },
    Dodge { cancelled: bool, amount: f64 },
}
#[derive(Clone, Debug)]
pub struct Effect {
    pub key: String,
    pub reduction_group: Option<String>,
    pub priority: f64,
    pub stacks: u32,
    pub mods: Option<Numbers>,
    pub has_flags: bool,
    pub reduction: Option<[f64; 4]>,
    pub shield: f64,
    pub shield_hits: u32,
    pub shield_types: Option<Vec<DamageType>>,
    pub keep_empty_shield: bool,
    pub injury: bool,
    pub observer: bool,
}
impl Default for Effect {
    fn default() -> Self {
        Self {
            key: "buff".into(),
            reduction_group: None,
            priority: 0.0,
            stacks: 1,
            mods: None,
            has_flags: false,
            reduction: None,
            shield: 0.0,
            shield_hits: 0,
            shield_types: None,
            keep_empty_shield: false,
            injury: false,
            observer: false,
        }
    }
}
fn index(kind: DamageType) -> usize {
    match kind {
        DamageType::Physical => 0,
        DamageType::Arts => 1,
        DamageType::True => 2,
        DamageType::Elemental => 3,
    }
}
fn taken(kind: DamageType) -> &'static str {
    match kind {
        DamageType::Physical => "physTakenMul",
        DamageType::Arts => "artsTakenMul",
        DamageType::True => "trueTakenMul",
        DamageType::Elemental => "elementalTakenMul",
    }
}
fn dodge_key(kind: DamageType) -> Option<&'static str> {
    match kind {
        DamageType::Physical => Some("dodgePhys"),
        DamageType::Arts => Some("dodgeArts"),
        _ => None,
    }
}
impl Effect {
    fn mod_value(&self, key: &str) -> Option<f64> {
        self.mods.as_ref()?.get(key).copied()
    }
    fn multiplier(&self, key: &str) -> f64 {
        self.mod_value(key).filter(|x| x.is_finite()).unwrap_or(1.0)
    }
    fn reduction_value(&self, kind: DamageType) -> f64 {
        self.reduction.map(|r| r[index(kind)]).unwrap_or(0.0)
    }
    fn participates(&self, kind: DamageType) -> bool {
        self.injury
            || self.observer
            || self.reduction_value(kind) > 0.0
            || self.shield_hits > 0
            || self.shield > 0.0
            || self.mod_value("dodgePhys").unwrap_or(0.0) > 0.0
            || self.mod_value("dodgeArts").unwrap_or(0.0) > 0.0
            || self.mod_value("dmgTakenMul").is_some()
            || self.mod_value(taken(kind)).is_some()
    }
    fn disposable(&self) -> bool {
        !self.keep_empty_shield
            && !(self.shield > 0.0)
            && self.shield_hits == 0
            && self.mods.is_none()
            && !self.has_flags
            && !self.injury
            && !self.observer
            && self.reduction.is_none()
    }
}
struct Entry {
    active: bool,
    effect: Effect,
}
pub struct Effects {
    entries: BTreeMap<EffectId, Entry>,
    next_id: u64,
    depth: usize,
    capacity: usize,
    max_depth: usize,
    max_deferred: usize,
    pub dirty: bool,
}
impl Effects {
    pub fn new(capacity: usize) -> Result<Self, String> {
        Self::with_limits(capacity, 64, capacity)
    }
    pub fn with_limits(
        capacity: usize,
        max_depth: usize,
        max_deferred: usize,
    ) -> Result<Self, String> {
        if capacity == 0 || max_depth == 0 || max_deferred == 0 {
            return Err("Effect limits must be positive".into());
        }
        Ok(Self {
            entries: BTreeMap::new(),
            next_id: 0,
            depth: 0,
            capacity,
            max_depth,
            max_deferred,
            dirty: false,
        })
    }
    pub fn insert(&mut self, effect: Effect) -> Result<EffectId, String> {
        if self.entries.len() >= self.capacity {
            return Err("Effect capacity exceeded".into());
        }
        if !effect.priority.is_finite() || effect.stacks == 0 {
            return Err("Invalid effect priority or stacks".into());
        }
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Effect identity exhausted")?;
        let id = self.next_id;
        self.entries.insert(
            id,
            Entry {
                active: true,
                effect,
            },
        );
        self.dirty = true;
        Ok(id)
    }
    pub fn get(&self, id: EffectId) -> Option<&Effect> {
        self.entries
            .get(&id)
            .filter(|e| e.active)
            .map(|e| &e.effect)
    }
    pub fn get_mut(&mut self, id: EffectId) -> Option<&mut Effect> {
        self.entries
            .get_mut(&id)
            .filter(|e| e.active)
            .map(|e| &mut e.effect)
    }
    pub fn remove(&mut self, id: EffectId) {
        if let Some(e) = self.entries.get_mut(&id) {
            if e.active {
                e.active = false;
                self.dirty = true;
            }
        }
        if self.depth == 0 {
            self.compact();
        }
    }
    pub fn active(&self) -> impl Iterator<Item = (EffectId, &Effect)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.active)
            .map(|(&id, e)| (id, &e.effect))
    }
    fn compact(&mut self) {
        self.entries.retain(|_, e| e.active);
    }
}
/// Content dispatch stays outside the effect pool; effect identity selects the
/// concrete unit/skill callback. No JS callbacks are used by this Rust interface.
pub trait Hooks: Sized {
    fn injury(
        &mut self,
        _id: EffectId,
        _event: &mut InjuryEvent,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn observe(
        &mut self,
        _id: EffectId,
        _event: Observation,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn notice(&mut self, _notice: Notice, _effects: &mut Effects, _rng: &mut Rng) -> HookResult {
        Ok(())
    }
    fn cleanup(&mut self, _token: u64) -> HookResult {
        Ok(())
    }
}
pub struct NoHooks;
impl Hooks for NoHooks {}
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub amount: f64,
    pub cancelled: bool,
    pub outcome: Option<Outcome>,
}
pub struct InjuryEvent {
    pub amount: f64,
    rest: f64,
    cancelled: bool,
    outcome: Option<Outcome>,
    mode: DodgeMode,
    ignore_block: bool,
    deferred: Vec<u64>,
    max_deferred: usize,
}
impl InjuryEvent {
    pub fn defer(&mut self, token: u64) -> HookResult {
        if self.deferred.len() >= self.max_deferred {
            return Err("Deferred injury callback limit exceeded".into());
        }
        self.deferred.push(token);
        Ok(())
    }
    fn succeeds(probability: Option<f64>, rng: &mut Rng) -> bool {
        match probability {
            None => true,
            Some(p) => p.is_finite() && p > 0.0 && rng.next_f64() < p.min(1.0),
        }
    }
    pub fn block<H: Hooks>(
        &mut self,
        p: Option<f64>,
        hooks: &mut H,
        effects: &mut Effects,
        rng: &mut Rng,
    ) -> Result<bool, String> {
        if self.cancelled || !(self.rest > 0.0) || !Self::succeeds(p, rng) {
            return Ok(false);
        }
        self.cancelled = !self.ignore_block;
        if self.cancelled {
            self.rest = 0.0;
            self.outcome = Some(Outcome::Blocked);
        }
        hooks.notice(
            Notice::Block {
                cancelled: self.cancelled,
            },
            effects,
            rng,
        )?;
        Ok(true)
    }
    pub fn dodge<H: Hooks>(
        &mut self,
        p: Option<f64>,
        hooks: &mut H,
        effects: &mut Effects,
        rng: &mut Rng,
    ) -> Result<bool, String> {
        if self.cancelled
            || !(self.rest > 0.0)
            || self.mode == DodgeMode::Ignore
            || !Self::succeeds(p, rng)
        {
            return Ok(false);
        }
        let amount = self.rest;
        self.cancelled = self.mode != DodgeMode::Uncancellable;
        if self.cancelled {
            self.rest = 0.0;
            self.outcome = Some(Outcome::Dodged);
        }
        hooks.notice(
            Notice::Dodge {
                cancelled: self.cancelled,
                amount,
            },
            effects,
            rng,
        )?;
        Ok(true)
    }
}
pub struct InjuryResult {
    pub amount: f64,
    pub cancelled: bool,
    pub outcome: Option<Outcome>,
    pub notify_zero: bool,
    deferred: Vec<u64>,
}
impl InjuryResult {
    pub fn settle<T, H: Hooks>(
        self,
        hooks: &mut H,
        apply: impl FnOnce(f64) -> Result<T, String>,
        zero: T,
    ) -> Result<T, String> {
        let result = if self.cancelled && !self.notify_zero {
            Ok(zero)
        } else {
            apply(self.amount)
        };
        let mut error = None;
        for token in self.deferred {
            if let Err(e) = hooks.cleanup(token) {
                error.get_or_insert(e);
            }
        }
        match result {
            Err(e) => Err(e),
            Ok(value) => {
                if let Some(e) = error {
                    Err(e)
                } else {
                    Ok(value)
                }
            }
        }
    }
}
pub fn execute<H: Hooks>(
    effects: &mut Effects,
    rng: &mut Rng,
    hooks: &mut H,
    amount: f64,
    kind: DamageType,
    mode: DodgeMode,
    ignore_block: bool,
) -> Result<InjuryResult, String> {
    if effects.depth >= effects.max_depth {
        return Err("Nested injury depth exceeded".into());
    }
    effects.depth += 1;
    let result = run(effects, rng, hooks, amount, kind, mode, ignore_block);
    effects.depth -= 1;
    if effects.depth == 0 {
        effects.compact();
    }
    result
}
fn run<H: Hooks>(
    effects: &mut Effects,
    rng: &mut Rng,
    hooks: &mut H,
    amount: f64,
    kind: DamageType,
    mode: DodgeMode,
    ignore_block: bool,
) -> Result<InjuryResult, String> {
    let mut order: Vec<_> = effects
        .active()
        .filter(|(_, e)| e.participates(kind))
        .map(|(id, e)| (id, e.priority))
        .collect();
    order.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    let mut strongest: BTreeMap<String, (EffectId, f64)> = BTreeMap::new();
    for &(id, _) in &order {
        let e = effects.get(id).unwrap();
        let v = e.reduction_value(kind);
        if !v.is_finite() || v <= 0.0 {
            continue;
        }
        let key = e.reduction_group.as_ref().unwrap_or(&e.key).clone();
        if strongest
            .get(&key)
            .map(|&(_, held)| v > held)
            .unwrap_or(true)
        {
            strongest.insert(key, (id, v));
        }
    }
    let reductions: BTreeSet<_> = strongest.values().map(|&(id, _)| id).collect();
    let mut event = InjuryEvent {
        amount,
        rest: amount,
        cancelled: false,
        outcome: if amount > 0.0 {
            None
        } else {
            Some(Outcome::Zero)
        },
        mode,
        ignore_block,
        deferred: vec![],
        max_deferred: effects.max_deferred,
    };
    let mut notify_zero = false;
    for (id, _) in order {
        if effects.get(id).is_none() {
            continue;
        }
        if event.rest > 0.0 && !event.cancelled {
            let e = effects.get(id).unwrap();
            let mult = (if matches!(kind, DamageType::Elemental) {
                1.0
            } else {
                e.multiplier("dmgTakenMul")
            }) * e.multiplier(taken(kind));
            for _ in 0..e.stacks {
                event.rest *= mult;
            }
            if !event.rest.is_finite() || event.rest < 0.0 {
                event.rest = 0.0;
            }
            if reductions.contains(&id) {
                event.rest = (event.rest - e.reduction_value(kind)).max(0.0);
            }
            let absorbs = e
                .shield_types
                .as_ref()
                .map(|types| types.contains(&kind))
                .unwrap_or(true);
            let before = event.rest;
            let mut consumed = false;
            let e = &mut effects.entries.get_mut(&id).unwrap().effect;
            if event.rest > 0.0 && absorbs && e.shield_hits > 0 {
                e.shield_hits -= 1;
                event.rest = 0.0;
                consumed = true;
            } else if event.rest > 0.0 && absorbs && e.shield > 0.0 {
                let take = e.shield.min(event.rest);
                e.shield -= take;
                event.rest -= take;
                if e.shield <= 1e-9 {
                    e.shield = 0.0;
                }
                consumed = true;
            }
            if consumed {
                effects.dirty = true;
            }
            if consumed && !(event.rest > 0.0) {
                if ignore_block {
                    event.rest = before;
                } else {
                    event.cancelled = true;
                    event.rest = 0.0;
                    event.outcome = Some(Outcome::Blocked);
                    notify_zero = true;
                }
                hooks.notice(
                    Notice::Block {
                        cancelled: !ignore_block,
                    },
                    effects,
                    rng,
                )?;
            }
            if consumed && effects.entries[&id].effect.disposable() {
                effects.remove(id);
            }
            if event.rest > 0.0 && effects.entries[&id].effect.injury {
                event.amount = event.rest;
                hooks.injury(id, &mut event, effects, rng)?;
                event.rest = if event.cancelled {
                    0.0
                } else if event.amount.is_finite() {
                    event.amount.max(0.0)
                } else {
                    0.0
                };
            }
            let e = &effects.entries[&id].effect;
            let p = dodge_key(kind)
                .and_then(|key| e.mod_value(key))
                .unwrap_or(0.0);
            let stacks = e.stacks;
            if event.rest > 0.0 && !event.cancelled && p > 0.0 {
                for _ in 0..stacks {
                    if event.dodge(Some(p), hooks, effects, rng)? {
                        break;
                    }
                }
            }
            if !(event.rest > 0.0) && event.outcome.is_none() {
                event.outcome = Some(Outcome::Zero);
            }
        }
        if effects.entries[&id].effect.observer {
            hooks.observe(
                id,
                Observation {
                    amount: event.rest,
                    cancelled: event.cancelled,
                    outcome: event.outcome,
                },
                effects,
                rng,
            )?;
        }
    }
    Ok(InjuryResult {
        amount: event.rest,
        cancelled: event.cancelled,
        outcome: event.outcome,
        notify_zero,
        deferred: event.deferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Nested {
        seen: Vec<EffectId>,
        nesting: bool,
    }
    impl Hooks for Nested {
        fn injury(
            &mut self,
            id: EffectId,
            _: &mut InjuryEvent,
            set: &mut Effects,
            rng: &mut Rng,
        ) -> HookResult {
            if !self.nesting {
                self.nesting = true;
                set.remove(id);
                execute(
                    set,
                    rng,
                    self,
                    2.0,
                    DamageType::True,
                    DodgeMode::Ignore,
                    false,
                )?;
            }
            Ok(())
        }
        fn observe(
            &mut self,
            id: EffectId,
            _: Observation,
            _: &mut Effects,
            _: &mut Rng,
        ) -> HookResult {
            self.seen.push(id);
            Ok(())
        }
    }
    #[test]
    fn nested_damage_keeps_removed_snapshot_alive_until_outer_return() {
        let mut set = Effects::new(4).unwrap();
        let a = set
            .insert(Effect {
                injury: true,
                observer: true,
                ..Effect::default()
            })
            .unwrap();
        let b = set
            .insert(Effect {
                observer: true,
                ..Effect::default()
            })
            .unwrap();
        let mut hooks = Nested::default();
        execute(
            &mut set,
            &mut Rng::new(42),
            &mut hooks,
            1.0,
            DamageType::True,
            DodgeMode::Ignore,
            false,
        )
        .unwrap();
        assert_eq!(hooks.seen, vec![b, a, b]); // removed current observer still runs
        assert!(set.get(a).is_none());
        assert_eq!(set.entries.len(), 1);
        assert_eq!(set.depth, 0);
    }
    struct Deferred {
        cleaned: Vec<u64>,
    }
    impl Hooks for Deferred {
        fn injury(
            &mut self,
            _: EffectId,
            e: &mut InjuryEvent,
            _: &mut Effects,
            _: &mut Rng,
        ) -> HookResult {
            e.defer(1)?;
            e.defer(2)?;
            Ok(())
        }
        fn cleanup(&mut self, t: u64) -> HookResult {
            self.cleaned.push(t);
            if t == 1 {
                Err("cleanup".into())
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn cleanup_continues_after_errors_and_preserves_apply_failure() {
        let mut set = Effects::new(4).unwrap();
        set.insert(Effect {
            injury: true,
            ..Effect::default()
        })
        .unwrap();
        let mut hooks = Deferred { cleaned: vec![] };
        let result = execute(
            &mut set,
            &mut Rng::new(42),
            &mut hooks,
            1.0,
            DamageType::True,
            DodgeMode::Ignore,
            false,
        )
        .unwrap();
        let settled: Result<f64, String> = result.settle(&mut hooks, |_| Err("apply".into()), 0.0);
        assert_eq!(settled.unwrap_err(), "apply");
        assert_eq!(hooks.cleaned, vec![1, 2]);
    }
    #[test]
    fn callback_budget_failure_releases_processing_depth() {
        let mut set = Effects::with_limits(4, 1, 1).unwrap();
        set.insert(Effect {
            injury: true,
            ..Effect::default()
        })
        .unwrap();
        let mut hooks = Deferred { cleaned: vec![] };
        assert!(execute(
            &mut set,
            &mut Rng::new(42),
            &mut hooks,
            1.0,
            DamageType::True,
            DodgeMode::Ignore,
            false
        )
        .is_err());
        assert_eq!(set.depth, 0);
    }
    #[test]
    fn equal_priorities_preserve_acquisition_order_even_for_negative_zero() {
        let mut set = Effects::new(4).unwrap();
        let a = set
            .insert(Effect {
                priority: -0.0,
                observer: true,
                ..Effect::default()
            })
            .unwrap();
        let b = set
            .insert(Effect {
                priority: 0.0,
                observer: true,
                ..Effect::default()
            })
            .unwrap();
        let mut hooks = Nested {
            seen: vec![],
            nesting: true,
        };
        execute(
            &mut set,
            &mut Rng::new(42),
            &mut hooks,
            1.0,
            DamageType::True,
            DodgeMode::Ignore,
            false,
        )
        .unwrap();
        assert_eq!(hooks.seen, vec![a, b]);
    }
}
