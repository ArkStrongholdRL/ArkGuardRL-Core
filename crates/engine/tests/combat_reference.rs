//! Continuous integration: real native search + skill + damage/HP paths against
//! upstream updateAlly/performAttack/resolveDamage. Range membership is a fixture.
use arkguard_engine::{
    battle::{
        ally, attacks, blocking,
        damage::DamageType,
        health::{self, Side, Unit, World},
        hit,
        injury::Effects,
        search::{self, AttackProfile, Commitments, Selection},
        skills::{self, Condition, Context, Event, Rule, Runtime, SpType, Spec},
        sp::{Bank, Kind},
        stats::{Base, Buff, UnitStats},
        targeting::{self, Profile, Queries},
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
    in_range: bool,
    stored: u32,
    executor: Option<attacks::Executor>,
    effects: Option<Effects>,
    rng: Rng,
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
    fn body_distance(&mut self, _: u64, _: [f64; 2], _: &mut World) -> Result<f64, String> {
        Ok(2.0)
    }
}
impl search::Host for Host {
    fn enemies(
        &mut self,
        id: u64,
        _: &[usize],
        p: Profile,
        w: &mut World,
        _: usize,
    ) -> Result<Vec<u64>, String> {
        Ok(
            if self.in_range && targeting::can_target_enemy(w, self, Some(id), 2, p)? {
                vec![2]
            } else {
                vec![]
            },
        )
    }
    fn injured_allies(
        &mut self,
        _: u64,
        _: &[usize],
        _: bool,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<u64>, String> {
        Ok(vec![])
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
        Ok(Some(AttackProfile::default()))
    }
    fn range(&mut self, _: u64, _: &World) -> Result<Vec<usize>, String> {
        Ok(vec![1])
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
impl hit::Hooks for Host {}
impl skills::Hooks for Host {
    fn event(
        &mut self,
        event: Event,
        _: &mut Context,
        _: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        match event {
            Event::ApplyModifiers => w.unit_mut(1)?.stats.put_named_buff(
                "skill",
                Buff {
                    mods: [("atkPct".into(), 0.5)].into_iter().collect(),
                    ..Buff::default()
                },
                8,
            )?,
            Event::RemoveModifiers => w.unit_mut(1)?.stats.remove_named_buff("skill"),
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
        Ok(self.in_range && w.unit(2)?.stats.alive)
    }
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(AttackProfile::default()))
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
        Ok(Some(AttackProfile::default()))
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
            mode: attacks::Mode::Damage(DamageType::Physical),
            hits: self.d["hits"].as_u64().unwrap() as u32,
            atk_scale: if used { 1.5 } else { 1.0 },
            skill_damage_mul: 1.0,
            target_damage_mul: 1.0,
            split_mul: None,
            heal_scale: 1.0,
            is_skill: used,
        };
        let mut executor = self.executor.take().ok_or("Reentrant test executor")?;
        let result = executor.perform(w, self, id, &selected.targets, payload, skill);
        self.executor = Some(executor);
        result
    }
}
impl attacks::Hooks for Host {
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
        let mut effects = self.effects.take().ok_or("Reentrant test injury")?;
        let mut rng = self.rng.clone();
        let result = hit::resolve(w, &mut effects, &mut rng, self, r);
        self.effects = Some(effects);
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
        let mut w = World::new(2, 1, 1).unwrap();
        let mut base = Base::default();
        for (k, x) in [("maxHp", 1000.0), ("atk", 80.0), ("bat", 0.2)] {
            base.0.insert(k.into(), x);
        }
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let mut base = Base::default();
        base.0.insert(
            "maxHp".into(),
            if d["dead"] == true { 500.0 } else { 100000.0 },
        );
        base.0.insert("def".into(), 20.0);
        let mut e = Unit::new(UnitStats::new(base), Side::Enemy);
        e.deployed = true;
        e.position = [10.0, 6.0];
        e.targeting.spawn_seq = 1;
        w.insert_unit(2, e).unwrap();
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
            in_range: true,
            stored: 0,
            executor: Some(attacks::Executor::new(8, 8).unwrap()),
            effects: Some(Effects::new(8).unwrap()),
            rng: Rng::new(42),
        };
        s.reset(&mut w, &mut h, None).unwrap();
        let mut cache = Commitments::new(1, 8, 8).unwrap();
        let mut states = vec![];
        for tick in 0..240 {
            w.time = tick as f64 / 30.0;
            h.in_range = tick % 17 < 12;
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
            h.in_range = (tick + 1) % 17 < 12;
            s.tick(&mut w, &mut h, 1.0 / 30.0).unwrap();
            ally::update(&mut w, &mut h, &cache, 1, Some(&mut s), 1.0 / 30.0, 8).unwrap();
            if d["stop"] == true && tick == 100 {
                s.end(&mut w, &mut h, "stopped").unwrap();
            }
            let atk = w.unit_mut(1).unwrap().stats.stats().unwrap().n("atk");
            let u = w.unit(1).unwrap();
            let e = w.unit(2).unwrap();
            states.push(json!({"hp":bits(e.stats.hp),"alive":e.stats.alive,"cd":bits(u.cadence.cooldown),"had":u.cadence.had_target,"attacks":u.attacks,"sp":bits(s.bank.sp),"charges":bits(s.bank.charges),"active":s.bank.active,"pending":s.pending,"time":bits(s.time_left),"ammo":bits(s.ammo_left),"last":bits(u.last_attack_at),"atk":bits(atk)}));
        }
        for (tick, (a, b)) in states
            .iter()
            .zip(v["states"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(a, b, "tick {tick} case {d}");
        }
        assert_eq!(json!(h.trace), v["trace"], "events {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 24);
}
