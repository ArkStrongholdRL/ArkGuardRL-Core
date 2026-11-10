// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SPECIAL",
    branch: "alchemist",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_tinman_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_tinman_1",
            kind: Kind::Instant,
            sp_type: SpType::Attack,
            rule: "DEFAULT",
            attack_override: false,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    let mode = tactical_mode(raw)?;

    s.extra.tactics = Tactics::Tinman(Tinman {
        mode,
        duration: n(bb, "projectile_delay_time", 10.),
        radius: n(bb, "projectile_range", 1.5),
        scale: n(bb, "atk_scale", 0.),
        regen: n(bb, "hp_recovery_per_sec_ratio", 0.),
        weaken: (-n(bb, "atk", 0.)).clamp(0., 1.),
        wither: n(t1, "skill@damage_scale", 1.),
        sp: n(tm, "sp_recovery_per_sec", 0.),
    });
    Ok(())
}

pub(crate) struct Tinman {
    pub mode: Mode,
    pub duration: f64,
    pub radius: f64,
    pub scale: f64,
    pub regen: f64,
    pub weaken: f64,
    pub wither: f64,
    pub sp: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub sequence: u64,
}

impl Host {
    pub(in crate::battle::runtime) fn tactical_sync_zones(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        let mut timers = std::mem::take(&mut self.tactical_timers);
        let result: Result<(), String> = (|| {
            for t in &mut timers {
                let Task::Zone {
                    position,
                    end,
                    radius,
                    regen,
                    wither,
                    key,
                    held,
                    ..
                } = &mut t.kind
                else {
                    continue;
                };
                let mut covered = std::collections::BTreeSet::new();
                if w.time + 1e-9 < *end {
                    if *wither > 1. {
                        for a in self.tac_radius(w, *position, *radius)? {
                            if targeting::flying(w, a)? {
                                continue;
                            }
                            covered.insert(a);
                            if !held.get(&a).is_some_and(|b| r.get(w, a, *b).is_ok()) {
                                let mut b = Buff {
                                    key: Some("tinman:wither".into()),
                                    ..Default::default()
                                };
                                b.runtime.source = t.source;
                                b.runtime.tick_payload = *wither;
                                b.runtime.refresh = buffs::Refresh::Independent;
                                b.runtime.max_stacks = i32::MAX as u32;
                                b.runtime.injury.on_injury = Some(WITHER);
                                let bid = r.add(
                                    w,
                                    self,
                                    a,
                                    buffs::Spec {
                                        buff: b,
                                        ..Default::default()
                                    },
                                )?;
                                if let Some(bid) = bid {
                                    held.insert(a, bid);
                                }
                            }
                        }
                    }
                    if *regen > 0. {
                        for a in self.roster.as_ref().unwrap().ids().to_vec() {
                            if Self::tac_up(w, a)
                                && body::in_radius(w.unit(a)?, *position, *radius)
                                && targeting::aura_selectable(w, a)?
                            {
                                covered.insert(a);
                                if !held.get(&a).is_some_and(|b| r.get(w, a, *b).is_ok()) {
                                    let mut b = Buff {
                                        key: Some(key.clone()),
                                        mods: [("hpRegen".into(), *regen)].into(),
                                        ..Default::default()
                                    };
                                    b.runtime.source = t.source;
                                    let bid = r.add(
                                        w,
                                        self,
                                        a,
                                        buffs::Spec {
                                            buff: b,
                                            ..Default::default()
                                        },
                                    )?;
                                    if let Some(bid) = bid {
                                        held.insert(a, bid);
                                    }
                                }
                            }
                        }
                    }
                }
                let departed: Vec<_> = held
                    .keys()
                    .copied()
                    .filter(|a| !covered.contains(a))
                    .collect();
                for a in departed {
                    let b = held.remove(&a).unwrap();
                    if w.unit(a).is_ok() {
                        r.remove_id(w, self, a, b)?;
                    }
                }
            }
            Ok(())
        })();
        timers.append(&mut self.tactical_timers);
        self.tactical_timers = timers;
        result
    }
    pub(in crate::battle::runtime) fn tactical_wither(
        e: &mut injury::InjuryEvent,
        c: &mut settlement::Context<'_, Self>,
    ) -> Result<(), String> {
        if c.request.attack_type != hit::AttackType::Buff {
            return Ok(());
        }
        let strongest = c
            .world
            .unit(c.request.hit.target)?
            .stats
            .buffs()
            .iter()
            .filter(|b| b.key.as_deref() == Some("tinman:wither"))
            .max_by(|a, b| {
                a.runtime
                    .tick_payload
                    .total_cmp(&b.runtime.tick_payload)
                    .then(b.runtime.id().cmp(&a.runtime.id()))
            })
            .map(|b| b.runtime.id());
        if strongest == Some(c.buff) {
            e.amount *= c
                .runtime
                .get(c.world, c.request.hit.target, c.buff)?
                .runtime
                .tick_payload;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn tinman_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Tinman,
    ) -> Result<(), String> {
        let candidates = self.tac_enemies(w, id, None, true)?;
        let mut target = None;
        for &a in &candidates {
            if !targeting::flying(w, a)? {
                target = Some(a);
                break;
            }
        }
        let target = target.or_else(|| candidates.first().copied());
        let position = if let Some(a) = target {
            w.unit(a)?.position
        } else {
            let u = w.unit(id)?;
            let v = self.roster.as_ref().unwrap().entry(id)?.direction.vector();
            [
                u.position[0] + f64::from(v[0]),
                u.position[1] + f64::from(v[1]),
            ]
        };
        let attack = Self::tac_stat(w, id, "atk")?;
        w.unit_mut(id)?.operator.tactical.tinman_mut()?.sequence += 1;
        let sequence = w.unit(id)?.operator.tactical.tinman()?.sequence;
        let key = format!("tinman:zone:{id}:{sequence}");
        let interval = if p.mode == Mode::First { 0.25 } else { 1. };
        let at = w.time;
        self.tac_timer(
            w,
            id,
            at,
            interval,
            (p.duration / interval).round().max(1.) as u32 + 1,
            Task::Zone {
                position,
                end: at + p.duration,
                radius: p.radius,
                attack,
                regen: attack * p.regen,
                weaken: p.weaken,
                wither: p.wither,
                key,
                held: Default::default(),
                pulse: 0,
            },
        )?;
        self.tactical_sync_zones(w, r)?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn tinman_zone_timer(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        t: &mut Timer,
    ) -> Result<bool, String> {
        let id = t.source.ok_or("Operator timer requires a source")?;
        let Task::Zone {
            position,
            end,
            radius,
            attack,
            weaken,
            held,
            key,
            pulse,
            ..
        } = &mut t.kind
        else {
            return Err("Invalid zone timer".into());
        };

        if w.time + 1e-9 >= *end {
            for (a, b) in std::mem::take(held) {
                if w.unit(a).is_ok() {
                    r.remove_id(w, self, a, b)?;
                }
            }
            return Ok(false);
        }
        let p = self.tac_def(id)?;
        let Tactics::Tinman(p) = &p.extra.tactics else {
            return Err("Invalid alchemy timer".into());
        };
        for a in self.tac_radius(w, *position, *radius)? {
            if targeting::flying(w, a)? || Self::tac_has(w, a, "untargetable") {
                continue;
            }
            if *weaken > 0. {
                self.advanced_status(w, r, id, a, "weaken", 0.3, Some(*weaken))?;
            }
            if p.mode == Mode::Second || *pulse % 4 == 0 {
                self.tac_damage(
                    w,
                    r,
                    id,
                    a,
                    *attack * p.scale,
                    DamageType::Arts,
                    false,
                    "zone",
                )?;
            }
        }
        *pulse += 1;
        let _ = key;

        Ok(true)
    }
}

impl State {
    pub fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            sequence: previous.map_or(0, |p| p.sequence),
        }
    }
}

impl Behavior for Tinman {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle()
    }
    fn sp_gain(&self, host: &Host, unit: Id, context: &mut skills::Context) {
        if context.reason == "time"
            && host
                .tactical_timers
                .iter()
                .any(|timer| timer.source == Some(unit) && matches!(timer.kind, Task::Zone { .. }))
        {
            context.amount += self.sp * DT;
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.tinman_start(k.world, id, k.buffs, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
