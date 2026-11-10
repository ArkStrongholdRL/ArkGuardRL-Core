// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 simdata.js and battle/spawns.js.
// These are engine inputs, never policy observations (future waves are private).
mod bounty;
use bounty::Layout;
use serde_json::{json, Value};
use std::collections::VecDeque;

pub(crate) fn first<'a>(v: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| !x.is_null()))
        .unwrap_or(&Value::Null)
}
pub(crate) fn number(v: &Value, fallback: f64) -> f64 {
    v.as_f64()
        .or_else(|| {
            v.as_str()
                .filter(|s| !s.trim().is_empty())
                .and_then(|s| s.trim().parse().ok())
        })
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}
fn pair(v: &Value) -> Value {
    if v.is_array() {
        json!([number(&v[0], 0.), number(&v[1], 0.)])
    } else if v.is_object() {
        json!([
            number(first(v, &["row", "r"]), 0.),
            number(first(v, &["col", "c"]), 0.)
        ])
    } else {
        Value::Null
    }
}
fn cp_type(v: &Value) -> &'static str {
    let s = v.as_str().unwrap_or("MOVE").to_uppercase();
    if s.contains("DISAPPEAR") {
        "DISAPPEAR"
    } else if s.contains("APPEAR") {
        "APPEAR"
    } else if s.contains("WAIT") {
        "WAIT"
    } else {
        "MOVE"
    }
}
pub(crate) fn has_patrol(route: &Value) -> bool {
    route["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["t"] == "patrol")
}
pub fn normalize_route(v: &Value, max_legs: usize) -> Result<Value, String> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    let mut cps = Vec::new();
    let steps = v["steps"].as_array().filter(|a| !a.is_empty());
    let raw = steps.or_else(|| first(v, &["checkpoints", "cp"]).as_array());
    if let Some(raw) = raw {
        if raw.len() > max_legs {
            return Err("Route checkpoint capacity exceeded".into());
        }
        for c in raw {
            let (kind, pos, time) = if steps.is_some() {
                (
                    cp_type(first(c, &["t", "type"])),
                    pair(first(c, &["p", "pos"])),
                    number(first(c, &["s", "time"]), 0.),
                )
            } else if c.is_array() && c[0].is_number() {
                ("MOVE", json!([number(&c[0], 0.), number(&c[1], 0.)]), 0.)
            } else if c.is_array() {
                (
                    cp_type(&c[0]),
                    json!([number(&c[1], 0.), number(&c[2], 0.)]),
                    number(&c[3], 0.),
                )
            } else if c.is_object() {
                let pos = first(c, &["pos", "position"]);
                (
                    cp_type(&c["type"]),
                    pair(&if pos.is_null() {
                        json!([c["row"], c["col"]])
                    } else {
                        pos.clone()
                    }),
                    number(first(c, &["time", "t"]), 0.),
                )
            } else {
                continue;
            };
            cps.push(
                json!({"type":kind,"pos":if pos.is_null(){json!([0,0])}else{pos},"time":time}),
            );
        }
    }
    // Four official portal routes omit APPEAR; upstream inserts it before MOVE.
    let mut i = 0;
    while i < cps.len() {
        if cps[i]["type"] == "DISAPPEAR" {
            let mut j = i + 1;
            while j < cps.len() && cps[j]["type"] == "WAIT" {
                j += 1;
            }
            if j < cps.len() && cps[j]["type"] == "MOVE" {
                if cps.len() >= max_legs {
                    return Err("Route portal checkpoint capacity exceeded".into());
                }
                cps.insert(j, json!({"type":"APPEAR","pos":cps[j]["pos"],"time":0.}));
            }
        }
        i += 1;
    }
    Ok(
        json!({"motion":if first(v,&["motion","m"]).as_str().unwrap_or("W").to_uppercase().starts_with('F') {"FLY"}else{"WALK"},
        "start":pair(first(v,&["start","s","startPosition"])),"end":pair(first(v,&["end","e","endPosition"])),"checkpoints":cps}),
    )
}

/// Normalized wave data. Non-SPAWN actions are kept separately so a caller cannot
/// accidentally accept an unimplemented content action by dropping it.
#[derive(Clone)]
pub struct Template {
    bounty_layout: Layout,
    pub devices: Vec<Value>,
    pub patrol_routes: std::collections::BTreeSet<usize>,
    pub routes: Vec<Value>,
    pub spawns: Vec<Value>,
    pub extra_routes: Vec<Value>,
    pub branches: std::collections::BTreeMap<String, Vec<Vec<Value>>>,
    pub overrides: Value,
    pub max_play_time: Value,
    pub content_actions: Vec<Value>,
}
impl Template {
    pub fn parse(
        v: &Value,
        mods: &Value,
        tag: &Value,
        capacity: usize,
        max_legs: usize,
    ) -> Result<Self, String> {
        if capacity == 0 || max_legs == 0 {
            return Err("Wave limits must be positive".into());
        }
        let routes = |key: &str| -> Result<Vec<Value>, String> {
            let a = v[key].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if a.len() > capacity {
                return Err("Wave route capacity exceeded".into());
            }
            a.iter().map(|r| normalize_route(r, max_legs)).collect()
        };
        let devices = v["devices"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if devices.len() > capacity {
            return Err("Wave device capacity exceeded".into());
        }
        for device in devices {
            if let Some(pos) = device.get("pos") {
                if !pos.as_array().is_some_and(|p| {
                    p.len() == 2 && p.iter().all(|v| v.as_f64().is_some_and(|n| n.is_finite()))
                }) {
                    return Err("Invalid wave device position".into());
                }
            }
        }
        let mut out = Self {
            bounty_layout: Layout::default(),
            devices: devices.to_vec(),
            routes: routes("routes")?,
            patrol_routes: v["routes"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(i, r)| has_patrol(r).then_some(i))
                .collect(),
            spawns: vec![],
            extra_routes: routes("extraRoutes")?,
            branches: Default::default(),
            overrides: v["overrides"].clone(),
            max_play_time: first(v, &["maxPlayTime", "timeLimit"]).clone(),
            content_actions: vec![],
        };
        if let Some(branches) = v["branches"].as_object() {
            if branches.len() > capacity {
                return Err("Wave branch capacity exceeded".into());
            }
            let mut entries = 0usize;
            let mut phase_count = 0usize;
            for (name, phases) in branches {
                let phases = phases.as_array().ok_or("Expected branch phases")?;
                phase_count = phase_count
                    .checked_add(phases.len())
                    .ok_or("Branch phase capacity overflow")?;
                if phase_count > capacity {
                    return Err("Branch phase capacity exceeded".into());
                }
                let mut parsed = vec![];
                for phase in phases {
                    let phase = phase.as_array().ok_or("Expected branch phase actions")?;
                    entries = entries
                        .checked_add(phase.len())
                        .ok_or("Branch capacity overflow")?;
                    if entries > capacity {
                        return Err("Branch action capacity exceeded".into());
                    }
                    parsed.push(phase.clone());
                }
                out.branches.insert(name.clone(), parsed);
            }
        }
        if let Some(a) = v["spawns"].as_array() {
            if a.len() > capacity {
                return Err("Wave action capacity exceeded".into());
            }
            for s in a {
                if s["action"]
                    .as_str()
                    .map(|s| !s.eq_ignore_ascii_case("SPAWN"))
                    .unwrap_or(false)
                {
                    out.content_actions.push(s.clone());
                    continue;
                }
                let key = first(s, &["enemyKey", "key"]);
                if key.is_null() || key == "" {
                    continue;
                }
                let mut p = json!({"time":number(first(s,&["time","t"]),0.),"enemyKey":key,
                    "routeIndex":number(first(s,&["routeIndex","route"]),0.),"count":number(first(s,&["count","n"]),1.).max(1.),
                    "interval":number(first(s,&["interval","int"]),0.)});
                for (k, fallback) in [("mods", mods), ("tag", tag)] {
                    let value = s.get(k).filter(|x| !x.is_null()).unwrap_or(fallback);
                    if !value.is_null() {
                        p[k] = value.clone();
                    }
                }
                for k in ["bounty", "sourcePlayerId", "group"] {
                    if let Some(x) = s.get(k).filter(|x| !x.is_null()) {
                        p[k] = x.clone();
                    }
                }
                if s["unharmful"] == true || p["tag"] == "part" {
                    p["countInTotal"] = json!(false);
                }
                if s["hidden"] == true {
                    p["hidden"] = json!(true);
                }
                out.spawns.push(p);
            }
        } else {
            let mut wave_t = 0.;
            for w in v["waves"].as_array().into_iter().flatten() {
                wave_t += number(&w["preDelay"], 0.);
                let mut frag_t = wave_t;
                let mut frag_end = frag_t;
                for f in w["fragments"].as_array().into_iter().flatten() {
                    frag_t += number(&f["preDelay"], 0.);
                    for a in f["actions"].as_array().into_iter().flatten() {
                        let key = first(a, &["key", "enemyKey"]);
                        if !key
                            .as_str()
                            .map(|s| s.starts_with("enemy_"))
                            .unwrap_or(false)
                        {
                            out.content_actions.push(a.clone());
                            if out.content_actions.len() > capacity {
                                return Err("Wave content action capacity exceeded".into());
                            }
                            continue;
                        }
                        if out.spawns.len() >= capacity {
                            return Err("Wave spawn capacity exceeded".into());
                        }
                        let n = number(first(a, &["n", "count"]), 1.).max(1.);
                        let t = frag_t + number(first(a, &["pre", "preDelay"]), 0.);
                        let interval = number(first(a, &["int", "interval"]), 0.);
                        let mut p = json!({"time":t,"enemyKey":key,"routeIndex":number(first(a,&["route","routeIndex"]),0.),"count":n,"interval":interval});
                        if !mods.is_null() {
                            p["mods"] = mods.clone();
                        }
                        if !tag.is_null() {
                            p["tag"] = tag.clone();
                        }
                        out.spawns.push(p);
                        frag_end = frag_end.max(t + interval * (n - 1.));
                    }
                }
                wave_t = wave_t.max(frag_end) + number(&w["postDelay"], 0.);
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Debug)]
pub struct Pending {
    pub time: f64,
    pub sequence: u64,
    pub spec: Value,
    pub counted: bool,
    pub in_total: bool,
    pub precounted: bool,
}
pub struct Queue {
    pending: VecDeque<Pending>,
    sequence: u64,
    capacity: usize,
}
impl Queue {
    pub fn new(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("Spawn queue capacity must be positive".into());
        }
        Ok(Self {
            pending: VecDeque::new(),
            sequence: 0,
            capacity,
        })
    }
    pub fn pending(&self) -> &VecDeque<Pending> {
        &self.pending
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn next_sequence(&mut self) -> Result<u64, String> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Spawn sequence exhausted")?;
        Ok(self.sequence)
    }
    /// Append one schedule atomically. Explicit resource limits error instead of
    /// silently clipping the upstream count (the upstream itself caps it at 600).
    pub fn push(
        &mut self,
        s: &Value,
        precount: bool,
        not_count_in_total: bool,
    ) -> Result<(), String> {
        if !s.is_object() {
            return Err("Expected spawn object".into());
        }
        let count = number(&s["count"], 1.).floor().max(1.);
        if count > 600. || count > (self.capacity - self.pending.len()) as f64 {
            return Err("Spawn queue capacity exceeded; no enemies dropped".into());
        }
        let count = count as usize;
        let end = self
            .sequence
            .checked_add(count as u64)
            .ok_or("Spawn sequence exhausted")?;
        let base = number(&s["time"], 0.).max(0.);
        let interval = number(&s["interval"], 0.).max(0.);
        let counted = s["countInTotal"]
            .as_bool()
            .unwrap_or(!not_count_in_total && s["tag"] != "boss" && s["tag"] != "part");
        let mut items = Vec::with_capacity(count);
        for i in 0..count {
            let time = base + i as f64 * interval;
            if !time.is_finite() {
                return Err("Spawn time overflow".into());
            }
            let mut spec = s.clone();
            // Normalized single-enemy options for spawnEnemy, independent of count.
            spec["time"] = json!(time);
            for k in ["count", "interval"] {
                spec.as_object_mut()
                    .ok_or("Expected spawn object")?
                    .remove(k);
            }
            items.push(Pending {
                time,
                sequence: self.sequence + i as u64 + 1,
                spec,
                counted,
                in_total: counted,
                precounted: precount && counted,
            });
        }
        self.pending.extend(items);
        self.sequence = end;
        self.pending
            .make_contiguous()
            .sort_by(|a, b| a.time.total_cmp(&b.time).then(a.sequence.cmp(&b.sequence)));
        Ok(())
    }
    pub fn pop_due(&mut self, time: f64) -> Option<Pending> {
        if self
            .pending
            .front()
            .map(|p| p.time <= time + 1e-9)
            .unwrap_or(false)
        {
            self.pending.pop_front()
        } else {
            None
        }
    }
    pub fn drain(&mut self) -> impl Iterator<Item = Pending> + '_ {
        self.pending.drain(..)
    }
}
