# ArkGuardRL-Core

卫戍协议 RL 环境与 Python/Rust 运行时。**Core 负责性能，ArkGuardRL 主仓库负责策略。**

Rust 已接管 PREP 棋子/道具动作枚举、动态观测打包、候选路线特征和关系编码；游戏状态更新与战斗仍由 Node 引擎执行。

## 安装

路线后端使用 Python 3.12；运行游戏需要 Node 22+；源码安装需要 Rust/Cargo。

```sh
python -m pip install -e '.[vector]'
```

游戏引擎和数据单独准备：[Stronghold Protocol](https://github.com/sganggs/Stronghold-Protocol)，
版本 `c9d804f470b8d7422ae04eb66faf671cef21ad52`（app 0.3.1）。
本仓库不包含引擎素材、数据、权重或轨迹。

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

## 本地训练观察台

```sh
python -m arkguardrl_core dashboard --results-root /path/to/results \
  --engine-root /path/to/engine --port 8765
```

打开 `http://127.0.0.1:8765`。无需前端构建、Torch 或额外服务；只读访问指定结果目录。
支持完整 `*.jsonl[.gz]` 轨迹逐步浏览、研究 JSON 的 Ready 棋盘/手牌快照、动作记录和指标。
可搜索实验、拖动时间轴、按回合跳转，并每 3 秒刷新文件。默认 3D 棋盘支持拖动旋转、
滚轮缩放、复位和 2D 切换；棋子标签可点击查看装备。`--engine-root` 可选，仅从外部
引擎读取棋子名称和地图，不复制素材。棋盘为状态示意图，不是战斗动画。
未记录的商店、动作概率或战斗过程不会重建；未闭合轨迹也不自动判为运行中。
新采集要看逐步状态时加 `iterate --trace`；默认训练仍不开启完整日志。
JSON 报告上限 32 MiB；完整轨迹单次索引上限 512 MiB 解压数据 / 50,000 帧，页面会提示截断。

代码使用 GPL-3.0-or-later，来源和外部资源说明见 [NOTICE](NOTICE.md) 与 [LICENSE](LICENSE)。
