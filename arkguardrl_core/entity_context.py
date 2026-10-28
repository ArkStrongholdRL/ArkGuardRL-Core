"""GPL-3.0 matched full-entity pooled-MLP / Transformer residual policies.

Both consume every visible entity, its public mechanism/semantic summaries,
ordered owner/equipment links, state, and the complete legal action domain.
No card lists, hidden simulator queries, teacher labels or reward changes.
"""
import torch
from torch import nn
from torch.nn import functional as F
from arkguardrl import structured as layout
from .models import Policy
from .specialist import SpecialistPolicy
from .routes import features as route


class Attention(nn.Module):
    def __init__(self, dim, heads):
        super().__init__()
        self.heads, self.dim = heads, dim
        self.q, self.kv, self.out = nn.Linear(dim, dim), nn.Linear(dim, 2*dim), nn.Linear(dim, dim)

    def forward(self, queries, tokens, mask):
        b, n, d = queries.shape
        q = self.q(queries).reshape(b,n,self.heads,d//self.heads).transpose(1,2)
        k,v = self.kv(tokens).reshape(b,len(tokens[0]),2,self.heads,d//self.heads).permute(2,0,3,1,4)
        result = F.scaled_dot_product_attention(q,k,v,attn_mask=mask,dropout_p=0.)
        return self.out(result.transpose(1,2).reshape(b,n,d))


class EntityBlock(nn.Module):
    def __init__(self, dim, heads, multiplier, attention):
        super().__init__()
        self.attention = Attention(dim,heads) if attention else None
        self.attention_norm = nn.LayerNorm(dim) if attention else None
        self.norm = nn.LayerNorm(dim)
        self.ff = nn.Sequential(nn.Linear(dim,multiplier*dim),nn.GELU(),nn.Linear(multiplier*dim,dim))

    def forward(self, tokens, mask):
        if self.attention is not None:
            norm = self.attention_norm(tokens)
            tokens = tokens+self.attention(norm,norm,mask)
        return tokens+self.ff(self.norm(tokens))


class EntityContextPolicy(SpecialistPolicy):
    def __init__(self, contract, definitions, width, route_backend, config):
        super().__init__(contract,definitions,width,route_backend,config)
        dim = config['hidden']
        basic_dim = layout.ENTITY_DIM-layout.ENTITY_PREFIX+24*width
        mechanism_dim = layout.MECH_DIM-2+8*width
        semantic_dim = 36*width+5
        state_dim = layout.GLOBAL_DIM+len(layout.BOND_IDS)*layout.BOND_DIM+layout.CONTROL_DIM-6
        self.specialist_entity = nn.Linear(4*basic_dim+mechanism_dim+semantic_dim,dim)
        self.specialist_state = nn.Linear(state_dim,dim)
        if 'prep_history' in config:
            from .prep_history import dimension
            self.specialist_history=nn.Linear(dimension(config['prep_history']),dim,bias=False)
            nn.init.zeros_(self.specialist_history.weight)
        self.specialist_blocks = nn.ModuleList([EntityBlock(dim,config['heads'],config['ff_multiplier'],
            config['encoder']=='transformer') for _ in range(config['layers'])])
        self.specialist_relation_bias = nn.Embedding(7,config['heads'])
        nn.init.zeros_(self.specialist_relation_bias.weight)
        self.specialist_pool = nn.Sequential(nn.Linear(2*dim,dim),nn.Tanh())
        self.specialist_query = nn.Sequential(nn.Linear(5*dim+layout.ACTION_DIM-5+route.ACTION_DIM,dim),nn.Tanh())
        self.specialist_cross = (Attention(dim,config['heads']) if config['encoder']=='transformer'
            else nn.Sequential(nn.Linear(2*dim,dim),nn.Tanh(),nn.Linear(dim,dim)))
        self.specialist_actor = nn.Sequential(nn.Linear(2*dim,dim),nn.Tanh(),nn.Linear(dim,1))
        self.specialist_value = nn.Sequential(nn.Linear(dim,dim),nn.Tanh(),nn.Linear(dim,1))
        for head in (self.specialist_actor,self.specialist_value):
            nn.init.zeros_(head[-1].weight)
            nn.init.zeros_(head[-1].bias)

    def forward(self,row):
        with torch.no_grad():
            logp,value = Policy.forward(self,row)
            obs = self.base.definitions.expand(row['obs'])
        facts = obs[layout.FACT_START:layout.CONTROL_START].reshape(layout.MAX_FACTS,layout.FACT_DIM)
        facts = facts[facts[:,0]==1]
        result,values = self.adjust_context(logp[None],value.reshape(1),obs[None],
            obs[layout.CONTROL_START:][None],row['candidates'][None],row['route_actions'][None],
            torch.ones_like(logp,dtype=torch.bool)[None],facts,
            torch.zeros(len(facts),dtype=torch.long,device=obs.device),
            row['prep_history'][None] if 'prep_history' in row else None)
        return result[0],values[0]

    def adjust_context(self,logp,value,tables,control,actions,routes,valid,facts,fact_state,prep_history=None):
        batch,count = valid.shape
        entities = tables[:,layout.ENTITY_START:layout.BOND_START].reshape(batch,layout.MAX_ENTITIES,layout.ENTITY_DIM)
        bonds = tables[:,layout.BOND_START:layout.MECH_START].reshape(batch,len(layout.BOND_IDS),layout.BOND_DIM)
        mechs = tables[:,layout.MECH_START:layout.GRID_START].reshape(batch,layout.MAX_MECHANISMS,layout.MECH_DIM)
        member_start = layout.ENTITY_PREFIX+len(layout.KINDS)+len(layout.AREAS)+len(layout.ENTITY_NUMBERS)+2*len(layout.STATS)
        bond_index = layout.BOND_IDS.index(self.specialist_config['bond'])
        phase = tables[:,len(layout.GLOBAL_FIELDS)+layout.PHASES.index('PREP')]>.5
        present = entities[:,:,0]>.5
        gate = phase & (present & (entities[:,:,member_start+bond_index]>.5)).any(1) & (bonds[:,bond_index,4]<.5)
        state = torch.cat((tables[:,:layout.GLOBAL_DIM],bonds.flatten(1),control[:,6:]),-1)
        basic = torch.cat((entities[:,:,layout.ENTITY_PREFIX:],self.base.encoder.identity(entities[:,:,1].long())),-1)
        basic = basic*present[:,:,None]
        bank = torch.cat((basic.new_zeros(batch,1,basic.shape[-1]),basic),1)
        owner = torch.arange(batch,device=tables.device)[:,None,None]
        linked = bank[owner,entities[:,:,2:5].long()].flatten(2)
        mech = torch.cat((self.base.encoder.mechanism_identity(mechs[:,:,1].long()),mechs[:,:,2:]),-1)
        mech = mech*(mechs[:,:,0]>.5)[:,:,None]
        mech_bank = torch.cat((mech.new_zeros(batch,1,mech.shape[-1]),mech),1)
        refs = entities[:,:,5:9].long()
        mechanism = mech_bank[owner,refs].sum(2)/(refs>0).sum(2).clamp_min(1)[:,:,None]
        symbols = self.base.encoder.semantic_identity(torch.stack((facts[:,2],facts[:,3],facts[:,6]),-1).long()).flatten(1)
        types = F.one_hot(facts[:,4].long(),5)[:,1:].to(tables.dtype)
        fact_features = torch.cat((symbols,types,facts[:,5:6]),-1)
        owners = fact_state*(1+layout.MAX_ENTITIES+layout.MAX_ENEMIES)+facts[:,1].long()
        total = batch*(1+layout.MAX_ENTITIES+layout.MAX_ENEMIES)
        sums = tables.new_zeros(total,fact_features.shape[-1]).index_add(0,owners,fact_features)
        nums = tables.new_zeros(total,1).index_add(0,owners,tables.new_ones(len(facts),1))
        semantics = (sums/nums.clamp_min(1)).reshape(batch,-1,fact_features.shape[-1])[:,1:layout.MAX_ENTITIES+1]
        raw = torch.cat((basic,linked,mechanism,semantics),-1)
        tokens = self.specialist_entity(raw)*present[:,:,None]
        state_token=self.specialist_state(state)
        if hasattr(self,'specialist_history'):
            state_token=state_token+self.specialist_history(prep_history)
        elif prep_history is not None:
            raise ValueError('History tensor supplied to a history-free entity policy')
        tokens = torch.cat((state_token[:,None],tokens),1)
        token_valid = torch.cat((torch.ones(batch,1,dtype=torch.bool,device=tables.device),present),1)
        key_mask = tables.new_zeros(batch,1,1,layout.MAX_ENTITIES+1).masked_fill(~token_valid[:,None,None],-torch.inf)
        if self.specialist_config['encoder']=='transformer':
            ids = torch.arange(layout.MAX_ENTITIES+1,device=tables.device)
            relation = torch.zeros(batch,layout.MAX_ENTITIES+1,layout.MAX_ENTITIES+1,dtype=torch.long,device=tables.device)
            for slot in range(3):
                pointers = entities[:,:,2+slot].long()
                edges = (pointers[:,:,None]==ids) & (pointers[:,:,None]>0) & present[:,:,None]
                relation[:,1:,:] = torch.where(edges,slot+1,relation[:,1:,:])
                relation[:,:,1:] = torch.where(edges.transpose(1,2),slot+4,relation[:,:,1:])
            bias = self.specialist_relation_bias(relation).permute(0,3,1,2)
            mask = bias+key_mask
        else:
            mask = key_mask
        for block in self.specialist_blocks:
            tokens = block(tokens,mask)*token_valid[:,:,None]
        mean = tokens.sum(1)/token_valid.sum(1)[:,None]
        maximum = tokens.masked_fill(~token_valid[:,:,None],-torch.inf).amax(1)
        context = self.specialist_pool(torch.cat((mean,maximum),-1))
        owner_action = torch.arange(batch,device=tables.device)[:,None,None]
        selected = tokens[owner_action,actions[:,:,:4].long()].flatten(2)
        queries = self.specialist_query(torch.cat((selected,context[:,None].expand(-1,count,-1),actions[:,:,5:],routes),-1))
        if self.specialist_config['encoder']=='transformer':
            attended = self.specialist_cross(queries,tokens,key_mask)
        else:
            attended = self.specialist_cross(torch.cat((queries,context[:,None].expand(-1,count,-1)),-1))
        delta = self.specialist_config['logit_bound']*self.specialist_actor(torch.cat((queries,attended),-1)).squeeze(-1).tanh()
        value_delta = self.specialist_value(context).squeeze(-1)
        raw_bank = torch.cat((entities.new_zeros(batch,1,layout.ENTITY_DIM),entities),1)
        referred = raw_bank[owner_action,actions[:,:,:4].long()]
        return self.finish(logp,value,delta,value_delta,gate,referred,actions,valid)
