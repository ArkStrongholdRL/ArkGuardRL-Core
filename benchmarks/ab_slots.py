"""Interleaved A/B: Gym Async processes vs Rust-owned slots.

Alternates order each repeat and records the load average around each block,
so a machine-load spike is visible instead of being read as a backend result.
"""
import argparse
import json
import os
import statistics
import time
from functools import partial

import gymnasium as gym
import numpy as np
from arkguardrl_core.native_gym import ArkGuardNativeEnv
from bench_slots import Policy
from native_batch_slots import NativeSlotBatch

REFERENCE_LIBRARY = None


def load():
    """Load average where the platform reports one; Windows reports CPU instead."""
    try:
        return {'load': round(os.getloadavg()[0], 2)}
    except (AttributeError, OSError):
        pass
    try:
        import psutil
        return {'cpu_percent': psutil.cpu_percent(interval=0.2)}
    except Exception:
        return {}


def scheduling(cpus=None, priority=None):
    import psutil
    process = psutil.Process()
    if cpus is not None:
        process.cpu_affinity(cpus)
    if priority is not None:
        if os.name != 'nt':
            raise ValueError('Explicit priority is supported only on Windows')
        process.nice({'normal': psutil.NORMAL_PRIORITY_CLASS,
                      'high': psutil.HIGH_PRIORITY_CLASS}[priority])
    return dict(pid=process.pid, affinity=process.cpu_affinity() if hasattr(process, 'cpu_affinity') else None,
                priority=int(process.nice()))


def make_worker(data_root, core, library, cpus, priority):
    actual = scheduling(cpus, priority)
    env = ArkGuardNativeEnv(data_root, core=core, native_library=library)
    env._benchmark_scheduling = actual
    return env


def gym_block(n, steps):
    envs = gym.vector.AsyncVectorEnv(
        [partial(make_worker, ARGS.data_root, ARGS.core, REFERENCE_LIBRARY,
                 ARGS.cpus, ARGS.priority) for _ in range(n)],
        context='spawn', shared_memory=True,
        autoreset_mode=gym.vector.AutoresetMode.DISABLED)
    try:
        obs, _ = envs.reset(seed=42)
        policy = Policy(n)
        policy.reset()
        started = time.perf_counter()
        for _ in range(steps):
            obs, _, terminated, truncated, _ = envs.step(policy.choose(obs, n))
            done = terminated | truncated
            if done.any():
                obs, _ = envs.reset(options={'reset_mask': done})
                policy.reset(done)
        return n * steps / (time.perf_counter() - started)
    finally:
        envs.close()


def slot_block(n, steps, threads):
    with NativeSlotBatch(ARGS.data_root, core=ARGS.core, num_envs=n, threads=threads,
                         native_library=ARGS.slots_library) as batch:
        obs, _ = batch.reset(seed=42)
        policy = Policy(n)
        policy.reset()
        started = time.perf_counter()
        for _ in range(steps):
            obs, _, terminated, truncated, _ = batch.step(policy.choose(obs, n))
            done = terminated | truncated
            if done.any():
                obs, _ = batch.reset(reset_mask=done)
                policy.reset(done)
        return n * steps / (time.perf_counter() - started)


def main():
    global ARGS, REFERENCE_LIBRARY
    parser = argparse.ArgumentParser()
    parser.add_argument('--data-root', required=True)
    parser.add_argument('--core', default='band_chen')
    parser.add_argument('--reference-library', required=True)
    parser.add_argument('--slots-library', required=True)
    parser.add_argument('--envs', type=int, default=4)
    parser.add_argument('--threads', type=int, default=4)
    parser.add_argument('--steps', type=int, default=192)
    parser.add_argument('--repeats', type=int, default=4)
    parser.add_argument('--cpus', help="Comma separated CPU indexes applied to every measured process")
    parser.add_argument('--priority', choices=('normal', 'high'),
                        help='Process priority applied to every measured process (Windows)')
    parser.add_argument('--out')
    ARGS = parser.parse_args()
    ARGS.cpus = [int(c) for c in ARGS.cpus.split(',')] if ARGS.cpus else None
    REFERENCE_LIBRARY = ARGS.reference_library
    n = ARGS.envs
    records = {'gym': [], 'slots': []}
    for repeat in range(ARGS.repeats):
        order = ('gym', 'slots') if repeat % 2 == 0 else ('slots', 'gym')
        for name in order:
            before = load()
            if name == 'gym':
                value = gym_block(n, ARGS.steps)
            else:
                scheduling(ARGS.cpus, ARGS.priority)
                value = slot_block(n, ARGS.steps, ARGS.threads)
            after = load()
            records[name].append({'decisions_per_second': value, 'before': before, 'after': after})
            print(json.dumps({name: round(value, 1), 'repeat': repeat,
                              'before': before, 'after': after}), flush=True)
    summary = {name: statistics.median(row['decisions_per_second'] for row in rows)
               for name, rows in records.items()}
    summary['slots_over_gym_pct'] = 100 * (summary['slots'] / summary['gym'] - 1)
    summary = {k: round(v, 1) for k, v in summary.items()}
    print(json.dumps(summary), flush=True)
    if ARGS.out:
        with open(ARGS.out, 'w') as handle:
            handle.write(json.dumps(dict(
                scope='Interleaved A/B, scripted numeric policy, masked reset, resets in loop, '
                      'no model/training; load average recorded per block',
                envs=n, threads=ARGS.threads, steps=ARGS.steps, repeats=ARGS.repeats,
                summary=summary, records=records), indent=2) + '\n')


if __name__ == '__main__':
    main()