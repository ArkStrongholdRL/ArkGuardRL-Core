// SPDX-License-Identifier: GPL-3.0-or-later
//! Original default operator lifecycle, sharing the world's units and buffs.
//! This roster is a battle component, not an env or a complete allied AI loop.
//! Only explicitly compiled kits are admitted; no generic unimplemented fallback.
use super::{
    blocking, buffs,
    content::operators::Kit,
    deploy::{self, Deployment},
    direction::Direction,
    grid::{Grid, COLS},
    health::{Id, Side, Unit, World},
    search::AttackProfile,
    skills,
    sp::{Bank, Kind},
    stats::{Base, Buff, Numbers, UnitStats},
    waves::{first, number},
};
use crate::data::Catalog;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) fn parse_range(raw: &Value) -> Result<Vec<[i32; 2]>, String> {
    let grid = raw.as_array().ok_or("Missing range grid")?;
    if grid.is_empty() || grid.len() > super::grid::N {
        return Err("Invalid range size".into());
    }
    grid.iter()
        .map(|p| {
            if p.as_array().map(Vec::len) != Some(2) {
                return Err("Expected range offset pair".to_string());
            }
            let get = |i: usize| {
                p[i].as_i64()
                    .and_then(|n| i32::try_from(n).ok())
                    .filter(|n| n.unsigned_abs() <= super::grid::COLS as u32)
                    .ok_or("Range offset outside field bound".to_string())
            };
            Ok([get(0)?, get(1)?])
        })
        .collect::<Result<Vec<_>, String>>()
}
pub struct Definition {
    pub key: String,
    base: Base,
    original_cost: f64,
    pub(crate) char_id: String,
    pub(crate) profession: &'static str,
    pub(crate) laterano: bool,
    max_charges: u32,
    pub(crate) extra: super::content::operators::Extra,
    modifiers: Numbers,
    permanent: Vec<Buff>,
    pub(crate) equipment: Vec<super::equipment::Grant>,
    pub(crate) range_grid: Vec<[i32; 2]>,
    pub(crate) combat: Combat,
    pub(crate) hit: super::content::operators::HitHandler,
    pub(crate) attack_filter: Option<super::content::operators::AttackFilter>,
    skill_flags: std::collections::BTreeSet<String>,
    pub(crate) death_heal: Option<super::content::operators::DeathHeal>,
    pub(crate) incoming_hit: Option<super::content::operators::PhysicalResistance>,
    pub(crate) attack_proc: Option<super::content::operators::AttackProc>,
    pub(crate) dp_on_kill: f64,
    pub(crate) initial_dp: f64,
    pub(crate) trigger_range: Option<Vec<[i32; 2]>>,
    pub(crate) skill_start: Option<super::content::operators::SkillStart>,
    pub(crate) grid_scale: Option<super::content::operators::GridScale>,
    pub(crate) while_blocked: Option<super::content::operators::WhileBlocked>,
    pub(crate) ground_bonus: Option<super::content::operators::GroundBonus>,
    pub(crate) aura: Option<super::content::operators::Aura>,
    pub(crate) element_attach: Option<super::content::operators::ElementAttach>,
    pub(crate) undying: bool,
    end_stun: f64,
    pub(crate) melee_operator: bool,
    kind: Kind,
    spec: skills::Spec,
    cost: f64,
    no_skill: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct Combat {
    pub profile: AttackProfile,
    pub damage: super::damage::DamageType,
    pub skill_damage: Option<super::damage::DamageType>,
    pub normal_hits: u32,
    pub skill_scale: f64,
    pub heal: bool,
    pub skill_heal: bool,
    pub skill_heal_scale: f64,
    pub skill_direct: bool,
    pub split_mul: Option<f64>,
    pub projectile: Option<&'static str>,
    pub projectile_speed: f64,
    pub area: super::attacks::Area,
    pub skill_area: super::attacks::Area,
    pub sluggish: f64,
    pub skill_sluggish: f64,
    pub skill_hits: u32,
    pub skill_profile: Option<AttackProfile>,
}
impl Combat {
    pub(crate) const MAX_HITS: u32 = 5;
}
impl Default for Combat {
    fn default() -> Self {
        Self {
            profile: AttackProfile::default(),
            damage: super::damage::DamageType::Physical,
            skill_damage: None,
            normal_hits: 1,
            skill_scale: 1.,
            heal: false,
            skill_heal: false,
            skill_heal_scale: 1.,
            skill_direct: false,
            split_mul: None,
            projectile: None,
            projectile_speed: 0.,
            area: Default::default(),
            skill_area: Default::default(),
            sluggish: 0.,
            skill_sluggish: 0.,
            skill_hits: 1,
            skill_profile: None,
        }
    }
}
impl Definition {
    pub(crate) fn skill_kind(&self) -> Kind {
        self.kind
    }
    pub(crate) fn sp_type(&self) -> skills::SpType {
        self.spec.sp_type
    }

    pub(crate) fn skill_event<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        e: skills::Event,
        direction: Direction,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        skills::Hooks::event(
            &mut Skill {
                r,
                h,
                definition: self,
                aura: self.aura.as_ref(),
                direction,
                end_stun: self.end_stun,
                mods: &self.modifiers,
                flags: &self.skill_flags,
            },
            e,
            c,
            s,
            w,
        )
    }
    pub fn load(data: &Catalog, key: &str) -> Result<Self, String> {
        Self::load_with_skill(data, key, None)
    }
    /// Select a supported skill by its external record index; module and potential
    /// stay at their defaults. Unknown indices never fall back to a default skill.
    pub fn load_with_skill(data: &Catalog, key: &str, index: Option<u32>) -> Result<Self, String> {
        let raw = &data.table("chess").ok_or("Missing chess data")?[key];
        let mut definition = if let Some(index) = index {
            let skill = raw["skills"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["index"].as_u64() == Some(u64::from(index)))
                .ok_or_else(|| format!("Unknown operator skill index {index}: {key}"))?;
            let mut selected = raw.clone();

            selected["skill"] = skill.clone();
            Self::compile(key, &selected)
        } else {
            Self::compile(key, raw)
        }?;
        definition
            .permanent
            .extend(super::content::operators::passive_garrisons(data, raw)?);
        for gid in raw["garrisonIds"].as_array().into_iter().flatten() {
            let g = &data.table("garrisons").ok_or("Missing garrison data")?
                [gid.as_str().ok_or("Expected garrison id")?];
            match g["effectKey"].as_str().unwrap_or("") {
                "act1autochess_gar_eff_chaos" => definition.extra.weakness = true,
                "act1autochess_gar_eff_attack_enemy" => {
                    if let Some(tag) = g["bbStr"]["check_tag"].as_str() {
                        definition
                            .extra
                            .tag_scale
                            .push((tag.into(), number(&g["bb"]["atk"], 1.)));
                    }
                }
                _ => {}
            }
        }
        Ok(definition)
    }

    /// Resolve a summon from the selected owner form.
    pub(crate) fn token(data: &Catalog, key: &str, owner: &str) -> Result<Self, String> {
        use super::content::operators::Skill;
        let raw = data
            .table("tokens")
            .and_then(|t| t.get(key))
            .ok_or("Unknown summon definition")?;
        let mut selected = raw.clone();
        if key == "enemy_9012_acloon" {
            selected["rangeGrid"] = serde_json::json!([[0, 0]]);
            selected["stats"]["spRecovery"] = Value::from(1.);
        }

        if let Some(variant) = raw["variants"].get(owner) {
            for (key, value) in variant.as_object().ok_or("Invalid summon variant")? {
                selected[key] = value.clone();
            }
        }
        let ranged = selected["attackKind"] == "ranged" || selected["attackKind"] == "heal";
        let projectile = match selected["projectile"].as_str().unwrap_or("none") {
            "none" | "orb" => "none",
            "arrow" => "arrow",
            "bolt" => "bolt",
            "lob" => "lob",
            "bomb" => "bomb",
            _ => return Err("Unsupported summon projectile".into()),
        };
        let position = if selected["position"] == "RANGED" {
            "RANGED"
        } else {
            "MELEE"
        };
        let kit = Kit {
            position,
            profession: "TOKEN",
            branch: "native_token",
            ranged,
            can_hit_fly: selected["canHitFly"] == true,
            physical: selected["dmgType"] != "arts",
            projectile,
            projectile_speed: match projectile {
                "arrow" => 14.,
                "bolt" => 11.,
                "bomb" | "lob" => 8.,
                _ => 0.,
            },
            required_range: None,
            skills: &[Skill {
                id: "native_token",
                kind: Kind::Instant,
                sp_type: skills::SpType::Time,
                rule: "NEVER",
                attack_override: false,
            }],
            build: |raw, _, spec| {
                spec.combat.profile.no_attack = raw["nativeNoAttack"] == true;
                if raw["tokenId"] == "enemy_9012_acloon" {
                    super::runtime::kits::yanyou::build(raw, spec);
                }
                if raw["tokenId"] == "token_10028_vigil_wolf" {
                    super::runtime::kits::wolf::build(raw, spec);
                }
                if raw["tokenId"] == "token_10030_mlyss_wtrman" {
                    super::runtime::kits::manifold::build(raw, spec);
                }
                if raw["tokenId"] == "token_10000_silent_healrb"
                    || raw["tokenId"] == "token_10041_cathy_catsld"
                {
                    super::runtime::kits::prepared_summon::build(raw, spec);
                }
                if raw["nativeTrueDamage"] == true {
                    spec.combat.damage = super::damage::DamageType::True;
                }
                if raw["tokenId"] == "token_10039_ulpia_block"
                    || raw["tokenId"] == "token_10031_swire2_gdtrap"
                {
                    spec.combat.profile.no_attack = true;
                    let mut buff = Buff {
                        key: Some("trait:untargetable".into()),
                        permanent: true,
                        ..Default::default()
                    };
                    buff.runtime.persist = true;
                    buff.flags.insert("untargetable".into());
                    if raw["tokenId"] == "token_10039_ulpia_block" {
                        spec.extra.effects.permanent.push(buff);
                    } else {
                        let mut hold = Buff {
                            key: Some("token:champagneHold".into()),
                            permanent: true,
                            ..Default::default()
                        };
                        hold.runtime.persist = true;
                        hold.flags = ["noHeal", "healFree", "isolated"]
                            .map(str::to_string)
                            .into();
                        spec.extra.effects.permanent.extend([hold, buff]);
                    }
                }
                let mut buff = Buff {
                    key: Some("trait:abnormal".into()),
                    permanent: true,
                    ..Default::default()
                };
                buff.runtime.persist = true;
                for flag in raw["abnormal"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if flag == "healFree" || flag == "isolated" {
                        buff.flags.insert("noHeal".into());
                    }
                    if flag == "isolated" {
                        buff.flags.insert("isolated".into());
                    }
                }
                if !buff.flags.is_empty() {
                    spec.extra.effects.permanent.push(buff);
                }
                Ok(())
            },
        };
        if selected["dmgType"] == "heal" {
            selected["dmgType"] = Value::from("phys");
        }
        selected["nativeTrueDamage"] = Value::Bool(selected["dmgType"] == "true");
        if selected["nativeTrueDamage"] == true {
            selected["dmgType"] = Value::from("phys");
        }
        selected["nativeNoAttack"] = Value::Bool(selected["attackKind"] == "none");
        selected["attackKind"] = Value::from(if ranged { "ranged" } else { "melee" });
        selected["projectile"] = Value::from(projectile);
        selected["position"] = Value::from(position);
        selected["profession"] = Value::from("TOKEN");
        selected["subProfessionId"] = Value::from("native_token");
        selected["immunities"] = Value::Null;
        selected["nativeSkill"] = selected["skill"].clone();
        selected["skill"] = serde_json::json!({ "skillId":"native_token", "spType":"INCREASE_WITH_TIME", "trigger":{"rule":"NEVER"} });
        let mut definition = Self::compile_kit(key, &selected, &kit)?;
        definition.no_skill = true;
        if key == "enemy_9012_acloon" {
            let sk = selected["skills"]
                .as_array()
                .and_then(|a| a.iter().find(|s| s["prefabKey"] == "Skill_2"))
                .ok_or("Missing yanyou skill")?;
            definition.no_skill = false;
            definition.kind = Kind::Duration;
            definition.cost = number(&sk["cooldown"], 15.);
            definition.spec.duration = number(&sk["bb"]["hit_duration"], 20.);
            definition.spec.init_sp =
                (definition.cost - number(&sk["initCooldown"], definition.cost)).max(0.);
            definition.spec.rule = skills::Rule::Default;
        }

        if key == "token_10030_mlyss_wtrman" {
            definition.kind = Kind::Toggle;
            definition.no_skill = false;
            definition.spec.duration = number(&selected["nativeSkill"]["duration"], 0.).max(0.);
            definition.spec.init_sp = number(&selected["nativeSkill"]["initSp"], 0.).max(0.);
            definition.spec.sp_type = skills::SpType::Time;
            definition.cost = number(&selected["nativeSkill"]["spCost"], 0.).max(0.);
        }

        Ok(definition)
    }
    pub fn load_with_loadout(
        data: &Catalog,
        key: &str,
        index: Option<u32>,
        items: &[String],
    ) -> Result<Self, String> {
        if items.len() > 16 {
            return Err("Equipment slot capacity exceeded".into());
        }
        let mut definition = Self::load_with_skill(data, key, index)?;
        for id in items {
            definition
                .equipment
                .push(super::equipment::compile(data, id)?);
        }
        definition.laterano = super::equipment::bonds(data, key, items)
            .iter()
            .any(|b| b == "lateranoShip");
        Ok(definition)
    }
    /// Compile an explicitly supported operator skill with its recorded loadout.
    pub fn compile(key: &str, raw: &Value) -> Result<Self, String> {
        let kit = if raw["isDiy"] == true {
            match raw["charId"].as_str() {
                Some("char_4231_clemnt") => &super::runtime::kits::clemnt::KIT,
                _ => {
                    return Err(format!(
                        "Unsupported DIY combat operator: {}",
                        raw["charId"]
                    ))
                }
            }
        } else {
            Kit::resolve(key)?
        };
        Self::compile_kit(key, raw, kit)
    }
    fn compile_kit(key: &str, raw: &Value, kit: &Kit) -> Result<Self, String> {
        let skill_id = raw["skill"]["skillId"].as_str().unwrap_or("");
        if !raw.is_object() {
            return Err(format!("Invalid operator definition: {key}"));
        }
        let selected = kit
            .skills
            .iter()
            .find(|s| s.id == skill_id)
            .ok_or_else(|| format!("Unsupported operator skill definition: {key}"))?;
        let st = &raw["stats"];
        let mut base = Base::default();
        for (key, aliases, default, min) in [
            ("maxHp", &["maxHp", "hp"][..], 1000., 1.),
            ("atk", &["atk"][..], 0., 0.),
            ("def", &["def"][..], 0., 0.),
            ("res", &["magicResistance", "res", "resistance"][..], 0., 0.),
            (
                "aspd",
                &["attackSpeed", "aspd"][..],
                100.,
                f64::NEG_INFINITY,
            ),
            ("bat", &["baseAttackTime", "bat"][..], 1., 0.05),
            ("blockCnt", &["blockCnt", "block"][..], 1., 0.),
            ("cost", &["cost"][..], 10., 0.),
            ("respawnTime", &["respawnTime", "redeploy"][..], 70., 0.),
            (
                "spRecovery",
                &["spRecoveryPerSec", "spRecovery"][..],
                1.,
                f64::NEG_INFINITY,
            ),
            (
                "tauntLevel",
                &["tauntLevel", "taunt"][..],
                0.,
                f64::NEG_INFINITY,
            ),
            (
                "massLevel",
                &["massLevel", "mass"][..],
                0.,
                f64::NEG_INFINITY,
            ),
            (
                "hpRecoveryPerSec",
                &["hpRecoveryPerSec"][..],
                0.,
                f64::NEG_INFINITY,
            ),
        ] {
            let n = number(first(st, aliases), default).max(min);
            base.0
                .insert(key.into(), if key == "aspd" && n == 0. { 100. } else { n });
        }
        base.0.insert("moveSpeed".into(), 0.);
        let skill = &raw["skill"];
        let ranged = kit.ranged;
        let rule = selected.rule;
        let sp_type = selected.sp_type;
        if raw["position"] != kit.position
            || raw["profession"] != kit.profession
            || raw["subProfessionId"] != kit.branch
            || raw["immunities"]
                .as_object()
                .is_some_and(|o| o.values().any(|v| *v == true))
            || skill["spType"]
                != match sp_type {
                    skills::SpType::None => "ON_DEPLOY",
                    skills::SpType::Hurt => "INCREASE_WHEN_TAKEN_DAMAGE",
                    skills::SpType::Time => "INCREASE_WITH_TIME",
                    skills::SpType::Attack => "INCREASE_WHEN_ATTACK",
                }
            || skill["trigger"]["rule"] != rule
            || number(&skill["maxChargeTime"], 1.).max(1.).floor() > 64.
        {
            return Err(format!("Unsupported operator mechanics: {key}"));
        }
        for (field, expected) in [
            (
                "attackKind",
                Value::from(if kit.healing() {
                    "heal"
                } else if matches!(kit.branch, "phalanx" | "librator" | "bard") {
                    "none"
                } else if ranged {
                    "ranged"
                } else {
                    "melee"
                }),
            ),
            (
                "dmgType",
                Value::from(if kit.healing() || kit.branch == "bard" {
                    "heal"
                } else if kit.physical {
                    "phys"
                } else {
                    "arts"
                }),
            ),
            ("projectile", Value::from(kit.projectile)),
            ("canHitFly", Value::from(kit.can_hit_fly)),
        ] {
            if !raw[field].is_null() && raw[field] != expected {
                return Err(format!("Unsupported operator profile {field}: {key}"));
            }
        }
        if (!raw["targetPriority"].is_null()
            && !(raw["targetPriority"] == "fly" && kit.branch == "fastshot")
            && !(raw["targetPriority"] == "lowestDef" && kit.branch == "longrange"))
            || !raw["splashRadius"].is_null()
        {
            return Err(format!("Unsupported operator targeting: {key}"));
        }
        let grid = raw["rangeGrid"]
            .as_array()
            .ok_or("Missing operator attack range")?;
        if grid.is_empty() || grid.len() > super::grid::N {
            return Err("Invalid or unsupported operator range".into());
        }
        let mut range_grid = parse_range(&raw["rangeGrid"])?;
        if kit
            .required_range
            .is_some_and(|required| range_grid != required)
        {
            return Err("Unsupported operator range".into());
        }
        let content = kit.build(raw)?;
        if content.combat.normal_hits == 0
            || content.combat.normal_hits > Combat::MAX_HITS
            || content.combat.skill_hits == 0
            || content.combat.skill_hits > Combat::MAX_HITS
        {
            return Err(format!("Unsupported operator hit count: {key}"));
        }
        if let Some(grid) = &content.extra.effects.base_range {
            range_grid = grid.clone();
        }
        base.0.insert(
            "respawnTime".into(),
            (base.0["respawnTime"] + content.extra.effects.respawn_delta).max(0.),
        );
        let original_cost = base.0["cost"];
        base.0.insert(
            "cost".into(),
            (original_cost + content.extra.cost_delta).max(0.),
        );
        let has_modifiers = !content.modifiers.is_empty() || !content.skill_flags.is_empty();
        Ok(Self {
            no_skill: false,
            key: key.into(),
            original_cost,
            char_id: raw["charId"].as_str().unwrap_or("").into(),
            profession: kit.profession,
            laterano: raw["bonds"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x == "lateranoShip")),
            equipment: Vec::new(),
            max_charges: number(&skill["maxChargeTime"], 1.).floor().max(1.) as u32,
            base,
            modifiers: content.modifiers,
            permanent: content
                .permanent
                .into_iter()
                .chain(content.extra.effects.permanent.iter().cloned())
                .collect(),
            range_grid,
            combat: content.combat,
            hit: content.hit,
            attack_filter: content.attack_filter,
            skill_flags: content.skill_flags,
            death_heal: content.death_heal,
            incoming_hit: content.incoming_hit,
            attack_proc: content.attack_proc,
            dp_on_kill: content.dp_on_kill,
            initial_dp: content.initial_dp,
            trigger_range: content.trigger_range,
            skill_start: content.skill_start,
            grid_scale: content.grid_scale,
            while_blocked: content.while_blocked,
            ground_bonus: content.ground_bonus,
            aura: content.aura,
            element_attach: content.element_attach,
            undying: content.undying,
            end_stun: content.end_stun,
            melee_operator: kit.position == "MELEE",
            kind: selected.kind,
            spec: skills::Spec {
                duration: content
                    .extra
                    .duration
                    .unwrap_or_else(|| number(&skill["duration"], 0.)),
                activate_on_deploy: content.extra.deploy,
                ammo: content.extra.effects.ammo_count,
                init_sp: number(&skill["initSp"], 0.).max(0.),
                rule: content
                    .rule_override
                    .unwrap_or_else(|| skills::Rule::parse(rule)),
                attack: selected.attack_override,
                heal_skill: content.combat.heal
                    || content.combat.skill_heal
                    || content.extra.support.plasma.is_some()
                    || content.extra.support.bandage.is_some(),
                trigger_allies: content.combat.heal || content.combat.skill_heal,
                extra_ranges: matches!(
                    content.extra.tactics,
                    super::content::operators::Tactics::Mlyss(_)
                ),
                on_tick: !matches!(
                    content.extra.advanced,
                    super::content::operators::Advanced::None
                ) || !matches!(
                    content.extra.tactics,
                    super::content::operators::Tactics::None
                ) || content.extra.support.cook.is_some()
                    || !content.extra.support.peaks.is_empty()
                    || content.extra.support.phalanx.is_some()
                    || content
                        .extra
                        .support
                        .enemy_auras
                        .iter()
                        .any(|a| a.active_only),
                modifiers: has_modifiers,
                sp_type,
                manual: skill["skillType"]
                    .as_str()
                    .unwrap_or("MANUAL")
                    .eq_ignore_ascii_case("MANUAL"),
                ..Default::default()
            },
            cost: if content.extra.deploy {
                0.
            } else {
                number(&skill["spCost"], 0.).max(0.)
            },
            extra: content.extra,
        })
    }
}
#[derive(PartialEq)]
pub(crate) enum RangeGeometry<'a> {
    Absolute(Cow<'a, [usize]>),
    Relative {
        grid: Cow<'a, [[i32; 2]]>,
        origin: [i32; 2],
        direction: Direction,
        extend: u32,
        extra: Cow<'a, [usize]>,
    },
}
impl RangeGeometry<'_> {
    pub(crate) fn keys(&self) -> Vec<usize> {
        match self {
            Self::Absolute(keys) => keys.to_vec(),
            Self::Relative {
                grid,
                origin,
                direction,
                extend,
                extra,
            } => {
                let mut keys = super::targeting::absolute_range(grid, *origin, *direction, *extend);
                for &key in extra.iter() {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
                keys
            }
        }
    }
    pub(crate) fn owned(self) -> RangeGeometry<'static> {
        match self {
            Self::Absolute(keys) => RangeGeometry::Absolute(Cow::Owned(keys.into_owned())),
            Self::Relative {
                grid,
                origin,
                direction,
                extend,
                extra,
            } => RangeGeometry::Relative {
                grid: Cow::Owned(grid.into_owned()),
                origin,
                direction,
                extend,
                extra: Cow::Owned(extra.into_owned()),
            },
        }
    }
}

pub struct Entry {
    next_aura: f64,
    next_cost: f64,
    next_reveal: f64,
    pub definition: Arc<Definition>,
    pub direction: Direction,
    skill: Option<skills::Runtime>,
}
impl Entry {
    pub(crate) fn talent_sp_eligible(&self, active: bool) -> bool {
        if let Some(s) = &self.skill {
            !s.bank.no_skill
                && s.bank.kind != Kind::Passive
                && !(s.bank.active && s.bank.kind.timed())
        } else {
            self.definition.kind != Kind::Passive && !(active && self.definition.kind.timed())
        }
    }
    pub fn skill(&self) -> Result<&skills::Runtime, String> {
        self.skill
            .as_ref()
            .ok_or_else(|| "Operator skill is already executing".into())
    }
}
/// Default 0.3.3 normal-field deployment comparator. This follows the source
/// comparator, including descending row within a column, rather than its prose.
#[derive(Clone, Copy, Default)]
pub enum DeployOrder {
    #[default]
    Column,
    RowBottomUp,
}
pub struct Limits {
    pub units: usize,
    pub callback_depth: usize,
    pub initial_cooldown: f64,
    pub dp_max: f64,
    pub dp_initial: f64,
    pub deploy_order: DeployOrder,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            units: 36,
            callback_depth: 32,
            initial_cooldown: 3.,
            dp_max: 99.,
            dp_initial: 10.,
            deploy_order: DeployOrder::Column,
        }
    }
}
pub struct Placement {
    pub uid: u64,
    pub home: [i32; 2],
    pub direction: Direction,
}
pub struct Roster {
    entries: BTreeMap<Id, Entry>,
    order: Vec<Id>,
    kit_observers: Vec<(Id, Arc<Definition>)>,
    tick_observers: Vec<(Id, Arc<Definition>)>,
    deployment: Deployment,
    capacity: usize,
    depth: usize,
    initial_cooldown: f64,
    owner: Id,
    deploy_order: DeployOrder,
    faulted: bool,
}
impl Roster {
    pub fn new(grid: &Grid, owner: Id, limits: Limits) -> Result<Self, String> {
        let Limits {
            units: capacity,
            callback_depth: depth,
            initial_cooldown,
            dp_max,
            dp_initial,
            deploy_order,
        } = limits;
        if capacity == 0
            || capacity > 65536
            || depth == 0
            || depth > 128
            || !initial_cooldown.is_finite()
            || initial_cooldown < 0.
        {
            return Err("Invalid operator roster limits".into());
        }
        let mut deployment = Deployment::new(grid.rect(), capacity, 1, dp_max)?;
        deployment.set_dp(owner, dp_initial)?;
        Ok(Self {
            entries: BTreeMap::new(),
            order: vec![],
            kit_observers: vec![],
            tick_observers: vec![],
            deployment,
            capacity,
            depth,
            initial_cooldown,
            owner,
            deploy_order,
            faulted: false,
        })
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    fn healthy(&self) -> Result<(), String> {
        if self.faulted {
            Err("Operator roster is faulted".into())
        } else {
            Ok(())
        }
    }
    fn finish<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    pub fn ids(&self) -> &[Id] {
        &self.order
    }
    fn register_observer(&mut self, id: Id) {
        use super::runtime::kits::dispatch::Event;
        let def = &self.entries[&id].definition;
        if let Some(kit) = def.extra.tactics.behavior() {
            if kit.observes(&Event::Tick) {
                self.tick_observers.push((id, Arc::clone(def)));
            }
            self.kit_observers.push((id, Arc::clone(def)));
        }
    }
    pub(crate) fn kit_observers(&self, tick: bool) -> &[(Id, Arc<Definition>)] {
        if tick {
            &self.tick_observers
        } else {
            &self.kit_observers
        }
    }
    pub fn entry(&self, id: Id) -> Result<&Entry, String> {
        self.entries
            .get(&id)
            .ok_or_else(|| "Unknown operator identity".into())
    }
    /// Earliest scheduled callback. None denotes a per-frame mechanism without a quiet planner.
    pub(crate) fn quiet_deadline(&self, w: &World) -> Result<Option<f64>, String> {
        use super::content::operators::{Advanced, Tactics};
        let mut at = f64::INFINITY;
        for &id in &self.order {
            let entry = self.entry(id)?;
            let def = &entry.definition;
            let extra = &def.extra;
            let u = w.unit(id)?;
            let cfg = &extra.support;
            let fx = &extra.effects;
            if !matches!(extra.advanced, Advanced::None)
                || !matches!(extra.tactics, Tactics::None | Tactics::Indigo(_))
                || extra.hp_speed.is_some()
                || fx.condition.is_some()
                || def.while_blocked.as_ref().is_some_and(|effect| {
                    u.blocking().is_empty()
                        && u.stats
                            .buffs()
                            .iter()
                            .any(|b| b.key.as_deref() == Some(effect.key))
                })
                || u.operator.blessing_pending
                || !u.operator.advanced.locked.is_empty()
                || !u.operator.support.sp_changes.is_empty()
                || cfg.sp_near != 0.
                || cfg.reaper != 0.
            {
                return Ok(None);
            }
            if def.aura.is_some() {
                at = at.min(entry.next_aura);
            }
            if extra.cost_aura.is_some() {
                at = at.min(entry.next_cost);
            }
            if extra.reveal {
                at = at.min(entry.next_reveal);
            }
            if fx.aura.is_some() || fx.enemy_atk.is_some() {
                at = at.min(u.operator.next_pulse);
            }
            if let Some(ammo) = fx
                .ammo
                .as_ref()
                .filter(|a| a.ally > 0. && !u.operator.ammo_granted)
            {
                at = at.min(u.deployment.deployed_at + ammo.delay);
            }
            let support = &u.operator.support;
            if cfg.lavender > 0. {
                at = at.min(support.next_lavender);
            }
            if cfg.sp_aura > 0. || cfg.reveal {
                at = at.min(support.next_sp_aura);
            }
            if cfg.support_atk != 0. {
                at = at.min(support.next_support_aura);
            }
            if support.next_aura.len() != cfg.enemy_auras.len() {
                return Ok(None);
            }
            for (&next, aura) in support.next_aura.iter().zip(&cfg.enemy_auras) {
                if !aura.active_only {
                    at = at.min(next);
                }
            }
            if cfg.phalanx.is_some() {
                at = at.min(support.next_phalanx);
            }
            if cfg.block_stats.is_some() {
                at = at.min(support.next_block_stats.max(0.1));
            }
            if cfg.forge.is_some() {
                at = at.min(support.next_forge.max(0.2));
            }
        }
        Ok(Some(at))
    }
    pub(crate) fn deployment_mut(&mut self) -> &mut Deployment {
        &mut self.deployment
    }
    pub fn deployment(&self) -> &Deployment {
        &self.deployment
    }
    pub fn dp(&self, owner: Id) -> Option<f64> {
        self.deployment.dp(owner)
    }
    pub(crate) fn clear_inactive_sp(&mut self, w: &mut World, id: Id) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        if let Some(skill) = &mut entry.skill {
            return skill.bank.clear_inactive(w);
        }
        if entry.definition.kind != Kind::Passive && !w.unit(id)?.operator.active_skill {
            // The attack callback owns this skill. Preserve clear/gain order until it flushes.
            let limit = w
                .unit_capacity()
                .checked_mul(self.ids().len())
                .ok_or("SP change capacity overflow")?;
            let pending = &mut w.unit_mut(id)?.operator.support.sp_changes;
            if pending.len() >= limit {
                return Err("Pending SP change capacity exceeded".into());
            }
            pending.push(crate::battle::content::operators::SpChange::Clear);
        }
        Ok(())
    }
    pub(crate) fn take_skill(&mut self, id: Id) -> Result<skills::Runtime, String> {
        self.healthy()?;
        self.entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?
            .skill
            .take()
            .ok_or_else(|| "Operator skill is already executing".into())
    }
    pub(crate) fn restore_skill(&mut self, id: Id, skill: skills::Runtime) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        if entry.skill.is_some() || skill.unit() != id {
            self.faulted = true;
            return Err("Invalid operator skill restoration".into());
        }
        entry.skill = Some(skill);
        Ok(())
    }
    pub(crate) fn scheduled<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
    ) -> Result<(), String> {
        self.healthy()?;
        let needs_cost_aura = self.order.iter().any(|id| {
            let entry = &self.entries[id];
            entry.definition.extra.cost_aura.is_some() && entry.next_cost <= w.time + 1e-9
        });
        let allies: Vec<_> = if needs_cost_aura {
            self.order
                .iter()
                .map(|&id| (id, self.entries[&id].definition.original_cost))
                .collect()
        } else {
            Vec::new()
        };
        for &id in &self.order {
            let entry = self.entries.get_mut(&id).unwrap();
            if let Some(aura) = &entry.definition.aura {
                if entry.next_aura <= w.time + 1e-9 {
                    aura.refresh(w, r, h, id, entry.direction, entry.skill()?.bank.active)?;
                    entry.next_aura += 0.2;
                }
            }
            if let Some(aura) = &entry.definition.extra.cost_aura {
                if entry.next_cost <= w.time + 1e-9 {
                    aura.refresh(w, r, h, id, &allies)?;
                    entry.next_cost += 0.5;
                }
            }
            if entry.definition.extra.reveal && entry.next_reveal <= w.time + 1e-9 {
                entry.next_reveal += 0.2;
                if !w.unit(id)?.stats.alive || !w.unit(id)?.deployed {
                    continue;
                }
                let active = entry.skill()?.bank.active;
                let range = if active {
                    entry
                        .definition
                        .extra
                        .skill_range
                        .as_ref()
                        .unwrap_or(&entry.definition.range_grid)
                } else {
                    &entry.definition.range_grid
                };
                let keys = super::targeting::absolute_range(
                    range,
                    w.unit(id)?.deployment.tile,
                    entry.direction,
                    0,
                );
                let ids: Vec<_> = w.unit_ids().collect();
                for enemy in ids {
                    let u = w.unit_mut(enemy)?;
                    if u.side == Side::Enemy
                        && u.stats.alive
                        && !u.hidden
                        && u.stats
                            .stats()
                            .map_err(str::to_string)?
                            .flags
                            .contains("stealth")
                        && super::body::in_range(u, &keys)
                    {
                        h.operator_target_status(w, r, id, enemy, "reveal", 0.2 + 0.1)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn tick_effects<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
    ) -> Result<(), String> {
        self.healthy()?;
        for &id in &self.order {
            if let Some(effect) = &self.entries[&id].definition.while_blocked {
                let u = w.unit(id)?;
                let on = u.stats.alive && u.deployed && !u.blocking().is_empty();
                let has = u
                    .stats
                    .buffs()
                    .iter()
                    .any(|b| b.key.as_deref() == Some(effect.key));
                if on && !has {
                    r.add(
                        w,
                        h,
                        id,
                        buffs::Spec {
                            buff: Buff {
                                key: Some(effect.key.into()),
                                mods: effect.mods.clone(),
                                ..Default::default()
                            },
                            mods_present: true,
                            ..Default::default()
                        },
                    )?;
                } else if !on && has {
                    r.remove(w, h, id, effect.key)?;
                }
            }
        }
        for &id in &self.order {
            self.entries[&id].definition.extra.tick(w, r, h, id)?;
        }
        Ok(())
    }
    pub(crate) fn add_dp(&mut self, amount: f64) -> Result<(), String> {
        self.healthy()?;
        self.deployment.add_dp(self.owner, amount)
    }
    pub(crate) fn recover_dp(&mut self, dt: f64) -> Result<(), String> {
        self.healthy()?;
        let dp = (self
            .deployment
            .dp(self.owner)
            .ok_or("Missing solo deployment points")?
            + dt)
            .min(self.deployment.dp_max);
        self.deployment.set_dp(self.owner, dp)
    }
    pub(crate) fn drain_sp(&mut self, w: &mut World, id: Id, amount: f64) -> Result<(), String> {
        let skill = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?;
        super::element::drain_sp(w, Some(&mut skill.bank), amount)
    }
    /// The definition is compiled before mutation. Capacity/placement rejection
    /// leaves identity counters and canonical state untouched.
    pub fn add<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
        def: Definition,
        placement: Placement,
    ) -> Result<Id, String> {
        self.healthy()?;
        let Placement {
            uid,
            home,
            direction,
        } = placement;
        let owner = self.owner;
        let rect = grid.rect();
        if self.order.len() >= self.capacity
            || self
                .order
                .iter()
                .filter(|&&id| {
                    w.unit(id)
                        .is_ok_and(|u| u.deployment.kind == deploy::Kind::Operator)
                })
                .count()
                >= 36
            || home[0] < rect.r0
            || home[0] > rect.r1
            || home[1] < rect.c0
            || home[1] > rect.c1
            || self.order.iter().any(|id| {
                w.unit(*id).is_ok_and(|u| {
                    u.deployment.home == home
                        || (u.owner == Some(owner) && u.deployment.uid == Some(uid))
                })
            })
        {
            return Err("Operator capacity, duplicate identity or placement rejected".into());
        }
        let result = (|| {
            let id = w.next_unit_id()?;
            let mut bank = Bank::new(id, def.kind, def.cost, def.max_charges, 64)?;
            bank.no_skill = def.no_skill;
            let skill = skills::Runtime::new(bank, def.spec.clone(), self.depth)?;
            let mut u = Unit::new(UnitStats::new(def.base.clone()), Side::Ally);
            u.operator.tactical.reset(&def.extra.tactics);
            u.stats.alive = false;
            u.owner = Some(owner);
            u.skill = true;
            u.profile_no_heal = def.extra.no_heal;
            u.targeting.melee_operator = def.melee_operator;
            u.targeting.enemy_filter = def
                .extra
                .tactics
                .behavior()
                .and_then(|kit| kit.enemy_filter());
            u.deployment.kind = deploy::Kind::Operator;
            u.deployment.uid = Some(uid);
            u.deployment.home = home;
            u.deployment.tile = home;
            u.position = [home[0] as f64, home[1] as f64];
            w.insert_unit(id, u)?;
            self.deployment.register(id, w)?;
            // Earlier Kits already granted their team effects before this
            // operator's own talents were installed in the reference runtime.
            for &source in &self.order {
                if let Some(team) = &self.entries[&source].definition.extra.effects.team {
                    if team.characters.is_empty() || team.characters.contains(&def.char_id.as_str())
                    {
                        team.apply(w, r, h, source, id)?;
                    }
                }
            }
            for buff in def.permanent.iter().cloned() {
                r.add(
                    w,
                    h,
                    id,
                    buffs::Spec {
                        buff,
                        allow_dead: true,
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
            if let Some(team) = &def.extra.effects.team {
                for &target in &self.order {
                    if team.characters.is_empty()
                        || team
                            .characters
                            .contains(&self.entries[&target].definition.char_id.as_str())
                    {
                        team.apply(w, r, h, id, target)?;
                    }
                }
                if team.characters.is_empty() || team.characters.contains(&def.char_id.as_str()) {
                    team.apply(w, r, h, id, id)?;
                }
            }
            self.entries.insert(
                id,
                Entry {
                    next_aura: 0.,
                    next_cost: 0.,
                    next_reveal: 0.2,
                    definition: Arc::new(def),
                    direction,
                    skill: Some(skill),
                },
            );
            self.order.push(id);
            self.register_observer(id);
            Ok(id)
        })();
        self.finish(result)
    }
    pub(crate) fn spawn_token<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
        def: Definition,
        summoner: Id,
        tile: [i32; 2],
        direction: Direction,
    ) -> Result<Option<Id>, String> {
        self.healthy()?;
        if !grid.in_rect(tile) || self.deployment.reserved(w, tile)? {
            return Ok(None);
        }
        if self.order.len() >= self.capacity {
            return Err("Summon roster capacity exceeded".into());
        }
        let result = (|| {
            let id = w.next_unit_id()?;
            let mut bank = Bank::new(id, def.kind, def.cost, def.max_charges, 64)?;
            bank.no_skill = def.no_skill;
            let skill = skills::Runtime::new(bank, def.spec.clone(), self.depth)?;
            let mut unit = Unit::new(UnitStats::new(def.base.clone()), Side::Ally);
            unit.stats.alive = false;
            unit.owner = w.unit(summoner)?.owner;
            unit.summoner = (def.key != "enemy_9012_acloon").then_some(summoner);
            unit.skill = true;
            unit.profile_no_heal = def.extra.no_heal;
            unit.targeting.melee_operator = def.melee_operator;
            unit.deployment.kind = deploy::Kind::Token;
            unit.deployment.home = tile;
            unit.deployment.tile = tile;
            unit.position = [tile[0] as f64, tile[1] as f64];
            unit.operator.tactical.reset(&def.extra.tactics);
            w.insert_unit(id, unit)?;
            self.deployment.register(id, w)?;
            for buff in def.permanent.iter().cloned() {
                r.add(
                    w,
                    h,
                    id,
                    buffs::Spec {
                        buff,
                        allow_dead: true,
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
            self.entries.insert(
                id,
                Entry {
                    next_aura: 0.,
                    next_cost: 0.,
                    next_reveal: 0.2,
                    definition: Arc::new(def),
                    direction,
                    skill: Some(skill),
                },
            );
            self.order.push(id);
            self.register_observer(id);
            let deployed = self.deployment.deploy(
                w,
                &mut Deploy {
                    entries: &mut self.entries,
                    r,
                    h,
                    grid: Some(grid.tiles()),
                    initial_cooldown: self.initial_cooldown,
                },
                id,
                deploy::Options::default(),
            )?;
            if !deployed {
                return Err("Validated summon deployment rejected".into());
            }
            Ok(Some(id))
        })();
        self.finish(result)
    }
    pub(crate) fn relocate<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
        id: Id,
        tile: [i32; 2],
    ) -> Result<bool, String> {
        let moved = self.deployment.relocate(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: Some(grid.tiles()),
                initial_cooldown: self.initial_cooldown,
            },
            id,
            tile,
        )?;
        if moved {
            h.field_deploy(w, r, id, false)?;
        }
        Ok(moved)
    }
    pub(crate) fn initial_order(&self, w: &World) -> Result<Vec<Id>, String> {
        self.healthy()?;
        let mut order = self.order.clone();
        // Input/unit update order is preserved separately from deployment order.
        let mut homes = BTreeMap::new();
        for &id in &order {
            homes.insert(
                id,
                (
                    w.unit(id)?.deployment.kind == deploy::Kind::Token,
                    w.unit(id)?.deployment.home,
                ),
            );
        }
        order.sort_by(|a, b| {
            let (a_token, [ar, ac]) = homes[a];
            let (b_token, [br, bc]) = homes[b];
            a_token
                .cmp(&b_token)
                .then_with(|| match self.deploy_order {
                    DeployOrder::Column => ac.cmp(&bc).then(br.cmp(&ar)),
                    DeployOrder::RowBottomUp => ar.cmp(&br).then(ac.cmp(&bc)),
                })
                .then(a.cmp(b))
        });
        Ok(order)
    }
    pub(crate) fn deploy_initial_unit<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
        id: Id,
    ) -> Result<(), String> {
        self.healthy()?;
        if w.unit(id)?.deployed {
            return Ok(());
        }

        let result = self.deployment.deploy(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: Some(grid.tiles()),
                initial_cooldown: self.initial_cooldown,
            },
            id,
            deploy::Options {
                initial: true,
                ..Default::default()
            },
        );
        self.finish(result).and_then(|deployed| {
            if deployed {
                Ok(())
            } else {
                Err("Initial operator deployment rejected".into())
            }
        })
    }
    pub fn deploy_initial<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
    ) -> Result<(), String> {
        let order = self.initial_order(w)?;
        let mut dispatch = Deploy {
            entries: &mut self.entries,
            r,
            h,
            grid: Some(grid.tiles()),
            initial_cooldown: self.initial_cooldown,
        };
        let result = (|| {
            for id in order {
                if !self.deployment.deploy(
                    w,
                    &mut dispatch,
                    id,
                    deploy::Options {
                        initial: true,
                        ..Default::default()
                    },
                )? {
                    return Err("Initial operator deployment rejected".into());
                }
            }
            Ok(())
        })();
        self.finish(result)
    }
    pub fn damaged<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        self.healthy()?;
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        if entry.skill.is_none() {
            if entry.definition.spec.sp_type == skills::SpType::Hurt
                || entry.definition.spec.rule == skills::Rule::TakeDamage
            {
                let limit = w
                    .unit_capacity()
                    .checked_mul(self.order.len())
                    .ok_or("SP change capacity overflow")?;
                let pending = &mut w.unit_mut(id)?.operator.support.sp_changes;
                if pending.len() >= limit {
                    return Err("Pending SP change capacity exceeded".into());
                }
                pending.push(crate::battle::content::operators::SpChange::Damaged);
            }
            return Ok(());
        }
        let result = entry
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?
            .on_damaged(
                w,
                &mut Skill {
                    r,
                    h,
                    definition: &entry.definition,
                    aura: entry.definition.aura.as_ref(),
                    direction: entry.direction,
                    end_stun: entry.definition.end_stun,
                    mods: &entry.definition.modifiers,
                    flags: &entry.definition.skill_flags,
                },
            );
        self.finish(result)
    }
    pub fn grant_sp<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
        amount: f64,
    ) -> Result<f64, String> {
        self.healthy()?;
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        let result = entry
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?
            .gain(
                w,
                &mut Skill {
                    r,
                    h,
                    definition: &entry.definition,
                    aura: entry.definition.aura.as_ref(),
                    direction: entry.direction,
                    end_stun: entry.definition.end_stun,
                    mods: &entry.definition.modifiers,
                    flags: &entry.definition.skill_flags,
                },
                amount,
                "talent",
                false,
                false,
            );
        self.finish(result)
    }
    pub fn tick_skills<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        dt: f64,
    ) -> Result<(), String> {
        self.healthy()?;
        if !dt.is_finite() || dt < 0. {
            return Err("Invalid operator tick duration".into());
        }
        let result = (|| {
            for &id in &self.order {
                if !w.unit(id)?.stats.alive || !w.unit(id)?.deployed {
                    continue;
                }
                let entry = self.entries.get_mut(&id).unwrap();
                entry
                    .skill
                    .as_mut()
                    .ok_or("Operator skill is already executing")?
                    .tick(
                        w,
                        &mut Skill {
                            r,
                            h,
                            definition: &entry.definition,
                            aura: entry.definition.aura.as_ref(),
                            direction: entry.direction,
                            end_stun: entry.definition.end_stun,
                            mods: &entry.definition.modifiers,
                            flags: &entry.definition.skill_flags,
                        },
                        dt,
                    )?;
            }
            Ok(())
        })();
        self.finish(result)
    }
    pub fn kill<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
        credit: Option<Id>,
    ) -> Result<(), String> {
        self.healthy()?;
        self.entry(id)?;
        let result = self.deployment.kill(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: None,
                initial_cooldown: self.initial_cooldown,
            },
            id,
            credit,
        );
        self.finish(result)
    }
    pub fn retreat<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        self.healthy()?;
        self.entry(id)?;
        let result = self.deployment.retreat(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: None,
                initial_cooldown: self.initial_cooldown,
            },
            id,
            "retreat",
            false,
        );
        self.finish(result)
    }
    pub(crate) fn copy_token(
        &mut self,
        data: &Catalog,
        w: &mut World,
        id: Id,
        source: Id,
        scale: f64,
        ranged: bool,
    ) -> Result<(), String> {
        let source_definition = Arc::clone(&self.entry(source)?.definition);
        let key = self.entry(id)?.definition.key.clone();
        let owner = w.unit(id)?.summoner.ok_or("Copy has no owner")?;
        let owner_key = &self.entry(owner)?.definition.key;
        let mut definition = Definition::token(data, &key, owner_key)?;
        for field in ["maxHp", "atk", "def", "res", "blockCnt", "bat", "aspd"] {
            let mut value = w.unit(source)?.stats.base_number(field);
            if matches!(field, "maxHp" | "atk" | "def" | "res") {
                value *= scale;
            }
            if field == "maxHp" {
                value = value.max(1.);
            }
            definition.base.0.insert(field.into(), value);
        }
        let copied = definition.base.0.clone();
        w.unit_mut(id)?.stats.edit_base(|base| {
            for (field, value) in copied {
                base.0.insert(field, value);
            }
        });
        definition.range_grid = source_definition.range_grid.clone();
        definition.melee_operator = !ranged;
        definition.combat.profile.selection.can_hit_fly =
            ranged || source_definition.combat.profile.selection.can_hit_fly;
        definition.combat.profile.no_attack = false;
        definition.combat.projectile = if ranged {
            Some(match source_definition.combat.projectile {
                Some("arrow") => "arrow",
                Some("bomb") => "bomb",
                Some("lob") => "lob",
                _ => "bolt",
            })
        } else {
            None
        };
        definition.combat.projectile_speed = match definition.combat.projectile {
            Some("arrow") => 14.,
            Some("bomb" | "lob") => 8.,
            Some(_) => 11.,
            None => 0.,
        };
        if !source_definition.combat.heal
            && !source_definition.combat.profile.no_attack
            && !source_definition.combat.profile.only_while_skill
        {
            definition.combat.damage = source_definition.combat.damage;
        }
        self.entries.get_mut(&id).unwrap().definition = Arc::new(definition);
        // Definition replacement is rare. Rebuild in roster order so equal-priority
        // listeners retain their original ordering, including copied summons.
        self.kit_observers.clear();
        self.tick_observers.clear();
        for cursor in 0..self.order.len() {
            self.register_observer(self.order[cursor]);
        }
        w.unit_mut(id)?.targeting.melee_operator = !ranged;
        let maximum = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("maxHp");
        w.unit_mut(id)?.stats.hp = maximum;
        Ok(())
    }
    pub(crate) fn disable_token_skill(&mut self, id: Id) -> Result<(), String> {
        let skill = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown clone")?
            .skill
            .as_mut()
            .ok_or("Clone skill is executing")?;
        skill.bank.no_skill = true;
        skill.bank.sp = 0.;
        skill.bank.charges = 0.;
        skill.bank.active = false;
        Ok(())
    }
    pub(crate) fn redeploy_free<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        tiles: &[super::grid::Tile],
        id: Id,
    ) -> Result<bool, String> {
        self.redeploy_at(w, r, h, tiles, id, None, false)
    }
    pub(crate) fn redeploy_at<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        tiles: &[super::grid::Tile],
        id: Id,
        tile: Option<[i32; 2]>,
        keep_sp: bool,
    ) -> Result<bool, String> {
        let result = self.deployment.redeploy_retaining(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: Some(tiles),
                initial_cooldown: self.initial_cooldown,
            },
            id,
            true,
            tile,
            keep_sp,
        );
        self.finish(result)
    }
    pub(crate) fn expire_token<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
    ) -> Result<(), String> {
        self.healthy()?;
        let result = self.deployment.retreat(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: None,
                initial_cooldown: self.initial_cooldown,
            },
            id,
            "expired",
            true,
        );
        self.finish(result)
    }
    pub fn check_redeploys<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
    ) -> Result<(), String> {
        self.healthy()?;
        let result = self.deployment.check_redeploys(
            w,
            &mut Deploy {
                entries: &mut self.entries,
                r,
                h,
                grid: Some(grid.tiles()),
                initial_cooldown: self.initial_cooldown,
            },
        );
        self.finish(result)
    }
    pub fn range(&self, w: &World, id: Id) -> Result<Vec<usize>, String> {
        Ok(self.range_geometry(w, id)?.keys())
    }
    pub(crate) fn range_geometry<'a>(
        &'a self,
        w: &'a World,
        id: Id,
    ) -> Result<RangeGeometry<'a>, String> {
        let entry = self.entry(id)?;
        if matches!(
            entry.definition.extra.tactics,
            super::content::operators::Tactics::Yanyou(_)
        ) {
            return Ok(RangeGeometry::Absolute(Cow::Borrowed(
                &w.unit(id)?.operator.tactical.yanyou()?.keys,
            )));
        }
        let active = entry
            .skill
            .as_ref()
            .map(|s| s.bank.active)
            .unwrap_or_else(|| w.unit(id).is_ok_and(|u| u.operator.active_skill));
        let doll_grid = match &entry.definition.extra.tactics {
            super::content::operators::Tactics::Kazema(parameters)
                if w.unit(id)?.operator.tactical.kazema()?.dollkeeper.doll =>
            {
                Some(&parameters.grid)
            }
            _ => None,
        };
        let grid = if let Some(grid) = doll_grid {
            grid
        } else if active {
            entry
                .definition
                .extra
                .skill_range
                .as_ref()
                .unwrap_or(&entry.definition.range_grid)
        } else {
            &entry.definition.range_grid
        };
        let extend = if active {
            entry.definition.extra.advanced_range_extend
        } else {
            0
        };
        let unit = w.unit(id)?;
        Ok(RangeGeometry::Relative {
            grid: Cow::Borrowed(grid),
            origin: unit.deployment.tile,
            direction: entry.direction,
            extend,
            extra: Cow::Borrowed(&unit.operator.advanced.extra_range),
        })
    }
    pub(crate) fn base_range(&self, w: &World, id: Id) -> Result<Vec<usize>, String> {
        let entry = self.entry(id)?;
        Ok(super::targeting::absolute_range(
            &entry.definition.range_grid,
            w.unit(id)?.deployment.tile,
            entry.direction,
            0,
        ))
    }
}
struct Skill<'a, H> {
    r: &'a mut buffs::Runtime,
    h: &'a mut H,
    definition: &'a Definition,
    aura: Option<&'a super::content::operators::Aura>,
    direction: Direction,
    end_stun: f64,
    mods: &'a Numbers,
    flags: &'a std::collections::BTreeSet<String>,
}
impl<H: buffs::Hooks> skills::Hooks for Skill<'_, H> {
    fn event(
        &mut self,
        e: skills::Event,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
        w: &mut World,
    ) -> Result<(), String> {
        match e {
            skills::Event::ApplyModifiers => {
                self.r.add(
                    w,
                    self.h,
                    s.unit(),
                    buffs::Spec {
                        buff: Buff {
                            key: Some(format!("skill:{}", s.unit())),
                            mods: self.mods.clone(),
                            flags: self.flags.clone(),
                            ..Default::default()
                        },
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
                if self.flags.contains("noBlock") {
                    self.h.operator_release(w, self.r, s.unit())?;
                }
            }
            skills::Event::RemoveModifiers => {
                self.r
                    .remove(w, self.h, s.unit(), &format!("skill:{}", s.unit()))?;
            }
            skills::Event::Deploy
            | skills::Event::Start
            | skills::Event::End
            | skills::Event::SkillStart
            | skills::Event::SkillEnd
            | skills::Event::DisplayOn
            | skills::Event::DisplayOff => {}
            skills::Event::PassiveWindow | skills::Event::AmmoUsed | skills::Event::Tick => {}
            _ => return Err("Unexpected callback in default operator kit".into()),
        }
        self.definition.extra.event(w, self.r, self.h, e, s)?;
        self.h
            .operator_skill_effect(w, self.r, self.direction, self.definition, e, c, s)?;
        if e == skills::Event::End
            && c.reason != "death"
            && w.unit(s.unit())?.stats.alive
            && self.end_stun > 0.
        {
            self.h
                .operator_status(w, self.r, s.unit(), "stun", self.end_stun)?;
        }
        if matches!(e, skills::Event::Start | skills::Event::End) {
            if let Some(aura) = self.aura {
                aura.refresh(w, self.r, self.h, s.unit(), self.direction, s.bank.active)?;
            }
        }
        Ok(())
    }
    fn condition(
        &mut self,
        _: skills::Condition,
        _: &skills::Runtime,
        _: &mut World,
    ) -> Result<bool, String> {
        Err("Default TAKE_DAMAGE kit has no geometric trigger".into())
    }
    fn profile(&mut self, _: Id, _: &mut World) -> Result<Option<AttackProfile>, String> {
        Ok(Some(AttackProfile::default()))
    }
    fn has_sp_hook(&self) -> bool {
        false
    }
}
struct Deploy<'a, H> {
    entries: &'a mut BTreeMap<Id, Entry>,
    r: &'a mut buffs::Runtime,
    h: &'a mut H,
    grid: Option<&'a [super::grid::Tile]>,
    initial_cooldown: f64,
}
impl<H: buffs::Hooks> blocking::Hooks for Deploy<'_, H> {
    fn blocked(&mut self, _: Id, _: Id, _: &mut World) -> Result<(), String> {
        Ok(())
    }
    fn ended(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        blocking::restore_stealth(w, self.r, self.h, id)
    }
}
impl<H: buffs::Hooks> deploy::Hooks for Deploy<'_, H> {
    fn initialize(
        &mut self,
        id: Id,
        opts: deploy::Options,
        w: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.element_buildup = [0.; 5];
        let [row, col] = u.deployment.tile;
        u.ground = self.grid.ok_or("Deployment requires tile context")?
            [row as usize * COLS + col as usize]
            .height
            == "LOW";
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        w.unit_mut(id)?.terrain.direction = Some(entry.direction);
        let skill = entry
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?;
        w.unit_mut(id)?.operator.active_skill = false;
        w.unit_mut(id)?.operator.skill_death_pending = false;
        w.unit_mut(id)?.operator.hits = 0;
        {
            let m = &mut w.unit_mut(id)?.operator.support;
            m.protege = None;
            m.debt = 0.;
            m.fragile_scale = 1.;
            m.reaper_queue.clear();
            m.next_reaper = f64::NEG_INFINITY;
            m.reaper_window = f64::NEG_INFINITY;
            m.reaper_accepted = 0;
            m.quiet_until = f64::NEG_INFINITY;
            m.bandage = None;
            m.sp_changes.clear();
        }
        let next_fast = w.unit(id)?.operator.advanced.next_fast;
        let next_aura = w.unit(id)?.operator.advanced.next_aura;
        let floating = std::mem::take(&mut w.unit_mut(id)?.operator.advanced.floating);
        w.unit_mut(id)?.operator.advanced = Default::default();
        w.unit_mut(id)?.operator.advanced.next_fast = next_fast;
        w.unit_mut(id)?.operator.advanced.next_aura = next_aura;
        w.unit_mut(id)?.operator.advanced.floating = floating;
        w.unit_mut(id)?.operator.advanced.last_hurt = f64::NEG_INFINITY;
        match &entry.definition.extra.advanced {
            super::content::operators::Advanced::Mudrok(p) => {
                w.unit_mut(id)?.operator.advanced.layers = p.layer_add.min(p.layer_cap);
                w.unit_mut(id)?.operator.advanced.next_layer = w.time + p.layer_period;
            }
            _ => {}
        }
        w.unit_mut(id)?
            .operator
            .tactical
            .reset(&entry.definition.extra.tactics);
        w.unit_mut(id)?.hits_sleep = entry.definition.combat.profile.selection.hit_sleep;
        w.unit_mut(id)?.operator.uses = 0;
        w.unit_mut(id)?.operator.revived = false;
        w.unit_mut(id)?.operator.ammo_granted = false;
        w.unit_mut(id)?.operator.blessing_pending =
            entry.definition.extra.effects.random_bless.is_some();
        if w.unit(id)?.operator.deployments == 0 {
            w.unit_mut(id)?.operator.next_pulse =
                if entry.definition.extra.effects.enemy_atk.is_some() {
                    0.1
                } else if entry
                    .definition
                    .extra
                    .effects
                    .aura
                    .as_ref()
                    .is_some_and(|a| a.grid.is_none())
                {
                    0.2
                } else {
                    0.
                };
        }
        if let Some(b) = entry.definition.extra.effects.deploy_buff.clone() {
            self.r.add(
                w,
                self.h,
                id,
                buffs::Spec {
                    buff: b,
                    ..Default::default()
                },
            )?;
        }
        let carry = if opts.keep_sp {
            Some((skill.bank.sp, skill.bank.charges))
        } else {
            None
        };
        skill.reset(
            w,
            &mut Skill {
                r: self.r,
                h: self.h,
                definition: &entry.definition,
                aura: entry.definition.aura.as_ref(),
                direction: entry.direction,
                end_stun: entry.definition.end_stun,
                mods: &entry.definition.modifiers,
                flags: &entry.definition.skill_flags,
            },
            None,
        )?;
        if let Some((sp, charges)) = carry {
            let cost = skill.bank.cost(w)?;
            if !skill.bank.no_skill
                && skill.bank.kind != super::sp::Kind::Passive
                && !skill.bank.active
                && cost > 0.
            {
                skill.bank.charges = charges.floor().clamp(0., skill.bank.max_charges() as f64);
                skill.bank.sp = if skill.bank.charges >= skill.bank.max_charges() as f64 {
                    cost
                } else {
                    sp.clamp(0., cost)
                };
            }
        }
        if let Some(p) = &entry.definition.extra.support.phalanx {
            self.r.add(
                w,
                self.h,
                id,
                buffs::Spec {
                    buff: Buff {
                        key: Some("trait:phalanxGuard".into()),
                        mods: p.guard.clone(),
                        ..Default::default()
                    },
                    mods_present: true,
                    ..Default::default()
                },
            )?;
        }
        if opts.initial {
            skill.op_ready_at = w.time + self.initial_cooldown;
        }
        if let Some(bonus) = &entry.definition.ground_bonus {
            let grid = self.grid.ok_or("Deployment requires grid context")?;
            let home = w.unit(id)?.deployment.tile;
            let low = [[1, 0], [-1, 0], [0, 1], [0, -1]]
                .into_iter()
                .filter(|d| {
                    let p = [home[0] + d[0], home[1] + d[1]];
                    Grid::in_bounds(p) && grid[p[0] as usize * COLS + p[1] as usize].height == "LOW"
                })
                .count();
            let v = if low as f64 >= bonus.count {
                bonus.surrounded
            } else {
                bonus.normal
            };
            if v != 0. {
                self.r.add(
                    w,
                    self.h,
                    id,
                    buffs::Spec {
                        buff: Buff {
                            key: Some(bonus.key.into()),
                            mods: [("atkPct".into(), v)].into(),
                            ..Default::default()
                        },
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
        }
        Ok(())
    }
    fn event(
        &mut self,
        e: deploy::Event,
        id: Id,
        _: Option<Id>,
        w: &mut World,
        deployment: &mut Deployment,
    ) -> Result<(), String> {
        if e == deploy::Event::Death {
            self.h.operator_removed(w, self.r, id)?;
        }
        if e == deploy::Event::Deploy {
            self.h.field_deploy(w, self.r, id, true)?;
            self.team_deploy(id, w)?;
            let entry = self.entries.get_mut(&id).unwrap();
            self.h.operator_skill_effect(
                w,
                self.r,
                entry.direction,
                &entry.definition,
                skills::Event::Deploy,
                &mut skills::Context {
                    reason: "deploy".into(),
                    dt: 0.,
                    amount: 0.,
                    no_ammo: false,
                    targets: vec![],
                },
                entry.skill.as_mut().ok_or("Deployment lost its skill")?,
            )?;

            let refund = self.entries[&id].definition.extra.support.refund;
            let u = w.unit(id)?;
            if refund > 0. && u.operator.deployments > 1 && !u.ground {
                deployment.add_dp(
                    u.owner.ok_or("Missing operator owner")?,
                    u.stats.base_number("cost") * refund,
                )?;
            }
        }
        if e == deploy::Event::EndSkill {
            let m = &mut w.unit_mut(id)?.operator.support;
            m.reaper_queue.clear();
            m.next_reaper = f64::NEG_INFINITY;
            m.reaper_window = f64::NEG_INFINITY;
            m.reaper_accepted = 0;
            m.sp_changes.clear();
            let entry = self
                .entries
                .get_mut(&id)
                .ok_or("Unknown operator identity")?;
            if let Some(skill) = entry.skill.as_mut() {
                skill.end(
                    w,
                    &mut Skill {
                        r: self.r,
                        h: self.h,
                        definition: &entry.definition,
                        aura: entry.definition.aura.as_ref(),
                        direction: entry.direction,
                        end_stun: entry.definition.end_stun,
                        mods: &entry.definition.modifiers,
                        flags: &entry.definition.skill_flags,
                    },
                    "death",
                )?;
            } else {
                w.unit_mut(id)?.operator.skill_death_pending = true;
            }
        }
        Ok(())
    }
    fn remove_temporary_effects(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        self.r.discard_temporary(w, self.h, id)
    }
}

impl<H: buffs::Hooks> Deploy<'_, H> {
    fn team_deploy(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        let u = w.unit_mut(id)?;
        u.operator.deployments = u
            .operator
            .deployments
            .checked_add(1)
            .ok_or("Operator deployment count exhausted")?;
        if u.operator.cost_cut > 0. {
            let cost = u.stats.base_number("cost") + u.operator.cost_cut;
            u.stats.edit_base(|b| {
                b.0.insert("cost".into(), cost);
            });
            u.operator.cost_cut = 0.;
        }
        if let Some(cut) = &self.entries[&id].definition.extra.effects.cost_cut {
            if cut.every || w.unit(id)?.operator.deployments == 1 {
                for (&other, entry) in self.entries.iter() {
                    let u = w.unit_mut(other)?;
                    if other == id
                        || u.stats.alive
                        || u.removed
                        || entry.definition.profession != "WARRIOR"
                    {
                        continue;
                    }
                    let cost = u.stats.base_number("cost");
                    let value = cut.amount.min(cut.cap - u.operator.cost_cut).min(cost);
                    if value > 0. {
                        u.stats.edit_base(|b| {
                            b.0.insert("cost".into(), cost - value);
                        });
                        u.operator.cost_cut += value;
                    }
                }
            }
        }
        if self.entries[&id].definition.profession == "PIONEER" {
            let amount: f64 = self
                .entries
                .values()
                .map(|e| e.definition.extra.effects.deploy_sp)
                .sum();
            let e = self.entries.get_mut(&id).unwrap();
            let s = e.skill.as_mut().ok_or("Missing deployment skill")?;
            if !(s.bank.active && s.bank.kind.timed()) && amount > 0. {
                s.gain(
                    w,
                    &mut Skill {
                        r: self.r,
                        h: self.h,
                        definition: &e.definition,
                        aura: e.definition.aura.as_ref(),
                        direction: e.direction,
                        end_stun: e.definition.end_stun,
                        mods: &e.definition.modifiers,
                        flags: &e.definition.skill_flags,
                    },
                    amount,
                    "init",
                    false,
                    false,
                )?;
            }
        }
        Ok(())
    }
}
