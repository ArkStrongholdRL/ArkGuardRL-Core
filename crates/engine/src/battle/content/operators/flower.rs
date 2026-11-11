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
            id: "skchr_flower_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_heal_up[2]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let count = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["hidden"] == true)
        .map(|t| number(&t["bb"]["attack@max_target"], 3.))
        .unwrap_or(3.);
    healing(s, count);
    s.extra.support.lavender = number(&t["atk_to_hp_recovery_ratio"], 0.);
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        ("aspd".into(), number(&bb["attack_speed"], 0.)),
    ]
    .into();
    Ok(())
}
