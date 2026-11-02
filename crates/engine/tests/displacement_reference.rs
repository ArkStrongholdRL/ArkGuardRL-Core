use arkguard_engine::{
    battle::{
        blocking,
        deploy::{self, Deployment, Options},
        direction::Direction,
        displacement::{self, Origin, Pull, Push},
        grid::{Grid, Rect},
        health::{Id, Side, Unit, World},
        motion::Route,
        stats::{Base, Buff, UnitStats},
    },
    data::Catalog,
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
fn pair(v: &Value) -> [f64; 2] {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}
#[derive(Default)]
struct Host {
    ended: u64,
}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        self.ended += 1;
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
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; push/pull forces, collisions and unbalance"]
fn displacement_physics_matches() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let data = Catalog::load(&Path::new(&root).join("data"), 64 << 20).unwrap();
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/displacement.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let c = &v["cfg"];
        let mut stage = data.table("stages").unwrap()[c["stageId"].as_str().unwrap()].clone();
        stage["legend"] = stage["tiles"].clone();
        let rect = Rect::NORMAL;
        let grid = Grid::new(&stage, rect).unwrap();
        let mut w = World::new(2, 1, 1).unwrap();
        let mut h = Host::default();
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 1000.);
        base.0
            .insert("massLevel".into(), c["mass"].as_f64().unwrap());
        let mut e = Unit::new(
            UnitStats::new(base),
            if c["ally"] == true {
                Side::Ally
            } else {
                Side::Enemy
            },
        );
        e.position = pair(&c["position"]);
        e.stats.alive = c["dead"] != true;
        e.deployed = true;
        e.boss = c["boss"] == true;
        e.static_body = c["static"] == true;
        e.motion_fly = c["fly"] == true;
        e.motion.unbalance_until = c["hold"].as_f64().unwrap_or(f64::NEG_INFINITY);
        e.motion.attack_stand_until = 3.;
        let mut route = Route::compile(&json!({"end":[10,8]}), rect, 8, e.position).unwrap();
        route.points = Some(vec![[10., 5.]]);
        e.motion.route = Some(route);
        if c["noDisplace"] == true {
            let mut b = Buff::default();
            b.flags.insert("noDisplace".into());
            e.stats.set_buffs(vec![b]);
        }
        w.insert_unit(1, e).unwrap();
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 1000.);
        base.0.insert("blockCnt".into(), 3.);
        let mut a = Unit::new(UnitStats::new(base), Side::Ally);
        a.deployment.kind = deploy::Kind::Operator;
        a.deployment.home = [10, 5];
        a.stats.alive = false;
        a.ground = true;
        w.insert_unit(2, a).unwrap();
        let mut d = Deployment::new(rect, 2, 1, 99.).unwrap();
        d.register(2, &w).unwrap();
        d.deploy(&mut w, &mut h, 2, Options::default()).unwrap();
        blocking::check(&mut w, &d, &grid, &mut h, 1).unwrap();
        let direction = match c["dir"].as_str().unwrap() {
            "RIGHT" => Direction::Right,
            "UP" => Direction::Up,
            "LEFT" => Direction::Left,
            _ => Direction::Down,
        };
        for (j, op) in v["ops"].as_array().unwrap().iter().enumerate() {
            w.time = j as f64 * 0.07;
            w.unit_mut(1).unwrap().motion.attack_stand_until = w.time + 0.4;
            let force = op["force"].as_f64().unwrap();
            let dir = [
                op["dir"]["y"].as_f64().unwrap(),
                op["dir"]["x"].as_f64().unwrap(),
            ];
            let from = Origin {
                position: pair(&op["from"]["position"]),
                forward: op["from"]["forward"]
                    .as_array()
                    .map(|_| pair(&op["from"]["forward"])),
                unit: Some(2),
            };
            let push = Push {
                from: Some(from),
                fixed: op["fixed"] == true,
                fixed_angle: op["fixedAngle"] == true,
                effect: op["effect"] == true,
                ..Push::default()
            };
            let moved = match op["op"].as_u64().unwrap() {
                0 => displacement::push(
                    &mut w,
                    &grid,
                    &mut h,
                    1,
                    force,
                    Push {
                        inward: op["inward"] == true,
                        ..push
                    },
                ),
                1 => displacement::push(
                    &mut w,
                    &grid,
                    &mut h,
                    1,
                    force,
                    Push {
                        direction: Some(dir),
                        ..push
                    },
                ),
                2 => {
                    let center = Origin::unit(&w, 2, direction).unwrap();
                    displacement::pull(
                        &mut w,
                        &grid,
                        &mut h,
                        1,
                        force,
                        Pull {
                            to: pair(&op["to"]),
                            center: Some(center),
                            stop: op["stop"].as_f64().unwrap(),
                        },
                    )
                }
                3 => displacement::pull_to_front(&mut w, &grid, &mut h, 1, 2, direction, force),
                4 => displacement::displace(
                    &mut w,
                    &grid,
                    &mut h,
                    1,
                    dir,
                    op["distance"].as_f64().unwrap(),
                ),
                5 => displacement::push_distance(&mut w, 1, force, op["effect"] == true),
                6 => {
                    displacement::unbalance(&mut w, 1, op["distance"].as_f64().unwrap()).map(|_| 0.)
                }
                _ => unreachable!(),
            }
            .unwrap();
            let level = displacement::force_level(&mut w, 1, force).unwrap();
            let e = w.unit(1).unwrap();
            let actual = json!({"moved":bits(moved),"position":e.position.map(bits),"until":bits(e.motion.unbalance_until),"stand":bits(e.motion.attack_stand_until),"blocked":e.blocked_by(),"blocking":w.unit(2).unwrap().blocking(),"cached":e.motion.route.as_ref().unwrap().points.is_some(),"ended":h.ended,"level":bits(level),"tables":([displacement::push_tiles(force,op["effect"]==true),displacement::push_hold(force),displacement::pull_hold(force)].map(bits))});
            assert_eq!(actual, v["states"][j], "case {cases} op {j}: {op}");
        }
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 320);
}
