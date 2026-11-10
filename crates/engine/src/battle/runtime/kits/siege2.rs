// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "artsfghter",
    ranged: false,
    can_hit_fly: false,
    physical: false,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_siege2_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "CUSTOM_RANGE",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    s.combat.damage = DamageType::Arts;
    s.combat.skill_damage = Some(DamageType::True);
    s.trigger_range = Some(tactical_grid(
        &raw["skill"]["trigger"]["customRangeGrid"],
        false,
    )?);
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        (
            "batPct".into(),
            flat_bat(raw, n(bb, "base_attack_time", 0.)),
        ),
    ]
    .into();
    let mut p = s.combat.profile;
    p.max_targets = n(bb, "attack@max_target", 1.).floor().max(1.);
    s.combat.skill_profile = Some(p);
    let grid = raw["talents"]
        .as_array()
        .and_then(|a| a.iter().find(|t| t["index"] == 0))
        .map(|t| &t["rangeGrid"])
        .ok_or("Missing Siege talent range")?;
    s.extra.tactics = Tactics::Siege2(Siege2 {
        grid: tactical_grid(grid, false)?,
        protection: n(t, "damage_resistance", 0.),
        attack: n(t, "atk", 0.),
        tremble: n(t1, "not_combat_normal", n(t1, "not_combat", 0.)),
        elite: n(t1, "not_combat_elite", n(t1, "not_combat", 0.)),
        fragile: n(tb, "damage_scale", 1.) - 1.,
        speed: n(tb, "attack_speed", 0.),
        tremble_scale: n(talent_at(raw, -1), "damage_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Siege2 {
    grid: Vec<[i32; 2]>,
    protection: f64,
    attack: f64,
    tremble: f64,
    elite: f64,
    fragile: f64,
    speed: f64,
    tremble_scale: f64,
}
#[derive(Default)]
pub(crate) struct State {
    lions: Vec<Id>,
    seen: BTreeSet<Id>,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            seen: previous.map(|s| s.seen.clone()).unwrap_or_default(),
            ..Default::default()
        }
    }
}
impl Behavior for Siege2 {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                for tile in k.summon_tiles(&self.grid, false)? {
                    if let Some(token) =
                        k.summon("token_10040_siege2_vlion", tile, skill.time_left)?
                    {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .siege2_mut()?
                            .lions
                            .push(token);
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                for token in
                    std::mem::take(&mut k.world.unit_mut(id)?.operator.tactical.siege2_mut()?.lions)
                {
                    let owner = k.unit;
                    k.unit = token;
                    let result = k.retreat();
                    k.unit = owner;
                    result?;
                }
                k.world.unit_mut(id)?.operator.advanced.extra_range.clear();
            }
            KitEvent::Scheduled(due) if due[2] && k.up() => {
                let keys = k.range(&self.grid)?;
                let mut allies = k.allies(&keys)?;
                if !allies.contains(&id) {
                    allies.push(id);
                }
                if self.protection > 0. {
                    for &ally in &allies {
                        k.buff(
                            ally,
                            "siege2:sigh",
                            [("physTakenMul".into(), 1. - self.protection)].into(),
                            0.4,
                        )?;
                    }
                }
                let count = allies.iter().filter(|&&a| a != id).count();
                if count > 0 && self.attack != 0. {
                    k.buff(
                        id,
                        "siege2:kings",
                        [("atkPct".into(), self.attack * count as f64)].into(),
                        0.4,
                    )?;
                }
                if self.speed != 0. && k.world.unit(id)?.blocking().is_empty() {
                    k.buff(id, "siege2:free", [("aspd".into(), self.speed)].into(), 0.4)?;
                }
                if self.fragile > 0. {
                    for target in k.world.unit_ids().collect::<Vec<_>>() {
                        if let Some(blocker) = k.world.unit(target)?.blocked_by() {
                            if blocker == id || k.world.unit(blocker)?.summoner == Some(id) {
                                k.status(target, "fragile", 0.4, Some(self.fragile))?;
                            }
                        }
                    }
                }
                let live = k.world.unit_ids().collect::<BTreeSet<_>>();
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .siege2_mut()?
                    .seen
                    .retain(|e| live.contains(e));
            }
            KitEvent::Tick if k.up() && k.active()? => {
                let keys = k.range(&self.grid)?;
                let mut targets = Vec::new();
                for ally in k.allies(&keys)? {
                    for &enemy in k.world.unit(ally)?.blocking() {
                        if let Some(key) = body::position_key(k.world.unit(enemy)?) {
                            targets.push(key);
                        }
                    }
                }
                k.world.unit_mut(id)?.operator.advanced.extra_range = targets;
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if self.tremble_scale > 1. && k.has(request.hit.target, "tremble") => {
                if request.hit.source.is_some_and(|a| {
                    a == id || k.world.unit(a).is_ok_and(|u| u.summoner == Some(id))
                }) {
                    request.damage.mul *= self.tremble_scale;
                }
            }
            KitEvent::Landed { event, .. }
                if event.source == Some(id)
                    && k.world.unit(event.target)?.side == health::Side::Enemy =>
            {
                if !k
                    .world
                    .unit(id)?
                    .operator
                    .tactical
                    .siege2()?
                    .seen
                    .contains(&event.target)
                {
                    let u = k.world.unit(event.target)?;
                    let duration = if u.boss || u.targeting.elite {
                        self.elite
                    } else {
                        self.tremble
                    };
                    if duration > 0. {
                        k.world
                            .unit_mut(id)?
                            .operator
                            .tactical
                            .siege2_mut()?
                            .seen
                            .insert(event.target);
                        if k.world.unit(event.target)?.stats.alive {
                            k.status(event.target, "tremble", duration, None)?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
