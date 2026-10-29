"""Auxiliary learned execution of an explicit owned-roster/position contract.

STOP returns to the caller, never sends Ready. This module has no PPO integration.
All native move candidates remain available; only the teacher uses support checks.
"""
import random
import time
import torch
from torch import nn
from .pose_tasks import positions, supported_move
from arkguardrl.candidates import public_context

SCHEMA = 'arkguardrl-core/committed-position-executor/v1'
FIELDS = ('stop', 'complete', 'error_fraction', 'move', 'source_wanted',
          'source_missing', 'source_wrong', 'source_at_goal', 'destination_is_goal',
          'direction_correct', 'goal_progress', 'goal_regress', 'error_delta',
          'destination_empty', 'withdraw', 'source_board', 'buffer_witness')


def row(decision, public, goal):
    state = decision.state
    actual = positions(state)
    context = public_context(state, public)
    locations, occupied, _ = context
    before = goal.errors(state)
    indices = [i for i,a in enumerate(decision.actions) if a['t']=='g.move'] + [None]
    result = torch.zeros((len(indices),len(FIELDS)),dtype=torch.float32)
    for j,index in enumerate(indices):
        if index is None:
            result[j,:3]=torch.tensor((1.,before==0,before/max(1,len(goal.poses))))
            continue
        a=decision.actions[index]
        uid=a['uid'];to=a['to'];wanted=uid in goal.poses
        cell=(to.get('row'),to.get('col'))
        gain=before-goal.errors(state,a)
        target=goal.poses.get(uid)
        empty=to['area']=='board' and cell not in occupied
        buffer=(wanted and uid in actual and actual[uid]!=target and uid not in goal.buffered
            and (to['area']=='hand' and state['own']['hand'][to['idx']] is None
                 or empty and supported_move(state,a,public,context)))
        result[j]=torch.tensor((0.,before==0,before/max(1,len(goal.poses)),1.,wanted,
            wanted and uid not in actual,wanted and actual.get(uid)!=target,
            wanted and actual.get(uid)==target,
            wanted and to['area']=='board' and cell==target[:2],
            wanted and a.get('dir')==target[2],gain>0,gain<0,gain/max(1,len(goal.poses)),
            empty,to['area']=='hand',locations[uid][0]=='board',buffer))
    return result,indices


def teacher(decision, public, goal, indices):
    if goal.errors(decision.state)==0:
        return len(indices)-1,[len(indices)-1],'complete'
    index,reason=goal.choose(decision,public)
    if index is None or decision.actions[index]['t']!='g.move':
        return None,[],'blocked'
    label=indices.index(index)
    if reason=='layout_progress':
        context=public_context(decision.state,public)
        before=goal.errors(decision.state)
        best=before-goal.errors(decision.state,decision.actions[index])
        acceptable=[j for j,i in enumerate(indices) if i is not None
            and supported_move(decision.state,decision.actions[i],public,context)
            and before-goal.errors(decision.state,decision.actions[i])==best]
    else:
        acceptable=[label]
    return label,acceptable,reason


class Policy(nn.Module):
    def __init__(self):
        super().__init__()
        self.score=nn.Sequential(nn.Linear(len(FIELDS),48),nn.Tanh(),nn.Linear(48,1))

    def forward(self, features):
        return self.score(features).squeeze(-1).log_softmax(0)


def fit(model, rows, *, steps=1000, batch_size=8, seed=671, max_wall=120):
    if not rows or min(steps,batch_size,max_wall)<=0:
        raise ValueError('Finite auxiliary task budget required')
    optimizer=torch.optim.Adam(model.parameters(),lr=.001)
    rng=random.Random(seed)
    start=time.monotonic();history=[];done=0
    for step in range(steps):
        if time.monotonic()-start>=max_wall:
            break
        optimizer.zero_grad(set_to_none=True);total=0.
        for r in rng.choices(rows,k=batch_size):
            logp=model(r['features'])
            loss=-.5*logp[r['index']]-.5*torch.logsumexp(logp[r['acceptable']],0)
            if not torch.isfinite(loss):
                raise ValueError('Nonfinite execution loss')
            (loss/batch_size).backward();total+=float(loss.detach())/batch_size
        nn.utils.clip_grad_norm_(model.parameters(),1.,error_if_nonfinite=True)
        optimizer.step();done+=1
        if done==1 or done%100==0:
            history.append(dict(step=done,loss=total,seconds=time.monotonic()-start))
    return dict(steps=done,history=history,stop='steps' if done==steps else 'wall')


def save(path, model, metadata):
    torch.save(dict(schema=SCHEMA,fields=FIELDS,state_dict={k:v.detach().cpu()
        for k,v in model.state_dict().items()},metadata=metadata),path)


def load(path):
    payload=torch.load(path,map_location='cpu',weights_only=True)
    if payload['schema']!=SCHEMA or tuple(payload['fields'])!=FIELDS:
        raise ValueError('Unknown executor input contract')
    model=Policy();model.load_state_dict(payload['state_dict'],strict=True)
    return model.eval(),payload['metadata']
