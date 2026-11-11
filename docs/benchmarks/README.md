# 性能测量

先执行 `python -m pip install -e ".[gym]"`，从仓库根目录导入以下函数。参数通过 Python 关键字传入，函数返回结果字典，`output` 指定报告路径。

## 多环境并行

Gym `AsyncVectorEnv` 和 Rust slots 均可继续用于向量化实验。slots 支持 1–64 个环境、显式局部重置及部分槽位推进，输出数组独立持有内存。战斗、公开账本、奖励、公开候选参数准备、枚举与过滤、数值编码在 Rust 中并行执行，直接复用单环境账本和候选模块。Python 负责快照绑定检查及独立输出数组的装配。批量绑定仅提供账本重置、当前候选编码、索引推进及空间读取；选中槽位全部初始化成功后统一提交重置。

```sh
PYO3_PYTHON="$(command -v python)" cargo build --release --locked --manifest-path crates/slots/Cargo.toml
```

```python
from benchmarks.bench_slots import run

if __name__ == "__main__":
    run(data_root="/path/to/0.3.3/data",
        reference_library="target/release/lib_native.dylib",
        slots_library="crates/slots/target/release/lib_native_slots.dylib")
```

上述动态库名适用于 macOS，Linux 使用 `.so`。Windows 使用 `.pyd`。

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

### 增量空间读取

`env.unwrapped.public_spatial_update(key=None)` 返回 `(key, geometry_json, dynamic_json)`。几何相同时 `geometry_json` 为 `None`，动态报文始终包含 schema、revision、回合、阶段、地图机变、当前预览关联及路径来源。key 依据公开地图、有效装置开关及当前模板生成，限定在同一不可变 catalog 内复用。原生 `NativeSolo` 还接受 `expected_revision`。slots 外观接受每槽 key 列表，底层为 `public_spatial_updates(revisions, keys)`，非决策槽位返回 `None`。现有完整空间报文保持原格式。

### 原生候选准备与索引执行

`NativeSolo.build_current_policy(revision)` 返回 `(token, actions_json, buffers)`，`step_index(revision, token, index, tick_budget, max_advances)` 执行该候选。批量对应 `build_current_policy_all(revisions)` 和 `step_index_all(revisions, tokens, indices, tick_budget, max_advances)`，暂停槽位的索引为 `None`。token 限定在所属原生实例内使用，跨 reset 单调递增；重新构造候选会替换旧 token，动作执行后候选缓存失效。每槽只保留当前一份、最多 16,384 个候选，清理在 GIL 外完成。批量索引在推进前整组验证。单环境动作字典入口与带参数的既有编码入口继续供记录回放和兼容调用。

## 原生战斗回放

按需导出战斗采样帧和可独立打开的 HTML，默认每 3 个逻辑帧采样一次。JSONL 包含动作、公开状态、地图、单位位置与血量、技能开启状态、弹道和终局结果。最大输出默认 128 MiB，达到上限会报告错误。

```python
from benchmarks.native_replay import run

run(data_root="/path/to/data", native_library="target/release/lib_native.dylib",
    core="band_chen", seed=42, output="/tmp/replay.jsonl", html_output="/tmp/replay.html")
```

默认使用购买部署诊断脚本。复现已有策略时可传入 `action_file="actions.json"`，文件形如 `{"core":"band_chen","engine_seed":123,"actions":[{"t":"g.ready"}]}`，其中 seed 是原生引擎种子。动作序列应覆盖完整对局，导出结果明确记录胜负。HTML 支持播放、拖动和倍速，也可通过 `native_replay.html` 选择 JSONL 文件。采样式回放使用网格、圆点和血条展示战斗。

`action_file` 也接受上述回放 JSONL，以及含初始种子和动作的 `arkguardrl/native-trajectory-v1` JSONL。导入旧轨迹时，原生会话负责开局配置，记录中的 `g.band` 必须与 `core` 相同。跨版本规则差异可能导致动作被拒绝或战果变化，导出时会执行动作并计算终局。通关验收添加 `require_victory=True`，实际终局失败时函数抛出异常。只有画面或状态快照的轨迹可用于播放，重新执行还需要种子和动作。

Huawei 的 `stronghold-human-seat-trajectory/v1` JSONL 也可作为 `action_file` 输入，使用完整单人绝境记录的原始种子、固定 `diy` 配置和开局重刷次数。导入时移除 `rid`，帧导出省略原记录中被拒绝的动作。当前支持阿米娅、萨卡兹、陈和休露丝核心。自选配置沿用四个槽位 ID，商店、库存、合成、出售与盟约读取组装后的干员数据。当前自选战斗技能实现为克莱门莎 S3，其他自选干员已有数值组装，部署到战斗时会报告尚未支持。

## 真实单人记录回归

使用 Huawei 的完整单人绝境轨迹进行回归，轨迹目录由调用者提供。

```python
from benchmarks.human_records import run

run(records_dir="benchmarks/local/huawei-solo", data_root="/path/to/data",
    native_library="target/release/lib_native.dylib",
    output="benchmarks/local/human-records.json")
```

逐决策比较阶段、回合、LP、资金、棋子与装备库存、部署、盟约层数、选择和商店，并比较动作接受情况及最终胜负、通过回合数和隐藏关进度。显示元数据和战斗浮点累计统计在这个比较范围之外。只读的 `g.unitStats` 请求会跳过。缺少种子、尚未支持的核心与房间配置单独报告；漂移或执行错误会让函数抛出异常。

Gym 通过 `ArkGuardNativeEnv(..., diy=picks)` 固定配置。也可先调用 `NativeCatalog.with_diy(json.dumps(picks))`，将返回的目录同时交给 `NativeSolo` 和 `PublicCatalog.from_native`，保证运行数据与候选／编码元数据一致。自选配置须与公开状态完全匹配，既有观测及候选数组形状保持一致。

## Gym 吞吐

独立 env 测量使用 `.[gym]` 安装与 `gym_env.py`，不加载模型、Torch、PPO 或轨迹采集器：

```python
from benchmarks.gym_env import run

if __name__ == "__main__":
    run(engine_root="/path/to/engine", vector="sync", envs=1,
        output="artifacts/gym-sync-1.json")
    run(engine_root="/path/to/engine", vector="async", envs=4,
        output="artifacts/gym-async-4.json")
```

动作在有效候选行中均匀抽样，每个 env 使用独立 RNG。每次重复从相同游戏/RNG 种子重新开始；
不截断候选域。吞吐计入循环内 reset，单列初始 reset 与启动/预热时间。`steps` 是每个 env 的真实
决策次数，reset 不计作 step；自然结束与预算截断分别计数。分项时间是所有 env 的累计值，
异步运行时不能当作墙钟时间占比。观测填充与 Gym 通信也计入测量，因此结果不是纯引擎速度。
不同策略负载分别报告吞吐。

## 原生绝境单环境

```python
from benchmarks.native_session import run

run(data_root="/path/to/Stronghold-Protocol/data",
    native_library="target/release/lib_native.dylib",
    output="benchmarks/local/native.json")
```

此脚本不加载模型、不训练、不并发调度。三个核心分别以 Gym seed 42、99 生成完整动作序列，
在原生会话与完整 Gym 接口重放，并核对终局公开状态相同。两种负载是空阵容直接准备，
以及每回合最多买一名干员并部署。每种配置预热后测量三次，每次十局，共 360 个计时 episode。
计时包括 reset、动作索引查找及公开观测，不含初始化、预热和终局一致性断言；
原生接口数据包含 Python/JSON 调用开销，不是纯 Rust 内核计时。

### 与当前原版 Node 直接对照

```python
from benchmarks.node_compare import run

run(reference_root="/path/to/Stronghold-Protocol",
    native_library="target/release/lib_native.dylib",
    output="benchmarks/local/node-comparison.json")
```

使用独立测速桥接的常驻 Node Match，与原生 PyO3 会话重放相同种子、动作；逐局核对终局公开字段。
每种负载两端各预热四局，随后交替测量顺序，各测三次、每次十局。两端均不做 Gym 特征编码，
均包含 reset 和公开观测；Node 包含 JSON 管道通信和开局确认/核心配置，不计进程启动与数据加载。
计时范围覆盖接口端到端开销。

## 跨平台构建

`native-build` CI 在每次 main PR、main 推送或手动触发时，编译并导入 `_native`、`_rust_routes`、`_native_slots` 三个模块。覆盖 CPython 3.12，以及 Linux x64、Windows x64、macOS Intel／Apple Silicon，产物名为 `native-py<版本>-<平台>`。

Linux 使用 Ubuntu 22.04（glibc 2.35）作为构建基线。迁移 AutoDL 时选择匹配 Python 版本与架构的产物，将三个模块放入同一目录，分别向运行时传入 `_native` 与 `_native_slots` 的路径；特征模块放入 `arkguardrl_core` 包目录。外部 0.3.3 数据另行配置。不同 Python ABI 或更旧的 glibc 环境可在目标机器执行以下构建命令。

```sh
export PYO3_PYTHON="$(command -v python)"
cargo build --release --locked -p arkguard-python
cargo build --release --locked --manifest-path crates/routes/Cargo.toml
cargo build --release --locked --manifest-path crates/slots/Cargo.toml
```
