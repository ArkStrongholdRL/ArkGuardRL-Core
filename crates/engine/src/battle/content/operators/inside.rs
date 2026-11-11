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
            id: "skchr_inside_2",
            kind: Kind::Ammo,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_inside_1",
            kind: Kind::Ammo,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    fastshot(raw, s);
    s.extra.effects.ammo = Some(AmmoSupport {
        delay: number(&t["duration"], 0.),
        own: number(&t["self_ammo"], 0.),
        ally: number(&t["ally_ammo"], 0.),
    });
    s.extra.effects.ammo_count = number(&bb["attack@trigger_time"], 1.).max(1.);
    if default {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            (
                "batPct".into(),
                flat_bat(raw, number(&bb["base_attack_time"], 0.)),
            ),
            ("taunt".into(), -1.),
        ]
        .into();
        let mut p = s.combat.profile;
        p.priority = crate::battle::targeting::Priority::Ranged;
        s.combat.skill_profile = Some(p);
    } else {
        s.combat.skill_scale = number(&bb["attack@atk_scale"], 1.);
    }
    Ok(())
}
