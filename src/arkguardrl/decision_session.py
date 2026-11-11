"""One owner for native environment results, history and decision construction."""
from .candidates import DecisionHistory, build_decision
from .structured import encode_observation
from .contracts import fingerprint


class DecisionSession:
    """Thin facade; raw SoloEnv remains available for explicit diagnostics.

    step owns history advancement, including rejected actions. Callers must not
    separately initialize/record the history. Returned decisions own snapshots.
    """
    def __init__(self, environment, catalog):
        self.environment, self.catalog = environment, catalog
        self.history = DecisionHistory()
        self.result = None

    def __enter__(self):
        self.environment.__enter__()
        return self

    def __exit__(self, *args):
        return self.environment.__exit__(*args)

    def reset(self, **kwargs):
        if self.result is not None:
            raise RuntimeError("Reset once per decision session")
        self.result = self.environment.reset(**kwargs)
        self.history.initialize(self.result)
        return self.result

    def decision(self):
        if self.result is None:
            raise RuntimeError("Reset before constructing a decision")
        return build_decision(self.result.observation, self.catalog, self.history,
                              encoder=getattr(self.environment, 'numeric_encoder', None))

    def step(self, action):
        if self.result is None:
            raise RuntimeError("Reset before stepping")
        before = self.result.observation
        if fingerprint(before) != self.history.progress["observation_id"]:
            raise ValueError("Decision snapshot was mutated; no command executed")
        result = self.environment.step(action)
        # An episode budget may return without executing a command.
        if result.action_id is not None:
            self.history.record(before, action, result=result)
        self.result = result
        return result

    def observation_features(self):
        if self.result is None:
            raise RuntimeError("Reset before encoding a successor")
        encoder = getattr(self.environment, 'numeric_encoder', None)
        if encoder is not None:
            state = self.result.observation
            self.catalog.validate_state(state)
            return encoder(state, (), self.catalog, self.history.snapshot(state))[0]
        return encode_observation(self.result.observation, self.catalog, self.history)
