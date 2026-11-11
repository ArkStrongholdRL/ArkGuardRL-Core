// SPDX-License-Identifier: GPL-3.0-or-later
use super::{add, Error, Hook, Kind, Piece, Player, Preparation};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(super) enum GainSource {
    Grant,
    Displayed,
    FixedGift,
}
#[derive(Clone)]
pub(super) enum ItemSource {
    Fixed(String),
    Pool {
        id: String,
        golden: bool,
        fallback: bool,
    },
}
impl Preparation {
    pub(super) fn merge_items(
        &mut self,
        key: &str,
        incoming: Option<Piece>,
        player: &mut Player,
    ) -> Result<u64, Error> {
        let card = &self.rules.items[key];
        let need = card.merge - usize::from(incoming.is_some());
        let upgrade = card.upgrade.clone().unwrap();
        let consumed: Vec<_> = self.item_locations(key).into_iter().take(need).collect();
        for loc in consumed {
            self.take_item(loc);
        }
        let elite = self.new_piece(Kind::Item, &upgrade, 0)?;
        let uid = elite.uid;
        self.stow(elite)?;
        self.counts.item_merges = add(self.counts.item_merges, 1).map_err(|_| Error::Capacity)?;
        self.dispatch_meta(Hook::Merge, None, player)?;
        Ok(uid)
    }
    pub(super) fn check_item_merges(&mut self, player: &mut Player) -> Result<(), Error> {
        // Original scan order chooses the first mergeable key in hand, then temp.
        // Consumption still takes temp copies before hand copies.
        let limit =
            self.hand.len() + self.temp.len() + self.pieces().map(|p| p.items.len()).sum::<usize>();
        for _ in 0..limit {
            let mut counts = BTreeMap::new();
            let mut order = Vec::new();
            for p in self
                .hand
                .iter()
                .chain(&self.temp)
                .flatten()
                .chain(
                    self.chess_locations()
                        .iter()
                        .flat_map(|loc| self.piece(*loc).items.iter()),
                )
                .filter(|p| p.kind == Kind::Item && self.rules.items[&p.key].merge > 1)
            {
                let count = counts.entry(p.key.clone()).or_insert(0);
                if *count == 0 {
                    order.push(p.key.clone());
                }
                *count += 1;
            }
            let Some(key) = order
                .into_iter()
                .find(|key| counts[key] >= self.rules.items[key].merge)
            else {
                break;
            };
            self.merge_items(&key, None, player)?;
        }
        Ok(())
    }
    pub(super) fn grant_items(
        &mut self,
        source: &ItemSource,
        count: f64,
        player: &mut Player,
    ) -> Result<(), Error> {
        for _ in 0..count.clamp(0., 10.).ceil() as usize {
            let (key, kind) = match source {
                ItemSource::Fixed(id) => (
                    self.rules.items.contains_key(id).then(|| id.clone()),
                    GainSource::FixedGift,
                ),
                ItemSource::Pool {
                    id,
                    golden,
                    fallback,
                } => {
                    let valid =
                        self.rules.data.table("choices").unwrap()["pools"][id]["kind"] == "equip";
                    let mut item = if valid {
                        self.roll_meta_item(Some(id), self.level, *golden)?
                    } else {
                        None
                    };
                    if item.is_none() && *fallback {
                        item = self.roll_meta_item(None, self.level, false)?;
                    }
                    (item, GainSource::Grant)
                }
            };
            if let Some(key) = key {
                self.gain(Kind::Item, &key, kind, player)?;
            }
        }
        Ok(())
    }
    pub(super) fn roll_meta_item(
        &mut self,
        pool_id: Option<&str>,
        max_tier: usize,
        golden: bool,
    ) -> Result<Option<String>, Error> {
        let pool = &self.rules.data.table("choices").unwrap()["pools"][pool_id.unwrap_or("")];
        for key in ["weighted", "items", "also"] {
            if pool[key].as_array().is_some_and(|a| a.len() > 4096) {
                return Err(Error::Capacity);
            }
        }
        if pool["tiers"].as_array().is_some_and(|a| a.len() > 64) {
            return Err(Error::Capacity);
        }
        let equip = pool["kind"] == "equip";
        if golden && equip {
            if let (Some(pairs), Some(weights)) = (
                pool["weighted"].as_array(),
                pool["goldenWeights"].as_array(),
            ) {
                if pairs.len() == weights.len() {
                    // Elite pool weights intentionally roll before the stock refusal.
                    let pairs: Vec<_> = pairs
                        .iter()
                        .zip(weights)
                        .filter_map(|(p, w)| {
                            let id = p[0].as_str()?;
                            self.rules
                                .items
                                .contains_key(id)
                                .then(|| (id.to_owned(), super::effects::n(w, 0.).max(0.)))
                        })
                        .filter(|pair| pair.1 > 0.)
                        .collect();
                    let weights: Vec<_> = pairs.iter().map(|p| p.1).collect();
                    if let Some(i) = self.meta.as_mut().unwrap().rng.weighted(&weights) {
                        return Ok(Some(pairs[i].0.clone()));
                    }
                }
            }
        }
        let mut pairs = Vec::<(String, f64)>::new();
        if let Some(weighted) = pool["weighted"]
            .as_array()
            .filter(|a| equip && !a.is_empty())
        {
            for p in weighted {
                if let Some(id) = p[0]
                    .as_str()
                    .filter(|id| self.rules.items.contains_key(*id) && self.item_available(id))
                {
                    let weight = super::effects::n(&p[1], 0.).max(0.);
                    if let Some(pair) = pairs.iter_mut().find(|p| p.0 == id) {
                        pair.1 = weight;
                    } else {
                        pairs.push((id.into(), weight));
                    }
                }
            }
        } else if let Some(items) = pool["items"].as_array().filter(|a| equip && !a.is_empty()) {
            for id in items
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| self.rules.items.contains_key(*id))
            {
                pairs.push((id.into(), 1.));
            }
        } else {
            let tiers = pool["tiers"].as_array().filter(|a| equip && !a.is_empty());
            let high = if equip && pool["maxTier"] != "shopLevel" {
                6
            } else {
                max_tier.clamp(1, 6)
            };
            let tiers: Vec<_> = tiers
                .map(|a| a.iter().filter_map(Value::as_u64).collect())
                .unwrap_or_else(|| (1..=high as u64).collect());
            for tier in tiers {
                pairs.extend(
                    self.rules
                        .items
                        .values()
                        .filter(|c| c.shop && c.tier as u64 == tier)
                        .map(|c| (c.key.clone(), 1.)),
                );
            }
        }
        if equip {
            for id in pool["also"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| self.rules.items.contains_key(*id))
            {
                if !pairs.iter().any(|p| p.0 == id) {
                    pairs.push((id.into(), 1.));
                }
            }
        }
        let mut seen = BTreeSet::new();
        pairs.retain(|p| seen.insert(p.0.clone()) && self.item_available(&p.0));
        let weights: Vec<_> = pairs.iter().map(|p| p.1).collect();
        Ok(self
            .meta
            .as_mut()
            .unwrap()
            .rng
            .weighted(&weights)
            .map(|i| pairs[i].0.clone()))
    }
}
