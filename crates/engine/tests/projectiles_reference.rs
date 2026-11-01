use arkguard_engine::battle::{
    health::{Side, Unit, World},
    projectiles::{self, Launch, Projectile, System},
    stats::{Base, UnitStats},
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
    d: Value,
    trace: Vec<Value>,
    spawned: bool,
}
fn fixed(from: [f64; 2], to: [f64; 2], token: u64) -> Launch<u64> {
    Launch {
        from,
        source: None,
        target: None,
        to: Some(to),
        speed: 12.0,
        hit_dead: false,
        max_age: 10.0,
        on_hit: true,
        visual: "arrow".into(),
        speed_changeable: None,
        removable: None,
        payload: token,
    }
}
impl projectiles::Hooks<u64> for Host {
    fn has_step(&self) -> bool {
        true
    }
    fn step(
        &mut self,
        p: &mut Projectile<u64>,
        s: &mut System<u64>,
        w: &mut World,
    ) -> Result<bool, String> {
        self.trace.push(json!([
            "step",
            p.payload,
            bits(p.position[1]),
            bits(p.position[0])
        ]));
        if self.d["spawnStep"] == true && !self.spawned {
            self.spawned = true;
            s.add(w, fixed(p.position, [2.0, 3.0], 10))?;
        }
        Ok(self.d["cancel"] == true && p.payload == 2)
    }
    fn impact(
        &mut self,
        p: &mut Projectile<u64>,
        target: Option<u64>,
        s: &mut System<u64>,
        w: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!([
            "impact",
            p.payload,
            target,
            bits(p.position[1]),
            bits(p.position[0])
        ]));
        if self.d["spawnHit"] == true && p.payload < 10 {
            s.add(w, fixed(p.position, [0.0, 0.0], p.payload + 100))?;
        }
        Ok(())
    }
}
fn world() -> World {
    let mut w = World::new(2, 1, 1).unwrap();
    w.insert_unit(1, Unit::new(UnitStats::new(Base::default()), Side::Ally))
        .unwrap();
    let mut target = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
    target.position = [1.0, 2.0];
    target.deployment.deploy_seq = 1;
    target.deployed = true;
    w.insert_unit(2, target).unwrap();
    w
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; projectile flight fixtures"]
fn flight_and_callback_order_match() {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/projectiles.mjs");
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
        let mut w = world();
        let mut s = System::new(32).unwrap();
        let mut h = Host {
            d: d.clone(),
            trace: vec![],
            spawned: false,
        };
        for token in [1, 2] {
            s.add(
                &mut w,
                Launch {
                    from: [0.0, 0.0],
                    source: Some(1),
                    target: Some(2),
                    to: None,
                    speed: d["speed"].as_f64().unwrap(),
                    hit_dead: d["hitDead"] == true,
                    max_age: d["maxAge"].as_f64().unwrap(),
                    on_hit: true,
                    visual: "arrow".into(),
                    speed_changeable: None,
                    removable: None,
                    payload: token,
                },
            )
            .unwrap();
        }
        let mut states = vec![];
        for tick in 0..30 {
            let target = w.unit_mut(2).unwrap();
            target.position = [1.0 + tick as f64 * 0.01, 2.0 + tick as f64 * 0.03];
            if d["die"] == true && tick == 2 {
                target.stats.alive = false;
            }
            if d["hide"] == true && tick == 2 {
                target.hidden = true;
            }
            if d["redeploy"] == true && tick == 3 {
                target.stats.alive = true;
                target.hidden = false;
                target.deployment.deploy_seq += 1;
            }
            s.update(&mut w, &mut h, 0.1).unwrap();
            states.push(json!(s.list().iter().map(|p|json!({"token":p.payload,"x":bits(p.position[1]),"y":bits(p.position[0]),"tx":bits(p.destination[1]),"ty":bits(p.destination[0]),"age":bits(p.age),"target":p.target})).collect::<Vec<_>>()));
        }
        assert_eq!(json!(states), v["states"], "flight {d}");
        assert_eq!(json!(h.trace), v["trace"], "events {d}");
        s.clear(&mut w).unwrap();
        assert_eq!(w.unit_pins(1).unwrap(), 0);
        assert_eq!(w.unit_pins(2).unwrap(), 0);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 128);
}
#[test]
fn retained_units_and_capacity_are_released_on_clear_and_failure() {
    let mut w = world();
    let mut s = System::new(1).unwrap();
    let mut p = fixed([0.0, 0.0], [1.0, 2.0], 1);
    p.source = Some(1);
    p.target = Some(2);
    s.add(&mut w, p.clone()).unwrap();
    assert!(s.add(&mut w, p.clone()).is_err());
    assert_eq!(w.unit_pins(1).unwrap(), 1);
    w.unit_mut(1).unwrap().removed = true;
    assert!(w.release_removed(1).is_err());
    s.clear(&mut w).unwrap();
    assert_eq!(w.unit_pins(1).unwrap(), 0);
    w.release_removed(1).unwrap();
    let mut w = world();
    let mut s = System::new(1).unwrap();
    s.add(&mut w, p).unwrap();
    let mut h = Host {
        d: json!({"spawnStep":true}),
        trace: vec![],
        spawned: false,
    };
    assert!(s.update(&mut w, &mut h, 0.01).is_err());
    assert!(s.faulted());
    assert_eq!(w.unit_pins(1).unwrap(), 0);
    assert_eq!(w.unit_pins(2).unwrap(), 0);
}
