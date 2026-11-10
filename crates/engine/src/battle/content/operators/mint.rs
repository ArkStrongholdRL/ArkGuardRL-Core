// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "phalanx",
    ranged: true,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_mint_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SEARCH",
            attack_override: true,
        },
        Skill {
            id: "skchr_mint_1",
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
    // The no-attack data row retains the caster profession's air targeting.
    s.combat.profile.selection.can_hit_fly = true;
    s.combat.profile.only_while_skill = true;
    s.combat.profile.all_in_range = true;
    s.combat.profile.range_aoe = true;
    s.combat.projectile = None;
    let mut p = s.combat.profile;
    p.selection.can_hit_fly = true;
    s.combat.skill_profile = Some(p);
    s.combat.skill_scale = number(&bb["attack@atk_scale"], number(&bb["atk_scale"], 1.));
    s.combat.skill_area.selection.can_hit_fly = true;
    skill_range(raw, s)?;
    let grid = raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] == 0)
        .map(|t| &t["rangeGrid"])
        .unwrap_or(&Value::Null);
    s.extra.support.phalanx = Some(Phalanx {
        guard: [
            ("defPct".into(), number(&tb["def"], 2.)),
            ("resFlat".into(), number(&tb["magic_resistance"], 20.)),
        ]
        .into(),
        keep: [
            ("defPct".into(), number(&tb["soil_e_002[buff].def"], 0.)),
            (
                "resFlat".into(),
                number(&tb["soil_e_002[buff].magic_resistance"], 0.),
            ),
        ]
        .into(),
        taunt: number(&t["taunt_level"], -1.),
        aura: StatAura {
            key: "talent:mint_geo",
            grid: Some(if grid.is_null() {
                vec![[1, 0], [0, -1], [0, 0], [0, 1], [-1, 0]]
            } else {
                crate::battle::operators::parse_range(grid)?
            }),
            profession: None,
            mods: [("defPct".into(), number(&t["def"], 0.))].into(),
        },
        burst: if default {
            number(&bb["atk_scale"], 0.)
        } else {
            0.
        },
    });
    if default {
        s.extra.support.inward = Some(number(&bb["attack@force"], number(&bb["force"], 0.)));
    }
    Ok(())
}
