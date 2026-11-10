// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "charger",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_wildmn_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_wildmn_1",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.dp_on_kill = number(&raw["trait"]["bb"]["cost"], 1.);
    s.extra.effects.cost_cut = Some(CostCut {
        amount: number(&t["value"], -1.).abs(),
        cap: number(&t["max_stack_cnt"], 1.),
        every: number(&t["flag"], 0.) == 1.,
    });
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        skill_range(raw, s)?;
        s.extra.effects.skill_push = Some(number(&bb["attack@force"], 1.));
    } else {
        s.extra.deploy = true;
        s.extra.duration = Some(number(&raw["skill"]["duration"], 0.));
        s.rule_override = Some(crate::battle::skills::Rule::Never);
        s.modifiers = [("aspd".into(), number(&bb["attack_speed"], 0.))].into();
    }
    Ok(())
}
