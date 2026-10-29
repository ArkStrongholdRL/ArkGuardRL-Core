"""On-policy economic decisions and explicit calls to a frozen executor.

Native chess-move probability is folded into COMMIT, not executed by the manager.
PPO rows describe actual manager choices; lower actions are macro transitions.
"""
import random
import torch
from torch import nn
from arkguardrl.features import SCHEDULES

SCHEMA='arkguardrl-core/economy-and-commit-manager/v1'
KINDS=tuple(SCHEDULES)+('commit',)
STATE_DIM=17
ACTION_DIM=len(KINDS)+7


def encode(decision,public,parent_logp,targets,allow_commit,*,prep_used):
    state=decision.state;own=state['own'];shop=own['shop']
    locations={p['uid']:(area,p) for area in ('board','hand','temp') for p in own[area] if p}
    chess=[p for _,p in locations.values() if p['kind']=='chess']
    bases={p['uid']:public._data['chess'][p['id']]['baseId'] for p in chess}
    board={p['uid'] for p in own['board'] if p['kind']=='chess'}
    state_features=torch.tensor((state['round']/16,own['lp']/45,own['funds']/30,
        shop['level']/6,shop['upgradePrice']/20,shop['refreshPrice']/5,
        own['deployCount']/max(1,own['deployCap']),sum(p is None for p in own['hand'])/10,
        sum(p is not None for p in own['temp'])/4,len(chess)/20,
        sum(public._data['chess'][p['id']]['isGolden'] for p in chess)/8,
        len(targets-board)/max(1,len(targets)),float(allow_commit),
        float(own['canReady']),shop['freeRefreshes']/5,
        sum(s is not None and s['kind']=='chess' and not s['sold']
            and s['price']<=own['funds'] for s in shop['slots'])/6,
        prep_used/64),dtype=torch.float32)
    indices=[];logits=[];features=[];movement=[]
    for i,a in enumerate(decision.actions):
        kind=a['t'][2:]
        area,p=locations.get(a.get('uid'),(None,None))
        if kind=='move' and not (p and (p['kind']=='token' or area=='temp' and a['to']['area']=='hand')):
            movement.append(i)
            continue
        vector=torch.zeros(ACTION_DIM,dtype=torch.float32)
        intent=SCHEDULES[int(decision.schedule_ids[i])]
        vector[KINDS.index(intent)]=1
        offer=None
        if kind=='buy':offer=shop['slots'][a['slot']]
        elif kind=='reward':offer=shop['rewardOffer']['slots'][a['idx']]
        if offer:
            vector[-7]=offer['price']/10
            if offer['kind']=='chess':
                rec=public._data['chess'][offer['id']]
                vector[-6]=rec['tier']/6;vector[-5]=float(rec['isGolden'])
                vector[-4]=sum(b==rec['baseId'] for b in bases.values())/3
                vector[-3]=sum(bases.get(p['uid'])==rec['baseId'] for p in own['board'])/3
        elif kind=='levelUp':vector[-7]=shop['upgradePrice']/20
        elif kind=='refresh':vector[-7]=shop['refreshPrice']/5
        elif kind=='sell' and p:
            vector[-2]=float(area=='board');vector[-1]=float(a['uid'] in targets)
        indices.append(i);logits.append(parent_logp[i]);features.append(vector)
    if allow_commit and movement and targets:
        vector=torch.zeros(ACTION_DIM,dtype=torch.float32)
        vector[KINDS.index('commit')]=1;vector[-1]=len(targets-board)/max(1,len(targets))
        indices.append(None);logits.append(torch.logsumexp(parent_logp[movement],0));features.append(vector)
    if not indices:
        raise ValueError('No manager action; capacity recovery is unresolved')
    row=dict(state=state_features,actions=torch.stack(features),prior=torch.stack(logits).log_softmax(0))
    return row,indices


class Policy(nn.Module):
    def __init__(self):
        super().__init__()
        self.actor=nn.Sequential(nn.Linear(STATE_DIM+ACTION_DIM,64),nn.Tanh(),nn.Linear(64,1))
        self.critic=nn.Sequential(nn.Linear(STATE_DIM,64),nn.Tanh(),nn.Linear(64,1))
        for layer in (self.actor[-1],self.critic[-1]):
            nn.init.zeros_(layer.weight);nn.init.zeros_(layer.bias)

    def forward(self,row):
        context=row['state'].expand(len(row['prior']),-1)
        delta=self.actor(torch.cat((context,row['actions']),-1)).squeeze(-1)
        return (row['prior']+delta).log_softmax(0),self.critic(row['state']).squeeze(-1)


def update(model,optimizer,episodes,*,critic_only=False,seed=681):
    if not episodes or any(not ep or not ep[-1]['terminated'] or any(r['truncated'] for r in ep) for ep in episodes):
        raise ValueError('Only complete fresh natural manager episodes are admitted')
    rows=[]
    for episode in episodes:
        tail=0.
        for i in reversed(range(len(episode))):
            r=episode[i]
            next_value=0. if r['terminated'] else episode[i+1]['value']
            delta=r['reward']+next_value-r['value']
            decay=1. if critic_only else .95**r['settlements']
            tail=delta+(0. if r['terminated'] else decay*tail)
            rows.append(dict(**r,advantage=tail,target=tail+r['value']))
    advantages=torch.tensor([r['advantage'] for r in rows])
    advantages=(advantages-advantages.mean())/advantages.std(unbiased=False).clamp_min(1e-8)
    for r,adv in zip(rows,advantages):r['normalized_advantage']=adv
    rng=random.Random(seed);updates=0;metrics=[]
    for _ in range(8 if critic_only else 2):
        rng.shuffle(rows)
        for start in range(0,len(rows),64):
            batch=rows[start:start+64];optimizer.zero_grad(set_to_none=True)
            terms=[];kls=[]
            for r in batch:
                logp,value=model(r['inputs'])
                vloss=(value-r['target'])**2
                if critic_only:
                    loss=.5*vloss
                else:
                    ratio=(logp[r['index']]-r['logp']).exp()
                    adv=r['normalized_advantage']
                    policy=-torch.minimum(ratio*adv,ratio.clamp(.9,1.1)*adv)
                    anchor=(r['inputs']['prior'].exp()*(r['inputs']['prior']-logp)).sum()
                    entropy=-(logp.exp()*logp).sum()
                    loss=policy+.5*vloss+.05*anchor-.001*entropy
                terms.append(loss)
                kls.append((r['old_logps'].exp()*(r['old_logps']-logp)).sum())
            loss=torch.stack(terms).mean()
            if not torch.isfinite(loss):raise ValueError('Nonfinite manager loss')
            loss.backward();nn.utils.clip_grad_norm_(model.parameters(),.5,error_if_nonfinite=True)
            optimizer.step();updates+=1
            kl=float(torch.stack(kls).mean().detach())
            metrics.append(dict(loss=float(loss.detach()),old_kl=kl))
            if not critic_only and kl>.02:
                return dict(rows=len(rows),updates=updates,stop='old_kl',last=metrics[-1])
    return dict(rows=len(rows),updates=updates,stop='epochs',last=metrics[-1])


def save(path,model,metadata):
    torch.save(dict(schema=SCHEMA,state_dict={k:v.detach().cpu() for k,v in model.state_dict().items()},
        metadata=metadata),path)


def fit_preferences(model, examples, *, steps=200, lr=.003, anchor=.1):
    """Separate auxiliary pair labels; never admitted as on-policy PPO rows."""
    if not examples:
        raise ValueError('Preference examples are required')
    optimizer = torch.optim.Adam(model.actor.parameters(), lr=lr)
    for _ in range(steps):
        losses = []
        for example in examples:
            row = example['inputs']
            logp, _ = model(row)
            gap = logp[example['preferred']] - logp[example['rejected']]
            kl = (row['prior'].exp() * (row['prior'] - logp)).sum()
            losses.append(torch.nn.functional.softplus(-gap) + anchor * kl)
        loss = torch.stack(losses).mean()
        if not torch.isfinite(loss):
            raise ValueError('Nonfinite auxiliary preference loss')
        optimizer.zero_grad(set_to_none=True)
        loss.backward()
        nn.utils.clip_grad_norm_(model.actor.parameters(), .5, error_if_nonfinite=True)
        optimizer.step()
    return dict(examples=len(examples), steps=steps, loss=float(loss.detach()),
                source='separate counterfactual preferences; no PPO admission')


def load(path):
    p=torch.load(path,map_location='cpu',weights_only=True)
    if p['schema']!=SCHEMA:raise ValueError('Unknown economic manager contract')
    m=Policy();m.load_state_dict(p['state_dict'],strict=True)
    return m.eval(),p['metadata']
