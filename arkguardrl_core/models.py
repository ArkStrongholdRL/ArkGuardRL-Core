"""GPL-3.0 native v7 encoder adaptation with explicit internal-width contract.

Derived from frozen arkguardrl/neural.py StructuredEncoder; native module remains
unchanged. Only learned latent widths scale; raw inputs, categories and routing
retain their exact original meanings. Width1 must reproduce the frozen model.
"""
from pathlib import Path
import torch
from torch import nn
from arkguardrl import neural, structured as layout, features, learn
from arkguardrl.neural import _indices, _finite, _masked_pool
from arkguardrl.contracts import fingerprint
from .routes import residual as rp
from .routes import features as rf

SCHEMA = 'arkguardrl-core/route-width-array-batch-policy/v1'

class StructuredEncoder(nn.Module):
    """Shared entity/mechanism encoders, regional pooling and spatial board CNN."""

    def __init__(self, width):
        super().__init__()
        self.width = width
        self.semantic_identity = nn.Embedding(layout.SEMANTIC_VOCAB, 12*width, padding_idx=0)
        self.semantic_fact = nn.Sequential(nn.Linear(36*width+5, 32*width), nn.Tanh())
        self.semantic_owner = nn.Sequential(nn.Linear(32*width+1, 32*width), nn.Tanh())
        self.identity = nn.Embedding(layout.ENTITY_VOCAB, 24*width, padding_idx=0)
        self.mechanism_identity = nn.Embedding(layout.MECHANISM_VOCAB, 8*width, padding_idx=0)
        self.mechanism = nn.Sequential(nn.Linear(layout.MECH_DIM-2+8*width, 32*width), nn.Tanh())
        self.range = nn.Sequential(nn.Linear(2, 8*width), nn.Tanh())
        self.entity = nn.Sequential(nn.Linear(layout.ENTITY_DIM-layout.ENTITY_PREFIX+104*width, 64*width), nn.Tanh())
        self.relations = nn.Sequential(nn.Linear(256*width, 64*width), nn.Tanh())
        self.board = nn.Sequential(nn.Conv2d(64*width+4, 32*width, 3, padding=1), nn.Tanh(),
                                   nn.Conv2d(32*width, 16*width, 3, padding=1), nn.Tanh())
        self.bonds = nn.Sequential(nn.Linear(len(layout.BOND_IDS)*layout.BOND_DIM, 64*width), nn.Tanh())
        self.enemy_identity = nn.Embedding(layout.ENEMY_VOCAB, 8*width, padding_idx=0)
        self.enemy = nn.Sequential(nn.Linear(layout.ENEMY_DIM-2+40*width, 16*width), nn.Tanh())
        self.global_ = nn.Sequential(nn.Linear(layout.GLOBAL_DIM, 32*width), nn.Tanh())
        self.combine = nn.Sequential(nn.Linear((len(layout.AREAS)*128+16*36+64+32+32)*width, 128*width), nn.Tanh())

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
        summed = rows.new_zeros(len(present), 32*self.width).index_add(0, owners, rows)
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
        mh = torch.cat((mh.new_zeros(1, 32*self.width), mh))
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
        base = torch.cat((hidden.new_zeros(1, 64*self.width), hidden))
        related = _indices(entity[:, 2:5], layout.MAX_ENTITIES, "owner/equipment")
        existence = torch.cat((present.new_ones(1), present))
        if bool(((related > 0) & ~existence[related]).any()):
            raise ValueError("Reference to absent related entity")
        # One relation layer preserves ordered equipment and owner links without recursive cycles.
        hidden = self.relations(torch.cat((hidden, base[related].flatten(1)), -1)) * present[:, None]
        entities = torch.cat((hidden.new_zeros(1, 64*self.width), hidden))
        occupants = _indices(grid[:, 2], layout.MAX_ENTITIES, "grid occupant")
        if bool(((occupants > 0) & ~existence[occupants]).any()):
            raise ValueError("Reference to absent board entity")
        cells = torch.cat((entities[occupants], grid[:, :2], grid[:, 3:]), -1)
        spatial = self.board(cells.T.reshape(1, 64*self.width+4, 4, 9)).reshape(16*self.width, 36).T
        areas = entity[:, layout.ENTITY_PREFIX + len(layout.KINDS):layout.ENTITY_PREFIX + len(layout.KINDS) + len(layout.AREAS)]
        regional = torch.cat([_masked_pool(hidden, present & (areas[:, i] == 1)) for i in range(len(layout.AREAS))])
        global_hidden = self.combine(torch.cat((regional, spatial.T.flatten(),
            self.bonds(obs[layout.BOND_START:layout.MECH_START]),
            _masked_pool(self.enemy(torch.cat((self.enemy_identity(_indices(enemies[:, 1], layout.ENEMY_VOCAB - 1, "enemy identity")), enemies[:, 2:], semantic[layout.MAX_ENTITIES + 1:]), -1)), enemies[:, 0] == 1),
            self.global_(obs[:layout.GLOBAL_DIM]))))
        return global_hidden, entities, torch.cat((spatial.new_zeros(1, 16*self.width), spatial)), existence

    def forward(self, obs):
        return self.tables(obs)[0]


class CompactEncoder(StructuredEncoder):
    def semantic_tables(self, facts, entities, enemies):
        mask = facts[:, 0] == 1
        _indices(facts[:, 0], 1, 'semantic presence')
        owners = _indices(facts[:, 1], layout.MAX_ENTITIES + layout.MAX_ENEMIES, 'semantic owner')
        present = torch.cat((mask.new_zeros(1), entities, enemies))
        if bool((mask & ~present[owners]).any()):
            raise ValueError('Public semantic fact references an absent owner')
        symbols = _indices(facts[:, [2, 3, 6]], layout.SEMANTIC_VOCAB - 1, 'semantic symbols')
        kinds = _indices(facts[:, 4], 4, 'semantic value type')
        if bool((mask & (kinds == 0)).any()):
            raise ValueError('Present semantic fact lacks a value type')
        # Validate ALL original rows before compaction, including masked indices.
        facts, symbols, kinds, owners = facts[mask], symbols[mask], kinds[mask], owners[mask]
        types = torch.nn.functional.one_hot(kinds, 5)[:, 1:].to(facts.dtype)
        rows = self.semantic_fact(torch.cat((self.semantic_identity(symbols).flatten(1),
                                             types, facts[:, 5:6]), -1))
        count = rows.new_zeros(len(present), 1).index_add(0, owners, rows.new_ones(len(rows), 1))
        summed = rows.new_zeros(len(present), 32*self.width).index_add(0, owners, rows)
        result = self.semantic_owner(torch.cat((summed / count.clamp_min(1), count.log1p() / 10), -1))
        return result * (count > 0)


class Base(neural.NativeActorCritic):
    def __init__(self, contract, definitions, width):
        nn.Module.__init__(self)
        neural.validate_contract(contract)
        self.contract = contract
        self.definitions = neural.validate_static_definitions(definitions, contract)
        self.encoder = StructuredEncoder(width)
        control = layout.CONTROL_ENCODED_DIM + 264*(width-1)
        global_dim = 128*width + control
        def scorer():
            return nn.Sequential(nn.Linear(global_dim+256*width+16*width+features.ACTION_DIM-5, 128*width), nn.Tanh(),
                                 nn.Linear(128*width, 64*width), nn.Tanh(), nn.Linear(64*width, 1))
        self.purchase_head = scorer()
        self.placement_head = scorer()
        self.scheduler = nn.Linear(global_dim, len(features.SCHEDULES))
        nn.init.zeros_(self.scheduler.weight)
        nn.init.zeros_(self.scheduler.bias)
        self.critic = nn.Sequential(nn.Linear(global_dim, 64*width), nn.Tanh(), nn.Linear(64*width, 1))


class Policy(rp.RoutePolicy):
    def __init__(self, contract, definitions, width=1, route_backend='array'):
        if route_backend not in ('array', 'reference'):
            raise ValueError('Explicit array/reference route backend required')
        self.route_backend = route_backend
        if type(width) is not int or width not in (1, 2):
            raise ValueError('Explicit width1 or width2 required')
        nn.Module.__init__(self)
        self.width = width
        self.base = Base(contract, definitions, width)
        self.base.encoder = CompactEncoder(width)
        self.route_encoder = nn.Sequential(nn.Linear(rf.OBS_DIM, 64*width), nn.Tanh(),
                                           nn.Linear(64*width, 32*width), nn.Tanh())
        self.schedule = nn.Linear(32*width, len(features.SCHEDULES))
        self.target = nn.Sequential(nn.Linear(32*width+rf.ACTION_DIM,64*width), nn.Tanh(), nn.Linear(64*width,1))
        nn.init.zeros_(self.schedule.weight); nn.init.zeros_(self.schedule.bias)
        nn.init.zeros_(self.target[-1].weight); nn.init.zeros_(self.target[-1].bias)


    def row(self, decision, geometry, public, *, route_cache=None):
        if self.route_backend == 'reference':
            return super().row(decision, geometry, public)
        from .route_arrays import encode
        tensors = neural.tensorize(decision)
        obs, actions = encode(decision.state, decision.actions, geometry, public, route_cache=route_cache)
        return dict(zip(('obs', 'candidates', 'schedules', 'experts'), tensors)) | {
            'route_obs': obs, 'route_actions': actions}

    def forward_batch(self, data):
        from .learning.network import forward
        return forward(self, data)
