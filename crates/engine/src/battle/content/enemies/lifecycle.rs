// SPDX-License-Identifier: GPL-3.0-or-later
use super::movement::Cooldown;
use super::*;
#[derive(Clone, Copy, Debug)]
pub enum Lifecycle {
    Warden {
        duration: f64,
        res: f64,
        atk: f64,
        defense: f64,
        bat: f64,
        speed: f64,
        every: [u32; 2],
        stun: f64,
        targets: usize,
        times: u32,
        penetration: f64,
    },
    Ugly {
        duration: f64,
        melee: f64,
        bomb: f64,
        stun: f64,
        speed: f64,
        defense: Option<f64>,
        res: Option<f64>,
        burn: f64,
    },
    Reid {
        duration: f64,
        invincible: f64,
        hp: f64,
        ratio: f64,
        atk: f64,
        skill: Option<Cooldown>,
        speed: f64,
        rush: f64,
    },
    Vine {
        attack: f64,
        time: f64,
        first: f64,
        normal: f64,
    },
    Ignitable {
        scale: f64,
    },
    Brood {
        skill: Option<Cooldown>,
    },
    Seed,
    Reborn {
        duration: f64,
        invincible: f64,
        neural: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RebornPhase {
    Fresh,
    Rebirth,
    Revived,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Memory {
    None,
    Vine {
        start: f64,
        first: bool,
    },
    Ignitable {
        on: bool,
    },
    Brood {
        left: f64,
    },
    Seed {
        projectile: Option<u64>,
    },
    Reborn(RebornPhase),
    Warden {
        phase: RebornPhase,
        count: u32,
        power: bool,
    },
    Ugly {
        phase: RebornPhase,
        splash: bool,
    },
    Reid {
        phase: RebornPhase,
        left: f64,
        enraged: bool,
    },
}
impl Lifecycle {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key| talent(raw, ov, &[key]);
        let v = |key, fallback| talent_value(raw, ov, key).unwrap_or(fallback);
        let skill = |key| abilities::skill(raw, ov, key);
        let every = |key| {
            let sp = skill(key)["spCost"].as_f64().unwrap_or(0.);
            abilities::count(if sp > 0. { sp + 1. } else { 3. })
        };
        Ok(Some(match key {
            "enemy_1516_jakill" => Self::Warden {
                duration: t("reborn.duration"),
                res: t("enhance.magic_resistance"),
                atk: t("enhance.atk"),
                defense: t("enhance.def"),
                bat: t("enhance.base_attack_time"),
                speed: t("enhance.move_speed"),
                every: [every("ironsandstorm")?, every("armorpiercing")?],
                stun: skill("ironsandstorm")["bb"]["stun"].as_f64().unwrap_or(0.),
                targets: abilities::count(
                    skill("ironsandstorm")["bb"]["max_target"]
                        .as_f64()
                        .unwrap_or(1.),
                )? as usize,
                times: abilities::count(
                    skill("armorpiercing")["bb"]["times"]
                        .as_f64()
                        .unwrap_or(1.)
                        .min(64.),
                )?,
                penetration: skill("armorpiercing")["bb"]["def_penetrate"]
                    .as_f64()
                    .unwrap_or(0.),
            },
            "enemy_1512_mcmstr" => Self::Ugly {
                duration: t("reborn.duration"),
                melee: v("combat.attack@mcmstr_rage_attack.atk_scale", 1.),
                bomb: skill("bomb[reborning]")["bb"]["atk_scale"]
                    .as_f64()
                    .unwrap_or(0.),
                stun: skill("bomb[reborning]")["bb"]["stun"]
                    .as_f64()
                    .unwrap_or(0.),
                speed: t("bird_run.move_speed"),
                defense: talent_value(raw, ov, "bird_run.def"),
                res: talent_value(raw, ov, "bird_run.magic_resistance"),
                burn: t("bird_run.damage"),
            },
            "enemy_1539_reid" => Self::Reid {
                duration: t("Reborn.duration"),
                invincible: t("Reborn.invincible"),
                hp: v("Reborn.hp_ratio", 0.5),
                ratio: v("atkup.hp_ratio", 0.5),
                atk: talent_value(raw, ov, "AtkUp.atk").unwrap_or_else(|| t("atkup.atk")),
                skill: Cooldown::compile(raw, ov, "Rush"),
                speed: skill("Rush")["bb"]["move_speed"].as_f64().unwrap_or(0.),
                rush: skill("Rush")["bb"]["duration"].as_f64().unwrap_or(0.),
            },
            "enemy_10065_ftzlc" => Self::Ignitable {
                scale: t("boom.atk_scale"),
            },
            "enemy_10067_ftsjc" => Self::Vine {
                attack: t("pow.add_max_atk"),
                time: t("pow.time"),
                first: t("pow.attack@ep_damage_ratio"),
                normal: t("pow.attack@ep_damage_ratio_normal"),
            },
            "enemy_1321_wdarft" => Self::Brood {
                skill: Cooldown::compile(raw, ov, "BornBugs"),
            },
            "enemy_1269_nhfly" => Self::Seed,
            "enemy_10098_crhro" => Self::Reborn {
                duration: t("reborn.duration"),
                invincible: t("reborn.invincible"),
                neural: t("inside.attack@ep_damage_ratio"),
            },
            _ => return Ok(None),
        }))
    }
    pub(crate) fn rebirth(self) -> Option<(f64, f64, f64)> {
        match self {
            Self::Reborn {
                duration,
                invincible,
                ..
            } => Some((duration, invincible, 1.)),
            Self::Warden { duration, .. } | Self::Ugly { duration, .. } => Some((duration, 0., 1.)),
            Self::Reid {
                duration,
                invincible,
                hp,
                ..
            } => Some((duration, invincible, hp.clamp(0., 1.))),
            _ => None,
        }
    }
    pub(super) fn spawn<H: buffs::Hooks>(
        self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        match self {
            Self::Warden { res, .. } => {
                let mut b = Buff::default();
                b.key = Some("ab:warden".into());
                b.mods.insert("resFlat".into(), res);
                b.runtime.persist = true;
                b.runtime.visible = true;
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
            Self::Ugly { .. } => {
                w.unit_mut(id)?.stats.edit_base(|b| {
                    let r = b.0.get("rangeRadius").copied().unwrap_or(0.);
                    b.0.insert("rangeRadius".into(), r.max(2.5));
                });
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn memory(self, time: f64) -> Memory {
        match self {
            Self::Warden { .. } => Memory::Warden {
                phase: RebornPhase::Fresh,
                count: 0,
                power: false,
            },
            Self::Ugly { .. } => Memory::Ugly {
                phase: RebornPhase::Fresh,
                splash: false,
            },
            Self::Reid { skill, .. } => Memory::Reid {
                phase: RebornPhase::Fresh,
                left: skill.map(|s| s.initial).unwrap_or(0.),
                enraged: false,
            },
            Self::Vine { .. } => Memory::Vine {
                start: time,
                first: true,
            },
            Self::Ignitable { .. } => Memory::Ignitable { on: false },
            Self::Brood { skill } => Memory::Brood {
                left: skill.map(|s| s.initial).unwrap_or(0.),
            },
            Self::Seed => Memory::Seed { projectile: None },
            Self::Reborn { .. } => Memory::Reborn(RebornPhase::Fresh),
        }
    }
}
impl Runtime {
    pub(crate) fn reborn_phase(&self, id: Id) -> Option<RebornPhase> {
        if let Kit::Caster(c) = self.kit_ref(id) {
            if c.rebirth().is_some() {
                return self.caster(id).phase;
            }
        }
        match self.lifecycle(id) {
            Memory::Reborn(p)
            | Memory::Warden { phase: p, .. }
            | Memory::Ugly { phase: p, .. }
            | Memory::Reid { phase: p, .. } => Some(p),
            _ => None,
        }
    }
    pub(crate) fn set_reborn_phase(&mut self, id: Id, p: RebornPhase) {
        let caster = matches!(self.kit_ref(id), Kit::Caster(c) if c.rebirth().is_some());
        if caster {
            self.units.get_mut(&id).unwrap().caster.phase = Some(p);
            return;
        }
        match &mut self.units.get_mut(&id).unwrap().lifecycle {
            Memory::Reborn(phase)
            | Memory::Warden { phase, .. }
            | Memory::Ugly { phase, .. }
            | Memory::Reid { phase, .. } => *phase = p,
            _ => unreachable!(),
        }
    }
    pub(crate) fn lifecycle(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.lifecycle)
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_lifecycle(&mut self, id: Id, state: Memory) {
        self.units.get_mut(&id).unwrap().lifecycle = state;
    }
}
