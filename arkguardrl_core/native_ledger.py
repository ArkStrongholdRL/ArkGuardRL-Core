"""Gym bridge to the public progress, history and reward owned by NativeSolo."""
from copy import deepcopy
import json
from uuid import uuid4

from arkguardrl.candidates import CandidateBlocked, DecisionHistory, build_decision
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
        self.result = None

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
        return self.result

    def decision(self):
        if self.result is None:
            raise RuntimeError('Reset before constructing a decision')
        if self.environment.numeric_encoder is None:
            return super().decision()
        return build_decision(self.result.observation, self.catalog, self.history,
                              candidate_encoder=self.environment._encode_candidates,
                              state_copy=self.environment._copy_state)


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
        if self._closed or self._state is None or self._reason or self._state['personalEnded']:
            raise RuntimeError('Reset required')
        self._previous = self._state
        try:
            raw = self.session.step_decision(json.dumps(action, allow_nan=False), self.tick_budget,
                                             self.max_advances - self._advances)
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
