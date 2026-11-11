// SPDX-License-Identifier: GPL-3.0-or-later
use super::movement::Cooldown;
use super::*;
#[derive(Clone, Copy, Debug)]
pub enum Leader {
    Skull {
        ratio: f64,
        atk: f64,
        defense: f64,
    },
    Axe {
        atk: f64,
        aspd: f64,
        max: u32,
        delay: f64,
    },
    Demon {
        aspd: f64,
        scale: f64,
    },
    Vines {
        every: u32,
        stun: f64,
        duration: f64,
        scale: f64,
    },
    Balloon {
        skill: Option<Cooldown>,
        duration: f64,
        speed: f64,
    },
    Spin {
        skill: Option<Cooldown>,
        duration: f64,
        interval: f64,
        radius: f64,
        scale: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Memory {
    None,
    Skull {
        splash: bool,
        enraged: bool,
    },
    Axe {
        last: Option<f64>,
    },
    Vines {
        count: u32,
    },
    Balloon {
        left: f64,
    },
    Spin {
        left: f64,
        until: f64,
        no_attack: bool,
        previous: Option<[f64; 2]>,
        hidden: bool,
    },
}
impl Leader {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let v = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        let n = |v: &Value, fallback| v.as_f64().filter(|n| n.is_finite()).unwrap_or(fallback);
        Ok(Some(match key {
            "enemy_1500_skulsr" => Self::Skull {
                ratio: v("atkup.hp_ratio", 0.5),
                atk: t("atkup.atk"),
                defense: -t("defdown.def"),
            },
            "enemy_2005_axetro" => Self::Axe {
                atk: t("atkup.atk"),
                aspd: t("atkup.attack_speed"),
                max: abilities::count(v("atkup.max_stack_cnt", 1.))?,
                delay: v("checker.delay", 4.),
            },
            "enemy_2048_smgrd" => Self::Demon {
                aspd: t("BlackFog.attack_speed"),
                scale: v("DamageUp.atk_scale", 1.),
            },
            "enemy_2052_smgia" => {
                let s = abilities::skill(raw, ov, "StunAttack");
                let sp = n(&s["spCost"], 0.);
                Self::Vines {
                    every: abilities::count(if sp > 0. { sp + 1. } else { 3. })?,
                    stun: n(&s["bb"]["stun"], 0.),
                    duration: t("Weak.weak[limit]"),
                    scale: v("Weak.damage_scale", 1.),
                }
            }
            "enemy_2004_balloon" => {
                let s = abilities::skill(raw, ov, "TakeOff");
                Self::Balloon {
                    skill: Cooldown::compile(raw, ov, "TakeOff"),
                    duration: n(&s["bb"]["duration"], 0.),
                    speed: n(&s["bb"]["balloon_s[fly].move_speed"], 0.),
                }
            }
            "enemy_10116_ymgtop" => Self::Spin {
                skill: Cooldown::compile(raw, ov, "SwitchModeTrigger").or(Some(Cooldown {
                    initial: 10.,
                    period: 60.,
                })),
                duration: t("EndRotate.rotate_duration"),
                interval: v("RotateDamage.interval", 1.),
                radius: v("RotateDamage.attack@range_radius", 1.),
                scale: v("RotateDamage.attack@atk_scale", 1.),
            },
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Skull { .. } => Memory::Skull {
                splash: false,
                enraged: false,
            },
            Self::Axe { .. } => Memory::Axe { last: None },
            Self::Vines { .. } => Memory::Vines { count: 0 },
            Self::Balloon { skill, .. } => Memory::Balloon {
                left: skill.map(|s| s.initial).unwrap_or(0.),
            },
            Self::Spin { skill, .. } => Memory::Spin {
                left: skill.map(|s| s.initial).unwrap_or(0.),
                until: f64::NEG_INFINITY,
                no_attack: false,
                previous: None,
                hidden: false,
            },
            _ => Memory::None,
        }
    }
}
impl Runtime {
    pub(crate) fn leader(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.leader)
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_leader(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().leader = state;
    }
}
