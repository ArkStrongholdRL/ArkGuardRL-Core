// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "executor",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_texas2_3",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_texas2_1",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_texas2_2",
            kind: Kind::Duration,
            sp_type: SpType::None,
            rule: "DEFAULT",
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

    s.extra.deploy = true;
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.) + n(t, "atk", 0.))].into();
    if mode == Mode::Second {
        s.combat.skill_hits = 2;
        s.extra.skill_arts = true;
    }
    s.extra.tactics = Tactics::Texas2(Texas2 {
        mode,
        swordplay: [
            ("aspd".into(), n(t1, "attack_speed", 8.)),
            ("dmgTakenMul".into(), 1. - n(t1, "damage_resistance", 0.25)),
        ]
        .into(),
        grid: tactical_grid(&raw["skill"]["rangeGrid"], true)?,
        burst: n(bb, "appear.atk_scale", n(bb, "atk_scale", 1.15)),
        burst_stun: n(bb, "appear.stun", 1.5),
        scale: n(bb, "atk_scale", 0.85),
        interval: n(bb, "texas2_s_3[sword].interval", 1.),
        stun: n(bb, "stun", 0.2),
        targets: n(bb, "max_target", 2.).max(1.) as usize,
        res: n(bb, "magic_resistance", 0.),
        res_duration: n(bb, "debuff_duration", 8.),
        silence: n(bb, "attack@silence", 5.),
        dot: n(bb, "attack@texas2_s_1[dot].dot_damage", 260.),
        dot_duration: n(bb, "attack@texas2_s_1[dot].duration", 5.),
        dot_interval: n(bb, "attack@texas2_s_1[dot].interval", 1.),
        heal: n(t, "hp_ratio", 1.),
        lonely: n(tb, "atk", 0.),
    });
    Ok(())
}

pub(crate) struct Texas2 {
    pub mode: Mode,
    pub swordplay: Numbers,
    pub grid: Vec<[i32; 2]>,
    pub burst: f64,
    pub burst_stun: f64,
    pub scale: f64,
    pub interval: f64,
    pub stun: f64,
    pub targets: usize,
    pub res: f64,
    pub res_duration: f64,
    pub silence: f64,
    pub dot: f64,
    pub dot_duration: f64,
    pub dot_interval: f64,
    pub heal: f64,
    pub lonely: f64,
}

pub(crate) struct State {
    pub ready: f64,
    pub first_kill: bool,
    pub recast: bool,
    pub starting: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ready: f64::NEG_INFINITY,
            first_kill: Default::default(),
            recast: Default::default(),
            starting: Default::default(),
        }
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            first_kill: previous.is_some_and(|s| s.first_kill),
            recast: previous.is_some_and(|s| s.recast),
            starting: previous.is_some_and(|s| s.starting),
            ..Default::default()
        }
    }
}
impl Host {
    pub(in crate::battle::runtime) fn tac_texas_cast(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Texas2,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.texas2_mut()?.starting = true;
        if p.mode != Mode::First {
            let keys = self.tac_keys(w, id, &p.grid)?;
            for a in self.tac_enemies(w, id, Some(&keys), true)? {
                for _ in 0..if p.mode == Mode::Third { 2 } else { 1 } {
                    if !w.unit(a)?.stats.alive {
                        break;
                    }
                    let amount = Self::tac_stat(w, id, "atk")? * p.burst;
                    self.tac_damage(w, r, id, a, amount, DamageType::Arts, false, "burst")?;
                }
                if w.unit(a)?.stats.alive {
                    if p.mode == Mode::Third {
                        self.advanced_status(w, r, id, a, "stun", p.burst_stun, None)?;
                    } else if p.res != 0. {
                        self.component_debuff(
                            w,
                            r,
                            id,
                            a,
                            Debuff {
                                key: "texas2:resDown",
                                stat: "resMul",
                                value: p.res,
                                duration: p.res_duration,
                                strongest: true,
                            },
                        )?;
                    }
                }
            }
        }
        w.unit_mut(id)?.operator.tactical.texas2_mut()?.starting = false;
        // The rain's first deadline is one tick early to match the skill countdown.
        w.unit_mut(id)?.operator.tactical.texas2_mut()?.ready = w.time + p.interval - DT;
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_finish(
        &mut self,
        w: &mut World,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        let id = s.unit();
        if w.unit(id)?.operator.tactical.texas2()?.recast {
            w.unit_mut(id)?.operator.tactical.texas2_mut()?.recast = false;
            s.end(w, self, "recast")?;
            s.activate(w, self, "kill", true)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Texas2,
    ) -> Result<(), String> {
        self.tac_texas_cast(w, r, id, p)?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        due: [bool; 5],
        p: &Texas2,
    ) -> Result<(), String> {
        if due[1] && p.lonely != 0. {
            let tile = w.unit(id)?.deployment.tile;
            let lonely = self.roster.as_ref().unwrap().ids().iter().all(|&a| {
                a == id
                    || !Self::tac_up(w, a)
                    || (w.unit(a).unwrap().deployment.tile[0] - tile[0]).abs()
                        + (w.unit(a).unwrap().deployment.tile[1] - tile[1]).abs()
                        != 1
            });
            self.advanced_toggle(
                w,
                r,
                id,
                "texas2:module",
                lonely,
                [("atkPct".into(), p.lonely)].into(),
            )?;
        }
        if p.mode == Mode::Third
            && active
            && w.time + 1e-9 >= w.unit(id)?.operator.tactical.texas2()?.ready
        {
            w.unit_mut(id)?.operator.tactical.texas2_mut()?.ready += p.interval;
            let keys = self.tac_keys(w, id, &p.grid)?;
            for a in self
                .tac_enemies(w, id, Some(&keys), true)?
                .into_iter()
                .take(p.targets)
            {
                let amount = Self::tac_stat(w, id, "atk")? * p.scale;
                self.tac_damage(w, r, id, a, amount, DamageType::Arts, false, "swordRain")?;
                if Self::tac_up(w, a) {
                    self.advanced_status(w, r, id, a, "stun", p.stun, None)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_landed(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        q: &hit::Request,
        e: &health::Event,
        p: &Texas2,
    ) -> Result<(), String> {
        if p.mode == Mode::First
            && q.is_attack
            && Self::tac_up(w, e.target)
            && w.unit(id)?.operator.active_skill
        {
            self.advanced_status(w, r, id, e.target, "silence", p.silence, None)?;
            let mut b = Buff {
                key: Some(format!("texas2:drizzleDot:{id}")),
                ..Default::default()
            };
            b.runtime.source = Some(id);
            b.runtime.duration = p.dot_duration + 1e-6;
            b.runtime.interval = p.dot_interval.max(0.1);
            b.runtime.tick_payload = p.dot;
            b.runtime.refresh = buffs::Refresh::Extend;
            b.runtime.on_tick = Some(DRIZZLE);
            r.add(
                w,
                self,
                e.target,
                buffs::Spec {
                    buff: b,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_kill(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        source: Id,
        p: &Texas2,
    ) -> Result<(), String> {
        if Self::tac_up(w, source) && !w.unit(source)?.operator.tactical.texas2()?.first_kill {
            w.unit_mut(source)?
                .operator
                .tactical
                .texas2_mut()?
                .first_kill = true;
            w.unit_mut(source)?.operator.tactical.texas2_mut()?.recast = true;
            r.remove(w, self, source, "texas2:swordplay")?;
            let amount = Self::tac_stat(w, source, "maxHp")? * p.heal;
            self.tac_heal(w, r, source, source, amount)?;
            if w.unit(source)?.operator.tactical.texas2()?.starting {
                self.tac_texas_cast(w, r, source, p)?;
            } else {
                self.texas2_flush_recast(w, r, source)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_deployed(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Texas2,
    ) -> Result<(), String> {
        if Self::tac_up(w, id) && !w.unit(id)?.operator.tactical.texas2()?.first_kill {
            r.add(
                w,
                self,
                id,
                buffs::Spec {
                    buff: Buff {
                        key: Some("texas2:swordplay".into()),
                        mods: p.swordplay.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_flush_recast(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
    ) -> Result<(), String> {
        if !w.unit(id)?.operator.tactical.texas2()?.recast {
            return Ok(());
        }
        if let Ok(mut skill) = self.roster.as_mut().unwrap().take_skill(id) {
            w.unit_mut(id)?.operator.tactical.texas2_mut()?.recast = false;
            let result = self.with_kit_buffs(w, r, |host, world| {
                skill.end(world, host, "recast")?;
                skill.activate(world, host, "kill", true)?;
                Ok(())
            });
            let restore = self.roster.as_mut().unwrap().restore_skill(id, skill);
            result.and(restore)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn texas2_dot_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?.clone();
        let id = b.runtime.source.ok_or("Missing texas2 buff source")?;

        self.tac_damage(
            w,
            r,
            id,
            c.unit,
            b.runtime.tick_payload,
            DamageType::Arts,
            false,
            "dot",
        )?;
        Ok(())
    }
}

impl Behavior for Texas2 {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_) | KitEvent::Death { .. })
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
                KitEvent::Buff {
                    context,
                    event: buffs::Event::Tick,
                    token: DRIZZLE,
                },
            ) => {
                k.host.texas2_dot_tick(k.world, k.buffs, *context)?;
            }

            (Role::Source, KitEvent::Landed { event, request }) => {
                k.host
                    .texas2_landed(k.world, id, k.buffs, request, event, self)?;
            }
            (Role::Source, KitEvent::Kill { .. }) => {
                k.host.texas2_kill(k.world, k.buffs, id, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Death { unit, .. } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.texas2_mut()?;
                state.first_kill = false;
                state.recast = false;
                state.starting = false;
            }
            KitEvent::Scheduled(due) => {
                if k.up() {
                    let active = k.active()?;
                    k.host
                        .texas2_scheduled(k.world, id, k.buffs, active, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.texas2_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::SkillStart,
                context,
                ..
            } if *unit == id => {
                if context.reason == "deploy" {
                    k.host.texas2_deployed(k.world, k.buffs, id, self)?;
                }
            }
            KitEvent::Resume { unit, skill } if *unit == id => {
                k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                    host.texas2_finish(world, skill)
                })?;
            }
            KitEvent::Restored(skills::Event::SkillStart) | KitEvent::Flush => {
                k.host.texas2_flush_recast(k.world, k.buffs, id)?;
            }
            _ => {}
        }
        Ok(())
    }
}
