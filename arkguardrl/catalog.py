"""Pinned, external PUBLIC data, not a simulator or a private legality oracle.

The engine and its datasets remain external. Public placement semantics were
researched against Stronghold Protocol 41bbca57, server/match/board.js,
shared/highGround.js and shared/loadoutRecord.js (GPL-3.0). No engine code is
imported. Dataset/asset licensing is separate from this framework's license.
"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import subprocess

from .engine.session import ENGINE_REVISION, ENGINE_VERSION
from .observation import validate_state

CATALOG_SCHEMA = "arkguardrl/external-public-catalog031-v4"


class UnsupportedPublicState(ValueError):
    """The explicit vanilla/default-loadout learning boundary was exceeded."""


class PublicCatalog:
    schema = CATALOG_SCHEMA

    def __init__(self, engine_root, expected_data_sha256=None):
        self.engine_root = Path(engine_root).resolve(strict=True)
        def git(*args):
            return subprocess.check_output(["git", "-C", str(self.engine_root), *args],
                                           text=True, timeout=10).strip()
        if git("rev-parse", "HEAD") != ENGINE_REVISION or git("status", "--porcelain", "--untracked-files=all"):
            raise ValueError("Public catalog requires the clean frozen engine checkout")
        if json.loads((self.engine_root / "package.json").read_text())["version"] != ENGINE_VERSION:
            raise ValueError("Public catalog requires engine package 0.3.3")
        digest, loaded = hashlib.sha256(), {}
        for path in sorted((self.engine_root / "data").glob("*.json")):
            raw = path.read_bytes()
            digest.update(path.name.encode() + b"\0" + raw + b"\0")
            if path.stem in ("bands", "chess", "items", "tokens", "config", "bonds", "garrisons", "enemies", "choices", "effects"):
                def unique(pairs):
                    out = {}
                    for key, value in pairs:
                        if key in out:
                            raise ValueError("Duplicate catalog JSON key")
                        out[key] = value
                    return out
                loaded[path.stem] = json.loads(raw, object_pairs_hook=unique)
        self.data_sha256 = digest.hexdigest()
        if expected_data_sha256 is not None and self.data_sha256 != expected_data_sha256:
            raise ValueError("Public catalog/session data identity mismatch")
        if set(loaded) != {"bands", "chess", "items", "tokens", "config", "bonds", "garrisons", "enemies", "choices", "effects"}:
            raise ValueError("Missing explicit public catalog files")
        self._data = loaded
        # The learning API exposes small public projections, not every artwork,
        # description, skill implementation or future-wave table in the dataset.
        fields = ("id", "chessId", "tokenId", "bandId", "baseId", "goldenId", "tier", "isGolden",
                  "position", "placeOnRanged", "isDiy", "itemType", "kind", "rangedTilesOnly",
                  "ownerRange", "ownerRangeOutside", "rangeGrid", "upgradeNum", "bonds")
        self._views = {}
        for kind, table in (("chess", "chess"), ("item", "items"), ("token", "tokens"), ("band", "bands")):
            self._views[kind] = {}
            for identifier, source in loaded[table].items():
                view = {key: deepcopy(source[key]) for key in fields if key in source}
                for key in ("trait", "traitBase"):
                    trait = source.get(key) or {}
                    if not trait:
                        view[key] = {}
                        continue
                    view[key] = {name: deepcopy(trait[name]) for name in ("desc", "moduleDesc", "rangeExpand", "modeOnly") if name in trait}
                    view[key]["bb"] = {name: value for name, value in (trait.get("bb") or {}).items() if name == "ability_range_forward_extend"}
                skill = source.get("skill") or {}
                view["skill"] = {key: deepcopy(skill[key]) for key in ("desc", "passiveRange", "rangeGrid") if key in skill}
                module = source.get("module") or {}
                view["module"] = {key: module[key] for key in ("id", "active") if key in module}
                view["modules"] = [{"uniEquipId": module.get("uniEquipId"), "talentChanges": [
                    {key: deepcopy(change[key]) for key in ("talentIndex", "rangeGrid") if key in change}
                    for change in module.get("talentChanges", [])]} for module in source.get("modules", [])]
                self._views[kind][identifier] = view
        bands = [b for b in loaded["bands"].values()
                 if "SINGLE" in b.get("modeTypeList", ["SINGLE"])]
        self.band_ids = tuple(b["bandId"] for b in sorted(bands, key=lambda b: (b["sortId"], b["bandId"])))
        self._placement_classes, self._ranges = {}, {}
        self.entity_ids = tuple(sorted((kind, key) for kind, rows in self._views.items() for key in rows))
        self._entity_indices = {identity: i + 1 for i, identity in enumerate(self.entity_ids)}
        self.enemy_ids = tuple(sorted(loaded["enemies"]))
        self._enemy_indices = {key: i + 1 for i, key in enumerate(self.enemy_ids)}
        self.mechanism_ids = tuple(sorted(loaded["garrisons"]))
        self._mechanism_indices = {key: i + 1 for i, key in enumerate(self.mechanism_ids)}
        from .semantics import facts, symbols, CHOICE_FIELDS, SEMANTIC_VOCAB
        self._semantic_facts = {}
        for kind, table in (("chess", "chess"), ("item", "items"), ("token", "tokens"),
                            ("band", "bands"), ("enemy", "enemies")):
            for key, row in loaded[table].items():
                definition = deepcopy(row)
                if kind != "enemy":
                    definition.pop("skills", None)  # Selected default skill only, not alternative loadouts.
                if kind in ("chess", "token"):
                    definition["garrisons"] = [loaded["garrisons"][g] for g in row.get("garrisonIds", [])]
                self._semantic_facts[(kind, key)] = facts(definition)
        self._choices = {}
        for kind in ("bounty", "tactic"):
            for row in loaded["choices"]["cards"][kind]:
                key = (kind, row["effectId"])
                if key in self._choices:
                    raise ValueError("Duplicate public choice identity")
                row = deepcopy(row)
                if kind == "bounty":
                    row["rounds"] = 2 if row.get("multiRound") or row.get("rounds", 1) >= 90 else max(1, min(99, row.get("rounds", 1)))
                self._choices[key] = deepcopy(row)
                effect = loaded["effects"].get(row["effectId"])
                if not isinstance(effect, dict):
                    raise ValueError("Choice lacks its public effect definition")
                self._semantic_facts[key] = facts({**row, "buffs": effect.get("buffs", []),
                    "params": effect.get("params", {})}, CHOICE_FIELDS)
                if kind == "bounty":
                    target = self._semantic_facts[("enemy", row["enemyKey"])]
                    self._semantic_facts[key] += tuple(("bounty_target." + scope, name, ty, num, val)
                                                       for scope, name, ty, num, val in target)
        vocabulary = sorted({name for rows in self._semantic_facts.values() for name in symbols(rows)})
        if len(vocabulary) >= SEMANTIC_VOCAB:
            raise ValueError("Pinned semantic vocabulary exceeds explicit capacity")
        self.semantic_symbols = tuple(vocabulary)
        self.semantic_indices = {key: i + 1 for i, key in enumerate(vocabulary)}


    def definitions(self):
        if not hasattr(self, "_definition_bank"):
            from .definitions import compile_definitions
            self._definition_bank = compile_definitions(self)
        return self._definition_bank

    def _record(self, kind, identifier):
        """Borrow an immutable catalog projection for internal read-only use."""
        row = self._views.get(kind, {}).get(identifier)
        if not isinstance(row, dict):
            raise UnsupportedPublicState(f"Unknown public {kind} identity: {identifier!r}")
        if kind == "chess" and row.get("isDiy"):
            raise UnsupportedPublicState("Custom DIY records require a separate catalog contract")
        return row

    def validate_identity(self, kind, identifier):
        """Check the public definition boundary without copying its projection."""
        self._record(kind, identifier)

    def record(self, kind, identifier):
        return deepcopy(self._record(kind, identifier))

    def semantic_facts(self, kind, identifier):
        try:
            return self._semantic_facts[(kind, identifier)]
        except KeyError as exc:
            raise UnsupportedPublicState("Unknown public semantic definition") from exc

    def visible_effect_definition(self, row):
        """Resolve public effect identities; never parse prose or instance UID as semantics."""
        identifier = row.get("id", "").split("#", 1)[0]
        if identifier.startswith("bounty:"):
            identifier = row.get("iconId")
        for kind in ("bounty", "tactic"):
            if (kind, identifier) in self._choices:
                return kind, identifier
        # Some item effect views have only a shared normal/golden icon, which
        # cannot identify concrete effect parameters. Keep that uncertainty.
        return None

    def choice_record(self, kind, row):
        # Bind by the actual public effect ID, never by display order or prose.
        key = row.get("effectId", row.get("id"))
        rec = self._choices.get((kind, key))
        if rec is None:
            raise UnsupportedPublicState("Unknown public choice effect")
        for field in ("enemyKey", "payout", "coin", "count", "rounds"):
            if field in row and row[field] != rec.get(field):
                raise UnsupportedPublicState("Public choice differs from pinned definition")
        return deepcopy(rec)

    def enemy_record(self, identifier):
        self.enemy_index(identifier)
        source = self._data["enemies"][identifier]
        return {key: deepcopy(source[key]) for key in
                ("stats", "applyWay", "rank", "isFlyEnemy", "notCountInTotal", "tokenOnly") if key in source}

    def economy_rules(self, state):
        mode = self._data["config"]["modes"][state["public"]["modeId"]]
        economy = mode.get("economy", self._data["config"]["economy"])
        level = state["own"]["shop"]["level"]
        maximum = mode["maxShopLevel"]
        nxt = min(level + 1, maximum)
        current, following = (mode["shopSlots"][str(n)] for n in (level, nxt))
        scale = mode.get("enemyScale", {}).get(str(state["round"]), {})
        return {"max_level": maximum, "next_tier": nxt,
                "current_chess_slots": current["chess"], "next_chess_slots": following["chess"],
                "current_item_slots": current["item"], "next_item_slots": following["item"],
                "funds_carry": state["own"].get("bandId") in economy["leftoverFundsKeptByBands"],
                "enemy_hp_scale": scale.get("hp", 1), "enemy_atk_scale": scale.get("atk", 1),
                "enemy_speed_scale": scale.get("speed", 1)}

    def enemy_index(self, identifier):
        if identifier not in self._enemy_indices:
            raise UnsupportedPublicState("Unknown visible enemy identity")
        return self._enemy_indices[identifier]

    def entity_index(self, kind, identifier, *, allow_unknown=False):
        if allow_unknown and (kind, identifier) not in self._entity_indices:
            return 0  # Rejected intents may name an identity absent from the public catalog.
        self.validate_identity(kind, identifier)
        return self._entity_indices[(kind, identifier)]

    def combat_record(self, kind, identifier):
        """Public default catalog values, never current effective combat attributes."""
        self.validate_identity(kind, identifier)
        table = {"chess": "chess", "item": "items", "token": "tokens", "band": "bands"}[kind]
        source = self._data[table][identifier]
        keys = ("stats", "canHitFly", "dmgType", "position", "garrisonIds")
        return {key: deepcopy(source[key]) for key in keys if key in source}

    def mechanisms(self, kind, identifier):
        """Public trait definitions. Prose is used only for the pinned active gate convention."""
        source = self.combat_record(kind, identifier)
        out = []
        for key in source.get("garrisonIds", []):
            row = self._data["garrisons"].get(key)
            if not isinstance(row, dict):
                raise UnsupportedPublicState("Missing public garrison definition")
            desc = row.get("desc", "")
            active = False if "无需激活" in desc else ("已激活" in desc or "激活且" in desc)
            out.append({"id": key, "index": self._mechanism_indices[key],
                        "event": row.get("eventType"), "effect": row.get("effectType"),
                        "bb": deepcopy(row.get("bb", {})), "strings": deepcopy(row.get("bbStr", {})),
                        "requires_active": active})
        return out

    def bond_rules(self):
        """Public identity and count thresholds only; no effect implementation."""
        return {key: {"thresholds": tuple(row.get("thresholds", ())),
                      "countsHand": bool(row.get("countsHand", False)),
                      "countsGoldenOnly": bool(row.get("countsGoldenOnly", False)),
                      "noStack": bool(row.get("noStack", False))}
                for key, row in self._data["bonds"].items()}

    def validate_state(self, state):
        validate_state(state)
        own, public = state["own"], state["public"]
        if (public.get("offers") or public.get("difficulty") not in ("FUNNY", "NORMAL", "HARD", "ABYSS")
                or public.get("modeId") not in self._data["config"]["modes"]
                or public.get("modeId") != "mode_single_" + public["difficulty"].lower()):
            raise UnsupportedPublicState("Plugins/custom modes are outside this vanilla catalog")
        for name in ("diy", "loadout", "ops"):
            if own.get(name) != {}:
                raise UnsupportedPublicState(f"Nondefault or unknown {name} is unsupported")
        if own.get("standIns") != []:
            raise UnsupportedPublicState("Stand-ins require a separately resolved public catalog")
        if public.get("rerollVote"):
            raise UnsupportedPublicState("Setup reroll votes are outside this solo action domain")

    def placement_class(self, piece):
        identity = (piece["kind"], piece["id"])
        if identity in self._placement_classes:
            return self._placement_classes[identity]
        rec = self.record(piece["kind"], piece["id"])
        position = rec.get("position")
        if position not in ("MELEE", "RANGED", "ALL"):
            raise UnsupportedPublicState("Missing or unknown public placement position")
        if rec.get("rangedTilesOnly") is True:
            result = "high"
        else:
            trait = rec.get("traitBase") or rec.get("trait") or {}
            result = "melee" if position == "MELEE" and not (rec.get("placeOnRanged") is True or "可以放置于远程位" in trait.get("desc", "")) else "all"
        self._placement_classes[identity] = result
        return result

    def attack_range(self, piece):
        """Default public loadout preview only; never simulated effective stats."""
        if piece["id"] in self._ranges:
            return self._ranges[piece["id"]]
        rec = self.record("chess", piece["id"])
        grid, skill, trait = rec.get("rangeGrid"), rec.get("skill") or {}, rec.get("trait") or {}
        if skill.get("passiveRange") is True or "被动效果：攻击范围扩大" in skill.get("desc", ""):
            grid = skill.get("rangeGrid")
        elif rec.get("isGolden") and (rec.get("module") or {}).get("active") and (
                trait.get("rangeExpand") is True or "攻击范围扩大" in trait.get("moduleDesc", "")):
            module_id = rec["module"].get("id")
            module = next((m for m in rec.get("modules", []) if m.get("uniEquipId") == module_id), None)
            changes = (module or {}).get("talentChanges", [])
            widened = next((t.get("rangeGrid") for t in changes if t.get("talentIndex") == -1 and t.get("rangeGrid")), None)
            if widened:
                grid = widened
        if not isinstance(grid, list) or not grid or any(not isinstance(x, list) or len(x) != 2 or any(type(v) is not int for v in x) for x in grid):
            raise UnsupportedPublicState("Missing public default attack range")
        result = {tuple(x) for x in grid}
        extend = (trait.get("bb") or {}).get("ability_range_forward_extend", 0)
        if trait.get("modeOnly") or "集成战略" in trait.get("moduleDesc", ""):
            extend = 0
        if type(extend) not in (int, float) or not 0 <= extend <= 21:
            raise UnsupportedPublicState("Unknown public range extension")
        for row in {r for r, _ in result}:
            end = max(c for r, c in result if r == row)
            result.update((row, end + n) for n in range(1, int(extend) + 1))
        self._ranges[piece["id"]] = tuple(sorted(result))
        return self._ranges[piece["id"]]
