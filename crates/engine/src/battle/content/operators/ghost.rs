// SPDX-License-Identifier: GPL-3.0-or-later
//! Specter: centurion attacks, undying skill, and post-skill stun.
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
        Skill::duration("skchr_ghost_2"),
        Skill::duration("skcom_atk_up[3]"),
    ],
    build,
};
pub(super) fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.combat.profile.hit_all_blocked = true;
    s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    let mut b = Buff {
        key: Some("talent:ghost".into()),
        mods: [
            ("hpPct".into(), number(&t["max_hp"], 0.)),
            (
                "hpRegenRatio".into(),
                number(&t["hp_recovery_per_sec_by_max_hp_ratio"], 0.),
            ),
        ]
        .into(),
        ..Default::default()
    };
    b.runtime.persist = true;
    s.permanent = Some(b);
    if default {
        s.undying = true;
        s.end_stun = number(&bb["stun"], 0.);
    }
    s.hit = HitHandler::Blocked {
        scale: number(&raw["trait"]["bb"]["atk_scale"], 1.),
        own: false,
        inverse: false,
    };
    Ok(())
}
