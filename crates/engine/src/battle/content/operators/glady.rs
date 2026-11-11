// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "hookmaster",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_glady_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_glady_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_glady_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_glady_1" => Mode::First,
        "skchr_glady_2" => Mode::Second,
        "skchr_glady_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    skill_range(raw, s)?;
    if !default {
        s.combat.skill_scale = number(&bb["attack@atk_scale"], number(&bb["atk_scale"], 1.));
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 1.);
        s.combat.skill_profile = Some(p);
        if mode == Mode::Second {
            s.modifiers = [(
                "batPct".into(),
                flat_bat(raw, number(&bb["base_attack_time"], 0.)).max(-0.9),
            )]
            .into();
        }
    }
    s.extra.advanced = Advanced::Glady(Glady {
        mode,
        force: number(&bb["force"], number(&bb["attack@force"], 0.)),
        interval: number(&bb["interval"], 1.5).max(0.1),
        scale: number(&bb["atk_scale"], 0.85),
        slow: number(&bb["move_speed"], -0.5),
        regen: number(&t["hp_recovery_per_sec_by_max_hp_ratio"], 0.025),
        resistance: number(&t["damage_resistance"], 0.25),
        max_weight: number(&t1["value"], 3.),
        light_scale: number(&t1["atk_scale"], 1.3),
        drag: number(&tb["value"], 0.),
        drag_distance: number(&tb["dist"], 1.).max(1e-6),
        far_radius: number(
            &tm["skill@range_radius"],
            number(&tm["attack@range_radius"], 0.),
        ),
        far_force: number(
            &tm["skill@delta_force"],
            number(&tm["attack@delta_force"], 0.),
        ),
    });
    Ok(())
}
