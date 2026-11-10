// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Data<'_> {
    fn power(&self, key: &str) -> f64 {
        let e = self.enemy(key);
        if let Some(p) = e["attrPower"].as_f64().filter(|p| p.is_finite()) {
            return p;
        }
        let s = &e["stats"];
        let b = &self.factions["generation"]["beFactors"];
        let p = ((n(&s["atk"], 0.) * n(&b["atk"], 5.)) as f32
            + (n(&s["maxHp"], 0.) * n(&b["hp"], 1.)) as f32) as f32;
        let p = (p + (n(&s["def"], 0.) * n(&b["def"], 3.)) as f32) as f32;
        (p + (n(&s["res"], 0.) * n(&b["res"], 3.)) as f32) as f64
    }
    fn replaced(&self, original: &str, count: u64, new: &str) -> u64 {
        let factor = |key| {
            let x = n(&self.enemy(key)["beFactor"], 1.);
            if x > 0. {
                x
            } else {
                1.
            }
        };
        let origin =
            ((count as f64 * self.power(original)) as f32 as f64 / factor(original)) as f32;
        let unit = (self.power(new) / factor(new)) as f32;
        let g = &self.factions["generation"];
        let min = g["minReplacedEnemyCount"]
            .as_u64()
            .unwrap_or(1)
            .clamp(1, 50);
        let max = g["maxReplacedEnemyCount"]
            .as_u64()
            .unwrap_or(5)
            .clamp(1, 50);
        if unit <= 0. {
            return min;
        }
        let ratio = (origin / unit) as f64;
        let f = ratio.floor();
        let rounded = if ratio - f == 0.5 {
            if f % 2. == 0. {
                f
            } else {
                f + 1.
            }
        } else {
            (ratio + 0.5).floor()
        };
        (rounded.min(max as f64).max(min as f64)) as u64
    }
    pub(super) fn plan(
        &self,
        round: usize,
        pick: &Value,
        boss: bool,
        boss_id: &str,
        speed: f64,
    ) -> Result<Value, String> {
        let rc = &self.mode["rounds"][round.to_string()];
        let mut id = if boss {
            rc["bossTemplates"][boss_id].as_str().or_else(|| {
                rc["bossTemplates"]
                    .as_object()
                    .and_then(|m| m.values().find_map(Value::as_str))
            })
        } else {
            rc["template"].as_str()
        }
        .ok_or("Missing round template")?
        .to_owned();
        if boss && !id.ends_with("_s") && self.table("waves")[format!("{id}_s")].is_object() {
            id.push_str("_s");
        }
        let tpl = &self.table("waves")[&id];
        if !tpl.is_object() {
            return Err(format!("Unknown round template: {id}"));
        }
        let original = list(&tpl["spawns"]);
        if original.len() > CAP || list(&tpl["routes"]).len() > CAP {
            return Err("Mode wave capacity exceeded".into());
        }
        let scale = &self.mode["enemyScale"][round.to_string()];
        let common = json!({"hpMul":n(&scale["hp"],1.).max(0.01),"atkMul":n(&scale["atk"],1.).max(0.),"speedMul":n(&scale["speed"],1.).max(0.01)});
        let mut spawns = Vec::new();
        let mut actions = Vec::new();
        for (i, s) in original.iter().enumerate() {
            if !s.is_object()
                || s["action"]
                    .as_str()
                    .is_some_and(|a| !a.eq_ignore_ascii_case("SPAWN"))
            {
                continue;
            }
            let Some(key0) = s["key"]
                .as_str()
                .or(s["enemyKey"].as_str())
                .filter(|k| *k != ACLOON)
            else {
                continue;
            };
            let is_boss = s["tag"] == "boss";
            let part = s["tag"] == "part";
            let count = s["count"].as_u64().unwrap_or(1).clamp(1, 50);
            let interval = n(&s["interval"], 0.).max(0.);
            let time = n(&s["time"], 0.).max(0.);
            let route = s["routeIndex"].as_u64().unwrap_or(0);
            let window = count as f64 * interval;
            if !window.is_finite() {
                return Err("Invalid action window".into());
            }
            let mut key = key0;
            let mut actual = count;
            let mut step = interval;
            let configured = &self.factions["generation"]["placeholders"];
            let fallback=self.factions["templateSlots"].as_object().and_then(|m|m.iter().find(|(_,v)|v==&key0)).map(|(slot,_)|json!({"slot":slot,"cls":match slot.as_str(){"E"|"EF"=>"elite","S"|"SF"=>"special",_=>"normal"},"fly":slot.ends_with('F')})).unwrap_or(Value::Null);
            let ph = if configured.is_object() {
                &configured[key0]
            } else {
                &fallback
            };
            if !is_boss && !part && ph.is_object() && !pick.is_null() {
                let new = match ph["cls"].as_str() {
                    Some("special") => pick["key"].as_str(),
                    Some("elite") => pick["elite"].as_str(),
                    _ => pick["normal"].as_str(),
                };
                if let Some(new) = new.filter(|k| self.enemy(k).is_object()) {
                    if self.fly(new) != (ph["fly"] == true) {
                        actions.push(json!({"index":i,"key":null,"tplKey":key0,"slot":ph.get("slot").or(s.get("slot")),"time":time,"count":0,"tplCount":count,"window":window,"routeIndex":route,"valid":false,"server":false}));
                        continue;
                    }
                    actual = self.replaced(key0, count, new);
                    step = (window / actual as f64).max(0.05 * window);
                    key = new;
                }
            }
            if !self.enemy(key).is_object() {
                continue;
            }
            let slot = s["slot"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(
                    || match (self.fly(key), self.enemy(key)["rank"] == "ELITE") {
                        (true, true) => "EF",
                        (true, false) => "NF",
                        (false, true) => "E",
                        _ => "N",
                    },
                );
            let mut mods = common.clone();
            if is_boss {
                mods.as_object_mut().unwrap().remove("hpMul");
            } else {
                let supply = n(&scale["supplyHp"], 1.);
                if supply > 0. && supply != 1. {
                    mods["supplyHpMul"] = json!(supply);
                }
            }
            mods["slot"] = json!(slot);
            let mut spawn = json!({"time":time,"enemyKey":key,"routeIndex":route,"count":actual,"interval":if actual>1 {step}else{0.},"mods":mods,"actionIndex":i});
            if !s["tag"].is_null() {
                spawn["tag"] = s["tag"].clone();
            }
            if s["unharmful"] == true || part {
                spawn["countInTotal"] = json!(false);
            }
            spawns.push(spawn);
            actions.push(json!({"index":i,"key":key,"tplKey":key0,"slot":s["slot"],"time":time,"count":actual,"tplCount":count,"window":window,"routeIndex":route,"valid":true,"server":!is_boss&&s["group"].as_str().is_none_or(|g|g.is_empty())}));
        }
        let income = self.cfg["economy"]["income"][round].as_u64().unwrap_or(
            (3 + round as u64).min(self.cfg["economy"]["incomeCap"].as_u64().unwrap_or(12)),
        );
        let time = n(
            &rc["combatTimeLimit"],
            n(&self.mode["combatTimeLimit"][round.to_string()], 60.),
        );
        let mut plan = json!({"kind":if !boss {"normal"}else if self.mode["hiddenRound"].as_u64()==Some(round as u64){"hidden"}else{"boss"},"timeLimit":if boss {Value::Null}else{json!(if time>0.{time*speed}else{60.*speed})},"income":income,"deployCap":positive(&self.cfg["economy"]["deployCap"],8),"templateId":id,"spawnActions":actions,"spawns":spawns,"routes":list(&tpl["routes"]),"extraRoutes":list(&tpl["extraRoutes"]),"overrides":if tpl["overrides"].is_object(){tpl["overrides"].clone()}else{json!({})},"actions":[]});
        plan["devices"] = json!(list(&tpl["devices"]));
        plan["branches"] = if tpl["branches"].is_object() {
            tpl["branches"].clone()
        } else {
            json!({})
        };
        if boss {
            let boss = &self.table("bosses")[boss_id];
            let diff = self.mode["difficulty"].as_str().unwrap_or("NORMAL");
            let hp = boss["bloodPoint"][diff]
                .as_f64()
                .or_else(|| {
                    boss["bloodPoint"]
                        .as_object()
                        .and_then(|m| m.values().find_map(Value::as_f64))
                })
                .unwrap_or(500000.);
            let share = n(
                &self.mode["bossHpScale"]["solo"],
                n(&self.cfg["bossHpScale"]["solo"], 1.),
            );
            plan["maxHp"] = json!((hp * if share > 0. { share } else { 1. } + 0.5)
                .floor()
                .max(1.));
            let after = n(&rc["levelMaxPlayTime"], 0.);
            let after = if after > 0. {
                after
            } else {
                n(&self.cfg["bossOvertimeAfter"], 150.).max(0.)
            };
            plan["overtime"] = json!({"afterReal":after,"combatTimeScale":speed,"lpPerSecond":n(&self.cfg["bossOvertimeDrainPerSec"],1.).max(0.)});
        }
        Ok(plan)
    }
}
