// SPDX-License-Identifier: GPL-3.0-or-later
//! Friendly tactical decisions: preparation gates and battle event effects.
use super::*;
use crate::battle::{
    deploy::Kind,
    stats::{Buff, Numbers},
};
#[derive(Clone, Debug)]
pub struct BattleChoice {
    pub effect_id: String,
    pub sequence: u64,
    pub bench_units: usize,
}
#[derive(Clone)]
enum Gate {
    Always,
    AtLeast(usize),
    AtMost(usize),
    Row(usize),
}
#[derive(Clone)]
enum Effect {
    Heal(f64),
    Flawless(Numbers),
    Stats(Numbers),
}
#[derive(Clone)]
struct Plan {
    key: String,
    gate: Gate,
    effect: Effect,
    active: bool,
}
#[derive(Default)]
pub(super) struct Choices {
    plans: Vec<Plan>,
    heal: f64,
}
impl BattleChoice {
    fn compile(&self, data: &Catalog) -> Result<Plan, String> {
        let buffs = data
            .table("effects")
            .and_then(|t| t.get(&self.effect_id))
            .and_then(|r| r["buffs"].as_array())
            .ok_or("Unknown battle choice")?;
        if buffs.len() > 32 || self.sequence == 0 || self.bench_units > 4096 {
            return Err("Invalid battle choice limits".into());
        }
        let mut gate = Gate::Always;
        let mut effect = None;
        for b in buffs {
            let number = |key: &str| {
                b["bb"][key]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("Invalid choice parameter: {key}"))
            };
            match b["key"].as_str() {
                Some("global_special_choice_all_activated") => (),
                Some(
                    "global_special_choice_prep_finish_bench_at_least"
                    | "global_special_choice_prep_finish_bench_at_most"
                    | "global_special_choice_prep_finish_same_row_at_least",
                ) => {
                    let count = number("count")?.max(0.).trunc().min(4096.) as usize;
                    gate = match b["key"].as_str().unwrap() {
                        "global_special_choice_prep_finish_bench_at_least" => Gate::AtLeast(count),
                        "global_special_choice_prep_finish_bench_at_most" => Gate::AtMost(count),
                        _ => Gate::Row(count.max(1)),
                    };
                }
                Some("char_respawntime_mul") => {
                    effect = Some(Effect::Stats(Numbers::from([(
                        "redeployMul".into(),
                        number("scale")?.max(0.),
                    )])))
                }
                Some("env_gbuff_new_with_verify") => {
                    effect = Some(match b["bbStr"]["key"].as_str() {
                        Some("act1autochess_debuff_3") => Effect::Heal(number("value")?.max(0.)),
                        Some("act1autochess_debuff_9") => Effect::Flawless(Numbers::from([
                            ("atkPct".into(), number("atk")?),
                            ("defPct".into(), number("def")?),
                        ])),
                        _ => return Err("Unsupported verified choice modifier".into()),
                    });
                }
                Some("env_gbuff_new") => {
                    let mut mods = Numbers::new();
                    for (key, value) in b["bb"].as_object().ok_or("Missing choice modifiers")? {
                        let key = match key.as_str() {
                            "magic_resist_penetrate_fixed" => "resIgnoreFlat",
                            "def_penetrate" => "defIgnorePct",
                            _ => return Err(format!("Unsupported choice modifier: {key}")),
                        };
                        mods.insert(
                            key.into(),
                            value
                                .as_f64()
                                .filter(|v| v.is_finite())
                                .ok_or("Invalid choice modifier")?,
                        );
                    }
                    effect = Some(Effect::Stats(mods));
                }
                _ => return Err(format!("Battle choice not implemented: {}", b["key"])),
            }
        }
        let active = match gate {
            Gate::Always => true,
            Gate::AtLeast(n) => self.bench_units >= n,
            Gate::AtMost(n) => self.bench_units <= n,
            Gate::Row(_) => false,
        };
        Ok(Plan {
            key: format!("choice:{}#{}", self.effect_id, self.sequence),
            gate,
            effect: effect.ok_or("Choice has no battle effect")?,
            active,
        })
    }
    pub(crate) fn validate(&self, data: &Catalog) -> Result<(), String> {
        self.compile(data).map(|_| ())
    }
}
impl Field {
    pub fn configure_choices(&mut self, choices: &[BattleChoice]) -> Result<(), String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 {
            return Err("Choices require an unstarted field".into());
        }
        if choices.len() > super::super::grid::N {
            return Err("Battle choice capacity exceeded".into());
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut plans = Vec::new();
        for choice in choices {
            let plan = choice.compile(&self.core.host.data)?;
            if !keys.insert(plan.key.clone()) {
                return Err("Duplicate battle choice identity".into());
            }
            plans.push(plan);
        }
        self.core.host.choices = Choices { plans, heal: 0. };
        Ok(())
    }
}
impl Host {
    pub(super) fn choices_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if self.choices.plans.is_empty() {
            return Ok(());
        }
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut rows = std::collections::BTreeMap::<i32, usize>::new();
        for &id in &ids {
            let u = w.unit(id)?;
            if u.deployment.kind == Kind::Operator && u.stats.alive && u.deployed {
                *rows.entry(u.deployment.tile[0]).or_default() += 1;
            }
        }
        let max = rows.values().copied().max().unwrap_or(0);
        for plan in &mut self.choices.plans {
            if let Gate::Row(n) = plan.gate {
                plan.active = max >= n;
            }
        }
        for plan in self.choices.plans.clone().into_iter().filter(|p| p.active) {
            match plan.effect {
                Effect::Heal(amount) => self.choices.heal += amount,
                Effect::Stats(mods) => {
                    for &id in &ids {
                        if w.unit(id)?.deployment.kind == Kind::Operator {
                            self.equipment_stat(w, r, id, plan.key.clone(), mods.clone())?;
                        }
                    }
                }
                Effect::Flawless(_) => (),
            }
        }
        self.choices_tick(w, r)
    }
    fn choice_full_health(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if !self
            .choices
            .plans
            .iter()
            .any(|p| p.active && matches!(p.effect, Effect::Flawless(_)))
        {
            return Ok(());
        }
        if w.unit(id)?.deployment.kind != Kind::Operator {
            return Ok(());
        }
        let full = Self::choice_is_full(w, id)?;
        for plan in self.choices.plans.clone().into_iter().filter(|p| p.active) {
            let Effect::Flawless(mods) = plan.effect else {
                continue;
            };
            let has = w
                .unit(id)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(&plan.key));
            if full && !has {
                r.add(
                    w,
                    self,
                    id,
                    buffs::Spec {
                        buff: Buff {
                            key: Some(plan.key),
                            mods,
                            permanent: true,
                            ..Default::default()
                        },
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            } else if !full && has {
                r.remove(w, self, id, &plan.key)?;
            }
        }
        Ok(())
    }
    fn choice_is_full(w: &mut World, id: Id) -> Result<bool, String> {
        let u = w.unit_mut(id)?;
        let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        Ok(u.stats.alive && u.deployed && u.stats.hp >= max - 1e-6)
    }
    pub(super) fn choices_clock_quiet(&self, w: &mut World) -> Result<bool, String> {
        let plans: Vec<_> = self
            .choices
            .plans
            .iter()
            .filter(|p| p.active && matches!(p.effect, Effect::Flawless(_)))
            .collect();
        if plans.is_empty() {
            return Ok(true);
        }
        for &id in self.roster.as_ref().unwrap().ids() {
            if w.unit(id)?.deployment.kind != Kind::Operator {
                continue;
            }
            let full = Self::choice_is_full(w, id)?;
            if plans.iter().any(|plan| {
                w.unit(id)
                    .unwrap()
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some(&plan.key))
                    != full
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn choices_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if !self
            .choices
            .plans
            .iter()
            .any(|p| p.active && matches!(p.effect, Effect::Flawless(_)))
        {
            return Ok(());
        }
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        for id in ids {
            self.choice_full_health(w, r, id)?;
        }
        Ok(())
    }
    pub(super) fn choices_damaged(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        q: &hit::Request,
    ) -> Result<(), String> {
        if self.choices.plans.is_empty() {
            return Ok(());
        }
        let u = w.unit(e.target)?;
        if u.side != health::Side::Ally
            || !matches!(u.deployment.kind, Kind::Operator | Kind::Token)
        {
            return Ok(());
        }
        if self.choices.heal > 0.
            && e.amount > 0.
            && !q.hit.hp_loss
            && q.hit.damage_type != super::super::damage::DamageType::Elemental
            && u.stats.alive
            && u.deployed
            && u.stats.hp > 0.
        {
            if w.unit_mut(e.target)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("noHeal")
            {
                return self.choice_full_health(w, r, e.target);
            }
            self.support_heal(
                w,
                r,
                None,
                e.target,
                self.choices.heal,
                health::HealOptions {
                    self_heal: true,
                    ..Default::default()
                },
            )?;
        }
        self.choice_full_health(w, r, e.target)
    }
}
