// SPDX-License-Identifier: GPL-3.0-or-later
use super::super::{Offer, Slot};
use super::*;

#[derive(Clone)]
pub(super) enum Recruit {
    Fixed {
        key: String,
        count: f64,
    },
    Pool {
        key: String,
        count: f64,
        row_min: Option<f64>,
    },
    Most {
        row_min: Option<f64>,
    },
    Offer {
        pools: Vec<String>,
        fallback: String,
    },
}
impl Preparation {
    fn recruit_available(&self, key: &str) -> bool {
        self.rules
            .chess
            .get(key)
            .is_some_and(|c| self.pool.get(&c.base).is_none_or(|e| e.left > 0))
    }
    pub(in crate::solo::preparation) fn grant_recruit(
        &mut self,
        key: &str,
        player: &mut Player,
    ) -> Result<bool, Error> {
        if !self.recruit_available(key) {
            return Ok(false);
        }
        Ok(self
            .gain(Kind::Chess, key, GainSource::Grant, player)?
            .is_some())
    }
    pub(in crate::solo::preparation) fn roll_meta_chess(
        &mut self,
        max_tier: usize,
        min_tier: usize,
        tier: Option<usize>,
        bonds: Option<&[String]>,
        exclude: &[String],
    ) -> Option<String> {
        let eligible: Vec<_> = self
            .stock_entries()
            .filter(|(key, e)| {
                e.left > 0
                    && tier.map_or(e.tier <= max_tier, |tier| e.tier == tier)
                    && e.tier >= min_tier
                    && !exclude.contains(key)
                    && bonds
                        .is_none_or(|bonds| self.own_bonds(key).iter().any(|b| bonds.contains(b)))
            })
            .map(|(key, e)| (key.clone(), e.left as f64))
            .collect();
        let weights: Vec<_> = eligible.iter().map(|p| p.1).collect();
        self.meta
            .as_mut()
            .unwrap()
            .rng
            .weighted(&weights)
            .map(|i| eligible[i].0.clone())
    }
    fn roll_chess_pool(&mut self, pool_id: &str) -> Result<Option<String>, Error> {
        let pool = &self.rules.data.table("choices").unwrap()["pools"][pool_id];
        if pool["kind"] != "chess" {
            return Ok(None);
        }
        if ["items", "weighted"]
            .iter()
            .any(|key| pool[key].as_array().is_some_and(|a| a.len() > 4096))
        {
            return Err(Error::Capacity);
        }
        let golden = pool["golden"] == true;
        let picked = if let Some(weighted) = pool["weighted"].as_array().filter(|a| !a.is_empty()) {
            let pairs: Vec<_> = weighted
                .iter()
                .filter_map(|p| p.as_array())
                .filter_map(|p| {
                    let id = p.first()?.as_str()?;
                    self.recruit_available(id).then(|| {
                        (
                            id.to_owned(),
                            n(p.get(1).unwrap_or(&Value::Null), 0.).max(0.),
                        )
                    })
                })
                .collect();
            let weights: Vec<_> = pairs.iter().map(|p| p.1).collect();
            self.meta
                .as_mut()
                .unwrap()
                .rng
                .weighted(&weights)
                .or_else(|| (!pairs.is_empty()).then_some(0))
                .map(|i| pairs[i].0.clone())
        } else if let Some(items) = pool["items"].as_array().filter(|a| !a.is_empty()) {
            let candidates: Vec<_> = items
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| self.recruit_available(id))
                .map(str::to_owned)
                .collect();
            self.meta.as_mut().unwrap().rng.pick(&candidates).cloned()
        } else {
            let max = if pool["maxTier"] == "shopLevel" {
                self.level.clamp(1, 6)
            } else {
                pool["maxTier"].as_u64().map_or(6, |n| n as usize)
            };
            let min = pool["minTier"].as_u64().map_or(1, |n| n as usize);
            let tier = pool["tier"].as_u64().map(|n| n as usize);
            let bond = pool["bond"].as_str().map(str::to_owned);
            self.roll_meta_chess(max, min, tier, bond.as_ref().map(std::slice::from_ref), &[])
        };
        Ok(picked.map(|key| {
            if golden {
                self.rules.chess[&key].upgrade.clone().unwrap_or(key)
            } else {
                key
            }
        }))
    }
    pub(super) fn recruit(
        &mut self,
        recruit: &Recruit,
        call: &Call,
        hook: &Hook,
        player: &mut Player,
    ) -> Result<(), Error> {
        match recruit {
            Recruit::Fixed { key, count } => {
                for _ in 0..count.clamp(0., 10.).ceil() as usize {
                    self.grant_recruit(key, player)?;
                }
            }
            Recruit::Pool {
                key,
                count,
                row_min,
            } => {
                if self.row_condition(*row_min, call, hook) {
                    for _ in 0..count.clamp(0., 10.).ceil() as usize {
                        if let Some(key) = self.roll_chess_pool(key)? {
                            self.grant_recruit(&key, player)?;
                        }
                    }
                }
            }
            Recruit::Most { row_min } => {
                if !self.row_condition(*row_min, call, hook) {
                    return Ok(());
                }
                let meta = self.meta.as_ref().unwrap();
                let mut best = Vec::new();
                let mut count = 0.;
                for bond in &meta.rules.bonds {
                    let current = meta.bonds.get(&bond.id).map_or(0., |s| s.count);
                    if current > count {
                        count = current;
                        best.clear();
                        best.push(bond.id.clone());
                    } else if current == count && current > 0. {
                        best.push(bond.id.clone());
                    }
                }
                if best.len() > 1 {
                    self.meta.as_mut().unwrap().rng.shuffle(&mut best);
                }
                for bond in best {
                    if let Some(key) =
                        self.roll_meta_chess(6, 1, None, Some(std::slice::from_ref(&bond)), &[])
                    {
                        if self.grant_recruit(&key, player)? {
                            break;
                        }
                    }
                }
            }
            Recruit::Offer { pools, fallback } => {
                let pool = pools
                    .get(self.level - 1)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(fallback);
                let tier = self.rules.data.table("choices").unwrap()["pools"][pool]["tier"]
                    .as_u64()
                    .map(|n| n as usize);
                let mut keys = Vec::new();
                for _ in 0..self.rules.reward_count * 4 {
                    if keys.len() >= self.rules.reward_count {
                        break;
                    }
                    if let Some(key) = self.roll_chess_pool(pool)? {
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                }
                let offer_tier = tier.unwrap_or(
                    (self.level as i64 + self.rules.reward_offset)
                        .min(self.rules.reward_max_tier as i64)
                        .max(0) as usize,
                );
                if keys.is_empty() && tier.is_some_and(|t| t > 0) {
                    for _ in 0..self.rules.reward_count {
                        for t in (1..=offer_tier).rev() {
                            if let Some(key) = self.roll_chess(6, Some(t), &keys) {
                                keys.push(key);
                                break;
                            }
                        }
                    }
                }
                if !keys.is_empty() {
                    if self.offers.len() >= self.capacity {
                        return Err(Error::Capacity);
                    }
                    self.offers.push_back(Offer {
                        tier: offer_tier,
                        slots: keys
                            .into_iter()
                            .take(6)
                            .map(|key| Slot {
                                kind: Kind::Chess,
                                key,
                                price: self.rules.reward_price,
                                frozen: false,
                                sold: false,
                            })
                            .collect(),
                    });
                }
            }
        }
        Ok(())
    }
}
