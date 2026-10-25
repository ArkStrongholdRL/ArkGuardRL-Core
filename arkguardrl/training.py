"""Native contracts, executed transitions and decision-time GAE.

Historical linear JSON training lives in compat.linear.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass, fields
import json
import math
from pathlib import Path
import re
from typing import ClassVar

from .contracts import ACTION_SCHEMA, action_id, canonical, decision_id, fingerprint
from .engine.session import ENGINE_REVISION
from .environment import ENV_SCHEMA, Transition as EnvironmentTransition
from .observation import SCHEMA as OBSERVATION_SCHEMA, validate_state

CONTRACT_SCHEMA = "arkguardrl/native-training-contract-v2"
TRANSITION_SCHEMA = "arkguardrl/native-training-transition-v1"
GAE_SCHEMA = "arkguardrl/decision-gae-v1"
MAX_JSON_BYTES = 8 * 1024 * 1024
MAX_DATASET_ROWS = 4096
MAX_CANDIDATES = 4096
MAX_FEATURE_DIM = 131072  # Bounded structured wire tables; not a dense neural layer.
MAX_TRAINING_STEPS = 10000


def _number(value, name):
    if type(value) not in (int, float):
        raise ValueError(f"{name} must be a finite number, not a boolean")
    try:
        result = float(value)
    except (OverflowError, ValueError) as exc:
        raise ValueError(f"{name} must be finite") from exc
    if not math.isfinite(result):
        raise ValueError(f"{name} must be finite")
    return result


def _text(value, name):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"Explicit {name} required")


def _digest(value, name):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError(f"{name} must be a lowercase SHA256")


def _vector(values, size, name):
    if not isinstance(values, (list, tuple)) or len(values) != size:
        raise ValueError(f"{name} must have exactly {size} entries")
    return tuple(_number(value, name) for value in values)


def _json_loads(text):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"Duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"Non-finite JSON number: {value}")

    value = json.loads(text, object_pairs_hook=unique, parse_constant=invalid_constant)
    canonical(value)  # Also rejects overflowed JSON exponents such as 1e10000.
    return value


def _read_json(path):
    with Path(path).open("rb") as stream:
        data = stream.read(MAX_JSON_BYTES + 1)
    if len(data) > MAX_JSON_BYTES:
        raise ValueError(f"JSON input exceeds {MAX_JSON_BYTES} bytes")
    return _json_loads(data.decode("utf-8"))


@dataclass(frozen=True)
class TrainingContract:
    engine_revision: str
    data_sha256: str
    observation_schema: str
    action_schema: str
    candidate_schema: str
    feature_schema: str
    feature_dim: int
    reward_schema: str
    history_schema: str
    difficulty: str
    requires_final_outcome: bool
    definition_sha256: str
    gamma: float = 1.0
    discount_unit: str = "decision"
    app_version: str = "0.3.1"
    environment_schema: str = ENV_SCHEMA
    schema: str = CONTRACT_SCHEMA

    def __post_init__(self):
        _digest(self.definition_sha256, "definition_sha256")
        for name in ("candidate_schema", "feature_schema", "reward_schema",
                     "history_schema", "difficulty"):
            _text(getattr(self, name), name)
        _digest(self.data_sha256, "data_sha256")
        required = {"engine_revision": ENGINE_REVISION, "app_version": "0.3.1",
                    "observation_schema": OBSERVATION_SCHEMA, "action_schema": ACTION_SCHEMA,
                    "environment_schema": ENV_SCHEMA, "schema": CONTRACT_SCHEMA,
                    "discount_unit": "decision"}
        if any(getattr(self, name) != value for name, value in required.items()):
            raise ValueError("Unsupported native training identity or discount unit")
        if type(self.feature_dim) is not int or not 1 <= self.feature_dim <= MAX_FEATURE_DIM:
            raise ValueError(f"Integer feature_dim in [1, {MAX_FEATURE_DIM}] required")
        if type(self.requires_final_outcome) is not bool:
            raise ValueError("Explicit boolean requires_final_outcome required")
        gamma = _number(self.gamma, "gamma")
        if gamma != 1.0:
            raise ValueError("This native decision contract requires gamma=1")
        object.__setattr__(self, "gamma", gamma)

    @property
    def contract_id(self):
        return fingerprint(self.to_dict())

    def to_dict(self):
        return asdict(self)

    @classmethod
    def from_dict(cls, value):
        if not isinstance(value, dict) or set(value) != {field.name for field in fields(cls)}:
            raise ValueError("Exact training-contract metadata required; no legacy conversion")
        return cls(**value)

    def assert_runtime(self, identity, *, difficulty):
        expected = {"engineRevision": self.engine_revision, "appVersion": self.app_version,
                    "dataSha256": self.data_sha256, "schema": self.observation_schema}
        if not isinstance(identity, dict) or any(identity.get(k) != v for k, v in expected.items()):
            raise ValueError("Runtime engine/data/observation identity mismatch")
        if difficulty != self.difficulty:
            raise ValueError("Runtime difficulty mismatch")


def _observation(contract, observation, *, decision=False):
    validate_state(observation)
    if observation["public"].get("difficulty") != contract.difficulty:
        raise ValueError("Observation difficulty differs from training contract")
    if decision and (observation["personalEnded"] or not observation["decisionRequired"]):
        raise ValueError("An active native player decision is required")


@dataclass(frozen=True)
class Candidate:
    """Caller-declared candidate, not evidence of legality or domain completeness."""

    contract_id: str
    decision_id: str
    observation_id: str
    action_json: str
    features: tuple[float, ...]

    def __post_init__(self):
        for name in ("contract_id", "decision_id", "observation_id"):
            _digest(getattr(self, name), name)
        action = _json_loads(self.action_json)
        action_id(action)
        if not action["t"].startswith("g."):
            raise ValueError("A native g.* player intent is required")
        object.__setattr__(self, "action_json", canonical(action).decode("utf-8"))
        if not isinstance(self.features, (list, tuple)) or not self.features:
            raise ValueError("Nonempty supplied joint state/action features required")
        object.__setattr__(self, "features", _vector(self.features, len(self.features), "features"))

    @classmethod
    def from_native(cls, contract, observation, action, features):
        _observation(contract, observation, decision=True)
        return cls(contract.contract_id, decision_id(observation), fingerprint(observation),
                   canonical(action).decode("utf-8"),
                   _vector(features, contract.feature_dim, "features"))

    @property
    def action(self):
        return _json_loads(self.action_json)

    @property
    def action_id(self):
        return action_id(self.action)


def _candidates(contract, observation, candidates):
    _observation(contract, observation, decision=True)
    rows = tuple(candidates)
    if not rows:
        raise ValueError("An explicit nonempty candidate domain is required")
    context, snapshot = decision_id(observation), fingerprint(observation)
    seen = set()
    for row in rows:
        if not isinstance(row, Candidate) or row.contract_id != contract.contract_id:
            raise ValueError("Candidate training-contract mismatch")
        if row.decision_id != context or row.observation_id != snapshot:
            raise ValueError("Stale or mixed candidate observation context")
        _vector(row.features, contract.feature_dim, "candidate features")
        if row.action_id in seen:
            raise ValueError("Duplicate native action identity in candidate domain")
        seen.add(row.action_id)
    return rows


@dataclass(frozen=True)
class Transition:
    schema: ClassVar[str] = TRANSITION_SCHEMA
    contract_id: str
    episode_id: str
    decision_index: int
    candidate: Candidate
    next_observation_id: str
    reward: float
    value: float
    next_value: float
    terminated: bool
    truncated: bool
    accepted: bool
    reason: str | None
    final_outcome_json: str | None

    def __post_init__(self):
        _digest(self.contract_id, "contract_id")
        _digest(self.next_observation_id, "next_observation_id")
        _text(self.episode_id, "episode_id")
        if type(self.decision_index) is not int or self.decision_index < 0:
            raise ValueError("Nonnegative integer decision_index required")
        if not isinstance(self.candidate, Candidate) or self.candidate.contract_id != self.contract_id:
            raise ValueError("Transition candidate contract mismatch")
        for name in ("reward", "value", "next_value"):
            object.__setattr__(self, name, _number(getattr(self, name), name))
        if any(type(getattr(self, name)) is not bool for name in ("terminated", "truncated", "accepted")):
            raise ValueError("Explicit boolean endpoint and acceptance flags required")
        if self.terminated and self.truncated:
            raise ValueError("Personal termination and truncation cannot both be true")
        if self.terminated and self.next_value != 0:
            raise ValueError("Natural personal termination requires explicit zero next_value")
        if self.truncated:
            _text(self.reason, "truncation reason")
        elif self.reason is not None:
            raise ValueError("Only a truncation may carry a reason")
        if self.final_outcome_json is not None:
            outcome = _json_loads(self.final_outcome_json)
            if not self.terminated or not isinstance(outcome, dict) or not outcome:
                raise ValueError("A nonempty final game outcome belongs only to a personal terminal")
            object.__setattr__(self, "final_outcome_json", canonical(outcome).decode("utf-8"))

    def to_dict(self):
        return {"schema": self.schema, **asdict(self)}

    @classmethod
    def from_step(cls, contract, *, episode_id, decision_index, before_observation,
                  candidate, result, reward, value, next_value, final_observation=None):
        """Bind one executed SoloEnv.step, including its auto-advanced wait interval.

        Compute any settlement-dependent terminal reward externally after calling
        finish_settlement(), then pass its completed observation here. Settlement
        is not an additional policy decision or an extra discounted transition.
        Callers must supply settlement from the same live episode: public player
        IDs/revisions alone cannot attest cross-session episode identity.
        """
        _candidates(contract, before_observation, (candidate,))
        if not isinstance(result, EnvironmentTransition) or type(result.accepted) is not bool:
            raise ValueError("An executed SoloEnv.step result is required, not reset or a no-op")
        if result.action_id != candidate.action_id:
            raise ValueError("Executed native action differs from the selected candidate")
        from .feedback import EXECUTION_SCHEMA, PROGRESS_SCHEMA
        if (not isinstance(result.execution, dict) or result.execution.get("schema") != EXECUTION_SCHEMA
                or result.execution.get("action_id") != candidate.action_id
                or result.execution.get("before_id") != fingerprint(before_observation)
                or result.execution.get("accepted") != result.accepted
                or not isinstance(result.progress, dict) or result.progress.get("schema") != PROGRESS_SCHEMA
                or result.progress.get("last_execution") != result.execution
                or result.progress.get("observation_id") != fingerprint(result.observation)):
            raise ValueError("Executed transition lacks its exact public feedback/progress binding")
        after = result.observation
        _observation(contract, after)
        if (result.terminated != after["personalEnded"]
                or after["revision"] <= before_observation["revision"]
                or after["own"]["playerId"] != before_observation["own"]["playerId"]):
            raise ValueError("Inconsistent native transition endpoint or progression")
        outcome = after.get("outcome") if result.terminated and after["gameEnded"] else None
        if final_observation is not None:
            _observation(contract, final_observation)
            if (not result.terminated or not final_observation["gameEnded"]
                    or final_observation["revision"] < after["revision"]
                    or final_observation["own"]["playerId"] != after["own"]["playerId"]):
                raise ValueError("A completed same-player terminal settlement is required")
            outcome = final_observation.get("outcome")
        if contract.requires_final_outcome and result.terminated and not outcome:
            raise ValueError("Settlement-dependent terminal reward requires the final game outcome")
        return cls(contract.contract_id, episode_id, decision_index, candidate, fingerprint(after),
                   reward, value, next_value, result.terminated, result.truncated, result.accepted,
                   result.reason, canonical(outcome).decode("utf-8") if outcome is not None else None)


@dataclass(frozen=True)
class AdvantageResult:
    schema: ClassVar[str] = GAE_SCHEMA
    contract_id: str
    gae_lambda: float
    advantages: tuple[float, ...]
    returns: tuple[float, ...]

    def to_dict(self):
        return {"schema": self.schema, **asdict(self)}


def generalized_advantage_estimate(contract, transitions, *, gae_lambda):
    """GAE for one contiguous episode segment; no implicit reward/value inference.

    Natural personal terminals have zero bootstrap. Truncations and an open
    rollout tail use the caller's finite next_value. Neither crosses into a new
    episode. Gamma applies once per player decision, not per engine callback.
    """
    lam = _number(gae_lambda, "gae_lambda")
    if not 0 <= lam <= 1:
        raise ValueError("gae_lambda must be in [0, 1]")
    rows = tuple(transitions)
    if not rows:
        raise ValueError("A nonempty contiguous transition segment is required")
    for index, row in enumerate(rows):
        if not isinstance(row, Transition) or row.contract_id != contract.contract_id:
            raise ValueError("Transition training-contract mismatch")
        _vector(row.candidate.features, contract.feature_dim, "candidate features")
        if contract.requires_final_outcome and row.terminated and row.final_outcome_json is None:
            raise ValueError("Settlement-dependent terminal reward has no final outcome")
        if index:
            previous = rows[index - 1]
            if (previous.terminated or previous.truncated or row.episode_id != previous.episode_id
                    or row.decision_index != previous.decision_index + 1
                    or previous.next_observation_id != row.candidate.observation_id
                    or previous.next_value != row.value):
                raise ValueError("Disconnected rollout, crossed endpoint or inconsistent value estimates")
    advantages = [0.0] * len(rows)
    carry = 0.0
    for index in range(len(rows) - 1, -1, -1):
        row = rows[index]
        delta = _number(row.reward + contract.gamma * row.next_value - row.value, "TD residual")
        continuation = 0.0 if row.terminated or row.truncated else carry
        carry = _number(delta + contract.gamma * lam * continuation, "advantage")
        advantages[index] = carry
    returns = tuple(_number(advantage + row.value, "return")
                    for advantage, row in zip(advantages, rows))
    return AdvantageResult(contract.contract_id, lam, tuple(advantages), returns)
