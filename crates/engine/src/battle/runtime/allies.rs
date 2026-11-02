// SPDX-License-Identifier: GPL-3.0-or-later
//! Concrete original operator combat dispatch. Unsupported operator definitions
//! are rejected by the roster before this host can execute them.
use super::super::{ally, attacks, projectiles, search, skills, targeting};
use super::*;
#[derive(Clone, Copy)]
pub(super) enum Shot {
    Enemy(enemy::Delivery),
    Ally {
        source: Id,
        attack_id: u64,
        payload: attacks::Direct,
        area: attacks::Area,
    },
}

pub(super) struct Blocking<'a> {
    pub host: &'a mut Host,
    pub buffs: &'a mut buffs::Runtime,
}
impl blocking::Hooks for Blocking<'_> {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        blocking::restore_stealth(w, self.buffs, self.host, id)
    }
}
impl Host {
    pub(super) fn rebuild_index(&mut self, w: &World) -> Result<(), String> {
        self.index
            .as_mut()
            .ok_or("Missing enemy index")?
            .rebuild(w, self.spawns.as_ref().unwrap().live())
    }
}
impl Field {
    pub fn retreat_operator(&mut self, id: Id) -> Result<(), String> {
        self.healthy()?;
        if self.finished() {
            return Err("Cannot retreat from a finished field".into());
        }
        self.core.host.roster.as_ref().unwrap().entry(id)?;
        self.core.world.time = self.clock.time();
        let mut roster = self.core.host.roster.take().unwrap();
        let mut r = self.core.host.buffs.take().unwrap();
        let result = roster.retreat(&mut self.core.world, &mut r, &mut self.core.host, id);
        self.core.host.roster = Some(roster);
        self.core.host.buffs = Some(r);
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    /// Owned internal combat diagnostics; not a public policy observation.
    pub fn ally_states(&mut self) -> Result<Vec<Value>, String> {
        self.healthy()?;
        let ids = self.allies().to_vec();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let entry = self.core.host.roster.as_ref().unwrap().entry(id)?;
            let skill = entry.skill()?;
            let u = self.core.world.unit_mut(id)?;
            let stats = u.stats.stats().map_err(str::to_string)?.clone();
            out.push(json!({"id":id,"key":entry.definition.key,"hp":u.stats.hp,"maxHp":stats.n("maxHp"),
                "atk":stats.n("atk"),"def":stats.n("def"),"res":stats.n("res"),"cd":u.cadence.cooldown,
                "alive":u.stats.alive,"deployed":u.deployed,"removed":u.removed,"blocking":u.blocking(),
                "deploy":u.deployment.deploy_seq,"aggro":u.deployment.aggro_seq,"body":u.deployment.body,
                "death":u.deployment.death_at,"respawn":u.deployment.respawn_at,"reason":u.deployment.remove_reason,
                "ground":u.ground,"element":u.element_buildup,"flags":stats.flags,
                "buffs":u.stats.buffs().iter().map(|b|json!({"key":b.key,"left":b.runtime.time_left,"stacks":b.stacks,"persist":b.runtime.persist})).collect::<Vec<_>>(),
                "sp":skill.bank.sp,"charges":skill.bank.charges,
                "left":skill.time_left,"active":skill.bank.active,"activations":skill.activations,
                "attacks":u.attacks,"dealt":u.damage_dealt,"taken":u.damage_taken}));
        }
        Ok(out)
    }
    pub fn deployment_points(&self) -> Result<f64, String> {
        self.healthy()?;
        self.core
            .host
            .roster
            .as_ref()
            .unwrap()
            .dp(1)
            .ok_or("Missing deployment points".into())
    }
    pub fn projectile_count(&self) -> usize {
        self.core.host.flights.as_ref().unwrap().list().len()
    }
}
impl skills::Hooks for Host {
    fn event(
        &mut self,
        e: skills::Event,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        let def = Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing operator skill context")?
                .entry(s.unit())?
                .definition,
        );
        let mut r = self.buffs.take().ok_or("Missing skill buff context")?;
        let result = def.skill_event(w, &mut r, self, e, c, s);
        self.buffs = Some(r);
        result
    }
    fn condition(
        &mut self,
        which: skills::Condition,
        skill: &skills::Runtime,
        w: &mut World,
    ) -> Result<bool, String> {
        if which != skills::Condition::Default {
            return Err("Unsupported operator trigger condition".into());
        }
        let id = skill.unit();
        let keys = search::Host::range(self, id, w)?;
        let p = self
            .roster
            .as_ref()
            .unwrap()
            .entry(id)?
            .definition
            .combat
            .profile;
        let limit = w.unit_capacity();
        if !search::Host::enemies(self, id, &keys, p.selection, w, limit)?.is_empty() {
            return Ok(true);
        }
        let held = w.unit(id)?.blocking().to_vec();
        for target in held {
            if targeting::can_target_enemy(w, self, Some(id), target, p.selection)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn profile(&mut self, id: Id, _: &mut World) -> Result<Option<search::AttackProfile>, String> {
        Ok(Some(
            self.roster
                .as_ref()
                .unwrap()
                .entry(id)?
                .definition
                .combat
                .profile,
        ))
    }
    fn has_sp_hook(&self) -> bool {
        false
    }
}
impl search::Host for Host {
    fn enemies(
        &mut self,
        id: Id,
        keys: &[usize],
        p: targeting::Profile,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        let index = self.index.take().ok_or("Missing target index context")?;
        let result = index.enemies(w, self, keys, Some(id), p, false, limit);
        self.index = Some(index);
        result
    }
    fn injured_allies(
        &mut self,
        _: Id,
        _: &[usize],
        _: bool,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Err("No healer kit is implemented in this field entry".into())
    }
    fn has_ally_targets(&self) -> bool {
        false
    }
    fn ally_targets(
        &mut self,
        _: Id,
        _: &[usize],
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Err("No allied-target content is implemented in this field entry".into())
    }
    fn profile(&mut self, id: Id, w: &mut World) -> Result<Option<search::AttackProfile>, String> {
        skills::Hooks::profile(self, id, w)
    }
    fn range(&mut self, id: Id, w: &World) -> Result<Vec<usize>, String> {
        self.roster.as_ref().unwrap().range(w, id)
    }
    fn skill_active(&self, id: Id, _: &World) -> Result<bool, String> {
        Ok(self
            .roster
            .as_ref()
            .unwrap()
            .entry(id)?
            .skill()?
            .bank
            .active)
    }
    fn can_attack(
        &mut self,
        _: Id,
        _: search::AttackProfile,
        _: &mut World,
    ) -> Result<bool, String> {
        Ok(true)
    }
}
impl ally::Host for Host {
    fn effective_profile(
        &mut self,
        id: Id,
        _: Option<&skills::Runtime>,
        w: &mut World,
    ) -> Result<Option<search::AttackProfile>, String> {
        search::Host::profile(self, id, w)
    }
    fn range_changed(&mut self, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn refresh_range(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn has_store_energy(&self, _: Id, _: search::AttackProfile) -> bool {
        false
    }
    fn store_energy(
        &mut self,
        _: Id,
        _: search::AttackProfile,
        _: &mut World,
    ) -> Result<bool, String> {
        Err("No stored attack energy implemented for this operator".into())
    }
    fn perform(
        &mut self,
        id: Id,
        _: search::AttackProfile,
        selected: search::Selection,
        skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        let combat = self.roster.as_ref().unwrap().entry(id)?.definition.combat;
        let is_skill = skill.as_deref().is_some_and(|s| s.attack_override());
        let area = if is_skill {
            combat.skill_area
        } else {
            combat.area
        };
        let payload = attacks::Direct {
            mode: attacks::Mode::Damage(combat.damage),
            hits: 1,
            atk_scale: 1.,
            skill_damage_mul: 1.,
            target_damage_mul: 1.,
            split_mul: None,
            heal_scale: 1.,
            is_skill,
        };
        let mut executor = self.executor.take().ok_or("Reentrant attack executor")?;
        let result = (|| {
            if let Some(projectile) = combat.projectile {
                let Some(prepared) = executor.prepare(w, self, id, &selected.targets, payload)?
                else {
                    return Ok(());
                };
                for &target in &prepared.context.targets {
                    self.flights
                        .as_mut()
                        .ok_or("Reentrant projectile launch")?
                        .add(
                            w,
                            projectiles::Launch {
                                from: w.unit(id)?.position,
                                source: Some(id),
                                target: Some(target),
                                to: None,
                                speed: combat.projectile_speed,
                                hit_dead: area.splash.is_some(),
                                max_age: 10.,
                                on_hit: true,
                                visual: projectile.into(),
                                speed_changeable: None,
                                removable: None,
                                payload: Shot::Ally {
                                    source: id,
                                    attack_id: prepared.attack_id,
                                    payload,
                                    area,
                                },
                            },
                        )?;
                }
                executor.finish(w, self, prepared, skill)
            } else {
                executor.perform_area(w, self, id, &selected.targets, payload, area, skill)
            }
        })();
        self.executor = Some(executor);
        result
    }
}
impl attacks::Hooks for Host {
    fn has_before_attack(&self) -> bool {
        false
    }
    fn before_attack(&mut self, _: &mut attacks::Context, _: &mut World) -> Result<(), String> {
        Err("No before-attack handler installed".into())
    }
    fn engage(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn damage(&mut self, w: &mut World, q: &mut hit::Request) -> Result<f64, String> {
        let mut r = self.buffs.take().ok_or("Missing attack buff context")?;
        let mut rng = self.rng.clone();
        let result = settlement::resolve(w, &mut r, &mut rng, self, q);
        self.buffs = Some(r);
        self.rng = rng;
        result
    }
    fn radius_targets(
        &mut self,
        origin: [f64; 2],
        radius: f64,
        splash: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        let ids = self.spawns.as_ref().unwrap().live().to_vec();
        spatial::enemies_in_radius(
            w,
            self,
            &ids,
            origin,
            radius,
            if splash {
                spatial::RadiusRule::Centre
            } else {
                spatial::RadiusRule::Body
            },
            splash,
            limit,
        )
    }
    fn chain_link(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(()) // The source event is display-only.
    }
    fn sluggish(
        &mut self,
        source: Id,
        target: Id,
        duration: f64,
        w: &mut World,
    ) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing attack status context")?;
        let result = super::super::status::apply(
            w,
            self,
            &mut r,
            Rect::NORMAL,
            target,
            "sluggish",
            super::super::status::Options {
                duration,
                source: Some(source),
                ..Default::default()
            },
        );
        self.buffs = Some(r);
        result.map(|_| ())
    }
    fn each_hit(
        &mut self,
        impact: attacks::Impact,
        _: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        if impact.kind == attacks::VictimKind::Main {
            if let Some(target) = impact
                .target
                .filter(|id| w.unit(*id).is_ok_and(|u| u.stats.alive))
            {
                let combat = self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(impact.attacker)?
                    .definition
                    .combat;
                let duration = if impact.is_skill {
                    combat.skill_sluggish
                } else {
                    combat.sluggish
                };
                if duration > 0. {
                    attacks::Hooks::sluggish(self, impact.attacker, target, duration, w)?;
                }
            }
        }
        Ok(())
    }
    fn after_hit(
        &mut self,
        _: attacks::Impact,
        _: Option<&mut skills::Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
    fn attack(
        &mut self,
        _: &mut attacks::Context,
        _: Option<&mut skills::Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
    fn after_attack(
        &mut self,
        _: &attacks::Context,
        _: Option<&mut skills::Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
        Ok(())
    }
}
impl projectiles::Hooks<Shot> for Host {
    fn has_step(&self) -> bool {
        false
    }
    fn step(
        &mut self,
        _: &mut projectiles::Projectile<Shot>,
        _: &mut projectiles::System<Shot>,
        _: &mut World,
    ) -> Result<bool, String> {
        Err("No projectile step content installed".into())
    }
    fn impact(
        &mut self,
        p: &mut projectiles::Projectile<Shot>,
        target: Option<Id>,
        _: &mut projectiles::System<Shot>,
        w: &mut World,
    ) -> Result<(), String> {
        match p.payload {
            Shot::Ally {
                source,
                attack_id,
                payload,
                area,
            } => {
                let mut executor = self
                    .executor
                    .take()
                    .ok_or("Reentrant projectile executor")?;
                let result = executor.impact_at(
                    w, self, source, target, attack_id, payload, area, p.position, None,
                );
                self.executor = Some(executor);
                result
            }
            Shot::Enemy(delivery) => {
                let mut r = self.buffs.take().ok_or("Missing projectile buff context")?;
                let mut rng = self.rng.clone();
                let result = enemy::impact(w, &mut r, &mut rng, self, delivery, target);
                self.buffs = Some(r);
                self.rng = rng;
                result.map(|_| ())
            }
        }
    }
}
