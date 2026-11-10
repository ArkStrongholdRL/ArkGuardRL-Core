//! Development runner for seeded modes or prepared single-player round plans.
use arkguard_engine::{
    battle::{
        direction::Direction,
        operators::Placement,
        runtime::{BossOvertime, Limits},
        waves::Template,
    },
    data::Catalog,
    solo::{
        preparation::{Action, Config as PrepConfig, EffectsConfig, Grant, Kind},
        Boss, Config, HiddenRule, Operator, Phase, PreparedRound, RoundKind, RoundPlan, Runner,
    },
};
use serde_json::{json, Value};
use std::{io::Read, path::Path, sync::Arc};

enum PreparationInput {
    Supplied {
        lineup: PreparedRound,
        spend: u64,
    },
    Owned {
        actions: Vec<Action>,
        layers: f64,
        spend: u64,
    },
}
fn owned_preparation(v: &Value) -> Result<PrepConfig, String> {
    keys(v, &["modeId", "banned", "initial", "capacity", "effects"])?;
    let banned = v
        .get("banned")
        .map(|v| {
            v.as_array()
                .ok_or("Expected banned array")?
                .iter()
                .map(|v| v.as_str().map(str::to_owned).ok_or("Expected banned id"))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let initial = v
        .get("initial")
        .map(|v| {
            v.as_array()
                .ok_or("Expected initial grant array")?
                .iter()
                .map(|v| {
                    keys(v, &["kind", "id"])?;
                    Ok(Grant {
                        kind: match v["kind"].as_str() {
                            Some("chess") => Kind::Chess,
                            Some("item") => Kind::Item,
                            _ => return Err("Invalid initial grant kind".into()),
                        },
                        key: v["id"].as_str().ok_or("Expected grant id")?.into(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    let effects = v
        .get("effects")
        .map(|v| {
            keys(v, &["disabledBonds", "layers", "countBonus"])?;
            Ok::<_, String>(EffectsConfig {
                disabled_bonds: serde_json::from_value(
                    v.get("disabledBonds").cloned().unwrap_or_else(|| json!([])),
                )
                .map_err(|e| e.to_string())?,
                layers: serde_json::from_value(
                    v.get("layers").cloned().unwrap_or_else(|| json!({})),
                )
                .map_err(|e| e.to_string())?,
                count_bonus: serde_json::from_value(
                    v.get("countBonus").cloned().unwrap_or_else(|| json!({})),
                )
                .map_err(|e| e.to_string())?,
            })
        })
        .transpose()?;
    Ok(PrepConfig {
        effects,
        mode_id: v["modeId"]
            .as_str()
            .ok_or("Expected preparation.modeId")?
            .into(),
        banned,
        initial,
        capacity: v
            .get("capacity")
            .map(|v| {
                v.as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or("Expected preparation capacity")
            })
            .transpose()?
            .unwrap_or(256),
    })
}

fn keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("Expected an input object")?;
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported single-player input: {key}"));
    }
    Ok(())
}
fn overtime(value: &Value) -> Result<Option<BossOvertime>, String> {
    if value.is_null() {
        return Ok(None);
    }
    keys(value, &["afterReal", "combatTimeScale", "lpPerSecond"])?;
    Ok(Some(BossOvertime {
        after_real_seconds: value["afterReal"]
            .as_f64()
            .ok_or("Expected overtime.afterReal")?,
        combat_time_scale: value["combatTimeScale"]
            .as_f64()
            .ok_or("Expected overtime.combatTimeScale")?,
        lp_per_real_second: value["lpPerSecond"]
            .as_f64()
            .ok_or("Expected overtime.lpPerSecond")?,
    }))
}
fn prepared(value: &Value) -> Result<PreparedRound, String> {
    keys(value, &["units", "activeLayers"])?;
    let units = value["units"].as_array().ok_or("Expected prepared.units")?;
    if units.len() > 36 {
        return Err("Prepared lineup exceeds board capacity".into());
    }
    let operators = units
        .iter()
        .map(|u| {
            keys(
                u,
                &["chessId", "uid", "row", "col", "dir", "skillIndex", "items"],
            )?;
            let direction = match u
                .get("dir")
                .filter(|v| !v.is_null())
                .map(|v| v.as_str().ok_or("Expected prepared facing string"))
                .transpose()?
                .unwrap_or("RIGHT")
            {
                "RIGHT" => Direction::Right,
                "UP" => Direction::Up,
                "DOWN" => Direction::Down,
                "LEFT" => Direction::Left,
                _ => return Err("Invalid prepared facing".into()),
            };
            let coordinate = |key| {
                u[key]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("Expected board coordinate")
            };
            Ok(Operator {
                items: serde_json::from_value(u.get("items").cloned().unwrap_or_else(|| json!([])))
                    .map_err(|e| e.to_string())?,
                key: u["chessId"].as_str().ok_or("Expected chessId")?.into(),
                placement: Placement {
                    uid: u["uid"].as_u64().ok_or("Expected operator uid")?,
                    home: [coordinate("row")?, coordinate("col")?],
                    direction,
                },
                skill_index: u
                    .get("skillIndex")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_u64()
                            .and_then(|v| u32::try_from(v).ok())
                            .ok_or("Expected skillIndex")
                    })
                    .transpose()?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(PreparedRound {
        operators,
        active_layers: value["activeLayers"]
            .as_f64()
            .ok_or("Expected prepared.activeLayers")?,
    })
}
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: solo DATA_ROOT INPUT.json TICK_BUDGET".into());
    }
    let data = Arc::new(Catalog::load(Path::new(&args[0]), 64 << 20)?);
    let mut bytes = Vec::new();
    std::fs::File::open(&args[1])
        .map_err(|e| e.to_string())?
        .take((4 << 20) + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 << 20 {
        return Err("Single-player input exceeds byte limit".into());
    }
    let mut input: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let setup = if input.get("modeId").is_some() {
        keys(
            &input,
            &[
                "seed",
                "modeId",
                "preparation",
                "roundActions",
                "map",
                "drafts",
                "core",
            ],
        )?;
        let mode = input["modeId"].as_str().ok_or("Expected modeId")?;
        let seed = input["seed"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("Expected seed in u32 domain")?;
        let mut prep = input
            .get("preparation")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !prep.is_object() {
            return Err("Expected preparation object".into());
        }
        if prep.get("modeId").is_some_and(|v| v != mode) {
            return Err("Conflicting preparation modeId".into());
        }
        prep["modeId"] = json!(mode);
        let options = owned_preparation(&prep)?;
        let mut generated =
            arkguard_engine::solo::mode::generate(&data, seed, mode, &options.banned)?;
        let mut effects = prep.get("effects").cloned().unwrap_or_else(|| json!({}));
        let mut disabled: Vec<String> = serde_json::from_value(
            generated.input["preparation"]["effects"]["disabledBonds"].clone(),
        )
        .map_err(|e| e.to_string())?;
        if let Some(e) = options.effects {
            disabled.extend(e.disabled_bonds);
        }
        disabled.sort();
        disabled.dedup();
        effects["disabledBonds"] = json!(disabled);
        prep["effects"] = effects;
        prep["banned"] = generated.input["preparation"]["banned"].clone();
        generated.input["preparation"] = prep;
        if let Some(actions) = input.get("roundActions") {
            let actions = actions.as_array().ok_or("Expected roundActions array")?;
            let plans = generated.input["plans"].as_array_mut().unwrap();
            if actions.len() > plans.len() {
                return Err("Too many roundActions".into());
            }
            for (plan, actions) in plans.iter_mut().zip(actions) {
                if !actions.is_array() {
                    return Err("Expected round action array".into());
                }
                plan["actions"] = actions.clone();
            }
        }
        if let Some(map) = input.get("map") {
            generated.input["map"] = map.clone();
        }
        if let Some(drafts) = input.get("drafts") {
            if drafts.as_bool().ok_or("Expected drafts boolean")? {
                generated.input["drafts"] = generated.setup["stageId"].clone();
            }
        }
        if let Some(core) = input.get("core") {
            generated.input["core"] = core.clone();
        }
        input = generated.input;
        Some(generated.setup)
    } else {
        None
    };
    keys(
        &input,
        &[
            "seed",
            "stage",
            "config",
            "plans",
            "preparation",
            "map",
            "drafts",
            "core",
        ],
    )?;
    let owned = input
        .get("preparation")
        .map(owned_preparation)
        .transpose()?;
    let seed = input["seed"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("Expected seed in u32 domain")?;
    let budget: usize = args[2].parse().map_err(|_| "Invalid tick budget")?;
    let cfg = &input["config"];
    keys(cfg, &["lp", "lpCap", "keepFunds", "hidden"])?;
    let hidden = if cfg["hidden"].is_null() {
        None
    } else {
        let h = &cfg["hidden"];
        keys(h, &["layerThreshold", "minLp"])?;
        Some(HiddenRule {
            layer_threshold: h["layerThreshold"]
                .as_f64()
                .ok_or("Expected hidden.layerThreshold")?,
            min_lp_exclusive: h["minLp"].as_f64().ok_or("Expected hidden.minLp")?,
        })
    };
    let config = Config {
        initial_lp: cfg["lp"].as_f64().ok_or("Expected config.lp")?,
        lp_cap_per_round: cfg["lpCap"].as_u64().ok_or("Expected config.lpCap")?,
        keep_funds: cfg["keepFunds"]
            .as_bool()
            .ok_or("Expected config.keepFunds")?,
        hidden,
    };
    let raw = input["plans"].as_array().ok_or("Expected plans array")?;
    if raw.is_empty() || raw.len() > 64 {
        return Err("Single-player round capacity exceeded".into());
    }
    let limits = Limits::default();
    let mut preps = Vec::new();
    let plans = raw
        .iter()
        .map(|p| {
            keys(
                p,
                &[
                    "kind",
                    "timeLimit",
                    "income",
                    "deployCap",
                    "routes",
                    "spawns",
                    "extraRoutes",
                    "templateId",
                    "spawnActions",
                    "branches",
                    "devices",
                    "overrides",
                    "maxHp",
                    "overtime",
                    "prepared",
                    "spend",
                    "actions",
                ],
            )?;
            let boss = || {
                Ok::<_, String>(Boss {
                    max_hp: p["maxHp"].as_f64().ok_or("Expected maxHp")?,
                    overtime: overtime(&p["overtime"])?,
                })
            };
            let kind = match p["kind"].as_str().ok_or("Expected round kind")? {
                "normal" => {
                    if !p["overtime"].is_null() {
                        return Err("Normal rounds use their combat time limit".into());
                    }
                    RoundKind::Normal
                }
                "boss" => RoundKind::FinalAssault(boss()?),
                "hidden" => RoundKind::HiddenCore(boss()?),
                _ => return Err("Invalid single-player round kind".into()),
            };
            let mut wave = Template::parse(
                p,
                &Value::Null,
                &Value::Null,
                limits.units,
                limits.route_legs,
            )?;
            if let Some(id) = p.get("templateId").filter(|v| !v.is_null()) {
                let id = id.as_str().ok_or("Expected templateId")?;
                let original = &data.table("waves").unwrap()[id];
                if !original.is_object() {
                    return Err("Unknown wave template".into());
                }
                let actions = p["spawnActions"]
                    .as_array()
                    .ok_or("Expected resolved spawnActions")?;
                wave.configure_bounty_actions(
                    original,
                    actions,
                    &data.table("factions").unwrap()["templateSlots"],
                    limits.units,
                )?;
            } else if p.get("spawnActions").is_some() {
                return Err("spawnActions require templateId".into());
            }
            wave.spawns = p["spawns"]
                .as_array()
                .ok_or("Expected round spawns")?
                .clone();
            let spend = p
                .get("spend")
                .map(|v| v.as_u64().ok_or("Expected preparation spend"))
                .transpose()?
                .unwrap_or(0);
            let preparation = if owned.is_some() {
                if p.get("prepared").is_some() {
                    keys(&p["prepared"], &["activeLayers"])?;
                }
                PreparationInput::Owned {
                    actions: p
                        .get("actions")
                        .map(|v| {
                            v.as_array()
                                .ok_or("Expected action array")?
                                .iter()
                                .map(Action::parse)
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .transpose()?
                        .unwrap_or_default(),
                    layers: p["prepared"]
                        .get("activeLayers")
                        .map(|v| v.as_f64().ok_or("Expected activeLayers"))
                        .transpose()?
                        .unwrap_or(0.),
                    spend,
                }
            } else {
                if p.get("actions").is_some() {
                    return Err("Preparation actions require preparation configuration".into());
                }
                PreparationInput::Supplied {
                    lineup: prepared(&p["prepared"])?,
                    spend,
                }
            };
            preps.push(preparation);
            Ok(RoundPlan {
                kind,
                wave,
                time_limit: p["timeLimit"]
                    .as_f64()
                    .or_else(|| {
                        (p["timeLimit"].is_null() && p["kind"] != "normal").then_some(f64::INFINITY)
                    })
                    .ok_or("Expected timeLimit")?,
                income: p["income"].as_u64().ok_or("Expected round income")?,
                deploy_cap: p["deployCap"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or("Expected deployCap")?,
            })
        })
        .collect::<Result<_, String>>()?;
    let effects = owned.as_ref().is_some_and(|p| p.effects.is_some());
    let mut run = if let Some(owned) = owned {
        Runner::new_with_preparation(
            data,
            input["stage"].clone(),
            seed,
            config,
            plans,
            limits,
            owned,
        )?
    } else {
        Runner::new(data, input["stage"].clone(), seed, config, plans, limits)?
    };
    run.configure_map(arkguard_engine::battle::runtime::MapConfig::parse(
        &input["map"],
    )?)?;
    if let Some(stage) = input.get("drafts") {
        run.enable_drafts(stage.as_str().ok_or("Expected draft stage ID")?)?;
    }
    if let Some(core) = input.get("core") {
        run.configure_core(core.as_str().ok_or("Expected core ID")?)?;
    }
    let mut preps = preps.into_iter().peekable();
    let mut ticks = 0;
    while ticks < budget && run.phase() != Phase::Result {
        if run.phase() == Phase::Draft {
            let Some(PreparationInput::Owned { actions, .. }) = preps.peek_mut() else {
                break;
            };
            if !matches!(actions.first(), Some(Action::Draft(_))) {
                break;
            }
            run.prep_action(actions.remove(0))?;
            continue;
        }
        if run.phase() == Phase::Prep {
            match preps.next().ok_or("Missing prepared round")? {
                PreparationInput::Supplied { lineup, spend } => {
                    run.spend(spend)?;
                    run.ready(lineup)?;
                }
                PreparationInput::Owned {
                    actions,
                    layers,
                    spend,
                } => {
                    run.spend(spend)?;
                    for action in actions {
                        run.prep_action(action)?;
                    }
                    if effects {
                        if layers != 0. {
                            return Err(
                                "Effects derive active layers from owned preparation".into()
                            );
                        }
                        run.ready_current()?;
                    } else {
                        run.ready_owned(layers)?;
                    }
                }
            }
        }
        ticks += run.advance(budget - ticks)?.ticks;
    }
    let mut report = json!({"ticks":ticks,"budgetExhausted":ticks==budget&&run.phase()!=Phase::Result,"state":run.report()?});
    if let Some(setup) = setup {
        report["setup"] = setup;
    }
    println!("{report}");
    Ok(())
}
