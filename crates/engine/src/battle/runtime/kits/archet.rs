// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "fastshot",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_archet_3",
            kind: Kind::Duration,
            sp_type: SpType::Attack,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_archet_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_archet_2",
            kind: Kind::Charges,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = tactical_mode(raw)?;

    if raw["skill"]["trigger"]["customRangeGrid"].is_array() {
        s.trigger_range = Some(crate::battle::operators::parse_range(
            &raw["skill"]["trigger"]["customRangeGrid"],
        )?);
    }
    fastshot(raw, s);
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    s.combat.skill_scale = n(bb, "atk_scale", 1.);
    s.combat.skill_hits = n(bb, "attack@times", 1.).floor().max(1.) as u32;
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.max_targets = n(bb, "attack@max_target", 1.);
        s.combat.skill_profile = Some(p);
        s.extra.advanced_range_extend = n(bb, "ability_range_forward_extend", 0.).max(0.) as u32;
    }
    let mut shield = Buff {
        key: Some("talent:archet_shield".into()),
        ..Default::default()
    };
    shield.runtime.shield_hits = 1;
    s.extra.effects.deploy_buff = Some(shield);
    let times = n(bb, "times", 5.).floor().max(1.);
    if times > f64::from(crate::battle::operators::Combat::MAX_HITS) {
        return Err("Unsupported pursuit hit count".into());
    }
    s.extra.tactics = Tactics::Archet(Archet {
        mode,
        scatter: n(bb, "atk_scale_2", 1.),
        targets: n(bb, "max_target", 4.).max(1.) as usize,
        times: times as u32,
        scale: n(bb, "atk_scale", 1.),
        interval: n(t, "interval", 2.5),
        sp: n(t, "sp", 1.),
        shield_sp: n(t1, "sp", 0.),
        aspd: n(tm, "attack_speed", 0.),
    });
    Ok(())
}

pub(crate) struct Archet {
    pub mode: Mode,
    pub scatter: f64,
    pub targets: usize,
    pub times: u32,
    pub scale: f64,
    pub interval: f64,
    pub sp: f64,
    pub shield_sp: f64,
    pub aspd: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub sp_next: f64,
    pub abnormal: bool,
}

impl Host {
    pub(in crate::battle::runtime) fn archet_start(
        &mut self,
        w: &mut World,
        id: Id,
        p: &Archet,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            if let Some(&a) = self.tac_enemies(w, id, None, true)?.first() {
                self.tac_fire(w, id, a, ArrowKind::Pursuit, p.scale, p.times)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn archet_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        archet_due: bool,
        p: &Archet,
    ) -> Result<(), String> {
        if archet_due {
            if self.tac_leader(w, id)? {
                for a in self.roster.as_ref().unwrap().ids().to_vec() {
                    let d = self.tac_def(a)?;
                    if Self::tac_up(w, a)
                        && d.profession == "SNIPER"
                        && d.sp_type() == skills::SpType::Attack
                    {
                        self.advanced_grant_sp(w, r, a, p.sp)?;
                    }
                }
            }
        }
        if due[0] && p.aspd > 0. {
            let mut ground = false;
            for a in self.tac_enemies(w, id, None, true)? {
                ground |= !targeting::flying(w, a)?;
            }
            self.advanced_toggle(
                w,
                r,
                id,
                "trait:archet_ground",
                ground,
                [("aspd".into(), p.aspd)].into(),
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn archet_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Archet,
    ) -> Result<(), String> {
        if impact.is_skill && p.mode == Mode::First {
            let position = impact.position;
            let candidates = self.tac_radius(w, position, 1.5)?;
            let mut ids = self.tac_selectable(w, id, candidates)?;
            ids.retain(|&a| a != target);
            ids.sort_by(|a, b| {
                body::distance(w.unit(*a).unwrap(), position)
                    .total_cmp(&body::distance(w.unit(*b).unwrap(), position))
                    .then(
                        w.unit(*a)
                            .unwrap()
                            .targeting
                            .spawn_seq
                            .cmp(&w.unit(*b).unwrap().targeting.spawn_seq),
                    )
            });
            for a in ids.into_iter().take(p.targets.saturating_sub(1)) {
                let amount = Self::tac_stat(w, id, "atk")? * p.scatter;
                self.tac_damage(w, r, id, a, amount, DamageType::Physical, false, "scatter")?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn archet_shield_lost(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        p: &Archet,
    ) -> Result<(), String> {
        if !Self::tac_has(w, e.target, "talent:archet_shield")
            && !w.unit(e.target)?.operator.tactical.archet()?.abnormal
        {
            w.unit_mut(e.target)?
                .operator
                .tactical
                .archet_mut()?
                .abnormal = true;
            self.advanced_grant_sp(w, r, e.target, p.shield_sp)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn archet_continue_arrow(
        &mut self,
        w: &mut World,
        arrow: Arrow,
        position: [f64; 2],
    ) -> Result<(), String> {
        let candidates = self.tac_radius(w, position, 1.5)?;
        let mut next = self.tac_selectable(w, arrow.source, candidates)?;
        next.retain(|t| !arrow.seen[..arrow.count].contains(t));
        next.sort_by(|a, b| {
            body::distance(w.unit(*a).unwrap(), position)
                .total_cmp(&body::distance(w.unit(*b).unwrap(), position))
                .then(
                    w.unit(*a)
                        .unwrap()
                        .targeting
                        .spawn_seq
                        .cmp(&w.unit(*b).unwrap().targeting.spawn_seq),
                )
        });
        if let Some(&t) = next.first() {
            let mut next_arrow = arrow;
            next_arrow.hits -= 1;
            next_arrow.seen[next_arrow.count] = t;
            next_arrow.count += 1;
            Self::tac_arrow_launch(
                w,
                self.flights.as_mut().ok_or("Missing pursuit projectiles")?,
                position,
                t,
                next_arrow,
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn archet_sp_due(
        &self,
        w: &mut World,
        id: Id,
        p: &Archet,
    ) -> Result<bool, String> {
        let at = w
            .unit(id)?
            .operator
            .tactical
            .archet()?
            .sp_next
            .max(p.interval);
        let due = w.time + 1e-9 >= at;
        if due {
            w.unit_mut(id)?.operator.tactical.archet_mut()?.sp_next = at + p.interval;
        }
        Ok(due)
    }
}

impl State {
    pub fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            sp_next: previous.map_or(0., |p| p.sp_next),
            ..Default::default()
        }
    }
}

impl Behavior for Archet {
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
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .archet_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (Role::Target, KitEvent::Landed { event, .. }) => {
                k.host.archet_shield_lost(k.world, k.buffs, event, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                let sp_due = k.host.archet_sp_due(k.world, id, self)?;
                if k.up() {
                    k.host
                        .archet_scheduled(k.world, id, k.buffs, *due, sp_due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.archet_start(k.world, id, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
