"""Native shared-encoder purchase/placement policy; optional, separately installed Torch.

There is no import of the retained research framework or old checkpoint loader.
Candidate routing is explicit native metadata, not an engine legality oracle.
"""
from __future__ import annotations

import hashlib
import math
from pathlib import Path

try:
    import torch
    from torch import nn
except ImportError as exc:
    raise ImportError("Native neural policies require separately installed PyTorch") from exc

from . import candidates as domains, features, structured as layout
from .contracts import canonical, fingerprint
from .training import TrainingContract
from .routing import ROUTING_SCHEMA

ARCHITECTURE = "arkguardrl/native-entity64-grid32-mechanism32-static-bank/v7"
ENTROPY = "arkguardrl/native-intent-plus-mean-normalized-conditional/v2"
SELECTOR = "arkguardrl/native-intent-first-target-argmax/v2"
CHECKPOINT_SCHEMA = "arkguardrl/native-torch-checkpoint-v2"
MAX_ARTIFACT_BYTES = 128 * 1024 * 1024


def identities():
    return {"architecture": ARCHITECTURE, "entropy": ENTROPY, "selector": SELECTOR,
            "observation_dim": features.OBS_DIM, "action_dim": features.ACTION_DIM,
            "schedules": list(features.SCHEDULES), "experts": ["purchase", "placement"],
            "feature_schema": features.FEATURE_SCHEMA, "routing_schema": ROUTING_SCHEMA,
            "semantic_schema": layout.SEMANTIC_SCHEMA,
            "candidate_schema": domains.CANDIDATE_SCHEMA,
            "history_schema": domains.HISTORY_SCHEMA}


def validate_contract(contract):
    if not isinstance(contract, TrainingContract):
        raise ValueError("An independent expected native TrainingContract is required")
    expected = {"feature_schema": features.FEATURE_SCHEMA,
                "candidate_schema": domains.CANDIDATE_SCHEMA,
                "history_schema": domains.HISTORY_SCHEMA,
                "feature_dim": features.OBS_DIM + features.ACTION_DIM}
    if any(getattr(contract, key) != value for key, value in expected.items()):
        raise ValueError("Native neural feature/candidate/history contract mismatch")


def device_name(device):
    if device not in ("cpu", "mps"):
        raise ValueError("Select cpu or mps explicitly")
    if device == "mps" and not torch.backends.mps.is_available():
        raise RuntimeError("MPS requested but unavailable; select cpu explicitly")
    return device


def source_hashes():
    """Portable paths identify the independent native source closure, not assets."""
    root = Path(__file__).parent
    paths = sorted((*root.rglob("*.py"), *root.rglob("*.mjs")))
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths if "__pycache__" not in path.parts and "compat" not in path.parts}


def _finite(tensor, name):
    if not bool(torch.isfinite(tensor).all()):
        raise ValueError(f"Nonfinite {name}")


def validate_tensors(obs, actions, schedules, experts, *, allow_empty=False):
    if (not isinstance(obs, torch.Tensor) or obs.dtype != torch.float32
            or obs.shape != (features.OBS_DIM,)):
        raise ValueError("Native observation tensor shape/dtype mismatch")
    if (not isinstance(actions, torch.Tensor) or actions.dtype != torch.float32
            or actions.ndim != 2 or actions.shape[1] != features.ACTION_DIM):
        raise ValueError("Native action tensor shape/dtype mismatch")
    count = len(actions)
    if not (0 if allow_empty else 1) <= count <= domains.MAX_CANDIDATES:
        raise ValueError("Native candidate count outside declared domain budget")
    for name, value, bound in (("schedule", schedules, len(features.SCHEDULES)),
                                ("expert", experts, 2)):
        if (not isinstance(value, torch.Tensor) or value.dtype != torch.int64
                or value.shape != (count,) or value.device != obs.device
                or bool(((value < 0) | (value >= bound)).any())):
            raise ValueError(f"Native {name} routing tensor mismatch")
    if actions.device != obs.device:
        raise ValueError("Native tensors must use one device")
    _finite(obs, "observation features")
    _finite(actions, "candidate features")


def tensorize(decision, device="cpu"):
    if (decision.feature_schema != features.FEATURE_SCHEMA
            or decision.candidate_schema != domains.CANDIDATE_SCHEMA
            or decision.history_schema != domains.HISTORY_SCHEMA):
        raise ValueError("Decision schema differs from native model")
    if not (len(decision.actions) == len(decision.candidate_features)
            == len(decision.schedule_ids) == len(decision.expert_ids)):
        raise ValueError("Decision candidate/routing lengths differ")
    if any(type(value) is not int for value in (*decision.schedule_ids, *decision.expert_ids)):
        raise ValueError("Integer native routing identities required")
    values = (torch.tensor(decision.observation_features, dtype=torch.float32, device=device),
              torch.tensor(decision.candidate_features, dtype=torch.float32, device=device)
              .reshape(-1, features.ACTION_DIM),
              torch.tensor(decision.schedule_ids, dtype=torch.int64, device=device),
              torch.tensor(decision.expert_ids, dtype=torch.int64, device=device))
    validate_tensors(*values, allow_empty=decision.terminal)
    return values


def feature_alias_count(obs, actions, schedules, experts, index):
    """Dynamic references resolve to full static definition IDs plus dynamic state."""
    from . import dynamic as wire
    rows = obs[wire.ENTITY_START:wire.BOND_START].reshape(layout.MAX_ENTITIES, wire.ENTITY_DIM)
    resolved = torch.cat((rows.new_zeros(1, wire.ENTITY_DIM), rows))
    refs = _indices(actions[:, :4], layout.MAX_ENTITIES, "BC candidate entities")
    same = (actions[:, 4:] == actions[index, 4:]).all(1)
    for role in range(4):
        same &= (resolved[refs[:, role]] == resolved[refs[index, role]]).all(1)
    same &= (schedules == schedules[index]) & (experts == experts[index])
    return int(same.sum())


def validate_static_definitions(bank, contract):
    from .static_model import StaticDefinitions
    return StaticDefinitions(bank, contract)


def _indices(values, upper, name):
    if bool(((values < 0) | (values > upper) | (values != values.round())).any()):
        raise ValueError("Invalid structured reference: " + name)
    return values.long()


def _masked_pool(rows, mask):
    weights = mask.to(rows.dtype)[:, None]
    mean = (rows * weights).sum(0) / weights.sum().clamp_min(1)
    maximum = rows.masked_fill(~mask[:, None], -torch.inf).amax(0)
    maximum = torch.where(mask.any(), maximum, torch.zeros_like(maximum))
    return torch.cat((mean, maximum))


class StructuredEncoder(nn.Module):
    """Shared entity/mechanism encoders, regional pooling and spatial board CNN."""

    def __init__(self):
        super().__init__()
        self.semantic_identity = nn.Embedding(layout.SEMANTIC_VOCAB, 12, padding_idx=0)
        self.semantic_fact = nn.Sequential(nn.Linear(3 * 12 + 4 + 1, 32), nn.Tanh())
        self.semantic_owner = nn.Sequential(nn.Linear(33, 32), nn.Tanh())
        self.identity = nn.Embedding(layout.ENTITY_VOCAB, 24, padding_idx=0)
        self.mechanism_identity = nn.Embedding(layout.MECHANISM_VOCAB, 8, padding_idx=0)
        self.mechanism = nn.Sequential(nn.Linear(layout.MECH_DIM - 2 + 8, 32), nn.Tanh())
        self.range = nn.Sequential(nn.Linear(2, 8), nn.Tanh())
        self.entity = nn.Sequential(nn.Linear(layout.ENTITY_DIM - layout.ENTITY_PREFIX + 24 + 32 + 16 + 32, 64), nn.Tanh())
        self.relations = nn.Sequential(nn.Linear(64 * 4, 64), nn.Tanh())
        self.board = nn.Sequential(nn.Conv2d(68, 32, 3, padding=1), nn.Tanh(),
                                   nn.Conv2d(32, 16, 3, padding=1), nn.Tanh())
        self.bonds = nn.Sequential(nn.Linear(len(layout.BOND_IDS) * layout.BOND_DIM, 64), nn.Tanh())
        self.enemy_identity = nn.Embedding(layout.ENEMY_VOCAB, 8, padding_idx=0)
        self.enemy = nn.Sequential(nn.Linear(layout.ENEMY_DIM - 2 + 8 + 32, 16), nn.Tanh())
        self.global_ = nn.Sequential(nn.Linear(layout.GLOBAL_DIM, 32), nn.Tanh())
        self.combine = nn.Sequential(nn.Linear(len(layout.AREAS) * 128 + 16 * 36 + 64 + 32 + 32, 128), nn.Tanh())

    def semantic_tables(self, facts, entities, enemies):
        mask = facts[:, 0] == 1
        _indices(facts[:, 0], 1, "semantic presence")
        owners = _indices(facts[:, 1], layout.MAX_ENTITIES + layout.MAX_ENEMIES, "semantic owner")
        present = torch.cat((mask.new_zeros(1), entities, enemies))
        if bool((mask & ~present[owners]).any()):
            raise ValueError("Public semantic fact references an absent owner")
        symbols = _indices(facts[:, [2, 3, 6]], layout.SEMANTIC_VOCAB - 1, "semantic symbols")
        kinds = _indices(facts[:, 4], 4, "semantic value type")
        if bool((mask & (kinds == 0)).any()):
            raise ValueError("Present semantic fact lacks a value type")
        types = torch.nn.functional.one_hot(kinds, 5)[:, 1:].to(facts.dtype)
        rows = self.semantic_fact(torch.cat((self.semantic_identity(symbols).flatten(1), types, facts[:, 5:6]), -1)) * mask[:, None]
        count = rows.new_zeros(len(present), 1).index_add(0, owners, mask.to(rows.dtype)[:, None])
        summed = rows.new_zeros(len(present), 32).index_add(0, owners, rows)
        result = self.semantic_owner(torch.cat((summed / count.clamp_min(1), count.log1p() / 10), -1))
        return result * (count > 0)

    def tables(self, obs):
        if obs.shape != (layout.OBS_DIM,) or obs.dtype != torch.float32:
            raise ValueError("Structured observation tensor shape/dtype mismatch")
        _finite(obs, "structured observation")
        entity = obs[layout.ENTITY_START:layout.BOND_START].reshape(layout.MAX_ENTITIES, layout.ENTITY_DIM)
        mechs = obs[layout.MECH_START:layout.GRID_START].reshape(layout.MAX_MECHANISMS, layout.MECH_DIM)
        grid = obs[layout.GRID_START:layout.ENEMY_START].reshape(36, layout.GRID_DIM)
        enemies = obs[layout.ENEMY_START:layout.RANGE_START].reshape(layout.MAX_ENEMIES, layout.ENEMY_DIM)
        ranges = obs[layout.RANGE_START:layout.FACT_START].reshape(layout.MAX_ENTITIES, layout.MAX_RANGE, 3)
        facts = obs[layout.FACT_START:layout.CONTROL_START].reshape(layout.MAX_FACTS, layout.FACT_DIM)
        semantic = self.semantic_tables(facts, entity[:, 0] == 1, enemies[:, 0] == 1)
        for rows in (entity, mechs, enemies):
            _indices(rows[:, 0], 1, "presence mask")
        _indices(ranges[:, :, 0], 1, "range mask")
        ids = _indices(entity[:, 1], layout.ENTITY_VOCAB - 1, "catalog identity")
        mids = _indices(mechs[:, 1], layout.MECHANISM_VOCAB - 1, "mechanism identity")
        refs = _indices(entity[:, 5:9], layout.MAX_MECHANISMS, "entity mechanisms")
        mask = mechs[:, 0] == 1
        mh = self.mechanism(torch.cat((self.mechanism_identity(mids), mechs[:, 2:]), -1)) * mask[:, None]
        mh = torch.cat((mh.new_zeros(1, 32), mh))
        if bool(((refs > 0) & ~torch.cat((mask.new_ones(1), mask))[refs]).any()):
            raise ValueError("Reference to absent mechanism")
        mechanism = mh[refs].sum(1) / (refs > 0).sum(1).clamp_min(1)[:, None]
        rh = self.range(ranges[:, :, 1:])
        rm = ranges[:, :, 0] == 1
        rmean = (rh * rm[:, :, None]).sum(1) / rm.sum(1).clamp_min(1)[:, None]
        rmax = rh.masked_fill(~rm[:, :, None], -torch.inf).amax(1)
        rmax = torch.where(rm.any(1)[:, None], rmax, torch.zeros_like(rmax))
        present = entity[:, 0] == 1
        hidden = self.entity(torch.cat((self.identity(ids), entity[:, layout.ENTITY_PREFIX:], mechanism, rmean, rmax, semantic[1:layout.MAX_ENTITIES + 1]), -1)) * present[:, None]
        base = torch.cat((hidden.new_zeros(1, 64), hidden))
        related = _indices(entity[:, 2:5], layout.MAX_ENTITIES, "owner/equipment")
        existence = torch.cat((present.new_ones(1), present))
        if bool(((related > 0) & ~existence[related]).any()):
            raise ValueError("Reference to absent related entity")
        # One relation layer preserves ordered equipment and owner links without recursive cycles.
        hidden = self.relations(torch.cat((hidden, base[related].flatten(1)), -1)) * present[:, None]
        entities = torch.cat((hidden.new_zeros(1, 64), hidden))
        occupants = _indices(grid[:, 2], layout.MAX_ENTITIES, "grid occupant")
        if bool(((occupants > 0) & ~existence[occupants]).any()):
            raise ValueError("Reference to absent board entity")
        cells = torch.cat((entities[occupants], grid[:, :2], grid[:, 3:]), -1)
        spatial = self.board(cells.T.reshape(1, 68, 4, 9)).reshape(16, 36).T
        areas = entity[:, layout.ENTITY_PREFIX + len(layout.KINDS):layout.ENTITY_PREFIX + len(layout.KINDS) + len(layout.AREAS)]
        regional = torch.cat([_masked_pool(hidden, present & (areas[:, i] == 1)) for i in range(len(layout.AREAS))])
        global_hidden = self.combine(torch.cat((regional, spatial.T.flatten(),
            self.bonds(obs[layout.BOND_START:layout.MECH_START]),
            _masked_pool(self.enemy(torch.cat((self.enemy_identity(_indices(enemies[:, 1], layout.ENEMY_VOCAB - 1, "enemy identity")), enemies[:, 2:], semantic[layout.MAX_ENTITIES + 1:]), -1)), enemies[:, 0] == 1),
            self.global_(obs[:layout.GLOBAL_DIM]))))
        return global_hidden, entities, torch.cat((spatial.new_zeros(1, 16), spatial)), existence

    def forward(self, obs):
        return self.tables(obs)[0]


class NativeActorCritic(nn.Module):
    """Joint categorical policy with a scheduler and two conditional scorers."""

    def __init__(self, contract, static_definitions):
        super().__init__()
        validate_contract(contract)
        self.contract = contract
        self.definitions = validate_static_definitions(static_definitions, contract)
        self.encoder = StructuredEncoder()

        def scorer():
            return nn.Sequential(nn.Linear(128 + layout.CONTROL_ENCODED_DIM + 4 * 64 + 16 + features.ACTION_DIM - 5, 128), nn.Tanh(),
                                 nn.Linear(128, 64), nn.Tanh(), nn.Linear(64, 1))

        self.purchase_head = scorer()
        self.placement_head = scorer()
        self.scheduler = nn.Linear(128 + layout.CONTROL_ENCODED_DIM, len(features.SCHEDULES))
        nn.init.zeros_(self.scheduler.weight)
        nn.init.zeros_(self.scheduler.bias)
        self.critic = nn.Sequential(nn.Linear(128 + layout.CONTROL_ENCODED_DIM, 64), nn.Tanh(), nn.Linear(64, 1))

    def value(self, obs):
        obs = self.definitions.expand(obs)
        hidden, entities, _, present = self.encoder.tables(obs)
        result = self.critic(torch.cat((hidden, self.control(obs, entities, present)))).squeeze(-1)
        _finite(result, "value estimate")
        return result

    def control(self, obs, entities, present):
        values = obs[layout.CONTROL_START:]
        refs = _indices(values[:3], layout.MAX_ENTITIES, "feedback entity")
        if bool(((refs > 0) & ~present[refs]).any()):
            raise ValueError("Feedback references an absent entity")
        identities = _indices(values[3:6], layout.ENTITY_VOCAB - 1, "feedback catalog identity")
        nums = values[6:]
        if bool(((nums[:2] < 0) | (nums[:2] > 1)).any()) or bool(nums[0] != nums[0].round()):
            raise ValueError("Invalid execution budget context")
        return torch.cat((entities[refs].flatten(), self.encoder.identity(identities).flatten(), nums))

    def forward(self, obs, actions, schedules, experts):
        validate_tensors(obs, actions, schedules, experts)
        obs = self.definitions.expand(obs)
        hidden, entities, cells, present = self.encoder.tables(obs)
        hidden = torch.cat((hidden, self.control(obs, entities, present)))
        refs = _indices(actions[:, :4], layout.MAX_ENTITIES, "candidate entities")
        if bool(((refs > 0) & ~present[refs]).any()):
            raise ValueError("Candidate references an absent entity")
        destinations = _indices(actions[:, 4], 36, "candidate destination")
        inputs = torch.cat((hidden.expand(len(actions), -1), entities[refs].flatten(1),
                            cells[destinations], actions[:, 5:]), dim=-1)
        raw = torch.where(experts == 0, self.purchase_head(inputs).squeeze(-1),
                          self.placement_head(inputs).squeeze(-1))
        masks = torch.arange(len(features.SCHEDULES), device=obs.device)[:, None] == schedules
        group_scores = self.scheduler(hidden).masked_fill(~masks.any(1), -torch.inf)
        schedule_logp = group_scores.log_softmax(0)
        # An absent group's all--inf logsumexp has undefined gradients even if
        # that output is never selected. Give only absent rows finite dummies.
        masked = raw[None, :].masked_fill(~masks, -torch.inf)
        masked = torch.where(masks.any(1)[:, None], masked, torch.zeros_like(masked))
        normalizers = torch.logsumexp(masked, dim=1)
        logp = schedule_logp[schedules] + raw - normalizers[schedules]
        value = self.critic(hidden).squeeze(-1)
        _finite(logp, "policy log probabilities")
        _finite(value, "value estimate")
        return logp, value

    @staticmethod
    def regularized_entropy(logp, schedules):
        group_logp, conditional = [], []
        for group in schedules.unique(sorted=True):
            subset = logp[schedules == group]
            mass = torch.logsumexp(subset, dim=0)
            group_logp.append(mass)
            within = subset - mass
            term = -(within.exp() * within).sum()
            conditional.append(term / math.log(len(subset)) if len(subset) > 1 else term * 0)
        masses = torch.stack(group_logp)
        return -(masses.exp() * masses).sum() + torch.stack(conditional).mean()

    @staticmethod
    def greedy_index(logp, schedules):
        groups = schedules.unique(sorted=True)
        mass = torch.stack([torch.logsumexp(logp[schedules == group], 0) for group in groups])
        return logp.masked_fill(schedules != groups[mass.argmax()], -torch.inf).argmax()

    def sample(self, decision):
        """Return the exact sampled index/logp/value; no teacher or fallback action."""
        tensors = tensorize(decision, str(next(self.parameters()).device))
        with torch.no_grad():
            logits, value = self(*tensors)
            distribution = torch.distributions.Categorical(logits=logits)
            index = distribution.sample()
            return int(index), float(distribution.log_prob(index)), float(value), tensors


def save_checkpoint(path, model, *, algorithm, updates, provenance):
    if algorithm not in ("native-behavior-cloning", "native-ppo"):
        raise ValueError("Explicit native learning algorithm required")
    if type(updates) is not int or updates < 0:
        raise ValueError("Nonnegative optimizer update count required")
    validate_contract(model.contract)
    canonical(provenance)
    metadata = {"algorithm": algorithm, "updates": updates, "provenance": provenance,
                "source_hashes": source_hashes()}
    weights = {key: value.detach().cpu().clone() for key, value in model.state_dict().items()}
    for value in weights.values():
        _finite(value, "checkpoint weights")
    payload = {"schema": CHECKPOINT_SCHEMA, "identities": identities(),
               "contract": model.contract.to_dict(), "contract_id": model.contract.contract_id,
               "metadata": metadata, "state_dict": weights, "static_definitions": model.definitions.bank}
    save_payload(path, payload)
    return metadata


def save_payload(path, payload):
    """Create bounded new files; never overwrite a checkpoint or leave partial bytes."""
    import os
    import uuid

    class BoundedWriter:
        def __init__(self, stream):
            self.stream = stream

        def write(self, data):
            if self.stream.tell() + len(data) > MAX_ARTIFACT_BYTES:
                raise ValueError("Native artifact exceeds 128 MiB serialized budget")
            return self.stream.write(data)

        def flush(self):
            self.stream.flush()

    target = Path(path)
    temporary = target.with_name(f".{target.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("xb") as stream:
            torch.save(payload, BoundedWriter(stream))
            stream.flush()
            os.fsync(stream.fileno())
        # Same-directory hard link publishes complete bytes atomically while
        # retaining exclusive-create semantics. Never replace an existing run.
        os.link(temporary, target)
    finally:
        temporary.unlink(missing_ok=True)


def load_payload(path):
    import zipfile
    with Path(path).open("rb") as stream:
        if stream.seek(0, 2) > MAX_ARTIFACT_BYTES:
            raise ValueError("Native artifact exceeds 128 MiB serialized budget")
        stream.seek(0)
        if not zipfile.is_zipfile(stream):
            raise ValueError("Only native Torch ZIP artifacts are accepted; no legacy pickle")
        stream.seek(0)
        with zipfile.ZipFile(stream) as archive:
            entries = archive.infolist()
            if (len(entries) > 20000 or sum(item.file_size for item in entries) > MAX_ARTIFACT_BYTES
                    or any(item.flag_bits & 1 for item in entries)):
                raise ValueError("Native archive expanded-size or entry budget exceeded")
        stream.seek(0)
        # Never use pickle fallback, even on installations lacking this API.
        return torch.load(stream, map_location="cpu", weights_only=True)


def load_checkpoint(path, expected_contract, *, device="cpu"):
    validate_contract(expected_contract)
    device_name(device)
    payload = load_payload(path)
    if (not isinstance(payload, dict) or set(payload) != {
            "schema", "identities", "contract", "contract_id", "metadata", "state_dict", "static_definitions"}
            or payload["schema"] != CHECKPOINT_SCHEMA or payload["identities"] != identities()
            or payload["contract"] != expected_contract.to_dict()
            or payload["contract_id"] != expected_contract.contract_id):
        raise ValueError("Native checkpoint identity mismatch; legacy conversion is unsupported")
    metadata = payload["metadata"]
    if (not isinstance(metadata, dict) or set(metadata) != {
            "algorithm", "updates", "provenance", "source_hashes"}
            or metadata["algorithm"] not in ("native-behavior-cloning", "native-ppo")
            or type(metadata["updates"]) is not int or metadata["updates"] < 0
            or metadata["source_hashes"] != source_hashes()):
        raise ValueError("Checkpoint algorithm/source provenance mismatch")
    canonical(metadata)
    model = NativeActorCritic(expected_contract, payload["static_definitions"])
    reference = model.state_dict()
    weights = payload["state_dict"]
    if not isinstance(weights, dict) or set(weights) != set(reference):
        raise ValueError("Exact native parameter names required")
    for key, value in weights.items():
        if (not isinstance(value, torch.Tensor) or value.shape != reference[key].shape
                or value.dtype != reference[key].dtype or value.layout != torch.strided):
            raise ValueError("Native parameter shape/dtype mismatch: " + key)
        _finite(value, "checkpoint parameter " + key)
    model.load_state_dict(weights, strict=True)
    model.to(device).eval()
    return model, metadata
