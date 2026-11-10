// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::battle::content::enemies::bosses::helm::{BladePhase, DroneLink, ShellTarget};
use crate::deterministic::math::hypot;
mod parts;
pub(in crate::battle::runtime) const PART_DOT: u64 = 1713;
pub(in crate::battle::runtime) const BLADE_RETURN: u64 = 1714;
impl Host {
    fn helm_range(w: &World, id: Id) -> Result<f64, String> {
        let r = w.unit(id)?.stats.base_number("rangeRadius");
        Ok(if r != 0. { r } else { 8. })
    }
    fn helm_ops(&mut self, w: &mut World, id: Id, radius: f64) -> Result<Vec<Id>, String> {
        Ok(self
            .boss_targets(w, id, radius)?
            .into_iter()
            .filter(|&t| {
                w.unit(t)
                    .is_ok_and(|u| u.deployment.kind == crate::battle::deploy::Kind::Operator)
            })
            .collect())
    }
    pub(super) fn helm_condition(
        &mut self,
        w: &mut World,
        id: Id,
        spell: Spell,
    ) -> Result<Option<bool>, String> {
        Ok(Some(match spell {
            Spell::HelmShell => !self.helm_ops(w, id, Self::helm_range(w, id)?)?.is_empty(),
            Spell::HelmDrones { .. } => true,
            Spell::BladeDive => {
                matches!(
                    self.content.as_ref().unwrap().boss(id),
                    Memory::Blade {
                        phase: BladePhase::Hover,
                        ..
                    }
                ) && !self.helm_ops(w, id, f64::INFINITY)?.is_empty()
            }
            _ => return Ok(None),
        }))
    }
    pub(super) fn helm_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
    ) -> Result<(), String> {
        match spell {
            Spell::HelmShell => {
                let mut targets = self.helm_ops(w, id, Self::helm_range(w, id)?)?;
                targeting::sort_allies(w, id, &mut targets, Default::default())?;
                let mut keyed = vec![];
                for t in targets {
                    keyed.push((t, Self::tac_stat(w, t, "atk")?));
                }
                keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
                if let Some(&(target, _)) = keyed.first() {
                    let tile = w.unit(target)?.deployment.tile;
                    let position = w.unit(id)?.position;
                    let options = serde_json::json!({"pos":position,"route":{"motion":"FLY","start":position,"end":tile,"steps":[{"t":"WAIT","s":99999}]},"tag":"part","countInTotal":false});
                    let shell = self.with_spawns(w, r, |spawns, context, world| {
                        spawns.spawn_runtime(world, context, "enemy_9016_acstmr", &options)
                    })?;
                    w.pin_unit(id)?;
                    self.content.as_mut().unwrap().set_boss(
                        shell,
                        Memory::Shell {
                            target: Some(ShellTarget { tile, source: id }),
                        },
                    );
                    self.rebuild_index(w)?;
                }
            }
            Spell::HelmDrones { ratio, hp } => {
                let content = self.content.as_ref().unwrap();
                let key = content
                    .child(id)
                    .unwrap_or_else(|| Arc::from("enemy_1005_yokai"));
                let branch = content
                    .branch(id)
                    .unwrap_or_else(|| Arc::from("boss_summon_enemy"));
                let mods = if hp > 0. {
                    serde_json::json!({"hpMul":hp})
                } else {
                    Value::Null
                };
                for drone in self.boss_spawn_branch(w, r, id, &branch, &key, mods, "FLY")? {
                    w.pin_unit(id)?;
                    self.content
                        .as_mut()
                        .unwrap()
                        .set_drone(drone, Some(DroneLink { owner: id, ratio }));
                }
            }
            Spell::BladeDive => self.blade_dive(w, r, id)?,
            _ => unreachable!(),
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn helm_attack(
        &mut self,
        w: &mut World,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        if event != enemy::Event::BeforeAttack {
            return Ok(());
        }
        match self.boss_config(c.enemy) {
            Some(Boss::Helm(_)) => {
                let targets = self.boss_targets(w, c.enemy, Self::helm_range(w, c.enemy)?)?;
                if let Some(t) = self.rng.pick(&targets).copied() {
                    c.targets = vec![t];
                }
            }
            Some(Boss::Blade(_)) => {
                let radius = w.unit(c.enemy)?.stats.base_number("rangeRadius");
                let targets =
                    self.boss_targets(w, c.enemy, if radius != 0. { radius } else { 1.6 })?;
                let mut ground = vec![];
                for t in targets {
                    if !targeting::flying(w, t)? {
                        ground.push(t);
                    }
                }
                if !ground.is_empty() {
                    c.targets = ground;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn helm_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        boss: Boss,
        dt: f64,
    ) -> Result<(), String> {
        match boss {
            Boss::Helm(config) => {
                for _ in 0..self.content.as_mut().unwrap().pulses(id, dt, 0.25) {
                    if matches!(
                        self.content.as_ref().unwrap().boss(id),
                        Memory::Helm { low: false }
                    ) && w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")? < config.low
                    {
                        self.content
                            .as_mut()
                            .unwrap()
                            .set_boss(id, Memory::Helm { low: true });
                        let mut b = buff(
                            "boss:helmGuard",
                            &[
                                ("physTakenMul", config.scale),
                                ("artsTakenMul", config.scale),
                            ],
                        );
                        b.runtime.persist = true;
                        b.runtime.visible = true;
                        self.enemy_buff(w, r, id, b)?;
                    }
                }
                self.enemy_caster_tick(w, r, id, config.caster, dt)?;
            }
            Boss::Shell { stun, dot } => {
                let Memory::Shell {
                    target: Some(target),
                } = self.content.as_ref().unwrap().boss(id)
                else {
                    return Ok(());
                };
                if self.helm_step(w, id, target.tile, dt)? {
                    let source = if w.unit(target.source)?.stats.alive {
                        target.source
                    } else {
                        id
                    };
                    self.helm_blast(w, r, source, id, target.tile, stun, dot, stun, false)?;
                    self.kill_enemy(id, None, r, w)?;
                }
            }
            Boss::Blade(config) => {
                self.blade_tick(w, r, id, config, dt)?;
                if w.unit(id)?.stats.alive {
                    self.enemy_caster_tick(w, r, id, config.caster, dt)?;
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn helm_step(
        &mut self,
        w: &mut World,
        id: Id,
        tile: [i32; 2],
        dt: f64,
    ) -> Result<bool, String> {
        let p = w.unit(id)?.position;
        let target = tile.map(|v| v as f64);
        let d = hypot(target[1] - p[1], target[0] - p[0]);
        let step = Self::tac_stat(w, id, "moveSpeed")? * 0.5 * dt;
        if d <= step + 1e-9 {
            w.unit_mut(id)?.position = target;
            return Ok(true);
        }
        let u = w.unit_mut(id)?;
        u.position[0] += (target[0] - p[0]) / d * step;
        u.position[1] += (target[1] - p[1]) / d * step;
        Ok(false)
    }
    pub(super) fn helm_profile(&self, kit: &Kit, p: &mut enemy::Profile) {
        if let Kit::Boss(Boss::Blade(config)) = kit {
            if config.hammer {
                p.atk_scale = 1.5;
            }
        }
    }
    fn helm_leader(&self, w: &World, id: Id) -> Result<Option<Id>, String> {
        let p = w.unit(id)?.position;
        let mut best = None;
        let mut distance = f64::INFINITY;
        for &t in self.spawns.as_ref().unwrap().live() {
            if !w.unit(t)?.boss
                || !w.unit(t)?.stats.alive
                || !matches!(self.boss_config(t), Some(Boss::Helm(_)))
            {
                continue;
            }
            let at = w.unit(t)?.position;
            let d = hypot(at[1] - p[1], at[0] - p[0]);
            if d < distance {
                best = Some(t);
                distance = d;
            }
        }
        Ok(best)
    }
    pub(in crate::battle::runtime) fn helm_taken(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        source: Option<Id>,
        credit: Option<Id>,
        amount: f64,
    ) -> Result<(), String> {
        let Some(Boss::Blade(config)) = self.boss_config(id) else {
            return Ok(());
        };
        if amount > 0. {
            if let Some(leader) = self.helm_leader(w, id)? {
                let by = source
                    .or(credit)
                    .filter(|&t| w.unit(t).is_ok_and(|u| u.side == health::Side::Ally));
                self.lose_hp_with_credit(
                    w,
                    r,
                    health::Hit {
                        source: None,
                        credit: by,
                        target: leader,
                        damage_type: DamageType::True,
                        no_sp: true,
                        silent: false,
                        hp_loss: true,
                        on_applied: false,
                    },
                    amount,
                    false,
                )?;
            }
        }
        if let Memory::Blade {
            phase: BladePhase::Dive { tile, hits },
            home,
        } = self.content.as_ref().unwrap().boss(id)
        {
            if w.unit(id)?.stats.alive {
                let hits = hits.checked_add(1).ok_or("Blade hit count overflow")?;
                self.content.as_mut().unwrap().set_boss(
                    id,
                    Memory::Blade {
                        phase: BladePhase::Dive { tile, hits },
                        home,
                    },
                );
                if hits as f64 >= config.hits {
                    self.blade_down(w, r, id, config, home)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn helm_detach(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        if let Some(link) = self.content.as_ref().unwrap().drone(id) {
            self.content.as_mut().unwrap().set_drone(id, None);
            w.unpin_unit(link.owner)?;
        }
        if let Memory::Shell {
            target: Some(target),
        } = self.content.as_ref().unwrap().boss(id)
        {
            self.content
                .as_mut()
                .unwrap()
                .set_boss(id, Memory::Shell { target: None });
            w.unpin_unit(target.source)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn helm_death(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        if let Some(link) = self.content.as_ref().unwrap().drone(id) {
            let key = self
                .spawns
                .as_ref()
                .unwrap()
                .definition(link.owner)?
                .key
                .clone();
            let owner = if w.unit(link.owner)?.stats.alive {
                Some(link.owner)
            } else {
                self.spawns
                    .as_ref()
                    .unwrap()
                    .live()
                    .iter()
                    .copied()
                    .find(|&t| {
                        w.unit(t).is_ok_and(|u| u.stats.alive && u.boss)
                            && self
                                .spawns
                                .as_ref()
                                .unwrap()
                                .definition(t)
                                .is_ok_and(|d| d.key == key)
                    })
            };
            if let Some(owner) = owner.filter(|_| link.ratio > 0.) {
                let amount = Self::tac_stat(w, owner, "maxHp")? * link.ratio;
                let by = credit.filter(|&t| w.unit(t).is_ok_and(|u| u.side == health::Side::Ally));
                self.lose_hp_with_credit(
                    w,
                    r,
                    health::Hit {
                        source: by,
                        credit: by,
                        target: owner,
                        damage_type: DamageType::True,
                        no_sp: true,
                        silent: false,
                        hp_loss: true,
                        on_applied: false,
                    },
                    amount,
                    true,
                )?;
            }
        }
        self.helm_detach(w, id)
    }
}
