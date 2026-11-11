// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "splashcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_mostma_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
        Skill {
            id: "skcom_atk_up[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_mostma_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let _tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_atk_up[3]" => Mode::First,
        "skchr_mostma_2" => Mode::Second,
        "skchr_mostma_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    module_range(raw, s)?;
    skill_range(raw, s)?;
    s.combat.area.splash = Some(Splash {
        radius: 1.1,
        scale: 1.,
        hits_fly: false,
        fortress_melee: false,
    });
    s.combat.skill_area = s.combat.area;
    s.extra.support.sp_aura = number(&t["sp_recovery_per_sec"], 0.4);
    s.extra.support.sp_profession = Some("CASTER");
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
        let mut p = s.combat.profile;
        p.all_in_range = true;
        s.combat.skill_profile = Some(p);
        s.combat.skill_area.splash = None;
        s.combat.skill_direct = true;
    } else if mode == Mode::First {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    }
    s.extra.advanced = Advanced::Mostma(Mostma {
        lock: mode == Mode::Second,
        ripple: default,
        slow: number(&t1["move_speed"], -0.15),
        slow_scale: number(&bb["talent_scale"], 3.),
        force: number(&bb["attack@force"], 0.),
        lock_scale: number(&bb["atk_scale"], 1.),
    });
    Ok(())
}
