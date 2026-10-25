"""Portable bounded one-iteration entrypoint; all data and seeds are explicit inputs."""
import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    sub.add_parser('identity', help='Print framework source identity; no engine or Torch required')
    run = sub.add_parser('iterate', help='Collect explicit complete games, then one PPO iteration')
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
    run.add_argument('--collect-only', action='store_true')
    args = parser.parse_args()
    if args.command == 'identity':
        import json
        from .identity import source_hashes
        from . import __version__
        print(json.dumps({'version': __version__, 'sources': source_hashes()}, indent=2))
        return
    import random
    import torch
    from arkguardrl import learn
    from . import checkpoint, sampling
    from .io import sha, write
    from .run import Run, SamplerConfig, PPOConfig
    from .learning.ppo import ppo_update, tensors_to
    if len(set(args.seeds)) != len(args.seeds) or not all(0 < s <= 0xffffffff for s in args.seeds):
        parser.error('Explicit unique uint32 game seeds required')
    if not 0 <= args.rng_seed <= 0xffffffff-len(args.seeds):
        parser.error('Sampling RNG range outside uint32')
    learn._setup_torch(seed=args.rng_seed)
    ppo = PPOConfig(batch_size=args.batch_size, epochs=args.epochs,
                    learning_rate=args.learning_rate, max_buffer_bytes=args.max_buffer_bytes)
    sampler = SamplerConfig(prep_action_cost=args.prep_action_cost,
                            max_episode_bytes=args.max_buffer_bytes//len(args.seeds))
    ctx = Run(args.out, engine_root=args.engine_root, expected_data_sha256=args.expected_data_sha256,
              difficulty=args.difficulty, sampler=sampler, ppo=ppo, device=args.device, max_wall=args.max_wall)
    try:
        model, _ = checkpoint.load(args.checkpoint, ctx.contract)
        anchor, _ = checkpoint.load(args.anchor, ctx.contract)
        anchor.requires_grad_(False)
        rows, reports = [], []
        write(ctx.root/'inputs.json', {'checkpoint_sha256': sha(args.checkpoint), 'anchor_sha256': sha(args.anchor),
            'seeds': args.seeds, 'rollout_rng_start': args.rng_seed, 'optimizer_rng': args.rng_seed,
            'collection': 'sequential complete episodes; parallel collection not implemented'})
        for index, seed in enumerate(args.seeds):
            new, report = sampling.collect(ctx, model, anchor, seed=seed, rng_seed=args.rng_seed+index,
                revision=0, checkpoint=args.checkpoint, directory=ctx.root/'games'/f'{index:03d}-{seed}')
            rows.extend(new)
            reports.append(report)
            write(ctx.root/'episodes.json', reports)
            del new
        if args.collect_only:
            write(ctx.root/'summary.json', {'state': 'complete', 'phase': 'collect-only', 'decisions': len(rows)})
            return
        optimizer = torch.optim.Adam(model.parameters(), lr=ppo.learning_rate)
        rng = random.Random(args.rng_seed)
        metrics = ppo_update(ctx, model, optimizer, rows, 0, args.checkpoint, ppo.kl_coef, rng, device=args.device)
        ctx.save(ctx.root/'checkpoint.pt', model, phase='one-live-ppo-iteration', metrics=metrics)
        neural = sampling.neural
        neural.save_payload(ctx.root/'training-state.pt', {
            'schema': 'arkguardrl-core/training-state/v1', 'checkpoint_sha256': sha(ctx.root/'checkpoint.pt'),
            'optimizer': tensors_to(optimizer.state_dict(), 'cpu'), 'sampler_state': rng.getstate(),
            'torch_rng': torch.get_rng_state(), 'run_sha256': sha(ctx.root/'run.json')})
        write(ctx.root/'summary.json', {'state': 'complete', 'phase': 'one-live-ppo-iteration', 'metrics': metrics})
        print(ctx.root/'summary.json')
    except BaseException as exc:
        write(ctx.root/'failure.json', {'state': 'failed', 'error': repr(exc)})
        raise
