//! Candidate parameters from the current owned public ledger and static metadata.
use super::*;
use crate::candidates::{build, Prepared};
use std::collections::BTreeSet;

fn natural(v: &Value, key: &str) -> Result<u64> {
    field(v, key)?
        .as_u64()
        .ok_or_else(|| format!("Invalid public {key}"))
}
fn boolean(v: &Value, key: &str) -> Result<bool> {
    field(v, key)?
        .as_bool()
        .ok_or_else(|| format!("Missing public boolean {key}"))
}
fn cell(row: &Value) -> Result<usize> {
    let (r, c) = (integer(field(row, "row")?)?, integer(field(row, "col")?)?);
    if !(9..13).contains(&r) || !(2..11).contains(&c) {
        return Err("Malformed public board occupancy".into());
    }
    Ok(((r - 9) * 9 + c - 2) as usize)
}

impl PolicyEncoder {
    pub fn current_candidates(
        &self,
        state: &Value,
        history: &Value,
        blocked: &BTreeSet<String>,
    ) -> Result<Vec<Value>> {
        let own = field(state, "own")?;
        let public = field(state, "public")?;
        if get(public, "modeId") != "mode_single_abyss" || truth(get(public, "offers")) {
            return Err("Only Desperation public policy is maintained".into());
        }
        if get(own, "diy") != &self.diy {
            return Err("DIY configuration differs from the resolved public catalog".into());
        }
        for key in ["loadout", "ops"] {
            if object(field(own, key)?)?.len() != 0 {
                return Err(format!("Nondefault or unknown {key} is unsupported"));
            }
        }
        if !array(field(own, "standIns")?)?.is_empty() || truth(get(public, "rerollVote")) {
            return Err("Unresolved public stand-ins or setup vote".into());
        }
        let terminal = boolean(state, "personalEnded")?;
        let phase = string(state, "phase")?;
        if terminal {
            return build(Vec::new(), None, blocked, true);
        }
        if !boolean(state, "decisionRequired")? {
            return Err("No player decision; advance the environment, do not invent wait".into());
        }
        let mut prefix = Vec::new();
        let mut prepared = None;
        match phase {
            "INFO_CHECK" => {
                prefix.push(serde_json::json!({
                    "t": "g.infoReady", "setupRevision": natural(public, "setupRevision")?
                }));
            }
            "BAND_DRAFT" => {
                let draft = field(public, "draft")?;
                if get(draft, "turn") != get(own, "playerId") {
                    return Err("Native solo draft is not this player's turn".into());
                }
                let picks = object(field(draft, "picks")?)?;
                for band in &self.bands {
                    if !picks.values().any(|v| v.as_str() == Some(band)) {
                        prefix.push(serde_json::json!({"t": "g.band", "bandId": band}));
                    }
                }
            }
            "SP_DRAFT" | "PREP" if phase == "SP_DRAFT" || truth(get(own, "personalChoice")) => {
                let choice = if phase == "SP_DRAFT" {
                    field(public, "sp")?
                } else {
                    field(own, "personalChoice")?
                };
                let cards = array(field(choice, "cards")?)?;
                if !(1..=6).contains(&cards.len()) {
                    return Err("Unknown mandatory choice domain".into());
                }
                if phase == "SP_DRAFT" && get(choice, "turn") != get(own, "playerId") {
                    return Err("Native solo SP draft is not this player's turn".into());
                }
                for (index, card) in cards.iter().enumerate() {
                    if !["item", "bounty", "tactic"].contains(&string(card, "kind")?) {
                        return Err("Unknown public choice card kind".into());
                    }
                    if phase == "SP_DRAFT" {
                        if natural(card, "idx")? != index as u64 {
                            return Err("Unknown SP card position/stock availability".into());
                        }
                        let sold = boolean(card, "soldOut")?;
                        if object(field(choice, "taken")?)?.contains_key(&index.to_string()) || sold
                        {
                            continue;
                        }
                    }
                    let mut action = serde_json::json!({"t": "g.choice", "idx": index});
                    if phase == "PREP" {
                        if get(choice, "round") != get(state, "round") {
                            return Err("Unknown personal choice identity".into());
                        }
                        action["choiceId"] = Value::String(string(choice, "id")?.to_owned());
                    }
                    prefix.push(action);
                }
            }
            "PREP" => {
                if boolean(own, "ready")? || !boolean(own, "alive")? {
                    return Err("No active prep decision".into());
                }
                let recovery =
                    natural(history, "used")? >= arkguard_engine::history::PREP_ACTION_LIMIT;
                let hand = array(field(own, "hand")?)?;
                let temp = array(field(own, "temp")?)?;
                if hand.len() != 10 {
                    return Err("Malformed public piece area".into());
                }
                let ready = boolean(own, "canReady")?;
                if ready {
                    if temp.iter().any(truth) {
                        return Err("canReady disagrees with public temporary reserve".into());
                    }
                    prefix.push(serde_json::json!({"t": "g.ready", "ready": true}));
                }
                let shop = field(own, "shop")?;
                if !recovery {
                    let funds = natural(own, "funds")?;
                    if hand.iter().any(Value::is_null) {
                        for (name, action, index_key, limit) in [
                            ("slots", "g.buy", "slot", 16),
                            ("rewardOffer", "g.reward", "idx", 6),
                        ] {
                            let slots = if name == "slots" {
                                array(field(shop, name)?)?.as_slice()
                            } else if truth(get(shop, name)) {
                                array(field(get(shop, name), "slots")?)?.as_slice()
                            } else {
                                &[]
                            };
                            if slots.len() > limit {
                                return Err("Shop/reward exceeds native protocol domain".into());
                            }
                            for (index, slot) in slots.iter().enumerate() {
                                if !truth(slot) {
                                    continue;
                                }
                                if !["chess", "item"].contains(&string(slot, "kind")?) {
                                    return Err("Unknown shop kind or availability".into());
                                }
                                if !boolean(slot, "sold")? && natural(slot, "price")? <= funds {
                                    let mut intent = serde_json::json!({"t": action});
                                    intent[index_key] = serde_json::json!(index);
                                    prefix.push(intent);
                                }
                            }
                        }
                    }
                    if funds >= natural(shop, "refreshPrice")? {
                        prefix.push(serde_json::json!({"t": "g.refresh"}));
                    }
                    prefix.push(serde_json::json!({"t": "g.freeze"}));
                    if natural(shop, "level")? < natural(shop, "maxLevel")?
                        && funds >= natural(shop, "upgradePrice")?
                    {
                        prefix.push(serde_json::json!({"t": "g.levelUp"}));
                    }
                }
                prepared = Some(self.piece_inputs(own, recovery, ready)?);
            }
            _ => return Err("Unsupported actionable native phase".into()),
        }
        for action in &mut prefix {
            action.as_object_mut().unwrap().sort_keys();
        }
        build(prefix, prepared, blocked, false)
    }

    fn piece_inputs(&self, own: &Value, recovery: bool, ready: bool) -> Result<Prepared> {
        let full = natural(own, "deployCount")? >= natural(own, "deployCap")?;
        if recovery && ready {
            return Ok((Vec::new(), 0, full, recovery, true));
        }
        let mut locations = HashMap::new();
        let mut pieces = Vec::new();
        let mut uids = HashSet::new();
        let mut occupied = 0_u64;
        for (area, name) in ["board", "hand", "temp"].into_iter().enumerate() {
            for row in array(field(own, name)?)? {
                if row.is_null() {
                    if area == 0 {
                        return Err("Null board occupant".into());
                    }
                    continue;
                }
                let uid = natural(row, "uid")?;
                if uid == 0 || !uids.insert(uid) {
                    return Err("Duplicate/invalid public UID".into());
                }
                if !["chess", "token", "item"].contains(&string(row, "kind")?) {
                    return Err("Unknown public piece kind".into());
                }
                self.definition(&identity(string(row, "kind")?, string(row, "id")?))?;
                if area == 0 {
                    let bit = 1 << cell(row)?;
                    if occupied & bit != 0 || string(row, "kind")? == "item" {
                        return Err("Malformed public board occupancy".into());
                    }
                    occupied |= bit;
                }
                for item in array(field(row, "items")?)? {
                    let uid = natural(item, "uid")?;
                    if uid == 0 || !uids.insert(uid) {
                        return Err("Duplicate/invalid equipment UID".into());
                    }
                    self.definition(&identity("item", string(item, "id")?))?;
                }
                locations.insert(uid, (area, row));
                pieces.push((area, row));
            }
        }
        if pieces
            .iter()
            .filter(|(area, row)| *area == 0 && get(row, "kind") == "chess")
            .count() as u64
            != natural(own, "deployCount")?
        {
            return Err("Public chess capacity disagrees with occupancy".into());
        }
        let deployment = field(own, "deployment")?;
        if !["normal", "bossL", "bossR"].contains(&string(deployment, "field")?) {
            return Err("Missing visible deployment map".into());
        }
        let mut terrain = [0_u64; 3];
        for tile in array(field(deployment, "cells")?)? {
            let bit = 1 << cell(tile)?;
            let kind = match string(tile, "kind")? {
                "melee" => 1,
                "ranged" => 2,
                _ => return Err("Unknown deployment cell".into()),
            };
            if terrain[0] & bit != 0 {
                return Err("Unknown/duplicate deployment cell".into());
            }
            terrain[0] |= bit;
            terrain[kind] |= bit;
        }
        let mut prepared = Vec::with_capacity(pieces.len());
        for (area, row) in pieces {
            let def = self.definition(&identity(string(row, "kind")?, string(row, "id")?))?;
            let mut kind = match string(row, "kind")? {
                "chess" => 0,
                "token" => 1,
                _ => 2,
            };
            let mut mask = 0;
            if !recovery || area == 2 {
                if kind < 2 {
                    if terrain[0] != 0 {
                        mask = match string(&def.domain, "placement")? {
                            "all" => terrain[0],
                            "melee" => terrain[1],
                            "high" => terrain[2],
                            _ => return Err("Missing or unknown public placement position".into()),
                        };
                    }
                    if kind == 1 && area != 0 {
                        let owner = locations.get(&natural(row, "ownerUid")?);
                        mask &= match owner {
                            Some((0, owner)) if get(owner, "kind") == "chess" => {
                                self.token_cells(row, owner, &def.domain)?
                            }
                            _ => 0,
                        } & !occupied;
                    }
                } else {
                    kind = match string(&def.domain, "itemType")? {
                        "EQUIP" => 2,
                        "MAGIC" => 3,
                        _ => return Err("Unknown item capability".into()),
                    };
                }
            }
            let position = if area == 0 { cell(row)? as i8 } else { -1 };
            let direction = if area == 0 {
                DIRECTIONS
                    .iter()
                    .position(|d| get(row, "dir").as_str() == Some(*d))
                    .ok_or("Malformed public board occupancy")? as i8
            } else {
                -1
            };
            prepared.push((
                natural(row, "uid")?,
                area as u8,
                kind,
                position,
                direction,
                mask,
                array(field(row, "items")?)?
                    .iter()
                    .map(|v| natural(v, "uid"))
                    .collect::<Result<_>>()?,
            ));
        }
        let empty = array(field(own, "hand")?)?
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_null())
            .fold(0_u16, |mask, (i, _)| mask | (1 << i));
        Ok((prepared, empty, full, recovery, ready))
    }

    fn token_cells(&self, token: &Value, owner: &Value, domain: &Value) -> Result<u64> {
        let adjacent = get(token, "id") == "token_10032_jesca2_jckshd";
        let range = get(domain, "ownerRange");
        let outside = get(domain, "ownerRangeOutside");
        if !adjacent && range != &Value::Bool(true) && outside != &Value::Bool(true) {
            return Ok(crate::candidates::CELL_MASK);
        }
        let owner_cell = cell(owner)?;
        let (r, c) = ((owner_cell / 9) as i64 + 9, (owner_cell % 9) as i64 + 2);
        let def = self.definition(&identity("chess", string(owner, "id")?))?;
        let mut allowed = 0;
        for target in 0..36 {
            let (row, col) = (target / 9 + 9, target % 9 + 2);
            let inside = def.ranges.iter().any(|&(dr, dc)| {
                let (dr, dc) = match get(owner, "dir").as_str() {
                    Some("RIGHT") => (dr, dc),
                    Some("UP") => (dc, -dr),
                    Some("LEFT") => (-dr, -dc),
                    Some("DOWN") => (-dc, dr),
                    _ => (i64::MAX, i64::MAX),
                };
                r.checked_add(dr) == Some(row) && c.checked_add(dc) == Some(col)
            });
            if if adjacent {
                (row - r).abs() + (col - c).abs() == 1
            } else {
                (!truth(range) || inside) && (!truth(outside) || !inside)
            } {
                allowed |= 1 << target;
            }
        }
        Ok(allowed)
    }
}
