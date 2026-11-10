# 性能测量

## 多环境并行

Gym `AsyncVectorEnv` 和 Rust slots 均可继续用于向量化实验。slots 支持 1–64 个环境、显式局部重置及部分槽位推进，输出数组独立持有内存。战斗与数值编码在 Rust 中并行执行，slots 的公开账本目前仍由 Python 管理。

```sh
PYO3_PYTHON="$(command -v python)" cargo build --release --locked --manifest-path benchmarks/native_slots/Cargo.toml
PYTHONPATH=.:benchmarks python benchmarks/bench_slots.py \
  --data-root /path/to/0.3.3/data \
  --reference-library target/release/lib_native.dylib \
  --slots-library benchmarks/native_slots/target/release/lib_native_slots.dylib
```

上述动态库名适用于 macOS，Linux 使用 `.so`。Windows 构建会一并产出三个 `.pyd` 模块。

## 公开空间输入

在 PREP／SP_DRAFT 决策边界调用 `env.unwrapped.public_spatial()`。原生会话支持 `session.public_spatial(expected_revision=None)`，返回 JSON 字符串。slots 外观的 `batch.public_spatial()` 返回按槽位排序的字典列表，非决策槽位为 `None`，底层批量入口为 `public_spatial_all()`。

`arkguardrl/public-spatial/v1` 包含当前 `revision`、回合、阶段、地图、399 个地格、公开装置及地图机变、当前公开模板的路线集合，以及按 `own.nextEnemies` 原顺序排列的 `previewRoutes`。每项 `routeCandidates` 引用输出 `routes` 的下标，保留可能关联的完整集合。该关联由入口、飞行标记和公开 Boss 起点筛选。

```python
obs, info = env.reset(seed=42)
spatial = env.unwrapped.public_spatial()
for route in spatial['routes']:
    print(route['motion'], route['start'], route['end'], route['checkpoints'])
```

所有坐标使用地图原始 `[row, col]`。`fieldBounds` 为 `[r0, c0, r1, c1]`，端点均包含。地图坐标加 `boardOffset` 得到公开部署坐标，普通场为 `[0, 0]`，单人 Boss 左场为 `[7, 0]`。399 个地格保留完整静态地图，`groundPassable`／`flyPassable` 表示当前场地范围内的通行性，`elevated` 标识已启用的高台／土堆装置。

每条路线保留类型化检查点及分段几何。`routeSet` 区分基础路线 `base` 与当前模板中供机制使用的额外路线 `extra`，预览关联仅指向基础路线。额外路线中的 `E_NUM` 保留为尚未具体化的模板占位，分段列表为空。`segments[].groundCells` 是在公开地图及当前装置配置上计算的默认地面路径，`null` 表示飞行直线段或地图上没有地面通路。飞行段由 `from`／`to` 定义。`repeatFrom` 标识巡逻回到的检查点，末段为回环连接。该接口提供基准空间事实，单位能力、阻挡和战斗中装置损坏可改变实际移动。

模板只依据当前模式、回合及敌人预览中已显示的 Boss 身份选择。接口读取地图静态数据和公开预览，不读取生成后的出怪路线分配、未来回合或战斗单位轨迹。读取不推进会话，不更改既有观测和候选布局。每个会话仅保留当前地图与路线的一份缓存，按地图身份、场地类型和有效装置开关重建地图，按当前模板变化重建路线。revision、阶段、地图机变标识和敌人预览关联逐次刷新，地图与路线 JSON 字段复用已序列化的缓存。返回值仍独立持有内存。

接口上限为 64 条路线、每条 256 个检查点、总计 65,536 个地面路径格、399 个装置及 8 MiB JSON。超限、revision 不匹配或非决策阶段调用会报错。

2026-11-10 的缓存前后对照覆盖 63 份真实记录、9,262 个决策，空间报文逐字节一致，动作回执、内部诊断与终局状态一致。陈核心 seed 42/99、完整购买部署脚本每决策读取空间信息，5 轮交替测量中位吞吐从 258.8 提升至 340.6 决策/秒（+31.6%）。原生空间构建及传输平均从 1.003 ms 降至 0.095 ms，Python JSON 解析约 0.38 ms。该测速未接入模型。[本机 A/B](local/public-spatial-cache-ab-20261110.json) [逐字节对照](local/public-spatial-cache-parity-20261110.json)

## 原生战斗回放

按需导出战斗采样帧和可独立打开的 HTML，默认每 3 个逻辑帧采样一次。JSONL 包含动作、公开状态、地图、单位位置与血量、技能开启状态、弹道和终局结果。最大输出默认 128 MiB，达到上限会报告错误。

```sh
PYTHONPATH=. python benchmarks/native_replay.py --data-root /path/to/data \
  --native-library target/release/lib_native.dylib --core band_chen --seed 42 \
  --out /tmp/replay.jsonl --html /tmp/replay.html
```

默认使用购买部署诊断脚本。复现已有策略时可传入 `--actions actions.json`，文件形如 `{"core":"band_chen","engine_seed":123,"actions":[{"t":"g.ready"}]}`，其中 seed 是原生引擎种子。动作序列应覆盖完整对局，导出结果明确记录胜负。HTML 支持播放、拖动和倍速，也可通过 `native_replay.html` 选择 JSONL 文件。采样式回放使用网格、圆点和血条展示战斗。

`--actions` 也接受上述回放 JSONL，以及含初始种子和动作的 `arkguardrl/native-trajectory-v1` JSONL。导入旧轨迹时，原生会话负责开局配置，记录中的 `g.band` 必须与 `--core` 相同。跨版本规则差异可能导致动作被拒绝或战果变化，导出时会执行动作并计算终局。通关验收添加 `--require-victory`，实际终局失败时命令返回错误。只有画面或状态快照的轨迹可用于播放，重新执行还需要种子和动作。

Huawei 的 `stronghold-human-seat-trajectory/v1` JSONL 也可作为 `--actions` 输入，使用完整单人绝境记录的原始种子、固定 `diy` 配置和开局重刷次数。导入时移除 `rid`，帧导出省略原记录中被拒绝的动作。当前支持阿米娅、萨卡兹、陈和休露丝核心。自选配置沿用四个槽位 ID，商店、库存、合成、出售与盟约读取组装后的干员数据。当前自选战斗技能实现为克莱门莎 S3，其他自选干员已有数值组装，部署到战斗时会报告尚未支持。

2026-11-10，在 Huawei 归档中筛到 20 份完整单人绝境胜利，当前支持核心中有 8 份带种子的 0.3.3 记录。已用萨卡兹种子 `4093799481` 和阿米娅种子 `3216653761` 各重执行一局，两局均实际通过 14 回合主线，分别导出 5,764 和 5,988 个采样帧。旧版记录已按要求从本地语料移除。真实通关重执行同时覆盖了弹道击杀后追加弹道，以及移动阶段回调查询剩余路径的重入场景。

## 真实单人记录回归

当前以 Huawei 的完整单人绝境轨迹作为主要回归输入，合成 Rust 测试从 23 项精简到 8 项，保留预算、回执、快照隔离及已发现的重入错误检查。本机轨迹存放在忽略目录 `benchmarks/local/huawei-solo/`。

```sh
PYTHONPATH=. python benchmarks/human_records.py \
  --records benchmarks/local/huawei-solo --data-root /path/to/data \
  --native-library target/release/lib_native.dylib \
  --out benchmarks/local/human-records.json
```

逐决策比较阶段、回合、LP、资金、棋子与装备库存、部署、盟约层数、选择和商店，并比较动作接受情况及最终胜负、通过回合数和隐藏关进度。显示元数据和战斗浮点累计统计在这个比较范围之外。只读的 `g.unitStats` 请求会跳过。缺少种子、尚未支持的核心与房间配置单独报告；漂移或执行错误会让命令返回非零退出码。

2026-11-10 下载了 183 份完整记录，按要求删除跳过与旧版的本地记录，保留 61 份 0.3.3 单人绝境记录。召唤物修复前为 29 份一致、15 份漂移、17 份执行错误。补齐召唤物发放、堆叠、主人绑定与移动回收，并接入战斗初始部署、赫默探机和凯瑟琳护盾后，共执行 8,652 个决策，结果为 35 份一致、21 份漂移、5 份执行错误。原先一致的 29 份全部保持一致。[本机完整结果](local/huawei-summons-regression-20261110.json)

后续修复深巡阻挡目标触发、古米天赋倍率、精英标记、蕾缪安通缉引用回收、洛洛无人机选敌、溯光星源停顿继承和法术伤害类型初始化。本轮继续修正玛恩纳游侠倍率的扣防顺序、分裂子体进入索敌表的相位、风丸替身出现伤害的格子范围，以及伊内丝影哨反隐对隐匿敌人的覆盖。

当前 63 份记录共执行 9,262 个决策，61 份一致、2 份漂移、0 份执行错误，全部最终胜负、通过回合数及隐藏关进度一致。此前一致的 58 份保持一致。`c7d0c7f3` 使用休露丝核心、四个固定自选配置和四次开局重刷，克莱门莎 S3 在第 13 回合实际部署。修正技能触发过滤后，Rust 与原版重执行均与该记录的 284 个决策状态全部一致。原记录与 Rust 均为通过 13 回合后失败。更新后的回放共 11,906 帧，其中 3,075 帧可见克莱门莎，2,370 帧技能开启。[完整结果](local/huawei-pr-cleanup-regression-20261110.json) [克莱门莎部署回放](local/huawei-clemnt-deployment.html)

本轮修复三处。`ACTIVE_RANGE` 使用干员本身的选敌能力，忍冬不再因阿罗玛浮空的敌人提前半秒开技能。`SKILL_RANGE` 按原版规则只检查范围内是否存在存活敌人，允许隐匿／不可选中目标触发，修复凛御银灰延迟开技能。敌人漏怪离场后清理伊内丝偷攻收益，修复 `6b91328a` 在 60.60 秒多出 110 点攻击伤害，该记录现已完全一致。

继续定位后修复两份记录。`18dc27da` 中荒芜拉普兰德浮游炮的死亡目标被提前回收，随机重现位移被丢弃，导致重选目标和恐惧方向变化。现在通过现有引用保留机制保存目标最终位置，重选与技能结束时释放，第 11 回合逐帧诊断中的位置、HP、技能状态和攻击时刻均一致。`320cd8b5` 中缇缇 S3 错把召唤物纳入免死保护，且流形再生持续尝试复用已永久退场的对象。补齐干员类型检查，并让召唤接口跳过已退场对象后，该记录全部 145 个决策一致。`18eed0b8` 的 LP 漂移也随本轮修复消失。

剩余两份沿用前次处理。`7929e734` 保留快照遍历，完整命中原版断崖因删除数组元素而漏打的目标。`1c8ee89d` 保留寒风先施加寒冷、冰泪随后检查的同帧顺序。验收仍以逐决策状态和最终战果为准。`320cd8b5` 的逐帧诊断还可见 35.70 秒原版属性缓存延迟刷新，以及再生流形在 66.07 秒复制技能晚一帧，均未造成决策状态差异。

PR 前的运行时清理保持 63 份记录的完整报告不变。另对照 Python／Rust 账本的 38 个决策，以及四种核心的单环境／slots 72 个状态快照，候选顺序、数值数组和奖励字节一致，覆盖 1／4 线程、局部重置及部分槽位推进。三个原生库均通过 macOS release 构建。

2026-11-10 再扫 Huawei 共 340 份轨迹。本地此前最新记录之后新增 36 份单人绝境记录，其中 26 份完整、10 份未完成。25 份完整记录使用当前未实现的核心，余下的阿米娅固定自选配置记录现已加入。本轮统计 0.3.3 自选配置共 12 位干员、18 种技能／模组组合，跨两档槽位共 72 份普通／精英数据与原版逐字段一致。单人绝境完整自选记录中，实际部署过的是克莱门莎 S3，另有阿斯卡纶商店展示。

召唤物缺卡及相关 UID 动作错误已在语料中消除。新增一致记录包括 `4337fae1-1e3a-46d5-9f52-b2608f01a819-seat0.jsonl` 的 14 回合胜利，包含医疗探机手牌，原策略未部署探机。其成功动作另行导出为可渲染回放，省略了原记录的两个拒绝动作。[通关回放](local/huawei-summons-clear.html)

另用 `18eed0b8-9891-4b21-97fd-77dd0a51016a-seat0.jsonl` 验证探机实际部署、治疗、10 秒退场及库存再部署，导出 1,151 个采样帧。这份原记录与原生重执行均为败局，13 回合通过，仍有第 13 回合起的 LP 差异。HTML 已检查内嵌数据，未完成浏览器视觉验收。[探机部署回放](local/huawei-drone-deployment.html)

Gym 通过 `ArkGuardNativeEnv(..., diy=picks)` 固定配置。也可先调用 `NativeCatalog.with_diy(json.dumps(picks))`，将返回的目录同时交给 `NativeSolo` 和 `PublicCatalog.from_native`，保证运行数据与候选／编码元数据一致。自选配置须与公开状态完全匹配，既有观测及候选数组形状保持一致。

## Gym 吞吐

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

## 原生绝境单环境

```sh
python benchmarks/native_session.py --data-root /path/to/Stronghold-Protocol/data \
  --native-library target/release/lib_native.dylib --cycles 5 --repeats 3 \
  --out benchmarks/local/native.json
```

此脚本不加载模型、不训练、不并发调度。三个核心分别以 Gym seed 42、99 生成完整动作序列，
在原生会话与完整 Gym 接口重放，并核对终局公开状态相同。两种负载是空阵容直接准备，
以及每回合最多买一名干员并部署。每种配置预热后测量三次，每次十局，共 360 个计时 episode。
计时包括 reset、动作索引查找及公开观测，不含初始化、预热和终局一致性断言；
原生接口数据包含 Python/JSON 调用开销，不是纯 Rust 内核计时。

2026-11-09，Apple M5 / 16 GiB，Python 3.13.14，release 构建，默认 16384 候选容量：

| 负载 | 接口 | 决策/秒 | episode/秒 | episode 平均耗时 |
| --- | --- | ---: | ---: | ---: |
| 空阵容 | 原生会话 | 108.6 | 20.36 | 49.1 ms |
| 空阵容 | 完整 Gym | 96.5 | 18.09 | 55.3 ms |
| 购买部署 | 原生会话 | 182.3 | 10.13 | 98.7 ms |
| 购买部署 | 完整 Gym | 137.7 | 7.65 | 130.7 ms |

表中跨核心合并每次测量后取三次吞吐中位数，耗时由对应吞吐换算。购买部署的原生战斗推进约
11.4 万帧/秒（逻辑帧率 30 Hz，约 3808 倍实时）；Gym step 分项累计时间中，环境调用路径占
85.1%、观测/候选编码占 13.4%、奖励占 1.5%。环境调用路径还包含公开状态处理，不能全算作
战斗内核。整个测量进程峰值 RSS 303.5 MiB，包含 Python、数据目录、两种接口及临时观测，
不能当作单环境增量内存。每份填充观测 2,984,640 字节，最大实际候选数 665。

这些原数据对局在第 3–8 回合自然结束；不是 15 回合通关或满编高层数压力测量，也不能与上面的
旧后端随机动作负载直接比较倍率。没有截断、拒绝或重放状态差异。详细本机结果保存在
`benchmarks/local/native-20261109.json`，不纳入版本控制。

### 与当前原版 Node 直接对照

```sh
python benchmarks/node_compare.py --reference-root /path/to/Stronghold-Protocol \
  --native-library target/release/lib_native.dylib \
  --out benchmarks/local/node-comparison.json
```

使用独立测速桥接的常驻 Node Match，与原生 PyO3 会话重放相同种子、动作；逐局核对终局公开字段。
每种负载两端各预热四局，随后交替测量顺序，各测三次、每次十局。两端均不做 Gym 特征编码，
均包含 reset 和公开观测；Node 包含 JSON 管道通信和开局确认/核心配置，不计进程启动与数据加载。
这是当前接口的端到端比较，不是纯语言或战斗内核比较。

2026-11-09，同一 M5，Node v26.7.0，原生 release：

| 负载 | 原生 episode/s | Node episode/s | Node / 原生 |
| --- | ---: | ---: | ---: |
| 空阵容 | 19.75 | 55.23 | 2.80× |
| 购买部署 | 9.55 | 19.02 | 1.99× |

共 360 个计时 episode，终局对照均通过。**当前原生实现没有性能优势，反而更慢。**
原生计时中约 94–96% 位于带战斗推进的 step 调用，reset 仅占约 2–5%；不能将差距归因于 Gym 编码。
尚未通过内核剖析定位具体热点。范围仍为第 3–8 回合结束的测试脚本，不能推断后期满编倍率。
本机详细结果：`benchmarks/local/node-comparison-20261109.json`。

### 首轮热点优化

干员管理结构改为稳定堆存储，回调暂取所有权时只移动指针，避免每次阻挡检查搬运内嵌占位数组。
空间索引复用碰撞体/格子临时列表，将每帧重建的两张树表合为复用容量的查询表；查询表不参与
遍历顺序，目标顺序仍由原有桶的插入顺序决定。容量上限、重复敌人检测及失败后索引失效语义保留。

相同机器、动作与测速方法的三次吞吐中位数：

| 负载 | 优化前原生 episode/s | 优化后原生 episode/s | 原生提升 | 本轮 Node episode/s |
| --- | ---: | ---: | ---: | ---: |
| 空阵容 | 19.75 | 29.32 | 48.5% | 54.49 |
| 购买部署 | 9.55 | 12.26 | 28.4% | 17.84 |

尚未超过 Node；本轮购买部署负载下 Node 仍快约 1.45 倍。结果为
`benchmarks/local/hot-index-20261109.json`，只改稳定堆存储的中间测量为
`benchmarks/local/hot-box-20261109.json`。复采样显示内存搬运已下降，分配/释放和空间索引仍是后续热点。

506 项库测试、53 项完整 Field 对照、碰撞体/阻挡/索敌对照及 15 项 Python 集成测试通过；
新增索引重复输入失败后恢复、位置变化、隐匿及退场清理回归。24 组整局对照和测速重放保持一致。

测试源码及原版对照夹具已按要求移除；上面的测试通过记录是移除前的历史结果。
测速入口保留，Node 桥接位于 `benchmarks/node_session.mjs`，Python 客户端不依赖测试框架。
