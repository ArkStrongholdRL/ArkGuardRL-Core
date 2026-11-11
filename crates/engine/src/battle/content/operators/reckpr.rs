// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "physician",
    ranged: true,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_reckpr_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_reckpr_1",
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
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let _t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let _mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_reckpr_1" => Mode::First,
        "skchr_reckpr_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    healing(s, 1.);
    s.extra.support.heal_bonus = Some(HealBonus {
        below: number(&tb["hp_ratio"], 0.),
        scale: number(&tb["heal_scale"], 1.),
        inclusive: false,
    });
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    } else {
        skill_healing(
            s,
            number(&bb["max_target"], 2.),
            number(&bb["heal_scale"], 1.),
        );
    }
    s.extra.advanced = Advanced::Reckpr(Reckpr {
        guard: default,
        duration: number(&bb["attack@buff_duration"], 10.),
        heal: number(&bb["attack@fixed_heal_value"], 0.),
        probability: number(&t["prob"], 1.),
        sp: number(&t["sp"], 0.),
        aspd: number(&t["attack_speed"], 0.),
        learn_duration: number(&t["duration"], 8.),
    });
    Ok(())
}
