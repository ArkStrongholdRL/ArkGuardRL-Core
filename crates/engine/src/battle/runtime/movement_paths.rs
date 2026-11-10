//! Per-enemy movement prefixes ending before the first possible interaction.
use super::*;
use crate::battle::{body, grid::N, operators::RangeGeometry, stats::Stat};
use crate::deterministic::math::round_half_even;
use std::collections::{BTreeMap, VecDeque};

struct AllyGeometry {
    id: Id,
    position: [f64; 2],
    tile: [i32; 2],
    range: Vec<usize>,
    geometry: RangeGeometry<'static>,
}

struct Path {
    expected: [f64; 2],
    target: [f64; 2],
    stride: f64,
    version: u64,
    leg: usize,
    point: usize,
    positions: VecDeque<[f64; 2]>,
}

pub(super) struct MovementPaths {
    allies: Vec<AllyGeometry>,
    danger: [bool; N],
    distance: [f64; N],
    paths: BTreeMap<Id, Path>,
    pub(super) updates: u64,
    pub(super) fast_updates: u64,
}
impl Default for MovementPaths {
    fn default() -> Self {
        Self {
            allies: Vec::new(),
            danger: [false; N],
            distance: [f64::INFINITY; N],
            paths: BTreeMap::new(),
            updates: 0,
            fast_updates: 0,
        }
    }
}

fn key(position: [f64; 2]) -> Option<usize> {
    let [r, c] = position.map(round_half_even);
    (r.is_finite()
        && c.is_finite()
        && r >= 0.
        && r < super::super::grid::ROWS as f64
        && c >= 0.
        && c < COLS as f64)
        .then(|| r as usize * COLS + c as usize)
}

impl MovementPaths {
    pub(super) fn refresh(&mut self, h: &Host, w: &World) -> Result<(), String> {
        let roster = h.roster.as_ref().unwrap();
        let mut count = 0;
        let mut changed = false;
        for &id in roster.ids() {
            let unit = w.unit(id)?;
            if unit.stats.alive && unit.deployed && !unit.hidden {
                let geometry = roster.range_geometry(w, id)?;
                changed |= self.allies.get(count).is_none_or(|old| {
                    old.id != id
                        || old.position != unit.position
                        || old.tile != unit.deployment.tile
                        || old.geometry != geometry
                });
                count += 1;
            }
        }
        if !changed && count == self.allies.len() {
            return Ok(());
        }
        let mut allies = Vec::new();
        for &id in roster.ids() {
            let unit = w.unit(id)?;
            if unit.stats.alive && unit.deployed && !unit.hidden {
                let geometry = roster.range_geometry(w, id)?;
                allies.push(AllyGeometry {
                    id,
                    position: unit.position,
                    tile: unit.deployment.tile,
                    range: geometry.keys(),
                    geometry: geometry.owned(),
                });
            }
        }
        self.paths.clear();
        self.danger.fill(false);
        self.distance.fill(f64::INFINITY);
        for ally in &allies {
            // Include collider overlap and every tile searched by blocking::check.
            for &tile in &ally.range {
                if let Some(danger) = self.danger.get_mut(tile) {
                    *danger = true;
                }
            }
            if Grid::in_bounds(ally.tile) {
                let tile = ally.tile[0] as usize * COLS + ally.tile[1] as usize;
                let (r, c) = (tile / COLS, tile % COLS);
                for row in r.saturating_sub(1)..=(r + 1).min(super::super::grid::ROWS - 1) {
                    for col in c.saturating_sub(1)..=(c + 1).min(COLS - 1) {
                        self.danger[row * COLS + col] = true;
                    }
                }
            }
            for (tile, distance) in self.distance.iter_mut().enumerate() {
                // Distance to the entire cell is a lower bound for every centre in it.
                let dy = ((tile / COLS) as f64 - ally.position[0]).abs();
                let dx = ((tile % COLS) as f64 - ally.position[1]).abs();
                let dy = (dy - 0.5).max(0.);
                let dx = (dx - 0.5).max(0.);
                *distance = distance.min(dx * dx + dy * dy);
            }
        }
        self.allies = allies;
        Ok(())
    }

    pub(super) fn clear_at(&self, position: [f64; 2], radius_squared: f64) -> bool {
        let mut clear = true;
        body::enemy_collider_keys(position, |key| clear &= !self.danger[key]);
        key(position).is_some_and(|key| clear && self.distance[key] > radius_squared)
    }

    pub(super) fn retain_live(&mut self, w: &World) {
        self.paths
            .retain(|&id, _| w.unit(id).is_ok_and(|u| u.stats.alive && !u.removed));
    }

    pub(super) fn step(
        &mut self,
        w: &mut World,
        h: &mut Host,
        grid: &Grid,
        id: Id,
        map_dirty: &mut bool,
    ) -> Result<bool, String> {
        self.updates = self.updates.saturating_add(1);
        let time = w.time;
        let u = w.unit_mut(id)?;
        let Some(cell) = key(u.position) else {
            return Ok(false);
        };
        // A stale exclusion can only choose the full update. Every accepted step
        // refreshes dirty geometry below before using a cached prefix.
        if self.danger[cell] {
            return Ok(false);
        }
        let base_radius = u.stats.base_number("rangeRadius") + 0.25;
        if self.distance[cell] <= base_radius * base_radius + 1e-9 {
            return Ok(false);
        }
        if !u.stats.alive
            || !u.deployed
            || u.hidden
            || u.removed
            || u.pool.is_some()
            || u.hit_area.is_some()
            || u.blocked_by().is_some()
            || u.enemy.carrier.is_some()
            || u.enemy.swing
            || u.enemy.targets().is_some()
            || u.enemy.pause_until > time
            || u.motion.attack_stand_until > time
            || u.motion.unbalance_until > time
            || u.motion.fear.is_some()
        {
            self.paths.remove(&id);
            return Ok(false);
        }
        let stats = u.stats.stats().map_err(str::to_string)?;
        let stride = stats.get(Stat::MoveSpeed) * 0.5 * DT;
        if !stats.flags.is_empty() || !stride.is_finite() || stride <= 1e-9 {
            self.paths.remove(&id);
            return Ok(false);
        }
        let Some(route) = u.motion.route.as_ref() else {
            return Ok(false);
        };
        let Some(target) = route
            .points
            .as_ref()
            .and_then(|points| points.get(route.point))
            .copied()
        else {
            return Ok(false);
        };
        let rect = grid.rect();
        if route.version != grid.version()
            || route.wait_left.is_some()
            || route.patrol
            || !matches!(route.legs().get(route.leg), Some(motion::Leg::Move { .. }))
            || !u
                .position
                .iter()
                .chain(target.iter())
                .all(|n| n.is_finite())
            || u.position[0] < rect.r0 as f64
            || u.position[0] > rect.r1 as f64
            || u.position[1] < rect.c0 as f64
            || u.position[1] > rect.c1 as f64
            || target[0] < rect.r0 as f64
            || target[0] > rect.r1 as f64
            || target[1] < rect.c0 as f64
            || target[1] > rect.c1 as f64
        {
            return Ok(false);
        }
        let (position, leg, point) = (u.position, route.leg, route.point);
        let profile = enemy::Hooks::profile(h, id, w)?;
        if matches!(profile.mode, enemy::Mode::Heal) {
            return Ok(false);
        }
        let radius =
            if profile.melee || profile.no_attack || matches!(profile.mode, enemy::Mode::None) {
                0.
            } else {
                w.unit(id)?.stats.base_number("rangeRadius") + 0.25
            };
        if !radius.is_finite() {
            return Ok(false);
        }
        if *map_dirty {
            self.refresh(h, w)?;
            *map_dirty = false;
        }
        let radius_squared = radius * radius + 1e-9;
        if !self.clear_at(position, radius_squared) {
            return Ok(false);
        }
        let valid = self.paths.get(&id).is_some_and(|p| {
            p.expected.map(f64::to_bits) == position.map(f64::to_bits)
                && p.target.map(f64::to_bits) == target.map(f64::to_bits)
                && p.stride.to_bits() == stride.to_bits()
                && p.version == grid.version()
                && p.leg == leg
                && p.point == point
                && !p.positions.is_empty()
        });
        if !valid {
            let mut path = Path {
                expected: position,
                target,
                stride,
                version: grid.version(),
                leg,
                point,
                positions: VecDeque::new(),
            };
            let mut next = position;
            // Bound storage by 32 positions per live enemy, independent of episode length.
            for _ in 0..32 {
                let (position, _, reached, _) = motion::step_position(next, target, stride);
                if reached || !self.clear_at(position, radius_squared) {
                    break;
                }
                path.positions.push_back(position);
                next = position;
            }
            self.paths.insert(id, path);
        }
        let path = self.paths.get_mut(&id).unwrap();
        let Some(next) = path.positions.pop_front() else {
            return Ok(false);
        };
        let unit = w.unit_mut(id)?;
        unit.position = next;
        unit.moving = true;
        unit.cadence.tick(DT, true);
        path.expected = next;
        self.fast_updates = self.fast_updates.saturating_add(1);
        Ok(true)
    }
}

impl Field {
    pub fn movement_path_steps(&self) -> [u64; 2] {
        [
            self.core.movement_paths.fast_updates,
            self.core.movement_paths.updates,
        ]
    }
}
