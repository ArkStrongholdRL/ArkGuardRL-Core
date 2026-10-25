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
框架现已支持可配置的多进程采样（默认四个worker）、每worker复用Node，以及bridge
中间调度步骤的轻量检查。几何信息随决策状态返回，避免每步单独请求。
外部引擎和观战快照路径保持原样；旧版本的计时不能直接代表当前实现。

## 安装

普通原生环境/CLI：Python3.10+；路线数组后端：CPython3.12。游戏执行另需Node22+和
按上游许可准备的干净引擎、依赖及数据。

```sh
python -m pip install '.[vector]'
python -m arkguardrl_core --help
python -m arkguardrl --help
python -m arkguardrl.learn --help
```

发布分发名为 `arkguardrl-core`，包含两个命名空间：`arkguardrl` 是原生契约/环境/基础
学习实现（候选与结构化编码已在提取后优化）；`arkguardrl_core` 是优化采样、路线模型与批量
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

路线采样/批量入口默认执行一轮PPO；可显式指定多轮，种子与输入文件由调用方传入：

```sh
python -m arkguardrl_core iterate \
  --engine-root /absolute/path/to/clean-engine \
  --expected-data-sha256 VERIFIED_EXTERNAL_DATA_SHA256 \
  --checkpoint /absolute/path/to/core-policy.pt \
  --anchor /absolute/path/to/core-anchor.pt \
  --seeds 10001 10002 10003 10004 --rng-seed 42 --workers 4 \
  --device mps --batch-size 256 --epochs 2 \
  --max-buffer-bytes 2147483648 --execution-memory-bytes 1073741824 \
  --max-wall 600 --out artifacts/new-run
```

示例种子只是命令形式，不是推荐的正式训练/评估划分。输出目录必须全新；加
`--collect-only`可只采样。`--workers 1`也使用同一采样器，按局串行并复用Node。
默认每worker的actor/anchor在CPU，学习端可选CPU/MPS；
CUDA代码路径可显式选择但拆分前没有本机硬件验证。设备不可用明确拒绝，不自动回退。
较大batch可能按候选补齐估计分块，按样本数加权累积，逻辑batch只裁剪/更新一次。
估计不是内存峰值保证；单状态超限明确停止，不删候选。

每个spawn worker使用一个Torch线程并独占Node、history和反馈流。每轮冻结同一策略，
父进程按任务顺序收齐完整自然终止对局，再执行PPO；任一局失败则整轮不更新。
回传的是本轮live行，分块拆成最多1MiB、有确认和摘要校验的帧，不从历史日志恢复。
默认`--transfer-mode stream`在下一决策补齐bootstrap后开始回传，用独立线程与数据pipe
重叠采样和编码/回传；控制与可选推理走另一条pipe。每worker默认最多4帧未确认、
2行队列（`--transfer-window 1..16`、`--queue-rows 1..8`），另有最多8行正在组块。父进程暂存的行
必须等整局审计通过且帧收齐才交给PPO；失败则整轮丢弃。`--transfer-mode episode`
保留整局结束后再回传的对照模式。流式回传仍在worker保留本局行用于审计，
会与父进程暂存重叠；它减少结束时等待，并不保证降低总RSS。
默认`--transport-rows 8`把相邻live行的每个张量字段组织成连续存储，变长候选由
shape和偏移表定位；接收端各行引用不重叠的存储切片，减少独立张量存储分配。
可设置2..32行；设为1使用原逐行codec作对照。未满一块的尾部在本局结束时发送。
原dtype/shape和完整元数据一起用zlib level1无损压缩，接收端先限制解压大小，
再验证行数、偏移、长度及身份。没有删候选、降低精度或省略审计字段。
跨进程仍通过pipe传输压缩数据；连续存储本身不等于跨进程零拷贝，也不保证总RSS下降。
报告区分压缩载荷、解压后载荷、张量/元数据大小及编解码时间；`wire_bytes`只统计
行载荷，不包含pipe控制消息和帧封装，也不代表训练数据驻留内存大小。
`max-buffer-bytes`约束live捕获和输入准备，每局分配总预算除以本轮局数；
`execution-memory-bytes`单独限制候选padding执行块，扩大采集预算不会同步放大执行块。
这些是数据/分配估计预算，不能当作进程RSS硬限制。

可在Python中跨轮保留`arkguardrl_core.parallel.Collector`上下文，多次调用`collect`；
每次传入新checkpoint、anchor、种子、RNG和递增revision。actor/anchor分别按文件内容
缓存；actor更新不会重载未变的anchor。每局绑定复用严格加载回执，同时重新检查
源码、文件和当前参数哈希。`--no-model-cache`供重载对照。Node跨局复用；每局结束
先dispose旧match和scheduler，再建立新游戏。

CLI的`--iterations N`将显式种子列表按顺序均分成N组，所有种子须唯一。worker、
Node、优化器和优化器随机流跨轮复用，anchor固定，下一轮actor使用本轮新检查点。
`--collect-only --iterations N`可测试同权重复用。每轮保存`iteration-XXXX.json`与
`episodes-rXXXX.json`，最终输出`summary.json`；`--max-wall`是整个运行的总预算。
每局抽样使用独立CPU Generator，不受worker调度、模型构造或推理batch顺序消耗RNG
的影响。协议为`per-episode-cpu-generator/v1`，旧版本种子的具体轨迹会改变。

实验选项`--inference-mode batch-cpu|batch-mps`把多个worker的当前决策汇集到父进程，
保留完整候选并按执行内存预算分块，最多等待`--inference-wait-ms`（默认2ms）。
每worker仍负责自己的特征、游戏、采样RNG和日志；默认`local`不发送推理请求。
推理请求另有`inference_request_bytes`统计，不计入live行的`wire_bytes`。
合批数较小时请求传输、同步与等待可能超过计算收益，应以同种子完整局实测决定。
报告提供各worker完成局数、任务占用率、最后派发至结束的尾部时长、分项耗时、
加载次数、传输窗口、进程峰值RSS及推理batch分布。单进程RSS不含Node；总峰值
需外部按进程树采样。流式`transfer_seconds`表示游戏结束后剩余回传等待，
`encode_seconds`等重叠阶段的耗时不能直接相加当作总耗时。

PPO更新前概率/价值核对和更新后KL统计复用准备好的输入，按学习设备批量执行；
输入准备保留索引、引用、语义事实和预算校验，不再通过完整encoder前向做检查。
同步PPO期间由live行持有候选/路线输入，准备阶段借用这些只读张量，避免再复制整份
输入；拼批仍复制到执行块，更新过程不修改live存储。通用`data.prepare`默认保持独立
副本；显式`copy_inputs=False`时调用方须保证整个消费期间不修改输入。
专家头与路线target头只计算有效候选，每个候选只执行对应专家；归一化仍使用候选mask。
路线特征按局缓存几何、单位覆盖与未变化棋盘的候选结果，仍逐次验证状态绑定，并在
路线、棋盘或公开效果/装备等上下文变化时失效。报告包含缓存命中、有效候选数量及
更新前核对、输入准备、设备搬运、优化和更新后统计的分项耗时。已完成一次单局完整
采集与MPS PPO更新验证；相同输入的旧版/新版顺序对照计时记录在研究仓库。

路线候选复用未变化棋盘部分的覆盖和支援累加，只计算移动/交换的变化部分；保留
原单位顺序及补偿求和，避免直接浮点相减改变零覆盖判定。支援使用整数计数并排除
自身。候选行的支援/治疗同样使用决策内整数查询表，按移走/换入的单位修正；
单位缓存键只包含公开默认几何真正读取的字段。路线汇总直接接收移除索引和有序追加
单位，避免每候选构造完整棋盘后再寻找未变化部分。候选生成与结构化编码在单次决策内
复用地形判断、单位定义、格子索引、数值/类别模板和路由结果。
动作ID使用最多16384项的进程内LRU；标准移动动作以完整字段数量和精确标量类型
匹配固定字段键，其他输入仍使用通用内容键/原JSON路径，复用原JSON/SHA结果；
字典被修改会重新取键，跨进程仍独立核对，不缓存可变对象身份或略过捕获验证。
路线候选使用预分配数组和批量有限值检查，差值填完后再展开重复行，保留原候选顺序。
单位覆盖/支援数值表在同一路线布局内按实际输入值缓存，输入变化、布局变化及容量
边界均处理；棋盘支援表只复用到绑定棋盘改变。移动候选直接解析特征引用和语义，
其他动作复用决策内描述信息；目录身份检查不复制整条定义，对外record仍返回独立副本。
selected Candidate的数值特征构造时规范化为不可变tuple；同进程同一tuple复用验证结果，
维度和契约检查保留。验证标记不进入artifact或pipe，接收端和替换tuple后重新验证。
决策保留编码时history的不可变快照；capture复用当次完整状态验证产生的规范字节
来构造独立状态及核对摘要。跨进程重新验证，原始可变状态的修改仍会被检测。
运行中的完整源码哈希每秒检查一次，并在采集开始/结束、学习前/后及保存前强制检查；
时间、配置和可用磁盘检查继续逐次执行，检查点加载仍要求精确源码身份。

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

未新增测试、fixture或测试框架。初次拆分检查与后续获准的运行记录见
[VALIDATION](docs/VALIDATION.md)，详细迁移边界见 [EXTRACTION](docs/EXTRACTION.md)。

公开准备期高水位输入可通过独立 entity-policy schema 启用，含每盟约历史最高档位、
活跃层数、当前拆档及预计 ready 成本。采样保存当时 CPU 张量，PPO 按完整前缀重新
核对；原观测与旧检查点身份不改写。接口和验证边界见 [PREP-HISTORY](docs/PREP-HISTORY.md)。
