// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone)]
pub(super) struct Choice {
    id: String,
    round: u32,
    source: String,
    cards: Vec<Value>,
}
#[derive(Clone)]
pub(super) struct Bounty {
    id: String,
    card: Value,
    left: u32,
}
impl Choice {
    pub(super) fn report(&self) -> Value {
        json!({"id":self.id,"round":self.round,"sourceItemId":self.source,"cards":self.cards})
    }
}
impl Bounty {
    pub(super) fn report(&self) -> Value {
        json!({"id":self.id,"card":self.card,"roundsLeft":self.left})
    }
}
impl Preparation {
    pub(super) fn add_bounty(&mut self, card: &Value) -> Result<(), Error> {
        if self.bounties.len() >= self.capacity {
            return Err(Error::Capacity);
        }
        let key = card["enemyKey"].as_str().ok_or(Error::BadTarget)?;
        if !self.rules.data.table("enemies").unwrap()[key].is_object() {
            return Err(Error::BadTarget);
        }
        let multi = card["multiRound"] == true || card["rounds"].as_f64().is_some_and(|r| r >= 90.);
        let rounds = if multi {
            2
        } else {
            card["rounds"].as_u64().unwrap_or(1).clamp(1, 99) as u32
        };
        let coin = effects::n(&card["coin"], 0.).trunc().max(0.);
        if coin > MAX_FUNDS as f64 {
            return Err(Error::Capacity);
        }
        self.uid = add(self.uid, 1).map_err(|_| Error::Capacity)?;
        self.bounties.push(Bounty { id:format!("bounty:{}",self.uid), left:rounds, card:json!({
            "effectId":card.get("effectId").or(card.get("id")), "name":card["name"].as_str().unwrap_or("悬赏"),
            "desc":card["desc"].as_str().unwrap_or(""),"tier":card["tier"].as_u64().unwrap_or(1),
            "coin":coin as u64,"payout":if card["payout"]=="perfect" {"perfect"} else {"kill"},
            "rounds":rounds,"multiRound":multi,"enemyKey":key,"count":card["count"].as_u64().unwrap_or(1).clamp(1,20)
        }) });
        Ok(())
    }
    pub(super) fn bounty_art(&mut self, item: &str) -> Result<(), Error> {
        let customer = self.rules.data.table("items").unwrap()[item]["buffs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["key"] == "trap_disney_special");
        if !customer && self.personal_choice.is_some() {
            return Err(Error::BadTarget);
        }
        let inactive = &self.rules.data.table("config").unwrap()["modes"][&self.rules.mode_id]
            ["inactiveEnemyKeys"];
        let cards = self.rules.data.table("choices").unwrap()["cards"]["bounty"]
            .as_array()
            .ok_or(Error::BadTarget)?;
        if cards.len() > 4096 {
            return Err(Error::Capacity);
        }
        let mut eligible: Vec<_> = cards
            .iter()
            .filter(|c| {
                let family = c["effectId"]
                    .as_str()
                    .and_then(|s| s.strip_prefix("enemyeffect_b_"))
                    .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
                (if customer {
                    family
                } else {
                    c["payout"] == "perfect"
                }) && c["enemyKey"]
                    .as_str()
                    .is_some_and(|k| self.rules.data.table("enemies").unwrap()[k].is_object())
                    && !inactive
                        .as_array()
                        .is_some_and(|a| a.contains(&c["enemyKey"]))
            })
            .cloned()
            .collect();
        if customer && eligible.is_empty() {
            let perfect = self.rules.data.table("items").unwrap()[item]["desc"]
                .as_str()
                .is_some_and(|d| d.contains("完美"));
            let fallback: Vec<_> = cards
                .iter()
                .filter(|c| {
                    c["tier"].as_f64().is_some_and(|n| n <= 2.)
                        && c["payout"] == if perfect { "perfect" } else { "kill" }
                        && c["enemyKey"].as_str().is_some_and(|k| {
                            self.rules.data.table("enemies").unwrap()[k].is_object()
                        })
                        && !inactive
                            .as_array()
                            .is_some_and(|a| a.contains(&c["enemyKey"]))
                })
                .cloned()
                .collect();
            let card = self
                .meta
                .as_mut()
                .unwrap()
                .rng
                .pick(&fallback)
                .cloned()
                .ok_or(Error::BadTarget)?;
            return self.add_bounty(&card);
        }
        if eligible.is_empty() {
            return Err(Error::BadTarget);
        }
        self.meta.as_mut().unwrap().rng.shuffle(&mut eligible);
        eligible.truncate(3);
        if customer {
            let card = self
                .meta
                .as_mut()
                .unwrap()
                .rng
                .pick(&eligible)
                .unwrap()
                .clone();
            self.add_bounty(&card)
        } else {
            self.uid = add(self.uid, 1).map_err(|_| Error::Capacity)?;
            self.personal_choice = Some(Choice {
                id: format!("solo.choice.{}", self.uid),
                round: self.meta.as_ref().unwrap().round,
                source: item.into(),
                cards: eligible,
            });
            Ok(())
        }
    }
    pub(super) fn pick_bounty(&mut self, index: usize, id: &str) -> Result<(), Error> {
        let choice = self
            .personal_choice
            .as_ref()
            .filter(|c| c.id == id && self.meta.as_ref().is_some_and(|m| m.round == c.round))
            .ok_or(Error::BadTarget)?;
        let card = choice.cards.get(index).ok_or(Error::BadTarget)?.clone();
        self.add_bounty(&card)?;
        self.personal_choice = None;
        Ok(())
    }
    pub(super) fn destroy_art(&mut self, item: &Piece, player: &mut Player) -> Result<(), Error> {
        if let Some(buffs) = self.rules.data.table("items").unwrap()[&item.key]["buffs"].as_array()
        {
            for buff in buffs {
                if buff["key"] == "trap_disney_special" {
                    let n = effects::n(
                        buff["bbStr"].get("count").unwrap_or(&buff["bb"]["count"]),
                        1.,
                    )
                    .trunc()
                    .max(0.);
                    if n > MAX_FUNDS as f64 {
                        return Err(Error::Capacity);
                    }
                    player.earn(n as u64).map_err(|_| Error::Capacity)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::solo) fn settle_bounties(&mut self, perfect: bool) -> Result<f64, String> {
        let mut coin = 0u64;
        for b in &mut self.bounties {
            if perfect && b.card["payout"] == "perfect" {
                coin = add(coin, b.card["coin"].as_u64().unwrap())?;
            }
            b.left -= 1;
        }
        self.bounties.retain(|b| b.left > 0);
        Ok(coin as f64)
    }
    /// Compile bounty enemies, then let the wave owner insert them into action windows.
    pub(in crate::solo) fn append_bounties(
        &self,
        wave: &mut crate::battle::waves::Template,
        round: usize,
        boss: bool,
    ) -> Result<(), String> {
        if wave.spawns.len() + self.bounties.len() > self.capacity {
            return Err("Bounty spawn capacity exceeded".into());
        }
        let scale = &self.rules.data.table("config").unwrap()["modes"][&self.rules.mode_id]
            ["enemyScale"][round.to_string()];
        let mut additions = Vec::new();
        for b in &self.bounties {
            let e =
                &self.rules.data.table("enemies").unwrap()[b.card["enemyKey"].as_str().unwrap()];
            let fly = e["isFlyEnemy"]
                .as_bool()
                .unwrap_or(e["stats"]["motion"] == "FLY");
            let motion = |r: &&Value| {
                if fly {
                    r["motion"] == "FLY"
                } else {
                    r["motion"] != "FLY" && r["motion"] != "E_NUM"
                }
            };
            let route = wave
                .routes
                .iter()
                .enumerate()
                .find(|(_, r)| {
                    motion(r) && (!boss || r["end"][1].as_f64().is_some_and(|x| x < 10.))
                })
                .or_else(|| wave.routes.iter().enumerate().find(|(_, r)| motion(r)))
                .map_or(0, |(i, _)| i);
            let mut mods = json!({"hpMul":effects::n(&scale["hp"],1.).max(0.01),"atkMul":effects::n(&scale["atk"],1.).max(0.),"speedMul":effects::n(&scale["speed"],1.).max(0.01),
                "slot":match (fly,e["rank"]=="ELITE") {(true,true)=>"EF",(true,false)=>"NF",(false,true)=>"E",_=>"N"},"bountyId":b.id});
            let supply = effects::n(&scale["supplyHp"], 1.);
            if supply > 0. && supply != 1. {
                mods["supplyHpMul"] = json!(supply);
            }
            let mut spawn = json!({"time":4,"enemyKey":b.card["enemyKey"],"routeIndex":route,"count":b.card["count"],"interval":0,"mods":mods,"tag":"bounty"});
            if b.card["payout"] != "perfect" && b.card["coin"].as_u64().unwrap() > 0 {
                spawn["bounty"] = json!({"coins":b.card["coin"]});
            }
            additions.push(spawn);
        }
        wave.insert_bounties(additions, boss, self.capacity)
    }
}
