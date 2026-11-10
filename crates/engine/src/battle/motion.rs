// SPDX-License-Identifier: GPL-3.0-or-later
// Stronghold Protocol 41bbca57 ai.js route/attract movement and fear.js.
// Attack, stun, blocking and displacement arbitration remains the enemy update's job.
use super::{
    buffs::Runtime,
    grid::{Flow, Grid, Rect, COLS, N},
    health::{Id, World},
    stats::Stat,
    status::FearStamp,
};
use crate::deterministic::{math::hypot, rng::Rng};
use serde_json::Value;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub enum Leg {
    Move { tile: [i32; 2], final_leg: bool },
    Wait(f64),
    Disappear,
    Appear([i32; 2]),
}
#[derive(Clone, Debug)]
pub struct Route {
    pub(crate) patrol: bool,
    capacity: usize,
    legs: Vec<Leg>,
    pub leg: usize,
    pub wait_left: Option<f64>,
    pub points: Option<Vec<[f64; 2]>>,
    pub point: usize,
    pub version: u64,
    suffix: Vec<f64>,
    tail: Option<Vec<f64>>,
    tail_version: u64,
    pub spawn: [f64; 2],
}
impl Route {
    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }
    pub fn compile(
        data: &Value,
        rect: Rect,
        capacity: usize,
        spawn: [f64; 2],
    ) -> Result<Self, String> {
        let mut legs = vec![];
        let tile = |v: &Value| -> Result<[i32; 2], String> {
            let coord = |n: &Value| {
                n.as_f64()
                    .filter(|n| {
                        n.is_finite()
                            && n.fract() == 0.0
                            && *n >= i32::MIN as f64
                            && *n <= i32::MAX as f64
                    })
                    .map(|n| n as i32)
                    .ok_or_else(|| "Route coordinates must be i32 tiles".to_string())
            };
            Ok([
                coord(&v[0])?.clamp(rect.r0, rect.r1),
                coord(&v[1])?.clamp(rect.c0, rect.c1),
            ])
        };
        if capacity == 0
            || !spawn.iter().all(|x| x.is_finite())
            || !Grid::in_bounds([rect.r0, rect.c0])
            || !Grid::in_bounds([rect.r1, rect.c1])
            || rect.r0 > rect.r1
            || rect.c0 > rect.c1
        {
            return Err("Invalid route limits/spawn".into());
        }
        if let Some(checkpoints) = data["checkpoints"].as_array() {
            for cp in checkpoints {
                let leg = match cp["type"].as_str() {
                    Some("MOVE") => Leg::Move {
                        tile: tile(&cp["pos"])?,
                        final_leg: false,
                    },
                    Some("WAIT") => Leg::Wait(
                        cp["time"]
                            .as_f64()
                            .filter(|x| x.is_finite())
                            .unwrap_or(0.0)
                            .max(0.0),
                    ),
                    Some("DISAPPEAR") => Leg::Disappear,
                    Some("APPEAR") => Leg::Appear(tile(&cp["pos"])?),
                    _ => continue,
                };
                if legs.len() >= capacity {
                    return Err("Route leg capacity exceeded; no checkpoints dropped".into());
                }
                legs.push(leg);
            }
        }
        if !data["end"].is_null() {
            if legs.len() >= capacity {
                return Err("Route leg capacity exceeded; no checkpoints dropped".into());
            }
            legs.push(Leg::Move {
                tile: tile(&data["end"])?,
                final_leg: true,
            });
        }
        Ok(Self {
            patrol: false,
            capacity,
            legs,
            leg: 0,
            wait_left: None,
            points: None,
            point: 0,
            version: 0,
            suffix: vec![],
            tail: None,
            tail_version: 0,
            spawn,
        })
    }
    pub(crate) fn after_wait(&self, spawn: [f64; 2], rect: Rect) -> Result<Self, String> {
        let mut next = self.remaining(spawn, rect)?;
        if matches!(next.legs.first(), Some(Leg::Wait(_))) {
            next.legs.remove(0);
        }
        Ok(next)
    }
    pub(crate) fn stationary(&self, spawn: [f64; 2], rect: Rect) -> Result<Self, String> {
        let mut next = self.remaining(spawn, rect)?;
        if next.capacity < 2 {
            return Err("Stationary child route exceeds leg capacity".into());
        }
        next.legs = vec![
            Leg::Wait(99999.),
            Leg::Move {
                tile: [
                    (spawn[0] + 0.5)
                        .floor()
                        .clamp(rect.r0 as f64, rect.r1 as f64) as i32,
                    (spawn[1] + 0.5)
                        .floor()
                        .clamp(rect.c0 as f64, rect.c1 as f64) as i32,
                ],
                final_leg: true,
            },
        ];
        Ok(next)
    }
    /// A child starts a fresh movement cursor at the parent's remaining legs.
    pub(crate) fn remaining(&self, spawn: [f64; 2], rect: Rect) -> Result<Self, String> {
        let mut legs: Vec<_> = self.legs.iter().skip(self.leg).cloned().collect();
        if let Some(Leg::Wait(time)) = legs.first_mut() {
            *time = self.wait_left.unwrap_or(*time).max(0.);
        }
        if !legs.iter().any(|l| {
            matches!(
                l,
                Leg::Move {
                    final_leg: true,
                    ..
                }
            )
        }) {
            legs.push(Leg::Move {
                tile: [
                    (spawn[0].round() as i32).clamp(rect.r0, rect.r1),
                    (spawn[1].round() as i32).clamp(rect.c0, rect.c1),
                ],
                final_leg: true,
            });
        }
        if legs.len() > self.capacity {
            return Err("Route leg capacity exceeded; no checkpoints dropped".into());
        }
        Ok(Self {
            patrol: self.patrol,
            capacity: self.capacity,
            legs,
            leg: 0,
            wait_left: None,
            points: None,
            point: 0,
            version: 0,
            suffix: vec![],
            tail: None,
            tail_version: 0,
            spawn,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Attract {
    pub points: Vec<[f64; 2]>,
    pub point: usize,
    pub version: u64,
    pub previous: Option<[f64; 2]>,
}
#[derive(Clone, Debug)]
pub struct Fear {
    pub sequence: Option<u64>,
    pub tiles: Vec<usize>,
    pub points: Option<Vec<[f64; 2]>>,
    pub point: usize,
    pub version: u64,
    pub previous: Option<[f64; 2]>,
    pub key: usize,
    pub goal: [f64; 2],
}
#[derive(Debug)]
pub struct State {
    pub route: Option<Route>,
    pub fear: Option<Fear>,
    pub attack_stand_until: f64,
    pub unbalance_until: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            route: None,
            fear: None,
            attack_stand_until: f64::NEG_INFINITY,
            unbalance_until: f64::NEG_INFINITY,
        }
    }
}
pub trait Hooks {
    fn check_block(&mut self, id: Id, w: &mut World, g: &mut Grid) -> Result<bool, String>;
    fn hidden(&mut self, id: Id, on: bool, w: &mut World) -> Result<(), String>;
    fn cut_attack_stand(&mut self, id: Id, w: &mut World) -> Result<(), String>;
    fn portal_pickup(&mut self, id: Id, w: &mut World, g: &mut Grid) -> Result<bool, String> {
        portal_pickup(w, g, id)
    }
    fn leak(&mut self, id: Id, w: &mut World, rng: &mut Rng) -> Result<(), String>;
}
fn round(x: f64) -> f64 {
    let f = x.floor();
    if x - f < 0.5 {
        f
    } else {
        f + 1.0
    }
}
fn key(tile: [i32; 2]) -> usize {
    tile[0] as usize * COLS + tile[1] as usize
}
fn coord(k: usize) -> [i32; 2] {
    [(k / COLS) as i32, (k % COLS) as i32]
}
fn centre(tile: [i32; 2]) -> [f64; 2] {
    tile.map(|x| x as f64)
}
fn here(g: &Grid, p: [f64; 2]) -> [i32; 2] {
    let r = g.rect();
    [
        round(p[0]).clamp(r.r0 as f64, r.r1 as f64) as i32,
        round(p[1]).clamp(r.c0 as f64, r.c1 as f64) as i32,
    ]
}
fn raw_tile(p: [f64; 2]) -> [i32; 2] {
    [round(p[0]) as i32, round(p[1]) as i32]
}
fn field(g: &mut Grid, tile: [i32; 2], start: [i32; 2]) -> Option<Arc<Flow>> {
    let f = g.flow_field(tile, true, false);
    if Grid::in_bounds(start) && f.dist[key(start)] >= 0 {
        return Some(f);
    }
    let f = g.flow_field(tile, true, true);
    if Grid::in_bounds(start) && f.dist[key(start)] >= 0 {
        Some(f)
    } else {
        None
    }
}
pub fn portal_pickup(w: &mut World, g: &Grid, id: Id) -> Result<bool, String> {
    let tile = raw_tile(w.unit(id)?.position);
    if !Grid::in_bounds(tile) || g.tiles()[key(tile)].special.as_deref() != Some("telin") {
        return Ok(false);
    }
    let special = |name: &str| {
        g.tiles()
            .iter()
            .enumerate()
            .filter(|(k, t)| {
                let [row, col] = coord(*k);
                let rect = g.rect();
                t.special.as_deref() == Some(name)
                    && row >= rect.r0
                    && row <= rect.r1
                    && col >= rect.c0
                    && col <= rect.c1
            })
            .map(|(k, _)| coord(k))
            .collect::<Vec<_>>()
    };
    let outs = special("telout");
    let Some(mut out) = outs.first().copied() else {
        return Ok(false);
    };
    let distance = |p: [i32; 2]| hypot((p[0] - tile[0]) as f64, (p[1] - tile[1]) as f64);
    for p in outs.into_iter().skip(1) {
        if distance(p) > distance(out) {
            out = p;
        }
    }
    let ends = special("end");
    let mut door = ends.first().copied();
    for p in ends.into_iter().skip(1) {
        if door.map(|d| distance(p) < distance(d)).unwrap_or(true) {
            door = Some(p);
        }
    }
    let r = route_mut(w, id)?;
    let extra = 3 + usize::from(door.is_some());
    if extra > r.capacity.saturating_sub(r.legs.len()) {
        return Err("Portal route capacity exceeded; no legs dropped".into());
    }
    let at = (r.leg + 1).min(r.legs.len());
    r.legs
        .splice(at..at, [Leg::Disappear, Leg::Wait(3.0), Leg::Appear(out)]);
    if let Some(tile) = door {
        r.legs.push(Leg::Move {
            tile,
            final_leg: true,
        });
    }
    r.points = None;
    r.tail = None;
    Ok(true)
}
fn route(w: &World, id: Id) -> Result<&Route, String> {
    w.unit(id)?
        .motion
        .route
        .as_ref()
        .ok_or_else(|| "Enemy route is missing".into())
}
fn route_mut(w: &mut World, id: Id) -> Result<&mut Route, String> {
    w.unit_mut(id)?
        .motion
        .route
        .as_mut()
        .ok_or_else(|| "Enemy route is missing".into())
}
fn plan_leg(w: &mut World, g: &mut Grid, id: Id, tile: [i32; 2]) -> Result<(), String> {
    let e = w.unit(id)?;
    let position = e.position;
    let fly = e.motion_fly;
    let mut points = vec![];
    if fly {
        points.push(centre(tile));
    } else {
        let start = raw_tile(position);
        let f = field(g, tile, start);
        if let Some(ref f) = f {
            let mut x = key(start);
            for _ in 0..COLS * 32 {
                if x == f.dest as usize {
                    break;
                }
                let next = f.next[x];
                if next < 0 {
                    break;
                }
                x = next as usize;
                points.push(centre(coord(x)));
            }
        }
        if points.last() != Some(&centre(tile)) {
            points.push(centre(tile));
        }
        if f.is_some()
            && position != centre(start)
            && !g.straight_clear(position[1], position[0], raw_tile(points[0]))
        {
            points.insert(0, centre(start));
        }
    }
    if points.len() > N + 1 {
        return Err("Route waypoint capacity exceeded".into());
    }
    let mut suffix = vec![0.0; points.len()];
    for i in (0..points.len().saturating_sub(1)).rev() {
        suffix[i] = suffix[i + 1]
            + hypot(
                points[i + 1][1] - points[i][1],
                points[i + 1][0] - points[i][0],
            );
    }
    let r = route_mut(w, id)?;
    r.points = Some(points);
    r.point = 0;
    r.version = g.version();
    r.suffix = suffix;
    Ok(())
}
pub fn remaining_distance(w: &mut World, g: &mut Grid, id: Id) -> Result<f64, String> {
    if w.unit(id)?.motion.route.is_none() {
        return Ok(0.0);
    }
    if route(w, id)?.tail.is_none() || route(w, id)?.tail_version != g.version() {
        let e = w.unit(id)?;
        let fly = e.motion_fly;
        let r = route(w, id)?;
        let mut prev = raw_tile(r.spawn);
        let mut starts = vec![];
        for leg in &r.legs {
            starts.push(prev);
            match leg {
                Leg::Move { tile, .. } | Leg::Appear(tile) => prev = *tile,
                _ => {}
            }
        }
        let legs = r.legs.clone();
        let mut tail = vec![0.0; legs.len() + 1];
        for i in (0..legs.len()).rev() {
            let mut len = 0.0;
            if let Leg::Move { tile, .. } = legs[i] {
                let start = starts[i];
                len = hypot((tile[0] - start[0]) as f64, (tile[1] - start[1]) as f64);
                if !fly && Grid::in_bounds(start) {
                    if let Some(f) = field(g, tile, start) {
                        let distance = Grid::field_length(&f, start);
                        if distance.is_finite() {
                            len = distance;
                        }
                    }
                }
            }
            tail[i] = tail[i + 1] + len;
        }
        let r = route_mut(w, id)?;
        r.tail = Some(tail);
        r.tail_version = g.version();
    }
    let r = route(w, id)?;
    let mut d = *r.tail.as_ref().unwrap().get(r.leg + 1).unwrap_or(&0.0);
    if let Some(Leg::Move { tile, .. }) = r.legs.get(r.leg) {
        let p = w.unit(id)?.position;
        if let Some(points) = r.points.as_ref().filter(|p| r.point < p.len()) {
            let aim = points[r.point];
            d += hypot(aim[1] - p[1], aim[0] - p[0]) + r.suffix[r.point];
        } else {
            d += hypot(tile[1] as f64 - p[1], tile[0] as f64 - p[0]);
        }
    }
    Ok(d)
}
/// The route's original loop limits are gameplay behavior, not a rollout truncation.
pub fn advance<H: Hooks>(
    w: &mut World,
    g: &mut Grid,
    h: &mut H,
    id: Id,
    dt: f64,
    mut standing: bool,
    rng: &mut Rng,
) -> Result<(), String> {
    if !dt.is_finite() || dt < 0.0 {
        return Err("Invalid movement tick".into());
    }
    let mut budget = dt;
    for _ in 0..16 {
        if budget <= 1e-9 || !w.unit(id)?.stats.alive {
            break;
        }
        let leg = route(w, id)?.legs.get(route(w, id)?.leg).cloned();
        let Some(leg) = leg else {
            if !h.portal_pickup(id, w, g)? {
                h.leak(id, w, rng)?;
                return Ok(());
            }
            continue;
        };
        match leg {
            Leg::Wait(time) => {
                let r = route_mut(w, id)?;
                let left = r.wait_left.get_or_insert(time);
                let used = budget.min(*left);
                *left -= used;
                budget -= used;
                if *left <= 1e-9 {
                    r.wait_left = None;
                    r.leg += 1;
                    r.points = None;
                }
                w.unit_mut(id)?.moving = false;
            }
            Leg::Disappear => {
                h.cut_attack_stand(id, w)?;
                h.hidden(id, true, w)?;
                w.unit_mut(id)?.motion.attack_stand_until = f64::NEG_INFINITY;
                standing = false;
                let r = route_mut(w, id)?;
                r.leg += 1;
                r.points = None;
            }
            Leg::Appear(tile) => {
                w.unit_mut(id)?.position = centre(tile);
                h.hidden(id, false, w)?;
                w.unit_mut(id)?.motion.unbalance_until = f64::NEG_INFINITY;
                let r = route_mut(w, id)?;
                r.leg += 1;
                r.points = None;
            }
            Leg::Move { tile, final_leg } => {
                if standing {
                    w.unit_mut(id)?.moving = false;
                    return Ok(());
                }
                if route(w, id)?.points.is_none() || route(w, id)?.version != g.version() {
                    plan_leg(w, g, id, tile)?;
                }
                let speed = w
                    .unit_mut(id)?
                    .stats
                    .stats()
                    .map_err(str::to_string)?
                    .get(Stat::MoveSpeed)
                    * 0.5;
                if speed <= 0.0 {
                    w.unit_mut(id)?.moving = false;
                    return Ok(());
                }
                let mut distance = speed * budget;
                w.unit_mut(id)?.moving = true;
                for _ in 0..64 {
                    if distance <= 1e-9 {
                        break;
                    }
                    if route(w, id)?.points.is_none() || route(w, id)?.version != g.version() {
                        plan_leg(w, g, id, tile)?;
                    }
                    let r = route(w, id)?;
                    let Some(p) = r.points.as_ref().and_then(|p| p.get(r.point)).copied() else {
                        break;
                    };
                    let (left, reached, _) = step(w, id, p, distance)?;
                    distance = left;
                    if reached {
                        route_mut(w, id)?.point += 1;
                    }
                    if h.check_block(id, w, g)? {
                        return Ok(());
                    }
                }
                budget = distance / speed;
                let r = route(w, id)?;
                if r.points
                    .as_ref()
                    .map(|p| r.point >= p.len())
                    .unwrap_or(false)
                {
                    if final_leg && !h.portal_pickup(id, w, g)? {
                        h.leak(id, w, rng)?;
                        return Ok(());
                    }
                    let r = route_mut(w, id)?;
                    r.leg += 1;
                    r.points = None;
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn step_position(
    mut position: [f64; 2],
    p: [f64; 2],
    distance: f64,
) -> ([f64; 2], f64, bool, f64) {
    let dy = p[0] - position[0];
    let dx = p[1] - position[1];
    let d = hypot(dx, dy);
    if d <= distance {
        (p, distance - d, true, d)
    } else {
        position[1] += (dx / d) * distance;
        position[0] += (dy / d) * distance;
        (position, 0.0, false, d)
    }
}
fn step(w: &mut World, id: Id, p: [f64; 2], distance: f64) -> Result<(f64, bool, f64), String> {
    let u = w.unit_mut(id)?;
    let (position, left, reached, used) = step_position(u.position, p, distance);
    u.position = position;
    Ok((left, reached, used))
}
fn open(g: &Grid, fly: bool, k: usize) -> bool {
    let tile = coord(k);
    (if fly {
        g.fly_passable(tile)
    } else {
        g.walkable(tile, true)
    }) && (fly || g.obstacle(tile) == 0)
}
pub fn reachable_tiles(
    w: &World,
    g: &mut Grid,
    id: Id,
    stamp: Option<FearStamp>,
) -> Result<Vec<usize>, String> {
    let Some(s) = stamp.filter(|s| !s.self_fear) else {
        return Ok(vec![]);
    };
    let dx = s.hit[1] - s.source[1];
    let dy = s.hit[0] - s.source[0];
    let d = hypot(dx, dy);
    if !(d > 1e-9) {
        return Ok(vec![]);
    }
    let ux = dx / d;
    let uy = dy / d;
    let fly = w.unit(id)?.motion_fly;
    let goal = w.unit(id)?.motion.route.as_ref().and_then(|r| {
        r.legs.iter().rev().find_map(|l| {
            if let Leg::Move {
                tile,
                final_leg: true,
            } = l
            {
                Some(*tile)
            } else {
                None
            }
        })
    });
    let fields = if !fly {
        goal.map(|p| (g.flow_field(p, true, false), g.flow_field(p, true, true)))
    } else {
        None
    };
    let mut out = vec![];
    let rect = g.rect();
    for r in rect.r0..=rect.r1 {
        for c in rect.c0..=rect.c1 {
            let vx = c as f64 - s.hit[1];
            let vy = r as f64 - s.hit[0];
            let len = hypot(vx, vy);
            if len > 10.0 + 1e-9
                || len > 1e-9 && vx * ux + vy * uy < std::f64::consts::FRAC_1_SQRT_2 * len - 1e-9
            {
                continue;
            }
            let p = [r, c];
            let k = key(p);
            if !(if fly {
                g.fly_passable(p)
            } else {
                g.walkable(p, true)
            }) || g.tiles()[k].special.as_deref() == Some("end")
            {
                continue;
            }
            if fields
                .as_ref()
                .map(|(a, b)| a.dist[k] < 0 && b.dist[k] < 0)
                .unwrap_or(false)
            {
                continue;
            }
            out.push(k);
        }
    }
    Ok(out)
}
fn steps_between(g: &Grid, fly: bool, from: usize, to: usize) -> usize {
    if from == to {
        return 0;
    }
    if !open(g, fly, to) {
        return usize::MAX;
    }
    let mut frontier = vec![from];
    let mut seen = [false; N];
    seen[from] = true;
    for d in 1..=5 {
        let mut next = vec![];
        for k in frontier {
            let [r, c] = coord(k);
            for [dr, dc] in [[1, 0], [0, 1], [-1, 0], [0, -1]] {
                let p = [r + dr, c + dc];
                if p[0] < g.rect().r0
                    || p[0] > g.rect().r1
                    || p[1] < g.rect().c0
                    || p[1] > g.rect().c1
                {
                    continue;
                }
                let k = key(p);
                if seen[k] {
                    continue;
                }
                if k == to {
                    return d;
                }
                seen[k] = true;
                if open(g, fly, k) {
                    next.push(k);
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    usize::MAX
}
fn fear_mut(w: &mut World, id: Id) -> Result<&mut Fear, String> {
    w.unit_mut(id)?
        .motion
        .fear
        .as_mut()
        .ok_or_else(|| "Missing fear controller".into())
}
fn plan_fear(w: &mut World, g: &mut Grid, id: Id) -> Result<bool, String> {
    let e = w.unit(id)?;
    let position = e.position;
    let tile = here(g, position);
    let fly = e.motion_fly;
    let m = e.motion.fear.as_ref().ok_or("Missing fear controller")?;
    let target = coord(m.key);
    let goal = m.goal;
    let mut points = vec![];
    if !fly && m.key != key(tile) {
        if !open(g, false, m.key) {
            return Ok(false);
        }
        let Some(path) = g.waypoints(tile, target, true, false) else {
            return Ok(false);
        };
        points.extend(
            path.iter()
                .skip(1)
                .take(path.len().saturating_sub(2))
                .copied()
                .map(centre),
        );
        let first = points.first().copied().unwrap_or(centre(target));
        if position != centre(tile) && !g.straight_clear(position[1], position[0], raw_tile(first))
        {
            points.insert(0, centre(tile));
        }
    }
    points.push(goal);
    let m = fear_mut(w, id)?;
    m.points = Some(points);
    m.point = 0;
    m.version = g.version();
    Ok(true)
}
fn pick(w: &mut World, g: &mut Grid, rng: &mut Rng, id: Id) -> Result<(), String> {
    let e = w.unit(id)?;
    let here = key(here(g, e.position));
    let fly = e.motion_fly;
    let n = e
        .motion
        .fear
        .as_ref()
        .ok_or("Missing fear controller")?
        .tiles
        .len();
    let mut k = here;
    if n > 0 {
        let index = (rng.next_f64() * n as f64).floor() as usize;
        let cand = fear_mut(w, id)?.tiles[index];
        if steps_between(g, fly, here, cand) <= 5 {
            k = cand;
        } else {
            fear_mut(w, id)?.tiles.remove(index);
        }
    }
    let ox = (rng.next_f64() - 0.5) * 0.5;
    let oy = (rng.next_f64() - 0.5) * 0.5;
    for target in [k, here] {
        let tile = coord(target);
        let m = fear_mut(w, id)?;
        m.key = target;
        m.goal = [tile[0] as f64 + oy, tile[1] as f64 + ox];
        if plan_fear(w, g, id)? {
            break;
        }
    }
    Ok(())
}
pub fn move_feared(
    w: &mut World,
    g: &mut Grid,
    rng: &mut Rng,
    id: Id,
    dt: f64,
) -> Result<(), String> {
    if !dt.is_finite() || dt < 0.0 {
        return Err("Invalid fear tick".into());
    }
    let stamp = w
        .unit(id)?
        .stats
        .buffs()
        .iter()
        .filter(|b| b.runtime.status.as_deref().or(b.key.as_deref()) == Some("fear"))
        .filter_map(|b| b.runtime.fear)
        .max_by_key(|s| s.sequence);
    let stale = w
        .unit(id)?
        .motion
        .fear
        .as_ref()
        .map(|m| {
            stamp
                .map(|s| m.sequence != Some(s.sequence))
                .unwrap_or(false)
        })
        .unwrap_or(true);
    if stale {
        let tiles = reachable_tiles(w, g, id, stamp)?;
        w.unit_mut(id)?.motion.fear = Some(Fear {
            sequence: stamp.map(|s| s.sequence),
            tiles,
            points: None,
            point: 0,
            version: 0,
            previous: None,
            key: 0,
            goal: [0.0; 2],
        });
    }
    let e = w.unit(id)?;
    let m = e.motion.fear.as_ref().unwrap();
    if m.points.is_some()
        && (m.version != g.version() || m.previous != Some(e.position))
        && !plan_fear(w, g, id)?
    {
        fear_mut(w, id)?.points = None;
    }
    if let Some(r) = w.unit_mut(id)?.motion.route.as_mut() {
        if let Some(Leg::Wait(time)) = r.legs.get(r.leg) {
            let left = r.wait_left.get_or_insert(*time);
            *left -= dt;
            if *left <= 1e-9 {
                r.wait_left = None;
                r.leg += 1;
            }
        }
        r.points = None;
    }
    let speed = w
        .unit_mut(id)?
        .stats
        .stats()
        .map_err(str::to_string)?
        .get(Stat::MoveSpeed)
        * 0.5;
    let mut moved = false;
    if speed > 0.0 {
        let mut distance = speed * dt;
        for _ in 0..8 {
            if distance <= 1e-9 {
                break;
            }
            let m = w.unit(id)?.motion.fear.as_ref().unwrap();
            if m.points
                .as_ref()
                .map(|p| m.point >= p.len())
                .unwrap_or(true)
            {
                pick(w, g, rng, id)?;
            }
            let m = w.unit(id)?.motion.fear.as_ref().unwrap();
            let p = m
                .points
                .as_ref()
                .and_then(|p| p.get(m.point))
                .copied()
                .ok_or("Missing fear waypoint")?;
            let (left, reached, d) = step(w, id, p, distance)?;
            distance = left;
            if reached {
                fear_mut(w, id)?.point += 1;
            }
            moved |= d > 1e-9;
        }
    }
    let position = w.unit(id)?.position;
    fear_mut(w, id)?.previous = Some(position);
    w.unit_mut(id)?.moving = moved;
    Ok(())
}
pub fn end_fear(w: &mut World, id: Id) -> Result<(), String> {
    let e = w.unit_mut(id)?;
    e.motion.fear = None;
    if let Some(r) = e.motion.route.as_mut() {
        r.points = None;
    }
    Ok(())
}
/// Locomotion dispatch after the enemy attack/control arbitration. Fear outranks
/// attraction; attack stands hold attraction but keep route WAIT/teleport legs live.
pub fn update<H: Hooks>(
    w: &mut World,
    g: &mut Grid,
    r: &mut Runtime,
    rng: &mut Rng,
    h: &mut H,
    id: Id,
    dt: f64,
    standing: bool,
    unbalanced: bool,
) -> Result<(), String> {
    if !dt.is_finite() || dt < 0.0 {
        return Err("Invalid movement tick".into());
    }
    let (no_move, fear, attracted) = {
        let u = w.unit_mut(id)?;
        let hidden = u.hidden;
        let f = &u.stats.stats().map_err(str::to_string)?.flags;
        (
            f.contains("noMove"),
            f.contains("fear") && !hidden,
            f.contains("attract"),
        )
    };
    if no_move {
        w.unit_mut(id)?.moving = false;
        return Ok(());
    }
    if fear {
        if unbalanced {
            w.unit_mut(id)?.moving = false;
            return Ok(());
        }
        return move_feared(w, g, rng, id, dt);
    }
    if w.unit(id)?.motion.fear.is_some() {
        end_fear(w, id)?;
    }
    if attracted {
        if standing {
            w.unit_mut(id)?.moving = false;
            return Ok(());
        }
        return move_attracted(w, g, r, id, dt);
    }
    advance(w, g, h, id, dt, standing, rng)
}
pub fn move_attracted(
    w: &mut World,
    g: &mut Grid,
    r: &mut Runtime,
    id: Id,
    dt: f64,
) -> Result<(), String> {
    r.call(|r| {
        if !dt.is_finite() || dt < 0.0 {
            return Err("Invalid attraction tick".into());
        }
        r.adopt(w, id)?;
        let Some(b) = w
            .unit(id)?
            .stats
            .buffs()
            .iter()
            .find(|b| b.runtime.status.as_deref() == Some("attract") && b.runtime.attract.is_some())
            .cloned()
        else {
            w.unit_mut(id)?.moving = false;
            return Ok(());
        };
        let buff = b.runtime.id();
        let aim = b.runtime.attract.unwrap();
        let position = w.unit(id)?.position;
        let fly = w.unit(id)?.motion_fly;
        if b.runtime
            .attract_path
            .as_ref()
            .map(|p| p.version != g.version() || p.previous != Some(position))
            .unwrap_or(true)
        {
            let mut points = None;
            if !fly {
                if let Some(path) = g
                    .waypoints(raw_tile(position), aim, true, false)
                    .or_else(|| g.waypoints(raw_tile(position), aim, true, true))
                {
                    let mut pts = vec![];
                    for (i, p) in path.into_iter().enumerate() {
                        let p = centre(p);
                        if i == 0
                            && (position[1] - p[1]).abs() < 1e-6
                            && (position[0] - p[0]).abs() < 1e-6
                        {
                            continue;
                        }
                        pts.push(p);
                    }
                    points = Some(pts);
                }
            }
            let controller = Attract {
                points: points.unwrap_or_else(|| vec![centre(aim)]),
                point: 0,
                version: g.version(),
                previous: None,
            };
            r.edit_timer(w, id, buff, |b| b.runtime.attract_path = Some(controller))?;
        }
        let mut distance = w
            .unit_mut(id)?
            .stats
            .stats()
            .map_err(str::to_string)?
            .get(Stat::MoveSpeed)
            * 0.5
            * dt;
        let mut moved = false;
        while distance > 1e-9 {
            let a = r.get(w, id, buff)?.runtime.attract_path.as_ref().unwrap();
            let Some(p) = a.points.get(a.point).copied() else {
                break;
            };
            let (left, reached, _) = step(w, id, p, distance)?;
            distance = left;
            if reached {
                r.edit_timer(w, id, buff, |b| {
                    b.runtime.attract_path.as_mut().unwrap().point += 1
                })?;
            }
            moved = true;
        }
        let position = w.unit(id)?.position;
        r.edit_timer(w, id, buff, |b| {
            b.runtime.attract_path.as_mut().unwrap().previous = Some(position)
        })?;
        w.unit_mut(id)?.moving = moved;
        if moved {
            if let Some(r) = w.unit_mut(id)?.motion.route.as_mut() {
                r.points = None;
            }
        }
        Ok(())
    })
}
