// SPDX-License-Identifier: GPL-3.0-or-later
//! Concrete original operator combat dispatch. Unsupported operator definitions
//! are rejected by the roster before this host can execute them.
use super::super::{ally, attacks, projectiles, search, skills, targeting};
use super::*;
#[derive(Clone, Copy)]
pub(super) enum Shot {
    Turret(super::turrets::Shot),
    Tactical(super::tactics::Arrow),
    Enemy(enemy::Delivery),
    Skill(super::enemies::SkillShot),
    Bomb(super::enemies::BombShot),
    Cloud(super::enemies::CloudShot),
    Seed {
        source: Id,
        amount: f64,
    },
    BoomerangReturn {
        source: Id,
        deployment: u64,
    },
    Ally {
        source: Id,
        deployment: u64,
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
    fn active_attack_filter(
        &self,
        w: &World,
        id: Id,
    ) -> Result<Option<super::super::content::operators::AttackFilter>, String> {
        let entry = self
            .roster
            .as_ref()
            .ok_or("Missing operator context")?
            .entry(id)?;
        let unit = w.unit(id)?;
        let Some(filter) = entry.definition.attack_filter else {
            return Ok(None);
        };
        let key = format!("skill:{id}");
        // Modifiers follow skill activation even while its runtime is borrowed.
        let active = unit
            .stats
            .buffs()
            .iter()
            .any(|b| b.key.as_deref() == Some(key.as_str()));
        Ok(active.then_some(filter))
    }
    fn filtered_targets(
        &mut self,
        w: &mut World,
        id: Id,
        p: search::AttackProfile,
        filter: super::super::content::operators::AttackFilter,
    ) -> Result<Vec<Id>, String> {
        let keys = search::Host::range(self, id, w)?;
        let candidates = search::Host::enemies(self, id, &keys, p.selection, w, w.unit_capacity())?;
        let mut targets = Vec::with_capacity(candidates.len());
        for target in candidates {
            if filter(w, target)? {
                targets.push(target);
            }
        }
        Ok(targets)
    }
    pub(super) fn operator_enemy_death(&mut self, w: &mut World, victim: Id) -> Result<(), String> {
        let ids = self
            .roster
            .as_ref()
            .ok_or("Missing death content context")?
            .ids()
            .to_vec();
        for id in ids {
            let entry = self.roster.as_ref().unwrap().entry(id)?;
            let def = Arc::clone(&entry.definition);
            if let Some(effect) = &def.death_heal {
                if let Some(amount) = effect.amount(w, id, victim, entry.direction)? {
                    health::heal(
                        w,
                        self,
                        Some(id),
                        id,
                        amount,
                        health::HealOptions {
                            self_heal: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    fn cast_start(
        &mut self,
        w: &mut World,
        source: Id,
        def: &super::super::operators::Definition,
    ) -> Result<(), String> {
        let Some(start) = &def.skill_start else {
            return Ok(());
        };
        self.roster.as_mut().unwrap().add_dp(start.dp)?;
        let Some(area) = &start.area else {
            return Ok(());
        };
        let entry = self.roster.as_ref().unwrap().entry(source)?;
        let keys = targeting::absolute_range(
            &area.grid,
            w.unit(source)?.deployment.tile,
            entry.direction,
            0,
        );
        let mut targets = search::Host::enemies(
            self,
            source,
            &keys,
            targeting::Profile {
                can_hit_fly: true,
                ..Default::default()
            },
            w,
            w.unit_capacity(),
        )?;
        targeting::sort_enemies(w, self, source, &mut targets, def.combat.profile.priority)?;
        for target in targets {
            for _ in 0..area.hits {
                if !w.unit(target)?.stats.alive {
                    break;
                }
                let amount = w
                    .unit_mut(source)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .n("atk")
                    * area.scale;
                let mut q = hit::Request::new(
                    health::Hit {
                        source: Some(source),
                        credit: Some(source),
                        target,
                        damage_type: super::super::damage::DamageType::Arts,
                        no_sp: false,
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
                q.is_skill = true;
                q.tags.push("skill".into());
                attacks::Hooks::damage(self, w, &mut q, None)?;
            }
            if w.unit(target)?.stats.alive {
                let mut r = self.buffs.take().ok_or("Missing cast status context")?;
                let result = self.apply_status(
                    w,
                    &mut r,
                    target,
                    "stun",
                    super::super::status::Options {
                        duration: area.stun,
                        source: Some(source),
                        ..Default::default()
                    },
                );
                self.buffs = Some(r);
                result?;
            }
        }
        Ok(())
    }
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
        if !Host::tac_up(&self.core.world, id) {
            return Ok(());
        }
        self.core.world.time = self.clock.time();
        let mut roster = self.core.host.roster.take().unwrap();
        let mut r = self.core.host.buffs.take().unwrap();
        let result = roster.retreat(&mut self.core.world, &mut r, &mut self.core.host, id);
        self.core.host.roster = Some(roster);
        let result = result.and_then(|_| {
            self.core.host.kit_event(
                &mut self.core.world,
                &mut r,
                &mut kits::dispatch::Event::Death {
                    unit: id,
                    credit: None,
                },
            )?;
            self.core
                .host
                .flush_kit_events(&mut self.core.world, &mut r)
        });
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
                "attacks":u.attacks,"dealt":u.damage_dealt,"taken":u.damage_taken,"healed":u.healing_done}));
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
    fn can_activate(&mut self, skill: &skills::Runtime, w: &mut World) -> Result<bool, String> {
        self.kit_can_activate(w, skill.unit())
    }
    fn event(
        &mut self,
        e: skills::Event,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if e == skills::Event::SpGain {
            self.kit_sp_gain(s.unit(), c)?;
            return self.support_sp_gain(w, s.unit(), c);
        }
        let def = Arc::clone(
            &self
                .roster
                .as_ref()
                .ok_or("Missing operator skill context")?
                .entry(s.unit())?
                .definition,
        );
        let mut r = self.buffs.take().ok_or("Missing skill buff context")?;
        let direction = self.roster.as_ref().unwrap().entry(s.unit())?.direction;
        let result = def.skill_event(w, &mut r, self, e, direction, c, s);
        self.buffs = Some(r);
        result?;
        if e == skills::Event::SkillStart {
            self.resume_kit_skill(w, s)?;
        }
        if e == skills::Event::Start {
            self.cast_start(w, s.unit(), &def)?;
            if def.extra.effects.force_attack {
                let profile = def.combat.profile;
                let keys = search::Host::range(self, s.unit(), w)?;
                let selection =
                    search::acquire(w, self, s.unit(), &keys, profile, w.unit_capacity())?;
                if !selection.targets.is_empty() {
                    ally::Host::perform(self, s.unit(), profile, selection, Some(s), w)?;
                }
            }
        }
        Ok(())
    }
    fn condition(
        &mut self,
        which: skills::Condition,
        skill: &skills::Runtime,
        w: &mut World,
    ) -> Result<bool, String> {
        let id = skill.unit();
        let definition = Arc::clone(&self.roster.as_ref().unwrap().entry(id)?.definition);
        if let Some(yes) = self.kit_condition(w, id, which)? {
            return Ok(yes);
        }
        if matches!(which, skills::Condition::Allies)
            || skill.spec.heal_skill && which == skills::Condition::Default
            || skill.spec.heal_skill && which == skills::Condition::TickRule
        {
            let keys = if which == skills::Condition::TickRule
                && skill.spec.rule == skills::Rule::Global
            {
                (0..super::super::grid::N).collect()
            } else if which == skills::Condition::TickRule {
                self.support_trigger_range(w, id, &definition)?
            } else {
                search::Host::range(self, id, w)?
            };
            return Ok(!search::Host::injured_allies(
                self,
                id,
                &keys,
                definition
                    .combat
                    .profile
                    .heal
                    .is_some_and(|h| h.element_heal),
                w,
                w.unit_capacity(),
            )?
            .is_empty());
        }
        if which == skills::Condition::TickRule && skill.spec.rule == skills::Rule::Search {
            let keys = search::Host::range(self, id, w)?;
            return Ok(!search::Host::enemies(
                self,
                id,
                &keys,
                definition.combat.profile.selection,
                w,
                w.unit_capacity(),
            )?
            .is_empty());
        }
        if which == skills::Condition::TickRule {
            if skill.spec.rule == skills::Rule::Global {
                let keys: Vec<_> = (0..super::super::grid::N).collect();
                return Ok(!search::Host::enemies(
                    self,
                    id,
                    &keys,
                    targeting::Profile {
                        can_hit_fly: true,
                        ..Default::default()
                    },
                    w,
                    w.unit_capacity(),
                )?
                .is_empty());
            }
            if skill.spec.rule == skills::Rule::SpFull {
                return Ok(true);
            }
            if skill.spec.rule == skills::Rule::ActiveRange {
                // ACTIVE_RANGE follows DEFAULT's blocked-target condition, even
                // when the blocker faces away from the enemy it is holding.
                let held = w.unit(id)?.blocking().to_vec();
                for target in held {
                    if targeting::can_target_enemy(
                        w,
                        self,
                        Some(id),
                        target,
                        definition.combat.profile.selection,
                    )? {
                        return Ok(true);
                    }
                }
            }
            if matches!(
                skill.spec.rule,
                skills::Rule::SkillRange | skills::Rule::ActiveRange | skills::Rule::CustomRange
            ) {
                let entry = self.roster.as_ref().unwrap().entry(id)?;
                let grid = entry
                    .definition
                    .trigger_range
                    .as_ref()
                    .ok_or("Missing skill trigger grid")?;
                let keys = targeting::absolute_range(
                    grid,
                    w.unit(id)?.deployment.tile,
                    entry.direction,
                    0,
                );
                if skill.spec.rule == skills::Rule::SkillRange {
                    return self
                        .index
                        .as_ref()
                        .ok_or("Missing enemy index")?
                        .any_enemy(w, &keys, false);
                }
                return Ok(!search::Host::enemies(
                    self,
                    id,
                    &keys,
                    if skill.spec.rule == skills::Rule::ActiveRange {
                        definition.combat.profile.selection
                    } else {
                        targeting::Profile {
                            can_hit_fly: true,
                            ..Default::default()
                        }
                    },
                    w,
                    w.unit_capacity(),
                )?
                .is_empty());
            }
        }
        if which != skills::Condition::Default {
            return Err("Unsupported operator trigger condition".into());
        }
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
        true
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
        id: Id,
        keys: &[usize],
        element: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String> {
        let ids = self
            .roster
            .as_ref()
            .ok_or("Missing healer roster")?
            .ids()
            .to_vec();
        spatial::injured_allies(w, self, &ids, keys, Some(id), element, limit)
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
    fn profile(&mut self, id: Id, _: &mut World) -> Result<Option<search::AttackProfile>, String> {
        let entry = self.roster.as_ref().unwrap().entry(id)?;
        let c = entry.definition.combat;
        Ok(Some(if entry.skill()?.bank.active {
            c.skill_profile.unwrap_or(c.profile)
        } else {
            c.profile
        }))
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
        id: Id,
        p: search::AttackProfile,
        w: &mut World,
    ) -> Result<bool, String> {
        let def = self.tac_def(id)?;
        if let Some(kit) = def.extra.tactics.behavior() {
            if !kit.can_attack(self, w, id)? {
                return Ok(false);
            }
        }
        if let Some(filter) = self.active_attack_filter(w, id)? {
            Ok(!self.filtered_targets(w, id, p, filter)?.is_empty())
        } else {
            Ok(true)
        }
    }
}
impl ally::Host for Host {
    fn effective_profile(
        &mut self,
        id: Id,
        skill: Option<&skills::Runtime>,
        w: &mut World,
    ) -> Result<Option<search::AttackProfile>, String> {
        let c = self.roster.as_ref().unwrap().entry(id)?.definition.combat;
        let mut profile = if skill.is_some_and(|s| s.bank.active) {
            c.skill_profile.unwrap_or(c.profile)
        } else {
            c.profile
        };
        if let super::super::content::operators::Advanced::Billro(p) = &self
            .roster
            .as_ref()
            .unwrap()
            .entry(id)?
            .definition
            .extra
            .advanced
        {
            if p.mode == super::super::content::operators::Mode::First
                && w.unit(id)?.operator.advanced.charged
                && skill.is_some_and(|s| s.bank.active)
            {
                profile.no_attack = true;
            }
        }
        if w.time < w.unit(id)?.operator.support.quiet_until {
            profile.no_attack = true;
        }
        self.kit_profile(w, id, skill, &mut profile)?;
        Ok(Some(profile))
    }
    fn range_changed(&mut self, _: Id, _: &mut World) -> Result<bool, String> {
        Ok(false)
    }
    fn refresh_range(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn has_store_energy(&self, id: Id, _: search::AttackProfile) -> bool {
        self.tac_def(id).is_ok_and(|d| {
            matches!(
                d.extra.tactics,
                super::super::content::operators::Tactics::Indigo(_)
            )
        })
    }
    fn store_energy(
        &mut self,
        id: Id,
        _: search::AttackProfile,
        world: &mut World,
    ) -> Result<bool, String> {
        let def = self.tac_def(id)?;
        let super::super::content::operators::Tactics::Indigo(kit) = &def.extra.tactics else {
            return Ok(false);
        };
        let state = world.unit_mut(id)?.operator.tactical.indigo_mut()?;
        if state.stored >= kit.capacity {
            return Ok(false);
        }
        state.stored += 1;
        Ok(true)
    }
    fn perform(
        &mut self,
        id: Id,
        _: search::AttackProfile,
        selected: search::Selection,
        mut skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        self.support_before_attack(w, id, &selected.targets, skill.as_deref_mut())?;
        let combat = self.roster.as_ref().unwrap().entry(id)?.definition.combat;
        let is_skill = skill.as_deref().is_some_and(|s| s.attack_override());
        let melee = selected.fortress_melee == Some(true);
        let mut area = if is_skill {
            combat.skill_area
        } else {
            combat.area
        };
        area.fortress_melee = melee;
        if melee {
            area.splash = None;
        }
        let mut payload = attacks::Direct {
            attachment: None,
            element_heal_ratio: 0.,
            mode: if combat.heal || is_skill && combat.skill_heal {
                attacks::Mode::Heal
            } else {
                attacks::Mode::Damage(if is_skill {
                    combat.skill_damage.unwrap_or(combat.damage)
                } else {
                    combat.damage
                })
            },
            hits: if is_skill
                && self
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(id)?
                    .definition
                    .extra
                    .effects
                    .hit_upgrade
                    .is_some_and(|n| w.unit(id).is_ok_and(|u| u.operator.hits >= n))
            {
                4
            } else if is_skill {
                combat.skill_hits
            } else {
                combat.normal_hits
            },
            atk_scale: if is_skill { combat.skill_scale } else { 1. },
            skill_damage_mul: 1.,
            target_damage_mul: 1.,
            split_mul: combat.split_mul,
            heal_scale: if is_skill {
                combat.skill_heal_scale
            } else {
                1.
            },
            is_skill,
        };
        self.kit_payload(w, id, &mut payload, &mut area)?;
        let suppress_sp = w.unit(id)?.operator.support.bandage.is_some();
        let sp_type = skill.as_deref().map(|s| s.spec.sp_type);
        if suppress_sp {
            if let Some(s) = skill.as_deref_mut() {
                s.spec.sp_type = skills::SpType::None;
            }
        }
        let mut executor = self.executor.take().ok_or("Reentrant attack executor")?;
        let result = (|| {
            if let Some(projectile) = combat.projectile.filter(|_| {
                !melee && !combat.heal && !(is_skill && (combat.skill_heal || combat.skill_direct))
            }) {
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
                                hit_dead: area.splash.is_some()
                                    || combat.projectile == Some("boomerang"),
                                max_age: 10.,
                                on_hit: true,
                                visual: projectile.into(),
                                speed_changeable: None,
                                removable: None,
                                payload: Shot::Ally {
                                    source: id,
                                    deployment: w.unit(id)?.deployment.deploy_seq,
                                    attack_id: prepared.attack_id,
                                    payload,
                                    area,
                                },
                            },
                        )?;
                }
                executor.finish(w, self, prepared, skill.as_deref_mut())
            } else {
                executor.perform_area(
                    w,
                    self,
                    id,
                    &selected.targets,
                    payload,
                    area,
                    skill.as_deref_mut(),
                )
            }
        })();
        self.executor = Some(executor);
        if let (Some(s), Some(sp)) = (skill, sp_type) {
            s.spec.sp_type = sp;
        }
        result
    }
}
impl attacks::Hooks for Host {
    fn heal(&mut self, w: &mut World, source: Id, target: Id, amount: f64) -> Result<f64, String> {
        let amount = self.kit_attack_heal(w, source, target, amount)?;
        let mut r = self.buffs.take().ok_or("Missing healing buff context")?;
        let result = self.support_heal(
            w,
            &mut r,
            Some(source),
            target,
            amount,
            health::HealOptions::default(),
        );
        self.buffs = Some(r);
        result
    }

    fn prepare_attachment(
        &mut self,
        w: &mut World,
        source: Id,
        target: Option<Id>,
        payload: &mut attacks::Direct,
    ) -> Result<(), String> {
        self.funnel_prepare(w, source, target, payload)
    }
    fn attachment_damage(
        &mut self,
        w: &mut World,
        source: Id,
        target: Id,
        attack_id: u64,
        payload: attacks::Direct,
    ) -> Result<f64, String> {
        self.funnel_damage(w, source, target, attack_id, payload)
    }
    fn has_before_attack(&self) -> bool {
        true
    }
    fn before_attack(&mut self, c: &mut attacks::Context, w: &mut World) -> Result<(), String> {
        self.component_before(c, w)?;
        self.advanced_before(c, w)?;
        self.kit_before(w, c)?;
        let proc = self
            .roster
            .as_ref()
            .unwrap()
            .entry(c.attacker)?
            .definition
            .attack_proc;
        if let Some(proc) = proc {
            let key = format!("skill:{}", c.attacker);
            let active = w
                .unit(c.attacker)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.key.as_deref() == Some(key.as_str()));
            if let Some(buff) = proc.roll(active, &mut self.rng) {
                let mut r = self
                    .buffs
                    .take()
                    .ok_or("Missing before-attack buff context")?;
                let result = r.add(
                    w,
                    self,
                    c.attacker,
                    buffs::Spec {
                        buff,
                        mods_present: true,
                        ..Default::default()
                    },
                );
                self.buffs = Some(r);
                result?;
            }
        }
        let Some(filter) = self.active_attack_filter(w, c.attacker)? else {
            return Ok(());
        };
        let mut all_eligible = true;
        for &target in &c.targets {
            all_eligible &= filter(w, target)?;
        }
        if all_eligible {
            return Ok(());
        }
        let p = self
            .roster
            .as_ref()
            .unwrap()
            .entry(c.attacker)?
            .definition
            .combat
            .profile;
        let mut targets = self.filtered_targets(w, c.attacker, p, filter)?;
        targeting::sort_enemies(w, self, c.attacker, &mut targets, p.priority)?;
        targets.truncate(c.targets.len().max(1));
        c.targets = targets;
        Ok(())
    }
    fn engage(&mut self, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn damage(
        &mut self,
        w: &mut World,
        q: &mut hit::Request,
        skill: Option<&mut skills::Runtime>,
    ) -> Result<f64, String> {
        // Damage can synchronously restart the attacker's skill (Texas's first
        // kill). Publish its actual state while resolving, then recover it even
        // on failure; do not clone state or defer the kill until after removal.
        if let Some(skill) = skill {
            let id = skill.unit();
            let owned = skill.take_for_callback()?;
            self.roster.as_mut().unwrap().restore_skill(id, owned)?;
            let result = attacks::Hooks::damage(self, w, q, None);
            *skill = self.roster.as_mut().unwrap().take_skill(id)?;
            return result;
        }
        let mut r = self.buffs.take().ok_or("Missing attack buff context")?;
        let result = self.resolve_hit(w, &mut r, q);
        self.buffs = Some(r);
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
        let result = self.apply_status(
            w,
            &mut r,
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
        mut skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        self.advanced_impact(impact, skill.as_deref_mut(), w)?;
        self.kit_impact(w, impact)?;
        self.support_hit(impact, w)?;
        self.component_hit(impact, w)?;
        if impact.kind == attacks::VictimKind::Main && !impact.heal {
            if let Some(target) = impact
                .target
                .filter(|id| w.unit(*id).is_ok_and(|u| u.stats.alive))
            {
                let def = Arc::clone(
                    &self
                        .roster
                        .as_ref()
                        .unwrap()
                        .entry(impact.attacker)?
                        .definition,
                );
                let effect = if impact.is_skill {
                    def.extra.skill_status.or(def.extra.normal_status)
                } else {
                    def.extra.normal_status
                };
                if let Some(effect) = effect.filter(|e| e.duration > 0.) {
                    let apply = effect.chance.map_or(true, |p| self.rng.chance(p));
                    if apply {
                        let mut r = self.buffs.take().ok_or("Missing on-hit status context")?;
                        let result = self.apply_status(
                            w,
                            &mut r,
                            target,
                            effect.key,
                            super::super::status::Options {
                                duration: effect.duration,
                                source: Some(impact.attacker),
                                ..Default::default()
                            },
                        );
                        self.buffs = Some(r);
                        result?;
                    }
                }
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
        impact: attacks::Impact,
        skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        if impact.heal {
            self.advanced_impact(impact, skill, w)?;
            self.kit_impact(w, impact)?;
        }
        Ok(())
    }
    fn attack(
        &mut self,
        c: &mut attacks::Context,
        mut skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        if let Some(skill) = skill.as_deref_mut() {
            self.advanced_finish_skill(w, skill)?;
            self.resume_kit_skill(w, skill)?;
            self.support_flush_sp(w, skill)?;
        }
        self.support_attack(c, w)?;
        self.advanced_attack(c, w)?;
        self.kit_attack(w, c, skill.as_deref_mut())?;
        if let Some(skill) = skill.as_deref_mut() {
            self.advanced_finish_skill(w, skill)?;
            self.resume_kit_skill(w, skill)?;
        }
        self.component_attack(c, w)?;
        let mut buffs = self.buffs.take().ok_or("Missing equipment attack buffs")?;
        let result = self.equipment_attack(w, &mut buffs, c);
        self.buffs = Some(buffs);
        result?;
        let proc = self
            .roster
            .as_ref()
            .unwrap()
            .entry(c.attacker)?
            .definition
            .attack_proc;
        if let Some(proc) = proc {
            let mut r = self.buffs.take().ok_or("Missing attack cleanup context")?;
            let result = r.remove(w, self, c.attacker, proc.key);
            self.buffs = Some(r);
            result?;
        }
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
        self.roster.as_ref().is_some_and(|r| {
            r.ids().iter().any(|&id| {
                r.entry(id).is_ok_and(|e| {
                    matches!(
                        e.definition.extra.tactics,
                        super::super::content::operators::Tactics::Yu(_)
                    )
                })
            })
        })
    }
    fn step(
        &mut self,
        p: &mut projectiles::Projectile<Shot>,
        flights: &mut projectiles::System<Shot>,
        w: &mut World,
    ) -> Result<bool, String> {
        let mut cancelled = false;
        self.flights = Some(std::mem::replace(flights, projectiles::System::new(1)?));
        let result = self.kit_event_with_buffs(
            w,
            &mut kits::dispatch::Event::ProjectileStep {
                id: p.id,
                source: p.source,
                position: p.position,
                speed: p.speed,
                removable: p.removable,
                cancelled: &mut cancelled,
            },
        );
        *flights = self
            .flights
            .take()
            .ok_or("Missing step projectile system")?;
        result.map(|_| cancelled)
    }
    fn impact(
        &mut self,
        p: &mut projectiles::Projectile<Shot>,
        target: Option<Id>,
        flights: &mut projectiles::System<Shot>,
        w: &mut World,
    ) -> Result<(), String> {
        // Callbacks can launch through death, skill and chained-hit hooks. Keep
        // the active system's capacity reservations and pins available to them.
        self.flights = Some(std::mem::replace(flights, projectiles::System::new(1)?));
        let result = (|| match p.payload {
            Shot::Turret(shot) => self.turret_impact(w, shot, target),
            Shot::Seed { source, amount } => {
                self.enemy_seed_impact(w, source, target, p.position, amount)
            }
            Shot::Bomb(shot) => self.enemy_bomb_impact(w, shot, target, p.position),
            Shot::Skill(shot) => self.enemy_skill_impact(w, shot, target),
            Shot::Cloud(shot) => self.enemy_cloud(w, shot, p.position),
            Shot::Tactical(arrow) => {
                let mut r = self
                    .buffs
                    .take()
                    .ok_or("Missing tactical projectile buffs")?;
                let result = self.tactical_arrow(w, &mut r, arrow, target, p.position);
                self.buffs = Some(r);
                result
            }
            Shot::BoomerangReturn { source, deployment } => {
                if Host::tac_up(w, source) && w.unit(source)?.deployment.deploy_seq == deployment {
                    let state = w.unit_mut(source)?.operator.tactical.caper_mut()?;
                    state.out = state.out.saturating_sub(1);
                }
                Ok(())
            }
            Shot::Ally {
                source,
                deployment,
                attack_id,
                mut payload,
                area,
            } => {
                if let Some(target) = target {
                    let entry = self.roster.as_ref().unwrap().entry(source)?;
                    if let Some(lord) = entry.definition.extra.lord {
                        let home = w.unit(source)?.deployment.tile;
                        let melee_range =
                            targeting::absolute_range(&[[0, 0], [0, 1]], home, entry.direction, 0);
                        if !(payload.is_skill && lord.skill_full)
                            && w.unit(target)?.blocked_by() != Some(source)
                            && !super::super::body::in_range(w.unit(target)?, &melee_range)
                        {
                            payload.target_damage_mul *= lord.scale;
                        }
                    }
                    if let Some(scale) = &entry.definition.grid_scale {
                        payload.target_damage_mul *= scale.multiplier(
                            w,
                            source,
                            target,
                            entry.direction,
                            payload.is_skill,
                        )?;
                    }
                }
                let mut executor = self
                    .executor
                    .take()
                    .ok_or("Reentrant projectile executor")?;
                let result = executor.impact_at(
                    w, self, source, target, attack_id, payload, area, p.position, None,
                );
                self.executor = Some(executor);
                if matches!(
                    self.tac_def(source)?.extra.tactics,
                    super::super::content::operators::Tactics::Caper(_)
                ) {
                    self.caper_return(w, source, deployment, p.position)?;
                }
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
        })();
        *flights = self
            .flights
            .take()
            .ok_or("Missing impact projectile system")?;
        result
    }
}
