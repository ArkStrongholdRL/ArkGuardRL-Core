// SPDX-License-Identifier: GPL-3.0-or-later
//! Native counterparts of the original tier-1 kits. Blackboard parameters stay
//! in external data; these builders compose the same SkillSpec/trait behaviour.
use crate::battle::{
    attacks::{Chain, Splash},
    operators::Combat,
    stats::{Buff, Numbers},
    waves::number,
};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kit {
    Yak,
    Leizi,
    Skgoat,
    Greyy,
}
pub(crate) struct Spec {
    pub modifiers: Numbers,
    pub permanent: Option<Buff>,
    pub combat: Combat,
}
impl Kit {
    pub fn resolve(key: &str) -> Result<Self, String> {
        Ok(match key {
            "chess_char_1_02_a" | "chess_char_1_02_b" => Self::Yak,
            "chess_char_1_03_a" | "chess_char_1_03_b" => Self::Leizi,
            "chess_char_1_11_a" | "chess_char_1_11_b" => Self::Skgoat,
            "chess_char_1_14_a" | "chess_char_1_14_b" => Self::Greyy,
            _ => return Err(format!("Operator kit not implemented: {key}")),
        })
    }
    pub fn identity(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Yak => ("skchr_yak_2", "TANK", "protector"),
            Self::Leizi => ("skchr_leizi_2", "CASTER", "chain"),
            Self::Skgoat => ("skcom_atk_up[2]", "SUPPORT", "slower"),
            Self::Greyy => ("skchr_greyy_2", "CASTER", "splashcaster"),
        }
    }
    pub fn attack_override(self) -> bool {
        matches!(self, Self::Leizi | Self::Greyy)
    }
    pub fn ranged(self) -> bool {
        self != Self::Yak
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
            combat.damage = crate::battle::damage::DamageType::Arts;
            combat.projectile = Some("bolt");
            combat.area.selection = combat.profile.selection;
        }
        let mut permanent = None;
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
                combat.unblocked_scale = number(&t["atk_scale"], 1.);
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
        }
    }
}
