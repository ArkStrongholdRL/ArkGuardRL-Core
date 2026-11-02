// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/spawns.js: native scheduled/runtime spawning.
use super::{
    definitions::Enemy,
    grid::{Grid, COLS},
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
}
pub struct Runtime {
    pub queue: Queue,
    pub ledger: Ledger,
    definitions: BTreeMap<String, Arc<Enemy>>,
    units: BTreeMap<Id, Arc<Enemy>>,
    live: Vec<Id>,
    routes: Vec<Value>,
    extra_routes: Vec<Value>,
    overrides: Value,
    fallback: Value,
    capacity: usize,
    max_legs: usize,
    faulted: bool,
    depth: usize,
}
impl Runtime {
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
            extra_routes: template.extra_routes,
            overrides: template.overrides,
            fallback,
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
        self.call(|r| {
            let def = r
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
            r.spawn_pending(w, g, h, p)
        })
    }
    fn spawn_pending<H: Hooks>(
        &mut self,
        w: &mut World,
        g: &Grid,
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
        let rect = g.rect();
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
            if let Some((i, _)) = g
                .tiles()
                .iter()
                .enumerate()
                .find(|(_, t)| t.special.as_deref() == Some("end"))
            {
                compiled["end"] = json!([i / COLS, i % COLS]);
            }
        }
        let route = Route::compile(&compiled, rect, self.max_legs, pos)?;
        let mut unit = def.unit(&p.spec["mods"], pos, w.time)?;
        unit.motion.route = Some(route);
        unit.owner = Some(1);
        unit.boss = p.spec["tag"] == "boss";
        unit.deployment.counted = p.counted;
        unit.deployment.in_total = p.in_total;
        let entry = ledger::Enemy::from_pending(&p, def.lpr, self.ledger.player())?;
        let id = w.next_unit_id()?;
        unit.targeting.spawn_seq = self.queue.next_sequence()?;
        unit.deployment.deploy_seq = w.next_deployment_sequence()?;
        w.insert_unit(id, unit)?;
        self.ledger.spawn(id, entry, p.precounted)?;
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
                    w.release_removed(id)?;
                    r.units.remove(&id);
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battle::{
        deploy::{self, Deployment, Options},
        health::{Side, Unit},
        stats::{Base, UnitStats},
    };
    fn fixture() -> (Runtime, World, Grid) {
        let grid = Grid::new(&json!({}), super::super::grid::Rect::FULL).unwrap();
        let def = Arc::new(Enemy::parse("enemy_a", &json!({"hp":1000})).unwrap());
        let queue = Queue::new(128).unwrap();
        let ledger = Ledger::new(json!("p"), &queue, 128, 128).unwrap();
        (
            Runtime {
                queue,
                ledger,
                definitions: BTreeMap::from([("enemy_a".into(), def)]),
                units: BTreeMap::new(),
                live: vec![],
                routes: vec![],
                extra_routes: vec![],
                overrides: Value::Null,
                fallback: Value::Null,
                capacity: 128,
                max_legs: 16,
                faulted: false,
                depth: 0,
            },
            World::new(128, 1, 1).unwrap(),
            grid,
        )
    }
    #[derive(Default)]
    struct Host {
        fail: bool,
        released: usize,
    }
    impl Hooks for Host {
        fn spawned(&mut self, _: Id, _: &mut World, _: &mut Runtime) -> Result<(), String> {
            if self.fail {
                Err("Enemy ability is not installed".into())
            } else {
                Ok(())
            }
        }
        fn release(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            self.released += 1;
            Ok(())
        }
    }
    impl super::super::blocking::Hooks for Host {
        fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
        fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
    }
    impl deploy::Hooks for Host {
        fn initialize(
            &mut self,
            _: Id,
            _: Options,
            _: &mut World,
            _: &mut Deployment,
        ) -> Result<(), String> {
            Ok(())
        }
        fn event(
            &mut self,
            _: deploy::Event,
            _: Id,
            _: Option<Id>,
            _: &mut World,
            _: &mut Deployment,
        ) -> Result<(), String> {
            Ok(())
        }
        fn remove_temporary_effects(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn content_failure_is_an_exception_not_a_valid_stats_only_continuation() {
        let (mut r, mut w, g) = fixture();
        let mut h = Host {
            fail: true,
            ..Host::default()
        };
        assert!(r.spawn(&mut w, &g, &mut h, "enemy_a", &json!({})).is_err());
        assert!(r.faulted());
        assert_eq!(r.live().len(), 1);
        assert!(r.process(&mut w, &g, &mut h).is_err());
        assert!(r.spawn(&mut w, &g, &mut h, "enemy_a", &json!({})).is_err());
        assert_eq!(r.live().len(), 1);
    }
    #[test]
    fn pinned_corpses_drop_owned_hooks_before_releasing_storage() {
        let (mut r, mut w, g) = fixture();
        let mut h = Host::default();
        let id = r.spawn(&mut w, &g, &mut h, "enemy_a", &json!({})).unwrap();
        w.pin_unit(id).unwrap();
        r.ledger.kill(id, false).unwrap();
        let u = w.unit_mut(id).unwrap();
        u.removed = true;
        u.stats.alive = false;
        r.release_removed(&mut w, &mut h).unwrap();
        assert!(r.live().is_empty());
        assert_eq!(r.retained(), 1);
        assert_eq!(h.released, 1);
        r.release_removed(&mut w, &mut h).unwrap();
        assert_eq!(h.released, 1);
        w.unpin_unit(id).unwrap();
        r.release_removed(&mut w, &mut h).unwrap();
        assert_eq!(r.retained(), 0);
        assert_eq!(h.released, 1);
        assert!(w.unit(id).is_err());
        let next = r.spawn(&mut w, &g, &mut h, "enemy_a", &json!({})).unwrap();
        assert!(next > id);
    }
    #[test]
    fn ally_redeploy_and_enemy_spawn_share_world_identities() {
        let (mut r, mut w, g) = fixture();
        let mut h = Host::default();
        let mut a = Unit::new(UnitStats::new(Base::default()), Side::Ally);
        a.stats.alive = false;
        a.deployment.kind = deploy::Kind::Operator;
        a.deployment.home = [10, 5];
        w.insert_unit(7, a).unwrap();
        let mut d = Deployment::new(g.rect(), 128, 1, 99.).unwrap();
        d.register(7, &w).unwrap();
        assert!(d.deploy(&mut w, &mut h, 7, Options::default()).unwrap());
        let id = r.spawn(&mut w, &g, &mut h, "enemy_a", &json!({})).unwrap();
        assert_eq!(id, 8);
        assert_eq!(w.unit(id).unwrap().deployment.deploy_seq, 2);
        d.retreat(&mut w, &mut h, 7, "retreat", false).unwrap();
        assert!(d.deploy(&mut w, &mut h, 7, Options::default()).unwrap());
        assert_eq!(w.unit(7).unwrap().deployment.deploy_seq, 3);
        let (mut fresh, mut world, grid) = fixture();
        let id = fresh
            .spawn(
                &mut world,
                &grid,
                &mut Host::default(),
                "enemy_a",
                &json!({}),
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(world.unit(id).unwrap().deployment.deploy_seq, 1);
    }
}
