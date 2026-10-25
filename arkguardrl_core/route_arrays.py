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
from arkguardrl.contracts import fingerprint
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


def _unit_key(piece):
    # Public default stats/range depend only on the catalog id; equipment and
    # live counters are deliberately not inputs to routes.features.unit.
    return (piece['uid'], piece['id'], piece.get('row'), piece.get('col'), piece.get('dir', 'RIGHT'))


class _SupportTable:
    """Decision-local integer queries with at most two removals and one swap."""
    def __init__(self, units, ground, contacts, healing):
        self.ground = ground
        self.required = {at: frozenset((at, *contacts.get(at, ()))) for at in CELLS}
        self.covers, self.blocker_totals = {}, {}
        self.providers = Counter()
        self.healing = healing
        self.blockers = Counter(u['at'] for u in units if self.is_blocker(u))
        for u in units:
            if u['dps'] > 0:
                self.providers.update(self.supports(u))

    def is_blocker(self, u):
        return u['block'] > 0 and u['at'] in self.ground

    def supports(self, u):
        key = frozenset(u['cover'])
        if key not in self.covers:
            self.covers[key] = frozenset(at for at, needed in self.required.items() if needed <= key)
        return self.covers[key]

    def query(self, actor, removed, added):
        at = actor['at']
        covered = self.supports(actor)
        providers, healers = self.providers[at], self.healing.get(at, 0)
        if covered not in self.blocker_totals:
            self.blocker_totals[covered] = sum(self.blockers[x] for x in covered)
        blockers = self.blocker_totals[covered]
        for sign, changed in ((-1, removed), (1, added)):
            for u in changed:
                providers += sign * (u['dps'] > 0 and at in self.supports(u))
                healers += sign * (u['heal'] and at in u['cover'])
                blockers += sign * (self.is_blocker(u) and u['at'] in covered)
        return float(at in covered), float(providers > 0), min(1, healers / 2), blockers / 8


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
        key = _unit_key(piece)
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
        # These are precisely the inputs read by encode/unit. Board, catalog,
        # route and setup identity are already bound by begin().
        return (action['t'], _unit_key(source), to.get('area'), to.get('row'),
                to.get('col'), to.get('idx'), action.get('dir', 'RIGHT'), destination is not None)

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


def _summaries(changes, ground, airpath, contacts, base_units=()):
    """Ordered base subsets plus appended units, with compensated float64 sums.

    Each change is (removed base indices, appended units). Candidate generation
    already knows this delta; do not rebuild and rediscover complete boards.
    """
    if not changes:
        return np.empty((0, 6), dtype=np.float64)
    if len(changes) > MAX_CANDIDATES or len(base_units) > 128:
        raise ValueError('Array route temporary-domain bound exceeded; no slicing')
    cells = {cell: i for i, cell in enumerate(CELLS)}
    unique = [None, *base_units]
    identities = {id(u): i for i, u in enumerate(base_units, 1)}
    base_positions = {u['uid']: i for i, u in enumerate(base_units)}
    if len(base_positions) != len(base_units):
        raise ValueError('Duplicate board UID')
    prefixes, prefix_ids, tails = {}, [], []
    for removed, added in changes:
        if (len(removed) > 2 or len(added) > 2 or len(set(removed)) != len(removed)
                or any(type(i) is not int or not 0 <= i < len(base_units) for i in removed)):
            raise ValueError('Invalid bounded route board delta')
        removed = tuple(sorted(removed))
        if removed not in prefixes:
            prefixes[removed] = (len(prefixes), tuple(i+1 for i in range(len(base_units)) if i not in removed))
        prefix_ids.append(prefixes[removed][0])
        if (len({u['uid'] for u in added}) != len(added)
                or any(u['uid'] in base_positions and base_positions[u['uid']] not in removed for u in added)):
            raise ValueError('Duplicate board UID')
        tail = []
        for u in added:
            key = id(u)
            if key not in identities:
                identities[key] = len(unique)
                unique.append(u)
            tail.append(identities[key])
        tails.append(tail)
    lengths = np.array([len(prefix) for _, prefix in prefixes.values()], dtype=np.int64)
    prefix_ids = np.asarray(prefix_ids, dtype=np.int64)
    width = max(1, max(int(lengths[g])+len(tail) for g, tail in zip(prefix_ids, tails)))
    if width > 128:
        raise ValueError('Array route temporary-domain bound exceeded; no slicing')
    prefix_indices = np.zeros((len(prefixes), width), dtype=np.int64)
    for group, prefix in prefixes.values():
        prefix_indices[group, :len(prefix)] = prefix
    indices = prefix_indices[prefix_ids]
    tail_width = max(map(len, tails), default=0)
    suffix = np.zeros((len(changes), tail_width), dtype=np.int64)
    for i, tail in enumerate(tails):
        suffix[i, :len(tail)] = tail
    for slot in range(tail_width):
        valid = suffix[:, slot] != 0
        indices[np.flatnonzero(valid), lengths[prefix_ids[valid]]+slot] = suffix[valid, slot]
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
    prefix_gc = np.zeros((len(prefixes), 36))
    prefix_ac = prefix_gc.copy()
    prefix_ge, prefix_ae = prefix_gc.copy(), prefix_gc.copy()
    prefix_hc = np.zeros((len(prefixes), 36), dtype=np.int64)
    prefix_support = np.zeros((len(prefixes), 36), dtype=np.int16)
    for group, prefix in prefixes.values():
        for selected in prefix:
            for total, error, values in (
                    (prefix_gc[group], prefix_ge[group], cover[selected]*dps[selected]),
                    (prefix_ac[group], prefix_ae[group], cover[selected]*air_dps[selected])):
                added = total+values
                error += np.where(np.abs(total) >= np.abs(values), (total-added)+values, (values-added)+total)
                total[:] = added
            prefix_hc[group] += cover[selected]*healer[selected]
            prefix_support[group] += supports[selected] & (dps[selected] > 0)
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
    unit_indices = {u['uid']: i for i, u in enumerate(units)}
    bycell = {u['at']: u for u in units}
    terrain = {(x['row'], x['col']): x['kind'] for x in state['own']['deployment']['cells']}
    support_table = _SupportTable(units, ground, contacts, hc)
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
            actor = old
            removed, added = ([old] if old else []), []
            known = True
            if target:
                occupant = bycell.get(target)
                if occupant and occupant['uid'] != source['uid']:
                    removed.append(occupant)
                    if old:
                        p = dict(pieces[occupant['uid']], row=src[0], col=src[1])
                        swapped = make_unit(p)
                        added.append(swapped)
                actor = make_unit(dict(source, row=target[0], col=target[1], dir=a.get('dir', 'RIGHT')))
            elif move and to.get('area') == 'hand' and (state['own']['hand'][to['idx']] is not None):
                known = False
            if actor:
                at = target or actor['at']
                support_features = support_table.query(actor, removed, added)
                st = actor['stats']
                after = [0.0] * 6
                appended = (*added, actor) if target else ()
                pending.append((len(rows), (tuple(unit_indices[u['uid']] for u in removed), appended)))
                row = [float(known), actor['block'] / 8, float(actor['heal']), float(actor['air']), actor['dps'] / 2000, float(st.get('maxHp', 0)) / 10000, float(st.get('def', 0)) / 1000, float(st.get('res', 0)) / 100, float(at in ground), float(at in airpath), progress[0].get(at, 0), progress[1].get(at, 0), len(actor['cover'] & ground) / max(1, len(ground)), len(actor['cover'] & airpath) / max(1, len(airpath)), *support_features, gc.get(at, 0) / 2000, ac.get(at, 0) / 2000, *[after[i] - before[i] for i in range(6)], float(src in ground), float(src in airpath), float(terrain.get(at) == 'ranged'), float(at in bycell), float(move), float(remove)]
        if len(row) != ACTION_DIM or not all((math.isfinite(x) for x in row)):
            raise ValueError('Nonfinite/misshaped route candidate features')
        cache[key] = row
        rows.append(row)
    summaries = _summaries([change for _, change in pending], ground, airpath, contacts, units)
    for (index, _), summary in zip(pending, summaries):
        delta = [float(summary[i]) - before[i] for i in range(6)]
        if not all(math.isfinite(value) for value in delta):
            raise ValueError('Nonfinite array route summary')
        rows[index][20:26] = delta
    if route_cache is not None:
        for key, index in remember:
            route_cache.remember_action(key, rows[index])
    return (torch.tensor(obs, dtype=torch.float32), torch.tensor(rows, dtype=torch.float32).reshape(-1, ACTION_DIM))
