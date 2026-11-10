// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::element;
use crate::battle::{skills::SpType, sp::Kind};
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "primcaster",
    ranged: true,
    can_hit_fly: true,
    physical: false,
    projectile: "bolt",
    projectile_speed: 11.0,
    required_range: None,
    skills: &[
        Skill {
            id: "skchr_nymph_2",
            kind: Kind::Charges,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: true,
        },
        Skill {
            id: "skchr_nymph_1",
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        },
        Skill {
            id: "skchr_nymph_3",
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
        "skchr_nymph_1" => Mode::First,
        "skchr_nymph_2" => Mode::Second,
        "skchr_nymph_3" => Mode::Third,
        _ => return Err("Unknown selected skill".into()),
    };
    if mode == Mode::Second {
        s.combat.skill_scale = n(bb, "atk_scale", 1.);
        s.combat.skill_area.splash = Some(Splash {
            radius: n(bb, "projectile_range", 1.5),
            scale: 1.,
            hits_fly: true,
            fortress_melee: false,
        });
    } else {
        s.modifiers = [
            ("atkPct".into(), n(bb, "atk", 0.)),
            ("aspd".into(), n(bb, "attack_speed", 0.)),
        ]
        .into();
    }
    if mode == Mode::Third {
        skill_range(raw, s)?;
        let mut p = s.combat.profile;
        p.max_targets = n(bb, "attack@max_target", 2.).max(1.);
        s.combat.skill_profile = Some(p);
        s.combat.skill_scale = n(bb, "attack@split_atk_scale", 1.);
    }
    s.extra.tactics = Tactics::Nymph(Nymph {
        mode: mode,
        ep: n(bb, "ep_damage_ratio", n(bb, "attack@ep_damage_ratio", 0.)),
        soul: n(t, "element_atk_scale", 0.),
        skill_soul: n(bb, "element_atk_scale", 0.),
        key_atk: n(t1, "atk", 0.),
        key_cap: n(t1, "max_stack_cnt", 10.).max(1.) as u32,
        lash: n(bb, "attack@extra_ep_damage_scale", 0.),
        fear: n(bb, "fear", 0.),
        burst_mul: n(tb, "damage_scale", 1.),
        sp: n(tm, "sp_recovery_per_sec", 0.),
    });
    Ok(())
}
pub(crate) struct Nymph {
    pub mode: Mode,
    pub ep: f64,
    pub soul: f64,
    pub skill_soul: f64,
    pub key_atk: f64,
    pub key_cap: u32,
    pub lash: f64,
    pub fear: f64,
    pub burst_mul: f64,
    pub sp: f64,
}

pub(in crate::battle::runtime) const SOUL: u64 = 1510;
impl Behavior for Nymph {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Hit { .. }) {
            -10
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id) => {
                if k.flag(request.hit.target, "burstLock")? {
                    request.damage.mul *= self.burst_mul;
                }
                if self.mode == Mode::Third
                    && k.active()?
                    && request.is_attack
                    && k.has(request.hit.target, "apoptosisBurst")
                    && matches!(
                        request.hit.damage_type,
                        DamageType::Physical | DamageType::Arts
                    )
                {
                    request.hit.damage_type = DamageType::Elemental;
                    request.element = Some(element::Element::Apoptosis);
                    request.dodge_mode = injury::DodgeMode::Ignore;
                    request.tags.push("nymphBreak".into());
                }
            }
            KitEvent::Impact(impact)
                if impact.attacker == id
                    && impact.is_skill
                    && self.mode == Mode::Second
                    && impact.kind == attacks::VictimKind::Main =>
            {
                if let Some(target) = impact.target.filter(|&t| Host::tac_up(k.world, t)) {
                    k.status(target, "fear", self.fear, None)?;
                }
            }
            KitEvent::Landed {
                event: landed,
                request,
            } if landed.source == Some(id)
                && request.is_attack
                && k.world.unit(landed.target)?.side == health::Side::Enemy
                && Host::tac_up(k.world, landed.target) =>
            {
                let target = landed.target;
                if (request.is_skill || self.mode == Mode::First && k.active()?)
                    && self.ep > 0.
                    && landed.amount > 0.
                    && !k.flag(target, "hitCount")?
                {
                    k.elemental(target, landed.amount * self.ep, element::Element::Apoptosis)?;
                }
                let scale = if request.is_skill {
                    self.skill_soul.max(self.soul)
                } else {
                    self.soul
                };
                if let Some(duration) = k
                    .world
                    .unit(target)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("apoptosisBurst"))
                    .map(|b| b.runtime.time_left)
                {
                    let key = format!("nymph:soul:{id}");
                    if let Some(buff) = k
                        .world
                        .unit(target)?
                        .stats
                        .buffs()
                        .iter()
                        .find(|b| b.key.as_deref() == Some(&key))
                        .map(|b| b.runtime.id())
                    {
                        k.buffs.edit_timer(k.world, target, buff, |b| {
                            b.runtime.tick_payload = b.runtime.tick_payload.max(scale)
                        })?;
                    } else if scale > 0. {
                        let mut buff = Buff {
                            key: Some(key),
                            ..Default::default()
                        };
                        buff.runtime.duration = duration.max(0.1);
                        buff.runtime.interval = 1.;
                        buff.runtime.tick_payload = scale;
                        buff.runtime.on_tick = Some(SOUL);
                        k.add(target, buff)?;
                    }
                    if self.mode == Mode::First
                        && k.active()?
                        && self.lash > 0.
                        && request.hit.damage_type != DamageType::Elemental
                    {
                        let amount = k.stat(id, "atk")? * self.lash;
                        k.element_damage(target, amount, element::Element::Apoptosis, "nymphLash")?;
                    }
                }
            }
            KitEvent::Element {
                phase: element::Phase::Burst,
                request,
            } if request.damage.element == Some(element::Element::Apoptosis) && k.up() => {
                let keys = k.keys()?;
                if k.world.unit(request.damage.hit.target)?.side == health::Side::Enemy
                    && body::in_range(k.world.unit(request.damage.hit.target)?, &keys)
                {
                    let mut buff = Buff {
                        key: Some("nymph:key".into()),
                        mods: [("atkPct".into(), self.key_atk)].into(),
                        ..Default::default()
                    };
                    buff.runtime.refresh = buffs::Refresh::Stack;
                    buff.runtime.max_stacks = self.key_cap;
                    k.add(id, buff)?;
                }
            }
            KitEvent::Scheduled(due) if due[2] && self.sp > 0. && k.up() => {
                let keys = k.keys()?;
                let foes = k.host.spawns.as_ref().unwrap().live().to_vec();
                let mut burst = false;
                for target in foes {
                    if Host::tac_up(k.world, target)
                        && body::in_range(k.world.unit(target)?, &keys)
                        && k.flag(target, "burstLock")?
                    {
                        burst = true;
                        break;
                    }
                }
                if burst {
                    k.buff(
                        id,
                        "nymph:module",
                        [("spRecoveryFlat".into(), self.sp)].into(),
                        0.5,
                    )?;
                }
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == SOUL => {
                let buff = k.buffs.get(k.world, context.unit, context.buff)?.clone();
                if buff.runtime.source != Some(id) {
                    return Ok(());
                }
                if !k.has(context.unit, "apoptosisBurst") {
                    k.buffs
                        .remove_id(k.world, k.host, context.unit, context.buff)?;
                } else {
                    let amount = k.stat(id, "atk")? * buff.runtime.tick_payload;
                    k.element_damage(
                        context.unit,
                        amount,
                        element::Element::Apoptosis,
                        "nymphSoul",
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
