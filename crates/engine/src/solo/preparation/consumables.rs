// SPDX-License-Identifier: GPL-3.0-or-later
//! Consume-on-equip actions. The outer preparation draft makes refusals atomic.
use super::*;
use effects::n;

pub(super) enum Effect {
    Coins { min: u32, max: u32 },
    Layers(f64),
    Shop(u32),
    Recruit(u32),
    Income(u64),
    Pending(u64),
    Deployment(usize),
    Offer { count: u32, sacrifice: bool },
    Mimic,
    Promote { delayed: bool },
}
pub(super) fn compile(data: &Catalog, key: &str) -> Result<Option<Effect>, Error> {
    let raw = &data.table("items").unwrap()[key];
    if !matches!(
        raw["kind"].as_str(),
        Some("consume_on_equip" | "consume_on_equip_or_delayed")
    ) {
        return Ok(None);
    }
    let buffs = raw["buffs"]
        .as_array()
        .filter(|a| a.len() == 1)
        .ok_or_else(|| Error::Content(format!("Unsupported consumable: {key}")))?;
    let b = &buffs[0];
    let value = |name: &str| n(b["bbStr"].get(name).unwrap_or(&b["bb"][name]), 0.);
    let count = |name: &str| -> Result<u32, Error> {
        let v = value(name).trunc();
        if !(0. ..=u32::MAX as f64).contains(&v) {
            return Err(Error::Content(format!(
                "Invalid consumable count: {key}/{name}"
            )));
        }
        Ok(v as u32)
    };
    let effect = match b["key"].as_str().unwrap_or("") {
        "equip_destory_gain_random_coin" => {
            let min = count("min")?;
            let max = count("max")?.max(min);
            if max - min == u32::MAX {
                return Err(Error::Capacity);
            }
            Effect::Coins { min, max }
        }
        "use_equip_reward_char_chess_bond_layer" => Effect::Layers(value("layer")),
        "use_equip_reward_random_char_chess_in_shop" => Effect::Shop(count("count")?.max(1)),
        "use_equip_reward_char_chess_with_same_bond" => Effect::Recruit(count("count")?.max(1)),
        "gain_coin_when_round_start" => Effect::Income(count("count")? as u64),
        "use_equip_gain_coin_when_next_round_start" => Effect::Pending(count("count")? as u64),
        "equip_destory_deployment_cnt_change" => {
            Effect::Deployment(count("count")?.min(12) as usize)
        }
        "use_equip_reward_special_goods_char_chess" => Effect::Offer {
            count: count("refresh_cnt")?.max(1),
            sacrifice: false,
        },
        "use_equip_recruit_new_char_and_give_char_to_player_most_bond" => Effect::Offer {
            count: count("refresh_cnt")?.max(1),
            sacrifice: true,
        },
        "use_equip_reward_char_chess" => Effect::Mimic,
        "use_equip_upgrade_char" => Effect::Promote { delayed: false },
        "equip_round_start_upgrade_char" => Effect::Promote { delayed: true },
        _ => return Err(Error::Content(format!("Unsupported consumable: {key}"))),
    };
    Ok(Some(effect))
}
impl Preparation {
    pub(super) fn consume_equipment(
        &mut self,
        uid: u64,
        target: u64,
        replace: Option<u64>,
        effect: Effect,
        player: &mut Player,
    ) -> Result<(), Error> {
        if self.meta.is_none() {
            return Err(Error::Content(
                "Consumables require preparation effects".into(),
            ));
        }
        if matches!(effect, Effect::Shop(n) | Effect::Recruit(n) | Effect::Offer { count: n, .. } if n as usize > self.capacity)
        {
            return Err(Error::Capacity);
        }
        if matches!(effect, Effect::Promote { .. })
            && self.rules.data.table("chess").unwrap()[&self.piece(self.find(target).unwrap()).key]
                ["isGolden"]
                == true
        {
            return Err(Error::BadTarget);
        }
        let keep = matches!(effect, Effect::Promote { delayed: true });
        let item = self.piece(self.find(uid).unwrap()).clone();
        let loc = self.find(target).unwrap();
        let cap = self.rules.equip_cap;
        let holder = self.piece_mut(loc);
        let off = if holder.items.len() >= cap {
            let at = replace.map_or(0, |id| {
                holder.items.iter().position(|p| p.uid == id).unwrap()
            });
            Some(holder.items.remove(at))
        } else {
            None
        };
        // The removed slot no longer contributes identity to the effect.
        let holder = self.piece(self.find(target).unwrap()).clone();
        let bonds = self.piece_bonds(&holder);
        match effect {
            Effect::Promote { delayed } => {
                if !delayed {
                    self.promote_equipped_holder(target)?;
                }
            }
            Effect::Offer { count, sacrifice } => {
                let tier = self.rules.chess[&holder.key].tier;
                if sacrifice {
                    let mut removed = self.take(self.find(target).unwrap());
                    self.remove_tokens(removed.uid);
                    for equipped in removed.items.drain(..) {
                        self.stow(equipped)?;
                    }
                    self.return_copies(&removed);
                    self.check_item_merges(player)?;
                    self.recompute_meta(player)?;
                }
                let mut keys = Vec::new();
                for _ in 0..count {
                    let picked = if sacrifice {
                        (1..=tier)
                            .rev()
                            .find_map(|t| self.roll_meta_chess(6, 1, Some(t), None, &keys))
                    } else {
                        self.roll_meta_chess(self.level, 1, None, Some(&bonds), &keys)
                    };
                    if let Some(key) = picked {
                        keys.push(key);
                    }
                }
                if !keys.is_empty() {
                    if self.offers.len() >= self.capacity {
                        return Err(Error::Capacity);
                    }
                    self.offers.push_back(Offer {
                        tier: if sacrifice {
                            tier
                        } else {
                            (self.level as i64 + self.rules.reward_offset)
                                .clamp(0, self.rules.reward_max_tier as i64)
                                as usize
                        },
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
            Effect::Mimic => {
                let base = self.rules.chess[&holder.key].base.clone();
                let owned = self
                    .chess_locations()
                    .into_iter()
                    .filter(|loc| self.piece(*loc).key == base)
                    .count();
                let key = if owned >= 2 {
                    Some(base)
                } else {
                    let eligible: Vec<_> = self
                        .pool
                        .keys()
                        .filter(|key| self.own_bonds(key).iter().any(|b| bonds.contains(b)))
                        .cloned()
                        .collect();
                    let weights = vec![1.; eligible.len()];
                    self.meta
                        .as_mut()
                        .unwrap()
                        .rng
                        .weighted(&weights)
                        .map(|i| eligible[i].clone())
                };
                if let Some(key) = key {
                    self.gain(Kind::Chess, &key, GainSource::Grant, player)?;
                }
            }
            Effect::Coins { min, max } => {
                let amount = min + self.meta.as_mut().unwrap().rng.int(max - min + 1);
                player.earn(amount as u64).map_err(|_| Error::Capacity)?;
            }
            Effect::Layers(amount) => {
                for bond in bonds {
                    self.add_layers(&bond, amount, false, player)?;
                }
            }
            Effect::Shop(count) => {
                let mut tried = BTreeSet::new();
                let mut granted = 0;
                while granted < count {
                    let choices: Vec<_> = self
                        .slots
                        .iter()
                        .enumerate()
                        .filter(|(i, slot)| {
                            !tried.contains(i)
                                && slot
                                    .as_ref()
                                    .is_some_and(|s| s.kind == Kind::Chess && !s.sold)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    let Some(index) = self.meta.as_mut().unwrap().rng.pick(&choices).copied()
                    else {
                        break;
                    };
                    let key = self.slots[index].as_ref().unwrap().key.clone();
                    if self.grant_recruit(&key, player)? {
                        self.slots[index] = None;
                        granted += 1;
                    } else {
                        tried.insert(index);
                    }
                }
            }
            Effect::Recruit(count) => {
                for _ in 0..count {
                    if let Some(key) =
                        self.roll_meta_chess(self.level.clamp(1, 6), 1, None, Some(&bonds), &[])
                    {
                        self.grant_recruit(&key, player)?;
                    }
                }
            }
            Effect::Income(amount) => {
                self.round_income = add(self.round_income, amount).map_err(|_| Error::Capacity)?
            }
            Effect::Pending(amount) => {
                player.pending = add(player.pending, amount).map_err(|_| Error::Capacity)?
            }
            Effect::Deployment(min) => {
                self.deploy_min = self.deploy_min.max(min);
                self.deploy_cap = self.deploy_cap.max(self.deploy_min);
            }
        }
        if let Some(off) = off {
            self.release_item(&off);
        }
        if let Some(loc) = self.find(uid) {
            self.take(loc);
        }
        if keep {
            self.piece_mut(self.find(target).unwrap()).items.push(item);
        } else {
            self.release_item(&item);
        }
        self.counts.items_equipped =
            add(self.counts.items_equipped, 1).map_err(|_| Error::Capacity)?;
        self.check_item_merges(player)?;
        self.refill();
        Ok(())
    }
}
