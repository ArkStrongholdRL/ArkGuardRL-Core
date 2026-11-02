//! Development binding for the native foundations. No incomplete Gym environment is registered.
use arkguard_engine::{data::Catalog, deterministic::rng};
use pyo3::{exceptions::PyValueError, prelude::*};

#[pyfunction]
fn derive_seed(seed: u32, salt: &str) -> u32 {
    rng::derive_seed(seed, salt)
}

#[pyclass(frozen)]
struct NativeCatalog {
    inner: Catalog,
}

#[pymethods]
impl NativeCatalog {
    #[new]
    fn new(py: Python<'_>, data_root: String, max_bytes: usize) -> PyResult<Self> {
        py.detach(|| Catalog::load(std::path::Path::new(&data_root), max_bytes))
            .map(|inner| Self { inner })
            .map_err(PyValueError::new_err)
    }

    fn definitions(&self) -> Vec<(u32, String, String)> {
        self.inner
            .definitions()
            .iter()
            .map(|d| (d.index, d.table.clone(), d.id.clone()))
            .collect()
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeCatalog>()?;
    m.add_function(wrap_pyfunction!(derive_seed, m)?)?;
    Ok(())
}
