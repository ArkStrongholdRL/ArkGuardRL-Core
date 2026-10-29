"""Adapt public inventory and terrain to the Rust PREP action domain."""
from arkguardrl.catalog import UnsupportedPublicState
from arkguardrl.domain import DIRECTIONS
from .native_runtime import kernel


def piece_actions(state, catalog, context, *, recovery):
    from arkguardrl.candidates import empty_deployment_allowed

    own = state['own']
    if recovery and own['canReady']:
        return []
    locations, occupied, cells = context
    terrain = {'all': 0, 'melee': 0, 'high': 0}
    for (row, col), kind in cells.items():
        bit = 1 << ((row - 9) * 9 + col - 2)
        terrain['all'] |= bit
        terrain['melee' if kind == 'melee' else 'high'] |= bit
    pieces = []
    for uid, (area, _, piece) in locations.items():
        if area == 'equipped':
            continue
        kind = {'chess': 0, 'token': 1, 'item': 2}[piece['kind']]
        mask = 0
        if not recovery or area == 'temp':
            if kind <= 1:
                mask = terrain[catalog.placement_class(piece)] if cells else 0
                if kind == 1:
                    mask = sum(1 << ((row - 9) * 9 + col - 2)
                               for row, col in cells
                               if mask & (1 << ((row - 9) * 9 + col - 2))
                               and ((area == 'board' and (row, col) in occupied)
                                    or empty_deployment_allowed(piece, (row, col), state, catalog, context)))
                    # Occupied destinations retain native swap candidates:
                    # swapping with the owner can change the legal range.
            else:
                capability = catalog.record('item', piece['id']).get('itemType')
                if capability not in ('EQUIP', 'MAGIC'):
                    raise UnsupportedPublicState('Unknown item capability')
                kind = 2 if capability == 'EQUIP' else 3
        cell = (piece['row'] - 9) * 9 + piece['col'] - 2 if area == 'board' else -1
        direction = DIRECTIONS.index(piece['dir']) if area == 'board' else -1
        pieces.append((uid, {'board': 0, 'hand': 1, 'temp': 2}[area], kind,
                       cell, direction, mask, [item['uid'] for item in piece['items']]))
    empty_hand = sum(1 << i for i, piece in enumerate(own['hand']) if piece is None)
    return kernel().prep_piece_actions(pieces, empty_hand,
        own['deployCount'] >= own['deployCap'], recovery, own['canReady'])
