// Dynamic observation layout v7. Static catalog ranges live once per encoder.
use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyFloat, PyTuple},
};

const ENTITIES: usize = 128;
const ENTITY_WIDTH: usize = 28;
const BONDS: usize = 23;
const ENEMIES: usize = 64;
const CELLS: usize = 36;
// Definition, owner/equipment references, area, direction, dynamic numbers.
type Entity = (usize, [usize; 3], usize, i8, [f64; 11]);
// Definition, row, column, direction, entity reference.
type BoardPiece = (usize, i32, i32, u8, usize);
// Definition, catalog reference, count/time/fly/elite/boss, gate.
type Enemy = (usize, usize, [f64; 5], i8);

#[pyclass(frozen)]
pub struct ObservationEncoder {
    ranges: Vec<Vec<(i32, i32)>>,
    air: Vec<bool>,
}

fn cell(row: i32, col: i32) -> Option<usize> {
    if (9..13).contains(&row) && (2..11).contains(&col) {
        Some(((row - 9) * 9 + col - 2) as usize)
    } else {
        None
    }
}

#[pymethods]
impl ObservationEncoder {
    #[new]
    fn new(ranges: Vec<Vec<(i32, i32)>>, air: Vec<bool>) -> PyResult<Self> {
        if ranges.len() != air.len()
            || ranges.iter().any(|r| {
                r.len() > 24
                    || r.iter()
                        .any(|&(a, b)| !(-128..=128).contains(&a) || !(-128..=128).contains(&b))
            })
        {
            return Err(PyValueError::new_err("Invalid static observation ranges"));
        }
        Ok(Self { ranges, air })
    }

    fn encode<'py>(
        &self,
        py: Python<'py>,
        global: Vec<f64>,
        entities: Vec<Entity>,
        bonds: Vec<[f64; 5]>,
        tiles: Vec<(i32, i32, bool)>,
        board: Vec<BoardPiece>,
        enemies: Vec<Enemy>,
        control: Vec<f64>,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let valid_def = |d: usize| d > 0 && d <= self.ranges.len();
        if global.len() != 42
            || control.len() != 95
            || entities.len() > ENTITIES
            || bonds.len() != BONDS
            || tiles.len() > CELLS
            || board.len() > CELLS
            || enemies.len() > ENEMIES
            || entities.iter().any(|e| {
                !valid_def(e.0)
                    || e.1.iter().any(|&r| r > entities.len())
                    || e.2 >= 8
                    || !(-1..4).contains(&e.3)
                    || e.4.iter().any(|v| !v.is_finite())
            })
            || board.iter().any(|b| {
                !valid_def(b.0)
                    || cell(b.1, b.2).is_none()
                    || b.3 >= 4
                    || b.4 == 0
                    || b.4 > entities.len()
            })
            || tiles.iter().any(|t| cell(t.0, t.1).is_none())
            || enemies.iter().any(|e| {
                !valid_def(e.0) || !(-1..4).contains(&e.3) || e.2.iter().any(|v| !v.is_finite())
            })
            || global
                .iter()
                .chain(control.iter())
                .chain(bonds.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err(PyValueError::new_err(
                "Invalid dynamic observation shape, reference or number",
            ));
        }
        let output = py.detach(|| {
            let mut output = vec![0.0; 4784];
            output[..42].copy_from_slice(&global);
            for (index, &(definition, refs, area, direction, numbers)) in
                entities.iter().enumerate()
            {
                let row = &mut output[42 + index * ENTITY_WIDTH..42 + (index + 1) * ENTITY_WIDTH];
                row[..5].copy_from_slice(&[
                    definition as f64,
                    1.0,
                    refs[0] as f64,
                    refs[1] as f64,
                    refs[2] as f64,
                ]);
                row[5 + area] = 1.0;
                let scales = [30., 20., 21., 10., 10., 10., 10., 10., 1., 10., 1.];
                for i in 0..11 {
                    row[13 + i] = if i == 8 || i == 10 {
                        numbers[i]
                    } else {
                        (numbers[i] / scales[i]).tanh()
                    };
                }
                if direction >= 0 {
                    row[24 + direction as usize] = 1.0;
                }
            }
            let bond_start = 42 + ENTITIES * ENTITY_WIDTH;
            for (index, values) in bonds.iter().enumerate() {
                output[bond_start + index * 5..bond_start + (index + 1) * 5]
                    .copy_from_slice(values);
            }
            let grid_start = bond_start + BONDS * 5;
            for &(row, col, ranged) in &tiles {
                let at = grid_start + cell(row, col).unwrap() * 5;
                output[at] = 1.0;
                output[at + 1] = ranged as u8 as f64;
            }
            let mut coverage = [0_u32; CELLS];
            let mut air_coverage = [0_u32; CELLS];
            for &(definition, row, col, direction, reference) in &board {
                output[grid_start + cell(row, col).unwrap() * 5 + 2] = reference as f64;
                for &(dr, dc) in &self.ranges[definition - 1] {
                    let (r, c) = match direction {
                        0 => (dr, dc),
                        1 => (dc, -dr),
                        2 => (-dc, dr),
                        _ => (-dr, -dc),
                    };
                    if let Some(at) = cell(row + r, col + c) {
                        coverage[at] += 1;
                        air_coverage[at] += self.air[definition - 1] as u32;
                    }
                }
            }
            for at in 0..CELLS {
                output[grid_start + at * 5 + 3] = (coverage[at] as f64 / 6.).tanh();
                output[grid_start + at * 5 + 4] = (air_coverage[at] as f64 / 6.).tanh();
            }
            let enemy_start = grid_start + CELLS * 5;
            for (index, &(definition, catalog, numbers, gate)) in enemies.iter().enumerate() {
                let row = &mut output[enemy_start + index * 12..enemy_start + (index + 1) * 12];
                row[..8].copy_from_slice(&[
                    definition as f64,
                    1.0,
                    catalog as f64,
                    (numbers[0] / 10.).tanh(),
                    (numbers[1] / 120.).tanh(),
                    numbers[2],
                    numbers[3],
                    numbers[4],
                ]);
                if gate >= 0 {
                    row[8 + gate as usize] = 1.0;
                }
            }
            output[enemy_start + ENEMIES * 12..].copy_from_slice(&control);
            output
        });
        // Share immutable padding zeros instead of allocating thousands of floats.
        let zero = PyFloat::new(py, 0.0);
        PyTuple::new(
            py,
            output.into_iter().map(|v| {
                if v.to_bits() == 0 {
                    zero.clone()
                } else {
                    PyFloat::new(py, v)
                }
            }),
        )
    }
}
