"""GPL-3.0 read-only frozen-policy controller rollouts, without PPO rows.

This runner can be loaded explicitly alongside an immutable compatible historical
Core. Model/checkpoint identity stays with that Core; runner identity is separate.
Controller hooks receive public state, actual candidates and conditional logits.
"""
import hashlib
import json
import time
from dataclasses import replace
from pathlib import Path
import torch
from arkguardrl import rewards
from arkguardrl.contracts import canonical, fingerprint
from arkguardrl.decision_session import DecisionSession
from arkguardrl.trajectory import verify_trajectory
from arkguardrl_core.active_growth import PlanMemory
from arkguardrl_core.capacity_recovery import RecoveryMemory
from arkguardrl_core.prep_history import tensor_from_tracker
from arkguardrl_core.ready_cost import ReadyCost
from arkguardrl_core.recording import RecordingEnv
from arkguardrl_core.route_arrays import RouteFeatureCache
from arkguardrl_core.sampling import archive_case, weights_hash
from arkguardrl_core.strategic_options import make_memory, distribution, state_features
from arkguardrl_core.io import write


def collect(model, public, *, engine, data_sha256, seed, rng_seed, directory,
            controller, max_decisions=1024, max_advances=8192, wall=300):
    """Return only a JSON report; no LiveRow, optimizer, bootstrap or PPO API."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=False)
    original_weights = weights_hash(model)
    model.eval()
    native_rng = torch.Generator(device='cpu').manual_seed(rng_seed)
    option_rng = torch.Generator(device='cpu').manual_seed(rng_seed + 1000000000)
    cfg = model.planner_config
    memory = PlanMemory(cfg, public)
    recovery = RecoveryMemory()
    options = make_memory(cfg['options'], cfg, public)
    history_config = getattr(model, 'specialist_config', {}).get('prep_history')
    tracker = ReadyCost(history_config['loss_config']) if history_config else None
    cache = RouteFeatureCache()
    objective = rewards.NativeProgressCore()
    total_reward = 0.
    behavior = hashlib.sha256()
    decisions = bands = rejected = 0
    ready = []
    item_exit = None
    item_exits = []
    environment = RecordingEnv(engine, expected_data_sha256=data_sha256,
        deadline=time.monotonic() + wall, max_decisions=max_decisions,
        max_advances=max_advances, trace_path=directory / 'trajectory.jsonl',
        log_mode='full', trace_max_bytes=128*1024**2,
        source='scripted-diagnostic', policy_id=controller.identity)
    with DecisionSession(environment, public) as session:
        result = session.reset(seed=seed, difficulty='ABYSS')
        objective.reset(result.observation)
        stage = result.observation['public']['stageId']
        boss_round = result.observation['public']['bossRound']
        while not (result.terminated or result.truncated):
            before = result.observation
            native = session.decision()
            temp = [p for p in before['own']['temp'] if p]
            item_step = False
            if (item_exit is None and before['phase'] == 'PREP' and session.history.prep_used >= 64
                    and not before['own']['canReady'] and len(temp) == 1 and temp[0]['kind'] == 'item'
                    and not before['own'].get('personalChoice')):
                item_exit = dict(uid=temp[0]['uid'], round=before['round'], remaining=2, stage='destroy')
            if item_exit is not None:
                item_step = True
                if item_exit['remaining'] <= 0 or item_exit['round'] != before['round']:
                    raise RuntimeError('Single-temp-item exit exceeded finite contract')
                allowed = [i for i,a in enumerate(native.actions) if
                    (item_exit['stage'] == 'destroy' and a['t'] == 'g.destroy' and a['uid'] == item_exit['uid']) or
                    (item_exit['stage'] == 'ready' and a['t'] == 'g.ready' and a.get('ready') and not temp and before['own']['canReady'])]
                if len(allowed) != 1:
                    raise RuntimeError('Single-temp-item exit lacks exact native candidate')
                i = allowed[0]
                decision = replace(native, actions=(native.actions[i],), candidate_features=(native.candidate_features[i],),
                    schedule_ids=(native.schedule_ids[i],), expert_ids=(native.expert_ids[i],))
            else:
                decision = recovery.constrain(native, public)
            inputs = model.row(decision, environment.geometry(), public, route_cache=cache)
            if tracker is not None:
                inputs['prep_history'] = tensor_from_tracker(tracker, before)
            inputs['plan_context'] = memory.tensor(before)
            inputs['recovery_context'] = recovery.tensor()
            previous = options.tensor(before)
            menu = options.menu(before, session.history.prep_used)
            if menu is not None:
                mt = options.menu_tensor(menu, before)
                with torch.no_grad():
                    logp, _ = distribution(model, state_features(model, inputs, previous), mt)
                sampled = int(torch.multinomial(logp.exp(), 1, generator=option_rng))
                selected = controller.option(before, menu, sampled,
                    prefix_sha256=behavior.hexdigest(), history=session.history.snapshot(before),
                    native_rng=native_rng.get_state(), option_rng=option_rng.get_state())
                if not 0 <= selected < len(menu):
                    raise ValueError('Controller option outside actual menu')
                options.choose(menu[selected], before)
                environment._record(dict(kind='diagnostic-option', decisionIndex=decisions,
                    menu=menu, sampled=sampled, selected=selected, native_logps=logp.tolist()))
            inputs['option_context'] = options.tensor(before)
            if cfg['options'].get('schema') == 'arkguardrl-core/search-and-adapt-options/v1':
                from arkguardrl_core.search_options import relations
                inputs['goal_relations'] = relations(before, decision.actions, public, inputs['option_context'], options.requested_offer)
            with torch.no_grad():
                logp, _ = model(inputs)
                logits = torch.distributions.Categorical(logits=logp)
                sampled = int(torch.multinomial(logits.probs, 1, generator=native_rng))
            selected = controller.action(before, decision.actions, logp, sampled,
                'single-temp-item-exit' if item_step else recovery.stage)
            if not 0 <= selected < len(decision.actions):
                raise ValueError('Controller action outside actual candidate domain')
            action = decision.actions[selected]
            environment._record(dict(kind='diagnostic-action', decisionIndex=decisions,
                sampled=sampled, selected=selected, controller=controller.snapshot(),
                nativeLogProbability=float(logits.log_prob(torch.tensor(selected))),
                on_policy_admissible=False))
            if action['t'] == 'g.ready' and action.get('ready'):
                ready.append(dict(round=before['round'], hp=before['own']['lp'],
                    board=before['own']['board'], bonds=before['own']['bonds']))
            result = session.step(action)
            if item_step:
                item_exit['remaining'] -= 1
                if not result.accepted:
                    raise RuntimeError('Single-temp-item exit candidate rejected')
                item_exits.append(dict(before=dict(item_exit),action=action,accepted=result.accepted))
                if item_exit['stage'] == 'destroy':
                    if any(p for p in result.observation['own']['temp']) or not result.observation['own']['canReady']:
                        raise RuntimeError('Destroy did not clear exact temp-item obligation')
                    item_exit['stage'] = 'ready'
                else:
                    item_exit = None
            else:
                recovery.note(before, action, result)
            memory.note(before, action, result.accepted)
            options.note(before, action, result)
            controller.note(before, action, result)
            if action['t'] == 'g.band':
                if (not result.accepted or result.observation['own']['bandId'] != 'band_sarkazb'
                        or result.observation['own']['lp'] != 45):
                    raise ValueError('Accepted Goliath/HP45 contract failed')
                bands += 1
            info = objective.step(before, result.observation)
            total_reward += info['reward']
            behavior.update(canonical((action, result.accepted, fingerprint(before),
                fingerprint(result.observation))))
            rejected += not result.accepted
            decisions += 1
        final = result.observation
        controller.finish(final)
        if result.truncated or not result.terminated or bands != 1:
            raise RuntimeError('Incomplete/technical episode; not a natural value observation')
    if weights_hash(model) != original_weights:
        raise ValueError('Frozen model weights changed')
    audit = verify_trajectory(directory / 'trajectory.jsonl')
    report = dict(seed=seed, stage_id=stage, boss_round=boss_round,
        natural=True, truncated=False, reward=total_reward,
        rounds_passed=round(objective.progress * boss_round), victory=info['victory'],
        final_hp=final['own']['lp'], decisions=decisions, rejections=rejected,
        behavior_sha256=behavior.hexdigest(), model_weights_sha256=original_weights,
        controller=controller.snapshot(), events=controller.events, fork=controller.fork,
        ready=ready, single_temp_item_exits=item_exits,
        trajectory_audit=audit, on_policy_admissible=False, updates=0)
    archive_case(directory)
    write(directory / 'summary.json', report)
    return report
