// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Indom {
    pub free: BTreeSet<Id>,
    last: BTreeMap<Id, f64>,
}

impl Host {
    fn indom_plan(&self, w: &World, id: Id) -> Option<Arc<Bond>> {
        let unit = w.unit(id).ok()?;
        if unit.deployment.kind != Kind::Operator || !unit.targeting.melee_operator {
            return None;
        }
        self.bonds
            .plans
            .iter()
            .find(|p| p.id == "indomShip")
            .cloned()
    }
    fn indom_roll(&mut self, p: &Bond) -> bool {
        let probability =
            (p.n("base_prob") + p.n("prob_per_stack") * self.bond_layers(&p.id)).clamp(0., 1.);
        self.rng.next_f64() < probability
    }
    pub(in crate::battle::runtime) fn bond_doll_swap(
        &mut self,
        w: &World,
        id: Id,
    ) -> Result<(), String> {
        if !w.unit(id)?.stats.alive || self.bonds.indom.free.contains(&id) {
            return Ok(());
        }
        if let Some(p) = self.indom_plan(w, id) {
            if self.indom_roll(&p) {
                self.bonds.indom.free.insert(id);
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn bond_indom_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        reason: Option<&str>,
    ) -> Result<(), String> {
        if !matches!(reason, Some("killed" | "retreat" | "merchant")) {
            return Ok(());
        }
        let p = self.indom_plan(w, id);
        let banked = self.bonds.indom.free.contains(&id);
        if p.is_none() && !banked {
            return Ok(());
        }
        if let Some(p) = p
            .as_ref()
            .filter(|p| p.tier >= 2 && reason == Some("killed"))
        {
            let amount = p.n("sp");
            if amount > 0. {
                for target in self.bond_ops(w) {
                    if !Self::tac_up(w, target) {
                        continue;
                    }
                    if self.roster.as_ref().unwrap().entry(target)?.skill().is_ok() {
                        let mut s = self.roster.as_mut().unwrap().take_skill(target)?;
                        let result = self.with_kit_buffs(w, r, |h, w| {
                            s.gain(w, h, amount, "bond", false, false)
                        });
                        let restored = self.roster.as_mut().unwrap().restore_skill(target, s);
                        result.and(restored)?;
                    } else {
                        let limit = w
                            .unit_capacity()
                            .checked_mul(self.roster.as_ref().unwrap().ids().len())
                            .ok_or("Bond SP capacity overflow")?;
                        let pending = &mut w.unit_mut(target)?.operator.support.sp_changes;
                        if pending.len() >= limit {
                            return Err("Pending bond SP capacity exceeded".into());
                        }
                        pending.push(crate::battle::content::operators::SpChange::Bond(amount));
                    }
                }
            }
        }
        let unit = w.unit(id)?;
        if unit.stats.alive || unit.removed || self.bonds.indom.last.get(&id) == Some(&w.time) {
            return Ok(());
        }
        if banked || p.as_ref().is_some_and(|p| self.indom_roll(p)) {
            self.bonds.indom.last.insert(id, w.time);
            let mut context = kits::dispatch::Context {
                host: self,
                world: w,
                buffs: r,
                unit: id,
            };
            context.redeploy_free(id)?;
        }
        Ok(())
    }
}
