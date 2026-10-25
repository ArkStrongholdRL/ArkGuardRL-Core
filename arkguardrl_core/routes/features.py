"""Public default-range geometry. No combat rollout, optimal-layout score or outcome."""
from functools import lru_cache
import math
import torch
from arkguardrl.structured import CELLS
from arkguardrl.contracts import fingerprint

SCHEMA='arkguardrl/route-contact-support-features/v2'
GRID_FIELDS=('ground_path','air_path','ground_progress','air_progress','blocking','ground_damage_cover','air_damage_cover','healing_cover')
GLOBAL_FIELDS=('route_count','ground_coverage','air_coverage','supported_blockers')
ACTION_FIELDS=('known_chess_geometry','block','healer','anti_air','raw_dps','hp','def','res','target_ground','target_air','target_ground_progress','target_air_progress','ground_range_fraction','air_range_fraction','self_contact_coverage','ally_contact_support','healer_support','supports_other_blockers','ground_damage_at_target','air_damage_at_target','delta_ground_coverage','delta_air_coverage','delta_supported_blockers','delta_blockers','delta_damage_contact','delta_healing_contact','source_ground','source_air','target_ranged','target_occupied','is_move','is_removal')
OBS_DIM=len(CELLS)*len(GRID_FIELDS)+len(GLOBAL_FIELDS)
ACTION_DIM=len(ACTION_FIELDS)

def validate_geometry(state,g):
    if g.get('observation_id')!=fingerprint(state) or g.get('revision')!=state['revision']:raise ValueError('Route snapshot is not bound to this observation')
    if g.get('schema')!='arkguardrl/own-field-route-geometry/v2' or (g['stageId'],g['round'],g['field'])!=(state['public']['stageId'],state['round'],state['own']['deployment']['field']):raise ValueError('Route geometry/state binding mismatch')
    if len(g['routes'])>64:raise ValueError('Too many routes')
    for r in g['routes']:
        if set(r)!={'fly','tiles'} or type(r['fly']) is not bool or len(r['tiles'])>256:raise ValueError('Route shape/budget')
        if any(type(x) is not list or len(x)!=2 or tuple(x) not in CELLS for x in r['tiles']):raise ValueError('Route outside own field')

def unit(catalog,p):
    rec=catalog._data['chess'][p['id']];stats=rec.get('stats') or {}
    heal=rec.get('dmgType')=='heal' or rec.get('attackKind')=='heal'
    block=0 if heal else max(0,float(stats.get('blockCnt',0)))
    dps=0 if heal else float(stats.get('atk',0))/max(.1,float(stats.get('bat',1)))
    air=bool(rec.get('canHitFly',False))
    at=(p['row'],p['col']);cover=set()
    for dr,dc in catalog.attack_range(p):
        rr,cc={'RIGHT':(dr,dc),'UP':(dc,-dr),'LEFT':(-dr,-dc),'DOWN':(-dc,dr)}[p.get('dir','RIGHT')];cover.add((at[0]+rr,at[1]+cc))
    return {'uid':p['uid'],'at':at,'cover':cover,'block':block,'heal':heal,'air':air,'dps':dps,'stats':stats}

def context(state,g,catalog):
    validate_geometry(state,g)
    paths=[(r['fly'],[tuple(x) for x in r['tiles']]) for r in g['routes']]
    ground={x for air,tiles in paths if not air for x in tiles};airpath={x for air,tiles in paths if air for x in tiles}
    contacts={}
    progress=[{},{}]
    for air,tiles in paths:
        for i,x in enumerate(tiles):
            progress[int(air)][x]=max(progress[int(air)].get(x,0),i/max(1,len(tiles)-1))
            if not air:contacts.setdefault(x,set()).add(tiles[max(0,i-1)])
    units=[unit(catalog,p) for p in state['own']['board'] if p['kind']=='chess']
    return paths,ground,airpath,contacts,progress,units

def metrics(units,ground,airpath,contacts):
    gc={x:sum(u['dps'] for u in units if x in u['cover']) for x in CELLS}
    ac={x:sum(u['dps'] for u in units if u['air'] and x in u['cover']) for x in CELLS}
    hc={x:sum(u['heal'] for u in units if x in u['cover']) for x in CELLS}
    blockers=[u for u in units if u['block']>0 and u['at'] in ground]
    supported=[u for u in blockers if any(v['uid']!=u['uid'] and v['dps']>0 and u['at'] in v['cover'] and all(x in v['cover'] for x in contacts.get(u['at'],())) for v in units)]
    summary=(sum(gc[x]>0 for x in ground)/max(1,len(ground)),sum(ac[x]>0 for x in airpath)/max(1,len(airpath)),len(supported)/max(1,len(blockers)),len(blockers)/8,sum(gc[u['at']] for u in blockers)/2000,sum(hc[u['at']] for u in blockers)/8)
    return gc,ac,hc,blockers,summary

def encode(state,actions,g,catalog):
    paths,ground,airpath,contacts,progress,units=context(state,g,catalog)
    gc,ac,hc,blockers,before=metrics(units,ground,airpath,contacts)
    obs=[]
    for x in CELLS:obs.extend((float(x in ground),float(x in airpath),progress[0].get(x,0),progress[1].get(x,0),sum(u['block'] for u in units if u['at']==x)/8,gc[x]/2000,ac[x]/2000,hc[x]/4))
    obs.extend((len(paths)/16,*before[:3]))
    pieces={p['uid']:p for area in ('board','hand','temp') for p in state['own'][area] if p}
    byuid={u['uid']:u for u in units};bycell={u['at']:u for u in units}
    terrain={(x['row'],x['col']):x['kind'] for x in state['own']['deployment']['cells']}
    cache={};rows=[]
    for a in actions:
        source=pieces.get(a.get('uid'));to=a.get('to',{});move=a['t']=='g.move';remove=a['t']=='g.sell' or move and to.get('area')=='hand'
        target=(to['row'],to['col']) if move and to.get('area')=='board' else None
        key=(a.get('uid'),target,a.get('dir','RIGHT'),remove,to.get('idx'))
        if key in cache:rows.append(cache[key]);continue
        row=[0.]*ACTION_DIM
        if source and source['kind']=='chess' and (move or remove):
            old=byuid.get(source['uid']);src=old['at'] if old else None
            afterunits=[u for u in units if u['uid']!=source['uid']];actor=old
            known=True
            if target:
                occupant=bycell.get(target)
                if occupant and occupant['uid']!=source['uid']:
                    afterunits=[u for u in afterunits if u['uid']!=occupant['uid']]
                    if old:
                        p=dict(pieces[occupant['uid']],row=src[0],col=src[1]);afterunits.append(unit(catalog,p))
                actor=unit(catalog,dict(source,row=target[0],col=target[1],dir=a.get('dir','RIGHT')));afterunits.append(actor)
            elif move and to.get('area')=='hand' and state['own']['hand'][to['idx']] is not None:known=False
            if actor:
                at=target or actor['at'];others=[u for u in afterunits if u['uid']!=actor['uid']]
                support=any(u['dps']>0 and at in u['cover'] and all(x in u['cover'] for x in contacts.get(at,())) for u in others)
                st=actor['stats'];after=metrics(afterunits,ground,airpath,contacts)[4]
                row=[float(known),actor['block']/8,float(actor['heal']),float(actor['air']),actor['dps']/2000,float(st.get('maxHp',0))/10000,float(st.get('def',0))/1000,float(st.get('res',0))/100,float(at in ground),float(at in airpath),progress[0].get(at,0),progress[1].get(at,0),len(actor['cover']&ground)/max(1,len(ground)),len(actor['cover']&airpath)/max(1,len(airpath)),float(at in actor['cover'] and all(x in actor['cover'] for x in contacts.get(at,()))),float(support),min(1,sum(u['heal'] and at in u['cover'] for u in others)/2),sum(u['block']>0 and u['at'] in ground and u['at'] in actor['cover'] and all(x in actor['cover'] for x in contacts.get(u['at'],())) for u in others)/8,gc.get(at,0)/2000,ac.get(at,0)/2000,*[after[i]-before[i] for i in range(6)],float(src in ground),float(src in airpath),float(terrain.get(at)=='ranged'),float(at in bycell),float(move),float(remove)]
        if len(row)!=ACTION_DIM or not all(math.isfinite(x) for x in row):raise ValueError('Nonfinite/misshaped route candidate features')
        cache[key]=row;rows.append(row)
    return torch.tensor(obs,dtype=torch.float32),torch.tensor(rows,dtype=torch.float32).reshape(-1,ACTION_DIM)
