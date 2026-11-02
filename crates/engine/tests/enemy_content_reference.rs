use arkguard_engine::{
    battle::{
        blocking, buffs,
        content::enemies::{Kit, Runtime},
        damage::{Damage, DamageType, Penetration},
        deploy::{self, Deployment, Options},
        grid::{Grid, Rect},
        health::{self, Hit, Id, Side, Unit, World},
        hit, settlement, spawns,
        stats::{Base, Buff, UnitStats},
        status,
        targeting::{self, Profile, Queries},
        waves::Template,
    },
    data::Catalog,
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

#[test]
fn unsupported_kits_and_invalid_capacity_never_become_plain_enemies() {
    assert!(Kit::compile("enemy_unknown", &json!({}), &Value::Null, None).is_err());
    // A content alias selects the implementation; restoration belongs to the
    // actual enemy's identity, as ab.key does in the original implementation.
    let kit = Kit::compile(
        "enemy_custom",
        &json!({"effectRef":"enemy_1283_sgkill"}),
        &Value::Null,
        None,
    )
    .unwrap();
    assert!(matches!(kit, Kit::Stealth { restore: 3. }));
    let mut w = World::new(2, 1, 1).unwrap();
    for id in 1..=2 {
        w.insert_unit(id, Unit::new(UnitStats::new(Base::default()), Side::Enemy))
            .unwrap();
    }
    let mut r = buffs::Runtime::new(4, 4, 8).unwrap();
    let mut h = Host::default();
    let mut content = Runtime::new(1).unwrap();
    content
        .spawn(
            &mut w,
            &mut r,
            &mut h,
            1,
            Kit::Stealth { restore: 3. },
            &Value::Null,
        )
        .unwrap();
    assert!(content
        .spawn(
            &mut w,
            &mut r,
            &mut h,
            2,
            Kit::Times { arts_only: false },
            &Value::Null
        )
        .is_err());
    assert!(content.faulted());
    assert!(r.faulted());
    assert_eq!(content.retained(), 1);
    assert!(w.unit(2).unwrap().stats.buffs().is_empty());
    assert!(content.tick(&mut w, &mut r, &mut h, &[1], 0.1).is_err());
}

#[test]
fn removed_content_releases_capacity_and_callback_errors_fault_the_owner() {
    let mut w = World::new(2, 1, 1).unwrap();
    for id in 1..=2 {
        w.insert_unit(id, Unit::new(UnitStats::new(Base::default()), Side::Enemy))
            .unwrap();
    }
    let mut r = buffs::Runtime::new(4, 4, 8).unwrap();
    let mut h = Host::default();
    let mut content = Runtime::new(1).unwrap();
    content
        .spawn(&mut w, &mut r, &mut h, 1, Kit::Plain, &Value::Null)
        .unwrap();
    content.remove(1);
    assert_eq!(content.retained(), 0);
    content
        .spawn(
            &mut w,
            &mut r,
            &mut h,
            2,
            Kit::Refraction {
                res: 70.,
                hp_pct: 1.,
            },
            &Value::Null,
        )
        .unwrap();
    content.tick(&mut w, &mut r, &mut h, &[2], 0.1).unwrap();
    let id = w.unit(2).unwrap().stats.buffs()[0].runtime.id();
    r.edit(&mut w, 2, id, |b| b.runtime.on_remove = Some(999))
        .unwrap();
    let mut b = Buff::default();
    b.key = Some("silence".into());
    b.flags.insert("silence".into());
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
    assert!(content.tick(&mut w, &mut r, &mut h, &[2], 0.1).is_err());
    assert!(content.faulted());
    assert!(content.tick(&mut w, &mut r, &mut h, &[2], 0.1).is_err());
}
#[derive(Default)]
struct Host {
    ended: Vec<Id>,
}
struct SpawnHost<'a> {
    data: &'a Catalog,
    content: &'a mut Runtime,
    buffs: &'a mut buffs::Runtime,
    host: &'a mut Host,
}
impl spawns::Hooks for SpawnHost<'_> {
    fn spawned(&mut self, id: Id, w: &mut World, r: &mut spawns::Runtime) -> Result<(), String> {
        r.install_content(self.data, w, self.buffs, self.content, self.host, id)
    }
    fn release(&mut self, id: Id, _: &mut World) -> Result<(), String> {
        self.content.remove(id);
        Ok(())
    }
}
impl health::Hooks for Host {
    fn kill(&mut self, id: Id, _: Option<Id>, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.;
        u.stats.alive = false;
        u.deployed = false;
        u.removed = true;
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
        Err("Unexpected content callback".into())
    }
}
impl settlement::Hooks for Host {}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, id: Id, _: &mut World) -> Result<(), String> {
        self.ended.push(id);
        Ok(())
    }
}
impl deploy::Hooks for Host {
    fn initialize(
        &mut self,
        _: Id,
        _: Options,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn event(
        &mut self,
        _: deploy::Event,
        _: Id,
        _: Option<Id>,
        _: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        Ok(())
    }
    fn remove_temporary_effects(&mut self, _: Id, _: &mut World) -> Result<(), String> {
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
        Err("Unexpected before-status hook".into())
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
        Err("Unexpected status-applied hook".into())
    }
}
impl Queries for Host {
    fn skip_enemy(&mut self, _: Option<Id>, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, _: Id, _: &mut World) -> Result<f64, String> {
        Err("No priority selector".into())
    }
    fn body_distance(&mut self, _: Id, _: [f64; 2], _: &mut World) -> Result<f64, String> {
        Err("No body query".into())
    }
}
fn view(w: &mut World, h: &mut Host, rng: &Rng) -> Value {
    let stealth = targeting::enemy_stealthed(w, 1).unwrap();
    let selectable = targeting::can_target_enemy(
        w,
        h,
        Some(10000),
        1,
        Profile {
            can_hit_fly: true,
            ..Profile::default()
        },
    )
    .unwrap();
    let u = w.unit_mut(1).unwrap();
    let s = u.stats.stats().unwrap().clone();
    let stats: serde_json::Map<_, _> = ["maxHp", "res", "atk"]
        .map(|k| (k.into(), json!(bits(s.n(k)))))
        .into_iter()
        .collect();
    let buffs:Vec<_>=u.stats.buffs().iter().map(|b|{let mods:serde_json::Map<_,_>=b.mods.iter().map(|(k,n)|(k.clone(),json!(bits(*n)))).collect();json!({"key":b.key,"left":bits(b.runtime.time_left),"persist":b.runtime.persist,"visible":b.runtime.visible,"stacks":b.stacks,"flags":b.flags,"mods":mods,"restore":if b.flags.contains("stealth"){Some(bits(b.runtime.stealth_restore.unwrap_or(3.)))}else{None}})}).collect();
    json!({"hp":bits(u.stats.hp),"alive":u.stats.alive,"stealth":stealth,"selectable":selectable,"blocked":u.blocked_by(),"stats":stats,"flags":s.flags,"buffs":buffs,"rng":rng.state()})
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; original supported enemy kits, statuses and damage"]
fn original_enemy_content_matches() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let data = Catalog::load(&Path::new(&root).join("data"), 64 << 20).unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/enemy-content.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(&root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let key = v["key"].as_str().unwrap();
        let mut w = World::new(2, 1, 1).unwrap();
        let mut h = Host::default();
        let g = Grid::new(&json!({"rows":vec!["r".repeat(21);19]}), Rect::FULL).unwrap();
        let mut r = buffs::Runtime::new(32, 64, 32).unwrap();
        let mut content = Runtime::new(2).unwrap();
        let template = Template::parse(
            &json!({"spawns":[],"overrides":{key:v["overrides"]}}),
            &Value::Null,
            &Value::Null,
            2,
            32,
        )
        .unwrap();
        let mut spawns = spawns::Runtime::new(&data, template, &g, json!("p"), 2, 32, 32).unwrap();
        let id = spawns
            .spawn(
                &mut w,
                &g,
                &mut SpawnHost {
                    data: &data,
                    content: &mut content,
                    buffs: &mut r,
                    host: &mut h,
                },
                key,
                &json!({"pos":[10,5.2],"mods":v["mods"],"tag":v["tag"]}),
            )
            .unwrap();
        assert_eq!(id, 1);
        let mut b = Base::default();
        b.0.insert("maxHp".into(), 1000.);
        b.0.insert("atk".into(), 120.);
        b.0.insert("blockCnt".into(), 3.);
        let mut a = Unit::new(UnitStats::new(b), Side::Ally);
        a.ground = true;
        a.stats.alive = false;
        a.deployment.home = [10, 5];
        a.deployment.kind = deploy::Kind::Operator;
        w.insert_unit(10000, a).unwrap();
        let mut d = Deployment::new(Rect::FULL, 2, 1, 99.).unwrap();
        d.register(10000, &w).unwrap();
        d.deploy(&mut w, &mut h, 10000, Options::default()).unwrap();
        let mut rng = Rng::new(42);
        assert_eq!(view(&mut w, &mut h, &rng), v["states"][0], "spawn {key}");
        for (j, op) in v["ops"].as_array().unwrap().iter().enumerate() {
            w.time = op["time"].as_f64().unwrap();
            if op["half"] == true {
                w.unit_mut(1).unwrap().stats.hp *= 0.7;
            }
            if let Some(key) = op["status"].as_str() {
                status::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    Rect::FULL,
                    1,
                    key,
                    status::Options {
                        duration: 0.4,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            if op["block"] == true {
                blocking::check(&mut w, &d, &g, &mut h, 1).unwrap();
            }
            if op["unblock"] == true {
                blocking::unblock(&mut w, &mut h, 1).unwrap();
            }
            for id in std::mem::take(&mut h.ended) {
                blocking::restore_stealth(&mut w, &mut r, &mut h, id).unwrap();
            }
            if op["extra"] == true {
                let mut b = Buff::default();
                b.key = Some("otherStealth".into());
                b.flags.insert("stealth".into());
                b.runtime.duration = 1.5;
                b.runtime.stealth_restore = Some(0.3);
                r.add(
                    &mut w,
                    &mut h,
                    1,
                    buffs::Spec {
                        buff: b,
                        flags_present: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            let kind = match op["hit"].as_str() {
                Some("arts") => DamageType::Arts,
                Some("true") => DamageType::True,
                Some("elemental") => DamageType::Elemental,
                _ => DamageType::Physical,
            };
            let hit = Hit {
                source: Some(10000),
                credit: Some(10000),
                target: 1,
                damage_type: kind,
                no_sp: true,
                silent: false,
                hp_loss: false,
                on_applied: false,
            };
            if op.get("hit").is_some() {
                let mut req = hit::Request::new(
                    hit,
                    Damage {
                        amount: 5.7,
                        mul: 1.,
                        resolved: false,
                        penetration: Penetration::default(),
                    },
                );
                req.is_attack = true;
                settlement::resolve(&mut w, &mut r, &mut rng, &mut h, &mut req).unwrap();
            }
            if op["loss"] == true {
                health::lose_hp(
                    &mut w,
                    &mut h,
                    Hit {
                        source: None,
                        credit: None,
                        damage_type: DamageType::True,
                        ..hit
                    },
                    0.3,
                    Penetration::default(),
                    false,
                )
                .unwrap();
            }
            w.unit_mut(1).unwrap().enemy.carrier = if op["carrier"] == true {
                Some(10000)
            } else {
                None
            };
            r.tick(&mut w, &mut h, &[1, 10000], 0.1).unwrap();
            content.tick(&mut w, &mut r, &mut h, &[1], 0.1).unwrap();
            assert_eq!(
                view(&mut w, &mut h, &rng),
                v["states"][j + 1],
                "{key} step {j} {op}"
            );
        }
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert!(cases >= 100);
    eprintln!("{cases} original-kit timelines");
}
