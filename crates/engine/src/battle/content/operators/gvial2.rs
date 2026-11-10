// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "centurion",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_gvial2_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_gvial2_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_gvial2_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let tb = &raw["trait"]["bb"];
    let t1 = talent_at(raw, 1);
    s.combat.profile.hit_all_blocked = true;
    s.extra.support.block_stats = Some([
        number(&t["atk"], 0.1),
        number(&t["def"], 0.1),
        number(&t["atk_add"], 0.04),
        number(&t["def_add"], 0.04),
    ]);
    s.extra.support.received_heal = Some(ReceivedHeal {
        below: number(&t1["hp_ratio"], 0.5),
        normal: number(&t1["heal_scale_1"], 1.2),
        low: number(&t1["heal_scale_2"], 1.4),
    });
    if !tb["atk_scale"].is_null() {
        s.hit = HitHandler::Blocked {
            scale: number(&tb["atk_scale"], 1.),
            own: true,
            inverse: false,
        };
    }
    if number(&tb["damage_resistance"], 0.) > 0. {
        s.incoming_hit = Some(PhysicalResistance {
            above_hp: number(&tb["hp_ratio"], 0.),
            fraction: number(&tb["damage_resistance"], 0.),
        });
    }
    s.modifiers = [
        ("atkPct".into(), number(&bb["atk"], 0.)),
        ("defPct".into(), number(&bb["def"], 0.)),
        ("aspd".into(), number(&bb["attack_speed"], 0.)),
        ("blockCnt".into(), number(&bb["block_cnt"], 0.)),
    ]
    .into();
    if default {
        s.extra.support.deferred = Some(Deferred {
            resistance: number(&bb["damage_resistance"], 0.5).clamp(0., 0.99),
            duration: number(&bb["final_duration"], 20.),
            interval: number(&bb["interval"], 0.1).max(1. / 30.),
        });
    } else if raw["skill"]["skillId"] == "skchr_gvial2_1" {
        s.extra.support.skill_drain_heal = number(&bb["heal_scale"], 0.3);
    } else {
        skill_range(raw, s)?;
        s.extra.support.pull_unblocked = true;
        s.extra.effects.skill_pull = Some(PullHit {
            force: number(&bb["attack@force"], 1.),
            travel_damage: 0.,
            distance: 1.,
        });
    }
    Ok(())
}
