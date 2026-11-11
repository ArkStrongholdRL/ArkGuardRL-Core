// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::element;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "MEDIC",
    branch: "wandermedic",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "orb",
    projectile_speed: 10.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_agoat2_3",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "GDGLOW_SKILL_2",
            attack_override: true,
        },
        Skill {
            id: "skchr_agoat2_1",
            kind: Kind::Toggle,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_agoat2_2",
            kind: Kind::Instant,
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
    let mode = match raw["skill"]["skillId"].as_str().unwrap_or("") {
        "skchr_agoat2_1" => Mode::First,
        "skchr_agoat2_2" => Mode::Second,
        "skchr_agoat2_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    healing(s, 1.);
    if let Some(h) = &mut s.combat.profile.heal {
        h.element_heal = true;
    }
    if mode == Mode::First {
        s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
        s.rule_override = Some(skills::Rule::SpFull);
        skill_healing(s, 2., 1.);
    }
    if mode == Mode::Third {
        s.extra.skill_range = Some(
            (-(crate::battle::grid::ROWS as i32 - 1)..crate::battle::grid::ROWS as i32)
                .flat_map(|r| (-(COLS as i32 - 1)..COLS as i32).map(move |c| [r, c]))
                .collect(),
        );
        let shots = kit_text_count(&raw["skill"]["desc"], "", "连发", 1, 5)?;
        skill_healing(s, shots as f64, n(bb, "attack@heal_scale", 1.));
        if let Some(p) = &mut s.combat.skill_profile {
            p.heal.as_mut().unwrap().element_heal = true;
        }
    }
    s.extra.tactics = Tactics::Agoat2(Agoat2 {
        mode: mode,
        element_ratio: n(tb, "ep_heal_ratio", 0.5),
        shots: kit_text_count(&raw["skill"]["desc"], "", "连发", 1, 5)? as usize,
        aura_element: suffix_number(bb, "ep_heal_ratio", 0.),
        veil: suffix_number(bb, "atk_scale", 5.),
        veil_duration: n(bb, "duration", 12.),
        mist: n(t, "heal_scale", 0.),
        mist_duration: n(t, "duration", 6.),
        mist_cap: n(t, "max_stack_cnt", 3.).max(1.) as u32,
        ash_hp: n(t1, "max_hp", 0.),
        ash_cut: n(t1, "ep_damage_resistance", 0.),
        talent_scale: n(bb, "talent_scale", 1.),
        linger: n(tb, "attack_speed", 0.),
    });
    Ok(())
}
pub(crate) struct Agoat2 {
    pub mode: Mode,
    pub element_ratio: f64,
    pub shots: usize,
    pub aura_element: f64,
    pub veil: f64,
    pub veil_duration: f64,
    pub mist: f64,
    pub mist_duration: f64,
    pub mist_cap: u32,
    pub ash_hp: f64,
    pub ash_cut: f64,
    pub talent_scale: f64,
    pub linger: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub accumulator: f64,
    pub healing: bool,
    pub veil_keys: Vec<usize>,
    pub veil: f64,
    pub veil_until: f64,
}

const MIST: u64 = 1517;
impl Agoat2 {
    fn guarded_heal(&self, k: &mut KitContext<'_>, target: Id, amount: f64) -> Result<(), String> {
        k.world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .agoat2_mut()?
            .healing = true;
        let result = k.heal(target, amount, health::HealOptions::default());
        k.world
            .unit_mut(k.unit)?
            .operator
            .tactical
            .agoat2_mut()?
            .healing = false;
        result.map(|_| ())
    }
}
impl Behavior for Agoat2 {
    fn payload(
        &self,
        _: &World,
        _: Id,
        payload: &mut attacks::Direct,
        _: &mut attacks::Area,
    ) -> Result<(), String> {
        payload.element_heal_ratio = self.element_ratio
            * if self.mode == Mode::Third && payload.is_skill {
                payload.heal_scale
            } else {
                1.
            };
        Ok(())
    }

    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Element { .. }) {
            20
        } else {
            0
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
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .agoat2_mut()?
                    .accumulator = 0.;
                if self.mode == Mode::Second {
                    let keys = k.keys()?;
                    let attack = k.stat(id, "atk")?;
                    for target in k.allies(&keys)? {
                        element::heal(k.world, target, attack * self.element_ratio, None)?;
                        if k.world.unit(target)?.stats.hp < k.stat(target, "maxHp")? {
                            self.guarded_heal(k, target, attack)?;
                        }
                    }
                    let until = k.world.time + self.veil_duration;
                    let state = k.world.unit_mut(id)?.operator.tactical.agoat2_mut()?;
                    state.veil_keys = keys;
                    state.veil = attack * self.veil;
                    state.veil_until = until;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id && self.mode == Mode::First => {
                let due = {
                    let s = k.world.unit_mut(id)?.operator.tactical.agoat2_mut()?;
                    s.accumulator += context.dt;
                    if s.accumulator + 1e-9 >= 1. {
                        s.accumulator -= 1.;
                        true
                    } else {
                        false
                    }
                };
                if due {
                    let keys = k.keys()?;
                    let amount = k.stat(id, "atk")? * self.aura_element;
                    for target in k.allies(&keys)? {
                        element::heal(k.world, target, amount, None)?;
                    }
                }
            }
            KitEvent::Before(context)
                if context.attacker == id
                    && k.active()?
                    && !context.targets.is_empty()
                    && context.targets.len() < self.shots =>
            {
                let base = context.targets.clone();
                context.targets = (0..self.shots)
                    .map(|index| base[index % base.len()])
                    .collect();
            }
            KitEvent::Heal { event, options }
                if !options.regen
                    && event.source == Some(id)
                    && !k.world.unit(id)?.operator.tactical.agoat2()?.healing
                    && self.mist > 0. =>
            {
                let mut buff = Buff {
                    key: Some(format!("agoat2:mist:{id}")),
                    ..Default::default()
                };
                buff.runtime.duration = self.mist_duration;
                buff.runtime.interval = 1.;
                buff.runtime.refresh = buffs::Refresh::Stack;
                buff.runtime.max_stacks = self.mist_cap;
                buff.runtime.tick_payload = k.stat(id, "atk")?;
                buff.runtime.on_tick = Some(MIST);
                k.add(event.target, buff)?;
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == MIST => {
                let buff = k.buffs.get(k.world, context.unit, context.buff)?.clone();
                if buff.runtime.source != Some(id) {
                    return Ok(());
                }
                let amount = buff.runtime.tick_payload * self.mist * buff.stacks.max(1) as f64;
                element::heal(k.world, context.unit, amount * self.element_ratio, None)?;
                if k.world.unit(context.unit)?.stats.hp < k.stat(context.unit, "maxHp")? {
                    self.guarded_heal(k, context.unit, amount)?;
                }
            }
            KitEvent::Scheduled(due) if k.up() => {
                let keys = k.keys()?;
                if due[3] {
                    let scale = if k.active()? { self.talent_scale } else { 1. };
                    for target in k.allies(&keys)? {
                        let mut buff = Buff {
                            key: Some("agoat2:ash".into()),
                            mods: [("hpPct".into(), self.ash_hp * scale)].into(),
                            ..Default::default()
                        };
                        buff.runtime.duration = 0.75;
                        buff.runtime.tick_payload = (self.ash_cut * scale).min(1.);
                        k.add(target, buff)?;
                    }
                }
                if due[2]
                    && self.linger != 0.
                    && k.allies(&keys)?.iter().any(|&target| {
                        k.world
                            .unit(target)
                            .is_ok_and(|u| u.element_buildup.iter().any(|&v| v > 0.))
                    })
                {
                    k.buff(
                        id,
                        "agoat2:linger",
                        [("aspd".into(), self.linger)].into(),
                        0.4,
                    )?;
                }
            }
            KitEvent::Element {
                phase: element::Phase::Hit,
                request,
            } => {
                if let Some(buff) = k
                    .world
                    .unit(request.damage.hit.target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| {
                        b.key.as_deref() == Some("agoat2:ash") && b.runtime.source == Some(id)
                    })
                {
                    request.damage.damage.mul *= (1. - buff.runtime.tick_payload).max(0.);
                }
            }
            KitEvent::ElementLate(request)
                if !request.damage.cancelled
                    && k.world.unit(request.damage.hit.target)?.side == health::Side::Ally =>
            {
                let now = k.world.time;
                let target = request.damage.hit.target;
                let covered = body::in_range(
                    k.world.unit(target)?,
                    &k.world.unit(id)?.operator.tactical.agoat2()?.veil_keys,
                );
                let s = k.world.unit_mut(id)?.operator.tactical.agoat2_mut()?;
                if covered && now < s.veil_until && s.veil > 1e-9 {
                    let amount = request.damage.damage.amount * request.damage.damage.mul;
                    if amount > 0. {
                        let take = s.veil.min(amount);
                        s.veil -= take;
                        if take >= amount - 1e-9 {
                            request.damage.cancelled = true;
                        } else {
                            request.damage.damage.mul *= (amount - take) / amount;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        let mut state = Self::default();
        if let Some(previous) = previous {
            state.veil_keys = previous.veil_keys.clone();
            state.veil = previous.veil.clone();
            state.veil_until = previous.veil_until.clone();
        }
        state
    }
}
