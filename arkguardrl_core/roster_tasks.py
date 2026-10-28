"""Public stable roster tasks and a supervised residual over a frozen actor.

Targets describe execution, not combat quality. No candidate mask, action
override, reward change or on-policy buffer is introduced here.
"""
import random
import time

import torch
from torch import nn
from arkguardrl.candidates import public_context, empty_deployment_allowed
from arkguardrl.features import SCHEDULES
from .routes import features as route

SCHEMA = 'arkguardrl-core/stable-owned-roster-task/v1'
FIELDS = ('active', 'missing_fraction', 'complete', 'source_wanted',
          'source_missing', 'destination_wanted', 'deploy_missing_empty',
          'deploy_missing_swap', 'withdraw_wanted', 'sell_wanted',
          'displace_wanted', 'withdraw_unwanted', 'ready_complete',
          'ready_incomplete', 'board_fraction', 'target_fraction')


class Memory:
    """Keep identities stable across moves, including board/hand swaps.

    Conservative baseline target: retain deployed chess, fill remaining slots
    from owned reserves by public tier/gold rank. It is not a learned planner.
    """
    def __init__(self):
        self.signature = None
        self.targets = frozenset()
        self.revisions = 0

    def update(self, state, public):
        if state['phase'] != 'PREP':
            self.signature = None
            self.targets = frozenset()
            return self.targets
        own = state['own']
        pieces = [p for area in ('board', 'hand', 'temp') for p in own[area] if p]
        signature = (state['round'], own['deployCap'], tuple(sorted(
            (p['uid'], p['id'], tuple(sorted((i['uid'], i['id']) for i in p['items'])))
            for p in pieces)))
        if signature != self.signature:
            deployed = [p for p in own['board'] if p['kind'] == 'chess']
            board_ids = {p['uid'] for p in deployed}
            reserve = [p for p in pieces if p['kind'] == 'chess' and p['uid'] not in board_ids]
            def rank(p):
                rec = public.record('chess', p['id'])
                return (-int(rec.get('isGolden', False)), -rec.get('tier', 0), p['uid'])
            chosen = deployed + sorted(reserve, key=rank)
            self.targets = frozenset(p['uid'] for p in chosen[:own['deployCap']])
            self.signature = signature
            self.revisions += 1
        return self.targets


def complete(state, targets):
    return targets <= {p['uid'] for p in state['own']['board']}


def encode(state, actions, targets):
    result = torch.zeros((len(actions), len(FIELDS)), dtype=torch.float32)
    if state['phase'] != 'PREP':
        return result
    own = state['own']
    board = {p['uid'] for p in own['board']}
    occupied = {(p['row'], p['col']): p['uid'] for p in own['board']}
    missing = targets - board
    for i, a in enumerate(actions):
        move = a['t'] == 'g.move'
        to = a.get('to', {})
        dest = occupied.get((to.get('row'), to.get('col'))) if to.get('area') == 'board' else None
        wanted = a.get('uid') in targets
        source_missing = a.get('uid') in missing
        deploy = move and source_missing and to.get('area') == 'board'
        withdraw = move and a.get('uid') in board and to.get('area') == 'hand'
        ready = a['t'] == 'g.ready'
        result[i] = torch.tensor((1., len(missing)/max(1, len(targets)), not missing,
            wanted, source_missing, dest in targets, deploy and dest is None,
            deploy and dest is not None, withdraw and wanted,
            a['t'] == 'g.sell' and wanted, move and dest in targets and not wanted,
            withdraw and not wanted, ready and not missing, ready and bool(missing),
            own['deployCount']/max(1, own['deployCap']), len(targets)/max(1, own['deployCap'])))
    return result


def corrective_indices(decision, public, targets, route_actions):
    """Positive task moves on supported empty tiles; no buying or selling.

    Return equally acceptable placements within a small public-route score band.
    Unsupported/blocked targets are reported by an empty set, never a fake Ready.
    """
    state = decision.state
    if state['phase'] != 'PREP' or complete(state, targets):
        return []
    context = public_context(state, public)
    locations, occupied, _ = context
    board = {p['uid'] for p in state['own']['board']}
    moves = []
    for i, a in enumerate(decision.actions):
        if a['t'] != 'g.move' or a.get('uid') not in targets - board:
            continue
        to = a['to']
        if to['area'] != 'board':
            continue
        piece = locations[a['uid']][2]
        if not empty_deployment_allowed(piece, (to['row'], to['col']), state, public, context):
            continue
        f = dict(zip(route.ACTION_FIELDS, route_actions[i].tolist()))
        score = (3*f['delta_ground_coverage'] + 2*f['delta_air_coverage']
                 + f['delta_supported_blockers'] + f['delta_damage_contact']
                 + f['delta_healing_contact'] + .2*f['ground_range_fraction'])
        moves.append((score, i))
    if not moves:
        return []
    best = max(score for score, _ in moves)
    return [i for score, i in moves if score >= best - .02]


class Residual(nn.Module):
    def __init__(self, enabled=True):
        super().__init__()
        self.enabled = enabled
        self.candidate = nn.Sequential(nn.Linear(len(FIELDS)+route.ACTION_DIM, 64),
            nn.Tanh(), nn.Linear(64, 1))
        self.scheduler = nn.Linear(2*len(FIELDS), len(SCHEDULES))
        nn.init.zeros_(self.candidate[-1].weight)
        nn.init.zeros_(self.candidate[-1].bias)
        nn.init.zeros_(self.scheduler.weight)
        nn.init.zeros_(self.scheduler.bias)

    def forward(self, parent_logp, schedules, route_actions, relations):
        if relations.shape != (len(parent_logp), len(FIELDS)):
            raise ValueError('Explicit candidate-aligned roster relations required')
        if not self.enabled:
            relations = torch.zeros_like(relations)
        delta = self.candidate(torch.cat((route_actions, relations), -1)).squeeze(-1)
        shift = self.scheduler(torch.cat((relations.mean(0), relations.amax(0))))
        groups = [schedules == i for i in range(len(SCHEDULES))]
        mass = torch.stack([torch.logsumexp(parent_logp[g], 0) if g.any()
                            else parent_logp.new_tensor(-torch.inf) for g in groups])
        new_mass = (mass + shift).log_softmax(0)
        raw = parent_logp - mass[schedules] + delta
        normalizer = torch.stack([torch.logsumexp(raw[g], 0) if g.any()
                                 else raw.new_zeros(()) for g in groups])
        return new_mass[schedules] + raw - normalizer[schedules]


def fit(residual, rows, *, steps, batch_size, learning_rate, seed, max_wall, anchor_kl=.02):
    """CPU supervised CE/acceptable-mass objective; parent scores are frozen.

    Cached scores belong only to this auxiliary learner. They are never PPO rows.
    """
    if not rows or min(steps, batch_size, learning_rate, max_wall) <= 0:
        raise ValueError('Finite supervised budget and nonempty rows required')
    optimizer = torch.optim.Adam(residual.parameters(), lr=learning_rate)
    rng = random.Random(seed)
    start, history, done = time.monotonic(), [], 0
    for step in range(steps):
        if time.monotonic()-start >= max_wall:
            break
        optimizer.zero_grad(set_to_none=True)
        loss_sum = 0.
        for row in rng.choices(rows, k=batch_size):
            logp = residual(*(row[k] for k in ('parent_logp', 'schedules', 'route_actions', 'relations')))
            parent = row['parent_logp']
            loss = (-.5*logp[row['index']] - .5*torch.logsumexp(logp[row['acceptable']], 0)
                    + anchor_kl*(parent.exp()*(parent-logp)).sum())
            if not torch.isfinite(loss):
                raise ValueError('Nonfinite task loss')
            (loss/batch_size).backward()
            loss_sum += float(loss.detach())/batch_size
        nn.utils.clip_grad_norm_(residual.parameters(), 1., error_if_nonfinite=True)
        optimizer.step()
        done += 1
        if done == 1 or done % 100 == 0 or done == steps:
            history.append(dict(step=done, loss=loss_sum, seconds=time.monotonic()-start))
    return dict(steps=done, stop='steps' if done == steps else 'wall', history=history,
                optimizer=optimizer.state_dict(), rng=rng.getstate())


def save(path, residual, metadata):
    torch.save(dict(schema=SCHEMA, enabled=residual.enabled,
        state_dict={k: v.detach().cpu() for k, v in residual.state_dict().items()},
        metadata=metadata), path)


def load(path):
    payload = torch.load(path, map_location='cpu', weights_only=True)
    if payload['schema'] != SCHEMA:
        raise ValueError('Unknown roster task checkpoint')
    model = Residual(payload['enabled'])
    model.load_state_dict(payload['state_dict'], strict=True)
    return model.eval(), payload['metadata']
