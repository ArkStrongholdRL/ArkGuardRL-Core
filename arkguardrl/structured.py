"""Public decision projection and internal encoder table specification.

Packing keeps existing CPU capture/GAE interfaces; the neural encoder unpacks tables
before learning. No relational hashing, engine probes, or private-state reads.
Public garrison semantics researched from pinned GPL-3.0 upstream
server/sim/content/garrisons/meta.js and data/garrisons.json; data stays external.
"""
from collections import Counter
import math

from .feedback import ERRORS, PROGRESS_SCHEMA, PublicProgress
from .contracts import fingerprint
from .semantics import MAX_FACTS, FACT_DIM, SEMANTIC_VOCAB, SEMANTIC_SCHEMA, encode as encode_facts

from .routing import (ACTION_TYPES, BOND_IDS, DIRECTIONS, PHASES, SCHEDULES,
                          classify, routing)

FEATURE_SCHEMA = "arkguardrl/native-dynamic-static-bank-v7"
MAX_ENTITIES, MAX_MECHANISMS, MAX_ENEMIES, MAX_RANGE = 128, 256, 64, 24
ENTITY_VOCAB, MECHANISM_VOCAB, ENEMY_VOCAB = 1024, 512, 2048
AREAS = ("board", "hand", "temp", "equipped", "shop", "offer", "faction", "active_effect")
KINDS = ("chess", "item", "token", "band", "bounty", "tactic", "effect")
CELLS = tuple((r, c) for r in range(9, 13) for c in range(2, 11))
MOVE_TYPES = ("deploy", "reserve_replace", "reposition", "board_swap", "withdraw", "reserve_move")
GLOBAL_FIELDS = ("round", "remaining_rounds", "boss_distance", "lp", "funds", "level",
                 "upgrade_price", "refresh_price", "free_refreshes", "frozen",
                 "deploy_free", "hand_free", "temp_used", "round_spent_observed", "round_spent_exact",
                 "round_refreshes_observed", "round_refreshes_exact", "direct_chess_gains_observed",
                 "direct_chess_history_complete", "max_level", "next_tier", "current_chess_slots", "next_chess_slots",
                 "current_item_slots", "next_item_slots", "funds_carry", "level_opens_empty_slots",
                 "enemy_hp_scale", "enemy_atk_scale", "enemy_speed_scale")
CONTROL_REFERENCES = ("source_entity", "target_entity", "replaced_entity", "source_catalog", "target_catalog", "replaced_catalog")
CHANGE_FIELDS = ("funds_delta", "spent_delta", "refreshes_delta", "merges_delta", "level_delta",
                 "deploy_delta", "hand_free_delta", "temp_used_delta")
INTENT_NUMBERS = ("requested_row", "requested_col", "requested_hand_slot", "requested_shop_slot", "requested_choice_slot",
                  "source_previous_row", "source_previous_col", "target_previous_row", "target_previous_col")
INTENT_FIELDS = (tuple("intent_" + d for d in DIRECTIONS) + ("to_board", "to_hand")
                 + tuple(n + suffix for n in INTENT_NUMBERS for suffix in ("", "_known")))
CONTROL_FIELDS = (("prep_budget_active", "prep_remaining", "last_action_known", "accepted", "same_round")
                  + tuple("last_" + t for t in ACTION_TYPES) + tuple("error_" + t for t in ERRORS) + CHANGE_FIELDS
                  + tuple("layer_delta_" + key for key in BOND_IDS) + INTENT_FIELDS)
CONTROL_DIM = len(CONTROL_REFERENCES) + len(CONTROL_FIELDS)
CONTROL_ENCODED_DIM = 3 * 64 + 3 * 24 + len(CONTROL_FIELDS)
STATS = ("maxHp", "atk", "def", "res", "bat", "aspd", "blockCnt")
STAT_SCALES = (3000, 1000, 500, 100, 3, 100, 3)
ENTITY_NUMBERS = ("tier", "golden", "price", "row", "col", "count", "upgrade_num",
                  "fly_known", "can_hit_fly", "phys", "magic", "heal", "melee", "ranged",
                  "bounty_coin", "bounty_count", "bounty_rounds", "visible_pre_refreshes", "refresh_history_complete", "effect_counter", "effect_counter_known")
# Row: present, identity, owner ref, two ordered equipment refs, four mechanism refs,
# kind/area flags, numbers, paired base stat value/known flags, bond membership, range known.
ENEMY_STATS = ("maxHp", "atk", "def", "res", "moveSpeed", "bat", "aspd", "rangeRadius",
               "blockCnt", "massLevel", "lpr", "hpRecoveryPerSec", "elementRes", "elementDmgRes")
ENEMY_SCALES = (10000, 1000, 1000, 100, 2, 3, 100, 5, 3, 5, 5, 100, 1000, 1000)
IMMUNITIES = ("stun", "silence", "sleep", "frozen", "levitate")
ENEMY_BASE_DIM = 2 * len(ENEMY_STATS) + 4 + 3 + 2 * len(IMMUNITIES) + 2 + 2
ENEMY_DIM = 11 + ENEMY_BASE_DIM
ENEMY_TARGET_DIM = 1 + ENEMY_BASE_DIM
ENTITY_PREFIX = 9
ENTITY_DIM = ENTITY_PREFIX + len(KINDS) + len(AREAS) + len(ENTITY_NUMBERS) + 2 * len(STATS) + len(BOND_IDS) + len(DIRECTIONS) + 1 + ENEMY_TARGET_DIM
BOND_FIELDS = ("count", "layers", "active", "tier", "off", "counts_hand", "golden_only", "no_stack",
               "threshold1", "threshold2", "threshold3", "threshold4")
BOND_DIM = len(BOND_FIELDS)
EVENTS = ("IN_BATTLE", "SERVER_CHESS_SOLD", "SERVER_GAIN", "SERVER_PREP_FIN",
          "SERVER_PREP_START", "SERVER_PRICE", "SERVER_REFRESH_SHOP")
# These are exact public symbolic effect families, not inferred from display text.
EFFECTS = ("SERVER_ADD_BOND", "SERVER_ADD_BOND_CHESS_ALL", "SERVER_ADD_BOND_METHOD",
           "SERVER_ADD_BOND_ACTIVATED_MOST_LAYER", "SERVER_ADD_ACT_BOND_DIFF_LV_MOST_LAYER",
           "SERVER_ADD_BOND_IN_HAND", "SERVER_ADD_BOND_POSITION", "SERVER_ADD_BOND_ROUND_COIN_COST",
           "SERVER_ADD_REFRESH_CNT_MULTIPLIER_BOND_LAYER", "SERVER_GAIN_BOND_LAYER_BY_REFRESH_CNT",
           "SERVER_ADD_MULTIPLE_BOND", "ADD_BOND")
METHODS = ("shoplv", "round_gain_char", "hand_count", "same_row", "same_bond_diff_lv")
PARAMS = ("count", "multi", "layer", "multiplier", "max_layer", "refresh_cnt",
          "bond_add_count", "consume_count", "max_add_count_per_battle")
# Structural support is distinct from execution prediction. Partial rows remain visible.
MECH_DIM = 2 + len(EVENTS) + len(EFFECTS) + len(METHODS) + 2 * len(PARAMS) + len(BOND_IDS) + 5
GLOBAL_DIM = len(GLOBAL_FIELDS) + len(PHASES)
ENTITY_START = GLOBAL_DIM
BOND_START = ENTITY_START + MAX_ENTITIES * ENTITY_DIM
MECH_START = BOND_START + len(BOND_IDS) * BOND_DIM
GRID_START = MECH_START + MAX_MECHANISMS * MECH_DIM
# Grid row: usable, ranged, occupant ref, coverage count, air coverage count.
GRID_DIM = 5
ENEMY_START = GRID_START + len(CELLS) * GRID_DIM
RANGE_START = ENEMY_START + MAX_ENEMIES * ENEMY_DIM
FACT_START = RANGE_START + MAX_ENTITIES * MAX_RANGE * 3
CONTROL_START = FACT_START + MAX_FACTS * FACT_DIM
OBS_DIM = CONTROL_START + CONTROL_DIM
# Four refs: source, target, replaced, offer/choice; destination cell, direction,
# kind, move subtype; actual cost plus public event params and inventory family summary.
ACTION_NUMBERS = ("cost", "coin", "count", "rounds", "board_copies", "hand_copies",
                  "temp_copies", "golden_owned", "upgrade_num", "copies_missing", "to_hand", "to_hand_slot")
ACTION_DIM = 5 + len(DIRECTIONS) + len(ACTION_TYPES) + len(MOVE_TYPES) + len(ACTION_NUMBERS)


def scalar(value, scale=1):
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ValueError("Structured features require finite public numbers")
    return math.tanh(value / scale)


def flags(value, vocabulary):
    return tuple(float(value == key) for key in vocabulary)


def _pack(rows, limit, width, name):
    if len(rows) > limit or any(len(row) != width for row in rows):
        raise ValueError("Structured " + name + " table exceeds declared shape")
    return tuple(v for row in rows for v in row) + (0.0,) * ((limit - len(rows)) * width)


def mechanism_supported(record):
    """Decoder coverage is audit metadata, never a policy feature."""
    event, effect, bb, strings = record["event"], record["effect"], record["bb"], record["strings"]
    method, condition = strings.get("add_method"), strings.get("conditionkey")
    return (effect in EFFECTS[:-2] and event in EVENTS
            and (effect != "SERVER_ADD_BOND_METHOD" or method in METHODS)
            and condition in (None, "", "character_target_inboard")
            and set(bb) <= set(PARAMS)
            and set(strings) <= {"bond", "add_method", "conditionkey", "dir"})


def mechanism_row(record):
    event, effect = record["event"], record["effect"]
    bb, strings = record["bb"], record["strings"]
    targets = set(filter(None, strings.get("bond", strings.get("bond_id", "")).split(',')))
    if targets - set(BOND_IDS):
        raise ValueError("Unknown mechanism target bond")
    method = strings.get("add_method")
    condition = strings.get("conditionkey")
    nums = tuple(v for key in PARAMS for v in
                 (scalar(bb.get(key, 0), 10), float(key in bb)))
    tail = (float(record["requires_active"]),
            float(condition == "character_target_inboard"),
            float(strings.get("dir") == "behind"), float(strings.get("dir") == "front"),
            float(effect == "SERVER_ADD_BOND_CHESS_ALL"))
    return (1.0, record["index"]) + flags(event, EVENTS) + flags(effect, EFFECTS) + flags(method, METHODS) + nums + tuple(float(key in targets) for key in BOND_IDS) + tail


def enemy_base(catalog, identifier):
    rec = catalog.enemy_record(identifier)
    stats = rec.get("stats", {})
    numeric = tuple(v for key, scale in zip(ENEMY_STATS, ENEMY_SCALES) for v in
                    (scalar(stats.get(key, 0), scale), float(type(stats.get(key)) in (int, float))))
    immunities = stats.get("immunities", {})
    damage = set(stats.get("dmgTypes", [stats.get("dmgType")]))
    return (numeric + tuple(float(k in damage) for k in ("phys", "magic", "arts", "element"))
            + flags(stats.get("motion"), ("WALK", "FLY", "NONE"))
            + tuple(v for key in IMMUNITIES for v in (float(immunities.get(key, False)), float(key in immunities)))
            + flags(rec.get("applyWay"), ("MELEE", "RANGED"))
            + (float(rec.get("notCountInTotal", False)), float(rec.get("tokenOnly", False))))


def project(state, catalog, history):
    """Return the compact public observation and reference maps. No Torch dependency."""
    own, public, shop = state["own"], state["public"], state["own"]["shop"]
    progress = history.get("progress")
    if (not isinstance(progress, dict) or progress.get("schema") != PROGRESS_SCHEMA
            or progress.get("observation_id") != fingerprint(state)):
        raise ValueError("Structured v7 requires observation-bound public progress")
    if len(catalog.entity_ids) >= ENTITY_VOCAB or len(catalog.mechanism_ids) >= MECHANISM_VOCAB or len(catalog.enemy_ids) >= ENEMY_VOCAB:
        raise ValueError("Catalog vocabulary exceeds structured embedding contract")
    entities, uid_refs, shop_refs, offer_refs, choice_refs, band_refs = [], {}, {}, {}, {}, {}
    def add(row, area):
        if len(entities) >= MAX_ENTITIES:
            raise ValueError("Public entity capacity exceeded; no truncation")
        entities.append((row, area))
        return len(entities)
    for area in ("board", "hand", "temp"):
        for row in own[area]:
            if row:
                uid_refs[row["uid"]] = add(row, area)
                for item in row.get("items", []):
                    uid_refs[item["uid"]] = add({"kind": "item", **item, "ownerUid": row["uid"]}, "equipped")
    for index, row in enumerate(shop["slots"]):
        if row and not row.get("sold", False):
            shop_refs[index] = add(row, "shop")
    reward = shop.get("rewardOffer") or {}
    for index, row in enumerate(reward.get("slots", [])):
        if row and not row.get("sold", False):
            offer_refs[index] = add(row, "offer")
    choice = (own.get("personalChoice") if state["phase"] == "PREP" else
              public.get("sp") if state["phase"] == "SP_DRAFT" else None) or {}
    for index, row in enumerate(choice.get("cards", [])):
        if not row.get("soldOut", False) and str(index) not in choice.get("taken", {}):
            choice_refs[index] = add(row, "offer")
    for effect in own.get("effects", []):
        if effect.get("iconKind") != "band":
            definition = catalog.visible_effect_definition(effect)
            add({"kind": "effect", "id": None, "definition": definition,
                 "counter": effect.get("counter")}, "active_effect")
    if own.get("bandId"):
        add({"id": own["bandId"], "kind": "band"}, "faction")
    if state["phase"] == "BAND_DRAFT":
        for identifier in catalog.band_ids:
            if identifier not in (public.get("draft") or {}).get("picks", {}).values():
                band_refs[identifier] = add({"id": identifier, "kind": "band"}, "offer")
    _, _, definition_indices = catalog.definitions()
    from arkguardrl_core.env_observation import encoder, number
    rows, board = [], []
    family_counts = {}
    from .dynamic import definition_key
    for row, area in entities:
        kind, identifier = row["kind"], row["id"]
        if kind in ("bounty", "tactic"):
            catalog.choice_record(kind, row)
        definition = definition_indices[definition_key(row)] - 1
        rec = catalog._views.get(kind, {}).get(identifier, {})
        equipment = [uid_refs[item["uid"]] for item in row.get("items", [])]
        if len(equipment) > 2:
            raise ValueError("Public equipment reference capacity exceeded")
        refs = (uid_refs.get(row.get("ownerUid"), 0), *(equipment + [0, 0])[:2])
        exposure = progress["pieces"].get(str(row.get("uid")), {})
        numbers = (number(row.get("price", 0)), number(row.get("row", -1)),
                   number(row.get("col", -1)), number(row.get("count", 0)),
                   number(row.get("coin", 0)), number(row.get("count", 0)),
                   number(row.get("rounds", 0)), number(exposure.get("visible_pre_refreshes", 0)),
                   float(exposure.get("history_complete", False)), number(row.get("counter") or 0),
                   float(type(row.get("counter")) in (int, float)))
        direction = DIRECTIONS.index(row["dir"]) if row.get("dir") in DIRECTIONS else -1
        rows.append((definition + 1, refs, AREAS.index(area), direction, numbers))
        if area == "board":
            board.append((definition + 1, row["row"], row["col"], direction, uid_refs[row["uid"]]))
        if kind == "chess" and area in ("board", "hand", "temp"):
            base = rec.get("baseId", identifier)
            counts = family_counts.setdefault(base, Counter())
            counts["golden" if rec.get("isGolden") else area] += 1
    visible = {b["bondId"]: b for b in own["bonds"]}
    if set(visible) - set(BOND_IDS):
        raise ValueError("Unknown visible bond")
    bond_rows = []
    for key in BOND_IDS:
        b = visible.get(key, {})
        bond_rows.append((scalar(b.get("count", 0), 12), scalar(b.get("layers", 0), 100),
                          float(b.get("active", False)), scalar(b.get("tier", 0), 3),
                          float(b.get("off", False) or key in public["disabledBonds"])))
    tiles = [(c["row"], c["col"], c["kind"] == "ranged") for c in own["deployment"]["cells"]]
    enemy_rows = [(definition_indices[("enemy", enemy["enemyKey"])], catalog.enemy_index(enemy["enemyKey"]),
                   (number(enemy.get("count", 0)), number(enemy.get("t", 0)), float(enemy.get("fly", False)),
                    float(enemy.get("elite", False)), float(enemy.get("boss", False))),
                   ("lower", "upper", "left", "right").index(enemy["gate"])
                   if enemy.get("gate") in ("lower", "upper", "left", "right") else -1)
                  for enemy in own["nextEnemies"]]
    economy = catalog.economy_rules(state)
    used, limit = history["used"], history["limit"]
    global_values = (scalar(state["round"], 20), scalar(public["lastRound"] - state["round"], 20),
                     scalar(public["bossRound"] - state["round"], 20), scalar(own["lp"], 30), scalar(own["funds"], 30),
                     scalar(shop["level"], 6), scalar(shop["upgradePrice"], 30), scalar(shop["refreshPrice"], 10),
                     scalar(shop["freeRefreshes"], 10), float(shop["frozen"]),
                     scalar(own["deployCap"] - own["deployCount"], 12),
                     scalar(sum(p is None for p in own["hand"]), 10), scalar(sum(p is not None for p in own["temp"]), 5),
                     scalar(progress["spent"], 30), float(progress["spent_known"]),
                     scalar(progress["refreshes"], 10), float(progress["refreshes_known"]),
                     scalar(progress["direct_chess_gains"], 10), float(progress["direct_gains_complete"]),
                     *(scalar(economy[k], 6) for k in ("max_level", "next_tier", "current_chess_slots", "next_chess_slots", "current_item_slots", "next_item_slots")),
                     float(economy["funds_carry"]), 1.0,
                     *(scalar(economy[k], 3) for k in ("enemy_hp_scale", "enemy_atk_scale", "enemy_speed_scale"))) + flags(state["phase"], PHASES)
    budget = (1.0, max(0.0, (limit - used) / limit)) if state["phase"] == "PREP" else (0.0, 0.0)
    report = progress["last_execution"]
    refs, definitions = [], []
    for role in ("source", "target", "replaced"):
        entity = (report or {}).get(role) or {}
        refs.append(uid_refs.get(entity.get("uid"), 0))
        definitions.append(catalog.entity_index(entity["kind"], entity["id"], allow_unknown=True)
                           if entity.get("kind") in ("chess", "item", "token", "band") else 0)
    changes = (report or {}).get("changes", {})
    control = tuple(refs + definitions) + budget + (float(report is not None),
        float(bool(report and report["accepted"])), float(bool(report and report["round"] == state["round"])))
    control += flags((report or {}).get("action_type"), ACTION_TYPES) + flags((report or {}).get("error_class"), ERRORS)
    control += tuple(scalar(changes.get(key, 0), 30 if key in ("funds_delta", "spent_delta") else 10) for key in CHANGE_FIELDS)
    control += tuple(scalar((report or {}).get("bond_layer_deltas", {}).get(key, 0), 100) for key in BOND_IDS)
    intent = (report or {}).get("intent", {})
    to = intent.get("to", {})
    values = (to.get("row", intent.get("row")), to.get("col", intent.get("col")),
              to.get("idx"), intent.get("slot"), intent.get("idx"),
              ((report or {}).get("source") or {}).get("row"), ((report or {}).get("source") or {}).get("col"),
              ((report or {}).get("target") or {}).get("row"), ((report or {}).get("target") or {}).get("col"))
    control += flags(intent.get("dir"), DIRECTIONS) + flags(to.get("area"), ("board", "hand"))
    control += tuple(v for value in values for v in
                     (scalar(value, 21) if value is not None else 0.0, float(value is not None)))
    observation = encoder(catalog).encode(global_values, rows, bond_rows, tiles, board, enemy_rows, control)
    return {"observation": observation, "uid_refs": uid_refs, "shop_refs": shop_refs,
            "offer_refs": offer_refs, "choice_refs": choice_refs, "band_refs": band_refs,
            "raw_entities": entities, "families": family_counts}


def encode_decision(state, actions, catalog, history_snapshot):
    p = project(state, catalog, history_snapshot)
    from .dynamic import OBS_DIM as WIRE_DIM
    obs = p["observation"]
    from .domain import _locations
    from .actions import describe_action, descriptor
    locations = _locations(state)
    descriptors = {uid: descriptor(row, area) for uid, (area, _, row) in locations.items()}
    occupied = {(r["row"], r["col"]): r for r in state["own"]["board"]}
    encoded, schedules, experts = [], [], []
    # A source appears in many placement candidates. Own one catalog projection
    # per definition for this decision instead of deep-copying it per action.
    chess_records, number_templates, slot_numbers, flag_templates, route_templates = {}, {}, {}, {}, {}
    cell_indices = {cell: index+1 for index, cell in enumerate(CELLS)}
    for action in actions:
        kind = action['t'][2:]
        to = action.get('to', {})
        location = locations.get(action.get('uid'))
        if kind == 'move' and location is not None and to.get('area') in ('board', 'hand'):
            # Movement features only need references and a semantic class. Avoid
            # constructing three descriptor dictionaries and a dataclass per cell.
            source = p['uid_refs'].get(action['uid'], 0)
            if to['area'] == 'board':
                occupant = occupied.get((to['row'], to['col']))
                semantic = (('board_swap' if occupant and occupant['uid'] != action['uid'] else 'reposition')
                            if location[0] == 'board' else ('reserve_replace' if occupant else 'deploy'))
            else:
                index = to['idx']
                hand = state['own']['hand']
                occupant = hand[index] if 0 <= index < len(hand) else None
                semantic = 'withdraw' if location[0] == 'board' else 'reserve_move'
            target = p['uid_refs'].get((occupant or {}).get('uid'), 0)
            replaced = p['uid_refs'].get(action.get('replaceUid'), 0)
            cost = 0
            description = None
        else:
            description = describe_action(state, action, locations=locations, occupied=occupied, descriptors=descriptors)
            source = p['uid_refs'].get((description.source or {}).get('uid'), 0)
            target = p['uid_refs'].get((description.target or {}).get('uid'), 0)
            replaced = p['uid_refs'].get((description.replaced or {}).get('uid'), 0)
            to, cost, semantic = description.destination, description.cost, description.semantic
        subject = (p["shop_refs"].get(action.get("slot"), 0) if kind == "buy" else
                   p["offer_refs"].get(action.get("idx"), 0) if kind == "reward" else
                   p["choice_refs"].get(action.get("idx"), 0) if kind == "choice" else
                   p["band_refs"].get(action.get("bandId"), 0) if kind == "band" else 0)
        row = p["raw_entities"][subject - 1][0] if subject else {}
        subject_row = row or (p["raw_entities"][source - 1][0] if source else {})
        number_key = (source, subject, type(cost), cost)
        if number_key not in number_templates:
            rec = {}
            if subject_row.get("kind") == "chess":
                identifier = subject_row['id']
                if identifier not in chess_records:
                    chess_records[identifier] = catalog._record('chess', identifier)
                rec = chess_records[identifier]
            counts = p["families"].get(rec.get("baseId", subject_row.get("id")), {})
            needed = rec.get("upgradeNum", 0) or 0
            base_numbers = (scalar(cost, 30), scalar(row.get("coin", 0), 10), scalar(row.get("count", 0), 10),
                    scalar(row.get("rounds", 0), 10), *(scalar(counts.get(a, 0), 3) for a in ("board", "hand", "temp", "golden")),
                    scalar(needed, 3), scalar(max(0, needed - sum(counts.get(a, 0) for a in ("board", "hand", "temp"))), 3))
            number_templates[number_key] = base_numbers
        slot = to.get("idx", -1)
        slot_key = (type(slot), slot)
        if slot_key not in slot_numbers:
            slot_numbers[slot_key] = scalar(slot, 10)
        nums = number_templates[number_key] + (float(to.get("area") == "hand"), slot_numbers[slot_key])
        cell = (to.get("row", action.get("row")), to.get("col", action.get("col")))
        destination = cell_indices.get(cell, 0)
        flag_key = (action.get("dir"), kind, semantic)
        if flag_key not in flag_templates:
            flag_templates[flag_key] = (flags(flag_key[0], DIRECTIONS) + flags(kind, ACTION_TYPES)
                                        + flags(semantic, MOVE_TYPES))
        encoded.append((source, target, replaced, subject, destination) + flag_templates[flag_key] + nums)
        route_key = (kind, semantic)
        if route_key not in route_templates:
            route_templates[route_key] = routing(state, action, description=description)
        schedule, expert = route_templates[route_key]
        schedules.append(schedule)
        experts.append(expert)
    if len(obs) != WIRE_DIM or any(len(r) != ACTION_DIM for r in encoded):
        raise AssertionError("Structured feature layout changed")
    return obs, tuple(encoded), tuple(schedules), tuple(experts)


def encode_observation(state, catalog, history):
    catalog.validate_state(state)
    snapshot = history.snapshot(state) if hasattr(history, "snapshot") else history
    return encode_decision(state, (), catalog, snapshot)[0]


def main():
    """Read-only field/catalog audit; does not start games or import Torch."""
    import argparse
    import json
    from pathlib import Path
    from .catalog import PublicCatalog
    parser = argparse.ArgumentParser(description=main.__doc__)
    parser.add_argument("--engine-root", required=True)
    parser.add_argument("--snapshot", help="Optional single raw public observation JSON")
    args = parser.parse_args()
    catalog = PublicCatalog(args.engine_root)
    from .dynamic import OBS_DIM as WIRE_DIM
    summary = {"schema": FEATURE_SCHEMA, "catalog": catalog.schema,
               "data_sha256": catalog.data_sha256, "wire_observation_dim": WIRE_DIM, "internal_encoder_slots": OBS_DIM, "static_definition_sha256": catalog.definitions()[1],
               "action_dim": ACTION_DIM, "routing_intents": SCHEDULES, "semantic_schema": SEMANTIC_SCHEMA,
               "semantic_symbols": len(catalog.semantic_symbols), "enemy_stats": ENEMY_STATS, "enemy_immunities": IMMUNITIES, "global_fields": GLOBAL_FIELDS, "control_fields": CONTROL_FIELDS, "control_references": CONTROL_REFERENCES,
               "entity_numbers": ENTITY_NUMBERS, "base_stats": STATS,
               "bond_fields": BOND_FIELDS, "mechanism_events": EVENTS,
               "mechanism_effects": EFFECTS, "mechanism_params": PARAMS,
               "action_numbers": ACTION_NUMBERS,
               "field_registry": {
                   "global": {"fields": GLOBAL_FIELDS, "source": "public own/shop", "encoding": "tanh scaled values; phase one-hot"},
                   "entities": {"prefix": ["present", "catalog_index", "owner_ref", "equipment1", "equipment2", "mechanism1", "mechanism2", "mechanism3", "mechanism4"],
                                "numbers": ENTITY_NUMBERS, "static_stats": STATS, "stat_scales": STAT_SCALES,
                                "stat_units": ["HP", "ATK", "DEF", "RES percent points", "seconds", "ASPD", "block count"],
                                "missing": "paired stat-known flags; parser coverage is audit metadata only"},
                   "mechanisms": {"events": EVENTS, "effects": EFFECTS, "methods": METHODS,
                                  "params": PARAMS, "source": "public garrisons; static structural semantics only"},
                   "grid": {"fields": ["usable", "ranged", "occupant_ref", "coverage", "air_coverage"],
                            "shape": [4, 9], "source": "public deployment and default rotated attack ranges"},
                   "actions": {"references": ["source", "target", "replaced", "offer_or_choice", "destination_cell"],
                               "numbers": ACTION_NUMBERS, "refs": "1-based with 0 absent; no UIDs in model"}},
               "limits": {"entities": MAX_ENTITIES, "mechanisms": MAX_MECHANISMS,
                          "enemies": MAX_ENEMIES, "range_cells_per_entity": MAX_RANGE, "semantic_facts": MAX_FACTS}}
    definitions = {}
    for kind, identifier in catalog.entity_ids:
        if kind == "chess" and not catalog._views[kind][identifier].get("isDiy"):
            for row in catalog.mechanisms(kind, identifier):
                definitions[row["id"]] = row
    summary["garrison_coverage"] = {"referenced_definitions": len(definitions),
        "structurally_supported": sum(mechanism_supported(row) for row in definitions.values()),
        "partial_ids": [key for key, row in definitions.items() if not mechanism_supported(row)],
        "source": "data/garrisons.json; server/sim/content/garrisons/meta.js",
        "limitation": "Structural garrison coverage only; skills/talents/items and live per-copy counters are incomplete"}
    if args.snapshot:
        state = json.loads(Path(args.snapshot).read_text(encoding='utf-8'))
        catalog.validate_state(state)
        observer = PublicProgress()
        observer.observe(state)
        tables = project(state, catalog, {"used": 0, "limit": 64, "blocked_count": 0,
                                         "progress": observer.snapshot(state)})
        summary["snapshot"] = {"observation": tables["observation"],
                               "schema": FEATURE_SCHEMA, "dimension": WIRE_DIM}
        summary["snapshot"]["entity_bindings"] = [{"index": i + 1, "kind": row.get("kind"),
            "id": row.get("id"), "area": area} for i, (row, area) in enumerate(tables["raw_entities"])]
        summary["snapshot"]["history_note"] = "Single-snapshot partial progress and empty attempt history; not training evidence"
    print(json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=2))


if __name__ == "__main__":
    main()
