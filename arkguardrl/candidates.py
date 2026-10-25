"""Native public parameter domain, deliberately NOT a complete legal-action mask.

No engine probes, hidden pool reads, temporary actions, or private state copies.
Sources: pinned 0.3.1 shared/protocol.js and server/match/{player,board}.js;
history/routing research originated in retained GPL-3.0 experiments/rl and
experiments/expert_policy. All contracts below are new and incompatible.
"""
from copy import deepcopy
from dataclasses import dataclass, field

from .catalog import UnsupportedPublicState
from .contracts import action_id, canonical, decision_id, fingerprint
from .feedback import PROGRESS_SCHEMA, EXECUTION_SCHEMA
from .features import DIRECTIONS, FEATURE_SCHEMA, PHASES, encode_decision

CANDIDATE_SCHEMA = "arkguardrl/native-public-domain-no-hand-reorder-prep64-v2"
HISTORY_SCHEMA = "arkguardrl/native-context-attempts-feedback-intent-prep64-v4"
PREP_ACTION_LIMIT = 64
MAX_CANDIDATES = 16384
FIELD_CELLS = tuple((row, col) for row in range(9, 13) for col in range(2, 11))


class CandidateBlocked(RuntimeError):
    """No candidate remains; this is not a natural termination or a legal wait."""


def _int(value, name, minimum=0):
    if type(value) is not int or value < minimum:
        raise UnsupportedPublicState(f"Invalid public {name}")
    return value


def public_context(state, catalog):
    catalog.validate_state(state)
    if state["phase"] not in PHASES:
        raise UnsupportedPublicState("Unknown native phase")
    own = state["own"]
    locations, occupied = {}, {}
    for area in ("board", "hand", "temp"):
        pieces = own.get(area)
        if not isinstance(pieces, list) or (area == "hand" and len(pieces) != 10):
            raise UnsupportedPublicState("Malformed public piece area")
        for index, piece in enumerate(pieces):
            if piece is None:
                if area == "board":
                    raise UnsupportedPublicState("Null board occupant")
                continue
            if not isinstance(piece, dict) or piece.get("kind") not in ("chess", "token", "item"):
                raise UnsupportedPublicState("Unknown public piece kind")
            uid = _int(piece.get("uid"), "uid", 1)
            if uid in locations:
                raise UnsupportedPublicState("Duplicate public UID")
            catalog.validate_identity(piece["kind"], piece.get("id"))
            locations[uid] = (area, index, piece)
            if area == "board":
                cell = (piece.get("row"), piece.get("col"))
                if cell not in FIELD_CELLS or cell in occupied or piece.get("dir") not in DIRECTIONS or piece["kind"] == "item":
                    raise UnsupportedPublicState("Malformed public board occupancy")
                occupied[cell] = piece
            items = piece.get("items")
            if not isinstance(items, list):
                raise UnsupportedPublicState("Missing equipment list")
            for item in items:
                iid = _int(item.get("uid"), "equipped uid", 1)
                if iid in locations:
                    raise UnsupportedPublicState("Duplicate equipment UID")
                catalog.validate_identity("item", item.get("id"))
                locations[iid] = ("equipped", -1, {"kind": "item", **item})
            if piece["kind"] == "token":
                _int(piece.get("ownerUid"), "token owner", 1)
                _int(piece.get("count"), "token count", 1)
    deployment = own.get("deployment")
    if not isinstance(deployment, dict) or deployment.get("field") not in ("normal", "bossL", "bossR") or not isinstance(deployment.get("cells"), list):
        raise UnsupportedPublicState("Missing visible deployment map")
    cells = {}
    for cell in deployment["cells"]:
        key = (cell.get("row"), cell.get("col"))
        if key not in FIELD_CELLS or key in cells or cell.get("kind") not in ("melee", "ranged"):
            raise UnsupportedPublicState("Unknown/duplicate deployment cell")
        cells[key] = cell["kind"]
    for key in ("ready", "canReady", "alive"):
        if type(own.get(key)) is not bool:
            raise UnsupportedPublicState("Missing explicit public player flag")
    for key in ("funds", "deployCap", "deployCount"):
        _int(own.get(key), key)
    if sum(p["kind"] == "chess" for p in own["board"]) != own["deployCount"]:
        raise UnsupportedPublicState("Public chess capacity disagrees with occupancy")
    for name in ("slots",):
        if not isinstance(own["shop"].get(name), list):
            raise UnsupportedPublicState("Missing public shop slots")
    if len(own["shop"]["slots"]) > 16:
        raise UnsupportedPublicState("Shop exceeds native protocol domain")
    for offer in [*own["shop"]["slots"], *((own["shop"].get("rewardOffer") or {}).get("slots", []))]:
        if offer is None:
            continue
        if offer.get("kind") not in ("chess", "item") or type(offer.get("sold")) is not bool:
            raise UnsupportedPublicState("Unknown shop kind or availability")
        catalog.validate_identity(offer["kind"], offer.get("id"))
        _int(offer.get("price"), "effective price")
    return locations, occupied, cells


def terrain_allows(piece, tile_kind, catalog):
    position = catalog.placement_class(piece)
    return position == "all" or (position == "melee" and tile_kind == "melee") or (position == "high" and tile_kind == "ranged")


def empty_deployment_allowed(piece, cell, state, catalog, context=None):
    """Conservative public empty-cell witness, not general move legality."""
    locations, occupied, cells = context or public_context(state, catalog)
    if cell not in cells or cell in occupied or not terrain_allows(piece, cells[cell], catalog):
        return False
    if piece["kind"] == "chess":
        return state["own"]["deployCount"] < state["own"]["deployCap"]
    if piece["kind"] != "token":
        return False
    owner = locations.get(piece["ownerUid"])
    if not owner or owner[0] != "board" or owner[2]["kind"] != "chess":
        return False
    owner = owner[2]
    rec = catalog.record("token", piece["id"])
    adjacent = piece["id"] == "token_10032_jesca2_jckshd"
    if adjacent:
        return abs(cell[0] - owner["row"]) + abs(cell[1] - owner["col"]) == 1
    if rec.get("ownerRange") is not True and rec.get("ownerRangeOutside") is not True:
        return True
    grid = catalog.attack_range(owner)
    def rotate(row, col):
        return {"RIGHT": (row, col), "UP": (col, -row), "LEFT": (-row, -col), "DOWN": (-col, row)}[owner["dir"]]
    absolute = {(owner["row"] + rotate(r, c)[0], owner["col"] + rotate(r, c)[1]) for r, c in grid}
    return (not rec.get("ownerRange") or cell in absolute) and (not rec.get("ownerRangeOutside") or cell not in absolute)


class DecisionHistory:
    schema = HISTORY_SCHEMA

    def __init__(self):
        self.context, self.attempts, self.prep_used = None, {}, 0
        self.progress = None

    def initialize(self, result):
        if self.progress is not None or result.accepted is not None or (result.progress or {}).get("last_execution") is not None:
            raise ValueError("Initialize history exactly once from a real reset result")
        self._progress(result.observation, result.progress)

    def _progress(self, state, progress):
        if (not isinstance(progress, dict) or progress.get("schema") != PROGRESS_SCHEMA
                or progress.get("observation_id") != fingerprint(state)):
            raise ValueError("Exact public progress stream required; legacy history is incompatible")
        if (not isinstance(progress.get("stream_id"), str) or len(progress["stream_id"]) != 32
                or type(progress.get("sequence")) is not int or progress["sequence"] < 1):
            raise ValueError("Public progress stream identity/order is missing")
        if self.progress is not None and (progress["stream_id"] != self.progress["stream_id"]
                                          or progress["sequence"] <= self.progress["sequence"]):
            raise ValueError("Public progress crossed streams or repeated an observation")
        self.progress = deepcopy(progress)

    def _sync(self, state):
        context = (state["round"], state["phase"])
        if context != self.context:
            self.context, self.attempts, self.prep_used = context, {}, 0
        return decision_id(state)

    def blocked(self, state):
        key = self._sync(state)
        return frozenset(self.attempts.get(key, set()))

    def record(self, state, action, *, result):
        """Call exactly once AFTER a real attempted step, accepted or rejected."""
        report = result.execution
        if (not isinstance(report, dict) or report.get("schema") != EXECUTION_SCHEMA
                or type(result.accepted) is not bool or report.get("accepted") != result.accepted
                or report.get("action_id") != action_id(action) or result.action_id != action_id(action)
                or report.get("intent") != action
                or report.get("before_id") != fingerprint(state)
                or not isinstance(result.progress, dict)
                or result.progress.get("last_execution") != report):
            raise ValueError("History requires the bound real execution receipt")
        if self.progress is None or self.progress["observation_id"] != fingerprint(state):
            raise ValueError("History execution stream is disconnected")
        if (report.get("stream_id") != self.progress["stream_id"]
                or not self.progress["sequence"] < report.get("sequence", 0) <= result.progress["sequence"]):
            raise ValueError("Execution receipt does not continue the current public stream")
        self._progress(result.observation, result.progress)
        key = self._sync(state)
        self.attempts.setdefault(key, set()).add(action_id(action))
        if state["phase"] == "PREP" and action["t"] != "g.choice":
            self.prep_used += 1

    def snapshot(self, state):
        if self.progress is None or self.progress["observation_id"] != fingerprint(state):
            raise ValueError("Initialize/update history with the real environment result before encoding")
        key = self._sync(state)
        return {"schema": self.schema, "progress": deepcopy(self.progress), "used": self.prep_used, "limit": PREP_ACTION_LIMIT,
                "blocked_count": len(self.attempts.get(key, ()))}


@dataclass(frozen=True)
class Decision:
    state: dict
    observation_features: tuple
    candidate_features: tuple
    actions: tuple
    schedule_ids: tuple
    expert_ids: tuple
    terminal: bool
    candidate_schema: str = CANDIDATE_SCHEMA
    feature_schema: str = FEATURE_SCHEMA
    history_schema: str = HISTORY_SCHEMA
    history_json: bytes | None = field(default=None, repr=False, compare=False)


def build_decision(state, catalog, history):
    locations, occupied, cells = public_context(state, catalog)
    own, public = state["own"], state["public"]
    if not isinstance(history, DecisionHistory):
        raise ValueError("Explicit native DecisionHistory required")
    blocked, actions, seen = history.blocked(state), [], set()
    sorted_cells = tuple(sorted(cells.items()))
    allowed_terrain = {}
    def add(action):
        key = action_id(action)
        if key in blocked or key in seen:
            return
        seen.add(key)
        actions.append(action)
        if len(actions) > MAX_CANDIDATES:
            raise CandidateBlocked("Native candidate hard cap exceeded; domain was not sliced")
    if not state["personalEnded"]:
        if not state["decisionRequired"]:
            raise CandidateBlocked("No player decision; advance the environment, do not invent wait")
        phase = state["phase"]
        if phase == "INFO_CHECK":
            add({"t": "g.infoReady", "setupRevision": _int(public.get("setupRevision"), "setup revision")})
        elif phase == "BAND_DRAFT":
            draft = public.get("draft") or {}
            if draft.get("turn") != own["playerId"]:
                raise CandidateBlocked("Native solo draft is not this player's turn")
            for band_id in catalog.band_ids:
                if band_id not in draft.get("picks", {}).values():
                    add({"t": "g.band", "bandId": band_id})
        elif phase == "SP_DRAFT" or (phase == "PREP" and own.get("personalChoice")):
            choice = own.get("personalChoice") if phase == "PREP" else public.get("sp")
            if not isinstance(choice, dict) or not isinstance(choice.get("cards"), list) or not 1 <= len(choice["cards"]) <= 6:
                raise UnsupportedPublicState("Unknown mandatory choice domain")
            if phase == "SP_DRAFT" and choice.get("turn") != own["playerId"]:
                raise CandidateBlocked("Native solo SP draft is not this player's turn")
            for index, card in enumerate(choice["cards"]):
                if card.get("kind") not in ("item", "bounty", "tactic"):
                    raise UnsupportedPublicState("Unknown public choice card kind")
                if phase == "SP_DRAFT" and (card.get("idx") != index or type(card.get("soldOut")) is not bool):
                    raise UnsupportedPublicState("Unknown SP card position/stock availability")
                if phase == "SP_DRAFT" and (str(index) in choice.get("taken", {}) or card["soldOut"]):
                    continue
                intent = {"t": "g.choice", "idx": index}
                if phase == "PREP":
                    if not isinstance(choice.get("id"), str) or choice.get("round") != state["round"]:
                        raise UnsupportedPublicState("Unknown personal choice identity")
                    intent["choiceId"] = choice["id"]
                add(intent)
        elif phase == "PREP":
            if own["ready"] or not own["alive"]:
                raise CandidateBlocked("No active prep decision")
            shop = own["shop"]
            recovery = history.prep_used >= PREP_ACTION_LIMIT
            temp_uids = {p["uid"] for p in own["temp"] if p}
            hand_free = any(p is None for p in own["hand"])
            if own["canReady"]:
                if temp_uids:
                    raise UnsupportedPublicState("canReady disagrees with public temporary reserve")
                add({"t": "g.ready", "ready": True})
            if not recovery:
                if hand_free:
                    for index, slot in enumerate(shop["slots"]):
                        if slot and not slot["sold"] and slot["price"] <= own["funds"]:
                            add({"t": "g.buy", "slot": index})
                    reward = shop.get("rewardOffer")
                    if reward:
                        if len(reward["slots"]) > 6:
                            raise UnsupportedPublicState("Reward exceeds native protocol domain")
                        for index, slot in enumerate(reward["slots"]):
                            if slot and not slot["sold"] and slot["price"] <= own["funds"]:
                                add({"t": "g.reward", "idx": index})
                if own["funds"] >= shop["refreshPrice"]:
                    add({"t": "g.refresh"})
                add({"t": "g.freeze"})
                if shop["level"] < shop["maxLevel"] and own["funds"] >= shop["upgradePrice"]:
                    add({"t": "g.levelUp"})
            if not recovery or not own["canReady"]:
                for uid, (area, index, piece) in locations.items():
                    if area == "equipped":
                        continue
                    if piece["kind"] == "chess":
                        # Recovery may need space or removal of a token's carrier.
                        add({"t": "g.sell", "uid": uid})
                    elif piece["kind"] == "item" and (not recovery or uid in temp_uids):
                        add({"t": "g.destroy", "uid": uid})
                    if recovery and uid not in temp_uids:
                        continue
                    # Reserve slot rearrangement is outside the policy domain.
                    # Keep board withdrawals and temp-to-hand overflow recovery.
                    if area != "hand":
                        for target_index in range(len(own["hand"])):
                            if recovery and own["hand"][target_index] is not None:
                                continue
                            add({"t": "g.move", "uid": uid, "to": {"area": "hand", "idx": target_index}})
                    if piece["kind"] in ("chess", "token"):
                        for (row, col), tile_kind in sorted_cells:
                            terrain_key = (piece['kind'], piece['id'], tile_kind)
                            if terrain_key not in allowed_terrain:
                                allowed_terrain[terrain_key] = terrain_allows(piece, tile_kind, catalog)
                            if not allowed_terrain[terrain_key]:
                                continue
                            occupant = occupied.get((row, col))
                            if area != "board":
                                if piece["kind"] == "token":
                                    if not empty_deployment_allowed(piece, (row, col), state, catalog, (locations, occupied, cells)):
                                        continue
                                elif (not occupant or occupant["kind"] != "chess") and own["deployCount"] >= own["deployCap"]:
                                    continue
                            for direction in DIRECTIONS:
                                if area == "board" and (row, col, direction) == (piece["row"], piece["col"], piece["dir"]):
                                    continue
                                add({"t": "g.move", "uid": uid, "to": {"area": "board", "row": row, "col": col}, "dir": direction})
                    elif piece["kind"] == "item":
                        rec = catalog.record("item", piece["id"])
                        if rec.get("itemType") == "EQUIP":
                            for target_uid, (target_area, _, target) in locations.items():
                                if target_area != "equipped" and target["kind"] == "chess":
                                    intent = {"t": "g.equip", "itemUid": uid, "targetUid": target_uid}
                                    add(intent)
                                    if len(target["items"]) >= 2:
                                        for old in target["items"]:
                                            add({**intent, "replaceUid": old["uid"]})
                        elif rec.get("itemType") == "MAGIC":
                            # Arts target the WHOLE protocol field, including nondeployable tiles.
                            for row, col in FIELD_CELLS:
                                for direction in DIRECTIONS:
                                    add({"t": "g.art", "itemUid": uid, "row": row, "col": col, "dir": direction})
                        else:
                            raise UnsupportedPublicState("Unknown item capability")
        else:
            raise CandidateBlocked("Unsupported actionable native phase")
        if not actions:
            raise CandidateBlocked("History/public domain exhausted; no fallback wait or forced Ready")
    snapshot = history.snapshot(state)
    observation, features, schedules, experts = encode_decision(state, actions, catalog, snapshot)
    # Retain the exact history used for the feature tensors, independently of
    # subsequent history advancement. Bytes cannot alias mutable progress.
    return Decision(deepcopy(state), observation, features, tuple(actions), schedules, experts,
                    state["personalEnded"], history_json=canonical(snapshot))
