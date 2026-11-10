// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "lord",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_qiubai_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_qiubai_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_qiubai_2",
            kind: Kind::Duration,
            sp_type: SpType::Attack,
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
    let mode = tactical_mode(raw)?;

    s.extra.lord = Some(Lord {
        scale: n(tb, "atk_scale", 0.8),
        arts: 0.,
        skill_full: mode == Mode::Third,
    });
    skill_range(raw, s)?;
    if mode != Mode::First {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    }
    if mode == Mode::Third {
        s.extra.skill_arts = true;
        let mut p = s.combat.profile;
        p.max_targets = 3.;
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Qiubai(Qiubai {
        mode,
        gap: n(t, "atk_scale_t", n(t, "atk_scale", 0.)),
        both: if t["atk_scale_t"].is_null() {
            1.
        } else {
            n(t, "value", 1.)
        },
        module: n(tb, "atk_scale_m", 0.),
        first: n(t1, "duration_advanced", 0.),
        probability: n(t1, "prob", 0.),
        bind: n(t1, "duration", 0.),
        skill_bind: n(bb, "duration", 2.),
        aoe: n(bb, "aoe_scale", 1.),
        begin: n(bb, "sword_begin_atk_scale", 1.),
        finish: n(bb, "sword_end_atk_scale", 1.),
        aspd: n(bb, "attack_speed", 0.),
        stacks: n(bb, "max_stack_cnt", 0.).max(0.) as u32,
        crowd: n(t, "cnt", 0.).max(0.) as usize,
        crowd_aspd: n(t, "attack_speed", 0.),
        gap_key: if t["atk_scale_t"].is_null() {
            "qiubai_talent_1[damage]"
        } else {
            "qiubai_e_002_t[damage]"
        },
    });
    Ok(())
}

pub(crate) struct Qiubai {
    pub mode: Mode,
    pub gap: f64,
    pub both: f64,
    pub module: f64,
    pub first: f64,
    pub probability: f64,
    pub bind: f64,
    pub skill_bind: f64,
    pub aoe: f64,
    pub begin: f64,
    pub finish: f64,
    pub aspd: f64,
    pub stacks: u32,
    pub crowd: usize,
    pub crowd_aspd: f64,
    pub gap_key: &'static str,
}

#[derive(Default)]
pub(crate) struct State {
    pub stacks: u32,
    pub hit: std::collections::BTreeMap<Id, u64>,
}

impl Host {
    pub(in crate::battle::runtime) fn qiubai_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Qiubai,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.qiubai_mut()?.stacks = 0;
        if p.mode == Mode::Second {
            for a in self.tac_enemies(w, id, None, false)? {
                let amount = Self::tac_stat(w, id, "atk")? * p.begin;
                self.tac_damage(w, r, id, a, amount, DamageType::Arts, false, "sword")?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_end(
        &mut self,
        c: &skills::Context,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Qiubai,
    ) -> Result<(), String> {
        r.remove(w, self, id, "qiubai:snow")?;
        if p.mode == Mode::Second && c.reason != "death" && Self::tac_up(w, id) {
            for a in self.tac_enemies(w, id, None, false)? {
                let amount = Self::tac_stat(w, id, "atk")? * p.finish;
                self.tac_damage(w, r, id, a, amount, DamageType::Physical, false, "swordEnd")?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_skill_tick(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Qiubai,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            for a in self.tac_enemies(w, id, None, false)? {
                self.advanced_status(w, r, id, a, "sluggish", 0.25, None)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Qiubai,
    ) -> Result<(), String> {
        if due[2] && p.crowd > 0 && p.crowd_aspd != 0. {
            if self.tac_enemies(w, id, None, true)?.len() >= p.crowd {
                self.component_buff(
                    w,
                    r,
                    id,
                    "qiubai:crowd",
                    [("aspd".into(), p.crowd_aspd)].into(),
                    0.4,
                    id,
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_attack(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        p: &Qiubai,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Third {
            let stack = w
                .unit(id)?
                .operator
                .tactical
                .qiubai()?
                .stacks
                .saturating_add(1)
                .min(p.stacks);
            w.unit_mut(id)?.operator.tactical.qiubai_mut()?.stacks = stack;
            self.component_buff(
                w,
                r,
                id,
                "qiubai:snow",
                [("aspd".into(), p.aspd * f64::from(stack))].into(),
                f64::INFINITY,
                id,
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Qiubai,
    ) -> Result<(), String> {
        if impact.is_skill && p.mode == Mode::First && Self::tac_up(w, target) {
            self.advanced_status(w, r, id, target, "bind", p.skill_bind, None)?;
            self.tac_timer(
                w,
                id,
                w.time + p.skill_bind,
                0.,
                1,
                Task::Explosion {
                    target,
                    position: w.unit(target)?.position,
                    scale: p.aoe,
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_calculate(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        c: &Qiubai,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        let target = q.hit.target;
        if c.gap > 0. {
            let mut b = Buff {
                key: Some(c.gap_key.into()),
                ..Default::default()
            };
            b.runtime.source = Some(id);
            b.runtime.on_remove = Some(GAP_FINISH);
            b.runtime.tick_payload = c.gap;
            let bid = r.add(
                w,
                self,
                target,
                buffs::Spec {
                    buff: b,
                    ..Default::default()
                },
            )?;
            if let Some(bid) = bid {
                self.tac_timer(w, id, w.time + 0.1, 0., 1, Task::Gap { target, buff: bid })?;
            }
        }
        let seq = w.unit(id)?.deployment.deploy_seq;
        let first =
            c.first > 0. && w.unit(id)?.operator.tactical.qiubai()?.hit.get(&target) != Some(&seq);
        if first {
            w.unit_mut(id)?
                .operator
                .tactical
                .qiubai_mut()?
                .hit
                .insert(target, seq);
            self.advanced_status(w, r, id, target, "bind", c.first, None)?;
        } else if c.probability > 0. && self.rng.chance(c.probability) {
            self.advanced_status(w, r, id, target, "bind", c.bind, None)?;
        }
        if c.module > 0. {
            let amount = Self::tac_stat(w, id, "atk")? * c.module;
            self.tac_damage(
                w,
                r,
                id,
                target,
                amount,
                DamageType::Arts,
                false,
                "lordAddition",
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn qiubai_explosion_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let id = t.source.ok_or("Operator timer requires a source")?;
        let Task::Explosion {
            target,
            position,
            scale,
        } = &mut t.kind
        else {
            return Err("Invalid explosion timer".into());
        };

        let pos = w.unit(*target).map(|u| u.position).unwrap_or(*position);
        for a in self.tac_radius(w, pos, 1.2)? {
            let amount = Self::tac_stat(w, id, "atk")? * *scale;
            self.tac_damage(w, r, id, a, amount, DamageType::Arts, false, "explosion")?;
        }

        Ok(true)
    }
    pub(in crate::battle::runtime) fn qiubai_gap_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let Task::Gap { target, buff } = &mut t.kind else {
            return Err("Invalid gap timer".into());
        };

        if w.unit(*target).is_ok() {
            r.remove_id(w, self, *target, *buff)?;
        }

        Ok(true)
    }
    pub(in crate::battle::runtime) fn qiubai_gap_removed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?.clone();
        let id = b.runtime.source.ok_or("Missing qiubai buff source")?;

        if w.unit(c.unit)?.stats.hp > 0. {
            let def = self.tac_def(id)?;
            let Tactics::Qiubai(p) = &def.extra.tactics else {
                return Err("Invalid gap callback".into());
            };
            let slug = Self::tac_has(w, c.unit, "sluggish");
            let bind = Self::tac_has(w, c.unit, "bind");
            if slug || bind {
                let amount =
                    Self::tac_stat(w, id, "atk")? * p.gap * if slug && bind { p.both } else { 1. };
                self.tac_damage(w, r, id, c.unit, amount, DamageType::Arts, false, "gap")?;
            }
        }
        return Ok(());
    }
}

impl Behavior for Qiubai {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_))
    }
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        let id = k.unit;
        match (role, event) {
            (
                Role::Source,
                KitEvent::Buff {
                    context,
                    token: GAP_FINISH,
                    ..
                },
            ) => {
                k.host.qiubai_gap_removed(k.world, k.buffs, *context)?;
            }

            (Role::Source, KitEvent::Attack { .. }) => {
                let active = k.active()?;
                k.host.qiubai_attack(k.world, id, k.buffs, active, self)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .qiubai_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (
                Role::Source,
                KitEvent::Hit {
                    phase: hit::Phase::Calculate,
                    request,
                },
            ) => {
                if k.world.unit(request.hit.target)?.side == health::Side::Enemy {
                    k.host
                        .qiubai_calculate(k.world, k.buffs, id, self, request)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                let mut hits =
                    std::mem::take(&mut k.world.unit_mut(id)?.operator.tactical.qiubai_mut()?.hit);
                hits.retain(|target, _| k.world.unit(*target).is_ok());
                k.world.unit_mut(id)?.operator.tactical.qiubai_mut()?.hit = hits;
                if k.up() {
                    k.host.qiubai_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.qiubai_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                ..
            } if *unit == id => {
                k.host.qiubai_skill_tick(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id => {
                k.host.qiubai_end(context, k.world, id, k.buffs, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
