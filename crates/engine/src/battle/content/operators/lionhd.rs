// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "splashcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_lionhd_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skcom_atk_up[3]",
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
    let t = talent(raw);
    s.combat.area.splash = Some(Splash {
        radius: 1.1,
        scale: 1.,
        hits_fly: true,
        fortress_melee: false,
    });
    s.combat.skill_area = s.combat.area;
    module_range(raw, s)?;
    s.extra.effects.enemy_atk = Some(EnemyAttack {
        value: number(&t["atk"], 0.04),
        cap: number(&t["max_valid_stack_cnt"], 5.),
    });
    if default {
        skill_range(raw, s)?;
        s.extra.effects.burst = Some(Burst {
            grid: crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?,
            count: usize::MAX,
            scale: number(&bb["atk_scale"], 1.7),
            damage: crate::battle::damage::DamageType::Arts,
            status: None,
            debuff: Some(Debuff {
                key: "lionhd:res",
                stat: "resMul",
                value: number(&bb["magic_resistance"], 0.),
                duration: number(&bb["duration"], 6.),
                strongest: true,
            }),
            pull: None,
            fly_scale: 1.,
            ground: false,
        });
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    }
    Ok(())
}
