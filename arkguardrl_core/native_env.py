"""In-process Rust decisions using the existing public feature/history contract.

Transport validates and owns each decoded observation. Internal progress consumes
that owned snapshot. Public transitions are decoded independently from the same
transport JSON, preserving caller isolation without Python recursive deepcopy.
"""
from copy import deepcopy
import importlib.machinery
import importlib.util
import json
from pathlib import Path

from arkguardrl.contracts import action_id
from arkguardrl.environment import Transition
from arkguardrl.feedback import PublicProgress
from arkguardrl.observation import validate_state


def load_native(library=None):
    if library is None:
        try:
            from . import _native
            return _native
        except ImportError as exc:
            raise RuntimeError("Build crates/python and pass its dynamic library as native_library") from exc
    path = str(Path(library).resolve(strict=True))
    loader = importlib.machinery.ExtensionFileLoader("_native", path)
    spec = importlib.util.spec_from_file_location("_native", path, loader=loader)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NativePool:
    """Own definitions once. Episodes share definitions, never mutable game state."""
    def __init__(self, data_root, core, banned, library, diy=None):
        native = load_native(library)
        self.catalog = native.NativeCatalog(str(Path(data_root).resolve()), 64 << 20)
        if diy:
            self.catalog = self.catalog.with_diy(json.dumps(diy))
        self.session = native.NativeSolo(self.catalog, core, list(banned))

    def close(self):
        self.session.close()


class NativeDecisionEnv:
    def __init__(self, pool, *, max_decisions, max_advances, tick_budget, policy_catalog=None):
        self.session = pool.session
        self.numeric_encoder = None
        if policy_catalog is not None:
            from .native_policy import metadata
            if getattr(pool, '_policy_catalog', None) is not policy_catalog:
                self.session.configure_policy(metadata(policy_catalog))
                pool._policy_catalog = policy_catalog
            self._policy_catalog = policy_catalog
            self.numeric_encoder = self._encode_numeric
        self.max_decisions, self.max_advances = max_decisions, max_advances
        self.tick_budget = tick_budget
        self._state = None
        self._raw = None
        self._progress = PublicProgress()
        self._decisions = self._advances = 0
        self._reason = None
        self._closed = False

    def __enter__(self):
        return self

    def public_spatial(self):
        """Public map/template geometry bound to the current decision revision."""
        if self._closed or self._state is None:
            raise RuntimeError("Reset required")
        return json.loads(self.session.public_spatial(self._state['revision']))

    def public_spatial_update(self, geometry_key=None):
        """Return (key, optional geometry JSON, current dynamic JSON).

        Reuse keys only with the same immutable catalog. The consumer owns its
        decoded geometry and must validate each dynamic packet's revision.
        """
        if self._closed or self._state is None:
            raise RuntimeError("Reset required")
        return self.session.public_spatial_update(geometry_key, self._state['revision'])

    def _encode_numeric(self, state, actions, catalog, history):
        from .native_policy import encode
        if catalog is not self._policy_catalog or state != self._state:
            raise ValueError('Numeric encoding requires the current public snapshot and catalog')
        return encode(self.session, state, actions, history)

    def _observe(self):
        # Validate once at the transport boundary; progress consumes the owned mapping.
        raw = self.session.observation()
        state = json.loads(raw)
        validate_state(state)
        self._state, self._raw = state, raw
        return state

    def _track(self, state, *, action=None, reply=None):
        """Progress consumes this validated transport snapshot without copying it."""
        return self._progress._observe_validated(state, action=action, reply=reply)

    def _result(self, accepted=None, execution=None, aid=None):
        # A second decode of the same transport bytes is an exact, isolated copy;
        # it is far cheaper than recursively copying the parsed public state.
        observation = json.loads(self._raw)
        return Transition(observation, observation["personalEnded"],
                          bool(self._reason and not observation["personalEnded"]), self._reason,
                          accepted=accepted, action_id=aid, execution=execution,
                          progress=self._progress.snapshot(self._state))

    def reset(self, *, seed, difficulty="ABYSS"):
        if difficulty != "ABYSS" or self._state is not None or self._closed:
            raise ValueError("Reset once per live Desperation decision session")
        self.session.reset(seed)
        self._track(self._observe())
        return self._result()

    def step(self, action):
        if self._closed or self._state is None or self._reason or self._state["personalEnded"]:
            raise RuntimeError("Reset required")
        # Keep the immediate action receipt distinct from automatic combat.
        intent = deepcopy(action)
        receipt = json.loads(self.session.step(json.dumps(intent, allow_nan=False), 0))
        accepted = receipt["accepted"]
        reply = {"accepted": accepted, "result": {"ok": accepted}}
        if not accepted:
            reply["result"]["error"] = receipt["rejection"]
        execution = self._track(self._observe(), action=intent, reply=reply)
        self._decisions += 1
        while receipt["budgetExhausted"]:
            if self._advances >= self.max_advances:
                # A half-finished battle has no action-value bootstrap state.
                # Fail the call instead of returning a spurious terminal/truncation.
                self.close(error=True)
                raise RuntimeError("Combat budget exhausted before a decision boundary")
            receipt = json.loads(self.session.advance(self.tick_budget))
            self._advances += 1
            if not receipt["budgetExhausted"]:
                self._track(self._observe())
        if not self._state["personalEnded"]:
            if self._decisions >= self.max_decisions:
                self._reason = "decision-budget"
            elif self._advances >= self.max_advances:
                self._reason = "advance-budget"
        return self._result(accepted, execution, action_id(intent))

    def close(self, *, error=False):
        self.session.close()
        self._closed = True

    def __exit__(self, *_):
        self.close()
