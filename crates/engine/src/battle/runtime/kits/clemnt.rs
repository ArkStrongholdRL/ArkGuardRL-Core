// SPDX-License-Identifier: GPL-3.0-or-later
use super::dispatch::{Behavior, Context as KitContext, Event as KitEvent, Role};
use super::*;
use crate::battle::{element, skills::SpType, sp::Kind};

pub(crate) const KIT: Kit = Kit {
    position: "MELEE",
    profession: "WARRIOR",
    branch: "primguard",
    ranged: false,
    can_hit_fly: false,
    physical: true,
    projectile: "none",
    projectile_speed: 0.,
    required_range: None,
    skills: &[Skill {
        id: "skchr_clemnt_3",
        kind: Kind::Ammo,
        sp_type: SpType::Time,
        rule: "ACTIVE_RANGE",
        attack_override: true,
    }],
    build,
};
fn build(raw: &Value, _: bool, s: &mut Spec) -> Result<(), String> {
    let bb = &raw["skill"]["bb"];
    let t = talent_at(raw, 0);
    let t1 = talent_at(raw, 1);
    skill_range(raw, s)?;
    let mut profile = s.combat.profile;
    profile.max_targets = n(bb, "attack@max_target", 3.);
    s.combat.skill_profile = Some(profile);
    s.extra.effects.ammo_count = n(bb, "trigger_time", 10.);
    s.combat.skill_scale = n(bb, "attack@atk_scale", 1.);
    s.extra.tactics = Tactics::Clemnt(Clemnt {
        crit: n(t, "prob_normal", 0.),
        crit_debuffed: n(t, "prob_special", 0.),
        crit_scale: n(t, "t1_atk_scale", 1.),
        resistance: n(t1, "damage_resistance_normal", 0.),
        sea_resistance: n(t1, "damage_resistance_seamonster", 0.),
        rider: n(bb, "attack@ep_damage_ratio", 0.),
        mark_life: n(bb, "ep_break_duration", 1.),
        tile_life: n(bb, "projectile_life_time", 5.),
        bomb_scale: n(bb, "s3_atk_scale", 1.),
        element_scale: n(bb, "ep_damage_scale", 0.),
    });
    Ok(())
}
pub(crate) struct Clemnt {
    crit: f64,
    crit_debuffed: f64,
    crit_scale: f64,
    resistance: f64,
    sea_resistance: f64,
    rider: f64,
    mark_life: f64,
    tile_life: f64,
    bomb_scale: f64,
    element_scale: f64,
}
#[derive(Default)]
pub(crate) struct State {
    marks: Vec<(Id, f64)>,
    tiles: BTreeMap<[i32; 2], f64>,
}
impl Clemnt {
    fn ground(k: &mut KitContext<'_>, target: Id) -> Result<bool, String> {
        targeting::can_target_enemy(
            k.world,
            k.host,
            Some(k.unit),
            target,
            targeting::Profile {
                ground_only: true,
                ..Default::default()
            },
        )
    }
    fn bombard(k: &mut KitContext<'_>, position: [f64; 2], attack: f64) -> Result<(), String> {
        let def = k.host.tac_def(k.unit)?;
        let Tactics::Clemnt(c) = &def.extra.tactics else {
            return Err("Invalid Clement bombardment".into());
        };
        let tile = [position[0] as i32, position[1] as i32];
        let grid = [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]];
        let keys = targeting::absolute_range(&grid, tile, Direction::Right, 0);
        for target in k.host.spawns.as_ref().unwrap().live().to_vec() {
            if !Self::ground(k, target)? || !body::in_range(k.world.unit(target)?, &keys) {
                continue;
            }
            for (kind, scale, element) in [
                (DamageType::Physical, c.bomb_scale, None),
                (
                    DamageType::Elemental,
                    c.element_scale,
                    Some(element::Element::Erosion),
                ),
            ] {
                if !k.world.unit(target)?.stats.alive || k.world.unit(target)?.stats.hp <= 0. {
                    break;
                }
                let mut request = hit::Request::new(
                    health::Hit {
                        source: Some(k.unit),
                        credit: Some(k.unit),
                        target,
                        damage_type: kind,
                        no_sp: false,
                        silent: false,
                        hp_loss: false,
                        on_applied: false,
                    },
                    Damage {
                        amount: attack * scale,
                        mul: 1.,
                        resolved: false,
                        penetration: Default::default(),
                    },
                );
                request.is_skill = true;
                request.element = element;
                request.dodge_mode = injury::DodgeMode::Ignore;
                request.tags = vec!["skill".into(), "clemnt:bomb".into()];
                k.deal(&mut request)?;
            }
        }
        Ok(())
    }
}
impl Behavior for Clemnt {
    fn handle_unit(
        &self,
        k: &mut KitContext<'_>,
        role: Role,
        event: &mut KitEvent<'_>,
    ) -> Result<(), String> {
        match event {
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if role == Role::Source
                && request.hit.damage_type == DamageType::Physical
                && k.world.unit(request.hit.target)?.side == health::Side::Enemy =>
            {
                let defence = k.stat(request.hit.target, "def")?;
                let base = k.world.unit(request.hit.target)?.stats.base_number("def");
                if k.host.rng.next_f64()
                    < if defence < base {
                        self.crit_debuffed
                    } else {
                        self.crit
                    }
                {
                    request.damage.amount *= self.crit_scale;
                }
            }
            KitEvent::Hit {
                phase: hit::Phase::Hit,
                request,
            } if role == Role::Target
                && matches!(
                    request.hit.damage_type,
                    DamageType::Physical | DamageType::Arts
                ) =>
            {
                if let Some(source) = request.hit.source {
                    let at = k.world.unit(source)?.position;
                    let home = k.world.unit(k.unit)?.deployment.home;
                    let dir = k.host.roster.as_ref().unwrap().entry(k.unit)?.direction;
                    let [dr, dc] = dir.vector();
                    if k.world.unit(source)?.blocked_by() == Some(k.unit)
                        || (at[0].round() - home[0] as f64) * dr as f64
                            + (at[1].round() - home[1] as f64) * dc as f64
                            >= 0.
                    {
                        let sea = k
                            .host
                            .spawns
                            .as_ref()
                            .unwrap()
                            .definition(source)
                            .is_ok_and(|d| {
                                k.host.data.table("enemies").unwrap()[&d.key]["tags"]
                                    .as_array()
                                    .is_some_and(|a| a.iter().any(|s| s == "seamonster"))
                            });
                        request.damage.mul *= 1.
                            - if sea {
                                self.sea_resistance
                            } else {
                                self.resistance
                            };
                    }
                }
            }
            KitEvent::Landed { event, request }
                if role == Role::Source
                    && request.is_attack
                    && request.is_skill
                    && event.amount > 0.
                    && k.world.unit(request.hit.target)?.stats.hp > 0. =>
            {
                k.elemental(
                    request.hit.target,
                    event.amount * self.rider,
                    element::Element::Erosion,
                )?;
            }
            _ => {}
        }
        Ok(())
    }
    fn handle(&self, k: &mut KitContext<'_>, event: &mut KitEvent<'_>) -> Result<(), String> {
        let id = k.unit;
        match event {
            KitEvent::Attack { context, .. } if context.attacker == id => context.no_ammo = true,
            KitEvent::Skill {
                event: skills::Event::Start | skills::Event::End,
                ..
            } => {
                let state = k.world.unit_mut(id)?.operator.tactical.clemnt_mut()?;
                state.marks.clear();
                state.tiles.clear();
            }
            KitEvent::Element {
                phase: element::Phase::Burst,
                request,
            } if request.damage.element == Some(element::Element::Erosion)
                && k.up()
                && k.active()?
                && Self::ground(k, request.damage.hit.target)? =>
            {
                let until = k.world.time + self.mark_life;
                let state = k.world.unit_mut(id)?.operator.tactical.clemnt_mut()?;
                if let Some(mark) = state
                    .marks
                    .iter_mut()
                    .find(|m| m.0 == request.damage.hit.target)
                {
                    mark.1 = until;
                } else {
                    state.marks.push((request.damage.hit.target, until));
                }
            }
            KitEvent::Skill {
                event: skills::Event::Tick,
                skill,
                ..
            } if k.up() && skills::can_act(k.world, id)? => {
                let now = k.world.time;
                let mut marks = k.world.unit(id)?.operator.tactical.clemnt()?.marks.clone();
                marks.retain(|(target, until)| {
                    *until >= now - 1e-9
                        && k.world
                            .unit(*target)
                            .is_ok_and(|u| u.stats.alive && !u.hidden)
                });
                k.world
                    .unit_mut(id)?
                    .operator
                    .tactical
                    .clemnt_mut()?
                    .marks
                    .clone_from(&marks);
                let pos = k.world.unit(id)?.position;
                marks.sort_by(|a, b| {
                    let dist = |target| {
                        let at = k.world.unit(target).unwrap().position;
                        (at[0] - pos[0]).powi(2) + (at[1] - pos[1]).powi(2)
                    };
                    dist(a.0).total_cmp(&dist(b.0)).then_with(|| a.0.cmp(&b.0))
                });
                for (target, _) in marks {
                    if !Self::ground(k, target)? {
                        continue;
                    }
                    let at = k.world.unit(target)?.position;
                    let tile = [(at[0] + 0.5).floor() as i32, (at[1] + 0.5).floor() as i32];
                    let state = k.world.unit_mut(id)?.operator.tactical.clemnt_mut()?;
                    if state.tiles.get(&tile).is_some_and(|end| *end > now) {
                        continue;
                    }
                    state.tiles.retain(|_, end| *end > now);
                    state.tiles.insert(tile, now + self.tile_life);
                    state.marks.retain(|m| m.0 != target);
                    let attack = k.stat(id, "atk")?;
                    for n in 0..3 {
                        k.later_at(
                            0.5 + n as f64,
                            [tile[0] as f64, tile[1] as f64],
                            attack,
                            true,
                            Self::bombard,
                        )?;
                    }
                    k.host.with_kit_buffs(k.world, k.buffs, |host, world| {
                        skill.consume_ammo(world, host)
                    })?;
                    break;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
