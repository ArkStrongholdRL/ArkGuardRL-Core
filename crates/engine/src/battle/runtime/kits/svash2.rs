// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::deploy;
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "counsellor",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_svash2_2",
        kind: crate::battle::sp::Kind::Charges,
        sp_type: skills::SpType::Time,
        rule: "SKILL_RANGE",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_svash2_2" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    s.trigger_range = Some(tactical_grid(&raw["skill"]["rangeGrid"], false)?);
    s.extra.tactics = Tactics::Svash2(Svash2 {
        grid: tactical_grid(&raw["skill"]["rangeGrid"], false)?,
        targets: n(bb, "max_target", 6.).max(1.) as usize,
        scale: n(bb, "atk_scale", 1.),
        cold: n(bb, "cold", 0.),
        cut: n(bb, "cost", 0.),
        stacks: n(bb, "max_stack_cnt", 2.).floor().max(1.) as u32,
        sp: n(t, "sp", 0.),
        respawn: n(t, "respawn_time", 1.),
        defence: n(t1, "def", 0.),
        regen: n(t1, "hp_recovery_per_sec_by_max_hp_ratio", 0.),
        double_after: n(t1, "interval", 15.),
    });
    Ok(())
}
pub(crate) struct Svash2 {
    grid: Vec<[i32; 2]>,
    targets: usize,
    scale: f64,
    cold: f64,
    cut: f64,
    stacks: u32,
    sp: f64,
    respawn: f64,
    defence: f64,
    regen: f64,
    double_after: f64,
}
#[derive(Default, Clone)]
pub(crate) struct State {
    costs: BTreeMap<Id, f64>,
    casts: BTreeMap<Id, u32>,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        previous.cloned().unwrap_or_default()
    }
}
impl Svash2 {
    fn eye_cost(k: &KitContext<'_>) -> f64 {
        k.host
            .data
            .table("tokens")
            .and_then(|t| t.get("token_10057_svash2_eagle2"))
            .and_then(|t| t["stats"]["cost"].as_f64())
            .unwrap_or(14.)
    }
    fn left(k: &KitContext<'_>, unit: Id) -> Result<bool, String> {
        let u = k.world.unit(unit)?;
        Ok(u.deployment.kind == deploy::Kind::Operator
            && u.owner == k.world.unit(k.unit)?.owner
            && (unit == k.unit || u.stats.base_number("cost") < Self::eye_cost(k)))
    }
    fn slash(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let keys = k.range(&self.grid)?;
        let mut targets = search::Host::enemies(
            k.host,
            k.unit,
            &keys,
            targeting::Profile {
                can_hit_fly: true,
                ignore_stealth: true,
                ..Default::default()
            },
            k.world,
            k.world.unit_capacity(),
        )?;
        targeting::sort_enemies(
            k.world,
            k.host,
            k.unit,
            &mut targets,
            targeting::Priority::None,
        )?;
        let amount = k.stat(k.unit, "atk")? * self.scale;
        for target in targets.into_iter().take(self.targets) {
            k.damage(target, amount, DamageType::Physical, true, "svash2")?;
            if Host::tac_up(k.world, target) {
                k.status(target, "cold", self.cold, None)?;
                k.status(target, "reveal", self.cold, None)?;
            }
        }
        Ok(())
    }
}
impl Behavior for Svash2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                self.slash(k)?;
                let eye = Self::eye_cost(k);
                let mut right = Vec::new();
                let mut left = Vec::new();
                for unit in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    let u = k.world.unit(unit)?;
                    if unit == id
                        || u.stats.alive
                        || u.removed
                        || u.deployment.kind != deploy::Kind::Operator
                    {
                        continue;
                    }
                    let cost = u.stats.base_number("cost");
                    if cost >= eye {
                        right.push((unit, cost));
                    } else {
                        left.push((unit, cost));
                    }
                }
                right.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                left.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let preferred = right.iter().find(|(unit, _)| {
                    k.host
                        .tac_def(*unit)
                        .is_ok_and(|d| ["WARRIOR", "CASTER", "SNIPER"].contains(&d.profession))
                });
                if let Some(&(target, cost)) = preferred.or(right.first()).or(left.first()) {
                    if self.cut > 0. {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .svash2_mut()?
                            .costs
                            .entry(target)
                            .or_insert(cost);
                        k.world.unit_mut(target)?.stats.edit_base(|b| {
                            b.0.insert("cost".into(), (cost - self.cut).max(0.));
                        });
                    }
                }
                for (target, _) in left {
                    let stacks = k
                        .world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .svash2_mut()?
                        .casts
                        .entry(target)
                        .or_default();
                    *stacks = (*stacks + 1).min(self.stacks);
                }
            }
            KitEvent::BattleStart if k.up() && self.sp > 0. => {
                for unit in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    if Host::tac_up(k.world, unit) && Self::left(k, unit)? {
                        let owner = k.unit;
                        k.unit = unit;
                        let amount = self.sp;
                        let result = k.skill(|skill, host, w| {
                            skill.gain(w, host, amount, "talent", false, false)
                        });
                        k.unit = owner;
                        result?;
                    }
                }
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::Deploy,
                skill,
            } => {
                if k.world.time > 0.
                    && Self::left(k, *unit)?
                    && (*unit == id || k.up())
                    && self.sp > 0.
                {
                    let amount = self.sp;
                    k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                        skill.gain(world, host, amount, "talent", false, false)
                    })?;
                }
                if let Some(cost) = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .svash2_mut()?
                    .costs
                    .remove(unit)
                {
                    k.world.unit_mut(*unit)?.stats.edit_base(|b| {
                        b.0.insert("cost".into(), cost);
                    });
                }
                let casts = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .svash2_mut()?
                    .casts
                    .remove(unit)
                    .unwrap_or(0);
                let owner = k.unit;
                k.unit = *unit;
                let result: Result<(), String> = (|| {
                    for _ in 0..casts {
                        self.slash(k)?;
                    }
                    Ok(())
                })();
                k.unit = owner;
                result?;
            }
            KitEvent::Death { unit, .. }
                if Self::left(k, *unit)?
                    && (*unit == id || k.up())
                    && self.respawn > 0.
                    && self.respawn < 1. =>
            {
                let u = k.world.unit_mut(*unit)?;
                if !u.removed && u.deployment.respawn_at.is_finite() {
                    u.deployment.respawn_at = u.deployment.death_at
                        + (u.deployment.respawn_at - u.deployment.death_at) * self.respawn;
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                let factor = if k.world.time - k.world.unit(id)?.deployment.deployed_at
                    >= self.double_after - 1e-9
                {
                    2.
                } else {
                    1.
                };
                for unit in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    if Host::tac_up(k.world, unit) && k.faction(unit, "kjeragShip", &["kjerag"])? {
                        k.buff(
                            unit,
                            "svash2:snow",
                            [
                                ("defFlat".into(), self.defence * factor),
                                ("hpRegenRatio".into(), self.regen * factor),
                            ]
                            .into(),
                            0.5,
                        )?;
                    }
                }
            }
            KitEvent::BeforeStatus(context)
                if context.key == "freeze"
                    && k.world.unit(context.target)?.side == health::Side::Ally
                    && k.has(context.target, "svash2:snow") =>
            {
                context.cancelled = true;
            }
            _ => {}
        }
        Ok(())
    }
}
