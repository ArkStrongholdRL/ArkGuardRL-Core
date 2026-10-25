"""GPL-3.0-or-later array backend for public route features v2.

Derived from scripts/route_features.py. Only after-board summaries are batched;
all original context validation, action order, swaps and withdrawals remain.
No engine execution or future combat score is used here.
"""
import math
import platform
import sys
from collections import Counter, OrderedDict

import numpy as np
import torch
from arkguardrl.candidates import MAX_CANDIDATES
from arkguardrl.contracts import canonical, fingerprint
from .routes.features import ACTION_DIM, ACTION_FIELDS, CELLS, context, metrics, unit, validate_geometry

SCHEMA = 'arkguardrl/route-feature-array-execution/v1'
CHUNK_SIZE = 128


def _observation(paths, ground, airpath, progress, units, gc, ac, hc, before):
    obs = []
    for x in CELLS:
        obs.extend((float(x in ground), float(x in airpath), progress[0].get(x, 0),
            progress[1].get(x, 0), sum(u['block'] for u in units if u['at'] == x)/8,
            gc[x]/2000, ac[x]/2000, hc[x]/4))
    obs.extend((len(paths)/16, *before[:3]))
    return tuple(obs)


class RouteFeatureCache:
    """Bounded per-episode public-feature cache; never stores neural activations.

    The PublicCatalog is immutable for an episode. Every call still validates
    the full observation/geometry binding. Economy/history do not enter these
    geometric features; actions and their source/hand destination do.
    """
    def __init__(self, *, max_units=1024, max_actions=MAX_CANDIDATES):
        if (type(max_units) is not int or not 1 <= max_units <= 4096
                or type(max_actions) is not int or not 1 <= max_actions <= MAX_CANDIDATES):
            raise ValueError('Explicit bounded route cache capacities required')
        self.max_units, self.max_actions = max_units, max_actions
        self._catalog = None
        self._scope = self._path_key = self._base_key = None
        self._units, self._actions = OrderedDict(), OrderedDict()
        self._path = self._base = None
        self.stats = Counter()

    def get_unit(self, piece):
        key = canonical(piece)
        if key in self._units:
            self.stats['unit_hits'] += 1
            self._units.move_to_end(key)
            return self._units[key]
        self.stats['unit_misses'] += 1
        result = unit(self._catalog, piece)
        self._units[key] = result
        if len(self._units) > self.max_units:
            self._units.popitem(last=False)
        return result

    def begin(self, state, geometry, catalog):
        validate_geometry(state, geometry)
        own, public = state['own'], state['public']
        scope = fingerprint({'data': catalog.data_sha256, 'phase': state['phase'],
            'own': {k: own.get(k) for k in ('bonds', 'effects', 'loadout', 'ops', 'bandId',
                'standIns', 'diy', 'diyBanned', 'deployment')},
            'public': {k: public.get(k) for k in ('stageId', 'setupRevision', 'disabledBonds',
                'factions', 'bossId', 'bossRound', 'bossPairs', 'combatMode')}})
        if self._catalog is not catalog or self._scope != scope:
            self._units.clear()
            self._base_key = None
            self._scope, self._catalog = scope, catalog
        path_key = fingerprint({k: geometry[k] for k in ('schema', 'stageId', 'round', 'field', 'routes')})
        if self._path_key != path_key:
            paths = [(row['fly'], [tuple(x) for x in row['tiles']]) for row in geometry['routes']]
            ground = {x for air, tiles in paths if not air for x in tiles}
            airpath = {x for air, tiles in paths if air for x in tiles}
            contacts, progress = {}, [{}, {}]
            for air, tiles in paths:
                for index, x in enumerate(tiles):
                    progress[int(air)][x] = max(progress[int(air)].get(x, 0), index/max(1, len(tiles)-1))
                    if not air:
                        contacts.setdefault(x, set()).add(tiles[max(0, index-1)])
            self._path = paths, ground, airpath, contacts, progress
            self._path_key = path_key
            self.stats['path_misses'] += 1
        else:
            self.stats['path_hits'] += 1
        base_key = (scope, path_key, fingerprint(own['board']))
        if self._base_key != base_key:
            self._actions.clear()
            units = [self.get_unit(piece) for piece in own['board'] if piece['kind'] == 'chess']
            paths, ground, airpath, contacts, progress = self._path
            gc, ac, hc, blockers, before = metrics(units, ground, airpath, contacts)
            obs = _observation(paths, ground, airpath, progress, units, gc, ac, hc, before)
            self._base = (*self._path, units, gc, ac, hc, blockers, before, obs)
            self._base_key = base_key
            self.stats['board_misses'] += 1
        else:
            self.stats['board_hits'] += 1
        return self._base

    def action_key(self, state, action, source):
        to = action.get('to', {})
        destination = state['own']['hand'][to['idx']] if to.get('area') == 'hand' else None
        return canonical({'action': action, 'source': source, 'destination': destination})

    def get_action(self, key):
        if key in self._actions:
            self.stats['action_hits'] += 1
            self._actions.move_to_end(key)
            return list(self._actions[key])
        self.stats['action_misses'] += 1
        return None

    def remember_action(self, key, row):
        self._actions[key] = tuple(row)
        if len(self._actions) > self.max_actions:
            self._actions.popitem(last=False)


def validate_runtime():
    # Exact compatibility is currently measured only against CPython3.12 sum.
    if platform.python_implementation() != 'CPython' or sys.version_info[:2] != (3, 12):
        raise RuntimeError('The array route backend currently requires CPython3.12')
    if ACTION_FIELDS[20:26] != ('delta_ground_coverage', 'delta_air_coverage',
            'delta_supported_blockers', 'delta_blockers', 'delta_damage_contact', 'delta_healing_contact'):
        raise ValueError('Reference route feature layout changed')


def _summaries(boards, ground, airpath, contacts, base_units=()):
    """Float64 with CPython3.12 compensated sum; arrays span candidates.

    Per-call banks contain only objects from this exact decision. No state cache
    persists between observations, nor are trainable neural activations cached.
    """
    if not boards:
        return np.empty((0, 6), dtype=np.float64)
    if len(boards) > MAX_CANDIDATES or any(len(board) > 128 for board in boards):
        raise ValueError('Array route temporary-domain bound exceeded; no slicing')
    cells = {cell: i for i, cell in enumerate(CELLS)}
    unique = [None]
    identities = {}
    width = max(1, max(map(len, boards)))
    indices = np.zeros((len(boards), width), dtype=np.int64)
    for i, board in enumerate(boards):
        if len({u['uid'] for u in board}) != len(board):
            raise ValueError('Duplicate board UID')
        for j, unit in enumerate(board):
            key = id(unit)
            if key not in identities:
                identities[key] = len(unique)
                unique.append(unit)
            indices[i, j] = identities[key]
    cover = np.zeros((len(unique), 36), dtype=bool)
    positions = np.zeros(len(unique), dtype=np.int64)
    dps = np.zeros(len(unique), dtype=np.float64)
    air_dps = dps.copy()
    healer = np.zeros(len(unique), dtype=np.int64)
    blocking = np.zeros(len(unique), dtype=bool)
    ground_mask = np.array([x in ground for x in CELLS])
    air_mask = np.array([x in airpath for x in CELLS])
    required = np.eye(36, dtype=bool)
    for cell, adjacent in contacts.items():
        for previous in adjacent:
            required[cells[cell], cells[previous]] = True
    for i, unit in enumerate(unique[1:], 1):
        positions[i] = cells[unit['at']]
        locations = [cells[x] for x in unit['cover'] if x in cells]
        cover[i, locations] = True
        dps[i] = unit['dps']
        air_dps[i] = unit['dps'] if unit['air'] else 0.
        healer[i] = unit['heal']
        blocking[i] = unit['block'] > 0 and unit['at'] in ground
    # Integer coverage of the complete contact requirement; no threshold on a
    # floating-point subtraction can create a spurious supported blocker.
    supports = cover.astype(np.int16) @ required.astype(np.int16).T == required.sum(1)
    # A move/removal retains an ordered subset of the current board, followed
    # by at most the swapped unit and moved unit. Accumulate each retained
    # prefix once; update only its changed suffix for each candidate. Keep the
    # compensated accumulator, not a rounded total, to preserve sum order.
    base_ids = {id(value) for value in base_units}
    prefixes, prefix_ids, tails = {}, [], []
    for board, index in zip(boards, indices):
        split = next((i for i, value in enumerate(board) if id(value) not in base_ids), len(board))
        key = tuple(index[:split])
        if key not in prefixes:
            prefixes[key] = len(prefixes)
        prefix_ids.append(prefixes[key])
        tails.append(index[split:len(board)])
    prefix_gc = np.zeros((len(prefixes), 36))
    prefix_ac = prefix_gc.copy()
    prefix_ge, prefix_ae = prefix_gc.copy(), prefix_gc.copy()
    prefix_hc = np.zeros((len(prefixes), 36), dtype=np.int64)
    prefix_support = np.zeros((len(prefixes), 36), dtype=np.int16)
    for prefix, group in prefixes.items():
        for selected in prefix:
            for total, error, values in (
                    (prefix_gc[group], prefix_ge[group], cover[selected]*dps[selected]),
                    (prefix_ac[group], prefix_ae[group], cover[selected]*air_dps[selected])):
                added = total+values
                error += np.where(np.abs(total) >= np.abs(values), (total-added)+values, (values-added)+total)
                total[:] = added
            prefix_hc[group] += cover[selected]*healer[selected]
            prefix_support[group] += supports[selected] & (dps[selected] > 0)
    tail_width = max(map(len, tails), default=0)
    suffix = np.zeros((len(boards), tail_width), dtype=np.int64)
    for i, tail in enumerate(tails):
        suffix[i, :len(tail)] = tail
    prefix_ids = np.asarray(prefix_ids, dtype=np.int64)
    output = []
    for start in range(0, len(indices), CHUNK_SIZE):
        index = indices[start:start+CHUNK_SIZE]
        count = len(index)
        group = prefix_ids[start:start+CHUNK_SIZE]
        gc, ac = prefix_gc[group].copy(), prefix_ac[group].copy()
        gc_error, ac_error = prefix_ge[group].copy(), prefix_ae[group].copy()
        hc = prefix_hc[group].copy()
        provider_counts = prefix_support[group].copy()
        for slot in range(tail_width):
            selected = suffix[start:start+CHUNK_SIZE, slot]
            # Match CPython3.12's compensated sum, vectorized over all boards
            # and cells. There are at most width iterations, not C*36 loops.
            for total, error, values in ((gc, gc_error, cover[selected] * dps[selected, None]),
                                        (ac, ac_error, cover[selected] * air_dps[selected, None])):
                added = total + values
                error += np.where(np.abs(total) >= np.abs(values),
                    (total-added)+values, (values-added)+total)
                total[:] = added
            hc += cover[selected] * healer[selected, None]
            provider_counts += supports[selected] & (dps[selected] > 0)[:, None]
        gc += gc_error
        ac += ac_error
        locations = positions[index]
        blockers = blocking[index]
        # Integer support counts allow exact self-exclusion without building
        # a candidate x querying-unit x provider-unit tensor.
        own_support = supports[index, locations] & (dps[index] > 0)
        supported = blockers & ((np.take_along_axis(provider_counts, locations, axis=1)-own_support) > 0)
        block_count = blockers.sum(1)
        damage, healing = np.zeros(count), np.zeros(count, dtype=np.int64)
        damage_error = damage.copy()
        for slot in range(width):
            values = gc[np.arange(count), locations[:, slot]] * blockers[:, slot]
            added = damage + values
            damage_error += np.where(np.abs(damage) >= np.abs(values),
                (damage-added)+values, (values-added)+damage)
            damage = added
            healing += hc[np.arange(count), locations[:, slot]] * blockers[:, slot]
        damage += damage_error
        # Keep the reference's final scalar arithmetic explicitly. Together
        # with compensated sums this preserves exact-zero delta features.
        gh = (gc[:, ground_mask] > 0).sum(1)
        ah = (ac[:, air_mask] > 0).sum(1)
        ns = supported.sum(1)
        output.append(np.array([[int(gh[i]) / max(1, len(ground)),
            int(ah[i]) / max(1, len(airpath)), int(ns[i]) / max(1, int(block_count[i])),
            int(block_count[i]) / 8, float(damage[i]) / 2000, int(healing[i]) / 8]
            for i in range(count)], dtype=np.float64))
    return np.concatenate(output)


def encode(state, actions, g, catalog, *, route_cache=None):
    validate_runtime()
    if route_cache is None:
        paths, ground, airpath, contacts, progress, units = context(state, g, catalog)
        gc, ac, hc, blockers, before = metrics(units, ground, airpath, contacts)
        obs = _observation(paths, ground, airpath, progress, units, gc, ac, hc, before)
        make_unit = lambda piece: unit(catalog, piece)
    else:
        if not isinstance(route_cache, RouteFeatureCache):
            raise ValueError('Explicit per-episode RouteFeatureCache required')
        paths, ground, airpath, contacts, progress, units, gc, ac, hc, blockers, before, obs = route_cache.begin(state, g, catalog)
        make_unit = route_cache.get_unit
    pieces = {p['uid']: p for area in ('board', 'hand', 'temp') for p in state['own'][area] if p}
    byuid = {u['uid']: u for u in units}
    bycell = {u['at']: u for u in units}
    terrain = {(x['row'], x['col']): x['kind'] for x in state['own']['deployment']['cells']}
    cache = {}
    rows = []
    pending = []
    remember = []
    for a in actions:
        source = pieces.get(a.get('uid'))
        to = a.get('to', {})
        move = a['t'] == 'g.move'
        remove = a['t'] == 'g.sell' or (move and to.get('area') == 'hand')
        target = (to['row'], to['col']) if move and to.get('area') == 'board' else None
        key = (a.get('uid'), target, a.get('dir', 'RIGHT'), remove, to.get('idx'))
        if key in cache:
            rows.append(cache[key])
            continue
        row = [0.0] * ACTION_DIM
        if source and source['kind'] == 'chess' and (move or remove):
            persistent_key = route_cache.action_key(state, a, source) if route_cache is not None else None
            if route_cache is not None:
                cached = route_cache.get_action(persistent_key)
                if cached is not None:
                    cache[key] = cached
                    rows.append(cached)
                    continue
                remember.append((persistent_key, len(rows)))
            old = byuid.get(source['uid'])
            src = old['at'] if old else None
            afterunits = [u for u in units if u['uid'] != source['uid']]
            actor = old
            known = True
            if target:
                occupant = bycell.get(target)
                if occupant and occupant['uid'] != source['uid']:
                    afterunits = [u for u in afterunits if u['uid'] != occupant['uid']]
                    if old:
                        p = dict(pieces[occupant['uid']], row=src[0], col=src[1])
                        afterunits.append(make_unit(p))
                actor = make_unit(dict(source, row=target[0], col=target[1], dir=a.get('dir', 'RIGHT')))
                afterunits.append(actor)
            elif move and to.get('area') == 'hand' and (state['own']['hand'][to['idx']] is not None):
                known = False
            if actor:
                at = target or actor['at']
                others = [u for u in afterunits if u['uid'] != actor['uid']]
                support = any((u['dps'] > 0 and at in u['cover'] and all((x in u['cover'] for x in contacts.get(at, ()))) for u in others))
                st = actor['stats']
                after = [0.0] * 6
                pending.append((len(rows), afterunits))
                row = [float(known), actor['block'] / 8, float(actor['heal']), float(actor['air']), actor['dps'] / 2000, float(st.get('maxHp', 0)) / 10000, float(st.get('def', 0)) / 1000, float(st.get('res', 0)) / 100, float(at in ground), float(at in airpath), progress[0].get(at, 0), progress[1].get(at, 0), len(actor['cover'] & ground) / max(1, len(ground)), len(actor['cover'] & airpath) / max(1, len(airpath)), float(at in actor['cover'] and all((x in actor['cover'] for x in contacts.get(at, ())))), float(support), min(1, sum((u['heal'] and at in u['cover'] for u in others)) / 2), sum((u['block'] > 0 and u['at'] in ground and (u['at'] in actor['cover']) and all((x in actor['cover'] for x in contacts.get(u['at'], ()))) for u in others)) / 8, gc.get(at, 0) / 2000, ac.get(at, 0) / 2000, *[after[i] - before[i] for i in range(6)], float(src in ground), float(src in airpath), float(terrain.get(at) == 'ranged'), float(at in bycell), float(move), float(remove)]
        if len(row) != ACTION_DIM or not all((math.isfinite(x) for x in row)):
            raise ValueError('Nonfinite/misshaped route candidate features')
        cache[key] = row
        rows.append(row)
    summaries = _summaries([board for _, board in pending], ground, airpath, contacts, units)
    for (index, _), summary in zip(pending, summaries):
        delta = [float(summary[i]) - before[i] for i in range(6)]
        if not all(math.isfinite(value) for value in delta):
            raise ValueError('Nonfinite array route summary')
        rows[index][20:26] = delta
    if route_cache is not None:
        for key, index in remember:
            route_cache.remember_action(key, rows[index])
    return (torch.tensor(obs, dtype=torch.float32), torch.tensor(rows, dtype=torch.float32).reshape(-1, ACTION_DIM))
