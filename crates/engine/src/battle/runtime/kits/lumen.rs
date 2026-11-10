// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "healer",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_lumen_3",
            kind: Kind::Ammo,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_lumen_1",
            kind: Kind::Instant,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_lumen_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
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

    healing(s, 1.);
    if mode == Mode::Third {
        s.extra.effects.ammo_count = n(bb, "attack@trigger_time", 4.);
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("aspd".into(), n(bb, "attack_speed", 0.)),
        ]
        .into();
    }
    if n(tb, "taunt_level", 0.) != 0. {
        s.extra.effects.permanent.push(persistent(
            "lumen:scope",
            [("taunt".into(), n(tb, "taunt_level", 0.))].into(),
        ));
    }
    if n(tb, "one_minus_status_resistance", 0.) < 0. {
        let mut b = persistent("lumen:scopeResist", Numbers::new());
        b.runtime.status = Some("resist".into());
        b.runtime.tick_payload = -n(tb, "one_minus_status_resistance", 0.);
        s.extra.effects.permanent.push(b);
    }
    s.extra.tactics = Tactics::Lumen(Lumen {
        mode,
        heal: n(bb, "heal_scale", 1.),
        targets: n(bb, "max_target", 2.).max(1.) as usize,
        rain: suffix_number(bb, "heal_scale", 0.),
        rain_life: suffix_number(bb, "projectile_life_time", 4.),
        rain_interval: suffix_number(bb, "interval", 1.),
        base: n(t, "status_resistance[limit]", 0.),
        special: n(
            t,
            "lumen_t_1[special].status_resistance[limit]",
            n(t, "status_resistance[limit]", 0.),
        ),
        resist: (-n(t, "one_minus_status_resistance", -0.5)).clamp(0., 1.),
        threshold: n(t, "hp_ratio", 0.75),
        emergency: n(t1, "heal_scale", 0.),
        cooldown: n(t1, "duration", 12.),
        far: n(tb, "heal_scale", 0.8),
    });
    Ok(())
}

pub(crate) struct Lumen {
    pub mode: Mode,
    pub heal: f64,
    pub targets: usize,
    pub rain: f64,
    pub rain_life: f64,
    pub rain_interval: f64,
    pub base: f64,
    pub special: f64,
    pub resist: f64,
    pub threshold: f64,
    pub emergency: f64,
    pub cooldown: f64,
    pub far: f64,
}

pub(crate) struct State {
    pub abnormal: bool,
    pub ready: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            abnormal: Default::default(),
            ready: f64::NEG_INFINITY,
        }
    }
}

impl Host {
    pub(in crate::battle::runtime) fn lumen_start(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        s: &mut skills::Runtime,
        p: &Lumen,
    ) -> Result<(), String> {
        if p.mode == Mode::Second {
            let full =
                s.bank.max_charges() > 1 && s.bank.charges >= f64::from(s.bank.max_charges() - 1);
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            let mut ids = self.tac_heal_targets(w, id, &keys, true)?;
            if full {
                for a in self.tac_heal_targets(w, id, &keys, false)? {
                    if !ids.contains(&a) && Self::tac_abnormal(w, a) {
                        ids.push(a);
                    }
                }
            }
            for a in ids.into_iter().take(p.targets) {
                let amount = Self::tac_stat(w, id, "atk")? * p.heal;
                self.tac_heal(w, r, id, a, amount)?;
                if full {
                    self.tac_cleanse(w, r, a)?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_before(
        &mut self,
        w: &mut World,
        id: Id,
        active: bool,
        c: &mut attacks::Context,
        p: &Lumen,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Third {
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            let targets = self.tac_heal_targets(w, id, &keys, false)?;
            let abnormal = targets.into_iter().find(|&a| Self::tac_abnormal(w, a));
            w.unit_mut(id)?.operator.tactical.lumen_mut()?.abnormal = abnormal.is_some();
            if let Some(a) = abnormal {
                c.targets = vec![a];
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_attack(
        &mut self,
        w: &mut World,
        id: Id,
        active: bool,
        c: &mut attacks::Context,
        p: &Lumen,
    ) -> Result<(), String> {
        if active && p.mode == Mode::Third {
            c.no_ammo = !w.unit(id)?.operator.tactical.lumen()?.abnormal;
            w.unit_mut(id)?.operator.tactical.lumen_mut()?.abnormal = false;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_impact(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        target: Id,
        impact: attacks::Impact,
        p: &Lumen,
    ) -> Result<(), String> {
        if impact.heal && impact.is_skill {
            if p.mode == Mode::Third && w.unit(id)?.operator.tactical.lumen()?.abnormal {
                self.tac_cleanse(w, r, target)?;
            }
            if p.mode == Mode::First && p.rain > 0. {
                let position = w.unit(target)?.position;
                for a in self.roster.as_ref().unwrap().ids().to_vec() {
                    if Self::tac_up(w, a)
                        && body::in_radius(w.unit(a)?, position, 1.5)
                        && !Self::tac_has(w, a, "noHeal")
                        && !w.unit(a)?.profile_no_heal
                    {
                        let mut b = Buff {
                            key: Some(format!("lumen:rain:{id}")),
                            ..Default::default()
                        };
                        b.runtime.source = Some(id);
                        b.runtime.duration = p.rain_life;
                        b.runtime.interval = p.rain_interval.max(0.2);
                        b.runtime.tick_payload = p.rain;
                        b.runtime.on_tick = Some(RAIN);
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
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_heal_event(
        &mut self,
        w: &mut World,
        id: Id,
        r: &mut buffs::Runtime,
        e: &health::Event,
        p: &Lumen,
    ) -> Result<(), String> {
        if e.kind != health::Kind::Regen && p.resist > 0. && p.base > 0. {
            let max = Self::tac_stat(w, e.target, "maxHp")?;
            let after = (w.unit(e.target)?.stats.hp + e.amount).min(max);
            self.advanced_status(
                w,
                r,
                id,
                e.target,
                "resist",
                if after / max > p.threshold {
                    p.special
                } else {
                    p.base
                },
                Some(p.resist),
            )?;
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_tick(
        &mut self,
        w: &mut World,
        id: Id,
        def: &Definition,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        if s.bank.active
            && w.unit(id)?.cadence.cooldown <= 0.
            && skills::can_act(w, id)?
            && !Self::tac_has(w, id, "disarm")
        {
            let keys = self.roster.as_ref().unwrap().range(w, id)?;
            if self.tac_heal_targets(w, id, &keys, true)?.is_empty() {
                if let Some(target) = self
                    .tac_heal_targets(w, id, &keys, false)?
                    .into_iter()
                    .find(|&a| Self::tac_abnormal(w, a))
                {
                    ally::Host::perform(
                        self,
                        id,
                        def.combat.profile,
                        search::Selection {
                            targets: vec![target],
                            fortress_melee: None,
                        },
                        Some(s),
                        w,
                    )?;
                    w.unit_mut(id)?.cadence.cooldown = Self::tac_stat(w, id, "interval")?;
                }
            }
        }
        Ok(())
    }
    pub(in crate::battle::runtime) fn lumen_rain_tick(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        c: buffs::Context,
    ) -> Result<(), String> {
        let b = r.get(w, c.unit, c.buff)?.clone();
        let id = b.runtime.source.ok_or("Missing lumen buff source")?;

        if w.unit(c.unit)?.stats.hp >= Self::tac_stat(w, c.unit, "maxHp")? {
            return Ok(());
        }
        let amount = Self::tac_stat(w, id, "atk")? * b.runtime.tick_payload;
        self.tac_heal(w, r, id, c.unit, amount)?;
        Ok(())
    }
}

impl Behavior for Lumen {
    fn observes(&self, event: &KitEvent<'_>) -> bool {
        event.is_lifecycle() || matches!(event, KitEvent::StatusFinished(_))
    }
    fn refreshes_skill(&self) -> bool {
        self.mode == Mode::Third
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
                    token: RAIN,
                },
            ) => {
                k.host.lumen_rain_tick(k.world, k.buffs, *context)?;
            }

            (Role::Source, KitEvent::Before(context)) => {
                let active = k.active()?;
                k.host.lumen_before(k.world, id, active, context, self)?;
            }
            (Role::Source, KitEvent::Attack { context, .. }) => {
                let active = k.active()?;
                k.host.lumen_attack(k.world, id, active, context, self)?;
            }
            (Role::Source, KitEvent::Impact(impact)) => {
                if let Some(target) = impact.target {
                    k.host
                        .lumen_impact(k.world, id, k.buffs, target, *impact, self)?;
                }
            }
            (Role::Source, KitEvent::Heal { event, .. }) => {
                k.host.lumen_heal_event(k.world, id, k.buffs, event, self)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn attack_heal(&self, world: &World, unit: Id, target: Id, amount: f64) -> Result<f64, String> {
        Ok(
            if self.mode == Mode::Third
                && world.unit(unit)?.operator.active_skill
                && Host::tac_abnormal(world, target)
            {
                amount * self.heal
            } else {
                amount
            },
        )
    }
    fn heal_amount(
        &self,
        world: &World,
        unit: Id,
        target: Id,
        amount: f64,
        options: health::HealOptions,
    ) -> Result<f64, String> {
        let a = world.unit(unit)?.deployment.tile;
        let b = world.unit(target)?.deployment.tile;
        Ok(
            if !options.regen
                && !options.restore
                && (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) > 2
            {
                amount * self.far
            } else {
                amount
            },
        )
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::StatusFinished(context) => {
                if ABNORMAL.contains(&context.key.as_str())
                    && k.world.unit(context.target)?.side == health::Side::Ally
                    && k.up()
                    && self.emergency > 0.
                    && k.world.time >= k.world.unit(id)?.operator.tactical.lumen()?.ready
                {
                    let keys = k.keys()?;
                    let tile = k.world.unit(context.target)?.deployment.tile;
                    if keys.contains(&(tile[0] as usize * COLS + tile[1] as usize)) {
                        k.world.unit_mut(id)?.operator.tactical.lumen_mut()?.ready =
                            k.world.time + self.cooldown;
                        let amount = k.stat(id, "atk")? * self.emergency;
                        k.host
                            .tac_heal(k.world, k.buffs, id, context.target, amount)?;
                    }
                }
            }

            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                k.host.lumen_start(k.world, id, k.buffs, skill, self)?;
            }
            KitEvent::Refresh { skill, .. } => {
                k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                    let definition = host.tac_def(id)?;
                    host.lumen_tick(world, id, &definition, skill)
                })?;
            }
            _ => {}
        }
        Ok(())
    }
}
