# stock-detail：GPUI Kit 股票详情页 + 完整 K 线

一个原生桌面的股票详情页示例：界面用 [GPUI Kit](https://github.com/longbridge/gpui-kit)（GPUI +
gpui-component）搭建，K 线由本仓库的 Aeris 图表引擎绘制，指标和画线工具移植自
[KLineChart](https://github.com/klinecharts/KLineChart) v10.0.3。

它的目的是学习：每一层都尽量直白，读完就知道一个专业 K 线图在 GPUI 里是怎么接起来的。

## 运行

macOS（Apple Silicon 或 Intel），需要 Xcode Command Line Tools 和 Rust stable：

```sh
cd apps/stock-detail
cargo run --release
```

第一次构建要编译 GPUI、wgpu 等依赖，需要几分钟；之后增量构建很快。仓库根目录的
`rust-toolchain.toml` 会让 rustup 自动装好 stable 工具链。

Linux 同样可以运行（X11 或 Wayland，需要 Vulkan 驱动）。

### 预设场景

`--scene` 会暂停模拟推送，把页面切到一个固定状态，方便截图或演示：

| 场景 | 内容 |
| --- | --- |
| `day` | 日 K + MA，副图 VOL、MACD，十字光标停在近期一根 K 线上 |
| `intraday` | 分时：价格线、均价线、昨收线，副图 VOL、MACD |
| `drawings` | 日 K + BOLL，副图 VOL、KDJ，以及斐波那契、直线、价格通道、注解、标签等画线 |
| `light` | 浅色主题 + 红涨绿跌：周 K + EMA、SAR，副图 VOL、RSI |

```sh
cargo run --release -- --scene drawings
```

### 接入长桥（Longbridge OpenAPI）实时行情

```sh
export LONGBRIDGE_APP_KEY=...
export LONGBRIDGE_APP_SECRET=...
export LONGBRIDGE_ACCESS_TOKEN=...
cargo run --release --features longbridge -- --symbol AAPL.US
```

`src/longbridge.rs` 用官方 Rust SDK（`longbridge` 5.1）的 blocking 接口实现了同一个
`MarketData` trait：切换周期时拉取历史 K 线（最多 1000 根，前复权），订阅报价和当前周期的
K 线推送，推送在后台线程排队，由页面每秒 `poll` 一次合并进图表。时间按交易所时区换算
（`.US` 美东、`.HK` 香港、`.SH`/`.SZ` 上海、`.SG` 新加坡）。这部分已对照 SDK 源码编译通过，
但还没有用真实账户跑过，第一次接入时请留意日 K 的时间戳和分时的交易时段是否符合预期。

## 功能

- **周期**：分时、1 分、5 分、15 分、1 小时、日 K、周 K、月 K。所有周期都由同一份数据聚合，
  所以互相一致。
- **指标**：KLineChart 的全部 27 个内置指标，数值与 KLineChart 逐位一致（仓库里有对拍测试）。
  主图 MA、EMA、SMA、BOLL、SAR、BBI；副图 VOL、MACD、KDJ、RSI，"更多"里还有 BIAS、BRAR、
  CCI、DMI、CR、PSY、DMA、TRIX、OBV、VR、WR、MTM、EMV、ROC、PVT、AO。分时图自动换成
  AVP 均价线。
- **图例**：每个窗格左上角按 KLineChart 的样式显示指标名、参数和光标所在 K 线的数值。
- **画线工具**（左侧工具栏，分组展开）：线段、直线、射线、价格线；水平直线/射线/线段、
  垂直直线/射线/线段；平行直线、价格通道线；斐波那契回撤；注解、标签、文本；矩形、折线、
  画笔；多头/空头仓位。另有磁吸开关（锚点吸附到 K 线的开高低收）和一键清除。
- **交互**：拖动平移，滚轮缩放，拖动时间轴/价格轴缩放，双击坐标轴复位，拖动窗格分隔线
  调整高度，点击选中画线后可拖动锚点或整体移动。
- **快捷键**（先点一下图表获得焦点）：`+`/`-` 缩放，`Home` 回到最新，`Delete` 删除选中画线，
  `Esc` 取消当前工具，`Enter` 结束折线。Shift 拖动锚点可吸附 0°/45°/90°，按住 Ctrl/Cmd
  临时磁吸。
- **外观**：深色/浅色主题（图表跟随应用主题取色），红涨绿跌/绿涨红跌一键切换
  （K 线、成交量柱、MACD 柱、SAR 点一起变色）。
- **模拟行情**：确定性的随机游走，两年日线 + 五个交易日分钟线，最后一个交易日仍在交易，
  每秒推进 10 秒行情；标题栏可以暂停。

## 代码导读

推荐的阅读顺序：

1. `src/market.rs`：页面消费的数据类型（`Candle`、`Quote`、`Period`）和 `MarketData`
   trait。换成真实数据源只需要实现这四个方法。
2. `src/chart.rs`：**核心**。`ChartView` 把 Aeris 的 `ChartEngine` 放进一个 GPUI view：
   - 数据：`set_series_data` 一次灌入整段 K 线，`update_series_bar` 更新或追加最新一根；
     成交量和成交额是两个隐藏的序列，供 VOL/OBV/AVP 等指标读取。
   - 指标：`add_klinechart_indicator` 绑定指标，引擎负责计算、增量更新、配色和窗格布局；
     删除任意一个输出序列会连同整个指标和它的窗格一起移除。
   - 绘制：GPUI 的 `canvas` 在 prepaint 阶段用 GPUI 自己的字形测量重建布局和帧
     （`recompute_layout_with_measure`、`build_frame_into`），在 paint 阶段交给
     `GpuiChartRenderer` 执行。帧没变化时直接重放上一次的绘制计划。
   - 输入：鼠标/滚轮/键盘事件换算成图表坐标后交给引擎的手势 API（平移、缩放、坐标轴拖动、
     窗格分隔线、画线创建与编辑）。画线工具的放置规则全在引擎里，宿主不需要区分工具种类。
3. `src/page.rs`：gpui-kit 组件搭的页面——标题栏、报价头、周期和指标按钮（`Button`、
   `DropdownMenu`、`PopupMenuItem`）、画线工具栏，以及每秒一次的行情轮询。
4. 引擎侧的 KLineChart 移植（仓库根目录下）：
   - `crates/aeris_charts_indicators/src/klinechart/`：27 个指标的公式，一个文件一个指标；
     `indicator.rs` 是可绑定的 `Indicator`（参数、输出、预热行、默认位置）。
   - `crates/aeris_charts_engine/src/klinechart_indicators.rs`：指标在图表里的呈现
     （线色、柱色规则、窗格）。
   - `crates/aeris_charts_engine/src/drawings/tools.rs` 和 `drawings/geometry.rs`：
     从 KLineChart overlay 移植的画线工具。

```text
MarketData ──bars/quotes──▶ StockPage (gpui-kit) ──▶ ChartView (GPUI view)
                                                        │
                           ChartEngine：数据、指标、画线、布局、交互状态
                                                        │ ChartFrame（有序绘制列表）
                                                        ▼
                                  GpuiChartRenderer ──▶ GPUI 场景（Metal / Vulkan）
```

## 许可

本目录和仓库一样以 AGPL-3.0 发布。指标公式和画线工具的几何移植自 KLineChart
（Copyright (c) 2019 lihu，Apache-2.0），见仓库根目录的 `NOTICE`。GPUI Kit 与
gpui-component 为 Apache-2.0，图标来自 Lucide（ISC）。
