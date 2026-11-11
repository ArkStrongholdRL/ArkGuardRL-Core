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
    ledger: Option<crate::ledger::DecisionLedger>,
    domain: Option<crate::ledger::PolicyDomain>,
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
    policy_token: u64,
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
            policy_token: 0,
        })
    }

    /// Commit selected episodes only after every selected initialization succeeds.
    fn reset_decision_all(
        &mut self,
        py: Python<'_>,
        seeds: Vec<Option<u32>>,
        streams: Vec<Option<String>>,
    ) -> PyResult<Vec<Option<String>>> {
        self.length(seeds.len(), "Seeds")?;
        self.length(streams.len(), "Streams")?;
        if seeds.iter().zip(&streams).any(|(seed, stream)| {
            seed.is_some() != stream.is_some() || stream.as_ref().is_some_and(|s| s.len() != 32)
        }) {
            return Err(PyValueError::new_err(
                "One progress stream per reset slot required",
            ));
        }
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
        let initialized = runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                let Some(seed) = seeds[slot] else {
                    live(episode)?;
                    return Ok(None);
                };
                let session = Session::new(Arc::clone(data), seed, &cores[slot], &banned[slot])?;
                let state = Arc::new(session.observation()?);
                let mut ledger = crate::ledger::DecisionLedger::new(
                    state,
                    streams[slot].as_ref().unwrap().clone(),
                )?;
                let snapshot = ledger.snapshot(None, None, 0)?;
                Ok(Some((
                    Episode {
                        session,
                        ledger: Some(ledger),
                        domain: None,
                    },
                    snapshot,
                )))
            })
        }))?;
        let mut snapshots = Vec::with_capacity(initialized.len());
        for (target, initialized) in self.episodes.iter_mut().zip(initialized) {
            snapshots.push(match initialized {
                Some((episode, snapshot)) => {
                    *target = Some(episode);
                    Some(snapshot)
                }
                None => None,
            });
        }
        Ok(snapshots)
    }

    fn step_index_all(
        &mut self,
        py: Python<'_>,
        revisions: Vec<u64>,
        tokens: Vec<u64>,
        indices: Vec<Option<usize>>,
        tick_budget: usize,
        max_advances: Vec<usize>,
    ) -> PyResult<Vec<Option<String>>> {
        self.length(revisions.len(), "Revisions")?;
        self.length(tokens.len(), "Candidate tokens")?;
        self.length(indices.len(), "Indices")?;
        self.length(max_advances.len(), "Max advances")?;
        if !(1..=MAX_TICK_BUDGET).contains(&tick_budget) {
            return Err(PyValueError::new_err("Invalid decision tick budget"));
        }
        // Validate the whole selection vector before any slot advances.
        let actions = self
            .episodes
            .iter()
            .enumerate()
            .map(|(slot, episode)| {
                let Some(index) = indices[slot] else {
                    return Ok(None);
                };
                let domain = episode
                    .as_ref()
                    .and_then(|e| e.domain.as_ref())
                    .ok_or_else(|| {
                        PyValueError::new_err("Build a native candidate domain before stepping")
                    })?;
                domain
                    .selected(revisions[slot], tokens[slot], index)
                    .map(Some)
                    .map_err(PyValueError::new_err)
            })
            .collect::<PyResult<Vec<_>>>()?;
        runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                let Some(action) = &actions[slot] else {
                    return Ok(None);
                };
                let episode = live(episode)?;
                episode.domain = None;
                let mut ledger = episode
                    .ledger
                    .take()
                    .ok_or("Reset a decision session before stepping")?;
                let result = ledger.step(
                    &mut episode.session,
                    action,
                    tick_budget,
                    max_advances[slot],
                )?;
                episode.ledger = Some(ledger);
                Ok(Some(result))
            })
        }))
    }

    fn build_current_policy_all<'py>(
        &mut self,
        py: Python<'py>,
        revisions: Vec<u64>,
    ) -> PyResult<(Vec<u64>, Vec<String>, PolicyBytes<'py>)> {
        self.length(revisions.len(), "Revisions")?;
        let policy = self
            .policy
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("Configure policy before numeric encoding"))?;
        let token = self.policy_token;
        self.policy_token = token
            .checked_add(self.episodes.len() as u64)
            .ok_or_else(|| PyRuntimeError::new_err("Candidate token overflow"))?;
        let results = py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                let episode = live(episode)?;
                let ledger = episode
                    .ledger
                    .as_ref()
                    .ok_or("Reset a decision session before encoding")?;
                if ledger.state["revision"].as_u64() != Some(revisions[slot]) {
                    return Err("Numeric observation revision mismatch".into());
                }
                episode.domain = None;
                let actions = policy.current_candidates(
                    &ledger.state,
                    &ledger.policy_history,
                    &ledger.blocked,
                )?;
                let encoded = policy.encode(&ledger.state, &ledger.policy_history, &actions)?;
                let json = serde_json::to_string(&actions).map_err(|e| e.to_string())?;
                let token = token + slot as u64 + 1;
                episode.domain = Some(crate::ledger::PolicyDomain {
                    revision: revisions[slot],
                    token,
                    actions,
                });
                Ok((
                    token,
                    json,
                    (
                        f32_bytes(&encoded.observation),
                        f32_bytes(&encoded.candidates),
                        i64_bytes(&encoded.schedules),
                        i64_bytes(&encoded.experts),
                    ),
                ))
            })
        });
        let mut tokens = Vec::with_capacity(results.len());
        let mut actions = Vec::with_capacity(results.len());
        let mut buffers = Vec::with_capacity(results.len());
        for (slot, result) in results.into_iter().enumerate() {
            let (token, json, wire) =
                result.map_err(|e| PyValueError::new_err(format!("Slot {slot}: {e}")))?;
            tokens.push(token);
            actions.push(json);
            buffers.push(wire);
        }
        Ok((tokens, actions, policy_bytes(py, buffers)))
    }

    fn public_spatial_all(&mut self, py: Python<'_>) -> PyResult<Vec<Option<String>>> {
        runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, _| {
                let session = &live(episode)?.session;
                if session.status()?["decisionRequired"] != true {
                    return Ok(None);
                }
                session.public_spatial_json().map(Some)
            })
        }))
    }

    fn public_spatial_updates(
        &mut self,
        py: Python<'_>,
        revisions: Vec<u64>,
        keys: Vec<Option<String>>,
    ) -> PyResult<Vec<Option<(String, Option<String>, String)>>> {
        self.length(revisions.len(), "Revisions")?;
        self.length(keys.len(), "Geometry keys")?;
        if keys.iter().flatten().any(|key| key.len() > 8 << 20) {
            return Err(PyValueError::new_err(
                "Public spatial key exceeds byte limit",
            ));
        }
        runtime(py.detach(|| {
            parallel(&mut self.episodes, self.threads, |episode, slot| {
                let session = &live(episode)?.session;
                let status = session.status()?;
                if status["revision"].as_u64() != Some(revisions[slot]) {
                    return Err("Public spatial revision mismatch".into());
                }
                if status["decisionRequired"] != true {
                    return Ok(None);
                }
                session
                    .public_spatial_update(keys[slot].as_deref())
                    .map(Some)
            })
        }))
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
        for episode in self.episodes.iter_mut().flatten() {
            episode.domain = None;
        }
        Ok(())
    }

    fn close(&mut self) {
        for episode in &mut self.episodes {
            *episode = None;
        }
    }
}

fn policy_bytes<'py>(py: Python<'py>, results: Vec<Wire>) -> PolicyBytes<'py> {
    let mut observations = Vec::with_capacity(results.len());
    let mut candidates = Vec::with_capacity(results.len());
    let mut schedules = Vec::with_capacity(results.len());
    let mut experts = Vec::with_capacity(results.len());
    for (observation, candidate, schedule, expert) in results {
        observations.push(PyBytes::new(py, &observation));
        candidates.push(PyBytes::new(py, &candidate));
        schedules.push(PyBytes::new(py, &schedule));
        experts.push(PyBytes::new(py, &expert));
    }
    (observations, candidates, schedules, experts)
}
