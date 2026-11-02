use arkguard_engine::battle::{
    health::{Side, Unit, World},
    search::AttackProfile,
    skills::{self, Condition, Context, Event, Rule, Runtime, SpType, Spec},
    sp::{Bank, Kind},
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
    installed: bool,
    restarted: bool,
}
impl skills::Hooks for Host {
    fn event(
        &mut self,
        event: Event,
        c: &mut Context,
        s: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        match event {
            Event::ApplyModifiers => {
                self.installed = true;
                self.trace.push(json!(["add"]));
            }
            Event::RemoveModifiers => {
                self.installed = false;
                self.trace.push(json!(["remove"]));
            }
            Event::RefreshRange => self.trace.push(json!(["range"])),
            Event::Start => self.trace.push(json!(["start", c.reason])),
            Event::End => {
                self.trace.push(json!(["end", c.reason, self.installed]));
                if self.d["reactivate"] == true && !self.restarted && c.reason == "stopped" {
                    self.restarted = true;
                    s.activate(w, self, "restart", true)?;
                }
            }
            Event::SkillStart => {
                self.trace.push(json!(["skillStart"]));
                if s.bank.kind == Kind::Ammo {
                    s.add_ammo(1.0);
                }
            }
            Event::SkillEnd => self.trace.push(json!(["skillEnd"])),
            Event::DisplayOn => self.trace.push(json!(["display", 1])),
            Event::DisplayOff => self.trace.push(json!(["display", 0])),
            Event::PassiveWindow => self.trace.push(json!(["after"])),
            Event::Tick => {
                self.trace.push(json!(["tick", bits(c.dt)]));
                if self.d["tickExtend"] == true {
                    s.extend(0.1);
                }
            }
            Event::Attack => {
                self.trace.push(json!(["attack"]));
                if self.d["freeAmmo"] == true {
                    c.no_ammo = true;
                }
            }
            Event::AmmoUsed => self.trace.push(json!(["ammoUsed"])),
            Event::SpGain => {
                self.trace.push(json!(["gain", c.reason, bits(c.amount)]));
                c.amount *= 0.75;
            }
        }
        Ok(())
    }
    fn condition(&mut self, c: Condition, _: &Runtime, _: &mut World) -> Result<bool, String> {
        Ok(match c {
            Condition::ExtraWithoutAttack => false,
            _ => self.d["condition"] == true,
        })
    }
    fn profile(&mut self, _: u64, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(AttackProfile {
            no_attack: self.d["noAttack"] == true,
            ..AttackProfile::default()
        }))
    }
    fn has_sp_hook(&self) -> bool {
        true
    }
}
fn snap(s: &Runtime, w: &mut World) -> Value {
    json!({"sp":bits(s.bank.sp),"charges":bits(s.bank.charges),"active":s.bank.active,"pending":s.pending,"time":bits(s.time_left),"ammo":bits(s.ammo_left),"maxAmmo":bits(s.ammo_max),"activations":s.activations,"last":bits(s.last_start),"op":bits(s.op_ready_at),"next":bits(s.next_cast_at),"attack":s.attack_override(),"targeting":s.targeting_override(),"total":bits(s.bank.total(w).unwrap())})
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; skill lifecycle sequences"]
fn lifecycle_sequences_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/skills.mjs");
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
        let mut w = World::new(1, 1, 1).unwrap();
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.0);
        base.0.insert("atk".into(), 10.0);
        let mut u = Unit::new(UnitStats::new(base), Side::Ally);
        u.deployed = true;
        w.insert_unit(1, u).unwrap();
        let none = d["noSkill"] == true;
        let kind = if none {
            Kind::Instant
        } else {
            match d["kind"].as_str().unwrap() {
                "charges" => Kind::Charges,
                "duration" => Kind::Duration,
                "ammo" => Kind::Ammo,
                "toggle" => Kind::Toggle,
                "passive" => Kind::Passive,
                _ => Kind::Instant,
            }
        };
        let mut bank = Bank::new(1, kind, 3.0, 2, 16).unwrap();
        bank.no_skill = none;
        let spec = if none {
            Spec::default()
        } else {
            Spec {
                duration: d["duration"].as_f64().unwrap(),
                ammo: 2.0,
                init_sp: 6.0,
                sp_type: match d["spType"].as_str().unwrap() {
                    "attack" => SpType::Attack,
                    "hurt" => SpType::Hurt,
                    "none" => SpType::None,
                    _ => SpType::Time,
                },
                manual: d["manual"] == true,
                rule: Rule::parse(d["rule"].as_str().unwrap()),
                attack: d["attack"] == true,
                targeting: d["targeting"] == true,
                modifiers: d["mod"] == true,
                range_change: d["targeting"] == true,
                trigger_allies: d["allies"] == true,
                activate_on_deploy: d["deploy"] == true,
                on_tick: true,
                on_attack: true,
                ..Spec::default()
            }
        };
        let mut s = Runtime::new(bank, spec, 64).unwrap();
        let mut h = Host {
            d: d.clone(),
            trace: vec![],
            installed: false,
            restarted: false,
        };
        s.reset(&mut w, &mut h, None).unwrap();
        w.unit_mut(1).unwrap().stats.set_buffs(vec![Buff {
            flags: ["noSp", "stun", "silence"]
                .into_iter()
                .filter(|k| d[*k] == true)
                .map(str::to_string)
                .collect(),
            ..Buff::default()
        }]);
        let mut states = vec![snap(&s, &mut w)];
        for op in 0..13 {
            match op {
                0 => {
                    s.activate(&mut w, &mut h, "manual", false).unwrap();
                }
                1 => {
                    w.time = 1.0 / 30.0;
                    s.tick(&mut w, &mut h, 1.0 / 30.0).unwrap();
                }
                2 => s.after_attack(&mut w, &mut h, &[1], true, true).unwrap(),
                3 => s.after_attack(&mut w, &mut h, &[1], true, false).unwrap(),
                4 => {
                    s.add_ammo(2.0);
                    s.extend(0.2);
                }
                5 => {
                    w.time = 0.5;
                    s.tick(&mut w, &mut h, 0.5).unwrap();
                }
                6 => s.end(&mut w, &mut h, "stopped").unwrap(),
                7 => {
                    s.activate(&mut w, &mut h, "free", true).unwrap();
                }
                8 => s.on_damaged(&mut w, &mut h).unwrap(),
                9 => {
                    s.about_to_attack(&mut w, &mut h).unwrap();
                }
                10 => {
                    w.time = 4.0;
                    s.tick(&mut w, &mut h, 0.1).unwrap();
                }
                11 => {
                    w.time = 5.0;
                    s.tick(&mut w, &mut h, 3.0).unwrap();
                }
                _ => s.end(&mut w, &mut h, "death").unwrap(),
            };
            states.push(snap(&s, &mut w));
        }
        assert_eq!(json!(states), v["states"], "state {d}");
        assert_eq!(json!(h.trace), v["trace"], "events {d}");
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 384);
}
