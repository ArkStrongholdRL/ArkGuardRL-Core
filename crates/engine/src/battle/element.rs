// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 attack/element.js: gauges, source-less bursts and recovery.
use super::{
    buffs::{self, Refresh, Runtime, Spec},
    damage::{Damage, DamageType, Penetration},
    grid::Rect,
    health::{Hit, Id, World},
    hit::{self, AttackType},
    stats::Buff,
    status::{self, Options},
    targeting,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Element {
    Burn,
    Neural,
    Necrosis,
    Apoptosis,
    Erosion,
}
impl Element {
    pub fn name(self) -> &'static str {
        match self {
            Self::Burn => "burn",
            Self::Neural => "neural",
            Self::Necrosis => "necrosis",
            Self::Apoptosis => "apoptosis",
            Self::Erosion => "erosion",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "burn" => Some(Self::Burn),
            "neural" => Some(Self::Neural),
            "necrosis" => Some(Self::Necrosis),
            "apoptosis" => Some(Self::Apoptosis),
            "erosion" => Some(Self::Erosion),
            _ => None,
        }
    }
}
/// Shared damage metadata survives an elementHit hook converting a gauge request to HP damage.
pub struct Request {
    pub damage: hit::Request,
    pub convert_to_hp: Option<DamageType>,
}
impl Request {
    pub fn new(
        source: Option<Id>,
        credit: Option<Id>,
        target: Id,
        element: Element,
        amount: f64,
    ) -> Self {
        let mut damage = hp_request(
            credit,
            target,
            element,
            amount,
            DamageType::Elemental,
            false,
            false,
        );
        damage.tags.clear();
        damage.hit.source = source;
        Self {
            damage,
            convert_to_hp: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Hit,
    ModifierApplied,
    Damaged,
    Applied,
    Burst,
}
pub trait Hooks: status::Hooks {
    fn battle_rect(&self) -> Rect;
    fn element_enabled(&self, phase: Phase) -> bool;
    fn element_event(
        &mut self,
        phase: Phase,
        request: &mut Request,
        amount: f64,
        gauge_added: f64,
        r: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String>;
    fn element_number(&mut self, target: Id, amount: f64, element: Element) -> Result<(), String>;
    fn burst_visual(&mut self, target: Id, element: Element, w: &World) -> Result<(), String>;
}
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    pub element: Element,
    pub credit: Option<Id>,
    pub enemy: bool,
}
impl Burst {
    pub fn periodic(self) -> bool {
        matches!(self.element, Element::Necrosis | Element::Apoptosis)
    }
}
pub fn gauge_max(w: &World, id: Id) -> Result<f64, String> {
    let u = w.unit(id)?;
    Ok(if u.enemy_entity && (u.boss || u.element_leader) {
        2000.0
    } else {
        1000.0
    })
}
pub fn locked(w: &mut World, id: Id) -> Result<bool, String> {
    let u = w.unit_mut(id)?;
    if u.burst_pending.iter().any(|p| *p) {
        return Ok(true);
    }
    Ok(u.stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("burstLock"))
}
pub fn intake(w: &mut World, id: Id) -> Result<f64, String> {
    let u = w.unit_mut(id)?;
    let resistance = if u.ep_resistance.is_nan() {
        0.0
    } else {
        u.ep_resistance
    };
    let frac = clamp(resistance / 100.0);
    Ok(u.stats.stats().map_err(str::to_string)?.n("elemTakenMul") * (1.0 - frac).max(0.05))
}
pub fn apply<H: Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    request: &mut Request,
) -> Result<f64, String> {
    r.call(|r| {
        let target = request.damage.hit.target;
        let source = request.damage.hit.source;
        let credit = request.damage.hit.credit;
        let Some(element) = request.damage.element else {
            return Ok(0.0);
        };
        let index = element as usize;
        let t = w.unit(target)?;
        let hp = t
            .pool
            .and_then(|p| w.pool(p).map(|p| p.hp))
            .unwrap_or(t.stats.hp);
        if !t.stats.alive || !(hp > 0.0) {
            return Ok(0.0);
        }
        let f = w
            .unit_mut(target)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .clone();
        if f.contains("invulnerable")
            || f.contains("sleep")
                && !request.damage.ignore_sleep
                && !source
                    .map(|id| w.unit(id).map(|u| u.hits_sleep))
                    .transpose()?
                    .unwrap_or(false)
        {
            return Ok(0.0);
        }
        if !request.damage.ignore_select && targeting::evades_ground(w, source, target)? {
            return Ok(0.0);
        }
        if locked(w, target)? {
            return Ok(0.0);
        }
        if h.element_enabled(Phase::Hit) {
            h.element_event(Phase::Hit, request, 0.0, 0.0, r, w)?;
            let t = w.unit(target)?;
            if request.damage.cancelled || !t.stats.alive || !t.deployed || locked(w, target)? {
                return Ok(0.0);
            }
            if let Some(kind) = request.convert_to_hp {
                request.damage.hit.target = target;
                request.damage.hit.source = source;
                request.damage.hit.credit = credit;
                request.damage.hit.damage_type = kind;
                return h.burst_damage(&mut request.damage, r, w);
            }
        }
        let max = gauge_max(w, target)?;
        let amount = request.damage.damage.amount * request.damage.damage.mul * intake(w, target)?;
        if !(amount > 0.0) || !amount.is_finite() {
            return Ok(0.0);
        }
        let before = w.unit(target)?.element_buildup[index];
        w.unit_mut(target)?.element_buildup[index] = (before + amount).min(max);
        h.element_number(target, amount, element)?;
        if let Some(credit) = credit {
            w.unit_mut(credit)?.element_damage_dealt += amount;
        }
        let added = w.unit(target)?.element_buildup[index] - before;
        for phase in [Phase::ModifierApplied, Phase::Damaged, Phase::Applied] {
            if h.element_enabled(phase) {
                h.element_event(
                    phase,
                    request,
                    amount,
                    if phase == Phase::ModifierApplied {
                        added
                    } else {
                        0.0
                    },
                    r,
                    w,
                )?;
            }
        }
        if w.unit(target)?.element_buildup[index] >= max && w.unit(target)?.stats.alive {
            burst(w, h, r, source, credit, target, element)?;
        }
        Ok(amount)
    })
}
pub fn burst<H: Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    source: Option<Id>,
    credit: Option<Id>,
    target: Id,
    element: Element,
) -> Result<(), String> {
    r.call(|r| {
        if !w.unit(target)?.stats.alive || locked(w, target)? {
            return Ok(());
        }
        w.unit_mut(target)?.burst_pending[element as usize] = true;
        let result = resolve_burst(w, h, r, source, credit, target, element);
        w.unit_mut(target)?.burst_pending[element as usize] = false;
        result
    })
}
fn resolve_burst<H: Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    source: Option<Id>,
    credit: Option<Id>,
    target: Id,
    element: Element,
) -> Result<(), String> {
    w.unit_mut(target)?.element_buildup[element as usize] = gauge_max(w, target)?;
    h.burst_visual(target, element, w)?;
    if h.element_enabled(Phase::Burst) {
        h.element_event(
            Phase::Burst,
            &mut Request::new(source, credit, target, element, 0.0),
            0.0,
            0.0,
            r,
            w,
        )?;
    }
    if !w.unit(target)?.stats.alive {
        w.unit_mut(target)?.element_buildup = [0.0; 5];
        return Ok(());
    }
    let enemy = w.unit(target)?.enemy_entity;
    let duration = match element {
        Element::Apoptosis => 15.0,
        Element::Necrosis => 12.0,
        Element::Erosion if enemy => 8.0,
        _ => 10.0,
    };
    let state = Burst {
        element,
        credit,
        enemy,
    };
    let mut b = Buff {
        key: Some(format!("{}Burst", element.name())),
        flags: ["burstLock".into()].into_iter().collect(),
        ..Default::default()
    };
    b.runtime.duration = duration;
    b.runtime.visible = true;
    b.runtime.element_burst = Some(state);
    match element {
        Element::Burn => {
            b.mods.insert("resFlat".into(), -20.0);
        }
        Element::Necrosis => {
            b.mods.insert("atkMul".into(), 0.8);
            b.runtime.interval = 1.0;
        }
        Element::Apoptosis => {
            b.runtime.interval = 1.0;
            if enemy {
                b.mods.insert("atkMul".into(), 0.5);
            } else {
                b.flags.extend(["silence".into(), "noSp".into()]);
            }
        }
        _ => {}
    }
    r.add(
        w,
        h,
        target,
        Spec {
            mods_present: !b.mods.is_empty(),
            flags_present: true,
            buff: b,
            ..Default::default()
        },
    )?;
    match element {
        Element::Burn => {
            h.burst_damage(
                &mut hp_request(
                    credit,
                    target,
                    element,
                    if enemy { 7000.0 } else { 1200.0 },
                    if enemy {
                        DamageType::Elemental
                    } else {
                        DamageType::Arts
                    },
                    false,
                    false,
                ),
                r,
                w,
            )?;
        }
        Element::Neural => {
            let rect = h.battle_rect();
            status::apply(
                w,
                h,
                r,
                rect,
                target,
                if enemy { "palsy" } else { "stun" },
                Options {
                    duration: if enemy { f64::INFINITY } else { 10.0 },
                    value: if enemy { Some(3.0) } else { None },
                    source,
                    force: !enemy,
                    ..Default::default()
                },
            )?;
            if w.unit(target)?.stats.alive {
                h.burst_damage(
                    &mut hp_request(
                        credit,
                        target,
                        element,
                        if enemy { 6000.0 } else { 1000.0 },
                        if enemy {
                            DamageType::Elemental
                        } else {
                            DamageType::True
                        },
                        false,
                        false,
                    ),
                    r,
                    w,
                )?;
            }
        }
        Element::Erosion => {
            let mut b = Buff {
                key: Some("erosionDown".into()),
                mods: [("defFlat".into(), if enemy { -120.0 } else { -100.0 })]
                    .into_iter()
                    .collect(),
                ..Default::default()
            };
            b.runtime.refresh = Refresh::Stack;
            b.runtime.max_stacks = 1_000_000;
            b.runtime.visible = true;
            r.add(
                w,
                h,
                target,
                Spec {
                    buff: b,
                    mods_present: true,
                    ..Default::default()
                },
            )?;
            h.burst_damage(
                &mut hp_request(
                    credit,
                    target,
                    element,
                    if enemy { 5000.0 } else { 800.0 },
                    if enemy {
                        DamageType::Elemental
                    } else {
                        DamageType::Physical
                    },
                    false,
                    false,
                ),
                r,
                w,
            )?;
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn tick<H: buffs::Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    unit: Id,
    id: buffs::BuffId,
) -> Result<(), String> {
    let b = r.get(w, unit, id)?;
    let burst = b
        .runtime
        .element_burst
        .ok_or("Missing elemental burst state")?;
    let (amount, kind, no_sp) = match burst.element {
        Element::Necrosis => (100.0, DamageType::True, false),
        Element::Apoptosis if burst.enemy => {
            let left = b.runtime.time_left;
            let mul = 1.0 - 0.5 * left.max(0.0) / 15.0;
            r.edit(w, unit, id, |b| {
                b.mods = [("atkMul".into(), mul)].into_iter().collect();
            })?;
            (800.0, DamageType::Elemental, true)
        }
        Element::Apoptosis => {
            h.burst_sp_loss(unit, 1.0, w)?;
            (100.0, DamageType::Arts, true)
        }
        _ => return Ok(()),
    };
    h.burst_damage(
        &mut hp_request(burst.credit, unit, burst.element, amount, kind, no_sp, true),
        r,
        w,
    )?;
    Ok(())
}
pub fn heal(
    w: &mut World,
    target: Id,
    amount: f64,
    element: Option<Element>,
) -> Result<f64, String> {
    if locked(w, target)? || !(amount > 0.0) {
        return Ok(0.0);
    }
    let mut removed = 0.0;
    for i in 0..5 {
        if element.map(|e| e as usize != i).unwrap_or(false) {
            continue;
        }
        let n = w.unit(target)?.element_buildup[i];
        let take = if n.is_nan() { 0.0 } else { n }.min(amount);
        if take > 0.0 {
            w.unit_mut(target)?.element_buildup[i] -= take;
            removed += take;
        }
    }
    Ok(removed)
}
/// Loss modifies total stored SP directly; no gain hook, and a running timed
/// skill keeps the SP state it had after casting, as SkillRuntime.setSpTotal does.
pub fn drain_sp(
    w: &mut World,
    bank: Option<&mut super::sp::Bank>,
    amount: f64,
) -> Result<(), String> {
    if !amount.is_finite() || amount < 0.0 {
        return Err("Invalid elemental SP loss".into());
    }
    let Some(bank) = bank else {
        return Ok(());
    };
    if !bank.no_skill
        && bank.kind != super::sp::Kind::Passive
        && !(bank.active && bank.kind.timed())
    {
        let total = bank.total(w)?;
        if total > 0.0 {
            bank.set_total(w, (total - amount).max(0.0))?;
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub element: Element,
    pub fill: f64,
    pub cooldown_end: f64,
    pub cooldown: f64,
}
pub fn view(w: &mut World, target: Id, now: f64) -> Result<Option<View>, String> {
    let u = w.unit_mut(target)?;
    if u.stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("burstLock")
    {
        let mut best = None;
        let mut left = 0.0;
        for b in u.stats.buffs() {
            if b.flags.contains("burstLock") && (best.is_none() || b.runtime.time_left > left) {
                best = Some(b);
                left = b.runtime.time_left;
            }
        }
        if let Some(b) = best {
            let element = b
                .key
                .as_deref()
                .and_then(|k| k.strip_suffix("Burst"))
                .and_then(Element::parse)
                .unwrap_or(Element::Burn);
            let duration = if b.runtime.duration.is_finite() && b.runtime.duration > 0.0 {
                b.runtime.duration
            } else {
                0.0
            };
            let left = if b.runtime.time_left.is_finite() {
                b.runtime.time_left.max(0.0)
            } else {
                0.0
            };
            return Ok(Some(View {
                element,
                fill: 1.0,
                cooldown_end: round((now + left) * 100.0) / 100.0,
                cooldown: round(duration * 100.0) / 100.0,
            }));
        }
    }
    let mut best = None;
    let mut value = 0.0;
    for element in [
        Element::Neural,
        Element::Erosion,
        Element::Burn,
        Element::Apoptosis,
        Element::Necrosis,
    ] {
        if u.element_buildup[element as usize] > value {
            value = u.element_buildup[element as usize];
            best = Some(element);
        }
    }
    let Some(element) = best else {
        return Ok(None);
    };
    let max = gauge_max(w, target)?;
    let fill = ((value / max * 100.0 + 1e-9).floor() / 100.0).clamp(0.01, 0.99);
    Ok(Some(View {
        element,
        fill,
        cooldown_end: 0.0,
        cooldown: 0.0,
    }))
}
fn hp_request(
    credit: Option<Id>,
    target: Id,
    element: Element,
    amount: f64,
    kind: DamageType,
    no_sp: bool,
    periodic: bool,
) -> hit::Request {
    let mut r = hit::Request::new(
        Hit {
            target,
            source: None,
            credit,
            damage_type: kind,
            no_sp,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount,
            mul: 1.0,
            penetration: Penetration::default(),
            resolved: false,
        },
    );
    r.element = Some(element);
    r.tags = vec!["burst".into(), element.name().into()];
    if periodic {
        r.attack_type = AttackType::Buff;
        if element == Element::Apoptosis {
            r.tags.push("dot".into());
        }
    }
    r
}
fn clamp(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}
fn round(v: f64) -> f64 {
    let f = v.floor();
    if v - f < 0.5 {
        f
    } else {
        f + 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::{
        blocking,
        health::{Side, Unit},
        stats::{Base, UnitStats},
    };
    struct Host;
    impl super::super::health::Hooks for Host {
        fn kill(&mut self, _: Id, _: Option<Id>, _: &mut World) -> Result<(), String> {
            unreachable!()
        }
    }
    impl blocking::Hooks for Host {
        fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
        fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
    }
    impl buffs::Hooks for Host {
        fn status(&mut self, _: Id, _: &str, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn callback(
            &mut self,
            _: u64,
            _: buffs::Event,
            _: buffs::Context,
            _: &mut Runtime,
            _: &mut World,
        ) -> Result<(), String> {
            unreachable!()
        }
    }
    impl status::Hooks for Host {
        fn has_before_status(&self) -> bool {
            false
        }
        fn before_status(
            &mut self,
            _: &mut status::Context,
            _: &mut Runtime,
            _: &mut World,
        ) -> Result<(), String> {
            unreachable!()
        }
        fn has_status_applied(&self) -> bool {
            false
        }
        fn status_applied(
            &mut self,
            _: &status::Context,
            _: &mut Runtime,
            _: &mut World,
        ) -> Result<(), String> {
            unreachable!()
        }
    }
    impl Hooks for Host {
        fn battle_rect(&self) -> Rect {
            Rect::FULL
        }
        fn element_enabled(&self, p: Phase) -> bool {
            p == Phase::Burst
        }
        fn element_event(
            &mut self,
            _: Phase,
            _: &mut Request,
            _: f64,
            _: f64,
            _: &mut Runtime,
            _: &mut World,
        ) -> Result<(), String> {
            Err("burst callback failed".into())
        }
        fn element_number(&mut self, _: Id, _: f64, _: Element) -> Result<(), String> {
            Ok(())
        }
        fn burst_visual(&mut self, _: Id, _: Element, _: &World) -> Result<(), String> {
            Ok(())
        }
    }
    fn world() -> World {
        let mut w = World::new(1, 1, 1).unwrap();
        let mut u = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        w
    }
    #[test]
    fn burst_callback_failure_clears_pending_and_faults_runtime() {
        let mut w = world();
        let mut r = Runtime::new(8, 16, 8).unwrap();
        let mut h = Host;
        let error = burst(&mut w, &mut h, &mut r, None, None, 1, Element::Burn).unwrap_err();
        assert_eq!(error, "burst callback failed");
        assert!(r.faulted());
        assert_eq!(w.unit(1).unwrap().burst_pending, [false; 5]);
        assert!(w.unit(1).unwrap().stats.buffs().is_empty());
    }
    #[test]
    fn recovery_lowers_each_gauge_and_pending_burst_locks_every_element() {
        let mut w = world();
        w.unit_mut(1).unwrap().element_buildup = [150.0, 250.0, 0.0, 80.0, 10.0];
        assert_eq!(heal(&mut w, 1, 100.0, None).unwrap(), 290.0);
        assert_eq!(
            w.unit(1).unwrap().element_buildup,
            [50.0, 150.0, 0.0, 0.0, 0.0]
        );
        w.unit_mut(1).unwrap().burst_pending[2] = true;
        assert_eq!(heal(&mut w, 1, 1000.0, Some(Element::Neural)).unwrap(), 0.0);
        assert_eq!(w.unit(1).unwrap().element_buildup[1], 150.0);
    }
    #[test]
    fn visible_gauge_ties_use_official_order_and_entity_class_is_independent_of_side() {
        let mut w = world();
        let u = w.unit_mut(1).unwrap();
        u.side = Side::Ally;
        u.enemy_entity = true;
        u.element_leader = true;
        u.element_buildup = [100.0; 5];
        assert_eq!(gauge_max(&w, 1).unwrap(), 2000.0);
        let v = view(&mut w, 1, 0.0).unwrap().unwrap();
        assert_eq!(v.element, Element::Neural);
        assert_eq!(v.fill, 0.05);
        w.unit_mut(1).unwrap().element_buildup = [0.0001, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(view(&mut w, 1, 0.0).unwrap().unwrap().fill, 0.01);
    }
}
