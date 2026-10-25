"""GPL-3.0 single covenant residual over a frozen survival policy.

Uses the unchanged public native input bank and existing live PPO collector.
The deterministic availability/visible-member gate is an experimental scope,
not a learned selector or an obligation to commit to the covenant.
"""
import math
import torch
from torch import nn
from arkguardrl import structured as layout
from .models import Policy
from .routes import features as route

SCHEMA = 'arkguardrl-core/frozen-base-covenant-residual/v1'
CONTEXT_SCHEMA = 'arkguardrl-core/full-entity-context-residual/v1'


def validate(config):
    fields = {'schema', 'bond', 'band_index', 'hidden', 'logit_bound'}
    if config.get('schema') == CONTEXT_SCHEMA:
        fields |= {'encoder', 'layers', 'heads', 'ff_multiplier'}
        if config['encoder'] not in ('pool', 'transformer'):
            raise ValueError('Explicit full-context encoder required')
        if config['layers'] != 2 or config['heads'] != 4 or config['hidden'] % config['heads']:
            raise ValueError('Two bounded layers and four dividing heads required')
        if config['ff_multiplier'] not in (2, 4):
            raise ValueError('Bounded feedforward multiplier required')
    if set(config) != fields:
        raise ValueError('Explicit specialist configuration required')
    if config['schema'] not in (SCHEMA, CONTEXT_SCHEMA) or config['bond'] not in layout.BOND_IDS:
        raise ValueError('Unknown specialist representation or covenant')
    if type(config['band_index']) is not int or not 0 < config['band_index'] < layout.ENTITY_VOCAB:
        raise ValueError('Public fixed-band catalog index required')
    if type(config['hidden']) is not int or not 8 <= config['hidden'] <= 256:
        raise ValueError('Bounded specialist hidden dimension required')
    if not math.isfinite(config['logit_bound']) or not 0 < config['logit_bound'] <= 4:
        raise ValueError('Specialist residual bound must be in (0,4]')


class SpecialistPolicy(Policy):
    def __init__(self, contract, definitions, width, route_backend, config):
        validate(config)
        super().__init__(contract, definitions, width, route_backend)
        self.specialist_config = dict(config)
        self.requires_grad_(False)
        state_dim = layout.GLOBAL_DIM + len(layout.BOND_IDS)*layout.BOND_DIM + layout.CONTROL_DIM - 6
        candidate_dim = 4*(layout.ENTITY_DIM-layout.ENTITY_PREFIX+24*width) + layout.ACTION_DIM-5 + route.ACTION_DIM
        self.specialist_actor = nn.Sequential(nn.Linear(state_dim+candidate_dim, config['hidden']),
            nn.Tanh(), nn.Linear(config['hidden'], 1))
        self.specialist_value = nn.Sequential(nn.Linear(state_dim, config['hidden']),
            nn.Tanh(), nn.Linear(config['hidden'], 1))
        for head in (self.specialist_actor, self.specialist_value):
            nn.init.zeros_(head[-1].weight)
            nn.init.zeros_(head[-1].bias)

    def forward(self, row):
        with torch.no_grad():
            logp, value = super().forward(row)
            obs = self.base.definitions.expand(row['obs'])
        result, values = self.adjust(logp[None], value.reshape(1),
            obs[None], obs[layout.CONTROL_START:][None], row['candidates'][None],
            row['route_actions'][None], torch.ones_like(logp, dtype=torch.bool)[None])
        return result[0], values[0]

    def adjust(self, logp, value, tables, control, actions, routes, valid):
        batch, count = valid.shape
        entities = tables[:, layout.ENTITY_START:layout.BOND_START].reshape(batch, layout.MAX_ENTITIES, layout.ENTITY_DIM)
        bonds = tables[:, layout.BOND_START:layout.MECH_START].reshape(batch, len(layout.BOND_IDS), layout.BOND_DIM)
        bond_index = layout.BOND_IDS.index(self.specialist_config['bond'])
        member_start = (layout.ENTITY_PREFIX+len(layout.KINDS)+len(layout.AREAS)
                        +len(layout.ENTITY_NUMBERS)+2*len(layout.STATS))
        phase = tables[:, len(layout.GLOBAL_FIELDS)+layout.PHASES.index('PREP')] > .5
        visible = ((entities[:, :, 0] > .5) & (entities[:, :, member_start+bond_index] > .5)).any(1)
        gate = phase & visible & (bonds[:, bond_index, layout.BOND_FIELDS.index('off')] < .5)
        state = torch.cat((tables[:, :layout.GLOBAL_DIM], bonds.flatten(1), control[:, 6:]), -1)
        bank = torch.cat((entities.new_zeros(batch, 1, layout.ENTITY_DIM), entities), 1)
        owner = torch.arange(batch, device=tables.device)[:, None, None]
        referred = bank[owner, actions[:, :, :4].long()]
        identities = self.base.encoder.identity(referred[:, :, :, 1].long())
        candidates = torch.cat((referred[:, :, :, layout.ENTITY_PREFIX:], identities), -1).flatten(2)
        features = torch.cat((state[:, None].expand(-1, count, -1), candidates, actions[:, :, 5:], routes), -1)
        delta = self.specialist_config['logit_bound'] * self.specialist_actor(features).squeeze(-1).tanh()
        value_delta = self.specialist_value(state).squeeze(-1)
        return self.finish(logp, value, delta, value_delta, gate, referred, actions, valid)

    def finish(self, logp, value, delta, value_delta, gate, referred, actions, valid):
        adjusted = logp + delta*gate[:, None]
        # Band selection is part of the declared actor for collection AND replay.
        # Finite zero-probability logits keep the existing PPO audit contract.
        band_column = 5+len(layout.DIRECTIONS)+layout.ACTION_TYPES.index('band')
        is_band = (actions[:, :, band_column] > .5) & valid
        has_band = is_band.any(1)
        subject = referred[:, :, 3, 1].long()
        fixed = is_band & (subject == self.specialist_config['band_index'])
        if bool((has_band & (fixed.sum(1) != 1)).any()):
            raise ValueError('Fixed band not present exactly once in legal actor candidates')
        forced = torch.where(fixed, adjusted.new_zeros(()), adjusted.new_full((), -1e9))
        adjusted = torch.where(has_band[:, None], forced, adjusted).masked_fill(~valid, -torch.inf)
        values = value + value_delta*gate
        return adjusted.log_softmax(1), values
