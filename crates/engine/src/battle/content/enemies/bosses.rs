// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    casters::{Caster, Passive, Spell},
    *,
};
pub(crate) mod ensemble;
pub(crate) mod gun;
pub(crate) mod quintus;
pub use ensemble::{Echo, EchoForm, Pipe};
pub use quintus::Quintus;
pub(crate) mod helm;
pub use gun::{Gun, Spring, SpringKind};
pub use helm::{Blade, Helm};
#[derive(Clone, Copy, Debug)]
pub struct Store {
    pub count: usize,
    pub scale: f64,
    pub shield: f64,
    pub cut: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Lion {
    pub caster: Caster,
    pub advance: f64,
    pub defense: f64,
    pub resistance: f64,
    pub cost: f64,
    pub stores: [Store; 2],
}
#[derive(Clone, Copy, Debug)]
pub struct Deer {
    pub caster: Caster,
    pub resistance: f64,
    pub columns: usize,
    pub surge: usize,
}
#[derive(Clone, Copy, Debug)]
pub enum Boss {
    Pipe(Pipe),
    Echo(Echo),
    Quintus(Quintus),
    Gun(Gun),
    Spring(Spring),
    Helm(Helm),
    Blade(Blade),
    Shell { stun: f64, dot: f64 },
    Lion(Lion),
    Deer(Deer),
    Equipment,
}
#[derive(Clone, Debug)]
pub(crate) enum Memory {
    Pipe {
        acc: f64,
        attack: f64,
        held_until: f64,
    },
    Echo(ensemble::EchoState),
    Quintus(quintus::QuintusState),
    Gun(gun::GunState),
    Spring(gun::SpringState),
    Helm {
        low: bool,
    },
    Blade {
        phase: helm::BladePhase,
        home: [f64; 2],
    },
    Shell {
        target: Option<helm::ShellTarget>,
    },
    None,
    Lion {
        sp: f64,
        advanced: bool,
        next_anim: Option<f64>,
        wand: bool,
    },
    Deer {
        low: bool,
        acc: f64,
        held_until: f64,
    },
}
impl Boss {
    pub(super) fn compile(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        if let Some(boss) = Self::ensemble(key, raw, ov)? {
            return Ok(Some(boss));
        }
        if let Some(boss) = Self::quintus(key, raw, ov) {
            return Ok(Some(boss));
        }
        if let Some(boss) = Self::gun(key, raw, ov)? {
            return Ok(Some(boss));
        }
        if let Some(boss) = Self::helm(key, raw, ov)? {
            return Ok(Some(boss));
        }
        let t = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let sk = |key| abilities::skill(raw, ov, key);
        let n = |v: &Value, default| v.as_f64().filter(|v| v.is_finite()).unwrap_or(default);
        let bb = |key, field, default| n(&sk(key)["bb"][field], default);
        let make = |key, spell, clip| {
            casters::ability(raw, ov, key, spell, clip, true, false).map(|mut s| {
                s.own = false;
                s
            })
        };
        let caster = |skills| Caster {
            passive: Passive::Plain,
            skills,
        };
        Ok(Some(match key {
            "enemy_9032_aclionk" => {
                let cost = n(&sk("equip")["spCost"], 0.);
                let store = |key, count| -> Result<Store, String> {
                    Ok(Store {
                        count: if key == "store" {
                            1
                        } else {
                            abilities::count(bb(key, "max_target", count).max(0.).floor())? as usize
                        },
                        scale: bb(key, "atk_scale", 0.),
                        shield: bb(key, "dynamic", 0.),
                        cut: bb(key, "damage_resistance", 0.),
                    })
                };
                Self::Lion(Lion {
                    caster: caster([
                        make(
                            "equip",
                            Spell::Decree {
                                scales: [
                                    bb("equip", "atk_scale", 0.),
                                    bb("equipA", "atk_scale", bb("equip", "atk_scale", 0.)),
                                ],
                            },
                            3.667,
                        ),
                        make(
                            "Suicide",
                            Spell::Repel {
                                scale: bb("Suicide", "atk_scale", 0.),
                                loss: bb("Suicide", "value", 0.).abs(),
                            },
                            5.667,
                        ),
                        None,
                        None,
                    ]),
                    advance: t("advance.hp_ratio", 0.),
                    defense: t("advance.def", 0.),
                    resistance: t("advance.magic_resistance", 0.),
                    cost: if sk("equip").is_null() {
                        f64::INFINITY
                    } else if cost > 0. {
                        cost
                    } else {
                        25.
                    },
                    stores: [
                        store("store", 1.)?,
                        if sk("storeA").is_null() {
                            store("store", 1.)?
                        } else {
                            store("storeA", 2.)?
                        },
                    ],
                })
            }
            "enemy_9033_acdeer" => Self::Deer(Deer {
                caster: caster([
                    make(
                        "Lasso",
                        Spell::Surge {
                            max: abilities::count(bb("Lasso", "max_target", 1.).max(0.).floor())?
                                as usize,
                            duration: bb("Lasso", "projectile_life_time", 0.),
                            scale: bb("Lasso", "atk_scale", 0.),
                        },
                        0.,
                    ),
                    make(
                        "Doom",
                        Spell::Repel {
                            scale: bb("Doom", "atk_scale", 0.),
                            loss: bb("Doom", "value", 0.).abs(),
                        },
                        2.667,
                    ),
                    None,
                    None,
                ]),
                resistance: t("Madness.damage_resistance", 0.),
                columns: abilities::count(
                    t("Madness.enemy_smdeer_mad[attack].max_cnt", 1.)
                        .max(0.)
                        .ceil(),
                )? as usize,
                surge: abilities::count(
                    t("Madness.enemy_smdeer_mad[skill].max_target", 1.)
                        .max(0.)
                        .floor(),
                )? as usize,
            }),
            "enemy_10028_vtswd" | "enemy_10029_vtshld" | "enemy_10030_vtwand" => Self::Equipment,
            _ => return Ok(None),
        }))
    }
    pub(crate) fn caster(self) -> Caster {
        self.caster_ref().copied().unwrap_or(casters::EMPTY)
    }
    /// Embedded caster configuration; dynamic Quintus and synthetic parts have none.
    pub(crate) fn caster_ref(&self) -> Option<&Caster> {
        match self {
            Self::Pipe(c) => Some(&c.caster),
            Self::Echo(c) => Some(&c.caster),
            Self::Gun(c) => Some(&c.caster),
            Self::Helm(c) => Some(&c.caster),
            Self::Blade(c) => Some(&c.caster),
            Self::Lion(c) => Some(&c.caster),
            Self::Deer(c) => Some(&c.caster),
            Self::Quintus(_) | Self::Spring(_) | Self::Shell { .. } | Self::Equipment => None,
        }
    }
    pub(super) fn memory(self, position: [f64; 2], now: f64) -> Memory {
        match self {
            Self::Pipe(_) => Memory::Pipe {
                acc: 0.,
                attack: f64::INFINITY,
                held_until: 0.,
            },
            Self::Echo(_) => Memory::Echo(ensemble::EchoState {
                form: EchoForm::Dark,
                strikes: 0,
                pulses: Default::default(),
                pulsing: false,
            }),
            Self::Quintus(_) => Memory::Quintus(quintus::QuintusState {
                stage: 0,
                born_at: now,
                stage_at: now,
                tentacles: 0,
            }),
            Self::Gun(_) => Memory::Gun(gun::GunState {
                last: None,
                stacks: 0,
                charge: None,
                link_acc: 0.,
            }),
            Self::Spring(_) => Memory::Spring(gun::SpringState {
                up: true,
                down_at: None,
                attacks: 0,
                dash: None,
            }),
            Self::Helm(_) => Memory::Helm { low: false },
            Self::Blade(_) => Memory::Blade {
                phase: helm::BladePhase::Hover,
                home: position,
            },
            Self::Shell { .. } => Memory::Shell { target: None },
            Self::Lion(_) => Memory::Lion {
                sp: 0.,
                advanced: false,
                next_anim: None,
                wand: false,
            },
            Self::Deer(_) => Memory::Deer {
                low: false,
                acc: f64::INFINITY,
                held_until: 0.,
            },
            Self::Equipment => Memory::None,
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
        b.runtime.persist = true;
        match self {
            Self::Pipe(config) => {
                if !config.gold {
                    let mut bound = flag_buff("boss:selfBound", "selfBound");
                    bound
                        .flags
                        .extend(["noMove", "unblockable"].map(str::to_string));
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: bound,
                            flags_present: true,
                            ..Default::default()
                        },
                    )?;
                }
                if config.raw_aspd {
                    let mut speed = Buff::default();
                    speed.key = Some("boss:aspd".into());
                    speed.runtime.persist = true;
                    speed
                        .mods
                        .insert("aspd".into(), 20. - w.unit(id)?.stats.base_number("aspd"));
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: speed,
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
                if config.gold {
                    let mut eternal = flag_buff("boss:eternal", "invulnerable");
                    eternal.flags.insert("untargetable".into());
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: eternal,
                            flags_present: true,
                            ..Default::default()
                        },
                    )?;
                }
                return Ok(());
            }
            Self::Echo(config) => {
                let hits = (w.unit(id)?.stats.base_number("maxHp") + 0.5)
                    .floor()
                    .max(1.);
                w.unit_mut(id)?.stats.edit_base(|base| {
                    base.0.insert("maxHp".into(), hits);
                });
                w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
                w.unit_mut(id)?.stats.hp = hits;
                w.unit_mut(id)?.block_weight = 2.;
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: flag_buff("ab:hitCount", "hitCount"),
                        flags_present: true,
                        ..Default::default()
                    },
                )?;
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: ensemble::form_buff(config, EchoForm::Dark),
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
                return Ok(());
            }
            Self::Gun(config) => {
                b.key = Some("boss:unblockable".into());
                b.flags.insert("unblockable".into());
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
                if config.raw_aspd {
                    let mut speed = Buff::default();
                    speed.key = Some("boss:aspd".into());
                    speed.runtime.persist = true;
                    speed
                        .mods
                        .insert("aspd".into(), 20. - w.unit(id)?.stats.base_number("aspd"));
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: speed,
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
                return Ok(());
            }
            Self::Spring(config) => {
                b.key = Some("boss:springUnblockable".into());
                b.flags.insert("unblockable".into());
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
                let shield = gun::spring_shield(w, id, config)?;
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: shield,
                        ..Default::default()
                    },
                )?;
                if config.kind != SpringKind::Arts {
                    let mut share = Buff::default();
                    share.key = Some("boss:springShare".into());
                    share.runtime.persist = true;
                    share.runtime.injury.observe = Some(gun::SPRING_SHARE);
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: share,
                            ..Default::default()
                        },
                    )?;
                }
                return Ok(());
            }
            Self::Shell { .. } => {
                let hits = w.unit(id)?.stats.base_number("maxHp");
                let hits = (hits + 0.5).floor().max(1.);
                w.unit_mut(id)?.stats.edit_base(|base| {
                    base.0.insert("maxHp".into(), hits);
                });
                w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
                w.unit_mut(id)?.stats.hp = hits;
                let count = flag_buff("ab:hitCount", "hitCount");
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: count,
                        flags_present: true,
                        ..Default::default()
                    },
                )?;
                b.key = Some("boss:shellGuard".into());
                b.flags.insert("noDisplace".into());
            }
            Self::Blade(_) => {
                let mut hover = Buff::default();
                hover.key = Some("boss:hover".into());
                hover.runtime.persist = true;
                hover.runtime.visible = true;
                hover
                    .flags
                    .extend(["invulnerable", "untargetable"].map(str::to_string));
                r.add(
                    w,
                    h,
                    id,
                    Spec {
                        buff: hover,
                        flags_present: true,
                        ..Default::default()
                    },
                )?;
                b.key = Some("boss:anchor".into());
                b.flags.extend(
                    ["noMove", "selfBound", "unblockable", "noDisplace"].map(str::to_string),
                );
            }
            Self::Equipment => {
                b.key = Some("boss:equip".into());
                b.flags
                    .extend(["invulnerable", "untargetable", "noMove"].map(str::to_string));
            }
            _ => {
                b.key = Some("boss:selfBound".into());
                b.flags
                    .extend(["noMove", "selfBound", "unblockable"].map(str::to_string));
            }
        }
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
    pub(crate) fn boss(&self, id: Id) -> Memory {
        self.units
            .get(&id)
            .map(|s| s.boss.clone())
            .unwrap_or(Memory::None)
    }
    pub(crate) fn set_boss(&mut self, id: Id, memory: Memory) {
        self.units.get_mut(&id).unwrap().boss = memory;
    }
    pub(crate) fn lose_lp(&mut self, amount: f64) -> Result<(), String> {
        let total = self.lp_loss + amount;
        if !amount.is_finite() || amount < 0. || !total.is_finite() {
            return Err("Invalid field LP loss".into());
        }
        self.lp_loss = total;
        Ok(())
    }
    pub(crate) fn lp_loss(&self) -> f64 {
        self.lp_loss
    }
}
