"""Model-free Gymnasium step benchmark. Run from an installed Core checkout."""
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


def run(*, engine_root, data_sha256=None, vector="sync", envs=1, steps=256,
        warmup=16, repeats=3, max_candidates=16384, max_decisions=1024,
        seed=42, rng_seed=123, output=None):
    output = Path(output) if output is not None else None
    if vector not in ("sync", "async"):
        raise ValueError("Invalid vector backend")
    if min(envs, steps, repeats) < 1 or warmup < 0:
        raise ValueError('Positive env/step/repeat counts and nonnegative warmup required')
    factory = partial(ArkGuardEnv, engine_root, expected_data_sha256=data_sha256,
                      max_candidates=max_candidates, max_decisions=max_decisions, profile=True)
    start = time.perf_counter()
    if vector == 'async':
        env = gym.vector.AsyncVectorEnv([factory]*envs, context='spawn', daemon=False,
            shared_memory=True, autoreset_mode=gym.vector.AutoresetMode.DISABLED)
    else:
        env = gym.vector.SyncVectorEnv([factory]*envs,
            autoreset_mode=gym.vector.AutoresetMode.DISABLED)
    try:
        rollout(env, warmup, seed, rng_seed)
        setup_seconds = time.perf_counter()-start
        runs = [rollout(env, steps, seed, rng_seed) for _ in range(repeats)]
    finally:
        env.close()
    result = dict(vector=vector, envs=envs, gymnasium=gym.__version__,
        seed=seed, rng_seed=rng_seed, max_candidates=max_candidates,
        max_decisions=max_decisions, warmup_steps=warmup, setup_seconds=setup_seconds,
        policy='uniform over current candidate indices; no model, training or trajectory capture',
        metric='decision steps including in-loop resets; initial reset/setup excluded; not pure engine time',
        torch_imported='torch' in sys.modules, runs=runs,
        median_decisions_per_second=statistics.median(r['decisions_per_second'] for r in runs))
    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))

    return result
