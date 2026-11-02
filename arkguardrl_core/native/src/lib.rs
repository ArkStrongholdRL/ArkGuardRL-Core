// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from route_arrays.py: ordered, compensated route summaries.
use numpy::{ndarray::Array2, IntoPyArray, PyArray2};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::collections::HashMap;
mod observation;
mod prep;
mod route;

const CELLS: usize = 36;
const MASK: u64 = (1 << CELLS) - 1;
const MAX_CANDIDATES: usize = 16384;
// Coverage, support, cell, ground DPS, air DPS, healer, blocker.
type Unit = (u64, u64, usize, f64, f64, i64, bool);
type Change = (Vec<usize>, Vec<usize>);

#[derive(Clone, Copy, Default)]
struct Sum {
    total: f64,
    error: f64,
}

impl Sum {
    // No fast-math/reassociation: preserve the Python/NumPy operation order.
    fn add(&mut self, value: f64) {
        let added = self.total + value;
        self.error += if self.total.abs() >= value.abs() {
            (self.total - added) + value
        } else {
            (value - added) + self.total
        };
        self.total = added;
    }

    fn value(self) -> f64 {
        self.total + self.error
    }
}

#[derive(Clone)]
struct Board {
    ground: [Sum; CELLS],
    air: [Sum; CELLS],
    healing: [i64; CELLS],
    providers: [i16; CELLS],
    retained: Vec<usize>,
}

impl Board {
    fn new() -> Self {
        Self {
            ground: [Sum::default(); CELLS],
            air: [Sum::default(); CELLS],
            healing: [0; CELLS],
            providers: [0; CELLS],
            retained: Vec::new(),
        }
    }

    fn add(&mut self, u: &Unit) {
        for cell in 0..CELLS {
            let covered = ((u.0 >> cell) & 1) as i64;
            // Include zero contributions, exactly as the array reference does.
            self.ground[cell].add(covered as f64 * u.3);
            self.air[cell].add(covered as f64 * u.4);
            self.healing[cell] += covered * u.5;
            self.providers[cell] += (((u.1 >> cell) & 1) != 0 && u.3 > 0.) as i16;
        }
    }
}

fn calculate(
    units: &[Unit],
    base_count: usize,
    changes: &[Change],
    ground: u64,
    air: u64,
) -> Vec<f64> {
    let mut prefixes: HashMap<Vec<usize>, Board> = HashMap::new();
    let mut output = Vec::with_capacity(changes.len() * 6);
    for (removed, appended) in changes {
        let prefix = prefixes.entry(removed.clone()).or_insert_with(|| {
            let mut b = Board::new();
            for (index, u) in units.iter().enumerate().take(base_count) {
                if !removed.contains(&index) {
                    b.add(u);
                    b.retained.push(index);
                }
            }
            b
        });
        let mut board = prefix.clone();
        for &index in appended {
            board.add(&units[index]);
        }
        let mut blocks = 0;
        let mut supported = 0;
        let mut damage = Sum::default();
        let mut healing = 0_i64;
        for &index in prefix.retained.iter().chain(appended.iter()) {
            let u = &units[index];
            blocks += u.6 as usize;
            let own = (((u.1 >> u.2) & 1) != 0 && u.3 > 0.) as i16;
            supported += (u.6 && board.providers[u.2] - own > 0) as usize;
            damage.add(board.ground[u.2].value() * u.6 as u8 as f64);
            healing += board.healing[u.2] * u.6 as i64;
        }
        let mut gh = 0;
        let mut ah = 0;
        for cell in 0..CELLS {
            gh += ((ground >> cell) & 1 != 0 && board.ground[cell].value() > 0.) as usize;
            ah += ((air >> cell) & 1 != 0 && board.air[cell].value() > 0.) as usize;
        }
        output.extend_from_slice(&[
            gh as f64 / ground.count_ones().max(1) as f64,
            ah as f64 / air.count_ones().max(1) as f64,
            supported as f64 / blocks.max(1) as f64,
            blocks as f64 / 8.,
            damage.value() / 2000.,
            healing as f64 / 8.,
        ]);
    }
    output
}

#[pyfunction]
fn summaries<'py>(
    py: Python<'py>,
    units: Vec<Unit>,
    base_count: usize,
    mut changes: Vec<Change>,
    ground: u64,
    air: u64,
) -> PyResult<Bound<'py, PyArray2<f64>>> {
    if base_count > 128
        || base_count > units.len()
        || changes.len() > MAX_CANDIDATES
        || units.len() > 128 + 2 * MAX_CANDIDATES
        || (ground | air) & !MASK != 0
    {
        return Err(PyValueError::new_err("Route kernel domain bound exceeded"));
    }
    for u in &units {
        if (u.0 | u.1) & !MASK != 0
            || u.2 >= CELLS
            || !u.3.is_finite()
            || !u.4.is_finite()
            || !(0..=1).contains(&u.5)
        {
            return Err(PyValueError::new_err("Invalid numeric route unit"));
        }
    }
    for (removed, appended) in &mut changes {
        removed.sort_unstable();
        if removed.len() > 2
            || appended.len() > 2
            || removed.windows(2).any(|x| x[0] == x[1])
            || removed.iter().any(|&i| i >= base_count)
            || appended.iter().any(|&i| i >= units.len())
            || appended.len() == 2 && appended[0] == appended[1]
            || appended
                .iter()
                .any(|&i| i < base_count && !removed.contains(&i))
            || base_count - removed.len() + appended.len() > 128
        {
            return Err(PyValueError::new_err("Invalid route board delta"));
        }
    }
    let count = changes.len();
    // All inputs now owned by Rust. No Python callbacks or borrowed Python buffers.
    let output = py.detach(|| calculate(&units, base_count, &changes, ground, air));
    if output.iter().any(|v| !v.is_finite()) {
        return Err(PyValueError::new_err("Nonfinite route summary"));
    }
    Ok(Array2::from_shape_vec((count, 6), output)
        .map_err(|_| PyValueError::new_err("Route output shape mismatch"))?
        .into_pyarray(py))
}

#[pymodule]
fn _rust_routes(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("ABI", "arkguardrl-core/rust-kernels/v4")?;
    m.add_function(wrap_pyfunction!(summaries, m)?)?;
    m.add_function(wrap_pyfunction!(search_relations, m)?)?;
    m.add_function(wrap_pyfunction!(prep::prep_piece_actions, m)?)?;
    m.add_class::<observation::ObservationEncoder>()?;
    m.add_class::<route::RouteEncoder>()?;
    Ok(())
}

// Action type, object, purchased object, equip target, deployment from reserve,
// asset present, ordinary pair present, requested reward slot selected.
type Candidate = (u8, u32, u32, u32, bool, bool, bool, bool);

#[pyfunction]
fn search_relations<'py>(
    py: Python<'py>,
    candidates: Vec<Candidate>,
    material: u32,
    acquired: u32,
    shared: [f32; 10],
) -> PyResult<Bound<'py, PyArray2<f32>>> {
    if candidates.len() > MAX_CANDIDATES
        || candidates.iter().any(|c| c.0 > 7)
        || shared
            .iter()
            .any(|&x| !x.is_finite() || !(0. ..=1.).contains(&x))
    {
        return Err(PyValueError::new_err("Invalid search relation facts"));
    }
    let count = candidates.len();
    let output = py.detach(|| {
        let mut output = Vec::with_capacity(count * 20);
        for (t, obj, purchased, equip, deploy, asset, pair, requested) in candidates {
            let flag = |x: bool| x as u8 as f32;
            output.extend_from_slice(&[
                shared[0],
                flag(t == 1 && asset),
                flag(t == 1 && asset && pair),
                flag(t == 1 && purchased == material && material > 0),
                flag(t == 2 && shared[1] > 0.),
                flag(t == 3 && requested),
                flag(t == 3 && !requested),
                flag(t == 4 && obj == acquired && acquired > 0 && deploy),
                flag(t == 5 && obj == acquired && acquired > 0),
                flag(t == 6 && equip == acquired && acquired > 0),
                flag(t == 4 && obj == material && material > 0 && deploy),
                flag(t == 5 && obj == material && material > 0),
                flag(t == 7 && shared[2] > 0.),
                flag(t == 7 && acquired > 0 && shared[3] > 0.),
                shared[4],
                shared[5],
                shared[6],
                shared[7],
                shared[8],
                shared[9],
            ]);
        }
        output
    });
    Ok(Array2::from_shape_vec((count, 20), output)
        .map_err(|_| PyValueError::new_err("Search relation output shape mismatch"))?
        .into_pyarray(py))
}
