// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{
    content::enemies::{
        lifecycle::{Memory, RebornPhase},
        Lifecycle,
    },
    element, projectiles, targeting,
};
use crate::deterministic::math::hypot;
pub(in crate::battle::runtime) const REBORN_FINISH: u64 = 1701;
impl Host {
    pub(in crate::battle::runtime) fn enemy_lifecycle_profile(
        &self,
        id: Id,
        kit: &Kit,
        profile: &mut enemy::Profile,
    ) {
        match kit {
            Kit::Lifecycle(Lifecycle::Seed) => profile.no_attack = true,
            Kit::Lifecycle(kit) if kit.rebirth().is_some() => {
                profile.no_attack |=
                    self.content.as_ref().unwrap().reborn_phase(id) == Some(RebornPhase::Rebirth);
                match kit {
                    Lifecycle::Warden { .. } => {
                        profile.melee = self.content.as_ref().unwrap().reborn_phase(id)
                            == Some(RebornPhase::Revived)
                    }
                    Lifecycle::Ugly { .. } => {
                        profile.melee = false;
                        profile.no_attack |= self.content.as_ref().unwrap().reborn_phase(id)
                            == Some(RebornPhase::Revived);
                        profile.atk_scale = match self.content.as_ref().unwrap().lifecycle(id) {
                            Memory::Ugly { splash: true, .. } => 1.,
                            _ => {
                                if let Lifecycle::Ugly { melee, .. } = kit {
                                    *melee
                                } else {
                                    1.
                                }
                            }
                        };
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    pub(super) fn enemy_lifecycle_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        self.enemy_rebirth_dealt(w, r, q, id)?;
        let Kit::Lifecycle(Lifecycle::Vine { first, normal, .. }) =
            self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        let Memory::Vine {
            start,
            first: initial,
        } = self.content.as_ref().unwrap().lifecycle(id)
        else {
            unreachable!()
        };
        if !initial {
            let amount = Self::tac_stat(w, id, "atk")? * normal;
            return self.enemy_element(w, r, id, q.hit.target, element::Element::Burn, amount);
        }
        self.content.as_mut().unwrap().set_lifecycle(
            id,
            Memory::Vine {
                start,
                first: false,
            },
        );
        let tile = w.unit(q.hit.target)?.deployment.tile;
        let mut targets = vec![q.hit.target];
        for target in self.roster.as_ref().unwrap().ids().to_vec() {
            if target == q.hit.target {
                continue;
            }
            let at = w.unit(target)?.deployment.tile;
            if (at[0] - tile[0]).abs() <= 1
                && (at[1] - tile[1]).abs() <= 1
                && targeting::area_selectable(w, Some(id), target)?
            {
                targets.push(target);
            }
        }
        for target in targets {
            let amount = Self::tac_stat(w, id, "atk")?;
            if target != q.hit.target {
                let mut hit = ability_damage(Some(id), target, amount, DamageType::Arts);
                self.enemy_damage(w, r, &mut hit)?;
            }
            let amount = Self::tac_stat(w, id, "atk")? * first;
            self.enemy_element(w, r, id, target, element::Element::Burn, amount)?;
        }
        r.remove(w, self, id, "ab:pow")?;
        Ok(())
    }
    pub(super) fn enemy_lifecycle_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        self.enemy_rebirth_attack(w, r, c, event)?;
        if event == enemy::Event::Attack
            && matches!(
                self.content.as_ref().unwrap().kit(id),
                Kit::Lifecycle(Lifecycle::Vine { .. })
            )
        {
            for other in self.enemy_nearby(w, id, 1.5)? {
                if self.spawns.as_ref().unwrap().definition(other)?.key == "enemy_10065_ftzlc"
                    && matches!(
                        self.content.as_ref().unwrap().lifecycle(other),
                        Memory::Ignitable { on: false }
                    )
                {
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_lifecycle(other, Memory::Ignitable { on: true });
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_lifecycle_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Kit::Lifecycle(Lifecycle::Ignitable { scale }) = self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        if !matches!(
            self.content.as_ref().unwrap().lifecycle(id),
            Memory::Ignitable { on: true }
        ) || silenced(w, id)?
        {
            return Ok(());
        }
        let amount = Self::tac_stat(w, id, "atk")? * scale;
        for target in enemy::Hooks::allies_in_radius(self, id, 1.25, w, w.unit_capacity())? {
            if w.unit(target)?.motion_fly || !targeting::area_selectable(w, Some(id), target)? {
                continue;
            }
            let mut hit = ability_damage(Some(id), target, amount, DamageType::Arts);
            self.enemy_damage(w, r, &mut hit)?;
        }
        Ok(())
    }
    pub(super) fn enemy_lifecycle_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: Lifecycle,
        dt: f64,
    ) -> Result<(), String> {
        self.enemy_reid_tick(w, r, id, kit, dt)?;
        match kit {
            Lifecycle::Vine { attack, time, .. } => {
                let Memory::Vine { start, first } = self.content.as_ref().unwrap().lifecycle(id)
                else {
                    unreachable!()
                };
                self.content
                    .as_mut()
                    .unwrap()
                    .set_lifecycle(id, Memory::Vine { start, first });
                let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.5);
                for _ in 0..pulses {
                    if !first || time <= 0. {
                        continue;
                    }
                    let mut b = buff(
                        "ab:pow",
                        &[("atkPct", attack * ((w.time - start) / time).min(1.))],
                    );
                    b.runtime.persist = true;
                    self.enemy_buff(w, r, id, b)?;
                }
            }
            Lifecycle::Brood { skill: Some(skill) } => {
                let Memory::Brood { mut left } = self.content.as_ref().unwrap().lifecycle(id)
                else {
                    unreachable!()
                };
                let castable = self.enemy_can_cast(w, id, false)?;
                let cast = skill.advance(&mut left, dt, castable);
                self.content
                    .as_mut()
                    .unwrap()
                    .set_lifecycle(id, Memory::Brood { left });
                if cast {
                    let children =
                        self.enemy_children_spec(w, id, Arc::from("enemy_1269_nhfly"), 3)?;
                    self.enemy_spawn_children(w, r, children)?;
                }
            }
            Lifecycle::Seed
                if matches!(
                    self.content.as_ref().unwrap().lifecycle(id),
                    Memory::Seed { projectile: None }
                ) =>
            {
                if w.time < w.unit(id)?.motion.unbalance_until - 1e-9 {
                    return Ok(());
                }
                let radius = w.unit(id)?.stats.base_number("rangeRadius");
                let mut targets =
                    self.enemy_targets_near(w, id, if radius == 0. { 1. } else { radius })?;
                let position = w.unit(id)?.position;
                targets.sort_by(|&a, &b| {
                    let a = w.unit(a).unwrap().position;
                    let b = w.unit(b).unwrap().position;
                    hypot(a[1] - position[1], a[0] - position[0])
                        .total_cmp(&hypot(b[1] - position[1], b[0] - position[0]))
                });
                let Some(&target) = targets.first() else {
                    return Ok(());
                };
                let amount = Self::tac_stat(w, id, "atk")?;
                let mut b = buff("ab:dive", &[]);
                b.flags.extend(
                    ["noMove", "untargetable", "invulnerable", "unblockable"].map(str::to_string),
                );
                self.enemy_buff(w, r, id, b)?;
                let shot = self
                    .flights
                    .as_mut()
                    .ok_or("Missing seed projectile system")?
                    .add(
                        w,
                        projectiles::Launch {
                            from: position,
                            source: Some(id),
                            target: Some(target),
                            to: None,
                            speed: 10.,
                            hit_dead: true,
                            max_age: 10.,
                            on_hit: true,
                            visual: "".into(),
                            speed_changeable: None,
                            removable: None,
                            payload: allies::Shot::Seed { source: id, amount },
                        },
                    )?;
                self.content.as_mut().unwrap().set_lifecycle(
                    id,
                    Memory::Seed {
                        projectile: Some(shot),
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_sync_seeds(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        ids: &[Id],
    ) -> Result<(), String> {
        for &id in ids {
            let Memory::Seed {
                projectile: Some(shot),
            } = self.content.as_ref().unwrap().lifecycle(id)
            else {
                continue;
            };
            if !w.unit(id)?.stats.alive {
                continue;
            }
            let position = self
                .flights
                .as_ref()
                .ok_or("Missing seed projectile system")?
                .list()
                .iter()
                .find(|p| p.id == shot)
                .map(|p| p.position);
            if let Some(position) = position {
                w.unit_mut(id)?.position = position;
            } else {
                self.kill_enemy(id, None, r, w)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_seed_impact(
        &mut self,
        w: &mut World,
        source: Id,
        target: Option<Id>,
        position: [f64; 2],
        amount: f64,
    ) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing seed impact buffs")?;
        let result = (|| {
            w.unit_mut(source)?.position = position;
            if let Some(target) = target {
                let mut hit = ability_damage(Some(source), target, amount, DamageType::Physical);
                self.enemy_damage(w, &mut r, &mut hit)?;
            }
            self.kill_enemy(source, None, &mut r, w)
        })();
        self.buffs = Some(r);
        result
    }
    pub(super) fn enemy_reborn_knockout(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<bool, String> {
        let kit = self.content.as_ref().unwrap().kit(id);
        let Some((duration, _, _)) = kit.rebirth() else {
            return Ok(false);
        };
        if self.content.as_ref().unwrap().reborn_phase(id) != Some(RebornPhase::Fresh) {
            return Ok(false);
        }
        self.content
            .as_mut()
            .unwrap()
            .set_reborn_phase(id, RebornPhase::Rebirth);
        w.unit_mut(id)?.motion.unbalance_until = f64::NEG_INFINITY;
        self.enemy_cleanse_rebirth(w, r, id)?;
        w.unit_mut(id)?.stats.hp = Self::tac_stat(w, id, "maxHp")?.min(1.);
        match kit {
            Kit::Lifecycle(k) => self.enemy_rebirth_start(w, r, id, k)?,
            Kit::Caster(c) => self.champion_rebirth_start(w, r, id, c)?,
            _ => unreachable!(),
        }
        if duration <= 0. {
            self.enemy_reborn_finish(w, r, id)?;
            return Ok(true);
        }
        let mut b = buff("ab:reborn", &[]);
        b.runtime.duration = duration;
        b.runtime.visible = true;
        b.runtime.persist = true;
        b.runtime.on_expire = Some(REBORN_FINISH);
        if matches!(
            kit,
            Kit::Caster(crate::battle::content::enemies::Caster {
                passive: crate::battle::content::enemies::casters::Passive::Wolf { .. },
                ..
            })
        ) {
            b.runtime.on_tick = Some(super::WOLF_REBORN);
        }
        b.flags.extend(
            [
                "invulnerable",
                "untargetable",
                "unblockable",
                "noMove",
                "stun",
            ]
            .map(str::to_string),
        );
        self.enemy_buff(w, r, id, b)?;
        Ok(true)
    }
    pub(in crate::battle::runtime) fn enemy_reborn_finish(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let kit = self.content.as_ref().unwrap().kit(id);
        let Some((_, invincible, hp)) = kit.rebirth() else {
            return Ok(());
        };
        if !w.unit(id)?.stats.alive
            || self.content.as_ref().unwrap().reborn_phase(id) != Some(RebornPhase::Rebirth)
        {
            return Ok(());
        }
        self.content
            .as_mut()
            .unwrap()
            .set_reborn_phase(id, RebornPhase::Revived);
        r.remove(w, self, id, "ab:reborn")?;
        match kit {
            Kit::Lifecycle(k) => self.enemy_rebirth_finish(w, r, id, k)?,
            Kit::Caster(c) => self.champion_rebirth_finish(w, r, id, c)?,
            _ => unreachable!(),
        }
        w.unit_mut(id)?.stats.hp = (Self::tac_stat(w, id, "maxHp")? * hp).max(1.);
        if invincible > 0. {
            let mut b = buff("ab:reborn:inv", &[]);
            b.runtime.duration = invincible;
            b.runtime.visible = true;
            b.flags.insert("invulnerable".into());
            if matches!(
                kit,
                Kit::Caster(crate::battle::content::enemies::Caster {
                    passive: crate::battle::content::enemies::casters::Passive::Xi { .. },
                    ..
                })
            ) {
                b.flags.insert("untargetable".into());
            }
            self.enemy_buff(w, r, id, b)?;
        }
        if let Some(route) = w.unit_mut(id)?.motion.route.as_mut() {
            route.points = None;
        }
        w.unit_mut(id)?.cadence.cooldown = 0.;
        Ok(())
    }
}
