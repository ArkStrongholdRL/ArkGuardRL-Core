// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit public projection. Never derive policy input by filtering diagnostics.
use super::*;
impl Preparation {
    pub(in crate::solo) fn public_view(&self, player: &Player) -> Result<Value, String> {
        let piece = |p: &Piece| {
            let table = match p.kind {
                Kind::Chess => "chess",
                Kind::Item => "items",
                Kind::Token => "tokens",
            };
            let rec = &self.rules.data.table(table).unwrap()[&p.key];
            json!({"uid":p.uid,"kind":p.kind.name(),"id":p.key,"golden":rec["isGolden"]==true,
                "tier":rec["tier"],"count":p.count,"ownerUid":p.owner_uid,
                "items":p.items.iter().map(|i|json!({"uid":i.uid,"id":i.key})).collect::<Vec<_>>()})
        };
        // Prices in the public protocol are nonnegative integer currency.
        let slot = |s: &Slot| json!({"kind":s.kind.name(),"id":s.key,"price":s.price as u64,"sold":s.sold,"frozen":s.frozen});
        let slots = self
            .slots
            .iter()
            .map(|s| {
                s.as_ref()
                    .map(|s| {
                        let mut v = slot(s);
                        let price = if s.kind == Kind::Chess {
                            self.meta_price(&s.key, s.price)
                                .map_err(|e| e.to_string())?
                        } else {
                            s.price.round().max(0.)
                        };
                        v["price"] = json!(price as u64);
                        Ok::<_, String>(v)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let board = self
            .board_order()
            .into_iter()
            .map(|(at, p)| {
                let mut v = piece(p);
                v["row"] = json!(at[0]);
                v["col"] = json!(at[1]);
                v["dir"] = json!(p.direction.unwrap_or(Direction::Right).name());
                v
            })
            .collect::<Vec<_>>();
        let mut bonds = Vec::new();
        if let Some(meta) = &self.meta {
            for b in &meta.rules.bonds {
                if let Some(s) = meta.bonds.get(&b.id) {
                    if s.count > 0. || s.layers > 0. || s.active() {
                        let mut v = json!({"bondId":b.id,"count":s.count,"layers":s.layers,"tier":s.tier,"active":s.active(),"thresholds":b.thresholds,"countsHand":matches!(b.mode,bonds::Counting::HandAndBoard)});
                        if s.harmony > 0 {
                            v["harmony"] = json!(s.harmony);
                        }
                        bonds.push(v);
                    }
                } else if b.off {
                    let mut ids = BTreeSet::new();
                    for p in self.board.values().chain(
                        self.hand
                            .iter()
                            .flatten()
                            .filter(|_| matches!(b.mode, bonds::Counting::HandAndBoard)),
                    ) {
                        if p.kind == Kind::Chess && self.piece_bonds(p).contains(&b.id) {
                            ids.insert(self.rules.chess[&p.key].base.clone());
                        }
                    }
                    if !ids.is_empty() {
                        bonds.push(json!({"bondId":b.id,"count":ids.len(),"layers":meta.layers.get(&b.id).copied().unwrap_or(0.),"tier":0,"active":false,"off":true,"thresholds":b.thresholds,"countsHand":matches!(b.mode,bonds::Counting::HandAndBoard)}));
                    }
                }
            }
        }
        bonds.sort_by(|a, b| {
            (a["off"] == true)
                .cmp(&(b["off"] == true))
                .then_with(|| (b["active"] == true).cmp(&(a["active"] == true)))
                .then_with(|| {
                    b["layers"]
                        .as_f64()
                        .unwrap_or(0.)
                        .total_cmp(&a["layers"].as_f64().unwrap_or(0.))
                })
        });
        let mut effects = Vec::new();
        for (id, seq) in self
            .draft_effects
            .battle
            .iter()
            .chain(&self.draft_effects.maps)
        {
            effects.push(json!({"id":format!("{id}#{seq}"),"iconId":id,"iconKind":"choice"}));
        }
        for u in &self.draft_effects.upgrades {
            effects.push(json!({"id":u.id,"iconId":u.id,"iconKind":"choice","counter":u.left}));
        }
        for b in &self.bounties {
            let b = b.report();
            effects.push(json!({"id":b["id"],"iconId":b["card"]["effectId"],"iconKind":"choice","counter":b["roundsLeft"]}));
        }
        Ok(
            json!({"t":"m.private","playerId":"solo","seat":0,"alive":player.lp>0.,"lp":player.lp,"funds":player.funds,
            "ready":false,"canReady":self.check_ready().is_ok(),"personalChoice":self.personal_choice.as_ref().map(bounties::Choice::report),
            "hand":self.hand.iter().map(|p|p.as_ref().map(&piece)).collect::<Vec<_>>(),
            "temp":self.temp.iter().map(|p|p.as_ref().map(&piece)).collect::<Vec<_>>(),"board":board,
            "shop":{"level":self.level,"maxLevel":self.rules.slots.len(),
                "upgradePrice":if self.level>=self.rules.slots.len(){0}else{self.upgrade_price as u64},
                "refreshPrice":if self.free_refreshes>0{0}else{self.rules.refresh as u64},"freeRefreshes":self.free_refreshes,"frozen":self.frozen,
                "slots":slots,
                "rewardOffer":self.offers.front().map(|o|json!({"tier":o.tier,"queued":self.offers.len()-1,"slots":o.slots.iter().map(&slot).collect::<Vec<_>>() }))},
            "deployCap":self.deploy_cap,"deployCount":self.board.values().filter(|p|p.kind==Kind::Chess).count(),
            "deployment":{"field":if self.boss{"bossL"}else{"normal"},"cells":self.grid.public_cells()},
            "bonds":bonds,"effects":effects,"loadout":{},"ops":{},"diy":self.rules.data.table("diy").cloned().unwrap_or_else(||json!({})),"standIns":[],
            "stats":{"gold":player.stats.funds_spent,"refreshes":self.counts.refreshes,"merges":self.counts.merges,
                "kills":player.stats.kills,"leaks":player.stats.leaks,"dmgDealt":player.stats.damage.round()}}),
        )
    }
}
