"""Bounded, CPU route-policy imitation from explicit supervised tensor shards.

Teacher rows are independent of LiveRow/PPO buffers. Runtime and actor inputs
are reused unchanged. Shards contain rows with inputs and target index only.
"""
from collections import OrderedDict
import bisect
import random
import time

import torch
from .learning import data


def fit(model, shards, *, steps, batch_size, learning_rate, seed, max_wall,
        max_cache_bytes=256*1024**2, on_progress=None, optimizer_state=None, rng_state=None):
    if not shards or min(steps, batch_size, learning_rate, max_wall) <= 0:
        raise ValueError('Explicit bounded supervised run required')
    if next(model.parameters()).device.type != 'cpu':
        raise ValueError('CPU actor required')
    cache, used, cumulative = OrderedDict(), 0, []
    for shard in shards:
        cumulative.append((cumulative[-1] if cumulative else 0) + shard['rows'])

    def select(index):
        nonlocal used
        part = bisect.bisect_right(cumulative, index)
        descriptor = shards[part]
        if part not in cache:
            records = torch.load(descriptor['path'], map_location='cpu', weights_only=True)
            size = sum(v.numel()*v.element_size() for row in records for v in row['inputs'].values())
            if size > max_cache_bytes:
                raise MemoryError('Supervised shard exceeds cache bound')
            while cache and used + size > max_cache_bytes:
                _, (_, old_size) = cache.popitem(last=False)
                used -= old_size
            cache[part] = (records, size)
            used += size
        cache.move_to_end(part)
        return cache[part][0][index - (cumulative[part-1] if part else 0)]

    actor_parameters = [p for name, p in model.named_parameters() if not name.startswith('base.critic.')]
    optimizer = torch.optim.Adam(actor_parameters, lr=learning_rate)
    rng = random.Random(seed)
    if optimizer_state is not None:
        optimizer.load_state_dict(optimizer_state)
    if rng_state is not None:
        rng.setstate(rng_state)
    start, history, complete = time.monotonic(), [], 0
    model.train()
    for step in range(steps):
        if time.monotonic() - start >= max_wall:
            break
        chosen = [select(rng.randrange(cumulative[-1])) for _ in range(batch_size)]
        rows = [data.prepare(model, row['inputs'], copy_inputs=False) for row in chosen]
        optimizer.zero_grad(set_to_none=True)
        loss_total = 0.0
        offset = 0
        for chunk in data.split_for_memory(rows, 256*1024**2):
            packed = data.collate(chunk, max_bytes=256*1024**2)
            logp, _ = model.forward_batch(packed)
            labels = torch.tensor([r['index'] for r in chosen[offset:offset+len(chunk)]])
            loss = -logp[torch.arange(len(chunk)), labels].mean()
            if not torch.isfinite(loss):
                raise ValueError('Nonfinite supervised loss')
            (loss * len(chunk)/batch_size).backward()
            loss_total += float(loss.detach()) * len(chunk)/batch_size
            offset += len(chunk)
        norm = torch.nn.utils.clip_grad_norm_(actor_parameters, 1., error_if_nonfinite=True)
        optimizer.step()
        complete += 1
        if complete == 1 or complete % 100 == 0 or complete == steps:
            record = dict(step=complete, loss=loss_total, grad_norm=float(norm),
                          elapsed_seconds=time.monotonic()-start, cache_bytes=used)
            history.append(record)
            if on_progress is not None:
                on_progress(record)
    model.eval()
    return dict(steps=complete, requested_steps=steps, history=history,
                elapsed_seconds=time.monotonic()-start, stop='steps' if complete == steps else 'wall',
                optimizer=optimizer.state_dict(), rng=rng.getstate(),
                critic='head excluded from optimizer; shared encoder changes')
