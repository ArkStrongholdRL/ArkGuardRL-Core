use arkguard_engine::deterministic::{
    math,
    rng::{derive_seed, Rng},
    scheduler::Scheduler,
};
use serde_json::Value;
use std::{path::Path, process::Command};
fn float(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn same(actual: f64, expected: &Value) {
    let expected = float(expected);
    if expected.is_nan() {
        assert!(actual.is_nan());
    } else {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; runs primitives only"]
fn matches_upstream_bits() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").expect("Set ARKGUARD_REFERENCE_ROOT");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/primitives.mjs");
    let output = Command::new("node").arg(script).arg(root).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        match v["kind"].as_str().unwrap() {
            "helpers" => {
                let mut r = Rng::new(v["seed"].as_u64().unwrap() as u32);
                same(r.next_f64(), &v["first"]);
                same(r.clone().next_f64(), &v["clone"]);
                assert_eq!(r.int(0), 0);
                assert_eq!(r.pick::<u32>(&[]), None);
                assert_eq!(r.weighted(&[0.0, -1.0]), None);
                let mut shuffled = [0, 1, 2, 3, 4, 5, 6, 7];
                r.shuffle(&mut shuffled);
                assert_eq!(serde_json::json!(shuffled), v["shuffled"]);
                let weights = [-1.0, 0.0, 1.0, 2.0, 3.0];
                for expected in v["weighted"].as_array().unwrap() {
                    let index = r.weighted(&weights).unwrap();
                    assert_eq!(weights[index], expected.as_f64().unwrap());
                }
                assert_eq!(r.state() as u64, v["state"].as_u64().unwrap());
            }
            "rng" => {
                let mut r = Rng::new(v["seed"].as_u64().unwrap() as u32);
                for x in v["values"].as_array().unwrap() {
                    assert_eq!(r.next_f64(), x.as_f64().unwrap());
                }
                assert_eq!(r.state() as u64, v["state"].as_u64().unwrap());
            }
            "derive" => assert_eq!(
                derive_seed(
                    v["seed"].as_u64().unwrap() as u32,
                    v["salt"].as_str().unwrap()
                ) as u64,
                v["value"].as_u64().unwrap()
            ),
            "math" => {
                let x = float(&v["x"]);
                same(math::sin(x), &v["sin"]);
                same(math::cos(x), &v["cos"]);
                same(math::round_half_even(x), &v["round"]);
            }
            "pair" => {
                let x = float(&v["x"]);
                let y = float(&v["y"]);
                same(math::hypot(x, y), &v["hypot"]);
                same(math::atan2(y, x), &v["atan2"]);
            }
            "power" => same(
                math::powi(float(&v["x"]), v["n"].as_i64().unwrap() as i32).unwrap(),
                &v["value"],
            ),
            "scheduler" => {
                let mut s = Scheduler::new(4).unwrap();
                let id = s.schedule("repeat", 2.0, true).unwrap();
                s.schedule("once", 2.0, false).unwrap();
                let mut events = vec![];
                while let Some(e) = s.pop().unwrap() {
                    events.push(serde_json::json!([e.payload, s.now()]));
                    if e.payload == "repeat" {
                        if s.now() == 4.0 {
                            s.cancel(id);
                        } else {
                            s.schedule("callback", 2.0, false).unwrap();
                        }
                    }
                }
                let expected = v["events"].as_array().unwrap();
                assert_eq!(events.len(), expected.len());
                for (actual, expected) in events.iter().zip(expected) {
                    assert_eq!(actual[0], expected[0]);
                    assert_eq!(actual[1].as_f64(), expected[1].as_f64());
                }
            }
            _ => panic!("Unknown case"),
        }
    }
}
