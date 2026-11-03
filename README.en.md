<p align="center">
  <img src="docs/assets/hero.svg" alt="ArkGuardRL-Core — a Gymnasium research environment for Stronghold Protocol" width="100%">
</p>

<p align="center">
  <a href="README.md">简体中文</a> · <strong>English</strong>
</p>

<p align="center">
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/Python-3.10%2B-3776AB?style=flat-square&logo=python&logoColor=white" alt="Python 3.10+"></a>
  <a href="https://nodejs.org/"><img src="https://img.shields.io/badge/Node.js-22%2B-339933?style=flat-square&logo=nodedotjs&logoColor=white" alt="Node.js 22+"></a>
  <a href="https://gymnasium.farama.org/"><img src="https://img.shields.io/badge/Gymnasium-1.1%2B-00897B?style=flat-square" alt="Gymnasium 1.1+"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0--or--later-476582?style=flat-square" alt="GPL-3.0-or-later"></a>
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> · <a href="#performance">Performance</a> · <a href="#environment-api">Environment API</a> · <a href="docs/NATIVE-033.md">Native Rust progress</a>
</p>

**Bring Stronghold Protocol into your reinforcement learning experiments.** ArkGuardRL-Core
provides a Gymnasium interface to the unofficial
[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) simulator, exposing
game decisions as structured observations, candidate actions, and action masks.

| Connect experiments | Inspect decisions | Scale collection |
| --- | --- | --- |
| `gym.make("ArkGuardRL-v0")` with NumPy dictionary observations | Engine seeds, reward components, and action acceptance | `SyncVectorEnv` and shared-memory `AsyncVectorEnv` |
| Rust/PyO3 action and observation kernels | Separate termination and budget truncation | Environment-only benchmarks without Torch or a trainer |

The current interface covers solo games with default loadouts, using **the app 0.3.1 engine (Node.js) for game execution and Rust for action and feature computation**.
A complete Rust environment is [in development](docs/NATIVE-033.md). Core owns the environment
and runtime; policy networks and training belong in downstream research projects.

## Quick start

### 1. Install the environment package

You need **Python 3.10+, Node.js 22+, Git, Rust/Cargo, and system linker tools**.
A Python virtual environment is recommended. Installation builds the Rust/PyO3 extension in release mode.

```sh
git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
cd ArkGuardRL-Core
python -m pip install -e '.[gym]'
```

### 2. Prepare the external engine

From this repository, clone the engine into a sibling directory:

```sh
git clone https://github.com/sganggs/Stronghold-Protocol.git ../Stronghold-Protocol
git -C ../Stronghold-Protocol checkout c9d804f470b8d7422ae04eb66faf671cef21ad52
```

`ArkGuardRL-v0` requires this exact revision (app 0.3.1), a **clean Git working tree**, and
the complete `data/*.json` files. This checkout includes the required JSON; Gym starts the engine
in the background without a web server or artwork downloads. Set `engine_root` to the engine repository root. See the
[upstream documentation](https://github.com/sganggs/Stronghold-Protocol/tree/c9d804f470b8d7422ae04eb66faf671cef21ad52)
for engine and resource information, and [NOTICE](NOTICE.md) for this project's resource boundaries.

### 3. Run a minimal example

Run this example from the Core repository. It samples masked actions at random; the default difficulty is `ABYSS`:

```python
import gymnasium as gym
import arkguardrl_core.gym_env  # Registers ArkGuardRL-v0.

with gym.make("ArkGuardRL-v0", engine_root="../Stronghold-Protocol") as env:
    env.action_space.seed(42)
    obs, info = env.reset(seed=42)

    while True:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        if terminated or truncated:
            break
```

## Performance

Recorded environment throughput includes observation construction, reward calculation,
Gym communication, and in-loop resets:

![Environment throughput: six measurements and medians for one synchronous and four asynchronous environments](docs/assets/benchmark-throughput.svg)

**Apple M5 · 16 GiB · November 2, 2026.** Uniform sampling over valid candidates, without model
inference or training. Each environment runs 64 warm-up steps and 1,024 measured decisions;
each variant has six runs. Short desktop measurements vary with load; these changes do not establish a training speedup.

→ **[Results, measurement scope, and reproduction commands](benchmarks/README.md)** ·
[Raw records](benchmarks/env-hotspots-20261102.json) · [Optimization notes](benchmarks/HOTSPOTS-20261102.md)

## Environment API

An action is a **row index into the current candidate set**; a row's meaning changes with the state.
Observations are dictionaries of NumPy arrays:

| Field | Shape / type | Content |
| --- | --- | --- |
| `observation` | `(4784,) float32` | Dynamic state features |
| `candidates` | `(max_candidates, 41) float32` | Candidate action features; zero padding |
| `schedules` / `experts` | `(max_candidates,) int64` | Candidate groups |
| `action_mask` | `(max_candidates,) int8` | Current valid candidate rows |

Candidates are constructed from public information; check `info["accepted"]` for the execution result.
The default candidate capacity is **16,384**. Overflow raises an error; selecting padding or an
out-of-range index raises `InvalidAction` without advancing the game.

<details>
<summary><strong>Seeds, rewards, episode boundaries, and vector execution</strong></summary>

- **Seeds:** the same Gym seed reproduces the environment. The derived engine seed is available
  in `info["engine_seed"]`. `reset(seed=None)` continues the RNG stream; action sampling has its own RNG.
- **Rewards:** `NativeProgressCore` computes rewards; components are returned in `info["reward_components"]`.
- **Episode boundaries:** natural endings return `terminated`. Exhausting `max_decisions`,
  `max_advances`, or `wall_seconds` returns `truncated`, a reason in `info["reason"]`, and the actual
  successor observation. Engine and transport failures raise exceptions.
- **Actions:** the mask identifies candidate rows that can be submitted; the engine may still reject
  execution. Unmasked random actions from a generic `check_env()` can select padding; sample with the mask.
- **Configuration:** set difficulty and budgets at construction. Single-environment `reset(options=...)`
  accepts no extra configuration. Read static definitions after reset via `env.unwrapped.catalog.definitions()`.
- **Vector execution:** this repository's async benchmark sets `context="spawn"` and `shared_memory=True`. Custom entry points need
  `if __name__ == "__main__"`. Each environment owns one Node process; default padded observations
  use **2,984,640 bytes per environment**.

</details>

## Documentation and development

The detailed development and benchmark notes are currently in Chinese.

| Guide | Contents |
| --- | --- |
| [Performance benchmarks](benchmarks/README.md) | Latest measurements, historical comparisons, reproduction, and timing scope |
| [Rust action and feature kernels](docs/RUST-ROUTES.md) | Kernel responsibilities and ABI in the installed package |
| [Complete Rust environment migration](docs/NATIVE-033.md) | Implementation scope, validation, and remaining work against app 0.3.3 |
| [Attribution and resource boundaries](NOTICE.md) | Code provenance, external engine, and resource ownership |

Environment integration tests require the external engine and run actual games:

```sh
ARKGUARD_ENGINE_ROOT=/path/to/Stronghold-Protocol \
  python -m unittest discover -s tests -p test_gym_env.py -v
```

Contributions are welcome: [report interface issues](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/issues),
share reproducible environment measurements, or help with the native migration.

## Acknowledgments and license

Thanks to [Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) for the upstream simulator.
This is an unofficial research framework. Code is licensed under **GPL-3.0-or-later**;
game assets, data, model weights, and trajectories are not included in this repository.
See [NOTICE](NOTICE.md) and [LICENSE](LICENSE) for provenance and licensing details.
