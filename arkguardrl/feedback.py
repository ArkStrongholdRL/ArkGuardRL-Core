"""Public execution receipts and observation-derived progress, independent of Torch.

Semantics researched against pinned GPL upstream player/{economy,views,round}.js
and garrisons/meta.js. No private roundStats/pieceCounters or engine queries.
"""
from copy import deepcopy
import uuid

from .contracts import action_id, decision_id, fingerprint
from .observation import validate_state
from .actions import describe_action

EXECUTION_SCHEMA = "arkguardrl/public-execution-report-v2"
PROGRESS_SCHEMA = "arkguardrl/public-observed-progress-v2"
ERRORS = ("NO_FUNDS", "HAND_FULL", "BOARD_FULL", "BAD_TILE", "BAD_TARGET", "SOLD_OUT",
          "MAX_LEVEL", "WRONG_PHASE", "NOT_YOUR_TURN", "ALREADY", "TEMP_NOT_EMPTY",
          "ELIMINATED", "NOT_READY", "BAD_MSG", "OTHER")


def owned(state):
    return {p["uid"]: (area, p) for area in ("board", "hand", "temp")
            for p in state["own"][area] if p is not None}


def counter(state, name):
    value = state["own"]["stats"].get(name)
    if type(value) is not int or value < 0:
        raise ValueError("Missing nonnegative public cumulative counter: " + name)
    return value


def execution_report(before, action, after, reply, *, schema=EXECUTION_SCHEMA):
    """A receipt describes the immediate native reply, before automatic advances."""
    accepted, result = reply.get("accepted"), reply.get("result")
    if type(accepted) is not bool or not isinstance(result, dict):
        raise ValueError("Real native acceptance/result required")
    if accepted != (result.get("ok") is True):
        raise ValueError("Native result/acceptance disagree")
    error = result.get("error")
    if error == "INTERNAL":
        raise RuntimeError("Native INTERNAL is a technical failure, not policy rejection")
    if not accepted and (not isinstance(error, str) or not error):
        raise ValueError("Native rejection omitted its error code")
    if accepted and error:
        raise ValueError("Accepted native result contains an error")
    description = describe_action(before, action)
    source, target, kind = description.source, description.target, description.kind
    a, b = before["own"], after["own"]
    layers_a = {r["bondId"]: r.get("layers", 0) for r in a["bonds"]}
    layers_b = {r["bondId"]: r.get("layers", 0) for r in b["bonds"]}
    changes = {"funds_delta": b["funds"] - a["funds"],
               "spent_delta": counter(after, "gold") - counter(before, "gold"),
               "refreshes_delta": counter(after, "refreshes") - counter(before, "refreshes"),
               "merges_delta": counter(after, "merges") - counter(before, "merges"),
               "level_delta": b["shop"]["level"] - a["shop"]["level"],
               "deploy_delta": b["deployCount"] - a["deployCount"],
               "hand_free_delta": sum(p is None for p in b["hand"]) - sum(p is None for p in a["hand"]),
               "temp_used_delta": sum(p is not None for p in b["temp"]) - sum(p is not None for p in a["temp"])}
    report = {"schema": schema, "action_id": action_id(action), "action_type": kind,
            "decision_id": decision_id(before), "before_id": fingerprint(before),
            "immediate_after_id": fingerprint(after), "round": before["round"], "phase": before["phase"],
            "accepted": accepted, "error": error if not accepted else None,
            "error_class": (error if error in ERRORS else "OTHER") if not accepted else None,
            "source": source, "target": target, "changes": changes,
            "bond_layer_deltas": {key: layers_b.get(key, 0) - layers_a.get(key, 0)
                                  for key in sorted(layers_a.keys() | layers_b.keys())
                                  if layers_b.get(key, 0) != layers_a.get(key, 0)}}
    if schema == EXECUTION_SCHEMA:
        report["intent"] = deepcopy(action)
        report["replaced"] = description.replaced
    elif schema != "arkguardrl/public-execution-report-v1":
        raise ValueError("Unknown execution receipt version")
    return report


class PublicProgress:
    """One ordered public stream; track exact counters only across observed boundaries.

    values with known=False are observed lower bounds, never silently exact totals.
    Per-piece exposure is an observed event count, not an engine trigger counter.
    """

    def __init__(self, *, execution_schema=EXECUTION_SCHEMA):
        self.execution_schema = execution_schema
        self.state = None
        self.stream_id = uuid.uuid4().hex
        self.spent = self.refreshes = self.direct_gains = 0
        self.spent_known = self.refreshes_known = self.gains_complete = False
        self.exposures, self.exposure_complete = {}, {}
        self.last_execution = None
        self.sequence = 0

    def observe(self, state, *, action=None, reply=None):
        """Untrusted caller boundary: validate and own an isolated snapshot."""
        validate_state(state)
        return self._observe_validated(deepcopy(state), action=action, reply=reply)

    def _observe_validated(self, state, *, action=None, reply=None):
        """Internal transport-owned snapshots are immutable by ownership convention."""
        before = self.state
        receipt = None
        if action is not None:
            if before is None or reply is None:
                raise ValueError("Execution observation needs a preceding state and native reply")
            receipt = execution_report(before, action, state, reply, schema=self.execution_schema)
            receipt.update(stream_id=self.stream_id, sequence=self.sequence + 1)
        elif reply is not None:
            raise ValueError("Reply without an executed intent")
        gold, refreshes = counter(state, "gold"), counter(state, "refreshes")
        changed_round = before is None or state["round"] != before["round"]
        if before is not None:
            if state["revision"] <= before["revision"]:
                raise ValueError("Public progress requires strictly ordered native observations")
            dg, dr = gold - counter(before, "gold"), refreshes - counter(before, "refreshes")
            if dg < 0 or dr < 0:
                raise ValueError("Public cumulative counters regressed")
            if state["round"] < before["round"]:
                raise ValueError("Cross-episode observation requires a new PublicProgress")
        else:
            dg = dr = 0
        if changed_round:
            self.spent = self.refreshes = self.direct_gains = 0
            # A zero cumulative total proves zero; otherwise a quiet, adjacent
            # round boundary proves there was no ambiguous cross-boundary increment.
            adjacent = before is not None and state["round"] == before["round"] + 1
            self.spent_known = gold == 0 or bool(adjacent and dg == 0)
            self.refreshes_known = refreshes == 0 or bool(adjacent and dr == 0)
            self.gains_complete = bool(adjacent)  # Direct command gains only; not total acquiredChess.
            self.exposures, self.exposure_complete = {}, {}
        else:
            self.spent += dg
            self.refreshes += dr
        current = owned(state)
        for uid, (area, piece) in current.items():
            if piece["kind"] != "chess":
                continue
            if uid not in self.exposures:
                self.exposures[uid] = 0
                # Complete for the recorded visible-pre-refresh statistic only.
                self.exposure_complete[uid] = (self.refreshes_known if changed_round else before is not None)
        if receipt is not None:
            self.last_execution = receipt
            if receipt["accepted"] and not changed_round:
                if receipt["action_type"] == "refresh":
                    for uid, (area, piece) in owned(before).items():
                        if piece["kind"] == "chess" and area in ("board", "hand"):
                            self.exposures.setdefault(uid, 0)
                            self.exposure_complete.setdefault(uid, False)
                            self.exposures[uid] += 1
                if receipt["action_type"] in ("buy", "reward") and (receipt["source"] or {}).get("kind") == "chess":
                    self.direct_gains += 1
        if len(self.exposures) > 4096:
            raise ValueError("Public per-round progress identity capacity exceeded")
        self.state = state
        self.sequence += 1
        return receipt

    def snapshot(self, state):
        observation_id = fingerprint(state)
        if self.state is None or fingerprint(self.state) != observation_id:
            raise ValueError("Progress is not bound to this exact public observation")
        current = owned(state)
        return {"schema": PROGRESS_SCHEMA, "observation_id": observation_id,
                "stream_id": self.stream_id, "sequence": self.sequence, "round": state["round"],
                "spent": self.spent, "spent_known": self.spent_known,
                "refreshes": self.refreshes, "refreshes_known": self.refreshes_known,
                "direct_chess_gains": self.direct_gains, "direct_gains_complete": self.gains_complete,
                "pieces": {str(uid): {"visible_pre_refreshes": self.exposures.get(uid, 0),
                                      "history_complete": self.exposure_complete.get(uid, False)}
                           for uid, (_, piece) in current.items() if piece["kind"] == "chess"},
                "last_execution": deepcopy(self.last_execution)}
