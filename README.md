# ArkGuardRL-Core

卫戍协议原生环境、优化采样和 RL 学习框架。组织：ArkStrongholdRL，私有仓库。
实验设计与结果留在 [ArkGuardRL](https://github.com/HarcoChen/ArkGuardRL)。

## 边界

| 本仓库维护 | 实验仓库维护 |
|---|---|
| 原生环境、公开候选、history/prep64、反馈与契约 | 固定训练/验证种子与数据划分 |
| 路线几何、候选棋盘指标数组化、有效语义事实压缩 | 教师/真人数据、BC/PPO 系数与消融方案 |
| 单次张量化、延后next-value、有界完整live采样、精简日志 | 固定父检查点、运行计划、指标与报告 |
| 跨状态batch、候选mask、内存分块梯度累积、PPO更新 | 历史实验启动器与结果回放 |
| 严格检查点身份、CPU序列化、显式CPU/MPS/CUDA学习端 | 原检查点严格加载后的显式新框架采用适配器 |

Stronghold Protocol 是独立外部引擎，固定提交
`c9d804f470b8d7422ae04eb66faf671cef21ad52`，app0.3.1。本仓库不内置引擎、数据或素材。
本次提取没有实现四环境并行采样，也没有修改bridge里的重复observe或观战快照路径。
这些是接下来在框架侧处理的工作。本文所述优化来自拆分前代码；拆分后没有新跑对局或
性能/梯度比较，不能把以前的计时当作新仓库实测。

## 安装

普通原生环境/CLI：Python3.10+；路线数组后端：CPython3.12。游戏执行另需Node22+和
按上游许可准备的干净引擎、依赖及数据。

```sh
python -m pip install '.[vector]'
python -m arkguardrl_core --help
python -m arkguardrl --help
python -m arkguardrl.learn --help
```

发布分发名为 `arkguardrl-core`，包含两个命名空间：`arkguardrl` 是保持源码字节一致的
原生契约/环境/基础学习实现；`arkguardrl_core` 是这次提取的优化采样、路线模型与批量
学习API。使用独立环境安装，避免与旧 `arkguardrl` 分发安装到同一路径。

```python
from arkguardrl_core.run import Run, SamplerConfig, PPOConfig
from arkguardrl_core.models import Policy
from arkguardrl_core import checkpoint, sampling
from arkguardrl_core.learning.ppo import ppo_update
```

`Run` 显式接收engine_root、expected_data_sha256、difficulty、资源预算和配置，不导入
实验仓库，不读取某个作者电脑的默认路径。数据SHA由独立选定的数据身份提供，不能从
待验证检查点里抄来绕过检查。默认PREP动作成本为0；实验若用非零成本，Run会生成独立
objective contract并记录具体系数，不能冒充原生或历史成本目标。

## 运行

基础原生教师捕获、BC和PPO继续使用 `python -m arkguardrl.learn`。路线模型创建方式：
`Policy(contract, public.definitions()[0], width=1, route_backend='array')`，其中contract
由选定公开catalog独立构造。可使用Run的public/contract；新模型保存后才绑定live策略。
路线模型目前支持width1/width2的参数布局，拆分前完整批量实测只有width1。

新的路线采样/批量入口每次只执行一轮PPO，种子与输入文件由调用方显式传入：

```sh
python -m arkguardrl_core iterate \
  --engine-root /absolute/path/to/clean-engine \
  --expected-data-sha256 VERIFIED_EXTERNAL_DATA_SHA256 \
  --checkpoint /absolute/path/to/core-policy.pt \
  --anchor /absolute/path/to/core-anchor.pt \
  --seeds 10001 10002 --rng-seed 42 \
  --device mps --batch-size 256 --epochs 2 \
  --max-wall 600 --out artifacts/new-run
```

示例种子只是命令形式，不是推荐的正式训练/评估划分。输出目录必须全新；加
`--collect-only`可只采样。当前采样仍按局串行，actor/anchor在CPU，学习端可选CPU/MPS；
CUDA代码路径可显式选择但拆分前没有本机硬件验证。设备不可用明确拒绝，不自动回退。
较大batch可能按候选补齐估计分块，按样本数加权累积，逻辑batch只裁剪/更新一次。
估计不是内存峰值保证；单状态超限明确停止，不删候选。

采样只返回本轮内存里的完整自然终止live行。精简日志是回执，不是完整状态或离线PPO
训练集；teacher/human数据也不能混进on-policy buffer。保留gamma1、逐局GAE、个人终止
和预算截断的区别。模型参数在一局内不变，更新前执行live概率/价值核对。

## 检查点与来源

新路线框架schema为 `arkguardrl-core/route-width-array-batch-policy/v1`，绑定输入契约、
源码闭包、宽度与特征后端。历史路线检查点不被直接接受。必须在原严格源码中先验证、
显式采用参数后保存为新格式，记录原文件SHA与原身份；不修改原文件或伪造旧身份。
实验仓库提供这个采用适配器。更改源码会使严格新loader拒绝旧源码身份，这是显式边界。

`SOURCE_MANIFEST.json`逐文件记录上游源码位置、哈希和是否修改；优化部分来自原工作树
的未提交代码，已单独记录，不能声称都包含在原HEAD提交中。许可见 [NOTICE](NOTICE.md)。

未新增测试、fixture或测试框架。初次拆分只做源码/安装包/CLI和原权重显式采用检查，
不启动训练、对局或重复性能诊断。详细迁移边界见 [EXTRACTION](docs/EXTRACTION.md)。
