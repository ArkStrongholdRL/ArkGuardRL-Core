// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "physician",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_silent_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skcom_heal_up[3]",
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
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skcom_heal_up[3]" => Mode::First,
        "skchr_silent_2" => Mode::Second,
        _ => return Err("Unknown selected skill".into()),
    };
    healing(s, 1.);
    if mode == Mode::First {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    } else {
        s.rule_override = Some(skills::Rule::SpFull);
    }
    s.extra.tactics = Tactics::Silent(Silent {
        aspd: n(t, "attack_speed", 0.),
        ground_heal: n(tb, "heal_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Silent {
    pub aspd: f64,
    pub ground_heal: f64,
}
impl Behavior for Silent {
    fn heal_multiplier(&self, w: &World, target: Id) -> Result<f64, String> {
        Ok(if w.unit(target)?.ground {
            self.ground_heal
        } else {
            1.
        })
    }

    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        match event {
            KitEvent::Scheduled(due)
                if (due[3] || k.world.time == 0.) && k.up() && self.aspd != 0. =>
            {
                for target in k.host.roster.as_ref().unwrap().ids().to_vec() {
                    if Host::tac_up(k.world, target)
                        && k.host.tac_def(target)?.profession == "MEDIC"
                    {
                        if let Some(buff) = k
                            .world
                            .unit(target)?
                            .stats
                            .buffs()
                            .iter()
                            .find(|buff| buff.key.as_deref() == Some("talent:silent"))
                        {
                            if buff.runtime.source != Some(k.unit)
                                && buff.runtime.tick_payload > self.aspd
                                && buff.runtime.time_left > 0.05
                            {
                                continue;
                            }
                        }
                        let mut buff = Buff {
                            key: Some("talent:silent".into()),
                            mods: [("aspd".into(), self.aspd)].into(),
                            ..Default::default()
                        };
                        buff.runtime.duration = 0.6;
                        buff.runtime.tick_payload = self.aspd;
                        k.add(target, buff)?;
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == k.unit && skill.spec.rule == skills::Rule::SpFull => {
                super::prepared_summon::release_drone(k)?;
            }
            _ => {}
        }
        Ok(())
    }
}
