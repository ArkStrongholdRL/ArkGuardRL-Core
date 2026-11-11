// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "underminer",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_gnosis_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_gnosis_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_gnosis_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
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
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_gnosis_1" => Mode::First,
        "skchr_gnosis_2" => Mode::Second,
        "skchr_gnosis_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    debuffer(raw, s);
    if mode == Mode::First {
        s.combat.skill_hits = 2;
        s.combat.skill_scale = number(&bb["atk_scale"], 1.35);
    }
    if mode == Mode::Third {
        s.modifiers = [("aspd".into(), number(&bb["attack_speed"], 0.))].into();
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["max_target"], 2.);
        s.combat.skill_profile = Some(p);
    }
    s.extra.advanced = Advanced::Gnosis(Gnosis {
        mode,
        cold: number(&t["cold"], 1.),
        burst_cold: number(&bb["cold"], 2.5),
        burst_scale: number(&bb["atk_scale"], 1.3),
        fragile_cold: number(&t["damage_scale_cold"], 1.25),
        fragile_frozen: number(&t["damage_scale_freeze"], 1.5),
        module_sp: number(&tb["sp_recovery_per_sec"], 0.),
        resist_delay: number(&t1["interval"], 10.),
        resist: (-number(&t1["one_minus_status_resistance"], -0.5)).clamp(0., 1.),
    });
    Ok(())
}
