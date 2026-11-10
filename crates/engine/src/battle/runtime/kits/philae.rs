// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "primprotector",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_philae_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_philae_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
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

    s.modifiers = [("hpPct".into(), n(bb, "max_hp", 0.))].into();
    if mode == Mode::Second {
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Philae(Philae {
        mode,
        scale: n(bb, "atk_scale", 1.),
        element: n(bb, "ep_damage_ratio", 0.),
        cooldown: n(bb, "aoe_cd", 2.),
        attack: n(bb, "atk", 0.),
        cut: n(t, "damage_resistance", 0.),
        sp: n(t, "sp", 0.),
        barrier: n(bb, "shield_value", 0.),
        element_scale: n(tb, "ep_damage_scale", 1.),
    });
    Ok(())
}

pub(crate) struct Philae {
    pub mode: Mode,
    pub scale: f64,
    pub element: f64,
    pub cooldown: f64,
    pub attack: f64,
    pub cut: f64,
    pub sp: f64,
    pub barrier: f64,
    pub element_scale: f64,
}

pub(crate) struct State {
    pub ready: f64,
    pub barrier: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ready: f64::NEG_INFINITY,
            barrier: Default::default(),
        }
    }
}

impl Host {
    pub(in crate::battle::runtime) fn philae_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Philae,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.philae_mut()?.ready = f64::NEG_INFINITY;
        w.unit_mut(id)?.operator.tactical.philae_mut()?.barrier = p.barrier;
        if p.mode == Mode::First {
            if !crate::battle::element::locked(w, id)? {
                w.unit_mut(id)?.element_buildup = [0.; 5];
            }
        } else {
            let mut b = Buff {
                key: Some("philae:counter".into()),
                ..Default::default()
            };
            b.runtime.injury.observe = Some(OBSERVE);
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
        Ok(())
    }
    pub(in crate::battle::runtime) fn philae_end(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.philae_mut()?.barrier = 0.;
        r.remove(w, self, id, "philae:counter")?;
        r.remove(w, self, id, "skill:philae_rage")?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn philae_observe(
        c: &mut settlement::Context<'_, Self>,
        target: Id,
    ) -> Result<(), String> {
        let def = c.host.tac_def(target)?;
        let Tactics::Philae(p) = &def.extra.tactics else {
            return Err("Invalid philae observer".into());
        };
        if !Self::tac_up(c.world, target)
            || !c.world.unit(target)?.operator.active_skill
            || c.world.time < c.world.unit(target)?.operator.tactical.philae()?.ready
        {
            return Ok(());
        }
        c.world
            .unit_mut(target)?
            .operator
            .tactical
            .philae_mut()?
            .ready = c.world.time + p.cooldown;
        let keys = c.host.tac_keys(
            c.world,
            target,
            &[
                [-1, -1],
                [-1, 0],
                [-1, 1],
                [0, -1],
                [0, 0],
                [0, 1],
                [1, -1],
                [1, 0],
                [1, 1],
            ],
        )?;
        for a in c.host.tac_enemies(c.world, target, Some(&keys), false)? {
            c.host.tac_timer(
                c.world,
                target,
                c.world.time + 0.0334,
                0.,
                1,
                Task::Blast {
                    target: a,
                    scale: p.scale,
                    element: p.element,
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn philae_blast_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let id = t.source.ok_or("Operator timer requires a source")?;
        let Task::Blast {
            target,
            scale,
            element,
        } = &mut t.kind
        else {
            return Err("Invalid blast timer".into());
        };

        if Self::tac_up(w, *target) {
            let attack = Self::tac_stat(w, id, "atk")?;
            self.tac_damage(
                w,
                r,
                id,
                *target,
                attack * *scale,
                DamageType::Arts,
                false,
                "counter",
            )?;
            if Self::tac_up(w, *target) && *element > 0. {
                let mut q = crate::battle::element::Request::new(
                    Some(id),
                    Some(id),
                    *target,
                    crate::battle::element::Element::Apoptosis,
                    attack * *element,
                );
                crate::battle::element::apply(w, self, r, &mut q)?;
            }
        }

        Ok(true)
    }
}

impl Behavior for Philae {
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
        if let KitEvent::Element {
            phase: crate::battle::element::Phase::Hit,
            request,
        } = event
        {
            let target = request.damage.hit.target;
            match role {
                Role::Target => {
                    if request.damage.element == Some(crate::battle::element::Element::Apoptosis)
                        && !k.active()?
                    {
                        k.host
                            .advanced_grant_sp(k.world, k.buffs, target, self.sp)?;
                    }
                    request.damage.damage.mul *= 1. - self.cut.clamp(0., 1.);
                    if k.active()? {
                        if self.mode == Mode::First {
                            let amount = request.damage.damage.amount * request.damage.damage.mul;
                            let barrier = k.world.unit(id)?.operator.tactical.philae()?.barrier;
                            if amount > 0. && barrier > 0. {
                                let take = amount.min(barrier);
                                k.world
                                    .unit_mut(id)?
                                    .operator
                                    .tactical
                                    .philae_mut()?
                                    .barrier -= take;
                                if take >= amount - 1e-9 {
                                    request.damage.cancelled = true;
                                } else {
                                    request.damage.damage.amount *= (amount - take) / amount;
                                }
                            }
                        } else if !k.has(id, "skill:philae_rage") {
                            k.host.component_buff(
                                k.world,
                                k.buffs,
                                id,
                                "skill:philae_rage",
                                [("atkPct".into(), self.attack)].into(),
                                f64::INFINITY,
                                id,
                            )?;
                        }
                    }
                }
                Role::Source => {
                    if k.world.unit(target)?.side == health::Side::Enemy
                        && !k.world.unit(id)?.blocking().is_empty()
                    {
                        request.damage.damage.mul *= self.element_scale;
                    }
                }
            }
        }
        Ok(())
    }
    fn observe_injury(
        &self,
        c: &mut settlement::Context<'_, Host>,
        key: &str,
        _: &Buff,
    ) -> Result<(), String> {
        if key == "philae:counter" {
            Host::philae_observe(c, c.request.hit.target)?;
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
                k.host.philae_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                k.host.philae_end(k.world, id, k.buffs)?;
            }
            _ => {}
        }
        Ok(())
    }
}
