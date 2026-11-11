// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "chain",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_pasngr_3",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "SKILL_RANGE",
            attack_override: false,
        },
        Skill {
            id: "skchr_pasngr_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_pasngr_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let mode = tactical_mode(raw)?;

    let chain = Chain {
        count: n(tb, "attack@max_target", 4.),
        radius: 1.7,
        falloff: 1. - n(tb, "attack@chain.atk_scale", 0.85),
        sluggish: n(tb, "attack@sluggish", 0.5),
    };
    s.combat.area.chain = Some(chain);
    s.combat.skill_area = s.combat.area;
    skill_range(raw, s)?;
    if raw["skill"]["trigger"]["customRangeGrid"].is_array() {
        s.trigger_range = Some(crate::battle::operators::parse_range(
            &raw["skill"]["trigger"]["customRangeGrid"],
        )?);
    }
    if mode == Mode::First {
        s.combat.skill_scale = suffix_number(bb, "atk_scale", 1.);
        s.combat.skill_area.chain = Some(Chain {
            count: suffix_number(bb, "max_target", 4.),
            sluggish: suffix_number(bb, "sluggish", 0.5),
            falloff: 1. - n(tb, "skill@pasngr_s_1.chain.atk_scale", 1. - chain.falloff),
            ..chain
        });
    }
    if mode == Mode::Second {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("batPct".into(), n(bb, "base_attack_time", 0.)),
        ]
        .into();
        s.extra.advanced_range_extend = n(bb, "ability_range_forward_extend", 0.).max(0.) as u32;
        s.combat.skill_area.chain = Some(Chain {
            count: n(bb, "attack@max_target", 5.),
            ..chain
        });
    }
    s.extra.tactics = Tactics::Pasngr(Pasngr {
        mode,
        chain: Chain {
            count: n(bb, "chain.max_target", 4.),
            falloff: 1. - n(tb, "skill@chain.atk_scale", 1. - chain.falloff),
            sluggish: n(tb, "skill@sluggish", n(bb, "sluggish", chain.sluggish)),
            ..chain
        },
        scale: n(bb, "atk_scale", 1.),
        duration: n(bb, "duration", 4.),
        interval: n(bb, "interval", 0.5).max(0.1),
        threshold: n(t, "hp_ratio", 0.8),
        enhance: suffix_number(t, "damage_scale", 1.),
        enhance_duration: suffix_number(t, "duration", 3.),
        attack: n(t1, "atk", 0.),
        sp: n(t1, "sp_recovery_per_sec", 0.),
    });
    Ok(())
}

pub(crate) struct Pasngr {
    pub mode: Mode,
    pub chain: Chain,
    pub scale: f64,
    pub duration: f64,
    pub interval: f64,
    pub threshold: f64,
    pub enhance: f64,
    pub enhance_duration: f64,
    pub attack: f64,
    pub sp: f64,
}

impl Host {
    pub(in crate::battle::runtime) fn pasngr_start(
        &mut self,
        w: &mut World,
        id: Id,
        def: &Definition,
        p: &Pasngr,
    ) -> Result<(), String> {
        if p.mode == Mode::Third {
            let keys = self.support_trigger_range(w, id, def)?;
            let mut ids = search::Host::enemies(
                self,
                id,
                &keys,
                targeting::Profile {
                    can_hit_fly: true,
                    ..Default::default()
                },
                w,
                w.unit_capacity(),
            )?;
            ids.sort_by(|a, b| {
                w.unit(*b)
                    .unwrap()
                    .stats
                    .hp
                    .total_cmp(&w.unit(*a).unwrap().stats.hp)
            });
            if let Some(&target) = ids.first() {
                let pos = w.unit(target)?.position;
                let position = [pos[0].round() as i32, pos[1].round() as i32];
                self.tac_timer(
                    w,
                    id,
                    w.time + p.interval,
                    p.interval,
                    (p.duration / p.interval).round().max(1.) as u32,
                    Task::Storm {
                        position,
                        scale: p.scale,
                        chain: p.chain,
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pasngr_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Pasngr,
    ) -> Result<(), String> {
        if due[2] {
            let keys = self.tac_keys(w, id, &[[-1, 0], [1, 0], [0, -1], [0, 1]])?;
            if !self.spawns.as_ref().unwrap().live().iter().any(|&a| {
                Self::tac_up(w, a)
                    && !w.unit(a).unwrap().hidden
                    && body::in_range(w.unit(a).unwrap(), &keys)
            }) {
                self.component_buff(
                    w,
                    r,
                    id,
                    "pasngr:lone",
                    [("atkPct".into(), p.attack), ("spRecoveryFlat".into(), p.sp)].into(),
                    0.4,
                    id,
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pasngr_phase(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        q: &mut hit::Request,
        c: &Pasngr,
    ) -> Result<(), String> {
        if w.unit(q.hit.target)?.side == health::Side::Enemy
            && q.hit.damage_type != DamageType::Elemental
        {
            let key = format!("pasngr:enhance:{id}");
            if q.is_attack
                && w.unit(q.hit.target)?.stats.hp / Self::tac_stat(w, q.hit.target, "maxHp")?
                    >= c.threshold - 1e-9
            {
                self.component_buff(
                    w,
                    r,
                    q.hit.target,
                    &key,
                    Numbers::new(),
                    c.enhance_duration,
                    id,
                )?;
            }
            if Self::tac_has(w, q.hit.target, &key) {
                q.damage.mul *= c.enhance;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pasngr_storm_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let id = t.source.ok_or("Operator timer requires a source")?;
        let Task::Storm {
            position,
            scale,
            chain,
        } = &mut t.kind
        else {
            return Err("Invalid storm timer".into());
        };

        if !chain.count.is_finite()
            || chain.count.ceil() > w.unit_capacity() as f64
            || !chain.radius.is_finite()
            || !chain.falloff.is_finite()
        {
            return Err("Storm chain capacity exceeded".into());
        }
        let grid: Vec<_> = (-2_i32..=2)
            .rev()
            .flat_map(|r| {
                (-2_i32..=2).filter_map(move |c| {
                    if r.abs() + c.abs() <= 2 {
                        Some([r, c])
                    } else {
                        None
                    }
                })
            })
            .collect();
        let keys = targeting::absolute_range(&grid, *position, Direction::Right, 0);
        // Random selection uses spatial query order, before attack-priority sorting.
        let targets = search::Host::enemies(
            self,
            id,
            &keys,
            targeting::Profile {
                can_hit_fly: true,
                ..Default::default()
            },
            w,
            w.unit_capacity(),
        )?;
        if let Some(&first) = self.rng.pick(&targets) {
            let mut seen = vec![];
            let mut current = Some(first);
            for k in 0..chain.count.ceil() as u32 {
                let Some(a) = current else {
                    break;
                };
                seen.push(a);
                let pos = w.unit(a)?.position;
                let amount = Self::tac_stat(w, id, "atk")?
                    * *scale
                    * powi(1. - chain.falloff, k as i32).map_err(str::to_string)?;
                self.tac_damage(w, r, id, a, amount, DamageType::Arts, true, "storm")?;
                if Self::tac_up(w, a) && chain.sluggish > 0. {
                    self.advanced_status(w, r, id, a, "sluggish", chain.sluggish, None)?;
                }
                let candidates = self.tac_radius(w, pos, chain.radius)?;
                let mut next = self.tac_selectable(w, id, candidates)?;
                next.retain(|a| !seen.contains(a));
                next.sort_by(|a, b| {
                    body::distance(w.unit(*a).unwrap(), pos)
                        .total_cmp(&body::distance(w.unit(*b).unwrap(), pos))
                });
                current = next.first().copied();
            }
        }

        Ok(true)
    }
}

impl Behavior for Pasngr {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_))
    }
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        let id = k.unit;
        match (role, event) {
            (
                Role::Source,
                KitEvent::Hit {
                    phase: hit::Phase::Hit,
                    request,
                },
            ) => {
                k.host.pasngr_phase(k.world, id, k.buffs, request, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                if k.up() {
                    k.host.pasngr_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let definition = k.host.tac_def(id)?;
                k.host.pasngr_start(k.world, id, &definition, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
