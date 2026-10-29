"""Stable public position goals for teacher diagnostics, not learned control.

A target uses the current layout plus at most one positive public-route move.
It is deliberately a local teacher prior, not a combat optimizer. No engine
simulation, hidden state, spending rule or environment reward is used.
"""
from arkguardrl.candidates import public_context, terrain_allows, empty_deployment_allowed
from .roster_teaching import ReplacementMemory
from . import roster_tasks
from .routes.features import ACTION_FIELDS


def positions(state):
    return {p['uid']: (p['row'], p['col'], p['dir']) for p in state['own']['board']}


def after_move(state, action):
    result = positions(state)
    if action['t'] != 'g.move':
        return result
    uid = action['uid']
    old = result.pop(uid, None)
    to = action['to']
    inventory = {p['uid']: p for area in ('board', 'hand', 'temp')
                 for p in state['own'][area] if p}
    source = inventory[uid]
    lifted = set()
    if to['area'] != 'board':
        if old is not None and source['kind'] == 'chess':
            lifted.add(uid)
            if to['area'] == 'hand':
                occupant = state['own']['hand'][to['idx']]
                if occupant is not None and occupant['kind'] == 'chess':
                    result[occupant['uid']] = old
        for p in state['own']['board']:
            if p['kind'] == 'token' and p['ownerUid'] in lifted:
                result.pop(p['uid'], None)
        return result
    target = (to['row'], to['col'])
    displaced = next((other for other, at in result.items() if at[:2] == target), None)
    if displaced is not None:
        previous = result.pop(displaced)
        if old is not None:
            result[displaced] = (*old[:2], previous[2])
        if inventory[displaced]['kind'] == 'chess':
            lifted.add(displaced)
    result[uid] = (*target, action['dir'])
    if source['kind'] == 'chess' and (old is None or old[:2] != target):
        lifted.add(uid)
    for p in state['own']['board']:
        if p['kind'] == 'token' and p['ownerUid'] in lifted:
            result.pop(p['uid'], None)
    return result


def route_gain(features):
    f = dict(zip(ACTION_FIELDS, features.tolist()))
    return (3*f['delta_ground_coverage'] + 2*f['delta_air_coverage']
        + f['delta_supported_blockers'] + f['delta_damage_contact']
        + f['delta_healing_contact'])


def supported_move(state, action, public, context):
    if action['t'] != 'g.move' or action['to']['area'] != 'board':
        return False
    locations, occupied, cells = context
    area, _, source = locations[action['uid']]
    if source['kind'] != 'chess':
        return False
    to = action['to']
    cell = (to['row'], to['col'])
    if cell not in cells or not terrain_allows(source, cells[cell], public):
        return False
    dest = occupied.get(cell)
    if dest is not None and dest['uid'] != source['uid']:
        if dest['kind'] != 'chess':
            return False
        if area == 'board':
            return terrain_allows(dest, cells[(source['row'], source['col'])], public)
        return True
    if area == 'board':
        return True
    return empty_deployment_allowed(source, cell, state, public, context)


class Memory:
    def __init__(self):
        self.roster = ReplacementMemory()
        self.poses = {}
        self.generations = 0
        self.planned_gain = 0.

    def errors(self, state, action=None):
        actual = positions(state) if action is None else after_move(state, action)
        return sum(actual.get(uid) != at for uid, at in self.poses.items())

    def update(self, decision, public, route_actions):
        state = decision.state
        previous = self.roster.signature
        targets = self.roster.update(state, public)
        if previous != self.roster.signature or state['phase'] != 'PREP':
            self.poses = {}
            self.planned_gain = 0.
        if (state['phase'] != 'PREP' or self.poses or not targets
                or not roster_tasks.complete(state, targets)):
            return targets
        initial = positions(state)
        self.poses = {uid: initial[uid] for uid in targets}
        context = public_context(state, public)
        best = None
        for i, action in enumerate(decision.actions):
            if action.get('uid') not in targets or not supported_move(state, action, public, context):
                continue
            predicted = after_move(state, action)
            if not targets <= predicted.keys():
                continue
            gain = route_gain(route_actions[i])
            if gain > .02 and (best is None or gain > best[0]):
                best = (gain, predicted)
        if best is not None:
            self.planned_gain, predicted = best
            self.poses = {uid: predicted[uid] for uid in targets}
        self.generations += 1
        return targets

    def teaching_indices(self, decision, public):
        """At completion preserve the parent's relative non-disruptive choices.

        This subset is only a teacher contract. It is not a trained policy or
        inference mask for the student, and does not specifically favor Ready.
        """
        if not self.poses:
            return [], 'inactive'
        state = decision.state
        before = self.errors(state)
        if before == 0:
            return [i for i, a in enumerate(decision.actions)
                    if a['t'] != 'g.move' or self.errors(state, a) == 0], 'pose_complete'
        context = public_context(state, public)
        progress = [(before-self.errors(state, a), i) for i, a in enumerate(decision.actions)
                    if supported_move(state, a, public, context)]
        best = max((gain for gain, _ in progress), default=0)
        return [i for gain, i in progress if gain == best and gain > 0], 'execute_pose'
