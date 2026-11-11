// SPDX-License-Identifier: GPL-3.0-or-later
//! Executor: all-in-range volleys, front-row multiplier, and fixed DEF ignore.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "reaperrange",
    can_hit_fly: true,
    ranged: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_excu_2")
        },
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_excu_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let mut buff = Buff {
        key: Some("talent:excu".into()),
        mods: [(
            "defIgnoreFlat".into(),
            number(&talent(raw)["def_penetrate_fixed"], 0.),
        )]
        .into(),
        ..Default::default()
    };
    buff.runtime.persist = true;
    s.permanent = Some(buff);
    s.combat.profile.all_in_range = true;
    s.grid_scale = Some(GridScale {
        grid: crate::battle::operators::parse_range(&raw["trait"]["rangeGrid"])?,
        scale: number(&raw["trait"]["bb"]["atk_scale"], 1.5),
        skill_all: !default_skill,
    });
    if default_skill {
        s.combat.skill_hits = 2;
        let bat = number(&raw["stats"]["bat"], 1.);
        s.modifiers = [(
            "batPct".into(),
            (number(&bb["base_attack_time"], 0.) / if bat == 0. { 1. } else { bat }).max(-0.9),
        )]
        .into();
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    }
    Ok(())
}
