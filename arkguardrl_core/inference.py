"""Optional bounded cross-worker inference, with independent worker-side RNG."""
import copy
import math
import time
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from multiprocessing import Pipe
import torch
from .device import available
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


def to_device(values, device):
    """Transfer three contiguous dtype buffers instead of many small tensors."""
    if device == 'cpu':
        return values
    result = {}
    for dtype in {v.dtype for v in values.values()}:
        fields = [(k, v) for k, v in values.items() if v.dtype == dtype]
        host = torch.cat([v.reshape(-1) for _, v in fields])
        if device == 'cuda':
            host = host.pin_memory()
        buffer = host.to(device, non_blocking=device == 'cuda')
        offset = 0
        for key, value in fields:
            result[key] = buffer[offset:offset+value.numel()].reshape(value.shape)
            offset += value.numel()
    return result


class InferenceExecutor:
    """One device job at a time, without blocking the collector's pipe reader."""
    def __init__(self, inference):
        self.inference = inference
        self.ready, self._notify = Pipe(duplex=False)
        self._executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix='inference',
                                            initializer=self._initialize_device)
        self._future = None
        self.batch = None

    def _initialize_device(self):
        # CUDA's current device is thread-local; follow the resident model.
        if self.inference.device == 'cuda':
            torch.cuda.set_device(next(self.inference.actor.parameters()).device)

    def submit(self, batch, selector):
        if self.batch is not None:
            raise RuntimeError('Inference execution slot is occupied')
        self.batch = batch
        evaluate = self.inference.evaluate_options if selector else self.inference.evaluate
        self._future = self._executor.submit(evaluate, [r[3] for r in batch])
        self._future.add_done_callback(lambda _: self._notify.send_bytes(b''))

    def receive(self):
        self.ready.recv_bytes()
        predictions = self._future.result()
        batch, self.batch = self.batch, None
        self._future = None
        return batch, predictions

    def close(self):
        self._executor.shutdown(wait=True, cancel_futures=True)
        self.ready.close()
        self._notify.close()


class BatchedInference:
    """Persistent device models; update parameters only when weights change."""
    def __init__(self, actor, anchor, device, max_bytes, *, actor_key=None, anchor_key=None):
        if not available(device):
            raise ValueError('Requested inference device unavailable')
        self.max_bytes, self.device = max_bytes, device
        self.actor = self.anchor = None
        self.actor_key = self.anchor_key = None
        self.update(actor, anchor, actor_key=actor_key, anchor_key=anchor_key)

    def update(self, actor, anchor, *, actor_key=None, anchor_key=None):
        from .sampling import weights_hash
        actor_key = actor_key or weights_hash(actor)
        anchor_key = anchor_key or weights_hash(anchor)
        self.cpu_actor = actor
        self.stats, self.histogram, self.selector_histogram = Counter(), Counter(), Counter()
        shared = (actor_key == anchor_key and type(actor) is type(anchor)
                  and getattr(actor, 'planner_config', None) == getattr(anchor, 'planner_config', None)
                  and getattr(actor, 'specialist_config', None) == getattr(anchor, 'specialist_config', None))
        if self.device == 'cpu':
            self.actor, self.anchor = actor, actor if shared else anchor
        else:
            # Split an aliased anchor before updating the actor's storage.
            if not shared:
                if self.anchor is None or self.anchor is self.actor:
                    self.anchor = copy.deepcopy(anchor).to(self.device).eval()
                    self.stats['anchor_loads'] += 1
                elif self.anchor_key != anchor_key:
                    self.anchor.load_state_dict(anchor.state_dict())
                    self.stats['anchor_loads'] += 1
            if self.actor is None:
                self.actor = copy.deepcopy(actor).to(self.device).eval()
                self.stats['actor_loads'] += 1
            elif self.actor_key != actor_key:
                self.actor.load_state_dict(actor.state_dict())
                self.stats['actor_loads'] += 1
            if shared:
                self.anchor = self.actor
            else:
                self.anchor.base.definitions = self.actor.base.definitions
        self.actor.eval(); self.anchor.eval()
        self.actor_key, self.anchor_key = actor_key, anchor_key
        self.shared = shared

    def evaluate(self, requests):
        from .learning.network import forward
        from .offer_binding import pushforward
        started = time.monotonic()
        prepared = [data.prepare(self.cpu_actor, row, copy_inputs=False, validate=False) for row in requests]
        self.stats['prepare_seconds'] += time.monotonic()-started
        results = []
        with torch.inference_mode():
            for chunk in data.split_for_memory(prepared, self.max_bytes):
                started = time.monotonic()
                packed = to_device(data.collate(chunk, 'cpu', self.max_bytes,
                    bucket_size=128 if self.device != 'cpu' else 0), self.device)
                raw, values = forward(self.actor, packed, apply_binding=False)
                if self.shared:
                    anchor = pushforward(raw, packed['reward_binding']) if 'reward_binding' in packed else raw
                else:
                    anchor = self.anchor.forward_batch(packed)[0]
                anchor = anchor.log_softmax(-1)
                # One blocking device-to-host transfer; no extra synchronize().
                cpu = torch.cat((raw, anchor, values[:, None]), -1).cpu()
                width = raw.shape[1]
                for i, row in enumerate(chunk):
                    count = len(row['candidates'])
                    results.append((cpu[i, :count].clone(), float(cpu[i, -1]),
                                    cpu[i, width:width+count].clone()))
                self.stats['forward_seconds'] += time.monotonic()-started
                self.stats['batches'] += 1
                self.stats['states'] += len(chunk)
                self.histogram[len(chunk)] += 1
        return results

    def evaluate_options(self, requests):
        from .strategic_options import distribution_batch
        # Menus are small, but preserve the same execution budget for selectors.
        chunks, current = [], []
        for request in requests:
            trial = current+[request]
            estimate = len(trial)*max(len(r['menu']) for r in trial)*4096+sum(
                v.numel()*v.element_size() for r in trial for v in r.values())
            if estimate > self.max_bytes:
                if not current:
                    raise MemoryError('One selector exceeds inference memory budget')
                chunks.append(current); current = []
                if len(request['menu'])*4096+sum(v.numel()*v.element_size() for v in request.values()) > self.max_bytes:
                    raise MemoryError('One selector exceeds inference memory budget')
            current.append(request)
        if current: chunks.append(current)
        results = []
        with torch.inference_mode():
            for chunk in chunks:
                started = time.monotonic()
                count = max(len(r['menu']) for r in chunk)
                menus = chunk[0]['menu'].new_zeros(len(chunk), count, chunk[0]['menu'].shape[-1])
                valid = torch.zeros(len(chunk), count, dtype=torch.bool)
                for i, r in enumerate(chunk):
                    menus[i, :len(r['menu'])] = r['menu']; valid[i, :len(r['menu'])] = True
                batch = to_device({'states': torch.stack([r['state'] for r in chunk]), 'menus': menus, 'valid': valid}, self.device)
                logp, values = distribution_batch(self.actor, batch['states'], batch['menus'], batch['valid'])
                cpu = torch.cat((logp, values[:, None]), -1).cpu()
                results.extend((cpu[i, :len(r['menu'])].clone(), float(cpu[i, -1]), None) for i, r in enumerate(chunk))
                self.stats['selector_seconds'] += time.monotonic()-started
                self.stats['selector_batches'] += 1; self.stats['selector_states'] += len(chunk)
                self.selector_histogram[len(chunk)] += 1
        return results

    def report(self):
        return dict(self.stats) | {'batch_histogram': dict(self.histogram),
            'selector_batch_histogram': dict(self.selector_histogram), 'shared_actor_anchor': self.shared,
            'mean_batch_size': self.stats['states']/max(1, self.stats['batches']), 'device': self.device}
