"""Interleaved A/B: Gym Async processes vs Rust-owned slots.

Alternates order each repeat and records the load average around each block,
so a machine-load spike is visible instead of being read as a backend result.
"""
import json
import os
import statistics
import time
from functools import partial

import gymnasium as gym
import numpy as np
from arkguardrl_core.native_gym import ArkGuardNativeEnv
from benchmarks.bench_slots import Policy
from benchmarks.native_batch_slots import NativeSlotBatch

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


def gym_block(n, steps, data_root, core, reference_library, cpus, priority):
    envs = gym.vector.AsyncVectorEnv(
        [partial(make_worker, data_root, core, reference_library, cpus, priority) for _ in range(n)],
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


def slot_block(n, steps, threads, data_root, core, slots_library):
    with NativeSlotBatch(data_root, core=core, num_envs=n, threads=threads,
                         native_library=slots_library) as batch:
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


def run(*, data_root, reference_library, slots_library, core="band_chen",
        envs=4, threads=4, steps=192, repeats=4, cpus=None, priority=None,
        output=None):
    if priority not in (None, "normal", "high"):
        raise ValueError("Invalid process priority")
    n = envs
    records = {'gym': [], 'slots': []}
    for repeat in range(repeats):
        order = ('gym', 'slots') if repeat % 2 == 0 else ('slots', 'gym')
        for name in order:
            before = load()
            if name == 'gym':
                value = gym_block(n, steps, data_root, core, reference_library, cpus, priority)
            else:
                scheduling(cpus, priority)
                value = slot_block(n, steps, threads, data_root, core, slots_library)
            after = load()
            records[name].append({'decisions_per_second': value, 'before': before, 'after': after})
            print(json.dumps({name: round(value, 1), 'repeat': repeat,
                              'before': before, 'after': after}), flush=True)
    summary = {name: statistics.median(row['decisions_per_second'] for row in rows)
               for name, rows in records.items()}
    summary['slots_over_gym_pct'] = 100 * (summary['slots'] / summary['gym'] - 1)
    summary = {k: round(v, 1) for k, v in summary.items()}
    print(json.dumps(summary), flush=True)
    if output:
        with open(output, 'w') as handle:
            handle.write(json.dumps(dict(
                scope='Interleaved A/B, scripted numeric policy, masked reset, resets in loop, '
                      'no model/training; load average recorded per block',
                envs=n, threads=threads, steps=steps, repeats=repeats,
                summary=summary, records=records), indent=2) + '\n')

    return dict(summary=summary, records=records)
