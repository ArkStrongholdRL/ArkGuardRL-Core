"""Dynamic-only wire layout. Static definitions are owned once by the model/run."""
from . import structured as base

FEATURE_SCHEMA = "arkguardrl/native-dynamic-static-bank-v7"
# Definition reference is independent of catalog embedding identity.
DYNAMIC_NUMBERS = ("price", "row", "col", "count", "bounty_coin", "bounty_count", "bounty_rounds",
                   "visible_pre_refreshes", "refresh_history_complete", "effect_counter", "effect_counter_known")
NUM_START = base.ENTITY_PREFIX + len(base.KINDS) + len(base.AREAS)
ENTITY_COLUMNS = ((0, 2, 3, 4) + tuple(range(base.ENTITY_PREFIX + len(base.KINDS), NUM_START))
                  + tuple(NUM_START + base.ENTITY_NUMBERS.index(key) for key in DYNAMIC_NUMBERS)
                  + tuple(range(NUM_START + len(base.ENTITY_NUMBERS) + 2 * len(base.STATS) + len(base.BOND_IDS),
                                NUM_START + len(base.ENTITY_NUMBERS) + 2 * len(base.STATS) + len(base.BOND_IDS) + len(base.DIRECTIONS))))
ENTITY_DIM = 1 + len(ENTITY_COLUMNS)
# Existing bond rules are static; per-state bond values are just the first five.
ENTITY_START = base.GLOBAL_DIM
BOND_START = ENTITY_START + base.MAX_ENTITIES * ENTITY_DIM
GRID_START = BOND_START + len(base.BOND_IDS) * 5
ENEMY_START = GRID_START + len(base.CELLS) * base.GRID_DIM
CONTROL_START = ENEMY_START + base.MAX_ENEMIES * 12
OBS_DIM = CONTROL_START + base.CONTROL_DIM
ACTION_DIM = base.ACTION_DIM


def definition_key(row):
    if row["kind"] == "effect":
        return tuple(row["definition"]) if row["definition"] else ("effect", "unresolved")
    return row["kind"], row["id"]
