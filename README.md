# ArkGuardRL-Core

> **A Gymnasium-compatible reinforcement learning environment for Stronghold Protocol, with Rust-accelerated observation and action kernels.**

面向《卫戍协议：盟约》（Stronghold Protocol）的开源 RL 环境。将复杂的招募、经营、布阵和战斗流程封装为可复现的 Gymnasium 接口，方便研究 **long-horizon decision making**、**dynamic action spaces** 与 **strategy learning**。

[![Python](https://img.shields.io/badge/Python-3.10%2B-3776AB?style=flat-square&logo=python&logoColor=white)](https://www.python.org/)
[![Rust](https://img.shields.io/badge/Rust-PyO3%200.27-dea584?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Gymnasium](https://img.shields.io/badge/Gymnasium-%E2%89%A51.1-4B8BBE?style=flat-square)](https://gymnasium.farama.org/)
[![License](https://img.shields.io/badge/License-GPL--3.0--or--later-blue?style=flat-square)](LICENSE)
[![Status](https://img.shields.io/badge/Status-Pre--release-orange?style=flat-square)](docs/NATIVE-033.md)

**[Quick start](#quick-start--快速开始) · [Benchmarks](#benchmarks--性能测量) · [Architecture](#architecture--架构) · [Research](https://github.com/HarcoChen/ArkGuardRL)**

## Highlights · 项目特性

- **Standard RL interface.** 注册 Gymnasium 环境 \`ArkGuardRL-v0\`，提供 \`reset()\` / \`step()\`、奖励分项和明确的终止/截断语义。
- **State-dependent legal candidates.** 动作是当前候选列表的索引；完整候选、意图分组与 \`action_mask\` 显式提供，最多支持 16,384 个候选，不静默裁剪。
- **Rust-powered feature kernels.** 通过 Rust/PyO3 加速动作候选、动态观测和路线特征的部分计算；目前实际游戏运行仍依赖外部 Node 引擎。
- **Vectorized environments.** 支持 Gymnasium 的 \`SyncVectorEnv\` / \`AsyncVectorEnv\`（共享内存），可用于并行采样。
- **Reproducible measurement.** 固定引擎版本、外部数据身份、随机种子和可审计的环境性能测量；无需 PyTorch 即可运行环境 benchmark。

> **Scope:** This repository provides the **environment**, not a pretrained winning agent. BC/PPO experiments, strategy development and research reports live in [ArkGuardRL Research](https://github.com/HarcoChen/ArkGuardRL).

## Benchmarks · 性能测量

2026-11-02 在 **Apple M5 / 16 GiB** 上进行的无模型、有效候选均匀随机策略测量：

| Configuration | Median environment decisions/s | Workload |
| :-- | --: | :-- |
| Sync · 1 env | **494.75** | 1,024 decisions per environment |
| Async · 4 env · shared memory | **937.19** | 4 × 1,024 decisions per environment |

测量计入循环内 \`reset\` 与 Gymnasium 观测/通信开销，不包括模型推理、反向传播或轨迹训练。每个变体累计 6 次测量，预热 64 步；数据使用 Python 3.12.13 / Node 26.7.0 / Gymnasium 1.4.0。不同环境数和不同日期的结果不应直接当作单步加速倍率。

**[Full methodology & historical results](benchmarks/README.md) · [2026-11-02 measurements](benchmarks/HOTSPOTS-20261102.md) · [Raw JSON](benchmarks/env-hotspots-20261102.json)**

## Architecture · 架构

~~~mermaid
flowchart LR
    A["RL algorithm / Agent"] --> B["Gymnasium API · ArkGuardRL-v0"]
    B --> C["Public observation & action candidates"]
    C <--> D["Stronghold Protocol 0.3.1 · Node"]
    C <--> E["Rust / PyO3 feature kernels"]
    B --> F["Sync / Async vector environments"]
~~~

**当前可运行路径：** 外部 Stronghold Protocol **app 0.3.1 / Node** 负责游戏状态推进与战斗；Python 封装 Gymnasium 协议，Rust/PyO3 负责部分特征与动作计算。

**独立开发中的路径：** 基于上游 **app 0.3.3** 的纯 Rust 引擎正在迁移，**尚未成为可运行的完整 Gymnasium 环境**，也尚未达到性能验收要求。详见 [Native 0.3.3 migration status](docs/NATIVE-033.md)。这两条路径不可混为同一性能结果。

## Quick start · 快速开始

**Prerequisites:** Python 3.10+、Node.js 22+、用于构建 PyO3 扩展的 Rust toolchain，以及独立准备的上游游戏引擎及授权数据。

1. 获取本仓库，安装 Gymnasium 环境：

   ~~~bash
   git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
   cd ArkGuardRL-Core
   python -m pip install -e '.[gym]'
   ~~~

2. 单独准备 [Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 的固定版本 \`c9d804f470b8d7422ae04eb66faf671cef21ad52\`（app 0.3.1），及其运行依赖和授权数据。本仓库**不提供**游戏资源或安装好的引擎。

3. 使用 Gymnasium：

   ~~~python
   import gymnasium as gym
   import arkguardrl_core.gym_env  # Registers ArkGuardRL-v0

   with gym.make("ArkGuardRL-v0", engine_root="/path/to/engine") as env:
       obs, info = env.reset(seed=42)
       terminated = truncated = False
       while not (terminated or truncated):
           action = env.action_space.sample(mask=obs["action_mask"])
           obs, reward, terminated, truncated, info = env.step(action)
   ~~~

此示例仅演示环境交互，使用随机动作，**不是有效游戏策略**。生产实验应固定引擎数据摘要、策略、种子及资源上限。

### Observation and action contract

| Key | Shape / type | Meaning |
| :-- | :-- | :-- |
| \`observation\` | \`(4784,) float32\` | 当前动态公开观测 |
| \`candidates\` | \`(max_candidates, 41) float32\` | 当前候选动作特征（零填充） |
| \`schedules\`, \`experts\` | \`(max_candidates,) int64\` | 意图与专家分组 |
| \`action_mask\` | \`(max_candidates,) int8\` | 当前实际候选行掩码 |

动作空间为 \`Discrete(max_candidates)\`，动作整数指向**当前**候选行。掩码中的候选来自公开信息，并非引擎合法性的完备保证：引擎仍可能拒绝某项候选，回执见 \`info["accepted"]\`。选择 padding / 越界动作则抛出 \`InvalidAction\`，不会被替换为其他动作。

自然对局结束标记为 \`terminated\`；决策、推进次数或墙钟预算用尽标记为 \`truncated\` 并返回真实后继观测。技术故障抛异常，不伪装成失败对局。奖励分项可在 \`info["reward_components"]\` 中读取。

## Vector environments · 并行采样

\`SyncVectorEnv\` 与 \`AsyncVectorEnv\` 均受支持；Async 使用 \`spawn\` 与共享内存，应用入口需使用 \`if __name__ == "__main__"\` 防止子进程重复启动。每个环境拥有独立 Node 进程；默认填充观测约 **2.98 MB/env**。

~~~bash
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --warmup 64 --steps 1024 --repeats 3 \
  --out artifacts/gym-async-4.json
~~~

更多测量口径见 [benchmarks](benchmarks/README.md)。

## Documentation · 文档与验证

| Document | Contents |
| :-- | :-- |
| [Benchmarks](benchmarks/README.md) | 负载、硬件、吞吐与可复测命令 |
| [Rust feature kernels](docs/RUST-ROUTES.md) | PyO3 动作/观测/路线内核 |
| [Native 0.3.3 status](docs/NATIVE-033.md) | 纯 Rust 迁移范围、验证与未完成项 |

环境集成测试会真实调用引擎，需要显式指定其路径：

~~~bash
ARKGUARD_ENGINE_ROOT=/path/to/engine python -m unittest discover -s tests -p test_gym_env.py -v
~~~

## Attribution · 来源及许可

Code: **GPL-3.0-or-later**. The simulator is **unofficial** and is not affiliated with or endorsed by the game publisher. See [NOTICE](NOTICE.md) and [LICENSE](LICENSE) for upstream attribution and resource restrictions. Official/third-party game data, characters, assets, model weights and recorded trajectories are **not distributed** with this repository.
