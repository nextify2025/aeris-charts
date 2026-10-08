# 指标 API

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

- [空白数据与输出起点](#空白数据与输出起点)
- [成交量分布](#成交量分布)
- [指标约定](#指标约定)
- [广度层指标](#广度层指标)
  - [广度层补全](#广度层补全)
- [KLineChart 指标](#klinechart-指标)

## 空白数据与输出起点

以下规则对所有内置、自定义与 KLineChart 研究成立（所有者决定 Q-G 与 Q-H；领域契约见[空白与预热](../features/studies.md#空白与预热)）：

- 没有值的源条目（空白数据，例如交易时段网格上没有成交的一分钟）不是样本。在该时间，指标的 `data()` 条目没有 `value`。递归研究（EMA、RSI、MACD、ATR、OBV、VWAP 等）从上一个有效样本继续；窗口研究（SMA、布林带、CCI、Donchian、Momentum、ROC、Stochastic 等）跨越最近 N 个有效条目，因此缺口之后的第一个条目即有值，等于删去缺口后计算出的值。
- 每个输出的 `data()` 从其第一个有值的时间开始，不包含开头的空白条目；源以空白数据开头时，返回的列表更短。修正若使第一个值提前或改变，整条输出被替换，依赖它的研究随之重建。市场结构、公允价值缺口与订单块的锚定输出例外，覆盖每个源时间。请按 `time` 对齐指标与价格，不要按下标对齐。
- `indicator_info().warmup_bars` 按无缺口的源度量；源以空白数据开头时第一个值会更晚出现。
- Klinger 把价格有效但成交量为空的柱按零成交量计入：它不贡献力量，也不重启三条 EMA，与 OBV 及其他成交量研究对缺失成交量的处理一致。
- Rust 的纯计算函数（`aeris_charts_indicators::sma` 等）在含 NaN 或 ±∞ 的输入上遵循同一规则，返回与图表相同的值。只接收成交量列的函数（`relative_volume`、`volume_oscillator`）无法区分空白柱与没有成交量的柱，把非有限的成交量视为空白行，与图表在空白槽位上的值相同。

## 成交量分布

```ts
const volume = chart.add_series("histogram", { visible: false });
volume.set_data(volumeBars); // { time, value }, actual volume in the host's chosen units
const profile = chart.add_volume_profile(candles, volume, {
  rows: 48, value_area_percent: 70, width_percent: 25,
  up_color: "rgba(8,153,129,0.45)", down_color: "rgba(247,82,95,0.45)",
});
const distribution = profile.snapshot(); // rows, total_volume, bar_count, poc, value-area bounds
profile.apply_options({ show_poc: true, show_value_area: true });
// profile.remove(); // idempotent; does not remove either source
```

价格数据源最初必须是 K 线/柱系列，成交量数据源则必须是标量系列，二者均来自同一图表。成交量按精确时间戳匹配；缺失、空白、非有限、零和负的成交量不产生任何贡献。每根有效柱的成交量在其最高价/最低价区间内均匀分布，当收盘价不低于开盘价时归为看涨，否则归为看跌。每一行堆叠绿色的看涨份额和红色的看跌份额，并紧贴窗格右边缘终止。平坦柱只计入一个桶。该 OHLCV 估算并不是每个价位的精确成交量、买卖量、盈亏或订单流。演示用的成交量是合成的。

POC 是成交量最大的桶的中心（并列时取价格最低者），使用一个实心标记。连续的价值区域从 POC 出发向相邻的较大桶扩展，并列时选择较低的桶，直到达到所请求的比例；更浓的行颜色表示成员归属，不绘制边界线。行数限制为 1–512，价值区域限制为 >0–100%，宽度限制为窗格的 >0–50%，每个图表的存活成交量分布至多 16 个。`snapshot().error` 报告无法表示的算术结果；空的/缺失的成交量会产生空行和 null 价位。无效的选项更新会保持先前的选项不变。

成交量分布跟随数据源所在的窗格/比例尺以及当前可见的时间范围。数据源更新和范围变化会重新计算桶；颜色/宽度变化和指针移动则复用这些桶。移除任一数据源都会使该句柄失效。成交量分布是仅限运行时的分布，不是标量输出系列，也不包含在 V1 状态导出中；恢复数据源数据之后需要重新创建其绑定。较早的 `create_volume_profile()` 辅助函数仍然是由调用方提供桶的绘图。

生成的 `wasm-bindgen` 类、仅能通过实现对象访问的方法、遥测、基准测试计数器、演示全局变量、夹具和测试钩子均属内部接口，即使 JavaScript 在运行时能够检查到它们。`drawings_json()` 是内部检查用的结构，不是持久化。

`value_snapshot()` 不执行历史导出。不带参数时，它为每个由引擎拥有的系列解析其自身最新的非空白数据所在的逻辑索引/时间。带整数参数时，它执行精确的合并逻辑索引查找；每个存活系列都保留在数组中，缺失值或空白值的数据为 null，而不会借用相邻值。OHLC 系列填充 `open`、`high`、`low` 和 `close`；标量系列填充 `value`；`previous_value` 是同一系列中此前一个非空白数据的收盘价/值。对应的格式化字段使用该系列当前的格式化器。宿主应用仍需负责品种/交易所元数据、VWAP 绑定之外的成交量系列关联、柱/日变化、交易时段日历、可见性设置以及图例 DOM。

由引擎拥有的高级功能系列通过 `value` 暴露其文档中说明的标量价格投影，保持旧版标量 `series_data` 的形状。实验性自定义系列的值由任意宿主回调在渲染期间计算，而不是作为规范的引擎数据存储。因此，它们的快照记录在精确索引查询中为 null，并且在某一帧记录下值之前也为 null；最新值模式只暴露最近一次记录的可见帧的值，并且在该系列未被渲染期间可能保持陈旧。

位于 `packages/charts/api/public-api-v1.json` 的声明清单记录了每个受支持的声明文件。CI 运行 `bun run check:api`；经过审慎评审之后，使用 `bun run update:api` 更新它。

网格线由引擎拥有，默认可见且为虚线。演示宿主可以关闭网格可见性而不替换规范的网格样式/颜色；这种展示选择不是库的默认值。

`chart.reset_style_to_defaults()` 是宿主将展示恢复为随产品发布的 Aeris 默认值的规范操作。它不会根据 `options()` 的输出重建默认值：引擎会恢复语义上的跟随状态，例如未固定的系列颜色和价格比例尺文字。水印内容与可见性、比例尺模式/范围/边距/布局约束、视口缩放/滚动，以及指标/数据语义在重置后保持不变；只有它们由引擎拥有的视觉样式会被恢复。

默认的鼠标滚轮缩放遵循对 TradingView 的测量结果：一次饱和的垂直滚动步长恰好使柱间距变化 10%（较小的触控板增量保持成比例），并保持右边缘固定，因为 `right_bar_stays_on_scroll` 默认为 `true`：在历史数据被压缩或展开时，最新一根柱之后的间隙保持不变。Ctrl/Cmd + 滚轮、macOS 触控板捏合（以 Ctrl + 滚轮的形式送达）以及触摸捏合则改为围绕指针缩放。设置 `right_bar_stays_on_scroll: false` 会恢复以光标为锚点的普通缩放。垂直和水平增量分别独立地缩放和平移窗格、时间轴或价格轴上的时间比例尺；Shift 不改变路由。`wheel_behavior: "pan"` 和 `"zoom"` 是明确的 Aeris 扩展；显式缩放模式仍保留价格轴上的滚轮缩放。

内置的系列实时价格线由引擎拥有。`price_line_extent` 默认为 `"partial"`（从被跟踪的柱/值延伸到窗格右边缘）；`"full"` 保留传统的贯穿整个窗格的线。两种延伸范围使用相同的 `price_line_source`、颜色、宽度和线型选项，包括实线、点线和虚线模式。指标输出继承相同的默认值，因为它们是普通的引擎系列。`create_partial_price_line()` 仅作为这些规范系列选项之上的兼容性控制器保留，不是单独的图元或渲染实现。

图表界面不会注入产品署名或品牌标识。`layout` 只包含图表的视觉配置；公共图表契约中没有署名标志选项。

`create_chart(container, { initialPane: { horizontal_domain } })` 会构建具有连续、时间、类别或极坐标语义的规范首个窗格。通用优先的图表不包含隐藏的金融系列，也无需先添加再移除的清理。省略 `initialPane` 则保留兼容的金融时间窗格和主 K 线系列。无效的创建选项会在拒绝之前移除该次尝试所安装的每一个画布。

通用坐标轴和系列句柄暴露 `set_visible(boolean)` 和 `setVisible(boolean)`。可见性变化会保留句柄及其数据，同时引擎会更新定义域、交互快照、图例、持久化以及下一次渲染的帧。

数值和时间类通用坐标轴句柄暴露 `pan`、`zoom` 和 `reset_view`/`resetView`。时间锚点使用 JavaScript 安全的整数 epoch 毫秒，时间刻度由引擎按有界的 UTC 间隔选取并格式化。

类别通用坐标轴句柄使用相同的视图生命周期。`zoom` 以可见的类别标识为锚点，`pan` 按可见类别窗口的一个比例进行平移，`reset_view` 恢复已配置的或自动的类别注册表。自动注册表发生变化时，会钳制保留的索引窗口，而不是保留过期的类别字符串。

可执行的笛卡尔通用坐标轴接受有界的、类型化的显式刻度，并可附带可选的可移植标签。显式的数值、时间和类别值在 Canvas2D、WebGPU、GPUI、原生帧、持久化和截图中驱动相同的标签与网格坐标；省略标签时使用内置格式化器。当前视图之外的显式刻度会被裁剪，同时指定 `ticks` 与 `tick_count` 会被拒绝。

通用坐标轴的 `grid_visible` 会把刻度线投影到被裁剪的窗格底层，并遵循图表范围的水平或垂直网格设置。数值型 `zero_line` 在零可见时绘制一条独立的实线；多个坐标轴之间共享的像素坐标会被去重。

相同的句柄暴露原子的 `apply_options` / `applyOptions` 方法，用于可变的坐标轴配置、系列呈现，以及兼容的通用系列窗格/坐标轴重新绑定。无效的颜色、刻度策略、分组、堆叠或绑定会使整个先前对象保持不变。通用系列暴露 `general_series_order(pane?)` 和要求传入精确排列的 `set_general_series_order(...)`；这一引擎顺序由渲染、图例、命中测试、React 带 key 的数组和持久化共享。React 的 `GeneralPane` 把这些普通的属性与顺序变化应用到已有句柄上，并在初始安装或就绪回调失败时回滚新获取的窗格或系列。移除自有的空的最后一个窗格会使其稳定标识退役，并留下一个全新的默认布局槽位，因此 React 清理过程不会创建临时窗格。

## 指标约定

内置的指标研究默认采用 TradingView/TA-Lib 的定义。`add_ema`、`add_dema`、`add_tema`、`add_rsi`、`add_rsi_with_source`、`add_macd`、`add_bollinger` 和 `add_bollinger_with_source` 的可选最后一个 `parameters` 参数用于选择另一种广泛使用的约定：

| 参数 | 默认（`convention: "tradingview"`） | `convention: "china"`（通达信/同花顺公式语言） |
| --- | --- | --- |
| `seed`（EMA、DEMA、TEMA、MACD、RSI） | `"sma"`：前 N 个样本的均值；EMA N 从柱 N-1 开始，RSI N 从柱 N 开始，MACD 12/26/9 从柱 25/33 开始 | `"first_value"`：`Y0 = X0`，因此数值从柱 0 开始（RSI 从柱 1 开始） |
| `histogram_multiplier`（MACD） | `1`：`MACD - signal` | `2`：`(DIF - DEA) * 2` |
| `estimator`（Bollinger） | `"population"`（除以 N） | `"sample"`（`STD`，除以 N-1） |
| `seed`（KDJ） | `"fifty"`：需要完整的 RSV 窗口，之后 K 和 D 从教科书上的 50 起步（首个值位于柱 N-1） | `"first_value"`：在不足 N 根柱时，对已有的柱计算 RSV，且 `SMA(X,N,1)` 从其第一个输入开始，因此在柱 0 处 K = D = J = RSV |

```ts
const [dif, dea, bars] = chart.add_macd(candles, 12, 26, 9, undefined, { convention: "china" });
const rsi6 = chart.add_rsi(candles, 6, undefined, { convention: "china" });
const boll = chart.add_bollinger(candles, 20, 2, undefined, { convention: "china", estimator: "population" });
const [k, d, j] = chart.add_kdj(candles, 9, 3, 3);
```

预设仅填充参数；显式字段会覆盖预设，`indicator_info().parameters` 报告展开后的 `seed`/`histogram_multiplier`/`estimator` 值，图表状态持久化存储的是这些显式值，绝不存储预设名称。这些参数出现之前写入的文档会恢复为 TradingView 默认值。Rust 宿主使用 `IndicatorKind::with_convention(IndicatorConvention::China)`。

`add_kdj(source, period = 9, k_smoothing = 3, d_smoothing = 3, options?, parameters?)` 在同一个振荡指标窗格中添加 K、D 和 J：`RSV = (C - LLV(L, N)) / (HHV(H, N) - LLV(L, N)) * 100`，`K = SMA(RSV, M1, 1)`，`D = SMA(K, M2, 1)`，`J = 3K - 2D`，其中 `SMA(X, N, 1)` 为 `Y = (X + (N-1) * Y') / N`。J 不会被裁剪到 0–100。平坦窗口（`HHV == LLV`）会重复上一个 RSV（第一个之前为 50），而不是产生 NaN。`seed` 决定该研究如何起始：

- `"fifty"`（默认）：教科书式 KDJ，通达信自带的帮助称之为“KDJ传统版”：第一个值要等待完整的 N 柱 RSV 窗口（柱 N-1），并以 50 代替缺失的前一个 K 和 D（“若无前一日K值与D值，则可分别用50来代替”；亦见百度百科/东方财富百科）。
- `"first_value"`（`{ convention: "china" }` 会选择它）：公式语言的 KDJ，通达信帮助称之为“KDJ普通版”（`RSV:=...; K:SMA(RSV,M1,1); D:SMA(K,M2,1); J:3*K-2*D`）。当存在的柱少于 N 根时，`HHV`/`LLV` 取现有的柱，因此 RSV 从第一根柱起就存在，且 `SMA(X,N,M)` 从其第一个输入开始（`Y0 = X0`）。K、D 和 J 都等于第一根柱的 RSV，并且每根柱都有值（`warmup_bars` 为 0）。

两者仅在已加载历史数据的起始附近有所不同。经过 `convergence_bars` 之后二者一致（在 9/3/3 下，K 为 26 根柱，D/J 为 44 根柱，两种 seed 相同），因此当需要精确的终端数值时，应在第一根可见柱之前加载相应数量的历史数据。对于已加载全部历史数据的新上市品种，`"first_value"` 从第一个交易时段起即遵循该公式，东方财富和新浪的网页图表即按此方式计算（平坦窗口除外，见下文）。保留裁剪与前置的历史数据会在新的第一根柱处重新开始局部窗口，与该历史数据一开始就已加载时完全一致。`indicator_info().parameters.kdj_seed` 报告该选择，V3 持久化会存储它（没有该字段的文档恢复为 `"fifty"`）。在该规则被验证之前以 `"first_value"` 保存的文档会恢复相同的参数，现在在前 N-1 根柱上也会显示数值。

**约定的核查方式（2026-09-28）**。没有可用的通达信、同花顺、东方财富或富途真实终端，因此这些结果来自已公布的定义、各平台自身的网页图表代码，以及一个平台公布的数值。已公布的数值是在仓库之外进行比较的，未提交到仓库；`crates/aeris_charts_indicators/tests/platform_values.rs` 转而在一条确定性的合成序列上固定了已验证的规则：

| 研究 | 证据 | 结果 |
| --- | --- | --- |
| KDJ `"fifty"` | 通达信帮助“通达信指标公式算法释疑”（help.tdx.com.cn/gspt）描述了采用 50 规则的 KDJ传统版；教科书类百科 | 仅有文档依据 |
| KDJ `"first_value"` | 通达信帮助：KDJ普通版公式；函数参考：`EMA` 在不足 N 根柱时即返回值（与 `EXPMEMA` 不同），`TMA`/`AMA` 从 X 起始。网页图表代码：东方财富（emcharts 3.18.1，quotekchart 1.0.6）在 `min(9, i + 1)` 根柱上计算 RSV，并在柱 0 处令 K = D = J = RSV；新浪财经的公式运行时在现有的柱上计算 `HHV`/`LLV`，并使 `SMA` 从 `Y0 = X0` 起始。数值：雪球服务端计算的两只 A 股自上市日起的 KDJ，以及四只较早上市股票的完整历史数据，均在仓库之外比较 | 雪球的 RSV 自第三个交易时段起与现有柱 RSV 完全一致，同一规则以 5e-5 的精度复现了全部六段历史数据。雪球令 K 和 D 从 100 起始，并将 J 裁剪到 0–100，而任何终端公式都不这样做，因此其最初几个交易时段存在差异；经过 `convergence_bars` 之后二者一致 |
| MACD `{ convention: "china" }` | 同样的仓库外雪球比较；东方财富、同花顺和新浪的网页图表代码 | 自第一个交易时段起完全一致：`EMA` 从第一个收盘价起始，柱 0 处 DIF = DEA = 0，`(DIF-DEA)*2` |
| BOLL `estimator` | 同样的仓库外雪球比较；网页图表代码；通达信帮助：`BOLL` 为 `MA(C,M) ± 2*STD(C,M)`，函数参考将 `STD` 定义为估计的（样本）σ，将 `STDP` 定义为总体 σ | 相互矛盾。雪球、东方财富网页版和同花顺网页版使用总体 σ（`"population"`，与雪球完全一致）。`{ convention: "china" }` 选择通达信 `STD` 定义的样本 σ，新浪的网页图表也是如此，但通达信自带的 BOLL 帮助页以 1/N 说明 σ，且已公布的用户对比结论不一（2008 年的一次测试得出总体 σ，2018 年使用 N-1 的公式则与之吻合）。传入 `estimator: "population"` 可与雪球或东方财富一致；终端的估计量尚需在真实的通达信/同花顺上核查 |
| RSI `"first_value"` | 同样的仓库外雪球比较；东方财富与新浪的网页图表代码 | 起始附近不一致：这些来源令 `SMA` 从 0 起始，雪球还以发行价计算上市首日的涨跌，而 `"first_value"` 从第一次变化起始。差距随历史数据增长而缩小（某只上市品种的 RSI6：柱 39 处为 0.65）。在终端核查之前保持不变 |

未验证：通达信/同花顺/富途终端的确切输出（因此不声称 China 预设与富途一致），终端如何处理平坦窗口（东方财富和新浪的网页图表使用 RSV 0，而不是重复上一个 RSV；通达信的公式帮助未记载除以零的处理），以及富途的网页图表（其页面处于机器人验证之后）。同花顺的旧版网页图表使用其他的 KDJ 起始方式（以 100 起始并裁剪，或对前 N 根柱使用滚动均值），不被视为终端定义。

`indicator_schema(kind)` 为修订 4：约定参数以带有 `options` 列表（按编辑器显示顺序）的 `"choice"` 参数形式出现，非选择参数不带 `options`；开关参数（包络线的 `exponential`）以 `"boolean"` 参数形式出现，VWAP 会列出一个可选的 `amount_source` 系列。修订 4 把此前的 `choices` 字段改名为上游的 `options`（见[兼容性](compatibility.md#已记录的不兼容变更)）；本仓库的修订号与上游不同（上游为 2），宿主不应跨仓库比较修订号。属性面板按 `parameter_type` 分派编辑器时，需要处理 `"integer"`、`"number"`、`"boolean"`、`"source"`、`"series"` 与 `"choice"` 六种取值。

`indicator_schema(kind, period = 14, deviation = 2)` 的名称解析由引擎拥有（`IndicatorKind::schema_definition`）。不传 `period` 与 `deviation` 时，返回每个研究自己的规范默认值：MACD 12/26/9、Stochastic `%D` 3、SuperTrend 倍数 3、EMA 彩带 5/10/20/50/200、VWAP 带 σ 1、Chaikin 3/10、KDJ 9/3/3，以及下文广度层各研究的默认值。签名无法表达“未传参”：`period` 14 与 `deviation` 2 本身即被视为隐式查询，即使显式传入也是如此，因此 `period` 14 时的 MACD、KDJ、Stochastic 与 EMA 彩带，以及 `deviation` 2 时的 SuperTrend 与 VWAP 带，同样返回上述规范默认值。只有其他显式值沿用原来的替换规则（例如 `indicator_schema("macd", 12)` 为 12/24/12，`indicator_schema("kdj", 9)` 为 9/3/3）。具有固定多参数默认值的种类忽略 `period`：KST、TSI、Mass Index、Klinger、KAMA、线性回归、Chaikin 振荡器、Coppock 曲线、终极振荡器与成交量振荡器始终返回各自的默认值（线性回归同样忽略 `deviation`）。带种子的种类报告 `seed: "sma"`，MACD 报告 `histogram_multiplier: 1`，布林带报告 `estimator: "population"`；`klinechart_*` 报告对应模板的默认参数；未知名称抛出 `invalid_options`。此前隐式查询会把 14 与 2 代入每个参数，变更记录见[兼容性](compatibility.md)。

**空白数据源**。指标源中的空白数据行（`{ time }`）保留其时间槽位，但绝不进入计算状态。每个研究都在该处输出一条空白数据输出行，并完全按该行不存在的方式继续计算：暂停的交易时段或预先填充的未来槽位不会重置或污染 EMA/RSI/MACD 递推，窗口类研究使用最近 N 根真实柱。之后填充某个空白槽位时，会从该行起重新计算。

**预热查询与历史数据加载**。`indicator_info()` 为每个输出报告 `warmup_bars`（第一个值之前根价格源的柱数）和 `convergence_bars`（经过这么多柱的历史数据后，数值不再取决于已加载历史数据从何处开始：窗口类研究为预热柱数，再加上使每个递归种子的权重降到 0.1% 以下所需的柱数）。两者都包含链式源，因此 RSI 之上的 SMA 报告的是二者之和。当任何柱数都不足以满足时，`convergence_bars` 为 `null`：交易时段 VWAP 和枢轴点取决于时间锚点，OBV、累积/派发、价量趋势、Parabolic SAR、SuperTrend、ZigZag、Klinger 和 McGinley Dynamic 则取决于整条路径；七个结构与时段研究同样为 `null`（数值取决于最近确认的枢轴，或所在的时段与周期），其 `warmup_bars` 对摆动点为 `left + right`，对其他研究为 0。

若要从第一根可见柱起就显示已收敛的值，请求其之前的历史数据：

```ts
const studies = [...chart.add_macd(candles, 12, 26, 9), chart.add_rsi(candles, 14)];
const needed = Math.max(...studies.map((series) => series.indicator_info()?.convergence_bars ?? 0));
// Host-owned market data: fetch `needed` extra bars before the first bar the user should see.
const history = await provider.bars({ to: first_visible_time, count: visible_bars + needed });
candles.set_data(history); // indicators rebuild from the new first bar
chart.time_scale().set_visible_logical_range({ from: needed, to: needed + visible_bars - 1 });
```

在默认 seed 下，MACD 12/26/9 报告预热为 33、收敛为 154；RSI 14 报告 14 和 108。系列保留（`max_points`）也必须至少保留这么多柱，因为裁剪会在新的第一根柱处重新为递归研究设置种子。

**均价（分时均价）**。`add_vwap(price, volume, options, { amount_source: turnover })` 按每个 VWAP 重置周期报告 `sum(amount) / sum(volume)`，而不是对典型价格加权。成交量与成交额按时间戳与价格行对齐；没有正成交量或有限成交额的分钟，以及空白数据价格行，均不贡献任何内容，且在该周期的第一笔成交之前线条为空白。`indicator_info().amount_source` 标识成交额系列；移除它会移除该研究。重置周期遵循 VWAP 的交易时段键。

**线条在重置处重新开始**。每条 VWAP 线（典型价格或按成交额加权）、每个 VWAP 带输出以及每个枢轴水平位，都在其周期重置处结束线条：新交易时段（VWAP 带则为新的一周或一月）中第一条被绘制的行开始新的一段连续线，没有任何线段、填充或命中区域将其与上一周期相连。周期遵循图表的交易所交易日（`time_zone`、`session_start`），与数值重置的方式完全一致，无论是完整安装之后还是实时更新之后均如此。某个周期中唯一被绘制的行就是其第一行时，会绘制一条一根柱宽的水平线段，因此日线柱上的交易时段 VWAP 每根柱显示一条短线段。普通的折线、面积和基线系列会跨日连接，除非 `break_on_trading_day: true` 要求它们在每个交易所交易日处断开（在 Renko 或 Tick 柱等非时间柱轴上，取每根柱开盘时间所在的日）；空白数据行绝不会使线断开。

## 广度层指标

广度层共 29 个内置研究。以下 19 个方法（上游 `2f62875`）在源系列上添加内置研究，返回值与其他内置研究一样是普通的 `series_api` 输出句柄；`options` 应用于每个输出。参数无效、或成交量研究缺少独立的标量成交量系列时抛出 `invalid_options`。所有周期必须为正整数。

| 方法 | 参数（默认值） | 输出与窗格 |
| --- | --- | --- |
| `add_aroon(source, period)` | | `Aroon Up`、`Aroon Down`，振荡器窗格 |
| `add_awesome_oscillator(source)` | 固定 5/34 | `AO`，动量柱状图 |
| `add_dpo(source, period)` | | `DPO` |
| `add_chande_momentum(source, period)` | | `CMO` |
| `add_bollinger_metrics(source, period, deviation)` | `deviation` 有限且不小于 0 | `%B` 与 `BandWidth`，各占一个振荡器窗格 |
| `add_envelopes(source, period, percent, exponential = false)` | `percent` 有限且不小于 0；`exponential` 选择 EMA 基线 | `Upper`、`Basis`、`Lower`，价格窗格，上下轨之间带状填充 |
| `add_alma(source, period, offset = 0.85, sigma = 6)` | `offset` 在 0–1，`sigma` 在 0.01–1,000,000 | `ALMA`，价格窗格 |
| `add_accumulation_distribution(source, volume_source)` | | `A/D` |
| `add_price_volume_trend(source, volume_source)` | | `PVT` |
| `add_chaikin_oscillator(source, fast, slow, volume_source)` | `fast < slow` | `Chaikin Oscillator` |
| `add_relative_volume(source, period, volume_source)` | | `Relative Volume`；基线为零时为空缺 |
| `add_volume_oscillator(source, fast, slow, signal, volume_source)` | `fast < slow` | `PVO`、`Signal`、`Histogram`（动量柱状图），同一窗格 |
| `add_elder_force(source, period, volume_source)` | | `Elder Force` |
| `add_ease_of_movement(source, period, volume_source, divisor = 100_000_000)` | `divisor` 有限且为正 | `EOM`；窗口含零成交量柱时为空缺 |
| `add_historical_volatility(source, period, annualization = 252)` | `period` 至少为 2；`annualization` 为每年的柱数 | `HV`（百分比） |
| `add_trix(source, period, signal = 9)` | | `TRIX`、`Signal` |
| `add_coppock_curve(source, long_period = 14, short_period = 11, smoothing = 10)` | | `Coppock Curve` |
| `add_fisher_transform(source, period = 10)` | | `Fisher`、`Trigger`（上一 Fisher 值） |
| `add_ultimate_oscillator(source, short_period = 7, medium_period = 14, long_period = 28)` | | `Ultimate Oscillator` |

`indicator_kind` 相应新增 `aroon`、`awesome_oscillator`、`dpo`、`chande_momentum`、`bollinger_metrics`、`envelopes`、`alma`、`accumulation_distribution`、`price_volume_trend`、`chaikin_oscillator`、`relative_volume`、`volume_oscillator`、`elder_force`、`ease_of_movement`、`historical_volatility`、`trix`、`coppock_curve`、`fisher_transform` 与 `ultimate_oscillator`。`indicator_info().parameters` 新增 `exponential`、`offset`、`sigma`、`divisor`、`annualization`、`long_period`、`short_period` 与 `smoothing`（对不使用它们的种类为 `null`）；`deviation` 对布林带指标报告偏差，对包络线报告百分比，对成交量振荡器与 TRIX 报告信号周期。

七个成交量研究（以及下文的 Klinger）把成交量序列中缺失的源时间戳视为零成交量。与其他研究一样，空白数据源行不进入计算，其输出为空白数据行。这些研究的 EMA 一律以前 N 个样本的均值起始，`{ convention: "china" }` 不适用于它们；布林带指标只使用总体标准差。

AO、PVT、TRIX 与 EMV 同时存在内置研究和同名的 KLineChart 模板（`klinechart_ao`、`klinechart_pvt`、`klinechart_trix`、`klinechart_emv`），二者公式不同：内置 PVT 把缺失成交量记为 0 并截断负成交量，模板沿用 KLineChart 的缺失成交量 1；内置 TRIX 的 `period` 必填、`signal` 默认 9，信号线为 EMA，模板默认 12/9，`MATRIX` 为简单移动平均；内置 EOM 在零成交量处为空缺且只输出平滑值，模板在零成交量或零区间处记 0 并另外输出 EMV；AO 的核心公式相同，但模板可配置周期并沿用 KLineChart 的柱体呈现。引擎图例中 KLineChart 绑定的行标题带 `KLineChart` 前缀（例如 `KLineChart PVT` 与内置的 `PVT`）。各输出系列的 `title` 则沿用模板自身的输出名，以保持与 KLineChart 的显示一致，因此内置 PVT 与模板 PVT 的系列标题（名称徽标）都是 `PVT`；宿主需要区分时应读取 `indicator_info().kind`（`price_volume_trend` 与 `klinechart_pvt`）。详见[指标计算与绑定](../architecture/data/indicators.md#广度层指标)。

### 广度层补全

上游 `9dd8cff` 新增以下 10 个方法，约定与上表相同。所有周期为 1 到 1,000,000 的整数；参数无效时抛出 `invalid_options`，且不创建任何输出或窗格。

| 方法 | 参数（默认值） | 输出与窗格 |
| --- | --- | --- |
| `add_kst(source, roc = [10, 15, 20, 30], smoothing = [10, 10, 10, 15], signal = 9)` | `roc` 与 `smoothing` 为四元组（`kst_periods`） | `KST`、`Signal`，振荡器窗格 |
| `add_tsi(source, long = 25, short = 13, signal = 13)` | | `TSI`（−100 到 100）、`Signal`，振荡器窗格 |
| `add_mass_index(source, ema_period = 9, sum_period = 25)` | | `Mass Index`，振荡器窗格 |
| `add_vortex(source, period = 14)` | | `VI+`、`VI-`，振荡器窗格 |
| `add_klinger(source, fast = 34, slow = 55, signal = 13, volume_source)` | `fast < slow`；成交量系列必填，位于周期之后（传 `undefined` 使用默认周期） | `Klinger`、`Signal`，振荡器窗格 |
| `add_kama(source, period = 10, fast = 2, slow = 30)` | `fast < slow` | `KAMA`，价格窗格 |
| `add_mcginley(source, period = 14)` | | `McGinley`，价格窗格 |
| `add_linear_regression(source, period = 20, deviation = 2)` | `deviation` 有限且不小于 0 | `Curve`、`Upper`、`Lower`（残差标准差通道），价格窗格 |
| `add_choppiness(source, period = 14)` | `period` 至少为 2 | `Choppiness`（0–100），振荡器窗格 |
| `add_atr_bands(source, period = 14, multiplier = 2)` | `multiplier` 有限且不小于 0 | `Upper`、`Basis`（收盘价）、`Lower`，价格窗格 |

`indicator_kind` 相应新增 `kst`、`tsi`、`mass_index`、`vortex`、`klinger`、`kama`、`mcginley`、`linear_regression`、`choppiness` 与 `atr_bands`。`indicator_info().parameters` 新增 `multiplier`（ATR 带）、`roc` 与 `smoothing_periods`（KST）、`ema_period` 与 `sum_period`（Mass Index），对不使用它们的种类为 `null`；KST、TSI 与 Klinger 的信号周期报告在 `signal`，TSI 的长短周期报告在 `long_period` 与 `short_period`，Klinger 与 KAMA 的快慢周期报告在 `fast` 与 `slow`。

这些研究同样把空白数据行视为不存在。TSI、KAMA、Klinger、McGinley、Mass Index 与 ATR 带从最后一个有效样本继续，与上游相同；KST、线性回归、Choppiness 与 Vortex 的窗口跨越最近 N 个有效行，上游则在缺口仍位于窗口内时让它们保持空白，本仓库不采用（参见[与上游空白规则的对照](../architecture/data/indicators.md#广度层指标)）。Klinger 与 McGinley 的 `convergence_bars` 为 `null`。TSI、Klinger、Mass Index 与 KAMA 以 SMA 起始，McGinley 以第一个收盘价起始；它们没有 `seed` 参数，`{ convention: "china" }` 不适用。Chop Zone 是 Choppiness 输出上的 38.2/61.8 阈值，目前没有绘制阈值区域。线性回归指标是滚动端点序列，与在两个锚点之间做一次拟合的 `regression_trend` 绘图不同；二者都使用残差的总体标准差。公式与空白数据处理详见[广度层补全](../architecture/data/indicators.md#广度层补全)。

## 结构与时段研究

上游 `051a447`..`dc39045` 新增七个研究。它们都在源的价格窗格与比例尺上输出，参数无效、源不是可用的 K 线源或结构研究的源为 as-of 对齐时抛出 `invalid_options`，且不创建任何输出：

| 方法 | 参数（默认值） | 输出 |
| --- | --- | --- |
| `add_swing_points(source, left = 5, right = 5)` | `left`、`right` 为 1 到 50 的整数 | `[摆动高点, 摆动低点]` 两条阶梯水平线 |
| `add_market_structure(source, left = 5, right = 5, break_on = "close")` | `break_on`：`"close"` 或 `"wick"` | 一个空白锚定输出；BOS/CHoCH 由引擎绘制 |
| `add_fair_value_gaps(source, { min_size = 0, mitigation = "touch", mitigation_price = "wick", max_active = 20, show_mitigated = false })` | `mitigation`：`"touch"`、`"half"`、`"full"`；`mitigation_price`：`"wick"`、`"close"`；`max_active` 为 1 到 64 | 一个空白锚定输出；区域由引擎绘制 |
| `add_order_blocks(source, { left = 5, right = 5, break_on = "close", zone = "wick", mitigation, mitigation_price, max_active = 20, show_mitigated = false })` | `zone`：`"wick"` 或 `"body"`；其余同上 | 一个空白锚定输出；区域由引擎绘制 |
| `add_session_levels(source, calendar = "exchange")` | `calendar`：`"exchange"`、`"utc"` 或 `"host"` | `[时段高点, 时段低点]` |
| `add_previous_period_levels(source, period = "day", calendar = "exchange")` | `period`：`"day"`、`"week"`、`"month"` | `[上一周期高点, 低点, 收盘价]` |
| `add_opening_range(source, duration_seconds, calendar = "exchange")` | `duration_seconds` 为正的 32 位整数 | `[开盘区间高点, 低点, 中点]` |

每个方法最后都可以再传一个 `options`（`Partial<series_options>`），应用于每个输出。`indicator_kind` 相应新增这七个 id，`indicator_schema(kind)` 以 `"choice"` 参数报告 `break_on`、`zone`、`mitigation`、`mitigation_price`、`period` 与 `calendar`，摆动窗口的上限为 50，`max_active` 的上限为 64；这些种类不列出 `source` 参数。结构研究只接受 `close` 输入，也不能切换输入。

**注释快照**。`chart.study_annotations(binding)` 返回结构研究按确认行排序的 `{ markers, zones }`：标记带有 `row`、`confirm_row`、`price`、`kind`（`"swing_high"`、`"swing_low"`、`{ bos: { up } }` 或 `{ choch: { up } }`）与 `from_row`（BOS/CHoCH 线段的起点行），区域带有 `start_row`、`confirm_row`、`top`、`bottom`、`bullish`、`end_row`（缓解或退役的行）与 `retired`。`binding` 须为该研究的第一个输出；非结构研究抛出 `unsupported_operation`，失效的句柄抛出 `invalid_handle`。注释只用于显示，没有命中目标，也不持久化。

**日历策略**。`calendar` 默认为 `"exchange"`：按图表的交易所时区与交易时段起点（`time_scale().apply_options({ time_zone, session_start })`）计算交易日，与 VWAP 和枢轴点相同，跨越午夜的夜盘属于下一个交易日及其所在的周与月；修改时区或交易时段起点时引擎自动重建这些研究。交易所时间为 UTC 且交易时段起点为 0 时，它与 `"utc"` 的结果相同。`"utc"` 按 UTC 自然日、周与月计算；`"host"` 使用下述宿主时段。`indicator_schema(kind)` 报告 `calendar` 的默认值 `"exchange"` 与选项 `["exchange", "utc", "host"]`。完整语义见[时段日历](../features/studies.md#时段日历)。

**时段日历**。`chart.set_study_calendar(boundaries)` 以 `resample_boundary` 行（`startTime`、`endTime`、`sessionId`，UTC 秒）原子地替换运行时日历：最多 20,000 行，有序且不相交，违反时抛出 `invalid_options` 并保留原日历；空列表与 `chart.clear_study_calendar()` 都会清空它。`calendar: "host"` 的研究使用这些时段，替换日历会重建它们及其依赖的研究，`"exchange"` 与 `"utc"` 研究不受影响。日历不进入导出的文档。可以用 `resample_boundaries(options)` 从交易所时段窗口生成这些行。

Rust 宿主使用 `ChartEngine::{add_swing_points, add_market_structure, add_fair_value_gaps, add_order_blocks, add_session_levels, add_previous_period_levels, add_opening_range}`（拒绝时返回空列表；日历策略为 `StudyCalendarPolicy::{Exchange, Utc, Host}`，`Default` 为 `Exchange`）、`ChartEngine::study_annotations(binding)`、`set_study_calendar(Vec<ResampleBoundary>)` 与 `clear_study_calendar()`。确认语义、空白数据规则、检查点与绘制顺序见[结构与时段研究](../features/studies.md)。

## 自定义研究

浏览器宿主用 `chart.register_custom_study(definition)` 注册一个同步计算的研究类型：`definition` 给出 `type`（小写字母、数字与 `._-`，最多 64 字节）、`version`、`title`、`parameters`（与 `indicator_schema` 相同的参数描述，`choice` 参数带 `options`）、1 到 5 个 `outputs`（`name`、`plot`：`"line"`/`"histogram"`/`"area"`/`"marker"`、`pane`：`"price"`/`"dedicated"`、可选的 `default_style`）、可选的 `uses_volume`，以及回调 `init(params) → state`、可选的 `update(state, ctx)` 与必需的 `rebuild(state, ctx)`。`ctx` 携带 `from`、`length`、`tail` 与 `time`、`open`、`high`、`low`、`close`、`volume` 六个 `Float64Array`，回调把 `[from, length)` 的结果写入 `ctx.outputs`，`NaN` 表示空白。`chart.add_custom_study(type, source, params?, { input_source?, volume_source?, ...series_options })` 绑定该类型并按输出顺序返回 `series_api`；输出可以作为其他研究的源。`chart.subscribe_custom_study_fault(callback)` 订阅故障事件（`{ binding, message }`），返回取消订阅函数。回调中调用任何图表 API 都抛出 `reentrant_call`；OffscreenCanvas worker 图表的这两个方法抛出 `unsupported`。

Rust 宿主使用 `ChartEngine::register_custom_study(CustomStudyDefinition, CustomStudyFactory)`、`add_custom_study`、`set_custom_study_parameters`、`set_custom_study_source`、`retry_custom_study`、`custom_study_stats` 与 `take_custom_study_faults`（需要轮询）。调度、界限、待定与故障状态、持久化与绘制见[自定义研究](../features/studies.md#自定义研究)。

## KLineChart 指标

`chart.add_klinechart_indicator(source, indicator, volume_source?, options?)` 添加 KLineChart 的 27 个指标模板之一，采用 KLineChart 的公式与呈现方式，并按输出顺序为每个输出返回一个 `series_api`。`indicator` 是一个 `klinechart_indicator`：即 `indicator` 中的模板名称加上该模板的参数，全部显式写出（不设任何默认值，因此缺少字段会被拒绝）：

```ts
const [dif, dea, histogram] = chart.add_klinechart_indicator(candles, { indicator: "macd", short: 12, long: 26, signal: 9 });
const [volume_bars, ma5, ma10] = chart.add_klinechart_indicator(candles, { indicator: "vol", periods: [5, 10] }, volume);
const [average_price] = chart.add_klinechart_indicator(turnover, { indicator: "avp" }, volume);
```

`options`（最后一个参数，位于 `volume_source` 之后，与 `add_vwap` 相同）是应用于每个输出的 `Partial<series_options>`；如需为单个输出设置样式，请通过其返回的句柄。这 27 个名称即 `klinechart_indicator_name` 联合类型，价格叠加层在前：`ma`、`ema`、`sma`、`boll`、`sar`、`bbi`、`avp`、`vol`、`macd`、`kdj`、`rsi`、`bias`、`brar`、`cci`、`dmi`、`cr`、`psy`、`dma`、`trix`、`obv`、`vr`、`wr`、`mtm`、`emv`、`roc`、`pvt`、`ao`。下表每行给出使用 KLineChart 默认值时的参数、输出（`key`，除非另有说明均绘制为线），以及该模板所需的内容：

| `indicator` | 参数（KLineChart 默认值） | 输出 | 需要 |
| --- | --- | --- | --- |
| `ma` | `periods` `[5, 10, 30, 60]` | `ma1`..`ma4`，每个周期一个 | |
| `ema` | `periods` `[6, 12, 20]` | `ema1`..`ema3` | |
| `sma` | `period` 12, `weight` 2 | `sma` | |
| `boll` | `period` 20, `multiplier` 2 | `up`, `mid`, `dn` | |
| `sar` | `start` 2, `step` 2, `max` 20（百分比） | `sar`（圆点） | |
| `bbi` | `periods` `[3, 6, 12, 24]`，恰好四项 | `bbi` | |
| `avp` | 无 | `avp` | 一个保存成交额的标量源系列，以及成交量 |
| `vol` | `periods` `[5, 10, 20]`，至多四项 | `volume`（柱），`ma1`..`ma3` | 成交量 |
| `macd` | `short` 12, `long` 26, `signal` 9 | `dif`, `dea`, `macd`（柱） | |
| `kdj` | `period` 9, `k_smoothing` 3, `d_smoothing` 3 | `k`, `d`, `j` | |
| `rsi` | `periods` `[6, 12, 24]` | `rsi1`..`rsi3` | |
| `bias` | `periods` `[6, 12, 24]` | `bias1`..`bias3` | |
| `brar` | `period` 26 | `br`, `ar` | |
| `cci` | `period` 20 | `cci` | |
| `dmi` | `period` 14, `adxr_period` 6 | `pdi`, `mdi`, `adx`, `adxr` | |
| `cr` | `period` 26, `ma_periods` `[10, 20, 40, 60]`，恰好四项 | `cr`, `ma1`..`ma4` | |
| `psy` | `period` 12, `ma_period` 6 | `psy`, `maPsy` | |
| `dma` | `short` 10, `long` 50, `signal` 10 | `dma`, `ama` | |
| `trix` | `period` 12, `ma_period` 9 | `trix`, `maTrix` | |
| `obv` | `ma_period` 30 | `obv`, `maObv` | 成交量 |
| `vr` | `period` 26, `ma_period` 6 | `vr`, `maVr` | 成交量 |
| `wr` | `periods` `[6, 10, 14]` | `wr1`..`wr3` | |
| `mtm` | `period` 12, `ma_period` 6 | `mtm`, `maMtm` | |
| `emv` | `period` 14 | `emv`, `maEmv` | 成交量 |
| `roc` | `period` 12, `ma_period` 6 | `roc`, `maRoc` | |
| `pvt` | 无 | `pvt` | 成交量 |
| `ao` | `short` 5, `long` 34 | `ao`（柱） | |

周期列表（`periods`）包含一到五项，每项对应一条输出线（`vol`：一到四项，因为成交量柱占用第一个输出）。周期为 1 到 1,000,000 的整数；`sma` 的权重和 `sar` 的系数为正数，`boll` 的倍数不为负。`indicator_schema("klinechart_<name>")` 从引擎报告相同的默认值（列表为 `period_1`、`period_2`……）和输出名称，因此设置编辑器可以直接读取它们，而无需抄录此表。KLineChart 列出了第二个 `emv` 参数 9，其公式从不读取它；它不属于该定义。

**源**。`source` 是公式读取的 OHLC 系列。`avp` 是例外：其源是一个标量系列，保存每根柱的成交值（成交额），通常处于隐藏状态，它将该系列的累计和除以累计成交量。标量系列（折线、面积、基线或直方图）也可以作为其他所有模板的源，并按开盘价 = 最高价 = 最低价 = 收盘价 = 该值来读取，因此对折线系列使用 `macd` 会计算该线的 MACD。标注为“成交量”的模板需要一个标量 `volume_source`（诸如直方图或折线之类的标量系列，绝不能是源本身）；成交量按精确时间戳与源行配对，没有成交量的柱使用 KLineChart 自己的默认值（`pvt` 为 1，其余为 0）。其他所有模板不得提供成交量系列。`add_klinechart_indicator` 没有 `amount_source`：引擎仅为 VWAP 接受成交额系列，因此 KLineChart 绑定改为在 `avp` 的源中携带成交额。

**无效输入**。未知的模板名称（名称区分大小写，且为小写）、缺失、非整数、超出范围或非数值的参数、周期数过多或过少、已不存在的源系列、传给 `avp` 的 OHLC 源，或缺失、多余、等于源或非标量的成交量系列，都会抛出代码为 `invalid_options` 的 `AerisChartsError`，并使图表保持不变。这些情形全部由引擎判定，错误消息会指明所传入的模板名称。不做任何取整：`{ short: 2.5 }` 会被拒绝而不是向下取整，这与其他 `add_*` 方法的周期参数不同。

**呈现**。外观由引擎拥有，与 KLineChart 的绘制方式一致。线输出为 1px 的线，按输出顺序使用 KLineChart 的五色调色板，不带标题徽标、最新值标签或价格线。`vol`、`macd` 和 `ao` 将其柱输出绘制为直方图，`sar` 绘制仅含标记的圆点。柱和圆点由引擎根据源 K 线的涨跌颜色逐行着色：`vol` 柱按 K 线方向着色（平盘时为灰色），`macd` 柱按符号以及是否上升着色，`ao` 柱按是否上升着色，`sar` 圆点按其相对 K 线中点的位置着色。KLineChart 会为上升的 `macd` 或 `ao` 柱描边；Aeris 直方图没有描边样式，因此这些柱改为以较浅的 alpha 填充。价格类模板（`ma`、`ema`、`sma`、`boll`、`sar`、`bbi`、`avp`）绘制在主窗格的 K 线之上，其他所有模板都绘制在主窗格下方各自独立的窗格中。

**溯源、预热与持久化**。每个输出的 `indicator_info()` 都具有 `kind: "klinechart_<name>"`（`indicator_kind` 的成员）、位于 `parameters.klinechart` 中的完整定义、设为第一个周期的 `period`（`avp`、`pvt` 和 `sar` 为 0）、`deviation: null`，以及所绑定的 `volume_source`。输出的 `warmup_bars` 之前的行没有值，也不会由 `data()` 返回；`ema`、`sma`、`macd`、`kdj`、`rsi`、`dmi`、`trix`、`obv`、`pvt`、`avp` 和 `sar` 的 `convergence_bars` 为 `null`，因为它们的值取决于整个已加载的历史数据（递归平滑、累计总和或路径状态）。绑定从检查点状态出发，每次一行地推进每个公式，因此一次实时 Tick 的开销是公式的窗口而不是整个历史数据，并发布发生变化的后缀。空白数据行（缺失的柱，或预先安装的交易时段槽位）不产生值，也绝不进入窗口：每个值都等于在没有该行的图表上计算出的值。V3 图表状态将该定义存储为 `{"kind": "klinechart", "indicator": "macd", "short": 12, "long": 26, "signal": 9 }`，并附带源、成交量源以及每个输出的样式引用，并像其他所有研究一样恢复到全新的图表中。这些公式移植自 KLineChart v10.0.3，其输出与之逐位一致（参见[指标架构](../architecture/data/indicators.md)与 [NOTICE](../../NOTICE)）。
