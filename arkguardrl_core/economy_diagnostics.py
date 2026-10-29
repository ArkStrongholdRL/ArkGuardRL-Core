"""Read-only loss-gradient and critic diagnostics on fresh natural episodes."""
from collections import defaultdict
import torch


def inspect(model, episodes):
    if not episodes or any(not ep or not ep[-1]['terminated'] or any(r['truncated'] for r in ep) for ep in episodes):
        raise ValueError('Complete fresh natural episodes required for gradient inspection')
    rows = []
    for episode in episodes:
        mc, gae = 0., 0.
        episode_rows = []
        for i in reversed(range(len(episode))):
            r = episode[i]
            mc = r['reward'] + mc
            next_value = 0. if r['terminated'] else episode[i+1]['value']
            delta = r['reward'] + next_value - r['value']
            gae = delta + (0. if r['terminated'] else .95 ** r['settlements'] * gae)
            episode_rows.append(dict(**r, mc_target=mc, gae=gae, mc_advantage=mc-r['value']))
        rows.extend(reversed(episode_rows))
    gae = torch.tensor([r['gae'] for r in rows])
    mc_adv = torch.tensor([r['mc_advantage'] for r in rows])
    for name, values in (('normalized_gae', gae), ('normalized_mc', mc_adv)):
        normalized = (values-values.mean()) / values.std(unbiased=False).clamp_min(1e-8)
        for r, value in zip(rows, normalized):
            r[name] = value
    parameters = list(model.actor.parameters())
    previous = [p.requires_grad for p in parameters]
    gradients = {name:torch.zeros(sum(p.numel() for p in parameters))
                 for name in ('policy_gae', 'policy_mc', 'entropy', 'prior_kl')}
    try:
        for p in parameters:
            p.requires_grad_(True)
        for start in range(0, len(rows), 64):
            batch = rows[start:start+64]
            losses = defaultdict(list)
            for r in batch:
                logp, _ = model(r['inputs'])
                ratio = (logp[r['index']]-r['logp']).exp()
                for name, advantage in (('policy_gae', r['normalized_gae']), ('policy_mc', r['normalized_mc'])):
                    losses[name].append(-torch.minimum(ratio*advantage, ratio.clamp(.9,1.1)*advantage))
                losses['entropy'].append(.001*(logp.exp()*logp).sum())
                losses['prior_kl'].append(.05*(r['inputs']['prior'].exp()*(r['inputs']['prior']-logp)).sum())
            for i, (name, terms) in enumerate(losses.items()):
                gradient = torch.autograd.grad(torch.stack(terms).sum()/len(rows), parameters,
                    retain_graph=i<len(losses)-1, allow_unused=True)
                vector = torch.cat([(torch.zeros_like(p) if g is None else g).detach().flatten()
                                    for p,g in zip(parameters, gradient)])
                gradients[name] += vector
    finally:
        for p, flag in zip(parameters, previous):
            p.requires_grad_(flag)
    norms = {name:float(v.norm()) for name,v in gradients.items()}
    def cosine(a, b):
        denominator = float(a.norm()*b.norm())
        return float(a.dot(b))/denominator if denominator>1e-15 else None
    targets = torch.tensor([r['mc_target'] for r in rows])
    values = torch.tensor([r['value'] for r in rows])
    variance = float(targets.var(unbiased=False))
    by_kind = defaultdict(list)
    for r in rows:
        by_kind[r.get('kind', 'unknown')].append(r)
    details = [dict(round=r.get('round'), kind=r.get('kind'), value=r['value'],
        mc_target=r['mc_target'], gae=r['gae'], mc_advantage=r['mc_advantage'],
        normalized_gae=float(r['normalized_gae']), normalized_mc=float(r['normalized_mc'])) for r in rows]
    return dict(episodes=len(episodes), rows=len(rows), gradient_norms=norms,
        gae_mc_gradient_cosine=cosine(gradients['policy_gae'],gradients['policy_mc']),
        gae_entropy_cosine=cosine(gradients['policy_gae'],gradients['entropy']),
        gae_kl_cosine=cosine(gradients['policy_gae'],gradients['prior_kl']),
        critic_mc_mse=float(((values-targets)**2).mean()), critic_bias=float((values-targets).mean()),
        mc_target_std=float(targets.std(unbiased=False)),
        critic_explained_variance=1-float((targets-values).var(unbiased=False))/variance if variance>1e-15 else None,
        normalized_advantage_sign_disagreement=float(((gae-gae.mean())*(mc_adv-mc_adv.mean())<0).float().mean()),
        by_kind={k:dict(rows=len(rs),mean_gae=sum(r['gae'] for r in rs)/len(rs),
                       mean_mc_advantage=sum(r['mc_advantage'] for r in rs)/len(rs)) for k,rs in by_kind.items()},
        optimizer_steps=0, note='read-only fresh data; not reconstruction of historical training gradients'), details
