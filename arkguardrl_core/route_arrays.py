"""GPL-3.0-or-later array backend for public route features v2.

Derived from scripts/route_features.py. Only after-board summaries are batched;
all original context validation, action order, swaps and withdrawals remain.
No engine execution or future combat score is used here.
"""
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
            if len(self.covers) >= MAX_CANDIDATES:
                self.covers.clear()
                self.blocker_totals.clear()
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


class _SummaryTables:
    """Bounded immutable numeric unit projections for one public route layout."""
    def __init__(self, ground, airpath, contacts, capacity=4096):
        self.key = self.scope(ground, airpath, contacts)
        self.cells = {cell: i for i, cell in enumerate(CELLS)}
        self.ground_mask = np.array([x in ground for x in CELLS])
        self.air_mask = np.array([x in airpath for x in CELLS])
        if any(x not in self.cells or any(p not in self.cells for p in previous)
               for x, previous in contacts.items()):
            raise ValueError('Contact outside route cell domain')
        self.required = tuple(frozenset((x, *contacts.get(x, ()))) for x in CELLS)
        self.ground, self.capacity = frozenset(ground), capacity
        self.rows, self.coverages = OrderedDict(), OrderedDict()
        self.hits = self.misses = 0

    @staticmethod
    def scope(ground, airpath, contacts):
        return (frozenset(ground), frozenset(airpath),
                frozenset((x, frozenset(previous)) for x, previous in contacts.items()))

    def row(self, unit):
        coverage = frozenset(unit['cover'])
        key = (unit['at'], coverage, unit['dps'], unit['air'], unit['heal'], unit['block'])
        if key in self.rows:
            self.hits += 1
            self.rows.move_to_end(key)
            return self.rows[key]
        self.misses += 1
        if coverage not in self.coverages:
            cover = np.array([x in coverage for x in CELLS], dtype=bool)
            support = np.array([needed <= coverage for needed in self.required], dtype=bool)
            cover.flags.writeable = support.flags.writeable = False
            self.coverages[coverage] = cover, support
            if len(self.coverages) > self.capacity:
                self.coverages.popitem(last=False)
        cover, support = self.coverages[coverage]
        self.coverages.move_to_end(coverage)
        result = (cover, support, self.cells[unit['at']], unit['dps'],
                  unit['dps'] if unit['air'] else 0., int(unit['heal']),
                  unit['block'] > 0 and unit['at'] in self.ground)
        self.rows[key] = result
        if len(self.rows) > self.capacity:
            self.rows.popitem(last=False)
        return result


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
        self._summary_tables = self._support = None
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
            self._summary_tables = None
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
            self._summary_tables = None
            self.stats['path_misses'] += 1
        else:
            self.stats['path_hits'] += 1
        if self._summary_tables is None:
            self._summary_tables = _SummaryTables(*self._path[1:4])
        base_key = (scope, path_key, fingerprint(own['board']))
        if self._base_key != base_key:
            self._actions.clear()
            units = [self.get_unit(piece) for piece in own['board'] if piece['kind'] == 'chess']
            paths, ground, airpath, contacts, progress = self._path
            gc, ac, hc, blockers, before = metrics(units, ground, airpath, contacts)
            obs = _observation(paths, ground, airpath, progress, units, gc, ac, hc, before)
            self._base = (*self._path, units, gc, ac, hc, blockers, before, obs)
            self._support = _SupportTable(units, ground, contacts, hc)
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


def _summaries(changes, ground, airpath, contacts, base_units=(), *, tables=None):
    """Ordered base subsets plus appended units, with compensated float64 sums.

    Each change is (removed base indices, appended units). Candidate generation
    already knows this delta; do not rebuild and rediscover complete boards.
    """
    if not changes:
        return np.empty((0, 6), dtype=np.float64)
    if len(changes) > MAX_CANDIDATES or len(base_units) > 128:
        raise ValueError('Array route temporary-domain bound exceeded; no slicing')
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
    tables = tables if tables is not None else _SummaryTables(ground, airpath, contacts)
    if tables.key != tables.scope(ground, airpath, contacts):
        raise ValueError('Numeric route tables belong to a different layout')
    ground_mask, air_mask = tables.ground_mask, tables.air_mask
    supports = np.zeros_like(cover)
    for i, u in enumerate(unique[1:], 1):
        (cover[i], supports[i], positions[i], dps[i], air_dps[i],
         healer[i], blocking[i]) = tables.row(u)
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
        # Integer counts are at most 128/36; float64 division exactly matches
        # the reference's scalar int-to-double arithmetic, without per-row lists.
        output.append(np.column_stack((gh / max(1, len(ground)),
            ah / max(1, len(airpath)), ns / np.maximum(1, block_count),
            block_count / 8, damage / 2000, healing / 8)))
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
    support_table = (route_cache._support if route_cache is not None
                     else _SupportTable(units, ground, contacts, hc))
    cache = {}
    actions = tuple(actions)
    if len(actions) > MAX_CANDIDATES:
        raise ValueError('Array route candidate-domain bound exceeded; no slicing')
    rows = np.zeros((len(actions), ACTION_DIM), dtype=np.float64)
    row_sources = np.arange(len(actions))
    pending = []
    remember = []
    for index, a in enumerate(actions):
        source = pieces.get(a.get('uid'))
        to = a.get('to', {})
        move = a['t'] == 'g.move'
        remove = a['t'] == 'g.sell' or (move and to.get('area') == 'hand')
        target = (to['row'], to['col']) if move and to.get('area') == 'board' else None
        key = (a.get('uid'), target, a.get('dir', 'RIGHT'), remove, to.get('idx'))
        if key in cache:
            row_sources[index] = cache[key]
            continue
        if source and source['kind'] == 'chess' and (move or remove):
            persistent_key = route_cache.action_key(state, a, source) if route_cache is not None else None
            if route_cache is not None:
                cached = route_cache.get_action(persistent_key)
                if cached is not None:
                    cache[key] = index
                    rows[index] = cached
                    continue
                remember.append((persistent_key, index))
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
                appended = (*added, actor) if target else ()
                pending.append((index, (tuple(unit_indices[u['uid']] for u in removed), appended)))
                rows[index] = (float(known), actor['block'] / 8, float(actor['heal']), float(actor['air']), actor['dps'] / 2000, float(st.get('maxHp', 0)) / 10000, float(st.get('def', 0)) / 1000, float(st.get('res', 0)) / 100, float(at in ground), float(at in airpath), progress[0].get(at, 0), progress[1].get(at, 0), len(actor['cover'] & ground) / max(1, len(ground)), len(actor['cover'] & airpath) / max(1, len(airpath)), *support_features, gc.get(at, 0) / 2000, ac.get(at, 0) / 2000, *(0.0,) * 6, float(src in ground), float(src in airpath), float(terrain.get(at) == 'ranged'), float(at in bycell), float(move), float(remove))
        cache[key] = index
    tables = route_cache._summary_tables if route_cache is not None else None
    hits, misses = (tables.hits, tables.misses) if tables is not None else (0, 0)
    summaries = _summaries([change for _, change in pending], ground, airpath, contacts, units, tables=tables)
    if route_cache is not None:
        route_cache.stats['numeric_hits'] += tables.hits-hits
        route_cache.stats['numeric_misses'] += tables.misses-misses
    if pending:
        indices = np.fromiter((index for index, _ in pending), dtype=np.int64, count=len(pending))
        rows[indices, 20:26] = summaries - np.asarray(before, dtype=np.float64)
    # Check every scalar in bulk, including computed deltas and cache hits.
    if not np.isfinite(rows).all():
        raise ValueError('Nonfinite route candidate features')
    if route_cache is not None:
        for key, index in remember:
            route_cache.remember_action(key, rows[index].tolist())
    # Gather aliases only after deltas are filled, matching shared-list rows.
    packed = rows[row_sources].astype(np.float32)
    return torch.tensor(obs, dtype=torch.float32), torch.from_numpy(packed)
