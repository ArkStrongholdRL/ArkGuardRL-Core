// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 simdata.normalizeEnemy / battle.spawnEnemy.
use super::{
    body::HitArea,
    damage::DamageType,
    enemy::{Animation, Mode, Profile},
    health::{Side, Unit},
    stats::{Base, UnitStats},
    waves::{first, number},
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct Enemy {
    pub key: String,
    pub base: Base,
    pub profile: Profile,
    pub fly: bool,
    pub leader: bool,
    pub lpr: f64,
    pub block_weight: f64,
    pub immune: BTreeSet<String>,
    pub ep_resistance: f64,
    pub elemental_resistance: f64,
    pub not_count_in_total: bool,
    pub hit_area: Option<HitArea>,
    pub static_body: bool,
}
impl Enemy {
    /// Normalization is independent of content dispatch. A normalized definition
    /// alone never implies its abilities have been implemented.
    pub fn parse(key: &str, raw: &Value) -> Result<Self, String> {
        if !raw.is_object() {
            return Err(format!("{key}: expected enemy object"));
        }
        let st = raw.get("stats").filter(|v| !v.is_null()).unwrap_or(raw);
        let extra = &raw["extra"];
        let range = number(first(st, &["rangeRadius", "range"]), 0.).max(0.);
        let apply = first(raw, &["applyWay"])
            .as_str()
            .or_else(|| st["applyWay"].as_str())
            .unwrap_or(if range > 0. { "RANGED" } else { "MELEE" })
            .to_uppercase();
        let damage = first(st, &["dmgType"]);
        let damage = if damage.is_null() {
            first(raw, &["dmgType", "dmg"])
        } else {
            damage
        };
        let damage = if damage.is_array() {
            &damage[0]
        } else {
            damage
        };
        let damage = damage.as_str().unwrap_or("phys").to_uppercase();
        let mode = if apply == "NONE" || matches!(damage.as_str(), "NO_DAMAGE" | "NONE" | "NO") {
            Mode::None
        } else if damage == "HEAL" || damage == "HEALING" {
            Mode::Heal
        } else if damage.starts_with("MAG") || damage == "ARTS" {
            Mode::Damage(DamageType::Arts)
        } else if matches!(damage.as_str(), "TRUE" | "REAL" | "PURE") || damage.starts_with("ELEM")
        {
            Mode::Damage(DamageType::True)
        } else {
            Mode::Damage(DamageType::Physical)
        };
        let mut immune = BTreeSet::new();
        let immunity = first(st, &["immunities"]);
        let immunity = if immunity.is_null() {
            first(raw, &["immune", "immunities"])
        } else {
            immunity
        };
        let mut add = |s: &str| {
            immune.insert(s.strip_suffix("Immune").unwrap_or(s).to_lowercase());
        };
        if let Some(a) = immunity.as_array() {
            for v in a {
                if let Some(s) = v.as_str() {
                    add(s);
                }
            }
        }
        if let Some(o) = immunity.as_object() {
            for (k, v) in o {
                if v == true || v.as_f64().map(|n| n != 0.).unwrap_or(false) {
                    add(k);
                }
            }
        }
        if let Some(a) = first(st, &["otherImmunities"])
            .as_array()
            .or_else(|| raw["otherImmunities"].as_array())
        {
            for v in a {
                if let Some(s) = v.as_str() {
                    add(s);
                }
            }
        }
        let get = |key: &str, extra_key: &str, default: f64| {
            let v = first(st, &[key]);
            let v = if v.is_null() {
                first(extra, &[extra_key])
            } else {
                v
            };
            number(if v.is_null() { &raw[extra_key] } else { v }, default)
        };
        let aspd = number(first(st, &["aspd", "attackSpeed"]), 100.);
        let base = Base(BTreeMap::from([
            (
                "maxHp".into(),
                number(first(st, &["maxHp", "hp"]), 1000.).max(1.),
            ),
            ("atk".into(), number(&st["atk"], 0.).max(0.)),
            ("def".into(), number(&st["def"], 0.).max(0.)),
            (
                "res".into(),
                number(first(st, &["res", "magicResistance"]), 0.).max(0.),
            ),
            ("aspd".into(), if aspd == 0. { 100. } else { aspd }),
            (
                "bat".into(),
                number(first(st, &["bat", "baseAttackTime"]), 2.).max(0.1),
            ),
            ("rangeRadius".into(), range),
            (
                "moveSpeed".into(),
                number(first(st, &["moveSpeed", "spd"]), 1.).max(0.),
            ),
            (
                "massLevel".into(),
                number(first(st, &["massLevel", "mass"]), 1.),
            ),
            ("blockCnt".into(), 0.),
            ("spRecovery".into(), 0.),
            ("tauntLevel".into(), get("tauntLevel", "tauntLevel", 0.)),
            (
                "hpRecoveryPerSec".into(),
                get("hpRecoveryPerSec", "hpRecoveryPerSec", 0.),
            ),
        ]));
        let anim = &raw["attackAnim"];
        let dur = number(&anim["dur"], 0.);
        let animation = if dur > 0. {
            Some(Animation {
                duration: dur,
                hit: if anim["hit"].is_null() {
                    None
                } else {
                    let n = number(&anim["hit"], f64::NAN);
                    n.is_finite().then(|| n.max(0.).min(dur))
                },
            })
        } else {
            None
        };
        let area = &raw["hitArea"];
        let hit_area = HitArea::new(
            number(&area["w"], 0.),
            number(&area["h"], 0.),
            number(
                &area["dx"],
                if area["dx"].is_null() { 0. } else { f64::NAN },
            ),
            number(
                &area["dy"],
                if area["dy"].is_null() { 0. } else { f64::NAN },
            ),
        );
        Ok(Self {
            key: key.into(),
            base,
            profile: Profile {
                mode,
                no_attack: matches!(mode, Mode::None),
                melee: apply == "MELEE",
                attack_moves: raw["attackMoves"] == true,
                animation,
                ..Profile::default()
            },
            fly: first(st, &["motion"])
                .as_str()
                .or_else(|| raw["motion"].as_str())
                .unwrap_or("WALK")
                .eq_ignore_ascii_case("FLY"),
            leader: raw["rank"]
                .as_str()
                .unwrap_or("NORMAL")
                .eq_ignore_ascii_case("BOSS"),
            lpr: number(
                first(st, &["lpr"]),
                number(first(raw, &["lpr", "lifePointReduce"]), 1.),
            ),
            block_weight: get("blockCnt", "blockCnt", 1.).max(1.),
            immune,
            ep_resistance: get("elementRes", "epResistance", 0.),
            elemental_resistance: get("elementDmgRes", "epDamageResistance", 0.),
            not_count_in_total: raw["notCountInTotal"] == true,
            hit_area,
            static_body: raw["staticBody"] == true,
        })
    }
    pub fn with_overrides(&self, value: &Value) -> Self {
        let mut out = self.clone();
        let st = &value["stats"];
        if st.is_object() {
            for (key, alias, min) in [
                ("maxHp", "hp", 1.),
                ("atk", "atk", 0.),
                ("def", "def", 0.),
                ("res", "res", 0.),
                ("moveSpeed", "moveSpeed", 0.),
                ("bat", "bat", 0.1),
                ("rangeRadius", "rangeRadius", 0.),
            ] {
                let old = out.base.0[key];
                out.base
                    .0
                    .insert(key.into(), number(first(st, &[key, alias]), old).max(min));
            }
            for key in ["aspd", "massLevel"] {
                let old = out.base.0[key];
                let n = number(&st[key], old);
                out.base
                    .0
                    .insert(key.into(), if key == "aspd" && n == 0. { old } else { n });
            }
            out.lpr = number(&st["lpr"], out.lpr);
            out.block_weight = number(&st["blockCnt"], out.block_weight).max(1.);
        }
        out
    }
    pub fn unit(&self, mods: &Value, position: [f64; 2], time: f64) -> Result<Unit, String> {
        if !position.iter().all(|n| n.is_finite()) || !time.is_finite() {
            return Err("Invalid enemy spawn coordinates/time".into());
        }
        let mut base = self.base.clone();
        for (key, mul) in [
            ("maxHp", "hpMul"),
            ("atk", "atkMul"),
            ("def", "defMul"),
            ("res", "resMul"),
            ("moveSpeed", "speedMul"),
        ] {
            let n = number(&mods[mul], 1.);
            let n = if n < 0. || key == "maxHp" && n == 0. {
                1.
            } else {
                n
            };
            let result = base.0[key] * n;
            if !result.is_finite() {
                return Err("Enemy spawn stat overflow".into());
            }
            base.0.insert(key.into(), result);
        }
        let mut u = Unit::new(UnitStats::new(base), Side::Enemy);
        u.position = position;
        u.motion_fly = self.fly;
        u.ground = !self.fly;
        u.deployed = true;
        u.deployment.home = position.map(|n| (n + 0.5).floor() as i32);
        u.deployment.tile = u.deployment.home;
        u.deployment.deployed_at = time;
        u.block_weight = self.block_weight;
        u.hit_area = self.hit_area;
        u.static_body = self.static_body;
        u.immune = self.immune.clone();
        u.ep_resistance = self.ep_resistance;
        u.elemental_resistance = self.elemental_resistance;
        u.element_leader = self.leader;
        Ok(u)
    }
}
