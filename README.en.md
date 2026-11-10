<a name="top"></a>

<p align="center">
  <img src="docs/assets/hero.svg" alt="ArkGuardRL-Core — Stronghold Protocol research environment" width="100%">
</p>

<h3 align="center">Stronghold Protocol · Reinforcement Learning Environment</h3>

<p align="center"><a href="README.md">简体中文</a> · <strong>English</strong></p>

<p align="center">Run complete solo games, connect a learning policy, and inspect each decision.</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0--or--later-476582?style=flat-square" alt="GPL-3.0-or-later"></a>
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/Python-3.12-3776AB?style=flat-square&amp;logo=python&amp;logoColor=white" alt="Python 3.12"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-PyO3-dea584?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust and PyO3"></a>
  <a href="https://gymnasium.farama.org/"><img src="https://img.shields.io/badge/Gymnasium-1.1%2B-00897B?style=flat-square" alt="Gymnasium 1.1+"></a>
</p>

<p align="center"><a href="#quick-start">Quick start</a> · <a href="#environment-api">Environment API</a> · <a href="#performance">Performance</a> · <a href="#documentation">Documentation</a> · <a href="#license-and-acknowledgments">License</a></p>

## Overview

ArkGuardRL-Core exposes the purchasing, team-building, and deployment decisions of Stronghold Protocol through Gymnasium.
Public observations, candidate actions, rewards, and episode boundaries let a policy drive complete games through `reset` and `step`.
Game rules follow the Stronghold Protocol simulator. [Upstream project](https://github.com/sganggs/Stronghold-Protocol)

| Core runtime | Policy research |
| --- | --- |
| Game execution, public observations, candidate enumeration, and numeric encoding | Model representations, policy networks, and learning algorithms |
| Seeded reproduction, decision budgets, and vector collection | Trajectory collection, training, and policy evaluation |
| Maintained in this repository | Maintained in [ArkGuardRL](https://github.com/HarcoChen/ArkGuardRL) |

Maintenance targets solo Desperation mode (`mode_single_abyss`). This branch's quick start uses **the 0.3.1 Node engine and Rust feature kernels**.
The 0.3.3 native runtime, Rust slots, and public spatial APIs are developed in PR #4. [Native runtime PR](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/pull/4)

## Quick start

Use Python 3.12, Node.js 22+, Git, and Rust/Cargo. Installation compiles a PyO3 extension and requires system linker tools.

```sh
git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
cd ArkGuardRL-Core
python -m pip install -e '.[gym]'

git clone https://github.com/sganggs/Stronghold-Protocol.git ../Stronghold-Protocol
git -C ../Stronghold-Protocol checkout c9d804f470b8d7422ae04eb66faf671cef21ad52
```

Keep the external engine checkout clean with all `data/*.json` files present. Set `engine_root` to its repository root.
This revision includes the required JSON files. The environment starts the engine in the background.

### Run a game

```python
import gymnasium as gym
import arkguardrl_core.gym_env

with gym.make("ArkGuardRL-v0", engine_root="../Stronghold-Protocol") as env:
    env.action_space.seed(42)
    obs, info = env.reset(seed=42)
    while True:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        if terminated or truncated:
            break
```

The example samples candidates using the mask. Replace `action` with the candidate row selected by your policy.

## Environment API

| Interface | Use |
| --- | --- |
| `ArkGuardRL-v0` | A single-environment Gymnasium decision loop |
| `SyncVectorEnv` / `AsyncVectorEnv` | Vector collection with an independent session per environment |
| `env.unwrapped.catalog.definitions()` | Read public static definitions |
| `info` | Inspect action acceptance, reward components, engine seeds, and episode reasons |

Observations are dictionaries of NumPy arrays. Action indices refer to the current state's candidate rows. The default capacity is 16,384.

| Field | Shape and type |
| --- | --- |
| `observation` | `(4784,) float32` |
| `candidates` | `(max_candidates, 41) float32` |
| `schedules` / `experts` | `(max_candidates,) int64` |
| `action_mask` | `(max_candidates,) int8` |

<details>
<summary>Actions, seeds, and rollout semantics</summary>

- Candidates are constructed from public information. Check `info["accepted"]` for execution results. Padding or out-of-range indices raise `InvalidAction`. Candidate overflow raises an error.
- The same Gym seed reproduces the environment. `info["engine_seed"]` exposes the derived seed. `reset(seed=None)` continues the RNG stream. Action sampling uses a separate RNG.
- `NativeProgressCore` computes rewards. Components appear in `info["reward_components"]`.
- Natural endings return `terminated`. Exhausting `max_decisions`, `max_advances`, or `wall_seconds` returns `truncated`, the actual successor observation, and `info["reason"]`. Engine or transport failures raise exceptions.
- Set difficulty and budgets at construction. Single-environment `reset(options=...)` accepts an empty configuration.
- Async collection with `spawn` requires an `if __name__ == "__main__"` entry point. Each v0 environment owns an independent Node process.

</details>

## Performance

| Configuration | Decisions per second |
| --- | ---: |
| Gym Sync, 1 environment | 494.75 |
| Gym Async, shared memory, 4 environments | 937.19 |

Recorded on November 2, 2026, with an Apple M5 / 16 GiB and the Node 0.3.1 engine plus Rust feature kernels.
Each environment runs 64 warm-up steps and 1,024 measured steps. Values are medians of six measurements.
Timing includes observations, rewards, Gym communication, and in-loop resets, with masked random actions.
Raw records, measurement plots, and reproduction commands are available in the benchmark guide. [Performance benchmarks](benchmarks/README.md)

## Documentation

Detailed development notes are currently in Chinese.

| Guide | Contents |
| --- | --- |
| [Performance benchmarks](benchmarks/README.md) | Measurements, timing scope, and commands |
| [Rust feature kernels](docs/RUST-ROUTES.md) | Action and observation encoding responsibilities and ABI |
| [Native migration notes](docs/NATIVE-033.md) | Development history for the 0.3.3 engine |
| [Issue tracker](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/issues) | Interface issues and reproducible reports |

## License and acknowledgments

Code is licensed under GPL-3.0-or-later. Stronghold Protocol provides the upstream simulator. See the license and attribution notes for game assets and external data. [LICENSE](LICENSE) · [NOTICE](NOTICE.md)

The landing-page layout draws on AutoTavern. The banner is an original schematic illustration. [AutoTavern](https://github.com/MCG-NJU/AutoTavern)

<p align="center"><a href="#top">Back to top</a></p>
