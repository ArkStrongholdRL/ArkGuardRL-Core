# 构建与使用

先执行 `python -m pip install -e ".[gym]"`，以下函数从仓库根目录调用。

## 跨平台构建

```sh
export PYO3_PYTHON="$(command -v python)"
cargo build --release --locked -p arkguard-python
cargo build --release --locked --manifest-path crates/routes/Cargo.toml
cargo build --release --locked --manifest-path crates/slots/Cargo.toml
```

CI 在 PR、main 推送和手动触发时构建并导入三个模块。Python 3.12 产物覆盖 Linux x64、Windows x64、macOS Intel／Apple Silicon，名称为 `native-py3.12-<平台>`。Linux 基线为 Ubuntu 22.04／glibc 2.35，其他 ABI 或更旧系统可在目标机器构建。

示例采用 macOS 的 `.dylib` 路径。Linux 使用 `.so`，Windows 使用 CI 的 `.pyd`。`_rust_routes` 安装到 `arkguardrl_core` 包内，`_native` 和 `_native_slots` 的路径通过调用参数指定，外部 0.3.3 数据目录由 `data_root` 指定。

## 多环境

```python
from benchmarks.bench_slots import run

if __name__ == "__main__":
    result = run(data_root="/path/to/data",
                 reference_library="target/release/lib_native.dylib",
                 slots_library="crates/slots/target/release/lib_native_slots.dylib")
```

Rust slots 支持 1–64 个环境、局部 reset 和部分槽位 step。Gym Async 使用 `spawn`，调用入口需放在主模块保护内。

## 空间接口

`env.unwrapped.public_spatial()` 返回当前公开地图、装置、模板路线及敌人预览关联。`public_spatial_update(key=None)` 返回 `(key, geometry_json, dynamic_json)`，几何复用时 `geometry_json` 为 `None`。

- 坐标为原始 `[row, col]`，加 `boardOffset` 得到部署坐标，`fieldBounds` 两端包含。
- `previewRoutes[].routeCandidates` 引用 `routes` 下标。`base` 为基础路线，`extra` 为机制额外路线。
- `segments[].groundCells` 为默认地面路径，飞行段使用 `from`／`to`。`repeatFrom` 标记巡逻回环，`E_NUM` 占位的分段为空。
- 输入来自当前地图与公开预览。单位能力、阻挡和装置损坏可改变实际移动。
- 入口用于 PREP／SP_DRAFT，支持最多 64 条路线、每条 256 个检查点、总计 65,536 个路径格及 8 MiB JSON。

原生候选通过 `build_current_policy(revision)` 获取 `(token, actions_json, buffers)`，再以 `step_index(revision, token, index, tick_budget, max_advances)` 执行。重新编码、推进或 reset 后旧 token 失效，每槽最多保留 16,384 个候选。批量入口加 `_all` 后缀，暂停槽位的索引为 `None`。

## 回放

```python
from benchmarks.native_replay import run

run(data_root="/path/to/data", native_library="target/release/lib_native.dylib",
    core="band_chen", output="/tmp/replay.jsonl", html_output="/tmp/replay.html")
```

默认使用购买部署策略。指定 `action_file` 可回放含种子与动作的 JSON、原生轨迹或 Huawei 单人绝境 JSONL。JSON 格式为 `{"core":"band_chen","engine_seed":123,"actions":[{"t":"g.ready"}]}`，动作需覆盖完整对局。

默认每 3 帧采样一次，输出上限 128 MiB，HTML 支持播放、拖动和倍速。`require_victory=True` 在败局时抛异常。自选数值组装已接入，战斗技能当前支持克莱门莎 S3。

## 测速与回归

各模块提供 `run(...)`，通过关键字传参并返回结果，`output` 指定报告路径。

| 模块 | 用途 | 必填参数 |
| --- | --- | --- |
| `benchmarks.native_session` | 原生会话与完整 Gym | `data_root`, `native_library`, `output` |
| `benchmarks.node_compare` | 同种子／动作的 Node 对照 | `reference_root`, `native_library`, `output` |
| `benchmarks.human_records` | 真实轨迹逐决策与终局回归 | `records_dir`, `data_root`, `native_library`, `output` |
| `benchmarks.gym_env` | Node v0 Gym Sync／Async | `engine_root` |

原生测速覆盖空阵容和购买部署，计入 reset、公开观测及接口开销，预热和初始化单独处理。Node 对照复用常驻 Match。回归比较动作接受结果、决策状态和最终战果，漂移或执行错误会抛异常。

首页图表为 2026-11-10 的 Rust 空间编码缓存 A/B，完整对局含空间张量化、五轮中位数。单环境 Gym 使用种子 42／99，171.3 → 341.9 决策／秒。四环境四线程 slots 使用种子 42–45，188.3 → 517.3 决策／秒。两组分别比较缓存前后，计时覆盖环境与数据准备，模型前向和训练另行计时。
