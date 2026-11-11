// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Cooldown {
    pub initial: f64,
    pub period: f64,
}
impl Cooldown {
    pub(super) fn compile(raw: &Value, ov: &Value, key: &str) -> Option<Self> {
        let s = abilities::skill(raw, ov, key);
        (!s.is_null()).then(|| Self {
            initial: s["initCooldown"]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(0.),
            period: s["cooldown"]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(0.),
        })
    }
    pub(crate) fn advance(self, left: &mut f64, dt: f64, castable: bool) -> bool {
        *left -= dt;
        if *left > 1e-9 || !castable {
            return false;
        }
        *left = self.period.max(1. / 30.);
        true
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Unbalance {
    Bleed { amount: f64, interval: f64 },
    Stun(f64),
    Wall(f64),
    LoseEgg(f64),
}
#[derive(Clone, Copy, Debug)]
pub enum Movement {
    Syufo {
        erosion: f64,
    },
    Parrot {
        down: f64,
        speed: f64,
        duration: f64,
    },
    Unbalanced(Unbalance),
    BoneSpike {
        restore: f64,
    },
    Run {
        skill: Option<Cooldown>,
        speed: f64,
        duration: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Memory {
    None,
    Syufo {
        crawl: bool,
    },
    Parrot {
        down: bool,
        chill: bool,
        ran: bool,
    },
    Bleed {
        elapsed: f64,
    },
    Travel {
        previous: Option<[f64; 2]>,
        hidden: bool,
        done: bool,
    },
    Cast {
        left: f64,
    },
}
impl Movement {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value, tag: Option<&str>) -> Option<Self> {
        let t = |key| talent(raw, ov, &[key]);
        Some(match key {
            "enemy_2025_syufo" => Self::Syufo {
                erosion: t("EpDamage.attack@ep_damage_ratio"),
            },
            "enemy_10045_parrot" => Self::Parrot {
                down: t("Stun.duration"),
                speed: talent_value(raw, ov, "M0SpeedUp.move_speed").unwrap_or(1.),
                duration: t("M0SpeedUp.duration"),
            },
            "enemy_1328_cbjedi" => Self::Unbalanced(Unbalance::Bleed {
                amount: t("unbalanced_bleed.damage"),
                interval: t("unbalanced_bleed.interval"),
            }),
            "enemy_10112_ymgds" => Self::Unbalanced(Unbalance::Stun(t("StunAfterUnbalance.stun"))),
            "enemy_10138_xdsnow" => Self::Unbalanced(Unbalance::Wall(t("hitWall.value"))),
            "enemy_10141_xdpeng_2" => Self::Unbalanced(Unbalance::LoseEgg(t("speed.move_speed"))),
            "enemy_9008_acbunn" => Self::BoneSpike {
                restore: if tag == Some("duck") { 1. } else { 0. },
            },
            "enemy_10001_trslim" => {
                let s = abilities::skill(raw, ov, "StartRun");
                Self::Run {
                    skill: Cooldown::compile(raw, ov, "StartRun"),
                    speed: s["bb"]["move_speed"]
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .unwrap_or(0.),
                    duration: s["bb"]["block_free_time"]
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .unwrap_or(0.),
                }
            }
            _ => return None,
        })
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Syufo { .. } => Memory::Syufo { crawl: false },
            Self::Parrot { .. } => Memory::Parrot {
                down: false,
                chill: false,
                ran: false,
            },
            Self::Unbalanced(Unbalance::Bleed { .. }) => Memory::Bleed { elapsed: 0. },
            Self::Unbalanced(_) => Memory::Travel {
                previous: None,
                hidden: false,
                done: false,
            },
            Self::Run { skill, .. } => Memory::Cast {
                left: skill.map(|s| s.initial).unwrap_or(0.),
            },
            _ => Memory::None,
        }
    }
    pub(super) fn spawn<H: buffs::Hooks>(
        self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let b = match self {
            Self::Syufo { .. } | Self::Parrot { .. } => {
                let mut b = flag_buff("ab:float", "float");
                b.flags
                    .extend(["unblockable", "noDisplace"].map(str::to_string));
                b
            }
            Self::BoneSpike { restore } => {
                let mut b = flag_buff("ab:stealth", "stealth");
                b.runtime.stealth_restore = Some(restore);
                b
            }
            _ => return Ok(()),
        };
        r.add(
            w,
            h,
            id,
            Spec {
                buff: b,
                flags_present: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn movement(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.movement)
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_movement(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().movement = state;
    }
}
