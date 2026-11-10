// SPDX-License-Identifier: GPL-3.0-or-later
//! Native counterparts of the original operator kits. Blackboard parameters stay
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

use crate::battle::{skills::SpType, sp::Kind};
pub(crate) struct Skill {
    pub id: &'static str,
    pub kind: Kind,
    pub sp_type: SpType,
    pub rule: &'static str,
    pub attack_override: bool,
}
impl Skill {
    pub const fn duration(id: &'static str) -> Self {
        Self {
            id,
            kind: Kind::Duration,
            sp_type: SpType::Time,
            rule: "DEFAULT",
            attack_override: false,
        }
    }
}
pub(crate) struct Kit {
    pub position: &'static str,
    pub profession: &'static str,
    pub branch: &'static str,
    pub ranged: bool,
    pub can_hit_fly: bool,
    pub physical: bool,
    pub projectile: &'static str,
    pub projectile_speed: f64,
    pub required_range: Option<&'static [[i32; 2]]>,
    pub skills: &'static [Skill], // default first
    pub build: fn(&Value, bool, &mut Spec) -> Result<(), String>,
}
pub(crate) type AttackFilter = fn(&mut World, Id) -> Result<bool, String>;
#[derive(Clone, Copy, Default)]
pub(crate) enum HitHandler {
    #[default]
    None,
    Leizi(leizi::Hit),
    Prove(prove::Hit),
    Vigna(vigna::Hit),
    Blocked {
        scale: f64,
        own: bool,
        inverse: bool,
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
            Self::Blocked {
                scale,
                own,
                inverse,
            } => {
                let blocker = w.unit(q.hit.target)?.blocked_by();
                let blocked = if own {
                    blocker == Some(source)
                } else {
                    blocker.is_some()
                };
                if blocked != inverse {
                    q.damage.amount *= scale;
                }
                Ok(())
            }
            Self::None => Ok(()),
            Self::Leizi(h) => h.apply(w, q),
            Self::Vigna(h) => h.apply(w, q),
            Self::Prove(h) => h.apply(w, source, dir, q, rng),
        }
    }
}
#[derive(Default)]
pub(crate) struct Spec {
    pub extra: Extra,
    pub while_blocked: Option<WhileBlocked>,
    pub ground_bonus: Option<GroundBonus>,
    pub aura: Option<Aura>,
    pub element_attach: Option<ElementAttach>,
    pub undying: bool,
    pub end_stun: f64,
    pub skill_flags: std::collections::BTreeSet<String>,
    pub death_heal: Option<DeathHeal>,
    pub incoming_hit: Option<PhysicalResistance>,
    pub attack_proc: Option<AttackProc>,
    pub dp_on_kill: f64,
    pub initial_dp: f64,
    pub trigger_range: Option<Vec<[i32; 2]>>,
    pub rule_override: Option<crate::battle::skills::Rule>,
    pub skill_start: Option<SkillStart>,
    pub grid_scale: Option<GridScale>,
    pub modifiers: Numbers,
    pub permanent: Option<Buff>,
    pub combat: Combat,
    pub hit: HitHandler,
    pub attack_filter: Option<AttackFilter>,
}
fn talent(raw: &Value) -> &Value {
    raw["talents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| t["index"] != -1)
        .map(|t| &t["bb"])
        .unwrap_or(&Value::Null)
}
use crate::battle::runtime::kits::{
    archet, blemsh, demkni, lumen, mlynar, papyrs, pasngr, pepe, philae, qiubai, rmixer, snhunt,
    sntlla, texas2, tinman,
};
mod tactics;
pub(crate) use tactics::*;
mod advanced;
pub(crate) use advanced::*;
mod aglina;
mod akkord;
mod angel;
mod aroma;
mod ayer;
mod billro;
mod bldsk;
mod bpipe;
mod branch;
mod bubble;
mod cathy;
mod f12yin;
mod fartth;
mod flamtl;
mod flower;
mod forcer;
mod glady;
mod gnosis;
mod grabds;
mod gvial2;
mod haini;
mod harold;
mod hsguma;
mod humus;
mod inside;
mod kroos2;
mod lionhd;
mod lisa;
pub(crate) mod mechanics;
mod mint;
mod mizuki;
mod mostma;
mod mudrok;
mod plosis;
mod podego;
mod reckpr;
mod slbell;
mod sunbr;
mod support;
mod surtr;
mod svrash;
mod vendla;
pub(crate) use support::*;
mod pinecn;
mod shotst;
mod skadi;
mod slchan;
mod wildmn;
pub(crate) use mechanics::*;
mod ashlok;
mod effects;
mod estell;
mod excu;
mod extras;
mod ghost;
mod gravel;
mod greyy;
mod leizi;
mod liskam;
mod pithst;
mod prove;
mod skgoat;
mod snakek;
mod swire;
mod texas;
mod udflow;
mod utage;
mod vigna;
mod whitew;
mod yak;
use crate::battle::runtime::kits::angel2;
use crate::battle::runtime::kits::dusk;
use crate::battle::runtime::kits::etlchi;
use crate::battle::runtime::kits::kazema;
use crate::battle::runtime::kits::lemuen;
use crate::battle::runtime::kits::mlyss;
use crate::battle::runtime::kits::nearl2;
use crate::battle::runtime::kits::sbell2;
use crate::battle::runtime::kits::siege2;
use crate::battle::runtime::kits::skadi2;
use crate::battle::runtime::kits::svash2;
use crate::battle::runtime::kits::swire2;
use crate::battle::runtime::kits::thorn2;
use crate::battle::runtime::kits::ulpia;
use crate::battle::runtime::kits::vigil;
use crate::battle::runtime::kits::whitw2;
use crate::battle::runtime::kits::yu;
use crate::battle::runtime::kits::{
    agoat2, blaze2, blkkgt, caper, cello, cetsyr, excu2, ghost2, halo2, horn, indigo, ines, kjera,
    malist, nymph, rockr, silent, tippi, titi, vulpis,
};
use crate::battle::runtime::kits::{reed2, rosmon};
pub(crate) use effects::{
    passive_garrisons, AreaCast, AttackProc, Aura, DeathHeal, ElementAttach, GridScale,
    GroundBonus, PhysicalResistance, SkillStart, WhileBlocked,
};
pub(crate) use extras::*;
impl Kit {
    pub fn healing(&self) -> bool {
        self.profession == "MEDIC" && self.branch != "incantationmedic"
    }

    pub fn resolve(key: &str) -> Result<&'static Self, String> {
        Ok(match key {
            "chess_char_1_02_a" | "chess_char_1_02_b" => &yak::KIT,
            "chess_char_1_03_a" | "chess_char_1_03_b" => &leizi::KIT,
            "chess_char_1_11_a" | "chess_char_1_11_b" => &skgoat::KIT,
            "chess_char_1_14_a" | "chess_char_1_14_b" => &greyy::KIT,
            "chess_char_1_07_a" | "chess_char_1_07_b" => &prove::KIT,
            "chess_char_1_12_a" | "chess_char_1_12_b" => &estell::KIT,
            "chess_char_1_05_a" | "chess_char_1_05_b" => &vigna::KIT,
            "chess_char_1_08_a" | "chess_char_1_08_b" => &texas::KIT,
            "chess_char_2_01_a" | "chess_char_2_01_b" => &excu::KIT,
            "chess_char_3_16_a" | "chess_char_3_16_b" => &snakek::KIT,
            "chess_char_2_07_a" | "chess_char_2_07_b" => &ghost::KIT,
            "chess_char_2_18_a" | "chess_char_2_18_b" => &ashlok::KIT,
            "chess_char_3_03_a" | "chess_char_3_03_b" => &swire::KIT,
            "chess_char_1_15_a" | "chess_char_1_15_b" => &pithst::KIT,
            "chess_char_1_18_a" | "chess_char_1_18_b" => &utage::KIT,
            "chess_char_2_12_a" | "chess_char_2_12_b" => &gravel::KIT,
            "chess_char_2_16_a" | "chess_char_2_16_b" => &whitew::KIT,
            "chess_char_1_20_a" | "chess_char_1_20_b" => &liskam::KIT,
            "chess_char_1_04_a" | "chess_char_1_04_b" => &udflow::KIT,
            "chess_char_1_01_a" | "chess_char_1_01_b" => &inside::KIT,
            "chess_char_1_19_a" | "chess_char_1_19_b" => &wildmn::KIT,
            "chess_char_2_03_a" | "chess_char_2_03_b" => &slchan::KIT,
            "chess_char_2_08_a" | "chess_char_2_08_b" => &bubble::KIT,
            "chess_char_2_15_a" | "chess_char_2_15_b" => &akkord::KIT,
            "chess_char_2_17_a" | "chess_char_2_17_b" => &branch::KIT,
            "chess_char_3_01_a" | "chess_char_3_01_b" => &angel::KIT,
            "chess_char_3_02_a" | "chess_char_3_02_b" => &ayer::KIT,
            "chess_char_3_05_a" | "chess_char_3_05_b" => &skadi::KIT,
            "chess_char_3_10_a" | "chess_char_3_10_b" => &pinecn::KIT,
            "chess_char_3_17_a" | "chess_char_3_17_b" => &shotst::KIT,
            "chess_char_4_06_a" | "chess_char_4_06_b" => &kroos2::KIT,
            "chess_char_4_07_a" | "chess_char_4_07_b" => &bpipe::KIT,
            "chess_char_4_14_a" | "chess_char_4_14_b" => &lionhd::KIT,
            "chess_char_4_17_a" | "chess_char_4_17_b" => &hsguma::KIT,
            "chess_char_1_06_a" | "chess_char_1_06_b" => &vendla::KIT,
            "chess_char_1_10_a" | "chess_char_1_10_b" => &sunbr::KIT,
            "chess_char_1_13_a" | "chess_char_1_13_b" => &podego::KIT,
            "chess_char_2_04_a" | "chess_char_2_04_b" => &grabds::KIT,
            "chess_char_2_09_a" | "chess_char_2_09_b" => &humus::KIT,
            "chess_char_2_14_a" | "chess_char_2_14_b" => &flower::KIT,
            "chess_char_3_07_a" | "chess_char_3_07_b" => &forcer::KIT,
            "chess_char_3_08_a" | "chess_char_3_08_b" => &mint::KIT,
            "chess_char_3_09_a" | "chess_char_3_09_b" => &haini::KIT,
            "chess_char_3_14_a" | "chess_char_3_14_b" => &slbell::KIT,
            "chess_char_4_11_a" | "chess_char_4_11_b" => &cathy::KIT,
            "chess_char_4_21_a" | "chess_char_4_21_b" => &plosis::KIT,
            "chess_char_4_22_a" | "chess_char_4_22_b" => &svrash::KIT,
            "chess_char_4_23_a" | "chess_char_4_23_b" => &gvial2::KIT,
            "chess_char_4_26_a" | "chess_char_4_26_b" => &bldsk::KIT,
            "chess_char_2_05_a" | "chess_char_2_05_b" => &harold::KIT,
            "chess_char_4_02_a" | "chess_char_4_02_b" => &mostma::KIT,
            "chess_char_4_09_a" | "chess_char_4_09_b" => &mizuki::KIT,
            "chess_char_4_10_a" | "chess_char_4_10_b" => &aroma::KIT,
            "chess_char_4_24_a" | "chess_char_4_24_b" => &billro::KIT,
            "chess_char_4_18_a" | "chess_char_4_18_b" => &mudrok::KIT,
            "chess_char_4_19_a" | "chess_char_4_19_b" => &flamtl::KIT,
            "chess_char_4_20_a" | "chess_char_4_20_b" => &fartth::KIT,
            "chess_char_5_10_a" | "chess_char_5_10_b" => &lisa::KIT,
            "chess_char_5_17_a" | "chess_char_5_17_b" => &f12yin::KIT,
            "chess_char_5_07_a" | "chess_char_5_07_b" => &surtr::KIT,
            "chess_char_5_20_a" | "chess_char_5_20_b" => &aglina::KIT,
            "chess_char_5_23_a" | "chess_char_5_23_b" => &reckpr::KIT,
            "chess_char_4_12_a" | "chess_char_4_12_b" => &glady::KIT,
            "chess_char_4_13_a" | "chess_char_4_13_b" => &gnosis::KIT,
            "chess_char_2_06_a" | "chess_char_2_06_b" => &papyrs::KIT,
            "chess_char_2_19_a" | "chess_char_2_19_b" => &tinman::KIT,
            "chess_char_3_06_a" | "chess_char_3_06_b" => &philae::KIT,
            "chess_char_3_11_a" | "chess_char_3_11_b" => &snhunt::KIT,
            "chess_char_3_12_a" | "chess_char_3_12_b" => &blemsh::KIT,
            "chess_char_3_21_a" | "chess_char_3_21_b" => &archet::KIT,
            "chess_char_4_01_a" | "chess_char_4_01_b" => &rmixer::KIT,
            "chess_char_4_16_a" | "chess_char_4_16_b" => &texas2::KIT,
            "chess_char_5_11_a" | "chess_char_5_11_b" => &demkni::KIT,
            "chess_char_5_19_a" | "chess_char_5_19_b" => &mlynar::KIT,
            "chess_char_5_21_a" | "chess_char_5_21_b" => &sntlla::KIT,
            "chess_char_6_05_a" | "chess_char_6_05_b" => &pasngr::KIT,
            "chess_char_6_15_a" | "chess_char_6_15_b" => &qiubai::KIT,
            "chess_char_6_06_a" | "chess_char_6_06_b" => &pepe::KIT,
            "chess_char_6_14_a" | "chess_char_6_14_b" => &lumen::KIT,
            "chess_char_1_09_a" | "chess_char_1_09_b" => &caper::KIT,
            "chess_char_1_17_a" | "chess_char_1_17_b" => &indigo::KIT,
            "chess_char_2_02_a" | "chess_char_2_02_b" => &silent::KIT,
            "chess_char_2_10_a" | "chess_char_2_10_b" => &rockr::KIT,
            "chess_char_2_13_a" | "chess_char_2_13_b" => &tippi::KIT,
            "chess_char_3_13_a" | "chess_char_3_13_b" => &malist::KIT,
            "chess_char_3_18_a" | "chess_char_3_18_b" => &vulpis::KIT,
            "chess_char_3_20_a" | "chess_char_3_20_b" => &kjera::KIT,
            "chess_char_4_04_a" | "chess_char_4_04_b" => &ines::KIT,
            "chess_char_4_25_a" | "chess_char_4_25_b" => &cetsyr::KIT,
            "chess_char_5_01_a" | "chess_char_5_01_b" => &excu2::KIT,
            "chess_char_5_02_a" | "chess_char_5_02_b" => &titi::KIT,
            "chess_char_5_03_a" | "chess_char_5_03_b" => &blaze2::KIT,
            "chess_char_5_08_a" | "chess_char_5_08_b" => &horn::KIT,
            "chess_char_5_13_a" | "chess_char_5_13_b" => &ghost2::KIT,
            "chess_char_5_22_a" | "chess_char_5_22_b" => &nymph::KIT,
            "chess_char_6_09_a" | "chess_char_6_09_b" => &cello::KIT,
            "chess_char_6_16_a" | "chess_char_6_16_b" => &halo2::KIT,
            "chess_char_6_19_a" | "chess_char_6_19_b" => &blkkgt::KIT,
            "chess_char_6_20_a" | "chess_char_6_20_b" => &agoat2::KIT,
            "chess_char_6_08_a" | "chess_char_6_08_b" => &reed2::KIT,
            "chess_char_6_12_a" | "chess_char_6_12_b" => &rosmon::KIT,
            "chess_char_6_02_a" | "chess_char_6_02_b" => &sbell2::KIT,
            "chess_char_6_04_a" | "chess_char_6_04_b" => &skadi2::KIT,
            "chess_char_5_14_a" | "chess_char_5_14_b" => &svash2::KIT,
            "chess_char_6_13_a" | "chess_char_6_13_b" => &angel2::KIT,
            "chess_char_5_15_a" | "chess_char_5_15_b" => &thorn2::KIT,
            "chess_char_6_03_a" | "chess_char_6_03_b" => &yu::KIT,
            "chess_char_5_12_a" | "chess_char_5_12_b" => &dusk::KIT,
            "chess_char_2_11_a" | "chess_char_2_11_b" => &kazema::KIT,
            "chess_char_6_01_a" | "chess_char_6_01_b" => &lemuen::KIT,
            "chess_char_6_17_a" | "chess_char_6_17_b" => &nearl2::KIT,
            "chess_char_6_07_a" | "chess_char_6_07_b" => &siege2::KIT,
            "chess_char_5_05_a" | "chess_char_5_05_b" => &ulpia::KIT,
            "chess_char_3_04_a" | "chess_char_3_04_b" => &swire2::KIT,
            "chess_char_6_18_a" | "chess_char_6_18_b" => &whitw2::KIT,
            "chess_char_5_06_a" | "chess_char_5_06_b" => &etlchi::KIT,
            "chess_char_3_19_a" | "chess_char_3_19_b" => &vigil::KIT,
            "chess_char_6_11_a" | "chess_char_6_11_b" => &mlyss::KIT,
            _ => return Err(format!("Operator kit not implemented: {key}")),
        })
    }
    pub fn build(&self, raw: &Value) -> Result<Spec, String> {
        let mut s = Spec::default();
        // Damage type is independent of ranged/projectile delivery.
        s.combat.damage = if self.physical {
            crate::battle::damage::DamageType::Physical
        } else {
            crate::battle::damage::DamageType::Arts
        };
        if self.ranged {
            s.combat.profile.selection.can_hit_fly = self.can_hit_fly;
            s.combat.projectile = Some(self.projectile);
            s.combat.projectile_speed = self.projectile_speed;
            s.combat.area.selection = s.combat.profile.selection;
            s.combat.skill_area = s.combat.area;
        }
        if self.healing() {
            support::healing(&mut s, if self.branch == "ringhealer" { 3. } else { 1. });
        }
        (self.build)(raw, raw["skill"]["skillId"] == self.skills[0].id, &mut s)?;
        if let Some(observer) = &s.extra.effects.observer {
            let mut b = persistent(observer.key, Numbers::new());
            b.runtime.injury.observe = Some(INJURY_OBSERVER);
            s.extra.effects.permanent.push(b);
        }
        Ok(s)
    }
}
