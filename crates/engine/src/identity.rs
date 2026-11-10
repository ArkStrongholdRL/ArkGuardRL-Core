//! Canonical JSON identities, byte-identical to `arkguardrl.contracts`.
//!
//! Python's `json.dumps(sort_keys=True, separators=(",", ":"),
//! ensure_ascii=False, allow_nan=False)` defines the bytes. That is not
//! `serde_json::to_string`: keys sort by Unicode code point (UTF-8 byte order),
//! floats use `repr` (shortest round-trip, fixed for decpt in -3..=16, `e+NN`
//! beyond, `-0.0` preserved), and non-ASCII strings stay raw UTF-8. So the
//! writer below is explicit, and SHA-256 is implemented on std alone because
//! this crate has no hash dependency.
//!
//! The input domain is `serde_json::Value`: strings contain Unicode scalar
//! values, and integers fit i64/u64. Python's arbitrary-size integers and lone
//! UTF-16 surrogates cannot be represented by that type. Parse JSON with
//! `float_roundtrip` enabled to preserve the original f64 before calling here;
//! the engine already enables it. Nonfinite floats are rejected by the writer
//! (serde_json::Number itself cannot contain them).

use serde_json::{Map, Value};

pub const ACTION_SCHEMA: &str = "arkguardrl/native-intent-v1";
pub const CONTEXT_SCHEMA: &str = "arkguardrl/decision-context-v1";

/// UTF-8 bytes of the canonical form; errors mirror the serializer's refusals.
pub fn canonical(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(256);
    write_value(value, &mut out)?;
    Ok(out)
}

/// Lowercase hex SHA-256 of [`canonical`].
pub fn fingerprint(value: &Value) -> Result<String, String> {
    Ok(hex(&sha256(&canonical(value)?)))
}

/// Identity of one native intent, over `{"schema": ACTION_SCHEMA, "intent": action}`.
///
/// The Python compact key (`board_move`/`hand_move` fast paths, then
/// `_intent_key`) only selects an `lru_cache` slot: `_intent_from_key` rebuilds
/// exactly the original value, and the fast paths' arity checks prove their
/// synthesized move carries no extra field. Its depth/key-count/type limits
/// therefore cannot change these bytes; the board/hand rewrite is mirrored for
/// clarity because it is the only branch that builds a value.
pub fn action_id(action: &Value) -> Result<String, String> {
    let Some(map) = action.as_object() else {
        return Err("A native intent must contain a string t".into());
    };
    if !matches!(map.get("t"), Some(Value::String(_))) {
        return Err("A native intent must contain a string t".into());
    }
    let mut out = Vec::with_capacity(256);
    out.extend_from_slice(b"{\"intent\":");
    match board_or_hand_move(map) {
        Some(intent) => write_value(&intent, &mut out)?,
        None => write_value(action, &mut out)?,
    }
    out.extend_from_slice(b",\"schema\":");
    write_string(ACTION_SCHEMA, &mut out);
    out.push(b'}');
    Ok(hex(&sha256(&out)))
}

/// Identity of the public decision context, ignoring transport timing fields.
pub fn decision_id(state: &Value) -> Result<String, String> {
    let map = state
        .as_object()
        .ok_or_else(|| "Decision state must be an object".to_string())?;
    let schema = map
        .get("schema")
        .ok_or_else(|| "Decision state has no schema".to_string())?;
    let public = map
        .get("public")
        .and_then(Value::as_object)
        .ok_or_else(|| "Decision state has no public object".to_string())?;
    let own = map
        .get("own")
        .ok_or_else(|| "Decision state has no own".to_string())?;
    let personal_end = map
        .get("personalEnded")
        .ok_or_else(|| "Decision state has no personalEnded".to_string())?;
    let game_end = map
        .get("gameEnded")
        .ok_or_else(|| "Decision state has no gameEnded".to_string())?;
    // Canonical key order of the wrapper; written directly to avoid cloning own/public.
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(b"{\"gameEnded\":");
    write_value(game_end, &mut out)?;
    out.extend_from_slice(b",\"observationSchema\":");
    write_value(schema, &mut out)?;
    out.extend_from_slice(b",\"own\":");
    write_value(own, &mut out)?;
    out.extend_from_slice(b",\"personalEnded\":");
    write_value(personal_end, &mut out)?;
    out.extend_from_slice(b",\"public\":");
    write_public(public, &mut out)?;
    out.extend_from_slice(b",\"schema\":");
    write_string(CONTEXT_SCHEMA, &mut out);
    out.push(b'}');
    Ok(hex(&sha256(&out)))
}

/// Rebuild the value `_intent_from_key` yields for a `g.move` fast-path key.
fn board_or_hand_move(action: &Map<String, Value>) -> Option<Value> {
    if action.get("t")?.as_str()? != "g.move" {
        return None;
    }
    let uid = action.get("uid")?;
    let to = action.get("to")?.as_object()?;
    if !is_integer(uid) || !matches!(to.get("area"), Some(Value::String(_))) {
        return None;
    }
    let mut inner = Map::new();
    inner.insert("area".into(), to.get("area")?.clone());
    match to.get("area")?.as_str()? {
        "board" if action.len() == 4 && to.len() == 3 => {
            let row = to.get("row")?;
            let col = to.get("col")?;
            let dir = action.get("dir")?;
            if !is_integer(row) || !is_integer(col) || dir.as_str()?.chars().count() > 256 {
                return None;
            }
            inner.insert("row".into(), row.clone());
            inner.insert("col".into(), col.clone());
            let mut out = Map::new();
            out.insert("t".into(), Value::String("g.move".into()));
            out.insert("uid".into(), uid.clone());
            out.insert("to".into(), Value::Object(inner));
            out.insert("dir".into(), dir.clone());
            Some(Value::Object(out))
        }
        "hand" if action.len() == 3 && to.len() == 2 => {
            let index = to.get("idx")?;
            if !is_integer(index) {
                return None;
            }
            inner.insert("idx".into(), index.clone());
            let mut out = Map::new();
            out.insert("t".into(), Value::String("g.move".into()));
            out.insert("uid".into(), uid.clone());
            out.insert("to".into(), Value::Object(inner));
            Some(Value::Object(out))
        }
        _ => None,
    }
}

/// Python `type(v) is int`: JSON integers, never booleans or floats.
fn is_integer(value: &Value) -> bool {
    matches!(value, Value::Number(n) if !n.is_f64() && (n.is_i64() || n.is_u64()))
}

fn write_public(public: &Map<String, Value>, out: &mut Vec<u8>) -> Result<(), String> {
    let mut entries: Vec<(&str, &Value)> = public
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "serverNow" | "deadline" | "overtimeAt"))
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
    out.push(b'{');
    for (index, (key, value)) in entries.iter().enumerate() {
        if index > 0 {
            out.push(b',');
        }
        write_string(key, out);
        out.push(b':');
        write_value(value, out)?;
    }
    out.push(b'}');
    Ok(())
}

fn write_value(value: &Value, out: &mut Vec<u8>) -> Result<(), String> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(number) => write_number(number, out)?,
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_value(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&str, &Value)> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            out.push(b'{');
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_string(key, out);
                out.push(b':');
                write_value(item, out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_number(number: &serde_json::Number, out: &mut Vec<u8>) -> Result<(), String> {
    if number.is_f64() {
        return write_float(number.as_f64().unwrap_or(f64::NAN), out);
    }
    if let Some(value) = number.as_u64() {
        out.extend_from_slice(value.to_string().as_bytes());
    } else if let Some(value) = number.as_i64() {
        out.extend_from_slice(value.to_string().as_bytes());
    } else {
        return Err(format!("Unrepresentable JSON integer {number}"));
    }
    Ok(())
}

/// Python `float.__repr__`, as used by `json.dumps` for every JSON float.
fn write_float(value: f64, out: &mut Vec<u8>) -> Result<(), String> {
    if !value.is_finite() {
        return Err("Out of range float values are not JSON compliant".into());
    }
    if value == 0.0 {
        out.extend_from_slice(if value.is_sign_negative() {
            b"-0.0"
        } else {
            b"0.0"
        });
        return Ok(());
    }
    // Rust's shortest formatter can break an exact halfway tie upward, while
    // dtoa mode 0 (Python repr) rounds the last digit half-to-even. Take the
    // shortest length from `{:e}`, then re-render at that precision, where the
    // fixed-precision path rounds the exact binary value correctly.
    let shortest = format!("{value:e}");
    let shortest_digits = shortest
        .trim_start_matches('-')
        .split_once('e')
        .map(|(mantissa, _)| mantissa.bytes().filter(|&b| b != b'.').count())
        .ok_or_else(|| format!("Malformed float exponent in {shortest}"))?;
    let scientific = format!("{:.1$e}", value, shortest_digits - 1);
    let (negative, body) = match scientific.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, scientific.as_str()),
    };
    let (mantissa, exponent) = body
        .split_once('e')
        .ok_or_else(|| format!("Malformed float exponent in {scientific}"))?;
    let exponent: i32 = exponent
        .parse()
        .map_err(|_| format!("Malformed float exponent in {scientific}"))?;
    let mut digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    // A minimal representation cannot end in an insignificant zero, but drop
    // one anyway: the decimal point position depends only on the exponent.
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    let length = digits.len() as i32;
    let decpt = exponent + 1; // Decimal point position: value = 0.digits * 10^decpt.
    let mut text = String::with_capacity(digits.len() + 8);
    if negative {
        text.push('-');
    }
    if decpt <= -4 || decpt > 16 {
        // Python uses exponential notation outside that window, two-digit
        // exponent minimum, and no forced ".0" in the mantissa.
        text.push(digits.as_bytes()[0] as char);
        if length > 1 {
            text.push('.');
            text.push_str(&digits[1..]);
        }
        text.push('e');
        let exponent = decpt - 1;
        text.push(if exponent < 0 { '-' } else { '+' });
        let magnitude = exponent.abs();
        if magnitude < 10 {
            text.push('0');
        }
        text.push_str(&magnitude.to_string());
    } else if decpt <= 0 {
        text.push_str("0.");
        for _ in 0..(-decpt) {
            text.push('0');
        }
        text.push_str(&digits);
    } else {
        let split = decpt as usize;
        if split >= digits.len() {
            text.push_str(&digits);
            for _ in 0..(split - digits.len()) {
                text.push('0');
            }
            text.push_str(".0");
        } else {
            text.push_str(&digits[..split]);
            text.push('.');
            text.push_str(&digits[split..]);
        }
    }
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

fn write_string(text: &str, out: &mut Vec<u8>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    let bytes = text.as_bytes();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte >= 0x20 && byte != b'"' && byte != b'\\' {
            continue;
        }
        out.extend_from_slice(&bytes[start..index]);
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x00..=0x1f => {
                out.extend_from_slice(b"\\u00");
                out.push(HEX[(byte >> 4) as usize]);
                out.push(HEX[(byte & 0x0f) as usize]);
            }
            _ => unreachable!(),
        }
        start = index + 1;
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut blocks = data.chunks_exact(64);
    for block in &mut blocks {
        compress(&mut state, block.try_into().unwrap());
    }
    let remainder = blocks.remainder();
    let mut tail = [0u8; 128];
    tail[..remainder.len()].copy_from_slice(remainder);
    tail[remainder.len()] = 0x80;
    let padded = if remainder.len() < 56 { 64 } else { 128 };
    tail[padded - 8..padded].copy_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());
    for block in tail[..padded].chunks_exact(64) {
        compress(&mut state, block.try_into().unwrap());
    }
    let mut digest = [0u8; 32];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut schedule = [0u32; 64];
    for index in 0..16 {
        schedule[index] = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
    }
    for index in 16..64 {
        let s0 = schedule[index - 15].rotate_right(7)
            ^ schedule[index - 15].rotate_right(18)
            ^ (schedule[index - 15] >> 3);
        let s1 = schedule[index - 2].rotate_right(17)
            ^ schedule[index - 2].rotate_right(19)
            ^ (schedule[index - 2] >> 10);
        schedule[index] = schedule[index - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[index - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(SHA256_K[index])
            .wrapping_add(schedule[index]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}
