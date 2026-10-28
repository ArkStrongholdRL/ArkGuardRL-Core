"""Route residual policy around the native v7 actor."""
import torch
from torch import nn
from arkguardrl import neural,learn
from arkguardrl.contracts import fingerprint
from . import features as rf

SCHEMA='arkguardrl/route-relational-policy/v2'

def contract(base):
    return {'schema':SCHEMA,'base_contract':base.to_dict(),'base_contract_id':base.contract_id,'features':rf.SCHEMA,'route_observation_dim':rf.OBS_DIM,'route_action_dim':rf.ACTION_DIM,'grid_fields':list(rf.GRID_FIELDS),'global_fields':list(rf.GLOBAL_FIELDS),'action_fields':list(rf.ACTION_FIELDS),'routing':'native-intent-then-target; residual logits','geometry':'own-field-route-geometry/v2; no enemy weights or outcomes'}

class RoutePolicy(nn.Module):
    def __init__(self,base):
        super().__init__();self.base=base
        self.route_encoder=nn.Sequential(nn.Linear(rf.OBS_DIM,64),nn.Tanh(),nn.Linear(64,32),nn.Tanh())
        self.schedule=nn.Linear(32,len(neural.features.SCHEDULES))
        self.target=nn.Sequential(nn.Linear(32+rf.ACTION_DIM,64),nn.Tanh(),nn.Linear(64,1))
        nn.init.zeros_(self.schedule.weight);nn.init.zeros_(self.schedule.bias);nn.init.zeros_(self.target[-1].weight);nn.init.zeros_(self.target[-1].bias)
    def forward(self,r):
        logp,value=self.base(*(r[k] for k in ('obs','candidates','schedules','experts')))
        h=self.route_encoder(r['route_obs']);sched=r['schedules'];delta=self.target(torch.cat((h.expand(len(sched),-1),r['route_actions']),-1)).squeeze(-1)
        masses=[];conditionals=[];groups=range(len(neural.features.SCHEDULES))
        for g in groups:
            mask=sched==g;masses.append(torch.logsumexp(logp[mask],0) if mask.any() else logp.new_tensor(-torch.inf))
        mass=torch.stack(masses);new_mass=(mass+self.schedule(h)).log_softmax(0)
        raw=logp-mass[sched]+delta;normalizers=[]
        for g in groups:
            mask=sched==g;normalizers.append(torch.logsumexp(raw[mask],0) if mask.any() else raw.new_tensor(0.))
        out=new_mass[sched]+raw-torch.stack(normalizers)[sched]
        neural._finite(out,'route policy');return out,value
    def row(self,decision,geometry,public):
        ts=neural.tensorize(decision);ro,ra=rf.encode(decision.state,decision.actions,geometry,public)
        return dict(zip(('obs','candidates','schedules','experts'),ts))|{'route_obs':ro,'route_actions':ra}
    def select(self,row,decoder):
        with torch.no_grad():
            logp,value=self(row)
            index=int(neural.NativeActorCritic.greedy_index(logp,row['schedules'])) if decoder=='greedy' else int(torch.distributions.Categorical(logits=logp).sample())
        return index,float(logp[index]),float(value)
