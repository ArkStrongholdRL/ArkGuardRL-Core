"""Gym bridge to the public progress, history and reward owned by NativeSolo."""
from copy import deepcopy
import json
from numbers import Integral
from uuid import uuid4

from arkguardrl.candidates import CandidateBlocked, Decision, DecisionHistory, build_decision
from arkguardrl.contracts import fingerprint, canonical
from arkguardrl.decision_session import DecisionSession
from arkguardrl.environment import Transition
from .native_env import NativeDecisionEnv


class NativeLedgerHistory(DecisionHistory):
    def __init__(self, environment):
        self.environment = environment

    @property
    def progress(self):
        return self.environment._ledger['history']['progress']

    @property
    def prep_used(self):
        return self.environment._ledger['history']['used']

    def _check(self, state):
        if fingerprint(state) != self.progress['observation_id']:
            raise ValueError('Initialize/update history with the real environment result before encoding')

    def blocked(self, state):
        self._check(state)
        return frozenset(self.environment._ledger['blocked'])

    def snapshot(self, state):
        self._check(state)
        return deepcopy(self.environment._ledger['history'])


class NativeLedgerSession(DecisionSession):
    def __init__(self, environment, catalog):
        self.environment, self.catalog = environment, catalog
        self.history = NativeLedgerHistory(environment)
        self.result = self._decision = None

    def reset(self, **kwargs):
        if self.result is not None:
            raise RuntimeError('Reset once per decision session')
        self.result = self.environment.reset(**kwargs)
        return self.result

    def step(self, action):
        if self.result is None:
            raise RuntimeError('Reset before stepping')
        if fingerprint(self.result.observation) != self.history.progress['observation_id']:
            raise ValueError('Decision snapshot was mutated; no command executed')
        self.result = self.environment.step(action)
        self._decision = None
        return self.result

    def decision(self):
        if self.result is None:
            raise RuntimeError('Reset before constructing a decision')
        if self.environment.numeric_encoder is None:
            self._decision = super().decision()
            return self._decision
        if self.catalog is not self.environment._policy_catalog:
            raise ValueError('Numeric encoding requires the current public snapshot and catalog')
        if not hasattr(self.environment.session, 'build_current_policy'):
            self._decision = build_decision(self.result.observation, self.catalog, self.history,
                                           candidate_encoder=self.environment._encode_candidates,
                                           state_copy=self.environment._copy_state)
        else:
            from .native_policy import decode
            state = self.result.observation
            snapshot = self.history.snapshot(state)
            try:
                token, raw, buffers = self.environment.session.build_current_policy(state['revision'])
            except ValueError as error:
                if str(error) in ('Native candidate hard cap exceeded; domain was not sliced',
                                  'History/public domain exhausted; no fallback wait or forced Ready'):
                    raise CandidateBlocked(str(error)) from None
                raise
            actions = tuple(json.loads(raw))
            observation, features, schedules, experts = decode(buffers, len(actions))
            self._decision = Decision(self.environment._copy_state(state), observation, features,
                                      actions, schedules, experts, state['personalEnded'],
                                      history_json=canonical(snapshot))
            self.environment._policy_token = token
        return self._decision

    def step_index(self, index):
        if self._decision is None:
            raise RuntimeError('Construct a current decision before selecting its index')
        if (not isinstance(index, Integral) or isinstance(index, bool)
                or not 0 <= index < len(self._decision.actions)):
            raise ValueError('Select a current unpadded candidate index')
        if getattr(self.environment, '_policy_token', None) is None:
            return self.step(self._decision.actions[index])
        if fingerprint(self.result.observation) != self.history.progress['observation_id']:
            raise ValueError('Decision snapshot was mutated; no command executed')
        self.result = self.environment._step(index=int(index))
        self._decision = None
        return self.result


class NativeLedgerEnv(NativeDecisionEnv):
    def _copy_state(self, state):
        # Decode the committed native packet for an independently owned snapshot.
        return json.loads(self._raw)['observation']

    def _encode_candidates(self, state, prefix, catalog, history, prepared):
        from .native_policy import decode
        if catalog is not self._policy_catalog or history != self._ledger['history']:
            raise ValueError('Numeric encoding requires the current public snapshot and catalog')
        # build_decision just bound this snapshot once through history.snapshot().
        try:
            raw_actions, *buffers = self.session.build_ledger_policy(
                state['revision'], canonical(prefix), prepared)
        except ValueError as error:
            if str(error) in (
                    'Native candidate hard cap exceeded; domain was not sliced',
                    'History/public domain exhausted; no fallback wait or forced Ready'):
                raise CandidateBlocked(str(error)) from None
            raise
        actions = json.loads(raw_actions)
        return actions, *decode(buffers, len(actions))

    def _encode_numeric(self, state, actions, catalog, history):
        from .native_policy import encode
        if catalog is not self._policy_catalog or history != self._ledger['history']:
            raise ValueError('Numeric encoding requires the current public snapshot and catalog')
        return encode(self.session, state, actions, history, native_history=True)

    def _consume(self, raw):
        ledger = json.loads(raw)
        state = ledger['observation']
        # NativeProgressCore validates this exact owned state before committing it.
        self._raw = raw
        self._policy_token = None
        self._ledger = ledger
        self._state = state
        self._advances += ledger['advances']
        if not state['personalEnded']:
            if self._decisions >= self.max_decisions:
                self._reason = 'decision-budget'
            elif self._advances >= self.max_advances:
                self._reason = 'advance-budget'
        execution = ledger['execution']
        return Transition(state, state['personalEnded'],
                          bool(self._reason and not state['personalEnded']), self._reason,
                          accepted=execution['accepted'] if execution else None,
                          action_id=execution['action_id'] if execution else None,
                          execution=deepcopy(execution),
                          progress=deepcopy(ledger['history']['progress']))

    def reset(self, *, seed, difficulty='ABYSS'):
        if difficulty != 'ABYSS' or self._state is not None or self._closed:
            raise ValueError('Reset once per live Desperation decision session')
        return self._consume(self.session.reset_decision(seed, uuid4().hex))

    def step(self, action):
        return self._step(action=action)

    def _step(self, *, action=None, index=None):
        if self._closed or self._state is None or self._reason or self._state['personalEnded']:
            raise RuntimeError('Reset required')
        self._previous = self._state
        try:
            if index is None:
                raw = self.session.step_decision(json.dumps(action, allow_nan=False), self.tick_budget,
                                                 self.max_advances - self._advances)
            else:
                raw = self.session.step_index(self._state['revision'], self._policy_token, index,
                                              self.tick_budget, self.max_advances - self._advances)
            self._decisions += 1
            result = self._consume(raw)
            self._reward_consumed = False
            return result
        except BaseException:
            self.close(error=True)
            raise


class NativeLedgerReward:
    """Consume the reward already committed with the current native transition."""
    def __init__(self, environment):
        self.environment = environment

    def reset(self, state):
        if state is not self.environment._state:
            raise ValueError('Reward reset requires the native reset observation')

    def step(self, before, after):
        env = self.environment
        if env._reward_consumed or before is not env._previous or after is not env._state:
            raise ValueError('Disconnected or already-ended reward episode')
        env._reward_consumed = True
        return env._ledger['reward']
