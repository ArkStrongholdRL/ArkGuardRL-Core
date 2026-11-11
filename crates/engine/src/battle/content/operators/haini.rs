// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "underminer",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_haini_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_haini_1",
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
    let t = talent_at(raw, 0);
    debuffer(raw, s);
    s.extra.support.enemy_auras.push(EnemyAura {
        key: "talent:haini_fragile",
        mods: [("dmgTakenMul".into(), number(&t["damage_scale"], 1.))].into(),
        filter: AuraFilter::Normal,
        period: 0.2,
        active_only: false,
        growth: Some([
            number(&bb["attack@talent_up"], 0.),
            number(&bb["attack@max_talent_up"], 1.),
        ]),
    });
    let mut p = s.combat.profile;
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        p.max_targets = number(&bb["attack@max_target"], 1.).floor().max(1.);
        s.extra.support.enemy_auras.push(EnemyAura {
            key: "skill:haini_slow",
            mods: [(
                "moveMul".into(),
                (1. + number(&bb["attack@move_speed"], 0.)).max(0.),
            )]
            .into(),
            filter: AuraFilter::Normal,
            period: 0.,
            active_only: true,
            growth: None,
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        p.max_targets = number(&bb["max_target"], 1.).floor().max(1.);
    }
    s.combat.skill_profile = Some(p);
    Ok(())
}
