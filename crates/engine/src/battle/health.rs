// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 attack/health.js and attack/heal.js.
// Unit removal/redeployment and content callbacks are delegated to the battle host.
use super::{
    damage::{self, DamageType, Penetration},
    stats::{Stat, UnitStats},
};
use std::collections::BTreeMap;
pub type Id = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Ally,
    Enemy,
}
pub struct Unit {
    pub(crate) operator: super::content::operators::Memory,
    pub deployment: super::deploy::State,
    pub targeting: super::targeting::State,
    pub hit_area: Option<super::body::HitArea>,
    pub static_body: bool,
    pub cadence: super::cadence::State,
    pub position: [f64; 2],
    pub ground: bool,
    pub profile_block_fly: bool,
    pub block_weight: f64,
    pub moving: bool,
    pub motion: super::motion::State,
    pub enemy: super::enemy::State,
    pub(crate) blocking: Vec<Id>,
    pub(crate) blocked_by: Option<Id>,
    pub stats: UnitStats,
    pub side: Side,
    pub owner: Option<Id>,
    pub summoner: Option<Id>,
    pub deployed: bool,
    pub removed: bool,
    pub hidden: bool,
    pub motion_fly: bool,
    pub hits_sleep: bool,
    pub elemental_resistance: f64,
    /// burn, neural, necrosis, apoptosis, erosion; shared by healing selectors
    /// and the elemental damage runtime, never inferred from HP damage.
    pub element_buildup: [f64; 5],
    pub burst_pending: [bool; 5],
    pub enemy_entity: bool,
    pub element_leader: bool,
    pub ep_resistance: f64,
    pub element_damage_dealt: f64,
    pub boss: bool,
    pub immune: std::collections::BTreeSet<String>,
    pub pool: Option<Id>,
    pub skill: bool,
    pub profile_no_heal: bool,
    pub damage_dealt: f64,
    pub damage_taken: f64,
    pub healing_done: f64,
    pub regen_acc: f64,
    pub last_hit_at: f64,
    pub(crate) polluted_at: f64,
    pub(crate) chimera_at: f64,
    pub(crate) terrain: super::terrain::State,
    pub last_attack_at: f64,
    pub attacks: u64,
    pub engaged: bool,
    pins: usize,
}
impl Unit {
    /// Operators and their tokens have kits; map devices do not.
    pub(crate) fn has_operator_kit(&self) -> bool {
        self.side == Side::Ally && self.deployment.kind != super::deploy::Kind::Device
    }
    pub fn blocking(&self) -> &[Id] {
        &self.blocking
    }
    pub fn blocked_by(&self) -> Option<Id> {
        self.blocked_by
    }
    pub fn new(stats: UnitStats, side: Side) -> Self {
        Self {
            operator: Default::default(),
            deployment: super::deploy::State::default(),
            targeting: super::targeting::State::default(),
            hit_area: None,
            static_body: false,
            cadence: super::cadence::State::default(),
            position: [0.0, 0.0],
            ground: false,
            profile_block_fly: false,
            block_weight: 1.0,
            moving: false,
            motion: Default::default(),
            enemy: Default::default(),
            blocking: vec![],
            blocked_by: None,
            stats,
            side,
            owner: None,
            summoner: None,
            deployed: false,
            removed: false,
            hidden: false,
            motion_fly: false,
            hits_sleep: false,
            elemental_resistance: 0.0,
            element_buildup: [0.0; 5],
            burst_pending: [false; 5],
            enemy_entity: side == Side::Enemy,
            element_leader: false,
            ep_resistance: 0.0,
            element_damage_dealt: 0.0,
            boss: false,
            immune: Default::default(),
            pool: None,
            skill: false,
            profile_no_heal: false,
            damage_dealt: 0.0,
            damage_taken: 0.0,
            healing_done: 0.0,
            regen_acc: 0.0,
            polluted_at: f64::NEG_INFINITY,
            chimera_at: f64::NEG_INFINITY,
            terrain: Default::default(),
            last_hit_at: f64::NEG_INFINITY,
            last_attack_at: f64::NEG_INFINITY,
            attacks: 0,
            engaged: false,
            pins: 0,
        }
    }
}
#[derive(Default)]
pub struct PlayerTotals {
    pub damage: f64,
    pub boss_damage: f64,
    /// Match-level SharedBossPool.onHit credits, accumulated in hit order.
    pub match_boss_damage: f64,
    pub healing: f64,
}
pub struct Pool {
    pub hp: f64,
    pub max_hp: f64,
}
#[derive(Clone, Copy)]
enum PoolPolicy {
    Exact,
    SharedBoss,
}
struct StoredPool {
    value: Pool,
    policy: PoolPolicy,
}
impl Pool {
    pub(crate) fn validate_shared(&self) -> Result<(), String> {
        if !self.max_hp.is_finite()
            || self.max_hp < 1.
            || !self.hp.is_finite()
            || self.hp < 0.
            || self.hp > self.max_hp
        {
            return Err("Boss pool requires finite 0 <= hp <= maxHp and maxHp >= 1".into());
        }
        Ok(())
    }
}
pub struct World {
    unit_sequence: u64,
    deployment_sequence: u64,
    attack_sequence: u64,
    units: BTreeMap<Id, Unit>,
    pools: BTreeMap<Id, StoredPool>,
    players: BTreeMap<Id, PlayerTotals>,
    capacities: [usize; 3],
    pub time: f64,
    pub boss_battle: bool,
}
impl World {
    pub fn next_unit_id(&mut self) -> Result<Id, String> {
        self.unit_sequence = self
            .unit_sequence
            .checked_add(1)
            .ok_or("Unit identity exhausted")?;
        Ok(self.unit_sequence)
    }
    pub fn next_deployment_sequence(&mut self) -> Result<u64, String> {
        self.deployment_sequence = self
            .deployment_sequence
            .checked_add(1)
            .ok_or("Deployment sequence exhausted")?;
        Ok(self.deployment_sequence)
    }
    pub fn attack_sequence(&self) -> u64 {
        self.attack_sequence
    }
    pub fn next_attack_id(&mut self) -> Result<u64, String> {
        self.attack_sequence = self
            .attack_sequence
            .checked_add(1)
            .ok_or("Attack identity exhausted")?;
        Ok(self.attack_sequence)
    }
    pub fn pin_unit(&mut self, id: Id) -> Result<(), String> {
        let u = unit_mut(self, id)?;
        u.pins = u
            .pins
            .checked_add(1)
            .ok_or("Unit retention count exhausted")?;
        Ok(())
    }
    pub fn unpin_unit(&mut self, id: Id) -> Result<(), String> {
        let u = unit_mut(self, id)?;
        u.pins = u
            .pins
            .checked_sub(1)
            .ok_or("Unit retention count underflow")?;
        Ok(())
    }
    pub fn unit_pins(&self, id: Id) -> Result<usize, String> {
        Ok(unit(self, id)?.pins)
    }
    pub fn unit_capacity(&self) -> usize {
        self.capacities[0]
    }
    pub fn new(
        unit_capacity: usize,
        pool_capacity: usize,
        player_capacity: usize,
    ) -> Result<Self, String> {
        if [unit_capacity, pool_capacity, player_capacity].contains(&0) {
            return Err("Health registry limits must be positive".into());
        }
        Ok(Self {
            unit_sequence: 0,
            deployment_sequence: 0,
            attack_sequence: 0,
            units: BTreeMap::new(),
            pools: BTreeMap::new(),
            players: BTreeMap::new(),
            capacities: [unit_capacity, pool_capacity, player_capacity],
            time: 0.0,
            boss_battle: false,
        })
    }
    pub fn insert_unit(&mut self, id: Id, value: Unit) -> Result<(), String> {
        if self.units.contains_key(&id) || self.units.len() >= self.capacities[0] {
            return Err("Duplicate unit or unit capacity exceeded".into());
        }
        self.unit_sequence = self.unit_sequence.max(id);
        self.deployment_sequence = self.deployment_sequence.max(value.deployment.deploy_seq);
        self.units.insert(id, value);
        Ok(())
    }
    pub fn insert_pool(&mut self, id: Id, value: Pool) -> Result<(), String> {
        self.insert_pool_with_policy(id, value, PoolPolicy::Exact)
    }
    pub(crate) fn insert_shared_boss_pool(&mut self, id: Id, value: Pool) -> Result<(), String> {
        value.validate_shared()?;
        self.insert_pool_with_policy(id, value, PoolPolicy::SharedBoss)
    }
    fn insert_pool_with_policy(
        &mut self,
        id: Id,
        value: Pool,
        policy: PoolPolicy,
    ) -> Result<(), String> {
        if self.pools.contains_key(&id) || self.pools.len() >= self.capacities[1] {
            return Err("Duplicate pool or pool capacity exceeded".into());
        }
        self.pools.insert(id, StoredPool { value, policy });
        Ok(())
    }
    pub fn insert_player(&mut self, id: Id, value: PlayerTotals) -> Result<(), String> {
        if self.players.contains_key(&id) || self.players.len() >= self.capacities[2] {
            return Err("Duplicate player or player capacity exceeded".into());
        }
        self.players.insert(id, value);
        Ok(())
    }
    pub fn unit_ids(&self) -> impl Iterator<Item = Id> + '_ {
        self.units.keys().copied()
    }
    pub fn unit(&self, id: Id) -> Result<&Unit, String> {
        unit(self, id)
    }
    pub fn unit_mut(&mut self, id: Id) -> Result<&mut Unit, String> {
        unit_mut(self, id)
    }
    /// End-of-tick release: knocked-out operators remain registered for redeployment.
    pub fn release_removed(&mut self, id: Id) -> Result<(), String> {
        let u = unit(self, id)?;
        if !u.removed || u.pins > 0 || u.blocked_by.is_some() || !u.blocking.is_empty() {
            return Err("Cannot release a retained or still linked unit".into());
        }
        super::enemy::clear(self, id)?;
        self.units.remove(&id);
        Ok(())
    }
    pub fn pool(&self, id: Id) -> Option<&Pool> {
        self.pools.get(&id).map(|p| &p.value)
    }
    pub fn pool_mut(&mut self, id: Id) -> Option<&mut Pool> {
        self.pools.get_mut(&id).map(|p| &mut p.value)
    }
    pub fn player(&self, id: Id) -> Option<&PlayerTotals> {
        self.players.get(&id)
    }
    pub(crate) fn player_mut(&mut self, id: Id) -> Option<&mut PlayerTotals> {
        self.players.get_mut(&id)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Damage,
    HpLoss,
    Heal,
    Regen,
    Restore,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fatal,
    HpChanged,
    ModifierApplied,
    DamageApplied,
    Damaged,
    SkillDamaged,
    OnApplied,
    Heal,
}
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub stage: Stage,
    pub target: Id,
    pub source: Option<Id>,
    pub credit: Option<Id>,
    pub kind: Kind,
    pub amount: f64,
    pub hp_removed: f64,
    pub delta: f64,
    pub prevented: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub target: Id,
    pub source: Option<Id>,
    pub credit: Option<Id>,
    pub damage_type: DamageType,
    pub no_sp: bool,
    pub silent: bool,
    pub hp_loss: bool,
    pub on_applied: bool,
}
/// All handlers run synchronously and may change world state, including the target's HP.
/// Errors are technical failures propagated to the future environment owner.
pub trait Hooks {
    fn adjust_heal(
        &mut self,
        _source: Option<Id>,
        _target: Id,
        amount: f64,
        _options: HealOptions,
        _world: &mut World,
    ) -> Result<f64, String> {
        Ok(amount)
    }

    fn emit(&mut self, _event: &mut Event, _world: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn number(&mut self, _heal: bool, _target: Id, _amount: f64) -> Result<(), String> {
        Ok(())
    }
    fn hit_cap(&mut self, _target: Id, _amount: f64) -> Result<(), String> {
        Ok(())
    }
    fn kill(&mut self, target: Id, credit: Option<Id>, world: &mut World) -> Result<(), String>;
    fn heal_through(&mut self, _source: Id, _target: Id, _world: &World) -> bool {
        false
    }
    fn overheal(
        &mut self,
        _target: Id,
        _excess: f64,
        _max_hp: f64,
        _duration: Option<f64>,
        _world: &mut World,
    ) -> Result<(), String> {
        Err("Overheal effect handler not installed".into())
    }
}
fn unit(w: &World, id: Id) -> Result<&Unit, String> {
    w.units
        .get(&id)
        .ok_or_else(|| format!("Unknown health unit {id}"))
}
fn unit_mut(w: &mut World, id: Id) -> Result<&mut Unit, String> {
    w.units
        .get_mut(&id)
        .ok_or_else(|| format!("Unknown health unit {id}"))
}
fn stats(w: &mut World, id: Id, stat: Stat) -> Result<f64, String> {
    unit_mut(w, id)?
        .stats
        .stats()
        .map(|s| s.get(stat))
        .map_err(str::to_string)
}
fn flag(w: &mut World, id: Id, key: &str) -> Result<bool, String> {
    unit_mut(w, id)?
        .stats
        .stats()
        .map(|s| s.flags.contains(key))
        .map_err(str::to_string)
}
fn event(hit: Hit, stage: Stage, kind: Kind, amount: f64, hp_removed: f64) -> Event {
    Event {
        stage,
        target: hit.target,
        source: hit.source,
        credit: hit.credit,
        kind,
        amount,
        hp_removed,
        delta: -hp_removed,
        prevented: false,
    }
}
pub fn leader_hit_cancelled<H: Hooks>(
    world: &World,
    hooks: &mut H,
    target: Id,
    amount: f64,
) -> Result<bool, String> {
    if world.boss_battle && unit(world, target)?.boss && amount.ceil() >= 300000.0 {
        hooks.hit_cap(target, amount.ceil())?;
        Ok(true)
    } else {
        Ok(false)
    }
}
pub fn lose_hp<H: Hooks>(
    world: &mut World,
    hooks: &mut H,
    mut hit: Hit,
    amount: f64,
    penetration: Penetration,
    no_limit: bool,
) -> Result<f64, String> {
    if !unit(world, hit.target)?.stats.alive || !(amount > 0.0) {
        return Ok(0.0);
    }
    let lost = if matches!(hit.damage_type, DamageType::Physical | DamageType::Arts) {
        let target = unit_mut(world, hit.target)?;
        damage::mitigate(
            amount,
            hit.damage_type,
            target.stats.stats().map_err(str::to_string)?,
            penetration,
            0.0,
        )
    } else {
        amount
    };
    if !lost.is_finite() || lost <= 0.0 {
        return Ok(0.0);
    }
    if !no_limit && leader_hit_cancelled(world, hooks, hit.target, lost)? {
        return Ok(0.0);
    }
    hit.hp_loss = true;
    apply_hp_decrease(world, hooks, hit, lost)
}
pub fn apply_hp_decrease<H: Hooks>(
    world: &mut World,
    hooks: &mut H,
    hit: Hit,
    amount: f64,
) -> Result<f64, String> {
    if !unit(world, hit.target)?.stats.alive {
        return Ok(0.0);
    }
    let kind = if hit.hp_loss {
        Kind::HpLoss
    } else {
        Kind::Damage
    };
    let mut hp_amount = amount;
    if !hit.hp_loss && amount > 0.0 {
        let all = flag(world, hit.target, "hitCount")?;
        let arts = flag(world, hit.target, "hitCountArts")?;
        if all || arts {
            hp_amount = if all || matches!(hit.damage_type, DamageType::Arts | DamageType::True) {
                1.0
            } else {
                0.0
            };
        }
    }
    let pool = unit(world, hit.target)?.pool;
    let dealt;
    if let Some(id) = pool {
        let credit_owner = hit
            .credit
            .and_then(|c| world.units.get(&c))
            .filter(|c| c.side == Side::Ally)
            .and_then(|c| c.owner);
        let stored = world.pools.get_mut(&id).ok_or("Unknown boss pool")?;
        let p = &mut stored.value;
        let before = p.hp;
        let credited = match stored.policy {
            PoolPolicy::SharedBoss if hp_amount.is_finite() && hp_amount > 0. && before > 0. => {
                let taken = if before - hp_amount < 1. {
                    before
                } else {
                    hp_amount
                };
                p.hp -= taken;
                taken
            }
            PoolPolicy::SharedBoss => 0.,
            PoolPolicy::Exact => {
                if hp_amount > 0. {
                    p.hp = (p.hp - hp_amount).max(0.);
                }
                0.
            }
        };
        let d = (before - p.hp).max(0.0);
        dealt = if d.is_finite() { d } else { 0.0 };
        let (pool_hp, max_hp) = (p.hp, p.max_hp);
        if let Some(owner) = credit_owner {
            if let Some(pp) = world.players.get_mut(&owner) {
                pp.boss_damage += dealt;
                pp.match_boss_damage += credited;
            }
        }
        let u = unit_mut(world, hit.target)?;
        if max_hp.is_finite() && max_hp > 0.0 {
            u.stats.edit_base(|b| {
                b.0.insert("maxHp".into(), max_hp);
            });
        }
        let ratio = if max_hp > 0.0 {
            if pool_hp.is_finite() {
                pool_hp.max(0.0) / max_hp
            } else {
                1.0
            }
        } else {
            0.0
        };
        u.stats.hp = u.stats.stats().map_err(str::to_string)?.get(Stat::MaxHp) * ratio.min(1.0);
    } else {
        let before = unit(world, hit.target)?.stats.hp;
        let u = unit_mut(world, hit.target)?;
        u.stats.hp -= hp_amount;
        if u.stats.hp <= 0.0 {
            u.stats.hp = 0.0;
            let mut e = event(hit, Stage::Fatal, kind, amount, 0.0);
            hooks.emit(&mut e, world)?;
            if e.prevented
                && unit(world, hit.target)?.stats.alive
                && unit(world, hit.target)?.stats.hp < 1.0
            {
                let max = stats(world, hit.target, Stat::MaxHp)?;
                unit_mut(world, hit.target)?.stats.hp = 1.0f64.min(max);
            }
        }
        dealt = (before - unit(world, hit.target)?.stats.hp.max(0.0)).max(0.0);
    }
    let side = unit(world, hit.target)?.side;
    if let Some(id) = hit.credit {
        let c = unit_mut(world, id)?;
        if c.side != side {
            c.damage_dealt += dealt;
            let owner = if c.side == Side::Ally { c.owner } else { None };
            if let Some(owner) = owner {
                if let Some(pp) = world.players.get_mut(&owner) {
                    pp.damage += dealt;
                }
            }
        }
    }
    let time = world.time;
    let u = unit_mut(world, hit.target)?;
    u.damage_taken += dealt;
    u.last_hit_at = time;
    if !hit.silent && amount >= 0.5 {
        hooks.number(false, hit.target, amount)?;
    }
    if dealt > 0.0 {
        hooks.emit(
            &mut event(hit, Stage::HpChanged, kind, amount, dealt),
            world,
        )?;
    }
    if !hit.hp_loss && amount > 0.0 {
        hooks.emit(
            &mut event(hit, Stage::ModifierApplied, kind, amount, dealt),
            world,
        )?;
        hooks.emit(
            &mut event(hit, Stage::DamageApplied, kind, amount, dealt),
            world,
        )?;
    }
    hooks.emit(&mut event(hit, Stage::Damaged, kind, amount, dealt), world)?;
    let u = unit(world, hit.target)?;
    if u.side == Side::Ally && u.skill && !hit.no_sp {
        hooks.emit(
            &mut event(hit, Stage::SkillDamaged, kind, amount, dealt),
            world,
        )?;
    }
    let u = unit(world, hit.target)?;
    let dead = if let Some(id) = u.pool {
        world.pools.get(&id).ok_or("Unknown boss pool")?.value.hp <= 0.0
    } else {
        u.stats.hp <= 0.0
    };
    if dead && u.stats.alive {
        hooks.kill(hit.target, hit.credit, world)?;
    }
    if !hit.hp_loss && amount > 0.0 && hit.on_applied {
        hooks.emit(
            &mut event(hit, Stage::OnApplied, kind, amount, dealt),
            world,
        )?;
    }
    Ok(dealt)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct HealOptions {
    pub restore: bool,
    pub regen: bool,
    pub self_heal: bool,
    pub ignore_heal_free: bool,
    pub overheal: bool,
    pub overheal_duration: Option<f64>,
    pub silent: bool,
}
pub fn heal<H: Hooks>(
    world: &mut World,
    hooks: &mut H,
    source: Option<Id>,
    target: Id,
    amount: f64,
    opts: HealOptions,
) -> Result<f64, String> {
    let u = unit(world, target)?;
    if !u.stats.alive || u.removed || !u.deployed || u.pool.is_some() {
        return Ok(0.0);
    }
    let self_ = source == Some(target) || opts.self_heal;
    let through = source
        .map(|s| hooks.heal_through(s, target, world))
        .unwrap_or(false);
    if !opts.restore
        && !self_
        && ((flag(world, target, "noHeal")? && !through) || unit(world, target)?.profile_no_heal)
    {
        return Ok(0.0);
    }
    let bypass = opts.regen || opts.restore;
    if flag(world, target, "healFree")? && !bypass && !opts.ignore_heal_free && !through {
        return Ok(0.0);
    }
    let mut amt = if bypass {
        amount
    } else {
        amount
            * if let Some(s) = source {
                stats(world, s, Stat::HealingDealtMul)?
            } else {
                1.0
            }
            * stats(world, target, Stat::HealingTakenMul)?
    };
    amt = hooks.adjust_heal(source, target, amt, opts, world)?;
    if !(amt > 0.0) || !amt.is_finite() {
        return Ok(0.0);
    }
    let kind = if opts.restore {
        Kind::Restore
    } else if opts.regen {
        Kind::Regen
    } else {
        Kind::Heal
    };
    let hit = Hit {
        target,
        source,
        credit: source,
        damage_type: DamageType::True,
        no_sp: true,
        silent: opts.silent,
        hp_loss: false,
        on_applied: false,
    };
    if !opts.restore {
        let mut e = event(hit, Stage::Heal, kind, amt, 0.0);
        hooks.emit(&mut e, world)?;
        if !opts.regen {
            amt = if e.amount.is_finite() {
                e.amount.max(0.0)
            } else {
                0.0
            };
        }
        let u = unit(world, target)?;
        if !u.stats.alive || !u.deployed {
            return Ok(0.0);
        }
    }
    let max = stats(world, target, Stat::MaxHp)?;
    let actual = amt.min(max - unit(world, target)?.stats.hp).max(0.0);
    let u = unit_mut(world, target)?;
    u.stats.hp = max.min(u.stats.hp + actual);
    if !opts.restore && opts.overheal && amt > actual {
        hooks.overheal(target, amt - actual, max, opts.overheal_duration, world)?;
    }
    if let Some(source) = source {
        if !opts.restore {
            let u = unit_mut(world, source)?;
            u.healing_done += actual;
            let owner = if u.side == Side::Ally { u.owner } else { None };
            if let Some(id) = owner {
                if let Some(pp) = world.players.get_mut(&id) {
                    pp.healing += actual;
                }
            }
        }
    }
    if actual > 0.0 {
        let mut e = event(hit, Stage::HpChanged, kind, actual, 0.0);
        e.delta = actual;
        hooks.emit(&mut e, world)?;
    }
    if !bypass && amt > 0.0 {
        let mut e = event(hit, Stage::ModifierApplied, kind, amt, 0.0);
        e.delta = actual;
        hooks.emit(&mut e, world)?;
    }
    if actual >= 0.5 && !opts.silent {
        hooks.number(true, target, actual)?;
    }
    Ok(actual)
}
