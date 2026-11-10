// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{deploy, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "longrange",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_lemuen_3",
        kind: Kind::Ammo,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 5.).floor().max(1.);
    let mut profile = s.combat.profile;
    profile.no_attack = true;
    s.combat.skill_profile = Some(profile);
    s.combat.profile.priority = targeting::Priority::LowDef;
    s.extra.tactics = Tactics::Lemuen(Lemuen {
        aim: n(bb, "attack@aim_interval", 0.5).max(0.05),
        inner: n(bb, "attack@dist_1", 0.8),
        outer: n(bb, "attack@dist_2", 1.5),
        close: n(bb, "attack@proj_atk_scale_1", 1.),
        far: n(bb, "attack@proj_atk_scale_2", 1.),
        spread: n(bb, "attack@emit_offset", 0.2).max(0.),
        wanted_interval: n(t, "interval", 8.),
        wanted_scale: n(t, "damage_scale", 1.),
        extradition: n(t1, "interval", 20.),
        attack: n(t1, "atk", 0.),
        ammo: n(t1, "add_count", 0.),
        other_ammo: n(t1, "ex_add_count", 0.),
        sp: n(&raw["trait"]["bb"], "sp", 0.),
    });
    Ok(())
}
pub(crate) struct Lemuen {
    aim: f64,
    inner: f64,
    outer: f64,
    close: f64,
    far: f64,
    spread: f64,
    wanted_interval: f64,
    wanted_scale: f64,
    extradition: f64,
    attack: f64,
    ammo: f64,
    other_ammo: f64,
    sp: f64,
}
#[derive(Clone)]
struct Lock {
    target: Id,
    position: [f64; 2],
    gone: bool,
}
struct Volley {
    locks: Vec<Lock>,
    attack: f64,
    next: usize,
}
#[derive(Default)]
pub(crate) struct State {
    accumulator: f64,
    locks: Vec<Lock>,
    times: BTreeMap<Id, f64>,
    marks: BTreeSet<Id>,
    volleys: BTreeMap<u64, Volley>,
    sequence: u64,
}
impl Lock {
    fn update(&mut self, w: &World) {
        if let Ok(e) = w.unit(self.target) {
            if !self.gone && (!e.stats.alive || !e.hidden) {
                self.position = e.position;
                self.gone = !e.stats.alive;
            }
        }
    }
}
impl Lemuen {
    fn extradition(k: &mut KitContext<'_>) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Lemuen(p) = &def.extra.tactics else {
            return Err("Missing Lemuen kit".into());
        };
        if k.up() && p.attack != 0. {
            k.buff(
                k.unit,
                "lemuen:extradition",
                [("atkPct".into(), p.attack)].into(),
                f64::INFINITY,
            )?;
        }
        Ok(())
    }
    fn blast(k: &mut KitContext<'_>, position: [f64; 2], attack: f64) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Lemuen(p) = &def.extra.tactics else {
            return Err("Missing Lemuen kit".into());
        };
        for target in attacks::Hooks::radius_targets(
            k.host,
            position,
            p.outer,
            true,
            k.world,
            k.world.unit_capacity(),
        )? {
            if k.flag(target, "untargetable")? {
                continue;
            }
            let amount = attack
                * if body::distance(k.world.unit(target)?, position) <= p.inner + 1e-9 {
                    p.close
                } else {
                    p.far
                };
            let mut request = hit::Request::new(
                health::Hit {
                    source: Some(k.unit),
                    credit: Some(k.unit),
                    target,
                    damage_type: DamageType::Physical,
                    no_sp: false,
                    hp_loss: false,
                    silent: false,
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
            request.tags = ["skill".into(), "bombard".into()].into();
            k.deal(&mut request)?;
        }
        Ok(())
    }
    fn fire(k: &mut KitContext<'_>, key: u64) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Lemuen(p) = &def.extra.tactics else {
            return Err("Missing Lemuen kit".into());
        };
        let mut volley = match k
            .world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .lemuen_mut()?
            .volleys
            .remove(&key)
        {
            Some(v) => v,
            None => return Ok(()),
        };
        let lock = &mut volley.locks[volley.next];
        lock.update(k.world);
        let local = [
            (k.host.rng.next_f64() * 2. - 1.) * p.spread,
            (k.host.rng.next_f64() * 2. - 1.) * p.spread,
        ];
        let dir = k.host.roster.as_ref().unwrap().entry(k.unit)?.direction;
        let offset = match dir {
            Direction::Right => local,
            Direction::Left => [-local[0], -local[1]],
            Direction::Up => [local[1], -local[0]],
            Direction::Down => [-local[1], local[0]],
        };
        k.later_at(
            0.3,
            [lock.position[0] + offset[0], lock.position[1] + offset[1]],
            volley.attack,
            true,
            Self::blast,
        )?;
        volley.next += 1;
        if volley.next < volley.locks.len().min(33) {
            k.world
                .unit_mut(k.unit)?
                .operator
                .tactical
                .lemuen_mut()?
                .volleys
                .insert(key, volley);
            k.later_key(0.3, key, Self::fire)?;
        }
        Ok(())
    }
    fn reconcile(k: &mut KitContext<'_>) -> Result<(), String> {
        let mut marks = BTreeSet::new();
        for id in k.host.roster.as_ref().unwrap().ids() {
            if Host::tac_up(k.world, *id) {
                if let Ok(state) = k.world.unit(*id)?.operator.tactical.lemuen() {
                    marks.extend(state.marks.iter().copied());
                }
            }
        }
        let enemies = k
            .world
            .unit_ids()
            .filter(|&id| {
                k.world
                    .unit(id)
                    .is_ok_and(|u| u.side == health::Side::Enemy)
            })
            .collect::<Vec<_>>();
        for enemy in enemies {
            if marks.contains(&enemy) && Host::tac_up(k.world, enemy) {
                if !k.has(enemy, "lemuen:wanted") {
                    k.buff(enemy, "lemuen:wanted", Numbers::new(), f64::INFINITY)?;
                }
            } else {
                k.remove(enemy, "lemuen:wanted")?;
            }
        }
        for id in k.host.roster.as_ref().unwrap().ids().to_vec() {
            if k.world.unit(id)?.operator.tactical.lemuen().is_ok() {
                let mut keys = Vec::new();
                if Host::tac_up(k.world, id) {
                    for &enemy in &marks {
                        // Marks are pruned on the quarter-second scan. A killed
                        // enemy may already have been reclaimed between scans.
                        let Ok(u) = k.world.unit(enemy) else { continue };
                        if Host::tac_up(k.world, enemy) && !u.hidden {
                            for key in body::keys(u) {
                                if !keys.contains(&key) {
                                    keys.push(key);
                                }
                            }
                        }
                    }
                }
                k.world.unit_mut(id)?.operator.advanced.extra_range = keys;
            }
        }
        Ok(())
    }
}
impl Behavior for Lemuen {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                k.later(self.extradition, Self::extradition)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                state.locks.clear();
                state.accumulator = self.aim;
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::SkillStart,
                skill,
            } if k.up()
                && skill.bank.kind == Kind::Ammo
                && k.world.time - k.world.unit(id)?.deployment.deployed_at
                    >= self.extradition - 1e-6 =>
            {
                if *unit == id {
                    skill.add_ammo(self.ammo);
                } else if k.faction(*unit, "lateranoShip", &[])? {
                    skill.add_ammo(self.other_ammo);
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                skill,
            } if *unit == id && skill.bank.active => {
                let mut locks = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?.locks,
                );
                for lock in &mut locks {
                    lock.update(k.world);
                }
                k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?.locks = locks;
                if skills::can_act(k.world, id)? {
                    let accumulator =
                        k.world.unit(id)?.operator.tactical.lemuen()?.accumulator + context.dt;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .lemuen_mut()?
                        .accumulator = accumulator;
                    if accumulator + 1e-9 >= self.aim {
                        let mut candidates = k.enemies(None, true)?;
                        targeting::sort_enemies(
                            k.world,
                            k.host,
                            id,
                            &mut candidates,
                            targeting::Priority::LowDef,
                        )?;
                        let locks = &k.world.unit(id)?.operator.tactical.lemuen()?.locks;
                        let target = candidates.into_iter().min_by_key(|target| {
                            locks.iter().filter(|lock| lock.target == *target).count()
                        });
                        if let Some(target) = target {
                            let position = k.world.unit(target)?.position;
                            let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                            state.accumulator -= self.aim;
                            if state.locks.len() >= 64 {
                                return Err("Lemuen lock capacity exceeded".into());
                            }
                            state.locks.push(Lock {
                                target,
                                position,
                                gone: false,
                            });
                            k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                                skill.consume_ammo(world, host)
                            })?;
                        } else {
                            k.world
                                .unit_mut(id)?
                                .operator
                                .tactical
                                .lemuen_mut()?
                                .accumulator = self.aim;
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id => {
                let locks = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?.locks,
                );
                if context.reason != "death" && k.up() && !locks.is_empty() {
                    let attack = k.stat(id, "atk")?;
                    let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                    if state.volleys.len() >= k.host.tactical_limit {
                        return Err("Lemuen volley capacity exceeded".into());
                    }
                    state.sequence = state
                        .sequence
                        .checked_add(1)
                        .ok_or("Lemuen volley sequence exhausted")?;
                    let key = state.sequence;
                    state.volleys.insert(
                        key,
                        Volley {
                            locks,
                            attack,
                            next: 0,
                        },
                    );
                    Self::fire(k, key)?;
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                let mut ranges = Vec::new();
                for ally in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    if Host::tac_up(k.world, ally)
                        && k.world.unit(ally)?.deployment.kind == deploy::Kind::Operator
                        && k.faction(ally, "lateranoShip", &[])?
                    {
                        ranges.push(k.host.roster.as_ref().unwrap().range(k.world, ally)?);
                    }
                }
                let valid = k
                    .world
                    .unit_ids()
                    .filter(|&e| {
                        k.world
                            .unit(e)
                            .is_ok_and(|u| u.side == health::Side::Enemy && u.stats.alive)
                    })
                    .collect::<BTreeSet<_>>();
                {
                    let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                    state.times.retain(|e, _| valid.contains(e));
                    state.marks.retain(|e| valid.contains(e));
                }
                for enemy in valid {
                    let u = k.world.unit(enemy)?;
                    if !(u.boss || u.targeting.elite)
                        || k.world
                            .unit(id)?
                            .operator
                            .tactical
                            .lemuen()?
                            .marks
                            .contains(&enemy)
                    {
                        continue;
                    }
                    if u.hidden || !ranges.iter().any(|keys| body::in_range(u, keys)) {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .lemuen_mut()?
                            .times
                            .remove(&enemy);
                        continue;
                    }
                    if !k
                        .world
                        .unit(id)?
                        .operator
                        .tactical
                        .lemuen()?
                        .times
                        .contains_key(&enemy)
                        && k.host.tac_selectable(k.world, id, vec![enemy])?.is_empty()
                    {
                        continue;
                    }
                    let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                    let time = state.times.entry(enemy).or_default();
                    *time += 0.25;
                    if *time >= self.wanted_interval - 1e-9 {
                        state.marks.insert(enemy);
                    }
                }
                Self::reconcile(k)?;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if k.up()
                && k.has(request.hit.target, "lemuen:wanted")
                && (request.is_attack || request.is_skill) =>
            {
                if let Some(source) = request.hit.source {
                    if k.faction(source, "lateranoShip", &[])? {
                        let strongest = k
                            .host
                            .roster
                            .as_ref()
                            .unwrap()
                            .ids()
                            .iter()
                            .copied()
                            .filter(|&a| Host::tac_up(k.world, a))
                            .filter_map(|a| match &k.host.tac_def(a).ok()?.extra.tactics {
                                Tactics::Lemuen(p) => Some((a, p.wanted_scale)),
                                _ => None,
                            })
                            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
                        if strongest.is_some_and(|(source, _)| source == id) {
                            request.damage.mul *= self.wanted_scale;
                        }
                    }
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && self.sp > 0.
                    && impact.target.is_some_and(|a| Host::tac_up(k.world, a))
                    && !k.active()? =>
            {
                let amount = self.sp;
                k.skill(|skill, host, w| skill.gain(w, host, amount, "trait", false, false))?;
            }
            KitEvent::Death { unit, .. } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.lemuen_mut()?;
                state.times.clear();
                state.marks.clear();
                Self::reconcile(k)?;
            }
            KitEvent::Tick => Self::reconcile(k)?,
            _ => {}
        }
        Ok(())
    }
}
