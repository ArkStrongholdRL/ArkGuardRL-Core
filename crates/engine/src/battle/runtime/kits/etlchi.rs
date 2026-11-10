// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "reaper",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_etlchi_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "ACTIVE_RANGE",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    let count = n(bb, "attack@max_target", 3.).floor().max(1.);
    if count > 64. {
        return Err("Candle count exceeds capacity".into());
    }
    s.extra.no_heal = true;
    s.combat.profile.all_in_range = true;
    s.extra.support.reaper = n(tb, "value", 50.);
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("aspd".into(), n(bb, "attack_speed", 0.)),
    ]
    .into();
    crate::battle::content::operators::mechanics::skill_range(raw, s)?;
    s.extra.tactics = Tactics::Etlchi(Etlchi {
        hp_scale: n(bb, "attack@max_hp_scale", 0.6),
        defence: n(bb, "attack@def_scale", 1.),
        resistance: n(bb, "attack@magic_resistance_scale", 1.),
        count: count as usize,
        steal: n(t, "attack@steal_hp", 0.),
        cap: n(t, "attack@steal_hp_max", 0.),
        dot: n(t, "magic_value", 0.),
        duration: n(t, "dot_duration", 5.),
        interval: n(t, "interval", 1.).max(0.1),
        reborn: n(t1, "hp_ratio", 0.),
        heal: n(t1, "etlchi_t_2[heal].hp_ratio", 0.),
        protection: n(t1, "damage_resistance", 0.),
        speed: n(tb, "attack_speed", 0.),
        enemies: n(tb, "cnt", 2.),
    });
    Ok(())
}
pub(crate) struct Etlchi {
    hp_scale: f64,
    defence: f64,
    resistance: f64,
    count: usize,
    steal: f64,
    cap: f64,
    dot: f64,
    duration: f64,
    interval: f64,
    reborn: f64,
    heal: f64,
    protection: f64,
    speed: f64,
    enemies: f64,
}
#[derive(Default)]
pub(crate) struct State {
    candles: BTreeMap<Id, Id>,
    stolen: f64,
    losses: BTreeMap<Id, f64>,
    reborn: bool,
}
const DOT: u64 = 1523;
impl Etlchi {
    fn extinguish(k: &mut KitContext<'_>, candle: Id) -> Result<(), String> {
        if Host::tac_up(k.world, candle) {
            settlement::Hooks::health_kill(k.host, candle, None, k.buffs, k.world)?;
        }
        Ok(())
    }
}
impl Behavior for Etlchi {
    fn reaper_eligible(&self, request: &hit::Request) -> bool {
        request.attack_type != hit::AttackType::Buff && !request.tags.iter().any(|tag| tag == "dot")
    }
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Hit { .. }) {
            -20
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
                let mut targets = k.enemies(None, false)?;
                targets.retain(|&a| {
                    !k.world.unit(a).unwrap().boss
                        && !k
                            .host
                            .spawns
                            .as_ref()
                            .unwrap()
                            .definition(a)
                            .is_ok_and(|d| d.key == "enemy_5601_entlec")
                });
                targets.sort_by(|&a, &b| {
                    k.world
                        .unit(b)
                        .unwrap()
                        .stats
                        .hp
                        .total_cmp(&k.world.unit(a).unwrap().stats.hp)
                        .then(
                            k.world
                                .unit(a)
                                .unwrap()
                                .targeting
                                .spawn_seq
                                .cmp(&k.world.unit(b).unwrap().targeting.spawn_seq),
                        )
                });
                for target in targets.into_iter().take(self.count) {
                    let position = k.world.unit(target)?.position;
                    let hp = (k.world.unit(target)?.stats.hp * self.hp_scale).max(1.);
                    let defence = k.stat(target, "def")? * self.defence;
                    let resistance = k.stat(target, "res")? * self.resistance;
                    let mass = k.world.unit(target)?.stats.base_number("massLevel");
                    let grid = k.host.grid.take().ok_or("Missing candle grid")?;
                    let mut spawns = k.host.spawns.take().ok_or("Missing candle spawns")?;
                    let mut content = k.host.content.take().ok_or("Missing candle content")?;
                    let result = spawns.spawn_candle(
                        k.world,
                        &grid,
                        &mut super::super::Spawn {
                            host: k.host,
                            buffs: k.buffs,
                            content: &mut content,
                        },
                        position,
                        hp,
                        defence,
                        resistance,
                        mass,
                    );
                    k.host.spawns = Some(spawns);
                    k.host.content = Some(content);
                    k.host.grid = Some(grid);
                    let candle = result?;
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .etlchi_mut()?
                        .candles
                        .insert(candle, target);
                    let mut buff = Buff {
                        key: Some("etlchi:candle".into()),
                        flags: ["unblockable", "noMove"].map(str::to_string).into(),
                        ..Default::default()
                    };
                    buff.runtime.source = Some(id);
                    k.add(candle, buff)?;
                    // New enemies enter targeting at the next EnemyIndex phase.
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                let candles = std::mem::take(
                    &mut k
                        .world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .etlchi_mut()?
                        .candles,
                );
                for candle in candles.keys() {
                    Self::extinguish(k, *candle)?;
                }
            }
            KitEvent::Element { request, .. }
                if request.damage.hit.source != Some(id)
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .etlchi()?
                        .candles
                        .contains_key(&request.damage.hit.target) =>
            {
                request.damage.cancelled = true
            }
            KitEvent::BeforeStatus(context)
                if context.source != Some(id)
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .etlchi()?
                        .candles
                        .contains_key(&context.target) =>
            {
                context.cancelled = true
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if k
                .world
                .unit(id)?
                .operator
                .tactical
                .etlchi()?
                .candles
                .contains_key(&request.hit.target) =>
            {
                if request.hit.source != Some(id) {
                    request.cancelled = true;
                } else if matches!(
                    request.hit.damage_type,
                    DamageType::Physical | DamageType::Arts
                ) {
                    let source = k
                        .world
                        .unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .clone();
                    let target = k
                        .world
                        .unit_mut(request.hit.target)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .clone();
                    let mut penetration = request.damage.penetration;
                    penetration.def_flat += source.n("defIgnoreFlat");
                    penetration.def_pct += source.n("defIgnorePct");
                    penetration.res_flat += source.n("resIgnoreFlat");
                    penetration.res_pct += source.n("resIgnorePct");
                    let amount = crate::battle::damage::mitigate(
                        request.damage.amount,
                        request.hit.damage_type,
                        &target,
                        penetration,
                        0.,
                    );
                    let floor = source.n("atk") * 0.35;
                    if amount < floor {
                        request.hit.damage_type = DamageType::True;
                        request.damage.amount = floor;
                        request.dodge_mode = injury::DodgeMode::Ignore;
                    }
                }
            }
            KitEvent::Before(context)
                if context.attacker != id
                    && k.world.unit(context.attacker)?.side == health::Side::Ally =>
            {
                let candles = &k.world.unit(id)?.operator.tactical.etlchi()?.candles;
                let mut out = Vec::new();
                for target in &context.targets {
                    let target = candles.get(target).copied().unwrap_or(*target);
                    if Host::tac_up(k.world, target) && !out.contains(&target) {
                        out.push(target);
                    }
                }
                context.targets = out;
            }
            KitEvent::Landed { event, request } => {
                if let Some(&original) = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .etlchi()?
                    .candles
                    .get(&event.target)
                {
                    if event.amount > 0.
                        && request.hit.damage_type != DamageType::Elemental
                        && Host::tac_up(k.world, original)
                    {
                        k.host.support_lose_hp(
                            k.world,
                            k.buffs,
                            original,
                            event.amount,
                            Some(id),
                            false,
                        )?;
                    }
                }
                if event.source == Some(id)
                    && request.is_attack
                    && request.hit.damage_type != DamageType::Elemental
                    && k.up()
                    && Host::tac_up(k.world, event.target)
                    && k.world.unit(event.target)?.side == health::Side::Enemy
                    && !k
                        .world
                        .unit(id)?
                        .operator
                        .tactical
                        .etlchi()?
                        .candles
                        .contains_key(&event.target)
                {
                    let target = event.target;
                    if self.dot > 0. {
                        let mut buff = Buff {
                            key: Some(format!("etlchi:dot:{id}")),
                            ..Default::default()
                        };
                        buff.runtime.duration = self.duration;
                        buff.runtime.interval = self.interval;
                        buff.runtime.on_tick = Some(DOT);
                        buff.runtime.tick_payload = self.dot;
                        buff.runtime.refresh = buffs::Refresh::Extend;
                        k.add(target, buff)?;
                    }
                    let stolen = k.world.unit(id)?.operator.tactical.etlchi()?.stolen;
                    let amount = self
                        .steal
                        .min(self.cap - stolen)
                        .min((k.stat(target, "maxHp")? - 1.).floor());
                    if amount > 0. && k.world.unit(target)?.pool.is_none() {
                        let hp = k.world.unit(target)?.stats.hp;
                        let state = k.world.unit_mut(id)?.operator.tactical.etlchi_mut()?;
                        state.stolen += amount;
                        let total = state.losses.entry(target).or_default();
                        *total += amount;
                        let total = *total;
                        let gain = state.stolen;
                        k.buff(
                            target,
                            &format!("etlchi:steal:{id}"),
                            [("hpFlat".into(), -total)].into(),
                            f64::INFINITY,
                        )?;
                        let maximum = k.stat(target, "maxHp")?;
                        k.world.unit_mut(target)?.stats.hp = hp.min(maximum);
                        k.buff(
                            id,
                            "etlchi:gain",
                            [("hpFlat".into(), gain)].into(),
                            f64::INFINITY,
                        )?;
                    }
                }
                if event.target == id
                    && k.up()
                    && !k.world.unit(id)?.operator.tactical.etlchi()?.reborn
                    && k.world.unit(id)?.stats.hp > 0.
                    && k.world.unit(id)?.stats.hp / k.stat(id, "maxHp")? < self.reborn
                {
                    k.world.unit_mut(id)?.operator.tactical.etlchi_mut()?.reborn = true;
                    let amount = k.stat(id, "maxHp")? * self.heal;
                    k.heal(
                        id,
                        amount,
                        health::HealOptions {
                            self_heal: true,
                            ..Default::default()
                        },
                    )?;
                    if self.protection > 0. {
                        k.buff(
                            id,
                            "etlchi:reborn",
                            [("physTakenMul".into(), 1. - self.protection)].into(),
                            f64::INFINITY,
                        )?;
                    }
                }
            }
            KitEvent::Buff {
                context,
                event: buffs::Event::Tick,
                token,
            } if *token == DOT => {
                let b = k.buffs.get(k.world, context.unit, context.buff)?;
                if b.runtime.source == Some(id) {
                    let amount = b.runtime.tick_payload;
                    k.damage(context.unit, amount, DamageType::Arts, false, "dot")?;
                }
            }
            KitEvent::Death { unit, .. } => {
                let candles = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .etlchi()?
                    .candles
                    .iter()
                    .filter_map(|(&c, &original)| if original == *unit { Some(c) } else { None })
                    .collect::<Vec<_>>();
                for candle in candles {
                    Self::extinguish(k, candle)?;
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                if self.speed != 0. && k.enemies(None, false)?.len() as f64 >= self.enemies {
                    k.buff(
                        id,
                        "trait:reaperAspd",
                        [("aspd".into(), self.speed)].into(),
                        0.4,
                    )?;
                }
                let valid = k.world.unit_ids().collect::<BTreeSet<_>>();
                let state = k.world.unit_mut(id)?.operator.tactical.etlchi_mut()?;
                state.losses.retain(|e, _| valid.contains(e));
                state.candles.retain(|c, _| valid.contains(c));
            }
            _ => {}
        }
        Ok(())
    }
}
