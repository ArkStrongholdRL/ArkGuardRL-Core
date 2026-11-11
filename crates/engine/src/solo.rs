// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-player round execution and settlement. Wave generation and preparation
//! supply validated round plans and lineups. This is an internal match API.
use crate::{
    battle::{
        grid::{Grid, Rect},
        operators::Placement,
        runtime::{
            BossConfig, BossKind, BossOvertime, BossRoundConfig, BossRoundOutcome, Field,
            FieldResult, Limits,
        },
        waves::Template,
    },
    data::Catalog,
    deterministic::rng::derive_seed,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc};

pub mod draft;
mod map;
pub mod mode;
mod player;
use map::DeployMap;
pub mod preparation;
pub mod session;

const MAX_ROUNDS: usize = 64;
const MAX_LINEUP: usize = 36;
// PlayerState funds use JS integers. Retain exact integer arithmetic at the API.
const MAX_FUNDS: u64 = (1_u64 << 53) - 1;

pub struct Boss {
    pub max_hp: f64,
    pub overtime: Option<BossOvertime>,
}
pub enum RoundKind {
    Normal,
    FinalAssault(Boss),
    HiddenCore(Boss),
}
pub struct RoundPlan {
    pub kind: RoundKind,
    pub wave: Template,
    pub time_limit: f64,
    pub income: u64,
    pub deploy_cap: usize,
}
/// Eligibility is fixed by mode/difficulty. The preparation owner supplies the
/// active layer sum at Ready, before the Final Assault starts.
pub struct HiddenRule {
    pub layer_threshold: f64,
    pub min_lp_exclusive: f64,
}
pub struct Config {
    pub initial_lp: f64,
    pub lp_cap_per_round: u64,
    pub keep_funds: bool,
    pub hidden: Option<HiddenRule>,
}
pub struct Operator {
    pub key: String,
    pub placement: Placement,
    pub skill_index: Option<u32>,
    pub items: Vec<String>,
}
pub struct PreparedRound {
    pub operators: Vec<Operator>,
    pub active_layers: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Prep,
    Draft,
    Combat,
    Result,
    Faulted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Eliminated,
    Defeat,
    Victory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub victory: bool,
    pub hidden_reached: bool,
    pub hidden_cleared: bool,
    pub reason: EndReason,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub kills: u64,
    pub credited_kills: u64,
    pub leaks: u64,
    pub lp_lost: f64,
    pub perfect_rounds: u64,
    pub funds_gained: u64,
    pub funds_spent: u64,
    pub damage: f64,
    pub healing: f64,
    pub boss_damage: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub phase: Phase,
    pub round: u32,
    pub rounds_passed: u32,
    pub lp: f64,
    pub funds: u64,
    pub pending_funds: u64,
    pub stats: Stats,
    pub outcome: Option<Outcome>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    pub ticks: usize,
    pub budget_exhausted: bool,
    pub phase: Phase,
}
enum State {
    Prep,
    Combat {
        field: Box<Field>,
        active_layers: f64,
    },
    Result(Outcome),
    Faulted(String),
}
pub struct Runner {
    data: Arc<Catalog>,
    stage: Value,
    seed: u32,
    config: Config,
    plans: Vec<Option<RoundPlan>>,
    boss_index: usize,
    index: usize,
    state: State,
    player: player::Player,
    rounds_passed: u32,
    limits: Limits,
    last_result: Option<FieldResult>,
    prep: Option<preparation::Preparation>,
    map: crate::battle::runtime::MapConfig,
    draft: Option<draft::Flow>,
    core: Option<String>,
}
impl Runner {
    /// Configure an already selected core; offer generation belongs to the selection flow.
    pub fn configure_core(&mut self, id: &str) -> Result<(), String> {
        self.healthy()?;
        if self.index != 0 || self.phase() != Phase::Prep || self.core.is_some() {
            return Err("Core must be selected once before the first battle".into());
        }
        Field::validate_core(&self.data, id)?;
        let hp = self.data.table("bands").unwrap()[id]["totalHp"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.)
            .ok_or("Invalid core life points")?;
        self.player.lp = hp;
        if let Some(prep) = &mut self.prep { prep.sciurus = id == "band_sciurus"; }
        self.core = Some(id.into());
        Ok(())
    }

    /// Map effects persist across rounds and may change only during preparation.
    pub fn configure_map(&mut self, map: crate::battle::runtime::MapConfig) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Prep {
            return Err("Map configuration requires preparation".into());
        }
        let map = map.resolved(&self.data)?;
        let boss = !matches!(
            self.plans[self.index].as_ref().unwrap().kind,
            RoundKind::Normal
        );
        let grid = DeployMap::new(&self.stage, &map, boss)?;
        if let Some(prep) = &self.prep {
            let mut prep = prep.clone();
            let mut player = self.player.clone();
            match prep.change_map(grid, &mut player) {
                Ok(()) => {
                    self.prep = Some(prep);
                    self.player = player;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        self.map = map;
        Ok(())
    }

    pub fn new(
        data: Arc<Catalog>,
        stage: Value,
        seed: u32,
        config: Config,
        plans: Vec<RoundPlan>,
        limits: Limits,
    ) -> Result<Self, String> {
        Self::build(data, stage, seed, config, plans, limits, None)
    }
    pub fn new_with_preparation(
        data: Arc<Catalog>,
        stage: Value,
        seed: u32,
        config: Config,
        plans: Vec<RoundPlan>,
        limits: Limits,
        preparation: preparation::Config,
    ) -> Result<Self, String> {
        Self::build(data, stage, seed, config, plans, limits, Some(preparation))
    }
    fn build(
        data: Arc<Catalog>,
        stage: Value,
        seed: u32,
        config: Config,
        plans: Vec<RoundPlan>,
        limits: Limits,
        preparation: Option<preparation::Config>,
    ) -> Result<Self, String> {
        if plans.is_empty()
            || plans.len() > MAX_ROUNDS
            || !config.initial_lp.is_finite()
            || config.initial_lp <= 0.
            || config.lp_cap_per_round == 0
            || config.lp_cap_per_round > MAX_FUNDS
        {
            return Err("Invalid single-player round configuration".into());
        }
        Grid::new(&stage, Rect::FULL)?;
        let boss_index = plans
            .iter()
            .position(|p| matches!(p.kind, RoundKind::FinalAssault(_)))
            .ok_or("Single-player run requires a Final Assault")?;
        let hidden = plans.len() == boss_index + 2;
        if plans.len() != boss_index + 1 && !hidden
            || config.hidden.is_some() != hidden
            || plans[..boss_index]
                .iter()
                .any(|p| !matches!(p.kind, RoundKind::Normal))
            || hidden && !matches!(plans[boss_index + 1].kind, RoundKind::HiddenCore(_))
        {
            return Err(
                "Expected normal rounds, one Final Assault, then an optional Hidden Core".into(),
            );
        }
        if let Some(rule) = &config.hidden {
            if !rule.layer_threshold.is_finite()
                || rule.layer_threshold < 0.
                || !rule.min_lp_exclusive.is_finite()
                || rule.min_lp_exclusive < 0.
            {
                return Err("Invalid Hidden Core eligibility".into());
            }
        }
        for plan in &plans {
            if plan.time_limit.is_nan()
                || plan.time_limit <= 0.
                || plan.income > MAX_FUNDS
                || plan.deploy_cap == 0
                || plan.deploy_cap > MAX_LINEUP
            {
                return Err("Invalid single-player round plan".into());
            }
            if let RoundKind::FinalAssault(boss) | RoundKind::HiddenCore(boss) = &plan.kind {
                if !boss.max_hp.is_finite() || boss.max_hp < 1. {
                    return Err("Invalid single-player Boss pool".into());
                }
                // Validate the same settings as the public battle entry, without
                // constructing a second field or duplicating its LP policy.
                super::battle::runtime::validate_boss_round(BossRoundConfig {
                    life_points: config.initial_lp,
                    overtime: boss.overtime,
                })?;
            }
        }
        let lp = config.initial_lp;
        let mut run = Self {
            data,
            stage,
            seed: if seed == 0 { 1 } else { seed },
            config,
            plans: plans.into_iter().map(Some).collect(),
            boss_index,
            index: 0,
            state: State::Prep,
            player: player::Player {
                lp,
                ..Default::default()
            },
            rounds_passed: 0,
            limits,
            last_result: None,
            prep: None,
            map: Default::default(),
            draft: None,
            core: None,
        };
        if let Some(config) = preparation {
            let plan = run.plans[0].as_ref().unwrap();
            let boss = !matches!(plan.kind, RoundKind::Normal);
            run.prep = Some(preparation::Preparation::new(
                Arc::clone(&run.data),
                run.seed,
                config,
                DeployMap::new(&run.stage, &run.map, boss)?,
                boss,
                plan.deploy_cap,
                &mut run.player,
            )?);
        }
        run.start_prep()?;
        Ok(run)
    }
    pub fn phase(&self) -> Phase {
        match self.state {
            State::Prep if self.draft.as_ref().is_some_and(|d| d.pending.is_some()) => Phase::Draft,
            State::Prep => Phase::Prep,
            State::Combat { .. } => Phase::Combat,
            State::Result(_) => Phase::Result,
            State::Faulted(_) => Phase::Faulted,
        }
    }
    fn healthy(&self) -> Result<(), String> {
        if let State::Faulted(error) = &self.state {
            Err(error.clone())
        } else {
            Ok(())
        }
    }
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        self.healthy()?;
        let mut lp = self.player.lp;
        let mut stats = self.player.stats;
        if let State::Combat { field, .. } = &self.state {
            if let Some(round) = field.boss_round_state()? {
                if round.leak_loss > 0. || round.skill_loss > 0. || round.overtime_loss > 0. {
                    lp = round.lp.round();
                }
                stats.boss_damage = field.match_boss_damage()?;
            }
        }
        Ok(Snapshot {
            phase: self.phase(),
            round: self.index as u32 + 1,
            rounds_passed: self.rounds_passed,
            lp,
            funds: self.player.funds,
            pending_funds: self.player.pending,
            stats,
            outcome: if let State::Result(outcome) = self.state {
                Some(outcome)
            } else {
                None
            },
        })
    }
    /// Owned internal diagnostics. Preparation inventories and policy
    /// observations are assembled by their respective owners.
    pub fn report(&self) -> Result<Value, String> {
        let s = self.snapshot()?;
        let mut report = json!({"phase":match s.phase {Phase::Prep=>"prep",Phase::Draft=>"draft",Phase::Combat=>"combat",Phase::Result=>"result",Phase::Faulted=>"faulted"},
            "round":s.round,"roundsPassed":s.rounds_passed,"lp":s.lp,"funds":s.funds,"pendingFunds":s.pending_funds,
            "stats":{"kills":s.stats.kills,"creditedKills":s.stats.credited_kills,"leaks":s.stats.leaks,"lpLost":s.stats.lp_lost,
                "perfectRounds":s.stats.perfect_rounds,"fundsGained":s.stats.funds_gained,"fundsSpent":s.stats.funds_spent,
                "damage":s.stats.damage,"healing":s.stats.healing,"bossDamage":s.stats.boss_damage},
            "outcome":s.outcome.map(|o|json!({"victory":o.victory,"hiddenReached":o.hidden_reached,"hiddenCleared":o.hidden_cleared,
                "reason":match o.reason {EndReason::Eliminated=>"eliminated",EndReason::Defeat=>"defeat",EndReason::Victory=>"victory"}}))});
        if let Some(prep) = &self.prep {
            report["preparation"] = prep.report(&self.player);
        }
        if let Some(id) = &self.core {
            report["core"] = json!(id);
        }
        if let Some(draft) = &self.draft {
            report["draft"] = draft.pending.clone().unwrap_or(Value::Null);
            report["draftRng"] = json!(draft.rng.state());
        }
        Ok(report)
    }
    /// The preparation owner charges completed actions once through this entry.
    pub fn spend(&mut self, amount: u64) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Prep || amount > self.player.funds {
            return Err("Preparation payment rejected".into());
        }
        self.player.spend(amount)
    }
    pub fn enable_preparation(&mut self, config: preparation::Config) -> Result<(), String> {
        self.healthy()?;
        if self.index != 0 || self.phase() != Phase::Prep || self.prep.is_some() {
            return Err("Preparation must be enabled once before the first battle".into());
        }
        if config.effects.is_some() {
            return Err(
                "Preparation effects require new_with_preparation before initial income".into(),
            );
        }
        let mut player = self.player.clone();
        let plan = self.plans[0].as_ref().unwrap();
        let boss = !matches!(plan.kind, RoundKind::Normal);
        let grid = DeployMap::new(&self.stage, &self.map, boss)?;
        let mut prep = preparation::Preparation::new(
            Arc::clone(&self.data),
            self.seed,
            config,
            grid.clone(),
            boss,
            plan.deploy_cap,
            &mut player,
        )?;
        prep.begin_round(1, grid, boss, plan.deploy_cap, &mut player, false)
            .map_err(|e| e.to_string())?;
        self.prep = Some(prep);
        self.player = player;
        Ok(())
    }
    pub fn prep_action(&mut self, action: preparation::Action) -> Result<(), String> {
        self.healthy()?;
        if let preparation::Action::Draft(index) = action {
            return self.pick_draft(index);
        }
        if self.phase() != Phase::Prep {
            return Err("WRONG_PHASE".into());
        }
        // Immutable definitions and the deployment map are shared by Arc. The
        // bounded mutable preparation draft commits with its wallet and RNG.
        let mut next = self
            .prep
            .as_ref()
            .ok_or("Preparation is not enabled")?
            .clone();
        let mut player = self.player.clone();
        match next.act(action, &mut player) {
            Ok(()) => {
                self.prep = Some(next);
                self.player = player;
                Ok(())
            }
            Err(error @ (preparation::Error::Capacity | preparation::Error::Content(_))) => {
                self.fail(error.to_string())
            }
            Err(error) => Err(error.to_string()),
        }
    }
    pub fn ready_owned(&mut self, active_layers: f64) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Prep {
            return Err("WRONG_PHASE".into());
        }
        if self
            .prep
            .as_ref()
            .ok_or("Preparation is not enabled")?
            .has_effects()
        {
            return Err("Preparation effects require ready_current".into());
        }
        self.ready_preparation(Some(active_layers))
    }
    /// Derive Hidden Core eligibility from the owned preparation state.
    pub fn ready_current(&mut self) -> Result<(), String> {
        self.ready_preparation(None)
    }
    fn ready_preparation(&mut self, layers: Option<f64>) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Prep {
            return Err("WRONG_PHASE".into());
        }
        let mut next = self
            .prep
            .as_ref()
            .ok_or("Preparation is not enabled")?
            .clone();
        next.check_ready()?;
        let mut player = self.player.clone();
        if let Err(error) = next.meta_end(&mut player) {
            return self.fail(error.to_string());
        }
        let prepared = next.prepared(layers.unwrap_or_else(|| next.active_layers()));
        let mut map = self.map.clone();
        next.battle_map(&mut map);
        self.ready_inner_with_map(prepared, map, next.battle_choices(), next.battle_bonds())?;
        if !self.config.keep_funds {
            player.funds = 0;
        }
        next.end_prep();
        self.prep = Some(next);
        self.player = player;
        Ok(())
    }
    /// Board positions stay in preparation coordinates, rows 9–12, cols 2–10.
    /// Readiness consumes the plan only after all lineup validation succeeds.
    pub fn ready(&mut self, prepared: PreparedRound) -> Result<(), String> {
        if self.prep.is_some() {
            return Err("Owned preparation requires ready_owned".into());
        }
        self.ready_inner(prepared)
    }
    fn ready_inner(&mut self, prepared: PreparedRound) -> Result<(), String> {
        self.ready_inner_with_map(prepared, self.map.clone(), vec![], vec![])
    }
    fn ready_inner_with_map(
        &mut self,
        prepared: PreparedRound,
        map: crate::battle::runtime::MapConfig,
        choices: Vec<crate::battle::runtime::BattleChoice>,
        bonds: Vec<crate::battle::runtime::BattleBond>,
    ) -> Result<(), String> {
        self.healthy()?;
        if self.phase() != Phase::Prep {
            return Err("Ready requires preparation".into());
        }
        let plan = self.plans[self.index].as_ref().unwrap();
        if !prepared.active_layers.is_finite()
            || prepared.active_layers < 0.
            || prepared.operators.len()
                > self
                    .prep
                    .as_ref()
                    .map_or(plan.deploy_cap, preparation::Preparation::deploy_cap)
        {
            return Err("Invalid prepared lineup or active layers".into());
        }
        let boss = !matches!(plan.kind, RoundKind::Normal);
        let grid = DeployMap::new(&self.stage, &map, boss)?;
        let tokens = self
            .prep
            .as_ref()
            .map(|prep| prep.prepared_tokens())
            .unwrap_or_default();
        let mut identities = BTreeSet::new();
        let mut cells = BTreeSet::new();
        for op in &prepared.operators {
            let home = op.placement.home;
            if op.placement.uid == 0
                || !identities.insert(op.placement.uid)
                || !cells.insert(home)
                || !(9..=12).contains(&home[0])
                || !(2..=10).contains(&home[1])
            {
                return Err("Invalid or duplicate prepared placement".into());
            }
            crate::battle::operators::Definition::load_with_loadout(
                &self.data,
                &op.key,
                op.skill_index,
                &op.items,
            )?;
            let record = &self.data.table("chess").unwrap()[&op.key];
            if !grid.can_place(record, home) {
                return Err("Prepared operator cannot stand on its deployment tile".into());
            }
        }
        let mut plan = self.plans[self.index].take().unwrap();
        if let Some(prep) = &self.prep {
            if let Err(error) = prep.append_bounties(&mut plan.wave, self.index + 1, boss) {
                return self.fail(error);
            }
        }
        let make = || {
            let mut field = match plan.kind {
                RoundKind::Normal => Field::new(
                    Arc::clone(&self.data),
                    &self.stage,
                    plan.wave,
                    derive_seed(self.seed, &format!("n:{}:0", self.index + 1)),
                    plan.time_limit,
                    self.limits,
                )?,
                RoundKind::FinalAssault(boss) | RoundKind::HiddenCore(boss) => {
                    let hidden = self.index > self.boss_index;
                    let mut field = Field::new_boss(
                        Arc::clone(&self.data),
                        &self.stage,
                        plan.wave,
                        derive_seed(self.seed, &format!("b1:{}", self.index + 1)),
                        plan.time_limit,
                        self.limits,
                        BossConfig {
                            kind: if hidden {
                                BossKind::HiddenCore
                            } else {
                                BossKind::FinalAssault
                            },
                            pool: crate::battle::health::Pool {
                                hp: boss.max_hp,
                                max_hp: boss.max_hp,
                            },
                        },
                    )?;
                    field.configure_boss_round(BossRoundConfig {
                        life_points: self.player.team_lp.unwrap_or(self.player.lp),
                        overtime: boss.overtime,
                    })?;
                    field.inherit_boss_damage(self.player.stats.boss_damage)?;
                    field
                }
            };
            let follow_wind =
                self.prep.is_some() && self.map.cold_wind.is_none() && map.cold_wind.is_some();
            field.configure_map(map)?;
            field.configure_choices(&choices)?;
            field.configure_bonds(&bonds)?;
            if let Some(id) = &self.core {
                field.configure_core(id)?;
            }
            if follow_wind {
                field.follow_cold_wind_layers();
            }
            field.configure_round_gained_chess(
                self.prep
                    .as_ref()
                    .map_or(0, preparation::Preparation::gained_chess),
            )?;
            // Preserve board reading order for unit identities and update order.
            // Owners can follow their tokens, so bind them after registration.
            let mut lineup = prepared
                .operators
                .iter()
                .enumerate()
                .map(|(i, op)| (op.placement.home, false, i))
                .chain(
                    tokens
                        .iter()
                        .enumerate()
                        .map(|(i, token)| (token.placement.home, true, i)),
                )
                .collect::<Vec<_>>();
            lineup.sort_by_key(|(home, _, _)| (-home[0], home[1]));
            let mut bindings = Vec::new();
            for (_, is_token, index) in lineup {
                if is_token {
                    let token = &tokens[index];
                    let owner = prepared
                        .operators
                        .iter()
                        .find(|op| op.placement.uid == token.owner_uid)
                        .ok_or("Prepared summon owner is missing")?;
                    let mut placement = crate::battle::operators::Placement {
                        uid: token.placement.uid,
                        home: token.placement.home,
                        direction: token.placement.direction,
                    };
                    if boss {
                        placement.home[0] -= 7;
                    }
                    let id = field.add_prepared_token(&token.key, placement, &owner.key)?;
                    bindings.push((id, token.owner_uid));
                } else {
                    let op = &prepared.operators[index];
                    let mut placement = crate::battle::operators::Placement {
                        uid: op.placement.uid,
                        home: op.placement.home,
                        direction: op.placement.direction,
                    };
                    if boss {
                        placement.home[0] -= 7;
                    }
                    let id = field.add_operator_with_loadout(
                        &op.key,
                        placement,
                        op.skill_index,
                        &op.items,
                    )?;
                    if self.prep.is_some() && self.data.table("chess").unwrap()[&op.key]["diyProto"] != true {
                        field.configure_operator_cultivation(id, 3)?;
                    }
                }
            }
            for (token, owner) in bindings {
                field.bind_prepared_token(token, owner)?;
            }
            Ok::<_, String>(field)
        };
        match make() {
            Ok(field) => {
                if !self.config.keep_funds {
                    self.player.funds = 0;
                }
                self.state = State::Combat {
                    field: Box::new(field),
                    active_layers: prepared.active_layers,
                };
                Ok(())
            }
            Err(error) => self.fail(error),
        }
    }
    /// Stops at the next preparation or result. A tick budget never settles an
    /// unfinished field and never consumes the next round's preparation.
    pub fn advance(&mut self, budget: usize) -> Result<Advance, String> {
        self.healthy()?;
        let State::Combat {
            field,
            active_layers,
        } = &mut self.state
        else {
            return Ok(Advance {
                ticks: 0,
                budget_exhausted: false,
                phase: self.phase(),
            });
        };
        let layers = *active_layers;
        let advance = match field.run(budget) {
            Ok(value) => value,
            Err(error) => return self.fail(error),
        };
        if advance.terminated {
            let result = match field.result() {
                Ok(value) => value,
                Err(error) => return self.fail(error),
            };
            let gains = field.layer_gains();
            if let Err(error) = self.settle(result, layers, &gains) {
                return self.fail(error);
            }
            if ![
                self.player.stats.damage,
                self.player.stats.healing,
                self.player.stats.boss_damage,
            ]
            .iter()
            .all(|n| n.is_finite())
            {
                return self.fail("Single-player combat statistics overflow".into());
            }
        }
        Ok(Advance {
            ticks: advance.ticks,
            budget_exhausted: advance.budget_exhausted,
            phase: self.phase(),
        })
    }
    pub fn field(&self) -> Result<Option<&Field>, String> {
        self.healthy()?;
        Ok(if let State::Combat { field, .. } = &self.state {
            Some(field)
        } else {
            None
        })
    }
    pub fn last_result(&self) -> Result<Option<FieldResult>, String> {
        self.healthy()?;
        Ok(self.last_result)
    }
    fn start_prep(&mut self) -> Result<(), String> {
        let income = self.plans[self.index].as_ref().unwrap().income;
        self.player.start_round(income)?;
        if let Some(prep) = &mut self.prep {
            let plan = self.plans[self.index].as_ref().unwrap();
            let boss = !matches!(plan.kind, RoundKind::Normal);
            prep.begin_round(
                self.index as u32 + 1,
                DeployMap::new(&self.stage, &self.map, boss)?,
                boss,
                plan.deploy_cap,
                &mut self.player,
                self.draft.is_some(),
            )
            .map_err(|e| e.to_string())?;
            if let Some(draft) = &mut self.draft {
                draft.pending =
                    prep.draw_draft(&mut draft.rng, &draft.stage, self.index as u32 + 1)?;
                if draft.pending.is_none() {
                    prep.enter_prep(&mut self.player)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.state = State::Prep;
        Ok(())
    }
    fn settle(
        &mut self,
        result: FieldResult,
        layers: f64,
        gains: &[(String, f64)],
    ) -> Result<(), String> {
        self.player.stats.kills = add(self.player.stats.kills, result.killed)?;
        self.player.stats.credited_kills =
            add(self.player.stats.credited_kills, result.credited_kills)?;
        self.player.stats.damage += result.damage;
        // The source records bounty earnings now and again when pending funds
        // reach the next preparation wallet.
        let bonus = if let Some(prep) = &mut self.prep {
            prep.settle_bounties(
                result.counted_leaks == 0
                    && result
                        .boss_round
                        .is_none_or(|b| b.outcome == Some(BossRoundOutcome::Victory)),
            )?
        } else {
            0.
        };
        self.player.bounty(result.coins + bonus)?;
        if let Some(prep) = &mut self.prep {
            prep.settle_layers(gains, &mut self.player)?;
        }
        self.last_result = Some(result);
        if let Some(prep) = &mut self.prep {
            prep.equipment_battle_result(&mut self.player)
                .map_err(|e| e.to_string())?;
        }
        if self.index < self.boss_index {
            let loss = result.counted_leaks.min(self.config.lp_cap_per_round) as f64;
            self.player.lp -= loss;
            self.player.stats.lp_lost += loss;
            self.player.stats.leaks = add(self.player.stats.leaks, result.counted_leaks)?;
            self.player.stats.healing += result.healing;
            self.player.stats.perfect_rounds += u64::from(result.counted_leaks == 0);
            if self.player.lp <= 0. {
                self.player.lp = 0.;
                self.player.clear();
                if let Some(prep) = &mut self.prep {
                    prep.eliminate();
                }
                self.finish(false, false, EndReason::Eliminated);
                return Ok(());
            }
            self.rounds_passed = self.index as u32 + 1;
        } else {
            let round = result.boss_round.unwrap();
            self.player.stats.boss_damage = result.match_boss_damage;
            self.player.team_lp = Some(round.lp);
            self.player.lp = round.lp.round();
            let won = round.outcome == Some(BossRoundOutcome::Victory);
            if self.index > self.boss_index {
                self.rounds_passed = self.boss_index as u32 + 1 + u32::from(won);
                self.finish(true, won, EndReason::Victory);
                return Ok(());
            }
            if !won {
                self.finish(false, false, EndReason::Defeat);
                return Ok(());
            }
            self.rounds_passed = self.index as u32 + 1;
            let hidden = self.config.hidden.as_ref().is_some_and(|rule| {
                layers > rule.layer_threshold && round.lp > rule.min_lp_exclusive
            });
            if !hidden {
                self.finish(true, false, EndReason::Victory);
                return Ok(());
            }
        }
        self.index += 1;
        self.start_prep()
    }
    fn finish(&mut self, victory: bool, hidden_cleared: bool, reason: EndReason) {
        self.state = State::Result(Outcome {
            victory,
            hidden_reached: self.index > self.boss_index,
            hidden_cleared,
            reason,
        });
    }
    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.state = State::Faulted(error.clone());
        Err(error)
    }
}
fn add(a: u64, b: u64) -> Result<u64, String> {
    a.checked_add(b)
        .filter(|&n| n <= MAX_FUNDS)
        .ok_or_else(|| "Single-player accounting capacity exceeded".into())
}
