// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 SkillRuntime SP accounting. Cast/trigger/content
// state transitions are implemented separately; this owns the charge arithmetic.
use super::{
    health::{Id, World},
    stats::Stat,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Instant,
    Charges,
    Duration,
    Ammo,
    Toggle,
    Passive,
}
impl Kind {
    pub fn timed(self) -> bool {
        matches!(self, Self::Duration | Self::Ammo | Self::Toggle)
    }
}
pub trait Hooks {
    fn has_gain_hook(&self) -> bool;
    fn gain(
        &mut self,
        amount: &mut f64,
        reason: &str,
        bank: &mut Bank,
        world: &mut World,
    ) -> Result<(), String>;
}
pub struct NoHooks;
impl Hooks for NoHooks {
    fn has_gain_hook(&self) -> bool {
        false
    }
    fn gain(&mut self, _: &mut f64, _: &str, _: &mut Bank, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Clone)]
pub struct Bank {
    pub unit: Id,
    pub kind: Kind,
    pub no_skill: bool,
    pub active: bool,
    pub sp: f64,
    pub charges: f64,
    base_cost: f64,
    cost_mul: f64,
    max_charges: u32,
}
impl Bank {
    pub fn new(
        unit: Id,
        kind: Kind,
        base_cost: f64,
        max_charges: u32,
        charge_limit: u32,
    ) -> Result<Self, String> {
        if !base_cost.is_finite()
            || base_cost < 0.0
            || max_charges == 0
            || max_charges > charge_limit
        {
            return Err("Invalid skill SP configuration or charge capacity".into());
        }
        Ok(Self {
            unit,
            kind,
            no_skill: false,
            active: false,
            sp: 0.0,
            charges: 0.0,
            base_cost: if kind == Kind::Passive {
                0.0
            } else {
                base_cost
            },
            cost_mul: 1.0,
            max_charges,
        })
    }
    pub(crate) fn clear_inactive(&mut self, w: &mut World) -> Result<(), String> {
        if !self.no_skill && self.kind != Kind::Passive && !self.active && self.cost(w)? > 0. {
            self.sp = 0.;
            self.charges = 0.;
        }
        Ok(())
    }
    pub fn max_charges(&self) -> u32 {
        self.max_charges
    }
    pub fn cost_multiplier(&self) -> f64 {
        self.cost_mul
    }
    pub fn cost(&self, w: &mut World) -> Result<f64, String> {
        let flat = w
            .unit_mut(self.unit)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .get(Stat::SpCostFlat);
        let c = (self.base_cost * self.cost_mul + flat).floor();
        Ok(if c.is_finite() {
            c.max(0.0)
        } else {
            self.base_cost
        })
    }
    pub fn ready(&self) -> bool {
        !self.no_skill && self.kind != Kind::Passive && self.charges >= 1.0
    }
    pub fn total(&self, w: &mut World) -> Result<f64, String> {
        if self.no_skill || self.kind == Kind::Passive {
            return Ok(0.0);
        }
        let cost = self.cost(w)?;
        Ok(if self.charges >= self.max_charges as f64 {
            self.max_charges as f64 * cost
        } else {
            self.charges * cost + self.sp
        })
    }
    pub fn set_multiplier(&mut self, w: &mut World, v: f64) -> Result<(), String> {
        self.cost_mul = if v.is_finite() && v >= 0.0 { v } else { 1.0 };
        self.normalize(w)
    }
    pub fn normalize(&mut self, w: &mut World) -> Result<(), String> {
        if self.no_skill || self.kind == Kind::Passive {
            return Ok(());
        }
        let cost = self.cost(w)?;
        if cost <= 0.0 {
            if self.sp > 0.0 {
                self.sp = 0.0;
                self.charges = self.max_charges as f64;
            }
            return Ok(());
        }
        self.fill(cost)?;
        if self.sp > cost {
            self.sp = cost;
        }
        Ok(())
    }
    fn fill(&mut self, cost: f64) -> Result<(), String> {
        if !self.charges.is_finite()
            || self.charges < 0.0
            || self.charges > self.max_charges as f64 + 1.0
        {
            return Err("Invalid skill charge state".into());
        }
        while self.sp > cost - 1e-9 && self.charges < self.max_charges as f64 {
            self.charges += 1.0;
            self.sp = if self.charges < self.max_charges as f64 {
                (self.sp - cost).max(0.0)
            } else {
                cost
            };
        }
        Ok(())
    }
    pub fn gain<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        amount: f64,
        reason: &str,
        silent: bool,
        ignore_lock: bool,
    ) -> Result<f64, String> {
        if self.no_skill || self.kind == Kind::Passive || !(amount > 0.0) {
            return Ok(0.0);
        }
        if self.active && self.kind.timed() && reason != "init" && !ignore_lock {
            return Ok(0.0);
        }
        if reason != "init"
            && !ignore_lock
            && w.unit_mut(self.unit)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("noSp")
        {
            return Ok(0.0);
        }
        let mut cost = self.cost(w)?;
        if self.charges >= self.max_charges as f64 && self.sp >= cost {
            return Ok(0.0);
        }
        let mut amt = amount;
        if !silent && h.has_gain_hook() {
            h.gain(&mut amt, reason, self, w)?;
            if !amt.is_finite() {
                amt = 0.0;
            }
            if !(amt > 0.0) {
                return Ok(0.0);
            }
            if self.kind == Kind::Passive || self.no_skill {
                return Ok(0.0);
            }
            cost = self.cost(w)?;
            if self.charges >= self.max_charges as f64 && self.sp >= cost {
                return Ok(0.0);
            }
        }
        if cost <= 0.0 {
            return Ok(0.0);
        }
        self.sp += amt;
        self.fill(cost)?;
        if self.charges >= self.max_charges as f64 {
            self.sp = cost;
        }
        Ok(amt)
    }
    pub(crate) fn apply_gain(&mut self, cost: f64, amount: f64) -> Result<(), String> {
        self.sp += amount;
        self.fill(cost)?;
        if self.charges >= self.max_charges as f64 {
            self.sp = cost;
        }
        Ok(())
    }
    pub fn set_total(&mut self, w: &mut World, total: f64) -> Result<(), String> {
        if self.no_skill
            || self.kind == Kind::Passive
            || self.active && self.kind.timed()
            || !total.is_finite()
        {
            return Ok(());
        }
        self.sp = 0.0;
        self.charges = 0.0;
        self.gain(w, &mut NoHooks, total.max(0.0), "init", true, false)?;
        if self.cost(w)? <= 0.0 {
            self.charges = self.max_charges as f64;
        }
        Ok(())
    }
    pub fn add_charge(&mut self, w: &mut World, n: f64) -> Result<(), String> {
        if !n.is_finite() {
            return Ok(());
        }
        self.charges = (self.charges + n).clamp(0.0, self.max_charges as f64);
        if self.charges >= self.max_charges as f64 {
            self.sp = self.cost(w)?;
        }
        Ok(())
    }
    /// Natural recovery is independent of stun/silence; noSp and active timed
    /// skills pause it, and a missing/dead/hidden deployment cannot recover.
    pub fn recover_time<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        dt: f64,
    ) -> Result<f64, String> {
        if self.no_skill || self.active && self.kind.timed() {
            return Ok(0.0);
        }
        let u = w.unit_mut(self.unit)?;
        let alive = u.stats.alive;
        let s = u.stats.stats().map_err(str::to_string)?;
        if s.flags.contains("noSp") || !alive || !u.deployed || u.hidden {
            return Ok(0.0);
        }
        let rate = s.get(Stat::SpRecovery);
        if rate > 0.0 {
            self.gain(w, h, rate * dt, "time", false, false)
        } else {
            Ok(0.0)
        }
    }
}
