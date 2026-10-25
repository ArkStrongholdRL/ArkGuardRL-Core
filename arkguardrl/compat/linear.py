"""Historical JSON linear training; not part of the neural learning path."""
import argparse
import math
import json
from pathlib import Path
from ..contracts import canonical, action_id, decision_id
from ..training import (TrainingContract, Candidate, _candidates, _observation, _vector,
    _number, _text, _read_json, _json_loads, MAX_TRAINING_STEPS, MAX_DATASET_ROWS,
    MAX_JSON_BYTES)

POLICY_SCHEMA = "arkguardrl/linear-categorical-bc-v1"
CHECKPOINT_SCHEMA = "arkguardrl/json-linear-checkpoint-v1"
DATASET_SCHEMA = "arkguardrl/caller-supervised-dataset-v1"


class LinearCategoricalPolicy:
    """Small supervised softmax scorer of externally supplied candidate features.

    There is no value head, learned feature extractor, PPO update, random policy
    sampling, legality oracle or claim that the caller supplied every action.
    """

    def __init__(self, contract, weights=None):
        if not isinstance(contract, TrainingContract):
            raise ValueError("An explicit TrainingContract is required")
        self._contract = contract
        self._weights = _vector([0.0] * contract.feature_dim if weights is None else weights,
                                contract.feature_dim, "weights")
        self._updates = 0

    @property
    def contract(self):
        return self._contract

    @property
    def weights(self):
        return self._weights

    @property
    def updates(self):
        return self._updates

    def _distribution(self, observation, candidates):
        rows = _candidates(self.contract, observation, candidates)
        try:
            scores = tuple(_number(math.fsum(w * x for w, x in zip(self.weights, row.features)),
                                   "policy score") for row in rows)
            peak = max(scores)
            shifted = tuple(_number(score - peak, "shifted score") for score in scores)
            exp = tuple(math.exp(score) for score in shifted)
            total = math.fsum(exp)
            probabilities = tuple(value / total for value in exp)
        except (OverflowError, ArithmeticError) as exc:
            raise ValueError("Non-finite policy computation; rescale supplied features") from exc
        return rows, shifted, total, probabilities

    def probabilities(self, observation, candidates):
        return self._distribution(observation, candidates)[3]

    def greedy_action(self, observation, candidates):
        rows, _, _, probabilities = self._distribution(observation, candidates)
        return rows[max(range(len(rows)), key=probabilities.__getitem__)].action

    def supervised_update(self, observation, candidates, target_action, *, learning_rate):
        """One cross-entropy gradient step; target is an explicit native intent.

        An absent label, stale domain or non-finite update fails before mutation.
        This off-policy supervised update does not create PPO rollout evidence.
        """
        rate = _number(learning_rate, "learning_rate")
        if rate <= 0:
            raise ValueError("Positive learning_rate required")
        rows, shifted, total, probabilities = self._distribution(observation, candidates)
        target_id = action_id(target_action)
        indexes = [index for index, row in enumerate(rows) if row.action_id == target_id]
        if len(indexes) != 1:
            raise ValueError("Supervised native label must identify one supplied candidate")
        target = indexes[0]
        loss = _number(math.log(total) - shifted[target], "cross-entropy loss")
        try:
            gradient = tuple(_number(math.fsum(probabilities[i] * row.features[j]
                                               for i, row in enumerate(rows))
                                     - rows[target].features[j], "gradient")
                             for j in range(self.contract.feature_dim))
            weights = tuple(_number(weight - rate * grad, "updated weight")
                            for weight, grad in zip(self.weights, gradient))
        except (OverflowError, ArithmeticError) as exc:
            raise ValueError("Non-finite supervised update; weights unchanged") from exc
        self._weights = weights
        self._updates += 1
        return loss

    def save_checkpoint(self, path):
        """Create a new JSON checkpoint exclusively; never overwrite an old run."""
        payload = {"schema": CHECKPOINT_SCHEMA, "policy_schema": POLICY_SCHEMA,
                   "contract": self.contract.to_dict(), "contract_id": self.contract.contract_id,
                   "weights": list(self.weights), "updates": self.updates}
        with Path(path).open("xb") as stream:
            stream.write(canonical(payload) + b"\n")


def load_checkpoint(path, expected_contract):
    """Require caller-owned expected metadata before accepting any model weights."""
    if not isinstance(expected_contract, TrainingContract):
        raise ValueError("An independent expected TrainingContract is required")
    payload = _read_json(path)
    required = {"schema", "policy_schema", "contract", "contract_id", "weights", "updates"}
    if not isinstance(payload, dict) or set(payload) != required:
        raise ValueError("Exact JSON checkpoint metadata required; no legacy conversion")
    if payload["schema"] != CHECKPOINT_SCHEMA or payload["policy_schema"] != POLICY_SCHEMA:
        raise ValueError("Unsupported checkpoint/policy schema")
    actual = TrainingContract.from_dict(payload["contract"])
    if (actual.to_dict() != expected_contract.to_dict()
            or payload["contract_id"] != expected_contract.contract_id):
        raise ValueError("Checkpoint training contract mismatch")
    if type(payload["updates"]) is not int or payload["updates"] < 0:
        raise ValueError("Invalid supervised update count")
    policy = LinearCategoricalPolicy(expected_contract, payload["weights"])
    policy._updates = payload["updates"]
    return policy



def train_json(input_path, output_path, *, steps, learning_rate):
    """Bounded deterministic supervised fitting, not an engine/teacher rollout.

    Validate every supplied row before updating. Cycle input rows in declared
    order for exactly steps updates; source is only a caller declaration. Return
    a small diagnostic summary and write the JSON checkpoint exclusively.
    """
    if type(steps) is not int or not 1 <= steps <= MAX_TRAINING_STEPS:
        raise ValueError(f"steps must be an integer in [1, {MAX_TRAINING_STEPS}]")
    rate = _number(learning_rate, "learning_rate")
    if rate <= 0:
        raise ValueError("Positive learning_rate required")
    payload = _read_json(input_path)
    if not isinstance(payload, dict) or set(payload) != {"schema", "source", "contract", "rows"}:
        raise ValueError("Exact caller-supervised dataset schema required")
    if payload["schema"] != DATASET_SCHEMA or payload["source"] != "caller-supplied":
        raise ValueError("Dataset must explicitly declare caller-supplied supervision")
    contract = TrainingContract.from_dict(payload["contract"])
    raw_rows = payload["rows"]
    if not isinstance(raw_rows, list) or not 1 <= len(raw_rows) <= MAX_DATASET_ROWS:
        raise ValueError(f"Dataset must contain 1..{MAX_DATASET_ROWS} rows")
    policy = LinearCategoricalPolicy(contract)
    rows = []
    for raw in raw_rows:
        if not isinstance(raw, dict) or set(raw) != {"observation", "candidates", "target_action"}:
            raise ValueError("Each row requires exactly observation, candidates, target_action")
        candidates = raw["candidates"]
        if not isinstance(candidates, list) or not 1 <= len(candidates) <= MAX_CANDIDATES:
            raise ValueError(f"Each row must contain 1..{MAX_CANDIDATES} supplied candidates")
        if any(not isinstance(row, dict) or set(row) != {"action", "features"} for row in candidates):
            raise ValueError("Each candidate requires exactly action and features")
        candidates = tuple(Candidate.from_native(contract, raw["observation"], row["action"], row["features"])
                           for row in candidates)
        _candidates(contract, raw["observation"], candidates)
        label_id = action_id(raw["target_action"])
        if sum(row.action_id == label_id for row in candidates) != 1:
            raise ValueError("Each label must identify exactly one supplied native candidate")
        rows.append((raw["observation"], candidates, raw["target_action"]))
    first_loss = last_loss = None
    for index in range(steps):
        observation, candidates, target = rows[index % len(rows)]
        last_loss = policy.supervised_update(observation, candidates, target, learning_rate=rate)
        if first_loss is None:
            first_loss = last_loss
    policy.save_checkpoint(output_path)
    return {"schema": CHECKPOINT_SCHEMA, "contract_id": contract.contract_id,
            "source": "caller-supplied", "rows": len(rows), "supervised_updates": policy.updates,
            "first_preupdate_loss": first_loss, "last_preupdate_loss": last_loss}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    inspect = subcommands.add_parser("inspect-checkpoint", help="Validate strict JSON against an expected contract")
    inspect.add_argument("checkpoint")
    inspect.add_argument("--contract", required=True, help="Independent expected contract JSON, not model metadata")
    train = subcommands.add_parser("train-json", help="Bounded supervised updates on caller-supplied candidates")
    train.add_argument("input", help="Strict caller-supervised dataset JSON (maximum 8 MiB)")
    train.add_argument("--output", required=True, help="New, nonexisting JSON checkpoint path")
    train.add_argument("--steps", type=int, required=True, help="Explicit update count, 1..10000")
    train.add_argument("--learning-rate", type=float, required=True)
    args = parser.parse_args()
    if args.command == "train-json":
        print(json.dumps(train_json(args.input, args.output, steps=args.steps,
                                    learning_rate=args.learning_rate), sort_keys=True, allow_nan=False))
        return
    contract = TrainingContract.from_dict(_read_json(args.contract))
    policy = load_checkpoint(args.checkpoint, contract)
    print(json.dumps({"policy_schema": POLICY_SCHEMA, "contract_id": contract.contract_id,
                      "feature_dim": contract.feature_dim, "supervised_updates": policy.updates}, sort_keys=True))


if __name__ == "__main__":
    main()
