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
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct Definition {
    pub key: String,
    base: Base,
    modifiers: Numbers,
    permanent: Option<Buff>,
    range_grid: Vec<[i32; 2]>,
    pub(crate) combat: Combat,
    pub(crate) hit: super::content::operators::HitHandler,
    kind: Kind,
    spec: skills::Spec,
    cost: f64,
}
#[derive(Clone, Copy)]
pub(crate) struct Combat {
    pub profile: AttackProfile,
    pub damage: super::damage::DamageType,
    pub projectile: Option<&'static str>,
    pub projectile_speed: f64,
    pub area: super::attacks::Area,
    pub skill_area: super::attacks::Area,
    pub sluggish: f64,
    pub skill_sluggish: f64,
}
impl Default for Combat {
    fn default() -> Self {
        Self {
            profile: AttackProfile::default(),
            damage: super::damage::DamageType::Physical,
            projectile: None,
            projectile_speed: 0.,
            area: Default::default(),
            skill_area: Default::default(),
            sluggish: 0.,
            skill_sluggish: 0.,
        }
    }
}
impl Definition {
    pub(crate) fn skill_event<H: buffs::Hooks>(
        &self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        e: skills::Event,
        c: &mut skills::Context,
        s: &mut skills::Runtime,
    ) -> Result<(), String> {
        skills::Hooks::event(
            &mut Skill {
                r,
                h,
                mods: &self.modifiers,
            },
            e,
            c,
            s,
            w,
        )
    }
    pub fn load(data: &Catalog, key: &str) -> Result<Self, String> {
        let raw = &data.table("chess").ok_or("Missing chess data")?[key];
        Self::compile(key, raw)
    }
    /// Defaults only: no selected skill/module, potential, items or custom data
    /// composition is inferred. Raw records already contain default module stats.
    pub fn compile(key: &str, raw: &Value) -> Result<Self, String> {
        let kit = Kit::resolve(key)?;
        let (skill_id, profession, branch) = kit.identity();
        if !raw.is_object() || raw["skill"]["skillId"] != skill_id {
            return Err(format!("Unsupported default operator definition: {key}"));
        }
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
        let ranged = kit.ranged();
        let rule = if ranged { "DEFAULT" } else { "TAKE_DAMAGE" };
        if raw["position"] != if ranged { "RANGED" } else { "MELEE" }
            || raw["profession"] != profession
            || raw["subProfessionId"] != branch
            || raw["immunities"]
                .as_object()
                .is_some_and(|o| o.values().any(|v| *v == true))
            || skill["spType"]
                != if kit.passive() {
                    "ON_DEPLOY"
                } else {
                    "INCREASE_WITH_TIME"
                }
            || skill["trigger"]["rule"] != rule
            || number(&skill["maxChargeTime"], 1.).max(1.).floor() != 1.
        {
            return Err(format!("Unsupported operator mechanics: {key}"));
        }
        for (field, expected) in [
            (
                "attackKind",
                Value::from(if ranged { "ranged" } else { "melee" }),
            ),
            (
                "dmgType",
                Value::from(if kit.physical() { "phys" } else { "arts" }),
            ),
            ("projectile", Value::from(kit.projectile())),
            ("canHitFly", Value::from(ranged)),
        ] {
            if !raw[field].is_null() && raw[field] != expected {
                return Err(format!("Unsupported operator profile {field}: {key}"));
            }
        }
        if !raw["targetPriority"].is_null() || !raw["splashRadius"].is_null() {
            return Err(format!("Unsupported operator targeting: {key}"));
        }
        let grid = raw["rangeGrid"]
            .as_array()
            .ok_or("Missing operator attack range")?;
        if grid.is_empty()
            || grid.len() > super::grid::N
            || (kit == Kit::Yak && raw["rangeGrid"] != serde_json::json!([[0, 0]]))
        {
            return Err("Invalid or unsupported operator range".into());
        }
        let range_grid = grid
            .iter()
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
            .collect::<Result<Vec<_>, String>>()?;
        let content = kit.build(raw);
        let has_modifiers = !content.modifiers.is_empty();
        Ok(Self {
            key: key.into(),
            base,
            modifiers: content.modifiers,
            permanent: content.permanent,
            range_grid,
            combat: content.combat,
            hit: content.hit,
            kind: if kit.passive() {
                Kind::Passive
            } else {
                Kind::Duration
            },
            spec: skills::Spec {
                duration: number(&skill["duration"], 0.),
                init_sp: number(&skill["initSp"], 0.).max(0.),
                rule: skills::Rule::parse(rule),
                attack: kit.attack_override(),
                modifiers: has_modifiers,
                sp_type: if kit.passive() {
                    skills::SpType::None
                } else {
                    skills::SpType::Time
                },
                manual: skill["skillType"]
                    .as_str()
                    .unwrap_or("MANUAL")
                    .eq_ignore_ascii_case("MANUAL"),
                ..Default::default()
            },
            cost: number(&skill["spCost"], 0.).max(0.),
        })
    }
}
pub struct Entry {
    pub definition: Arc<Definition>,
    pub direction: Direction,
    skill: Option<skills::Runtime>,
}
impl Entry {
    pub fn skill(&self) -> Result<&skills::Runtime, String> {
        self.skill
            .as_ref()
            .ok_or("Operator skill is already executing".into())
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
    pub fn entry(&self, id: Id) -> Result<&Entry, String> {
        self.entries
            .get(&id)
            .ok_or("Unknown operator identity".into())
    }
    pub fn deployment(&self) -> &Deployment {
        &self.deployment
    }
    pub fn dp(&self, owner: Id) -> Option<f64> {
        self.deployment.dp(owner)
    }
    pub(crate) fn take_skill(&mut self, id: Id) -> Result<skills::Runtime, String> {
        self.healthy()?;
        self.entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?
            .skill
            .take()
            .ok_or("Operator skill is already executing".into())
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
            let bank = Bank::new(id, def.kind, def.cost, 1, 1)?;
            let skill = skills::Runtime::new(bank, def.spec.clone(), self.depth)?;
            let mut u = Unit::new(UnitStats::new(def.base.clone()), Side::Ally);
            u.stats.alive = false;
            u.owner = Some(owner);
            u.skill = true;
            u.deployment.kind = deploy::Kind::Operator;
            u.deployment.uid = Some(uid);
            u.deployment.home = home;
            u.deployment.tile = home;
            u.position = [home[0] as f64, home[1] as f64];
            w.insert_unit(id, u)?;
            self.deployment.register(id, w)?;
            if let Some(buff) = def.permanent.clone() {
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
                    definition: Arc::new(def),
                    direction,
                    skill: Some(skill),
                },
            );
            self.order.push(id);
            Ok(id)
        })();
        self.finish(result)
    }
    pub fn deploy_initial<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        grid: &Grid,
    ) -> Result<(), String> {
        self.healthy()?;
        let mut order = self.order.clone();
        // Input/unit update order is preserved separately from deployment order.
        let mut homes = BTreeMap::new();
        for &id in &order {
            homes.insert(id, w.unit(id)?.deployment.home);
        }
        order.sort_by(|a, b| {
            let [ar, ac] = homes[a];
            let [br, bc] = homes[b];
            match self.deploy_order {
                DeployOrder::Column => ac.cmp(&bc).then(br.cmp(&ar)),
                DeployOrder::RowBottomUp => ar.cmp(&br).then(ac.cmp(&bc)),
            }
            .then(a.cmp(b))
        });
        let mut dispatch = Deploy {
            entries: &mut self.entries,
            r,
            h,
            grid: Some(grid),
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
        let result = entry
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?
            .on_damaged(
                w,
                &mut Skill {
                    r,
                    h,
                    mods: &entry.definition.modifiers,
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
                    mods: &entry.definition.modifiers,
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
                            mods: &entry.definition.modifiers,
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
                grid: Some(grid),
                initial_cooldown: self.initial_cooldown,
            },
        );
        self.finish(result)
    }
    pub fn range(&self, w: &World, id: Id) -> Result<Vec<usize>, String> {
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
    mods: &'a Numbers,
}
impl<H: buffs::Hooks> skills::Hooks for Skill<'_, H> {
    fn event(
        &mut self,
        e: skills::Event,
        _: &mut skills::Context,
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
                            ..Default::default()
                        },
                        mods_present: true,
                        ..Default::default()
                    },
                )?;
            }
            skills::Event::RemoveModifiers => {
                self.r
                    .remove(w, self.h, s.unit(), &format!("skill:{}", s.unit()))?;
            }
            skills::Event::Start
            | skills::Event::End
            | skills::Event::SkillStart
            | skills::Event::SkillEnd
            | skills::Event::DisplayOn
            | skills::Event::DisplayOff => {}
            skills::Event::PassiveWindow => {}
            _ => return Err("Unexpected callback in default operator kit".into()),
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
    grid: Option<&'a Grid>,
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
        u.ground = self.grid.ok_or("Deployment requires grid context")?.tiles()
            [row as usize * COLS + col as usize]
            .height
            == "LOW";
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        let skill = entry
            .skill
            .as_mut()
            .ok_or("Operator skill is already executing")?;
        skill.reset(
            w,
            &mut Skill {
                r: self.r,
                h: self.h,
                mods: &entry.definition.modifiers,
            },
            None,
        )?;
        if opts.initial {
            skill.op_ready_at = w.time + self.initial_cooldown;
        }
        Ok(())
    }
    fn event(
        &mut self,
        e: deploy::Event,
        id: Id,
        _: Option<Id>,
        w: &mut World,
        _: &mut Deployment,
    ) -> Result<(), String> {
        if e == deploy::Event::EndSkill {
            let entry = self
                .entries
                .get_mut(&id)
                .ok_or("Unknown operator identity")?;
            entry
                .skill
                .as_mut()
                .ok_or("Operator skill is already executing")?
                .end(
                    w,
                    &mut Skill {
                        r: self.r,
                        h: self.h,
                        mods: &entry.definition.modifiers,
                    },
                    "death",
                )?;
        }
        Ok(())
    }
    fn remove_temporary_effects(&mut self, id: Id, w: &mut World) -> Result<(), String> {
        self.r.discard_temporary(w, self.h, id)
    }
}
