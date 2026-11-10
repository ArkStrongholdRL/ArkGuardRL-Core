// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub(crate) enum BuffEffect {
    Vest {
        side: f64,
        cut: f64,
    },
    XiShield {
        radius: f64,
        scale: f64,
    },
    Fear {
        started: f64,
        hp: f64,
        ramp: f64,
        peak: f64,
    },
}
impl Caster {
    pub(super) fn champion(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let bb = |key: &str, field: &str, default| {
            n(&abilities::skill(raw, ov, key)["bb"][field], default)
        };
        let make = |key, spell, clip, turn| ability(raw, ov, key, spell, clip, turn, false);
        let (passive, skills) = match key {
            "enemy_1517_xi" => {
                let shield = |key, form2| {
                    make(
                        key,
                        Spell::XiShield {
                            form2,
                            duration: bb(key, "duration", 0.),
                            amount: bb(key, "dynamic", 0.),
                            radius: bb(key, "range_radius", 0.),
                            scale: bb(key, "atk_scale", 0.),
                        },
                        0.,
                        true,
                    )
                    .map(|mut s| {
                        s.own = false;
                        s
                    })
                };
                (
                    Passive::Xi {
                        duration: t("reborn.duration", 0.),
                        invincible: t("reborn.invincible", 0.),
                        atk: t("reborn.atk", 0.),
                    },
                    [
                        shield("ShieldBurst", false),
                        shield("ShieldBurstReborn", true),
                        make(
                            "CrossAttack",
                            Spell::XiCross {
                                scale: bb("CrossAttack", "atk_scale", 1.),
                            },
                            1.333,
                            true,
                        ),
                        None,
                    ],
                )
            }
            "enemy_1525_blkswb" => {
                let blink = |key, form2| {
                    make(
                        key,
                        Spell::MaceBlink {
                            form2,
                            distance: bb(key, "dist", 1.5),
                            scale: bb(key, "atk_scale", 1.),
                        },
                        0.,
                        false,
                    )
                };
                let circle = |key, form2| {
                    make(
                        key,
                        Spell::MaceCircle {
                            form2,
                            radius: bb(key, "range_radius", 0.),
                            scale: bb(key, "atk_scale", 1.),
                        },
                        2.067,
                        true,
                    )
                };
                (
                    Passive::Mace {
                        resist: {
                            let v = -t("Buff.one_minus_status_resistance", -0.5);
                            if v > 0. {
                                v.min(1.)
                            } else {
                                0.5
                            }
                        },
                        duration: t("Reborn.duration", 0.),
                        invincible: t("Reborn.invincible", 0.),
                        pen: [
                            t("DefPenetrate.enemy_blkswb_t_2.def_penetrate", 0.),
                            t(
                                "DefPenetrate.enemy_blkswb_t_2[reborn].def_penetrate",
                                t("DefPenetrate.enemy_blkswb_t_2.def_penetrate", 0.),
                            ),
                        ],
                        cut: t("ClearSp.hp_ratio", 0.),
                        disarm: t("ClearSp.duration", 0.),
                        radius: t("ClearSp.range_radius", 0.),
                    },
                    [
                        blink("Blink", false),
                        blink("Blink2", true),
                        circle("CircleAttack", false),
                        circle("CircleAttack2", true),
                    ],
                )
            }
            "enemy_1535_wlfmster" => {
                let mut fear = make(
                    "FearCage",
                    Spell::Fear {
                        max: abilities::count(bb("FearCage", "max_target", 3.))? as usize,
                        aspd: bb("FearCage", "attack_speed", 0.),
                        peak: bb("FearCage", "hp_ratio", 0.),
                        ramp: bb("FearCage", "duration_bleed", 40.).max(1.),
                        cut: bb("FearCage", "hp_ratio_offset", 0.2).abs(),
                        refund: bb("FearCage", "sp", 0.),
                        wait: bb("FearCage", "duration_wait", 0.),
                    },
                    0.,
                    true,
                );
                if let Some(s) = &mut fear {
                    let sp = n(&abilities::skill(raw, ov, "FearCage")["spCost"], 0.);
                    if sp != 0. {
                        s.clock = Cooldown {
                            initial: sp,
                            period: sp,
                        };
                    }
                }
                (
                    Passive::Wolf {
                        duration: t("Reborn.duration", 0.),
                        invincible: t("Passive2.invincible_time", 0.),
                        resistance: t("Passive.damage_resistance", 0.),
                        atk: t("Passive2.atk", 0.),
                        bat: t("Passive2.base_attack_time", 0.),
                    },
                    [fear, None, None, None],
                )
            }
            "enemy_2016_csphtm" | "enemy_2017_csphts" => (
                Passive::Lucien {
                    penetration: t("penetrate.def_penetrate", 0.),
                    dodge: t("evade.prob", 0.),
                    element: t("combat.attack@ep_damage_ratio", 0.),
                },
                [
                    make(
                        "aoe",
                        Spell::LucienAoe {
                            scale: bb("aoe", "atk_scale", 0.),
                            element: bb("aoe", "ep_damage_ratio", 0.),
                        },
                        1.8,
                        true,
                    ),
                    if key == "enemy_2016_csphtm" {
                        make(
                            "blink",
                            Spell::Phantom {
                                distance: bb("blink", "dist", 1.5),
                            },
                            0.,
                            false,
                        )
                    } else {
                        None
                    },
                    None,
                    None,
                ],
            ),
            _ => return Ok(None),
        };
        Ok(Some(Self { passive, skills }))
    }
    pub(crate) fn rebirth(&self) -> Option<(f64, f64, f64)> {
        match self.passive {
            Passive::Xi {
                duration,
                invincible,
                ..
            }
            | Passive::Mace {
                duration,
                invincible,
                ..
            }
            | Passive::Wolf {
                duration,
                invincible,
                ..
            } => Some((duration, invincible, 1.)),
            _ => None,
        }
    }
}
