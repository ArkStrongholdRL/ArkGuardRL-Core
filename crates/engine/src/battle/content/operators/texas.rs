// SPDX-License-Identifier: GPL-3.0-or-later
//! Texas: battle-start DP, instant charge, and two-hit sword rain.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "pioneer",
    can_hit_fly: false,
    ranged: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            kind: Kind::Instant,
            rule: "SKILL_RANGE",
            ..Skill::duration("skchr_texas_2")
        },
        Skill {
            kind: Kind::Instant,
            ..Skill::duration("skcom_charge_cost[3]")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    s.initial_dp = number(&talent(raw)["cost"], 0.).max(0.);
    let area = if default_skill {
        let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?;
        s.trigger_range = Some(grid.clone());
        Some(AreaCast {
            grid,
            scale: number(&bb["atk_scale"], 0.),
            hits: 2,
            stun: number(&bb["stun"], 0.),
        })
    } else {
        s.rule_override = Some(crate::battle::skills::Rule::SpFull);
        None
    };
    s.skill_start = Some(SkillStart {
        dp: number(&bb["cost"], 0.),
        area,
    });
    Ok(())
}
