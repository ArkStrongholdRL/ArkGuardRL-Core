# Benchmarks · 环境性能测量

本目录提供 **Gymnasium 环境吞吐测量**、原始指标和复测程序。指标不代表强化学习训练速度、模型推理速度或进行中的纯 Rust 0.3.3 环境性能。

## Quick results · 最近测量

**2026-11-02 · Apple M5 / 16 GiB · Python 3.12.13 · Node 26.7.0 · Gymnasium 1.4.0 · NumPy 2.5.4**

| 配置 | 优化前中位决策/秒 | 优化后中位决策/秒 | 相对变化 |
| :-- | --: | --: | --: |
| Sync · 1 env | 480.57 | **494.75** | +2.95% |
| Async · 4 env · 共享内存 | 899.14 | **937.19** | +4.23% |

测量采用**在有效候选行中均匀随机选择动作**，没有加载模型、Torch、PPO 或训练轨迹。每环境预热 64 步、测量 1,024 步，各版本进行 6 次测量，按旧／新／新／旧顺序交替执行（每次 3 遍）。吞吐 = 所有环境的真实决策总次数 / 墙钟秒。计入测量循环中的 `reset`、观测构造及 Gym 通信；排除初次 `reset` 与启动开销。

> **Interpretation:** 4 环境 Async 的 937.19 decisions/s 是四环境**合计吞吐**，不是每个环境的单线程速度。两项小幅热点优化的实测结果受桌面负载波动影响，尚不足以证明训练加速。

**详情：** [热点测量报告](HOTSPOTS-20261102.md) · [原始 JSON](env-hotspots-20261102.json) · [测量程序](gym_env.py)。

## Historical baseline · 2026-10-28

**Apple M5 / 16 GB · Gymnasium 1.4.0 · 无 Torch 环境 · 每环境 1,024 步 · 预热 16 步 · 3 次重复。**

| 配置 | 中位决策/秒 | 每次测量墙钟中位数 |
| :-- | --: | --: |
| Sync · 1 env | 447 | 2.29 s / 1,024 steps |
| Sync · 4 env | 451 | 9.08 s / 4,096 steps |
| Async · 4 env · shared memory | 982 | 4.17 s / 4,096 steps |

在这组**同日、同负载**测量中，4 env Async / 4 env Sync 吞吐比为 **2.18×**。四环境 Sync/Async 的动作、奖励和结束标志摘要一致；每次测量中自然结束 18 局、截断 0 局，剩余轨迹可能尚未结束，因此不得用决策吞吐直接推断单局耗时。

不同日期数据不具有受控的直接可比性；不要把 10 月 28 日的 982 与 11 月 2 日的 937.19 解释为性能退步。

## Reproduce · 复测

先按照 [根 README](../README.md#quick-start--快速开始) 安装 `.[gym]`，准备固定外部引擎与授权数据。以下操作会真实运行游戏环境：

```bash
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector sync --envs 1 --warmup 64 --steps 1024 --repeats 3 \
  --out artifacts/gym-sync-1.json

python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --warmup 64 --steps 1024 --repeats 3 \
  --out artifacts/gym-async-4.json
```

每个 env 使用独立的采样 RNG；相同配置的每个重复从相同游戏和采样种子重新开始。默认 `--seed 42 --rng-seed 123 --max-candidates 16384`。Async 使用 `spawn` 及共享内存；每个 env 拥有独立 Node 进程。

**测量边界与限制：** `steps` 为每个环境的真实 `step` 次数，不包括 `reset`；在循环内发生的 `reset` 时间则计入墙钟。自然结束与预算截断分别统计。分项 engine / encoding / reward 时间是所有 env 的**累计时间**，并行环境下不得当成墙钟时间占比。默认填充观测约 2.98 MB/env；目前测得的最大候选数量并不代表可支持的动作上限。这个 benchmark 与使用学习策略的完整轨迹采集负载不同，不能直接比较速度倍率。

**Parity checks (2026-11-02):** 测量报告记录了 3 个种子、每个 1,024 步前后的完整观测/候选/动作/奖励等逐字节或逐项一致核对。8 项 Gym 环境测试及 83 项非可选 Rust 测试通过；35 项需要外部参考引擎的 Rust 测试未运行。详见 [热点报告](HOTSPOTS-20261102.md)。
