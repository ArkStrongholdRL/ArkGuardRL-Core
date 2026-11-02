use arkguard_engine::battle::{
    damage::{DamageType, Penetration},
    health::{self, Event, HealOptions, Hit, Hooks, PlayerTotals, Pool, Side, Stage, Unit, World},
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
    d: Value,
    trace: Vec<Value>,
    shield: f64,
}
impl Hooks for Host {
    fn emit(&mut self, e: &mut Event, w: &mut World) -> Result<(), String> {
        let name = match e.stage {
            Stage::Fatal => "fatal",
            Stage::HpChanged => "hpChanged",
            Stage::ModifierApplied => "modifierApplied",
            Stage::DamageApplied => "damageApplied",
            Stage::Damaged => "damaged",
            Stage::Heal => "heal",
            Stage::SkillDamaged => {
                self.trace.push(json!(["skill"]));
                return Ok(());
            }
            Stage::OnApplied => {
                self.trace.push(json!(["applied"]));
                return Ok(());
            }
        };
        self.trace.push(json!([name, bits(e.amount)]));
        if e.stage == Stage::Fatal {
            match self.d["fatal"].as_u64().unwrap() {
                1 => e.prevented = true,
                2 => w.unit_mut(2).unwrap().stats.hp = 5.0,
                3 => w.unit_mut(2).unwrap().stats.alive = false,
                _ => {}
            }
        }
        if e.stage == Stage::Heal {
            e.amount *= 0.5;
        }
        Ok(())
    }
    fn kill(&mut self, t: u64, _: Option<u64>, w: &mut World) -> Result<(), String> {
        self.trace.push(json!(["kill"]));
        let u = w.unit_mut(t).unwrap();
        u.stats.alive = false;
        u.deployed = false;
        Ok(())
    }
    fn hit_cap(&mut self, _: u64, n: f64) -> Result<(), String> {
        self.trace.push(json!(["hitCap", bits(n)]));
        Ok(())
    }
    fn heal_through(&mut self, _: u64, _: u64, _: &World) -> bool {
        self.d["through"] == true
    }
    fn overheal(
        &mut self,
        _: u64,
        excess: f64,
        max: f64,
        _: Option<f64>,
        _: &mut World,
    ) -> Result<(), String> {
        self.shield = max.min(self.shield + excess);
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; HP functions only"]
fn health_and_healing_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/health.mjs");
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
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.0);
        base.0.insert("atk".into(), 100.0);
        let mut source = Unit::new(UnitStats::new(base.clone()), Side::Ally);
        source.owner = Some(10);
        source.deployed = true;
        source.stats.set_buffs(vec![Buff {
            mods: [("healingDealtMul".into(), 1.2)].into_iter().collect(),
            ..Buff::default()
        }]);
        base.0.insert("atk".into(), 0.0);
        base.0.insert("def".into(), 25.0);
        base.0.insert("res".into(), 30.0);
        let mut target = Unit::new(
            UnitStats::new(base),
            if d["targetAlly"] == true {
                Side::Ally
            } else {
                Side::Enemy
            },
        );
        target.stats.hp = 37.0;
        target.deployed = true;
        target.skill = true;
        target.boss = d["boss"] == true;
        target.stats.set_buffs(vec![Buff {
            mods: [("healingTakenMul".into(), 1.3)].into_iter().collect(),
            flags: ["noHeal", "healFree", "hitCount", "hitCountArts"]
                .into_iter()
                .filter(|k| d[*k] == true)
                .map(str::to_string)
                .collect(),
            ..Buff::default()
        }]);
        if d["pool"] == true {
            target.pool = Some(1);
        }
        let mut world = World::new(2, 1, 1).unwrap();
        world.insert_unit(1, source).unwrap();
        world.insert_unit(2, target).unwrap();
        world
            .insert_pool(
                1,
                Pool {
                    hp: 63.0,
                    max_hp: 100.0,
                },
            )
            .unwrap();
        world.insert_player(10, PlayerTotals::default()).unwrap();
        world.time = 5.0;
        world.boss_battle = true;
        let mut host = Host {
            d: d.clone(),
            trace: vec![],
            shield: 0.0,
        };
        let amount = d["amount"].as_f64().unwrap();
        let self_ = d["self"] == true;
        let source = if self_ { 2 } else { 1 };
        let result = if d["heal"] == true {
            health::heal(
                &mut world,
                &mut host,
                Some(source),
                2,
                amount,
                HealOptions {
                    regen: d["mode"] == "regen",
                    restore: d["mode"] == "restore",
                    overheal: true,
                    ..HealOptions::default()
                },
            )
            .unwrap()
        } else {
            let hit = Hit {
                target: 2,
                source: Some(1),
                credit: Some(source),
                damage_type: match d["type"].as_str().unwrap() {
                    "phys" => DamageType::Physical,
                    "arts" => DamageType::Arts,
                    "true" => DamageType::True,
                    _ => DamageType::Elemental,
                },
                no_sp: d["noSp"] == true,
                silent: false,
                hp_loss: false,
                on_applied: true,
            };
            if d["loss"] == true {
                health::lose_hp(
                    &mut world,
                    &mut host,
                    hit,
                    amount,
                    Penetration::default(),
                    false,
                )
                .unwrap()
            } else {
                health::apply_hp_decrease(&mut world, &mut host, hit, amount).unwrap()
            }
        };
        let t = world.unit(2).unwrap();
        let c = world.unit(source).unwrap();
        let pp = world.player(10).unwrap();
        let actual = json!({"result":bits(result),"hp":bits(t.stats.hp),"alive":t.stats.alive,"deployed":t.deployed,"taken":bits(t.damage_taken),"damage":bits(c.damage_dealt),"healing":bits(c.healing_done),"last":bits(t.last_hit_at),"pool":bits(world.pool(1).unwrap().hp),"pp":{"damageDealt":bits(pp.damage),"bossDamage":bits(pp.boss_damage),"healingDone":bits(pp.healing)},"shield":bits(host.shield),"trace":host.trace});
        let mut expected = v.clone();
        expected.as_object_mut().unwrap().remove("scenario");
        assert_eq!(actual, expected, "case {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 768);
}
