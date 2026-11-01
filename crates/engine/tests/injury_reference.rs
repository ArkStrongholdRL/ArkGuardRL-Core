use arkguard_engine::{
    battle::{
        damage::DamageType,
        injury::{
            self, DodgeMode, Effect, EffectId, Effects, HookResult, Hooks, InjuryEvent, Notice,
            Observation, Outcome,
        },
    },
    deterministic::rng::Rng,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn kind(s: &str) -> DamageType {
    match s {
        "phys" => DamageType::Physical,
        "arts" => DamageType::Arts,
        "true" => DamageType::True,
        _ => DamageType::Elemental,
    }
}
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
fn outcome(o: Option<Outcome>) -> Value {
    match o {
        None => Value::Null,
        Some(o) => json!(match o {
            Outcome::Zero => "zero",
            Outcome::Blocked => "blocked",
            Outcome::Dodged => "dodged",
        }),
    }
}
struct Driver {
    descriptors: Vec<Value>,
    trace: Vec<Value>,
}
impl Hooks for Driver {
    fn injury(
        &mut self,
        id: EffectId,
        event: &mut InjuryEvent,
        effects: &mut Effects,
        rng: &mut Rng,
    ) -> HookResult {
        let d = self.descriptors[id as usize - 1].clone();
        self.trace.push(json!(["injury", id, bits(event.amount)]));
        match d["action"].as_u64().unwrap() {
            1 => event.amount *= d["scale"].as_f64().unwrap(),
            2 => {
                event.block(Some(d["probability"].as_f64().unwrap()), self, effects, rng)?;
            }
            3 => {
                event.dodge(Some(d["probability"].as_f64().unwrap()), self, effects, rng)?;
            }
            4 => effects.remove(d["remove"].as_u64().unwrap()),
            _ => {}
        }
        event.defer(id)?;
        Ok(())
    }
    fn observe(
        &mut self,
        id: EffectId,
        event: Observation,
        _: &mut Effects,
        _: &mut Rng,
    ) -> HookResult {
        self.trace.push(json!([
            "observe",
            id,
            bits(event.amount),
            event.cancelled,
            outcome(event.outcome)
        ]));
        Ok(())
    }
    fn notice(&mut self, n: Notice, _: &mut Effects, _: &mut Rng) -> HookResult {
        self.trace.push(match n {
            Notice::Block { cancelled } => json!(["block", cancelled]),
            Notice::Dodge { cancelled, amount } => json!(["dodge", cancelled, bits(amount)]),
        });
        Ok(())
    }
    fn cleanup(&mut self, token: u64) -> HookResult {
        self.trace.push(json!(["cleanup", token]));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; injury effects only"]
fn ordered_effects_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/injury.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let descriptors = v["descriptors"].as_array().unwrap().clone();
        let mut set = Effects::new(32).unwrap();
        let t = kind(v["type"].as_str().unwrap());
        for d in &descriptors {
            let mods = d["mods"].as_object().map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
                    .collect()
            });
            let reduction = d["reduction"].as_f64().map(|r| {
                let mut values = [0.0; 4];
                values[match t {
                    DamageType::Physical => 0,
                    DamageType::Arts => 1,
                    DamageType::True => 2,
                    DamageType::Elemental => 3,
                }] = r;
                values
            });
            set.insert(Effect {
                key: d["key"].as_str().unwrap().into(),
                priority: d["priority"].as_f64().unwrap(),
                stacks: d["stacks"].as_u64().unwrap() as u32,
                mods,
                shield: d["shield"].as_f64().unwrap(),
                shield_hits: d["hits"].as_u64().unwrap() as u32,
                shield_types: d["shieldTypes"]
                    .as_array()
                    .map(|a| a.iter().map(|v| kind(v.as_str().unwrap())).collect()),
                keep_empty_shield: d["keep"] == true,
                reduction,
                injury: d["action"] != 0,
                observer: d["observer"] == true,
                ..Effect::default()
            })
            .unwrap();
        }
        set.dirty = false;
        let mut rng = Rng::new(v["i"].as_u64().unwrap() as u32 + 1);
        let mut hooks = Driver {
            descriptors,
            trace: vec![],
        };
        let mode = match v["mode"].as_str().unwrap() {
            "ignore" => DodgeMode::Ignore,
            "normal" => DodgeMode::Normal,
            _ => DodgeMode::Uncancellable,
        };
        let result = injury::execute(
            &mut set,
            &mut rng,
            &mut hooks,
            v["amount"].as_f64().unwrap(),
            t,
            mode,
            v["ignoreBlock"] == true,
        )
        .unwrap();
        assert_eq!(
            json!({"amount":bits(result.amount),"cancelled":result.cancelled,"outcome":outcome(result.outcome),"notifyZero":result.notify_zero}),
            v["result"],
            "case {}",
            v["i"]
        );
        let mut applied = None;
        let settled = result
            .settle(
                &mut hooks,
                |a| {
                    applied = Some(a);
                    Ok(a)
                },
                0.0,
            )
            .unwrap();
        if let Some(a) = applied {
            let pos = hooks
                .trace
                .iter()
                .position(|e| e[0] == "cleanup")
                .unwrap_or(hooks.trace.len());
            hooks.trace.insert(pos, json!(["apply", bits(a)]));
        }
        assert_eq!(bits(settled), v["settled"].as_str().unwrap());
        assert_eq!(
            hooks.trace,
            v["trace"].as_array().unwrap().clone(),
            "case {}",
            v["i"]
        );
        assert_eq!(rng.state() as u64, v["rng"].as_u64().unwrap());
        for expected in v["state"].as_array().unwrap() {
            let e = set.get(expected["id"].as_u64().unwrap());
            assert_eq!(e.is_some(), expected["active"] == true);
            if let Some(e) = e {
                assert_eq!(bits(e.shield), expected["shield"].as_str().unwrap());
                assert_eq!(e.shield_hits as u64, expected["hits"].as_u64().unwrap());
            }
        }
        // Reference markDirty is called once after shield consumption; removal also dirties
        // the native set immediately, including removal from inside an injury callback.
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 1024);
}
