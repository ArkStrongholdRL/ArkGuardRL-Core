// SPDX-License-Identifier: GPL-3.0-or-later
//! Ashlock: fortress melee/bomb profiles and deployment terrain talent.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "fortress",
    can_hit_fly: false,
    ranged: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill::duration("skcom_atk_up[3]"),
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_ashlok_2")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.combat.profile.fortress = true;
    s.combat.profile.selection.can_hit_fly = false;
    s.combat.profile.selection.ground_only = true;
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.area.splash = Some(Splash {
        radius: 1.,
        scale: 1.,
        hits_fly: false,
        fortress_melee: true,
    });
    s.combat.area.fortress_melee = true;
    s.combat.skill_area = s.combat.area;
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    if !default {
        let bat = number(&raw["stats"]["bat"], 1.);
        s.modifiers.insert(
            "batPct".into(),
            (number(&bb["base_attack_time"], 0.) / if bat == 0. { 1. } else { bat }).max(-0.9),
        );
        s.skill_flags.insert("noBlock".into());
        let mut p = s.combat.profile;
        p.fortress = false;
        s.combat.skill_profile = Some(p);
        s.combat.skill_area.fortress_melee = false;
        s.combat.skill_area.splash.as_mut().unwrap().fortress_melee = false;
    }
    s.ground_bonus = Some(GroundBonus {
        key: "talent:ashlok",
        count: number(&t["cnt"], 4.),
        normal: number(&t["atk"], 0.),
        surrounded: number(&t["ashlok_t_1.atk"], number(&t["atk"], 0.)),
    });
    s.hit = HitHandler::Blocked {
        scale: number(&raw["trait"]["bb"]["atk_scale"], 1.),
        own: false,
        inverse: false,
    };
    Ok(())
}
