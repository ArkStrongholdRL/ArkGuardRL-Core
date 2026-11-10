// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
use crate::battle::status;
pub(crate) const KIT: Kit = Kit {
    position: "RANGED",
    profession: "CASTER",
    branch: "phalanx",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_sbell2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "CUSTOM_RANGE",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    if raw["skill"]["skillId"] != "skchr_sbell2_3" {
        return Err("Unknown selected skill".into());
    }
    let bb = &raw["skill"]["bb"];
    let tb = &raw["trait"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tm = talent_at(raw, -1);
    s.combat.profile.only_while_skill = true;
    s.combat.profile.all_in_range = true;
    s.combat.profile.range_aoe = true;
    s.combat.profile.selection.can_hit_fly = true;
    s.combat.area.selection = s.combat.profile.selection;
    s.combat.skill_area = s.combat.area;
    s.combat.projectile = None;
    s.combat.skill_scale = n(bb, "attack@atk_scale_s3", 1.);
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("aspd".into(), n(bb, "attack_speed", 0.)),
        (
            "resIgnoreFlat".into(),
            n(bb, "magic_resist_penetrate_fixed", 0.),
        ),
    ]
    .into();
    skill_range(raw, s)?;
    s.extra.tactics = Tactics::Sbell2(Sbell2 {
        guard_def: n(tb, "def", 2.),
        guard_res: n(tb, "magic_resistance", 20.),
        interval: n(t, "interval", 5.5).max(0.1),
        skill_interval: n(bb, "interval", n(t, "interval", 5.5)).max(0.1),
        layers: n(t, "max_cast_cnt", 5.).floor().max(1.) as u32,
        first: n(t, "first_snow", 0.) > 0.,
        slow: n(t, "move_speed", 0.),
        snow_damage: n(t, "talent_magic_scale", 0.),
        cold: n(t1, "cold", 0.),
        freeze: n(t1, "freeze", 0.),
        enemy_freeze: n(t1, "c2e_freeze", 0.),
        restore: n(t1, "hp_ratio", 1.),
        lure: n(bb, "attract_time", 0.),
        module_damage: n(tm, "damage_scale", 0.),
        module_cap: n(tm, "max_valid_stack_cnt", 5.).floor().max(1.) as usize,
    });
    Ok(())
}
pub(crate) struct Sbell2 {
    guard_def: f64,
    guard_res: f64,
    interval: f64,
    skill_interval: f64,
    layers: u32,
    first: bool,
    slow: f64,
    snow_damage: f64,
    cold: f64,
    freeze: f64,
    enemy_freeze: f64,
    restore: f64,
    lure: f64,
    module_damage: f64,
    module_cap: usize,
}
#[derive(Default)]
pub(crate) struct State {
    snow: BTreeMap<usize, u32>,
    last: BTreeMap<Id, usize>,
    accumulator: f64,
    blessed: bool,
}
impl Sbell2 {
    fn lay(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        for key in k.keys()? {
            let tile = [(key / COLS) as i32, (key % COLS) as i32];
            if k.host.grid.as_ref().unwrap().ground_passable(tile, true) {
                let layers = k
                    .world
                    .unit_mut(k.unit)?
                    .operator
                    .tactical
                    .sbell2_mut()?
                    .snow
                    .entry(key)
                    .or_default();
                *layers = (*layers + 1).min(self.layers);
            }
        }
        Ok(())
    }
    fn guard(&self, k: &mut KitContext<'_>) -> Result<(), String> {
        k.buff(
            k.unit,
            "trait:phalanxGuard",
            [
                ("defPct".into(), self.guard_def),
                ("resFlat".into(), self.guard_res),
            ]
            .into(),
            f64::INFINITY,
        )
    }
}
impl Behavior for Sbell2 {
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            -50
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
                self.guard(k)?;
                if self.first {
                    self.lay(k)?;
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::SkillStart,
                ..
            } if *unit == id => {
                k.remove(id, "trait:phalanxGuard")?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::SkillEnd,
                ..
            } if *unit == id && k.up() => {
                self.guard(k)?;
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id && self.lure > 0. => {
                let home = k.world.unit(id)?.deployment.tile;
                for enemy in k.enemies(None, true)? {
                    if targeting::flying(k.world, enemy)? || k.world.unit(enemy)?.boss {
                        continue;
                    }
                    let at = k.world.unit(enemy)?.position;
                    let mut goal = None;
                    let mut nearest = f64::INFINITY;
                    for dr in -1..=1 {
                        for dc in -1..=1 {
                            if dr == 0 && dc == 0 {
                                continue;
                            }
                            let tile = [home[0] + dr, home[1] + dc];
                            if !k.host.grid.as_ref().unwrap().ground_passable(tile, false) {
                                continue;
                            }
                            let distance = crate::deterministic::math::hypot(
                                at[0] - tile[0] as f64,
                                at[1] - tile[1] as f64,
                            );
                            if distance < nearest - 1e-9 {
                                nearest = distance;
                                goal = Some(tile.map(|v| v as f64));
                            }
                        }
                    }
                    if let Some(point) = goal {
                        status::apply(
                            k.world,
                            k.host,
                            k.buffs,
                            Rect::FULL,
                            enemy,
                            "attract",
                            status::Options {
                                duration: self.lure,
                                source: Some(id),
                                point: Some(point),
                                ..Default::default()
                            },
                        )?;
                    }
                }
            }
            KitEvent::Tick if k.up() => {
                let active = k.active()?;
                let state = k.world.unit_mut(id)?.operator.tactical.sbell2_mut()?;
                state.accumulator += DT;
                if state.accumulator + 1e-9
                    >= if active {
                        self.skill_interval
                    } else {
                        self.interval
                    }
                {
                    state.accumulator = 0.;
                    self.lay(k)?;
                }
                if k.world
                    .unit(id)?
                    .operator
                    .tactical
                    .sbell2()?
                    .snow
                    .is_empty()
                {
                    return Ok(());
                }
                for enemy in k.host.spawns.as_ref().unwrap().live().to_vec() {
                    if !Host::tac_up(k.world, enemy) || k.world.unit(enemy)?.hidden {
                        continue;
                    }
                    let pos = k.world.unit(enemy)?.position;
                    let tile = pos.map(|v| (v + 0.5).floor() as i32);
                    if !Grid::in_bounds(tile) {
                        continue;
                    }
                    let key = tile[0] as usize * COLS + tile[1] as usize;
                    let flying = targeting::flying(k.world, enemy)?;
                    let state = k.world.unit_mut(id)?.operator.tactical.sbell2_mut()?;
                    if state.last.get(&enemy).copied() != Some(key) {
                        if let Some(prev) = state.last.insert(enemy, key) {
                            if !flying {
                                state.snow.remove(&prev);
                            }
                        }
                        if !flying && state.snow.contains_key(&key) && self.snow_damage > 0. {
                            let amount = k.stat(id, "atk")? * self.snow_damage;
                            k.damage(enemy, amount, DamageType::Arts, false, "snow")?;
                        }
                    }
                    if let Some(&layers) =
                        k.world.unit(id)?.operator.tactical.sbell2()?.snow.get(&key)
                    {
                        if k.world.unit(enemy)?.stats.alive && self.slow != 0. {
                            k.buff(
                                enemy,
                                "sbell2:snow",
                                [("moveMul".into(), (1. + self.slow * layers as f64).max(0.))]
                                    .into(),
                                0.2,
                            )?;
                        }
                    }
                }
                let valid: BTreeSet<_> = k.world.unit_ids().collect();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .sbell2_mut()?
                    .last
                    .retain(|enemy, _| valid.contains(enemy));
            }
            KitEvent::Landed { event, request }
                if event.target == id
                    && request.hit.damage_type != DamageType::Elemental
                    && self.cold > 0. =>
            {
                if let Some(source) = event.source.filter(|&source| {
                    Host::tac_up(k.world, source)
                        && k.world
                            .unit(source)
                            .is_ok_and(|u| u.side == health::Side::Enemy)
                }) {
                    k.status(source, "cold", self.cold, None)?;
                }
            }
            KitEvent::Fatal(event)
                if event.target == id
                    && !event.prevented
                    && !k.world.unit(id)?.operator.tactical.sbell2()?.blessed =>
            {
                event.prevented = true;
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .sbell2_mut()?
                    .blessed = true;
                let hp = (k.stat(id, "maxHp")? * self.restore).max(1.);
                k.world.unit_mut(id)?.stats.hp = hp;
                if self.freeze > 0. {
                    status::apply(
                        k.world,
                        k.host,
                        k.buffs,
                        Rect::FULL,
                        id,
                        "freeze",
                        status::Options {
                            source: Some(id),
                            duration: self.freeze,
                            force: true,
                            ..Default::default()
                        },
                    )?;
                }
                if self.enemy_freeze > 0. {
                    for enemy in k.enemies(None, true)? {
                        k.status(enemy, "freeze", self.enemy_freeze, None)?;
                    }
                }
            }
            KitEvent::Scheduled(due) if due[2] && k.up() && self.module_damage > 0. => {
                let count = k.enemies(None, true)?.len().min(self.module_cap);
                if count > 0 {
                    k.buff(
                        id,
                        "sbell2:heart",
                        [("dmgDealtMul".into(), 1. + self.module_damage * count as f64)].into(),
                        0.4,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
