"""Typed public configuration facts. External data stays in the pinned engine.

Facts preserve scope, parameter name, type and value; symbolic IDs are lookup
keys, never ordinal numeric features. They expose configuration, not evaluated
skill logic, live counters, RNG, wave overrides or future wave templates.
"""
import math

SEMANTIC_SCHEMA = "arkguardrl/public-scoped-config-facts-v1"
SEMANTIC_VOCAB = 4096
MAX_FACTS, FACT_DIM = 8192, 7
# No prose, artwork, alternative loadouts, module alternatives or future tables.
TOP_FIELDS = ("skill", "skills", "abilities", "talents", "trait", "traitBase", "buffs", "params", "category",
              "itemType", "giveBondId", "canGiveBond", "requiresBondId", "duration",
              "effectId", "garrisons", "bondIds", "summons", "targetPriority", "attackKind")
LEAF_FIELDS = {"bb", "bbStr", "key", "countType", "skillId", "prefabKey", "skillType",
               "durationType", "duration", "spType", "spCost", "initSp", "maxChargeTime",
               "increment", "priority", "cooldown", "initCooldown", "cd", "icd", "sp", "sil", "hidden", "fromModule", "tokenKey",
               "format", "eventType", "effectType", "passiveRange", "trigger", "rule", "rawRule", "index"}
CHOICE_FIELDS = ("effectId", "enemyKey", "count", "coin", "rounds", "payout", "multiRound",
                 "kind", "team", "stageId", "buffs", "params", "adds")


def facts(source, fields=TOP_FIELDS):
    """Yield (scope, key, type, numeric, symbolic), retaining indexed effect groups."""
    out = []
    def walk(value, scope, key, unrestricted=False):
        if isinstance(value, dict):
            for name in sorted(value):
                if unrestricted or name in LEAF_FIELDS:
                    walk(value[name], scope + "." + key, name,
                         unrestricted or name in ("bb", "bbStr", "params", "trigger"))
        elif isinstance(value, list):
            for i, item in enumerate(value):
                walk(item, scope + "." + key + "[" + str(i) + "]", "value", unrestricted)
        elif value is None:
            out.append((scope, key, 4, 0.0, None))
        elif type(value) is bool:
            out.append((scope, key, 2, float(value), None))
        elif type(value) in (int, float):
            if not math.isfinite(value):
                raise ValueError("Nonfinite public semantic parameter")
            numeric = math.copysign(math.log1p(abs(value)), value) / 10
            out.append((scope, key, 1, numeric, None))
        elif isinstance(value, str):
            out.append((scope, key, 3, 0.0, value))
        else:
            raise ValueError("Unsupported public semantic value")
    for key in fields:
        if key in source:
            walk(source[key], "definition", key, key in ("params", "adds", "summons"))
    return tuple(out)


def symbols(rows):
    for scope, key, _, _, value in rows:
        yield "scope:" + scope
        yield "key:" + key
        if value is not None:
            yield "value:" + value


def encode(rows, owner, indices):
    out = []
    for scope, key, kind, value, symbolic in rows:
        names = ("scope:" + scope, "key:" + key)
        try:
            scope_id, key_id = (indices[name] for name in names)
            value_id = indices["value:" + symbolic] if symbolic is not None else 0
        except KeyError as exc:
            raise ValueError("Public semantic symbol outside pinned vocabulary") from exc
        out.append((1.0, owner, scope_id, key_id, kind, value, value_id))
    return out
