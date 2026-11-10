// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "shotprotector",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_rmixer_3",
            kind: Kind::Ammo,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_rmixer_1",
            kind: Kind::Instant,
            sp_type: SpType::Hurt,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_rmixer_2",
            kind: Kind::Ammo,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
    ],
    build,
};
fn build(raw: &Value, _default: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let mode = tactical_mode(raw)?;

    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "atk_scale", 1.3);
        s.combat.skill_hits = 3;
    } else {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("defPct".into(), n(bb, "def", 0.)),
            ("hpPct".into(), n(bb, "max_hp", 0.)),
        ]
        .into();
        s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 30.);
    }
    if mode == Mode::Third {
        skill_range(raw, s)?;
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Rmixer(Rmixer {
        mode,
        targets: n(bb, "attack@max_target", 3.).max(1.) as usize,
        counter_ratio: n(bb, "base_attack_time", 0.6),
        reload: n(bb, "charge", 1.),
        team_reload: n(bb, "ammo", 0.),
        fatal_cost: n(bb, "ammo_cost", 30.),
        defence: n(t, "def", 0.),
        aspd: n(t, "attack_speed", 0.),
        stacks: n(t, "max_stack_cnt", 3.).max(1.) as u32,
        duration: n(t, "duration", 10.),
        idle: n(t1, "interval", 8.),
        shield: n(t1, "shield", 0.15),
        reveal: raw["module"]["active"] == true && raw["module"]["id"] == "uniequip_002_rmixer",
    });
    Ok(())
}

pub(crate) struct Rmixer {
    pub mode: Mode,
    pub targets: usize,
    pub counter_ratio: f64,
    pub reload: f64,
    pub team_reload: f64,
    pub fatal_cost: f64,
    pub defence: f64,
    pub aspd: f64,
    pub stacks: u32,
    pub duration: f64,
    pub idle: f64,
    pub shield: f64,
    pub reveal: bool,
}

pub(crate) struct State {
    pub ready: f64,
    pub boost: bool,
    pub last_attack: f64,
    pub shield_lost: f64,
    pub pre_hp: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            ready: f64::NEG_INFINITY,
            boost: Default::default(),
            last_attack: f64::NEG_INFINITY,
            shield_lost: f64::NEG_INFINITY,
            pre_hp: Default::default(),
        }
    }
}

impl Host {
    pub(in crate::battle::runtime) fn tac_laterano(
        &self,
        w: &World,
        id: Id,
        near: bool,
    ) -> Result<Vec<Id>, String> {
        let mut out = vec![];
        for &a in self.roster.as_ref().unwrap().ids() {
            if a != id
                && Self::tac_up(w, a)
                && self.tac_def(a)?.laterano
                && (!near || body::in_radius(w.unit(a)?, w.unit(id)?.position, 1.5))
            {
                out.push(a);
            }
        }
        out.sort_by_key(|a| std::cmp::Reverse(w.unit(*a).unwrap().deployment.deploy_seq));
        Ok(out)
    }
    pub(in crate::battle::runtime) fn tac_reload(
        &mut self,
        w: &mut World,
        id: Id,
        amount: f64,
    ) -> Result<bool, String> {
        if self.roster.as_ref().unwrap().entry(id)?.skill().is_err() {
            return Ok(false);
        }
        let mut s = self.roster.as_mut().unwrap().take_skill(id)?;
        let active = s.bank.active && s.bank.kind == crate::battle::sp::Kind::Ammo;
        if active {
            s.ammo_left = (s.ammo_left + amount).min(s.ammo_max);
        }
        self.roster.as_mut().unwrap().restore_skill(id, s)?;
        let _ = w;
        Ok(active)
    }
    pub(in crate::battle::runtime) fn rmixer_fatal(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &mut health::Event,
        p: &Rmixer,
    ) -> Result<(), String> {
        if e.stage != health::Stage::Fatal
            || e.prevented
            || self.holds_undying(w, e.target)?
            || w.unit(e.target)?.side != health::Side::Ally
        {
            return Ok(());
        }
        let id = e.target;
        if p.mode != Mode::Second || !w.unit(id)?.operator.active_skill {
            return Ok(());
        }
        self.with_kit_buffs(w, r, |h, w| {
            let mut s = h.roster.as_mut().unwrap().take_skill(id)?;
            let result = (|| {
                if s.ammo_left > 0. {
                    e.prevented = true;
                    w.unit_mut(id)?.stats.hp = w
                        .unit(id)?
                        .operator
                        .tactical
                        .rmixer()?
                        .pre_hp
                        .max(1.)
                        .min(Self::tac_stat(w, id, "maxHp")?);
                    s.ammo_left = (s.ammo_left - p.fatal_cost).max(0.);
                    if s.ammo_left <= 0. {
                        s.end(w, h, "ammo")?;
                    }
                }
                Ok(())
            })();
            let restore = h.roster.as_mut().unwrap().restore_skill(id, s);
            result.and(restore)
        })
    }
    pub(in crate::battle::runtime) fn rmixer_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Rmixer,
    ) -> Result<(), String> {
        if p.mode == Mode::Third {
            w.unit_mut(id)?.operator.tactical.rmixer_mut()?.ready = f64::NEG_INFINITY;
            let mut b = Buff {
                key: Some("rmixer:counter".into()),
                ..Default::default()
            };
            b.runtime.injury.observe = Some(OBSERVE);
            r.add(
                w,
                self,
                id,
                buffs::Spec {
                    buff: b,
                    ..Default::default()
                },
            )?;
            for a in self.tac_laterano(w, id, false)? {
                self.tac_reload(w, a, p.team_reload)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn rmixer_end(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
    ) -> Result<(), String> {
        r.remove(w, self, id, "rmixer:counter")?;
        Ok(())
    }
    pub(in crate::battle::runtime) fn rmixer_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Rmixer,
    ) -> Result<(), String> {
        if due[0] && !Self::tac_has(w, id, "rmixer:shield") {
            let since = w
                .unit(id)?
                .deployment
                .deployed_at
                .max(w.unit(id)?.operator.tactical.rmixer()?.last_attack)
                .max(w.unit(id)?.operator.tactical.rmixer()?.shield_lost);
            if w.time - since >= p.idle - 1e-9 {
                let amount = Self::tac_stat(w, id, "maxHp")? * p.shield;
                let mut b = Buff {
                    key: Some("rmixer:shield".into()),
                    shield: amount,
                    ..Default::default()
                };
                b.runtime.on_remove = Some(SHIELD_LOST);
                r.add(
                    w,
                    self,
                    id,
                    buffs::Spec {
                        buff: b,
                        ..Default::default()
                    },
                )?;
            }
        }
        if due[1] && p.reveal {
            for a in self.advanced_enemies(w, id)? {
                if Self::tac_has(w, a, "stealth") {
                    let mut b = Buff {
                        key: Some("aura:reveal".into()),
                        flags: ["reveal".into()].into(),
                        ..Default::default()
                    };
                    b.runtime.duration = 0.25;
                    r.add(
                        w,
                        self,
                        a,
                        buffs::Spec {
                            buff: b,
                            ..Default::default()
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn rmixer_attack(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        if !w.unit(id)?.operator.tactical.rmixer()?.boost {
            w.unit_mut(id)?.operator.tactical.rmixer_mut()?.last_attack = w.time;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn rmixer_impact(
        &mut self,
        w: &mut World,
        id: Id,
        impact: attacks::Impact,
        p: &Rmixer,
    ) -> Result<(), String> {
        if impact.is_skill && p.mode == Mode::First {
            for a in self.tac_laterano(w, id, true)? {
                if self.tac_reload(w, a, p.reload)? {
                    break;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn rmixer_observe(
        c: &mut settlement::Context<'_, Self>,
        target: Id,
    ) -> Result<(), String> {
        let def = c.host.tac_def(target)?;
        let Tactics::Rmixer(p) = &def.extra.tactics else {
            return Err("Invalid rmixer observer".into());
        };
        if !c.world.unit(target)?.operator.active_skill
            || !skills::can_act(c.world, target)?
            || Self::tac_has(c.world, target, "disarm")
            || c.world.time < c.world.unit(target)?.operator.tactical.rmixer()?.ready - 1e-9
        {
            return Ok(());
        }
        let mut ids = c.host.tac_enemies(c.world, target, None, true)?;
        if let Some(source) = c.request.hit.source {
            if let Some(i) = ids.iter().position(|&a| a == source) {
                ids.remove(i);
                ids.insert(0, source);
            }
        }
        ids.truncate(p.targets);
        c.world
            .unit_mut(target)?
            .operator
            .tactical
            .rmixer_mut()?
            .ready = c.world.time + Self::tac_stat(c.world, target, "interval")? * p.counter_ratio;
        c.world
            .unit_mut(target)?
            .operator
            .tactical
            .rmixer_mut()?
            .boost = true;
        let result = c.host.with_kit_buffs(c.world, c.runtime, |h, w| {
            let mut s = h.roster.as_mut().unwrap().take_skill(target)?;
            let nested = h.executor.is_none();
            if nested {
                h.executor = Some(attacks::Executor::new(
                    w.unit_capacity(),
                    crate::battle::operators::Combat::MAX_HITS,
                )?);
            }
            let result = if ids.is_empty() {
                w.unit_mut(target)?.attacks += 1;
                s.after_attack(w, h, &[], true, false)
            } else {
                ally::Host::perform(
                    h,
                    target,
                    def.combat.profile,
                    search::Selection {
                        targets: ids,
                        fortress_melee: None,
                    },
                    Some(&mut s),
                    w,
                )
            };
            if nested {
                h.executor = None;
            }
            let restore = h.roster.as_mut().unwrap().restore_skill(target, s);
            result.and(restore)
        });
        c.world
            .unit_mut(target)?
            .operator
            .tactical
            .rmixer_mut()?
            .boost = false;
        result?;
        Ok(())
    }

    pub(in crate::battle::runtime) fn rmixer_shield_lost(
        &mut self,
        w: &mut World,
        id: Id,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.rmixer_mut()?.shield_lost = w.time;
        Ok(())
    }
}

impl Behavior for Rmixer {
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
            (Role::Source, KitEvent::DamageApplied { event, .. }) => {
                if k.up()
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && event.amount > 0.
                {
                    let mut buff = Buff {
                        key: Some("rmixer:t1".into()),
                        mods: [("defFlat".into(), self.defence), ("aspd".into(), self.aspd)].into(),
                        ..Default::default()
                    };
                    buff.runtime.refresh = buffs::Refresh::Stack;
                    buff.runtime.max_stacks = self.stacks;
                    buff.runtime.duration = self.duration;
                    k.buffs.add(
                        k.world,
                        k.host,
                        id,
                        buffs::Spec {
                            buff,
                            ..Default::default()
                        },
                    )?;
                }
            }

            (
                Role::Source,
                KitEvent::Buff {
                    context,
                    token: SHIELD_LOST,
                    ..
                },
            ) => {
                k.host.rmixer_shield_lost(k.world, context.unit)?;
            }

            (Role::Source, KitEvent::Attack { .. }) => {
                k.host.rmixer_attack(k.world, id)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                k.host.rmixer_impact(k.world, id, *impact, self)?;
            }
            (
                Role::Target,
                KitEvent::Hit {
                    phase: hit::Phase::Hit,
                    ..
                },
            ) => {
                k.world.unit_mut(id)?.operator.tactical.rmixer_mut()?.pre_hp =
                    k.world.unit(id)?.stats.hp;
            }
            (Role::Target, KitEvent::Fatal(event)) => {
                k.host.rmixer_fatal(k.world, k.buffs, event, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn observe_injury(
        &self,
        c: &mut settlement::Context<'_, Host>,
        key: &str,
        _: &Buff,
    ) -> Result<(), String> {
        if key == "rmixer:counter" {
            Host::rmixer_observe(c, c.request.hit.target)?;
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                if k.up() {
                    k.host.rmixer_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.rmixer_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                k.host.rmixer_end(k.world, id, k.buffs)?;
            }
            _ => {}
        }
        Ok(())
    }
}
