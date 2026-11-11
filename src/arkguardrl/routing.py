"""Intent-first scheduling, independent of the number of parameter candidates."""
from .domain import ACTION_TYPES, BOND_IDS, DIRECTIONS, PHASES, classify

ROUTING_SCHEMA = "arkguardrl/native-economic-intents/v2"
SCHEDULES = ("buy", "levelUp", "refresh", "freeze", "sell", "deploy", "replace",
             "adjust", "withdraw", "equip", "art", "destroy", "ready", "confirm",
             "band", "bandSkip", "choice", "reward", "wait")


def routing(state, action, *, description=None):
    if description is None:
        from .actions import describe_action
        description = describe_action(state, action)
    kind, semantic = description.kind, description.semantic
    if kind == "move":
        schedule = {"deploy": "deploy", "reserve_replace": "replace", "withdraw": "withdraw",
                    "reposition": "adjust", "board_swap": "adjust", "reserve_move": "adjust"}[semantic]
    else:
        schedule = "confirm" if kind == "infoReady" else kind
    if schedule not in SCHEDULES:
        raise ValueError("Unknown explicit native decision intent")
    expert = int(kind not in ("buy", "refresh", "freeze", "levelUp", "band", "choice", "reward"))
    return SCHEDULES.index(schedule), expert
