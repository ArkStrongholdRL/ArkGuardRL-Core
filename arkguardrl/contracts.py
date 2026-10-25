"""Identities for native actions and their public decision context; no engine queries."""
import hashlib
import json

ACTION_SCHEMA = "arkguardrl/native-intent-v1"
CONTEXT_SCHEMA = "arkguardrl/decision-context-v1"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def fingerprint(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def action_id(action):
    if not isinstance(action, dict) or not isinstance(action.get("t"), str):
        raise ValueError("A native intent must contain a string t")
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
