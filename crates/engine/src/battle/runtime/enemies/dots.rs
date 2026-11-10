// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{element, targeting};

pub(in crate::battle::runtime) const BLEED: u64 = 1700;
impl Host {
    pub(super) fn enemy_bleed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        amount: f64,
        duration: f64,
    ) -> Result<(), String> {
        if amount <= 0. || duration <= 0. || !w.unit(target)?.stats.alive {
            return Ok(());
        }
        let mut b = buff("ab:bleed", &[]);
        b.runtime.source = Some(source);
        b.runtime.duration = duration;
        b.runtime.interval = 1.;
        b.runtime.tick_payload = amount;
        b.runtime.visible = true;
        b.runtime.on_tick = Some(BLEED);
        r.add(
            w,
            self,
            target,
            buffs::Spec {
                buff: b,
                ..Default::default()
            }
            .retaining_unit(source),
        )?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_bleed_event(
        &mut self,
        event: buffs::Event,
        c: buffs::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let source = b.runtime.source.ok_or("Missing bleed source")?;
        let amount = b.runtime.tick_payload;
        if event != buffs::Event::Tick {
            return Err("Unexpected bleed callback".into());
        }
        let mut damage = ability_damage(Some(source), c.unit, amount, DamageType::Arts);
        damage.is_skill = false;
        damage.ignore_select = true;
        damage.tags.push("bleed".into());
        self.enemy_damage(w, r, &mut damage)
    }
    pub(super) fn enemy_dot_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        ability: Ability,
    ) -> Result<(), String> {
        let (radius, scale, burn, dodge, no_air, tag) = match ability {
            Ability::Nest { radius, .. } => {
                if w.unit(id)?.hidden {
                    return Ok(());
                }
                (radius, 1., 0., true, false, "nestPulse")
            }
            Ability::Boil {
                radius,
                scale,
                burn,
                ..
            } => (radius, scale, burn, false, true, "boil"),
            _ => return Ok(()),
        };
        for target in enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())? {
            if no_air && w.unit(target)?.motion_fly
                || !targeting::area_selectable(w, Some(id), target)?
            {
                continue;
            }
            let atk = w
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("atk");
            let mut damage = ability_damage(Some(id), target, atk * scale, DamageType::Arts);
            damage.tags.push(tag.into());
            if dodge {
                damage.dodge_mode = crate::battle::injury::DodgeMode::Normal;
            }
            self.enemy_damage(w, r, &mut damage)?;
            if burn > 0. {
                let amount = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("atk")
                    * burn;
                self.enemy_element(w, r, id, target, element::Element::Burn, amount)?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_element(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        kind: element::Element,
        amount: f64,
    ) -> Result<(), String> {
        if amount <= 0. {
            return Ok(());
        }
        let mut effect = element::Request::new(Some(source), Some(source), target, kind, amount);
        effect.damage.tags = vec!["enemyAbility".into()];
        effect.damage.attack_type = hit::AttackType::None;
        element::apply(w, self, r, &mut effect).map(|_| ())
    }
}
