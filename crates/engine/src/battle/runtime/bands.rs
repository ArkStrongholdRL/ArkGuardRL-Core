// SPDX-License-Identifier: GPL-3.0-or-later
//! Selected core effects. Unsupported cores fail before battle state is changed.
use super::*;
use crate::battle::{deploy::Kind, stats::Numbers};
#[derive(Default)]
pub(super) struct Bands {
    plan: Option<Plan>,
}
struct Plan {
    id: String,
    kind: Effect,
    bb: Numbers,
}
enum Effect {
    Bonds,
    Weakness,
    Life,
    Preparation,
}
impl Plan {
    fn compile(data: &Catalog, id: &str) -> Result<Self, String> {
        let record = data
            .table("bands")
            .and_then(|v| v.get(id))
            .ok_or("Unknown core")?;
        if !record["modeTypeList"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "SINGLE"))
        {
            return Err("Core is unavailable in Desperation".into());
        }
        if !matches!(
            id,
            "band_amiya" | "band_sarkazb" | "band_chen" | "band_sciurus"
        ) {
            return Err("Core is outside the supported scope".into());
        }
        if id == "band_sciurus" {
            if record["buffs"][0]["key"] != "first_buy_in_round_char_price_change" {
                return Err("Unsupported Sciurus effects".into());
            }
            return Ok(Self {
                id: id.into(),
                kind: Effect::Preparation,
                bb: Numbers::new(),
            });
        }
        if id == "band_sarkazb" {
            if record["buffs"].as_array().is_none_or(|a| !a.is_empty()) {
                return Err("Unsupported Goliath effects".into());
            }
            return Ok(Self {
                id: id.into(),
                kind: Effect::Life,
                bb: Numbers::new(),
            });
        }
        let buffs = record["buffs"]
            .as_array()
            .filter(|a| a.len() == 1)
            .ok_or("Unsupported core effects")?;
        let b = &buffs[0];
        let kind = match b["bbStr"]["key"].as_str() {
            Some("act1autochess_band2_buff") => Effect::Bonds,
            Some("act1autochess_band19_buff") => Effect::Weakness,
            _ => return Err("Unsupported core effects".into()),
        };
        let bb = b["bb"]
            .as_object()
            .ok_or("Missing core parameters")?
            .iter()
            .map(|(k, v)| {
                v.as_f64()
                    .filter(|n| n.is_finite())
                    .map(|n| (k.clone(), n))
                    .ok_or("Invalid core parameter".to_string())
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id: id.into(),
            kind,
            bb,
        })
    }
    fn n(&self, key: &str) -> f64 {
        self.bb.get(key).copied().unwrap_or(0.)
    }
}
impl Field {
    pub(crate) fn validate_core(data: &Catalog, id: &str) -> Result<(), String> {
        Plan::compile(data, id).map(|_| ())
    }
    pub fn configure_core(&mut self, id: &str) -> Result<(), String> {
        self.healthy()?;
        if self.clock.started() || self.ticks() != 0 || self.core.host.bands.plan.is_some() {
            return Err("Core requires an unstarted field and can be configured once".into());
        }
        let plan = Plan::compile(&self.core.host.data, id)?;
        self.core.host.bands.plan = Some(plan);
        Ok(())
    }
}
impl Host {
    pub(super) fn bands_install(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        start: bool,
    ) -> Result<(), String> {
        let Some(p) = &self.bands.plan else {
            return Ok(());
        };
        let ops: Vec<_> = self
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .filter(|&id| {
                w.unit(id)
                    .is_ok_and(|u| u.deployment.kind == Kind::Operator)
            })
            .collect();
        let (ids, atk, hp) = match p.kind {
            Effect::Bonds if !start => {
                let count = self.growth.live.values().filter(|b| b.tier > 0).count() as f64;
                let (mut atk, mut hp) = (0., 0.);
                for i in 1..=9 {
                    let Some(threshold) = p.bb.get(&format!("value_{i}")) else {
                        break;
                    };
                    if count >= *threshold {
                        atk = p.n(&format!("atk_{i}"));
                        hp = p.n(&format!("max_hp_{i}"));
                    }
                }
                (ops, atk, hp)
            }
            _ => return Ok(()),
        };
        if atk <= 0. && hp <= 0. {
            return Ok(());
        }
        let key = format!("band:{}", p.id);
        let mods: Numbers = [("atkPct".into(), atk), ("hpPct".into(), hp)].into();
        for id in ids {
            self.equipment_stat(w, r, id, key.clone(), mods.clone())?;
        }
        Ok(())
    }
    pub(super) fn core_weakness(&self, w: &World, source: Id, target: Id) -> bool {
        self.bands
            .plan
            .as_ref()
            .is_some_and(|p| matches!(p.kind, Effect::Weakness))
            && w.unit(source)
                .is_ok_and(|u| u.deployment.kind == Kind::Operator)
            && w.unit(target).is_ok_and(|u| u.side == health::Side::Enemy)
    }
}
