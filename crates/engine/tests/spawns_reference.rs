//! A headless native tick composition, against actual Battle.step. Content is
//! disabled on both sides; this does not certify any enemy's special ability.
use arkguard_engine::{
    battle::{
        blocking, body, buffs,
        clock::{self, Clock, End, EndState, Phase, DT},
        enemy::{self, Delivery, Profile},
        grid::{Grid, Rect},
        health::{self, Id, World},
        motion, settlement,
        spatial::{self, RadiusRule},
        spawns,
        targeting::Queries,
        waves::Template,
    },
    data::Catalog,
    deterministic::rng::Rng,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
};
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
struct Host {
    spawns: Option<spawns::Runtime>,
    released: Vec<Id>,
}
impl Host {
    fn remove(&mut self, id: Id, killed: bool, w: &mut World) -> Result<(), String> {
        if killed {
            self.spawns.as_mut().unwrap().ledger.kill(id, false)?;
        } else {
            self.spawns.as_mut().unwrap().ledger.leak(id)?;
        }
        enemy::clear(w, id)?;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        u.removed = true;
        u.deployment.death_at = time;
        u.deployment.remove_reason = Some(if killed { "killed" } else { "leak" }.into());
        if killed {
            u.stats.hp = 0.;
        }
        Ok(())
    }
}
impl spawns::Hooks for Host {
    fn spawned(&mut self, id: Id, w: &mut World, r: &mut spawns::Runtime) -> Result<(), String> {
        assert_eq!(r.definition(id)?.key, r.ledger.alive()[&id].key);
        assert!(w.unit(id)?.deployed);
        Ok(())
    }
    fn release(&mut self, id: Id, _: &mut World) -> Result<(), String> {
        assert!(!self.released.contains(&id));
        self.released.push(id);
        Ok(())
    }
}
impl health::Hooks for Host {
    fn kill(&mut self, id: Id, _: Option<Id>, w: &mut World) -> Result<(), String> {
        self.remove(id, true, w)
    }
}
impl buffs::Hooks for Host {
    fn status(&mut self, _: Id, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        _: u64,
        _: buffs::Event,
        _: buffs::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("No content callbacks in bare reference".into())
    }
}
impl settlement::Hooks for Host {}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Err("No blockers in fixture".into())
    }
    fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl motion::Hooks for Host {
    fn check_block(&mut self, _: Id, _: &mut World, _: &mut Grid) -> Result<bool, String> {
        Ok(false)
    }
    fn hidden(&mut self, id: Id, on: bool, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.hidden = on;
        Ok(())
    }
    fn cut_attack_stand(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.motion.attack_stand_until = f64::NEG_INFINITY;
        Ok(())
    }
    fn leak(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        self.remove(id, false, w)
    }
}
impl Queries for Host {
    fn enemy_stealthed(&mut self, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn skip_enemy(&mut self, _: Option<Id>, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, _: Id, _: &mut World) -> Result<f64, String> {
        Err("No allied priority selector in fixture".into())
    }
    fn body_distance(&mut self, id: Id, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        Ok(body::distance(w.unit(id)?, p))
    }
}
impl enemy::Hooks for Host {
    fn profile(&mut self, id: Id, _: &mut World) -> Result<Profile, String> {
        Ok(self
            .spawns
            .as_ref()
            .unwrap()
            .definition(id)?
            .profile
            .clone())
    }
    fn allies_in_radius(
        &mut self,
        _: Id,
        _: f64,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Ok(vec![])
    }
    fn enemies_in_radius(
        &mut self,
        id: Id,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        let p = w.unit(id)?.position;
        spatial::enemies_in_radius(w, self, &ids, p, radius, RadiusRule::Body, false, limit)
    }
    fn can_target(&mut self, _: Id, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(true)
    }
    fn enemy_event_enabled(&self, _: enemy::Event) -> bool {
        false
    }
    fn enemy_event(
        &mut self,
        _: enemy::Event,
        _: &mut enemy::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
        _: &mut Grid,
        _: &mut Rng,
    ) -> Result<(), String> {
        Err("No content in fixture".into())
    }
    fn attack_visual(&mut self, _: Id, _: Id, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn palsy_visual(&mut self, _: Id, _: &World) -> Result<(), String> {
        Ok(())
    }
    fn deliver(
        &mut self,
        _: Delivery,
        _: &mut buffs::Runtime,
        _: &mut World,
        _: &mut Rng,
    ) -> Result<(), String> {
        Err("No allied targets in fixture".into())
    }
}
struct Field {
    world: World,
    grid: Grid,
    buffs: buffs::Runtime,
    rng: Rng,
    host: Host,
    time_limit: f64,
}
impl clock::Host for Field {
    fn phase(&mut self, p: Phase, c: &mut Clock) -> Result<(), String> {
        self.world.time = c.time();
        match p {
            Phase::Start => c.mark_started(),
            Phase::Spawns => {
                let mut r = self.host.spawns.take().unwrap();
                let result = r.process(&mut self.world, &self.grid, &mut self.host);
                self.host.spawns = Some(r);
                result?;
            }
            Phase::Buffs => {
                let ids = self.host.spawns.as_ref().unwrap().live().to_vec();
                self.buffs.tick(&mut self.world, &mut self.host, &ids, DT)?;
            }
            Phase::Enemies => {
                let ids = self.host.spawns.as_ref().unwrap().live().to_vec();
                for id in ids {
                    enemy::update(
                        &mut self.world,
                        &mut self.grid,
                        &mut self.buffs,
                        &mut self.rng,
                        &mut self.host,
                        id,
                        DT,
                        512,
                    )?;
                    let u = self.world.unit_mut(id)?;
                    let r = self.grid.rect();
                    let m = 0.5 - 1. / 1048576.;
                    u.position[0] = u.position[0].max(r.r0 as f64 - m).min(r.r1 as f64 + m);
                    u.position[1] = u.position[1].max(r.c0 as f64 - m).min(r.c1 as f64 + m);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn compact_enemies(&mut self, _: &mut Clock) -> Result<(), String> {
        self.host.spawns.as_mut().unwrap().compact(&self.world)
    }
    fn has_removed(&self) -> bool {
        true
    }
    fn release_removed(&mut self, _: &mut Clock) -> Result<(), String> {
        let mut r = self.host.spawns.take().unwrap();
        let result = r.release_removed(&mut self.world, &mut self.host);
        self.host.spawns = Some(r);
        result
    }
    fn has_tick_hook(&self) -> bool {
        false
    }
    fn end_state(&self) -> Option<EndState> {
        let r = self.host.spawns.as_ref().unwrap();
        Some(EndState {
            shared_hp: None,
            boss_like: false,
            auto_finish: true,
            pending_spawns: !r.queue.pending().is_empty(),
            enemy_alive: !r.live().is_empty(),
            pending_holds: false,
            time_limit: self.time_limit,
            max_battle_time: 3600.,
        })
    }
    fn finish(&mut self, reason: &End, _: &Clock) -> Result<(), String> {
        if *reason == End::Timeout {
            let r = self.host.spawns.as_mut().unwrap();
            r.ledger.timeout(&mut r.queue)?;
        }
        Ok(())
    }
}
fn view(f: &Field, c: &Clock) -> Value {
    let r = f.host.spawns.as_ref().unwrap();
    let reason = c.reason().map(|r| match r {
        End::Cleared => "cleared",
        End::Timeout => "timeout",
        End::Forced => "forced",
    });
    let units:Vec<_>=r.live().iter().map(|&id|{let u=f.world.unit(id).unwrap();json!({"id":id,"key":r.definition(id).unwrap().key,"pos":u.position.map(bits),"hp":bits(u.stats.hp),"cd":bits(u.cadence.cooldown),"hidden":u.hidden,"moving":u.moving,"leg":u.motion.route.as_ref().unwrap().leg,"spawn":u.targeting.spawn_seq,"deploy":u.deployment.deploy_seq})}).collect();
    json!({"time":bits(c.time()),"ticks":c.ticks(),"finished":c.finished(),"reason":reason,"pending":r.queue.pending().len(),"sequence":r.queue.sequence(),"units":units,"report":r.ledger.report(c.time(),reason)})
}
fn canonical(v: &mut Value) {
    match v {
        Value::Number(n) => {
            let x = n.as_f64().unwrap();
            if x.fract() == 0. && x.abs() < i64::MAX as f64 {
                *v = json!(x as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(canonical),
        Value::Object(o) => o.values_mut().for_each(canonical),
        _ => {}
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; native spawn/buff/enemy/tick/end composition"]
fn native_spawn_to_result_matches_headless_battle() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let data = Arc::new(Catalog::load(&Path::new(&root).join("data"), 64 << 20).unwrap());
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/spawns.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    let mut ticks = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let o = &v["options"];
        let mut stage = data.table("stages").unwrap()[o["stageId"].as_str().unwrap()].clone();
        stage["legend"] = stage["tiles"].clone();
        let rect = Rect::NORMAL;
        let grid = Grid::new(&stage, rect).unwrap();
        let mut template = Template::parse(
            &json!({"spawns":o["spawns"],"routes":o["routes"],"overrides":o["enemyOverrides"]}),
            &Value::Null,
            &Value::Null,
            512,
            256,
        )
        .unwrap();
        // This oracle supplies Battle's explicit SpawnSpecs, rather than calling
        // spawnsFromTemplate (which deliberately has a narrower input contract).
        template.spawns = o["spawns"].as_array().unwrap().clone();
        let spawns =
            spawns::Runtime::new(&data, template, &grid, json!("p"), 512, 512, 256).unwrap();
        let mut f = Field {
            world: World::new(512, 1, 1).unwrap(),
            grid,
            buffs: buffs::Runtime::new(64, 512, 32).unwrap(),
            rng: Rng::new(o["seed"].as_u64().unwrap() as u32),
            host: Host {
                spawns: Some(spawns),
                released: vec![],
            },
            time_limit: o["timeLimit"].as_f64().unwrap(),
        };
        let mut c = Clock::new();
        for (t, expected) in v["states"].as_array().unwrap().iter().enumerate() {
            if t == 6 {
                for id in f.host.spawns.as_ref().unwrap().live() {
                    f.world.unit_mut(*id).unwrap().stats.hp *= 0.5;
                }
            }
            for op in v["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| o["tick"].as_u64() == Some(t as u64))
            {
                match op["op"].as_str().unwrap() {
                    "spawn" => {
                        let mut r = f.host.spawns.take().unwrap();
                        r.spawn(
                            &mut f.world,
                            &f.grid,
                            &mut f.host,
                            op["key"].as_str().unwrap(),
                            &op["def"],
                        )
                        .unwrap();
                        f.host.spawns = Some(r);
                    }
                    "kill" => f
                        .host
                        .remove(op["id"].as_u64().unwrap(), true, &mut f.world)
                        .unwrap(),
                    _ => unreachable!(),
                }
            }
            c.step(&mut f).unwrap();
            let mut actual = view(&f, &c);
            let mut expected = expected.clone();
            canonical(&mut actual);
            canonical(&mut expected);
            assert_eq!(actual, expected, "case {cases} tick {t}");
            ticks += 1;
            // Zero budget is a genuine pause, including immediately before termination.
            assert_eq!(c.run_steps(&mut f, 0).unwrap(), 0);
        }
        let owned = view(&f, &c);
        let before = owned.clone();
        if !c.finished() {
            c.run_steps(&mut f, 3).unwrap();
        }
        assert_eq!(owned, before);
        assert!(!f.host.spawns.as_ref().unwrap().faulted());
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 40);
    eprintln!("{cases} headless battle timelines; {ticks} ticks");
}
