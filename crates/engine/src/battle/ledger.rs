// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/deploy.js, lifecycle.js, economy.js.
// Solo field accounting; unit HP, skills and hooks retain their existing owners.
use super::{
    health::Id,
    waves::{Pending, Queue},
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct Enemy {
    pub key: String,
    pub mods: Value,
    pub lpr: f64,
    pub source_player: Value,
    pub tag: Value,
    pub counted: bool,
    pub in_total: bool,
    pub boss: bool,
    pub no_leak: bool,
    pub bounty: f64,
}
impl Enemy {
    pub fn from_pending(p: &Pending, lpr: f64, player: &Value) -> Result<Self, String> {
        if !lpr.is_finite() {
            return Err("Invalid enemy LP reduction".into());
        }
        Ok(Self {
            key: p.spec["enemyKey"]
                .as_str()
                .ok_or("Missing enemy key")?
                .into(),
            mods: p.spec["mods"].clone(),
            lpr,
            source_player: if p.spec["sourcePlayerId"].is_null() {
                player.clone()
            } else {
                p.spec["sourcePlayerId"].clone()
            },
            tag: p.spec["tag"].clone(),
            counted: p.counted,
            in_total: p.in_total,
            boss: p.spec["tag"] == "boss",
            no_leak: false,
            bounty: p.spec["bounty"]["coins"]
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.)
                .unwrap_or(0.),
        })
    }
}

pub struct Ledger {
    player: Value,
    alive: BTreeMap<Id, Enemy>,
    leaks: Vec<Value>,
    unspawned: Vec<Value>,
    capacity: usize,
    report_capacity: usize,
    pub total: u64,
    pub killed: u64,
    pub killed_in_total: u64,
    pub leaked_count: u64,
    pub leaked_in_total: u64,
    pub credited_kills: u64,
    pub coins: f64,
    timed_out: bool,
}
impl Ledger {
    pub fn new(
        player: Value,
        queue: &Queue,
        capacity: usize,
        report_capacity: usize,
    ) -> Result<Self, String> {
        if capacity == 0 || report_capacity == 0 || !player.is_string() {
            return Err("Invalid solo ledger limits/player".into());
        }
        Ok(Self {
            player,
            alive: BTreeMap::new(),
            leaks: vec![],
            unspawned: vec![],
            capacity,
            report_capacity,
            total: queue.pending().iter().filter(|p| p.precounted).count() as u64,
            killed: 0,
            killed_in_total: 0,
            leaked_count: 0,
            leaked_in_total: 0,
            credited_kills: 0,
            coins: 0.,
            timed_out: false,
        })
    }
    pub fn player(&self) -> &Value {
        &self.player
    }
    pub fn alive(&self) -> &BTreeMap<Id, Enemy> {
        &self.alive
    }
    pub fn resolved(&self) -> u64 {
        self.total.min(self.killed_in_total + self.leaked_in_total)
    }
    pub fn spawn(&mut self, id: Id, enemy: Enemy, precounted: bool) -> Result<(), String> {
        if self.timed_out {
            return Err("Cannot spawn after timeout".into());
        }
        if self.alive.len() >= self.capacity || self.alive.contains_key(&id) {
            return Err("Duplicate enemy or live enemy capacity exceeded".into());
        }
        if enemy.in_total && !precounted {
            self.total += 1;
        }
        self.alive.insert(id, enemy);
        Ok(())
    }
    pub fn kill(&mut self, id: Id, credited_to_ally: bool) -> Result<(), String> {
        if self.timed_out {
            return Err("Cannot kill after timeout".into());
        }
        let enemy = self
            .alive
            .remove(&id)
            .ok_or("Enemy already resolved or unknown")?;
        if enemy.counted {
            self.killed += 1;
        }
        if enemy.in_total {
            self.killed_in_total += 1;
        }
        if credited_to_ally {
            self.credited_kills += 1;
        }
        self.coins += enemy.bounty;
        Ok(())
    }
    fn leak_entry(e: &Enemy) -> Value {
        let mut v = json!({"enemyKey":e.key,"mods":e.mods,"lpr":e.lpr,"sourcePlayerId":e.source_player,
            "tag":e.tag,"counted":e.counted || e.boss,"spawned":true});
        if e.boss {
            v["boss"] = json!(true);
        }
        v
    }
    fn record_leak(&mut self, e: &Enemy) {
        self.leaks.push(Self::leak_entry(e));
        if e.counted {
            self.leaked_count += 1;
        }
        if e.in_total {
            self.leaked_in_total += 1;
        }
    }
    pub fn leak(&mut self, id: Id) -> Result<(), String> {
        if self.timed_out {
            return Err("Cannot leak after timeout".into());
        }
        if self.leaks.len() >= self.report_capacity {
            return Err("Leak report capacity exceeded; no entries dropped".into());
        }
        let e = self
            .alive
            .remove(&id)
            .ok_or("Enemy already resolved or unknown")?;
        self.record_leak(&e);
        Ok(())
    }
    /// Timeout billing does not kill/remove surviving bodies. A computation budget
    /// must *not* call this: truncation keeps pending spawns and real live state.
    pub fn timeout(&mut self, queue: &mut Queue) -> Result<(), String> {
        if self.timed_out {
            return Ok(());
        }
        let leaks = self
            .alive
            .values()
            .filter(|e| !e.boss && !e.no_leak)
            .count();
        if leaks > self.report_capacity - self.leaks.len()
            || queue.pending().len() > self.report_capacity
        {
            return Err("Timeout report capacity exceeded; state preserved".into());
        }
        // Ordered unit IDs are creation order, the same order as upstream enemies.
        let ids: Vec<_> = self
            .alive
            .iter()
            .filter(|(_, e)| !e.boss && !e.no_leak)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let e = self.alive.get(&id).unwrap().clone();
            self.record_leak(&e);
        }
        for p in queue.drain() {
            if p.precounted {
                self.total -= 1;
            }
            self.unspawned.push(json!({"enemyKey":p.spec["enemyKey"],"time":p.time,"tag":p.spec["tag"],"sourcePlayerId":p.spec["sourcePlayerId"]}));
        }
        self.timed_out = true;
        Ok(())
    }
    pub fn counted_leaks(&self) -> u64 {
        self.leaks.iter().filter(|e| e["counted"] == true).count() as u64
    }
    /// Accounting projection of BattleResult. The battle assembles unit end state,
    /// damage/healing and layer gains separately before publishing its full result.
    pub fn report(&self, time: f64, reason: Option<&str>) -> Value {
        let pp = json!({"killed":self.killed,"total":self.total,"leaked":self.leaks,
            "perfect":self.counted_leaks() == 0,"killedInTotal":self.killed_in_total,
            "leakedInTotal":self.leaked_in_total,"resolved":self.resolved(),"creditedKills":self.credited_kills,"coins":self.coins});
        let mut result = json!({"time":(time*1000.+0.5).floor()/1000.,"reason":reason,
            "perPlayer":{self.player.as_str().unwrap():pp},"killed":self.killed,"total":self.total,"resolved":self.resolved(),"errors":0});
        if !self.unspawned.is_empty() {
            result["unspawned"] = json!(self.unspawned);
        }
        result
    }
}
