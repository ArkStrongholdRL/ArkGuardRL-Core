use arkguard_engine::{
    battle::{
        blocking, cadence,
        deploy::{self, Deployment, Options},
        direction::Direction,
        health::{Id, Side, Unit, World},
        stats::{Base, Buff, UnitStats},
        targeting::{self, Priority, Profile, Queries},
    },
    deterministic::math::hypot,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
struct Context {
    records: Vec<Value>,
    skip: bool,
}
impl Queries for Context {
    fn enemy_stealthed(&mut self, id: Id, w: &mut World) -> Result<bool, String> {
        let e = w.unit_mut(id)?;
        let f = &e.stats.stats().map_err(str::to_string)?.flags;
        Ok(f.contains("stealth") && !f.contains("reveal") && e.blocked_by().is_none())
    }
    fn skip_enemy(&mut self, _: Option<Id>, id: Id, _: &mut World) -> Result<bool, String> {
        Ok(self.skip && id == 6)
    }
    fn remaining_distance(&mut self, id: Id, _: &mut World) -> Result<f64, String> {
        Ok(self.records[id as usize - 1]["remaining"]
            .as_f64()
            .unwrap_or(f64::INFINITY))
    }
    fn body_distance(&mut self, id: Id, origin: [f64; 2], w: &mut World) -> Result<f64, String> {
        let p = w.unit(id)?.position;
        Ok(hypot(p[1] - origin[1], p[0] - origin[0]))
    }
}
struct Host;
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
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
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; targeting and cadence only"]
fn filters_priorities_and_commit_gate_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/targeting.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut targets = 0;
    let mut cadence_cases = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if v["cadence"] == true {
            let p = &v["profile"];
            let state = cadence::State {
                cooldown: v["cd"].as_f64().unwrap(),
                had_target: v["had"].as_bool(),
            };
            let profile = cadence::Profile {
                no_attack: p["noAttack"] == true,
                only_while_skill: p["noAttackUnlessSkill"] == true,
                heal_attack: p["dmgType"] == "heal",
                all_in_range: p["allInRange"] == true,
                range_aoe: p["rangeAoe"] == true,
            };
            assert_eq!(
                state.may_commit(
                    1.0 / 30.0,
                    v["act"] == true,
                    v["disarm"] == true,
                    v["active"] == true,
                    profile
                ),
                v["commit"] == true,
                "{v}"
            );
            assert_eq!(
                cadence::countdown(state.cooldown, 1.0 / 30.0).to_bits(),
                v["countdown"].as_f64().unwrap().to_bits()
            );
            cadence_cases += 1;
            continue;
        }
        let records = v["records"].as_array().unwrap().clone();
        let mut w = World::new(6, 1, 1).unwrap();
        let mut dep =
            Deployment::new(arkguard_engine::battle::grid::Rect::FULL, 6, 1, 99.0).unwrap();
        for d in &records {
            let id = d["id"].as_u64().unwrap();
            let mut base = Base::default();
            base.0.extend(
                d["base"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_f64().unwrap())),
            );
            if id == 1 {
                base.0.insert("blockCnt".into(), 10.0);
            }
            let mut u = Unit::new(
                UnitStats::new(base),
                if id == 1 { Side::Ally } else { Side::Enemy },
            );
            u.position = [10.0, 4.0];
            u.deployment.home = [10, 4];
            u.deployment.kind = deploy::Kind::Operator;
            u.ground = true;
            u.deployed = true;
            w.insert_unit(id, u).unwrap();
        }
        dep.register(1, &w).unwrap();
        w.unit_mut(1).unwrap().deployed = false;
        dep.deploy(&mut w, &mut Host, 1, Options::default())
            .unwrap();
        let grid = arkguard_engine::battle::grid::Grid::new(
            &json!({"rows":vec!["r".repeat(21);19]}),
            arkguard_engine::battle::grid::Rect::FULL,
        )
        .unwrap();
        assert!(blocking::check(&mut w, &dep, &grid, &mut Host, 2).unwrap());
        for d in &records {
            let id = d["id"].as_u64().unwrap();
            let u = w.unit_mut(id).unwrap();
            u.position = [d["y"].as_f64().unwrap(), d["x"].as_f64().unwrap()];
            u.stats.hp = d["hp"].as_f64().unwrap();
            u.stats.alive = d["alive"] == true;
            u.deployed = d["deployed"] == true;
            u.hidden = d["hidden"] == true;
            u.motion_fly = d["motion"] == "FLY";
            u.targeting.spawn_seq = d["spawn"].as_u64().unwrap();
            u.targeting.elite = d["elite"] == true;
            u.targeting.ranged_apply_melee = d["apply"] == "MELEE";
            u.boss = d["boss"] == true;
            u.deployment.deploy_seq = id;
            u.deployment.aggro_seq = d["aggro"].as_u64().unwrap();
            u.stats.set_buffs(vec![Buff {
                flags: d["flags"].as_object().unwrap().keys().cloned().collect(),
                ..Buff::default()
            }]);
        }
        w.unit_mut(1).unwrap().targeting.doll = v["doll"] == true;
        let p = &v["profile"];
        let profile = Profile {
            hit_sleep: p["hitSleep"] == true,
            can_hit_fly: p["canHitFly"] == true,
            doll_hits_fly: p["dollHitsFly"] == true,
            ground_only: p["groundOnly"] == true,
            block_free: p["blockFree"] == true,
        };
        let mut ctx = Context {
            records,
            skip: p["skip"] == true,
        };
        let priorities = [
            Priority::None,
            Priority::Fly,
            Priority::LowDef,
            Priority::HighDef,
            Priority::Ranged,
            Priority::LowestHp,
            Priority::HighestHp,
            Priority::LowestHpRatio,
            Priority::HighestAtk,
            Priority::Boss,
            Priority::Elite,
            Priority::NotBurst,
            Priority::Ground,
            Priority::Heaviest,
            Priority::Nearest,
            Priority::Farthest,
        ];
        for (i, priority) in priorities.into_iter().enumerate() {
            let mut c = [2, 3, 4, 5, 6];
            targeting::sort_enemies(&mut w, &mut ctx, 1, &mut c, priority).unwrap();
            assert_eq!(
                json!(c),
                v["enemyOrder"][i],
                "sort {priority:?} case {}",
                v["i"]
            );
        }
        let mut allies = [5, 1, 4, 3];
        targeting::sort_allies(&mut w, 2, &mut allies, profile).unwrap();
        assert_eq!(json!(allies), v["allyOrder"]);
        for expected in v["filters"].as_array().unwrap() {
            let id = expected["id"].as_u64().unwrap();
            let actual = json!({"id":id,"enemy":targeting::can_target_enemy(&mut w,&mut ctx,Some(1),id,profile).unwrap(),"ally":targeting::can_target_ally(&mut w,2,id,true,profile).unwrap(),"aura":targeting::aura_selectable(&mut w,id).unwrap(),"area":targeting::area_selectable(&mut w,Some(2),id).unwrap()});
            assert_eq!(actual, *expected, "filter case {}", v["i"]);
        }
        for (i, dir) in Direction::ALL.into_iter().enumerate() {
            assert_eq!(
                json!(targeting::absolute_range(
                    &[[0, 0], [1, 1], [0, 1], [1, -1], [0, 0]],
                    [10, 4],
                    dir,
                    (v["i"].as_u64().unwrap() % 24) as u32
                )),
                v["ranges"][i]
            );
        }
        targets += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(targets, 512);
    assert_eq!(cadence_cases, 1024);
}
