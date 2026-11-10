// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "splashcaster",
    can_hit_fly: true,
    ranged: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_greyy_2")
        },
        Skill::duration("skcom_magic_rage[2]"),
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.modifiers = {
        // Original Greyy's radius is 1.0 rather than the branch's 1.1.
        s.combat.area.splash = Some(Splash {
            radius: 1.,
            scale: 1.,
            hits_fly: false,
            fortress_melee: false,
        });
        s.combat.sluggish = number(&t["sluggish"], 0.);
        [("aspd".into(), number(&bb["attack_speed"], 0.))].into()
    };
    s.combat.skill_area = s.combat.area;
    s.combat.skill_sluggish = if default_skill {
        s.combat.sluggish * number(&bb["talent_scale"], 1.)
    } else {
        s.combat.sluggish
    };
    Ok(())
}
