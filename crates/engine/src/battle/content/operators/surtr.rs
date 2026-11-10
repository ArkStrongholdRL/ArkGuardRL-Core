// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "artsfghter",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_surtr_3",
            kind: Kind::Toggle,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_surtr_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_surtr_2",
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
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_surtr_1" => Mode::First,
        "skchr_surtr_2" => Mode::Second,
        "skchr_surtr_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.damage = crate::battle::damage::DamageType::Arts;
    skill_range(raw, s)?;
    if raw["skill"]["trigger"]["customRangeGrid"].is_array() {
        s.trigger_range = Some(crate::battle::operators::parse_range(
            &raw["skill"]["trigger"]["customRangeGrid"],
        )?);
    }
    s.extra.effects.permanent.push(persistent(
        "surtr:magma",
        [(
            "resIgnoreFlat".into(),
            number(&t["magic_resist_penetrate_fixed"], 0.),
        )]
        .into(),
    ));
    if mode == Mode::First {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        let max = number(&bb["max_hp"], 0.);
        if max != 0. {
            s.modifiers
                .insert(if max.abs() > 5. { "hpFlat" } else { "hpPct" }.into(), max);
        }
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 1.);
        s.combat.skill_profile = Some(p);
    }
    if mode != Mode::First {
        s.extra.advanced_range_extend = number(&bb["ability_range_forward_extend"], 0.)
            .round()
            .max(0.) as u32;
    }
    s.extra.advanced = Advanced::Surtr(Surtr {
        mode,
        withdraw: number(&t1["surtr_t_2[withdraw].interval"], 8.),
        drain_interval: number(&bb["interval"], 0.2).max(0.05),
        drain_peak: number(&bb["hp_ratio"], 0.),
        drain_ramp: number(&bb["duration"], 60.).max(0.1),
        solo: number(&bb["attack@surtr_s_2[critical].atk_scale"], 1.),
        module_aspd: number(&tb["attack_speed"], 0.),
        module_fragile: number(&tb["damage_scale"], 1.) - 1.,
    });
    Ok(())
}
