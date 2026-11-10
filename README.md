# ArkGuardRL-Core

[![Python](https://img.shields.io/badge/python-3.10%2B-3776AB?style=flat-square&logo=python&logoColor=white)](https://www.python.org/)
[![Rust](https://img.shields.io/badge/rust-PyO3%200.27-dea584?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Gymnasium](https://img.shields.io/badge/gymnasium-%E2%89%A51.1-4B8BBE?style=flat-square)](https://gymnasium.farama.org/)
[![Engine](https://img.shields.io/badge/engine-app%200.3.3-lightgrey?style=flat-square)](https://github.com/sganggs/Stronghold-Protocol)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue?style=flat-square)](LICENSE)
[![Status](https://img.shields.io/badge/status-pre--release-orange?style=flat-square)](docs/NATIVE-033.md)

[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 的 Gymnasium 环境。

当前维护与原生迁移范围仅限绝境（`mode_single_abyss`），进度见 [迁移文档](docs/NATIVE-033.md)。

## 安装与使用

Python 3.10+、Rust stable，以及另行准备的 Stronghold Protocol 0.3.3 数据目录。
本仓库不含游戏素材、数据、权重或轨迹。

```sh
python -m pip install -e '.[gym]'
PYO3_PYTHON="$(command -v python)" cargo build --release --locked -p arkguard-python
```

```python
import gymnasium as gym
import arkguardrl_core.native_gym

with gym.make(
    "ArkGuardRL-v1",
    data_root="/path/to/0.3.3/data",
    core="band_sarkazb",  # band_amiya / band_sarkazb / band_chen / band_sciurus
    native_library="target/release/lib_native.dylib",
) as env:
    obs, info = env.reset(seed=42)
    done = False
    while not done:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        done = terminated or truncated
```

Linux 动态库后缀为 `.so`，CI 提供 Linux x64、Windows x64、macOS Intel／Apple Silicon 的 CPython 3.12 原生模块，详见[构建说明](benchmarks/README.md#跨平台构建)。原生运行时读取数据目录，核心由构造参数指定。`banned` 接受禁用干员 ID，`tick_budget` 控制每次战斗续跑的帧数。

观测是 NumPy `Dict`，动作是**当前候选行索引**：

| 字段 | 形状 / 类型 | 含义 |
| --- | --- | --- |
| `observation` | `(4784,) float32` | 动态观测 |
| `candidates` | `(max_candidates, 41) float32` | 候选特征，padding 为 0 |
| `schedules` / `experts` | `(max_candidates,) int64` | 候选分组 |
| `action_mask` | `(max_candidates,) int8` | 有效候选行 |

- 候选由公开参数域构造，游戏规则可能拒绝提交的动作，结果见 `info["accepted"]`。
- padding 或越界索引抛 `InvalidAction`，不替换动作、不推进游戏。
- 候选容量默认 16384，超限报错，不裁剪动作域。
- 自然结束为 `terminated`；`max_decisions` / `max_advances` / `wall_seconds` 耗尽为
  `truncated`（`info["reason"]`）且返回真实后继观测；引擎/传输错误抛异常。
- 奖励由 `NativeProgressCore` 计算，分项在 `info["reward_components"]`。
- 相同 Gym 种子可复现，实际引擎种子见 `info["engine_seed"]`；`reset(seed=None)` 延续 RNG 流。
- 静态定义通过 `env.unwrapped.catalog.definitions()` 读取。候选依赖游戏状态，
  通用 `check_env()` 的无掩码随机动作会选中 padding，请按掩码采样。
- 难度与预算只能在构造时配置，`reset(options=...)` 不接受其他选项。

## 并行与回放

Gym 支持 `SyncVectorEnv`／`AsyncVectorEnv`，每个环境拥有独立原生会话；Async 使用 `spawn` 时需要 `if __name__ == "__main__"` 入口。Rust slots 支持 1–64 个环境、显式局部 reset 和部分槽位推进。

公开地图与当前模板路线通过 `env.unwrapped.public_spatial()` 读取，增量入口为 `public_spatial_update(key)`。真实记录回归、渲染回放和吞吐命令见[性能与验收说明](benchmarks/README.md)。

## Node 兼容入口

旧研究调用可通过 `import arkguardrl_core.gym_env` 注册 `ArkGuardRL-v0`，构造时传入 `engine_root`。该路径使用 Node 22+ 及对应的 0.3.1 引擎／数据；观测与候选布局见上表。

## 文档

| 文档 | 内容 |
| --- | --- |
| [性能测量](benchmarks/README.md) | 负载定义、测量边界与基线数据 |
| [Rust 内核](docs/RUST-ROUTES.md) | 动作/观测/路线内核的职责与 ABI |
| [原生迁移状态](docs/NATIVE-033.md) | 0.3.3 Rust 引擎实现范围与开发命令 |


GPL-3.0-or-later；来源与外部资源边界见 [NOTICE](NOTICE.md) 与 [LICENSE](LICENSE)。
