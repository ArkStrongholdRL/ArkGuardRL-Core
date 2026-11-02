use arkguard_engine::battle::{
    body::HitArea,
    deploy::Kind,
    health::{Side, Unit, World},
    spatial::{self, RadiusRule},
    stats::{Base, Buff, UnitStats},
    targeting::Queries,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
struct Query {
    revealed: Vec<bool>,
}
impl Queries for Query {
    fn enemy_stealthed(&mut self, id: u64, _: &mut World) -> Result<bool, String> {
        Ok(!self.revealed[id as usize - 1])
    }
    fn skip_enemy(&mut self, _: Option<u64>, _: u64, _: &mut World) -> Result<bool, String> {
        unreachable!()
    }
    fn remaining_distance(&mut self, _: u64, _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
    fn body_distance(&mut self, _: u64, _: [f64; 2], _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; actual area queries"]
fn radius_queries_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/radius.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut w = World::new(12, 1, 1).unwrap();
        let mut ids = vec![];
        let mut q = Query { revealed: vec![] };
        for d in v["records"].as_array().unwrap() {
            let id = d["id"].as_u64().unwrap();
            let mut u = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
            u.position = [d["y"].as_f64().unwrap(), d["x"].as_f64().unwrap()];
            u.stats.alive = d["alive"] == true;
            u.deployed = d["deployed"] == true;
            u.hidden = d["hidden"] == true;
            u.owner = d["owner"].as_u64();
            if d["device"] == true {
                u.deployment.kind = Kind::Device;
            }
            if d["huge"] == true {
                u.hit_area = HitArea::new(4.95, 2.95, 0., 1.);
            }
            u.stats.set_buffs(vec![Buff {
                flags: ["stealth", "untargetable"]
                    .into_iter()
                    .filter(|k| d[*k] == true)
                    .map(str::to_string)
                    .collect(),
                ..Buff::default()
            }]);
            q.revealed.push(d["revealed"] == true);
            w.insert_unit(id, u).unwrap();
            ids.push(id);
        }
        let radius = v["radius"].as_f64().unwrap();
        for (key, rule, foes) in [
            ("centre", RadiusRule::Centre, false),
            ("body", RadiusRule::Body, false),
            ("foes", RadiusRule::Body, true),
            ("collider", RadiusRule::Collider, true),
        ] {
            assert_eq!(
                json!(spatial::enemies_in_radius(
                    &mut w,
                    &mut q,
                    &ids,
                    [10., 4.],
                    radius,
                    rule,
                    foes,
                    12
                )
                .unwrap()),
                v[key],
                "{key} case {}",
                v["i"]
            );
        }
        assert_eq!(
            json!(spatial::allies_in_radius(
                &w,
                &ids,
                [10., 4.],
                radius,
                v["owner"].as_u64(),
                v["devices"] == true,
                12
            )
            .unwrap()),
            v["allies"],
            "allies {}",
            v["i"]
        );
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 256);
}
