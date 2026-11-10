"""Replay complete human solo Desperation records and compare decision state.

Checks phase/round, inventory, economy, deployment, bonds, choices, shop and final
victory/round count/hidden progress. Display metadata and combat floating-point
accounting are outside this comparison. No source revision gate is applied.
"""
import argparse
from collections import Counter
import json
from pathlib import Path

from arkguardrl_core.native_env import load_native

CORES = {'band_chen', 'band_amiya', 'band_sarkazb', 'band_sciurus'}
OWN_FIELDS = ('alive', 'lp', 'funds', 'bandId', 'ready', 'canReady', 'deployCap',
              'deployCount', 'board', 'hand', 'temp', 'bonds', 'personalChoice')
SHOP_FIELDS = ('level', 'maxLevel', 'upgradePrice', 'refreshPrice', 'freeRefreshes', 'frozen')
SLOT_FIELDS = ('kind', 'id', 'price', 'sold', 'frozen')
OUTCOME_FIELDS = ('victory', 'roundsPassed', 'hiddenReached', 'hiddenCleared')


def first_difference(expected, actual, path):
    if expected == actual:
        return None
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in sorted(expected.keys() | actual.keys()):
            difference = first_difference(expected.get(key, '<missing>'),
                                          actual.get(key, '<missing>'), f'{path}.{key}')
            if difference is not None:
                return difference
    if isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            return dict(field=path+'.length', expected=len(expected), actual=len(actual))
        for index, (left, right) in enumerate(zip(expected, actual)):
            difference = first_difference(left, right, f'{path}[{index}]')
            if difference is not None:
                return difference
    return dict(field=path, expected=expected, actual=actual)


def decision_state(state):
    own = state['own']
    shop = own['shop']
    result = {key: own.get(key) for key in OWN_FIELDS}
    result['shop'] = {key: shop.get(key) for key in SHOP_FIELDS}
    result['shop']['slots'] = [None if slot is None else {key: slot.get(key) for key in SLOT_FIELDS}
                               for slot in shop['slots']]
    result['phase'] = state['public']['phase']
    result['round'] = state['public']['round']
    return result


def read_case(path):
    if path.stat().st_size > 64 << 20:
        raise ValueError('Record exceeds 64 MiB')
    header = footer = outcome = core = None
    actions = []
    setup_revision = 0
    with path.open() as stream:
        for line in stream:
            row = json.loads(line)
            if header is None:
                header = row
            elif row['kind'] == 'footer':
                footer = row
            elif row['kind'] == 'outcome-message' and row['message']['t'] == 'm.result':
                outcome = row['message']
            elif row['kind'] == 'action':
                action = {key: value for key, value in row['action'].items() if key != 'rid'}
                if action['t'] == 'room.rerollSetup' and row['accepted'] and not actions:
                    if action.get('setupRevision') != setup_revision:
                        return None, 'nonsequential setup reroll'
                    setup_revision += 1
                elif not action['t'].startswith('g.') and not row['accepted']:
                    continue
                elif action['t'] == 'g.band':
                    if row['accepted']:
                        core = action['bandId']
                elif action['t'] not in ('g.infoReady', 'g.unitStats'):
                    before = decision_state(row['before']) if action['t'].startswith('g.') else {}
                    actions.append((action, row['accepted'], before))
    if not header or header.get('schema') != 'stronghold-human-seat-trajectory/v1':
        return None, 'record schema'
    if header.get('roomMode') != 'solo' or header.get('difficulty') != 'ABYSS':
        return None, 'mode outside solo Desperation'
    if not footer or footer.get('complete') is not True or outcome is None:
        return None, 'incomplete recording'
    if core not in CORES:
        return None, f'unsupported core {core}'
    if type(header.get('seed')) is not int:
        return None, 'missing original seed'
    if any(not action['t'].startswith('g.') for action, _, _ in actions):
        return None, 'room setup changes'
    return dict(core=core, seed=header['seed'], actions=actions, outcome=outcome,
                diy=header.get('diy', {}), setup_revision=setup_revision), None


def replay(native, catalog, case):
    session = None
    differences = Counter()
    first = None
    checked = 0
    try:
        session = native.NativeSolo(catalog, case['core'], diy_json=json.dumps(case['diy']), setup_revision=case['setup_revision'])
        session.reset(case['seed'])
        for index, (action, accepted, expected) in enumerate(case['actions']):
            actual = decision_state(json.loads(session.observation()))
            for key in expected:
                if actual[key] != expected[key]:
                    differences[key] += 1
                    if first is None:
                        first = dict(action=index, round=expected['round'],
                                     **first_difference(expected[key], actual[key], key))
            receipt = json.loads(session.step(json.dumps(action), 108000))
            checked += 1
            if receipt['accepted'] != accepted:
                raise RuntimeError(f'Action {index} {action["t"]} acceptance differs')
            for _ in range(16):
                if not receipt['budgetExhausted']:
                    break
                receipt = json.loads(session.advance(108000))
            if receipt['budgetExhausted']:
                raise RuntimeError('Battle exceeded replay tick budget')
        state = json.loads(session.observation())
        if not state['personalEnded']:
            raise RuntimeError('Recording ended before native termination')
        outcome = state['outcome']
        actual_outcome = {key: outcome.get(key) for key in OUTCOME_FIELDS}
        actual_outcome['roundsPassed'] = outcome['players'][0]['roundsPassed']
        expected_outcome = {key: case['outcome'].get(key) for key in OUTCOME_FIELDS}
        if actual_outcome != expected_outcome:
            differences['outcome'] += 1
            if first is None:
                first = dict(field='outcome', expected=expected_outcome, actual=actual_outcome)
        return dict(status='drift' if differences else 'passed', decisions=checked,
                    differences=dict(differences), first_difference=first,
                    expected_outcome=expected_outcome, actual_outcome=actual_outcome)
    except (RuntimeError, ValueError) as error:
        return dict(status='error', decisions=checked, error=str(error),
                    differences=dict(differences), first_difference=first)
    finally:
        if session is not None:
            session.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--records', type=Path, required=True)
    parser.add_argument('--data-root', required=True)
    parser.add_argument('--native-library', required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    native = load_native(args.native_library)
    catalog = native.NativeCatalog(args.data_root, 64 << 20)
    results = []
    paths = sorted(args.records.glob('*.jsonl'))
    if not paths:
        parser.error('No JSONL records found')
    for index, path in enumerate(paths):
        try:
            case, reason = read_case(path)
        except (ValueError, KeyError) as error:
            results.append(dict(file=path.name, status='error', error=f'Record input: {error}'))
            continue
        if case is None:
            result = dict(file=path.name, status='skipped', reason=reason)
        else:
            result = dict(file=path.name, core=case['core'], seed=case['seed'], **replay(native, catalog, case))
            if case['diy']:
                result['diy'] = case['diy']
        results.append(result)
        if (index + 1) % 20 == 0:
            print(json.dumps(dict(records=index+1, counts=dict(Counter(r['status'] for r in results)))), flush=True)
    summary = dict(records=len(results), counts=dict(Counter(r['status'] for r in results)),
                   decisions=sum(r.get('decisions', 0) for r in results), results=results)
    args.out.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({key: value for key, value in summary.items() if key != 'results'}), flush=True)
    if any(r['status'] in ('drift', 'error') for r in results):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
