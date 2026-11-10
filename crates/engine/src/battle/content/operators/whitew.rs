// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "lord",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::Attack,
            attack_override: true,
            ..Skill::duration("skchr_whitew_2")
        },
        Skill {
            kind: Kind::Toggle,
            sp_type: SpType::Attack,
            ..Skill::duration("skchr_whitew_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    if default {
        let mut p = s.combat.profile;
        p.max_targets = 2.;
        s.combat.skill_profile = Some(p);
        s.combat.skill_damage = Some(crate::battle::damage::DamageType::Arts);
    } else {
        s.modifiers
            .insert("dodgePhys".into(), number(&bb["prob"], 0.));
    }
    s.extra.lord = Some(Lord {
        scale: number(&tb["atk_scale"], 0.8),
        arts: number(&tb["atk_scale_m"], 0.),
        skill_full: default,
    });
    s.extra.normal_status = Some(OnHitStatus {
        key: "silence",
        duration: number(&talent(raw)["duration"], 0.),
        chance: None,
    });
    Ok(())
}
