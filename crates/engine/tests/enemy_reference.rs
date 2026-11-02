use arkguard_engine::{
    battle::{
        blocking,
        buffs::{self, Runtime},
        damage::DamageType,
        deploy::{self, Deployment, Kind, Options},
        enemy::{self, Animation, Context, Delivery, Event, Mode, Profile},
        grid::{Grid, Rect},
        health::{self, Side, Unit, World},
        motion,
        projectiles::{self, Launch, Projectile, System},
        settlement,
        spatial::{self, RadiusRule},
        stats::{Base, UnitStats},
        status,
        targeting::{self, Queries},
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
fn dmg(k: DamageType) -> &'static str {
    match k {
        DamageType::Physical => "phys",
        DamageType::Arts => "arts",
        DamageType::True => "true",
        DamageType::Elemental => "elemental",
    }
}
struct Host {
    cfg: Value,
    tick: u32,
    trace: Vec<Value>,
    deployment: Option<Deployment>,
    flights: Option<System<Delivery>>,
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
impl buffs::Hooks for Host {
    fn status(&mut self, _: u64, _: &str, _: bool) -> Result<(), String> {
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
}
impl settlement::Hooks for Host {
    fn phase_enabled(&self, p: arkguard_engine::battle::hit::Phase) -> bool {
        p == arkguard_engine::battle::hit::Phase::Hit
    }
    fn damage_phase(
        &mut self,
        _: arkguard_engine::battle::hit::Phase,
        r: &mut arkguard_engine::battle::hit::Request,
        _: &mut f64,
        _: &mut Runtime,
        _: &mut World,
        _: &mut Rng,
    ) -> Result<(), String> {
        self.trace.push(json!([
            "hit",
            r.hit.source.unwrap(),
            r.hit.target,
            bits(r.damage.amount),
            dmg(r.hit.damage_type),
            r.attack_id.unwrap(),
            r.is_projectile
        ]));
        Ok(())
    }
}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: u64, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl deploy::Hooks for Host {
    fn initialize(
        &mut self,
        _: u64,
        _: Options,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn event(
        &mut self,
        _: deploy::Event,
        _: u64,
        _: Option<u64>,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn remove_temporary_effects(&mut self, _: u64, _: &mut World) -> Result<(), String> {
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
impl motion::Hooks for Host {
    fn check_block(&mut self, id: u64, w: &mut World, g: &mut Grid) -> Result<bool, String> {
        let d = self.deployment.take().unwrap();
        let result = blocking::check(w, &d, g, self, id);
        self.deployment = Some(d);
        result
    }
    fn hidden(&mut self, id: u64, on: bool, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.hidden = on;
        Ok(())
    }
    fn cut_attack_stand(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn leak(&mut self, id: u64, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["leak"]));
        Ok(())
    }
}
impl Queries for Host {
    fn enemy_stealthed(&mut self, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn skip_enemy(&mut self, _: Option<u64>, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, _: u64, _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
    fn body_distance(&mut self, _: u64, _: [f64; 2], _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
}
impl enemy::Hooks for Host {
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Profile, String> {
        let c = &self.cfg;
        Ok(Profile {
            mode: match c["mode"].as_str().unwrap() {
                "heal" => Mode::Heal,
                "none" => Mode::None,
                "arts" => Mode::Damage(DamageType::Arts),
                "true" => Mode::Damage(DamageType::True),
                _ => Mode::Damage(DamageType::Physical),
            },
            melee: c["melee"] == true,
            attack_moves: c["moves"] == true,
            deferred: c["deferred"] == true,
            shot: "shell".into(),
            selection: targeting::Profile {
                block_free: c["blockFree"] == true,
                ..Default::default()
            },
            max_targets: c["maxTargets"].as_f64().unwrap(),
            atk_scale: if self.tick >= 80 { 1.5 } else { 1.2 },
            animation: c["clip"].as_object().map(|_| Animation {
                duration: c["clip"]["dur"].as_f64().unwrap(),
                hit: c["clip"]["hit"].as_f64(),
            }),
            ..Default::default()
        })
    }
    fn allies_in_radius(
        &mut self,
        id: u64,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        spatial::allies_in_radius(
            w,
            &[2, 3, 4],
            w.unit(id)?.position,
            radius,
            None,
            false,
            limit,
        )
    }
    fn enemies_in_radius(
        &mut self,
        id: u64,
        radius: f64,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let origin = w.unit(id)?.position;
        spatial::enemies_in_radius(
            w,
            self,
            &[1, 5],
            origin,
            radius,
            RadiusRule::Body,
            false,
            limit,
        )
    }
    fn can_target(&mut self, _: u64, ally: u64, _: &mut World) -> Result<bool, String> {
        Ok(self.cfg["skip"] != true || ally != 3)
    }
    fn enemy_event_enabled(&self, _: Event) -> bool {
        true
    }
    fn enemy_event(
        &mut self,
        event: Event,
        c: &mut Context,
        r: &mut Runtime,
        w: &mut World,
        _: &mut Grid,
        _: &mut Rng,
    ) -> Result<(), String> {
        match event {
            Event::AttackStart => {
                self.trace.push(json!(["start", c.targets]));
                if self.cfg["interrupt"] == true {
                    let mut b = arkguard_engine::battle::stats::Buff {
                        key: Some("interrupt".into()),
                        flags: ["stun".into()].into_iter().collect(),
                        ..Default::default()
                    };
                    b.runtime.duration = 0.2;
                    r.add(
                        w,
                        self,
                        1,
                        buffs::Spec {
                            buff: b,
                            flags_present: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
            Event::PalsyTrigger => {
                self.trace.push(json!([
                    "palsyTrigger",
                    r.get(w, 1, c.palsy.unwrap())?.stacks
                ]));
                c.keep = self.cfg["keep"] == true;
            }
            Event::BeforeAttack => {
                self.trace.push(json!(["before", c.targets]));
                if self.cfg["cancel"] == true {
                    c.targets.clear();
                }
            }
            Event::Attack => {
                self.trace.push(json!(["attack", c.targets, c.deployments]));
                if self.cfg["i"].as_u64().unwrap() % 13 == 0 {
                    let atk = w.unit(1)?.stats.base_number("atk");
                    w.unit_mut(1)?.stats.edit_base(|b| {
                        b.0.insert("atk".into(), atk + 1.0);
                    });
                }
            }
        }
        Ok(())
    }
    fn attack_visual(&mut self, source: u64, target: u64, visual: &str) -> Result<(), String> {
        self.trace.push(json!(["atk", source, target, visual]));
        Ok(())
    }
    fn palsy_visual(&mut self, _: u64, _: &World) -> Result<(), String> {
        self.trace.push(json!(["palsy"]));
        Ok(())
    }
    fn deliver(
        &mut self,
        d: Delivery,
        r: &mut Runtime,
        w: &mut World,
        rng: &mut Rng,
    ) -> Result<(), String> {
        if d.projectile {
            self.flights.as_mut().unwrap().add(
                w,
                Launch {
                    from: w.unit(d.source)?.position,
                    source: Some(d.source),
                    target: Some(d.target),
                    to: None,
                    speed: 10.0,
                    hit_dead: false,
                    max_age: 10.0,
                    on_hit: true,
                    visual: "enemy".into(),
                    speed_changeable: None,
                    removable: None,
                    payload: d,
                },
            )?;
        } else {
            enemy::impact(w, r, rng, self, d, Some(d.target))?;
        }
        Ok(())
    }
}
struct Flight<'a> {
    h: &'a mut Host,
    r: &'a mut Runtime,
    rng: &'a mut Rng,
}
impl projectiles::Hooks<Delivery> for Flight<'_> {
    fn has_step(&self) -> bool {
        false
    }
    fn step(
        &mut self,
        _: &mut Projectile<Delivery>,
        _: &mut System<Delivery>,
        _: &mut World,
    ) -> Result<bool, String> {
        Ok(false)
    }
    fn impact(
        &mut self,
        p: &mut Projectile<Delivery>,
        target: Option<u64>,
        _: &mut System<Delivery>,
        w: &mut World,
    ) -> Result<(), String> {
        enemy::impact(w, self.r, self.rng, self.h, p.payload, target)?;
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; full enemy updates with real blocking and shots"]
fn enemy_updates_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/enemy.mjs");
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
        let mut w = World::new(5, 1, 1).unwrap();
        let mut g = Grid::new(
            &json!({"rows":vec!["r".repeat(21);19],"legend":{}}),
            Rect::FULL,
        )
        .unwrap();
        let mut h = Host {
            cfg: c.clone(),
            tick: 0,
            trace: vec![],
            deployment: Some(Deployment::new(Rect::FULL, 3, 1, 99.0).unwrap()),
            flights: Some(System::new(64).unwrap()),
        };
        let mut r = Runtime::new(32, 256, 64).unwrap();
        let mut rng = Rng::new(i as u32 + 1);
        let mut base = Base::default();
        for (k, n) in [
            ("maxHp", 100000.0),
            ("atk", 40.0),
            ("def", 10.0),
            ("bat", c["bat"].as_f64().unwrap()),
            ("rangeRadius", 3.0),
            ("moveSpeed", 1.0),
        ] {
            base.0.insert(k.into(), n);
        }
        let mut e = Unit::new(UnitStats::new(base), Side::Enemy);
        e.position = [10.0, 8.0];
        e.deployed = true;
        e.motion.route = Some(
            motion::Route::compile(
                &json!({"checkpoints":[{"type":"WAIT","time":0.1}],"end":[10,4]}),
                Rect::FULL,
                32,
                e.position,
            )
            .unwrap(),
        );
        w.insert_unit(1, e).unwrap();
        for (j, tile) in [[10, 6], [10, 7], [11, 7]].into_iter().enumerate() {
            let id = j as u64 + 2;
            let mut base = Base::default();
            base.0.insert(
                "maxHp".into(),
                if i % 19 == 0 && j == 0 {
                    80.0
                } else {
                    100000.0
                },
            );
            base.0.insert("blockCnt".into(), 2.0);
            let mut u = Unit::new(UnitStats::new(base), Side::Ally);
            u.ground = true;
            u.deployment.kind = Kind::Operator;
            u.deployment.home = tile;
            u.deployment.tile = tile;
            w.insert_unit(id, u).unwrap();
            let mut dep = h.deployment.take().unwrap();
            dep.register(id, &w).unwrap();
            dep.deploy(&mut w, &mut h, id, Options::default()).unwrap();
            h.deployment = Some(dep);
        }
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 30000.0);
        let mut u = Unit::new(UnitStats::new(base), Side::Enemy);
        u.position = [10.0, 8.0];
        u.stats.hp = 500.0;
        u.deployed = true;
        w.insert_unit(5, u).unwrap();
        status::apply(
            &mut w,
            &mut h,
            &mut r,
            Rect::FULL,
            1,
            "palsy",
            status::Options {
                value: Some((i % 3 + 1) as f64),
                duration: 2.0,
                ..Default::default()
            },
        )
        .unwrap();
        let mut states = vec![];
        for tick in 0..180 {
            h.tick = tick;
            w.time = tick as f64 / 30.0;
            if tick == 3 {
                w.unit_mut(2).unwrap().position = [15.0, 15.0];
            }
            if tick == 30 {
                w.unit_mut(2).unwrap().position = [10.0, 6.0];
            }
            for (at, id, key, duration) in [
                (20, 1, "stun", 0.2),
                (45, 3, "sleep", 0.2),
                (65, 1, "disarm", 0.25),
                (104, 1, "levitate", 0.1),
            ] {
                if tick == at {
                    status::apply(
                        &mut w,
                        &mut h,
                        &mut r,
                        Rect::FULL,
                        id,
                        key,
                        status::Options {
                            duration,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
            }
            if tick == 90 {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    "fear",
                    status::Options {
                        duration: 0.25,
                        source: Some(2),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            if tick == 110 {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    "attract",
                    status::Options {
                        duration: 0.25,
                        point: Some([10.0, 8.0]),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            if tick == 100 {
                w.unit_mut(1).unwrap().motion.unbalance_until = w.time + 0.3;
            }
            if tick == 80 {
                let atk = w.unit(1).unwrap().stats.base_number("atk");
                w.unit_mut(1).unwrap().stats.edit_base(|b| {
                    b.0.insert("atk".into(), atk + 3.0);
                });
            }
            if tick == 130 && i % 9 == 0 {
                let e = w.unit_mut(1).unwrap();
                e.stats.alive = false;
                e.deployed = false;
            }
            let e = w.unit_mut(1).unwrap();
            e.enemy.carrier = if (70..75).contains(&tick) {
                Some(5)
            } else {
                None
            };
            e.hidden = (50..55).contains(&tick);
            e.enemy.pause_until = if (60..64).contains(&tick) {
                3.0
            } else {
                f64::NEG_INFINITY
            };
            r.tick(&mut w, &mut h, &[1, 2, 3, 4, 5], 1.0 / 30.0)
                .unwrap();
            enemy::update(&mut w, &mut g, &mut r, &mut rng, &mut h, 1, 1.0 / 30.0, 5).unwrap();
            let mut flights = h.flights.take().unwrap();
            flights
                .update(
                    &mut w,
                    &mut Flight {
                        h: &mut h,
                        r: &mut r,
                        rng: &mut rng,
                    },
                    1.0 / 30.0,
                )
                .unwrap();
            h.flights = Some(flights);
            let e = w.unit(1).unwrap();
            states.push(json!({"hp":(1..=5).map(|id|bits(w.unit(id).unwrap().stats.hp)).collect::<Vec<_>>(),"alive":(1..=5).map(|id|w.unit(id).unwrap().stats.alive).collect::<Vec<_>>(),"pos":e.position.map(bits),"moving":e.moving,"cd":bits(e.cadence.cooldown),"swing":e.enemy.swing,"targets":e.enemy.targets(),"stand":bits(e.motion.attack_stand_until),"unbalance":bits(e.motion.unbalance_until),"blocked":e.blocked_by(),"attacks":e.attacks,"last":bits(e.last_attack_at),"flights":h.flights.as_ref().unwrap().list().len(),"rng":rng.state(),"palsy":e.stats.buffs().iter().find(|b|b.key.as_deref()==Some("palsy")).map(|b|b.stacks).unwrap_or(0)}));
        }
        for (t, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "scenario {c} tick {t}");
        }
        assert_eq!(json!(h.trace), v["trace"], "trace {c}");
        h.flights.as_mut().unwrap().clear(&mut w).unwrap();
        enemy::clear(&mut w, 1).unwrap();
        for id in 1..=5 {
            assert_eq!(w.unit_pins(id).unwrap(), 0);
        }
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 144);
}

fn boundary_fixture() -> (World, Grid, Runtime, Rng, Host) {
    let mut w = World::new(5, 1, 1).unwrap();
    for id in 1..=5 {
        let mut base = Base::default();
        for (k, n) in [
            ("maxHp", 1000.0),
            ("atk", 40.0),
            ("bat", 1.0),
            ("rangeRadius", 3.0),
        ] {
            base.0.insert(k.into(), n);
        }
        let mut u = Unit::new(
            UnitStats::new(base),
            if id == 1 || id == 5 {
                Side::Enemy
            } else {
                Side::Ally
            },
        );
        u.deployed = true;
        u.position = if id == 1 || id == 5 {
            [10.0, 8.0]
        } else {
            [10.0, 6.0]
        };
        u.deployment.kind = Kind::Operator;
        w.insert_unit(id, u).unwrap();
    }
    let g = Grid::new(
        &json!({"rows":vec!["r".repeat(21);19],"legend":{}}),
        Rect::FULL,
    )
    .unwrap();
    let h = Host {
        cfg: json!({"i":1,"mode":"phys","melee":false,"moves":false,"deferred":false,"blockFree":false,"maxTargets":1,"clip":{"dur":1.0,"hit":0.5},"skip":false,"keep":false,"cancel":false,"interrupt":false}),
        tick: 0,
        trace: vec![],
        deployment: Some(Deployment::new(Rect::FULL, 3, 1, 99.0).unwrap()),
        flights: Some(System::new(16).unwrap()),
    };
    (w, g, Runtime::new(16, 32, 16).unwrap(), Rng::new(42), h)
}
#[test]
fn locked_targets_cannot_be_freed_and_owner_removal_releases_them() {
    let (mut w, mut g, mut r, mut rng, mut h) = boundary_fixture();
    assert!(enemy::attack(&mut w, &mut g, &mut r, &mut rng, &mut h, 1, 0.0, 5).unwrap());
    assert!(w.unit(1).unwrap().enemy.swing);
    for id in 2..=4 {
        assert_eq!(w.unit_pins(id).unwrap(), 1);
    }
    let u = w.unit_mut(2).unwrap();
    u.removed = true;
    u.stats.alive = false;
    u.deployed = false;
    assert!(w.release_removed(2).is_err());
    w.unit_mut(1).unwrap().removed = true;
    w.release_removed(1).unwrap();
    for id in 2..=4 {
        assert_eq!(w.unit_pins(id).unwrap(), 0);
    }
    w.release_removed(2).unwrap();
}
#[test]
fn enemy_query_overflow_is_a_fault_without_truncating_or_pinning_targets() {
    let (mut w, mut g, mut r, mut rng, mut h) = boundary_fixture();
    assert!(enemy::update(&mut w, &mut g, &mut r, &mut rng, &mut h, 1, 1.0 / 30.0, 1).is_err());
    assert!(r.faulted());
    assert!(!w.unit(1).unwrap().enemy.swing);
    assert_eq!(w.attack_sequence(), 0);
    for id in 1..=5 {
        assert_eq!(w.unit_pins(id).unwrap(), 0);
    }
}
#[test]
fn world_attack_identity_is_shared_and_restarts_with_a_new_world() {
    let (mut w, mut g, mut r, mut rng, mut h) = boundary_fixture();
    assert_eq!(w.next_attack_id().unwrap(), 1);
    enemy::attack(&mut w, &mut g, &mut r, &mut rng, &mut h, 1, 0.0, 5).unwrap();
    w.unit_mut(1).unwrap().cadence.cooldown = 0.0;
    enemy::attack(&mut w, &mut g, &mut r, &mut rng, &mut h, 1, 1.0 / 30.0, 5).unwrap();
    assert_eq!(w.attack_sequence(), 2);
    assert_eq!(w.next_attack_id().unwrap(), 3);
    assert_eq!(World::new(1, 1, 1).unwrap().next_attack_id().unwrap(), 1);
    h.flights.as_mut().unwrap().clear(&mut w).unwrap();
    enemy::clear(&mut w, 1).unwrap();
}
