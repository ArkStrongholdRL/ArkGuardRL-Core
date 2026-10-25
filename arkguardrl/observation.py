"""Structural checks for the explicitly versioned native solo observation."""
from .contracts import canonical

SCHEMA = "arkguardrl/native-session-v2"


def validate_state(state):
    """Validate the live mapping and return its canonical bytes for this call."""
    if not isinstance(state, dict) or state.get("schema") != SCHEMA:
        raise ValueError("Unsupported native observation schema")
    for name in ("gameEnded", "personalEnded", "decisionRequired"):
        if type(state.get(name)) is not bool:
            raise ValueError(f"Missing boolean {name}")
    if type(state.get("revision")) is not int or state["revision"] < 0:
        raise ValueError("Invalid native revision")
    public, own = state["public"], state["own"]
    if state["phase"] != public["phase"] or state["round"] != public["round"]:
        raise ValueError("Inconsistent native phase/round")
    players = public["players"]
    if len(players) != 1 or players[0]["playerId"] != own["playerId"] or own["seat"] != 0:
        raise ValueError("Expected native solo seat")
    if type(own.get("alive")) is not bool:
        raise ValueError("Missing personal alive flag")
    if state["gameEnded"] and not state["personalEnded"]:
        raise ValueError("Game ended without personal endpoint")
    if not own["alive"] and not state["personalEnded"]:
        raise ValueError("Personal defeat not marked")
    if state["personalEnded"] and state["decisionRequired"]:
        raise ValueError("Decision after personal endpoint")
    if not state["gameEnded"] and state.get("outcome") is not None:
        raise ValueError("Final outcome before game end")
    stack = [state]
    while stack:
        value = stack.pop()
        if isinstance(value, list):
            stack.extend(value)
        elif isinstance(value, dict):
            if any("seed" in k.lower() or "rng" in k.lower() or "hiddenbossid" in k.lower() for k in value):
                raise ValueError("Forbidden hidden identity in observation")
            stack.extend(value.values())
    return canonical(state)
