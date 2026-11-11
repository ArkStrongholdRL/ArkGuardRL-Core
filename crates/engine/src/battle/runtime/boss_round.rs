// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct BossOvertime {
    pub after_real_seconds: f64,
    pub combat_time_scale: f64,
    pub lp_per_real_second: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct BossRoundConfig {
    pub life_points: f64,
    pub overtime: Option<BossOvertime>,
}
pub(crate) fn validate_boss_round(config: BossRoundConfig) -> Result<(), String> {
    Round::new(config).map(|_| ())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossRoundOutcome {
    Victory,
    LifeDepleted,
    Timeout,
    Forced,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BossRoundState {
    pub initial_lp: f64,
    pub lp: f64,
    pub leak_loss: f64,
    pub skill_loss: f64,
    pub overtime_loss: f64,
    pub outcome: Option<BossRoundOutcome>,
}
pub(super) enum Cost {
    Leak,
    Skill,
    Overtime,
}
pub(super) struct Round {
    state: BossRoundState,
    overtime: Option<BossOvertime>,
}
impl Round {
    fn new(config: BossRoundConfig) -> Result<Self, String> {
        if !config.life_points.is_finite() || config.life_points <= 0. {
            return Err("Boss round requires positive finite life points".into());
        }
        if let Some(o) = config.overtime {
            if !o.after_real_seconds.is_finite()
                || o.after_real_seconds < 0.
                || !o.combat_time_scale.is_finite()
                || o.combat_time_scale <= 0.
                || !o.lp_per_real_second.is_finite()
                || o.lp_per_real_second < 0.
            {
                return Err("Invalid boss overtime settings".into());
            }
        }
        Ok(Self {
            state: BossRoundState {
                initial_lp: config.life_points,
                lp: config.life_points,
                leak_loss: 0.,
                skill_loss: 0.,
                overtime_loss: 0.,
                outcome: None,
            },
            overtime: config.overtime,
        })
    }
    pub(super) fn charge(&mut self, cost: Cost, amount: f64) -> Result<(), String> {
        let total = match cost {
            Cost::Leak => &mut self.state.leak_loss,
            Cost::Skill => &mut self.state.skill_loss,
            Cost::Overtime => &mut self.state.overtime_loss,
        };
        let next = *total + amount;
        if !amount.is_finite() || amount < 0. || !next.is_finite() {
            return Err("Invalid or overflowing boss LP cost".into());
        }
        *total = next;
        self.state.lp = (self.state.lp - amount).max(0.);
        Ok(())
    }
    pub(super) fn advance(&mut self, time: f64) -> Result<(), String> {
        if let Some(o) = self.overtime {
            let elapsed = time / o.combat_time_scale - o.after_real_seconds;
            let due = if elapsed >= 1. {
                elapsed.floor() * o.lp_per_real_second
            } else {
                0.
            };
            if !due.is_finite() {
                return Err("Boss overtime cost overflow".into());
            }
            if due > self.state.overtime_loss {
                self.charge(Cost::Overtime, due - self.state.overtime_loss)?;
                self.state.overtime_loss = due;
            }
        }
        Ok(())
    }
    pub(super) fn depleted(&self) -> bool {
        self.state.lp <= 0.
    }
    pub(super) fn finish(&mut self, reason: &End, hp: f64) {
        self.state.outcome = Some(if hp <= 0. {
            BossRoundOutcome::Victory
        } else if self.depleted() {
            BossRoundOutcome::LifeDepleted
        } else if *reason == End::Timeout {
            BossRoundOutcome::Timeout
        } else {
            BossRoundOutcome::Forced
        });
    }
}
impl Field {
    /// Configure the single-player Boss round before its first tick.
    pub fn configure_boss_round(&mut self, config: BossRoundConfig) -> Result<(), String> {
        self.healthy()?;
        if self.core.boss.is_none()
            || self.finished()
            || self.clock.started()
            || self.ticks() != 0
            || self.core.host.boss_round.is_some()
        {
            return Err("Boss round must be configured once before a boss field starts".into());
        }
        self.core.host.boss_round = Some(Round::new(config)?);
        Ok(())
    }
    // MatchBoss applies overtime after Battle.step, including its final frame.
    // Keep the round clock outside the combat phases so a natural timeout wins
    // over a force-end request issued at that same deadline.
    pub(super) fn run_boss_round(&mut self, budget: usize) -> Result<usize, String> {
        let mut ticks = 0;
        while ticks < budget && !self.finished() {
            self.clock.step(&mut self.core)?;
            ticks += 1;
            let round = self.core.host.boss_round.as_mut().unwrap();
            round.advance(self.clock.time())?;
            if round.depleted() && self.core.world.pool(1).unwrap().hp > 0. {
                self.clock.force_end(&mut self.core, "forced".into())?;
            }
            self.finish_boss_round();
        }
        Ok(ticks)
    }
    pub(super) fn finish_boss_round(&mut self) {
        if let (Some(round), Some(reason)) = (&mut self.core.host.boss_round, self.clock.reason()) {
            round.finish(reason, self.core.world.pool(1).unwrap().hp);
        }
    }
    pub fn boss_round_state(&self) -> Result<Option<BossRoundState>, String> {
        self.healthy()?;
        Ok(self.core.host.boss_round.as_ref().map(|r| r.state))
    }
}
impl Host {
    pub(super) fn lose_lp(&mut self, amount: f64) -> Result<(), String> {
        self.content.as_mut().unwrap().lose_lp(amount)?;
        if let Some(round) = &mut self.boss_round {
            round.charge(Cost::Skill, amount)?;
        }
        Ok(())
    }
}
