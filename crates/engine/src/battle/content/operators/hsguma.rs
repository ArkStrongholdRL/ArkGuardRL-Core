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
            id: "skchr_hsguma_2",
            kind: Kind::Passive,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_hsguma_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_hsguma_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t0 = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    blocked_def(raw, s, "hsguma:module");
    let mut armor = persistent("hsguma:armor", Numbers::new());
    armor.runtime.injury.on_injury = Some(ARMOR_BLOCK);
    armor.runtime.tick_payload = number(&t0["prob"], 0.25);
    s.extra.effects.permanent.push(armor);
    s.extra.effects.aura = Some(StatAura {
        key: "hsguma:def",
        grid: None,
        profession: Some("TANK"),
        mods: [("defPct".into(), number(&t1["def"], 0.06))].into(),
    });
    s.modifiers = [
        ("defPct".into(), number(&bb["def"], 0.)),
        ("atkPct".into(), number(&bb["atk"], 0.)),
    ]
    .into();
    if default {
        s.extra.effects.observer = Some(InjuryObserver {
            key: "hsguma:thorns",
            counter: Some(Counter {
                key: "hsguma:thorns",
                stat: "atk",
                scale: number(&bb["atk_scale"], 0.65),
            }),
            debuff: None,
        });
    } else if raw["skill"]["skillId"] == "skchr_hsguma_3" {
        let mut p = s.combat.profile;
        p.all_in_range = true;
        s.combat.skill_profile = Some(p);
    }
    Ok(())
}
