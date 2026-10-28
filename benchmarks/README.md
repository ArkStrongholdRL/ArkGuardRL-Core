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

使用 `python -m arkguardrl_core iterate --collect-only` 采集完整对局。
重复至少三次，固定权重、游戏种子、RNG、worker 数量、日志和传输配置。
采集耗时包含特征、推理、环境执行、记录和传输，不是纯游戏引擎耗时。
逐步日志默认关闭；性能测量不要加 `--trace`，该选项用于排障。

```sh
python benchmarks/report.py artifacts/run-01 artifacts/run-02 artifacts/run-03 \
  --out artifacts/performance.json
```

工具只读取已完成的采集摘要，输出耗时和吞吐的中位数、最小值、最大值。
配置是否一致由调用者控制；比较不同实现时，分别汇总各自的重复测量。

- `collection_seconds`：采集调用的墙钟时间。
- `decisions_per_second`：完成决策数除以采集时间。
- `entrypoint_seconds`：包含 Python 导入、进程启动和退出的入口时间。

每次 CLI 调用都会启动新进程。`--iterations N` 在一次调用内复用进程，并把种子分成
N 组；它不是同一批种子的 N 次重复。失败和截断对局不要计入完成吞吐。
若报告内存，测量应包含 Node 的整个进程树，并注明采样间隔。

GPU 推理分别用 `--inference-mode batch-mps` 或 `batch-cuda`；`--device` 只控制 PPO。
报告冷启动和后续复用轮次，并附上实际 batch 大小分布。相同数量的 worker 下比较
端到端吞吐，不能用大 batch 的固定输入前向速度替代完整采集结果。

多 env 实验显式设置 `--envs-per-worker`。比较时固定总游戏数与种子，同时报告
Python worker 数和并发 env 数，例如 8×1 与 8×2 均采集同一批 16 局。
每个 env 槽位拥有独立 Node；增加槽位会增加 Node 数和同时驻留的轨迹。

2026-10-28 本机实验（Apple M5、16 GB、PyTorch 2.14.1、MPS）：固定权重及 16 局种子，
共 3446 次决策。冻结的 main `6af66bf` 与多 env 分支各取 5 次热运行，采集中位数
从 30.97 秒降至 25.74 秒，吞吐从 111.3 提至 133.9 decisions/s；平均 batch 中位数
从 2.73 增至 4.58。两组均为 8 个 Python worker，Node 会话从 8 增至 16。
这是该负载的端到端 MPS 测量，CUDA 尚未实测。
