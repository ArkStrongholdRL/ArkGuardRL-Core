use arkguard_engine::{
    battle::{
        blocking,
        deploy::{self, Deployment, Options},
        grid::{Grid, Rect},
        health::{Id, Side, Unit, World},
        search::{self, AttackProfile, Commitments, Heal},
        stats::{Base, Buff, UnitStats},
        targeting::{Priority, Profile, Queries},
    },
    deterministic::math::hypot,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
struct Host {
    fixture: Value,
    p: AttackProfile,
}
impl Queries for Host {
    fn enemy_stealthed(&mut self, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn skip_enemy(&mut self, _: Option<Id>, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, id: Id, _: &mut World) -> Result<f64, String> {
        Ok(10.0 - id as f64)
    }
    fn body_distance(&mut self, id: Id, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        let q = w.unit(id)?.position;
        Ok(hypot(q[0] - p[0], q[1] - p[1]))
    }
}
impl Host {
    fn list(&self, key: &str) -> Vec<Id> {
        self.fixture[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect()
    }
}
impl search::Host for Host {
    fn enemies(
        &mut self,
        _: Id,
        keys: &[usize],
        _: Profile,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Ok(self.list(if keys[0] == 1 { "enemies" } else { "near" }))
    }
    fn injured_allies(
        &mut self,
        _: Id,
        _: &[usize],
        _: bool,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Ok(self.list("heal"))
    }
    fn has_ally_targets(&self) -> bool {
        !self.list("extra").is_empty() || !self.list("nearExtra").is_empty()
    }
    fn ally_targets(
        &mut self,
        _: Id,
        keys: &[usize],
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Ok(self.list(if keys[0] == 1 { "extra" } else { "nearExtra" }))
    }
    fn profile(&mut self, _: Id, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(self.p))
    }
    fn range(&mut self, _: Id, _: &World) -> Result<Vec<usize>, String> {
        Ok(vec![1])
    }
    fn skill_active(&self, _: Id, _: &World) -> Result<bool, String> {
        Ok(true)
    }
    fn can_attack(&mut self, _: Id, _: AttackProfile, _: &mut World) -> Result<bool, String> {
        Ok(true)
    }
}
struct Deploy;
impl blocking::Hooks for Deploy {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl deploy::Hooks for Deploy {
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
fn setup(v: &Value) -> (World, Host) {
    let d = &v["scenario"];
    let mut w = World::new(9, 1, 1).unwrap();
    for id in 1..=9 {
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.0);
        base.0.insert("blockCnt".into(), 3.0);
        let mut u = Unit::new(
            UnitStats::new(base),
            if id == 1 || id == 9 {
                Side::Ally
            } else {
                Side::Enemy
            },
        );
        u.deployment.kind = deploy::Kind::Operator;
        u.deployment.home = [10, 4];
        u.position = [10.0, 4.0];
        u.ground = true;
        u.stats.hp = id as f64 * 10.0;
        u.deployed = true;
        u.targeting.spawn_seq = 10 - id;
        w.insert_unit(id, u).unwrap();
    }
    if d["blocked"] == true {
        let mut dep = Deployment::new(Rect::FULL, 9, 1, 99.0).unwrap();
        dep.register(1, &w).unwrap();
        w.unit_mut(1).unwrap().deployed = false;
        dep.deploy(&mut w, &mut Deploy, 1, Options::default())
            .unwrap();
        w.unit_mut(1).unwrap().stats.hp = 10.0;
        let g = Grid::new(&json!({"rows":vec!["r".repeat(21);19]}), Rect::FULL).unwrap();
        assert!(blocking::check(&mut w, &dep, &g, &mut Deploy, 2).unwrap());
    }
    let a = w.unit_mut(1).unwrap();
    a.stats.set_buffs(vec![Buff {
        mods: [("maxTargets".into(), d["extraTargets"].as_f64().unwrap())]
            .into_iter()
            .collect(),
        ..Buff::default()
    }]);
    a.cadence.cooldown = d["cooldown"].as_f64().unwrap();
    a.cadence.had_target = d["had"].as_bool();
    let p = AttackProfile {
        selection: Profile {
            can_hit_fly: true,
            ..Profile::default()
        },
        priority: Priority::LowestHp,
        heal: if d["heal"] == true {
            Some(Heal {
                multi: d["multi"] == true,
                count: d["healCount"].as_f64().unwrap(),
                hp_at_most: d["hpAtMost"].as_f64().unwrap(),
                element_heal: false,
            })
        } else {
            None
        },
        heal_attack: d["heal"] == true,
        fortress: d["fortress"] == true,
        all_in_range: d["all"] == true,
        range_aoe: d["rangeAoe"] == true,
        hit_all_blocked: d["hitAll"] == true,
        max_targets: d["maxTargets"].as_f64().unwrap(),
        ..AttackProfile::default()
    };
    (
        w,
        Host {
            fixture: v["fixture"].clone(),
            p,
        },
    )
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; acquire and commitment only"]
fn acquisition_and_commitment_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/search.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let (mut w, mut h) = setup(&v);
        let p = h.p;
        let got = search::acquire(&mut w, &mut h, 1, &[1], p, 32).unwrap();
        assert_eq!(json!(got.targets), v["selected"], "case {}", v["scenario"]);
        assert_eq!(json!(got.fortress_melee), v["melee"]);
        let mut cache = Commitments::new(2, 32, 32).unwrap();
        cache.capture(&mut w, &mut h, &[1], 1.0 / 30.0).unwrap();
        assert_eq!(
            json!(cache.targets(&mut w, &mut h, 1, p).unwrap()),
            v["raw"],
            "capture {}",
            v["scenario"]
        );
        w.unit_mut(2).unwrap().stats.alive = false;
        w.unit_mut(3).unwrap().stats.set_buffs(vec![Buff {
            flags: ["untargetable".into()].into_iter().collect(),
            ..Buff::default()
        }]);
        assert_eq!(
            json!(cache.targets(&mut w, &mut h, 1, p).unwrap()),
            v["fallback"],
            "fallback {}",
            v["scenario"]
        );
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 512);
}
#[test]
fn overflow_invalidates_previous_snapshot_and_empty_tick_replaces_it() {
    let v = json!({"scenario":{"blocked":false,"extraTargets":0,"cooldown":0,"heal":false,"fortress":false,"maxTargets":5},"fixture":{"enemies":[3,4],"near":[],"extra":[],"nearExtra":[],"heal":[]}});
    let (mut w, mut h) = setup(&v);
    let p = h.p;
    let mut cache = Commitments::new(2, 4, 4).unwrap();
    cache.capture(&mut w, &mut h, &[1], 1.0 / 30.0).unwrap();
    assert!(!cache.targets(&mut w, &mut h, 1, p).unwrap().is_empty());
    h.fixture["enemies"] = json!([2, 3, 4, 5, 6]);
    assert!(cache.capture(&mut w, &mut h, &[1], 1.0 / 30.0).is_err());
    assert!(cache.targets(&mut w, &mut h, 1, p).unwrap().is_empty());
    h.fixture["enemies"] = json!([3]);
    cache.capture(&mut w, &mut h, &[1], 1.0 / 30.0).unwrap();
    cache.capture(&mut w, &mut h, &[], 1.0 / 30.0).unwrap();
    assert!(cache.targets(&mut w, &mut h, 1, p).unwrap().is_empty());
}
