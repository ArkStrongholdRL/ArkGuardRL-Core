"""Public route features: Python state adapter and Rust candidate encoder."""
from collections import Counter, OrderedDict

import torch
from arkguardrl.candidates import MAX_CANDIDATES
from arkguardrl.contracts import fingerprint
from arkguardrl.domain import DIRECTIONS
from .native_runtime import kernel
from .routes.features import CELLS, context, metrics, unit, validate_geometry

SCHEMA = 'arkguardrl/route-feature-array-execution/v1'
CELL_INDEX = {cell: i for i, cell in enumerate(CELLS)}
DIRECTION_INDEX = {direction: i for i, direction in enumerate(DIRECTIONS)}


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


class RouteFeatureCache:
    """Bounded per-episode public-feature cache; never stores neural activations.

    The PublicCatalog is immutable for an episode. Every call still validates
    the full observation/geometry binding. Economy/history do not enter these
    geometric features; actions and their source/hand destination do.
    """
    def __init__(self, *, max_units=1024):
        if type(max_units) is not int or not 1 <= max_units <= 4096:
            raise ValueError('Explicit bounded route cache capacities required')
        self.max_units = max_units
        self._catalog = None
        self._scope = self._path_key = self._base_key = None
        self._units = OrderedDict()
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


def _encoder(catalog):
    cached = getattr(catalog, '_route_encoder', None)
    if cached is None:
        # The retained baseline metrics use CPython 3.12's compensated sum.
        import platform
        import sys
        if platform.python_implementation() != 'CPython' or sys.version_info[:2] != (3, 12):
            raise RuntimeError('The route backend currently requires CPython3.12')
        definitions, ranges, indices = [], [], {}
        for kind, identifier in catalog.definitions()[0]['keys']:
            if kind != 'chess':
                continue
            rec = catalog._data['chess'][identifier]
            stats = rec.get('stats') or {}
            heal = rec.get('dmgType') == 'heal' or rec.get('attackKind') == 'heal'
            indices[identifier] = len(definitions)
            definitions.append((0. if heal else max(0., float(stats.get('blockCnt', 0))),
                0. if heal else float(stats.get('atk', 0))/max(.1, float(stats.get('bat', 1))),
                float(stats.get('maxHp', 0)), float(stats.get('def', 0)), float(stats.get('res', 0)),
                heal, bool(rec.get('canHitFly', False))))
            ranges.append(catalog.attack_range({'id': identifier}))
        cached = kernel().RouteEncoder(definitions, ranges), indices
        catalog._route_encoder = cached
    return cached


def _mask(cells):
    return sum(1 << CELL_INDEX[cell] for cell in cells)


def encode(state, actions, g, catalog, *, route_cache=None):
    if len(actions) > MAX_CANDIDATES:
        raise ValueError('Route candidate-domain bound exceeded; no slicing')
    if route_cache is None:
        paths, ground, airpath, contacts, progress, units = context(state, g, catalog)
        gc, ac, hc, blockers, before = metrics(units, ground, airpath, contacts)
        obs = _observation(paths, ground, airpath, progress, units, gc, ac, hc, before)
    else:
        paths, ground, airpath, contacts, progress, units, gc, ac, hc, blockers, before, obs = route_cache.begin(state, g, catalog)
    encoder, definitions = _encoder(catalog)
    pieces, indices, base_count = [], {}, 0
    own = state['own']
    for area in ('board', 'hand', 'temp'):
        for p in own[area]:
            if not p or p['kind'] != 'chess':
                continue
            indices[p['uid']] = len(pieces)
            on_board = area == 'board'
            pieces.append((definitions[p['id']], CELL_INDEX[(p['row'], p['col'])] if on_board else -1,
                           DIRECTION_INDEX[p.get('dir', 'RIGHT')]))
            base_count += on_board
    packed = []
    for a in actions:
        to = a.get('to', {})
        move = a['t'] == 'g.move'
        hand = move and to.get('area') == 'hand'
        target = CELL_INDEX[(to['row'], to['col'])] if move and to.get('area') == 'board' else -1
        packed.append((indices.get(a.get('uid'), -1), target, DIRECTION_INDEX[a.get('dir', 'RIGHT')],
                       move, a['t'] == 'g.sell' or hand, bool(hand and own['hand'][to['idx']] is not None)))
    ranged = {(c['row'], c['col']) for c in own['deployment']['cells'] if c['kind'] == 'ranged'}
    required = [_mask({cell, *contacts.get(cell, ())}) for cell in CELLS]
    result = encoder.encode(pieces, base_count, packed, _mask(ground), _mask(airpath), _mask(ranged),
        required, [[p.get(cell, 0.) for cell in CELLS] for p in progress],
        [gc[cell] for cell in CELLS], [ac[cell] for cell in CELLS], [hc[cell] for cell in CELLS], before)
    return torch.tensor(obs, dtype=torch.float32), torch.from_numpy(result)
