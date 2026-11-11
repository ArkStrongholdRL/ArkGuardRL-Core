// SPDX-License-Identifier: GPL-3.0-or-later
//! Single-player SP draft generation. The caller owns the independent draft RNG
//! and supplies live pool availability; generating a draft does not award cards.
use crate::{data::Catalog, deterministic::rng::Rng};
use serde_json::{json, Value};
mod bounty;
mod runner;
pub(super) use runner::Flow;

pub struct Context<'a> {
    pub mode: &'a str,
    pub stage: Option<&'a str>,
    pub round: u32,
    pub bond_available: &'a dyn Fn(&str) -> bool,
    pub item_available: &'a dyn Fn(&str) -> bool,
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn num(v: &Value, fallback: f64) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .or_else(|| v.as_bool().map(|b| f64::from(u8::from(b))))
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}
fn pick(rng: &mut Rng, weights: &[f64]) -> Option<usize> {
    rng.weighted(weights)
        .or_else(|| (!weights.is_empty()).then_some(0))
}
fn weighted<'a>(
    rng: &mut Rng,
    entries: &[&'a Value],
    weight: impl Fn(&Value) -> f64,
) -> Option<&'a Value> {
    pick(rng, &entries.iter().map(|v| weight(v)).collect::<Vec<_>>()).map(|i| entries[i])
}
struct Data<'a> {
    catalog: &'a Catalog,
    choices: &'a Value,
    ctx: &'a Context<'a>,
}
impl Data<'_> {
    fn record(&self, table: &str, id: &Value) -> &Value {
        id.as_str()
            .and_then(|s| self.catalog.table(table)?.get(s))
            .unwrap_or(&Value::Null)
    }
    fn items(&self, lo: i64, hi: i64) -> Vec<String> {
        let mut items: Vec<_> = self
            .catalog
            .table("items")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(id, it)| {
                it["itemType"] == "EQUIP"
                    && it["isGolden"] != true
                    && it["hideInShop"] != true
                    && it["shopExcluded"] != true
                    && it["tier"].as_i64().is_some_and(|t| t >= lo && t <= hi)
                    && (self.ctx.item_available)(id)
            })
            .map(|(id, it)| (it["tier"].as_i64().unwrap(), id.clone()))
            .collect();
        items.sort();
        items.into_iter().map(|(_, id)| id).collect()
    }
    fn item_card(&self, id: &str) -> Value {
        let it = &self.catalog.table("items").unwrap()[id];
        json!({"kind":"item","id":id,"name":it.get("name").unwrap_or(&json!(id)),"desc":it["desc"].as_str().unwrap_or(""),"tier":it["tier"].as_i64().unwrap_or(1),"price":0})
    }
    fn bond_ok(&self, c: &Value) -> bool {
        let effect = self.record("effects", &c["effectId"]);
        let buffs = list(&effect["buffs"]);
        if buffs.is_empty() {
            return true;
        }
        let mut bonds = Vec::new();
        for b in buffs {
            match b["key"].as_str() {
                Some("single_special_choice_gain_bond_chess") if b["bbStr"]["bond"].is_string() => {
                    bonds.push(b["bbStr"]["bond"].as_str().unwrap())
                }
                Some("global_special_choice_bond_addlayer") => bonds.extend(
                    b["bbStr"]["bond_list"]
                        .as_str()
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty()),
                ),
                _ => return true,
            }
        }
        bonds.is_empty() || bonds.into_iter().any(self.ctx.bond_available)
    }
    fn tactics(&self, rng: &mut Rng, n: usize) -> Vec<Value> {
        let pool: Vec<_> = list(&self.choices["cards"]["tactic"])
            .iter()
            .filter(|c| {
                (c["kind"] != "terrain" || self.ctx.stage.is_some_and(|id| c["stageId"] == id))
                    && self.record("effects", &c["effectId"]).is_object()
                    && self.bond_ok(c)
            })
            .collect();
        let spec = &self.choices["tacticDraft"];
        let official = spec.is_object()
            && (!spec["rounds"].is_array()
                || list(&spec["rounds"]).contains(&json!(self.ctx.round)));
        let kinds = if official { list(&spec["kinds"]) } else { &[] };
        let filtered: Vec<_> = pool
            .iter()
            .copied()
            .filter(|c| kinds.is_empty() || kinds.contains(&c["kind"]))
            .collect();
        let pool = if filtered.is_empty() {
            &pool
        } else {
            &filtered
        };
        (0..n).filter_map(|_|weighted(rng,pool,|c|if official {num(&spec["weights"][c["effectId"].as_str().unwrap_or("")],1.)}else{1.}))
            .map(|c|json!({"kind":"tactic","id":c["effectId"],"name":c["name"],"desc":c["desc"].as_str().unwrap_or(""),"tier":null,"team":c["team"]==true,"tacticKind":c["kind"]})).collect()
    }
    fn shop(&self, rng: &mut Rng, n: usize) -> Option<Vec<Value>> {
        let spec = &self.choices["shopDraft"];
        let slots: Vec<_> = list(&spec["slots"])
            .iter()
            .filter(|v| v.is_object())
            .collect();
        if slots.is_empty()
            || spec["rounds"].is_array() && !list(&spec["rounds"]).contains(&json!(self.ctx.round))
        {
            return None;
        }
        let coin = spec["coin"]
            .as_str()
            .filter(|_| self.record("items", &spec["coin"]).is_object());
        let mut out = Vec::new();
        for slot in slots {
            let mut kinds: Vec<_> = slot
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| {
                    if *k == "coin" {
                        coin.is_some()
                    } else {
                        k.parse::<i64>().is_ok()
                    }
                })
                .collect();
            // JS Object.entries visits integer-index keys before other keys.
            kinds.sort_by_key(|(k, _)| {
                k.parse::<u32>()
                    .ok()
                    .filter(|n| *n < u32::MAX && n.to_string() == **k)
                    .map_or((1, 0), |n| (0, n))
            });
            let Some(i) = pick(
                rng,
                &kinds.iter().map(|(_, w)| num(w, 0.)).collect::<Vec<_>>(),
            ) else {
                continue;
            };
            let kind = kinds[i].0;
            let candidates = if kind == "coin" {
                vec![coin.unwrap().to_owned()]
            } else {
                let tier = kind.parse::<i64>().unwrap().clamp(0, 64);
                (1..=tier)
                    .rev()
                    .map(|t| self.items(t, t))
                    .find(|v| !v.is_empty())
                    .unwrap_or_else(|| self.items(1, 6))
            };
            if let Some(i) = pick(
                rng,
                &candidates
                    .iter()
                    .map(|id| num(&spec["itemWeights"][id], 1.))
                    .collect::<Vec<_>>(),
            ) {
                out.push(self.item_card(&candidates[i]));
            }
        }
        rng.shuffle(&mut out);
        out.truncate(n);
        Some(out)
    }
    fn cards(&self, rng: &mut Rng, family: &str, n: usize, sch: &Value) -> Vec<Value> {
        match family {
            "bounty" => self.bounties(rng, n, sch),
            "tactic" => self.tactics(rng, n),
            "shop" | "supply" => {
                if family == "shop" {
                    if let Some(cards) = self.shop(rng, n) {
                        return cards;
                    }
                }
                let tiers = list(&sch["supplyTiers"]);
                let (lo, hi) = if family == "supply" && tiers.len() == 2 {
                    (num(&tiers[0], 1.) as i64, num(&tiers[1], 6.) as i64)
                } else {
                    (1, 6)
                };
                let mut items = self.items(lo, hi);
                if items.is_empty() {
                    items = self.items(1, 6);
                }
                (0..n)
                    .filter_map(|_| rng.pick(&items).map(|id| self.item_card(id)))
                    .collect()
            }
            _ => vec![],
        }
    }
}
/// Generate only at a scheduled SP round. Failure preserves the caller's RNG.
/// Availability filters are pure reads of the current match pools.
pub fn generate(data: &Catalog, rng: &mut Rng, ctx: &Context<'_>) -> Result<Option<Value>, String> {
    let choices = data.table("choices").ok_or("Missing choices")?;
    let mode = &data.table("config").ok_or("Missing config")?["modes"][ctx.mode];
    if mode["type"] != "SINGLE" {
        return Err("Draft requires a single-player mode".into());
    }
    let schedule = &choices["schedule"][ctx.mode];
    if !list(&mode["spRounds"]).contains(&json!(ctx.round)) {
        return Ok(None);
    }
    validate(choices, 0, &mut 100_000)?;
    let fallback = json!({"families":[{"family":"supply","weight":1}],"cards":3,"supplyTiers":[1,6.min(1+ctx.round/3)]});
    let sch = schedule["rounds"]
        .get(ctx.round.to_string())
        .filter(|v| v.is_object())
        .unwrap_or(&fallback);
    let mut next = rng.clone();
    let fams = list(&sch["families"]);
    let mut family = pick(
        &mut next,
        &fams
            .iter()
            .map(|f| num(&f["weight"], 0.))
            .collect::<Vec<_>>(),
    )
    .and_then(|i| fams[i]["family"].as_str())
    .unwrap_or("supply")
    .to_owned();
    // A missing family list still performs the original one-entry weighted draw.
    if fams.is_empty() {
        next.next_f64();
    }
    let n = sch["cards"]
        .as_u64()
        .filter(|n| *n > 0)
        .map(|n| n.min(6) as usize)
        .unwrap_or_else(|| {
            choices["format"]["solo"]["cards"]
                .as_u64()
                .unwrap_or(3)
                .min(6) as usize
        });
    let d = Data {
        catalog: data,
        choices,
        ctx,
    };
    let mut cards = d.cards(&mut next, &family, n, sch);
    if cards.is_empty() && family != "supply" {
        family = "supply".into();
        cards = d.cards(&mut next, &family, n, sch);
    }
    let result = if cards.is_empty() {
        None
    } else {
        for (i, c) in cards.iter_mut().enumerate() {
            c["idx"] = json!(i);
            c["family"] = json!(family);
        }
        let info = &choices["families"][&family];
        let event = next
            .pick(list(&sch["events"][&family]))
            .cloned()
            .unwrap_or(Value::Null);
        let name = info["name"].as_str().unwrap_or(match family.as_str() {
            "bounty" => "悬赏决策",
            "supply" => "道具补给",
            "shop" => "机密商店",
            "tactic" => "战术决策",
            _ => &family,
        });
        Some(
            json!({"family":family,"name":name,"desc":info["desc"].as_str().unwrap_or(""),"eventId":event,"cards":cards}),
        )
    };
    *rng = next;
    Ok(result)
}
fn validate(v: &Value, depth: usize, left: &mut usize) -> Result<(), String> {
    if depth > 16 || *left == 0 {
        return Err("Draft definition capacity exceeded".into());
    }
    *left -= 1;
    match v {
        Value::Array(a) => {
            if a.len() > 4096 {
                return Err("Draft list capacity exceeded".into());
            }
            for v in a {
                validate(v, depth + 1, left)?;
            }
        }
        Value::Object(m) => {
            for (k, v) in m {
                if ["count", "size", "slots", "open", "perSeries", "maxSeries16"]
                    .contains(&k.as_str())
                    && v.as_u64().is_some_and(|n| n > 64)
                {
                    return Err("Draft draw capacity exceeded".into());
                }
                validate(v, depth + 1, left)?;
            }
        }
        _ => (),
    }
    Ok(())
}
