// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "musha",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::None,
            ..Skill::duration("skchr_utage_2")
        },
        Skill {
            attack_override: true,
            ..Skill::duration("skchr_utage_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.extra.no_heal = true;
    s.extra.self_heal = number(&raw["trait"]["bb"]["value"], 50.);
    s.extra.hp_speed = Some(HpSpeed {
        maximum: number(&t["min_attack_speed"], 0.),
        minimum: number(&t["min_hp_ratio"], 0.),
        protect: number(&t["damage_resistance"], 0.),
        below: number(&t["hp_ratio"], 0.),
    });
    if default {
        s.extra.deploy = true;
        s.extra.duration = Some(number(&bb["duration"], 0.));
        s.extra.hp_loss = number(&bb["hp_ratio"], 0.);
        s.extra.skill_arts = true;
        s.rule_override = Some(crate::battle::skills::Rule::Never);
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    } else {
        s.modifiers = [
            ("defPct".into(), number(&bb["def"], 0.)),
            (
                "hpRegenRatio".into(),
                number(&bb["hp_recovery_per_sec_by_max_hp_ratio"], 0.),
            ),
            ("blockCnt".into(), -99.),
        ]
        .into();
        s.extra.release_on_start = true;
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    Ok(())
}
