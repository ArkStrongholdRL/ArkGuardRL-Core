// SPDX-License-Identifier: GPL-3.0-or-later
//! Deterministic single-player setup and resolved round plans. Setup diagnostics
//! include future waves and must not be exposed as a policy observation.
use crate::{
    data::Catalog,
    deterministic::rng::{derive_seed, Rng},
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
mod waves;
const ACLOON: &str = "enemy_9012_acloon";
const CAP: usize = 4096;
pub struct Generated {
    pub input: Value,
    pub setup: Value,
}
fn n(v: &Value, d: f64) -> f64 {
    v.as_f64().filter(|x| x.is_finite()).unwrap_or(d)
}
fn positive(v: &Value, d: usize) -> usize {
    v.as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(d)
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn weighted(rng: &mut Rng, pairs: &[(String, f64)]) -> Option<String> {
    let w: Vec<_> = pairs.iter().map(|(_, w)| *w).collect();
    rng.weighted(&w)
        .or_else(|| (!pairs.is_empty()).then_some(0))
        .map(|i| pairs[i].0.clone())
}
struct Data<'a> {
    data: &'a Catalog,
    cfg: &'a Value,
    mode: &'a Value,
    factions: &'a Value,
}
impl Data<'_> {
    fn table(&self, key: &str) -> &Value {
        self.data.table(key).unwrap()
    }
    fn enemy(&self, key: &str) -> &Value {
        &self.table("enemies")[key]
    }
    fn fly(&self, key: &str) -> bool {
        let e = self.enemy(key);
        e["isFlyEnemy"]
            .as_bool()
            .unwrap_or(e["stats"]["motion"] == "FLY")
    }
    fn pick(&self, rng: &mut Rng, kind: &str, round: usize) -> Value {
        let g = &self.factions["generation"];
        let half = round <= positive(&g["firstHalfMaxRound"], positive(&g["maxLevelCnt"], 15) / 2);
        let mut entries: Vec<_> = self.factions["entries"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter(|e| {
                e["type"] == kind
                    && (e["firstHalf"] == true) == half
                    && e["key"]
                        .as_str()
                        .is_some_and(|k| k != ACLOON && self.enemy(k).is_object())
                    && !list(&self.mode["inactiveEnemyKeys"]).contains(&e["key"])
            })
            .collect();
        entries.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        if entries.is_empty() {
            return if kind != "SPECIAL" {
                self.pick(rng, "SPECIAL", round)
            } else {
                Value::Null
            };
        }
        let weights: Vec<_> = entries.iter().map(|e| n(&e["weight"], 1.)).collect();
        let e = entries[rng.weighted(&weights).unwrap_or(0)];
        let mut attached = |field| {
            let keys: Vec<_> = list(&e[field])
                .iter()
                .filter_map(|v| v.as_str().or(v["key"].as_str()))
                .filter(|k| *k != ACLOON && self.enemy(k).is_object())
                .collect();
            rng.pick(&keys).copied().map(str::to_owned)
        };
        let normal = attached("N");
        let elite = attached("E");
        json!({"round":round,"type":e["type"],"key":e["key"],"normal":normal,"elite":elite,"fly":e["fly"].as_bool().unwrap_or_else(||self.fly(e["key"].as_str().unwrap())),"firstHalf":half})
    }
}
/// Generate the legacy solo CLI input shape, plus private setup diagnostics.
/// Manual bans affect only the final pool list, never any RNG stream.
pub fn generate(
    data: &Catalog,
    seed: u32,
    mode_id: &str,
    manual_banned: &[String],
) -> Result<Generated, String> {
    generate_at_revision(data, seed, mode_id, manual_banned, 0)
}
pub(super) fn generate_at_revision(
    data: &Catalog,
    seed: u32,
    mode_id: &str,
    manual_banned: &[String],
    revision: u32,
) -> Result<Generated, String> {
    let cfg = data.table("config").ok_or("Missing config")?;
    let mode = &cfg["modes"][mode_id];
    if !mode.is_object() || mode["type"] != "SINGLE" {
        return Err("Expected a single-player mode".into());
    }
    if manual_banned.len() > CAP {
        return Err("Manual ban capacity exceeded".into());
    }
    let d = Data {
        data,
        cfg,
        mode,
        factions: data.table("factions").ok_or("Missing factions")?,
    };
    let mut manual = BTreeSet::new();
    for key in manual_banned {
        let c = &d.table("chess")[key];
        if !c.is_object() {
            return Err(format!("Unknown banned chess: {key}"));
        }
        manual.insert(c["baseId"].as_str().unwrap_or(key).to_owned());
    }
    let seed = seed.max(1);
    let salt = if revision == 0 {
        "setup".into()
    } else {
        format!("reroll-setup:{revision}")
    };
    let mut rng = Rng::new(derive_seed(seed, &salt));
    let stages: Vec<_> = list(&mode["stages"])
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let s = &d.table("stages")[id];
            (s.is_object() && s["active"] != false && n(&s["weight"], 0.) > 0.)
                .then(|| (id.into(), n(&s["weight"], 0.)))
        })
        .collect();
    let stage = weighted(&mut rng, &stages)
        .or_else(|| {
            d.table("stages")
                .as_object()
                .into_iter()
                .flat_map(|m| m.iter())
                .filter(|(_, s)| s["active"] == true)
                .map(|(id, _)| id.clone())
                .min()
        })
        .ok_or("No active stage")?;
    let types = &d.factions["types"];
    let g = &d.factions["generation"];
    let mut chosen: Vec<String> = types
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter(|t| t["involveRandom"] == true)
        .filter_map(|t| t["type"].as_str().map(str::to_owned))
        .collect();
    if chosen.len() > CAP {
        return Err("Faction capacity exceeded".into());
    }
    chosen.sort();
    rng.shuffle(&mut chosen);
    chosen.truncate(
        g["specialEnemyNum"]
            .as_u64()
            .unwrap_or(3)
            .min(chosen.len() as u64) as usize,
    );
    let mut factions = chosen.clone();
    factions.sort_by(|a, b| n(&types[a]["sortId"], 9.).total_cmp(&n(&types[b]["sortId"], 9.)));
    let boss_pairs = |field: &str| {
        mode[field]
            .as_object()
            .into_iter()
            .flat_map(|m| m.iter())
            .filter(|(id, w)| d.table("bosses")[*id].is_object() && n(w, 0.) > 0.)
            .map(|(id, w)| (id.clone(), n(w, 0.)))
            .collect::<Vec<_>>()
    };
    let boss = weighted(&mut rng, &boss_pairs("bossWeights")).ok_or("No eligible boss")?;
    let hidden_round = mode["hiddenRound"]
        .as_u64()
        .filter(|r| *r > 0)
        .map(|r| r as usize);
    let hidden = if hidden_round.is_some() {
        weighted(&mut rng, &boss_pairs("hiddenBossWeights"))
    } else {
        None
    };
    let mut schedule_rng = Rng::new(derive_seed(rng.state(), "waves:schedule"));
    let max = positive(&g["maxLevelCnt"], 15).min(256);
    let fill = g["fillType"]
        .as_str()
        .or(g["alwaysIncludedType"].as_str())
        .unwrap_or("SPECIAL");
    let mut schedule = Vec::new();
    for kind in &chosen {
        for _ in 0..positive(&types[kind]["count"], 3).min(max - schedule.len()) {
            schedule.push(kind.clone());
        }
    }
    schedule.resize(max, fill.into());
    schedule_rng.shuffle(&mut schedule);
    let mut picks = vec![Value::Null];
    for (i, kind) in schedule.iter().enumerate() {
        picks.push(d.pick(&mut schedule_rng, kind, i + 1));
    }
    // Automatic bond bans use the setup stream after setupMatchWaves, not the schedule stream.
    let difficulty = mode["difficulty"].as_str().unwrap_or("NORMAL");
    let bonds = d.table("bonds");
    let mut bond_ids: Vec<_> = bonds
        .as_object()
        .into_iter()
        .flat_map(|m| m.keys())
        .cloned()
        .collect();
    bond_ids.sort_by(|a, b| {
        n(&bonds[a]["identifier"], 99.)
            .total_cmp(&n(&bonds[b]["identifier"], 99.))
            .then(a.cmp(b))
    });
    let mut static_off: Vec<_> = list(&mode["inactiveBondIds"])
        .iter()
        .filter_map(Value::as_str)
        .filter(|b| bonds[*b].is_object())
        .map(str::to_owned)
        .collect();
    static_off.sort();
    let mut drawn = Vec::new();
    for (core, key, default) in [
        (true, "core", if difficulty == "FUNNY" { 2 } else { 3 }),
        (false, "addon", 4),
    ] {
        let mut eligible: Vec<_> = bond_ids
            .iter()
            .filter(|id| {
                n(&bonds[*id]["weight"], 0.) > 0.
                    && !static_off.contains(id)
                    && (bonds[*id]["isCore"] == true) == core
            })
            .cloned()
            .collect();
        rng.shuffle(&mut eligible);
        eligible.truncate(
            cfg["bans"][difficulty][key]
                .as_u64()
                .unwrap_or(default)
                .min(eligible.len() as u64) as usize,
        );
        drawn.extend(eligible);
    }
    drawn.sort();
    let off: BTreeSet<_> = drawn.iter().chain(&static_off).cloned().collect();
    let mut automatic = Vec::new();
    for (id, c) in d.table("chess").as_object().ok_or("Invalid chess table")? {
        if c["visible"] == true
            && c["isGolden"] != true
            && c["isHidden"] != true
            && c["isDiy"] != true
            && !list(&c["bonds"]).is_empty()
            && list(&c["bonds"])
                .iter()
                .all(|b| b.as_str().is_some_and(|b| off.contains(b)))
        {
            automatic.push(id.clone());
        }
    }
    automatic.sort();
    let banned: BTreeSet<_> = automatic
        .iter()
        .cloned()
        .chain(manual.iter().cloned())
        .collect();
    let boss_round = positive(
        &mode["bossRound"],
        positive(
            &mode["lastRound"],
            if mode_id == "mode_single_funny" {
                9
            } else {
                14
            },
        ),
    );
    let last = hidden_round
        .filter(|_| hidden.is_some())
        .unwrap_or(boss_round);
    if last > 64 || boss_round >= last + 1 || hidden_round.is_some_and(|r| r != boss_round + 1) {
        return Err("Invalid mode round schedule".into());
    }
    let speed = n(&cfg["combatTimeScale"], 2.);
    if speed <= 0. {
        return Err("Invalid combat time scale".into());
    }
    let mut plans = Vec::new();
    for round in 1..=last {
        let is_boss = round >= boss_round;
        let boss_id = if round > boss_round {
            hidden.as_deref().unwrap()
        } else {
            &boss
        };
        plans.push(d.plan(
            round,
            picks.get(round).unwrap_or(&Value::Null),
            is_boss,
            boss_id,
            speed,
        )?);
    }
    let eligible = hidden.is_some()
        && list(&cfg["hiddenCore"]["difficulties"])
            .iter()
            .any(|v| v == difficulty);
    let hidden_rule=eligible.then(||json!({"layerThreshold":n(&cfg["hiddenCore"]["single"],350.),"minLp":n(&cfg["hiddenCore"]["minTeamLpExclusive"],1.)}));
    let setup = json!({"stageId":stage,"factions":factions,"bossId":boss,"hiddenBossId":hidden,"typeSlots":schedule,"picks":picks,"disabledBonds":drawn,"staticInactiveBonds":static_off,"automaticBanned":automatic,"manualBanned":manual,"banned":banned,"rng":rng.state()});
    // Random bond bans restrict the pool; surviving members still activate their bonds.
    let input = json!({"seed":seed,"stage":d.table("stages")[&stage],"config":{"lp":n(&cfg["economy"]["defaultStartLp"],28.),"lpCap":cfg["lpCapPerRound"].as_u64().filter(|v|*v>0).unwrap_or(10),"keepFunds":cfg["economy"]["leftoverFundsLost"]==false,"hidden":hidden_rule},"preparation":{"modeId":mode_id,"banned":banned,"effects":{"disabledBonds":[]}},"plans":plans});
    Ok(Generated { input, setup })
}
