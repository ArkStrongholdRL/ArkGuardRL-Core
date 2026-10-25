"""Identities for native actions and their public decision context; no engine queries."""
import hashlib
import json
from functools import lru_cache

ACTION_SCHEMA = "arkguardrl/native-intent-v1"
CONTEXT_SCHEMA = "arkguardrl/decision-context-v1"


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
    return {k: _intent_from_key(value) if kind is dict else value for k, kind, value in key}


@lru_cache(maxsize=16384)
def _cached_action_id(schema, key):
    return fingerprint({"schema": schema, "intent": _intent_from_key(key)})


def action_id(action):
    if not isinstance(action, dict) or not isinstance(action.get("t"), str):
        raise ValueError("A native intent must contain a string t")
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
