"""Optional bounded cross-worker inference, with independent worker-side RNG."""
import copy
import math
import time
from collections import Counter
import torch
from arkguardrl import neural
from .device import available, synchronize
from .learning import data

INPUTS = {'obs', 'candidates', 'schedules', 'experts', 'route_obs', 'route_actions'}
DTYPES = {'float32': torch.float32, 'int64': torch.int64}


def pack_tensors(values, max_bytes):
    packet, size = {}, 0
    for key, value in values.items():
        if value.device.type != 'cpu' or value.dtype not in DTYPES.values():
            raise ValueError('Inference pipe requires CPU float32/int64 tensors')
        raw = value.detach().contiguous().numpy().tobytes()
        size += len(raw)
        if size > max_bytes:
            raise MemoryError('Inference packet exceeds byte budget')
        packet[key] = (str(value.dtype).removeprefix('torch.'), tuple(value.shape), raw)
    return packet, size


def unpack_tensors(packet, fields, max_bytes):
    if not isinstance(packet, dict) or set(packet) != fields:
        raise ValueError('Inference packet fields differ')
    result, size = {}, 0
    for key, value in packet.items():
        if not isinstance(value, tuple) or len(value) != 3:
            raise ValueError('Invalid inference tensor descriptor')
        dtype, shape, raw = value
        if (dtype not in DTYPES or not isinstance(shape, tuple) or len(shape) > 2
                or any(type(n) is not int or n < 0 for n in shape) or not isinstance(raw, bytes)):
            raise ValueError('Invalid inference tensor shape/dtype')
        size += len(raw)
        if size > max_bytes or math.prod(shape)*(4 if dtype == 'float32' else 8) != len(raw):
            raise ValueError('Inference tensor byte size mismatch')
        result[key] = (torch.frombuffer(bytearray(raw), dtype=DTYPES[dtype]).reshape(shape)
                       if raw else torch.empty(shape, dtype=DTYPES[dtype]))
    return result


class BatchedInference:
    def __init__(self, actor, anchor, device, max_bytes):
        if not available(device):
            raise ValueError('Requested inference device unavailable')
        self.cpu_actor, self.max_bytes, self.device = actor, max_bytes, device
        self.actor = actor if device == 'cpu' else copy.deepcopy(actor).to(device)
        self.anchor = anchor if device == 'cpu' else copy.deepcopy(anchor).to(device)
        self.actor.eval()
        self.anchor.eval()
        self.stats = Counter()
        self.histogram = Counter()

    def evaluate(self, requests):
        started = time.monotonic()
        prepared = [data.prepare(self.cpu_actor, row) for row in requests]
        self.stats['prepare_seconds'] += time.monotonic()-started
        results = []
        with torch.no_grad():
            for chunk in data.split_for_memory(prepared, self.max_bytes):
                started = time.monotonic()
                packed = data.collate(chunk, self.device, self.max_bytes)
                logp, values = self.actor.forward_batch(packed)
                anchor = self.anchor.forward_batch(packed)[0]
                cpu_logp, cpu_values, cpu_anchor = logp.cpu(), values.cpu(), anchor.cpu()
                for i, row in enumerate(chunk):
                    count = len(row['candidates'])
                    a, b = cpu_logp[i, :count].clone(), cpu_anchor[i, :count].clone()
                    neural._finite(a, 'batched actor distribution')
                    neural._finite(b, 'batched anchor distribution')
                    neural._finite(cpu_values[i], 'batched value')
                    results.append((a, float(cpu_values[i]), b))
                synchronize(self.device)
                self.stats['forward_seconds'] += time.monotonic()-started
                self.stats['batches'] += 1
                self.stats['states'] += len(chunk)
                self.histogram[len(chunk)] += 1
        return results

    def report(self):
        return dict(self.stats) | {'batch_histogram': dict(self.histogram),
            'mean_batch_size': self.stats['states']/max(1, self.stats['batches']), 'device': self.device}
