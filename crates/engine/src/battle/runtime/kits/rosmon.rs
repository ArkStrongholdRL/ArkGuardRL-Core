// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "bombarder",
    ranged: true,
    can_hit_fly: false,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_rosmon_2",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_rosmon_2" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        (
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        ),
    ]
    .into();
    s.combat.profile.selection.ground_only = true;
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.area.splash = Some(Splash {
        radius: 0.9,
        scale: 1.,
        hits_fly: false,
        fortress_melee: false,
    });
    s.combat.skill_area = s.combat.area;
    let penetration = n(t, "def_penetrate_fixed", 0.);
    if penetration != 0. {
        s.extra.effects.permanent.push(persistent(
            "rosmon:pierce",
            [("defIgnoreFlat".into(), penetration)].into(),
        ));
    }
    let times = n(tb, "attack@times", 2.).floor().max(1.);
    let extra = n(bb, "add_times", 0.).floor().max(0.);
    if times + extra > 64. {
        return Err("Aftershock count exceeds capacity".into());
    }
    s.extra.tactics = Tactics::Rosmon(Rosmon {
        times: times as u32,
        extra: extra as u32,
        scale: n(tb, "attack@append_atk_scale", 0.5),
        probability: n(bb, "attack@prob", 0.),
        stun: n(bb, "attack@stun", 0.),
        stable: n(t1, "atk", 0.),
    });
    Ok(())
}
pub(crate) struct Rosmon {
    times: u32,
    extra: u32,
    scale: f64,
    probability: f64,
    stun: f64,
    stable: f64,
}
impl Rosmon {
    fn stable(k: &mut KitContext<'_>) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Rosmon(p) = &def.extra.tactics else {
            return Err("Invalid rosmon kit".into());
        };
        let mut casters = Vec::new();
        for &id in k.host.roster.as_ref().unwrap().ids() {
            if Host::tac_up(k.world, id) && k.host.tac_def(id)?.profession == "CASTER" {
                casters.push(id);
            }
        }
        if let Some(target) = k.host.rng.pick(&casters).copied() {
            k.buff(
                k.unit,
                "rosmon:stable",
                [("atkPct".into(), p.stable)].into(),
                f64::INFINITY,
            )?;
            k.buff(
                target,
                "rosmon:stable",
                [("atkPct".into(), p.stable)].into(),
                f64::INFINITY,
            )?;
        }
        Ok(())
    }
    fn aftershock(k: &mut KitContext<'_>, position: [f64; 2], _: f64) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Rosmon(p) = &def.extra.tactics else {
            return Err("Invalid rosmon kit".into());
        };
        let radius = if k.active()? { 1.5 } else { 0.9 };
        let targets = attacks::Hooks::radius_targets(
            k.host,
            position,
            radius,
            true,
            k.world,
            k.world.unit_capacity(),
        )?;
        for target in targets {
            if targeting::flying(k.world, target)? {
                continue;
            }
            let amount = k.stat(k.unit, "atk")? * p.scale;
            let mut request = hit::Request::new(
                health::Hit {
                    source: Some(k.unit),
                    credit: Some(k.unit),
                    target,
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
            request.is_splash = true;
            request.tags.push("aftershock".into());
            k.deal(&mut request)?;
        }
        Ok(())
    }
}
impl Behavior for Rosmon {
    fn payload(
        &self,
        w: &World,
        unit: Id,
        _: &mut attacks::Direct,
        area: &mut attacks::Area,
    ) -> Result<(), String> {
        if w.unit(unit)?.operator.active_skill {
            if let Some(splash) = &mut area.splash {
                splash.radius = 1.5;
            }
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == k.unit && self.stable != 0. => {
                k.later(0., Self::stable)?;
            }
            KitEvent::Impact(impact)
                if impact.attacker == k.unit && impact.kind == attacks::VictimKind::Main =>
            {
                let times = (self.times + if k.active()? { self.extra } else { 0 })
                    .saturating_sub(1)
                    .max(1);
                for i in 1..=times {
                    k.later_at(0.3 * i as f64, impact.position, 0., true, Self::aftershock)?;
                }
            }
            KitEvent::Landed { event, request }
                if event.source == Some(k.unit)
                    && k.active()?
                    && self.stun > 0.
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && k.world.unit(event.target)?.stats.alive
                    && (request.is_attack || request.tags.iter().any(|t| t == "aftershock")) =>
            {
                if k.host.rng.next_f64() < self.probability {
                    k.status(event.target, "stun", self.stun, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
