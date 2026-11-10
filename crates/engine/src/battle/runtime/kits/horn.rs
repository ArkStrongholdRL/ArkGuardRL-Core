// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "fortress",
    ranged: true,
    can_hit_fly: false,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_horn_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_horn_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_horn_2",
            kind: Kind::Ammo,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_horn_1" => Mode::First,
        "skchr_horn_2" => Mode::Second,
        "skchr_horn_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.fortress = true;
    s.combat.area.splash = Some(Splash {
        radius: 1.,
        scale: 1.,
        hits_fly: false,
        fortress_melee: false,
    });
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.skill_area = s.combat.area;
    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
    }
    if mode == Mode::Second {
        s.extra.effects.ammo_count = n(bb, "attack@s2.trigger_time", 10.);
        s.combat.skill_scale = n(bb, "attack@s2.atk_scale", 1.);
        if let Some(a) = &mut s.combat.skill_area.splash {
            a.fortress_melee = true;
        }
    }
    if mode == Mode::Third {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            (
                "batPct".into(),
                flat_bat(raw, n(bb, "base_attack_time", 0.)),
            ),
        ]
        .into();
    }
    s.extra.tactics = Tactics::Horn(Horn {
        mode: mode,
        duration: n(&raw["skill"], "duration", 0.),
        flare_radius: n(bb, "projectile_range", 1.7),
        flare_duration: n(bb, "projectile_delay_time", 6.),
        ammo: n(bb, "attack@s2.trigger_time", 10.).max(1.),
        magic: n(bb, "attack@s2.magic_atk_scale", 0.),
        over_atk: n(bb, "horn_s_3[overload_start].atk", n(bb, "atk", 0.)) - n(bb, "atk", 0.),
        over_interval: n(bb, "horn_s_3[overload_start].interval", 0.2).max(0.05),
        over_peak: n(bb, "horn_s_3[overload_start].hp_ratio", 0.),
        over_ramp: n(
            bb,
            "horn_s_3[overload_start].damage_duration",
            n(&raw["skill"], "duration", 24.) / 2.,
        )
        .max(0.1),
        tank_atk: n(t, "atk", 0.),
        blood_hp: n(t1, "max_hp", 0.),
        blood_heal: n(t1, "hp_ratio", 1.),
        blood_aspd: n(t1, "attack_speed", 0.),
        blood_def: n(t1, "def", 0.),
        blocked_mul: n(tb, "atk_scale", 0.),
        module_aspd: n(tm, "attack_speed", 0.),
    });
    Ok(())
}
pub(crate) struct Horn {
    pub mode: Mode,
    pub duration: f64,
    pub flare_radius: f64,
    pub flare_duration: f64,
    pub ammo: f64,
    pub magic: f64,
    pub over_atk: f64,
    pub over_interval: f64,
    pub over_peak: f64,
    pub over_ramp: f64,
    pub tank_atk: f64,
    pub blood_hp: f64,
    pub blood_heal: f64,
    pub blood_aspd: f64,
    pub blood_def: f64,
    pub blocked_mul: f64,
    pub module_aspd: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub elapsed: f64,
    pub ammo_remaining: f64,
    pub accumulator: f64,
    pub overloaded: bool,
    pub flare: bool,
    pub flares: Vec<([f64; 2], f64)>,
    pub blood: bool,
}

impl Behavior for Horn {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            -40
        } else {
            0
        }
    }
    fn payload(
        &self,
        _: &World,
        _: Id,
        payload: &mut attacks::Direct,
        area: &mut attacks::Area,
    ) -> Result<(), String> {
        if self.mode == Mode::First && payload.is_skill && !area.fortress_melee {
            if let Some(splash) = &mut area.splash {
                splash.radius = splash.radius.max(self.flare_radius);
            }
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                let s = k.world.unit_mut(id)?.operator.tactical.horn_mut()?;
                s.ammo_remaining = skill.ammo_left;
                s.elapsed = 0.;
                s.accumulator = 0.;
                s.overloaded = false;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                let main = self.duration.max(0.1) - self.over_ramp.min(self.duration.max(0.1));
                let enter = {
                    let s = k.world.unit_mut(id)?.operator.tactical.horn_mut()?;
                    s.elapsed += context.dt;
                    !s.overloaded && s.elapsed >= main - 1e-9
                };
                if enter {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .horn_mut()?
                        .overloaded = true;
                    k.buff(
                        id,
                        "horn:overload",
                        [("atkPct".into(), self.over_atk)].into(),
                        f64::INFINITY,
                    )?;
                }
                if k.world.unit(id)?.operator.tactical.horn()?.overloaded && self.over_peak > 0. {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .horn_mut()?
                        .accumulator += context.dt;
                    for _ in 0..30 {
                        let rate = {
                            let s = k.world.unit_mut(id)?.operator.tactical.horn_mut()?;
                            if s.accumulator < self.over_interval {
                                break;
                            }
                            s.accumulator -= self.over_interval;
                            self.over_peak * ((s.elapsed - main) / self.over_ramp).min(1.)
                        };
                        if rate > 0. {
                            let amount = k.stat(id, "maxHp")? * rate * self.over_interval;
                            k.lose_hp(amount)?;
                        }
                        if !k.up() {
                            break;
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .horn_mut()?
                    .overloaded = false;
                k.remove(id, "horn:overload")?;
            }
            KitEvent::Before(context) if context.attacker == id && context.is_skill => {
                if self.mode == Mode::Second {
                    let shot =
                        self.ammo - k.world.unit(id)?.operator.tactical.horn()?.ammo_remaining + 1.;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .horn_mut()?
                        .overloaded = shot > self.ammo * 0.5 + 1e-9;
                }
                if self.mode == Mode::First {
                    let melee = if let Some(&target) = context.targets.first() {
                        let keys = k.range(&[[0, 0], [0, 1]])?;
                        k.world.unit(target)?.blocked_by() == Some(id)
                            || body::in_range(k.world.unit(target)?, &keys)
                    } else {
                        true
                    };
                    k.world.unit_mut(id)?.operator.tactical.horn_mut()?.flare = !melee;
                }
            }
            KitEvent::Impact(impact) if impact.attacker == id && impact.is_skill => {
                if self.mode == Mode::First
                    && impact.kind == attacks::VictimKind::Main
                    && k.world.unit(id)?.operator.tactical.horn()?.flare
                {
                    k.world.unit_mut(id)?.operator.tactical.horn_mut()?.flare = false;
                    let until = k.world.time + self.flare_duration;
                    let limit = k.host.tactical_limit;
                    let state = k.world.unit_mut(id)?.operator.tactical.horn_mut()?;
                    if state.flares.len() >= limit {
                        return Err("Horn flare capacity exceeded".into());
                    }
                    state.flares.push((impact.position, until));
                }
                if self.mode == Mode::Second
                    && k.world.unit(id)?.operator.tactical.horn()?.overloaded
                    && self.magic > 0.
                {
                    if let Some(target) = impact.target.filter(|&t| Host::tac_up(k.world, t)) {
                        let amount = k.stat(id, "atk")? * self.magic;
                        let mut request = hit::Request::new(
                            health::Hit {
                                source: Some(id),
                                credit: Some(id),
                                target,
                                damage_type: DamageType::Arts,
                                no_sp: false,
                                silent: false,
                                hp_loss: false,
                                on_applied: false,
                            },
                            Damage {
                                amount,
                                mul: 1.,
                                resolved: false,
                                penetration: Default::default(),
                            },
                        );
                        request.is_skill = true;
                        request.is_splash = true;
                        request.tags.push("hornOverload".into());
                        k.deal(&mut request)?;
                    }
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id)
                && self.blocked_mul > 1.
                && k.world.unit(request.hit.target)?.blocked_by().is_some() =>
            {
                request.damage.amount *= self.blocked_mul;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::AmmoUsed,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .horn_mut()?
                    .ammo_remaining -= 1.;
            }

            KitEvent::Fatal(event)
                if event.target == id
                    && !event.prevented
                    && !k.world.unit(id)?.operator.tactical.horn()?.blood =>
            {
                event.prevented = true;
                k.world.unit_mut(id)?.operator.tactical.horn_mut()?.blood = true;
                k.buff(
                    id,
                    "horn:bloodBattle",
                    [
                        ("hpMul".into(), 1. - self.blood_hp),
                        ("aspd".into(), self.blood_aspd),
                        ("defPct".into(), self.blood_def),
                    ]
                    .into(),
                    f64::INFINITY,
                )?;
                k.world.unit_mut(id)?.stats.hp = k.world.unit(id)?.stats.hp.max(1.);
                let amount = k.stat(id, "maxHp")? * self.blood_heal;
                k.heal(
                    id,
                    amount,
                    health::HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
            KitEvent::Scheduled(due) => {
                let now = k.world.time;
                let mut flares =
                    std::mem::take(&mut k.world.unit_mut(id)?.operator.tactical.horn_mut()?.flares);
                flares.retain(|(_, until)| *until > now + 1e-9);
                if due[2] {
                    for &(at, _) in &flares {
                        for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                            if Host::tac_up(k.world, target)
                                && !k.world.unit(target)?.hidden
                                && body::in_radius(k.world.unit(target)?, at, self.flare_radius)
                            {
                                k.status(target, "reveal", 0.5, None)?;
                            }
                        }
                    }
                }
                k.world.unit_mut(id)?.operator.tactical.horn_mut()?.flares = flares;
                if k.up() {
                    if due[2] && self.tank_atk != 0. {
                        for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                            if Host::tac_up(k.world, target)
                                && k.host.tac_def(target)?.profession == "TANK"
                            {
                                k.buff(
                                    target,
                                    "horn:fortress",
                                    [("atkPct".into(), self.tank_atk)].into(),
                                    0.5,
                                )?;
                            }
                        }
                    }
                    if due[1] && self.module_aspd != 0. && k.world.unit(id)?.blocking().is_empty() {
                        k.buff(
                            id,
                            "horn:moduleY",
                            [("aspd".into(), self.module_aspd)].into(),
                            0.3,
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        let mut state = Self::default();
        if let Some(previous) = previous {
            state.flares = previous.flares.clone();
        }
        state
    }
}
