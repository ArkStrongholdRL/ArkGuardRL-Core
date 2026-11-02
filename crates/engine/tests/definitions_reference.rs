use arkguard_engine::battle::{damage::DamageType, definitions::Enemy, enemy::Mode};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
fn canonical(v: &mut Value) {
    match v {
        Value::Number(n) => {
            let x = n.as_f64().unwrap();
            if x.fract() == 0. && x.abs() < i64::MAX as f64 {
                *v = json!(x as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(canonical),
        Value::Object(o) => o.values_mut().for_each(canonical),
        _ => {}
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; every enemy definition and actual spawnEnemy"]
fn every_enemy_definition_and_spawn_match() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/definitions.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let c: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let key = c["key"].as_str().unwrap();
        let d = Enemy::parse(key, &c["raw"])
            .unwrap()
            .with_overrides(&c["overrides"]);
        let mut actual = snapshot(&d);
        let mut expected = c["output"].clone();
        canonical(&mut actual);
        canonical(&mut expected);
        assert_eq!(actual, expected, "definition {key}");
        let mut u = d.unit(&c["mods"], [10., 6.], 0.).unwrap();
        let base: serde_json::Map<_, _> = d
            .base
            .0
            .keys()
            .map(|k| (k.clone(), json!(bits(u.stats.base_number(k)))))
            .collect();
        let fly = u.stats.is_flying(true, u.motion_fly).unwrap();
        let actual = json!({"base":base,"hp":bits(u.stats.hp),"blockWeight":bits(u.block_weight),"fly":fly,"staticBody":u.static_body,"immune":u.immune,"position":u.position.map(bits)});
        assert_eq!(actual, c["unit"], "spawn {key}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert!(count > 200);
    eprintln!("{count} real enemy definitions and spawns");
}
#[test]
fn invalid_spawns_fail_before_creating_units() {
    let d = Enemy::parse("enemy_a", &json!({"hp":1000})).unwrap();
    assert!(d.unit(&json!({"hpMul":f64::MAX}), [0.; 2], 0.).is_err());
    assert!(d.unit(&Value::Null, [f64::NAN, 0.], 0.).is_err());
    assert!(Enemy::parse("enemy_a", &Value::Null).is_err());
}

fn snapshot(d: &Enemy) -> Value {
    let mode = match d.profile.mode {
        Mode::None => "none",
        Mode::Heal => "heal",
        Mode::Damage(DamageType::Physical) => "phys",
        Mode::Damage(DamageType::Arts) => "arts",
        Mode::Damage(DamageType::True) => "true",
        Mode::Damage(DamageType::Elemental) => "elemental",
    };
    json!({"key":d.key,"base":d.base.0,"dmgType":mode,"melee":d.profile.melee,"motion":if d.fly{"FLY"}else{"WALK"},"leader":d.leader,
            "lpr":d.lpr,"blockWeight":d.block_weight,"immune":d.immune,"epResistance":d.ep_resistance,"elementalResistance":d.elemental_resistance,
            "notCountInTotal":d.not_count_in_total,"hitArea":d.hit_area.map(|a|json!({"w":a.width,"h":a.height,"dx":a.dx,"dy":a.dy})),
            "attackAnim":d.profile.animation.map(|a|json!({"dur":a.duration,"hit":a.hit})),"attackMoves":d.profile.attack_moves,"staticBody":d.static_body})
}
