# 性能测量

独立 env 测量使用 `.[gym]` 安装与 `gym_env.py`，不加载模型、Torch、PPO 或轨迹采集器：

```sh
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector sync --envs 1 --steps 256 --repeats 3 --out artifacts/gym-sync-1.json
python benchmarks/gym_env.py --engine-root /path/to/engine \
  --vector async --envs 4 --steps 256 --repeats 3 --out artifacts/gym-async-4.json
```

动作在有效候选行中均匀抽样，每个 env 使用独立 RNG。每次重复从相同游戏/RNG 种子重新开始；
不截断候选域。吞吐计入循环内 reset，单列初始 reset 与启动/预热时间。`steps` 是每个 env 的真实
决策次数，reset 不计作 step；自然结束与预算截断分别计数。分项时间是所有 env 的累计值，
异步运行时不能当作墙钟时间占比。观测填充与 Gym 通信也计入测量，因此结果不是纯引擎速度。
它与下面使用学习策略的完整采集 benchmark 负载不同，不直接比较倍率。

2026-10-28，Apple M5 / 16 GB，Gymnasium 1.4.0，独立无 Torch 环境：每个 env
1024 步，预热 16 步，重复 3 次，Gym seed 42、采样 seed 123，默认候选容量 16384。

| 配置 | 中位吞吐（决策/秒） | 每次测量墙钟中位数 |
| --- | ---: | ---: |
| Sync，1 env | 447 | 2.29 秒 / 1024 步 |
| Sync，4 env | 451 | 9.08 秒 / 4096 步 |
| Async，共享内存，4 env | 982 | 4.17 秒 / 4096 步 |

4 env 的 Sync/Async 动作、奖励和结束标志摘要一致，每次均自然结束 18 局、截断 0 局；
剩余轨迹不一定结束，因此不能用总耗时除以结束局数估算单局耗时。
Async 相对同负载 Sync 吞吐为 2.18 倍。单 env 的约 2.24 毫秒/步包含观测、奖励及 Gym 开销。
默认填充观测为每 env 2,984,640 字节；本次最多 401 个候选，不代表所有对局的候选上限。
