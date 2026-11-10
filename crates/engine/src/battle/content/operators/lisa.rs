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
            id: "skchr_lisa_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_lisa_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_lisa_2",
            kind: Kind::Toggle,
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
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_lisa_1" => Mode::First,
        "skchr_lisa_2" => Mode::Second,
        "skchr_lisa_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.extra.normal_status = Some(OnHitStatus {
        key: "sluggish",
        duration: number(&tb["sluggish"], 0.8),
        chance: None,
    });
    s.extra.support.sp_aura = number(&t["sp_recovery_per_sec"], 0.);
    s.extra.support.sp_period = Some(0.25);
    s.extra.support.sp_duration = Some(0.5);
    s.extra.support.sp_profession = Some("SUPPORT");
    skill_range(raw, s)?;
    if default {
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    } else {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("aspd".into(), number(&bb["attack_speed"], 0.)),
        ]
        .into();
        if mode == Mode::Second {
            let mut p = s.combat.profile;
            p.max_targets = number(&bb["attack@max_target"], 2.);
            s.combat.skill_profile = Some(p);
        }
    }
    s.extra.advanced = Advanced::Lisa(Lisa {
        fox: default,
        fragile: number(&t1["damage_scale"], 1.) - 1.,
        boost: number(&bb["scale_delta_to_one"], 1.),
        regen: number(&bb["attack@atk_to_hp_recovery_ratio"], 0.),
        module_sp: number(&tm["sp_recovery_per_sec"], 0.),
    });
    Ok(())
}
