// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn id(c: &Value) -> &str {
    c["effectId"].as_str().unwrap_or("")
}
fn contains(cards: &[&Value], c: &Value) -> bool {
    cards.iter().any(|x| id(x) == id(c))
}
fn by_ids<'a>(ids: &Value, all: &[&'a Value]) -> Vec<&'a Value> {
    list(ids)
        .iter()
        .filter_map(|v| all.iter().find(|c| c["effectId"] == *v).copied())
        .collect()
}
fn initial<'a>(rng: &mut Rng, rule: &Value, all: &[&'a Value]) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let per = rule["perSeries"].as_u64().unwrap_or(1) as usize;
    let prefer = list(&rule["prefer"]);
    let pref_tiers: Vec<_> = all
        .iter()
        .filter(|c| prefer.contains(&c["effectId"]))
        .map(|c| &c["tier"])
        .collect();
    let mut tiers = list(&rule["tiers"]).to_vec();
    rng.shuffle(&mut tiers);
    let ordered: Vec<_> = tiers
        .iter()
        .filter(|t| pref_tiers.contains(t))
        .chain(tiers.iter().filter(|t| !pref_tiers.contains(t)))
        .collect();
    for tier in ordered {
        let candidates: Vec<_> = all
            .iter()
            .copied()
            .filter(|c| {
                !contains(&out, c)
                    && c["tier"] == *tier
                    && list(&rule["series"]).contains(&c["series"])
                    && out.iter().filter(|x| x["series"] == c["series"]).count() < per
            })
            .collect();
        let preferred: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|c| prefer.contains(&c["effectId"]))
            .collect();
        if let Some(c) = rng.pick(if preferred.is_empty() {
            &candidates
        } else {
            &preferred
        }) {
            out.push(*c);
        }
    }
    out
}
fn hunter<'a>(
    rng: &mut Rng,
    rule: &Value,
    mut out: Vec<&'a Value>,
    all: &[&'a Value],
    size: usize,
) -> Vec<&'a Value> {
    let giants = by_ids(&rule["giants"], all);
    if out.len() < size && !out.iter().any(|c| contains(&giants, c)) {
        let candidates: Vec<_> = giants
            .iter()
            .copied()
            .filter(|c| !contains(&out, c))
            .collect();
        if let Some(c) = rng.pick(&candidates) {
            out.push(*c);
        }
    }
    let mut candidates = by_ids(&rule["cards"], all);
    rng.shuffle(&mut candidates);
    for c in candidates {
        if out.len() >= size {
            break;
        }
        let fits = if list(&rule["onePerSeries"]).contains(&c["series"]) {
            !out.iter().any(|x| x["series"] == c["series"])
        } else {
            c["series"] != 16
                || out
                    .iter()
                    .filter(|x| x["series"] == 16 && !contains(&giants, x))
                    .count()
                    < rule["maxSeries16"].as_u64().unwrap_or(2) as usize
        };
        if !contains(&out, c) && fits {
            out.push(c);
        }
    }
    out
}
fn structured<'a>(rng: &mut Rng, kind: &str, spec: &Value, all: &[&'a Value]) -> Vec<&'a Value> {
    let count = spec["count"].as_u64().unwrap_or(6) as usize;
    let groups = list(&spec["groups"]);
    let group = if spec["pick"] == "seen" {
        pick(
            rng,
            &groups
                .iter()
                .map(|g| {
                    list(&g["seen"])
                        .iter()
                        .filter(|v| v.as_i64().is_some())
                        .count()
                        .max(1) as f64
                })
                .collect::<Vec<_>>(),
        )
        .map(|i| &groups[i])
    } else {
        let slots = groups
            .len()
            .max(spec["slots"].as_u64().unwrap_or(0) as usize);
        let slot = if slots > 0 {
            rng.int(slots as u32) as usize
        } else {
            0
        };
        groups.get(slot)
    };
    let rule = &spec["rule"];
    let mut weights = Vec::new();
    let mut cards = if let Some(group) = group {
        let mut cards = Vec::new();
        for (i, key) in list(&group["cards"]).iter().enumerate() {
            if let Some(c) = all.iter().find(|c| c["effectId"] == *key) {
                cards.push(*c);
                weights.push(num(&group["weights"][i], 1.));
            }
        }
        if kind == "hunter" && num(&group["open"], 0.) > 0. {
            let size = cards.len() + num(&group["open"], 0.) as usize;
            cards = hunter(rng, rule, cards, all, size);
        }
        cards
    } else if kind == "initial" {
        initial(rng, rule, all)
    } else if kind == "hunter" {
        hunter(
            rng,
            rule,
            vec![],
            all,
            rule["size"].as_u64().unwrap_or(7) as usize,
        )
    } else {
        vec![]
    };
    weights.resize(cards.len(), 1.);
    for w in &mut weights {
        if *w == 0. {
            *w = 1.;
        }
        *w = w.max(0.);
    }
    let mut out = Vec::new();
    while out.len() < count && !cards.is_empty() {
        let i = pick(rng, &weights).unwrap();
        out.push(cards.remove(i));
        weights.remove(i);
    }
    out
}
fn text(v: &Value, multi: bool) -> Value {
    if !multi {
        return v.clone();
    }
    let Some(s) = v.as_str() else {
        return v.clone();
    };
    let mut s = s.to_owned();
    for prefix in ["之后的", "后续的", "后续"] {
        s = s.replace(
            &format!("{prefix}<@ba.vdown>每场</>作战"),
            "接下来<@ba.vup>两场作战</>",
        );
        s = s.replace(&format!("{prefix}每场作战"), "接下来两场作战");
    }
    json!(s)
}
impl Data<'_> {
    pub(super) fn bounties(&self, rng: &mut Rng, n: usize, sch: &Value) -> Vec<Value> {
        let kind = sch["bountyDraft"]
            .as_str()
            .unwrap_or(if self.ctx.round < 8 {
                "initial"
            } else if self.ctx.round < 11 {
                "boss"
            } else {
                "hunter"
            });
        let all: Vec<_> = list(&self.choices["cards"]["bounty"])
            .iter()
            .filter(|c| {
                (!c["draftPool"].is_string() || c["draftPool"] == kind)
                    && c["draft"].as_bool().unwrap_or(c["payout"] != "perfect")
                    && self.record("enemies", &c["enemyKey"]).is_object()
            })
            .collect();
        let spec = &self.choices["bountyDrafts"][kind];
        let mut out = if spec.is_object() {
            structured(rng, kind, spec, &all)
        } else {
            vec![]
        };
        if out.len() < n {
            let mut candidates = all.clone();
            rng.shuffle(&mut candidates);
            for c in candidates {
                if out.len() >= n {
                    break;
                }
                if !contains(&out, c) {
                    out.push(c);
                }
            }
        }
        rng.shuffle(&mut out);
        out.truncate(n);
        out.into_iter().map(|c|{
            let multi=c["multiRound"]==true||num(&c["rounds"],0.)>=90.;
            let rounds=if multi{2}else{c["rounds"].as_u64().unwrap_or(1).clamp(1,99)};
            let desc=json!(c["desc"].as_str().unwrap_or(""));
            json!({"kind":"bounty","id":c["effectId"],"name":c["name"],"desc":text(&desc,multi),"descRaw":text(&self.record("effects",&c["effectId"])["descRaw"],multi),"tier":c["tier"],"coin":c["coin"],"payout":c["payout"],"rounds":rounds,"enemyKey":c["enemyKey"],"count":c["count"]})
        }).collect()
    }
}
