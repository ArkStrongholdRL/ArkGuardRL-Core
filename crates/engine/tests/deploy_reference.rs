use arkguard_engine::battle::{
    deploy::{Deployment, Event, Hooks, Kind, Options},
    grid::Rect,
    health::{Side, Unit, World},
    stats::{Base, UnitStats},
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
    revive: bool,
    trace: Vec<Value>,
}
impl arkguard_engine::battle::blocking::Hooks for Host {
    fn blocked(&mut self, _: u64, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl Hooks for Host {
    fn initialize(
        &mut self,
        _: u64,
        _: Options,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn event(
        &mut self,
        e: Event,
        id: u64,
        _: Option<u64>,
        w: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        let name = match e {
            Event::Kill => {
                if self.revive {
                    w.unit_mut(id)?.stats.hp = 15.0;
                }
                "kill"
            }
            Event::Death => "death",
            Event::Deploy => "deploy",
            _ => return Ok(()),
        };
        self.trace.push(json!([name, id]));
        Ok(())
    }
    fn remove_temporary_effects(&mut self, id: u64, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.stats.set_buffs(vec![]);
        Ok(())
    }
}
fn make(kind: Kind, home: [i32; 2]) -> Unit {
    let mut b = Base::default();
    for (k, v) in [("maxHp", 100.0), ("cost", 7.0), ("respawnTime", 3.0)] {
        b.0.insert(k.into(), v);
    }
    let mut u = Unit::new(UnitStats::new(b), Side::Ally);
    u.stats.alive = false;
    u.owner = Some(1);
    u.deployment.kind = kind;
    u.deployment.uid = Some(1);
    u.deployment.home = home;
    u.deployment.tile = home;
    u
}
fn view(w: &World, id: u64) -> Value {
    let u = w.unit(id).unwrap();
    json!({"alive":u.stats.alive,"deployed":u.deployed,"removed":u.removed,"hp":bits(u.stats.hp),"down":Deployment::is_down(w,id).unwrap(),"body":u.deployment.body,"rest":Deployment::rest_tile(w,id).unwrap(),"respawn":bits(u.deployment.respawn_at)})
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; deployment lifecycle only"]
fn deploy_death_redeploy_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/deploy.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let d = &v["scenario"];
        let mut w = World::new(3, 1, 1).unwrap();
        let mut dep = Deployment::new(
            Rect {
                r0: 9,
                r1: 12,
                c0: 2,
                c1: 10,
            },
            3,
            1,
            99.0,
        )
        .unwrap();
        dep.set_dp(1, 20.0).unwrap();
        w.insert_unit(
            1,
            make(
                if d["token"] == true {
                    Kind::Token
                } else {
                    Kind::Operator
                },
                [9, 2],
            ),
        )
        .unwrap();
        w.insert_unit(
            2,
            make(
                Kind::Operator,
                if d["foreign"] == true {
                    [10, 4]
                } else {
                    [12, 10]
                },
            ),
        )
        .unwrap();
        dep.register(1, &w).unwrap();
        dep.register(2, &w).unwrap();
        let mut h = Host {
            revive: d["revive"] == true,
            trace: vec![],
        };
        dep.deploy(&mut w, &mut h, 1, Options::default()).unwrap();
        dep.deploy(&mut w, &mut h, 2, Options::default()).unwrap();
        if d["foreign"] == true {
            assert!(dep.relocate(&mut w, &mut h, 2, [12, 9]).unwrap());
        }
        assert!(dep.relocate(&mut w, &mut h, 1, [10, 4]).unwrap());
        if d["homeTaken"] == true {
            assert!(dep.relocate(&mut w, &mut h, 2, [9, 2]).unwrap());
        }
        w.unit_mut(1).unwrap().deployment.down_at_home = d["downHome"] == true;
        w.time = 1.0;
        if d["retreat"] == true {
            dep.retreat(
                &mut w,
                &mut h,
                1,
                if d["raid"] == true { "raid" } else { "retreat" },
                d["permanent"] == true,
            )
            .unwrap();
        } else {
            dep.kill(&mut w, &mut h, 1, None).unwrap();
        }
        assert_eq!(view(&w, 1), v["before"], "before {d}");
        w.time = 5.0;
        dep.check_redeploys(&mut w, &mut h).unwrap();
        let mut after = view(&w, 1);
        let u = w.unit(1).unwrap();
        after["seq"] = json!(u.deployment.deploy_seq);
        after["tile"] = json!(u.deployment.tile);
        after["dp"] = json!(bits(dep.dp(1).unwrap()));
        assert_eq!(after, v["after"], "after {d}");
        assert_eq!(json!(h.trace), v["trace"], "trace {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 384);
}
