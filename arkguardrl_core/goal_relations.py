"""GPL-3.0 public candidate-to-option relations, without action preferences.

All flags describe current public objects/candidates. They predict no combat
outcome, query no engine legality oracle and never override the sampled action.
"""
import torch
from torch import nn
from arkguardrl.routing import SCHEDULES

SCHEMA='arkguardrl-core/public-goal-candidate-relations/v1'
FIELDS=('active','buy_target','buy_other','ordinary_triple_candidate',
        'deploy_target','withdraw_target','sell_target','equip_target',
        'displace_target','target_replace_other','free_hand_slot','free_board_slot',
        'ready_with_target_offer','refresh_with_target_offer','level_with_level_plan',
        'ready_target_not_board','pair_plan','level_plan','growth_plan','target_offer_present')
DIM=len(FIELDS)


def validate(config):
    if not isinstance(config,dict) or set(config)!={'schema','enabled','freeze_selector'}:
        raise ValueError('Explicit goal relation contract required')
    if config['schema']!=SCHEMA or type(config['enabled']) is not bool or config['freeze_selector'] is not True:
        raise ValueError('Known goal relation and fixed selector contract required')


def target_from_context(public,context):
    index=int(context[0])
    if index==0:return None
    kind,target=public.entity_ids[index-1]
    if kind!='chess' or public.entity_index(kind,target)!=index:raise ValueError('Invalid public goal identity')
    return target


def encode(state,actions,public,context):
    from .strategic_options import validate_tensor
    validate_tensor(context)
    target=target_from_context(public,context)
    result=torch.zeros((len(actions),DIM),dtype=torch.float32)
    if target is None:return result
    own=state['own'];locations={p['uid']:(area,p) for area in ('board','hand','temp') for p in own[area] if p}
    occupied={(p['row'],p['col']):p for p in own['board']}
    def base(p):
        return public.record('chess',p['id']).get('baseId',p['id']) if p and p['kind']=='chess' else None
    ordinary=sum(base(p)==target and not public.record('chess',p['id']).get('isGolden') for _,p in locations.values())
    on_board=any(base(p)==target for p in own['board'])
    offers={}
    for i,a in enumerate(actions):
        if a['t']=='g.buy':p=own['shop']['slots'][a['slot']]
        elif a['t']=='g.reward':p=own['shop']['rewardOffer']['slots'][a['idx']]
        else:continue
        if base(p)==target:offers[i]=p
    offer_present=bool(offers)
    for i,a in enumerate(actions):
        kind=a['t'];area,p=locations.get(a.get('uid'),(None,None));is_target=base(p)==target
        to=a.get('to',{});move=kind=='g.move';dest=occupied.get((to.get('row'),to.get('col'))) if to.get('area')=='board' else None
        equips=locations.get(a.get('targetUid'),(None,None))[1] if kind=='g.equip' else None
        buy=kind in ('g.buy','g.reward');target_buy=i in offers
        triple=target_buy and ordinary>=2 and not public.record('chess',offers[i]['id']).get('isGolden')
        result[i]=torch.tensor([1.,float(target_buy),float(buy and not target_buy),float(triple),
            float(move and is_target and to.get('area')=='board' and area!='board'),
            float(move and is_target and area=='board' and to.get('area')=='hand'),
            float(kind=='g.sell' and is_target),float(kind=='g.equip' and base(equips)==target),
            float(move and not is_target and base(dest)==target),
            float(move and is_target and area!='board' and dest is not None and base(dest)!=target),
            float(kind=='g.sell' and area=='hand' and not is_target),
            float(kind=='g.sell' and area=='board' and not is_target),
            float(kind=='g.ready' and offer_present),float(kind=='g.refresh' and offer_present),
            float(kind=='g.levelUp' and context[3]>.5),float(kind=='g.ready' and not on_board),
            float(context[2]),float(context[3]),float(context[4]),float(offer_present)],dtype=torch.float32)
    return result


def validate_tensor(value,count=None,batch=None):
    shape=(count,DIM) if batch is None else (batch,count,DIM)
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=shape:
        raise ValueError('Explicit goal candidate relation input required')
    if not bool(torch.isfinite(value).all()) or bool(((value!=0)&(value!=1)).any()):
        raise ValueError('Goal relations must be public binary facts')


def attach(model,config):
    validate(config)
    def head():
        module=nn.Sequential(nn.Linear(DIM,32),nn.Tanh(),nn.Linear(32,1))
        nn.init.zeros_(module[-1].weight);nn.init.zeros_(module[-1].bias)
        return module
    model.goal_economy=head();model.goal_placement=head()
    model.goal_schedule=nn.Linear(2*DIM,len(SCHEDULES))
    nn.init.zeros_(model.goal_schedule.weight);nn.init.zeros_(model.goal_schedule.bias)
    from .strategic_options import parameters
    for parameter in parameters(model):parameter.requires_grad_(False)


def adjustments(model,relations,experts,valid):
    batch,count=valid.shape
    if not model.planner_config['relations']['enabled']:relations=torch.zeros_like(relations)
    delta=torch.where(experts==0,model.goal_economy(relations).squeeze(-1),model.goal_placement(relations).squeeze(-1))
    mask=valid[...,None]
    mean=(relations*mask).sum(1)/valid.sum(1).clamp_min(1)[:,None]
    maximum=relations.masked_fill(~mask,-torch.inf).amax(1)
    return delta,model.goal_schedule(torch.cat((mean,maximum),-1))


def verify_rows(model,rows,public):
    from .strategic_options import is_search
    if is_search(getattr(model,'planner_config',{}).get('options')):
        # The parent replay verifies role facts together with requested offer,
        # receipt lineage and event memory; these cannot be inferred from the
        # flat legacy target context alone.
        from .search_options import validate_relations
        for row in rows:validate_relations(row.goal_relations,len(row.capture['actions']))
        return
    enabled='relations' in getattr(model,'planner_config',{})
    for row in rows:
        if not enabled:
            if row.goal_relations.numel():raise ValueError('Unexpected goal relation input')
            continue
        expected=encode(row.capture['observation'],row.capture['actions'],public,row.option_context)
        if row.goal_relations.device.type!='cpu' or not torch.equal(expected,row.goal_relations):
            raise ValueError('Public goal-to-action relation replay mismatch')
