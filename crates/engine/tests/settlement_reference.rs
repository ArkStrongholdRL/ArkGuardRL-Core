use arkguard_engine::{
    battle::{
        buffs::{self, Refresh, Runtime, Spec},
        damage::{Damage, DamageType, Penetration},
        health::{self, Hit, Side, Unit, World},
        hit::Request,
        injury::{DodgeMode, InjuryEvent, Notice, Observation, Outcome},
        settlement::{self, Context},
        stats::{Base, Buff, UnitStats},
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
fn kind(s: &str) -> DamageType {
    match s {
        "phys" => DamageType::Physical,
        "arts" => DamageType::Arts,
        "elemental" => DamageType::Elemental,
        _ => DamageType::True,
    }
}
fn spec(d: &Value) -> Spec {
    let mut b = Buff {
        key: d["key"].as_str().map(str::to_string),
        stacks: d["stacks"].as_u64().unwrap_or(1) as u32,
        shield: d["shield"].as_f64().unwrap_or(0.0),
        ..Default::default()
    };
    if let Some(m) = d["mods"].as_object() {
        b.mods = m
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
    }
    b.runtime.duration = if d["duration"] == "inf" {
        f64::INFINITY
    } else {
        d["duration"].as_f64().unwrap_or(f64::INFINITY)
    };
    b.runtime.visible = d["visible"] == true;
    b.runtime.shield_hits = d["shieldHits"].as_u64().unwrap_or(0) as u32;
    b.runtime.refresh = match d["refresh"].as_str().unwrap_or("replace") {
        "extend" => Refresh::Extend,
        "stack" => Refresh::Stack,
        _ => Refresh::Replace,
    };
    b.runtime.max_stacks = d["maxStacks"].as_u64().unwrap_or(1) as u32;
    b.runtime.on_remove = if d["remove"] == true { Some(9) } else { None };
    b.runtime.injury = buffs::Injury {
        priority: d["injuryPriority"].as_f64().unwrap_or(0.0),
        reduction: d["damageReduction"].as_object().map(|_| {
            ["phys", "arts", "true", "elemental"]
                .map(|k| d["damageReduction"][k].as_f64().unwrap_or(0.0))
        }),
        reduction_group: d["reductionGroup"].as_str().map(str::to_string),
        shield_types: d["shieldType"]
            .as_array()
            .map(|a| a.iter().map(|v| kind(v.as_str().unwrap())).collect()),
        keep_empty_shield: d["keepEmptyShield"] == true,
        on_injury: d["injury"].as_u64(),
        observe: if d["observe"] == true { Some(1) } else { None },
    };
    Spec {
        buff: b,
        mods_present: d["mods"].is_object(),
        flags_present: d["flags"].is_object(),
        ..Default::default()
    }
}
fn request(d: &Value) -> Request {
    let mut r = Request::new(
        Hit {
            target: 1,
            source: None,
            credit: None,
            damage_type: kind(d["type"].as_str().unwrap()),
            no_sp: true,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount: d["amount"].as_f64().unwrap(),
            mul: 1.0,
            penetration: Penetration::default(),
            resolved: false,
        },
    );
    r.dodge_mode = match d["dodgeMode"].as_str().unwrap_or("normal") {
        "ignore" => DodgeMode::Ignore,
        "uncancellable" => DodgeMode::Uncancellable,
        _ => DodgeMode::Normal,
    };
    r.ignore_block = d["ignoreCancelReasons"]
        .as_array()
        .map(|a| a.iter().any(|v| v == "block"))
        .unwrap_or(false);
    r
}
struct Host {
    trace: Vec<Value>,
    nesting: bool,
}
impl health::Hooks for Host {
    fn kill(&mut self, _: u64, _: Option<u64>, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["kill"]));
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
        token: u64,
        _: buffs::Event,
        c: buffs::Context,
        r: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        assert_eq!(token, 9);
        self.trace.push(json!([
            "remove",
            c.buff,
            bits(r.get(w, c.unit, c.buff)?.runtime.time_left)
        ]));
        Ok(())
    }
}
impl settlement::Hooks for Host {
    fn injury(token: u64, e: &mut InjuryEvent, c: &mut Context<'_, Self>) -> Result<(), String> {
        c.host.trace.push(json!(["injury", c.buff, bits(e.amount)]));
        match token {
            1 => e.amount *= 0.7,
            2 => {
                c.block(e, Some(0.2))?;
            }
            3 => {
                e.defer(7)?;
                e.amount = (e.amount - 3.0).max(0.0);
            }
            4 => {
                if !c.host.nesting {
                    c.runtime.remove_id(c.world, c.host, 1, c.buff)?;
                    c.host.nesting = true;
                    let result = c.resolve(&mut request(&json!({"type":"true","amount":4})));
                    c.host.nesting = false;
                    result?;
                }
            }
            5 => {
                c.runtime.remove(c.world, c.host, 1, "b")?;
            }
            6 => {
                if !c
                    .world
                    .unit(1)?
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some("late"))
                {
                    c.runtime.add(c.world,c.host,1,spec(&json!({"key":"late","duration":0.3,"mods":{"dmgTakenMul":0.8},"observe":true})))?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn observe(_: u64, e: Observation, c: &mut Context<'_, Self>) -> Result<(), String> {
        let attached = c
            .world
            .unit(1)?
            .stats
            .buffs()
            .iter()
            .any(|b| b.runtime.id() == c.buff);
        let outcome = e.outcome.map(|o| match o {
            Outcome::Zero => "zero",
            Outcome::Blocked => "blocked",
            Outcome::Dodged => "dodged",
        });
        c.host.trace.push(json!([
            "observe",
            c.buff,
            bits(e.amount),
            e.cancelled,
            outcome,
            attached
        ]));
        c.runtime.edit(c.world, 1, c.buff, |b| b.shield += 2.0)
    }
    fn damage_notice(
        &mut self,
        n: Notice,
        _: f64,
        _: &mut Request,
        _: &mut Runtime,
        _: &mut World,
        _: &mut Rng,
    ) -> Result<(), String> {
        let (k, cancelled) = match n {
            Notice::Block { cancelled } => ("block", cancelled),
            Notice::Dodge { cancelled, .. } => ("dodge", cancelled),
        };
        self.trace.push(json!([k, cancelled]));
        Ok(())
    }
    fn damage_cleanup(&mut self, token: u64, _: &mut Runtime, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["cleanup", token]));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; canonical buff/HP settlement"]
fn canonical_buffs_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/settlement.mjs");
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
        for (k, n) in [("maxHp", 10000.0), ("def", 20.0), ("res", 30.0)] {
            base.0.insert(k.into(), n);
        }
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let mut r = Runtime::new(32, 256, 64).unwrap();
        let mut h = Host {
            trace: vec![],
            nesting: false,
        };
        let mut rng = Rng::new(v["i"].as_u64().unwrap() as u32 + 1);
        for d in v["descriptors"].as_array().unwrap() {
            r.add(&mut w, &mut h, 1, spec(d)).unwrap();
        }
        let mut states = vec![];
        for op in v["ops"].as_array().unwrap() {
            let result = if let Some(dt) = op["tick"].as_f64() {
                w.time += dt;
                r.tick(&mut w, &mut h, &[1], dt).unwrap();
                None
            } else if !op["add"].is_null() {
                r.add(&mut w, &mut h, 1, spec(&op["add"])).unwrap();
                None
            } else {
                Some(
                    settlement::resolve(&mut w, &mut r, &mut rng, &mut h, &mut request(&op["hit"]))
                        .unwrap(),
                )
            };
            let u = w.unit_mut(1).unwrap();
            let shield = u.stats.stats().unwrap().n("shield");
            states.push(json!({"result":result.map(bits),"hp":bits(u.stats.hp),"taken":bits(u.damage_taken),"rng":rng.state(),"shield":bits(shield),"buffs":u.stats.buffs().iter().map(|b|json!({"id":b.runtime.id(),"key":b.key,"stacks":b.stacks,"shield":bits(b.shield),"hits":b.runtime.shield_hits,"left":bits(b.runtime.time_left)})).collect::<Vec<_>>()}));
        }
        for (j, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "case {} step {j}", v["i"]);
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {}", v["i"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 256);
}
