// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "hammer",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_pepe_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_pepe_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_pepe_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "ACTIVE_RANGE",
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

    let splash = Splash {
        radius: n(tb, "attack@ability_range_radius", 1.),
        scale: n(tb, "attack@atk_scale_2", 0.5),
        hits_fly: false,
        fortress_melee: false,
    };
    s.combat.area.splash = Some(splash);
    s.combat.skill_area = s.combat.area;
    skill_range(raw, s)?;
    if mode == Mode::First {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
    } else {
        let base_attack_time = n(bb, "base_attack_time", 0.);
        let bat_pct = if base_attack_time > 0. {
            base_attack_time
        } else {
            flat_bat(raw, base_attack_time).max(-0.9)
        };
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("aspd".into(), n(bb, "attack_speed", 0.)),
            ("batPct".into(), bat_pct),
        ]
        .into();
    }
    s.extra.tactics = Tactics::Pepe(Pepe {
        mode,
        stack_attack: n(bb, "attack@atk", 0.),
        stacks: n(bb, "attack@max_stack_cnt", 0.).max(0.) as u32,
        extend: n(bb, "attack@ability_range_forward_extend", 0.),
        stun: n(bb, "attack@stun", 0.),
        main_stun: n(bb, "attack@stun_main", n(bb, "attack@stun", 0.)),
        sp: n(t, "sp", 0.),
        sp_cap: n(t, "max_sp", 0.),
        aura: n(t1, "atk", 0.),
        module_scale: n(tb, "atk_scale_e", 1.),
        module_count: n(tb, "cnt", f64::INFINITY),
        rage: n(bb, "attack_speed_extra", 0.),
        rage_cap: n(bb, "max_stack_cnt", 2.).max(0.) as u32,
    });
    Ok(())
}

pub(crate) struct Pepe {
    pub mode: Mode,
    pub stack_attack: f64,
    pub stacks: u32,
    pub extend: f64,
    pub stun: f64,
    pub main_stun: f64,
    pub sp: f64,
    pub sp_cap: f64,
    pub aura: f64,
    pub module_scale: f64,
    pub module_count: f64,
    pub rage: f64,
    pub rage_cap: u32,
}

#[derive(Default)]
pub(crate) struct State {
    pub stacks: u32,
    pub rage: u32,
    pub kills: u32,
    pub boost: bool,
}

impl Host {
    pub(in crate::battle::runtime) fn pepe_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        p: &Pepe,
    ) -> Result<(), String> {
        w.unit_mut(id)?.operator.tactical.pepe_mut()?.kills = 0;
        w.unit_mut(id)?.operator.tactical.pepe_mut()?.stacks = 0;
        if p.mode == Mode::Second {
            let n = w.unit(id)?.operator.tactical.pepe()?.rage;
            if n > 0 {
                self.component_buff(
                    w,
                    r,
                    id,
                    "pepe:rage",
                    [("aspd".into(), p.rage * f64::from(n))].into(),
                    f64::INFINITY,
                    id,
                )?;
            }
            w.unit_mut(id)?.operator.tactical.pepe_mut()?.rage =
                n.saturating_add(1).min(p.rage_cap);
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_end(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        s: &mut skills::Runtime,
        p: &Pepe,
    ) -> Result<(), String> {
        r.remove(w, self, id, "pepe:tremor")?;
        r.remove(w, self, id, "pepe:rage")?;
        if Self::tac_up(w, id) {
            let amount =
                (f64::from(w.unit(id)?.operator.tactical.pepe()?.kills) * p.sp).min(p.sp_cap);
            self.with_kit_buffs(w, r, |h, w| s.gain(w, h, amount, "talent", false, false))?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_scheduled(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        due: [bool; 5],
        p: &Pepe,
    ) -> Result<(), String> {
        if due[3] && p.aura != 0. {
            for a in self.roster.as_ref().unwrap().ids().to_vec() {
                if Self::tac_up(w, a) && self.tac_def(a)?.profession == "WARRIOR" {
                    self.component_buff(
                        w,
                        r,
                        a,
                        "pepe:lotus",
                        [("atkPct".into(), p.aura)].into(),
                        0.75,
                        id,
                    )?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_before(
        &mut self,
        w: &mut World,
        id: Id,
        def: &Definition,
        active: bool,
        c: &mut attacks::Context,
        p: &Pepe,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Second {
            let mut ids = self.tac_enemies(w, id, None, false)?;
            for &a in w.unit(id)?.blocking() {
                if !ids.contains(&a) {
                    ids.push(a);
                }
            }
            if let Some(&a) = self.rng.pick(&ids) {
                c.targets = vec![a];
            }
        }
        if p.module_scale > 1. {
            if let Some(&a) = c.targets.first() {
                let radius = def.combat.area.splash.map(|a| a.radius).unwrap_or(1.)
                    + if active && p.mode == Mode::Third {
                        p.extend * f64::from(w.unit(id)?.operator.tactical.pepe()?.stacks)
                    } else {
                        0.
                    };
                w.unit_mut(id)?.operator.tactical.pepe_mut()?.boost =
                    self.tac_radius(w, w.unit(a)?.position, radius)?.len() as f64 >= p.module_count;
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_attack(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        active: bool,
        p: &Pepe,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Third {
            let stack = w
                .unit(id)?
                .operator
                .tactical
                .pepe()?
                .stacks
                .saturating_add(1)
                .min(p.stacks);
            w.unit_mut(id)?.operator.tactical.pepe_mut()?.stacks = stack;
            self.component_buff(
                w,
                r,
                id,
                "pepe:tremor",
                [("atkPct".into(), p.stack_attack * f64::from(stack))].into(),
                f64::INFINITY,
                id,
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Pepe,
    ) -> Result<(), String> {
        if impact.is_skill
            && impact.kind == attacks::VictimKind::Main
            && p.mode == Mode::Third
            && Self::tac_up(w, target)
            && p.main_stun > 0.
        {
            self.advanced_status(w, r, id, target, "stun", p.main_stun, None)?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_phase(
        &mut self,
        w: &mut World,
        id: Id,
        q: &mut hit::Request,
        c: &Pepe,
    ) -> Result<(), String> {
        if q.is_attack && w.unit(id)?.operator.tactical.pepe()?.boost {
            q.damage.mul *= c.module_scale;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_kill(
        &mut self,
        w: &mut World,
        source: Id,
    ) -> Result<(), String> {
        if w.unit(source)?.operator.active_skill {
            w.unit_mut(source)?.operator.tactical.pepe_mut()?.kills = w
                .unit(source)?
                .operator
                .tactical
                .pepe()?
                .kills
                .saturating_add(1);
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn pepe_tick(
        &mut self,
        w: &mut World,
        id: Id,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        if s.bank.ready() && !s.bank.active && Self::tac_abnormal(w, id) {
            let mut r = self.buffs.take().unwrap();
            let result = self.tac_cleanse(w, &mut r, id);
            self.buffs = Some(r);
            result?;
            s.activate(w, self, "abnormal", false)?;
        }
        Ok(())
    }
}

impl State {
    pub fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            rage: previous.map_or(0, |p| p.rage),
            ..Default::default()
        }
    }
}

impl Behavior for Pepe {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::Scheduled(_))
    }
    fn refreshes_skill(&self) -> bool {
        self.mode == Mode::First
    }
    fn payload(
        &self,
        world: &World,
        unit: Id,
        payload: &mut attacks::Direct,
        area: &mut attacks::Area,
    ) -> Result<(), String> {
        if self.mode == Mode::Third && payload.is_skill {
            if let Some(splash) = &mut area.splash {
                splash.radius +=
                    self.extend * f64::from(world.unit(unit)?.operator.tactical.pepe()?.stacks);
            }
        }
        Ok(())
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
                let active = k.active()?;
                let definition = k.host.tac_def(id)?;
                k.host
                    .pepe_before(k.world, id, &definition, active, context, self)?;
            }
            (Role::Source, KitEvent::Attack { .. }) => {
                let active = k.active()?;
                k.host.pepe_attack(k.world, id, k.buffs, active, self)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .pepe_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (
                Role::Source,
                KitEvent::Hit {
                    phase: hit::Phase::Hit,
                    request,
                },
            ) => {
                k.host.pepe_phase(k.world, id, request, self)?;
            }
            (Role::Source, KitEvent::Landed { event, request })
                if self.mode == Mode::Third
                    && request.is_splash
                    && self.stun > 0.
                    && k.active()?
                    && Host::tac_up(k.world, event.target) =>
            {
                k.host.advanced_status(
                    k.world,
                    k.buffs,
                    id,
                    event.target,
                    "stun",
                    self.stun,
                    None,
                )?;
            }
            (Role::Source, KitEvent::Kill { .. }) => {
                k.host.pepe_kill(k.world, id)?;
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
                    k.host.pepe_scheduled(k.world, id, k.buffs, *due, self)?;
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                k.host.pepe_start(k.world, id, k.buffs, self)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                skill,
                ..
            } if *unit == id => {
                k.host.pepe_end(k.world, id, k.buffs, skill, self)?;
            }
            KitEvent::Refresh { skill, .. } => {
                k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                    host.pepe_tick(world, id, skill)
                })?;
            }
            _ => {}
        }
        Ok(())
    }
}
