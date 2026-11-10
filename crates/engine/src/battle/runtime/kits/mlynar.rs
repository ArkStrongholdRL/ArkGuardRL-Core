// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "librator",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_mlynar_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "CUSTOM_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_mlynar_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SEARCH",
            attack_override: true,
        },
        Skill {
            id: "skchr_mlynar_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
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
    let mode = tactical_mode(raw)?;

    s.combat.profile.only_while_skill = true;
    skill_range(raw, s)?;
    s.combat.skill_scale = n(bb, "attack@atk_scale", 1.);
    if mode == Mode::First {
        s.modifiers = [("defPct".into(), n(bb, "def", 0.))].into();
    }
    if mode == Mode::Second {
        s.modifiers = [(
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        )]
        .into();
        s.combat.skill_hits = 2;
    }
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.selection.can_hit_fly = true;
        p.max_targets = n(bb, "attack@max_target", 1.);
        s.combat.skill_profile = Some(p);
    }
    s.extra.effects.permanent.push(persistent(
        "mlynar:unmoved",
        [("taunt".into(), n(t1, "taunt_level", 0.))].into(),
    ));
    s.extra.tactics = Tactics::Mlynar(Mlynar {
        mode,
        ramp: n(tb, "atk", 2.),
        ramp_time: n(tb, "max_stack_cnt", 40.),
        initial: n(tb, "init_atk", 0.),
        scale: n(t, "atk_scale_base", 1.),
        crowd_scale: n(t, "atk_scale_up", 1.),
        crowd: n(t, "cnt", 3.).max(1.) as usize,
        cut: n(t, "damage_resistance", 0.),
        reflect: n(t1, "atk_scale", 0.),
        mark: n(bb, "atk_scale", 0.),
        up: n(bb, "trait_up", 1.),
        loss: n(bb, "per_kill_reduce", 0.),
    });
    Ok(())
}

pub(crate) struct Mlynar {
    pub mode: Mode,
    pub ramp: f64,
    pub ramp_time: f64,
    pub initial: f64,
    pub scale: f64,
    pub crowd_scale: f64,
    pub crowd: usize,
    pub cut: f64,
    pub reflect: f64,
    pub mark: f64,
    pub up: f64,
    pub loss: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub ramp: f64,
    pub kills: u32,
    pub pending_kills: u32,
    pub near: usize,
}

impl Host {
    pub(in crate::battle::runtime) fn tac_mlynar_up(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Mlynar,
    ) -> Result<(), String> {
        if p.mode != Mode::Third {
            return Ok(());
        }
        let m = w.unit(id)?.operator.tactical.mlynar()?;
        let extra = (m.ramp * p.up + p.loss * f64::from(m.kills)).max(0.) - m.ramp;
        if extra != 0. {
            self.component_buff(
                w,
                r,
                id,
                "mlynar:traitUp",
                [("atkPct".into(), extra)].into(),
                f64::INFINITY,
                id,
            )?;
        } else {
            r.remove(w, self, id, "mlynar:traitUp")?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn tactical_sync_injury(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        for id in self.roster.as_ref().unwrap().ids().to_vec() {
            let def = self.tac_def(id)?;
            let Tactics::Mlynar(p) = &def.extra.tactics else {
                continue;
            };
            let up = Self::tac_up(w, id);
            let leader = up
                && self
                    .roster
                    .as_ref()
                    .unwrap()
                    .ids()
                    .iter()
                    .copied()
                    .find(|&a| {
                        Self::tac_up(w, a)
                            && self.tac_def(a).is_ok_and(|d| d.char_id == def.char_id)
                    })
                    == Some(id);
            for a in w.unit_ids().collect::<Vec<_>>() {
                let ally = w.unit(a)?.side == health::Side::Ally;
                let key = format!("mlynar:{}:{id}", if ally { "reflection" } else { "mark" });
                let covered = if ally {
                    leader
                        && Self::tac_up(w, a)
                        && w.unit(a)?.has_operator_kit()
                        && self.tac_faction(a, "kazimierz")?
                        && p.reflect > 0.
                } else {
                    up && w.unit(id)?.operator.active_skill
                        && p.mode == Mode::Third
                        && Self::tac_up(w, a)
                        && body::in_range(w.unit(a)?, &self.roster.as_ref().unwrap().range(w, id)?)
                };
                if covered && !Self::tac_has(w, a, &key) {
                    let mut b = Buff {
                        key: Some(key),
                        ..Default::default()
                    };
                    b.runtime.source = Some(id);
                    b.runtime.injury.observe = Some(OBSERVE);
                    r.add(
                        w,
                        self,
                        a,
                        buffs::Spec {
                            buff: b,
                            ..Default::default()
                        },
                    )?;
                } else if !covered {
                    r.remove(w, self, a, &key)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Mlynar,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.mlynar_mut()?.kills = 0;
        w.unit_mut(id)?
            .operator
            .tactical
            .mlynar_mut()?
            .pending_kills = 0;
        r.remove(w, self, id, "trait:libratorBlock")?;
        self.tac_mlynar_up(w, r, id, p)?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_end(
        &mut self,
        c: &skills::Context,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Mlynar,
    ) -> Result<(), String> {
        r.remove(w, self, id, "mlynar:traitUp")?;
        if p.mode != Mode::Second
            || w.unit(id)?.operator.tactical.mlynar()?.kills == 0
            || c.reason == "death"
        {
            w.unit_mut(id)?.operator.tactical.mlynar_mut()?.ramp = 0.;
            r.remove(w, self, id, "trait:libratorRamp")?;
        }
        if Self::tac_up(w, id) {
            self.component_buff(
                w,
                r,
                id,
                "trait:libratorBlock",
                [("blockCnt".into(), -99.)].into(),
                f64::INFINITY,
                id,
            )?;
            buffs::Hooks::operator_release(self, w, r, id)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        due: [bool; 5],
        p: &Mlynar,
    ) -> Result<(), String> {
        if due[4] && !active {
            let ramp = (w.unit(id)?.operator.tactical.mlynar()?.ramp
                + p.ramp / p.ramp_time.max(1.))
            .min(p.ramp);
            w.unit_mut(id)?.operator.tactical.mlynar_mut()?.ramp = ramp;
            self.component_buff(
                w,
                r,
                id,
                "trait:libratorRamp",
                [("atkPct".into(), ramp)].into(),
                f64::INFINITY,
                id,
            )?;
        }
        if due[1] {
            let near = self.tac_radius(w, w.unit(id)?.position, 1.5)?.len();
            w.unit_mut(id)?.operator.tactical.mlynar_mut()?.near = near;
            if near >= p.crowd && p.cut > 0. {
                self.component_buff(
                    w,
                    r,
                    id,
                    "mlynar:ranger",
                    [("dmgTakenMul".into(), 1. - p.cut)].into(),
                    0.3,
                    id,
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_attack(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        p: &Mlynar,
    ) -> Result<(), String> {
        let kills = w.unit(id)?.operator.tactical.mlynar()?.pending_kills;
        w.unit_mut(id)?.operator.tactical.mlynar_mut()?.kills += kills;
        w.unit_mut(id)?
            .operator
            .tactical
            .mlynar_mut()?
            .pending_kills = 0;
        if kills > 0 && active {
            self.tac_mlynar_up(w, r, id, p)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_phase(
        &mut self,
        w: &mut World,
        id: Id,
        q: &mut hit::Request,
        c: &Mlynar,
    ) -> Result<(), String> {
        if q.is_attack {
            q.damage.amount *= if w.unit(id)?.operator.tactical.mlynar()?.near >= c.crowd {
                c.crowd_scale
            } else {
                c.scale
            };
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_landed(
        &mut self,
        w: &mut World,
        id: Id,
        q: &hit::Request,
        e: &health::Event,
    ) -> Result<(), String> {
        if w.unit(id)?.operator.active_skill
            && (q.is_attack || q.tags.iter().any(|t| t == "markOwn"))
            && w.unit(e.target)?.stats.hp <= 0.
        {
            w.unit_mut(id)?
                .operator
                .tactical
                .mlynar_mut()?
                .pending_kills = w
                .unit(id)?
                .operator
                .tactical
                .mlynar()?
                .pending_kills
                .saturating_add(1);
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_observe(
        c: &mut settlement::Context<'_, Self>,
        target: Id,
        key: &str,
        buff: &Buff,
    ) -> Result<(), String> {
        let id = buff.runtime.source.ok_or("Missing mlynar provider")?;
        let def = c.host.tac_def(id)?;
        let Tactics::Mlynar(p) = &def.extra.tactics else {
            return Err("Invalid mlynar observer".into());
        };
        let Some(source) = c.request.hit.source else {
            return Ok(());
        };
        if !Self::tac_up(c.world, id) {
            return Ok(());
        }
        if key.starts_with("mlynar:reflection:")
            && c.world.unit(source)?.side == health::Side::Enemy
            && Self::tac_up(c.world, source)
        {
            let amount = Self::tac_stat(c.world, id, "atk")? * p.reflect;
            c.host.tac_damage(
                c.world,
                c.runtime,
                id,
                source,
                amount,
                DamageType::True,
                false,
                "reflect",
            )?;
        }
        if key.starts_with("mlynar:mark:")
            && c.world.unit(source)?.side == health::Side::Ally
            && c.host.tac_faction(source, "kazimierz")?
            && matches!(
                c.request.attack_type,
                hit::AttackType::Normal | hit::AttackType::Splash
            )
            && c.world.unit(id)?.operator.active_skill
            && c.world.unit(target)?.stats.hp > 0.
        {
            let amount = Self::tac_stat(c.world, id, "atk")? * p.mark;
            c.host.tac_damage(
                c.world,
                c.runtime,
                id,
                target,
                amount,
                DamageType::True,
                false,
                if source == id && c.request.is_attack {
                    "markOwn"
                } else {
                    "mark"
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn mlynar_idle(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Mlynar,
    ) -> Result<(), String> {
        if Self::tac_up(w, id)
            && !Self::tac_has(w, id, "trait:libratorBlock")
            && !w.unit(id)?.operator.active_skill
        {
            if p.initial > 0. && w.unit(id)?.operator.tactical.mlynar()?.ramp > 0. {
                self.component_buff(
                    w,
                    r,
                    id,
                    "trait:libratorRamp",
                    [(
                        "atkPct".into(),
                        w.unit(id)?.operator.tactical.mlynar()?.ramp,
                    )]
                    .into(),
                    f64::INFINITY,
                    id,
                )?;
            }
            self.component_buff(
                w,
                r,
                id,
                "trait:libratorBlock",
                [("blockCnt".into(), -99.)].into(),
                f64::INFINITY,
                id,
            )?;
        }
        Ok(())
    }
}

impl State {
    pub fn deployed(p: &Mlynar) -> Self {
        Self {
            ramp: p.initial.min(p.ramp),
            ..Default::default()
        }
    }
}

impl Behavior for Mlynar {
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
            (Role::Source, KitEvent::Attack { .. }) => {
                let active = k.active()?;
                k.host.mlynar_attack(k.world, id, k.buffs, active, self)?;
            }
            (
                Role::Source,
                KitEvent::Hit {
                    phase: hit::Phase::Hit,
                    request,
                },
            ) => {
                k.host.mlynar_phase(k.world, id, request, self)?;
            }
            (Role::Source, KitEvent::Landed { event, request }) => {
                k.host.mlynar_landed(k.world, id, request, event)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn observe_injury(
        &self,
        c: &mut settlement::Context<'_, Host>,
        key: &str,
        buff: &Buff,
    ) -> Result<(), String> {
        if key.starts_with("mlynar:") {
            Host::mlynar_observe(c, c.request.hit.target, key, buff)?;
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                if k.up() {
                    let active = k.active()?;
                    k.host
                        .mlynar_scheduled(k.world, id, k.buffs, active, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.mlynar_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id => {
                k.host.mlynar_end(context, k.world, id, k.buffs, self)?;
            }
            KitEvent::Flush => {
                k.host.mlynar_idle(k.world, k.buffs, id, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
