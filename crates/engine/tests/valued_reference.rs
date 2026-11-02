use arkguard_engine::battle::{
    buffs::{self, Runtime, Spec},
    health::{self, Side, Unit, World},
    stats::{Base, Buff, UnitStats},
    valued::{self, Kind, Request},
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
impl buffs::Hooks for Host {
    fn status(&mut self, u: u64, key: &str, on: bool) -> Result<(), String> {
        self.trace
            .push(json!(["status", u, key, if on { 1 } else { 0 }]));
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
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; strongest-effect timelines"]
fn strongest_timelines_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/valued.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let name = v["name"].as_str().unwrap();
        let kind = match name {
            "slow" => Kind::Slow,
            "fragile" => Kind::Fragile,
            "artsFragile" => Kind::ArtsFragile,
            "physFragile" => Kind::PhysFragile,
            "elemFragile" => Kind::ElemFragile,
            "weaken" => Kind::Weaken,
            "aspdDown" => Kind::AspdDown,
            "defDown" => Kind::DefDown,
            "resDown" => Kind::ResDown,
            "resist" => Kind::Resist,
            _ => Kind::Custom(|v| [("atkPct".into(), v)].into_iter().collect()),
        };
        let mut w = World::new(1, 1, 1).unwrap();
        let mut base = Base::default();
        for (k, x) in [
            ("maxHp", 1000.0),
            ("atk", 100.0),
            ("def", 100.0),
            ("res", 30.0),
            ("moveSpeed", 1.0),
        ] {
            base.0.insert(k.into(), x);
        }
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let mut r = Runtime::new(64, 256, 64).unwrap();
        let mut h = Host { trace: vec![] };
        if name == "resist" {
            let mut b = Buff {
                key: Some("palsy".into()),
                stacks: 3,
                ..Default::default()
            };
            b.runtime.status = Some("palsy".into());
            b.runtime.visible = true;
            b.runtime.max_stacks = 3;
            r.add(
                &mut w,
                &mut h,
                1,
                Spec {
                    buff: b,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let mut states = vec![];
        for op in v["ops"].as_array().unwrap() {
            match op["op"].as_str().unwrap() {
                "tick" => {
                    let dt = op["dt"].as_f64().unwrap();
                    w.time += dt;
                    r.tick(&mut w, &mut h, &[1], dt).unwrap();
                }
                "remove" => {
                    r.remove(&mut w, &mut h, 1, name).unwrap();
                }
                "raw" => {
                    let mut b = Buff {
                        key: Some(name.into()),
                        mods: [("atkPct".into(), 0.01)].into_iter().collect(),
                        ..Default::default()
                    };
                    b.runtime.status = if name == "plain" {
                        None
                    } else {
                        Some(name.into())
                    };
                    b.runtime.duration = op["duration"].as_f64().unwrap();
                    r.add(
                        &mut w,
                        &mut h,
                        1,
                        Spec {
                            buff: b,
                            mods_present: true,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                _ => {
                    valued::apply(
                        &mut w,
                        &mut h,
                        &mut r,
                        1,
                        Request {
                            key: name,
                            kind,
                            plain: name == "plain",
                            duration: if op["duration"] == "inf" {
                                f64::INFINITY
                            } else {
                                op["duration"].as_f64().unwrap()
                            },
                            value: op["value"].as_f64().unwrap(),
                            source: op["source"].as_u64(),
                            stack_as: if name == "plain" {
                                None
                            } else {
                                op["stackAs"].as_f64()
                            },
                        },
                    )
                    .unwrap();
                }
            }
            let u = w.unit_mut(1).unwrap();
            let s = u.stats.stats().unwrap();
            let stats: Vec<_> = [
                "atk",
                "def",
                "res",
                "moveSpeed",
                "dmgTakenMul",
                "artsTakenMul",
                "physTakenMul",
                "elementalTakenMul",
                "aspd",
            ]
            .iter()
            .map(|k| bits(s.n(k)))
            .collect();
            states.push(json!({"stats":stats,"buffs":u.stats.buffs().iter().map(|b|{let e=b.runtime.valued.as_ref();json!({"id":b.runtime.id(),"key":b.key,"stacks":b.stacks,"duration":bits(b.runtime.duration),"left":bits(b.runtime.time_left),"acc":bits(b.runtime.accumulator),"value":e.filter(|e|e.value_present).map(|e|bits(e.value)),"as":e.and_then(|e|e.stack_as).map(bits),"tail":e.map(|e|e.tail.iter().map(|e|json!({"value":bits(e.value),"as":e.stack_as.map(bits),"until":bits(e.until)})).collect::<Vec<_>>()).unwrap_or_default()})}).collect::<Vec<_>>()}));
        }
        for (j, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "case {} op {j} {name}", v["i"]);
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {}", v["i"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 384);
}
