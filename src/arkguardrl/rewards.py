"""Public native solo progress/CORE objective, on decision intervals.

Formula follows the repository's legacy solo-progress-core-v2 research objective.
This is a new native observation/clock contract, not legacy checkpoint parity.
"""
import math

from .contracts import fingerprint
from .observation import validate_state

REWARD_SCHEMA = "arkguardrl/native-solo-progress-core/v1"
CORE_BONDS = frozenset(("yanShip", "sargonShip", "victoriaShip", "kjeragShip",
                       "lateranoShip", "egirShip", "siracusaShip", "kazimierzShip"))


def metadata():
    return {"schema": REWARD_SCHEMA, "gamma": 1.0, "discount_unit": "decision",
            "win": 1.0, "loss": -1.0, "progress_weight": 0.5,
            "progress_denominator": "public bossRound captured at reset",
            "potential_weight": 0.1, "potential_scale": 100.0,
            "core_bonds": sorted(CORE_BONDS),
            "potential": "0 at personal terminal; 0.1*tanh(max active non-off CORE layers/100)",
            "shaping": "Phi(after)-Phi(before); automatic advances belong to preceding decision"}


def _core(state):
    if state["personalEnded"]:
        return 0.0
    layers = [0.0]
    bonds = state["own"].get("bonds")
    if not isinstance(bonds, list):
        raise ValueError("Explicit public bond list required")
    for bond in bonds:
        if not isinstance(bond, dict):
            raise ValueError("Malformed public bond")
        if bond.get("bondId") in CORE_BONDS and bond.get("active") and not bond.get("off"):
            value = bond.get("layers")
            if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
                raise ValueError("Active CORE layers must be finite and nonnegative")
            layers.append(value)
    return 0.1 * math.tanh(max(layers) / 100.0)


class NativeProgressCore:
    """Stateful one-episode reward adapter; technical failures never become rewards.

    Call reset once, then step on every executed decision including rejection.
    Budget truncation is not a loss: its live potential and bootstrap remain.
    A dead personal endpoint needs no later settlement; an alive game endpoint
    requires the engine's unambiguous public solo result. No reward for spending,
    attempted upgrades, request count, or inferred future combat is added.
    """

    def __init__(self):
        self._last = None
        self._ended = False

    def _progress(self, state):
        validate_state(state)
        public, own = state["public"], state["own"]
        if (public.get("bossRound") != self.boss_round
                or public.get("difficulty") != self.difficulty
                or public.get("modeId") != self.mode_id
                or own.get("playerId") != self.player_id):
            raise ValueError("Reward episode identity changed")
        players = public.get("players")
        if (not isinstance(players, list) or len(players) != 1
                or players[0].get("isBot") is not False):
            raise ValueError("Exactly one native non-bot solo player required")
        round_ = state["round"]
        if type(round_) is not int or round_ < 0:
            raise ValueError("Nonnegative public round required")
        passed = min(max(0, round_ - 1), self.boss_round - 1)
        won = False
        if state["gameEnded"]:
            outcome = state.get("outcome")
            if (not isinstance(outcome, dict) or outcome.get("errors") != 0
                    or outcome.get("modeId") != self.mode_id
                    or outcome.get("difficulty") != self.difficulty
                    or type(outcome.get("victory")) is not bool):
                raise ValueError("Clean explicit native outcome required")
            settled = outcome.get("players")
            if not isinstance(settled, list) or len(settled) != 1:
                raise ValueError("Exactly one native player result required")
            player = settled[0]
            if (player.get("playerId") != self.player_id
                    or type(player.get("victory")) is not bool
                    or player.get("alive") is not own["alive"]
                    or type(player.get("roundsPassed")) is not int
                    or player["roundsPassed"] < 0
                    or player["victory"] != (outcome["victory"] and own["alive"])):
                raise ValueError("Inconsistent native solo outcome")
            won, passed = player["victory"], player["roundsPassed"]
        elif state["personalEnded"] and own["alive"]:
            raise ValueError("Alive personal endpoint without final outcome is ambiguous")
        return min(passed / self.boss_round, 1.0), won

    def reset(self, state):
        if self._last is not None:
            raise RuntimeError("Use a fresh reward adapter per episode")
        validate_state(state)
        self.boss_round = state["public"].get("bossRound")
        if type(self.boss_round) is not int or self.boss_round < 1:
            raise ValueError("Positive public bossRound required")
        self.difficulty = state["public"].get("difficulty")
        if self.difficulty not in ("FUNNY", "NORMAL", "HARD", "ABYSS"):
            raise ValueError("Known native solo difficulty required")
        self.mode_id = "mode_single_" + self.difficulty.lower()
        self.player_id = state["own"]["playerId"]
        if state["personalEnded"]:
            raise ValueError("Reset requires a live native episode")
        self.progress, _ = self._progress(state)
        self.potential = _core(state)
        self._last = fingerprint(state)

    def step(self, before, after):
        if self._last is None or self._ended or fingerprint(before) != self._last:
            raise ValueError("Disconnected or already-ended reward episode")
        progress, won = self._progress(after)
        if progress < self.progress:
            raise ValueError("Public rounds-passed progress regressed")
        potential = _core(after)
        terminated = after["personalEnded"]
        components = {"outcome": (1.0 if won else -1.0) if terminated else 0.0,
                      "progress": 0.5 * (progress - self.progress),
                      "core_shaping": potential - self.potential}
        reward = math.fsum(components.values())
        self.progress, self.potential = progress, potential
        self._last, self._ended = fingerprint(after), terminated
        return {"reward": reward, "components": components, "progress": progress,
                "terminated": terminated, "victory": won if terminated else None,
                "schema": REWARD_SCHEMA}
