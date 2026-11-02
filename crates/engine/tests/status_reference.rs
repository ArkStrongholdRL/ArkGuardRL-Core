use arkguard_engine::battle::{
    blocking,
    buffs::{self, Refresh, Runtime, Spec},
    grid::Rect,
    health::{self, Side, Unit, World},
    stats::{Base, Buff, UnitStats},
    status::{self, Options},
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
    cfg: Value,
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
impl status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        self.cfg["before"] == true
    }
    fn before_status(
        &mut self,
        c: &mut status::Context,
        _: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!([
            "before",
            c.key,
            bits(c.duration),
            c.value.map(bits)
        ]));
        let i = self.cfg["i"].as_u64().unwrap();
        if i % 17 == 0 {
            c.cancelled = true;
        }
        if i % 19 == 0 {
            w.unit_mut(1)?.stats.alive = false;
        }
        if i % 5 == 0 {
            c.duration *= 0.5;
        }
        if i % 7 == 0 {
            c.value = Some(0.8);
        }
        if i % 11 == 0 {
            c.duration = f64::NAN;
        }
        Ok(())
    }
    fn has_status_applied(&self) -> bool {
        true
    }
    fn status_applied(
        &mut self,
        c: &status::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!([
            "applied",
            c.key,
            bits(c.duration),
            c.value.map(bits),
            c.entered
        ]));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; all catalogue status applications"]
fn catalogue_statuses_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/status.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let c = &v["cfg"];
        let mut w = World::new(2, 1, 1).unwrap();
        let side = if c["side"] == "ally" {
            Side::Ally
        } else {
            Side::Enemy
        };
        let mut base = Base::default();
        for (k, n) in [
            ("maxHp", 1000.0),
            ("atk", 100.0),
            ("def", 100.0),
            ("res", 30.0),
            ("massLevel", c["mass"].as_f64().unwrap()),
        ] {
            base.0.insert(k.into(), n);
        }
        let mut u = Unit::new(UnitStats::new(base), side);
        u.position = [10.0, 6.0];
        u.deployed = true;
        u.motion_fly = c["fly"] == true;
        u.immune = c["immune"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().into())
            .collect();
        w.insert_unit(1, u).unwrap();
        let mut u = Unit::new(
            UnitStats::new(Base::default()),
            if side == Side::Ally {
                Side::Enemy
            } else {
                Side::Ally
            },
        );
        u.position = [9.5, 4.5];
        u.motion_fly = c["sourceFly"] == true;
        u.deployed = true;
        w.insert_unit(2, u).unwrap();
        let mut r = Runtime::new(128, 1024, 64).unwrap();
        let mut h = Host {
            cfg: c.clone(),
            trace: vec![],
        };
        let initial = Buff {
            key: Some("initial".into()),
            flags: v["flags"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, v)| **v == true)
                .map(|(k, _)| k.clone())
                .collect(),
            ..Default::default()
        };
        r.add(
            &mut w,
            &mut h,
            1,
            Spec {
                buff: initial,
                flags_present: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut states = vec![];
        for op in v["ops"].as_array().unwrap() {
            let accepted = if let Some(dt) = op["tick"].as_f64() {
                w.time += dt;
                r.tick(&mut w, &mut h, &[1, 2], dt).unwrap();
                None
            } else {
                Some(
                    status::apply(
                        &mut w,
                        &mut h,
                        &mut r,
                        Rect {
                            r0: 2,
                            r1: 16,
                            c0: 3,
                            c1: 17,
                        },
                        1,
                        op["key"].as_str().unwrap(),
                        Options {
                            duration: if op["duration"] == "inf" {
                                f64::INFINITY
                            } else {
                                op["duration"].as_f64().unwrap()
                            },
                            value: op["value"].as_f64(),
                            source: op["source"].as_u64(),
                            force: op["force"] == true,
                            refresh: op["refresh"].as_str().map(|s| match s {
                                "replace" => Refresh::Replace,
                                "keep" => Refresh::Keep,
                                _ => Refresh::Extend,
                            }),
                            point: op["point"]
                                .as_array()
                                .map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]),
                            resist_applied: op["resistApplied"] == true,
                            stack_as: op["stackAs"].as_f64(),
                            reenter: op["reenter"] == true,
                            ignore_select: op["ignoreSelect"] == true,
                        },
                    )
                    .unwrap(),
                )
            };
            let resist = status::resist_of(&w, 1).unwrap();
            let u = w.unit_mut(1).unwrap();
            let s = u.stats.stats().unwrap();
            let stats: Vec<_> = ["atk", "def", "res", "moveSpeed", "dmgTakenMul", "aspd"]
                .into_iter()
                .map(|k| bits(s.n(k)))
                .collect();
            let flags: Vec<_> = s.flags.iter().cloned().collect();
            states.push(json!({"accepted":accepted,"alive":u.stats.alive,"resist":bits(resist),"stats":stats,"flags":flags,"buffs":u.stats.buffs().iter().map(|b|json!({"id":b.runtime.id(),"key":b.key,"stacks":b.stacks,"left":bits(b.runtime.time_left),"duration":bits(b.runtime.duration),"acc":bits(b.runtime.accumulator),"attract":b.runtime.attract,"fear":b.runtime.fear.map(|f|json!({"seq":f.sequence,"hit":f.hit.map(bits),"source":f.source.map(bits),"self":f.self_fear}))})).collect::<Vec<_>>()}));
        }
        for (j, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "case {} operation {j}", c["i"]);
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {}", c["i"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 384);
}
