// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub enum TerrainKit {
    Infection,
    Chimera {
        damage: f64,
        recovery: f64,
    },
    Nazg {
        radius: f64,
    },
    Chalice {
        radius: f64,
        share: f64,
    },
    Deepsea {
        swim: bool,
        atk: f64,
        drown: f64,
        every: u32,
        stun: f64,
    },
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct State {
    pub active: bool,
    pub wet: bool,
    pub elapsed: f64,
}
impl TerrainKit {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        Ok(Some(match key {
            "enemy_1422_lrsldr" | "enemy_1422_lrsldr_2" => Self::Infection,
            "enemy_1425_lrcmra" | "enemy_1425_lrcmra_2" => Self::Chimera {
                damage: t("OrigAura.damage"),
                recovery: t("OrigAura.sp_recover_ratio"),
            },
            "enemy_1427_lrnazg" => Self::Nazg {
                radius: raw["stats"]["rawRangeRadius"]
                    .as_f64()
                    .filter(|v| *v != 0.)
                    .or_else(|| raw["stats"]["rangeRadius"].as_f64().filter(|v| *v != 0.))
                    .unwrap_or(1.3),
            },
            "enemy_1430_lrrook" => Self::Chalice {
                radius: talent_value(raw, ov, "takeDmg.range_radius").unwrap_or(1.7),
                share: t("takeDmg.damage_scale"),
            },
            "enemy_1158_divman" => Self::Deepsea {
                swim: true,
                atk: t("Swim.atk"),
                drown: 0.,
                every: 0,
                stun: 0.,
            },
            "enemy_1160_hvyslr" | "enemy_1160_hvyslr_2" => {
                let s = abilities::skill(raw, ov, "stuncombat");
                let sp = s["spCost"].as_f64().unwrap_or(0.);
                Self::Deepsea {
                    swim: false,
                    atk: 0.,
                    drown: t("Drown.damage"),
                    every: abilities::count(if sp > 0. { sp + 1. } else { 3. })?,
                    stun: s["bb"]["stun"].as_f64().unwrap_or(0.),
                }
            }
            _ => return Ok(None),
        }))
    }
}
impl Runtime {
    pub(crate) fn terrain_state(&self, id: Id) -> State {
        self.units.get(&id).map(|s| s.terrain).unwrap_or_default()
    }
    pub(crate) fn set_terrain_state(&mut self, id: Id, state: State) {
        self.units.get_mut(&id).unwrap().terrain = state;
    }
}
