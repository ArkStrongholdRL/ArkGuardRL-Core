use arkguard_engine::battle::{
    health::{Side, Unit, World},
    sp::{Bank, Hooks, Kind},
    stats::{Base, Buff, UnitStats},
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
    enabled: bool,
    mutate: bool,
    trace: Vec<Value>,
}
impl Hooks for Host {
    fn has_gain_hook(&self) -> bool {
        self.enabled
    }
    fn gain(&mut self, a: &mut f64, r: &str, b: &mut Bank, w: &mut World) -> Result<(), String> {
        self.trace.push(json!(["spGain", r, bits(*a)]));
        *a *= 0.5;
        if self.mutate {
            b.set_multiplier(w, 0.5)?;
        }
        Ok(())
    }
}
fn snap(b: &Bank, w: &mut World, result: Option<f64>) -> Value {
    json!({"result":result.map(bits),"sp":bits(b.sp),"charges":bits(b.charges),"cost":bits(b.cost(w).unwrap()),"total":bits(b.total(w).unwrap()),"ready":b.ready(),"mul":bits(b.cost_multiplier())})
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; SP accounting only"]
fn charge_accounting_matches() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/sp.mjs");
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
        let mut w = World::new(1, 1, 1).unwrap();
        let mut u = Unit::new(UnitStats::new(Base::default()), Side::Ally);
        u.deployed = true;
        u.stats.set_buffs(vec![Buff {
            mods: [("spCostFlat".into(), v["flat"].as_f64().unwrap())]
                .into_iter()
                .collect(),
            flags: if d["noSp"] == true {
                ["noSp".into()].into_iter().collect()
            } else {
                Default::default()
            },
            ..Buff::default()
        }]);
        w.insert_unit(1, u).unwrap();
        let kind = if d["noSkill"] == true {
            Kind::Instant
        } else {
            match d["kind"].as_str().unwrap() {
                "charges" => Kind::Charges,
                "duration" => Kind::Duration,
                "ammo" => Kind::Ammo,
                "toggle" => Kind::Toggle,
                "passive" => Kind::Passive,
                _ => Kind::Instant,
            }
        };
        let mut b = Bank::new(
            1,
            kind,
            d["cost"].as_f64().unwrap(),
            d["charges"].as_u64().unwrap() as u32,
            32,
        )
        .unwrap();
        b.no_skill = d["noSkill"] == true;
        let mut h = Host {
            enabled: d["hook"] == true,
            mutate: d["mutate"] == true,
            trace: vec![],
        };
        b.set_total(&mut w, d["total"].as_f64().unwrap()).unwrap();
        b.active = d["active"] == true;
        let mut states = vec![snap(&b, &mut w, None)];
        for (amount, reason, silent, ignore) in [
            (1.0 / 30.0, "time", false, false),
            (5.0, "gift", false, true),
            (3.0, "init", true, false),
        ] {
            let got = b
                .gain(&mut w, &mut h, amount, reason, silent, ignore)
                .unwrap();
            states.push(snap(&b, &mut w, Some(got)));
        }
        b.set_multiplier(&mut w, 0.7).unwrap();
        states.push(snap(&b, &mut w, None));
        b.set_total(&mut w, 7.5).unwrap();
        states.push(snap(&b, &mut w, None));
        b.add_charge(&mut w, 0.5).unwrap();
        states.push(snap(&b, &mut w, None));
        b.set_multiplier(&mut w, f64::NAN).unwrap();
        states.push(snap(&b, &mut w, None));
        b.gain(&mut w, &mut h, 100.0, "hurt", false, false).unwrap();
        states.push(snap(&b, &mut w, None));
        assert_eq!(json!(states), v["states"], "case {d}");
        assert_eq!(json!(h.trace), v["trace"]);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 768);
}
