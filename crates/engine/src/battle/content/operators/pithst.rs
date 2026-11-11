// SPDX-License-Identifier: GPL-3.0-or-later
//! Pithst: ordered neural/burn/apoptosis attachment after real HP damage.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "ritualist",
    can_hit_fly: true,
    ranged: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[Skill {
        sp_type: SpType::Attack,
        attack_override: true,
        ..Skill::duration("skchr_pithst_1")
    }],
    build,
};
pub(super) fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.modifiers = [("aspd".into(), number(&bb["attack_speed"], 0.))].into();
    s.combat.profile.priority = crate::battle::targeting::Priority::NotBurst;
    let mut p = s.combat.profile;
    p.max_targets = number(&bb["attack@max_target"], 1.).floor().max(1.);
    s.combat.skill_profile = Some(p);
    let normal = number(&t["ep_damage_ratio"], 0.);
    s.element_attach = Some(ElementAttach {
        normal,
        leader: number(&t["ep_damage_ratio_boss"], normal),
    });
    Ok(())
}
