"""GPL-3.0 public-history covenant loss cost at accepted end-PREP only.

Temporary restoration within PREP is free. This is an explicit non-potential
objective; round-local public high-water history is recorded in reward receipts.
"""
import math

SCHEMA='arkguardrl-core/end-prep-covenant-loss/v1'


def validate(config):
    if config is None:return
    if set(config)!={'schema','bonds','flat_per_tier','layer_weight','layer_scale','max_cost','replacement_credit'}:
        raise ValueError('Explicit end-PREP loss configuration required')
    if config['schema']!=SCHEMA or not isinstance(config['bonds'],(list,tuple)) or not config['bonds']:
        raise ValueError('Known end-PREP loss schema and bonds required')
    if len(set(config['bonds']))!=len(config['bonds']):raise ValueError('Distinct target bonds required')
    for key in ('flat_per_tier','layer_weight','max_cost','replacement_credit','layer_scale'):
        if not math.isfinite(config[key]) or config[key]<0:raise ValueError('Finite nonnegative loss coefficients required')
    if not 0<config['layer_scale'] or not 0<config['max_cost']<=.5 or config['replacement_credit']>1:
        raise ValueError('Bounded end-PREP cost/scale/credit required')


class ReadyCost:
    def __init__(self,config):
        validate(config)
        self.config=config;self.round=None;self.start={};self.peak={}

    def statuses(self,state):
        disabled=set(state['public']['disabledBonds'])
        result={key:(0,0.) for key in self.config['bonds']}
        for b in state['own']['bonds']:
            key=b['bondId']
            if key not in result or key in disabled or b.get('off') or not b.get('active'):continue
            result[key]=(max(1,int(b['tier'])),max(0.,float(b['layers'])))
        return result

    def observe(self,state):
        if state['phase']!='PREP' or state['personalEnded']:return
        now=self.statuses(state)
        if self.round!=state['round']:
            self.round=state['round'];self.start=dict(now);self.peak=dict(now)
        for key,(tier,layers) in now.items():
            old=self.peak[key]
            self.peak[key]=(max(tier,old[0]),max(layers,old[1]))

    def step(self,before,after,action,accepted):
        self.observe(before)
        receipt=None
        cost=0.
        if (accepted and before['phase']=='PREP' and action['t']=='g.ready'
                and action.get('ready',True)):
            now=self.statuses(before);cfg=self.config
            losses={};gains={}
            for key,(tier,layers) in now.items():
                peak,peak_layers=self.peak[key]
                lost=max(0,peak-tier)
                # Retaining a lower tier of the same dismantled bond is not a
                # replacement for the tier it just lost.
                gained=max(0,tier-self.start[key][0]) if lost==0 else 0
                losses[key]=lost*(cfg['flat_per_tier']+cfg['layer_weight']*math.tanh(peak_layers/cfg['layer_scale']))
                gains[key]=gained*(cfg['flat_per_tier']+cfg['layer_weight']*math.tanh(layers/cfg['layer_scale']))
            gross=math.fsum(losses.values())
            credit=cfg['replacement_credit']*math.fsum(gains.values())
            cost=min(cfg['max_cost'],max(0.,gross-credit))
            receipt=dict(round=before['round'],start=self.start.copy(),peak=self.peak.copy(),
                final=now,losses=losses,replacement_gains=gains,gross_loss=gross,
                replacement_credit=credit,cost=cost)
        if after['round']==self.round:self.observe(after)
        return cost,receipt
