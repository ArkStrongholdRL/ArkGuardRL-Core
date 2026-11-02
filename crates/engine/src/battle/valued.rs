// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 battle/status.js _applyValuedStatus.
use super::{
    buffs::{self, Runtime, Spec},
    health::{Id, World},
    stats::{Buff, Numbers},
};
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Slow,
    Fragile,
    ArtsFragile,
    PhysFragile,
    ElemFragile,
    Weaken,
    AspdDown,
    DefDown,
    ResDown,
    Resist,
    Custom(fn(f64) -> Numbers),
}
impl Kind {
    pub fn default_value(self) -> f64 {
        match self {
            Self::Slow | Self::Resist => 0.5,
            Self::ElemFragile => 0.2,
            Self::AspdDown => -30.0,
            Self::ResDown => 20.0,
            _ => 0.3,
        }
    }
    pub fn mods(self, value: f64) -> Numbers {
        let (key, n) = match self {
            Self::Slow => ("moveMul", 1.0 - clamp(value)),
            Self::Weaken => ("atkMul", 1.0 - clamp(value)),
            Self::DefDown => ("defMul", 1.0 - clamp(value)),
            Self::Fragile => ("dmgTakenMul", 1.0 + value),
            Self::ArtsFragile => ("artsTakenMul", 1.0 + value),
            Self::PhysFragile => ("physTakenMul", 1.0 + value),
            Self::ElemFragile => ("elementalTakenMul", 1.0 + value),
            Self::AspdDown => ("aspd", value),
            Self::ResDown => ("resFlat", -value),
            Self::Resist => return Numbers::new(),
            Self::Custom(f) => return f(value),
        };
        [(key.into(), n)].into_iter().collect()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub value: f64,
    pub stack_as: Option<f64>,
    pub until: f64,
}
#[derive(Clone, Debug)]
pub struct Effect {
    pub kind: Kind,
    pub default_value: f64,
    pub plain: bool,
    pub value: f64,
    pub value_present: bool,
    pub stack_as: Option<f64>,
    pub tail: Vec<Entry>,
}
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub key: &'a str,
    pub kind: Kind,
    pub plain: bool,
    pub duration: f64,
    pub value: f64,
    pub source: Option<Id>,
    pub stack_as: Option<f64>,
}
fn strength(value: f64, stack_as: Option<f64>, default: f64) -> f64 {
    stack_as
        .filter(|x| x.is_finite())
        .unwrap_or(if value.is_finite() { value } else { default })
        .abs()
}
fn after(mut tail: Vec<Entry>, until: f64) -> Vec<Entry> {
    let n = tail.iter().take_while(|e| e.until <= until).count();
    tail.drain(..n);
    tail
}
fn insert(mut tail: Vec<Entry>, incoming: Entry, default: f64) -> Vec<Entry> {
    let mut at = 0;
    while at < tail.len() {
        let current = tail[at];
        let diff = strength(incoming.value, incoming.stack_as, default)
            - strength(current.value, current.stack_as, default);
        if diff > 1e-12 {
            let rest = after(tail.split_off(at), incoming.until);
            tail.push(incoming);
            tail.extend(rest);
            return tail;
        }
        if diff < -1e-12 {
            if incoming.until <= current.until {
                return tail;
            }
            at += 1;
            continue;
        }
        let kept = if incoming.until > current.until {
            incoming
        } else {
            current
        };
        let rest = after(tail.split_off(at + 1), kept.until);
        tail.truncate(at);
        tail.push(kept);
        tail.extend(rest);
        return tail;
    }
    tail.push(incoming);
    tail
}
fn make(key: &str, source: Option<Id>, duration: f64, effect: Effect) -> Spec {
    let mut b = Buff {
        key: Some(key.into()),
        mods: effect.kind.mods(effect.value),
        ..Default::default()
    };
    b.runtime.duration = duration;
    b.runtime.source = source;
    b.runtime.visible = !effect.plain;
    b.runtime.status = if effect.plain { None } else { Some(key.into()) };
    b.runtime.resist_decay = matches!(effect.kind, Kind::Resist);
    if b.runtime.resist_decay {
        b.runtime.interval = 5.0;
    }
    b.runtime.valued_expire = true;
    b.runtime.valued = Some(effect);
    Spec {
        buff: b,
        mods_present: true,
        ..Default::default()
    }
}
pub fn apply<H: buffs::Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    unit: Id,
    request: Request<'_>,
) -> Result<(), String> {
    r.call(|r| {
        r.adopt(w, unit)?;
        let Request {
            key,
            kind,
            plain,
            duration,
            value,
            source,
            stack_as,
        } = request;
        let stack_as = stack_as.filter(|x| x.is_finite());
        let default = if plain { value } else { kind.default_value() };
        let old = w
            .unit(unit)?
            .stats
            .buffs()
            .iter()
            .find(|b| {
                b.key.as_deref() == Some(key) && (plain || b.runtime.status.as_deref() == Some(key))
            })
            .cloned();
        let effect = Effect {
            kind,
            default_value: default,
            plain,
            value,
            value_present: true,
            stack_as,
            tail: vec![],
        };
        let Some(old) = old else {
            r.add(w, h, unit, make(key, source, duration, effect))?;
            return Ok(());
        };
        let old_effect = old.runtime.valued.clone();
        let old_v = old_effect
            .as_ref()
            .filter(|e| e.value_present)
            .map(|e| e.value)
            .filter(|v| v.is_finite())
            .unwrap_or(default);
        let old_as = old_effect
            .as_ref()
            .and_then(|e| e.stack_as)
            .filter(|v| v.is_finite());
        let old_tail = old_effect
            .as_ref()
            .map(|e| e.tail.clone())
            .unwrap_or_default();
        let old_end = w.time + old.runtime.time_left;
        let new_end = w.time + duration;
        let diff = strength(value, stack_as, default) - strength(old_v, old_as, default);
        if diff > 1e-12 {
            let mut tail = vec![Entry {
                value: old_v,
                stack_as: old_as,
                until: old_end,
            }];
            tail.extend(old_tail);
            let tail = after(tail, new_end);
            r.check_tail(tail.len())?;
            r.add(
                w,
                h,
                unit,
                make(key, source, duration, Effect { tail, ..effect }),
            )?;
        } else if diff < -1e-12 {
            if new_end > old_end {
                let tail = insert(
                    old_tail,
                    Entry {
                        value,
                        stack_as,
                        until: new_end,
                    },
                    default,
                );
                r.check_tail(tail.len())?;
                let mut kept = old_effect.unwrap_or(Effect {
                    value: old_v,
                    stack_as: old_as,
                    ..effect
                });
                kept.value = old_v;
                kept.value_present = true;
                kept.tail = tail;
                r.edit(w, unit, old.runtime.id(), |b| b.runtime.valued = Some(kept))?;
            }
        } else if duration > old.runtime.time_left {
            r.edit(w, unit, old.runtime.id(), |b| {
                b.runtime.time_left = duration;
                b.runtime.duration = if b.runtime.duration.is_nan() || duration.is_nan() {
                    f64::NAN
                } else {
                    b.runtime.duration.max(duration)
                };
                let mut kept = old_effect.unwrap_or(Effect {
                    value_present: false,
                    value: old_v,
                    stack_as: old_as,
                    ..effect
                });
                kept.tail = after(old_tail, new_end);
                b.runtime.valued = Some(kept);
            })?;
        }
        Ok(())
    })
}
pub(crate) fn expire<H: buffs::Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    unit: Id,
    id: buffs::BuffId,
) -> Result<(), String> {
    let b = r.get(w, unit, id)?;
    let Some(mut effect) = b.runtime.valued.clone() else {
        return Ok(());
    };
    let key = b.key.clone().unwrap_or_else(|| "buff".into());
    let source = b.runtime.source;
    effect.tail = after(effect.tail, w.time + 1e-6);
    if !effect.tail.is_empty() && w.unit(unit)?.stats.alive {
        let first = effect.tail.remove(0);
        effect.value = first.value;
        effect.value_present = true;
        effect.stack_as = first.stack_as;
        r.add(w, h, unit, make(&key, source, first.until - w.time, effect))?;
    }
    Ok(())
}
pub(crate) fn decay_palsy<H: buffs::Hooks>(
    w: &mut World,
    h: &mut H,
    r: &mut Runtime,
    unit: Id,
) -> Result<(), String> {
    if let Some(b) = w
        .unit(unit)?
        .stats
        .buffs()
        .iter()
        .find(|b| b.key.as_deref() == Some("palsy"))
        .cloned()
    {
        if b.stacks <= 1 {
            r.remove_id(w, h, unit, b.runtime.id())?;
        } else {
            r.edit(w, unit, b.runtime.id(), |b| b.stacks -= 1)?;
        }
    }
    Ok(())
}
fn clamp(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}
