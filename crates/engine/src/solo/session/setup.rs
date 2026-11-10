// SPDX-License-Identifier: GPL-3.0-or-later
//! Convert the generated, private mode plan into a live runner once per reset.
use super::*;
use crate::battle::{runtime::BossOvertime, waves::Template};
use crate::solo::{preparation, Boss, Config, HiddenRule, RoundKind, RoundPlan};

pub(super) fn start(
    data: Arc<Catalog>,
    seed: u32,
    core: &str,
    banned: &[String],
    setup_revision: u32,
) -> Result<(Runner, Value), String> {
    crate::battle::runtime::Field::validate_core(&data, core)?;
    let generated = super::super::mode::generate_at_revision(
        &data,
        seed,
        "mode_single_abyss",
        banned,
        setup_revision,
    )?;
    let input = generated.input;
    let cfg = &input["config"];
    let number = |v: &Value| {
        v.as_f64()
            .ok_or_else(|| "Invalid generated number".to_owned())
    };
    let integer = |v: &Value| {
        v.as_u64()
            .ok_or_else(|| "Invalid generated integer".to_owned())
    };
    let config = Config {
        initial_lp: number(&cfg["lp"])?,
        lp_cap_per_round: integer(&cfg["lpCap"])?,
        keep_funds: cfg["keepFunds"]
            .as_bool()
            .ok_or("Invalid generated keepFunds")?,
        hidden: if cfg["hidden"].is_null() {
            None
        } else {
            Some(HiddenRule {
                layer_threshold: number(&cfg["hidden"]["layerThreshold"])?,
                min_lp_exclusive: number(&cfg["hidden"]["minLp"])?,
            })
        },
    };
    let limits = crate::battle::runtime::Limits::default();
    let plans = input["plans"]
        .as_array()
        .ok_or("Missing generated plans")?
        .iter()
        .map(|p| {
            let boss = || -> Result<Boss, String> {
                let o = &p["overtime"];
                Ok(Boss {
                    max_hp: number(&p["maxHp"])?,
                    overtime: if o.is_null() {
                        None
                    } else {
                        Some(BossOvertime {
                            after_real_seconds: number(&o["afterReal"])?,
                            combat_time_scale: number(&o["combatTimeScale"])?,
                            lp_per_real_second: number(&o["lpPerSecond"])?,
                        })
                    },
                })
            };
            let kind = match p["kind"].as_str() {
                Some("normal") => RoundKind::Normal,
                Some("boss") => RoundKind::FinalAssault(boss()?),
                Some("hidden") => RoundKind::HiddenCore(boss()?),
                _ => return Err("Invalid generated round kind".into()),
            };
            let mut wave = Template::parse(
                p,
                &Value::Null,
                &Value::Null,
                limits.units,
                limits.route_legs,
            )?;
            if let Some(id) = p["templateId"].as_str() {
                wave.configure_bounty_actions(
                    &data.table("waves").ok_or("Missing waves")?[id],
                    p["spawnActions"]
                        .as_array()
                        .ok_or("Missing generated spawn actions")?,
                    &data.table("factions").ok_or("Missing factions")?["templateSlots"],
                    limits.units,
                )?;
            }
            wave.spawns = p["spawns"]
                .as_array()
                .ok_or("Missing generated spawns")?
                .clone();
            Ok(RoundPlan {
                kind,
                wave,
                time_limit: if p["timeLimit"].is_null() && p["kind"] != "normal" {
                    f64::INFINITY
                } else {
                    number(&p["timeLimit"])?
                },
                income: integer(&p["income"])?,
                deploy_cap: usize::try_from(integer(&p["deployCap"])?)
                    .map_err(|_| "Invalid generated deployment cap")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let strings = |v: &Value| -> Result<Vec<String>, String> {
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
    };
    let mut banned = strings(&input["preparation"]["banned"])?;
    let off: std::collections::BTreeSet<_> = generated.setup["disabledBonds"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            generated.setup["staticInactiveBonds"]
                .as_array()
                .into_iter()
                .flatten(),
        )
        .filter_map(Value::as_str)
        .collect();
    for (id, rec) in data.table("chess").unwrap().as_object().unwrap() {
        if rec["diyFor"] == id.as_str()
            && rec["bonds"].as_array().is_some_and(|bs| {
                !bs.is_empty()
                    && bs
                        .iter()
                        .all(|b| b.as_str().is_some_and(|b| off.contains(b)))
            })
        {
            banned.push(id.clone());
        }
    }
    let prep = preparation::Config {
        mode_id: "mode_single_abyss".into(),
        banned,
        initial: vec![],
        capacity: 256,
        effects: Some(preparation::EffectsConfig {
            disabled_bonds: strings(&input["preparation"]["effects"]["disabledBonds"])?,
            ..Default::default()
        }),
    };
    let mut run = Runner::new_with_preparation(
        data,
        input["stage"].clone(),
        seed.max(1),
        config,
        plans,
        limits,
        prep,
    )?;
    run.configure_map(Default::default())?;
    run.configure_core(core)?;
    run.enable_drafts(
        generated.setup["stageId"]
            .as_str()
            .ok_or("Missing generated stage ID")?,
    )?;
    let setup = &generated.setup;
    let mode = &run.data.table("config").unwrap()["modes"]["mode_single_abyss"];
    let disabled: std::collections::BTreeSet<_> = setup["disabledBonds"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            setup["staticInactiveBonds"]
                .as_array()
                .into_iter()
                .flatten(),
        )
        .filter_map(Value::as_str)
        .collect();
    let public = json!({"modeId":"mode_single_abyss","difficulty":"ABYSS",
        "stageId":setup["stageId"],"factions":setup["factions"],"bossId":setup["bossId"],
        "disabledBonds":disabled,"drawnDisabledBonds":setup["disabledBonds"],"bannedChess":setup["banned"],
        "bossRound":run.boss_index+1,"lastRound":mode["lastRound"].as_u64().unwrap_or((run.boss_index+1) as u64),
        "hiddenRound":mode["hiddenRound"],"offers":[]});
    Ok((run, public))
}
