# 性能测量

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
