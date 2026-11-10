// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::element;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "SUPPORT",
    branch: "ritualist",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_cello_1",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_cello_2",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_cello_3",
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
    let tm = talent_at(raw, -1);
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_cello_1" => Mode::First,
        "skchr_cello_2" => Mode::Second,
        "skchr_cello_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    s.modifiers = match mode {
        Mode::First => Default::default(),
        Mode::Second => [("aspd".into(), n(bb, "attack_speed", 0.))].into(),
        Mode::Third => [("atkPct".into(), n(bb, "atk", 0.))].into(),
    };
    if mode == Mode::First {
        s.combat.profile.only_while_skill = true;
        s.combat.profile.priority = targeting::Priority::NotBurst;
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
    }
    if mode == Mode::Second {
        let mut p = s.combat.profile;
        p.max_targets = 2.;
        s.combat.skill_profile = Some(p);
    }
    if mode == Mode::Third {
        skill_range(raw, s)?;
        let mut p = s.combat.profile;
        p.no_attack = true;
        s.combat.skill_profile = Some(p);
    }
    s.extra.tactics = Tactics::Cello(Cello {
        mode: mode,
        ep: n(bb, "ep_damage_ratio", 0.),
        aura_ep: n(t, "ep_damage_ratio", 0.),
        sluggish: n(t, "sluggish", 0.),
        amp: n(t1, "ep_damage_scale", 1.),
        fragile: n(t1, "damage_scale", 1.) - 1.,
        elite_mul: n(tb, "ep_damage_scale", 1.),
        wide: !t1["damage_value"].is_null(),
        dot: n(t1, "damage_value", 0.),
        dot_interval: n(t1, "interval", 1.).max(0.1),
        module_fragile: n(tm, "damage_scale", 1.) - 1.,
        boost: n(bb, "scale_delta_to_one", 1.),
        hp_up: n(bb, "cello_s_3[max_hp].max_hp", 0.),
        atk_up: n(bb, "cello_s_3[atk].atk", 0.),
        def_up: n(bb, "cello_s_3[def].def", 0.),
    });
    Ok(())
}
pub(crate) struct Cello {
    pub mode: Mode,
    pub ep: f64,
    pub aura_ep: f64,
    pub sluggish: f64,
    pub amp: f64,
    pub fragile: f64,
    pub elite_mul: f64,
    pub wide: bool,
    pub dot: f64,
    pub dot_interval: f64,
    pub module_fragile: f64,
    pub boost: f64,
    pub hp_up: f64,
    pub atk_up: f64,
    pub def_up: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub boost: f64,
    pub partner: Option<Id>,
    pub pick: f64,
    pub tango: f64,
    pub dot: f64,
}

impl Cello {
    fn boost(&self, k: &KitContext<'_>) -> Result<f64, String> {
        Ok(k.world
            .unit(k.unit)?
            .operator
            .tactical
            .cello()?
            .boost
            .max(1.))
    }
    fn partner(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let keys = k.keys()?;
        let mut best = None;
        let mut attack = f64::NEG_INFINITY;
        for unit in k.allies(&keys)? {
            if unit == k.unit {
                continue;
            }
            let value = k.stat(unit, "atk")?;
            if value > attack + 1e-9 {
                best = Some(unit);
                attack = value;
            }
        }
        let s = k.world.unit_mut(k.unit)?.operator.tactical.cello_mut()?;
        s.partner = best;
        s.pick = 0.5;
        Ok(())
    }
}
impl Behavior for Cello {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Hit { .. }) {
            -1000
        } else {
            0
        }
    }
    fn profile(
        &self,
        _: &World,
        _: Id,
        skill: Option<&skills::Runtime>,
        profile: &mut search::AttackProfile,
    ) -> Result<(), String> {
        if self.mode == Mode::First {
            profile.only_while_skill = !skill.is_some_and(|s| s.bank.ready() || s.bank.active);
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                if self.mode == Mode::Second {
                    self.partner(k)?;
                }
                if self.mode == Mode::Third {
                    let s = k.world.unit_mut(id)?.operator.tactical.cello_mut()?;
                    s.boost = self.boost;
                    s.tango = f64::INFINITY;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                if self.mode == Mode::Second {
                    let partner = k.world.unit(id)?.operator.tactical.cello()?.partner;
                    let partner_up = partner.is_some_and(|a| Host::tac_up(k.world, a));
                    let pick = {
                        let s = k.world.unit_mut(id)?.operator.tactical.cello_mut()?;
                        s.pick -= context.dt;
                        s.pick <= 0. || !partner_up
                    };
                    if pick {
                        self.partner(k)?;
                    }
                }
                if self.mode == Mode::Third {
                    let due = {
                        let s = k.world.unit_mut(id)?.operator.tactical.cello_mut()?;
                        s.tango += context.dt;
                        if s.tango >= 0.5 {
                            s.tango = 0.;
                            true
                        } else {
                            false
                        }
                    };
                    if due {
                        let keys = k.keys()?;
                        let allies = k.allies(&keys)?;
                        for (stat, key, value) in [
                            ("maxHp", "cello:tango:hp", self.hp_up),
                            ("atk", "cello:tango:atk", self.atk_up),
                            ("def", "cello:tango:def", self.def_up),
                        ] {
                            let mut best = None;
                            let mut maximum = f64::NEG_INFINITY;
                            for &a in &allies {
                                if a == id {
                                    continue;
                                }
                                let v = k.world.unit(a)?.stats.base_number(stat);
                                if v > maximum + 1e-9 {
                                    maximum = v;
                                    best = Some(a);
                                }
                            }
                            if let Some(a) = best {
                                k.buff(
                                    a,
                                    key,
                                    [(
                                        if stat == "maxHp" {
                                            "hpPct"
                                        } else if stat == "atk" {
                                            "atkPct"
                                        } else {
                                            "defPct"
                                        }
                                        .into(),
                                        value,
                                    )]
                                    .into(),
                                    0.75,
                                )?;
                            }
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let s = k.world.unit_mut(id)?.operator.tactical.cello_mut()?;
                s.partner = None;
                s.boost = 1.;
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && self.mode == Mode::First
                    && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target.filter(|&a| Host::tac_up(k.world, a)) {
                    let amount = k.stat(id, "atk")? * self.ep;
                    k.elemental(target, amount, element::Element::Apoptosis)?;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if self.mode == Mode::Second
                && k.up()
                && k.active()?
                && request.hit.damage_type != DamageType::Elemental
                && request.damage.amount > 0.
                && k.world.unit(request.hit.target)?.side == health::Side::Enemy =>
            {
                if request.hit.source == Some(id)
                    || request.hit.source == k.world.unit(id)?.operator.tactical.cello()?.partner
                {
                    let amount = k.stat(id, "atk")? * self.ep;
                    if amount > 0. {
                        k.elemental(request.hit.target, amount, element::Element::Apoptosis)?;
                    }
                }
            }
            KitEvent::Scheduled(due) if k.up() => {
                let foes = k.enemies(None, true)?;
                if due[4] {
                    for &target in &foes {
                        let amount = k.stat(id, "atk")? * self.aura_ep;
                        if amount > 0. {
                            k.elemental(target, amount, element::Element::Apoptosis)?;
                        }
                        if self.sluggish > 0. && Host::tac_up(k.world, target) {
                            k.status(target, "sluggish", self.sluggish, None)?;
                        }
                    }
                }
                if due[2] {
                    let boost = self.boost(k)?;
                    for &target in &foes {
                        if self.fragile > 0. && k.has(target, "apoptosisBurst") {
                            k.status(target, "elemFragile", 0.4, Some(self.fragile * boost))?;
                        }
                        if self.module_fragile > 0. {
                            k.status(target, "elemFragile", 0.4, Some(self.module_fragile))?;
                        }
                    }
                }
            }
            KitEvent::Tick if self.dot > 0. => {
                let now = k.world.time;
                let due = {
                    let state = k.world.unit_mut(id)?.operator.tactical.cello_mut()?;
                    let at = state.dot.max(self.dot_interval);
                    if now + 1e-9 >= at {
                        state.dot = at + self.dot_interval;
                        true
                    } else {
                        false
                    }
                };
                if due && k.up() {
                    let amount = self.dot * self.boost(k)?;
                    for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
                        if Host::tac_up(k.world, target)
                            && !k.world.unit(target)?.hidden
                            && k.has(target, "apoptosisBurst")
                        {
                            k.element_damage(
                                target,
                                amount,
                                element::Element::Apoptosis,
                                "elementDmg",
                            )?;
                        }
                    }
                }
            }
            KitEvent::Element {
                phase: element::Phase::Hit,
                request,
            } if request.damage.element == Some(element::Element::Apoptosis)
                && k.world.unit(request.damage.hit.target)?.side == health::Side::Enemy =>
            {
                if k.host.tac_leader(k.world, id)? {
                    let target = request.damage.hit.target;
                    let mut best = 1.;
                    for caster in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if !Host::tac_up(k.world, caster) {
                            continue;
                        }
                        let def = k.host.tac_def(caster)?;
                        if let Tactics::Cello(p) = &def.extra.tactics {
                            let keys = k.host.roster.as_ref().unwrap().range(k.world, caster)?;
                            if p.wide || body::in_range(k.world.unit(target)?, &keys) {
                                let boost = k
                                    .world
                                    .unit(caster)?
                                    .operator
                                    .tactical
                                    .cello()?
                                    .boost
                                    .max(1.);
                                let scale = 1. + (p.amp - 1.) * boost;
                                if scale > best {
                                    best = scale;
                                }
                            }
                        }
                    }
                    request.damage.damage.mul *= best;
                }
                if request.damage.hit.source == Some(id)
                    && (k.world.unit(request.damage.hit.target)?.boss
                        || k.world.unit(request.damage.hit.target)?.targeting.elite)
                {
                    request.damage.damage.mul *= self.elite_mul;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            dot: previous.map_or(0., |state| state.dot),
            ..Self::default()
        }
    }
}
