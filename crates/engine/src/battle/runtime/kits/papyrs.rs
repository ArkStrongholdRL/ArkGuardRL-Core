// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "chainhealer",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_papyrs_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_papyrs_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
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
    let mode = tactical_mode(raw)?;

    healing(s, 1.);
    if mode == Mode::First {
        s.combat.skill_heal_scale = n(bb, "heal_scale", 1.);
    } else {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            (
                "batPct".into(),
                flat_bat(raw, n(bb, "base_attack_time", 0.)),
            ),
        ]
        .into();
    }
    s.extra.tactics = Tactics::Papyrs(Papyrs {
        mode,
        count: n(tb, "attack@chain.max_target", 3.),
        falloff: 1. - n(tb, "attack@chain.atk_scale", 0.75),
        extra: n(bb, "attack@chain.extra_value", 0.),
        shield: n(t, "attack@scale", 0.),
        duration: n(t, "attack@shield_duration", 8.),
        boost: n(bb, "shield_scale_skill", 1.),
    });
    Ok(())
}

pub(crate) struct Papyrs {
    pub mode: Mode,
    pub count: f64,
    pub falloff: f64,
    pub extra: f64,
    pub shield: f64,
    pub duration: f64,
    pub boost: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub lock: Option<Id>,
}

impl Host {
    pub(in crate::battle::runtime) fn tac_lock(
        &self,
        w: &mut World,
        id: Id,
    ) -> Result<Option<Id>, String> {
        let keys = self.roster.as_ref().unwrap().range(w, id)?;
        let mut ids = self.tac_heal_targets(w, id, &keys, false)?;
        ids.retain(|&a| a != id);
        let mut rank = vec![];
        for a in ids {
            rank.push((
                a,
                Self::tac_stat(w, a, "maxHp")?,
                w.unit(a)?.deployment.deploy_seq,
            ));
        }
        rank.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
        Ok(rank.first().map(|x| x.0))
    }

    pub(in crate::battle::runtime) fn papyrs_start(
        &mut self,
        w: &mut World,
        id: Id,
        p: &Papyrs,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            w.unit_mut(id)?.operator.tactical.papyrs_mut()?.lock = self.tac_lock(w, id)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn papyrs_end(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.papyrs_mut()?.lock = None;
        Ok(())
    }
    pub(in crate::battle::runtime) fn papyrs_skill_tick(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        s: &mut skills::Runtime,
        p: &Papyrs,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            if !w
                .unit(id)?
                .operator
                .tactical
                .papyrs()?
                .lock
                .is_some_and(|a| Self::tac_up(w, a))
            {
                self.with_kit_buffs(w, r, |h, w| s.end(w, h, "target"))?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn papyrs_before(
        &mut self,
        w: &mut World,
        id: Id,
        active: bool,
        c: &mut attacks::Context,
        p: &Papyrs,
    ) -> Result<(), String> {
        if p.mode == Mode::Second && active {
            if let Some(a) = w.unit(id)?.operator.tactical.papyrs()?.lock {
                let keys = self.roster.as_ref().unwrap().range(w, id)?;
                if Self::tac_up(w, a) && body::in_range(w.unit(a)?, &keys) {
                    c.targets = vec![a];
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn papyrs_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Papyrs,
    ) -> Result<(), String> {
        if impact.heal {
            let count = p.count
                + if w.unit(id)?.operator.active_skill && p.mode == Mode::Second {
                    p.extra
                } else {
                    0.
                };
            if count.ceil() > w.unit_capacity() as f64 {
                return Err("Heal-chain capacity exceeded".into());
            }
            let mut seen = vec![target];
            let mut prev = target;
            for k in 1..count.ceil() as i32 {
                let tile = w.unit(prev)?.deployment.tile;
                let mut choices = vec![];
                for &a in self.roster.as_ref().unwrap().ids() {
                    if seen.contains(&a) || !Self::tac_up(w, a) {
                        continue;
                    }
                    let u = w.unit(a)?;
                    if u.hidden
                        || u.profile_no_heal
                        || Self::tac_has(w, a, "noHeal")
                        || (u.deployment.tile[0] - tile[0]).abs() > 1
                        || (u.deployment.tile[1] - tile[1]).abs() > 1
                    {
                        continue;
                    }
                    choices.push((
                        a,
                        u.stats.hp / Self::tac_stat(w, a, "maxHp")?,
                        w.unit(a)?.deployment.aggro_seq,
                    ));
                }
                choices.sort_by(|a, b| a.1.total_cmp(&b.1).then(b.2.cmp(&a.2)));
                let Some(a) = choices.first().map(|x| x.0) else {
                    break;
                };
                self.tac_heal(
                    w,
                    r,
                    id,
                    a,
                    impact.amount * powi(1. - p.falloff, k).map_err(str::to_string)?,
                )?;
                seen.push(a);
                prev = a;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn papyrs_heal_event(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        e: &health::Event,
        p: &Papyrs,
    ) -> Result<(), String> {
        if e.amount > 0. && !(e.kind == health::Kind::Regen && id == e.target) {
            let active = w.unit(id)?.operator.active_skill;
            let pending = self
                .roster
                .as_ref()
                .unwrap()
                .entry(id)?
                .skill()
                .ok()
                .map_or(active && p.mode == Mode::First, |s| {
                    s.pending && s.bank.active
                });
            let amount =
                Self::tac_stat(w, id, "atk")? * p.shield * if pending { p.boost } else { 1. };
            if amount > 0. {
                let mut b = Buff {
                    key: Some("papyrs:shield".into()),
                    shield: amount,
                    ..Default::default()
                };
                b.runtime.duration = p.duration;
                b.runtime.source = Some(id);
                r.add(
                    w,
                    self,
                    e.target,
                    buffs::Spec {
                        buff: b,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
}

impl Behavior for Papyrs {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Heal { .. }) {
            -10
        } else {
            0
        }
    }
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle()
    }
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        let id = k.unit;
        match (role, event) {
            (Role::Source, KitEvent::Before(context)) => {
                let active = k.active()?;
                k.host.papyrs_before(k.world, id, active, context, self)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .papyrs_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (Role::Source, KitEvent::Heal { event, .. }) => {
                k.host
                    .papyrs_heal_event(k.world, id, k.buffs, event, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn can_activate(&self, host: &Host, world: &mut World, unit: Id) -> Result<bool, String> {
        Ok(self.mode != Mode::Second || host.tac_lock(world, unit)?.is_some())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.papyrs_start(k.world, id, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                skill,
                ..
            } if *unit == id => {
                k.host
                    .papyrs_skill_tick(k.world, id, k.buffs, skill, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                k.host.papyrs_end(k.world, id)?;
            }
            _ => {}
        }
        Ok(())
    }
}
