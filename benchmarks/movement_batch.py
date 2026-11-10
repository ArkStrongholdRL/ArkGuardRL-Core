"""A/B complete Desperation episodes and measure movement-window coverage."""
import argparse
import json
from pathlib import Path
import statistics

from arkguardrl_core.native_env import load_native
from arkguardrl_core.native_gym import ArkGuardNativeEnv
from benchmarks.native_session import choose, ending, measure, pilot
from benchmarks.ledger_compare import parity


def choose_rear(env, roster, bought):
    index = choose(env, roster, bought)
    chosen = env._decision.actions[index]
    if chosen['t'] != 'g.move':
        return index
    occupied = {(p['row'], p['col']) for p in env._result.observation['own']['board']}
    moves = [(i, action) for i, action in enumerate(env._decision.actions)
             if action['t'] == 'g.move' and action['uid'] == chosen['uid']
             and action['to']['area'] == 'board' and action.get('dir') == 'RIGHT'
             and (action['to']['row'], action['to']['col']) not in occupied]
    return min(moves, key=lambda item: (item[1]['to']['col'], item[1]['to']['row']))[0]


def coverage(native, catalog, core, cases):
    session = native.NativeSolo(catalog, core)
    ticks = batched = tail = battles = 0
    fast_updates = enemy_updates = 0
    event_ticks = event_windows = 0
    deployed_battles = 0
    try:
        for _, engine_seed, actions, expected in cases:
            session.reset(engine_seed)
            for action in actions:
                receipt = json.loads(session.step(json.dumps(action), 0))
                if receipt['accepted'] is not True:
                    raise RuntimeError('Rejected coverage action')
                count = 0
                paths = (0, 0)
                windows = (0, 0)
                if receipt['budgetExhausted']:
                    battles += 1
                    deployed_battles += bool(json.loads(session.observation())['own']['board'])
                while receipt['budgetExhausted']:
                    receipt = json.loads(session.advance(64))
                    ticks += receipt['ticks']
                    observed = session.movement_batch_ticks()
                    movement_paths = session.movement_path_steps()
                    observed_windows = session.event_window_stats()
                    if observed_windows is not None:
                        windows = observed_windows
                    if movement_paths is not None:
                        paths = movement_paths
                    if observed is not None:
                        count = observed
                    else:
                        # Settlement releases the Field. Bound the unobserved last chunk.
                        tail += receipt['ticks']
                batched += count
                fast_updates += paths[0]
                enemy_updates += paths[1]
                event_ticks += windows[0]
                event_windows += windows[1]
            if ending(json.loads(session.observation())) != expected:
                raise RuntimeError('Coverage replay changed public terminal state')
    finally:
        session.close()
    return dict(ticks=ticks, battles=battles, deployed_battles=deployed_battles,
                observed_event_ticks=event_ticks, observed_event_windows=event_windows,
                mean_event_window_ticks=event_ticks / event_windows if event_windows else 0.,
                batch_ticks_lower_bound=batched,
                observed_fast_enemy_updates=fast_updates, observed_enemy_updates=enemy_updates,
                observed_fast_enemy_fraction=fast_updates / enemy_updates if enemy_updates else 0.,
                excluded_settlement_ticks=tail,
                batch_ticks_upper_bound=min(ticks, batched + tail),
                lower_fraction=batched / ticks if ticks else 0.,
                upper_fraction=min(ticks, batched + tail) / ticks if ticks else 0.)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-root', required=True)
    parser.add_argument('--native-library', required=True)
    parser.add_argument('--reference-library', required=True)
    parser.add_argument('--core', default='band_chen', choices=('band_chen', 'band_amiya', 'band_sarkazb'))
    parser.add_argument('--workload', default='both', choices=('empty', 'buy_deploy', 'both'))
    parser.add_argument('--deployment', default='front', choices=('front', 'rear'))
    parser.add_argument('--cycles', type=int, default=20)
    parser.add_argument('--repeats', type=int, default=6)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if min(args.cycles, args.repeats) < 1:
        parser.error('Positive cycles and repeats required')
    native = load_native(args.native_library)
    catalog = native.NativeCatalog(args.data_root, 64 << 20)
    results, compared = [], 0
    with ArkGuardNativeEnv(args.data_root, core=args.core, native_library=args.reference_library, profile=True) as before, \
            ArkGuardNativeEnv(args.data_root, core=args.core, native_library=args.native_library, profile=True) as after:
        rosters = (False, True) if args.workload == 'both' else (args.workload == 'buy_deploy',)
        for roster in rosters:
            chooser = choose_rear if args.deployment == 'rear' else None
            cases = [(seed, *pilot(before, seed, roster, chooser=chooser)) for seed in (42, 99)]
            for seed, _, actions, _ in cases:
                compared += parity(before, after, seed, actions)
            windows = coverage(native, catalog, args.core, cases)
            for env in (before, after):
                measure(env, None, cases, 2, 'gym')
            runs = {'before': [], 'after': []}
            for repeat in range(args.repeats):
                order = (('before', before), ('after', after))
                if repeat % 2:
                    order = order[::-1]
                for name, env in order:
                    runs[name].append(measure(env, None, cases, args.cycles, 'gym'))
            medians = {name: statistics.median(run['decisions_per_second'] for run in values)
                       for name, values in runs.items()}
            result = dict(workload='buy_deploy' if roster else 'empty', coverage=windows,
                          decisions_per_second=medians, speedup=medians['after'] / medians['before'], runs=runs)
            results.append(result)
            print(json.dumps({key: value for key, value in result.items() if key != 'runs'}), flush=True)
    output = dict(core=args.core, deployment=args.deployment, seeds=[42, 99], cycles=args.cycles, repeats=args.repeats,
                  order='alternating', parity_decisions=compared,
                  scope='Complete Gym episodes, same Python path and action sequence, before/after native libraries.',
                  results=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(output, indent=2) + '\n')


if __name__ == '__main__':
    main()
