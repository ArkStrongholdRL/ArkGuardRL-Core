// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
pub(crate) const SPRING_SHIELD: u64 = 1715;
pub(crate) const SPRING_SHARE: u64 = 1716;
#[derive(Clone, Copy, Debug)]
pub struct Gun {
    pub caster: Caster,
    pub hidden: bool,
    pub raw_aspd: bool,
    pub step: f64,
    pub max: u32,
    pub erosion: f64,
    pub speed: f64,
    pub scale: f64,
    pub duration: f64,
    pub confession: f64,
    pub link: Option<[f64; 2]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpringKind {
    Arts,
    Element,
    Hits,
}
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub kind: SpringKind,
    pub scale: f64,
    pub regen: f64,
    pub pool: f64,
    pub counter: f64,
    pub erosion: f64,
    pub every: Option<u32>,
    pub times: u32,
    pub shot: f64,
}
#[derive(Clone, Debug)]
pub(crate) struct Charge {
    pub target: [f64; 2],
    pub until: f64,
    pub hit: Vec<Id>,
}
#[derive(Clone, Debug)]
pub(crate) struct Dash {
    pub gun: Id,
    pub until: f64,
    pub mul: f64,
    pub hit: Vec<Id>,
}
#[derive(Clone, Debug)]
pub(crate) struct GunState {
    pub last: Option<Id>,
    pub stacks: u32,
    pub charge: Option<Charge>,
    pub link_acc: f64,
}
#[derive(Clone, Debug)]
pub(crate) struct SpringState {
    pub up: bool,
    pub down_at: Option<f64>,
    pub attacks: u32,
    pub dash: Option<Dash>,
}
impl Boss {
    pub(super) fn gun(key: &str, raw: &Value, ov: &Value) -> Result<Option<Self>, String> {
        let t = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        let sk = |key| abilities::skill(raw, ov, key);
        let bb = |key, field: &str, default| {
            sk(key)["bb"][field]
                .as_f64()
                .filter(|x| x.is_finite())
                .unwrap_or(default)
        };
        Ok(Some(match key {
            "enemy_9017_achunt" | "enemy_9017_achunt_2" => {
                let hidden = key.ends_with("_2");
                Self::Gun(Gun {
                    caster: Caster {
                        passive: Passive::Plain,
                        skills: [
                            casters::ability(raw, ov, "1", Spell::GunCharge, 0., false, false),
                            if hidden {
                                casters::ability(
                                    raw,
                                    ov,
                                    "2",
                                    Spell::SpringDash {
                                        duration: bb("2", "dog_duration", 0.),
                                        mul: 1. + bb("2", "move_speed", 0.),
                                    },
                                    0.,
                                    false,
                                    false,
                                )
                            } else {
                                None
                            },
                            None,
                            None,
                        ],
                    },
                    hidden,
                    raw_aspd: raw["stats"]["aspd"].as_f64() == Some(0.),
                    step: t("2.attack_speed", 0.),
                    max: abilities::count(t("2.max_stack_cnt", 0.).max(0.).floor())?,
                    erosion: t("1.ep_damage_ratio", 0.),
                    speed: t("3.move_speed", 0.),
                    scale: t("3.atk_scale", 1.),
                    duration: t("3.duration", 0.),
                    confession: t("4.damage_scale", 1.),
                    link: if hidden && !sk("3").is_null() {
                        Some([bb("3", "interval", 1.), bb("3", "value", 0.)])
                    } else {
                        None
                    },
                })
            }
            "enemy_9018_actrpa" | "enemy_9019_actrpb" | "enemy_9020_actrpc" => {
                let kind = match key {
                    "enemy_9018_actrpa" => SpringKind::Arts,
                    "enemy_9019_actrpb" => SpringKind::Element,
                    _ => SpringKind::Hits,
                };
                let skill = sk("1");
                Self::Spring(Spring {
                    kind,
                    scale: t("1.damage_scale", 1.),
                    regen: t("1.regenerate_duration", 0.),
                    pool: if kind == SpringKind::Arts {
                        t("1.shield_ratio", 0.)
                    } else {
                        t("1.block_damage_max_times", 0.)
                    },
                    counter: t("1.atk_scale", 0.),
                    erosion: t("1.ep_damage_ratio", 0.),
                    every: if kind != SpringKind::Arts && !skill.is_null() {
                        let cost = skill["spCost"].as_f64().unwrap_or(0.);
                        Some(abilities::count(if cost > 0. { cost + 1. } else { 3. })?)
                    } else {
                        None
                    },
                    times: abilities::count(bb("1", "times", 1.).max(1.).ceil())?,
                    shot: if kind == SpringKind::Element {
                        bb("1", "ep_damage_ratio", 0.)
                    } else {
                        bb("1", "atk_scale", 0.)
                    },
                })
            }
            _ => return Ok(None),
        }))
    }
}
pub(crate) fn spring_shield(w: &mut World, id: Id, config: Spring) -> Result<Buff, String> {
    let mut b = Buff::default();
    b.key = Some("boss:springShield".into());
    b.runtime.persist = true;
    b.runtime.visible = true;
    b.runtime.injury.keep_empty_shield = true;
    b.runtime.injury.on_injury = Some(SPRING_SHIELD);
    b.runtime.injury.observe = Some(SPRING_SHIELD);
    match config.kind {
        SpringKind::Arts => {
            b.shield = w
                .unit_mut(id)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("maxHp")
                * config.pool;
            b.runtime.injury.shield_types = Some(vec![crate::battle::damage::DamageType::Arts]);
        }
        SpringKind::Hits => b.runtime.shield_hits = abilities::count(config.pool.max(0.).floor())?,
        SpringKind::Element => {}
    }
    Ok(b)
}
