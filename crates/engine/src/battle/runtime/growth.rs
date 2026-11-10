// SPDX-License-Identifier: GPL-3.0-or-later
//! Battle-local covenant growth; preparation receives only the ordered delta.
use super::*;
use crate::battle::{body, deploy::Kind, skills};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Growth {
    pub live: BTreeMap<String, BattleBond>,
    pub enabled: bool,
    pub follow_wind: bool,
    gains: Vec<(String, f64)>,
    traits: Vec<Trait>,
    pending: bool,
    onstart: Vec<Id>,
}
#[derive(Clone)]
struct Trait {
    unit: Id,
    id: String,
    key: String,
    record: Arc<Value>,
    count: u64,
    used: BTreeMap<String, f64>,
    whole: f64,
    doll: bool,
    steps: i64,
}
impl Trait {
    fn n(&self, key: &str, fallback: f64) -> f64 {
        self.record["bb"][key]
            .as_f64()
            .filter(|n| n.is_finite())
            .unwrap_or(fallback)
    }
    fn s(&self, key: &str) -> &str {
        self.record["bbStr"][key].as_str().unwrap_or("")
    }
}
fn ids(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
impl Field {
    pub(crate) fn follow_cold_wind_layers(&mut self) {
        self.core.host.growth.follow_wind = true;
    }
    pub fn layer_gains(&self) -> Vec<(String, f64)> {
        self.core.host.growth.gains.clone()
    }
    pub fn bond_layers(&self) -> BTreeMap<String, f64> {
        self.core
            .host
            .growth
            .live
            .iter()
            .map(|(id, b)| (id.clone(), b.layers))
            .collect()
    }
}
impl Host {
    pub(super) fn movement_growth_quiet(&self) -> bool {
        !self.growth.pending && self.growth.onstart.is_empty()
    }
    fn growth_ops(&self, w: &World) -> Vec<Id> {
        self.roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .filter(|&id| {
                w.unit(id)
                    .is_ok_and(|u| u.deployment.kind == Kind::Operator)
            })
            .collect()
    }
    fn growth_front(&self, w: &World, id: Id) -> Result<[i32; 2], String> {
        let pos = w.unit(id)?.deployment.tile;
        let vec = self.roster.as_ref().unwrap().entry(id)?.direction.vector();
        Ok([pos[0] + vec[0], pos[1] + vec[1]])
    }
    pub(super) fn growth_install(&mut self, w: &World) -> Result<(), String> {
        if self.growth.live.is_empty() {
            return Ok(());
        }
        let mut owned = BTreeSet::new();
        let mut records = BTreeMap::<String, Arc<Value>>::new();
        let mut list = Vec::new();
        let ops = self.growth_ops(w);
        let mut add = |unit: Id, id: &str| -> Result<(), String> {
            if !owned.insert((unit, id.to_owned())) {
                return Ok(());
            }
            let Some(record) = self.data.table("garrisons").and_then(|t| t.get(id)) else {
                return Err("Unknown growth garrison".into());
            };
            if record["eventType"] != "IN_BATTLE" {
                return Ok(());
            }
            if list.len() >= w.unit_capacity() * 32 {
                return Err("Growth trait capacity exceeded".into());
            }
            list.push(Trait {
                unit,
                id: id.into(),
                key: record["effectKey"].as_str().unwrap_or("").into(),
                record: records
                    .entry(id.into())
                    .or_insert_with(|| Arc::new(record.clone()))
                    .clone(),
                count: 0,
                used: BTreeMap::new(),
                whole: 0.,
                doll: false,
                steps: -1,
            });
            Ok(())
        };
        for &unit in &ops {
            let def = &self.roster.as_ref().unwrap().entry(unit)?.definition;
            for id in self.data.table("chess").unwrap()[&def.key]["garrisonIds"]
                .as_array()
                .into_iter()
                .flatten()
            {
                add(unit, id.as_str().ok_or("Invalid growth garrison")?)?;
            }
        }
        drop(add);
        // Grants are one pass over native traits; granted grants do not recurse.
        for t in list.clone() {
            if t.key != "ADD_BOND" && t.record["battleRuneKey"] != "give_garrion_to_front" {
                continue;
            }
            let id = if t.s("give_garrison_id").is_empty() {
                t.s("give_garrion_id")
            } else {
                t.s("give_garrison_id")
            };
            let Some(record) = self.data.table("garrisons").and_then(|m| m.get(id)) else {
                continue;
            };
            if record["eventType"] != "IN_BATTLE" {
                continue;
            }
            let desc = t.record["desc"].as_str().unwrap_or("");
            let home = w.unit(t.unit)?.deployment.tile;
            let front = self.growth_front(w, t.unit)?;
            let right = ops
                .iter()
                .copied()
                .filter(|&u| w.unit(u).unwrap().deployment.tile[0] == home[0])
                .max_by_key(|&u| w.unit(u).unwrap().deployment.tile[1]);
            for &unit in &ops {
                let at = w.unit(unit)?.deployment.tile;
                let selected = if desc.contains("所有") {
                    true
                } else if desc.contains("同一行最右边") {
                    Some(unit) == right
                } else {
                    at == front || desc.contains("自身和身前") && unit == t.unit
                };
                if !selected
                    || !t.s("check_bond_id").is_empty()
                        && !self.equipment_member(unit, t.s("check_bond_id"))
                    || !owned.insert((unit, id.into()))
                {
                    continue;
                }
                if list.len() >= w.unit_capacity() * 32 {
                    return Err("Growth trait capacity exceeded".into());
                }
                list.push(Trait {
                    unit,
                    id: id.into(),
                    key: record["effectKey"].as_str().unwrap_or("").into(),
                    record: records
                        .entry(id.into())
                        .or_insert_with(|| Arc::new(record.clone()))
                        .clone(),
                    count: 0,
                    used: BTreeMap::new(),
                    whole: 0.,
                    doll: false,
                    steps: -1,
                });
            }
        }
        self.growth.traits = list;
        Ok(())
    }
    pub(super) fn growth_bond_gain(
        &mut self,
        w: &World,
        source: Id,
        id: &str,
        mut amount: f64,
    ) -> Result<(), String> {
        if !self.growth.enabled {
            return Ok(());
        }
        let u = w.unit(source)?;
        if Self::tac_up(w, source) || !u.stats.alive && u.deployment.death_at == w.time {
            for extra in &self.growth.traits {
                if extra.key == "act1autochess_gar_event_addition_cnt"
                    && extra.unit != source
                    && Self::tac_up(w, extra.unit)
                    && matches!(extra.s("range_id"), "" | "1-1")
                    && self.growth_front(w, extra.unit)? == u.deployment.tile
                {
                    amount += extra.n("extra_cnt", 0.);
                }
            }
        }
        let Some(b) = self.growth.live.get_mut(id).filter(|b| b.tier > 0) else {
            return Ok(());
        };
        let amount = amount.floor().max(0.).min((999. - b.layers).max(0.));
        if amount <= 0. {
            return Ok(());
        }
        b.layers += amount;
        if let Some((_, n)) = self.growth.gains.iter_mut().find(|(k, _)| k == id) {
            *n += amount;
        } else {
            self.growth.gains.push((id.into(), amount));
        }
        self.growth.pending = true;
        self.growth_map();
        Ok(())
    }
    fn growth_fire(&mut self, w: &World, index: usize) -> Result<(), String> {
        if !self.growth.enabled {
            return Ok(());
        }
        let t = self.growth.traits[index].clone();
        let tile = w.unit(t.unit)?.deployment.tile;
        let ops = self.growth_ops(w);
        let row = ops
            .iter()
            .filter(|&&u| Self::tac_up(w, u) && w.unit(u).unwrap().deployment.tile[0] == tile[0])
            .count();
        let col = ops
            .iter()
            .filter(|&&u| Self::tac_up(w, u) && w.unit(u).unwrap().deployment.tile[1] == tile[1])
            .count();
        if t.s("conditionkey") == "character_same_row" && (row as f64) < t.n("check_count", 0.)
            || t.s("conditionkey") == "character_same_col" && (col as f64) < t.n("check_count", 0.)
        {
            return Ok(());
        }
        let active = |id: &String| self.growth.live.get(id).is_some_and(|b| b.tier > 0);
        let bonds = match t.s("bond_type") {
            "bond_self" => self
                .equipment
                .units
                .get(&t.unit)
                .map(|l| l.bonds.clone())
                .unwrap_or_default(),
            "bond_actived_maxstack" => {
                let mut best = None;
                let mut top = -1.;
                for (id, _) in self.data.table("bonds").unwrap().as_object().unwrap() {
                    if active(id) && self.growth.live[id].layers > top {
                        top = self.growth.live[id].layers;
                        best = Some(id.clone());
                    }
                }
                best.into_iter().collect()
            }
            _ => ids(t.s("bond_id")),
        };
        let n = match t.s("bond_add_type") {
            "by_charcount_samerow" => {
                row as f64 * t.n("bond_add_count_multi", t.n("bond_add_count", 1.))
            }
            "by_charlevel" => {
                let def = &self.roster.as_ref().unwrap().entry(t.unit)?.definition;
                self.data.table("chess").unwrap()[&def.key]["tier"]
                    .as_f64()
                    .unwrap_or(1.)
                    .floor()
                    .max(1.)
                    * t.n("bond_add_count", 1.)
            }
            _ => t.n("bond_add_count", 0.),
        }
        .floor()
        .max(0.);
        let cap = t.n("max_add_count_per_battle", 0.);
        let cap = if cap > 0. { cap.floor() } else { f64::INFINITY };
        let whole = t.s("bond_type") == "bond_actived_maxstack" && cap.is_finite();
        let mut whole_added = false;
        for id in bonds {
            let Some(b) = self.growth.live.get(&id) else {
                continue;
            };
            if b.tier == 0 || b.layers >= 999. {
                continue;
            }
            let used = if whole {
                t.whole
            } else {
                t.used.get(&id).copied().unwrap_or(0.)
            };
            let base = n.min((cap - used).max(0.));
            if base <= 0. {
                continue;
            }
            let mut amount = base;
            for extra in &self.growth.traits {
                if extra.key != "act1autochess_gar_event_addition_cnt"
                    || !Self::tac_up(w, extra.unit)
                    || extra.unit == t.unit
                    || !matches!(extra.s("range_id"), "" | "1-1")
                {
                    continue;
                }
                if self.growth_front(w, extra.unit)? == tile {
                    amount += extra.n("extra_cnt", 0.);
                }
            }
            let amount = amount.floor().min((999. - b.layers).max(0.));
            if amount <= 0. {
                continue;
            }
            self.growth.live.get_mut(&id).unwrap().layers += amount;
            if let Some((_, n)) = self.growth.gains.iter_mut().find(|(k, _)| k == &id) {
                *n += amount;
            } else {
                self.growth.gains.push((id.clone(), amount));
            }
            if whole {
                whole_added = true;
            } else {
                *self.growth.traits[index].used.entry(id).or_default() += base;
            }
            self.growth.pending = true;
        }
        self.growth_map();
        if whole_added {
            self.growth.traits[index].whole += n.min((cap - t.whole).max(0.));
        }
        Ok(())
    }
    pub(super) fn growth_skill(
        &mut self,
        w: &World,
        unit: Id,
        event: skills::Event,
    ) -> Result<(), String> {
        if event == skills::Event::Deploy
            && self
                .growth
                .traits
                .iter()
                .any(|t| t.unit == unit && t.key == "act2autochess_gar_eff_attrByBond_add_onstart")
        {
            if self.growth.onstart.len() >= w.unit_capacity() * 32 {
                return Err("Growth deployment capacity exceeded".into());
            }
            self.growth.onstart.push(unit);
        }
        for i in 0..self.growth.traits.len() {
            let t = &self.growth.traits[i];
            let fire = match event {
                skills::Event::Deploy => {
                    t.unit == unit && t.key == "act2autochess_gar_event_onstart"
                }
                skills::Event::SkillStart => {
                    t.unit == unit && t.key == "act1autochess_gar_event_useskill"
                }
                skills::Event::AmmoUsed if t.key == "act1autochess_gar_event_consume_ammo" => {
                    let scope = match t.s("range_id") {
                        "1-1" => {
                            Self::tac_up(w, t.unit)
                                && Self::tac_up(w, unit)
                                && unit != t.unit
                                && self.growth_front(w, t.unit)? == w.unit(unit)?.deployment.tile
                        }
                        "x-5" => {
                            unit == t.unit
                                || Self::tac_up(w, t.unit)
                                    && Self::tac_up(w, unit)
                                    && w.unit(unit)?.deployment.kind == Kind::Operator
                                    && {
                                        let a = w.unit(unit)?.deployment.tile;
                                        let b = w.unit(t.unit)?.deployment.tile;
                                        (a[0] - b[0]).abs() + (a[1] - b[1]).abs() == 1
                                    }
                        }
                        _ => unit == t.unit,
                    };
                    if scope {
                        let period = t.n("consume_count", 1.).max(1.);
                        self.growth.traits[i].count += 1;
                        (self.growth.traits[i].count as f64) % period == 0.
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if fire {
                self.growth_fire(w, i)?;
            }
        }
        Ok(())
    }
    pub(super) fn growth_death(
        &mut self,
        w: &World,
        unit: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        for deaths in [false, true] {
            for i in 0..self.growth.traits.len() {
                let t = &self.growth.traits[i];
                let fire =
                    if deaths && t.unit == unit && t.key == "act1autochess_gar_event_selfdead" {
                        true
                    } else if !deaths
                        && Some(t.unit) == credit
                        && t.key == "act1autochess_gar_event_selfkillenemy"
                        && (w.unit(unit)?.side == health::Side::Enemy
                            || t.record["desc"].as_str().unwrap_or("").contains("我方干员")
                                && w.unit(unit)?.deployment.kind == Kind::Operator)
                    {
                        let period = t.n("check_cnt", 1.).max(1.);
                        self.growth.traits[i].count += 1;
                        (self.growth.traits[i].count as f64) % period == 0.
                    } else {
                        false
                    };
                if fire {
                    self.growth_fire(w, i)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn growth_status(
        &mut self,
        w: &mut World,
        c: &super::super::status::Context,
    ) -> Result<(), String> {
        if !c.entered {
            return Ok(());
        }
        for i in 0..self.growth.traits.len() {
            let t = &self.growth.traits[i];
            let freeze = t.key == "act1autochess_gar_event_enemy_abflag_inrange"
                && c.key == "freeze"
                && w.unit(c.target)?.side == health::Side::Enemy;
            let control = t.key == "act2autochess_gar_event_allyenemy_sleepstun_inrange"
                && matches!(c.key.as_str(), "sleep" | "stun")
                && (w.unit(c.target)?.side == health::Side::Enemy
                    || w.unit(c.target)?.deployment.kind == Kind::Operator);
            if !(freeze || control) || !Self::tac_up(w, t.unit) {
                continue;
            }
            let keys = self
                .roster
                .as_ref()
                .ok_or("Missing growth range context")?
                .range(w, t.unit)?;
            if !body::in_range(w.unit(c.target)?, &keys) {
                continue;
            }
            if freeze && self.rng.next_f64() >= t.n("prob", 1.) {
                continue;
            }
            self.growth_fire(w, i)?;
        }
        Ok(())
    }
    pub(super) fn growth_refresh(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if !self.growth.pending {
            return Ok(());
        }
        self.growth.pending = false;
        self.bonds_install(w, r)?;
        self.bonds_aura(w, r)?;
        self.growth_readers(w, r)?;
        Ok(())
    }
    fn growth_map(&mut self) {
        self.devices.config.top_layers = self
            .growth
            .live
            .values()
            .map(|b| b.layers)
            .fold(0., f64::max);
        if !self.growth.follow_wind {
            return;
        }
        if let (Some(b), Some(wind)) = (
            self.growth.live.get("kjeragShip"),
            self.devices.config.cold_wind.as_mut(),
        ) {
            if b.tier >= 2 {
                if let Some(bb) = self
                    .data
                    .table("bonds")
                    .and_then(|t| t.get("kjeragShip"))
                    .and_then(|v| v["buffs"].as_array())
                    .and_then(|a| a.iter().find(|b| b["key"] == "env_gbuff_new"))
                    .map(|v| &v["bb"])
                {
                    wind.duration = bb["bond_eff_kjerag[storm].base_time"]
                        .as_f64()
                        .unwrap_or(0.)
                        + bb["bond_eff_kjerag[storm].time_per_stack"]
                            .as_f64()
                            .unwrap_or(0.)
                            * b.layers;
                }
            }
        }
    }
}

impl Host {
    fn growth_steps(&self, t: &Trait) -> i64 {
        let sum: f64 = ids(t.s("bond_id"))
            .iter()
            .filter_map(|id| self.growth.live.get(id))
            .filter(|b| b.tier > 0)
            .map(|b| b.layers)
            .sum();
        (sum / t.n("divide_num", 1.).max(1.)).floor() as i64
    }
    pub(super) fn growth_begin_readers(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if !self.growth.live.is_empty() {
            // Definition construction installs static traits early; the original
            // battle domain orders covenant passives before garrison passives.
            for id in self.growth_ops(w) {
                let stats = &mut w.unit_mut(id)?.stats;
                stats
                    .buffs_mut()
                    .sort_by_key(|b| b.key.as_deref().is_some_and(|k| k.starts_with("gar:")));
                stats.mark_dirty();
            }
        }
        self.growth_readers(w, r)
    }
    pub(super) fn growth_readers(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let order: Vec<_> = [
            "act1autochess_gar_eff_attrByBond",
            "act1autochess_gar_eff_respawnTimeByBond",
            "act2autochess_gar_eff_attrByBond_add_onstart",
            "act2autochess_gar_eff_ab_damageScaleByBond",
        ]
        .into_iter()
        .flat_map(|key| {
            self.growth
                .traits
                .iter()
                .enumerate()
                .filter_map(move |(i, t)| (t.key == key).then_some(i))
        })
        .collect();
        for i in order {
            let t = self.growth.traits[i].clone();
            if !matches!(
                t.key.as_str(),
                "act1autochess_gar_eff_attrByBond"
                    | "act1autochess_gar_eff_respawnTimeByBond"
                    | "act2autochess_gar_eff_attrByBond_add_onstart"
                    | "act2autochess_gar_eff_ab_damageScaleByBond"
            ) {
                continue;
            }
            let k = self.growth_steps(&t);
            if k == t.steps {
                continue;
            }
            self.growth.traits[i].steps = k;
            let key = format!("gar:{}", t.id);
            let mut mods = crate::battle::stats::Numbers::new();
            match t.key.as_str() {
                "act1autochess_gar_eff_attrByBond" => {
                    for (param, name) in [
                        ("atk", "atkPct"),
                        ("max_hp", "hpPct"),
                        ("def", "defPct"),
                        ("attack_speed", "aspd"),
                        ("hp_recovery_per_sec", "hpRegen"),
                        ("sp_recovery_per_sec", "spRecoveryFlat"),
                    ] {
                        let value = t.n(param, 0.) * k as f64;
                        if value != 0. {
                            mods.insert(name.into(), value);
                        }
                    }
                }
                "act1autochess_gar_eff_respawnTimeByBond" => {
                    mods.insert(
                        "redeployMul".into(),
                        (1. + t.n("respawn_time", 0.) * k as f64).max(0.),
                    );
                }
                "act2autochess_gar_eff_attrByBond_add_onstart" => {
                    if let Some(left) = w
                        .unit(t.unit)?
                        .stats
                        .buffs()
                        .iter()
                        .find(|b| b.key.as_deref() == Some(&key))
                        .map(|b| b.runtime.time_left)
                    {
                        self.growth_timed_reader(w, r, &t, left)?;
                    }
                    continue;
                }
                _ => continue,
            }
            if k > 0 {
                self.equipment_stat(w, r, t.unit, key, mods)?;
            } else {
                r.remove(w, self, t.unit, &key)?;
            }
        }
        Ok(())
    }
    fn growth_timed_reader(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &Trait,
        duration: f64,
    ) -> Result<(), String> {
        let k = self.growth_steps(t);
        let key = format!("gar:{}", t.id);
        if k <= 0 || duration <= 0. {
            r.remove(w, self, t.unit, &key)?;
            return Ok(());
        }
        let mut buff = crate::battle::stats::Buff {
            key: Some(key),
            mods: [
                ("atkFlat".into(), t.n("atk", 0.) * k as f64),
                ("hpFlat".into(), t.n("max_hp", 0.) * k as f64),
            ]
            .into(),
            ..Default::default()
        };
        buff.runtime.duration = duration;
        r.add(
            w,
            self,
            t.unit,
            buffs::Spec {
                buff,
                mods_present: true,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    pub(super) fn growth_flush_onstart(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let pending = std::mem::take(&mut self.growth.onstart);
        if !pending.is_empty() {
            self.tactical_sync_injury(w, r)?;
        }
        for unit in pending {
            self.growth_onstart(w, r, unit)?;
        }
        Ok(())
    }
    fn growth_onstart(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
    ) -> Result<(), String> {
        for t in self.growth.traits.clone() {
            if t.unit == unit && t.key == "act2autochess_gar_eff_attrByBond_add_onstart" {
                self.growth_timed_reader(w, r, &t, t.n("duration", 0.))?;
            }
        }
        Ok(())
    }
    pub(super) fn growth_doll_swap(&mut self, w: &World, unit: Id) -> Result<(), String> {
        for i in 0..self.growth.traits.len() {
            let t = &self.growth.traits[i];
            if t.unit == unit
                && t.key == "act2autochess_gar_event_onstart"
                && matches!(t.id.as_str(), "garrison_108_a" | "garrison_108_b")
            {
                self.growth_fire(w, i)?;
            }
        }
        Ok(())
    }
    pub(super) fn growth_dolls(&mut self, w: &World) -> Result<(), String> {
        for i in 0..self.growth.traits.len() {
            let t = &self.growth.traits[i];
            if t.key != "act1autochess_gar_event_selfdead"
                || !t.record["desc"].as_str().unwrap_or("").contains("替身")
            {
                continue;
            }
            let doll = w
                .unit(t.unit)?
                .operator
                .tactical
                .dollkeeper()
                .map(|s| s.doll)
                .unwrap_or(false);
            if doll != t.doll {
                let up = Self::tac_up(w, t.unit);
                self.growth.traits[i].doll = doll;
                if up {
                    self.growth_fire(w, i)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn growth_hit(&self, w: &World, q: &mut hit::Request) -> Result<(), String> {
        if !w
            .unit(q.hit.target)
            .is_ok_and(|u| u.side == health::Side::Enemy)
            || q.hit.damage_type == super::super::damage::DamageType::Elemental
        {
            return Ok(());
        }
        for t in &self.growth.traits {
            if Some(t.unit) != q.hit.source
                || t.key != "act2autochess_gar_eff_ab_damageScaleByBond"
                || t.steps <= 0
            {
                continue;
            }
            let buffs = w.unit(q.hit.target)?.stats.buffs();
            let has = |key: &str| {
                buffs
                    .iter()
                    .any(|b| b.key.as_deref() == Some(key) || b.flags.contains(key))
            };
            if t.n("check_ab_flag", 0.) != 0. && has("bind")
                || t.n("check_sluggish", 0.) != 0. && has("sluggish")
            {
                q.damage.mul *= 1. + t.n("damage_scale_per_stack", 0.) * t.steps as f64;
            }
        }
        Ok(())
    }
}
