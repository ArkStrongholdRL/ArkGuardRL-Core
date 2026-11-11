# AutoTavern 实现对照

更新于 2026-11-10。AutoTavern 部分依据源码分析，本项目部分依据当前实现和本机回放测量。AutoTavern 的本机吞吐尚待测量。参考仓库为 [MCG-NJU/AutoTavern](https://github.com/MCG-NJU/AutoTavern)。

## 可借鉴的设计

- 战斗按一次攻击及其触发链推进。我们每次购买部署决策平均推进约 540–625 个逻辑帧，包含移动、阻挡、弹道和周期效果。跨项目比较需统一 step 和整局的计量范围。[战斗循环](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/combat.rs)
- 攻击、生命等常用数值直接存储，卡牌通过紧凑 ID 访问。本项目的派生属性已经使用 `Stat` 索引数组，基础属性与修饰量仍保留字符串映射及原聚合顺序，标记使用字符串集合。下一步应依据查找和分发热点决定进一步类型化的范围。[状态](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/state.rs)、[卡牌](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/cards.rs)、[本项目属性实现](../../crates/engine/src/battle/stats.rs)
- 脚本预先登记事件监听位掩码，分发入口先检查全局监听掩码，再收集相关监听者。这个设计适合减少本项目的逐帧无关机制扫描。[脚本注册](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/script.rs)、[事件分发](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/engine.rs)
- Rust 从类型化状态直接生成数值观测。我们可以继续缩短状态到数组的路径，并维持现有模型输入维度、候选顺序和奖励语义。[观测编码](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/src/obs.rs)
- 其整局基准包含纯 Rust 脚本 bot，独立战斗基准复用终盘阵容。独立战斗循环跳过 `a == b` 的配对，吞吐需按实际执行场数理解。跨项目数据应分别标明 Rust 战斗、完整会话与 Gym 采样范围。[基准实现](https://github.com/MCG-NJU/AutoTavern/blob/main/crates/taverngym/examples/bench.rs)

## 本项目当前结果

Rust 账本、候选枚举及数值编码已接入运行路径。移动前缀缓存和全局安静窗口已实现，窗口根据交互范围、技能、Buff、出生与地形等边界回退到逐帧推进。

最新一轮复用了上一不可变状态的指纹，直接序列化借用的账本字段，复用了支援及战术调度名单缓冲区，并批量复制无需转义的字符串片段。Python 可变快照的修改检测继续参与运行。

`band_chen` 完整购买部署 Gym 回放使用种子 42/99，每轮 30 个循环，优化前后交替测量 8 轮，以下为吞吐中位数。加载、pilot 与预热在计时范围外。

| 部署 | 优化前决策/秒 | 优化后决策/秒 | 变化 |
|---|---:|---:|---:|
| 前排 | 404.8 | 415.8 | +2.7% |
| 后排 | 266.5 | 266.6 | 波动范围内持平 |

验证覆盖 20 项 Rust 测试、三核心 1,139 次随机决策、8 组边界场景共 116 次决策，以及 114 份逐字节一致的原生返回报文。结果支持当前负载上的一致性，吞吐收益仍依赖场景。[本机测速及验证数据](../../benchmarks/local/hotspot-optimization-summary-20261110.json)

剖析中战斗循环约占完整 Gym 主线程采样的 46%，Rust 标识序列化与哈希约占 7.5%，空间索引重建约占 3.1%，战术调度约占 3.0%。这些来自本轮优化前的采样，分组存在包含关系，不能相加或直接当作可实现的收益。[剖析方法与结果](../../benchmarks/local/hotspot-summary-20261110.json)

## 下一轮实施方向

第一步已接入 roster 的 Kit 与 Tick 监听集合。新增单位时登记，复制体定义替换时按 roster 顺序重建，全局事件保留细粒度 `observes` 和动态优先级。Kit 周期调度从登记的 Kit 集合取名单。按需战斗帧接口与 JSONL/HTML 导出工具已加入，使用方法见[回放说明](README.md#原生战斗回放)。

先优化事件监听者筛选，再处理周期调度，工作范围限定为绝境模式 `mode_single_abyss`。

1. 在 `Behavior` 与 roster 生命周期中维护事件类别的保守监听信息，优先覆盖 Tick 与全局事件分发。当前 `kit_event` 遍历 roster、读取定义后才执行 `observes(event)`，可以先按预登记信息缩小候选监听者集合。保留运行时的细粒度过滤和动态优先级计算。[分发入口](../../crates/engine/src/battle/runtime/kits/dispatch.rs)
2. 集合更新覆盖初始单位、召唤单位、定义替换与清理。按照当前 roster 顺序构造每次事件的监听者快照，保留同优先级顺序、死亡处理及嵌套回调语义。装备、成长和死亡目标的独立处理器也需纳入分发判断。
3. 随后根据实际启用的能力缩小周期调度集合，再评估到期时间调度。`dispatch_scheduled_kits` 当前会遍历 roster 并检查五档周期，优化需保持时钟递推、同帧顺序及错误传播。[周期调度入口](../../crates/engine/src/battle/runtime/kits/dispatch.rs)

实现后沿用相同种子和动作序列对拍状态、回执、历史、候选数组及奖励位模式，检查死亡、复活、召唤和嵌套事件。分别测量前后排完整 Gym 回放及 Rust 内部成本，并记录扫描对象数量，区分减少的工作量与新增维护成本。性能预期以实测结果为准。
