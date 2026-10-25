"""Conservative deterministic native baseline, not a stronger-play claim.

Selects only captured candidates. Research provenance: the retained GPL-3.0
experiments/expert_policy/teacher.py and capacity.py; native data/semantics and
policy identity are new. No private simulation, rejected-action probe, paid
leveling incentive, automatic sale, destruction or equipment replacement.
Blocked overflow is explicit, never called a natural game end.
"""
from .candidates import Decision, public_context, empty_deployment_allowed
from .features import DIRECTIONS

POLICY_ID = "arkguardrl/native-conservative-buy-deploy-ready-v1"


class BaselineBlocked(RuntimeError):
    pass


def select_with_reason(decision, catalog, policy_id=POLICY_ID):
    if policy_id != POLICY_ID or not isinstance(decision, Decision):
        raise ValueError("Explicit native baseline policy and captured Decision required")
    if decision.terminal or not decision.actions:
        raise BaselineBlocked("No live native candidate decision")
    state, actions = decision.state, decision.actions
    locations, occupied, cells = context = public_context(state, catalog)
    own, shop = state["own"], state["own"]["shop"]
    for kind in ("g.infoReady", "g.band", "g.choice"):
        choices = [(i, action) for i, action in enumerate(actions) if action["t"] == kind]
        if choices:
            return choices[0][0], "mandatory native setup/draft/choice"
    if state["phase"] != "PREP":
        raise BaselineBlocked("Unsupported baseline phase")
    deploys, cleanup, equips = [], [], []
    for index, action in enumerate(actions):
        if action["t"] == "g.move":
            area, slot, piece = locations[action["uid"]]
            if area not in ("hand", "temp"):
                continue
            target = action["to"]
            if target["area"] == "board" and piece["kind"] in ("chess", "token"):
                cell = (target["row"], target["col"])
                if empty_deployment_allowed(piece, cell, state, catalog, context):
                    rank = (area != "temp", piece["kind"] != "chess", slot, cell,
                            DIRECTIONS.index(action["dir"]), index)
                    deploys.append((rank, index))
            elif area == "temp" and target["area"] == "hand" and own["hand"][target["idx"]] is None:
                cleanup.append(((slot, -target["idx"], index), index))
        elif action["t"] == "g.equip" and "replaceUid" not in action:
            area, slot, piece = locations[action["itemUid"]]
            target_area, _, target = locations[action["targetUid"]]
            rec = catalog.record("item", piece["id"])
            if not isinstance(rec.get("kind"), str) or rec["kind"].startswith("consume_on_equip") or len(target["items"]) >= 2:
                continue
            equips.append(((area != "temp", target_area != "board", slot, target["uid"], index), index))
    for group, reason in (([x for x in deploys if not x[0][0]], "deploy temporary reserve on a publicly supported empty tile"),
                          (cleanup, "move temporary reserve to an empty hand slot"),
                          ([x for x in equips if not x[0][0]], "equip temporary nonconsumable item without replacement")):
        if group:
            return min(group)[1], reason
    if any(piece is not None for piece in own["temp"]):
        raise BaselineBlocked("Temporary reserve has no supported nondestructive resolution; no automatic sale/destruction")
    if deploys:
        return min(deploys)[1], "deploy reserve on a publicly supported empty tile"
    if equips:
        return min(equips)[1], "equip nonconsumable item without replacement"
    rewards = [(i, a) for i, a in enumerate(actions) if a["t"] == "g.reward"]
    if rewards:
        return rewards[0][0], "select first available visible reward (explicit native recovery coverage)"
    owned = [piece for area in ("board", "hand", "temp") for piece in own[area] if piece and piece["kind"] == "chess"]
    purchases = []
    for index, action in enumerate(actions):
        if action["t"] != "g.buy":
            continue
        offer = shop["slots"][action["slot"]]
        if offer["kind"] != "chess":
            continue
        if len(owned) >= own["deployCap"]:
            continue
        purchases.append(((offer["price"], action["slot"], index), index))
    if purchases:
        return min(purchases)[1], "buy affordable visible chess for deployment capacity"
    for index, action in enumerate(actions):
        if action["t"] == "g.ready":
            return index, "finish prep after supported nondestructive decisions"
    raise BaselineBlocked("No supported conservative candidate; no invented wait or destructive fallback")


def select_index(decision, catalog, policy_id=POLICY_ID):
    return select_with_reason(decision, catalog, policy_id)[0]
