// SPDX-License-Identifier: GPL-3.0-or-later
// Enemy content uses the same settlement, status, selection and buff runtimes as operators.
use super::*;
mod bosses;
mod casters;
pub(super) use bosses::{BossTask, BLADE_RETURN, PART_DOT, SURGE_TICK};
mod catapult;
mod dots;
mod specials;
mod terrain;
pub(super) use casters::{
    SkillShot, SpellTask, ARTS_BARRIER, BURN_TICK, FEAR_CAGE, SAND_TICK, WOLF_REBORN, XI_SHIELD,
};
pub(super) use specials::{BombShot, Task as SpecialTask, PARASITE_TICK, TRANSLATOR_FINISH};
mod leaders;
mod supporters;
pub(super) use leaders::LANDING;
mod lifecycle;
mod rebirth;
pub(super) use rebirth::PILOT_BURN;
mod movement;
pub(super) use lifecycle::REBORN_FINISH;
mod forms;
mod reactions;
mod zones;
pub(super) use zones::{CloudShot, EnemyTask};
mod families;
pub(super) use dots::BLEED;
mod splits;
use crate::battle::{
    content::enemies::{Ability, Kit},
    damage::{Damage, DamageType, Penetration},
    stats::Buff,
    status,
};

fn silenced(w: &mut World, id: Id) -> Result<bool, String> {
    Ok(w.unit_mut(id)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .contains("silence"))
}
fn buff(key: &str, mods: &[(&str, f64)]) -> Buff {
    let mut b = Buff::default();
    b.key = Some(key.into());
    b.mods = mods.iter().map(|&(k, v)| (k.into(), v)).collect();
    b
}
fn ability_damage(source: Option<Id>, target: Id, amount: f64, kind: DamageType) -> hit::Request {
    let mut request = hit::Request::new(
        health::Hit {
            source,
            credit: source,
            target,
            damage_type: kind,
            no_sp: false,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount,
            mul: 1.,
            resolved: false,
            penetration: Penetration::default(),
        },
    );
    request.is_skill = true;
    request.dodge_mode = crate::battle::injury::DodgeMode::Ignore;
    request.tags = vec!["enemyAbility".into()];
    request
}
impl Host {
    fn enemy_buff(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        b: Buff,
    ) -> Result<(), String> {
        r.add(
            w,
            self,
            id,
            buffs::Spec {
                mods_present: !b.mods.is_empty(),
                buff: b,
                ..Default::default()
            },
        )?;
        Ok(())
    }
    pub(super) fn enemy_content_hit(
        &mut self,
        w: &mut World,
        q: &mut hit::Request,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        if q.cancelled || !q.is_attack {
            return Ok(());
        }
        let Some(id) = q.hit.source else {
            return Ok(());
        };
        self.enemy_boss_hit(id, q);
        self.enemy_special_hit(w, q, id)?;
        self.enemy_terrain_hit(w, q, id)?;
        self.enemy_rebirth_hit(w, r, q, id)?;
        self.enemy_caster_hit(w, r, q, id)?;
        self.enemy_leader_hit(w, r, q, id)?;
        self.enemy_supporter_hit(w, q, id)?;
        self.enemy_reaction_hit(w, r, q, id)?;
        if matches!(
            self.content.as_ref().unwrap().kit_ref(id),
            Kit::Split(content::Split::Teapot { .. }) | Kit::Zone(content::ZoneKit::Cannon { .. })
        ) {
            let centre = w.unit(q.hit.target)?.position;
            let targets = spatial::allies_in_radius(
                w,
                self.roster.as_ref().unwrap().ids(),
                centre,
                1.,
                None,
                false,
                w.unit_capacity(),
            )?;
            for target in targets {
                if target == q.hit.target
                    || w.unit(target)?.motion_fly
                    || !crate::battle::targeting::area_selectable(w, Some(id), target)?
                {
                    continue;
                }
                let amount = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("atk");
                let mut splash = ability_damage(Some(id), target, amount, q.hit.damage_type);
                splash.tags.push("splash".into());
                splash.dodge_mode = crate::battle::injury::DodgeMode::Normal;
                self.enemy_damage(w, r, &mut splash)?;
            }
        }
        if let Kit::Zone(content::ZoneKit::Tank(tank)) = self.content.as_ref().unwrap().kit_ref(id)
        {
            if !self.content.as_ref().unwrap().tank_skill(id)
                && w.unit(id)?.blocked_by() == Some(q.hit.target)
            {
                q.damage.amount *= tank.melee;
            }
        }
        match self.content.as_ref().unwrap().kit_ref(id) {
            Kit::Family(content::Family::ShadowKiller { scale })
                if self.content.as_ref().unwrap().shadow_power(id) =>
            {
                q.damage.amount *= scale;
            }
            Kit::Ability(Ability::FrozenPower(scale)) => {
                if w.unit_mut(q.hit.target)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("freeze")
                {
                    q.damage.amount *= scale;
                }
            }
            Kit::Ability(Ability::Power { scale, .. })
                if self.content.as_ref().unwrap().triggered(id) =>
            {
                q.damage.amount *= scale
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_content_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        amount: f64,
    ) -> Result<(), String> {
        let id = q.hit.target;
        if q.hit.damage_type != DamageType::Elemental {
            self.helm_taken(w, r, id, q.hit.source, q.hit.credit, amount)?;
        }
        if q.hit.damage_type != DamageType::Elemental {
            self.echo_taken(w, r, id, q.hit.source, q.hit.credit)?;
        }
        self.enemy_rebirth_taken(w, r, id)?;
        self.enemy_caster_taken(w, r, id)?;
        self.enemy_leader_taken(w, r, q, amount)?;
        self.enemy_supporter_taken(w, r, id)?;
        self.enemy_movement_taken(w, r, id)?;
        self.enemy_reaction_taken(w, r, id)?;
        let Kit::Ability(ability) = self.content.as_ref().unwrap().kit_ref(id) else {
            return Ok(());
        };
        let ability = *ability;
        let b = match ability {
            Ability::Nucleus { speed, .. } => {
                if !self.content.as_mut().unwrap().trigger_once(id) {
                    return Ok(());
                }
                let mut b = buff("ab:combat", &[("moveMul", speed)]);
                b.runtime.persist = true;
                b.runtime.visible = true;
                b
            }
            Ability::Expose { duration, scale } => {
                if duration <= 0. || silenced(w, id)? {
                    return Ok(());
                }
                let Some(source) = q.hit.source else {
                    return Ok(());
                };
                let u = w.unit(source)?;
                if !u.stats.alive
                    || u.side != health::Side::Ally
                    || u.deployment.kind == crate::battle::deploy::Kind::Device
                {
                    return Ok(());
                }
                let mut b = buff("ab:exposed", &[("dmgTakenMul", scale)]);
                b.runtime.duration = duration;
                b.runtime.refresh = buffs::Refresh::Extend;
                b.runtime.visible = true;
                return self.enemy_buff(w, r, source, b);
            }
            Ability::LowHp(atk) => {
                let s = &mut w.unit_mut(id)?.stats;
                let hp = s.hp;
                if hp / s.stats().map_err(str::to_string)?.n("maxHp") >= 0.5
                    || !self.content.as_mut().unwrap().trigger_once(id)
                {
                    return Ok(());
                }
                let mut b = buff("ab:lowhp", &[("atkPct", atk)]);
                b.runtime.persist = true;
                b.runtime.visible = true;
                b
            }
            Ability::Run(speed) if speed > 0. => {
                if !self.content.as_mut().unwrap().trigger_once(id) {
                    return Ok(());
                }
                let mut b = buff("ab:duckRun", &[("moveMul", 1. + speed)]);
                b.runtime.persist = true;
                b
            }
            Ability::DefDecay { max, def, res } if max > 0 => {
                let mut b = buff(
                    "ab:defDecay",
                    &[("defFlat", def / max as f64), ("resFlat", res / max as f64)],
                );
                b.runtime.persist = true;
                b.runtime.refresh = buffs::Refresh::Stack;
                b.runtime.max_stacks = max;
                b
            }
            Ability::Song {
                every,
                duration,
                aspd,
                ..
            } => {
                if !self.content.as_mut().unwrap().counted(id, every) {
                    return Ok(());
                }
                let ids = self.spawns.as_ref().unwrap().live().to_vec();
                for target in ids {
                    if !w.unit(target)?.stats.alive {
                        continue;
                    }
                    let mut b = buff("ab:songOfWar", &[("aspd", aspd)]);
                    b.runtime.duration = duration;
                    b.runtime.refresh = buffs::Refresh::Extend;
                    b.runtime.visible = true;
                    self.enemy_buff(w, r, target, b)?;
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        self.enemy_buff(w, r, id, b)
    }
    pub(super) fn enemy_content_attack(
        &mut self,
        event: enemy::Event,
        c: &mut enemy::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
        grid: Option<&Grid>,
    ) -> Result<(), String> {
        let id = c.enemy;
        self.enemy_catapult_attack(w, r, c, event)?;
        if self.enemy_special_attack(w, r, c, event)? {
            return Ok(());
        }
        if self.enemy_caster_attack(w, r, c, event, grid)? {
            return Ok(());
        }
        self.helm_attack(w, c, event)?;
        self.quintus_attack(w, c, event)?;
        self.gun_attack(w, r, c, event)?;
        self.enemy_terrain_attack(w, r, c, event)?;
        self.enemy_leader_attack(w, r, c, event)?;
        self.enemy_supporter_attack(w, r, c.enemy, event)?;
        self.enemy_movement_attack(w, c, event)?;
        self.enemy_lifecycle_attack(w, r, c, event)?;
        self.enemy_reaction_attack(w, c, event)?;
        self.enemy_zone_attack(w, c, event)?;
        self.enemy_family_attack(event, c, r, w)?;
        if event == enemy::Event::Attack {
            self.enemy_split_attack(c, r, w)?;
        }
        if event == enemy::Event::BeforeAttack {
            self.content.as_mut().unwrap().before_attack(id);
            return Ok(());
        }
        let ability = match self.content.as_ref().unwrap().kit_ref(id) {
            Kit::Ability(ability) => Some(*ability),
            _ => None,
        };
        match ability {
            Some(Ability::Stun { every, duration }) => {
                if silenced(w, id)?
                    || !self.content.as_mut().unwrap().counted(id, every)
                    || duration <= 0.
                {
                    return Ok(());
                }
                for &target in &c.targets {
                    let u = w.unit(target)?;
                    if !u.stats.alive || !c.deployments.contains(&(target, u.deployment.deploy_seq))
                    {
                        continue;
                    }
                    self.apply_status(
                        w,
                        r,
                        target,
                        "stun",
                        status::Options {
                            source: Some(id),
                            duration,
                            ..Default::default()
                        },
                    )?;
                }
            }
            Some(Ability::Power { every, .. }) => {
                self.content.as_mut().unwrap().counted(id, every);
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_damage(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        request: &mut hit::Request,
    ) -> Result<(), String> {
        if request.damage.amount <= 0. {
            return Ok(());
        }
        let result = self.resolve_hit(w, r, request);
        result.map(|_| ())
    }
    pub(super) fn enemy_content_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        self.helm_death(w, r, id, credit)?;
        self.pipe_death(w, r, id)?;
        self.parasite_detach(w, r, id)?;
        self.enemy_lifecycle_death(w, r, id)?;
        self.enemy_reaction_death(w, r, id)?;
        self.enemy_zone_death(w, id, credit)?;
        self.enemy_teapot_death(w, r, id)?;
        let Some((radius, scale, cold)) = (match self.content.as_ref().unwrap().kit_ref(id) {
            Kit::Ability(Ability::DeathBoom {
                radius,
                scale,
                cold,
            }) => Some((*radius, *scale, *cold)),
            _ => None,
        }) else {
            return Ok(());
        };
        if silenced(w, id)? {
            return Ok(());
        }
        let atk = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("atk");
        let radius = if radius > 0. { radius } else { 1.25 };
        let targets = enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?;
        for target in targets {
            if w.unit(target)?.motion_fly
                || !super::super::targeting::area_selectable(w, Some(id), target)?
            {
                continue;
            }
            let mut damage = ability_damage(Some(id), target, atk * scale, DamageType::Physical);
            self.enemy_damage(w, r, &mut damage)?;
            if let Some(duration) = cold {
                if w.unit(target)?.stats.alive {
                    self.apply_status(
                        w,
                        r,
                        target,
                        "cold",
                        status::Options {
                            duration,
                            source: Some(id),
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_content_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        ids: &[Id],
        dt: f64,
    ) -> Result<(), String> {
        self.enemy_sync_seeds(w, r, ids)?;
        for &id in ids {
            let mut content = self.content.take().ok_or("Missing enemy content context")?;
            let result = content.tick(w, r, self, &[id], dt);
            self.content = Some(content);
            result?;
            let u = w.unit(id)?;
            if !u.stats.alive || u.enemy.carrier.is_some() {
                continue;
            }
            let ability = match self.content.as_ref().unwrap().kit(id) {
                Kit::Ability(ability) => ability,
                Kit::Special(kit) => {
                    self.enemy_special_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Terrain(kit) => {
                    self.enemy_terrain_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Boss(boss) => {
                    if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
                        if route.patrol
                            && route.legs().len() >= 2
                            && route.leg >= route.legs().len() - 1
                        {
                            route.leg = 0;
                            route.points = None;
                            route.wait_left = None;
                        }
                    }
                    self.enemy_boss_tick(w, r, id, boss, dt)?;
                    continue;
                }
                Kit::Caster(config) => {
                    self.enemy_caster_tick(w, r, id, config, dt)?;
                    continue;
                }
                Kit::Leader(kit) => {
                    self.enemy_leader_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Supporter(kit) => {
                    self.enemy_supporter_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Movement(kit) => {
                    self.enemy_movement_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Lifecycle(kit) => {
                    self.enemy_lifecycle_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Reaction(kit) => {
                    self.enemy_reaction_tick(w, r, id, kit, dt)?;
                    continue;
                }
                Kit::Split(split) => {
                    self.enemy_split_tick(w, r, id, split, dt)?;
                    continue;
                }
                Kit::Family(family) => {
                    self.enemy_family_tick(w, r, id, family, dt)?;
                    continue;
                }
                _ => continue,
            };
            let interval = match ability {
                Ability::Frost { .. } => {
                    if silenced(w, id)? {
                        continue;
                    }
                    0.5
                }
                Ability::Enhance { interval, .. } => interval,
                Ability::SelfDamage(_) | Ability::Nest { .. } => 1.,
                Ability::Boil { interval, .. } => interval,
                Ability::Nucleus { interval, .. } => interval,
                _ => continue,
            };
            let pulses = self.content.as_mut().unwrap().pulses(id, dt, interval);
            for _ in 0..pulses {
                if !w.unit(id)?.stats.alive {
                    break;
                }
                match ability {
                    Ability::Frost { radius, aspd } => {
                        let targets =
                            enemy::Hooks::allies_in_radius(self, id, radius, w, w.unit_capacity())?;
                        for target in targets {
                            let mut b = buff("ab:frost", &[("aspd", aspd)]);
                            b.runtime.duration = 0.6;
                            b.runtime.visible = true;
                            self.enemy_buff(w, r, target, b)?;
                        }
                    }
                    Ability::Enhance { speed, atk, .. } => {
                        if !self.content.as_mut().unwrap().enhance(id) {
                            continue;
                        }
                        let speed = speed * w.unit(id)?.stats.base_number("moveSpeed");
                        let mut b = buff("ab:enhance", &[("moveFlat", speed), ("atkPct", atk)]);
                        b.runtime.persist = true;
                        b.runtime.refresh = buffs::Refresh::Stack;
                        b.runtime.max_stacks = 5;
                        self.enemy_buff(w, r, id, b)?;
                    }
                    Ability::Nucleus { radius, neural, .. } => {
                        if self.content.as_ref().unwrap().triggered(id) {
                            self.enemy_element_pulse(w, r, id, radius, neural)?;
                        }
                    }
                    Ability::Nest { .. } | Ability::Boil { .. } => {
                        self.enemy_dot_tick(w, r, id, ability)?
                    }
                    Ability::SelfDamage(amount) => {
                        let mut damage = ability_damage(None, id, amount, DamageType::True);
                        damage.is_skill = false;
                        damage.attack_type = hit::AttackType::Buff;
                        damage.tags = vec!["dot".into(), "periodic".into()];
                        self.enemy_damage(w, r, &mut damage)?;
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(())
    }
}
