//! Continuous integration: real native search + skill + damage/HP paths against
//! upstream updateAlly/performAttack/resolveDamage. Real spatial queries, scripted movement.
use arkguard_engine::{
    battle::{
        ally, attacks, blocking, body, buffs,
        damage::DamageType,
        element::{self, Element},
        grid::{Grid, Rect},
        health::{self, Side, Unit, World},
        hit, motion,
        projectiles::{self, Launch, Projectile, System},
        search::{self, AttackProfile, Commitments, Heal, Selection},
        settlement,
        skills::{self, Condition, Context, Event, Rule, Runtime, SpType, Spec},
        sp::{Bank, Kind},
        spatial::{self, Index},
        stats::{Base, Buff, UnitStats},
        status,
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
    d: Value,
    trace: Vec<Value>,
    index: Option<Index>,
    enemies: Vec<u64>,
    grid: Option<Grid>,
    stored: u32,
    executor: Option<attacks::Executor>,
    flights: Option<System<(u64, u64, attacks::Direct, attacks::Area)>>,
    buffs: Option<buffs::Runtime>,
    rng: Rng,
}
impl Host {
    fn move_target(&mut self, w: &mut World, inside: bool) -> Result<(), String> {
        let e = w.unit_mut(2)?;
        if self.d["mobile"] != true {
            e.position = [10.0, if inside { 6.0 } else { 8.0 }];
            e.deployment.tile = [10, if inside { 6 } else { 8 }];
        }
        self.index.as_mut().unwrap().rebuild(w, &self.enemies)
    }
    fn base_profile(&self) -> AttackProfile {
        AttackProfile {
            heal: if self.d["healer"] == true {
                Some(Heal {
                    multi: false,
                    count: 1.0,
                    hp_at_most: 0.0,
                    element_heal: false,
                })
            } else {
                None
            },
            heal_attack: self.d["healer"] == true,
            ..AttackProfile::default()
        }
    }
}
impl Queries for Host {
    fn enemy_stealthed(&mut self, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn skip_enemy(&mut self, _: Option<u64>, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn remaining_distance(&mut self, _: u64, _: &mut World) -> Result<f64, String> {
        Ok(5.0)
    }
    fn body_distance(&mut self, id: u64, p: [f64; 2], w: &mut World) -> Result<f64, String> {
        Ok(body::distance(w.unit(id)?, p))
    }
}
impl spatial::Healing for Host {
    fn heal_through(&mut self, _: u64, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
}
impl search::Host for Host {
    fn enemies(
        &mut self,
        id: u64,
        keys: &[usize],
        p: Profile,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let index = self.index.take().unwrap();
        let result = index.enemies(w, self, keys, Some(id), p, false, limit);
        self.index = Some(index);
        result
    }
    fn injured_allies(
        &mut self,
        id: u64,
        keys: &[usize],
        include_element: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<u64>, String> {
        let allies: &[u64] = if self.d["healer"] == true {
            &[1, 2]
        } else {
            &[1]
        };
        spatial::injured_allies(w, self, allies, keys, Some(id), include_element, limit)
    }
    fn has_ally_targets(&self) -> bool {
        false
    }
    fn ally_targets(
        &mut self,
        _: u64,
        _: &[usize],
        _: &mut World,
        _: usize,
    ) -> Result<Vec<u64>, String> {
        Ok(vec![])
    }
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(self.base_profile()))
    }
    fn range(&mut self, _: u64, _: &World) -> Result<Vec<usize>, String> {
        Ok(vec![216])
    }
    fn skill_active(&self, _: u64, _: &World) -> Result<bool, String> {
        Ok(false)
    }
    fn can_attack(&mut self, _: u64, _: AttackProfile, _: &mut World) -> Result<bool, String> {
        Ok(true)
    }
}
impl blocking::Hooks for Host {
    fn blocked(&mut self, _: u64, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
}
impl motion::Hooks for Host {
    fn check_block(&mut self, _: u64, _: &mut World, _: &mut Grid) -> Result<bool, String> {
        Ok(false)
    }
    fn hidden(&mut self, id: u64, on: bool, w: &mut World) -> Result<(), String> {
        w.unit_mut(id)?.hidden = on;
        Ok(())
    }
    fn cut_attack_stand(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn leak(&mut self, id: u64, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["leak"]));
        Ok(())
    }
}
impl health::Hooks for Host {
    fn kill(&mut self, id: u64, _: Option<u64>, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.stats.hp = 0.0;
        u.stats.alive = false;
        u.deployed = false;
        self.trace.push(json!(["kill"]));
        Ok(())
    }
}
impl buffs::Hooks for Host {
    fn burst_damage(
        &mut self,
        r: &mut hit::Request,
        runtime: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<f64, String> {
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, runtime, &mut rng, self, r);
        self.rng = rng;
        result
    }
    fn status(&mut self, _: u64, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn callback(
        &mut self,
        token: u64,
        _: buffs::Event,
        c: buffs::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if token != 1 {
            return Err("Unknown fixture buff callback".into());
        }
        self.trace.push(json!(["pulse"]));
        r.edit(w, c.unit, c.buff, |b| b.stacks += 1)
    }
}
impl status::Hooks for Host {
    fn has_before_status(&self) -> bool {
        false
    }
    fn before_status(
        &mut self,
        _: &mut status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected status hook".into())
    }
    fn has_status_applied(&self) -> bool {
        false
    }
    fn status_applied(
        &mut self,
        _: &status::Context,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected status hook".into())
    }
}
impl element::Hooks for Host {
    fn battle_rect(&self) -> Rect {
        Rect::FULL
    }
    fn element_enabled(&self, _: element::Phase) -> bool {
        false
    }
    fn element_event(
        &mut self,
        _: element::Phase,
        _: &mut element::Request,
        _: f64,
        _: f64,
        _: &mut buffs::Runtime,
        _: &mut World,
    ) -> Result<(), String> {
        Err("Unexpected element hook".into())
    }
    fn element_number(&mut self, _: u64, _: f64, _: Element) -> Result<(), String> {
        Ok(())
    }
    fn burst_visual(&mut self, _: u64, _: Element, _: &World) -> Result<(), String> {
        Ok(())
    }
}
impl settlement::Hooks for Host {}
impl skills::Hooks for Host {
    fn event(
        &mut self,
        event: Event,
        _: &mut Context,
        _: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        match event {
            Event::ApplyModifiers => {
                let mut r = self.buffs.take().unwrap();
                let result = r.add(
                    w,
                    self,
                    1,
                    buffs::Spec {
                        buff: Buff {
                            key: Some("skill".into()),
                            mods: [("atkPct".into(), 0.5)].into_iter().collect(),
                            ..Default::default()
                        },
                        mods_present: true,
                        ..Default::default()
                    },
                );
                self.buffs = Some(r);
                result?;
            }
            Event::RemoveModifiers => {
                let mut r = self.buffs.take().unwrap();
                let result = r.remove(w, self, 1, "skill");
                self.buffs = Some(r);
                result?;
            }
            Event::Start => self.trace.push(json!(["start"])),
            Event::End => self.trace.push(json!(["end"])),
            Event::SkillStart => self.trace.push(json!(["skillStart"])),
            Event::SkillEnd => self.trace.push(json!(["skillEnd"])),
            Event::AmmoUsed => self.trace.push(json!(["ammoUsed"])),
            _ => {}
        }
        Ok(())
    }
    fn condition(&mut self, _: Condition, _: &Runtime, w: &mut World) -> Result<bool, String> {
        if self.d["healer"] == true {
            Ok(!search::Host::injured_allies(self, 1, &[216], false, w, 8)?.is_empty())
        } else {
            Ok(!search::Host::enemies(self, 1, &[216], Profile::default(), w, 8)?.is_empty())
        }
    }
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(self.base_profile()))
    }
    fn has_sp_hook(&self) -> bool {
        false
    }
}
impl ally::Host for Host {
    fn effective_profile(
        &mut self,
        _: u64,
        _: Option<&Runtime>,
        _: &mut World,
    ) -> Result<Option<AttackProfile>, String> {
        Ok(Some(self.base_profile()))
    }
    fn range_changed(&mut self, _: u64, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn refresh_range(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn has_store_energy(&self, _: u64, _: AttackProfile) -> bool {
        self.d["store"] == true
    }
    fn store_energy(&mut self, _: u64, _: AttackProfile, _: &mut World) -> Result<bool, String> {
        self.trace.push(json!(["store"]));
        if self.stored >= 2 {
            return Ok(false);
        }
        self.stored += 1;
        Ok(true)
    }
    fn perform(
        &mut self,
        id: u64,
        _: AttackProfile,
        selected: Selection,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        let used = skill
            .as_deref()
            .map(|s| s.attack_override() || s.targeting_override())
            .unwrap_or(false);
        let payload = attacks::Direct {
            mode: if self.d["healer"] == true {
                attacks::Mode::Heal
            } else {
                attacks::Mode::Damage(DamageType::Physical)
            },
            hits: self.d["hits"].as_u64().unwrap() as u32,
            atk_scale: if used { 1.5 } else { 1.0 },
            skill_damage_mul: 1.0,
            target_damage_mul: 1.0,
            split_mul: None,
            heal_scale: 1.0,
            is_skill: used,
        };
        let area = attacks::Area {
            splash: if self.d["splash"] == true {
                Some(attacks::Splash {
                    radius: 1.5,
                    scale: 0.7,
                    hits_fly: false,
                    fortress_melee: false,
                })
            } else {
                None
            },
            chain: if self.d["chain"] == true {
                Some(attacks::Chain {
                    count: 4.0,
                    radius: 1.7,
                    falloff: 0.15,
                    sluggish: 0.0,
                })
            } else {
                None
            },
            ..Default::default()
        };
        let mut executor = self.executor.take().ok_or("Reentrant test executor")?;
        let result = if self.d["ranged"] == true && self.d["healer"] != true {
            let Some(prepared) = executor.prepare(w, self, id, &selected.targets, payload)? else {
                self.executor = Some(executor);
                return Ok(());
            };
            for &target in &prepared.context.targets {
                let from = w.unit(id)?.position;
                self.flights
                    .as_mut()
                    .ok_or("Reentrant fixture flight queue")?
                    .add(
                        w,
                        Launch {
                            from,
                            source: Some(id),
                            target: Some(target),
                            to: None,
                            speed: 14.0,
                            hit_dead: area.splash.is_some(),
                            max_age: 10.0,
                            on_hit: true,
                            visual: "arrow".into(),
                            speed_changeable: None,
                            removable: None,
                            payload: (id, prepared.attack_id, payload, area),
                        },
                    )?;
            }
            executor.finish(w, self, prepared, skill)
        } else {
            executor.perform_area(w, self, id, &selected.targets, payload, area, skill)
        };
        self.executor = Some(executor);
        result
    }
}
impl attacks::Hooks for Host {
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
                spatial::RadiusRule::Centre
            } else {
                spatial::RadiusRule::Body
            },
            splash,
            limit,
        )
    }
    fn chain_link(&mut self, _: u64, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn sluggish(&mut self, _: u64, _: u64, _: f64, _: &mut World) -> Result<(), String> {
        Err("Fixture has no status handler".into())
    }
    fn has_before_attack(&self) -> bool {
        false
    }
    fn before_attack(&mut self, _: &mut attacks::Context, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn engage(&mut self, _: u64, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn damage(&mut self, w: &mut World, r: &mut hit::Request) -> Result<f64, String> {
        let mut runtime = self.buffs.take().ok_or("Reentrant fixture buff runtime")?;
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, &mut runtime, &mut rng, self, r);
        self.buffs = Some(runtime);
        self.rng = rng;
        result
    }
    fn each_hit(
        &mut self,
        _: attacks::Impact,
        _: Option<&mut Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
    fn after_hit(
        &mut self,
        _: attacks::Impact,
        _: Option<&mut Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
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
struct FlightHost<'a> {
    host: &'a mut Host,
    skill: &'a mut Runtime,
}
impl projectiles::Hooks<(u64, u64, attacks::Direct, attacks::Area)> for FlightHost<'_> {
    fn has_step(&self) -> bool {
        false
    }
    fn step(
        &mut self,
        _: &mut Projectile<(u64, u64, attacks::Direct, attacks::Area)>,
        _: &mut System<(u64, u64, attacks::Direct, attacks::Area)>,
        _: &mut World,
    ) -> Result<bool, String> {
        Ok(false)
    }
    fn impact(
        &mut self,
        p: &mut Projectile<(u64, u64, attacks::Direct, attacks::Area)>,
        target: Option<u64>,
        _: &mut System<(u64, u64, attacks::Direct, attacks::Area)>,
        w: &mut World,
    ) -> Result<(), String> {
        let mut executor = self.host.executor.take().unwrap();
        let result = executor.impact_at(
            w,
            self.host,
            p.payload.0,
            target,
            p.payload.1,
            p.payload.2,
            p.payload.3,
            p.position,
            Some(self.skill),
        );
        self.host.executor = Some(executor);
        result
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; continuous 240-tick combat fixtures"]
fn native_composition_matches_continuous_attacks() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/combat.mjs");
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
        let mut w = World::new(6, 1, 1).unwrap();
        let mut base = Base::default();
        for (k, x) in [("maxHp", 1000.0), ("atk", 80.0), ("bat", 0.2)] {
            base.0.insert(k.into(), x);
        }
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        u.position = [10.0, 4.0];
        u.deployment.tile = [10, 4];
        w.insert_unit(1, u).unwrap();
        let mut base = Base::default();
        base.0.insert(
            "maxHp".into(),
            if d["dead"] == true { 500.0 } else { 100000.0 },
        );
        base.0.insert("def".into(), 20.0);
        base.0.insert("atk".into(), 30.0);
        base.0.insert("res".into(), 30.0);
        base.0.insert("moveSpeed".into(), 2.0);
        let mut e = Unit::new(
            UnitStats::new(base),
            if d["healer"] == true {
                Side::Ally
            } else {
                Side::Enemy
            },
        );
        if d["healer"] == true {
            e.stats.hp = 100.0;
        }
        e.deployed = true;
        e.position = [10.0, 6.0];
        e.deployment.tile = [10, 6];
        e.targeting.spawn_seq = 1;
        if d["mobile"] == true {
            e.motion.route=Some(motion::Route::compile(&json!({"checkpoints":[{"type":"WAIT","time":0.2},{"type":"MOVE","pos":[10,9]},{"type":"MOVE","pos":[10,6]}],"end":[10,9]}),Rect::FULL,64,e.position).unwrap());
        }
        w.insert_unit(2, e).unwrap();
        let mut enemies = if d["healer"] == true { vec![] } else { vec![2] };
        let mut extra_ids = vec![];
        if d["i"].as_u64().unwrap() >= 24 && d["healer"] != true {
            for j in 0..4 {
                let mut base = Base::default();
                base.0.insert(
                    "maxHp".into(),
                    if d["dead"] == true { 500.0 } else { 100000.0 },
                );
                base.0.insert("def".into(), 20.0);
                let mut e = Unit::new(UnitStats::new(base), Side::Enemy);
                e.position = [10.0, 6.8 + j as f64 * 0.6];
                e.motion_fly = j % 2 == 1;
                e.deployed = true;
                e.targeting.spawn_seq = j + 2;
                let id = j + 3;
                w.insert_unit(id, e).unwrap();
                enemies.push(id);
                extra_ids.push(id);
            }
        }
        let bank = Bank::new(
            1,
            match d["kind"].as_str().unwrap() {
                "duration" => Kind::Duration,
                "ammo" => Kind::Ammo,
                _ => Kind::Instant,
            },
            2.0,
            1,
            8,
        )
        .unwrap();
        let spec = Spec {
            duration: 0.3,
            ammo: 2.0,
            sp_type: SpType::Attack,
            rule: Rule::parse(d["rule"].as_str().unwrap()),
            attack: d["attack"] == true,
            modifiers: true,
            ..Spec::default()
        };
        let mut s = Runtime::new(bank, spec, 64).unwrap();
        let mut h = Host {
            d: d.clone(),
            trace: vec![],
            index: Some(Index::new(6, 256).unwrap()),
            enemies,
            grid: Some(
                Grid::new(
                    &json!({"rows":vec!["r".repeat(21);19],"legend":{}}),
                    Rect::FULL,
                )
                .unwrap(),
            ),
            stored: 0,
            executor: Some(attacks::Executor::new(8, 8).unwrap()),
            flights: Some(System::new(128).unwrap()),
            buffs: Some(buffs::Runtime::new(16, 256, 64).unwrap()),
            rng: Rng::new(42),
        };
        h.move_target(&mut w, true).unwrap();
        s.reset(&mut w, &mut h, None).unwrap();
        let mut cache = Commitments::new(1, 8, 8).unwrap();
        let mut states = vec![];
        for tick in 0..240 {
            w.time = tick as f64 / 30.0;
            if d["i"].as_u64().unwrap() >= 40 && d["healer"] != true && tick == 0 {
                let el = [
                    Element::Burn,
                    Element::Neural,
                    Element::Necrosis,
                    Element::Apoptosis,
                    Element::Erosion,
                ][d["i"].as_u64().unwrap() as usize % 5];
                let mut r = h.buffs.take().unwrap();
                element::apply(
                    &mut w,
                    &mut h,
                    &mut r,
                    &mut element::Request::new(Some(1), Some(1), 2, el, 1100.0),
                )
                .unwrap();
                h.buffs = Some(r);
            }
            if d["shooterDeath"] == true && tick == 2 {
                let u = w.unit_mut(1).unwrap();
                u.stats.alive = false;
                u.deployed = false;
                u.removed = true;
                s.end(&mut w, &mut h, "death").unwrap();
            }
            if d["healer"] == true {
                let e = w.unit_mut(2).unwrap();
                e.stats.hp = (e.stats.hp - 3.0).max(1.0);
            }

            let mut buffs = h.buffs.take().unwrap();
            if d["healer"] != true && d["i"].as_u64().unwrap() >= 24 {
                if tick % 31 == 0 {
                    let mut b = Buff {
                        key: Some("battleShield".into()),
                        shield: 50.0,
                        ..Default::default()
                    };
                    b.runtime.duration = 0.3;
                    buffs
                        .add(
                            &mut w,
                            &mut h,
                            2,
                            buffs::Spec {
                                buff: b,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
                if tick % 37 == 0 {
                    let mut b = Buff {
                        key: Some("protection".into()),
                        mods: [("dmgTakenMul".into(), 0.8), ("dodgePhys".into(), 0.2)]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    };
                    b.runtime.duration = 0.5;
                    buffs
                        .add(
                            &mut w,
                            &mut h,
                            2,
                            buffs::Spec {
                                buff: b,
                                mods_present: true,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
            }

            if d["i"].as_u64().unwrap() >= 36 && tick % 23 == 0 {
                let mut b = Buff {
                    key: Some("pulse".into()),
                    mods: [("atkPct".into(), 0.1)].into_iter().collect(),
                    ..Default::default()
                };
                b.runtime.duration = 0.4;
                b.runtime.interval = 0.1;
                b.runtime.on_tick = Some(1);
                buffs
                    .add(
                        &mut w,
                        &mut h,
                        1,
                        buffs::Spec {
                            buff: b,
                            mods_present: true,
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
            let units: Vec<_> = (1..=if extra_ids.is_empty() { 2 } else { 6 }).collect();
            if d["mobile"] == true {
                if tick == 20 {
                    status::apply(
                        &mut w,
                        &mut h,
                        &mut buffs,
                        Rect::FULL,
                        2,
                        "fear",
                        status::Options {
                            duration: 0.6,
                            source: Some(1),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                if tick == 40 {
                    status::apply(
                        &mut w,
                        &mut h,
                        &mut buffs,
                        Rect::FULL,
                        2,
                        "attract",
                        status::Options {
                            duration: 0.6,
                            point: Some([10.0, 6.0]),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                if tick == 90 {
                    h.grid
                        .as_mut()
                        .unwrap()
                        .set_obstacle([10, 7], true, 1)
                        .unwrap();
                }
                if tick == 130 {
                    h.grid
                        .as_mut()
                        .unwrap()
                        .set_obstacle([10, 7], false, 1)
                        .unwrap();
                }
            }
            buffs.tick(&mut w, &mut h, &units, 1.0 / 30.0).unwrap();
            h.buffs = Some(buffs);

            h.move_target(&mut w, tick % 17 < 12).unwrap();
            w.unit_mut(1)
                .unwrap()
                .stats
                .put_named_buff(
                    "control",
                    Buff {
                        flags: if d["stun"] == true && (30..45).contains(&tick) {
                            ["stun".into()].into_iter().collect()
                        } else {
                            Default::default()
                        },
                        ..Buff::default()
                    },
                    8,
                )
                .unwrap();
            cache.capture(&mut w, &mut h, &[1], 1.0 / 30.0).unwrap();
            if d["mobile"] == true && w.unit(2).unwrap().stats.alive {
                let mut buffs = h.buffs.take().unwrap();
                let mut grid = h.grid.take().unwrap();
                let mut rng = h.rng.clone();
                motion::update(
                    &mut w,
                    &mut grid,
                    &mut buffs,
                    &mut rng,
                    &mut h,
                    2,
                    1.0 / 30.0,
                    false,
                    false,
                )
                .unwrap();
                h.buffs = Some(buffs);
                h.grid = Some(grid);
                h.rng = rng;
            }
            h.move_target(&mut w, (tick + 1) % 17 < 12).unwrap();
            if w.unit(1).unwrap().stats.alive && w.unit(1).unwrap().deployed {
                s.tick(&mut w, &mut h, 1.0 / 30.0).unwrap();
            }
            ally::update(&mut w, &mut h, &cache, 1, Some(&mut s), 1.0 / 30.0, 8).unwrap();
            let mut flights = h.flights.take().unwrap();
            flights
                .update(
                    &mut w,
                    &mut FlightHost {
                        host: &mut h,
                        skill: &mut s,
                    },
                    1.0 / 30.0,
                )
                .unwrap();
            h.flights = Some(flights);
            if d["stop"] == true && tick == 100 {
                s.end(&mut w, &mut h, "stopped").unwrap();
            }
            let atk = w.unit_mut(1).unwrap().stats.stats().unwrap().n("atk");
            let element_lock = element::locked(&mut w, 2).unwrap();
            let enemy_stats = w.unit_mut(2).unwrap().stats.stats().unwrap();
            let (enemy_atk, enemy_res, enemy_shield) = (
                enemy_stats.n("atk"),
                enemy_stats.n("res"),
                enemy_stats.n("shield"),
            );
            let u = w.unit(1).unwrap();
            let e = w.unit(2).unwrap();
            states.push(json!({"hp":bits(e.stats.hp),"alive":e.stats.alive,"position":e.position.map(bits),"moving":e.moving,"element":e.element_buildup.map(bits),"elementLock":element_lock,"enemyAtk":bits(enemy_atk),"enemyRes":bits(enemy_res),"shield":bits(enemy_shield),"rng":h.rng.state(),"otherHp":extra_ids.iter().map(|id|bits(w.unit(*id).unwrap().stats.hp)).collect::<Vec<_>>(),"flights":h.flights.as_ref().unwrap().list().len(),"cd":bits(u.cadence.cooldown),"had":u.cadence.had_target,"attacks":u.attacks,"sp":bits(s.bank.sp),"charges":bits(s.bank.charges),"active":s.bank.active,"pending":s.pending,"time":bits(s.time_left),"ammo":bits(s.ammo_left),"last":bits(u.last_attack_at),"atk":bits(atk)}));
        }
        for (tick, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "tick {tick} case {d}");
        }
        assert_eq!(json!(h.trace), v["trace"], "events {d}");
        h.flights.as_mut().unwrap().clear(&mut w).unwrap();
        assert_eq!(w.unit_pins(1).unwrap(), 0);
        assert_eq!(w.unit_pins(2).unwrap(), 0);
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 60);
}
