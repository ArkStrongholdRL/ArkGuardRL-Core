// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-session bodies for the existing four DIY identities. Tables remain shared.
use super::*;
use serde_json::{json, Map};

fn strip_potential(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in ["potDown", "potMin", "potBelow"] {
                map.remove(key);
            }
            for value in map.values_mut() {
                strip_potential(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(strip_potential),
        _ => {}
    }
}
fn form<'a>(unit: &'a Value, slot: &Value) -> Result<&'a Value, String> {
    let s = &slot["status"];
    let key = format!(
        "{}/{}/{}/{}",
        s["phase"], s["level"], s["skillLevel"], s["equipLevel"]
    );
    unit["forms"]
        .get(&key)
        .ok_or_else(|| format!("Missing DIY form {key}"))
}
fn compose(slot: &Value, unit: &Value, pick: &Value, bonds: &Value) -> Result<Value, String> {
    let form = form(unit, slot)?;
    let skills = form["skills"].as_array().ok_or("Missing DIY skills")?;
    let skill = skills
        .iter()
        .find(|s| s["index"] == pick["skillIndex"])
        .ok_or("Unknown DIY skill")?;
    let module = form["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| !pick["uniEquipId"].is_null() && m["uniEquipId"] == pick["uniEquipId"]);
    let mut record = Map::new();
    for key in [
        "chessId",
        "baseId",
        "goldenId",
        "isGolden",
        "tier",
        "identifier",
        "isHidden",
        "isDiy",
        "visible",
        "chessType",
        "backup",
        "shopSortId",
        "bonds",
        "garrisonIds",
        "price",
        "sellPrice",
        "upgradeNum",
        "upgradeChessId",
        "status",
        "diyRequirement",
    ] {
        if let Some(v) = slot.get(key) {
            record.insert(key.into(), v.clone());
        }
    }
    record.insert("bonds".into(), bonds.clone());
    for key in [
        "charId",
        "name",
        "appellation",
        "rarity",
        "profession",
        "subProfessionId",
        "subProfessionName",
        "position",
        "nationId",
    ] {
        if let Some(v) = unit.get(key) {
            record.insert(key.into(), v.clone());
        }
    }
    for key in [
        "stats",
        "immunities",
        "rangeId",
        "rangeGrid",
        "dmgType",
        "attackKind",
        "projectile",
        "canHitFly",
        "targetPriority",
        "trait",
    ] {
        if let Some(v) = form.get(key) {
            record.insert(key.into(), v.clone());
        }
    }
    if form["placeOnRanged"] == true {
        record.insert("placeOnRanged".into(), json!(true));
    }
    let mut talents = form["talents"].as_array().cloned().unwrap_or_default();
    for t in &mut talents {
        strip_potential(t);
    }
    if let Some(m) = module {
        for (key, value) in m["attr"].as_object().into_iter().flatten() {
            let v = record["stats"][key].as_f64().unwrap_or(0.)
                + value.as_f64().ok_or("Invalid DIY module attribute")?;
            let v = if v.fract() != 0. && v.abs() < 1e6 {
                (v * 1e6 + 0.5).floor() / 1e6
            } else {
                v
            };
            record.get_mut("stats").ok_or("Missing DIY stats")?[key] = json!(v);
        }
        if !m["traitOverride"].is_null() {
            record.insert("trait".into(), m["traitOverride"].clone());
        }
        for change in m["talentChanges"].as_array().into_iter().flatten() {
            let mut t = change.clone();
            strip_potential(&mut t);
            let index = t["talentIndex"].clone();
            let map = t.as_object_mut().ok_or("Invalid DIY module talent")?;
            map.remove("talentIndex");
            map.insert("index".into(), index.clone());
            map.insert("fromModule".into(), json!(true));
            if let Some(old) = talents
                .iter_mut()
                .find(|x| index.as_i64().is_some_and(|i| i >= 0) && x["index"] == index)
            {
                for key in ["name", "desc", "descRaw", "rangeGrid", "tokenKey"] {
                    if t[key].is_null() || t[key].as_str() == Some("") {
                        t[key] = old[key].clone();
                    }
                }
                for key in ["bb", "bbStr"] {
                    let mut fields = old[key].as_object().cloned().unwrap_or_default();
                    fields.extend(t[key].as_object().cloned().unwrap_or_default());
                    t[key] = Value::Object(fields);
                }
                t["hidden"] = json!(old["hidden"] == true && t["hidden"] == true);
                *old = t;
            } else {
                talents.push(t);
            }
        }
    }
    talents.retain(|t| {
        t["name"].as_str().is_some_and(|s| !s.is_empty())
            || t["desc"].as_str().is_some_and(|s| !s.is_empty())
            || t["bb"].as_object().is_some_and(|m| !m.is_empty())
            || t["tokenKey"].is_string()
    });
    let mut tokens = std::collections::BTreeSet::new();
    for id in form["displayTokens"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(std::iter::once(&skill["overrideTokenKey"]))
        .chain(talents.iter().map(|t| &t["tokenKey"]))
    {
        if let Some(id) = id
            .as_str()
            .filter(|_| form["tokens"].as_array().is_some_and(|a| a.contains(id)))
        {
            tokens.insert(id);
        }
    }
    record.insert("tokens".into(), json!(tokens));
    record.insert("talents".into(), json!(talents));
    record.insert("skill".into(), skill.clone());
    record.insert(
        "skills".into(),
        json!(skills
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s["isDefault"] = json!(s["index"] == pick["skillIndex"]);
                s
            })
            .collect::<Vec<_>>()),
    );
    let level = slot["status"]["equipLevel"].as_u64().unwrap_or(0);
    let mid = pick["uniEquipId"].as_str();
    let info = &unit["moduleNames"][mid.unwrap_or("")];
    let module_metadata = if mid.is_some() {
        json!({
            "id": pick["uniEquipId"], "name": info["name"], "type": info["typeName"],
            "level": level, "active": level > 0 && module.is_some()
        })
    } else if level > 0 {
        json!({"id": null, "name": null, "type": null, "level": level, "active": false})
    } else {
        Value::Null
    };
    record.insert("module".into(), module_metadata);
    if slot["isGolden"] == true && level > 0 {
        record.insert("statsBase".into(), form["stats"].clone());
        record.insert("traitBase".into(), form["trait"].clone());
        let mut ts = form["talents"].clone();
        strip_potential(&mut ts);
        record.insert("talentsBase".into(), ts);
        let mut mods = form["modules"].as_array().cloned().unwrap_or_default();
        for m in &mut mods {
            strip_potential(m);
            m["isDefault"] = json!(mid.is_some() && m["uniEquipId"] == pick["uniEquipId"]);
        }
        record.insert("modules".into(), json!(mods));
    }
    let a = &unit["assets"];
    let golden = slot["isGolden"] == true;
    let mut assets = Map::new();
    for key in [
        "avatar",
        "portrait",
        "spine",
        "skillIcon",
        "subProfIcon",
        "voice",
    ] {
        let alt = format!("{key}Golden");
        let v = if golden {
            a.get(&alt).filter(|v| !v.is_null()).or_else(|| a.get(key))
        } else {
            a.get(key)
        };
        if let Some(v) = v {
            assets.insert(key.into(), v.clone());
        }
    }
    if !assets.contains_key("skillIcon") {
        assets.insert("skillIcon".into(), skill["iconId"].clone());
    }
    record.insert("assets".into(), Value::Object(assets));
    for key in [
        "kitRef",
        "skillSpec",
        "talentMods",
        "dsl",
        "pluginId",
        "origin",
    ] {
        if let Some(v) = form.get(key).or_else(|| unit.get(key)) {
            record.insert(key.into(), v.clone());
        }
    }
    record.insert("diyFor".into(), slot["baseId"].clone());
    Ok(Value::Object(record))
}
impl Catalog {
    pub fn with_diy(&self, picks: &Value) -> Result<Self, String> {
        let picks = picks
            .as_object()
            .filter(|p| p.len() <= 4)
            .ok_or("DIY requires at most four slot picks")?;
        if picks.is_empty() {
            let mut out = self.clone();
            out.tables
                .insert("chess".into(), Arc::clone(&self.base_chess));
            out.tables.remove("diy");
            return Ok(out);
        }
        let backups = self.table("backups").ok_or("Missing backups")?;
        let diy = &backups["diy"];
        let mut chess = self.base_chess.as_ref().clone();
        let mut kept = Map::new();
        let mut tiers_seen = std::collections::BTreeSet::new();
        let mut owned_seen = std::collections::BTreeSet::new();
        for (id, input) in picks {
            let slot = diy["slots"]
                .get(id)
                .ok_or_else(|| format!("Unknown DIY slot {id}"))?;
            if input.is_null() {
                continue;
            }
            let char_id = input["charId"]
                .as_str()
                .filter(|id| id.len() <= 64)
                .ok_or("Invalid DIY operator")?;
            let tier = slot["tier"].to_string();
            let proto = diy["prototypes"][&tier]
                .as_array()
                .is_some_and(|a| a.contains(&json!(char_id)));
            if !proto
                && !diy["ownedPool"]
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(char_id)))
            {
                return Err(format!("Operator unavailable in DIY slot {id}"));
            }
            if !tiers_seen.insert((tier.clone(), char_id))
                || (!proto && !owned_seen.insert(char_id))
            {
                return Err("Duplicate DIY operator".into());
            }
            let mut pick = input.clone();
            if !pick.is_object() {
                return Err("Invalid DIY pick".into());
            }
            if proto {
                let lock = &diy["locked"][&tier][char_id];
                if !lock.is_object()
                    || (!pick["skillIndex"].is_null() && pick["skillIndex"] != lock["skillIndex"])
                    || (!pick["uniEquipId"].is_null() && pick["uniEquipId"] != lock["uniEquipId"])
                {
                    return Err("Invalid locked DIY selection".into());
                }
                pick = json!({"charId":char_id,"skillIndex":lock["skillIndex"],"uniEquipId":lock["uniEquipId"]});
            } else {
                if pick["skillIndex"].as_u64().is_none_or(|i| i > 9) {
                    return Err("Invalid DIY skill index".into());
                }
                if pick["uniEquipId"] == "none" {
                    pick["uniEquipId"] = Value::Null;
                }
                pick = json!({"charId":char_id,"skillIndex":pick["skillIndex"],"uniEquipId":pick["uniEquipId"]});
            }
            let unit = backups["units"]
                .get(char_id)
                .ok_or("Missing DIY operator body")?;
            let golden_id = slot["goldenId"]
                .as_str()
                .ok_or("Missing DIY golden identity")?;
            let elite = form(unit, &chess[golden_id])?;
            if !pick["uniEquipId"].is_null() {
                let m = elite["modules"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|m| m["uniEquipId"] == pick["uniEquipId"])
                    .ok_or("Unknown DIY module")?;
                if m["typeName"]
                    .as_str()
                    .is_some_and(|t| ["ISW-", "SO-", "RA-"].iter().any(|p| t.starts_with(p)))
                {
                    return Err("DIY module belongs to another mode".into());
                }
            }
            for key in [id.as_str(), golden_id] {
                let mut body = compose(
                    &chess[key],
                    unit,
                    &pick,
                    &diy["operators"][char_id]["bonds"],
                )?;
                if proto {
                    body["diyProto"] = json!(true);
                }
                chess[key] = body;
            }
            kept.insert(id.clone(), pick);
        }
        let mut out = self.clone();
        out.tables.insert("chess".into(), Arc::new(chess));
        out.tables
            .insert("diy".into(), Arc::new(Value::Object(kept)));
        Ok(out)
    }
}
