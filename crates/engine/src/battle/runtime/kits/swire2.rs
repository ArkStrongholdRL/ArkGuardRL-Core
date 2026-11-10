// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
const GRID: [[i32; 2]; 9] = [
    [2, 0],
    [1, 0],
    [0, -2],
    [0, -1],
    [0, 0],
    [0, 1],
    [0, 2],
    [-1, 0],
    [-2, 0],
];
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "merchant",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_swire2_2",
        kind: crate::battle::sp::Kind::Passive,
        sp_type: skills::SpType::None,
        rule: "DEFAULT",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let module = talent_at(raw, -1);
    s.extra.tactics = Tactics::Swire2(Swire2 {
        cap: n(bb, "sp", 3.),
        coin_cost: n(bb, "attack@sp", -1.).abs().max(1.),
        scale: n(bb, "atk_scale", 1.),
        sluggish: n(bb, "sluggish", 2.),
        interval: n(&raw["trait"]["bb"], "interval", 3.).max(0.1),
        cost: n(&raw["trait"]["bb"], "cost", -3.).abs(),
        initial: n(t, "sp", 1.),
        coins: n(t, "trait_sp", 1.),
        attack: n(t, "atk", 0.),
        stacks: n(t, "max_stack_cnt", 8.).max(1.) as u32,
        module: n(module, "atk", 0.),
        module_cap: n(module, "max_stack_cnt", 5.).max(1.) as u32,
        revive: n(t1, "cost", -5.).abs(),
        revive_mul: n(t1, "cost_multi", 2.),
        revive_hp: n(t1, "hp_ratio", 0.7),
    });
    Ok(())
}
pub(crate) struct Swire2 {
    cap: f64,
    coin_cost: f64,
    scale: f64,
    sluggish: f64,
    interval: f64,
    cost: f64,
    initial: f64,
    coins: f64,
    attack: f64,
    stacks: u32,
    module: f64,
    module_cap: u32,
    revive: f64,
    revive_mul: f64,
    revive_hp: f64,
}
#[derive(Default)]
pub(crate) struct State {
    coins: f64,
    saves: u32,
    bombs: Vec<Id>,
    hold: f64,
    next_pay: f64,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            bombs: previous.map(|s| s.bombs.clone()).unwrap_or_default(),
            next_pay: previous.map_or(0., |s| s.next_pay),
            ..Default::default()
        }
    }
}
impl Swire2 {
    fn throw(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        let id = k.unit;
        if k.up()
            && skills::can_act(k.world, id)?
            && k.world.unit(id)?.cadence.cooldown <= 1e-9
            && !k.flag(id, "disarm")?
        {
            let state = k.world.unit(id)?.operator.tactical.swire2()?;
            if state.coins >= self.coin_cost && k.world.time >= state.hold {
                let tiles = Self::tiles(k.host, k.world, id)?;
                if !tiles.is_empty() {
                    let mut key = None;
                    for enemy in k.enemies(Some(&tiles), false)? {
                        let pos = k.world.unit(enemy)?.position;
                        let candidate = (pos[0] + 0.5).floor() as usize * COLS
                            + (pos[1] + 0.5).floor() as usize;
                        if tiles.contains(&candidate) {
                            key = Some(candidate);
                            break;
                        }
                    }
                    let key = key.unwrap_or_else(|| *k.host.rng.pick(&tiles).unwrap());
                    let tile = [(key / COLS) as i32, (key % COLS) as i32];
                    if let Some(bomb) = k.summon("token_10031_swire2_gdtrap", tile, 0.)? {
                        let state = k.world.unit_mut(id)?.operator.tactical.swire2_mut()?;
                        state.coins -= self.coin_cost;
                        state.bombs.push(bomb);
                    } else {
                        let interval = k.stat(id, "interval")?;
                        k.world.unit_mut(id)?.operator.tactical.swire2_mut()?.hold =
                            k.world.time + interval;
                    }
                }
            }
        }
        Ok(())
    }
    fn tiles(host: &Host, w: &World, id: Id) -> Result<Vec<usize>, String> {
        let grid = host.grid.as_ref().ok_or("Missing Swire grid")?;
        let roster = host.roster.as_ref().ok_or("Missing Swire roster")?;
        let range = targeting::absolute_range(
            &GRID,
            w.unit(id)?.deployment.tile,
            roster.entry(id)?.direction,
            0,
        );
        let mut out = Vec::new();
        for key in range {
            let tile = [(key / COLS) as i32, (key % COLS) as i32];
            if grid.in_rect(tile)
                && grid.can_stand(tile, false)
                && grid.ground_passable(tile, true)
                && grid.tiles()[key].height == "LOW"
                && !roster.deployment().reserved(w, tile)?
            {
                out.push(key);
            }
        }
        Ok(out)
    }
    fn stack(k: &mut KitContext<'_>, key: &str, attack: f64, cap: u32) -> Result<(), String> {
        let mut b = Buff {
            key: Some(key.into()),
            mods: [("atkPct".into(), attack)].into(),
            ..Default::default()
        };
        b.runtime.refresh = buffs::Refresh::Stack;
        b.runtime.max_stacks = cap;
        k.add(k.unit, b)?;
        Ok(())
    }
}
impl Behavior for Swire2 {
    fn can_attack(&self, host: &Host, w: &World, id: Id) -> Result<bool, String> {
        let state = w.unit(id)?.operator.tactical.swire2()?;
        Ok(state.coins < self.coin_cost
            || w.time < state.hold
            || Self::tiles(host, w, id)?.is_empty())
    }
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Hit { .. } | KitEvent::Element { .. }) {
            1000
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Deploy,
                ..
            } if *unit == id => {
                let state = k.world.unit_mut(id)?.operator.tactical.swire2_mut()?;
                state.coins = self.initial.min(self.cap);
                state.saves = 0;
            }
            KitEvent::Scheduled(_)
                if k.world.time + 1e-9
                    >= k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .swire2()?
                        .next_pay
                        .max(self.interval) =>
            {
                let next = k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .swire2()?
                    .next_pay
                    .max(self.interval)
                    + self.interval;
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .swire2_mut()?
                    .next_pay = next;
                if k.up() {
                    let dp = k
                        .host
                        .roster
                        .as_ref()
                        .unwrap()
                        .dp(k.world.unit(id)?.owner.unwrap())
                        .ok_or("Missing merchant DP")?;
                    if dp >= self.cost {
                        k.dp(-self.cost)?;
                        if self.module > 0. {
                            Self::stack(k, "trait:swire2_module", self.module, self.module_cap)?;
                        }
                        if k.active()? {
                            let coins = k.world.unit(id)?.operator.tactical.swire2()?.coins;
                            k.world.unit_mut(id)?.operator.tactical.swire2_mut()?.coins =
                                (coins + self.coins).min(self.cap);
                            Self::stack(k, "talent:swire2_buyer", self.attack, self.stacks)?;
                        }
                    } else {
                        k.retreat_with_reason("merchant")?;
                    }
                }
            }
            KitEvent::Fatal(context) if context.target == id && !context.prevented => {
                let count = k.world.unit(id)?.operator.tactical.swire2()?.saves;
                let cost =
                    self.revive * powi(self.revive_mul, count as i32).map_err(str::to_string)?;
                if k.host
                    .roster
                    .as_ref()
                    .unwrap()
                    .dp(k.world.unit(id)?.owner.unwrap())
                    .is_some_and(|dp| dp + 1e-9 >= cost)
                {
                    k.dp(-cost)?;
                    k.world.unit_mut(id)?.operator.tactical.swire2_mut()?.saves = count
                        .checked_add(1)
                        .ok_or("Swire revive counter exhausted")?;
                    context.prevented = true;
                    let hp = k.stat(id, "maxHp")? * self.revive_hp;
                    k.world.unit_mut(id)?.stats.hp = hp.max(1.);
                }
            }
            KitEvent::Hit { request, .. }
                if k.world.unit(request.hit.target)?.summoner == Some(id)
                    && k.host.tac_def(request.hit.target)?.key == "token_10031_swire2_gdtrap" =>
            {
                request.cancelled = true
            }
            KitEvent::Element { request, .. }
                if k.world.unit(request.damage.hit.target)?.summoner == Some(id)
                    && k.host.tac_def(request.damage.hit.target)?.key
                        == "token_10031_swire2_gdtrap" =>
            {
                request.damage.cancelled = true
            }
            KitEvent::DamageApplied { event, .. }
                if k.world.unit(event.target)?.summoner == Some(id)
                    && k.host.tac_def(event.target)?.key == "token_10031_swire2_gdtrap" =>
            {
                let hp = k.stat(event.target, "maxHp")?;
                k.world.unit_mut(event.target)?.stats.hp = hp;
            }
            KitEvent::Tick => {
                let bombs = std::mem::take(
                    &mut k.world.unit_mut(id)?.operator.tactical.swire2_mut()?.bombs,
                );
                // The owner's throw listener precedes the existing bombs' listeners.
                // Newly spawned bombs first check contact on the following tick.
                self.throw(k)?;
                let mut kept = Vec::new();
                for bomb in bombs {
                    if !Host::tac_up(k.world, bomb) {
                        continue;
                    }
                    let tile = k.world.unit(bomb)?.deployment.tile;
                    let mut target = None;
                    for enemy in k.world.unit_ids().collect::<Vec<_>>() {
                        let unit = k.world.unit(enemy)?;
                        let on_tile = if unit.hit_area.is_some() {
                            body::on_tile(unit, tile[0], tile[1])
                        } else {
                            (unit.position[0] - f64::from(tile[0])).abs() <= 0.5
                                && (unit.position[1] - f64::from(tile[1])).abs() <= 0.5
                        };
                        if unit.side != health::Side::Enemy || !on_tile {
                            continue;
                        }
                        if targeting::can_target_enemy(
                            k.world,
                            k.host,
                            Some(bomb),
                            enemy,
                            targeting::Profile {
                                ground_only: true,
                                can_hit_fly: false,
                                ..Default::default()
                            },
                        )? {
                            target = Some(enemy);
                            break;
                        }
                    }
                    if let Some(target) = target {
                        let raw =
                            &k.host.data.table("tokens").unwrap()["token_10031_swire2_gdtrap"];
                        let owner = k.host.tac_def(id)?;
                        let switch = n(
                            &raw["variants"][&owner.key]["skill"]["bb"],
                            "duration_switch",
                            3.,
                        );
                        let times = if k.world.time - k.world.unit(bomb)?.deployment.deployed_at
                            >= switch - 1e-9
                        {
                            2
                        } else {
                            1
                        };
                        let amount = k.stat(id, "atk")? * self.scale;
                        for _ in 0..times {
                            if Host::tac_up(k.world, target) {
                                k.damage(target, amount, DamageType::Physical, true, "trap")?;
                            }
                        }
                        if Host::tac_up(k.world, target) {
                            k.status(target, "sluggish", self.sluggish, None)?;
                        }
                        k.unit = bomb;
                        let result = k.retreat();
                        k.unit = id;
                        result?;
                    } else {
                        kept.push(bomb);
                    }
                }
                let state = k.world.unit_mut(id)?.operator.tactical.swire2_mut()?;
                kept.append(&mut state.bombs);
                state.bombs = kept;
            }
            _ => {}
        }
        Ok(())
    }
}
