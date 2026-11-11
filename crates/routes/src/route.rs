// Candidate geometry, swaps, support queries and dense output in one Rust call.
use crate::{calculate, Change, Unit, CELLS, MASK, MAX_CANDIDATES};
use numpy::{ndarray::Array2, IntoPyArray, PyArray2};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::HashMap;

// Definition index, board cell (-1 for reserve), facing RIGHT/UP/DOWN/LEFT.
type Piece = (usize, i8, u8);
// Piece index (-1 for non-chess), target cell, facing, move, removal, occupied hand.
type Action = (i32, i8, u8, bool, bool, bool);
// Block, DPS, max HP, DEF, RES, healer, anti-air.
type Definition = (f64, f64, f64, f64, f64, bool, bool);

#[pyclass(frozen)]
pub struct RouteEncoder {
    definitions: Vec<Definition>,
    covers: Vec<[u64; CELLS * 4]>,
}

#[pymethods]
impl RouteEncoder {
    #[new]
    fn new(definitions: Vec<Definition>, ranges: Vec<Vec<(i32, i32)>>) -> PyResult<Self> {
        if definitions.len() != ranges.len()
            || definitions.len() > 16384
            || definitions
                .iter()
                .any(|d| ![d.0, d.1, d.2, d.3, d.4].iter().all(|x| x.is_finite()))
            || ranges.iter().any(|r| {
                r.len() > 256
                    || r.iter()
                        .any(|&(r, c)| !(-128..=128).contains(&r) || !(-128..=128).contains(&c))
            })
        {
            return Err(PyValueError::new_err("Invalid route definitions"));
        }
        let covers = ranges
            .iter()
            .map(|range| {
                let mut covers = [0; CELLS * 4];
                for at in 0..CELLS {
                    for dir in 0..4 {
                        for &(dr, dc) in range {
                            let (rr, cc) = match dir {
                                0 => (dr, dc),
                                1 => (dc, -dr),
                                2 => (-dc, dr),
                                _ => (-dr, -dc),
                            };
                            let (r, c) = (at as i32 / 9 + rr, at as i32 % 9 + cc);
                            if (0..4).contains(&r) && (0..9).contains(&c) {
                                covers[at * 4 + dir] |= 1 << (r * 9 + c);
                            }
                        }
                    }
                }
                covers
            })
            .collect();
        Ok(Self {
            definitions,
            covers,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn encode<'py>(
        &self,
        py: Python<'py>,
        pieces: Vec<Piece>,
        base_count: usize,
        actions: Vec<Action>,
        ground: u64,
        air: u64,
        ranged: u64,
        required: [u64; CELLS],
        progress: [[f64; CELLS]; 2],
        gc: [f64; CELLS],
        ac: [f64; CELLS],
        hc: [i64; CELLS],
        before: [f64; 6],
    ) -> PyResult<Bound<'py, PyArray2<f32>>> {
        if pieces.len() > 128
            || base_count > CELLS
            || base_count > pieces.len()
            || actions.len() > MAX_CANDIDATES
            || (ground | air | ranged) & !MASK != 0
            || required.iter().any(|m| m & !MASK != 0)
            || pieces.iter().enumerate().any(|(i, p)| {
                p.0 >= self.definitions.len()
                    || p.2 > 3
                    || if i < base_count {
                        !(0..CELLS as i8).contains(&p.1)
                    } else {
                        p.1 != -1
                    }
            })
            || actions.iter().any(|a| {
                a.0 < -1 || a.0 >= pieces.len() as i32 || a.1 < -1 || a.1 >= CELLS as i8 || a.2 > 3
            })
            || progress
                .iter()
                .flatten()
                .chain(gc.iter())
                .chain(ac.iter())
                .chain(before.iter())
                .any(|v| !v.is_finite())
        {
            return Err(PyValueError::new_err("Invalid route candidate inputs"));
        }
        let mut occupied = [None; CELLS];
        for (i, p) in pieces.iter().take(base_count).enumerate() {
            if occupied[p.1 as usize].replace(i).is_some() {
                return Err(PyValueError::new_err("Duplicate board cell"));
            }
        }
        let output = py.detach(|| {
            self.calculate(
                &pieces, base_count, &actions, ground, air, ranged, &required, &progress, &gc, &ac,
                &hc, &before, &occupied,
            )
        });
        if output.iter().any(|v| !v.is_finite()) {
            return Err(PyValueError::new_err("Nonfinite route output"));
        }
        Ok(Array2::from_shape_vec((actions.len(), 32), output)
            .map_err(|_| PyValueError::new_err("Route output shape"))?
            .into_pyarray(py))
    }
}

impl RouteEncoder {
    fn unit(&self, p: Piece, ground: u64, required: &[u64; CELLS]) -> Unit {
        let at = p.1 as usize;
        let d = self.definitions[p.0];
        let cover = self.covers[p.0][at * 4 + p.2 as usize];
        let mut support = 0;
        for (i, &needed) in required.iter().enumerate() {
            if cover & needed == needed {
                support |= 1 << i;
            }
        }
        (
            cover,
            support,
            at,
            d.1,
            if d.6 { d.1 } else { 0. },
            d.5 as i64,
            d.0 > 0. && ground & (1 << at) != 0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn calculate(
        &self,
        pieces: &[Piece],
        base_count: usize,
        actions: &[Action],
        ground: u64,
        air: u64,
        ranged: u64,
        required: &[u64; CELLS],
        progress: &[[f64; CELLS]; 2],
        gc: &[f64; CELLS],
        ac: &[f64; CELLS],
        hc: &[i64; CELLS],
        before: &[f64; 6],
        occupied: &[Option<usize>; CELLS],
    ) -> Vec<f32> {
        let mut units: Vec<Unit> = pieces
            .iter()
            .take(base_count)
            .map(|&p| self.unit(p, ground, required))
            .collect();
        let mut variants = HashMap::new();
        for (i, &p) in pieces.iter().take(base_count).enumerate() {
            variants.insert(p, i);
        }
        let mut providers = [0_i64; CELLS];
        let mut blockers = [0_i64; CELLS];
        for u in &units {
            for (i, total) in providers.iter_mut().enumerate() {
                *total += (u.3 > 0. && u.1 & (1 << i) != 0) as i64;
            }
            blockers[u.2] += u.6 as i64;
        }
        let mut changes: Vec<Change> = Vec::new();
        let mut pending = Vec::new();
        let mut rows = vec![[0_f64; 32]; actions.len()];
        for (index, &(source, target, dir, moving, removal, hand_occupied)) in
            actions.iter().enumerate()
        {
            if source < 0 || !(moving || removal) {
                continue;
            }
            let source = source as usize;
            let old = (source < base_count).then_some(source);
            let mut removed: Vec<usize> = old.into_iter().collect();
            let mut added = Vec::new();
            let mut actor = old;
            let mut make_unit = |p: Piece| -> usize {
                *variants.entry(p).or_insert_with(|| {
                    let i = units.len();
                    units.push(self.unit(p, ground, required));
                    i
                })
            };
            if target >= 0 {
                if let Some(other) = occupied[target as usize].filter(|&i| i != source) {
                    removed.push(other);
                    if let Some(src) = old {
                        added.push(make_unit((pieces[other].0, pieces[src].1, pieces[other].2)));
                    }
                }
                actor = Some(make_unit((pieces[source].0, target, dir)));
            }
            let Some(actor) = actor else {
                continue;
            };
            let u = units[actor];
            let at = u.2;
            let d = self.definitions[pieces[source].0];
            let mut support = providers[at];
            let mut healing = hc[at];
            let mut supported_blocks: i64 = blockers
                .iter()
                .enumerate()
                .filter(|(i, _)| u.1 & (1 << i) != 0)
                .map(|(_, n)| *n)
                .sum();
            // Query the other units only: the actor is appended after this query.
            for (sign, indices) in [(-1_i64, &removed), (1, &added)] {
                for &i in indices {
                    let v = units[i];
                    support += sign * (v.3 > 0. && v.1 & (1 << at) != 0) as i64;
                    healing += sign * v.5 * (v.0 & (1 << at) != 0) as i64;
                    supported_blocks += sign * (v.6 && u.1 & (1 << v.2) != 0) as i64;
                }
            }
            if target >= 0 {
                added.push(actor);
            }
            removed.sort_unstable();
            changes.push((removed, added));
            pending.push(index);
            let src = old.map(|i| pieces[i].1 as usize);
            rows[index] = [
                (!(target < 0 && moving && hand_occupied)) as u8 as f64,
                d.0 / 8.,
                d.5 as u8 as f64,
                d.6 as u8 as f64,
                d.1 / 2000.,
                d.2 / 10000.,
                d.3 / 1000.,
                d.4 / 100.,
                (ground & (1 << at) != 0) as u8 as f64,
                (air & (1 << at) != 0) as u8 as f64,
                progress[0][at],
                progress[1][at],
                (u.0 & ground).count_ones() as f64 / ground.count_ones().max(1) as f64,
                (u.0 & air).count_ones() as f64 / air.count_ones().max(1) as f64,
                (u.1 & (1 << at) != 0) as u8 as f64,
                (support > 0) as u8 as f64,
                (healing as f64 / 2.).min(1.),
                supported_blocks as f64 / 8.,
                gc[at] / 2000.,
                ac[at] / 2000.,
                0.,
                0.,
                0.,
                0.,
                0.,
                0.,
                src.is_some_and(|i| ground & (1 << i) != 0) as u8 as f64,
                src.is_some_and(|i| air & (1 << i) != 0) as u8 as f64,
                (ranged & (1 << at) != 0) as u8 as f64,
                occupied[at].is_some() as u8 as f64,
                moving as u8 as f64,
                removal as u8 as f64,
            ];
        }
        let summaries = calculate(&units, base_count, &changes, ground, air);
        for (j, &row) in pending.iter().enumerate() {
            for k in 0..6 {
                rows[row][20 + k] = summaries[j * 6 + k] - before[k];
            }
        }
        rows.into_iter().flatten().map(|v| v as f32).collect()
    }
}
