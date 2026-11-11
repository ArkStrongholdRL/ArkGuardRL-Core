// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent};
use super::*;
pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "crusher",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_ulpia_3",
        kind: crate::battle::sp::Kind::Duration,
        sp_type: skills::SpType::Time,
        rule: "CUSTOM_RANGE",
        attack_override: false,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    let tb = &raw["trait"]["bb"];
    s.modifiers = [
        ("atkPct".into(), n(bb, "atk", 0.)),
        ("hpPct".into(), n(bb, "max_hp", 0.)),
    ]
    .into();
    s.combat.profile.hit_all_blocked = true;
    let grid = tactical_grid(&raw["skill"]["trigger"]["customRangeGrid"], false)?;
    s.trigger_range = Some(grid.clone());
    if n(tb, "heal_scale", 0.) > 0. {
        s.extra.effects.permanent.push(persistent(
            "ulpia:module",
            [("healingTakenMul".into(), n(tb, "heal_scale", 1.))].into(),
        ));
    }
    s.extra.tactics = Tactics::Ulpia(Ulpia {
        reach: grid
            .iter()
            .filter(|p| p[0] == 0)
            .map(|p| p[1])
            .max()
            .unwrap_or(1)
            .max(1),
        radius: n(bb, "projectile_range", 1.5),
        scale: n(bb, "atk_scale", 0.),
        stun: n(bb, "stun", 0.),
        heal: n(t, "value1", 0.),
        low_heal: n(t, "value2", 0.),
        threshold: n(t, "hp_ratio", 0.5),
        hp: n(t1, "max_hp", 0.),
        attack: n(t1, "atk", 0.),
        cap: n(t1, "max_stack_cnt", 9.).max(1.) as u32,
        share_hp: n(t1, "ulpia_t_1[abyssal].max_hp", 0.),
        share_attack: n(t1, "ulpia_t_1[abyssal].atk", 0.),
        share_cap: n(t1, "ulpia_t_1[abyssal].max_stack_cnt", 9.).max(1.) as u32,
        block_threshold: n(t1, "add_block_cnt_stack_cnt", 0.),
        blocks: n(t1, "block_cnt", 0.),
        blocked_scale: n(tb, "damage_scale", 1.),
    });
    Ok(())
}
pub(crate) struct Ulpia {
    reach: i32,
    radius: f64,
    scale: f64,
    stun: f64,
    heal: f64,
    low_heal: f64,
    threshold: f64,
    hp: f64,
    attack: f64,
    cap: u32,
    share_hp: f64,
    share_attack: f64,
    share_cap: u32,
    block_threshold: f64,
    blocks: f64,
    blocked_scale: f64,
}
#[derive(Default)]
pub(crate) struct State {
    home: Option<([i32; 2], Id)>,
}
impl Ulpia {
    fn stack(
        k: &mut KitContext<'_>,
        unit: Id,
        key: &str,
        mods: Numbers,
        cap: u32,
    ) -> Result<(), String> {
        let mut b = Buff {
            key: Some(key.into()),
            mods,
            ..Default::default()
        };
        b.runtime.refresh = buffs::Refresh::Stack;
        b.runtime.max_stacks = cap;
        k.add(unit, b)?;
        Ok(())
    }
    fn abyssal(k: &KitContext<'_>, unit: Id) -> Result<bool, String> {
        Ok(
            k.world.unit(unit)?.deployment.kind == crate::battle::deploy::Kind::Operator
                && [
                    "char_263_skadi",
                    "char_143_ghost",
                    "char_218_cuttle",
                    "char_474_glady",
                    "char_1023_ghost2",
                    "char_4145_ulpia",
                ]
                .contains(&k.host.tac_def(unit)?.char_id.as_str()),
        )
    }
}
impl Behavior for Ulpia {
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Skill {
                unit,
                event: skills::Event::Start,
                skill,
                ..
            } if *unit == id => {
                let origin = k.world.unit(id)?.deployment.tile;
                let facing = k
                    .host
                    .roster
                    .as_ref()
                    .unwrap()
                    .entry(id)?
                    .direction
                    .vector();
                let mut stop = 0;
                if k.world.unit(id)?.blocking().iter().all(|&e| {
                    !Host::tac_up(k.world, e) || k.world.unit(e).unwrap().blocked_by() != Some(id)
                }) {
                    for distance in 1..=self.reach {
                        let tile = [
                            origin[0] + facing[0] * distance,
                            origin[1] + facing[1] * distance,
                        ];
                        let grid = k.host.grid.as_ref().unwrap();
                        if !grid.in_rect(tile) || grid.obstacle(tile) != 0 {
                            break;
                        }
                        stop = distance;
                        let keys = [tile[0] as usize * COLS + tile[1] as usize];
                        if !k.enemies(Some(&keys), true)?.is_empty() {
                            break;
                        }
                    }
                }
                let anchor = [origin[0] + facing[0] * stop, origin[1] + facing[1] * stop];
                let position = anchor.map(f64::from);
                let amount = k.stat(id, "atk")? * self.scale;
                for target in k.radius(position, self.radius)? {
                    k.damage(target, amount, DamageType::Physical, true, "anchor")?;
                    if Host::tac_up(k.world, target) {
                        k.status(target, "stun", self.stun, None)?;
                    }
                }
                if stop > 0 && k.up() {
                    let mut destination = None;
                    for distance in [stop, stop + 1] {
                        let tile = [
                            origin[0] + facing[0] * distance,
                            origin[1] + facing[1] * distance,
                        ];
                        let grid = k.host.grid.as_ref().unwrap();
                        if grid.in_rect(tile)
                            && tile[1] >= 1
                            && tile[1] <= 9
                            && grid.can_stand(tile, false)
                            && grid.obstacle(tile) == 0
                            && !k
                                .host
                                .roster
                                .as_ref()
                                .unwrap()
                                .deployment()
                                .reserved(k.world, tile)?
                        {
                            destination = Some(tile);
                            break;
                        }
                    }
                    if let Some(destination) = destination {
                        if k.move_redeploy(destination, skill, false)?
                            && k.up()
                            && skill.bank.active
                        {
                            k.world.unit_mut(id)?.deployment.down_at_home = true;
                            if let Some(marker) = k.summon("token_10039_ulpia_block", origin, 0.)? {
                                k.world.unit_mut(id)?.operator.tactical.ulpia_mut()?.home =
                                    Some((origin, marker));
                            }
                        }
                    }
                }
            }
            KitEvent::Skill {
                unit,
                event: skills::Event::End,
                skill,
                ..
            } if *unit == id => {
                if k.up() {
                    k.world.unit_mut(id)?.deployment.down_at_home = false;
                }
                if let Some((home, marker)) = k
                    .world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .ulpia_mut()?
                    .home
                    .take()
                {
                    let owner = k.unit;
                    k.unit = marker;
                    let result = k.retreat();
                    k.unit = owner;
                    result?;
                    if k.up() {
                        k.move_redeploy(home, skill, true)?;
                    }
                }
            }
            KitEvent::Landed { event, request }
                if event.target == id
                    && k.up()
                    && k.world.unit(id)?.stats.hp > 0.
                    && event.amount > 0.
                    && request.hit.damage_type != DamageType::Elemental
                    && !request.hit.hp_loss =>
            {
                let ratio = k.world.unit(id)?.stats.hp / k.stat(id, "maxHp")?;
                let amount = if ratio < self.threshold {
                    self.low_heal
                } else {
                    self.heal
                };
                if amount > 0. {
                    k.heal(
                        id,
                        amount,
                        health::HealOptions {
                            self_heal: true,
                            ..Default::default()
                        },
                    )?;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if request.hit.source == Some(id)
                && k.world.unit(request.hit.target)?.blocked_by().is_some() =>
            {
                request.damage.mul *= self.blocked_scale;
            }
            KitEvent::Death {
                unit,
                credit: Some(killer),
            } if k.up()
                && k.world.unit(*unit)?.side == health::Side::Enemy
                && (*killer == id || self.block_threshold > 0. && Self::abyssal(k, *killer)?) =>
            {
                Self::stack(
                    k,
                    id,
                    "ulpia:blood",
                    [("hpFlat".into(), self.hp), ("atkFlat".into(), self.attack)].into(),
                    self.cap,
                )?;
                let stacks = k
                    .world
                    .unit(id)?
                    .stats
                    .buffs()
                    .iter()
                    .find(|b| b.key.as_deref() == Some("ulpia:blood"))
                    .map_or(0, |b| b.stacks);
                if self.block_threshold > 0. && stacks as f64 >= self.block_threshold {
                    k.buff(
                        id,
                        "ulpia:bloodBlock",
                        [("blockCnt".into(), self.blocks)].into(),
                        f64::INFINITY,
                    )?;
                }
                if self.share_hp != 0. || self.share_attack != 0. {
                    for ally in k.host.roster.as_ref().unwrap().ids().to_vec() {
                        if ally != id && Host::tac_up(k.world, ally) && Self::abyssal(k, ally)? {
                            Self::stack(
                                k,
                                ally,
                                "ulpia:bloodShare",
                                [
                                    ("hpFlat".into(), self.share_hp),
                                    ("atkFlat".into(), self.share_attack),
                                ]
                                .into(),
                                self.share_cap,
                            )?;
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}
