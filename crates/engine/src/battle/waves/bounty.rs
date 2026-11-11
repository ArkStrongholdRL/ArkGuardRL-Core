// SPDX-License-Identifier: GPL-3.0-or-later
//! Original action windows needed when bounties join resolved wave actions.
use super::*;
#[derive(Clone, Debug, Default)]
pub(super) struct Layout {
    ground: Option<Host>,
    fly: Option<Host>,
}
#[derive(Clone, Debug)]
struct Host {
    index: usize,
    time: f64,
    window: f64,
    route: Option<usize>,
    own: usize,
}
impl Template {
    /// Attach original template windows and the generator's resolved action counts.
    /// Legacy explicit waves retain the source's no-template fallback.
    pub fn configure_bounty_actions(
        &mut self,
        original: &Value,
        actions: &[Value],
        slots: &Value,
        capacity: usize,
    ) -> Result<(), String> {
        let list = original["spawns"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if list.len() > capacity || actions.len() > capacity {
            return Err("Wave action capacity exceeded".into());
        }
        let make = |fly: bool| -> Result<Option<Host>, String> {
            let spawn = |s: &&Value| {
                s.is_object()
                    && s.get("action").is_none_or(|a| {
                        a.is_null() || a.as_str().is_some_and(|a| a.eq_ignore_ascii_case("SPAWN"))
                    })
            };
            let want = &slots[if fly { "NF" } else { "N" }];
            let selected = list
                .iter()
                .enumerate()
                .find(|(_, s)| spawn(s) && first(s, &["key", "enemyKey"]) == want)
                .or_else(|| list.iter().enumerate().find(|(_, s)| spawn(s)));
            let Some((index, s)) = selected else {
                return Ok(None);
            };
            let count = s["count"].as_u64().unwrap_or(1).max(1);
            if count > 50 {
                return Err("Original wave action unit capacity exceeded".into());
            }
            let time = number(&s["time"], 0.).max(0.);
            let window = count as f64 * number(&s["interval"], 0.).max(0.);
            if !window.is_finite() {
                return Err("Invalid wave action window".into());
            }
            let own = actions
                .iter()
                .find(|a| a["index"].as_u64() == Some(index as u64))
                .filter(|a| a["valid"] == true && a["server"] != false)
                .map_or(0, |a| a["count"].as_u64().unwrap_or(0).min(50) as usize);
            Ok(Some(Host {
                index,
                time,
                window,
                route: s["routeIndex"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok()),
                own,
            }))
        };
        self.bounty_layout = Layout {
            ground: make(false)?,
            fly: make(true)?,
        };
        Ok(())
    }
    /// Re-time normal-wave host units around bounty insertions. Boss waves keep
    /// their original spawn list, as bountySpawns does in the source boss owner.
    pub fn insert_bounties(
        &mut self,
        additions: Vec<Value>,
        boss: bool,
        capacity: usize,
    ) -> Result<(), String> {
        if additions.is_empty() {
            return Ok(());
        }
        if additions.len() > capacity || self.spawns.len() > capacity {
            return Err("Bounty spawn capacity exceeded".into());
        }
        let mut groups: Vec<(Option<Host>, Vec<usize>)> = Vec::new();
        for (i, s) in additions.iter().enumerate() {
            let fly = matches!(s["mods"]["slot"].as_str(), Some("NF" | "EF"));
            let host = if fly {
                &self.bounty_layout.fly
            } else {
                &self.bounty_layout.ground
            };
            let index = host.as_ref().map(|h| h.index);
            let group = groups
                .iter()
                .position(|(h, _)| h.as_ref().map(|h| h.index) == index);
            let at = group.unwrap_or_else(|| {
                groups.push((host.clone(), Vec::new()));
                groups.len() - 1
            });
            groups[at].1.push(i);
        }
        let mut retimed = std::collections::BTreeMap::new();
        let mut extras = Vec::new();
        for (host, ids) in groups {
            let own = host.as_ref().map_or(0, |h| h.own);
            let time = host.as_ref().map_or(4., |h| h.time);
            let window = host.as_ref().map_or(0., |h| h.window);
            let mut added = Vec::new();
            for id in ids {
                let n = additions[id]["count"].as_u64().unwrap_or(1).clamp(1, 20) as usize;
                if added.len() + n > capacity.saturating_mul(20) {
                    return Err("Bounty unit capacity exceeded".into());
                }
                added.extend(std::iter::repeat_n(Some(id), n));
            }
            let mut order = vec![None; own];
            if own == 0 {
                order = added;
            } else {
                let total = added.len();
                for (i, id) in added.into_iter().enumerate() {
                    let at = ((i + 1) * order.len()) / (total + 1);
                    order.insert(at, id);
                }
            }
            let step = if window > 0. {
                (window / order.len() as f64).max(0.05 * window)
            } else {
                0.
            };
            let mut own_runs = Vec::new();
            let mut start = 0;
            while start < order.len() {
                let mut end = start + 1;
                while end < order.len() && order[end] == order[start] {
                    end += 1;
                }
                let count = end - start;
                let at = time + start as f64 * step;
                if !at.is_finite() {
                    return Err("Bounty spawn time exceeds finite domain".into());
                }
                if let Some(id) = order[start] {
                    let mut s = additions[id].clone();
                    s["time"] = json!(at);
                    s["count"] = json!(count);
                    s["interval"] = json!(if count > 1 { step } else { 0. });
                    if let Some(h) = &host {
                        s["actionIndex"] = json!(h.index);
                        if let Some(route) = h.route.filter(|i| {
                            self.routes.get(*i).is_some_and(|r| {
                                let fly = matches!(s["mods"]["slot"].as_str(), Some("NF" | "EF"));
                                (if fly {
                                    r["motion"] == "FLY"
                                } else {
                                    r["motion"] != "FLY" && r["motion"] != "E_NUM"
                                }) && (!boss || r["end"][1].as_f64().is_some_and(|x| x < 10.))
                            })
                        }) {
                            s["routeIndex"] = json!(route);
                        }
                    }
                    extras.push(s);
                } else {
                    own_runs.push((at, count, if count > 1 { step } else { 0. }));
                }
                start = end;
            }
            if let Some(h) = host.filter(|_| own > 0 && !boss) {
                retimed.insert(h.index, own_runs);
            }
        }
        let mut output = Vec::new();
        for s in &self.spawns {
            let runs = s["actionIndex"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| retimed.get(&n))
                .filter(|_| s["tag"] != "bounty");
            if let Some(runs) = runs {
                for (time, count, interval) in runs {
                    let mut s = s.clone();
                    s["time"] = json!(time);
                    s["count"] = json!(count);
                    s["interval"] = json!(interval);
                    output.push(s);
                }
            } else {
                output.push(s.clone());
            }
            if output.len() + extras.len() > capacity {
                return Err("Bounty spawn capacity exceeded".into());
            }
        }
        output.extend(extras);
        if output.len() > capacity {
            return Err("Bounty spawn capacity exceeded".into());
        }
        self.spawns = output;
        Ok(())
    }
}
