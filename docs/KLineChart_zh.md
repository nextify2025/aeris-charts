# KLineChart 移植说明（中文）

这个分支把 [KLineChart](https://github.com/klinecharts/KLineChart) v10.0.3 的指标和画线工具
移植进 Aeris Charts，并把 Aeris 的 GPUI 渲染器换到 gpui-kit 使用的 GPUI 版本上，最后用一个
GPUI Kit 的股票详情页把它们串起来。本文说明每一部分在哪里、怎么验证、和 KLineChart 有哪些
差异。架构层面的约定以 `docs/Architecture.md` 为准。

## 1. 指标：27 个，逐位一致

位置：`crates/aeris_charts_indicators/src/klinechart/`，一个文件对应 KLineChart
`src/extension/indicator` 下的一个文件。

MA、EMA、SMA、BBI、VOL、MACD、BOLL、KDJ、RSI、BIAS、BRAR、CCI、CR、DMA、DMI、EMV、MTM、
OBV、PVT、PSY、ROC、SAR、TRIX、VR、WR、AO、AVP。

这些公式沿用国内和香港行情软件的约定，和 Aeris 原有的同名指标不同，所以放在单独的模块里：

- MACD 柱是 `(DIF - DEA) * 2`，EMA 用前 N 个值的简单平均做种子；
- `SMA` 是加权平滑 `SMA(X, N, M)`，普通滚动均值是 `MA`；
- KDJ 的 K、D 从 50 开始递推。

**验证方式**：`tools/klinechart_parity/generate.ts` 直接运行 KLineChart 原版的 `calc` 函数
（不改一行），在三组数据（带涨跌平和零成交量段的随机游走、很短的序列、常数序列）上用默认参数
和自定义参数生成期望值，存成 `crates/aeris_charts_indicators/tests/fixtures/klinechart_parity.json`。
Rust 版按和 KLineChart 相同的运算顺序实现，`tests/klinechart_parity.rs` 要求全部 47,792 个数值
以及所有空值行**逐位相等**（不是近似相等）。重新生成的方法写在 `generate.ts` 文件头。

`indicator.rs` 里的 `klinechart::Indicator` 是可绑定的形式：模板名 + `calcParams`，还带着
输出名、图形类型（线/柱/点）、默认位置（主图或副图）、数值格式、是否需要成交量，以及每个输出
从第几行开始有值。它可以序列化成 `{"indicator": "macd", "short": 12, "long": 26, "signal": 9}`。

## 2. 在图表里使用指标

```rust
use aeris_charts_engine::{klinechart::Indicator, ChartEngine, SeriesKind};

let mut chart = ChartEngine::new(1200.0, 800.0, 2.0);
// 系列 0 是 K 线；成交量放在一个隐藏的序列里，供 VOL/OBV/PVT/EMV/VR 读取。
// 两者都用 set_series_data 灌入数据（时间戳一致），之后用 update_series_bar 推送最新一根。
let volume = chart.add_series(SeriesKind::Histogram);
chart.set_series_visible(volume, false);

let macd = Indicator::from_name("MACD").unwrap();
let outputs = chart.add_klinechart_indicator(0, macd, None);
let vol = chart.add_klinechart_indicator(0, Indicator::from_name("VOL").unwrap(), Some(volume));
// 删除任意一个输出序列，会连同整个指标和它的窗格一起移除。
chart.remove_series(outputs[0]);
```

引擎负责的部分（`crates/aeris_charts_engine/src/klinechart_indicators.rs`）：

- 增量更新：新增、修改最后一根、历史修正、插入、删除都会重算，并且只写回变化的那一段；
- 呈现：KLineChart 的五色线（`#FF9600`、`#935EBD`、`#1677FF`、`#E11D74`、`#01C5C4`，1px），
  VOL/MACD/AO 画成柱，SAR 画成点，逐行着色；主图指标叠在 K 线上，其余各占一个副图；
- 涨跌色跟随图表：柱和点的颜色取 K 线序列的涨跌色，没有设置时取 `layout.bullishColor` /
  `bearishColor`；切换红涨绿跌后调用 `refresh_klinechart_colors()` 重新着色；
- 持久化和 wasm 接口也支持这种指标，JSON 形式是
  `{"kind": "klinechart", "indicator": "macd", "short": 12, "long": 26, "signal": 9}`。

`crates/aeris_charts_native/examples/klinechart_gallery.rs` 把全部指标渲染成 PNG，方便肉眼比对。

## 3. 画线工具

KLineChart 内置 15 个 overlay。其中 4 个 Aeris 原本就有（线段 = `TrendLine`、水平直线 =
`HorizontalLine`、水平射线 = `HorizontalRay`、垂直直线 = `VerticalLine`），另外 11 个是这次新增的：

| KLineChart | Aeris `DrawingKind` | 锚点 |
| --- | --- | --- |
| `straightLine` 直线 | `StraightLine` | 2 |
| `rayLine` 射线 | `RayLine` | 2 |
| `horizontalSegment` 水平线段 | `HorizontalSegment` | 2（同一价格） |
| `verticalRayLine` 垂直射线 | `VerticalRay` | 2（同一根 K 线） |
| `verticalSegment` 垂直线段 | `VerticalSegment` | 2（同一根 K 线） |
| `priceLine` 价格线 | `PriceLine` | 1 |
| `parallelStraightLine` 平行直线 | `ParallelLine` | 3 |
| `priceChannelLine` 价格通道线 | `PriceChannel` | 3 |
| `fibonacciLine` 斐波那契回撤 | `FibonacciLine` | 2 |
| `simpleAnnotation` 注解 | `SimpleAnnotation` | 1 |
| `simpleTag` 标签 | `SimpleTag` | 1 |

它们都登记在引擎的工具目录里（`drawings/tools.rs`），几何在 `drawings/geometry.rs`，所以
放置、拖动、命中测试、选中、持久化和四个渲染后端都自动支持。"同一价格/同一根 K 线"的约束由
锚点联动（`DrawingAnchorLink`）保证，放置、拖动和程序设置锚点时都成立。
`crates/aeris_charts_native/examples/klinechart_drawings.rs` 会把每种工具画成一张缩略图。

## 4. GPUI 版本

`aeris_charts_render_gpui` 原来依赖 Zed 仓库里某个固定提交的 GPUI，现在改为依赖
`gpui-pre` 0.3.6，也就是 gpui-kit 0.6.6 / gpui-component 使用的那个 GPUI 快照。渲染器源码
不用改就能编译通过，图表因此可以直接放进 gpui-kit 应用，和组件库共用同一套 `gpui` 类型。

## 5. 示例应用

`apps/stock-detail`：GPUI Kit 写的股票详情页，包括报价头、周期切换、主图/副图指标、分组画线
工具栏、十字光标图例、深浅主题和红涨绿跌，数据可以用模拟行情，也可以接长桥 OpenAPI。
运行方式和代码导读见 `apps/stock-detail/README.md`。

## 6. 和 KLineChart 的差异

- KLineChart 把上涨的 MACD/AO 柱画成空心；Aeris 的柱状图没有描边样式，这里用更浅的透明度
  表示"空心"。
- KLineChart 每次更新都整段重算指标；这里的运行时也从第一行重算（保证结果一致），但只把
  变化的那一段写回图表。
- 周期、时区和交易时段由应用负责：引擎只认时间戳，示例应用把交易所的本地时间当作 UTC 存储，
  这样坐标轴直接显示交易所时间。

## 7. 验证

```sh
cargo test --workspace                                              # 全部测试（含逐位对拍）
cargo test -p aeris_charts_render_gpui --features gpui-backend      # GPUI 渲染器测试
cargo run -p aeris_charts_native --example klinechart_gallery      # 指标截图
cargo run -p aeris_charts_native --example klinechart_drawings     # 画线截图
cd apps/stock-detail && cargo test --release --features longbridge  # 应用自身的测试
```

## 许可

仓库以 AGPL-3.0 发布。移植自 KLineChart 的部分保留其 Apache-2.0 版权声明，详见根目录 `NOTICE`
和各模块的文档注释。
