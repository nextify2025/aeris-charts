# 公共 API 与兼容性策略

## 受支持的产品接口面

受支持的产品是 1.0 之前的浏览器包 `@aeristerminal/aeris-charts`。其与框架无关的根 ESM 入口、可选的 `./react` 适配器、`./wasm` 资源和 `./design.css` 样式表是受支持的 npm 导出路径。React 是可选的 peer 依赖，根入口的使用者不会加载它。受支持的根接口面如下：

- 图表的创建与初始化；
- `packages/charts/src/types.ts` 中声明的图表、系列、时间比例尺、窗格、价格比例尺、价格线和绘图句柄；
- 窗格本地的命名价格比例尺，通过 `chart.add_price_scale()`、`chart.price_scales()`、`chart.move_price_scale()`、`chart.remove_price_scale()`、`chart.price_scale()`/`pane.price_scale()` 中的任意字符串 ID，以及系列的比例尺标识与重新绑定来提供；
- 价格轴语义：刻度标签始终落在系列的 `min_move` 网格上；内置的 `price_format` 若指定了 `min_move` 而未指定 `precision`，则自动推导精度（参考实现 `precisionByMinMove`）；`price_format.tick_ladder` 价格区间（交易所价差表，例如 HKEX 表）将每个标签舍入到所属区间的价位刻度并按区间设置精度，使坐标轴刻度保持在可见区间的公共网格上，并驱动该系列所在比例尺上的交易磁吸；系列选项 `autoscale_info_provider`（参考实现 `autoscaleInfoProvider`）替换系列的自动缩放信息（它在渲染期间运行：在其内部调用图表 API 会抛出 `unsupported_operation` 且不改动图表，抛出异常的 provider 在该次渲染 pass 中被忽略）；比例尺选项 `tick_mark_density` 和 `ensure_edge_tick_marks_visible` 遵循参考实现，Aeris 扩展项 `base_value`（与绘图共享的显式百分比/指数化基准）、`autoscale_center`（对称自动缩放）和 `stable_auto_scale`（可选启用的迟滞；默认保持与参考实现完全一致）都是普通的 `price_scale_options`；格式错误的价位梯和扩展值会抛出 `invalid_options`，并使价格格式或比例尺保持不变。指数化到 100 的标签使用参考实现的固定两位小数格式化器；
- [分时图](#分时图)一节所述的分时构建模块：`session_slot_times()`、显式的时间轴 `tick_marks`、`histogram_updown_rule` 的前收盘价成交量着色（配合宿主提供的 `up_color`/`down_color`）、显示其首根有成交的柱的基线系列、新增的 `break_on_trading_day` 线/面积/基线选项（默认 `false`），以及在每次重置时重新开始的 VWAP/枢轴线；收盘时间显示标签 `time_scale_options.bar_time_label` 按柱的收盘时间显示柱，而柱本身仍保持开盘时间戳（参见 [收盘时间标签](#收盘时间标签)）；
- 通过系列选项 `time_alignment: "as_of"` 和 `as_of_max_staleness` 实现的多日历叠加层（参见 [多日历叠加层](#时间交易所时区与交易时段)）；
- 这些句柄所声明的内置系列、指标、绘图类型、选项、主题、数据写入、交互、订阅、截图和生命周期操作；
- 通过规范的 `drawing_kind` 值 `"long_position"` 和 `"short_position"` 提供的多头头寸与空头头寸绘图；每个绘图按入场、目标、止损的顺序存储三个可编辑锚点，绘制目标/入场/止损信息，将三个价格都投影到所属 Y 轴，并使用共享的绘图历史、持久化、命中测试和后端帧路径。统计数据使用两个持久化的绘图选项：`position_account_size`（假设的余额，默认 1,000）和 `position_risk_percent`（在止损处承担风险的占比，0–100，默认 25），与券商订单无关；
- 通过规范的 `drawing_kind` 值 `"price_range"`、`"date_range"` 和 `"date_and_price_range"` 提供的测量绘图（早先的拼写 `"date_price_range"` 在导入、模板、剪贴板和同步时仍会被读取，但绝不会写出）；每个绘图存储一个可编辑的起始锚点和结束锚点，吸附到整根柱和价格刻度，标注带符号的价格变化、百分比、刻度数（按品种刻度或价格区间价位梯计数）、柱数和经过的时间（`labels` 选项决定显示哪些度量项），并以绘图颜色绘制；
- 内置指针处理中的 Shift 点击快速测量：在图表空白区域 Shift + 按下，会启动一次临时的日期与价格测量，该测量跟随指针（上涨使用绘图默认颜色，下跌使用市场下跌颜色），在拖动后松开时或下一次点击时冻结，并由随后的点击或 Escape 取消。它绝不是绘图、历史条目或持久化对象；
- 通过 `chart.add_volume_profile(prices, volume, options)` 提供的可见范围成交量分布，返回带有 `options()`、`apply_options()`、`snapshot()` 和 `remove()` 的分布句柄；
- KLineChart 的 27 个指标模板，通过 `chart.add_klinechart_indicator(source, indicator, volume_source?, options?)` 提供，详见 [KLineChart 指标](#klinechart-指标)；
- 一等的、由 Tick 驱动的足迹图 / Numbers Bars 系列，通过 `chart.add_series("footprint")` 提供，包括对象与类型化列的成交写入、显式、报价和 tick 规则的主动方处理、每价位的 Bid × Ask/总量/delta、控制点（POC）、最终/最大/最小/交易时段 delta、可配置的对角失衡与堆叠失衡、密度细节层级（LOD），以及派生的柱/价位查询；通用 OHLC setter 会被拒绝，因为它们无法提供订单流真值；
- 一个图表级回放时钟，加上共享的规范成交流、类型化批量写入、普通的 K 线/柱绑定、精确的 seek 工作量遥测、`chart.trade_stream_stats()` 中的实时末端依赖工作计数器（`dependent_rows_computed`、`bar_rows_projected`、`bubble_trades_scanned`、`bubble_markers_sized`），以及通过 `configure_synthetic_bar_series`、`set_synthetic_bar_source[_typed]` 和 `update_synthetic_bar_source` 提供的固定/ATR Renko、Line Break、Kagi 和 Point & Figure 变换；合成数据源/历史仍归宿主所有，不属于图表状态持久化的一部分；
- [Tick 转 K 线与重采样](#tick-转-k-线与重采样)一节所述的由 Tick 构建的普通 K 线与 OHLCV 重采样：以交易所交易时段为锚点的成交流时间柱（`chart.set_trade_stream_sessions()`）、成交流成交量直方图（`chart.add_trade_volume_series()`），以及通过 `chart.configure_resampled_series()`、`chart.resampled_bars()`、`chart.resample_stats()` 和由交易时段派生的 `resample_boundaries()` 辅助函数提供的引擎重采样；重采样配置仅在运行时存在；
- 由引擎解析的辅助点击上下文，通过 `chart.subscribe_chart_context()` 提供，包括窗格、时间、逻辑索引、坐标、命中的系列，以及其比例尺上的精确价格；菜单、剪贴板操作和订单操作由宿主拥有；
- 图表范围的引擎值查询，通过 `chart.value_snapshot(logical_index?)` 提供，包括每个存活系列的句柄/ID、当前类型、窗格/比例尺归属位置、由引擎拥有的精确值或各系列独立的最新值、前驱值，以及格式化字段；`mouse_event_params.value_snapshot` 携带相同的记录，并在十字光标离开时恢复最新值，而旧版 `series_data` 仍只包含有值的系列；
- 新增的完整指标谱系元数据：稳定的绑定 ID、结构化参数、数据源与可选的 VWAP 成交量数据源，以及稳定的输出名称/索引/数量，同时保留旧字段；
- [指标约定](#指标约定)一节所述的指标计算约定、KDJ、对空白数据安全的指标数据源、预热查询和成交额加权平均价；
- 五输出 EMA 色带，通过 `chart.add_ema_ribbon()` 提供，默认周期为 `5/10/20/50/200`，默认颜色为 `#335cff/#FF9800/#7d52f4/#fb4ba3/#fb3748`，以及通过 `chart.set_ema_ribbon_periods()` 进行的原子的就地周期修改；
- 自有的、与券商无关的交易状态、即时/手动确认、预览、命中测试、语义样式，以及类型化的意图订阅，均通过 `chart.trading()` 暴露；
- 以宿主为权威的警报线指标，以及十字光标加号徽标创建请求，通过 `chart.alerts()` 暴露；条件/频率作为配置元数据被保留，而对话框、评估、持久化、限额、过期、后台投递和通知由宿主拥有；
- 默认的图表无障碍、其新增的 `chart.accessibility()` 单例句柄、用于兼容的 `enable_accessibility()`、无障碍选项，以及键盘数据/绘图操作；
- 新增的 `wheel_behavior` 图表选项（`auto`、`pan` 或 `zoom`）；现有的手势选项名称保持兼容；
- 时间比例尺视口契约：数据更新（历史前插、乱序插入、缺口回填、保留期修剪）绝不会移动已向后滚动的视图，而实时边缘按 `shift_visible_range_on_new_bar` 跟随新柱；`set_visible_logical_range()` 保留小数边界；`scroll_to_real_time()` 以动画滚动到已配置的 `right_offset`；键盘时间比例尺移动遵循 `handle_scroll`/`handle_scale`；在处理函数同步修改数据之后，可见范围订阅者始终以最终范围收尾；
- 新增的 `lock_visible_logical_range` 时间比例尺选项（默认 `false`），用于固定的整交易时段视图，例如分时图：将每个交易时段槽位安装为空白数据，调用 `set_visible_logical_range({ from: 0, to: slots - 1 })`，该范围从开盘前状态直到收盘，在数据更新和尺寸调整期间始终保持精确；
- `AerisChartsError` 及其机器可读的错误码；
- 通过 `chart.export_state()` 和 `chart.import_state()` 实现的图表状态持久化 V1。
- 面向常见 JavaScript 生命周期的驼峰命名别名（`createChart`、`initWasm`、图表/系列/比例尺创建与数据方法），同时每个现有的蛇形命名入口在相同句柄上仍受支持；
- 通过 `chart.reset_style_to_defaults()` 实现的规范展示重置。它为图表所选主题恢复由 Aeris 拥有的图表与系列视觉默认值，包括语义上的未设置/跟随状态，同时保留数据、窗格、绘图、指标、系列可见性/元数据、价格格式、比例尺绑定以及比例尺/视图状态。它有意与 `chart.reset_view()` 分离，后者会更改时间/价格比例尺的视图状态。
- 通过 `chart.backend_status()` 提供的只读后端诊断，包括请求的后端和活动后端、稳定的回退阶段/原因、安全上下文与 `navigator.gpu` 的暴露情况，以及可选的不稳定平台细节。`chart.backend()` 保留其现有的活动后端返回值。

`./react` 入口导出 `AerisChart`、`FinancialSeries`、`GeneralPane` 和 `useAerisChart`，以及它们的配置类型。它是位于根命令式 API 之上的编写适配器：普通的重新渲染会保留图表/系列的标识，数据变化会修改这些已有句柄，结构性的通用坐标轴或系列变化只替换受影响的引擎对象，卸载则使用规范的销毁路径。它不会独立于 Rust 引擎定义图表语义。导入该模块在 SSR 下是安全的；DOM/WASM 图表创建从已挂载组件的 effect 开始。`FinancialSeries` 以流式方式更新：新的 `data` 数组若与先前已应用的数组相比仅有被替换的最后一个点和/或按升序追加的点（通过标识或浅相等识别，至多 1,024 个变化点），则通过 `series.update()` 应用；其他任何变化均为一次 `setData`。

### 成交量分布

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

### 指标约定

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

`indicator_schema(kind)` 为修订 2：约定参数以带有 `choices` 列表的 `"choice"` 参数形式出现，VWAP 会列出一个可选的 `amount_source` 系列。

**空白数据源**。指标源中的空白数据行（`{ time }`）保留其时间槽位，但绝不进入计算状态。每个研究都在该处输出一条空白数据输出行，并完全按该行不存在的方式继续计算：暂停的交易时段或预先填充的未来槽位不会重置或污染 EMA/RSI/MACD 递推，窗口类研究使用最近 N 根真实柱。之后填充某个空白槽位时，会从该行起重新计算。

**预热查询与历史数据加载**。`indicator_info()` 为每个输出报告 `warmup_bars`（第一个值之前根价格源的柱数）和 `convergence_bars`（经过这么多柱的历史数据后，数值不再取决于已加载历史数据从何处开始：窗口类研究为预热柱数，再加上使每个递归种子的权重降到 0.1% 以下所需的柱数）。两者都包含链式源，因此 RSI 之上的 SMA 报告的是二者之和。当任何柱数都不足以满足时，`convergence_bars` 为 `null`：交易时段 VWAP 和枢轴点取决于时间锚点，OBV、Parabolic SAR、SuperTrend 和 ZigZag 则取决于整条路径。

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

### KLineChart 指标

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

**溯源、预热与持久化**。每个输出的 `indicator_info()` 都具有 `kind: "klinechart_<name>"`（`indicator_kind` 的成员）、位于 `parameters.klinechart` 中的完整定义、设为第一个周期的 `period`（`avp`、`pvt` 和 `sar` 为 0）、`deviation: null`，以及所绑定的 `volume_source`。输出的 `warmup_bars` 之前的行没有值，也不会由 `data()` 返回；`ema`、`sma`、`macd`、`kdj`、`rsi`、`dmi`、`trix`、`obv`、`pvt`、`avp` 和 `sar` 的 `convergence_bars` 为 `null`，因为它们的值取决于整个已加载的历史数据（递归平滑、累计总和或路径状态）。绑定从检查点状态出发，每次一行地推进每个公式，因此一次实时 Tick 的开销是公式的窗口而不是整个历史数据，并发布发生变化的后缀。空白数据行（缺失的柱，或预先安装的交易时段槽位）不产生值，也绝不进入窗口：每个值都等于在没有该行的图表上计算出的值。V3 图表状态将该定义存储为 `{"kind": "klinechart", "indicator": "macd", "short": 12, "long": 26, "signal": 9 }`，并附带源、成交量源以及每个输出的样式引用，并像其他所有研究一样恢复到全新的图表中。这些公式移植自 KLineChart v10.0.3，其输出与之逐位一致（参见 `docs/Architecture.md` 与 `NOTICE`）。

## 坐标与窗格

每个公共坐标都位于同一个图表内容空间中，绝不是窗格局部坐标。`x` 是自绘图区左边缘（左侧价格栏的右侧）起算的 CSS px，因此容器中的 `x` 为 `pane.get_geometry().left + x`。`y` 是自堆叠窗格区域顶部（即窗格 0 的顶部）起算的 CSS px。在该 `y` 中，窗格跨越 `[geometry.top, geometry.top + geometry.height]`；其窗格局部 `y` 为 `y - geometry.top`。同一空间也承载 `series.price_to_coordinate`/`coordinate_to_price`、`chart.price_to_coordinate`/`coordinate_to_price`、`time_scale()` 转换、`mouse_event_params.point`、十字光标、命中测试、绘图以及交易几何，因此某个 API 得到的 `y` 可以直接交给任何其他 API。所有值都反映最近一次布局 pass：在 `pane.set_height`、分隔条拖动或窗格移动之后，请在图表完成布局后再次读取它们。

**选择窗格**。系列句柄在其自身所在的窗格和价格比例尺上进行转换，采用该比例尺的模式和基准。需要较下方窗格坐标的宿主，会持有该窗格中某个系列的句柄：

```ts
// `rsi` is a series handle in pane 1, for example from `chart.add_rsi(candles, 14)`.
const pane = chart.panes()[1].get_geometry();
const y = rsi.price_to_coordinate(70); // in pane 1's [top, top + height] while 70 is in range; never pane-local
const paneLocalY = y! - pane.top;      // pane-relative chrome subtracts the pane top itself
rsi.coordinate_to_price(y!);           // 70
```

`chart.price_to_coordinate(price)` 在窗格 0 的默认比例尺上转换（即第一个可见的非叠加层系列的比例尺，否则为右侧比例尺），`chart.coordinate_to_price(y)` 使用包含 `y` 的窗格的默认比例尺：分隔条归属于其上方的窗格，位于所有窗格下方的 `y` 归属于最后一个窗格。这就是十字光标标签在该窗格中读取的比例尺，因此二者绝不会不一致。两者都不遵循系列创建顺序或某个叠加层的比例尺。时间、逻辑和 `x` 的转换对每个窗格都相同。

**另外两个空间**。插件绘制上下文的转换器（`price_to_y`、`time_to_x`、`logical_to_x`）返回整个图表的位图 px，其中 `x` 包含 `pane_left`，这正是插件画布绘制所在的空间；减去 `pane_left` 并除以 `dpr`，即可与上文的 CSS px 转换器比较。插件坐标轴标签描述符的 `coordinate` 是窗格局部坐标（价格：距窗格顶部的 px；时间：距绘图区左侧的 px），除非系列图元提供了 `price`，此时它会在该系列的比例尺上转换。

**联动十字光标**。`crosshair_sync_position()` 以及 `take_sync_events()` 返回的 `crosshair` 事件携带 `pane_index` 和位于该窗格默认比例尺上的 `price`，窗格由十字光标的 `y` 确定（分隔条算作其上方的窗格）。`set_crosshair_position(price, time, series)` 通过给定系列的比例尺放置该线；当该系列位于窗格的默认比例尺上（并且在百分比和指数化模式下共享其基准）时，发出的价格即原始 `price`，否则会在默认比例尺上重新表达，使联动的图表落在同一条线上。`apply_external_crosshair` 在所请求窗格的默认比例尺上转换价格，并将该线限定在该窗格内：位于窗格可见范围之外的价格会停在窗格边缘，而不会绘制到相邻窗格中。

## 时间、交易所时区与交易时段

规范的图表时间为整数 UTC 秒。`business_day` 值和严格的 `"YYYY-MM-DD"` 字符串按 UTC 零点处理；图表返回的值始终是数值型 UTC 秒。金融时间轴是序数式的：柱按索引等距排列，因此周末、节假日、午休和半日市都不占宽度。宿主拥有交易所日历，并且只发送实际存在的柱；节假日日历不属于引擎功能。默认情况下，每个系列的时间戳都会加入时间轴，因此来自另一个市场日历的叠加层会把它自己的柱作为槽位加入；请改为让它选用 `time_alignment: "as_of"`（参见 **多日历叠加层**）。

**多日历叠加层**。`series_options.time_alignment` 为 `"union"`（默认值，参考实现的行为）或 `"as_of"`。as-of 系列（例如叠加在另一交易所股票之上的指数、叠加在股票之上的加密货币、叠加在其标的之上的期货夜盘）不会增加时间点，因此其他系列的时间轴保持无间断。每个时间点——直到其他系列的最后一根真实柱为止——显示该叠加层在不晚于该时间点的时间处的最后一行：位于两个时间点之间的行会合并到后一个时间点，没有更新行的时间点重复上一行，最后一个时间点之后的行则等待其他系列到达它们（预先安装的日内交易时段绝不会在其未来槽位中显示叠加层）。`as_of_max_staleness`（整数秒，默认 `null`）会让时间点留空，而不是重复比该值更旧的行；`0` 只显示恰好位于某个时间点上的行。时间点来自 union 系列，因此图表上单独存在的 as-of 系列不显示任何内容。所有读取该系列的功能都遵循同样的时间点：渲染、命中测试、十字光标和图例的值、`value_snapshot`（报告该时间点的时间）、比较锚点和百分比基准、自动缩放、标记（放置在不早于其时间的第一个时间点上；比所有时间点都新的标记随其行一同等待，被陈旧度上限留空的时间点会隐藏它）、交易时段高亮（每个时间点按其所显示的行着色）、无障碍焦点环（位于显示该焦点行的第一个时间点上），以及最新值界面元素。`data()`、`data_by_index` 和 `last_value_data` 返回叠加层自身的行及其自身的时间。绑定到 as-of 系列的研究在其自身的行上计算，并以同样方式显示；它们的 `time_alignment` 读回的是源的值，且无法设置。任一侧的实时 Tick 仍与其所改变的时间点成比例。仅拥有自身行数据的折线、面积、基线、直方图、柱和 K 线系列接受它：自定义、高级、足迹图、成交绑定、成交研究和合成系列，以及非时间（Tick、成交量、区间或合成）柱轴上的每个系列，都会抛出 `unsupported_operation`。把 as-of 系列转换为自定义、高级或足迹图系列、将其绑定到成交流，或将其配置为合成变换，会使它回到 union。错误的值，或在没有 `"as_of"` 时给出陈旧度上限，会在该调用的其他选项生效之前抛出 `invalid_options`；`add_series` 在创建（或接管）该系列之前检查这两个键，被拒绝时不会在主线程或 worker 中留下任何系列。重复应用当前的对齐方式是空操作，不会通知任何 `subscribe_data_changed` 处理器；发生变化时会以 `"full"` 通知每个处理器一次。worker 图表在 `add_series` 选项中接受这两个键，之后可通过 `offscreen_chart.apply_series_options(patch, series_id)` 更改它们；`offscreen_chart.series_options(series_id)` 读回这些选项。worker 图表通过 `add_series` 返回的数值 id 来定位系列（`0` 为主系列）。该补丁遵循与 `apply_options` 相同的规则并抛出相同的错误码（省略的键保持其值，切换到 `"union"` 会清除陈旧度上限，未变化的请求是空操作），抛出错误时不应用任何内容。它只接受 `time_alignment` 和 `as_of_max_staleness`：任何其他带值的键都会抛出点名该键的 `unsupported_operation`，非对象补丁则抛出 `invalid_options`。不是 `0..=4294967295` 范围内整数的 id，或指向没有存活系列的 id，会抛出 `invalid_handle`（对于已被移除的系列则抛出 `stale_handle`），即使补丁为空也是如此；已被移除的图表则抛出 `disposed`。调用成功时，会在返回之前重绘 worker 画布，与其他 worker 变更一致（未变化的请求也是如此）；抛出错误的调用不会绘制任何内容。worker 图表没有系列句柄，因此没有 `subscribe_data_changed` 通知；请在调用之后读取 `visible_logical_range()`。Rust 宿主调用 `ChartEngine::set_series_time_alignment(id, TimeAlignment::AsOf { max_staleness })`。与其他金融系列选项一样，该设置由宿主拥有，且不被持久化。

**交易所时区**。`chart.time_scale().apply_options({ time_zone, session_start })`，或声明式图表选项 `timeScale: { timeZone, sessionStart }`（worker 图表同样接受），用于设定时刻如何分组和标注。`time_zone` 可以是 `"UTC"`（默认值）、IANA 名称（如 `"Asia/Shanghai"` 或 `"America/New_York"`），或由 `{ from_utc_seconds, offset_seconds }` 转换点构成的显式时间表（严格升序，至多 1024 个，偏移量在 ±18 h 以内；第一个偏移量也适用于其条目之前的时间）。该包对每个时区仅用 `Intl.DateTimeFormat` 在 1970–2100 年间解析一次 IANA 名称（至多约 262 次 DST 转换），并至多保留 32 个已解析的时区；超出该区间时采用最近的偏移量。引擎本身绝不读取浏览器的时区：`timeScale.timeZone` 只接受 `"UTC"` 或显式时间表。TradingView 对标列表中的时区（`TRADINGVIEW_TIME_ZONES`；WASM 的 `supported_time_zones_json()`）则可以通过 `ChartEngine::set_time_zone(&str)`（发生变化时返回 `Ok(true)`；WASM 的 `set_time_zone`）或顶层引擎选项 `timezone` 以名称传给引擎；它会被一次性解析为同一种时间表（1970–2100 年，原生约 4 ms），因此分组和标签与显式时间表一致，并且该名称还会本地化通用（非金融）时间坐标轴以及 `ChartEngine::time_zone_clock_text(utc_seconds, show_seconds)`。`ChartEngine::time_zone_id()`（WASM 的 `time_zone()`）返回已命名的时区，默认为 `Etc/UTC`，在安装了显式时间表期间返回 `custom`。未知时区、格式错误的时间表、超出范围的交易时段起点，或与已安装收盘时间标签（[收盘时间标签](#收盘时间标签)）的窗口不适配的交易时段起点，会在同一次调用中应用任何其他键之前抛出 `invalid_options`；不受支持或非字符串的 `timezone` 以同样方式拒绝该补丁。`time_scale().options()` 报告 IANA 名称（或时间表）和 `session_start`。Rust 宿主通过 `ChartEngine::set_exchange_offsets(UtcOffsetSchedule)` 设置显式时间表，通过 `set_session_start_seconds(i32)` 设置交易时段起点；引擎 JSON 选项接受 `timeScale.timeZone`（`"UTC"` 或转换数组）、`timeScale.sessionStart` 和 `timezone`。V2 持久化会往返保存它们：时间表始终保存，而 `timezone` 名称仅在已安装命名时区期间保存（显式时间表会将其清除）。当同一个补丁同时带有时间表和名称时，时间表决定分组和标签，名称决定通用坐标轴和时钟。导入早于这些键的文档时，会保留图表已安装的时区和交易时段起点。

仅显示的时间投影：`ChartEngine::set_future_time_projection(cadence_seconds, points)` 和 `set_past_time_projection(cadence_seconds, points)`（各自上限为 4,096 个点；传入 `None` 或 0 个点会将其清除；`has_future_time_projection` / `has_past_time_projection` 可读回）为时间轴上最后一根柱之后和第一根柱之前的空白区域标注标签。投影点仅是标签（没有数据行、基础索引或点数），且不会被持久化。

**交易日**。`session_start` 是交易日开始时刻相对交易所本地午夜的偏移秒数（默认 `0`，范围 ±86 399）。负值会把夜盘时段归入下一个交易日，例如 `-3 * 3600` 使 21:00 的中国期货夜盘从次日开始；起点为负时，本应落在周六或周日的交易日会顺延到周一，因此周五夜盘属于周一。Day/Month/Year 刻度标记、VWAP 的 `session`/`weekly`/`monthly` 重置以及枢轴点交易时段均使用交易日。每周周期从周一开始。

周日晚间开盘的市场（CME Globex，美国中部时间 17:00）将 `session_start: -25200`：周日 17:00 属于周一的交易日，周一 17:00 属于周二的交易日，因此 Day 标记、交易时段 VWAP 与每周 VWAP 重置以及枢轴点都与交易时段对齐。取 `0` 时，周日晚间自成一个交易日：在交易时段中间的午夜会出现 Day 标记和交易时段 VWAP 重置，而每周 VWAP 把周日晚间的柱留在上一周，并在该午夜重置。起点为负时，每个周六或周日的时刻都属于周一，而窗口放置（`session_slot_times`、`resample_boundaries`、`set_trade_stream_sessions`）假定一周在周五晚间开盘：这对中国期货是正确的，但周日开盘的市场应在每次调用中以 `session_start` 为 `0` 来放置其晚间窗口（参见 *“分时图”* 和 *“Tick 转 K 线与重采样”*）。

**遵循交易所时间的内容**。刻度边界（Day/Month/Year 来自交易日；小时和分钟标记基于交易所墙上时钟时间，因此在 DST 和非整点偏移下仍保持在交易所交易时间内）、每个界面上的内置标签（坐标轴刻度、十字光标、矩形绘图的坐标轴标签、delta 提示框、`create_tooltip` 以及无障碍文本）、VWAP 和枢轴点重置、交易时段高亮的小时门限与周末判断，以及倒计时窗口。Day 标记刻度标签标注交易日期；十字光标和提示框文本显示该时刻的交易所墙上时钟日期和时间。

**日历日期数据**。当每个有数据的金融系列都以 `business_day` 或 `"YYYY-MM-DD"` 时间给出时，图表将其时间点视为日历日期：它们在每个时区中保持自己的日期，绝不会被时区或 `session_start` 移位。任一金融系列中只要有一个数值时间，时间点就重新成为时刻。日线及更长周期的柱应以日历日期发送；以 UTC 午夜为时间的数值日线柱是时刻，在 UTC 以西的时区会显示为前一天晚上。类型化列输入始终是数值时刻。Rust 宿主通过 `ChartEngine::set_calendar_date_axis(bool)` 声明相同状态。

**格式化钩子**。`time_scale_options.tick_mark_formatter(time, tick_mark_type, locale, context)` 和 `localization.time_formatter(time, context)` 接收 UTC 秒数以及一个 `time_label_context`，其 `business_day` 对日历日期行为该日历日期，对时刻则为 `null`。宿主的 `time_formatter` 会覆盖所有输出时间点的界面：十字光标标签、矩形坐标轴标签、delta 提示框、`create_tooltip` 以及无障碍（除非无障碍选项设置了自己的 `time_formatter`）。没有它时，`create_tooltip` 和无障碍会输出十字光标标签（`chart.format_time_label()`）：按图表时区使用 `localization.date_format` 和 `localization.locale`，日内行附加当日时间，日历日期行则不附加。Rust 的 `TickMarkFormatterFn` 和 `TimeFormatterFn` 签名保持不变。配置了收盘时间标签（[收盘时间标签](#收盘时间标签)）时，两个回调收到的都是标签时刻（柱的收盘时间），而不是柱的开盘时间；在自己的格式化函数中加上一个周期的宿主，在采用该选项时必须去掉这一加法。

**倒计时时钟**。K 线收盘倒计时仅在时钟处于正在形成的柱的区间 `[last_bar_time, last_bar_time + bar_interval)` 内时显示；在区间之外——午休、隔夜、周末、提前收盘之后——它会隐藏而不是循环。日历日期柱在其日期对应的交易所交易日内形成（`session_start` 为负时，周一的柱从周五晚间的交易时段起始时刻开始：中国期货为周五的夜盘，CME Globex 这类周日开盘的市场为美国中部时间周五 17:00），而相隔 28 天或更多天的柱持续到各自所在日历月的月末。`chart.set_clock(() => utc_seconds)` 和 `offscreen_chart.set_clock(...)` 替换 `Date.now()` 用于倒计时跳动；`null`（或抛出异常或返回非有限值的时钟）回退到系统时钟。

**交易时段高亮**。`create_session_highlighting(series, { start_hour, end_hour })` 接受交易所本地时间的小数小时（`9.5` 即 09:30；`start_hour > end_hour` 时跨越午夜）；两者必须同时设置或同时不设置。`start_hour_utc`/`end_hour_utc` 仍是已弃用的别名（在默认 UTC 图表上结果相同）。回调重载仅对由实时 `update` 追加的行求值（`max_points` 保留策略淘汰最旧行时同样如此），并且仅在整体替换或历史在底层发生变化时才对整个系列重新求值。Rust 的 `SessionHighlightingOptions` 字段为 `start_hour`/`end_hour: Option<f64>`。

## 分时图

分时图在从开盘起的固定宽度内显示一个（或多个）交易时段：交易时段内的每一分钟在成交之前就有一个槽位，价格相对前收盘价绘制，均价为成交额除以成交量，并且视图从不滚动。它由普通系列和选项组合而成；`examples/web_demo/intraday.html` 是完整的参考宿主（一个 Asia/Shanghai 的 A 股交易日，以及通过 `?days=5` 得到的五日变体）。

**1. 交易时段槽位**。`session_slot_times({ date, windows, interval_seconds, time_zone, session_start?, convention? })` 返回某一个交易日期每根柱的 UTC 秒数。`windows` 是交易所本地的 `["HH:MM", "HH:MM"]` 数对，按时间顺序排列（至多 32 个；结束时间等于或早于开始时间表示跨越午夜，`"24:00"` 表示结束于午夜），`time_zone` 是 IANA 名称或显式时间表，结果上限为 100 000 个槽位并经过校验（`invalid_options`）。每个窗口使用该日期当日生效的偏移量进行转换，因此同一组窗口在跨越 DST 后仍保持在交易所交易时间内。`session_start` 是本次调用自带的，默认为 `0`；它不会从图表读取，因此中国期货宿主必须显式传入 `session_start: -10800`（若省略，21:00 的夜盘窗口会被放在日历日期（周一 21:00）而不是周五 21:00，且没有任何提示）。`session_start` 为负时，开始于交易时段起始时刻或其之后的窗口属于前一晚（周一对应周五晚间），与图表的交易日和中国期货夜盘一致。该放置方式假定一周在周五晚间开盘。

周日晚间重新开盘的市场（CME Globex）传入 `session_start: 0`，并对每个晚间日期各调用一次，使用 `windows: [["17:00", "16:00"]]`。该日期是时段开盘的那个晚上：周日日期放置周一的交易日（1380 个一分钟槽位，周日 17:00 至周一 16:00），周一日期放置周二的交易日，依此类推，因此在 `0` 下每次调用都以晚间的日历日期而不是交易日期为键。两次调用可得到相同的槽位：周日日期搭配 `[["17:00", "24:00"]]` 与 `session_start: 0`，然后周一日期搭配 `[["00:00", "16:00"]]`。在负的 `session_start` 下，不要给周一日期一个开始于 17:00 或之后的窗口：那会放置周五 17:00 至周六 16:00。哪些日期交易、节假日和提前收盘都是宿主的日历数据；为每个日期传入适用的窗口（没有夜盘的日期，其窗口列表中不含夜盘窗口）。它需要引擎模块：在 `init_wasm()` 或 `create_chart()` 之后调用。Rust 宿主调用 `aeris_charts_engine::session_slot_times(day, &windows, interval, chart.exchange_time(), convention)`。

`convention` 决定由哪个时刻来命名一个槽位。Aeris 的柱以开盘时间作为时间戳，因此默认的 `"bar_open"` 为 A 股交易日给出 240 个一分钟槽位（09:30..11:29、13:00..14:59）。同花顺和富途则以收盘时间标注一分钟，并把开盘集合竞价成交显示为其自己的第一个点：`"bar_close_with_open"` 复现它们的 241 个点（09:30、09:31..11:30、13:01..15:00）；`"bar_close"` 是不含开盘点的收盘标注形式。请使用数据提供方给分钟打时间戳所采用的约定。无论哪种方式，午休都不占宽度：上午最后一个槽位与下午第一个槽位相邻。

两类图表对这些约定的用法不同。按时刻采样的折线（经典分时价格线，每分钟末一个价格）使用以收盘为时间戳的槽位：其点就是那些时刻，241 个点是该折线自身的属性。区间柱（由 Tick 构建的 K 线或重采样的分钟）使用 `"bar_open"` 槽位和以开盘为时间戳的行，并通过 `bar_time_label` 选项（[收盘时间标签](#收盘时间标签)）输出其收盘时间；它们不会多出单独的第 241 根集合竞价柱，因为 09:25 的集合竞价成交并入第一根柱。

**2. 预留交易时段**。安装每一个槽位，无论是否有成交；未成交的分钟是空白数据行（`{ time }`）。将整个交易时段锁定在视图内并禁用手势（键盘移动遵循同样的开关）。将范围在第一个和最后一个槽位之外各保留半根柱：这样槽位中心距窗格边缘半根柱，使开盘和收盘的列在窄屏幕上仍位于窗格内，而不是跨在窗格左边缘上——参考实现的坐标映射会把未加内边距的第一个槽位放在那里：

```ts
const chart = await create_chart(container, {
  handle_scroll: false, handle_scale: false,
  leftPriceScale: { visible: true }, rightPriceScale: { visible: true },
});
const slots = session_slot_times({
  date: "2026-09-25", windows: [["09:30", "11:30"], ["13:00", "15:00"]],
  interval_seconds: 60, time_zone: "Asia/Shanghai", convention: "bar_close_with_open",
});
const row = (time: number, value?: number) => (value === undefined ? { time } : { time, value });
chart.time_scale().apply_options({
  time_zone: "Asia/Shanghai", time_visible: true, lock_visible_logical_range: true,
});
chart.time_scale().set_visible_logical_range({ from: -0.5, to: slots.length - 0.5 });
```

**3. 价格相对前收盘价**。`baseline_value: prev_close` 的基线系列在其上方为红色、下方为绿色（设置 `top_*`/`bottom_*` 颜色）；仅第一分钟有成交时，它绘制一段与柱同宽的线段。让其比例尺以前收盘价为中心，并把同样的价格放到第二个比例尺上，该比例尺为以前收盘价为基准的百分比模式：

```ts
const price = chart.add_series("baseline", {
  price_scale_id: "left", baseline_value: prev_close,
  top_line_color: "#f7525f", bottom_line_color: "#089981",
});
const percent = chart.add_series("line", { price_scale_id: "right", line_visible: false });
chart.price_scale("left").apply_options({ autoscale_center: prev_close, scale_margins: { top: 0.08, bottom: 0.08 } });
chart.price_scale("right").apply_options({
  mode: 2, base_value: prev_close, autoscale_center: prev_close, scale_margins: { top: 0.08, bottom: 0.08 },
});
price.set_data(slots.map((time, i) => row(time, closes[i])));   // closes[i] undefined for future minutes
percent.set_data(slots.map((time, i) => row(time, closes[i])));
price.create_price_line({ price: prev_close, line_style: "dashed" });
```

上下边距相等时，前收盘价在两个坐标轴上都位于窗格正中。

**4. 均价**。成交量和成交额是按时间对齐的独立系列；均价是以成交额为来源的 VWAP，在每个交易时段重置（`session_start` 定义交易时段）：

```ts
const volume = chart.add_series("histogram", {
  pane: 1, pane_stretch: 0.35, price_format: { type: "volume" },
  histogram_updown: true, histogram_updown_rule: "previous_close",
  up_color: "#f7525f", down_color: "#089981",
});
const amount = chart.add_series("line", { pane: 1, visible: false });
volume.set_data(slots.map((time, i) => row(time, shares[i])));   // shares, not lots
amount.set_data(slots.map((time, i) => row(time, turnover[i])));  // currency
const average = chart.add_vwap(price, volume, { price_scale_id: "left", color: "#f59e0a" }, { amount_source: amount });
```

保持单位一致：成交额以元计时，成交量必须以股计（将手数乘以 100）。均价线在每次交易时段重置时重新开始，因此在多日图表上，没有线段把某一天的最后一个均价连到下一天的第一个均价。若要以同样方式分隔各天的价格线，请给价格系列（及其百分比镜像）设置 `break_on_trading_day: true`；五日演示就是这样做的。

**5. 成交量颜色**。`histogram_updown_rule: "previous_close"` 按主价格系列的收盘价相对前一个有成交的收盘价为每根柱状列着色（收盘价不变算作上涨）；第一个有成交的分钟与作为主系列 `baseline_value`（或其比例尺的 `base_value`）给出的前收盘价比较。主系列是添加到图表的第一个系列。`up_color`/`down_color` 取代半透明的市场调色板；默认的 `"open_close"` 规则保持参考实现的行为。与参考实现一致，直方图的自动缩放范围始终包含其 `base`（0），因此柱状列的高度与成交量成比例，开盘分钟也不例外。

**6. 时间轴上的交易时段锚点**。`tick_marks` 取代坐标轴标签和垂直网格的自动刻度选择；位于未来空白槽位上的标记同样有效，`null` 恢复自动刻度，标签默认为其槽位的交易所时间标签：

```ts
// slot("10:30"): the slot whose exchange wall-clock time is 10:30.
chart.time_scale().apply_options({ tick_marks: [
  { time: slot("09:30") }, { time: slot("10:30") },
  { time: slot("11:30"), label: "11:30/13:00" },
  { time: slot("14:00") }, { time: slot("15:00") },
] });
```

时间不是槽位的标记（241 点约定下的 13:00）不绘制任何内容，因此应显式标注衔接点。对于多日图表，用日期标签标记每一天的第一个槽位，并将各天的槽位首尾相接地安装；标记的网格线将各天分隔开。没有标签时，每日开盘标记以粗体显示交易日期，一旦图表跨越多个交易日，图表的第一个槽位也包括在内（单日图表的第一个标记显示其时间）。显式标记至多 512 个，严格升序，标签至多 64 字节；标签保持在坐标轴内，会与其左侧相邻标签重叠的标签将被跳过（其网格线保留）。worker 图表以 `timeScale.tickMarks` 接受同样的列表，V2 持久化也会携带它。

**7. 实时分钟**。用 `update()` 填充下一个槽位，用 `merge()` 细化正在形成的分钟；`{ sequence }` 使每次投递幂等（过期或重放的投递会被以 `stale_sequence` 拒绝）。填充空白槽位绝不会移动已锁定的视图：

```ts
// The minute's first trade fills its whitespace slot (each series keeps its own sequence guard).
sequence += 1;
for (const series of [price, percent]) series.update({ time, value: first_trade }, { sequence });
volume.update({ time, value: shares_so_far }, { sequence });
amount.update({ time, value: turnover_so_far }, { sequence });
// Later ticks of the forming minute merge the latest price and the cumulative size.
sequence += 1;
for (const series of [price, percent]) series.merge({ time, value: last_trade }, { sequence });
volume.merge({ time, value: minute_shares }, { sequence });
amount.merge({ time, value: minute_turnover }, { sequence });
```

在第一笔成交之前，时间轴、其锚点和垂直网格已经显示整个交易时段，而价格坐标轴保持为空：与参考实现一致，没有数据的系列不会自动缩放。开盘成交随即立刻绘出（只有一个成交行的基线系列会画出一段与柱同宽的线段）。打开 `intraday.html?traded=0` 即可查看该状态。

## Tick 转 K 线与重采样

两种做法都在图表的交易所时间中构建普通 K 线，因此应先设置交易所时区（夜盘还需设置 `session_start`）。K 线以其开盘时间作为时间戳，即 Aeris 的规范柱时间。若要改为输出每根 K 线的收盘时间（A 股分钟线的 09:31 … 15:00），请设置 `bar_time_label`（[收盘时间标签](#收盘时间标签)）：K 线、其成交量、回放、倒计时以及宿主传入或读回的所有时间仍保持以开盘时间为时间戳。

### Tick 转 K 线

图表级成交流拥有 Tick 成交带；绑定到它的普通 K 线（或柱状）系列呈现该流的时间柱，成交量直方图由同一批柱派生：

```ts
chart.time_scale().apply_options({ time_zone: "Asia/Shanghai" });
const candles = chart.add_series("candlestick");       // add first: it tints the volume columns
const stream = chart.add_trade_stream("SSE:600000", {
  tick_size: 0.01, bar_type: "time", interval_seconds: 60,   // 300 for 5-minute candles
});
chart.bind_trade_bar_series_to_stream(candles, stream);
chart.set_trade_stream_sessions(stream, { windows: [["09:30", "11:30"], ["13:00", "15:00"]] });
const volume = chart.add_trade_volume_series(stream, 1);   // total volume per bar, pane 1
volume.apply_options({ histogram_updown_rule: "previous_close" });
chart.set_trade_stream_trades(stream, history);           // footprint_trade[]; typed columns also work
chart.update_trade_stream_trades_typed(stream, live_columns); // "tip" for in-order prints
```

**交易时段锚定**。没有交易时段时，时间柱对齐到 `anchor_seconds` 网格（默认 0，即 UTC 网格），适用于 24 小时市场。`set_trade_stream_sessions()` 按图表的 `time_zone` 和 `session_start` 把交易所本地窗口放置到每个交易日上（采用该日期当日生效的偏移量，因此遵循 DST），并且每个窗口在其开盘处重启柱网格：A 股 60 分钟柱在 09:30、10:30、13:00 和 14:00 开盘，US 60 分钟柱在 DST 变更前后均于美东时间 09:30 … 15:30 开盘，并且午休不占宽度。`interval_seconds` 为 86 400 时每个交易日一根柱，在其第一个窗口开盘。更改图表的时区或交易时段起点会重新放置窗口；`null` 恢复普通网格。只有整秒时间柱接受交易时段；无效窗口或其他柱类型会抛出 `invalid_options` 且不做任何更改。

`set_trade_stream_sessions()` 始终使用图表自身的 `session_start`（没有逐次调用的覆盖项），并且对所有日期使用同一个窗口列表，因此休市之后的日期无法去掉其夜盘窗口。对于周日开盘的市场（CME Globex），请将图表的 `session_start` 保持为 `0`，并传入一个跨越午夜的窗口 `[["17:00", "16:00"]]`：前一日查找会放置周日晚间，代价是 Day 标记和 VWAP 重置采用午夜交易日语义。`-25200` 的图表会把周一的窗口放在周五晚间，因此周日和周一的成交落在其后：`fold` 会把它们送入该窗口的最后一根柱（周六 15:xx），而 `exclude` 会丢弃它们。

**窗口外的成交。** `outside: "fold"`（默认值）保留每一笔成交：09:25 的开盘集合竞价成交开启 09:30 这根柱，11:30:00 与 15:00:00 的收盘成交则关闭其所在窗口的最后一根柱，与中国平台的显示方式一致。`outside: "exclude"` 则将盘前和盘后成交排除在所有柱之外（US 常规交易时段图表），但保留时间戳落在窗口收盘那一秒的成交，例如 16:00:00 的收盘撮合（closing cross）。被并入或被排除的成交仍然参与主动方分类。成交的 `session_id` 变化总是开启新的一根柱并重置交易时段 delta；窗口已经将上午和下午分开，因此每个日期一个 id 就足够了。

**引擎持有的系列。** 已绑定的 K 线或柱，以及成交量、CVD 和 delta 研究，仅由其成交流写入，且一个系列只有一个引擎写入方：对于不是 K 线或柱系列、带有 `max_points` 上限、或已经是足迹图、研究、重采样目标或合成柱系列的系列，`bind_trade_bar_series_to_stream` 会抛出 `invalid_options`（将已绑定的 K 线重新绑定到另一个成交流仍然允许）；并且不能基于由成交流写入的系列创建足迹图、重采样目标或合成系列。它们的 `set_data`、`set_data_typed`、`update`、`update_typed`、`merge`、`merge_typed`（无论是否带 `{ sequence }`）、`pop` 和 `set_ring_source` 均被拒绝：数据调用会将 `last_ingestion_diagnostics()` 记录为 `{ status: "rejected", code: "derived_series" }`，发出警告，且不做任何更改；`pop` 记录同样的拒绝，但不重绘也不触发 `data_changed`；`set_ring_source` 会抛出 `unsupported_operation`（使用 `null` 解除绑定仍然有效；在系列变为派生系列之前已绑定的环形缓冲区，会一直向 `frame_stats().ring_dropped_rows` 排空，直到解除绑定）。样式、窗格移动、可见性、`histogram_updown_rule` 以及研究自身的 `max_points` 仍然适用；已绑定的 K 线拒绝 `max_points`，因为它遵循成交流的保留策略。应改为向成交流写入数据。

Rust 宿主通过常规写入入口得到相同的拒绝（`false`、`0`、`None`、`Err(UnsupportedSeriesData)` 或 `Rejected(UnsupportedSeries)`），这些结果同样可能表示未知 id 或无效数据，因此可用 `ChartEngine::series_is_source_owned(id)` 区分引擎持有的系列，而 `ChartEngine::apply_momentum_histogram_colors` 对 delta 与成交量研究返回 `false`。`FootprintError::SeriesOwned` 是 `bind_trade_bar_series_to_stream` 在 K 线或柱已被重采样器、合成柱或研究（已转换为 K 线）写入时返回的错误，也是 `configure_footprint_series` 在任何系列由成交流、研究、重采样器或合成柱写入时返回的错误。`bind_trade_bar_series_to_stream` 先检查系列种类和 `max_points`，因此足迹图或标量研究会得到 `UnsupportedTradeBarSeries` 或 `InvalidAggregation`。

**实时、更正与回放。** 按序到达的成交就地更新正在形成的柱，位于柱边界或其之后的第一笔成交开启下一根柱（`"tip"`）；迟到或被更正的成交会使成交流重建一次（`"historical"`）。同一成交流上的成交量直方图、CVD/delta 研究和足迹图跟随每一次变化。`chart.set_replay_clock_micros(clock)` 恰好显示成交带中截至该时钟的柱；一根由被并入的开盘前成交开启、且领先于时钟的柱，会在时钟到达其开盘时间时出现。成交量柱取图表主（第一个）价格系列的涨跌色调（`histogram_updown`）；可像任何直方图一样重设其样式。

Rust 宿主调用 `ChartEngine::set_trade_stream_sessions(stream, Some(TradeSessionOptions { windows, outside: OutOfSessionPolicy::Fold }))` 和 `add_trade_volume_series(stream, pane)`；`SessionBarGrid` 为其他 Tick 消费方提供相同的放置方式。

### 重采样

`configure_resampled_series(target, options)` 在引擎内从源系列派生出 K 线或柱 `target`（以及可选的成交量直方图）。`resample_boundaries()` 根据交易时段窗口、交易所时区和宿主的交易日期推导出各周期：

```ts
import { resample_boundaries } from "@aeristerminal/aeris-charts";

const minute = chart.add_series("candlestick");
const minute_volume = chart.add_series("histogram", { pane: 1 });
minute.set_data(minute_bars);                 // stamped with each minute's open time
minute_volume.set_data(minute_volumes);
const hour = chart.add_series("candlestick");
const hour_volume = chart.add_series("histogram", { pane: 1 });
const boundaries = resample_boundaries({
  dates: ["2026-09-24", "2026-09-25"],        // host calendar; future dates are allowed
  windows: [["09:30", "11:30"], ["13:00", "15:00"]],
  time_zone: "Asia/Shanghai",
});
chart.configure_resampled_series(hour, {
  source: minute, volume_source: minute_volume, volume_target: hour_volume,
  interval_seconds: 3600, boundaries,
});
```

**周期。** `span: "window"`（默认值）为每个交易时段窗口返回一个边界，因此 5、15、30 和 60 分钟的柱在每个窗口开盘时重新开始（A 股 60 分钟柱位于 09:30、10:30、13:00、14:00；长度不是间隔整数倍的窗口，以一根更短的柱结束）。`span: "day"` 为每个交易日期返回一个边界，从其第一次开盘到最后一次收盘；配合 `interval_seconds: 86400`，则每个日期一根日线柱，时间戳为交易时段开盘时间，因此由延长交易时段分钟数据（美东时间 04:00–20:00）构建的 US 日线柱，在 DST 前后仍保持每天一根，尽管冬季的交易时段会越过 UTC 午夜。每个边界都以所请求的日期作为 `session_id`（`YYYYMMDD`）：对于交易时段在该日开始的市场，是交易日期；对于周日开盘的市场（见下文），是晚间日期。日期严格递增，并使用 `session_slot_times` 的放置方式（包括 `session_start` 为负的夜盘交易时段）；每个图表至多 20 000 个边界和 32 个重采样系列。宿主也可以传入自己的 `{ start_time, end_time, session_id }` 周期（例如用于周或月）。

`resample_boundaries` 有自己的 `session_start`，默认值为 `0`，独立于图表自身的值，因此中国期货需显式传入 `-10800`。对于周日开盘的市场（CME Globex），请传入 `session_start: 0`、晚间日期以及 `windows: [["17:00", "16:00"]]`；此时每个边界的 `session_id` 即所请求的晚间日期（对于 2024-01-07 周日开启、属于周一交易日的交易时段，为 `20240107`）；或者自行构建 `{ start_time, end_time, session_id }` 周期。切勿将 `-25200` 与周一日期一起传入：它会把该交易时段放在周五 17:00 至周六 16:00，导致周日和周一的行落在所有边界之外而被省略。

窗口列表适用于一次调用中的每个日期。拥有日历的宿主对每组窗口调用一次 `resample_boundaries` 并拼接各数组，这些数组只需有序且互不相交：正常日期使用夜盘与日盘窗口，夜盘不交易的日期（例如休市后的第一个交易日）仅使用日盘窗口。使用 `span: "day"` 时，柱的时间戳取第一个窗口的开盘时间，因此共用的夜盘加日盘列表，会把该日期的日线柱打上从未交易的夜盘交易时段的时间戳。

**源行。** 源行必须以柱开盘时间作为时间戳；位于所有边界之外的行会被省略，因此宿主在重采样之前，仍须通过减去一个间隔，将数据提供方的收盘时间戳（09:31 … 15:00）转换为开盘时间戳。若数据同时带有单独的开盘集合竞价分钟（241 根柱的数据把它标为 09:30，与收盘时间戳 09:31 并存），则必须在重采样之前将该行合并进第一分钟：若对其做平移，它会落在第一个窗口之前，并连同其成交量一起被省略。直接绘制而不做重采样的 241 根柱数据，则可以改为同样将集合竞价行前移：它落在 09:29，位于所有窗口之外，而 `bar_time_label` 将其打印为 09:30（参见 [收盘时间标签](#收盘时间标签)）。空白数据行（`session_slot_times` 预留）为其桶预留位置但不含价格：未成交的桶是空白柱，正在形成的桶截止于其最后一个已成交行。重采样需要时间轴：在坐标轴为非时间柱序列（成交笔数流、成交量流或区间流，合成柱）的图表上会被拒绝，并且这类序列不能加入已有重采样系列的图表。

**实时更新。** 对源系列或其成交量调用 `update`、`merge` 以及类型化批量，仅刷新受影响的尾部：派生柱中未变化的前缀被保留，尾部从第一个可能含有变化行的桶开始重建，因此实时的一分钟只会重读一个桶，绝不会重读历史，并且超出最后一个源行的边界（提前配置的日期）不产生任何开销。`pop`、源系列头部的保留上限裁剪、会丢失某根派生柱的向后回放，以及完整的 `set_data`，都会触发一次重建。`chart.resample_stats(target)` 报告 `rebuilds`、`tail_refreshes` 和 `rows_scanned`。在回放时钟下，正在形成的柱仅聚合时钟当时或之前的行。要到达已配置边界之后的日期，请再次调用 `configure_resampled_series`（一次重建）；也可以提前包含新日期，因为没有数据的日期不会产生柱。重新配置会保留绑定的源（换用其他源会抛出 `invalid_options`）。无论配置顺序如何，绑定都绝不链式连接：目标（或成交量目标）不得是另一个绑定的源（或成交量源）或输出，且绑定的成交量源不得是其自身的成交量目标；每种情况都会抛出 `invalid_options`（"resampling dependencies may not be chained or cyclic"）且不做任何更改。

目标由引擎持有：对其调用 `set_data`、`update`、`update_typed`、`merge` 和 `merge_typed` 会被拒绝（`last_ingestion_diagnostics()` 报告 `status: "rejected"` 及 `code: "derived_series"`）且不做任何更改，`pop` 记录同样的拒绝。目标不得是足迹图、绑定到成交流的 K 线、成交研究或合成柱系列（`invalid_options`）；不过，绑定到成交流的 K 线或成交量研究可以作为该绑定的源。移除绑定中的任一系列（源、成交量源或目标），会连同其目标系列一并移除该绑定，与指标输出相同。`chart.resampled_bars(target)` 返回派生柱，附带其 `session_id` 以及聚合的源行数量。Rust 宿主调用 `ChartEngine::configure_resampled_series(source, volume_source, target, volume_target, ResampleOptions { interval_seconds, boundaries })` 和 `aeris_charts_engine::resample_boundaries(&days, &windows, chart.exchange_time(), ResampleSpan::Window)`。

## 收盘时间标签

每根柱都以其开盘时间（Aeris 的规范柱时间）作为时间戳，并以该时间戳作为其标识：行、系列数据、十字光标事件、快照、倒计时、回放、交易时段、交易日、绘图、标记、提醒、重采样，以及宿主传入或读回的每一个时间，均为开盘时间戳。因此 A 股分钟图保存的是 09:30 … 14:59。而按柱收盘时间读取柱的用户则期望看到 09:31 … 15:00。`time_scale_options.bar_time_label`（声明式写法为 `timeScale.barTimeLabel`，worker 图表也接受）仅改变图表为柱打印的文本：

```ts
chart.time_scale().apply_options({
  time_zone: "Asia/Shanghai", time_visible: true,
  bar_time_label: {
    anchor: "close", interval_seconds: 60,
    windows: [["09:30", "11:30"], ["13:00", "15:00"]],   // optional, exchange-local
  },
});
```

默认值 `"open"` 不改变任何内容。使用 `{ anchor: "close", … }` 时，柱的打印时间是其开盘时间加上 `interval_seconds`（1 至 86 399），或者当该柱是其所在窗口较短的最后一根柱时，为包含其开盘时间的交易时段窗口的结束时间。设为 `"open"` 可恢复开盘文本。`time_scale().options().bar_time_label` 报告 `"open"` 或 `{ anchor, interval_seconds, windows }`。无效标签（间隔超出 1..86 399、超过 32 个、无序，或对图表的 `session_start` 而言长度为零的窗口、未知键）会抛出 `invalid_options` 且不做任何更改；标签与同一次调用中的 `time_zone`、`session_start` 和 `tick_marks` 一起，依据该次调用所安装的交易时段起点进行校验。

**打印标签的位置。** 十字光标时间标签、自动刻度标签、显式 `tick_marks` 的默认文本、绘图坐标轴标签、绘图统计（`date_time_range`）与预测目标时间、delta 提示框的时间行、`create_tooltip`，以及无障碍文本。宿主的 `tick_mark_formatter` 和 `localization.time_formatter` 接收标签时刻。小时与分钟的刻度权重跟随打印时间，因此“10:00”刻度位于整点收盘的柱上（即 09:59 开盘的那根柱），绝不会位于 10:00 开盘的柱上；日、月、年权重以及每一次交易日重置仍然跟随柱自身的交易日，因此在午夜结束的窗口的最后一根柱，打印的是次日的 00:00，但仍属于它自己的交易日。worker 图表在每个由引擎绘制的界面上打印该标签；由包持有的提示框与无障碍文本属于主线程界面。标签是普通文本：帧契约、绘制列表以及任何后端均不发生变化。

**仍保持开盘时间戳的内容。** 所有传入与传出的时间：`series_data`、`bars_in_logical_range`、十字光标与点击事件、系列快照、`coordinate_to_time`、可见范围、十字光标同步位置、标记、成交、提醒、绘图锚点、`tick_marks[].time`、倒计时、回放时钟、交易时段高亮以及重采样。数据提供方按收盘时间给柱打时间戳的宿主，仍须在传入时将其转换为开盘时间戳（减去一个间隔），并在传出时读取开盘时间戳。原生竖线标签保留其宿主文本。无障碍选项的 `time_formatter` 接收宿主自己的数据时间，而不是标签。显式刻度标记按标识指明其柱：要为 11:29 开盘（打印 11:30）的柱添加标签，请传入 `11:29`；位于仅作为标签存在的时刻 11:30 的标记不匹配任何柱，也不绘制任何内容。

**每个图表一个间隔。** `interval_seconds` 是图表的主柱间隔。重采样目标与其源共用一条时间轴，因此无法为一分钟源与小时目标逐系列打标签：请选择用户所读柱的间隔，并在切换时间周期的同一步骤中更新该选项（在此之前标签使用旧间隔）。

**窗口与较短的最后一根柱。** `windows` 是交易所本地的 `["HH:MM", "HH:MM"]` 对，按图表的 `time_zone` 和 `session_start` 放置，与 `session_slot_times` 完全一致（至多 32 个；结束时间不晚于开始时间表示跨越午夜，`"24:00"` 表示在午夜结束）。它们使窗口的最后一根柱精确结束：US 09:30–16:00 的小时线交易时段有一根较短的 15:30 柱，在 DST 变更前后都打印 16:00，HK 09:30–12:00 的上午窗口，其 11:30 的柱打印 12:00。没有 windows 时，较短的最后一根柱打印其开盘时间加间隔（16:30）。开盘时间不在任何窗口内的柱同样打印其开盘时间加间隔，这就是宿主提供的 241 根柱数据（将集合竞价行前移一个间隔至 09:29）在没有引擎构建的集合竞价柱的情况下，打印为 09:30、09:31 … 15:00 的原因。windows 与 `session_start` 必须相互匹配：当安装了带 windows 的标签时，若某个 `session_start`（通过 `apply_options`、`timeScale.sessionStart`、V2 导入或 `set_session_start_seconds`）使这些窗口无法被放置，则会抛出 `invalid_options`（Rust 中为 `ExchangeTimeError::BarTimeLabelWindows`）且不做任何更改，因此已保存的文档始终可以再次导入。要同时移动两者，请在一次 `apply_options` 调用中一并发送（标签依据该次调用的起点校验），或先将标签设为 `"open"`。`time_zone` 变更绝不会与 windows 冲突；仅当某个时刻的窗口被 DST 切换折叠时，该时刻才打印开盘时间加间隔。同花顺和富途如何标注较短的最后一根小时线柱，未能验证（没有可用的实时终端），因此在依赖它之前，请将打印出的窗口结束时间与你的参考终端比对。

**不适用的场景。** 日历日期坐标轴和非时间柱序列（成交笔数流、成交量流和区间流，合成柱）打印它们自己的时间，并忽略该选项。间隔柱不会多出单独的第 241 根集合竞价柱：引擎构建的 K 线将 09:25 的集合竞价成交并入第一根柱，正如由 Tick 构建的 K 线已经做的那样；而分时折线的 241 个点仍然是按收盘时间戳标注的时刻槽位的属性（`session_slot_times` 与 `"bar_close_with_open"`）。

标签存放在选项存储中，因此只要它不是 `"open"`，V2 持久化就会携带它；导入不带该键的文档会保留已安装的标签（若文档的 `sessionStart` 与已安装的 windows 不匹配，则整份文档被拒绝），而默认图表的文档保持不变。Rust 宿主调用 `ChartEngine::set_bar_time_label(BarTimeLabel::Close { interval_seconds, windows })`、`bar_time_label()` 和 `bar_label_time(open_time)`（柱所打印的时刻）；引擎 JSON 选项为 `timeScale.barTimeLabel`，WASM 导出为 `bar_label_time(seconds)`。

## 实验性接口

自定义系列、窗格/系列/画布图元、导出的自定义系列/图元功能包、内置插件辅助函数、离屏 worker 图表、拆分网格辅助函数和快捷键辅助函数均为公共实验性 API。其当前的生命周期与隔离行为已有测试，但其确切类型可能在 1.0 之前的次版本发布中变化。`create_delta_tooltip()` 在 K 线系列上被有意设为不可用；K 线改用常规的悬停 `create_tooltip()`。delta 提示框仍可用于面积、折线、柱等非 K 线系列，并直接组合进可刷选面积交互中。可刷选面积是基于普通 `area` 系列的组合，而不是一种单独的承载数据的系列类型；旧的 `brushable_area` 输入写法仍作为兼容别名保留，并规范化为 `area`。该辅助函数在挂载期间，将鼠标/触控笔的主键窗格拖动保留给对比刷选；坐标轴拖动保持其普通的自动/手动比例尺行为，且该辅助函数不会全局禁用图表的滚动或比例尺选项。移除该辅助函数会立即恢复普通面积图的窗格拖动路径。常规的 `create_tooltip()` 是规范的结构化行情数据检视器。其引擎快照为每种普通系列呈现方式（包括面积和折线）保留 Open/High/Low/Close；标量行自然在四个字段中报告相同的值，而被馈入保留的 OHLC 行的面积/折线系列可以渲染 Close，同时仍能检视完整的柱。宿主可以绑定显式的 `volume_series`，以添加与时间戳对齐的 Volume 行；图表绝不会猜测哪个直方图代表成交量。诸如 `create_rectangle_drawing()` 和 `create_rectangle_drawing_tool()` 之类的便捷辅助函数，是针对引擎持有的规范绘图类型的控制器；它们不定义单独的矩形功能或持久化标识。同样，视觉形状类似绘图的图元辅助函数仍然是图元，演示与宿主应当如此呈现。扩展在宿主渲染时运行，不得从渲染回调中重入图表变更，拥有其外部对象和持久化，并且恰好收到一次拆除通知。回调失败在宿主边界处被隔离，因此一个扩展不会妨碍其他扩展的拆除。任意扩展对象或可执行回调绝不会从持久化的 JSON 中重建。

## 错误与生命周期

可预期的失败会抛出 `AerisChartsError`，它是 `Error` 的子类，带有以下稳定错误码之一：`disposed`、`invalid_handle`、`stale_handle`、`invalid_data`、`invalid_options`、`unsupported_operation`、`serialization_error`、`persistence_version_error`、`extension_error`、`renderer_platform_error` 或 `resource_limit`。

## 品牌更名

浏览器包为 `@aeristerminal/aeris-charts`（含 `@aeristerminal/aeris-charts/react`）。原先带有品牌名的错误导出已重命名为 `AerisChartsError` 和 `AerisChartsErrorCode`；迁移时请更新导入与 `instanceof` 检查。Rust 使用方使用仅限仓库的 `aeris_charts_*` crate。

每一个原先带有品牌名的公共标识符都已被彻底重命名：

| 公共接口面 | 新标识符 |
| --- | --- |
| 浏览器错误类 | `AerisChartsError` |
| 浏览器错误码类型 | `AerisChartsErrorCode` |
| React 图表组件 | `AerisChart` |
| React 图表 props | `AerisChartProps` |
| React 图表 hook | `useAerisChart` |
| WebAssembly 图表类 | `AerisChart` |
| WebAssembly 工作区类 | `AerisWorkspace` |
| GPUI 已准备帧类型 | `PreparedAerisFrame` |
| GPUI 视口类型 | `AerisViewport` |

持久化的图表与工作区 schema 标识符、浏览器事件名称、生成的 WebAssembly 资源名称、DOM ID/类名、CSS 选择器以及基准测试环境变量，现在均使用 `aeris_charts` 前缀。已停用的品牌没有任何兼容别名。

`chart.remove()` 是幂等的。移除之后，每个需要有效图表状态的操作都会抛出 `disposed`。调用方已持有的标识字段仍然可以读取。已移除的系列、绘图、窗格和价格比例尺会抛出 `stale_handle`；过期的句柄绝不会指向替代对象。扩展清理异常仍被隔离，并作为开发警告报告。

创建具名价格比例尺时，会拒绝空的、保留的、重复的或过长的 ID、不存在的窗格，以及每个窗格的资源上限，且不产生部分变更。重新绑定到未知比例尺会抛出 `invalid_options`；移除内置比例尺或非空的比例尺会抛出 `unsupported_operation`。具名比例尺 ID 区分大小写，是窗格内局部的、长度为 1-128 字节的 UTF-8 字符串，每个窗格至多 16 个。

干净的批量/当前柱写入保留其无分配/null 诊断路径。被修复、丢弃、重排、去重、拒绝或语义异常的输入，可通过 `series.last_ingestion_diagnostics()` 获取；OHLC 异常只报告，不改写数值。数值时间必须是有限的整数 UTC 秒，且位于闭区间 `-62167219200..253402300799`（年份 0000..9999）内，并且绝不自动转换。当缩放后会得到范围内的值时，拒绝原因会提示毫秒、微秒或纳秒。任何无效时间戳都会使直接的 set/update 批量被原子地拒绝；无效的单次 update 会使当前系列保持不变并在控制台发出警告，`update_typed` 亦如此。共享环形缓冲区的排空会逐行拒绝格式错误的行，并将其计入 `frame_stats().ring_dropped_rows`。worker 图表通过 `offscreen_chart.last_ingestion_diagnostics()` 暴露最近一次结果。

流式写入保持参考实现的 `series.update` 语义：一个数据点会替换其时间处的整根柱。`update()` 会对那些会静默改写柱的载荷报告机器可读的诊断 `code`，并指向 `merge()`：`value_on_ohlc_series`（`{ time, value }` 会压平 K 线/柱）、`price_less_payload`（例如 `{ time, volume }` 会变成空白数据），以及被拒绝的 `partial_ohlc`。对引擎拥有的系列（与成交绑定的 K 线或研究、重采样或合成的柱）的写入，在每条数据路径上都会以 `derived_series` 被拒绝；足迹图句柄则改为抛出 `unsupported_operation`。`series.merge(point, options?)` 是由引擎拥有的部分更新路径：存在的 open/high/low/close/value 字段会覆盖，缺失的字段保留现有柱的值，K 线/柱的结果会被规范化，使 `high >= max(open, close)` 且 `low <= min(open, close)`（针对新时间的仅含收盘价的 Tick 会创建 O=H=L=C；标量系列取 `value`）。不含价格字段的合并会以 `empty_merge` 被拒绝；成交量和成交额会合并到各自的系列中。`series.merge_typed(columns, options?)` 是列式形式：第 `i` 行的合并方式与 `merge()` 相同，其中 `NaN` 条目和省略的列视为缺失；各行按输入顺序应用，仅触发一次引擎同步；只要有一行无效，整个批量即被拒绝（`offscreen_chart.merge_typed` 是 worker 形式）。自定义、高级和足迹图系列会抛出 `unsupported_operation`。流式写入的数据点若显式给出 `color`/`wick_color`/`border_color`，即使此前没有任何数据点带有颜色，也会为该柱着色，与 `set_data` 中同一项的行为完全一致。

`update`、`merge`、`update_typed` 和 `merge_typed`（包括 `offscreen_chart` 的类型化形式）接受 `{ sequence }`，其值为非负安全整数；无效值会被拒绝，并像无效数据一样发出警告。若提供了 sequence，且其不大于该系列上已应用的最后一个 sequence，则会被判为过期而拒绝（`code: "stale_sequence"`、`last_sequence`），不会改变数据，也不会触发 `data_changed`；不带 sequence 的调用始终会应用。完整的 `set_data`/`set_data_typed` 会清除该防护，或将其 `{ sequence }` 作为基线写入。该防护对每个系列为 O(1)，仅存在于运行时，不会持久化。自定义和高级系列在传入 sequence 时会抛出 `unsupported_operation`。Rust 宿主使用 `ChartEngine::merge_series_bar`、`merge_series_bars`、`update_series_bar_sequenced`、`update_series_bars_sanitized_sequenced` 和 `set_series_update_sequence`。

## 绘图锚点、磁吸与价格基准

每个绘图锚点都具有时间标识。`drawing.points()` 返回 `{logical, price, time}`，其中 `time` 为 UTC 秒数：小数位置在相邻柱的时间之间插值，超出数据范围的位置则按当前柱间隔外推（未来区域矩形的时间标签显示的就是这个外推出的日期）。`chart.add_drawing()` 和 `drawing.set_points()` 接受 `{logical, price}`、`{time, price}` 或两者同时给出；两者同时存在且不一致时，以 `time` 为准，`points()` 的输出可以精确往返。在图表尚无数据时添加的时间锚点保持待定状态，数据到达后再解析。非时间柱图表不报告 `time`。

数据变化时，锚点跟随合并后的时间轴。新旧数据共有的时间戳保持精确映射，同间隔的保留裁剪或窗口平移则保持其按柱数的外推。前置追加的历史数据会按时间重新放置位于旧数据左侧的锚点，因此比较短的分时历史更早的绘图，在宿主分页载入更多柱的同时仍保持其日期。当数据没有共有的时间戳，或间隔发生变化（1m 到 1h 到 1D、先清空再设置、品种重新加载）时，每个锚点都根据其时间在新坐标轴上解析：10:37 落在从 10:00 的小时柱到下一根柱的 37/60 处，在日线数据上则落在当天的那根柱内。撤销/重做历史以及进行中的创建或拖动状态遵循相同的规则。同步载荷（`drawing_sync_payload`）和剪贴板载荷（`copy_drawings`）携带锚点时间，因此接收端图表会在自己的间隔和历史窗口上解析它们。每一次已提交的绘图变更（API 调用、放置、手绘笔画、移动了内容的指针拖动或键盘编辑、文本编辑）都会推进同步修订，因此已同步的单元格会接受下一个载荷。剪贴板载荷与持久化的绘图文档一样有界（至多 10,000 个绘图、250,000 个锚点和 8 MiB）：超出这些上限时 `copy_drawings` 抛出 `resource_limit`，所列绘图均不存在时抛出 `invalid_data`；`clone_drawing` 可复制图表持有的任意绘图。命名模板（`drawing_template`、`apply_drawing_template`）仅携带样式（外加仓位工具的 `position_account_size` 和 `position_risk_percent`）：绝不携带绘图的名称、分组、修订、可见性、锁定、z 序、周期可见性、价格比例尺或文本，因此应用模板只会重设目标的样式，并保留其标识和自身的文本。

`chart.set_drawing_magnet_mode("off" | "weak" | "strong")` 设置持久的工具栏磁吸（默认 `"off"`，即保留历史行为：仅在按住 Ctrl/Cmd 时启用磁吸）。`"strong"` 始终会把放置或编辑中的锚点吸附到指针下方那根柱上最近的已渲染 OHLC 值；`"weak"` 仅在 12 CSS px 范围内吸附（`DRAWING_WEAK_MAGNET_DISTANCE`）。绘图自身的 `magnet` 选项会为该绘图提升模式。按住 Ctrl/Cmd 会切换实际生效的磁吸（未启用时变为 strong，已启用时变为 off）。触控输入没有修饰键，使用图表的模式。键盘微移绝不吸附。

对于格式错误或超出范围的选项补丁，`chart.add_drawing()` 会抛出 `invalid_options`，而不是丢弃这些选项。在拖动进行中执行撤销/重做，会先取消该拖动。键盘编辑（`Enter`、`Tab`、方向键）会循环切换绘图的可编辑手柄（每个锚点、矩形的八个边界手柄、Long/Short Position 的目标、入场、宽度和止损控件，或绘图族放置在其几何上的手柄，见下文各族的说明），并按微移距离移动当前聚焦的手柄。`drawing_handle_count()` 统计这些手柄的数量。每次微移都会实时应用；`Enter` 会把整个键盘编辑作为一个撤销步骤提交，`Escape` 则把绘图恢复为编辑开始时的样子。未移动任何内容的微移（绘图已锁定、绘图无法沿该轴移动、被窗格边缘钳位）不会改变任何内容，并会据此播报。

绘图自身的文本会在图表的内联编辑器中就地编辑，适用于每一种会绘制文本的绘图：文本工具、趋势线的标签、每种线条、通道、斐波那契、叉形线、形态和形状工具的文本（单行；当标签沿线段排布时，文本沿描边旋转），以及下文列出的“投影与标注”工具的文本框（多行）。档位、点和波浪标签、比率与统计信息均为引擎格式化的文本，仍仅可通过选项设置。有九个工具接受 `text`，但从不在图表上绘制或编辑它：`forecast`、`bars_pattern`、`price_range`、`date_range`、`date_and_price_range`、`projection`、`flag_mark`、`icon` 和 `simple_tag`（其 `text` 即价格坐标轴标签）。双击已选中的绘图，或双击未选中绘图的文本（其第一次点击会选中它），或在图表拥有焦点且绘图已选中时按 Enter 或 F2（在其无障碍绘图目标上按 F2，此时 Enter 仍用于几何编辑），即可打开编辑器；已锁定、已隐藏以及按周期隐藏的绘图不会打开它，文本完全位于其窗格绘图区之外的绘图同样不会（引擎在每个宿主和每条路径上都采用这一规则：双击、Enter、F2、放置以及直接开始编辑）。引擎决定编辑哪段文本以及它所在的位置，因此没有文本的未选中绘图没有可供双击的标签：请先选中它再双击，或按 Enter 或 F2，或通过其选项添加第一个标签（只有趋势线会在悬停时提示 `+ Add text`）。未选中绘图的文本在悬停时响应文本光标，在点击时响应选择，除非该处有位于更上层的绘图或已选中绘图的锚点手柄。输入时实时重绘，按 Enter 或离开编辑器即提交，按 Escape 则恢复原文本。整次编辑为一个撤销步骤，并仅在提交时一次性反映到 `drawing_sync_payload` 中。文本长度以 `MAX_DRAWING_TEXT_BYTES` 为上限（65,536 字节：选项中更长的 `text` 会被拒绝，且不会应用补丁的其余部分，输入则在上限处停止）；文本工具、趋势线标签以及其余所有沿线标签都保持为单行（换行符会变成一个空格），而各族的文本框可容纳多行（Shift+Enter 添加一行，粘贴时插入纯文本）。编辑器是带标签的文本框，通过无障碍 live region 播报其打开和关闭，并把焦点归还到打开它的位置。放置文本工具、`anchored_text`、`note`、`callout`、`comment`、`signpost` 或 `simple_annotation` 时会立即打开编辑器，插入符位于默认文本之后；提交或按 Escape 都会保留该绘图，即使文本已被清空（只有文本工具在文本为空时会自行移除）。放置 `price_note`、`price_label` 或箭头标记（它们一开始没有自己的文本）不会打开编辑器。

双击仅在点击能够选中该绘图的位置对已选中的绘图生效：其文本、其主体或其某个手柄。第一次点击落在交易对象或警报控件上的一对点击不作用于任何绘图，在绘图保持选中的状态下于其他位置双击同样不作用于任何绘图。

在双击打开编辑器之后，宿主的 `dbl_click` 订阅者仍会运行。因此，把双击绑定到自己设置面板的宿主会同时看到两者：处理程序运行时编辑器已经打开，而在面板控件上调用 `focus()` 会将其关闭（编辑器按原文本提交，不记录撤销步骤，也不产生同步修订），并让焦点停留在该控件上，因此绘图与之前完全一致。编辑器打开期间点击宿主控件会以同样方式将其关闭，并让焦点停留在该控件上；只有 Enter 和 Escape 会把焦点归还到图表内编辑器打开时所在的位置。

绘图的虚线或点线 `style` 在 WebGPU、Canvas2D、GPUI 和原生渲染上绘制出相同的虚线，通用系列的 `line_style` 亦然：引擎会在任何后端绘制之前，把这些描边拆分为虚线段。

### 价格基准（复权）切换

引擎没有复权因子模型；调整后的 OHLC 由宿主计算。绘图通过三次调用跟随基准切换：

1. 以新基准替换系列数据（`series.set_data()`）；指标会重新计算。
2. 调用 `chart.rescale_drawing_prices(segments, basis_label)`。每个分段为 `{from_time?, to_time?, factor}`（UTC 秒数，`[from, to)`，互不重叠，factor 取 1e-6..1e6）。时间落在某分段内的每个锚点价格都会乘以该因子（在 Tick 柱、成交量柱或区间柱图表上，锚点的时间是其所在柱的开盘时间）；Long/Short Position 的各价位使用入场锚点所在的分段，江恩扇形线或固定方格的 `scale_ratio`（每根柱的价格）随其第一个锚点所在的分段缩放。这是数据基准的变更，而非编辑：它同样适用于已锁定的绘图，会以新基准重写撤销/重做历史，并且不记录撤销步骤，因此撤销绝不会恢复旧基准的价格。重新缩放是原子的：分段无效，或因子会使任何价格超出受支持的数值范围，则不会改变任何内容。标签参数会在同一步骤中设置基准。仓位进度会针对新的 K 线重新评估。
3. 保持 `chart.drawing_price_basis()` 同步（`set_drawing_price_basis()` 也可单独设置它）。该标签会被持久化，并随同步和剪贴板载荷携带。在恢复或同步之后，将其与数据基准比较，两者不一致时进行重新缩放。

对于前复权 ↔ 不复权的切换，分段即除权日区间，取每个区间的累计因子（例如 1 拆 2 的拆股之后为 `{to_time: ex_date, factor: 0.5}`）。`chart.set_drawings_points([{drawing, points}])` 会以一个撤销步骤原子地改写多个绘图的锚点，用于宿主计算出的编辑。价格线、警报、标记和交易对象仍归宿主所有；宿主自行改写它们。图表发出的交易意图（来自 Long/Short Position 的括号订单、订单拖动）携带的是显示基准价格，因此在非原始基准下，宿主必须先把它们转换为原始价格，再提交给券商。

## 绘图族

B8 绘图族扩展了 `drawing_kind` 目录。其工具与其他所有绘图使用相同的放置、选择、手柄、拖动、磁吸、键盘编辑、锚点时间标识、历史、持久化、剪贴板、同步和 schema API。部分工具会在其几何上、锚点之外增加手柄（在各族中分别列出）；这些手柄的拖动、磁吸和键盘微移与锚点手柄相同，每次拖动以及每个键盘编辑会话都是一个撤销步骤，其中包括它所编辑的任何选项。各族专属的选项按族各占 `options.tool_options` 下的一个块；补丁会对其进行深度合并（缺失的键保留其值，`null` 会重置一个块，无效的块会以 `invalid_options` 拒绝整个补丁）。schema 描述符使用 `tool_options.line.stats_position` 这样的点分路径命名这些选项，`drawing_kind_options()` 返回解析后的块。种类默认值（例如射线的 `extend_right`）即 schema 默认值，不会写入持久化。

<!-- B8: lines — begin -->
### 线条

- `ray`、`extended_line`、`info_line`、`trend_angle` 和 `arrow_line` 放置两个锚点。在这些工具上，`extend_left` 向第一个锚点之外延伸，`extend_right` 向第二个锚点之外延伸，均沿直线自身方向延伸至窗格边缘；射线默认为 `extend_right`，延长线默认为两者都启用。端帽（`stroke_start`、`stroke_end`）仅绘制在未延伸的端点上；箭头线将 `stroke_end` 默认为 `"arrow"`。`text` 标签像趋势线的标签一样沿线段排布，并以同样方式就地编辑；只有趋势线会在悬停时提示 `+ Add text`。
- 可见的 `labels` 渲染为一个统计框：第一行为价格、价格变化、百分比变化和 tick 数；下一行为柱数、时间范围和持续时间；最后一行为屏幕角度和 CSS px 距离。数值使用绘图比例尺的价格格式化器和锚点的时间标识。`info_line` 默认启用价格变化、百分比变化、柱数、持续时间和角度。`tool_options.line.stats_position`（`"start"`、`"middle"`、`"end"`；默认 `"end"`）把该框放置在第一个锚点之外、中点下方或第二个锚点之外。该框是选择和拖动的主体目标。`volume_in_range` 不渲染任何内容，因为绘图不携带成交量来源。
- `trend_angle` 会增加一条指向第二个锚点的水平虚线参考线、连接到线段的圆弧，以及以度为单位的屏幕角度（上升为正，-90 至 90）。
- `cross_line` 放置一个锚点，并绘制穿过该锚点的全幅水平线和垂直线，水平线的价格标签显示在坐标轴上。其主体可沿两个坐标轴拖动。
- `horizontal_segment` 使两个锚点保持在同一价格上，`vertical_ray` 和 `vertical_segment` 则使两个锚点保持在同一根柱上。放置、拖动或提供某个锚点时，会把共享坐标移动到另一个锚点上，该坐标取自最后放置或拖动的那个锚点，因此提供或导入的不一致锚点对会以同样方式被修复。垂直射线默认为 `extend_right`，使其从第一个锚点穿过第二个锚点，延伸到第二个锚点一侧的窗格边缘。
- `price_line` 放置一个锚点，并绘制一条从该锚点到窗格右边缘的清晰线条，锚点价格印在线条起点上方，并标注在价格坐标轴上（KLineChart 的价格线）。其主体即射线。它自身的 `text` 是通用线条标签，放置方式与水平射线的相同，不会取代价格。
- `drawing_kind_options()` 对每个线条工具返回 `{ kind: "line", stats_position }`。
<!-- B8: lines — end -->
<!-- B8: channels — begin -->
### 通道

- `parallel_channel`、`flat_top_bottom` 和 `disjoint_channel` 放置三个锚点。前两个锚点定义基线。第二条线跨越相同的柱，并位于经过第三个锚点的那条线上，无论该锚点位于哪根柱上：平行通道为基线在屏幕上垂直移动后的线（在每种比例尺模式下都平行），平顶/平底通道为位于第三个锚点价格处的水平线，不相交通道为基线斜率取镜像后的线。`extend_left` 和 `extend_right` 分别把两条线及填充延伸到第一个和第二个锚点之外的窗格边缘。放置时，第一次点击后预览基线，第二次点击后预览整个通道。
- `price_channel` 是 KLineChart 的价格通道：经过前两个锚点的基线为中心线，第二条线平行于它并经过第三个锚点，第三条线则在基线另一侧与第二条线镜像对称。它默认启用 `extend_left` 和 `extend_right` 且无填充（`fill_enabled: true` 会为整个带状区域着色），并且没有中间线。
- 线条之间的填充默认开启（`fill_enabled`）；`fill_color` 默认为描边颜色，alpha 为 20%。线条相交处（平顶/平底、不相交通道），填充在交点处汇合。线条是主体目标；填充仅在绘图被选中时才是可拖动表面，与矩形相同。`text` 标签像趋势线的标签一样沿基线排布。通道线忽略 `stroke_start` 和 `stroke_end`。
- `tool_options.channel.middle_line` 在两条线正中间绘制一条 1 px 的虚线（平行通道默认开启，其他通道默认关闭），颜色为 `middle_color`（`""` 表示跟随 `color`）。
- `regression_trend` 放置两个用于选定柱范围的锚点（位置取整，含两端）；其主体和手柄仅沿时间方向移动。引擎对这些柱上的源系列拟合一条最小二乘直线——源系列是添加到该绘图所在窗格和价格比例尺上、仍然有效的第一个普通系列（指标输出和自定义系列绝不符合条件，足迹图系列和 feature 系列则通过其 OHLC 投影符合条件；重新排序或隐藏系列不会改变源，移除并重新添加其他系列也不会）——并以虚线绘制该直线（`middle_line`、`middle_color`），在其上下分别绘制与之相距 `upper_deviation`（默认 2）和 `lower_deviation`（默认 -2）个残差标准差的线（样本标准差，`n − 1`），二者分别由 `use_upper_deviation` 和 `use_lower_deviation` 开关控制，两线之间的区域被填充；除非 `show_pearsons` 为 false，否则在起点下方显示 Pearson's R（柱位置与数值之间的带符号相关系数，四位小数）。`source` 选择柱的取值（`indicator_input_source`，默认 `"close"`）。这些线跟随源的流式更新；替换最新一根柱或追加柱，其开销只与发生变化的行相关，而与锚定的范围无关。在 as-of（`time_alignment: "as_of"`）源上，拟合对范围内源自身的每根柱只读取一次，而不是重复的坐标轴点。没有源柱的范围会绘制锚点之间的虚线段。锚点的价格会被存储，但不会影响这些线的形状，`text` 标签位于锚点的框内。默认宽度为 1；其他通道默认为 2。
- 手柄位于已绘制的线条上，按锚点顺序每个锚点一个（`drawing_handle_count` 为 3 或 2）：基线的两个端点，对应第三个锚点的第二条线的中点（拖动或微移它会移动第二条线），以及回归线的两个端点，二者跟随拟合结果（放置过程中同样如此）。按住 Shift 拖动基线端点会像趋势线那样把基线拉直。
- 来自 `drawing_template()` 的模板在应用时会替换绘图的 `tool_options.channel`，因此模板中保持默认值的选项也会被重置。
- `drawing_kind_options()` 对三种通过点击放置的通道返回 `{ kind: "channel", middle_line, middle_color }`，并返回带有所有解析后回归选项的 `{ kind: "regression_trend", ... }`。`tool_options.channel` 仅存储已设置的字段。
<!-- B8: channels — end -->
<!-- B8: fibonacci — begin -->
### 斐波那契

| 工具 | 锚点 | 几何 |
| --- | --- | --- |
| `fib_retracement` | 2 | 锚点价格之间的水平档位：0 档在第二个锚点上，1 档在第一个锚点上，其外为延伸档位。档位横跨锚点的时间。 |
| `trend_based_fib_extension` | 3 | 第一段走势的幅度从第三个锚点起投影（0 档在第三个锚点上，1 档距其一整段走势幅度），宽度为第一段的宽度，自第三个锚点起算。 |
| `fib_channel` | 3 | 与第一段平行的线；1 档经过第三个锚点。 |
| `fib_time_zone` | 2 | 全高线条，位于距第一个锚点为锚点时间间距 0、1、2、3、5、8、13、21、34、55、89 倍的位置。 |
| `trend_based_fib_time` | 3 | 全高线条，位于距第三个锚点为第一段持续时间的各比率倍数处。 |
| `fib_speed_resistance_fan` | 2 | 从第一个锚点出发的射线：按每个价格比率穿过第二个锚点的时间，按每个时间比率穿过其价格，延伸至窗格边缘，另加锚点框内的比率网格。 |
| `fib_speed_resistance_arcs` | 2 | 以第一个锚点为中心的圆弧，半径为比率 × 锚点间的屏幕距离，位于第二个锚点一侧（可选完整圆）。 |
| `fib_circles` | 2 | 以锚点中点为圆心的圆；1 档经过两个锚点。 |
| `fib_spiral` | 2 | 以第一个锚点为中心、经过第二个锚点的黄金螺旋线，每四分之一圈按 φ 倍增长，在屏幕上呈顺时针。 |
| `fib_wedge` | 3 | 以第一个锚点为中心、位于指向第二个和第三个锚点的两条边之间的比率圆弧；1 档位于第二个锚点的距离处。 |

- 档位列表是通用的 `levels` 选项：`value`、`color`、`visible`、`style`（`"solid"`、`"dotted"`、`"dashed"`）、`fill_between`、可选的 `fill_color` 以及 `label_visible`，至多 64 个档位。回撤、延伸和通道默认使用 TradingView 可见的回撤档位 0、0.236、0.382、0.5、0.618、0.786、1、1.618、2.618、3.618、4.236 及其调色板（0 和 1 为灰色，0.236 为红色，0.382 为橙色，0.5 为绿色，0.618 为青绿色，0.786 为青色，1.618 为蓝色，2.618 为红色，3.618 为紫色，4.236 为粉色）。其他工具使用惯例表：基于趋势的时间为 0、0.382、0.5、0.618、1、1.382、1.618、2、2.382、2.618、3；扇形线为 0、0.25、0.382、0.5、0.618、0.75、1；圆弧和圆为 0.236 至 4.236；楔形为 0.236 至 1。取自回撤表的值保留其颜色；其他值按列表顺序取用调色板。螺旋线没有档位。
- 可见档位按值排序。`fill_enabled` 是背景开关（除时间区和螺旋线外默认开启）；当上方档位的 `fill_between` 开启时，两个相邻档位之间的带取上方档位的 `fill_color`，或其颜色的 20% 不透明度。带仅在绘图处于选中状态时才能选中并拖动该绘图；档位线、趋势线和标签则始终可以。
- 绘图自身的 `color`、`width` 和 `style`（默认 `#787b86`、1 px、虚线）用于穿过锚点的趋势线、扇形线的网格以及楔形的边（实线）；螺旋线也用它们绘制。档位线使用各档位自己的颜色和样式，宽度取绘图的宽度。
- `extend_left` 和 `extend_right` 把回撤、延伸和通道的档位延伸到窗格的左右边缘。
- 档位标签显示值，对于回撤和延伸还显示经绘图比例尺格式化器格式化后的价格，例如 `0.618 (102.53)`，颜色与该档位相同。
- `tool_options.fibonacci`（`fibonacci_tool_options`）：`reverse`（交换 0 档和 1 档所在的两端；时间区向后投影；螺旋线逆时针旋转）、`show_levels`、`show_prices`、`levels_as_percent`（`61.8%`）、`log_scale`（价格档位在对数空间中插值）、`trend_line`、`grid`（扇形线）、`full_circles`（圆弧）、`label_h_align` 和 `label_v_align`（价格档位默认为 `"left"`/`"middle"`：位于左端之外、在线上居中；时间档位默认为 `"right"`/`"bottom"`）。每个工具的 schema 只列出它读取的字段及其解析后的默认值。`drawing_kind_options()` 返回 `{ kind: "fibonacci", ... }`，其中每个字段均已解析。
- 剔除遵循已绘制的档位：当锚点被滚动到视口之外时，位于锚点之外的价格档位和时间档位仍使绘图保持可见且可命中。扇形线、圆弧、圆、螺旋线和楔形取决于其锚点之间的屏幕距离，因此仅按窗格进行剔除。
<!-- B8: fibonacci — end -->
<!-- B8: pitchforks_gann — begin -->
### 叉形线与江恩

- `andrews_pitchfork`、`schiff_pitchfork`、`modified_schiff_pitchfork` 与 `inside_pitchfork` 放置三个锚点：枢轴点，然后是叉柄的两端。中线从枢轴点出发（Schiff：位于枢轴点的时间，价格取前两个锚点价格的中间；modified Schiff 与 inside：位于前两个锚点的中点）。它穿过叉柄的中点；对 inside 叉形线则穿过第三个锚点，其叉齿穿过第二个锚点及其关于第三个锚点的反射点。点击之间，一条引导线连接已放置的锚点与指针。除每个锚点各有一个手柄外，叉形线与叉形扇在第二与第三个锚点之间的中点还有第四个手柄，它同时移动这两个锚点（`drawing_handle_count` 4）。
- 叉形线的 `levels` 是以半叉柄宽度为单位的中线偏移：层级 `v` 是中线每一侧的一条叉齿，层级 1 穿过叉柄的两端。默认值：0.25、0.382、0.5、0.618、0.75、1、1.5、1.75 与 2，其中 0.5 与 1 可见；`fill_between` 区域按层级颜色的 20% 填充（`fill_enabled`）；层级标签关闭；中线 `color` 为 `#f23645`；宽度 1。未延伸的线越过叉柄延伸一个中线长度；`extend_left` 与 `extend_right` 使每条线延伸到窗格边缘。枢轴偏移变体在前两个锚点之间增加一条虚线引导线。与矩形内部一样，本族中每个工具的区域填充（叉形线与扇形区域、江恩框区域、方图弧线）仅在绘图被选中时才是拖动目标。
- `pitchfan` 放置三个锚点，绘制相同的层级，表现为从第一个锚点出发、穿过另外两个锚点之间叉柄上各层级点的射线。
- `gann_box` 放置两个角点。其 `levels` 是水平价格层级，`tool_options.gann.time_levels` 是垂直时间层级，二者均表示为相对于从第一个角点起算的框的分数（默认 0、0.25、0.382、0.5、0.618、0.75 与 1，四条边上均填充并标注）。`show_angles` 添加从枢轴角点出发的 `angles` 扇形。它有八个边界手柄，Shift 使其在屏幕上成为正方形。
- `gann_square` 放置两个角点，并绘制 `levels` 网格（默认为每边的五等分）、`angles` 扇形（1×8 到 8×1，1×1 位于对角线上）、围绕枢轴角点的四分之一 `arcs` 弧线（边长的五等分，已填充），以及一个显示价格范围、柱数与每柱价格的框。与江恩框一样，它有八个边界手柄，Shift 使其在屏幕上成为正方形。`gann_square_fixed` 放置一个锚点：方形宽 `size_bars`、价格方向高 `size_bars × scale_ratio`，没有比例时则在屏幕上为正方形。它的第二个手柄，即远端角点，用于调整其大小：角点所在的柱以整柱数设置 `size_bars`（至少为 1），将其拖到锚点下方会设置 `reverse`，有 `scale_ratio` 时角点的价格设置该比例（Shift 保持比例不变）；没有比例时，方形在屏幕上保持正方形，并跟随角点到锚点的较大距离。用键盘微调该角点时，边沿箭头所指方向至少移动一整柱（没有比例时，按箭头所在的轴确定大小），因此无论一根柱有多宽，反复按键都会持续调整大小。
- `gann_fan` 放置两个锚点。其 `levels` 是 1×1 斜率的倍数（默认 1/8、1/4、1/3、1/2、1、2、3、4 与 8，标注为 `8x1` 到 `1x8`，区域已填充）。1×1 线穿过第二个锚点，或在设置 `scale_ratio` 时每柱上升相应数量的价格单位。线默认为射线（`extend_right`）；未延伸时，它们止于锚点所围的框。Shift 将第二个锚点校正为 45°。
- `tool_options.gann`（缺省字段保持其默认值）：

  | 字段 | 工具 | 默认值 |
  | --- | --- | --- |
  | `time_levels` | 江恩框 | 如上，0 … 1 |
  | `angles` | 江恩框（配合 `show_angles`）、江恩方图 | 1/8 … 8，正值 |
  | `arcs` | 江恩方图 | 0.2、0.4、0.6、0.8、1，正值 |
  | `reverse` | 江恩框、江恩方图 | `false`；`true` 从第二个锚点的价格起量（江恩框也从它起计时间），并使固定方形向下增长 |
  | `show_angles` | 江恩框 | `false` |
  | `show_stats` | 江恩方图 | `true` |
  | `scale_ratio` | 江恩扇形、固定方形 | `null`：1×1 的每柱价格，正值 |
  | `size_bars` | 固定方形 | 20，范围 1 到 100000 |

  每个工具的 schema 仅列出它使用的字段。`drawing_kind_options()` 为叉形线与叉形扇返回 `{ kind: "pitchfork", levels }`，为江恩工具返回 `{ kind: "gann", levels, time_levels, angles, arcs, reverse, show_angles, show_stats, scale_ratio, size_bars }`。
<!-- B8: pitchforks_gann — end -->
<!-- B8: projection_annotations — begin -->
### 投影与标注

默认值遵循专业平台的常规外观：绘图的 `color`（除非另有说明，即规范主色）绘制标记、引线与框背景；框内文字为 `text_color`，或与框形成黑/白对比；`box_border_color` 为标注框描边；文字使用图表字号，除非设置了 `text_size`。每个工具各自拥有其 `text`（它不遵循其他工具的 3×3 框标签），并将任何可见的 `labels` 渲染为引擎格式化的统计信息。其中八个工具不绘制文字：其 `text` 会被接受并保留，但从不显示，也不能就地编辑。

- `forecast`（源点、目标点）：一条线段，端帽取自 `stroke_start`/`stroke_end`，在远侧有一个源点圆点和一个源价格框，另有一个目标框，显示变化量与百分比、目标时间和结果。结果取自该绘图的源系列（与回归趋势的规则相同：添加到其窗格与价格比例尺上的第一个有效的普通系列），并跟随其流式更新：当源柱之后的某根柱不晚于目标柱到达目标价格（上升目标取最高价，下降目标取最低价）时，结果为 `Success`（上涨色框）——此处的柱指 as-of 源自身的柱，包括在两个坐标轴点之间折叠的柱；重复源柱的点不是后续柱；当目标柱之后已存在已成交的柱而仍未满足上述条件时，结果为 `Failure`（下跌色框）（位于最新的、可能仍在形成中的柱上的目标保持待定，空白数据行（例如未来的交易时段槽位）不是柱）；待定期间没有结果（使用绘图颜色）。
- `bars_pattern`（两个锚点）：用已激活的工具放置时（该工具会就地预览副本），或由不带 `bars` 的 `add_drawing` 创建时，它会把其锚点柱索引之间的柱（至多 128 根；更长的范围聚合为 128 个 OHLC 桶）复制到 `tool_options.projection_annotation.bars`，并把其锚点固定在副本的框上——第一根被复制的柱位于副本的最高值，最后一根位于最低值——使幽灵图形恰好从其源的位置开始。幽灵图形始终填满其锚点之间的框：移动它会移动副本，锚点则在时间上拉伸它，并按副本完整范围的比例缩放它（价格基准重新缩放会精确地缩放它）。`bars_mode` 为 `"hl_bars"`（默认）、`"oc_bars"`、`"line_open"`、`"line_high"`、`"line_low"` 或 `"line_close"`；`mirrored` 在时间上反转副本，`flipped` 使其在框内上下翻转。粘贴、同步与持久化都会携带被复制的柱；命名模板只保留样式，因此应用模板绝不会替换形态的副本，已激活的工具始终复制其自身的范围。对于 as-of 源，它复制该源自身的柱，每根一次。在任何数据存在之前创建的形态没有副本，绘制一个虚线框。
- `price_range`、`date_range`、`date_and_price_range`（两个锚点）：锚点之间的填充（`fill_enabled` 默认开启；`fill_color`，或绘图颜色的 20%），被测量坐标轴的边缘线，穿过中部、指向第二个锚点的带箭头测量线（`stroke_end` 默认为 `"arrow"`），以及位于被测量一端之外的统计框（日期范围则在其下方）。默认 `labels`：价格变化、百分比变化与 tick 数；柱数与持续时间；或全部五项。锚点吸附到整柱与价格 tick（拖动、键盘微调或移动主体时同样如此）；tick 按品种 tick 或价格带阶梯计数，回退到比例尺的 `min_move`。`"date_price_range"`，即早期构建的写法，会被读作 `"date_and_price_range"`，且从不写出。Shift 点击的快速测量会绘制一个临时的日期与价格范围。
- `projection`（顶点、半径点、价格点）：以顶点为中心的圆形扇区，从穿过半径点的射线到穿过价格点的射线（取较短的转向），已填充（`fill_enabled` 默认开启）并描边。可见的 `labels` 度量从顶点到价格点。放置时，第一次点击将顶点显示为一个手柄，并带一条指向指针的临时线；第二次点击后，扇区通过指针预览，第三次点击提交它。
- `anchored_text`（一个锚点）：固定在窗格位置上的文字。其锚点是窗格分数——`logical` 为 x / 窗格宽度，`price` 为从窗格顶部起的 y / 窗格高度——因此在图表滚动、缩放、重新缩放或切换周期时它保持不动。其锚点不带 `time`（`time` 输入被忽略，仅含时间的锚点被拒绝），分数被钳制到 `0..=1`（在 `add_drawing`、`set_points`、粘贴、同步与拖动中，因此它始终可触及），而含有 `0..=1` 之外分数的 `import_state` 文档则为 `invalid_data`，并且磁吸、粘贴偏移与成组移动均不适用。默认文字为 `"Text"`，在锚点处按左/上对齐；`box_color`/`box_border_color` 如同文字工具一样为其加框。
- `note`（一个锚点）：一枚图钉，其尖端即锚点，文字（默认 `"Note"`）显示在钉头旁的框中。该框在备注被悬停、被选中或正在编辑时显示；`tool_options.projection_annotation.always_show_text`（默认 `false`）使其保持可见。
- `price_note`（两个锚点）：一条从定价点引向第二个锚点处的框的引线，框内显示其价格（以及任何文字）。
- `callout`（尖端、框）：一个文本框（默认 `"Callout"`），按 `text_h_align`/`text_v_align`（默认居中）放置在第二个锚点上，带有指向第一个锚点的指针。
- `comment` 与 `price_label`（一个锚点）：一个对话气泡，其尾部尖端即锚点；评论显示其文字（默认 `"Comment"`），价格标签显示锚点的价格及任何文字。
- `signpost`（一个锚点）：一根从锚点向上伸出的杆，顶端是文字牌（默认 `"Signpost"`）。
- `flag_mark`（一个锚点）：一面立在锚点上的旗帜。
- `arrow_mark_up`、`arrow_mark_down`、`arrow_mark_left`、`arrow_mark_right`（一个锚点）：一个块状箭头，其尖端即锚点，任何文字位于箭尾之后，使用 `text_color` 或箭头颜色。向上默认为上涨色，向下默认为下跌色。
- `simple_tag`（一个锚点）：KLineChart 的 simple tag：一条在锚点价格处横贯整个窗格的虚线，并在价格轴上加标签。绘图有 `text` 时标签显示该文字，否则显示价格；文字不绘制在图表上，因此没有就地编辑器（通过选项中的 `text` 设置）。
- `simple_annotation`（一个锚点）：KLineChart 的 simple annotation：一根虚线杆从锚点升起至一个小头部，`text` 位于头部上方的框中（初始为空）。放置它会打开编辑器。
- `icon`（一个锚点）：`tool_options.projection_annotation.icon`——`"star"`（默认）、`"heart"`、`"check"`、`"cross"`、`"circle"`、`"square"`、`"diamond"`、`"triangle_up"` 或 `"triangle_down"`——以锚点为中心，`icon_size` 为宽度（单位 CSS px，8..128，默认 24），使用绘图颜色。
- 各类默认值（填充、箭头、统计、默认文字、箭头颜色）是 schema 默认值，持久化时省略；被清空的默认文字持久化为 `""`。
- `anchored_text`、`note`、`price_note`、`callout`、`comment`、`price_label`、`signpost`、`simple_annotation` 与箭头标记的文字可就地编辑（参见上文的就地文字编辑）；价格备注与价格标签将其价格行保留在文字上方。被清空的框在编辑期间保留一个插入符行。放置 `anchored_text`、`note`、`callout`、`comment`、`signpost` 或 `simple_annotation` 会打开编辑器（在工具带有默认文字时，对默认文字进行编辑）；放置 `price_note`、`price_label` 或箭头标记则不会。
- `drawing_kind_options()` 为本族的每个工具返回 `{ kind: "projection_annotation", bars_mode, mirrored, flipped, pattern_bars, icon, icon_size, always_show_text }`。
<!-- B8: projection_annotations — end -->
<!-- B8: patterns_elliott_cycles — begin -->
### 形态、艾略特波浪与周期

每个工具通过点击放置固定数量的锚点，在放置过程中预览已放置的各段，并为每个锚点提供一个手柄。默认值遵循 TradingView：各工具各有颜色，宽度为 2（循环线为 1），且在 `fill_color` 为空时，区域填充为绘图颜色的 15%。点标签与比例标签使用绘图的字号、字重与斜体（`text_color` 会覆盖其对比色默认值）；它们是主体目标。

| 工具 | 锚点 | 默认颜色 | 绘制内容 |
| --- | --- | --- | --- |
| `xabcd_pattern` | X, A, B, C, D | `#2962FF` | 各段、带阴影的 XAB 与 BCD、比例 AB/XA、BC/AB、CD/BC、AD/XA |
| `cypher_pattern` | X, A, B, C, D | `#2962FF` | 各段、带阴影的 XAB 与 BCD、比例 AB/XA、XC/XA、CD/XC |
| `abcd_pattern` | A, B, C, D | `#089981` | 各段、比例 BC/AB 与 CD/BC |
| `head_and_shoulders` | 基点、左肩、颈点、头部、颈点、右肩、基点 | `#089981` | 各段、位于外侧两段之间的颈线、带阴影的双肩与头部、各部位标签 |
| `triangle_pattern` | A、B、C、D（高点与低点交替） | `#673AB7` | 各段、A–C 与 B–D 两边（当其顶点位于前方且在一个形态宽度之内时延伸至顶点）、带阴影的三角形 |
| `three_drives_pattern` | 起点、驱动 1、回撤、驱动 2、回撤、驱动 3、终点 | `#673AB7` | 各段、标注为 1–3 的驱动段、每段相对于其前一段的比例 |
| `elliott_impulse_wave` | 0, 1, 2, 3, 4, 5 | `#3D85C6` | 各浪标注为 1–5 |
| `elliott_correction_wave` | 0, A, B, C | `#3D85C6` | 各浪标注为 A–C |
| `elliott_triangle_wave` | 0, A, B, C, D, E | `#FF9800` | 各浪标注为 A–E |
| `elliott_double_combo` | 0, W, X, Y | `#6AA84F` | 各浪标注为 W、X、Y |
| `elliott_triple_combo` | 0, W, X, Y, X, Z | `#6AA84F` | 各浪标注为 W、X、Y、X、Z |
| `cyclic_lines` | 周期起点、周期终点 | `#80CCDB` | 虚线连接线，以及从较早的锚点起至右边缘、每隔一个间隔绘制一条的全高垂直线 |
| `time_cycles` | 周期起点（底边）、周期终点（决定拱高） | `#159980` | 宽度与高度同锚点的半椭圆拱，向两侧重复，带阴影 |
| `sine_line` | 一个波峰或波谷、下一个相反的极值 | `#159980` | 穿过两个锚点并贯穿窗格的正弦曲线 |

- 比例为价格比例，以三位小数印在虚线连接线上；`tool_options.pattern.show_ratios: false` 隐藏连接线与比例。
- `tool_options.pattern.degree` 选择艾略特波浪级别：`"supermillennium"`、`"millennium"`、`"submillennium"`、`"grand_supercycle"`、`"supercycle"`、`"cycle"`、`"primary"`、`"intermediate"`（默认）、`"minor"`、`"minute"`、`"minuette"` 或 `"subminuette"`。第 3 浪与 C 浪依次显示为 `{III}`/`{c}`、`[III]`/`[c]`、`<III>`/`<c>`、带圈的 `III`/`c`、`(III)`/`(c)`、`III`/`c`、带圈的 `3`/`C`、`(3)`/`(C)`、`3`/`C`、带圈的 `iii`/`c`、`(iii)`/`(c)` 与 `iii`/`c`。`tool_options.pattern.show_wave: false` 则只保留标签。
- 间隔小于 3 CSS px 的周期重复会折叠为定义周期。
- `fill_enabled` 与 `fill_color` 为 XABCD、cypher、头肩形、三角形形态与时间周期着色；区域填充仅在绘图被选中时才是主体目标。`extend_left`、`extend_right`、`stroke_start` 与 `stroke_end` 不适用于这些工具，而填充不适用于其他工具。`text` 标签相对于锚点所围的框放置。
- `drawing_kind_options()` 为 XABCD、cypher、ABCD 与三驱动返回 `{ kind: "pattern", show_ratios }`，为艾略特工具返回 `{ kind: "elliott_wave", degree, show_wave }`，为头肩形、三角形形态与周期工具返回 `{ kind: "generic" }`。
<!-- B8: patterns_elliott_cycles — end -->
<!-- B8: shapes — begin -->
### 形状

- `rotated_rectangle` 放置三个锚点：两条短边的中点，然后是长边上的一点，其到该轴的距离决定宽度。它在任何缩放下在屏幕上都保持直角。其手柄是两个轴端点，以及位于每条长边中点的宽度手柄（`drawing_handle_count` 4）；第三个锚点没有自己的手柄。宽度手柄将宽度设置为其到轴的距离，拖动轴端点则在屏幕上保持宽度不变，因此旋转矩形绝不会将其压扁。宽度拖动之后，第三个锚点位于其所在长边的中点。
- `ellipse` 放置两个框角点，内接于它们的框；它用矩形的八个手柄编辑，Shift 使框保持正方形（即圆）。`circle` 放置其圆心与圆周上的一点。`triangle` 放置三个顶点。
- `arc` 放置其起点、终点与它经过的一点（共线的点得到直线弦）。`curve` 放置其起点、终点与它在中间经过的点；`double_curve` 放置其起点、终点与它在三分之一和三分之二处经过的点。每个手柄都位于曲线上，`extend_left`/`extend_right` 将曲线端点的切线延续到窗格边缘。放置三锚点或四锚点形状时，到目前为止已点击的锚点与指针以绘图描边的折线显示，直到除最后一个之外的所有锚点都已放置；之后形状本身通过指针预览，直到最后一次点击提交它。
- `polyline` 像 `path` 一样放置顶点：点击添加，双击或 Enter 结束，Backspace 删除最新的顶点，Escape 取消。放置三个顶点后，再次点击第一个顶点会以闭合方式结束折线（指针位于其上方时，预览会合拢）。`tool_options.shape.closed`（默认 `false`）将最后一个顶点与第一个顶点相连，并按非零规则填充所围区域。填充是有界工作：顶点数超过 2,048 的闭合折线，或自交程度严重到其填充超出三角剖分界限的闭合折线，只绘制其轮廓，不填充，也没有内部选择目标（其描边仍可选中它）。这不是错误，所有顶点都会保留，并且在每个后端上完全一致；跟随数千根图表数据柱的区域应属于系列，而不是绘图折线。
- `highlighter` 与 `brush` 一样是自由手绘拖动：一条 20 px 的记号笔笔画，40% 琥珀色，圆形端点。在每个后端上，它在自身重叠处都保持单一不透明度，并忽略 `style`、端帽与填充。
- 描边默认为 2 px。旋转矩形、椭圆、圆、三角形、圆弧与折线将 `fill_enabled` 默认为 `true`，并以 `fill_color` 填充，未设置时使用描边颜色的 20% 不透明度；圆弧填充圆弧与其弦之间的弓形，曲线一旦启用即填充曲线与其弦之间的区域，开放折线从不填充。形状的填充仅在其被选中时才可选中并拖动它，因此未选中形状的内部仍会平移图表。端帽（`stroke_start`、`stroke_end`）适用于开放形状：圆弧、曲线与开放折线。
- 框文字（`text`）相对于形状自身的框对齐，例如圆自身的框，而不是其圆心与圆周锚点所围的框。
- `drawing_kind_options()` 为形状族的每个工具返回 `{ kind: "shape", closed }`；`tool_options.shape.closed` schema 描述符仅为 `polyline` 列出。
<!-- B8: shapes — end -->

### KLineChart overlay 的等价项

从 KLineChart 迁移的宿主可在此找到其每个绘图 overlay。其中七个是独立的工具（wire id 38..=41、52、147 与 148）；其余则是带选项的现有工具，表格说明了具体做法。

| KLineChart overlay | Aeris 工具 |
|---|---|
| `straightLine` | `extended_line` |
| `rayLine` | `ray` |
| `horizontalSegment` | `horizontal_segment` |
| `verticalRayLine` | `vertical_ray` |
| `verticalSegment` | `vertical_segment` |
| `parallelStraightLine` | `parallel_channel`，设置 `extend_left` 与 `extend_right`、`fill_enabled: false` 以及 `tool_options.channel.middle_line: false` |
| `priceChannelLine` | `price_channel` |
| `fibonacciLine` | `fib_retracement`，设置 `extend_left` 与 `extend_right`（层级横跨窗格） |
| `priceLine` | `price_line` |
| `simpleTag` | `simple_tag` |
| `simpleAnnotation` | `simple_annotation` |

## 持久化 V1

持久化 schema 的版本控制独立于 npm 包版本。V1 仅包含：

- 有序的窗格标识、拉伸系数与空窗格保留标志；
- 有序的内置绘图，包含持久 ID、种类、窗格引用、语义锚点（`{logical, price, time?}`，在非时间柱图表上另加 `anchor_times_micros` 附带数据）以及样式；
- 可选的顶层 `drawing_price_basis` 标签（宿主定义的绘图价格的价格基准）。

宿主的行情历史、系列与指标定义、图表选项、交易持仓/订单/成交/预览/意图、预警线/创建请求、自定义扩展、回调、订阅、选择、交互会话、代次、细节层级（LOD）、绘图边界/索引、保留的帧以及 GPU 资源均不持久化。宿主将 V1 恢复到一个全新的图表中，然后重新安装宿主拥有的数据、系列/指标配置、交易状态、预警状态、选项与扩展。

命名价格比例尺描述符与系列到比例尺的绑定同样由宿主拥有。宿主在恢复对比系列绑定之前，先在每个窗格中重建命名比例尺；图表状态 V1 保持不变。

导入会在变更之前检查整个文档，并作为一个事务安装。被导入的窗格获得全新的实时句柄 ID，同时保留各自独立的持久窗格 ID；因此导入前的窗格句柄与价格比例尺句柄会变为过期。仅当绘图 ID 尚未发放，且图表仍保持其初始窗格拓扑时，才接受导入。这可防止旧的绘图句柄重新指向具有相同持久 ID 的已恢复绘图。

绘图锚点以 `{logical, price, time?}` 的形式存储，另有可选的顶层 `drawing_price_basis` 标签；这两个字段均为可选，无需变更 schema 版本。在普通时间图表上，恢复时以 `time` 为准：在数据之后导入时，按每个锚点的时间在已加载窗口上解析锚点；在数据之前导入（网格工作区的顺序）时，锚点按时间保持待定状态，直到宿主安装数据。没有锚点时间的文档保留其逻辑锚点。非时间（tick/成交量/range）图表继续使用 `anchor_times_micros`。

绘图样式字段均为可选，省略时恢复该类型自身的默认值；导出仅在字段与默认值不同时才写入该字段。B8 绘图族选项存放在可选的 `style.tool_options` 对象中（序列化后至多 16 KiB），因此旧文档无需迁移。

不受信任输入的限制为：每个文档 8 MiB、64 个窗格、10,000 个绘图、每个绘图 100,000 个锚点、总计 250,000 个锚点、每个绘图 64 KiB 文本，以及绘图文本总计 1 MiB。未知的可选 V1 字段会被忽略。未知的 schema 版本、绘图类型、窗格引用、重复 ID、无效的锚点数量、非有限/不安全的数字以及超出限制的情况，均会在结构层面失败，并使图表保持不变。V1 夹具是兼容性输入；未来版本必须为文档中记载的兼容窗口保留明确的 V1 迁移路径。

## 持久化 V3 研究

带有引擎持有指标的金融图表导出 schema 版本 3。V3 仍由宿主持有市场历史与普通系列数据，但会持久化有序的研究绑定、标量输入选择、类型化指标参数（包括显式的 seed、histogram 与 estimator 取值）、按时间戳对齐的成交量与成交额来源引用，以及各输出的样式。链式来源被编码为对较早研究输出的引用，因此恢复不依赖旧的实时系列标识。宿主必须在导入 V3 之前重新创建来源系列及其数据；导入会在变更之前校验每一项依赖、参数、输出样式数量和资源限制。V1 和 V2 文档仍被原样接受，且 V3 文档只能恢复到全新的金融图表中。

## 版本策略

浏览器包版本低于 1.0 期间：

- patch：兼容的正确性、安全、性能、文档与打包修复；
- minor：新增稳定 API、经明确评审的实验性 API 变更，或兼容的行为新增；
- major（包括最终的 1.0 边界）：移除/重命名/更改稳定 API 的签名、不兼容的稳定行为，或终止文档中记载的持久化兼容窗口。

新增绘图/系列类型通常属于 minor 级别的包功能，但改变既有类型的含义则不兼容。新增持久化 schema 不会使 V1 失效；移除 V1 支持遵循另行记载的持久化窗口，属于 major 级别的兼容性事件。

## Rust 分发

Rust crate 仅限仓库内使用（`publish = false`）；不会向 crates.io 发布任何内容，浏览器包是唯一发布的产物。Aeris Terminal 等宿主通过固定的 Git 修订版本或本地路径使用 `aeris_charts_*` crate。Rust API 低于 1.0，可能在任何修订中变更，因此宿主在更换其固定修订时应查阅下面的说明。

`aeris_charts_render_gpui` 是实验性的。它以精确版本要求固定 `gpui-pre` 0.3.7，即 gpui-kit 0.7.0 所依赖的 GPUI 快照，因此绘制图表的宿主必须使用同一个 `gpui`（宿主若使用其他 GPUI 构建，例如 Zed 的某个 Git 修订版本，就会持有其类型的两份不兼容副本）。GPUI 升级是显式的 manifest 与 lockfile 变更。

在 macOS 上，宿主必须启用 `font-kit` feature 来构建其 GPUI 平台 crate（`gpui-pre-platform`，或直接构建 `gpui-pre-macos`），该 feature 即 GPUI 的 macOS 文本系统。否则 GPUI 会改用空操作的文本系统：图表不绘制任何文本（坐标轴、标签、图例、绘图文本），并将每个字符串的宽度度量为零，唯一的信号是启动时的一条 `log::warn!`。Linux 和 Windows 不受影响。

通过 `GpuiChartInput::scroll_wheel` 绑定滚轮事件的宿主可获得与浏览器等效的滚动。固定在较早修订的宿主在水平滚轮或触控板横扫时，平移时间比例尺的方向相反：适配器未翻转就透传了 GPUI 的水平增量，而 GPUI 报告的是内容运动（正值表示露出左侧），引擎则与浏览器一样，将正值视为向右移动。适配器现在会翻转水平轴；自行补偿了旧符号的宿主必须移除其补偿。

### 更换固定修订

以下说明列出了更换固定修订所带来的宿主可见变更，以便调用点只需评审一次。每个分组都会指明修订级别的变更及其影响的调用点。

每个分组都以承载其变更的提交为键。分组适用于不包含该提交的固定修订（`git merge-base --is-ancestor <commit> <pin>` 以非零状态退出）；已包含该提交的固定修订已经采纳了该变更。哈希为本仓库的哈希。“上游”标记来自 `AerisTerminal/aeris-charts` main 并在此保留其哈希的提交，因为上游是通过合并引入的，从未变基；“自有线”标记仅存在于本仓库的提交。两条历史在 `ed2910d` 处分叉，旧写法仅存在于某一侧的分组会注明是哪一侧。

**子窗格坐标**（自有线，来自 `84b85e6 fix(engine): sub-pane crosshair sync, chart-level pane selection, coordinate contract`；参见[坐标与窗格](#坐标与窗格)）。三项行为发生了变更：

- 图表级的 `price_to_coordinate` 和 `coordinate_to_price` 不再跟随按创建顺序排列的第一个可见系列。价格在窗格 0 的默认比例尺上换算，坐标则在包含 `y` 的窗格的默认比例尺上换算，因此依赖于叠加系列先创建、或主系列被隐藏的调用必须改用该系列自己的句柄。主系列最先创建的单窗格图表不受影响，系列句柄的换算从未改变。
- 下方窗格（`pane_index` 为 1 或更大）上的联动十字光标现在可以往返一致。较早的修订将窗格偏移量应用了两次并读取了错误的比例尺，因此 `crosshair_sync_position` 与 `apply_external_crosshair` 在第一个窗格之外的任何窗格上结果都不一致。同步的价格现在是窗格默认比例尺上的价格，超出范围的价格会把该线保持在其窗格内。
- `ChartEngine::pane_index_at_y`（以及浏览器包中的 `pane_index_at_y`）现在对分隔条返回其上方的窗格，对位于内容上方的 `y` 返回窗格 0，而这两种情况过去都解析为最后一个窗格。为价格坐标轴命中测试选取窗格的 GPUI 宿主会获得修正后的映射。

**成交流派生系列**（自有线，来自 `a51242c fix(engine): guard trade-stream-derived series against host writes`；参见 [Tick 转 K 线](#tick-转-k-线)）。需要评审两项，其中第一项无法在本仓库中检查：

- Terminal 不得向通过 `bind_trade_bar_series_to_stream` 绑定的 K 线，或向 CVD、delta 或成交量系列（`add_cvd_series`、`add_delta_series`、`add_trade_volume_series`）写入。宿主对它们的每一次数据写入现在都会像对足迹图的写入那样被拒绝（`false`、`0`、`None`、`Err(UnsupportedSeriesData)` 或 `Rejected(UnsupportedSeries)`；`series_is_source_owned(id)` 可将该拒绝与未知 id 区分开），且 `apply_momentum_histogram_colors` 对 delta 与成交量研究返回 `false`。浏览器包以 `code: "derived_series"` 拒绝这些写入。
- `FootprintError` 新增 `SeriesOwned(SeriesId)` 变体，因此对它做穷尽 `match` 的代码需要补一个分支。对于已被重采样器、合成柱或某个被转换为 K 线的研究写入的 K 线或柱系列，`bind_trade_bar_series_to_stream` 会返回它（足迹图或标量研究仍得到 `UnsupportedTradeBarSeries`，因为 K 线类型检查先运行）；对于已被成交流、研究、重采样器或合成柱写入的系列，`configure_footprint_series` 会返回它。重采样目标和合成柱系列会拒绝已绑定成交流的 K 线（`ResampleError::UnsupportedTarget`、`SyntheticBarError::UnsupportedSeries`）。

**批量化的周期重置研究线**（自有线，来自 `fafecda perf(render): batch period-reset study segments into one Segments primitive`）。这对穷尽匹配 `aeris_charts_render::draw_list::Prim` 的 Rust 代码（自定义执行器、帧检查器、点池重新定基）构成编译期破坏性变更：

- `Prim` 新增 `Segments { first_point, segment_count, width, color }`，它是对 `points[first_point .. first_point + 2 * segment_count]` 的 `segment_count` 条相互独立的两点描边的批量，每条的描边方式与实线的简单两点 `Polyline` 相同（虚线已展开为每段虚线一对点）。对于日线或更长周期柱上的交易时段 VWAP、VWAP 带和枢轴线，引擎会发出它来取代每根柱一条的两点 `Polyline`，因此能通过编译的 `_ => {}` 分支会悄悄停止绘制这些研究。请从 `draw_list::segment_points` 获取点对窗口（超出点池的范围会使该图元被丢弃），并在重新定基某一层时，将 `first_point` 与其他所有点池索引一并移动。本仓库中的 Canvas2D、WebGPU、GPUI 和原生执行器已经处理了它。由于宿主通过更换固定修订来采纳该变更，因此对穷尽匹配的代码而言它是破坏性的。

**命名时区**（两条线，来自合并 `2e7d19f merge: sync with AerisTerminal/aeris-charts main`，该合并将上游的 `f796529 feat(time): add selectable IANA chart time zones` 引入自有线的交易所时间时钟；参见[时间、交易所时区与交易时段](#时间交易所时区与交易时段)）。需要评审的行为：

- `ChartEngine::set_time_zone` 接受来自 `TRADINGVIEW_TIME_ZONES` 的 IANA id（`Result<bool, String>`；已安装时返回 `Ok(false)`）。接受 `UtcOffsetSchedule` 的设置方法是 `set_exchange_offsets`，因此针对 `set_time_zone` 接受 schedule 的修订所编写的调用点必须改用新名称。只有合并之前的自有线固定修订接受 schedule；从 `f796529` 起的上游固定修订已经传入 id。
- 从 `f796529` 起的上游固定修订在 `aeris_charts_core` 中带有感知时区的辅助函数，每个都接受 `ChartTimeZone`：`format_tick_label_with_time_zone`、`format_date_pattern_with_time_zone`、`format_crosshair_time_with_time_zone`、`weight_by_time_in_time_zone` 和 `fill_weights_for_points_in_time_zone`。该合并删除了它们，并保留自有线的 `ExchangeTime` 形式作为唯一的时钟（自有线固定修订已具备这些形式）；它们接受 `&ExchangeTime` 而非时区：`format_tick_label_in`、`format_crosshair_time_in`、`weight_by_time_in` 和 `fill_weights_for_points_in`。用 `ChartEngine::exchange_time()` 读取引擎自己的时钟，或用 `ExchangeTime::new(zone.offset_schedule()?, 0)?` 构建一个。`format_date_pattern` 没有 `ExchangeTime` 形式：请用 `ExchangeTime::local_seconds` 平移时间戳并对结果格式化，`format_crosshair_time_in` 即是如此。只有从 `f796529` 起的上游固定修订具有被删除的辅助函数；自有线固定修订从未具有。
- 命名时区会一次性解析为显式 schedule，因此刻度权重、标签、VWAP 与枢轴的周期键、交易时段和倒计时都与 `set_exchange_offsets` 完全一样地遵循它。该名称还会本地化通用时间轴和 `time_zone_clock_text`；显式 schedule 则不会，此时 `time_zone_id()` 返回 `custom`，而不是 TradingView id。
- 顶层 `timezone` 选项若不是字符串，或其 id 不在一致性列表之内，现在会拒绝整个选项补丁（较早的修订会静默忽略它）。转发 TradingView 占位符（例如 `exchange`）的宿主必须在打补丁之前将其过滤掉。导入已保存的 V2 文档是例外：其选项中无法解析或非字符串的 `timezone`（较早的构建所存储的原始值）会被丢弃，以便布局的其余部分仍能恢复。
- `time_scale_options_json()["time_zone"]` 报告交易所 schedule（`"UTC"` 或转换数组），而不是较早修订在此处输出的 TradingView id；请通过 `time_zone_id()` 读取命名时区。
- V2 文档可以在 `timeScale.timeZone` 旁携带一个新增的 `timezone` 字符串，仅在安装了命名时区时写入。通过 Git 依赖 `aeris_charts_core` 的使用方不会读取本仓库的 `.cargo/config.toml`，因此它会编译完整的 tz 表，而不是本仓库产物所保留的 98 个一致性时区。

**绘图文本编辑**（两条线，来自合并 `2e7d19f merge: sync with AerisTerminal/aeris-charts main`，该合并使上游的 `7518e7e feat(drawings): engine-owned text typing session for every host` 成为唯一的会话，并保留了自有线的布局、命中测试和可编辑工具；参见[绘图锚点、磁吸与价格基准](#绘图锚点磁吸与价格基准)）。每一项都会说明它适用于哪些固定修订：

- 在 main 上，一个引擎会话是唯一的文本编辑状态。用 `begin_drawing_text_edit(id, paint_caret)` 打开它（自行绘制插入符的宿主传 `false`，浏览器即如此）；用 `set_drawing_text_edit(text, caret)` 镜像宿主的可编辑表面；用 `commit_drawing_text_edit()` 或 `cancel_drawing_text_edit()` 结束它；`editing_drawing()` 读取已打开的会话。原生宿主使用 `drawing_text_edit_insert`、`drawing_text_edit_key`、`drawing_text_edit_select_all` 和 `drawing_text_edit_caret_at`。实时文本不记录撤销步骤；提交会记录一步。将键盘事件转发给 `GpuiChartInput::key_down` 的 GPUI 宿主无需调用这些方法：适配器会路由按键，包括平台的按词和按行移动以及剪贴板快捷键（参见下文的引擎输入控制器）。
- `ChartEngine::set_editing_drawing` 已被移除。上游固定修订具有它（与该会话并存），`36c9f09` 之前的自有线固定修订也具有；从 `36c9f09` 起的自有线固定修订则没有。
- 从 `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling` 起直到该合并为止的自有线固定修订使用其自己的三调用会话，该合并已将其移除：`begin_drawing_text_edit(id)`、`set_drawing_edit_text(text)` 和 `end_drawing_text_edit(commit)`。新的调用为 `begin_drawing_text_edit(id, paint_caret)`、`set_drawing_text_edit(text, caret)`（镜像值现在携带插入符），以及分别对应 `commit` 为 true 或 false 的 `commit_drawing_text_edit()` 或 `cancel_drawing_text_edit()`。这不是单纯的重命名：旧调用既不修剪文本，也不移除文本工具，而现在提交会修剪文本并移除留空的文本工具，取消则会移除一开始就为空的文本工具。上游固定修订从未有过三调用形式。
- 包含 `7518e7e` 但不包含 `b75f092 feat(drawings): text-field selection in the drawing typing session` 的上游固定修订具有 `drawing_text_edit_key(key)`。该提交将其改为 `drawing_text_edit_key(key, extend_selection)`（Shift 扩展选择），并为 `DrawingTextEditKey` 增加了变体 `DeleteWordBackward`、`DeleteWordForward`、`WordLeft` 和 `WordRight`，因此这样的固定修订需要加上该参数，并且对按键做穷尽 `match` 的代码还需补上这些分支。自有线固定修订从未有过单参数形式。
- `begin_drawing_text_edit` 接受每一种自行绘制文本的绘图，并拒绝（不影响已打开的会话）已锁定、已隐藏、按周期隐藏或非文本的绘图，以及锚点尚无法换算的绘图。上游的会话只打开文本工具和趋势线，并且即使拒绝也会关闭已打开的会话；`d438dab` 之前的自有线固定修订止步于这些以及绘图族中的绘图。尚未布局的图表没有可用于换算锚点的价格比例尺，因此在第一帧之前就开始会话的宿主会得到 `false`。
- 上游固定修订将文本限制在 256 字节；main 以 `MAX_DRAWING_TEXT_BYTES`（65,536 字节）为界：会超出该界限的插入整体被拒绝，镜像值则在字符边界处被钳制。文本 run 标签保持在一行内；绘图族文本框（`comment`、`callout`、`note`、`signpost`、`anchored_text`）保留换行。原生宿主目前支持点击定位插入符和在文本框中输入，但尚不支持上下行导航。
- `drawing_text_hit_at` 会对每一种绘制文本 run 的工具（线条、通道、斐波那契、形状）的标签作答，而不仅限于趋势线，并会与更上层的绘图主体进行仲裁；上游固定修订和 `d438dab` 之前的自有线固定修订只对趋势线作答。哪一次点击开始输入，对所有宿主是同一条规则：第一次点击仅对文本工具的两步点击和趋势线标签打开输入，而放置会请求编辑器的工具时则在放置时打开；对已选中绘图的双击、Enter 或 F2 会打开每个带文本的绘图的编辑器。浏览器手势层和原生输入控制器都应用该规则；直接驱动引擎 API 的宿主需自行应用。
- 上游的 `3527136 Default rectangle borders off and EMA strokes to one pixel`（合并 `2e7d19f` 之前的自有线固定修订和 `3527136` 之前的上游固定修订都不具备它）使矩形默认无边框（`border_visible: false`）。省略该键的已保存文档导入时边框可见，因此较早的文档保持其外观。

**引擎输入控制器**（上游，来自 `17a591f feat(input): engine-owned interaction controller for every native host`，自有线通过合并 `3eef45e` 引入；设计见 [Architecture.md](Architecture.md#aeris_charts_engine)）。引擎现在拥有指针、滚轮和按键的路由：宿主将平台事件转换为 `PointerInput`、`WheelSample` 和 `ChartKey`，并调用 `ChartEngine::input_*`。按下仲裁、拖动生命周期、点击与双击、键盘绑定、悬停、光标选择和惯性运动都归引擎所有。请评审以下调用点：

- 哪些固定修订具有已移除的手势 API。`begin_financial_drag`、`update_financial_drag`、`update_financial_crosshair`、`end_financial_drag`、`apply_financial_wheel`、`apply_financial_navigation`、`financial_drag` 以及类型 `FinancialDrag` 和 `FinancialNavigation` 仅存在于上游一侧，来自 `f9b052f refactor(engine): own native financial gestures`（`apply_financial_navigation` 和 `FinancialNavigation` 来自 `4aaaf78 refactor(engine): own financial scale commands`），并存在于自有线从合并 `2e7d19f` 起、直到合并 `3eef45e` 引入 `17a591f` 为止的修订上，该提交移除了它们，且没有提供兼容层。`2e7d19f` 之前的自有线固定修订从未有过它们，因此没有需要删除的宿主手势 API：它直接采用 `input_*` 调用，下表只说明每个调用现在的行为。坐标未变（窗格空间：x 自 plot 区域左边缘起，y 自图表顶部起）。

  | 已移除 | 现在 |
  | --- | --- |
  | `begin_financial_drag(x, y, click_count, radius)` | `input_pointer_down(PointerInput, click_count)`；`click_count` 为 `u32`（原为 `usize`）；4 px 的分隔条半径是常量 `PANE_SEPARATOR_HIT`；双击坐标轴重置遵循 `InteractionOptions::axis_double_click_reset_time` 和 `axis_double_click_reset_price` |
  | `update_financial_drag(x, y)`、`update_financial_crosshair(x, y, radius)` | `input_pointer_move(PointerInput, primary_pressed)`，它还会对绘图、系列和交易执行悬停提升，并在按下被捕获期间保持十字光标跟踪（限制在 plot 区域内）；`input_pointer_leave()` 在没有处于打开状态的按下时清除十字光标 |
  | `end_financial_drag()` | `input_pointer_up(PointerInput)` 用于完成，`input_cancel()` 用于放弃 |
  | `apply_financial_wheel(x, y, normalized_x, normalized_y)` | `input_wheel(WheelSample)`，返回图表是否消费了该滚轮事件。在默认的 `WheelBehavior::Auto` 下，`delta_y` 像 `normalized_y` 那样经过 `wheel_zoom_scale`，`delta_x` 像 `normalized_x` 那样经过 `WHEEL_SCROLL_PX_PER_DELTA`，因此作为 `normalized_x` 和 `normalized_y` 传入的值原样传给 `delta_x` 和 `delta_y`。它在行为上并不等价：参见下文的差异 |
  | `apply_financial_navigation(action, accelerated)` 和 `FinancialNavigation` | `input_key_down(ChartKey, InputModifiers, repeat, now_ms)` 和 `input_key_up(ChartKey)`：`PageUp`/`PageDown` 对应 `PreviousPage`/`NextPage`，`ZoomIn`/`ZoomOut` 保持原名，`accelerated` 即 `InputModifiers` 中的 Control 或 Shift。`PreviousBar`/`NextBar` 仅在方向上映射到 `ChartKey::ArrowLeft`/`ArrowRight`：参见下文的差异 |
  | `financial_drag()` 和 `FinancialDrag` | 无：已打开的手势是引擎私有的，指针反馈为 `input_cursor()` |

- 替代项与其所替代的调用行为不一致之处。这些是在代码中比对的，而非仅比对签名，且被移除的函数自引入起到 `17a591f` 之间都没有变化：
  - 价格坐标轴上的滚轮。无论滚轮行为如何，只要指针位于某个价格比例尺的坐标轴条带上，`apply_financial_wheel` 就会缩放该价格比例尺。`input_wheel` 仅在 `InteractionOptions::wheel_behavior` 为 `WheelBehavior::Zoom` 或 `InteractionOptions::price_axis_wheel_zoom` 为 `true`（默认 `false`）时才缩放它；在默认的 `WheelBehavior::Auto` 下，同样的滚轮在坐标轴条带上缩放的是时间比例尺。要恢复旧行为，请以 `price_axis_wheel_zoom: true` 调用 `set_interaction_options`。`WheelBehavior::Zoom` 在那里同样缩放价格比例尺，但它会把每一次滚轮都变成缩放，因此水平滚轮不再平移。
  - plot 区域上的滚轮缩放。旧调用围绕指针缩放时间比例尺。普通滚轮现在遵循 `right_bar_stays_on_scroll`（默认 `true`；参见下文的后续变更），因此最新柱之后的间隙得以保留，只有 Ctrl/Cmd 才会围绕指针缩放。`input_wheel` 还遵循 `wheel_zoom` 和 `wheel_scroll`（两者默认均为 `true`），并会停止正在进行的惯性滑行、按住方向键的平移或动画滚动。
  - 拖动阈值。旧调用从第一次 `update_financial_drag` 起就生效（窗格平移、坐标轴缩放或分隔条调整大小）。现在窗格平移、坐标轴缩放和分隔条拖动都在共享的 5 px 阈值处开始，未移动的释放是一次点击。因此在价格坐标轴上的普通按下不再将该比例尺切换为手动：旧的 `begin_financial_drag` 在按下时就关闭自动缩放，现在则由第一个缩放步骤来关闭。窗格拖动仅在手动价格比例尺是按下位置下方系列（或窗格默认）的比例尺时才平移它；旧调用还会回退到窗格的第一个手动右侧或左侧比例尺。
  - 价格坐标轴上的双击。`begin_financial_drag` 会运行 `reset_price_scales()`，即图表中的每个价格比例尺；`input_pointer_down` 只重置被按下的比例尺（`reset_price_scale(pane, target)`）。需要图表范围重置的宿主应为其自己的命令保留 `reset_price_scales()` 或 `reset_view()`。
  - 释放与取消。`end_financial_drag` 只结束缩放和滚动会话。未移动的 `input_pointer_up` 还会选择或激活指针下方的对象，`input_cancel()` 结束同样的会话，但还会还原已打开的绘图或交易拖动，并清除悬停、实时测量和光标。在按下处于打开状态时，`primary_pressed` 为 false 的 `input_pointer_move` 会放弃该次按下。
  - 方向键与缩放键。旧调用每次调用跳转 1 根柱（加速时 10 根）；`ArrowLeft` 和 `ArrowRight` 启动由速度持有的平移（参见关于时钟的一项）。`ZoomIn` 和 `ZoomOut` 锚定在 plot 区域中心；在新的 `right_bar_stays_on_scroll` 默认值下，它们改为保留最新柱之后的间隙。按键还遵循 `InteractionOptions` 的开关（滚动键使用 `pan` 或 `wheel_scroll`，缩放键使用 `wheel_zoom`；默认均为 `true`），被开关拦住的按键保持未被消费。
- 宿主提供时钟。方向键平移由速度持有，因此需要调用 `input_key_up`，传入平台的按键重复标志，每准备一帧调用一次 `input_tick(now_ms)`，并且仅在 `input_animating()` 成立期间请求下一帧。`input_wake_deadline_ms()` 是唯一的延迟截止时间（交易提示框的停留时间）：为它安排一次唤醒并重绘。`flush_coalesced_input()` 每次 prepaint 转发一次最新捕获的绘图采样。
- 引擎将仅宿主可做的工作以 `ChartInputEvent` 事件的形式交还（`ContextMenu`、`DrawingCreated`、`RemoveSeries`）；每次输入调用之后用 `take_input_events()` 取出它们。宿主保留事件转换、指针捕获、应用 `input_cursor()`、定时器与帧调度、菜单、剪贴板和持久化。应在 `drawing_revision()` 变化时持久化绘图，而不是跟踪手势。参考实现的 `handleScroll`/`handleScale` 开关即 `InteractionOptions`（`interaction_options()` 和 `set_interaction_options`）。
- 较低层的手势操作（`drawing_tool_pointer_*`、`measure_pointer_*`、`delta_tooltip_mouse_*`、`kinetic_*`、`start_keyboard_scroll`、`keyboard_scroll_tick`、`cancel_keyboard_scroll`、`time_axis_*` 和 `price_axis_*` 的缩放与滚动步骤、`drag_pane_separator`）仍然是公开的引擎操作，但现在由控制器对它们排序。仍在 `input_*` 之外继续驱动它们的宿主会绕过控制器的按下仲裁和光标，因此对它们排序的宿主路由应当删除。
- 帧准备随之变化：`prepare_financial_frame_with_measure` 在任何图层失效或输入变化之后重建，并在窗格调整大小的拖动之后重新布局。曾清空其帧以强制重建、或在 `update_financial_drag` 报告窗格调整大小时强制布局的宿主，可以停止这样做。
- GPUI 宿主（feature `gpui-backend`）通过 `aeris_charts_render_gpui::input` 绑定，每个监听器一次适配器调用：`GpuiChartInput::mouse_down`、`mouse_move`、`mouse_up`（图表之外的释放也要绑定）、`context_menu`、`scroll_wheel`、`pinch`、`modifiers_changed`、`key_down` 和 `key_up`。`scroll_wheel` 自行转换 GPUI 的增量（像素除以 100，行按 `WHEEL_LINE_HEIGHT`（32 px）计）。prepaint 调用 `set_canvas_bounds(bounds)` 和 `prepare_frame(&mut engine)`，`wake_delay(&engine)` 安排延迟唤醒，`cursor_style(engine.input_cursor())` 是唯一的光标映射，`install_text_metrics(&mut engine, window)` 在准备帧之前运行，使绘图标签的度量与其绘制一致。宿主自己的按键表、光标优先级和文本编辑路由在适配器之外是多余的，应当删除而不是保留。
- `17a591f` 之后的两项后续变更。`1869773 feat(input): TradingView wheel zoom anchoring, engine-owned on every host`（上游）将 `GpuiChartInput::set_origin(Point<Pixels>)` 重命名为 `set_canvas_bounds(Bounds<Pixels>)`（只有恰好位于 `17a591f` 的固定修订具有旧名称），新增了带有 `GpuiChartInput::pinch` 的 `input_pinch`，并使 `TimeScaleOptions::right_bar_stays_on_scroll` 默认为 `true`：普通滚轮或键盘缩放会保留最新柱之后的间隙，只有 Ctrl/Cmd 滚轮和捏合缩放才围绕指针缩放。如需较早的围绕指针（滚轮）或 plot 区域中心（按键）缩放，请调用 `ChartEngine::set_right_bar_stays_on_scroll(false)`。合并 `3eef45e merge: sync with AerisTerminal/aeris-charts main (range tools, input controller)`（自有线）新增了 `ChartKey::EditText`（F2），因此上游固定修订上对 `ChartKey` 的穷尽 `match` 需要补一个分支。

**GPUI 依赖**（自有线，来自 `9ae1c58 gpui: build the executor on gpui-pre 0.3.6, the GPUI gpui-kit pins`；参见 [Rust 分发](#rust-分发)）。`aeris_charts_render_gpui` 不再依赖 Zed 的 Git 修订：

- 此前，它使用来自 `https://github.com/zed-industries/zed` 的 `gpui` 0.2.2，修订为 `1057c2cf3d5b4aefd04755e1387c7826a4d7fba6`（其清单从 Zed 获取 GPUI，以跟进当前 1.0 之前的 API，并把 `gpui` 与 `gpui_platform` 固定在那一个经过评审的提交上）。现在，`gpui-backend` feature 启用 `gpui = { package = "gpui-pre", version = "=0.3.6" }`，一致性测试框架则使用 `=0.3.6` 的 `gpui-pre-platform`。清单把它记录为 gpui-kit 0.6.6（`gpui-component`）所固定的 `gpui`，因此使用 gpui-kit 0.6.6 的宿主应当已经解析到它；该固定不在本仓库中检查。
- 宿主必须解析到同一个包和同一版本：把其 Zed Git 依赖替换为 `package = "gpui-pre"`、`version = "=0.3.6"` 的形式（若使用平台 crate，则同时替换为 `gpui-pre-platform`）。仍停留在 Zed 修订上的宿主会同时持有两份 GPUI，每一个接收或返回 GPUI 类型的适配器与执行器调用（`input.mouse_down(&mut engine, &MouseDownEvent)`、`install_text_metrics(&mut engine, &Window)`、返回 `CursorStyle` 的 `cursor_style`）都无法针对宿主的那一份通过类型检查。
- 在 Windows 上保持启用 `windows-manifest` feature：GPUI 导入 `comctl32!TaskDialogIndirect`，它只有在该 feature 的清单所嵌入的 comctl32 v6 激活上下文下才能解析，因此没有它的可执行文件会在 `main` 之前加载失败（`STATUS_ENTRYPOINT_NOT_FOUND`）。
- `9ae1c58` 只改动清单、锁文件和文档：执行器与适配器源码未经修改即可针对 `gpui-pre` 0.3.6 编译，因此 Aeris 自身的 API 没有变化。宿主自己的 GPUI 代码不在本仓库中检查：迁移到 `gpui-pre` 0.3.6 是宿主需要自行完成并验证的变更。

**GPUI 快照 0.3.7**（自有线，即把 `gpui-pre` 升到 0.3.7、并把其余所有依赖升到最新发布版本的那个提交；可用 `git log -S'=0.3.7' -- crates/aeris_charts_render_gpui/Cargo.toml` 找到它）。它跟随 gpui-kit，后者已从 0.6.6 升到 0.7.0，并把 `gpui-pre =0.3.7` 与 `gpui-pre-platform`、`gpui-pre-web`、`gpui-pre-macros` 和 `gpui-pre-sum-tree` 固定在同一版本：

- 使用此执行器的宿主必须只使用一个 `gpui-pre` 版本，因此仍在 gpui-kit 0.6.6（`gpui-pre =0.3.6`）上的宿主，要在升级 Aeris 固定修订的同一次变更中迁移到 gpui-kit 0.7.0；两侧版本各异时，Cargo 会解析出两份互不兼容的 `gpui`，每一个接收或返回 GPUI 类型的适配器与执行器调用都无法通过类型检查，与上文较早的 Zed 修订的情形相同。
- Aeris 自身的 API 没有变化：执行器与适配器源码未经修改即可针对 0.3.7 编译，完整的 GPUI 测试套件也未经修改即通过。宿主自己的 GPUI 代码不在本仓库中检查。
- 一致性测试框架的 X11 捕获已迁移到 `x11rb` 0.14（仅为示例的 Linux 开发依赖）。GPUI 的 Linux 平台仍依赖 `x11rb` 0.13，因此示例构建同时包含两者；宿主所链接的内容不受影响。

**测量工具**（两条线，来自合并提交 `3eef45e merge: sync with AerisTerminal/aeris-charts main (range tools, input controller)`）。上游的 `5a2e6e8 feat(drawings): add price/date range measuring tools and Shift-click measure` 与自有线的 `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling` 各自独立地构建了这三个范围工具，合并保留了自有线的实现。固定修订采用哪种写法，取决于它属于哪一侧：

- 上游自 `5a2e6e8` 起的固定修订具有 `DrawingKind::DatePriceRange`、种类名称 `date_price_range`，以及线上 id 13（`PriceRange`）、14（`DateRange`）和 15（`DatePriceRange`）。自有线自 `36c9f09` 起的固定修订（例如 `36c9f09` 本身）已经具有 `DrawingKind::DateAndPriceRange`、名称 `date_and_price_range` 以及 id 130、131 和 132，这也是 main 所保留的。对自有线的固定修订而言，没有重命名，也没有 id 重映射；只适用下文的网格吸附。任一侧早于这些提交的固定修订都没有范围工具。
- 对上游的固定修订而言，`DrawingKind::DatePriceRange` 现在是 `DrawingKind::DateAndPriceRange`（`PriceRange` 与 `DateRange` 保持原名）。`DrawingKind::to_u8` 与 `from_u8` 的数值线上 id 已变动：`PriceRange` 为 130，`DateRange` 为 131，`DateAndPriceRange` 为 132，此前分别为 13、14 和 15。id 13 至 15 现为未分配，id 0 至 12 不变，因此存储了上游固定修订数值 id 的宿主需要重映射这些 id。
- 种类名称 `date_price_range` 仍会被读取（serde 别名和 `DrawingKind::from_name`），但绝不会被写出，因此由上游固定修订保存的文档仍可加载；保存的文档、模板和剪贴板载荷写出的是 `date_and_price_range`。
- 网格吸附。三个范围工具以及多头和空头仓位工具的创建、锚点拖动、主体拖动和键盘微调，都会吸附到整根柱以及品种 tick 或价格带价位梯。合并之前的自有线固定修订在这些工具上没有这种吸附，因此这五个工具都会获得这一变更（仓位工具的价格吸附更早，随合并 `2e7d19f` 引入）。上游自 `5a2e6e8` 起的固定修订已经让它们吸附到柱和价格 tick；它新增了价格带价位梯（`SeriesPriceFormat::tick_ladder`）。
- Shift 点击快速测量随 `5a2e6e8` 引入（合并之前的自有线固定修订从未具有它）。main 由输入控制器驱动它（在窗格上按下 Shift），因此无需为此调用 `measure_pointer_*`。

**其他源码级变更。** 每一项都注明携带该变更的提交。所涉及的公共枚举均不是 `#[non_exhaustive]`，因此每新增一个变体，对穷尽的 `match` 都是编译期破坏性变更；每新增一个字段，对列出全部字段的结构体字面量也是如此。

- `a565efc fix(kline): close K-line engine pitfalls across time, indicators, drawings, streaming, viewport, price axis, and intraday charts`（自有线）：
  - `PriceScaleCore::build_tick_marks` 与 `price_tick_span_calculator::composite_tick_span` 现在接收 `min_move: f64`（每个刻度所在的价格网格），此前接收的是 `base: i64`（格式化器的基数）。base 为 100 即 `min_move` 为 `0.01`，这也是引擎在百分比与指数化比例尺上的网格。
  - `SessionHighlightingOptions::start_hour_utc` 与 `end_hour_utc`（`Option<u8>`）现在是 `start_hour` 与 `end_hour`（`Option<f64>`）：带小数的交易所本地小时数（`9.5` 即 09:30；`start_hour > end_hour` 时跨越午夜）。`Some(9)` 变为 `Some(9.0)`，在交易所时间为 UTC 时含义相同。
  - `IndicatorKind::Ema`、`Dema`、`Tema` 与 `Rsi` 新增 `seed: IndicatorSeed`，`Macd` 新增 `seed` 与 `histogram_multiplier: f64`，`Bollinger` 新增 `estimator: DeviationEstimator`。`IndicatorSeed::Sma`、`1.0` 与 `DeviationEstimator::Population` 即此前的行为（同时也是 serde 默认值，因此已保存的文档仍可读取）；列出这些字段的模式需要加上 `..`。
  - `DrawingClipboardItem::points` 现在是 `Vec<DrawingAnchor>`，而不再是 `Vec<DrawingPoint>`（`DrawingAnchor { logical: Option<f64>, price, time: Option<f64> }`，可用 `DrawingAnchor::from(point)` 转换）。
  - 新增变体：`IndicatorKind::Kdj` 与 `IndicatorParameterType::Choice`。新增字段：`TimeScaleOptions::lock_visible_logical_range`、`PriceScaleCoreOptions::{ ensure_edge_tick_marks_visible, base_value, autoscale_center, stable_auto_scale}`、`PriceMark::edge`、`IndicatorInput::amount`、`SeriesPriceFormat::tick_ladder`、`SeriesEntry::histogram_updown_rule`、`IndicatorBindingInfo::amount_source`、`IndicatorParameterDescriptor::choices`，以及 `DrawingClipboardPayload` 和 `DrawingSyncPayload` 上的 `price_basis`。
- `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling`（自有线）：`ChartEngine::copy_drawings_json` 返回 `Result<String, ChartError>`，而不再返回 `Option<String>`（没有任何已知绘图可复制时返回 `ErrorCode::InvalidData`；超过 `MAX_DRAWING_CLIPBOARD_POINTS` 个锚点（最先检查）以及超过 `MAX_DRAWING_CLIPBOARD_BYTES` 字节时返回 `ErrorCode::ResourceLimit`）。上游的固定修订同样是 `Option<String>` 形式。新增变体：`DrawingKind`（B8 目录；自有线的后续提交又加入 `HorizontalSegment`、`VerticalRay`、`VerticalSegment`、`PriceChannel`、`PriceLine`、`SimpleTag` 和 `SimpleAnnotation`）、十个 `DrawingKindOptions` 变体、`DrawingDragPart::Handle(usize)`、`FootprintError::InvalidSessions`、`ResampleError::{InvalidSessions, TimeAxisRequired}`、`TradeStudyKind::Volume`，以及 `aeris_charts_core::model::plot_list::PlotValues::AsOf`。新增字段：`Drawing::tool_options`、`SeriesEntry::break_on_trading_day`，以及 `TradeStreamStats::{dependent_rows_computed, bar_rows_projected, bubble_trades_scanned, bubble_markers_sized}`。
- 自有线的后续新增：`ExchangeTimeError::BarTimeLabelWindows`（`78d7d59 feat(time): close-time display labels for open-stamped bars`）、`IndicatorKind::KLineChart`（`c2d837e engine: bind KLineChart indicators as chart studies`），以及 `DrawingTextEditLayout` 上的 `angle` 与 `multiline`；自有线自 `36c9f09` 起就有该类型（`d438dab feat(drawings): edit the text of every text-bearing tool and open the editor on placement`）。
- `960e011 feat(drawings): add complete position statistics and adaptive borders`（上游）：`DrawingKindOptions::Position` 新增 `account_size: f64` 与 `risk_percent: f64`，`Drawing` 新增 `position_account_size` 与 `position_risk_percent`。
- `5c62071 feat(engine): own native workspace identity`（上游）：`prepare_financial_frame_with_measure` 现在接收一个 `FinancialFrameRequest { width, height, dpr, force_layout, fit_content, frame, axis_primitives }`，其后是 `measure` 与 `countdown_measure`；此前它接收九个位置参数（`width`、`height`、`dpr`、`force_layout`、`fit_content`、`measure`、`countdown_measure`、`frame`、`axis_primitives`）；它仍返回 `FinancialFramePreparation`。
- 上游在分叉点 `ed2910d` 与 `1869773` 之间的提交还修改了这些既有的公共类型，因此不包含这些提交的固定修订，在其所匹配或构造的声明上会与此处存在差异：`TradingHitKind` 新增 `TakeProfitButton` 与 `StopLossButton`（`24e8e2d fix(trading): dedicated TP/SL buttons with pixel-exact, optically centered controls`），`TradingStyle` 与 `TradingStyleOptions` 新增 `execution_buy` 与 `execution_sell`，而 `ExecutionMarkerShape` 的默认值由 `Circle` 变为 `Arrow`（`6f56736 fix(trading): bar-anchored execution arrows with stacked multi-fill marks`）。范围种类与 `Position` 字段已在上文涵盖。早于 `ed2910d` 的固定修订还会与此前变更的类型存在差异，本节未列出这些类型。

## 发布策略

标签发布依赖必需的 Rust、包以及可移植的 Chromium/Firefox/WebKit 作业。它还会检查持久化夹具、公共声明、Node/SSR 导入、包内容以及已配置的性能预算。可移植正确性禁止使用 `continue-on-error`。

对硬件和机器敏感的截图哈希、GPU 计时、堆采样和挂钟时间证据属于校准诊断。它们仍然不具权威性，不得仅为让某个 runner 通过而予以批准。共享绘制流一致性、裁剪/顺序/帧契约测试、回放确定性以及可移植的浏览器行为才是具有权威性的门禁。没有配置基准预算的场景仍明确为仅报告。
