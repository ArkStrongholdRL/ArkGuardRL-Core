// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared execution of typed operator components through canonical engine APIs.
use super::super::{ally, body, projectiles};
use super::super::{
    attacks::{self, Chain, Splash},
    content::operators::*,
    damage::{Damage, DamageType},
    direction::Direction,
    displacement, hit, injury,
    operators::Definition,
    search, skills,
    stats::{Buff, Numbers},
    targeting,
};
use super::tactics::{
    Arrow, ArrowKind, Task, Timer, ABNORMAL, DRIZZLE, GAP_FINISH, OBSERVE, RAIN, SHIELD_LOST,
    WITHER,
};
use super::*;
use crate::deterministic::math::powi;
use std::collections::{BTreeMap, BTreeSet};
pub(crate) mod agoat2;
pub(crate) mod archet;
pub(crate) mod blaze2;
pub(crate) mod blemsh;
pub(crate) mod blkkgt;
pub(crate) mod caper;
pub(crate) mod cello;
pub(crate) mod cetsyr;
pub(crate) mod clemnt;
pub(crate) mod demkni;
pub(crate) mod dispatch;
pub(crate) mod excu2;
pub(crate) mod funnel;
pub(crate) mod ghost2;
pub(crate) mod halo2;
pub(crate) mod horn;
pub(crate) mod indigo;
pub(crate) mod ines;
pub(crate) mod kjera;
pub(crate) mod lumen;
pub(crate) mod malist;
pub(crate) mod mlynar;
pub(crate) mod nymph;
pub(crate) mod papyrs;
pub(crate) mod pasngr;
pub(crate) mod pepe;
pub(crate) mod philae;
pub(crate) mod prepared_summon;
pub(crate) mod qiubai;
pub(crate) mod rmixer;
pub(crate) mod rockr;
pub(crate) mod silent;
pub(crate) mod snhunt;
pub(crate) mod sntlla;
pub(crate) mod texas2;
pub(crate) mod tinman;
pub(crate) mod tippi;
pub(crate) mod titi;
pub(crate) mod vulpis;

pub(crate) mod reed2;
pub(crate) mod rosmon;

pub(crate) mod sbell2;

pub(crate) mod skadi2;

pub(crate) mod svash2;

pub(crate) mod angel2;

pub(crate) mod thorn2;

pub(crate) mod yu;

pub(crate) mod dollkeeper;
pub(crate) mod dusk;

pub(crate) mod kazema;

pub(crate) mod lemuen;

pub(crate) mod nearl2;

pub(crate) mod siege2;

pub(crate) mod ulpia;

pub(crate) mod swire2;

pub(crate) mod whitw2;

pub(crate) mod etlchi;

pub(crate) mod wolf;

pub(crate) mod vigil;

pub(crate) mod manifold;

pub(crate) mod mlyss;

impl Host {
    pub(super) fn component_buff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        key: &str,
        mods: Numbers,
        duration: f64,
        source: Id,
    ) -> Result<(), String> {
        let mut b = Buff {
            key: Some(key.into()),
            mods,
            ..Default::default()
        };
        b.runtime.duration = duration;
        b.runtime.source = Some(source);
        r.add(
            w,
            self,
            id,
            buffs::Spec {
                buff: b,
                mods_present: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    fn component_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        amount: f64,
        kind: DamageType,
        tag: &str,
        ignore_dodge: bool,
    ) -> Result<f64, String> {
        let mut q = hit::Request::new(
            health::Hit {
                source: Some(source),
                credit: Some(source),
                target,
                damage_type: kind,
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
        q.is_skill = true;
        q.is_splash = tag == "sonic";
        q.tags.push(tag.into());
        if ignore_dodge {
            q.dodge_mode = injury::DodgeMode::Ignore;
            q.ignore_select = true;
        }
        self.resolve_hit(w, r, &mut q)
    }
    pub(super) fn component_debuff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        d: Debuff,
    ) -> Result<(), String> {
        if !w.unit(target)?.stats.alive || d.value == 0. {
            return Ok(());
        }
        if d.strongest {
            let value = -d.value;
            super::super::valued::apply(
                w,
                self,
                r,
                target,
                super::super::valued::Request {
                    key: d.key,
                    kind: super::super::valued::Kind::Custom(|v| {
                        [("resMul".into(), (1. - v).max(0.))].into()
                    }),
                    plain: true,
                    duration: d.duration,
                    value,
                    source: Some(source),
                    stack_as: None,
                },
            )?;
        } else {
            let mut b = Buff {
                key: Some(d.key.into()),
                mods: [(d.stat.into(), d.value)].into(),
                ..Default::default()
            };
            b.runtime.duration = d.duration;
            b.runtime.refresh = buffs::Refresh::Extend;
            b.runtime.source = Some(source);
            r.add(
                w,
                self,
                target,
                buffs::Spec {
                    buff: b,
                    mods_present: true,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    fn component_status(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        effect: OnHitStatus,
    ) -> Result<(), String> {
        if w.unit(target)?.stats.alive && effect.duration > 0. {
            self.apply_status(
                w,
                r,
                target,
                effect.key,
                super::super::status::Options {
                    duration: effect.duration,
                    source: Some(source),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    fn component_pull(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        dir: Direction,
        p: PullHit,
    ) -> Result<(), String> {
        let grid = self.grid.take().ok_or("Missing pull grid")?;
        let result = displacement::pull_to_front(
            w,
            &grid,
            &mut allies::Blocking {
                host: self,
                buffs: r,
            },
            target,
            source,
            dir,
            p.force,
        );
        self.grid = Some(grid);
        let moved = result?;
        if moved > 0. && p.travel_damage > 0. {
            self.component_damage(
                w,
                r,
                source,
                target,
                p.travel_damage * moved / p.distance,
                DamageType::Arts,
                "drag",
                true,
            )?;
        }
        Ok(())
    }
    pub(super) fn component_skill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        dir: Direction,
        def: &Definition,
        event: skills::Event,
        c: &skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        let id = s.unit();
        let fx = &def.extra.effects;
        if event == skills::Event::Start {
            w.unit_mut(id)?.operator.active_skill = true;
            w.unit_mut(id)?.operator.hits = 0;
            if s.bank.kind == super::super::sp::Kind::Ammo {
                let mut extra = 0.;
                if let Some(a) = &fx.ammo {
                    if w.time - w.unit(id)?.deployment.deployed_at + 1e-9 >= a.delay {
                        extra += a.own;
                    }
                }
                let grants = w.unit(id)?.operator.ammo_grants.clone();
                for (source, (seq, n)) in grants {
                    let valid = w.unit(source).is_ok_and(|u| {
                        u.stats.alive && u.deployed && u.deployment.deploy_seq == seq
                    });
                    if valid {
                        extra += n;
                    } else {
                        w.unit_mut(id)?.operator.ammo_grants.remove(&source);
                    }
                }
                s.add_ammo(extra);
            }
            if let Some(steps) = &fx.growing_atk {
                let uses = w.unit(id)?.operator.uses;
                let amount = steps
                    .get(uses.min(steps.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(0.);
                w.unit_mut(id)?.operator.uses = uses.saturating_add(1).min(steps.len());
                self.component_buff(
                    w,
                    r,
                    id,
                    "skill:pinecn_atk",
                    [("atkPct".into(), amount)].into(),
                    f64::INFINITY,
                    id,
                )?;
            }
            if let Some(counter) = &fx.skill_counter {
                let mut b = Buff {
                    key: Some(counter.key.into()),
                    ..Default::default()
                };
                b.runtime.injury.observe = Some(SKILL_COUNTER);
                r.add(
                    w,
                    self,
                    id,
                    buffs::Spec {
                        buff: b,
                        ..Default::default()
                    },
                )?;
            }
            if fx.resist > 0. {
                self.apply_status(
                    w,
                    r,
                    id,
                    "resist",
                    super::super::status::Options {
                        duration: s.time_left,
                        value: Some(fx.resist),
                        source: Some(id),
                        ..Default::default()
                    },
                )?;
            }
            if let Some((grid, effect)) = &fx.start_status {
                let keys = targeting::absolute_range(grid, w.unit(id)?.deployment.tile, dir, 0);
                let targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    targeting::Profile {
                        can_hit_fly: false,
                        ground_only: true,
                        ..Default::default()
                    },
                    w,
                    w.unit_capacity(),
                )?;
                // Every eligible target receives the status. This can run during
                // an enemy attack, while the movement phase owns the route grid.
                for target in targets {
                    self.component_status(w, r, id, target, *effect)?;
                }
            }
            if let Some(burst) = &fx.burst {
                let keys =
                    targeting::absolute_range(&burst.grid, w.unit(id)?.deployment.tile, dir, 0);
                let mut targets = search::Host::enemies(
                    self,
                    id,
                    &keys,
                    targeting::Profile {
                        can_hit_fly: !burst.ground,
                        ..Default::default()
                    },
                    w,
                    w.unit_capacity(),
                )?;
                targeting::sort_enemies(w, self, id, &mut targets, def.combat.profile.priority)?;
                targets.truncate(burst.count);
                for target in targets {
                    if let Some(pull) = burst.pull {
                        self.component_pull(w, r, id, target, dir, pull)?;
                    }
                    if !w.unit(target)?.stats.alive {
                        continue;
                    }
                    let scale = if targeting::flying(w, target)? {
                        burst.fly_scale
                    } else {
                        1.
                    };
                    let amount = w
                        .unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .n("atk")
                        * burst.scale
                        * scale;
                    for _ in 0..def.extra.advanced_burst_hits.max(1) {
                        if !w.unit(target)?.stats.alive {
                            break;
                        }
                        self.component_damage(
                            w,
                            r,
                            id,
                            target,
                            amount,
                            burst.damage,
                            "burst",
                            false,
                        )?;
                    }
                    if let Some(effect) = burst.status {
                        self.component_status(w, r, id, target, effect)?;
                    }
                    if let Some(debuff) = burst.debuff {
                        self.component_debuff(w, r, id, target, debuff)?;
                    }
                }
            }
        }
        if event == skills::Event::End {
            w.unit_mut(id)?.operator.active_skill = false;
            if let Some(counter) = &fx.skill_counter {
                r.remove(w, self, id, counter.key)?;
            }
            if fx.growing_atk.is_some() {
                r.remove(w, self, id, "skill:pinecn_atk")?;
            }
            if c.reason != "death" && w.unit(id)?.stats.alive && fx.end_heal > 0. {
                let amount = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("maxHp")
                    * fx.end_heal;
                health::heal(
                    w,
                    self,
                    Some(id),
                    id,
                    amount,
                    health::HealOptions {
                        self_heal: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn component_scheduled(&mut self, w: &mut World) -> Result<(), String> {
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut r = self
            .buffs
            .take()
            .ok_or("Missing component scheduler buffs")?;
        let result = (|| {
            for &id in &ids {
                let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
                let fx = &def.extra.effects;
                let up = w.unit(id)?.stats.alive && w.unit(id)?.deployed;
                if w.unit(id)?.operator.blessing_pending {
                    w.unit_mut(id)?.operator.blessing_pending = false;
                    if up {
                        let candidates: Vec<_> = ids
                            .iter()
                            .copied()
                            .filter(|&other| {
                                other != id
                                    && targeting::aura_selectable(w, other).unwrap_or(false)
                                    && !w.unit(other).unwrap().stats.buffs().iter().any(|b| {
                                        b.key.as_deref() == Some("talent:angel_bless_ally")
                                    })
                            })
                            .collect();
                        if let Some(&other) = self.rng.pick(&candidates) {
                            let mut b = persistent(
                                "talent:angel_bless_ally",
                                fx.random_bless.as_ref().unwrap().clone(),
                            );
                            b.runtime.source = Some(id);
                            r.add(
                                w,
                                self,
                                other,
                                buffs::Spec {
                                    buff: b,
                                    mods_present: true,
                                    ..Default::default()
                                },
                            )?;
                        }
                    }
                }
                if let Some(a) = fx.ammo.as_ref().filter(|a| a.ally > 0.) {
                    if up
                        && !w.unit(id)?.operator.ammo_granted
                        && w.time + 1e-9 >= w.unit(id)?.deployment.deployed_at + a.delay
                    {
                        w.unit_mut(id)?.operator.ammo_granted = true;
                        let mut candidates = vec![];
                        for &other in &ids {
                            let entry = self.roster.as_ref().unwrap().entry(other)?;
                            if other != id
                                && w.unit(other)?.stats.alive
                                && entry.definition.laterano
                                && entry.skill()?.bank.kind == super::super::sp::Kind::Ammo
                            {
                                candidates.push(other);
                            }
                        }
                        if let Some(&other) = self.rng.pick(&candidates) {
                            let seq = w.unit(id)?.deployment.deploy_seq;
                            w.unit_mut(other)?
                                .operator
                                .ammo_grants
                                .insert(id, (seq, a.ally));
                        }
                    }
                }
                if fx.aura.is_none() && fx.enemy_atk.is_none() {
                    continue;
                }
                if w.time + 1e-9 < w.unit(id)?.operator.next_pulse {
                    continue;
                }
                let interval = if fx.enemy_atk.is_some() { 0.1 } else { 0.2 };
                w.unit_mut(id)?.operator.next_pulse += interval;
                if !up {
                    continue;
                }
                let keys = search::Host::range(self, id, w)?;
                if let Some(aura) = &fx.aura {
                    let dir = self.roster.as_ref().unwrap().entry(id)?.direction;
                    let range = aura.grid.as_ref().map(|g| {
                        targeting::absolute_range(g, w.unit(id).unwrap().deployment.tile, dir, 0)
                    });
                    for &other in &ids {
                        let entry = self.roster.as_ref().unwrap().entry(other)?;
                        // Friendly auras still reach stealthed and sleeping allies.
                        let selectable = if aura.grid.is_none() {
                            Self::tac_up(w, other)
                        } else {
                            targeting::friendly_selectable(w, id, other)?
                        };
                        let eligible = selectable
                            && aura
                                .profession
                                .is_none_or(|p| entry.definition.profession == p)
                            && range.as_ref().is_none_or(|g| {
                                super::super::body::in_range(w.unit(other).unwrap(), g)
                            });
                        if eligible {
                            self.component_buff(
                                w,
                                &mut r,
                                other,
                                aura.key,
                                aura.mods.clone(),
                                if aura.grid.is_some() {
                                    0.4 + 0.05
                                } else {
                                    0.25
                                },
                                id,
                            )?;
                        } else if aura.grid.is_some()
                            && w.unit(other)?.stats.buffs().iter().any(|b| {
                                b.key.as_deref() == Some(aura.key) && b.runtime.source == Some(id)
                            })
                        {
                            r.remove(w, self, other, aura.key)?;
                        }
                    }
                }
                if let Some(bonus) = &fx.enemy_atk {
                    let n = search::Host::enemies(
                        self,
                        id,
                        &keys,
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        },
                        w,
                        w.unit_capacity(),
                    )?
                    .len() as f64;
                    let value = n.min(bonus.cap) * bonus.value;
                    if value > 0.
                        && !w.unit(id)?.stats.buffs().iter().any(|b| {
                            b.key.as_deref() == Some("lionhd:t1")
                                && b.mods.get("atkPct") == Some(&value)
                        })
                    {
                        self.component_buff(
                            w,
                            &mut r,
                            id,
                            "lionhd:t1",
                            [("atkPct".into(), value)].into(),
                            f64::INFINITY,
                            id,
                        )?;
                    }
                    if value <= 0. {
                        r.remove(w, self, id, "lionhd:t1")?;
                    }
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn refresh_component_conditions(&mut self, w: &mut World) -> Result<(), String> {
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut r = self.buffs.take().ok_or("Missing component tick buffs")?;
        let result = (|| {
            for id in ids {
                let def = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
                if let Some(condition) = &def.extra.effects.condition {
                    let u = w.unit(id)?;
                    let up = u.stats.alive && u.deployed;
                    let want = up
                        && match condition.when {
                            Predicate::Unblocked => u.blocking().is_empty(),
                            Predicate::AllyInRange => {
                                let keys = search::Host::range(self, id, w)?;
                                let allies = self.roster.as_ref().unwrap().ids();
                                allies.iter().any(|&other| {
                                    other != id
                                        && targeting::aura_selectable(w, other).unwrap_or(false)
                                        && super::super::body::in_range(
                                            w.unit(other).unwrap(),
                                            &keys,
                                        )
                                })
                            }
                        };
                    let has = w
                        .unit(id)?
                        .stats
                        .buffs()
                        .iter()
                        .any(|b| b.key.as_deref() == Some(condition.key));
                    if want && !has {
                        self.component_buff(
                            w,
                            &mut r,
                            id,
                            condition.key,
                            condition.mods.clone(),
                            f64::INFINITY,
                            id,
                        )?;
                    }
                    if !want && has {
                        r.remove(w, self, id, condition.key)?;
                    }
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn component_observe(
        c: &mut settlement::Context<'_, Self>,
        token: u64,
    ) -> Result<(), String> {
        if !matches!(token, INJURY_OBSERVER | SKILL_COUNTER) {
            return Err("Unknown operator observer callback".into());
        }
        let target = c.request.hit.target;
        let Some(source) = c.request.hit.source else {
            return Ok(());
        };
        if !c.world.unit(target)?.stats.alive
            || c.world.unit(source)?.side != health::Side::Enemy
            || c.request.hit.hp_loss
            || c.request.hit.damage_type == DamageType::Elemental
        {
            return Ok(());
        }
        let def = Arc::clone(&c.host.roster.as_ref().unwrap().entry(target)?.definition);
        let fx = &def.extra.effects;
        let counter = if token == SKILL_COUNTER {
            fx.skill_counter
        } else {
            fx.observer.as_ref().and_then(|o| o.counter)
        };
        // Talent observers are installed before the skill counter, preserving injury snapshot order.
        if let Some(d) = fx
            .observer
            .as_ref()
            .and_then(|o| o.debuff)
            .filter(|_| token != SKILL_COUNTER)
        {
            if targeting::can_target_enemy(
                c.world,
                c.host,
                Some(target),
                source,
                def.combat.profile.selection,
            )? {
                c.host
                    .component_debuff(c.world, c.runtime, target, source, d)?;
            }
        }
        if let Some(counter) = counter {
            if c.world.unit(source)?.stats.alive {
                let amount = c
                    .world
                    .unit_mut(target)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n(counter.stat)
                    * counter.scale;
                let mut q = hit::Request::new(
                    health::Hit {
                        source: Some(target),
                        credit: Some(target),
                        target: source,
                        damage_type: DamageType::Physical,
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
                q.dodge_mode = injury::DodgeMode::Ignore;
                q.ignore_select = true;
                q.is_skill = true;
                q.tags.push("counter".into());
                c.resolve(&mut q)?;
            }
        }
        Ok(())
    }
}
impl Host {
    pub(super) fn component_phase(
        &mut self,
        phase: hit::Phase,
        q: &mut hit::Request,
        _: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if phase != hit::Phase::Hit {
            return Ok(());
        }
        if w.unit(q.hit.target)?.side == health::Side::Ally
            && w.unit(q.hit.target)?.deployment.kind != crate::battle::deploy::Kind::Device
        {
            let def = Arc::clone(
                &self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(q.hit.target)?
                    .definition,
            );
            if q.hit.source.is_some_and(|s| {
                w.unit(s)
                    .is_ok_and(|u| u.blocked_by() == Some(q.hit.target))
            }) {
                q.damage.mul *= def.extra.effects.blocked_taken;
            }
        }
        let Some(source) = q
            .hit
            .source
            .filter(|&s| w.unit(s).is_ok_and(|u| u.has_operator_kit()))
        else {
            return Ok(());
        };
        if !q.is_attack || w.unit(q.hit.target)?.side != health::Side::Enemy {
            return Ok(());
        }
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
        let fx = &def.extra.effects;
        if targeting::flying(w, q.hit.target)? {
            q.damage.mul *= fx.fly_scale;
        }
        if let Some(d) = &fx.distance {
            let a = w.unit(source)?.position;
            let b = w.unit(q.hit.target)?.position;
            let distance = crate::deterministic::math::hypot(a[0] - b[0], a[1] - b[1]);
            let fraction = if d.maximum > d.minimum {
                ((distance - d.minimum) / (d.maximum - d.minimum)).clamp(0., 1.)
            } else {
                1.
            };
            q.damage.amount *= 1. + d.attack_scale * fraction;
            q.damage.mul *= 1. + d.damage_scale * fraction;
        }
        if let Some(boost) = &fx.attack_boost {
            if w.unit(source)?.operator.boosted {
                q.damage.mul *= boost.scale;
            }
        }
        if let Some(crit) = &fx.crit {
            if rng.chance(crit.chance) {
                q.damage.amount *= crit.attack_scale;
                q.damage.mul *= crit.damage_scale;
                q.tags.push("kroos2Crit".into());
            }
        }
        if q.is_skill {
            q.damage.penetration.def_flat += fx.skill_def_ignore;
        }
        Ok(())
    }
    pub(super) fn component_landed(
        &mut self,
        e: &health::Event,
        q: &hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let Some(source) = q
            .hit
            .source
            .filter(|&s| w.unit(s).is_ok_and(|u| u.has_operator_kit()))
        else {
            return Ok(());
        };
        if !q.is_attack || w.unit(e.target)?.side != health::Side::Enemy {
            return Ok(());
        }
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
        let fx = &def.extra.effects;
        if fx.hit_upgrade.is_some() && w.unit(source)?.operator.active_skill {
            let u = w.unit_mut(source)?;
            u.operator.hits = u.operator.hits.saturating_add(1);
        }
        if q.tags.iter().any(|t| t == "kroos2Crit") {
            if let Some(crit) = &fx.crit {
                self.component_status(
                    w,
                    r,
                    source,
                    e.target,
                    OnHitStatus {
                        key: "stun",
                        duration: crit.stun,
                        chance: None,
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn component_fatal(
        &mut self,
        e: &mut health::Event,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if e.stage != health::Stage::Fatal
            || e.prevented
            || w.unit(e.target)?.side != health::Side::Ally
            || w.unit(e.target)?.operator.revived
        {
            return Ok(());
        }
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(e.target)?.definition);
        if let Some(revive) = &def.extra.effects.revive {
            w.unit_mut(e.target)?.operator.revived = true;
            e.prevented = true;
            self.component_buff(
                w,
                r,
                e.target,
                "trait:skadi_tide",
                [
                    ("hpMul".into(), revive.hp_mul),
                    ("aspd".into(), revive.aspd),
                ]
                .into(),
                f64::INFINITY,
                e.target,
            )?;
            let u = w.unit_mut(e.target)?;
            u.stats.hp =
                (u.stats.stats().map_err(str::to_string)?.n("maxHp") * revive.hp_ratio).max(1.);
        }
        Ok(())
    }
    pub(super) fn component_before(
        &mut self,
        c: &mut attacks::Context,
        w: &mut World,
    ) -> Result<(), String> {
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(c.attacker)?.definition);
        if let Some(boost) = &def.extra.effects.attack_boost {
            let boosted = self.rng.chance(boost.chance);
            w.unit_mut(c.attacker)?.operator.boosted = boosted;
            if boosted {
                let keys = search::Host::range(self, c.attacker, w)?;
                let mut candidates = search::Host::enemies(
                    self,
                    c.attacker,
                    &keys,
                    def.combat.profile.selection,
                    w,
                    w.unit_capacity(),
                )?;
                candidates.retain(|t| !c.targets.contains(t));
                targeting::sort_enemies(
                    w,
                    self,
                    c.attacker,
                    &mut candidates,
                    def.combat.profile.priority,
                )?;
                if let Some(&other) = candidates.first() {
                    c.targets.push(other);
                }
            }
        }
        Ok(())
    }
    pub(super) fn component_hit(
        &mut self,
        impact: attacks::Impact,
        w: &mut World,
    ) -> Result<(), String> {
        if impact.kind != attacks::VictimKind::Main || !impact.is_skill {
            return Ok(());
        }
        let Some(target) = impact
            .target
            .filter(|&t| w.unit(t).is_ok_and(|u| u.stats.alive))
        else {
            return Ok(());
        };
        let source = impact.attacker;
        let entry = self.roster.as_ref().unwrap().entry(source)?;
        let dir = entry.direction;
        let def = Arc::clone(&entry.definition);
        let fx = &def.extra.effects;
        let mut r = self
            .buffs
            .take()
            .ok_or("Missing operator hit buff context")?;
        let result = (|| {
            if let Some(d) = fx.skill_debuff {
                self.component_debuff(w, &mut r, source, target, d)?;
            }
            if let Some(p) = fx.skill_pull {
                if !def.extra.support.pull_unblocked || w.unit(target)?.blocked_by().is_none() {
                    self.component_pull(w, &mut r, source, target, dir, p)?;
                }
            }
            if let Some(force) = fx.skill_push {
                let forward = dir.vector().map(|n| n as f64);
                let a = w.unit(source)?.position;
                let b = w.unit(target)?.position;
                let behind = (b[0] - a[0]) * forward[0] + (b[1] - a[1]) * forward[1] < 0.;
                let options = displacement::Push {
                    from: Some(displacement::Origin::unit(w, source, dir)?),
                    direction: Some(forward),
                    effect: behind || def.extra.support.effect_push,
                    ..Default::default()
                };
                let grid = self.grid.take().ok_or("Missing push grid")?;
                let result = displacement::push(
                    w,
                    &grid,
                    &mut allies::Blocking {
                        host: self,
                        buffs: &mut r,
                    },
                    target,
                    force,
                    options,
                );
                self.grid = Some(grid);
                result?;
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn component_attack(
        &mut self,
        c: &attacks::Context,
        w: &mut World,
    ) -> Result<(), String> {
        let source = c.attacker;
        let def = Arc::clone(&self.roster.as_ref().unwrap().entry(source)?.definition);
        w.unit_mut(source)?.operator.boosted = false;
        let active = w.unit(source)?.operator.active_skill;
        let Some(secondary) = def.extra.effects.secondary.as_ref().filter(|_| active) else {
            return Ok(());
        };
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        let mut r = self
            .buffs
            .take()
            .ok_or("Missing secondary attack buff context")?;
        let result = (|| {
            match secondary {
                Secondary::Sonic { radius, scale } => {
                    let keys = search::Host::range(self, source, w)?;
                    for id in ids {
                        if id == source
                            || !targeting::aura_selectable(w, id)?
                            || !super::super::body::in_range(w.unit(id)?, &keys)
                        {
                            continue;
                        }
                        let targets = attacks::Hooks::radius_targets(
                            self,
                            w.unit(id)?.position,
                            *radius,
                            false,
                            w,
                            w.unit_capacity(),
                        )?;
                        for target in targets {
                            if w.unit_mut(target)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .flags
                                .contains("untargetable")
                            {
                                continue;
                            }
                            let amount = w
                                .unit_mut(source)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("atk")
                                * scale;
                            self.component_damage(
                                w,
                                &mut r,
                                source,
                                target,
                                amount,
                                DamageType::Arts,
                                "sonic",
                                false,
                            )?;
                        }
                    }
                }
                Secondary::NearbyBlocked { scale } => {
                    let home = w.unit(source)?.deployment.tile;
                    let mut seen = std::collections::BTreeSet::new();
                    for id in ids {
                        let u = w.unit(id)?;
                        let tile = u.deployment.tile;
                        if !u.stats.alive
                            || !u.deployed
                            || (tile[0] - home[0]).abs() > 1
                            || (tile[1] - home[1]).abs() > 1
                        {
                            continue;
                        }
                        for target in u.blocking().to_vec() {
                            if !w.unit(target)?.stats.alive || !seen.insert(target) {
                                continue;
                            }
                            let amount = w
                                .unit_mut(source)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("atk")
                                * scale;
                            self.component_damage(
                                w,
                                &mut r,
                                source,
                                target,
                                amount,
                                DamageType::Arts,
                                "ayerBlade",
                                false,
                            )?;
                        }
                    }
                }
            }
            Ok(())
        })();
        self.buffs = Some(r);
        result
    }
}

pub(crate) fn kit_text_count(
    text: &Value,
    prefix: &str,
    suffix: &str,
    fallback: u32,
    limit: u32,
) -> Result<u32, String> {
    let text = text.as_str().unwrap_or("");
    let after = if prefix.is_empty() {
        text
    } else {
        text.split_once(prefix).map_or("", |(_, after)| after)
    };
    let before = after.split_once(suffix).map_or("", |(before, _)| before);
    let digits: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let count = digits.parse::<u32>().unwrap_or(fallback);
    if count == 0 || count > limit {
        return Err("Kit text count exceeds supported payload capacity".into());
    }
    Ok(count)
}

pub(crate) mod yanyou;
