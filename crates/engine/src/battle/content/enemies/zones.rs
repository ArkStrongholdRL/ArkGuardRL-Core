// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct Pollution {
    pub radius: f64,
    pub life: f64,
    pub low: f64,
    pub high: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Cloud {
    pub radius: f64,
    pub life: f64,
    pub interval: f64,
    pub scale: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Tank {
    pub max: f64,
    pub initial: f64,
    pub melee: f64,
    pub pollution: Pollution,
}
#[derive(Clone, Copy, Debug)]
pub enum ZoneKit {
    Pollution(Pollution),
    Poison(Cloud),
    Tank(Tank),
    Cannon { radius: f64, life: f64, amount: f64 },
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct TankState {
    pub sp: f64,
    pub skill: bool,
}
impl ZoneKit {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let val = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let n = |v: &Value, default| v.as_f64().filter(|n| n.is_finite()).unwrap_or(default);
        Ok(Some(match key {
            "enemy_1267_nhpbr" | "enemy_1267_nhpbr_2" => Self::Pollution(Pollution {
                radius: val("PollutedDie.projectile_range", 1.),
                life: t("PollutedDie.projectile_life_time"),
                low: t("PollutedDie.polluted_damage_low"),
                high: t("PollutedDie.polluted_damage_high"),
            }),
            "enemy_9006_actoxi" => Self::Poison(Cloud {
                radius: val("1.projectile_range", 0.8),
                life: t("1.projectile_life_time"),
                interval: val("1.interval", 1.),
                scale: t("1.damage_atk_scale"),
            }),
            "enemy_1272_nhtank" | "enemy_1272_nhtank_2" => {
                let s = abilities::skill(raw, ov, "PollutedRangedAtk");
                let max = [
                    n(&raw["sp"]["maxSp"], 0.),
                    n(&s["spCost"], 0.),
                    val("Empty.sp", 2.),
                ]
                .into_iter()
                .find(|n| *n != 0.)
                .unwrap_or(0.)
                .max(1.);
                Self::Tank(Tank {
                    max,
                    initial: n(&raw["sp"]["initSp"], 0.).max(0.).min(max),
                    melee: val("Empty.attack@chuang_atk_scale", 1.),
                    pollution: Pollution {
                        radius: 1.7,
                        life: n(&s["bb"]["projectile_life_time"], 0.),
                        low: n(&s["bb"]["polluted_damage_low"], 0.),
                        high: n(&s["bb"]["polluted_damage_high"], 0.),
                    },
                })
            }
            "enemy_10122_uacann_2" => Self::Cannon {
                radius: val("ProjectileBoomRange.attack@projectile_range", 1.) * 1.5,
                life: t("ProjectileBoomRange.attack@projectile_life_time"),
                amount: t("ProjectileBoomRange.attack@value"),
            },
            _ => return Ok(None),
        }))
    }
}
impl Runtime {
    pub(crate) fn tank_before(&mut self, id: Id) {
        let s = self.units.get_mut(&id).unwrap();
        if let (Kit::Zone(ZoneKit::Tank(config)), Some(state)) = (s.definition.kit, &mut s.tank) {
            state.skill = state.sp >= config.max;
        }
    }
    pub(crate) fn tank_skill(&self, id: Id) -> bool {
        self.units
            .get(&id)
            .and_then(|s| s.tank)
            .is_some_and(|s| s.skill)
    }
    pub(crate) fn tank_attack(&mut self, id: Id) -> bool {
        let s = self.units.get_mut(&id).unwrap();
        if let (Kit::Zone(ZoneKit::Tank(config)), Some(state)) = (s.definition.kit, &mut s.tank) {
            if state.skill {
                state.skill = false;
                state.sp = 0.;
                return true;
            }
            state.sp = (state.sp + 1.).min(config.max);
        }
        false
    }
}
