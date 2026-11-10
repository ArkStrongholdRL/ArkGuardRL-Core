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
            id: "skchr_kroos2_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_kroos2_1",
            kind: Kind::Duration,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    fastshot(raw, s);
    let t = talent(raw);
    s.extra.effects.crit = Some(Critical {
        chance: number(&t["prob"], 0.2),
        attack_scale: 1.,
        damage_scale: number(&t["atk_scale"], 1.5),
        stun: number(&t["stun"], 0.2),
    });
    s.combat.skill_hits = 2;
    if default {
        s.modifiers = [(
            "batPct".into(),
            flat_bat(raw, number(&bb["base_attack_time"], 0.)),
        )]
        .into();
        s.extra.effects.hit_upgrade =
            Some(number(&bb["attack@max_stack_count"], 40.).max(0.) as u64);
    } else {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        s.skill_flags.insert("camou".into());
    }
    Ok(())
}
