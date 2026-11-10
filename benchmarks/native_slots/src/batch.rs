// SPDX-License-Identifier: GPL-3.0-or-later
//! Ordered native slots. All engine work and numeric encoding run without the GIL.
use crate::{policy::PolicyEncoder, NativeCatalog};
use arkguard_engine::{
    data::Catalog,
    solo::session::{Session, MAX_TICK_BUDGET},
};
use pyo3::{
    exceptions::{PyRuntimeError, PyValueError},
    prelude::*,
    types::PyBytes,
};
use serde_json::Value;
use std::sync::Arc;

type Result<T> = std::result::Result<T, String>;
type Wire = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);
type PolicyBytes<'py> = (
    Vec<Bound<'py, PyBytes>>,
    Vec<Bound<'py, PyBytes>>,
    Vec<Bound<'py, PyBytes>>,
    Vec<Bound<'py, PyBytes>>,
);

struct Episode {
    session: Session,
    public: Option<Value>,
}

/// Join every worker, including after errors; flatten chunks in slot order.
/// Workers never share mutable slots and may not stop other slots on failure.
fn parallel<T: Send, U: Send, F>(items: &mut [T], threads: usize, f: F) -> Vec<Result<U>>
where
    F: Fn(&mut T, usize) -> Result<U> + Sync,
{
    let total = items.len();
    let chunk_size = total.div_ceil(threads);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for (chunk_index, chunk) in items.chunks_mut(chunk_size).enumerate() {
            let f = &f;
            let start = chunk_index * chunk_size;
            let len = chunk.len();
            workers.push((
                len,
                scope.spawn(move || {
                    chunk
                        .iter_mut()
                        .enumerate()
                        .map(|(i, item)| f(item, start + i))
                        .collect::<Vec<_>>()
                }),
            ));
        }
        let mut results = Vec::with_capacity(total);
        for (len, worker) in workers {
            match worker.join() {
                Ok(chunk) => results.extend(chunk),
                Err(_) => {
                    results.extend((0..len).map(|_| Err("Native batch worker panicked".into())))
                }
            }
        }
        results
    })
}

fn live(episode: &mut Option<Episode>) -> Result<&mut Episode> {
    episode.as_mut().ok_or_else(|| "Reset required".into())
}
fn runtime<T>(results: Vec<Result<T>>) -> PyResult<Vec<T>> {
    results
        .into_iter()
        .enumerate()
        .map(|(slot, result)| {
            result.map_err(|error| PyRuntimeError::new_err(format!("Slot {slot}: {error}")))
        })
        .collect()
}
fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn i64_bytes(values: &[i64]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[pyclass]
pub struct NativeBatch {
    data: Arc<Catalog>,
    cores: Vec<String>,
    banned: Vec<Vec<String>>,
    threads: usize,
    episodes: Vec<Option<Episode>>,
    policy: Option<Arc<PolicyEncoder>>,
}
impl NativeBatch {
    fn length(&self, len: usize, name: &str) -> PyResult<()> {
        if len != self.episodes.len() {
            return Err(PyValueError::new_err(format!(
                "{name} length must equal slot count"
            )));
        }
        Ok(())
    }
    fn invalidate(&mut self) {
        for episode in self.episodes.iter_mut().flatten() {
            episode.public = None;
        }
    }
    fn parse_actions(&self, py: Python<'_>, actions: &[String]) -> PyResult<Vec<Value>> {
        self.length(actions.len(), "Actions")?;
        if actions.iter().any(|action| action.len() > 4096) {
            return Err(PyValueError::new_err("Action exceeds byte limit"));
        }
        py.detach(|| {
            actions
                .iter()
                .map(|action| serde_json::from_str(action))
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .map_err(|error| PyValueError::new_err(error.to_string()))
    }
}

#[pymethods]
impl NativeBatch {
    #[new]
    fn new(
        catalog: &NativeCatalog,
        cores: Vec<String>,
        banned: Vec<Vec<String>>,
        threads: usize,
    ) -> PyResult<Self> {
        if !(1..=64).contains(&cores.len()) {
            return Err(PyValueError::new_err("Slot count must be in 1..=64"));
        }
        if cores.len() != banned.len() {
            return Err(PyValueError::new_err(
                "Cores and bans must have equal lengths",
            ));
        }
        if !(1..=cores.len()).contains(&threads) {
            return Err(PyValueError::new_err("Threads must be in 1..=slot count"));
        }
        if cores.iter().any(|core| {
            !matches!(
                core.as_str(),
                "band_amiya" | "band_sarkazb" | "band_chen" | "band_sciurus"
            )
        }) {
            return Err(PyValueError::new_err("Unsupported core"));
        }
        if banned
            .iter()
            .any(|bans| bans.len() > 4096 || bans.iter().any(|id| id.len() > 256))
        {
            return Err(PyValueError::new_err("Manual bans exceed capacity"));
        }
        let episodes = (0..cores.len()).map(|_| None).collect();
        Ok(Self {
            data: Arc::clone(&catalog.inner),
            cores,
            banned,
            threads,
            episodes,
            policy: None,
        })
    }

    /// A failed reset leaves all selected slots without episodes, never half
    /// initialized. Unselected slots and their public caches are left untouched.
    fn reset(&mut self, py: Python<'_>, seeds: Vec<Option<u32>>) -> PyResult<Vec<String>> {
        self.length(seeds.len(), "Seeds")?;
        if seeds
            .iter()
            .zip(&self.episodes)
            .any(|(seed, episode)| seed.is_none() && episode.is_none())
        {
            return Err(PyValueError::new_err(
                "Initial reset requires a seed for every slot",
            ));
        }
        for (episode, seed) in self.episodes.iter_mut().zip(&seeds) {
            if seed.is_some() {
                *episode = None;
            }
        }
        let data = &self.data;
        let cores = &self.cores;
        let banned = &self.banned;
        let results = py.detach(|| {
            parallel(
                &mut self.episodes,
                self.threads,
                |episode, slot| match seeds[slot] {
                    Some(seed) => {
                        let session =
                            Session::new(Arc::clone(data), seed, &cores[slot], &banned[slot])?;
                        let status = session.status()?.to_string();
                        Ok((
                            Some(Episode {
                                session,
                                public: None,
                            }),
                            status,
                        ))
                    }
                    None => Ok((None, live(episode)?.session.status()?.to_string())),
                },
            )
        });
        let initialized = runtime(results)?;
        let mut statuses = Vec::with_capacity(initialized.len());
        for (target, (episode, status)) in self.episodes.iter_mut().zip(initialized) {
            if let Some(episode) = episode {
                *target = Some(episode);
            }
            statuses.push(status);
        }
        Ok(statuses)
    }

    /// Technical errors return a whole-batch error, but successful/partially
    /// advanced slots are retained. This operation is deliberately not transactional.
    fn step_all(
        &mut self,
        py: Python<'_>,
        actions: Vec<String>,
        tick_budget: usize,
    ) -> PyResult<Vec<String>> {
        let actions = self.parse_actions(py, &actions)?;
        self.invalidate();
        runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                live(episode)?
                    .session
                    .step(&actions[slot], tick_budget)
                    .map(|receipt| receipt.to_string())
            })
        }))
    }

    /// Match NativeDecisionEnv.step, including observing the action's immediate
    /// public state before combat advances. Errors never roll back advanced slots.
    fn step_episode_all(
        &mut self,
        py: Python<'_>,
        actions: Vec<Option<String>>,
        tick_budget: usize,
        max_advances: Vec<usize>,
    ) -> PyResult<(Vec<String>, Vec<String>, Vec<String>, Vec<usize>)> {
        self.length(actions.len(), "Actions")?;
        self.length(max_advances.len(), "Max advances")?;
        if actions.iter().flatten().any(|action| action.len() > 4096) {
            return Err(PyValueError::new_err("Action exceeds byte limit"));
        }
        let actions: Vec<Option<Value>> = py
            .detach(|| {
                actions
                    .iter()
                    .map(|action| {
                        action
                            .as_ref()
                            .map(|json| serde_json::from_str(json))
                            .transpose()
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        if tick_budget > MAX_TICK_BUDGET {
            return Err(PyValueError::new_err("Tick budget exceeds per-call limit"));
        }
        for (episode, action) in self.episodes.iter_mut().zip(&actions) {
            if action.is_some() {
                if let Some(episode) = episode {
                    episode.public = None;
                }
            }
        }
        let results = py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                let Some(action) = &actions[slot] else {
                    return Ok((String::new(), String::new(), "{}".into(), 0, None));
                };
                let session = &mut live(episode)?.session;
                let mut receipt = session.step(action, 0)?;
                if !receipt.is_object() {
                    return Err("Step receipt must be an object".into());
                }
                // Preserve accepted/rejection from the action receipt: automatic
                // advance receipts deliberately have accepted=null.
                let action_receipt_json = receipt.to_string();
                let immediate = session.observation()?;
                let immediate_json = immediate.to_string();
                let mut advances = 0;
                while receipt["budgetExhausted"].as_bool() == Some(true) {
                    if advances >= max_advances[slot] {
                        return Err("Maximum combat advances exhausted".into());
                    }
                    receipt = session.advance(tick_budget)?;
                    advances += 1;
                    if !receipt.is_object() {
                        return Err("Advance receipt must be an object".into());
                    }
                    if advances >= max_advances[slot]
                        && receipt["budgetExhausted"].as_bool() == Some(true)
                    {
                        return Err("Maximum combat advances exhausted".into());
                    }
                }
                let final_state = if advances == 0 {
                    immediate
                } else {
                    session.observation()?
                };
                Ok((
                    immediate_json,
                    final_state.to_string(),
                    action_receipt_json,
                    advances,
                    Some(final_state),
                ))
            })
        });
        let mut immediate = Vec::with_capacity(results.len());
        let mut final_states = Vec::with_capacity(results.len());
        let mut receipts = Vec::with_capacity(results.len());
        let mut counts = Vec::with_capacity(results.len());
        let mut error = None;
        // Merge caches only after leaving the detached scope; retain successful
        // caches even if another slot failed, just as its advanced engine state.
        for (slot, result) in results.into_iter().enumerate() {
            match result {
                Ok((first, last, receipt, advances, state)) => {
                    if let Some(state) = state {
                        self.episodes[slot].as_mut().unwrap().public = Some(state);
                    }
                    immediate.push(first);
                    final_states.push(last);
                    receipts.push(receipt);
                    counts.push(advances);
                }
                Err(message) => {
                    if error.is_none() {
                        error = Some(format!("Slot {slot}: {message}"));
                    }
                }
            }
        }
        if let Some(error) = error {
            return Err(PyRuntimeError::new_err(error));
        }
        Ok((immediate, final_states, receipts, counts))
    }

    fn public_spatial_all(&mut self, py: Python<'_>) -> PyResult<Vec<Option<String>>> {
        runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, _| {
                let session = &live(episode)?.session;
                if session.status()?["decisionRequired"] != true {
                    return Ok(None);
                }
                session
                    .public_spatial()
                    .map(|spatial| Some(spatial.to_string()))
            })
        }))
    }

    fn observe_all(&mut self, py: Python<'_>) -> PyResult<Vec<String>> {
        let results = py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, _| {
                let episode = live(episode)?;
                // Reuse only an observation of this same unmodified revision.
                let state = match &episode.public {
                    Some(state) => state.clone(),
                    None => episode.session.observation()?,
                };
                let json = state.to_string();
                Ok((state, json))
            })
        });
        let mut strings = Vec::with_capacity(results.len());
        let mut error = None;
        for (slot, result) in results.into_iter().enumerate() {
            match result {
                Ok((state, json)) => {
                    self.episodes[slot].as_mut().unwrap().public = Some(state);
                    strings.push(json);
                }
                Err(message) => {
                    if error.is_none() {
                        error = Some(format!("Slot {slot}: {message}"));
                    }
                }
            }
        }
        if let Some(error) = error {
            return Err(PyRuntimeError::new_err(error));
        }
        Ok(strings)
    }

    fn configure_policy(&mut self, py: Python<'_>, metadata_json: &[u8]) -> PyResult<()> {
        if metadata_json.len() > 8 << 20 {
            return Err(PyValueError::new_err("Policy metadata exceeds byte limit"));
        }
        let encoder = py
            .detach(|| {
                let metadata = serde_json::from_slice(metadata_json).map_err(|e| e.to_string())?;
                PolicyEncoder::new(&metadata)
            })
            .map_err(PyValueError::new_err)?;
        self.policy = Some(Arc::new(encoder));
        Ok(())
    }

    fn encode_all<'py>(
        &mut self,
        py: Python<'py>,
        revisions: Vec<u64>,
        histories: Vec<Vec<u8>>,
        actions: Vec<Vec<u8>>,
    ) -> PyResult<PolicyBytes<'py>> {
        self.length(revisions.len(), "Revisions")?;
        self.length(histories.len(), "Histories")?;
        self.length(actions.len(), "Actions")?;
        if histories.iter().any(|h| h.len() > 1 << 20) || actions.iter().any(|a| a.len() > 8 << 20)
        {
            return Err(PyValueError::new_err("Policy input exceeds byte limit"));
        }
        for (episode, action) in self.episodes.iter().zip(&actions) {
            if action.is_empty() {
                continue;
            }
            let episode = episode
                .as_ref()
                .ok_or_else(|| PyRuntimeError::new_err("Reset required"))?;
            if episode.public.is_none() {
                return Err(PyRuntimeError::new_err("Observe before numeric encoding"));
            }
        }
        let policy = self
            .policy
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Configure policy before numeric encoding"))?;
        let results: Vec<Result<Wire>> = py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                if actions[slot].is_empty() {
                    return Ok((Vec::new(), Vec::new(), Vec::new(), Vec::new()));
                }
                let state = live(episode)?
                    .public
                    .as_ref()
                    .ok_or("Observe before numeric encoding")?;
                if state["revision"].as_u64() != Some(revisions[slot]) {
                    return Err("Numeric observation revision mismatch".into());
                }
                let history =
                    serde_json::from_slice(&histories[slot]).map_err(|e| e.to_string())?;
                let actions: Vec<Value> =
                    serde_json::from_slice(&actions[slot]).map_err(|e| e.to_string())?;
                let encoded = policy.encode(state, &history, &actions)?;
                Ok((
                    f32_bytes(&encoded.observation),
                    f32_bytes(&encoded.candidates),
                    i64_bytes(&encoded.schedules),
                    i64_bytes(&encoded.experts),
                ))
            })
        });
        let mut observations = Vec::with_capacity(results.len());
        let mut candidates = Vec::with_capacity(results.len());
        let mut schedules = Vec::with_capacity(results.len());
        let mut experts = Vec::with_capacity(results.len());
        for (slot, result) in results.into_iter().enumerate() {
            let (observation, candidate, schedule, expert) =
                result.map_err(|e| PyValueError::new_err(format!("Slot {slot}: {e}")))?;
            observations.push(PyBytes::new(py, &observation));
            candidates.push(PyBytes::new(py, &candidate));
            schedules.push(PyBytes::new(py, &schedule));
            experts.push(PyBytes::new(py, &expert));
        }
        Ok((observations, candidates, schedules, experts))
    }

    fn close(&mut self) {
        for episode in &mut self.episodes {
            *episode = None;
        }
    }
}
