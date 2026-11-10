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
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_slbell_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_slbell_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    debuffer(raw, s);
    let t1 = talent_at(raw, 1);
    s.combat.profile.max_targets = number(&t1["attack@max_target"], 1.).floor().max(1.);
    s.extra.support.enemy_auras.push(EnemyAura {
        key: "talent:weak_fragile",
        mods: [("dmgTakenMul".into(), number(&t["damage_scale"], 1.))].into(),
        filter: AuraFilter::LowHp(number(&t["hp_ratio"], 0.4)),
        period: 0.1,
        active_only: false,
        growth: None,
    });
    let mods = if default {
        let mr = number(&bb["magic_resistance"], 0.);
        [
            ("defPct".into(), number(&bb["def"], 0.)),
            (
                if mr.abs() < 1. {
                    "resMul".into()
                } else {
                    "resFlat".into()
                },
                if mr.abs() < 1. { (1. + mr).max(0.) } else { mr },
            ),
        ]
        .into()
    } else {
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], number(&bb["max_target"], 1.))
            .floor()
            .max(1.);
        s.combat.skill_profile = Some(p);
        [("aspd".into(), number(&bb["attack_speed"], 0.))].into()
    };
    s.extra.support.enemy_auras.push(EnemyAura {
        key: if default {
            "skill:slbell_shred"
        } else {
            "skill:slbell_aspd"
        },
        mods,
        filter: AuraFilter::All,
        period: 0.,
        active_only: true,
        growth: None,
    });
    Ok(())
}
