use arkguard_engine::battle::events::{self, Context, Handle, Runtime};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
fn num(v: &Value) -> f64 {
    if v == "inf" {
        f64::INFINITY
    } else if v == "nan" {
        f64::NAN
    } else {
        v.as_f64().unwrap()
    }
}
struct Payload {
    value: i64,
    depth: u32,
    added: bool,
}
struct Host {
    trace: Vec<Value>,
    handles: Vec<Handle>,
    timers: Vec<Handle>,
    nesting: bool,
}
impl events::Host<String, Payload> for Host {
    fn hook(
        &mut self,
        _: Handle,
        token: u64,
        c: &mut Context<Payload>,
        r: &mut Runtime<String>,
    ) -> Result<(), String> {
        self.trace
            .push(json!(["hook", token, c.value.value, c.value.depth]));
        match token {
            1 => c.value.value += 1,
            2 => {
                if let Some(id) = self.handles.last() {
                    r.off(*id);
                }
            }
            3 => {
                if !c.value.added {
                    c.value.added = true;
                    self.handles.push(r.on("B".into(), 1, 0.0, Some(2), false)?);
                }
            }
            4 => {
                if c.value.depth < 2 {
                    let mut nested = Context::new(Payload {
                        value: c.value.value,
                        depth: c.value.depth + 1,
                        added: false,
                    });
                    r.emit(&"B".into(), &mut nested, self)?;
                    c.value.value = nested.value.value;
                }
            }
            5 => r.off_owner(Some(1)),
            6 => c.stop_propagation = true,
            7 => {
                if c.value.depth < 1 {
                    let mut nested = Context::new(Payload {
                        value: c.value.value,
                        depth: c.value.depth + 1,
                        added: false,
                    });
                    r.emit(&"A".into(), &mut nested, self)?;
                    c.value.value = nested.value.value;
                }
            }
            8 => {
                if let Some(id) = self.handles.first() {
                    r.off(*id);
                }
            }
            9 => {
                if let Some(id) = self.timers.last() {
                    r.cancel(*id);
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn timer(&mut self, id: Handle, token: u64, r: &mut Runtime<String>) -> Result<(), String> {
        let s = r.timer(id).unwrap();
        self.trace.push(json!([
            "timer",
            token,
            id,
            s.count,
            bits(s.due),
            s.cancelled
        ]));
        match token {
            2 => {
                if !self.nesting {
                    self.timers.push(r.after(0.0, 1, Some(2), false)?);
                    self.nesting = true;
                    let result = r.run_scheduled(self);
                    self.nesting = false;
                    result?;
                }
            }
            3 => r.cancel(id),
            4 => r.off_owner(Some(1)),
            5 => r.emit(
                &"A".into(),
                &mut Context::new(Payload {
                    value: 7,
                    depth: 0,
                    added: false,
                }),
                self,
            )?,
            6 => {
                let s = r.timer_mut(id).unwrap();
                s.due -= 0.05;
                s.interval = 0.1;
            }
            7 => r.timer_mut(id).unwrap().interval = 0.0,
            _ => {}
        }
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; hook and tick timer timelines"]
fn callback_timelines_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/events.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut r = Runtime::new(128, 256, 64, 8192).unwrap();
        let mut h = Host {
            trace: vec![],
            handles: vec![],
            timers: vec![],
            nesting: false,
        };
        let mut states = vec![];
        for op in v["ops"].as_array().unwrap() {
            let mut value = None;
            match op["op"].as_str().unwrap() {
                "on" => h.handles.push(
                    r.on(
                        op["key"].as_str().unwrap().into(),
                        op["token"].as_u64().unwrap(),
                        num(&op["priority"]),
                        op["owner"].as_u64(),
                        op["once"] == true,
                    )
                    .unwrap(),
                ),
                "emit" => {
                    let mut c = Context::new(Payload {
                        value: op["value"].as_i64().unwrap(),
                        depth: op["depth"].as_u64().unwrap() as u32,
                        added: false,
                    });
                    r.emit(&op["key"].as_str().unwrap().to_string(), &mut c, &mut h)
                        .unwrap();
                    value = Some(json!([c.value.value, c.stop_propagation]));
                }
                "timer" => {
                    let token = op["token"].as_u64().unwrap();
                    h.timers.push(
                        if op["every"] == true {
                            r.every(
                                num(&op["seconds"]),
                                token,
                                op["owner"].as_u64(),
                                op["immediate"] == true,
                            )
                        } else {
                            r.after(
                                num(&op["seconds"]),
                                token,
                                op["owner"].as_u64(),
                                op["holds"] == true,
                            )
                        }
                        .unwrap(),
                    );
                }
                "run" => {
                    r.set_time(r.time() + op["dt"].as_f64().unwrap()).unwrap();
                    r.run_scheduled(&mut h).unwrap();
                }
                "off" => r.off_key(
                    &op["key"].as_str().unwrap().to_string(),
                    op["token"].as_u64(),
                ),
                _ => r.off_owner(op["owner"].as_u64()),
            }
            states.push(json!({"value":value,"hooks":[r.handles(&"A".into()),r.handles(&"B".into())],"timers":h.timers.iter().filter_map(|id|r.timer(*id).filter(|s|!s.cancelled).map(|s|json!({"id":id,"due":bits(s.due),"interval":bits(s.interval),"count":s.count,"holds":s.holds_battle}))).collect::<Vec<_>>(),"holds":r.pending_holds()}));
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
