//! Development field runner. Input uses original stage/routes/spawns shapes;
//! empty allied roster only. This is not a Gymnasium environment or benchmark.
use arkguard_engine::{
    battle::{
        runtime::{Field, Limits},
        waves::Template,
    },
    data::Catalog,
};
use serde_json::Value;
use std::io::Read;
use std::{path::Path, sync::Arc};
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(
            "usage: field DATA_ROOT INPUT.json TICK_BUDGET (empty allied roster only)".into(),
        );
    }
    let data = Arc::new(Catalog::load(Path::new(&args[0]), 64 << 20)?);
    let mut text = Vec::new();
    std::fs::File::open(&args[1])
        .map_err(|e| e.to_string())?
        .take((4 << 20) + 1)
        .read_to_end(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 4 << 20 {
        return Err("Field input exceeds byte limit".into());
    }
    let input: Value = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    let object = input.as_object().ok_or("Expected field input object")?;
    for key in object.keys() {
        if ![
            "stage",
            "stageId",
            "routes",
            "spawns",
            "overrides",
            "seed",
            "timeLimit",
            "players",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unsupported field input: {key}"));
        }
    }
    let budget: usize = args[2].parse().map_err(|_| "Invalid tick budget")?;
    let limits = Limits::default();
    let stage = input
        .get("stage")
        .or_else(|| {
            input["stageId"]
                .as_str()
                .and_then(|key| data.table("stages")?.get(key))
        })
        .ok_or("Specify stage or stageId")?;
    if let Some(players) = input.get("players") {
        let players = players.as_array().ok_or("Expected player array")?;
        if players.len() != 1
            || players[0]["playerId"] != "p"
            || !players[0]["units"]
                .as_array()
                .map(|u| u.is_empty())
                .unwrap_or(false)
            || players[0]
                .as_object()
                .unwrap()
                .keys()
                .any(|k| !matches!(k.as_str(), "playerId" | "units"))
        {
            return Err("Only the empty solo player {playerId: p, units: []} is implemented in this field entry".into());
        }
    }
    let mut template = Template::parse(
        &input,
        &Value::Null,
        &Value::Null,
        limits.units,
        limits.route_legs,
    )?;
    if let Some(spawns) = input["spawns"].as_array() {
        template.spawns = spawns.clone();
    }
    let seed = input
        .get("seed")
        .map(|v| v.as_u64().ok_or("Expected nonnegative integer seed"))
        .transpose()?
        .map(u32::try_from)
        .transpose()
        .map_err(|_| "Seed outside u32")?
        .unwrap_or(1);
    let mut field = Field::new(
        Arc::clone(&data),
        stage,
        template,
        seed,
        input["timeLimit"].as_f64().unwrap_or(60.),
        limits,
    )?;
    let advance = field.run(budget)?;
    let output = serde_json::json!({"ticks":advance.ticks,"terminated":advance.terminated,"budget_exhausted":advance.budget_exhausted,"report":field.report()?});
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}
