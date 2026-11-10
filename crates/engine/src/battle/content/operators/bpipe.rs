// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "charger",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_bpipe_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skcom_quickattack[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_bpipe_3",
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
    let t0 = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    s.dp_on_kill = number(&tb["cost"], 1.);
    s.extra.effects.attack_boost = Some(AttackBoost {
        chance: number(&t0["prob"], 0.25),
        scale: number(&t0["atk_scale"], 1.3),
    });
    s.extra.effects.deploy_sp = number(&t1["sp"], 6.);
    if !tb["hp_ratio"].is_null() {
        s.hit = HitHandler::Vigna(super::vigna::Hit {
            below: number(&tb["hp_ratio"], 0.),
            scale: number(&tb["atk_scale"], 1.),
        });
    }
    if default {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.45);
        s.combat.skill_hits = 2;
    } else {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
            ("aspd".into(), number(&bb["attack_speed"], 0.)),
            ("blockCnt".into(), number(&bb["block_cnt"], 0.)),
            (
                "batPct".into(),
                flat_bat(raw, number(&bb["base_attack_time"], 0.)),
            ),
        ]
        .into();
        if raw["skill"]["skillId"] == "skchr_bpipe_3" {
            s.combat.skill_hits = 3;
        }
    }
    Ok(())
}
