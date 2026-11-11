// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{content::enemies::terrain::TerrainKit, targeting, terrain::Kind};
use crate::deterministic::math::hypot;
impl Host {
    fn enemy_on_terrain(&self, w: &World, id: Id, kind: Kind) -> Result<bool, String> {
        Ok(self.terrain.at(w.unit(id)?.position) == kind)
    }
    pub(super) fn enemy_terrain_hit(
        &mut self,
        w: &mut World,
        q: &mut hit::Request,
        id: Id,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Terrain(TerrainKit::Infection)
                if self.enemy_on_terrain(w, id, Kind::Infection)? =>
            {
                q.hit.damage_type = DamageType::Arts
            }
            Kit::Terrain(TerrainKit::Chimera { .. }) => {
                let mut state = self.content.as_ref().unwrap().terrain_state(id);
                if !state.active && self.enemy_on_terrain(w, id, Kind::Infection)? {
                    state.active = true;
                    self.content.as_mut().unwrap().set_terrain_state(id, state);
                }
                if state.active {
                    q.hit.damage_type = DamageType::Arts;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn enemy_terrain_in(&self, q: &mut hit::Request) {
        if let Some(content) = &self.content {
            if matches!(
                content.kit(q.hit.target),
                Kit::Terrain(TerrainKit::Deepsea { swim: true, .. })
            ) && content.terrain_state(q.hit.target).wet
                && q.tags.iter().any(|t| t == "deepsea" || t == "drown")
            {
                q.cancelled = true;
            }
        }
    }
    pub(in crate::battle::runtime) fn enemy_chalice_share(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
    ) -> Result<(), String> {
        if !self.content.as_ref().is_some_and(|c| c.has_chalices())
            || q.cancelled
            || w.unit(q.hit.target)?.side != health::Side::Enemy
            || q.tags.iter().any(|t| t == "chalice")
            || targeting::flying(w, q.hit.target)?
        {
            return Ok(());
        }
        let at = w.unit(q.hit.target)?.position;
        for id in self.spawns.as_ref().unwrap().live().to_vec() {
            if id == q.hit.target || !w.unit(id)?.stats.alive {
                continue;
            }
            let Kit::Terrain(TerrainKit::Chalice { radius, share }) =
                self.content.as_ref().unwrap().kit(id)
            else {
                continue;
            };
            let p = w.unit(id)?.position;
            if share <= 0. || hypot(p[0] - at[0], p[1] - at[1]) > radius {
                continue;
            }
            let amount = q.damage.amount * share;
            q.damage.amount -= amount;
            let mut part = ability_damage(q.hit.source, id, amount, q.hit.damage_type);
            part.hit.credit = q.hit.credit;
            part.is_skill = false;
            part.tags = q.tags.clone();
            part.tags.push("chalice".into());
            self.enemy_damage(w, r, &mut part)?;
            break;
        }
        Ok(())
    }
    pub(super) fn enemy_terrain_attack(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        let id = c.enemy;
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Terrain(TerrainKit::Nazg { radius }) if event == enemy::Event::BeforeAttack => {
                let mut targets = vec![];
                let at = w.unit(id)?.position;
                let infected = self.enemy_on_terrain(w, id, Kind::Infection)?;
                for t in self.roster.as_ref().unwrap().ids() {
                    let p = w.unit(*t)?.position;
                    if (hypot(p[0] - at[0], p[1] - at[1]) <= radius
                        || infected && self.enemy_on_terrain(w, *t, Kind::Infection)?)
                        && targeting::can_target_ally(w, id, *t, true, Default::default())?
                    {
                        targets.push(*t);
                    }
                }
                if !targets.is_empty() {
                    targeting::sort_allies(w, id, &mut targets, Default::default())?;
                    c.targets = targets;
                }
            }
            Kit::Terrain(TerrainKit::Deepsea { every, stun, .. })
                if event == enemy::Event::Attack =>
            {
                if every == 0
                    || silenced(w, id)?
                    || !self.content.as_mut().unwrap().counted(id, every)
                {
                    return Ok(());
                }
                for &t in &c.targets {
                    if stun > 0.
                        && w.unit(t)?.stats.alive
                        && c.deployments
                            .contains(&(t, w.unit(t)?.deployment.deploy_seq))
                    {
                        self.apply_status(
                            w,
                            r,
                            t,
                            "stun",
                            status::Options {
                                source: Some(id),
                                duration: stun,
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
    pub(super) fn enemy_terrain_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        kit: TerrainKit,
        dt: f64,
    ) -> Result<(), String> {
        if !matches!(kit, TerrainKit::Deepsea { .. } | TerrainKit::Chimera { .. }) {
            return Ok(());
        }
        let pulses = self.content.as_mut().unwrap().pulses(id, dt, 0.25);
        for _ in 0..pulses {
            let mut state = self.content.as_ref().unwrap().terrain_state(id);
            match kit {
                TerrainKit::Chimera { damage, recovery } => {
                    state.active |= self.enemy_on_terrain(w, id, Kind::Infection)?;
                    if !state.active {
                        continue;
                    }
                    state.elapsed += 0.25;
                    let pulse = state.elapsed >= 0.5 - 1e-9;
                    if pulse {
                        state.elapsed -= 0.5;
                    }
                    for t in enemy::Hooks::allies_in_radius(self, id, 1.2, w, w.unit_capacity())? {
                        let mut b = buff(
                            "ab:originium",
                            &[("spRecoveryMul", (1. + recovery).max(0.))],
                        );
                        b.runtime.duration = 0.35;
                        self.enemy_buff(w, r, t, b)?;
                        if !pulse || damage <= 0. || w.time - w.unit(t)?.chimera_at < 0.5 - 1e-6 {
                            continue;
                        }
                        let now = w.time;
                        w.unit_mut(t)?.chimera_at = now;
                        let mut q = ability_damage(None, t, damage, DamageType::True);
                        q.hit.credit = Some(id);
                        q.hit.no_sp = false;
                        q.is_skill = false;
                        q.tags = vec!["dot".into(), "pollution".into()];
                        self.enemy_damage(w, r, &mut q)?;
                    }
                }
                TerrainKit::Deepsea {
                    swim, atk, drown, ..
                } => {
                    let wet = self.enemy_on_terrain(w, id, Kind::Deepsea)?;
                    if swim {
                        if wet && !state.wet {
                            let mut b = buff("ab:swim", &[("atkPct", atk)]);
                            b.flags.insert("stealth".into());
                            b.runtime.visible = true;
                            self.enemy_buff(w, r, id, b)?;
                        }
                        if !wet && state.wet {
                            r.remove(w, self, id, "ab:swim")?;
                        }
                        if wet {
                            r.remove(w, self, id, "terrain:deepsea")?;
                        }
                    }
                    if wet && drown > 0. {
                        let mut q = ability_damage(None, id, drown * 0.25, DamageType::True);
                        q.hit.no_sp = false;
                        q.is_skill = false;
                        q.tags = vec!["dot".into(), "periodic".into(), "drown".into()];
                        self.enemy_damage(w, r, &mut q)?;
                    }
                    state.wet = wet;
                }
                _ => {}
            }
            self.content.as_mut().unwrap().set_terrain_state(id, state);
        }
        Ok(())
    }
}
