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
            id: "skchr_blemsh_3",
            kind: Kind::Duration,
            sp_type: SpType::Hurt,
            rule: "TAKE_DAMAGE",
            attack_override: true,
        },
        Skill {
            id: "skchr_blemsh_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_blemsh_2",
            kind: Kind::Duration,
            sp_type: SpType::Hurt,
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

    s.combat.profile.selection.hit_sleep = true;
    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
    } else {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("defPct".into(), n(bb, "def", 0.)),
        ]
        .into();
    }
    if mode == Mode::Third {
        s.rule_override = Some(crate::battle::skills::Rule::Default);
    }
    if n(tb, "damage_resistance", 0.) > 0. {
        s.extra.effects.permanent.push(persistent(
            "trait:blemsh_guard",
            [("dmgTakenMul".into(), 1. - n(tb, "damage_resistance", 0.))].into(),
        ));
    }
    if n(tb, "heal_scale", 0.) > 0. {
        s.extra.support.heal_bonus = Some(HealBonus {
            below: n(tb, "hp_ratio", 0.5),
            scale: n(tb, "heal_scale", 1.),
            inclusive: false,
        });
    }
    s.extra.tactics = Tactics::Blemsh(Blemsh {
        mode,
        grid: tactical_grid(&raw["skill"]["rangeGrid"], true)?,
        heal: n(bb, "heal_scale", 0.),
        regen: n(
            bb,
            "attack@atk_to_hp_recovery_ratio",
            n(bb, "atk_to_hp_recovery_ratio", 0.),
        ),
        bonus: n(bb, "attack@blemsh_s_3_extra_dmg[magic].atk_scale", 0.),
        mercy: n(t1, "atk_scale", 1.),
        sp: n(t, "sp", 1.),
    });
    Ok(())
}

pub(crate) struct Blemsh {
    pub mode: Mode,
    pub grid: Vec<[i32; 2]>,
    pub heal: f64,
    pub regen: f64,
    pub bonus: f64,
    pub mercy: f64,
    pub sp: f64,
}

#[derive(Default)]
pub(crate) struct State {
    pub aura: f64,
}

impl Host {
    pub(in crate::battle::runtime) fn tac_blemsh_aura(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        id: Id,
        p: &Blemsh,
    ) -> Result<(), String> {
        let keys = self.tac_keys(w, id, &p.grid)?;
        let value = Self::tac_stat(w, id, "atk")? * p.regen;
        if value > 0. {
            for a in self.roster.as_ref().unwrap().ids().to_vec() {
                let u = w.unit(a)?;
                if !u.stats.alive
                    || !u.deployed
                    || u.hidden
                    || u.deployment.kind == crate::battle::deploy::Kind::Device
                    || !body::in_range(u, &keys)
                {
                    continue;
                }
                if a != id
                    && w.unit_mut(a)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("isolated")
                {
                    continue;
                }
                self.component_buff(
                    w,
                    r,
                    a,
                    &format!("blemsh:regen:{id}"),
                    [("hpRegen".into(), value)].into(),
                    0.5,
                    id,
                )?;
            }
        }
        let mut extra = vec![];
        for &a in self.spawns.as_ref().unwrap().live() {
            if Self::tac_up(w, a)
                && Self::tac_has(w, a, "sleep")
                && body::in_range(w.unit(a)?, &keys)
            {
                extra.extend(body::keys(w.unit(a)?));
            }
        }
        extra.sort_unstable();
        extra.dedup();
        w.unit_mut(id)?.operator.advanced.extra_range = extra;
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        s: &mut skills::Runtime,
        p: &Blemsh,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            let tile = w.unit(id)?.deployment.tile;
            let blocked = w.unit(id)?.blocking().to_vec();
            let enemies = self.spawns.as_ref().unwrap().live().to_vec();
            for a in enemies {
                if !w.unit(a)?.stats.alive || w.unit(a)?.hidden {
                    continue;
                }
                if blocked.contains(&a)
                    || !targeting::flying(w, a)? && body::on_tile(w.unit(a)?, tile[0], tile[1])
                {
                    self.advanced_status(w, r, id, a, "sleep", s.time_left, None)?;
                }
            }
            w.unit_mut(id)?.operator.tactical.blemsh_mut()?.aura = 0.;
            self.tac_blemsh_aura(w, r, id, p)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_end(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Blemsh,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            w.unit_mut(id)?.operator.advanced.extra_range.clear();
            for a in self.roster.as_ref().unwrap().ids().to_vec() {
                r.remove(w, self, a, &format!("blemsh:regen:{id}"))?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_skill_tick(
        &mut self,
        dt: f64,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Blemsh,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.blemsh_mut()?.aura += dt;
        if p.mode == Mode::Second {
            if w.unit(id)?.operator.tactical.blemsh()?.aura >= 0.25 - 1e-9 {
                w.unit_mut(id)?.operator.tactical.blemsh_mut()?.aura = 0.;
                self.tac_blemsh_aura(w, r, id, p)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_before(
        &mut self,
        w: &mut World,
        id: Id,
        c: &mut attacks::Context,
    ) -> Result<(), String> {
        let keys = self.roster.as_ref().unwrap().range(w, id)?;
        for a in self.spawns.as_ref().unwrap().live() {
            if Self::tac_up(w, *a)
                && Self::tac_has(w, *a, "sleep")
                && body::in_range(w.unit(*a)?, &keys)
            {
                c.targets.retain(|t| t != a);
                c.targets.insert(0, *a);
                c.targets.truncate(1);
                break;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_attack(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        p: &Blemsh,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Third && p.heal > 0. {
            let keys = self.tac_keys(w, id, &p.grid)?;
            if let Some(a) = self
                .tac_heal_targets(w, id, &keys, true)?
                .into_iter()
                .find(|a| *a != id)
            {
                let amount = Self::tac_stat(w, id, "atk")? * p.heal;
                self.tac_heal(w, r, id, a, amount)?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Blemsh,
    ) -> Result<(), String> {
        if impact.is_skill {
            if p.mode == Mode::First {
                let keys = self.tac_keys(w, id, &p.grid)?;
                if let Some(&a) = self.tac_heal_targets(w, id, &keys, true)?.first() {
                    let amount = Self::tac_stat(w, id, "atk")? * p.heal;
                    self.tac_heal(w, r, id, a, amount)?;
                }
            } else if p.mode == Mode::Third && Self::tac_up(w, target) {
                let amount = Self::tac_stat(w, id, "atk")? * p.bonus;
                self.tac_damage(
                    w,
                    r,
                    id,
                    target,
                    amount,
                    DamageType::Arts,
                    false,
                    "blemshBonus",
                )?;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn blemsh_phase(
        &mut self,
        w: &mut World,
        q: &mut hit::Request,
        c: &Blemsh,
    ) -> Result<(), String> {
        if w.unit(q.hit.target)?.side == health::Side::Enemy
            && Self::tac_has(w, q.hit.target, "sleep")
        {
            q.damage.mul *= c.mercy;
        }
        Ok(())
    }
}

impl Behavior for Blemsh {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::AttackFinished { .. })
    }
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        let id = k.unit;
        match (role, event) {
            (Role::Source, KitEvent::Before(context)) => {
                k.host.blemsh_before(k.world, id, context)?;
            }
            (Role::Source, KitEvent::Attack { .. }) => {
                let active = k.active()?;
                k.host.blemsh_attack(k.world, id, k.buffs, active, self)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .blemsh_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (
                Role::Source,
                KitEvent::Hit {
                    phase: hit::Phase::Hit,
                    request,
                },
            ) => {
                k.host.blemsh_phase(k.world, request, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::AttackFinished { context, skill } => {
                let first = k
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .ids()
                    .iter()
                    .copied()
                    .find(|&unit| {
                        Host::tac_up(k.world, unit)
                            && k.host
                                .tac_def(unit)
                                .is_ok_and(|def| matches!(def.extra.tactics, Tactics::Blemsh(_)))
                    });
                if first == Some(id)
                    && k.host.tac_def(context.attacker)?.sp_type() == skills::SpType::Hurt
                {
                    if let Some(skill) = skill {
                        k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                            skill.gain(world, host, self.sp, "talent", false, false)
                        })?;
                    } else {
                        k.host
                            .advanced_grant_sp(k.world, k.buffs, context.attacker, self.sp)?;
                    }
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                k.host.blemsh_start(k.world, id, k.buffs, skill, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                k.host
                    .blemsh_skill_tick(context.dt, k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                k.host.blemsh_end(k.world, id, k.buffs, self)?;
            }
            _ => {}
        }
        Ok(())
    }
}
