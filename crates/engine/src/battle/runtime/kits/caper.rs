// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "loopshooter",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_caper_2",
            kind: Kind::Duration,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_caper_1",
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
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_caper_1" => Mode::First,
        "skchr_caper_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.projectile = Some("boomerang");
    s.combat.projectile_speed = 15.;
    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
    } else {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
        s.combat.skill_hits = n(bb, "cnt", 1.).max(1.) as u32;
    }
    s.extra.tactics = Tactics::Caper(Caper {
        near: n(tb, "atk_scale", 1.),
        prob: n(t, "prob", 0.),
        crit: n(t, "atk_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Caper {
    pub near: f64,
    pub prob: f64,
    pub crit: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub out: u32,
}
impl Behavior for Caper {
    fn can_attack(&self, _host: &Host, world: &World, unit: Id) -> Result<bool, String> {
        Ok(world.unit(unit)?.operator.tactical.caper()?.out == 0)
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Attack { context, .. } if context.attacker == id => {
                k.world.unit_mut(id)?.operator.tactical.caper_mut()?.out += 1;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id) && request.is_attack => {
                let at = k.world.unit(id)?.position;
                if body::tile_reach(
                    k.world.unit(request.hit.target)?,
                    at[0].round() as i32,
                    at[1].round() as i32,
                ) <= 1.
                {
                    request.damage.amount *= self.near;
                }
                if !request.is_splash && self.prob > 0. && k.host.rng.chance(self.prob) {
                    request.damage.amount *= self.crit;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
impl Host {
    pub(in crate::battle::runtime) fn caper_return(
        &mut self,
        world: &mut World,
        source: Id,
        deployment: u64,
        at: [f64; 2],
    ) -> Result<(), String> {
        if !Self::tac_up(world, source) || world.unit(source)?.deployment.deploy_seq != deployment {
            return Ok(());
        }
        self.flights
            .as_mut()
            .ok_or("Missing return projectiles")?
            .add(
                world,
                projectiles::Launch {
                    from: at,
                    source: Some(source),
                    target: Some(source),
                    to: None,
                    speed: 3.75,
                    hit_dead: true,
                    max_age: 10.,
                    on_hit: true,
                    visual: "boomerangReturn".into(),
                    speed_changeable: None,
                    removable: None,
                    payload: allies::Shot::BoomerangReturn { source, deployment },
                },
            )
            .map(|_| ())
    }
}
