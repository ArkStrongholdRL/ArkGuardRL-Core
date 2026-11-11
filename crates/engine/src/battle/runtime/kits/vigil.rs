// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "PIONEER",
    branch: "tactician",
    ranged: true,
    can_hit_fly: true,
    physical: true,
    projectile: "arrow",
    projectile_speed: 14.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_vigil_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "SP_FULL",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    s.combat.skill_hits = 3;
    s.extra.tactics = Tactics::Vigil(Vigil {
        interval: n(bb, "interval", 1.5).max(0.1),
        step: n(bb, "cost", 1.).max(0.01),
        cap: n(bb, "value", 0.),
        bonus: n(bb, "attack@vigil_s_3.atk_scale", 0.),
        penetration: n(talent_at(raw, 1), "def_penetrate_fixed", 0.),
        trait_scale: n(&raw["trait"]["bb"], "atk_scale", 1.5),
    });
    Ok(())
}
pub(crate) struct Vigil {
    interval: f64,
    step: f64,
    cap: f64,
    bonus: f64,
    penetration: f64,
    trait_scale: f64,
}
#[derive(Default)]
pub(crate) struct State {
    wolf: Option<Id>,
    accumulator: f64,
    paid: f64,
    marked: BTreeSet<Id>,
}
impl Behavior for Vigil {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                if let Some(tile) = k.tactical_point()? {
                    if let Some(wolf) = k.summon("token_10028_vigil_wolf", tile, 0.)? {
                        k.world.unit_mut(id)?.operator.tactical.vigil_mut()?.wolf = Some(wolf);
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.vigil_mut()?;
                state.accumulator = 0.;
                state.paid = 0.;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                context,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.vigil_mut()?;
                state.accumulator += context.dt;
                let mut amount = 0.;
                while state.accumulator >= self.interval - 1e-9
                    && state.paid + self.step <= self.cap + 1e-9
                {
                    state.accumulator -= self.interval;
                    state.paid += self.step;
                    amount += self.step;
                }
                if amount > 0. {
                    k.dp(amount)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                context,
                ..
            } if *unit == id && context.reason == "duration" => {
                let rest = self.cap - k.world.unit(id)?.operator.tactical.vigil()?.paid;
                if rest > 1e-9 {
                    k.dp(rest)?;
                    k.world.unit_mut(id)?.operator.tactical.vigil_mut()?.paid += rest;
                }
            }
            KitEvent::Death { unit, .. } if *unit == id => {
                if let Some(wolf) = k.world.unit(id)?.operator.tactical.vigil()?.wolf {
                    let owner = k.unit;
                    k.unit = wolf;
                    let result = k.retreat();
                    k.unit = owner;
                    result?;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.is_attack => {
                if let Some(wolf) = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .vigil()?
                    .wolf
                    .filter(|&wolf| Host::tac_up(k.world, wolf))
                {
                    if k.world.unit(request.hit.target)?.blocked_by() == Some(wolf) {
                        if request.hit.source == Some(id) {
                            request.damage.mul *= self.trait_scale;
                        }
                        if request.hit.source == Some(id) || request.hit.source == Some(wolf) {
                            request.damage.penetration.def_flat += self.penetration;
                        }
                    }
                }
            }
            KitEvent::Landed { event, request }
                if request.is_attack && self.bonus > 0. && k.active()? =>
            {
                if let Some(wolf) = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .vigil()?
                    .wolf
                    .filter(|&wolf| Host::tac_up(k.world, wolf))
                {
                    if (event.source == Some(id) || event.source == Some(wolf))
                        && k.world.unit(event.target)?.side == health::Side::Enemy
                        && Host::tac_up(k.world, event.target)
                        && k.world.unit(event.target)?.blocked_by() == Some(wolf)
                    {
                        let amount = k.stat(id, "atk")? * self.bonus;
                        let owner = k.unit;
                        k.unit = event.source.unwrap();
                        let result =
                            k.damage(event.target, amount, DamageType::Arts, true, "vigilBonus");
                        k.unit = owner;
                        result?;
                    }
                }
            }
            KitEvent::Scheduled(due) if due[0] => {
                let mut marked = BTreeSet::new();
                if let Some(wolf) = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .vigil()?
                    .wolf
                    .filter(|&wolf| Host::tac_up(k.world, wolf) && k.up())
                {
                    let def = k.host.tac_def(wolf)?;
                    if let Tactics::Wolf(p) = &def.extra.tactics {
                        if p.taunt != 0. {
                            let own = k.unit;
                            k.unit = wolf;
                            for enemy in k.world.unit(wolf)?.blocking().to_vec() {
                                if Host::tac_up(k.world, enemy) {
                                    k.buff(
                                        enemy,
                                        "token:pack_mark",
                                        [("taunt".into(), p.taunt)].into(),
                                        0.25,
                                    )?;
                                    marked.insert(enemy);
                                }
                            }
                            k.unit = own;
                        }
                    }
                }
                let previous = std::mem::replace(
                    &mut k.world.unit_mut(id)?.operator.tactical.vigil_mut()?.marked,
                    marked.clone(),
                );
                for enemy in previous.difference(&marked) {
                    if k.world.unit(*enemy).is_ok() {
                        k.remove(*enemy, "token:pack_mark")?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
