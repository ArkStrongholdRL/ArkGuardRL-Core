// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Kind, Preparation};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub struct EffectsConfig {
    pub disabled_bonds: Vec<String>,
    pub layers: BTreeMap<String, f64>,
    pub count_bonus: BTreeMap<String, i32>,
}
pub(super) struct Bond {
    pub id: String,
    pub core: bool,
    pub off: bool,
    pub mode: Counting,
    pub thresholds: Vec<f64>,
    pub downward: Option<f64>,
    pub raw: Value,
}
#[derive(Clone, Copy)]
pub(super) enum Counting {
    Board,
    HandAndBoard,
    Elites,
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct State {
    pub count: f64,
    pub tier: usize,
    pub layers: f64,
    pub harmony: u8,
}
impl State {
    pub fn active(&self) -> bool {
        self.tier > 0
    }
}
impl Bond {
    pub fn load(data: &Value, mode: &Value, disabled: &[String]) -> Result<Vec<Self>, String> {
        let table = data.as_object().ok_or("Missing bond definitions")?;
        if table.len() > 256 || disabled.len() > 256 {
            return Err("Bond definition capacity exceeded".into());
        }
        if disabled.iter().any(|id| !table.contains_key(id)) {
            return Err("Unknown disabled bond".into());
        }
        let mut ids: Vec<_> = table.keys().cloned().collect();
        ids.sort_by(|a, b| {
            table[a]["identifier"]
                .as_f64()
                .unwrap_or(99.)
                .total_cmp(&table[b]["identifier"].as_f64().unwrap_or(99.))
                .then(a.cmp(b))
        });
        ids.into_iter()
            .map(|id| {
                let raw = &table[&id];
                let mut thresholds = raw["thresholds"]
                    .as_array()
                    .filter(|a| !a.is_empty())
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_f64)
                            .filter(|v| v.is_finite())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| {
                        vec![raw["activeCount"]
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .unwrap_or(2.)]
                    });
                if thresholds.len() > 64 {
                    return Err("Bond tier capacity exceeded".into());
                }
                thresholds.sort_by(f64::total_cmp);
                let off = disabled.contains(&id)
                    || mode["inactiveBondIds"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|v| v == &id));
                let downward =
                    (raw["thresholdTemplate"] == "count_threshold_downward").then(|| {
                        raw["maxCount"]
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .unwrap_or_else(|| thresholds.first().copied().unwrap_or(f64::NAN))
                    });
                let mode = if raw["countMode"] == "BOARD_ALL_CHESS"
                    || raw["thresholdTemplate"] == "count_threshold_upward_golden"
                {
                    Counting::Elites
                } else if raw["countMode"] == "BOARD_AND_DECK" {
                    Counting::HandAndBoard
                } else {
                    Counting::Board
                };
                Ok(Self {
                    id,
                    core: raw["isCore"] == true,
                    off,
                    mode,
                    thresholds,
                    downward,
                    raw: raw.clone(),
                })
            })
            .collect()
    }
    fn tier(&self, count: f64) -> usize {
        if let Some(max) = self.downward {
            usize::from(
                count >= self.thresholds.first().copied().unwrap_or(f64::NAN) && count <= max,
            )
        } else {
            self.thresholds.iter().filter(|n| count >= **n).count()
        }
    }
    pub fn buff(&self, key: &str) -> Value {
        let Some(buff) = self.raw["buffs"]
            .as_array()
            .and_then(|a| a.iter().find(|b| b["key"] == key))
        else {
            return json!({});
        };
        let mut out = buff["bb"].as_object().cloned().unwrap_or_default();
        if let Some(strings) = buff["bbStr"].as_object() {
            out.extend(strings.clone());
        }
        Value::Object(out)
    }
}
impl Preparation {
    pub(super) fn own_bonds(&self, key: &str) -> Vec<String> {
        self.rules.data.table("chess").unwrap()[key]["bonds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }
    pub(super) fn piece_bonds(&self, piece: &super::Piece) -> Vec<String> {
        let Some(loc) = self.find(piece.uid) else {
            return self.own_bonds(&piece.key);
        };
        let live = self.piece(loc);
        let items: Vec<_> = live.items.iter().map(|p| p.key.clone()).collect();
        crate::battle::equipment::bonds(&self.rules.data, &live.key, &items)
    }
    pub(super) fn bond_states(&self) -> BTreeMap<String, State> {
        let Some(meta) = &self.meta else {
            return BTreeMap::new();
        };
        let mut board = BTreeMap::<String, BTreeSet<String>>::new();
        let mut variants = BTreeMap::<String, BTreeSet<(String, bool)>>::new();
        let mut hand = BTreeMap::<String, BTreeSet<String>>::new();
        let mut elites = 0;
        for piece in self.board.values() {
            if piece.kind != Kind::Chess {
                continue;
            }
            let base = &self.rules.chess[&piece.key].base;
            let golden = self.rules.data.table("chess").unwrap()[&piece.key]["isGolden"] == true;
            elites += usize::from(golden);
            for bond in self.piece_bonds(piece) {
                board.entry(bond.clone()).or_default().insert(base.clone());
                variants
                    .entry(bond)
                    .or_default()
                    .insert((base.clone(), golden));
            }
        }
        for piece in self.hand.iter().flatten().filter(|p| p.kind == Kind::Chess) {
            for bond in self.piece_bonds(piece) {
                hand.entry(bond)
                    .or_default()
                    .insert(self.rules.chess[&piece.key].base.clone());
            }
        }
        let mut raw = BTreeMap::new();
        for bond in &meta.rules.bonds {
            let count = match bond.mode {
                Counting::Elites => elites,
                Counting::HandAndBoard => board
                    .get(&bond.id)
                    .into_iter()
                    .flatten()
                    .chain(hand.get(&bond.id).into_iter().flatten())
                    .collect::<BTreeSet<_>>()
                    .len(),
                Counting::Board => board.get(&bond.id).map_or(0, BTreeSet::len),
            };
            raw.insert(
                bond.id.clone(),
                (count as f64 + f64::from(*meta.bonus.get(&bond.id).unwrap_or(&0))).max(0.),
            );
        }
        let harmony = meta
            .rules
            .bonds
            .iter()
            .find(|b| b.id == "maniShip" && !b.off)
            .is_some_and(|b| b.tier(raw[&b.id]) > 0);
        let mut out = BTreeMap::new();
        for bond in &meta.rules.bonds {
            if bond.off {
                continue;
            }
            let h = u8::from(
                harmony && bond.core && board.get(&bond.id).is_some_and(|s| !s.is_empty()),
            );
            let count = raw[&bond.id] + f64::from(h);
            let mut tier = bond.tier(count);
            if bond.id == "deputShip" && tier > 0 {
                let count = variants.get(&bond.id).map_or(0, BTreeSet::len) as f64
                    + f64::from(*meta.bonus.get(&bond.id).unwrap_or(&0));
                tier = 1;
                for (i, n) in bond.thresholds.iter().enumerate().skip(1) {
                    if count >= *n {
                        tier = i + 1;
                    }
                }
            }
            out.insert(
                bond.id.clone(),
                State {
                    count,
                    tier,
                    layers: *meta.layers.get(&bond.id).unwrap_or(&0.),
                    harmony: h,
                },
            );
        }
        out
    }
}
