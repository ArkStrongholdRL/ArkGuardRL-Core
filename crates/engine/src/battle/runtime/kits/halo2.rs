// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "slower",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_halo2_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_halo2_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_halo2_2",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
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
        "skchr_halo2_1" => Mode::First,
        "skchr_halo2_2" => Mode::Second,
        "skchr_halo2_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.sluggish = n(tb, "sluggish", 0.8);
    s.combat.skill_sluggish = s.combat.sluggish;
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("batPct".into(), {
            let v = n(bb, "base_attack_time", 0.);
            if v > 0. {
                v.max(-0.9)
            } else {
                flat_bat(raw, v).max(-0.9)
            }
        }),
    ]
    .into();
    if mode == Mode::Second {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
        s.combat.skill_sluggish = n(bb, "sluggish", 3.);
    }
    if mode == Mode::Third {
        skill_range(raw, s)?;
        let mut p = s.combat.profile;
        p.max_targets = n(bb, "attack@max_target", 1.).max(1.);
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Halo2(Halo2 {
        mode: mode,
        sluggish: n(tb, "sluggish", 0.8),
        jumps: n(bb, "attack@chain.max_target", 3.).floor().max(0.) as usize,
        jump_radius: n(bb, "attack@projectile_range", 1.7),
        link_radius: n(bb, "ability_range_radius", 2.),
        link_targets: n(bb, "max_target", 2.).floor().max(0.) as usize,
        link_scale: n(bb, "atk_scale_link", n(bb, "atk_scale", 1.)),
        force: n(bb, "force", 0.),
        locks: n(bb, "attack@max_target", 1.).max(1.) as usize,
        share: n(bb, "attack@atk_share", 0.),
        aspd: n(t, "attack_speed", 0.),
        stack_cap: n(t, "max_stack_cnt", 18.).max(1.) as u32,
        full_atk: n(tm, "atk", 0.),
        fragile: suffix_number(t1, "damage_scale", 1.),
        fragile_max: suffix_number(
            t1,
            "damage_scale_max",
            suffix_number(t1, "damage_scale", 1.),
        ),
        fragile_time: suffix_number(t1, "interval", 7.),
    });
    Ok(())
}
pub(crate) struct Halo2 {
    pub mode: Mode,
    pub sluggish: f64,
    pub jumps: usize,
    pub jump_radius: f64,
    pub link_radius: f64,
    pub link_targets: usize,
    pub link_scale: f64,
    pub force: f64,
    pub locks: usize,
    pub share: f64,
    pub aspd: f64,
    pub stack_cap: u32,
    pub full_atk: f64,
    pub fragile: f64,
    pub fragile_max: f64,
    pub fragile_time: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub locks: Vec<Id>,
    pub linking: bool,
    pub stacks: u32,
    pub stay: BTreeMap<Id, f64>,
}

impl Behavior for Halo2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start | skills::Event::End,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .halo2_mut()?
                    .locks
                    .clear();
            }
            KitEvent::Before(context) if context.attacker == id && k.active()? => {
                if self.mode == Mode::Second && context.is_skill {
                    let candidates = k.enemies(None, true)?;
                    let mut best = None;
                    let mut remaining = f64::NEG_INFINITY;
                    for target in candidates {
                        let value =
                            targeting::Queries::remaining_distance(k.host, target, k.world)?;
                        if value > remaining + 1e-9 {
                            remaining = value;
                            best = Some(target);
                        }
                    }
                    if let Some(target) = best {
                        context.targets = vec![target];
                    }
                }
                if self.mode == Mode::Third {
                    let keys = k.keys()?;
                    let mut keep = Vec::new();
                    for target in k.world.unit(id)?.operator.tactical.halo2()?.locks.clone() {
                        if Host::tac_up(k.world, target)
                            && body::in_range(k.world.unit(target)?, &keys)
                            && targeting::can_target_enemy(
                                k.world,
                                k.host,
                                Some(id),
                                target,
                                targeting::Profile {
                                    can_hit_fly: true,
                                    ..Default::default()
                                },
                            )?
                        {
                            keep.push(target);
                        }
                    }
                    for &target in &context.targets {
                        if keep.len() < self.locks && !keep.contains(&target) {
                            keep.push(target);
                        }
                    }
                    keep.truncate(self.locks);
                    context.targets = keep.clone();
                    k.world.unit_mut(id)?.operator.tactical.halo2_mut()?.locks = keep;
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target {
                    if self.mode == Mode::First {
                        if self.jumps > 64 {
                            return Err("Halo chain capacity exceeded".into());
                        }
                        let mut previous = target;
                        let mut position = k.world.unit(target)?.position;
                        for _ in 0..self.jumps {
                            let mut best = None;
                            let mut distance = f64::INFINITY;
                            for next in k.radius(position, self.jump_radius)? {
                                if next == previous
                                    || !targeting::can_target_enemy(
                                        k.world,
                                        k.host,
                                        Some(id),
                                        next,
                                        targeting::Profile {
                                            can_hit_fly: true,
                                            ..Default::default()
                                        },
                                    )?
                                {
                                    continue;
                                }
                                let d = body::distance(k.world.unit(next)?, position);
                                if d < distance - 1e-9 {
                                    distance = d;
                                    best = Some(next);
                                }
                            }
                            let Some(next) = best else {
                                break;
                            };
                            let amount = k.stat(id, "atk")?;
                            let mut request = hit::Request::new(
                                health::Hit {
                                    source: Some(id),
                                    credit: Some(id),
                                    target: next,
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
                            request.is_attack = true;
                            request.is_skill = true;
                            request.tags.push("chain".into());
                            k.deal(&mut request)?;
                            if Host::tac_up(k.world, next) {
                                k.status(next, "sluggish", self.sluggish, None)?;
                            }
                            previous = next;
                            position = k.world.unit(next)?.position;
                        }
                    }
                    if self.mode == Mode::Second {
                        let position = k.world.unit(target)?.position;
                        let mut near = k.radius(position, self.link_radius)?;
                        near.retain(|&next| next != target);
                        near.sort_by(|a, b| {
                            body::distance(k.world.unit(*a).unwrap(), position)
                                .total_cmp(&body::distance(k.world.unit(*b).unwrap(), position))
                                .then(
                                    k.world
                                        .unit(*a)
                                        .unwrap()
                                        .targeting
                                        .spawn_seq
                                        .cmp(&k.world.unit(*b).unwrap().targeting.spawn_seq),
                                )
                        });
                        for next in near.into_iter().take(self.link_targets) {
                            if !targeting::can_target_enemy(
                                k.world,
                                k.host,
                                Some(id),
                                next,
                                targeting::Profile {
                                    can_hit_fly: true,
                                    ..Default::default()
                                },
                            )? {
                                continue;
                            }
                            k.pull(next, self.force, Some(position))?;
                            if Host::tac_up(k.world, next) {
                                let amount = k.stat(id, "atk")? * self.link_scale;
                                k.damage(next, amount, DamageType::Arts, true, "link")?;
                            }
                        }
                    }
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if self.mode == Mode::Third
                && k.active()?
                && request.hit.damage_type == DamageType::Arts
                && !request.tags.iter().any(|t| t == "link") =>
            {
                let state = k.world.unit(id)?.operator.tactical.halo2()?;
                if self.share > 0.
                    && !state.linking
                    && state.locks.len() > 1
                    && state.locks.contains(&request.hit.target)
                {
                    let locks = state.locks.clone();
                    k.world.unit_mut(id)?.operator.tactical.halo2_mut()?.linking = true;
                    let result: Result<(), String> = (|| {
                        for target in locks {
                            if target == request.hit.target || !Host::tac_up(k.world, target) {
                                continue;
                            }
                            let source = request.hit.source.or(Some(id));
                            let amount = request.damage.amount * self.share;
                            let mut linked = hit::Request::new(
                                health::Hit {
                                    source,
                                    credit: source,
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
                            linked.is_skill = true;
                            linked.dodge_mode = injury::DodgeMode::Ignore;
                            linked.tags = vec!["skill".into(), "link".into()];
                            k.deal(&mut linked)?;
                        }
                        Ok(())
                    })();
                    k.world.unit_mut(id)?.operator.tactical.halo2_mut()?.linking = false;
                    result?;
                }
            }
            KitEvent::Status(context)
                if context.source == Some(id) && context.key == "sluggish" && k.up() =>
            {
                let count = {
                    let state = k.world.unit_mut(id)?.operator.tactical.halo2_mut()?;
                    if state.stacks >= self.stack_cap {
                        return Ok(());
                    }
                    state.stacks += 1;
                    state.stacks
                };
                k.buff(
                    id,
                    "halo2:model",
                    [
                        ("aspd".into(), self.aspd * count as f64),
                        (
                            "atkPct".into(),
                            if count >= self.stack_cap {
                                self.full_atk
                            } else {
                                0.
                            },
                        ),
                    ]
                    .into(),
                    f64::INFINITY,
                )?;
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                let foes = k.enemies(None, true)?;
                let mut stay =
                    std::mem::take(&mut k.world.unit_mut(id)?.operator.tactical.halo2_mut()?.stay);
                stay.retain(|target, _| foes.contains(target));
                for target in foes {
                    let time = stay.entry(target).or_insert(0.);
                    *time += 0.25;
                    let value = if *time > self.fragile_time {
                        self.fragile_max
                    } else {
                        self.fragile
                    };
                    if value > 1. {
                        k.status(target, "fragile", 0.4, Some(value - 1.))?;
                    }
                }
                k.world.unit_mut(id)?.operator.tactical.halo2_mut()?.stay = stay;
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
            state.stay = previous.stay.clone();
        }
        state
    }
}
