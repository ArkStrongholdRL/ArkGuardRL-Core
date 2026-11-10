// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "PIONEER",
    branch: "pioneer",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_flamtl_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_flamtl_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_flamtl_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let _t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_flamtl_1" => Mode::First,
        "skchr_flamtl_2" => Mode::Second,
        "skchr_flamtl_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])
        .unwrap_or_else(|_| vec![[0, 0], [0, 1]]);
    if mode == Mode::First {
        s.rule_override = Some(crate::battle::skills::Rule::SpFull);
    }
    if mode == Mode::Second {
        skill_range(raw, s)?;
        s.skill_start = Some(SkillStart {
            dp: number(&bb["cost"], 11.),
            area: None,
        });
        s.extra.effects.burst = Some(Burst {
            grid: grid.clone(),
            count: number(&bb["max_target"], 6.).floor().max(1.) as usize,
            scale: number(&bb["atk_scale"], 1.8),
            damage: crate::battle::damage::DamageType::Physical,
            status: Some(OnHitStatus {
                key: "stun",
                duration: number(&bb["stun"], 0.5),
                chance: None,
            }),
            debuff: None,
            pull: None,
            fly_scale: 1.,
            ground: false,
        });
        s.extra.advanced_burst_hits = 2;
    }
    if default {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            (
                "batPct".into(),
                (number(&bb["base_attack_time"], 1.) - 1.).max(-0.9),
            ),
            ("blockCnt".into(), number(&bb["block_cnt"], 1.)),
            ("dodgePhys".into(), number(&bb["prob"], 0.6)),
            ("dodgeArts".into(), number(&bb["prob"], 0.6)),
        ]
        .into();
    }
    s.extra.advanced = Advanced::Flamtl(Flamtl {
        mode,
        total: number(&bb["value"], 8.).floor().max(0.) as u32,
        cost: number(&bb["cost"], if mode == Mode::First { 6. } else { 1. }),
        ally_dodge: number(&bb["flamtl_s_2.prob"], 0.4),
        ally_duration: number(&bb["flamtl_s_2.duration"], 10.),
        faction_dodge: number(&t1["prob"], 0.22),
        grid,
        module: [
            ("atkPct".into(), number(&tb["atk"], 0.)),
            ("defPct".into(), number(&tb["def"], 0.)),
        ]
        .into(),
    });
    Ok(())
}
