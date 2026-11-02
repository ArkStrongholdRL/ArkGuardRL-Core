use arkguard_engine::battle::{
    blocking, buffs,
    damage::{Damage, DamageType, Penetration},
    direction::Direction,
    grid::{Grid, Rect},
    health::{self, Hit, Id, World},
    hit,
    operators::{Definition, DeployOrder, Limits, Placement, Roster},
    settlement,
    stats::{Base, Buff, UnitStats},
};
use arkguard_engine::deterministic::rng::Rng;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};

struct Host {
    roster: Option<Roster>,
    grid: Option<Grid>,
}
impl Host {
    fn call<T>(
        &mut self,
        f: impl FnOnce(&mut Roster, &Grid, &mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut roster = self.roster.take().ok_or("Missing roster context")?;
        let grid = self.grid.take().ok_or("Missing grid context")?;
        let result = f(&mut roster, &grid, self);
        self.roster = Some(roster);
        self.grid = Some(grid);
        result
    }
}
impl health::Hooks for Host {
    fn kill(&mut self, _: Id, _: Option<Id>, _: &mut World) -> Result<(), String> {
        Err("Death requires the active buff context".into())
    }
}
impl buffs::Hooks for Host {
    fn status(&mut self, _: Id, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        _: u64,
        _: buffs::Event,
        _: buffs::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected fixture buff callback".into())
    }
}
impl settlement::Hooks for Host {
    fn health_kill(
        &mut self,
        id: Id,
        credit: Option<Id>,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        self.call(|roster, _, h| roster.kill(w, r, h, id, credit))
    }
    fn health_event(
        &mut self,
        e: &mut health::Event,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if e.stage == health::Stage::SkillDamaged {
            self.call(|roster, _, h| roster.damaged(w, r, h, e.target))?;
        }
        Ok(())
    }
}
fn setup(capacity: usize, max_buffs: usize) -> (World, buffs::Runtime, Host) {
    setup_order(capacity, max_buffs, DeployOrder::Column)
}
fn setup_order(
    capacity: usize,
    max_buffs: usize,
    deploy_order: DeployOrder,
) -> (World, buffs::Runtime, Host) {
    let grid = Grid::new(&json!({"rows":vec!["r".repeat(21);19]}), Rect::NORMAL).unwrap();
    let roster = Roster::new(
        &grid,
        1,
        Limits {
            units: capacity,
            dp_initial: 99.,
            deploy_order,
            ..Default::default()
        },
    )
    .unwrap();
    (
        World::new(8, 1, 1).unwrap(),
        buffs::Runtime::new(max_buffs, 64, 32).unwrap(),
        Host {
            roster: Some(roster),
            grid: Some(grid),
        },
    )
}
fn authored() -> Definition {
    // Independent authored record: no game assets or data bundled in the test.
    Definition::compile("chess_char_1_02_a",&json!({
        "stats":{"maxHp":100,"def":20,"res":5},"skill":{"skillId":"skchr_yak_2",
        "duration":2,"initSp":1,"spCost":1,"spType":"INCREASE_WITH_TIME","trigger":{"rule":"TAKE_DAMAGE"},
        "bb":{"max_hp":0.3,"def":0.1,"magic_resistance":0.6}},
        "talents":[{"index":0,"bb":{"magic_resistance":7}}],"rangeGrid":[[0,0]],
        "position":"MELEE","profession":"TANK","subProfessionId":"protector"
    })).unwrap()
}
fn add(
    w: &mut World,
    r: &mut buffs::Runtime,
    h: &mut Host,
    d: Definition,
    uid: u64,
    tile: [i32; 2],
) -> Result<Id, String> {
    h.call(|roster, g, h| {
        roster.add(
            w,
            r,
            h,
            g,
            d,
            Placement {
                uid,
                home: tile,
                direction: Direction::Right,
            },
        )
    })
}
fn damage(
    w: &mut World,
    r: &mut buffs::Runtime,
    h: &mut Host,
    amount: f64,
    no_sp: bool,
    kind: DamageType,
) -> Result<f64, String> {
    let mut q = hit::Request::new(
        Hit {
            target: 1,
            source: None,
            credit: None,
            damage_type: kind,
            no_sp,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    settlement::resolve(w, r, &mut Rng::new(42), h, &mut q)
}
fn bits(n: f64) -> String {
    format!("{:016x}", n.to_bits())
}
fn view(w: &mut World, h: &Host) -> Value {
    let roster = h.roster.as_ref().unwrap();
    let order: Vec<_> = roster
        .ids()
        .iter()
        .map(|id| {
            let u = w.unit(*id).unwrap();
            json!([u.deployment.uid, u.deployment.deploy_seq])
        })
        .collect();
    let u = w.unit_mut(1).unwrap();
    let s = u.stats.stats().unwrap().clone();
    let roster = h.roster.as_ref().unwrap();
    let k = roster.entry(1).unwrap().skill().unwrap();
    let stats: serde_json::Map<_, _> = ["maxHp", "atk", "def", "res", "blockCnt", "interval"]
        .into_iter()
        .map(|n| (n.into(), json!(bits(s.n(n)))))
        .collect();
    let buffs: Vec<_> = u
        .stats
        .buffs()
        .iter()
        .map(|b| {
            let mods: serde_json::Map<_, _> = b
                .mods
                .iter()
                .map(|(k, v)| (k.clone(), json!(bits(*v))))
                .collect();
            json!({"key":b.key,"mods":mods,"persist":b.runtime.persist})
        })
        .collect();
    json!({"hp":bits(u.stats.hp),"alive":u.stats.alive,"deployed":u.deployed,"removed":u.removed,
        "stats":stats,"order":order,"dp":bits(roster.dp(1).unwrap()),"deploy":u.deployment.deploy_seq,"aggro":u.deployment.aggro_seq,
        "respawn":bits(u.deployment.respawn_at),"death":bits(u.deployment.death_at),"reason":u.deployment.remove_reason,
        "body":u.deployment.body,"ground":u.ground,"element":u.element_buildup.map(bits),"buffs":buffs,
        "skill":{"active":k.bank.active,"sp":bits(k.bank.sp),"charges":bits(k.bank.charges),"left":bits(k.time_left),
        "activations":k.activations,"readyAt":bits(k.op_ready_at)}})
}
#[test]
fn roster_rejections_preserve_units_and_identities() {
    let (mut w, mut r, mut h) = setup(1, 8);
    assert!(add(&mut w, &mut r, &mut h, authored(), 1, [0, 0]).is_err());
    assert_eq!(
        add(&mut w, &mut r, &mut h, authored(), 1, [10, 5]).unwrap(),
        1
    );
    assert!(add(&mut w, &mut r, &mut h, authored(), 2, [10, 6]).is_err());
    assert!(!h.roster.as_ref().unwrap().faulted());
    assert_eq!(w.next_unit_id().unwrap(), 2);
    assert!(Definition::compile("chess_unported", &json!({})).is_err());
}
#[test]
fn death_ends_skill_in_canonical_context_and_redeploy_resets_it() {
    let (mut w, mut r, mut h) = setup(2, 8);
    add(&mut w, &mut r, &mut h, authored(), 1, [10, 5]).unwrap();
    h.call(|roster, g, h| roster.deploy_initial(&mut w, &mut r, h, g))
        .unwrap();
    w.time = 3.;
    damage(&mut w, &mut r, &mut h, 0., true, DamageType::True).unwrap();
    assert!(
        !h.roster
            .as_ref()
            .unwrap()
            .entry(1)
            .unwrap()
            .skill()
            .unwrap()
            .bank
            .active
    );
    damage(&mut w, &mut r, &mut h, 0., false, DamageType::True).unwrap();
    assert!(
        h.roster
            .as_ref()
            .unwrap()
            .entry(1)
            .unwrap()
            .skill()
            .unwrap()
            .bank
            .active
    );
    damage(&mut w, &mut r, &mut h, 10000., false, DamageType::True).unwrap();
    let u = w.unit(1).unwrap();
    assert!(!u.stats.alive);
    assert!(!u.removed);
    assert_eq!(u.stats.buffs().len(), 1);
    assert_eq!(u.stats.buffs()[0].key.as_deref(), Some("talent:yak"));
    assert!(
        !h.roster
            .as_ref()
            .unwrap()
            .entry(1)
            .unwrap()
            .skill()
            .unwrap()
            .bank
            .active
    );
    w.time = 73.;
    h.call(|roster, g, h| roster.check_redeploys(&mut w, &mut r, h, g))
        .unwrap();
    assert!(w.unit(1).unwrap().stats.alive);
    assert_eq!(w.unit(1).unwrap().deployment.deploy_seq, 2);
    assert_eq!(h.roster.as_ref().unwrap().dp(1), Some(89.));
    assert_eq!(
        w.unit_mut(1).unwrap().stats.stats().unwrap().n("maxHp"),
        100.
    );
}
#[test]
fn effect_overflow_faults_roster_and_never_runs_an_incomplete_skill() {
    let (mut w, mut r, mut h) = setup(1, 1);
    add(&mut w, &mut r, &mut h, authored(), 1, [10, 5]).unwrap();
    h.call(|roster, g, h| roster.deploy_initial(&mut w, &mut r, h, g))
        .unwrap();
    w.time = 3.;
    assert!(damage(&mut w, &mut r, &mut h, 0., false, DamageType::True).is_err());
    assert!(h.roster.as_ref().unwrap().faulted());
    assert!(h
        .call(|roster, _, h| roster.tick_skills(&mut w, &mut r, h, 1. / 30.))
        .is_err());
}
#[test]
fn operator_death_releases_blocked_enemies_in_the_same_buff_context() {
    struct Contact;
    impl blocking::Hooks for Contact {
        fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
            Ok(())
        }
        fn ended(&mut self, _: Id, _: &mut World) -> Result<(), String> {
            Err("Unexpected contact release".into())
        }
    }
    let (mut w, mut r, mut h) = setup(1, 8);
    add(&mut w, &mut r, &mut h, authored(), 1, [10, 5]).unwrap();
    h.call(|roster, g, h| roster.deploy_initial(&mut w, &mut r, h, g))
        .unwrap();
    let mut enemy = health::Unit::new(UnitStats::new(Base::default()), health::Side::Enemy);
    enemy.position = [10., 5.2];
    enemy.deployed = true;
    w.insert_unit(2, enemy).unwrap();
    let mut cloak = Buff {
        key: Some("cloak".into()),
        ..Default::default()
    };
    cloak.flags.insert("stealth".into());
    cloak.runtime.persist = true;
    r.add(
        &mut w,
        &mut h,
        2,
        buffs::Spec {
            buff: cloak,
            flags_present: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(blocking::check(
        &mut w,
        h.roster.as_ref().unwrap().deployment(),
        h.grid.as_ref().unwrap(),
        &mut Contact,
        2
    )
    .unwrap());
    damage(&mut w, &mut r, &mut h, 10000., false, DamageType::True).unwrap();
    assert!(w.unit(1).unwrap().blocking().is_empty());
    assert_eq!(w.unit(2).unwrap().blocked_by(), None);
    assert!(w
        .unit(2)
        .unwrap()
        .stats
        .buffs()
        .iter()
        .any(|b| b.key.as_deref() == Some("stealthOff:cloak")));
    assert!(!arkguard_engine::battle::targeting::enemy_stealthed(&mut w, 2).unwrap());
    r.tick(&mut w, &mut h, &[2], 3.).unwrap();
    assert!(arkguard_engine::battle::targeting::enemy_stealthed(&mut w, 2).unwrap());
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; default original operator lifecycle"]
fn default_operators_match_original_deployment_hp_and_skills() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").expect("Set reference root");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/operators.mjs");
    let mut child = Command::new("node")
        .args([script.to_str().unwrap(), &root])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut cases = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let (mut w, mut r, mut h) = setup_order(
            3,
            8,
            if v["orderRule"] == "rowBottomUp" {
                DeployOrder::RowBottomUp
            } else {
                DeployOrder::Column
            },
        );
        add(
            &mut w,
            &mut r,
            &mut h,
            Definition::compile(v["key"].as_str().unwrap(), &v["raw"]).unwrap(),
            1,
            [10, 5],
        )
        .unwrap();
        add(
            &mut w,
            &mut r,
            &mut h,
            Definition::compile(v["key"].as_str().unwrap(), &v["raw"]).unwrap(),
            2,
            [12, 5],
        )
        .unwrap();
        add(
            &mut w,
            &mut r,
            &mut h,
            Definition::compile(v["key"].as_str().unwrap(), &v["raw"]).unwrap(),
            3,
            [9, 4],
        )
        .unwrap();
        h.call(|roster, g, h| roster.deploy_initial(&mut w, &mut r, h, g))
            .unwrap();
        assert_eq!(view(&mut w, &h), v["states"][0], "{} initial", v["key"]);
        for (t, expected) in v["states"].as_array().unwrap().iter().skip(1).enumerate() {
            w.time = t as f64 / 30.;
            for op in v["ops"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|op| op["tick"].as_u64() == Some(t as u64))
            {
                match op["op"].as_str().unwrap() {
                    "damage" => {
                        damage(
                            &mut w,
                            &mut r,
                            &mut h,
                            op["amount"].as_f64().unwrap(),
                            op["noSp"] == true,
                            if op["type"] == "arts" {
                                DamageType::Arts
                            } else {
                                DamageType::True
                            },
                        )
                        .unwrap();
                    }
                    "gain" => {
                        h.call(|roster, _, h| {
                            roster.grant_sp(&mut w, &mut r, h, 1, op["amount"].as_f64().unwrap())
                        })
                        .unwrap();
                    }
                    "retreat" => h
                        .call(|roster, _, h| roster.retreat(&mut w, &mut r, h, 1))
                        .unwrap(),
                    "buff" => {
                        let mut buff = Buff {
                            key: Some("discard-on-death".into()),
                            mods: [("defFlat".into(), 21.)].into(),
                            ..Default::default()
                        };
                        buff.runtime.duration = 100.;
                        r.add(
                            &mut w,
                            &mut h,
                            1,
                            buffs::Spec {
                                buff,
                                mods_present: true,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    }
                    _ => panic!("Unknown oracle operation"),
                }
            }
            r.tick(&mut w, &mut h, &[1], 1. / 30.).unwrap();
            h.call(|roster, _, h| roster.tick_skills(&mut w, &mut r, h, 1. / 30.))
                .unwrap();
            h.call(|roster, g, h| roster.check_redeploys(&mut w, &mut r, h, g))
                .unwrap();
            assert_eq!(
                &view(&mut w, &h),
                expected,
                "{} case {cases} tick {t}",
                v["key"]
            );
        }
        cases += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(cases, 4);
}
