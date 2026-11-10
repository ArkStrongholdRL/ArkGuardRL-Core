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
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_billro_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SEARCH",
            attack_override: false,
        },
        Skill {
            id: "skchr_billro_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "SEARCH",
            attack_override: false,
        },
        Skill {
            id: "skchr_billro_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_billro_1" => Mode::First,
        "skchr_billro_2" => Mode::Second,
        "skchr_billro_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.combat.profile.only_while_skill = true;
    s.combat.profile.all_in_range = true;
    s.combat.profile.range_aoe = true;
    s.combat.profile.selection.can_hit_fly = true;
    s.combat.projectile = None;
    skill_range(raw, s)?;
    if mode == Mode::First {
        s.modifiers = [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
        ]
        .into();
    }
    if mode == Mode::Second {
        s.modifiers = [(
            "batPct".into(),
            flat_bat(raw, number(&bb["base_attack_time"], 0.)).max(-0.9),
        )]
        .into();
    }
    let mut guard = Buff {
        key: Some("trait:phalanxGuard".into()),
        mods: [
            ("defPct".into(), number(&tb["def"], 2.)),
            ("resFlat".into(), number(&tb["magic_resistance"], 20.)),
        ]
        .into(),
        ..Default::default()
    };
    guard.runtime.source = None;
    s.extra.effects.deploy_buff = Some(guard);
    s.extra.advanced = Advanced::Billro(Billro {
        mode,
        heal: number(&t["heal_scale"], 0.4),
        charged_heal: number(&t["billro_t_1[enhance].heal_scale"], 0.8),
        recovery: number(&t1["sp_recovery_per_sec"], 0.6),
        charged_atk: number(&bb["atk"], 0.1),
        ramp_atk: number(&bb["atk"], 0.),
        mark: number(&bb["attack@damage_scale"], 0.2),
        guard: [
            ("defPct".into(), number(&tb["def"], 2.)),
            ("resFlat".into(), number(&tb["magic_resistance"], 20.)),
        ]
        .into(),
        keep: [
            ("defPct".into(), number(&tb["billro_e_002[buff].def"], 0.)),
            (
                "resFlat".into(),
                number(&tb["billro_e_002[buff].magic_resistance"], 0.),
            ),
        ]
        .into(),
        sluggish: number(&bb["attack@sluggish"], 0.3),
        bind: number(&bb["attack@root"], 0.3),
        per_enemy: number(&tm["damage_scale"], 0.),
        enemy_cap: number(&tm["max_valid_stack_cnt"], 5.),
    });
    Ok(())
}
