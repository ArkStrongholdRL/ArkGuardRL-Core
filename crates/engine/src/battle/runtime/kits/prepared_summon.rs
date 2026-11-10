// SPDX-License-Identifier: GPL-3.0-or-later
//! Effects and reusable board pieces for placeable medical drones and shields.
use super::dispatch::{Behavior, Context, Event};
use super::*;
use crate::battle::grid;

pub(crate) enum PreparedSummon {
    Drone {
        lifetime: f64,
    },
    Shield {
        idle: f64,
        interval: f64,
        cap: f64,
        refill: f64,
    },
}
#[derive(Default)]
pub(crate) struct State {
    stocked: bool,
    ready_at: f64,
    next_pulse: f64,
    target: Option<Id>,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            stocked: previous.is_some_and(|s| s.stocked),
            ..Default::default()
        }
    }
}
pub(crate) fn build(raw: &Value, spec: &mut Spec) {
    let drone = raw["tokenId"] == "token_10000_silent_healrb";
    let mut protection = Buff {
        key: Some("token:untargetable".into()),
        permanent: true,
        ..Default::default()
    };
    protection.runtime.persist = true;
    protection.flags.insert("untargetable".into());
    if drone {
        // The countdown bar is its lifetime, with damage and healing disabled.
        for flag in ["invulnerable", "noHeal", "healFree"] {
            protection.flags.insert(flag.into());
        }
        spec.combat.heal = true;
        spec.combat.projectile = None;
        spec.combat.profile.heal = Some(search::Heal {
            multi: false,
            count: 1.,
            hp_at_most: 1.,
            element_heal: false,
        });
        spec.combat.profile.heal_attack = true;
        spec.extra.tactics = Tactics::PreparedSummon(PreparedSummon::Drone {
            lifetime: n(&raw["nativeSkill"], "duration", 10.),
        });
    } else {
        protection.flags.insert("invulnerable".into());
        spec.combat.profile.no_attack = true;
        let talent = talent_at(raw, 0);
        spec.extra.tactics = Tactics::PreparedSummon(PreparedSummon::Shield {
            idle: n(talent, "interval", 5.),
            interval: n(talent, "catsld_t_1[timer][interval].interval", 1.).max(0.01),
            cap: n(talent, "max_shield_ratio", 0.),
            refill: n(talent, "shield_ratio_each_trigger", 0.),
        });
    }
    spec.extra.effects.permanent.push(protection);
}

fn try_drone(k: &mut Context<'_>, token: Id) -> Result<bool, String> {
    let unit = k.world.unit(token)?;
    let state = unit.operator.tactical.prepared_summon()?;
    if !state.stocked
        || unit.stats.alive
        || unit.removed
        || k.world.time + 1e-9 < state.ready_at
        || !unit
            .summoner
            .is_some_and(|owner| Host::tac_up(k.world, owner))
    {
        return Ok(false);
    }
    if !k.redeploy_free(token)? {
        return Ok(false);
    }
    k.world
        .unit_mut(token)?
        .operator
        .tactical
        .prepared_summon_mut()?
        .stocked = false;
    Ok(true)
}

pub(crate) fn release_drone(k: &mut Context<'_>) -> Result<(), String> {
    let token = k
        .host
        .roster
        .as_ref()
        .unwrap()
        .ids()
        .iter()
        .copied()
        .find(|&id| {
            k.world
                .unit(id)
                .is_ok_and(|u| u.summoner == Some(k.unit) && u.deployment.uid.is_some())
                && k.host
                    .tac_def(id)
                    .is_ok_and(|d| d.key == "token_10000_silent_healrb")
        });
    if let Some(token) = token {
        k.world
            .unit_mut(token)?
            .operator
            .tactical
            .prepared_summon_mut()?
            .stocked = true;
        try_drone(k, token)?;
    }
    Ok(())
}

impl PreparedSummon {
    fn shield(&self, k: &mut Context<'_>, connect: bool) -> Result<(), String> {
        let Self::Shield {
            idle,
            interval,
            cap,
            refill,
        } = *self
        else {
            return Ok(());
        };
        let id = k.unit;
        let previous = k
            .world
            .unit(id)?
            .operator
            .tactical
            .prepared_summon()?
            .target;
        let valid = |target: Id| {
            k.world.unit(target).is_ok_and(|u| {
                u.stats.alive
                    && u.deployed
                    && u.deployment.kind == crate::battle::deploy::Kind::Operator
            })
        };
        let target = if !connect && previous.is_some_and(valid) {
            previous
        } else {
            let keys = k.keys()?;
            let mut target = None;
            for key in keys {
                let tile = [(key / grid::COLS) as i32, (key % grid::COLS) as i32];
                let candidate =
                    k.host
                        .roster
                        .as_ref()
                        .unwrap()
                        .ids()
                        .iter()
                        .copied()
                        .find(|&other| {
                            valid(other)
                                && k.world.unit(other).is_ok_and(|u| u.deployment.tile == tile)
                        });
                if let Some(candidate) = candidate {
                    let occupied =
                        k.host
                            .roster
                            .as_ref()
                            .unwrap()
                            .ids()
                            .iter()
                            .copied()
                            .any(|other| {
                                other != id
                                    && Host::tac_up(k.world, other)
                                    && k.world.unit(other).is_ok_and(|u| {
                                        u.operator
                                            .tactical
                                            .prepared_summon()
                                            .is_ok_and(|s| s.target == Some(candidate))
                                    })
                            });
                    if !occupied {
                        target = Some(candidate);
                        break;
                    }
                }
            }
            target
        };
        k.world
            .unit_mut(id)?
            .operator
            .tactical
            .prepared_summon_mut()?
            .target = target;
        let Some(target) = target else {
            return Ok(());
        };
        let owner = k.world.unit(id)?.summoner.unwrap_or(id);
        let hp = k.stat(owner, "maxHp")?;
        let active = Host::tac_up(k.world, owner) && k.world.unit(owner)?.operator.active_skill;
        let rate = if active {
            let raw = &k.host.data.table("chess").unwrap()[&k.host.tac_def(owner)?.key];
            n(&raw["skill"]["bb"], "overwrite_ratio", refill)
        } else {
            refill
        };
        let amount = if connect || previous != Some(target) {
            hp * cap
        } else if active || k.world.time - k.world.unit(target)?.last_hit_at >= idle - 1e-9 {
            hp * rate * interval
        } else {
            0.
        };
        if amount <= 0. || cap <= 0. {
            return Ok(());
        }
        if let Some(buff) = k
            .world
            .unit_mut(target)?
            .stats
            .buffs_mut()
            .iter_mut()
            .find(|buff| buff.key.as_deref() == Some("cathy:shield"))
        {
            buff.shield = (buff.shield + amount).min(hp * cap);
            k.world.unit_mut(target)?.stats.mark_dirty();
        } else {
            let mut buff = Buff {
                key: Some("cathy:shield".into()),
                shield: amount.min(hp * cap),
                ..Default::default()
            };
            buff.runtime.duration = f64::INFINITY;
            k.add(target, buff)?;
        }
        Ok(())
    }
}
impl Behavior for PreparedSummon {
    fn handle(&self, k: &mut Context<'_>, event: &mut Event<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            Event::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => match *self {
                Self::Drone { lifetime } => {
                    k.later(lifetime, |k| k.retreat())?;
                }
                Self::Shield { interval, .. } => {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .prepared_summon_mut()?
                        .next_pulse = k.world.time + interval;
                    self.shield(k, true)?;
                }
            },
            Event::Death { unit, .. } if *unit == id && matches!(self, Self::Drone { .. }) => {
                k.world.unit_mut(id)?.removed = false;
                let ready_at =
                    k.world.time + k.world.unit(id)?.stats.base_number("respawnTime").max(0.);
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .prepared_summon_mut()?
                    .ready_at = ready_at;
                try_drone(k, id)?;
            }
            Event::Tick if matches!(self, Self::Drone { .. }) => {
                try_drone(k, id)?;
                if k.world
                    .unit(id)?
                    .operator
                    .tactical
                    .prepared_summon()?
                    .stocked
                {
                    if let Some(owner) = k
                        .world
                        .unit(id)?
                        .summoner
                        .filter(|&owner| Host::tac_up(k.world, owner))
                    {
                        let mut buff = Buff {
                            key: Some("silent:stockFull".into()),
                            ..Default::default()
                        };
                        buff.runtime.duration = 2. * crate::battle::clock::DT;
                        buff.flags.insert("noSp".into());
                        k.add(owner, buff)?;
                    }
                }
            }
            Event::Tick if k.up() => {
                if let Self::Shield { interval, .. } = *self {
                    let now = k.world.time;
                    let state = k
                        .world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .prepared_summon_mut()?;
                    if now + 1e-9 >= state.next_pulse {
                        state.next_pulse += interval;
                        self.shield(k, false)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
