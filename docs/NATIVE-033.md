# 0.3.3 原生迁移

目标：Rust 完整环境内核 + Python 薄 Gymnasium 接口。行为基线为 Stronghold Protocol
`41bbca5712ac08394bbc4d5b4e3411c6be1634b6`（0.3.3）；训练保留在 ArkGuardRL。
运行数据由用户另行提供，不打包素材、权重或轨迹。

**目前不能发布为完整原生环境。** 正式安装仍提供旧 Node 环境；`crates/python` 只有数据目录
和种子派生的开发绑定，未注册 `ArkGuardRL-v1`。完整局内状态机、原生批量接口、
逐决策整局对照及完整 env 单线程至少 5 倍的性能验收都尚未完成。

## 已实现

`crates/engine` 不依赖 Python、Node、游戏源码目录或 Git：

- 外部 JSON 目录、稳定定义 ID、mulberry32/种子派生、确定性数学及有界事件队列。
- 地形/寻路、单位属性、HP/治疗/伤判、元素损伤、状态与增益、部署/重部署、阻挡与索敌。
- 1/30 秒阶段调度、技能/SP 生命周期、攻击锁定、直接/群体/链式攻击及追踪弹道。
- 路线/传送门、恐惧/诱导、推拉碰撞、失衡、敌方攻击前后摇及命中时属性读取。
- 波次转换、刷怪、预统计/运行时敌人、击杀/漏怪/赏金、超时扣除未生成敌人。
- 78 种敌人的完整能力组合：原版纯属性敌人、基础隐匿、14 种频次单位、基础折射/镜膜，
  以及 7 种元素攻击敌人。未支持的内容报错，不退化为普通攻击。
- 保留原有进度奖励公式；其输入仍需由未来 match 层的真实结算产生。

各模块仍通过显式 Rust host 接口连接内容。数据归一化覆盖全部 249 个外部敌人定义，
**不代表其能力都已实现**。技能生命周期与攻击计算通过对照，也不代表原版干员技能已完成。
不迁移渲染、音画、特效或显示事件；只保留影响运算的前摇、伤害帧等时序。

## 具体执行入口

`battle::runtime::Field` 已直接拥有 World、RNG、刷怪、内容、增益和时钟，
独立执行刷怪→状态→敌人更新→内容 tick→退场回收→结算。
当前只支持普通单人场的**空友军棋盘**和上述敌人；干员、地图战斗装置、特殊地形、
Boss/部件尚未接入该入口，遇到未支持输入报错。它不是 Gymnasium 环境。

`run(tick_budget)` 同步返回推进步数、自然结束及预算耗尽；预算耗尽保留真实后继状态，
不转成超时或重开。技术错误使执行入口失效，不能继续采样或返回有效结算。
`report()` / `enemy_states()` 返回独立拥有的数据；它们是内部战斗诊断接口，不能作为策略观测。
资源容量按创建参数限制，超限报错而不裁剪。数据文件字节预算仍不等于解析后的总内存预算。

开发期可直接运行 Rust 入口，无需 Node：

```sh
cargo run -p arkguard-engine --release --example field -- /path/to/data input.json 1800
```

输入使用原版的 `stage` 或 `stageId`、`routes`、`spawns`、`seed`、`timeLimit`；
当前友军棋盘必须为空。例如地形可用 `{"rows":["..."]}` 的原版形状；
实际关卡包含尚未移植的装置/地形时会报错。
此工具用于开发运算，不计作完整环境或性能验收。

## 验证

```sh
cargo test -p arkguard-engine --offline --release
ARKGUARD_REFERENCE_ROOT=/path/to/game cargo test -p arkguard-engine --offline --release -- --include-ignored
PYO3_PYTHON=/path/to/python3 cargo build -p arkguard-python --offline
```

Node 仅在 `tools/reference/` 中充当开发期上游对照。数值、RNG、事件次序与连续状态逐位比较；
不通过放宽误差掩盖分支差异。连续战斗覆盖 96 组、23,040 tick；具体 Field 入口另对照
上游完整内容模式下的 78 个空棋盘战场。另检查预算暂停、结果所有权、reset 隔离、
退场回收、未实现内容拒绝及技术错误失效。

后续关键工作是完整干员/敌人/盟约/装备/Boss 内容、战斗与局内状态机连接，
再接合法动作、公开结构化观测、Gymnasium、批量执行及整局性能验收。
这些完成前保留现有可运行链路，不将旧接口冒充原生 `ArkGuardRL-v1`。
