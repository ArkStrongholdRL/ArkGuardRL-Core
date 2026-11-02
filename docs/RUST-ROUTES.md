# Rust 环境与特征内核

Rust/PyO3 提供三个内核：

- `native/src/prep.rs`：PREP 棋子和道具动作枚举，保留候选顺序、容量限制与恢复动作。
- `native/src/observation.rs`：目录级静态射程缓存，以及 4784 维动态观测编码。
- `native/src/route.rs`：公开棋盘上的路线特征与候选关系编码，供研究仓库复用。

`env_actions.py` 整理库存与地形，`env_observation.py` 复用目录编码器。
Python 保留商店/选卡枚举和历史重试过滤；Node 执行游戏状态更新与战斗。
策略网络和训练不属于环境包。

输入转成 Rust 自有数值后，相关批量计算释放 GIL。特征输出为独立 NumPy 数组，
观测输出为不可变数值元组，动作输出为 Python 字典列表。
保持输入布局、候选顺序、求和顺序和浮点阈值，避免清理代码改变既有策略的输入语义。

```sh
python -m pip install -e '.[gym]'
```

安装时构建 release 扩展。修改 Rust 后重新安装并重启进程。
加载时检查扩展接口版本，不扫描源码或核对二进制指纹。
