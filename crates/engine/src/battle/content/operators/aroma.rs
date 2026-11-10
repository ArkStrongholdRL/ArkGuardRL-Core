// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "blastcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_aroma_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_aroma_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let _t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let _mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_aroma_1" => Mode::First,
        "skchr_aroma_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.range_aoe = true;
    s.combat.profile.all_in_range = true;
    s.combat.projectile = None;
    if default {
        s.modifiers = [("atkPct".into(), number(&bb["atk"], 0.))].into();
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
    }
    if number(&tb["damage_scale"], 0.) > 0. {
        s.extra.effects.distance = Some(DistanceScale {
            minimum: number(&tb["min_dist"], 0.),
            maximum: number(&tb["max_dist"], 4.),
            attack_scale: 0.,
            damage_scale: number(&tb["damage_scale"], 0.),
        });
    }
    s.extra.advanced = Advanced::Aroma(Aroma {
        landing: default,
        first_scale: number(&t["damage_scale"], 1.1),
        levitate: number(&t["levitate_duration"], 2.5),
        land_scale: number(&bb["attack@atk_scale_when_fly_finish"], 0.65),
        fly_scale: number(&bb["atk_scale_to_fly"], 0.55),
    });
    Ok(())
}
