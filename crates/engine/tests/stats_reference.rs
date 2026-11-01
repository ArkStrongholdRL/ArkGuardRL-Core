use arkguard_engine::battle::{
    damage::{self, Damage, DamageType, Penetration},
    stats::{Base, Buff, Cultivation, Numbers, Stats, UnitStats},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn numbers(v: &Value) -> Numbers {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
        .collect()
}
fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}
fn same(x: f64, v: &Value) {
    let expected = u64::from_str_radix(v.as_str().unwrap(), 16).unwrap();
    if f64::from_bits(expected).is_nan() {
        assert!(x.is_nan());
    } else {
        assert_eq!(bits(x), v.as_str().unwrap(), "{x}");
    }
}
fn check(s: &Stats, v: &Value) {
    assert_eq!(s.numbers.len(), v["numbers"].as_object().unwrap().len());
    for (k, &x) in &s.numbers {
        same(x, &v["numbers"][k]);
    }
    assert_eq!(serde_json::json!(s.flags), v["flags"]);
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; numeric formulas only"]
fn stats_and_damage_match_bits() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/stats.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let mut base = Base::default();
        base.0.extend(numbers(&v["base"]));
        let hp = base.0["maxHp"] * 0.37;
        let mut u = UnitStats::new(base);
        u.hp = hp;
        u.set_cultivation(Cultivation {
            hp: v["cm"]["hp"].as_f64().unwrap(),
            atk: v["cm"]["atk"].as_f64().unwrap(),
            def: v["cm"]["def"].as_f64().unwrap(),
        });
        u.set_move_floor(v["floor"].as_f64().unwrap());
        u.set_buffs(
            v["buffs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| Buff {
                    mods: numbers(&b["mods"]),
                    flags: b["flags"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .filter(|(_, v)| v.as_bool().unwrap())
                        .map(|(k, _)| k.clone())
                        .collect(),
                    stacks: b["stacks"].as_u64().unwrap() as u32,
                    shield: b["shield"].as_f64().unwrap(),
                    permanent: b["permanent"] == true,
                })
                .collect(),
        );
        let s = u.stats().unwrap().clone();
        check(&s, &v["first"]);
        same(u.hp, &v["hp"]);
        u.set_buffs(vec![]);
        check(u.stats().unwrap(), &v["second"]);
        same(u.hp, &v["hpAfter"]);
        let target = Stats {
            numbers: ["def", "res"]
                .into_iter()
                .map(|name| {
                    let value = &v["target"]["numbers"][name];
                    (
                        name,
                        f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap()),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            flags: BTreeSet::new(),
        };
        let d = &v["dmg"];
        let dmg = Damage {
            amount: d["amount"].as_f64().unwrap(),
            mul: d["mul"].as_f64().unwrap(),
            resolved: d["calculation"] == "resolved",
            penetration: Penetration {
                def_flat: d["defIgnoreFlat"].as_f64().unwrap(),
                def_pct: d["defIgnorePct"].as_f64().unwrap(),
                res_flat: d["resIgnoreFlat"].as_f64().unwrap(),
                res_pct: d["resIgnorePct"].as_f64().unwrap(),
            },
        };
        let ep = v["ep"].as_f64().unwrap();
        for expected in v["damages"].as_array().unwrap() {
            let kind = match expected["type"].as_str().unwrap() {
                "phys" => DamageType::Physical,
                "arts" => DamageType::Arts,
                "true" => DamageType::True,
                _ => DamageType::Elemental,
            };
            same(
                damage::mitigate(dmg.amount, kind, &target, dmg.penetration, ep),
                &expected["mitigate"],
            );
            same(
                damage::attack_input(dmg.amount, kind, &s),
                &expected["input"],
            );
            same(
                damage::expected(dmg, kind, &target, ep),
                &expected["expected"],
            );
            same(
                damage::calculate(dmg, kind, &target, Some(&s), ep),
                &expected["base"],
            );
            same(
                damage::calculate(dmg, kind, &target, None, ep),
                &expected["noSource"],
            );
            same(
                damage::estimate(dmg.amount, kind, &s),
                &expected["estimate"],
            );
        }
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 768);
}
