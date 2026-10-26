"""GPL-3.0 sampled public strategic options and separate live SMDP PPO.

Option choices are latent controller decisions, never synthetic engine actions.
Native action PPO remains conditional on the sampled persistent option. A
separate option critic uses the pre-choice public state and unmodified sums of
native rewards. These are two factorized clipped objectives, not joint-clipped
PPO or a learned world model.
"""
from collections import Counter
import math
import torch
from torch import nn
from arkguardrl import structured as layout
from arkguardrl.candidates import PREP_ACTION_LIMIT
from .routes import features as route_features

SCHEMA = 'arkguardrl-core/public-strategic-options/v1'
DIM = 12
MENU_DIM = 11
FIELDS = ('target_catalog_index','idle','pair','level_then_merge','growth',
          'remaining_rounds_div2','remaining_net_debit_fraction','normal_copies_div3',
          'on_board','upgrade_since_start','golden_increase','age_rounds_div2')
MODES = ('idle','pair','level_then_merge','growth')
MENU_FIELDS = ('target_catalog_index','idle','pair','level_then_merge','growth',
               'continue','duration_div2','net_debit_limit_div24',
               'normal_copies_div3','owned','on_board')
STATE_DIM = layout.GLOBAL_DIM + (layout.MECH_START-layout.BOND_START) + route_features.OBS_DIM + 8 + DIM


def validate(config):
    if not isinstance(config,dict) or set(config) != {'schema','enabled','durations','net_debit_limits'}:
        raise ValueError('Explicit strategic option contract required')
    if config['schema'] != SCHEMA or type(config['enabled']) is not bool or config['durations'] != [1,2] or config['net_debit_limits'] != [12,24]:
        raise ValueError('Unknown finite strategic option contract')


def validate_tensor(value, batch=None):
    shape=(DIM,) if batch is None else (batch,DIM)
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=shape or not bool(torch.isfinite(value).all()):
        raise ValueError('Explicit finite strategic option input required')
    if bool((value<0).any()) or bool((value[...,1:]>1).any()):
        raise ValueError('Strategic option state out of domain')
    ids=value[...,0]
    if bool(((ids!=ids.round()) | (ids>=layout.ENTITY_VOCAB)).any()):
        raise ValueError('Invalid option target identity')


class OptionMemory:
    def __init__(self, config, planner_config, public):
        validate(config)
        self.config,self.planner_config,self.public=config,planner_config,public
        self.mode='idle';self.target=None;self.started=0;self.duration=0
        self.limit=0.;self.debit=0.;self.upgraded=False;self.initial_gold=0
        self.chosen_round=None;self.was_owned=False;self.events=[]

    def base(self,p):
        return self.public.record('chess',p['id']).get('baseId',p['id'])

    def inventory(self,state):
        pieces=[p for area in ('board','hand','temp') for p in state['own'][area] if p and p['kind']=='chess']
        normal=Counter(self.base(p) for p in pieces if not self.public.record('chess',p['id']).get('isGolden'))
        gold=Counter(self.base(p) for p in pieces if self.public.record('chess',p['id']).get('isGolden'))
        owned={self.base(p) for p in pieces}
        board={self.base(p) for p in state['own']['board'] if p['kind']=='chess'}
        return normal,gold,owned,board

    def end(self,reason,state):
        if self.target is not None:
            self.events.append(dict(event='option-end',reason=reason,mode=self.mode,target=self.target,
                round=state['round'],net_debit_proxy=self.debit,started=self.started))
        self.mode='idle';self.target=None;self.duration=0;self.limit=0.;self.debit=0.;self.upgraded=False

    def sync(self,state):
        if not self.target:return
        normal,gold,owned,_=self.inventory(state)
        if gold[self.target]>self.initial_gold and self.mode in ('pair','level_then_merge'):
            self.end('merge-realized' if self.mode=='pair' or self.upgraded else 'merged-before-level',state)
        elif self.was_owned and self.target not in owned:self.end('target-abandoned',state)
        elif state['round']-self.started>=self.duration:self.end('round-horizon',state)
        elif self.debit>=self.limit:self.end('net-debit-limit',state)
        if self.target:self.was_owned |= self.target in owned

    def tensor(self,state):
        self.sync(state)
        normal,gold,_,board=self.inventory(state)
        if not self.target:
            result=[0.,1.,0.,0.,0.,0.,0.,0.,0.,0.,0.,0.]
        else:
            age=max(0,state['round']-self.started)
            result=[float(self.public.entity_index('chess',self.target))]+[float(self.mode==m) for m in MODES]
            result += [max(0.,self.duration-age)/2,max(0.,1-self.debit/self.limit),
                min(1.,normal[self.target]/3),float(self.target in board),float(self.upgraded),
                float(gold[self.target]>self.initial_gold),min(1.,age/2)]
        tensor=torch.tensor(result,dtype=torch.float32);validate_tensor(tensor);return tensor

    def menu(self,state,used):
        self.sync(state)
        if (not self.config['enabled'] or state['phase']!='PREP' or state['own'].get('personalChoice')
                or used>=PREP_ACTION_LIMIT or self.chosen_round==state['round']):return None
        normal,_,owned,board=self.inventory(state)
        menu=[]
        if self.target:
            menu.append(dict(mode=self.mode,target=self.target,continue_plan=True,
                duration=self.duration,limit=self.limit))
        menu.append(dict(mode='idle',target=None,continue_plan=False,duration=0,limit=0))
        targets={('pair',t) for t,n in normal.items() if n>=1}
        if state['own']['shop']['level']<state['own']['shop']['maxLevel']:
            targets |= {('level_then_merge',t) for t,n in normal.items() if n>=2}
        anchors=set(self.planner_config['growth_anchors'])
        visible=set()
        shop=state['own']['shop']
        for p in [*shop['slots'],*((shop.get('rewardOffer') or {}).get('slots',[]))]:
            if p and p['kind']=='chess' and not p['sold'] and p['price']<=state['own']['funds']:
                visible.add(self.base(p))
        targets |= {('growth',t) for t in anchors & (owned|visible)}
        for mode,target in sorted(targets):
            for duration,limit in zip(self.config['durations'],self.config['net_debit_limits']):
                menu.append(dict(mode=mode,target=target,continue_plan=False,duration=duration,limit=limit))
        return menu

    def menu_tensor(self,menu,state):
        normal,_,owned,board=self.inventory(state)
        rows=[]
        for choice in menu:
            target=choice['target']
            rows.append([float(self.public.entity_index('chess',target)) if target else 0.]
                +[float(choice['mode']==m) for m in MODES]
                +[float(choice['continue_plan']),choice['duration']/2,choice['limit']/24,
                  min(1.,normal[target]/3),float(target in owned),float(target in board)])
        return torch.tensor(rows,dtype=torch.float32)

    def choose(self,choice,state):
        self.chosen_round=state['round']
        if choice['continue_plan']:
            if choice['target']!=self.target:raise ValueError('Continue target mismatch')
            self.events.append(dict(event='option-continue',target=self.target,mode=self.mode,round=state['round']))
            return
        self.end('policy-exit-or-switch',state)
        if choice['mode']=='idle':
            self.events.append(dict(event='option-idle',round=state['round']));return
        self.mode=choice['mode'];self.target=choice['target'];self.started=state['round']
        self.duration=choice['duration'];self.limit=choice['limit'];self.debit=0.;self.upgraded=False
        _,gold,owned,_=self.inventory(state);self.initial_gold=gold[self.target];self.was_owned=self.target in owned
        self.events.append(dict(event='option-start',mode=self.mode,target=self.target,round=state['round'],
            duration=self.duration,net_debit_limit=self.limit))

    def note(self,before,action,result):
        if self.target and result.accepted:
            if action['t'] in ('g.buy','g.reward','g.refresh','g.levelUp'):
                self.debit += max(0.,before['own']['funds']-result.observation['own']['funds'])
            self.upgraded |= action['t']=='g.levelUp' and result.observation['own']['shop']['level']>before['own']['shop']['level']
        self.sync(result.observation)


def state_features(model,inputs,previous):
    validate_tensor(previous)
    obs=model.base.definitions.expand(inputs['obs'])
    result=torch.cat((obs[:layout.GLOBAL_DIM],obs[layout.BOND_START:layout.MECH_START],
        inputs['route_obs'],inputs['recovery_context'],previous))
    if result.shape!=(STATE_DIM,) or not bool(torch.isfinite(result).all()):raise ValueError('Option public state shape')
    return result


def attach(model):
    context_dim=24*model.width+11
    model.option_projection=nn.Linear(context_dim,context_dim,bias=False)
    nn.init.zeros_(model.option_projection.weight)
    model.option_state=nn.Sequential(nn.Linear(STATE_DIM,64),nn.Tanh())
    model.option_actor=nn.Sequential(nn.Linear(64+24*model.width+MENU_DIM-1,64),nn.Tanh(),nn.Linear(64,1))
    model.option_schedule=nn.Linear(64,5)
    nn.init.zeros_(model.option_schedule.weight);nn.init.zeros_(model.option_schedule.bias)
    model.option_critic=nn.Linear(64,1)
    nn.init.zeros_(model.option_actor[-1].weight);nn.init.zeros_(model.option_actor[-1].bias)
    nn.init.zeros_(model.option_critic.weight);nn.init.zeros_(model.option_critic.bias)


def distribution(model,state,menu):
    if state.shape!=(STATE_DIM,) or menu.ndim!=2 or menu.shape[1]!=MENU_DIM:
        raise ValueError('Option distribution input mismatch')
    hidden=model.option_state(state)
    identity=model.base.encoder.identity(menu[:,0].long()).detach()
    raw=model.option_actor(torch.cat((hidden.expand(len(menu),-1),identity,menu[:,1:]),-1)).squeeze(-1)
    group=torch.where(menu[:,5]>.5,0,torch.argmax(menu[:,1:5],dim=1)+1)
    exists=torch.stack([(group==i).any() for i in range(5)])
    mass=model.option_schedule(hidden).masked_fill(~exists,-torch.inf).log_softmax(0)
    logp=torch.stack([mass[group[i]]+raw[i]-torch.logsumexp(raw[group==group[i]],0) for i in range(len(menu))])
    return logp,model.option_critic(hidden).squeeze(-1)


def verify_rows(model,rows,public,probabilities=False):
    config=getattr(model,'planner_config',{}).get('options')
    memory=None;episode=None;seen=set();index=0;choices=0
    from types import SimpleNamespace
    for row in rows:
        if config is None:
            if row.option_context.numel() or row.option_event is not None:raise ValueError('Unexpected option rows')
            continue
        if episode!=row.capture['episode_id']:
            episode=row.capture['episode_id']
            if episode in seen:raise ValueError('Noncontiguous option episode')
            seen.add(episode);memory=OptionMemory(config,model.planner_config,public);index=0
        if row.transition.decision_index!=index:raise ValueError('Options require complete live prefix')
        index+=1;before=row.capture['observation'];previous=memory.tensor(before)
        menu=memory.menu(before,row.capture['history']['used'])
        event=row.option_event
        if (menu is None)!=(event is None):raise ValueError('Option decision schedule mismatch')
        if event is not None:
            expected_menu=memory.menu_tensor(menu,before)
            if event['menu']!=menu or event['previous_context']!=previous.tolist() or event['menu_tensor']!=expected_menu.tolist():
                raise ValueError('Option public menu/history mismatch')
            raw=state_features(model,row.inputs(),previous)
            if raw.tolist()!=event['state']:raise ValueError('Option state replay mismatch')
            if probabilities:
                with torch.no_grad():logp,value=distribution(model,raw,expected_menu)
                if not torch.allclose(logp,torch.tensor(event['old_logps']),rtol=1e-5,atol=1e-5) or not math.isclose(float(value),event['value'],rel_tol=1e-5,abs_tol=1e-5):
                    raise ValueError('Option probability/value replay mismatch')
            memory.choose(menu[event['index']],before);choices+=1
        if not torch.equal(memory.tensor(before),row.option_context):raise ValueError('Persistent option input replay mismatch')
        memory.note(before,row.capture['actions'][row.capture['index']],SimpleNamespace(
            accepted=row.transition.accepted,observation=row.recovery_domain['successor']))
    return choices


def option_samples(model,rows,lam=.95):
    """Complete-live SMDP transitions; terminal zero, no archived-row adapter."""
    samples=[];episodes={}
    for row in rows:episodes.setdefault(row.capture['episode_id'],[]).append(row)
    for episode,group in episodes.items():
        if not group[-1].transition.terminated or group[-1].transition.truncated:
            raise ValueError('Only complete natural option episodes admitted')
        starts=[i for i,r in enumerate(group) if r.option_event is not None]
        current=[]
        for k,start in enumerate(starts):
            end=starts[k+1] if k+1<len(starts) else len(group)
            event=group[start].option_event
            reward=math.fsum(r.transition.reward for r in group[start:end])
            battles=sum(r.battle_count for r in group[start:end])
            next_value=group[end].option_event['value'] if end<len(group) else 0.
            current.append(dict(event=event,reward=reward,battles=battles,next_value=next_value,
                episode=episode,start=start,end=end))
        tail=0.
        for item in reversed(current):
            value=item['event']['value']
            tail=item['reward']+item['next_value']-value+(lam**item['battles'])*tail
            item['advantage']=tail;item['return']=tail+value
        samples.extend(current)
    return samples


def parameters(model):
    for name,p in model.named_parameters():
        if name.startswith(('option_state.','option_actor.','option_schedule.','option_critic.')):yield p


def update(ctx,model,optimizer,rows,revision,path,rng,epochs=2,batch_size=32):
    from .sampling import verify_live_metadata
    verify_live_metadata(ctx,rows,revision,path)
    verify_rows(model,rows,ctx.public,probabilities=True)
    if ctx.objective_contract.gamma!=1.:raise ValueError('Strategic option credit requires gamma1')
    samples=option_samples(model,rows,ctx.training['gae_lambda'])
    if not samples:return dict(choices=0,updates=0)
    advantages=torch.tensor([s['advantage'] for s in samples],dtype=torch.float32)
    advantages=(advantages-advantages.mean())/advantages.std(unbiased=False).clamp_min(1e-8)
    metrics=[]
    for epoch in range(epochs):
        order=list(range(len(samples)));rng.shuffle(order)
        for start in range(0,len(order),batch_size):
            optimizer.zero_grad(set_to_none=True);terms=[];kl=[];entropy=[]
            selected=order[start:start+batch_size]
            for index in selected:
                sample=samples[index];event=sample['event']
                logp,value=distribution(model,torch.tensor(event['state']),torch.tensor(event['menu_tensor']))
                old=torch.tensor(event['old_logps']);chosen=event['index']
                ratio=(logp[chosen]-old[chosen]).exp()
                policy=-torch.minimum(ratio*advantages[index],ratio.clamp(.9,1.1)*advantages[index])
                terms.append(policy+.5*.5*(value-sample['return']).square())
                kl.append((old.exp()*(old-logp)).sum());entropy.append(-(logp.exp()*logp).sum())
            loss=torch.stack(terms).mean()
            if not torch.isfinite(loss):raise ValueError('Nonfinite option PPO')
            if float(torch.stack(kl).mean().detach())>1.5*ctx.training['target_old_kl']:
                return dict(choices=len(samples),updates=len(metrics),early_stop=True,metrics=metrics)
            loss.backward();norm=nn.utils.clip_grad_norm_(list(parameters(model)),.5,error_if_nonfinite=True)
            optimizer.step()
            metrics.append(dict(loss=float(loss.detach()),old_kl=float(torch.stack(kl).mean().detach()),
                entropy=float(torch.stack(entropy).mean().detach()),gradient_norm=float(norm)))
    return dict(choices=len(samples),updates=len(metrics),early_stop=False,metrics=metrics,
        credit='gamma1 native reward sum; lambda per observed settlement; separate pre-choice critic')
