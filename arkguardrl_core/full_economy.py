"""Economic PPO on frozen full-state/entity representations and a tempered prior."""
import random
import torch
from torch import nn
from arkguardrl import features, structured
from . import economy_manager as compact
from .routes import features as route

SCHEMA = 'arkguardrl-core/full-state-economic-policy/v1'
CONTEXT_DIM = 128 + structured.CONTROL_ENCODED_DIM + 32 + compact.STATE_DIM
CANDIDATE_DIM = 256 + 16 + features.ACTION_DIM - 5 + route.ACTION_DIM + compact.ACTION_DIM


def encode(row, indices, native_inputs, parent, native_logp, prior_model, *, temperature=2.):
    """Only frozen learned representations; no teacher action/value labels."""
    if parent.width != 1 or temperature <= 0:
        raise ValueError('Explicit width1 representation and positive temperature required')
    with torch.no_grad():
        obs = parent.base.definitions.expand(native_inputs['obs'])
        hidden, entities, cells, present = parent.base.encoder.tables(obs)
        control = parent.base.control(obs, entities, present)
        route_hidden = parent.route_encoder(native_inputs['route_obs'])
        context = torch.cat((hidden, control, route_hidden, row['state']))
        candidates = native_inputs['candidates']
        native_features = torch.cat((entities[candidates[:, :4].long()].flatten(1),
            cells[candidates[:, 4].long()], candidates[:, 5:], native_inputs['route_actions']), -1)
        represented = {i for i in indices if i is not None}
        folded = [i for i in range(len(candidates)) if i not in represented]
        action_features = []
        for j, native in enumerate(indices):
            if native is None:
                weights = native_logp[folded].softmax(0)
                representation = (weights[:, None] * native_features[folded]).sum(0)
            else:
                representation = native_features[native]
            action_features.append(torch.cat((representation, row['actions'][j])))
        reference, _ = prior_model(row)
        return dict(context=context.detach(), actions=torch.stack(action_features).detach(),
                    prior=(reference / temperature).log_softmax(0).detach(),
                    untempered_prior=reference.detach())


class Policy(nn.Module):
    def __init__(self, temperature=2.):
        super().__init__()
        self.temperature = float(temperature)
        self.actor = nn.Sequential(nn.Linear(CONTEXT_DIM + CANDIDATE_DIM, 128), nn.Tanh(), nn.Linear(128, 1))
        self.critic = nn.Sequential(nn.Linear(CONTEXT_DIM, 128), nn.Tanh(), nn.Linear(128, 1))
        for layer in (self.actor[-1], self.critic[-1]):
            nn.init.zeros_(layer.weight)
            nn.init.zeros_(layer.bias)

    def forward(self, row):
        context = row['context'].expand(len(row['prior']), -1)
        delta = self.actor(torch.cat((context, row['actions']), -1)).squeeze(-1)
        logp = (row['prior'] + delta / self.temperature).log_softmax(0)
        return logp, self.critic(row['context']).squeeze(-1)


def gradient_norm(parameters):
    return float(sum((p.grad.detach().square().sum() for p in parameters if p.grad is not None),
                     torch.tensor(0.)).sqrt())


def update(model, optimizer, episodes, *, critic_only=False, seed=686):
    if not episodes or any(not e or not e[-1]['terminated'] or any(r['truncated'] for r in e) for e in episodes):
        raise ValueError('Only fresh complete natural on-policy manager episodes admitted')
    rows = []
    returns = []
    for episode in episodes:
        tail = 0.
        returns.append(sum(r['reward'] for r in episode))
        for i in reversed(range(len(episode))):
            r = episode[i]
            next_value = 0. if r['terminated'] else episode[i + 1]['value']
            delta = r['reward'] + next_value - r['value']
            tail = delta + (0. if r['terminated'] else (1. if critic_only else .95 ** r['settlements']) * tail)
            rows.append(dict(**r, advantage=tail, target=tail + r['value']))
    advantages = torch.tensor([r['advantage'] for r in rows])
    normalized = (advantages - advantages.mean()) / advantages.std(unbiased=False).clamp_min(1e-8)
    for r, a in zip(rows, normalized):
        r['normalized_advantage'] = a
    rng = random.Random(seed)
    updates, history = 0, []
    stop = 'epochs'
    for _ in range(8 if critic_only else 2):
        rng.shuffle(rows)
        for start in range(0, len(rows), 64):
            batch = rows[start:start + 64]
            optimizer.zero_grad(set_to_none=True)
            terms = []
            for r in batch:
                logp, value = model(r['inputs'])
                loss = .5 * (value - r['target']) ** 2
                if not critic_only:
                    ratio = (logp[r['index']] - r['logp']).exp()
                    advantage = r['normalized_advantage']
                    policy = -torch.minimum(ratio * advantage, ratio.clamp(.9, 1.1) * advantage)
                    anchor = (r['inputs']['prior'].exp() * (r['inputs']['prior'] - logp)).sum()
                    entropy = -(logp.exp() * logp).sum()
                    loss = loss + policy + .05 * anchor - .001 * entropy
                terms.append(loss)
            loss = torch.stack(terms).mean()
            if not torch.isfinite(loss):
                raise ValueError('Nonfinite economic PPO loss')
            loss.backward()
            actor_grad = gradient_norm(model.actor.parameters())
            critic_grad = gradient_norm(model.critic.parameters())
            # A large value error must not rescale the actor's entire gradient.
            nn.utils.clip_grad_norm_(model.actor.parameters(), .5, error_if_nonfinite=True)
            nn.utils.clip_grad_norm_(model.critic.parameters(), .5, error_if_nonfinite=True)
            optimizer.step()
            updates += 1
            with torch.no_grad():
                old_kls = []
                changes = []
                for r in batch:
                    new_logp, _ = model(r['inputs'])
                    old_kls.append((r['old_logps'].exp() * (r['old_logps'] - new_logp)).sum())
                    changes.append((new_logp.exp() - r['old_logps'].exp()).abs().max())
                kl = float(torch.stack(old_kls).mean())
            history.append(dict(loss=float(loss.detach()), actor_grad=actor_grad, critic_grad=critic_grad,
                                old_kl=kl, max_probability_change=float(torch.stack(changes).max())))
            if not critic_only and kl > .02:
                stop = 'old_kl'
                break
        if stop == 'old_kl':
            break
    return dict(rows=len(rows), updates=updates, stop=stop, returns=returns,
                advantage_std=float(advantages.std(unbiased=False)), history=history)


def save(path, model, metadata):
    torch.save(dict(schema=SCHEMA, temperature=model.temperature,
        state_dict={k:v.detach().cpu() for k,v in model.state_dict().items()}, metadata=metadata), path)


def load(path):
    payload = torch.load(path, map_location='cpu', weights_only=True)
    if payload['schema'] != SCHEMA:
        raise ValueError('Unknown full-state economic policy')
    model = Policy(payload['temperature'])
    model.load_state_dict(payload['state_dict'], strict=True)
    return model.eval(), payload['metadata']
