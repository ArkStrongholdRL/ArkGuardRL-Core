// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "longrange",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_fartth_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "CUSTOM_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skcom_quickattack[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_fartth_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_quickattack[3]" => Mode::First,
        "skchr_fartth_2" => Mode::Second,
        "skchr_fartth_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    skill_range(raw, s)?;
    s.combat.profile.priority = crate::battle::targeting::Priority::LowDef;
    if raw["skill"]["trigger"]["customRangeGrid"].is_array() {
        s.trigger_range = Some(crate::battle::operators::parse_range(
            &raw["skill"]["trigger"]["customRangeGrid"],
        )?);
    }
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        ("aspd".into(), number(&bb["attack_speed"], 0.)),
        ("taunt".into(), number(&t1["taunt_level"], -1.)),
    ]
    .into();
    if default {
        s.extra.skill_range = Some(
            (0..crate::battle::grid::COLS as i32)
                .map(|c| [0, c])
                .collect(),
        );
    }
    if number(&tb["damage_scale"], 0.) > 0. {
        s.extra.effects.distance = Some(DistanceScale {
            minimum: number(&tb["min_dist"], 1.),
            maximum: number(&tb["max_dist"], 4.5),
            attack_scale: 0.,
            damage_scale: number(&tb["damage_scale"], 0.),
        });
    }
    s.extra.advanced = Advanced::Fartth(Fartth {
        mode,
        focus: number(&t["atk"], 0.15),
        delay: number(&t["delay"], 10.),
        far_scale: number(&bb["damage_scale"], 1.25),
        module_sp: number(&tb["sp"], 0.),
        base_grid: crate::battle::operators::parse_range(&raw["rangeGrid"])?,
    });
    Ok(())
}
