// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
const PLUS: [[i32; 2]; 5] = [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]];
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "fearless",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_nearl2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "ACTIVE_RANGE",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("defPct".into(), n(bb, "def", 0.)),
    ]
    .into();
    crate::battle::content::operators::mechanics::skill_range(raw, s)?;
    let penetration = n(t1, "def_penetrate", 0.);
    if penetration != 0. {
        s.extra.effects.permanent.push(persistent(
            "nearl2:dawn",
            [("defIgnorePct".into(), penetration)].into(),
        ));
    }
    s.extra.tactics = Tactics::Nearl2(Nearl2 {
        scale: n(bb, "value", 0.),
        stun: n(bb, "value2", 0.),
        dawn_scale: n(t, "atk_scale", 0.),
        dawn_stun: n(t, "stun", 0.),
        dawn_grid: raw["talents"]
            .as_array()
            .and_then(|a| a.iter().find(|t| t["index"] == 0))
            .and_then(|t| t["rangeGrid"].as_array())
            .map(|_| tactical_grid(&raw["talents"][0]["rangeGrid"], false))
            .transpose()?
            .unwrap_or(PLUS.to_vec()),
        blocked: n(tb, "atk_scale", 1.),
        stand: n(tb, "value", 0.),
        stand_speed: n(tb, "attack_speed", 0.),
        stand_hp: n(tb, "hp_ratio", 1.),
    });
    Ok(())
}
pub(crate) struct Nearl2 {
    scale: f64,
    stun: f64,
    dawn_scale: f64,
    dawn_stun: f64,
    dawn_grid: Vec<[i32; 2]>,
    blocked: f64,
    stand: f64,
    stand_speed: f64,
    stand_hp: f64,
}
#[derive(Default)]
pub(crate) struct State {
    sun: Option<Id>,
    stood: bool,
    dawn_times: u32,
}
impl Nearl2 {
    fn dawn(&self, k: &mut KitContext<'_>, times: u32) -> Result<(), String> {
        let keys = k.range(&self.dawn_grid)?;
        let amount = k.stat(k.unit, "atk")? * self.dawn_scale;
        for enemy in k.world.unit_ids().collect::<Vec<_>>() {
            let u = k.world.unit(enemy)?;
            if u.side != health::Side::Enemy
                || !u.stats.alive
                || u.hidden
                || !body::in_range(u, &keys)
            {
                continue;
            }
            for _ in 0..times {
                if !k.world.unit(enemy)?.stats.alive {
                    break;
                }
                k.damage(enemy, amount, DamageType::True, false, "talent")?;
            }
            if k.world.unit(enemy)?.stats.alive {
                k.status(enemy, "stun", self.dawn_stun, None)?;
            }
        }
        Ok(())
    }
}
impl Behavior for Nearl2 {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            -60
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                let previous = k.host.last_operator;
                let times = if let Some(previous) = previous.filter(|&a| a != id) {
                    if k.faction(previous, "kazimierzShip", &[])? {
                        2
                    } else {
                        1
                    }
                } else {
                    1
                };
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .nearl2_mut()?
                    .dawn_times = times;
                self.dawn(k, times)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                let previous_kazimierz = k
                    .host
                    .last_operator
                    .map(|previous| k.faction(previous, "kazimierzShip", &[]))
                    .transpose()?
                    .unwrap_or(false);
                let tile = k.best_summon_tile(&k.summon_tiles(&PLUS, true)?);
                let token = if let Some(tile) = tile {
                    k.summon("token_10019_nearl2_sword", tile, skill.time_left)?
                } else {
                    None
                };
                k.world.unit_mut(id)?.operator.tactical.nearl2_mut()?.sun = token;
                let source = token.unwrap_or(id);
                let own = k.unit;
                k.unit = source;
                let result: Result<(), String> = (|| {
                    let keys = k.range(&PLUS)?;
                    let amount = k.stat(own, "atk")? * self.scale;
                    for target in k.world.unit_ids().collect::<Vec<_>>() {
                        let u = k.world.unit(target)?;
                        if u.side == health::Side::Enemy
                            && u.stats.alive
                            && !u.hidden
                            && body::in_range(u, &keys)
                        {
                            for _ in 0..if token.is_some() && previous_kazimierz {
                                2
                            } else {
                                1
                            } {
                                if k.world.unit(target)?.stats.alive {
                                    k.damage(target, amount, DamageType::True, true, "skill")?;
                                }
                            }
                            if k.world.unit(target)?.stats.alive {
                                k.status(target, "stun", self.stun, None)?;
                            }
                        }
                    }
                    Ok(())
                })();
                k.unit = own;
                result?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                if let Some(token) = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .nearl2_mut()?
                    .sun
                    .take()
                {
                    let own = k.unit;
                    k.unit = token;
                    let result = k.retreat();
                    k.unit = own;
                    result?;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id) => {
                if request.is_attack && k.world.unit(request.hit.target)?.blocked_by().is_some() {
                    request.damage.mul *= self.blocked;
                }
                let blocker = k.world.unit(request.hit.target)?.blocked_by();
                if request.is_attack
                    && k.active()?
                    && blocker.is_some()
                    && (blocker == Some(id)
                        || blocker == k.world.unit(id)?.operator.tactical.nearl2()?.sun)
                {
                    request.hit.damage_type = DamageType::True;
                }
            }
            KitEvent::Fatal(context)
                if context.target == id
                    && !context.prevented
                    && self.stand > 0.
                    && !k.world.unit(id)?.operator.tactical.nearl2()?.stood =>
            {
                context.prevented = true;
                k.world.unit_mut(id)?.operator.tactical.nearl2_mut()?.stood = true;
                k.buff(
                    id,
                    "nearl2:stand",
                    [
                        ("hpMul".into(), (1. - self.stand).max(0.05)),
                        ("aspd".into(), self.stand_speed),
                    ]
                    .into(),
                    f64::INFINITY,
                )?;
                let hp = k.stat(id, "maxHp")? * self.stand_hp;
                k.world.unit_mut(id)?.stats.hp = hp.max(1.);
                self.dawn(
                    k,
                    k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .nearl2()?
                        .dawn_times
                        .max(1),
                )?;
            }
            _ => {}
        }
        Ok(())
    }
}
