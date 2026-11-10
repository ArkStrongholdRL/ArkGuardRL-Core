//! Public-state projection and dynamic policy wire layout (v7).
//! Static metadata is compiled once from PublicCatalog, never from engine internals.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[path = "candidate_inputs.rs"]
mod candidate_inputs;

type Result<T> = std::result::Result<T, String>;
const MAX_ENTITIES: usize = 128;
const MAX_ENEMIES: usize = 64;
const MAX_ACTIONS: usize = 16384;
const OBS_DIM: usize = 4784;
const ACTION_DIM: usize = 41;
const BOND_START: usize = 42 + 128 * 28;
const GRID_START: usize = BOND_START + 23 * 5;
const ENEMY_START: usize = GRID_START + 36 * 5;
const CONTROL_START: usize = ENEMY_START + 64 * 12;
const DIRECTIONS: [&str; 4] = ["RIGHT", "UP", "DOWN", "LEFT"];
const PHASES: [&str; 12] = [
    "INFO_CHECK",
    "BAND_DRAFT",
    "BATTLE_CHECK",
    "ROUND_START",
    "SP_DRAFT",
    "PREP",
    "COMBAT",
    "UNITE",
    "SETTLE",
    "FINAL_ASSAULT",
    "HIDDEN_CORE",
    "RESULT",
];
const ACTIONS: [&str; 14] = [
    "infoReady",
    "band",
    "choice",
    "buy",
    "reward",
    "refresh",
    "freeze",
    "levelUp",
    "sell",
    "move",
    "equip",
    "art",
    "destroy",
    "ready",
];
const MOVES: [&str; 6] = [
    "deploy",
    "reserve_replace",
    "reposition",
    "board_swap",
    "withdraw",
    "reserve_move",
];
const SCHEDULES: [&str; 19] = [
    "buy", "levelUp", "refresh", "freeze", "sell", "deploy", "replace", "adjust", "withdraw",
    "equip", "art", "destroy", "ready", "confirm", "band", "bandSkip", "choice", "reward", "wait",
];
const BONDS: [&str; 23] = [
    "arcaneShip",
    "deputShip",
    "egirShip",
    "emptyShip",
    "indomShip",
    "investShip",
    "kazimierzShip",
    "kjeragShip",
    "lateranoShip",
    "maniShip",
    "miraShip",
    "preciShip",
    "raidShip",
    "sargonShip",
    "siracusaShip",
    "skillfulShip",
    "soloShip",
    "steadShip",
    "suntShip",
    "swiftShip",
    "victoriaShip",
    "visiShip",
    "yanShip",
];
const ERRORS: [&str; 15] = [
    "NO_FUNDS",
    "HAND_FULL",
    "BOARD_FULL",
    "BAD_TILE",
    "BAD_TARGET",
    "SOLD_OUT",
    "MAX_LEVEL",
    "WRONG_PHASE",
    "NOT_YOUR_TURN",
    "ALREADY",
    "TEMP_NOT_EMPTY",
    "ELIMINATED",
    "NOT_READY",
    "BAD_MSG",
    "OTHER",
];
const CHANGES: [&str; 8] = [
    "funds_delta",
    "spent_delta",
    "refreshes_delta",
    "merges_delta",
    "level_delta",
    "deploy_delta",
    "hand_free_delta",
    "temp_used_delta",
];
const AREAS: [&str; 8] = [
    "board",
    "hand",
    "temp",
    "equipped",
    "shop",
    "offer",
    "faction",
    "active_effect",
];
const EMPTY: Value = Value::Null;

fn object(v: &Value) -> Result<&serde_json::Map<String, Value>> {
    v.as_object().ok_or_else(|| "Expected public object".into())
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().ok_or_else(|| "Expected public array".into())
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    object(v)?
        .get(key)
        .ok_or_else(|| format!("Missing public field: {key}"))
}
fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&EMPTY)
}
fn text(v: &Value) -> Result<&str> {
    v.as_str().ok_or_else(|| "Expected public string".into())
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    text(field(v, key)?)
}
fn number(v: &Value) -> Result<f64> {
    v.as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| "Structured features require finite public numbers".into())
}
fn num(v: &Value, key: &str, default: f64) -> Result<f64> {
    match v.get(key) {
        Some(n) => number(n),
        None => Ok(default),
    }
}
fn integer(v: &Value) -> Result<i64> {
    v.as_i64().ok_or_else(|| "Expected public integer".into())
}
fn index(v: &Value) -> Result<usize> {
    let n = integer(v)?;
    usize::try_from(n).map_err(|_| "Expected nonnegative public index".into())
}
// Python truthiness is used only by the original boolean/default expressions.
fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.) != 0.,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
fn bit(b: bool) -> f64 {
    if b {
        1.
    } else {
        0.
    }
}
// Python float(bool/number) admits both JSON booleans and numbers.
fn float_value(v: &Value) -> Result<f64> {
    match v {
        Value::Bool(b) => Ok(bit(*b)),
        Value::Null => Err("Structured features require finite public numbers".into()),
        _ => number(v),
    }
}
fn float_field(v: &Value, key: &str, default: f64) -> Result<f64> {
    match v.get(key) {
        Some(value) => float_value(value),
        None => Ok(default),
    }
}
fn scalar(v: f64, scale: f64) -> f64 {
    (v / scale).tanh()
}
fn flags(out: &mut Vec<f64>, value: &Value, vocabulary: &[&str]) {
    out.extend(
        vocabulary
            .iter()
            .map(|key| bit(value.as_str() == Some(*key))),
    );
}
fn identity(kind: &str, id: &str) -> (String, String) {
    (kind.to_owned(), id.to_owned())
}
fn cell(row: i64, col: i64) -> Option<usize> {
    if (9..13).contains(&row) && (2..11).contains(&col) {
        Some(((row - 9) * 9 + col - 2) as usize)
    } else {
        None
    }
}
// Python tuple/dict keys treat integral floats as their integer counterparts.
fn coord(v: &Value) -> Result<Option<i64>> {
    let n = number(v)?;
    if n.fract() != 0. || n.abs() > 1e15 {
        return Ok(None);
    }
    Ok(Some(n as i64))
}

fn uid(v: &Value) -> Result<String> {
    // JSON object keys in progress use str(uid), while refs preserve UID type.
    match v {
        Value::String(s) => Ok(format!("s:{s}")),
        Value::Number(_) if v.as_i64().is_some() || v.as_u64().is_some() => Ok(format!("n:{v}")),
        _ => Err("Invalid public UID".into()),
    }
}
fn progress_uid(v: &Value) -> Result<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(_) => {
            uid(v)?;
            Ok(v.to_string())
        }
        Value::Null => Ok("None".into()),
        _ => Err("Invalid public UID".into()),
    }
}
fn optional_object(v: &Value) -> Result<&Value> {
    if !v.is_null() {
        object(v)?;
    }
    Ok(v)
}
fn optional_array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    match v.get(key) {
        None => Ok(&[]),
        Some(a) => Ok(array(a)?.as_slice()),
    }
}

#[derive(Clone)]
struct Definition {
    index: usize,
    ranges: Vec<(i64, i64)>,
    air: bool,
    base: String,
    golden: bool,
    upgrade: f64,
    domain: Value,
}

pub struct Encoded {
    pub observation: Vec<f32>,
    pub candidates: Vec<f32>,
    pub schedules: Vec<i64>,
    pub experts: Vec<i64>,
}

pub struct PolicyEncoder {
    definitions: HashMap<(String, String), Definition>,
    entities: HashMap<(String, String), usize>,
    enemies: HashMap<String, usize>,
    bands: Vec<String>,
    choices: HashMap<(String, String), Value>,
    config: Value,
    diy: Value,
}

struct Entity {
    row: Value,
    area: usize,
    definition: (String, String),
}
struct Projection {
    entities: Vec<Entity>,
    uids: HashMap<String, usize>,
    shop: HashMap<usize, usize>,
    offer: HashMap<usize, usize>,
    choice: HashMap<usize, usize>,
    bands: HashMap<String, usize>,
    families: HashMap<String, [usize; 4]>,
    occupied: HashMap<(i64, i64), usize>,
}
impl Projection {
    fn new() -> Self {
        Self {
            entities: Vec::with_capacity(MAX_ENTITIES),
            uids: HashMap::with_capacity(MAX_ENTITIES),
            shop: HashMap::new(),
            offer: HashMap::new(),
            choice: HashMap::new(),
            bands: HashMap::new(),
            families: HashMap::new(),
            occupied: HashMap::with_capacity(36),
        }
    }
    fn add(&mut self, row: Value, area: usize, definition: (String, String)) -> Result<usize> {
        if self.entities.len() >= MAX_ENTITIES {
            return Err("Public entity capacity exceeded; no truncation".into());
        }
        self.entities.push(Entity {
            row,
            area,
            definition,
        });
        Ok(self.entities.len())
    }
    fn reference(&self, value: &Value) -> Result<usize> {
        if value.is_null() {
            return Ok(0);
        }
        self.uids
            .get(&uid(value)?)
            .copied()
            .ok_or_else(|| "Unknown public entity UID reference".into())
    }
    // Receipts may refer to a piece sold/merged away since the prior decision.
    fn receipt_reference(&self, value: &Value) -> Result<usize> {
        if value.is_null() {
            return Ok(0);
        }
        Ok(self.uids.get(&uid(value)?).copied().unwrap_or(0))
    }
}

impl PolicyEncoder {
    pub fn new(metadata: &Value) -> Result<Self> {
        let mut definitions = HashMap::new();
        let mut definition_indices = HashSet::new();
        let rows = array(field(metadata, "definitions")?)?;
        for row in rows {
            let kind = string(row, "kind")?;
            let id = string(row, "id")?;
            let i = index(field(row, "index")?)?;
            if i == 0 || i > rows.len() || !definition_indices.insert(i) {
                return Err("Invalid definition index".into());
            }
            let mut ranges = Vec::new();
            for pair in array(field(row, "ranges")?)? {
                let p = array(pair)?;
                if p.len() != 2 {
                    return Err("Invalid static observation range".into());
                }
                let (a, b) = (integer(&p[0])?, integer(&p[1])?);
                if !(-128..=128).contains(&a) || !(-128..=128).contains(&b) {
                    return Err("Invalid static observation range".into());
                }
                ranges.push((a, b));
            }
            if ranges.len() > 24 {
                return Err("Static range capacity exceeded".into());
            }
            let def = Definition {
                domain: get(row, "domain").clone(),
                index: i,
                ranges,
                air: field(row, "air")?
                    .as_bool()
                    .ok_or("Invalid definition air flag")?,
                base: string(row, "base")?.to_owned(),
                golden: field(row, "isGolden")?
                    .as_bool()
                    .ok_or("Invalid definition golden flag")?,
                upgrade: if truth(get(row, "upgradeNum")) {
                    number(get(row, "upgradeNum"))?
                } else {
                    0.
                },
            };
            if definitions.insert(identity(kind, id), def).is_some() {
                return Err("Duplicate definition identity".into());
            }
        }
        let mut entities = HashMap::new();
        let mut indices = HashSet::new();
        let entity_rows = array(field(metadata, "entities")?)?;
        if entity_rows.len() >= 1024 {
            return Err("Catalog entity vocabulary exceeds contract".into());
        }
        for row in entity_rows {
            let (kind, id) = (string(row, "kind")?, string(row, "id")?);
            let i = index(field(row, "index")?)?;
            if i == 0
                || i >= 1024
                || !indices.insert(i)
                || entities.insert(identity(kind, id), i).is_some()
            {
                return Err("Invalid catalog entity index".into());
            }
        }
        // Historical draft order is catalog.band_ids (sortId/mode filtered),
        // not the lexicographic catalog embedding order.
        let mut bands = Vec::new();
        let mut seen_bands = HashSet::new();
        for id in optional_array(metadata, "band_ids")? {
            let id = text(id)?;
            if !entities.contains_key(&identity("band", id)) || !seen_bands.insert(id) {
                return Err("Unknown or duplicate public draft band".into());
            }
            bands.push(id.to_owned());
        }
        let mut enemies = HashMap::new();
        let mut indices = HashSet::new();
        let enemy_rows = array(field(metadata, "enemies")?)?;
        if enemy_rows.len() >= 2048 {
            return Err("Catalog enemy vocabulary exceeds contract".into());
        }
        for row in enemy_rows {
            let i = index(field(row, "index")?)?;
            if i == 0
                || i >= 2048
                || !indices.insert(i)
                || enemies.insert(string(row, "id")?.to_owned(), i).is_some()
            {
                return Err("Invalid catalog enemy index".into());
            }
        }
        let mut choices = HashMap::new();
        for row in array(field(metadata, "choices")?)? {
            let (kind, id) = (string(row, "kind")?, string(row, "id")?);
            if !["bounty", "tactic"].contains(&kind)
                || !definitions.contains_key(&identity(kind, id))
                || choices.insert(identity(kind, id), row.clone()).is_some()
            {
                return Err("Invalid canonical public choice".into());
            }
        }
        let config = field(metadata, "config")?.clone();
        object(&config)?;
        Ok(Self {
            definitions,
            entities,
            enemies,
            bands,
            choices,
            config,
            diy: metadata
                .get("diy")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        })
    }

    fn definition(&self, key: &(String, String)) -> Result<&Definition> {
        self.definitions
            .get(key)
            .ok_or_else(|| format!("Unknown public definition: {}/{}", key.0, key.1))
    }
    fn row_key(&self, row: &Value) -> Result<(String, String)> {
        let kind = string(row, "kind")?;
        let id = string(row, "id")?;
        let key = identity(kind, id);
        self.definition(&key)?;
        if kind == "bounty" || kind == "tactic" {
            let choice_key = identity(
                kind,
                text(row.get("effectId").unwrap_or(field(row, "id")?))?,
            );
            let canonical = self
                .choices
                .get(&choice_key)
                .ok_or("Unknown public choice effect")?;
            for field in ["enemyKey", "payout", "coin", "count", "rounds"] {
                if let Some(value) = row.get(field) {
                    if !python_equal(value, get(canonical, field)) {
                        return Err(format!(
                            "Public choice differs from pinned definition: {field}"
                        ));
                    }
                }
            }
        }
        Ok(key)
    }
    fn effect_key(&self, effect: &Value) -> Result<(String, String)> {
        let mut id = match effect.get("id") {
            Some(v) => text(v)?,
            None => "",
        }
        .split('#')
        .next()
        .unwrap_or("");
        if id.starts_with("bounty:") {
            id = get(effect, "iconId").as_str().unwrap_or("");
        }
        for kind in ["bounty", "tactic"] {
            let key = identity(kind, id);
            if self.choices.contains_key(&key) {
                return Ok(key);
            }
        }
        let key = identity("effect", "unresolved");
        self.definition(&key)?;
        Ok(key)
    }

    fn project(&self, state: &Value) -> Result<Projection> {
        let own = field(state, "own")?;
        let public = field(state, "public")?;
        let shop = field(own, "shop")?;
        let phase = string(state, "phase")?;
        let mut p = Projection::new();
        for (area, name) in AREAS[..3].iter().enumerate() {
            for row in array(field(own, name)?)? {
                if !truth(row) {
                    continue;
                }
                let reference = p.add(row.clone(), area, self.row_key(row)?)?;
                if p.uids.insert(uid(field(row, "uid")?)?, reference).is_some() {
                    return Err("Duplicate public UID".into());
                }
                for item in optional_array(row, "items")? {
                    let mut equipped = object(item)?.clone();
                    equipped
                        .entry("kind")
                        .or_insert(Value::String("item".into()));
                    equipped.insert("ownerUid".into(), field(row, "uid")?.clone());
                    let equipped = Value::Object(equipped);
                    let reference = p.add(equipped.clone(), 3, self.row_key(&equipped)?)?;
                    if p.uids
                        .insert(uid(field(item, "uid")?)?, reference)
                        .is_some()
                    {
                        return Err("Duplicate public UID".into());
                    }
                }
            }
        }
        for (i, row) in array(field(shop, "slots")?)?.iter().enumerate() {
            if truth(row) && !truth(get(row, "sold")) {
                let r = p.add(row.clone(), 4, self.row_key(row)?)?;
                p.shop.insert(i, r);
            }
        }
        let reward = optional_object(get(shop, "rewardOffer"))?;
        for (i, row) in optional_array(reward, "slots")?.iter().enumerate() {
            if truth(row) && !truth(get(row, "sold")) {
                let r = p.add(row.clone(), 5, self.row_key(row)?)?;
                p.offer.insert(i, r);
            }
        }
        let choice = match phase {
            "PREP" => get(own, "personalChoice"),
            "SP_DRAFT" => get(public, "sp"),
            _ => &EMPTY,
        };
        optional_object(choice)?;
        if let Some(taken) = choice.get("taken") {
            object(taken)?;
        }
        for (i, row) in optional_array(choice, "cards")?.iter().enumerate() {
            if !truth(get(row, "soldOut")) && get(choice, "taken").get(i.to_string()).is_none() {
                let r = p.add(row.clone(), 5, self.row_key(row)?)?;
                p.choice.insert(i, r);
            }
        }
        for effect in optional_array(own, "effects")? {
            if get(effect, "iconKind").as_str() != Some("band") {
                let key = self.effect_key(effect)?;
                let mut row = serde_json::Map::new();
                row.insert("kind".into(), Value::String("effect".into()));
                row.insert("id".into(), Value::Null);
                row.insert("counter".into(), get(effect, "counter").clone());
                p.add(Value::Object(row), 7, key)?;
            }
        }
        if truth(get(own, "bandId")) {
            let id = text(get(own, "bandId"))?;
            let row = simple_row("band", id);
            p.add(row.clone(), 6, self.row_key(&row)?)?;
        }
        if phase == "BAND_DRAFT" {
            if self.bands.is_empty() {
                return Err("BAND_DRAFT requires ordered public band_ids metadata".into());
            }
            let picks = get(get(public, "draft"), "picks");
            if !picks.is_null() {
                object(picks)?;
            }
            for id in &self.bands {
                if picks
                    .as_object()
                    .is_some_and(|m| m.values().any(|v| v.as_str() == Some(id.as_str())))
                {
                    continue;
                }
                let row = simple_row("band", id);
                let r = p.add(row.clone(), 5, self.row_key(&row)?)?;
                p.bands.insert(id.clone(), r);
            }
        }
        for (i, entity) in p.entities.iter().enumerate() {
            if entity.area == 0 {
                let row = coord(field(&entity.row, "row")?)?.ok_or("Invalid public board row")?;
                let col =
                    coord(field(&entity.row, "col")?)?.ok_or("Invalid public board column")?;
                if cell(row, col).is_none() || p.occupied.insert((row, col), i + 1).is_some() {
                    return Err("Invalid or duplicate public board cell".into());
                }
            }
            if get(&entity.row, "kind").as_str() == Some("chess") && entity.area < 3 {
                let def = self.definition(&entity.definition)?;
                let counts = p.families.entry(def.base.clone()).or_insert([0; 4]);
                counts[if def.golden { 3 } else { entity.area }] += 1;
            }
        }
        if p.occupied.len() > 36 {
            return Err("Public board capacity exceeded".into());
        }
        Ok(p)
    }

    pub fn encode(&self, state: &Value, history: &Value, actions: &[Value]) -> Result<Encoded> {
        if actions.len() > MAX_ACTIONS {
            return Err("Public action capacity exceeded; no truncation".into());
        }
        let own = field(state, "own")?;
        let public = field(state, "public")?;
        let phase = string(state, "phase")?;
        if !PHASES.contains(&phase) {
            return Err("Unknown public phase".into());
        }
        if string(public, "modeId")? != "mode_single_abyss" {
            return Err("Only Desperation public policy is maintained".into());
        }
        let progress = field(history, "progress")?;
        if string(progress, "schema")? != "arkguardrl/public-observed-progress-v2" {
            return Err("Structured v7 requires observation-bound public progress".into());
        }
        // The Python boundary verifies observation/history fingerprints. No private reads.
        let p = self.project(state)?;
        let mut observation = vec![0_f64; OBS_DIM];
        let global = self.global(state, history)?;
        if global.len() != 42 {
            return Err("Invalid global feature shape".into());
        }
        observation[..42].copy_from_slice(&global);
        let exposures = field(progress, "pieces")?;
        object(exposures)?;
        for (i, e) in p.entities.iter().enumerate() {
            let row = &e.row;
            let def = self.definition(&e.definition)?;
            let out = &mut observation[42 + i * 28..42 + (i + 1) * 28];
            out[0] = def.index as f64;
            out[1] = 1.;
            out[2] = p.reference(get(row, "ownerUid"))? as f64;
            let items = optional_array(row, "items")?;
            if items.len() > 2 {
                return Err("Public equipment reference capacity exceeded".into());
            }
            for (j, item) in items.iter().enumerate() {
                out[3 + j] = p.reference(field(item, "uid")?)? as f64;
            }
            out[5 + e.area] = 1.;
            let exposure = exposures
                .get(progress_uid(get(row, "uid"))?)
                .unwrap_or(&EMPTY);
            optional_object(exposure)?;
            let counter = get(row, "counter");
            let numbers = [
                num(row, "price", 0.)?,
                num(row, "row", -1.)?,
                num(row, "col", -1.)?,
                num(row, "count", 0.)?,
                num(row, "coin", 0.)?,
                num(row, "count", 0.)?,
                num(row, "rounds", 0.)?,
                num(exposure, "visible_pre_refreshes", 0.)?,
                float_field(exposure, "history_complete", 0.)?,
                if truth(counter) { number(counter)? } else { 0. },
                bit(counter.is_number()),
            ];
            let scales = [30., 20., 21., 10., 10., 10., 10., 10., 1., 10., 1.];
            for j in 0..11 {
                out[13 + j] = if j == 8 || j == 10 {
                    numbers[j]
                } else {
                    scalar(numbers[j], scales[j])
                };
            }
            if let Some(direction) = DIRECTIONS
                .iter()
                .position(|d| get(row, "dir").as_str() == Some(d))
            {
                out[24 + direction] = 1.;
            }
        }
        let mut visible = HashMap::new();
        for bond in array(field(own, "bonds")?)? {
            let id = string(bond, "bondId")?;
            if !BONDS.contains(&id) {
                return Err("Unknown visible bond".into());
            }
            visible.insert(id, bond);
        }
        let disabled = array(field(public, "disabledBonds")?)?;
        for (i, key) in BONDS.iter().enumerate() {
            let b = visible.get(key).copied().unwrap_or(&EMPTY);
            let out = &mut observation[BOND_START + i * 5..BOND_START + (i + 1) * 5];
            out.copy_from_slice(&[
                scalar(num(b, "count", 0.)?, 12.),
                scalar(num(b, "layers", 0.)?, 100.),
                float_field(b, "active", 0.)?,
                scalar(num(b, "tier", 0.)?, 3.),
                if truth(get(b, "off")) {
                    float_value(get(b, "off"))?
                } else {
                    bit(disabled.iter().any(|v| v.as_str() == Some(key)))
                },
            ]);
        }
        let tiles = array(field(field(own, "deployment")?, "cells")?)?;
        if tiles.len() > 36 {
            return Err("Public deployment cell capacity exceeded".into());
        }
        for tile in tiles {
            let row = coord(field(tile, "row")?)?.ok_or("Invalid public deployment cell")?;
            let col = coord(field(tile, "col")?)?.ok_or("Invalid public deployment cell")?;
            let at = cell(row, col).ok_or("Invalid public deployment cell")?;
            observation[GRID_START + at * 5] = 1.;
            observation[GRID_START + at * 5 + 1] = bit(string(tile, "kind")? == "ranged");
        }
        let mut coverage = [0_u32; 36];
        let mut air_coverage = [0_u32; 36];
        for (&(row, col), &reference) in &p.occupied {
            let e = &p.entities[reference - 1];
            let def = self.definition(&e.definition)?;
            let direction = DIRECTIONS
                .iter()
                .position(|d| get(&e.row, "dir").as_str() == Some(d))
                .ok_or("Invalid public board direction")?;
            observation[GRID_START + cell(row, col).unwrap() * 5 + 2] = reference as f64;
            for &(dr, dc) in &def.ranges {
                let (r, c) = match direction {
                    0 => (dr, dc),
                    1 => (dc, -dr),
                    2 => (-dc, dr),
                    _ => (-dr, -dc),
                };
                if let Some(at) = cell(row + r, col + c) {
                    coverage[at] += 1;
                    air_coverage[at] += u32::from(def.air);
                }
            }
        }
        for at in 0..36 {
            observation[GRID_START + at * 5 + 3] = scalar(coverage[at] as f64, 6.);
            observation[GRID_START + at * 5 + 4] = scalar(air_coverage[at] as f64, 6.);
        }
        let enemies = array(field(own, "nextEnemies")?)?;
        if enemies.len() > MAX_ENEMIES {
            return Err("Public enemy capacity exceeded; no truncation".into());
        }
        for (i, enemy) in enemies.iter().enumerate() {
            let id = string(enemy, "enemyKey")?;
            let def = self.definition(&identity("enemy", id))?;
            let catalog = self
                .enemies
                .get(id)
                .ok_or("Unknown visible enemy identity")?;
            let out = &mut observation[ENEMY_START + i * 12..ENEMY_START + (i + 1) * 12];
            out[..8].copy_from_slice(&[
                def.index as f64,
                1.,
                *catalog as f64,
                scalar(num(enemy, "count", 0.)?, 10.),
                scalar(num(enemy, "t", 0.)?, 120.),
                float_field(enemy, "fly", 0.)?,
                float_field(enemy, "elite", 0.)?,
                float_field(enemy, "boss", 0.)?,
            ]);
            if let Some(gate) = ["lower", "upper", "left", "right"]
                .iter()
                .position(|key| get(enemy, "gate").as_str() == Some(key))
            {
                out[8 + gate] = 1.;
            }
        }
        let control = self.control(state, history, &p)?;
        if control.len() != 95 {
            return Err("Invalid control feature shape".into());
        }
        observation[CONTROL_START..].copy_from_slice(&control);
        let mut candidates = Vec::with_capacity(actions.len() * ACTION_DIM);
        let mut schedules = Vec::with_capacity(actions.len());
        let mut experts = Vec::with_capacity(actions.len());
        for action in actions {
            let (row, schedule, expert) = self.action(state, &p, action)?;
            if row.len() != ACTION_DIM {
                return Err("Invalid action feature shape".into());
            }
            candidates.extend(row.into_iter().map(|v| v as f32));
            schedules.push(schedule);
            experts.push(expert);
        }
        if observation.iter().any(|v| !v.is_finite()) || candidates.iter().any(|v| !v.is_finite()) {
            return Err("Nonfinite policy features".into());
        }
        // All nonlinear calculations and arithmetic above stay f64, matching Python.
        let observation: Vec<f32> = observation.into_iter().map(|v| v as f32).collect();
        if observation.iter().any(|v| !v.is_finite()) {
            return Err("Public observation exceeds finite f32 layout".into());
        }
        Ok(Encoded {
            observation,
            candidates,
            schedules,
            experts,
        })
    }

    fn global(&self, state: &Value, history: &Value) -> Result<Vec<f64>> {
        let own = field(state, "own")?;
        let public = field(state, "public")?;
        let shop = field(own, "shop")?;
        let progress = field(history, "progress")?;
        let round = number(field(state, "round")?)?;
        let mode_id = string(public, "modeId")?;
        let mode = field(field(&self.config, "modes")?, mode_id)?;
        let economy = mode
            .get("economy")
            .unwrap_or(field(&self.config, "economy")?);
        let level_value = field(shop, "level")?;
        let level = integer(level_value)?;
        let maximum = integer(field(mode, "maxShopLevel")?)?;
        let next = (level + 1).min(maximum);
        let slots = field(mode, "shopSlots")?;
        let current = field(slots, &level.to_string())?;
        let following = field(slots, &next.to_string())?;
        let scale = get(
            get(mode, "enemyScale"),
            &integer(field(state, "round")?)?.to_string(),
        );
        let hand = array(field(own, "hand")?)?;
        let temp = array(field(own, "temp")?)?;
        let mut out = vec![
            scalar(round, 20.),
            scalar(number(field(public, "lastRound")?)? - round, 20.),
            scalar(number(field(public, "bossRound")?)? - round, 20.),
            scalar(number(field(own, "lp")?)?, 30.),
            scalar(number(field(own, "funds")?)?, 30.),
            scalar(number(level_value)?, 6.),
            scalar(number(field(shop, "upgradePrice")?)?, 30.),
            scalar(number(field(shop, "refreshPrice")?)?, 10.),
            scalar(number(field(shop, "freeRefreshes")?)?, 10.),
            float_value(field(shop, "frozen")?)?,
            scalar(
                number(field(own, "deployCap")?)? - number(field(own, "deployCount")?)?,
                12.,
            ),
            scalar(hand.iter().filter(|v| v.is_null()).count() as f64, 10.),
            scalar(temp.iter().filter(|v| !v.is_null()).count() as f64, 5.),
            scalar(number(field(progress, "spent")?)?, 30.),
            float_value(field(progress, "spent_known")?)?,
            scalar(number(field(progress, "refreshes")?)?, 10.),
            float_value(field(progress, "refreshes_known")?)?,
            scalar(number(field(progress, "direct_chess_gains")?)?, 10.),
            float_value(field(progress, "direct_gains_complete")?)?,
        ];
        for value in [
            maximum as f64,
            next as f64,
            number(field(current, "chess")?)?,
            number(field(following, "chess")?)?,
            number(field(current, "item")?)?,
            number(field(following, "item")?)?,
        ] {
            out.push(scalar(value, 6.));
        }
        let carry = array(field(economy, "leftoverFundsKeptByBands")?)?
            .iter()
            .any(|v| python_equal(v, get(own, "bandId")));
        out.extend([
            bit(carry),
            1.,
            scalar(num(scale, "hp", 1.)?, 3.),
            scalar(num(scale, "atk", 1.)?, 3.),
            scalar(num(scale, "speed", 1.)?, 3.),
        ]);
        flags(&mut out, field(state, "phase")?, &PHASES);
        Ok(out)
    }

    fn control(&self, state: &Value, history: &Value, p: &Projection) -> Result<Vec<f64>> {
        let report = field(field(history, "progress")?, "last_execution")?;
        optional_object(report)?;
        let mut refs = Vec::with_capacity(3);
        let mut definitions = Vec::with_capacity(3);
        for role in ["source", "target", "replaced"] {
            let e = optional_object(get(report, role))?;
            refs.push(p.receipt_reference(get(e, "uid"))? as f64);
            let kind = get(e, "kind").as_str().unwrap_or("");
            let definition = if ["chess", "item", "token", "band"].contains(&kind) {
                let id = field(e, "id")?;
                id.as_str()
                    .and_then(|id| self.entities.get(&identity(kind, id)))
                    .copied()
                    .unwrap_or(0)
            } else {
                0
            };
            definitions.push(definition as f64);
        }
        let mut out = refs;
        out.extend(definitions);
        // history used/limit must be valid even outside PREP, as in project().
        let used = number(field(history, "used")?)?;
        let limit = number(field(history, "limit")?)?;
        if string(state, "phase")? == "PREP" {
            if limit == 0. {
                return Err("Zero public preparation budget".into());
            }
            out.extend([1., ((limit - used) / limit).max(0.)]);
        } else {
            out.extend([0., 0.]);
        }
        out.extend([
            bit(!report.is_null()),
            bit(truth(report) && truth(field(report, "accepted")?)),
            bit(truth(report) && python_equal(field(report, "round")?, field(state, "round")?)),
        ]);
        flags(&mut out, get(report, "action_type"), &ACTIONS);
        flags(&mut out, get(report, "error_class"), &ERRORS);
        let changes = optional_object(get(report, "changes"))?;
        for (i, key) in CHANGES.iter().enumerate() {
            out.push(scalar(
                num(changes, key, 0.)?,
                if i < 2 { 30. } else { 10. },
            ));
        }
        let deltas = optional_object(get(report, "bond_layer_deltas"))?;
        for key in BONDS {
            out.push(scalar(num(deltas, key, 0.)?, 100.));
        }
        let intent = optional_object(get(report, "intent"))?;
        let to = optional_object(get(intent, "to"))?;
        flags(&mut out, get(intent, "dir"), &DIRECTIONS);
        flags(&mut out, get(to, "area"), &["board", "hand"]);
        let values = [
            to.get("row").unwrap_or(get(intent, "row")),
            to.get("col").unwrap_or(get(intent, "col")),
            get(to, "idx"),
            get(intent, "slot"),
            get(intent, "idx"),
            get(get(report, "source"), "row"),
            get(get(report, "source"), "col"),
            get(get(report, "target"), "row"),
            get(get(report, "target"), "col"),
        ];
        for value in values {
            out.extend([
                if value.is_null() {
                    0.
                } else {
                    scalar(number(value)?, 21.)
                },
                bit(!value.is_null()),
            ]);
        }
        Ok(out)
    }

    fn action(
        &self,
        state: &Value,
        p: &Projection,
        action: &Value,
    ) -> Result<(Vec<f64>, i64, i64)> {
        let t = string(action, "t")?;
        let kind = t.get(2..).ok_or("Invalid native action type")?;
        if !ACTIONS.contains(&kind) {
            return Err("Unknown native action feature identity".into());
        }
        let own = field(state, "own")?;
        let shop = field(own, "shop")?;
        let has_subject = ["buy", "reward", "choice", "band"].contains(&kind);
        // Subject actions project a catalog identity instead of an instance UID.
        let mut source = if has_subject {
            0
        } else {
            p.reference(action.get("uid").unwrap_or(get(action, "itemUid")))?
        };
        let mut target = p.reference(get(action, "targetUid"))?;
        let replaced = p.reference(get(action, "replaceUid"))?;
        let mut subject = 0;
        let mut destination = serde_json::Map::new();
        let mut cost = 0.;
        let mut semantic = kind;
        if kind == "move" {
            if source == 0 {
                return Err("Move requires known public source UID".into());
            }
            let to = field(action, "to")?;
            destination = object(to)?.clone();
            let area = string(to, "area")?;
            let location = &p.entities[source - 1];
            if area == "board" {
                let row = coord(field(to, "row")?)?;
                let col = coord(field(to, "col")?)?;
                target = row
                    .zip(col)
                    .and_then(|cell| p.occupied.get(&cell).copied())
                    .unwrap_or(0);
                semantic = if location.area == 0 {
                    if target != 0 && target != source {
                        "board_swap"
                    } else {
                        "reposition"
                    }
                } else if target != 0 {
                    "reserve_replace"
                } else {
                    "deploy"
                };
            } else if area == "hand" {
                let slot = integer(field(to, "idx")?)?;
                let hand = array(field(own, "hand")?)?;
                let occupant = usize::try_from(slot)
                    .ok()
                    .and_then(|i| hand.get(i))
                    .unwrap_or(&EMPTY);
                target = if occupant.is_null() {
                    0
                } else {
                    p.reference(field(occupant, "uid")?)?
                };
                semantic = if location.area == 0 {
                    "withdraw"
                } else {
                    "reserve_move"
                };
            } else {
                return Err("Unknown public move destination area".into());
            }
        } else {
            for key in ["row", "col"] {
                if let Some(v) = action.get(key) {
                    destination.insert(key.to_owned(), v.clone());
                }
            }
            if ["buy", "reward", "choice"].contains(&kind) {
                let key = if kind == "buy" { "slot" } else { "idx" };
                let slot = integer(field(action, key)?)?;
                let slots = if kind == "buy" {
                    field(shop, "slots")?
                } else if kind == "reward" {
                    get(get(shop, "rewardOffer"), "slots")
                } else {
                    let choice = if string(state, "phase")? == "PREP" {
                        get(own, "personalChoice")
                    } else {
                        get(field(state, "public")?, "sp")
                    };
                    get(choice, "cards")
                };
                let row = if slots.is_null() {
                    &EMPTY
                } else {
                    usize::try_from(slot)
                        .ok()
                        .and_then(|i| slots.as_array().and_then(|a| a.get(i)))
                        .unwrap_or(&EMPTY)
                };
                cost = if row.is_null() {
                    0.
                } else {
                    num(row, "price", 0.)?
                };
                source = 0; // purchase/choice descriptor has no instance UID
                let map = if kind == "buy" {
                    &p.shop
                } else if kind == "reward" {
                    &p.offer
                } else {
                    &p.choice
                };
                subject = usize::try_from(slot)
                    .ok()
                    .and_then(|i| map.get(&i))
                    .copied()
                    .ok_or("Unknown public offer reference")?;
            } else if kind == "band" {
                source = 0;
                subject = *p
                    .bands
                    .get(string(action, "bandId")?)
                    .ok_or("Unknown public band offer reference")?;
            } else if kind == "levelUp" {
                cost = number(field(shop, "upgradePrice")?)?;
            } else if kind == "refresh" {
                cost = if number(field(shop, "freeRefreshes")?)? > 0. {
                    0.
                } else {
                    number(field(shop, "refreshPrice")?)?
                };
            }
        }
        let row = if subject > 0 {
            &p.entities[subject - 1].row
        } else {
            &EMPTY
        };
        let subject_entity = if subject > 0 {
            Some(&p.entities[subject - 1])
        } else if source > 0 {
            Some(&p.entities[source - 1])
        } else {
            None
        };
        let mut counts = [0; 4];
        let mut needed = 0.;
        if let Some(entity) = subject_entity {
            let def = self.definition(&entity.definition)?;
            // Python uses rec={} for non-chess; family lookup then uses row.id.
            let base = if get(&entity.row, "kind").as_str() == Some("chess") {
                needed = def.upgrade;
                Some(def.base.as_str())
            } else {
                get(&entity.row, "id").as_str()
            };
            if let Some(base) = base {
                counts = p.families.get(base).copied().unwrap_or([0; 4]);
            }
        }
        let to = Value::Object(destination);
        let row_value = to.get("row").unwrap_or(get(action, "row"));
        let col_value = to.get("col").unwrap_or(get(action, "col"));
        let destination = if row_value.is_null() || col_value.is_null() {
            0
        } else {
            // Python dict lookup accepts numerically equal integer/float cells.
            let r = number(row_value)?;
            let c = number(col_value)?;
            if r.fract() == 0. && c.fract() == 0. {
                cell(r as i64, c as i64).map(|i| i + 1).unwrap_or(0)
            } else {
                0
            }
        };
        let mut out = vec![
            source as f64,
            target as f64,
            replaced as f64,
            subject as f64,
            destination as f64,
        ];
        flags(&mut out, get(action, "dir"), &DIRECTIONS);
        flags(&mut out, &Value::String(kind.to_owned()), &ACTIONS);
        flags(&mut out, &Value::String(semantic.to_owned()), &MOVES);
        out.extend([
            scalar(cost, 30.),
            scalar(num(row, "coin", 0.)?, 10.),
            scalar(num(row, "count", 0.)?, 10.),
            scalar(num(row, "rounds", 0.)?, 10.),
        ]);
        out.extend(counts.into_iter().map(|n| scalar(n as f64, 3.)));
        out.extend([
            scalar(needed, 3.),
            scalar(
                (needed - (counts[0] + counts[1] + counts[2]) as f64).max(0.),
                3.,
            ),
            bit(get(&to, "area").as_str() == Some("hand")),
            scalar(num(&to, "idx", -1.)?, 10.),
        ]);
        let schedule = if kind == "move" {
            match semantic {
                "deploy" => "deploy",
                "reserve_replace" => "replace",
                "withdraw" => "withdraw",
                "reposition" | "board_swap" | "reserve_move" => "adjust",
                _ => return Err("Unknown public movement intent".into()),
            }
        } else if kind == "infoReady" {
            "confirm"
        } else {
            kind
        };
        let schedule = SCHEDULES
            .iter()
            .position(|s| *s == schedule)
            .ok_or("Unknown explicit native decision intent")? as i64;
        let expert = i64::from(
            ![
                "buy", "refresh", "freeze", "levelUp", "band", "choice", "reward",
            ]
            .contains(&kind),
        );
        Ok((out, schedule, expert))
    }
}

fn simple_row(kind: &str, id: &str) -> Value {
    let mut row = serde_json::Map::new();
    row.insert("kind".into(), Value::String(kind.to_owned()));
    row.insert("id".into(), Value::String(id.to_owned()));
    Value::Object(row)
}
// Python public choice comparison treats 1 and 1.0 (and booleans) as equal.
fn python_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => a.as_f64() == b.as_f64(),
        (Value::Bool(x), Value::Number(_)) | (Value::Number(_), Value::Bool(x)) => {
            let n = if a.is_number() { a } else { b };
            n.as_f64() == Some(bit(*x))
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| python_equal(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|other| python_equal(v, other)))
        }
        _ => a == b,
    }
}
