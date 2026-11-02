// SPDX-License-Identifier: GPL-3.0-or-later
//! Native counterparts of the original tier-1 kits. Blackboard parameters stay
//! in external data; these builders compose the same SkillSpec/trait behaviour.
use crate::battle::{
    attacks::{Chain, Splash},
    body,
    direction::Direction,
    grid::{Grid, COLS},
    health::{Id, World},
    hit,
    operators::Combat,
    stats::{Buff, Numbers},
    waves::number,
};
use crate::deterministic::rng::Rng;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kit {
    Yak,
    Leizi,
    Skgoat,
    Greyy,
    Prove,
}
#[derive(Clone, Copy, Default)]
pub(crate) enum HitHandler {
    #[default]
    None,
    Leizi {
        unblocked_scale: f64,
    },
    Prove {
        drop: f64,
        up: f64,
        prob: f64,
        front_prob: f64,
        crit_scale: f64,
    },
}
impl HitHandler {
    pub fn apply(
        self,
        w: &mut World,
        source: Id,
        dir: Direction,
        q: &mut hit::Request,
        rng: &mut Rng,
    ) -> Result<(), String> {
        match self {
            Self::None => {}
            Self::Leizi { unblocked_scale } => {
                if w.unit(q.hit.target)?.blocked_by().is_none() {
                    q.damage.amount *= unblocked_scale;
                }
            }
            Self::Prove {
                drop,
                up,
                prob,
                front_prob,
                crit_scale,
            } => {
                if drop > 0. && up > 0. {
                    let u = w.unit_mut(q.hit.target)?;
                    let max = u.stats.stats().map_err(str::to_string)?.n("maxHp");
                    let ratio = if max > 0. { u.stats.hp / max } else { 0. };
                    let lost = (1. - ratio).min(1.).max(0.);
                    if lost > 0. {
                        q.damage.amount *= 1. + (lost / drop) * up;
                    }
                }
                if q.is_splash || q.tags.iter().any(|t| t == "chain") {
                    return Ok(());
                }
                let home = w.unit(source)?.deployment.tile;
                let v = dir.vector();
                let front = [home[0] + v[0], home[1] + v[1]];
                let in_front = Grid::in_bounds(front)
                    && body::in_range(
                        w.unit(q.hit.target)?,
                        &[front[0] as usize * COLS + front[1] as usize],
                    );
                let p = if in_front { front_prob } else { prob };
                if p > 0. && rng.chance(p) {
                    q.damage.amount *= crit_scale;
                }
            }
        }
        Ok(())
    }
}
pub(crate) struct Spec {
    pub modifiers: Numbers,
    pub permanent: Option<Buff>,
    pub combat: Combat,
    pub hit: HitHandler,
}
impl Kit {
    pub fn resolve(key: &str) -> Result<Self, String> {
        Ok(match key {
            "chess_char_1_02_a" | "chess_char_1_02_b" => Self::Yak,
            "chess_char_1_03_a" | "chess_char_1_03_b" => Self::Leizi,
            "chess_char_1_11_a" | "chess_char_1_11_b" => Self::Skgoat,
            "chess_char_1_14_a" | "chess_char_1_14_b" => Self::Greyy,
            "chess_char_1_07_a" | "chess_char_1_07_b" => Self::Prove,
            _ => return Err(format!("Operator kit not implemented: {key}")),
        })
    }
    pub fn identity(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Yak => ("skchr_yak_2", "TANK", "protector"),
            Self::Leizi => ("skchr_leizi_2", "CASTER", "chain"),
            Self::Skgoat => ("skcom_atk_up[2]", "SUPPORT", "slower"),
            Self::Greyy => ("skchr_greyy_2", "CASTER", "splashcaster"),
            Self::Prove => ("skchr_prove_1", "SNIPER", "closerange"),
        }
    }
    pub fn attack_override(self) -> bool {
        matches!(self, Self::Leizi | Self::Greyy)
    }
    pub fn ranged(self) -> bool {
        self != Self::Yak
    }
    pub fn passive(self) -> bool {
        self == Self::Prove
    }
    pub fn physical(self) -> bool {
        matches!(self, Self::Yak | Self::Prove)
    }
    pub fn projectile(self) -> &'static str {
        if self == Self::Prove {
            "arrow"
        } else if self.ranged() {
            "bolt"
        } else {
            "none"
        }
    }
    pub fn build(self, raw: &Value) -> Spec {
        let bb = &raw["skill"]["bb"];
        let t = raw["talents"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["index"] != -1)
            .map(|t| &t["bb"])
            .unwrap_or(&Value::Null);
        let tb = &raw["trait"]["bb"];
        let mut combat = Combat::default();
        if self.ranged() {
            combat.profile.selection.can_hit_fly = true;
            combat.damage = if self.physical() {
                crate::battle::damage::DamageType::Physical
            } else {
                crate::battle::damage::DamageType::Arts
            };
            combat.projectile = Some(self.projectile());
            combat.projectile_speed = if self == Self::Prove { 14. } else { 11. };
            combat.area.selection = combat.profile.selection;
        }
        let mut permanent = None;
        let mut hit = HitHandler::None;
        let modifiers = match self {
            Self::Yak => {
                let mut buff = Buff {
                    key: Some("talent:yak".into()),
                    mods: [("resFlat".into(), number(&t["magic_resistance"], 0.))].into(),
                    ..Default::default()
                };
                buff.runtime.persist = true;
                permanent = Some(buff);
                [
                    ("hpPct".into(), number(&bb["max_hp"], 0.)),
                    ("defPct".into(), number(&bb["def"], 0.)),
                    ("resMul".into(), 1. + number(&bb["magic_resistance"], 0.)),
                ]
                .into()
            }
            Self::Leizi => {
                combat.area.chain = Some(Chain {
                    count: number(&tb["attack@max_target"], 3.),
                    radius: 1.7,
                    falloff: if tb["attack@chain.atk_scale"].is_null() {
                        0.15
                    } else {
                        1. - number(&tb["attack@chain.atk_scale"], 0.85)
                    },
                    sluggish: number(&tb["attack@sluggish"], 0.5),
                });
                hit = HitHandler::Leizi {
                    unblocked_scale: number(&t["atk_scale"], 1.),
                };
                [("atkPct".into(), number(&bb["atk"], 0.))].into()
            }
            Self::Skgoat => {
                combat.sluggish = number(&tb["sluggish"], 0.8) + number(&t["sluggish"], 0.);
                [("atkPct".into(), number(&bb["atk"], 0.))].into()
            }
            Self::Greyy => {
                // Original Greyy's radius is 1.0 rather than the branch's 1.1.
                combat.area.splash = Some(Splash {
                    radius: 1.,
                    scale: 1.,
                    hits_fly: false,
                    fortress_melee: false,
                });
                combat.sluggish = number(&t["sluggish"], 0.);
                [("aspd".into(), number(&bb["attack_speed"], 0.))].into()
            }
            Self::Prove => {
                hit = HitHandler::Prove {
                    drop: number(&bb["hp_ratio_drop"], 0.),
                    up: number(&bb["atk_scale_up"], 0.),
                    prob: number(&t["prob"], 0.),
                    front_prob: number(&t["prob2"], 0.),
                    crit_scale: number(&t["atk_scale"], 1.),
                };
                Numbers::new()
            }
        };
        combat.skill_area = combat.area;
        if self == Self::Leizi {
            combat.skill_area.chain.as_mut().unwrap().falloff = 0.;
        }
        combat.skill_sluggish = if self == Self::Greyy {
            combat.sluggish * number(&bb["talent_scale"], 1.)
        } else {
            combat.sluggish
        };
        Spec {
            modifiers,
            permanent,
            combat,
            hit,
        }
    }
}
