use arkguard_engine::{
    battle::{
        attacks::{self, Area, Chain, Splash, VictimKind},
        body::HitArea,
        damage::DamageType,
        health::{self, Side, Unit, World},
        hit,
        injury::Effects,
        search::AttackProfile,
        skills::{self, Runtime},
        spatial::{self, RadiusRule},
        stats::{Base, Buff, UnitStats},
        targeting::{Profile, Queries},
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
    cfg: Value,
    trace: Vec<Value>,
    enemies: Vec<u64>,
    effects: Option<Effects>,
    rng: Rng,
}
impl Queries for Host {
    fn enemy_stealthed(&mut self, id: u64, w: &mut World) -> Result<bool, String> {
        let u = w.unit_mut(id)?;
        let f = &u.stats.stats().map_err(str::to_string)?.flags;
        Ok(f.contains("stealth") && !f.contains("reveal"))
    }
    fn skip_enemy(&mut self, _: Option<u64>, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, _: u64, _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
    fn body_distance(&mut self, _: u64, _: [f64; 2], _: &mut World) -> Result<f64, String> {
        unreachable!()
    }
}
impl health::Hooks for Host {
    fn kill(&mut self, id: u64, _: Option<u64>, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["kill", id]));
        Ok(())
    }
}
impl hit::Hooks for Host {}
impl skills::Hooks for Host {
    fn event(
        &mut self,
        _: skills::Event,
        _: &mut skills::Context,
        _: &mut Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        unreachable!()
    }
    fn condition(
        &mut self,
        _: skills::Condition,
        _: &Runtime,
        _: &mut World,
    ) -> Result<bool, String> {
        unreachable!()
    }
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Option<AttackProfile>, String> {
        unreachable!()
    }
    fn has_sp_hook(&self) -> bool {
        false
    }
}
impl attacks::Hooks for Host {
    fn has_before_attack(&self) -> bool {
        false
    }
    fn before_attack(&mut self, _: &mut attacks::Context, _: &mut World) -> Result<(), String> {
        unreachable!()
    }
    fn engage(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn damage(&mut self, w: &mut World, r: &mut hit::Request) -> Result<f64, String> {
        self.trace.push(json!([
            "hit",
            r.hit.target,
            bits(r.damage.amount),
            bits(r.damage.mul),
            r.hit.no_sp,
            r.is_splash,
            r.tags,
            r.attack_id.unwrap()
        ]));
        let mut effects = self.effects.take().unwrap();
        let mut rng = self.rng.clone();
        let result = hit::resolve(w, &mut effects, &mut rng, self, r);
        self.effects = Some(effects);
        self.rng = rng;
        result
    }
    fn radius_targets(
        &mut self,
        origin: [f64; 2],
        radius: f64,
        splash: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let enemies = self.enemies.clone();
        spatial::enemies_in_radius(
            w,
            self,
            &enemies,
            origin,
            radius,
            if splash {
                RadiusRule::Centre
            } else {
                RadiusRule::Body
            },
            splash,
            limit,
        )
    }
    fn chain_link(&mut self, from: u64, to: u64, _: &mut World) -> Result<(), String> {
        self.trace.push(json!(["link", from, to]));
        Ok(())
    }
    fn sluggish(
        &mut self,
        _: u64,
        target: u64,
        duration: f64,
        _: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!(["slow", target, bits(duration)]));
        Ok(())
    }
    fn each_hit(
        &mut self,
        c: attacks::Impact,
        _: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        let kind = match c.kind {
            VictimKind::Main => "main",
            VictimKind::Splash => "splash",
            VictimKind::Chain => "chain",
        };
        self.trace.push(json!([
            "each",
            c.target.unwrap(),
            kind,
            bits(c.amount),
            c.main,
            c.attack_id,
            c.is_skill
        ]));
        if c.kind == VictimKind::Main {
            if self.cfg["move"] == true {
                w.unit_mut(c.target.unwrap())?.position[1] += 0.6;
            }
            if self.cfg["changeAtk"] == true {
                w.unit_mut(1)?.stats.edit_base(|base| {
                    base.0.insert("atk".into(), 999.);
                });
            }
        }
        Ok(())
    }
    fn after_hit(
        &mut self,
        c: attacks::Impact,
        _: Option<&mut Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        self.trace.push(json!([
            "after",
            c.target,
            bits(c.amount),
            bits(c.position[1]),
            bits(c.position[0]),
            c.is_skill
        ]));
        Ok(())
    }
    fn attack(
        &mut self,
        _: &mut attacks::Context,
        _: Option<&mut Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
    fn after_attack(
        &mut self,
        _: &attacks::Context,
        _: Option<&mut Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; splash/chain settlement"]
fn area_payload_matches() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/area.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let c = &v["cfg"];
        let mut w = World::new(13, 1, 1).unwrap();
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 1000.);
        base.0.insert("atk".into(), 60.25);
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let mut enemies = vec![];
        for d in v["records"].as_array().unwrap() {
            let id = d["id"].as_u64().unwrap();
            let mut base = Base::default();
            for (k, x) in [("maxHp", 1000.), ("def", 20.), ("res", 30.)] {
                base.0.insert(k.into(), x);
            }
            let mut u = Unit::new(UnitStats::new(base), Side::Enemy);
            u.stats.hp = d["hp"].as_f64().unwrap();
            u.stats.alive = d["alive"] == true;
            u.deployed = d["deployed"] == true;
            u.hidden = d["hidden"] == true;
            u.motion_fly = d["fly"] == true;
            u.position = [d["y"].as_f64().unwrap(), d["x"].as_f64().unwrap()];
            u.targeting.spawn_seq = d["seq"].as_u64().unwrap();
            if d["huge"] == true {
                u.hit_area = HitArea::new(4.95, 2.95, 0., 1.);
            }
            u.stats.set_buffs(vec![Buff {
                flags: ["sleep", "stealth", "reveal", "untargetable"]
                    .into_iter()
                    .filter(|k| d[*k] == true)
                    .map(str::to_string)
                    .collect(),
                ..Buff::default()
            }]);
            w.insert_unit(id, u).unwrap();
            enemies.push(id);
        }
        let mut h = Host {
            cfg: c.clone(),
            trace: vec![],
            enemies,
            effects: Some(Effects::new(32).unwrap()),
            rng: Rng::new(42),
        };
        let p = attacks::Direct {
            mode: attacks::Mode::Damage(match c["type"].as_str().unwrap() {
                "arts" => DamageType::Arts,
                "true" => DamageType::True,
                "elemental" => DamageType::Elemental,
                _ => DamageType::Physical,
            }),
            hits: c["hits"].as_u64().unwrap() as u32,
            atk_scale: 1.25,
            skill_damage_mul: 1.15,
            target_damage_mul: c["mul"].as_f64().unwrap(),
            split_mul: c["split"].as_f64(),
            heal_scale: 1.,
            is_skill: c["skill"] == true,
        };
        let area = Area {
            selection: Profile {
                can_hit_fly: c["canHitFly"] == true,
                ground_only: c["groundOnly"] == true,
                ..Profile::default()
            },
            fortress_melee: c["fortressMelee"] == true,
            splash: if c["splash"] == true {
                Some(Splash {
                    radius: 1.5,
                    scale: v["splashScale"].as_f64().unwrap(),
                    hits_fly: c["splashFly"] == true,
                    fortress_melee: c["fortressSplash"] == true,
                })
            } else {
                None
            },
            chain: if c["chain"] == true {
                Some(Chain {
                    count: c["count"].as_f64().unwrap(),
                    radius: c["radius"].as_f64().unwrap(),
                    falloff: c["falloff"].as_f64().unwrap(),
                    sluggish: c["sluggish"].as_f64().unwrap(),
                })
            } else {
                None
            },
        };
        attacks::Executor::new(16, 8)
            .unwrap()
            .impact_at(
                &mut w,
                &mut h,
                1,
                c["main"].as_u64(),
                17,
                p,
                area,
                [10., 6.],
                None,
            )
            .unwrap();
        assert_eq!(json!(h.trace), v["trace"], "trace case {c}");
        assert_eq!(
            json!(h
                .enemies
                .iter()
                .map(|id| bits(w.unit(*id).unwrap().stats.hp))
                .collect::<Vec<_>>()),
            v["hp"],
            "hp case {c}"
        );
        assert_eq!(
            json!(h
                .enemies
                .iter()
                .map(|id| w.unit(*id).unwrap().stats.alive)
                .collect::<Vec<_>>()),
            v["alive"]
        );
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 512);
}

#[test]
fn area_capacity_errors_do_not_clip_or_emit_after_hit() {
    let mut w = World::new(3, 1, 1).unwrap();
    for (id, side) in [(1, Side::Ally), (2, Side::Enemy), (3, Side::Enemy)] {
        let mut u = Unit::new(UnitStats::new(Base::default()), side);
        u.deployed = true;
        w.insert_unit(id, u).unwrap();
    }
    let mut h = Host {
        cfg: Value::Null,
        trace: vec![],
        enemies: vec![2, 3],
        effects: Some(Effects::new(8).unwrap()),
        rng: Rng::new(42),
    };
    let p = attacks::Direct {
        mode: attacks::Mode::Damage(DamageType::Physical),
        hits: 1,
        atk_scale: 1.0,
        skill_damage_mul: 1.0,
        target_damage_mul: 1.0,
        split_mul: None,
        heal_scale: 1.0,
        is_skill: false,
    };
    let mut executor = attacks::Executor::new(2, 1).unwrap();
    for count in [0.0, 3.0, 2.1, f64::NAN, f64::INFINITY] {
        let area = Area {
            chain: Some(Chain {
                count,
                radius: 1.7,
                falloff: 0.15,
                sluggish: 0.0,
            }),
            ..Default::default()
        };
        assert!(executor
            .impact_at(&mut w, &mut h, 1, None, 1, p, area, [0.0, 0.0], None)
            .is_err());
        assert!(h.trace.is_empty());
    }
    let area = Area {
        splash: Some(Splash {
            radius: 1.0,
            scale: 1.0,
            hits_fly: false,
            fortress_melee: false,
        }),
        ..Default::default()
    };
    assert!(attacks::Executor::new(1, 1)
        .unwrap()
        .impact_at(&mut w, &mut h, 1, None, 1, p, area, [0.0, 0.0], None)
        .is_err());
    assert!(h.trace.is_empty());
    assert_eq!(w.unit(2).unwrap().stats.hp, 1.0);
    assert_eq!(w.unit(3).unwrap().stats.hp, 1.0);
}
