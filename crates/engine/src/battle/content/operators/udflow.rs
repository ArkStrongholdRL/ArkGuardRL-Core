// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "shotprotector",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            rule: "ACTIVE_RANGE",
            attack_override: true,
            ..Skill::duration("skchr_udflow_2")
        },
        Skill {
            rule: "TAKE_DAMAGE",
            ..Skill::duration("skchr_udflow_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.extra.dot = Some(Dot {
        duration: number(&t["duration"], 0.),
        interval: number(&t["interval"], 1.),
        amount: number(&t["damage"], 0.),
        sea: number(&t["damage_seamonster"], number(&t["damage"], 0.) * 2.),
    });
    s.extra.reveal = raw["isGolden"] == true && !raw["module"].is_null();
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    if default {
        s.modifiers
            .insert("aspd".into(), number(&bb["attack_speed"], 0.));
        let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?;
        s.trigger_range = Some(grid.clone());
        s.extra.skill_range = Some(grid);
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 1.).max(1.);
        s.combat.skill_profile = Some(p);
        s.extra.skill_status = Some(OnHitStatus {
            key: "sluggish",
            duration: number(&bb["attack@sluggish"], 1.),
            chance: None,
        });
    } else {
        s.modifiers.insert("defPct".into(), number(&bb["def"], 0.));
    }
    Ok(())
}
