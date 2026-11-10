//! Experimental Rust-owned parallel slots. The current engine and numeric
//! encoder are dependencies of this isolated experiment crate.
use arkguard_engine::data::Catalog;
use pyo3::{exceptions::PyValueError, prelude::*};
use std::sync::Arc;
mod batch;
#[path = "../../../crates/python/src/policy.rs"]
mod policy;

#[pyclass(frozen)]
pub(crate) struct NativeCatalog {
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
            serde_json::Value::Object(out).to_string()
        })
    }
}
#[pymodule]
fn _native_slots(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeCatalog>()?;
    m.add_class::<batch::NativeBatch>()?;
    Ok(())
}