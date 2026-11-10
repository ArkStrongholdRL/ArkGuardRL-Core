// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "hookmaster",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_slchan_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_slchan_1",
            kind: Kind::Instant,
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
    s.extra.effects.condition = Some(Conditional {
        key: "slchan:hunter",
        when: Predicate::Unblocked,
        mods: [
            ("atkPct".into(), number(&t["atk"], 0.)),
            ("defPct".into(), number(&t["def"], 0.)),
        ]
        .into(),
    });
    let pull = PullHit {
        force: number(&bb["force"], 1.),
        travel_damage: number(&tb["value"], 0.),
        distance: number(&tb["dist"], 1.).max(0.01),
    };
    if default {
        let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?;
        s.trigger_range = Some(grid.clone());
        s.extra.effects.burst = Some(Burst {
            grid,
            count: number(&bb["max_target"], 1.).max(1.) as usize,
            scale: number(&bb["atk_scale"], 1.),
            damage: crate::battle::damage::DamageType::True,
            status: Some(OnHitStatus {
                key: "stun",
                duration: number(&bb["stun"], 0.),
                chance: None,
            }),
            debuff: None,
            pull: Some(pull),
            fly_scale: 1.,
            ground: false,
        });
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        s.combat.skill_damage = Some(crate::battle::damage::DamageType::Physical);
        s.extra.effects.skill_pull = Some(PullHit {
            travel_damage: 0.,
            ..pull
        });
    }
    Ok(())
}
