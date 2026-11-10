// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol attack/calculation.js. Pure arithmetic only: no injury hooks or HP mutation.
use super::stats::{Stat, Stats};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageType {
    Physical,
    Arts,
    True,
    Elemental,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Penetration {
    pub def_flat: f64,
    pub def_pct: f64,
    pub res_flat: f64,
    pub res_pct: f64,
}
fn clamp01(x: f64) -> f64 {
    if x < 0.0 {
        0.0
    } else if x > 1.0 {
        1.0
    } else {
        x
    }
}
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
fn positive(x: f64) -> f64 {
    if x > 0.0 && x.is_finite() {
        x
    } else {
        0.0
    }
}
pub fn mitigate(
    amount: f64,
    kind: DamageType,
    target: &Stats,
    ign: Penetration,
    elemental_res: f64,
) -> f64 {
    match kind {
        DamageType::Physical => {
            let eff = max(0.0, target.get(Stat::Def) - ign.def_flat) * (1.0 - clamp01(ign.def_pct));
            max(amount - eff, 0.05 * amount)
        }
        DamageType::Arts => {
            let eff = max(0.0, target.get(Stat::Res) - ign.res_flat) * (1.0 - clamp01(ign.res_pct));
            let eff = if eff.is_nan() { eff } else { eff.min(100.0) };
            max(amount * (1.0 - eff / 100.0), 0.05 * amount)
        }
        DamageType::Elemental => {
            let d = if elemental_res.is_finite() {
                elemental_res.clamp(0.0, 100.0)
            } else {
                0.0
            };
            max(amount * (1.0 - d / 100.0), 0.05 * amount)
        }
        DamageType::True => amount,
    }
}
pub fn attack_input(amount: f64, kind: DamageType, target: &Stats) -> f64 {
    amount
        + if matches!(kind, DamageType::Arts) && target.get(Stat::ArtsAttackAddition).is_finite() {
            target.get(Stat::ArtsAttackAddition)
        } else {
            0.0
        }
}
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub amount: f64,
    pub mul: f64,
    pub penetration: Penetration,
    pub resolved: bool,
}
pub fn expected(dmg: Damage, kind: DamageType, target: &Stats, elemental_res: f64) -> f64 {
    let base = mitigate(
        dmg.amount,
        kind,
        target,
        Penetration::default(),
        elemental_res,
    );
    positive(base * if dmg.mul.is_finite() { dmg.mul } else { 1.0 })
}
pub fn calculate(
    dmg: Damage,
    kind: DamageType,
    target: &Stats,
    source: Option<&Stats>,
    elemental_res: f64,
) -> f64 {
    let mut ign = dmg.penetration;
    if let Some(s) = source {
        ign.def_flat += s.get(Stat::DefIgnoreFlat);
        ign.def_pct += s.get(Stat::DefIgnorePct);
        ign.res_flat += s.get(Stat::ResIgnoreFlat);
        ign.res_pct += s.get(Stat::ResIgnorePct);
    }
    let mut final_ = if dmg.resolved {
        dmg.amount
    } else {
        mitigate(dmg.amount, kind, target, ign, elemental_res)
    };
    let mut mul = dmg.mul;
    if let Some(s) = source {
        if !dmg.resolved {
            mul *= s.get(Stat::DmgDealtMul)
                * match kind {
                    DamageType::Physical => s.get(Stat::PhysDealtMul),
                    DamageType::Arts => s.get(Stat::ArtsDealtMul),
                    _ => 1.0,
                };
        }
    }
    final_ *= mul;
    positive(final_)
}
pub fn estimate(base: f64, kind: DamageType, target: &Stats) -> f64 {
    let mul = (if matches!(kind, DamageType::Elemental) {
        1.0
    } else {
        target.get(Stat::DmgTakenMul)
    }) * target.get(match kind {
        DamageType::Physical => Stat::PhysTakenMul,
        DamageType::Arts => Stat::ArtsTakenMul,
        DamageType::Elemental => Stat::ElementalTakenMul,
        DamageType::True => Stat::TrueTakenMul,
    });
    positive(base * mul)
}
