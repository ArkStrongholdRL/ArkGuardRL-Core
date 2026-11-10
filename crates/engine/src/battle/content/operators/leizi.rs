// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "chain",
    can_hit_fly: true,
    ranged: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_leizi_2")
        },
        Skill::duration("skcom_atk_up[3]"),
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    let tb = &raw["trait"]["bb"];
    s.modifiers = {
        s.combat.area.chain = Some(Chain {
            count: number(&tb["attack@max_target"], 3.),
            radius: 1.7,
            falloff: if tb["attack@chain.atk_scale"].is_null() {
                0.15
            } else {
                1. - number(&tb["attack@chain.atk_scale"], 0.85)
            },
            sluggish: number(&tb["attack@sluggish"], 0.5),
        });
        s.hit = HitHandler::Leizi(Hit {
            unblocked_scale: number(&t["atk_scale"], 1.),
        });
        [("atkPct".into(), number(&bb["atk"], 0.))].into()
    };
    s.combat.skill_area = s.combat.area;
    if default_skill {
        s.combat.skill_area.chain.as_mut().unwrap().falloff = 0.;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub unblocked_scale: f64,
}
impl Hit {
    pub fn apply(&self, w: &mut World, q: &mut hit::Request) -> Result<(), String> {
        let unblocked_scale = self.unblocked_scale;
        if w.unit(q.hit.target)?.blocked_by().is_none() {
            q.damage.amount *= unblocked_scale;
        }
        Ok(())
    }
}
