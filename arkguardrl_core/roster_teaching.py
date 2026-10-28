"""Public replacement targets and distribution-preserving task teaching.

The roster score is an approximate teacher prior, not a combat predictor or
environment reward. Tier-power and swap search follow the GPL engine bot's
public lineup design; no engine files or private match state are imported.
"""
import random
import time

import torch
from arkguardrl.candidates import public_context, empty_deployment_allowed, terrain_allows
from . import roster_tasks as task
from .routes import features as route


class ReplacementMemory(task.Memory):
    """Reconsider all owned chess after acquisition; keep the set stable on moves."""
    def update(self, state, public):
        previous = self.signature
        super().update(state, public)
        if state['phase'] != 'PREP' or previous == self.signature:
            return self.targets
        own = state['own']
        pieces = [p for area in ('board', 'hand', 'temp') for p in own[area]
                  if p and p['kind'] == 'chess']
        # record() is an identity/mechanism projection; combat attributes live
        # in the public catalog table, also used by the route feature encoder.
        records = {p['uid']: public._data['chess'][p['id']] for p in pieces}
        power = (0, 10, 12.5, 15, 18, 21.5, 25)
        def value(p):
            rec = records[p['uid']]
            return power[max(1, min(6, rec['tier']))]*(1.9 if rec['isGolden'] else 1) + 5*len(p['items'])
        disabled = set(state['public']['disabledBonds'])
        bonds = public._data['bonds']
        def score(group):
            recs = [records[p['uid']] for p in group]
            blockers = sum(public.placement_class(p) == 'melee'
                and (records[p['uid']].get('stats') or {}).get('blockCnt', 0) > 0
                and records[p['uid']].get('attackKind') != 'none' for p in group)
            healers = sum(r.get('dmgType') == 'heal' or r.get('attackKind') == 'heal' for r in recs)
            total = sum(value(p) for p in group)
            total -= max(0, min(2, len(group)) - blockers)*15
            total -= max(0, healers-2)*10
            counts = {}
            for rec in recs:
                for bond in rec.get('bonds', []):
                    counts.setdefault(bond, set()).add(rec['baseId'])
            for key, members in counts.items():
                bond = bonds[key]
                # Only this simple public BOARD template is approximated here.
                if (key in disabled or bond.get('countMode') != 'BOARD'
                        or bond.get('countsGoldenOnly') or bond.get('thresholdTemplate') != 'count_threshold_upward'):
                    continue
                tier = sum(len(members) >= n for n in bond.get('thresholds', []))
                total += tier*(14 if bond.get('isCore') else 9)
            return total
        ordered = sorted(pieces, key=lambda p: (-value(p), p['uid']))
        selected = ordered[:own['deployCap']]
        reserve = ordered[own['deployCap']:]
        current = score(selected)
        for _ in range(12):
            best = None
            for j, incoming in enumerate(reserve):
                for i in range(len(selected)):
                    trial = selected.copy()
                    trial[i] = incoming
                    gain = score(trial)
                    if gain > current+.5 and (best is None or gain > best[0]):
                        best = (gain, i, j)
            if best is None:
                break
            current, i, j = best
            selected[i], reserve[j] = reserve[j], selected[i]
        self.targets = frozenset(p['uid'] for p in selected)
        return self.targets


def corrective_indices(decision, public, targets, route_actions):
    """Empty-cell deployment or exact wanted-for-unwanted replacement only."""
    state = decision.state
    if state['phase'] != 'PREP' or task.complete(state, targets):
        return []
    context = public_context(state, public)
    locations, occupied, cells = context
    board = {p['uid'] for p in state['own']['board']}
    moves = []
    for i, a in enumerate(decision.actions):
        if a['t'] != 'g.move' or a.get('uid') not in targets-board:
            continue
        to = a['to']
        if to['area'] != 'board':
            continue
        cell = (to['row'], to['col'])
        source = locations[a['uid']][2]
        dest = occupied.get(cell)
        if dest is None:
            supported = empty_deployment_allowed(source, cell, state, public, context)
        else:
            supported = (dest['kind'] == 'chess' and dest['uid'] not in targets
                         and cell in cells and terrain_allows(source, cells[cell], public))
        if not supported:
            continue
        f = dict(zip(route.ACTION_FIELDS, route_actions[i].tolist()))
        value = (3*f['delta_ground_coverage'] + 2*f['delta_air_coverage']
                 + f['delta_supported_blockers'] + f['delta_damage_contact']
                 + f['delta_healing_contact'] + .2*f['ground_range_fraction'])
        moves.append((value, i))
    if not moves:
        return []
    best = max(value for value, _ in moves)
    return [i for value, i in moves if value >= best-.02]


def fit(residual, rows, *, steps, batch_size, learning_rate, seed, max_wall):
    """Balanced correction CE and full-distribution preservation, no PPO rows."""
    corrections = [r for r in rows if r['correction']]
    preserve = [r for r in rows if not r['correction'] and not r['repeated_move']]
    if not corrections or not preserve or min(steps, batch_size, learning_rate, max_wall) <= 0:
        raise ValueError('Finite budget, corrections and preservation states required')
    optimizer = torch.optim.Adam(residual.parameters(), lr=learning_rate)
    rng = random.Random(seed)
    start, history, done = time.monotonic(), [], 0
    for step in range(steps):
        if time.monotonic()-start >= max_wall:
            break
        optimizer.zero_grad(set_to_none=True)
        total = 0.
        for i in range(batch_size):
            row = rng.choice(corrections if i % 2 == 0 else preserve)
            parent = row['parent_logp']
            logp = residual(parent, row['schedules'], row['route_actions'], row['relations'])
            kl = (parent.exp()*(parent-logp)).sum()
            if row['correction']:
                loss = -.5*logp[row['index']]-.5*torch.logsumexp(logp[row['acceptable']], 0)+.02*kl
            else:
                loss = kl
            if not torch.isfinite(loss):
                raise ValueError('Nonfinite task loss')
            (loss/batch_size).backward()
            total += float(loss.detach())/batch_size
        torch.nn.utils.clip_grad_norm_(residual.parameters(), 1., error_if_nonfinite=True)
        optimizer.step()
        done += 1
        if done == 1 or done % 100 == 0 or done == steps:
            history.append(dict(step=done, loss=total, seconds=time.monotonic()-start))
    return dict(steps=done, stop='steps' if done == steps else 'wall', history=history,
        corrections=len(corrections), preservation=len(preserve),
        optimizer=optimizer.state_dict(), rng=rng.getstate())
