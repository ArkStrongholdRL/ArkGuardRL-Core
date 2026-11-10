// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Host {
    pub(in crate::battle::runtime) fn bonds_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
    ) -> Result<(), String> {
        self.bonds_core_landed(w, r, q)?;
        self.bond_thorns(w, r, q)?;
        let Some(source) = q.hit.source else {
            return Ok(());
        };
        let Some(p) = self.bonds.plans.iter().find(|p| p.id == "arcaneShip") else {
            return Ok(());
        };
        if q.hit.damage_type != crate::battle::damage::DamageType::Arts
            || w.unit(q.hit.target)?.side != health::Side::Enemy
            || !w.unit(q.hit.target)?.stats.alive
            || w.unit(source)?.deployment.kind != Kind::Operator
            || !self.equipment_member(source, &p.id)
        {
            return Ok(());
        }
        let l = self.bond_layers(&p.id);
        let mut value =
            (p.value("base_damage_scale", 1.) + p.n("damage_scale_per_stack") * l).max(0.);
        let target = w.unit_mut(q.hit.target)?;
        let ratio = target.stats.hp / target.stats.stats().map_err(str::to_string)?.n("maxHp");
        if p.tier >= 2 && ratio < p.n("hp_ratio") {
            value = (value * p.value("power_weak_scale", 1.)).max(0.);
        }
        crate::battle::valued::apply(
            w,
            self,
            r,
            q.hit.target,
            crate::battle::valued::Request {
                key: "bond:arcaneShip",
                kind: crate::battle::valued::Kind::Custom(|v| [("artsTakenMul".into(), v)].into()),
                plain: true,
                duration: p.value("weak_duration", 3.),
                value,
                source: Some(source),
                stack_as: None,
            },
        )?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn bonds_hit(
        &self,
        w: &mut World,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        let Some(source) = q.hit.source else {
            return Ok(());
        };
        let Some(p) = self.bonds.plans.iter().find(|p| p.id == "kjeragShip") else {
            return Ok(());
        };
        if !self.equipment_member(source, &p.id)
            || w.unit(q.hit.target)?.side != health::Side::Enemy
        {
            return Ok(());
        }
        let stats = w
            .unit_mut(q.hit.target)?
            .stats
            .stats()
            .map_err(str::to_string)?;
        if stats.flags.contains("cold") || stats.flags.contains("freeze") {
            let base = p.value("base_damage_scale", 1.);
            let target = p.value("base_ex_damage_scale", base)
                + p.n("ex_damage_scale_per_stack") * self.bond_layers(&p.id);
            if base > 0. && target > 0. {
                q.damage.mul *= target / base;
            }
        }
        Ok(())
    }
}
