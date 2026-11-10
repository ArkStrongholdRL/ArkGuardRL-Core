"""Single-env native/Gym throughput on identical, complete Desperation episodes."""
import argparse
import json
from pathlib import Path
import platform
import statistics
import time

try:
    import resource
except ImportError:  # Windows
    resource = None

from arkguardrl_core.native_env import load_native
from arkguardrl_core.native_gym import ArkGuardNativeEnv

CORES = ('band_amiya', 'band_sarkazb', 'band_chen')


def peak_rss_bytes():
    """Peak resident set where the platform reports it, else current RSS."""
    if resource is not None:
        scale = 1 if platform.system() == 'Darwin' else 1024
        return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * scale
    import psutil
    info = psutil.Process().memory_info()
    return int(getattr(info, 'peak_wset', info.rss))


def choose(env, roster, bought):
    state = env._result.observation
    own, actions = state['own'], env._decision.actions
    for i, action in enumerate(actions):
        if action['t'] == 'g.choice':
            return i
    if roster:
        hand = {p['uid'] for p in own['hand'] if p and p['kind'] == 'chess'}
        occupied = {(p['row'], p['col']) for p in own['board']}
        moves = [(i, a) for i, a in enumerate(actions) if a['t'] == 'g.move'
                 and a['uid'] in hand and a['to']['area'] == 'board' and a.get('dir') == 'RIGHT'
                 and (a['to']['row'], a['to']['col']) not in occupied]
        if moves and own['deployCount'] < own['deployCap']:
            return max(moves, key=lambda x: (x[1]['to']['col'], x[1]['to']['row']))[0]
        if state['round'] not in bought and None in own['hand']:
            bought.add(state['round'])
            for i, action in enumerate(actions):
                if action['t'] == 'g.buy':
                    slot = own['shop']['slots'][action['slot']]
                    if slot and slot['kind'] == 'chess' and not slot['sold'] and slot['price'] <= own['funds']:
                        return i
    return next(i for i, a in enumerate(actions) if a['t'] == 'g.ready')


def ending(state):
    return {key: state[key] for key in ('round', 'own', 'outcome')}


def pilot(env, seed, roster, chooser=None):
    _, info = env.reset(seed=seed)
    engine_seed, actions, bought = info['engine_seed'], [], set()
    for _ in range(150):
        index = (chooser or choose)(env, roster, bought)
        actions.append(env._decision.actions[index])
        _, _, terminated, truncated, info = env.step(index)
        if truncated or not info.get('accepted'):
            raise RuntimeError(f'Pilot did not execute a complete valid episode: {info}')
        if terminated:
            return engine_seed, actions, ending(env._result.observation)
    raise RuntimeError('Pilot exceeded decision limit')


def measure(env, session, cases, cycles, backend):
    durations, steps, ticks, candidate_max = [], 0, 0, 0
    phases = dict(engine_seconds=0., encoding_seconds=0., reward_seconds=0.)
    reset_seconds = combat_seconds = step_seconds = 0.
    for _ in range(cycles):
        for seed, engine_seed, actions, expected in cases:
            begin = time.perf_counter()
            if backend == 'gym':
                observation, info = env.reset(seed=seed)
                candidate_max = max(candidate_max, info['candidate_count'])
            else:
                session.reset(engine_seed)
                json.loads(session.observation())
            reset_seconds += time.perf_counter() - begin
            for action in actions:
                if backend == 'gym':
                    index = env._decision.actions.index(action)
                    start = time.perf_counter()
                    observation, _, terminated, truncated, info = env.step(index)
                    step_seconds += time.perf_counter() - start
                    if truncated or not info.get('accepted'):
                        raise RuntimeError('Rejected or truncated benchmark')
                    candidate_max = max(candidate_max, info['candidate_count'])
                    for key in phases:
                        phases[key] += info['timing'].get(key, 0.)
                else:
                    start = time.perf_counter()
                    receipt = json.loads(session.step(json.dumps(action), 108000))
                    elapsed = time.perf_counter() - start
                    step_seconds += elapsed
                    if not receipt['accepted'] or receipt['budgetExhausted']:
                        raise RuntimeError('Rejected or incomplete benchmark')
                    ticks += receipt['ticks']
                    if receipt['ticks']:
                        combat_seconds += elapsed
                    state = json.loads(session.observation())
                steps += 1
            durations.append(time.perf_counter() - begin)
            state = env._result.observation if backend == 'gym' else state
            if not state['gameEnded'] or ending(state) != expected:
                raise RuntimeError('Replay changed outcome or public terminal state')
    seconds = sum(durations)
    result = dict(episodes=len(durations), decisions=steps, seconds=seconds,
                  episodes_per_second=len(durations)/seconds, decisions_per_second=steps/seconds,
                  median_episode_ms=statistics.median(durations)*1000,
                  reset_seconds=reset_seconds, step_seconds=step_seconds)
    if backend == 'gym':
        result.update(phase_seconds=phases, candidate_max=candidate_max,
                      observation_bytes=sum(a.nbytes for a in observation.values()))
    else:
        result.update(combat_ticks=ticks, combat_step_seconds=combat_seconds,
                      combat_ticks_per_second=ticks/combat_seconds if combat_seconds else 0.)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data-root', required=True)
    parser.add_argument('--native-library', required=True)
    parser.add_argument('--cycles', type=int, default=5)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if min(args.cycles, args.repeats) < 1:
        parser.error('Positive cycles and repeats required')
    native = load_native(args.native_library)
    catalog = native.NativeCatalog(args.data_root, 64 << 20)
    results = []
    startup = time.perf_counter()
    for core in CORES:
        session = native.NativeSolo(catalog, core)
        with ArkGuardNativeEnv(args.data_root, core=core, native_library=args.native_library,
                               profile=True) as env:
            for roster in (False, True):
                cases = [(seed, *pilot(env, seed, roster)) for seed in (42, 99)]
                # Warm both paths; data loading/compilation are outside steady-state metrics.
                measure(env, session, cases, 1, 'native')
                for backend in ('native', 'gym'):
                    runs = [measure(env, session, cases, args.cycles, backend) for _ in range(args.repeats)]
                    result = dict(core=core, workload='buy_deploy' if roster else 'empty', backend=backend,
                                  terminal_rounds=[case[3]['round'] for case in cases], runs=runs,
                                  median_decisions_per_second=statistics.median(r['decisions_per_second'] for r in runs),
                                  median_episodes_per_second=statistics.median(r['episodes_per_second'] for r in runs))
                    results.append(result)
                    print(json.dumps({k:v for k,v in result.items() if k != 'runs'}), flush=True)
        session.close()
    output = dict(platform=platform.platform(), python=platform.python_version(),
                  scope='Single env; no model, training or scheduling. Same actions and terminal states across backends.',
                  timing='Includes episode reset and action lookup; excludes initialization/pilot/terminal equality check.',
                  seeds=[42, 99], cycles=args.cycles, repeats=args.repeats,
                  wall_seconds=time.perf_counter()-startup,
                  peak_rss_bytes=peak_rss_bytes(),
                  results=results)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(output, indent=2)+'\n')


if __name__ == '__main__':
    main()
