// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded lifecycle components shared by deployment and attack kits.
use super::*;
use crate::battle::{buffs, health, skills, valued};
pub(crate) const DECAY: u64 = 1001;
pub(crate) const DOT: u64 = 1002;
#[derive(Default)]
pub(crate) struct Extra {
    pub effects: Mechanics,
    pub advanced: Advanced,
    pub tactics: Tactics,
    pub advanced_start_heal: f64,
    pub advanced_burst_hits: u32,
    pub advanced_range_extend: u32,
    pub support: Support,
    pub weakness: bool,
    pub tag_scale: Vec<(String, f64)>,
    pub deploy: bool,
    pub release_on_start: bool,
    pub duration: Option<f64>,
    pub hp_loss: f64,
    pub start_buff: Option<StartBuff>,
    pub skill_range: Option<Vec<[i32; 2]>>,
    pub no_heal: bool,
    pub cost_delta: f64,
    pub cost_aura: Option<CostAura>,
    pub hp_speed: Option<HpSpeed>,
    pub reveal: bool,
    pub dot: Option<Dot>,
    pub normal_status: Option<OnHitStatus>,
    pub skill_status: Option<OnHitStatus>,
    pub sp_hurt: Option<SpHurt>,
    pub lord: Option<Lord>,
    pub skill_arts: bool,
    pub self_heal: f64,
}
pub(crate) struct StartBuff {
    pub key: &'static str,
    pub shield: f64,
    pub def: f64,
    pub block: bool,
    pub decay: bool,
}
pub(crate) struct CostAura {
    pub value: f64,
    pub limit: f64,
}
pub(crate) struct HpSpeed {
    pub maximum: f64,
    pub minimum: f64,
    pub protect: f64,
    pub below: f64,
}
#[derive(Clone, Copy)]
pub(crate) struct Dot {
    pub duration: f64,
    pub interval: f64,
    pub amount: f64,
    pub sea: f64,
}
#[derive(Clone, Copy)]
pub(crate) struct OnHitStatus {
    pub key: &'static str,
    pub duration: f64,
    pub chance: Option<f64>,
}
pub(crate) struct SpHurt {
    pub amount: f64,
    pub grid: Vec<[i32; 2]>,
}
#[derive(Clone, Copy)]
pub(crate) struct Lord {
    pub scale: f64,
    pub arts: f64,
    pub skill_full: bool,
}
impl Extra {
    pub fn event<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        e: skills::Event,
        s: &skills::Runtime,
    ) -> Result<(), String> {
        let id = s.unit();
        if e == skills::Event::Start {
            if self.release_on_start {
                h.operator_release(w, r, id)?;
            }
            let hp = w.unit(id)?.stats.hp;
            let loss = hp * self.hp_loss;
            if loss > 0. && hp - loss >= 1. {
                health::lose_hp(
                    w,
                    h,
                    health::Hit {
                        source: Some(id),
                        credit: Some(id),
                        target: id,
                        damage_type: crate::battle::damage::DamageType::True,
                        no_sp: true,
                        silent: false,
                        hp_loss: true,
                        on_applied: false,
                    },
                    loss,
                    Default::default(),
                    false,
                )?;
            }
            if let Some(start) = &self.start_buff {
                let max = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("maxHp");
                let mut b = Buff {
                    key: Some(start.key.into()),
                    shield: max * start.shield,
                    ..Default::default()
                };
                if start.def != 0. {
                    b.mods.insert("defPct".into(), start.def);
                }
                b.runtime.duration = s.spec.duration + if start.block { 0.05 } else { 0. };
                b.runtime.shield_hits = if start.block { 1 } else { 0 };
                if start.decay {
                    b.runtime.interval = 1.;
                    b.runtime.on_tick = Some(DECAY);
                }
                r.add(
                    w,
                    h,
                    id,
                    buffs::Spec {
                        buff: b,
                        mods_present: start.def != 0.,
                        ..Default::default()
                    },
                )?;
            }
        }
        if e == skills::Event::End {
            if let Some(start) = &self.start_buff {
                r.remove(w, h, id, start.key)?;
            }
        }
        Ok(())
    }
    pub fn tick<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let u = w.unit(id)?;
        if !u.stats.alive || !u.deployed {
            return Ok(());
        }
        if let Some(p) = self
            .hp_speed
            .as_ref()
            .filter(|p| p.maximum > 0. && p.minimum < 1.)
        {
            let u = w.unit_mut(id)?;
            let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
            let ratio = if max > 0. { u.stats.hp / max } else { 0. };
            let v = p.maximum * ((1. - ratio) / (1. - p.minimum)).clamp(0., 1.);
            let current = u
                .stats
                .buffs()
                .iter()
                .find(|b| b.key.as_deref() == Some("utage:serious"))
                .and_then(|b| b.mods.get("aspd"))
                .copied();
            if current.map_or(true, |old| (old - v).abs() >= 0.5) {
                if v < 0.5 {
                    if current.is_some() {
                        r.remove(w, h, id, "utage:serious")?;
                    }
                } else {
                    r.add(
                        w,
                        h,
                        id,
                        buffs::Spec {
                            buff: Buff {
                                key: Some("utage:serious".into()),
                                mods: [("aspd".into(), v)].into(),
                                ..Default::default()
                            },
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        self.protect(w, r, h, id)?;
        Ok(())
    }
    pub fn protect<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        if let Some(p) = &self.hp_speed {
            let u = w.unit_mut(id)?;
            let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
            if u.stats.alive
                && u.deployed
                && p.protect > 0.
                && max > 0.
                && u.stats.hp / max < p.below
            {
                valued::apply(
                    w,
                    h,
                    r,
                    id,
                    valued::Request {
                        key: "protect",
                        kind: valued::Kind::Custom(|v| {
                            [
                                ("physTakenMul".into(), 1. - v),
                                ("artsTakenMul".into(), 1. - v),
                            ]
                            .into()
                        }),
                        plain: false,
                        duration: 0.05,
                        value: p.protect,
                        source: Some(id),
                        stack_as: None,
                    },
                )?;
            }
        }
        Ok(())
    }
}
impl CostAura {
    pub fn refresh<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        source: Id,
        allies: &[(Id, f64)],
    ) -> Result<(), String> {
        let u = w.unit(source)?;
        if !u.stats.alive || !u.deployed {
            return Ok(());
        }
        for &(id, cost) in allies {
            if cost > self.limit || !crate::battle::targeting::aura_selectable(w, id)? {
                continue;
            }
            if w.unit(id)?.stats.buffs().iter().any(|b| {
                b.key.as_deref() == Some("talent:gravel")
                    && b.runtime.source != Some(source)
                    && b.mods.get("defPct").copied().unwrap_or(0.) > self.value
                    && b.runtime.time_left > 0.05
            }) {
                continue;
            }
            let mut b = Buff {
                key: Some("talent:gravel".into()),
                mods: [("defPct".into(), self.value)].into(),
                ..Default::default()
            };
            b.runtime.duration = 0.6;
            b.runtime.source = Some(source);
            r.add(
                w,
                h,
                id,
                buffs::Spec {
                    buff: b,
                    mods_present: true,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
}
