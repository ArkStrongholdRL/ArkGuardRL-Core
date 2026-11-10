// SPDX-License-Identifier: GPL-3.0-or-later
//! Swire: melee-operator ATK aura, skill amplification and expanded talent range.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "instructor",
    can_hit_fly: false,
    ranged: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::Attack,
            ..Skill::duration("skchr_swire_2")
        },
        Skill {
            rule: "SKILL_RANGE",
            ..Skill::duration("skchr_swire_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 0)
        .ok_or("Missing Swire talent")?;
    let grid = if t["rangeGrid"].is_null() {
        vec![
            [1, -1],
            [1, 0],
            [1, 1],
            [0, -1],
            [0, 0],
            [0, 1],
            [-1, -1],
            [-1, 0],
            [-1, 1],
        ]
    } else {
        crate::battle::operators::parse_range(&t["rangeGrid"])?
    };
    let skill_grid = if number(&bb["talent_range_flag"], 0.) > 0. {
        Some(crate::battle::operators::parse_range(
            &raw["skill"]["rangeGrid"],
        )?)
    } else {
        None
    };
    if !default {
        s.trigger_range = Some(crate::battle::operators::parse_range(
            &raw["skill"]["rangeGrid"],
        )?);
    }
    s.aura = Some(Aura {
        key: "talent:swire_guide",
        grid,
        skill_grid,
        atk: number(&t["bb"]["atk"], 0.),
        skill_scale: number(&bb["talent_scale"], 1.),
    });
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    }
    s.hit = HitHandler::Blocked {
        scale: number(&raw["trait"]["bb"]["atk_scale"], 1.2),
        own: true,
        inverse: true,
    };
    Ok(())
}
