"""Optimized complete-episode collection; independent of experiment plans/seeds.

GPL-3.0-or-later, extracted from ArkGuardRL route_ppo_fast. No archived rows are
accepted as PPO data; only complete live episodes leave this collector.
"""
from collections import Counter
from dataclasses import dataclass
import gzip
import hashlib
import json
import math
from pathlib import Path
import shutil
import time
import torch
from arkguardrl import learn, neural, rewards
from arkguardrl.contracts import fingerprint, canonical, action_id
from arkguardrl.decision_session import DecisionSession
from arkguardrl.training import Candidate, Transition
from arkguardrl.trajectory import verify_trajectory
from .routes import residual as rp
from .recording import RecordingEnv
from .io import sha, write
from .construction import potential as construction_potential
from .ready_cost import ReadyCost

ROLLOUT_SCHEMA = 'arkguardrl-core/live-capture-receipt/v1'

def chargeable(before, action):
    return before['phase'] == 'PREP' and action['t'] not in ('g.ready', 'g.choice')


def tensor_sha(value):
    return hashlib.sha256(value.detach().cpu().contiguous().numpy().tobytes()).hexdigest()


def input_sha(row):
    return fingerprint({k:tensor_sha(v) for k,v in row.inputs().items()})


def archive_case(directory):
    """Audit actual native records, then gzip byte-for-byte without tensor archives."""
    trace=directory/'trajectory.jsonl';before=sha(trace)
    compressed=trace.with_suffix('.jsonl.gz')
    with trace.open('rb') as src,gzip.open(compressed,'xb',compresslevel=3) as dst:
        shutil.copyfileobj(src,dst)
    digest=hashlib.sha256()
    with gzip.open(compressed,'rb') as stream:
        for block in iter(lambda:stream.read(1024*1024),b''):digest.update(block)
    if digest.hexdigest()!=before:raise ValueError('Compressed trajectory digest mismatch')
    write(directory/'archive.json',{'raw_sha256':before,'gzip_sha256':sha(compressed),
        'raw_bytes':trace.stat().st_size,'gzip_bytes':compressed.stat().st_size,
        'verified_lossless':True})
    trace.unlink()


def weights_hash(model, actor_only=False):
    digest = hashlib.sha256()
    for name, value in sorted(model.state_dict().items()):
        if actor_only and name.startswith('base.critic.'):
            continue
        digest.update(name.encode())
        digest.update(str((tuple(value.shape), value.dtype)).encode())
        digest.update(value.detach().cpu().contiguous().numpy().tobytes())
    return digest.hexdigest()


@dataclass
class LiveRow:
    capture: dict
    transition: Transition | None
    route_obs: torch.Tensor
    route_actions: torch.Tensor
    geometry: dict
    old_logps: torch.Tensor
    anchor_logps: torch.Tensor
    policy_sha: str
    base_reward: float
    prep_cost: float

    def inputs(self):
        return {k: self.capture[k] for k in ('obs', 'candidates', 'schedules', 'experts')} | {
            'route_obs': self.route_obs, 'route_actions': self.route_actions}

    def evidence(self):
        return {'capture': self.capture, 'transition': self.transition.to_dict(),
                'route_obs': self.route_obs, 'route_actions': self.route_actions,
                'geometry': self.geometry, 'old_logps': self.old_logps,
                'anchor_logps': self.anchor_logps, 'policy_sha256': self.policy_sha,
                'base_reward': self.base_reward, 'prep_cost': self.prep_cost}


def snapshot_from_inputs(decision, history, inputs, *, episode_id, index, reason,
                         policy_revision, old_logp, value):
    """Transfer owned CPU input tensors into the native live capture once.

    model.row creates fresh tensors; collection/model code never mutates them.
    Trainable hidden activations are not cached across optimizer updates.
    """
    actions = json.loads(canonical(list(decision.actions)))
    record = {'observation': decision.state,
        'history': (json.loads(decision.history_json) if decision.history_json is not None
                    else json.loads(canonical(history.snapshot(decision.state)))),
        'actions': actions, 'action_ids': [action_id(a) for a in actions],
        **{k: inputs[k].detach() for k in ('obs', 'candidates', 'schedules', 'experts')},
        'index': index, 'reason': reason, 'episode_id': episode_id,
        'captured_at': learn._utc(), 'policy_revision': policy_revision,
        'old_logp': old_logp, 'value': value}
    if any(record[k].device.type != 'cpu' for k in ('obs', 'candidates', 'schedules', 'experts')):
        raise ValueError('CPU capture tensors required')
    observation_bytes = learn._validate_record(record, on_policy=True)
    # Validation serializes this exact live mapping once. Decode those bytes to
    # isolate capture from the caller's mutable decision, without re-encoding.
    record['observation'] = json.loads(observation_bytes)
    return record


def collect(ctx, model, anchor, *, seed, rng_seed, revision, checkpoint, directory, log_mode=None,
            session_pool=None, on_row=None, inference=None):
    """Collect one complete live episode. Never reload PPO rows from artifacts."""
    ctx.check(sources=True)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=False)
    rng = torch.Generator(device='cpu').manual_seed(rng_seed)
    model.eval()
    ctx.bind_policy(model, checkpoint, revision)
    policy_sha = sha(checkpoint)
    state_hash = weights_hash(model)
    episode_id = f'{directory.parent.name}/{directory.name}/{seed}/{revision}'
    rows, levels = [], []
    pending = None
    if log_mode is None:
        log_mode = 'full' if ctx.collection_index % ctx.config['full_trace_every'] == 0 else 'compact'
    ctx.collection_index += 1
    used_bytes = 0
    reward = rewards.NativeProgressCore()
    ready_cost = ReadyCost(ctx.config['end_prep_loss']) if ctx.config.get('end_prep_loss') else None
    counts, components = Counter(), Counter()
    rejected, total_reward = 0, 0.0
    started = time.monotonic()
    phase_times = Counter()
    behavior = hashlib.sha256()
    from .route_arrays import RouteFeatureCache
    route_cache = RouteFeatureCache() if model.route_backend == 'array' else None
    trace_name = 'trajectory.jsonl' if log_mode == 'full' else 'receipts.jsonl.gz'
    environment = RecordingEnv(ctx.engine_root, deadline=min(ctx.deadline, started + ctx.config['game_wall']),
        session_pool=session_pool,
        expected_data_sha256=ctx.contract.data_sha256, trace_path=directory / trace_name,
        log_mode=log_mode, trace_max_bytes=ctx.config['trace_max_bytes'],
        source='ppo-policy', policy_id=f'{ctx.schema}:{policy_sha}:{revision}',
        max_decisions=ctx.config['max_decisions'], max_advances=ctx.config['max_advances'])
    with DecisionSession(environment, ctx.public) as session:
        ctx.contract.assert_runtime(environment.session.identity, difficulty=ctx.contract.difficulty)
        result = session.reset(seed=seed, difficulty=ctx.contract.difficulty)
        reward.reset(result.observation)
        while not (result.terminated or result.truncated):
            if len(rows) % 64 == 0:
                ctx.check()
            phase_started = time.monotonic()
            before = result.observation
            decision = session.decision()
            geometry = environment.geometry()
            inputs = model.row(decision, geometry, ctx.public, route_cache=route_cache)
            phase_times['feature_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            with torch.no_grad():
                if inference is None:
                    logp, value = model(inputs)
                    anchor_logps = anchor(inputs)[0].log_softmax(0).detach().cpu().clone()
                else:
                    logp, value, anchor_logps = inference(inputs, len(rows))
                distribution = torch.distributions.Categorical(logits=logp)
                chosen = torch.multinomial(distribution.probs, 1, generator=rng).squeeze(0)
                index, old_logp, value = int(chosen), float(distribution.log_prob(chosen)), float(value)
                old_logps = distribution.logits.detach().cpu().clone()
            phase_times['inference_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            if pending is not None:
                rows[-1].transition = Transition.from_step(ctx.objective_contract,
                    **pending, next_value=value)
                environment._record({'kind': 'bootstrap-link', 'decisionIndex': len(rows)-1,
                    'nextValue': value, 'source': 'next-live-decision-value'})
                pending = None
                if on_row is not None:
                    on_row(rows[-1])
            capture = snapshot_from_inputs(decision, session.history, inputs, episode_id=episode_id,
                index=index, reason='fresh-autonomous-route-policy', policy_revision=revision,
                old_logp=old_logp, value=value)
            size = (learn._record_bytes(capture) + 32 * ctx.contract.feature_dim
                    + 2 * sum(x.numel() * x.element_size() for x in
                              (inputs['route_obs'], inputs['route_actions'], old_logps, anchor_logps))
                    + 2 * len(json.dumps(geometry)))
            used_bytes += size
            if used_bytes > ctx.config['max_episode_bytes']:
                raise MemoryError('Live episode capture bound exceeded; batch not trained')
            action = decision.actions[index]
            candidate = Candidate.from_native(ctx.objective_contract, before, action,
                tuple(decision.observation_features) + tuple(decision.candidate_features[index]))
            environment._record({'kind': 'route-input', 'schema': rp.SCHEMA,
                'observationId': fingerprint(before), 'geometry': geometry,
                'geometrySha256': fingerprint(geometry), 'selectedIndex': index,
                'selectedLogProbability': old_logp, 'policyRevision': revision,
                'policySha256': policy_sha})
            phase_times['capture_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            result = session.step(action)
            phase_times['environment_seconds'] += time.monotonic()-phase_started
            behavior.update(canonical((action, result.accepted, fingerprint(before), fingerprint(result.observation))))
            if type(result.accepted) is not bool:
                raise RuntimeError('Selected action was not executed; batch not trained')
            info = reward.step(before, result.observation)
            if ready_cost is not None:
                loss,receipt=ready_cost.step(before,result.observation,action,result.accepted)
                info={**info,'schema':ctx.objective_contract.reward_schema,
                    'base_schema':info['schema'],'reward':info['reward']-loss,
                    'components':{**info['components'],'end_prep_loss':-loss},
                    'end_prep_loss':receipt}
            construction=ctx.config.get('construction_potential')
            if construction is not None:
                before_phi=construction_potential(before,ctx.public,construction)
                after_phi=construction_potential(result.observation,ctx.public,construction)
                shaped=after_phi-before_phi
                info={**info,'schema':ctx.objective_contract.reward_schema,
                    'base_schema':info['schema'],
                    'reward':info['reward']+shaped,
                    'components':{**info['components'],'construction_potential':shaped},
                    'construction_before':before_phi,'construction_after':after_phi}
            charged = ctx.config['prep_action_cost'] if ctx.cost_arm and chargeable(before, action) else 0.
            objective_reward = info['reward'] - charged
            pending = dict(episode_id=episode_id, decision_index=len(rows),
                before_observation=before, candidate=candidate,
                result=result, reward=objective_reward, value=value)
            next_value = None
            transition = None
            if result.terminated or result.truncated:
                # Incomplete episodes are rejected below. Preserve their true tail
                # estimate in diagnostics, never invent a terminal for a cutoff.
                next_value = 0. if result.terminated else learn._bootstrap(model.base, session, 'cpu')
                transition = Transition.from_step(ctx.objective_contract, **pending, next_value=next_value)
                pending = None
            rows.append(LiveRow(capture, transition, inputs['route_obs'].detach().clone(),
                inputs['route_actions'].detach().clone(), geometry, old_logps, anchor_logps, policy_sha, info['reward'], charged))
            if on_row is not None and rows[-1].transition is not None:
                on_row(rows[-1])
            environment._record({'kind': 'policy-sample', 'experiment': ctx.schema,
                'episodeId': episode_id, 'decisionIndex': len(rows)-1,
                'policyRevision': revision, 'policySha256': policy_sha,
                'selectedActionId': candidate.action_id, 'selectedIndex': index,
                'oldLogProbability': old_logp, 'value': value, 'nextValue': next_value,
                'nextValuePending': next_value is None,
                'reward': info, 'objectiveReward': objective_reward, 'prepCost': charged,
                'objectiveContractId': ctx.objective_contract.contract_id, 'candidateCount': len(decision.actions)})
            counts[action['t']] += 1
            rejected += int(not result.accepted)
            total_reward += objective_reward
            components['prep_action_cost'] -= charged
            components.update(info['components'])
            event = learn._level_event(before, result)
            if event:
                levels.append(event)
    if not result.terminated or result.truncated or not rows or pending is not None:
        raise RuntimeError('Only complete natural episodes admitted; unfinished batch discarded')
    if weights_hash(model) != state_hash:
        raise ValueError('Policy/critic weights changed during a live episode')
    # Native validation checks continuity and terminal bootstrap before any learning.
    learn._advantages(ctx.objective_contract, rows, 1.0)
    audit = (verify_trajectory(directory / 'trajectory.jsonl') if log_mode == 'full'
             else environment.audit())
    if audit['status'] not in ('personal-ended', 'game-ended') or not audit['public_feedback_verified']:
        raise ValueError('Incomplete public trajectory audit')
    report = {'seed': seed, 'rng_seed': rng_seed, 'decoder': 'sampled',
        'status': 'personal-ended', 'reason': None, 'error': None,
        'rounds_passed': round(reward.progress * reward.boss_round),
        'final_level': result.observation['own']['shop']['level'],
        'level_events': levels, 'rejected': rejected, 'decisions': len(rows),
        'return': total_reward, 'reward_components': dict(components), 'actions': dict(counts),
        'base_return': total_reward-components['prep_action_cost'],
        'total_prep_cost': -components['prep_action_cost'],
        'objective_contract_id': ctx.objective_contract.contract_id,
        'policy_sha256': policy_sha, 'policy_revision': revision,
        'weights_sha256': state_hash, 'capture_bytes_upper_bound': used_bytes,
        'trajectory_audit': audit, 'log_mode': log_mode,
        'collection_elapsed': time.monotonic()-started,
        'route_cache': dict(route_cache.stats) if route_cache is not None else {},
        'sampling_rng': 'per-episode-cpu-generator/v1', 'behavior_sha256': behavior.hexdigest(),
        'phases': dict(phase_times)}
    write(directory / 'summary.json', report)
    if log_mode == 'full':
        # Full diagnostics retain independent post-read verification/compression.
        audit_rows = [{'decision_index': row.transition.decision_index,
            'action_id': row.transition.candidate.action_id,
            'input_sha256': input_sha(row), 'old_logps_sha256': tensor_sha(row.old_logps),
            'anchor_logps_sha256': tensor_sha(row.anchor_logps)} for row in rows]
        write(directory / 'capture-audit.json', {'schema': ROLLOUT_SCHEMA,
            'objective_contract': ctx.objective_contract.to_dict(), 'rows': audit_rows,
            'role': 'hash receipts only; not a PPO replay dataset'})
        archive_case(directory)
    else:
        # Online independent observer checked each actual engine transition;
        # avoid a second full-state dump, reread, gzip and decompression pass.
        write(directory / 'behavior-audit.json', environment.behavior() | {
            'seed': seed, 'trajectory_audit': audit,
            'actual_upgrades': sum(e['levelIncrease'] for e in levels)})
        write(directory / 'archive.json', {'schema': audit['schema'],
            'bytes': environment.writer.bytes, 'sha256': sha(directory / trace_name),
            'full_observations_archived': False, 'offline_ppo_ready': False})
    report['elapsed'] = time.monotonic()-started
    write(directory / 'summary.json', report)
    ctx.check(sources=True)
    return rows, report


def verify_live_metadata(ctx, rows, revision, checkpoint):
    policy_sha = sha(checkpoint)
    for row in rows:
        capture = row.capture
        observation_bytes = learn._validate_record(capture, on_policy=True)
        if (capture['policy_revision'] != revision or row.policy_sha != policy_sha
                or capture['action_ids'][capture['index']] != row.transition.candidate.action_id
                or row.geometry['observation_id'] != hashlib.sha256(observation_bytes).hexdigest()
                or row.transition.contract_id != ctx.objective_contract.contract_id):
            raise ValueError('Stale/mismatched live row')
        action=capture['actions'][capture['index']]
        expected_cost=ctx.config['prep_action_cost'] if ctx.cost_arm and chargeable(capture['observation'],action) else 0.
        if (row.prep_cost!=expected_cost or row.transition.reward!=row.base_reward-row.prep_cost):
            raise ValueError('Objective reward/cost identity mismatch')
        for name, value in (('old', row.old_logps), ('anchor', row.anchor_logps)):
            if (value.device.type != 'cpu' or value.dtype != torch.float32
                    or value.shape != (len(capture['actions']),)):
                raise ValueError('Live '+name+' distribution shape/device mismatch')
            neural._finite(value, 'live '+name+' distribution')
    ctx.check(sources=True)


def verify_replay(row, logp, value):
    """Same per-row capture tolerances for scalar and batch replay."""
    capture = row.capture
    if (logp.device.type != 'cpu' or logp.shape != row.old_logps.shape
            or not torch.allclose(logp, row.old_logps, rtol=1e-5, atol=1e-5)
            or not math.isclose(float(logp[capture['index']]), capture['old_logp'], rel_tol=1e-5, abs_tol=1e-5)
            or not math.isclose(float(value), capture['value'], rel_tol=1e-5, abs_tol=1e-5)):
        raise ValueError('Captured policy probability/value replay mismatch')


def verify_live(ctx, model, rows, revision, checkpoint):
    verify_live_metadata(ctx, rows, revision, checkpoint)
    for row in rows:
        with torch.no_grad():
            logp, value = model(row.inputs())
            logp = logp.log_softmax(0)
        verify_replay(row, logp, value)
    ctx.check(sources=True)


def distribution_metrics(model, rows):
    total = Counter()
    with torch.no_grad():
        for row in rows:
            logp, _ = model(row.inputs())
            logp = logp.log_softmax(0)
            total['exact_old_kl'] += float((row.old_logps.exp() * (row.old_logps-logp)).sum())
            total['exact_anchor_kl'] += float((row.anchor_logps.exp() * (row.anchor_logps-logp)).sum())
    return {k: value/len(rows) for k, value in total.items()}

