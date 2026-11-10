// SPDX-License-Identifier: GPL-3.0-or-later
// Payload path of ai.js performAttack/resolveHit/doHeal.
// Projectile delivery uses prepare -> impact callback -> finish; finish follows
// launch, not arrival. Profession-specific payloads need separate dispatch.
use super::{
    damage::{Damage, DamageType, Penetration},
    health::{self, HealOptions, Hit, Id, Side, World},
    hit::Request,
    skills::Runtime,
    targeting::{self, Profile, Queries},
};
use crate::deterministic::math::{hypot, powi};
#[derive(Clone, Copy, Debug)]
pub enum Mode {
    Damage(DamageType),
    Heal,
}
#[derive(Clone, Copy, Debug)]
pub struct Direct {
    pub attachment: Option<Attachment>,
    pub element_heal_ratio: f64,
    pub mode: Mode,
    pub hits: u32,
    pub atk_scale: f64,
    pub skill_damage_mul: f64,
    pub target_damage_mul: f64,
    pub split_mul: Option<f64>,
    pub heal_scale: f64,
    pub is_skill: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Attachment {
    pub count: u32,
    pub initial: f64,
    pub delta: f64,
    pub cap: f64,
    pub per_target: bool,
    pub scale: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct Splash {
    pub radius: f64,
    pub scale: f64,
    pub hits_fly: bool,
    pub fortress_melee: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Chain {
    /// Total victims including the main target. Fractional counts follow JS loop semantics.
    pub count: f64,
    pub radius: f64,
    pub falloff: f64,
    pub sluggish: f64,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Area {
    pub splash: Option<Splash>,
    pub chain: Option<Chain>,
    pub fortress_melee: bool,
    pub selection: Profile,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VictimKind {
    Main,
    Splash,
    Chain,
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
    pub kind: VictimKind,
    pub main: Option<Id>,
    pub position: [f64; 2],
}
pub struct Prepared {
    pub context: Context,
    pub attack_id: u64,
    pub payload: Direct,
}
pub trait Hooks: super::skills::Hooks + health::Hooks + Queries {
    fn heal(&mut self, w: &mut World, source: Id, target: Id, amount: f64) -> Result<f64, String>
    where
        Self: Sized,
    {
        health::heal(
            w,
            self,
            Some(source),
            target,
            amount,
            HealOptions::default(),
        )
    }

    fn prepare_attachment(
        &mut self,
        _world: &mut World,
        _source: Id,
        _target: Option<Id>,
        _payload: &mut Direct,
    ) -> Result<(), String> {
        Ok(())
    }
    fn attachment_damage(
        &mut self,
        _world: &mut World,
        _source: Id,
        _target: Id,
        _attack_id: u64,
        _payload: Direct,
    ) -> Result<f64, String> {
        Ok(0.)
    }
    fn has_before_attack(&self) -> bool;
    fn before_attack(&mut self, c: &mut Context, w: &mut World) -> Result<(), String>;
    fn engage(&mut self, id: Id, w: &mut World) -> Result<(), String>;
    fn damage(
        &mut self,
        w: &mut World,
        request: &mut Request,
        skill: Option<&mut Runtime>,
    ) -> Result<f64, String>;
    /// Raw body-radius or selectable centre-radius query, in battle enemy order.
    fn radius_targets(
        &mut self,
        origin: [f64; 2],
        radius: f64,
        splash: bool,
        w: &mut World,
        limit: usize,
    ) -> Result<Vec<Id>, String>;
    fn chain_link(&mut self, from: Id, to: Id, w: &mut World) -> Result<(), String>;
    fn sluggish(
        &mut self,
        source: Id,
        target: Id,
        duration: f64,
        w: &mut World,
    ) -> Result<(), String>;
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
    max_targets: usize,
    max_hits: u32,
}
impl Executor {
    pub fn new(max_targets: usize, max_hits: u32) -> Result<Self, String> {
        if max_targets == 0 || max_hits == 0 {
            return Err("Attack execution limits must be positive".into());
        }
        Ok(Self {
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
        let time = w.time;
        let count = w
            .unit(id)?
            .attacks
            .checked_add(1)
            .ok_or("Unit attack count exhausted")?;
        let next = w.next_attack_id()?;
        let u = w.unit_mut(id)?;
        u.last_attack_at = time;
        u.attacks = count;
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
        skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        let position = match target {
            Some(id) => w.unit(id)?.position,
            None => [0.0, 0.0],
        };
        self.impact_at(
            w,
            h,
            id,
            target,
            attack_id,
            p,
            Area::default(),
            position,
            skill,
        )
    }
    pub fn impact_at<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        target: Option<Id>,
        attack_id: u64,
        mut p: Direct,
        area: Area,
        mut position: [f64; 2],
        mut skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        self.validate(p)?;
        if p.attachment.is_some() {
            h.prepare_attachment(w, id, target, &mut p)?;
        }
        if let Some(chain) = area.chain {
            // k < count: a fractional count may produce ceil(count) victims.
            let count = if chain.count == 0.0 {
                3.0
            } else {
                chain.count.max(1.0)
            };
            if !chain.count.is_finite()
                || count.ceil() > self.max_targets as f64
                || count.ceil() > i32::MAX as f64
            {
                return Err("Chain victim capacity exceeded; no jumps dropped".into());
            }
            if !chain.falloff.is_finite()
                || !chain.radius.is_finite()
                || !chain.sluggish.is_finite()
            {
                return Err("Invalid chain parameters".into());
            }
        }
        let (atk, atk_mul) = {
            let s = w.unit_mut(id)?.stats.stats().map_err(str::to_string)?;
            (s.n("atk"), s.n("atkScaleMul"))
        };
        let scale = p.atk_scale * atk_mul;
        let (amount, heal) = match p.mode {
            Mode::Heal => {
                let target = target.ok_or("A direct heal requires a target")?;
                if p.element_heal_ratio > 0. {
                    super::element::heal(w, target, atk * p.element_heal_ratio, None)?;
                }
                let amount = atk * (p.atk_scale * p.heal_scale * atk_mul);
                h.heal(w, id, target, amount)?;
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
                        total += h.damage(w, &mut request, skill.as_deref_mut())?;
                    }
                    if p.attachment.is_some() {
                        total += h.attachment_damage(w, id, target, attack_id, p)?;
                    }
                    h.each_hit(
                        Impact {
                            attacker: id,
                            target: Some(target),
                            attack_id,
                            amount: total,
                            is_skill: p.is_skill,
                            heal: false,
                            kind: VictimKind::Main,
                            main: Some(target),
                            position,
                        },
                        skill.as_deref_mut(),
                        w,
                    )?;
                    position = w.unit(target)?.position;
                }
                if let Some(splash) = area
                    .splash
                    .filter(|s| s.radius > 0.0 && (!area.fortress_melee || s.fortress_melee))
                {
                    let victims =
                        h.radius_targets(position, splash.radius, true, w, self.max_targets)?;
                    if victims.len() > self.max_targets {
                        return Err("Splash victim capacity exceeded".into());
                    }
                    for victim in victims {
                        if Some(victim) == target {
                            continue;
                        }
                        let u = w.unit_mut(victim)?;
                        let flying = u
                            .stats
                            .is_flying(u.side == Side::Enemy, u.motion_fly)
                            .map_err(str::to_string)?;
                        if area.selection.ground_only && flying
                            || !area.selection.can_hit_fly && flying && !splash.hits_fly
                            || u.stats
                                .stats()
                                .map_err(str::to_string)?
                                .flags
                                .contains("untargetable")
                        {
                            continue;
                        }
                        let amount = atk * scale * splash.scale * p.skill_damage_mul;
                        let mut r = single_hit(id, victim, attack_id, kind, amount, p.is_skill);
                        r.is_splash = true;
                        r.attack_type = super::hit::AttackType::Splash;
                        let dealt = h.damage(w, &mut r, skill.as_deref_mut())?;
                        total += dealt;
                        h.each_hit(
                            Impact {
                                attacker: id,
                                target: Some(victim),
                                attack_id,
                                amount: dealt,
                                is_skill: p.is_skill,
                                heal: false,
                                kind: VictimKind::Splash,
                                main: target,
                                position,
                            },
                            skill.as_deref_mut(),
                            w,
                        )?;
                    }
                }
                if let (Some(chain), Some(main)) = (area.chain, target) {
                    let mut seen = vec![main];
                    let mut prev = main;
                    let n = if chain.count == 0.0 {
                        3.0
                    } else {
                        chain.count.max(1.0)
                    };
                    let radius = if chain.radius == 0.0 {
                        1.7
                    } else {
                        chain.radius
                    };
                    let mut k = 1;
                    while (k as f64) < n {
                        let origin = w.unit(prev)?.position;
                        let candidates =
                            h.radius_targets(origin, radius, false, w, self.max_targets)?;
                        if candidates.len() > self.max_targets {
                            return Err("Chain candidate capacity exceeded".into());
                        }
                        let mut best = None;
                        let mut bd = f64::INFINITY;
                        for e in candidates {
                            if seen.contains(&e)
                                || !targeting::can_target_enemy(w, h, Some(id), e, area.selection)?
                            {
                                continue;
                            }
                            let [y, x] = w.unit(e)?.position;
                            let d = hypot(x - origin[1], y - origin[0]);
                            let earlier = match best {
                                Some(old) => {
                                    w.unit(e)?.targeting.spawn_seq
                                        < w.unit(old)?.targeting.spawn_seq
                                }
                                None => false,
                            };
                            if d < bd - 1e-9 || (d - bd).abs() <= 1e-9 && earlier {
                                bd = d;
                                best = Some(e);
                            }
                        }
                        let Some(victim) = best else {
                            break;
                        };
                        seen.push(victim);
                        h.chain_link(prev, victim, w)?;
                        let amount =
                            atk * scale * p.skill_damage_mul * powi(1.0 - chain.falloff, k)?;
                        let mut r = single_hit(id, victim, attack_id, kind, amount, p.is_skill);
                        r.tags.push("chain".into());
                        let dealt = h.damage(w, &mut r, skill.as_deref_mut())?;
                        total += dealt;
                        if chain.sluggish != 0.0 && w.unit(victim)?.stats.alive {
                            h.sluggish(id, victim, chain.sluggish, w)?;
                        }
                        h.each_hit(
                            Impact {
                                attacker: id,
                                target: Some(victim),
                                attack_id,
                                amount: dealt,
                                is_skill: p.is_skill,
                                heal: false,
                                kind: VictimKind::Chain,
                                main: target,
                                position,
                            },
                            skill.as_deref_mut(),
                            w,
                        )?;
                        prev = victim;
                        k += 1;
                    }
                    if chain.sluggish != 0.0 && w.unit(main)?.stats.alive {
                        h.sluggish(id, main, chain.sluggish, w)?;
                    }
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
                kind: VictimKind::Main,
                main: target,
                position,
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
        skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        self.perform_area(w, h, id, targets, p, Area::default(), skill)
    }
    pub fn perform_area<H: Hooks>(
        &mut self,
        w: &mut World,
        h: &mut H,
        id: Id,
        targets: &[Id],
        p: Direct,
        area: Area,
        mut skill: Option<&mut Runtime>,
    ) -> Result<(), String> {
        let Some(prepared) = self.prepare(w, h, id, targets, p)? else {
            return Ok(());
        };
        for &target in &prepared.context.targets {
            let position = w.unit(target)?.position;
            self.impact_at(
                w,
                h,
                id,
                Some(target),
                prepared.attack_id,
                p,
                area,
                position,
                skill.as_deref_mut(),
            )?;
        }
        self.finish(w, h, prepared, skill)
    }
}
fn single_hit(
    source: Id,
    target: Id,
    attack_id: u64,
    kind: DamageType,
    amount: f64,
    is_skill: bool,
) -> Request {
    let mut r = Request::new(
        Hit {
            target,
            source: Some(source),
            credit: Some(source),
            damage_type: kind,
            no_sp: false,
            silent: false,
            hp_loss: false,
            on_applied: false,
        },
        Damage {
            amount,
            mul: 1.0,
            penetration: Penetration::default(),
            resolved: false,
        },
    );
    r.attack_id = Some(attack_id);
    r.is_attack = true;
    r.is_skill = is_skill;
    r
}
