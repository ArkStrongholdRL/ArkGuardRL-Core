"""Portable bounded iteration entrypoint; all data and seeds are explicit inputs."""
import argparse
from pathlib import Path
import time


def main():
    entry_started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('identity', help='Print runtime version and Rust ABI')
    dashboard = sub.add_parser('dashboard', help='Browse local trajectories and training reports')
    dashboard.add_argument('--results-root', type=Path, required=True)
    dashboard.add_argument('--engine-root', type=Path)
    dashboard.add_argument('--port', type=int, default=8765)
    run = sub.add_parser('iterate', help='Collect explicit complete games, then update PPO with persistent workers')
    run.add_argument('--engine-root', type=Path, required=True)
    run.add_argument('--expected-data-sha256', required=True)
    run.add_argument('--checkpoint', type=Path, required=True)
    run.add_argument('--anchor', type=Path, required=True)
    run.add_argument('--out', type=Path, required=True)
    run.add_argument('--seeds', type=int, nargs='+', required=True)
    run.add_argument('--rng-seed', type=int, required=True)
    run.add_argument('--difficulty', default='ABYSS')
    run.add_argument('--device', choices=('cpu', 'mps', 'cuda'), default='cpu')
    run.add_argument('--batch-size', type=int, default=256)
    run.add_argument('--epochs', type=int, default=2)
    run.add_argument('--learning-rate', type=float, default=3e-5)
    run.add_argument('--prep-action-cost', type=float, default=0)
    run.add_argument('--max-wall', type=float, default=600)
    run.add_argument('--max-buffer-bytes', type=int, default=1024**3)
    run.add_argument('--execution-memory-bytes', type=int, default=1024**3)
    run.add_argument('--workers', type=int, default=4,
                     help='Persistent CPU collectors, each with a reusable Node process (1..16)')
    run.add_argument('--iterations', type=int, default=1,
                     help='Equal consecutive groups of explicit seeds; keep workers and optimizer alive')
    run.add_argument('--transfer-mode', choices=('stream', 'episode'), default='stream')
    run.add_argument('--transfer-window', type=int, default=4, help='Outstanding 1 MiB frames per worker (1..16)')
    run.add_argument('--queue-rows', type=int, default=2, help='Bounded sender queue per worker (1..8)')
    run.add_argument('--transport-rows', type=int, default=8, help='Rows per columnar block (1 uses legacy row codec; max 32)')
    run.add_argument('--no-model-cache', action='store_true', help='Reload both policies per game for comparison')
    run.add_argument('--inference-mode', choices=('local', 'batch-cpu', 'batch-mps', 'batch-cuda'), default='local',
                     help='Sampling inference device; independent of --device for PPO')
    run.add_argument('--inference-wait-ms', type=float, default=2.,
                     help='Idle-device batching window (0..50 ms); excludes waiting for an active batch')
    run.add_argument('--collect-only', action='store_true')
    run.add_argument('--trace', action='store_true', help='Record full trajectories for debugging (off by default)')
    args = parser.parse_args()
    if args.command == 'dashboard':
        from .dashboard import serve
        serve(args.results_root, args.engine_root, port=args.port)
        return
    if args.command == 'identity':
        import json
        from .native_runtime import kernel
        from . import __version__
        print(json.dumps({'version': __version__, 'rust_abi': kernel().ABI}, indent=2))
        return
    import random
    import torch
    from arkguardrl import learn
    from . import checkpoint, sampling
    from .io import sha, write
    from .run import Run, SamplerConfig, PPOConfig
    from .learning.ppo import ppo_update, tensors_to
    from .parallel import Collector
    if len(set(args.seeds)) != len(args.seeds) or not all(0 < s <= 0xffffffff for s in args.seeds):
        parser.error('Explicit unique uint32 game seeds required')
    if not 0 <= args.rng_seed <= 0xffffffff-len(args.seeds):
        parser.error('Sampling RNG range outside uint32')
    if not 1 <= args.workers <= 16:
        parser.error('Workers must be 1..16')
    if args.iterations < 1 or len(args.seeds) % args.iterations:
        parser.error('Seeds must split into nonempty equal groups for iterations')
    if (not 1 <= args.transfer_window <= 16 or not 1 <= args.queue_rows <= 8
            or not 1 <= args.transport_rows <= 32 or not 0 <= args.inference_wait_ms <= 50):
        parser.error('Invalid bounded transfer/inference settings')
    games_per_iteration = len(args.seeds)//args.iterations
    learn._setup_torch(seed=args.rng_seed)
    ppo = PPOConfig(batch_size=args.batch_size, epochs=args.epochs,
                    learning_rate=args.learning_rate, max_buffer_bytes=args.max_buffer_bytes,
                    execution_memory_bytes=args.execution_memory_bytes)
    sampler = SamplerConfig(prep_action_cost=args.prep_action_cost,
                            log_mode='full' if args.trace else 'none',
                            max_episode_bytes=args.max_buffer_bytes//games_per_iteration)
    ctx = Run(args.out, engine_root=args.engine_root, expected_data_sha256=args.expected_data_sha256,
              difficulty=args.difficulty, sampler=sampler, ppo=ppo, device=args.device, max_wall=args.max_wall)
    try:
        model, _ = checkpoint.load(args.checkpoint, ctx.contract)
        # Validate the anchor before dispatch; Collector keeps its own reusable copy.
        checked_anchor, _ = checkpoint.load(args.anchor, ctx.contract)
        del checked_anchor
        options = dict(transfer_mode=args.transfer_mode, transfer_window=args.transfer_window,
            queue_rows=args.queue_rows, block_rows=args.transport_rows, model_cache=not args.no_model_cache,
            inference_mode=args.inference_mode, inference_wait_ms=args.inference_wait_ms)
        write(ctx.root/'inputs.json', {'checkpoint_sha256': sha(args.checkpoint), 'anchor_sha256': sha(args.anchor),
            'seeds': args.seeds, 'rollout_rng_start': args.rng_seed, 'optimizer_rng': args.rng_seed,
            'sampling_rng': 'per-episode-cpu-generator/v1', 'iterations': args.iterations,
            'workers': min(args.workers, games_per_iteration), 'collector_options': options,
            'collection': 'persistent spawn workers; complete live episodes; reusable Node per worker'})
        optimizer = None if args.collect_only else torch.optim.Adam(model.parameters(), lr=ppo.learning_rate)
        rng = random.Random(args.rng_seed)
        policy_path = args.checkpoint
        rounds = []
        all_started = time.monotonic()
        with Collector(ctx, workers=min(args.workers, games_per_iteration), **options) as collector:
            for revision in range(args.iterations):
                iteration_started = time.monotonic()
                offset = revision*games_per_iteration
                rows, reports, timing = collector.collect(model, checkpoint=policy_path,
                    anchor=args.anchor, seeds=args.seeds[offset:offset+games_per_iteration],
                    rng_seed=args.rng_seed+offset, revision=revision)
                result = {'state': 'complete', 'phase': 'collect-only' if args.collect_only else 'live-ppo-iteration',
                          'revision': revision, 'games': len(reports), 'decisions': len(rows), 'timing': timing}
                if not args.collect_only:
                    update_started = time.monotonic()
                    metrics = ppo_update(ctx, model, optimizer, rows, revision, policy_path,
                                         ppo.kl_coef, rng, device=args.device)
                    timing['update_seconds'] = time.monotonic()-update_started
                    save_started = time.monotonic()
                    policy_path = ctx.root/('checkpoint.pt' if revision == args.iterations-1
                                            else f'checkpoint-r{revision+1:04d}.pt')
                    ctx.save(policy_path, model, phase='live-ppo-iteration', revision=revision+1, metrics=metrics)
                    sampling.neural.save_payload(ctx.root/('training-state.pt' if revision == args.iterations-1
                                                          else f'training-state-r{revision+1:04d}.pt'), {
                        'schema': 'arkguardrl-core/training-state/v1', 'checkpoint_sha256': sha(policy_path),
                        'revision': revision+1, 'optimizer': tensors_to(optimizer.state_dict(), 'cpu'),
                        'sampler_state': rng.getstate(), 'torch_rng': torch.get_rng_state(),
                        'run_sha256': sha(ctx.root/'run.json')})
                    timing['save_seconds'] = time.monotonic()-save_started
                    result['metrics'] = metrics
                del rows
                timing['iteration_seconds'] = time.monotonic()-iteration_started
                rounds.append(result)
                write(ctx.root/f'iteration-{revision:04d}.json', result)
        elapsed = time.monotonic()-all_started
        if args.iterations == 1:
            summary = rounds[0]
            summary['timing']['collection_with_worker_lifecycle_seconds'] = (
                elapsed-summary['timing'].get('update_seconds', 0)-summary['timing'].get('save_seconds', 0))
            summary['timing']['iteration_seconds'] = elapsed
            summary['timing']['entrypoint_seconds'] = time.monotonic()-entry_started
        else:
            summary = {'state': 'complete', 'phase': 'persistent-iterations', 'iterations': rounds,
                'timing': {'total_seconds': elapsed, 'entrypoint_seconds': time.monotonic()-entry_started,
                           'worker_startup_seconds': collector.startup_seconds}}
        write(ctx.root/'summary.json', summary)
        print(ctx.root/'summary.json')
    except BaseException as exc:
        write(ctx.root/'failure.json', {'state': 'failed', 'error': repr(exc)})
        raise
