// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "funnel",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_malist_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skcom_quickattack[3]",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_quickattack[3]" => Mode::First,
        "skchr_malist_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    if mode == Mode::First {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("aspd".into(), n(bb, "attack_speed", 0.)),
        ]
        .into();
    } else {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
        s.combat.skill_hits = 2;
    }
    s.extra.tactics = Tactics::Malist(Malist {
        mode: mode,
        funnel: funnel::Parameters::read(raw),
        prob: n(t, "prob", 0.),
        crit: n(t, "atk_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Malist {
    pub mode: Mode,
    pub funnel: funnel::Parameters,
    pub prob: f64,
    pub crit: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub funnel: funnel::State,
}
impl Behavior for Malist {
    fn payload(
        &self,
        _: &World,
        _: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        payload.attachment = Some(self.funnel.attachment(payload.hits, true));
        payload.hits = if payload.is_skill && self.mode == Mode::Second {
            2
        } else {
            1
        };
        Ok(())
    }
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Before(_)) {
            -100
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        if let KitEvent::Before(context) = event {
            if context.attacker == k.unit {
                k.world
                    .unit_mut(k.unit)?
                    .operator
                    .tactical
                    .malist_mut()?
                    .funnel
                    .map
                    .retain(|target, _| context.targets.contains(target));
            }
        }
        Ok(())
    }
}
