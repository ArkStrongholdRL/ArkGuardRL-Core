// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-player shop, inventory, bond membership and preparation effects.
use super::map::DeployMap;
use super::{add, player::Player, Operator, PreparedRound, MAX_FUNDS};
use crate::{
    battle::{direction::Direction, operators::Placement},
    data::Catalog,
    deterministic::rng::{derive_seed, Rng},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
mod arts;
mod bonds;
mod bounties;
mod consumables;
mod drafts;
mod effects;
mod equipment;
mod equipment_lifecycle;
mod input;
mod items;
pub use bonds::EffectsConfig;
use effects::{Hook, Meta};
use items::GainSource;
mod rules;
mod summons;
mod view;
use rules::Rules;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Chess,
    Item,
    Token,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Chess => "chess",
            Self::Item => "item",
            Self::Token => "token",
        }
    }
}
pub struct Grant {
    pub kind: Kind,
    pub key: String,
}
pub struct Config {
    pub mode_id: String,
    pub banned: Vec<String>,
    pub initial: Vec<Grant>,
    pub capacity: usize,
    pub effects: Option<EffectsConfig>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Hand(usize),
    Board {
        home: [i32; 2],
        direction: Direction,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Draft(usize),
    Choice {
        index: usize,
        id: String,
    },
    Buy(usize),
    Refresh,
    Freeze,
    LevelUp,
    Sell(u64),
    Destroy(u64),
    PickReward(usize),
    Move {
        uid: u64,
        to: Destination,
    },
    Equip {
        item: u64,
        target: u64,
        replace: Option<u64>,
    },
    Unequip(u64),
    Art {
        item: u64,
        tile: [i32; 2],
        direction: Direction,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    BadTarget,
    SoldOut,
    NoFunds,
    HandFull,
    BoardFull,
    BadTile,
    MaxLevel,
    Capacity,
    Content(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BadTarget => "BAD_TARGET",
            Self::SoldOut => "SOLD_OUT",
            Self::NoFunds => "NO_FUNDS",
            Self::HandFull => "HAND_FULL",
            Self::BoardFull => "BOARD_FULL",
            Self::BadTile => "BAD_TILE",
            Self::MaxLevel => "MAX_LEVEL",
            Self::Capacity => "Preparation capacity exceeded",
            Self::Content(message) => message,
        })
    }
}
#[derive(Clone)]
struct Piece {
    uid: u64,
    kind: Kind,
    key: String,
    copies: u64,
    count: u64,
    owner_uid: Option<u64>,
    direction: Option<Direction>,
    refresh_seen: u64,
    sale_charge: u32,
    items: Vec<Piece>,
}
#[derive(Clone, Copy)]
enum Location {
    Hand(usize),
    Temp(usize),
    Board([i32; 2]),
}
#[derive(Clone)]
struct Slot {
    kind: Kind,
    key: String,
    price: f64,
    frozen: bool,
    sold: bool,
}
#[derive(Clone)]
struct Offer {
    tier: usize,
    slots: Vec<Slot>,
}
#[derive(Clone)]
struct Stock {
    cap: i64,
    left: i64,
    tier: usize,
}
#[derive(Clone, Default)]
struct Counts {
    buys: u64,
    sells: u64,
    refreshes: u64,
    merges: u64,
    item_merges: u64,
    items_equipped: u64,
}
#[derive(Clone, Default)]
struct RoundCounts {
    sciurus_bought: bool,
    buys: u64,
    sells: u64,
    refreshes: u64,
    gained_chess: u64,
    cauldron: BTreeMap<u64, u32>,
    arts: u64,
}
#[derive(Clone)]
pub(super) struct Preparation {
    pub(in crate::solo) sciurus: bool,
    rules: Arc<Rules>,
    personal_choice: Option<bounties::Choice>,
    draft_effects: drafts::DraftEffects,
    bounties: Vec<bounties::Bounty>,
    meta: Option<Meta>,
    free_refreshes: u64,
    grid: Arc<DeployMap>,
    boss: bool,
    deploy_cap: usize,
    deploy_min: usize,
    round_income: u64,
    capacity: usize,
    rng: Rng,
    uid: u64,
    pool: BTreeMap<String, Stock>,
    debt: BTreeMap<String, u64>,
    hand: [Option<Piece>; 10],
    temp: Vec<Option<Piece>>,
    board: BTreeMap<[i32; 2], Piece>,
    board_insertion: Vec<[i32; 2]>,
    level: usize,
    upgrade_price: f64,
    frozen: bool,
    slots: Vec<Option<Slot>>,
    layout: [usize; 2],
    offers: VecDeque<Offer>,
    counts: Counts,
    round: RoundCounts,
}
impl Preparation {
    pub fn new(
        data: Arc<Catalog>,
        seed: u32,
        config: Config,
        grid: DeployMap,
        boss: bool,
        deploy_cap: usize,
        player: &mut Player,
    ) -> Result<Self, String> {
        if !(15..=4096).contains(&config.capacity)
            || config.initial.len() > config.capacity
            || config.banned.len() > 4096
        {
            return Err("Invalid preparation limits".into());
        }
        let meta = config
            .effects
            .map(|cfg| {
                Meta::new(
                    &data,
                    &data.table("config").unwrap()["modes"][&config.mode_id],
                    seed,
                    cfg,
                )
            })
            .transpose()?;
        let rules = Arc::new(Rules::load(data, &config.mode_id)?);
        let mut banned: BTreeSet<_> = config.banned.into_iter().collect();
        if let Some(meta) = &meta {
            for (key, rec) in rules.data.table("chess").unwrap().as_object().unwrap() {
                if rec["diyFor"].is_string()
                    && rec["bonds"].as_array().is_some_and(|bonds| {
                        !bonds.is_empty()
                            && bonds.iter().all(|b| {
                                meta.rules
                                    .bonds
                                    .iter()
                                    .any(|rule| Some(rule.id.as_str()) == b.as_str() && rule.off)
                            })
                    })
                {
                    banned.insert(key.clone());
                }
            }
        }
        let pool: BTreeMap<_, _> = rules
            .chess
            .values()
            .filter(|c| c.shop && !banned.contains(&c.key) && c.cap.unwrap_or(0) > 0)
            .map(|c| {
                (
                    c.key.clone(),
                    Stock {
                        cap: c.cap.unwrap() as i64,
                        left: c.cap.unwrap() as i64,
                        tier: c.tier,
                    },
                )
            })
            .collect();
        let total = pool
            .values()
            .try_fold(0_u64, |sum, e| add(sum, e.cap as u64))?;
        if total > MAX_FUNDS {
            return Err("Preparation stock capacity exceeded".into());
        }
        let upgrade_price = rules.upgrades[0];
        let mut prep = Self {
            sciurus: false,
            personal_choice: None,
            draft_effects: Default::default(),
            bounties: Vec::new(),
            rules,
            meta,
            free_refreshes: 0,
            grid: Arc::new(grid),
            boss,
            deploy_cap,
            deploy_min: 0,
            round_income: 0,
            capacity: config.capacity,
            rng: Rng::new(derive_seed(seed, "shop")),
            uid: 0,
            pool,
            debt: BTreeMap::new(),
            hand: std::array::from_fn(|_| None),
            temp: vec![None; 5],
            board: BTreeMap::new(),
            board_insertion: Vec::new(),
            level: 1,
            upgrade_price,
            frozen: false,
            slots: vec![],
            layout: [0, 0],
            offers: VecDeque::new(),
            counts: Counts::default(),
            round: RoundCounts::default(),
        };
        prep.recompute_meta(player).map_err(|e| e.to_string())?;
        for grant in config.initial {
            prep.gain(grant.kind, &grant.key, GainSource::Grant, player)
                .map_err(|e| e.to_string())?;
        }
        Ok(prep)
    }
    pub fn begin_round(
        &mut self,
        round: u32,
        grid: DeployMap,
        boss: bool,
        deploy_cap: usize,
        player: &mut Player,
        defer_prep: bool,
    ) -> Result<(), Error> {
        self.grid = Arc::new(grid);
        self.boss = boss;
        self.deploy_cap = deploy_cap.max(self.deploy_min);
        self.round = RoundCounts::default();
        if round > 1 {
            self.upgrade_price = (self.upgrade_price - 1.).max(0.);
        }
        for (_, owner) in self.owner_positions() {
            self.grant_tokens(owner)?;
        }
        self.roll(true);
        self.frozen = false;
        for slot in self.slots.iter_mut().flatten() {
            slot.frozen = false;
        }
        self.evict_illegal()?;
        self.meta_begin(round, player, defer_prep)
    }
    pub fn change_map(&mut self, grid: DeployMap, player: &mut Player) -> Result<(), Error> {
        self.grid = Arc::new(grid);
        self.evict_illegal()?;
        self.recompute_meta(player)
    }
    fn evict_illegal(&mut self) -> Result<(), Error> {
        let mut evicted: Vec<_> = self
            .board
            .iter()
            .filter(|(at, p)| !self.legal(p, **at))
            .map(|(at, _)| *at)
            .collect();
        evicted.sort_by_key(|at| (-at[0], at[1]));
        for at in evicted {
            let Some(piece) = self.remove_board(&at) else {
                continue;
            };
            if piece.kind == Kind::Chess {
                self.remove_tokens(piece.uid);
            }
            self.stow(piece)?;
        }
        self.refill();
        Ok(())
    }
    pub fn end_prep(&mut self) {
        self.meta_combat();
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|s| s.sold || !s.frozen) {
                *slot = None;
            }
        }
    }
    // JS Map iteration controls which token UID survives stack recombination.
    fn insert_board(&mut self, at: [i32; 2], piece: Piece) {
        if !self.board.contains_key(&at) {
            self.board_insertion.push(at);
        }
        self.board.insert(at, piece);
    }
    fn remove_board(&mut self, at: &[i32; 2]) -> Option<Piece> {
        let piece = self.board.remove(at);
        if piece.is_some() {
            self.board_insertion.retain(|key| key != at);
        }
        piece
    }
    /// Node boardOrder: descending row, then left to right.
    fn board_order(&self) -> Vec<([i32; 2], &Piece)> {
        let mut board: Vec<_> = self.board.iter().map(|(at, p)| (*at, p)).collect();
        board.sort_by_key(|(at, _)| (-at[0], at[1]));
        board
    }
    /// Bookkeeping only: no merge, callback or deployment ordering implied.
    fn pieces(&self) -> impl Iterator<Item = &Piece> {
        self.temp
            .iter()
            .chain(&self.hand)
            .flatten()
            .chain(self.board.values())
    }
    fn locations(&self) -> Vec<Location> {
        self.temp
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|_| Location::Temp(i)))
            .chain(
                self.hand
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| p.as_ref().map(|_| Location::Hand(i))),
            )
            .chain(
                self.board_order()
                    .into_iter()
                    .map(|(at, _)| Location::Board(at)),
            )
            .collect()
    }
    fn piece(&self, loc: Location) -> &Piece {
        match loc {
            Location::Hand(i) => self.hand[i].as_ref().unwrap(),
            Location::Temp(i) => self.temp[i].as_ref().unwrap(),
            Location::Board(at) => &self.board[&at],
        }
    }
    fn piece_mut(&mut self, loc: Location) -> &mut Piece {
        match loc {
            Location::Hand(i) => self.hand[i].as_mut().unwrap(),
            Location::Temp(i) => self.temp[i].as_mut().unwrap(),
            Location::Board(at) => self.board.get_mut(&at).unwrap(),
        }
    }
    fn take(&mut self, loc: Location) -> Piece {
        match loc {
            Location::Hand(i) => self.hand[i].take().unwrap(),
            Location::Temp(i) => self.temp[i].take().unwrap(),
            Location::Board(at) => self.remove_board(&at).unwrap(),
        }
    }
    fn put(&mut self, loc: Location, piece: Piece) {
        match loc {
            Location::Hand(i) => self.hand[i] = Some(piece),
            Location::Temp(i) => self.temp[i] = Some(piece),
            Location::Board(at) => {
                self.insert_board(at, piece);
            }
        }
    }
    fn find(&self, uid: u64) -> Option<Location> {
        // UIDs are unique; lookup does not need board reading order.
        self.temp
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.uid == uid))
            .map(Location::Temp)
            .or_else(|| {
                self.hand
                    .iter()
                    .position(|p| p.as_ref().is_some_and(|p| p.uid == uid))
                    .map(Location::Hand)
            })
            .or_else(|| {
                self.board
                    .iter()
                    .find(|(_, p)| p.uid == uid)
                    .map(|(at, _)| Location::Board(*at))
            })
    }
    fn free_hand(&self) -> Option<usize> {
        self.hand.iter().rposition(Option::is_none)
    }
    fn len(&self) -> usize {
        self.pieces().map(|p| 1 + p.items.len()).sum()
    }
    fn stow(&mut self, piece: Piece) -> Result<(), Error> {
        if let Some(i) = self.free_hand() {
            self.hand[i] = Some(piece);
            return Ok(());
        }
        if self.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        if self.temp.len() <= 5 {
            if let Some(i) = self.temp.iter().rposition(Option::is_none) {
                self.temp[i] = Some(piece);
                return Ok(());
            }
        }
        self.temp.push(Some(piece));
        Ok(())
    }
    fn refill(&mut self) {
        for j in (0..5).rev().chain(5..self.temp.len()) {
            if self.temp[j].is_some() {
                let Some(i) = self.free_hand() else { break };
                self.hand[i] = self.temp[j].take();
            }
        }
        if self.temp.len() > 5 {
            let pending = (0..5)
                .rev()
                .chain(5..self.temp.len())
                .filter_map(|i| self.temp[i].take())
                .collect::<Vec<_>>();
            self.temp = vec![None; 5];
            for (i, piece) in pending.into_iter().enumerate() {
                if i < 5 {
                    self.temp[4 - i] = Some(piece);
                } else {
                    self.temp.push(Some(piece));
                }
            }
        }
    }
    fn legal(&self, piece: &Piece, home: [i32; 2]) -> bool {
        match piece.kind {
            Kind::Chess => self
                .grid
                .can_place(&self.rules.data.table("chess").unwrap()[&piece.key], home),
            Kind::Token => self.token_legal(piece, home),
            Kind::Item => false,
        }
    }
    fn stock_take(&mut self, key: &str, n: u64) -> u64 {
        if let Some(e) = self.pool.get_mut(key) {
            e.left -= n as i64;
            n
        } else {
            0
        }
    }
    fn return_copies(&mut self, piece: &Piece) {
        if let Some(e) = self.pool.get_mut(&self.rules.chess[&piece.key].base) {
            e.left = (e.left + piece.copies as i64).min(e.cap);
        }
    }
    fn item_left(&self, key: &str) -> Option<u64> {
        let card = &self.rules.items[key];
        let cap = card.cap?;
        let held = self
            .pieces()
            .flat_map(|p| std::iter::once(p).chain(&p.items))
            .filter(|p| p.kind == Kind::Item && self.rules.items[&p.key].base == card.base)
            .map(|p| self.rules.items[&p.key].copies)
            .sum();
        Some(cap.saturating_sub(held))
    }
    fn item_available(&self, key: &str) -> bool {
        self.item_left(key)
            .is_none_or(|left| left >= self.rules.items[key].copies)
    }
    fn release_item(&mut self, piece: &Piece) {
        let card = &self.rules.items[&piece.key];
        if let Some(debt) = self.debt.get_mut(&card.base) {
            *debt = debt.saturating_sub(card.copies);
            if *debt == 0 {
                self.debt.remove(&card.base);
            }
        }
    }
    fn new_piece(&mut self, kind: Kind, key: &str, copies: u64) -> Result<Piece, Error> {
        self.uid = add(self.uid, 1).map_err(|_| Error::Capacity)?;
        Ok(Piece {
            uid: self.uid,
            kind,
            key: key.into(),
            copies,
            count: 1,
            owner_uid: None,
            direction: None,
            refresh_seen: 0,
            sale_charge: 0,
            items: Vec::new(),
        })
    }
    fn gain(
        &mut self,
        kind: Kind,
        key: &str,
        source: GainSource,
        player: &mut Player,
    ) -> Result<Option<u64>, Error> {
        let card = self.rules.card(kind, key).ok_or(Error::BadTarget)?;
        let displayed = matches!(source, GainSource::Displayed);
        let unlimited =
            matches!(source, GainSource::FixedGift) && self.rules.fixed_gifts.contains(&card.base);
        if kind == Kind::Item && !displayed && !unlimited && !self.item_available(key) {
            return Ok(None);
        }
        let (base, upgrade, need, golden_copies) = (
            card.base.clone(),
            card.upgrade.clone(),
            card.merge,
            card.copies,
        );
        let matches: Vec<_> = self
            .locations()
            .into_iter()
            .filter(|loc| {
                let p = self.piece(*loc);
                p.kind == kind && p.key == key
            })
            .collect();
        let deferred = kind == Kind::Item && self.meta.as_ref().is_some_and(|m| m.defer_items > 0);
        let count = if kind == Kind::Item {
            self.item_locations(key).len()
        } else {
            matches.len()
        };
        let merges = !deferred && need > 1 && count + 1 >= need;
        if !merges && self.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        if merges && kind == Kind::Chess && self.offers.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        let left = if kind == Kind::Item {
            self.item_left(key)
        } else {
            None
        };
        let copies = if kind == Kind::Chess {
            self.stock_take(&base, golden_copies)
        } else {
            0
        };
        let incoming = self.new_piece(kind, key, copies)?;
        if kind == Kind::Chess {
            self.round.gained_chess =
                add(self.round.gained_chess, 1).map_err(|_| Error::Capacity)?;
        }
        let owned = if merges && kind == Kind::Item {
            self.merge_items(key, Some(incoming), player)?
        } else if merges {
            let mut copies = incoming.copies;
            let mut tiles = Vec::new();
            let mut returned = Vec::new();
            for loc in matches.into_iter().take(need - 1) {
                let mut p = self.take(loc);
                self.remove_tokens(p.uid);
                returned.append(&mut p.items);
                self.record_refresh_merge(&p);
                copies += p.copies;
                if let Location::Board(at) = loc {
                    tiles.push((at, p.direction));
                }
            }
            let key = upgrade.unwrap();
            if kind == Kind::Chess {
                let extra = self.rules.chess[&key].copies.saturating_sub(copies);
                copies += self.stock_take(&base, extra);
            }
            let mut elite = self.new_piece(kind, &key, copies)?;
            let uid = elite.uid;
            tiles.sort_by_key(|(at, _)| (at[1], -at[0]));
            if let Some((at, dir)) = tiles.into_iter().find(|(at, _)| self.legal(&elite, *at)) {
                elite.direction = dir;
                self.insert_board(at, elite);
            } else {
                self.stow(elite)?;
            }
            for item in returned {
                self.stow(item)?;
            }
            self.check_item_merges(player)?;
            if self.board.values().any(|p| p.uid == uid) {
                self.grant_tokens(uid)?;
            }
            self.counts.merges = add(self.counts.merges, 1).map_err(|_| Error::Capacity)?;
            self.reward();
            uid
        } else {
            let uid = incoming.uid;
            self.stow(incoming)?;
            uid
        };
        if displayed && kind == Kind::Item {
            if let Some(left) = left {
                let shortfall = golden_copies.saturating_sub(left);
                if shortfall > 0 {
                    let debt = self.debt.entry(base).or_default();
                    *debt = add(*debt, shortfall).map_err(|_| Error::Capacity)?;
                }
            }
        }
        if merges && kind == Kind::Chess {
            self.dispatch_meta(Hook::Merge, None, player)?;
        }
        self.refill();
        self.recompute_meta(player)?;
        let piece = (kind == Kind::Chess).then(|| self.piece(self.find(owned).unwrap()).clone());
        if piece.is_some() {
            self.cauldron_gain(player)?;
        }
        self.dispatch_meta(Hook::Gain, piece, player)?;
        self.refill();
        self.recompute_meta(player)?;
        Ok(Some(owned))
    }
    // Shared stock is sampled first, then the seat's DIY stock in pick order.
    // In solo both have the same accounting owner, so take/give use the key map.
    fn stock_entries(&self) -> impl Iterator<Item = (&String, &Stock)> {
        self.pool
            .iter()
            .filter(|(key, _)| self.rules.data.table("chess").unwrap()[*key]["isDiy"] != true)
            .chain(
                self.rules
                    .data
                    .table("diy")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|picks| picks.keys())
                    .filter_map(|key| self.pool.get_key_value(key)),
            )
    }
    fn roll_chess(
        &mut self,
        max_tier: usize,
        tier: Option<usize>,
        exclude: &[String],
    ) -> Option<String> {
        let eligible: Vec<_> = self
            .stock_entries()
            .filter(|(key, e)| {
                e.left > 0
                    && tier.map_or(e.tier <= max_tier, |t| e.tier == t)
                    && !exclude.contains(key)
            })
            .map(|(key, e)| (key.clone(), e.left as f64))
            .collect();
        let weights: Vec<_> = eligible.iter().map(|(_, w)| *w).collect();
        self.rng.weighted(&weights).map(|i| eligible[i].0.clone())
    }
    fn roll_item(&mut self) -> Option<String> {
        let mut counts = BTreeMap::<usize, u64>::new();
        let mut total = 0_u64;
        for e in self
            .pool
            .iter()
            .filter(|(key, _)| self.rules.data.table("chess").unwrap()[*key]["isDiy"] != true)
            .map(|(_, e)| e)
            .filter(|e| e.left > 0 && e.tier <= self.level)
        {
            *counts.entry(e.tier).or_default() += e.left as u64;
            total += e.left as u64;
        }
        let tier = if total > 0 {
            let mut r = self.rng.next_f64();
            let mut picked = *counts.keys().next_back().unwrap();
            for (tier, n) in counts {
                r -= n as f64 / total as f64;
                if r < 0. {
                    picked = tier;
                    break;
                }
            }
            picked
        } else {
            1 + self.rng.int(self.level as u32) as usize
        };
        for t in (1..=tier).rev().chain(tier + 1..=6) {
            let list: Vec<_> = self
                .rules
                .items
                .values()
                .filter(|c| c.shop && c.tier == t && self.item_available(&c.key))
                .map(|c| c.key.clone())
                .collect();
            if !list.is_empty() {
                return self.rng.pick(&list).cloned();
            }
        }
        None
    }
    fn roll(&mut self, keep: bool) {
        let old = std::mem::take(&mut self.slots);
        let layout = self.rules.slots[self.level - 1];
        for (kind, start, n) in [
            (Kind::Chess, 0, layout[0]),
            (Kind::Item, self.layout[0], layout[1]),
        ] {
            for i in 0..n {
                let previous = if i < self.layout[if kind == Kind::Chess { 0 } else { 1 }] {
                    old.get(start + i)
                        .and_then(Option::as_ref)
                        .filter(|s| keep && s.kind == kind && s.frozen && !s.sold)
                        .cloned()
                } else {
                    None
                };
                let slot = previous.or_else(|| {
                    let key = if kind == Kind::Chess {
                        self.roll_chess(self.level, None, &[])
                    } else {
                        self.roll_item()
                    }?;
                    Some(Slot {
                        kind,
                        price: self.rules.card(kind, &key).unwrap().price,
                        key,
                        frozen: self.frozen,
                        sold: false,
                    })
                });
                self.slots.push(slot);
            }
        }
        self.layout = layout;
    }
    fn reward(&mut self) {
        let tier = (self.level as i64 + self.rules.reward_offset)
            .min(self.rules.reward_max_tier as i64)
            .max(0) as usize;
        let mut keys = Vec::new();
        for _ in 0..self.rules.reward_count {
            for t in (1..=tier).rev() {
                if let Some(key) = self.roll_chess(6, Some(t), &keys) {
                    keys.push(key);
                    break;
                }
            }
        }
        if !keys.is_empty() {
            self.offers.push_back(Offer {
                tier,
                slots: keys
                    .into_iter()
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
    fn charge(&mut self, account: &mut Player, price: f64) -> Result<(), Error> {
        if (account.funds as f64) < price {
            return Err(Error::NoFunds);
        }
        let price = price.trunc().max(0.) as u64;
        account.spend(price).map_err(|_| Error::Capacity)?;
        Ok(())
    }
    pub fn act(&mut self, action: Action, account: &mut Player) -> Result<(), Error> {
        match action {
            Action::Draft(_) => return Err(Error::BadTarget),
            Action::Choice { index, id } => self.pick_bounty(index, &id)?,
            Action::Art {
                item,
                tile,
                direction,
            } => self.use_art(item, tile, direction, account)?,
            Action::Buy(i) => {
                let slot = self
                    .slots
                    .get(i)
                    .and_then(Option::as_ref)
                    .ok_or(Error::BadTarget)?
                    .clone();
                if slot.sold {
                    return Err(Error::SoldOut);
                }
                let price = if slot.kind == Kind::Chess {
                    self.meta_price(&slot.key, slot.price)?
                } else {
                    slot.price.round().max(0.)
                };
                if (account.funds as f64) < price {
                    return Err(Error::NoFunds);
                }
                if self.free_hand().is_none() {
                    return Err(Error::HandFull);
                }
                self.charge(account, price)?;
                if slot.kind == Kind::Chess
                    && self.own_bonds(&slot.key).iter().any(|b| b == "kjeragShip")
                {
                    self.round.sciurus_bought = true;
                }
                self.slots[i].as_mut().unwrap().sold = true;
                let purchased = self.gain(slot.kind, &slot.key, GainSource::Displayed, account)?;
                self.counts.buys = add(self.counts.buys, 1).map_err(|_| Error::Capacity)?;
                self.round.buys = add(self.round.buys, 1).map_err(|_| Error::Capacity)?;
                self.dispatch_meta(Hook::Buy, None, account)?;
                self.upgrade_purchase(purchased, slot.kind, account)?;
            }
            Action::PickReward(i) => {
                let slot = self
                    .offers
                    .front()
                    .and_then(|o| o.slots.get(i))
                    .ok_or(Error::BadTarget)?
                    .clone();
                if slot.sold {
                    return Err(Error::SoldOut);
                }
                if self.free_hand().is_none() {
                    return Err(Error::HandFull);
                }
                self.charge(account, slot.price.trunc().max(0.))?;
                self.offers.pop_front();
                self.gain(slot.kind, &slot.key, GainSource::Displayed, account)?;
            }
            Action::Refresh => {
                if self.free_refreshes > 0 {
                    self.free_refreshes -= 1;
                } else {
                    self.charge(account, self.rules.refresh)?;
                }
                self.frozen = false;
                self.roll(false);
                self.counts.refreshes =
                    add(self.counts.refreshes, 1).map_err(|_| Error::Capacity)?;
                self.round.refreshes = add(self.round.refreshes, 1).map_err(|_| Error::Capacity)?;
                self.dispatch_meta(Hook::Refresh, None, account)?;
            }
            Action::Freeze => {
                self.frozen = !self.frozen;
                for slot in self.slots.iter_mut().flatten().filter(|s| !s.sold) {
                    slot.frozen = self.frozen;
                }
            }
            Action::LevelUp => {
                if self.level >= self.rules.slots.len() {
                    return Err(Error::MaxLevel);
                }
                self.charge(account, self.upgrade_price)?;
                self.level += 1;
                self.upgrade_price = self.rules.upgrades[self.level - 1];
                let layout = self.rules.slots[self.level - 1];
                let mut slots = std::mem::take(&mut self.slots);
                let mut items = slots.split_off(self.layout[0]);
                slots.resize(layout[0].max(slots.len()), None);
                items.resize(layout[1].max(items.len()), None);
                self.layout = [slots.len(), items.len()];
                slots.extend(items);
                self.slots = slots;
                self.dispatch_meta(Hook::LevelUp, None, account)?;
            }
            Action::Sell(uid) => {
                let loc = self.find(uid).ok_or(Error::BadTarget)?;
                if self.piece(loc).kind != Kind::Chess {
                    return Err(Error::BadTarget);
                }
                let mut piece = self.take(loc);
                self.remove_tokens(piece.uid);
                for item in piece.items.drain(..) {
                    self.stow(item)?;
                }
                self.return_copies(&piece);
                self.dispatch_meta(Hook::Sold, Some(piece.clone()), account)?;
                self.equipment_event(equipment_lifecycle::Event::Sold, account)?;
                account
                    .earn(self.rules.chess[&piece.key].sell)
                    .map_err(|_| Error::Capacity)?;
                self.counts.sells = add(self.counts.sells, 1).map_err(|_| Error::Capacity)?;
                self.round.sells = add(self.round.sells, 1).map_err(|_| Error::Capacity)?;
                self.check_item_merges(account)?;
                self.refill();
            }
            Action::Destroy(uid) => {
                let loc = self.find(uid).ok_or(Error::BadTarget)?;
                if self.piece(loc).kind != Kind::Item {
                    return Err(Error::BadTarget);
                }
                let piece = self.take(loc);
                self.release_item(&piece);
                self.destroy_art(&piece, account)?;
                self.refill();
            }
            Action::Move { uid, to } => self.move_piece(uid, to)?,
            Action::Equip {
                item,
                target,
                replace,
            } => self.equip(item, target, replace, account)?,
            Action::Unequip(_) => return Err(Error::BadTarget),
        }
        self.recompute_meta(account)
    }
    fn move_chess_or_item(&mut self, uid: u64, to: Destination) -> Result<(), Error> {
        let from = self.find(uid).ok_or(Error::BadTarget)?;
        match to {
            Destination::Hand(i) => {
                if i >= 10 {
                    return Err(Error::BadTarget);
                }
                if matches!(from,Location::Hand(j) if i==j) {
                    return Ok(());
                }
                if let (Location::Board(at), Some(other)) = (from, &self.hand[i]) {
                    if other.kind == Kind::Chess {
                        if !self.legal(other, at) {
                            return Err(Error::BadTile);
                        }
                    } else {
                        let free = self.free_hand().ok_or(Error::HandFull)?;
                        self.hand[free] = Some(self.take(from));
                        self.refill();
                        return Ok(());
                    }
                }
                let piece = self.take(from);
                if let Some(mut other) = self.hand[i].take() {
                    if matches!(from, Location::Board(_)) {
                        other.direction = Some(piece.direction.unwrap_or(Direction::Right));
                    }
                    self.put(from, other);
                }
                self.hand[i] = Some(piece);
            }
            Destination::Board { home, direction } => {
                if self.piece(from).kind != Kind::Chess {
                    return Err(Error::BadTarget);
                }
                if !self.legal(self.piece(from), home) {
                    return Err(Error::BadTile);
                }
                if let Location::Board(at) = from {
                    if at == home {
                        self.board.get_mut(&at).unwrap().direction = Some(direction);
                        return Ok(());
                    }
                    if let Some(other) = self.board.get(&home) {
                        if !(other.kind == Kind::Token && other.owner_uid == Some(uid))
                            && !self.legal(other, at)
                        {
                            return Err(Error::BadTile);
                        }
                    }
                } else if self.board.get(&home).is_none_or(|p| p.kind != Kind::Chess)
                    && self
                        .board
                        .values()
                        .filter(|p| p.kind == Kind::Chess)
                        .count()
                        >= self.deploy_cap
                {
                    return Err(Error::BoardFull);
                }
                let mut piece = self.take(from);
                if let Some(other) = self.remove_board(&home) {
                    if other.kind == Kind::Token && !matches!(from, Location::Board(_)) {
                        let prefer = match from {
                            Location::Hand(i) => Some(i),
                            _ => None,
                        };
                        self.return_token(other, prefer, true)?;
                    } else {
                        self.put(from, other);
                    }
                }
                piece.direction = Some(direction);
                self.insert_board(home, piece);
            }
        }
        Ok(())
    }
    pub fn has_effects(&self) -> bool {
        self.meta.is_some()
    }
    pub(crate) fn battle_map(&self, map: &mut crate::battle::runtime::MapConfig) {
        let Some(meta) = &self.meta else {
            return;
        };
        map.top_layers = meta.bonds.values().map(|b| b.layers).fold(0., f64::max);
        if map.cold_wind.is_none() {
            map.cold_wind = meta.cold_wind();
        }
    }
    pub(crate) fn battle_bonds(&self) -> Vec<crate::battle::runtime::BattleBond> {
        let Some(meta) = &self.meta else {
            return vec![];
        };
        meta.bonds
            .iter()
            .map(|(id, state)| crate::battle::runtime::BattleBond {
                id: id.clone(),
                tier: state.tier,
                layers: state.layers,
            })
            .collect()
    }
    pub(crate) fn settle_layers(
        &mut self,
        gains: &[(String, f64)],
        player: &mut Player,
    ) -> Result<(), String> {
        for (id, amount) in gains {
            self.add_layers(id, *amount, false, player)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn active_layers(&self) -> f64 {
        self.meta.as_ref().map_or(0., Meta::active_sum)
    }
    pub fn check_ready(&self) -> Result<(), String> {
        if self.personal_choice.is_some() {
            return Err("BAD_TARGET".into());
        }
        if self.temp.iter().any(Option::is_some) {
            return Err("TEMP_NOT_EMPTY".into());
        }
        Ok(())
    }
    pub fn deploy_cap(&self) -> usize {
        self.deploy_cap
    }
    pub fn gained_chess(&self) -> u64 {
        self.round.gained_chess
    }
    pub fn prepared(&self, active_layers: f64) -> PreparedRound {
        let operators = self
            .board_order()
            .into_iter()
            .filter(|(_, p)| p.kind == Kind::Chess)
            .map(|(home, p)| Operator {
                key: p.key.clone(),
                placement: Placement {
                    uid: p.uid,
                    home,
                    direction: p.direction.unwrap_or(Direction::Right),
                },
                skill_index: None,
                items: p.items.iter().map(|p| p.key.clone()).collect(),
            })
            .collect();
        PreparedRound {
            operators,
            active_layers,
        }
    }
    pub fn eliminate(&mut self) {
        for loc in self.locations() {
            let p = self.take(loc);
            for item in &p.items {
                self.release_item(item);
            }
            if p.kind == Kind::Chess {
                self.return_copies(&p);
            } else if p.kind == Kind::Item {
                self.release_item(&p);
            }
        }
        self.slots.clear();
        self.offers.clear();
        self.bounties.clear();
        self.personal_choice = None;
        let states = self.bond_states();
        if let Some(meta) = &mut self.meta {
            meta.bonds = states;
        }
    }
    fn piece_report(p: &Piece) -> Value {
        let mut out = json!({"uid":p.uid,"kind":p.kind.name(),"id":p.key,"poolCopies":p.copies,"count":p.count,"ownerUid":p.owner_uid,"dir":p.direction.map(Direction::name)});
        if !p.items.is_empty() {
            out["items"] = json!(p.items.iter().map(Self::piece_report).collect::<Vec<_>>());
        }
        out
    }
    pub fn report(&self, player: &Player) -> Value {
        let piece = Self::piece_report;
        let slot = |s: &Slot| json!({"kind":s.kind.name(),"id":s.key,"price":s.price,"frozen":s.frozen,"sold":s.sold});
        let board = self
            .board_order()
            .into_iter()
            .map(|(home, p)| json!({"row":home[0],"col":home[1],"piece":piece(p)}))
            .collect::<Vec<_>>();
        let mut report = json!({"deployCap":self.deploy_cap,"roundIncome":self.round_income,"rng":self.rng.state(),"uid":self.uid,"level":self.level,"upgradePrice":self.upgrade_price,"frozen":self.frozen,"slots":self.slots.iter().map(|s|s.as_ref().map(&slot)).collect::<Vec<_>>(),
            "hand":self.hand.iter().map(|p|p.as_ref().map(&piece)).collect::<Vec<_>>(),"temp":self.temp.iter().map(|p|p.as_ref().map(&piece)).collect::<Vec<_>>(),"board":board,
            "offers":self.offers.iter().map(|o|json!({"tier":o.tier,"slots":o.slots.iter().map(&slot).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "personalChoice":self.personal_choice.as_ref().map(bounties::Choice::report),"bounties":self.bounties.iter().map(bounties::Bounty::report).collect::<Vec<_>>(),
            "pool":self.pool.iter().map(|(key,e)|(key.clone(),json!({"cap":e.cap,"left":e.left.max(0),"rawLeft":e.left}))).collect::<BTreeMap<_,_>>(),"itemDebt":self.debt,
            "stats":{"buys":self.counts.buys,"sells":self.counts.sells,"refreshes":self.counts.refreshes,"merges":self.counts.merges,"itemMerges":self.counts.item_merges,"itemsEquipped":self.counts.items_equipped},
            "round":{"buys":self.round.buys,"sells":self.round.sells,"refreshes":self.round.refreshes,"spent":player.round_spent,"gainedChess":self.round.gained_chess,"arts":self.round.arts}});
        if self.draft_effects.sequence > 0 {
            report["draftEffects"] = json!({"sequence":self.draft_effects.sequence,"upgrades":self.draft_effects.upgrades.iter().map(|u|json!({"id":u.id,"kind":u.kind.name(),"left":u.left})).collect::<Vec<_>>()});
        }
        if !self.draft_effects.battle.is_empty() {
            report["draftEffects"]["battle"] = json!(self
                .draft_effects
                .battle
                .iter()
                .map(|(id, seq)| json!({"effectId":id,"sequence":seq}))
                .collect::<Vec<_>>());
        }
        if let Some(meta) = &self.meta {
            report["effects"] = meta.report(self.free_refreshes);
        }
        report
    }
}
