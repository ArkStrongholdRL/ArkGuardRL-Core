// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(super) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SNIPER",
    branch: "reaperrange",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_pinecn_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_pinecn_1",
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
    let t = talent(raw);
    s.combat.profile.all_in_range = true;
    s.grid_scale = Some(GridScale {
        grid: crate::battle::operators::parse_range(&raw["trait"]["rangeGrid"])?,
        scale: number(&raw["trait"]["bb"]["atk_scale"], 1.5),
        skill_all: false,
    });
    let mut power = Buff {
        key: Some("talent:pinecn_power".into()),
        mods: [(
            "spRecoveryFlat".into(),
            number(&t["sp_recovery_per_sec"], 0.),
        )]
        .into(),
        ..Default::default()
    };
    power.runtime.duration = number(&t["duration"], 60.);
    s.extra.effects.deploy_buff = Some(power);
    if default {
        skill_range(raw, s)?;
        s.rule_override = Some(crate::battle::skills::Rule::CustomRange);
        let mut entries: Vec<_> = bb
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| k.ends_with("].atk"))
            .collect();
        entries.sort_by_key(|(k, _)| *k);
        s.extra.effects.growing_atk =
            Some(entries.into_iter().map(|(_, v)| number(v, 0.)).collect());
    } else {
        s.combat.skill_scale = number(&bb["atk_scale"], 1.);
        s.extra.effects.skill_def_ignore = number(&bb["def_penetrate_fixed"], 0.);
        s.extra.effects.force_attack = true;
    }
    Ok(())
}
