// SPDX-License-Identifier: GPL-3.0-or-later
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
            ..Skill::duration("skchr_yak_2")
        },
        Skill {
            rule: "TAKE_DAMAGE",
            ..Skill::duration("skchr_yak_1")
        },
    ],
    build,
};
pub(super) fn build(raw: &Value, default_skill: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.modifiers = {
        let mut buff = Buff {
            key: Some("talent:yak".into()),
            mods: [("resFlat".into(), number(&t["magic_resistance"], 0.))].into(),
            ..Default::default()
        };
        buff.runtime.persist = true;
        s.permanent = Some(buff);
        if !default_skill {
            [
                ("hpPct".into(), number(&bb["max_hp"], 0.)),
                ("hpRegen".into(), number(&bb["hp_recovery_per_sec"], 0.)),
            ]
            .into()
        } else {
            [
                ("hpPct".into(), number(&bb["max_hp"], 0.)),
                ("defPct".into(), number(&bb["def"], 0.)),
                ("resMul".into(), 1. + number(&bb["magic_resistance"], 0.)),
            ]
            .into()
        }
    };
    Ok(())
}
