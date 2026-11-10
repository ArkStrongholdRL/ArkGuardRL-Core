// SPDX-License-Identifier: GPL-3.0-or-later
use crate::NativeCatalog;
use arkguard_engine::{data::Catalog, solo::session::Session};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};
use std::sync::Arc;

/// Persistent execution API; JSON diagnostics are deliberately separate from receipts.
/// No policy observation or Gym registration is implied by this development binding.
#[pyclass]
pub struct NativeSolo {
    data: Arc<Catalog>,
    core: String,
    banned: Vec<String>,
    setup_revision: u32,
    session: Option<Session>,
    public: Option<Arc<serde_json::Value>>,
    policy: Option<crate::policy::PolicyEncoder>,
    ledger: Option<crate::ledger::DecisionLedger>,
}
impl NativeSolo {
    fn live(&mut self) -> PyResult<&mut Session> {
        self.session
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("Reset required"))
    }
}
#[pymethods]
impl NativeSolo {
    #[new]
    #[pyo3(signature = (catalog, core, banned=None, diy_json=None, setup_revision=0))]
    fn new(
        catalog: &NativeCatalog,
        core: String,
        banned: Option<Vec<String>>,
        diy_json: Option<&str>,
        setup_revision: u32,
    ) -> PyResult<Self> {
        if !matches!(
            core.as_str(),
            "band_amiya" | "band_sarkazb" | "band_chen" | "band_sciurus"
        ) {
            return Err(PyValueError::new_err("Unsupported core"));
        }
        let banned = banned.unwrap_or_default();
        if banned.len() > 4096 || banned.iter().any(|s| s.len() > 256) {
            return Err(PyValueError::new_err("Manual bans exceed capacity"));
        }
        let data = if let Some(picks) = diy_json {
            if picks.len() > 4096 {
                return Err(PyValueError::new_err(
                    "DIY configuration exceeds byte limit",
                ));
            }
            let picks =
                serde_json::from_str(picks).map_err(|e| PyValueError::new_err(format!("{e}")))?;
            Arc::new(
                catalog
                    .inner
                    .with_diy(&picks)
                    .map_err(PyValueError::new_err)?,
            )
        } else {
            Arc::clone(&catalog.inner)
        };
        Ok(Self {
            data,
            core,
            banned,
            setup_revision,
            session: None,
            public: None,
            policy: None,
            ledger: None,
        })
    }
    /// Replace the episode, retaining immutable definitions. A failed reset
    /// leaves no old episode available to accidentally continue.
    fn reset(&mut self, py: Python<'_>, seed: u32) -> PyResult<String> {
        self.session = None;
        self.public = None;
        self.ledger = None;
        let session = py
            .detach(|| {
                Session::new_at_revision(
                    Arc::clone(&self.data),
                    seed,
                    &self.core,
                    &self.banned,
                    self.setup_revision,
                )
            })
            .map_err(PyRuntimeError::new_err)?;
        let status = session
            .status()
            .map_err(PyRuntimeError::new_err)?
            .to_string();
        self.session = Some(session);
        Ok(status)
    }
    fn step(&mut self, py: Python<'_>, action_json: &str, tick_budget: usize) -> PyResult<String> {
        if action_json.len() > 4096 {
            return Err(PyValueError::new_err("Action exceeds byte limit"));
        }
        let action = serde_json::from_str(action_json)
            .map_err(|e: serde_json::Error| PyValueError::new_err(e.to_string()))?;
        self.public = None;
        self.ledger = None;
        let live = self.live()?;
        py.detach(|| live.step(&action, tick_budget))
            .map(|v| v.to_string())
            .map_err(PyRuntimeError::new_err)
    }
    fn advance(&mut self, py: Python<'_>, tick_budget: usize) -> PyResult<String> {
        self.public = None;
        self.ledger = None;
        let live = self.live()?;
        py.detach(|| live.advance(tick_budget))
            .map(|v| v.to_string())
            .map_err(PyRuntimeError::new_err)
    }
    fn observation(&mut self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| {
            if self.public.is_none() {
                self.public = Some(Arc::new(
                    self.live()?
                        .observation()
                        .map_err(PyRuntimeError::new_err)?,
                ));
            }
            Ok(self.public.as_ref().unwrap().to_string())
        })
    }
    fn reset_decision(&mut self, py: Python<'_>, seed: u32, stream_id: String) -> PyResult<String> {
        self.reset(py, seed)?;
        let result = py.detach(|| {
            let state = Arc::new(
                self.live()?
                    .observation()
                    .map_err(PyRuntimeError::new_err)?,
            );
            let mut ledger = crate::ledger::DecisionLedger::new(state, stream_id)
                .map_err(PyValueError::new_err)?;
            let result = ledger
                .snapshot(None, None, 0)
                .map_err(PyValueError::new_err)?;
            self.public = Some(ledger.state.clone());
            self.ledger = Some(ledger);
            Ok(result)
        });
        if result.is_err() {
            self.close();
        }
        result
    }
    fn step_decision(
        &mut self,
        py: Python<'_>,
        action_json: &str,
        tick_budget: usize,
        max_advances: usize,
    ) -> PyResult<String> {
        if action_json.len() > 4096 || !(1..=108000).contains(&tick_budget) {
            return Err(PyValueError::new_err(
                "Invalid decision action size or tick budget",
            ));
        }
        let action =
            serde_json::from_str(action_json).map_err(|e| PyValueError::new_err(format!("{e}")))?;
        let mut ledger = self
            .ledger
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Reset a decision session before stepping"))?;
        let result = py.detach(|| {
            ledger
                .step(self.live()?, &action, tick_budget, max_advances)
                .map_err(PyRuntimeError::new_err)
        });
        match result {
            Ok(result) => {
                self.public = Some(ledger.state.clone());
                self.ledger = Some(ledger);
                Ok(result)
            }
            Err(error) => {
                self.close();
                Err(error)
            }
        }
    }
    /// Compile public model metadata once, outside the per-decision path.
    fn configure_policy(&mut self, py: Python<'_>, metadata_json: &[u8]) -> PyResult<()> {
        if metadata_json.len() > 8 << 20 {
            return Err(PyValueError::new_err("Policy metadata exceeds byte limit"));
        }
        let encoder = py
            .detach(|| {
                let metadata = serde_json::from_slice(metadata_json).map_err(|e| e.to_string())?;
                crate::policy::PolicyEncoder::new(&metadata)
            })
            .map_err(PyValueError::new_err)?;
        self.policy = Some(encoder);
        Ok(())
    }
    /// Read the same allowlisted Rust state as observation(), with no Python
    /// state projection or numeric tuples. History contains only public progress.
    fn encode_policy<'py>(
        &mut self,
        py: Python<'py>,
        expected_revision: u64,
        history_json: &[u8],
        actions_json: &[u8],
    ) -> PyResult<(
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
    )> {
        if history_json.len() > 1 << 20 || actions_json.len() > 8 << 20 {
            return Err(PyValueError::new_err("Policy input exceeds byte limit"));
        }
        self.live()?;
        let state = self
            .public
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Observe before numeric encoding"))?;
        if state["revision"].as_u64() != Some(expected_revision) {
            return Err(PyValueError::new_err(
                "Numeric observation revision mismatch",
            ));
        }
        let encoder = self
            .policy
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Configure policy before numeric encoding"))?;
        let encoded = py
            .detach(|| {
                let history = serde_json::from_slice(history_json).map_err(|e| e.to_string())?;
                let actions: Vec<serde_json::Value> =
                    serde_json::from_slice(actions_json).map_err(|e| e.to_string())?;
                encoder.encode(state, &history, &actions)
            })
            .map_err(PyValueError::new_err)?;
        policy_buffers(py, &encoded)
    }
    fn encode_ledger_policy<'py>(
        &mut self,
        py: Python<'py>,
        expected_revision: u64,
        actions_json: &[u8],
    ) -> PyResult<PolicyBuffers<'py>> {
        if actions_json.len() > 8 << 20 {
            return Err(PyValueError::new_err("Policy input exceeds byte limit"));
        }
        let ledger = self
            .ledger
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("Reset a decision session before encoding"))?;
        if ledger.state["revision"].as_u64() != Some(expected_revision) {
            return Err(PyValueError::new_err(
                "Numeric observation revision mismatch",
            ));
        }
        let encoder = self
            .policy
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Configure policy before numeric encoding"))?;
        let encoded = py
            .detach(|| {
                let actions: Vec<serde_json::Value> =
                    serde_json::from_slice(actions_json).map_err(|e| e.to_string())?;
                encoder.encode(&ledger.state, &ledger.policy_history, &actions)
            })
            .map_err(PyValueError::new_err)?;
        policy_buffers(py, &encoded)
    }
    fn status(&mut self) -> PyResult<String> {
        self.live()?
            .status()
            .map(|v| v.to_string())
            .map_err(PyRuntimeError::new_err)
    }
    fn build_ledger_policy<'py>(
        &mut self,
        py: Python<'py>,
        expected_revision: u64,
        prefix_json: &[u8],
        prepared: Option<crate::candidates::Prepared>,
    ) -> PyResult<(
        String,
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
        Bound<'py, PyBytes>,
    )> {
        if prefix_json.len() > 8 << 20 {
            return Err(PyValueError::new_err("Policy input exceeds byte limit"));
        }
        let ledger = self
            .ledger
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Reset a decision session before encoding"))?;
        if ledger.state["revision"].as_u64() != Some(expected_revision) {
            return Err(PyValueError::new_err(
                "Numeric observation revision mismatch",
            ));
        }
        let encoder = self
            .policy
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Configure policy before numeric encoding"))?;
        let (actions, encoded) = py
            .detach(|| {
                let prefix = serde_json::from_slice(prefix_json).map_err(|e| e.to_string())?;
                let actions = crate::candidates::build(
                    prefix,
                    prepared,
                    &ledger.blocked,
                    ledger.state["personalEnded"] == true,
                )?;
                let encoded = encoder.encode(&ledger.state, &ledger.policy_history, &actions)?;
                Ok::<_, String>((
                    serde_json::to_string(&actions).map_err(|e| e.to_string())?,
                    encoded,
                ))
            })
            .map_err(PyValueError::new_err)?;
        let (observation, candidates, schedules, experts) = policy_buffers(py, &encoded)?;
        Ok((actions, observation, candidates, schedules, experts))
    }
    /// Private debugging data, including RNG and pool state. Not a policy input.
    fn diagnostics(&mut self, py: Python<'_>) -> PyResult<String> {
        let live = self.live()?;
        py.detach(|| live.diagnostics())
            .map(|v| v.to_string())
            .map_err(PyRuntimeError::new_err)
    }
    fn replay_frame(&mut self, py: Python<'_>) -> PyResult<String> {
        let live = self.live()?;
        py.detach(|| live.replay_frame())
            .map(|frame| frame.to_string())
            .map_err(PyRuntimeError::new_err)
    }
    fn movement_batch_ticks(&mut self) -> PyResult<Option<u64>> {
        self.live()?
            .movement_batch_ticks()
            .map_err(PyRuntimeError::new_err)
    }
    fn movement_path_steps(&mut self) -> PyResult<Option<[u64; 2]>> {
        self.live()?
            .movement_path_steps()
            .map_err(PyRuntimeError::new_err)
    }
    fn event_window_stats(&mut self) -> PyResult<Option<[u64; 2]>> {
        self.live()?
            .event_window_stats()
            .map_err(PyRuntimeError::new_err)
    }
    fn close(&mut self) {
        self.session = None;
        self.public = None;
        self.ledger = None;
    }
}

type PolicyBuffers<'py> = (
    Bound<'py, PyBytes>,
    Bound<'py, PyBytes>,
    Bound<'py, PyBytes>,
    Bound<'py, PyBytes>,
);

fn policy_buffers<'py>(
    py: Python<'py>,
    encoded: &crate::policy::Encoded,
) -> PyResult<PolicyBuffers<'py>> {
    let floats = |values: &[f32]| {
        PyBytes::new_with(py, values.len() * 4, |bytes| {
            for (out, value) in bytes.chunks_exact_mut(4).zip(values) {
                out.copy_from_slice(&value.to_le_bytes());
            }
            Ok(())
        })
    };
    let ints = |values: &[i64]| {
        PyBytes::new_with(py, values.len() * 8, |bytes| {
            for (out, value) in bytes.chunks_exact_mut(8).zip(values) {
                out.copy_from_slice(&value.to_le_bytes());
            }
            Ok(())
        })
    };
    Ok((
        floats(&encoded.observation)?,
        floats(&encoded.candidates)?,
        ints(&encoded.schedules)?,
        ints(&encoded.experts)?,
    ))
}
