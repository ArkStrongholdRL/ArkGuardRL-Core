"""GPL-3.0 decision gamma1, trace decay only across observed settlements."""
SCHEMA='arkguardrl-core/settlement-boundary-gae/v1'


def advantages(contract,rows,lam):
    from arkguardrl import learn
    # Retain native complete-episode/continuity/termination checks first.
    learn._advantages(contract,rows,lam)
    if contract.gamma!=1.:raise ValueError('Settlement GAE requires explicit native gamma1')
    result=[0.]*len(rows);returns=[0.]*len(rows);tail=0.
    for i in reversed(range(len(rows))):
        row=rows[i];t=row.transition
        if type(row.battle_count) is not int or not 0<=row.battle_count<=4:raise ValueError('Explicit bounded actual settlement count required')
        if i==len(rows)-1 or rows[i+1].transition.episode_id!=t.episode_id or t.terminated or t.truncated:tail=0.
        delta=t.reward+(0. if t.terminated else t.next_value)-t.value
        value=delta+(0. if t.terminated or t.truncated else lam**row.battle_count*tail)
        result[i]=value;returns[i]=value+t.value;tail=value
    return result,returns
