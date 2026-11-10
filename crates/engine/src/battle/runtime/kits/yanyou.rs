// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as C, Event as E};
use super::*;
use crate::battle::targeting::{self, Queries};
use crate::deterministic::math::hypot;
pub(crate) struct Yanyou {
    radius: f64,
    speed: f64,
    burn: f64,
    fragile: f64,
    scale: f64,
    flame_radius: f64,
}
pub(crate) struct State {
    pub keys: Vec<usize>,
    pub hover: f64,
    at: Option<[f64; 2]>,
    target: Option<Id>,
    acc: f64,
    aura: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            keys: vec![],
            hover: 0.,
            at: None,
            target: None,
            acc: 0.,
            aura: 0.2,
        }
    }
}
pub(crate) fn build(raw: &Value, s: &mut Spec) {
    let t = &raw["talents"]["bb"];
    let sk = raw["skills"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["prefabKey"] == "Skill_2"))
        .unwrap_or(&Value::Null);
    s.combat.profile.max_targets = n(t, "1.attack@max_target", 1.);
    s.extra.tactics = Tactics::Yanyou(Yanyou {
        radius: n(&raw["stats"], "rangeRadius", 2.),
        speed: n(&raw["stats"], "moveSpeed", 1.) * 0.5,
        burn: n(t, "2.ep_damage_ratio", 0.),
        fragile: n(t, "2.damage_scale", 1.) - 1.,
        scale: n(&sk["bb"], "atk_scale", 0.),
        flame_radius: n(&sk["bb"], "range_radius", 0.),
    });
}
impl Yanyou {
    fn targets(&self, k: &mut C<'_>, ranged: bool) -> Result<Vec<Id>, String> {
        let keys = k
            .world
            .unit(k.unit)?
            .operator
            .tactical
            .yanyou()?
            .keys
            .clone();
        let mut out = Vec::new();
        for id in k.host.spawns.as_ref().unwrap().live().to_vec() {
            if targeting::can_target_enemy(
                k.world,
                k.host,
                Some(k.unit),
                id,
                targeting::Profile {
                    can_hit_fly: true,
                    ..Default::default()
                },
            )? && (!ranged || body::in_range(k.world.unit(id)?, &keys))
            {
                let taunt = k.stat(id, "taunt")?;
                let distance = k.host.remaining_distance(id, k.world)?;
                let seq = k.world.unit(id)?.targeting.spawn_seq;
                out.push((id, taunt, distance, seq));
            }
        }
        out.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then(a.2.total_cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        Ok(out.into_iter().map(|x| x.0).collect())
    }
    fn lock(&self, k: &mut C<'_>) -> Result<Option<Id>, String> {
        let t = k.world.unit(k.unit)?.operator.tactical.yanyou()?.target;
        if let Some(t) = t {
            if k.world.unit(t).is_ok()
                && targeting::can_target_enemy(
                    k.world,
                    k.host,
                    Some(k.unit),
                    t,
                    targeting::Profile {
                        can_hit_fly: true,
                        ..Default::default()
                    },
                )?
            {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }
    fn keys(&self, k: &mut C<'_>) -> Result<(), String> {
        let pos = k.world.unit(k.unit)?.position;
        let s = k.world.unit_mut(k.unit)?.operator.tactical.yanyou_mut()?;
        if s.at
            .is_some_and(|a| (a[0] - pos[0]).abs() < 0.05 && (a[1] - pos[1]).abs() < 0.05)
        {
            return Ok(());
        }
        s.keys.clear();
        for r in 0..crate::battle::grid::ROWS {
            for c in 0..COLS {
                if hypot(r as f64 - pos[0], c as f64 - pos[1]) <= self.radius + 1e-9 {
                    s.keys.push(r * COLS + c);
                }
            }
        }
        s.at = Some(pos);
        Ok(())
    }
}
impl Behavior for Yanyou {
    fn profile(
        &self,
        w: &World,
        id: Id,
        skill: Option<&skills::Runtime>,
        p: &mut search::AttackProfile,
    ) -> Result<(), String> {
        p.no_attack = skill
            .map(|s| s.bank.active)
            .unwrap_or(w.unit(id)?.operator.active_skill);
        Ok(())
    }
    fn handle(&self, k: &mut C<'_>, e: &mut E<'_>) -> Result<(), String> {
        match e {
            E::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == k.unit => {
                k.world.unit_mut(k.unit)?.motion_fly = true;
                self.keys(k)?;
            }
            E::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == k.unit => {
                let target = self.targets(k, true)?.first().copied();
                let s = k.world.unit_mut(k.unit)?.operator.tactical.yanyou_mut()?;
                s.target = target;
                s.acc = 1.;
                if target.is_none() {
                    k.host
                        .with_kit_buffs(k.world, k.buffs, |h, w| skill.end(w, h, "noTarget"))?;
                }
            }
            E::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == k.unit => {
                k.world
                    .unit_mut(k.unit)?
                    .operator
                    .tactical
                    .yanyou_mut()?
                    .target = None;
            }
            E::Skill {
                unit,
                event: skills::Event::Tick,
                skill,
                context,
            } if *unit == k.unit => {
                let target = self.lock(k)?;
                let silent = k
                    .world
                    .unit_mut(k.unit)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .flags
                    .contains("silence");
                if silent || target.is_none() {
                    k.host.with_kit_buffs(k.world, k.buffs, |h, w| {
                        skill.end(w, h, if silent { "silence" } else { "targetLost" })
                    })?;
                    return Ok(());
                }
                if !skills::can_act(k.world, k.unit)? {
                    return Ok(());
                }
                let s = k.world.unit_mut(k.unit)?.operator.tactical.yanyou_mut()?;
                s.acc += context.dt;
                if s.acc < 1. - 1e-9 {
                    return Ok(());
                }
                s.acc -= 1.;
                let t = target.unwrap();
                let mut hit = if self.flame_radius > 0. {
                    k.radius(k.world.unit(t)?.position, self.flame_radius)?
                } else {
                    vec![]
                };
                if !hit.contains(&t) {
                    hit.insert(0, t);
                }
                let damage = k.stat(k.unit, "atk")? * self.scale;
                for enemy in hit {
                    k.damage(enemy, damage, DamageType::Arts, true, "flame")?;
                }
            }
            E::DamageApplied { event, request }
                if event.source == Some(k.unit)
                    && event.amount > 0.
                    && request.hit.damage_type != DamageType::Elemental
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && k.world.unit(event.target)?.stats.hp > 0. =>
            {
                let amount = k.stat(k.unit, "atk")? * self.burn;
                if amount > 0. {
                    k.elemental(event.target, amount, crate::battle::element::Element::Burn)?;
                }
            }
            E::Element { request, .. } if request.damage.hit.target == k.unit => {
                request.damage.cancelled = true;
            }
            E::Tick if k.up() => {
                if skills::can_act(k.world, k.unit)?
                    && !k
                        .world
                        .unit_mut(k.unit)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("noMove")
                    && self.speed > 0.
                {
                    let target = match self.lock(k)? {
                        Some(t) => Some(t),
                        None => self.targets(k, false)?.first().copied(),
                    };
                    if let Some(t) = target {
                        let from = k.world.unit(k.unit)?.position;
                        let to = k.world.unit(t)?.position;
                        let hover = k.world.unit(k.unit)?.operator.tactical.yanyou()?.hover;
                        let dr = to[0] + hover - from[0];
                        let dc = to[1] - from[1];
                        let d = hypot(dc, dr);
                        if d > 0.25 + 1e-6 {
                            let step = (d - 0.25).min(self.speed * DT);
                            let rect = k.host.rect;
                            k.world.unit_mut(k.unit)?.position = [
                                (from[0] + dr / d * step).clamp(rect.r0 as f64, rect.r1 as f64),
                                (from[1] + dc / d * step).clamp(rect.c0 as f64, rect.c1 as f64),
                            ];
                        }
                    }
                }
                self.keys(k)?;
                let s = k.world.unit_mut(k.unit)?.operator.tactical.yanyou_mut()?;
                s.aura += DT;
                if self.fragile > 0. && s.aura >= 0.2 - 1e-9 {
                    s.aura = 0.;
                    for t in k.radius(k.world.unit(k.unit)?.position, 1.5)? {
                        k.status(t, "elemFragile", 0.4, Some(self.fragile))?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
