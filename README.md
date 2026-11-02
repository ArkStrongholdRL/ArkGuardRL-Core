# ArkGuardRL-Core

[![Python](https://img.shields.io/badge/python-3.10%2B-3776AB?style=flat-square&logo=python&logoColor=white)](https://www.python.org/)
[![Rust](https://img.shields.io/badge/rust-PyO3%200.27-dea584?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Gymnasium](https://img.shields.io/badge/gymnasium-%E2%89%A51.1-4B8BBE?style=flat-square)](https://gymnasium.farama.org/)
[![Engine](https://img.shields.io/badge/engine-app%200.3.1-lightgrey?style=flat-square)](https://github.com/sganggs/Stronghold-Protocol)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue?style=flat-square)](LICENSE)
[![Status](https://img.shields.io/badge/status-pre--release-orange?style=flat-square)](docs/NATIVE-033.md)

[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 的 Gymnasium 环境。

## 安装

Python 3.10+、Node 22+
```sh
python -m pip install -e '.[gym]'
```

引擎与数据单独准备：[Stronghold-Protocol](https://github.com/sganggs/Stronghold-Protocol)
`c9d804f470b8d7422ae04eb66faf671cef21ad52`（app 0.3.1）
本仓库不含游戏素材、数据、权重或轨迹。

## 使用

```python
import gymnasium as gym
import arkguardrl_core.gym_env  # 注册 ArkGuardRL-v0

with gym.make("ArkGuardRL-v0", engine_root="/path/to/engine") as env:
    obs, info = env.reset(seed=42)
    done = False
    while not done:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        done = terminated or truncated
```

观测是 NumPy `Dict`，动作是**当前候选行索引**：

| 字段 | 形状 / 类型 | 含义 |
| --- | --- | --- |
| `observation` | `(4784,) float32` | 动态观测 |
| `candidates` | `(max_candidates, 41) float32` | 候选特征，padding 为 0 |
| `schedules` / `experts` | `(max_candidates,) int64` | 候选分组 |
| `action_mask` | `(max_candidates,) int8` | 有效候选行 |

- 候选由公开信息构造；引擎仍可能拒绝合法候选，结果见 `info["accepted"]`。
- padding 或越界索引抛 `InvalidAction`，不替换动作、不推进游戏。
- 候选容量默认 16384，超限报错，不裁剪动作域。
- 自然结束为 `terminated`；`max_decisions` / `max_advances` / `wall_seconds` 耗尽为
  `truncated`（`info["reason"]`）且返回真实后继观测；引擎/传输错误抛异常。
- 奖励由 `NativeProgressCore` 计算，分项在 `info["reward_components"]`。
- 相同 Gym 种子可复现，实际引擎种子见 `info["engine_seed"]`；`reset(seed=None)` 延续 RNG 流。
- 静态定义通过 `env.unwrapped.catalog.definitions()` 读取。候选依赖游戏状态，
  通用 `check_env()` 的无掩码随机动作会选中 padding，请按掩码采样。
- 难度与预算只能在构造时配置，`reset(options=...)` 不接受其他选项。

## 并行与测量

支持 `SyncVectorEnv` / `AsyncVectorEnv`。Async 使用 `spawn` 和共享内存，入口需要
`if __name__ == "__main__"`。每个 env 独占一个 Node 进程，默认填充观测约 2.98 MB/env。

```sh
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --steps 256 --repeats 3 --out artifacts/gym-async-4.json
```

## 测试

原生集成测试需显式提供引擎路径，会实际运行对局：

```sh
ARKGUARD_ENGINE_ROOT=/path/to/engine python -m unittest discover -s tests -p test_gym_env.py -v
```

## 文档

| 文档 | 内容 |
| --- | --- |
| [性能测量](benchmarks/README.md) | 负载定义、测量边界与基线数据 |
| [Rust 内核](docs/RUST-ROUTES.md) | 动作/观测/路线内核的职责与 ABI |
| [原生迁移状态](docs/NATIVE-033.md) | 0.3.3 Rust 引擎实现范围与开发命令 |


GPL-3.0-or-later；来源与外部资源边界见 [NOTICE](NOTICE.md) 与 [LICENSE](LICENSE)。
