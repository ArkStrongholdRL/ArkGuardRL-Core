// SPDX-License-Identifier: GPL-3.0-or-later
//! Placeable summon inventory, bound to the deployed owner's UID.
use super::*;

pub(in crate::solo) struct PreparedToken {
    pub key: String,
    pub placement: Placement,
    pub owner_uid: u64,
}

impl Preparation {
    pub(super) fn owner_positions(&self) -> Vec<([i32; 2], u64)> {
        self.board_insertion
            .iter()
            .map(|at| (*at, &self.board[at]))
            .filter(|(_, p)| p.kind == Kind::Chess)
            .map(|(at, p)| (at, p.uid))
            .collect()
    }
    fn token_stack(&self, key: &str, owner_uid: Option<u64>) -> Option<Location> {
        let matches = |p: &Piece| p.kind == Kind::Token && p.key == key && p.owner_uid == owner_uid;
        self.hand
            .iter()
            .position(|p| p.as_ref().is_some_and(matches))
            .map(Location::Hand)
            .or_else(|| {
                self.temp
                    .iter()
                    .position(|p| p.as_ref().is_some_and(matches))
                    .map(Location::Temp)
            })
    }
    pub(super) fn grant_tokens(&mut self, owner_uid: u64) -> Result<(), Error> {
        let Some(owner) = self
            .board
            .values()
            .find(|p| p.uid == owner_uid && p.kind == Kind::Chess)
        else {
            return Ok(());
        };
        let data = &self.rules.data;
        let rec = &data.table("chess").unwrap()[&owner.key];
        let mut stock = Vec::new();
        for key in rec["tokens"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let token = &data.table("tokens").unwrap()[key];
            if token["kind"] != "summon" || token["placeable"] != true {
                continue;
            }
            let base = self.rules.chess[&owner.key].base.as_str();
            let variant = token["variants"]
                .get(&owner.key)
                .or_else(|| token["variants"].get(base));
            if variant.is_some_and(|v| {
                !v["sources"]
                    .as_array()
                    .is_some_and(|sources| sources.iter().any(|s| s == "talent" || s == "skill"))
            }) {
                continue;
            }
            let count = variant
                .and_then(|v| v["stats"]["deployLimit"].as_u64())
                .filter(|n| *n > 0)
                .or_else(|| token["deployLimit"].as_u64().filter(|n| *n > 0))
                .unwrap_or(1)
                .min(9);
            stock.push((key.to_owned(), count));
        }
        for (key, count) in stock {
            let held: u64 = self
                .pieces()
                .filter(|p| p.kind == Kind::Token && p.key == key && p.owner_uid == Some(owner_uid))
                .map(|p| p.count)
                .sum();
            if held >= count {
                continue;
            }
            if let Some(loc) = self.token_stack(&key, Some(owner_uid)) {
                self.piece_mut(loc).count += count - held;
            } else {
                if self.len() >= self.capacity {
                    return Err(Error::Capacity);
                }
                let mut token = self.new_piece(Kind::Token, &key, 0)?;
                token.count = count - held;
                token.owner_uid = Some(owner_uid);
                self.stow(token)?;
            }
        }
        Ok(())
    }
    pub(super) fn remove_tokens(&mut self, owner_uid: u64) {
        let locations: Vec<_> = self
            .locations()
            .into_iter()
            .filter(|&loc| {
                let p = self.piece(loc);
                p.kind == Kind::Token && p.owner_uid == Some(owner_uid)
            })
            .collect();
        for loc in locations {
            self.take(loc);
        }
    }
    pub(super) fn return_token(
        &mut self,
        mut token: Piece,
        prefer: Option<usize>,
        allow_temp: bool,
    ) -> Result<(), Error> {
        token.direction = None;
        if let Some(loc) = self.token_stack(&token.key, token.owner_uid) {
            self.piece_mut(loc).count =
                add(self.piece(loc).count, token.count).map_err(|_| Error::Capacity)?;
        } else if let Some(i) = prefer.filter(|&i| self.hand[i].is_none()) {
            self.hand[i] = Some(token);
        } else if allow_temp || self.free_hand().is_some() {
            self.stow(token)?;
        } else {
            return Err(Error::HandFull);
        }
        Ok(())
    }
    pub(super) fn token_legal(&self, piece: &Piece, home: [i32; 2]) -> bool {
        let token = &self.rules.data.table("tokens").unwrap()[&piece.key];
        if !self.grid.can_place(token, home) {
            return false;
        }
        let Some((at, owner)) = self
            .board
            .iter()
            .find(|(_, p)| p.kind == Kind::Chess && Some(p.uid) == piece.owner_uid)
        else {
            return false;
        };
        let fixed = piece.key == "token_10032_jesca2_jckshd";
        if token["ownerRange"] != true && token["ownerRangeOutside"] != true && !fixed {
            return true;
        }
        let offsets: Vec<[i32; 2]> = if fixed {
            vec![[0, 1], [1, 0], [0, -1], [-1, 0]]
        } else {
            let raw = &self.rules.data.table("chess").unwrap()[&owner.key]["rangeGrid"];
            match serde_json::from_value(raw.clone()) {
                Ok(grid) => grid,
                Err(_) => return false,
            }
        };
        let keys = crate::battle::targeting::absolute_range(
            &offsets,
            *at,
            owner.direction.unwrap_or(Direction::Right),
            0,
        );
        let inside =
            keys.contains(&(home[0] as usize * crate::battle::grid::COLS + home[1] as usize));
        if token["ownerRangeOutside"] == true {
            !inside
        } else {
            inside
        }
    }
    fn move_token(&mut self, from: Location, to: Destination) -> Result<(), Error> {
        match to {
            Destination::Hand(i) => {
                if i >= self.hand.len() {
                    return Err(Error::BadTarget);
                }
                if matches!(from, Location::Board(_)) {
                    let token = self.take(from);
                    self.return_token(token, Some(i), false)?;
                } else {
                    let token = self.take(from);
                    if let Some(other) = self.hand[i].take() {
                        self.put(from, other);
                    }
                    self.hand[i] = Some(token);
                }
            }
            Destination::Board { home, direction } => {
                let source = self.piece(from).clone();
                if let Location::Board(old) = from {
                    if old == home {
                        if !self.legal(&source, home) {
                            return Err(Error::BadTile);
                        }
                        self.board.get_mut(&home).unwrap().direction = Some(direction);
                        return Ok(());
                    }
                    let mut token = self.take(from);
                    let other = self.remove_board(&home);
                    if let Some(other) = other {
                        self.insert_board(old, other);
                    }
                    token.direction = Some(direction);
                    self.insert_board(home, token);
                    if !self.legal(&self.board[&home], home)
                        || self.board.get(&old).is_some_and(|p| !self.legal(p, old))
                    {
                        return Err(Error::BadTile);
                    }
                } else {
                    if self.board.contains_key(&home) || !self.legal(&source, home) {
                        return Err(Error::BadTile);
                    }
                    let mut token = if source.count > 1 {
                        if self.len() >= self.capacity {
                            return Err(Error::Capacity);
                        }
                        self.piece_mut(from).count -= 1;
                        let mut token = self.new_piece(Kind::Token, &source.key, 0)?;
                        token.owner_uid = source.owner_uid;
                        token
                    } else {
                        self.take(from)
                    };
                    token.direction = Some(direction);
                    self.insert_board(home, token);
                }
            }
        }
        Ok(())
    }
    pub(super) fn move_piece(&mut self, uid: u64, to: Destination) -> Result<(), Error> {
        let from = self.find(uid).ok_or(Error::BadTarget)?;
        let mut old_owners = self.owner_positions();
        old_owners.sort_by_key(|(_, owner)| *owner == uid);
        let board_swap_order = match (from, to) {
            (Location::Board(old), Destination::Board { home, .. })
                if old != home && self.board.contains_key(&home) =>
            {
                Some(self.board_insertion.clone())
            }
            _ => None,
        };
        let token = self.piece(from).kind == Kind::Token;
        if token {
            self.move_token(from, to)?;
        } else {
            if matches!(from, Location::Board(_)) && matches!(to, Destination::Hand(_)) {
                self.remove_tokens(uid);
            }
            self.move_chess_or_item(uid, to)?;
        }
        if let Some(order) = board_swap_order {
            self.board_insertion = order;
        }
        for (old, owner) in old_owners {
            let now = self
                .board
                .iter()
                .find(|(_, p)| p.uid == owner)
                .map(|(at, _)| *at);
            if now.is_none() {
                self.remove_tokens(owner);
            } else if now != Some(old) {
                let lifted: Vec<_> = self
                    .board_insertion
                    .iter()
                    .copied()
                    .filter(|at| {
                        let p = &self.board[at];
                        p.kind == Kind::Token
                            && p.owner_uid == Some(owner)
                            && !(token && p.uid == uid)
                    })
                    .collect();
                for at in lifted {
                    let p = self.remove_board(&at).unwrap();
                    self.return_token(p, None, true)?;
                }
            }
        }
        let illegal: Vec<_> = self
            .board
            .iter()
            .filter(|(at, p)| p.kind == Kind::Token && !self.legal(p, **at))
            .map(|(at, _)| *at)
            .collect();
        if matches!((from, to), (Location::Board(old), Destination::Board { home, .. }) if old == home)
        {
            let needs_slot: BTreeSet<_> = illegal
                .iter()
                .map(|at| &self.board[at])
                .filter(|p| self.token_stack(&p.key, p.owner_uid).is_none())
                .map(|p| (&p.key, p.owner_uid))
                .collect();
            let available = self
                .hand
                .iter()
                .chain(&self.temp)
                .filter(|p| p.is_none())
                .count();
            if needs_slot.len() > available {
                return Err(Error::HandFull);
            }
        }
        for at in illegal {
            let p = self.remove_board(&at).unwrap();
            self.return_token(p, None, true)?;
        }
        for (_, owner) in self.owner_positions() {
            self.grant_tokens(owner)?;
        }
        self.refill();
        Ok(())
    }
    pub(in crate::solo) fn prepared_tokens(&self) -> Vec<PreparedToken> {
        self.board_order()
            .into_iter()
            .filter(|(_, p)| p.kind == Kind::Token)
            .map(|(home, p)| PreparedToken {
                key: p.key.clone(),
                owner_uid: p.owner_uid.unwrap(),
                placement: Placement {
                    uid: p.uid,
                    home,
                    direction: p.direction.unwrap_or(Direction::Right),
                },
            })
            .collect()
    }
}
