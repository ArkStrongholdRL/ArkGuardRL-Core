// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 status catalogue, selection guards and application.
use super::{
    blocking,
    buffs::{self, Refresh, Runtime, Spec},
    grid::Rect,
    health::{Id, Side, World},
    stats::{Buff, Numbers},
    targeting,
    valued::{self, Kind},
};
#[derive(Clone, Copy, Debug)]
pub struct FearStamp {
    pub sequence: u64,
    pub hit: [f64; 2],
    pub source: [f64; 2],
    pub self_fear: bool,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub duration: f64,
    pub value: Option<f64>,
    pub source: Option<Id>,
    pub force: bool,
    pub refresh: Option<Refresh>,
    pub point: Option<[f64; 2]>,
    pub resist_applied: bool,
    pub stack_as: Option<f64>,
    pub reenter: bool,
    pub ignore_select: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            duration: f64::INFINITY,
            value: None,
            source: None,
            force: false,
            refresh: None,
            point: None,
            resist_applied: false,
            stack_as: None,
            reenter: false,
            ignore_select: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Context {
    pub source: Option<Id>,
    pub target: Id,
    pub key: String,
    pub duration: f64,
    pub value: Option<f64>,
    pub cancelled: bool,
    pub entered: bool,
}
pub trait Hooks: buffs::Hooks + blocking::Hooks {
    fn has_before_status(&self) -> bool;
    fn before_status(
        &mut self,
        c: &mut Context,
        r: &mut Runtime,
        w: &mut World,
    ) -> Result<(), String>;
    fn has_status_applied(&self) -> bool;
    fn status_applied(&mut self, c: &Context, r: &mut Runtime, w: &mut World)
        -> Result<(), String>;
}
struct Template {
    flags: Vec<String>,
    mods: Numbers,
    valued: Option<Kind>,
    immune: Option<&'static str>,
    enemy_only: bool,
}
fn template(key: &str, value: Option<f64>) -> Template {
    let mut t = Template {
        flags: vec![],
        mods: Numbers::new(),
        valued: None,
        immune: None,
        enemy_only: false,
    };
    let flags: &[&str] = match key {
        "stun" => {
            t.immune = Some("stun");
            &["stun", "noBlock"]
        }
        "freeze" => {
            t.immune = Some("frozen");
            t.enemy_only = true;
            t.mods.insert("resFlat".into(), -15.0);
            &["freeze", "stun"]
        }
        "cold" => {
            t.mods.insert("aspd".into(), -30.0);
            &["cold"]
        }
        "sleep" => {
            t.immune = Some("sleep");
            &["sleep", "noBlock"]
        }
        "sluggish" => {
            t.mods.insert("moveMul".into(), 0.2);
            &[]
        }
        "bind" => {
            t.mods.insert("moveMul".into(), 0.0);
            &["bind", "noMove"]
        }
        "attract" => &["attract", "unblockable"],
        "silence" => {
            t.immune = Some("silence");
            &["silence"]
        }
        "fear" => {
            t.immune = Some("feared");
            &["fear", "unblockable"]
        }
        "tremble" => {
            t.immune = Some("feared");
            &["tremble"]
        }
        "levitate" => {
            t.immune = Some("levitate");
            &["levitate", "stun", "unblockable", "noDisplace"]
        }
        "groundbind" => {
            t.mods.insert("moveMul".into(), 0.0);
            &["groundbind", "noMove"]
        }
        "palsy" => {
            t.immune = Some("palsy");
            &[]
        }
        "taunt" => {
            t.mods.insert("taunt".into(), value.unwrap_or(1.0));
            &[]
        }
        "burnBurst" | "neuralBurst" | "necrosisBurst" | "apoptosisBurst" | "erosionBurst" => {
            &["burstLock"]
        }
        "slow" => {
            t.valued = Some(Kind::Slow);
            &[]
        }
        "fragile" => {
            t.valued = Some(Kind::Fragile);
            &[]
        }
        "artsFragile" => {
            t.valued = Some(Kind::ArtsFragile);
            &[]
        }
        "physFragile" => {
            t.valued = Some(Kind::PhysFragile);
            &[]
        }
        "elemFragile" => {
            t.valued = Some(Kind::ElemFragile);
            &[]
        }
        "weaken" => {
            t.valued = Some(Kind::Weaken);
            &[]
        }
        "aspdDown" => {
            t.valued = Some(Kind::AspdDown);
            &[]
        }
        "defDown" => {
            t.valued = Some(Kind::DefDown);
            &[]
        }
        "resDown" => {
            t.valued = Some(Kind::ResDown);
            &[]
        }
        "resist" => {
            t.valued = Some(Kind::Resist);
            &[]
        }
        _ => &[key],
    };
    t.flags = flags.iter().map(|s| s.to_string()).collect();
    if let Some(kind) = t.valued {
        t.mods = kind.mods(value.unwrap_or(kind.default_value()));
    }
    t
}
fn resisted(key: &str) -> bool {
    matches!(
        key,
        "stun"
            | "freeze"
            | "cold"
            | "sleep"
            | "fear"
            | "tremble"
            | "attract"
            | "levitate"
            | "bind"
            | "silence"
            | "disarm"
            | "sluggish"
            | "slow"
            | "groundbind"
    )
}
pub fn resist_of(w: &World, unit: Id) -> Result<f64, String> {
    let mut out = 0.0;
    for b in w.unit(unit)?.stats.buffs() {
        if b.runtime.status.as_deref() == Some("resist") {
            let v = b
                .runtime
                .valued
                .as_ref()
                .filter(|e| e.value_present)
                .map(|e| e.value)
                .filter(|v| v.is_finite())
                .unwrap_or(0.5);
            if v > out {
                out = v;
            }
        }
    }
    Ok(out.min(0.95))
}
fn flags(w: &mut World, unit: Id) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(w.unit_mut(unit)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .flags
        .clone())
}
pub fn apply<H: Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    rect: Rect,
    unit: Id,
    key: &str,
    opts: Options,
) -> Result<bool, String> {
    r.call(|r| {
        if !w.unit(unit)?.stats.alive {
            return Ok(false);
        }
        let f = flags(w, unit)?;
        if let Some(src) = opts.source {
            if w.unit(src)?.side != w.unit(unit)?.side
                && f.contains("invulnerable")
                && f.contains("untargetable")
            {
                return Ok(false);
            }
        }
        if !opts.ignore_select
            && f.contains("liftoff")
            && targeting::evades_ground(w, opts.source, unit)?
        {
            return Ok(false);
        }
        let mut tpl = template(key, opts.value);
        let mut duration = opts.duration;
        let mut value = opts.value;
        if !(duration > 0.0) {
            return Ok(false);
        }
        if !opts.force
            && tpl
                .immune
                .map(|k| w.unit(unit).unwrap().immune.contains(k))
                .unwrap_or(false)
        {
            return Ok(false);
        }
        if key == "levitate"
            && (w.unit(unit)?.motion_fly && !f.contains("groundbind") || f.contains("levitate"))
        {
            return Ok(false);
        }
        if h.has_before_status() {
            let mut c = Context {
                source: opts.source,
                target: unit,
                key: key.into(),
                duration,
                value,
                cancelled: false,
                entered: false,
            };
            h.before_status(&mut c, r, w)?;
            if c.cancelled || !w.unit(unit)?.stats.alive {
                return Ok(false);
            }
            if c.duration == f64::INFINITY || c.duration.is_finite() && c.duration > 0.0 {
                duration = c.duration;
            } else if c.duration.is_finite() && c.duration <= 0.0 {
                return Ok(false);
            }
            value = c.value;
            tpl = template(key, value);
        }
        if resisted(key) && !opts.resist_applied {
            let rv = resist_of(w, unit)?;
            if rv > 0.0 {
                duration *= 1.0 - rv;
            }
        }
        if matches!(key, "levitate" | "groundbind")
            && w.unit_mut(unit)?
                .stats
                .stats()
                .map_err(str::to_string)?
                .n("massLevel")
                > 3.0
        {
            duration /= 2.0;
        }
        if !(duration > 0.0) {
            return Ok(false);
        }
        if key == "cold" && !w.unit(unit)?.immune.contains("frozen") {
            if let Some(old) = w
                .unit(unit)?
                .stats
                .buffs()
                .iter()
                .find(|b| b.key.as_deref() == Some("cold"))
            {
                let spans: Vec<_> = [old.runtime.time_left, duration]
                    .into_iter()
                    .filter(|t| *t == f64::INFINITY || t.is_finite() && *t > 0.0)
                    .collect();
                let freeze_for = spans.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let froze = apply(
                    w,
                    h,
                    r,
                    rect,
                    unit,
                    "freeze",
                    Options {
                        duration: if spans.is_empty() { 3.0 } else { freeze_for },
                        source: opts.source,
                        force: opts.force,
                        resist_applied: !spans.is_empty(),
                        ..Default::default()
                    },
                )?;
                if !w.unit(unit)?.stats.alive {
                    return Ok(false);
                }
                if froze && w.unit(unit)?.side == Side::Enemy {
                    r.remove(w, h, unit, "cold")?;
                    return Ok(true);
                }
            }
        }
        let entered = opts.reenter
            || !w
                .unit(unit)?
                .stats
                .buffs()
                .iter()
                .any(|b| b.runtime.status.as_deref().or(b.key.as_deref()) == Some(key));
        if key == "palsy" {
            let n = value
                .filter(|v| v.is_finite())
                .unwrap_or(1.0)
                .floor()
                .clamp(1.0, 3.0) as u32;
            let mut b = Buff {
                key: Some(key.into()),
                stacks: n,
                ..Default::default()
            };
            b.runtime.duration = duration;
            b.runtime.max_stacks = 3;
            b.runtime.refresh = Refresh::Stack;
            b.runtime.status = Some(key.into());
            b.runtime.visible = true;
            b.runtime.source = opts.source;
            r.add(
                w,
                h,
                unit,
                Spec {
                    buff: b,
                    ..Default::default()
                },
            )?;
        } else if let Some(kind) = tpl.valued.filter(|_| opts.refresh.is_none()) {
            valued::apply(
                w,
                h,
                r,
                unit,
                valued::Request {
                    key,
                    kind,
                    plain: false,
                    duration,
                    value: value.unwrap_or(kind.default_value()),
                    source: opts.source,
                    stack_as: opts.stack_as,
                },
            )?;
        } else {
            let mods_present =
                !tpl.mods.is_empty() && (!tpl.enemy_only || w.unit(unit)?.side == Side::Enemy);
            let mut b = Buff {
                key: Some(key.into()),
                mods: if tpl.enemy_only && w.unit(unit)?.side != Side::Enemy {
                    Numbers::new()
                } else {
                    tpl.mods
                },
                flags: tpl.flags.iter().cloned().collect(),
                ..Default::default()
            };
            b.runtime.duration = duration;
            b.runtime.refresh = opts.refresh.unwrap_or(Refresh::Extend);
            b.runtime.status = Some(key.into());
            b.runtime.visible = true;
            b.runtime.source = opts.source;
            let id = r.add(
                w,
                h,
                unit,
                Spec {
                    buff: b,
                    mods_present,
                    flags_present: !tpl.flags.is_empty(),
                    ..Default::default()
                },
            )?;
            if let Some(id) = id {
                if key == "attract" {
                    let point = opts
                        .point
                        .filter(|p| p.iter().all(|n| n.is_finite()))
                        .or(opts
                            .source
                            .map(|id| w.unit(id).map(|u| u.position))
                            .transpose()?
                            .filter(|p| p.iter().all(|n| n.is_finite())));
                    if let Some([row, col]) = point {
                        let tile = [
                            round(row).clamp(rect.r0 as f64, rect.r1 as f64) as i32,
                            round(col).clamp(rect.c0 as f64, rect.c1 as f64) as i32,
                        ];
                        r.edit(w, unit, id, |b| {
                            b.runtime.attract = Some(tile);
                            b.runtime.attract_path = None;
                        })?;
                    }
                }
                if key == "fear" && w.unit(unit)?.side == Side::Enemy {
                    let self_fear = opts.source.is_none() || opts.source == Some(unit);
                    let hit = w.unit(unit)?.position;
                    let source = if self_fear {
                        hit
                    } else {
                        w.unit(opts.source.unwrap())?.position
                    };
                    let sequence = r.next_fear()?;
                    r.edit(w, unit, id, |b| {
                        b.runtime.fear = Some(FearStamp {
                            sequence,
                            hit,
                            source,
                            self_fear,
                        })
                    })?;
                }
            }
        }
        if w.unit(unit)?.side == Side::Enemy
            && tpl
                .flags
                .iter()
                .any(|k| matches!(k.as_str(), "levitate" | "unblockable" | "fear" | "sleep"))
        {
            h.unblock_with_buffs(w, r, unit)?;
        }
        if w.unit(unit)?.side == Side::Ally && tpl.flags.iter().any(|k| k == "noBlock") {
            h.release_with_buffs(w, r, unit)?;
        }
        if h.has_status_applied() {
            h.status_applied(
                &Context {
                    source: opts.source,
                    target: unit,
                    key: key.into(),
                    duration,
                    value,
                    cancelled: false,
                    entered,
                },
                r,
                w,
            )?;
        }
        Ok(true)
    })
}
fn round(x: f64) -> f64 {
    let f = x.floor();
    if x - f < 0.5 {
        f
    } else {
        f + 1.0
    }
}
