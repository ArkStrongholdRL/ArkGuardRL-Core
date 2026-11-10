// SPDX-License-Identifier: GPL-3.0-or-later
//! Deployment-driven core covenants: Siracusa and Kazimierz.
use super::*;
use crate::battle::{
    damage::{Damage, DamageType},
    injury::DodgeMode,
};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Siracusa {
    pub constant: f64,
    attempts: u64,
    ended: BTreeMap<Id, f64>,
}
#[derive(Default)]
pub(super) struct Kazimierz {
    deploys: u64,
    pub next: Option<f64>,
}

// Keep the reference's summation order; a probability outside the supported
// convergence bound is rejected before changing the field configuration.
pub(super) fn prd_constant(p: f64) -> Result<f64, String> {
    if p <= 0. {
        return Ok(0.);
    }
    if p >= 1. {
        return Ok(1.);
    }
    let rate = |c: f64| -> Result<f64, String> {
        let mut expected = 0.;
        let mut pending = 1.;
        let limit = (1. / c).ceil() + 1.;
        for n in 1..=65536 {
            if n as f64 > limit || pending <= 0. {
                return Ok(if expected > 0. { 1. / expected } else { 1. });
            }
            let chance = (n as f64 * c).min(1.);
            expected += n as f64 * pending * chance;
            pending *= 1. - chance;
        }
        Err("Covenant probability exceeds PRD convergence bound".into())
    };
    let (mut lo, mut hi) = (0., p);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.;
        if rate(mid)? < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok((lo + hi) / 2.)
}
pub(super) fn damage(source: Id, target: Id, amount: f64, key: &str) -> hit::Request {
    let mut q = hit::Request::new(
        health::Hit {
            source: Some(source),
            credit: Some(source),
            target,
            damage_type: DamageType::True,
            no_sp: false,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount,
            mul: 1.,
            penetration: Default::default(),
            resolved: false,
        },
    );
    q.tags.push(key.into());
    q.dodge_mode = DodgeMode::Ignore;
    q
}
impl Host {
    pub(in crate::battle::runtime) fn bond_siracusa_end(&mut self, id: Id, time: f64) {
        self.bonds.siracusa.ended.insert(id, time);
    }
    pub(super) fn bonds_deploy(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if w.unit(id)?.deployment.kind != Kind::Operator {
            return Ok(());
        }
        if let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "siracusaShip")
            .cloned()
            .filter(|p| self.equipment_member(id, &p.id))
        {
            let l = self.bond_layers(&p.id);
            let duration = p.n("base_duration") + p.n("duration_per_stack") * l;
            if duration > 0. {
                let mut buff = Buff {
                    key: Some("bond:siracusa".into()),
                    mods: [(
                        "aspd".into(),
                        p.n("base_attack_speed") + p.n("attack_speed_per_stack") * l,
                    )]
                    .into(),
                    ..Default::default()
                };
                buff.runtime.duration = duration;
                r.add(
                    w,
                    self,
                    id,
                    buffs::Spec {
                        buff,
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
                if p.tier >= 2 {
                    self.bonds.siracusa.ended.remove(&id);
                    let mut buff = Buff {
                        key: Some("bond:siracusa:stealth".into()),
                        flags: ["stealth".into()].into(),
                        ..Default::default()
                    };
                    buff.runtime.duration = duration;
                    buff.runtime.status = Some("stealth".into());
                    buff.runtime.on_expire = Some(SIRACUSA_END);
                    buff.runtime.on_remove = Some(SIRACUSA_END);
                    r.add(
                        w,
                        self,
                        id,
                        buffs::Spec {
                            buff,
                            flags_present: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        if self.bonds.plans.iter().any(|p| p.id == "kazimierzShip") {
            self.bonds.kazimierz.deploys = self
                .bonds
                .kazimierz
                .deploys
                .checked_add(1)
                .ok_or("Covenant deployment counter exhausted")?;
            self.bond_kazimierz_refresh(w, r)?;
        }
        Ok(())
    }
    pub(super) fn bond_kazimierz_refresh(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "kazimierzShip")
            .cloned()
        else {
            return Ok(());
        };
        let cap = p.n("base_max_atk_when_born")
            + p.n("max_atk_when_born_per_stack") * self.bond_layers(&p.id);
        let bonus = cap
            .min(self.bonds.kazimierz.deploys as f64 * p.n("atk_when_born"))
            .max(0.);
        if bonus <= 0. {
            return Ok(());
        }
        for id in self.bond_ops(w) {
            if self.equipment_member(id, &p.id) {
                self.equipment_stat(
                    w,
                    r,
                    id,
                    "bond:kazimierz".into(),
                    [("atkPct".into(), bonus)].into(),
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn bond_kazimierz_pulse(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "kazimierzShip")
            .cloned()
        else {
            return Ok(());
        };
        self.bonds.kazimierz.next = Some(w.time + p.n("damage_interval"));
        for id in self.bond_ops(w) {
            if !Self::tac_up(w, id)
                || !self.equipment_member(id, &p.id)
                || w.unit(id)?.blocking().is_empty()
            {
                continue;
            }
            let targets = self.tac_radius(w, w.unit(id)?.position, p.n("range_radius"))?;
            let amount = p.n("damage_atk_scale") * Self::tac_stat(w, id, "atk")?;
            for target in targets {
                if !w.unit(target)?.stats.alive {
                    continue;
                }
                let mut q = damage(id, target, amount, "bond:kazimierz");
                q.is_splash = true;
                self.enemy_damage(w, r, &mut q)?;
                if w.unit(target)?.stats.alive && p.n("stun") > 0. {
                    self.apply_status(
                        w,
                        r,
                        target,
                        "stun",
                        crate::battle::status::Options {
                            duration: p.n("stun"),
                            source: Some(id),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn bonds_core_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
    ) -> Result<(), String> {
        let Some(source) = q.hit.source else {
            return Ok(());
        };
        let target = q.hit.target;
        if w.unit(target)?.side != health::Side::Enemy || !w.unit(target)?.stats.alive {
            return Ok(());
        }
        if let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "siracusaShip" && p.tier >= 2)
            .cloned()
            .filter(|p| self.equipment_member(source, &p.id))
        {
            let ordinary = !q.is_splash
                && !q.hit.hp_loss
                && q.hit.damage_type != DamageType::Elemental
                && !q.tags.iter().any(|s| {
                    matches!(
                        s.as_str(),
                        "dot" | "periodic" | "addition" | "item" | "hpLoss"
                    ) || s.starts_with("bond:")
                });
            let hidden = w
                .unit_mut(source)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .flags
                .contains("stealth")
                || w.time
                    <= self
                        .bonds
                        .siracusa
                        .ended
                        .get(&source)
                        .copied()
                        .unwrap_or(f64::NEG_INFINITY)
                        + p.n("end_duration")
                        + 1e-9;
            if ordinary && hidden && self.bonds.siracusa.constant > 0. {
                let state = &mut self.bonds.siracusa;
                state.attempts = state.attempts.saturating_add(1);
                if self.rng.next_f64() < (state.constant * state.attempts as f64).min(1.) {
                    state.attempts = 0;
                    let amount =
                        p.n("base_damage") + p.n("damage_per_stack") * self.bond_layers(&p.id);
                    self.enemy_damage(w, r, &mut damage(source, target, amount, "bond:siracusa"))?;
                    if w.unit(target)?.stats.alive && p.n("fear") > 0. {
                        self.apply_status(
                            w,
                            r,
                            target,
                            "fear",
                            crate::battle::status::Options {
                                duration: p.n("fear"),
                                source: Some(source),
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
        }
        if !w.unit(target)?.stats.alive {
            return Ok(());
        }
        if let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "kazimierzShip" && p.tier >= 2)
            .cloned()
            .filter(|p| self.equipment_member(source, &p.id))
        {
            if !q.hit.hp_loss
                && q.attack_type == hit::AttackType::Normal
                && q.hit.damage_type != DamageType::Elemental
                && !q.tags.iter().any(|s| s == "bond:kazimierz")
                && w.unit(source)?.blocking().is_empty()
                && p.n("pure_atk_scale") > 0.
            {
                let mut extra = damage(
                    source,
                    target,
                    p.n("pure_atk_scale") * Self::tac_stat(w, source, "atk")?,
                    "bond:kazimierz",
                );
                extra.hit.source = None;
                extra.calculation = hit::Calculation::Precomputed;
                extra.attack_type = hit::AttackType::Addition;
                self.enemy_damage(w, r, &mut extra)?;
            }
        }
        Ok(())
    }
}
