// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "ringhealer",
    ranged: true,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_plosis_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skcom_heal_up[3]",
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
    healing(s, 3.);
    module_range(raw, s)?;
    s.extra.support.sp_aura = number(&t["sp_recovery_per_sec"], 0.3);
    if default {
        skill_range(raw, s)?;
        s.modifiers = [(
            "batPct".into(),
            flat_bat(raw, number(&bb["base_attack_time"], 0.)),
        )]
        .into();
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    }
    Ok(())
}
