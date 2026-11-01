use arkguard_engine::battle::{
    blocking::{self, Hooks},
    deploy::{self, Deployment, Kind, Options},
    grid::{Grid, Rect},
    health::{Side, Unit, World},
    stats::{Base, Buff, UnitStats},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
struct Host {
    trace: Vec<Value>,
}
impl Hooks for Host {
    fn blocked(&mut self, b: u64, e: u64, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["blocked", b, e]));
        Ok(())
    }
    fn ended(&mut self, e: u64, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["ended", e]));
        Ok(())
    }
}
impl deploy::Hooks for Host {
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
        _: deploy::Event,
        _: u64,
        _: Option<u64>,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn remove_temporary_effects(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; contact blocking only"]
fn contact_and_release_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/blocking.mjs");
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
        let mut rows = vec!["r".repeat(21); 19];
        if d["fence"] == true {
            rows[10].replace_range(4..5, "b");
        }
        let g = Grid::new(&json!({"rows":rows}), Rect::FULL).unwrap();
        let mut w = World::new(3, 1, 1).unwrap();
        let mut dep = Deployment::new(Rect::FULL, 3, 1, 99.0).unwrap();
        let mut h = Host { trace: vec![] };
        for (id, tile) in [(1, [10, 4]), (2, [10, 5])] {
            let mut b = Base::default();
            b.0.insert("blockCnt".into(), d["cap"].as_f64().unwrap());
            let mut u = Unit::new(UnitStats::new(b), Side::Ally);
            u.deployment.kind = if d["device"] == true {
                Kind::Device
            } else {
                Kind::Operator
            };
            u.deployment.home = tile;
            u.deployment.tile = tile;
            u.ground = d["ground"] == true;
            u.profile_block_fly = d["profileFly"] == true;
            u.stats.set_buffs(vec![Buff {
                mods: [("blockRadiusScale".into(), d["radius"].as_f64().unwrap())]
                    .into_iter()
                    .collect(),
                flags: ["blockFly", "sleep"]
                    .into_iter()
                    .filter(|k| d[*k] == true)
                    .map(str::to_string)
                    .collect(),
                ..Buff::default()
            }]);
            w.insert_unit(id, u).unwrap();
            dep.register(id, &w).unwrap();
            dep.deploy(&mut w, &mut h, id, Options::default()).unwrap();
        }
        let mut e = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
        e.position = [d["y"].as_f64().unwrap(), d["x"].as_f64().unwrap()];
        e.motion_fly = d["fly"] == true;
        e.block_weight = d["weight"].as_f64().unwrap();
        e.hidden = d["hidden"] == true;
        e.moving = true;
        e.stats.set_buffs(vec![Buff {
            flags: if d["unblockable"] == true {
                ["unblockable".into()].into_iter().collect()
            } else {
                Default::default()
            },
            ..Buff::default()
        }]);
        w.insert_unit(3, e).unwrap();
        let mut checks = vec![];
        for _ in 0..2 {
            checks.push(blocking::check(&mut w, &dep, &g, &mut h, 3).unwrap());
        }
        let held = w.unit(3).unwrap().blocked_by();
        if d["i"].as_u64().unwrap() % 2 == 1 {
            blocking::unblock(&mut w, &mut h, 3).unwrap();
        } else {
            for id in [1, 2] {
                blocking::release(&mut w, &mut h, id).unwrap();
            }
        }
        checks.push(blocking::check(&mut w, &dep, &g, &mut h, 3).unwrap());
        let actual = json!({"checks":checks,"held":held,"final":w.unit(3).unwrap().blocked_by(),"lists":[w.unit(1).unwrap().blocking(),w.unit(2).unwrap().blocking()],"moving":w.unit(3).unwrap().moving,"trace":h.trace});
        let mut expected = v.clone();
        expected.as_object_mut().unwrap().remove("scenario");
        assert_eq!(actual, expected, "case {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 1024);
}
