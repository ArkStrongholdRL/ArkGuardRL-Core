// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct Helm {
    pub caster: Caster,
    pub low: f64,
    pub scale: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Blade {
    pub caster: Caster,
    pub hammer: bool,
    pub hits: f64,
    pub stun: f64,
    pub down: f64,
    pub scale: f64,
    pub dot: f64,
    pub duration: f64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum BladePhase {
    Hover,
    Dive { tile: [i32; 2], hits: u32 },
    Down,
    Gone,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShellTarget {
    pub tile: [i32; 2],
    pub source: Id,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct DroneLink {
    pub owner: Id,
    pub ratio: f64,
}
impl Boss {
    pub(super) fn helm(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let sk = |key| abilities::skill(raw, ov, key);
        let n = |v: &Value, default| v.as_f64().filter(|v| v.is_finite()).unwrap_or(default);
        let t = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let caster = |skills| Caster {
            passive: Passive::Plain,
            skills,
        };
        let make = |key, spell, turn, clip| {
            casters::ability(raw, ov, key, spell, clip, turn, false).map(|mut s| {
                s.own = false;
                s
            })
        };
        Ok(Some(match key {
            "enemy_9013_acstmk" | "enemy_9013_acstmk_2" => Self::Helm(Helm {
                caster: caster([
                    make("1", Spell::HelmShell, true, 2.333),
                    make(
                        "2",
                        Spell::HelmDrones {
                            ratio: n(&sk("2")["bb"]["hp_ratio"], 0.),
                            hp: n(&sk("2")["bb"]["summon.hp_ratio"], 0.),
                        },
                        false,
                        0.,
                    ),
                    None,
                    None,
                ]),
                low: t("1.hp_ratio", 0.),
                scale: t("1.damage_scale", 1.),
            }),
            "enemy_9014_acstma" | "enemy_9015_acstmb" => Self::Blade(Blade {
                caster: caster([make("1", Spell::BladeDive, false, 0.), None, None, None]),
                hammer: key == "enemy_9015_acstmb",
                hits: n(&sk("1")["bb"]["max_hit_cnt"], f64::INFINITY),
                stun: n(&sk("1")["bb"]["stun"], 0.),
                down: n(
                    &sk("1")["bb"]["special_stun_duration"],
                    n(&sk("1")["bb"]["stun"], 0.),
                ),
                scale: n(&sk("1")["bb"]["damage_scale"], 1.),
                dot: n(&sk("1")["bb"]["dot_damage"], 0.),
                duration: n(&sk("1")["bb"]["dot_duration"], 0.),
            }),
            "enemy_9016_acstmr" => Self::Shell {
                stun: t("killed.duration", 0.),
                dot: t("killed.value", 0.),
            },
            _ => return Ok(None),
        }))
    }
}
impl Runtime {
    pub(crate) fn drone(&self, id: Id) -> Option<DroneLink> {
        self.units.get(&id).and_then(|s| s.drone)
    }
    pub(crate) fn set_drone(&mut self, id: Id, link: Option<DroneLink>) {
        self.units.get_mut(&id).unwrap().drone = link;
    }
}
