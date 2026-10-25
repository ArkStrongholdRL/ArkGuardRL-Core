"""GPL-3.0 explicitly configured public construction-potential shaping.

No built-in card whitelist. Experiment supplies bond, candidate anchor IDs and
bounded weights. Potential vanishes at personal terminal, not at truncation.
"""
import math

SCHEMA = 'arkguardrl-core/public-construction-potential/v1'


def validate(config):
    if config is None:
        return
    if set(config) != {'schema','bond','anchors','owned_members','board_members','weights','layer_scale'}:
        raise ValueError('Explicit construction potential configuration required')
    if config['schema'] != SCHEMA or not isinstance(config['bond'],str):
        raise ValueError('Unknown construction shaping schema')
    if not isinstance(config['anchors'],(tuple,list)) or not config['anchors'] or len(set(config['anchors'])) != len(config['anchors']):
        raise ValueError('Distinct public anchor identities required')
    for k in ('owned_members','board_members'):
        if type(config[k]) is not int or not 1 <= config[k] <= 12:
            raise ValueError('Construction count threshold outside domain')
    if len(config['weights']) != 4 or not all(math.isfinite(w) and 0 <= w <= .25 for w in config['weights']):
        raise ValueError('Four finite nonnegative bounded potential weights required')
    if sum(config['weights']) > .5 or not math.isfinite(config['layer_scale']) or config['layer_scale'] <= 0:
        raise ValueError('Construction potential bound/scale invalid')


def potential(state,public,config):
    if config is None or state['personalEnded']:
        return 0.
    own=state['own'];bond=config['bond']
    status=next((b for b in own['bonds'] if b['bondId']==bond),{})
    if bond in state['public']['disabledBonds'] or status.get('off'):
        return 0.
    board=set();owned=set();board_anchor=False;owned_anchor=False
    anchors=set(config['anchors'])
    for area in ('board','hand'):
        for piece in own[area]:
            if not piece or piece['kind']!='chess':continue
            rec=public.record('chess',piece['id'])
            if bond not in rec.get('bonds',[]):continue
            base=rec.get('baseId',piece['id'])
            owned.add(base);owned_anchor |= base in anchors
            if area=='board':board.add(base);board_anchor |= base in anchors
    active=bool(status.get('active')) and board_anchor
    stages=(owned_anchor and len(owned)>=config['owned_members'] and own['deployCap']>=config['owned_members'],
        board_anchor and len(board)>=config['board_members'],active,
        math.tanh(max(0,status.get('layers',0))/config['layer_scale']) if active else 0.)
    return math.fsum(w*float(stage) for w,stage in zip(config['weights'],stages))
