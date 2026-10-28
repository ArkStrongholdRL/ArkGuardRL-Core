"""GPL-3.0 frozen public-prefix continuation, independent diagnostic only.

Loadable beside an explicitly pinned historical compatible Core. The model
retains its strict source identity; runner identity is recorded separately.
No LiveRow, optimizer, training buffer or checkpoint migration is provided.
"""
from pathlib import Path
import copy
import hashlib
import time
import torch
from arkguardrl import rewards
from arkguardrl.contracts import canonical, fingerprint
from arkguardrl.decision_session import DecisionSession
from arkguardrl.trajectory import verify_trajectory
from arkguardrl_core.active_growth import PlanMemory
from arkguardrl_core.capacity_recovery import RecoveryMemory
from arkguardrl_core import search_options, offer_binding
from arkguardrl_core.recording import RecordingEnv
from arkguardrl_core.route_arrays import RouteFeatureCache
from arkguardrl_core.sampling import archive_case, weights_hash
from arkguardrl_core.io import write

SCHEMA = 'arkguardrl-core/frozen-prefix-continuation/v1'


def collect(model, public, *, engine, data_sha256, seed, rng_seed, directory,
            prefix, source_choices, expected_state, controller, wall=300,
            max_decisions=1024, max_advances=8192, record_learning_signals=False,
            restart_rng_after_step=None):
    if restart_rng_after_step is not None and (
            len(restart_rng_after_step) != 2 or restart_rng_after_step[0] < 0):
        raise ValueError('Expected nonnegative decision step and continuation seed')
    directory = Path(directory)
    directory.mkdir(exist_ok=False, parents=True)
    weight_hash = weights_hash(model)
    model.eval()
    cfg = model.planner_config
    memory = PlanMemory(cfg, public)
    recovery = RecoveryMemory()
    upper = search_options.SearchMemory(cfg['options'], cfg, public)
    native_rng = torch.Generator(device='cpu').manual_seed(rng_seed)
    upper_rng = torch.Generator(device='cpu').manual_seed(rng_seed+1000000000)
    cache = RouteFeatureCache()
    objective = rewards.NativeProgressCore()
    total = 0.
    bands = rejected = 0
    ready = []
    behavior = hashlib.sha256()
    env = RecordingEnv(engine, expected_data_sha256=data_sha256,
        deadline=time.monotonic()+wall, max_decisions=max_decisions, max_advances=max_advances,
        trace_path=directory/'trajectory.jsonl', log_mode='full', trace_max_bytes=128*1024**2,
        source='scripted-diagnostic', policy_id=controller.identity)
    with DecisionSession(env, public) as session:
        result = session.reset(seed=seed, difficulty='ABYSS')
        objective.reset(result.observation)
        for n, source in enumerate(prefix):
            before = result.observation
            if canonical(before) != canonical(source['before']): raise ValueError('Native prefix public state mismatch')
            decision = recovery.constrain(session.decision(), public)
            if source['action'] not in decision.actions: raise ValueError('Prefix action outside legal recovery domain')
            memory.tensor(before)
            upper.stream_id = session.history.progress['stream_id']
            upper.tensor(before)
            menu = upper.menu(before, session.history.prep_used)
            choice = source_choices.get(n)
            if (menu is None) != (choice is None): raise ValueError('Source upper event schedule mismatch')
            if choice is not None:
                event = choice['event']
                if menu != event['menu']: raise ValueError('Prefix actual upper menu mismatch')
                upper.choose(menu[event['index']], before)
            result = session.step(source['action'])
            if result.accepted != source['accepted']: raise ValueError('Prefix native acceptance mismatch')
            recovery.note(before, source['action'], result)
            memory.note(before, source['action'], result.accepted)
            upper.note(before, source['action'], result)
            total += objective.step(before, result.observation)['reward']
            if source['action']['t'] == 'g.band':
                if not result.accepted or result.observation['own']['bandId'] != 'band_sarkazb' or result.observation['own']['lp'] != 45:
                    raise ValueError('Accepted Goliath/HP45 mismatch')
                bands += 1
        if canonical(result.observation) != canonical(expected_state): raise ValueError('Acquisition branch state differs')
        if result.terminated or result.truncated: raise ValueError('Branch must precede personal end')
        start = result.observation
        start_return = total
        start_progress = objective.progress
        controller.begin(start)
        start_memory = dict(plan=memory.tensor(start).tolist(), parent=upper.tensor(start).tolist(),
            recovery=recovery.tensor().tolist(), upper_events=copy.deepcopy(upper.events))
        env._record(dict(kind='continuation-branch', schema=SCHEMA, prefix_decisions=len(prefix),
            observationId=fingerprint(start), memory=start_memory, rng_seed=rng_seed,
            on_policy_admissible=False, controller=controller.snapshot()))
        steps = 0
        while not (result.terminated or result.truncated):
            if restart_rng_after_step is not None and steps == restart_rng_after_step[0]+1:
                continuation_seed = restart_rng_after_step[1]
                native_rng.manual_seed(continuation_seed)
                upper_rng.manual_seed(continuation_seed+1000000000)
                env._record(dict(kind='diagnostic-rng-restart', step=steps,
                    native_seed=continuation_seed, upper_seed=continuation_seed+1000000000,
                    on_policy_admissible=False))
            before = result.observation
            controller.observe(before)
            decision = recovery.constrain(session.decision(), public)
            row = model.row(decision, env.geometry(), public, route_cache=cache)
            row.update(plan_context=memory.tensor(before), recovery_context=recovery.tensor())
            upper.stream_id = session.history.progress['stream_id']
            previous = upper.tensor(before)
            menu = upper.menu(before, session.history.prep_used)
            if menu is not None:
                mt = upper.menu_tensor(menu, before)
                with torch.no_grad(): lp, value = search_options.distribution(model, search_options.state_features(model, row, previous), mt)
                index = int(torch.multinomial(lp.exp(), 1, generator=upper_rng))
                upper.choose(menu[index], before)
                env._record(dict(kind='continuation-upper', step=steps, menu=menu, selected=index,
                    logp=float(lp[index]), value=float(value), on_policy_admissible=False))
            row['option_context'] = upper.tensor(before)
            row['goal_relations'] = search_options.relations(before, decision.actions, public, row['option_context'], upper.requested_offer)
            row['reward_binding'], receipt = offer_binding.encode(cfg['binding'], before, decision.actions, upper, upper.stream_id)
            with torch.no_grad():
                raw, native_value = model.forward_raw(row)
                raw_index, sampled = offer_binding.decode(raw, row['reward_binding'], generator=native_rng)
                effective = offer_binding.pushforward(raw, row['reward_binding']).log_softmax(0)
                direct, direct_value = model(row)
            if not torch.allclose(effective, direct.log_softmax(0), atol=1e-5, rtol=1e-5): raise ValueError('Actual continuation scoring route mismatch')
            if not torch.allclose(native_value, direct_value, atol=1e-5, rtol=1e-5):
                raise ValueError('Actual continuation value route mismatch')
            inspect = getattr(controller, 'inspect_policy', None)
            if inspect is not None:
                inspect(before, decision.actions, effective, float(native_value), session.history.snapshot(before))
            if record_learning_signals:
                env._record(dict(kind='diagnostic-value', step=steps, value=float(native_value),
                    observation_id=fingerprint(before), on_policy_admissible=False))
            selected, reason = controller.action(before, decision.actions, effective, sampled)
            if not 0 <= selected < len(decision.actions): raise ValueError('Controller action outside support')
            action = decision.actions[selected]
            env._record(dict(kind='continuation-action', step=steps, raw_selected=raw_index,
                native_selected=sampled, executed=selected, action=action, reason=reason,
                actor_action_logp=float(effective[selected]), behavior_log_probability=None,
                behavior='independent-controller-mapping-no-PPO', binding=receipt,
                native_action_logp=float(effective[sampled]), controller=controller.snapshot(),
                on_policy_admissible=False))
            settling_before = env.settlement_count
            if action['t'] == 'g.ready' and action.get('ready'):
                ready.append(dict(step=steps, round=before['round'], hp=before['own']['lp'],
                    board=before['own']['board'], bonds=before['own']['bonds'], after=None))
            result = session.step(action)
            recovery.note(before, action, result)
            memory.note(before, action, result.accepted)
            upper.note(before, action, result)
            controller.note(before, action, result, env.settlement_count-settling_before)
            if action['t'] == 'g.ready' and action.get('ready'):
                ready[-1]['after'] = dict(hp=result.observation['own']['lp'], round=result.observation['round'],
                    phase=result.observation['phase'], bonds=result.observation['own']['bonds'])
            info = objective.step(before, result.observation)
            if record_learning_signals:
                env._record(dict(kind='diagnostic-outcome', step=steps,
                    reward=info['reward'], components=info['components'],
                    settlements=env.settlement_count-settling_before,
                    terminated=result.terminated, truncated=result.truncated,
                    on_policy_admissible=False))
            total += info['reward']
            behavior.update(canonical((action, result.accepted, fingerprint(before), fingerprint(result.observation))))
            rejected += not result.accepted
            steps += 1
        final = result.observation
        controller.finish(final)
        if result.truncated or not result.terminated or bands != 1: raise RuntimeError('Technical or budget cutoff is not a natural observation')
    if weights_hash(model) != weight_hash: raise ValueError('Frozen model changed')
    audit = verify_trajectory(directory/'trajectory.jsonl')
    report = dict(schema=SCHEMA, seed=seed, stage_id=start['public']['stageId'], natural=True,
        rounds_passed=round(objective.progress*final['public']['bossRound']), reward=total,
        suffix_reward=total-start_return, prefix_return=start_return, start_progress=start_progress,
        victory=info['victory'], final_hp=final['own']['lp'], start_round=start['round'], start_hp=start['own']['lp'],
        start_state_sha256=fingerprint(start), prefix_decisions=len(prefix), suffix_decisions=steps,
        start_memory=start_memory, rejections=rejected, ready=ready, controller=controller.snapshot(),
        events=controller.events, behavior_sha256=behavior.hexdigest(), model_weights_sha256=weight_hash,
        on_policy_admissible=False, updates=0, trajectory_audit=audit)
    archive_case(directory)
    write(directory/'summary.json', report)
    return report
