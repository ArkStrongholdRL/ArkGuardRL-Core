"""Execute a fixed engine layout through public native candidate actions.

Teacher use only. No spending, roster selection, policy mask or reward shaping.
"""
from .pose_tasks import positions, after_move, supported_move
from arkguardrl.candidates import public_context


def supported_execution_move(state, action, public, context, wanted):
    if action['t']=='g.move' and action['to']['area']=='hand':
        location=context[0][action['uid']]
        return (location[0]=='board' and location[2]['kind']=='chess'
            and action['uid'] not in wanted and state['own']['hand'][action['to']['idx']] is None)
    return supported_move(state,action,public,context)


class Goal:
    def __init__(self, layout):
        if layout['missing']:
            raise ValueError('Engine could not place all requested members')
        self.poses = {p['uid']: (*p['cell'], p['dir']) for p in layout['positions']}
        self.buffered = set()

    def errors(self, state, action=None):
        actual = positions(state) if action is None else after_move(state, action)
        chess={p['uid'] for area in ('board','hand','temp')
               for p in state['own'][area] if p and p['kind']=='chess'}
        extra=len((actual.keys() & chess)-self.poses.keys())
        return sum(actual.get(uid) != at for uid, at in self.poses.items())+extra

    def choose(self, decision, public):
        state = decision.state
        ready = next((i for i,a in enumerate(decision.actions) if a['t']=='g.ready'), None)
        before = self.errors(state)
        if before == 0:
            return ready, 'layout_complete'
        context = public_context(state, public)
        choices = [(before-self.errors(state,a), i) for i,a in enumerate(decision.actions)
            if supported_execution_move(state,a,public,context,self.poses)]
        progress = [(gain,i) for gain,i in choices if gain>0]
        if progress:
            return max(progress, key=lambda pair:pair[0])[1], 'layout_progress'
        actual = positions(state)
        # Resolve a terrain-incompatible swap cycle with one public buffer move.
        # The same unit is buffered at most once; failed plans have finite exit.
        for i,a in enumerate(decision.actions):
            uid = a.get('uid')
            if (a['t']!='g.move' or uid not in self.poses or uid in self.buffered
                    or uid not in actual or actual[uid]==self.poses[uid]):
                continue
            to = a['to']
            if to['area']=='hand' and state['own']['hand'][to['idx']] is None:
                self.buffered.add(uid)
                return i, 'layout_hand_buffer'
        for gain,i in choices:
            a=decision.actions[i]
            uid=a['uid']
            cell=(a['to']['row'],a['to']['col'])
            occupied={at[:2] for at in actual.values()}
            if (gain==0 and uid not in self.buffered and uid in actual
                    and cell not in occupied and actual[uid]!=self.poses[uid]):
                self.buffered.add(uid)
                return i, 'layout_cell_buffer'
        return ready, 'layout_blocked'
