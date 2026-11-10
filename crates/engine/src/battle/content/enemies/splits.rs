// SPDX-License-Identifier: GPL-3.0-or-later
use super::families::Memory;
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Split {
    Plain {
        count: f64,
    },
    Blades {
        count: f64,
        per_used: f64,
        atk: f64,
    },
    Veil {
        count: f64,
        radius: f64,
        duration: f64,
        cooldown: Option<f64>,
        initial: f64,
    },
    Teapot {
        count: f64,
        duration: f64,
        aspd: f64,
    },
    Boat {
        erosion: f64,
    },
}
impl Split {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let count = t("DeadSpawn.cnt");
        Ok(Some(match key {
            "enemy_1195_sfyin" | "enemy_1195_sfyin_2" | "enemy_1197_sfshu"
            | "enemy_1197_sfshu_2" | "enemy_1199_sfjin" => Self::Plain { count },
            "enemy_1207_sfji" | "enemy_1207_sfji_2" => {
                if count.ceil() > i32::MAX as f64 {
                    return Err("Blade count outside i32 domain".into());
                }
                Self::Blades {
                    count,
                    per_used: t("DeadSpawn.cnt_add"),
                    atk: talent(raw, ov, &["Atkup.atk", "AtkUp.atk"]),
                }
            }
            "enemy_1209_sfden" | "enemy_1209_sfden_2" => {
                let s = abilities::skill(raw, ov, "InvisibleShield");
                Self::Veil {
                    count,
                    radius: raw["stats"]["rawRangeRadius"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n != 0.)
                        .unwrap_or(2.),
                    duration: talent_value(raw, ov, "InvisibleShield.duration")
                        .unwrap_or_else(|| s["bb"]["duration"].as_f64().unwrap_or(0.)),
                    cooldown: s
                        .get("cooldown")
                        .map(|v| v.as_f64().filter(|n| n.is_finite()).unwrap_or(0.)),
                    initial: s["initCooldown"]
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .unwrap_or(0.),
                }
            }
            "enemy_1203_sfhu" | "enemy_1203_sfhu_2" => Self::Teapot {
                count,
                duration: t("DeadBoom.duration"),
                aspd: t("DeadBoom.attack_speed"),
            },
            "enemy_1162_magmot" => Self::Boat {
                erosion: t("EpDamage.attack@ep_damage_ratio"),
            },
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Blades { .. } => Memory::Blades {
                used: 0,
                clear_cd: false,
            },
            Self::Veil { initial, .. } => Memory::Veil { left: initial },
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
        if let Self::Blades { count, atk, .. } = self {
            if count > 0. && atk != 0. {
                let mut b = Buff::default();
                b.key = Some("ab:blade".into());
                b.runtime.persist = true;
                b.runtime.visible = true;
                b.mods.insert("atkPct".into(), atk);
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: b,
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn split_count(&self, id: Id, split: Split) -> usize {
        let n = match split {
            Split::Plain { count } | Split::Veil { count, .. } | Split::Teapot { count, .. } => {
                count
            }
            Split::Boat { .. } => 1.,
            Split::Blades {
                count, per_used, ..
            } => {
                let used = match self.units.get(&id).unwrap().memory {
                    Memory::Blades { used, .. } => used,
                    _ => 0,
                };
                (count + per_used * used as f64).max(1.)
            }
        };
        n.floor().clamp(0., 20.) as usize
    }
    pub(crate) fn spend_blade(&mut self, id: Id) -> bool {
        let s = self.units.get_mut(&id).unwrap();
        if let (Kit::Split(Split::Blades { count, .. }), Memory::Blades { used, clear_cd }) =
            (s.definition.kit, &mut s.memory)
        {
            if *used as f64 >= count {
                return false;
            }
            *used += 1;
            if (*used as f64) >= count {
                *clear_cd = true;
                return true;
            }
        }
        false
    }
    pub(crate) fn clear_blade_cooldown(&mut self, id: Id) -> bool {
        if let Memory::Blades { clear_cd, .. } = &mut self.units.get_mut(&id).unwrap().memory {
            return std::mem::take(clear_cd);
        }
        false
    }
    pub(crate) fn veil_ready(&mut self, id: Id, dt: f64, castable: bool) -> bool {
        let s = self.units.get_mut(&id).unwrap();
        if let (
            Kit::Split(Split::Veil {
                cooldown: Some(cd), ..
            }),
            Memory::Veil { left },
        ) = (s.definition.kit, &mut s.memory)
        {
            *left -= dt;
            if *left <= 1e-9 && castable {
                *left = cd;
                return true;
            }
        }
        false
    }
}
