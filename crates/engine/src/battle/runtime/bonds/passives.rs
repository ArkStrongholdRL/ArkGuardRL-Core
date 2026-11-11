// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Host {
    pub(in crate::battle::runtime) fn bonds_install(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if self.bonds.plans.is_empty() {
            return Ok(());
        }
        let ids = self.roster.as_ref().unwrap().ids().to_vec();
        for p in self.bonds.plans.clone() {
            let l = self.bond_layers(&p.id);
            if p.id == "kjeragShip" && p.tier >= 2 && self.devices.config.cold_wind.is_none() {
                let interval = p.n("bond_eff_kjerag[storm].interval");
                let duration = p.n("bond_eff_kjerag[storm].base_time")
                    + p.n("bond_eff_kjerag[storm].time_per_stack") * l;
                if interval > 0. && duration.is_finite() && duration >= 0. {
                    self.devices.config.cold_wind = Some(ColdWind {
                        interval,
                        duration,
                        first: None,
                    });
                    self.devices.cold_due = Some(interval);
                    self.growth.follow_wind = true;
                }
            }
            for &id in &ids {
                if w.unit(id)?.deployment.kind != Kind::Operator {
                    continue;
                }
                let member = self.equipment_member(id, &p.id);
                let elite = self.bond_elite(id);
                let mut mods = Numbers::new();
                let mut key = format!("bond:{}", p.id);
                match p.id.as_str() {
                    "preciShip" => {
                        let def = &self.roster.as_ref().unwrap().entry(id)?.definition;
                        let ranged = self
                            .data
                            .table("chess")
                            .and_then(|t| t.get(&def.key))
                            .is_some_and(|c| c["position"] == "RANGED");
                        if !member && !(p.tier >= 2 && ranged) {
                            continue;
                        }
                        mods.insert("atkPct".into(), p.n("base_atk") + p.n("atk_per_stack") * l);
                        if p.tier >= 2 {
                            mods.insert("defIgnorePct".into(), p.n("power_def_penetrate"));
                            mods.insert("resIgnorePct".into(), p.n("power_magic_resist_penetrate"));
                        }
                    }
                    "raidShip" => {
                        if member
                            && w.unit(id)?
                                .stats
                                .buffs()
                                .iter()
                                .any(|b| b.key.as_deref() == Some("bond:raidShip"))
                        {
                            self.raid_buff(w, r, id, &p)?;
                        }
                        if l < p.value("power_bond_stack_cnt", f64::INFINITY) {
                            continue;
                        }
                        key.push_str(":aspd");
                        mods.insert("aspd".into(), p.n("power_attack_speed"));
                        for (param, stat) in [
                            ("power_cost_scale", "costMul"),
                            ("power_respawn_time", "redeployMul"),
                        ] {
                            if let Some(value) = p.bb.get(param) {
                                mods.insert(stat.into(), *value);
                            }
                        }
                    }
                    "yanShip" => {
                        if !member {
                            continue;
                        }
                        key = "bond:yan".into();
                        mods.insert("atkPct".into(), p.n("base_atk") + p.n("atk_per_stack") * l);
                    }
                    "egirShip" => {
                        if !member {
                            continue;
                        }
                        key = "bond:egir".into();
                        mods.insert(
                            "hpPct".into(),
                            p.n("base_max_hp") + p.n("max_hp_per_stack") * l,
                        );
                    }
                    "steadShip" => {
                        mods.insert(
                            "hpPct".into(),
                            p.n("base_max_hp") + p.n("max_hp_per_stack") * l,
                        );
                    }
                    "deputShip" => {
                        mods.insert("defPct".into(), p.n("base_def") + p.n("def_per_stack") * l);
                        mods.insert("redeployMul".into(), (1. + p.n("respawn_time")).max(0.));
                    }
                    "victoriaShip" => {
                        if !member {
                            continue;
                        }
                        let items = &self.equipment.units[&id].items;
                        if items.is_empty() {
                            continue;
                        }
                        let atk = items
                            .iter()
                            .map(|item| {
                                p.n("atk_normal_equip")
                                    + if item.ends_with("_b") {
                                        p.n("atk_golden_equip")
                                    } else {
                                        0.
                                    }
                            })
                            .sum::<f64>();
                        mods.insert(
                            "dmgDealtMul".into(),
                            p.value("base_damage_scale", 1.) + p.n("damage_scale_per_stack") * l,
                        );
                        key = "bond:victoria".into();
                        self.equipment_stat(w, r, id, key.clone(), mods.clone())?;
                        if p.tier < 2 {
                            continue;
                        }
                        key = "bond:victoria:atk".into();
                        mods.clear();
                        mods.insert("atkPct".into(), p.value("power_atk", atk));
                    }
                    "kjeragShip" => {
                        if !member {
                            continue;
                        }
                        key = "bond:kjerag".into();
                        mods.insert("dmgDealtMul".into(), p.value("base_damage_scale", 1.));
                    }
                    "emptyShip" => {
                        let taken = (1. - p.n("damage_resistance")).max(0.);
                        mods.insert("physTakenMul".into(), taken);
                        mods.insert("artsTakenMul".into(), taken);
                        self.equipment_stat(w, r, id, key.clone(), mods.clone())?;
                        if !member {
                            continue;
                        }
                        key.push_str(":dmg");
                        mods.clear();
                        mods.insert(
                            "dmgDealtMul".into(),
                            p.value(
                                if elite {
                                    "damage_scale_extra"
                                } else {
                                    "damage_scale_normal"
                                },
                                1.,
                            ),
                        );
                    }
                    "soloShip" => {
                        if !member {
                            continue;
                        }
                        mods.insert("atkPct".into(), p.n("atk"));
                        mods.insert("hpPct".into(), p.n("max_hp"));
                    }
                    "suntShip" => {
                        if !elite {
                            continue;
                        }
                        mods.insert("atkPct".into(), p.n("power_atk"));
                        if p.tier >= 2 && !self.bonds.skill_costs.contains(&id) {
                            let mut s = self.roster.as_mut().unwrap().take_skill(id)?;
                            let result = s.bank.set_multiplier(
                                w,
                                s.bank.cost_multiplier() * p.value("sp_ratio", 1.),
                            );
                            self.roster.as_mut().unwrap().restore_skill(id, s)?;
                            result?;
                            self.bonds.skill_costs.insert(id);
                        }
                    }
                    _ => continue,
                }
                if mods.values().any(|v| !v.is_finite()) {
                    return Err("Bond passive overflow".into());
                }
                self.equipment_stat(w, r, id, key, mods)?;
            }
        }
        self.bond_kazimierz_refresh(w, r)?;
        Ok(())
    }
}
