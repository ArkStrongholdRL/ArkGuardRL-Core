// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "shotprotector",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::Hurt,
            attack_override: true,
            ..Skill::duration("skchr_liskam_2")
        },
        Skill {
            sp_type: SpType::Hurt,
            ..Skill::duration("skchr_liskam_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 0)
        .ok_or("Missing Liskarm talent")?;
    let t1 = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 1)
        .map(|t| &t["bb"])
        .unwrap_or(&Value::Null);
    let resistance = number(&t1["magic_resistance"], 0.);
    if resistance != 0. {
        let mut b = Buff {
            key: Some("talent:liskam".into()),
            mods: [("resFlat".into(), resistance)].into(),
            ..Default::default()
        };
        b.runtime.persist = true;
        s.permanent = Some(b);
    }
    s.extra.sp_hurt = Some(SpHurt {
        amount: number(&t["bb"]["sp"], 0.),
        grid: if t["rangeGrid"].is_null() {
            vec![[1, 0], [0, -1], [0, 1], [-1, 0]]
        } else {
            crate::battle::operators::parse_range(&t["rangeGrid"])?
        },
    });
    s.extra.reveal = raw["isGolden"] == true && !raw["module"].is_null();
    if default {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("batPct".into(), number(&bb["base_attack_time"], 0.).max(0.)),
        ]
        .into();
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["attack@max_target"], 1.).floor().max(1.);
        s.combat.skill_profile = Some(p);
        s.combat.skill_damage = Some(crate::battle::damage::DamageType::Arts);
        s.extra.skill_status = Some(OnHitStatus {
            key: "stun",
            duration: number(&bb["attack@stun"], 0.),
            chance: Some(number(&bb["attack@buff_prob"], 0.)),
        });
        s.end_stun = number(&bb["stun"], 0.);
    } else {
        s.extra.duration = Some(number(&bb["duration"], 8.));
        s.modifiers = [("defPct".into(), number(&bb["def"], 0.))].into();
        s.rule_override = Some(crate::battle::skills::Rule::SpFull);
        s.extra.start_buff = Some(StartBuff {
            key: "liskam:block",
            shield: 0.,
            def: 0.,
            block: true,
            decay: false,
        });
    }
    Ok(())
}
