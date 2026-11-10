// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "physician",
    ranged: true,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_bldsk_1",
            kind: Kind::Charges,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_bldsk_2",
            kind: Kind::Instant,
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
    healing(s, 1.);
    s.extra.support.heal_bonus = Some(HealBonus {
        below: number(&tb["hp_ratio"], 0.),
        scale: number(&tb["heal_scale"], 1.),
        inclusive: true,
    });
    s.extra.support.death_sp = Some([
        number(&t["bldsk_t_1[self].sp"], 2.),
        number(&t["bldsk_t_1[rand].sp"], 2.),
    ]);
    if default {
        s.rule_override = Some(crate::battle::skills::Rule::Never);
        s.extra.support.bandage = Some(number(&bb["hp_ratio"], 0.15));
    } else {
        s.extra.support.plasma = Some(Plasma {
            duration: number(&bb["duration"], 15.),
            interval: number(&bb["interval"], 1.).max(0.1),
            ratio: number(&bb["hp_ratio"], 0.03),
            attack: number(&bb["atk"], 0.),
        });
    }
    Ok(())
}
