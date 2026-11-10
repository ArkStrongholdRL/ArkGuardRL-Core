// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "TANK",
    branch: "guardian",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_demkni_2",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_demkni_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_demkni_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "TAKE_DAMAGE",
            attack_override: false,
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

    if mode == Mode::First {
        skill_healing(s, 1., n(bb, "heal_scale", 1.));
        let p = s.combat.skill_profile.as_mut().unwrap();
        p.heal.as_mut().unwrap().hp_at_most = 0.5;
        s.extra.skill_range = Some(tactical_grid(&raw["skill"]["rangeGrid"], true)?);
    }
    if mode == Mode::Second {
        s.rule_override = Some(crate::battle::skills::Rule::Never);
    }
    s.extra.support.heal_bonus = Some(HealBonus {
        below: n(tb, "hp_ratio", 0.),
        scale: n(tb, "heal_scale", 1.),
        inclusive: false,
    });
    if n(tb, "damage_resistance", 0.) > 0. {
        s.extra.effects.permanent.push(persistent(
            "saria:moduleY",
            [("dmgTakenMul".into(), 1. - n(tb, "damage_resistance", 0.))].into(),
        ));
    }
    s.extra.tactics = Tactics::Demkni(Demkni {
        mode,
        grid: tactical_grid(&raw["skill"]["rangeGrid"], false)?,
        heal: n(bb, "heal_scale", n(bb, "attack@heal_scale", 0.)),
        slow: n(bb, "demkni_s_3.move_speed", 0.),
        arts: n(bb, "demkni_s_3.damage_scale", 1.),
        interval: n(t, "interval", 20.),
        stacks: n(t, "max_stack_cnt", 5.).max(1.) as u32,
        attack: n(t, "atk", 0.),
        defence: n(t, "def", 0.),
        sp: n(t1, "sp", 0.),
    });
    Ok(())
}

pub(crate) struct Demkni {
    pub mode: Mode,
    pub grid: Vec<[i32; 2]>,
    pub heal: f64,
    pub slow: f64,
    pub arts: f64,
    pub interval: f64,
    pub stacks: u32,
    pub attack: f64,
    pub defence: f64,
    pub sp: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub aura: f64,
    pub accumulator: f64,
}

impl Host {
    pub(in crate::battle::runtime) fn demkni_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Demkni,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.demkni_mut()?.aura = 0.25;
        w.unit_mut(id)?.operator.tactical.demkni_mut()?.accumulator = 0.;
        if p.mode == Mode::Second {
            let keys = self.tac_keys(w, id, &p.grid)?;
            let amount = Self::tac_stat(w, id, "atk")? * p.heal;
            for a in self.tac_heal_targets_in_order(w, id, &keys, true)? {
                self.tac_heal(w, r, id, a, amount)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn demkni_skill_tick(
        &mut self,
        dt: f64,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Demkni,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.demkni_mut()?.aura += dt;
        w.unit_mut(id)?.operator.tactical.demkni_mut()?.accumulator += dt;
        if p.mode == Mode::Third {
            let keys = self.tac_keys(w, id, &p.grid)?;
            if w.unit(id)?.operator.tactical.demkni()?.aura >= 0.25 - 1e-9 {
                w.unit_mut(id)?.operator.tactical.demkni_mut()?.aura = 0.;
                let ids = self.spawns.as_ref().unwrap().live().to_vec();
                for a in ids {
                    if Self::tac_up(w, a) && body::in_range(w.unit(a)?, &keys) {
                        self.component_buff(
                            w,
                            r,
                            a,
                            "saria:calcify",
                            [
                                ("artsTakenMul".into(), p.arts),
                                ("moveMul".into(), (1. + p.slow).max(0.)),
                            ]
                            .into(),
                            0.5,
                            id,
                        )?;
                    }
                }
            }
            if w.unit(id)?.operator.tactical.demkni()?.accumulator >= 1. - 1e-9 {
                w.unit_mut(id)?.operator.tactical.demkni_mut()?.accumulator -= 1.;
                let amount = Self::tac_stat(w, id, "atk")? * p.heal;
                for a in self.tac_heal_targets_in_order(w, id, &keys, true)? {
                    self.tac_heal(w, r, id, a, amount)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn demkni_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Demkni,
    ) -> Result<(), String> {
        if due[4] {
            let n = ((w.time - w.unit(id)?.deployment.deployed_at + 1e-6) / p.interval.max(1.))
                .floor()
                .max(0.) as u32;
            let n = n.min(p.stacks);
            if n > 0 {
                let old = w
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("saria:suit"))
                    .map(|b| b.stacks);
                if old != Some(n) {
                    let mut b = Buff {
                        key: Some("saria:suit".into()),
                        stacks: n,
                        mods: [("atkPct".into(), p.attack), ("defPct".into(), p.defence)].into(),
                        ..Default::default()
                    };
                    b.runtime.max_stacks = p.stacks;
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
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn demkni_heal_event(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        e: &health::Event,
        p: &Demkni,
    ) -> Result<(), String> {
        if e.kind != health::Kind::Regen && e.amount > 0. {
            self.advanced_grant_sp(w, r, e.target, p.sp)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn demkni_tick(
        &mut self,
        w: &mut World,
        id: Id,
        s: &mut skills::Runtime,
        p: &Demkni,
    ) -> Result<(), String> {
        if s.bank.ready()
            && skills::can_act(w, id)?
            && !Self::tac_has(w, id, "silence")
            && w.time - s.last_start >= Self::tac_stat(w, id, "interval")? - 1e-9
        {
            let keys = self.tac_keys(w, id, &p.grid)?;
            if !self.tac_heal_targets(w, id, &keys, true)?.is_empty() {
                s.activate(w, self, "DEFAULT", false)?;
            }
        }
        Ok(())
    }
}

impl Behavior for Demkni {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_))
    }
    fn refreshes_skill(&self) -> bool {
        self.mode == Mode::Second
    }
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        match (role, event) {
            (Role::Source, KitEvent::Heal { event, .. }) => {
                k.host.demkni_heal_event(k.world, k.buffs, event, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn condition(
        &self,
        host: &Host,
        world: &mut World,
        unit: Id,
        condition: skills::Condition,
    ) -> Result<Option<bool>, String> {
        if self.mode == Mode::First
            && matches!(
                condition,
                skills::Condition::Default
                    | skills::Condition::Allies
                    | skills::Condition::TickRule
            )
        {
            let keys = host.tac_keys(world, unit, &self.grid)?;
            for target in host.tac_heal_targets(world, unit, &keys, true)? {
                if world.unit(target)?.stats.hp / Host::tac_stat(world, target, "maxHp")?
                    <= 0.5 + 1e-9
                {
                    return Ok(Some(true));
                }
            }
            return Ok(Some(false));
        }
        Ok(None)
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Scheduled(due) => {
                if k.up() {
                    k.host.demkni_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.demkni_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                k.host
                    .demkni_skill_tick(context.dt, k.world, id, k.buffs, self)?;
            }
            KitEvent::Refresh { skill, .. } => {
                k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                    host.demkni_tick(world, id, skill, self)
                })?;
            }
            _ => {}
        }
        Ok(())
    }
}
