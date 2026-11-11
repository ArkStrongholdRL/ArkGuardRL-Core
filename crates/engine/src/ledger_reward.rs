//! Stateful public reward adapter, matching `arkguardrl.rewards.NativeProgressCore`.
//! The separate low-level `reward` module remains independent of this ledger.
//!
//! The three component sum is a direct CPython `math.fsum` port, so rewards are
//! bit-identical to the Python class on this host. `passed / boss_round` follows
//! Python's correctly rounded integer division while both operands stay within
//! +/-2**53, which covers every native round count; beyond that domain the two
//! f64 conversions can differ from a true rational quotient by one ulp.
use crate::identity::fingerprint;
use serde_json::{json, Value};

type Result<T> = std::result::Result<T, String>;
pub const REWARD_SCHEMA: &str = "arkguardrl/native-solo-progress-core/v1";
const CORE_BONDS: &[&str] = &[
    "yanShip",
    "sargonShip",
    "victoriaShip",
    "kjeragShip",
    "lateranoShip",
    "egirShip",
    "siracusaShip",
    "kazimierzShip",
];

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key)
        .ok_or_else(|| format!("Missing public field: {key}"))
}
fn integer(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) if !n.is_f64() => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from)),
        _ => None,
    }
}
fn boolean(v: &Value, key: &str) -> Result<bool> {
    field(v, key)?
        .as_bool()
        .ok_or_else(|| format!("Missing boolean {key}"))
}
fn zero(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Bool(b) => !*b,
        _ => false,
    }
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
    }
}

/// Structural validation from observation.py; no catalog or engine queries.
fn validate(state: &Value) -> Result<()> {
    if state.get("schema").and_then(Value::as_str) != Some("arkguardrl/native-session-v2") {
        return Err("Unsupported native observation schema".into());
    }
    let game = boolean(state, "gameEnded")?;
    let personal = boolean(state, "personalEnded")?;
    let decision = boolean(state, "decisionRequired")?;
    if state
        .get("revision")
        .and_then(integer)
        .is_none_or(|v| v < 0)
    {
        return Err("Invalid native revision".into());
    }
    let public = field(state, "public")?;
    let own = field(state, "own")?;
    if field(state, "phase")? != field(public, "phase")?
        || field(state, "round")? != field(public, "round")?
    {
        return Err("Inconsistent native phase/round".into());
    }
    let players = field(public, "players")?
        .as_array()
        .ok_or("Expected native solo seat")?;
    if players.len() != 1
        || field(&players[0], "playerId")? != field(own, "playerId")?
        || own.get("seat").and_then(integer) != Some(0)
    {
        return Err("Expected native solo seat".into());
    }
    let alive = field(own, "alive")?
        .as_bool()
        .ok_or("Missing personal alive flag")?;
    if game && !personal {
        return Err("Game ended without personal endpoint".into());
    }
    if !alive && !personal {
        return Err("Personal defeat not marked".into());
    }
    if personal && decision {
        return Err("Decision after personal endpoint".into());
    }
    if !game && state.get("outcome").is_some_and(|v| !v.is_null()) {
        return Err("Final outcome before game end".into());
    }
    let mut stack = vec![state];
    while let Some(v) = stack.pop() {
        match v {
            Value::Array(a) => stack.extend(a),
            Value::Object(m) => {
                for (key, value) in m {
                    let key = key.to_lowercase();
                    if key.contains("seed") || key.contains("rng") || key.contains("hiddenbossid") {
                        return Err("Forbidden hidden identity in observation".into());
                    }
                    stack.push(value);
                }
            }
            _ => (),
        }
    }
    Ok(())
}

fn core(state: &Value) -> Result<f64> {
    if boolean(state, "personalEnded")? {
        return Ok(0.0);
    }
    let bonds = field(state, "own")?
        .get("bonds")
        .and_then(Value::as_array)
        .ok_or("Explicit public bond list required")?;
    let mut layers = 0.0;
    for bond in bonds {
        if !bond.is_object() {
            return Err("Malformed public bond".into());
        }
        if bond
            .get("bondId")
            .and_then(Value::as_str)
            .is_some_and(|id| CORE_BONDS.contains(&id))
            && bond.get("active").is_some_and(truthy)
            && !bond.get("off").is_some_and(truthy)
        {
            let value = bond
                .get("layers")
                .and_then(Value::as_f64)
                .ok_or("Active CORE layers must be finite and nonnegative")?;
            if !value.is_finite() || value < 0.0 {
                return Err("Active CORE layers must be finite and nonnegative".into());
            }
            // Python max([0.0, ...]) retains its first argument on ties (including -0).
            if value > layers {
                layers = value;
            }
        }
    }
    Ok(0.1 * (layers / 100.0).tanh())
}

/// CPython 3.12 math.fsum for the three bounded finite reward components.
/// Keep separate binary64 operations and the final half-even correction.
/// Source: Python-3.12.13 Modules/mathmodule.c, math_fsum (PSF license).
pub(crate) fn fsum(values: [f64; 3]) -> f64 {
    let mut p = [0.0_f64; 3];
    let mut n = 0;
    for mut x in values {
        let mut i = 0;
        for j in 0..n {
            let mut y = p[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let yr = hi - x;
            let lo = y - yr;
            if lo != 0.0 {
                p[i] = lo;
                i += 1;
            }
            x = hi;
        }
        n = i;
        if x != 0.0 {
            p[n] = x;
            n += 1;
        }
    }
    let mut hi = 0.0;
    let mut lo = 0.0;
    if n > 0 {
        n -= 1;
        hi = p[n];
        while n > 0 {
            let x = hi;
            n -= 1;
            let y = p[n];
            hi = x + y;
            let yr = hi - x;
            lo = y - yr;
            if lo != 0.0 {
                break;
            }
        }
        if n > 0 && ((lo < 0.0 && p[n - 1] < 0.0) || (lo > 0.0 && p[n - 1] > 0.0)) {
            let y = lo * 2.0;
            let x = hi + y;
            let yr = x - hi;
            if y == yr {
                hi = x;
            }
        }
    }
    hi
}

#[derive(Default)]
pub struct NativeProgressCore {
    last: Option<String>,
    ended: bool,
    boss_round: i128,
    difficulty: Value,
    mode_id: String,
    player_id: Value,
    progress: f64,
    potential: f64,
}
impl NativeProgressCore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn reset(&mut self, state: &Value) -> Result<()> {
        if self.last.is_some() {
            return Err("Use a fresh reward adapter per episode".into());
        }
        validate(state)?;
        let public = field(state, "public")?;
        self.boss_round = public
            .get("bossRound")
            .and_then(integer)
            .filter(|n| *n >= 1)
            .ok_or("Positive public bossRound required")?;
        self.difficulty = public.get("difficulty").cloned().unwrap_or(Value::Null);
        let difficulty = self
            .difficulty
            .as_str()
            .filter(|s| ["FUNNY", "NORMAL", "HARD", "ABYSS"].contains(s))
            .ok_or("Known native solo difficulty required")?;
        self.mode_id = format!("mode_single_{}", difficulty.to_lowercase());
        self.player_id = field(field(state, "own")?, "playerId")?.clone();
        if boolean(state, "personalEnded")? {
            return Err("Reset requires a live native episode".into());
        }
        let (progress, _) = self.values(state)?;
        let potential = core(state)?;
        let last = fingerprint(state)?;
        self.progress = progress;
        self.potential = potential;
        self.last = Some(last);
        Ok(())
    }
    fn values(&self, state: &Value) -> Result<(f64, bool)> {
        validate(state)?;
        let public = field(state, "public")?;
        let own = field(state, "own")?;
        if public.get("bossRound").and_then(integer) != Some(self.boss_round)
            || public.get("difficulty") != Some(&self.difficulty)
            || public.get("modeId").and_then(Value::as_str) != Some(self.mode_id.as_str())
            || own.get("playerId") != Some(&self.player_id)
        {
            return Err("Reward episode identity changed".into());
        }
        let players = public
            .get("players")
            .and_then(Value::as_array)
            .ok_or("Exactly one native non-bot solo player required")?;
        if players.len() != 1 || players[0].get("isBot") != Some(&Value::Bool(false)) {
            return Err("Exactly one native non-bot solo player required".into());
        }
        let round = state
            .get("round")
            .and_then(integer)
            .filter(|v| *v >= 0)
            .ok_or("Nonnegative public round required")?;
        let mut passed = (round - 1).max(0).min(self.boss_round - 1);
        let mut won = false;
        if boolean(state, "gameEnded")? {
            let outcome = state
                .get("outcome")
                .filter(|v| v.is_object())
                .ok_or("Clean explicit native outcome required")?;
            let victory = outcome.get("victory").and_then(Value::as_bool);
            if !outcome.get("errors").is_some_and(zero)
                || outcome.get("modeId").and_then(Value::as_str) != Some(self.mode_id.as_str())
                || outcome.get("difficulty") != Some(&self.difficulty)
                || victory.is_none()
            {
                return Err("Clean explicit native outcome required".into());
            }
            let settled = outcome
                .get("players")
                .and_then(Value::as_array)
                .ok_or("Exactly one native player result required")?;
            if settled.len() != 1 {
                return Err("Exactly one native player result required".into());
            }
            let player = &settled[0];
            let alive = boolean(own, "alive")?;
            let pv = player.get("victory").and_then(Value::as_bool);
            let rounds = player.get("roundsPassed").and_then(integer);
            if player.get("playerId") != Some(&self.player_id)
                || pv.is_none()
                || player.get("alive") != Some(&Value::Bool(alive))
                || rounds.is_none_or(|v| v < 0)
                || pv != Some(victory.unwrap() && alive)
            {
                return Err("Inconsistent native solo outcome".into());
            }
            won = pv.unwrap();
            passed = rounds.unwrap();
        } else if boolean(state, "personalEnded")? && boolean(own, "alive")? {
            return Err("Alive personal endpoint without final outcome is ambiguous".into());
        }
        Ok(((passed as f64 / self.boss_round as f64).min(1.0), won))
    }
    pub fn step(&mut self, before: &Value, after: &Value) -> Result<Value> {
        if self.last.is_none() || self.ended || self.last.as_deref() != Some(&fingerprint(before)?)
        {
            return Err("Disconnected or already-ended reward episode".into());
        }
        let (progress, won) = self.values(after)?;
        if progress < self.progress {
            return Err("Public rounds-passed progress regressed".into());
        }
        let potential = core(after)?;
        let terminated = boolean(after, "personalEnded")?;
        let outcome = if terminated {
            if won {
                1.0
            } else {
                -1.0
            }
        } else {
            0.0
        };
        let progress_reward = 0.5 * (progress - self.progress);
        let shaping = potential - self.potential;
        let reward = fsum([outcome, progress_reward, shaping]);
        let last = fingerprint(after)?;
        self.progress = progress;
        self.potential = potential;
        self.last = Some(last);
        self.ended = terminated;
        Ok(json!({
            "reward": reward,
            "components": {
                "outcome": outcome,
                "progress": progress_reward,
                "core_shaping": shaping,
            },
            "progress": progress,
            "terminated": terminated,
            "victory": if terminated { json!(won) } else { Value::Null },
            "schema": REWARD_SCHEMA,
        }))
    }
}
