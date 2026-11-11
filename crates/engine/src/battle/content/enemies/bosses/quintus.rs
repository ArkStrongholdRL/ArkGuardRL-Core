// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
#[derive(Clone, Copy, Debug)]
pub struct Quintus {
    pub growth: [f64; 2],
    pub deadlines: [f64; 2],
    pub attack: [f64; 2],
    pub neural: f64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct QuintusState {
    pub stage: u8,
    pub born_at: f64,
    pub stage_at: f64,
    pub tentacles: usize,
}
#[derive(Clone, Debug)]
pub(crate) struct Stages {
    pub casters: [Caster; 3],
    pub branches: [Arc<str>; 3],
}
impl Stages {
    pub(in crate::battle::content::enemies) fn compile(raw: &Value, ov: &Value) -> Self {
        let names = [
            ["Tidewater", "Rockfall", "SummonTentac", ""],
            ["TidewaterG1", "RockfallG1", "SummonTentacG1", ""],
            ["TidewaterG2", "RockfallG2", "SummonTentacG2", "Doom"],
        ];
        let casters = std::array::from_fn(|stage| Caster {
            passive: Passive::Plain,
            skills: std::array::from_fn(|slot| {
                let name = names[stage][slot];
                let sk = abilities::skill(raw, ov, name);
                let n = |key: &str| {
                    sk["bb"][key]
                        .as_f64()
                        .filter(|n| n.is_finite())
                        .unwrap_or(0.)
                };
                let (spell, clip) = match slot {
                    0 => (
                        Spell::Tide {
                            stage: stage as u8,
                            scale: n("atk_scale"),
                            element: n("ep_damage_ratio"),
                        },
                        1.667,
                    ),
                    1 => (
                        Spell::Rockfall {
                            stage: stage as u8,
                            scale: n("atk_scale"),
                        },
                        3.,
                    ),
                    2 => (Spell::Tentacles { stage: stage as u8 }, 2.),
                    _ => (
                        Spell::Species {
                            loss: n("value").abs(),
                        },
                        4.467,
                    ),
                };
                casters::ability(raw, ov, name, spell, clip, true, false).map(|mut s| {
                    s.own = slot >= 2;
                    s
                })
            }),
        });
        Self {
            casters,
            branches: std::array::from_fn(|stage| {
                Arc::from(
                    abilities::skill(raw, ov, names[stage][2])["bbStr"]["branch_id"]
                        .as_str()
                        .unwrap_or("dslily_dstnta"),
                )
            }),
        }
    }
}
impl Boss {
    pub(super) fn quintus(key: &str, raw: &Value, ov: &Value) -> Option<Self> {
        if key != "enemy_1521_dslily" {
            return None;
        }
        let t = |key, default| talent_value(raw, ov, key).unwrap_or(default);
        Some(Self::Quintus(Quintus {
            growth: [t("growup1.hp_ratio", 1.), 2. * t("growup2.hp_ratio", 1.)],
            deadlines: [
                t("growup1.interval", f64::INFINITY),
                t("growup2.interval", f64::INFINITY),
            ],
            attack: [t("atkup1.atk", 0.), t("atkup2.atk", 0.)],
            neural: t("epdamage.attack@ep_damage_ratio", 0.),
        }))
    }
}
impl Runtime {
    pub(crate) fn quintus_caster(&self, id: Id) -> Option<Caster> {
        self.quintus_caster_ref(id).copied()
    }
    pub(crate) fn quintus_caster_ref(&self, id: Id) -> Option<&Caster> {
        let unit = self.units.get(&id)?;
        let Memory::Quintus(state) = unit.boss else {
            return None;
        };
        unit.definition
            .quintus
            .as_ref()
            .map(|s| &s.casters[state.stage as usize])
    }
    pub(crate) fn tentacle_branch(&self, id: Id) -> Option<Arc<str>> {
        let unit = self.units.get(&id)?;
        let Memory::Quintus(state) = unit.boss else {
            return None;
        };
        unit.definition
            .quintus
            .as_ref()
            .map(|s| s.branches[state.stage as usize].clone())
    }
}
