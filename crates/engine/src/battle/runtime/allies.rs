// SPDX-License-Identifier: GPL-3.0-or-later
//! Concrete default protector combat dispatch. Unsupported operator definitions
//! are rejected by the roster before this host can execute them.
use super::super::{ally, attacks, projectiles, search, skills, targeting};
use super::*;

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
        _: skills::Condition,
        _: &skills::Runtime,
        _: &mut World,
    ) -> Result<bool, String> {
        Err("Default TAKE_DAMAGE kit has no geometric trigger".into())
    }
    fn profile(&mut self, id: Id, _: &mut World) -> Result<Option<search::AttackProfile>, String> {
        self.roster.as_ref().unwrap().entry(id)?;
        Ok(Some(search::AttackProfile::default()))
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
        Err("Protector has no stored attack energy".into())
    }
    fn perform(
        &mut self,
        id: Id,
        _: search::AttackProfile,
        selected: search::Selection,
        skill: Option<&mut skills::Runtime>,
        w: &mut World,
    ) -> Result<(), String> {
        let mut executor = self.executor.take().ok_or("Reentrant attack executor")?;
        let result = executor.perform_area(
            w,
            self,
            id,
            &selected.targets,
            attacks::Direct {
                mode: attacks::Mode::Damage(super::super::damage::DamageType::Physical),
                hits: 1,
                atk_scale: 1.,
                skill_damage_mul: 1.,
                target_damage_mul: 1.,
                split_mul: None,
                heal_scale: 1.,
                is_skill: false,
            },
            attacks::Area::default(),
            skill,
        );
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
        _: [f64; 2],
        _: f64,
        _: bool,
        _: &mut World,
        _: usize,
    ) -> Result<Vec<Id>, String> {
        Err("No protector area attack implemented".into())
    }
    fn chain_link(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Err("No protector chain attack implemented".into())
    }
    fn sluggish(&mut self, _: Id, _: Id, _: f64, _: &mut World) -> Result<(), String> {
        Err("No protector attack status implemented".into())
    }
    fn each_hit(
        &mut self,
        _: attacks::Impact,
        _: Option<&mut skills::Runtime>,
        _: &mut World,
    ) -> Result<(), String> {
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
impl projectiles::Hooks<enemy::Delivery> for Host {
    fn has_step(&self) -> bool {
        false
    }
    fn step(
        &mut self,
        _: &mut projectiles::Projectile<enemy::Delivery>,
        _: &mut projectiles::System<enemy::Delivery>,
        _: &mut World,
    ) -> Result<bool, String> {
        Err("No projectile step content installed".into())
    }
    fn impact(
        &mut self,
        p: &mut projectiles::Projectile<enemy::Delivery>,
        target: Option<Id>,
        _: &mut projectiles::System<enemy::Delivery>,
        w: &mut World,
    ) -> Result<(), String> {
        let mut r = self.buffs.take().ok_or("Missing projectile buff context")?;
        let mut rng = self.rng.clone();
        let result = enemy::impact(w, &mut r, &mut rng, self, p.payload, target);
        self.buffs = Some(r);
        self.rng = rng;
        result.map(|_| ())
    }
}
