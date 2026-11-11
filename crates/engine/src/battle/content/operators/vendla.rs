// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "incantationmedic",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_vendla_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_magic_rage[3]",
            kind: Kind::Duration,
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
    s.extra.support.incantation = number(&tb["scale"], 0.5);
    s.extra.support.top_heal = number(&t["heal_scale"], 1.);
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        s.extra.support.protege = Some(Protege {
            taunt: number(&bb["taunt_level"], 1.),
            scale: number(&bb["atk_scale"], 0.),
        });
    } else {
        s.modifiers = [("aspd".into(), number(&bb["attack_speed"], 0.))].into();
    }
    Ok(())
}
