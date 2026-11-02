// SPDX-License-Identifier: GPL-3.0-or-later
//! Original default operator lifecycle, sharing the world's units and buffs.
//! This roster is a battle component, not an env or a complete allied AI loop.
//! Only explicitly compiled kits are admitted; no generic unimplemented fallback.
use super::{
    blocking, buffs,
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

pub struct Definition {
    pub key: String,
    base: Base,
    modifiers: Numbers,
    resistance: f64,
    spec: skills::Spec,
    cost: f64,
}
impl Definition {
    pub fn load(data: &Catalog, key: &str) -> Result<Self, String> {
        let raw = &data.table("chess").ok_or("Missing chess data")?[key];
        Self::compile(key, raw)
    }
    /// Defaults only: no selected skill/module, potential, items or custom data
    /// composition is inferred. Raw records already contain default module stats.
    pub fn compile(key: &str, raw: &Value) -> Result<Self, String> {
        if !matches!(key, "chess_char_1_02_a" | "chess_char_1_02_b") {
            return Err(format!("Operator kit not implemented: {key}"));
        }
        if !raw.is_object() || raw["skill"]["skillId"] != "skchr_yak_2" {
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
        // These two original kits have no profile, immunity or range overrides.
        // Refuse altered content instead of silently executing a partial kit.
        if raw["rangeGrid"] != serde_json::json!([[0, 0]])
            || raw["position"] != "MELEE"
            || raw["profession"] != "TANK"
            || raw["subProfessionId"] != "protector"
            || raw["immunities"]
                .as_object()
                .is_some_and(|o| o.values().any(|v| *v == true))
            || skill["spType"] != "INCREASE_WITH_TIME"
            || skill["trigger"]["rule"] != "TAKE_DAMAGE"
            || number(&skill["maxChargeTime"], 1.) != 1.
        {
            return Err(format!("Unsupported operator mechanics: {key}"));
        }
        let bb = &skill["bb"];
        let resistance = raw["talents"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| t["index"] != -1)
            .map(|t| number(&t["bb"]["magic_resistance"], 0.))
            .unwrap_or(0.);
        Ok(Self {
            key: key.into(),
            base,
            modifiers: [
                ("hpPct".into(), number(&bb["max_hp"], 0.)),
                ("defPct".into(), number(&bb["def"], 0.)),
                ("resMul".into(), 1. + number(&bb["magic_resistance"], 0.)),
            ]
            .into(),
            resistance,
            spec: skills::Spec {
                duration: number(&skill["duration"], 0.),
                init_sp: number(&skill["initSp"], 0.).max(0.),
                rule: skills::Rule::TakeDamage,
                modifiers: true,
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
    pub definition: Definition,
    pub direction: Direction,
    pub skill: skills::Runtime,
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
            let bank = Bank::new(id, Kind::Duration, def.cost, 1, 1)?;
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
            let mut buff = Buff {
                key: Some("talent:yak".into()),
                mods: [("resFlat".into(), def.resistance)].into(),
                ..Default::default()
            };
            buff.runtime.persist = true;
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
            self.entries.insert(
                id,
                Entry {
                    definition: def,
                    direction,
                    skill,
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
            grid,
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
        let result = entry.skill.on_damaged(
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
        let result = entry.skill.gain(
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
                entry.skill.tick(
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
        grid: &Grid,
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
                grid,
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
        grid: &Grid,
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
                grid,
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
                grid,
                initial_cooldown: self.initial_cooldown,
            },
        );
        self.finish(result)
    }
    pub fn range(&self, w: &World, id: Id) -> Result<Vec<usize>, String> {
        self.entry(id)?;
        let [row, col] = w.unit(id)?.deployment.tile;
        Ok(vec![row as usize * COLS + col as usize])
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
    grid: &'a Grid,
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
        u.ground = self.grid.tiles()[row as usize * COLS + col as usize].height == "LOW";
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Unknown operator identity")?;
        entry.skill.reset(
            w,
            &mut Skill {
                r: self.r,
                h: self.h,
                mods: &entry.definition.modifiers,
            },
            None,
        )?;
        if opts.initial {
            entry.skill.op_ready_at = w.time + self.initial_cooldown;
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
            entry.skill.end(
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
