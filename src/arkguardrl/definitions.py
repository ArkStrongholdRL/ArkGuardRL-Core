"""Compile pinned public definitions once; no state, UIDs, waves or neural weights."""
from .contracts import fingerprint
from . import structured as layout
from .semantics import encode as encode_facts

SCHEMA = "arkguardrl/static-public-definitions-v1"


def compile_definitions(catalog):
    keys = [key for key in catalog.entity_ids
            if key[0] != "chess" or not catalog._views[key[0]][key[1]].get("isDiy")]
    keys += sorted(catalog._choices)
    keys += [("enemy", key) for key in catalog.enemy_ids]
    keys += [("effect", "unresolved")]
    indices = {key: i + 1 for i, key in enumerate(keys)}
    mechs, mech_refs = [], {}
    entities, ranges, enemies, facts = [], [], [], []
    for kind, identifier in keys:
        base_kind = kind in ("chess", "item", "token", "band")
        rec = catalog.record(kind, identifier) if base_kind else {}
        combat = catalog.combat_record(kind, identifier) if base_kind else {}
        references = []
        for mechanism in catalog.mechanisms(kind, identifier) if base_kind else ():
            if mechanism["id"] not in mech_refs:
                mech_refs[mechanism["id"]] = len(mechs) + 1
                mechs.append(layout.mechanism_row(mechanism))
            references.append(mech_refs[mechanism["id"]])
        if len(references) > 4:
            raise ValueError("Static mechanism reference capacity exceeded")
        references += [0] * (4 - len(references))
        nums = {"tier": layout.scalar(rec.get("tier", 0) or 0, 6), "golden": float(rec.get("isGolden", False)),
                "upgrade_num": layout.scalar(rec.get("upgradeNum", 0) or 0, 3),
                "fly_known": float("canHitFly" in combat), "can_hit_fly": float(combat.get("canHitFly", False)),
                "melee": float(combat.get("position") == "MELEE"), "ranged": float(combat.get("position") == "RANGED")}
        nums.update(zip(("phys", "magic", "heal"), layout.flags(combat.get("dmgType"), ("phys", "magic", "heal"))))
        if kind == "bounty":
            choice = catalog._choices[(kind, identifier)]
            nums["tier"] = layout.scalar(choice.get("tier", 0), 6)
        stats = combat.get("stats", {})
        stat_values = tuple(v for key, scale in zip(layout.STATS, layout.STAT_SCALES) for v in
                            (layout.scalar(stats.get(key, 0), scale), float(type(stats.get(key)) in (int, float))))
        target = ((1.0,) + layout.enemy_base(catalog, catalog._choices[(kind, identifier)]["enemyKey"])
                  if kind == "bounty" else (0.0,) * layout.ENEMY_TARGET_DIM)
        row = ((0.0, catalog.entity_index(kind, identifier) if base_kind else 0, 0, 0, 0, *references)
               + layout.flags(kind, layout.KINDS) + (0.0,) * len(layout.AREAS)
               + tuple(nums.get(key, 0.0) for key in layout.ENTITY_NUMBERS) + stat_values
               + tuple(float(key in rec.get("bonds", [])) for key in layout.BOND_IDS)
               + (0.0,) * len(layout.DIRECTIONS) + (float(kind == "chess"),) + target)
        entities.append(row)
        grid = catalog.attack_range({"kind": kind, "id": identifier}) if kind == "chess" else ()
        ranges.append(layout._pack([(1.0, layout.scalar(dr, 21), layout.scalar(dc, 21)) for dr, dc in grid], layout.MAX_RANGE, 3, "static range"))
        enemies.append(layout.enemy_base(catalog, identifier) if kind == "enemy" else (0.0,) * layout.ENEMY_BASE_DIM)
        facts.append(encode_facts(catalog.semantic_facts(kind, identifier), 0, catalog.semantic_indices)
                     if kind != "effect" else [])
    bank = {"schema": SCHEMA, "data_sha256": catalog.data_sha256, "keys": keys,
            "entities": entities, "ranges": ranges, "enemies": enemies, "facts": facts,
            "mechanisms": mechs, "bond_rules": catalog.bond_rules()}
    return bank, fingerprint(bank), indices
