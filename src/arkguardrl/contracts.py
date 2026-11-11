"""Public transition records and identities for actions and decision contexts."""
import hashlib
import json
from functools import lru_cache
from dataclasses import dataclass

ACTION_SCHEMA = "arkguardrl/native-intent-v1"
CONTEXT_SCHEMA = "arkguardrl/decision-context-v1"


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


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def fingerprint(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def _intent_key(action, depth=0):
    """Snapshot small native dictionaries by value, never by object identity.

    Tag scalar types so True/1 and None/missing cannot alias. Other JSON values
    use the original serializer, including its float/nonfinite validation.
    """
    if depth > 2 or len(action) > 16 or any(type(k) is not str or len(k) > 256 for k in action):
        return None
    fields = []
    for k, value in sorted(action.items()):
        kind = type(value)
        if kind is dict:
            value = _intent_key(value, depth+1)
            if value is None:
                return None
        elif kind not in (str, int, bool, type(None)) or (kind is str and len(value) > 256):
            return None
        fields.append((k, kind, value))
    return tuple(fields)


def _intent_from_key(key):
    if key and key[0] == 'board_move':
        _, uid, row, col, direction = key
        return {'t': 'g.move', 'uid': uid, 'to': {'area': 'board', 'row': row, 'col': col}, 'dir': direction}
    if key and key[0] == 'hand_move':
        return {'t': 'g.move', 'uid': key[1], 'to': {'area': 'hand', 'idx': key[2]}}
    return {k: _intent_from_key(value) if kind is dict else value for k, kind, value in key}


@lru_cache(maxsize=16384)
def _cached_action_id(schema, key):
    return fingerprint({"schema": schema, "intent": _intent_from_key(key)})


def action_id(action):
    if not isinstance(action, dict) or not isinstance(action.get("t"), str):
        raise ValueError("A native intent must contain a string t")
    # Most candidates are placements. Exact key counts plus exact scalar types
    # prove this compact key covers every field; extensions use the generic path.
    if type(action) is dict and type(action['t']) is str and action['t'] == 'g.move':
        uid, to = action.get('uid'), action.get('to')
        if type(uid) is int and type(to) is dict and type(to.get('area')) is str:
            if len(action) == 4 and len(to) == 3 and to.get('area') == 'board':
                row, col, direction = to.get('row'), to.get('col'), action.get('dir')
                if type(row) is int and type(col) is int and type(direction) is str and len(direction) <= 256:
                    return _cached_action_id(ACTION_SCHEMA, ('board_move', uid, row, col, direction))
            elif len(action) == 3 and len(to) == 2 and to.get('area') == 'hand':
                index = to.get('idx')
                if type(index) is int:
                    return _cached_action_id(ACTION_SCHEMA, ('hand_move', uid, index))
    key = _intent_key(action)
    if key is not None:
        return _cached_action_id(ACTION_SCHEMA, key)
    return fingerprint({"schema": ACTION_SCHEMA, "intent": action})


def decision_id(state):
    """Ignore transport revision and virtual timestamps, preserve round/phase and visible setup.

    This key supports repetition diagnostics, not an automatic candidate/history mask.
    """
    public = {k: v for k, v in state["public"].items()
              if k not in ("serverNow", "deadline", "overtimeAt")}
    return fingerprint({"schema": CONTEXT_SCHEMA, "observationSchema": state["schema"],
                        "public": public, "own": state["own"],
                        "personalEnded": state["personalEnded"], "gameEnded": state["gameEnded"]})
