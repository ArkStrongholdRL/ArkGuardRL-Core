// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    bonds::{Bond, EffectsConfig, State},
    items::{GainSource, ItemSource},
    Error, Kind, Location, Piece, Preparation,
};
use crate::{
    data::Catalog,
    deterministic::rng::{derive_seed, Rng},
    solo::{add, player::Player, Phase, MAX_FUNDS},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod dispatch;
mod recruits;
use dispatch::{Call, Origin, Refresh};
use recruits::Recruit;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Hook {
    Gain,
    Sold,
    PrepStart,
    PrepEnd,
    Refresh,
    Price,
    RoundStart,
    Buy,
    Merge,
    LevelUp,
    Active,
    Layers(String),
}
#[derive(Clone)]
enum Method {
    Shop,
    Acquired,
    Hand,
    Row,
    Tiers,
}
#[derive(Clone)]
enum Formula {
    Fixed(f64),
    Method(Method, f64),
    Spent { per: f64, layer: f64 },
    Refresh { multi: f64, max: f64 },
}
#[derive(Clone)]
enum Targets {
    Own,
    Named(Vec<String>),
    Multiple(Vec<(String, f64)>),
}
#[derive(Clone)]
enum Effect {
    Layers {
        targets: Targets,
        formula: Formula,
        active: bool,
        row_min: Option<f64>,
    },
    Most {
        count: f64,
        diversity: bool,
    },
    Hand(f64),
    Position {
        behind: bool,
        count: f64,
    },
    FirstRefresh {
        ids: Vec<String>,
        count: f64,
        at: f64,
        active: bool,
    },
    Pending {
        count: f64,
        condition: Vec<String>,
    },
    Free(f64),
    Discount(f64),
    Items {
        source: ItemSource,
        count: f64,
        rounds: Vec<f64>,
    },
    Recruit(Recruit),
    Trigger {
        hook: Option<Hook>,
        scope: TriggerScope,
    },
    Copy {
        hook: Hook,
        key: String,
    },
    RandomLayers {
        count: f64,
        repeats: f64,
    },
    ForwardLayers(f64),
    Unsupported(String),
}
#[derive(Clone)]
enum TriggerScope {
    Front(f64),
    Farthest,
}
#[derive(Clone)]
struct Rule {
    hook: Hook,
    board_only: bool,
    effect: Effect,
}
pub(super) struct Rules {
    pub bonds: Vec<Bond>,
    garrisons: BTreeMap<String, Rule>,
}
#[derive(Clone)]
pub(super) struct Meta {
    pub rules: Arc<Rules>,
    pub layers: BTreeMap<String, f64>,
    pub bonus: BTreeMap<String, i32>,
    pub bonds: BTreeMap<String, State>,
    pub rng: Rng,
    pub defer_items: usize,
    hammer_paid: f64,
    pub round: u32,
    phase: Phase,
    depth: usize,
    refresh: Option<Refresh>,
    ended_round: Option<u32>,
    visi_paid: f64,
    mira_paid: f64,
    discount: u8,
}
pub(super) fn n(v: &Value, fallback: f64) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .filter(|v| v.is_finite())
        .unwrap_or(fallback)
}
fn ids_of_bonds(v: &Value) -> Vec<String> {
    v.as_str()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
impl Hook {
    fn garrison(event: &str) -> Option<Self> {
        Some(match event {
            "SERVER_GAIN" => Self::Gain,
            "SERVER_CHESS_SOLD" => Self::Sold,
            "SERVER_PREP_START" => Self::PrepStart,
            "SERVER_PREP_FIN" => Self::PrepEnd,
            "SERVER_REFRESH_SHOP" => Self::Refresh,
            "SERVER_PRICE" => Self::Price,
            _ => return None,
        })
    }
}
impl Rule {
    fn load(id: &str, raw: &Value) -> Option<Self> {
        let hook = Hook::garrison(raw["eventType"].as_str()?)?;
        let bb = &raw["bb"];
        let s = &raw["bbStr"];
        let desc = raw["desc"].as_str().unwrap_or("");
        let active =
            !desc.contains("无需激活") && (desc.contains("已激活") || desc.contains("激活且"));
        let count = n(&bb["count"], 0.);
        let named = Targets::Named(ids_of_bonds(&s["bond"]));
        let effect = match raw["effectKey"].as_str().unwrap_or("") {
            "SERVER_ADD_BOND_CHESS_ALL" => Effect::Layers {
                targets: Targets::Own,
                formula: Formula::Fixed(count),
                active,
                row_min: None,
            },
            "SERVER_ADD_BOND" => Effect::Layers {
                targets: named,
                formula: Formula::Fixed(count),
                active,
                row_min: (s["conditionkey"] == "character_same_row")
                    .then(|| n(&bb["check_count"], 0.)),
            },
            "SERVER_ADD_MULTIPLE_BOND" => {
                let counts = ids_of_bonds(&s["count"])
                    .iter()
                    .map(|s| s.parse::<f64>().unwrap_or(0.))
                    .collect::<Vec<_>>();
                Effect::Layers {
                    targets: Targets::Multiple(
                        ids_of_bonds(&s["bond"])
                            .into_iter()
                            .enumerate()
                            .map(|(i, b)| {
                                (
                                    b,
                                    counts
                                        .get(i)
                                        .or_else(|| counts.last())
                                        .copied()
                                        .unwrap_or(0.),
                                )
                            })
                            .collect(),
                    ),
                    formula: Formula::Fixed(1.),
                    active,
                    row_min: None,
                }
            }
            "SERVER_ADD_BOND_METHOD" => {
                let method = match s["add_method"].as_str().unwrap_or("") {
                    "shoplv" => Some(Method::Shop),
                    "round_gain_char" => Some(Method::Acquired),
                    "hand_count" => Some(Method::Hand),
                    "same_row" => Some(Method::Row),
                    "same_bond_diff_lv" => Some(Method::Tiers),
                    _ => None,
                };
                if let Some(method) = method {
                    Effect::Layers {
                        targets: named,
                        formula: Formula::Method(method, n(&bb["multi"], 1.)),
                        active,
                        row_min: None,
                    }
                } else {
                    Effect::Unsupported(format!("SERVER_ADD_BOND_METHOD {}", s["add_method"]))
                }
            }
            "SERVER_ADD_BOND_ROUND_COIN_COST" => Effect::Layers {
                targets: named,
                formula: Formula::Spent {
                    per: count.max(1.),
                    layer: n(&bb["layer"], 0.),
                },
                active,
                row_min: None,
            },
            "SERVER_ADD_REFRESH_CNT_MULTIPLIER_BOND_LAYER" => Effect::Layers {
                targets: named,
                formula: Formula::Refresh {
                    multi: n(&bb["multiplier"], 0.),
                    max: n(&bb["max_layer"], 0.),
                },
                active,
                row_min: None,
            },
            "SERVER_ADD_BOND_ACTIVATED_MOST_LAYER" => Effect::Most {
                count,
                diversity: false,
            },
            "SERVER_ADD_ACT_BOND_DIFF_LV_MOST_LAYER" => Effect::Most {
                count: n(&bb["multi"], 1.),
                diversity: true,
            },
            "SERVER_ADD_BOND_IN_HAND" => Effect::Hand(count),
            "SERVER_ADD_BOND_POSITION" => Effect::Position {
                behind: s["dir"] == "behind",
                count,
            },
            "SERVER_GAIN_BOND_LAYER_BY_REFRESH_CNT" => Effect::FirstRefresh {
                ids: ids_of_bonds(&s["bond"]),
                count: n(&bb["layer"], 0.),
                at: n(&bb["refresh_cnt"], 1.),
                active,
            },
            "SERVER_ONCE_GOLD" => Effect::Pending {
                count,
                condition: vec![],
            },
            "SERVER_ONCE_GOLD_WITH_BOND_CONDITION" => Effect::Pending {
                count,
                condition: ids_of_bonds(&s["bond"]),
            },
            "SERVER_GAIN_CHAR" => Effect::Recruit(Recruit::Fixed {
                key: s["chess"].as_str().unwrap_or("").into(),
                count: n(&bb["count"], 1.),
            }),
            "SERVER_POOL_CHAR" => Effect::Recruit(Recruit::Pool {
                key: s["pool"].as_str().unwrap_or("").into(),
                count: n(&bb["count"], 1.),
                row_min: (s["conditionkey"] == "character_same_row")
                    .then(|| n(&bb["check_count"], 0.)),
            }),
            "SERVER_MOST_BOND" => Effect::Recruit(Recruit::Most {
                row_min: (s["conditionkey"] == "character_same_row")
                    .then(|| n(&bb["check_count"], 0.)),
            }),
            "SERVER_SELL_CHESS_GAIN_SPECIAL_GOODS" => Effect::Recruit(Recruit::Offer {
                pools: (1..=6)
                    .map(|i| s[format!("pool{i}")].as_str().unwrap_or("").into())
                    .collect(),
                fallback: s["max_pool"].as_str().unwrap_or("").into(),
            }),
            "SERVER_TRIGGER_ANOTHER" => Effect::Trigger {
                hook: Hook::garrison(s["event"].as_str().unwrap_or("SERVER_GAIN")),
                scope: if s["scope"] == "front" {
                    TriggerScope::Front(1.)
                } else {
                    TriggerScope::Farthest
                },
            },
            "SERVER_TRIGGER_FRONT_COUNT" => Effect::Trigger {
                hook: Hook::garrison(s["event"].as_str().unwrap_or("SERVER_GAIN")),
                scope: TriggerScope::Front(n(&bb["count"], 1.)),
            },
            "SERVER_FRONT_SAME_EFFECT_PREP_FIN" | "SERVER_FRONT_SAME_EFFECT_PREP_START" => {
                Effect::Copy {
                    hook: if raw["effectKey"] == "SERVER_FRONT_SAME_EFFECT_PREP_FIN" {
                        Hook::PrepEnd
                    } else {
                        Hook::PrepStart
                    },
                    key: raw["effectKey"].as_str().unwrap().into(),
                }
            }
            "SERVER_ADD_BOND_ACTIVATED_RANDOM" => Effect::RandomLayers {
                count,
                repeats: n(&bb["rand_count"], 1.),
            },
            "SERVER_ADD_BOND_FRONT_ALL_LAYER" => Effect::ForwardLayers(count),
            "SERVER_GAIN_EQUIP" => Effect::Items {
                source: ItemSource::Fixed(s["chess"].as_str().unwrap_or("").into()),
                count: n(&bb["count"], 1.),
                rounds: vec![],
            },
            "SERVER_POOL_EQUIP" | "SERVER_GAIN_RANDOM_EQUIP_CHESS_IN_POOL" => Effect::Items {
                source: ItemSource::Pool {
                    id: s["pool"].as_str().unwrap_or("").into(),
                    golden: raw["effectKey"] == "SERVER_POOL_EQUIP" && id.ends_with("_b"),
                    fallback: raw["effectKey"] == "SERVER_GAIN_RANDOM_EQUIP_CHESS_IN_POOL",
                },
                count: n(&bb["count"], 1.),
                rounds: ids_of_bonds(&s["round_list"])
                    .iter()
                    .map(|s| s.parse().unwrap_or(-1.))
                    .collect(),
            },
            "SERVER_GAIN_FREE_REFRESH_COUNT" => Effect::Free(n(&bb["count"], 1.)),
            "SERVER_CHESS_PRICE" => Effect::Discount(n(&bb["price"], 0.)),
            key => Effect::Unsupported(key.into()),
        };
        Some(Self {
            hook,
            board_only: s["conditionkey"] == "character_target_inboard",
            effect,
        })
    }
}
impl Meta {
    pub fn new(
        data: &Catalog,
        mode: &Value,
        seed: u32,
        config: EffectsConfig,
    ) -> Result<Self, String> {
        let bonds = Bond::load(data.table("bonds").unwrap(), mode, &config.disabled_bonds)?;
        if config.layers.len() > 256 || config.count_bonus.len() > 256 {
            return Err("Bond state capacity exceeded".into());
        }
        for (id, amount) in &config.layers {
            if !amount.is_finite() || *amount < 0. || !bonds.iter().any(|b| &b.id == id) {
                return Err("Invalid initial bond layers".into());
            }
        }
        for id in config.count_bonus.keys() {
            if !bonds.iter().any(|b| &b.id == id) {
                return Err("Unknown bond count bonus".into());
            }
        }
        let garrisons = data.table("garrisons").unwrap().as_object().unwrap();
        if garrisons.len() > 4096 {
            return Err("Garrison definition capacity exceeded".into());
        }
        let rules = Rules {
            bonds,
            garrisons: garrisons
                .iter()
                .filter_map(|(id, raw)| Rule::load(id, raw).map(|r| (id.clone(), r)))
                .collect(),
        };
        Ok(Self {
            rules: Arc::new(rules),
            layers: config.layers,
            bonus: config.count_bonus,
            bonds: BTreeMap::new(),
            rng: Rng::new(derive_seed(seed, "meta")),
            defer_items: 0,
            hammer_paid: 0.,
            round: 0,
            phase: Phase::Prep,
            depth: 0,
            refresh: None,
            ended_round: None,
            visi_paid: 0.,
            mira_paid: 0.,
            discount: 0,
        })
    }
    fn active(&self, id: &str) -> bool {
        self.bonds.get(id).is_some_and(State::active)
    }
    fn buff(&self, id: &str, key: &str) -> Value {
        self.rules
            .bonds
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.buff(key))
            .unwrap_or_else(|| json!({}))
    }
    pub fn cold_wind(&self) -> Option<crate::battle::runtime::ColdWind> {
        let bond = self.rules.bonds.iter().find(|b| b.id == "kjeragShip")?;
        let state = self.bonds.get(&bond.id)?;
        let bb = self.buff(&bond.id, "env_gbuff_new");
        let need = n(&bb["power_bond_char_cnt"], 0.);
        let reached = need > 0.
            && (state.count >= need
                || bond
                    .thresholds
                    .iter()
                    .position(|v| *v == need)
                    .is_some_and(|i| state.tier >= i + 1));
        let interval = n(&bb["bond_eff_kjerag[storm].interval"], 0.);
        if !state.active() || !reached || interval <= 0. {
            return None;
        }
        Some(crate::battle::runtime::ColdWind {
            interval,
            duration: n(&bb["bond_eff_kjerag[storm].base_time"], 0.)
                + n(&bb["bond_eff_kjerag[storm].time_per_stack"], 0.) * state.layers,
            first: None,
        })
    }
    pub fn active_sum(&self) -> f64 {
        self.rules
            .bonds
            .iter()
            .filter_map(|b| self.bonds.get(&b.id))
            .filter(|b| b.active())
            .fold(0., |sum, b| sum + b.layers)
    }
    pub fn report(&self, free: u64) -> Value {
        json!({"rng":self.rng.state(),"round":self.round,"layers":self.layers,"bonds":self.bonds.iter().map(|(id,b)| {let mut v=json!({"count":b.count,"tier":b.tier,"active":b.active(),"layers":b.layers});if b.harmony>0 {v["harmony"]=json!(b.harmony);} (id.clone(),v)}).collect::<BTreeMap<_,_>>(),"freeRefreshes":free,"activatedLayers":self.active_sum(),"visiPaid":self.visi_paid,"miraPaid":self.mira_paid,"hammerPaid":self.hammer_paid,"discount":self.discount,"prepEndedRound":self.ended_round})
    }
}
impl Preparation {
    pub(super) fn recompute_meta(&mut self, player: &mut Player) -> Result<(), Error> {
        let states = self.bond_states();
        let Some(meta) = &mut self.meta else {
            return Ok(());
        };
        let changed = states.iter().any(|(id, b)| b.active() != meta.active(id));
        meta.bonds = states;
        if changed {
            self.dispatch_meta(Hook::Active, None, player)?;
        }
        Ok(())
    }
    pub(in crate::solo::preparation) fn add_layers(
        &mut self,
        id: &str,
        amount: f64,
        active: bool,
        player: &mut Player,
    ) -> Result<(), Error> {
        let Some(meta) = &mut self.meta else {
            return Ok(());
        };
        if !amount.is_finite()
            || amount <= 0.
            || !meta.rules.bonds.iter().any(|b| b.id == id)
            || active && !meta.active(id)
        {
            return Ok(());
        }
        let before = *meta.layers.get(id).unwrap_or(&0.);
        let gained = amount.floor().min((999. - before).max(0.));
        if gained <= 0. {
            return Ok(());
        }
        meta.layers.insert(id.into(), before + gained);
        self.recompute_meta(player)?;
        self.dispatch_meta(Hook::Layers(id.into()), None, player)
    }
    fn member_tiers(&self, bond: &str) -> usize {
        self.board
            .values()
            .filter(|p| p.kind == Kind::Chess && self.piece_bonds(p).iter().any(|b| b == bond))
            .map(|p| self.rules.chess[&p.key].tier)
            .collect::<BTreeSet<_>>()
            .len()
    }
    fn same_row(&self, piece: &Piece) -> usize {
        let Some(Location::Board(at)) = self.find(piece.uid) else {
            return 0;
        };
        self.board
            .iter()
            .filter(|(p, c)| p[0] == at[0] && c.kind == Kind::Chess)
            .count()
    }
    fn formula(&self, f: &Formula, piece: &Piece, bond: &str, player: &Player) -> f64 {
        match f {
            Formula::Fixed(n) => *n,
            Formula::Method(method, multi) => {
                *multi
                    * match method {
                        Method::Shop => self.level as f64,
                        Method::Acquired => self.round.gained_chess as u32 as i32 as f64,
                        Method::Hand => self
                            .hand
                            .iter()
                            .flatten()
                            .filter(|p| p.kind == Kind::Chess)
                            .count() as f64,
                        Method::Row => self.same_row(piece) as f64,
                        Method::Tiers => self.member_tiers(bond) as f64,
                    }
            }
            Formula::Spent { per, layer } => {
                (player.round_spent as u32 as i32 as f64 / per).floor() * layer
            }
            Formula::Refresh { multi, max } => {
                let n = self.round.refreshes as u32 as i32 as f64 * multi;
                if *max > 0. {
                    n.min(*max)
                } else {
                    n
                }
            }
        }
    }
    fn run_rule(&mut self, rule: &Rule, call: &Call, player: &mut Player) -> Result<(), Error> {
        let piece = &call.piece;
        match &rule.effect {
            Effect::Layers {
                targets,
                formula,
                active,
                row_min,
            } => {
                if !self.row_condition(*row_min, call, &rule.hook) {
                    return Ok(());
                }
                let bonds = match targets {
                    Targets::Own => self
                        .piece_bonds(piece)
                        .into_iter()
                        .map(|b| (b, 1.))
                        .collect(),
                    Targets::Named(ids) => ids.iter().cloned().map(|b| (b, 1.)).collect(),
                    Targets::Multiple(ids) => ids.clone(),
                };
                for (bond, scale) in bonds {
                    let amount = self.formula(formula, piece, &bond, player) * scale;
                    self.add_layers(&bond, amount, *active, player)?;
                }
            }
            Effect::Most { count, diversity } => {
                let meta = self.meta.as_ref().unwrap();
                let mut best = None;
                let mut layers = -1.;
                for bond in &meta.rules.bonds {
                    if let Some(state) = meta.bonds.get(&bond.id).filter(|s| s.active()) {
                        if state.layers > layers {
                            layers = state.layers;
                            best = Some(bond.id.clone());
                        }
                    }
                }
                if let Some(bond) = best {
                    let n = if *diversity {
                        self.member_tiers(&bond) as f64 * count
                    } else {
                        *count
                    };
                    self.add_layers(&bond, n, true, player)?;
                }
            }
            Effect::Hand(count) => {
                let keys = self
                    .hand
                    .iter()
                    .flatten()
                    .filter(|p| p.kind == Kind::Chess)
                    .cloned()
                    .collect::<Vec<_>>();
                for p in keys {
                    for bond in self.piece_bonds(&p) {
                        self.add_layers(&bond, *count, true, player)?;
                    }
                }
            }
            Effect::Position { behind, count } => {
                let Some(Location::Board(at)) = self.find(piece.uid) else {
                    return Ok(());
                };
                let [r, c] = piece
                    .direction
                    .unwrap_or(crate::battle::direction::Direction::Right)
                    .vector();
                let sign = if *behind { -1 } else { 1 };
                let mut keys = vec![piece.clone()];
                if let Some(other) = self
                    .board
                    .get(&[at[0] + sign * r, at[1] + sign * c])
                    .filter(|p| p.kind == Kind::Chess && p.uid != piece.uid)
                {
                    keys.push(other.clone());
                }
                for p in keys {
                    for bond in self.piece_bonds(&p) {
                        self.add_layers(&bond, *count, true, player)?;
                    }
                }
            }
            Effect::FirstRefresh {
                ids,
                count,
                at,
                active,
            } => {
                if call.triggered {
                    return Ok(());
                }
                let loc = self.find(piece.uid);
                let p = match loc {
                    Some(Location::Hand(i)) => self.hand[i].as_mut().unwrap(),
                    Some(Location::Temp(i)) => self.temp[i].as_mut().unwrap(),
                    Some(Location::Board(at)) => self.board.get_mut(&at).unwrap(),
                    None => match self
                        .meta
                        .as_mut()
                        .unwrap()
                        .refresh
                        .as_mut()
                        .and_then(|r| r.merged.get_mut(&piece.uid))
                    {
                        Some(p) => p,
                        None => return Ok(()),
                    },
                };
                p.refresh_seen = add(p.refresh_seen, 1).map_err(|_| Error::Capacity)?;
                if p.refresh_seen as f64 == *at {
                    for bond in ids {
                        self.add_layers(bond, *count, *active, player)?;
                    }
                }
            }
            Effect::Pending { count, condition } => {
                if call.origin != Origin::Hand
                    || condition.is_empty()
                    || condition
                        .iter()
                        .any(|b| self.meta.as_ref().unwrap().active(b))
                {
                    let n = count.trunc().max(0.);
                    if n > MAX_FUNDS as f64 {
                        return Err(Error::Capacity);
                    }
                    player.pending = add(player.pending, n as u64).map_err(|_| Error::Capacity)?;
                }
            }
            Effect::Free(count) => {
                let n = count.trunc().max(0.);
                if n > MAX_FUNDS as f64 {
                    return Err(Error::Capacity);
                }
                self.free_refreshes =
                    add(self.free_refreshes, n as u64).map_err(|_| Error::Capacity)?;
            }
            Effect::Discount(_) => (),
            Effect::Items {
                source,
                count,
                rounds,
            } => {
                if rounds.is_empty() || rounds.contains(&(self.meta.as_ref().unwrap().round as f64))
                {
                    self.grant_items(source, *count, player)?;
                }
            }
            Effect::Recruit(recruit) => self.recruit(recruit, call, &rule.hook, player)?,
            Effect::Trigger {
                hook: Some(hook),
                scope,
            } if *hook != Hook::Price => match scope {
                TriggerScope::Front(count) => {
                    for n in 1..=count.clamp(0., 8.).floor() as i32 {
                        if let Some(target) = self.front(piece, n) {
                            self.trigger_garrisons(&target, None, hook, player)?;
                        }
                    }
                }
                TriggerScope::Farthest => {
                    let mut candidates = self.board_pieces();
                    candidates.sort_by_key(|(at, _)| (-at[0], -at[1]));
                    if let Some(target) = candidates
                        .into_iter()
                        .map(|(_, p)| p)
                        .find(|p| p.uid != piece.uid && self.has_hook(p, hook))
                    {
                        self.trigger_garrisons(&target, None, hook, player)?;
                    }
                }
            },
            Effect::Trigger { .. } => (),
            Effect::Copy { hook, key } => {
                let mut target = self.front(piece, 1);
                for _ in 0..8 {
                    let Some(p) = &target else { break };
                    let traits = self
                        .garrison_ids(&p.key)
                        .into_iter()
                        .filter(|id| {
                            self.meta
                                .as_ref()
                                .unwrap()
                                .rules
                                .garrisons
                                .get(id)
                                .is_some_and(|r| r.hook == *hook)
                        })
                        .collect::<Vec<_>>();
                    if traits.is_empty() {
                        return Ok(());
                    }
                    if !traits.iter().all(|id| {
                        self.rules.data.table("garrisons").unwrap()[id]["effectKey"] == *key
                    }) {
                        break;
                    }
                    target = self.front(p, 1);
                }
                if let Some(target) = target.filter(|p| p.uid != piece.uid) {
                    self.trigger_garrisons(&target, Some(piece), hook, player)?;
                }
            }
            Effect::RandomLayers { count, repeats } => {
                let meta = self.meta.as_ref().unwrap();
                let active = meta
                    .rules
                    .bonds
                    .iter()
                    .filter(|b| meta.active(&b.id))
                    .map(|b| b.id.clone())
                    .collect::<Vec<_>>();
                for _ in 0..repeats.clamp(0., 10.).ceil() as usize {
                    if let Some(id) = self.meta.as_mut().unwrap().rng.pick(&active).cloned() {
                        self.add_layers(&id, *count, true, player)?;
                    }
                }
            }
            Effect::ForwardLayers(count) => {
                if let Some(Location::Board(at)) = self.find(piece.uid) {
                    let [r, c] = piece
                        .direction
                        .unwrap_or(crate::battle::direction::Direction::Right)
                        .vector();
                    for (other, p) in self.board_pieces() {
                        if (other[0] - at[0]) * c == (other[1] - at[1]) * r
                            && (other[0] - at[0]) * r + (other[1] - at[1]) * c > 0
                        {
                            for bond in self.piece_bonds(&p) {
                                self.add_layers(&bond, *count, true, player)?;
                            }
                        }
                    }
                }
            }
            Effect::Unsupported(key) => {
                return Err(Error::Content(format!(
                    "Unsupported preparation effect: {key}"
                )))
            }
        }
        Ok(())
    }
    fn settle_meta_milestones(&mut self, hook: &Hook, player: &mut Player) -> Result<(), Error> {
        for id in ["victoriaShip", "visiShip", "miraShip"] {
            if matches!(hook,Hook::Layers(b) if b!=id)
                || !matches!(
                    hook,
                    Hook::Layers(_)
                        | Hook::Active
                        | Hook::RoundStart
                        | Hook::PrepStart
                        | Hook::PrepEnd
                        | Hook::Buy
                        | Hook::Gain
                        | Hook::Sold
                        | Hook::Merge
                )
            {
                continue;
            }
            let meta = self.meta.as_ref().unwrap();
            if !meta.active(id) {
                continue;
            }
            if id == "victoriaShip" {
                let b = meta.buff(id, "bond_layer_added_reward_equip");
                let step = n(&b["layer"], 0.).floor();
                if step > 0. {
                    let due = (meta.layers.get(id).copied().unwrap_or(0.) / step).floor();
                    let count = n(&b["count"], 1.).floor().max(0.);
                    if count > self.capacity as f64 {
                        return Err(Error::Capacity);
                    }
                    for _ in 0..100 {
                        let meta = self.meta.as_mut().unwrap();
                        if meta.hammer_paid >= due {
                            break;
                        }
                        meta.hammer_paid += 1.;
                        for _ in 0..count as usize {
                            if let Some(key) = self.roll_meta_item(b["pool"].as_str(), 6, false)? {
                                self.gain(Kind::Item, &key, GainSource::Grant, player)?;
                            }
                        }
                    }
                }
                continue;
            }
            let b = meta.buff(id, "bond_layer_gain_coin");
            let step = n(&b["layer"], 0.);
            let count = n(&b["count"], 0.);
            if step > 0. && count > 0. {
                let due = (meta.layers.get(id).copied().unwrap_or(0.) / step).floor();
                let paid = if id == "visiShip" {
                    meta.visi_paid
                } else {
                    meta.mira_paid
                };
                if due > paid {
                    let gain = (due - paid) * count;
                    if !gain.is_finite() || gain > MAX_FUNDS as f64 {
                        return Err(Error::Capacity);
                    }
                    let pending = meta.phase == Phase::Prep && meta.ended_round == Some(meta.round);
                    let meta = self.meta.as_mut().unwrap();
                    if id == "visiShip" {
                        meta.visi_paid = due;
                    } else {
                        meta.mira_paid = due;
                    }
                    if pending {
                        player.pending = add(player.pending, gain.trunc() as u64)
                            .map_err(|_| Error::Capacity)?;
                    } else {
                        player
                            .earn(gain.trunc() as u64)
                            .map_err(|_| Error::Capacity)?;
                    }
                }
            }
            if id == "visiShip" {
                let meta = self.meta.as_mut().unwrap();
                let b = meta.buff(id, "bond_multi_layer_char_goods_price_bond_discount");
                let own = meta.buff(id, "bond_layer_char_goods_price_bond_discount");
                let all = meta.buff(id, "bond_layer_char_goods_price_discount");
                let layers = meta.layers.get(id).copied().unwrap_or(0.);
                let t = if layers >= n(&b["layer2"], n(&all["layer"], f64::INFINITY)) {
                    2
                } else if layers >= n(&b["layer1"], n(&own["layer"], f64::INFINITY)) {
                    1
                } else {
                    0
                };
                meta.discount = meta.discount.max(t);
            }
        }
        Ok(())
    }
    pub(super) fn meta_price(&self, key: &str, price: f64) -> Result<f64, Error> {
        let Some(meta) = &self.meta else {
            return Ok(price.round().max(0.));
        };
        let mut price = price;
        for id in self.rules.data.table("chess").unwrap()[key]["garrisonIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(rule) = meta
                .rules
                .garrisons
                .get(id)
                .filter(|r| r.hook == Hook::Price)
            {
                match &rule.effect {
                    Effect::Discount(n) => price = (price - n).max(0.),
                    Effect::Unsupported(k) => {
                        return Err(Error::Content(format!(
                            "Unsupported preparation effect: {k}"
                        )))
                    }
                    _ => (),
                }
            }
        }
        if meta.discount > 0 {
            let b = meta.buff(
                "visiShip",
                "bond_multi_layer_char_goods_price_bond_discount",
            );
            let old = meta.buff("visiShip", "bond_layer_char_goods_price_discount");
            let own = meta.buff("visiShip", "bond_layer_char_goods_price_bond_discount");
            let bond = b["bond"]
                .as_str()
                .or(own["bond"].as_str())
                .unwrap_or("visiShip");
            if meta.discount >= 2 || self.own_bonds(key).iter().any(|id| id == bond) {
                price = (price - n(&b["discount"], n(&old["discount"], n(&own["discount"], 0.))))
                    .max(0.);
            }
        }
        if self.sciurus && !self.round.sciurus_bought && self.own_bonds(key).iter().any(|b| b == "kjeragShip") {
            price = price.min(1.);
        }
        Ok(price.round().max(0.))
    }
    pub(super) fn meta_begin(
        &mut self,
        round: u32,
        player: &mut Player,
        defer_prep: bool,
    ) -> Result<(), Error> {
        if let Some(meta) = &mut self.meta {
            meta.round = round;
            meta.phase = Phase::Prep;
        }
        for piece in self
            .hand
            .iter_mut()
            .chain(&mut self.temp)
            .flatten()
            .chain(self.board.values_mut())
        {
            piece.refresh_seen = 0;
        }
        self.recompute_meta(player)?;
        self.dispatch_meta(Hook::RoundStart, None, player)?;
        self.equipment_event(super::equipment_lifecycle::Event::RoundStart, player)?;
        player
            .earn(self.round_income)
            .map_err(|_| Error::Capacity)?;
        if defer_prep {
            return Ok(());
        }
        self.enter_prep(player)
    }
    pub(in crate::solo) fn enter_prep(&mut self, player: &mut Player) -> Result<(), Error> {
        self.check_item_merges(player)?;
        self.dispatch_meta(Hook::PrepStart, None, player)?;
        self.recompute_meta(player)
    }
    pub(in crate::solo) fn meta_end(&mut self, player: &mut Player) -> Result<(), Error> {
        self.dispatch_meta(Hook::PrepEnd, None, player)
    }
    pub(super) fn meta_combat(&mut self) {
        if let Some(meta) = &mut self.meta {
            meta.phase = Phase::Combat;
        }
    }
}
