// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{reactions::Memory, Reaction},
    element, targeting,
};

impl Host {
    pub(super) fn enemy_reaction_hit(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Reaction(Reaction::Double { scale, .. }) => {
                if matches!(
                    self.content.as_ref().unwrap().reaction(id),
                    Memory::Power { powered: true, .. }
                ) {
                    q.damage.amount *= scale;
                }
            }
            Kit::Reaction(Reaction::Fear(config)) => {
                if let Some(amount) = config.steal {
                    let target = q.hit.target;
                    let active = self
                        .roster
                        .as_ref()
                        .unwrap()
                        .entry(target)?
                        .skill()
                        .is_ok_and(|s| {
                            s.bank.active
                                && s.bank.kind == crate::battle::sp::Kind::Ammo
                                && s.ammo_left > 0.
                        });
                    if active {
                        let mut skill = self.roster.as_mut().unwrap().take_skill(target)?;
                        skill.ammo_left = (skill.ammo_left - amount).max(0.);
                        let result = if skill.ammo_left <= 0. {
                            self.with_kit_buffs(w, r, |h, w| skill.end(w, h, "ammo"))
                        } else {
                            Ok(())
                        };
                        self.roster.as_mut().unwrap().restore_skill(target, skill)?;
                        result?;
                        q.cancelled = true;
                    } else if let Ok(state) = w.unit_mut(target)?.operator.tactical.snhunt_mut() {
                        if state.ammo > 0. {
                            state.ammo = (state.ammo - amount).max(0.);
                            q.cancelled = true;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_reaction_in(
        &self,
        w: &World,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        if q.cancelled || !matches!(q.hit.damage_type, DamageType::Physical | DamageType::Arts) {
            return Ok(());
        }
        let Some(source) = q.hit.source else {
            return Ok(());
        };
        if w.unit(source)?.side != health::Side::Ally {
            return Ok(());
        }
        if let Kit::Reaction(Reaction::Front { reduction, .. } | Reaction::Crowd { reduction }) =
            self.content.as_ref().unwrap().kit(q.hit.target)
        {
            if let Memory::Facing { sign, .. } =
                self.content.as_ref().unwrap().reaction(q.hit.target)
            {
                if reduction > 0.
                    && (w.unit(source)?.position[1] - w.unit(q.hit.target)?.position[1]) * sign
                        > 1e-6
                {
                    q.damage.mul *= (1. - reduction).max(0.);
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_reaction_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Kit::Reaction(Reaction::Fear(config)) = self.content.as_ref().unwrap().kit(id) else {
            return Ok(());
        };
        let hp = w.unit(id)?.stats.hp;
        if hp / Self::tac_stat(w, id, "maxHp")? >= 0.5
            || !matches!(
                self.content.as_ref().unwrap().reaction(id),
                Memory::Once { done: false }
            )
        {
            return Ok(());
        }
        self.content
            .as_mut()
            .unwrap()
            .set_reaction(id, Memory::Once { done: true });
        if config.duration > 0. {
            self.apply_status(
                w,
                r,
                id,
                "fear",
                status::Options {
                    source: Some(id),
                    duration: config.duration,
                    ..Default::default()
                },
            )?;
        }
        if config.speed > 0. && config.speed_duration > 0. {
            let mut b = buff("ab:fearRun", &[("moveMul", config.speed)]);
            b.runtime.duration = config.speed_duration;
            b.flags.insert("unblockable".into());
            self.enemy_buff(w, r, id, b)?;
        }
        Ok(())
    }
    pub(super) fn enemy_reaction_attack(
        &mut self,
        w: &mut World,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Reaction(Reaction::Double { every, .. }) => {
                if let Memory::Power { count, powered } =
                    self.content.as_ref().unwrap().reaction(id)
                {
                    let state = match event {
                        enemy::Event::BeforeAttack => Memory::Power {
                            count,
                            powered: every > 0 && count + 1 == every,
                        },
                        enemy::Event::Attack => Memory::Power {
                            count: if every == 0 || count + 1 >= every {
                                0
                            } else {
                                count + 1
                            },
                            powered,
                        },
                        _ => return Ok(()),
                    };
                    self.content.as_mut().unwrap().set_reaction(id, state);
                }
            }
            Kit::Reaction(Reaction::Shell) if event == enemy::Event::Attack => {
                let amount = Self::tac_stat(w, id, "atk")?;
                for &target in &c.targets {
                    self.enemy_schedule(
                        w,
                        Some(id),
                        3.,
                        0.,
                        1,
                        EnemyTask::Shell {
                            position: w.unit(target)?.position,
                            amount,
                        },
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn enemy_stop_rush(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        self.content.as_mut().unwrap().set_reaction(
            id,
            Memory::Rush {
                active: false,
                layers: 0,
                elapsed: 0.,
            },
        );
        r.remove(w, self, id, "ab:rush")?;
        Ok(())
    }
    pub(super) fn enemy_reaction_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Reaction(Reaction::Double { scale, .. }) => {
                let powered = matches!(
                    self.content.as_ref().unwrap().reaction(id),
                    Memory::Power { powered: true, .. }
                );
                let amount = Self::tac_stat(w, id, "atk")? * if powered { scale } else { 1. };
                let mut hit = ability_damage(Some(id), q.hit.target, amount, DamageType::Physical);
                hit.is_skill = false;
                self.enemy_damage(w, r, &mut hit)?;
            }
            Kit::Reaction(Reaction::Rush { scale, .. })
                if matches!(
                    self.content.as_ref().unwrap().reaction(id),
                    Memory::Rush { active: true, .. }
                ) =>
            {
                let amount = Self::tac_stat(w, id, "moveSpeed")? * scale;
                self.enemy_stop_rush(w, r, id)?;
                if w.unit(q.hit.target)?.stats.alive {
                    let mut hit =
                        ability_damage(Some(id), q.hit.target, amount, DamageType::Physical);
                    hit.hit.no_sp = true;
                    hit.tags.push("rush".into());
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
            Kit::Reaction(Reaction::FirstSplash { scale })
                if matches!(
                    self.content.as_ref().unwrap().reaction(id),
                    Memory::Once { done: false }
                ) =>
            {
                self.content
                    .as_mut()
                    .unwrap()
                    .set_reaction(id, Memory::Once { done: true });
                let amount = Self::tac_stat(w, id, "atk")? * scale;
                let targets = spatial::allies_in_radius(
                    w,
                    self.roster.as_ref().unwrap().ids(),
                    w.unit(q.hit.target)?.position,
                    1.,
                    None,
                    false,
                    w.unit_capacity(),
                )?;
                for target in targets {
                    if target == q.hit.target {
                        continue;
                    }
                    let mut hit = ability_damage(Some(id), target, amount, DamageType::Physical);
                    hit.ignore_select = true;
                    hit.tags.push("aoeAttack".into());
                    self.enemy_damage(w, r, &mut hit)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_reaction_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let kit = self.content.as_ref().unwrap().kit(id);
        match kit {
            Kit::Reaction(Reaction::ExposeDeath {
                radius,
                duration,
                scale,
            }) if !silenced(w, id)? => {
                if duration <= 0. {
                    return Ok(());
                }
                for target in
                    enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?
                {
                    let mut b = buff("ab:exposed", &[("dmgTakenMul", scale)]);
                    b.runtime.duration = duration;
                    b.runtime.refresh = buffs::Refresh::Extend;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, target, b)?;
                }
            }
            Kit::Reaction(Reaction::HealDeath { radius, scale }) if !silenced(w, id)? => {
                let amount = Self::tac_stat(w, id, "atk")? * scale;
                for target in self.enemy_nearby(w, id, radius)? {
                    if target != id {
                        self.support_heal(
                            w,
                            r,
                            Some(id),
                            target,
                            amount,
                            health::HealOptions::default(),
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_reaction_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Reaction,
        dt: f64,
    ) -> Result<(), String> {
        if let Reaction::Crowd { .. } = kit {
            let Memory::Facing { x, mut sign } = self.content.as_ref().unwrap().reaction(id) else {
                unreachable!()
            };
            let mut left = 0;
            let mut right = 0;
            let at = w.unit(id)?.position[1];
            for &other in self.roster.as_ref().unwrap().ids() {
                let u = w.unit(other)?;
                if !u.stats.alive
                    || !u.deployed
                    || u.hidden
                    || u.deployment.kind != crate::battle::deploy::Kind::Operator
                {
                    continue;
                }
                if u.position[1] > at + 1e-6 {
                    right += 1;
                } else if u.position[1] < at - 1e-6 {
                    left += 1;
                }
            }
            if left != right {
                sign = if right > left { 1. } else { -1. };
            }
            self.content
                .as_mut()
                .unwrap()
                .set_reaction(id, Memory::Facing { x, sign });
            return Ok(());
        }
        let interval = match kit {
            Reaction::Aura { .. } => {
                if silenced(w, id)? {
                    return Ok(());
                }
                0.5
            }
            Reaction::Blood { .. } | Reaction::Front { .. } => 1.,
            Reaction::Rush { .. } => 0.1,
            _ => return Ok(()),
        };
        if let Reaction::Front { .. } = kit {
            if let Memory::Facing { x, mut sign } = self.content.as_ref().unwrap().reaction(id) {
                let u = w.unit(id)?;
                let dx = match u
                    .blocked_by()
                    .filter(|&b| w.unit(b).is_ok_and(|u| u.stats.alive))
                {
                    Some(blocker) => w.unit(blocker)?.position[1] - u.position[1],
                    None => u.position[1] - x,
                };
                if dx.abs() > 1e-6 {
                    sign = if dx > 0. { 1. } else { -1. };
                }
                self.content.as_mut().unwrap().set_reaction(
                    id,
                    Memory::Facing {
                        x: u.position[1],
                        sign,
                    },
                );
            }
        }
        let pulses = self.content.as_mut().unwrap().pulses(id, dt, interval);
        for _ in 0..pulses {
            if !w.unit(id)?.stats.alive {
                break;
            }
            match kit {
                Reaction::Aura {
                    radius,
                    defense,
                    amount,
                } => {
                    let radius = if !defense && radius == 0. {
                        2.5
                    } else {
                        radius
                    };
                    for target in self.enemy_nearby(w, id, radius)? {
                        if target == id {
                            continue;
                        }
                        let mut b = buff(
                            if defense { "ab:defdrn" } else { "ab:mrfly" },
                            &[(if defense { "defFlat" } else { "resFlat" }, amount)],
                        );
                        b.runtime.duration = 0.6;
                        self.enemy_buff(w, r, target, b)?;
                    }
                }
                Reaction::Blood { ratio } if ratio > 0. => {
                    let amount = Self::tac_stat(w, id, "maxHp")? * ratio;
                    self.support_lose_hp(w, r, id, amount, None, false)?;
                }
                Reaction::Front { neural, .. } => {
                    if let Some(target) = w.unit(id)?.blocked_by() {
                        let amount = Self::tac_stat(w, id, "atk")? * neural;
                        self.enemy_element(w, r, id, target, element::Element::Neural, amount)?;
                    }
                }
                Reaction::Rush {
                    speed,
                    interval,
                    max,
                    ..
                } => {
                    let held = w
                        .unit(id)?
                        .stats
                        .buffs()
                        .iter()
                        .any(|b| matches!(b.runtime.status.as_deref(), Some("stun" | "bind")));
                    let Memory::Rush {
                        active,
                        mut layers,
                        mut elapsed,
                    } = self.content.as_ref().unwrap().reaction(id)
                    else {
                        unreachable!()
                    };
                    if held {
                        if active {
                            self.enemy_stop_rush(w, r, id)?;
                        }
                        continue;
                    }
                    if !active {
                        if w.unit(id)?.blocked_by().is_none() {
                            self.content.as_mut().unwrap().set_reaction(
                                id,
                                Memory::Rush {
                                    active: true,
                                    layers: 0,
                                    elapsed: 0.,
                                },
                            );
                        }
                        continue;
                    }
                    if layers >= max || interval <= 0. {
                        continue;
                    }
                    elapsed += 0.1;
                    let old = layers;
                    while elapsed >= interval - 1e-9 && layers < max && layers - old < 8 {
                        elapsed -= interval;
                        layers += 1;
                    }
                    self.content.as_mut().unwrap().set_reaction(
                        id,
                        Memory::Rush {
                            active,
                            layers,
                            elapsed,
                        },
                    );
                    if layers != old {
                        let mut b = buff("ab:rush", &[("moveMul", 1. + speed * layers as f64)]);
                        b.runtime.persist = true;
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub(super) fn enemy_shell(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Option<Id>,
        position: [f64; 2],
        amount: f64,
    ) -> Result<(), String> {
        let targets = spatial::allies_in_radius(
            w,
            self.roster.as_ref().unwrap().ids(),
            position,
            1.45,
            None,
            false,
            w.unit_capacity(),
        )?;
        for target in targets {
            if !targeting::area_selectable(w, source, target)? {
                continue;
            }
            let mut damage = ability_damage(None, target, amount, DamageType::Physical);
            damage.is_skill = false;
            damage.tags.push("shell".into());
            self.enemy_damage(w, r, &mut damage)?;
        }
        Ok(())
    }
}
