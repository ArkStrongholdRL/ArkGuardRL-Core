// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 content/enemies/{archetypes,helpers}.js.
use crate::battle::{
    buffs::{self, Spec},
    element::{self, Element},
    health::{Id, Side, World},
    hit,
    stats::Buff,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

mod abilities;
pub(crate) mod bosses;
pub use bosses::Boss;
pub(crate) mod specials;
pub use specials::Special;
pub(crate) mod terrain;
pub use terrain::TerrainKit;
pub(crate) mod casters;
pub use casters::Caster;
pub(crate) mod leaders;
pub(crate) mod supporters;
pub use leaders::Leader;
pub use supporters::Supporter;
pub(crate) mod lifecycle;
pub(crate) mod movement;
pub use lifecycle::Lifecycle;
pub use movement::Movement;
mod families;
pub(crate) mod forms;
pub(crate) mod reactions;
pub(crate) mod zones;
pub use forms::Form;
pub use reactions::Reaction;
pub use zones::ZoneKit;
mod splits;
pub use abilities::Ability;
pub use families::{Family, Prisoner};
pub use splits::Split;

#[derive(Clone, Copy, Debug)]
pub enum Kit {
    Plain,
    Boss(Boss),
    Special(Special),
    Terrain(TerrainKit),
    Caster(Caster),
    Leader(Leader),
    Supporter(Supporter),
    Movement(Movement),
    Lifecycle(Lifecycle),
    Reaction(Reaction),
    Form(Form),
    Zone(ZoneKit),
    Ability(Ability),
    Family(Family),
    Split(Split),
    Stealth { restore: f64 },
    Times { arts_only: bool },
    Refraction { res: f64, hp_pct: f64 },
    Element { kind: Element, ratio: f64 },
}
fn talent_value(raw: &Value, overrides: &Value, key: &str) -> Option<f64> {
    let base = raw["talents"]
        .get("bb")
        .filter(|v| !v.is_null())
        .unwrap_or(&raw["talent"]);
    overrides["talents"]["bb"]
        .get(key)
        .or_else(|| base.get(key))
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
}
fn talent(raw: &Value, overrides: &Value, keys: &[&str]) -> f64 {
    keys.iter()
        .find_map(|key| talent_value(raw, overrides, key))
        .unwrap_or(0.)
}

impl Kit {
    pub(crate) fn validate_tag(self, tag: Option<&str>) -> Result<(), String> {
        let supported = match tag {
            Some("boss") => matches!(
                self,
                Self::Boss(
                    Boss::Helm(_)
                        | Boss::Gun(_)
                        | Boss::Lion(_)
                        | Boss::Deer(_)
                        | Boss::Pipe(_)
                        | Boss::Quintus(_)
                ) | Self::Caster(Caster {
                    passive: casters::Passive::Lucien { .. },
                    ..
                })
            ),
            Some("part") => matches!(
                self,
                Self::Boss(
                    Boss::Blade(_)
                        | Boss::Shell { .. }
                        | Boss::Spring(_)
                        | Boss::Pipe(_)
                        | Boss::Echo(_)
                        | Boss::Equipment
                )
            ),
            _ => true,
        };
        if supported {
            Ok(())
        } else {
            Err("Boss or part tag requires an implemented kit".into())
        }
    }
    pub(crate) fn rebirth(self) -> Option<(f64, f64, f64)> {
        match self {
            Self::Lifecycle(k) => k.rebirth(),
            Self::Caster(c) => c.rebirth(),
            _ => None,
        }
    }
    /// Explicit content dispatch, not an inference from an enemy's name or tags.
    pub fn compile(
        key: &str,
        raw: &Value,
        overrides: &Value,
        tag: Option<&str>,
    ) -> Result<Self, String> {
        let original_key = key;
        let key = raw["effectRef"].as_str().unwrap_or(key);
        if let Some(kit) = Boss::compile(key, raw, overrides)? {
            return Ok(Self::Boss(kit));
        }
        if let Some(kit) = Special::compile(key, raw, overrides)? {
            return Ok(Self::Special(kit));
        }
        if let Some(kit) = TerrainKit::compile(key, raw, overrides)? {
            return Ok(Self::Terrain(kit));
        }
        if let Some(kit) = Caster::compile(key, raw, overrides)? {
            return Ok(Self::Caster(kit));
        }
        if let Some(kit) = Leader::compile(key, raw, overrides)? {
            return Ok(Self::Leader(kit));
        }
        if let Some(kit) = Supporter::compile(key, raw, overrides)? {
            return Ok(Self::Supporter(kit));
        }
        if let Some(kit) = Movement::compile(key, raw, overrides, tag) {
            return Ok(Self::Movement(kit));
        }
        if let Some(kit) = Lifecycle::compile(key, raw, overrides)? {
            return Ok(Self::Lifecycle(kit));
        }
        if let Some(reaction) = Reaction::compile(key, raw, overrides)? {
            return Ok(Self::Reaction(reaction));
        }
        if let Some(form) = Form::compile(key, raw, overrides)? {
            return Ok(Self::Form(form));
        }
        if let Some(zone) = ZoneKit::compile(key, raw, overrides)? {
            return Ok(Self::Zone(zone));
        }
        if let Some(split) = Split::compile(key, raw, overrides)? {
            return Ok(Self::Split(split));
        }
        if let Some(family) = Family::compile(key, raw, overrides, original_key, tag)? {
            return Ok(Self::Family(family));
        }
        if let Some(ability) = Ability::compile(key, raw, overrides)? {
            return Ok(Self::Ability(ability));
        }
        Ok(match key {
            "enemy_2009_csaudc" => Self::Element {
                kind: Element::Neural,
                ratio: talent(raw, overrides, &["combat.attack@ep_damage_ratio"]),
            },
            "enemy_10094_crstf" => Self::Element {
                kind: Element::Neural,
                ratio: talent(raw, overrides, &["ep.ep_damage_ratio"]),
            },
            "enemy_10099_crvln" => Self::Element {
                kind: Element::Neural,
                ratio: talent(raw, overrides, &["inside.attack@ep_damage_ratio"]),
            },
            "enemy_1148_dssbr" | "enemy_1148_dssbr_2" => Self::Element {
                kind: Element::Neural,
                ratio: talent(raw, overrides, &["epdamage.attack@ep_damage_ratio"]),
            },
            "enemy_1305_mhslim" | "enemy_1305_mhslim_2" => Self::Element {
                kind: Element::Burn,
                ratio: talent(raw, overrides, &["EpDamage.attack@ep_damage_ratio"]),
            },
            "enemy_2021_syfish" => Self::Element {
                kind: Element::Erosion,
                ratio: talent(raw, overrides, &["EpDamage.attack@ep_damage_ratio"]),
            },
            "enemy_1229_darmy" | "enemy_1229_darmy_2" => Self::Element {
                kind: Element::Apoptosis,
                ratio: talent(raw, overrides, &["epdamage.attack@ep_damage_ratio"]),
            },
            "enemy_1009_lurker"
            | "enemy_1019_jshoot"
            | "enemy_1019_jshoot_2"
            | "enemy_1023_jmage"
            | "enemy_1283_sgkill"
            | "enemy_1283_sgkill_2"
            | "enemy_1389_winbab_2"
            | "enemy_10031_cnvsld"
            | "enemy_10042_prtrop"
            | "enemy_10042_prtrop_2" => {
                let restore: f64 = match original_key {
                    "enemy_1283_sgkill" | "enemy_1283_sgkill_2" => 1.,
                    "enemy_1389_winbab_2" | "enemy_10031_cnvsld" => 0.,
                    _ => 3.,
                };
                Self::Stealth {
                    restore: if tag == Some("duck") {
                        restore.max(1.)
                    } else {
                        restore
                    },
                }
            }
            "enemy_1196_msfyin"
            | "enemy_1196_msfyin_2"
            | "enemy_1198_msfshu"
            | "enemy_1198_msfshu_2"
            | "enemy_1200_msfjin"
            | "enemy_1200_msfjin_2"
            | "enemy_1202_msfzhi"
            | "enemy_1202_msfzhi_2"
            | "enemy_1208_msfji"
            | "enemy_1208_msfji_2"
            | "enemy_1210_msfden"
            | "enemy_1210_msfden_2" => Self::Times { arts_only: false },
            "enemy_1204_msfhu" | "enemy_1204_msfhu_2" => Self::Times { arts_only: true },
            "enemy_1165_duhond"
            | "enemy_1165_duhond_2"
            | "enemy_1166_dusbr"
            | "enemy_1166_dusbr_2"
            | "enemy_1168_dumage"
            | "enemy_1168_dumage_2"
            | "enemy_1170_dushld"
            | "enemy_1170_dushld_2" => Self::Refraction {
                res: talent(
                    raw,
                    overrides,
                    &["refracting.magic_resistance", "Refracting.magic_resistance"],
                ),
                hp_pct: 0.,
            },
            "enemy_9011_acrefr" => Self::Refraction {
                res: talent(raw, overrides, &["Refracting.magic_resistance"]),
                hp_pct: talent(raw, overrides, &["Refracting.max_hp"]),
            },
            "enemy_1000_gopro_2"
            | "enemy_1001_bigbo"
            | "enemy_1005_yokai"
            | "enemy_1005_yokai_2"
            | "enemy_1005_yokai_3"
            | "enemy_1006_shield"
            | "enemy_1006_shield_2"
            | "enemy_1006_shield_3"
            | "enemy_1007_slime"
            | "enemy_1010_demon"
            | "enemy_1010_demon_2"
            | "enemy_1041_lazerd"
            | "enemy_1041_lazerd_2"
            | "enemy_1043_zomsbr"
            | "enemy_1061_zomshd"
            | "enemy_1046_agent"
            | "enemy_1071_dftman"
            | "enemy_1092_mdgint"
            | "enemy_1251_lysyta"
            | "enemy_1251_lysyta_2"
            | "enemy_1252_lysytb_2"
            | "enemy_1254_lypa_2"
            | "enemy_1325_cbgpro"
            | "enemy_1325_cbgpro_2"
            | "enemy_1381_winman"
            | "enemy_1381_winman_2"
            | "enemy_1387_winshd"
            | "enemy_1415_mmkabi_2"
            | "enemy_1433_dsbasi"
            | "enemy_1433_dsbasi_2"
            | "enemy_1438_dspred"
            | "enemy_2002_bearmi"
            | "enemy_2002_bearmi_2"
            | "enemy_9012_acloon"
            | "enemy_10040_cnvbln"
            | "enemy_10043_sailor"
            | "enemy_10073_mpcar"
            | "enemy_10124_uashld_2" => Self::Plain,
            _ => return Err(format!("Enemy content not implemented: {key}")),
        })
    }
}
#[derive(Clone)]
pub(crate) struct Definition {
    kit: Kit,
    child: Option<Arc<str>>,
    branch: Option<Arc<str>>,
    quintus: Option<Arc<bosses::quintus::Stages>>,
}
impl Definition {
    pub(crate) fn compile(
        key: &str,
        raw: &Value,
        overrides: &Value,
        tag: Option<&str>,
    ) -> Result<Self, String> {
        let kit = Kit::compile(key, raw, overrides, tag)?;
        let child = if matches!(kit, Kit::Boss(Boss::Helm(_))) {
            Some(Arc::from(
                abilities::skill(raw, overrides, "2")["bbStr"]["enemy_key"]
                    .as_str()
                    .unwrap_or("enemy_1005_yokai"),
            ))
        } else if matches!(kit, Kit::Boss(Boss::Pipe(_))) {
            Some(Arc::from(
                overrides["talents"]["bbStr"]
                    .get("1.enemy_key")
                    .or_else(|| raw["talents"]["bbStr"].get("1.enemy_key"))
                    .and_then(Value::as_str)
                    .unwrap_or("enemy_9023_acdums"),
            ))
        } else if let Kit::Split(split) = kit {
            let field = if matches!(split, Split::Boat { .. }) {
                "DeathRattle.enemy_key"
            } else {
                "DeadSpawn.enemy_key"
            };
            overrides["talents"]["bbStr"]
                .get(field)
                .or_else(|| raw["talents"]["bbStr"].get(field))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(Arc::from)
        } else if matches!(kit, Kit::Supporter(Supporter::Torturer { .. })) {
            overrides["talents"]["bbStr"]
                .get("Summon.enemy_key")
                .or_else(|| raw["talents"]["bbStr"].get("Summon.enemy_key"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(Arc::from)
        } else {
            None
        };
        let branch = matches!(kit, Kit::Boss(Boss::Helm(_))).then(|| {
            Arc::from(
                abilities::skill(raw, overrides, "2")["bbStr"]["branch_id"]
                    .as_str()
                    .unwrap_or("boss_summon_enemy"),
            )
        });
        let quintus = matches!(kit, Kit::Boss(Boss::Quintus(_)))
            .then(|| Arc::new(bosses::quintus::Stages::compile(raw, overrides)));
        Ok(Self {
            kit,
            child,
            branch,
            quintus,
        })
    }
}
#[derive(Clone)]
struct State {
    definition: Definition,
    on: Option<bool>,
    hp_lost: bool,
    count: u32,
    accumulator: f64,
    memory: families::Memory,
    form: Option<forms::State>,
    tank: Option<zones::TankState>,
    reaction: reactions::Memory,
    movement: movement::Memory,
    lifecycle: lifecycle::Memory,
    leader: leaders::Memory,
    caster: casters::State,
    boss: bosses::Memory,
    terrain: terrain::State,
    special: specials::Memory,
    supporter: supporters::Memory,
    drone: Option<bosses::helm::DroneLink>,
}
pub struct Runtime {
    units: BTreeMap<Id, State>,
    capacity: usize,
    faulted: bool,
    freed_all: bool,
    chalices: usize,
    lp_loss: f64,
}
fn flag_buff(key: &str, flag: &str) -> Buff {
    let mut b = Buff::default();
    b.key = Some(key.into());
    b.flags.insert(flag.into());
    b.runtime.persist = true;
    b
}
impl Runtime {
    pub fn new(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("Enemy content capacity must be positive".into());
        }
        Ok(Self {
            units: BTreeMap::new(),
            capacity,
            faulted: false,
            freed_all: false,
            chalices: 0,
            lp_loss: 0.,
        })
    }
    pub(crate) fn has_chalices(&self) -> bool {
        self.chalices > 0
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    pub fn retained(&self) -> usize {
        self.units.len()
    }
    pub(crate) fn branch(&self, id: Id) -> Option<Arc<str>> {
        self.units
            .get(&id)
            .and_then(|s| s.definition.branch.clone())
    }
    pub(crate) fn child(&self, id: Id) -> Option<Arc<str>> {
        self.units.get(&id).and_then(|s| s.definition.child.clone())
    }
    pub fn remove(&mut self, id: Id) {
        if self
            .units
            .remove(&id)
            .is_some_and(|s| matches!(s.definition.kit, Kit::Terrain(TerrainKit::Chalice { .. })))
        {
            self.chalices -= 1;
        }
    }
    /// The actual `damaged` callback: one proc per resolved normal-attack damage
    /// instance, before fatal removal. Caller immediately applies the returned
    /// request through the shared element pipeline, including nested bursts.
    pub fn damaged(
        &self,
        w: &mut World,
        request: &hit::Request,
    ) -> Result<Option<element::Request>, String> {
        if self.faulted {
            return Err("Enemy content runtime is faulted".into());
        }
        if !request.is_attack {
            return Ok(None);
        }
        let Some(source) = request.hit.source else {
            return Ok(None);
        };
        if w.unit(source)?.side != Side::Enemy || w.unit(request.hit.target)?.side != Side::Ally {
            return Ok(None);
        }
        let Some(state) = self.units.get(&source) else {
            return Ok(None);
        };
        let (kind, ratio) = match state.definition.kit {
            Kit::Element { kind, ratio } => (kind, ratio),
            Kit::Caster(Caster {
                passive: casters::Passive::DeathEye { element },
                ..
            }) => (Element::Apoptosis, element),
            Kit::Movement(Movement::Syufo { erosion }) => (Element::Erosion, erosion),
            Kit::Lifecycle(Lifecycle::Reborn { neural, .. }) => (Element::Neural, neural),
            Kit::Ability(Ability::Song { neural, .. }) => (Element::Neural, neural),
            Kit::Split(Split::Boat { erosion }) => (Element::Erosion, erosion),
            Kit::Ability(Ability::Power { erosion, .. })
            | Kit::Ability(Ability::Tide { erosion }) => (Element::Erosion, erosion),
            _ => return Ok(None),
        };
        if !(ratio > 0.) || !w.unit(request.hit.target)?.stats.alive {
            return Ok(None);
        }
        let amount = w
            .unit_mut(source)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .n("atk")
            * ratio;
        let mut req =
            element::Request::new(Some(source), Some(source), request.hit.target, kind, amount);
        req.damage.attack_type = hit::AttackType::None;
        req.damage.tags = vec!["enemyAbility".into()];
        Ok(Some(req))
    }
    fn call<T>(
        &mut self,
        r: &mut buffs::Runtime,
        f: impl FnOnce(&mut Self, &mut buffs::Runtime) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.faulted {
            return Err("Enemy content runtime is faulted".into());
        }
        let result = r.call(|r| f(self, r));
        if result.is_err() {
            self.faulted = true;
        }
        result
    }
    pub fn spawn<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
        kit: Kit,
        mods: &Value,
    ) -> Result<(), String> {
        self.install(
            w,
            r,
            h,
            id,
            Definition {
                kit,
                child: None,
                branch: None,
                quintus: None,
            },
            mods,
        )
    }
    pub(crate) fn install<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        id: Id,
        definition: Definition,
        mods: &Value,
    ) -> Result<(), String> {
        let kit = definition.kit;
        self.call(r, |this, r| {
            if w.unit(id)?.side != Side::Enemy {
                return Err("Enemy kit installed on a non-enemy".into());
            }
            if this.units.contains_key(&id) {
                return Err("Duplicate enemy kit installation".into());
            }
            if this.units.len() >= this.capacity {
                return Err("Enemy content capacity exceeded".into());
            }
            if matches!(kit, Kit::Terrain(TerrainKit::Chalice { .. })) {
                this.chalices += 1;
            }
            this.units.insert(
                id,
                State {
                    drone: None,
                    boss: match kit {
                        Kit::Boss(k) => k.memory(w.unit(id)?.position, w.time),
                        _ => bosses::Memory::None,
                    },
                    special: match kit {
                        Kit::Special(k) => k.memory(),
                        _ => specials::Memory::None,
                    },
                    terrain: terrain::State::default(),
                    caster: match kit {
                        Kit::Caster(k) => k.state(),
                        Kit::Boss(k) => definition
                            .quintus
                            .as_ref()
                            .map(|q| q.casters[0])
                            .unwrap_or_else(|| k.caster())
                            .state(),
                        _ => casters::State::default(),
                    },
                    leader: match kit {
                        Kit::Leader(k) => k.memory(),
                        _ => leaders::Memory::None,
                    },
                    supporter: match kit {
                        Kit::Supporter(k) => k.memory(),
                        _ => supporters::Memory::None,
                    },
                    movement: match kit {
                        Kit::Movement(k) => k.memory(),
                        _ => movement::Memory::None,
                    },
                    lifecycle: match kit {
                        Kit::Lifecycle(k) => k.memory(w.time),
                        _ => lifecycle::Memory::None,
                    },
                    reaction: match kit {
                        Kit::Reaction(k) => k.memory(w.unit(id)?.position),
                        _ => reactions::Memory::None,
                    },
                    form: match kit {
                        Kit::Form(f) => Some(f.state()),
                        _ => None,
                    },
                    tank: match kit {
                        Kit::Zone(ZoneKit::Tank(t)) => Some(zones::TankState {
                            sp: t.initial,
                            skill: false,
                        }),
                        _ => None,
                    },
                    definition,
                    on: None,
                    hp_lost: false,
                    count: 0,
                    accumulator: 0.,
                    memory: match kit {
                        Kit::Family(f) => f.memory(),
                        Kit::Split(s) => s.memory(),
                        _ => families::Memory::None,
                    },
                },
            );
            match kit {
                Kit::Boss(k) => k.spawn(w, r, h, id)?,
                Kit::Special(k) => k.spawn(w, r, h, id)?,
                Kit::Caster(k) => k.spawn(w, r, h, id)?,
                Kit::Lifecycle(k) => k.spawn(w, r, h, id)?,
                Kit::Supporter(k) => k.spawn(w, r, h, id)?,
                Kit::Movement(k) => k.spawn(w, r, h, id)?,
                Kit::Reaction(reaction) => reaction.spawn(w, r, h, id)?,
                Kit::Split(split) => split.spawn(w, r, h, id)?,
                Kit::Family(family) => family.spawn(w, r, h, id)?,
                Kit::Ability(ability) => Self::spawn_ability(w, r, h, id, ability)?,
                Kit::Stealth { restore } => {
                    let mut b = flag_buff("ab:stealth", "stealth");
                    b.runtime.stealth_restore = Some(restore);
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: b,
                            flags_present: true,
                            ..Spec::default()
                        },
                    )?;
                }
                Kit::Times { arts_only } => {
                    let supply = crate::battle::waves::number(&mods["supplyHpMul"], 1.);
                    let supply = if supply > 0. { supply } else { 1. };
                    let n = w.unit(id)?.stats.base_number("maxHp") / supply;
                    if !n.is_finite() {
                        return Err("Hit-count HP overflow".into());
                    }
                    let f = n.floor();
                    let hits = (if n - f < 0.5 { f } else { f + 1. }).max(1.);
                    let u = w.unit_mut(id)?;
                    u.stats.edit_base(|b| {
                        b.0.insert("maxHp".into(), hits);
                    });
                    u.stats.stats().map_err(str::to_string)?;
                    u.stats.hp = hits;
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: flag_buff(
                                "ab:hitCount",
                                if arts_only {
                                    "hitCountArts"
                                } else {
                                    "hitCount"
                                },
                            ),
                            flags_present: true,
                            ..Spec::default()
                        },
                    )?;
                    r.add(
                        w,
                        h,
                        id,
                        Spec {
                            buff: flag_buff("ab:unblockable", "unblockable"),
                            flags_present: true,
                            ..Spec::default()
                        },
                    )?;
                }
                _ => {}
            }
            Ok(())
        })
    }
    /// Run at the original enemy-content tick hook, after projectiles/redeploys.
    /// Concealment and hit-count mechanics themselves read canonical World buffs.
    pub fn tick<H: buffs::Hooks>(
        &mut self,
        w: &mut World,
        r: &mut buffs::Runtime,
        h: &mut H,
        ids: &[Id],
        dt: f64,
    ) -> Result<(), String> {
        self.call(r, |this, r| {
            if !dt.is_finite() || dt < 0. || ids.len() > w.unit_capacity() {
                return Err("Invalid enemy content tick limits".into());
            }
            for &id in ids {
                // Read the kit without copying the whole runtime state; most
                // enemies have no refraction at all.
                let Some(state) = this.units.get(&id) else {
                    continue;
                };
                let refraction = state.definition.kit.refraction();
                let u = w.unit(id)?;
                if !u.stats.alive || u.enemy.carrier.is_some() {
                    continue;
                }
                if let Some((res, hp_pct)) = refraction {
                    let on = !w
                        .unit_mut(id)?
                        .stats
                        .stats()
                        .map_err(str::to_string)?
                        .flags
                        .contains("silence");
                    let s = this.units.get_mut(&id).unwrap();
                    if hp_pct != 0. && !on {
                        s.hp_lost = true;
                    }
                    if s.on == Some(on) {
                        continue;
                    }
                    s.on = Some(on);
                    if on {
                        let mut b = Buff::default();
                        b.key = Some("ab:refraction".into());
                        b.mods.insert("resFlat".into(), res);
                        if hp_pct != 0. && !s.hp_lost {
                            b.mods.insert("hpPct".into(), hp_pct);
                        }
                        b.runtime.persist = true;
                        b.runtime.visible = true;
                        r.add(
                            w,
                            h,
                            id,
                            Spec {
                                buff: b,
                                mods_present: true,
                                ..Spec::default()
                            },
                        )?;
                    } else {
                        r.remove(w, h, id, "ab:refraction")?;
                    }
                }
            }
            Ok(())
        })
    }
}
