// SPDX-License-Identifier: GPL-3.0-or-later
// Direct main-target payload path of ai.js performAttack/resolveHit/doHeal.
// Projectile, splash, chain and profession-specific payloads must use their own
// dispatch paths; this type cannot represent them and never approximates them.
use super::{
    damage::{Damage, DamageType, Penetration},
    health::{self, HealOptions, Hit, Id, Side, World},
    hit::Request,
    skills::Runtime,
};
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Damage(DamageType),
    Heal,
}
#[derive(Clone, Copy, Debug)]
pub struct Direct {
    pub mode: Mode,
    pub hits: u32,
    pub atk_scale: f64,
    pub skill_damage_mul: f64,
    pub target_damage_mul: f64,
    pub split_mul: Option<f64>,
    pub heal_scale: f64,
    pub is_skill: bool,
}
#[derive(Clone, Debug)]
pub struct Context {
    pub attacker: Id,
    pub targets: Vec<Id>,
    pub is_skill: bool,
    pub no_ammo: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Impact {
    pub attacker: Id,
    pub target: Id,
    pub attack_id: u64,
    pub amount: f64,
    pub is_skill: bool,
    pub heal: bool,
}
pub trait Hooks: super::skills::Hooks + health::Hooks {
    fn has_before_attack(&self) -> bool;
    fn before_attack(&mut self, c: &mut Context, w: &mut World) -> Result<(), String>;
    fn engage(&mut self, id: Id, w: &mut World) -> Result<(), String>;
    fn damage(&mut self, w: &mut World, request: &mut Request) -> Result<f64, String>;
    fn each_hit(
        &mut self,
        impact: Impact,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String>;
    fn after_hit(
        &mut self,
        impact: Impact,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String>;
    fn attack(
        &mut self,
        c: &mut Context,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String>;
    fn after_attack(
        &mut self,
        c: &Context,
        skill: Option<&mut Runtime>,
        w: &mut World,
    ) -> Result<(), String>;
}
pub struct Executor {
    sequence: u64,
    max_targets: usize,
    max_hits: u32,
}
impl Executor {
    pub fn new(max_targets: usize, max_hits: u32) -> Result<Self, String> {
        if max_targets == 0 || max_hits == 0 {
            return Err("Attack execution limits must be positive".into());
        }
        Ok(Self {
            sequence: 0,
            max_targets,
            max_hits,
        })
    }
    pub fn perform<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        targets: &[Id],
        p: Direct,
        mut skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        if targets.len() > self.max_targets || p.hits == 0 || p.hits > self.max_hits {
            return Err("Attack payload capacity exceeded; no instances dropped".into());
        }
        let mut c = Context {
            attacker: id,
            targets: targets.to_vec(),
            is_skill: p.is_skill,
            no_ammo: false,
        };
        if h.has_before_attack() {
            h.before_attack(&mut c, w)?;
            if c.targets.len() > self.max_targets {
                return Err("Attack callback target capacity exceeded".into());
            }
            let mut live = Vec::with_capacity(c.targets.len());
            for target in c.targets {
                if w.unit(target)?.stats.alive {
                    live.push(target);
                }
            }
            c.targets = live;
            if c.targets.is_empty() || !w.unit(id)?.stats.alive {
                return Ok(());
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Attack identity exhausted")?;
        let attack_id = self.sequence;
        let time = w.time;
        let u = w.unit_mut(id)?;
        u.last_attack_at = time;
        u.attacks = u
            .attacks
            .checked_add(1)
            .ok_or("Unit attack count exhausted")?;
        if !matches!(p.mode, Mode::Heal)
            && u.side == Side::Ally
            && !u.engaged
            && c.targets
                .iter()
                .any(|&t| w.unit(t).map(|u| u.side == Side::Enemy).unwrap_or(false))
        {
            w.unit_mut(id)?.engaged = true;
            h.engage(id, w)?;
        }
        for &target in &c.targets {
            let (atk, scale) = {
                let s = w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
                (s.n("atk"), p.atk_scale * s.n("atkScaleMul"))
            };
            let (amount, heal) = match p.mode {
                Mode::Heal => {
                    let amount = atk
                        * (p.atk_scale
                            * p.heal_scale
                            * w.unit_mut(id)?
                                .stats
                                .stats()
                                .map_err(str::to_string)?
                                .n("atkScaleMul"));
                    health::heal(w, h, Some(id), target, amount, HealOptions::default())?;
                    (amount, true)
                }
                Mode::Damage(kind) => {
                    let mut total = 0.0;
                    let mut mul = p.skill_damage_mul;
                    if p.target_damage_mul.is_finite() && p.target_damage_mul != 0.0 {
                        mul *= p.target_damage_mul;
                    }
                    for hit in 0..p.hits {
                        if !w.unit(target)?.stats.alive {
                            break;
                        }
                        let split = p.split_mul.filter(|m| m.is_finite() && *m > 0.0);
                        let mut request = Request::new(
                            Hit {
                                target,
                                source: Some(id),
                                credit: Some(id),
                                damage_type: kind,
                                no_sp: split.is_some() && hit > 0,
                                silent: false,
                                hp_loss: false,
                                on_applied: false,
                            },
                            Damage {
                                amount: atk * scale * mul,
                                mul: split.unwrap_or(1.0),
                                penetration: Penetration::default(),
                                resolved: false,
                            },
                        );
                        request.attack_id = Some(attack_id);
                        request.is_attack = true;
                        request.is_skill = p.is_skill;
                        total += h.damage(w, &mut request)?;
                    }
                    let impact = Impact {
                        attacker: id,
                        target,
                        attack_id,
                        amount: total,
                        is_skill: p.is_skill,
                        heal: false,
                    };
                    h.each_hit(impact, skill.as_deref_mut(), w)?;
                    (total, false)
                }
            };
            h.after_hit(
                Impact {
                    attacker: id,
                    target,
                    attack_id,
                    amount,
                    is_skill: p.is_skill,
                    heal,
                },
                skill.as_deref_mut(),
                w,
            )?;
        }
        h.attack(&mut c, skill.as_deref_mut(), w)?;
        if let Some(s) = skill.as_deref_mut() {
            s.after_attack(w, h, &c.targets, p.is_skill, c.no_ammo)?;
        }
        h.after_attack(&c, skill, w)?;
        Ok(())
    }
}
