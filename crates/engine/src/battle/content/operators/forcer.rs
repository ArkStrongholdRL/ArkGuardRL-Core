// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "pusher",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_forcer_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_forcer_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
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
    s.combat.profile.hit_all_blocked = true;
    s.extra.support.heavy_penetration = Some([
        number(&t["value"], 3.),
        number(&t["def_penetrate_fixed"], 0.),
    ]);
    s.extra.support.refund = number(&tb["value"], 0.);
    if default {
        let grid = crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"])?;
        s.trigger_range = Some(grid.clone());
        let direct = number(
            &bb["forcer_s_2[hit_directly].stun"],
            number(&bb["stun"], 1.),
        );
        s.extra.support.push = Some(PushCast {
            grid,
            force: number(&bb["force"], 1.),
            direct,
            wall: number(&bb["stun"], direct),
            collision: number(&bb["forcer_s_2[brush].stun"], direct),
        });
    } else {
        s.extra.support.effect_push = true;
        s.extra.effects.skill_push = Some(number(&bb["force"], 1.));
        s.extra.skill_status = Some(OnHitStatus {
            key: "stun",
            duration: number(&bb["stun"], 0.),
            chance: None,
        });
    }
    Ok(())
}
