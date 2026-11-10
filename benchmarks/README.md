# 性能测量

## 多环境并行

Gym `AsyncVectorEnv` 和 Rust slots 均可继续用于向量化实验。slots 支持 1–64 个环境、显式局部重置及部分槽位推进，输出数组独立持有内存。战斗、公开账本、奖励、公开候选参数准备、枚举与过滤、数值编码在 Rust 中并行执行，直接复用单环境账本和候选模块。Python 负责快照绑定检查及独立输出数组的装配。批量绑定仅提供账本重置、当前候选编码、索引推进及空间读取；选中槽位全部初始化成功后统一提交重置。

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

### 增量空间读取与张量缓存

`env.unwrapped.public_spatial_update(key=None)` 返回 `(key, geometry_json, dynamic_json)`。几何相同时 `geometry_json` 为 `None`，动态报文始终包含 schema、revision、回合、阶段、地图机变、当前预览关联及路径来源。key 依据公开地图、有效装置开关及当前模板生成，限定在同一不可变 catalog 内复用。原生 `NativeSolo` 还接受 `expected_revision`。slots 外观接受每槽 key 列表，底层为 `public_spatial_updates(revisions, keys)`，非决策槽位返回 `None`。现有完整空间报文保持原格式。

研究侧 `SpatialCache.read(environment, decision.state)` 持有一份当前几何及其张量，地图／模板变化时重建。批量入口用 `SpatialCache.update(packet, state)` 消费每槽增量包。候选空间行从 36 个目标格的预编码表批量索引，预览关联及 summary 的预览计数逐次更新。返回张量独立持有内存，布局与数值保持一致。BC 和在线采集入口已接线，研究侧改动在 ArkGuardRL 工作树。

63 份真实记录、9,262 个决策中，完整报文保持逐字节一致，增量字段合并后与完整报文一致，8,433 次复用几何、829 次重建。四核心 137 个决策的全部空间张量与改动前逐字节一致，另覆盖四核心 72 个槽位快照、1／4 线程及局部 reset／step。9 项 Rust 测试通过。

陈核心完整对局、5 轮交替 A/B，双方均包含完整空间张量化。单环境吞吐为 171.3 → 341.9 决策/秒（2.00 倍），空间读取与编码平均 3.410 → 0.470 ms；四环境四线程为 188.3 → 517.3 决策/秒（2.75 倍），空间部分平均 3.926 → 0.466 ms/决策。包含冷缓存，未计模型前向或训练。这组负载增加了空间张量化，前文较早的吞吐仅包含空间 JSON 读取。[单环境 A/B](local/spatial-tensor-cache-ab-20261110.json) [四环境 A/B](local/spatial-tensor-cache-batch-ab-20261110.json) [真实记录](local/spatial-update-records-20261110.json)

### 原生候选准备与索引执行

`NativeSolo.build_current_policy(revision)` 返回 `(token, actions_json, buffers)`，`step_index(revision, token, index, tick_budget, max_advances)` 执行该候选。批量对应 `build_current_policy_all(revisions)` 和 `step_index_all(revisions, tokens, indices, tick_budget, max_advances)`，暂停槽位的索引为 `None`。token 限定在所属原生实例内使用，跨 reset 单调递增；重新构造候选会替换旧 token，动作执行后候选缓存失效。每槽只保留当前一份、最多 16,384 个候选，清理在 GIL 外完成。批量索引在推进前整组验证。单环境动作字典入口与带参数的既有编码入口继续供记录回放和兼容调用。

Gym 和 slots 已接入此路径。研究侧 `open_session` 改为按 Python／原生 API 能力选择 Rust 账本，在线采集使用索引执行；旧构建回退至可用入口。仅有完整空间投影的原生构建也能通过完整报文适配空间缓存。这两处研究接线在 ArkGuardRL 工作树。

63 份记录共 9,262 次动作，新旧执行报文逐字节一致。原特征接口可编码的 9,183 个决策，候选顺序与四类数值 buffer 一致，其中 8,979 次使用原生索引执行。另 204 次记录动作位于原有候选域外，通过字典入口回放；一份自选记录的 79 个状态仍受原有自选特征定义缺失限制，两路均无法编码。四核心 384 个槽位快照，以及旧 token、越界索引、整批非法选择、局部 reset／step、错误恢复与旧构建兼容检查通过。

同负载交替 A/B，Gym 420.7 → 437.2 决策/秒（+3.9%，不含空间张量化），四环境四线程 slots 555.9 → 585.7（+5.4%，含空间 JSON 读取）。实际研究采集入口包含完整空间张量化，从 293.0 → 338.5（+15.5%）；该项同时计入原生账本接线的收益。均为完整购买部署对局，未计模型前向或训练。[Gym A/B](local/native-domain-gym-ab-20261110.json) [slots A/B](local/native-domain-slots-ab-20261110.json) [采集入口 A/B](local/native-domain-research-ab-20261110.json) [记录对照](local/native-domain-records-20261110.json)

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

## 真实单人记录回归

当前以 Huawei 的完整单人绝境轨迹作为主要回归输入，保留 9 项针对性 Rust 测试，覆盖预算、回执、快照隔离及已发现的重入错误。本机轨迹存放在忽略目录 `benchmarks/local/huawei-solo/`。

```sh
PYTHONPATH=. python benchmarks/human_records.py \
  --records benchmarks/local/huawei-solo --data-root /path/to/data \
  --native-library target/release/lib_native.dylib \
  --out benchmarks/local/human-records.json
```

逐决策比较阶段、回合、LP、资金、棋子与装备库存、部署、盟约层数、选择和商店，并比较动作接受情况及最终胜负、通过回合数和隐藏关进度。显示元数据和战斗浮点累计统计在这个比较范围之外。只读的 `g.unitStats` 请求会跳过。缺少种子、尚未支持的核心与房间配置单独报告；漂移或执行错误会让命令返回非零退出码。

当前 63 份记录共 9,262 个决策，61 份一致、2 份已知漂移、0 执行错误；全部最终胜负、通过回合数及隐藏关进度一致。[回归结果](local/huawei-pr-cleanup-regression-20261110.json)

剩余两份沿用前次处理。`7929e734` 保留快照遍历，完整命中原版断崖因删除数组元素而漏打的目标。`1c8ee89d` 保留寒风先施加寒冷、冰泪随后检查的同帧顺序。验收仍以逐决策状态和最终战果为准。`320cd8b5` 的逐帧诊断还可见 35.70 秒原版属性缓存延迟刷新，以及再生流形在 66.07 秒复制技能晚一帧，均未造成决策状态差异。

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
不同策略负载分别报告吞吐。

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

### 与当前原版 Node 直接对照

```sh
python benchmarks/node_compare.py --reference-root /path/to/Stronghold-Protocol \
  --native-library target/release/lib_native.dylib \
  --out benchmarks/local/node-comparison.json
```

使用独立测速桥接的常驻 Node Match，与原生 PyO3 会话重放相同种子、动作；逐局核对终局公开字段。
每种负载两端各预热四局，随后交替测量顺序，各测三次、每次十局。两端均不做 Gym 特征编码，
均包含 reset 和公开观测；Node 包含 JSON 管道通信和开局确认/核心配置，不计进程启动与数据加载。
计时范围覆盖接口端到端开销。
