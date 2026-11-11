// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "unyield",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_branch_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_branch_1",
            kind: Kind::Duration,
            sp_type: SpType::Hurt,
            rule: "TAKE_DAMAGE",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    s.extra.no_heal = true;
    s.extra.effects.end_heal = number(&talent(raw)["hp_ratio"], 0.);
    s.extra.effects.blocked_taken = number(&raw["trait"]["bb"]["damage_scale"], 1.);
    s.modifiers = [("defPct".into(), number(&bb["def"], 0.))].into();
    if default {
        s.modifiers.insert("atkPct".into(), number(&bb["atk"], 0.));
        let mut p = s.combat.profile;
        p.hit_all_blocked = true;
        s.combat.skill_profile = Some(p);
        s.extra.effects.start_status = Some((
            crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?,
            OnHitStatus {
                key: "tremble",
                duration: number(&bb["not_combat"], 0.),
                chance: None,
            },
        ));
    } else {
        s.extra.effects.resist = (-number(&bb["one_minus_status_resistance"], 0.)).clamp(0., 0.95);
    }
    Ok(())
}
