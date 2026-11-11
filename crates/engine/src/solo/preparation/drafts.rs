// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{battle::runtime::MapConfig, solo::draft};
#[derive(Clone, Default)]
pub(super) struct DraftEffects {
    pub sequence: u64,
    pub upgrades: Vec<Upgrade>,
    pub battle: Vec<(String, u64)>,
    pub maps: Vec<(String, u64)>,
}
#[derive(Clone)]
pub(super) struct Upgrade {
    pub id: String,
    pub kind: Kind,
    pub left: u64,
}
impl Preparation {
    pub(super) fn upgrade_purchase(
        &mut self,
        uid: Option<u64>,
        kind: Kind,
        player: &mut Player,
    ) -> Result<(), Error> {
        let Some(uid) = uid else {
            return Ok(());
        };
        for i in 0..self.draft_effects.upgrades.len() {
            if self.draft_effects.upgrades[i].kind != kind {
                continue;
            }
            let Some(loc) = self.find(uid) else {
                break;
            };
            let piece = self.piece(loc);
            let table = if kind == Kind::Chess {
                "chess"
            } else {
                "items"
            };
            if self.rules.data.table(table).unwrap()[&piece.key]["isGolden"] == true {
                break;
            }
            let Some(upgrade) = self
                .rules
                .card(kind, &piece.key)
                .and_then(|c| c.upgrade.clone())
            else {
                break;
            };
            if kind == Kind::Item {
                // The purchased normal already owns one copy. No overdraw for
                // this extra copy: retain the charge until stock permits it.
                if self.item_left(&piece.key).is_some_and(|n| n < 1) {
                    break;
                }
                self.piece_mut(loc).key = upgrade;
            } else {
                self.promote_equipped_holder(uid)?;
            }
            self.draft_effects.upgrades[i].left -= 1;
            self.recompute_meta(player)?;
            // Later refs see the same purchased piece, now golden, and keep their charge.
            break;
        }
        self.draft_effects.upgrades.retain(|u| u.left > 0);
        Ok(())
    }
    pub(in crate::solo) fn battle_choices(&self) -> Vec<crate::battle::runtime::BattleChoice> {
        let bench_units = self.hand.iter().chain(&self.temp).flatten().count();
        self.draft_effects
            .battle
            .iter()
            .map(|(id, sequence)| crate::battle::runtime::BattleChoice {
                effect_id: id.clone(),
                sequence: *sequence,
                bench_units,
            })
            .collect()
    }
    pub(in crate::solo) fn draft_mode(&self) -> &str {
        &self.rules.mode_id
    }
    pub(in crate::solo) fn draw_draft(
        &self,
        rng: &mut Rng,
        stage: &str,
        round: u32,
    ) -> Result<Option<Value>, String> {
        let bond_available = |id: &str| {
            self.meta
                .as_ref()
                .is_some_and(|m| m.rules.bonds.iter().any(|b| b.id == id && !b.off))
                && self
                    .pool
                    .keys()
                    .any(|key| self.own_bonds(key).iter().any(|b| b == id))
        };
        draft::generate(
            &self.rules.data,
            rng,
            &draft::Context {
                mode: &self.rules.mode_id,
                stage: Some(stage),
                round,
                bond_available: &bond_available,
                item_available: &|id| self.item_available(id),
            },
        )
    }
    pub(in crate::solo) fn apply_draft(
        &mut self,
        card: &Value,
        map: &mut MapConfig,
        player: &mut Player,
    ) -> Result<(), Error> {
        let id = card["id"].as_str().ok_or(Error::BadTarget)?;
        match card["kind"].as_str() {
            Some("item") => {
                self.gain(Kind::Item, id, GainSource::Grant, player)?;
            }
            Some("bounty") => {
                let record = self.rules.data.table("choices").unwrap()["cards"]["bounty"]
                    .as_array()
                    .and_then(|a| a.iter().find(|c| c["effectId"] == id))
                    .ok_or(Error::BadTarget)?
                    .clone();
                self.add_bounty(&record)?;
            }
            Some("tactic") => {
                let buffs = self.rules.data.table("effects").unwrap()[id]["buffs"]
                    .as_array()
                    .ok_or(Error::BadTarget)?
                    .clone();
                let mut changes_map = false;
                let mut battle = false;
                for buff in buffs {
                    let count = effects::n(&buff["bb"]["count"], 1.).trunc().max(0.);
                    if count < 0. || count > MAX_FUNDS as f64 {
                        return Err(Error::Capacity);
                    }
                    match buff["key"].as_str() {
                        Some("auto_chess_change_map") => changes_map = true,
                        Some(
                            "env_gbuff_new"
                            | "env_gbuff_new_with_verify"
                            | "char_respawntime_mul"
                            | "global_special_choice_prep_finish_bench_at_least"
                            | "global_special_choice_prep_finish_bench_at_most"
                            | "global_special_choice_prep_finish_same_row_at_least",
                        ) => battle = true,
                        Some(
                            "single_special_choice_gloden_equip_chess"
                            | "single_special_choice_gloden_char_chess",
                        ) => {
                            let count = count.trunc() as u64;
                            if count > 0 {
                                if self.draft_effects.upgrades.len()
                                    + self.draft_effects.battle.len()
                                    >= self.capacity
                                {
                                    return Err(Error::Capacity);
                                }
                                let kind =
                                    if buff["key"] == "single_special_choice_gloden_equip_chess" {
                                        Kind::Item
                                    } else {
                                        Kind::Chess
                                    };
                                self.draft_effects.sequence = add(self.draft_effects.sequence, 1)
                                    .map_err(|_| Error::Capacity)?;
                                self.draft_effects.upgrades.push(Upgrade {
                                    id: format!("choice:{id}#{}", self.draft_effects.sequence),
                                    kind,
                                    left: count,
                                });
                            }
                        }
                        Some("global_special_choice_gain_equip") => {
                            for _ in 0..count.min(32.).ceil() as usize {
                                let pool = buff["bbStr"]["pool"].as_str();
                                let valid = pool.is_some_and(|id| {
                                    self.rules.data.table("choices").unwrap()["pools"][id]["kind"]
                                        == "equip"
                                });
                                let mut key = if valid {
                                    self.roll_meta_item(pool, 6, false)?
                                } else {
                                    None
                                };
                                if key.is_none() {
                                    let items: Vec<_> = self
                                        .rules
                                        .items
                                        .values()
                                        .filter(|c| {
                                            c.shop && c.tier == 1 && self.item_available(&c.key)
                                        })
                                        .map(|c| c.key.clone())
                                        .collect();
                                    key = self.meta.as_mut().unwrap().rng.pick(&items).cloned();
                                }
                                if let Some(key) = key {
                                    self.gain(Kind::Item, &key, GainSource::Grant, player)?;
                                }
                            }
                        }
                        Some("single_special_choice_gain_bond_chess") => {
                            let bond = buff["bbStr"]["bond"]
                                .as_str()
                                .ok_or(Error::BadTarget)?
                                .to_owned();
                            for _ in 0..count.min(32.).ceil() as usize {
                                let bonds = [bond.clone()];
                                let key = self
                                    .roll_meta_chess(self.level.max(1), 1, None, Some(&bonds), &[])
                                    .or_else(|| {
                                        self.roll_meta_chess(6, 1, None, Some(&bonds), &[])
                                    });
                                if let Some(key) = key {
                                    self.grant_recruit(&key, player)?;
                                }
                            }
                        }
                        Some("global_special_choice_bond_addlayer") => {
                            for bond in buff["bbStr"]["bond_list"]
                                .as_str()
                                .unwrap_or("")
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                            {
                                self.add_layers(bond, count, false, player)?;
                            }
                        }
                        Some("global_special_choice_gain_coin") => {
                            player.earn(count as u64).map_err(|_| Error::Capacity)?
                        }
                        Some("global_special_choice_refresh_free") => {
                            self.free_refreshes = add(self.free_refreshes, count as u64)
                                .map_err(|_| Error::Capacity)?
                        }
                        Some("global_special_choice_all_activated") => (),
                        _ => {
                            return Err(Error::Content(format!(
                                "Draft effect not implemented: {id}: {}",
                                buff["key"]
                            )))
                        }
                    }
                }
                if battle {
                    if self.draft_effects.battle.len() + self.draft_effects.upgrades.len()
                        >= self.capacity
                    {
                        return Err(Error::Capacity);
                    }
                    let sequence =
                        add(self.draft_effects.sequence, 1).map_err(|_| Error::Capacity)?;
                    crate::battle::runtime::BattleChoice {
                        effect_id: id.into(),
                        sequence,
                        bench_units: 0,
                    }
                    .validate(&self.rules.data)
                    .map_err(Error::Content)?;
                    self.draft_effects.battle.push((id.into(), sequence));
                    self.draft_effects.sequence = sequence;
                }
                if changes_map {
                    let switches = MapConfig {
                        effects: vec![id.into()],
                        ..Default::default()
                    }
                    .resolved(&self.rules.data)
                    .map_err(Error::Content)?;
                    map.device_overrides.extend(switches.device_overrides);
                    map.validate().map_err(Error::Content)?;
                    self.draft_effects.sequence =
                        add(self.draft_effects.sequence, 1).map_err(|_| Error::Capacity)?;
                    if self.draft_effects.maps.len() >= self.capacity {
                        return Err(Error::Capacity);
                    }
                    self.draft_effects
                        .maps
                        .push((id.into(), self.draft_effects.sequence));
                }
            }
            _ => return Err(Error::BadTarget),
        }
        self.recompute_meta(player)
    }
}
