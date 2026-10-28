"""Optimized complete-episode collection; independent of experiment plans/seeds.

GPL-3.0-or-later, extracted from ArkGuardRL route_ppo_fast. No archived rows are
accepted as PPO data; only complete live episodes leave this collector.
"""
from collections import Counter
from dataclasses import dataclass, field
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
    prep_history: torch.Tensor = field(default_factory=lambda: torch.empty(0, dtype=torch.float32))

    plan_context: torch.Tensor = field(default_factory=lambda: torch.empty(0,dtype=torch.float32))
    battle_count: int = 0

    recovery_context: torch.Tensor = field(default_factory=lambda: torch.empty(0,dtype=torch.float32))
    recovery_domain: dict | None = None

    option_context: torch.Tensor = field(default_factory=lambda: torch.empty(0,dtype=torch.float32))
    option_event: dict | None = None

    goal_relations: torch.Tensor = field(default_factory=lambda: torch.empty(0,dtype=torch.float32))

    reward_binding: torch.Tensor = field(default_factory=lambda: torch.empty(0,dtype=torch.float32))
    binding_receipt: dict | None = None

    def inputs(self):
        result = {k: self.capture[k] for k in ('obs', 'candidates', 'schedules', 'experts')} | {
            'route_obs': self.route_obs, 'route_actions': self.route_actions}
        if self.prep_history.numel(): result['prep_history'] = self.prep_history
        if self.plan_context.numel():result['plan_context']=self.plan_context
        if self.recovery_context.numel():result['recovery_context']=self.recovery_context
        if self.option_context.numel():result['option_context']=self.option_context
        if self.goal_relations.numel():result['goal_relations']=self.goal_relations
        if self.reward_binding.numel():result['reward_binding']=self.reward_binding
        return result

    def evidence(self):
        return {'capture': self.capture, 'transition': self.transition.to_dict(),
                'route_obs': self.route_obs, 'route_actions': self.route_actions,
                'geometry': self.geometry, 'old_logps': self.old_logps,
                'anchor_logps': self.anchor_logps, 'policy_sha256': self.policy_sha,
                'base_reward': self.base_reward, 'prep_cost': self.prep_cost,
                'prep_history': self.prep_history,'plan_context':self.plan_context,'battle_count':self.battle_count,
                'recovery_context':self.recovery_context,'recovery_domain':self.recovery_domain,
                'option_context':self.option_context,'option_event':self.option_event,'goal_relations':self.goal_relations,
                'reward_binding':self.reward_binding,'binding_receipt':self.binding_receipt}


def snapshot_from_inputs(decision, history, inputs, *, episode_id, index, reason,
                         policy_revision, old_logp, value, validate=False):
    """Transfer the decision snapshot and CPU tensors into live capture.

    build_decision owns an isolated state and fresh action dictionaries. Neither
    the caller nor the learner may mutate them after this ownership transfer.
    PPO validates captures at its input boundary; debug tracing can check here too.
    """
    actions = list(decision.actions)
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
    if type(index) is not int or not 0 <= index < len(actions):
        raise ValueError('Selected index outside captured action domain')
    if validate:
        learn._validate_record(record, on_policy=True)
    return record


def collect(ctx, model, anchor, *, seed, rng_seed, revision, checkpoint, directory, log_mode=None,
            session_pool=None, on_row=None, inference=None):
    """Collect one complete live episode. Never reload PPO rows from artifacts."""
    ctx.check()
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
        log_mode = ctx.config['log_mode']
    tracing = log_mode == 'full'
    used_bytes = 0
    reward = rewards.NativeProgressCore()
    ready_cost = ReadyCost(ctx.config['end_prep_loss']) if ctx.config.get('end_prep_loss') else None
    history_config = getattr(model, 'specialist_config', {}).get('prep_history')
    if history_config != getattr(anchor, 'specialist_config', {}).get('prep_history'):
        raise ValueError('Actor/anchor PREP history contracts differ')
    if history_config and ready_cost and history_config['loss_config'] != ready_cost.config:
        raise ValueError('PREP history projection differs from reward contract')
    history_tracker = ReadyCost(history_config['loss_config']) if history_config else None
    planner_config=getattr(model,'planner_config',None)
    if planner_config!=getattr(anchor,'planner_config',None):raise ValueError('Actor/anchor plan identities differ')
    from .active_growth import PlanMemory
    plan_memory=PlanMemory(planner_config,ctx.public) if planner_config else None
    from .capacity_recovery import RecoveryMemory, RecoveryFailure
    recovery_memory=RecoveryMemory() if planner_config and 'recovery' in planner_config else None
    from .strategic_options import make_memory,distribution as option_distribution,state_features as option_state,is_search
    option_memory=make_memory(planner_config['options'],planner_config,ctx.public) if planner_config and 'options' in planner_config else None
    option_rng=torch.Generator(device='cpu').manual_seed(rng_seed+1000000000)
    counts, components = Counter(), Counter()
    rejected, total_reward = 0, 0.0
    started = time.monotonic()
    phase_times = Counter()
    behavior = hashlib.sha256() if tracing else None
    from .route_arrays import RouteFeatureCache
    route_cache = RouteFeatureCache() if model.route_backend == 'array' else None
    trace_path = directory / 'trajectory.jsonl' if tracing else None
    environment = RecordingEnv(ctx.engine_root, deadline=min(ctx.deadline, started + ctx.config['game_wall']),
        session_pool=session_pool,
        expected_data_sha256=ctx.contract.data_sha256, trace_path=trace_path,
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
            recovery_domain=None
            if recovery_memory is not None:
                recovery_domain=dict(history_json=decision.history_json,actions=list(decision.actions),
                    features=list(decision.candidate_features),schedules=list(decision.schedule_ids),experts=list(decision.expert_ids))
                try:
                    decision=recovery_memory.constrain(decision,ctx.public)
                except RecoveryFailure as error:
                    if tracing:
                        environment._record(dict(kind='capacity-recovery-failure',reason=str(error),
                            context=recovery_memory.snapshot(),technical_truncation=True,personal_termination=False))
                    raise
            geometry = environment.geometry()
            inputs = model.row(decision, geometry, ctx.public, route_cache=route_cache)
            if history_tracker is not None:
                from .prep_history import tensor_from_tracker
                inputs['prep_history'] = tensor_from_tracker(history_tracker, before)
            if plan_memory is not None:inputs['plan_context']=plan_memory.tensor(before)
            if recovery_memory is not None:inputs['recovery_context']=recovery_memory.tensor()
            option_event=None
            if option_memory is not None:
                if is_search(planner_config['options']):option_memory.stream_id=session.history.progress['stream_id']
                previous=option_memory.tensor(before)
                menu=option_memory.menu(before,session.history.prep_used)
                if menu is not None:
                    raw=option_state(model,inputs,previous)
                    menu_tensor=option_memory.menu_tensor(menu,before)
                    if inference is None:
                        with torch.no_grad():option_logp,option_value=option_distribution(model,raw,menu_tensor)
                    else:
                        option_logp,option_value,_=inference({'state':raw,'menu':menu_tensor},len(rows),selector=True)
                    option_index=int(torch.multinomial(option_logp.exp(),1,generator=option_rng))
                    option_event=dict(schema=planner_config['options']['schema'],menu=menu,menu_tensor=menu_tensor.tolist(),
                        previous_context=previous.tolist(),state=raw.tolist(),old_logps=option_logp.tolist(),
                        value=float(option_value),index=option_index)
                    option_memory.choose(menu[option_index],before)
                inputs['option_context']=option_memory.tensor(before)
            if planner_config and 'relations' in planner_config:
                from .goal_relations import encode as encode_relations
                inputs['goal_relations']=encode_relations(before,decision.actions,ctx.public,inputs['option_context'])
            if planner_config and is_search(planner_config.get('options')):
                from .search_options import relations as search_relations
                inputs['goal_relations']=search_relations(before,decision.actions,ctx.public,inputs['option_context'],option_memory.requested_offer)
            binding_receipt=None
            if planner_config and 'binding' in planner_config:
                from .offer_binding import encode as encode_binding
                inputs['reward_binding'],binding_receipt=encode_binding(planner_config['binding'],before,decision.actions,option_memory,session.history.progress['stream_id'])
            phase_times['feature_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            with torch.no_grad():
                if binding_receipt is not None:
                    from .offer_binding import decode,pushforward
                    if inference is None:
                        raw_logp,value=model.forward_raw(inputs)
                        anchor_logps=anchor(inputs)[0].log_softmax(0).detach().cpu().clone()
                    else:
                        raw_logp,value,anchor_logps=inference(inputs,len(rows))
                    raw_index,index=decode(raw_logp,inputs['reward_binding'],generator=rng)
                    logp=pushforward(raw_logp,inputs['reward_binding'])
                    binding_receipt.update(raw_selected_index=raw_index,raw_selected_logp=float(raw_logp[raw_index]),
                        mapped=index!=raw_index,raw_reward_selected=decision.actions[raw_index]['t']=='g.reward')
                elif inference is None:
                    logp, value = model(inputs)
                    anchor_logps = anchor(inputs)[0].log_softmax(0).detach().cpu().clone()
                else:
                    logp, value, anchor_logps = inference(inputs, len(rows))
                distribution = torch.distributions.Categorical(logits=logp, validate_args=False)
                if binding_receipt is None:
                    chosen = torch.multinomial(distribution.probs, 1, generator=rng).squeeze(0)
                    index=int(chosen)
                old_logp,value=float(distribution.log_prob(torch.tensor(index))),float(value)
                old_logps = distribution.logits.detach().cpu().clone()
            phase_times['inference_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            if pending is not None:
                rows[-1].transition = Transition.from_step(ctx.objective_contract,
                    **pending, next_value=value)
                if tracing:
                    environment._record({'kind': 'bootstrap-link', 'decisionIndex': len(rows)-1,
                        'nextValue': value, 'source': 'next-live-decision-value'})
                pending = None
                if on_row is not None:
                    on_row(rows[-1])
            capture = snapshot_from_inputs(decision, session.history, inputs, episode_id=episode_id,
                index=index, reason='fresh-autonomous-route-policy', policy_revision=revision,
                old_logp=old_logp, value=value, validate=tracing)
            size = (learn._record_bytes(capture) + 32 * ctx.contract.feature_dim
                    + 2 * sum(x.numel() * x.element_size() for x in
                              (inputs['route_obs'], inputs['route_actions'], old_logps, anchor_logps,
                               inputs.get('prep_history', torch.empty(0)), inputs.get('plan_context',torch.empty(0))))
                    + 2 * len(json.dumps(geometry))
                    + (2*len(canonical(option_event)) if option_event is not None else 0)
                    + 2*inputs.get('option_context',torch.empty(0)).numel()*4
                    + 2*inputs.get('goal_relations',torch.empty(0)).numel()*4
                    + 2*inputs.get('reward_binding',torch.empty(0)).numel()*4
                    + (2*len(canonical(binding_receipt)) if binding_receipt else 0))
            used_bytes += size
            if used_bytes > ctx.config['max_episode_bytes']:
                raise MemoryError('Live episode capture bound exceeded; batch not trained')
            action = decision.actions[index]
            if binding_receipt is not None:
                if tracing:
                    environment._record(dict(kind='offer-binding',decisionIndex=len(rows),receipt=binding_receipt,
                        mapSha256=tensor_sha(inputs['reward_binding']),executedIndex=index,effectiveLogProbability=old_logp))
            candidate = Candidate.from_native(ctx.objective_contract, before, action,
                tuple(decision.observation_features) + tuple(decision.candidate_features[index]))
            if tracing:
                environment._record({'kind': 'route-input', 'schema': rp.SCHEMA,
                    'observationId': fingerprint(before), 'geometry': geometry,
                    'geometrySha256': fingerprint(geometry), 'selectedIndex': index,
                    'selectedLogProbability': old_logp, 'policyRevision': revision,
                    'policySha256': policy_sha,
                    'prepHistorySha256': tensor_sha(inputs['prep_history']) if history_config else None,
                    'planContextSha256':tensor_sha(inputs['plan_context']) if plan_memory else None,
                    'goalRelationsSha256':tensor_sha(inputs['goal_relations']) if 'goal_relations' in inputs else None})
            phase_times['capture_seconds'] += time.monotonic()-phase_started
            phase_started = time.monotonic()
            settlements_before=environment.settlement_count
            result = session.step(action)
            if recovery_memory is not None:
                recovery_domain['successor']=result.observation
                try:
                    recovery_memory.note(before,action,result)
                except RecoveryFailure as error:
                    if tracing:
                        environment._record(dict(kind='capacity-recovery-failure',reason=str(error),
                            context=recovery_memory.snapshot(),technical_truncation=True,personal_termination=False))
                    raise
                if tracing:
                    environment._record(dict(kind='capacity-recovery',context=recovery_memory.snapshot(),
                        inputSha256=tensor_sha(inputs['recovery_context']),allowedActionIds=capture['action_ids']))
            battle_count=environment.settlement_count-settlements_before
            if plan_memory is not None:plan_memory.note(before,action,result.accepted)
            if option_memory is not None:option_memory.note(before,action,result)
            phase_times['environment_seconds'] += time.monotonic()-phase_started
            if tracing:
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
                if result.terminated:next_value=0.
                elif plan_memory is not None:
                    with torch.no_grad():
                        successor=torch.tensor(session.observation_features(),dtype=torch.float32)
                        next_value=float(model.bootstrap_value(successor,plan_memory.tensor(result.observation),recovery_memory.tensor(),option_memory.tensor(result.observation)) if option_memory else model.bootstrap_value(successor,plan_memory.tensor(result.observation),recovery_memory.tensor()) if recovery_memory else model.bootstrap_value(successor,plan_memory.tensor(result.observation)))
                else:next_value=learn._bootstrap(model.base,session,'cpu')
                transition = Transition.from_step(ctx.objective_contract, **pending, next_value=next_value)
                pending = None
            rows.append(LiveRow(capture, transition, inputs['route_obs'].detach().clone(),
                inputs['route_actions'].detach().clone(), geometry, old_logps, anchor_logps, policy_sha, info['reward'], charged,
                inputs.get('prep_history', torch.empty(0, dtype=torch.float32)).detach().clone(),
                inputs.get('plan_context',torch.empty(0,dtype=torch.float32)).detach().clone(),battle_count,
                inputs.get('recovery_context',torch.empty(0,dtype=torch.float32)).detach().clone(),recovery_domain,
                inputs.get('option_context',torch.empty(0,dtype=torch.float32)).detach().clone(),option_event,
                inputs.get('goal_relations',torch.empty(0,dtype=torch.float32)).detach().clone(),
                inputs.get('reward_binding',torch.empty(0,dtype=torch.float32)).detach().clone(),binding_receipt))
            if option_event is not None:
                if tracing:
                    environment._record(dict(kind='strategic-option-choice',decisionIndex=len(rows)-1,
                        event=option_event,optionContext=inputs['option_context'].tolist(),
                        selectedLogProbability=option_event['old_logps'][option_event['index']],
                        jointLogProbability=old_logp+option_event['old_logps'][option_event['index']],
                        nativeLogProbability=old_logp))
            if on_row is not None and rows[-1].transition is not None:
                on_row(rows[-1])
            if tracing:
                environment._record({'kind': 'policy-sample', 'experiment': ctx.schema,
                    'episodeId': episode_id, 'decisionIndex': len(rows)-1,
                    'policyRevision': revision, 'policySha256': policy_sha,
                    'selectedActionId': candidate.action_id, 'selectedIndex': index,
                    'oldLogProbability': old_logp, 'value': value, 'nextValue': next_value,
                    'nextValuePending': next_value is None,
                    'reward': info, 'objectiveReward': objective_reward, 'prepCost': charged,'settlements':battle_count,
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
    environment.require_complete()
    audit = verify_trajectory(trace_path) if tracing else None
    if audit is not None and (audit['status'] not in ('personal-ended', 'game-ended') or not audit['public_feedback_verified']):
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
        'sampling_rng': 'per-episode-cpu-generator/v1', 'behavior_sha256': behavior.hexdigest() if tracing else None,
        'phases': dict(phase_times),'settlements':sum(r.battle_count for r in rows),
        'plan_events':plan_memory.events if plan_memory else [],
        'plan_modes':dict(Counter(int(r.plan_context[1:5].argmax()) for r in rows)) if plan_memory else {},
        'capacity_recovery':recovery_memory.events if recovery_memory else [],
        'strategic_options':option_memory.events if option_memory else [],
        'option_choices':sum(r.option_event is not None for r in rows)}
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
    report['elapsed'] = time.monotonic()-started
    write(directory / 'summary.json', report)
    ctx.check()
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
    ctx.check()


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
    from .prep_history import verify_rows
    verify_rows(model, rows)
    from .active_growth import verify_rows as verify_plan_rows
    verify_plan_rows(model,rows,ctx.public)
    from .capacity_recovery import verify_rows as verify_recovery_rows
    verify_recovery_rows(model,rows,ctx.public)
    from .strategic_options import verify_rows as verify_option_rows
    verify_option_rows(model,rows,ctx.public,probabilities=True)
    from .goal_relations import verify_rows as verify_relations
    verify_relations(model,rows,ctx.public)
    for row in rows:
        from .offer_binding import verify_behavior
        verify_behavior(model,row)
        with torch.no_grad():
            logp, value = model(row.inputs())
            logp = logp.log_softmax(0)
        verify_replay(row, logp, value)
    ctx.check()


def distribution_metrics(model, rows):
    total = Counter()
    with torch.no_grad():
        for row in rows:
            logp, _ = model(row.inputs())
            logp = logp.log_softmax(0)
            total['exact_old_kl'] += float((row.old_logps.exp() * (row.old_logps-logp)).sum())
            total['exact_anchor_kl'] += float((row.anchor_logps.exp() * (row.anchor_logps-logp)).sum())
    return {k: value/len(rows) for k, value in total.items()}

