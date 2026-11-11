// SPDX-License-Identifier: GPL-3.0-or-later
//! Estelle: centurion attacks, self-heal on nearby death, and CEN-Y resistance.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "centurion",
    can_hit_fly: false,
    ranged: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            sp_type: SpType::Hurt,
            ..Skill::duration("skchr_estell_2")
        },
        Skill::duration("skcom_atk_up[2]"),
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 0);
    let tb = &raw["trait"]["bb"];
    s.combat.profile.hit_all_blocked = true;
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    if default_skill {
        s.skill_flags.insert("noHeal".into());
    }
    s.death_heal = Some(DeathHeal {
        ratio: number(&t.map(|t| &t["bb"]["hp_ratio"]).unwrap_or(&Value::Null), 0.),
        grid: t
            .map(|t| &t["rangeGrid"])
            .filter(|g| !g.is_null())
            .map(crate::battle::operators::parse_range)
            .transpose()?,
    });
    if !tb["damage_resistance"].is_null() {
        s.incoming_hit = Some(PhysicalResistance {
            above_hp: number(&tb["hp_ratio"], 0.),
            fraction: number(&tb["damage_resistance"], 0.),
        });
    }
    Ok(())
}
