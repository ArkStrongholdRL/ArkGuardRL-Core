// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "splashcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_dusk_1",
        kind: crate::battle::sp::Kind::Charges,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    s.combat.skill_scale = n(bb, "atk_scale", 1.);
    s.combat.area.splash = Some(Splash {
        radius: 1.1,
        scale: 1.,
        hits_fly: true,
        fortress_melee: false,
    });
    s.combat.skill_area = s.combat.area;
    s.combat.skill_area.splash.as_mut().unwrap().radius = 1.7;
    crate::battle::content::operators::mechanics::module_range(raw, s)?;
    s.extra.tactics = Tactics::Dusk(Dusk {
        attack: n(talent_at(raw, 0), "atk", 0.),
        cap: n(talent_at(raw, 0), "max_stack_cnt", 15.).max(1.) as u32,
        duration: n(talent_at(raw, 1), "attack@tokenduration", 25.),
    });
    Ok(())
}
pub(crate) struct Dusk {
    attack: f64,
    cap: u32,
    duration: f64,
}
#[derive(Default)]
pub(crate) struct State {
    summoned: bool,
}
impl Behavior for Dusk {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Attack { context, .. }
                if context.attacker == id
                    && !k.world.unit(id)?.operator.tactical.dusk()?.summoned =>
            {
                if let Some(&target) = context.targets.first() {
                    k.world.unit_mut(id)?.operator.tactical.dusk_mut()?.summoned = true;
                    let p = k
                        .world
                        .unit(target)?
                        .position
                        .map(|v| (v + 0.5).floor() as i32);
                    let grid = k.host.grid.as_ref().ok_or("Missing dusk summon grid")?;
                    if grid.can_stand(p, false) && grid.obstacle(p) == 0 {
                        k.summon("token_10015_dusk_drgn", p, self.duration)?;
                    }
                }
            }
            KitEvent::Death {
                unit,
                credit: Some(killer),
            } if k.up()
                && k.world.unit(*unit)?.side == health::Side::Enemy
                && (*killer == id || k.world.unit(*killer)?.summoner == Some(id)) =>
            {
                let mut buff = Buff {
                    key: Some("dusk:realm".into()),
                    mods: [("atkPct".into(), self.attack)].into(),
                    ..Default::default()
                };
                buff.runtime.refresh = buffs::Refresh::Stack;
                buff.runtime.max_stacks = self.cap;
                k.add(id, buff)?;
            }
            _ => {}
        }
        Ok(())
    }
}
