# ArkGuardRL-Core

[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol) 的 Gymnasium 环境。
Core 提供环境、公开动作候选、观测编码和 Rust 内核；策略网络、PPO、采集调度与研究脚本位于
[ArkGuardRL](https://github.com/HarcoChen/ArkGuardRL)。

版本 `0.1.0rc1`：Node 0.3.3 执行完整游戏，Rust 加速动作枚举与特征编码。
完整 Rust 环境另在实验分支开发。

游戏状态更新与战斗仍由外部 Node 引擎执行。Rust 负责 PREP 棋子/道具动作枚举、动态观测打包
和特征编码；本项目目前不是完整的 Rust 游戏模拟器。RC 阶段已验证的平台会随实际安装测试记录，
不以历史 0.3.1 的吞吐代表当前训练采集速度。

## 安装

需要 Python 3.10+、Node 22+，源码构建还需要 Rust/Cargo。

```sh
python -m pip install 'arkguardrl-core[gym]==0.1.0rc1'
# 源码开发：python -m pip install -e '.[gym]'
```

单独准备 Stronghold Protocol 引擎和数据：当前支持提交
`41bbca5712ac08394bbc4d5b4e3411c6be1634b6`（app 0.3.3），引擎目录需保持干净。
本仓库不提供游戏素材、数据、模型权重或轨迹，也不需要 Torch。

## 使用

```python
import gymnasium as gym

with gym.make("arkguardrl_core.gym_env:ArkGuardRL-v0", engine_root="/path/to/engine") as env:
    obs, info = env.reset(seed=42)
    while True:
        action = env.action_space.sample(mask=obs["action_mask"])
        obs, reward, terminated, truncated, info = env.step(action)
        if terminated or truncated:
            break
```

观测是 NumPy `Dict`：

| 字段 | 含义 |
| --- | --- |
| `observation` | 4784 维动态观测 |
| `candidates` | 当前候选的特征矩阵，填充至 `max_candidates` |
| `schedules`、`experts` | 候选分组 |
| `action_mask` | 有效候选行 |

动作是当前候选行索引。候选由公开信息构造，引擎仍可能拒绝动作，见 `info["accepted"]`。
选择 padding 会抛出 `InvalidAction`，不会替换动作或推进游戏。候选容量默认 16384；
超限报错，不裁剪动作域。静态定义可通过 `env.unwrapped.catalog.definitions()` 读取。

奖励使用 `NativeProgressCore`，分项见 `info["reward_components"]`。
自然结束返回 `terminated`，预算耗尽返回 `truncated` 和真实后继观测；引擎或传输错误抛出异常。
`max_decisions`、`max_advances` 和可选的 `wall_seconds` 控制运行预算。

每个 env 在 reset 之间复用自己的 Node 进程。Gym 种子派生非零 uint32 引擎种子，记录在
`info["engine_seed"]`；相同 Gym 种子可复现，`reset(seed=None)` 继续 RNG 流。

## 并行与测量

单个 Gym 环境不自动并行；同时采集多个对局需显式使用 `AsyncVectorEnv`，
或研究包的 `iterate --workers 4` 入口。`SyncVectorEnv` 逐个推进环境。Async 使用 `spawn` 和共享内存，
脚本入口需要 `if __name__ == "__main__"`。默认填充观测约 2.98 MB/env，每个 env 还会创建
一个 Node 进程，增加并发时需要考虑两者的内存占用。

```sh
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --steps 256 --repeats 3 --out artifacts/gym-async-4.json
```

[性能测量](benchmarks/README.md)记录负载和测量边界；[Rust 内核](docs/RUST-ROUTES.md)说明实现范围。

原生集成测试需要显式提供引擎路径，会实际运行对局：

```sh
ARKGUARD_ENGINE_ROOT=/path/to/engine python -m unittest discover -s tests -p test_gym_env.py -v
```

通用 `check_env()` 的无掩码随机动作可能选中 padding；现有测试按掩码选择动作，检查种子、
状态转移、截断、自然结束及向量环境行为。

## 代码边界

`arkguardrl_core` 包含 Gymnasium 接口、Rust 内核及 Node 会话复用；`arkguardrl` 包含底层状态、
候选、奖励与观测契约。保留两个包名以维持环境输入兼容。
原训练入口已迁至研究仓库的 `python -m arkguardrl_research.runtime`。

代码采用 GPL-3.0-or-later；来源与外部游戏资源边界见 [NOTICE](NOTICE.md) 和 [LICENSE](LICENSE)。
