use arkguard_engine::battle::{
    buffs::{self, BuffId, Context, Event, Refresh, Runtime, Spec},
    health::{self, Side, Unit, World},
    stats::{Base, Buff},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
fn spec(d: &Value) -> Spec {
    let mut b = Buff::default();
    b.key = Some(d["key"].as_str().unwrap().into());
    b.stacks = d["stacks"].as_u64().unwrap_or(1) as u32;
    b.shield = d["shield"].as_f64().unwrap_or(0.);
    if let Some(m) = d["mods"].as_object() {
        b.mods = m
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
    }
    if let Some(f) = d["flags"].as_object() {
        b.flags = f
            .iter()
            .filter(|(_, v)| **v == true)
            .map(|(k, _)| k.clone())
            .collect();
    }
    b.runtime.duration = if d["duration"] == "inf" {
        f64::INFINITY
    } else {
        d["duration"].as_f64().unwrap_or(f64::INFINITY)
    };
    b.runtime.max_stacks = d["cap"].as_u64().unwrap_or(1) as u32;
    b.runtime.refresh = match d["refresh"].as_str().unwrap_or("replace") {
        "extend" => Refresh::Extend,
        "stack" => Refresh::Stack,
        "independent" => Refresh::Independent,
        "keep" => Refresh::Keep,
        _ => Refresh::Replace,
    };
    b.runtime.visible = d["visible"] == true;
    b.runtime.status = d["status"].as_str().map(str::to_string);
    b.runtime.persist = d["persist"] == true;
    b.runtime.interval = d["interval"].as_f64().unwrap_or(0.0);
    b.runtime.shield_hits = d["hits"].as_u64().unwrap_or(0) as u32;
    b.runtime.on_tick = d["tick"].as_u64();
    b.runtime.on_remove = d["remove"].as_u64();
    b.runtime.on_expire = d["expire"].as_u64();
    Spec {
        buff: b,
        mods_present: d["mods"].is_object(),
        flags_present: d["flags"].is_object(),
        allow_dead: d["allowDead"] == true,
    }
}
struct Host {
    trace: Vec<Value>,
    spawned: BTreeSet<BuffId>,
    nested: bool,
}
impl health::Hooks for Host {
    fn kill(&mut self, _: u64, _: Option<u64>, _: &mut World) -> Result<(), String> {
        unreachable!()
    }
}
impl buffs::Hooks for Host {
    fn status(&mut self, unit: u64, key: &str, on: bool) -> Result<(), String> {
        self.trace
            .push(json!(["status", unit, key, if on { 1 } else { 0 }]));
        Ok(())
    }
    fn callback(
        &mut self,
        token: u64,
        event: Event,
        c: Context,
        r: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let key = b.key.clone().unwrap();
        let attached = w
            .unit(c.unit)?
            .stats
            .buffs()
            .iter()
            .any(|b| b.runtime.id() == c.buff);
        let event_name = match event {
            Event::Tick => "tick",
            Event::Remove => "remove",
            Event::Expire => "expire",
        };
        self.trace.push(json!([
            event_name,
            c.buff,
            bits(c.dt),
            bits(b.runtime.time_left),
            bits(b.runtime.accumulator),
            attached
        ]));
        if event != Event::Tick {
            if token == 4 {
                r.remove(w, self, c.unit, "b")?;
            }
            if token == 5 {
                r.add(
                    w,
                    self,
                    c.unit,
                    spec(&json!({"key":key,"duration":0.4,"tick":1,"mods":{"atkPct":0.2}})),
                )?;
            }
            return Ok(());
        }
        match token {
            2 => {
                r.remove_id(w, self, c.unit, c.buff)?;
            }
            3 => {
                if self.spawned.insert(c.buff) {
                    r.add(
                        w,
                        self,
                        c.unit,
                        spec(
                            &json!({"key":"child","duration":0.15,"tick":1,"mods":{"atkPct":0.1}}),
                        ),
                    )?;
                }
            }
            6 => {
                r.add(
                    w,
                    self,
                    c.unit,
                    spec(&json!({"key":key,"duration":0.2,"tick":1,"mods":{"atkPct":0.3}})),
                )?;
            }
            7 => {
                let u = w.unit_mut(c.unit)?;
                u.stats.hp = 0.;
                u.stats.alive = false;
                u.deployed = false;
            }
            8 => {
                if !self.nested {
                    self.nested = true;
                    let result = r.tick(w, self, &[c.unit], c.dt / 2.);
                    self.nested = false;
                    result?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; timed buff sequences"]
fn buff_lifetimes_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/buffs.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut w = World::new(1, 1, 1).unwrap();
        let mut base = Base::default();
        for (k, n) in [
            ("maxHp", 1000.),
            ("atk", 60.),
            (
                "hpRegen",
                if v["i"].as_u64().unwrap() % 2 == 1 {
                    2.3
                } else {
                    10.
                },
            ),
        ] {
            base.0.insert(k.into(), n);
        }
        let mut u = Unit::new(
            arkguard_engine::battle::stats::UnitStats::new(base),
            Side::Ally,
        );
        u.stats.hp = 950.;
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let mut h = Host {
            trace: vec![],
            spawned: BTreeSet::new(),
            nested: false,
        };
        let mut r = Runtime::new(256, 1024, 64).unwrap();
        let mut states = vec![];
        for d in v["ops"].as_array().unwrap() {
            match d["op"].as_str().unwrap() {
                "add" => {
                    r.add(&mut w, &mut h, 1, spec(d)).unwrap();
                }
                "remove" => {
                    r.remove(&mut w, &mut h, 1, d["key"].as_str().unwrap())
                        .unwrap();
                }
                _ => {
                    let dt = d["dt"].as_f64().unwrap();
                    w.time += dt;
                    r.tick(&mut w, &mut h, &[1], dt).unwrap();
                }
            }
            let u = w.unit_mut(1).unwrap();
            let atk = u.stats.stats().unwrap().n("atk");
            states.push(json!({"hp":bits(u.stats.hp),"regen":bits(u.regen_acc),"alive":u.stats.alive,"atk":bits(atk),"buffs":u.stats.buffs().iter().map(|b|json!({"id":b.runtime.id(),"key":b.key,"stacks":b.stacks,"cap":b.runtime.max_stacks,"duration":bits(b.runtime.duration),"left":bits(b.runtime.time_left),"acc":bits(b.runtime.accumulator),"shield":bits(b.shield),"hits":b.runtime.shield_hits})).collect::<Vec<_>>()}));
        }
        for (j, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "case {} operation {j}", v["i"]);
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {}", v["i"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 256);
}
