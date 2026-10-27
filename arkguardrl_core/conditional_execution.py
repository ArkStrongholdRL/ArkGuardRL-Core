"""GPL-3.0 independent supervised execution diagnostic on real public states.

This module never constructs native PPO rows, rewards, GAE or an alternate
classifier. Conditions use the existing production actor and full support.
"""
import random
import time
import torch
from .learning import data, network

SCHEMA = 'arkguardrl-core/conditional-execution-cross-entropy/v1'
PREFIXES = ('option_projection.', 'search_economy.', 'search_schedule.')


def configure(model):
    from .strategic_options import is_search
    if not is_search(model.planner_config.get('options')):
        raise ValueError('Existing precise search pick interface required')
    model.requires_grad_(False)
    selected = []
    for name, parameter in model.named_parameters():
        if name.startswith(PREFIXES):
            parameter.requires_grad_(True)
            selected.append((name, parameter))
    if not selected or next(model.parameters()).device.type != 'cpu':
        raise ValueError('CPU existing scoring parameters required')
    return selected


def responses(model, cases):
    result = []
    with torch.no_grad():
        for case in cases:
            matrix, conditional, targets, choices, full = [], [], [], [], []
            for item in case['conditions']:
                logp, _ = model(item['row'])
                if abs(float(logp.exp().sum()) - 1) > 1e-5:
                    raise ValueError('Production distribution not normalized')
                lp, _ = network.forward(model, data.collate([data.prepare(model, item['row'])]))
                if not torch.allclose(logp, lp[0], atol=1e-5, rtol=1e-5):
                    raise ValueError('Production scalar/batch replay mismatch')
                idx = case['reward_indices']
                matrix.append(logp[idx].exp().tolist())
                conditional.append(logp[idx].softmax(0).tolist())
                targets.append(item['target'])
                choices.append(int(logp.argmax()))
                full.append(float(logp[item['target']].exp()))
            result.append(dict(id=case['id'], seed=case['seed'], matrix=matrix,
                conditional_matrix=conditional, target_indices=targets,
                greedy_indices=choices, target_probability=full,
                correct=[a == b for a, b in zip(targets, choices)],
                original_preference_ranks=[case['original_preference'].index(t)+1 for t in targets]))
    return result


def fit(model, train_cases, *, steps, offers_per_step, learning_rate, seed,
        max_wall, on_progress=None):
    if not train_cases or any(c['split'] != 'train' for c in train_cases):
        raise ValueError('Only independently labelled training-source conditions')
    if min(steps, offers_per_step, learning_rate, max_wall) <= 0:
        raise ValueError('Positive finite bounded auxiliary budget required')
    selected = configure(model)
    frozen = {n: p.detach().clone() for n, p in model.named_parameters() if not p.requires_grad}
    optimizer = torch.optim.Adam([p for _, p in selected], lr=learning_rate)
    rng = random.Random(seed)
    prepared = {c['id']: [data.prepare(model, item['row']) for item in c['conditions']] for c in train_cases}
    started = time.monotonic()
    history = []
    for step in range(steps):
        if time.monotonic()-started > max_wall:
            raise TimeoutError('Auxiliary task wall budget, not a game termination')
        chosen = [rng.choice(train_cases) for _ in range(offers_per_step)]
        rows = [row for c in chosen for row in prepared[c['id']]]
        targets = torch.tensor([i['target'] for c in chosen for i in c['conditions']], dtype=torch.long)
        packed = data.collate(rows, max_bytes=512*1024**2)
        optimizer.zero_grad(set_to_none=True)
        logp, _ = network.forward(model, packed)
        loss = -logp[torch.arange(len(targets)), targets].mean()
        if not torch.isfinite(loss): raise ValueError('Nonfinite auxiliary loss')
        loss.backward()
        norm = torch.nn.utils.clip_grad_norm_([p for _, p in selected], 1., error_if_nonfinite=True)
        optimizer.step()
        if step == 0 or (step+1) % 50 == 0 or step+1 == steps:
            record = dict(step=step+1, minibatch_loss=float(loss.detach()),
                grad_norm=float(norm), elapsed=time.monotonic()-started)
            history.append(record)
            if on_progress is not None: on_progress(record)
    if any(not torch.equal(value, dict(model.named_parameters())[name]) for name, value in frozen.items()):
        raise ValueError('Auxiliary optimizer changed a frozen parameter')
    return dict(schema=SCHEMA, steps=steps, on_policy=False, native_reward_changed=False,
        trained_parameters=[n for n, _ in selected], frozen_parameters_exact=True,
        optimizer=optimizer.state_dict(), python_rng=rng.getstate(), torch_rng=torch.get_rng_state(),
        history=history, elapsed=time.monotonic()-started)
