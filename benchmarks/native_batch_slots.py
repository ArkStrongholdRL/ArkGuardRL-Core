"""Rust-owned parallel slots; existing public Python history/reward are retained.

Use the current Core source on PYTHONPATH and an explicitly built _native_slots.
The module is an experiment, not a replacement installation of the Core package.
"""
import importlib.machinery
import importlib.util
import json
from copy import deepcopy
from pathlib import Path
import time

import numpy as np
from arkguardrl.catalog import PublicCatalog
from arkguardrl.candidates import DecisionHistory, build_decision
from arkguardrl.contracts import action_id, canonical
from arkguardrl.environment import Transition
from arkguardrl.feedback import PublicProgress
from arkguardrl.observation import validate_state
from arkguardrl.rewards import NativeProgressCore
from arkguardrl_core.native_policy import metadata


def load_slots(path):
    path = str(Path(path).resolve(strict=True))
    loader = importlib.machinery.ExtensionFileLoader('_native_slots', path)
    spec = importlib.util.spec_from_file_location('_native_slots', path, loader=loader)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class _Slot:
    def __init__(self):
        self.rng = np.random.default_rng()
        self.state = self.decision = None
        self.done = True
        self.info = {}

    def reset(self, engine_seed):
        self.history, self.progress = DecisionHistory(), PublicProgress()
        self.reward = NativeProgressCore()
        self.decisions = self.advances = 0
        self.reason = None
        self.engine_seed = engine_seed
        self.done = False
        self.started = time.monotonic()
        self.info = {}


class NativeSlotBatch:
    """One batch call runs all native battles; one more encodes all model inputs.

    Explicit masked reset, no autoreset. Output arrays own their memory. Runtime
    errors invalidate the entire Python facade, because native slots may already
    have advanced when a different worker fails. A full reset recovers the batch.
    """
    def __init__(self, data_root, *, core, num_envs=1, threads=1, native_library,
                 banned=(), max_candidates=16384, max_decisions=1024,
                 max_advances=4096, tick_budget=4096, wall_seconds=None,
                 max_output_bytes=256 << 20):
        if type(num_envs) is not int or not 1 <= num_envs <= 64:
            raise ValueError('num_envs must be 1..64')
        if type(threads) is not int or not 1 <= threads <= num_envs:
            raise ValueError('threads must be 1..num_envs')
        if type(max_candidates) is not int or not 1 <= max_candidates <= 16384:
            raise ValueError('Invalid candidate capacity')
        if any(type(n) is not int or n <= 0 for n in (max_decisions, max_advances)):
            raise ValueError('Invalid decision/advance budgets')
        if type(tick_budget) is not int or not 1 <= tick_budget <= 108000:
            raise ValueError('Invalid tick budget')
        if wall_seconds is not None and (not np.isfinite(wall_seconds) or wall_seconds <= 0):
            raise ValueError('Invalid wall budget')
        output_bytes = num_envs * (4784 * 4 + max_candidates * (41 * 4 + 17) + 10)
        if output_bytes > max_output_bytes:
            raise ValueError('Batch output exceeds memory limit')
        native = load_slots(native_library)
        catalog = native.NativeCatalog(str(Path(data_root).resolve()), 64 << 20)
        self.catalog = PublicCatalog.from_native(catalog)
        self.catalog.definitions()
        cores = [core] * num_envs if isinstance(core, str) else list(core)
        if len(cores) != num_envs:
            raise ValueError('One core per slot required')
        self.native = native.NativeBatch(catalog, cores, [list(banned) for _ in cores], threads)
        self.native.configure_policy(metadata(self.catalog))
        self.max_candidates, self.num_envs = max_candidates, num_envs
        self.max_decisions, self.max_advances = max_decisions, max_advances
        self.tick_budget, self.wall_seconds = tick_budget, wall_seconds
        self.slots = [_Slot() for _ in cores]
        self._closed = self._faulted = False

    def _mask(self, value):
        mask = np.ones(self.num_envs, bool) if value is None else np.asarray(value)
        if mask.shape != (self.num_envs,) or mask.dtype != np.bool_:
            raise ValueError('Mask must be a boolean vector with num_envs entries')
        return mask

    def _adopt(self, slot, raw):
        state = json.loads(raw)
        validate_state(state)
        slot.state = state
        return state

    def _refresh(self, slot):
        if slot.state['decisionRequired'] and not slot.state['personalEnded']:
            # Enumerate exactly the old public domain/history. The expensive
            # numerical projection is issued later, once for all slots in Rust.
            slot.decision = build_decision(slot.state, self.catalog, slot.history,
                                           encoder=lambda *_: ((), (), (), ()))
            if len(slot.decision.actions) > self.max_candidates:
                raise ValueError('Candidate capacity exceeded; no domain truncation')
        else:
            slot.decision = None
        slot.info = dict(candidate_count=len(slot.decision.actions) if slot.decision else 0,
                         phase=slot.state['phase'], round=slot.state['round'],
                         engine_seed=slot.engine_seed, reason=slot.reason or '')

    def _output(self):
        n, k = self.num_envs, self.max_candidates
        histories = [canonical(s.history.snapshot(s.state)) for s in self.slots]
        actions = [canonical(s.decision.actions if s.decision else ()) for s in self.slots]
        buffers = self.native.encode_all([s.state['revision'] for s in self.slots], histories, actions)
        obs = dict(observation=np.empty((n, 4784), np.float32),
                   candidates=np.zeros((n, k, 41), np.float32),
                   schedules=np.zeros((n, k), np.int64), experts=np.zeros((n, k), np.int64),
                   action_mask=np.zeros((n, k), np.int8))
        for i, slot in enumerate(self.slots):
            obs['observation'][i] = np.frombuffer(buffers[0][i], dtype='<f4')
            count = len(slot.decision.actions) if slot.decision else 0
            if count:
                obs['candidates'][i, :count] = np.frombuffer(buffers[1][i], dtype='<f4').reshape(count, 41)
                obs['schedules'][i, :count] = np.frombuffer(buffers[2][i], dtype='<i8')
                obs['experts'][i, :count] = np.frombuffer(buffers[3][i], dtype='<i8')
                obs['action_mask'][i, :count] = 1
        return obs, [deepcopy(s.info) for s in self.slots]

    def reset(self, *, seed=None, reset_mask=None):
        if self._closed:
            raise RuntimeError('Batch is closed')
        mask = self._mask(reset_mask)
        if self._faulted and not mask.all():
            raise RuntimeError('Full reset required after runtime error')
        if any(s.state is None and not flag for s, flag in zip(self.slots, mask)):
            raise ValueError('Initial reset must initialize every slot')
        if type(seed) is int:
            seeds = [seed + i for i in range(self.num_envs)]
        else:
            seeds = [None] * self.num_envs if seed is None else list(seed)
        if len(seeds) != self.num_envs or any(s is not None and (type(s) is not int or s < 0) for s in seeds):
            raise ValueError('Invalid reset seeds')
        engines = []
        for slot, s, flag in zip(self.slots, seeds, mask):
            if not flag:
                engines.append(None)
            else:
                if s is not None:
                    slot.rng = np.random.default_rng(s)
                engines.append(int(slot.rng.integers(1, 2**32)))
        try:
            self.native.reset(engines)
            raw = self.native.observe_all()
            for i in np.flatnonzero(mask):
                slot = self.slots[i]
                slot.reset(engines[i])
                state = self._adopt(slot, raw[i])
                slot.progress._observe_validated(state)
                slot.history.initialize(Transition(state, False, False, None,
                                                  progress=slot.progress.snapshot(state)))
                slot.reward.reset(state)
                self._refresh(slot)
            self._faulted = False
            return self._output()
        except BaseException:
            self._faulted = True
            self.native.close()
            raise

    def step(self, actions, *, active_mask=None):
        if self._closed or self._faulted or any(s.state is None for s in self.slots):
            raise RuntimeError('Reset a live batch before stepping')
        mask = self._mask(active_mask)
        actions = np.asarray(actions)
        if actions.shape != (self.num_envs,) or actions.dtype.kind not in 'iu':
            raise ValueError('Actions must be an integer vector with num_envs entries')
        query, budgets = [], []
        for slot, index, flag in zip(self.slots, actions, mask):
            if not flag:
                query.append(None); budgets.append(0); continue
            if slot.done or slot.decision is None:
                raise RuntimeError('Explicit reset required for ended slots')
            if not 0 <= int(index) < len(slot.decision.actions):
                raise ValueError('Select a current unpadded candidate index')
            query.append(json.dumps(slot.decision.actions[int(index)], allow_nan=False))
            budgets.append(max(0, self.max_advances - slot.advances))
        try:
            immediate, final, receipts, counts = self.native.step_episode_all(query, self.tick_budget, budgets)
            rewards = np.zeros(self.num_envs, np.float64)
            for i in np.flatnonzero(mask):
                slot = self.slots[i]
                intent = deepcopy(slot.decision.actions[int(actions[i])])
                receipt = json.loads(receipts[i])
                before = slot.state
                state = self._adopt(slot, immediate[i])
                accepted = receipt['accepted']
                reply = {'accepted': accepted, 'result': {'ok': accepted}}
                if not accepted:
                    reply['result']['error'] = receipt['rejection']
                execution = slot.progress._observe_validated(state, action=intent, reply=reply)
                slot.decisions += 1
                slot.advances += counts[i]
                if counts[i]:
                    self._adopt(slot, final[i])
                    slot.progress._observe_validated(slot.state)
                terminated = slot.state['personalEnded']
                if not terminated:
                    if slot.decisions >= self.max_decisions:
                        slot.reason = 'decision-budget'
                    elif slot.advances >= self.max_advances:
                        slot.reason = 'advance-budget'
                result = Transition(slot.state, terminated, bool(slot.reason and not terminated),
                                    slot.reason, accepted=accepted, action_id=action_id(intent),
                                    execution=execution, progress=slot.progress.snapshot(slot.state))
                slot.history.record(before, intent, result=result)
                reward = slot.reward.step(before, slot.state)
                if not terminated and self.wall_seconds is not None and time.monotonic() - slot.started >= self.wall_seconds:
                    slot.reason = 'wall-budget'
                slot.done = terminated or bool(slot.reason)
                self._refresh(slot)
                slot.info['accepted'] = accepted
                slot.info['reward_components'] = reward['components']
                rewards[i] = reward['reward']
            obs, info = self._output()
            return (obs, rewards, np.array([s.state['personalEnded'] for s in self.slots], bool),
                    np.array([bool(s.reason and not s.state['personalEnded']) for s in self.slots], bool), info)
        except BaseException:
            self._faulted = True
            self.native.close()
            raise

    def close(self):
        self.native.close()
        self._closed = True

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
