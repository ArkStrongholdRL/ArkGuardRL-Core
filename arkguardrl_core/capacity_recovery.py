"""GPL-3.0 bounded PREP64 single-chess overflow execution contract.

This restricts otherwise game-legal swaps. No prices, ranks, covenant labels,
rewards, native engine probes or scripted post-sampling overrides are used.
Uncovered obligations are technical failures, never personal deaths.
"""
from dataclasses import replace
import json
import torch
from arkguardrl.candidates import PREP_ACTION_LIMIT, public_context, terrain_allows
from arkguardrl.contracts import action_id

SCHEMA = 'arkguardrl-core/prep64-single-chess-finite-recovery/v1'
DIM = 8
FIELDS = ('active', 'open', 'finish', 'ready', 'remaining_div3',
          'root_uid', 'owed_uid', 'swapped')
CONFIG = dict(schema=SCHEMA, attempts=3, scope='single-unequipped-temp-chess')


class RecoveryFailure(RuntimeError):
    """Finite technical sampling cutoff, not a game termination."""


def validate_tensor(value, batch=None):
    shape = (DIM,) if batch is None else (batch, DIM)
    if not isinstance(value, torch.Tensor) or value.dtype != torch.float32 or tuple(value.shape) != shape:
        raise ValueError('Explicit public recovery tensor required')
    if not bool(torch.isfinite(value).all()) or bool((value < 0).any()):
        raise ValueError('Invalid recovery context')


class RecoveryMemory:
    def __init__(self):
        self.round = None
        self.stage = 'idle'
        self.remaining = 3
        self.root = self.owed = 0
        self.swapped = False
        self.rejected = set()
        self.events = []

    def snapshot(self):
        return dict(schema=SCHEMA, round=self.round, stage=self.stage,
                    remaining=self.remaining, root_uid=self.root, owed_uid=self.owed,
                    swapped=self.swapped, rejected=sorted(self.rejected))

    def tensor(self):
        return torch.tensor([float(self.stage != 'idle'), float(self.stage == 'open'),
            float(self.stage == 'finish'), float(self.stage == 'ready'),
            self.remaining/3 if self.stage != 'idle' else 0., float(self.root),
            float(self.owed), float(self.swapped)], dtype=torch.float32)

    def constrain(self, decision, public):
        state = decision.state
        if state['round'] != self.round:
            self.round = state['round']
            self.stage = 'idle'
            self.remaining = 3
            self.root = self.owed = 0
            self.swapped = False
            self.rejected.clear()
        own = state['own']
        used = json.loads(decision.history_json)['used']
        temp = [p for p in own['temp'] if p]
        if state['phase'] != 'PREP' or used < PREP_ACTION_LIMIT:
            return decision
        if self.stage == 'idle':
            # canReady=false alone is not evidence of a capacity obligation.
            if not temp or own.get('personalChoice'):
                return decision
            if own['canReady'] or len(temp) != 1 or temp[0]['kind'] != 'chess' or temp[0]['items']:
                raise RecoveryFailure('Unsupported PREP64 overflow obligation')
            self.stage = 'open'
            self.root = self.owed = temp[0]['uid']
            self.events.append(dict(event='start', **self.snapshot()))
        if self.remaining <= 0:
            raise RecoveryFailure('PREP64 recovery attempt bound exhausted')
        locations, occupied, cells = public_context(state, public)
        allowed = []
        for i, action in enumerate(decision.actions):
            if action_id(action) in self.rejected:
                continue
            if self.stage == 'ready':
                keep = action['t'] == 'g.ready' and own['canReady'] and not temp
            elif action['t'] == 'g.sell':
                area, _, piece = locations[action['uid']]
                keep = piece['kind'] == 'chess' and not piece['items'] and (
                    action['uid'] == self.owed or (area == 'hand' and self.remaining >= 2)
                    or (self.stage == 'open' and self.remaining >= 3 and area == 'board'
                        and terrain_allows(locations[self.owed][2],
                            cells[(piece['row'],piece['col'])], public)))
            elif action['t'] == 'g.move' and action['uid'] == self.owed:
                to = action['to']
                if to['area'] == 'hand':
                    keep = own['hand'][to['idx']] is None
                else:
                    occupant = occupied.get((to['row'], to['col']))
                    keep = occupant is None or (self.stage == 'open' and self.remaining >= 3
                        and occupant['kind'] == 'chess' and not occupant['items'])
            else:
                keep = False
            # Every unresolved move/sale needs one remaining accepted Ready.
            if self.stage != 'ready' and self.remaining < 2:
                keep = False
            if keep:
                allowed.append(i)
        if not allowed:
            raise RecoveryFailure('No supported bounded recovery exit candidate')
        return replace(decision, actions=tuple(decision.actions[i] for i in allowed),
            candidate_features=tuple(decision.candidate_features[i] for i in allowed),
            schedule_ids=tuple(decision.schedule_ids[i] for i in allowed),
            expert_ids=tuple(decision.expert_ids[i] for i in allowed))

    def note(self, before, action, result):
        if self.stage == 'idle':
            return
        previous = self.snapshot()
        self.remaining -= 1  # Rejections consume budget too; UID changes never reset.
        if not result.accepted:
            self.rejected.add(action_id(action))
        elif action['t'] == 'g.ready':
            self.events.append(dict(event='ready-accepted', before=previous, after=self.snapshot()))
            self.stage = 'idle'
            return
        else:
            expected = self.owed
            if action['t'] == 'g.sell' and action['uid'] == self.owed:
                expected = 0
            elif action['t'] == 'g.move':
                to = action['to']
                occupant = next((p for p in before['own']['board'] if to['area']=='board'
                    and (p['row'],p['col']) == (to['row'],to['col'])), None)
                expected = occupant['uid'] if occupant else 0
                self.swapped |= occupant is not None
            temp = [p for p in result.observation['own']['temp'] if p]
            # Native recompute automatically drains temp into a newly freed
            # hand slot. Accept only the exact owed UID actually relocated.
            relocated = expected and not temp and any(p and p['uid'] == expected
                for area in ('hand','board') for p in result.observation['own'][area])
            if relocated:expected = 0
            if [p['uid'] for p in temp] != ([expected] if expected else []):
                raise RecoveryFailure('Native side effect invalidated short recovery path')
            if result.observation['round'] != before['round'] or result.observation['own'].get('personalChoice'):
                raise RecoveryFailure('Unexpected recovery phase or mandatory choice')
            self.owed = expected
            self.stage = 'finish' if expected else 'ready'
            if not expected and not result.observation['own']['canReady']:
                raise RecoveryFailure('Cleared temp did not restore native Ready')
        self.events.append(dict(event='attempt', action=action, accepted=result.accepted,
                                before=previous, after=self.snapshot()))


def verify_rows(model, rows, public):
    """Reconstruct native domain, attempt history and constraint from full prefix."""
    from copy import deepcopy
    from types import SimpleNamespace
    from arkguardrl.candidates import DecisionHistory, build_decision
    from arkguardrl.contracts import decision_id
    enabled = 'recovery' in getattr(model, 'planner_config', {})
    memory = history = None
    episode = None
    seen = set()
    index = 0
    for row in rows:
        if not enabled:
            if row.recovery_context.numel() or row.recovery_domain is not None:
                raise ValueError('Recovery input supplied to old contract')
            continue
        capture = row.capture
        if episode != capture['episode_id']:
            episode = capture['episode_id']
            if episode in seen:raise ValueError('Noncontiguous recovery episode')
            seen.add(episode)
            memory, history, index = RecoveryMemory(), DecisionHistory(), 0
        if row.transition.decision_index != index:
            raise ValueError('Recovery verification needs complete prefix')
        index += 1
        state = capture['observation']
        # Progress receipts remain bound by native capture/transition validation;
        # attempts and PREP64 usage are independently folded from real actions.
        history.progress = deepcopy(capture['history']['progress'])
        native = build_decision(state, public, history)
        if json.loads(native.history_json) != capture['history']:
            raise ValueError('Native attempt history replay mismatch')
        original = row.recovery_domain
        if original is None or list(native.actions) != original['actions']:
            raise ValueError('Pre-constraint native domain replay mismatch')
        constrained = memory.constrain(native, public)
        if list(constrained.actions) != capture['actions'] or not torch.equal(memory.tensor(), row.recovery_context):
            raise ValueError('Recovery support/history replay mismatch')
        action = capture['actions'][capture['index']]
        memory.note(state, action, SimpleNamespace(accepted=row.transition.accepted,
                    observation=original['successor']))
        history.attempts.setdefault(decision_id(state), set()).add(action_id(action))
        if state['phase'] == 'PREP' and action['t'] != 'g.choice':
            history.prep_used += 1
