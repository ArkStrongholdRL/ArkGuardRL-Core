// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/lifecycle.js step/_phase/_checkEnd.
// Hosts execute the concrete unit/content phases; this module owns tick ordering.
pub const DT: f64 = 1.0 / 30.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Start,
    Scheduled,
    Spawns,
    Dp,
    Buffs,
    AttackCommit,
    Enemies,
    EnemyIndex,
    Allies,
    Rate,
    Projectiles,
    Redeploy,
    Boss,
    TickHook,
    EndCheck,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    Cleared,
    Timeout,
    Forced,
}
#[derive(Clone, Copy)]
pub struct EndState {
    pub shared_hp: Option<f64>,
    pub boss_like: bool,
    pub auto_finish: bool,
    pub pending_spawns: bool,
    pub enemy_alive: bool,
    pub pending_holds: bool,
    pub time_limit: f64,
    pub max_battle_time: f64,
}
pub trait Host: Sized {
    fn phase(&mut self, phase: Phase, clock: &mut Clock) -> Result<(), String>;
    fn compact_enemies(&mut self, clock: &mut Clock) -> Result<(), String>;
    fn has_removed(&self) -> bool;
    fn release_removed(&mut self, clock: &mut Clock) -> Result<(), String>;
    fn has_tick_hook(&self) -> bool;
    fn end_state(&self) -> Option<EndState> {
        None
    }
    fn finish(&mut self, reason: &End, clock: &Clock) -> Result<(), String>;
}
#[derive(Default)]
pub struct Clock {
    started: bool,
    finished: bool,
    stepping: bool,
    faulted: bool,
    ticks: u64,
    time: f64,
    request: Option<String>,
    reason: Option<End>,
}
impl Clock {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn time(&self) -> f64 {
        self.time
    }
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
    pub fn started(&self) -> bool {
        self.started
    }
    pub fn finished(&self) -> bool {
        self.finished
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    pub fn reason(&self) -> Option<&End> {
        self.reason.as_ref()
    }
    pub fn mark_started(&mut self) {
        self.started = true;
    }
    pub fn force_end<H: Host>(&mut self, host: &mut H, reason: String) -> Result<(), String> {
        if self.faulted {
            return Err("Battle tick is faulted".into());
        }
        if self.finished {
            return Ok(());
        }
        if self.stepping {
            if self.request.is_none() {
                self.request = Some(reason);
            }
            return Ok(());
        }
        if !self.started {
            if let Err(error) = host.phase(Phase::Start, self) {
                self.faulted = true;
                return Err(error);
            }
        }
        if self.finished {
            return Ok(());
        }
        self.finish(
            host,
            if reason == "timeout" {
                End::Timeout
            } else {
                End::Forced
            },
        )
    }
    fn finish<H: Host>(&mut self, host: &mut H, reason: End) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.request = None;
        self.reason = Some(reason.clone());
        let result = host.finish(&reason, self);
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    fn phase<H: Host>(&mut self, host: &mut H, p: Phase) -> Result<(), String> {
        if !self.finished && self.request.is_none() {
            host.phase(p, self)?;
        }
        Ok(())
    }
    pub fn step<H: Host>(&mut self, host: &mut H) -> Result<(), String> {
        if self.faulted {
            return Err("Battle tick is faulted".into());
        }
        if self.finished || self.stepping {
            return Ok(());
        }
        self.stepping = true;
        let result = self.step_inner(host);
        self.stepping = false;
        if let Err(error) = result {
            self.faulted = true;
            return Err(error);
        }
        if !self.finished {
            if let Some(reason) = self.request.take() {
                self.force_end(host, reason)?;
            }
        }
        Ok(())
    }
    fn step_inner<H: Host>(&mut self, host: &mut H) -> Result<(), String> {
        if !self.started {
            self.phase(host, Phase::Start)?;
        }
        for p in [
            Phase::Scheduled,
            Phase::Spawns,
            Phase::Dp,
            Phase::Buffs,
            Phase::AttackCommit,
            Phase::Enemies,
        ] {
            self.phase(host, p)?;
        }
        host.compact_enemies(self)?;
        for p in [
            Phase::EnemyIndex,
            Phase::Allies,
            Phase::Rate,
            Phase::Projectiles,
            Phase::Redeploy,
            Phase::Boss,
        ] {
            self.phase(host, p)?;
        }
        if host.has_tick_hook() {
            self.phase(host, Phase::TickHook)?;
        }
        host.compact_enemies(self)?;
        if host.has_removed() {
            host.release_removed(self)?;
        }
        if self.request.is_none() && !self.finished {
            self.ticks = self
                .ticks
                .checked_add(1)
                .ok_or("Battle tick count exhausted")?;
            self.time = self.ticks as f64 * DT;
            self.phase(host, Phase::EndCheck)?;
            if !self.finished && self.request.is_none() {
                self.check_end(host)?;
            }
        }
        Ok(())
    }
    fn check_end<H: Host>(&mut self, host: &mut H) -> Result<(), String> {
        let Some(s) = host.end_state() else {
            return Ok(());
        };
        if s.shared_hp.map(|hp| hp <= 0.0).unwrap_or(false) {
            return self.finish(host, End::Cleared);
        }
        let pool_holds = s.shared_hp.is_some() && s.boss_like;
        if s.auto_finish
            && !pool_holds
            && !s.pending_spawns
            && self.started
            && !s.enemy_alive
            && !s.pending_holds
        {
            return self.finish(host, End::Cleared);
        }
        if self.time >= s.time_limit - 1e-9 || self.time >= s.max_battle_time {
            return self.finish(host, End::Timeout);
        }
        Ok(())
    }
    /// A decision owner may stop after a bounded number of steps without inventing
    /// a terminal outcome. State and clock remain available for truncation output.
    pub fn run_steps<H: Host>(&mut self, host: &mut H, budget: usize) -> Result<usize, String> {
        if self.faulted {
            return Err("Battle tick is faulted".into());
        }
        let mut n = 0;
        while n < budget && !self.finished {
            self.step(host)?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Driver {
        fail: bool,
        request_on_scheduled: bool,
        calls: usize,
        finishes: usize,
    }
    impl Host for Driver {
        fn phase(&mut self, p: Phase, c: &mut Clock) -> Result<(), String> {
            self.calls += 1;
            if p == Phase::Start {
                c.mark_started();
            }
            if self.fail && p == Phase::Scheduled {
                return Err("scheduled callback failed".into());
            }
            if self.request_on_scheduled && p == Phase::Scheduled {
                c.force_end(self, "timeout".into())?;
                c.force_end(self, "later request".into())?;
            }
            Ok(())
        }
        fn compact_enemies(&mut self, _: &mut Clock) -> Result<(), String> {
            Ok(())
        }
        fn has_removed(&self) -> bool {
            false
        }
        fn release_removed(&mut self, _: &mut Clock) -> Result<(), String> {
            unreachable!()
        }
        fn has_tick_hook(&self) -> bool {
            false
        }
        fn finish(&mut self, _: &End, _: &Clock) -> Result<(), String> {
            self.finishes += 1;
            Ok(())
        }
    }
    #[test]
    fn time_is_tick_multiple_and_budget_does_not_terminate() {
        let mut c = Clock::new();
        let mut h = Driver::default();
        assert_eq!(c.run_steps(&mut h, 9000).unwrap(), 9000);
        assert_eq!(c.time().to_bits(), (9000.0 * DT).to_bits());
        assert!(!c.finished());
        assert_eq!(h.finishes, 0);
        assert_eq!(c.run_steps(&mut h, 0).unwrap(), 0);
        assert_eq!(c.ticks(), 9000);
        c.force_end(&mut h, "external".into()).unwrap();
        assert_eq!(c.run_steps(&mut h, 10).unwrap(), 0);
        assert_eq!(h.finishes, 1);
    }
    #[test]
    fn first_end_request_wins_and_external_end_starts_battle() {
        let mut c = Clock::new();
        let mut h = Driver {
            request_on_scheduled: true,
            ..Driver::default()
        };
        c.step(&mut h).unwrap();
        assert_eq!(c.reason(), Some(&End::Timeout));
        assert_eq!(c.ticks(), 0);
        assert_eq!(h.finishes, 1);
        let mut fresh = Clock::new();
        let mut host = Driver::default();
        fresh.force_end(&mut host, "anything".into()).unwrap();
        assert!(fresh.started());
        assert_eq!(fresh.reason(), Some(&End::Forced));
        assert_eq!(host.calls, 1);
    }
    #[test]
    fn phase_failure_never_becomes_a_valid_continuation() {
        let mut c = Clock::new();
        let mut h = Driver {
            fail: true,
            ..Driver::default()
        };
        assert!(c.step(&mut h).is_err());
        assert!(c.faulted());
        assert!(!c.finished());
        assert_eq!(c.ticks(), 0);
        let calls = h.calls;
        h.fail = false;
        assert!(c.step(&mut h).is_err());
        assert_eq!(h.calls, calls);
        assert_eq!(h.finishes, 0);
    }
}
