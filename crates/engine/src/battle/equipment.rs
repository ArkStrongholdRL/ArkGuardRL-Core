// SPDX-License-Identifier: GPL-3.0-or-later
//! Equipment definitions shared by preparation and battle installation.
use super::stats::{Buff, Numbers};
use crate::data::Catalog;
use std::sync::Arc;

pub(crate) const DRAIN: u64 = 0xE901;
pub(crate) const PEGASUS_HEAL: u64 = 0xE902;
pub(crate) const FIXED_REDUCTION: u64 = 0xE903;

#[derive(Clone, Copy)]
pub(crate) enum Hammer {
    Burn(f64),
    Undying(f64),
    Speed(f64),
    Tremble { probability: f64, duration: f64 },
}
impl Hammer {
    pub fn index(self) -> usize {
        match self {
            Self::Burn(_) => 0,
            Self::Undying(_) => 1,
            Self::Speed(_) => 2,
            Self::Tremble { .. } => 3,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Steam {
    pub burn: f64,
    pub undying: f64,
    pub speed: f64,
    pub probability: f64,
    pub tremble: f64,
}
impl Steam {
    pub fn hammer(self, index: usize) -> Hammer {
        match index {
            0 => Hammer::Burn(self.burn),
            1 => Hammer::Undying(self.undying),
            2 => Hammer::Speed(self.speed),
            3 => Hammer::Tremble {
                probability: self.probability,
                duration: self.tremble,
            },
            _ => unreachable!(),
        }
    }
}
pub(crate) struct Hammers {
    pub own: [u32; 4],
    pub params: [Option<Hammer>; 4],
    pub steam: Option<Steam>,
    pub speed: f64,
    pub next: f64,
    pub order: usize,
    pub lock_at: Option<u64>,
    pub window_at: Option<u64>,
    pub until: f64,
}
#[derive(Clone, Copy)]
pub(crate) enum Timer {
    Effect(usize),
    Hammers,
}
pub(crate) struct FieldHammers {
    pub at: f64,
    pub present: [bool; 4],
}

#[derive(Clone, Copy)]
pub(crate) enum Effect {
    Silence(f64),
    Cocoon {
        cap: u32,
        used: u32,
    },
    Cauldron {
        speed: f64,
        cap: u32,
    },
    Chivalry {
        duration: f64,
        attack: f64,
        speed: f64,
        movement: f64,
        next: f64,
        until: f64,
        combo: bool,
        doomed: bool,
    },
    Crest {
        per: f64,
        cap: f64,
        scale: f64,
        next: f64,
        bonus: f64,
        deployment: u64,
    },
    PegasusHeal,
    PegasusDamage(f64),
    Trench {
        cooldown: f64,
        scale: f64,
        ready: f64,
    },
    Compass {
        initial: f64,
        end: f64,
        team: f64,
        spent: bool,
    },
    GunKnight {
        probability: f64,
        scale: f64,
        combo: f64,
    },
    Tears {
        interval: f64,
        next: f64,
        scale: f64,
        combo: f64,
    },
    Hammer(Hammer),
    Steam(Steam),
    DeploySp(f64),
    FrontHealth(f64),
    SideAura {
        speed: f64,
        next: f64,
    },
    AmmoClip {
        probability: f64,
        ratio: f64,
        cap: u32,
        used: u32,
        deployment: u64,
    },
    Weakness,
    Sniper {
        radius: f64,
        scale: f64,
    },
    SkillAttack {
        attack: f64,
        cap: u32,
        probability: f64,
        stacks: u32,
    },
    Grenade {
        window: f64,
        duration: f64,
    },
    Accelerator {
        speed: f64,
        cap: u32,
    },
    Camouflage {
        duration: f64,
        used: bool,
    },
    RiotShield(f64),
    AttackHeal(f64),
    Flag {
        scale: f64,
        window: f64,
        interval: f64,
        decay: f64,
    },
    StatusImmunity(f64),
    DamageStatus {
        key: &'static str,
        probability: f64,
        duration: f64,
    },
    HealShield {
        probability: f64,
        cap: u32,
    },
}

#[derive(Clone)]
pub(crate) struct Grant {
    pub buffs: Vec<Buff>,
    pub effects: Vec<Effect>,
    pub tag: Arc<str>,
    pub partner: Option<Arc<str>>,
}

#[derive(Clone)]
pub(crate) struct Instance {
    pub tag: Arc<str>,
    pub partner: Option<Arc<str>>,
    pub effect: Effect,
}
pub(crate) struct Loadout {
    pub effects: Vec<Instance>,
    pub bonds: Vec<String>,
    pub items: Vec<Arc<str>>,
    pub hammers: Option<Hammers>,
}
#[derive(Default)]
pub(crate) struct Runtime {
    pub units: std::collections::BTreeMap<super::health::Id, Loadout>,
    pub pegasus_damage: bool,
    pub gained_chess: u64,
    pub fields: std::collections::BTreeMap<super::health::Id, FieldHammers>,
}
impl Runtime {
    pub(crate) fn clock_quiet(&self) -> bool {
        self.fields.is_empty()
            && self
                .units
                .values()
                .all(|u| u.effects.is_empty() && u.hammers.is_none())
    }
    pub fn register(&mut self, unit: super::health::Id, grants: &[Grant], bonds: Vec<String>) {
        let effects: Vec<_> = grants
            .iter()
            .flat_map(|g| {
                g.effects.iter().map(|&effect| Instance {
                    tag: Arc::clone(&g.tag),
                    partner: g.partner.clone(),
                    effect,
                })
            })
            .collect();
        self.pegasus_damage |= effects
            .iter()
            .any(|instance| matches!(instance.effect, Effect::PegasusDamage(_)));
        let hammers = effects
            .iter()
            .position(|instance| matches!(instance.effect, Effect::Hammer(_) | Effect::Steam(_)))
            .map(|order| Hammers {
                own: [0; 4],
                params: [None; 4],
                steam: None,
                speed: 0.,
                next: 0.,
                order,
                lock_at: None,
                window_at: None,
                until: f64::NEG_INFINITY,
            });
        self.units.insert(
            unit,
            Loadout {
                effects,
                bonds,
                items: grants.iter().map(|g| Arc::clone(&g.tag)).collect(),
                hammers,
            },
        );
    }
    pub fn effects(&self, unit: super::health::Id) -> Vec<(usize, Instance)> {
        self.units
            .get(&unit)
            .into_iter()
            .flat_map(|loadout| &loadout.effects)
            .cloned()
            .enumerate()
            .collect()
    }
}

pub(crate) fn compile(data: &Catalog, id: &str) -> Result<Grant, String> {
    let raw = &data.table("items").unwrap()[id];
    if !raw.is_object()
        || raw["itemType"] != "EQUIP"
        || !matches!(
            raw["kind"].as_str(),
            Some(
                "passive"
                    | "passive_counter"
                    | "passive_player"
                    | "post_battle_transform"
                    | "consume_on_equip_or_delayed"
            )
        )
    {
        return Err(format!("Unsupported equipment: {id}"));
    }
    let buffs = raw["buffs"]
        .as_array()
        .ok_or_else(|| format!("Missing equipment buffs: {id}"))?;
    if buffs.len() > 64 {
        return Err("Equipment buff capacity exceeded".into());
    }
    let mut out = Vec::new();
    let mut effects = Vec::new();
    let mut partner = None;
    for b in buffs {
        if let Some(id) = b["bbStr"]["equip_chess_id"].as_str() {
            partner = Some(Arc::from(id));
        }
        let key = b["bbStr"]["key"]
            .as_str()
            .or(b["key"].as_str())
            .unwrap_or("");
        let n =
            |name: &str| super::waves::number(b["bbStr"].get(name).unwrap_or(&b["bb"][name]), 0.);
        let cap = |name| -> Result<u32, String> {
            let value = n(name).floor().max(1.);
            if value > i32::MAX as f64 {
                return Err("Equipment stack capacity exceeded".into());
            }
            Ok(value as u32)
        };
        let effect = match key {
            "attr_common_global_buff"
            | "act1autochess_equip_acarm056_global_buff"
            | "act1autochess_equip_acarm050_global_buff"
            | "magic_penetrate_global_buff"
            | "act1autochess_equip_acarm054_global_buff" => None,
            "periodic_damage" => {
                let mut buff = Buff {
                    key: Some(format!("item:{id}:drain")),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.persist = true;
                buff.runtime.interval = 1.;
                buff.runtime.tick_payload = n("damage");
                buff.runtime.on_tick = Some(DRAIN);
                out.push(buff);
                continue;
            }
            "act1autochess_equip_acarm076_global_buff" => Some(Effect::GunKnight {
                probability: n("prob"),
                scale: n("atk_scale_1"),
                combo: n("atk_scale_2"),
            }),
            "act2autochess_equip_acarm102_ability" => Some(Effect::Tears {
                interval: n("interval").max(0.1),
                next: n("interval").max(0.1),
                scale: n("atk_scale"),
                combo: n("atk_scale_ex"),
            }),
            "act2autochess_equip_acarm118_global_buff" => {
                let mut buff = Buff {
                    key: Some(format!("item:{id}:set")),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.persist = true;
                buff.runtime.interval = 1.;
                buff.runtime.tick_payload = n("hp_recovery_per_sec_by_max_hp_ratio");
                buff.runtime.on_tick = Some(PEGASUS_HEAL);
                out.push(buff);
                Some(Effect::PegasusHeal)
            }
            "act2autochess_equip_acarm117_global_buff" => {
                Some(Effect::PegasusDamage(n("atk_scale")))
            }
            "act2autochess_equip_acarm078_global_buff" => Some(Effect::Trench {
                cooldown: n("lock_duration"),
                scale: n("atk_scale"),
                ready: f64::NEG_INFINITY,
            }),
            "halfidle_block_fixed_damage" => {
                let mut buff = Buff {
                    key: Some(format!("item:{id}:fixedReduction")),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.persist = true;
                buff.runtime.tick_payload = n("value");
                buff.runtime.injury.on_injury = Some(FIXED_REDUCTION);
                out.push(buff);
                continue;
            }
            "act1autochess_equip_acarm103_global_buff" => Some(Effect::Compass {
                initial: n("init_sp"),
                end: n("sp"),
                team: n("addition_sp"),
                spent: false,
            }),
            "act1autochess_equip_acarm068_global_buff" => Some(Effect::Cocoon {
                cap: cap("max_respawn_cnt")?,
                used: 0,
            }),
            "act1autochess_equip_acarm077_global_buff" => Some(Effect::Cauldron {
                speed: n("attack_speed"),
                cap: cap("max_cnt")?,
            }),
            "equip_with_another_gain_coin_when_gain_char"
            | "act2autochess_equip_acarm119_ability" => continue,
            "act2autochess_equip_acarm119_global_buff" => {
                let ability = buffs
                    .iter()
                    .find(|v| v["bbStr"]["key"] == "act2autochess_equip_acarm119_ability");
                let a = |key: &str| {
                    ability.map_or(1., |v| {
                        super::waves::number(v["bbStr"].get(key).unwrap_or(&v["bb"][key]), 1.)
                    })
                };
                Some(Effect::Chivalry {
                    duration: n("duration"),
                    attack: n("atk"),
                    speed: a("attack_speed"),
                    movement: a("move_speed"),
                    next: 0.,
                    until: f64::NEG_INFINITY,
                    combo: false,
                    doomed: false,
                })
            }
            "act2autochess_equip_acarm122_global_buff" => Some(Effect::Crest {
                per: n("atk_per_sec"),
                cap: n("max_atk"),
                scale: n("atk_scale"),
                next: 0.25,
                bonus: 0.,
                deployment: 0,
            }),
            "silence_attachment" => Some(Effect::Silence(n("silence"))),
            "act1autochess_equip_acarm042_global_buff" => {
                Some(Effect::Hammer(Hammer::Burn(n("damage_scale"))))
            }
            "act1autochess_equip_acarm043_global_buff" => {
                Some(Effect::Hammer(Hammer::Undying(n("undeadable_duration"))))
            }
            "act1autochess_equip_acarm044_global_buff" => {
                Some(Effect::Hammer(Hammer::Speed(n("attack_speed"))))
            }
            "act1autochess_equip_acarm045_global_buff" => Some(Effect::Hammer(Hammer::Tremble {
                probability: n("prob"),
                duration: n("disarmed_duration"),
            })),
            "act1autochess_equip_acarm079_global_buff" => Some(Effect::Steam(Steam {
                burn: n("damage_scale"),
                undying: n("undeadable_duration"),
                speed: n("attack_speed"),
                probability: n("prob"),
                tremble: n("disarmed_duration"),
            })),
            "act1autochess_equip_acarm051_global_buff" => {
                Some(Effect::DeploySp(n("sp_each_person")))
            }
            "act1autochess_equip_acarm049_global_buff" => {
                let mut buff = Buff {
                    key: Some(format!("item:{id}:hp")),
                    mods: [("hpPct".into(), n("init_max_hp"))].into(),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.persist = true;
                out.push(buff);
                Some(Effect::FrontHealth(n("ex_max_hp")))
            }
            "act2autochess_equip_acarm121_ability" => Some(Effect::SideAura {
                speed: n("attack_speed"),
                next: 0.,
            }),
            "act1autochess_equip_acarm057_global_buff" => Some(Effect::AmmoClip {
                probability: n("prob"),
                ratio: n("ammo_percent"),
                cap: cap("max_trigger_cnt")?,
                used: 0,
                deployment: 0,
            }),
            "act1autochess_equip_acarm067_global_buff" => Some(Effect::Weakness),
            "act1autochess_equip_acarm063_global_buff" => Some(Effect::Sniper {
                radius: n("radius"),
                scale: n("damage_scale"),
            }),
            "act1vautochess_equip_acarm024_global_buff" => Some(Effect::SkillAttack {
                attack: n("atk"),
                cap: cap("atk_buff_cnt")?,
                probability: n("prob"),
                stacks: 0,
            }),
            "act1autochess_equip_acarm064_global_buff" => Some(Effect::Grenade {
                window: n("duration"),
                duration: n("stun"),
            }),
            "act1vautochess_equip_acarm037_global_buff" => Some(Effect::Accelerator {
                speed: n("attack_speed"),
                cap: cap("max_buff_cnt")?,
            }),
            "act2autochess_equip_acarm055_global_buff" => Some(Effect::Camouflage {
                duration: n("duration"),
                used: false,
            }),
            "act1autochess_equip_acarm060_global_buff" => {
                Some(Effect::RiotShield(n("damage_scale")))
            }
            "act1vautochess_equip_acarm025_global_buff" => Some(Effect::AttackHeal(n("hp_ratio"))),
            "act2autochess_equip_acarm120_global_buff" => Some(Effect::Flag {
                scale: n("damage_scale"),
                window: n("interval"),
                interval: n("ex_interval").max(0.1),
                decay: n("damage_scale_minus"),
            }),
            "act1autochess_equip_acarm059_global_buff" => {
                Some(Effect::StatusImmunity(n("hp_ratio")))
            }
            "act1autochess_equip_acarm058_global_buff" => Some(Effect::DamageStatus {
                key: "palsy",
                probability: n("prob"),
                duration: f64::INFINITY,
            }),
            "act1vautochess_equip_acarm003_global_buff" => Some(Effect::DamageStatus {
                key: "cold",
                probability: n("prob"),
                duration: n("cold"),
            }),
            "act1autochess_equip_acarm061_global_buff" => Some(Effect::HealShield {
                probability: n("prob"),
                cap: cap("max_stack_cnt")?,
            }),
            "equip_round_start_upgrade_char"
            | "sell_char_count_gain_equip_owner_bond"
            | "char_chess_transformation_equip" => None,
            _ => return Err(format!("Unsupported equipment effect {key}: {id}")),
        };
        if let Some(effect) = effect {
            effects.push(effect);
            continue;
        }
        let mut mods = Numbers::new();
        for (from, to) in [
            ("atk", "atkPct"),
            ("def", "defPct"),
            ("max_hp", "hpPct"),
            ("attack_speed", "aspd"),
            ("magic_resistance", "resFlat"),
            ("sp_recovery_per_sec", "spRecoveryFlat"),
            ("magic_resist_penetrate", "resIgnorePct"),
            ("taunt_level", "taunt"),
        ] {
            let value = n(from);
            if value != 0. {
                mods.insert(
                    to.into(),
                    if from == "max_hp" {
                        value.max(-0.99)
                    } else {
                        value
                    },
                );
            }
        }
        if n("respawn_time") != 0. {
            mods.insert("redeployMul".into(), (1. + n("respawn_time")).max(0.));
        }
        if !mods.is_empty() {
            let mut buff = Buff {
                key: Some(format!("item:{id}:stat:{key}")),
                mods,
                permanent: true,
                ..Default::default()
            };
            buff.runtime.persist = true;
            out.push(buff);
        }
    }
    Ok(Grant {
        buffs: out,
        effects,
        tag: format!("item:{id}").into(),
        partner,
    })
}

pub(crate) fn bonds(data: &Catalog, key: &str, items: &[String]) -> Vec<String> {
    let mut out: Vec<_> = data.table("chess").unwrap()[key]["bonds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::to_owned)
        .collect();
    let table = data.table("items").unwrap();
    if items.len() >= 2 && items.iter().any(|id| table[id]["canGiveBond"] == true) {
        for id in items {
            if table[id]["canGiveBond"] == true {
                continue;
            }
            if let Some(bond) = table[id]["giveBondId"]
                .as_str()
                .filter(|id| data.table("bonds").unwrap()[*id].is_object())
            {
                if !out.iter().any(|b| b == bond) {
                    out.push(bond.into());
                }
            }
        }
    }
    out
}
