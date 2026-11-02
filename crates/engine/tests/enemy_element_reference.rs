use arkguard_engine::{
    battle::{
        blocking, buffs,
        content::enemies::{Kit, Runtime},
        damage::{Damage, DamageType, Penetration},
        definitions::Enemy,
        element::{self, Element},
        grid::Rect,
        health::{self, Hit, Id, Side, Unit, World},
        hit, settlement,
        stats::{Base, Buff, UnitStats},
        status,
    },
    deterministic::rng::Rng,
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
fn kind(k: DamageType) -> &'static str {
    match k {
        DamageType::Physical => "phys",
        DamageType::Arts => "arts",
        DamageType::True => "true",
        DamageType::Elemental => "elemental",
    }
}
struct Host {
    content: Runtime,
    rng: Rng,
    trace: Vec<Value>,
    fail: bool,
}
impl health::Hooks for Host {
    fn kill(&mut self, id: Id, _: Option<Id>, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.;
        u.stats.alive = false;
        u.deployed = false;
        if u.side == Side::Ally {
            let kept = u
                .stats
                .buffs()
                .iter()
                .filter(|b| b.runtime.persist)
                .cloned()
                .collect();
            u.stats.set_buffs(kept);
        }
        Ok(())
    }
}
impl settlement::Hooks for Host {
    fn damage_health_event(
        &mut self,
        e: &mut health::Event,
        request: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if e.stage == health::Stage::Damaged {
            if self.fail {
                return Err("Enemy damage callback failed".into());
            }
            self.trace.push(json!([
                e.source,
                e.target,
                request.is_attack,
                request.attack_id,
                kind(request.hit.damage_type),
                bits(e.amount),
                w.unit(2)?.stats.alive,
                bits(w.unit(2)?.stats.hp)
            ]));
            if let Some(mut req) = self.content.damaged(w, request)? {
                self.rng = rng.clone();
                let result = element::apply(w, self, r, &mut req);
                *rng = self.rng.clone();
                result?;
            }
        }
        Ok(())
    }
}
impl buffs::Hooks for Host {
    fn status(&mut self, _: Id, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        _: u64,
        _: buffs::Event,
        _: buffs::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected callback".into())
    }
    fn burst_damage(
        &mut self,
        request: &mut hit::Request,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<f64, String> {
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, r, &mut rng, self, request);
        self.rng = rng;
        result
    }
    fn burst_sp_loss(&mut self, _: Id, amount: f64, w: &mut World) -> Result<(), String> {
        element::drain_sp(w, None, amount)
    }
}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        false
    }
    fn before_status(
        &mut self,
        _: &mut status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected status hook".into())
    }
    fn has_status_applied(&self) -> bool {
        false
    }
    fn status_applied(
        &mut self,
        _: &status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected status hook".into())
    }
}
impl element::Hooks for Host {
    fn battle_rect(&self) -> Rect {
        Rect::NORMAL
    }
    fn element_enabled(&self, p: element::Phase) -> bool {
        p == element::Phase::Damaged
    }
    fn element_event(
        &mut self,
        _: element::Phase,
        req: &mut element::Request,
        amount: f64,
        _: f64,
        _: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!([
            req.damage.hit.source,
            req.damage.hit.target,
            req.damage.is_attack,
            req.damage.attack_id,
            "element",
            bits(amount),
            w.unit(2)?.stats.alive,
            bits(w.unit(2)?.stats.hp)
        ]));
        Ok(())
    }
    fn element_number(&mut self, _: Id, _: f64, _: Element) -> Result<(), String> {
        Ok(())
    }
    fn burst_visual(&mut self, _: Id, _: Element, _: &World) -> Result<(), String> {
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; actual elemental enemy procs and nested bursts"]
fn elemental_attack_procs_match_original_damage_callbacks() {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/enemy-element.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let key = v["key"].as_str().unwrap();
        let mut w = World::new(2, 1, 1).unwrap();
        let def = Enemy::parse(key, &v["raw"]).unwrap();
        w.insert_unit(1, def.unit(&Value::Null, [10., 5.], 0.).unwrap())
            .unwrap();
        let mut base = Base::default();
        for (k, n) in [("maxHp", 100000.), ("atk", 0.), ("def", 70.), ("res", 20.)] {
            base.0.insert(k.into(), n);
        }
        let mut a = Unit::new(UnitStats::new(base), Side::Ally);
        a.deployed = true;
        a.position = [10., 5.];
        w.insert_unit(2, a).unwrap();
        let mut r = buffs::Runtime::new(32, 128, 64).unwrap();
        let mut content = Runtime::new(2).unwrap();
        let mut h = Host {
            content: Runtime::new(2).unwrap(),
            rng: Rng::new(42),
            trace: vec![],
            fail: false,
        };
        content
            .spawn(
                &mut w,
                &mut r,
                &mut h,
                1,
                Kit::compile(key, &v["raw"], &v["overrides"], None).unwrap(),
                &Value::Null,
            )
            .unwrap();
        h.content = content;
        for (j, op) in v["ops"].as_array().unwrap().iter().enumerate() {
            w.time = op["time"].as_f64().unwrap();
            if let Some(atk) = op["atk"].as_f64() {
                w.unit_mut(1).unwrap().stats.edit_base(|b| {
                    b.0.insert("atk".into(), atk);
                });
            }
            if op["silence"] == true {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::NORMAL,
                    1,
                    "silence",
                    status::Options {
                        duration: 1.,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            for (key, flag, duration) in [("guard", "invulnerable", 0.8), ("dodge", "", 3.)] {
                if op[key] == true {
                    let mut b = Buff::default();
                    b.key = Some(key.into());
                    b.runtime.duration = duration;
                    if key == "dodge" {
                        b.mods.insert("dodgePhys".into(), 0.3);
                        b.mods.insert("dodgeArts".into(), 0.4);
                    } else {
                        b.flags.insert(flag.into());
                    }
                    r.add(
                        &mut w,
                        &mut h,
                        2,
                        buffs::Spec {
                            buff: b,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
            }
            if op["deathTarget"] == true {
                w.unit_mut(2).unwrap().stats.hp = 1.;
            }
            if op["sourceDead"] == true {
                let u = w.unit_mut(1).unwrap();
                u.stats.alive = false;
                u.deployed = false;
            }
            let k = match op["kind"].as_str().unwrap() {
                "arts" => DamageType::Arts,
                "true" => DamageType::True,
                "elemental" => DamageType::Elemental,
                _ => DamageType::Physical,
            };
            for _ in 0..op["multi"].as_u64().unwrap() {
                let hit = Hit {
                    source: Some(1),
                    credit: Some(1),
                    target: 2,
                    damage_type: k,
                    no_sp: true,
                    silent: false,
                    hp_loss: false,
                    on_applied: false,
                };
                let mut req = hit::Request::new(
                    hit,
                    Damage {
                        amount: op["amount"].as_f64().unwrap(),
                        mul: 1.,
                        resolved: false,
                        penetration: Penetration::default(),
                    },
                );
                req.is_attack = op["attack"] == true;
                req.attack_id = op["attackId"].as_u64();
                req.is_projectile = true;
                let mut rng = h.rng.clone();
                settlement::resolve(&mut w, &mut r, &mut rng, &mut h, &mut req).unwrap();
                h.rng = rng;
            }
            r.tick(&mut w, &mut h, &[1, 2], 0.1).unwrap();
            let s = w.unit_mut(2).unwrap().stats.stats().unwrap().clone();
            let a = w.unit(2).unwrap();
            let actual = json!({"hp":bits(a.stats.hp),"alive":a.stats.alive,"gauges":a.element_buildup.map(bits),"flags":s.flags,"res":bits(s.n("res")),"def":bits(s.n("def")),"rng":h.rng.state(),"sourceDamage":bits(w.unit(1).unwrap().damage_dealt),"taken":bits(a.damage_taken),"buffs":a.stats.buffs().iter().map(|b|json!({"key":b.key,"left":bits(b.runtime.time_left)})).collect::<Vec<_>>()});
            assert_eq!(actual, v["states"][j], "{key} case {cases} step {j} {op}");
        }
        let expected = v["trace"].as_array().unwrap();
        assert_eq!(
            h.trace.len(),
            expected.len(),
            "trace length {key} case {cases}"
        );
        for (i, (a, b)) in h.trace.iter().zip(expected).enumerate() {
            assert_eq!(a, b, "metadata {key} case {cases} event {i}");
        }
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 42);
}

#[test]
fn damage_callback_failure_preserves_real_state_and_rejects_continuation() {
    let mut w = World::new(2, 1, 1).unwrap();
    for (id, side) in [(1, Side::Enemy), (2, Side::Ally)] {
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.);
        let mut u = Unit::new(UnitStats::new(base), side);
        u.deployed = true;
        w.insert_unit(id, u).unwrap();
    }
    let mut h = Host {
        content: Runtime::new(2).unwrap(),
        rng: Rng::new(42),
        trace: vec![],
        fail: true,
    };
    let mut r = buffs::Runtime::new(4, 8, 8).unwrap();
    let mut rng = Rng::new(42);
    let hit = Hit {
        source: Some(1),
        credit: Some(1),
        target: 2,
        damage_type: DamageType::True,
        no_sp: true,
        silent: false,
        hp_loss: false,
        on_applied: false,
    };
    let mut req = hit::Request::new(
        hit,
        Damage {
            amount: 10.,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    req.attack_id = Some(7);
    assert!(settlement::resolve(&mut w, &mut r, &mut rng, &mut h, &mut req).is_err());
    assert!(r.faulted());
    assert_eq!(w.unit(2).unwrap().stats.hp, 90.);
    assert_eq!(req.attack_id, Some(7));
    assert!(settlement::resolve(&mut w, &mut r, &mut rng, &mut h, &mut req).is_err());
    assert_eq!(w.unit(2).unwrap().stats.hp, 90.);
}
