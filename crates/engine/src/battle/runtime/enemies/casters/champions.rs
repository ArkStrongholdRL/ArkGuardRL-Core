// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{casters::BuffEffect, lifecycle::RebornPhase},
    element::Element,
};
pub(in crate::battle::runtime) const XI_SHIELD: u64 = 1709;
pub(in crate::battle::runtime) const FEAR_CAGE: u64 = 1710;
pub(in crate::battle::runtime) const WOLF_REBORN: u64 = 1711;
impl Host {
    fn champion_form2(&self, id: Id) -> bool {
        self.content.as_ref().unwrap().caster(id).phase == Some(RebornPhase::Revived)
    }
    pub(super) fn champion_condition(
        &mut self,
        w: &mut World,
        id: Id,
        spell: Spell,
    ) -> Result<Option<bool>, String> {
        let second = self.champion_form2(id);
        Ok(Some(match spell {
            Spell::XiShield { form2, .. } => second == form2,
            Spell::MaceBlink { form2, .. } => second == form2 && w.unit(id)?.blocked_by().is_some(),
            Spell::MaceCircle { form2, radius, .. } => {
                second == form2 && !self.caster_targets(w, id, radius)?.is_empty()
            }
            Spell::XiCross { .. } => !self.caster_targets(w, id, f64::INFINITY)?.is_empty(),
            Spell::Phantom { .. } => w.unit(id)?.blocked_by().is_some(),
            Spell::LucienAoe { .. } => !self.caster_targets(w, id, 2.)?.is_empty(),
            Spell::Fear { .. } => {
                !second
                    && w.time >= self.content.as_ref().unwrap().caster(id).wait_until
                    && !self.roster.as_ref().unwrap().ids().iter().any(|&t| {
                        w.unit(t).is_ok_and(|u| {
                            u.stats.alive
                                && u.deployed
                                && u.stats
                                    .buffs()
                                    .iter()
                                    .any(|b| b.key.as_deref() == Some("ab:fearCage"))
                        })
                    })
                    && self.caster_targets(w, id, f64::INFINITY)?.iter().any(|&t| {
                        w.unit(t).is_ok_and(|u| {
                            u.deployment.kind == crate::battle::deploy::Kind::Operator
                        })
                    })
            }
            _ => return Ok(None),
        }))
    }
    pub(super) fn champion_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
    ) -> Result<(), String> {
        match spell {
            Spell::XiShield {
                duration,
                amount,
                radius,
                scale,
                ..
            } => {
                let mut b = buff("ab:xiShield", &[]);
                b.shield = amount;
                b.runtime.duration = duration;
                b.runtime.visible = true;
                b.runtime.on_expire = Some(XI_SHIELD);
                b.runtime.source = Some(id);
                b.runtime.enemy_effect = Some(BuffEffect::XiShield { radius, scale });
                self.enemy_buff(w, r, id, b)?;
            }
            Spell::XiCross { scale } => {
                let mut targets = self.caster_targets(w, id, f64::INFINITY)?;
                targeting::sort_allies(
                    w,
                    id,
                    &mut targets,
                    targeting::Profile {
                        block_free: true,
                        ..Default::default()
                    },
                )?;
                let origin = w.unit(id)?.position;
                targets.sort_by(|&a, &b| {
                    let a = w.unit(a).unwrap().position;
                    let b = w.unit(b).unwrap().position;
                    hypot(a[1] - origin[1], a[0] - origin[0])
                        .total_cmp(&hypot(b[1] - origin[1], b[0] - origin[0]))
                });
                let mut centers = targets.first().copied().into_iter().collect::<Vec<_>>();
                if self.champion_form2(id) && targets.len() > 1 {
                    centers.push(*targets.last().unwrap());
                }
                for center in centers {
                    let tile = w.unit(center)?.deployment.tile;
                    for t in self.roster.as_ref().unwrap().ids().to_vec() {
                        let p = w.unit(t)?.deployment.tile;
                        let delta = [(p[0] - tile[0]).abs(), (p[1] - tile[1]).abs()];
                        if (delta[0] == 0 || delta[1] == 0)
                            && delta[0] + delta[1] <= 2
                            && targeting::area_selectable(w, Some(id), t)?
                        {
                            self.champion_hurt(w, r, id, t, scale, DamageType::Arts)?;
                        }
                    }
                }
            }
            Spell::MaceCircle { radius, scale, .. } => {
                self.champion_aoe(w, r, id, radius, scale, 0.)?
            }
            Spell::LucienAoe { scale, element } => {
                self.champion_aoe(w, r, id, 2., scale, element)?
            }
            Spell::MaceBlink {
                distance, scale, ..
            } => {
                let blocker = w.unit(id)?.blocked_by();
                let from = w.unit(id)?.position;
                if self.caster_blink(w, r, id, distance)?.is_some() {
                    let to = w.unit(id)?.position;
                    let delta = [to[0] - from[0], to[1] - from[1]];
                    let length = delta[0] * delta[0] + delta[1] * delta[1];
                    for t in self.roster.as_ref().unwrap().ids().to_vec() {
                        if Some(t) != blocker {
                            let p = w.unit(t)?.position;
                            let k = if length > 0. {
                                ((p[0] - from[0]) * delta[0] + (p[1] - from[1]) * delta[1]) / length
                            } else {
                                0.
                            };
                            let k = k.clamp(0., 1.);
                            if hypot(p[1] - from[1] - k * delta[1], p[0] - from[0] - k * delta[0])
                                > 0.5
                                || !targeting::area_selectable(w, Some(id), t)?
                            {
                                continue;
                            }
                        }
                        self.champion_hurt(w, r, id, t, scale, DamageType::Physical)?;
                    }
                }
            }
            Spell::Phantom { distance } => {
                let mut child =
                    self.enemy_children_spec(w, id, Arc::from("enemy_2017_csphts"), 1)?;
                // Boss summons use the battle's summon scaling. Solo Field has no global enemy scaling.
                child.clear_scaling();
                if self.caster_blink(w, r, id, distance)?.is_some() {
                    self.enemy_spawn_children(w, r, child)?;
                }
            }
            Spell::Fear {
                max,
                aspd,
                peak,
                ramp,
                ..
            } => {
                let mut targets = self.caster_targets(w, id, f64::INFINITY)?;
                targets.retain(|&t| {
                    w.unit(t).is_ok_and(|u| {
                        u.deployment.kind == crate::battle::deploy::Kind::Operator
                            && !u
                                .stats
                                .buffs()
                                .iter()
                                .any(|b| b.key.as_deref() == Some("ab:fearCage"))
                    })
                });
                self.rng.shuffle(&mut targets);
                let hp = w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")?;
                for t in targets.into_iter().take(max) {
                    let mut b = buff("ab:fearCage", &[("aspd", aspd)]);
                    b.runtime.visible = true;
                    b.runtime.interval = 1.;
                    b.runtime.source = Some(id);
                    b.runtime.on_tick = Some(FEAR_CAGE);
                    b.runtime.enemy_effect = Some(BuffEffect::Fear {
                        started: w.time,
                        hp,
                        ramp,
                        peak,
                    });
                    r.add(
                        w,
                        self,
                        t,
                        buffs::Spec {
                            buff: b,
                            mods_present: true,
                            ..Default::default()
                        }
                        .retaining_unit(id),
                    )?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn champion_aoe(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        radius: f64,
        scale: f64,
        element: f64,
    ) -> Result<(), String> {
        for t in self.champion_area(w, id, w.unit(id)?.position, radius)? {
            self.champion_hurt(w, r, id, t, scale, DamageType::Physical)?;
            let amount = Self::tac_stat(w, id, "atk")? * element;
            self.enemy_element(w, r, id, t, Element::Neural, amount)?;
        }
        Ok(())
    }
    fn champion_hurt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
        scale: f64,
        kind: DamageType,
    ) -> Result<(), String> {
        let amount = Self::tac_stat(w, id, "atk")? * scale;
        let mut q = ability_damage(Some(id), target, amount, kind);
        self.enemy_damage(w, r, &mut q)
    }
    fn champion_area(
        &mut self,
        w: &mut World,
        id: Id,
        position: [f64; 2],
        radius: f64,
    ) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for t in spatial::allies_in_radius(
            w,
            self.roster.as_ref().unwrap().ids(),
            position,
            radius,
            None,
            false,
            w.unit_capacity(),
        )? {
            if targeting::area_selectable(w, Some(id), t)? {
                out.push(t);
            }
        }
        Ok(out)
    }
    pub(in crate::battle::runtime) fn champion_buff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?;
        let source = b.runtime.source.ok_or("Missing champion buff source")?;
        let effect = b
            .runtime
            .enemy_effect
            .ok_or("Missing champion buff effect")?;
        match effect {
            BuffEffect::Vest { .. } => unreachable!("Vest has no timed callback"),
            BuffEffect::XiShield { radius, scale } => {
                if w.unit(source)?.stats.alive {
                    for t in self.champion_area(w, source, w.unit(source)?.position, radius)? {
                        self.champion_hurt(w, r, source, t, scale, DamageType::Arts)?;
                    }
                }
            }
            BuffEffect::Fear {
                started,
                ramp,
                peak,
                ..
            } => {
                if !w.unit(source)?.stats.alive || w.unit(source)?.removed {
                    r.remove_id(w, self, c.unit, c.buff)?;
                    return Ok(());
                }
                let amount = Self::tac_stat(w, c.unit, "maxHp")?
                    * peak
                    * ((w.time - started) / ramp).min(1.);
                self.support_lose_hp(w, r, c.unit, amount, Some(source), false)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn champion_rebirth_start(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Caster,
    ) -> Result<(), String> {
        let mut state = self.content.as_ref().unwrap().caster(id);
        state.reborn_at = w.time;
        self.content.as_mut().unwrap().set_caster(id, state);
        match config.passive {
            Passive::Xi { .. } => {
                r.remove(w, self, id, "ab:xiShield")?;
            }
            Passive::Wolf { .. } => {
                for t in self.roster.as_ref().unwrap().ids().to_vec() {
                    r.remove(w, self, t, "ab:fearCage")?;
                }
                r.remove(w, self, id, "ab:wolfGuard")?;
                w.unit_mut(id)?.immune.remove("stun");
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime::enemies) fn champion_rebirth_finish(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Caster,
    ) -> Result<(), String> {
        let mut state = self.content.as_ref().unwrap().caster(id);
        state.left = config
            .skills
            .map(|s| s.map(|s| s.clock.initial).unwrap_or(0.));
        state.next = 1.;
        self.content.as_mut().unwrap().set_caster(id, state);
        let mut b = match config.passive {
            Passive::Xi { atk, .. } => buff("ab:xiReborn", &[("atkPct", atk)]),
            Passive::Mace { .. } => {
                let mut b = buff("ab:stealth", &[]);
                b.flags.insert("stealth".into());
                b
            }
            Passive::Wolf { atk, bat, .. } => {
                w.unit_mut(id)?.stats.edit_base(|b| {
                    let r = b.0.get("rangeRadius").copied().unwrap_or(0.);
                    b.0.insert("rangeRadius".into(), r.max(1.25));
                });
                let bat0 = w.unit(id)?.stats.base_number("bat");
                buff(
                    "ab:wolfRage",
                    &[
                        ("atkPct", atk),
                        ("batPct", bat / if bat0 != 0. { bat0 } else { 1. }),
                    ],
                )
            }
            _ => return Ok(()),
        };
        b.runtime.persist = true;
        b.runtime.visible = !matches!(config.passive, Passive::Mace { .. });
        self.enemy_buff(w, r, id, b)
    }
    pub(super) fn champion_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Caster,
        dt: f64,
    ) -> Result<(), String> {
        match config.passive {
            Passive::Lucien { dodge, .. } => {
                let on = w.unit(id)?.blocked_by().is_none() && dodge > 0.;
                let mut state = self.content.as_ref().unwrap().caster(id);
                if state.evade != Some(on) {
                    state.evade = Some(on);
                    self.content.as_mut().unwrap().set_caster(id, state);
                    if on {
                        let mut b =
                            buff("boss:evade", &[("dodgePhys", dodge), ("dodgeArts", dodge)]);
                        b.runtime.persist = true;
                        self.enemy_buff(w, r, id, b)?;
                    } else {
                        r.remove(w, self, id, "boss:evade")?;
                    }
                }
            }
            Passive::Wolf { .. } => {
                let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.1);
                for _ in 0..pulses {
                    if self.champion_form2(id) {
                        self.wolf_awe(w, r, id)?;
                        continue;
                    }
                    let Some((slot, skill)) =
                        config.skills.iter().enumerate().find_map(|(i, s)| {
                            s.map(|s| (i, s))
                                .filter(|(_, s)| matches!(s.spell, Spell::Fear { .. }))
                        })
                    else {
                        continue;
                    };
                    let Spell::Fear {
                        cut, refund, wait, ..
                    } = skill.spell
                    else {
                        unreachable!()
                    };
                    let hp = w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")?;
                    let mut n = 0;
                    for t in self.roster.as_ref().unwrap().ids().to_vec() {
                        if !w.unit(t)?.stats.alive || !w.unit(t)?.deployed {
                            continue;
                        }
                        let b = w.unit(t)?.stats.buffs().iter().find(|b| {
                            b.key.as_deref() == Some("ab:fearCage") && b.runtime.source == Some(id)
                        });
                        if let Some(b) = b {
                            let Some(BuffEffect::Fear { hp: at, .. }) = b.runtime.enemy_effect
                            else {
                                unreachable!()
                            };
                            if hp <= at - cut + 1e-9 {
                                r.remove(w, self, t, "ab:fearCage")?;
                            } else {
                                n += 1;
                            }
                        }
                    }
                    let mut state = self.content.as_ref().unwrap().caster(id);
                    if n < state.caught {
                        state.left[slot] -= refund * (state.caught - n) as f64;
                        if n == 0 {
                            state.wait_until = w.time + wait;
                        }
                    }
                    if n > 0 || w.time < state.wait_until {
                        state.left[slot] += 0.1;
                    }
                    state.caught = n;
                    self.content.as_mut().unwrap().set_caster(id, state);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn wolf_reborn(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some(duration) = self.caster_config(id).and_then(|c| match c.passive {
            Passive::Wolf { duration, .. } => Some(duration),
            _ => None,
        }) else {
            return Ok(());
        };
        let state = self.content.as_ref().unwrap().caster(id);
        w.unit_mut(id)?.stats.hp = (Self::tac_stat(w, id, "maxHp")?
            * ((w.time - state.reborn_at) / duration.max(DT)).min(1.))
        .max(1.);
        self.wolf_awe(w, r, id)
    }
    fn wolf_awe(&mut self, w: &mut World, r: &mut buffs::Runtime, id: Id) -> Result<(), String> {
        for t in spatial::allies_in_radius(
            w,
            self.roster.as_ref().unwrap().ids(),
            w.unit(id)?.position,
            1.5,
            None,
            false,
            w.unit_capacity(),
        )? {
            if !targeting::aura_selectable(w, t)? {
                continue;
            }
            let mut b = buff("ab:ancientAwe", &[("aspd", -50.)]);
            b.runtime.duration = 0.2 + 0.1;
            b.runtime.visible = true;
            self.enemy_buff(w, r, t, b)?;
        }
        Ok(())
    }
    pub(super) fn champion_hit(
        &self,
        w: &World,
        id: Id,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        if let Some(config) = self.caster_config(id) {
            if let Passive::Mace { pen, .. } = &config.passive {
                if w.unit(id)?.blocked_by() == Some(q.hit.target) {
                    q.damage.penetration.def_pct = (q.damage.penetration.def_pct
                        + pen[usize::from(self.champion_form2(id))])
                    .min(1.);
                }
            }
        }
        Ok(())
    }
    pub(super) fn champion_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        q: &hit::Request,
    ) -> Result<(), String> {
        let (element, rebirth) = match self.caster_config(id) {
            Some(config) => (
                match &config.passive {
                    Passive::Lucien { element, .. } => Some(*element),
                    _ => None,
                },
                config.rebirth().is_some(),
            ),
            None => return Ok(()),
        };
        if let Some(element) = element {
            let amount = Self::tac_stat(w, id, "atk")? * element;
            self.enemy_element(w, r, id, q.hit.target, Element::Neural, amount)?;
        }
        if rebirth && self.champion_form2(id) && w.unit(q.hit.target)?.stats.alive {
            let amount = Self::tac_stat(w, id, "atk")?;
            let mut hit = ability_damage(Some(id), q.hit.target, amount, q.hit.damage_type);
            hit.is_skill = false;
            hit.dodge_mode = crate::battle::injury::DodgeMode::Normal;
            hit.tags.push("secondHit".into());
            self.champion_hit(w, id, &mut hit)?;
            self.enemy_damage(w, r, &mut hit)?;
        }
        Ok(())
    }
    pub(super) fn champion_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Some((cut, disarm, radius)) = self.caster_config(id).and_then(|c| match c.passive {
            Passive::Mace {
                cut,
                disarm,
                radius,
                ..
            } => Some((cut, disarm, radius)),
            _ => None,
        }) else {
            return Ok(());
        };
        if !self.champion_form2(id) || cut <= 0. {
            return Ok(());
        }
        let hp = w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")?;
        for _ in 0..64 {
            // bounded crossings of one HP bar
            let mut state = self.content.as_ref().unwrap().caster(id);
            if !w.unit(id)?.stats.alive || hp > state.next - cut + 1e-9 || state.next - cut <= 1e-9
            {
                break;
            }
            let previous = state.next;
            state.next -= cut;
            if state.next == previous {
                break;
            }
            self.content.as_mut().unwrap().set_caster(id, state);
            for t in self.champion_area(w, id, w.unit(id)?.position, radius)? {
                self.roster.as_mut().unwrap().clear_inactive_sp(w, t)?;
                if disarm > 0. {
                    self.apply_status(
                        w,
                        r,
                        t,
                        "disarm",
                        status::Options {
                            duration: disarm,
                            source: Some(id),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
}
