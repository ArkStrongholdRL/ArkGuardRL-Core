// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Host {
    pub(in crate::battle::runtime) fn bonds_aura(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let Some(p) = self.bonds.plans.iter().find(|p| p.id == "skillfulShip") else {
            return Ok(());
        };
        let l = self.bond_layers(&p.id);
        let wide = l >= p.value("power_bond_stack_cnt", f64::INFINITY);
        let aspd = p.n("base_attack_speed") + p.n("attack_speed_per_stack") * l;
        let ids = self.bond_ops(w);
        let mut next = std::collections::BTreeSet::new();
        for &id in &ids {
            if !self.equipment_member(id, "skillfulShip") {
                continue;
            }
            let live = Self::tac_up(w, id);
            let unit = w.unit(id)?;
            if !live
                && !(unit.deployment.remove_reason.as_deref() == Some("killed")
                    && crate::battle::deploy::Deployment::is_down(w, id)?)
            {
                continue;
            }
            let at = if live {
                unit.deployment.tile
            } else {
                crate::battle::deploy::Deployment::rest_tile(w, id)?
            };
            if live {
                next.insert(id);
            }
            for &target in &ids {
                if !Self::tac_up(w, target) || w.unit(target)?.deployment.kind != Kind::Operator {
                    continue;
                }
                let tile = w.unit(target)?.deployment.tile;
                let dr = (tile[0] - at[0]).abs();
                let dc = (tile[1] - at[1]).abs();
                if dr.max(dc) == 1 && (wide || dr + dc == 1) {
                    next.insert(target);
                }
            }
        }
        for id in ids {
            if !next.contains(&id) {
                r.remove(w, self, id, "bond:skillfulShip")?;
                continue;
            }
            let stats = &mut w.unit_mut(id)?.stats;
            if let Some(b) = stats
                .buffs_mut()
                .iter_mut()
                .find(|b| b.key.as_deref() == Some("bond:skillfulShip"))
            {
                b.mods.insert("aspd".into(), aspd);
                stats.mark_dirty();
            } else {
                let buff = Buff {
                    key: Some("bond:skillfulShip".into()),
                    mods: [("aspd".into(), aspd)].into(),
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
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn bonds_flush(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if self.bonds.aura.pending {
            self.bonds.aura.pending = false;
            self.bonds_aura(w, r)?;
        }
        Ok(())
    }
}
