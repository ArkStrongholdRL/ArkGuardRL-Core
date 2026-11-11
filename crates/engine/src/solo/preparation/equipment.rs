// SPDX-License-Identifier: GPL-3.0-or-later
use super::{add, Error, Kind, Location, Piece, Player, Preparation};

pub(super) enum ItemLocation {
    Loose(Location),
    Equipped { holder: u64, uid: u64 },
}
impl Preparation {
    pub(super) fn chess_locations(&self) -> Vec<Location> {
        let mut out: Vec<_> = self
            .board_order()
            .into_iter()
            .map(|(at, _)| Location::Board(at))
            .collect();
        out.extend(self.hand.iter().enumerate().filter_map(|(i, p)| {
            p.as_ref()
                .filter(|p| p.kind == Kind::Chess)
                .map(|_| Location::Hand(i))
        }));
        out.extend(self.temp.iter().enumerate().filter_map(|(i, p)| {
            p.as_ref()
                .filter(|p| p.kind == Kind::Chess)
                .map(|_| Location::Temp(i))
        }));
        out
    }
    pub(super) fn item_locations(&self, key: &str) -> Vec<ItemLocation> {
        let mut out: Vec<_> = self
            .locations()
            .into_iter()
            .filter(|loc| {
                let p = self.piece(*loc);
                p.kind == Kind::Item && p.key == key
            })
            .map(ItemLocation::Loose)
            .collect();
        for loc in self.chess_locations() {
            let holder = self.piece(loc);
            out.extend(holder.items.iter().filter(|p| p.key == key).map(|p| {
                ItemLocation::Equipped {
                    holder: holder.uid,
                    uid: p.uid,
                }
            }));
        }
        out
    }
    pub(super) fn take_item(&mut self, loc: ItemLocation) -> Piece {
        match loc {
            ItemLocation::Loose(loc) => self.take(loc),
            ItemLocation::Equipped { holder, uid } => {
                let loc = self.find(holder).unwrap();
                let p = self.piece_mut(loc);
                let at = p.items.iter().position(|p| p.uid == uid).unwrap();
                p.items.remove(at)
            }
        }
    }
    pub(super) fn equip(
        &mut self,
        item: u64,
        target: u64,
        replace: Option<u64>,
        player: &mut Player,
    ) -> Result<(), Error> {
        let from = self.find(item).ok_or(Error::BadTarget)?;
        let p = self.piece(from);
        if p.kind != Kind::Item
            || self.rules.data.table("items").unwrap()[&p.key]["itemType"] != "EQUIP"
        {
            return Err(Error::BadTarget);
        }
        let key = p.key.clone();
        let to = self.find(target).ok_or(Error::BadTarget)?;
        let owner = self.piece(to);
        if owner.kind != Kind::Chess || !self.rules.bench_equip && !matches!(to, Location::Board(_))
        {
            return Err(Error::BadTarget);
        }
        if replace.is_some_and(|uid| !owner.items.iter().any(|p| p.uid == uid)) {
            return Err(Error::BadTarget);
        }
        if let Some(effect) = super::consumables::compile(&self.rules.data, &key)? {
            return self.consume_equipment(item, target, replace, effect, player);
        }
        crate::battle::equipment::compile(&self.rules.data, &key).map_err(Error::Content)?;
        let item = self.take(from);
        let need = self.rules.items[&key].merge;
        if need > 1 && self.item_locations(&key).len() + 1 >= need {
            self.merge_items(&key, Some(item), player)?;
        } else {
            // Detaching the incoming item does not move its carrier.
            let to = self.find(target).unwrap();
            if self.piece(to).items.len() >= self.rules.equip_cap {
                let at = replace.map_or(0, |uid| {
                    self.piece(to)
                        .items
                        .iter()
                        .position(|p| p.uid == uid)
                        .unwrap()
                });
                let old = self.piece_mut(to).items.remove(at);
                self.release_item(&old);
            }
            self.piece_mut(to).items.push(item);
            self.counts.items_equipped =
                add(self.counts.items_equipped, 1).map_err(|_| Error::Capacity)?;
            self.check_item_merges(player)?;
        }
        self.refill();
        Ok(())
    }
}

impl Preparation {
    pub(super) fn cauldron_gain(&mut self, player: &mut Player) -> Result<(), Error> {
        let mut live = std::collections::BTreeSet::new();
        let mut rewards = Vec::new();
        let table = self.rules.data.table("items").unwrap();
        for location in self.locations() {
            let holder = self.piece(location);
            if holder.kind != Kind::Chess {
                continue;
            }
            for item in &holder.items {
                let Some(buffs) = table[&item.key]["buffs"].as_array() else {
                    continue;
                };
                for buff in buffs {
                    if buff["key"] != "equip_with_another_gain_coin_when_gain_char" {
                        continue;
                    }
                    live.insert(item.uid);
                    let bond = buff["bbStr"]["bond"].as_str().unwrap_or("yanShip");
                    if !self.piece_bonds(holder).iter().any(|id| id == bond) {
                        continue;
                    }
                    let partners = buff["bbStr"]["other_equip"].as_str().unwrap_or("");
                    let family = |id: &str| {
                        id.strip_suffix("_a")
                            .or_else(|| id.strip_suffix("_b"))
                            .unwrap_or(id)
                            .to_owned()
                    };
                    if !holder.items.iter().any(|other| {
                        other.uid != item.uid
                            && partners.split(',').any(|id| {
                                !id.trim().is_empty() && family(id.trim()) == family(&other.key)
                            })
                    }) {
                        continue;
                    }
                    let n = |key| {
                        super::effects::n(buff["bbStr"].get(key).unwrap_or(&buff["bb"][key]), 0.)
                    };
                    let cap = n("max").max(0.).trunc() as u32;
                    if self.round.cauldron.get(&item.uid).copied().unwrap_or(0) < cap {
                        rewards.push((item.uid, n("count").max(0.).trunc() as u64));
                    }
                }
            }
        }
        self.round.cauldron.retain(|uid, _| live.contains(uid));
        for (uid, amount) in rewards {
            *self.round.cauldron.entry(uid).or_default() += 1;
            player.earn(amount).map_err(|_| Error::Capacity)?;
        }
        Ok(())
    }
}
