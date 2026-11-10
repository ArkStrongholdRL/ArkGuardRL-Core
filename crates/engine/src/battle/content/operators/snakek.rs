// SPDX-License-Identifier: GPL-3.0-or-later
//! Cuora: shell defence, self regeneration, and blocking module.
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "protector",
    can_hit_fly: false,
    ranged: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: Some(&[[0, 0]]),
    skills: &[
        Skill {
            rule: "TAKE_DAMAGE",
            attack_override: true,
            ..Skill::duration("skchr_snakek_2")
        },
        Skill {
            rule: "TAKE_DAMAGE",
            ..Skill::duration("skcom_def_up[2]")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let mut b = Buff {
        key: Some("talent:snakek_def".into()),
        mods: [("defPct".into(), number(&talent(raw)["def"], 0.))].into(),
        ..Default::default()
    };
    b.runtime.persist = true;
    s.permanent = Some(b);
    s.modifiers = [("defPct".into(), number(&bb["def"], 0.))].into();
    if default {
        s.modifiers
            .insert("blockCnt".into(), number(&bb["block_cnt"], 0.));
        s.modifiers.insert(
            "hpRegenRatio".into(),
            number(&bb["hp_recovery_per_sec_by_max_hp_ratio"], 0.),
        );
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    let d = number(&raw["trait"]["bb"]["def"], 0.);
    if d > 0. {
        s.while_blocked = Some(WhileBlocked {
            key: "trait:snakek_block",
            mods: [("defPct".into(), d)].into(),
        });
    }
    Ok(())
}
