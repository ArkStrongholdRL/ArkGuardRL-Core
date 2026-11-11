// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SUPPORT",
    branch: "craftsman",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_cathy_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_cathy_1",
            kind: Kind::Passive,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    if default {
        s.modifiers = [
            ("hpPct".into(), number(&bb["max_hp"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
        ]
        .into();
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    } else {
        let mods: Numbers = [
            ("atkPct".into(), number(&bb["s1_atk"], 0.)),
            ("defPct".into(), number(&bb["s1_def"], 0.)),
        ]
        .into();
        s.modifiers = mods.clone();
        s.extra.support.forge = Some(mods);
    }
    Ok(())
}
