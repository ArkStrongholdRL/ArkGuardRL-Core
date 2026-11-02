// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 ai.js updateEnemy/enemyAttack/enemyHeal.
use super::{
    blocking,
    buffs::{BuffId, Runtime},
    damage::{Damage, DamageType, Penetration},
    grid::Grid,
    health::{self, Hit, Id, World},
    hit::Request,
    motion, settlement,
    targeting::{self, Profile as Selection},
};
use crate::deterministic::{math::hypot, rng::Rng};
#[derive(Clone, Debug)]
pub struct State {
    pub carrier: Option<Id>,
    pub pause_until: f64,
    pub swing: bool,
    targets: Option<Vec<Id>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            carrier: None,
            pause_until: f64::NEG_INFINITY,
            swing: false,
            targets: None,
        }
    }
}
impl State {
    pub fn targets(&self) -> Option<&[Id]> {
        self.targets.as_deref()
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    None,
    Heal,
    Damage(DamageType),
}
#[derive(Clone, Copy, Debug)]
pub struct Animation {
    pub duration: f64,
    pub hit: Option<f64>,
}
#[derive(Clone, Debug)]
pub struct Profile {
    pub mode: Mode,
    pub no_attack: bool,
    pub melee: bool,
    pub attack_moves: bool,
    pub deferred: bool,
    pub shot: String,
    pub selection: Selection,
    pub max_targets: f64,
    pub atk_scale: f64,
    pub animation: Option<Animation>,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            mode: Mode::Damage(DamageType::Physical),
            no_attack: false,
            melee: true,
            attack_moves: false,
            deferred: false,
            shot: "none".into(),
            selection: Selection::default(),
            max_targets: 1.0,
            atk_scale: 1.0,
            animation: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    AttackStart,
    PalsyTrigger,
    BeforeAttack,
    Attack,
}
#[derive(Debug)]
pub struct Context {
    pub enemy: Id,
    pub targets: Vec<Id>,
    pub deployments: Vec<(Id, u64)>,
    pub palsy: Option<BuffId>,
    pub keep: bool,
    pub attack_id: Option<u64>,
}
#[derive(Clone, Copy, Debug)]
pub struct Delivery {
    pub source: Id,
    pub target: Id,
    pub kind: DamageType,
    pub attack_id: u64,
    pub ranged: bool,
    pub projectile: bool,
}
pub trait Hooks: motion::Hooks + blocking::Hooks + settlement::Hooks {
    fn profile(&mut self, id: Id, w: &mut World) -> Result<Profile, String>;
    fn allies_in_radius(
        &mut self,
        id: Id,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn enemies_in_radius(
        &mut self,
        id: Id,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn can_target(&mut self, enemy: Id, ally: Id, w: &mut World) -> Result<bool, String>;
    fn enemy_event_enabled(&self, event: Event) -> bool;
    fn enemy_event(
        &mut self,
        event: Event,
        c: &mut Context,
        r: &mut Runtime,
        w: &mut World,
        g: &mut Grid,
        rng: &mut Rng,
    ) -> Result<(), String>;
    fn attack_visual(&mut self, source: Id, target: Id, visual: &str) -> Result<(), String>;
    fn palsy_visual(&mut self, id: Id, w: &World) -> Result<(), String>;
    /// Implementors enqueue projectiles; immediate delivery may use impact below.
    fn deliver(
        &mut self,
        d: Delivery,
        r: &mut Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String>;
}
fn flags(w: &mut World, id: Id) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(w.unit_mut(id)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .clone())
}
fn number(w: &mut World, id: Id, key: &str) -> Result<f64, String> {
    Ok(w.unit_mut(id)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .n(key))
}
pub fn windup(interval: f64, animation: Option<Animation>) -> f64 {
    let Some(a) = animation.filter(|a| a.duration > 0.0) else {
        return 0.0;
    };
    let hit = a
        .hit
        .filter(|h| h.is_finite())
        .map(|h| h.max(0.0).min(a.duration))
        .unwrap_or(a.duration / 2.0);
    let speed = if interval > 0.0 && interval < a.duration {
        a.duration / interval
    } else {
        1.0
    };
    hit / speed
}
pub fn stand(interval: f64, p: &Profile) -> (f64, f64) {
    if p.attack_moves {
        return (0.0, 0.0);
    }
    let Some(a) = p.animation.filter(|a| a.duration > 0.0) else {
        return (0.0, 0.35);
    };
    let speed = if interval > 0.0 && interval < a.duration {
        a.duration / interval
    } else {
        1.0
    };
    let hit = a
        .hit
        .filter(|h| h.is_finite())
        .map(|h| h.max(0.0).min(a.duration))
        .unwrap_or(a.duration / 2.0);
    (hit / speed, (a.duration - hit) / speed)
}
/// Release target references when a swing is cancelled, an enemy leaves the
/// battle, or the owning world is reset. Kept targets use the same unit registry.
pub fn clear(w: &mut World, id: Id) -> Result<(), String> {
    w.unit_mut(id)?.enemy.swing = false;
    let ids = w.unit_mut(id)?.enemy.targets.take();
    if let Some(ids) = ids {
        for id in ids {
            w.unpin_unit(id)?;
        }
    }
    Ok(())
}
fn capture(w: &mut World, id: Id, ids: Vec<Id>, limit: usize) -> Result<(), String> {
    if ids.len() > limit {
        return Err("Enemy swing target capacity exceeded; no candidates dropped".into());
    }
    for &target in &ids {
        w.unit(target)?;
    }
    clear(w, id)?;
    for &target in &ids {
        w.pin_unit(target)?;
    }
    let s = &mut w.unit_mut(id)?.enemy;
    s.targets = Some(ids);
    s.swing = true;
    Ok(())
}
fn bounded(ids: Vec<Id>, limit: usize) -> Result<Vec<Id>, String> {
    if ids.len() > limit {
        Err("Enemy query target capacity exceeded; no candidates dropped".into())
    } else {
        Ok(ids)
    }
}
fn acquire<H: Hooks>(
    w: &mut World,
    h: &mut H,
    id: Id,
    radius: f64,
    p: &Profile,
    limit: usize,
) -> Result<Vec<Id>, String> {
    let blocked = w.unit(id)?.blocked_by();
    let mut out = vec![];
    if let Some(blocker) = blocked.filter(|_| !(radius > 0.0 && p.selection.block_free)) {
        if radius > 0.0 {
            let candidates = bounded(h.allies_in_radius(id, radius + 0.25, w, limit)?, limit)?;
            for ally in candidates {
                if ally == blocker || targeting::can_target_ally(w, id, ally, true, p.selection)? {
                    out.push(ally);
                }
            }
            if !out.contains(&blocker) && w.unit(blocker)?.stats.alive {
                if out.len() >= limit {
                    return Err("Enemy blocker target capacity exceeded".into());
                }
                out.push(blocker);
            }
        } else if w.unit(blocker)?.stats.alive && w.unit(blocker)?.deployed {
            out.push(blocker);
        }
    } else if radius > 0.0 {
        for ally in bounded(h.allies_in_radius(id, radius + 0.25, w, limit)?, limit)? {
            if targeting::can_target_ally(w, id, ally, true, p.selection)? {
                out.push(ally);
            }
        }
    }
    let mut filtered = vec![];
    for ally in out {
        if h.can_target(id, ally, w)? {
            filtered.push(ally);
        }
    }
    Ok(filtered)
}
fn still(w: &mut World, id: Id, target: Id, radius: f64, p: &Profile) -> Result<bool, String> {
    let t = w.unit(target)?;
    if !t.stats.alive {
        return Ok(false);
    }
    if w.unit(id)?.blocked_by() == Some(target) && !(radius > 0.0 && p.selection.block_free) {
        return Ok(t.deployed);
    }
    targeting::can_target_ally(w, id, target, radius > 0.0, p.selection)
}
fn context(id: Id, targets: Vec<Id>) -> Context {
    Context {
        enemy: id,
        targets,
        deployments: vec![],
        palsy: None,
        keep: false,
        attack_id: None,
    }
}
pub fn attack<H: Hooks>(
    w: &mut World,
    g: &mut Grid,
    r: &mut Runtime,
    rng: &mut Rng,
    h: &mut H,
    id: Id,
    previous_cd: f64,
    limit: usize,
) -> Result<bool, String> {
    let p = h.profile(id, w)?;
    if p.no_attack || matches!(p.mode, Mode::None) || number(w, id, "atk")? <= 0.0 {
        w.unit_mut(id)?.enemy.swing = false;
        return Ok(false);
    }
    let f = flags(w, id)?;
    if f.contains("fear")
        || f.contains("disarm")
        || f.contains("tremble") && w.unit(id)?.blocked_by().is_some()
    {
        clear(w, id)?;
        return Ok(false);
    }
    if matches!(p.mode, Mode::Heal) {
        if w.unit(id)?.cadence.cooldown <= 0.0 {
            heal(w, r, rng, h, id, limit)?;
        }
        return Ok(false);
    }
    let radius = if p.melee {
        0.0
    } else {
        w.unit(id)?.stats.base_number("rangeRadius")
    };
    let wind = windup(number(w, id, "interval")?, p.animation);
    if w.unit(id)?.cadence.cooldown > wind + 1e-9 {
        clear(w, id)?;
        return Ok(false);
    }
    let targets = if w.unit(id)?.enemy.swing {
        w.unit(id)?.enemy.targets.clone().unwrap_or_default()
    } else {
        let targets = acquire(w, h, id, radius, &p, limit)?;
        if targets.is_empty() {
            clear(w, id)?;
            return Ok(false);
        }
        capture(w, id, targets.clone(), limit)?;
        if previous_cd < wind {
            w.unit_mut(id)?.cadence.cooldown = wind;
        }
        targets
    };
    if w.unit(id)?.cadence.cooldown > 0.0 {
        return Ok(w.unit(id)?.blocked_by().is_none() && radius > 0.0 && !p.attack_moves);
    }
    w.unit_mut(id)?.enemy.swing = false;
    let mut retained = w.unit_mut(id)?.enemy.targets.take().unwrap_or_default();
    let result = (|| {
        let locked = targets;
        let mut selected = vec![];
        let selected_profile = h.profile(id, w)?;
        for target in &locked {
            if still(w, id, *target, radius, &selected_profile)? {
                selected.push(*target);
            }
        }
        if selected.len() > 1 {
            targeting::sort_allies(w, id, &mut selected, selected_profile.selection)?;
        }
        if h.enemy_event_enabled(Event::AttackStart) {
            let mut c = context(id, selected.clone());
            h.enemy_event(Event::AttackStart, &mut c, r, w, g, rng)?;
            if !w.unit(id)?.stats.alive || flags(w, id)?.contains("stun") {
                return Ok(false);
            }
        }
        r.adopt(w, id)?;
        if let Some(palsy) = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.key.as_deref() == Some("palsy"))
            .cloned()
        {
            let mut c = context(id, vec![]);
            c.palsy = Some(palsy.runtime.id());
            if h.enemy_event_enabled(Event::PalsyTrigger) {
                h.enemy_event(Event::PalsyTrigger, &mut c, r, w, g, rng)?;
                if !w.unit(id)?.stats.alive {
                    return Ok(false);
                }
            }
            if !c.keep {
                let stacks = r.get(w, id, palsy.runtime.id())?.stacks;
                if stacks <= 1 {
                    r.remove_id(w, h, id, palsy.runtime.id())?;
                } else {
                    r.edit(w, id, palsy.runtime.id(), |b| b.stacks -= 1)?;
                }
            }
            w.unit_mut(id)?.cadence.cooldown = number(w, id, "interval")?;
            let live = h.profile(id, w)?;
            if w.unit(id)?.blocked_by().is_none() && radius > 0.0 && !live.attack_moves {
                w.unit_mut(id)?.motion.attack_stand_until = w.time + 0.35;
            }
            h.palsy_visual(id, w)?;
            return Ok(false);
        }
        let n =
            (h.profile(id, w)?.max_targets.floor() + number(w, id, "maxTargets")?.floor()).max(1.0);
        if selected.len() as f64 > n {
            selected.truncate(n as usize);
        }
        if h.enemy_event_enabled(Event::BeforeAttack) {
            let mut c = context(
                id,
                if selected.is_empty() {
                    locked.clone()
                } else {
                    selected
                },
            );
            h.enemy_event(Event::BeforeAttack, &mut c, r, w, g, rng)?;
            if c.targets.len() > limit {
                return Err("Enemy callback target capacity exceeded".into());
            }
            if !w.unit(id)?.stats.alive || c.targets.is_empty() {
                return Ok(false);
            }
            selected = vec![];
            let selected_profile = h.profile(id, w)?;
            for t in c.targets {
                if still(w, id, t, radius, &selected_profile)? {
                    selected.push(t);
                }
            }
        }
        for &target in &selected {
            if !retained.contains(&target) {
                w.pin_unit(target)?;
                retained.push(target);
            }
        }
        let count = w
            .unit(id)?
            .attacks
            .checked_add(1)
            .ok_or("Enemy attack count exhausted")?;
        let attack_id = w.next_attack_id()?;
        let time = w.time;
        let e = w.unit_mut(id)?;
        e.last_attack_at = time;
        e.attacks = count;
        let ranged = radius > 0.0
            && !(w.unit(id)?.blocked_by().is_some()
                && selected.first().copied() == w.unit(id)?.blocked_by()
                && radius < 1.0);
        let mut deployments = vec![];
        for &t in &selected {
            deployments.push((t, w.unit(t)?.deployment.deploy_seq));
        }
        let Mode::Damage(kind) = p.mode else {
            return Err("Invalid enemy attack mode".into());
        };
        let delivery_profile = h.profile(id, w)?;
        for &target in &selected {
            h.attack_visual(
                id,
                target,
                if delivery_profile.deferred {
                    &delivery_profile.shot
                } else if ranged {
                    "enemy"
                } else {
                    "none"
                },
            )?;
            if delivery_profile.deferred {
                continue;
            }
            let e = w.unit(id)?;
            let t = w.unit(target)?;
            let projectile = ranged
                && hypot(t.position[1] - e.position[1], t.position[0] - e.position[0]) > 0.75;
            h.deliver(
                Delivery {
                    source: id,
                    target,
                    kind,
                    attack_id,
                    ranged,
                    projectile,
                },
                r,
                w,
                rng,
            )?;
        }
        if h.enemy_event_enabled(Event::Attack) {
            let mut c = context(id, selected);
            c.deployments = deployments;
            c.attack_id = Some(attack_id);
            h.enemy_event(Event::Attack, &mut c, r, w, g, rng)?;
        }
        let interval = number(w, id, "interval")?;
        w.unit_mut(id)?.cadence.cooldown = interval;
        let current = h.profile(id, w)?;
        w.unit_mut(id)?.motion.attack_stand_until = w.time + stand(interval, &current).1;
        Ok(false)
    })();
    let mut error = None;
    for target in retained {
        if let Err(e) = w.unpin_unit(target) {
            error.get_or_insert(e);
        }
    }
    match result {
        Err(e) => Err(e),
        Ok(value) => error.map_or(Ok(value), Err),
    }
}
fn heal<H: Hooks>(
    w: &mut World,
    _r: &mut Runtime,
    _rng: &mut Rng,
    h: &mut H,
    id: Id,
    limit: usize,
) -> Result<(), String> {
    let radius = w.unit(id)?.stats.base_number("rangeRadius").max(1.0);
    let candidates = bounded(h.enemies_in_radius(id, radius, w, limit)?, limit)?;
    let mut best = None;
    let mut ratio = 0.0;
    for target in candidates {
        if target == id {
            continue;
        }
        let max = number(w, target, "maxHp")?;
        let u = w.unit(target)?;
        if u.stats.hp >= max || u.pool.is_some() {
            continue;
        }
        let current = if max > 0.0 { u.stats.hp / max } else { 0.0 };
        if best.is_none() || current < ratio {
            best = Some(target);
            ratio = current;
        }
    }
    let Some(target) = best else {
        return Ok(());
    };
    let count = w
        .unit(id)?
        .attacks
        .checked_add(1)
        .ok_or("Enemy attack count exhausted")?;
    let time = w.time;
    let u = w.unit_mut(id)?;
    u.last_attack_at = time;
    u.attacks = count;
    h.attack_visual(id, target, "orb")?;
    let amount = number(w, id, "atk")?;
    health::heal(w, h, Some(id), target, amount, Default::default())?;
    w.unit_mut(id)?.cadence.cooldown = number(w, id, "interval")?;
    Ok(())
}
/// Read current ATK/profile at arrival; the damage type and attack identity belong
/// to the launch. A ranged attack survives its shooter's death, a direct melee hit does not.
pub fn impact<H: Hooks>(
    w: &mut World,
    r: &mut Runtime,
    rng: &mut Rng,
    h: &mut H,
    d: Delivery,
    target: Option<Id>,
) -> Result<f64, String> {
    let Some(target) = target else {
        return Ok(0.0);
    };
    if !w.unit(target)?.stats.alive || !w.unit(d.source)?.stats.alive && !d.ranged {
        return Ok(0.0);
    }
    let scale = h.profile(d.source, w)?.atk_scale;
    let amount = number(w, d.source, "atk")? * scale;
    let mut request = Request::new(
        Hit {
            source: Some(d.source),
            credit: Some(d.source),
            target,
            damage_type: d.kind,
            no_sp: false,
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
    request.attack_id = Some(d.attack_id);
    request.is_attack = true;
    request.is_projectile = d.projectile;
    settlement::resolve(w, r, rng, h, &mut request)
}
pub fn update<H: Hooks>(
    w: &mut World,
    g: &mut Grid,
    r: &mut Runtime,
    rng: &mut Rng,
    h: &mut H,
    id: Id,
    dt: f64,
    limit: usize,
) -> Result<(), String> {
    r.call(|r| {
        if !dt.is_finite() || dt < 0.0 || limit == 0 {
            return Err("Invalid enemy update limits".into());
        }
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        w.pin_unit(id)?;
        let result = (|| {
            if w.unit(id)?.enemy.carrier.is_some() {
                w.unit_mut(id)?.moving = false;
                return Ok(());
            }
            let f = flags(w, id)?;
            let stunned = f.contains("stun");
            let hidden = w.unit(id)?.hidden;
            if stunned || hidden || f.contains("fear") {
                h.cut_attack_stand(id, w)?;
            }
            if f.contains("levitate") && w.unit(id)?.motion.unbalance_until > w.time {
                w.unit_mut(id)?.motion.unbalance_until = f64::NEG_INFINITY;
            }
            let unbalanced = w.time < w.unit(id)?.motion.unbalance_until - 1e-9;
            let previous = w.unit(id)?.cadence.cooldown;
            w.unit_mut(id)?.cadence.tick(dt, !stunned && !hidden);
            if w.unit(id)?.enemy.swing && (stunned || hidden || unbalanced) {
                clear(w, id)?;
            }
            let winding =
                !hidden && !stunned && !unbalanced && attack(w, g, r, rng, h, id, previous, limit)?;
            if !w.unit(id)?.stats.alive {
                return Ok(());
            }
            if stunned && !w.unit(id)?.hidden {
                w.unit_mut(id)?.motion.attack_stand_until = f64::NEG_INFINITY;
                if w.unit(id)?.blocked_by().is_some() && flags(w, id)?.contains("sleep") {
                    blocking::unblock(w, h, id)?;
                }
                h.check_block(id, w, g)?;
                return Ok(());
            }
            if let Some(blocker) = w.unit(id)?.blocked_by() {
                let bf = flags(w, blocker)?;
                let ef = flags(w, id)?;
                let bl = w.unit(blocker)?;
                if !bl.stats.alive
                    || !bl.deployed
                    || bl.hidden
                    || bf.contains("noBlock")
                    || bf.contains("sleep")
                    || ef.contains("unblockable")
                    || ef.contains("levitate")
                    || ef.contains("fear")
                {
                    blocking::unblock(w, h, id)?;
                } else {
                    return Ok(());
                }
            }
            if !w.unit(id)?.hidden && h.check_block(id, w, g)? {
                return Ok(());
            }
            if w.time < w.unit(id)?.enemy.pause_until {
                return Ok(());
            }
            let standing = winding
                || unbalanced
                || w.time < w.unit(id)?.motion.attack_stand_until
                    && !flags(w, id)?.contains("fear");
            motion::update(w, g, r, rng, h, id, dt, standing, unbalanced)
        })();
        let release = w.unpin_unit(id);
        match result {
            Err(e) => Err(e),
            Ok(()) => release,
        }
    })
}
