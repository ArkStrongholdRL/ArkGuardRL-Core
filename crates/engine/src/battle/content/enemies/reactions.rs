// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Clone, Copy, Debug)]
pub struct Fear {
    pub duration: f64,
    pub speed: f64,
    pub speed_duration: f64,
    pub steal: Option<f64>,
}
#[derive(Clone, Copy, Debug)]
pub enum Reaction {
    Aura {
        radius: f64,
        defense: bool,
        amount: f64,
    },
    ExposeDeath {
        radius: f64,
        duration: f64,
        scale: f64,
    },
    HealDeath {
        radius: f64,
        scale: f64,
    },
    Barrier {
        resistance: f64,
        aspd: f64,
        shield: f64,
    },
    Double {
        every: u32,
        scale: f64,
    },
    Rush {
        speed: f64,
        interval: f64,
        max: u32,
        scale: f64,
    },
    FirstSplash {
        scale: f64,
    },
    Fear(Fear),
    Front {
        reduction: f64,
        neural: f64,
    },
    Crowd {
        reduction: f64,
    },
    Shell,
    Blood {
        ratio: f64,
    },
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Memory {
    None,
    Once {
        done: bool,
    },
    Power {
        count: u32,
        powered: bool,
    },
    Rush {
        active: bool,
        layers: u32,
        elapsed: f64,
    },
    Facing {
        x: f64,
        sign: f64,
    },
}
impl Reaction {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let v = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let n = |v: &Value, default| v.as_f64().filter(|n| n.is_finite()).unwrap_or(default);
        Ok(Some(match key {
            "enemy_2085_skzjxd" | "enemy_2085_skzjxd_2" => Self::Crowd {
                reduction: t("Weakness.damage_resistance"),
            },
            "enemy_1017_defdrn" => Self::Aura {
                radius: v("defup.range_radius", 2.5),
                defense: true,
                amount: t("defup.def"),
            },
            "enemy_1355_mrfly" | "enemy_1355_mrfly_2" => Self::Aura {
                radius: n(&raw["stats"]["rangeRadius"], 0.).max(0.),
                defense: false,
                amount: t("magdef_add.magic_resistance"),
            },
            "enemy_1407_hummbd" => Self::ExposeDeath {
                radius: v("Expose.range_radius", 1.),
                duration: t("Expose.weak[limit]"),
                scale: v("Expose.damage_scale", 1.),
            },
            "enemy_10156_mncrer" => Self::HealDeath {
                radius: v("Boom.projectile_range", 1.),
                scale: t("Boom.heal_scale"),
            },
            "enemy_10127_rkmbst_2" => Self::Barrier {
                resistance: t("M0Shield.damage_resistance"),
                aspd: t("M0Shield.attack_speed"),
                shield: t("M0Shield.init_shield_hp_ratio"),
            },
            "enemy_10118_ymgprc" => {
                let s = abilities::skill(raw, ov, "PowerAttack");
                let sp = n(&s["spCost"], 0.);
                Self::Double {
                    every: abilities::count(if sp > 0. { sp + 1. } else { 3. })?,
                    scale: s["bb"]["atk_scale"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n != 0.)
                        .unwrap_or(1.),
                }
            }
            "enemy_1072_dlancer" => Self::Rush {
                speed: t("rush.dlancer_t[trigger].move_speed"),
                interval: {
                    let value = v("rush.dlancer_t[trigger].interval", 0.5);
                    // The official 0.5-second step permits a tenfold speedup.
                    if value > 0. {
                        value.max(0.05)
                    } else {
                        value
                    }
                },
                max: abilities::count(t("rush.dlancer_t[trigger].trig_cnt"))?,
                scale: t("firstattack.atk_scale"),
            },
            "enemy_1320_wdrrl_2" => Self::FirstSplash {
                scale: t("AOEAttack.atk_scale"),
            },
            "enemy_10083_hlbird" | "enemy_10084_hlegle" => Self::Fear(Fear {
                duration: t("SelfFear.fear"),
                speed: t("SelfFear.move_speed"),
                speed_duration: t("SelfFear.speed_duration"),
                steal: (key == "enemy_10084_hlegle")
                    .then(|| v("DamageOrBullet.attack@minus_bullet", 1.)),
            }),
            "enemy_10097_crshd" => Self::Front {
                reduction: t("weakness.damage_resistance"),
                neural: t("block.ep_damage_ratio"),
            },
            "enemy_1112_emppnt" | "enemy_1112_emppnt_2" => Self::Shell,
            "enemy_1367_dseed" => Self::Blood {
                ratio: t("Passive.hp_ratio"),
            },
            _ => return Ok(None),
        }))
    }
    pub(super) fn memory(self, position: [f64; 2]) -> Memory {
        match self {
            Self::Fear(_) | Self::FirstSplash { .. } => Memory::Once { done: false },
            Self::Double { .. } => Memory::Power {
                count: 0,
                powered: false,
            },
            Self::Rush { .. } => Memory::Rush {
                active: true,
                layers: 0,
                elapsed: 0.,
            },
            Self::Front { .. } | Self::Crowd { .. } => Memory::Facing {
                x: position[1],
                sign: -1.,
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
        let mut list = vec![];
        match self {
            Self::Crowd { .. } => {
                list.push(flag_buff("ab:unblockable", "unblockable"));
            }
            Self::FirstSplash { .. } => w.unit_mut(id)?.block_weight = 3.,
            Self::Barrier {
                resistance,
                aspd,
                shield,
            } => {
                let mut taunt = Buff::default();
                taunt.key = Some("ab:taunt".into());
                taunt.mods.insert("taunt".into(), 1.);
                taunt.runtime.persist = true;
                list.push(taunt);
                let mut m = Buff::default();
                m.key = Some("ab:m0".into());
                m.mods = [
                    ("dmgTakenMul".into(), 1. - resistance),
                    ("aspd".into(), aspd),
                ]
                .into();
                m.runtime.persist = true;
                m.runtime.visible = true;
                list.push(m);
                let amount = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("maxHp")
                    * shield;
                if amount > 0. {
                    let mut b = Buff::default();
                    b.key = Some("ab:m0barrier".into());
                    b.shield = amount;
                    b.runtime.persist = true;
                    list.push(b);
                }
            }
            Self::Blood { .. } => {
                let mut b = flag_buff("ab:dseed", "selfBound");
                b.flags
                    .extend(["noMove", "unblockable", "noDisplace"].map(str::to_string));
                list.push(b);
            }
            _ => {}
        }
        for b in list {
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
    pub(crate) fn reaction(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.reaction)
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_reaction(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().reaction = state;
    }
}
