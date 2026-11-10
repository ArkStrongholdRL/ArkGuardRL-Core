// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "executor",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::None,
            ..Skill::duration("skchr_gravel_2")
        },
        Skill {
            sp_type: SpType::None,
            ..Skill::duration("skchr_gravel_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.extra.deploy = true;
    s.extra.duration = Some(number(&bb["duration"], 10.));
    s.rule_override = Some(crate::battle::skills::Rule::Never);
    s.combat.normal_hits = 2;
    s.combat.split_mul = Some(0.5);
    s.extra.cost_delta = number(&t["cost"], 0.);
    s.extra.start_buff = Some(StartBuff {
        key: if default {
            "gravel:rats"
        } else {
            "gravel:shadow"
        },
        shield: if default {
            number(&bb["hp_ratio"], 0.)
        } else {
            0.
        },
        def: if default { 0. } else { number(&bb["def"], 0.) },
        block: false,
        decay: true,
    });
    if !t["cond.cost"].is_null() && number(&t["def"], 0.) != 0. {
        s.extra.cost_aura = Some(CostAura {
            value: number(&t["def"], 0.),
            limit: number(&t["cond.cost"], 0.),
        });
    }
    Ok(())
}
