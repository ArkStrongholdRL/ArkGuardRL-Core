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
    /// Advance a proven event-free movement window, or return zero to use full ticks.
    fn movement_batch(&mut self, _clock: &mut Clock, _budget: usize) -> Result<usize, String> {
        Ok(0)
    }
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
    movement_ticks: u64,
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
    pub fn movement_ticks(&self) -> u64 {
        self.movement_ticks
    }
    pub(crate) fn movement_tick(&mut self) -> Result<(), String> {
        self.ticks = self
            .ticks
            .checked_add(1)
            .ok_or("Battle tick count exhausted")?;
        self.movement_ticks = self
            .movement_ticks
            .checked_add(1)
            .ok_or("Movement tick count exhausted")?;
        self.time = self.ticks as f64 * DT;
        Ok(())
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
            if self.started && budget - n >= 2 && self.request.is_none() && !self.stepping {
                let before = self.ticks;
                self.stepping = true;
                let batch = host.movement_batch(self, budget - n);
                self.stepping = false;
                let count = match batch {
                    Ok(count) if count <= budget - n && self.ticks - before == count as u64 => {
                        count
                    }
                    Ok(_) => {
                        self.faulted = true;
                        return Err("Invalid movement batch accounting".into());
                    }
                    Err(error) => {
                        self.faulted = true;
                        return Err(error);
                    }
                };
                if count > 0 {
                    n += count;
                    continue;
                }
            }
            self.step(host)?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MovingHost {
        invalid: bool,
    }
    impl Host for MovingHost {
        fn movement_batch(&mut self, clock: &mut Clock, budget: usize) -> Result<usize, String> {
            if self.invalid {
                return Ok(budget);
            }
            let count = budget.min(4);
            for _ in 0..count {
                clock.movement_tick()?;
            }
            Ok(count)
        }
        fn phase(&mut self, phase: Phase, clock: &mut Clock) -> Result<(), String> {
            if phase == Phase::Start {
                clock.mark_started();
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
            Ok(())
        }
        fn has_tick_hook(&self) -> bool {
            false
        }
        fn finish(&mut self, _: &End, _: &Clock) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn movement_batches_respect_tick_budgets() {
        let mut host = MovingHost { invalid: false };
        let mut clock = Clock::new();
        let mut total = 0;
        for budget in [0, 1, 2, 3, 4, 17, 64, 65] {
            assert_eq!(clock.run_steps(&mut host, budget).unwrap(), budget);
            total += budget as u64;
            assert_eq!(clock.ticks(), total);
            assert_eq!(clock.time().to_bits(), (total as f64 * DT).to_bits());
        }
        assert!(clock.movement_ticks() > 100);
        assert!(clock.movement_ticks() < total);
    }

    #[test]
    fn invalid_movement_accounting_faults_the_clock() {
        let mut host = MovingHost { invalid: true };
        let mut clock = Clock::new();
        clock.run_steps(&mut host, 1).unwrap();
        assert_eq!(
            clock.run_steps(&mut host, 2).unwrap_err(),
            "Invalid movement batch accounting"
        );
        assert!(clock.faulted());
        assert!(clock.run_steps(&mut host, 1).is_err());
    }
}
