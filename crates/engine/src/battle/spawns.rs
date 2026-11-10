// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/spawns.js: native scheduled/runtime spawning.
use super::{
    definitions::Enemy,
    grid::{Grid, Rect, COLS},
    health::{Id, World},
    ledger::{self, Ledger},
    motion::Route,
    waves::{self, Pending, Queue, Template},
};
use crate::data::Catalog;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

pub trait Hooks {
    /// Required: install the real enemy content and emit deploy/enemySpawn in order.
    /// A missing ability is a technical error, never an implicit stats-only enemy.
    fn spawned(&mut self, id: Id, w: &mut World, runtime: &mut Runtime) -> Result<(), String>;
    fn release(&mut self, id: Id, w: &mut World) -> Result<(), String>;
    /// Physical retirement follows the last projectile/effect reference.
    fn retired(&mut self, _id: Id, _w: &mut World) -> Result<(), String> {
        Ok(())
    }
}
pub struct Runtime {
    devices: Vec<Value>,
    pub queue: Queue,
    pub ledger: Ledger,
    definitions: BTreeMap<String, Arc<Enemy>>,
    units: BTreeMap<Id, Arc<Enemy>>,
    live: Vec<Id>,
    routes: Vec<Value>,
    patrol_routes: std::collections::BTreeSet<usize>,
    path_routes: std::collections::BTreeSet<u64>,
    extra_routes: Vec<Value>,
    branches: BTreeMap<String, Vec<Vec<Value>>>,
    overrides: Value,
    fallback: Value,
    rect: Rect,
    capacity: usize,
    max_legs: usize,
    faulted: bool,
    depth: usize,
}
impl Runtime {
    /// The complete battle owner checks scheduled content before starting. The
    /// explicit bare-engine oracle can still exercise unsupported definitions.
    pub fn validate_content(&self, data: &Catalog) -> Result<(), String> {
        for p in self.queue.pending() {
            let key = p.spec["enemyKey"].as_str().ok_or("Missing enemy key")?;
            let raw = &data.table("enemies").ok_or("Missing enemy data")?[key];
            super::content::enemies::Kit::compile(
                key,
                raw,
                self.content_overrides(key),
                p.spec["tag"].as_str(),
            )?;
        }
        Ok(())
    }
    /// Use from the required spawned callback, after deploy and before enemySpawn
    /// consumers. No JS objects, source checkout or renderer enter this path.
    pub fn install_content<H: super::buffs::Hooks>(
        &self,
        data: &Catalog,
        w: &mut World,
        buffs: &mut super::buffs::Runtime,
        content: &mut super::content::enemies::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        let key = &self.definition(id)?.key;
        let entry = self
            .ledger
            .alive()
            .get(&id)
            .ok_or("Missing spawned enemy ledger entry")?;
        let raw = &data.table("enemies").ok_or("Missing enemy data")?[key];
        let definition = super::content::enemies::Definition::compile(
            key,
            raw,
            self.content_overrides(key),
            entry.tag.as_str(),
        )?;
        content.install(w, buffs, h, id, definition, &entry.mods)
    }
    pub fn new(
        data: &Catalog,
        template: Template,
        grid: &Grid,
        player: Value,
        capacity: usize,
        report_capacity: usize,
        max_legs: usize,
    ) -> Result<Self, String> {
        if capacity == 0 || max_legs == 0 {
            return Err("Invalid spawn runtime limits".into());
        }
        if !template.content_actions.is_empty() {
            return Err("Wave content actions require an implemented content dispatcher".into());
        }
        let raw = data.table("enemies").ok_or("Missing enemy data")?;
        // Cache definitions, not live state. Sharing the catalog cache across batch
        // slots is a later owner-level optimization; each slot's roster stays private.
        let mut definitions = BTreeMap::new();
        for (key, v) in raw.as_object().ok_or("Expected enemy data object")? {
            let def = Enemy::parse(key, v)?.with_overrides(&template.overrides[key]);
            definitions.insert(key.clone(), Arc::new(def));
        }
        let mut queue = Queue::new(capacity)?;
        for s in &template.spawns {
            let key = s["enemyKey"]
                .as_str()
                .ok_or("Missing scheduled enemy key")?;
            let d = definitions
                .get(key)
                .ok_or_else(|| format!("Unknown enemy {key}; no fallback substituted"))?;
            queue.push(s, true, d.not_count_in_total)?;
        }
        let ledger = Ledger::new(player, &queue, capacity, report_capacity)?;
        let tile = |kind: &str| {
            grid.tiles()
                .iter()
                .enumerate()
                .find(|(_, t)| t.special.as_deref() == Some(kind))
                .map(|(i, _)| json!([i / COLS, i % COLS]))
        };
        let fallback = match (tile("start"), tile("end")) {
            (Some(start), Some(end)) => {
                json!({"motion":"WALK","start":start,"end":end,"checkpoints":[]})
            }
            _ => Value::Null,
        };
        Ok(Self {
            queue,
            ledger,
            definitions,
            units: BTreeMap::new(),
            live: vec![],
            routes: template.routes,
            patrol_routes: template.patrol_routes,
            path_routes: Default::default(),
            extra_routes: template.extra_routes,
            branches: template.branches,
            devices: template.devices,
            overrides: template.overrides,
            fallback,
            rect: grid.rect(),
            capacity,
            max_legs,
            faulted: false,
            depth: 0,
        })
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    pub fn live(&self) -> &[Id] {
        &self.live
    }
    pub fn retained(&self) -> usize {
        self.units.len()
    }
    pub(crate) fn branch_phase(&self, name: &str) -> Option<Vec<Value>> {
        self.branches.get(name).and_then(|p| p.first()).cloned()
    }
    pub(crate) fn branch_phases(&self, name: &str) -> usize {
        self.branches.get(name).map(Vec::len).unwrap_or(0)
    }
    pub(crate) fn tentacle_spots(&self, name: &str, phase: usize) -> Vec<[f64; 2]> {
        let Some(phases) = self.branches.get(name).filter(|p| !p.is_empty()) else {
            return vec![];
        };
        phases[phase.min(phases.len() - 1)]
            .iter()
            .filter_map(|a| {
                let d = self.devices.iter().find(|d| d["alias"] == a["key"])?;
                Some([d["pos"][0].as_f64()?, d["pos"][1].as_f64()?])
            })
            .collect()
    }
    pub(crate) fn branch_route(&self, name: &str) -> Option<Value> {
        let entry = self.branches.get(name)?.first()?.first()?;
        self.extra_routes
            .get(entry["routeIndex"].as_u64()? as usize)
            .cloned()
    }
    pub(crate) fn spawn_runtime<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        key: &str,
        options: &Value,
    ) -> Result<Id, String> {
        self.call(|r| r.spawn_in(w, r.rect, None, h, key, options))
    }
    pub fn extra_routes(&self) -> &[Value] {
        &self.extra_routes
    }
    pub fn content_overrides(&self, key: &str) -> &Value {
        &self.overrides[key]
    }
    pub fn definition(&self, id: Id) -> Result<&Enemy, String> {
        self.units
            .get(&id)
            .map(Arc::as_ref)
            .ok_or_else(|| format!("Unknown spawned enemy {id}"))
    }
    fn call<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if self.faulted {
            return Err("Spawn runtime is faulted".into());
        }
        if self.depth >= 32 {
            self.faulted = true;
            return Err("Spawn callback depth exceeded".into());
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    pub fn process<H: Hooks>(&mut self, w: &mut World, g: &Grid, h: &mut H) -> Result<(), String> {
        self.call(|r| {
            while let Some(p) = r.queue.pop_due(w.time) {
                r.spawn_pending(w, g, h, p)?;
            }
            Ok(())
        })
    }
    pub fn spawn<H: Hooks>(
        &mut self,
        w: &mut World,
        g: &Grid,
        h: &mut H,
        key: &str,
        options: &Value,
    ) -> Result<Id, String> {
        self.call(|r| r.spawn_in(w, g.rect(), Self::exit(g), h, key, options))
    }
    pub(crate) fn remaining_route(&self, w: &World, parent: Id) -> Result<Route, String> {
        let unit = w.unit(parent)?;
        unit.motion
            .route
            .as_ref()
            .ok_or("Missing parent route")?
            .remaining(unit.position, self.rect)
    }
    pub(crate) fn spawn_child<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        key: &str,
        mut route: Route,
        position: [f64; 2],
        mods: &Value,
        source_player: &Value,
        counted: Option<bool>,
        tag: Option<&str>,
    ) -> Result<Id, String> {
        self.call(|r| {
            let def = r
                .definitions
                .get(key)
                .cloned()
                .ok_or_else(|| format!("Unknown enemy {key}; no fallback substituted"))?;
            let position = r.clamp_position(position);
            route.spawn = position;
            let entry = ledger::Enemy {
                key: key.into(),
                mods: mods.clone(),
                lpr: def.lpr,
                source_player: source_player.clone(),
                tag: tag.map(Value::from).unwrap_or(Value::Null),
                counted: counted.unwrap_or(!def.not_count_in_total),
                in_total: false,
                boss: false,
                no_leak: key == "enemy_5601_entlec",
                bounty: 0.,
            };
            r.insert(w, h, def, route, position, entry, false)
        })
    }
    pub(crate) fn spawn_part<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        key: &str,
        position: [f64; 2],
    ) -> Result<Id, String> {
        let route = Route::compile(
            &json!({"motion":"FLY","start":position,"end":position,"steps":[{"t":"WAIT","s":99999}]}),
            self.rect,
            self.max_legs,
            position,
        )?;
        let source_player = self.ledger.player().clone();
        self.spawn_child(
            w,
            h,
            key,
            route,
            position,
            &Value::Null,
            &source_player,
            Some(false),
            Some("part"),
        )
    }
    pub(crate) fn rect(&self) -> Rect {
        self.rect
    }
    fn clamp_position(&self, position: [f64; 2]) -> [f64; 2] {
        [
            position[0]
                .max(self.rect.r0 as f64 - 0.5)
                .min(self.rect.r1 as f64 + 0.5),
            position[1]
                .max(self.rect.c0 as f64 - 0.5)
                .min(self.rect.c1 as f64 + 0.5),
        ]
    }
    fn exit(g: &Grid) -> Option<[usize; 2]> {
        g.tiles()
            .iter()
            .position(|t| t.special.as_deref() == Some("end"))
            .map(|i| [i / COLS, i % COLS])
    }
    fn spawn_in<H: Hooks>(
        &mut self,
        w: &mut World,
        rect: Rect,
        exit: Option<[usize; 2]>,
        h: &mut H,
        key: &str,
        options: &Value,
    ) -> Result<Id, String> {
        let def = self
            .definitions
            .get(key)
            .ok_or_else(|| format!("Unknown enemy {key}; no fallback substituted"))?;
        if !options.is_object() {
            return Err("Expected runtime spawn options".into());
        }
        let counted = options["countInTotal"].as_bool().unwrap_or(
            !def.not_count_in_total && options["tag"] != "boss" && options["tag"] != "part",
        );
        let mut spec = options.clone();
        spec["enemyKey"] = json!(key);
        // spawn_pending increments the shared scheduled/runtime spawn sequence.
        let p = Pending {
            time: w.time,
            sequence: 0,
            spec,
            counted,
            in_total: options["inTotal"] == true && counted,
            precounted: false,
        };
        self.spawn_pending_in(w, rect, exit, h, p)
    }
    pub(crate) fn path_tiles(&mut self, grid: &mut Grid) -> std::collections::BTreeSet<usize> {
        let used = self
            .queue
            .pending()
            .iter()
            .filter(|p| p.spec["route"].is_null())
            .filter_map(|p| {
                Some(p.spec["routeIndex"].as_f64().unwrap_or(0.))
                    .filter(|v| v.is_finite() && *v >= 0. && v.fract() == 0.)
                    .map(|v| v as u64)
            })
            .collect::<std::collections::BTreeSet<_>>();
        self.path_routes.extend(
            used.into_iter()
                .filter(|&index| (index as usize) < self.routes.len()),
        );
        let used = &self.path_routes;
        let restrict = used.iter().any(|&i| {
            self.routes
                .get(i as usize)
                .is_some_and(|r| r["motion"] != "FLY")
        });
        let mut keys = std::collections::BTreeSet::new();
        let rect = grid.rect();
        let point = |v: &Value| {
            v.as_array().filter(|a| a.len() >= 2).and_then(|p| {
                Some([
                    (p[0].as_f64()? + 0.5)
                        .floor()
                        .clamp(rect.r0 as f64, rect.r1 as f64) as i32,
                    (p[1].as_f64()? + 0.5)
                        .floor()
                        .clamp(rect.c0 as f64, rect.c1 as f64) as i32,
                ])
            })
        };
        for (index, route) in self.routes.iter().enumerate() {
            if route["motion"] == "FLY" || (restrict && !used.contains(&(index as u64))) {
                continue;
            }
            let Some(start) = point(&route["start"]) else {
                continue;
            };
            let mut points = vec![start];
            for checkpoint in route["checkpoints"].as_array().into_iter().flatten() {
                if matches!(checkpoint["type"].as_str(), Some("MOVE" | "APPEAR")) {
                    if let Some(p) = point(&checkpoint["pos"]) {
                        points.push(p);
                    }
                }
            }
            if let Some(end) = point(&route["end"]) {
                points.push(end);
            }
            for pair in points.windows(2) {
                if let Some(path) = grid
                    .find_path(pair[0], pair[1], false)
                    .or_else(|| grid.find_path(pair[0], pair[1], true))
                {
                    for [r, c] in path {
                        keys.insert(r as usize * COLS + c as usize);
                    }
                }
            }
        }
        keys
    }
    pub(crate) fn spawn_candle<H: Hooks>(
        &mut self,
        w: &mut World,
        grid: &Grid,
        h: &mut H,
        position: [f64; 2],
        hp: f64,
        defence: f64,
        resistance: f64,
        mass: f64,
    ) -> Result<Id, String> {
        let key = "enemy_5601_entlec";
        let raw = json!({"applyWay":"NONE","notCountInTotal":true,"lifePointReduce":0,"stats":{"maxHp":hp.max(1.),"atk":0,"def":defence,"magicResistance":resistance,"moveSpeed":0,"massLevel":mass}});
        let definition = Enemy::parse(key, &raw)?;
        self.definitions.insert(key.into(), Arc::new(definition));
        let tile = position.map(|v| (v + 0.5).floor() as i32);
        self.call(|runtime|runtime.spawn_pending(w,grid,h,Pending{time:w.time,sequence:0,
            spec:json!({"enemyKey":key,"pos":position,"noLeak":true,"route":{"motion":"WALK","start":tile,"end":tile,"checkpoints":[]}}),
            counted:false,in_total:false,precounted:false}))
    }
    fn spawn_pending<H: Hooks>(
        &mut self,
        w: &mut World,
        g: &Grid,
        h: &mut H,
        p: Pending,
    ) -> Result<Id, String> {
        self.spawn_pending_in(w, g.rect(), Self::exit(g), h, p)
    }
    fn spawn_pending_in<H: Hooks>(
        &mut self,
        w: &mut World,
        rect: Rect,
        exit: Option<[usize; 2]>,
        h: &mut H,
        p: Pending,
    ) -> Result<Id, String> {
        if self.units.len() >= self.capacity || self.live.len() >= self.capacity {
            return Err("Enemy roster capacity exceeded; no spawn dropped".into());
        }
        let key = p.spec["enemyKey"].as_str().ok_or("Missing enemy key")?;
        let def = self
            .definitions
            .get(key)
            .cloned()
            .ok_or_else(|| format!("Unknown enemy {key}"))?;
        let route = if !p.spec["route"].is_null() {
            waves::normalize_route(&p.spec["route"], self.max_legs)?
        } else {
            let i = waves::number(&p.spec["routeIndex"], 0.);
            let i = if i >= 0. && i.fract() == 0. && i < usize::MAX as f64 {
                i as usize
            } else {
                usize::MAX
            };
            self.routes
                .get(i)
                .or_else(|| self.routes.first())
                .unwrap_or(&self.fallback)
                .clone()
        };
        let pos = p.spec["pos"].as_array().filter(|a| a.len() >= 2).map(|a| {
            [
                a[0].as_f64().unwrap_or(f64::NAN),
                a[1].as_f64().unwrap_or(f64::NAN),
            ]
        });
        let start = |v: &Value| {
            v.as_array()
                .filter(|a| a.len() >= 2)
                .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]))
        };
        let pos = pos
            .filter(|p| p.iter().all(|n| n.is_finite()))
            .or_else(|| start(&route["start"]))
            .unwrap_or([rect.r0 as f64, rect.c1 as f64]);
        let pos = [
            pos[0].max(rect.r0 as f64 - 0.5).min(rect.r1 as f64 + 0.5),
            pos[1].max(rect.c0 as f64 - 0.5).min(rect.c1 as f64 + 0.5),
        ];
        let mut compiled = route;
        if compiled.is_null() {
            compiled = json!({"checkpoints":[]});
        }
        if compiled["end"].is_null()
            && compiled["checkpoints"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(true)
        {
            if let Some(tile) = exit {
                compiled["end"] = json!(tile);
            }
        }
        let mut route = Route::compile(&compiled, rect, self.max_legs, pos)?;
        route.patrol = if p.spec["route"].is_null() {
            let i = waves::number(&p.spec["routeIndex"], 0.);
            let index = if i >= 0. && i.fract() == 0. && i < self.routes.len() as f64 {
                i as usize
            } else {
                0
            };
            self.patrol_routes.contains(&index)
        } else {
            waves::has_patrol(&p.spec["route"])
        };
        let mut entry = ledger::Enemy::from_pending(&p, def.lpr, self.ledger.player())?;
        entry.no_leak = key == "enemy_5601_entlec";
        self.insert(w, h, def, route, pos, entry, p.precounted)
    }
    fn insert<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        def: Arc<Enemy>,
        route: Route,
        pos: [f64; 2],
        entry: ledger::Enemy,
        precounted: bool,
    ) -> Result<Id, String> {
        if self.units.len() >= self.capacity || self.live.len() >= self.capacity {
            return Err("Enemy roster capacity exceeded; no spawn dropped".into());
        }
        let mut unit = def.unit(&entry.mods, pos, w.time)?;
        unit.motion.route = Some(route);
        unit.owner = Some(1);
        unit.boss = entry.boss;
        unit.deployment.counted = entry.counted;
        unit.deployment.in_total = entry.in_total;
        let id = w.next_unit_id()?;
        unit.targeting.spawn_seq = self.queue.next_sequence()?;
        unit.deployment.deploy_seq = w.next_deployment_sequence()?;
        w.insert_unit(id, unit)?;
        self.ledger.spawn(id, entry, precounted)?;
        self.units.insert(id, def);
        self.live.push(id);
        h.spawned(id, w, self)?;
        Ok(id)
    }
    pub fn compact(&mut self, w: &World) -> Result<(), String> {
        if self.faulted {
            return Err("Spawn runtime is faulted".into());
        }
        for &id in &self.live {
            w.unit(id)?;
        }
        self.live.retain(|&id| w.unit(id).unwrap().stats.alive);
        Ok(())
    }
    /// Call after real death/leak hooks. Registry entries survive while an attack
    /// or projectile pins them; output records own their data independently.
    pub fn release_removed<H: Hooks>(&mut self, w: &mut World, h: &mut H) -> Result<(), String> {
        self.call(|r| {
            r.compact(w)?;
            let ids: Vec<_> = r.units.keys().copied().collect();
            for id in ids {
                let u = w.unit(id)?;
                if u.removed && !u.stats.alive && !u.deployment.release_notified {
                    h.release(id, w)?;
                    w.unit_mut(id)?.deployment.release_notified = true;
                }
                let u = w.unit(id)?;
                if u.removed && !u.stats.alive && w.unit_pins(id)? == 0 {
                    h.retired(id, w)?;
                    w.release_removed(id)?;
                    r.units.remove(&id);
                }
            }
            Ok(())
        })
    }
}
