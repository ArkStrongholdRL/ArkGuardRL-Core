// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "slower",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_aglina_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_aglina_1",
            kind: Kind::Duration,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_aglina_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SEARCH",
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
        "skchr_aglina_1" => Mode::First,
        "skchr_aglina_2" => Mode::Second,
        "skchr_aglina_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.extra.normal_status = Some(OnHitStatus {
        key: "sluggish",
        duration: number(&tb["sluggish"], 0.8),
        chance: None,
    });
    skill_range(raw, s)?;
    s.combat.profile.only_while_skill = mode != Mode::First;
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    if mode == Mode::Second {
        let bat = number(&bb["base_attack_time"], 1.);
        s.modifiers.insert(
            "batPct".into(),
            if bat > 0. && bat < 1. {
                bat - 1.
            } else {
                flat_bat(raw, bat)
            },
        );
        s.combat.skill_scale = number(&bb["damage_scale"], 1.);
    }
    if default {
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 1.);
        s.combat.skill_profile = Some(p);
    }
    s.extra.advanced = Advanced::Aglina(Aglina {
        weightless: default,
        aspd: number(&t["attack_speed"], 0.),
        regen: number(&t1["hp_recovery_per_sec"], 0.),
        module_sp: number(&tm["sp_recovery_per_sec"], 0.),
    });
    Ok(())
}
