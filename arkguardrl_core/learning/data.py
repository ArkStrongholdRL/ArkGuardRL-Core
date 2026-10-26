"""CPU validation/static expansion, then bounded variable-candidate collation.

Only immutable inputs survive preparation; learned activations are discarded.
Derived from the native/route-width GPL model's input contracts.
"""
import torch
from arkguardrl import neural, structured as layout
from arkguardrl.neural import _indices
from ..routes import features as rf


def validate_expanded(obs, actions):
    """The encoder/control's input checks, without learned operations.

    Validate masked rows too, before semantic compaction or integer gathers.
    This replaces validation-by-forward, not the underlying input contract.
    """
    if obs.shape != (layout.OBS_DIM,) or obs.dtype != torch.float32 or obs.device.type != 'cpu':
        raise ValueError('Expanded CPU observation shape/dtype mismatch')
    neural._finite(obs, 'structured observation')
    entity = obs[layout.ENTITY_START:layout.BOND_START].reshape(layout.MAX_ENTITIES, layout.ENTITY_DIM)
    mechs = obs[layout.MECH_START:layout.GRID_START].reshape(layout.MAX_MECHANISMS, layout.MECH_DIM)
    grid = obs[layout.GRID_START:layout.ENEMY_START].reshape(36, layout.GRID_DIM)
    enemies = obs[layout.ENEMY_START:layout.RANGE_START].reshape(layout.MAX_ENEMIES, layout.ENEMY_DIM)
    ranges = obs[layout.RANGE_START:layout.FACT_START].reshape(layout.MAX_ENTITIES, layout.MAX_RANGE, 3)
    facts = obs[layout.FACT_START:layout.CONTROL_START].reshape(layout.MAX_FACTS, layout.FACT_DIM)
    for rows in (entity, mechs, enemies):
        _indices(rows[:, 0], 1, 'presence mask')
    _indices(ranges[:, :, 0], 1, 'range mask')
    _indices(entity[:, 1], layout.ENTITY_VOCAB-1, 'catalog identity')
    _indices(mechs[:, 1], layout.MECHANISM_VOCAB-1, 'mechanism identity')
    _indices(enemies[:, 1], layout.ENEMY_VOCAB-1, 'enemy identity')
    present = torch.cat((torch.ones(1, dtype=torch.bool), entity[:, 0] == 1))
    references = _indices(entity[:, 5:9], layout.MAX_MECHANISMS, 'entity mechanisms')
    mechanism_present = torch.cat((torch.ones(1, dtype=torch.bool), mechs[:, 0] == 1))
    if bool(((references > 0) & ~mechanism_present[references]).any()):
        raise ValueError('Reference to absent mechanism')
    for values, bound, name in ((entity[:, 2:5], layout.MAX_ENTITIES, 'owner/equipment'),
                                (grid[:, 2], layout.MAX_ENTITIES, 'grid occupant'),
                                (actions[:, :4], layout.MAX_ENTITIES, 'candidate entities')):
        references = _indices(values, bound, name)
        if bool(((references > 0) & ~present[references]).any()):
            raise ValueError(name+' references an absent entity')
    _indices(actions[:, 4], 36, 'candidate destination')
    _indices(facts[:, 0], 1, 'semantic presence')
    owners = _indices(facts[:, 1], layout.MAX_ENTITIES+layout.MAX_ENEMIES, 'semantic owner')
    semantic_present = torch.cat((torch.zeros(1, dtype=torch.bool), entity[:, 0] == 1, enemies[:, 0] == 1))
    if bool(((facts[:, 0] == 1) & ~semantic_present[owners]).any()):
        raise ValueError('Public semantic fact references an absent owner')
    _indices(facts[:, [2, 3, 6]], layout.SEMANTIC_VOCAB-1, 'semantic symbols')
    kinds = _indices(facts[:, 4], 4, 'semantic value type')
    if bool(((facts[:, 0] == 1) & (kinds == 0)).any()):
        raise ValueError('Present semantic fact lacks a value type')
    control = obs[layout.CONTROL_START:]
    references = _indices(control[:3], layout.MAX_ENTITIES, 'feedback entity')
    if bool(((references > 0) & ~present[references]).any()):
        raise ValueError('Feedback references an absent entity')
    _indices(control[3:6], layout.ENTITY_VOCAB-1, 'feedback catalog identity')
    nums = control[6:]
    if bool(((nums[:2] < 0) | (nums[:2] > 1)).any()) or bool(nums[0] != nums[0].round()):
        raise ValueError('Invalid execution budget context')


def prepare(model, row, *, copy_inputs=True):
    """Validate/expand inputs; optionally borrow caller-owned read-only tensors.

    Borrowing keeps the tensors alive but shares storage. The caller must retain
    exclusive mutation ownership until all collation/update work finishes.
    """
    if next(model.parameters()).device.type != 'cpu':
        raise ValueError('Prepare public inputs with the CPU actor')
    if any(value.device.type != 'cpu' for value in row.values()):
        raise ValueError('Preparation requires owned CPU inputs')
    neural.validate_tensors(*(row[k] for k in ('obs', 'candidates', 'schedules', 'experts')))
    if hasattr(model,'planner_config'):
        from ..active_growth import validate_tensor as validate_plan
        validate_plan(row.get('plan_context'))
    elif 'plan_context' in row:raise ValueError('Unexpected plan input')
    if 'recovery' in getattr(model,'planner_config',{}):
        from ..capacity_recovery import validate_tensor as validate_recovery
        validate_recovery(row.get('recovery_context'))
    elif 'recovery_context' in row:raise ValueError('Unexpected recovery input')
    history_config = getattr(model, 'specialist_config', {}).get('prep_history')
    if history_config is not None:
        from ..prep_history import validate_tensor
        validate_tensor(row.get('prep_history'), history_config)
    elif 'prep_history' in row:
        raise ValueError('Unexpected PREP history for history-free model')
    for key, shape in (('route_obs', (rf.OBS_DIM,)),
                       ('route_actions', (len(row['candidates']), rf.ACTION_DIM))):
        value = row[key]
        if value.dtype != torch.float32 or value.shape != shape:
            raise ValueError('Route input shape/dtype mismatch: '+key)
        neural._finite(value, key)
    with torch.no_grad():
        obs = model.base.definitions.expand(row['obs'])
        validate_expanded(obs, row['candidates'])
    facts = obs[layout.FACT_START:layout.CONTROL_START].reshape(layout.MAX_FACTS, layout.FACT_DIM)
    return {k: (v.detach().clone() if copy_inputs else v.detach())
            for k, v in row.items() if k != 'obs'} | {
        'tables': obs[:layout.FACT_START].clone(),
        'control': obs[layout.CONTROL_START:].clone(), 'facts': facts[facts[:, 0] == 1].clone()}


def estimated_bytes(rows):
    if not rows:
        return 0
    return (len(rows)*max(len(r['candidates']) for r in rows)*4096
            + sum(sum(v.numel()*v.element_size() for v in r.values()) for r in rows))


def split_for_memory(rows, max_bytes):
    """Preserve order and all candidates; split execution, not the PPO update."""
    chunks, current = [], []
    current_bytes, largest = 0, 0
    for row in rows:
        count = len(row['candidates'])
        size = sum(v.numel()*v.element_size() for v in row.values())
        if count*4096+size > max_bytes:
            raise ValueError('One state exceeds padded memory budget')
        if current and (len(current)+1)*max(largest, count)*4096+current_bytes+size > max_bytes:
            chunks.append(current)
            current = []
            current_bytes, largest = 0, 0
        current.append(row)
        current_bytes += size
        largest = max(largest, count)
    if current:
        chunks.append(current)
    return chunks


def collate(rows, device='cpu', max_bytes=1024**3):
    if not rows or len(rows) > 256:
        raise ValueError('Batch must contain 1..256 validated states')
    batch, count = len(rows), max(len(r['candidates']) for r in rows)
    # Bound padded candidate activations conservatively, before allocation.
    # Width2, both expert heads, backward intermediates and grouping masks.
    estimate = estimated_bytes(rows)
    if estimate > max_bytes:
        raise ValueError('Padded batch exceeds declared memory budget; no candidate truncation')
    result = {k: torch.stack([r[k] for r in rows]) for k in ('tables', 'control', 'route_obs')}
    if any('plan_context' in r for r in rows):
        if not all('plan_context' in r for r in rows):raise ValueError('Mixed plan input contracts')
        result['plan_context']=torch.stack([r['plan_context'] for r in rows])
    if any('recovery_context' in r for r in rows):
        if not all('recovery_context' in r for r in rows):raise ValueError('Mixed recovery contracts')
        result['recovery_context']=torch.stack([r['recovery_context'] for r in rows])
    if any('prep_history' in r for r in rows):
        if not all('prep_history' in r for r in rows):
            raise ValueError('Mixed PREP history contracts in batch')
        result['prep_history'] = torch.stack([r['prep_history'] for r in rows])
    if 'labels' in rows[0]:
        if not all('labels' in r and 'anchor_logps' in r for r in rows):
            raise ValueError('Mixed supervised/unsupervised batch')
        result['labels'] = torch.stack([r['labels'] for r in rows])
    for key in ('anchor_logps', 'old_logps'):
        if key in rows[0]:
            if not all(key in row and row[key].shape == (len(row['candidates']),) for row in rows):
                raise ValueError('Distribution bank shape/identity mismatch')
            bank = rows[0][key].new_zeros(batch, count)
            for i, row in enumerate(rows):
                bank[i, :len(row[key])] = row[key]
            result[key] = bank
    valid = torch.zeros(batch, count, dtype=torch.bool)
    for key in ('candidates', 'route_actions', 'schedules', 'experts'):
        example = rows[0][key]
        value = example.new_zeros((batch, count, *example.shape[1:]))
        for i, row in enumerate(rows):
            value[i, :len(row[key])] = row[key]
            valid[i, :len(row[key])] = True
        result[key] = value
    result['valid'] = valid
    # Build gather/scatter indices on CPU; no GPU nonzero/boolean-index sync.
    counts = torch.tensor([len(row['candidates']) for row in rows], dtype=torch.long)
    result['candidate_state'] = torch.repeat_interleave(torch.arange(batch), counts)
    slot = torch.cat([torch.arange(int(n)) for n in counts])
    result['candidate_flat_index'] = result['candidate_state']*count+slot
    result['active_candidates'] = torch.cat([row['candidates'] for row in rows])
    result['active_route_actions'] = torch.cat([row['route_actions'] for row in rows])
    active_experts = torch.cat([row['experts'] for row in rows])
    for expert in (0, 1):
        result[f'expert_{expert}'] = (active_experts == expert).nonzero().flatten()
    result['facts'] = torch.cat([r['facts'] for r in rows])
    result['fact_state'] = torch.cat([torch.full((len(r['facts']),), i, dtype=torch.long)
                                      for i, r in enumerate(rows)])
    result['groups'] = (torch.arange(len(neural.features.SCHEDULES))[None, :, None]
                        == result['schedules'][:, None, :]) & valid[:, None, :]
    return {k: v.to(device) for k, v in result.items()}
