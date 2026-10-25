"""CPU validation/static expansion, then bounded variable-candidate collation.

Only immutable inputs survive preparation; learned activations are discarded.
Derived from the native/route-width GPL model's input contracts.
"""
import torch
from arkguardrl import neural, structured as layout
from arkguardrl.neural import _indices
from ..routes import features as rf


def prepare(model, row):
    if next(model.parameters()).device.type != 'cpu':
        raise ValueError('Prepare public inputs with the CPU actor')
    if any(value.device.type != 'cpu' for value in row.values()):
        raise ValueError('Preparation requires owned CPU inputs')
    neural.validate_tensors(*(row[k] for k in ('obs', 'candidates', 'schedules', 'experts')))
    for key, shape in (('route_obs', (rf.OBS_DIM,)),
                       ('route_actions', (len(row['candidates']), rf.ACTION_DIM))):
        value = row[key]
        if value.dtype != torch.float32 or value.shape != shape:
            raise ValueError('Route input shape/dtype mismatch: '+key)
        neural._finite(value, key)
    with torch.no_grad():
        obs = model.base.definitions.expand(row['obs'])
        # Reuse the complete original validation, including masked fact indices,
        # presence, references and execution budgets. No activations are cached.
        _, entities, _, present = model.base.encoder.tables(obs)
        model.base.control(obs, entities, present)
        refs = _indices(row['candidates'][:, :4], layout.MAX_ENTITIES, 'candidate entities')
        _indices(row['candidates'][:, 4], 36, 'candidate destination')
        if bool(((refs > 0) & ~present[refs]).any()):
            raise ValueError('Candidate references an absent entity')
    facts = obs[layout.FACT_START:layout.CONTROL_START].reshape(layout.MAX_FACTS, layout.FACT_DIM)
    return {k: v.detach().clone() for k, v in row.items() if k != 'obs'} | {
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
    for row in rows:
        if estimated_bytes([row]) > max_bytes:
            raise ValueError('One state exceeds padded memory budget')
        if current and estimated_bytes(current+[row]) > max_bytes:
            chunks.append(current)
            current = []
        current.append(row)
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
    if 'labels' in rows[0]:
        if not all('labels' in r and 'anchor_logps' in r for r in rows):
            raise ValueError('Mixed supervised/unsupervised batch')
        result['labels'] = torch.stack([r['labels'] for r in rows])
        anchor = rows[0]['anchor_logps'].new_zeros(batch, count)
        for i, row in enumerate(rows):
            anchor[i, :len(row['anchor_logps'])] = row['anchor_logps']
        result['anchor_logps'] = anchor
    valid = torch.zeros(batch, count, dtype=torch.bool)
    for key in ('candidates', 'route_actions', 'schedules', 'experts'):
        example = rows[0][key]
        value = example.new_zeros((batch, count, *example.shape[1:]))
        for i, row in enumerate(rows):
            value[i, :len(row[key])] = row[key]
            valid[i, :len(row[key])] = True
        result[key] = value
    result['valid'] = valid
    result['facts'] = torch.cat([r['facts'] for r in rows])
    result['fact_state'] = torch.cat([torch.full((len(r['facts']),), i, dtype=torch.long)
                                      for i, r in enumerate(rows)])
    result['groups'] = (torch.arange(len(neural.features.SCHEDULES))[None, :, None]
                        == result['schedules'][:, None, :]) & valid[:, None, :]
    return {k: v.to(device) for k, v in result.items()}
