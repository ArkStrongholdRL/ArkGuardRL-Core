// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "reaper",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_excu2_2",
            kind: Kind::Ammo,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_excu2_1",
            kind: Kind::Ammo,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_excu2_3",
            kind: Kind::Ammo,
            sp_type: SpType::Attack,
            rule: "ACTIVE_RANGE",
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
        "skchr_excu2_1" => Mode::First,
        "skchr_excu2_2" => Mode::Second,
        "skchr_excu2_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.extra.no_heal = true;
    let mut p = s.combat.profile;
    p.all_in_range = true;
    p.selection.ground_only = true;
    s.combat.profile = p;
    s.combat.area.selection = p.selection;
    s.combat.skill_area = s.combat.area;
    s.extra.support.reaper = n(tb, "value", 50.);
    s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 12.);
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("defPct".into(), n(bb, "def", 0.)),
        ("blockCnt".into(), n(bb, "block_cnt", 0.)),
        ("defIgnoreFlat".into(), n(bb, "def_penetrate_fixed", 0.)),
        (
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        ),
    ]
    .into();
    skill_range(raw, s)?;
    s.extra.tactics = Tactics::Excu2(Excu2 {
        mode: mode,
        prob: n(t, "prob", 0.),
        prob_add: n(t, "prob_add", 0.),
        bonus_ammo: n(t1, "add_count", 1.),
        ammo_cap: n(t1, "add_count_max_stack", 4.),
        dodge: n(bb, "prob", 0.),
        refill: n(bb, "recover_cnt", 1.),
        verdict: n(bb, "attack@final_atk_scale", 0.),
        stack_atk: n(bb, "attack@atk", 0.),
        stack_cap: n(bb, "attack@max_stack_cnt", 30.).max(1.) as u32,
        heal_ratio: n(bb, "trait_ratio", 1.),
    });
    Ok(())
}
pub(crate) struct Excu2 {
    pub mode: Mode,
    pub prob: f64,
    pub prob_add: f64,
    pub bonus_ammo: f64,
    pub ammo_cap: f64,
    pub dodge: f64,
    pub refill: f64,
    pub verdict: f64,
    pub stack_atk: f64,
    pub stack_cap: u32,
    pub heal_ratio: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub spent: u32,
    pub pending_ammo: f64,
    pub extra: bool,
    pub verdict: BTreeSet<Id>,
}

pub(in crate::battle::runtime) const DODGE: u64 = 1511;
impl Excu2 {
    fn extra_attack(k: &mut KitContext<'_>) -> Result<(), String> {
        let id = k.unit;
        if !skills::can_act(k.world, id)? || k.flag(id, "disarm")? {
            return Ok(());
        }
        k.world.unit_mut(id)?.operator.tactical.excu2_mut()?.extra = true;
        let result = k.skill(|skill, host, world| {
            let Some(profile) = ally::Host::effective_profile(host, id, Some(skill), world)? else {
                return Ok(());
            };
            let keys = host.roster.as_ref().unwrap().range(world, id)?;
            let selected = search::acquire(world, host, id, &keys, profile, world.unit_capacity())?;
            if selected.targets.is_empty() {
                return Ok(());
            }
            ally::Host::perform(host, id, profile, selected, Some(skill), world)
        });
        k.world.unit_mut(id)?.operator.tactical.excu2_mut()?.extra = false;
        result
    }
    fn refill(k: &mut KitContext<'_>, skill: &mut skills::Runtime) -> Result<(), String> {
        let pending = std::mem::take(
            &mut k
                .world
                .unit_mut(k.unit)?
                .operator
                .tactical
                .excu2_mut()?
                .pending_ammo,
        );
        if skill.bank.active {
            skill.add_ammo(pending);
        }
        Ok(())
    }
}
impl Behavior for Excu2 {
    fn reaper_multiplier(&self, world: &World, id: Id) -> Result<f64, String> {
        Ok(
            if self.mode == Mode::Third && world.unit(id)?.operator.active_skill {
                self.heal_ratio
            } else {
                1.
            },
        )
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
                let s = k.world.unit_mut(id)?.operator.tactical.excu2_mut()?;
                s.spent = 0;
                s.verdict.clear();
                let mut count: f64 = 0.;
                for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    if Host::tac_up(k.world, target)
                        && k.faction(target, "lateranoShip", &["laterano"])?
                    {
                        count += 1.;
                    }
                }
                skill.add_ammo(count.min(self.ammo_cap) * self.bonus_ammo);
                if self.mode == Mode::Second {
                    let mut buff = Buff {
                        key: Some("excu2:meleeDodge".into()),
                        ..Default::default()
                    };
                    buff.runtime.injury.priority = 2000.;
                    buff.runtime.injury.on_injury = Some(DODGE);
                    k.add(id, buff)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let targets = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.excu2_mut()?.verdict,
                );
                k.world.unit_mut(id)?.operator.tactical.excu2_mut()?.spent = 0;
                if self.mode == Mode::Third && k.world.unit(id)?.stats.alive {
                    for target in targets {
                        if k.world.unit(target).is_ok_and(|u| u.stats.alive) {
                            let amount = k.stat(id, "atk")? * self.verdict;
                            k.damage(target, amount, DamageType::Physical, true, "verdict")?;
                        }
                    }
                }
                k.remove(id, "excu2:verdict")?;
                k.remove(id, "excu2:meleeDodge")?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::AmmoUsed,
                ..
            } if *unit == id => {
                k.world.unit_mut(id)?.operator.tactical.excu2_mut()?.spent += 1;
                if self.mode == Mode::Third && k.active()? {
                    let mut buff = Buff {
                        key: Some("excu2:verdict".into()),
                        mods: [("atkPct".into(), self.stack_atk)].into(),
                        ..Default::default()
                    };
                    buff.runtime.refresh = buffs::Refresh::Stack;
                    buff.runtime.max_stacks = self.stack_cap;
                    k.add(id, buff)?;
                }
            }
            KitEvent::Scheduled(due) if due[2] => {
                let valid: BTreeSet<_> = k.world.unit_ids().collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .excu2_mut()?
                    .verdict
                    .retain(|target| valid.contains(target));
            }
            KitEvent::Resume { unit, skill } if *unit == id => {
                Self::refill(k, skill)?;
            }
            KitEvent::Before(context)
                if context.attacker == id && k.world.unit(id)?.operator.tactical.excu2()?.extra =>
            {
                context.no_ammo = true;
            }
            KitEvent::Attack { context, skill } if context.attacker == id => {
                if let Some(skill) = skill {
                    Self::refill(k, skill)?;
                }
                if !k.world.unit(id)?.operator.tactical.excu2()?.extra && k.up() {
                    let chance = self.prob
                        + if k.active()? {
                            self.prob_add
                                * k.world.unit(id)?.operator.tactical.excu2()?.spent as f64
                        } else {
                            0.
                        };
                    if chance > 0. && k.host.rng.chance(chance.min(1.)) {
                        k.later(0., Self::extra_attack)?;
                    }
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id && self.mode == Mode::Third && k.active()? =>
            {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .excu2_mut()?
                    .verdict
                    .extend(impact.target);
            }
            _ => {}
        }
        Ok(())
    }
}
impl Host {
    pub(in crate::battle::runtime) fn excu2_injury(
        c: &mut settlement::Context<'_, Self>,
        event: &mut injury::InjuryEvent,
    ) -> Result<(), String> {
        if c.request.is_projectile
            || !c.request.hit.source.is_some_and(|source| {
                c.world
                    .unit(source)
                    .is_ok_and(|u| u.side == health::Side::Enemy)
            })
        {
            return Ok(());
        }
        let id = c.request.hit.target;
        let def = c.host.tac_def(id)?;
        let Tactics::Excu2(p) = &def.extra.tactics else {
            return Err("Invalid Excu2 injury owner".into());
        };
        if c.dodge(event, Some(p.dodge))? {
            if c.host.roster.as_ref().unwrap().entry(id)?.skill().is_ok() {
                let mut skill = c.host.roster.as_mut().unwrap().take_skill(id)?;
                skill.add_ammo(p.refill);
                c.host.roster.as_mut().unwrap().restore_skill(id, skill)?;
            } else {
                c.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .excu2_mut()?
                    .pending_ammo += p.refill;
            }
        }
        Ok(())
    }
}
