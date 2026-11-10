// SPDX-License-Identifier: GPL-3.0-or-later
use super::ensemble::AreaStrike;
use super::*;
use crate::battle::{content::enemies::bosses::Quintus, element::Element};
impl Host {
    fn quintus_targets(&mut self, w: &mut World, id: Id, radius: f64) -> Result<Vec<Id>, String> {
        let mut targets = self.boss_targets(w, id, radius)?;
        targeting::sort_allies(
            w,
            id,
            &mut targets,
            targeting::Profile {
                block_free: true,
                ..Default::default()
            },
        )?;
        let mut keyed = vec![];
        for t in targets {
            keyed.push((t, Self::tac_stat(w, t, "def")?));
        }
        keyed.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(keyed.into_iter().map(|x| x.0).collect())
    }
    pub(super) fn quintus_condition(&self, id: Id, spell: Spell) -> Option<bool> {
        match spell {
            Spell::Tide { stage, .. } | Spell::Rockfall { stage, .. } => Some(
                matches!(self.content.as_ref().unwrap().boss(id),Memory::Quintus(s) if s.stage==stage),
            ),
            Spell::Tentacles { stage } => {
                let Memory::Quintus(s) = self.content.as_ref().unwrap().boss(id) else {
                    return Some(false);
                };
                let branch = self
                    .content
                    .as_ref()
                    .unwrap()
                    .tentacle_branch(id)
                    .unwrap_or_else(|| Arc::from("dslily_dstnta"));
                let phases = self.spawns.as_ref().unwrap().branch_phases(&branch);
                Some(s.stage == stage && (phases == 0 || s.tentacles < phases))
            }
            Spell::Species { .. } => Some(
                matches!(self.content.as_ref().unwrap().boss(id),Memory::Quintus(s) if s.stage==2),
            ),
            _ => None,
        }
    }
    pub(super) fn quintus_fire(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        spell: Spell,
    ) -> Result<(), String> {
        match spell {
            Spell::Tide { scale, element, .. } => self.boss_arts_area(
                w,
                r,
                id,
                AreaStrike {
                    position: w.unit(id)?.position,
                    radius: f64::INFINITY,
                    scale,
                    element: Element::Neural,
                    ratio: element,
                    attack: None,
                },
            )?,
            Spell::Rockfall { stage, scale } => {
                for target in self
                    .quintus_targets(w, id, f64::INFINITY)?
                    .into_iter()
                    .take([2, 4, 8][stage as usize])
                {
                    self.enemy_schedule(
                        w,
                        Some(id),
                        1.,
                        0.,
                        1,
                        EnemyTask::Boss(BossTask::Rockfall { target, scale }),
                    )?;
                }
            }
            Spell::Tentacles { .. } => {
                let Memory::Quintus(mut state) = self.content.as_ref().unwrap().boss(id) else {
                    unreachable!()
                };
                let branch = self
                    .content
                    .as_ref()
                    .unwrap()
                    .tentacle_branch(id)
                    .unwrap_or_else(|| Arc::from("dslily_dstnta"));
                let mut spots = self
                    .spawns
                    .as_ref()
                    .unwrap()
                    .tentacle_spots(&branch, state.tentacles);
                if spots.is_empty() {
                    let targets = self.boss_targets(w, id, f64::INFINITY)?;
                    if let Some(&t) = self.rng.pick(&targets) {
                        spots.push(w.unit(t)?.deployment.tile.map(|v| v as f64));
                    }
                }
                state.tentacles = state
                    .tentacles
                    .checked_add(1)
                    .ok_or("Tentacle phase count exhausted")?;
                self.content
                    .as_mut()
                    .unwrap()
                    .set_boss(id, Memory::Quintus(state));
                for spot in spots {
                    for t in self.boss_area(w, id)? {
                        let p = w.unit(t)?.position;
                        if crate::deterministic::math::hypot(p[1] - spot[1], p[0] - spot[0]) <= 1. {
                            self.apply_status(
                                w,
                                r,
                                t,
                                "stun",
                                status::Options {
                                    duration: 5.,
                                    source: Some(id),
                                    ..Default::default()
                                },
                            )?;
                        }
                    }
                }
            }
            Spell::Species { loss } => self.enemy_schedule(
                w,
                Some(id),
                4.,
                0.,
                1,
                EnemyTask::Boss(BossTask::Species { loss }),
            )?,
            _ => unreachable!(),
        }
        Ok(())
    }
    pub(super) fn quintus_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        config: Quintus,
        dt: f64,
    ) -> Result<(), String> {
        for _ in 0..self.content.as_mut().unwrap().pulses(id, dt, 0.25) {
            let Memory::Quintus(mut state) = self.content.as_ref().unwrap().boss(id) else {
                unreachable!()
            };
            if state.stage >= 2 {
                break;
            }
            let stage = state.stage as usize;
            let lost = 1. - w.unit(id)?.stats.hp / Self::tac_stat(w, id, "maxHp")?;
            let since = if stage == 0 {
                state.born_at
            } else {
                state.stage_at
            };
            if lost >= config.growth[stage] || w.time - since >= config.deadlines[stage] {
                state.stage += 1;
                state.stage_at = w.time;
                self.content
                    .as_mut()
                    .unwrap()
                    .set_boss(id, Memory::Quintus(state));
                let mut b = buff("boss:grow", &[("atkPct", config.attack[stage])]);
                b.runtime.persist = true;
                b.runtime.visible = true;
                self.enemy_buff(w, r, id, b)?;
                let caster = self
                    .content
                    .as_ref()
                    .unwrap()
                    .quintus_caster(id)
                    .ok_or("Missing Quintus stage skills")?;
                let mut clock = self.content.as_ref().unwrap().caster(id);
                clock.left = caster
                    .skills
                    .map(|s| s.map(|s| s.clock.initial).unwrap_or(0.));
                self.content.as_mut().unwrap().set_caster(id, clock);
            }
        }
        let caster = self
            .content
            .as_ref()
            .unwrap()
            .quintus_caster(id)
            .ok_or("Missing Quintus stage skills")?;
        self.enemy_caster_tick(w, r, id, caster, dt)
    }
    pub(in crate::battle::runtime::enemies) fn quintus_attack(
        &mut self,
        w: &mut World,
        c: &mut enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        if event == enemy::Event::BeforeAttack
            && matches!(self.boss_config(c.enemy), Some(Boss::Quintus(_)))
        {
            let radius = w.unit(c.enemy)?.stats.base_number("rangeRadius");
            let targets =
                self.quintus_targets(w, c.enemy, if radius != 0. { radius } else { 99. })?;
            if !targets.is_empty() {
                c.targets = targets.into_iter().take(2).collect();
            }
        }
        Ok(())
    }
    pub(super) fn quintus_dealt(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        target: Id,
    ) -> Result<(), String> {
        if let Some(Boss::Quintus(config)) = self.boss_config(id) {
            let amount = Self::tac_stat(w, id, "atk")? * config.neural;
            self.enemy_element(w, r, id, target, Element::Neural, amount)?;
        }
        Ok(())
    }
}
