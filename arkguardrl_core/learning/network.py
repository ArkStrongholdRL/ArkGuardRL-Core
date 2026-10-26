"""True state-batch execution of unchanged GPL native-width route layers.

Inputs are CPU-validated by data.prepare. Learned operations have a leading state
axis; candidate and intent normalization never mix independent states.
"""
import torch
from arkguardrl import structured as layout


def pool(hidden, mask):
    mean = (hidden*mask[..., None]).sum(1)/mask.sum(1).clamp_min(1)[:, None]
    maximum = hidden.masked_fill(~mask[..., None], -torch.inf).amax(1)
    maximum = torch.where(mask.any(1)[:, None], maximum, torch.zeros_like(maximum))
    return torch.cat((mean, maximum), -1)


def group_lse(values, groups):
    present = groups.any(-1)
    masked = values[:, None, :].masked_fill(~groups, -torch.inf)
    # Empty groups get finite dummy reductions, avoiding undefined backward.
    masked = torch.where(present[..., None], masked, torch.zeros_like(masked))
    return torch.logsumexp(masked, -1), present


def expert_scores(model, inputs, data):
    """Score each valid candidate once, preserving zero gradients for absent heads.

    Adam must still see zero gradients for an unused expert: changing those to
    None would skip its momentum update and change the original optimizer rule.
    """
    raw = inputs.new_zeros(len(inputs))
    for expert, head in enumerate((model.base.purchase_head, model.base.placement_head)):
        indices = data[f'expert_{expert}']
        if indices.numel():
            raw = raw.index_copy(0, indices, head(inputs[indices]).squeeze(-1))
        else:
            zero = sum(parameter.reshape(-1)[0]*0 for parameter in head.parameters())
            raw = raw+zero
    return raw


def forward(model, data):
    enc, width = model.base.encoder, model.width
    obs, actions = data['tables'], data['candidates']
    batch = len(obs)
    state = torch.arange(batch, device=obs.device)[:, None]
    entity = obs[:, layout.ENTITY_START:layout.BOND_START].reshape(batch, layout.MAX_ENTITIES, layout.ENTITY_DIM)
    mechs = obs[:, layout.MECH_START:layout.GRID_START].reshape(batch, layout.MAX_MECHANISMS, layout.MECH_DIM)
    grid = obs[:, layout.GRID_START:layout.ENEMY_START].reshape(batch, 36, layout.GRID_DIM)
    enemies = obs[:, layout.ENEMY_START:layout.RANGE_START].reshape(batch, layout.MAX_ENEMIES, layout.ENEMY_DIM)
    ranges = obs[:, layout.RANGE_START:layout.FACT_START].reshape(batch, layout.MAX_ENTITIES, layout.MAX_RANGE, 3)
    facts = data['facts']
    owner_count = 1+layout.MAX_ENTITIES+layout.MAX_ENEMIES
    owners = facts[:, 1].long()+data['fact_state']*owner_count
    symbols = facts[:, [2, 3, 6]].long()
    types = torch.nn.functional.one_hot(facts[:, 4].long(), 5)[:, 1:].to(obs.dtype)
    fact_hidden = enc.semantic_fact(torch.cat((enc.semantic_identity(symbols).flatten(1), types, facts[:, 5:6]), -1))
    count = obs.new_zeros(batch*owner_count, 1).index_add(0, owners, obs.new_ones(len(facts), 1))
    summed = obs.new_zeros(batch*owner_count, 32*width).index_add(0, owners, fact_hidden)
    semantic = enc.semantic_owner(torch.cat((summed/count.clamp_min(1), count.log1p()/10), -1))*(count > 0)
    semantic = semantic.reshape(batch, owner_count, 32*width)
    mech_mask = mechs[:, :, 0] == 1
    mh = enc.mechanism(torch.cat((enc.mechanism_identity(mechs[:, :, 1].long()), mechs[:, :, 2:]), -1))*mech_mask[..., None]
    mh = torch.cat((mh.new_zeros(batch, 1, 32*width), mh), 1)
    refs = entity[:, :, 5:9].long()
    mechanism = mh[state[:, :, None], refs].sum(2)/(refs > 0).sum(2).clamp_min(1)[..., None]
    rh, rm = enc.range(ranges[:, :, :, 1:]), ranges[:, :, :, 0] == 1
    rmean = (rh*rm[..., None]).sum(2)/rm.sum(2).clamp_min(1)[..., None]
    rmax = rh.masked_fill(~rm[..., None], -torch.inf).amax(2)
    rmax = torch.where(rm.any(2)[..., None], rmax, torch.zeros_like(rmax))
    present = entity[:, :, 0] == 1
    hidden = enc.entity(torch.cat((enc.identity(entity[:, :, 1].long()), entity[:, :, layout.ENTITY_PREFIX:],
        mechanism, rmean, rmax, semantic[:, 1:layout.MAX_ENTITIES+1]), -1))*present[..., None]
    base = torch.cat((hidden.new_zeros(batch, 1, 64*width), hidden), 1)
    related = base[state[:, :, None], entity[:, :, 2:5].long()].flatten(2)
    hidden = enc.relations(torch.cat((hidden, related), -1))*present[..., None]
    entities = torch.cat((hidden.new_zeros(batch, 1, 64*width), hidden), 1)
    cells = torch.cat((entities[state, grid[:, :, 2].long()], grid[:, :, :2], grid[:, :, 3:]), -1)
    spatial = enc.board(cells.transpose(1, 2).reshape(batch, 64*width+4, 4, 9)).reshape(batch, 16*width, 36).transpose(1, 2)
    areas = entity[:, :, layout.ENTITY_PREFIX+len(layout.KINDS):layout.ENTITY_PREFIX+len(layout.KINDS)+len(layout.AREAS)]
    regional = torch.cat([pool(hidden, present & (areas[:, :, i] == 1)) for i in range(len(layout.AREAS))], -1)
    enemy = enc.enemy(torch.cat((enc.enemy_identity(enemies[:, :, 1].long()), enemies[:, :, 2:],
                                semantic[:, layout.MAX_ENTITIES+1:]), -1))
    global_hidden = enc.combine(torch.cat((regional, spatial.transpose(1, 2).flatten(1),
        enc.bonds(obs[:, layout.BOND_START:layout.MECH_START]), pool(enemy, enemies[:, :, 0] == 1),
        enc.global_(obs[:, :layout.GLOBAL_DIM])), -1))
    control = data['control']
    control_hidden = torch.cat((entities[state, control[:, :3].long()].flatten(1),
                               enc.identity(control[:, 3:6].long()).flatten(1), control[:, 6:]), -1)
    hidden = torch.cat((global_hidden, control_hidden), -1)
    cells = torch.cat((spatial.new_zeros(batch, 1, 16*width), spatial), 1)
    count_candidates = actions.shape[1]
    active = data['active_candidates']
    owner = data['candidate_state']
    inputs = torch.cat((hidden[owner],
        entities[owner[:, None], active[:, :4].long()].flatten(1),
        cells[owner, active[:, 4].long()], active[:, 5:]), -1)
    active_raw = expert_scores(model, inputs, data)
    raw = active_raw.new_zeros(batch*count_candidates).index_copy(
        0, data['candidate_flat_index'], active_raw).reshape(batch, count_candidates)
    groups, schedules = data['groups'], data['schedules']
    normalizers, existing = group_lse(raw, groups)
    schedule_logp = model.base.scheduler(hidden).masked_fill(~existing, -torch.inf).log_softmax(1)
    # Padding has schedule zero even when that intent is absent. Replace absent
    # masses before any padding arithmetic; no inf*0 or NaN is permitted.
    safe = schedule_logp.masked_fill(~existing, 0)
    logp = safe.gather(1, schedules)+raw-normalizers.gather(1, schedules)
    route = model.route_encoder(data['route_obs'])
    active_delta = model.target(torch.cat((route[owner], data['active_route_actions']), -1)).squeeze(-1)
    delta = active_delta.new_zeros(batch*count_candidates).index_copy(
        0, data['candidate_flat_index'], active_delta).reshape(batch, count_candidates)
    masses, _ = group_lse(logp, groups)
    new_mass = (masses.masked_fill(~existing, -torch.inf)+model.schedule(route)).log_softmax(1).masked_fill(~existing, 0)
    adjusted = logp-masses.gather(1, schedules)+delta
    normalizers, _ = group_lse(adjusted, groups)
    result = new_mass.gather(1, schedules)+adjusted-normalizers.gather(1, schedules)
    result = result.masked_fill(~data['valid'], -torch.inf)
    # Match the collector/PPO's additional per-state normalization exactly in meaning.
    value = model.base.critic(hidden).squeeze(-1)
    if hasattr(model, 'adjust_planning'):
        return model.adjust_planning(result.log_softmax(1),value,obs,actions,data['route_actions'],
            data['schedules'],data['experts'],data['valid'],data.get('plan_context'),data.get('recovery_context'))
    if hasattr(model, 'adjust_context'):
        return model.adjust_context(result.log_softmax(1), value, obs, control, actions,
            data['route_actions'], data['valid'], data['facts'], data['fact_state'], data.get('prep_history'))
    if hasattr(model, 'specialist_config'):
        return model.adjust(result.log_softmax(1), value, obs, control, actions,
                            data['route_actions'], data['valid'])
    return result.log_softmax(1), value


def entropy(logp, data):
    if 'recovery_context' in data:
        safe = logp.masked_fill(~data['valid'], 0)
        return -(logp.exp()*safe).sum(1)
    groups = data['groups']
    masses, existing = group_lse(logp, groups)
    safe = logp.masked_fill(~data['valid'], 0)
    within = torch.where(groups, safe[:, None, :]-masses[..., None], torch.zeros_like(safe[:, None, :]))
    terms = -(within.exp()*within*groups).sum(-1)
    counts = groups.sum(-1)
    conditional = torch.where(counts > 1, terms/counts.clamp_min(2).to(logp.dtype).log(), terms*0)
    schedule = -(masses.exp()*masses*existing).sum(1)
    return schedule+conditional.sum(1)/existing.sum(1)
