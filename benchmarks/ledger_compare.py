"""Compare Python/Rust ledgers on the same complete Desperation episodes."""
import argparse
import json
from pathlib import Path
import statistics
import struct

import numpy as np

from arkguardrl_core.native_gym import ArkGuardNativeEnv
from benchmarks.native_session import measure, pilot


def public_stream(value):
    # Each reset deliberately creates a fresh UUID. All other fields must match.
    if isinstance(value, dict):
        return {key: '<stream>' if key == 'stream_id' else public_stream(item)
                for key, item in value.items()}
    if isinstance(value, list):
        return [public_stream(item) for item in value]
    return value


def compare(left, right, arrays_left, arrays_right, histories):
    a, b = left._result, right._result
    assert a.observation == b.observation, 'Public observation differs'
    assert (a.terminated, a.truncated, a.reason, a.accepted, a.action_id) == (
        b.terminated, b.truncated, b.reason, b.accepted, b.action_id), 'Transition differs'
    assert public_stream(a.execution) == public_stream(b.execution), 'Execution differs'
    assert public_stream(a.progress) == public_stream(b.progress), 'Progress differs'
    ah, bh = histories
    assert public_stream(ah.snapshot(a.observation)) == public_stream(bh.snapshot(b.observation)), 'History differs'
    assert ah.blocked(a.observation) == bh.blocked(b.observation), 'Blocked actions differ'
    if left._decision is not None:
        assert left._decision.actions == right._decision.actions, 'Candidate order differs'
        assert public_stream(json.loads(left._decision.history_json)) == public_stream(
            json.loads(right._decision.history_json)), 'Decision history differs'
    else:
        assert right._decision is None, 'Terminal candidates differ'
    for key in arrays_left:
        assert arrays_left[key].dtype == arrays_right[key].dtype, key
        assert arrays_left[key].shape == arrays_right[key].shape, key
        assert arrays_left[key].tobytes() == arrays_right[key].tobytes(), key


def compare_reward(a, b, info_a, info_b):
    assert struct.pack('d', a) == struct.pack('d', b), 'Reward bits differ'
    for key, value in info_a['reward_components'].items():
        assert struct.pack('d', value) == struct.pack('d', info_b['reward_components'][key]), key


def parity(left, right, seed, actions):
    a, _ = left.reset(seed=seed)
    b, _ = right.reset(seed=seed)
    histories = left._session.history, right._session.history
    compare(left, right, a, b, histories)
    for action in actions:
        index_a = left._decision.actions.index(action)
        index_b = right._decision.actions.index(action)
        a, reward_a, done_a, trunc_a, info_a = left.step(index_a)
        b, reward_b, done_b, trunc_b, info_b = right.step(index_b)
        assert (done_a, trunc_a) == (done_b, trunc_b)
        compare_reward(reward_a, reward_b, info_a, info_b)
        compare(left, right, a, b, histories)
    return len(actions)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-root', required=True)
    parser.add_argument('--native-library', required=True)
    parser.add_argument('--cycles', type=int, default=20)
    parser.add_argument('--repeats', type=int, default=5)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if min(args.cycles, args.repeats) < 1:
        parser.error('Positive cycles and repeats required')
    options = dict(core='band_chen', native_library=args.native_library, profile=True)
    results, parity_steps = [], 0
    with ArkGuardNativeEnv(args.data_root, native_ledger=False, **options) as left, \
            ArkGuardNativeEnv(args.data_root, native_ledger=True, **options) as right:
        for roster in (False, True):
            cases = [(seed, *pilot(left, seed, roster)) for seed in (42, 99)]
            for seed, _, actions, _ in cases:
                parity_steps += parity(left, right, seed, actions)
            # The arrays returned by one decision must survive later decisions.
            observation, _ = right.reset(seed=42)
            saved = {key: value.copy() for key, value in observation.items()}
            funds = right._result.observation['own']['funds']
            right._decision.state['own']['funds'] += 1
            assert right._result.observation['own']['funds'] == funds, 'Decision snapshot aliases live state'
            right.step(0)
            assert all(np.array_equal(value, saved[key]) for key, value in observation.items())
            for env in (left, right):
                measure(env, None, cases, 2, 'gym')
            runs = {'python': [], 'rust': []}
            for repeat in range(args.repeats):
                order = (('python', left), ('rust', right))
                if repeat % 2:
                    order = order[::-1]
                for name, env in order:
                    runs[name].append(measure(env, None, cases, args.cycles, 'gym'))
            medians = {name: statistics.median(run['decisions_per_second'] for run in values)
                       for name, values in runs.items()}
            result = dict(workload='buy_deploy' if roster else 'empty',
                          decisions_per_second=medians, speedup=medians['rust'] / medians['python'],
                          runs=runs)
            results.append(result)
            print(json.dumps({key: value for key, value in result.items() if key != 'runs'}), flush=True)
    output = dict(core='band_chen', seeds=[42, 99], cycles=args.cycles, repeats=args.repeats,
                  scope='Complete Gym decisions, same release library, alternating Python/Rust ledger runs.',
                  parity_decisions=parity_steps, results=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(output, indent=2) + '\n')


if __name__ == '__main__':
    main()
