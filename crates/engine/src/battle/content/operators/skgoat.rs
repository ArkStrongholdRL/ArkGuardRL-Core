// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "slower",
    can_hit_fly: true,
    ranged: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[Skill::duration("skcom_atk_up[2]")],
    build,
};
pub(super) fn build(raw: &Value, _default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    let tb = &raw["trait"]["bb"];
    s.modifiers = {
        s.combat.sluggish = number(&tb["sluggish"], 0.8) + number(&t["sluggish"], 0.);
        [("atkPct".into(), number(&bb["atk"], 0.))].into()
    };
    s.combat.skill_sluggish = s.combat.sluggish;
    Ok(())
}
