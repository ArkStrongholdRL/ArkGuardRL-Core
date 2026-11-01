// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 attack/hit.js: HP-damage route.
// Element gauges and concrete skill/transfer handlers are separate content systems.
use super::{
    damage::{self, Damage},
    health::{self, Hit, Side, World},
    injury::{self, DodgeMode, EffectId, Effects, HookResult, InjuryEvent, Notice, Observation},
    stats::Stats,
};
use crate::deterministic::rng::Rng;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calculation {
    Formula,
    Precomputed,
    Resolved,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Hit,
    Calculate,
    CalculateTarget,
    Output,
    BeforeApply,
    Applying,
    HpDamage,
}
pub struct Request {
    pub attack_id: Option<u64>,
    pub is_attack: bool,
    pub is_skill: bool,
    pub is_splash: bool,
    pub tags: Vec<String>,
    pub hit: Hit,
    pub damage: Damage,
    pub calculation: Calculation,
    pub cancelled: bool,
    pub ignore_sleep: bool,
    pub ignore_select: bool,
    pub dodge_mode: DodgeMode,
    pub ignore_block: bool,
}
impl Request {
    pub fn new(hit: Hit, damage: Damage) -> Self {
        let calculation = if damage.resolved {
            Calculation::Resolved
        } else {
            Calculation::Formula
        };
        Self {
            attack_id: None,
            is_attack: false,
            is_skill: false,
            is_splash: false,
            tags: Vec::new(),
            hit,
            damage,
            calculation,
            cancelled: false,
            ignore_sleep: false,
            ignore_select: false,
            dodge_mode: DodgeMode::Normal,
            ignore_block: false,
        }
    }
}
/// Content dispatch may synchronously change stats, effects and unit lifecycle.
/// No phase is enabled until the host installs a handler for it.
pub trait Hooks: health::Hooks + Sized {
    fn enabled(&self, _phase: Phase) -> bool {
        false
    }
    fn phase(
        &mut self,
        _phase: Phase,
        _request: &mut Request,
        _amount: &mut f64,
        _world: &mut World,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn injury(
        &mut self,
        _id: EffectId,
        _event: &mut InjuryEvent,
        _request: &mut Request,
        _world: &mut World,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn observe(
        &mut self,
        _id: EffectId,
        _event: Observation,
        _request: &mut Request,
        _world: &mut World,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn notice(
        &mut self,
        _notice: Notice,
        _expected: f64,
        _request: &mut Request,
        _world: &mut World,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
    fn cleanup(&mut self, _token: u64, _world: &mut World) -> HookResult {
        Ok(())
    }
    /// Runs after HP guards, before HP application. The concrete content host owns
    /// bounded transfer reservations and their ancestry; default means no transfers.
    fn transfers(
        &mut self,
        _request: &Request,
        _world: &mut World,
        _effects: &mut Effects,
        _rng: &mut Rng,
    ) -> HookResult {
        Ok(())
    }
}
fn snapshot(world: &mut World, id: u64) -> Result<Stats, String> {
    world
        .unit_mut(id)?
        .stats
        .stats()
        .cloned()
        .map_err(str::to_string)
}
fn stopped(world: &World, r: &Request) -> Result<bool, String> {
    let u = world.unit(r.hit.target)?;
    Ok(r.cancelled || !u.stats.alive || !u.deployed)
}
fn protected(world: &World, r: &Request, s: &Stats) -> Result<bool, String> {
    Ok(s.flags.contains("invulnerable")
        || (s.flags.contains("sleep")
            && !r.ignore_sleep
            && !r
                .hit
                .source
                .map(|id| world.unit(id).map(|u| u.hits_sleep))
                .transpose()?
                .unwrap_or(false)))
}
fn input_guard(world: &mut World, r: &Request, s: &Stats) -> Result<bool, String> {
    if protected(world, r, s)? {
        return Ok(true);
    }
    if !s.flags.contains("liftoff") || r.ignore_select {
        return Ok(false);
    }
    if let Some(id) = r.hit.source {
        let u = world.unit_mut(id)?;
        let enemy = u.side == Side::Enemy;
        let motion = u.motion_fly;
        if enemy && !u.stats.is_flying(true, motion).map_err(str::to_string)? {
            return Ok(true);
        }
    }
    Ok(false)
}
fn positive(v: f64) -> f64 {
    if v.is_finite() {
        v.max(0.0)
    } else {
        0.0
    }
}
fn phase<H: Hooks>(
    h: &mut H,
    p: Phase,
    r: &mut Request,
    amount: &mut f64,
    w: &mut World,
    e: &mut Effects,
    rng: &mut Rng,
) -> HookResult {
    h.phase(p, r, amount, w, e, rng)
}
struct Adapter<'a, H> {
    host: &'a mut H,
    world: &'a mut World,
    expected: f64,
    request: &'a mut Request,
}
impl<H: Hooks> injury::Hooks for Adapter<'_, H> {
    fn injury(
        &mut self,
        id: EffectId,
        event: &mut InjuryEvent,
        effects: &mut Effects,
        rng: &mut Rng,
    ) -> HookResult {
        self.host
            .injury(id, event, self.request, self.world, effects, rng)
    }
    fn observe(
        &mut self,
        id: EffectId,
        event: Observation,
        effects: &mut Effects,
        rng: &mut Rng,
    ) -> HookResult {
        self.host
            .observe(id, event, self.request, self.world, effects, rng)
    }
    fn notice(&mut self, n: Notice, effects: &mut Effects, rng: &mut Rng) -> HookResult {
        self.host
            .notice(n, self.expected, self.request, self.world, effects, rng)
    }
    fn cleanup(&mut self, token: u64) -> HookResult {
        self.host.cleanup(token, self.world)
    }
}
pub fn resolve<H: Hooks>(
    world: &mut World,
    effects: &mut Effects,
    rng: &mut Rng,
    host: &mut H,
    r: &mut Request,
) -> Result<f64, String> {
    let target = r.hit.target;
    let u = world.unit(target)?;
    if !u.stats.alive || u.removed || u.hidden || !u.deployed {
        return Ok(0.0);
    }
    let mut ts = snapshot(world, target)?;
    if input_guard(world, r, &ts)? {
        return Ok(0.0);
    }
    let mut unused = 0.0;
    if host.enabled(Phase::Hit) {
        phase(host, Phase::Hit, r, &mut unused, world, effects, rng)?;
        if stopped(world, r)? {
            return Ok(0.0);
        }
        ts = snapshot(world, target)?;
        if protected(world, r, &ts)? {
            return Ok(0.0);
        }
    }
    if r.calculation != Calculation::Resolved {
        if r.hit.source.is_some()
            && r.calculation != Calculation::Precomputed
            && host.enabled(Phase::Calculate)
        {
            phase(host, Phase::Calculate, r, &mut unused, world, effects, rng)?;
            if stopped(world, r)? {
                return Ok(0.0);
            }
        }
        if host.enabled(Phase::CalculateTarget) {
            phase(
                host,
                Phase::CalculateTarget,
                r,
                &mut unused,
                world,
                effects,
                rng,
            )?;
            if stopped(world, r)? {
                return Ok(0.0);
            }
        }
        ts = snapshot(world, target)?;
        r.damage.amount = damage::attack_input(r.damage.amount, r.hit.damage_type, &ts);
        if protected(world, r, &ts)? {
            return Ok(0.0);
        }
    }
    let dodge_mode = r.dodge_mode;
    let damage_type = r.hit.damage_type;
    let ss = r.hit.source.map(|id| snapshot(world, id)).transpose()?;
    let ep = world.unit(target)?.elemental_resistance;
    let mut dmg = r.damage;
    dmg.resolved = r.calculation == Calculation::Resolved;
    let mut final_ = damage::calculate(dmg, damage_type, &ts, ss.as_ref(), ep);
    if host.enabled(Phase::Output) {
        phase(host, Phase::Output, r, &mut final_, world, effects, rng)?;
        final_ = positive(final_);
        if stopped(world, r)? {
            return Ok(0.0);
        }
    }
    let expected = damage::expected(
        r.damage,
        damage_type,
        &ts,
        world.unit(target)?.elemental_resistance,
    );
    if r.hit.source.is_some() && host.enabled(Phase::BeforeApply) {
        phase(
            host,
            Phase::BeforeApply,
            r,
            &mut final_,
            world,
            effects,
            rng,
        )?;
        final_ = positive(final_);
        if stopped(world, r)? {
            return Ok(0.0);
        }
        ts = snapshot(world, target)?;
    }
    if host.enabled(Phase::Applying) {
        phase(host, Phase::Applying, r, &mut final_, world, effects, rng)?;
        final_ = positive(final_);
        if stopped(world, r)? {
            return Ok(0.0);
        }
        ts = snapshot(world, target)?;
    }
    let estimate = damage::estimate(final_, damage_type, &ts);
    if estimate > 0.0 && health::leader_hit_cancelled(world, host, target, estimate)? {
        return Ok(0.0);
    }
    let ignore_block = r.ignore_block;
    let mut adapter = Adapter {
        host,
        world,
        expected,
        request: r,
    };
    let result = injury::execute(
        effects,
        rng,
        &mut adapter,
        final_,
        damage_type,
        dodge_mode,
        ignore_block,
    )?;
    result.settle_with(
        &mut adapter,
        |a, mut amount| {
            if amount > 0.0 && a.host.enabled(Phase::HpDamage) {
                let before = amount;
                phase(
                    a.host,
                    Phase::HpDamage,
                    a.request,
                    &mut amount,
                    a.world,
                    effects,
                    rng,
                )?;
                amount = if amount.is_finite() {
                    amount.max(0.0).min(before)
                } else {
                    before
                };
                a.host.transfers(a.request, a.world, effects, rng)?;
            }
            health::apply_hp_decrease(a.world, a.host, a.request.hit, amount)
        },
        0.0,
    )
}
