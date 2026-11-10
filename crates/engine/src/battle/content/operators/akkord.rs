// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "blastcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_akkord_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_akkord_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    let tb = &raw["trait"]["bb"];
    s.combat.profile.range_aoe = true;
    s.combat.profile.all_in_range = true;
    s.combat.projectile = None;
    s.extra.effects.condition = Some(Conditional {
        key: "akkord:sync",
        when: Predicate::AllyInRange,
        mods: [("atkPct".into(), number(&t["atk"], 0.))].into(),
    });
    s.extra.effects.distance = Some(DistanceScale {
        minimum: number(&tb["min_dist"], 0.),
        maximum: number(&tb["max_dist"], 4.),
        attack_scale: number(&tb["damage_scale"], 0.),
        damage_scale: 0.,
    });
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        s.extra.effects.secondary = Some(Secondary::Sonic {
            radius: number(&bb["attack@range_radius"], 0.9),
            scale: number(&bb["attack@aoe_atk_scale"], 0.),
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
    }
    Ok(())
}
