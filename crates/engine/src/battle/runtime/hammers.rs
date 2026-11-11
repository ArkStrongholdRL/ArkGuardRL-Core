// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared hammer grants, field borrowing and per-deployment undying windows.
use super::*;
use crate::battle::{
    equipment::{FieldHammers, Hammer},
    stats::Buff,
};

impl Host {
    fn field_hammers(&mut self, w: &World, owner: Id) -> Result<[bool; 4], String> {
        if let Some(cache) = self.equipment.fields.get(&owner) {
            if w.time - cache.at < 0.5 - 1e-9 {
                return Ok(cache.present);
            }
        }
        let mut present = [false; 4];
        for (&unit, loadout) in &self.equipment.units {
            let a = w.unit(unit)?;
            if a.owner != Some(owner) || !Self::tac_up(w, unit) {
                continue;
            }
            if let Some(hammers) = &loadout.hammers {
                for (index, &count) in hammers.own.iter().enumerate() {
                    present[index] |= count > 0;
                }
            }
        }
        self.equipment.fields.insert(
            owner,
            FieldHammers {
                at: w.time,
                present,
            },
        );
        Ok(present)
    }
    fn hammer_effect(
        &mut self,
        w: &World,
        unit: Id,
        index: usize,
    ) -> Result<Option<(Hammer, u32)>, String> {
        let Some(loadout) = self.equipment.units.get(&unit) else {
            return Ok(None);
        };
        let Some(hammers) = &loadout.hammers else {
            return Ok(None);
        };
        let count = hammers.own[index];
        let params = hammers.params[index].or(hammers.steam.map(|steam| steam.hammer(index)));
        let steam =
            hammers.steam.is_some() && loadout.bonds.iter().any(|bond| bond == "victoriaShip");
        let borrowed = if steam {
            self.field_hammers(w, w.unit(unit)?.owner.ok_or("Hammer carrier has no owner")?)?[index]
        } else {
            false
        };
        let count = count + u32::from(borrowed);
        Ok(params.filter(|_| count > 0).map(|params| (params, count)))
    }
    pub(super) fn hammer_refresh(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        unit: Id,
    ) -> Result<(), String> {
        let speed = match self.hammer_effect(w, unit, 2)? {
            Some((Hammer::Speed(speed), count)) => speed * count as f64,
            _ => 0.,
        };
        let hammers = self
            .equipment
            .units
            .get_mut(&unit)
            .unwrap()
            .hammers
            .as_mut()
            .unwrap();
        if speed == hammers.speed {
            return Ok(());
        }
        hammers.speed = speed;
        let key = format!("item:hammer#{unit}:aspd");
        if speed == 0. {
            r.remove(w, self, unit, &key)?;
        } else {
            let mut buff = Buff {
                key: Some(key),
                mods: [("aspd".into(), speed)].into(),
                permanent: true,
                ..Default::default()
            };
            buff.runtime.persist = true;
            r.add(
                w,
                self,
                unit,
                buffs::Spec {
                    buff,
                    mods_present: true,
                    allow_dead: true,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn holds_undying(&self, w: &mut World, unit: Id) -> Result<bool, String> {
        let u = w.unit(unit)?;
        let held = self
            .equipment
            .units
            .get(&unit)
            .and_then(|loadout| loadout.hammers.as_ref())
            .is_some_and(|hammers| {
                hammers.window_at == Some(u.deployment.deploy_seq) && w.time < hammers.until
            });
        Ok(held || Self::tac_has(w, unit, "undying"))
    }
    pub(super) fn hammer_fatal(&mut self, w: &World, e: &mut health::Event) -> Result<(), String> {
        if e.prevented {
            return Ok(());
        }
        let unit = e.target;
        let Some(hammers) = self
            .equipment
            .units
            .get(&unit)
            .and_then(|loadout| loadout.hammers.as_ref())
        else {
            return Ok(());
        };
        let deployment = w.unit(unit)?.deployment.deploy_seq;
        if hammers.lock_at == Some(deployment) {
            return Ok(());
        }
        if let Some((Hammer::Undying(duration), count)) = self.hammer_effect(w, unit, 1)? {
            let hammers = self
                .equipment
                .units
                .get_mut(&unit)
                .unwrap()
                .hammers
                .as_mut()
                .unwrap();
            hammers.lock_at = Some(deployment);
            hammers.window_at = Some(deployment);
            hammers.until = w.time + duration * count as f64;
            e.prevented = true;
        }
        Ok(())
    }
    pub(super) fn hammer_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        attack: &super::super::attacks::Context,
    ) -> Result<(), String> {
        let unit = attack.attacker;
        if !self
            .roster
            .as_ref()
            .unwrap()
            .entry(unit)?
            .definition
            .melee_operator
        {
            return Ok(());
        }
        if let Some((
            Hammer::Tremble {
                probability,
                duration,
            },
            count,
        )) = self.hammer_effect(w, unit, 3)?
        {
            let probability = (probability * count as f64).min(1.);
            for &target in &attack.targets {
                if w.unit(target)?.side == health::Side::Enemy
                    && w.unit(target)?.stats.alive
                    && self.equipment_chance(probability)
                {
                    self.apply_status(
                        w,
                        r,
                        target,
                        "tremble",
                        super::super::status::Options {
                            duration,
                            source: Some(unit),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn hammer_landed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        q: &hit::Request,
    ) -> Result<(), String> {
        if q.hit.damage_type != super::super::damage::DamageType::Arts
            || q.tags.iter().any(|tag| tag == "item")
            || e.amount <= 0.
            || w.unit(e.target)?.side != health::Side::Enemy
            || w.unit(e.target)?.stats.hp <= 0.
        {
            return Ok(());
        }
        let Some(source) = e.source else {
            return Ok(());
        };
        if let Some((Hammer::Burn(scale), count)) = self.hammer_effect(w, source, 0)? {
            let mut request = super::super::element::Request::new(
                Some(source),
                Some(source),
                e.target,
                super::super::element::Element::Burn,
                e.amount * scale * count as f64,
            );
            request.damage.tags = vec!["item".into(), "item:hammer_burn".into()];
            super::super::element::apply(w, self, r, &mut request)?;
        }
        Ok(())
    }
}
