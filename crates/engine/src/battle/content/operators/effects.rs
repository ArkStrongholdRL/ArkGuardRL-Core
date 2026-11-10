// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared content components; all geometry and settlement use engine primitives.
use super::*;
pub(crate) struct DeathHeal {
    pub ratio: f64,
    pub grid: Option<Vec<[i32; 2]>>,
}
impl DeathHeal {
    pub fn amount(
        &self,
        w: &mut World,
        source: Id,
        victim: Id,
        dir: Direction,
    ) -> Result<Option<f64>, String> {
        let u = w.unit(source)?;
        if !u.stats.alive || !u.deployed || u.removed {
            return Ok(None);
        }
        let near = if let Some(grid) = &self.grid {
            let keys = crate::battle::targeting::absolute_range(grid, u.deployment.tile, dir, 0);
            body::in_range(w.unit(victim)?, &keys)
        } else {
            let e = w.unit(victim)?;
            (u.position[0] - e.position[0])
                .abs()
                .max((u.position[1] - e.position[1]).abs())
                <= 1.
        };
        Ok(if near {
            Some(
                w.unit_mut(source)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("maxHp")
                    * self.ratio,
            )
        } else {
            None
        })
    }
}
#[derive(Clone, Copy)]
pub(crate) struct PhysicalResistance {
    pub above_hp: f64,
    pub fraction: f64,
}
impl PhysicalResistance {
    pub fn apply(self, w: &mut World, q: &mut hit::Request) -> Result<(), String> {
        if q.hit.damage_type != crate::battle::damage::DamageType::Physical {
            return Ok(());
        }
        let u = w.unit_mut(q.hit.target)?;
        let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
        let ratio = if max > 0. { u.stats.hp / max } else { 0. };
        if ratio > self.above_hp {
            q.damage.mul *= 1. - self.fraction;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct AttackProc {
    pub key: &'static str,
    pub normal: f64,
    pub active: f64,
    pub atk: f64,
}
impl AttackProc {
    pub fn roll(self, active: bool, rng: &mut Rng) -> Option<Buff> {
        let p = if active { self.active } else { self.normal };
        if p <= 0. || !rng.chance(p) {
            return None;
        }
        let mut b = Buff {
            key: Some(self.key.into()),
            mods: [("atkPct".into(), self.atk)].into(),
            ..Default::default()
        };
        b.runtime.duration = 0.1;
        Some(b)
    }
}

pub(crate) struct AreaCast {
    pub grid: Vec<[i32; 2]>,
    pub scale: f64,
    pub hits: u32,
    pub stun: f64,
}
pub(crate) struct SkillStart {
    pub dp: f64,
    pub area: Option<AreaCast>,
}
pub(crate) struct GridScale {
    pub grid: Vec<[i32; 2]>,
    pub scale: f64,
    pub skill_all: bool,
}
impl GridScale {
    pub fn multiplier(
        &self,
        w: &World,
        source: Id,
        target: Id,
        dir: Direction,
        skill: bool,
    ) -> Result<f64, String> {
        let keys = crate::battle::targeting::absolute_range(
            &self.grid,
            w.unit(source)?.deployment.tile,
            dir,
            0,
        );
        Ok(
            if skill && self.skill_all || body::in_range(w.unit(target)?, &keys) {
                self.scale
            } else {
                1.
            },
        )
    }
}

pub(crate) struct WhileBlocked {
    pub key: &'static str,
    pub mods: Numbers,
}
pub(crate) struct GroundBonus {
    pub key: &'static str,
    pub count: f64,
    pub normal: f64,
    pub surrounded: f64,
}
pub(crate) struct Aura {
    pub key: &'static str,
    pub grid: Vec<[i32; 2]>,
    pub skill_grid: Option<Vec<[i32; 2]>>,
    pub atk: f64,
    pub skill_scale: f64,
}
impl Aura {
    pub fn refresh<H: crate::battle::buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut crate::battle::buffs::Runtime,
        h: &mut H,
        source: Id,
        dir: Direction,
        active: bool,
    ) -> Result<(), String> {
        use crate::battle::{buffs, health::Side, targeting};
        let u = w.unit(source)?;
        let on = u.stats.alive && u.deployed && !u.removed;
        let grid = if active {
            self.skill_grid.as_ref().unwrap_or(&self.grid)
        } else {
            &self.grid
        };
        let keys = targeting::absolute_range(grid, u.deployment.tile, dir, 0);
        let ids: Vec<_> = w.unit_ids().collect();
        for id in ids {
            let u = w.unit(id)?;
            if u.side != Side::Ally {
                continue;
            }
            let eligible = on
                && u.targeting.melee_operator
                && targeting::aura_selectable(w, id)?
                && body::in_range(w.unit(id)?, &keys);
            if eligible {
                let mut b = Buff {
                    key: Some(self.key.into()),
                    mods: [(
                        "atkPct".into(),
                        self.atk * if active { self.skill_scale } else { 1. },
                    )]
                    .into(),
                    ..Default::default()
                };
                b.runtime.duration = 0.45;
                b.runtime.source = Some(source);
                b.runtime.refresh = buffs::Refresh::Extend;
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
            } else if w
                .unit(id)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(self.key) && b.runtime.source == Some(source))
            {
                r.remove(w, h, id, self.key)?;
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub(crate) struct ElementAttach {
    pub normal: f64,
    pub leader: f64,
}

pub(crate) fn passive_garrisons(
    data: &crate::data::Catalog,
    raw: &Value,
) -> Result<Vec<Buff>, String> {
    let mut out = Vec::new();
    let ids = raw["garrisonIds"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if ids.len() > crate::battle::grid::N {
        return Err("Garrison list exceeds field bound".into());
    }
    for gid in ids {
        let Some(key) = gid.as_str() else {
            return Err("Expected garrison id".into());
        };
        let g = &data.table("garrisons").ok_or("Missing garrison data")?[key];
        if g["eventType"] != "IN_BATTLE" {
            continue;
        }
        let mut mods = Numbers::new();
        match g["effectKey"].as_str() {
            Some("GAIN_BUFF") => {
                for (from, to) in [("atk", "atkMul"), ("max_hp", "hpMul"), ("def", "defMul")] {
                    let n = number(&g["bb"][from], 0.);
                    if n > 0. {
                        mods.insert(to.into(), n);
                    }
                }
            }
            Some("attr_common_global_buff") => {
                for (from, to) in [
                    ("sp_recovery_per_sec", "spRecoveryFlat"),
                    ("atk", "atkPct"),
                    ("max_hp", "hpPct"),
                    ("attack_speed", "aspd"),
                ] {
                    let n = number(&g["bb"][from], 0.);
                    if n != 0. {
                        mods.insert(to.into(), n);
                    }
                }
            }
            _ => continue,
        }
        let mut b = Buff {
            key: Some(format!("gar:{key}")),
            mods,
            ..Default::default()
        };
        b.runtime.persist = true;
        out.push(b);
    }
    Ok(out)
}
