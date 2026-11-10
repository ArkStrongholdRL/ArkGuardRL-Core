// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol ai.js attackCountdown / noteAttackCommit timing gate.
#[derive(Clone, Debug)]
pub struct State {
    pub cooldown: f64,
    pub had_target: Option<bool>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            cooldown: 0.0,
            had_target: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Profile {
    pub no_attack: bool,
    pub only_while_skill: bool,
    pub heal_attack: bool,
    pub all_in_range: bool,
    pub range_aoe: bool,
}
pub fn countdown(cd: f64, dt: f64) -> f64 {
    let left = cd - dt;
    if left > 1e-9 {
        left
    } else {
        0.0
    }
}
impl State {
    pub fn tick(&mut self, dt: f64, can_act: bool) {
        if self.cooldown > 0.0 && can_act {
            self.cooldown = countdown(self.cooldown, dt);
        }
    }
    pub fn may_commit(
        &self,
        dt: f64,
        can_act: bool,
        disarmed: bool,
        skill_active: bool,
        p: Profile,
    ) -> bool {
        can_act
            && !disarmed
            && !(self.cooldown > dt + 1e-9)
            && !(self.cooldown <= 0.0 && self.had_target == Some(false))
            && !p.no_attack
            && (!p.only_while_skill || skill_active)
            && !p.heal_attack
            && !p.all_in_range
            && !p.range_aoe
    }
    pub fn complete_attack(&mut self, interval: f64) {
        self.cooldown = if self.cooldown.is_nan() || interval.is_nan() {
            f64::NAN
        } else {
            self.cooldown.max(interval)
        };
    }
}

/// Decay the actual unit's timer using the same canAct gate as Unit.canAct.
pub fn tick_unit(
    world: &mut super::health::World,
    id: super::health::Id,
    dt: f64,
) -> Result<(), String> {
    let u = world.unit_mut(id)?;
    let can_act = u.stats.alive
        && u.deployed
        && !u.hidden
        && !u
            .stats
            .stats()
            .map_err(str::to_string)?
            .flags
            .contains("stun");
    u.cadence.tick(dt, can_act);
    Ok(())
}
