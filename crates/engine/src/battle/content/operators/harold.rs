// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "wandermedic",
    ranged: true,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_harold_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_heal_up[3]",
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
    let _t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let _mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_heal_up[3]" => Mode::First,
        "skchr_harold_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    healing(s, 1.);
    if let Some(h) = &mut s.combat.profile.heal {
        h.element_heal = true;
    }
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        ("aspd".into(), number(&bb["attack_speed"], 0.)),
    ]
    .into();
    s.extra.advanced = Advanced::Harold(Harold {
        severe: default,
        element_ratio: number(&tb["ep_heal_ratio"], 0.5),
        extra_scale: number(&bb["trait_scale"], 1.),
        resistance: number(&t["ep_damage_resistance"], 0.).clamp(0., 1.),
    });
    Ok(())
}
