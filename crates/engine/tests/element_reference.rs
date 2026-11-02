use arkguard_engine::{
    battle::{
        blocking,
        buffs::{self, Runtime, Spec},
        element::{self, Element, Phase},
        grid::Rect,
        health::{self, Side, Unit, World},
        hit::{self, AttackType},
        settlement,
        sp::{Bank, Kind},
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
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
fn kind(k: arkguard_engine::battle::damage::DamageType) -> &'static str {
    use arkguard_engine::battle::damage::DamageType::*;
    match k {
        Physical => "phys",
        Arts => "arts",
        True => "true",
        Elemental => "elemental",
    }
}
struct Host {
    cfg: Value,
    trace: Vec<Value>,
    rng: Rng,
    bank: Option<Bank>,
}
impl health::Hooks for Host {
    fn kill(&mut self, id: u64, _: Option<u64>, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["kill", id]));
        Ok(())
    }
}
impl settlement::Hooks for Host {}
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
    fn burst_damage(
        &mut self,
        r: &mut hit::Request,
        runtime: &mut Runtime,
        w: &mut World,
    ) -> Result<f64, String> {
        let attack = match r.attack_type {
            AttackType::Buff => "BUFF",
            AttackType::Normal => "NORMAL",
            _ => unreachable!(),
        };
        self.trace.push(json!([
            "hpHit",
            r.hit.source,
            r.hit.credit,
            bits(r.damage.amount),
            kind(r.hit.damage_type),
            r.hit.no_sp,
            attack,
            r.tags,
            r.element.map(|e| e.name())
        ]));
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, runtime, &mut rng, self, r);
        self.rng = rng;
        result
    }
    fn burst_sp_loss(&mut self, _: u64, amount: f64, w: &mut World) -> Result<(), String> {
        element::drain_sp(w, self.bank.as_mut(), amount)
    }
}
impl status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        false
    }
    fn before_status(
        &mut self,
        _: &mut status::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
    fn has_status_applied(&self) -> bool {
        false
    }
    fn status_applied(
        &mut self,
        _: &status::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
}
impl element::Hooks for Host {
    fn battle_rect(&self) -> Rect {
        Rect::FULL
    }
    fn element_enabled(&self, p: Phase) -> bool {
        p != Phase::Hit || self.cfg["hooks"] == true
    }
    fn element_event(
        &mut self,
        p: Phase,
        g: &mut element::Request,
        amount: f64,
        added: f64,
        r: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let i = self.cfg["i"].as_u64().unwrap();
        match p {
            Phase::Hit => {
                self.trace.push(json!([
                    "elementHit",
                    bits(g.damage.damage.amount),
                    bits(g.damage.damage.mul)
                ]));
                if i % 19 == 0 {
                    g.damage.cancelled = true;
                }
                if i % 23 == 0 {
                    g.damage.damage.amount *= 1.5;
                }
                if i % 29 == 0 {
                    g.convert_to_hp = Some(arkguard_engine::battle::damage::DamageType::Arts);
                }
            }
            Phase::ModifierApplied => {
                self.trace
                    .push(json!(["modifierApplied", bits(amount), bits(added)]))
            }
            Phase::Damaged => self.trace.push(json!(["damaged", bits(amount)])),
            Phase::Applied => self.trace.push(json!(["elementApplied", bits(amount)])),
            Phase::Burst => {
                self.trace.push(json!([
                    "elementBurst",
                    g.damage.element.unwrap().name(),
                    element::locked(w, 1)?
                ]));
                let got = element::apply(
                    w,
                    self,
                    r,
                    &mut element::Request::new(
                        Some(2),
                        Some(2),
                        1,
                        g.damage.element.unwrap(),
                        1000.0,
                    ),
                )?;
                self.trace.push(json!(["nested", bits(got)]));
                if i % 41 == 0 {
                    let u = w.unit_mut(1)?;
                    u.stats.alive = false;
                    u.stats.hp = 0.;
                    u.deployed = false;
                }
            }
        }
        Ok(())
    }
    fn element_number(&mut self, target: u64, amount: f64, e: Element) -> Result<(), String> {
        self.trace.push(json!([
            "dmg",
            target,
            (amount + 0.5).floor() as u64,
            e.name()
        ]));
        Ok(())
    }
    fn burst_visual(&mut self, _: u64, _: Element, _: &World) -> Result<(), String> {
        self.trace.push(json!(["burstVisual"]));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; gauge and burst/HP timelines"]
fn elemental_timelines_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/element.mjs");
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
        let i = c["i"].as_u64().unwrap();
        let mut w = World::new(2, 1, 1).unwrap();
        let side = if c["side"] == "ally" {
            Side::Ally
        } else {
            Side::Enemy
        };
        let mut base = Base::default();
        for (k, n) in [
            ("maxHp", if i % 17 == 0 { 300.0 } else { 30000.0 }),
            ("atk", 100.0),
            ("def", 100.0),
            ("res", 30.0),
        ] {
            base.0.insert(k.into(), n);
        }
        let mut u = Unit::new(UnitStats::new(base), side);
        u.deployed = true;
        u.enemy_entity = c["entity"] == true;
        u.element_leader = c["leader"] == true;
        u.ep_resistance = c["ep"].as_f64().unwrap();
        u.element_buildup[0] = 120.;
        u.element_buildup[1] = 120.;
        w.insert_unit(1, u).unwrap();
        let mut base = Base::default();
        base.0.insert("atk".into(), 999.0);
        let mut u = Unit::new(
            UnitStats::new(base),
            if side == Side::Ally {
                Side::Enemy
            } else {
                Side::Ally
            },
        );
        u.deployed = true;
        w.insert_unit(2, u).unwrap();
        let mut bank = Bank::new(
            1,
            match c["skill"].as_str().unwrap() {
                "passive" => Kind::Passive,
                "instant" => Kind::Instant,
                _ => Kind::Duration,
            },
            3.0,
            3,
            8,
        )
        .unwrap();
        bank.set_total(&mut w, 8.0).unwrap();
        bank.active = c["active"] == true;
        bank.no_skill = c["noSkill"] == true;
        let mut h = Host {
            cfg: c.clone(),
            trace: vec![],
            rng: Rng::new(42),
            bank: Some(bank),
        };
        let mut r = Runtime::new(64, 256, 64).unwrap();
        r.add(
            &mut w,
            &mut h,
            2,
            Spec {
                buff: Buff {
                    key: Some("source".into()),
                    mods: [("dmgDealtMul".into(), 2.0)].into_iter().collect(),
                    ..Default::default()
                },
                mods_present: true,
                ..Default::default()
            },
        )
        .unwrap();
        r.add(
            &mut w,
            &mut h,
            1,
            Spec {
                buff: Buff {
                    key: Some("baseline".into()),
                    mods: [
                        ("elemTakenMul".into(), if i % 3 == 0 { 1.3 } else { 1.0 }),
                        ("dodgePhys".into(), if i % 5 == 0 { 0.3 } else { 0.0 }),
                        ("dodgeArts".into(), if i % 7 == 0 { 0.3 } else { 0.0 }),
                    ]
                    .into_iter()
                    .collect(),
                    flags: ["sleep", "invulnerable"]
                        .into_iter()
                        .filter(|k| {
                            if *k == "sleep" {
                                c["sleep"] == true
                            } else {
                                c["invul"] == true
                            }
                        })
                        .map(str::to_string)
                        .collect(),
                    ..Default::default()
                },
                mods_present: true,
                flags_present: true,
                ..Default::default()
            },
        )
        .unwrap();
        let mut states = vec![];
        for op in v["ops"].as_array().unwrap() {
            let result = match op["op"].as_str().unwrap() {
                "hit" => {
                    let mut req = element::Request::new(
                        Some(2),
                        Some(2),
                        1,
                        Element::parse(op["element"].as_str().unwrap()).unwrap(),
                        op["amount"].as_f64().unwrap(),
                    );
                    req.damage.damage.mul = 1.1;
                    req.damage.ignore_sleep = i % 3 == 0;
                    Some(element::apply(&mut w, &mut h, &mut r, &mut req).unwrap())
                }
                "heal" => Some(
                    element::heal(
                        &mut w,
                        1,
                        op["amount"].as_f64().unwrap(),
                        op["element"].as_str().and_then(Element::parse),
                    )
                    .unwrap(),
                ),
                "remove" => Some(
                    r.remove(&mut w, &mut h, 1, op["key"].as_str().unwrap())
                        .unwrap() as f64,
                ),
                _ => {
                    let dt = op["dt"].as_f64().unwrap();
                    w.time += dt;
                    r.tick(&mut w, &mut h, &[1, 2], dt).unwrap();
                    None
                }
            };
            let locked = element::locked(&mut w, 1).unwrap();
            let time = w.time;
            let view = element::view(&mut w, 1, time).unwrap().map(|v| {
                json!([
                    v.element.name(),
                    bits(v.fill),
                    bits(v.cooldown_end),
                    bits(v.cooldown)
                ])
            });
            let sp = h.bank.as_ref().unwrap().total(&mut w).unwrap();
            let u = w.unit_mut(1).unwrap();
            let s = u.stats.stats().unwrap();
            let (atk, def, res) = (s.n("atk"), s.n("def"), s.n("res"));
            states.push(json!({"result":result.map(bits),"hp":bits(u.stats.hp),"alive":u.stats.alive,"gauges":u.element_buildup.map(bits),"locked":locked,"view":view,"atk":bits(atk),"def":bits(def),"res":bits(res),"sp":bits(sp),"elemStat":bits(w.unit(2).unwrap().element_damage_dealt),"dmgStat":bits(w.unit(2).unwrap().damage_dealt),"rng":h.rng.state(),"buffs":w.unit(1).unwrap().stats.buffs().iter().map(|b|json!({"key":b.key,"stacks":b.stacks,"left":bits(b.runtime.time_left),"duration":bits(b.runtime.duration),"acc":bits(b.runtime.accumulator)})).collect::<Vec<_>>()}));
        }
        for (j, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "case {c} operation {j}");
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {c}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 320);
}
