# ArkGuardRL-Core

[卫戍协议：盟约](https://github.com/sganggs/Stronghold-Protocol)的 Gymnasium 强化学习环境。
提供公开观测、候选动作、奖励和并行环境接口；Node 0.3.3 执行游戏，Rust 加速动作枚举与特征计算。
策略训练和多进程采集由 [ArkGuardRL](https://github.com/HarcoChen/ArkGuardRL) 提供。

## 性能

Apple M5／16 GB，随机合法动作、不运行模型，吞吐中位数：

| 配置 | 决策／秒 |
| --- | ---: |
| 单环境 | 495 |
| 4环境并行 | 937 |

以上为引擎0.3.1的历史基线；当前0.3.3尚未做同口径吞吐复测。测量包含观测编码、通信和局间reset。

[安装与使用](docs/GYM.md) · [测量详情](benchmarks/HOTSPOTS-20261102.md) · [GPL-3.0-or-later](LICENSE)
