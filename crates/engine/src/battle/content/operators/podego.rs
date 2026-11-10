// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "slower",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_podego_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_podego_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    slow_sp(raw, s);
    s.extra.normal_status = Some(OnHitStatus {
        key: "sluggish",
        duration: number(&tb["sluggish"], 0.8),
        chance: None,
    });
    s.extra.support.support_atk = number(&t["atk"], 0.);
    if default {
        s.extra.support.zone = Some(Zone {
            duration: number(&bb["projectile_delay_time"], 5.),
            scale: number(&bb["atk_scale"], 0.),
        });
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        skill_healing(s, 1., 1.);
        s.extra.support.auto_heal = true;
    }
    Ok(())
}
