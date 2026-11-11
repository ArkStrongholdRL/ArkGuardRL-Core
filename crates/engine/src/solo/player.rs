// SPDX-License-Identifier: GPL-3.0-or-later
use super::{add, Stats, MAX_FUNDS};

#[derive(Clone, Default)]
pub(super) struct Player {
    pub lp: f64,
    pub team_lp: Option<f64>,
    pub funds: u64,
    pub pending: u64,
    pub round_spent: u64,
    pub stats: Stats,
}
impl Player {
    pub fn spend(&mut self, amount: u64) -> Result<(), String> {
        if amount > self.funds {
            return Err("Insufficient preparation funds".into());
        }
        let spent = add(self.stats.funds_spent, amount)?;
        let round_spent = add(self.round_spent, amount)?;
        self.funds -= amount;
        self.stats.funds_spent = spent;
        self.round_spent = round_spent;
        Ok(())
    }
    pub fn earn(&mut self, amount: u64) -> Result<(), String> {
        let funds = add(self.funds, amount)?;
        let gained = add(self.stats.funds_gained, amount)?;
        self.funds = funds;
        self.stats.funds_gained = gained;
        Ok(())
    }
    pub fn start_round(&mut self, income: u64) -> Result<(), String> {
        self.round_spent = 0;
        self.earn(add(income, self.pending)?)?;
        self.pending = 0;
        Ok(())
    }
    pub fn bounty(&mut self, amount: f64) -> Result<(), String> {
        let amount = amount.trunc().max(0.);
        if !amount.is_finite() || amount > MAX_FUNDS as f64 {
            return Err("Invalid battle bounty total".into());
        }
        let pending = add(self.pending, amount as u64)?;
        let gained = add(self.stats.funds_gained, amount as u64)?;
        self.pending = pending;
        self.stats.funds_gained = gained;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.funds = 0;
        self.pending = 0;
    }
}
