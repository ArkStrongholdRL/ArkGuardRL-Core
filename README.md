<a name="top"></a>

<p align="center">
  <img src="docs/assets/hero.svg" alt="ArkGuardRL-Core — Stronghold Protocol research environment" width="100%">
</p>

<h3 align="center">卫戍协议 · 强化学习环境</h3>

<p align="center"><strong>简体中文</strong> · <a href="README.en.md">English</a></p>

<p align="center">复现完整单人对局，接入学习策略，检查每一步的决策结果。</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-GPL--3.0--or--later-476582?style=flat-square" alt="GPL-3.0-or-later"></a>
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/Python-3.12-3776AB?style=flat-square&amp;logo=python&amp;logoColor=white" alt="Python 3.12"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-PyO3-dea584?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust and PyO3"></a>
  <a href="https://gymnasium.farama.org/"><img src="https://img.shields.io/badge/Gymnasium-1.1%2B-00897B?style=flat-square" alt="Gymnasium 1.1+"></a>
</p>

<p align="center"><a href="#快速开始">快速开始</a> · <a href="#环境接口">环境接口</a> · <a href="#性能">性能</a> · <a href="#文档">文档</a> · <a href="#许可与致谢">许可与致谢</a></p>

## 概览

ArkGuardRL-Core 将《卫戍协议：盟约》的购买、构筑和部署决策接入 Gymnasium。
环境提供公开状态、候选动作、奖励与结束标志，策略代码可以通过标准 `reset`／`step` 循环驱动完整对局。
游戏规则基于 Stronghold Protocol 的模拟实现。[上游项目](https://github.com/sganggs/Stronghold-Protocol)

| Core 环境运行时 | 策略研究 |
| --- | --- |
| 游戏推进、公开观测、候选枚举与数值编码 | 模型表示、策略网络与学习算法 |
| 种子复现、预算控制与多环境采样 | 轨迹采集、训练与策略评估 |
| 在本仓库维护 | 在 [ArkGuardRL](https://github.com/HarcoChen/ArkGuardRL) 维护 |

维护范围为单人绝境模式（`mode_single_abyss`）。本分支的快速开始对应 **0.3.1 Node 引擎 + Rust 特征内核**。
0.3.3 原生运行时、Rust slots 和公开空间接口集中在 PR #4。[原生运行时 PR](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/pull/4)

## 快速开始

使用 Python 3.12、Node.js 22+、Git 和 Rust/Cargo。安装时编译 PyO3 扩展，需要系统链接工具。

```sh
git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
cd ArkGuardRL-Core
python -m pip install -e '.[gym]'

git clone https://github.com/sganggs/Stronghold-Protocol.git ../Stronghold-Protocol
git -C ../Stronghold-Protocol checkout c9d804f470b8d7422ae04eb66faf671cef21ad52
```

保持外部引擎 checkout 干净，并保留完整的 `data/*.json`。`engine_root` 指向其仓库根目录。
该版本包含所需 JSON，环境会在后台启动引擎。

### 运行一局

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

示例按掩码随机选择候选。将 `action` 替换为策略选出的候选行索引，即可接入自己的决策器。

## 环境接口

| 接口 | 用途 |
| --- | --- |
| `ArkGuardRL-v0` | 单环境 Gymnasium 决策循环 |
| `SyncVectorEnv`／`AsyncVectorEnv` | 多环境采样，每个环境持有独立会话 |
| `env.unwrapped.catalog.definitions()` | 读取公开静态定义 |
| `info` | 检查动作接受结果、奖励分项、引擎种子及结束原因 |

观测为 NumPy 字典。动作索引引用当前状态的候选行，候选容量默认 16,384。

| 字段 | 形状与类型 |
| --- | --- |
| `observation` | `(4784,) float32` |
| `candidates` | `(max_candidates, 41) float32` |
| `schedules`／`experts` | `(max_candidates,) int64` |
| `action_mask` | `(max_candidates,) int8` |

<details>
<summary>动作、种子与轨迹语义</summary>

- 候选根据公开信息构造，动作接受结果见 `info["accepted"]`。padding 或越界索引抛出 `InvalidAction`，候选超限报错。
- 相同 Gym 种子可复现环境，`info["engine_seed"]` 给出派生种子。`reset(seed=None)` 延续 RNG 流，动作采样使用独立 RNG。
- 奖励由 `NativeProgressCore` 计算，分项见 `info["reward_components"]`。
- 自然结束返回 `terminated`。`max_decisions`、`max_advances` 或 `wall_seconds` 耗尽时返回 `truncated`、真实后继观测和 `info["reason"]`。引擎或传输错误抛异常。
- 难度与预算在构造时设置。单环境 `reset(options=...)` 接受空配置。
- Async 使用 `spawn` 时，入口需要 `if __name__ == "__main__"`。当前 v0 每环境持有独立 Node 进程。

</details>

## 性能

| 配置 | 决策／秒 |
| --- | ---: |
| Gym Sync，1 环境 | 494.75 |
| Gym Async，共享内存，4 环境 | 937.19 |

2026-11-02，Apple M5／16 GiB，Node 0.3.1 + Rust 特征内核。每环境预热 64 步、测量 1,024 步，表中为六次测量中位数。
计时包含观测、奖励、Gym 通信与循环内 reset，动作按掩码随机采样。完整原始记录、分布图和复现命令见性能文档。[性能基准](benchmarks/README.md)

## 文档

| 文档 | 内容 |
| --- | --- |
| [性能基准](benchmarks/README.md) | 测量结果、计时边界与运行命令 |
| [Rust 特征内核](docs/RUST-ROUTES.md) | 动作与观测编码的职责及 ABI |
| [原生迁移笔记](docs/NATIVE-033.md) | 0.3.3 引擎的开发记录 |
| [问题反馈](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/issues) | 接口问题与可复现报告 |

## 许可与致谢

代码采用 GPL-3.0-or-later。Stronghold Protocol 提供上游模拟实现，游戏素材与外部数据的来源和使用边界见许可说明。[LICENSE](LICENSE) · [NOTICE](NOTICE.md)

首页编排参考 AutoTavern，横幅为本项目原创示意图。[AutoTavern](https://github.com/MCG-NJU/AutoTavern)

<p align="center"><a href="#top">回到顶部</a></p>
