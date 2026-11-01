//! NativeProgressCore formula, separated from policy code and the future match state machine.
const CORE_BONDS: &[&str] = &[
    "yanShip",
    "sargonShip",
    "victoriaShip",
    "kjeragShip",
    "lateranoShip",
    "egirShip",
    "siracusaShip",
    "kazimierzShip",
];

#[derive(Clone, Copy, Debug)]
pub struct Bond<'a> {
    pub id: &'a str,
    pub active: bool,
    pub off: bool,
    pub layers: f64,
}

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    Live,
    Loss,
    Victory,
}

/// The match supplies actual completed rounds, including final settlement if ended.
/// Budget truncation uses Live, retaining the successor state's potential.
pub struct Progress<'a> {
    pub rounds_passed: u32,
    pub boss_round: u32,
    pub outcome: Outcome,
    pub bonds: &'a [Bond<'a>],
}

fn values(state: &Progress<'_>) -> Result<(f64, f64), &'static str> {
    if state.boss_round == 0 {
        return Err("Positive boss round required");
    }
    let progress = (state.rounds_passed as f64 / state.boss_round as f64).min(1.0);
    if !matches!(state.outcome, Outcome::Live) {
        return Ok((progress, 0.0));
    }
    let mut layers: f64 = 0.0;
    for bond in state.bonds {
        if CORE_BONDS.contains(&bond.id) && bond.active && !bond.off {
            if !bond.layers.is_finite() || bond.layers < 0.0 {
                return Err("Active CORE layers must be finite and nonnegative");
            }
            layers = layers.max(bond.layers);
        }
    }
    Ok((progress, 0.1 * (layers / 100.0).tanh()))
}

#[derive(Debug)]
pub struct Components {
    pub outcome: f64,
    pub progress: f64,
    pub core_shaping: f64,
}

impl Components {
    pub fn total(&self) -> f64 {
        // Error-free partial sums and the same final tie correction as math.fsum.
        let mut partials = [0.0; 3];
        let mut n = 0;
        for mut x in [self.outcome, self.progress, self.core_shaping] {
            let mut i = 0;
            for j in 0..n {
                let mut y: f64 = partials[j];
                if x.abs() < y.abs() {
                    std::mem::swap(&mut x, &mut y);
                }
                let hi = x + y;
                let lo = y - (hi - x);
                if lo != 0.0 {
                    partials[i] = lo;
                    i += 1;
                }
                x = hi;
            }
            partials[i] = x;
            n = i + 1;
        }
        n -= 1;
        let mut hi = partials[n];
        let mut lo = 0.0;
        while n > 0 {
            n -= 1;
            let x = hi;
            let y = partials[n];
            hi = x + y;
            lo = y - (hi - x);
            if lo != 0.0 {
                break;
            }
        }
        if n > 0 && ((lo < 0.0 && partials[n - 1] < 0.0) || (lo > 0.0 && partials[n - 1] > 0.0)) {
            let y = lo * 2.0;
            let x = hi + y;
            if y == x - hi {
                hi = x;
            }
        }
        hi
    }
}

pub struct ProgressReward {
    boss_round: u32,
    progress: f64,
    potential: f64,
    ended: bool,
}

impl ProgressReward {
    pub fn new(initial: &Progress<'_>) -> Result<Self, &'static str> {
        if !matches!(initial.outcome, Outcome::Live) {
            return Err("Reset requires a live episode");
        }
        let (progress, potential) = values(initial)?;
        Ok(Self {
            boss_round: initial.boss_round,
            progress,
            potential,
            ended: false,
        })
    }

    pub fn step(&mut self, after: &Progress<'_>) -> Result<Components, &'static str> {
        if self.ended {
            return Err("Reward episode already ended");
        }
        if after.boss_round != self.boss_round {
            return Err("Reward boss round changed");
        }
        let (progress, potential) = values(after)?;
        if progress < self.progress {
            return Err("Rounds-passed progress regressed");
        }
        let outcome = match after.outcome {
            Outcome::Live => 0.0,
            Outcome::Loss => -1.0,
            Outcome::Victory => 1.0,
        };
        let components = Components {
            outcome,
            progress: 0.5 * (progress - self.progress),
            core_shaping: potential - self.potential,
        };
        self.progress = progress;
        self.potential = potential;
        self.ended = !matches!(after.outcome, Outcome::Live);
        Ok(components)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_and_terminal_potential_telescope() {
        let bonds = [Bond {
            id: "yanShip",
            active: true,
            off: false,
            layers: 50.0,
        }];
        let initial = Progress {
            rounds_passed: 0,
            boss_round: 13,
            outcome: Outcome::Live,
            bonds: &[],
        };
        let mut reward = ProgressReward::new(&initial).unwrap();
        let a = reward
            .step(&Progress {
                rounds_passed: 6,
                bonds: &bonds,
                ..initial
            })
            .unwrap();
        let b = reward
            .step(&Progress {
                rounds_passed: 13,
                outcome: Outcome::Victory,
                bonds: &bonds,
                ..initial
            })
            .unwrap();
        assert_eq!(a.core_shaping, -b.core_shaping);
        assert_eq!(a.total() + b.total(), 1.5);
        assert!(reward.step(&initial).is_err());
    }

    #[test]
    fn inactive_layers_and_live_truncation_do_not_create_rewards() {
        let bonds = [Bond {
            id: "yanShip",
            active: true,
            off: true,
            layers: f64::NAN,
        }];
        let live = Progress {
            rounds_passed: 0,
            boss_round: 13,
            outcome: Outcome::Live,
            bonds: &bonds,
        };
        let mut reward = ProgressReward::new(&live).unwrap();
        assert_eq!(reward.step(&live).unwrap().total(), 0.0);
        let bad = [Bond {
            off: false,
            ..bonds[0]
        }];
        assert!(reward
            .step(&Progress {
                bonds: &bad,
                ..live
            })
            .is_err());
        assert_eq!(reward.step(&live).unwrap().total(), 0.0);
        assert_eq!(
            reward
                .step(&Progress {
                    outcome: Outcome::Loss,
                    ..live
                })
                .unwrap()
                .total(),
            -1.0
        );
    }
}
