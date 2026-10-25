"""GPL-3.0 public preparation history extension; no goal or future state.

Reward configuration is explicit in model/input identity even in unshaped eval.
Captured tensors, not mutable policy memory, are the only learner inputs.
"""
import math
import torch
from .ready_cost import ReadyCost,validate as validate_cost

SCHEMA='arkguardrl-core/public-prep-high-water-input/v1'
FIELDS=('start_tier_div3','peak_tier_div3','peak_active_layers_tanh',
        'current_tier_div3','lost_tiers_div3')
GLOBAL_FIELDS=('gross_loss_normalized','replacement_credit_normalized','ready_cost_normalized')


def validate(config):
    if not isinstance(config,dict) or set(config)!={'schema','loss_config'} or config['schema']!=SCHEMA:
        raise ValueError('Explicit public PREP history input contract required')
    validate_cost(config['loss_config'])
    if config['loss_config'] is None:raise ValueError('History projection requires explicit loss coefficients')


def dimension(config):
    validate(config)
    return len(config['loss_config']['bonds'])*len(FIELDS)+len(GLOBAL_FIELDS)


def description(config):
    validate(config)
    return dict(schema=SCHEMA,fields=FIELDS,global_fields=GLOBAL_FIELDS,
        bond_order=list(config['loss_config']['bonds']),dimension=dimension(config),
        loss_config=config['loss_config'],outside_prep='all zeros',
        reset='new episode and first live decision of a new PREP round',
        source='past and current public decision states only; no persistent goal inferred')


def tensor_from_tracker(tracker,state):
    cfg=tracker.config;receipt=tracker.project(state)
    count=len(cfg['bonds'])*len(FIELDS)+len(GLOBAL_FIELDS)
    if receipt is None:return torch.zeros(count,dtype=torch.float32)
    values=[]
    for bond in cfg['bonds']:
        start=receipt['start'][bond][0];peak,layers=receipt['peak'][bond];current=receipt['final'][bond][0]
        values.extend((start/3,peak/3,math.tanh(layers/cfg['layer_scale']),current/3,max(0,peak-current)/3))
    limit=max(1e-12,3*len(cfg['bonds'])*(cfg['flat_per_tier']+cfg['layer_weight']))
    values.extend((receipt['gross_loss']/limit,receipt['replacement_credit']/limit,receipt['cost']/cfg['max_cost']))
    value=torch.tensor(values,dtype=torch.float32)
    validate_tensor(value,dict(schema=SCHEMA,loss_config=cfg))
    return value


def validate_tensor(value,config,batch=None):
    expected=(dimension(config),) if batch is None else (batch,dimension(config))
    if not isinstance(value,torch.Tensor) or value.dtype!=torch.float32 or tuple(value.shape)!=expected:
        raise ValueError('PREP history tensor shape/dtype differs from declared contract')
    if not bool(torch.isfinite(value).all()) or bool(((value<0)|(value>1)).any()):
        raise ValueError('PREP history tensor outside normalized finite domain')


def verify_rows(model,rows):
    config=getattr(model,'specialist_config',{}).get('prep_history')
    if config is None:
        if any(row.prep_history.numel() for row in rows):raise ValueError('History rows supplied to history-free policy')
        return
    current=None;tracker=None;seen=set();index=0
    for row in rows:
        episode=row.capture['episode_id']
        if episode!=current:
            if episode in seen:raise ValueError('Noncontiguous PREP history episode')
            if row.transition.decision_index!=0:raise ValueError('History reconstruction requires complete episode prefix')
            seen.add(episode);current=episode;tracker=ReadyCost(config['loss_config']);index=0
        if row.transition.decision_index!=index:raise ValueError('PREP history decision prefix has gaps')
        index+=1
        expected=tensor_from_tracker(tracker,row.capture['observation'])
        validate_tensor(row.prep_history,config)
        if row.prep_history.device.type!='cpu' or not torch.equal(row.prep_history,expected):
            raise ValueError('Captured PREP history differs from preceding public episode states')
