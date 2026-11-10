// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "hunter",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_snhunt_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_snhunt_1",
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
    let mode = tactical_mode(raw)?;

    s.combat.skill_scale = n(bb, "atk_scale", 1.);
    s.extra.tactics = Tactics::Snhunt(Snhunt {
        mode,
        capacity: n(tb, "value", 8.),
        ammo_scale: n(tb, "atk_scale", 1.2),
        reload: n(tb, "extra_add", 0.),
        moving: n(bb, "atk_scale_1", 1.),
        still: n(bb, "atk_scale_2", n(bb, "atk_scale_1", 1.)),
        beast: n(t, "atk_scale", 1.),
        cold: n(t, "cold", 3.),
    });
    Ok(())
}

pub(crate) struct Snhunt {
    pub mode: Mode,
    pub capacity: f64,
    pub ammo_scale: f64,
    pub reload: f64,
    pub moving: f64,
    pub still: f64,
    pub beast: f64,
    pub cold: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub lock: Option<Id>,
    pub ammo: f64,
    pub reload: f64,
    pub previous_ammo: f64,
    pub last_attack: f64,
}

impl Host {
    pub(in crate::battle::runtime) fn snhunt_start(
        &mut self,
        w: &mut World,
        id: Id,
        p: &Snhunt,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            let target = self.tac_enemies(w, id, None, true)?.first().copied();
            w.unit_mut(id)?.operator.tactical.snhunt_mut()?.lock = target;
            if let Some(t) = target {
                self.tac_fire(w, id, t, ArrowKind::Snow, p.moving, 2)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn snhunt_attack(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.snhunt_mut()?.ammo =
            (w.unit(id)?.operator.tactical.snhunt()?.ammo - 1.).max(0.);
        w.unit_mut(id)?.operator.tactical.snhunt_mut()?.last_attack = w.time;
        Ok(())
    }
    pub(in crate::battle::runtime) fn snhunt_tick(
        &mut self,
        w: &mut World,
        id: Id,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        if w.unit(id)?.operator.tactical.snhunt()?.ammo <= 0.
            && s.bank.ready()
            && !s.op_cooling(w.time)
            && skills::can_act(w, id)?
            && !Self::tac_has(w, id, "silence")
        {
            if !self.tac_enemies(w, id, None, true)?.is_empty() {
                s.activate(w, self, "DEFAULT", false)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn snhunt_skill_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Snhunt,
    ) -> Result<(), String> {
        let target = if p.mode == Mode::Second {
            w.unit(id)?.operator.tactical.snhunt()?.lock
        } else {
            self.tac_enemies(w, id, None, true)?.first().copied()
        };
        if let Some(a) = target {
            let amount = Self::tac_stat(w, id, "atk")? * p.beast;
            self.tac_damage(
                w,
                r,
                id,
                a,
                amount,
                DamageType::Physical,
                false,
                "cloudbeast",
            )?;
            if w.unit(a)?.stats.alive {
                self.advanced_status(w, r, id, a, "cold", p.cold, None)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn snhunt_reload(
        &mut self,
        w: &mut World,
        id: Id,
        p: &Snhunt,
        dt: f64,
    ) -> Result<(), String> {
        let now = w.time;
        let m = w.unit_mut(id)?.operator.tactical.snhunt_mut()?;
        if now - m.last_attack >= 1. && m.ammo < p.capacity {
            m.reload += dt;
            if m.reload >= 1. {
                m.reload -= 1.;
                m.ammo += 1.;
            }
        } else {
            m.reload = 0.;
        }
        if m.previous_ammo == 0. && m.ammo == 1. {
            m.ammo = (m.ammo + p.reload).min(p.capacity);
        }
        m.previous_ammo = m.ammo;
        Ok(())
    }
    pub(in crate::battle::runtime) fn snhunt_arrow_scale(
        &self,
        w: &World,
        target: Option<Id>,
        scale: f64,
        p: &Snhunt,
    ) -> Result<f64, String> {
        if let Some(t) = target {
            let u = w.unit(t)?;
            let still = u.blocked_by().is_some()
                || !u.moving
                || Self::tac_has(w, t, "stun")
                || Self::tac_has(w, t, "bind");
            return Ok(if still { p.still } else { p.moving });
        }

        Ok(scale)
    }
}

impl State {
    pub fn deployed(p: &Snhunt) -> Self {
        Self {
            ammo: p.capacity,
            previous_ammo: p.capacity,
            last_attack: f64::NEG_INFINITY,
            ..Default::default()
        }
    }
}

impl Behavior for Snhunt {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle()
    }
    fn refreshes_skill(&self) -> bool {
        true
    }
    fn profile(
        &self,
        world: &World,
        unit: Id,
        _: Option<&skills::Runtime>,
        profile: &mut search::AttackProfile,
    ) -> Result<(), String> {
        if world.unit(unit)?.operator.tactical.snhunt()?.ammo <= 0. {
            profile.no_attack = true;
        }
        Ok(())
    }
    fn payload(
        &self,
        world: &World,
        unit: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        if world.unit(unit)?.operator.tactical.snhunt()?.ammo > 0. {
            payload.atk_scale *= self.ammo_scale;
        }
        Ok(())
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
                k.host.snhunt_attack(k.world, id)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.snhunt_start(k.world, id, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::SkillStart,
                ..
            } if *unit == id => {
                k.host.snhunt_skill_start(k.world, k.buffs, id, self)?;
            }
            KitEvent::Refresh { skill, dt, .. } => {
                k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                    host.snhunt_reload(world, id, self, *dt)?;
                    if self.mode == Mode::Second {
                        host.snhunt_tick(world, id, skill)?;
                    }
                    Ok(())
                })?;
            }
            _ => {}
        }
        Ok(())
    }
}
