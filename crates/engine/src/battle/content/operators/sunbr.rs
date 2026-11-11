// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "guardian",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_sunbr_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_sunbr_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    if number(&t["prob"], 0.) > 0. {
        s.extra.effects.crit = Some(Critical {
            chance: number(&t["prob"], 0.),
            attack_scale: number(&t["atk_scale"], 1.),
            damage_scale: 1.,
            stun: number(&t["stun"], 0.),
        });
    }
    s.extra.support.heal_bonus = Some(HealBonus {
        below: number(&tb["hp_ratio"], 0.),
        scale: number(&tb["heal_scale"], 1.),
        inclusive: false,
    });
    skill_healing(
        s,
        1.,
        if default {
            number(&bb["heal_scale"], 1.)
        } else {
            1.
        },
    );
    skill_range(raw, s)?;
    if default {
        s.rule_override = Some(crate::battle::skills::Rule::SkillRange);
    } else {
        let cook = number(&bb["disarm"], 10.).max(0.);
        s.extra.duration = Some(cook + number(&raw["skill"]["duration"], 30.).max(0.));
        s.modifiers = [("batPct".into(), number(&bb["base_attack_time"], 0.).max(0.))].into();
        s.extra.support.cook = Some(Cook {
            duration: cook,
            defence: number(&bb["def"], 0.),
            attack: number(&bb["atk"], 0.),
        });
    }
    Ok(())
}
