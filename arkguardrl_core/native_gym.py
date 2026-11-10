"""Desperation-only Gym adapter for the in-process Rust simulator."""
import gymnasium as gym
from arkguardrl.catalog import PublicCatalog
from .gym_env import ArkGuardEnv
from .native_env import NativePool, NativeDecisionEnv


class ArkGuardNativeEnv(ArkGuardEnv):
    """Existing dynamic feature shapes and candidate-mask semantics, native execution.

    core selects band_amiya, band_sarkazb, band_chen, or band_sciurus.
    No core-selection actions are added. native_library is a development-build
    path; omit it when arkguardrl_core._native is installed.
    """
    def __init__(self, data_root, *, core, banned=(), native_library=None, diy=None,
                 tick_budget=4096, numeric_observations=True, native_ledger=True, **kwargs):
        if type(tick_budget) is not int or not 1 <= tick_budget <= 108000:
            raise ValueError("tick_budget must be an integer in 1..108000")
        if kwargs.get("difficulty", "ABYSS") != "ABYSS":
            raise ValueError("Only Desperation (ABYSS) is maintained")
        if kwargs.get("expected_data_sha256") is not None:
            raise ValueError("Native data is configured by directory, without a source identity gate")
        if type(numeric_observations) is not bool:
            raise ValueError('numeric_observations must be boolean')
        if type(native_ledger) is not bool:
            raise ValueError('native_ledger must be boolean')
        self.native_ledger = native_ledger
        self.numeric_observations = numeric_observations
        self.core, self.banned = core, tuple(banned)
        self.diy = diy
        self.native_library, self.tick_budget = native_library, tick_budget
        super().__init__(data_root, **kwargs)

    def _make_pool(self):
        return NativePool(self.engine_root, self.core, self.banned, self.native_library, self.diy)

    def _load_catalog(self):
        return PublicCatalog.from_native(self._pool.catalog)

    def _make_environment(self):
        environment = NativeDecisionEnv
        if self.native_ledger:
            from .native_ledger import NativeLedgerEnv
            environment = NativeLedgerEnv
        return environment(self._pool, max_decisions=self.max_decisions,
                                 max_advances=self.max_advances, tick_budget=self.tick_budget,
                                 policy_catalog=self.catalog if self.numeric_observations else None)

    def _make_decision_session(self, environment):
        if self.native_ledger:
            from .native_ledger import NativeLedgerSession
            return NativeLedgerSession(environment, self.catalog)
        return super()._make_decision_session(environment)

    def _make_reward(self):
        if self.native_ledger:
            from .native_ledger import NativeLedgerReward
            return NativeLedgerReward(self._session.environment)
        return super()._make_reward()


gym.register("ArkGuardRL-v1", entry_point="arkguardrl_core.native_gym:ArkGuardNativeEnv")
