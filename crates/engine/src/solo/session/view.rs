// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::solo::RoundKind;
impl Session {
    pub(super) fn current_preview(&self) -> Result<Vec<Value>, String> {
        let run = &self.run;
        let Some(plan) = run.plans.get(run.index).and_then(Option::as_ref) else {
            return Ok(self.preview.clone());
        };
        let boss = !matches!(plan.kind, RoundKind::Normal);
        let mut wave = plan.wave.clone();
        if let Some(prep) = &run.prep {
            prep.append_bounties(&mut wave, run.index + 1, boss)?;
        }
        let mut out = Vec::new();
        // The public drawer retains the original wave actions. Interleaving
        // bounties splits those actions only for combat scheduling; expose the
        // added bounty runs without also exposing those internal wave splits.
        let spawns = plan.wave.spawns.iter().chain(
            wave.spawns
                .iter()
                .filter(|s| s["tag"] == "bounty" && !plan.wave.spawns.contains(s)),
        );
        for s in spawns {
            if s["tag"] == "part" {
                continue;
            }
            let Some(key) = s["enemyKey"].as_str() else {
                continue;
            };
            let e = &run.data.table("enemies").ok_or("Missing enemies")?[key];
            let route = wave
                .routes
                .get(s["routeIndex"].as_u64().unwrap_or(0) as usize);
            let start = route.map(|r| &r["start"]).unwrap_or(&Value::Null);
            let upper = start[0]
                .as_f64()
                .is_some_and(|r| r + if boss { 13. } else { 6. } >= 18.);
            let tag = if s["tag"] == "boss" {
                json!("boss")
            } else if s["tag"] == "bounty" {
                json!("bounty")
            } else {
                Value::Null
            };
            let mut row = json!({"enemyKey":key,"count":s["count"].as_u64().unwrap_or(1).max(1),
                "t":s["time"].as_f64().unwrap_or(0.).max(0.),"gate":if upper{"upper"}else{"lower"},
                "fly":e["isFlyEnemy"].as_bool().unwrap_or(e["stats"]["motion"]=="FLY"),
                "elite":tag=="boss" || e["rank"].as_str().is_some_and(|r|r!="NORMAL"),
                "boss":tag=="boss","source":if tag=="bounty"{"bounty"}else{"wave"},"tag":tag});
            if tag == "boss" && start.is_array() {
                row["start"] = start.clone();
            }
            out.push(row);
        }
        out.sort_by(|a, b| {
            a["t"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["t"].as_f64().unwrap())
        });
        Ok(out)
    }
    /// Detached, explicitly allowlisted public state for the existing model schema.
    pub fn observation(&self) -> Result<Value, String> {
        self.healthy()?;
        let run = &self.run;
        let s = run.snapshot()?;
        let ended = s.phase == Phase::Result;
        let phase = match s.phase {
            Phase::Prep => "PREP",
            Phase::Draft => "SP_DRAFT",
            Phase::Combat => "COMBAT",
            Phase::Result => "RESULT",
            Phase::Faulted => return Err("Faulted session".into()),
        };
        let mut own = run
            .prep
            .as_ref()
            .ok_or("Public observation requires owned preparation")?
            .public_view(&run.player)?;
        own["bandId"] = json!(run.core);
        if let Some(core) = &run.core {
            let band = &run.data.table("bands").ok_or("Missing bands")?[core];
            let id = band["effectId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(core);
            let icon = band["iconId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(core);
            own["effects"]
                .as_array_mut()
                .unwrap()
                .insert(0, json!({"id":id,"iconId":icon,"iconKind":"band"}));
        }
        own["lp"] = json!(s.lp);
        // Boss settlement can exhaust team LP without eliminating the player.
        // Only ordinary-round elimination clears the board and ready flag.
        let eliminated = s
            .outcome
            .is_some_and(|o| o.reason == crate::solo::EndReason::Eliminated);
        own["alive"] = json!(!eliminated);
        own["ready"] = json!(s.phase == Phase::Combat || (ended && !eliminated));
        own["canReady"] = json!(s.phase == Phase::Prep && own["canReady"] == true);
        own["nextEnemies"] = if eliminated {
            json!([])
        } else {
            json!(self.current_preview()?)
        };
        let mut public = self.public_setup.clone();
        public["phase"] = json!(phase);
        public["round"] = json!(s.round);
        public["players"] = json!([{"playerId":"solo","seat":0,"isBot":false,"alive":own["alive"],"lp":s.lp,"bandId":run.core}]);
        public["sp"] = run
            .draft
            .as_ref()
            .and_then(|d| d.pending.clone())
            .unwrap_or(Value::Null);
        if public["sp"].is_object() {
            public["sp"]["turn"] = json!("solo");
            public["sp"]["taken"] = json!({});
            if let Some(cards) = public["sp"]["cards"].as_array_mut() {
                for (i, card) in cards.iter_mut().enumerate() {
                    let mut visible = json!({"idx":i,"kind":card["kind"],"id":card["id"],
                        "name":card["name"],"desc":card["desc"],"tier":card["tier"],"soldOut":false});
                    match card["kind"].as_str() {
                        Some("bounty") => {
                            for key in ["descRaw", "coin", "payout", "rounds", "enemyKey", "count"]
                            {
                                visible[key] = card[key].clone();
                            }
                        }
                        Some("item") => visible["price"] = json!(0),
                        Some("tactic") => {
                            visible["team"] = card["team"].clone();
                            visible["tacticKind"] = card["tacticKind"].clone();
                        }
                        _ => (),
                    }
                    *card = visible;
                }
            }
        }
        let outcome=s.outcome.map(|o|json!({"errors":0,"modeId":"mode_single_abyss","difficulty":"ABYSS","victory":o.victory,
            "hiddenReached":o.hidden_reached,"hiddenCleared":o.hidden_cleared,
            "reason":match o.reason {crate::solo::EndReason::Eliminated=>"eliminated",crate::solo::EndReason::Defeat=>"defeat",crate::solo::EndReason::Victory=>"victory"},
            "players":[{"playerId":"solo","alive":own["alive"],"victory":o.victory && own["alive"]==true,"roundsPassed":s.rounds_passed}]}));
        Ok(
            json!({"schema":"arkguardrl/native-session-v2","revision":self.revision,"phase":phase,"round":s.round,
            "gameEnded":ended,"personalEnded":ended,"decisionRequired":matches!(s.phase,Phase::Prep|Phase::Draft),
            "public":public,"own":own,"outcome":outcome}),
        )
    }
}
