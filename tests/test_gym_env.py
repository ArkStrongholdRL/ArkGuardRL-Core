"""Native integration checks: set ARKGUARD_ENGINE_ROOT to the external game checkout."""
from copy import deepcopy
from contextlib import closing
from functools import partial
import os
import unittest

import gymnasium as gym
import numpy as np
from gymnasium.utils.env_checker import check_reset_seed_determinism
from gymnasium.utils.passive_env_checker import env_step_passive_checker

from arkguardrl_core.gym_env import ArkGuardEnv

ENGINE = os.environ.get('ARKGUARD_ENGINE_ROOT')


def equal_observations(test, first, second):
    test.assertEqual(first.keys(), second.keys())
    for key in first:
        np.testing.assert_array_equal(first[key], second[key])


@unittest.skipUnless(ENGINE, 'Set ARKGUARD_ENGINE_ROOT for native integration tests')
class GymEnvTests(unittest.TestCase):
    def test_seed_ownership_and_native_reuse(self):
        with ArkGuardEnv(ENGINE) as env:
            check_reset_seed_determinism(env)
            first, info = env.reset(seed=0)
            saved = deepcopy(first)
            pid = env._pool.session._process.pid
            result = env_step_passive_checker(env, env.action_space.sample(mask=first['action_mask']))
            self.assertTrue(env.observation_space.contains(result[0]))
            equal_observations(self, saved, first)
            again, info_again = env.reset(seed=0)
            equal_observations(self, saved, again)
            self.assertEqual(info, info_again)
            self.assertEqual(pid, env._pool.session._process.pid)
            continued, info_continued = env.reset()
            env.reset(seed=0)
            repeated, info_repeated = env.reset()
            equal_observations(self, continued, repeated)
            self.assertEqual(info_continued, info_repeated)
            env.reset(seed=2**80)
        env.close()

    def test_mask_rejection_does_not_advance(self):
        with ArkGuardEnv(ENGINE) as env:
            observation, info = env.reset(seed=42)
            revision = env._result.observation['revision']
            for action in (info['candidate_count'], -1, 0.5, True):
                with self.assertRaises(gym.error.InvalidAction):
                    env.step(action)
                self.assertEqual(env._result.observation['revision'], revision)
            env.step(env.action_space.sample(mask=observation['action_mask']))

    def test_truncation_preserves_actual_successor(self):
        for limits, reason in (({'max_decisions': 1}, 'decision-budget'),
                               ({'wall_seconds': 1e-9}, 'wall-budget')):
            with ArkGuardEnv(ENGINE, **limits) as env:
                env.reset(seed=42)
                observation, reward, terminated, truncated, info = env.step(0)
                self.assertFalse(terminated)
                self.assertTrue(truncated)
                self.assertEqual(reward, 0.)
                self.assertEqual(info['reason'], reason)
                self.assertEqual(info['phase'], 'BAND_DRAFT')
                self.assertGreater(observation['action_mask'].sum(), 0)
                with self.assertRaises(gym.error.ResetNeeded):
                    env.step(0)
                env.reset(seed=42)

    def test_candidate_overflow_is_not_silently_clipped(self):
        with ArkGuardEnv(ENGINE, max_candidates=1) as env:
            env.reset(seed=42)
            with self.assertRaisesRegex(ValueError, 'no actions were dropped'):
                env.step(0)
            self.assertIsNone(env._pool.session)

    def test_natural_terminal_observation(self):
        rng = np.random.default_rng(np.random.SeedSequence(123).spawn(1)[0])
        with ArkGuardEnv(ENGINE) as env:
            observation, _ = env.reset(seed=42)
            for _ in range(1024):
                action = int(rng.integers(observation['action_mask'].sum()))
                observation, _, terminated, truncated, _ = env.step(action)
                if terminated or truncated:
                    break
            self.assertTrue(terminated)
            self.assertFalse(truncated)
            self.assertTrue(env._result.observation['personalEnded'])
            self.assertEqual(observation['action_mask'].sum(), 0)
            self.assertTrue(env.observation_space.contains(observation))
            self.assertIsNone(env._session)
            with self.assertRaises(gym.error.ResetNeeded):
                env.step(0)
            env.reset(seed=42)

    def test_matches_native_transition_and_reward(self):
        from arkguardrl.decision_session import DecisionSession
        from arkguardrl.rewards import NativeProgressCore
        from arkguardrl_core.routes.session import RouteEnv
        with ArkGuardEnv(ENGINE, max_decisions=16) as env:
            observation, info = env.reset(seed=42)
            native = RouteEnv(ENGINE, deadline=float('inf'), max_decisions=16)
            with DecisionSession(native, env.catalog) as session:
                result = session.reset(seed=info['engine_seed'], difficulty='ABYSS')
                reward = NativeProgressCore()
                reward.reset(result.observation)
                for _ in range(16):
                    decision = session.decision()
                    count = len(decision.actions)
                    np.testing.assert_array_equal(observation['observation'], np.asarray(decision.observation_features, np.float32))
                    np.testing.assert_array_equal(observation['candidates'][:count], np.asarray(decision.candidate_features, np.float32))
                    self.assertEqual(int(observation['action_mask'].sum()), count)
                    before = result.observation
                    result = session.step(decision.actions[0])
                    expected_reward = reward.step(before, result.observation)['reward']
                    observation, actual_reward, terminated, truncated, info = env.step(0)
                    self.assertEqual((actual_reward, terminated, truncated, info['accepted']),
                                     (expected_reward, result.terminated, result.truncated, result.accepted))
                    if terminated or truncated:
                        break

    def test_sync_async_shared_memory_and_partial_reset(self):
        factories = [partial(ArkGuardEnv, ENGINE, max_decisions=limit) for limit in (2, 3)]
        mode = gym.vector.AutoresetMode.DISABLED
        with closing(gym.vector.SyncVectorEnv(factories, autoreset_mode=mode)) as sync, \
             closing(gym.vector.AsyncVectorEnv(factories, context='spawn', daemon=False,
                                       shared_memory=True, autoreset_mode=mode)) as async_:
            first, _ = sync.reset(seed=[42, 43])
            second, _ = async_.reset(seed=[42, 43])
            rng = np.random.default_rng(7)
            for _ in range(8):
                equal_observations(self, first, second)
                saved = deepcopy(second)
                actions = np.asarray([rng.integers(mask.sum()) for mask in first['action_mask']])
                a, b = sync.step(actions), async_.step(actions)
                equal_observations(self, saved, second)
                first, second = a[0], b[0]
                for x, y in zip(a[1:4], b[1:4]):
                    np.testing.assert_array_equal(x, y)
                equal_observations(self, first, second)
                done = a[2] | a[3]
                if done.any():
                    final = deepcopy(second)
                    first, _ = sync.reset(options={'reset_mask': done.copy()})
                    second, _ = async_.reset(options={'reset_mask': done.copy()})
                    for key in second:
                        np.testing.assert_array_equal(second[key][~done], final[key][~done])


if __name__ == '__main__':
    unittest.main()
