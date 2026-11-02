use arkguard_engine::{
    battle::{
        blocking,
        buffs::{self, Runtime},
        grid::{Grid, Rect},
        health::{self, Side, Unit, World},
        motion::{self, Route},
        stats::{Base, UnitStats},
        status::{self, Options},
    },
    deterministic::rng::Rng,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
struct Host {
    trace: Vec<Value>,
}
impl health::Hooks for Host {
    fn kill(&mut self, _: u64, _: Option<u64>, _: &mut World) -> Result<(), String> {
        unreachable!()
    }
}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: u64, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl buffs::Hooks for Host {
    fn status(&mut self, _: u64, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        _: u64,
        _: buffs::Event,
        _: buffs::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
}
impl status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        false
    }
    fn before_status(
        &mut self,
        _: &mut status::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
    fn has_status_applied(&self) -> bool {
        false
    }
    fn status_applied(
        &mut self,
        _: &status::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
}
impl motion::Hooks for Host {
    fn check_block(&mut self, _: u64, _: &mut World, _: &mut Grid) -> Result<bool, String> {
        Ok(false)
    }
    fn hidden(&mut self, id: u64, on: bool, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.hidden = on;
        self.trace.push(json!(["hidden", on]));
        Ok(())
    }
    fn cut_attack_stand(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["cut"]));
        Ok(())
    }
    fn leak(&mut self, id: u64, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["leak"]));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; route, fear and attraction movement"]
fn movement_timelines_match() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let stages: Value =
        serde_json::from_slice(&std::fs::read(Path::new(&root).join("data/stages.json")).unwrap())
            .unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/motion.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(&root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let i = v["i"].as_u64().unwrap();
        let mut stage = stages[v["stage"].as_str().unwrap()].clone();
        stage["legend"] = stage
            .get("tiles")
            .or_else(|| stage.get("legend"))
            .cloned()
            .unwrap();
        if !v["synthetic"].is_null() {
            stage = v["synthetic"].clone();
        }
        let mut g = Grid::new(&stage, Rect::FULL).unwrap();
        let mut w = World::new(2, 1, 1).unwrap();
        let pos = [v["pos"][0].as_f64().unwrap(), v["pos"][1].as_f64().unwrap()];
        let tile = |k: &str| {
            [
                v[k][0].as_i64().unwrap() as i32,
                v[k][1].as_i64().unwrap() as i32,
            ]
        };
        let mut base = Base::default();
        base.0.insert(
            "moveSpeed".into(),
            if i >= 104 { 30.0 } else { 2.0 + (i % 4) as f64 },
        );
        let mut u = Unit::new(UnitStats::new(base), Side::Enemy);
        u.position = pos;
        u.motion_fly = v["fly"] == true;
        u.deployed = true;
        u.motion.route = Some(Route::compile(&v["route"], Rect::FULL, 128, pos).unwrap());
        w.insert_unit(1, u).unwrap();
        let mut src = Unit::new(UnitStats::new(Base::default()), Side::Ally);
        src.position = [pos[0] - 1.0, pos[1] - 2.0];
        src.deployed = true;
        w.insert_unit(2, src).unwrap();
        let mut r = Runtime::new(16, 64, 16).unwrap();
        let mut h = Host { trace: vec![] };
        let mut rng = Rng::new(i as u32 + 1);
        let mut states = vec![];
        for tick in 0..v["ticks"].as_u64().unwrap() {
            w.time = tick as f64 / 30.0;
            if tick == 2 {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    "fear",
                    Options {
                        duration: 0.8,
                        source: Some(if i % 5 == 0 { 1 } else { 2 }),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            if tick == 8 {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    "attract",
                    Options {
                        duration: 1.5,
                        point: Some(tile("dest").map(|x| x as f64)),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            if tick == 20 {
                let u = w.unit_mut(1).unwrap();
                u.position[1] += 0.3;
                u.position[0] -= 0.2;
            }
            if tick == 25 {
                g.set_obstacle(tile("middle"), true, 1).unwrap();
            }
            if tick == 45 {
                g.set_obstacle(tile("middle"), false, 1).unwrap();
            }
            if tick == 35 {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    "fear",
                    Options {
                        duration: 0.4,
                        source: Some(2),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            r.tick(&mut w, &mut h, &[1, 2], 1.0 / 30.0).unwrap();
            if w.unit(1).unwrap().stats.alive {
                motion::update(
                    &mut w,
                    &mut g,
                    &mut r,
                    &mut rng,
                    &mut h,
                    1,
                    1.0 / 30.0,
                    (50..55).contains(&tick),
                    false,
                )
                .unwrap();
            }
            let distance = motion::remaining_distance(&mut w, &mut g, 1).unwrap();
            let u = w.unit(1).unwrap();
            let route = u.motion.route.as_ref().unwrap();
            let fear=u.motion.fear.as_ref().map(|m|json!({"seq":m.sequence.map(|x|x as i64).unwrap_or(-1),"tiles":m.tiles,"point":m.point,"key":m.key,"goal":m.goal.map(bits),"points":m.points.as_ref().map(|p|p.iter().map(|p|p.map(bits)).collect::<Vec<_>>())}));
            let attract=u.stats.buffs().iter().find(|b|b.runtime.status.as_deref()==Some("attract")).map(|b|json!({"point":b.runtime.attract_path.as_ref().map(|a|a.point).unwrap_or(0),"points":b.runtime.attract_path.as_ref().map(|a|a.points.iter().map(|p|p.map(bits)).collect::<Vec<_>>())}));
            states.push(json!({"pos":u.position.map(bits),"moving":u.moving,"alive":u.stats.alive,"hidden":u.hidden,"rng":rng.state(),"leg":route.leg,"wait":route.wait_left.map(bits),"distance":bits(distance),"fear":fear,"attract":attract}));
        }
        for (tick, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "scenario {} tick {tick}", v["i"]);
        }
        assert_eq!(json!(h.trace), v["trace"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 112);
}
