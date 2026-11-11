// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "sword",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_blkkgt_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_blkkgt_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_blkkgt_2",
            kind: Kind::Charges,
            sp_type: SpType::Attack,
            rule: "SKILL_RANGE",
            attack_override: false,
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
        "skchr_blkkgt_1" => Mode::First,
        "skchr_blkkgt_2" => Mode::Second,
        "skchr_blkkgt_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.normal_hits = 2;
    let mods: Numbers = [
        ("defIgnoreFlat".into(), n(tb, "def_penetrate_fixed", 0.)),
        ("atkPct".into(), n(t1, "atk", 0.)),
    ]
    .into();
    if mods.values().any(|v| *v != 0.) {
        s.extra
            .effects
            .permanent
            .push(persistent("blkkgt:contract", mods));
    }
    if mode == Mode::First {
        s.combat.skill_hits = 2;
        s.combat.skill_scale = n(bb, "atk_scale_s1", 1.);
        let mut p = s.combat.profile;
        p.max_targets = n(bb, "max_target", 5.).max(1.);
        s.combat.skill_profile = Some(p);
    }
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
        s.extra.duration = Some(
            kit_text_count(&raw["skill"]["desc"], "总计", "次", 10, 64)? as f64
                * n(bb, "d_hit_interval", 0.3).max(0.05),
        );
    }
    skill_range(raw, s)?;
    s.extra.tactics = Tactics::Blkkgt(Blkkgt {
        mode: mode,
        slashes: kit_text_count(&raw["skill"]["desc"], "总计", "次", 10, 64)?,
        interval: n(bb, "d_hit_interval", 0.3).max(0.05),
        pull_interval: n(bb, "p_hit_interval", 1.).max(0.1),
        targets: n(bb, "max_target", 1.).floor().max(1.) as usize,
        grid: tactical_grid(&raw["skill"]["rangeGrid"], false)?,
        slash_scale: n(bb, "d_atk_scale", 1.),
        dot_scale: n(bb, "dot_scale", 1.),
        finisher: n(bb, "e_atk_scale_end", 1.),
        pull_force: n(bb, "p_force", 0.),
        final_force: n(bb, "e_force", 0.),
        free_hits: n(bb, "blkkgt_s_2[not_blocked].trig_cnt", 2.)
            .floor()
            .max(0.) as u32,
        blocked_hits: n(bb, "blkkgt_s_2[blocked].trig_cnt", 3.).floor().max(0.) as u32,
        prob: n(t, "prob", 0.),
        skill_prob: n(bb, "prob", n(t, "prob", 0.)),
        crit: n(t, "atk_scale", 1.),
        tremble: n(t, "not_combat", 0.),
        pen: n(t1, "def_penetrate", 0.),
        initial_tremble: n(t1, "not_combat", 0.),
        skill_mul: n(tb, "damage_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Blkkgt {
    pub mode: Mode,
    pub slashes: u32,
    pub interval: f64,
    pub pull_interval: f64,
    pub targets: usize,
    pub grid: Vec<[i32; 2]>,
    pub slash_scale: f64,
    pub dot_scale: f64,
    pub finisher: f64,
    pub pull_force: f64,
    pub final_force: f64,
    pub free_hits: u32,
    pub blocked_hits: u32,
    pub prob: f64,
    pub skill_prob: f64,
    pub crit: f64,
    pub tremble: f64,
    pub pen: f64,
    pub initial_tremble: f64,
    pub skill_mul: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub active: bool,
    pub finale: bool,
    pub slashes: u32,
    pub accumulator: f64,
    pub pull_acc: f64,
    pub seen: BTreeSet<Id>,
}

impl Blkkgt {
    fn slash(&self, k: &mut KitContext<'_>, scale: f64) -> Result<(), String> {
        let targets = k.enemies(None, true)?;
        for target in targets.into_iter().take(self.targets) {
            let amount = k.stat(k.unit, "atk")? * scale;
            k.damage(target, amount, DamageType::Physical, true, "slash")?;
        }
        Ok(())
    }
    fn pull(&self, k: &mut KitContext<'_>, force: f64, finale: bool) -> Result<(), String> {
        for target in k.enemies(None, true)? {
            if finale {
                k.pull(target, force, Some(k.world.unit(k.unit)?.position))?;
            } else {
                k.pull(target, force, None)?;
            }
        }
        Ok(())
    }
}
impl Behavior for Blkkgt {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                if self.mode == Mode::Second {
                    let keys = k.range(&self.grid)?;
                    for target in k
                        .enemies(Some(&keys), false)?
                        .into_iter()
                        .take(self.targets)
                    {
                        let count = if k.world.unit(target)?.blocked_by().is_some() {
                            self.blocked_hits
                        } else {
                            self.free_hits
                        };
                        if count > 64 {
                            return Err("Slash count exceeds capacity".into());
                        }
                        for _ in 0..count {
                            if !Host::tac_up(k.world, target) {
                                break;
                            }
                            let amount = k.stat(id, "atk")? * self.dot_scale;
                            k.damage(target, amount, DamageType::Physical, true, "slash")?;
                        }
                    }
                }
                if self.mode == Mode::Third {
                    let state = k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?;
                    state.active = true;
                    state.slashes = 1;
                    state.accumulator = 0.;
                    state.pull_acc = 0.;
                    let mut buff = Buff {
                        key: Some("blkkgt:slashes".into()),
                        flags: ["invulnerable".into()].into(),
                        ..Default::default()
                    };
                    buff.runtime.duration = self.slashes as f64 * self.interval + 1.;
                    k.add(id, buff)?;
                    self.slash(k, self.slash_scale)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                {
                    let state = k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?;
                    state.accumulator += context.dt;
                    state.pull_acc += context.dt;
                }
                for _ in 0..self.slashes {
                    let due = {
                        let state = k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?;
                        if state.accumulator + 1e-9 >= self.interval && state.slashes < self.slashes
                        {
                            state.accumulator -= self.interval;
                            state.slashes += 1;
                            true
                        } else {
                            false
                        }
                    };
                    if !due {
                        break;
                    }
                    self.slash(k, self.slash_scale)?;
                }
                let pull = {
                    let state = k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?;
                    if state.pull_acc + 1e-9 >= self.pull_interval {
                        state.pull_acc -= self.pull_interval;
                        true
                    } else {
                        false
                    }
                };
                if pull {
                    self.pull(k, self.pull_force, false)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?.active = false;
                k.remove(id, "blkkgt:slashes")?;
                if context.reason != "death" && k.world.unit(id)?.stats.alive {
                    k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?.finale = true;
                    let result = self.slash(k, self.finisher);
                    k.world.unit_mut(id)?.operator.tactical.blkkgt_mut()?.finale = false;
                    result?;
                    self.pull(k, self.final_force, true)?;
                }
            }
            KitEvent::BeforeStatus(context)
                if context.target == id
                    && k.world.unit(id)?.operator.tactical.blkkgt()?.active
                    && matches!(context.key.as_str(), "stun" | "freeze") =>
            {
                context.cancelled = true;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id)
                && k.world.unit(request.hit.target)?.side == health::Side::Enemy =>
            {
                if self.pen > 0. && k.has(request.hit.target, "tremble") {
                    request.damage.penetration.def_pct += self.pen;
                }
                let probability =
                    if k.active()? || k.world.unit(id)?.operator.tactical.blkkgt()?.finale {
                        self.skill_prob
                    } else {
                        self.prob
                    };
                if probability > 0. && (probability >= 1. || k.host.rng.next_f64() < probability) {
                    request.damage.amount *= self.crit;
                    if self.tremble > 0. && Host::tac_up(k.world, request.hit.target) {
                        k.status(request.hit.target, "tremble", self.tremble, None)?;
                    }
                }
                if request.is_skill {
                    request.damage.mul *= self.skill_mul;
                }
            }
            KitEvent::Scheduled(due) if due[2] => {
                let valid: BTreeSet<_> = k.world.unit_ids().collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .blkkgt_mut()?
                    .seen
                    .retain(|target| valid.contains(target));
            }
            KitEvent::Tick if k.up() && self.initial_tremble > 0. => {
                for target in k.enemies(None, true)? {
                    if targeting::flying(k.world, target)? {
                        continue;
                    }
                    if k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .blkkgt_mut()?
                        .seen
                        .insert(target)
                    {
                        k.status(target, "tremble", self.initial_tremble, None)?;
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
            state.seen = previous.seen.clone();
        }
        state
    }
}
