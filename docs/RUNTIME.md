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
| `--inference-mode` | 默认 local；可选 batch-cpu/batch-mps |
| `--max-buffer-bytes` | 本轮 live 数据预算 |
| `--execution-memory-bytes` | 候选 padding 执行块预算 |
| `--transfer-mode` | 默认 stream，边采集边回传；episode 为整局结束后回传 |
| `--transport-rows` | 每块行数，默认 8；1 使用逐行传输 |
| `--max-wall` | 整次运行的时间预算 |

完整参数：`python -m arkguardrl_core iterate --help`。
预算是数据分配估计，不等于进程 RSS 硬限制。只有完整自然终止的对局进入 PPO；
截断、失败和教师数据不能混进 on-policy buffer。

## 输出

默认只写运行配置、逐局结果和耗时汇总，不写轨迹、张量摘要或压缩回执，也不回读轨迹。
`--trace` 会额外写完整轨迹并检查归档，测吞吐量时应保持关闭。

- `run.json`、`inputs.json`：运行配置和输入。
- `summary.json`、`iteration-XXXX.json`：总计与逐轮耗时。
- `episodes-rXXXX.json`：逐局结果、分项耗时。
- `checkpoint.pt`、`training-state.pt`：训练时保存的参数与优化器状态。

检查点核对模型结构、输入契约和参数形状；旧文件内的源码哈希不再限制加载。
Rust 修改后重新安装并重启进程。只要输入和参数布局未改变，就不需要迁移权重。
