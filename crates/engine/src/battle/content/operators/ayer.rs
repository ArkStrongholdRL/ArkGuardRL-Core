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
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_ayer_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_ayer_1",
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
    let tb = &raw["trait"]["bb"];
    s.extra.lord = Some(Lord {
        scale: number(&tb["atk_scale"], 0.8),
        arts: number(&tb["atk_scale_m"], 0.),
        skill_full: false,
    });
    s.extra.effects.lord_skill_scale = number(&bb["atk_scale"], 1.);
    let grid = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 0)
        .map(|t| &t["rangeGrid"])
        .unwrap_or(&Value::Null);
    s.extra.effects.aura = Some(StatAura {
        key: "talent:ayer_aspd",
        grid: Some(if grid.is_null() {
            (-1..=1)
                .flat_map(|r| (-1..=1).map(move |c| [r, c]))
                .collect()
        } else {
            crate::battle::operators::parse_range(grid)?
        }),
        profession: None,
        mods: [("aspd".into(), number(&talent(raw)["attack_speed"], 0.))].into(),
    });
    s.combat.skill_damage = Some(crate::battle::damage::DamageType::Arts);
    if default {
        skill_range(raw, s)?;
        s.combat.skill_scale = number(&bb["attack@atk_scale"], 1.);
        s.extra.effects.secondary = Some(Secondary::NearbyBlocked {
            scale: number(&bb["atk_scale"], 1.),
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        let mut p = s.combat.profile;
        p.max_targets = number(&bb["max_target"], 1.).floor().max(1.);
        s.combat.skill_profile = Some(p);
        s.extra.skill_status = Some(OnHitStatus {
            key: "sluggish",
            duration: number(&bb["sluggish"], 0.),
            chance: None,
        });
    }
    Ok(())
}
