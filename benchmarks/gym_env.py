"""Model-free Gymnasium step benchmark. Run from an installed Core checkout."""
import argparse
from functools import partial
import hashlib
import json
from pathlib import Path
import statistics
import sys
import time

import gymnasium as gym
import numpy as np
from arkguardrl_core.gym_env import ArkGuardEnv


def rollout(env, steps, seed, rng_seed):
    reset_started = time.perf_counter()
    observation, _ = env.reset(seed=seed)
    initial_reset_seconds = time.perf_counter()-reset_started
    rngs = [np.random.default_rng(s) for s in np.random.SeedSequence(rng_seed).spawn(env.num_envs)]
    digest = hashlib.sha256()
    phases = dict(engine_seconds=0., encoding_seconds=0., reward_seconds=0.)
    terminated_count = truncated_count = 0
    step_seconds = reset_seconds = action_seconds = 0.
    candidate_max = 0
    started = time.perf_counter()
    for _ in range(steps):
        then = time.perf_counter()
        counts = np.count_nonzero(observation['action_mask'], axis=1)
        candidate_max = max(candidate_max, int(counts.max()))
        actions = np.asarray([rng.integers(count) for rng, count in zip(rngs, counts)], dtype=np.int64)
        action_seconds += time.perf_counter()-then
        then = time.perf_counter()
        observation, rewards, terminated, truncated, infos = env.step(actions)
        step_seconds += time.perf_counter()-then
        digest.update(actions.tobytes()); digest.update(rewards.tobytes())
        digest.update(terminated.tobytes()); digest.update(truncated.tobytes())
        for key in phases:
            phases[key] += float(np.sum(infos['timing'][key]))
        terminated_count += int(terminated.sum())
        truncated_count += int(truncated.sum())
        done = terminated | truncated
        if done.any():
            # Disabled autoreset keeps the actual final observation above. A
            # masked reset is separate work, never counted as an environment step.
            then = time.perf_counter()
            observation, _ = env.reset(options={'reset_mask': done})
            reset_seconds += time.perf_counter()-then
    elapsed = time.perf_counter()-started
    return dict(decisions=steps*env.num_envs, seconds=elapsed,
                decisions_per_second=steps*env.num_envs/elapsed,
                initial_reset_seconds=initial_reset_seconds, step_seconds=step_seconds,
                reset_seconds=reset_seconds, action_sampling_seconds=action_seconds,
                terminated=terminated_count, truncated=truncated_count,
                candidate_max=candidate_max, observation_bytes_per_env=sum(v[0].nbytes for v in observation.values()),
                summed_env_phase_seconds=phases, behavior_digest=digest.hexdigest())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine-root', required=True)
    parser.add_argument('--data-sha256')
    parser.add_argument('--vector', choices=('sync', 'async'), default='sync')
    parser.add_argument('--envs', type=int, default=1)
    parser.add_argument('--steps', type=int, default=256, help='Actual decisions per env per repeat')
    parser.add_argument('--warmup', type=int, default=16)
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--max-candidates', type=int, default=16384)
    parser.add_argument('--max-decisions', type=int, default=1024)
    parser.add_argument('--seed', type=int, default=42)
    parser.add_argument('--rng-seed', type=int, default=123)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    if min(args.envs, args.steps, args.repeats) < 1 or args.warmup < 0:
        parser.error('Positive env/step/repeat counts and nonnegative warmup required')
    factory = partial(ArkGuardEnv, args.engine_root, expected_data_sha256=args.data_sha256,
                      max_candidates=args.max_candidates, max_decisions=args.max_decisions, profile=True)
    start = time.perf_counter()
    if args.vector == 'async':
        env = gym.vector.AsyncVectorEnv([factory]*args.envs, context='spawn', daemon=False,
            shared_memory=True, autoreset_mode=gym.vector.AutoresetMode.DISABLED)
    else:
        env = gym.vector.SyncVectorEnv([factory]*args.envs,
            autoreset_mode=gym.vector.AutoresetMode.DISABLED)
    try:
        rollout(env, args.warmup, args.seed, args.rng_seed)
        setup_seconds = time.perf_counter()-start
        runs = [rollout(env, args.steps, args.seed, args.rng_seed) for _ in range(args.repeats)]
    finally:
        env.close()
    result = dict(vector=args.vector, envs=args.envs, gymnasium=gym.__version__,
        seed=args.seed, rng_seed=args.rng_seed, max_candidates=args.max_candidates,
        max_decisions=args.max_decisions, warmup_steps=args.warmup, setup_seconds=setup_seconds,
        policy='uniform over current candidate indices; no model, training or trajectory capture',
        metric='decision steps including in-loop resets; initial reset/setup excluded; not pure engine time',
        torch_imported='torch' in sys.modules, runs=runs,
        median_decisions_per_second=statistics.median(r['decisions_per_second'] for r in runs))
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
