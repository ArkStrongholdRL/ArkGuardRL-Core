// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "protector",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_bubble_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
        Skill {
            id: "skcom_def_up[2]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent(raw);
    s.modifiers = [("defPct".into(), number(&bb["def"], 0.))].into();
    s.extra.effects.observer = Some(InjuryObserver {
        key: "bubble:spikeObserver",
        counter: None,
        debuff: Some(Debuff {
            key: "bubble:spike",
            stat: "atkPct",
            value: number(&t["atk"], 0.),
            duration: number(&t["duration"], 5.),
            strongest: false,
        }),
    });
    blocked_def(raw, s, "bubble:guard");
    if default {
        s.modifiers
            .insert("taunt".into(), number(&bb["taunt_level"], 0.));
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
        s.extra.effects.skill_counter = Some(Counter {
            key: "bubble:counter",
            stat: "def",
            scale: number(&bb["atk_scale"], 0.),
        });
    }
    Ok(())
}
