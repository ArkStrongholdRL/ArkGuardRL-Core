use arkguard_engine::{
    battle::{
        damage::{Damage, DamageType, Penetration},
        health::{self, Hit, Side, Unit, World},
        hit::{self, Calculation, Phase, Request},
        injury::{DodgeMode, Effect, Effects, Notice},
        stats::{Base, Buff, UnitStats},
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
struct Host {
    d: Value,
    trace: Vec<Value>,
    sleep: bool,
    liftoff: bool,
    invulnerable: bool,
}
impl Host {
    fn update(&self, w: &mut World) -> Result<(), String> {
        w.unit_mut(2)?.stats.set_buffs(vec![Buff {
            mods: [
                ("artsAttackAddition".into(), 7.0),
                ("dmgTakenMul".into(), 1.2),
            ]
            .into_iter()
            .collect(),
            flags: [
                ("sleep", self.sleep),
                ("liftoff", self.liftoff),
                ("invulnerable", self.invulnerable),
            ]
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| k.into())
            .collect(),
            ..Buff::default()
        }]);
        Ok(())
    }
}
impl health::Hooks for Host {
    fn emit(&mut self, e: &mut health::Event, _: &mut World) -> Result<(), String> {
        let name = match e.stage {
            health::Stage::Fatal => "fatal",
            health::Stage::HpChanged => "hpChanged",
            health::Stage::ModifierApplied => "modifierApplied",
            health::Stage::DamageApplied => "damageApplied",
            health::Stage::Damaged => "damaged",
            _ => panic!("unexpected health event"),
        };
        self.trace.push(json!([name, bits(e.amount)]));
        Ok(())
    }
    fn kill(&mut self, id: u64, _: Option<u64>, w: &mut World) -> Result<(), String> {
        self.trace.push(json!(["kill"]));
        w.unit_mut(id)?.stats.alive = false;
        w.unit_mut(id)?.deployed = false;
        Ok(())
    }
    fn hit_cap(&mut self, _: u64, n: f64) -> Result<(), String> {
        self.trace.push(json!(["hitCap", bits(n)]));
        Ok(())
    }
}
impl hit::Hooks for Host {
    fn enabled(&self, _: Phase) -> bool {
        true
    }
    fn phase(
        &mut self,
        p: Phase,
        r: &mut Request,
        a: &mut f64,
        w: &mut World,
        _: &mut Effects,
        _: &mut Rng,
    ) -> Result<(), String> {
        let (name, amount) = match p {
            Phase::Hit => ("hit", r.damage.amount),
            Phase::Calculate => ("damageCalculate", r.damage.amount),
            Phase::CalculateTarget => ("damageCalculateTarget", r.damage.amount),
            Phase::Output => ("damageOutput", *a),
            Phase::BeforeApply => ("damageBeforeApply", *a),
            Phase::Applying => ("damageApplying", *a),
            Phase::HpDamage => ("hpDamage", *a),
        };
        self.trace.push(json!([name, bits(amount)]));
        let action = self.d["action"].as_u64().unwrap();
        if p == Phase::Hit {
            if action == 1 {
                r.cancelled = true;
            }
            if action == 3 {
                self.invulnerable = true;
                self.update(w)?;
            }
            if action == 4 {
                self.liftoff = true;
                self.update(w)?;
            }
        }
        if p == Phase::CalculateTarget {
            if action == 2 {
                w.unit_mut(2)?.stats.alive = false;
            }
            if action == 5 {
                w.unit_mut(2)?.stats.edit_base(|b| {
                    b.0.insert("def".into(), 120.0);
                });
            }
        }
        if p == Phase::Output && action == 6 {
            *a *= 0.5;
            r.dodge_mode = DodgeMode::Ignore;
        }
        if p == Phase::HpDamage && action == 7 {
            *a = 3.0;
        }
        Ok(())
    }
    fn notice(
        &mut self,
        n: Notice,
        _: f64,
        _: &mut Request,
        _: &mut World,
        _: &mut Effects,
        _: &mut Rng,
    ) -> Result<(), String> {
        self.trace.push(match n {
            Notice::Block { cancelled } => json!(["block", cancelled]),
            Notice::Dodge { cancelled, .. } => json!(["dodge", cancelled]),
        });
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; HP-damage pipeline only"]
fn hit_pipeline_matches() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/hit.mjs");
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
        let mut w = World::new(2, 1, 1).unwrap();
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 1000.0);
        let mut s = Unit::new(
            UnitStats::new(base.clone()),
            if d["enemy"] == true {
                Side::Enemy
            } else {
                Side::Ally
            },
        );
        s.deployed = true;
        s.motion_fly = d["fly"] == true;
        s.hits_sleep = d["hitSleep"] == true;
        w.insert_unit(1, s).unwrap();
        base.0.insert("def".into(), 60.0);
        base.0.insert("res".into(), 30.0);
        let mut t = Unit::new(UnitStats::new(base), Side::Ally);
        t.deployed = true;
        t.hidden = d["hidden"] == true;
        t.boss = d["boss"] == true;
        w.insert_unit(2, t).unwrap();
        w.time = 3.0;
        w.boss_battle = true;
        let mut host = Host {
            d: d.clone(),
            trace: vec![],
            sleep: d["sleep"] == true,
            liftoff: d["liftoff"] == true,
            invulnerable: false,
        };
        host.update(&mut w).unwrap();
        let mut effects = Effects::new(8).unwrap();
        let dodge = d["dodge"].as_f64().unwrap();
        effects
            .insert(Effect {
                mods: Some(
                    [
                        ("dmgTakenMul".into(), 1.2),
                        ("dodgePhys".into(), dodge),
                        ("dodgeArts".into(), dodge),
                    ]
                    .into_iter()
                    .collect(),
                ),
                shield: d["shield"].as_f64().unwrap(),
                ..Effect::default()
            })
            .unwrap();
        let mut r = Request::new(
            Hit {
                target: 2,
                source: if d["source"] == true { Some(1) } else { None },
                credit: if d["source"] == true { Some(1) } else { None },
                damage_type: match d["type"].as_str().unwrap() {
                    "phys" => DamageType::Physical,
                    "arts" => DamageType::Arts,
                    "true" => DamageType::True,
                    _ => DamageType::Elemental,
                },
                no_sp: true,
                silent: true,
                hp_loss: false,
                on_applied: false,
            },
            Damage {
                amount: d["amount"].as_f64().unwrap(),
                mul: 1.0,
                penetration: Penetration::default(),
                resolved: false,
            },
        );
        r.calculation = match d["calculation"].as_str().unwrap() {
            "resolved" => Calculation::Resolved,
            "precomputed" => Calculation::Precomputed,
            _ => Calculation::Formula,
        };
        r.ignore_select = d["ignoreSelect"] == true;
        r.dodge_mode = match d["mode"].as_str().unwrap() {
            "ignore" => DodgeMode::Ignore,
            "normal" => DodgeMode::Normal,
            _ => DodgeMode::Uncancellable,
        };
        let mut rng = Rng::new(d["i"].as_u64().unwrap() as u32 + 1);
        let result = hit::resolve(&mut w, &mut effects, &mut rng, &mut host, &mut r).unwrap();
        let target = w.unit(2).unwrap();
        let actual = json!({"result":bits(result),"hp":bits(target.stats.hp),"alive":target.stats.alive,"deployed":target.deployed,"shield":bits(effects.get(1).unwrap().shield),"taken":bits(target.damage_taken),"damage":bits(w.unit(1).unwrap().damage_dealt),"rng":rng.state(),"trace":host.trace});
        let mut expected = v.clone();
        expected.as_object_mut().unwrap().remove("scenario");
        assert_eq!(actual, expected, "scenario {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 1024);
}
