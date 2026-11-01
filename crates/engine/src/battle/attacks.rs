// SPDX-License-Identifier: GPL-3.0-or-later
// Main-target payload path of ai.js performAttack/resolveHit/doHeal.
// Projectile delivery uses prepare -> impact callback -> finish; finish follows
// launch, not arrival. Splash/chain/profession payloads need separate dispatch.
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
    pub target: Option<Id>,
    pub attack_id: u64,
    pub amount: f64,
    pub is_skill: bool,
    pub heal: bool,
}
pub struct Prepared {
    pub context: Context,
    pub attack_id: u64,
    pub payload: Direct,
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
    fn validate(&self, p: Direct) -> Result<(), String> {
        if p.hits == 0 || p.hits > self.max_hits {
            return Err("Attack payload capacity exceeded; no instances dropped".into());
        }
        Ok(())
    }
    pub fn prepare<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        targets: &[Id],
        p: Direct,
    ) -> Result<Option<Prepared>, String> {
        self.validate(p)?;
        if targets.len() > self.max_targets {
            return Err("Attack target capacity exceeded; no instances dropped".into());
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
                return Ok(None);
            }
        }
        let next = self
            .sequence
            .checked_add(1)
            .ok_or("Attack identity exhausted")?;
        let time = w.time;
        let u = w.unit_mut(id)?;
        let count = u
            .attacks
            .checked_add(1)
            .ok_or("Unit attack count exhausted")?;
        u.last_attack_at = time;
        u.attacks = count;
        self.sequence = next;
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
        Ok(Some(Prepared {
            context: c,
            attack_id: next,
            payload: p,
        }))
    }
    /// Read current ATK at impact while retaining the launch-time profile/skill scale.
    pub fn impact<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        target: Option<Id>,
        attack_id: u64,
        p: Direct,
        mut skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        self.validate(p)?;
        let (atk, atk_mul) = {
            let s = w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
            (s.n("atk"), s.n("atkScaleMul"))
        };
        let scale = p.atk_scale * atk_mul;
        let (amount, heal) = match p.mode {
            Mode::Heal => {
                let target = target.ok_or("A direct heal requires a target")?;
                let amount = atk * (p.atk_scale * p.heal_scale * atk_mul);
                health::heal(w, h, Some(id), target, amount, HealOptions::default())?;
                (amount, true)
            }
            Mode::Damage(kind) => {
                let mut total = 0.0;
                let live_target = match target {
                    Some(id) if w.unit(id)?.stats.alive => Some(id),
                    _ => None,
                };
                if let Some(target) = live_target {
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
                    h.each_hit(
                        Impact {
                            attacker: id,
                            target: Some(target),
                            attack_id,
                            amount: total,
                            is_skill: p.is_skill,
                            heal: false,
                        },
                        skill.as_deref_mut(),
                        w,
                    )?;
                }
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
            skill,
            w,
        )?;
        Ok(())
    }
    pub fn finish<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        mut prepared: Prepared,
        mut skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        let c = &mut prepared.context;
        h.attack(c, skill.as_deref_mut(), w)?;
        if let Some(s) = skill.as_deref_mut() {
            s.after_attack(w, h, &c.targets, prepared.payload.is_skill, c.no_ammo)?;
        }
        h.after_attack(c, skill, w)
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
        let Some(prepared) = self.prepare(w, h, id, targets, p)? else {
            return Ok(());
        };
        for &target in &prepared.context.targets {
            self.impact(
                w,
                h,
                id,
                Some(target),
                prepared.attack_id,
                p,
                skill.as_deref_mut(),
            )?;
        }
        self.finish(w, h, prepared, skill)
    }
}
