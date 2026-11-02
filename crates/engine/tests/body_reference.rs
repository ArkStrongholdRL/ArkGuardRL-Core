use arkguard_engine::battle::{
    body::{self, HitArea},
    health::{Side, Unit, World},
    spatial::{self, Index},
    stats::{Base, Buff, UnitStats},
    targeting::{Profile, Queries},
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
struct Query {
    skip: bool,
    through: bool,
}
impl spatial::Healing for Query {
    fn heal_through(&mut self, _: u64, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(self.through)
    }
}
impl Queries for Query {
    fn enemy_stealthed(&mut self, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn skip_enemy(&mut self, _: Option<u64>, id: u64, _: &mut World) -> Result<bool, String> {
        Ok(self.skip && id == 3)
    }
    fn remaining_distance(&mut self, _: u64, _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
    fn body_distance(&mut self, id: u64, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        Ok(body::distance(w.unit(id)?, p))
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; collider and spatial queries"]
fn geometry_and_index_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/body.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut bodies = 0;
    let mut indices = 0;
    let mut healing = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if v["healing"] == true {
            let mut w = World::new(9, 1, 1).unwrap();
            let mut ids = vec![];
            for d in v["records"].as_array().unwrap() {
                let id = d["id"].as_u64().unwrap();
                let mut base = Base::default();
                base.0.insert("maxHp".into(), d["maxHp"].as_f64().unwrap());
                let mut u = Unit::new(UnitStats::new(base), Side::Ally);
                u.stats.hp = d["hp"].as_f64().unwrap();
                u.deployment.tile = [
                    d["tile"][0].as_i64().unwrap() as i32,
                    d["tile"][1].as_i64().unwrap() as i32,
                ];
                u.deployment.deploy_seq = d["seq"].as_u64().unwrap();
                if d["device"] == true {
                    u.deployment.kind = arkguard_engine::battle::deploy::Kind::Device;
                }
                u.stats.alive = d["alive"] == true;
                u.deployed = d["deployed"] == true;
                u.hidden = d["hidden"] == true;
                u.profile_no_heal = d["profileNoHeal"] == true;
                u.element_buildup[0] = d["element"].as_f64().unwrap();
                u.stats.set_buffs(vec![Buff {
                    flags: if d["noHeal"] == true {
                        ["noHeal".into()].into_iter().collect()
                    } else {
                        Default::default()
                    },
                    ..Buff::default()
                }]);
                w.insert_unit(id, u).unwrap();
                ids.push(id);
            }
            let mut q = Query {
                skip: false,
                through: v["through"] == true,
            };
            assert_eq!(
                json!(spatial::injured_allies(
                    &mut w,
                    &mut q,
                    &ids,
                    &[216, 218],
                    v["healer"].as_u64(),
                    v["includeElement"] == true,
                    9
                )
                .unwrap()),
                v["targets"],
                "healing {}",
                v["i"]
            );
            let overflow = spatial::injured_allies(
                &mut w,
                &mut q,
                &ids,
                &[216, 218],
                v["healer"].as_u64(),
                v["includeElement"] == true,
                0,
            );
            assert_eq!(
                overflow.is_err(),
                !v["targets"].as_array().unwrap().is_empty()
            );
            healing += 1;
            continue;
        }
        if v["body"] == true {
            let mut u = Unit::new(
                UnitStats::new(Base::default()),
                if v["side"] == "ally" {
                    Side::Ally
                } else {
                    Side::Enemy
                },
            );
            u.position = [v["y"].as_f64().unwrap(), v["x"].as_f64().unwrap()];
            let a = &v["area"];
            if !a.is_null() {
                u.hit_area = HitArea::new(
                    a["w"].as_f64().unwrap(),
                    a["h"].as_f64().unwrap(),
                    a["dx"].as_f64().unwrap(),
                    a["dy"].as_f64().unwrap(),
                );
            }
            let keys = [214, 215, 235, 236, 237];
            let actual = json!({"keys":body::keys(&u),"collider":body::collider_keys(&u),"on":body::on_tile(&u,10,4),"inKeys":body::in_keys(&u,&keys),"inRange":body::in_range(&u,&keys),"distance":bits(body::distance(&u,[10.0,4.0])),"radius":body::in_radius(&u,[10.0,4.0],2.0),"reach":bits(body::tile_reach(&u,10,4))});
            for (k, a) in actual.as_object().unwrap() {
                assert_eq!(*a, v[k], "{k} body {}", v["i"]);
            }
            bodies += 1;
            continue;
        }
        let mut w = World::new(7, 1, 1).unwrap();
        w.insert_unit(1, Unit::new(UnitStats::new(Base::default()), Side::Ally))
            .unwrap();
        let mut ids = vec![];
        for d in v["records"].as_array().unwrap() {
            let id = d["id"].as_u64().unwrap();
            let mut u = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
            u.position = [d["y"].as_f64().unwrap(), d["x"].as_f64().unwrap()];
            u.stats.alive = d["alive"] == true;
            u.deployed = d["deployed"] == true;
            u.hidden = d["hidden"] == true;
            u.motion_fly = d["fly"] == true;
            if d["huge"] == true {
                u.hit_area = HitArea::new(4.95, 2.95, 0.0, 1.0);
            }
            u.stats.set_buffs(vec![Buff {
                flags: ["sleep", "untargetable"]
                    .into_iter()
                    .filter(|k| d[*k] == true)
                    .map(str::to_string)
                    .collect(),
                ..Buff::default()
            }]);
            w.insert_unit(id, u).unwrap();
            ids.push(id);
        }
        let mut index = Index::new(7, 4000).unwrap();
        index.rebuild(&w, &ids).unwrap();
        let mut q = Query {
            skip: v["skip"] == true,
            through: false,
        };
        let p = Profile {
            can_hit_fly: v["fly"] == true,
            hit_sleep: v["hitSleep"] == true,
            ..Profile::default()
        };
        let keys: Vec<_> = v["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(
            json!(index
                .enemies(
                    &mut w,
                    &mut q,
                    &keys,
                    Some(1),
                    p,
                    v["rangeTiles"] == true,
                    64
                )
                .unwrap()),
            v["enemies"],
            "index {}",
            v["i"]
        );
        assert_eq!(
            json!(index
                .enemies(&mut w, &mut q, &keys, Some(1), p, true, 64)
                .unwrap()),
            v["tiles"]
        );
        assert_eq!(
            index.any_enemy(&w, &keys, v["rangeTiles"] == true).unwrap(),
            v["any"] == true
        );
        indices += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(bodies, 1024);
    assert_eq!(indices, 128);
    assert_eq!(healing, 256);
}
#[test]
fn invalid_or_overflowed_rebuild_cannot_reuse_previous_index() {
    let mut w = World::new(1, 1, 1).unwrap();
    let mut e = Unit::new(UnitStats::new(Base::default()), Side::Enemy);
    e.position = [10.0, 4.0];
    e.deployed = true;
    w.insert_unit(1, e).unwrap();
    let mut idx = Index::new(1, 2).unwrap();
    idx.rebuild(&w, &[1]).unwrap();
    assert!(idx.any_enemy(&w, &[214], false).unwrap());
    assert!(idx.rebuild(&w, &[1, 1]).is_err());
    assert!(idx.any_enemy(&w, &[214], false).is_err());
    w.unit_mut(1).unwrap().hit_area = HitArea::new(4.95, 2.95, 0.0, 1.0);
    assert!(idx.rebuild(&w, &[1]).is_err());
    assert!(idx.any_enemy(&w, &[214], false).is_err());
}
