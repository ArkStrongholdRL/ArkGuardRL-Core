// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{leaders::Memory, Leader},
    targeting,
};
use crate::deterministic::math::hypot;
pub(in crate::battle::runtime) const LANDING: u64 = 1702;
impl Host {
    pub(super) fn enemy_tile_allies(
        &self,
        w: &World,
        tile: [i32; 2],
        reach: i32,
    ) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for &id in self.roster.as_ref().unwrap().ids() {
            let u = w.unit(id)?;
            let at = u.deployment.tile;
            if u.stats.alive
                && u.deployed
                && !u.hidden
                && (at[0] - tile[0]).abs() <= reach
                && (at[1] - tile[1]).abs() <= reach
            {
                out.push(id);
            }
        }
        Ok(out)
    }
    pub(super) fn enemy_skull_debuff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        target: Id,
        defense: f64,
    ) -> Result<(), String> {
        if defense != 0. {
            self.apply_status(
                w,
                r,
                target,
                "defDown",
                status::Options {
                    source: Some(source),
                    duration: 5.,
                    value: Some(defense),
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn enemy_leader_hit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Leader(Leader::Skull { defense, .. })
                if matches!(
                    self.content.as_ref().unwrap().leader(id),
                    Memory::Skull { splash: true, .. }
                ) =>
            {
                let amount = Self::tac_stat(w, id, "atk")? * 0.26;
                for target in self.enemy_tile_allies(w, w.unit(q.hit.target)?.deployment.tile, 1)? {
                    if target == q.hit.target || !targeting::area_selectable(w, Some(id), target)? {
                        continue;
                    }
                    let mut hit = ability_damage(Some(id), target, amount, DamageType::Physical);
                    hit.tags.push("splash".into());
                    self.enemy_damage(w, r, &mut hit)?;
                    self.enemy_skull_debuff(w, r, id, target, defense)?;
                }
            }
            Kit::Leader(Leader::Demon { scale, .. }) => {
                let pos = w.unit(id)?.position.map(|x| (x + 0.5).floor() as i32);
                let tile = w.unit(q.hit.target)?.deployment.tile;
                if (tile[0] - pos[0]).abs() <= 1 && (tile[1] - pos[1]).abs() <= 1 {
                    q.damage.amount *= scale;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_leader_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        if let Kit::Leader(Leader::Skull { defense, .. }) = self.content.as_ref().unwrap().kit(id) {
            if matches!(
                self.content.as_ref().unwrap().leader(id),
                Memory::Skull { splash: true, .. }
            ) {
                self.enemy_skull_debuff(w, r, id, q.hit.target, defense)?;
            }
        }
        Ok(())
    }
    pub(super) fn enemy_leader_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        amount: f64,
    ) -> Result<(), String> {
        let id = q.hit.target;
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Leader(Leader::Skull { ratio, atk, .. }) => {
                let Memory::Skull { splash, enraged } = self.content.as_ref().unwrap().leader(id)
                else {
                    unreachable!()
                };
                let hp = w.unit(id)?.stats.hp;
                if enraged || hp / Self::tac_stat(w, id, "maxHp")? >= ratio {
                    return Ok(());
                }
                self.content.as_mut().unwrap().set_leader(
                    id,
                    Memory::Skull {
                        splash,
                        enraged: true,
                    },
                );
                let mut b = buff("ab:lowhp", &[("atkPct", atk)]);
                b.runtime.persist = true;
                b.runtime.visible = true;
                self.enemy_buff(w, r, id, b)?;
            }
            Kit::Leader(Leader::Vines {
                duration, scale, ..
            }) if duration > 0. && amount > 0. && q.tags.iter().any(|t| t == "terrain") => {
                let mut b = buff("ab:natureWeak", &[("dmgTakenMul", scale)]);
                b.runtime.duration = duration;
                b.runtime.visible = true;
                self.enemy_buff(w, r, id, b)?;
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_leader_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Leader(Leader::Skull { .. }) if event == enemy::Event::BeforeAttack => {
                let Memory::Skull { enraged, .. } = self.content.as_ref().unwrap().leader(id)
                else {
                    unreachable!()
                };
                self.content.as_mut().unwrap().set_leader(
                    id,
                    Memory::Skull {
                        enraged,
                        splash: w.unit(id)?.blocked_by().is_none(),
                    },
                );
            }
            Kit::Leader(Leader::Axe { atk, aspd, max, .. }) if event == enemy::Event::Attack => {
                self.content
                    .as_mut()
                    .unwrap()
                    .set_leader(id, Memory::Axe { last: Some(w.time) });
                let mut b = buff("ab:axeStack", &[("atkPct", atk), ("aspd", aspd)]);
                b.runtime.persist = true;
                b.runtime.refresh = buffs::Refresh::Stack;
                b.runtime.max_stacks = max;
                self.enemy_buff(w, r, id, b)?;
            }
            Kit::Leader(Leader::Vines { every, stun, .. }) if event == enemy::Event::Attack => {
                let Memory::Vines { count } = self.content.as_ref().unwrap().leader(id) else {
                    unreachable!()
                };
                let next = count.saturating_add(1);
                let fire = every > 0 && next >= every;
                self.content.as_mut().unwrap().set_leader(
                    id,
                    Memory::Vines {
                        count: if fire { 0 } else { next },
                    },
                );
                if !fire || stun <= 0. {
                    return Ok(());
                }
                for &target in &c.targets {
                    let u = w.unit(target)?;
                    if u.stats.alive && c.deployments.contains(&(target, u.deployment.deploy_seq)) {
                        self.apply_status(
                            w,
                            r,
                            target,
                            "stun",
                            status::Options {
                                duration: stun,
                                source: Some(id),
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_leader_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Leader,
        dt: f64,
    ) -> Result<(), String> {
        match kit {
            Leader::Axe { delay, .. } => {
                let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.5);
                for _ in 0..pulses {
                    if let Memory::Axe { last: Some(last) } =
                        self.content.as_ref().unwrap().leader(id)
                    {
                        if w.time - last >= delay {
                            self.content
                                .as_mut()
                                .unwrap()
                                .set_leader(id, Memory::Axe { last: None });
                            r.remove(w, self, id, "ab:axeStack")?;
                        }
                    }
                }
            }
            Leader::Demon { aspd, .. } => {
                let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.5);
                for _ in 0..pulses {
                    let tile = w.unit(id)?.position.map(|x| (x + 0.5).floor() as i32);
                    for target in self.enemy_tile_allies(w, tile, 1)? {
                        let mut b = buff("ab:blackFog", &[("aspd", aspd)]);
                        b.runtime.duration = 0.6;
                        b.runtime.visible = true;
                        self.enemy_buff(w, r, target, b)?;
                    }
                }
            }
            Leader::Balloon {
                skill: Some(skill),
                duration,
                speed,
            } => {
                let Memory::Balloon { mut left } = self.content.as_ref().unwrap().leader(id) else {
                    unreachable!()
                };
                let castable =
                    self.enemy_can_cast(w, id, false)? && w.unit(id)?.blocked_by().is_some();
                let cast = skill.advance(&mut left, dt, castable);
                self.content
                    .as_mut()
                    .unwrap()
                    .set_leader(id, Memory::Balloon { left });
                if cast {
                    let mut b = buff("ab:takeoff", &[("moveMul", 1. + speed)]);
                    b.runtime.duration = duration;
                    b.runtime.visible = true;
                    b.runtime.on_expire = Some(LANDING);
                    b.flags
                        .extend(["unblockable", "float", "noDisplace"].map(str::to_string));
                    self.enemy_buff(w, r, id, b)?;
                    let mut b = buff("ab:takeoffBrake", &[("moveMul", 0.1)]);
                    b.runtime.duration = 1.5;
                    self.enemy_buff(w, r, id, b)?;
                }
            }
            Leader::Spin {
                skill,
                duration,
                interval,
                radius,
                scale,
            } => {
                let Memory::Spin {
                    mut left,
                    mut until,
                    previous,
                    hidden,
                    ..
                } = self.content.as_ref().unwrap().leader(id)
                else {
                    unreachable!()
                };
                let flags = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .clone();
                if !flags.contains("silence") {
                    let pulses = self.content.as_mut().unwrap().pulses(id, dt, interval);
                    for _ in 0..pulses {
                        if until <= w.time
                            || flags.contains("stun")
                            || flags.contains("sleep")
                            || flags.contains("disarm")
                        {
                            continue;
                        }
                        let amount = Self::tac_stat(w, id, "atk")? * scale;
                        for target in
                            enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?
                        {
                            if w.unit(target)?.motion_fly
                                || !targeting::area_selectable(w, Some(id), target)?
                            {
                                continue;
                            }
                            let mut hit =
                                ability_damage(Some(id), target, amount, DamageType::Physical);
                            self.enemy_damage(w, r, &mut hit)?;
                        }
                    }
                }
                let castable = self.enemy_can_cast(w, id, true)?;
                if skill.is_some_and(|s| s.advance(&mut left, dt, castable)) {
                    until = w.time + duration;
                }
                let no_attack = until > w.time;
                let u = w.unit(id)?;
                let position = u.position;
                let now_hidden = u.hidden;
                if let Some(p) = previous.filter(|_| !hidden && !now_hidden) {
                    let speed = Self::tac_stat(w, id, "moveSpeed")?;
                    if hypot(position[1] - p[1], position[0] - p[0])
                        - (speed * 0.5 * dt * 1.5 + 1e-3)
                        > 0.05
                        && until > w.time
                    {
                        until = 0.;
                    }
                }
                self.content.as_mut().unwrap().set_leader(
                    id,
                    Memory::Spin {
                        left,
                        until,
                        no_attack,
                        previous: Some(position),
                        hidden: now_hidden,
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_landing(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if w.unit(id)?.stats.alive {
            let mut b = buff("ab:landing", &[("moveMul", 0.1)]);
            b.runtime.duration = 1.333;
            b.flags.insert("unblockable".into());
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_leader_profile(
        &self,
        id: Id,
        profile: &mut enemy::Profile,
    ) {
        match self.content.as_ref().unwrap().leader(id) {
            Memory::Skull { splash, .. } => profile.atk_scale = if splash { 0.26 } else { 1. },
            Memory::Spin { no_attack, .. } => profile.no_attack = no_attack,
            _ => {}
        }
    }
}
