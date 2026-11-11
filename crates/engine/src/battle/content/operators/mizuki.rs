// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "stalker",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_mizuki_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_mizuki_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_mizuki_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_mizuki_1" => Mode::First,
        "skchr_mizuki_2" => Mode::Second,
        "skchr_mizuki_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.all_in_range = true;
    s.extra.effects.permanent.push(persistent(
        "trait:stalker",
        [
            ("dodgePhys".into(), number(&tb["prob"], 0.5)),
            ("dodgeArts".into(), number(&tb["prob"], 0.5)),
            ("taunt".into(), -1.),
        ]
        .into(),
    ));
    skill_range(raw, s)?;
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        (
            "batPct".into(),
            if mode == Mode::Second {
                flat_bat(raw, number(&bb["base_attack_time"], 0.)).max(-0.9)
            } else {
                0.
            },
        ),
    ]
    .into();
    if mode == Mode::First {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
    }
    s.extra.advanced = Advanced::Mizuki(Mizuki {
        mode,
        scale: number(&t["attack@mizuki_t_1.atk_scale"], 0.5),
        count: number(&t["attack@max_target"], 1.).floor().max(1.) as usize,
        extra: number(&bb["attack@max_target"], 0.).floor().max(0.) as usize,
        skill_scale: number(&bb["talent_scale"], 2.),
        status: number(
            &bb[if mode == Mode::Second {
                "attack@unmovable"
            } else {
                "attack@stun"
            }],
            0.8,
        ),
        loss: number(&bb["attack@hp_ratio"], 0.15),
        threshold: number(&t1["hp_ratio"], 0.5),
        attack: number(&t1["atk"], 0.),
        slow: number(&tm["move_speed"], 0.),
    });
    Ok(())
}
