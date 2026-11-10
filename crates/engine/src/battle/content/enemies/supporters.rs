// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use super::{movement::Cooldown, reactions::Fear};
#[derive(Clone, Copy, Debug)]
pub enum Supporter {
    Roar {
        fear: Fear,
        skill: Option<Cooldown>,
        duration: f64,
        aspd: f64,
    },
    Torturer {
        count: usize,
        radius: f64,
        heal: f64,
        atk: f64,
        max: u32,
        ranged: f64,
    },
    Barrel {
        speed: f64,
        scale: f64,
        life: f64,
        aspd: f64,
        dodge: f64,
    },
    Carrier {
        max: usize,
        camel: bool,
        speed: f64,
    },
}
#[derive(Clone, Debug)]
pub(crate) enum Memory {
    None,
    Roar { left: f64, feared: bool },
    Barrel { done: bool },
    Carrier { passengers: Vec<Id>, seats: usize },
}
impl Supporter {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let v = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        let n = |v: &Value, fallback| v.as_f64().filter(|n| n.is_finite()).unwrap_or(fallback);
        Ok(Some(match key {
            "enemy_10085_hllevi_2" => {
                let s = abilities::skill(raw, ov, "Roar");
                Self::Roar {
                    fear: Fear {
                        duration: t("SelfFear.fear"),
                        speed: t("SelfFear.move_speed"),
                        speed_duration: t("SelfFear.speed_duration"),
                        steal: None,
                    },
                    skill: Cooldown::compile(raw, ov, "Roar"),
                    duration: n(&s["bb"]["duration"], 0.),
                    aspd: n(&s["bb"]["attack_speed"], 0.),
                }
            }
            "enemy_1364_spnaxe_2" => Self::Torturer {
                count: t("Summon.cnt").floor().max(0.).min(20.) as usize,
                radius: v("Attack.range_radius", 1.),
                heal: t("Attack.hp_ratio"),
                atk: t("Attack.atk"),
                max: abilities::count(v("Attack.max_stack_cnt", 1.))?,
                ranged: v("Attack.attack@ranged_atk_scale", 1.),
            },
            "enemy_10044_wintun" => {
                let s = abilities::skill(raw, ov, "BlockedBoom");
                Self::Barrel {
                    speed: v("1.move_speed", 1.),
                    scale: n(&s["bb"]["blockee_atk_scale"], 1.),
                    life: n(&s["bb"]["fixed_duration"], 0.),
                    aspd: n(&s["bb"]["attack_speed"], 0.),
                    dodge: n(&s["bb"]["prob"], 0.),
                }
            }
            "enemy_1302_ymtro_2" | "enemy_10159_mntrjn" => {
                let camel = key == "enemy_10159_mntrjn";
                Self::Carrier {
                    max: abilities::count(
                        talent_value(raw, ov, if camel { "bus.max_cnt" } else { "Bus.max_cnt" })
                            .or_else(|| talent_value(raw, ov, "bus.max_cnt"))
                            .or_else(|| talent_value(raw, ov, "Bus.max_cnt"))
                            .unwrap_or(if camel { 5. } else { 4. }),
                    )? as usize,
                    camel,
                    speed: v("bus.move_speed", -0.2),
                }
            }
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self) -> Memory {
        match self {
            Self::Roar { skill, .. } => Memory::Roar {
                left: skill.map(|s| s.initial).unwrap_or(0.),
                feared: false,
            },
            Self::Barrel { .. } => Memory::Barrel { done: false },
            Self::Carrier { .. } => Memory::Carrier {
                passengers: vec![],
                seats: 0,
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
            Self::Barrel { speed, .. } => {
                let mut b = Buff::default();
                b.key = Some("ab:barrel".into());
                b.runtime.persist = true;
                b.mods.insert("moveMul".into(), speed);
                Some(b)
            }
            Self::Carrier { camel: true, .. } => {
                w.unit_mut(id)?.stats.set_move_floor(0.1);
                Some(flag_buff("ab:unblockable", "unblockable"))
            }
            Self::Carrier { camel: false, .. } => {
                w.unit_mut(id)?.block_weight = 4.;
                None
            }
            _ => None,
        };
        if let Some(b) = b {
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
        }
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn supporter(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.supporter.clone())
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_supporter(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().supporter = state;
    }
}
