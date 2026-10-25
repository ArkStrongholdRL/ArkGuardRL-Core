"""Current public vocabulary and pure action classification."""
DIRECTIONS = ("RIGHT", "UP", "DOWN", "LEFT")
PHASES = ("INFO_CHECK", "BAND_DRAFT", "BATTLE_CHECK", "ROUND_START", "SP_DRAFT", "PREP",
          "COMBAT", "UNITE", "SETTLE", "FINAL_ASSAULT", "HIDDEN_CORE", "RESULT")
ACTION_TYPES = ("infoReady", "band", "choice", "buy", "reward", "refresh", "freeze",
                "levelUp", "sell", "move", "equip", "art", "destroy", "ready")
SEMANTICS = tuple(t for t in ACTION_TYPES if t != "move") + (
    "deploy", "reserve_replace", "reposition", "board_swap", "withdraw", "reserve_move")
BOND_IDS = ("arcaneShip", "deputShip", "egirShip", "emptyShip", "indomShip",
            "investShip", "kazimierzShip", "kjeragShip", "lateranoShip", "maniShip",
            "miraShip", "preciShip", "raidShip", "sargonShip", "siracusaShip",
            "skillfulShip", "soloShip", "steadShip", "suntShip", "swiftShip",
            "victoriaShip", "visiShip", "yanShip")

def _locations(state):
    out = {}
    for area in ("board", "hand", "temp"):
        for index, piece in enumerate(state["own"][area]):
            if piece is not None:
                out[piece["uid"]] = (area, index if area != "board" else -1, piece)
                for item in piece.get("items", []):
                    out[item["uid"]] = ("equipped", -1, {"kind": "item", **item})
    return out


def classify(state, action, locations=None, occupied=None):
    kind = action["t"][2:]
    if kind != "move":
        if kind not in ACTION_TYPES:
            raise ValueError("Unknown native action feature identity")
        return kind
    locations = _locations(state) if locations is None else locations
    area, _, piece = locations[action["uid"]]
    target = action["to"]
    if target["area"] == "hand":
        return "withdraw" if area == "board" else "reserve_move"
    occupied = {(p["row"], p["col"]): p for p in state["own"]["board"]} if occupied is None else occupied
    occupant = occupied.get((target["row"], target["col"]))
    if area == "board":
        return "board_swap" if occupant and occupant["uid"] != piece["uid"] else "reposition"
    return "reserve_replace" if occupant else "deploy"

