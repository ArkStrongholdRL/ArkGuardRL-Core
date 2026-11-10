//! Development bindings for native data and persistent execution. No Gym environment is registered.
use arkguard_engine::{data::Catalog, deterministic::rng};
use pyo3::{exceptions::PyValueError, prelude::*};
use std::sync::Arc;
mod policy;
mod session;
mod ledger;
mod candidates;

#[pyfunction]
fn derive_seed(seed: u32, salt: &str) -> u32 {
    rng::derive_seed(seed, salt)
}

#[pyclass(frozen)]
struct NativeCatalog {
    inner: Arc<Catalog>,
}

#[pymethods]
impl NativeCatalog {
    #[new]
    fn new(py: Python<'_>, data_root: String, max_bytes: usize) -> PyResult<Self> {
        py.detach(|| Catalog::load(std::path::Path::new(&data_root), max_bytes))
            .map(|inner| Self {
                inner: Arc::new(inner),
            })
            .map_err(PyValueError::new_err)
    }

    fn with_diy(&self, py: Python<'_>, picks_json: &str) -> PyResult<Self> {
        if picks_json.len() > 4096 {
            return Err(PyValueError::new_err("DIY configuration exceeds byte limit"));
        }
        py.detach(|| {
            let picks = serde_json::from_str(picks_json).map_err(|e| format!("{e}"))?;
            self.inner.with_diy(&picks).map(|inner| Self {
                inner: Arc::new(inner),
            })
        })
        .map_err(PyValueError::new_err)
    }

    fn public_tables(&self, py: Python<'_>) -> String {
        py.detach(|| {
            let mut out = serde_json::Map::new();
            for name in [
                "bands",
                "chess",
                "items",
                "tokens",
                "config",
                "bonds",
                "garrisons",
                "enemies",
                "choices",
                "effects",
            ] {
                out.insert(name.into(), self.inner.table(name).unwrap().clone());
            }
            if let Some(picks) = self.inner.table("diy") { out.insert("diy".into(), picks.clone()); }
            serde_json::Value::Object(out).to_string()
        })
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
    m.add_class::<session::NativeSolo>()?;
    m.add_function(wrap_pyfunction!(derive_seed, m)?)?;
    Ok(())
}
