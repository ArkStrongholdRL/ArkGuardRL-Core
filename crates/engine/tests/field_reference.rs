use arkguard_engine::{
    battle::{
        damage::{Damage, DamageType, Penetration},
        health::Hit,
        hit,
        runtime::{Field, Limits},
        waves::Template,
    },
    data::Catalog,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
};
fn fixture_data() -> Arc<Catalog> {
    // Minimal independently authored data, not bundled game data.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("arkguard-field-{}-{serial}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    for key in [
        "config",
        "chess",
        "bonds",
        "garrisons",
        "items",
        "bands",
        "effects",
        "choices",
        "enemies",
        "factions",
        "waves",
        "stages",
        "bosses",
        "tokens",
        "backups",
    ] {
        let value = if key == "enemies" {
            json!({"enemy_1007_slime":{"hp":100,"atk":10,"moveSpeed":1},"enemy_1005_yokai_2":{"hp":1000,"atk":10,"range":5,"applyWay":"RANGED","bat":0.1,"moveSpeed":0.1},"enemy_1200_msfjin":{"hp":30},"enemy_9011_acrefr":{"hp":100,"talents":{"bb":{"Refracting.max_hp":1,"Refracting.magic_resistance":70}}},"enemy_unported":{}})
        } else if key == "chess" {
            let mut chess = json!({"chess_char_1_02_a":{"stats":{"maxHp":100,"atk":20,"def":5,"bat":1.2,"blockCnt":3,"cost":5,"respawnTime":3},
                "skill":{"skillId":"skchr_yak_2","duration":2,"initSp":1,"spCost":1,"spType":"INCREASE_WITH_TIME","trigger":{"rule":"TAKE_DAMAGE"},"bb":{"max_hp":0.3,"def":0.1,"magic_resistance":0.6}},
                "talents":[{"index":0,"bb":{"magic_resistance":7}}],"rangeGrid":[[0,0]],"position":"MELEE","profession":"TANK","subProfessionId":"protector"}});
            for (id, skill, profession, branch, bb, talent, tb) in [
                (
                    "chess_char_1_03_a",
                    "skchr_leizi_2",
                    "CASTER",
                    "chain",
                    json!({"atk":0.8}),
                    json!({"atk_scale":1.2}),
                    json!({"attack@max_target":3,"attack@sluggish":0.5}),
                ),
                (
                    "chess_char_1_11_a",
                    "skcom_atk_up[2]",
                    "SUPPORT",
                    "slower",
                    json!({"atk":0.35}),
                    json!({"sluggish":0.1}),
                    json!({"sluggish":0.8}),
                ),
                (
                    "chess_char_1_14_a",
                    "skchr_greyy_2",
                    "CASTER",
                    "splashcaster",
                    json!({"attack_speed":50,"talent_scale":1.6}),
                    json!({"sluggish":0.4}),
                    json!({}),
                ),
            ] {
                chess[id] = json!({"stats":{"maxHp":100,"atk":20,"bat":1.,"cost":5,"respawnTime":3},
                    "skill":{"skillId":skill,"duration":2,"initSp":1,"spCost":1,"spType":"INCREASE_WITH_TIME","trigger":{"rule":"DEFAULT"},"bb":bb},
                    "talents":[{"index":0,"bb":talent}],"trait":{"bb":tb},"rangeGrid":[[0,0],[0,1],[0,2]],"position":"RANGED","profession":profession,"subProfessionId":branch});
            }
            chess["chess_char_1_07_a"] = json!({"stats":{"maxHp":100,"atk":20,"bat":1.,"cost":5,"respawnTime":3},
                "skill":{"skillId":"skchr_prove_1","skillType":"PASSIVE","duration":0,"initSp":0,"spCost":0,"maxChargeTime":0,"spType":"ON_DEPLOY","trigger":{"rule":"DEFAULT"},"bb":{"hp_ratio_drop":0.2,"atk_scale_up":0.12}},
                "talents":[{"index":0,"bb":{"prob":1.,"prob2":1.,"atk_scale":1.5}}],"rangeGrid":[[0,0],[0,1],[0,2]],"position":"RANGED","profession":"SNIPER","subProfessionId":"closerange"});
            chess
        } else {
            json!({})
        };
        std::fs::write(root.join(format!("{key}.json")), value.to_string()).unwrap();
    }
    let result = Catalog::load(&root, 1 << 20);
    std::fs::remove_dir_all(root).unwrap();
    Arc::new(result.unwrap())
}
fn stage() -> Value {
    json!({"rows":vec!["r".repeat(21);19]})
}
fn template(key: &str, count: usize) -> Template {
    Template::parse(&json!({"routes":[{"start":[10,10],"end":[10,0]}],"spawns":[{"enemyKey":key,"count":count,"time":0}]}),&Value::Null,&Value::Null,32,16).unwrap()
}
#[test]
fn field_budget_preserves_successor_and_owned_outputs() {
    let data = fixture_data();
    let mut a = Field::new(
        Arc::clone(&data),
        &stage(),
        template("enemy_1007_slime", 2),
        42,
        30.,
        Limits::default(),
    )
    .unwrap();
    let mut b = Field::new(
        data,
        &stage(),
        template("enemy_1007_slime", 2),
        42,
        30.,
        Limits::default(),
    )
    .unwrap();
    let initial = a.report().unwrap();
    assert!(a.run(0).unwrap().budget_exhausted);
    assert_eq!(a.ticks(), 0);
    assert!(a.run(17).unwrap().budget_exhausted);
    let output = a.enemy_states().unwrap();
    let old_position = output[0].position;
    for _ in 0..17 {
        b.run(1).unwrap();
    }
    assert_eq!(view(&mut a), view(&mut b));
    a.run(13).unwrap();
    assert_eq!(output[0].position, old_position);
    assert_eq!(initial["resolved"], 0);
    assert_ne!(a.enemy_states().unwrap()[0].position, old_position);
    assert!(!a.finished());
    let id = a.enemies()[0];
    let before = view(&mut a);
    assert!(a.spawn("enemy_unported", &json!({})).is_err());
    assert!(!a.faulted());
    assert_eq!(before, view(&mut a));
    assert!(a.unit(id).unwrap().stats.alive);
    a.force_end(true).unwrap();
    assert!(a.finished());
    assert_eq!(a.report().unwrap()["resolved"], 2);
    assert_eq!(a.run(20).unwrap().ticks, 0);
    assert!(!b.finished());
}
#[test]
fn field_constructor_rejects_unimplemented_mechanics_before_execution() {
    let data = fixture_data();
    assert!(Field::new(
        Arc::clone(&data),
        &stage(),
        template("enemy_unported", 1),
        42,
        3.,
        Limits::default()
    )
    .is_err());
    let mut device = stage();
    device["devices"] = json!([{"key":"trap_unported","pos":[10,5],"active":true}]);
    assert!(Field::new(
        Arc::clone(&data),
        &device,
        template("enemy_1007_slime", 1),
        42,
        3.,
        Limits::default()
    )
    .is_err());
    let mut terrain = stage();
    terrain["legend"] = json!({"r":{"terrain":"infection"}});
    assert!(Field::new(
        Arc::clone(&data),
        &terrain,
        template("enemy_1007_slime", 1),
        42,
        3.,
        Limits::default()
    )
    .is_err());
    assert!(Field::new(
        data,
        &stage(),
        template("enemy_1007_slime", 1),
        42,
        3.,
        Limits {
            units: 0,
            ..Limits::default()
        }
    )
    .is_err());
}
#[test]
fn technical_content_failure_is_never_a_terminal_rollout() {
    let mut field = Field::new(
        fixture_data(),
        &stage(),
        template("enemy_1200_msfjin", 1),
        42,
        3.,
        Limits {
            buffs_per_unit: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(field.run(1).is_err());
    assert!(field.faulted());
    assert!(!field.finished());
    assert!(field.report().is_err());
    assert!(field.enemy_states().is_err());
    assert!(field.run(1).is_err());
    assert!(field.force_end(true).is_err());
}
#[test]
fn removed_units_release_storage_and_new_fields_restart_identities() {
    let data = fixture_data();
    let mut f = Field::new(
        Arc::clone(&data),
        &stage(),
        template("enemy_1007_slime", 2),
        0,
        3.,
        Limits::default(),
    )
    .unwrap();
    f.run(1).unwrap();
    let h = Hit {
        source: None,
        credit: None,
        target: 1,
        damage_type: DamageType::True,
        no_sp: true,
        silent: false,
        hp_loss: false,
        on_applied: false,
    };
    let mut q = hit::Request::new(
        h,
        Damage {
            amount: 200.,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    f.damage(&mut q).unwrap();
    assert_eq!(f.retained_units(), 2);
    f.run(1).unwrap();
    assert_eq!(f.retained_units(), 1);
    assert!(f.unit(1).is_err());
    assert_eq!(f.report().unwrap()["killed"], 1);
    let id = f.spawn("enemy_1007_slime", &json!({"pos":[10,8]})).unwrap();
    assert_eq!(id, 3);
    let mut fresh = Field::new(
        data,
        &stage(),
        template("enemy_1007_slime", 1),
        0,
        3.,
        Limits::default(),
    )
    .unwrap();
    fresh.run(1).unwrap();
    assert_eq!(fresh.enemies(), &[1]);
}
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
fn view(f: &mut Field) -> Value {
    let states = f.enemy_states().unwrap();
    let reason = f.reason().map(|r| match r {
        arkguard_engine::battle::clock::End::Cleared => "cleared",
        arkguard_engine::battle::clock::End::Timeout => "timeout",
        _ => "forced",
    });
    let units:Vec<_>=states.into_iter().map(|e|json!({"id":e.id,"key":e.key,"pos":e.position.map(bits),"hp":bits(e.hp),"maxHp":bits(e.max_hp),"atk":bits(e.atk),"res":bits(e.res),"cd":bits(e.cooldown),"moving":e.moving,"hidden":e.hidden,"leg":e.route_leg,"spawn":e.spawn_sequence,"deploy":e.deployment_sequence,"element":e.element.map(bits),"flags":e.flags})).collect();
    json!({"time":bits(f.time()),"ticks":f.ticks(),"finished":f.finished(),"reason":reason,"pending":f.pending_spawns(),"sequence":f.spawn_sequence(),"rng":f.rng_state(),"units":units,"report":f.report().unwrap()})
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
fn encode_bits(v: &mut Value) {
    match v {
        Value::Number(n) => *v = json!(bits(n.as_f64().unwrap())),
        Value::Array(a) => a.iter_mut().for_each(encode_bits),
        Value::Object(o) => o.values_mut().for_each(encode_bits),
        _ => {}
    }
}
fn combat_view(f: &mut Field) -> Value {
    let enemies:Vec<_>=f.enemy_states().unwrap().into_iter().map(|e|json!({
        "id":e.id,"key":e.key,"pos":e.position,"hp":e.hp,"maxHp":e.max_hp,"atk":e.atk,"res":e.res,"cd":e.cooldown,
        "moving":e.moving,"hidden":e.hidden,"leg":e.route_leg,"spawn":e.spawn_sequence,"deploy":e.deployment_sequence,
        "element":e.element,"flags":e.flags,"buffs":f.unit(e.id).unwrap().stats.buffs().iter().map(|b|json!({"key":b.key,"left":b.runtime.time_left,"stacks":b.stacks,"persist":b.runtime.persist})).collect::<Vec<_>>()
    })).collect();
    let reason = f.reason().map(|r| match r {
        arkguard_engine::battle::clock::End::Cleared => "cleared",
        arkguard_engine::battle::clock::End::Timeout => "timeout",
        _ => "forced",
    });
    let mut out = json!({"time":f.time(),"ticks":f.ticks(),"finished":f.finished(),"reason":reason,"rng":f.rng_state(),
        "dp":f.deployment_points().unwrap(),"projectiles":f.projectile_count(),"enemies":enemies,"allies":f.ally_states().unwrap(),"report":f.report().unwrap()});
    encode_bits(&mut out);
    out
}
fn first_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    if a == b {
        return None;
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                if let Some(d) = first_difference(
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                    &format!("{path}.{key}"),
                ) {
                    return Some(d);
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(d) = first_difference(a, b, &format!("{path}[{i}]")) {
                    return Some(d);
                }
            }
        }
        _ => {}
    }
    Some(format!("{path}: native={a} reference={b}"))
}
fn operator_home(home: [i32; 2]) -> arkguard_engine::battle::operators::Placement {
    arkguard_engine::battle::operators::Placement {
        uid: 1,
        home,
        direction: arkguard_engine::battle::direction::Direction::Right,
    }
}
#[test]
fn ranged_fields_share_native_delivery_and_keep_main_hit_status_scope() {
    let data = fixture_data();
    for (key, victims, status_victims) in [
        ("chess_char_1_03_a", 3, 3),
        ("chess_char_1_11_a", 1, 1),
        ("chess_char_1_14_a", 4, 1),
    ] {
        let mut f = Field::new(
            Arc::clone(&data),
            &stage(),
            template("enemy_1007_slime", 4),
            42,
            30.,
            Limits::default(),
        )
        .unwrap();
        f.add_operator(key, operator_home([10, 8])).unwrap();
        f.run(1).unwrap();
        assert_eq!(f.unit(1).unwrap().attacks, 1);
        assert_eq!(f.projectile_count(), 1);
        let output = f.ally_states().unwrap();
        f.run(7).unwrap();
        assert_eq!(
            f.enemies()
                .iter()
                .filter(|id| f.unit(**id).unwrap().damage_taken > 0.)
                .count(),
            victims,
            "{key}"
        );
        assert_eq!(
            f.enemies()
                .iter()
                .filter(|id| !f.unit(**id).unwrap().stats.buffs().is_empty())
                .count(),
            status_victims,
            "{key}"
        );
        assert_eq!(output[0]["activations"], 0);
        f.run(100).unwrap();
        assert!(
            f.ally_states().unwrap()[0]["activations"].as_u64().unwrap() >= 1,
            "{key}"
        );
    }
}
#[test]
fn unsupported_range_offsets_fail_before_operator_installation() {
    use arkguard_engine::battle::operators::Definition;
    let data = fixture_data();
    let mut raw = data.table("chess").unwrap()["chess_char_1_03_a"].clone();
    raw["rangeGrid"] = json!([[i32::MIN, 0]]);
    assert!(Definition::compile("chess_char_1_03_a", &raw).is_err());
    raw["rangeGrid"] = json!(vec![[0, 0]; 400]);
    assert!(Definition::compile("chess_char_1_03_a", &raw).is_err());
}
#[test]
fn allied_field_pauses_rejects_invalid_setup_and_preserves_output_ownership() {
    let data = fixture_data();
    let create = || {
        Field::new(
            Arc::clone(&data),
            &stage(),
            template("enemy_1007_slime", 2),
            42,
            30.,
            Limits::default(),
        )
        .unwrap()
    };
    let mut a = create();
    let mut b = create();
    assert!(a
        .add_operator("chess_unported", operator_home([10, 6]))
        .is_err());
    assert!(a
        .add_operator("chess_char_1_02_a", operator_home([-1, 6]))
        .is_err());
    assert!(!a.faulted());
    assert_eq!(
        a.add_operator("chess_char_1_02_a", operator_home([10, 6]))
            .unwrap(),
        1
    );
    assert!(a
        .add_operator("chess_char_1_02_a", operator_home([10, 6]))
        .is_err());
    b.add_operator("chess_char_1_02_a", operator_home([10, 6]))
        .unwrap();
    a.run(300).unwrap();
    for _ in 0..10 {
        b.run(30).unwrap();
    }
    assert_eq!(combat_view(&mut a), combat_view(&mut b));
    let old = a.ally_states().unwrap();
    let saved = old.clone();
    assert!(!a.unit(1).unwrap().blocking().is_empty());
    let before = combat_view(&mut a);
    assert!(a
        .add_operator("chess_char_1_02_a", operator_home([11, 6]))
        .is_err());
    assert!(a.retreat_operator(999).is_err());
    assert_eq!(combat_view(&mut a), before);
    a.run(900).unwrap();
    assert_eq!(old, saved);
    assert!(a.finished());
    assert_eq!(a.report().unwrap()["killed"], 2);
    assert_eq!(a.unit(1).unwrap().deployment.kills, 2);
}
#[test]
fn passive_hit_callbacks_preserve_main_hit_rng_scope_and_continuous_hp_scaling() {
    let mut f = Field::new(
        fixture_data(),
        &stage(),
        template("enemy_1007_slime", 1),
        42,
        30.,
        Limits::default(),
    )
    .unwrap();
    f.add_operator("chess_char_1_07_a", operator_home([10, 8]))
        .unwrap();
    f.run(1).unwrap();
    assert_eq!(f.ally_states().unwrap()[0]["active"], true);
    assert!(f.unit(1).unwrap().stats.buffs().is_empty());
    let mut q = hit::Request::new(
        Hit {
            source: Some(1),
            credit: Some(1),
            target: 2,
            damage_type: DamageType::True,
            no_sp: false,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount: 10.,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    let seed = f.rng_state();
    q.is_attack = true;
    q.tags.push("chain".into());
    assert_eq!(f.damage(&mut q).unwrap(), 10.);
    assert_eq!(f.rng_state(), seed);
    q.damage.amount = 10.;
    q.is_attack = false;
    q.tags.clear();
    assert_eq!(f.damage(&mut q).unwrap(), 10.);
    assert_eq!(f.rng_state(), seed);
    q.damage.amount = 10.;
    q.is_attack = true;
    q.is_splash = true;
    let dealt = f.damage(&mut q).unwrap();
    assert_eq!(f.rng_state(), seed);
    assert_eq!(q.damage.amount, 11.2);
    assert_eq!(dealt, 80. - (80. - 11.2));
    q.damage.amount = 0.;
    q.is_splash = false;
    assert_eq!(f.damage(&mut q).unwrap(), 0.);
    assert_ne!(f.rng_state(), seed);
}
#[test]
fn external_field_commands_use_current_successor_time_for_death_and_respawn() {
    let mut f = Field::new(
        fixture_data(),
        &stage(),
        template("enemy_1007_slime", 2),
        42,
        30.,
        Limits::default(),
    )
    .unwrap();
    f.add_operator("chess_char_1_02_a", operator_home([10, 6]))
        .unwrap();
    f.run(30).unwrap();
    let at = f.time();
    f.retreat_operator(1).unwrap();
    assert_eq!(f.unit(1).unwrap().deployment.death_at, at);
    assert_eq!(f.unit(1).unwrap().deployment.respawn_at, at + 3.);
    f.run(100).unwrap();
    assert!(f.unit(1).unwrap().stats.alive);
    assert_eq!(f.unit(1).unwrap().deployment.deploy_seq, 4); // two enemy deployments precede redeploy
    let mut q = hit::Request::new(
        Hit {
            source: None,
            credit: None,
            target: 1,
            damage_type: DamageType::True,
            no_sp: true,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount: 1.,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    let at = f.time();
    f.damage(&mut q).unwrap();
    assert_eq!(f.unit(1).unwrap().last_hit_at, at);
    let id = f.spawn("enemy_1007_slime", &json!({"pos":[10,9]})).unwrap();
    assert_eq!(f.unit(id).unwrap().deployment.deployed_at, at);
}
#[test]
fn projectile_capacity_failure_faults_the_field_without_truncating_flights() {
    let mut f = Field::new(
        fixture_data(),
        &stage(),
        template("enemy_1005_yokai_2", 3),
        42,
        30.,
        Limits {
            projectiles: 1,
            ..Default::default()
        },
    )
    .unwrap();
    f.add_operator("chess_char_1_02_a", operator_home([10, 6]))
        .unwrap();
    assert!(f.run(1).is_err());
    assert!(f.faulted());
    assert_eq!(f.projectile_count(), 1);
    assert!(!f.finished());
    assert!(f.report().is_err());
    assert!(f.ally_states().is_err());
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; concrete full-content allied field"]
fn allied_fields_match_full_battle() {
    use arkguard_engine::battle::{direction::Direction, operators::Placement};
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let data = Arc::new(Catalog::load(&Path::new(&root).join("data"), 64 << 20).unwrap());
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/field-allies.mjs");
    let mut child = Command::new("node")
        .args([script.to_str().unwrap(), &root])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    let mut ticks = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let o = &v["options"];
        let mut template = Template::parse(o, &Value::Null, &Value::Null, 32, 128).unwrap();
        template.spawns = o["spawns"].as_array().unwrap().clone();
        let mut field = Field::new(
            Arc::clone(&data),
            &o["stage"],
            template,
            o["seed"].as_u64().unwrap() as u32,
            o["timeLimit"].as_f64().unwrap(),
            Limits::default(),
        )
        .unwrap();
        for u in o["players"][0]["units"].as_array().unwrap() {
            field
                .add_operator(
                    u["chessId"].as_str().unwrap(),
                    Placement {
                        uid: u["uid"].as_u64().unwrap(),
                        home: [
                            u["row"].as_i64().unwrap() as i32,
                            u["col"].as_i64().unwrap() as i32,
                        ],
                        direction: match u["dir"].as_str().unwrap_or("RIGHT") {
                            "RIGHT" => Direction::Right,
                            "UP" => Direction::Up,
                            "LEFT" => Direction::Left,
                            "DOWN" => Direction::Down,
                            _ => panic!("Unknown reference direction"),
                        },
                    },
                )
                .unwrap();
        }
        for (i, expected) in v["states"].as_array().unwrap().iter().enumerate() {
            for op in v["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|op| op["tick"].as_u64() == Some(i as u64))
            {
                if op["op"] == "retreat" {
                    field.retreat_operator(op["id"].as_u64().unwrap()).unwrap();
                } else {
                    let mut q = hit::Request::new(
                        Hit {
                            source: None,
                            credit: None,
                            target: op["id"].as_u64().unwrap(),
                            damage_type: DamageType::True,
                            no_sp: true,
                            silent: false,
                            hp_loss: false,
                            on_applied: false,
                        },
                        Damage {
                            amount: op["amount"].as_f64().unwrap(),
                            mul: 1.,
                            resolved: false,
                            penetration: Penetration::default(),
                        },
                    );
                    field.damage(&mut q).unwrap();
                }
            }
            field.run(1).unwrap_or_else(|e| {
                panic!(
                    "case {cases} tick {i} key {}: {e}",
                    o["spawns"][0]["enemyKey"]
                )
            });
            if let Some(diff) = first_difference(&combat_view(&mut field), expected, "") {
                panic!(
                    "case {cases} tick {i} key {}: {diff}",
                    o["spawns"][0]["enemyKey"]
                );
            }
            ticks += 1;
        }
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 116);
    eprintln!("Compared {cases} allied fields / {ticks} ticks");
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; concrete native Field against full upstream Battle"]
fn concrete_fields_match_full_content() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let data = Arc::new(Catalog::load(&Path::new(&root).join("data"), 64 << 20).unwrap());
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/field.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(&root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    let mut ticks = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let o = &v["options"];
        let mut t = Template::parse(o, &Value::Null, &Value::Null, 256, 128).unwrap();
        t.spawns = o["spawns"].as_array().unwrap().clone();
        let mut f = Field::new(
            Arc::clone(&data),
            &o["stage"],
            t,
            o["seed"].as_u64().unwrap() as u32,
            o["timeLimit"].as_f64().unwrap(),
            Limits {
                units: 256,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(f.run(0).unwrap().budget_exhausted);
        assert_eq!(f.ticks(), 0);
        for (i, expected) in v["states"].as_array().unwrap().iter().enumerate() {
            for op in v["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|x| x["tick"].as_u64() == Some(i as u64))
            {
                match op["op"].as_str().unwrap() {
                    "spawn" => {
                        f.spawn(op["key"].as_str().unwrap(), &op["options"])
                            .unwrap();
                    }
                    "hitChain" => {
                        for _ in 0..30 {
                            let h = Hit {
                                source: op["source"].as_u64(),
                                credit: op["source"].as_u64(),
                                target: op["id"].as_u64().unwrap(),
                                damage_type: DamageType::True,
                                no_sp: false,
                                silent: false,
                                hp_loss: false,
                                on_applied: false,
                            };
                            let mut q = hit::Request::new(
                                h,
                                Damage {
                                    amount: 0.,
                                    mul: 1.,
                                    resolved: false,
                                    penetration: Penetration::default(),
                                },
                            );
                            q.is_attack = true;
                            q.attack_id = Some(700);
                            f.damage(&mut q).unwrap();
                        }
                    }
                    "damage" => {
                        let h = Hit {
                            source: None,
                            credit: None,
                            target: op["id"].as_u64().unwrap(),
                            damage_type: DamageType::True,
                            no_sp: false,
                            silent: false,
                            hp_loss: false,
                            on_applied: false,
                        };
                        f.damage(&mut hit::Request::new(
                            h,
                            Damage {
                                amount: op["spec"]["amount"].as_f64().unwrap(),
                                mul: 1.,
                                resolved: false,
                                penetration: Penetration::default(),
                            },
                        ))
                        .unwrap();
                    }
                    _ => unreachable!(),
                }
            }
            let advanced = f.run(1).unwrap();
            assert_eq!(advanced.ticks, 1);
            let mut actual = view(&mut f);
            let mut expected = expected.clone();
            canonical(&mut actual);
            canonical(&mut expected);
            assert_eq!(actual, expected, "field {cases} tick {i}");
            ticks += 1;
        }
        let report = f.report().unwrap();
        let before = report.clone();
        assert_eq!(f.run(17).unwrap().ticks, 0);
        assert_eq!(report, before);
        assert!(!f.faulted());
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 78);
    eprintln!("{cases} concrete full-content fields; {ticks} ticks");
}
