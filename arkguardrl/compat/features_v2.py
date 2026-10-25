"""Native public relational features and purchase/placement scheduler routing.

Research provenance: retained experiments/rl/entity_features.py and
experiments/expert_policy/{semantics,rl_api,source_position}.py (GPL-3.0).
This is a new native contract, NOT their 354-dimensional encoder or checkpoints.
Structured signed hashes retain relations, not guaranteed collision-free IDs.
All identities and numerical catalog fields are public; UIDs bind actions only.
"""
from collections import Counter
import hashlib
import math

from ..contracts import canonical

FEATURE_SCHEMA = "arkguardrl/native-public-economy-relations-routing-v2"
SCHEDULES = ("buy", "deploy", "adjust", "ready", "confirm", "band", "bandSkip", "choice", "reward", "wait")
DIRECTIONS = ("RIGHT", "UP", "DOWN", "LEFT")
PHASES = ("INFO_CHECK", "BAND_DRAFT", "BATTLE_CHECK", "ROUND_START", "SP_DRAFT", "PREP",
          "COMBAT", "UNITE", "SETTLE", "FINAL_ASSAULT", "HIDDEN_CORE", "RESULT")
ACTION_TYPES = ("infoReady", "band", "choice", "buy", "reward", "refresh", "freeze",
                "levelUp", "sell", "move", "equip", "art", "destroy", "ready")
SEMANTICS = tuple(t for t in ACTION_TYPES if t != "move") + (
    "deploy", "reserve_replace", "reposition", "board_swap", "withdraw", "reserve_move")
GLOBAL_FIELDS = ("round", "last_round", "boss_round", "funds", "lp", "deploy_count", "deploy_cap",
                 "level", "max_level", "upgrade_price", "refresh_price", "free_refreshes", "board_size",
                 "hand_used", "temp_used", "alive", "ready", "can_ready", "frozen", "merges",
                 "refreshes", "core_layers", "banned_count", "disabled_count")
ACTION_FIELDS = ("price", "source_tier", "source_golden", "source_equipment", "source_row", "source_col",
                 "source_slot", "source_count", "target_tier", "target_golden", "target_equipment",
                 "target_row", "target_col", "target_slot", "to_row", "to_col", "to_hand", "to_slot",
                 "replace", "source_board", "source_hand", "source_temp", "offer_tier", "offer_golden")
# Fixed public vocabulary from the pinned catalog, not a data-dependent tensor size.
BOND_IDS = ("arcaneShip", "deputShip", "egirShip", "emptyShip", "indomShip",
            "investShip", "kazimierzShip", "kjeragShip", "lateranoShip", "maniShip",
            "miraShip", "preciShip", "raidShip", "sargonShip", "siracusaShip",
            "skillfulShip", "soloShip", "steadShip", "suntShip", "swiftShip",
            "victoriaShip", "visiShip", "yanShip")
BOND_FIELDS = ("count", "layers", "tier", "active", "off", "next_threshold_gap")
ACQUISITION_FIELDS = ("board_copies", "hand_copies", "temp_copies", "golden_owned",
                      "upgrade_num", "copies_missing")
OBS_DIM = len(GLOBAL_FIELDS) + len(PHASES) + 128 + len(BOND_IDS) * len(BOND_FIELDS) + 4
ACTION_DIM = (len(ACTION_TYPES) + len(ACTION_FIELDS) + len(DIRECTIONS)
              + len(SEMANTICS) + 64 + len(BOND_IDS) + len(ACQUISITION_FIELDS))
CORE_BONDS = frozenset(("yanShip", "sargonShip", "victoriaShip", "kjeragShip", "lateranoShip",
                        "egirShip", "siracusaShip", "kazimierzShip"))


def _number(value, scale=1):
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError("Features require finite numerical public fields")
    return math.tanh(value / scale)


def _hash_rows(rows, dimension, domain):
    counts = Counter(canonical(row) for row in rows)
    out = [0] * dimension
    for row, count in sorted(counts.items()):
        digest = hashlib.blake2b(row, digest_size=16, person=domain).digest()
        for start in (0, 8):
            index = int.from_bytes(digest[start:start + 4], "little") % dimension
            out[index] += count if digest[start + 4] & 1 else -count
    return tuple(math.copysign(math.log1p(abs(x)), x) if x else 0.0 for x in out)


def _locations(state):
    out = {}
    for area in ("board", "hand", "temp"):
        for index, piece in enumerate(state["own"][area]):
            if piece is not None:
                out[piece["uid"]] = (area, index if area != "board" else -1, piece)
                for item in piece.get("items", []):
                    out[item["uid"]] = ("equipped", -1, {"kind": "item", **item})
    return out


def _piece(piece, area, slot, locations):
    keys = ("id", "kind", "tier", "golden", "row", "col", "dir", "count")
    row = {key: piece[key] for key in keys if key in piece}
    # Equipment order matters: an unspecified replacement removes the oldest.
    row.update(area=area, slot=slot, items=[item["id"] for item in piece.get("items", [])])
    owner = locations.get(piece.get("ownerUid"))
    if owner is not None:
        row["owner"] = {key: owner[2][key] for key in ("id", "row", "col", "dir") if key in owner[2]}
    return row


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


def routing(state, action):
    kind, semantic = action["t"][2:], classify(state, action)
    schedule = "confirm" if kind == "infoReady" else kind if kind in SCHEDULES else "deploy" if semantic == "deploy" else "adjust"
    expert = int(kind not in ("buy", "refresh", "freeze", "levelUp", "band", "choice", "reward"))
    return SCHEDULES.index(schedule), expert


def _identity(row):
    """Small public identity projection; never hash prose or transport UIDs."""
    return {key: row[key] for key in ("id", "kind", "tier", "golden", "price", "type",
                                     "itemId", "chessId", "bandId", "coin", "count", "rounds",
                                     "payout", "enemyKey", "soldOut") if key in row}


def _bonds(own, catalog):
    rules = catalog.bond_rules()
    rows = {row["bondId"]: row for row in own["bonds"]}
    if set(rules) != set(BOND_IDS) or set(rows) - set(BOND_IDS):
        raise ValueError("Public bond vocabulary differs from feature contract")
    out = []
    for key in BOND_IDS:
        row = rows.get(key, {})
        count = row.get("count", 0)
        higher = [n for n in rules[key]["thresholds"] if n > count]
        out.extend((_number(count, 12), _number(row.get("layers", 0), 100),
                    _number(row.get("tier", 0), 3), float(row.get("active", False)),
                    float(row.get("off", False)), _number(min(higher) - count if higher else 0, 12)))
    return tuple(out)


def _acquisition(subject, catalog, locations):
    if subject.get("kind") != "chess":
        return (0.0,) * (len(BOND_IDS) + len(ACQUISITION_FIELDS))
    record = catalog.record("chess", subject["id"])
    bonds = record.get("bonds", [])
    if set(bonds) - set(BOND_IDS):
        raise ValueError("Unknown public card bond")
    # Counts refer to physical normal cards in the same base family, not predicted merges.
    base = record.get("baseId", subject["id"])
    counts, golden = Counter(), 0
    for area, _, piece in locations.values():
        if piece.get("kind") != "chess":
            continue
        rec = catalog.record("chess", piece["id"])
        if rec.get("baseId", piece["id"]) != base:
            continue
        if rec.get("isGolden", False):
            golden += 1
        else:
            counts[area] += 1
    needed = record.get("upgradeNum") or 0
    nums = (counts["board"] / 3, counts["hand"] / 3, counts["temp"] / 3,
            golden / 3, needed / 3, max(0, needed - sum(counts.values())) / 3)
    return tuple(float(key in bonds) for key in BOND_IDS) + tuple(_number(x) for x in nums)


def encode_decision(state, actions, catalog, history_snapshot):
    own, public = state["own"], state["public"]
    shop, locations = own["shop"], _locations(state)
    occupied = {(p["row"], p["col"]): p for p in own["board"]}
    descriptors = {uid: _piece(piece, area, slot, locations) for uid, (area, slot, piece) in locations.items()}
    core = max([0] + [b["layers"] for b in own["bonds"] if b.get("active") and not b.get("off") and b.get("bondId") in CORE_BONDS])
    globals_ = (state["round"] / 20, public["lastRound"] / 20, public["bossRound"] / 20,
                own["funds"] / 30, own["lp"] / 30, own["deployCount"] / 12, own["deployCap"] / 12,
                shop["level"] / 6, shop["maxLevel"] / 6, shop["upgradePrice"] / 30,
                shop["refreshPrice"] / 10, shop["freeRefreshes"] / 10, len(own["board"]) / 36,
                sum(p is not None for p in own["hand"]) / 10, sum(p is not None for p in own["temp"]) / 5,
                int(own["alive"]), int(own["ready"]), int(own["canReady"]), int(shop["frozen"]),
                own["stats"]["merges"] / 10, own["stats"]["refreshes"] / 10, core / 100,
                len(public["bannedChess"]) / 100, len(public["disabledBonds"]) / 30)
    relations = []
    for area in ("board", "hand", "temp"):
        for index, piece in enumerate(own[area]):
            if piece is None:
                continue
            value = _piece(piece, area, index if area != "board" else -1, locations)
            relations.extend((["entity", value], ["identity-area", piece["id"], area],
                              ["pose", piece["id"], {key: value[key] for key in ("row", "col", "dir", "slot") if key in value}]))
    # Keep local structured identities and geometry; discard whole-list/text blobs.
    for index, offer in enumerate(shop["slots"]):
        if offer:
            relations.append(["shop-slot", index, _identity(offer)])
    relations.append(["band", own["bandId"]])
    relations.append(["field", own["deployment"].get("field")])
    for enemy in own["nextEnemies"]:
        relations.append(["next-enemy", {key: enemy[key] for key in
                          ("enemyKey", "count", "fly", "elite", "boss", "gate", "t") if key in enemy}])
    for effect in own["effects"]:
        relations.append(["effect", {key: effect[key] for key in
                         ("iconId", "iconKind", "counter") if key in effect}])
    for name in ("stageId", "bossId", "factions", "bannedChess", "disabledBonds"):
        relations.append(["public", name, public.get(name)])
    choice = own.get("personalChoice")
    # Choice details belong to their candidate, not one opaque array hash.
    used, limit = history_snapshot["used"], history_snapshot["limit"]
    observation = tuple(_number(x) for x in globals_) + tuple(float(state["phase"] == phase) for phase in PHASES) + _hash_rows(relations, 128, b"agrl-obs031-v2") + _bonds(own, catalog) + (
        min(1.0, used / limit), float(used >= limit), _number(history_snapshot["blocked_count"], 64), float(state["decisionRequired"]))
    encoded, schedules, experts = [], [], []
    acquisition_cache = {}
    for action in actions:
        kind = action["t"][2:]
        source_loc = locations.get(action.get("uid", action.get("itemUid")), ("", -1, {}))
        source = source_loc[2]
        to = action.get("to", {})
        target_loc = locations.get(action.get("targetUid"), ("", -1, {}))
        if kind == "move" and to.get("area") == "board":
            target_loc = next((loc for loc in locations.values() if loc[0] == "board" and (loc[2]["row"], loc[2]["col"]) == (to["row"], to["col"])), target_loc)
        target = target_loc[2]
        offer = {}
        if kind in ("buy", "reward"):
            slots = shop["slots"] if kind == "buy" else shop["rewardOffer"]["slots"]
            offer = slots[action["slot"] if kind == "buy" else action["idx"]]
        offer_rec = catalog.record(offer["kind"], offer["id"]) if offer else {}
        nums = (offer.get("price", 0) / 30, (source.get("tier") or 0) / 6, int(source.get("golden", False)), len(source.get("items", [])) / 2,
                source.get("row", -1) / 19, source.get("col", -1) / 21, source_loc[1] / 10, source.get("count", 0) / 10,
                (target.get("tier") or 0) / 6, int(target.get("golden", False)), len(target.get("items", [])) / 2,
                target.get("row", -1) / 19, target.get("col", -1) / 21, target_loc[1] / 10,
                to.get("row", action.get("row", -1)) / 19, to.get("col", action.get("col", -1)) / 21,
                int(to.get("area") == "hand"), to.get("idx", -1) / 10, int("replaceUid" in action),
                int(source_loc[0] == "board"), int(source_loc[0] == "hand"), int(source_loc[0] == "temp"),
                (offer_rec.get("tier") or 0) / 6, int(offer_rec.get("isGolden", False)))
        semantic = classify(state, action, locations, occupied)
        action_public = {key: value for key, value in action.items() if key not in ("uid", "itemUid", "targetUid", "replaceUid", "choiceId")}
        bound = [["action", action_public], ["source", descriptors[source["uid"]] if source else None],
                 ["target", descriptors[target["uid"]] if target else None], ["offer", _identity(offer)],
                 ["replaced", locations.get(action.get("replaceUid"), (None, None, {}))[2].get("id")]]
        if kind == "choice":
            cards = (choice or public["sp"])["cards"]
            bound.append(["choice", _identity(cards[action["idx"]])])
        subject = offer or source
        subject_key = (subject.get("kind"), subject.get("id"))
        if subject_key not in acquisition_cache:
            acquisition_cache[subject_key] = _acquisition(subject, catalog, locations)
        encoded.append(tuple(float(kind == t) for t in ACTION_TYPES) + tuple(_number(x) for x in nums)
                       + tuple(float(action.get("dir") == d) for d in DIRECTIONS)
                       + tuple(float(semantic == label) for label in SEMANTICS)
                       + _hash_rows(bound, 64, b"agrl-act031-v2")
                       + acquisition_cache[subject_key])
        schedule = SCHEDULES.index("confirm" if kind == "infoReady" else kind if kind in SCHEDULES else "deploy" if semantic == "deploy" else "adjust")
        expert = int(kind not in ("buy", "refresh", "freeze", "levelUp", "band", "choice", "reward"))
        schedules.append(schedule)
        experts.append(expert)
    if len(observation) != OBS_DIM or any(len(row) != ACTION_DIM for row in encoded):
        raise AssertionError("Native feature dimension changed")
    return observation, tuple(encoded), tuple(schedules), tuple(experts)


def encode_observation(state, catalog, history):
    """Value bootstrap at a public nondecision state, without inventing an action."""
    catalog.validate_state(state)
    snapshot = history.snapshot(state) if hasattr(history, "snapshot") else history
    return encode_decision(state, (), catalog, snapshot)[0]
