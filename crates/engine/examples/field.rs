//! Development field runner. Input uses original stage/routes/spawns shapes;
//! Explicitly supported default kits only; not a Gymnasium env or benchmark.
use arkguard_engine::{
    battle::{
        direction::Direction,
        health::Pool,
        operators::Placement,
        runtime::{BossConfig, BossKind, BossOvertime, BossRoundConfig, Field, Limits},
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
        return Err("usage: field DATA_ROOT INPUT.json TICK_BUDGET".into());
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
            "kind",
            "map",
            "bossPool",
            "bossRound",
            "branches",
            "devices",
            "extraRoutes",
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
            || !players[0]["units"].is_array()
            || players[0]
                .as_object()
                .unwrap()
                .keys()
                .any(|k| !matches!(k.as_str(), "playerId" | "units"))
        {
            return Err("Only the solo player {playerId: p, units: [...]} is implemented in this field entry".into());
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
    let kind = input
        .get("kind")
        .map(|v| v.as_str().ok_or("Expected field kind string"))
        .transpose()?
        .unwrap_or("normal");
    let time_limit =
        input["timeLimit"]
            .as_f64()
            .unwrap_or(if kind == "normal" { 60. } else { f64::INFINITY });
    let mut field = match kind {
        "normal" => {
            if input.get("bossPool").is_some() {
                return Err("bossPool requires boss or hidden kind".into());
            }
            Field::new(Arc::clone(&data), stage, template, seed, time_limit, limits)?
        }
        "boss" | "hidden" => {
            let pool = input["bossPool"]
                .as_object()
                .ok_or("Specify bossPool for boss or hidden kind")?;
            if pool
                .keys()
                .any(|key| !matches!(key.as_str(), "hp" | "maxHp"))
            {
                return Err("Unsupported bossPool input".into());
            }
            let max_hp = input["bossPool"]["maxHp"]
                .as_f64()
                .ok_or("Expected bossPool.maxHp number")?;
            let hp = pool
                .get("hp")
                .map(|v| v.as_f64().ok_or("Expected bossPool.hp number"))
                .transpose()?
                .unwrap_or(max_hp);
            Field::new_boss(
                Arc::clone(&data),
                stage,
                template,
                seed,
                time_limit,
                limits,
                BossConfig {
                    kind: if kind == "boss" {
                        BossKind::FinalAssault
                    } else {
                        BossKind::HiddenCore
                    },
                    pool: Pool { hp, max_hp },
                },
            )?
        }
        _ => return Err(format!("Unsupported field kind: {kind}")),
    };
    field.configure_map(arkguard_engine::battle::runtime::MapConfig::parse(
        &input["map"],
    )?)?;
    if let Some(value) = input.get("bossRound") {
        let object = value.as_object().ok_or("Expected bossRound object")?;
        if object
            .keys()
            .any(|k| !matches!(k.as_str(), "lp" | "overtime"))
        {
            return Err("Unsupported bossRound input".into());
        }
        let life_points = value["lp"].as_f64().ok_or("Expected bossRound.lp number")?;
        let overtime = if let Some(o) = value.get("overtime") {
            let object = o.as_object().ok_or("Expected overtime object")?;
            if object
                .keys()
                .any(|k| !matches!(k.as_str(), "afterReal" | "combatTimeScale" | "lpPerSecond"))
            {
                return Err("Unsupported overtime input".into());
            }
            Some(BossOvertime {
                after_real_seconds: o["afterReal"]
                    .as_f64()
                    .ok_or("Expected overtime.afterReal number")?,
                combat_time_scale: o["combatTimeScale"]
                    .as_f64()
                    .ok_or("Expected overtime.combatTimeScale number")?,
                lp_per_real_second: o["lpPerSecond"]
                    .as_f64()
                    .ok_or("Expected overtime.lpPerSecond number")?,
            })
        } else {
            None
        };
        field.configure_boss_round(BossRoundConfig {
            life_points,
            overtime,
        })?;
    }
    for u in input["players"][0]["units"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let object = u.as_object().ok_or("Expected operator object")?;
        if object.keys().any(|k| {
            !matches!(
                k.as_str(),
                "chessId" | "uid" | "row" | "col" | "dir" | "skillIndex"
            )
        }) {
            return Err(
                "Only supported skills with default modules and no items are implemented".into(),
            );
        }
        let direction = u
            .get("dir")
            .map(|v| v.as_str().ok_or("Expected operator direction string"))
            .transpose()?
            .unwrap_or("RIGHT");
        let dir = match direction {
            "RIGHT" => Direction::Right,
            "LEFT" => Direction::Left,
            "UP" => Direction::Up,
            "DOWN" => Direction::Down,
            _ => return Err("Invalid operator direction".into()),
        };
        let coordinate = |k: &str| -> Result<i32, String> {
            i32::try_from(
                u[k].as_i64()
                    .ok_or("Expected integer operator coordinate")?,
            )
            .map_err(|_| "Operator coordinate outside i32".into())
        };
        field.add_operator_with_skill(
            u["chessId"].as_str().ok_or("Expected chessId")?,
            Placement {
                uid: u["uid"]
                    .as_u64()
                    .ok_or("Expected nonnegative operator uid")?,
                home: [coordinate("row")?, coordinate("col")?],
                direction: dir,
            },
            u.get("skillIndex")
                .map(|v| {
                    v.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or("Expected nonnegative u32 skillIndex")
                })
                .transpose()?,
        )?;
    }
    let advance = field.run(budget)?;
    let output = serde_json::json!({"ticks":advance.ticks,"terminated":advance.terminated,"budget_exhausted":advance.budget_exhausted,"report":field.report()?,"allies":field.ally_states()?});
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| e.to_string())?
    );
    Ok(())
}
