// SPDX-License-Identifier: GPL-3.0-or-later
//! Carried preparation effects retain their state on the equipment itself.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Event {
    RoundStart,
    Sold,
    BattleResult,
}

impl Preparation {
    pub(super) fn promote_equipped_holder(&mut self, uid: u64) -> Result<(), Error> {
        let loc = self.find(uid).ok_or(Error::BadTarget)?;
        let p = self.piece(loc);
        if self.rules.data.table("chess").unwrap()[&p.key]["isGolden"] == true {
            return Ok(());
        }
        let card = &self.rules.chess[&p.key];
        let Some(key) = card.upgrade.clone() else {
            return Ok(());
        };
        let base = card.base.clone();
        let extra = self.rules.chess[&key].copies.saturating_sub(p.copies);
        let copies = self.stock_take(&base, extra);
        let p = self.piece_mut(loc);
        p.key = key;
        p.copies += copies;
        self.grant_tokens(uid)?;
        Ok(())
    }
    pub(in crate::solo) fn equipment_battle_result(
        &mut self,
        player: &mut Player,
    ) -> Result<(), Error> {
        self.equipment_event(Event::BattleResult, player)
    }
    pub(super) fn equipment_event(
        &mut self,
        event: Event,
        player: &mut Player,
    ) -> Result<(), Error> {
        if self.meta.is_none() {
            return Ok(());
        }
        let mut calls = Vec::new();
        for loc in self.chess_locations() {
            let holder = self.piece(loc);
            for item in &holder.items {
                let raw = &self.rules.data.table("items").unwrap()[&item.key];
                if let Some(buffs) = raw["buffs"].as_array() {
                    for buff in buffs {
                        let applies = matches!(
                            (event, buff["key"].as_str()),
                            (Event::RoundStart, Some("equip_round_start_upgrade_char"))
                                | (Event::Sold, Some("sell_char_count_gain_equip_owner_bond"))
                                | (Event::BattleResult, Some("char_chess_transformation_equip"))
                        );
                        if applies {
                            let count = effects::n(
                                buff["bbStr"].get("count").unwrap_or(&buff["bb"]["count"]),
                                8.,
                            )
                            .trunc()
                            .max(1.);
                            if count > u32::MAX as f64 {
                                return Err(Error::Capacity);
                            }
                            calls.push((holder.uid, item.uid, count as u32));
                        }
                    }
                }
            }
        }
        for (uid, item_uid, need) in calls {
            let Some(loc) = self.find(uid) else {
                continue;
            };
            let Some(at) = self.piece(loc).items.iter().position(|p| p.uid == item_uid) else {
                continue;
            };
            match event {
                Event::RoundStart => {
                    let item = self.piece_mut(loc).items.remove(at);
                    self.release_item(&item);
                    self.promote_equipped_holder(uid)?;
                }
                Event::Sold => {
                    let item = &mut self.piece_mut(loc).items[at];
                    item.sale_charge += 1;
                    if item.sale_charge < need {
                        continue;
                    }
                    item.sale_charge = 0;
                    let bonds = self.piece_bonds(self.piece(loc));
                    if let Some(key) = self.roll_meta_chess(self.level, 1, None, Some(&bonds), &[])
                    {
                        self.grant_recruit(&key, player)?;
                    }
                }
                Event::BattleResult => {
                    let tier = (self.rules.chess[&self.piece(loc).key].tier + 1).min(6);
                    let Some(key) = self.roll_meta_chess(6, 1, Some(tier), None, &[]) else {
                        continue;
                    };
                    let mut holder = self.take(loc);
                    self.remove_tokens(holder.uid);
                    self.return_copies(&holder);
                    for item in holder.items.drain(..) {
                        self.stow(item)?;
                    }
                    self.check_item_merges(player)?;
                    self.gain(Kind::Chess, &key, GainSource::Grant, player)?;
                }
            }
            self.recompute_meta(player)?;
        }
        Ok(())
    }
}
