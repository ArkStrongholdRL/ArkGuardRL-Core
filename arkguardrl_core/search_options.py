"""GPL-3.0 learned search-and-adapt parent options, without action overrides.

Materials are replaceable means. Actual rewards and their intended use are
separate identities. All menus use current public facts, never future offers.
"""
from collections import Counter
from types import SimpleNamespace
import math
import torch
from torch import nn
from arkguardrl import structured as layout
from arkguardrl.contracts import fingerprint
from arkguardrl.candidates import PREP_ACTION_LIMIT
from arkguardrl.routing import SCHEDULES
from .routes import features as route_features

SCHEMA = 'arkguardrl-core/search-and-adapt-options/v1'
CONFIG = dict(schema=SCHEMA, enabled=True, durations=[2,4], net_debit_limits=[12,24])
STAGES = ('idle','accumulate','realize','offer','use')
TIMINGS = ('defer','now','upgrade_first')
USES = ('undecided','reinforce','bridge','growth')
KINDS = ('idle','begin','continue','defer','now','upgrade_first','pick','undecided','reinforce','bridge','growth','exit')
FIELDS = ('merge_family','acquired_object','parent_active', *('stage_'+s for s in STAGES),
    *('timing_'+s for s in TIMINGS), *('use_'+s for s in USES),
    'parent_serial_div64','age_fraction','remaining_rounds_div4','debit_fraction',
    'upgraded_after_commit','order_violation','actual_offer','offer_tier_div6','offer_serial_div64',
    'acquired_on_board','material_gold_increase','debit_div24','remaining_debit_div24',
    'owned_paths_div128','ordinary_pairs_div128','visible_paths_div128','pending_use',
    'purpose_search','queued_rewards_div16','merges_div16','rewards_div16','direction_bond_index','closing',
    'requested_object','requested_offer_slot_div16')
ASSET_FIELDS = ('family','normal_div3','gold_div3','on_board','affordable_visible','can_complete_ordinary_triple','min_price_div24','tier_div6')
MAX_ASSETS = layout.MAX_ENTITIES
HEADER = len(FIELDS)
DIM = HEADER + MAX_ASSETS*len(ASSET_FIELDS)
MENU_FIELDS = ('family', *KINDS, 'duration_div4','limit_div24','normal_div3','visible','bond_index')
MENU_DIM = len(MENU_FIELDS)
RELATION_FIELDS = ('parent_active','buy_asset','complete_any_pair','buy_material','level_before_commit',
    'buy_requested_offer','buy_other_actual_offer','deploy_acquired','sell_acquired','equip_acquired',
    'deploy_material','sell_material','ready_pending_offer','ready_acquired_off_board',
    'accumulating','realizing','offer_stage','use_stage','direction_undecided','order_violation')
REL_DIM = len(RELATION_FIELDS)


def validate(config):
    if not isinstance(config,dict) or config != (CONFIG | {'enabled':config.get('enabled')}) or type(config.get('enabled')) is not bool:
        raise ValueError('Explicit finite search-and-adapt option contract required')


def validate_tensor(value, batch=None):
    shape=(DIM,) if batch is None else (batch,DIM)
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=shape or not bool(torch.isfinite(value).all()):
        raise ValueError('Explicit search parent/asset input required')
    flat=value.reshape(-1,DIM)
    if bool((flat<0).any()):raise ValueError('Negative search state')
    ids=torch.cat((flat[:,:2],flat[:,FIELDS.index('requested_object'):FIELDS.index('requested_object')+1],flat[:,HEADER:].reshape(len(flat),MAX_ASSETS,-1)[:,:,0]),1)
    if bool(((ids!=ids.round())|(ids>=layout.ENTITY_VOCAB)).any()):raise ValueError('Search catalog identity domain')
    direction=flat[:,FIELDS.index('direction_bond_index')]
    if bool(((direction!=direction.round())|(direction>len(layout.BOND_IDS))).any()):raise ValueError('Search direction domain')
    numeric=flat[:,2:HEADER].clone();numeric[:,FIELDS.index('direction_bond_index')-2]=0
    numeric[:,FIELDS.index('requested_object')-2]=0
    if bool((numeric>1).any()) or bool((flat[:,HEADER:].reshape(len(flat),MAX_ASSETS,-1)[:,:,1:]>1).any()):
        raise ValueError('Search normalized facts domain')


class SearchMemory:
    def __init__(self,config,planner_config,public):
        validate(config)
        self.config,self.public=config,public
        self.active=False;self.stage='idle';self.material=None;self.acquired=None;self.direction=None
        self.timing='defer';self.use='undecided';self.started=0;self.duration=0;self.limit=0.;self.debit=0.
        self.serial=0;self.offer_serial=0;self.last_offer=None;self.requested_offer=None;self.requested_object=None
        self.stream_id=None;self.requested_binding=None
        self.commit_level=0;self.upgraded=False;self.violation=False;self.gold_start=0
        self.merges=0;self.rewards=0;self.pending=[];self.receipt_serial=0;self.closing=False
        self.seen=set();self.choice_count=Counter();self.events=[];self._signal=None;self._menu=None;self.use_pending_action=False
        self._inventory_key=None;self._inventory_cache=None;self._asset_tensor=None

    def base(self,p):
        return self.public._record('chess',p['id']).get('baseId',p['id']) if p['kind']=='chess' else p['id']

    def pieces(self,state):
        return [p for area in ('board','hand','temp') for p in state['own'][area] if p]

    def inventory(self,state):
        own=state['own']
        # A bounded content key also handles distinct snapshots and in-place edits.
        # These are exactly the fields used by inventory and the asset tensor.
        key=(tuple(tuple((p['kind'],p['id']) for p in own[area] if p)
                   for area in ('board','hand','temp')),own['funds'],
             tuple((p['kind'],p['id'],p['sold'],p['price']) if p else None
                   for p in own['shop']['slots']))
        if key==self._inventory_key:return self._inventory_cache
        normal,gold,board=Counter(),Counter(),set()
        for area in ('board','hand','temp'):
            for p in own[area]:
                if not p or p['kind']!='chess':continue
                rec=self.public._record('chess',p['id'])
                family=rec.get('baseId',p['id'])
                (gold if rec.get('isGolden') else normal)[family]+=1
                if area=='board':board.add(family)
        visible={}
        for p in own['shop']['slots']:
            if p and p['kind']=='chess' and not p['sold'] and p['price']<=own['funds'] and not self.public._record('chess',p['id']).get('isGolden'):
                family=self.base(p);visible[family]=min(visible.get(family,math.inf),p['price'])
        # Keep only the latest content; memory does not grow across an episode.
        self._inventory_key=key
        self._inventory_cache=normal,gold,board,visible
        self._asset_tensor=None
        return self._inventory_cache

    def assets(self,state):
        normal,gold,board,visible=self.inventory(state)
        if self._asset_tensor is not None:return self._asset_tensor
        families=sorted(set(normal)|set(gold)|set(visible))
        if len(families)>MAX_ASSETS:raise ValueError('Public search assets exceed explicit capacity')
        rows=[]
        for family in families:
            rows.append([float(self.public.entity_index('chess',family)),min(normal[family]/3,1),
                min(gold[family]/3,1),float(family in board),float(family in visible),
                float(normal[family]>=2 and family in visible),min(visible.get(family,0)/24,1),
                min(self.public._record('chess',family).get('tier',0)/6,1)])
        rows += [[0.]*len(ASSET_FIELDS) for _ in range(MAX_ASSETS-len(rows))]
        self._asset_tensor=torch.tensor(rows,dtype=torch.float32)
        return self._asset_tensor

    def end(self,reason,state):
        if self.active:self.events.append(dict(event='search-end',parent=self.serial,reason=reason,round=state['round'],debit=self.debit))
        self.active=False;self.stage='idle';self.material=None;self.acquired=None;self.direction=None
        self.timing='defer';self.use='undecided';self.pending=[];self.closing=False;self.requested_offer=None
        self.requested_object=None;self.requested_binding=None
        self.duration=0;self.limit=0.;self.debit=0.;self.upgraded=False;self.violation=False
        self.use_pending_action=False

    def sync(self,state):
        offer=state['own']['shop'].get('rewardOffer')
        key=fingerprint({k:offer.get(k) for k in ('tier','source','label','slots')}) if offer else None
        if key!=self.last_offer:
            if key is not None:self.offer_serial+=1
            self.last_offer=key;self.requested_offer=None;self.requested_object=None;self.requested_binding=None
        if not self.active:
            self.stage='offer' if offer else 'use' if self.pending else 'idle'
            return
        self.closing |= state['round']-self.started>=self.duration or self.debit>=self.limit
        if offer:self.stage='offer'
        elif self.pending:self.stage='use'
        elif self.closing and not self.use_pending_action:self.end('deadline-or-debit',state)

    def tensor(self,state):
        self.sync(state)
        normal,gold,board,visible=self.inventory(state)
        offer=state['own']['shop'].get('rewardOffer') or {}
        def index(obj):return float(self.public.entity_index(obj['kind'],obj['family'])) if obj else 0.
        acquired=dict(kind=self.acquired['kind'],family=self.acquired['family']) if self.acquired else None
        age=max(0,state['round']-self.started)
        active=self.active
        values=[float(self.public.entity_index('chess',self.material)) if self.material else 0.,index(acquired),float(active)]
        values += [float(self.stage==s) for s in STAGES]+[float(self.timing==s) for s in TIMINGS]+[float(self.use==s) for s in USES]
        values += [min(self.serial/64,1),min(age/max(self.duration,1),1) if active else 0.,
            min(max(0,self.duration-age)/4,1) if active else 0.,min(self.debit/max(self.limit,1),1),
            float(self.upgraded),float(self.violation),float(bool(offer)),min(offer.get('tier',0)/6,1),min(self.offer_serial/64,1),
            float(bool(acquired and any(p['kind']==acquired['kind'] and self.base(p)==acquired['family'] for p in state['own']['board']))),
            float(bool(self.material and gold[self.material]>self.gold_start)),min(self.debit/24,1),min(max(0,self.limit-self.debit)/24,1),
            min(len(set(normal)|set(gold))/MAX_ASSETS,1),min(sum(n>=2 for n in normal.values())/MAX_ASSETS,1),
            min(len(visible)/MAX_ASSETS,1),float(bool(self.pending)),float(active),min(offer.get('queued',0)/16,1),
            min(self.merges/16,1),min(self.rewards/16,1),float(layout.BOND_IDS.index(self.direction)+1) if self.direction else 0.,float(self.closing),
            index(self.requested_object),float(self.requested_offer+1)/16 if self.requested_offer is not None else 0.]
        result=torch.cat((torch.tensor(values,dtype=torch.float32),self.assets(state).flatten()))
        validate_tensor(result);return result

    def menu(self,state,used):
        self.sync(state)
        if not self.config['enabled'] or state['phase']!='PREP' or state['own'].get('personalChoice') or used>=PREP_ACTION_LIMIT:return None
        normal,gold,board,visible=self.inventory(state)
        offer=state['own']['shop'].get('rewardOffer')
        signal=(state['round'],self.active,self.stage,self.serial,self.offer_serial,
            tuple(r['serial'] for r in self.pending),
            tuple(sorted(normal.items())),tuple(sorted(gold.items())),tuple(sorted(visible.items())))
        if offer:signal=(state['round'],'offer',self.offer_serial,self.serial)
        elif self.pending:signal=(state['round'],'use',self.serial,tuple(r['serial'] for r in self.pending))
        if signal in self.seen:return None
        if self.choice_count[state['round']]>=8 and not (offer or self.pending):return None
        if offer:
            menu=[dict(kind='pick',family=self.base(p),object_kind=p['kind'],slot=i,offer_serial=self.offer_serial)
                for i,p in enumerate(offer['slots']) if p and not p['sold'] and p['price']<=state['own']['funds']]
            menu += [dict(kind='continue' if self.active else 'idle',family=None)]
            if self.active:menu.append(dict(kind='exit',family=None))
        elif self.pending:
            menu=[dict(kind='exit' if self.active else 'idle',family=None)]
            for receipt in self.pending:
                for use in USES:
                    if use=='growth':
                        bonds=self.public._record('chess',receipt['family']).get('bonds',[]) if receipt['kind']=='chess' else []
                        for bond in sorted(set(bonds)-set(state['public']['disabledBonds'])):
                            if bond in layout.BOND_IDS:menu.append(dict(kind=use,family=receipt['family'],object_kind=receipt['kind'],receipt=receipt['serial'],bond=bond))
                    else:menu.append(dict(kind=use,family=receipt['family'],object_kind=receipt['kind'],receipt=receipt['serial']))
        elif not self.active:
            menu=[dict(kind='idle',family=None)]
            menu += [dict(kind='begin',family=None,duration=d,limit=l) for d,l in zip(self.config['durations'],self.config['net_debit_limits'])]
        else:
            menu=[dict(kind='continue',family=self.material),dict(kind='defer',family=None),dict(kind='exit',family=None)]
            for family in sorted(set(normal)|set(visible)):
                menu.append(dict(kind='now',family=family))
                if state['own']['shop']['level']<state['own']['shop']['maxLevel']:menu.append(dict(kind='upgrade_first',family=family))
        self._signal=signal
        self._menu=menu
        return menu

    def menu_tensor(self,menu,state):
        normal,_,_,visible=self.inventory(state)
        rows=[]
        for choice in menu:
            family=choice.get('family');kind=choice.get('object_kind','chess')
            rows.append([float(self.public.entity_index(kind,family)) if family else 0.]
                +[float(choice['kind']==k) for k in KINDS]
                +[choice.get('duration',0)/4,choice.get('limit',0)/24,min(normal.get(family,0)/3,1),float(family in visible),
                  float(layout.BOND_IDS.index(choice['bond'])+1) if choice.get('bond') else 0.])
        return torch.tensor(rows,dtype=torch.float32)

    def choose(self,choice,state):
        if self._signal is None or self._signal in self.seen or self._menu is None or choice not in self._menu:raise ValueError('Search choice lacks a fresh actual menu')
        self.seen.add(self._signal);self.choice_count[state['round']]+=1
        self._menu=None
        kind=choice['kind']
        before=dict(parent=self.serial,started=self.started,debit=self.debit,material=self.material,
            stage=self.stage,timing=self.timing,upgraded=self.upgraded)
        if not self.active and kind in ('pick',*USES):
            self.serial+=1;self.active=True;self.started=state['round']
            self.duration=self.config['durations'][0];self.limit=self.config['net_debit_limits'][0];self.debit=0.
            self.merges=0;self.rewards=0
            self.events.append(dict(event='search-adopt-actual-receipt',parent=self.serial,round=state['round']))
        if kind=='begin':
            self.serial+=1;self.active=True;self.stage='accumulate';self.started=state['round']
            self.duration=choice['duration'];self.limit=choice['limit'];self.debit=0.;self.merges=0;self.rewards=0
        elif kind=='exit':self.end('policy-exit',state)
        elif kind=='defer':self.stage='accumulate';self.material=None;self.timing='defer'
        elif kind in ('now','upgrade_first'):
            same_realization=self.stage=='realize' and self.timing==kind
            self.material=choice['family'];self.timing=kind;self.stage='realize'
            if not same_realization:
                self.commit_level=state['own']['shop']['level'];self.upgraded=False
            self.gold_start=self.inventory(state)[1][self.material]
        elif kind=='pick':
            self.requested_offer=choice['slot']
            self.requested_object=dict(kind=choice['object_kind'],family=choice['family'])
            from .offer_binding import token
            self.requested_binding=token(self,state)
        elif kind in USES:
            self.acquired=next(r for r in self.pending if r['serial']==choice['receipt'])
            self.pending=[r for r in self.pending if r['serial']!=choice['receipt']]
            self.use=kind;self.direction=choice.get('bond');self.stage='use'
            self.use_pending_action=True
            self.material=None;self.timing='defer'
        self.events.append(dict(event='search-choice',choice=choice,round=state['round'],before=before,
            parent=self.serial,started=self.started,debit=self.debit,stage=self.stage))

    def note(self,before,action,result):
        after=result.observation
        self.use_pending_action=False
        if result.accepted and self.active:
            if action['t'] in ('g.buy','g.reward','g.refresh','g.levelUp'):
                self.debit+=max(0.,before['own']['funds']-after['own']['funds'])
            self.upgraded |= action['t']=='g.levelUp' and after['own']['shop']['level']>self.commit_level
            oldgold=self.inventory(before)[1];newgold=self.inventory(after)[1]
            merged=max(0,after['own']['stats'].get('merges',0)-before['own']['stats'].get('merges',0))
            if merged:
                self.merges+=merged
                if self.material and newgold[self.material]>oldgold[self.material] and self.timing=='upgrade_first' and not self.upgraded:self.violation=True
                self.events.append(dict(event='search-merge',parent=self.serial,round=before['round'],
                    materials=sorted(k for k in newgold if newgold[k]>oldgold[k]),base_level=before['own']['shop']['level'],order_violation=self.violation))
            if action['t']=='g.ready' and action.get('ready') and self.closing:
                self.end('closing-ready',after)
        if result.accepted and action['t']=='g.reward':
            offer=before['own']['shop']['rewardOffer'];p=offer['slots'][action['idx']]
            self.receipt_serial+=1;self.rewards+=1
            receipt=dict(serial=self.receipt_serial,kind=p['kind'],family=self.base(p),
                offer_serial=self.offer_serial,source=offer.get('source'),tier=offer.get('tier'),
                actual_slot=action['idx'],requested_slot=self.requested_offer)
            self.pending.append(receipt)
            self.events.append(dict(event='search-acquired',parent=self.serial,round=before['round'],receipt=receipt))
            # An accepted pick consumes the front offer; an identical next
            # queued offer is a distinct event. Queue-length changes alone are not.
            self.last_offer=None
            self.requested_offer=None;self.requested_object=None;self.requested_binding=None
        self.sync(after)


def description(config):
    validate(config)
    return dict(schema=SCHEMA,fields=FIELDS,asset_fields=ASSET_FIELDS,max_assets=MAX_ASSETS,
        menu_fields=MENU_FIELDS,relation_fields=RELATION_FIELDS,config=config,
        control='Learned choices and native actions only; no target protection, forced purchase/deploy or reward changes',
        lifecycle='Parent debit/horizon survive material changes; actual offers/use receipts create same-PREP choices',
        boundaries='8 ordinary public choice events per round; mandatory offer/use handoffs additionally bounded by native PREP limit')


def context_features(model,value):
    batch=value.ndim==2
    validate_tensor(value,len(value) if batch else None)
    x=value if batch else value[None]
    requested=FIELDS.index('requested_object')
    roles=model.base.encoder.identity(torch.cat((x[:,:2],x[:,requested:requested+1]),1).long()).detach().flatten(1)
    assets=x[:,HEADER:].reshape(len(x),MAX_ASSETS,len(ASSET_FIELDS))
    identity=model.base.encoder.identity(assets[:,:,0].long()).detach()
    present=assets[:,:,0]>0
    pools=[]
    for weight in (assets[:,:,1],assets[:,:,4],present.float()):
        weight=weight*present
        pools.append((identity*weight[:,:,None]).sum(1)/weight.sum(1).clamp_min(1)[:,None])
    numeric=torch.cat((x[:,2:requested],x[:,requested+1:HEADER]),1)
    result=torch.cat((roles,numeric,*pools),1)
    return result if batch else result[0]


def state_features(model,inputs,previous):
    obs=model.base.definitions.expand(inputs['obs'])
    return torch.cat((obs[:layout.GLOBAL_DIM],obs[layout.BOND_START:layout.MECH_START],
        inputs['route_obs'],inputs['recovery_context'],context_features(model,previous)))


def attach(model):
    context_dim=24*model.width+11
    feature_dim=6*24*model.width+HEADER-3
    state_dim=layout.GLOBAL_DIM+(layout.MECH_START-layout.BOND_START)+route_features.OBS_DIM+8+feature_dim
    model.option_projection=nn.Linear(feature_dim,context_dim,bias=False);nn.init.zeros_(model.option_projection.weight)
    model.option_state=nn.Sequential(nn.Linear(state_dim,64),nn.Tanh())
    model.option_actor=nn.Sequential(nn.Linear(64+24*model.width+MENU_DIM-1,64),nn.Tanh(),nn.Linear(64,1))
    model.option_schedule=nn.Linear(64,len(KINDS));model.option_critic=nn.Linear(64,1)
    for layer in (model.option_actor[-1],model.option_schedule,model.option_critic):
        nn.init.zeros_(layer.weight);nn.init.zeros_(layer.bias)
    def head():
        h=nn.Sequential(nn.Linear(REL_DIM,32),nn.Tanh(),nn.Linear(32,1))
        nn.init.zeros_(h[-1].weight);nn.init.zeros_(h[-1].bias);return h
    model.search_economy=head();model.search_placement=head();model.search_schedule=nn.Linear(2*REL_DIM,len(SCHEDULES))
    nn.init.zeros_(model.search_schedule.weight);nn.init.zeros_(model.search_schedule.bias)


def distribution(model,state,menu):
    expected=model.option_state[0].in_features
    if state.shape!=(expected,) or menu.ndim!=2 or menu.shape[1]!=MENU_DIM:raise ValueError('Search selector input mismatch')
    hidden=model.option_state(state)
    identity=model.base.encoder.identity(menu[:,0].long()).detach()
    raw=model.option_actor(torch.cat((hidden.expand(len(menu),-1),identity,menu[:,1:]),-1)).squeeze(-1)
    group=menu[:,1:1+len(KINDS)].argmax(1)
    exists=torch.stack([(group==i).any() for i in range(len(KINDS))])
    mass=model.option_schedule(hidden).masked_fill(~exists,-torch.inf).log_softmax(0)
    logp=torch.stack([mass[group[i]]+raw[i]-torch.logsumexp(raw[group==group[i]],0) for i in range(len(menu))])
    return logp,model.option_critic(hidden).squeeze(-1)


def relations(state,actions,public,context,requested_slot=None):
    validate_tensor(context)
    from .native_runtime import kernel
    from arkguardrl.candidates import MAX_CANDIDATES
    if len(actions)>MAX_CANDIDATES:raise ValueError('Search relation candidate bound exceeded')
    # One tensor-to-host conversion per decision; never extract scalars per action.
    values=context.tolist()
    material,acquired=map(int,values[:2]);active=bool(values[2])
    assets=context[HEADER:].reshape(MAX_ASSETS,-1)
    # Keep the original float32 comparison against 2/3 (Python double differs).
    pairs=(assets[:,1]>=2/3).tolist()
    paths={int(values[HEADER+i*len(ASSET_FIELDS)]):pairs[i] for i in range(MAX_ASSETS)
           if values[HEADER+i*len(ASSET_FIELDS)]>0}
    locations={p['uid']:(area,p) for area in ('board','hand','temp') for p in state['own'][area] if p}
    identities={}
    def index(p):
        if not p:return 0
        identity=(p['kind'],p['id'])
        if identity in identities:return identities[identity]
        key=public._record('chess',p['id']).get('baseId',p['id']) if p['kind']=='chess' else p['id']
        result=public.entity_index(p['kind'],key)
        identities[identity]=result
        return result
    offer=state['own']['shop'].get('rewardOffer') or {}
    kinds={'g.buy':1,'g.levelUp':2,'g.reward':3,'g.move':4,'g.sell':5,'g.equip':6,'g.ready':7}
    candidates=[]
    for a in actions:
        t=a['t'];area,p=locations.get(a.get('uid'),(None,None));obj=index(p)
        bought=state['own']['shop']['slots'][a['slot']] if t=='g.buy' else offer['slots'][a['idx']] if t=='g.reward' else None
        purchased=index(bought)
        eq=index(locations.get(a.get('targetUid'),(None,None))[1]) if t=='g.equip' else 0
        candidates.append((kinds.get(t,0),obj,purchased,eq,
            bool(t=='g.move' and a['to']['area']=='board' and area!='board'),
            purchased in paths,bool(paths.get(purchased,False)),
            bool(t=='g.reward' and requested_slot is not None and a['idx']==requested_slot)))
    shared=[float(active),float(values[FIELDS.index('timing_upgrade_first')]>0),
            float(bool(offer)),float(values[FIELDS.index('acquired_on_board')]==0),
            *[values[FIELDS.index('stage_'+s)] for s in STAGES[1:]],
            values[FIELDS.index('use_undecided')],values[FIELDS.index('order_violation')]]
    result=torch.from_numpy(kernel().search_relations(candidates,material,acquired,shared))
    # Preserve the historical empty-input shape; live decisions are nonempty.
    return result if actions else result.reshape(0)


def validate_relations(value,count,batch=None):
    shape=(count,REL_DIM) if batch is None else (batch,count,REL_DIM)
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=shape or not bool(torch.isfinite(value).all()) or bool(((value!=0)&(value!=1)).any()):
        raise ValueError('Explicit search phase/candidate relation facts required')


def adjustments(model,value,experts,valid):
    batch,count=valid.shape;validate_relations(value,count,batch)
    delta=torch.where(experts==0,model.search_economy(value).squeeze(-1),model.search_placement(value).squeeze(-1))
    mean=(value*valid[:,:,None]).sum(1)/valid.sum(1).clamp_min(1)[:,None]
    maximum=value.masked_fill(~valid[:,:,None],-torch.inf).amax(1)
    return delta,model.search_schedule(torch.cat((mean,maximum),1))


def verify_rows(model,rows,public,probabilities=False):
    memory=None;episode=None;seen=set();index=choices=0
    for row in rows:
        if episode!=row.capture['episode_id']:
            episode=row.capture['episode_id']
            if episode in seen:raise ValueError('Noncontiguous search episode')
            seen.add(episode);memory=SearchMemory(model.planner_config['options'],model.planner_config,public);index=0
        if row.transition.decision_index!=index:raise ValueError('Search requires complete ordered live prefix')
        index+=1;before=row.capture['observation'];memory.stream_id=row.capture['history']['progress']['stream_id'];previous=memory.tensor(before)
        menu=memory.menu(before,row.capture['history']['used']);event=row.option_event
        if (menu is None)!=(event is None):raise ValueError('Search event boundary mismatch')
        if event is not None:
            mt=memory.menu_tensor(menu,before);raw=state_features(model,row.inputs(),previous)
            if event['schema']!=SCHEMA or menu!=event['menu'] or previous.tolist()!=event['previous_context'] or mt.tolist()!=event['menu_tensor'] or raw.tolist()!=event['state']:
                raise ValueError('Search parent/menu/assets history mismatch')
            if probabilities:
                with torch.no_grad():logp,value=distribution(model,raw,mt)
                if not torch.allclose(logp,torch.tensor(event['old_logps']),rtol=1e-5,atol=1e-5) or not math.isclose(float(value),event['value'],rel_tol=1e-5,abs_tol=1e-5):raise ValueError('Search behavior probability mismatch')
            memory.choose(menu[event['index']],before);choices+=1
        if not torch.equal(memory.tensor(before),row.option_context):raise ValueError('Search context replay mismatch')
        expected=relations(before,row.capture['actions'],public,row.option_context,memory.requested_offer)
        if row.goal_relations.device.type!='cpu' or not torch.equal(expected,row.goal_relations):raise ValueError('Search role relation replay mismatch')
        from .offer_binding import verify_row
        verify_row(model,row,memory,probabilities)
        memory.note(before,row.capture['actions'][row.capture['index']],SimpleNamespace(accepted=row.transition.accepted,observation=row.recovery_domain['successor']))
    return choices
