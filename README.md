# ArkGuardRL-Core

卫戍协议 RL 环境与 Python/Rust 运行时。**Core 负责性能，ArkGuardRL 主仓库负责策略。**

Rust 已接管 PREP 棋子/道具动作枚举、动态观测打包、候选路线特征和关系编码；游戏状态更新与战斗仍由 Node 引擎执行。

## 安装

路线后端使用 Python 3.12；运行游戏需要 Node 22+；源码安装需要 Rust/Cargo。

```sh
python -m pip install -e '.[vector]'
```

只运行 Gymnasium 环境和无模型 benchmark，可安装 `python -m pip install -e '.[gym]'`，不需要 Torch。

游戏引擎和数据单独准备：[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol)，
版本 `c9d804f470b8d7422ae04eb66faf671cef21ad52`（app 0.3.1）。
本仓库不包含引擎素材、数据、权重或轨迹。

## Gymnasium 环境

```python
import gymnasium as gym

with gym.make("arkguardrl_core.gym_env:ArkGuardRL-v0", engine_root="/path/to/engine") as env:
    obs, info = env.reset(seed=42)
    action = env.action_space.sample(mask=obs["action_mask"])
    obs, reward, terminated, truncated, info = env.step(action)
```

观测为 NumPy `Dict`：`observation` 是 4784 维动态观测，`candidates` 是候选特征矩阵，
`schedules`、`experts` 是候选分组，`action_mask` 标记有效行。静态定义由
`env.unwrapped.catalog.definitions()` 提供。动作是当前候选行索引；掩码表示公开候选域，
引擎仍可能拒绝动作，结果通过 `info["accepted"]` 返回。选择 padding 会抛出 `InvalidAction`，
不会替换动作或推进游戏。候选上限默认 16384，可配置 `max_candidates`；超限报错，不裁剪动作域。

每个 env 在 reset 间复用自己的 Node 进程。Gym 种子通过 `np_random` 派生引擎需要的非零
uint32 种子，记录在 `info["engine_seed"]`；相同 Gym 种子可复现，`reset(seed=None)` 继续 RNG 流。
自然结束返回 `terminated`，预算耗尽返回 `truncated` 和真实后继观测；引擎/传输错误直接抛出。
奖励沿用 `NativeProgressCore`，没有模型、高层选项或 PPO。现有策略专用上下文需要在环境外构造。

批量运行直接使用 Gymnasium `SyncVectorEnv` / `AsyncVectorEnv`。Async 用 `spawn` 和共享内存，
脚本入口需要 `if __name__ == "__main__"`。benchmark 使用 `AutoresetMode.DISABLED`，
读取终止观测后通过 `reset_mask` 单独 reset，避免将自动 reset 误计为游戏决策。

```sh
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --steps 256 --repeats 3 --out artifacts/gym-async-4.json
```

测试：`ARKGUARD_ENGINE_ROOT=/path/to/engine python -m unittest discover -s tests -p test_gym_env.py -v`。
测试按掩码选动作，检查 Gym 的 reset 确定性与 step 返回值；通用 `check_env()` 的无掩码随机动作
可能选中 padding，因此不直接适用于这个动作契约。

## 性能采集

```sh
python -m arkguardrl_core iterate \
  --engine-root /path/to/engine \
  --expected-data-sha256 DATA_SHA256 \
  --checkpoint /path/to/policy.pt --anchor /path/to/anchor.pt \
  --seeds 10001 10002 10003 10004 --rng-seed 42 \
  --workers 4 --device cpu --collect-only \
  --max-buffer-bytes 2147483648 --max-wall 600 \
  --out artifacts/run-01
```

重复测量保持相同权重、种子和配置，输出目录每次使用新名字。
`--collect-only` 只采集、不更新权重。测量包含特征编码、推理、游戏执行和传输。
默认关闭逐步日志和轨迹回读，只保留结果与耗时汇总；排障时用 `--trace` 开启完整轨迹。

```sh
python benchmarks/report.py artifacts/run-01 artifacts/run-02 \
  --out artifacts/performance.json
```

检查点按模型结构和输入契约加载，不再绑定 Python 源码或 Rust 二进制哈希。
普通代码修改或重新编译无需重新导出权重。

[运行参数](docs/RUNTIME.md) · [Rust 内核](docs/RUST-ROUTES.md) · [性能测量](benchmarks/README.md)

代码使用 GPL-3.0-or-later，来源和外部资源说明见 [NOTICE](NOTICE.md) 与 [LICENSE](LICENSE)。
