"""Compile public policy metadata once; numerical projection lives in Rust.

This is a model-layout bridge, not a source identity gate. Raw observations and
public progress remain available for the existing history/reward contracts.
"""
import json
import numpy as np
from arkguardrl.contracts import canonical, fingerprint
from arkguardrl.dynamic import OBS_DIM, ACTION_DIM
from arkguardrl.feedback import PROGRESS_SCHEMA


def metadata(catalog):
    cached = getattr(catalog, '_native_policy_metadata', None)
    if cached is not None:
        return cached
    from arkguardrl import structured as layout
    if (len(catalog.entity_ids) >= layout.ENTITY_VOCAB
            or len(catalog.mechanism_ids) >= layout.MECHANISM_VOCAB
            or len(catalog.enemy_ids) >= layout.ENEMY_VOCAB):
        raise ValueError('Catalog vocabulary exceeds structured embedding contract')
    bank, _, indices = catalog.definitions()
    air_column = (layout.ENTITY_PREFIX + len(layout.KINDS) + len(layout.AREAS)
                  + layout.ENTITY_NUMBERS.index('can_hit_fly'))
    definitions = []
    for i, (kind, identifier) in enumerate(bank['keys']):
        record = catalog._views.get(kind, {}).get(identifier, {})
        definitions.append(dict(kind=kind, id=identifier, index=indices[kind, identifier],
            ranges=catalog.attack_range({'kind': kind, 'id': identifier}) if kind == 'chess' else (),
            air=bool(bank['entities'][i][air_column]), base=record.get('baseId', identifier),
            isGolden=bool(record.get('isGolden', False)), upgradeNum=record.get('upgradeNum', 0),
            domain=dict(placement=catalog.placement_class({'kind': kind, 'id': identifier})
                        if kind in ('chess', 'token') and record.get('position') in ('MELEE', 'RANGED', 'ALL') else None,
                        itemType=record.get('itemType'), ownerRange=record.get('ownerRange'),
                        ownerRangeOutside=record.get('ownerRangeOutside'))))
    compiled = dict(definitions=definitions, band_ids=list(catalog.band_ids),
        entities=[dict(kind=k, id=i, index=n) for (k, i), n in catalog._entity_indices.items()],
        enemies=[dict(id=i, index=n) for i, n in catalog._enemy_indices.items()],
        config=catalog._data['config'], diy=catalog._data.get('diy', {}),
        choices=[dict(rec, kind=k, id=i) for (k, i), rec in catalog._choices.items()])
    cached = json.dumps(compiled, ensure_ascii=False, allow_nan=False,
                        separators=(',', ':')).encode('utf-8')
    catalog._native_policy_metadata = cached
    return cached


def encode(session, state, actions, history, *, native_history=False):
    progress = history.get('progress')
    if (not isinstance(progress, dict) or progress.get('schema') != PROGRESS_SCHEMA
            or progress.get('observation_id') != fingerprint(state)):
        raise ValueError('Structured v7 requires observation-bound public progress')
    # Public history remains independently owned; no callbacks or Python state
    # traversal occur inside the numerical encoder.
    if native_history:
        buffers = session.encode_ledger_policy(state['revision'], canonical(actions))
    else:
        buffers = session.encode_policy(state['revision'], canonical(history), canonical(actions))
    return decode(buffers, len(actions))


def decode(buffers, count):
    observation = np.frombuffer(buffers[0], dtype='<f4')
    candidates = np.frombuffer(buffers[1], dtype='<f4')
    schedules = np.frombuffer(buffers[2], dtype='<i8')
    experts = np.frombuffer(buffers[3], dtype='<i8')
    if (observation.size != OBS_DIM or candidates.size != count * ACTION_DIM
            or schedules.size != count or experts.size != count):
        raise RuntimeError('Native policy output shape mismatch')
    # Decision buffers are immutable Python-owned bytes. Gym returns a writable
    # observation copy; subsequent steps never overwrite earlier decisions.
    return observation, candidates.reshape(count, ACTION_DIM), schedules, experts
