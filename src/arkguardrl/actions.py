"""One public interpretation of a native command for features and receipts."""
from dataclasses import dataclass
from .domain import _locations, classify


def descriptor(row, area):
    if row is None:
        return None
    return {key: row[key] for key in ("kind", "id", "uid", "row", "col", "dir") if key in row} | {"area": area}


@dataclass(frozen=True)
class ActionDescription:
    kind: str
    semantic: str
    source: dict | None
    target: dict | None
    replaced: dict | None
    subject: dict
    destination: dict
    direction: str | None
    cost: float


def describe_action(state, action, *, locations=None, occupied=None, descriptors=None):
    locations = _locations(state) if locations is None else locations
    occupied = ({(p["row"], p["col"]): p for p in state["own"]["board"]}
                if occupied is None else occupied)
    def find(uid):
        if descriptors is not None:
            return descriptors.get(uid)
        value = locations.get(uid)
        return descriptor(value[2], value[0]) if value else None
    kind = action["t"][2:]
    source = find(action.get("uid", action.get("itemUid")))
    target, replaced = find(action.get("targetUid")), find(action.get("replaceUid"))
    shop = state["own"]["shop"]
    subject = {}
    if kind in ("buy", "reward", "choice"):
        if kind == "buy":
            slots, index, area = shop["slots"], action["slot"], "shop"
        elif kind == "reward":
            slots, index, area = (shop.get("rewardOffer") or {}).get("slots", []), action["idx"], "offer"
        else:
            choice = (state["own"].get("personalChoice") if state["phase"] == "PREP" else state["public"].get("sp")) or {}
            slots, index, area = choice.get("cards", []), action["idx"], "offer"
        subject = slots[index] if 0 <= index < len(slots) and slots[index] else {}
        # Receipt descriptors intentionally retain their stable v2 projection.
        source = {"kind": subject["kind"], "id": subject.get("id"), "area": area} if subject else None
    elif kind == "band":
        source = {"kind": "band", "id": action["bandId"], "area": "offer"}
    to = action.get("to", {}) if kind == "move" else {key: action[key] for key in ("row", "col") if key in action}
    if kind == "move" and to.get("area") == "board":
        row = occupied.get((to["row"], to["col"]))
        target = find(row["uid"]) if descriptors is not None and row is not None else descriptor(row, "board")
    elif kind == "move" and to.get("area") == "hand":
        hand, index = state["own"]["hand"], to["idx"]
        row = hand[index] if 0 <= index < len(hand) else None
        target = find(row["uid"]) if descriptors is not None and row is not None else descriptor(row, "hand")
    semantic = classify(state, action, locations, occupied) if kind != "move" or source else "move"
    cost = (shop["upgradePrice"] if kind == "levelUp" else
            (0 if shop["freeRefreshes"] > 0 else shop["refreshPrice"]) if kind == "refresh" else subject.get("price", 0))
    return ActionDescription(kind, semantic, source, target, replaced, subject, to, action.get("dir"), cost)
