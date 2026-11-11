// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "lord",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_svrash_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_svrash_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_svrash_2",
            kind: Kind::Toggle,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    s.permanent = Some(persistent(
        "svrash:t1",
        [("atkPct".into(), number(&t["atk"], 0.1))].into(),
    ));
    s.extra.effects.team = Some(TeamBuff {
        key: "svrash:leader",
        characters: &[],
        mods: [(
            "redeployMul".into(),
            (1. + number(&t["respawn_time"], -0.1)).max(0.),
        )]
        .into(),
    });
    s.extra.support.reveal = true;
    s.extra.lord = Some(Lord {
        scale: number(&tb["atk_scale"], 0.8),
        arts: number(&tb["atk_scale_m"], 0.),
        skill_full: default,
    });
    s.extra.effects.lord_skill_scale = number(&bb["atk_scale"], 1.);
    if number(&tb["atk_scale_m"], 0.) > 0. {
        s.extra.support.elite_scale = Some(number(&t["atk_scale"], 1.));
    }
    skill_range(raw, s)?;
    if default {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
        ]
        .into();
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 4.).floor().max(1.);
        s.combat.skill_profile = Some(p);
        s.combat.skill_direct = true;
    } else if raw["skill"]["skillId"] == "skchr_svrash_1" {
        s.combat.skill_scale = number(&bb["atk_scale"], 2.05);
    } else {
        s.modifiers = [
            ("defPct".into(), number(&bb["def"], 0.)),
            (
                "hpRegenRatio".into(),
                number(&bb["hp_recovery_per_sec_by_max_hp_ratio"], 0.),
            ),
        ]
        .into();
    }
    Ok(())
}
