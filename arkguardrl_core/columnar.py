"""Lossless ragged tensor slabs for bounded blocks of live rollout rows.

Each field owns one flat CPU storage with per-row shapes/offsets. Metadata is
kept intact, including transitions and audit observations. This private pipe
codec does not admit historical training samples or cross-process witnesses.
"""
import math
import pickle
import zlib
import torch
from .transport import CAPTURE_TENSORS, ROW_TENSORS, DTYPES

SCHEMA = 'arkguardrl-core/live-columnar-zlib/v2'
MAX_ROWS = 32
KEYS = {f'{prefix}.{name}' for prefix, names in
        (('capture', CAPTURE_TENSORS), ('row', ROW_TENSORS)) for name in names}


def encode_rows(rows, max_bytes):
    if not 1 <= len(rows) <= MAX_ROWS:
        raise ValueError('Columnar block must contain 1..32 rows')
    fields, banks, tensor_bytes = [], {}, 0
    for row in rows:
        item = dict(vars(row))
        item['capture'] = dict(row.capture)
        fields.append(item)
        for prefix, names, source in (('capture', CAPTURE_TENSORS, item['capture']),
                                      ('row', ROW_TENSORS, item)):
            for name in names:
                value = source.pop(name)
                if value.device.type != 'cpu' or value.dtype not in DTYPES.values() or value.ndim > 2:
                    raise ValueError('Columnar inputs require CPU float32/int64 tensors of rank <=2')
                key = prefix+'.'+name
                dtype = str(value.dtype).removeprefix('torch.')
                if key not in banks:
                    banks[key] = [dtype, [], [0], bytearray()]
                bank = banks[key]
                if dtype != bank[0]:
                    raise ValueError('Mixed field dtype in columnar block')
                # Extend from the NumPy buffer, avoiding a separate bytes allocation
                # per tensor. Only owned flat slabs survive serialization.
                raw = memoryview(value.detach().contiguous().numpy()).cast('B') if value.numel() else b''
                tensor_bytes += len(raw)
                if tensor_bytes > max_bytes:
                    raise MemoryError('Columnar tensors exceed block byte budget')
                bank[1].append(tuple(value.shape))
                bank[2].append(bank[2][-1]+value.numel())
                bank[3].extend(raw)
    metadata_bytes = len(pickle.dumps(fields, protocol=5))
    if tensor_bytes+metadata_bytes > max_bytes:
        raise MemoryError('Columnar data exceeds block byte budget')
    raw = pickle.dumps(dict(schema=SCHEMA, fields=fields, tensors=banks), protocol=5)
    if len(raw) > max_bytes:
        raise MemoryError('Columnar envelope exceeds block byte budget')
    payload = zlib.compress(raw, level=1)
    if len(payload) > max_bytes:
        raise MemoryError('Compressed columnar block exceeds budget')
    return payload, dict(raw_bytes=len(raw), tensor_bytes=tensor_bytes, metadata_bytes=metadata_bytes)


def decode_rows(payload, max_bytes, expected_rows):
    if type(expected_rows) is not int or not 1 <= expected_rows <= MAX_ROWS:
        raise ValueError('Invalid columnar row count')
    decoder = zlib.decompressobj()
    raw = decoder.decompress(payload, max_bytes+1)
    if (len(raw) > max_bytes or decoder.unconsumed_tail or decoder.unused_data or not decoder.eof):
        raise ValueError('Columnar compressed stream exceeds bound or is incomplete')
    envelope = pickle.loads(raw)
    if (not isinstance(envelope, dict) or set(envelope) != {'schema','fields','tensors'}
            or envelope['schema'] != SCHEMA):
        raise ValueError('Unknown columnar schema')
    fields, banks = envelope['fields'], envelope['tensors']
    if not isinstance(fields, list) or len(fields) != expected_rows or not isinstance(banks, dict) or set(banks) != KEYS:
        raise ValueError('Columnar row count/tensor fields mismatch')
    from .sampling import LiveRow
    for item in fields:
        if not isinstance(item, dict) or set(item) != set(LiveRow.__dataclass_fields__) - set(ROW_TENSORS):
            raise ValueError('Columnar metadata fields differ')
        if not isinstance(item['capture'], dict) or any(k in item['capture'] for k in CAPTURE_TENSORS):
            raise ValueError('Duplicate columnar capture tensor')
    total_bytes = 0
    for key, descriptor in banks.items():
        if not isinstance(descriptor, list) or len(descriptor) != 4:
            raise ValueError('Invalid columnar bank descriptor')
        dtype, shapes, offsets, content = descriptor
        if (dtype not in DTYPES or not isinstance(content, bytearray)
                or not isinstance(shapes,list) or len(shapes) != expected_rows
                or not isinstance(offsets,list) or len(offsets) != expected_rows+1
                or any(type(n) is not int or n < 0 for n in offsets) or offsets[0] != 0):
            raise ValueError('Invalid columnar dtype/offset table')
        itemsize = 4 if dtype == 'float32' else 8
        total_bytes += len(content)
        if total_bytes > max_bytes or offsets[-1]*itemsize != len(content):
            raise ValueError('Columnar storage size mismatch')
        for index, shape in enumerate(shapes):
            if (not isinstance(shape,tuple) or len(shape)>2
                    or any(type(n) is not int or n < 0 for n in shape)
                    or offsets[index+1]-offsets[index] != math.prod(shape)):
                raise ValueError('Columnar shape/offset mismatch')
        # Pickle reconstructed an owned writable bytearray. Tensor views retain
        # it after this function returns; row slices never overlap or get reused.
        slab = torch.frombuffer(content,dtype=DTYPES[dtype]) if content else torch.empty(0,dtype=DTYPES[dtype])
        prefix, name = key.split('.')
        for index, item in enumerate(fields):
            target = item['capture'] if prefix == 'capture' else item
            target[name] = slab[offsets[index]:offsets[index+1]].reshape(shapes[index])
    return [LiveRow(**item) for item in fields], len(raw)
