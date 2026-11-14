// SPDX-License-Identifier: GPL-3.0-or-later
//! Concrete, headless solo field owner with explicitly supported original kits.
//! This is not the complete match/env API.
//! All mutation, tick phases, RNG and failure state live here, not in a test host.
use super::{
    blocking, buffs,
    clock::{self, Clock, End, EndState, Phase, DT},
    content::enemies as content,
    enemy,
    grid::{Grid, Rect, COLS},
    health::{self, Id, Unit, World},
    hit, motion, settlement,
    spatial::{self, RadiusRule},
    targeting::Queries,
    waves::Template,
};
use crate::{data::Catalog, deterministic::rng::Rng};
use serde_json::{json, Value};
use std::sync::Arc;
mod advanced;
mod boss;
mod boss_round;
pub use boss::{BossConfig, BossKind, BossState};
pub(crate) use boss_round::validate_boss_round;
pub use boss_round::{BossOvertime, BossRoundConfig, BossRoundOutcome, BossRoundState};
mod allies;
mod bands;
mod bonds;
mod growth;
pub use bonds::BattleBond;
mod choices;
mod devices;
pub use choices::BattleChoice;
mod turrets;
pub use devices::{ColdWind, MapConfig};
mod enemies;
mod equipment;
mod equipment_special;
mod event_window;
mod hammers;
pub(crate) mod kits;
mod movement_batch;
mod movement_paths;
mod settlement_hooks;
mod support;
mod tactics;
mod terrain;

#[derive(Clone, Copy)]
pub struct Limits {
    pub units: usize,
    pub reports: usize,
    pub route_legs: usize,
    pub buffs_per_unit: usize,
    pub callback_depth: usize,
    pub projectiles: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            units: 2048,
            reports: 4096,
            route_legs: 512,
            buffs_per_unit: 64,
            callback_depth: 32,
            projectiles: 4096,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Advance {
    pub ticks: usize,
    pub terminated: bool,
    pub budget_exhausted: bool,
}
/// Owned terminal accounting for match settlement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldResult {
    pub killed: u64,
    pub credited_kills: u64,
    pub counted_leaks: u64,
    pub coins: f64,
    pub damage: f64,
    pub healing: f64,
    pub match_boss_damage: f64,
    pub boss: Option<BossState>,
    pub boss_round: Option<BossRoundState>,
}
/// Internal state for battle parity/debugging, not a policy observation.
pub struct EnemyState {
    pub element: [f64; 5],
    pub id: Id,
    pub key: String,
    pub position: [f64; 2],
    pub hp: f64,
    pub max_hp: f64,
    pub atk: f64,
    pub res: f64,
    pub cooldown: f64,
    pub moving: bool,
    pub hidden: bool,
    pub route_leg: usize,
    pub spawn_sequence: u64,
    pub deployment_sequence: u64,
    pub flags: Vec<String>,
}
pub struct Field {
    clock: Clock,
    core: Core,
    faulted: bool,
}
struct Core {
    movement_batching: bool,
    event_windows: u64,
    movement_paths: movement_paths::MovementPaths,
    world: World,
    host: Host,
    commitments: super::search::Commitments,
    time_limit: f64,
    unit_limit: usize,
    boss: Option<BossKind>,
}
pub(crate) struct Host {
    boss_round: Option<boss_round::Round>,
    rect: Rect,
    data: Arc<Catalog>,
    terrain: Arc<super::terrain::Config>,
    grid: Option<Box<Grid>>,
    route_query_grid: Box<Grid>,
    tiles: Arc<[super::grid::Tile]>,
    // Callbacks temporarily take ownership; keep the embedded occupancy grid in place.
    roster: Option<Box<super::operators::Roster>>,
    index: Option<spatial::Index>,
    executor: Option<super::attacks::Executor>,
    flights: Option<super::projectiles::System<allies::Shot>>,
    spawns: Option<super::spawns::Runtime>,
    content: Option<content::Runtime>,
    buffs: Option<buffs::Runtime>,
    rng: Rng,
    zones: Vec<support::ZoneState>,
    tactical_timers: Vec<tactics::Timer>,
    tactical_limit: usize,
    tactical_slots: usize,
    pending_skill_starts: Vec<(Id, Vec<Id>)>,
    deferred_kit_events: std::collections::VecDeque<kits::dispatch::DeferredEvent>,
    kit_event_limit: usize,
    flushing_kit_events: bool,
    last_operator: Option<Id>,
    equipment: super::equipment::Runtime,
    devices: devices::Devices,
    choices: choices::Choices,
    bonds: bonds::Runtime,
    bands: bands::Bands,
    growth: growth::Growth,
    /// Reused per-tick identity list; empty between phases.
    scratch: Vec<Id>,
}
struct Spawn<'a> {
    host: &'a mut Host,
    buffs: &'a mut buffs::Runtime,
    content: &'a mut content::Runtime,
}
impl super::spawns::Hooks for Spawn<'_> {
    fn spawned(
        &mut self,
        id: Id,
        w: &mut World,
        r: &mut super::spawns::Runtime,
    ) -> Result<(), String> {
        if r.definition(id)?.key == "enemy_5601_entlec" {
            return Ok(());
        }
        self.host.attach_boss_pool(w, id)?;
        self.host.field_terrain(w, self.buffs, id, true)?;
        let data = Arc::clone(&self.host.data);
        r.install_content(&data, w, self.buffs, self.content, self.host, id)?;
        if let content::Kit::Special(config @ content::Special::Armor { .. }) = self.content.kit(id)
        {
            self.host.enemy_special_spawn(w, id, config)?;
            if let content::specials::Memory::Armor { left, combo, .. } = self.content.special(id) {
                self.content.set_special(
                    id,
                    content::specials::Memory::Armor {
                        left,
                        combo,
                        appeared: true,
                    },
                );
            }
        }

        let resist = match self.content.kit(id) {
            content::Kit::Ability(content::Ability::Nest { resist, .. }) => Some(resist),
            content::Kit::Caster(content::Caster {
                passive: content::casters::Passive::Mace { resist, .. },
                ..
            }) => Some(resist),
            _ => None,
        };
        if let Some(resist) = resist {
            self.host.apply_status(
                w,
                self.buffs,
                id,
                "resist",
                super::status::Options {
                    source: Some(id),
                    value: Some(resist),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    fn release(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn retired(&mut self, id: Id, _: &mut World) -> Result<(), String> {
        self.host
            .index
            .as_mut()
            .ok_or("Missing retirement index")?
            .retire(id);
        self.content.remove(id);
        Ok(())
    }
}
impl Field {
    /// `stage` and `template` come from external data. Unsupported devices and
    /// unsupported terrain/content fail before the clock starts. There is no
    /// 'disable content' option in this execution entry.
    pub fn new(
        data: Arc<Catalog>,
        stage: &Value,
        template: Template,
        seed: u32,
        time_limit: f64,
        limits: Limits,
    ) -> Result<Self, String> {
        Self::build(data, stage, template, seed, time_limit, limits, None)
    }
    fn build(
        data: Arc<Catalog>,
        stage: &Value,
        template: Template,
        seed: u32,
        time_limit: f64,
        limits: Limits,
        boss: Option<BossConfig>,
    ) -> Result<Self, String> {
        if let Some(config) = &boss {
            config.validate()?;
        }
        let rect = if boss.is_some() {
            Rect::BOSS
        } else {
            Rect::NORMAL
        };
        if [
            limits.units,
            limits.reports,
            limits.route_legs,
            limits.buffs_per_unit,
            limits.callback_depth,
            limits.projectiles,
        ]
        .contains(&0)
            || limits.units > 65536
            || limits.reports > 65536
            || limits.route_legs > 4096
            || limits.buffs_per_unit > 4096
            || limits.callback_depth > 128
            || limits.projectiles > 65536
            || !(time_limit > 0.)
            || time_limit.is_nan()
        {
            return Err("Invalid field limits".into());
        }
        let mut normalized = stage.clone();
        if !normalized.is_object() {
            return Err("Expected stage data object".into());
        }
        if normalized["legend"].is_null() {
            normalized["legend"] = normalized["tiles"].clone();
        }
        if normalized["rows"].is_null() {
            normalized["rows"] = normalized["grid"].clone();
        }
        let mut grid = Grid::new(&normalized, rect).map_err(str::to_string)?;
        let terrain = super::terrain::Config::compile(&normalized, &grid)?;
        let devices = devices::Devices::compile(&normalized, &mut grid)?;
        for s in
            template
                .spawns
                .iter()
                .chain(template.branches.values().flatten().flatten().filter(|s| {
                    s["action"]
                        .as_str()
                        .is_none_or(|a| a.eq_ignore_ascii_case("SPAWN"))
                        && s["key"]
                            .as_str()
                            .or_else(|| s["enemyKey"].as_str())
                            .is_some_and(|k| k.starts_with("enemy_"))
                }))
        {
            if let Some(key) = s["enemyKey"].as_str().or_else(|| s["key"].as_str()) {
                let kit = content::Kit::compile(
                    key,
                    &data.table("enemies").ok_or("Missing enemies")?[key],
                    &template.overrides[key],
                    s["tag"].as_str(),
                )?;
                kit.validate_tag(s["tag"].as_str())?;
            }
        }
        let spawns = super::spawns::Runtime::new(
            &data,
            template,
            &grid,
            json!("p"),
            limits.units,
            limits.reports,
            limits.route_legs,
        )?;
        spawns.validate_content(&data)?;
        let buffs = buffs::Runtime::new(
            limits.buffs_per_unit,
            limits
                .units
                .checked_mul(limits.buffs_per_unit)
                .ok_or("Buff retention limit overflow")?,
            limits.callback_depth,
        )?;
        let content = content::Runtime::new(limits.units)?;
        let ally_limit = limits.units;
        let roster = super::operators::Roster::new(
            &grid,
            1,
            super::operators::Limits {
                units: ally_limit,
                callback_depth: limits.callback_depth,
                deploy_order: if data
                    .table("config")
                    .is_some_and(|d| d["version"]["rules"]["deployOrder"] == "rowBottomUp")
                {
                    super::operators::DeployOrder::RowBottomUp
                } else {
                    super::operators::DeployOrder::Column
                },
                ..Default::default()
            },
        )?;
        let mut world = World::new(limits.units, 1, 1)?;
        world.insert_player(1, health::PlayerTotals::default())?;
        let boss = boss
            .map(|config| {
                world.boss_battle = true;
                world.insert_shared_boss_pool(1, config.pool)?;
                Ok::<_, String>(config.kind)
            })
            .transpose()?;
        let field = Self {
            clock: Clock::new(),
            core: Core {
                movement_batching: true,
                event_windows: 0,
                movement_paths: movement_paths::MovementPaths::default(),
                world,
                boss,
                host: Host {
                    boss_round: None,
                    rect,
                    data,
                    terrain: Arc::new(terrain),
                    devices,
                    choices: Default::default(),
                    bonds: bonds::Runtime::for_stage(&normalized),
                    bands: Default::default(),
                    growth: Default::default(),
                    scratch: Vec::new(),
                    zones: Vec::new(),
                    tactical_timers: Vec::new(),
                    tactical_limit: limits.projectiles,
                    tactical_slots: 0,
                    pending_skill_starts: Vec::new(),
                    deferred_kit_events: Default::default(),
                    kit_event_limit: limits.units * limits.callback_depth,
                    flushing_kit_events: false,
                    last_operator: None,
                    equipment: Default::default(),
                    tiles: grid.tile_layout(),
                    route_query_grid: Box::new(grid.clone()),
                    grid: Some(Box::new(grid)),
                    roster: Some(Box::new(roster)),
                    index: Some(spatial::Index::new(
                        limits.units,
                        limits.units * super::grid::N,
                    )?),
                    executor: Some(super::attacks::Executor::new(
                        limits.units,
                        super::operators::Combat::MAX_HITS,
                    )?),
                    flights: Some(super::projectiles::System::new(limits.projectiles)?),
                    spawns: Some(spawns),
                    content: Some(content),
                    buffs: Some(buffs),
                    rng: Rng::new(if seed == 0 { 1 } else { seed }),
                },
                commitments: super::search::Commitments::new(
                    ally_limit,
                    limits.units,
                    ally_limit * limits.units,
                )?,
                time_limit,
                unit_limit: limits.units,
            },
            faulted: false,
        };
        Ok(field)
    }
    fn healthy(&self) -> Result<(), String> {
        if self.faulted || self.clock.faulted() {
            Err("Field is faulted; reset requires a new field".into())
        } else {
            Ok(())
        }
    }
    pub fn faulted(&self) -> bool {
        self.faulted || self.clock.faulted()
    }
    pub fn time(&self) -> f64 {
        self.clock.time()
    }
    pub fn ticks(&self) -> u64 {
        self.clock.ticks()
    }
    pub fn finished(&self) -> bool {
        self.clock.finished()
    }
    pub fn reason(&self) -> Option<&End> {
        self.clock.reason()
    }
    pub fn enemies(&self) -> &[Id] {
        self.core.host.spawns.as_ref().unwrap().live()
    }
    pub fn allies(&self) -> &[Id] {
        self.core.host.roster.as_ref().unwrap().ids()
    }
    /// Install an explicitly supported default operator before the first tick.
    pub fn add_operator(
        &mut self,
        key: &str,
        placement: super::operators::Placement,
    ) -> Result<Id, String> {
        self.add_operator_with_skill(key, placement, None)
    }
    /// Install a supported skill index with the default module and potential.
    pub fn add_operator_with_skill(
        &mut self,
        key: &str,
        placement: super::operators::Placement,
        skill_index: Option<u32>,
    ) -> Result<Id, String> {
        self.add_operator_with_loadout(key, placement, skill_index, &[])
    }
    pub fn configure_round_gained_chess(&mut self, count: u64) -> Result<(), String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 {
            return Err("Round context must be installed before field start".into());
        }
        self.core.host.equipment.gained_chess = count;
        Ok(())
    }
    pub fn add_operator_with_loadout(
        &mut self,
        key: &str,
        placement: super::operators::Placement,
        skill_index: Option<u32>,
        items: &[String],
    ) -> Result<Id, String> {
        self.healthy()?;
        if self.ticks() != 0 || self.clock.started() || self.finished() {
            return Err("Operators must be installed before field start".into());
        }
        let def = super::operators::Definition::load_with_loadout(
            &self.core.host.data,
            key,
            skill_index,
            items,
        )?;
        let equipment = def.equipment.clone();
        let grid = self.core.host.take_grid();
        if grid.obstacle(placement.home) & super::grid::CRATE != 0 {
            self.core.host.grid = Some(grid);
            return Err("Operator deployment tile is occupied by a crate".into());
        }
        let mut roster = self.core.host.roster.take().unwrap();
        let mut r = self.core.host.buffs.take().unwrap();
        let result = roster.add(
            &mut self.core.world,
            &mut r,
            &mut self.core.host,
            &grid,
            def,
            placement,
        );
        if let Ok(id) = &result {
            let bonds = super::equipment::bonds(&self.core.host.data, key, items);
            self.core.host.equipment.register(*id, &equipment, bonds);
        }
        self.faulted |= roster.faulted();
        self.core.host.roster = Some(roster);
        self.core.host.buffs = Some(r);
        self.core.host.grid = Some(grid);
        result
    }
    pub(crate) fn add_prepared_token(
        &mut self,
        key: &str,
        placement: super::operators::Placement,
        owner_key: &str,
    ) -> Result<Id, String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 {
            return Err("Summons must be installed before field start".into());
        }
        let definition = super::operators::Definition::token(&self.core.host.data, key, owner_key)?;
        let grid = self.core.host.take_grid();
        let mut roster = self.core.host.roster.take().unwrap();
        let mut buffs = self.core.host.buffs.take().unwrap();
        let result = roster.add(
            &mut self.core.world,
            &mut buffs,
            &mut self.core.host,
            &grid,
            definition,
            placement,
        );
        self.faulted |= roster.faulted();
        self.core.host.roster = Some(roster);
        self.core.host.buffs = Some(buffs);
        self.core.host.grid = Some(grid);
        let id = result?;
        let unit = self.core.world.unit_mut(id)?;
        unit.deployment.kind = super::deploy::Kind::Token;
        Ok(id)
    }
    pub(crate) fn bind_prepared_token(&mut self, token: Id, owner_uid: u64) -> Result<(), String> {
        let owner = self
            .allies()
            .iter()
            .copied()
            .find(|&id| {
                self.unit(id).is_ok_and(|u| {
                    u.deployment.kind == super::deploy::Kind::Operator
                        && u.deployment.uid == Some(owner_uid)
                })
            })
            .ok_or("Prepared summon owner is missing")?;
        self.core.world.unit_mut(token)?.summoner = Some(owner);
        Ok(())
    }
    /// The owner's cultivation is an independent multiplier, not an additive
    /// bond/equipment percentage. Raw Field callers opt in; owned solo defaults
    /// to the original player's E2 level-60 tier.
    pub fn configure_operator_cultivation(&mut self, id: Id, tier: u8) -> Result<(), String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 || tier > 3 {
            return Err("Cultivation requires an unstarted field and tier 0..3".into());
        }
        if self.core.world.unit(id)?.deployment.kind != super::deploy::Kind::Operator {
            return Err("Cultivation requires an operator".into());
        }
        let effect = format!("aceffect_char_{}", tier + 1);
        let buff = self
            .core
            .host
            .data
            .table("effects")
            .and_then(|v| v.get(&effect))
            .and_then(|v| v["buffs"].as_array())
            .and_then(|a| a.iter().find(|b| b["key"] == "char_attribute_mul"));
        let Some(buff) = buff else {
            return Ok(());
        };
        let mul = |key: &str| {
            buff["bb"][key]
                .as_f64()
                .filter(|v| v.is_finite() && *v > 0.)
                .unwrap_or(1.)
        };
        let cultivation = super::stats::Cultivation {
            hp: mul("max_hp"),
            atk: mul("atk"),
            def: mul("def"),
        };
        self.core
            .world
            .unit_mut(id)?
            .stats
            .set_cultivation(cultivation);
        Ok(())
    }
    pub fn unit(&self, id: Id) -> Result<&Unit, String> {
        self.core.world.unit(id)
    }
    /// On-demand rendering data. Reading it never materializes lazy combat stats.
    pub fn replay_frame(&self) -> Result<Value, String> {
        self.healthy()?;
        let mut units = Vec::new();
        for &id in self.allies().iter().chain(self.enemies()) {
            let u = self.unit(id)?;
            if !u.stats.alive || !u.deployed || u.hidden {
                continue;
            }
            let key = match u.side {
                health::Side::Ally => {
                    &self
                        .core
                        .host
                        .roster
                        .as_ref()
                        .unwrap()
                        .entry(id)?
                        .definition
                        .key
                }
                health::Side::Enemy => &self.core.host.spawns.as_ref().unwrap().definition(id)?.key,
            };
            units.push(json!({"id":id,"key":key,
                "side":if u.side == health::Side::Ally {"ally"} else {"enemy"},
                "position":u.position,"hp":u.stats.hp,"maxHp":u.stats.observed_max_hp(),
                "moving":u.moving,"skillActive":u.operator.active_skill,
                "attacks":u.attacks,"lastAttackAt":u.last_attack_at}));
        }
        let projectiles: Vec<_> = self
            .core
            .host
            .flights
            .as_ref()
            .unwrap()
            .list()
            .iter()
            .map(|p| {
                json!({"id":p.id,"position":p.position,"to":p.destination,
                "source":p.source,"target":p.target,"visual":p.visual})
            })
            .collect();
        let rect = self.core.host.grid.as_ref().unwrap().rect();
        let tiles: Vec<_> = self
            .core
            .host
            .grid
            .as_ref()
            .unwrap()
            .tiles()
            .iter()
            .map(|tile| &tile.glyph)
            .collect();
        Ok(json!({"tick":self.ticks(),"time":self.time(),
            "bounds":[rect.r0,rect.c0,rect.r1,rect.c1],"tiles":tiles,
            "units":units,"projectiles":projectiles}))
    }
    pub fn rng_state(&self) -> u32 {
        self.core.host.rng.state()
    }
    pub fn pending_spawns(&self) -> usize {
        self.core
            .host
            .spawns
            .as_ref()
            .unwrap()
            .queue
            .pending()
            .len()
    }
    pub fn spawn_sequence(&self) -> u64 {
        self.core.host.spawns.as_ref().unwrap().queue.sequence()
    }
    pub fn retained_units(&self) -> usize {
        self.core.host.spawns.as_ref().unwrap().retained()
    }
    /// Owned values remain valid across subsequent ticks and field destruction.
    pub fn enemy_states(&mut self) -> Result<Vec<EnemyState>, String> {
        self.healthy()?;
        let ids = self.enemies().to_vec();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let u = self.core.world.unit_mut(id)?;
            let s = u.stats.stats().map_err(str::to_string)?.clone();
            out.push(EnemyState {
                element: u.element_buildup,
                id,
                key: self
                    .core
                    .host
                    .spawns
                    .as_ref()
                    .unwrap()
                    .definition(id)?
                    .key
                    .clone(),
                position: u.position,
                hp: u.stats.hp,
                max_hp: s.n("maxHp"),
                atk: s.n("atk"),
                res: s.n("res"),
                cooldown: u.cadence.cooldown,
                moving: u.moving,
                hidden: u.hidden,
                route_leg: u.motion.route.as_ref().map(|r| r.leg).unwrap_or(0),
                spawn_sequence: u.targeting.spawn_seq,
                deployment_sequence: u.deployment.deploy_seq,
                flags: s.flags.into_iter().collect(),
            });
        }
        Ok(out)
    }
    pub fn run(&mut self, budget: usize) -> Result<Advance, String> {
        self.healthy()?;
        let result = if self.core.host.boss_round.is_some() {
            self.run_boss_round(budget)
        } else {
            self.clock.run_steps(&mut self.core, budget)
        };
        match result {
            Ok(ticks) => Ok(Advance {
                ticks,
                terminated: self.finished(),
                budget_exhausted: !self.finished() && ticks == budget,
            }),
            Err(e) => {
                self.faulted = true;
                Err(e)
            }
        }
    }
    pub fn force_end(&mut self, timeout: bool) -> Result<(), String> {
        self.healthy()?;
        let result = self.clock.force_end(
            &mut self.core,
            if timeout { "timeout" } else { "forced" }.into(),
        );
        if result.is_err() {
            self.faulted = true;
        } else {
            self.finish_boss_round();
        }
        result
    }
    /// Internal combat API for the future match/battle owner, not an RL action.
    pub fn damage(&mut self, request: &mut hit::Request) -> Result<f64, String> {
        self.healthy()?;
        if self.finished() {
            return Err("Cannot damage a finished field".into());
        }
        // Invalid actor/target identities fail before any mutation.
        self.core.world.unit(request.hit.target)?;
        for id in [request.hit.source, request.hit.credit]
            .into_iter()
            .flatten()
        {
            self.core.world.unit(id)?;
        }
        self.core.world.time = self.clock.time();
        let mut buffs = self.core.host.buffs.take().unwrap();
        let result = self
            .core
            .host
            .resolve_hit(&mut self.core.world, &mut buffs, request);
        self.core.host.buffs = Some(buffs);
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    pub fn spawn(&mut self, key: &str, options: &Value) -> Result<Id, String> {
        self.healthy()?;
        if self.finished() {
            return Err("Cannot spawn into a finished field".into());
        }
        let raw = &self
            .core
            .host
            .data
            .table("enemies")
            .ok_or("Missing enemies")?[key];
        content::Kit::compile(
            key,
            raw,
            self.core
                .host
                .spawns
                .as_ref()
                .unwrap()
                .content_overrides(key),
            options["tag"].as_str(),
        )?
        .validate_tag(options["tag"].as_str())?;
        self.core.world.time = self.clock.time();
        let mut buffs = self.core.host.buffs.take().unwrap();
        let grid = self.core.host.take_grid();
        let result = self.core.host.with_spawns(
            &mut self.core.world,
            &mut buffs,
            |spawns, context, world| spawns.spawn(world, &grid, context, key, options),
        );
        self.core.host.buffs = Some(buffs);
        self.core.host.grid = Some(grid);
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    /// Owned accounting snapshot. This internal field result is not a policy
    /// observation and exposes no provisional success for a faulted execution.
    /// Settled public event counters only, separate from policy observations.
    pub fn contribution_feedback(&self) -> Result<Value, String> {
        self.healthy()?;
        if !self.finished() {
            return Err("Contribution feedback requires completed combat".into());
        }
        self.core.world.contribution_feedback()
    }
    pub fn result(&self) -> Result<FieldResult, String> {
        self.healthy()?;
        if !self.finished() {
            return Err("Field result requires a finished battle".into());
        }
        let ledger = &self.core.host.spawns.as_ref().unwrap().ledger;
        let player = self.core.world.player(1).unwrap();
        Ok(FieldResult {
            killed: ledger.killed,
            credited_kills: ledger.credited_kills,
            counted_leaks: ledger.counted_leaks(),
            coins: ledger.coins,
            damage: player.damage,
            healing: player.healing,
            match_boss_damage: player.match_boss_damage,
            boss: self.boss_state()?,
            boss_round: self.boss_round_state()?,
        })
    }
    pub fn report(&self) -> Result<Value, String> {
        self.healthy()?;
        let reason = self.reason().map(|e| match e {
            End::Cleared => "cleared",
            End::Timeout => "timeout",
            End::Forced => "forced",
        });
        let mut report = self
            .core
            .host
            .spawns
            .as_ref()
            .unwrap()
            .ledger
            .report(self.time(), reason);
        let loss = self.core.host.content.as_ref().unwrap().lp_loss();
        if self.finished() && loss > 0. {
            report["lpLoss"] = Value::from(loss);
        }
        if let Some(state) = self.boss_state()? {
            report["bossHpLeft"] = Value::from(state.hp);
            report["perPlayer"]["p"]["bossDamage"] = Value::from(state.damage);
        }
        if let Some(state) = self.boss_round_state()? {
            let (outcome, reason) = match state.outcome {
                None => (None, None),
                Some(BossRoundOutcome::Victory) => (Some("victory"), Some("cleared")),
                Some(BossRoundOutcome::LifeDepleted) => (Some("defeat"), Some("lp_depleted")),
                Some(BossRoundOutcome::Timeout) => (Some("defeat"), Some("timeout")),
                Some(BossRoundOutcome::Forced) => (Some("defeat"), Some("forced")),
            };
            report["bossRound"] = json!({"initialLp":state.initial_lp,"lp":state.lp,"leakLoss":state.leak_loss,"skillLoss":state.skill_loss,"overtimeLoss":state.overtime_loss,"outcome":outcome,"reason":reason});
        }
        Ok(report)
    }
}
fn inside(r: Rect, p: [i32; 2]) -> bool {
    p[0] >= r.r0 && p[0] <= r.r1 && p[1] >= r.c0 && p[1] <= r.c1
}
impl clock::Host for Core {
    fn movement_batch(&mut self, clock: &mut Clock, budget: usize) -> Result<usize, String> {
        self.run_movement_batch(clock, budget)
    }
    fn phase(&mut self, phase: Phase, c: &mut Clock) -> Result<(), String> {
        self.world.time = c.time();
        match phase {
            Phase::Start => {
                self.install_devices()?;
                self.host.growth_install(&self.world)?;
                let mut buffs = self.host.buffs.take().unwrap();
                let result = self
                    .host
                    .bonds_install(&mut self.world, &mut buffs)
                    .and_then(|_| self.host.growth_begin_readers(&mut self.world, &mut buffs))
                    .and_then(|_| self.host.equipment_install(&mut self.world, &mut buffs))
                    .and_then(|_| self.host.bands_install(&mut self.world, &mut buffs, false));
                self.host.buffs = Some(buffs);
                result?;
                self.host.rebuild_index(&self.world)?;
                let order = self
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .initial_order(&self.world)?;
                for id in order {
                    let grid = self.host.take_grid();
                    let mut roster = self.host.roster.take().unwrap();
                    let mut buffs = self.host.buffs.take().unwrap();
                    let result = roster.deploy_initial_unit(
                        &mut self.world,
                        &mut buffs,
                        &mut self.host,
                        &grid,
                        id,
                    );
                    self.host.roster = Some(roster);
                    self.host.grid = Some(grid);
                    let result = result
                        .and_then(|_| self.host.flush_kit_events(&mut self.world, &mut buffs));
                    self.host.buffs = Some(buffs);
                    result?;
                }
                let mut summons = self
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .ids()
                    .iter()
                    .copied()
                    .filter(|&id| {
                        self.world.unit(id).is_ok_and(|u| {
                            u.deployment.kind == super::deploy::Kind::Token
                                && u.stats.alive
                                && u.deployed
                        })
                    })
                    .collect::<Vec<_>>();
                summons.sort_by_key(|&id| self.world.unit(id).unwrap().deployment.deploy_seq);
                for id in summons {
                    let sequence = self.world.next_deployment_sequence()?;
                    self.world.unit_mut(id)?.deployment.aggro_seq = sequence;
                }
                self.start_map_devices()?;
                let mut buffs = self.host.buffs.take().unwrap();
                let result = self
                    .host
                    .choices_start(&mut self.world, &mut buffs)
                    .and_then(|_| {
                        self.host.kit_event(
                            &mut self.world,
                            &mut buffs,
                            &mut kits::dispatch::Event::BattleStart,
                        )
                    })
                    .and_then(|_| self.host.yan_start(&mut self.world, &mut buffs))
                    .and_then(|_| self.host.egir_start(&mut self.world, &mut buffs))
                    .and_then(|_| self.host.bands_install(&mut self.world, &mut buffs, true));
                self.host.buffs = Some(buffs);
                result?;
                let ids = self.host.roster.as_ref().unwrap().ids().to_vec();
                for id in ids {
                    let roster = self.host.roster.as_mut().unwrap();
                    let dp = roster.entry(id)?.definition.initial_dp;
                    if dp != 0. {
                        roster.add_dp(dp)?;
                    }
                }
                let mut buffs = self.host.buffs.take().unwrap();
                let result = self.host.advanced_flush_starts(&mut self.world, &mut buffs);
                self.host.buffs = Some(buffs);
                result?;
                self.host.refresh_kit_effects(&mut self.world, 0.)?;
                c.mark_started();
            }
            Phase::Scheduled => {
                let mut r = self.host.buffs.take().unwrap();
                let result = self.host.growth_refresh(&mut self.world, &mut r);
                self.host.buffs = Some(r);
                result?;
                self.device_scheduled()?;
                self.cold_wind_scheduled()?;
                let mut roster = self.host.roster.take().unwrap();
                let mut r = self.host.buffs.take().unwrap();
                let result = roster.scheduled(&mut self.world, &mut r, &mut self.host);
                self.host.roster = Some(roster);
                self.host.buffs = Some(r);
                result?;
                self.host.component_scheduled(&mut self.world)?;
                self.host.support_scheduled(&mut self.world)?;
                self.host.advanced_scheduled(&mut self.world)?;
                self.host.tactical_scheduled(&mut self.world)?;
            }
            Phase::Spawns => {
                let mut buffs = self.host.buffs.take().unwrap();
                let grid = self.host.take_grid();
                let result =
                    self.host
                        .with_spawns(&mut self.world, &mut buffs, |spawns, context, world| {
                            spawns.process(world, &grid, context)
                        });
                self.host.buffs = Some(buffs);
                self.host.grid = Some(grid);
                result?;
            }
            Phase::Dp => self.host.roster.as_mut().unwrap().recover_dp(DT)?,
            Phase::Buffs => {
                // Reusable tick list: this phase never re-enters another phase.
                let mut ids = std::mem::take(&mut self.host.scratch);
                ids.clear();
                ids.extend_from_slice(self.host.roster.as_ref().unwrap().ids());
                ids.extend_from_slice(self.host.spawns.as_ref().unwrap().live());
                let mut r = self.host.buffs.take().unwrap();
                let result = r.tick(&mut self.world, &mut self.host, &ids, DT);
                self.host.buffs = Some(r);
                self.host.scratch = ids;
                result?;
            }
            Phase::Enemies => {
                let mut ids = std::mem::take(&mut self.host.scratch);
                ids.clear();
                ids.extend_from_slice(self.host.spawns.as_ref().unwrap().live());
                let mut r = self.host.buffs.take().unwrap();
                let mut rng = self.host.rng.clone();
                let mut grid = self.host.take_grid();
                let result = (|| {
                    let mut movement_map_dirty = true;
                    for &id in &ids {
                        let moved = self.movement_batching
                            && self.movement_paths.step(
                                &mut self.world,
                                &mut self.host,
                                &grid,
                                id,
                                &mut movement_map_dirty,
                            )?;
                        if !moved {
                            enemy::update(
                                &mut self.world,
                                &mut grid,
                                &mut r,
                                &mut rng,
                                &mut self.host,
                                id,
                                DT,
                                self.unit_limit,
                            )?;
                            movement_map_dirty = true;
                        }
                        self.host.devices.flush(&mut grid)?;
                        if self.host.route_query_grid.version() != grid.version() {
                            self.host.route_query_grid.clone_from(&grid);
                        }
                        self.host.enemy_special_flush(&mut self.world, &mut r, id)?;
                        if self.world.unit(id)?.deployment.remove_reason.as_deref() == Some("leak")
                        {
                            // Leaving the field releases Ines' stolen attack too.
                            let operators = self.host.roster.as_ref().unwrap().ids().to_vec();
                            for source in operators {
                                if matches!(
                                    self.host.tac_def(source)?.extra.tactics,
                                    super::content::operators::Tactics::Ines(_)
                                ) {
                                    self.host.kit_event_for(
                                        &mut self.world,
                                        &mut r,
                                        source,
                                        &mut kits::dispatch::Event::Death {
                                            unit: id,
                                            credit: None,
                                        },
                                    )?;
                                }
                            }
                        }
                        if !matches!(
                            self.host.content.as_ref().unwrap().kit_ref(id),
                            content::Kit::Plain
                        ) {
                            movement_map_dirty = true;
                        }
                        let u = self.world.unit_mut(id)?;
                        let rect = grid.rect();
                        let m = 0.5 - 1. / 1048576.;
                        u.position[0] = u.position[0]
                            .max(rect.r0 as f64 - m)
                            .min(rect.r1 as f64 + m);
                        u.position[1] = u.position[1]
                            .max(rect.c0 as f64 - m)
                            .min(rect.c1 as f64 + m);
                    }
                    self.movement_paths.retain_live(&self.world);
                    Ok::<_, String>(())
                })();
                self.host.buffs = Some(r);
                self.host.rng = rng;
                self.host.grid = Some(grid);
                self.host.scratch = ids;
                result?;
            }
            Phase::AttackCommit => {
                self.host.rebuild_index(&self.world)?;
                let mut ids = std::mem::take(&mut self.host.scratch);
                ids.clear();
                ids.extend_from_slice(self.host.roster.as_ref().unwrap().ids());
                let result = self
                    .commitments
                    .capture(&mut self.world, &mut self.host, &ids, DT);
                self.host.scratch = ids;
                result?;
            }
            Phase::EnemyIndex => self.host.rebuild_index(&self.world)?,
            Phase::Allies => {
                let mut cursor = 0;
                while cursor < self.host.roster.as_ref().unwrap().ids().len() {
                    let id = self.host.roster.as_ref().unwrap().ids()[cursor];
                    cursor += 1;
                    if !self.world.unit(id)?.stats.alive || !self.world.unit(id)?.deployed {
                        continue;
                    }
                    let mut skill = self.host.roster.as_mut().unwrap().take_skill(id)?;
                    let result = (|| {
                        skill.tick(&mut self.world, &mut self.host, DT)?;
                        self.host.resume_kit_skill(&mut self.world, &mut skill)?;
                        super::ally::update(
                            &mut self.world,
                            &mut self.host,
                            &self.commitments,
                            id,
                            Some(&mut skill),
                            DT,
                            self.unit_limit,
                        )?;
                        self.host
                            .advanced_finish_skill(&mut self.world, &mut skill)?;
                        self.host.support_flush_sp(&mut self.world, &mut skill)
                    })();
                    let restore = self.host.roster.as_mut().unwrap().restore_skill(id, skill);
                    result?;
                    restore?;
                }
            }
            Phase::Projectiles => {
                let mut flights = self.host.flights.take().unwrap();
                let result = flights.update(&mut self.world, &mut self.host, DT);
                self.host.flights = Some(flights);
                result?;
            }
            Phase::Redeploy => {
                let grid = self.host.take_grid();
                let mut roster = self.host.roster.take().unwrap();
                let mut r = self.host.buffs.take().unwrap();
                let result = roster.check_redeploys(&mut self.world, &mut r, &mut self.host, &grid);
                self.host.roster = Some(roster);
                self.host.grid = Some(grid);
                let result =
                    result.and_then(|_| self.host.advanced_flush_starts(&mut self.world, &mut r));
                self.host.buffs = Some(r);
                result?;
            }
            Phase::TickHook => {
                self.host.growth_dolls(&self.world)?;
                let mut ids = std::mem::take(&mut self.host.scratch);
                ids.clear();
                ids.extend_from_slice(self.host.spawns.as_ref().unwrap().live());
                let mut r = self.host.buffs.take().unwrap();
                let result = self
                    .host
                    .enemy_content_tick(&mut self.world, &mut r, &ids, DT);
                self.host.buffs = Some(r);
                self.host.scratch = ids;
                result?;
                let mut r = self.host.buffs.take().unwrap();
                let result = self.host.refresh_terrain(&mut self.world, &mut r);
                self.host.buffs = Some(r);
                result?;
                self.tick_turrets(DT)?;
                let mut roster = self.host.roster.take().unwrap();
                let mut r = self.host.buffs.take().unwrap();
                let result = roster.tick_effects(&mut self.world, &mut r, &mut self.host);
                self.host.roster = Some(roster);
                self.host.buffs = Some(r);
                result?;
                self.host.refresh_kit_effects(&mut self.world, DT)?;
                self.host
                    .kit_event_with_buffs(&mut self.world, &mut kits::dispatch::Event::Tick)?;
                let mut r = self.host.buffs.take().unwrap();
                let result = self.host.choices_tick(&mut self.world, &mut r);
                self.host.buffs = Some(r);
                result?;
            }
            Phase::Boss if self.boss.is_some() => self.host.sync_bosses(&mut self.world)?,
            // Remaining phases require no separate work here.
            _ => {}
        }
        Ok(())
    }
    fn compact_enemies(&mut self, _: &mut Clock) -> Result<(), String> {
        self.host.spawns.as_mut().unwrap().compact(&self.world)
    }
    fn has_removed(&self) -> bool {
        self.host.spawns.as_ref().unwrap().retained()
            > self.host.spawns.as_ref().unwrap().live().len()
    }
    fn release_removed(&mut self, _: &mut Clock) -> Result<(), String> {
        let mut buffs = self.host.buffs.take().unwrap();
        let result =
            self.host
                .with_spawns(&mut self.world, &mut buffs, |spawns, context, world| {
                    spawns.release_removed(world, context)
                });
        self.host.buffs = Some(buffs);
        result
    }

    fn has_tick_hook(&self) -> bool {
        true
    }
    fn end_state(&self) -> Option<EndState> {
        let r = self.host.spawns.as_ref().unwrap();
        Some(EndState {
            shared_hp: self.boss.and_then(|_| self.world.pool(1).map(|p| p.hp)),
            boss_like: self.boss.is_some(),
            auto_finish: true,
            pending_spawns: !r.queue.pending().is_empty(),
            enemy_alive: !r.live().is_empty(),
            pending_holds: false,
            time_limit: self.time_limit,
            max_battle_time: 3600.,
        })
    }
    fn finish(&mut self, reason: &End, _: &Clock) -> Result<(), String> {
        if *reason == End::Timeout {
            let r = self.host.spawns.as_mut().unwrap();
            r.ledger.timeout(&mut r.queue)?;
        }
        self.host.flights.as_mut().unwrap().clear(&mut self.world)?;
        Ok(())
    }
}
impl health::Hooks for Host {
    fn adjust_heal(
        &mut self,
        source: Option<Id>,
        target: Id,
        amount: f64,
        options: health::HealOptions,
        w: &mut World,
    ) -> Result<f64, String> {
        self.support_heal_amount(w, source, target, amount, options)
    }

    fn kill(&mut self, id: Id, credit: Option<Id>, w: &mut World) -> Result<(), String> {
        let mut r = self
            .buffs
            .take()
            .ok_or("Death requires the active buff context")?;
        let result = settlement::Hooks::health_kill(self, id, credit, &mut r, w);
        self.buffs = Some(r);
        result
    }
}
impl Host {
    fn take_grid(&mut self) -> Box<Grid> {
        // Reentrant targeting queries share the same geometry while movement or
        // deployment temporarily owns the authoritative grid. Flow caches are local.
        let grid = self.grid.take().unwrap();
        if self.route_query_grid.version() != grid.version() {
            self.route_query_grid.clone_from(&grid);
        }
        grid
    }
    fn apply_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
        options: super::status::Options,
    ) -> Result<bool, String> {
        let rect = self.rect;
        super::status::apply(w, self, r, rect, id, key, options)
    }

    fn with_spawns<T>(
        &mut self,
        w: &mut World,
        buffs: &mut buffs::Runtime,
        operation: impl FnOnce(
            &mut super::spawns::Runtime,
            &mut Spawn<'_>,
            &mut World,
        ) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.spawns.is_none() || self.content.is_none() {
            return Err("Spawn context is already borrowed".into());
        }
        let mut spawns = self.spawns.take().unwrap();
        let mut content = self.content.take().unwrap();
        let result = operation(
            &mut spawns,
            &mut Spawn {
                host: self,
                buffs,
                content: &mut content,
            },
            w,
        );
        self.spawns = Some(spawns);
        self.content = Some(content);
        result
    }
    fn kill_enemy(
        &mut self,
        id: Id,
        credit: Option<Id>,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        if self.enemy_knockout(w, r, id)? {
            return Ok(());
        }
        self.kit_kill(w, r, id, credit)?;
        // A synchronous kill reaction can finish removing the same victim.
        if !w.unit(id)?.stats.alive {
            return Ok(());
        }
        self.enemy_cancel_form_timers(w, id)?;
        if let Some(source) = credit {
            if w.unit(source)?.has_operator_kit() {
                let roster = self.roster.as_mut().ok_or("Missing kill content context")?;
                let dp = roster.entry(source)?.definition.dp_on_kill;
                if dp != 0. {
                    roster.add_dp(dp)?;
                }
            }
        }
        let children = self
            .enemy_torturer_children(w, id)?
            .or(self.enemy_children(w, id)?);
        self.spawns.as_mut().unwrap().ledger.kill(
            id,
            credit
                .map(|i| w.unit(i).map(|u| u.side == health::Side::Ally))
                .transpose()?
                .unwrap_or(false),
        )?;
        enemy::clear(w, id)?;
        blocking::unblock(
            w,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            id,
        )?;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.;
        u.stats.alive = false;
        u.deployed = false;
        u.removed = true;
        u.deployment.death_at = time;
        u.deployment.remove_reason = Some("killed".into());
        if let Some(credit) = credit {
            w.unit_mut(credit)?.deployment.kills += 1;
        }
        if let Some(children) = children {
            self.enemy_spawn_children(w, r, children)?;
        }
        let mut rng = self.rng.clone();
        self.enemy_unload(w, id, &mut rng)?;
        self.rng = rng;
        self.enemy_content_death(w, r, id, credit)?;
        self.enemy_other_death(w, r, id)?;
        self.support_enemy_death(w, r, id)?;
        self.growth_death(w, id, credit)?;
        self.kit_event(w, r, &mut kits::dispatch::Event::Death { unit: id, credit })?;
        self.operator_enemy_death(w, id)
    }
}
impl buffs::Hooks for Host {
    fn field_deploy(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        fresh: bool,
    ) -> Result<(), String> {
        let u = w.unit(id)?;
        if fresh
            && u.deployment.remove_reason.as_deref() == Some("killed")
            && u.deployment.respawn_at > w.time + 1e-9
            && self.equipment_member(id, "raidShip")
        {
            self.bonds.raid_revived.insert(id, u.deployment.deploy_seq);
        }
        self.field_terrain(w, r, id, fresh)
    }

    fn operator_removed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if w.unit(id)?.deployment.kind == super::deploy::Kind::Operator {
            self.equipment.fields.clear();
        }
        self.advanced_removed(w, r, id)
    }

    fn heal_with_buffs(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Option<Id>,
        target: Id,
        amount: f64,
        options: health::HealOptions,
    ) -> Result<f64, String> {
        self.support_heal(w, r, source, target, amount, options)
    }

    fn operator_skill_effect(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        dir: super::direction::Direction,
        def: &super::operators::Definition,
        event: super::skills::Event,
        ctx: &mut super::skills::Context,
        skill: &mut super::skills::Runtime,
    ) -> Result<(), String> {
        self.component_skill(w, r, dir, def, event, ctx, skill)?;
        self.support_skill(w, r, dir, def, event, ctx, skill)?;
        self.advanced_event(w, r, dir, def, event, ctx, skill)?;
        self.kit_skill_event(w, r, def, event, ctx, skill)
    }

    fn operator_target_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        key: &str,
        duration: f64,
    ) -> Result<(), String> {
        self.apply_status(
            w,
            r,
            target,
            key,
            super::status::Options {
                duration,
                source: Some(source),
                ..Default::default()
            },
        )?;
        Ok(())
    }
    fn operator_release(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        blocking::release(
            w,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            id,
        )
    }
    fn operator_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
        duration: f64,
    ) -> Result<(), String> {
        self.apply_status(
            w,
            r,
            id,
            key,
            super::status::Options {
                duration,
                source: Some(id),
                ..Default::default()
            },
        )?;
        Ok(())
    }
    fn status(&mut self, _: Id, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        token: u64,
        event: buffs::Event,
        c: buffs::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if token == bonds::SIRACUSA_END {
            self.bond_siracusa_end(c.unit, w.time);
            return Ok(());
        }
        if token == bonds::SARGON_EXPIRE {
            return self.bond_sargon_sync(w, r, c.unit);
        }
        use super::content::operators::{DECAY, DOT};
        if token == super::equipment::PEGASUS_HEAL && event == buffs::Event::Tick {
            return self.equipment_pegasus_heal(w, r, c);
        }
        if token == super::equipment::DRAIN && event == buffs::Event::Tick {
            return self.equipment_drain(w, r, c);
        }
        if matches!(token, super::terrain::INFECTION | super::terrain::DEEPSEA)
            && event == buffs::Event::Tick
        {
            return self.terrain_damage(w, r, c, token == super::terrain::DEEPSEA);
        }
        if token == enemies::LANDING && event == buffs::Event::Expire {
            return self.enemy_landing(w, r, c.unit);
        }
        if token == enemies::WOLF_REBORN && event == buffs::Event::Tick {
            return self.wolf_reborn(w, r, c.unit);
        }
        if token == enemies::REBORN_FINISH && event == buffs::Event::Expire {
            return self.enemy_reborn_finish(w, r, c.unit);
        }
        if token == enemies::TRANSLATOR_FINISH && event == buffs::Event::Expire {
            return self.enemy_translator_finish(w, r, c.unit);
        }
        if token == enemies::XI_SHIELD && event == buffs::Event::Expire
            || token == enemies::FEAR_CAGE && event == buffs::Event::Tick
        {
            return self.champion_buff(w, r, c);
        }
        if token == enemies::SURGE_TICK && event == buffs::Event::Tick {
            return self.enemy_surge_tick(w, r, c);
        }
        if token == enemies::PART_DOT && event == buffs::Event::Tick {
            return self.helm_dot(w, r, c);
        }
        if token == enemies::BLADE_RETURN && event == buffs::Event::Expire {
            return self.blade_return(w, r, c.unit);
        }
        if token == enemies::PARASITE_TICK && event == buffs::Event::Tick {
            return self.enemy_parasite_tick(w, r, c);
        }
        if token == enemies::PILOT_BURN && event == buffs::Event::Tick {
            return self.enemy_pilot_tick(w, r, c);
        }
        if token == enemies::BURN_TICK && event == buffs::Event::Tick {
            return self.enemy_burn_tick(w, r, c);
        }
        if token == enemies::SAND_TICK && event == buffs::Event::Tick {
            return self.enemy_sand_tick(w, r, c);
        }
        if token == enemies::BLEED {
            return self.enemy_bleed_event(event, c, r, w);
        }
        if (1500..1600).contains(&token) {
            return self.kit_event(
                w,
                r,
                &mut kits::dispatch::Event::Buff {
                    context: c,
                    event,
                    token,
                },
            );
        }
        if matches!(
            token,
            tactics::RAIN | tactics::DRIZZLE | tactics::SHIELD_LOST | tactics::GAP_FINISH
        ) {
            return self.kit_buff_event(w, r, c, event, token);
        }
        if matches!(
            token,
            super::content::operators::HP_DRAIN | super::content::operators::PLASMA_DRAIN
        ) {
            return self.support_drain(w, r, c, token);
        }
        if event != buffs::Event::Tick {
            return Err("Unexpected operator buff callback".into());
        }
        match token {
            DECAY => {
                let b = r.get(w, c.unit, c.buff)?;
                let duration = b.runtime.duration;
                let step = b.runtime.tick_count as f64;
                let prev = (duration - step + 1.).max(0.);
                let next = (duration - step).max(0.);
                let start = self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(c.unit)?
                    .definition
                    .extra
                    .start_buff
                    .as_ref()
                    .ok_or("Missing decay descriptor")?;
                let def = start.def;
                r.edit(w, c.unit, c.buff, |b| {
                    if prev > 0. && b.shield > 0. {
                        b.shield *= next / prev;
                    }
                    if def > 0. {
                        b.mods = [("defPct".into(), def * next / duration)].into();
                    }
                })
            }
            DOT => {
                let b = r.get(w, c.unit, c.buff)?;
                let source = b.runtime.source.ok_or("Missing DoT source")?;
                let amount = b.runtime.tick_payload;
                let mut q = hit::Request::new(
                    health::Hit {
                        source: Some(source),
                        credit: Some(source),
                        target: c.unit,
                        damage_type: super::damage::DamageType::Arts,
                        no_sp: false,
                        silent: false,
                        hp_loss: false,
                        on_applied: false,
                    },
                    super::damage::Damage {
                        amount,
                        mul: 1.,
                        resolved: false,
                        penetration: Default::default(),
                    },
                );
                q.dodge_mode = super::injury::DodgeMode::Ignore;
                q.tags.push("dot".into());
                let result = self.resolve_hit(w, r, &mut q);
                result.map(|_| ())
            }
            _ => Err("Buff callback is not implemented".into()),
        }
    }
    fn burst_damage(
        &mut self,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<f64, String> {
        self.resolve_hit(w, r, q)
    }
    fn burst_sp_loss(&mut self, id: Id, amount: f64, w: &mut World) -> Result<(), String> {
        if w.unit(id)?.has_operator_kit() {
            self.roster
                .as_mut()
                .ok_or("Missing elemental SP context")?
                .drain_sp(w, id, amount)
        } else {
            super::element::drain_sp(w, None, amount)
        }
    }
}
impl blocking::Hooks for Host {
    fn unblock_with_buffs(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        blocking::unblock(
            w,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            id,
        )
    }
    fn release_with_buffs(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        blocking::release(
            w,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            id,
        )
    }
    fn blocked(&mut self, blocker: Id, id: Id, w: &mut World) -> Result<(), String> {
        self.device_blocked(w, blocker, id)?;
        self.enemy_special_blocked(id);
        Ok(())
    }
    fn ended(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        let mut r = self
            .buffs
            .take()
            .ok_or("Block restoration requires the active buff context")?;
        let result = blocking::restore_stealth(w, &mut r, self, id);
        self.buffs = Some(r);
        result
    }
}
impl motion::Hooks for Host {
    fn check_block(&mut self, id: Id, w: &mut World, g: &mut Grid) -> Result<bool, String> {
        let roster = self.roster.take().ok_or("Missing blocker context")?;
        let result = blocking::check(w, roster.deployment(), g, self, id);
        self.roster = Some(roster);
        result
    }
    fn hidden(&mut self, id: Id, on: bool, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.hidden = on;
        Ok(())
    }
    fn cut_attack_stand(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn leak(&mut self, id: Id, w: &mut World, rng: &mut Rng) -> Result<(), String> {
        let cost = if self.boss_round.is_some() {
            let enemy = self
                .spawns
                .as_ref()
                .unwrap()
                .ledger
                .alive()
                .get(&id)
                .ok_or("Missing leaking enemy")?;
            Some(if enemy.lpr >= 0. { enemy.lpr } else { 1. })
        } else {
            None
        };
        self.spawns.as_mut().unwrap().ledger.leak(id)?;
        if let Some(cost) = cost {
            self.boss_round
                .as_mut()
                .unwrap()
                .charge(boss_round::Cost::Leak, cost)?;
        }
        enemy::clear(w, id)?;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        u.removed = true;
        u.deployment.death_at = time;
        u.deployment.remove_reason = Some("leak".into());
        self.enemy_cancel_form_timers(w, id)?;
        self.helm_detach(w, id)?;
        self.enemy_unload(w, id, rng)?;
        Ok(())
    }
}
impl Queries for Host {
    fn skip_enemy(
        &mut self,
        source: Option<Id>,
        target: Id,
        world: &mut World,
    ) -> Result<bool, String> {
        if let Some(source) =
            source.filter(|&id| world.unit(id).is_ok_and(|u| u.side == health::Side::Ally))
        {
            if let Some(filter) = world.unit(source)?.targeting.enemy_filter {
                return filter(world, source, target);
            }
        }
        Ok(false)
    }
    fn remaining_distance(&mut self, id: Id, w: &mut World) -> Result<f64, String> {
        let grid = self.grid.as_mut().unwrap_or(&mut self.route_query_grid);
        motion::remaining_distance(w, grid, id)
    }
    fn body_distance(&mut self, id: Id, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        Ok(super::body::distance(w.unit(id)?, p))
    }
}
impl enemy::Hooks for Host {
    fn profile(&mut self, id: Id, _w: &mut World) -> Result<enemy::Profile, String> {
        let mut profile = self
            .spawns
            .as_ref()
            .unwrap()
            .definition(id)?
            .profile
            .clone();
        let kit = self.content.as_ref().unwrap().kit_ref(id);
        match kit {
            content::Kit::Ability(content::Ability::MaxTargets(n)) => {
                profile.max_targets = *n;
            }
            content::Kit::Ability(content::Ability::Nest { .. }) => profile.no_attack = true,
            content::Kit::Reaction(content::Reaction::Shell) => {
                profile.deferred = true;
                profile.shot = "mortar";
            }
            content::Kit::Reaction(content::Reaction::Blood { .. }) => profile.no_attack = true,
            _ => {}
        }
        self.enemy_movement_profile(id, kit, &mut profile);
        self.enemy_lifecycle_profile(id, kit, &mut profile);
        self.enemy_leader_profile(id, &mut profile);
        self.enemy_boss_profile(kit, &mut profile);
        self.enemy_caster_profile(id, kit, &mut profile);
        self.enemy_special_profile(id, kit, &mut profile);
        self.enemy_form_profile(id, &mut profile);
        Ok(profile)
    }
    fn allies_in_radius(
        &mut self,
        id: Id,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        spatial::allies_in_radius(
            w,
            self.roster.as_ref().unwrap().ids(),
            w.unit(id)?.position,
            radius,
            None,
            false,
            limit,
        )
    }
    fn enemies_in_radius(
        &mut self,
        id: Id,
        r: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        spatial::enemies_in_radius(
            w,
            self,
            &ids,
            w.unit(id)?.position,
            r,
            RadiusRule::Body,
            false,
            limit,
        )
    }
    fn can_target(&mut self, enemy: Id, ally: Id, w: &mut World) -> Result<bool, String> {
        let unit = w.unit(ally)?;
        Ok(match self.content.as_ref().unwrap().kit(enemy) {
            content::Kit::Boss(content::Boss::Blade(_)) => !super::targeting::flying(w, ally)?,
            content::Kit::Lifecycle(content::Lifecycle::Ugly { .. }) => !unit.motion_fly,
            content::Kit::Zone(content::ZoneKit::Tank(_)) => unit.ground && !unit.motion_fly,
            content::Kit::Caster(content::Caster {
                passive: content::casters::Passive::Tomb { .. },
                ..
            })
            | content::Kit::Zone(content::ZoneKit::Cannon { .. })
            | content::Kit::Leader(content::Leader::Skull { .. }) => !unit.motion_fly,
            content::Kit::Movement(
                content::Movement::Syufo { .. }
                | content::Movement::BoneSpike { .. }
                | content::Movement::Unbalanced(content::movement::Unbalance::LoseEgg(_)),
            ) => !unit.motion_fly,
            content::Kit::Reaction(content::Reaction::Fear(config)) if config.steal.is_some() => {
                !unit.motion_fly
            }
            content::Kit::Form(content::Form::Stone(_)) if w.unit(enemy)?.motion_fly => {
                !unit.motion_fly
            }
            _ => true,
        })
    }
    fn enemy_event_enabled(&self, event: enemy::Event) -> bool {
        matches!(event, enemy::Event::BeforeAttack | enemy::Event::Attack)
    }
    fn enemy_event(
        &mut self,
        event: enemy::Event,
        c: &mut enemy::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
        grid: &mut Grid,
        rng: &mut Rng,
    ) -> Result<(), String> {
        self.rng = rng.clone();
        let result = self.enemy_content_attack(event, c, r, w, Some(grid));
        *rng = self.rng.clone();
        result
    }
    fn attack_visual(&mut self, _: Id, _: Id, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn palsy_visual(&mut self, _: Id, _: &World) -> Result<(), String> {
        Ok(())
    }
    fn deliver(
        &mut self,
        d: enemy::Delivery,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if d.projectile {
            self.flights
                .as_mut()
                .ok_or("Reentrant projectile launch")?
                .add(
                    w,
                    super::projectiles::Launch {
                        from: w.unit(d.source)?.position,
                        source: Some(d.source),
                        target: Some(d.target),
                        to: None,
                        speed: 10.,
                        hit_dead: false,
                        max_age: 10.,
                        on_hit: true,
                        visual: "enemy".into(),
                        speed_changeable: None,
                        removable: None,
                        payload: allies::Shot::Enemy(d),
                    },
                )?;
        } else {
            enemy::impact(w, r, rng, self, d, Some(d.target))?;
        }
        Ok(())
    }
}
