// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57: buffs.aggregateMods and units.Unit._recalc.
use crate::deterministic::math::powi;
use std::collections::{BTreeMap, BTreeSet};
pub type Numbers = BTreeMap<String, f64>;
#[derive(Clone, Debug)]
pub struct Buff {
    pub mods: Numbers,
    pub flags: BTreeSet<String>,
    pub stacks: u32,
    pub shield: f64,
    pub permanent: bool,
}
impl Default for Buff {
    fn default() -> Self {
        Self {
            mods: Numbers::new(),
            flags: BTreeSet::new(),
            stacks: 1,
            shield: 0.0,
            permanent: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Modifiers {
    pub add: Numbers,
    pub mul: Numbers,
    pub flags: BTreeSet<String>,
    pub shield: f64,
    pub permanent_range: f64,
}
impl Modifiers {
    pub fn aggregate(buffs: &[Buff]) -> Result<Self, &'static str> {
        let mut out = Self::default();
        let mut dodge: BTreeMap<&str, (u64, f64, f64)> = BTreeMap::new();
        for b in buffs {
            if b.stacks == 0 || b.stacks > i32::MAX as u32 {
                return Err("Buff stacks outside positive i32 domain");
            }
            let st = b.stacks as f64;
            for (k, &v) in &b.mods {
                if !v.is_finite() || k == "shield" {
                    continue;
                }
                if k.ends_with("Mul") {
                    let p = if b.stacks == 1 {
                        v
                    } else {
                        powi(v, b.stacks as i32)?
                    };
                    *out.mul.entry(k.clone()).or_insert(1.0) *= p;
                } else if matches!(k.as_str(), "dodgePhys" | "dodgeArts") && v > 0.0 {
                    let p = v.min(1.0);
                    let miss = powi(1.0 - p, b.stacks as i32)?;
                    if let Some(d) = dodge.get_mut(k.as_str()) {
                        d.0 += b.stacks as u64;
                        d.2 *= miss;
                    } else {
                        dodge.insert(k, (b.stacks as u64, p, miss));
                    }
                } else {
                    *out.add.entry(k.clone()).or_default() += v * st;
                    if k == "rangeExtend" && b.permanent {
                        out.permanent_range += v * st;
                    }
                }
            }
            out.flags.extend(b.flags.iter().cloned());
            if b.shield > 0.0 {
                out.shield += b.shield;
            }
        }
        for (k, (n, p, miss)) in dodge {
            *out.add.entry(k.into()).or_default() += if n == 1 { p } else { 1.0 - miss };
        }
        Ok(out)
    }
    fn a(&self, k: &str) -> f64 {
        self.add.get(k).copied().unwrap_or(0.0)
    }
    fn m(&self, k: &str) -> f64 {
        self.mul.get(k).copied().unwrap_or(1.0)
    }
}
fn finite(v: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}
fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        x
    } else {
        let f = x.floor();
        if x - f < 0.5 {
            f
        } else {
            f + 1.0
        }
    }
}
// Rust max/min discard NaN, whereas JS Math.max/min propagate it.
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}
fn clamp(v: f64, a: f64, b: f64) -> f64 {
    if v < a {
        a
    } else if v > b {
        b
    } else {
        v
    }
}
#[derive(Clone, Debug)]
pub struct Base(pub Numbers);
impl Default for Base {
    fn default() -> Self {
        Self(
            [
                ("maxHp", 1.0),
                ("atk", 0.0),
                ("def", 0.0),
                ("res", 0.0),
                ("aspd", 100.0),
                ("bat", 1.0),
                ("blockCnt", 0.0),
                ("moveSpeed", 0.0),
                ("spRecovery", 1.0),
                ("tauntLevel", 0.0),
                ("massLevel", 0.0),
                ("hpRecoveryPerSec", 0.0),
                ("cost", 0.0),
                ("respawnTime", 0.0),
                ("rangeRadius", 0.0),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect(),
        )
    }
}
impl Base {
    fn n(&self, k: &str) -> f64 {
        self.0.get(k).copied().unwrap_or(f64::NAN)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Cultivation {
    pub hp: f64,
    pub atk: f64,
    pub def: f64,
}
impl Default for Cultivation {
    fn default() -> Self {
        Self {
            hp: 1.0,
            atk: 1.0,
            def: 1.0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Stats {
    pub numbers: BTreeMap<&'static str, f64>,
    pub flags: BTreeSet<String>,
}
impl Stats {
    pub fn n(&self, k: &str) -> f64 {
        *self.numbers.get(k).expect("Unknown stat")
    }
    pub fn calculate(b: &Base, mods: &Modifiers, cm: Cultivation, move_floor: f64) -> Self {
        let a = |k| mods.a(k);
        let m = |k| mods.m(k);
        let n = |k| b.n(k);
        let mut numbers = BTreeMap::new();
        let bhp = if finite(n("maxHp"), 1.0) > 0.0 {
            finite(n("maxHp"), 1.0)
        } else {
            1.0
        };
        let max_hp = max(
            1.0,
            finite(
                (bhp + a("hpFlat")) * max(0.0, 1.0 + a("hpPct")) * m("hpMul") * cm.hp,
                bhp,
            ),
        );
        let atk = max(
            0.0,
            finite(
                ((n("atk") + a("atkFlat")) * max(0.0, 1.0 + a("atkPct")) + a("atkFinal"))
                    * m("atkMul")
                    * cm.atk,
                finite(n("atk"), 0.0),
            ),
        );
        let def = max(
            0.0,
            finite(
                (n("def") + a("defFlat")) * max(0.0, 1.0 + a("defPct")) * m("defMul") * cm.def,
                finite(n("def"), 0.0),
            ),
        );
        let res = clamp(
            finite(
                (n("res") + a("resFlat")) * m("resMul"),
                finite(n("res"), 0.0),
            ),
            0.0,
            100.0,
        );
        let aspd = clamp(finite(n("aspd") + a("aspd"), 100.0), 20.0, 600.0);
        let bbat = if finite(n("bat"), 1.0) > 0.0 {
            finite(n("bat"), 1.0)
        } else {
            1.0
        };
        let bat = finite(bbat * max(0.1, 1.0 + a("batPct")), bbat);
        for (k, v) in [
            ("maxHp", max_hp),
            ("atk", atk),
            ("def", def),
            ("res", res),
            ("aspd", aspd),
            ("bat", bat),
            ("interval", bat * 100.0 / aspd),
            (
                "blockCnt",
                max(0.0, finite(js_round(n("blockCnt") + a("blockCnt")), 0.0)),
            ),
            (
                "moveSpeed",
                max(
                    move_floor,
                    finite(
                        (n("moveSpeed") + a("moveFlat")) * m("moveMul"),
                        finite(n("moveSpeed"), 0.0),
                    ),
                ),
            ),
            ("rangeExtend", max(0.0, js_round(a("rangeExtend")))),
            (
                "blockRadiusScale",
                max(0.0, finite(a("blockRadiusScale"), 0.0)),
            ),
            (
                "baseRangeExtend",
                max(0.0, finite(js_round(mods.permanent_range), 0.0)),
            ),
            (
                "massLevel",
                max(
                    0.0,
                    finite(finite(n("massLevel"), 0.0) + a("massFlat"), 0.0),
                ),
            ),
            ("maxTargets", a("maxTargets")),
            (
                "taunt",
                finite(n("tauntLevel"), 0.0)
                    + a("taunt")
                    + if mods.flags.contains("taunt") {
                        1.0
                    } else {
                        0.0
                    },
            ),
            (
                "spRecovery",
                max(
                    0.0,
                    finite(
                        (n("spRecovery") + a("spRecoveryFlat")) * m("spRecoveryMul"),
                        0.0,
                    ),
                ),
            ),
            (
                "hpRegen",
                finite(
                    n("hpRecoveryPerSec") + a("hpRegen") + a("hpRegenRatio") * max_hp,
                    0.0,
                ),
            ),
            ("shield", mods.shield),
        ] {
            numbers.insert(k, v);
        }
        for k in ["dodgePhys", "dodgeArts", "defIgnorePct", "resIgnorePct"] {
            numbers.insert(k, clamp(a(k), 0.0, 1.0));
        }
        for k in [
            "defIgnoreFlat",
            "resIgnoreFlat",
            "artsAttackAddition",
            "spCostFlat",
        ] {
            numbers.insert(k, a(k));
        }
        for k in [
            "dmgDealtMul",
            "physDealtMul",
            "artsDealtMul",
            "dmgTakenMul",
            "physTakenMul",
            "artsTakenMul",
            "trueTakenMul",
            "elemTakenMul",
            "elementalTakenMul",
            "healingDealtMul",
            "healingTakenMul",
            "atkScaleMul",
            "redeployMul",
            "costMul",
        ] {
            numbers.insert(k, m(k));
        }
        let mut flags = mods.flags.clone();
        if ["stun", "freeze", "sleep", "levitate"]
            .iter()
            .any(|k| flags.contains(*k))
        {
            flags.insert("stun".into());
        }
        Self { numbers, flags }
    }
}
/// Lazy aggregation preserves HP ratio exactly when stats are read, matching Unit.s.
/// Unit identity, content hooks and deployment lifecycle are added by the battle layer.
pub struct UnitStats {
    base: Base,
    buffs: Vec<Buff>,
    cultivation: Cultivation,
    move_floor: f64,
    cached: Option<Stats>,
    dirty: bool,
    pub hp: f64,
    pub alive: bool,
    pub rate_dirty: bool,
}
impl UnitStats {
    pub fn new(base: Base) -> Self {
        let hp = base.n("maxHp");
        Self {
            base,
            buffs: vec![],
            cultivation: Cultivation::default(),
            move_floor: 0.0,
            cached: None,
            dirty: true,
            hp,
            alive: true,
            rate_dirty: false,
        }
    }
    pub fn edit_base(&mut self, edit: impl FnOnce(&mut Base)) {
        edit(&mut self.base);
        self.dirty = true;
    }
    pub fn set_buffs(&mut self, buffs: Vec<Buff>) {
        self.buffs = buffs;
        self.dirty = true;
    }
    pub fn set_cultivation(&mut self, c: Cultivation) {
        self.cultivation = c;
        self.dirty = true;
    }
    pub fn set_move_floor(&mut self, floor: f64) {
        self.move_floor = floor;
        self.dirty = true;
    }
    pub fn stats(&mut self) -> Result<&Stats, &'static str> {
        if self.dirty {
            let prev = self
                .cached
                .as_ref()
                .map(|s| s.n("maxHp"))
                .unwrap_or(self.base.n("maxHp"));
            let s = Stats::calculate(
                &self.base,
                &Modifiers::aggregate(&self.buffs)?,
                self.cultivation,
                self.move_floor,
            );
            if self
                .cached
                .as_ref()
                .map(|old| (old.n("interval") - s.n("interval")).abs() > 1e-9)
                .unwrap_or(true)
            {
                self.rate_dirty = true;
            }
            if self.alive && prev > 0.0 && (prev - s.n("maxHp")).abs() > 1e-9 {
                self.hp = clamp(
                    finite(self.hp * (s.n("maxHp") / prev), s.n("maxHp")),
                    0.0,
                    s.n("maxHp"),
                );
            }
            self.cached = Some(s);
            self.dirty = false;
        }
        Ok(self.cached.as_ref().unwrap())
    }
    /// Allies use motion directly; querying flight must not trigger HP rescaling.
    pub fn is_flying(&mut self, enemy: bool, motion_fly: bool) -> Result<bool, &'static str> {
        if !enemy {
            return Ok(motion_fly);
        }
        let flags = &self.stats()?.flags;
        Ok(if flags.contains("levitate") {
            true
        } else if flags.contains("groundbind") {
            false
        } else {
            motion_fly || flags.contains("float")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hp_buff() -> Buff {
        Buff {
            mods: [("hpMul".into(), 2.0)].into_iter().collect(),
            ..Buff::default()
        }
    }
    #[test]
    fn allied_flight_query_does_not_recalculate_hp() {
        let mut base = Base::default();
        base.0.insert("maxHp".into(), 100.0);
        let mut u = UnitStats::new(base);
        u.hp = 25.0;
        u.stats().unwrap();
        u.set_buffs(vec![hp_buff()]);
        assert!(!u.is_flying(false, false).unwrap());
        assert_eq!(u.hp, 25.0);
        assert_eq!(u.stats().unwrap().n("maxHp"), 200.0);
        assert_eq!(u.hp, 50.0);
        u.stats().unwrap();
        assert_eq!(u.hp, 50.0);
    }
    #[test]
    fn flying_precedence_and_dead_hp_preservation() {
        let mut u = UnitStats::new(Base::default());
        let mut b = hp_buff();
        b.flags.insert("groundbind".into());
        b.flags.insert("float".into());
        u.alive = false;
        u.hp = 0.25;
        u.set_buffs(vec![b.clone()]);
        assert!(!u.is_flying(true, true).unwrap());
        assert_eq!(u.hp, 0.25);
        b.flags.insert("levitate".into());
        u.set_buffs(vec![b]);
        assert!(u.is_flying(true, false).unwrap());
        assert!(u.stats().unwrap().flags.contains("stun"));
    }
    #[test]
    fn invalid_stacks_do_not_commit_partial_stats() {
        let mut u = UnitStats::new(Base::default());
        u.stats().unwrap();
        let hp = u.hp;
        u.set_buffs(vec![Buff {
            stacks: 0,
            ..hp_buff()
        }]);
        assert!(u.stats().is_err());
        assert_eq!(u.hp, hp);
        u.set_buffs(vec![]);
        assert_eq!(u.stats().unwrap().n("maxHp"), 1.0);
    }
}
