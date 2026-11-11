// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
impl Host {
    pub(super) fn field_terrain(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        fresh: bool,
    ) -> Result<(), String> {
        if self.devices.elevated(w.unit(id)?.deployment.tile)
            && w.unit(id)?.side == health::Side::Ally
        {
            w.unit_mut(id)?.ground = false;
        }
        Arc::clone(&self.terrain).refresh(w, r, self, id, fresh)
    }
    pub(super) fn refresh_terrain(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if !self.terrain.active() {
            return Ok(());
        }
        let ids: Vec<_> = self
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .chain(self.spawns.as_ref().unwrap().live())
            .copied()
            .collect();
        for id in ids {
            self.field_terrain(w, r, id, false)?;
        }
        Ok(())
    }
    pub(super) fn terrain_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
        deep: bool,
    ) -> Result<(), String> {
        let amount = r.get(w, c.unit, c.buff)?.runtime.tick_payload;
        if amount <= 0. || !w.unit(c.unit)?.stats.alive {
            return Ok(());
        }
        let mut q = hit::Request::new(
            health::Hit {
                source: None,
                credit: None,
                target: c.unit,
                damage_type: super::super::damage::DamageType::True,
                no_sp: deep,
                silent: false,
                hp_loss: false,
                on_applied: false,
            },
            super::super::damage::Damage {
                amount,
                mul: 1.,
                resolved: false,
                penetration: Default::default(),
            },
        );
        q.dodge_mode = super::super::injury::DodgeMode::Ignore;
        q.tags = if deep {
            vec!["dot".into(), "periodic".into(), "deepsea".into()]
        } else {
            vec!["terrain".into()]
        };
        self.enemy_damage(w, r, &mut q)?;
        Ok(())
    }
}
