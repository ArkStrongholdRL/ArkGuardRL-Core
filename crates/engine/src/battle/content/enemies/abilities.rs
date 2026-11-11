// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol content/enemies/{archetypes,special,fly}.js.
use super::*;

#[derive(Clone, Copy, Debug)]
pub enum Ability {
    Unblockable,
    Bleed {
        amount: f64,
        duration: f64,
    },
    Nest {
        ratio: f64,
        radius: f64,
        resist: f64,
    },
    Boil {
        interval: f64,
        radius: f64,
        scale: f64,
        burn: f64,
    },
    Expose {
        duration: f64,
        scale: f64,
    },
    Tide {
        erosion: f64,
    },
    Nucleus {
        speed: f64,
        interval: f64,
        radius: f64,
        neural: f64,
    },
    Taunt(f64),
    HitShield(u32),
    MaxTargets(f64),
    LowHp(f64),
    Run(f64),
    DefDecay {
        max: u32,
        def: f64,
        res: f64,
    },
    FrozenPower(f64),
    Stun {
        every: u32,
        duration: f64,
    },
    Power {
        every: u32,
        scale: f64,
        erosion: f64,
    },
    Song {
        every: u32,
        duration: f64,
        aspd: f64,
        neural: f64,
    },
    Frost {
        radius: f64,
        aspd: f64,
    },
    Enhance {
        interval: f64,
        speed: f64,
        atk: f64,
    },
    SelfDamage(f64),
    DeathBoom {
        radius: f64,
        scale: f64,
        cold: Option<f64>,
    },
}
pub(super) fn count(n: f64) -> Result<u32, String> {
    if !n.is_finite() || n < 0. || n.ceil() > i32::MAX as f64 {
        return Err("Enemy ability count outside nonnegative i32 domain".into());
    }
    Ok(n.ceil() as u32)
}
pub(super) fn skill<'a>(raw: &'a Value, overrides: &'a Value, key: &str) -> &'a Value {
    overrides
        .get("skills")
        .and_then(Value::as_array)
        .or_else(|| raw["skills"].as_array())
        .and_then(|a| a.iter().find(|s| s["prefabKey"] == key))
        .unwrap_or(&Value::Null)
}
fn number(v: &Value, default: f64) -> f64 {
    v.as_f64().filter(|v| v.is_finite()).unwrap_or(default)
}
fn every(s: &Value) -> Result<u32, String> {
    let sp = number(&s["spCost"], 0.);
    count(if sp > 0. { sp + 1. } else { 3. })
}
impl Ability {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let value = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        Ok(Some(match key {
            "enemy_1270_nhstlk" | "enemy_1270_nhstlk_2" => Self::Bleed {
                amount: t("Bleeding.attack@bleeding_damage"),
                duration: t("Bleeding.attack@duration"),
            },
            "enemy_1234_dsubrl" | "enemy_1234_dsubrl_2" => {
                let resist = -value("Buff.one_minus_status_resistance", -0.5);
                Self::Nest {
                    ratio: t("EpDamage.ep_damage_ratio"),
                    radius: raw["stats"]["rangeRadius"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n != 0.)
                        .unwrap_or(1.6),
                    resist: if resist > 0. { resist.min(1.) } else { 0.5 },
                }
            }
            "enemy_10054_cjhot" => Self::Boil {
                interval: raw["stats"]["bat"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n != 0.)
                    .unwrap_or(1.),
                radius: raw["stats"]["rawRangeRadius"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n != 0.)
                    .unwrap_or(1.6),
                scale: value("aoe.atk_scale", 1.),
                burn: t("aoe.ep_damage_ratio"),
            },
            "enemy_1402_tgshd_2" => Self::Expose {
                duration: t("Expose.weak[limit]"),
                scale: value("Expose.damage_scale", 1.),
            },
            "enemy_1161_tidmag" | "enemy_1161_tidmag_2" => Self::Tide {
                erosion: t("EpDamage.attack@ep_damage_ratio"),
            },
            "enemy_1439_dslntf" | "enemy_1439_dslntf_2" => Self::Nucleus {
                speed: value("0.move_speed", 1.),
                interval: value("1.interval", 1.),
                radius: t("1.range_radius"),
                neural: t("1.ep_damage_ratio"),
            },
            "enemy_1026_aghost" => Self::Unblockable,
            "enemy_1081_sotisd" => Self::Taunt(t("taunt.taunt_level")),
            "enemy_1249_lysdb_2" => Self::HitShield(count(t("Shield.max_block_damage_cnt"))?),
            "enemy_1501_demonk" | "enemy_10018_sgrobh" => Self::MaxTargets(2.),
            "enemy_1025_reveng" => Self::LowHp(t("atkup.atk")),
            "enemy_2001_duckmi" | "enemy_2001_duckmi_2" => Self::Run(t("run.attack@move_speed")),
            "enemy_1329_cbshld" | "enemy_1329_cbshld_2" => Self::DefDecay {
                max: count(t("def_reduce.max_stack_cnt"))?,
                def: t("def_reduce.def"),
                res: t("def_reduce.magic_resistance"),
            },
            "enemy_1069_icebrk_2" => Self::FrozenPower(value("atkup.atk_scale", 1.)),
            "enemy_1045_hammer" | "enemy_1045_hammer_2" => {
                let s = skill(raw, ov, "stuncombat");
                Self::Stun {
                    every: every(s)?,
                    duration: number(&s["bb"]["stun"], 0.),
                }
            }
            "enemy_1183_mlasrt" => {
                let s = skill(raw, ov, "PowerAttack");
                Self::Power {
                    every: every(s)?,
                    scale: number(&s["bb"]["atk_scale"], 1.),
                    erosion: t("EpDamage.attack@ep_damage_ratio"),
                }
            }
            "enemy_2010_csdcr" => Self::Song {
                every: talent_value(raw, ov, "AttackSpeedUp.stack_cnt")
                    .map(|n| count(n.max(1.)))
                    .transpose()?
                    .unwrap_or(0),
                duration: t("AttackSpeedUp.duration"),
                aspd: t("AttackSpeedUp.attack_speed"),
                neural: t("attack.attack@ep_damage_ratio"),
            },
            "enemy_1042_frostd" => Self::Frost {
                radius: value("defup.range_radius", 2.5),
                aspd: t("atkSpeedDown.attack_speed") * 100.,
            },
            "enemy_10087_hlchgr" => {
                let s = skill(raw, ov, "ForeverEnhance");
                Self::Enhance {
                    interval: value("SkillTrigger.interval", 6.),
                    speed: number(&s["bb"]["move_speed_add"], 0.),
                    atk: number(&s["bb"]["atk_add"], 0.),
                }
            }
            "enemy_1062_rager_2" => Self::SelfDamage(t("periodic_damage.damage")),
            "enemy_1021_bslime" => Self::DeathBoom {
                radius: 1.25,
                scale: t("boom.atk_scale"),
                cold: None,
            },
            "enemy_1067_snslime" => Self::DeathBoom {
                radius: number(&raw["stats"]["rawRangeRadius"], 0.).max(0.),
                scale: t("boom.atk_scale"),
                cold: Some(t("boom.freeze")),
            },
            _ => return Ok(None),
        }))
    }
}
impl Runtime {
    const PLAIN: Kit = Kit::Plain;
    pub(crate) fn kit(&self, id: Id) -> Kit {
        self.units
            .get(&id)
            .map(|s| s.definition.kit)
            .unwrap_or(Kit::Plain)
    }
    /// Read-only kit access for hot dispatch paths that only inspect a variant.
    pub(crate) fn kit_ref(&self, id: Id) -> &Kit {
        self.units
            .get(&id)
            .map(|s| &s.definition.kit)
            .unwrap_or(&Self::PLAIN)
    }
    pub(crate) fn trigger_once(&mut self, id: Id) -> bool {
        let Some(s) = self.units.get_mut(&id) else {
            return false;
        };
        if s.on == Some(true) {
            return false;
        }
        s.on = Some(true);
        true
    }
    /// Counters wrap at their ability's period, independently of the world's attack IDs.
    pub(crate) fn counted(&mut self, id: Id, every: u32) -> bool {
        if every == 0 {
            return false;
        }
        let Some(s) = self.units.get_mut(&id) else {
            return false;
        };
        s.count = s.count.saturating_add(1);
        if s.count < every {
            return false;
        }
        s.count = 0;
        true
    }
    pub(crate) fn before_attack(&mut self, id: Id) {
        if let Some(s) = self.units.get_mut(&id) {
            if let Kit::Ability(Ability::Power { every, .. }) = s.definition.kit {
                s.on = Some(s.count + 1 == every);
            }
        }
    }
    pub(crate) fn triggered(&self, id: Id) -> bool {
        self.units.get(&id).is_some_and(|s| s.on == Some(true))
    }
    /// Pausing a silenced interval keeps its phase. Catch-up matches the source's four-pulse cap.
    pub(crate) fn pulses(&mut self, id: Id, dt: f64, interval: f64) -> usize {
        let s = self.units.get_mut(&id).unwrap();
        if interval <= 0. {
            return 1;
        }
        s.accumulator += dt;
        let mut n = 0;
        while s.accumulator >= interval - 1e-9 && n < 4 {
            s.accumulator -= interval;
            n += 1;
        }
        n
    }
    pub(crate) fn enhance(&mut self, id: Id) -> bool {
        let s = self.units.get_mut(&id).unwrap();
        if s.count == 5 {
            return false;
        }
        s.count += 1;
        true
    }
    pub(super) fn spawn_ability<H: buffs::Hooks>(
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
        ability: Ability,
    ) -> Result<(), String> {
        if matches!(ability, Ability::Nest { .. } | Ability::Boil { .. }) {
            w.unit_mut(id)?.immune.insert("sluggish".into());
        }
        let b = match ability {
            Ability::Unblockable | Ability::Run(_) => flag_buff("ab:unblockable", "unblockable"),
            Ability::Taunt(n) if n != 0. => {
                let mut b = Buff::default();
                b.key = Some("ab:taunt".into());
                b.runtime.persist = true;
                b.mods.insert("taunt".into(), n);
                b
            }
            Ability::HitShield(n) if n > 0 => {
                let mut b = Buff::default();
                b.key = Some("ab:hitShield".into());
                b.runtime.shield_hits = n;
                b.runtime.injury.shield_types = Some(vec![
                    crate::battle::damage::DamageType::Physical,
                    crate::battle::damage::DamageType::Arts,
                ]);
                b
            }
            _ => return Ok(()),
        };
        r.add(
            w,
            h,
            id,
            Spec {
                mods_present: !b.mods.is_empty(),
                flags_present: !b.flags.is_empty(),
                buff: b,
                ..Spec::default()
            },
        )?;
        Ok(())
    }
}
