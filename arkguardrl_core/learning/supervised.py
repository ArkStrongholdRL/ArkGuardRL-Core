"""Prepared CPU inputs and device-independent bounded supervised updates."""
from collections import OrderedDict
from pathlib import Path
import json
import shutil
import time
import torch
from . import data


def write(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')
    temporary.replace(path)


class PreparedBank:
    def __init__(self, cpu_model, shards, cache_dir):
        self.model, self.shards = cpu_model, shards
        self.index = [(i, j) for i, s in enumerate(shards) for j in range(s['rows'])]
        self.cache, self.bytes = OrderedDict(), 0
        self.prepared = Path(cache_dir)

    def prepare_once(self, on_progress=None, should_stop=None):
        self.prepared.mkdir(exist_ok=True)
        manifest = self.prepared / 'sources.json'
        if manifest.exists() and json.loads(manifest.read_text()) != self.shards:
            raise ValueError('Prepared source manifest changed')
        write(manifest, self.shards)
        for i, shard in enumerate(self.shards):
            path = self.prepared / f'{i:05d}.pt'
            if not path.exists():
                if shutil.disk_usage(self.prepared).free < 8*1024**3:
                    raise RuntimeError('Insufficient disk for fixed input cache')
                raw = torch.load(shard['path'], weights_only=True, map_location='cpu')
                rows = []
                for r in raw:
                    inputs = data.prepare(self.model, r['inputs'], copy_inputs=False)
                    # Save only owned compact storage, not the larger tensors behind views.
                    rows.append(dict(inputs={k:v.clone() for k,v in inputs.items()}, label=r['index']))
                tmp = path.with_suffix('.tmp')
                torch.save(rows, tmp)
                tmp.replace(path)
            if i % 100 == 0 or i+1 == len(self.shards):
                if on_progress:
                    on_progress(i+1, len(self.shards))
            if should_stop and should_stop():
                raise InterruptedError('Stopped during resumable input preparation')

    def get(self, index):
        i, j = self.index[index]
        if i not in self.cache:
            rows = torch.load(self.prepared / f'{i:05d}.pt', weights_only=True,
                              map_location='cpu', mmap=True)
            size = sum(v.numel()*v.element_size() for r in rows for v in r['inputs'].values())
            # These are read-only memory maps, not eagerly resident tensors.
            # Keep their metadata; the OS pages storage in/out as needed.
            # Eviction here repeatedly reparses torch files on almost every row.
            self.cache[i] = (rows, size)
            self.bytes += size
        self.cache.move_to_end(i)
        return self.cache[i][0][j]


def update(model, optimizer, chosen, *, parameters, max_bytes):
    """One full-batch update; microbatches preserve every label and candidate."""
    device = next(model.parameters()).device
    if not chosen:
        raise ValueError('Nonempty supervised batch required')
    chosen = sorted(chosen, key=lambda r: len(r['inputs']['candidates']))
    optimizer.zero_grad(set_to_none=True)
    offset, loss_sum, chunks = 0, 0., 0
    pack_seconds, compute_seconds = 0., 0.
    for chunk in data.split_for_memory([r['inputs'] for r in chosen], max_bytes):
        tick = time.monotonic()
        packed = data.collate(chunk, device=device, max_bytes=max_bytes)
        if device.type == 'mps':
            torch.mps.synchronize()
        elif device.type == 'cuda':
            torch.cuda.synchronize(device)
        pack_seconds += time.monotonic()-tick
        tick = time.monotonic()
        logp, _ = model.forward_batch(packed)
        labels = torch.tensor([r['label'] for r in chosen[offset:offset+len(chunk)]], device=device)
        loss = -logp[torch.arange(len(chunk), device=device), labels].sum()/len(chosen)
        if not torch.isfinite(loss):
            raise ValueError('Nonfinite supervised loss')
        loss.backward()
        loss_sum += float(loss.detach())
        compute_seconds += time.monotonic()-tick
        offset += len(chunk)
        chunks += 1
    norm = torch.nn.utils.clip_grad_norm_(parameters, 1., error_if_nonfinite=True)
    optimizer.step()
    return dict(loss=loss_sum, gradient_norm=float(norm), microbatches=chunks,
                pack_seconds=pack_seconds, forward_backward_seconds=compute_seconds)
