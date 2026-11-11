"""Compare Rust-owned slots with Gym vectors on the same scripted policy.

Run with the current Core on PYTHONPATH plus this directory. No model/training.
"""
from contextlib import closing
from functools import partial
import json
import statistics
import time

import gymnasium as gym
import numpy as np
from arkguardrl_core.native_gym import ArkGuardNativeEnv
from arkguardrl_core.native_batch import NativeSlotBatch


class Policy:
    def __init__(self, n):
        self.bought = [set() for _ in range(n)]

    def reset(self, mask=np.zeros(0, bool)):
        for i in np.flatnonzero(mask) if len(mask) else range(len(self.bought)):
            self.bought[i].clear()

    def choose(self, obs, n):
        out = []
        for i in range(n):
            mask = obs['action_mask'][i]
            count = int(mask.sum())
            if not count:
                out.append(0)
                continue
            cand = obs['candidates'][i, :count]
            types = 5 + 4
            rows = lambda kind: np.flatnonzero(cand[:, types + kind])  # noqa: E731
            choice = rows(2)
            if len(choice):
                out.append(int(choice[0]))
                continue
            moves = np.flatnonzero((cand[:, types + 14] != 0) & (cand[:, 5] != 0))
            if len(moves):
                out.append(int(moves[-1]))
                continue
            buys = rows(3)
            round_key = float(obs['observation'][i, 0])
            if round_key not in self.bought[i] and len(buys):
                self.bought[i].add(round_key)
                out.append(int(buys[0]))
                continue
            ready = rows(13)
            out.append(int(ready[0]) if len(ready) else 0)
        return np.asarray(out, dtype=np.int64)


def measure(batch_step, batch_reset, n, steps, reps):
    rates = []
    for rep in range(reps + 1):
        obs, _ = batch_reset()
        policy = Policy(n)
        policy.reset()
        started = time.perf_counter()
        for _ in range(steps):
            obs, _, terminated, truncated, _ = batch_step(policy.choose(obs, n))
            done = terminated | truncated
            if done.any():
                obs, _ = batch_reset(done)
                policy.reset(done)
        if rep:
            rates.append(n * steps / (time.perf_counter() - started))
    return statistics.median(rates)


def gym_async(data_root, core, n, steps, reps, native_library):
    factories = [partial(ArkGuardNativeEnv, data_root, core=core,
                         native_library=native_library) for _ in range(n)]

    with closing(gym.vector.AsyncVectorEnv(
            factories, context='spawn', shared_memory=True,
            autoreset_mode=gym.vector.AutoresetMode.DISABLED)) as env:
        def reset(mask=None):
            return env.reset(options={'reset_mask': mask} if mask is not None else None)
        return measure(env.step, reset, n, steps, reps)


def run(*, data_root, reference_library, slots_library, steps=192, repeats=3,
        envs=(4,), output=None):
    out = {}
    for n in envs:
        out[f'gym-async-{n}'] = gym_async(
            data_root, 'band_chen', n, steps, repeats, reference_library)
        for threads in sorted({min(n, threads) for threads in (1, 2, 4)}):
            with NativeSlotBatch(data_root, core='band_chen', num_envs=n, threads=threads,
                                native_library=slots_library) as batch:
                reset = lambda mask=None: batch.reset(reset_mask=mask)  # noqa: E731
                rate = measure(batch.step, reset, n, steps, repeats)
            out[f'slots-{n}-t{threads}'] = rate
            print(json.dumps({f'slots-{n}-t{threads}': round(rate, 1),
                              f'gym-async-{n}': round(out[f"gym-async-{n}"], 1)}), flush=True)
    if output:
        with open(output, 'w') as handle:
            handle.write(json.dumps(dict(scope='Scripted numeric policy, masked reset, '
                                         'resets in loop, no model/training', steps=steps,
                                         repeats=repeats, results=out), indent=2) + '\n')
    print(json.dumps({k: round(v, 1) for k, v in out.items()}, indent=1))

    return out
