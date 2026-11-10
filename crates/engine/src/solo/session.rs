// SPDX-License-Identifier: GPL-3.0-or-later
//! Persistent native execution boundary. Diagnostics are NOT policy observations.
//! A tick budget suspends combat; it never fabricates an episode truncation.
use super::{preparation::Action, Phase, Runner};
use crate::data::Catalog;
use serde_json::{json, Value};
use std::sync::Arc;
mod setup;
mod spatial;
mod view;

/// Bounded work per call; callers can resume the same battle with advance().
pub const MAX_TICK_BUDGET: usize = 108_000;

pub struct Session {
    run: Runner,
    revision: u64,
    fault: Option<String>,
    public_setup: Value,
    preview: Vec<Value>,
    spatial: std::sync::Mutex<spatial::Cache>,
}
impl Session {
    /// Only Desperation is supported. Core choice is an experiment parameter.
    pub fn new(
        data: Arc<Catalog>,
        seed: u32,
        core: &str,
        banned: &[String],
    ) -> Result<Self, String> {
        Self::new_at_revision(data, seed, core, banned, 0)
    }
    pub fn new_at_revision(
        data: Arc<Catalog>,
        seed: u32,
        core: &str,
        banned: &[String],
        setup_revision: u32,
    ) -> Result<Self, String> {
        let (run, public_setup) = setup::start(data, seed, core, banned, setup_revision)?;
        let mut session = Self::from_runner(run);
        session.public_setup = public_setup;
        Ok(session)
    }
    pub(in crate::solo) fn from_runner(run: Runner) -> Self {
        Self {
            run,
            revision: 0,
            fault: None,
            public_setup: json!({}),
            preview: vec![],
            spatial: std::sync::Mutex::new(spatial::Cache::default()),
        }
    }
    fn healthy(&self) -> Result<(), String> {
        if let Some(error) = &self.fault {
            Err(error.clone())
        } else {
            Ok(())
        }
    }
    fn check_budget(&self, budget: usize) -> Result<(), String> {
        self.healthy()?;
        if budget > MAX_TICK_BUDGET {
            return Err("Tick budget exceeds per-call limit".into());
        }
        if self.revision == u64::MAX {
            return Err("Session revision exhausted".into());
        }
        Ok(())
    }
    fn fail<T>(&mut self, error: String) -> Result<T, String> {
        self.fault = Some(error.clone());
        Err(error)
    }
    /// Internal inspection only: includes pool and RNG state, never feed to a policy.
    pub fn diagnostics(&self) -> Result<Value, String> {
        self.healthy()?;
        self.run.report()
    }
    pub fn replay_frame(&self) -> Result<Value, String> {
        self.healthy()?;
        match self.run.field()? {
            Some(field) => {
                let mut frame = field.replay_frame()?;
                frame["round"] = json!(self.run.snapshot()?.round);
                Ok(frame)
            }
            None => Ok(Value::Null),
        }
    }
    /// Diagnostic only. None means that no battle is currently suspended.
    pub fn movement_batch_ticks(&self) -> Result<Option<u64>, String> {
        self.healthy()?;
        Ok(self.run.field()?.map(|field| field.movement_batch_ticks()))
    }
    pub fn movement_path_steps(&self) -> Result<Option<[u64; 2]>, String> {
        self.healthy()?;
        Ok(self.run.field()?.map(|field| field.movement_path_steps()))
    }
    pub fn event_window_stats(&self) -> Result<Option<[u64; 2]>, String> {
        self.healthy()?;
        Ok(self.run.field()?.map(|field| field.event_window_stats()))
    }
    fn receipt(
        &self,
        accepted: Option<bool>,
        rejection: Option<&str>,
        ticks: usize,
    ) -> Result<Value, String> {
        let s = self.run.snapshot()?;
        Ok(json!({
            "revision": self.revision,
            "phase": match s.phase {
                Phase::Prep => "prep", Phase::Draft => "draft", Phase::Combat => "combat",
                Phase::Result => "result", Phase::Faulted => "faulted",
            },
            "round": s.round,
            "terminated": s.phase == Phase::Result,
            "decisionRequired": matches!(s.phase, Phase::Prep | Phase::Draft),
            "budgetExhausted": s.phase == Phase::Combat,
            "accepted": accepted, "rejection": rejection, "ticks": ticks,
        }))
    }
    pub fn status(&self) -> Result<Value, String> {
        self.healthy()?;
        self.receipt(None, None, 0)
    }
    /// Submit exactly one preparation/draft intent. g.ready advances only the
    /// resulting battle, stopping at the next decision or the supplied budget.
    /// Malformed input does not consume a revision; rule rejection does.
    pub fn step(&mut self, value: &Value, budget: usize) -> Result<Value, String> {
        self.check_budget(budget)?;
        if !matches!(self.run.phase(), Phase::Prep | Phase::Draft) {
            return Err(
                "Step requires a decision; resume combat with advance or reset an ended session"
                    .into(),
            );
        }
        let ready = value["t"] == "g.ready";
        let action = if ready {
            if value
                .as_object()
                .is_none_or(|o| o.keys().any(|k| k != "t" && k != "ready"))
                || value.get("ready").is_some_and(|v| v != true)
            {
                return Err("g.ready requires ready=true when supplied".into());
            }
            None
        } else {
            Some(Action::parse(value)?)
        };
        if ready && self.run.phase() == Phase::Prep {
            self.preview = self.current_preview()?;
        }
        let result = if let Some(action) = action {
            self.run.prep_action(action)
        } else {
            self.run.ready_current()
        };
        match result {
            Err(error) if self.run.phase() != Phase::Faulted && is_rejection(&error) => {
                self.revision += 1;
                self.receipt(Some(false), Some(&error), 0)
            }
            Err(error) => self.fail(error),
            Ok(()) => {
                self.revision += 1;
                let advance = match self.run.advance(budget) {
                    Ok(a) => a,
                    Err(error) => return self.fail(error),
                };
                self.receipt(Some(true), None, advance.ticks)
            }
        }
    }
    /// Resume a suspended battle without replaying its ready action or settlement.
    pub fn advance(&mut self, budget: usize) -> Result<Value, String> {
        self.check_budget(budget)?;
        if self.run.phase() != Phase::Combat {
            return Err("Advance requires suspended combat".into());
        }
        let advance = match self.run.advance(budget) {
            Ok(a) => a,
            Err(error) => return self.fail(error),
        };
        self.revision += 1;
        self.receipt(None, None, advance.ticks)
    }
}
fn is_rejection(error: &str) -> bool {
    matches!(
        error,
        "BAD_TARGET"
            | "SOLD_OUT"
            | "NO_FUNDS"
            | "HAND_FULL"
            | "BOARD_FULL"
            | "BAD_TILE"
            | "MAX_LEVEL"
            | "TEMP_NOT_EMPTY"
            | "WRONG_PHASE"
    )
}
