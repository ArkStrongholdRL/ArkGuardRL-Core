// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "dollkeeper",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_ghost2_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_ghost2_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_ghost2_3",
            kind: Kind::Duration,
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
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_ghost2_1" => Mode::First,
        "skchr_ghost2_2" => Mode::Second,
        "skchr_ghost2_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("aspd".into(), n(bb, "attack_speed", 0.)),
        ("hpPct".into(), n(bb, "max_hp", 0.)),
        (
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        ),
    ]
    .into();
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.hit_all_blocked = true;
        s.combat.skill_profile = Some(p);
    }
    let team = if n(t1, "max_hp", 0.) != 0. {
        Some(TeamBuff {
            key: "ghost2:abyss",
            characters: &[
                "char_263_skadi",
                "char_143_ghost",
                "char_218_cuttle",
                "char_474_glady",
                "char_1023_ghost2",
                "char_4145_ulpia",
            ],
            mods: [("hpPct".into(), n(t1, "max_hp", 0.))].into(),
        })
    } else {
        None
    };
    s.extra.tactics = Tactics::Ghost2(Ghost2 {
        team,
        mode: mode,
        slow: n(t, "move_speed", 0.),
        scale: n(t, "atk_scale", 0.),
        doll_atk: n(tb, "atk", 0.),
        doll_hp: n(tb, "max_hp", 0.),
        extra: n(bb, "attack@atk_scale_ex", 0.),
        loss: n(bb, "attack@hp_ratio", 0.),
        grid: if raw["skill"]["rangeGrid"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            tactical_grid(&raw["skill"]["rangeGrid"], true)?
        } else {
            vec![
                [-1, -1],
                [-1, 0],
                [-1, 1],
                [0, -1],
                [0, 0],
                [0, 1],
                [1, -1],
                [1, 0],
                [1, 1],
            ]
        },
    });
    Ok(())
}
pub(crate) struct Ghost2 {
    team: Option<TeamBuff>,
    pub mode: Mode,
    pub slow: f64,
    pub scale: f64,
    pub doll_atk: f64,
    pub doll_hp: f64,
    pub extra: f64,
    pub loss: f64,
    pub grid: Vec<[i32; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub dollkeeper: super::dollkeeper::State,
    pub heavy: BTreeMap<Id, bool>,
}

impl Ghost2 {
    fn enter(&self, k: &mut KitContext<'_>) -> Result<bool, String> {
        super::dollkeeper::enter(k, 20., 1., true)
    }
    fn resume(&self, k: &mut KitContext<'_>, skill: &mut skills::Runtime) -> Result<(), String> {
        super::dollkeeper::resume(k, skill)
    }
}
impl Behavior for Ghost2 {
    fn priority(&self, event: &KitEvent<'_>, world: &World, id: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            if self.mode == Mode::Second && world.unit(id).is_ok_and(|u| u.operator.active_skill) {
                10
            } else {
                -100
            }
        } else if matches!(event, KitEvent::Before(_)) {
            -100
        } else {
            0
        }
    }
    fn can_attack(&self, _host: &Host, world: &World, id: Id) -> Result<bool, String> {
        Ok(!world.unit(id)?.operator.tactical.ghost2()?.dollkeeper.doll)
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        super::dollkeeper::handle(k, event)?;
        let id = k.unit;
        match event {
            KitEvent::BattleStart => {
                if let Some(team) = &self.team {
                    let units = k.host.roster.as_ref().unwrap().ids().to_vec();
                    for target in units {
                        if team
                            .characters
                            .contains(&k.host.tac_def(target)?.char_id.as_str())
                        {
                            team.apply(k.world, k.buffs, k.host, id, target)?;
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id && self.mode == Mode::First => {
                let keys = k.range(&self.grid)?;
                let mut pick = None;
                let mut lowest = f64::INFINITY;
                for target in k.allies(&keys)? {
                    if target == id {
                        continue;
                    }
                    let ratio = k.world.unit(target)?.stats.hp / k.stat(target, "maxHp")?;
                    if ratio < lowest {
                        lowest = ratio;
                        pick = Some(target);
                    }
                }
                if let Some(target) = pick {
                    let maximum = k.stat(id, "maxHp")?;
                    let other = k.stat(target, "maxHp")?;
                    let mine = k.world.unit(id)?.stats.hp / maximum;
                    k.world.unit_mut(id)?.stats.hp = (maximum * lowest).min(maximum).max(1.);
                    k.world.unit_mut(target)?.stats.hp = (other * mine).min(other).max(1.);
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id
                && self.mode == Mode::Second
                && context.reason != "death"
                && context.reason != "substitute" =>
            {
                self.enter(k)?;
            }
            KitEvent::Resume { unit, skill } if *unit == id => {
                self.resume(k, skill)?;
            }
            KitEvent::Attack {
                context,
                skill: Some(skill),
            } if context.attacker == id => {
                self.resume(k, skill)?;
            }
            KitEvent::Fatal(event) if event.target == id && !event.prevented => {
                if k.world
                    .unit(id)?
                    .operator
                    .tactical
                    .ghost2()?
                    .dollkeeper
                    .switching
                    || self.mode == Mode::Second && k.active()?
                {
                    event.prevented = true;
                } else {
                    event.prevented = self.enter(k)?;
                }
            }
            KitEvent::BeforeStatus(context)
                if context.target == id
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .ghost2()?
                        .dollkeeper
                        .switching
                    && ["stun", "freeze", "sleep"].contains(&context.key.as_str()) =>
            {
                context.cancelled = true;
            }
            KitEvent::Before(context)
                if context.attacker == id && self.mode == Mode::Third && k.active()? =>
            {
                let ratio = k.world.unit(id)?.stats.hp / k.stat(id, "maxHp")?;
                let mut heavy = BTreeMap::new();
                for &target in &context.targets {
                    heavy.insert(
                        target,
                        k.world.unit(target)?.stats.hp / k.stat(target, "maxHp")? >= ratio - 1e-9,
                    );
                }
                k.world.unit_mut(id)?.operator.tactical.ghost2_mut()?.heavy = heavy;
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && self.mode == Mode::Third
                    && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target {
                    if k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .ghost2()?
                        .heavy
                        .get(&target)
                        .copied()
                        .unwrap_or(false)
                    {
                        if k.world.unit(target)?.stats.alive {
                            let amount = k.stat(id, "atk")? * self.extra;
                            k.damage(target, amount, DamageType::Physical, true, "ghost2Weight")?;
                        }
                    } else if k.world.unit(id)?.stats.alive {
                        let amount = k.stat(id, "maxHp")? * self.loss;
                        k.lose_hp(amount)?;
                    }
                }
            }
            KitEvent::Scheduled(due) => {
                if !k.up()
                    || !k
                        .world
                        .unit(id)?
                        .operator
                        .tactical
                        .ghost2()?
                        .dollkeeper
                        .doll
                {
                    return Ok(());
                }
                if due[1] {
                    if self.doll_hp != 0. {
                        k.buff(
                            id,
                            "ghost2:moduleY",
                            [("hpPct".into(), self.doll_hp)].into(),
                            0.3,
                        )?;
                    }
                    if self.doll_atk != 0. {
                        k.buff(
                            id,
                            "ghost2:module",
                            [("atkPct".into(), self.doll_atk)].into(),
                            0.3,
                        )?;
                    }
                }
                if !k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .ghost2()?
                    .dollkeeper
                    .switching
                    && (due[2] || due[4])
                {
                    let keys = k.range(&[
                        [-1, -1],
                        [-1, 0],
                        [-1, 1],
                        [0, -1],
                        [0, 0],
                        [0, 1],
                        [1, -1],
                        [1, 0],
                        [1, 1],
                    ])?;
                    for target in k.enemies(Some(&keys), false)? {
                        if due[2] && self.slow != 0. {
                            k.buff(
                                target,
                                "ghost2:embrace",
                                [("moveMul".into(), (1. + self.slow).max(0.))].into(),
                                0.5,
                            )?;
                        }
                        if due[4] && self.scale > 0. {
                            let amount = k.stat(id, "atk")? * self.scale;
                            k.damage(target, amount, DamageType::Arts, false, "embrace")?;
                        }
                    }
                }
            }

            _ => {}
        }
        Ok(())
    }
}
