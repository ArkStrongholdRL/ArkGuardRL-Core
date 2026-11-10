// SPDX-License-Identifier: GPL-3.0-or-later
//! Steadfast: post-mitigation sharing and per-member retaliation cooldowns.
use super::*;
use crate::battle::{damage::DamageType, status};

const SHARE: &str = "bond:stead:share";
const THORN: &str = "bond:stead:thorn";

impl Host {
    pub(in crate::battle::runtime) fn bonds_has_share(&self) -> bool {
        self.bonds
            .plans
            .iter()
            .any(|p| p.id == "steadShip" && p.tier >= 2)
    }

    pub(in crate::battle::runtime) fn bond_share(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        amount: &mut f64,
    ) -> Result<(), String> {
        let target = w.unit(q.hit.target)?;
        if target.side != health::Side::Ally
            || target.deployment.kind != Kind::Operator
            || q.hit.hp_loss
            || q.tags.iter().any(|t| t == SHARE)
            || self.equipment_member(q.hit.target, "steadShip")
        {
            return Ok(());
        }
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "steadShip" && p.tier >= 2)
        else {
            return Ok(());
        };
        let ratio = p.n("damage_resistance").clamp(0., 1.);
        let members: Vec<_> = self
            .bond_ops(w)
            .into_iter()
            .filter(|&id| Self::tac_up(w, id) && self.equipment_member(id, "steadShip"))
            .collect();
        if members.is_empty() || ratio <= 0. {
            return Ok(());
        }
        let shared = *amount * ratio;
        *amount -= shared;
        for id in &members {
            let mut extra = deployment::damage(*id, *id, shared / members.len() as f64, SHARE);
            extra.tags.extend(q.tags.iter().cloned());
            extra.hit.source = None;
            extra.hit.credit = None;
            extra.calculation = hit::Calculation::Resolved;
            self.enemy_damage(w, r, &mut extra)?;
        }
        Ok(())
    }

    pub(super) fn bond_thorns(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
    ) -> Result<(), String> {
        let id = q.hit.target;
        let target = w.unit(id)?;
        if target.side != health::Side::Ally
            || target.deployment.kind != Kind::Operator
            || q.hit.hp_loss
            || q.tags.iter().any(|t| t == SHARE || t == THORN)
            || !self.equipment_member(id, "steadShip")
        {
            return Ok(());
        }
        let Some(p) = self
            .bonds
            .plans
            .iter()
            .find(|p| p.id == "steadShip" && p.tier >= 2)
            .cloned()
        else {
            return Ok(());
        };
        if !p.bb.contains_key("base_damage_value") && !p.bb.contains_key("damage_value_per_stack") {
            return Ok(());
        }
        let last = self
            .bonds
            .thorns_at
            .get(&id)
            .copied()
            .unwrap_or(f64::NEG_INFINITY);
        if w.time - last < p.n("cd_duration") - 1e-9 {
            return Ok(());
        }
        // Sourceless damage spends the opportunity, even though there is nobody to hit.
        self.bonds.thorns_at.insert(id, w.time);
        let Some(source) = q.hit.source.filter(|&source| {
            w.unit(source)
                .is_ok_and(|u| u.side == health::Side::Enemy && u.stats.alive)
        }) else {
            return Ok(());
        };
        let amount =
            p.n("base_damage_value") + p.n("damage_value_per_stack") * self.bond_layers(&p.id);
        let mut extra = deployment::damage(id, source, amount, THORN);
        extra.hit.source = None;
        extra.hit.damage_type = DamageType::Arts;
        self.enemy_damage(w, r, &mut extra)?;
        if w.unit(source)?.stats.alive && p.bb.contains_key("damage_scale") {
            self.apply_status(
                w,
                r,
                source,
                "fragile",
                status::Options {
                    duration: p.value("weak[limit]", 5.),
                    value: Some(p.value("damage_scale", 1.) - 1.),
                    source: Some(id),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
}
