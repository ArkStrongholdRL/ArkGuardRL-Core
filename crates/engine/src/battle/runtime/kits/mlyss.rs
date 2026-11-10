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
        id: "skchr_mlyss_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "MLYSS_WTRMAN",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 1);
    let module = talent_at(raw, -1);
    s.modifiers = [("atkPct".into(), n(bb, "atk", 0.))].into();
    s.extra.tactics = Tactics::Mlyss(Mlyss {
        cost: n(bb, "cost", 0.),
        attack: n(bb, "atk", 0.),
        speed: n(bb, "attack_speed", 0.),
        interval: suffix_number(bb, "interval", 2.).max(0.2),
        bind: suffix_number(bb, "duration", 1.5),
        copy_sp: n(module, "sp", 0.),
        other_sp: n(module, "sp_other", 0.),
        cost_cut: n(t, "cost", 0.),
        first_cut: n(t, "runtime_cost", 0.),
        trait_scale: n(&raw["trait"]["bb"], "atk_scale", 1.5),
    });
    Ok(())
}
pub(crate) struct Mlyss {
    cost: f64,
    attack: f64,
    speed: f64,
    interval: f64,
    pub bind: f64,
    pub copy_sp: f64,
    other_sp: f64,
    cost_cut: f64,
    first_cut: f64,
    trait_scale: f64,
}
#[derive(Default)]
pub(crate) struct State {
    first: bool,
    pub(super) clone_of: Option<Id>,
    costs_applied: bool,
    skill_left: f64,
    pulse: Option<f64>,
    pending: Option<([i32; 2], f64, u32)>,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            first: previous.is_some_and(|s| s.first),
            costs_applied: previous.is_some_and(|s| s.costs_applied),
            ..Default::default()
        }
    }
}
pub(crate) fn rhine(k: &KitContext<'_>, id: Id) -> Result<bool, String> {
    let character = &k.host.tac_def(id)?.char_id;
    Ok([
        "char_108_silent",
        "char_128_plosis",
        "char_202_demkni",
        "char_249_mlyss",
        "char_1047_halo2",
        "char_135_halo",
    ]
    .contains(&character.as_str()))
}
impl Mlyss {
    fn mine(k: &KitContext<'_>) -> Result<Vec<Id>, String> {
        Ok(k.host
            .roster
            .as_ref()
            .unwrap()
            .ids()
            .iter()
            .copied()
            .filter(|&a| {
                k.world
                    .unit(a)
                    .is_ok_and(|u| u.summoner == Some(k.unit) && Host::tac_up(k.world, a))
                    && k.world
                        .unit(a)
                        .unwrap()
                        .operator
                        .tactical
                        .manifold()
                        .is_ok()
            })
            .collect())
    }
    fn pulse(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        for token in Self::mine(k)? {
            if !k
                .world
                .unit(token)?
                .operator
                .tactical
                .manifold()?
                .from
                .is_some()
                || k.world.unit(token)?.operator.tactical.manifold()?.ranged
            {
                continue;
            }
            let def = k.host.tac_def(token)?;
            let force = if let Tactics::Manifold(p) = &def.extra.tactics {
                p.force
            } else {
                0.
            };
            let owner = k.unit;
            k.unit = token;
            let result: Result<(), String> = (|| {
                let keys = k.range(&[
                    [-1, -1],
                    [-1, 0],
                    [-1, 1],
                    [0, -1],
                    [0, 1],
                    [1, -1],
                    [1, 0],
                    [1, 1],
                ])?;
                for enemy in k.enemies(Some(&keys), true)? {
                    k.pull(enemy, force, None)?;
                }
                for enemy in k.world.unit(token)?.blocking().to_vec() {
                    if Host::tac_up(k.world, enemy) {
                        k.unit = owner;
                        k.status(enemy, "stun", self.interval + 0.1, None)?;
                        k.unit = token;
                    }
                }
                Ok(())
            })();
            k.unit = owner;
            result?;
        }
        Ok(())
    }
}
impl Behavior for Mlyss {
    fn condition(
        &self,
        host: &Host,
        w: &mut World,
        id: Id,
        condition: skills::Condition,
    ) -> Result<Option<bool>, String> {
        if !matches!(
            condition,
            skills::Condition::Default | skills::Condition::ExtraWithoutAttack
        ) {
            return Ok(None);
        }
        for &a in host.roster.as_ref().unwrap().ids() {
            if w.unit(a)?.summoner == Some(id)
                && Host::tac_up(w, a)
                && w.unit(a)?
                    .operator
                    .tactical
                    .manifold()
                    .is_ok_and(|s| s.from.is_some())
            {
                let keys = host.roster.as_ref().unwrap().range(w, a)?;
                for enemy in host.spawns.as_ref().unwrap().live() {
                    let e = w.unit(*enemy)?;
                    if e.stats.alive && !e.hidden && body::in_range(e, &keys) {
                        return Ok(Some(true));
                    }
                }
            }
        }
        Ok((condition == skills::Condition::ExtraWithoutAttack).then_some(false))
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                if Self::mine(k)?.iter().all(|&a| {
                    k.world
                        .unit(a)
                        .unwrap()
                        .operator
                        .tactical
                        .manifold()
                        .unwrap()
                        .clone
                }) {
                    let previous = k
                        .host
                        .roster
                        .as_ref()
                        .unwrap()
                        .ids()
                        .iter()
                        .copied()
                        .filter(|&a| k.world.unit(a).is_ok_and(|u| u.summoner == Some(id)))
                        .filter(|&a| {
                            k.world
                                .unit(a)
                                .unwrap()
                                .operator
                                .tactical
                                .manifold()
                                .is_ok_and(|s| !s.clone)
                        })
                        .max_by_key(|&a| k.world.unit(a).unwrap().deployment.deploy_seq);
                    let mut tile = previous
                        .map(|a| k.world.unit(a).unwrap().deployment.home)
                        .filter(|&tile| {
                            !k.host
                                .roster
                                .as_ref()
                                .unwrap()
                                .deployment()
                                .reserved(k.world, tile)
                                .unwrap_or(true)
                        });
                    if tile.is_none() {
                        tile = k.tactical_point()?;
                    }
                    if let Some(tile) = tile {
                        k.summon("token_10030_mlyss_wtrman", tile, 0.)?;
                    }
                }
            }
            KitEvent::BattleStart => {
                if !k.world.unit(id)?.operator.tactical.mlyss()?.costs_applied {
                    let mut targets = k
                        .host
                        .roster
                        .as_ref()
                        .unwrap()
                        .ids()
                        .iter()
                        .copied()
                        .filter(|&a| {
                            k.world.unit(a).unwrap().deployment.kind
                                == crate::battle::deploy::Kind::Operator
                                && k.world.unit(a).unwrap().owner == k.world.unit(id).unwrap().owner
                        })
                        .collect::<Vec<_>>();
                    targets.retain(|&a| rhine(k, a).unwrap_or(false));
                    targets.sort_by_key(|&a| k.world.unit(a).unwrap().deployment.deploy_seq);
                    for (index, target) in targets.into_iter().enumerate() {
                        let delta = self.cost_cut + if index == 0 { self.first_cut } else { 0. };
                        let cost = k.world.unit(target)?.stats.base_number("cost");
                        k.world.unit_mut(target)?.stats.edit_base(|b| {
                            b.0.insert("cost".into(), (cost + delta).max(0.));
                        });
                    }
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .mlyss_mut()?
                        .costs_applied = true;
                }
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::Deploy,
                skill,
            } if k.world.unit(*unit)?.summoner == Some(id) => {
                if let Some(source) = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .mlyss_mut()?
                    .clone_of
                    .take()
                {
                    let owner = k.unit;
                    k.unit = *unit;
                    let result = super::manifold::Manifold::copy(k, source, 1., true, true);
                    k.unit = owner;
                    result?;
                    skill.bank.no_skill = true;
                    return Ok(());
                }
                let def = k.host.tac_def(*unit)?;
                if let Tactics::Manifold(p) = &def.extra.tactics {
                    if !k.world.unit(id)?.operator.tactical.mlyss()?.first {
                        let amount = p.first_sp;
                        if amount > 0. {
                            k.host.with_kit_buffs(k.world, k.buffs, |host, w| {
                                skill.gain(w, host, amount, "talent", false, false)
                            })?;
                        }
                        k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.first = true;
                    }
                }
                k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pending = None;
                if k.active()? {
                    let duration = k.world.unit(id)?.operator.tactical.mlyss()?.skill_left;
                    k.buff(
                        *unit,
                        "mlyss:s3",
                        [("atkPct".into(), self.attack), ("aspd".into(), self.speed)].into(),
                        duration,
                    )?;
                }
            }
            KitEvent::SkillObserved {
                unit,
                event: skills::Event::Deploy,
                skill,
            } if self.other_sp > 0.
                && k.world.unit(*unit)?.deployment.kind
                    == crate::battle::deploy::Kind::Operator
                && k.world.unit(*unit)?.operator.deployments > 1
                && rhine(k, *unit)? =>
            {
                if Self::mine(k)?.iter().any(|&a| {
                    k.world
                        .unit(a)
                        .unwrap()
                        .operator
                        .tactical
                        .manifold()
                        .is_ok_and(|s| s.from == Some(*unit))
                }) {
                    let amount = self.other_sp;
                    k.host.with_kit_buffs(k.world, k.buffs, |host, w| {
                        skill.gain(w, host, amount, "talent", false, false)
                    })?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .mlyss_mut()?
                    .skill_left = skill.time_left;
                k.dp(self.cost)?;
                let tokens = Self::mine(k)?;
                let ranged = tokens.iter().any(|&a| {
                    k.world
                        .unit(a)
                        .unwrap()
                        .operator
                        .tactical
                        .manifold()
                        .unwrap()
                        .ranged
                });
                for token in tokens {
                    k.buff(
                        token,
                        "mlyss:s3",
                        [("atkPct".into(), self.attack), ("aspd".into(), self.speed)].into(),
                        skill.time_left,
                    )?;
                    if ranged {
                        let maximum = k.stat(token, "maxHp")?;
                        k.world.unit_mut(token)?.stats.hp = maximum;
                    }
                }
                if ranged {
                    if let Some((tile, _, _)) = k
                        .world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .mlyss_mut()?
                        .pending
                        .take()
                    {
                        k.summon("token_10030_mlyss_wtrman", tile, 0.)?;
                    }
                }
                self.pulse(k)?;
                let next = k.world.time + self.interval;
                k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pulse = Some(next);
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Tick,
                skill,
                ..
            } if *unit == id => {
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .mlyss_mut()?
                    .skill_left = skill.time_left
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?;
                state.pulse = None;
                state.skill_left = 0.;
            }
            KitEvent::Tick if k.up() => {
                if k.active()?
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .mlyss()?
                        .pulse
                        .is_some_and(|at| k.world.time + 1e-9 >= at)
                {
                    self.pulse(k)?;
                    k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pulse =
                        Some(k.world.time + self.interval);
                }
                if let Some((tile, at, tries)) =
                    k.world.unit(id)?.operator.tactical.mlyss()?.pending
                {
                    if k.world.time + 1e-9 >= at {
                        if tries <= 120 {
                            if k.summon("token_10030_mlyss_wtrman", tile, 0.)?.is_none() {
                                let at = k.world.time + 1.;
                                k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pending =
                                    Some((tile, at, tries + 1));
                            }
                        } else {
                            k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pending = None;
                        }
                    }
                }
            }
            KitEvent::Death { unit, .. } if *unit == id => {
                k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pending = None;
            }

            KitEvent::Death { unit, .. }
                if k.world.unit(*unit)?.summoner == Some(id)
                    && k.world.unit(*unit)?.deployment.remove_reason.as_deref()
                        == Some("killed")
                    && k.world
                        .unit(*unit)?
                        .operator
                        .tactical
                        .manifold()
                        .is_ok_and(|s| !s.clone) =>
            {
                if Self::mine(k)?.iter().all(|&a| {
                    k.world
                        .unit(a)
                        .unwrap()
                        .operator
                        .tactical
                        .manifold()
                        .unwrap()
                        .clone
                }) {
                    let def = k.host.tac_def(*unit)?;
                    let delay = if let Tactics::Manifold(p) = &def.extra.tactics {
                        p.respawn
                    } else {
                        25.
                    };
                    let tile = k.world.unit(*unit)?.deployment.home;
                    let at = k.world.time + delay;
                    k.world.unit_mut(id)?.operator.tactical.mlyss_mut()?.pending =
                        Some((tile, at, 0));
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id) && request.is_attack => {
                if let Some(blocker) = k.world.unit(request.hit.target)?.blocked_by() {
                    if k.world.unit(blocker)?.summoner == Some(id) {
                        // Tactician dmgMul is applied to the attack amount before mitigation.
                        request.damage.amount *= self.trait_scale;
                    }
                }
            }
            KitEvent::Scheduled(due) if due[2] => {
                for token in Self::mine(k)? {
                    let def = k.host.tac_def(token)?;
                    if let Tactics::Manifold(p) = &def.extra.tactics {
                        if p.taunt > 0. {
                            let own = k.unit;
                            k.unit = token;
                            for enemy in k.world.unit(token)?.blocking().to_vec() {
                                if Host::tac_up(k.world, enemy) {
                                    k.buff(
                                        enemy,
                                        "mlyss:exposed",
                                        [("taunt".into(), p.taunt)].into(),
                                        0.4,
                                    )?;
                                }
                            }
                            k.unit = own;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
