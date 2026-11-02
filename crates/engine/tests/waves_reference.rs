use arkguard_engine::battle::{
    ledger::{Enemy, Ledger},
    waves::{Pending, Queue, Template},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
fn queue_view(q: &Queue) -> Value {
    json!(q.pending().iter().map(|p|json!({"time":bits(p.time),"sequence":p.sequence,"key":p.spec["enemyKey"],"counted":p.counted,"inTotal":p.in_total,"precounted":p.precounted})).collect::<Vec<_>>())
}
fn same(a: &Value, b: &Value, ctx: &str) {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => assert_eq!(
            a.as_f64().unwrap().to_bits(),
            b.as_f64().unwrap().to_bits(),
            "{ctx}: {a} vs {b}"
        ),
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len(), "{ctx}");
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                same(a, b, &format!("{ctx}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.len(), b.len(), "{ctx}: {a:?} vs {b:?}");
            for (k, a) in a {
                same(a, &b[k], &format!("{ctx}.{k}"));
            }
        }
        _ => assert_eq!(a, b, "{ctx}"),
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; all wave templates and actual Battle billing"]
fn templates_and_solo_accounting_match() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/waves.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut templates = 0;
    let mut timelines = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let c: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if c["kind"] == "template" {
            let t = Template::parse(
                &c["input"],
                &c["opts"]["mods"],
                &c["opts"]["tag"],
                8192,
                4096,
            )
            .unwrap();
            if c.get("queue").is_some() {
                let mut q = Queue::new(8192).unwrap();
                for s in &t.spawns {
                    let excluded = c["excluded"].as_array().unwrap().contains(&s["enemyKey"]);
                    q.push(s, true, excluded).unwrap();
                }
                assert_eq!(queue_view(&q), c["queue"], "{}", c["key"]);
            }
            let mut output = json!({"routes":t.routes,"spawns":t.spawns,"maxPlayTime":t.max_play_time,"overrides":t.overrides,"extraRoutes":t.extra_routes});
            if c["output"].get("maxPlayTime").is_none() {
                for k in ["maxPlayTime", "overrides", "extraRoutes"] {
                    output.as_object_mut().unwrap().remove(k);
                }
            }
            same(&output, &c["output"], c["key"].as_str().unwrap());
            templates += 1;
            continue;
        }
        let mut q = Queue::new(128).unwrap();
        for s in c["spawns"].as_array().unwrap() {
            q.push(
                s,
                true,
                c["enemies"][s["enemyKey"].as_str().unwrap()]["notCountInTotal"] == true,
            )
            .unwrap();
        }
        let pending:Vec<_>=q.pending().iter().map(|p|json!({"time":bits(p.time),"sequence":p.sequence,"key":p.spec["enemyKey"],"counted":p.counted,"inTotal":p.in_total,"precounted":p.precounted})).collect();
        assert_eq!(json!(pending), c["queue"]);
        let mut l = Ledger::new(json!("p"), &q, 128, 128).unwrap();
        let mut id = 0;
        same(&l.report(0., None), &c["states"][0], "initial");
        let spawn = |p: &Pending, l: &mut Ledger| {
            let raw = &c["enemies"][p.spec["enemyKey"].as_str().unwrap()];
            Enemy::from_pending(p, raw["lpr"].as_f64().unwrap_or(1.), l.player()).unwrap()
        };
        for (i, op) in c["ops"].as_array().unwrap().iter().enumerate() {
            let time = op["time"].as_f64().unwrap();
            let mut reason = None;
            match op["op"].as_str().unwrap() {
                "process" => {
                    while let Some(p) = q.pop_due(time) {
                        id += 1;
                        let e = spawn(&p, &mut l);
                        l.spawn(id, e, p.precounted).unwrap();
                        q.next_sequence().unwrap();
                    }
                }
                "spawn" => {
                    let s = &op["def"];
                    let counted = s["countInTotal"]
                        .as_bool()
                        .unwrap_or(s["tag"] != "part" && s["tag"] != "boss");
                    let p = Pending {
                        time,
                        sequence: q.next_sequence().unwrap(),
                        spec: s.clone(),
                        counted,
                        in_total: s["inTotal"] == true && counted,
                        precounted: false,
                    };
                    id += 1;
                    let e = spawn(&p, &mut l);
                    l.spawn(id, e, false).unwrap();
                }
                "kill" => l.kill(op["id"].as_u64().unwrap(), false).unwrap(),
                "leak" => l.leak(op["id"].as_u64().unwrap()).unwrap(),
                "timeout" => {
                    l.timeout(&mut q).unwrap();
                    reason = Some("timeout");
                }
                "forced" => reason = Some("forced"),
                _ => unreachable!(),
            }
            same(
                &l.report(time, reason),
                &c["states"][i + 1],
                &format!("timeline {timelines} op {i} {op}"),
            );
        }
        timelines += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(
        templates, 39,
        "38 external wave templates plus research aliases"
    );
    assert_eq!(timelines, 160);
    eprintln!("{templates} templates; {timelines} real Battle accounting timelines");
}
