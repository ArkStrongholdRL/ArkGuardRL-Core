# 安装与使用

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


并行采集使用 `AsyncVectorEnv` 或研究包的 `iterate --workers 4`。
源码测量命令见[性能测量](../benchmarks/README.md)。
