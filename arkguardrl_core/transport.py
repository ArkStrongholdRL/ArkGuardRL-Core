"""Lossless live-row codec for private collector pipes, not an artifact format.

Keep tensor dtype/shape and every metadata field. Numerical payloads use plain
contiguous CPU bytes instead of individually pickled Torch storages; fast zlib
compression removes repeated zeros/one-hot fields and repeated action metadata.
Only the parent and children it created may use this trusted-pickle protocol.
"""
import math
import pickle
import zlib

import torch

SCHEMA = 'arkguardrl-core/live-row-zlib/v6'
CAPTURE_TENSORS = ('obs', 'candidates', 'schedules', 'experts')
ROW_TENSORS = ('route_obs', 'route_actions', 'old_logps', 'anchor_logps', 'prep_history', 'plan_context', 'recovery_context', 'option_context', 'goal_relations')
DTYPES = {'float32': torch.float32, 'int64': torch.int64}


def encode_row(row, max_bytes):
    capture = dict(row.capture)
    fields = dict(vars(row))
    fields['capture'] = capture
    tensors = {}
    tensor_bytes = 0
    for prefix, names, source in (('capture', CAPTURE_TENSORS, capture),
                                   ('row', ROW_TENSORS, fields)):
        for name in names:
            tensor = source.pop(name)
            if tensor.device.type != 'cpu' or tensor.dtype not in DTYPES.values():
                raise ValueError('Live transport requires float32/int64 CPU tensors')
            raw = tensor.detach().contiguous().numpy().tobytes()
            tensor_bytes += len(raw)
            tensors[prefix+'.'+name] = (str(tensor.dtype).removeprefix('torch.'), tuple(tensor.shape), raw)
    metadata_bytes = len(pickle.dumps(fields, protocol=5))
    raw = pickle.dumps({'schema': SCHEMA, 'fields': fields, 'tensors': tensors}, protocol=5)
    if len(raw) > max_bytes:
        raise MemoryError('Uncompressed live row exceeds transport bound')
    payload = zlib.compress(raw, level=1)
    if len(payload) > max_bytes:
        raise MemoryError('Compressed live row exceeds transport bound')
    return payload, {'raw_bytes': len(raw), 'tensor_bytes': tensor_bytes,
                     'metadata_bytes': metadata_bytes}


def decode_row(payload, max_bytes):
    # Bound inflated bytes before unpickling/allocation. Reject trailing streams
    # and partial frames even though the outer pipe already checks SHA256.
    decoder = zlib.decompressobj()
    raw = decoder.decompress(payload, max_bytes+1)
    if (len(raw) > max_bytes or decoder.unconsumed_tail or decoder.unused_data
            or not decoder.eof):
        raise ValueError('Live row compressed stream exceeds bound or is incomplete')
    envelope = pickle.loads(raw)
    if (not isinstance(envelope, dict) or set(envelope) != {'schema', 'fields', 'tensors'}
            or envelope['schema'] != SCHEMA):
        raise ValueError('Unknown live row transport schema')
    fields, tensors = envelope['fields'], envelope['tensors']
    expected = {'capture.'+name for name in CAPTURE_TENSORS} | {'row.'+name for name in ROW_TENSORS}
    if set(tensors) != expected:
        raise ValueError('Live row tensor bank differs from protocol')
    for key, (dtype, shape, content) in tensors.items():
        if (dtype not in DTYPES or not isinstance(shape, tuple) or len(shape) > 2
                or any(type(n) is not int or n < 0 for n in shape)
                or not isinstance(content, bytes)):
            raise ValueError('Invalid live tensor shape/dtype')
        itemsize = 4 if dtype == 'float32' else 8
        if math.prod(shape)*itemsize != len(content) or len(content) > max_bytes:
            raise ValueError('Live tensor byte size mismatch')
        # Own writable storage independent of pickle/decompression buffers.
        value = (torch.frombuffer(bytearray(content), dtype=DTYPES[dtype]).reshape(shape)
                 if content else torch.empty(shape, dtype=DTYPES[dtype]))
        prefix, name = key.split('.')
        target = fields['capture'] if prefix == 'capture' else fields
        if name in target:
            raise ValueError('Duplicate live tensor field')
        target[name] = value
    from .sampling import LiveRow
    if set(fields) != set(LiveRow.__dataclass_fields__):
        raise ValueError('Live row fields differ from protocol')
    return LiveRow(**fields), len(raw)
