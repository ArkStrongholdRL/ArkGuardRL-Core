// SPDX-License-Identifier: GPL-3.0-or-later
//! Layered covenant effects; values come from the canonical content blackboards.
use super::*;
use crate::battle::{
    deploy::Kind,
    skills,
    stats::{Buff, Numbers},
};

#[derive(Clone, Debug)]
pub struct BattleBond {
    pub id: String,
    pub tier: usize,
    pub layers: f64,
}
#[derive(Clone)]
struct Bond {
    id: String,
    tier: usize,
    bb: Numbers,
}
impl Bond {
    fn n(&self, key: &str) -> f64 {
        self.bb.get(key).copied().unwrap_or(0.)
    }
    fn value(&self, key: &str, default: f64) -> f64 {
        self.bb.get(key).copied().unwrap_or(default)
    }
}
mod aura;
mod damage;
mod defence;
mod deployment;
mod egir;
mod lifecycle;
mod passives;
mod raid;
mod revival;
mod yan;
#[derive(Default)]
pub(super) struct Runtime {
    plans: Vec<Arc<Bond>>,
    ammo_used: u64,
    reserved_map_chars: std::collections::BTreeSet<usize>,
    egir: egir::Egir,
    raid_next: f64,
    pub(super) raid_revived: std::collections::BTreeMap<Id, u64>,
    indom: revival::Indom,
    thorns_at: std::collections::BTreeMap<Id, f64>,
    skill_costs: std::collections::BTreeSet<Id>,
    aura: Aura,
    siracusa: deployment::Siracusa,
    kazimierz: deployment::Kazimierz,
}
struct Aura {
    pending: bool,
    next: f64,
}
impl Default for Aura {
    fn default() -> Self {
        Self {
            pending: false,
            next: 0.25,
        }
    }
}
const ORDER: &[&str] = &[
    "yanShip",
    "egirShip",
    "sargonShip",
    "victoriaShip",
    "kjeragShip",
    "lateranoShip",
    "siracusaShip",
    "kazimierzShip",
    "preciShip",
    "steadShip",
    "indomShip",
    "raidShip",
    "deputShip",
    "swiftShip",
    "arcaneShip",
    "skillfulShip",
    "emptyShip",
    "soloShip",
    "suntShip",
];
pub(super) const SIRACUSA_END: u64 = 0x424f4e440002;
pub(super) const SARGON_EXPIRE: u64 = 0x424f4e440001;
pub(super) fn core_bond(id: &str) -> bool {
    matches!(
        id,
        "yanShip"
            | "sargonShip"
            | "victoriaShip"
            | "kjeragShip"
            | "lateranoShip"
            | "egirShip"
            | "siracusaShip"
            | "kazimierzShip"
    )
}
impl Field {
    pub fn configure_bonds(&mut self, bonds: &[BattleBond]) -> Result<(), String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 {
            return Err("Bonds require an unstarted field".into());
        }
        if bonds.len() > 256 {
            return Err("Bond passive capacity exceeded".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut plans = Vec::new();
        for bond in bonds {
            if bond.tier > 64
                || !bond.layers.is_finite()
                || bond.layers < 0.
                || !seen.insert(&bond.id)
            {
                return Err("Invalid or unsupported battle bond".into());
            }
            let record = self
                .core
                .host
                .data
                .table("bonds")
                .and_then(|t| t.get(&bond.id))
                .ok_or("Missing battle bond")?;
            if bond.tier == 0 || !ORDER.contains(&bond.id.as_str()) {
                continue;
            }
            let key = format!(
                "{}autochess_bond_eff_{}",
                if matches!(bond.id.as_str(), "siracusaShip" | "kazimierzShip") {
                    "act2"
                } else {
                    "act1"
                },
                bond.id.trim_end_matches("Ship")
            );
            let bb = record["buffs"]
                .as_array()
                .and_then(|a| {
                    a.iter().find(|b| {
                        if core_bond(&bond.id) {
                            b["bbStr"]["key"] == key || b["key"] == key
                        } else {
                            b["key"] == "env_gbuff_new"
                        }
                    })
                })
                .map(|b| &b["bb"])
                .and_then(|b| b.as_object())
                .ok_or("Missing bond parameters")?;
            let mut numbers = Numbers::new();
            for (k, v) in bb {
                numbers.insert(
                    k.clone(),
                    v.as_f64()
                        .filter(|n| n.is_finite())
                        .ok_or("Invalid bond parameter")?,
                );
            }
            plans.push(Arc::new(Bond {
                id: bond.id.clone(),
                tier: bond.tier,
                bb: numbers,
            }));
        }
        plans.sort_by_key(|p| ORDER.iter().position(|id| *id == p.id).unwrap());
        let mut runtime = Runtime {
            reserved_map_chars: self.core.host.bonds.reserved_map_chars.clone(),
            plans,
            ..Default::default()
        };
        if let Some(p) = runtime
            .plans
            .iter()
            .find(|p| p.id == "siracusaShip" && p.tier >= 2)
        {
            runtime.siracusa.constant = deployment::prd_constant(p.n("prob"))?;
        }
        if let Some(p) = runtime.plans.iter().find(|p| {
            p.id == "kazimierzShip"
                && p.tier >= 2
                && p.n("damage_interval") > 0.
                && p.n("damage_atk_scale") > 0.
        }) {
            runtime.kazimierz.next = Some(p.n("damage_interval"));
        }
        let mut growth = super::growth::Growth::default();
        growth.live = bonds.iter().map(|b| (b.id.clone(), b.clone())).collect();
        growth.enabled = self.core.boss.is_none();
        self.core.host.growth = growth;
        self.core.host.bonds = runtime;
        Ok(())
    }
}
impl Host {
    pub(super) fn movement_bonds_quiet(&self) -> bool {
        !self.bonds.aura.pending
    }
    pub(super) fn movement_bond_deadline(&self) -> f64 {
        let mut at = self.bonds.kazimierz.next.unwrap_or(f64::INFINITY);
        if self.bonds.plans.iter().any(|p| p.id == "skillfulShip") {
            at = at.min(self.bonds.aura.next);
        }
        if self.bonds.plans.iter().any(|p| p.id == "raidShip") {
            at = at.min(self.bonds.raid_next.max(0.25));
        }
        at
    }
    fn bond_layers(&self, id: &str) -> f64 {
        self.growth.live.get(id).map_or(0., |b| b.layers)
    }
    pub(super) fn bond_elite(&self, id: Id) -> bool {
        self.roster
            .as_ref()
            .and_then(|r| r.entry(id).ok())
            .is_some_and(|e| e.definition.key.ends_with("_b"))
    }
}

#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum Periodic {
    Kazimierz,
    Aura,
    Raid,
}
impl Host {
    pub(in crate::battle::runtime) fn bond_deadlines(&self, w: &World) -> Vec<(f64, Periodic)> {
        let mut out = Vec::with_capacity(2);
        if let Some(at) = self.bonds.kazimierz.next.filter(|at| w.time >= *at) {
            out.push((at, Periodic::Kazimierz));
        }
        if self.bonds.plans.iter().any(|p| p.id == "skillfulShip") && w.time >= self.bonds.aura.next
        {
            out.push((self.bonds.aura.next, Periodic::Aura));
        }
        if self.bonds.plans.iter().any(|p| p.id == "raidShip")
            && w.time >= self.bonds.raid_next.max(0.25)
        {
            out.push((self.bonds.raid_next.max(0.25), Periodic::Raid));
        }
        out
    }
    pub(in crate::battle::runtime) fn bond_periodic(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: Periodic,
    ) -> Result<(), String> {
        match event {
            Periodic::Raid => {
                self.bonds.raid_next = self.bonds.raid_next.max(0.25) + 0.25;
                self.raid_poll(w, r)
            }
            Periodic::Kazimierz => self.bond_kazimierz_pulse(w, r),
            Periodic::Aura => {
                self.bonds.aura.next = w.time + 0.25;
                self.bonds_aura(w, r)
            }
        }
    }
    fn bond_ops(&self, w: &World) -> Vec<Id> {
        self.roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .filter(|&id| {
                w.unit(id)
                    .is_ok_and(|u| u.deployment.kind == Kind::Operator)
            })
            .collect()
    }
}

impl Runtime {
    pub(super) fn for_stage(stage: &Value) -> Self {
        let mut state = Self::default();
        for record in stage["mapChars"].as_array().into_iter().flatten() {
            if let (Some(r), Some(c)) = (record["pos"][0].as_u64(), record["pos"][1].as_u64()) {
                if r < crate::battle::grid::ROWS as u64 && c < COLS as u64 {
                    state
                        .reserved_map_chars
                        .insert(r as usize * COLS + c as usize);
                }
            }
        }
        state
    }
}
