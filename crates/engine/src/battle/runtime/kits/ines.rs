// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "agent",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_ines_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_ines_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_ines_3",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
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
        "skchr_ines_1" => Mode::First,
        "skchr_ines_2" => Mode::Second,
        "skchr_ines_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    if mode == Mode::Second {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
        s.skill_flags.insert("stealth".into());
        skill_range(raw, s)?;
    }
    if mode == Mode::Third {
        s.rule_override = Some(skills::Rule::Never);
        s.extra.deploy = false;
    }
    s.extra.tactics = Tactics::Ines(Ines {
        mode: mode,
        dp: n(bb, "cost", 1.),
        bleed: n(bb, "bleed_atk_scale", 0.4),
        bleed_duration: n(bb, "bleed_duration", 3.),
        steal_aspd: n(bb, "attack@steal_atk_speed", 5.),
        aspd_cap: n(bb, "attack@steal_atk_speed_max", 50.),
        steal_atk: n(t, "steal_atk", 90.),
        atk_cap: n(t, "steal_atk_max", f64::INFINITY),
        bind: n(t, "duration", 5.),
        slow: n(t1, "move_speed", -0.3),
        respawn: n(tb, "respawn_time", 0.),
        recall_scale: n(bb, "atk_scale", 1.1),
        recall_radius: n(bb, "projectile_range", 1.4),
        recall_targets: n(bb, "max_target", 4.).max(1.) as usize,
        attack: n(bb, "atk", 0.),
        base_grid: tactical_grid(&raw["rangeGrid"], false)?,
    });
    Ok(())
}
pub(crate) struct Ines {
    pub mode: Mode,
    pub dp: f64,
    pub bleed: f64,
    pub bleed_duration: f64,
    pub steal_aspd: f64,
    pub aspd_cap: f64,
    pub steal_atk: f64,
    pub atk_cap: f64,
    pub bind: f64,
    pub slow: f64,
    pub respawn: f64,
    pub recall_scale: f64,
    pub recall_radius: f64,
    pub recall_targets: usize,
    pub attack: f64,
    pub base_grid: Vec<[i32; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub woven: BTreeSet<Id>,
    pub atk_victims: Vec<Id>,
    pub aspd_victims: BTreeSet<Id>,
    pub aspd: f64,
    pub sentry: Vec<usize>,
    pub sentry_at: Option<[f64; 2]>,
    pub placed: bool,
    pub first_retreat: bool,
}

const BLEED: u64 = 1515;
impl Ines {
    fn clear_aspd(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let targets = std::mem::take(
            &mut k
                .world
                .unit_mut(k.unit)?
                .operator
                .tactical
                .ines_mut()?
                .aspd_victims,
        );
        for target in targets {
            if k.world.unit(target).is_ok() {
                k.remove(target, &format!("ines:aspd:{}", k.unit))?;
            }
        }
        k.world.unit_mut(k.unit)?.operator.tactical.ines_mut()?.aspd = 0.;
        k.remove(k.unit, "ines:aspdGain")
    }
    fn refresh(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let mut targets = std::mem::take(
            &mut k
                .world
                .unit_mut(k.unit)?
                .operator
                .tactical
                .ines_mut()?
                .atk_victims,
        );
        targets.retain(|&target| k.world.unit(target).is_ok_and(|u| u.stats.alive));
        let gain = (self.steal_atk * targets.len() as f64).min(self.atk_cap);
        k.world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .ines_mut()?
            .atk_victims = targets;
        if k.world.unit(k.unit)?.stats.alive {
            if gain > 0. {
                k.buff(
                    k.unit,
                    "ines:atkGain",
                    [("atkFlat".into(), gain)].into(),
                    f64::INFINITY,
                )?;
            } else {
                k.remove(k.unit, "ines:atkGain")?;
            }
        }
        Ok(())
    }
    fn initial_retreat(k: &mut KitContext<'_>) -> Result<(), String> {
        k.retreat()?;
        k.world.unit_mut(k.unit)?.deployment.respawn_at = k.world.time;
        Ok(())
    }
}
impl Behavior for Ines {
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        if matches!((role, event), (Role::Target, KitEvent::Death { .. }))
            && k.world.unit(k.unit)?.deployment.remove_reason.as_deref() != Some("expired")
        {
            let keys = k.range(&self.base_grid)?;
            let at = k.world.unit(k.unit)?.position;
            let state = k.world.unit_mut(k.unit)?.operator.tactical.ines_mut()?;
            state.sentry = keys;
            if self.mode == Mode::Third {
                state.sentry_at = Some(at);
            }
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                skill,
                ..
            } if *unit == id && self.mode == Mode::Third => {
                if k.world.unit(id)?.operator.tactical.ines()?.placed {
                    k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                        skill.activate(world, host, "deploy", false)
                    })?;
                } else {
                    k.world.unit_mut(id)?.operator.tactical.ines_mut()?.placed = true;
                    k.later(0., Self::initial_retreat)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                if self.mode == Mode::Second {
                    self.clear_aspd(k)?;
                }
                if self.mode == Mode::Third {
                    k.buff(
                        id,
                        "ines:s3",
                        [("atkPct".into(), self.attack)].into(),
                        f64::INFINITY,
                    )?;
                    let from = k
                        .world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .ines_mut()?
                        .sentry_at
                        .take();
                    if let Some(from) = from {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .ines_mut()?
                            .sentry
                            .clear();
                        let to = k.world.unit(id)?.position;
                        let delta = [to[0] - from[0], to[1] - from[1]];
                        let len = delta[0] * delta[0] + delta[1] * delta[1];
                        let mut hits = Vec::new();
                        for target in k.enemies(None, true)? {
                            let p = k.world.unit(target)?.position;
                            let t = if len > 1e-9 {
                                (((p[0] - from[0]) * delta[0] + (p[1] - from[1]) * delta[1]) / len)
                                    .clamp(0., 1.)
                            } else {
                                0.
                            };
                            let distance = body::distance(
                                k.world.unit(target)?,
                                [from[0] + t * delta[0], from[1] + t * delta[1]],
                            );
                            if distance <= self.recall_radius + 1e-9 {
                                hits.push((t, distance, target));
                            }
                        }
                        hits.sort_by(|a, b| {
                            a.0.total_cmp(&b.0)
                                .then(a.1.total_cmp(&b.1))
                                .then(a.2.cmp(&b.2))
                        });
                        let damage = k.stat(id, "atk")? * self.recall_scale;
                        for (_, _, target) in hits.into_iter().take(self.recall_targets) {
                            k.damage(target, damage, DamageType::Physical, true, "sentryRecall")?;
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                if self.mode == Mode::Second {
                    self.clear_aspd(k)?;
                }
                if self.mode == Mode::Third {
                    k.remove(id, "ines:s3")?;
                }
            }
            KitEvent::Attack { context, .. }
                if context.attacker == id && context.is_skill && self.mode == Mode::Second =>
            {
                k.dp(self.dp)?;
            }
            KitEvent::Impact(impact)
                if impact.attacker == id && impact.kind == attacks::VictimKind::Main =>
            {
                let Some(target) = impact.target else {
                    return Ok(());
                };
                if self.mode == Mode::First && impact.is_skill {
                    k.dp(self.dp)?;
                    if k.world.unit(target)?.stats.alive {
                        let mut buff = Buff {
                            key: Some(format!("ines:bleed:{id}")),
                            ..Default::default()
                        };
                        buff.runtime.duration = self.bleed_duration + 1e-6;
                        buff.runtime.interval = 1.;
                        buff.runtime.refresh = buffs::Refresh::Extend;
                        buff.runtime.on_tick = Some(BLEED);
                        k.add(target, buff)?;
                    }
                }
                // S2's live onHit hook follows the skill state at impact. S1's
                // attack override above stays attached to the launched projectile.
                if self.mode == Mode::Second && k.active()? && k.world.unit(target)?.stats.alive {
                    let gain = self
                        .steal_aspd
                        .min(self.aspd_cap - k.world.unit(id)?.operator.tactical.ines()?.aspd);
                    if gain > 0. {
                        let key = format!("ines:aspd:{id}");
                        let old = k
                            .world
                            .unit(target)?
                            .stats
                            .buffs()
                            .iter()
                            .find(|b| b.key.as_deref() == Some(&key))
                            .and_then(|b| b.mods.get("aspd"))
                            .copied()
                            .unwrap_or(0.);
                        k.buff(
                            target,
                            &key,
                            [("aspd".into(), old - gain)].into(),
                            f64::INFINITY,
                        )?;
                        let s = k.world.unit_mut(id)?.operator.tactical.ines_mut()?;
                        s.aspd += gain;
                        s.aspd_victims.insert(target);
                        let total = s.aspd;
                        k.buff(
                            id,
                            "ines:aspdGain",
                            [("aspd".into(), total)].into(),
                            f64::INFINITY,
                        )?;
                    }
                }
            }
            KitEvent::Landed { event, request }
                if event.source == Some(id)
                    && request.hit.damage_type != DamageType::Elemental
                    && k.world.unit(event.target)?.side == health::Side::Enemy =>
            {
                if self.mode == Mode::Third && k.active()? && event.amount > 0. {
                    k.dp(self.dp)?;
                }
                if k.world.unit(id)?.stats.alive
                    && k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .ines_mut()?
                        .woven
                        .insert(event.target)
                    && k.world.unit(event.target)?.stats.alive
                {
                    k.status(event.target, "bind", self.bind, None)?;
                    k.buff(
                        event.target,
                        &format!("ines:atkSteal:{id}"),
                        [("atkFlat".into(), -self.steal_atk)].into(),
                        f64::INFINITY,
                    )?;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .ines_mut()?
                        .atk_victims
                        .push(event.target);
                    self.refresh(k)?;
                }
            }
            KitEvent::Death { unit, .. } => {
                if *unit == id {
                    let targets = std::mem::take(
                        &mut k
                            .world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .ines_mut()?
                            .atk_victims,
                    );
                    for target in targets {
                        if k.world.unit(target).is_ok() {
                            k.remove(target, &format!("ines:atkSteal:{id}"))?;
                        }
                    }
                    let now = k.world.time;
                    let unit = k.world.unit_mut(id)?;
                    let s = unit.operator.tactical.ines_mut()?;
                    s.woven.clear();
                    if !s.first_retreat
                        && self.respawn != 0.
                        && unit.deployment.respawn_at.is_finite()
                    {
                        s.first_retreat = true;
                        unit.deployment.respawn_at =
                            now + (unit.deployment.respawn_at - now) * (1. + self.respawn).max(0.);
                    }
                } else {
                    self.refresh(k)?;
                }
            }
            KitEvent::Scheduled(due) if due[1] => {
                let mut keys = k.world.unit(id)?.operator.tactical.ines()?.sentry.clone();
                if k.up() {
                    keys.extend(k.keys()?);
                }
                keys.sort_unstable();
                keys.dedup();
                if !keys.is_empty() {
                    // Reveals must also cover enemies that cannot yet be selected.
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        let enemy = k.world.unit(target)?;
                        if !enemy.stats.alive || enemy.hidden || !body::in_range(enemy, &keys) {
                            continue;
                        }
                        let mut buff = Buff {
                            key: Some(format!("ines:sentry:{id}")),
                            mods: [("moveMul".into(), (1. + self.slow).max(0.))].into(),
                            flags: ["reveal".into()].into(),
                            ..Default::default()
                        };
                        buff.runtime.duration = 0.25;
                        k.add(target, buff)?;
                    }
                }
                let valid: BTreeSet<_> = k.world.unit_ids().collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .ines_mut()?
                    .woven
                    .retain(|target| valid.contains(target));
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == BLEED => {
                if k.buffs
                    .get(k.world, context.unit, context.buff)?
                    .runtime
                    .source
                    != Some(id)
                {
                    return Ok(());
                }
                let amount = k.stat(id, "atk")? * self.bleed;
                k.damage(context.unit, amount, DamageType::Arts, true, "dot")?;
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
            state.woven = previous.woven.clone();
            state.atk_victims = previous.atk_victims.clone();
            state.sentry = previous.sentry.clone();
            state.sentry_at = previous.sentry_at.clone();
            state.placed = previous.placed.clone();
            state.first_retreat = previous.first_retreat.clone();
        }
        state
    }
}
