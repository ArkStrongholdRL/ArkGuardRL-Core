# 运行时

`arkguardrl` 提供环境、候选和基础学习接口；`arkguardrl_core` 提供 Rust 特征计算、
多进程采样和批量 PPO。策略训练方案属于主仓库。

## 常用参数

| 参数 | 作用 |
|---|---|
| `--workers` | 1–16 个采样进程，默认 4；各自复用 Node |
| `--collect-only` | 只采集，不执行 PPO |
| `--trace` | 排障时显式开启完整轨迹；默认不记录逐步日志 |
| `--iterations` | 种子列表分为等长组，跨轮复用 worker、Node 和优化器 |
| `--device` | 学习端 cpu/mps/cuda；默认 cpu |
| `--inference-mode` | 采样推理：默认 local；可选 batch-cpu/batch-mps/batch-cuda |
| `--inference-wait-ms` | 设备空闲时的合批等待窗口，默认 2 ms；不含等待正在执行的 batch |
| `--max-buffer-bytes` | 本轮 live 数据预算 |
| `--execution-memory-bytes` | 候选 padding 执行块预算 |
| `--transfer-mode` | 默认 stream，边采集边回传；episode 为整局结束后回传 |
| `--transport-rows` | 每块行数，默认 8；1 使用逐行传输 |
| `--max-wall` | 整次运行的时间预算 |

完整参数：`python -m arkguardrl_core iterate --help`。
预算是数据分配估计，不等于进程 RSS 硬限制。只有完整自然终止的对局进入 PPO；
截断、失败和教师数据不能混进 on-policy buffer。

`--device` 控制 PPO 学习端，`--inference-mode` 独立控制采样推理端。
例如 `--inference-mode batch-mps --device mps` 或 `--inference-mode batch-cuda --device cuda`。
设备模型在 Collector 生命周期内常驻，权重变化时才更新；actor/anchor 权重及结构相同时共享前向。
原生动作与高层策略选项都支持合批，binding 在原始动作抽样后执行，随机流仍由各 worker 独立持有。
推理在独立线程串行执行；主进程同时接收请求和回传数据，完成通知通过 pipe 唤醒事件循环。
优先返回预测，每次最多解码一个数据帧，再检查推理消息；数据管道轮转，避免大块回传阻塞其他 worker。
高层选项优先执行，使对应 worker 尽快生成下一次原生动作输入。每个 worker 最多挂起一个请求，
设备上最多执行一个 batch，训练仍在整轮完整采集结束后开始。
GPU 前向不重复做输入检查；学习输入边界仍核对模型输入和完整 rollout。
CUDA 输入按 dtype 合并后通过 pinned memory 搬运；MPS 使用合并缓冲区；输出集中回传，不再额外同步设备。
GPU 候选和 MLP 输入按 128 行分桶，语义事实按 256 行分桶，减少动态形状变化；
padding 不参与动作归一化，接近执行内存预算时使用原始尺寸。
每个候选只执行对应的 expert head，避免同时计算购买和放置两个分支。

少量 env 可能无法形成有效 batch。看报告中的 `inference.mean_batch_size`、`batch_histogram`、
`selector_batch_histogram` 和端到端吞吐判断收益，不能只看模型前向时间。
CPU 与 GPU 的 float32 结果允许正常舍入差异，不要求逐位相同。CUDA 需在 NVIDIA 硬件上独立测量。
`inference_queue.mean_wait_seconds` 统计请求被主进程接收后到提交执行的平均等待时间，
包括等待上一批推理；`max_pending` 是尚未提交的排队请求峰值，不含正在执行的 batch。

## 输出

默认只写运行配置、逐局结果和耗时汇总，不写轨迹、张量摘要或压缩回执，也不回读轨迹。
`--trace` 会额外写完整轨迹并检查归档，测吞吐量时应保持关闭。

- `run.json`、`inputs.json`：运行配置和输入。
- `summary.json`、`iteration-XXXX.json`：总计与逐轮耗时。
- `episodes-rXXXX.json`：逐局结果、分项耗时。
- `checkpoint.pt`、`training-state.pt`：训练时保存的参数与优化器状态。

检查点核对模型结构、输入契约和参数形状；旧文件内的源码哈希不再限制加载。
Rust 修改后重新安装并重启进程。只要输入和参数布局未改变，就不需要迁移权重。
