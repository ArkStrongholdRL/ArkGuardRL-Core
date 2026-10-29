"""Learn token-family placement and STOP without binding stacks to transient UIDs."""
from collections import Counter
import torch
from torch import nn
from arkguardrl.candidates import public_context, empty_deployment_allowed
from .committed_executor import fit

SCHEMA = 'arkguardrl-core/summon-executor/v1'
FIELDS = ('stop', 'complete', 'error_fraction', 'move', 'token', 'wanted_family',
          'source_board', 'source_at_goal', 'destination_goal', 'direction_correct',
          'progress', 'regress', 'error_delta', 'empty', 'supported', 'withdraw',
          'progress_available', 'owner_valid', 'stack_fraction')


def family(piece):
    return piece['ownerUid'], piece['id']


def tokens(state):
    return [p for area in ('board', 'hand', 'temp') for p in state['own'][area]
            if p and p['kind'] == 'token']


def owner_signature(state):
    return tuple(sorted((p['uid'], p['id'], p['row'], p['col'], p['dir'],
                         tuple(sorted((i['uid'], i['id']) for i in p['items'])))
                        for p in state['own']['board'] if p['kind'] == 'chess'))


def management_signature(state):
    """Acquisitions renew a contract; token split/merge/movement do not."""
    inventory = [p for area in ('board', 'hand', 'temp') for p in state['own'][area] if p]
    ordinary = tuple(sorted((p['uid'], p['id'], tuple(sorted((i['uid'], i['id']) for i in p['items'])))
                            for p in inventory if p['kind'] != 'token'))
    counts = Counter()
    for p in tokens(state):
        counts[family(p)] += p.get('count', 1)
    return state['round'], state['own']['deployCap'], ordinary, tuple(sorted(counts.items()))


class Goal:
    def __init__(self, state, layout):
        self.owners = owner_signature(state)
        self.targets = Counter((p['ownerUid'], p['tokenId'], *p['cell'], p['dir'])
                               for p in layout['positions'])
        self.families = {(p['ownerUid'], p['tokenId']) for p in
                         layout['positions'] + layout['unplaced']}
        self.unplaced = layout['unplaced']

    def valid(self, state):
        return owner_signature(state) == self.owners

    def actual(self, state):
        return Counter((*family(p), p['row'], p['col'], p['dir'])
                       for p in state['own']['board']
                       if p['kind'] == 'token' and family(p) in self.families)

    def errors(self, state, action=None):
        actual = self.actual(state)
        if action is not None and action['t'] == 'g.move':
            inventory = {p['uid']: (area, p) for area in ('board', 'hand', 'temp')
                         for p in state['own'][area] if p}
            area, p = inventory[action['uid']]
            if p['kind'] == 'token' and family(p) in self.families:
                if area == 'board':
                    actual[(*family(p), p['row'], p['col'], p['dir'])] -= 1
                to = action['to']
                if to['area'] == 'board':
                    actual[(*family(p), to['row'], to['col'], action['dir'])] += 1
        return sum((self.targets - actual).values()) + sum((actual - self.targets).values())


def supported(state, action, public, context):
    if action['t'] != 'g.move':
        return False
    locations, occupied, _ = context
    area, _, p = locations[action['uid']]
    if p['kind'] != 'token':
        return False
    to = action['to']
    if to['area'] == 'hand':
        dest = state['own']['hand'][to['idx']]
        return area == 'board' and (dest is None or dest['kind'] == 'token' and family(dest) == family(p))
    if to['area'] != 'board':
        return False
    cell = to['row'], to['col']
    dest = occupied.get(cell)
    if dest is not None:
        return dest['uid'] == p['uid']  # Reorientation only; teacher does not swap owners.
    return empty_deployment_allowed(p, cell, state, public, context)


def row(decision, public, goal):
    state = decision.state
    context = public_context(state, public)
    locations, occupied, _ = context
    before = goal.errors(state)
    scale = max(1, sum(goal.targets.values()))
    indices = [i for i, a in enumerate(decision.actions) if a['t'] == 'g.move'] + [None]
    result = torch.zeros((len(indices), len(FIELDS)), dtype=torch.float32)
    gains = [before - goal.errors(state, decision.actions[i]) if i is not None else 0 for i in indices]
    support = [supported(state, decision.actions[i], public, context) if i is not None else False for i in indices]
    progress = any(g > 0 and s for g, s in zip(gains, support))
    for j, native in enumerate(indices):
        if native is None:
            result[j, :3] = torch.tensor((1., before == 0, before / scale))
            result[j, FIELDS.index('progress_available')] = progress
            result[j, FIELDS.index('owner_valid')] = goal.valid(state)
            continue
        a = decision.actions[native]
        area, _, p = locations[a['uid']]
        token = p['kind'] == 'token'
        key = family(p) if token else (None, None)
        to = a['to']
        pose = (*key, to.get('row'), to.get('col'), a.get('dir'))
        actual_pose = (*key, p.get('row'), p.get('col'), p.get('dir'))
        cell_goal = any(t[:4] == pose[:4] for t in goal.targets)
        result[j] = torch.tensor((0., before == 0, before / scale, 1., token,
            key in goal.families, area == 'board', actual_pose in goal.targets,
            cell_goal, pose in goal.targets, gains[j] > 0, gains[j] < 0,
            gains[j] / scale, to['area'] == 'board' and (to['row'], to['col']) not in occupied,
            support[j], to['area'] == 'hand', progress, goal.valid(state), p.get('count', 1) / 9))
    return result, indices


def teacher(features):
    stop = len(features) - 1
    if features[stop, FIELDS.index('complete')]:
        return stop, [stop], 'complete'
    choices = [i for i in range(stop) if features[i, FIELDS.index('supported')]
               and features[i, FIELDS.index('progress')]]
    if not choices:
        return stop, [stop], 'blocked'
    best = max(float(features[i, FIELDS.index('error_delta')]) for i in choices)
    acceptable = [i for i in choices if float(features[i, FIELDS.index('error_delta')]) >= best - 1e-6]
    return acceptable[0], acceptable, 'progress'


class Policy(nn.Module):
    def __init__(self):
        super().__init__()
        self.score = nn.Sequential(nn.Linear(len(FIELDS), 48), nn.Tanh(), nn.Linear(48, 1))

    def forward(self, features):
        return self.score(features).squeeze(-1).log_softmax(0)


def execute(session, public, model, *, mode='greedy', on_transition, max_moves=24, rng=None):
    state = session.result.observation
    layout = session.environment.summon_goal()
    goal = Goal(state, layout)
    events, labels = [], []
    for step in range(max_moves + 1):
        decision = session.decision()
        if not goal.valid(decision.state):
            return dict(status='owner_changed', events=events, layout=layout), labels
        features, indices = row(decision, public, goal)
        if len(indices) == 1 and goal.errors(decision.state):
            return dict(status='native_budget', events=events, layout=layout), labels
        if mode == 'teacher':
            index, acceptable, reason = teacher(features)
            labels.append(dict(features=features, index=index, acceptable=acceptable, stop=indices[index] is None))
        else:
            with torch.no_grad():
                logp = model(features)
                index = int(logp.argmax()) if mode == 'greedy' else int(torch.multinomial(logp.exp(), 1, generator=rng))
            reason = 'learned'
        native = indices[index]
        error = goal.errors(decision.state)
        if native is None:
            status = 'complete' if error == 0 else 'blocked_stop' if reason == 'blocked' else 'premature_stop'
            events.append(dict(stop=True, errors=error, reason=reason))
            return dict(status=status, events=events, layout=layout), labels
        if step == max_moves:
            return dict(status='task_budget', events=events, layout=layout), labels
        action = decision.actions[native]
        predicted = goal.errors(decision.state, action)
        before = decision.state
        result = session.step(action)
        on_transition(before, result)
        event = dict(stop=False, action=action, accepted=result.accepted, errors=error,
                     predicted_errors=predicted, actual_errors=goal.errors(result.observation))
        events.append(event)
        if not result.accepted:
            return dict(status='rejected', events=events, layout=layout), labels
        if result.terminated or result.truncated:
            return dict(status='native_end', events=events, layout=layout), labels
    raise RuntimeError('Bounded summon execution loop escaped')


def save(path, model, metadata):
    torch.save(dict(schema=SCHEMA, fields=FIELDS, state_dict={k:v.detach().cpu()
        for k,v in model.state_dict().items()}, metadata=metadata), path)


def load(path):
    payload = torch.load(path, map_location='cpu', weights_only=True)
    if payload['schema'] != SCHEMA or tuple(payload['fields']) != FIELDS:
        raise ValueError('Unknown summon executor contract')
    model = Policy()
    model.load_state_dict(payload['state_dict'], strict=True)
    return model.eval(), payload['metadata']
