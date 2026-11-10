// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    Board,
    Hand,
    Temp,
    Gain,
    Sold,
    Trigger,
}
impl Origin {
    fn at(loc: Location) -> Self {
        match loc {
            Location::Board(_) => Self::Board,
            Location::Hand(_) => Self::Hand,
            Location::Temp(_) => Self::Temp,
        }
    }
}
#[derive(Clone)]
pub(super) struct Call {
    pub piece: Piece,
    pub copied_from: Option<Piece>,
    pub origin: Origin,
    pub triggered: bool,
}
#[derive(Clone)]
pub(super) struct Refresh {
    witnesses: Vec<(Piece, Origin)>,
    pub merged: BTreeMap<u64, Piece>,
}
impl Preparation {
    pub(super) fn board_pieces(&self) -> Vec<([i32; 2], Piece)> {
        let mut board: Vec<_> = self.board.iter().map(|(at, p)| (*at, p.clone())).collect();
        board.sort_by_key(|(at, _)| (-at[0], at[1]));
        board
    }
    pub(super) fn garrison_ids(&self, key: &str) -> Vec<String> {
        self.rules.data.table("chess").unwrap()[key]["garrisonIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }
    pub(super) fn has_hook(&self, piece: &Piece, hook: &Hook) -> bool {
        let rules = &self.meta.as_ref().unwrap().rules.garrisons;
        self.garrison_ids(&piece.key)
            .iter()
            .any(|id| rules.get(id).is_some_and(|r| r.hook == *hook))
    }
    pub(super) fn front(&self, piece: &Piece, distance: i32) -> Option<Piece> {
        let Location::Board(at) = self.find(piece.uid)? else {
            return None;
        };
        let [r, c] = piece
            .direction
            .unwrap_or(crate::battle::direction::Direction::Right)
            .vector();
        self.board
            .get(&[at[0] + distance * r, at[1] + distance * c])
            .filter(|p| p.kind == Kind::Chess && p.uid != piece.uid)
            .cloned()
    }
    pub(super) fn row_condition(&self, minimum: Option<f64>, call: &Call, hook: &Hook) -> bool {
        let Some(minimum) = minimum else { return true };
        let chess = self.rules.data.table("chess").unwrap();
        let copied_exception = *hook == Hook::PrepStart
            && chess[&call.piece.key]["charId"] == "char_128_plosis"
            && call.copied_from.as_ref().is_some_and(|p| {
                matches!(
                    chess[&p.key]["charId"].as_str(),
                    Some("char_474_glady" | "char_2026_yu")
                )
            });
        copied_exception || self.same_row(&call.piece) as f64 >= minimum
    }
    pub(in crate::solo::preparation) fn record_refresh_merge(&mut self, piece: &Piece) {
        if let Some(refresh) = self.meta.as_mut().and_then(|m| m.refresh.as_mut()) {
            if refresh.witnesses.iter().any(|(p, _)| p.uid == piece.uid) {
                refresh.merged.insert(piece.uid, piece.clone());
            }
        }
    }
    fn gain_repeat(&self, hook: &Hook) -> Result<usize, Error> {
        let meta = self.meta.as_ref().unwrap();
        if *hook != Hook::Gain || !meta.active("investShip") {
            return Ok(1);
        }
        let bond = meta
            .rules
            .bonds
            .iter()
            .find(|b| b.id == "investShip")
            .unwrap();
        let count = n(&bond.raw["bb"]["count"], 2.);
        let base = if count > 1. && count.fract() == 0. {
            count as usize
        } else {
            2
        };
        let threshold = bond.raw["layerMilestones"]
            .as_array()
            .and_then(|a| {
                a.iter().find_map(|m| {
                    let v = n(&m["layer"], 0.);
                    (v > 0.).then_some(v)
                })
            })
            .unwrap_or_else(|| {
                let v = n(&bond.raw["bb"]["layer"], 100.);
                if v > 0. {
                    v
                } else {
                    100.
                }
            });
        let repeat = base
            .checked_add(usize::from(meta.bonds["investShip"].layers >= threshold))
            .ok_or(Error::Capacity)?;
        if repeat > 4096 {
            return Err(Error::Capacity);
        }
        Ok(repeat)
    }
    fn run_garrisons(
        &mut self,
        hook: &Hook,
        call: &Call,
        player: &mut Player,
    ) -> Result<(), Error> {
        let donor = call.copied_from.as_ref().unwrap_or(&call.piece);
        let ids = self.garrison_ids(&donor.key);
        let mut call = call.clone();
        for id in ids {
            if call.triggered {
                call.origin = self
                    .find(call.piece.uid)
                    .map_or(Origin::Trigger, Origin::at);
            }
            let Some(mut rule) = self
                .meta
                .as_ref()
                .unwrap()
                .rules
                .garrisons
                .get(&id)
                .filter(|r| r.hook == *hook)
                .cloned()
            else {
                continue;
            };
            if !call.triggered && call.origin == Origin::Hand && rule.board_only {
                continue;
            }
            if let Effect::Layers {
                targets: Targets::Named(targets),
                formula: Formula::Fixed(_),
                ..
            } = &mut rule.effect
            {
                let table = self.rules.data.table("garrisons").unwrap();
                let raw = &table[&id];
                for sibling in self.garrison_ids(&call.piece.key) {
                    let other = &table[&sibling];
                    if sibling != id
                        && other["effectType"] == "SERVER_ADD_MULTIPLE_BOND"
                        && other["eventType"] == raw["eventType"]
                        && other["desc"] == raw["desc"]
                    {
                        let covered = ids_of_bonds(&other["bbStr"]["bond"]);
                        targets.retain(|bond| !covered.contains(bond));
                    }
                }
            }
            for _ in 0..self.gain_repeat(hook)? {
                self.run_rule(&rule, &call, player)?;
            }
        }
        Ok(())
    }
    pub(super) fn trigger_garrisons(
        &mut self,
        donor: &Piece,
        receiver: Option<&Piece>,
        hook: &Hook,
        player: &mut Player,
    ) -> Result<(), Error> {
        if *hook == Hook::Price || self.meta.as_ref().unwrap().depth >= 6 {
            return Ok(());
        }
        // Trigger only garrisons. Global/bond callbacks belong to the enclosing event.
        let receiver = receiver.unwrap_or(donor);
        if self.find(donor.uid).is_none() || self.find(receiver.uid).is_none() {
            return Ok(());
        }
        let call = Call {
            piece: receiver.clone(),
            copied_from: (receiver.uid != donor.uid).then(|| donor.clone()),
            origin: self.find(receiver.uid).map_or(Origin::Trigger, Origin::at),
            triggered: true,
        };
        self.meta.as_mut().unwrap().depth += 1;
        let result = self.run_garrisons(hook, &call, player);
        self.meta.as_mut().unwrap().depth -= 1;
        result
    }
    fn owned_garrisons(&mut self, hook: &Hook, player: &mut Player) -> Result<(), Error> {
        if *hook == Hook::Refresh {
            let witnesses = self
                .meta
                .as_ref()
                .unwrap()
                .refresh
                .as_ref()
                .unwrap()
                .witnesses
                .clone();
            for (witness, original) in witnesses {
                let live = self
                    .find(witness.uid)
                    .filter(|loc| !matches!(loc, Location::Temp(_)))
                    .map(|loc| (self.piece(loc).clone(), Origin::at(loc)));
                let merged = self
                    .meta
                    .as_ref()
                    .unwrap()
                    .refresh
                    .as_ref()
                    .unwrap()
                    .merged
                    .get(&witness.uid)
                    .cloned()
                    .map(|p| (p, original));
                if let Some((piece, origin)) = live.or(merged) {
                    self.run_garrisons(
                        hook,
                        &Call {
                            piece,
                            copied_from: None,
                            origin,
                            triggered: false,
                        },
                        player,
                    )?;
                }
            }
        } else {
            // Prep events snapshot the board, then walk the current hand slots.
            for (_, piece) in self.board_pieces() {
                self.run_garrisons(
                    hook,
                    &Call {
                        piece,
                        copied_from: None,
                        origin: Origin::Board,
                        triggered: false,
                    },
                    player,
                )?;
            }
            for i in 0..self.hand.len() {
                if let Some(piece) = self.hand[i]
                    .as_ref()
                    .filter(|p| p.kind == Kind::Chess)
                    .cloned()
                {
                    self.run_garrisons(
                        hook,
                        &Call {
                            piece,
                            copied_from: None,
                            origin: Origin::Hand,
                            triggered: false,
                        },
                        player,
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::solo::preparation) fn dispatch_meta(
        &mut self,
        hook: Hook,
        piece: Option<Piece>,
        player: &mut Player,
    ) -> Result<(), Error> {
        let Some(meta) = &mut self.meta else {
            return Ok(());
        };
        if meta.depth >= 6 {
            return Ok(());
        }
        meta.depth += 1;
        if hook == Hook::PrepEnd {
            meta.ended_round = Some(meta.round);
            meta.defer_items += 1;
        }
        if hook == Hook::Refresh {
            let witnesses = self
                .board_pieces()
                .into_iter()
                .map(|(_, p)| (p, Origin::Board))
                .chain(
                    self.hand
                        .iter()
                        .flatten()
                        .filter(|p| p.kind == Kind::Chess)
                        .cloned()
                        .map(|p| (p, Origin::Hand)),
                )
                .collect();
            self.meta.as_mut().unwrap().refresh = Some(Refresh {
                witnesses,
                merged: BTreeMap::new(),
            });
        }
        let result = (|| {
            self.settle_meta_milestones(&hook, player)?;
            if hook == Hook::PrepEnd {
                let meta = self.meta.as_ref().unwrap();
                if let Some(state) = meta.bonds.get("deputShip").filter(|b| b.active()) {
                    let buff = meta.buff("deputShip", "bond_activated_add_layer");
                    let amount = if state.tier >= 2 {
                        n(&buff["more_layer"], n(&buff["layer"], 0.))
                    } else {
                        n(&buff["layer"], 0.)
                    };
                    let active = meta
                        .rules
                        .bonds
                        .iter()
                        .filter(|b| meta.active(&b.id))
                        .map(|b| b.id.clone())
                        .collect::<Vec<_>>();
                    for id in active {
                        self.add_layers(&id, amount, true, player)?;
                    }
                }
            }
            if hook == Hook::Refresh && self.free_refreshes == 0 {
                let meta = self.meta.as_ref().unwrap();
                if meta.active("miraShip") {
                    let b = meta.buff("miraShip", "bond_refresh_shop_next_free");
                    let chance = (n(&b["baseprob"], 0.)
                        + n(&b["prob"], 0.) * meta.layers.get("miraShip").copied().unwrap_or(0.))
                    .clamp(0., 1.);
                    if self.meta.as_mut().unwrap().rng.next_f64() < chance {
                        self.free_refreshes =
                            add(self.free_refreshes, 1).map_err(|_| Error::Capacity)?;
                    }
                }
            }
            if matches!(hook, Hook::Gain | Hook::Sold) {
                if let Some(piece) = piece.filter(|p| p.kind == Kind::Chess) {
                    self.run_garrisons(
                        &hook,
                        &Call {
                            piece,
                            copied_from: None,
                            origin: if hook == Hook::Gain {
                                Origin::Gain
                            } else {
                                Origin::Sold
                            },
                            triggered: false,
                        },
                        player,
                    )?;
                }
            } else if matches!(hook, Hook::PrepStart | Hook::PrepEnd | Hook::Refresh) {
                self.owned_garrisons(&hook, player)?;
            }
            Ok(())
        })();
        let meta = self.meta.as_mut().unwrap();
        if hook == Hook::Refresh {
            meta.refresh = None;
        }
        if hook == Hook::PrepEnd {
            meta.defer_items -= 1;
        }
        meta.depth -= 1;
        result
    }
}
