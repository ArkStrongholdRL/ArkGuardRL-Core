"""Model-owned immutable configuration tensors, reconstructed once at load."""
from copy import deepcopy
import torch
from torch import nn
from . import dynamic as wire, structured as layout
from .contracts import fingerprint
from .definitions import SCHEMA


class StaticDefinitions(nn.Module):
    def __init__(self, bank, contract):
        super().__init__()
        if (not isinstance(bank, dict) or bank.get("schema") != SCHEMA
                or bank.get("data_sha256") != contract.data_sha256
                or fingerprint(bank) != contract.definition_sha256):
            raise ValueError("Static definition identity differs from independent training contract")
        self.bank = deepcopy(bank)
        self.size = len(bank["keys"])
        if not 1 <= self.size <= 4096:
            raise ValueError("Static definition capacity exceeded")
        for name, width in (("entities", layout.ENTITY_DIM), ("ranges", layout.MAX_RANGE * 3),
                            ("enemies", layout.ENEMY_BASE_DIM)):
            tensor = torch.tensor(bank[name], dtype=torch.float32)
            if tensor.shape != (self.size, width) or not torch.isfinite(tensor).all():
                raise ValueError("Static definition shape/value mismatch: " + name)
            self.register_buffer(name, torch.cat((tensor.new_zeros(1, width), tensor)), persistent=False)
        mechs = torch.tensor(bank["mechanisms"], dtype=torch.float32).reshape(-1, layout.MECH_DIM)
        if len(mechs) > layout.MAX_MECHANISMS or not torch.isfinite(mechs).all():
            raise ValueError("Static mechanism capacity/value mismatch")
        self.register_buffer("mechanisms", torch.cat((mechs, mechs.new_zeros(layout.MAX_MECHANISMS - len(mechs), layout.MECH_DIM))), persistent=False)
        max_facts = max(len(rows) for rows in bank["facts"])
        if len(bank["facts"]) != self.size or max_facts > 1024:
            raise ValueError("Static per-definition fact capacity mismatch")
        facts = torch.zeros(self.size + 1, max_facts, layout.FACT_DIM)
        for i, rows in enumerate(bank["facts"], 1):
            if rows:
                facts[i, :len(rows)] = torch.tensor(rows, dtype=torch.float32)
        if not torch.isfinite(facts).all():
            raise ValueError("Static semantic facts must be finite")
        self.register_buffer("facts", facts, persistent=False)
        rules = []
        for key in layout.BOND_IDS:
            r = bank["bond_rules"][key]
            thresholds = r["thresholds"]
            if len(thresholds) > 4:
                raise ValueError("Static bond threshold capacity exceeded")
            rules.append((float(r["countsHand"]), float(r["countsGoldenOnly"]), float(r["noStack"]))
                         + tuple(layout.scalar(n, 12) for n in thresholds) + (0.0,) * (4 - len(thresholds)))
        self.register_buffer("bond_rules", torch.tensor(rules, dtype=torch.float32), persistent=False)
        self.register_buffer("columns", torch.tensor(wire.ENTITY_COLUMNS, dtype=torch.long), persistent=False)

    def expand(self, obs):
        from .neural import _indices
        if obs.shape != (wire.OBS_DIM,) or obs.dtype != torch.float32:
            raise ValueError("Dynamic observation shape/dtype mismatch")
        entities = obs[wire.ENTITY_START:wire.BOND_START].reshape(layout.MAX_ENTITIES, wire.ENTITY_DIM)
        ids = _indices(entities[:, 0], self.size, "entity static definition")
        rows = self.entities[ids].clone()
        rows[:, self.columns] = entities[:, 1:]
        # An applied effect owns its definition but keeps its dynamic effect role.
        area = layout.ENTITY_PREFIX + len(layout.KINDS) + layout.AREAS.index("active_effect")
        active = rows[:, area] == 1
        rows[active, layout.ENTITY_PREFIX:layout.ENTITY_PREFIX + len(layout.KINDS)] = 0
        rows[active, layout.ENTITY_PREFIX + layout.KINDS.index("effect")] = 1
        rows *= (entities[:, 1] == 1)[:, None]
        bonds = torch.cat((obs[wire.BOND_START:wire.GRID_START].reshape(len(layout.BOND_IDS), 5), self.bond_rules), -1)
        enemies = obs[wire.ENEMY_START:wire.CONTROL_START].reshape(layout.MAX_ENEMIES, 12)
        eids = _indices(enemies[:, 0], self.size, "enemy static definition")
        enemy_rows = torch.cat((enemies[:, 1:], self.enemies[eids]), -1)
        owners = torch.cat((ids, eids))
        selected = self.facts[owners].clone()
        selected[:, :, 1] = torch.arange(1, len(owners) + 1, device=obs.device)[:, None]
        selected = selected.reshape(-1, layout.FACT_DIM)
        selected = selected[selected[:, 0] == 1]
        if len(selected) > layout.MAX_FACTS:
            raise ValueError("Public semantic fact capacity exceeded; no truncation")
        selected = torch.cat((selected, obs.new_zeros(layout.MAX_FACTS - len(selected), layout.FACT_DIM)))
        return torch.cat((obs[:wire.ENTITY_START], rows.flatten(), bonds.flatten(), self.mechanisms.flatten(),
                          obs[wire.GRID_START:wire.ENEMY_START], enemy_rows.flatten(), self.ranges[ids].flatten(),
                          selected.flatten(), obs[wire.CONTROL_START:]))
