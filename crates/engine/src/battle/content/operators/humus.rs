// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "reaper",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_humus_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_humus_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    s.extra.no_heal = true;
    s.combat.profile.all_in_range = true;
    s.extra.support.reaper = number(&tb["value"], 50.);
    s.extra.support.overflow = number(&t["max_hp_ratio"], 0.);
    if default {
        s.modifiers = [("blockCnt".into(), number(&bb["block_cnt"], 0.))].into();
        let mut peaks = Vec::new();
        for (key, value) in bb.as_object().into_iter().flatten() {
            if key.ends_with(".peak_performance.hp_ratio") {
                let atk = key.replace(".hp_ratio", ".atk");
                peaks.push([number(value, 1.), number(&bb[&atk], 0.)]);
            }
        }
        peaks.sort_by(|a, b| b[0].total_cmp(&a[0]));
        s.extra.support.peaks = peaks;
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        s.extra.support.skill_heal = number(&bb["value"], 0.);
    }
    Ok(())
}
