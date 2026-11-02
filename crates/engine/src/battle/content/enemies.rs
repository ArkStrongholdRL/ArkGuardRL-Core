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
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub enum Kit {
    Plain,
    Stealth { restore: f64 },
    Times { arts_only: bool },
    Refraction { res: f64, hp_pct: f64 },
    Element { kind: Element, ratio: f64 },
}
fn talent(raw: &Value, overrides: &Value, keys: &[&str]) -> f64 {
    let base = raw["talents"]
        .get("bb")
        .filter(|v| !v.is_null())
        .unwrap_or(&raw["talent"]);
    keys.iter()
        .find_map(|key| {
            overrides["talents"]["bb"]
                .get(*key)
                .or_else(|| base.get(*key))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
        })
        .unwrap_or(0.)
}
impl Kit {
    /// Explicit content dispatch, not an inference from an enemy's name or tags.
    pub fn compile(
        key: &str,
        raw: &Value,
        overrides: &Value,
        tag: Option<&str>,
    ) -> Result<Self, String> {
        let original_key = key;
        let key = raw["effectRef"].as_str().unwrap_or(key);
        Ok(match key {
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
#[derive(Clone, Copy)]
struct State {
    kit: Kit,
    on: Option<bool>,
    hp_lost: bool,
}
pub struct Runtime {
    units: BTreeMap<Id, State>,
    capacity: usize,
    faulted: bool,
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
        })
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    pub fn retained(&self) -> usize {
        self.units.len()
    }
    pub fn remove(&mut self, id: Id) {
        self.units.remove(&id);
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
        if w.unit(source)?.side != Side::Enemy {
            return Ok(None);
        }
        let Some(state) = self.units.get(&source) else {
            return Ok(None);
        };
        let Kit::Element { kind, ratio } = state.kit else {
            return Ok(None);
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
            this.units.insert(
                id,
                State {
                    kit,
                    on: None,
                    hp_lost: false,
                },
            );
            match kit {
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
                let Some(state) = this.units.get(&id).copied() else {
                    continue;
                };
                let u = w.unit(id)?;
                if !u.stats.alive || u.enemy.carrier.is_some() {
                    continue;
                }
                if let Kit::Refraction { res, hp_pct } = state.kit {
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
