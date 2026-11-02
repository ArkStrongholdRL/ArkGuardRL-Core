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
mod allies;

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
    world: World,
    host: Host,
    commitments: super::search::Commitments,
    time_limit: f64,
    unit_limit: usize,
}
struct Host {
    data: Arc<Catalog>,
    grid: Option<Grid>,
    roster: Option<super::operators::Roster>,
    index: Option<spatial::Index>,
    executor: Option<super::attacks::Executor>,
    flights: Option<super::projectiles::System<allies::Shot>>,
    spawns: Option<super::spawns::Runtime>,
    content: Option<content::Runtime>,
    buffs: Option<buffs::Runtime>,
    rng: Rng,
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
        let data = Arc::clone(&self.host.data);
        r.install_content(&data, w, self.buffs, self.content, self.host, id)
    }
    fn release(&mut self, id: Id, _: &mut World) -> Result<(), String> {
        self.content.remove(id);
        Ok(())
    }
}
impl Field {
    /// `stage` and `template` come from external data. Active combat devices and
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
        let mut grid = Grid::new(&normalized, Rect::NORMAL).map_err(str::to_string)?;
        for r in Rect::NORMAL.r0..=Rect::NORMAL.r1 {
            for c in Rect::NORMAL.c0..=Rect::NORMAL.c1 {
                if grid.tiles()[(r as usize) * COLS + c as usize]
                    .terrain
                    .is_some()
                {
                    return Err("Field terrain effects are not implemented".into());
                }
            }
        }
        for d in normalized["devices"].as_array().into_iter().flatten() {
            let active = d["active"].as_bool().unwrap_or(d["hidden"] != true);
            let at = [
                d["row"]
                    .as_i64()
                    .or_else(|| d["pos"][0].as_i64())
                    .unwrap_or(-1) as i32,
                d["col"]
                    .as_i64()
                    .or_else(|| d["pos"][1].as_i64())
                    .unwrap_or(-1) as i32,
            ];
            if active && inside(grid.rect(), at) {
                if matches!(d["role"].as_str(), Some("platform" | "mound")) {
                    grid.set_obstacle(at, true, super::grid::BLOCK)
                        .map_err(str::to_string)?;
                } else {
                    return Err(format!("Stage device not implemented: {}", d["key"]));
                }
            }
        }
        if !template.extra_routes.is_empty() {
            return Err("Additional content routes are not implemented in this field entry".into());
        }
        for s in &template.spawns {
            if s["tag"] == "boss" || s["tag"] == "part" {
                return Err("Boss and part fields are not implemented".into());
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
        let ally_limit = limits.units.min(36);
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
        Ok(Self {
            clock: Clock::new(),
            core: Core {
                world: World::new(limits.units, 1, 1)?,
                host: Host {
                    data,
                    grid: Some(grid),
                    roster: Some(roster),
                    index: Some(spatial::Index::new(
                        limits.units,
                        limits.units * super::grid::N,
                    )?),
                    executor: Some(super::attacks::Executor::new(limits.units, 1)?),
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
        })
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
        self.healthy()?;
        if self.ticks() != 0 || self.clock.started() || self.finished() {
            return Err("Operators must be installed before field start".into());
        }
        let def = super::operators::Definition::load(&self.core.host.data, key)?;
        let grid = self.core.host.grid.take().unwrap();
        if grid.obstacle(placement.home) != 0 {
            self.core.host.grid = Some(grid);
            return Err("Operator deployment on a combat device is not implemented".into());
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
        self.faulted |= roster.faulted();
        self.core.host.roster = Some(roster);
        self.core.host.buffs = Some(r);
        self.core.host.grid = Some(grid);
        result
    }
    pub fn unit(&self, id: Id) -> Result<&Unit, String> {
        self.core.world.unit(id)
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
        let result = self.clock.run_steps(&mut self.core, budget);
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
        let mut rng = self.core.host.rng.clone();
        let result = settlement::resolve(
            &mut self.core.world,
            &mut buffs,
            &mut rng,
            &mut self.core.host,
            request,
        );
        self.core.host.buffs = Some(buffs);
        self.core.host.rng = rng;
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
        if matches!(options["tag"].as_str(), Some("boss" | "part")) {
            return Err("Boss and part fields are not implemented".into());
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
        )?;
        self.core.world.time = self.clock.time();
        let mut spawns = self.core.host.spawns.take().unwrap();
        let mut buffs = self.core.host.buffs.take().unwrap();
        let mut content = self.core.host.content.take().unwrap();
        let grid = self.core.host.grid.take().unwrap();
        let result = spawns.spawn(
            &mut self.core.world,
            &grid,
            &mut Spawn {
                host: &mut self.core.host,
                buffs: &mut buffs,
                content: &mut content,
            },
            key,
            options,
        );
        self.core.host.spawns = Some(spawns);
        self.core.host.buffs = Some(buffs);
        self.core.host.content = Some(content);
        self.core.host.grid = Some(grid);
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    /// Owned accounting snapshot. This internal field result is not a policy
    /// observation and exposes no provisional success for a faulted execution.
    pub fn report(&self) -> Result<Value, String> {
        self.healthy()?;
        let reason = self.reason().map(|e| match e {
            End::Cleared => "cleared",
            End::Timeout => "timeout",
            End::Forced => "forced",
        });
        Ok(self
            .core
            .host
            .spawns
            .as_ref()
            .unwrap()
            .ledger
            .report(self.time(), reason))
    }
}
fn inside(r: Rect, p: [i32; 2]) -> bool {
    p[0] >= r.r0 && p[0] <= r.r1 && p[1] >= r.c0 && p[1] <= r.c1
}
impl clock::Host for Core {
    fn phase(&mut self, phase: Phase, c: &mut Clock) -> Result<(), String> {
        self.world.time = c.time();
        match phase {
            Phase::Start => {
                let grid = self.host.grid.take().unwrap();
                let mut roster = self.host.roster.take().unwrap();
                let mut r = self.host.buffs.take().unwrap();
                let result = roster.deploy_initial(&mut self.world, &mut r, &mut self.host, &grid);
                self.host.roster = Some(roster);
                self.host.buffs = Some(r);
                self.host.grid = Some(grid);
                result?;
                c.mark_started();
            }
            Phase::Spawns => {
                let mut spawns = self.host.spawns.take().unwrap();
                let mut buffs = self.host.buffs.take().unwrap();
                let mut content = self.host.content.take().unwrap();
                let grid = self.host.grid.take().unwrap();
                let result = spawns.process(
                    &mut self.world,
                    &grid,
                    &mut Spawn {
                        host: &mut self.host,
                        buffs: &mut buffs,
                        content: &mut content,
                    },
                );
                self.host.spawns = Some(spawns);
                self.host.buffs = Some(buffs);
                self.host.content = Some(content);
                self.host.grid = Some(grid);
                result?;
            }
            Phase::Dp => self.host.roster.as_mut().unwrap().recover_dp(DT)?,
            Phase::Buffs => {
                let mut ids = self.host.roster.as_ref().unwrap().ids().to_vec();
                ids.extend_from_slice(self.host.spawns.as_ref().unwrap().live());
                let mut r = self.host.buffs.take().unwrap();
                let result = r.tick(&mut self.world, &mut self.host, &ids, DT);
                self.host.buffs = Some(r);
                result?;
            }
            Phase::Enemies => {
                let ids = self.host.spawns.as_ref().unwrap().live().to_vec();
                let mut r = self.host.buffs.take().unwrap();
                let mut rng = self.host.rng.clone();
                let mut grid = self.host.grid.take().unwrap();
                let result = (|| {
                    for id in ids {
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
                    Ok::<_, String>(())
                })();
                self.host.buffs = Some(r);
                self.host.rng = rng;
                self.host.grid = Some(grid);
                result?;
            }
            Phase::AttackCommit => {
                self.host.rebuild_index(&self.world)?;
                let ids = self.host.roster.as_ref().unwrap().ids().to_vec();
                self.commitments
                    .capture(&mut self.world, &mut self.host, &ids, DT)?;
            }
            Phase::EnemyIndex => self.host.rebuild_index(&self.world)?,
            Phase::Allies => {
                let ids = self.host.roster.as_ref().unwrap().ids().to_vec();
                for id in ids {
                    if !self.world.unit(id)?.stats.alive || !self.world.unit(id)?.deployed {
                        continue;
                    }
                    let mut skill = self.host.roster.as_mut().unwrap().take_skill(id)?;
                    let result = (|| {
                        skill.tick(&mut self.world, &mut self.host, DT)?;
                        super::ally::update(
                            &mut self.world,
                            &mut self.host,
                            &self.commitments,
                            id,
                            Some(&mut skill),
                            DT,
                            self.unit_limit,
                        )
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
                let grid = self.host.grid.take().unwrap();
                let mut roster = self.host.roster.take().unwrap();
                let mut r = self.host.buffs.take().unwrap();
                let result = roster.check_redeploys(&mut self.world, &mut r, &mut self.host, &grid);
                self.host.roster = Some(roster);
                self.host.buffs = Some(r);
                self.host.grid = Some(grid);
                result?;
            }
            Phase::TickHook => {
                let ids = self.host.spawns.as_ref().unwrap().live().to_vec();
                let mut r = self.host.buffs.take().unwrap();
                let mut content = self.host.content.take().unwrap();
                let result = content.tick(&mut self.world, &mut r, &mut self.host, &ids, DT);
                self.host.buffs = Some(r);
                self.host.content = Some(content);
                result?;
            }
            // Timers, boss pools and custom content hooks are not admitted here.
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
        let mut spawns = self.host.spawns.take().unwrap();
        let mut buffs = self.host.buffs.take().unwrap();
        let mut content = self.host.content.take().unwrap();
        let result = spawns.release_removed(
            &mut self.world,
            &mut Spawn {
                host: &mut self.host,
                buffs: &mut buffs,
                content: &mut content,
            },
        );
        self.host.spawns = Some(spawns);
        self.host.buffs = Some(buffs);
        self.host.content = Some(content);
        result
    }
    fn has_tick_hook(&self) -> bool {
        true
    }
    fn end_state(&self) -> Option<EndState> {
        let r = self.host.spawns.as_ref().unwrap();
        Some(EndState {
            shared_hp: None,
            boss_like: false,
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
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Enemy buff callback is not implemented".into())
    }
    fn burst_damage(
        &mut self,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<f64, String> {
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, r, &mut rng, self, q);
        self.rng = rng;
        result
    }
    fn burst_sp_loss(&mut self, id: Id, amount: f64, w: &mut World) -> Result<(), String> {
        if w.unit(id)?.side == health::Side::Ally {
            self.roster
                .as_mut()
                .ok_or("Missing elemental SP context")?
                .drain_sp(w, id, amount)
        } else {
            super::element::drain_sp(w, None, amount)
        }
    }
}
impl settlement::Hooks for Host {
    fn phase_enabled(&self, p: hit::Phase) -> bool {
        p == hit::Phase::Hit
    }
    fn damage_phase(
        &mut self,
        p: hit::Phase,
        q: &mut hit::Request,
        _: &mut f64,
        _: &mut buffs::Runtime,
        w: &mut World,
        _: &mut Rng,
    ) -> Result<(), String> {
        if p == hit::Phase::Hit && q.is_attack && w.unit(q.hit.target)?.side == health::Side::Enemy
        {
            if let Some(id) = q.hit.source {
                if w.unit(id)?.side == health::Side::Ally
                    && w.unit(q.hit.target)?.blocked_by().is_none()
                {
                    q.damage.amount *= self
                        .roster
                        .as_ref()
                        .ok_or("Missing operator hit context")?
                        .entry(id)?
                        .definition
                        .combat
                        .unblocked_scale;
                }
            }
        }
        Ok(())
    }
    fn health_kill(
        &mut self,
        id: Id,
        credit: Option<Id>,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if w.unit(id)?.side == health::Side::Enemy {
            return self.kill_enemy(id, credit, r, w);
        }
        let mut roster = self.roster.take().ok_or("Missing operator death context")?;
        let result = roster.kill(w, r, self, id, credit);
        self.roster = Some(roster);
        result
    }
    fn health_event(
        &mut self,
        e: &mut health::Event,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if e.stage == health::Stage::SkillDamaged {
            let mut roster = self
                .roster
                .take()
                .ok_or("Missing operator damage context")?;
            let result = roster.damaged(w, r, self, e.target);
            self.roster = Some(roster);
            result?;
        }
        Ok(())
    }
    fn damage_health_event(
        &mut self,
        e: &mut health::Event,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if e.stage == health::Stage::Damaged {
            if let Some(mut effect) = self.content.as_ref().unwrap().damaged(w, q)? {
                self.rng = rng.clone();
                let result = super::element::apply(w, self, r, &mut effect);
                *rng = self.rng.clone();
                result?;
            }
        }
        self.health_event(e, r, w)
    }
}
impl super::status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        false
    }
    fn before_status(
        &mut self,
        _: &mut super::status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Enemy before-status hook not implemented".into())
    }
    fn has_status_applied(&self) -> bool {
        false
    }
    fn status_applied(
        &mut self,
        _: &super::status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Enemy status-applied hook not implemented".into())
    }
}
impl super::element::Hooks for Host {
    fn battle_rect(&self) -> Rect {
        Rect::NORMAL
    }
    fn element_enabled(&self, _: super::element::Phase) -> bool {
        false
    }
    fn element_event(
        &mut self,
        _: super::element::Phase,
        _: &mut super::element::Request,
        _: f64,
        _: f64,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Enemy element hook not implemented".into())
    }
    fn element_number(&mut self, _: Id, _: f64, _: super::element::Element) -> Result<(), String> {
        Ok(())
    }
    fn burst_visual(&mut self, _: Id, _: super::element::Element, _: &World) -> Result<(), String> {
        Ok(())
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
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(()) // Current default operator kits have no block-event handlers.
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
    fn leak(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        self.spawns.as_mut().unwrap().ledger.leak(id)?;
        enemy::clear(w, id)?;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        u.removed = true;
        u.deployment.death_at = time;
        u.deployment.remove_reason = Some("leak".into());
        Ok(())
    }
}
impl Queries for Host {
    fn skip_enemy(&mut self, _: Option<Id>, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, id: Id, w: &mut World) -> Result<f64, String> {
        motion::remaining_distance(w, self.grid.as_mut().ok_or("Missing route query grid")?, id)
    }
    fn body_distance(&mut self, id: Id, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        Ok(super::body::distance(w.unit(id)?, p))
    }
}
impl enemy::Hooks for Host {
    fn profile(&mut self, id: Id, _: &mut World) -> Result<enemy::Profile, String> {
        Ok(self
            .spawns
            .as_ref()
            .unwrap()
            .definition(id)?
            .profile
            .clone())
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
    fn can_target(&mut self, _: Id, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(true)
    }
    fn enemy_event_enabled(&self, _: enemy::Event) -> bool {
        false
    }
    fn enemy_event(
        &mut self,
        _: enemy::Event,
        _: &mut enemy::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
        _: &mut Grid,
        _: &mut Rng,
    ) -> Result<(), String> {
        Err("Enemy event callback not implemented".into())
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
