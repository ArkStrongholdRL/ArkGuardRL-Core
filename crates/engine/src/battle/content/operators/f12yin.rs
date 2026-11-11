// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "fighter",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_f12yin_2",
            kind: Kind::Toggle,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_f12yin_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_f12yin_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
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
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_f12yin_1" => Mode::First,
        "skchr_f12yin_2" => Mode::Second,
        "skchr_f12yin_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    skill_range(raw, s)?;
    s.extra.effects.permanent.push(persistent(
        "f12yin:body",
        [
            ("defPct".into(), number(&t1["def"], 0.)),
            ("dodgePhys".into(), number(&t1["prob"], 0.)),
        ]
        .into(),
    ));
    if number(&tb["prob"], 0.) > 0. {
        s.extra.effects.permanent.push(persistent(
            "f12yin:moduleX",
            [("dodgePhys".into(), number(&tb["prob"], 0.))].into(),
        ));
    }
    let mut p = s.combat.profile;
    if mode == Mode::First {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        p.max_targets = number(&bb["max_target"], 2.);
    } else {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
            ("blockCnt".into(), number(&bb["block_cnt"], 0.)),
            (
                "hpRegenRatio".into(),
                number(&bb["hp_recovery_per_sec_by_max_hp_ratio"], 0.),
            ),
            (
                "batPct".into(),
                if mode == Mode::Third {
                    flat_bat(raw, number(&bb["base_attack_time"], 0.))
                } else {
                    0.
                },
            ),
        ]
        .into();
    }
    if default {
        p.hit_all_blocked = true;
    }
    if mode == Mode::Third {
        s.combat.skill_hits = 2;
        p.max_targets = number(&bb["attack@max_target"], 3.);
        s.extra.support.radial_push = Some(number(&bb["attack@force"], 1.));
    }
    s.combat.skill_profile = Some(p);
    s.extra.advanced = Advanced::Mountain(Mountain {
        mode,
        probability: number(&t["prob"], 0.),
        active_probability: number(&bb["talent@prob"], number(&t["prob"], 0.)),
        scale: number(&t["atk_scale"], 1.),
        weaken: number(&t["atk"], 0.),
        duration: number(&t["duration"], 3.),
        module_aspd: number(&tb["attack_speed"], 0.),
    });
    Ok(())
}
