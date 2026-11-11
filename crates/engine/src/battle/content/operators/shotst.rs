// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "fastshot",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_shotst_2",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_shotst_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    fastshot(raw, s);
    s.extra.effects.fly_scale *= number(&talent(raw)["atk_scale"], 1.);
    let debuff = Debuff {
        key: "skill:shotst_shred",
        stat: "defPct",
        value: number(&bb["def"], 0.),
        duration: number(&bb["duration"], 5.),
        strongest: false,
    };
    if default {
        s.extra.effects.burst = Some(Burst {
            grid: crate::battle::operators::parse_range(&raw["rangeGrid"])?,
            count: 5,
            scale: number(&bb["atk_scale"], 1.),
            damage: crate::battle::damage::DamageType::Physical,
            status: None,
            debuff: Some(debuff),
            pull: None,
            fly_scale: s.extra.effects.fly_scale,
            ground: false,
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        s.extra.effects.skill_debuff = Some(debuff);
    }
    Ok(())
}
