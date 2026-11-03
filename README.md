<p align="center">
  <img src="docs/assets/hero.svg" alt="ArkGuardRL-Core — Stronghold Protocol 的 Gymnasium 研究环境" width="100%">
</p>

<p align="center">
  <strong>简体中文</strong> · <a href="README.en.md">English</a>
</p>

<p align="center">
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/Python-3.10%2B-3776AB?style=flat-square&logo=python&logoColor=white" alt="Python 3.10+"></a>
  <a href="https://nodejs.org/"><img src="https://img.shields.io/badge/Node.js-22%2B-339933?style=flat-square&logo=nodedotjs&logoColor=white" alt="Node.js 22+"></a>
  <a href="https://gymnasium.farama.org/"><img src="https://img.shields.io/badge/Gymnasium-1.1%2B-00897B?style=flat-square" alt="Gymnasium 1.1+"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0--or--later-476582?style=flat-square" alt="GPL-3.0-or-later"></a>
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> · <a href="#性能速览">性能数据</a> · <a href="#环境接口">环境接口</a> · <a href="docs/NATIVE-033.md">Rust 迁移进度</a>
</p>

**把《卫戍协议：盟约》接入强化学习实验。** ArkGuardRL-Core 为非官方
[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 提供 Gymnasium 接口，
将游戏决策表示为结构化观测、候选动作与动作掩码。

| 接入实验 | 检查决策 | 扩展采样 |
| --- | --- | --- |
| `gym.make("ArkGuardRL-v0")`，NumPy 字典观测 | 引擎种子、奖励分项、动作接受结果 | `SyncVectorEnv` / 共享内存 `AsyncVectorEnv` |
| Rust/PyO3 动作与观测内核 | 区分自然结束与预算截断 | 独立环境 benchmark，无需 Torch 或训练器 |

当前接口覆盖单人、默认配置的游戏流程，由 **app 0.3.1 引擎（Node.js）执行游戏，Rust 加速动作与特征计算**。
完整 Rust 环境正在[迁移](docs/NATIVE-033.md)。Core 负责环境与运行时；策略网络和训练由上层研究项目负责。

## 快速开始

### 1. 安装环境包

准备 **Python 3.10+、Node.js 22+、Git、Rust/Cargo 与系统链接工具**。
建议在 Python 虚拟环境中安装；安装过程会编译 Rust/PyO3 release 扩展。

```sh
git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
cd ArkGuardRL-Core
python -m pip install -e '.[gym]'
```

### 2. 准备外部引擎

在本仓库目录执行，将引擎放到同级目录：

```sh
git clone https://github.com/sganggs/Stronghold-Protocol.git ../Stronghold-Protocol
git -C ../Stronghold-Protocol checkout c9d804f470b8d7422ae04eb66faf671cef21ad52
```

`ArkGuardRL-v0` 使用这个固定提交（app 0.3.1），需要**干净的 Git 工作区**及完整的
`data/*.json`。该 checkout 已包含所需 JSON；Gym 在后台启动引擎，无需网页服务器或美术资源。
`engine_root` 指向引擎仓库根目录；引擎与资源说明见
[上游文档](https://github.com/sganggs/Stronghold-Protocol/tree/c9d804f470b8d7422ae04eb66faf671cef21ad52)。
本仓库单独分发环境代码，资源边界见 [NOTICE](NOTICE.md)。

### 3. 运行最小示例

在本仓库目录运行以下示例。示例按掩码随机采样，默认难度为 `ABYSS`：

```python
import gymnasium as gym
import arkguardrl_core.gym_env  # 注册 ArkGuardRL-v0

with gym.make("ArkGuardRL-v0", engine_root="../Stronghold-Protocol") as env:
    env.action_space.seed(42)
    obs, info = env.reset(seed=42)

    while True:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        if terminated or truncated:
            break
```

## 性能速览

已有测量记录中的环境吞吐，包含观测构造、奖励计算、Gym 通信及循环内 reset：

![环境吞吐：单环境与四环境并行的六次测量及中位数](docs/assets/benchmark-throughput.svg)

**Apple M5 · 16 GiB · 2026-11-02**。随机采样有效候选，无模型推理或训练；
每环境预热 64 步、测量 1,024 步，每个版本各 6 次。
短时桌面测量存在波动，图中变化不等同于训练提速。

→ **[查看结果、测量边界与复现命令](benchmarks/README.md)** ·
[原始记录](benchmarks/env-hotspots-20261102.json) · [优化说明](benchmarks/HOTSPOTS-20261102.md)

## 环境接口

动作是**当前状态的候选行索引**，候选行的含义会随状态变化。观测为包含 NumPy 数组的字典：

| 字段 | 形状 / 类型 | 内容 |
| --- | --- | --- |
| `observation` | `(4784,) float32` | 动态状态特征 |
| `candidates` | `(max_candidates, 41) float32` | 候选动作特征，padding 为 0 |
| `schedules` / `experts` | `(max_candidates,) int64` | 候选分组 |
| `action_mask` | `(max_candidates,) int8` | 当前有效候选行 |

候选由公开信息构造，执行结果通过 `info["accepted"]` 检查。
默认候选容量为 **16,384**，超限报错；padding 或越界索引抛出 `InvalidAction`，且不推进游戏。

<details>
<summary><strong>种子、奖励、结束条件与并行语义</strong></summary>

- **种子**：相同 Gym 种子可复现环境；实际引擎种子见 `info["engine_seed"]`。
  `reset(seed=None)` 延续 RNG 流，动作采样使用单独的随机源。
- **奖励**：由 `NativeProgressCore` 计算，分项见 `info["reward_components"]`。
- **结束**：自然结束返回 `terminated`；`max_decisions`、`max_advances` 或 `wall_seconds`
  耗尽返回 `truncated`，原因见 `info["reason"]`，同时保留真实后继观测。引擎与传输错误抛异常。
- **动作**：掩码标记可提交的候选行，引擎仍可能拒绝执行。通用 `check_env()` 的无掩码随机动作
  可能选中 padding，请按掩码采样。
- **配置**：难度与预算在构造时设置；单环境 `reset(options=...)` 不接受额外配置。
  静态定义在 reset 后通过 `env.unwrapped.catalog.definitions()` 读取。
- **并行**：本仓库的异步基准使用 `context="spawn"` 和 `shared_memory=True`；自定义入口需要 `if __name__ == "__main__"`。
  每个 env 独占一个 Node 进程；默认填充观测为 **2,984,640 字节/env**。

</details>

## 文档与开发

| 入口 | 内容 |
| --- | --- |
| [性能基准](benchmarks/README.md) | 最新记录、历史对照、复现命令与计时边界 |
| [Rust 动作与特征内核](docs/RUST-ROUTES.md) | 当前安装包的内核职责与 ABI |
| [完整 Rust 环境迁移](docs/NATIVE-033.md) | 对照 app 0.3.3 的实现范围、验证与待完成工作 |
| [来源与资源边界](NOTICE.md) | 代码来源、外部引擎与资源归属 |

环境集成测试需要外部引擎，会实际运行对局：

```sh
ARKGUARD_ENGINE_ROOT=/path/to/Stronghold-Protocol \
  python -m unittest discover -s tests -p test_gym_env.py -v
```

欢迎通过 [Issues](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/issues)
报告接口问题、提交可复现的性能测量，或参与原生迁移。

## 致谢与许可

感谢 [Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 提供上游模拟实现。
本项目是非官方研究框架，代码采用 **GPL-3.0-or-later**；游戏素材、数据、权重和轨迹不随仓库分发。
代码来源与外部资源许可详见 [NOTICE](NOTICE.md) 和 [LICENSE](LICENSE)。
