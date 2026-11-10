// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{content::enemies::Family, element, targeting};
use crate::deterministic::math::hypot;

impl Host {
    pub(super) fn liberate_prisoner(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<bool, String> {
        if !w.unit(id)?.stats.alive {
            return Ok(false);
        }
        let Some(p) = self.content.as_mut().unwrap().free_prisoner(id) else {
            return Ok(false);
        };
        r.remove(w, self, id, "ab:confined")?;
        let mut b = buff(
            "ab:liberty",
            &[
                ("atkPct", p.atk),
                ("defIgnorePct", p.penetration),
                ("resFlat", p.res),
                ("hpRegen", p.regen),
            ],
        );
        b.runtime.persist = true;
        b.runtime.visible = true;
        self.enemy_buff(w, r, id, b)?;
        Ok(p.free_all && self.content.as_mut().unwrap().free_all_once())
    }
    pub(super) fn enemy_family_attack(
        &mut self,
        event: enemy::Event,
        c: &mut enemy::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(c.enemy) {
            Kit::Family(Family::Prisoner(_)) if event == enemy::Event::BeforeAttack => {
                if self.content.as_mut().unwrap().prisoner_attack(c.enemy)
                    && self.liberate_prisoner(w, r, c.enemy)?
                {
                    let ids = self.spawns.as_ref().unwrap().live().to_vec();
                    for id in ids {
                        self.liberate_prisoner(w, r, id)?;
                    }
                }
            }
            Kit::Family(Family::ShadowKiller { .. }) if event == enemy::Event::Attack => {
                self.content.as_mut().unwrap().shadow_spent(c.enemy)
            }
            Kit::Ability(Ability::Tide { .. }) | Kit::Split(content::Split::Boat { .. })
                if event == enemy::Event::BeforeAttack =>
            {
                let Some(&first) = c.targets.first() else {
                    return Ok(());
                };
                let tile = w.unit(first)?.deployment.tile;
                let ids = self.roster.as_ref().unwrap().ids().to_vec();
                for id in ids {
                    let at = w.unit(id)?.deployment.tile;
                    if (at[0] - tile[0]).abs() + (at[1] - tile[1]).abs() <= 1
                        && !c.targets.contains(&id)
                        && targeting::area_selectable(w, Some(c.enemy), id)?
                    {
                        c.targets.push(id);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_content_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
    ) -> Result<(), String> {
        if let Some(source) = q.hit.source {
            if let Kit::Ability(Ability::Nest { ratio, .. }) =
                self.content.as_ref().unwrap().kit(source)
            {
                if w.unit(source)?.stats.alive
                    && q.tags.iter().any(|t| t == "nestPulse")
                    && q.hit.damage_type != DamageType::Elemental
                    && ratio > 0.
                {
                    let amount = w
                        .unit_mut(source)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .n("atk")
                        * ratio;
                    self.enemy_element(
                        w,
                        r,
                        source,
                        q.hit.target,
                        element::Element::Neural,
                        amount,
                    )?;
                }
            }
        }
        if !q.is_attack
            || q.hit.damage_type == DamageType::Elemental
            || w.unit(q.hit.target)?.side != health::Side::Ally
        {
            return Ok(());
        }
        let Some(id) = q.hit.source else {
            return Ok(());
        };
        self.enemy_boss_dealt(w, r, id, q.hit.target)?;
        self.enemy_special_dealt(w, r, q, id)?;
        self.enemy_caster_dealt(w, r, q, id)?;
        self.enemy_leader_dealt(w, r, q, id)?;
        self.enemy_lifecycle_dealt(w, r, q, id)?;
        self.enemy_reaction_dealt(w, r, q, id)?;
        self.enemy_zone_dealt(w, q, id)?;
        if let Kit::Ability(Ability::Bleed { amount, duration }) =
            self.content.as_ref().unwrap().kit(id)
        {
            self.enemy_bleed(w, r, id, q.hit.target, amount, duration)?;
        }
        if let Kit::Family(Family::Tear { stun, .. }) = self.content.as_ref().unwrap().kit(id) {
            if stun > 0. {
                self.apply_status(
                    w,
                    r,
                    q.hit.target,
                    "stun",
                    status::Options {
                        source: Some(id),
                        duration: stun,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_nearby(
        &mut self,
        w: &mut World,
        id: Id,
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        spatial::enemies_in_radius(
            w,
            self,
            &ids,
            w.unit(id)?.position,
            radius,
            RadiusRule::Body,
            false,
            w.unit_capacity(),
        )
    }
    pub(super) fn enemy_family_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        family: Family,
        dt: f64,
    ) -> Result<(), String> {
        if let Family::ShadowKiller { .. } = family {
            let on = targeting::enemy_stealthed(w, id)?;
            self.content.as_mut().unwrap().shadow_tick(id, on);
            return Ok(());
        }
        if matches!(family, Family::Prisoner(_) | Family::Tear { .. }) {
            return Ok(());
        }
        let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.5);
        for _ in 0..pulses {
            match family {
                Family::Phalanx { radius, def, .. } => {
                    let key = self.spawns.as_ref().unwrap().definition(id)?.key.clone();
                    let mut n = 0;
                    for other in self.enemy_nearby(w, id, radius)? {
                        if other != id
                            && self.spawns.as_ref().unwrap().definition(other)?.key == key
                        {
                            n += 1;
                        }
                    }
                    if n > 0 {
                        let mut b = buff("ab:phalanx", &[("defFlat", def * n as f64)]);
                        b.runtime.duration = 0.6;
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
                Family::HolyGuard { radius, aspd, .. } => {
                    let mut on = false;
                    for other in self.enemy_nearby(w, id, radius)? {
                        if self
                            .spawns
                            .as_ref()
                            .unwrap()
                            .definition(other)?
                            .key
                            .contains("enemy_1175_dushdo")
                        {
                            on = true;
                            break;
                        }
                    }
                    if !on {
                        continue;
                    }
                    let targets =
                        enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?;
                    for target in targets {
                        if !targeting::aura_selectable(w, target)? {
                            continue;
                        }
                        let mut b = buff("ab:forceField", &[("aspd", aspd)]);
                        b.runtime.duration = 0.6;
                        b.runtime.visible = true;
                        self.enemy_buff(w, r, target, b)?;
                    }
                }
                Family::ShadowBlade { bat } => {
                    let mut partner = None;
                    for other in self.enemy_nearby(w, id, 3.)? {
                        if self
                            .spawns
                            .as_ref()
                            .unwrap()
                            .definition(other)?
                            .key
                            .contains("enemy_1174_duholy")
                        {
                            partner = Some(other);
                            break;
                        }
                    }
                    let Some(partner) = partner else {
                        continue;
                    };
                    let radius = match self.content.as_ref().unwrap().kit(partner) {
                        Kit::Family(Family::HolyGuard { radius, .. }) => radius,
                        _ => 1.1,
                    };
                    let a = w.unit(id)?.position;
                    let b = w.unit(partner)?.position;
                    let base = w.unit(id)?.stats.base_number("bat");
                    if base > 0. && hypot(a[0] - b[0], a[1] - b[1]) <= radius + 1e-9 {
                        let mut b = buff("ab:shadowSync", &[("batPct", bat / base)]);
                        b.runtime.duration = 0.6;
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }
    pub(super) fn enemy_element_pulse(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        radius: f64,
        ratio: f64,
    ) -> Result<(), String> {
        let targets = enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?;
        for target in targets {
            if !targeting::area_selectable(w, Some(id), target)? {
                continue;
            }
            let amount = w
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("atk")
                * ratio;
            if amount <= 0. {
                continue;
            }
            let mut effect =
                element::Request::new(Some(id), Some(id), target, element::Element::Neural, amount);
            effect.damage.attack_type = hit::AttackType::None;
            effect.damage.tags = vec!["enemyAbility".into()];
            element::apply(w, self, r, &mut effect)?;
        }
        Ok(())
    }
}
