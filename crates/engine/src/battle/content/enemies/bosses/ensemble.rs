// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::VecDeque;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EchoForm {
    Dark,
    Gold,
}
#[derive(Clone, Copy, Debug)]
pub struct Pipe {
    pub caster: Caster,
    pub gold: bool,
    pub raw_aspd: bool,
    pub intervals: [f64; 2],
    pub threshold: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Echo {
    pub caster: Caster,
    pub gold_aspd: f64,
    pub dark_atk: f64,
    pub dark_slow: f64,
    pub strikes: [f64; 2],
    pub pulse: f64,
    pub element: f64,
    pub ensemble: [f64; 2],
}
#[derive(Clone, Debug)]
pub(crate) struct EchoState {
    pub form: EchoForm,
    pub strikes: u64,
    pub pulses: VecDeque<f64>,
    pub pulsing: bool,
}
impl Boss {
    pub(super) fn ensemble(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key: &str, default| talent_value(raw, ov, key).unwrap_or(default);
        let skill = abilities::skill(raw, ov, "1");
        let bb = |field: &str| {
            skill["bb"][field]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(0.)
        };
        let caster = |skill| Caster {
            passive: Passive::Plain,
            skills: [skill, None, None, None],
        };
        Ok(Some(match key {
            "enemy_9021_acduml" | "enemy_9021_acduml_2" | "enemy_9022_acdumm" => {
                let gold = key == "enemy_9022_acdumm";
                let prefix = if gold { "acdumm" } else { "acduml" };
                let hi = t(&format!("1.{prefix}_t_1[high_hp].interval"), 40.);
                Self::Pipe(Pipe {
                    caster: caster(
                        casters::ability(
                            raw,
                            ov,
                            "1",
                            Spell::PipeStrike {
                                gold,
                                radius: bb("range_radius"),
                                element: bb("ep_damage_ratio"),
                            },
                            if gold { 3.333 } else { 4.167 },
                            true,
                            false,
                        )
                        .map(|mut s| {
                            s.own = false;
                            s
                        }),
                    ),
                    gold,
                    raw_aspd: raw["stats"]["aspd"].as_f64() == Some(0.),
                    intervals: [hi, t(&format!("1.{prefix}_t_1[low_hp].interval"), hi)],
                    threshold: t("1.hp_ratio", 0.),
                })
            }
            "enemy_9023_acdums" => Self::Echo(Echo {
                caster: caster(casters::ability(
                    raw,
                    ov,
                    "Skill",
                    Spell::Ensemble,
                    0.,
                    false,
                    false,
                )),
                gold_aspd: t("1.attack_speed", 0.),
                dark_atk: t("2.atk", 0.),
                dark_slow: t("2.move_speed", 0.),
                strikes: [
                    t("2.hit_times_to_switch", 0.),
                    t("1.hit_times_to_switch", 0.),
                ],
                pulse: t("3.atk_scale", 0.),
                element: t("3.ep_damage_ratio", 0.),
                ensemble: [
                    t("4.ep_damage_ratio_depassion", 0.),
                    t("4.ep_damage_ratio_passion", 0.),
                ],
            }),
            _ => return Ok(None),
        }))
    }
}
pub(crate) fn form_buff(config: Echo, form: EchoForm) -> Buff {
    let mut b = Buff::default();
    b.key = Some("boss:echoForm".into());
    b.runtime.persist = true;
    b.runtime.visible = true;
    b.mods = match form {
        EchoForm::Gold => [("aspd".into(), config.gold_aspd)].into(),
        EchoForm::Dark => [
            ("atkPct".into(), config.dark_atk),
            ("moveMul".into(), 1. - config.dark_slow),
        ]
        .into(),
    };
    b
}
