use arkguard_engine::battle::clock::{Clock, End, EndState, Host, Phase};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
const PHASES: [Phase; 15] = [
    Phase::Start,
    Phase::Scheduled,
    Phase::Spawns,
    Phase::Dp,
    Phase::Buffs,
    Phase::AttackCommit,
    Phase::Enemies,
    Phase::EnemyIndex,
    Phase::Allies,
    Phase::Rate,
    Phase::Projectiles,
    Phase::Redeploy,
    Phase::Boss,
    Phase::TickHook,
    Phase::EndCheck,
];
fn name(p: Phase) -> &'static str {
    match p {
        Phase::Start => "start",
        Phase::Scheduled => "scheduled",
        Phase::Spawns => "spawns",
        Phase::Dp => "dp",
        Phase::Buffs => "buffs",
        Phase::AttackCommit => "attackCommit",
        Phase::Enemies => "enemies",
        Phase::EnemyIndex => "enemyIndex",
        Phase::Allies => "allies",
        Phase::Rate => "rate",
        Phase::Projectiles => "projectiles",
        Phase::Redeploy => "redeploy",
        Phase::Boss => "boss",
        Phase::TickHook => "tickHook",
        Phase::EndCheck => "endCheck",
    }
}
struct Driver {
    stop: Option<Phase>,
    reenter: bool,
    hook: bool,
    removed: bool,
    trace: Vec<String>,
    policy: Option<EndState>,
    reason: Option<String>,
}
impl Host for Driver {
    fn phase(&mut self, p: Phase, c: &mut Clock) -> Result<(), String> {
        if self.policy.is_none() {
            self.trace.push(name(p).into());
        }
        if self.reenter {
            c.step(self)?;
        }
        if p == Phase::Start {
            c.mark_started();
        }
        if self.stop == Some(p) {
            c.force_end(self, "requested".into())?;
        }
        Ok(())
    }
    fn compact_enemies(&mut self, _: &mut Clock) -> Result<(), String> {
        if self.policy.is_none() {
            self.trace.push("compact".into());
        }
        Ok(())
    }
    fn has_removed(&self) -> bool {
        self.removed
    }
    fn release_removed(&mut self, _: &mut Clock) -> Result<(), String> {
        self.trace.push("release".into());
        self.removed = false;
        Ok(())
    }
    fn has_tick_hook(&self) -> bool {
        self.hook
    }
    fn end_state(&self) -> Option<EndState> {
        self.policy
    }
    fn finish(&mut self, e: &End, _: &Clock) -> Result<(), String> {
        let r = match e {
            End::Cleared => "cleared",
            End::Timeout => "timeout",
            End::Forced => "forced",
        };
        self.reason = Some(r.into());
        self.trace.push(format!("finish:{r}"));
        Ok(())
    }
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT and Node; lifecycle orchestration only"]
fn tick_order_and_end_policy_match() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/clock.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let policy = v["policy"] == true;
        let mut h = Driver {
            stop: if policy || v["stop"] == -1 {
                None
            } else {
                Some(PHASES[v["stop"].as_u64().unwrap() as usize])
            },
            reenter: v["reenter"] == true,
            hook: v["hook"] == true,
            removed: !policy,
            trace: vec![],
            policy: None,
            reason: None,
        };
        if policy {
            h.policy = Some(EndState {
                shared_hp: v["shared"].as_f64(),
                boss_like: v["boss"] == true,
                auto_finish: v["auto"] == true,
                pending_spawns: v["pending"] == true,
                enemy_alive: v["enemy"] == true,
                pending_holds: v["hold"] == true,
                time_limit: if v["timeout"] == true { 0.0 } else { 10.0 },
                max_battle_time: 180.0,
            });
        }
        let mut c = Clock::new();
        c.run_steps(&mut h, if policy { 1 } else { 3 }).unwrap();
        if policy {
            assert_eq!(json!(h.reason), v["reason"], "policy {v}");
        } else {
            assert_eq!(json!(h.trace), v["trace"], "case {v}");
            assert_eq!(c.time().to_bits(), v["time"].as_f64().unwrap().to_bits());
            assert_eq!(c.ticks(), v["ticks"].as_u64().unwrap());
            assert_eq!(c.finished(), v["finished"] == true);
        }
        count += 1;
    }
    assert!(child.wait().unwrap().success());
    assert_eq!(count, 256);
}
