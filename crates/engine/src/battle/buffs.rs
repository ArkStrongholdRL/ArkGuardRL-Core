// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/status.js: canonical modifier lifetimes.
// Stable handles retain removed records only for the current synchronous call tree.
use super::{
    health::{self, HealOptions, Id, World},
    stats::Buff,
};
use std::collections::BTreeMap;
pub type BuffId = u64;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Refresh {
    #[default]
    Replace,
    Extend,
    Stack,
    Independent,
    Keep,
}
#[derive(Clone, Debug)]
pub struct State {
    pub(crate) id: BuffId,
    pub tick_count: u64,
    pub tick_payload: f64,
    pub(crate) enemy_effect: Option<super::content::enemies::casters::BuffEffect>,
    pub duration: f64,
    pub time_left: f64,
    pub max_stacks: u32,
    pub refresh: Refresh,
    pub visible: bool,
    pub status: Option<String>,
    pub source: Option<Id>,
    retained_unit: Option<Id>,
    pub persist: bool,
    pub interval: f64,
    pub accumulator: f64,
    pub shield_hits: u32,
    pub on_tick: Option<u64>,
    pub on_remove: Option<u64>,
    pub on_expire: Option<u64>,
    pub valued: Option<super::valued::Effect>,
    pub valued_expire: bool,
    pub resist_decay: bool,
    pub attract: Option<[i32; 2]>,
    pub attract_path: Option<super::motion::Attract>,
    pub fear: Option<super::status::FearStamp>,
    pub element_burst: Option<super::element::Burst>,
    pub injury: Injury,
    pub mods_present: bool,
    pub flags_present: bool,
    pub stealth_restore: Option<f64>,
}
impl State {
    pub(crate) fn retained_unit(&self) -> Option<Id> {
        self.retained_unit
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            id: 0,
            tick_count: 0,
            tick_payload: 0.,
            enemy_effect: None,
            duration: f64::INFINITY,
            time_left: f64::INFINITY,
            max_stacks: 1,
            refresh: Refresh::Replace,
            visible: false,
            status: None,
            source: None,
            retained_unit: None,
            persist: false,
            interval: 0.0,
            accumulator: 0.0,
            shield_hits: 0,
            on_tick: None,
            on_remove: None,
            on_expire: None,
            valued: None,
            valued_expire: false,
            resist_decay: false,
            attract: None,
            attract_path: None,
            fear: None,
            element_burst: None,
            injury: Injury::default(),
            mods_present: false,
            flags_present: false,
            stealth_restore: None,
        }
    }
}
impl State {
    pub fn id(&self) -> BuffId {
        self.id
    }
}
#[derive(Clone, Debug, Default)]
pub struct Injury {
    pub priority: f64,
    pub reduction: Option<[f64; 4]>,
    pub reduction_group: Option<String>,
    pub shield_types: Option<Vec<super::damage::DamageType>>,
    pub keep_empty_shield: bool,
    pub on_injury: Option<u64>,
    pub observe: Option<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct Spec {
    pub buff: Buff,
    pub mods_present: bool,
    pub flags_present: bool,
    pub allow_dead: bool,
}
impl Spec {
    /// Keep a referenced unit available for this effect's complete attached lifetime.
    pub(crate) fn retaining_unit(mut self, id: Id) -> Self {
        self.buff.runtime.retained_unit = Some(id);
        self
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Tick,
    Remove,
    Expire,
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub unit: Id,
    pub buff: BuffId,
    pub dt: f64,
}
pub trait Hooks: health::Hooks {
    fn field_deploy(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _id: Id,
        _fresh: bool,
    ) -> Result<(), String> {
        Ok(())
    }

    fn operator_removed(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _id: Id,
    ) -> Result<(), String> {
        Ok(())
    }
    fn heal_with_buffs(
        &mut self,
        w: &mut World,
        _r: &mut Runtime,
        source: Option<Id>,
        target: Id,
        amount: f64,
        options: HealOptions,
    ) -> Result<f64, String>
    where
        Self: Sized,
    {
        health::heal(w, self, source, target, amount, options)
    }

    fn operator_skill_effect(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _dir: super::direction::Direction,
        _definition: &super::operators::Definition,
        _event: super::skills::Event,
        _ctx: &mut super::skills::Context,
        _skill: &mut super::skills::Runtime,
    ) -> Result<(), String> {
        Ok(())
    }

    fn operator_target_status(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _source: Id,
        _target: Id,
        _key: &str,
        _duration: f64,
    ) -> Result<(), String> {
        Err("Target status handler not installed".into())
    }
    fn operator_release(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _id: Id,
    ) -> Result<(), String> {
        Err("Operator release handler not installed".into())
    }
    fn operator_status(
        &mut self,
        _w: &mut World,
        _r: &mut Runtime,
        _id: Id,
        _key: &str,
        _duration: f64,
    ) -> Result<(), String> {
        Err("Operator control status handler not installed".into())
    }
    fn burst_damage(
        &mut self,
        _request: &mut super::hit::Request,
        _runtime: &mut Runtime,
        _w: &mut World,
    ) -> Result<f64, String> {
        Err("Element burst damage handler not installed".into())
    }
    fn burst_sp_loss(&mut self, _unit: Id, _amount: f64, _w: &mut World) -> Result<(), String> {
        Err("Element burst SP handler not installed".into())
    }
    fn status(&mut self, unit: Id, key: &str, on: bool) -> Result<(), String>;
    fn callback(
        &mut self,
        token: u64,
        event: Event,
        c: Context,
        runtime: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String>;
}
pub struct Runtime {
    sequence: u64,
    depth: usize,
    max_depth: usize,
    max_buffs: usize,
    max_retained: usize,
    retained: BTreeMap<BuffId, (Id, Buff)>,
    faulted: bool,
    fear_sequence: u64,
}
impl Runtime {
    pub fn new(max_buffs: usize, max_retained: usize, max_depth: usize) -> Result<Self, String> {
        if [max_buffs, max_retained, max_depth].contains(&0) {
            return Err("Buff limits must be positive".into());
        }
        Ok(Self {
            sequence: 0,
            depth: 0,
            max_depth,
            max_buffs,
            max_retained,
            retained: BTreeMap::new(),
            faulted: false,
            fear_sequence: 0,
        })
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    pub(crate) fn deferred_limit(&self) -> usize {
        self.max_retained
    }
    pub(crate) fn next_fear(&mut self) -> Result<u64, String> {
        self.fear_sequence = self
            .fear_sequence
            .checked_add(1)
            .ok_or("Fear identity exhausted")?;
        Ok(self.fear_sequence)
    }
    pub(crate) fn check_tail(&self, len: usize) -> Result<(), String> {
        if len > self.max_buffs {
            Err("Waiting status capacity exceeded; no effects dropped".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn call<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.faulted {
            return Err("Buff runtime is faulted".into());
        }
        if self.depth >= self.max_depth {
            self.faulted = true;
            return Err("Buff callback depth exhausted".into());
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        if result.is_err() {
            self.faulted = true;
        }
        if self.depth == 0 {
            self.retained.clear();
        }
        match result {
            Ok(_) if self.faulted => Err("Buff callback suppressed a technical failure".into()),
            other => other,
        }
    }
    fn next(&mut self) -> Result<BuffId, String> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Buff identity exhausted")?;
        Ok(self.sequence)
    }
    pub(crate) fn adopt(&mut self, w: &mut World, unit: Id) -> Result<(), String> {
        let s = &mut w.unit_mut(unit)?.stats;
        if s.buffs().len() > self.max_buffs {
            return Err("Unit buff capacity exceeded".into());
        }
        for b in s.buffs_mut() {
            if b.runtime.id == 0 {
                b.runtime.id = self.next()?;
                b.runtime.persist |= b.permanent;
            }
        }
        Ok(())
    }
    pub fn get<'a>(&'a self, w: &'a World, unit: Id, id: BuffId) -> Result<&'a Buff, String> {
        if let Some(b) = w
            .unit(unit)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.runtime.id == id)
        {
            return Ok(b);
        }
        self.retained
            .get(&id)
            .filter(|(u, _)| *u == unit)
            .map(|(_, b)| b)
            .ok_or_else(|| "Unknown buff identity".into())
    }
    pub fn edit(
        &mut self,
        w: &mut World,
        unit: Id,
        id: BuffId,
        f: impl FnOnce(&mut Buff),
    ) -> Result<(), String> {
        self.edit_inner(w, unit, id, true, f)
    }
    pub(crate) fn edit_timer(
        &mut self,
        w: &mut World,
        unit: Id,
        id: BuffId,
        f: impl FnOnce(&mut Buff),
    ) -> Result<(), String> {
        self.edit_inner(w, unit, id, false, f)
    }
    fn edit_inner(
        &mut self,
        w: &mut World,
        unit: Id,
        id: BuffId,
        dirty: bool,
        f: impl FnOnce(&mut Buff),
    ) -> Result<(), String> {
        let s = &mut w.unit_mut(unit)?.stats;
        if let Some(b) = s.buffs_mut().iter_mut().find(|b| b.runtime.id == id) {
            f(b);
            b.permanent = b.runtime.persist && b.runtime.duration == f64::INFINITY;
            if dirty {
                s.mark_dirty();
            }
            return Ok(());
        }
        let (_, b) = self
            .retained
            .get_mut(&id)
            .filter(|(u, _)| *u == unit)
            .ok_or("Unknown buff identity")?;
        f(b);
        Ok(())
    }
    fn attached(w: &World, unit: Id, id: BuffId) -> Result<bool, String> {
        Ok(w.unit(unit)?
            .stats
            .buffs()
            .iter()
            .any(|b| b.runtime.id == id))
    }
    pub fn add<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        spec: Spec,
    ) -> Result<Option<BuffId>, String> {
        self.call(|r| r.add_inner(w, h, unit, spec))
    }
    fn add_inner<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        spec: Spec,
    ) -> Result<Option<BuffId>, String> {
        if !w.unit(unit)?.stats.alive && !spec.allow_dead {
            return Ok(None);
        }
        self.adopt(w, unit)?;
        let mut b = spec.buff;
        b.runtime.mods_present = spec.mods_present || !b.mods.is_empty();
        b.runtime.flags_present = spec.flags_present || !b.flags.is_empty();
        if !b.runtime.injury.priority.is_finite() {
            b.runtime.injury.priority = 0.0;
        }
        if b.stacks == 0
            || b.stacks > i32::MAX as u32
            || b.runtime.max_stacks == 0
            || b.runtime.max_stacks > i32::MAX as u32
        {
            return Err("Buff stack capacity outside positive i32 domain".into());
        }
        b.key.get_or_insert_with(|| "buff".into());
        if !b.runtime.duration.is_finite() && b.runtime.duration != f64::INFINITY {
            b.runtime.duration = f64::INFINITY;
        }
        b.runtime.time_left = b.runtime.duration;
        b.runtime.accumulator = 0.0;
        b.runtime.id = self.next()?;
        b.permanent = b.runtime.persist && b.runtime.duration == f64::INFINITY;
        let index = w
            .unit(unit)?
            .stats
            .buffs()
            .iter()
            .position(|old| old.key == b.key);
        if let Some(index) = index.filter(|_| b.runtime.refresh != Refresh::Independent) {
            if b.runtime.refresh == Refresh::Replace {
                if self.retained.len() >= self.max_retained {
                    return Err("Retained buff capacity exceeded".into());
                }
                if let Some(id) = b.runtime.retained_unit {
                    w.pin_unit(id)?;
                }
            }
            let s = &mut w.unit_mut(unit)?.stats;
            let old = &mut s.buffs_mut()[index];
            let id = old.runtime.id;
            match b.runtime.refresh {
                Refresh::Keep => return Ok(Some(id)),
                Refresh::Stack => {
                    let cap = old.runtime.max_stacks.max(b.runtime.max_stacks);
                    old.stacks = (old.stacks as u64 + b.stacks as u64).min(cap as u64) as u32;
                    old.runtime.max_stacks = cap;
                    old.runtime.time_left = b.runtime.duration;
                    old.runtime.duration = b.runtime.duration;
                    if spec.mods_present {
                        old.mods = b.mods;
                        old.runtime.mods_present = true;
                    }
                    if spec.flags_present {
                        old.flags = b.flags;
                        old.runtime.flags_present = true;
                    }
                }
                Refresh::Extend => {
                    old.runtime.time_left = js_max(old.runtime.time_left, b.runtime.duration);
                    old.runtime.duration = js_max(old.runtime.duration, b.runtime.duration);
                    if spec.mods_present {
                        old.mods = b.mods;
                        old.runtime.mods_present = true;
                    }
                    if spec.flags_present {
                        old.flags = b.flags;
                        old.runtime.flags_present = true;
                    }
                    if b.shield > old.shield {
                        old.shield = b.shield;
                    }
                    if b.runtime.shield_hits > old.runtime.shield_hits {
                        old.runtime.shield_hits = b.runtime.shield_hits;
                    }
                }
                _ => {
                    let removed = std::mem::replace(old, b);
                    let retained_unit = removed.runtime.retained_unit;
                    self.retained.insert(id, (unit, removed));
                    s.mark_dirty();
                    let installed = s.buffs()[index].runtime.id;
                    if let Some(id) = retained_unit {
                        w.unpin_unit(id)?;
                    }
                    return Ok(Some(installed));
                }
            }
            old.permanent = old.runtime.persist && old.runtime.duration == f64::INFINITY;
            s.mark_dirty();
            return Ok(Some(id));
        }
        if b.runtime.refresh == Refresh::Independent {
            let same: Vec<_> = w
                .unit(unit)?
                .stats
                .buffs()
                .iter()
                .filter(|old| old.key == b.key)
                .map(|old| old.runtime.id)
                .collect();
            if same.len() >= b.runtime.max_stacks as usize {
                if let Some(oldest) = same.into_iter().min() {
                    self.remove_one(w, h, unit, oldest, true)?;
                }
            }
        }
        // onRemove may add another instance; enforce capacity again after that callback.
        if w.unit(unit)?.stats.buffs().len() >= self.max_buffs {
            return Err("Unit buff capacity exceeded; no effects dropped".into());
        }
        if let Some(id) = b.runtime.retained_unit {
            w.pin_unit(id)?;
        }
        let s = &mut w.unit_mut(unit)?.stats;
        let id = b.runtime.id;
        let key = status_key(&b);
        let visible = b.runtime.visible || b.runtime.status.is_some();
        s.buffs_mut().push(b);
        s.mark_dirty();
        if visible && s.buffs().iter().filter(|b| status_key(b) == key).count() == 1 {
            h.status(unit, &key, true)?;
        }
        Ok(Some(id))
    }
    pub fn remove<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        key: &str,
    ) -> Result<usize, String> {
        self.call(|r| {
            r.adopt(w, unit)?;
            let ids: Vec<_> = w
                .unit(unit)?
                .stats
                .buffs()
                .iter()
                .rev()
                .filter(|b| b.key.as_deref() == Some(key))
                .map(|b| b.runtime.id)
                .collect();
            let mut n = 0;
            for id in ids {
                n += r.remove_one(w, h, unit, id, true)? as usize;
            }
            Ok(n)
        })
    }
    pub fn remove_id<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        id: BuffId,
    ) -> Result<bool, String> {
        self.call(|r| r.remove_one(w, h, unit, id, true))
    }
    fn remove_one<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        id: BuffId,
        call_remove: bool,
    ) -> Result<bool, String> {
        let Some(index) = w
            .unit(unit)?
            .stats
            .buffs()
            .iter()
            .position(|b| b.runtime.id == id)
        else {
            return Ok(false);
        };
        if self.retained.len() >= self.max_retained {
            return Err("Retained buff capacity exceeded".into());
        }
        let s = &mut w.unit_mut(unit)?.stats;
        let b = s.buffs_mut().remove(index);
        s.mark_dirty();
        let key = status_key(&b);
        let off = (b.runtime.visible || b.runtime.status.is_some())
            && !s.buffs().iter().any(|b| status_key(b) == key);
        let token = b.runtime.on_remove;
        let retained_unit = b.runtime.retained_unit;
        self.retained.insert(id, (unit, b));
        if let Some(id) = retained_unit {
            w.unpin_unit(id)?;
        }
        if off {
            h.status(unit, &key, false)?;
        }
        if call_remove && self.get(w, unit, id)?.runtime.element_burst.is_some() {
            w.unit_mut(unit)?.element_buildup = [0.0; 5];
        }
        if call_remove {
            if let Some(token) = token {
                h.callback(
                    token,
                    Event::Remove,
                    Context {
                        unit,
                        buff: id,
                        dt: 0.0,
                    },
                    self,
                    w,
                )?;
            }
        }
        Ok(true)
    }
    /// Deployment removal keeps persistent effects without running onRemove or
    /// elemental-expiry callbacks (battle/deploy.js). Detached effects remain
    /// accessible to an enclosing callback until the outer dispatch completes.
    pub fn discard_temporary<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
    ) -> Result<(), String> {
        self.call(|r| {
            r.adopt(w, unit)?;
            let ids: Vec<_> = w
                .unit(unit)?
                .stats
                .buffs()
                .iter()
                .filter(|b| !b.runtime.persist)
                .map(|b| b.runtime.id)
                .collect();
            for id in ids {
                r.remove_one(w, h, unit, id, false)?;
            }
            Ok(())
        })
    }
    pub fn tick<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        units: &[Id],
        dt: f64,
    ) -> Result<(), String> {
        self.call(|r| {
            if !dt.is_finite() || dt < 0.0 {
                return Err("Invalid buff tick duration".into());
            }
            if units.len() > w.unit_capacity() {
                return Err("Buff unit snapshot capacity exceeded".into());
            }
            for &unit in units {
                let u = w.unit(unit)?;
                if !u.stats.alive || u.removed {
                    continue;
                }
                r.adopt(w, unit)?;
                let snapshot: Vec<_> = w
                    .unit(unit)?
                    .stats
                    .buffs()
                    .iter()
                    .map(|b| b.runtime.id)
                    .collect();
                for id in snapshot {
                    if !w.unit(unit)?.stats.alive {
                        break;
                    }
                    if !Self::attached(w, unit, id)? {
                        continue;
                    }
                    let b = r.get(w, unit, id)?;
                    if b.runtime.on_tick.is_some()
                        || b.runtime.resist_decay
                        || b.runtime
                            .element_burst
                            .map(|b| b.periodic())
                            .unwrap_or(false)
                    {
                        if b.runtime.interval > 0.0 {
                            r.edit_timer(w, unit, id, |b| b.runtime.accumulator += dt)?;
                            for _ in 0..8 {
                                let b = r.get(w, unit, id)?;
                                if !(b.runtime.accumulator >= b.runtime.interval - 1e-9) {
                                    break;
                                }
                                let interval = b.runtime.interval;
                                r.edit_timer(w, unit, id, |b| b.runtime.accumulator -= interval)?;
                                r.fire_tick(w, h, unit, id, interval)?;
                            }
                        } else {
                            r.fire_tick(w, h, unit, id, dt)?;
                        }
                    }
                    if r.get(w, unit, id)?.runtime.time_left != f64::INFINITY {
                        r.edit_timer(w, unit, id, |b| b.runtime.time_left -= dt)?;
                        if r.get(w, unit, id)?.runtime.time_left <= 1e-9
                            && Self::attached(w, unit, id)?
                        {
                            r.remove_one(w, h, unit, id, false)?;
                            if r.get(w, unit, id)?.runtime.element_burst.is_some() {
                                w.unit_mut(unit)?.element_buildup = [0.0; 5];
                            }
                            if r.get(w, unit, id)?.runtime.valued_expire {
                                super::valued::expire(w, h, r, unit, id)?;
                            }
                            if let Some(token) = r.get(w, unit, id)?.runtime.on_expire {
                                h.callback(
                                    token,
                                    Event::Expire,
                                    Context {
                                        unit,
                                        buff: id,
                                        dt: 0.0,
                                    },
                                    r,
                                    w,
                                )?;
                            }
                        }
                    }
                }
                let u = w.unit_mut(unit)?;
                if u.stats.alive && u.deployed && u.pool.is_none() {
                    let s = u.stats.stats().map_err(str::to_string)?;
                    let regen = s.n("hpRegen");
                    let max = s.n("maxHp");
                    if regen > 0.0 && u.stats.hp < max {
                        u.regen_acc += regen * dt;
                        if u.regen_acc >= 1.0 || u.stats.hp + u.regen_acc >= max {
                            let amount = u.regen_acc;
                            h.heal_with_buffs(
                                w,
                                r,
                                Some(unit),
                                unit,
                                amount,
                                HealOptions {
                                    self_heal: true,
                                    silent: true,
                                    regen: true,
                                    ..Default::default()
                                },
                            )?;
                            w.unit_mut(unit)?.regen_acc = 0.0;
                        }
                    }
                }
            }
            Ok(())
        })
    }
    fn fire_tick<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        unit: Id,
        id: BuffId,
        dt: f64,
    ) -> Result<(), String> {
        if self
            .get(w, unit, id)?
            .runtime
            .element_burst
            .map(|b| b.periodic())
            .unwrap_or(false)
        {
            super::element::tick(w, h, self, unit, id)
        } else if self.get(w, unit, id)?.runtime.resist_decay {
            super::valued::decay_palsy(w, h, self, unit)
        } else {
            let token = self
                .get(w, unit, id)?
                .runtime
                .on_tick
                .ok_or("Buff tick callback removed during interval loop")?;
            let count = self
                .get(w, unit, id)?
                .runtime
                .tick_count
                .checked_add(1)
                .ok_or("Buff tick count exhausted")?;
            self.edit_timer(w, unit, id, |b| b.runtime.tick_count = count)?;
            h.callback(token, Event::Tick, Context { unit, buff: id, dt }, self, w)
        }
    }
}
fn status_key(b: &Buff) -> String {
    b.runtime
        .status
        .clone()
        .or_else(|| b.key.clone())
        .unwrap_or_else(|| "buff".into())
}
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
