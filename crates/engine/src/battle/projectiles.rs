// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 ProjectileSystem: flight and callback ordering.
use super::health::{Id, World};
use crate::deterministic::math::hypot;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Launch<P> {
    pub from: [f64; 2],
    pub source: Option<Id>,
    pub target: Option<Id>,
    pub to: Option<[f64; 2]>,
    pub speed: f64,
    pub hit_dead: bool,
    pub max_age: f64,
    pub on_hit: bool,
    pub visual: String,
    pub speed_changeable: Option<bool>,
    pub removable: Option<bool>,
    pub payload: P,
}
#[derive(Debug)]
pub struct Projectile<P> {
    pub id: u64,
    pub position: [f64; 2],
    pub source: Option<Id>,
    pub target: Option<Id>,
    pub target_sequence: u64,
    pub destination: [f64; 2],
    pub speed: f64,
    pub hit_dead: bool,
    pub age: f64,
    pub max_age: f64,
    pub on_hit: bool,
    pub visual: String,
    pub speed_changeable: bool,
    pub removable: bool,
    pub payload: P,
    pins: [Option<Id>; 2],
}
pub trait Hooks<P>: Sized {
    fn has_step(&self) -> bool;
    fn step(
        &mut self,
        p: &mut Projectile<P>,
        system: &mut System<P>,
        w: &mut World,
    ) -> Result<bool, String>;
    fn impact(
        &mut self,
        p: &mut Projectile<P>,
        target: Option<Id>,
        system: &mut System<P>,
        w: &mut World,
    ) -> Result<(), String>;
}
pub struct System<P> {
    list: Vec<Projectile<P>>,
    sequence: u64,
    capacity: usize,
    reserved: usize,
    updating: bool,
    faulted: bool,
    pins: BTreeMap<Id, usize>,
}
fn finite(x: f64, fallback: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        fallback
    }
}
impl<P> System<P> {
    pub fn new(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("Projectile capacity must be positive".into());
        }
        Ok(Self {
            list: vec![],
            sequence: 0,
            capacity,
            reserved: 0,
            updating: false,
            faulted: false,
            pins: BTreeMap::new(),
        })
    }
    pub fn list(&self) -> &[Projectile<P>] {
        &self.list
    }
    pub fn faulted(&self) -> bool {
        self.faulted
    }
    fn pin(&mut self, w: &mut World, id: Id) -> Result<(), String> {
        w.pin_unit(id)?;
        *self.pins.entry(id).or_default() += 1;
        Ok(())
    }
    fn unpin(&mut self, w: &mut World, id: Id) -> Result<(), String> {
        w.unpin_unit(id)?;
        let count = self
            .pins
            .get_mut(&id)
            .ok_or("Missing projectile retention")?;
        *count = count
            .checked_sub(1)
            .ok_or("Projectile retention underflow")?;
        if *count == 0 {
            self.pins.remove(&id);
        }
        Ok(())
    }
    fn release(&mut self, w: &mut World, p: &Projectile<P>) -> Result<(), String> {
        for id in p.pins.into_iter().flatten() {
            self.unpin(w, id)?;
        }
        Ok(())
    }
    fn sync_pins(&mut self, w: &mut World, p: &mut Projectile<P>) -> Result<(), String> {
        for (i, next) in [p.source, p.target].into_iter().enumerate() {
            if next == p.pins[i] {
                continue;
            }
            if let Some(id) = next {
                self.pin(w, id)?;
            }
            if let Some(id) = p.pins[i] {
                self.unpin(w, id)?;
            }
            p.pins[i] = next;
        }
        Ok(())
    }
    pub fn add(&mut self, w: &mut World, p: Launch<P>) -> Result<u64, String> {
        if self.faulted {
            return Err("Projectile system is faulted".into());
        }
        if self.list.len() + self.reserved >= self.capacity {
            return Err("Projectile capacity exceeded".into());
        }
        let from = p.from.map(|x| finite(x, 0.0));
        let (target_pos, sequence) = if let Some(id) = p.target {
            let u = w.unit(id)?;
            (u.position, u.deployment.deploy_seq)
        } else {
            (from, 0)
        };
        if let Some(id) = p.source {
            w.unit(id)?;
        }
        let to = p.to.unwrap_or(target_pos);
        let destination = [finite(to[0], from[0]), finite(to[1], from[1])];
        let id = self
            .sequence
            .checked_add(1)
            .ok_or("Projectile identity exhausted")?;
        if let Some(id) = p.source {
            self.pin(w, id)?;
        }
        if let Some(id) = p.target {
            if let Err(e) = self.pin(w, id) {
                if let Some(id) = p.source {
                    self.unpin(w, id)?;
                }
                return Err(e);
            }
        }
        self.sequence = id;
        let speed_changeable = p
            .speed_changeable
            .unwrap_or(p.visual != "none" && !p.visual.is_empty());
        self.list.push(Projectile {
            id,
            position: from,
            source: p.source,
            target: p.target,
            target_sequence: sequence,
            destination,
            speed: if p.speed.is_finite() && p.speed >= 0.0 {
                p.speed
            } else {
                12.0
            },
            hit_dead: p.hit_dead,
            age: 0.0,
            max_age: if p.max_age > 0.0 { p.max_age } else { 10.0 },
            on_hit: p.on_hit,
            speed_changeable,
            removable: p.removable.unwrap_or(speed_changeable),
            visual: p.visual,
            payload: p.payload,
            pins: [p.source, p.target],
        });
        Ok(id)
    }
    pub fn clear(&mut self, w: &mut World) -> Result<(), String> {
        for p in std::mem::take(&mut self.list) {
            self.release(w, &p)?;
        }
        Ok(())
    }
    pub fn update<H: Hooks<P>>(&mut self, w: &mut World, h: &mut H, dt: f64) -> Result<(), String> {
        if self.faulted {
            return Err("Projectile system is faulted".into());
        }
        if self.updating {
            return Err("Reentrant projectile update".into());
        }
        if !dt.is_finite() || dt < 0.0 {
            return Err("Invalid projectile timestep".into());
        }
        self.updating = true;
        let result = self.update_inner(w, h, dt);
        self.updating = false;
        if result.is_err() {
            self.faulted = true;
            self.list.clear();
            self.reserved = 0;
            for (id, count) in std::mem::take(&mut self.pins) {
                for _ in 0..count {
                    w.unpin_unit(id)?;
                }
            }
        }
        result
    }
    fn update_inner<H: Hooks<P>>(
        &mut self,
        w: &mut World,
        h: &mut H,
        dt: f64,
    ) -> Result<(), String> {
        let moving = std::mem::take(&mut self.list);
        self.reserved = moving.len();
        let mut keep = Vec::new();
        let mut arrived = Vec::new();
        for mut p in moving {
            p.age += dt;
            if let Some(id) = p.target {
                let u = w.unit(id)?;
                if u.stats.alive && !u.hidden && u.deployment.deploy_seq == p.target_sequence {
                    p.destination = [
                        finite(u.position[0], p.destination[0]),
                        finite(u.position[1], p.destination[1]),
                    ];
                } else if !p.hit_dead {
                    self.reserved -= 1;
                    self.release(w, &p)?;
                    continue;
                } else {
                    p.target = None;
                }
            }
            let dy = p.destination[0] - p.position[0];
            let dx = p.destination[1] - p.position[1];
            let distance = hypot(dx, dy);
            let step = p.speed * dt;
            let lands = distance <= step || p.age >= p.max_age;
            if lands {
                p.position = p.destination;
            } else {
                p.position[1] += (dx / distance) * step;
                p.position[0] += (dy / distance) * step;
            }
            let cancel = h.has_step() && h.step(&mut p, self, w)?;
            self.sync_pins(w, &mut p)?;
            if cancel {
                self.reserved -= 1;
                self.release(w, &p)?;
                continue;
            }
            if lands {
                arrived.push(p);
            } else {
                keep.push(p);
            }
        }
        let spawned = std::mem::take(&mut self.list);
        keep.extend(spawned);
        self.list = keep;
        self.reserved = arrived.len();
        for mut p in arrived {
            self.reserved -= 1;
            if p.on_hit {
                let target = if let Some(id) = p.target {
                    let u = w.unit(id)?;
                    if u.stats.alive && u.deployment.deploy_seq == p.target_sequence {
                        Some(id)
                    } else {
                        None
                    }
                } else {
                    None
                };
                h.impact(&mut p, target, self, w)?;
            }
            self.release(w, &p)?;
        }
        Ok(())
    }
}
