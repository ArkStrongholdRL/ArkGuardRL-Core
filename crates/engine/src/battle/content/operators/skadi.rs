// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "fearless",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_skadi_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_quickattack[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_skadi_2",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t0 = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    s.extra.effects.team = Some(TeamBuff {
        key: "talent:skadi_predator",
        characters: &[
            "char_263_skadi",
            "char_143_ghost",
            "char_218_cuttle",
            "char_474_glady",
            "char_1023_ghost2",
            "char_4145_ulpia",
        ],
        mods: [("atkPct".into(), number(&t0["atk"], 0.))].into(),
    });
    s.extra.effects.respawn_delta = number(&t1["respawn_time"], 0.);
    if !tb["value"].is_null() || !tb["hp_ratio"].is_null() {
        s.extra.effects.revive = Some(Revive {
            hp_mul: if !tb["value"].is_null() {
                1. - number(&tb["value"], 0.)
            } else {
                number(&tb["max_hp"], 0.4)
            }
            .max(0.05),
            aspd: number(&tb["attack_speed"], 0.),
            hp_ratio: number(&tb["hp_ratio"], 1.),
        });
    }
    if !tb["atk_scale"].is_null() {
        s.hit = HitHandler::Blocked {
            scale: number(&tb["atk_scale"], 1.),
            own: false,
            inverse: false,
        };
    }
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        ("defPct".into(), number(&bb["def"], 0.)),
        ("hpPct".into(), number(&bb["max_hp"], 0.)),
        ("aspd".into(), number(&bb["attack_speed"], 0.)),
    ]
    .into();
    if raw["skill"]["skillId"] == "skchr_skadi_2" {
        s.extra.deploy = true;
        s.extra.duration = Some(number(&bb["duration"], 20.));
        s.rule_override = Some(crate::battle::skills::Rule::Never);
    }
    Ok(())
}
