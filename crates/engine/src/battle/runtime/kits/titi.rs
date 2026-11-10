// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::status;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "incantationmedic",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_titi_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_titi_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_titi_2",
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
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_titi_1" => Mode::First,
        "skchr_titi_2" => Mode::Second,
        "skchr_titi_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.extra.support.incantation = n(&raw["trait"]["bb"], "scale", 0.5);
    s.combat.profile.selection.hit_sleep = true;
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.skill_area = s.combat.area;
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    if mode == Mode::Second {
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.max_targets = n(bb, "attack@max_target", 2.).max(1.);
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Titi(Titi {
        mode: mode,
        still: n(t, "extra_atk_scale", 0.),
        dream: n(t, "damage_atk_scale", 0.),
        talent_scale: n(bb, "talent_scale", 1.),
        vigor_threshold: n(t1, "hp_ratio", 0.5),
        vigor: n(t1, "attack_speed", 0.),
        sleep: n(bb, "attack@sleep", n(bb, "sleep", 5.)),
        prob: n(bb, "attack@prob", 0.),
        chain_sleep: n(bb, "sleep", n(bb, "attack@sleep", 5.)),
        radius: n(bb, "range_radius", 1.5),
        chain_targets: n(bb, "max_target", 1.).max(1.) as usize,
        wake_min: n(bb, "min_atk_scale", 1.),
        wake_max: n(bb, "max_atk_scale", n(bb, "min_atk_scale", 1.)),
    });
    Ok(())
}
pub(crate) struct Titi {
    pub mode: Mode,
    pub still: f64,
    pub dream: f64,
    pub talent_scale: f64,
    pub vigor_threshold: f64,
    pub vigor: f64,
    pub sleep: f64,
    pub prob: f64,
    pub chain_sleep: f64,
    pub radius: f64,
    pub chain_targets: usize,
    pub wake_min: f64,
    pub wake_max: f64,
}
#[derive(Default)]
pub(crate) struct State {
    // Wake and spread in sleep-application order.
    pub asleep: Vec<(Id, f64)>,
    pub wards: Vec<Id>,
    pub accumulator: f64,
}

const ALLY_SLEEP: u64 = 1516;
impl Titi {
    fn sleeping(
        &self,
        k: &mut KitContext<'_>,
        target: Id,
        key: &str,
        guard: bool,
    ) -> Result<(), String> {
        let entered = !k.flag(target, "sleep")?;
        let mut buff = Buff {
            key: Some(key.into()),
            flags: ["sleep".into()].into(),
            ..Default::default()
        };
        buff.runtime.status = Some("sleep".into());
        if guard {
            buff.runtime.interval = 0.2;
            buff.runtime.on_tick = Some(ALLY_SLEEP);
        }
        k.add(target, buff)?;
        blocking::release(
            k.world,
            &mut allies::Blocking {
                host: k.host,
                buffs: k.buffs,
            },
            target,
        )?;
        status::Hooks::status_applied(
            k.host,
            &status::Context {
                source: Some(k.unit),
                target,
                key: "sleep".into(),
                duration: f64::INFINITY,
                value: None,
                cancelled: false,
                entered,
            },
            k.buffs,
            k.world,
        )
    }
    fn spread(&self, k: &mut KitContext<'_>, from: Id) -> Result<(), String> {
        let at = k.world.unit(from)?.position;
        let mut candidates = Vec::new();
        let nearby = k.radius(at, self.radius)?;
        let targets = k.host.tac_selectable(k.world, k.unit, nearby)?;
        for target in targets {
            if target == from || k.flag(target, "sleep")? {
                continue;
            }
            let taunt = k.stat(target, "taunt")?;
            let distance = body::distance(k.world.unit(target)?, at);
            candidates.push((taunt, distance, target));
        }
        candidates.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        for (_, _, target) in candidates.into_iter().take(self.chain_targets) {
            k.status(target, "sleep", self.chain_sleep, None)?;
        }
        Ok(())
    }
    fn clear(&self, k: &mut KitContext<'_>, key: &str) -> Result<(), String> {
        for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
            if k.world
                .unit(target)?
                .stats
                .buffs()
                .iter()
                .any(|buff| buff.key.as_deref() == Some(key) && buff.runtime.source == Some(k.unit))
            {
                k.remove(target, key)?;
            }
        }
        Ok(())
    }
}
impl Behavior for Titi {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            5
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let keys = k.keys()?;
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
                let mut wards = vec![id];
                if let Some(target) = pick {
                    wards.push(target);
                }
                for &target in &wards {
                    self.sleeping(k, target, "titi:ward", false)?;
                }
                let state = k.world.unit_mut(id)?.operator.tactical.titi_mut()?;
                state.wards = wards;
                state.accumulator = 0.25;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::Second => {
                let due = {
                    let s = k.world.unit_mut(id)?.operator.tactical.titi_mut()?;
                    s.accumulator += context.dt;
                    if s.accumulator + 1e-9 >= 0.25 {
                        s.accumulator = 0.;
                        true
                    } else {
                        false
                    }
                };
                if due {
                    let wards = k.world.unit(id)?.operator.tactical.titi()?.wards.clone();
                    let mut pulsed = BTreeSet::new();
                    for target in wards {
                        if !Host::tac_up(k.world, target) || !k.has(target, "titi:ward") {
                            continue;
                        }
                        let at = k.world.unit(target)?.position;
                        for enemy in k.radius(at, 1.7)? {
                            let reenter = pulsed.insert(enemy)
                                && !k.world.unit(enemy)?.stats.buffs().iter().any(|buff| {
                                    buff.runtime.status.as_deref() == Some("sleep")
                                        && buff.runtime.time_left > 0.5 + 1e-9
                                });
                            k.host.apply_status(
                                k.world,
                                k.buffs,
                                enemy,
                                "sleep",
                                status::Options {
                                    duration: 0.5,
                                    source: Some(id),
                                    reenter,
                                    ..Default::default()
                                },
                            )?;
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
                    self.clear(k, "titi:ward")?;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .titi_mut()?
                        .wards
                        .clear();
                }
                if self.mode == Mode::Third {
                    self.clear(k, "titi:allySleep")?;
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && impact.kind == attacks::VictimKind::Main
                    && self.mode != Mode::Second =>
            {
                if let Some(target) = impact
                    .target
                    .filter(|&target| Host::tac_up(k.world, target))
                {
                    if !k.flag(target, "sleep")?
                        && (self.mode == Mode::Third || k.host.rng.chance(self.prob))
                    {
                        k.status(target, "sleep", self.sleep, None)?;
                    }
                }
            }
            KitEvent::Landed {
                event: landed,
                request,
            } if landed.source == Some(id)
                && request.is_attack
                && !request.is_splash
                && request.hit.damage_type != DamageType::Elemental =>
            {
                let target = landed.target;
                if k.world.unit(target)?.side == health::Side::Enemy
                    && k.world.unit(target)?.stats.alive
                    && (k.world.unit(target)?.blocked_by().is_some()
                        || !k.world.unit(target)?.moving
                        || k.flag(target, "stun")?
                        || k.flag(target, "noMove")?
                        || k.stat(target, "moveSpeed")? <= 0.)
                {
                    let scale = if self.mode == Mode::Second && k.active()? {
                        self.talent_scale
                    } else {
                        1.
                    };
                    let amount = k.stat(id, "atk")? * self.still * scale;
                    k.damage(target, amount, DamageType::Arts, false, "titiStill")?;
                }
            }
            KitEvent::Scheduled(due) if k.up() => {
                if due[4] {
                    let scale = if self.mode == Mode::Second && k.active()? {
                        self.talent_scale
                    } else {
                        1.
                    };
                    let amount = k.stat(id, "atk")? * self.dream * scale;
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        if Host::tac_up(k.world, target)
                            && !k.world.unit(target)?.hidden
                            && k.flag(target, "sleep")?
                        {
                            k.damage(target, amount, DamageType::Arts, false, "titiDream")?;
                        }
                    }
                }
                if due[2] {
                    for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if Host::tac_up(k.world, target)
                            && k.faction(target, "sargonShip", &["sargon", "minos"])?
                            && k.world.unit(target)?.stats.hp / k.stat(target, "maxHp")?
                                > self.vigor_threshold
                        {
                            k.buff(
                                target,
                                "titi:vigor",
                                [("aspd".into(), self.vigor)].into(),
                                0.5,
                            )?;
                        }
                    }
                }
            }
            KitEvent::Status(context)
                if self.mode == Mode::Third
                    && context.key == "sleep"
                    && k.world.unit(context.target)?.side == health::Side::Enemy =>
            {
                let time = k.world.time;
                let asleep = &mut k.world.unit_mut(id)?.operator.tactical.titi_mut()?.asleep;
                if !asleep.iter().any(|&(target, _)| target == context.target) {
                    asleep.push((context.target, time));
                }
            }
            KitEvent::Tick if self.mode == Mode::Third => {
                let watched = k.world.unit(id)?.operator.tactical.titi()?.asleep.clone();
                for (target, time) in watched {
                    if !k.world.unit(target).is_ok_and(|u| u.stats.alive) {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .titi_mut()?
                            .asleep
                            .retain(|&(other, _)| other != target);
                        continue;
                    }
                    if k.flag(target, "sleep")? {
                        continue;
                    }
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .titi_mut()?
                        .asleep
                        .retain(|&(other, _)| other != target);
                    if k.up() && k.active()? {
                        let scale = self.wake_min
                            + (self.wake_max - self.wake_min)
                                * ((k.world.time - time) / self.chain_sleep.max(0.1)).min(1.);
                        let amount = k.stat(id, "atk")? * scale;
                        k.damage(target, amount, DamageType::Arts, true, "titiWake")?;
                        self.spread(k, target)?;
                    }
                }
            }
            KitEvent::Kill { victim } if self.mode == Mode::Third && *victim != id => {
                let asleep = &mut k.world.unit_mut(id)?.operator.tactical.titi_mut()?.asleep;
                if let Some(index) = asleep.iter().position(|&(target, _)| target == *victim) {
                    asleep.remove(index);
                    if k.up() && k.active()? {
                        self.spread(k, *victim)?;
                    }
                }
            }
            KitEvent::Fatal(event)
                if self.mode == Mode::Third
                    && event.target != id
                    && !event.prevented
                    && k.world.unit(event.target)?.side == health::Side::Ally =>
            {
                if let Some(buff) = k
                    .world
                    .unit(event.target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("titi:allySleep"))
                {
                    if buff.runtime.source == Some(id) {
                        event.prevented = true;
                    }
                    return Ok(());
                }
                if k.world.unit(event.target)?.deployment.kind
                    == crate::battle::deploy::Kind::Operator
                    && k.up()
                    && k.active()?
                    && body::in_range(k.world.unit(event.target)?, &k.keys()?)
                {
                    event.prevented = true;
                    self.sleeping(k, event.target, "titi:allySleep", true)?;
                }
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == ALLY_SLEEP => {
                let buff = k.buffs.get(k.world, context.unit, context.buff)?;
                if buff.runtime.source == Some(id)
                    && k.world.unit(context.unit)?.stats.hp >= k.stat(context.unit, "maxHp")? - 1e-6
                {
                    k.buffs
                        .remove_id(k.world, k.host, context.unit, context.buff)?;
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
            state.asleep = previous.asleep.clone();
        }
        state
    }
}
