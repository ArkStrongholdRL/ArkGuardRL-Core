// SPDX-License-Identifier: GPL-3.0-or-later
use super::movement::Cooldown;
use super::*;
#[derive(Clone, Copy, Debug)]
pub enum Special {
    Catapult {
        times: u32,
        interval: f64,
        radius: f64,
        bound: f64,
    },
    Bomb {
        skill: Option<Cooldown>,
        speed: f64,
    },
    Cloud {
        grab: Option<Cooldown>,
        fire: Option<Cooldown>,
        radius: f64,
        duration: f64,
        gain: f64,
        max: f64,
        cost: f64,
        scale: f64,
    },
    Parasite {
        scale: f64,
        element: f64,
    },
    Armor {
        skill: Option<Cooldown>,
        appear: u32,
        times: u32,
        ranged: f64,
    },
    Translator {
        counts: [u32; 2],
        mods: [[f64; 7]; 3],
        anger: f64,
    },
    Elk {
        skill: Option<Cooldown>,
        duration: f64,
        scale: f64,
        stun: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    Fuchou,
    Youling,
    Shushi,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Translation {
    Origin,
    Pending(Form),
    Changing(Form),
    Done(Form),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum BombPhase {
    Ready,
    Charging { release: f64 },
    Dropped { waited: bool, landed: bool },
    Done,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Charge {
    pub target: Id,
    pub until: f64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Memory {
    Catapult {
        target: Option<Id>,
        position: Option<[f64; 2]>,
    },
    None,
    Bomb {
        phase: BombPhase,
        left: f64,
    },
    Cloud {
        left: [f64; 2],
        ammo: f64,
    },
    Parasite {
        host: Option<Id>,
    },
    Armor {
        left: f64,
        combo: u32,
        appeared: bool,
    },
    Translator {
        phase: Translation,
        counts: [u32; 2],
        anger: bool,
    },
    Elk {
        left: f64,
        charge: Option<Charge>,
        previous: Option<[f64; 2]>,
        hidden: bool,
    },
}
impl Special {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key: &str| talent(raw, ov, &[key]);
        let v = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        let sk = |key| abilities::skill(raw, ov, key);
        let n = |v: &Value, f| v.as_f64().filter(|v| v.is_finite()).unwrap_or(f);
        Ok(Some(match key {
            "enemy_10162_mnctpt" => Self::Catapult {
                times: v("Attack.attack@times", 1.).clamp(1., 100.).floor() as u32,
                interval: v("Attack.attack@hit_interval", 0.3),
                radius: v("Attack.attack@projectile_range", 0.5),
                bound: v("Attack.attack@offset_bound", 3.),
            },
            "enemy_1040_bombd" => Self::Bomb {
                skill: Cooldown::compile(raw, ov, "boomb"),
                speed: n(&sk("boomb")["bb"]["move_speed"], 0.),
            },
            "enemy_9009_acfort" => Self::Cloud {
                grab: Cooldown::compile(raw, ov, "KillOthers"),
                fire: Cooldown::compile(raw, ov, "FireWeapon"),
                radius: n(&sk("KillOthers")["bb"]["range_radius"], 1.5) * 2.5,
                duration: n(&sk("KillOthers")["bb"]["duration"], 4.),
                gain: n(&sk("KillOthers")["bb"]["sp"], 1.).min(64.),
                max: n(&raw["sp"]["maxSp"], 3.).max(1.).min(64.),
                cost: n(&sk("FireWeapon")["spCost"], 1.).max(1.),
                scale: n(&sk("FireWeapon")["bb"]["atk_scale"], 1.),
            },
            "enemy_9007_acelem" => Self::Parasite {
                scale: t("1.atk_scale"),
                element: v("1.ep_damage_scale", 1.),
            },
            "enemy_10027_vtsk" => Self::Armor {
                skill: Cooldown::compile(raw, ov, "MultiCombat"),
                appear: abilities::count(n(&sk("Appear")["bb"]["times"], 0.).clamp(0., 100.))?,
                times: abilities::count(n(&sk("MultiCombat")["bb"]["times"], 1.).clamp(0., 100.))?,
                ranged: v("range.attack@atk_scale_range", 1.),
            },
            "enemy_10081_mpplai" => {
                let bat = n(&raw["stats"]["bat"], 1.);
                let speed = n(&raw["stats"]["moveSpeed"], 1.);
                let mods = [
                    "Mode_Fuchou_Passive",
                    "Mode_Youling_Passive",
                    "Mode_Shushi_Passive",
                ]
                .map(|prefix| {
                    [
                        t(&format!("{prefix}.max_hp")),
                        t(&format!("{prefix}.atk")),
                        t(&format!("{prefix}.def")),
                        t(&format!("{prefix}.magic_resistance")),
                        (1. + t(&format!("{prefix}.move_speed"))
                            / if speed == 0. { 1. } else { speed })
                        .max(0.),
                        t(&format!("{prefix}.base_attack_time")) / if bat == 0. { 1. } else { bat },
                        t(&format!("{prefix}.mass_level")),
                    ]
                });
                Self::Translator {
                    counts: [
                        abilities::count(v("Passive.phy_max_count", 4.))?,
                        abilities::count(v("Passive.magic_max_count", 4.))?,
                    ],
                    mods,
                    anger: t("Mode_Fuchou_Anger.atk"),
                }
            }
            "enemy_10144_xdelk_2" => Self::Elk {
                skill: Cooldown::compile(raw, ov, "skill"),
                duration: n(&sk("skill")["bb"]["duration"], 0.),
                scale: n(&sk("skill")["bb"]["atk_scale_s"], 1.) * 2.,
                stun: t("data.attack@fail_duration"),
            },
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Catapult { .. } => Memory::Catapult {
                target: None,
                position: None,
            },
            Self::Bomb { skill, .. } => Memory::Bomb {
                phase: BombPhase::Ready,
                left: skill.map(|s| s.initial).unwrap_or(f64::INFINITY),
            },
            Self::Cloud { grab, fire, .. } => Memory::Cloud {
                left: [grab, fire].map(|s| s.map(|s| s.initial).unwrap_or(f64::INFINITY)),
                ammo: 0.,
            },
            Self::Parasite { .. } => Memory::Parasite { host: None },
            Self::Armor { skill, .. } => Memory::Armor {
                left: skill.map(|s| s.initial).unwrap_or(f64::INFINITY),
                combo: 0,
                appeared: false,
            },
            Self::Translator { .. } => Memory::Translator {
                phase: Translation::Origin,
                counts: [0; 2],
                anger: false,
            },
            Self::Elk { skill, .. } => Memory::Elk {
                left: skill.map(|s| s.initial).unwrap_or(f64::INFINITY),
                charge: None,
                previous: None,
                hidden: false,
            },
        }
    }
    pub(super) fn spawn<H: buffs::Hooks>(
        self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let mut b = Buff::default();
        match self {
            Self::Translator { .. } => {
                b.key = Some("ab:origin".into());
                b.runtime.persist = true;
                b.flags.insert("noDisplace".into());
            }
            Self::Parasite { .. } => {
                b.key = Some("ab:taunt".into());
                b.runtime.persist = true;
                b.mods.insert("taunt".into(), -1.);
            }
            _ => return Ok(()),
        }
        r.add(
            w,
            h,
            id,
            Spec {
                mods_present: !b.mods.is_empty(),
                flags_present: !b.flags.is_empty(),
                buff: b,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn special(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.special)
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_special(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().special = state;
    }
}
