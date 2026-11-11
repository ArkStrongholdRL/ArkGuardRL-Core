// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Host {
    pub(in crate::battle::runtime) fn bond_sargon_sync(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "sargonShip" && p.tier >= 2)
        else {
            return Ok(());
        };
        let atk = p.n("base_atk");
        let n = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .filter(|b| b.key.as_deref() == Some("bond:sargon"))
            .count();
        if n > 0 && w.unit(id)?.stats.alive {
            let buff = Buff {
                key: Some("bond:sargon:atk".into()),
                mods: [("atkPct".into(), atk * n as f64)].into(),
                ..Default::default()
            };
            r.add(
                w,
                self,
                id,
                buffs::Spec {
                    buff,
                    mods_present: true,
                    ..Default::default()
                },
            )?;
        } else {
            r.remove(w, self, id, "bond:sargon:atk")?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn bonds_skill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        event: skills::Event,
        c: &skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        if !matches!(
            event,
            skills::Event::Deploy
                | skills::Event::End
                | skills::Event::SkillStart
                | skills::Event::SkillEnd
                | skills::Event::AmmoUsed
        ) {
            return Ok(());
        }
        let id = s.unit();
        if event == skills::Event::Deploy {
            self.bonds.indom.free.remove(&id);
            self.bonds_deploy(w, r, id)?;
            self.bonds_aura(w, r)?;
        }
        if event == skills::Event::End {
            self.bonds.aura.pending = true;
        }
        for p in self.bonds.plans.clone() {
            let l = self.bond_layers(&p.id);
            let member = self.equipment_member(id, &p.id);
            match p.id.as_str() {
                "soloShip" if member && event == skills::Event::Deploy => {
                    if p.n("sp") > 0. {
                        self.with_kit_buffs(w, r, |h, w| {
                            s.gain(w, h, p.n("sp"), "init", false, false)
                        })?;
                    }
                }
                "suntShip" if p.tier >= 2 && event == skills::Event::Deploy => {
                    let owner = w.unit(id)?.summoner.unwrap_or(id);
                    // Each skill runtime receives the multiplier once, including elite summons.
                    if self.bond_elite(owner)
                        && !s.bank.no_skill
                        && !self.bonds.skill_costs.contains(&id)
                    {
                        s.bank.set_multiplier(
                            w,
                            s.bank.cost_multiplier() * p.value("sp_ratio", 1.),
                        )?;
                        self.bonds.skill_costs.insert(id);
                    }
                }
                "swiftShip"
                    if event == skills::Event::SkillEnd
                        && c.reason != "death"
                        && w.unit(id)?.stats.alive
                        && w.unit(id)?.deployment.kind == Kind::Operator =>
                {
                    let prob = (p.n("base_prob") + p.n("prob_per_stack") * l).clamp(0., 1.);
                    for (roll, amount) in [
                        (member, p.n("normal_sp")),
                        (
                            l >= p.value("power_bond_stack_cnt", f64::INFINITY),
                            p.n("power_sp"),
                        ),
                    ] {
                        if roll && self.rng.next_f64() < prob {
                            self.with_kit_buffs(w, r, |h, w| {
                                s.gain(w, h, amount, "bond", false, false)
                            })?;
                        }
                    }
                }
                "sargonShip" if member && event == skills::Event::SkillStart => {
                    let dur = p.n("base_time") + p.n("time_per_stack") * l;
                    if dur <= 0. {
                        continue;
                    }
                    for target in self.roster.as_ref().unwrap().ids().to_vec() {
                        if !Self::tac_up(w, target) || !self.equipment_member(target, &p.id) {
                            continue;
                        }
                        let mut buff = Buff {
                            key: Some("bond:sargon".into()),
                            mods: [("aspd".into(), p.n("base_attack_speed"))].into(),
                            ..Default::default()
                        };
                        buff.runtime.duration = dur;
                        buff.runtime.refresh = buffs::Refresh::Independent;
                        buff.runtime.max_stacks =
                            p.value("max_buff_stack_cnt", 25.).clamp(1., 4096.) as u32;
                        buff.runtime.on_remove = Some(SARGON_EXPIRE);
                        buff.runtime.on_expire = Some(SARGON_EXPIRE);
                        r.add(
                            w,
                            self,
                            target,
                            buffs::Spec {
                                buff,
                                mods_present: true,
                                ..Default::default()
                            },
                        )?;
                        self.bond_sargon_sync(w, r, target)?;
                    }
                }
                "lateranoShip"
                    if member
                        && event == skills::Event::SkillStart
                        && s.bank.kind == crate::battle::sp::Kind::Ammo
                        && s.ammo_left > 0. =>
                {
                    let ammo = (s.ammo_left
                        * (1. + p.n("base_ammo_percent") + p.n("ammo_percent_per_stack") * l)
                        + 1e-9)
                        .floor();
                    if ammo.is_finite() && ammo > s.ammo_left {
                        s.ammo_left = ammo;
                    }
                }
                "lateranoShip"
                    if member
                        && event == skills::Event::AmmoUsed
                        && p.tier >= 2
                        && p.n("atk_per_consume") > 0. =>
                {
                    let cap = p.value("max_atk_for_consume", f64::INFINITY);
                    if self.bonds.ammo_used as f64 * p.n("atk_per_consume") >= cap {
                        continue;
                    }
                    self.bonds.ammo_used = self.bonds.ammo_used.saturating_add(1);
                    let bonus = (self.bonds.ammo_used as f64 * p.n("atk_per_consume")).min(cap);
                    for target in self.roster.as_ref().unwrap().ids().to_vec() {
                        if self.equipment_member(target, &p.id) {
                            self.equipment_stat(
                                w,
                                r,
                                target,
                                "bond:laterano:ammo".into(),
                                [("atkPct".into(), bonus)].into(),
                            )?;
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
}
