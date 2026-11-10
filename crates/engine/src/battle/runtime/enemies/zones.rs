// SPDX-License-Identifier: GPL-3.0-or-later
use super::forms::FormEvent;
use super::*;
use crate::battle::{
    content::enemies::zones::{Cloud, Pollution, ZoneKit},
    projectiles, targeting,
};

#[derive(Clone, Copy)]
pub(in crate::battle::runtime) enum ZoneDamage {
    Pollution {
        low: f64,
        high: f64,
    },
    Arts {
        amount: f64,
        no_air: bool,
        poison: bool,
    },
}
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) struct Zone {
    pub position: [f64; 2],
    pub radius: f64,
    pub damage: ZoneDamage,
}
#[derive(Clone)]
pub(in crate::battle::runtime) enum EnemyTask {
    Catapult(super::catapult::SharedVolley),
    Boss(super::BossTask),
    Special(super::SpecialTask),
    Blast {
        position: [f64; 2],
        scale: f64,
        stun: f64,
    },
    Form(FormEvent),
    Spell(super::SpellTask),
    Barrel {
        position: [f64; 2],
        aspd: f64,
        dodge: f64,
    },
    Shell {
        position: [f64; 2],
        amount: f64,
    },
    Zone(Zone),
}
#[derive(Clone, Copy)]
pub(in crate::battle::runtime) struct CloudShot {
    pub cloud: Cloud,
    pub amount: f64,
}
impl EnemyTask {
    pub(in crate::battle::runtime) fn retained_target(&self) -> Option<Id> {
        match self {
            Self::Catapult(v) => v.target,
            Self::Special(t) => t.target(),
            Self::Spell(t) => t.target(),
            Self::Boss(t) => t.target(),
            _ => None,
        }
    }
}
impl Host {
    pub(super) fn enemy_schedule(
        &mut self,
        w: &mut World,
        source: Option<Id>,
        delay: f64,
        interval: f64,
        count: u32,
        task: EnemyTask,
    ) -> Result<(), String> {
        let seq = source
            .map(|id| w.unit(id).map(|u| u.deployment.deploy_seq))
            .transpose()?
            .unwrap_or(0);
        let references = [source, task.retained_target()];
        for (index, reference) in references.iter().enumerate() {
            if let Some(id) = reference {
                if let Err(error) = w.pin_unit(*id) {
                    for previous in references[..index].iter().flatten() {
                        w.unpin_unit(*previous)?;
                    }
                    return Err(error);
                }
            }
        }
        let result = self.enqueue_timer(super::super::tactics::Timer {
            source,
            seq,
            at: w.time + delay,
            interval,
            left: count,
            kind: super::super::tactics::Task::Enemy(task),
        });
        if result.is_err() {
            for id in references.into_iter().flatten() {
                w.unpin_unit(id)?;
            }
        }
        result
    }
    fn enemy_zone(
        &mut self,
        w: &mut World,
        source: Option<Id>,
        zone: Zone,
        life: f64,
        interval: f64,
    ) -> Result<(), String> {
        let interval = if interval > 0. {
            interval
        } else {
            super::super::DT
        };
        let count = ((life - 1e-6) / interval).ceil().max(1.);
        if !count.is_finite() || count > i32::MAX as f64 {
            return Err("Enemy zone pulse count outside i32 domain".into());
        }
        self.enemy_schedule(
            w,
            source,
            interval,
            interval,
            count as u32,
            EnemyTask::Zone(zone),
        )
    }
    pub(super) fn enemy_pollution(
        &mut self,
        w: &mut World,
        id: Id,
        position: [f64; 2],
        p: Pollution,
    ) -> Result<(), String> {
        self.enemy_zone(
            w,
            Some(id),
            Zone {
                position,
                radius: p.radius,
                damage: ZoneDamage::Pollution {
                    low: p.low,
                    high: p.high,
                },
            },
            p.life,
            1.,
        )
    }
    pub(in crate::battle::runtime) fn enemy_cloud(
        &mut self,
        w: &mut World,
        shot: CloudShot,
        position: [f64; 2],
    ) -> Result<(), String> {
        self.enemy_zone(
            w,
            None,
            Zone {
                position,
                radius: shot.cloud.radius,
                damage: ZoneDamage::Arts {
                    amount: shot.amount,
                    no_air: false,
                    poison: true,
                },
            },
            shot.cloud.life,
            shot.cloud.interval,
        )
    }
    pub(in crate::battle::runtime) fn enemy_task(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Option<Id>,
        task: &mut EnemyTask,
        last: bool,
    ) -> Result<bool, String> {
        if let EnemyTask::Catapult(volley) = task {
            self.enemy_catapult_stone(w, r, source.ok_or("Missing catapult source")?, volley)?;
            return Ok(true);
        }
        match *task {
            EnemyTask::Catapult(_) => unreachable!(),
            EnemyTask::Boss(t) => {
                return self.boss_task(w, r, source.ok_or("Missing boss task source")?, t)
            }
            EnemyTask::Special(t) => {
                return self.enemy_special_task(
                    w,
                    r,
                    source.ok_or("Missing special timer source")?,
                    t,
                )
            }
            EnemyTask::Blast {
                position,
                scale,
                stun,
            } => {
                let id = source.ok_or("Missing blast source")?;
                if !w.unit(id)?.stats.alive {
                    return Ok(false);
                }
                for t in spatial::allies_in_radius(
                    w,
                    self.roster.as_ref().unwrap().ids(),
                    position,
                    3.,
                    None,
                    false,
                    w.unit_capacity(),
                )? {
                    if !targeting::area_selectable(w, Some(id), t)? {
                        continue;
                    }
                    let amount = Self::tac_stat(w, id, "atk")? * scale;
                    let mut q = ability_damage(Some(id), t, amount, DamageType::Physical);
                    self.enemy_damage(w, r, &mut q)?;
                    if stun > 0. && w.unit(t)?.stats.alive {
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
            EnemyTask::Spell(t) => return self.enemy_spell_task(w, r, source, t, last),
            EnemyTask::Barrel {
                position,
                aspd,
                dodge,
            } => self.enemy_barrel_pulse(w, r, position, aspd, dodge)?,
            EnemyTask::Shell { position, amount } => {
                self.enemy_shell(w, r, source, position, amount)?
            }
            EnemyTask::Form(event) => {
                let id = source.ok_or("Missing form timer source")?;
                if !w.unit(id)?.stats.alive {
                    return Ok(false);
                }
                self.enemy_form_event(w, r, id, event)?;
            }
            EnemyTask::Zone(zone) => {
                let targets = spatial::allies_in_radius(
                    w,
                    self.roster.as_ref().unwrap().ids(),
                    zone.position,
                    zone.radius,
                    None,
                    false,
                    w.unit_capacity(),
                )?;
                for target in targets {
                    let (amount, kind, tag, ignore) = match zone.damage {
                        ZoneDamage::Pollution { low, high } => {
                            let time = w.time;
                            let u = w.unit_mut(target)?;
                            let amount = if u.ground { low } else { high };
                            if amount <= 0. || time - u.polluted_at < 1. - 1e-6 {
                                continue;
                            }
                            u.polluted_at = time;
                            (amount, DamageType::True, "pollution", true)
                        }
                        ZoneDamage::Arts {
                            amount,
                            no_air,
                            poison,
                        } => {
                            if no_air && w.unit(target)?.motion_fly
                                || !targeting::area_selectable(w, source, target)?
                            {
                                continue;
                            }
                            (
                                amount,
                                DamageType::Arts,
                                if poison { "poison" } else { "burning" },
                                false,
                            )
                        }
                    };
                    let mut damage = ability_damage(source, target, amount, kind);
                    damage.ignore_select = ignore;
                    damage.tags.push(tag.into());
                    self.enemy_damage(w, r, &mut damage)?;
                }
            }
        }
        Ok(true)
    }
    pub(super) fn enemy_zone_attack(
        &mut self,
        w: &mut World,
        c: &enemy::Context,
        event: enemy::Event,
    ) -> Result<(), String> {
        if event == enemy::Event::BeforeAttack {
            self.content.as_mut().unwrap().tank_before(c.enemy);
        }
        if event == enemy::Event::Attack {
            if let Kit::Zone(ZoneKit::Tank(tank)) = self.content.as_ref().unwrap().kit(c.enemy) {
                if self.content.as_mut().unwrap().tank_attack(c.enemy) {
                    if let Some(&target) = c.targets.first() {
                        self.enemy_pollution(w, c.enemy, w.unit(target)?.position, tank.pollution)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn enemy_zone_dealt(
        &mut self,
        w: &mut World,
        q: &hit::Request,
        id: Id,
    ) -> Result<(), String> {
        if let Kit::Zone(ZoneKit::Cannon {
            radius,
            life,
            amount,
        }) = self.content.as_ref().unwrap().kit(id)
        {
            self.enemy_zone(
                w,
                Some(id),
                Zone {
                    position: w.unit(q.hit.target)?.position,
                    radius,
                    damage: ZoneDamage::Arts {
                        amount,
                        no_air: true,
                        poison: false,
                    },
                },
                life,
                1.,
            )?;
        }
        Ok(())
    }
    pub(super) fn enemy_zone_death(
        &mut self,
        w: &mut World,
        id: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        match self.content.as_ref().unwrap().kit(id) {
            Kit::Zone(ZoneKit::Pollution(p)) if !silenced(w, id)? => {
                self.enemy_pollution(w, id, w.unit(id)?.position, p)?
            }
            Kit::Zone(ZoneKit::Poison(cloud)) if !silenced(w, id)? => {
                let shot = CloudShot {
                    cloud,
                    amount: w
                        .unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .n("atk")
                        * cloud.scale,
                };
                let target = credit.filter(|&k| {
                    w.unit(k)
                        .is_ok_and(|u| u.side == health::Side::Ally && u.stats.alive)
                });
                if let Some(target) = target {
                    self.flights
                        .as_mut()
                        .ok_or("Missing poison projectile system")?
                        .add(
                            w,
                            projectiles::Launch {
                                from: w.unit(id)?.position,
                                source: None,
                                target: Some(target),
                                to: None,
                                speed: 8.,
                                hit_dead: true,
                                max_age: 10.,
                                on_hit: true,
                                visual: "lob".into(),
                                speed_changeable: None,
                                removable: None,
                                payload: allies::Shot::Cloud(shot),
                            },
                        )?;
                } else {
                    self.enemy_cloud(w, shot, w.unit(id)?.position)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
