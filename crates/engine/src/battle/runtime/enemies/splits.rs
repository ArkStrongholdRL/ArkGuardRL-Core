// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::{content::enemies::Split, motion::Route};

pub(in crate::battle::runtime) struct Children {
    key: Arc<str>,
    count: usize,
    position: [f64; 2],
    route: Route,
    mods: Value,
    source_player: Value,
    counted: Option<bool>,
}
impl Children {
    pub(super) fn clear_scaling(&mut self) {
        self.mods = Value::Null;
    }
    pub(super) fn make_stationary(&mut self, w: &World, id: Id, rect: Rect) -> Result<(), String> {
        self.route = self.route.stationary(w.unit(id)?.position, rect)?;
        self.counted = Some(false);
        Ok(())
    }
}
impl Host {
    pub(in crate::battle::runtime) fn enemy_children(
        &self,
        w: &World,
        id: Id,
    ) -> Result<Option<Children>, String> {
        let content = self
            .content
            .as_ref()
            .ok_or("Missing enemy content context")?;
        let Kit::Split(split) = content.kit(id) else {
            return Ok(None);
        };
        let count = content.split_count(id, split);
        if count == 0 {
            return Ok(None);
        }
        let Some(child) = content.child(id) else {
            return Ok(None);
        };
        self.enemy_children_spec(w, id, child, count).map(Some)
    }
    pub(super) fn enemy_children_spec(
        &self,
        w: &World,
        id: Id,
        child: Arc<str>,
        count: usize,
    ) -> Result<Children, String> {
        let spawns = self.spawns.as_ref().ok_or("Missing child spawn context")?;
        let entry = spawns
            .ledger
            .alive()
            .get(&id)
            .ok_or("Missing parent spawn record")?;
        let mut mods = entry.mods.clone();
        if let Some(m) = mods.as_object_mut() {
            m.remove("bountyId");
            m.remove("bountyCoins");
        }
        let u = w.unit(id)?;
        let route = spawns.remaining_route(w, id)?;
        Ok(Children {
            key: child,
            count,
            position: u.position,
            route,
            mods,
            source_player: entry.source_player.clone(),
            counted: None,
        })
    }

    pub(in crate::battle::runtime) fn enemy_spawn_children(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        children: Children,
    ) -> Result<(), String> {
        // The enemy index is a phase snapshot. Children born after EnemyIndex
        // join range queries when the next EnemyIndex phase rebuilds it.
        self.with_spawns(w, r, |spawns, context, world| {
            for i in 0..children.count {
                let offset = if children.count > 1 {
                    (i as f64 - (children.count as f64 - 1.) / 2.) * 0.2
                } else {
                    0.
                };
                let position = [children.position[0], children.position[1] + offset];
                spawns.spawn_child(
                    world,
                    context,
                    &children.key,
                    children.route.clone(),
                    position,
                    &children.mods,
                    &children.source_player,
                    children.counted,
                    None,
                )?;
            }
            Ok(())
        })
    }
    pub(super) fn enemy_split_attack(
        &mut self,
        c: &enemy::Context,
        r: &mut buffs::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        if self.content.as_mut().unwrap().spend_blade(c.enemy) {
            r.remove(w, self, c.enemy, "ab:blade")?;
        }
        Ok(())
    }
    pub(super) fn enemy_split_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        split: Split,
        dt: f64,
    ) -> Result<(), String> {
        match split {
            Split::Blades { .. } => {
                if self.content.as_mut().unwrap().clear_blade_cooldown(id) {
                    w.unit_mut(id)?.cadence.cooldown = 0.;
                }
            }
            Split::Veil {
                radius, duration, ..
            } => {
                let time = w.time;
                let u = w.unit_mut(id)?;
                let castable = !u.hidden
                    && !u
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("stun")
                    && time >= u.motion.unbalance_until - 1e-9;
                if self.content.as_mut().unwrap().veil_ready(id, dt, castable) {
                    for other in self.enemy_nearby(w, id, radius)? {
                        if other == id || duration <= 0. {
                            continue;
                        }
                        let mut b = buff("ab:veiled", &[]);
                        b.runtime.duration = duration;
                        b.runtime.visible = true;
                        b.runtime.stealth_restore = Some(0.);
                        b.flags.insert("stealth".into());
                        self.enemy_buff(w, r, other, b)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn enemy_teapot_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        let Kit::Split(Split::Teapot { duration, aspd, .. }) =
            self.content.as_ref().unwrap().kit(id)
        else {
            return Ok(());
        };
        if silenced(w, id)? {
            return Ok(());
        }
        let amount = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("atk");
        for target in enemy::Hooks::allies_in_radius(self, id, 1.25, w, w.unit_capacity())? {
            if w.unit(target)?.motion_fly
                || !crate::battle::targeting::area_selectable(w, Some(id), target)?
            {
                continue;
            }
            let mut damage = ability_damage(None, target, amount, DamageType::Arts);
            damage.tags.push("teaBoom".into());
            self.enemy_damage(w, r, &mut damage)?;
            if w.unit(target)?.stats.alive && duration > 0. && aspd != 0. {
                let mut b = buff(&format!("ab:teaBoom:{id}"), &[("aspd", aspd)]);
                b.runtime.duration = duration;
                b.runtime.visible = true;
                self.enemy_buff(w, r, target, b)?;
            }
        }
        Ok(())
    }
}
