// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "slower",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_grabds_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_grabds_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    slow_sp(raw, s);
    s.permanent = Some(persistent(
        "talent:grabds",
        [("aspd".into(), number(&t["attack_speed"], 0.))].into(),
    ));
    s.extra.support.sluggish = Some([
        number(&tb["sluggish"], 0.8),
        number(&t["sluggish_addition"], 0.),
    ]);
    if default {
        s.modifiers = [("aspd".into(), number(&bb["attack_speed"], 0.))].into();
        skill_range(raw, s)?;
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["max_target"], 1.).floor().max(1.);
        s.combat.skill_profile = Some(p);
        s.extra.support.quiet = Some(Quiet {
            duration: number(&bb["sleep"], 0.),
            count: p.max_targets as usize,
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        s.modifiers = [("maxTargets".into(), 1.)].into();
    }
    Ok(())
}
