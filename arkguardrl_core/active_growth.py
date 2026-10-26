"""GPL-3.0 action-led public plan memory and separated universal actor heads.

Plans are deterministic memory of accepted native actions, not sampled latent
options or script overrides. PPO probabilities remain those of native actions.
"""
from collections import Counter
import torch
from torch import nn
from arkguardrl import structured as layout
from arkguardrl.routing import SCHEDULES
from .models import Policy

SCHEMA='arkguardrl-core/action-led-separated-growth/v1'
PLAN_SCHEMA='arkguardrl-core/public-action-led-plan/v1'
DIM=12
FIELDS=('target_catalog_index','idle','pair','level_then_merge','growth',
        'elapsed_rounds_div4','normal_copies_div3','target_on_board',
        'accepted_steps_div16','spent_div12','upgrade_since_start','pending_merge_reward')


def validate(config):
    if not isinstance(config,dict) or set(config)!={'schema','band_index','growth_anchors','max_plan_rounds','hidden'}:
        raise ValueError('Explicit action-led plan policy config required')
    if config['schema']!=SCHEMA or config['hidden']!=64 or config['max_plan_rounds']!=3:
        raise ValueError('Known bounded action-led plan representation required')
    if type(config['band_index']) is not int or not 0<config['band_index']<layout.ENTITY_VOCAB:
        raise ValueError('Explicit public fixed band index required')
    if not isinstance(config['growth_anchors'],(list,tuple)) or len(set(config['growth_anchors']))!=len(config['growth_anchors']):
        raise ValueError('Explicit distinct public anchor identities required')


def description(config):
    validate(config)
    return dict(schema=PLAN_SCHEMA,fields=FIELDS,dimension=DIM,config=config,
        source='past accepted own actions and current public inventory only',
        control='memory only; all native action choices learned, no action overrides except declared fixed band')


def validate_tensor(value,batch=None):
    shape=(DIM,) if batch is None else (batch,DIM)
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=shape:
        raise ValueError('Explicit float32 public plan input required')
    if not bool(torch.isfinite(value).all()) or bool((value<0).any()) or bool((value[:,1:]>1).any() if value.ndim==2 else (value[1:]>1).any()):
        raise ValueError('Plan memory outside finite domain')
    ids=value[:,0] if value.ndim==2 else value[0]
    if bool(((ids!=ids.round())|(ids>=layout.ENTITY_VOCAB)).any()):raise ValueError('Invalid public plan target index')


class PlanMemory:
    def __init__(self,config,public):
        validate(config);self.config=config;self.public=public
        self.target=None;self.mode='idle';self.started=0;self.steps=0;self.spent=0.;self.upgraded=False;self.pending=None
        self.events=[]

    def base(self,piece):
        return self.public.record('chess',piece['id']).get('baseId',piece['id'])

    def pieces(self,state):
        return [p for area in ('board','hand','temp') for p in state['own'][area] if p and p['kind']=='chess']

    def clear(self,reason,state):
        if self.target:self.events.append(dict(event='plan-end',reason=reason,target=self.target,round=state['round']))
        self.target=None;self.mode='idle';self.steps=0;self.spent=0.;self.upgraded=False

    def tensor(self,state):
        pieces=self.pieces(state)
        normal=Counter(self.base(p) for p in pieces if not self.public.record('chess',p['id']).get('isGolden'))
        bases={self.base(p) for p in pieces}
        if self.pending:
            action,accepted,old=self.pending;self.pending=None
            if accepted:
                acquired=None
                if action['t']=='g.buy':acquired=old['own']['shop']['slots'][action['slot']]
                elif action['t']=='g.reward':acquired=old['own']['shop']['rewardOffer']['slots'][action['idx']]
                if self.target:
                    self.steps+=1;self.spent+=max(0.,old['own']['funds']-state['own']['funds'])
                    self.upgraded|=action['t']=='g.levelUp'
                    if self.upgraded and self.mode=='pair':self.mode='level_then_merge'
                if acquired and acquired['kind']=='chess':
                    key=self.base(acquired)
                    if key in self.config['growth_anchors'] and key in bases:
                        if self.target!=key or self.mode!='growth':
                            self.clear('new-growth-acquisition',state);self.target=key;self.mode='growth';self.started=state['round']
                            self.events.append(dict(event='plan-start',mode=self.mode,target=key,round=state['round']))
                    elif self.target is None and normal[key]>=2:
                        self.target=key;self.mode='pair';self.started=state['round']
                        self.events.append(dict(event='plan-start',mode=self.mode,target=key,round=state['round']))
        if self.target and (self.target not in bases or state['round']-self.started>=self.config['max_plan_rounds']):
            self.clear('target-gone-or-round-window',state)
        offer=state['own']['shop'].get('rewardOffer') or {}
        merge_reward=offer.get('source')=='merge'
        if self.target and self.mode in ('pair','level_then_merge') and normal[self.target]<2 and not merge_reward:
            self.clear('pair-resolved-or-abandoned',state)
        values=[float(self.public.entity_index('chess',self.target)) if self.target else 0.]
        values += [float(self.mode==m) for m in ('idle','pair','level_then_merge','growth')]
        values += [min(1.,max(0,state['round']-self.started)/4) if self.target else 0.,
            min(1.,normal[self.target]/3) if self.target else 0.,
            float(any(self.base(p)==self.target for p in state['own']['board'] if p['kind']=='chess')) if self.target else 0.,
            min(1.,self.steps/16),min(1.,self.spent/12),float(self.upgraded),float(merge_reward)]
        value=torch.tensor(values,dtype=torch.float32);validate_tensor(value);return value

    def note(self,before,action,accepted):
        if self.pending is not None:raise ValueError('Plan pending action not consumed')
        self.pending=(action,accepted,before)


def verify_rows(model,rows,public):
    config=getattr(model,'planner_config',None);seen=set();current=None;memory=None;index=0
    for row in rows:
        if config is None:
            if row.plan_context.numel():raise ValueError('Plan rows supplied to plan-free actor')
            continue
        episode=row.capture['episode_id']
        if episode!=current:
            if episode in seen:raise ValueError('Noncontiguous public plan episode')
            seen.add(episode);current=episode;memory=PlanMemory(config,public);index=0
        if row.transition.decision_index!=index:raise ValueError('Plan input requires complete ordered episode prefix')
        index+=1;expected=memory.tensor(row.capture['observation'])
        if row.plan_context.device.type!='cpu' or not torch.equal(expected,row.plan_context):raise ValueError('Captured plan differs from past public actions')
        memory.note(row.capture['observation'],row.capture['actions'][row.capture['index']],row.transition.accepted)


class ActiveGrowthPolicy(Policy):
    def __init__(self,contract,definitions,width,route_backend,config):
        validate(config);super().__init__(contract,definitions,width,route_backend)
        self.planner_config=dict(config)
        self.requires_grad_(False)
        # Keep identity/low-level spatial encoding, open shared decision layers,
        # independent candidate heads, scheduler and critic from BC init.
        for module in (self.base.encoder.combine,self.base.purchase_head,self.base.placement_head,self.base.scheduler,self.base.critic):
            module.requires_grad_(True)
        context_dim=24*width+DIM-1
        candidate_dim=4*24*width+layout.ACTION_DIM-5+32+context_dim
        def head(n):return nn.Sequential(nn.Linear(n,64),nn.Tanh(),nn.Linear(64,1))
        self.plan_economy=head(candidate_dim);self.plan_placement=head(candidate_dim)
        self.plan_schedule=nn.Linear(context_dim,len(SCHEDULES));self.plan_value=head(context_dim)
        for module in (self.plan_economy,self.plan_placement,self.plan_value):
            nn.init.zeros_(module[-1].weight);nn.init.zeros_(module[-1].bias)
        nn.init.zeros_(self.plan_schedule.weight);nn.init.zeros_(self.plan_schedule.bias)

    def bootstrap_value(self,obs,plan):
        validate_tensor(plan)
        context=torch.cat((self.base.encoder.identity(plan[0].long()),plan[1:]),-1)
        return self.base.value(obs)+self.plan_value(context).squeeze(-1)

    def forward(self,row):
        logp,value=Policy.forward(self,row)
        obs=self.base.definitions.expand(row['obs'])
        adjusted,values=self.adjust_planning(logp[None],value.reshape(1),obs[None],
            row['candidates'][None],row['route_actions'][None],row['schedules'][None],row['experts'][None],
            torch.ones_like(logp,dtype=torch.bool)[None],row.get('plan_context',None)[None] if 'plan_context' in row else None)
        return adjusted[0],values[0]

    def adjust_planning(self,logp,value,tables,actions,routes,schedules,experts,valid,plan):
        batch,count=valid.shape;validate_tensor(plan,batch)
        entities=tables[:,layout.ENTITY_START:layout.BOND_START].reshape(batch,layout.MAX_ENTITIES,layout.ENTITY_DIM)
        bank=torch.cat((entities.new_zeros(batch,1,layout.ENTITY_DIM),entities),1)
        owner=torch.arange(batch,device=tables.device)[:,None,None]
        referred=bank[owner,actions[:,:,:4].long()]
        identity=self.base.encoder.identity(referred[:,:,:,1].long()).flatten(2)
        context=torch.cat((self.base.encoder.identity(plan[:,0].long()),plan[:,1:]),-1)
        inputs=torch.cat((identity,actions[:,:,5:],routes,context[:,None].expand(-1,count,-1)),-1)
        delta=torch.where(experts==0,self.plan_economy(inputs).squeeze(-1),self.plan_placement(inputs).squeeze(-1))
        groups=(schedules[:,None,:]==torch.arange(len(SCHEDULES),device=tables.device)[None,:,None])&valid[:,None,:]
        from .learning.network import group_lse
        masses,exists=group_lse(logp,groups)
        masses=masses.masked_fill(~exists,-torch.inf)
        new_mass=(masses+self.plan_schedule(context)).log_softmax(1).masked_fill(~exists,0)
        safe=masses.masked_fill(~exists,0)
        raw=logp-safe.gather(1,schedules)+delta
        norms,_=group_lse(raw,groups)
        result=(new_mass.gather(1,schedules)+raw-norms.gather(1,schedules)).masked_fill(~valid,-torch.inf)
        band_col=5+len(layout.DIRECTIONS)+layout.ACTION_TYPES.index('band')
        is_band=(actions[:,:,band_col]>.5)&valid;has_band=is_band.any(1)
        fixed=is_band&(referred[:,:,3,1].long()==self.planner_config['band_index'])
        if bool((has_band&(fixed.sum(1)!=1)).any()):raise ValueError('Fixed band absent from actual legal domain')
        forced=torch.where(fixed,result.new_zeros(()),result.new_full((),-1e9))
        result=torch.where(has_band[:,None],forced,result).masked_fill(~valid,-torch.inf)
        return result.log_softmax(1),value+self.plan_value(context).squeeze(-1)
