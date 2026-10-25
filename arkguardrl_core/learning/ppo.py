"""Live-only PPO minibatches; identical objective, shuffle and update budget.

The CPU actor is copied to an explicitly selected learner device per iteration.
Optimizer state returns to CPU; no stale actor or learned activation cache.
"""
import copy
import math
import time
import torch
from .. import sampling as worker
from . import data, network
from ..device import available, synchronize

EXECUTION_SCHEMA = "arkguardrl-core/live-batch-ppo/v1"


def tensors_to(value, device):
    if isinstance(value, torch.Tensor):
        return value.detach().to(device).clone()
    if isinstance(value, dict):
        return {k: tensors_to(v, device) for k, v in value.items()}
    if isinstance(value, list):
        return [tensors_to(v, device) for v in value]
    if isinstance(value, tuple):
        return tuple(tensors_to(v, device) for v in value)
    return value


def objective(model, packed, beta, config):
    logp, value = model.forward_batch(packed)
    labels = packed['labels']
    chosen, old, advantages, returns = labels[:, 0].long(), labels[:, 1], labels[:, 2], labels[:, 3]
    log_ratio = logp.gather(1, chosen[:, None]).squeeze(1)-old
    ratio = log_ratio.exp()
    policy_loss = -torch.minimum(ratio*advantages,
        ratio.clamp(1-config['clip'], 1+config['clip'])*advantages)
    value_loss = .5*(value-returns).square()
    entropy = network.entropy(logp, packed)
    anchor = packed['anchor_logps']
    safe_logp = logp.masked_fill(~packed['valid'], 0)
    anchor_kl = (anchor.exp()*(anchor-safe_logp)*packed['valid']).sum(1)
    loss = policy_loss+config['value_coef']*value_loss-config['entropy_coef']*entropy+beta*anchor_kl
    metrics = torch.stack((loss, policy_loss, value_loss, entropy, anchor_kl,
        ratio-1-log_ratio, ((ratio-1).abs() > config['clip']).to(logp.dtype)), 1)
    return loss.mean(), metrics


METRICS = ('loss', 'policy_loss', 'value_loss', 'entropy', 'anchor_kl', 'approx_old_kl', 'clip_fraction')


def ppo_update(ctx, actor, optimizer, rows, revision, checkpoint, beta, rng, *, device="cpu"):
    config = ctx.training
    if not available(device):
        raise ValueError('Requested learner device unavailable: '+device)
    worker.verify_live(ctx, actor, rows, revision, checkpoint)
    advantages, returns = worker.learn._advantages(ctx.objective_contract, rows, config['gae_lambda'])
    advantages = torch.tensor(advantages, dtype=torch.float32)
    returns = torch.tensor(returns, dtype=torch.float32)
    raw_mean, raw_std = float(advantages.mean()), float(advantages.std(unbiased=False))
    advantages = (advantages-advantages.mean())/advantages.std(unbiased=False).clamp_min(1e-8)
    started = time.monotonic()
    prepared, prepared_bytes = [], 0
    for index, row in enumerate(rows):
        item = data.prepare(actor, row.inputs())
        item['anchor_logps'] = rows[index].anchor_logps
        item['labels'] = torch.tensor([rows[index].capture['index'], rows[index].capture['old_logp'],
                                      advantages[index], returns[index]], dtype=torch.float32)
        prepared_bytes += sum(v.numel()*v.element_size() for v in item.values())
        if prepared_bytes > config['max_buffer_bytes']:
            raise ValueError('Prepared input bank exceeds live memory budget')
        prepared.append(item)
    preparation_seconds = time.monotonic()-started
    learner = actor if device == 'cpu' else copy.deepcopy(actor).to(device)
    learner_optimizer = optimizer
    if device != 'cpu':
        learner_optimizer = torch.optim.Adam(learner.parameters(), lr=config['learning_rate'])
        learner_optimizer.load_state_dict(copy.deepcopy(optimizer.state_dict()))
    metrics, early_stop = [], False
    synchronize(device)
    started = time.monotonic()
    for epoch in range(config['epochs']):
        indices = list(range(len(rows)))
        rng.shuffle(indices)
        for start in range(0, len(rows), config['batch_size']):
            if len(metrics) % 32 == 0:
                ctx.check()
            selected = indices[start:start+config['batch_size']]
            learner_optimizer.zero_grad(set_to_none=True)
            selected_inputs = [prepared[i] for i in selected]
            chunks = data.split_for_memory(selected_inputs, config['max_buffer_bytes'])
            batch_values = []
            for chunk in chunks:
                packed = data.collate(chunk, device, config['max_buffer_bytes'])
                loss, values = objective(learner, packed, beta, config)
                worker.neural._finite(values, 'batch PPO objective/metrics')
                (loss*(len(chunk)/len(selected))).backward()
                batch_values.extend(values.detach().cpu().tolist())
                del loss, values, packed
            average = {key: math.fsum(v[j] for v in batch_values)/len(batch_values)
                       for j, key in enumerate(METRICS)}
            if average['approx_old_kl'] > 1.5*config['target_old_kl']:
                learner_optimizer.zero_grad(set_to_none=True)
                early_stop = True
                break
            norm = torch.nn.utils.clip_grad_norm_(learner.parameters(), config['max_grad_norm'], error_if_nonfinite=True)
            learner_optimizer.step()
            metrics.append(average | {'gradient_norm': norm.detach(), 'batch_rows': len(selected),
                                      'execution_microbatches': len(chunks)})

        if early_stop:
            break
    synchronize(device)
    if metrics:
        norms = torch.stack([m['gradient_norm'] for m in metrics]).cpu().tolist()
        for metric, norm in zip(metrics, norms):
            metric['gradient_norm'] = norm
    optimization_seconds = time.monotonic()-started
    if not metrics:
        raise RuntimeError('No finite batch optimizer updates')
    if device != 'cpu':
        actor.load_state_dict(tensors_to(learner.state_dict(), 'cpu'), strict=True)
        optimizer.load_state_dict(tensors_to(learner_optimizer.state_dict(), 'cpu'))
        del learner_optimizer, learner
    return {'updates': len(metrics), 'optimizer_examples': sum(x['batch_rows'] for x in metrics),
        'decisions': len(rows), 'early_stop': early_stop, 'kl_coef': beta,
        'raw_advantage_mean': raw_mean, 'raw_advantage_std': raw_std,
        'preparation_seconds': preparation_seconds, 'optimization_seconds': optimization_seconds,
        'learner_device': device, 'batch_execution': EXECUTION_SCHEMA,
        'requested_batch_size': config['batch_size'],
        'execution_microbatches': sum(m['execution_microbatches'] for m in metrics),
        'means': {k: math.fsum(x[k] for x in metrics)/len(metrics) for k in metrics[0]},
        **worker.distribution_metrics(actor, rows)}
