"""Bounded native teacher capture, behavior cloning and live on-policy PPO.

Torch is optional until a neural command is executed. CLI help launches neither
the engine nor training. Generated data stays in an explicitly new output folder.
"""
from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass, replace
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import random
import re
import time
import uuid

from .contracts import ACTION_SCHEMA, action_id, canonical, fingerprint
from .engine.session import ENGINE_REVISION
from .environment import SoloEnv
from .decision_session import DecisionSession
from .observation import SCHEMA as OBSERVATION_SCHEMA, validate_state
from .training import (Candidate, TrainingContract, Transition,
                       generalized_advantage_estimate, _read_json)

DATASET_SCHEMA = "arkguardrl/native-teacher-demonstrations-v2"
ROLLOUT_SCHEMA = "arkguardrl/native-on-policy-capture-v2"
MAX_BYTES = 128 * 1024 * 1024
MAX_ROWS = 4096
MAX_GAMES = 64
MAX_UPDATES = 10000


def _modules():
    from . import candidates, catalog, features, neural, rewards, teacher
    return candidates, catalog, features, neural, rewards, teacher


def _positive(value, name, upper=None, *, integer=False):
    if (type(value) not in ((int,) if integer else (int, float))
            or not math.isfinite(value) or value <= 0 or upper is not None and value > upper):
        raise ValueError(f"Invalid finite {name} budget")
    return value


def _utc():
    return datetime.now(timezone.utc).isoformat()


def _sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _write_json(path, value):
    with Path(path).open("xb") as stream:
        stream.write(canonical(value) + b"\n")


def _setup_torch(*, device="cpu", threads=1, seed=0):
    neural = _modules()[3]
    neural.device_name(device)
    if type(threads) is not int or threads not in (1, 2):
        raise ValueError("Use one or two Torch CPU threads")
    if type(seed) is not int or not 0 <= seed <= 0xffffffff:
        raise ValueError("Explicit uint32 training RNG seed required")
    neural.torch.set_num_threads(threads)
    neural.torch.manual_seed(seed)
    random.seed(seed)
    return neural.torch


def make_contract(public_catalog, difficulty):
    domains, _, features, _, rewards, _ = _modules()
    return TrainingContract(engine_revision=ENGINE_REVISION, data_sha256=public_catalog.data_sha256,
        observation_schema=OBSERVATION_SCHEMA, action_schema=ACTION_SCHEMA,
        candidate_schema=domains.CANDIDATE_SCHEMA, feature_schema=features.FEATURE_SCHEMA,
        feature_dim=features.OBS_DIM + features.ACTION_DIM, reward_schema=rewards.REWARD_SCHEMA,
        history_schema=domains.HISTORY_SCHEMA, difficulty=difficulty, requires_final_outcome=False, definition_sha256=public_catalog.definitions()[1])


def _validate_learning_contract(contract):
    _, _, _, neural, rewards, _ = _modules()
    neural.validate_contract(contract)
    if contract.reward_schema != rewards.REWARD_SCHEMA or contract.requires_final_outcome:
        raise ValueError("Explicit native progress/core immediate-personal-end reward required")


def _seeds(seeds):
    values = tuple(seeds)
    if (not 1 <= len(values) <= MAX_GAMES or len(set(values)) != len(values)
            or any(type(value) is not int or not 1 <= value <= 0xffffffff for value in values)):
        raise ValueError("Supply 1..64 distinct native uint32 game seeds greater than zero")
    return values


class RunBudgetExceeded(TimeoutError):
    """Our local wall budget expired, distinct from an engine-response timeout."""


class _DeadlineEnv(SoloEnv):
    """Check the same wall budget before every RPC, including automatic advances."""

    def __init__(self, *args, deadline, **kwargs):
        self.deadline = deadline
        super().__init__(*args, **kwargs)
        self._timeout_limit = self.session.timeout

    def _remaining(self):
        left = self.deadline - time.monotonic()
        if left <= 0:
            raise RunBudgetExceeded("Native run wall budget exhausted")
        self.session.timeout = min(self._timeout_limit, left)

    def __enter__(self):
        self._remaining()
        return super().__enter__()

    def _call(self, method, **kwargs):
        self._remaining()
        return super()._call(method, **kwargs)

    def __exit__(self, exc_type, *args):
        if exc_type is not None and issubclass(exc_type, RunBudgetExceeded):
            self._reason = "wall-budget"
            self.close(error=False)
        else:
            super().__exit__(exc_type, *args)


def _new_run(output_dir, contract, config):
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=False)
    _write_json(out / "contract.json", contract.to_dict())
    _write_json(out / "plan.json", {"created_at": _utc(), "contract_id": contract.contract_id,
                                    "config": config})
    return out


def _bounds(max_wall, max_decisions, max_advances):
    _positive(max_wall, "wall seconds", 86400)
    _positive(max_decisions, "episode decisions", MAX_ROWS, integer=True)
    _positive(max_advances, "episode automatic advances", 16384, integer=True)


def _expected_digest(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError("Independent expected complete engine data SHA256 required")


def _snapshot(decision, history, *, episode_id, index, reason, policy_revision=None,
              old_logp=None, value=None):
    """Own detached CPU tensors and an exact ordered native candidate domain."""
    _, _, _, neural, _, _ = _modules()
    tensors = neural.tensorize(decision)
    actions = json.loads(canonical(list(decision.actions)))
    record = {"observation": json.loads(canonical(decision.state)),
              "history": json.loads(canonical(history.snapshot(decision.state))),
              "actions": actions, "action_ids": [action_id(action) for action in actions],
              "obs": tensors[0], "candidates": tensors[1], "schedules": tensors[2],
              "experts": tensors[3], "index": index, "reason": reason,
              "episode_id": episode_id, "captured_at": _utc()}
    if policy_revision is not None:
        record.update(policy_revision=policy_revision, old_logp=old_logp, value=value)
    _validate_record(record, on_policy=policy_revision is not None)
    return record


def _record_bytes(row):
    # Conservative per-row bound includes tensors, duplicate native action IDs,
    # JSON metadata and serialization overhead; check it BEFORE retaining a row.
    torch = _modules()[3].torch
    tensor_bytes = sum(value.numel() * value.element_size()
                       for value in row.values() if isinstance(value, torch.Tensor))
    metadata = {key: value for key, value in row.items() if not isinstance(value, torch.Tensor)}
    return 2 * (tensor_bytes + len(canonical(metadata))) + 16384


def _validate_record(row, *, on_policy=False):
    _, _, _, neural, _, _ = _modules()
    required = {"observation", "history", "actions", "action_ids", "obs", "candidates",
                "schedules", "experts", "index", "reason", "episode_id", "captured_at"}
    if on_policy:
        required |= {"policy_revision", "old_logp", "value"}
    if not isinstance(row, dict) or set(row) != required:
        raise ValueError("Exact native captured-decision fields required")
    observation_bytes = validate_state(row["observation"])
    if row["observation"]["personalEnded"] or not row["observation"]["decisionRequired"]:
        raise ValueError("Captured policy row must precede an active native decision")
    tensors = (row["obs"], row["candidates"], row["schedules"], row["experts"])
    neural.validate_tensors(*tensors)
    if any(str(value.device) != "cpu" for value in tensors):
        raise ValueError("Store native capture tensors on CPU")
    actions = row["actions"]
    if (not isinstance(actions, list) or len(actions) != len(row["candidates"])
            or any(not action.get("t", "").startswith("g.") for action in actions)):
        raise ValueError("Native ordered candidate actions required")
    ids = [action_id(action) for action in actions]
    if row["action_ids"] != ids or len(set(ids)) != len(ids):
        raise ValueError("Captured native candidate identity mismatch or duplicate")
    if type(row["index"]) is not int or not 0 <= row["index"] < len(actions):
        raise ValueError("Captured selection must identify one original native candidate")
    for key in ("reason", "episode_id", "captured_at"):
        if not isinstance(row[key], str) or not row[key]:
            raise ValueError("Explicit native capture provenance required: " + key)
    canonical(row["history"])
    history = row["history"]
    if (not isinstance(history, dict) or history.get("schema") != _modules()[0].HISTORY_SCHEMA
            or history.get("progress", {}).get("observation_id") != hashlib.sha256(observation_bytes).hexdigest()):
        raise ValueError("Captured history/progress differs from its native observation")
    if on_policy:
        if type(row["policy_revision"]) is not int or row["policy_revision"] < 0:
            raise ValueError("Explicit on-policy revision required")
        if any(type(row[key]) not in (int, float) or not math.isfinite(row[key])
               for key in ("old_logp", "value")) or row["old_logp"] > 1e-5:
            raise ValueError("Finite sampled log probability/value required")

    return observation_bytes


def _episode_report(episode_id, seed, state, *, steps, rejected, status, reason, start,
                    total_reward=0.0, components=None, levels=None):
    return {"episode_id": episode_id, "seed": seed, "status": status, "reason": reason,
            "decisions": steps, "rejected": rejected, "elapsed_seconds": time.monotonic() - start,
            "round": state.get("round") if state else None,
            "clockMs": state.get("clockMs") if state else None,
            "revision": state.get("revision") if state else None,
            "personalEnded": state.get("personalEnded") if state else None,
            "gameEnded": state.get("gameEnded") if state else None,
            "outcome": state.get("outcome") if state else None,
            "final_level": state["own"]["shop"]["level"] if state else None,
            "level_events": levels or [], "return": total_reward,
            "reward_components": components or {}}


def _level_event(before, result):
    if result.metrics and result.metrics.get("levelIncrease", 0) > 0:
        return {"round": before["round"], "clockMs": before["clockMs"],
                "revision": before["revision"], **result.metrics}
    return None


def collect_teacher(engine_root, output_dir, *, seeds, difficulty, expected_data_sha256,
                    max_wall, max_decisions=256, max_advances=4096, max_records=MAX_ROWS,
                    teacher_policy=None, threads=1):
    """Capture newly executed rule-teacher decisions; never infer human provenance."""
    start = time.monotonic()
    domains, catalog_api, _, neural, _, teacher = _modules()
    _bounds(max_wall, max_decisions, max_advances)
    _expected_digest(expected_data_sha256)
    deadline = start + max_wall
    _positive(max_records, "captured records", MAX_ROWS, integer=True)
    seeds = _seeds(seeds)
    _setup_torch(threads=threads)
    policy_id = teacher.POLICY_ID if teacher_policy is None else teacher_policy
    if policy_id != teacher.POLICY_ID:
        raise ValueError("Select the explicit current native teacher identity")
    catalog = catalog_api.PublicCatalog(engine_root, expected_data_sha256=expected_data_sha256)
    contract = make_contract(catalog, difficulty)
    config = {"algorithm": "native-teacher-capture", "seeds": list(seeds), "difficulty": difficulty,
              "max_wall": max_wall, "max_decisions": max_decisions, "max_advances": max_advances,
              "max_records": max_records, "teacher_id": policy_id, "threads": threads}
    out = _new_run(output_dir, contract, config)
    rows, episodes = [], []
    used_bytes = 2 * len(canonical(catalog.definitions()[0])) + 16384
    if used_bytes > MAX_BYTES:
        raise ValueError("Static definition bank exceeds capture budget")
    stop_reason, failure = "games", None
    for game, seed in enumerate(seeds):
        if time.monotonic() >= deadline:
            stop_reason = "wall-budget"
            break
        episode_id = uuid.uuid4().hex
        state, steps, rejected, levels = None, 0, 0, []
        episode_start = time.monotonic()
        status, reason = "interrupted", "capture-budget"
        try:
            env = _DeadlineEnv(engine_root, deadline=deadline, expected_data_sha256=catalog.data_sha256,
                               trace_path=out / f"episode-{game:03d}.jsonl", source="rule-teacher",
                               policy_id=policy_id, max_decisions=max_decisions,
                               max_advances=max_advances)
            with DecisionSession(env, catalog) as session:
                history = session.history
                contract.assert_runtime(env.session.identity, difficulty=difficulty)
                result = session.reset(seed=seed, difficulty=difficulty)
                state = result.observation
                while not (result.terminated or result.truncated):
                    if len(rows) >= max_records or time.monotonic() >= deadline:
                        stop_reason = "record-budget" if len(rows) >= max_records else "wall-budget"
                        reason = stop_reason
                        break
                    decision = session.decision()
                    index, why = teacher.select_with_reason(decision, catalog, policy_id=policy_id)
                    row = _snapshot(decision, history, episode_id=episode_id, index=index, reason=why)
                    size = _record_bytes(row)
                    if used_bytes + size > MAX_BYTES:
                        stop_reason = reason = "capture-byte-budget"
                        break
                    before = state
                    result = session.step(decision.actions[index])
                    state, steps = result.observation, steps + 1
                    if type(result.accepted) is not bool:
                        raise RuntimeError("Teacher action was not executed")
                    rejected += int(not result.accepted)
                    level = _level_event(before, result)
                    if level:
                        levels.append(level)
                    # Duplicate feature/route aliases cannot teach distinct target IDs.
                    aliases = neural.feature_alias_count(row["obs"], row["candidates"],
                                                         row["schedules"], row["experts"], index)
                    included = bool(result.accepted and len(decision.actions) > 1 and aliases == 1)
                    env._record({"kind": "teacher-label", "episodeId": episode_id,
                                 "decisionIndex": steps - 1, "selectedActionId": row["action_ids"][index],
                                 "teacherReason": why, "capturedAt": row["captured_at"],
                                 "includedInBC": included, "candidateCount": len(decision.actions),
                                 "featureAliases": aliases})
                    if included:
                        rows.append(row)
                        used_bytes += size
                if result.terminated or result.truncated:
                    status = "personal-ended" if result.terminated else "truncated"
                    reason = result.reason
        except RunBudgetExceeded as exc:
            status, reason, stop_reason = "partial-budget", str(exc), "wall-budget"
            state = env._state if env._state is not None else state
        except Exception as exc:
            status, reason = "error", f"{type(exc).__name__}: {exc}"
            failure = reason
            stop_reason = "technical-error"
        report = _episode_report(episode_id, seed, state, steps=steps, rejected=rejected,
                                 status=status, reason=reason, start=episode_start, levels=levels)
        episodes.append(report)
        print(json.dumps({"teacher_episode": report}, allow_nan=False), flush=True)
        if failure or stop_reason != "games":
            break
    payload = {"schema": DATASET_SCHEMA, "contract": contract.to_dict(),
               "contract_id": contract.contract_id, "source": "rule-teacher", "teacher_id": policy_id,
               "episodes": episodes, "rows": rows,
               "created_at": _utc(), "stop_reason": stop_reason, "partial": failure is not None,
               "static_definitions": catalog.definitions()[0]}
    neural.save_payload(out / ("demonstrations-partial.pt" if failure else "demonstrations.pt"), payload)
    summary = {"records": len(rows), "episodes": episodes, "stop_reason": stop_reason,
               "error": failure, "elapsed_seconds": time.monotonic() - start,
               "cached_bytes_upper_bound": used_bytes, "contract_id": contract.contract_id}
    _write_json(out / "summary.json", summary)
    return summary


def load_demonstrations(path, expected_contract):
    """Strict safe native dataset reader. Partial technical-error data is not trainable."""
    _validate_learning_contract(expected_contract)
    neural = _modules()[3]
    payload = neural.load_payload(path)
    fields = {"schema", "contract", "contract_id", "source", "teacher_id",
              "episodes", "rows", "created_at", "stop_reason", "partial", "static_definitions"}
    if (not isinstance(payload, dict) or set(payload)-{"source_hashes"} != fields or payload["schema"] != DATASET_SCHEMA
            or payload["contract"] != expected_contract.to_dict()
            or payload["contract_id"] != expected_contract.contract_id
            or payload["source"] != "rule-teacher" or payload["teacher_id"] != _modules()[5].POLICY_ID
            or payload["partial"] is not False):
        raise ValueError("Native teacher dataset identity/provenance mismatch")
    neural.validate_static_definitions(payload["static_definitions"], expected_contract)
    rows, episodes = payload["rows"], payload["episodes"]
    if not isinstance(rows, list) or not 1 <= len(rows) <= MAX_ROWS:
        raise ValueError("Native teacher dataset must contain 1..4096 supervised decisions")
    if not isinstance(episodes, list) or not 1 <= len(episodes) <= MAX_GAMES:
        raise ValueError("Whole-episode provenance is required")
    ids = [row.get("episode_id") for row in episodes if isinstance(row, dict)]
    if len(ids) != len(episodes) or len(set(ids)) != len(ids):
        raise ValueError("Unique dataset episode provenance required")
    total_bytes = 0
    for row in rows:
        _validate_record(row)
        if row["episode_id"] not in ids or row["observation"]["public"]["difficulty"] != expected_contract.difficulty:
            raise ValueError("Dataset episode or difficulty mismatch")
        total_bytes += _record_bytes(row)
        if total_bytes > MAX_BYTES:
            raise ValueError("Dataset exceeds 128 MiB native cache budget")
    return payload


def train_bc(dataset_path, output_dir, *, expected_contract, steps, max_wall,
             batch_size=16, learning_rate=3e-4, device="cpu", threads=1, train_seed=0):
    """Scheduler-balanced supervised fitting, without critic-target fabrication.

    Critic-head weights are not optimized. The shared encoder still changes,
    therefore BC value predictions are not fitted or claimed to be calibrated.
    """
    start = time.monotonic()
    _positive(steps, "BC updates", MAX_UPDATES, integer=True)
    _positive(max_wall, "wall seconds", 86400)
    deadline = start + max_wall
    _positive(batch_size, "batch size", 256, integer=True)
    _positive(learning_rate, "learning rate", 1)
    torch = _setup_torch(device=device, threads=threads, seed=train_seed)
    neural = _modules()[3]
    dataset = load_demonstrations(dataset_path, expected_contract)
    config = {"algorithm": "native-behavior-cloning", "steps": steps, "max_wall": max_wall,
              "batch_size": batch_size, "learning_rate": learning_rate, "device": device,
              "threads": threads, "train_seed": train_seed, "dataset_sha256": _sha(dataset_path),
              "torch_version": str(torch.__version__),
              "sampling": "uniform observed schedules then uniform supervised rows",
              "validation": "none; no held-out generalization claim"}
    out = _new_run(output_dir, expected_contract, config)
    model = neural.NativeActorCritic(expected_contract, dataset["static_definitions"]).to(device)
    parameters = [value for key, value in model.named_parameters() if not key.startswith("critic.")]
    optimizer = torch.optim.Adam(parameters, lr=learning_rate)
    rows, buckets = dataset["rows"], {}
    for index, row in enumerate(rows):
        buckets.setdefault(int(row["schedules"][row["index"]]), []).append(index)
    groups = sorted(buckets)
    provenance = {**config, "teacher_id": dataset["teacher_id"], "episodes": dataset["episodes"],
                  "counts_by_schedule": {str(key): len(value) for key, value in buckets.items()}}
    neural.save_checkpoint(out / "initial.pt", model, algorithm="native-behavior-cloning",
                           updates=0, provenance=provenance)
    metrics, sampled = [], Counter()
    updates = 0
    for step in range(steps):
        if time.monotonic() >= deadline:
            break
        losses = []
        optimizer.zero_grad(set_to_none=True)
        for _ in range(batch_size):
            if time.monotonic() >= deadline:
                break
            group = groups[int(torch.randint(len(groups), ()))]
            row = rows[buckets[group][int(torch.randint(len(buckets[group]), ()))]]
            logits, _ = model(*(row[key].to(device) for key in ("obs", "candidates", "schedules", "experts")))
            loss = -logits[row["index"]]
            neural._finite(loss, "BC objective")
            (loss / batch_size).backward()
            losses.append(float(loss.detach()))
            sampled[group] += 1
        if len(losses) != batch_size:
            optimizer.zero_grad(set_to_none=True)
            break
        mean_loss = math.fsum(losses) / len(losses)
        norm = torch.nn.utils.clip_grad_norm_(parameters, 1.0, error_if_nonfinite=True)
        optimizer.step()
        updates += 1
        if updates == 1 or updates % 50 == 0 or updates == steps:
            metric = {"updates": updates, "loss": mean_loss, "gradient_norm": float(norm),
                      "elapsed_seconds": time.monotonic() - start}
            metrics.append(metric)
            print(json.dumps(metric, allow_nan=False), flush=True)
    summary = {**provenance, "updates": updates, "metrics": metrics,
               "sampled_counts_by_schedule": {str(key): value for key, value in sampled.items()},
               "critic_training": "head frozen; shared encoder changes; no fitted value targets",
               "stop_reason": "updates" if updates == steps else "wall-budget",
               "elapsed_seconds": time.monotonic() - start}
    neural.save_checkpoint(out / "checkpoint.pt", model, algorithm="native-behavior-cloning",
                           updates=updates, provenance=summary)
    _write_json(out / "summary.json", summary)
    return summary


@dataclass(frozen=True)
class _OnPolicyRow:
    capture: dict
    transition: Transition


def _bootstrap(model, session, device):
    """Value the actual successor, even if truncation leaves no player decision."""
    state = session.result.observation
    if state["personalEnded"]:
        return 0.0
    _, _, features, neural, _, _ = _modules()
    encoded = session.observation_features()
    with neural.torch.no_grad():
        obs = neural.torch.tensor(encoded, dtype=neural.torch.float32, device=device)
        return float(model.value(obs))


def _advantages(contract, rollout, lam):
    """Split exact episode segments, including natural and truncated reset boundaries."""
    advantages, returns, segment = [], [], []

    def finish():
        if segment:
            result = generalized_advantage_estimate(contract, segment, gae_lambda=lam)
            advantages.extend(result.advantages)
            returns.extend(result.returns)
            segment.clear()

    for row in rollout:
        transition = row.transition
        if segment and transition.episode_id != segment[-1].episode_id:
            finish()
        segment.append(transition)
        if transition.terminated or transition.truncated:
            finish()
    finish()
    return advantages, returns


def _ppo_update(model, optimizer, rollout, *, policy_revision, gae_lambda, clip,
                epochs, batch_size, entropy_coef, deadline, device):
    """Consume only a fresh live buffer owned by train_ppo, never diagnostic files."""
    neural = _modules()[3]
    torch = neural.torch
    if not rollout or time.monotonic() >= deadline:
        return {"updates": 0, "metrics": []}
    # Check EVERY old probability before the first optimizer mutation. The exact
    # captured candidates/routing are replayed, never rebuilt after environment step.
    for row in rollout:
        if time.monotonic() >= deadline:
            return {"updates": 0, "stop_reason": "wall-budget-before-update"}
        captured = row.capture
        _validate_record(captured, on_policy=True)
        if captured["policy_revision"] != policy_revision:
            raise ValueError("Mixed or stale policy revisions in live PPO buffer")
        if row.transition.candidate.action_id != captured["action_ids"][captured["index"]]:
            raise ValueError("Selected candidate identity changed before PPO update")
        with torch.no_grad():
            logits, value = model(*(captured[key].to(device)
                                   for key in ("obs", "candidates", "schedules", "experts")))
            actual = torch.distributions.Categorical(logits=logits).log_prob(
                torch.tensor(captured["index"], device=device))
        if (not math.isclose(float(actual), captured["old_logp"], rel_tol=1e-5, abs_tol=1e-5)
                or not math.isclose(float(value), captured["value"], rel_tol=1e-5, abs_tol=1e-5)):
            raise ValueError("PPO rollout was not sampled from this unchanged policy/domain")
    advantages, returns = _advantages(model.contract, rollout, gae_lambda)
    advantages = torch.tensor(advantages, dtype=torch.float32, device=device)
    returns = torch.tensor(returns, dtype=torch.float32, device=device)
    if len(advantages) > 1:
        advantages = (advantages - advantages.mean()) / advantages.std(unbiased=False).clamp_min(1e-8)
    metrics = []
    for _ in range(epochs):
        if time.monotonic() >= deadline:
            break
        order = torch.randperm(len(rollout)).tolist()
        for start in range(0, len(order), batch_size):
            if time.monotonic() >= deadline:
                break
            selected = order[start:start + batch_size]
            batch_metrics = []
            optimizer.zero_grad(set_to_none=True)
            for index in selected:
                if time.monotonic() >= deadline:
                    break
                captured = rollout[index].capture
                logits, value = model(*(captured[key].to(device)
                                       for key in ("obs", "candidates", "schedules", "experts")))
                distribution = torch.distributions.Categorical(logits=logits)
                new_logp = distribution.log_prob(torch.tensor(captured["index"], device=device))
                ratio = (new_logp - captured["old_logp"]).exp()
                policy_loss = -torch.minimum(ratio * advantages[index],
                                            ratio.clamp(1 - clip, 1 + clip) * advantages[index])
                value_loss = 0.5 * (value - returns[index]).square()
                entropy = model.regularized_entropy(logits, captured["schedules"].to(device))
                loss = policy_loss + 0.5 * value_loss - entropy_coef * entropy
                neural._finite(loss, "PPO objective")
                (loss / len(selected)).backward()
                batch_metrics.append({"loss": float(loss.detach()), "policy_loss": float(policy_loss.detach()),
                                      "value_loss": float(value_loss.detach()), "entropy": float(entropy.detach()),
                                      "approx_kl": float(((ratio - 1) - (new_logp - captured["old_logp"])).detach()),
                                      "clip_fraction": float(((ratio - 1).abs() > clip).float().detach())})
            if len(batch_metrics) != len(selected):
                optimizer.zero_grad(set_to_none=True)
                break
            norm = torch.nn.utils.clip_grad_norm_(model.parameters(), 0.5, error_if_nonfinite=True)
            optimizer.step()
            metric = {key: math.fsum(row[key] for row in batch_metrics) / len(batch_metrics)
                      for key in batch_metrics[0]}
            metrics.append({**metric, "gradient_norm": float(norm)})
    average = ({key: math.fsum(row[key] for row in metrics) / len(metrics) for key in metrics[0]}
               if metrics else {})
    return {"updates": len(metrics), "decisions": len(rollout), **average}


def train_ppo(engine_root, output_dir, *, seeds, difficulty, expected_data_sha256,
              steps, max_wall, max_decisions=256, max_advances=4096, rollout_steps=64,
              epochs=4, batch_size=16, learning_rate=3e-4, gae_lambda=0.95, clip=0.2,
              entropy_coef=0.01, device="cpu", threads=1, train_seed=0,
              initial_checkpoint=None, seed_repeats=1):
    """Actual stochastic on-policy PPO, optionally initialized from fresh native BC.

    All sampled candidates, routing, log probabilities and values are captured
    before the selected intent is executed. Each decision's reward includes every
    automatic engine advance up to the next decision or endpoint. No teacher is
    consulted by this path; saved diagnostic traces cannot be loaded into it.
    """
    start = time.monotonic()
    domains, catalog_api, _, neural, rewards, _ = _modules()
    _bounds(max_wall, max_decisions, max_advances)
    _expected_digest(expected_data_sha256)
    deadline = start + max_wall
    for value, name, bound in ((steps, "PPO decisions", MAX_ROWS),
                               (rollout_steps, "rollout decisions", 256),
                               (epochs, "PPO epochs", 16), (batch_size, "batch size", 256)):
        _positive(value, name, bound, integer=True)
    _positive(learning_rate, "learning rate", 1)
    if (type(gae_lambda) not in (int, float) or not 0 <= gae_lambda <= 1
            or type(clip) not in (int, float) or not 0 < clip < 1
            or type(entropy_coef) not in (int, float) or not 0 <= entropy_coef <= 1):
        raise ValueError("Finite bounded PPO lambda, clipping and entropy coefficient required")
    seeds = _seeds(seeds)
    _positive(seed_repeats, "seed repetitions", MAX_GAMES, integer=True)
    if len(seeds) * seed_repeats > MAX_GAMES:
        raise ValueError("At most 64 total PPO episodes including seed repetitions")
    episode_seeds = seeds * seed_repeats
    torch = _setup_torch(device=device, threads=threads, seed=train_seed)
    catalog = catalog_api.PublicCatalog(engine_root, expected_data_sha256=expected_data_sha256)
    contract = make_contract(catalog, difficulty)
    model = neural.NativeActorCritic(contract, catalog.definitions()[0]).to(device)
    if initial_checkpoint is not None:
        model, metadata = neural.load_checkpoint(initial_checkpoint, contract, device=device)
        if metadata["algorithm"] != "native-behavior-cloning":
            raise ValueError("Only explicit native BC warm start is supported; no PPO resume")
    model.train()
    config = {"algorithm": "native-ppo", "seeds": list(seeds), "difficulty": difficulty,
              "seed_repeats": seed_repeats, "episode_seeds": list(episode_seeds),
              "steps": steps, "max_wall": max_wall, "max_decisions": max_decisions,
              "max_advances": max_advances, "rollout_steps": rollout_steps,
              "epochs": epochs, "batch_size": batch_size, "learning_rate": learning_rate,
              "gae_lambda": gae_lambda, "clip": clip, "entropy_coef": entropy_coef,
              "device": device, "threads": threads, "train_seed": train_seed,
              "torch_version": str(torch.__version__),
              "gamma": 1.0, "discount_unit": "decision", "reward": rewards.metadata(),
              "initial_checkpoint_sha256": _sha(initial_checkpoint) if initial_checkpoint else None}
    out = _new_run(output_dir, contract, config)
    neural.save_payload(out / "static-definitions.pt", catalog.definitions()[0])
    optimizer = torch.optim.Adam(model.parameters(), lr=learning_rate)
    neural.save_checkpoint(out / "initial.pt", model, algorithm="native-ppo", updates=0, provenance=config)
    episodes, rollout, metrics = [], [], []
    total_steps = updates = revision = capture_bytes = batch_number = 0
    sampled_schedules, exposed_schedules = Counter(), Counter()
    stop_reason, failure = "games", None

    def flush():
        nonlocal updates, revision, batch_number
        if not rollout:
            return
        # This archive is evidence, NOT an accepted offline PPO training input.
        artifact = {"schema": ROLLOUT_SCHEMA, "contract": contract.to_dict(),
                    "contract_id": contract.contract_id, "source": "live-ppo-policy",
                    "static_definitions_sha256": contract.definition_sha256,
                    "policy_revision": revision,
                    "rows": [{"capture": row.capture, "transition": row.transition.to_dict()}
                             for row in rollout]}
        neural.save_payload(out / f"rollout-{batch_number:04d}.pt", artifact)
        batch_number += 1
        result = _ppo_update(model, optimizer, rollout, policy_revision=revision, gae_lambda=gae_lambda,
                             clip=clip, epochs=epochs, batch_size=batch_size,
                             entropy_coef=entropy_coef, deadline=deadline, device=device)
        updates += result["updates"]
        revision += result["updates"]
        metrics.append(result)
        print(json.dumps({"ppo_update": result, "decisions": total_steps}, allow_nan=False), flush=True)
        rollout.clear()

    for game, seed in enumerate(episode_seeds):
        if total_steps >= steps or time.monotonic() >= deadline:
            stop_reason = "steps" if total_steps >= steps else "wall-budget"
            break
        episode_id = uuid.uuid4().hex
        reward = rewards.NativeProgressCore()
        state, episode_steps, rejected, levels = None, 0, 0, []
        total_reward, components = 0.0, Counter()
        episode_start = time.monotonic()
        status, reason = "interrupted", "training-budget"
        try:
            env = _DeadlineEnv(engine_root, deadline=deadline, expected_data_sha256=catalog.data_sha256,
                               trace_path=out / f"episode-{game:03d}.jsonl", source="ppo-policy",
                               policy_id=f"{neural.ARCHITECTURE}:{contract.contract_id}",
                               max_decisions=max_decisions, max_advances=max_advances)
            with DecisionSession(env, catalog) as session:
                history = session.history
                contract.assert_runtime(env.session.identity, difficulty=difficulty)
                result = session.reset(seed=seed, difficulty=difficulty)
                state = result.observation
                if result.terminated or result.truncated:
                    raise RuntimeError("Native episode ended before its first policy decision")
                reward.reset(state)
                while not (result.terminated or result.truncated):
                    if total_steps >= steps or time.monotonic() >= deadline:
                        stop_reason = "steps" if total_steps >= steps else "wall-budget"
                        reason = stop_reason
                        break
                    decision = session.decision()
                    index, old_logp, value, _ = model.sample(decision)
                    captured = _snapshot(decision, history, episode_id=episode_id, index=index,
                                         reason="stochastic-native-policy", policy_revision=revision,
                                         old_logp=old_logp, value=value)
                    size = _record_bytes(captured)
                    if capture_bytes + size > MAX_BYTES:
                        stop_reason = reason = "capture-byte-budget"
                        break
                    # Match predecessor next_value to the exact value sampled at this
                    # successor. Tiny device differences cannot create a false GAE link.
                    if rollout and rollout[-1].transition.episode_id == episode_id:
                        previous = rollout[-1]
                        if not math.isclose(previous.transition.next_value, value, rel_tol=1e-5, abs_tol=1e-5):
                            raise ValueError("Policy/value changed while collecting a rollout")
                        rollout[-1] = replace(previous, transition=replace(previous.transition, next_value=value))
                    before = state
                    action = decision.actions[index]
                    candidate = Candidate.from_native(contract, before, action,
                        tuple(decision.observation_features) + tuple(decision.candidate_features[index]))
                    result = session.step(action)
                    if type(result.accepted) is not bool:
                        raise RuntimeError("Sampled PPO intent was not executed")
                    state = result.observation
                    reward_info = reward.step(before, state)
                    next_value = _bootstrap(model, session, device)
                    transition = Transition.from_step(contract, episode_id=episode_id,
                        decision_index=episode_steps, before_observation=before, candidate=candidate,
                        result=result, reward=reward_info["reward"], value=value, next_value=next_value)
                    rollout.append(_OnPolicyRow(captured, transition))
                    sampled_schedules[decision.schedule_ids[index]] += 1
                    exposed_schedules.update(set(decision.schedule_ids))
                    capture_bytes += size
                    total_steps += 1
                    episode_steps += 1
                    rejected += int(not result.accepted)
                    total_reward += reward_info["reward"]
                    components.update(reward_info["components"])
                    level = _level_event(before, result)
                    if level:
                        levels.append(level)
                    env._record({"kind": "policy-sample", "episodeId": episode_id,
                                 "decisionIndex": episode_steps - 1, "policyRevision": revision,
                                 "selectedActionId": candidate.action_id, "selectedIndex": index,
                                 "oldLogProbability": old_logp, "value": value, "nextValue": next_value,
                                 "reward": reward_info, "capturedAt": captured["captured_at"],
                                 "candidateCount": len(decision.actions)})
                    if len(rollout) >= rollout_steps:
                        flush()
                if result.terminated or result.truncated:
                    status = "personal-ended" if result.terminated else "truncated"
                    reason = result.reason
                elif rollout and rollout[-1].transition.episode_id == episode_id:
                    last = rollout[-1]
                    rollout[-1] = replace(last, transition=replace(last.transition, truncated=True,
                                                                  reason=reason))
        except RunBudgetExceeded as exc:
            status, reason, stop_reason = "partial-budget", str(exc), "wall-budget"
            state = env._state if env._state is not None else state
            if rollout and rollout[-1].transition.episode_id == episode_id:
                last = rollout[-1]
                if not last.transition.terminated and not last.transition.truncated:
                    rollout[-1] = replace(last, transition=replace(last.transition, truncated=True,
                                                                  reason="wall-budget-before-next-row"))
        except Exception as exc:
            status, reason = "error", f"{type(exc).__name__}: {exc}"
            failure, stop_reason = reason, "technical-error"
            # A failed action/advance has no trustworthy successor. Do not invent
            # loss rewards or zero bootstraps. Earlier valid rows end at their own
            # actual public successor; mark that boundary before any future reset.
            if rollout and rollout[-1].transition.episode_id == episode_id:
                last = rollout[-1]
                if not last.transition.terminated and not last.transition.truncated:
                    rollout[-1] = replace(last, transition=replace(last.transition, truncated=True,
                                                                  reason="technical-error-before-next-row"))
        report = _episode_report(episode_id, seed, state, steps=episode_steps, rejected=rejected,
                                 status=status, reason=reason, start=episode_start, levels=levels,
                                 total_reward=total_reward, components=dict(components))
        episodes.append(report)
        print(json.dumps({"ppo_episode": report}, allow_nan=False), flush=True)
        if failure or stop_reason != "games":
            break
    # A failed PPO update may have mutated parameters partway through. Preserve
    # diagnostics but never retry that old buffer against an uncertain revision.
    if rollout and failure is None:
        flush()
    summary = {**config, "actual_steps": total_steps, "optimizer_updates": updates,
               "episodes": episodes, "metrics": metrics, "stop_reason": stop_reason,
               "error": failure, "elapsed_seconds": time.monotonic() - start,
               "capture_bytes_upper_bound": capture_bytes, "contract_id": contract.contract_id}
    summary.update(sampled_counts_by_schedule={str(key): value for key, value in sampled_schedules.items()},
                   exposed_decisions_by_schedule={str(key): value for key, value in exposed_schedules.items()})
    if failure is None:
        neural.save_checkpoint(out / "checkpoint.pt", model, algorithm="native-ppo",
                               updates=updates, provenance=summary)
    _write_json(out / "summary.json", summary)
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    collect = commands.add_parser("collect-teacher", help="Collect new native rule-teacher BC demonstrations")
    bc = commands.add_parser("train-bc", help="Fit strict native teacher data with scheduler-balanced BC")
    ppo = commands.add_parser("train-ppo", help="Sample and update the actual native policy in a bounded run")
    inspect = commands.add_parser("inspect-checkpoint", help="Validate native weights against an independent contract")
    for command in (collect, bc, ppo):
        command.add_argument("--out", required=True, help="New nonexisting output directory")
        command.add_argument("--max-wall", type=float, required=True, help="Explicit finite wall budget, seconds")
        command.add_argument("--threads", type=int, choices=(1, 2), default=1)
    for command in (collect, ppo):
        command.add_argument("--engine-root", required=True)
        command.add_argument("--expected-data-sha256", required=True, help="Independently verified complete data digest")
        command.add_argument("--difficulty", required=True)
        command.add_argument("--first-seed", type=int, required=True)
        command.add_argument("--games", type=int, required=True, help="Maximum distinct games, 1..64")
        command.add_argument("--max-decisions", type=int, default=256)
        command.add_argument("--max-advances", type=int, default=4096)
    collect.add_argument("--max-records", type=int, default=MAX_ROWS)
    for command in (bc, ppo):
        command.add_argument("--steps", type=int, required=True, help="BC updates or PPO sampled decisions")
        command.add_argument("--batch-size", type=int, default=16)
        command.add_argument("--learning-rate", type=float, default=3e-4)
        command.add_argument("--device", choices=("cpu", "mps"), default="cpu")
        command.add_argument("--train-seed", type=int, default=0)
    bc.add_argument("dataset")
    bc.add_argument("--contract", required=True, help="Independent expected native contract JSON")
    ppo.add_argument("--rollout-steps", type=int, default=64)
    ppo.add_argument("--seed-repeats", type=int, default=1,
                     help="Repeat the seed list with fresh sessions; at most 64 total episodes")
    ppo.add_argument("--epochs", type=int, default=4)
    ppo.add_argument("--gae-lambda", type=float, default=0.95)
    ppo.add_argument("--clip", type=float, default=0.2)
    ppo.add_argument("--entropy-coef", type=float, default=0.01)
    ppo.add_argument("--initial-checkpoint", help="Explicit new native BC checkpoint only")
    inspect.add_argument("checkpoint")
    inspect.add_argument("--contract", required=True)
    args = parser.parse_args()
    if args.command == "inspect-checkpoint":
        contract = TrainingContract.from_dict(_read_json(args.contract))
        _validate_learning_contract(contract)
        model, metadata = _modules()[3].load_checkpoint(args.checkpoint, contract)
        print(json.dumps({"contract_id": model.contract.contract_id, "metadata": metadata}, allow_nan=False))
        return
    common = {"max_wall": args.max_wall, "threads": args.threads}
    if args.command == "train-bc":
        result = train_bc(args.dataset, args.out,
            expected_contract=TrainingContract.from_dict(_read_json(args.contract)),
            steps=args.steps, batch_size=args.batch_size, learning_rate=args.learning_rate,
            device=args.device, train_seed=args.train_seed, **common)
    else:
        _positive(args.games, "games", MAX_GAMES, integer=True)
        game = {"seeds": range(args.first_seed, args.first_seed + args.games),
                "difficulty": args.difficulty, "expected_data_sha256": args.expected_data_sha256,
                "max_decisions": args.max_decisions, "max_advances": args.max_advances, **common}
        if args.command == "collect-teacher":
            result = collect_teacher(args.engine_root, args.out, max_records=args.max_records, **game)
        else:
            result = train_ppo(args.engine_root, args.out, steps=args.steps, rollout_steps=args.rollout_steps,
                epochs=args.epochs, batch_size=args.batch_size, learning_rate=args.learning_rate,
                gae_lambda=args.gae_lambda, clip=args.clip, entropy_coef=args.entropy_coef,
                device=args.device, train_seed=args.train_seed, initial_checkpoint=args.initial_checkpoint,
                seed_repeats=args.seed_repeats, **game)
    print(json.dumps(result, allow_nan=False), flush=True)
    if result.get("error"):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
