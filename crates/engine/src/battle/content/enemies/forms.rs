// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct Husk {
    pub hits: f64,
    pub delay: f64,
    pub shield: u32,
    pub regen: bool,
}
impl Husk {
    pub(crate) fn key(self) -> &'static str {
        if self.regen {
            "ab:regen"
        } else {
            "ab:ember"
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Stone {
    pub res: f64,
    pub duration: f64,
    pub def: f64,
    pub stone_res: f64,
}
#[derive(Clone, Copy, Debug)]
pub enum Form {
    Husk(Husk),
    Stone(Stone),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum HuskPhase {
    Living { original: Option<f64> },
    Rebirth { original: f64, until: f64 },
    Husk { original: f64 },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum StonePhase {
    Ground,
    Stone,
    Flight,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum State {
    Husk { config: Husk, phase: HuskPhase },
    Stone { config: Stone, phase: StonePhase },
}
impl Form {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        Ok(Some(match key {
            "enemy_1288_duskls"
            | "enemy_1288_duskls_2"
            | "enemy_1292_duskld"
            | "enemy_9010_acpupp" => {
                let shield = t("Aura.max_damage_block_cnt");
                if shield < 0. || shield.ceil() > i32::MAX as f64 {
                    return Err("Husk shield count outside i32 domain".into());
                }
                Self::Husk(Husk {
                    hits: t("Revive[Trigger].prop_max_hp"),
                    delay: t("Revive[Trigger].interval"),
                    shield: shield.ceil() as u32,
                    regen: key == "enemy_9010_acpupp",
                })
            }
            "enemy_1172_dugago" | "enemy_1172_dugago_2" => Self::Stone(Stone {
                res: talent(
                    raw,
                    ov,
                    &["refracting.magic_resistance", "Refracting.magic_resistance"],
                ),
                duration: t("stone.duration"),
                def: t("stone.def"),
                stone_res: t("stone.magic_resistance"),
            }),
            _ => return Ok(None),
        }))
    }
    pub(super) fn state(self) -> State {
        match self {
            Self::Husk(config) => State::Husk {
                config,
                phase: HuskPhase::Living { original: None },
            },
            Self::Stone(config) => State::Stone {
                config,
                phase: StonePhase::Ground,
            },
        }
    }
}
impl Runtime {
    pub(crate) fn form(&self, id: Id) -> Option<State> {
        self.units.get(&id).and_then(|s| s.form)
    }
    pub(crate) fn set_form(&mut self, id: Id, state: State) {
        self.units.get_mut(&id).unwrap().form = Some(state);
    }
}
