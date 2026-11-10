// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) struct Manifold {
    pub scale: f64,
    pub respawn: f64,
    steal_attack: f64,
    steal_defence: f64,
    attack_cap: f64,
    defence_cap: f64,
    split: u64,
    lifetime: f64,
    guard: f64,
    pub first_sp: f64,
    pub force: f64,
    pub taunt: f64,
}
#[derive(Default)]
pub(crate) struct State {
    pub from: Option<Id>,
    pub ranged: bool,
    pub clone: bool,
    attack: f64,
    defence: f64,
    attacks: u64,
}
pub(crate) fn build(raw: &Value, s: &mut Spec) {
    let talents = raw["talents"].as_array().cloned().unwrap_or_default();
    let find = |key: &str, fallback: f64| {
        talents
            .iter()
            .find_map(|t| t["bb"][key].as_f64())
            .unwrap_or(fallback)
    };
    let copy = talents
        .iter()
        .find(|t| t["bb"]["scale"].is_number())
        .map(|t| &t["bb"])
        .unwrap_or(&Value::Null);
    let steal = talents
        .iter()
        .find(|t| t["bb"]["steal_atk"].is_number())
        .map(|t| &t["bb"])
        .unwrap_or(&Value::Null);
    s.combat.profile.no_attack = true;
    s.extra.tactics = Tactics::Manifold(Manifold {
        scale: n(copy, "scale", 1.),
        respawn: n(copy, "interval", 25.),
        steal_attack: n(steal, "steal_atk", 0.),
        steal_defence: n(steal, "steal_def", 0.),
        attack_cap: n(steal, "steal_atk_max", 0.),
        defence_cap: n(steal, "steal_def_max", 0.),
        split: n(steal, "mlyss_wtrman_t_2[range].max_stack_cnt", 10.).max(1.) as u64,
        lifetime: n(steal, "interval", 25.),
        guard: find("damage_scale", 1.),
        first_sp: find("sp", 0.),
        force: n(&raw["nativeSkill"]["bb"], "force", 0.),
        taunt: find("taunt_level", 0.),
    });
}
impl Manifold {
    fn pick(k: &KitContext<'_>) -> Result<Option<Id>, String> {
        let owner = k
            .world
            .unit(k.unit)?
            .summoner
            .ok_or("Manifold has no summoner")?;
        let position = k.world.unit(k.unit)?.deployment.tile;
        let mut best = None;
        let mut best_score = None;
        for &id in k.host.roster.as_ref().unwrap().ids() {
            let u = k.world.unit(id)?;
            if id == owner
                || !Host::tac_up(k.world, id)
                || u.hidden
                || u.deployment.kind != crate::battle::deploy::Kind::Operator
                || u.owner != k.world.unit(owner)?.owner
            {
                continue;
            }
            let distance = (u.deployment.tile[0] - position[0])
                .abs()
                .max((u.deployment.tile[1] - position[1]).abs());
            let score = (distance, -u.stats.base_number("atk"), id);
            if best_score.is_none_or(|previous: (i32, f64, u64)| {
                score.0 < previous.0
                    || score.0 == previous.0
                        && (score.1 < previous.1 || score.1 == previous.1 && score.2 < previous.2)
            }) {
                best = Some(id);
                best_score = Some(score);
            }
        }
        Ok(best)
    }
    pub(crate) fn copy(
        k: &mut KitContext<'_>,
        source: Id,
        scale: f64,
        ranged: bool,
        clone: bool,
    ) -> Result<(), String> {
        let data = Arc::clone(&k.host.data);
        k.host
            .roster
            .as_mut()
            .unwrap()
            .copy_token(&data, k.world, k.unit, source, scale, ranged)?;
        k.remove(k.unit, "mlyss:stolen")?;
        let state = k.world.unit_mut(k.unit)?.operator.tactical.manifold_mut()?;
        *state = State {
            from: Some(source),
            ranged,
            clone,
            ..Default::default()
        };
        Ok(())
    }
}
impl Behavior for Manifold {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                if let Some(source) = Self::pick(k)? {
                    let ranged = !k.world.unit(source)?.targeting.melee_operator;
                    Self::copy(k, source, self.scale, ranged, false)?;
                    let owner = k.world.unit(id)?.summoner.unwrap();
                    let def = k.host.tac_def(owner)?;
                    if let Tactics::Mlyss(p) = &def.extra.tactics {
                        if p.copy_sp > 0. && super::mlyss::rhine(k, source)? {
                            let own = k.unit;
                            k.unit = owner;
                            let amount = p.copy_sp;
                            let result = k.skill(|skill, host, w| {
                                skill.gain(w, host, amount, "talent", false, false)
                            });
                            k.unit = own;
                            result?;
                        }
                    }
                } else {
                    k.host.with_kit_buffs(k.world, k.buffs, |host, w| {
                        skill.end(w, host, "noTarget")?;
                        let amount = skill.bank.cost(w)?;
                        skill.gain(w, host, amount, "refund", false, false)
                    })?;
                }
            }
            KitEvent::Tick if k.up() && !k.world.unit(id)?.operator.tactical.manifold()?.clone => {
                if Self::pick(k)?.is_some()
                    && skills::can_act(k.world, id)?
                    && !k.flag(id, "silence")?
                {
                    let ready = {
                        let skill = k.host.roster.as_ref().unwrap().entry(id)?.skill()?;
                        skill.bank.ready()
                            && !skill.bank.active
                            && k.world.time + 1e-9 >= skill.op_ready_at
                    };
                    if ready {
                        k.skill(|skill, host, w| {
                            skill.activate(w, host, "MLYSS_WTRMAN", false)?;
                            Ok(())
                        })?;
                    }
                }
            }
            KitEvent::Attack { context, .. }
                if context.attacker == id
                    && k.up()
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .manifold()?
                        .from
                        .is_some() =>
            {
                if !k.world.unit(id)?.operator.tactical.manifold()?.ranged {
                    let mut changed = false;
                    for &enemy in &context.targets {
                        if !Host::tac_up(k.world, enemy)
                            || k.world.unit(enemy)?.side != health::Side::Enemy
                        {
                            continue;
                        }
                        let state = k.world.unit(id)?.operator.tactical.manifold()?;
                        let attack = self
                            .steal_attack
                            .min(self.attack_cap - state.attack)
                            .max(0.);
                        let defence = self
                            .steal_defence
                            .min(self.defence_cap - state.defence)
                            .max(0.);
                        if attack <= 0. && defence <= 0. {
                            break;
                        }
                        let state = k.world.unit_mut(id)?.operator.tactical.manifold_mut()?;
                        state.attack += attack;
                        state.defence += defence;
                        let mut old_attack = 0.;
                        let mut old_defence = 0.;
                        if let Some(buff) = k
                            .world
                            .unit(enemy)?
                            .stats
                            .buffs()
                            .iter()
                            .find(|b| b.key.as_deref() == Some("mlyss:robbed"))
                        {
                            old_attack = -buff.mods.get("atkFlat").copied().unwrap_or(0.);
                            old_defence = -buff.mods.get("defFlat").copied().unwrap_or(0.);
                        }
                        k.buff(
                            enemy,
                            "mlyss:robbed",
                            [
                                ("atkFlat".into(), -old_attack - attack),
                                ("defFlat".into(), -old_defence - defence),
                            ]
                            .into(),
                            f64::INFINITY,
                        )?;
                        changed = true;
                    }
                    if changed {
                        let state = k.world.unit(id)?.operator.tactical.manifold()?;
                        let mods = [
                            ("atkFlat".into(), state.attack),
                            ("defFlat".into(), state.defence),
                        ]
                        .into();
                        k.buff(id, "mlyss:stolen", mods, f64::INFINITY)?;
                    }
                } else if !k.world.unit(id)?.operator.tactical.manifold()?.clone {
                    let state = k.world.unit_mut(id)?.operator.tactical.manifold_mut()?;
                    state.attacks = state
                        .attacks
                        .checked_add(1)
                        .ok_or("Manifold attack counter exhausted")?;
                    if state.attacks % self.split == 0 {
                        let origin = k.world.unit(id)?.deployment.tile;
                        let mut tiles = Vec::new();
                        for offset in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
                            let tile = [origin[0] + offset[0], origin[1] + offset[1]];
                            let grid = k.host.grid.as_ref().unwrap();
                            if grid.in_rect(tile)
                                && grid.can_stand(tile, true)
                                && !k
                                    .host
                                    .roster
                                    .as_ref()
                                    .unwrap()
                                    .deployment()
                                    .reserved(k.world, tile)?
                            {
                                tiles.push(tile);
                            }
                        }
                        if let Some(tile) = k.best_summon_tile(&tiles) {
                            let owner = k.world.unit(id)?.summoner.unwrap();
                            k.world
                                .unit_mut(owner)?
                                .operator
                                .tactical
                                .mlyss_mut()?
                                .clone_of = Some(id);
                            k.unit = owner;
                            let result = k.summon("token_10030_mlyss_wtrman", tile, self.lifetime);
                            k.unit = id;
                            k.world
                                .unit_mut(owner)?
                                .operator
                                .tactical
                                .mlyss_mut()?
                                .clone_of = None;
                            if let Some(clone) = result? {
                                k.host.roster.as_mut().unwrap().disable_token_skill(clone)?;
                            }
                        }
                    }
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.target == id
                && request
                    .hit
                    .source
                    .is_some_and(|a| k.world.unit(a).is_ok_and(|u| u.blocked_by() == Some(id))) =>
            {
                request.damage.mul *= self.guard
            }
            KitEvent::Landed { event, request }
                if event.source == Some(id)
                    && request.is_attack
                    && k.world.unit(id)?.operator.tactical.manifold()?.ranged
                    && Host::tac_up(k.world, event.target) =>
            {
                let owner = k.world.unit(id)?.summoner.unwrap();
                if k.world.unit(owner)?.operator.active_skill {
                    let def = k.host.tac_def(owner)?;
                    if let Tactics::Mlyss(p) = &def.extra.tactics {
                        if p.bind > 0. {
                            let own = k.unit;
                            k.unit = owner;
                            let result = k.status(event.target, "bind", p.bind, None);
                            k.unit = own;
                            result?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
