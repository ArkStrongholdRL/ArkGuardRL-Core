// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "fastshot",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_angel_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_angel_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_angel_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    fastshot(raw, s);
    let t0 = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    s.extra.effects.permanent.push(persistent(
        "talent:angel_mag",
        [("aspd".into(), number(&t0["attack_speed"], 0.))].into(),
    ));
    let bless: Numbers = [
        ("atkPct".into(), number(&t1["atk"], 0.)),
        ("hpPct".into(), number(&t1["max_hp"], 0.)),
    ]
    .into();
    s.extra
        .effects
        .permanent
        .push(persistent("talent:angel_bless", bless.clone()));
    s.extra.effects.random_bless = Some(bless);
    let id = raw["skill"]["skillId"].as_str().unwrap_or("");
    s.combat.skill_scale = number(
        &bb[if id == "skchr_angel_1" {
            "atk_scale"
        } else {
            "attack@atk_scale"
        }],
        1.,
    );
    s.combat.skill_hits = number(
        &bb[if id == "skchr_angel_1" {
            "times"
        } else {
            "attack@times"
        }],
        1.,
    )
    .floor()
    .max(1.) as u32;
    if default {
        s.modifiers = [("batPct".into(), number(&bb["base_attack_time"], 0.))].into();
    }
    Ok(())
}
