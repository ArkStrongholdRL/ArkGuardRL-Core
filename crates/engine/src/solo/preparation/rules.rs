// SPDX-License-Identifier: GPL-3.0-or-later
use super::Kind;
use crate::data::Catalog;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Card {
    pub key: String,
    pub base: String,
    pub tier: usize,
    pub upgrade: Option<String>,
    pub merge: usize,
    pub copies: u64,
    pub price: f64,
    pub sell: u64,
    pub cap: Option<u64>,
    pub shop: bool,
}
pub(super) struct Rules {
    pub mode_id: String,
    pub data: Arc<Catalog>,
    pub chess: BTreeMap<String, Card>,
    pub items: BTreeMap<String, Card>,
    pub fixed_gifts: BTreeSet<String>,
    pub equip_cap: usize,
    pub bench_equip: bool,
    pub slots: Vec<[usize; 2]>,
    pub upgrades: Vec<f64>,
    pub refresh: f64,
    pub reward_count: usize,
    pub reward_offset: i64,
    pub reward_max_tier: usize,
    pub reward_price: f64,
}
fn number(v: &Value, fallback: f64) -> f64 {
    v.as_f64().filter(|v| v.is_finite()).unwrap_or(fallback)
}
fn positive(v: &Value, fallback: usize) -> usize {
    v.as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(fallback)
}
fn merge(to: &mut Value, from: &Value) {
    if let (Some(to), Some(from)) = (to.as_object_mut(), from.as_object()) {
        for (key, value) in from {
            merge(to.entry(key.clone()).or_insert(Value::Null), value);
        }
    } else {
        *to = from.clone();
    }
}
impl Rules {
    pub fn load(data: Arc<Catalog>, mode_id: &str) -> Result<Self, String> {
        let cfg = data.table("config").unwrap();
        let mode = &cfg["modes"][mode_id];
        if !mode.is_object() || !(mode["type"] == "SINGLE" || mode_id.starts_with("mode_single_")) {
            return Err("Preparation requires a known single-player mode".into());
        }
        let mut economy = cfg["economy"].clone();
        if mode["economy"].is_object() {
            merge(&mut economy, &mode["economy"]);
        }
        let levels = positive(&mode["maxShopLevel"], 6);
        if levels > 32 {
            return Err("Shop level capacity exceeded".into());
        }
        let defaults = [[3, 1], [4, 1], [4, 1], [5, 1], [5, 1], [5, 1]];
        let slots = (1..=levels)
            .map(|level| {
                let d = defaults[(level - 1).min(5)];
                let s = &mode["shopSlots"][level.to_string()];
                [
                    s["chess"]
                        .as_u64()
                        .map(|v| v.min(8) as usize)
                        .unwrap_or(d[0]),
                    s["item"]
                        .as_u64()
                        .map(|v| v.min(4) as usize)
                        .unwrap_or(d[1]),
                ]
            })
            .collect();
        let upgrade_defaults = [5., 8., 11., 12., 13.];
        let upgrades = (0..levels)
            .map(|i| {
                if i + 1 == levels {
                    0.
                } else {
                    if let Some(prices) = mode["upgradePrices"].as_array() {
                        number(prices.get(i).unwrap_or(&Value::Null), 99.).max(0.)
                    } else {
                        upgrade_defaults.get(i).copied().unwrap_or(99.)
                    }
                }
            })
            .collect();
        let mut chess = BTreeMap::new();
        let mut items = BTreeMap::new();
        let golden_copies = positive(&economy["goldenCopies"], 3) as u64;
        for (kind, table, out) in [
            (Kind::Chess, "chess", &mut chess),
            (Kind::Item, "items", &mut items),
        ] {
            let raw = data.table(table).unwrap().as_object().unwrap();
            if raw.len() > 4096 {
                return Err("Preparation definition capacity exceeded".into());
            }
            for (key, rec) in raw {
                let golden = rec["isGolden"] == true;
                let tier = rec["tier"]
                    .as_u64()
                    .filter(|v| *v > 0 && *v <= 64)
                    .unwrap_or(1) as usize;
                let base = if kind == Kind::Chess {
                    rec["baseId"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if golden {
                                key.strip_suffix("_b")
                                    .map(|s| format!("{s}_a"))
                                    .unwrap_or(key.clone())
                            } else {
                                key.clone()
                            }
                        })
                } else {
                    key.strip_suffix("_b")
                        .map(|s| format!("{s}_a"))
                        .unwrap_or(key.clone())
                };
                let upgrade = if kind == Kind::Chess {
                    rec["goldenId"]
                        .as_str()
                        .filter(|id| raw.contains_key(*id))
                        .map(str::to_owned)
                        .or_else(|| {
                            let alt = key
                                .strip_suffix("_a")
                                .map(|s| format!("{s}_b"))
                                .unwrap_or(key.clone());
                            raw.contains_key(&alt).then_some(alt)
                        })
                } else {
                    rec["upgradeChessId"]
                        .as_str()
                        .or(rec["goldenId"].as_str())
                        .filter(|id| raw.contains_key(*id))
                        .map(str::to_owned)
                };
                let recipe = if kind == Kind::Chess {
                    rec["upgradeNum"]
                        .as_u64()
                        .filter(|v| *v > 0)
                        .map(|v| v as usize)
                        .unwrap_or_else(|| {
                            economy["mergeCountOverrides"][key]
                                .as_i64()
                                .map(|v| v.max(0) as usize)
                                .unwrap_or(positive(&economy["mergeCount"], 3))
                        })
                } else {
                    rec["upgradeNum"]
                        .as_u64()
                        .map(|v| v as usize)
                        .unwrap_or(positive(&economy["itemMergeCount"], 2))
                };
                let merge = if golden
                    || upgrade.is_none()
                    || kind == Kind::Item
                        && (rec["itemType"] != "EQUIP" || rec["mergeable"] != true)
                {
                    0
                } else {
                    recipe
                };
                let form = if golden { "golden" } else { "normal" };
                let price = if kind == Kind::Chess {
                    let fallback = [0., 2., 3., 3., 3., 4., 4.]
                        .get(tier)
                        .copied()
                        .unwrap_or(3.);
                    let row = &economy["chessPrice"][tier.to_string()];
                    rec["price"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.)
                        .unwrap_or_else(|| number(&row[form], fallback))
                } else {
                    rec["price"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.)
                        .unwrap_or(2.)
                };
                let override_price = mode["economy"]["chessPrice"][tier.to_string()][form]
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.);
                let price = if kind == Kind::Chess {
                    override_price.unwrap_or(price)
                } else {
                    price
                };
                let sell = number(
                    &mode["economy"]["chessSell"][tier.to_string()][form],
                    f64::NAN,
                );
                let sell = if sell >= 0. {
                    sell
                } else {
                    rec["sellPrice"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.)
                        .unwrap_or_else(|| {
                            number(&economy["chessSell"][tier.to_string()][form], 1.)
                        })
                };
                let shop = if kind == Kind::Chess {
                    (rec["visible"] == true && rec["isDiy"] != true || rec["diyFor"].is_string())
                        && !golden
                        && rec["isHidden"] != true
                        && rec["tier"].as_i64().is_some()
                } else {
                    !golden
                        && rec["hideInShop"] != true
                        && rec["shopExcluded"] != true
                        && rec["itemType"] == "EQUIP"
                        && rec["tier"].as_i64().is_some()
                };
                let cap = if kind == Kind::Chess {
                    Some(
                        economy["poolCopiesOverrides"][&base]
                            .as_u64()
                            .or(economy["poolCopies"][tier.to_string()].as_u64())
                            .unwrap_or([0, 12, 14, 18, 16, 8, 5].get(tier).copied().unwrap_or(10)),
                    )
                } else {
                    let normal = raw.get(&base).unwrap_or(&Value::Null);
                    let shop_base = normal["isGolden"] != true
                        && normal["hideInShop"] != true
                        && normal["shopExcluded"] != true
                        && normal["itemType"] == "EQUIP"
                        && normal["tier"].as_i64().is_some();
                    if shop_base {
                        economy["itemPoolCopiesOverrides"][&base]
                            .as_u64()
                            .or(economy["itemPoolCopies"][normal["tier"].to_string()].as_u64())
                    } else {
                        None
                    }
                };
                if !price.is_finite()
                    || price.abs() > super::super::MAX_FUNDS as f64
                    || sell.abs() > super::super::MAX_FUNDS as f64
                    || cap.is_some_and(|v| v > super::super::MAX_FUNDS)
                    || recipe > 4096
                    || golden_copies > 4096
                {
                    return Err("Preparation numeric capacity exceeded".into());
                }
                out.insert(
                    key.clone(),
                    Card {
                        key: key.clone(),
                        base,
                        tier,
                        upgrade,
                        merge,
                        copies: if kind == Kind::Chess {
                            if golden {
                                golden_copies
                            } else {
                                1
                            }
                        } else if golden {
                            2
                        } else {
                            1
                        },
                        price,
                        sell: sell.trunc().max(0.) as u64,
                        cap,
                        shop,
                    },
                );
            }
        }
        let reward = &economy["rewardOffer"];
        let fixed_gifts = data
            .table("garrisons")
            .unwrap()
            .as_object()
            .unwrap()
            .values()
            .filter(|g| g["effectKey"] == "SERVER_GAIN_EQUIP")
            .filter_map(|g| g["bbStr"]["chess"].as_str())
            .map(str::to_owned)
            .collect();
        let equip_cap = positive(&economy["equipPerChess"], 2);
        if equip_cap > 16 {
            return Err("Equipment slot capacity exceeded".into());
        }
        let bench_equip = cfg["version"]["rules"]["benchEquip"] != false;
        Ok(Self {
            mode_id: mode_id.into(),
            equip_cap,
            bench_equip,
            fixed_gifts,
            data,
            chess,
            items,
            slots,
            upgrades,
            refresh: number(&economy["refreshPrice"], 1.).max(0.),
            reward_count: positive(&reward["count"], 3).min(6),
            reward_offset: reward["tierOffset"].as_i64().unwrap_or(1),
            reward_max_tier: positive(&reward["maxTier"], 6).min(64),
            reward_price: number(&reward["price"], 0.).max(0.),
        })
    }
    pub fn card(&self, kind: Kind, key: &str) -> Option<&Card> {
        match kind {
            Kind::Chess => self.chess.get(key),
            Kind::Item => self.items.get(key),
            Kind::Token => None,
        }
    }
}
