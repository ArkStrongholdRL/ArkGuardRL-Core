"""Decision-oriented native solo environment, without reward or model compatibility claims."""
from copy import deepcopy
from dataclasses import dataclass

from .contracts import action_id, decision_id
from .engine import NativeSession
from .trajectory import TrajectoryWriter
from .feedback import PublicProgress

ENV_SCHEMA = "arkguardrl/native-solo-decisions-feedback-intent-v3"


@dataclass(frozen=True)
class Transition:
    observation: dict
    terminated: bool
    truncated: bool
    reason: str | None
    accepted: bool | None = None
    metrics: dict | None = None
    action_id: str | None = None
    execution: dict | None = None
    progress: dict | None = None


def management_changes(before, after):
    """Actual changes immediately caused by an action, not rewards for attempted leveling."""
    a, b = before["own"], after["own"]
    return {"levelBefore": a["shop"]["level"], "levelAfter": b["shop"]["level"],
            "levelIncrease": max(0, b["shop"]["level"] - a["shop"]["level"]),
            "fundsDelta": b["funds"] - a["funds"],
            "mergesDelta": b["stats"]["merges"] - a["stats"]["merges"],
            "deployCountDelta": b["deployCount"] - a["deployCount"]}


class SoloEnv:
    """Explicit native actions, automatic non-decision clock progression, one episode.

    A rejected action remains a transition. Limits cause truncation; an exception
    remains an error. There is deliberately no reward/discount/bootstrap value yet.
    NativeSession remains available for lower-level diagnostic clock control.
    """

    def __init__(self, engine_root, *, trace_path=None, source=None, policy_id=None,
                 max_decisions=1024, max_advances=4096, **session_options):
        if type(max_decisions) is not int or type(max_advances) is not int or min(max_decisions, max_advances) < 1:
            raise ValueError("Positive integer episode limits required")
        session_options.setdefault("max_requests", max_decisions + max_advances + 4)
        self.session = NativeSession(engine_root, **session_options)
        self.trace_path, self.source, self.policy_id = trace_path, source, policy_id
        self.max_decisions, self.max_advances = max_decisions, max_advances
        self._trace = None
        self._state = None
        self._decisions = self._advances = 0
        self._seen = {}
        self._reason = None
        self._opened = False
        self._failed = False
        self._progress_state = PublicProgress()

    def __enter__(self):
        self.session.__enter__()
        self._opened = True
        return self

    def _call(self, method, **kwargs):
        try:
            return method(**kwargs)
        except BaseException:
            self._failed = True
            raise

    def _record(self, record):
        if self._trace is not None:
            self._trace.append(record)

    def _result(self, *, accepted=None, metrics=None, action_id=None, execution=None):
        return Transition(deepcopy(self._state), bool(self._state["personalEnded"]),
                          self._reason is not None and not self._state["personalEnded"],
                          self._reason, accepted, metrics, action_id, deepcopy(execution),
                          self._progress_state.snapshot(self._state))

    def reset(self, *, seed=1, difficulty="ABYSS"):
        if not self._opened or self._state is not None or self._trace is not None:
            raise RuntimeError("Reset once inside a new SoloEnv context")
        if self.trace_path is not None:
            self._trace = TrajectoryWriter(self.trace_path, identity={"environment": ENV_SCHEMA,
                **self.session.identity}, config={"seed": seed, "difficulty": difficulty,
                "maxDecisions": self.max_decisions, "maxAdvances": self.max_advances},
                source=self.source, policy_id=self.policy_id)
        self._state = self._call(self.session.reset, seed=seed, difficulty=difficulty)
        self._progress_state._observe_validated(self._state)
        self._record({"kind": "reset", "state": self._state})
        self._progress()
        return self._result()

    def _progress(self, *, settlement=False):
        while not self._state["gameEnded"]:
            if self._state["personalEnded"] and not settlement:
                return
            if self._state["decisionRequired"]:
                if settlement:
                    raise RuntimeError("Settlement unexpectedly requires a player decision")
                return
            if self._advances >= self.max_advances:
                self._reason = "advance-budget"
                return
            before = self._state
            reply = self._call(self.session.advance)
            self._advances += 1
            self._state = reply["state"]
            if reply["executed"]:
                self._progress_state._observe_validated(self._state)
            self._record({"kind": "advance", "executed": reply["executed"],
                          "before": before, "after": self._state})
            if reply["executed"] == 0 and not self._state["gameEnded"]:
                self._reason = "engine-stalled"
                return

    def step(self, action):
        if self._state is None or self._reason or self._state["personalEnded"]:
            raise RuntimeError("No active episode decision")
        if not self._state["decisionRequired"]:
            raise RuntimeError("Native state is not at a player decision")
        if self._decisions >= self.max_decisions:
            self._reason = "decision-budget"
            return self._result()
        intent = deepcopy(action)
        aid = action_id(intent)
        before = self._state
        key = (decision_id(before), aid)
        self._seen[key] = self._seen.get(key, 0) + 1
        reply = self._call(self.session.act, action=intent)
        self._decisions += 1
        self._state = reply["state"]
        execution = self._progress_state._observe_validated(self._state, action=intent, reply=reply)
        metrics = management_changes(before, self._state)
        self._record({"kind": "action", "action": intent, "actionId": aid,
                      "decisionId": key[0], "attemptAtContext": self._seen[key],
                      "accepted": reply["accepted"], "result": reply["result"], "execution": execution,
                      "metrics": metrics, "before": before, "after": self._state})
        self._progress()
        if (self._decisions >= self.max_decisions and not self._state["personalEnded"]
                and self._reason is None):
            self._reason = "decision-budget"
        return self._result(accepted=reply["accepted"], metrics=metrics, action_id=aid, execution=execution)

    def finish_settlement(self):
        """After a natural personal endpoint, collect the separate final game outcome."""
        if self._state is None or not self._state["personalEnded"] or self._reason:
            raise RuntimeError("Natural personal endpoint required")
        self._progress(settlement=True)
        return self._result()

    def close(self, *, error=False):
        try:
            if self._trace is not None:
                if error or self._failed:
                    status, reason = "error", "environment-exception"
                elif self._state and self._state["gameEnded"]:
                    status, reason = "game-ended", "natural"
                elif self._reason:
                    status, reason = "truncated", self._reason
                elif self._state and self._state["personalEnded"]:
                    status, reason = "personal-ended", "natural"
                else:
                    status, reason = "interrupted", "context-closed"
                self._trace.close(status=status, reason=reason)
        finally:
            self.session.close()
            self._opened = False

    def __exit__(self, exc_type, *_):
        self.close(error=exc_type is not None)
