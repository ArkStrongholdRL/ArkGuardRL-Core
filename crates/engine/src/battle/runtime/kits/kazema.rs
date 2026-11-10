// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "SPECIAL",
    branch: "dollkeeper",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_kazema_2",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "DEFAULT",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    s.modifiers = [("atkPct".into(), n(&raw["skill"]["bb"], "atk", 0.))].into();
    s.extra.tactics = Tactics::Kazema(Kazema {
        loss: n(&raw["skill"]["bb"], "hp_ratio", 0.),
        scale: n(talent_at(raw, 0), "damage_scale", 0.),
        duration: n(&raw["trait"]["bb"], "duration", 20.),
        bonus: n(&raw["trait"]["bb"], "atk", 0.),
        grid: tactical_grid(&raw["trait"]["rangeGrid"], false)?,
    });
    Ok(())
}
pub(crate) struct Kazema {
    loss: f64,
    scale: f64,
    duration: f64,
    bonus: f64,
    pub grid: Vec<[i32; 2]>,
}
#[derive(Default)]
pub(crate) struct State {
    pub dollkeeper: super::dollkeeper::State,
    token: Option<Id>,
    substituted: bool,
}
impl State {
    pub(crate) fn redeployed(previous: Option<&Self>) -> Self {
        Self {
            // The tick observer, rather than deployment, detects a form change.
            // An immediate revive and another substitution can happen between ticks.
            substituted: previous.is_some_and(|s| s.substituted),
            ..Default::default()
        }
    }
}
impl Kazema {
    fn token_stats(k: &KitContext<'_>) -> Result<Value, String> {
        let def = k.host.tac_def(k.unit)?;
        let token =
            &k.host.data.table("tokens").ok_or("Missing summon data")?["token_10022_kazema_shadow"];
        Ok(token["variants"][&def.key]
            .get("stats")
            .unwrap_or(&token["stats"])
            .clone())
    }
    fn burst(&self, k: &mut KitContext<'_>, source: Id) -> Result<(), String> {
        let tile = k
            .world
            .unit(source)?
            .position
            .map(|value| (value + 0.5).floor() as i32);
        let amount = k.stat(source, "atk")? * self.scale;
        let owner = k.unit;
        k.unit = source;
        let result = (|| {
            for target in k.world.unit_ids().collect::<Vec<_>>() {
                let e = k.world.unit(target)?;
                if e.side == health::Side::Enemy
                    && Host::tac_up(k.world, target)
                    && !e.hidden
                    && body::tile_reach(e, tile[0], tile[1]) <= 1.
                    && !k.flag(target, "untargetable")?
                {
                    k.damage(target, amount, DamageType::Arts, true, "talent")?;
                }
            }
            Ok(())
        })();
        k.unit = owner;
        result
    }
}
impl Behavior for Kazema {
    fn profile(
        &self,
        w: &World,
        id: Id,
        _: Option<&skills::Runtime>,
        profile: &mut search::AttackProfile,
    ) -> Result<(), String> {
        if w.unit(id)?.operator.tactical.kazema()?.dollkeeper.doll {
            profile.selection.can_hit_fly = true;
        }
        Ok(())
    }
    fn priority(&self, event: &KitEvent<'_>, _: &World, _: Id) -> i32 {
        if matches!(event, KitEvent::Fatal(_)) {
            -100
        } else {
            0
        }
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        super::dollkeeper::handle(k, event)?;
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                ..
            } if *unit == id => {
                let amount = k.world.unit(id)?.stats.hp * self.loss;
                if amount > 0. && k.world.unit(id)?.stats.hp - amount >= 1. {
                    k.lose_hp(amount)?;
                }
                if let Some(tile) = k.free_adjacent()? {
                    if let Some(token) = k.summon("token_10022_kazema_shadow", tile, 0.)? {
                        k.world.unit_mut(id)?.operator.tactical.kazema_mut()?.token = Some(token);
                        self.burst(k, token)?;
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                ..
            } if *unit == id => {
                if let Some(token) = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .kazema_mut()?
                    .token
                    .take()
                {
                    let owner = k.unit;
                    k.unit = token;
                    let result = k.retreat();
                    k.unit = owner;
                    result?;
                }
            }
            KitEvent::Fatal(context) if context.target == id && !context.prevented => {
                let state = k.world.unit(id)?.operator.tactical.kazema()?;
                if state.dollkeeper.switching {
                    context.prevented = true;
                } else if !state.dollkeeper.doll {
                    let stats = Self::token_stats(k)?;
                    let hp = n(
                        &stats,
                        "maxHp",
                        k.world.unit(id)?.stats.base_number("maxHp"),
                    ) / k.world.unit(id)?.stats.base_number("maxHp").max(1.);
                    context.prevented = super::dollkeeper::enter(k, self.duration, hp, false)?;
                }
            }
            KitEvent::Resume { unit, skill } if *unit == id => super::dollkeeper::resume(k, skill)?,
            KitEvent::Attack {
                context,
                skill: Some(skill),
            } if context.attacker == id => super::dollkeeper::resume(k, skill)?,
            KitEvent::BeforeStatus(context)
                if context.target == id
                    && k.world
                        .unit(id)?
                        .operator
                        .tactical
                        .kazema()?
                        .dollkeeper
                        .switching
                    && ["stun", "freeze", "sleep"].contains(&context.key.as_str()) =>
            {
                context.cancelled = true
            }
            KitEvent::Tick => {
                let active = k.up() && k.has(id, "trait:substitute");
                let previous = k.world.unit(id)?.operator.tactical.kazema()?.substituted;
                if active && !previous {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .kazema_mut()?
                        .substituted = true;
                    let stats = Self::token_stats(k)?;
                    let mods = [
                        (
                            "atkFlat".into(),
                            n(&stats, "atk", 0.) - k.world.unit(id)?.stats.base_number("atk"),
                        ),
                        (
                            "defFlat".into(),
                            n(&stats, "def", 0.) - k.world.unit(id)?.stats.base_number("def"),
                        ),
                        ("atkPct".into(), self.bonus),
                    ]
                    .into();
                    k.buff(id, "kazema:doll", mods, f64::INFINITY)?;
                    self.burst(k, id)?;
                } else if !active && previous {
                    k.world
                        .unit_mut(id)?
                        .operator
                        .tactical
                        .kazema_mut()?
                        .substituted = false;
                    k.remove(id, "kazema:doll")?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
