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

维护范围为单人绝境模式（`mode_single_abyss`），当前使用 **0.3.3 Rust 原生运行时**。
战斗、公开账本、奖励、候选生成和数值编码在 Rust 中执行，Python 提供 Gymnasium 接口和采样入口。

## 快速开始

使用 Python 3.12、Git、Rust stable 和系统链接工具，另行准备 Stronghold Protocol 0.3.3 数据目录。

```sh
git clone https://github.com/ArkStrongholdRL/ArkGuardRL-Core.git
cd ArkGuardRL-Core
python -m pip install -e '.[gym]'
PYO3_PYTHON="$(command -v python)" cargo build --release --locked -p arkguard-python
```

CI 提供 Python 3.12 的 Linux x64、Windows x64 和 macOS Intel／Apple Silicon 模块。
下例动态库路径适用于 macOS，Linux 使用 `target/release/lib_native.so`，Windows 使用 CI 产物 `_native*.pyd` 的实际路径。[跨平台构建](docs/benchmarks/README.md#跨平台构建)

### 运行一局

```python
import gymnasium as gym
import arkguardrl_core.native_gym

with gym.make(
    "ArkGuardRL-v1",
    data_root="/path/to/0.3.3/data",
    core="band_sarkazb",
    native_library="target/release/lib_native.dylib",
) as env:
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
| `ArkGuardRL-v1` | 原生单环境 Gymnasium 决策循环 |
| `SyncVectorEnv`／`AsyncVectorEnv` | 多环境采样，每个环境持有独立会话 |
| Rust slots | 1–64 个环境，支持局部重置和部分槽位推进 |
| `env.unwrapped.public_spatial()` | 读取公开地图、当前模板路线及敌人预览关联 |
| `env.unwrapped.catalog.definitions()` | 读取公开静态定义 |
| `info` | 检查动作接受结果、奖励分项、引擎种子及结束原因 |

核心支持阿米娅（`band_amiya`）、萨卡兹（`band_sarkazb`）、陈（`band_chen`）和休露丝（`band_sciurus`）。
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
- Async 使用 `spawn` 时，入口需要 `if __name__ == "__main__"`。每个环境持有独立原生会话。

</details>

公开空间增量读取使用 `env.unwrapped.public_spatial_update(key)`。真实记录回归和战斗轨迹导出入口见基准文档。[空间接口与渲染回放](docs/benchmarks/README.md)

<details>
<summary>Node v0 兼容入口</summary>

旧调用通过 `import arkguardrl_core.gym_env` 注册 `ArkGuardRL-v0`，构造时传入 `engine_root`。
该入口使用 Node.js 22+ 和 0.3.1 引擎／数据。

</details>

## 性能

| 配置与计时范围 | 决策／秒 |
| --- | ---: |
| 原生 Gym 单环境，购买部署 | 437.2 |
| Rust slots，4 环境／4 线程，含空间 JSON 读取 | 585.7 |

以上为原生候选准备与索引执行后的完整对局 A/B 记录，计时覆盖环境执行与接口开销。
模型前向与训练另行计时。负载定义、真实记录回归与复现用法见性能文档。[性能基准](docs/benchmarks/README.md)

## 目录结构

```text
src/          Python 环境包
crates/       Rust 引擎、Python 绑定与并行扩展
docs/         接口说明与性能文档
benchmarks/   基准脚本与测量数据
.github/      跨平台 CI
```

## 文档

| 文档 | 内容 |
| --- | --- |
| [性能基准](docs/benchmarks/README.md) | 测量结果、计时边界与运行命令 |
| [问题反馈](https://github.com/ArkStrongholdRL/ArkGuardRL-Core/issues) | 接口问题与可复现报告 |

## 许可与致谢

代码采用 GPL-3.0-or-later。Stronghold Protocol 提供上游模拟实现，游戏素材与外部数据的来源和使用边界见许可说明。[LICENSE](LICENSE) · [NOTICE](NOTICE.md)

<p align="center"><a href="#top">回到顶部</a></p>
