// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "unyield",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_mudrok_2",
            kind: Kind::Instant,
            sp_type: SpType::Hurt,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skcom_def_up[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_mudrok_3",
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
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let _tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_def_up[3]" => Mode::First,
        "skchr_mudrok_2" => Mode::Second,
        "skchr_mudrok_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.extra.no_heal = true;
    skill_range(raw, s)?;
    let grid =
        crate::battle::operators::parse_range(&raw["skill"]["rangeGrid"]).unwrap_or_else(|_| {
            vec![
                [-1, -1],
                [-1, 0],
                [-1, 1],
                [0, -1],
                [0, 0],
                [0, 1],
                [1, -1],
                [1, 0],
                [1, 1],
            ]
        });
    if default {
        s.extra.skill_range = Some(grid.clone());
        let mut p = s.combat.profile;
        p.all_in_range = true;
        p.selection.ground_only = true;
        p.selection.can_hit_fly = false;
        s.combat.skill_profile = Some(p);
        s.combat.skill_scale = number(&bb["atk_scale"], 1.9);
        s.extra.skill_status = Some(OnHitStatus {
            key: "stun",
            duration: number(&bb["stun"], 0.5),
            chance: Some(number(&bb["buff_prob"], 0.3)),
        });
    }
    if mode == Mode::First {
        s.modifiers = [("defPct".into(), number(&bb["def"], 0.))].into();
    }
    if mode == Mode::Third {
        let mut p = s.combat.profile;
        p.hit_all_blocked = true;
        s.combat.skill_profile = Some(p);
        s.extra.effects.blocked_taken = number(&tb["damage_scale"], 1.);
    }
    s.extra.effects.blocked_taken = number(&tb["damage_scale"], 1.);
    let mut layer = Buff {
        key: Some("mudrok:layers".into()),
        ..Default::default()
    };
    layer.runtime.shield_hits =
        (number(&t["times"], 1.).max(0.) as u32).min(number(&t["max_times"], 3.).max(0.) as u32);
    if layer.runtime.shield_hits > 0 {
        s.extra.effects.deploy_buff = Some(layer);
    }
    s.extra.advanced = Advanced::Mudrok(Mudrok {
        mode,
        grid,
        sleep: number(&bb["sleep"], 10.),
        slow: number(&bb["move_speed"], -0.6),
        stun: number(&bb["stun"], 3.),
        awake: [
            ("atkPct".into(), number(&bb["atk"], 0.)),
            ("defPct".into(), number(&bb["def"], 0.)),
            (
                "batPct".into(),
                flat_bat(raw, number(&bb["base_attack_time"], 0.)).max(-0.9),
            ),
        ]
        .into(),
        layer_period: number(&t["interval"], 9.).max(1. / 30.),
        layer_add: number(&t["times"], 1.).max(0.) as u32,
        layer_cap: number(&t["max_times"], 3.).max(0.) as u32,
        layer_heal: number(&t["hp_ratio"], 0.2),
        layer_sp: number(&t["sp"], 0.),
        sarkaz_resistance: number(&t1["damage_resistance"], 0.3),
        module: [
            ("atkPct".into(), number(&tb["atk"], 0.)),
            ("defPct".into(), number(&tb["def"], 0.)),
        ]
        .into(),
    });
    if default {
        s.extra.advanced_start_heal = number(&bb["hp_ratio"], 0.04);
    }
    Ok(())
}
