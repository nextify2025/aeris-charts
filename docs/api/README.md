# 公共 API

[文档导航](../README.md) · [架构总览](../Architecture.md)

本目录区分受支持接口、实验性接口与尚未实现的提案。源码入口是 [`packages/charts/src/index.ts`](../../packages/charts/src/index.ts)，句柄类型见 [`types.ts`](../../packages/charts/src/types.ts)，npm 导出以 [`package.json`](../../packages/charts/package.json) 为准。

## 按任务阅读

| 任务 | 文档 |
| --- | --- |
| 指标约定、KLineChart 模板、成交量分布 | [指标](indicators.md) |
| 坐标、窗格、交易所时区、收盘标签 | [坐标与时间](coordinates-and-time.md) |
| 固定整交易时段视图 | [分时图](intraday.md) |
| 成交聚合与 OHLCV 重采样 | [聚合](aggregation.md) |
| 绘图锚点、磁吸、复权与工具目录 | [绘图](drawings.md) |
| 通用系列、坐标轴与类型化数据 | [通用图表](general-charts.md) |
| 错误、生命周期、持久化与版本策略 | [兼容性](compatibility.md) |
| Rust/GPUI 接入与固定修订升级 | [Rust 接入](rust.md) |

架构实现见[架构总览](../Architecture.md)，足迹图与深度的领域契约见[文档导航](../README.md#领域设计)。

## 受支持的产品接口面

受支持的产品是 1.0 之前的浏览器包 `@aeristerminal/aeris-charts`。其与框架无关的根 ESM 入口、可选的 `./react` 适配器、`./wasm` 资源和 `./design.css` 样式表是受支持的 npm 导出路径。React 是可选的 peer 依赖，根入口的使用者不会加载它。受支持的根接口面如下：

- 图表的创建与初始化；
- `packages/charts/src/types.ts` 中声明的图表、系列、时间比例尺、窗格、价格比例尺、价格线和绘图句柄；
- 窗格本地的命名价格比例尺，通过 `chart.add_price_scale()`、`chart.price_scales()`、`chart.move_price_scale()`、`chart.remove_price_scale()`、`chart.price_scale()`/`pane.price_scale()` 中的任意字符串 ID，以及系列的比例尺标识与重新绑定来提供；
- 价格轴语义：刻度标签始终落在系列的 `min_move` 网格上；内置的 `price_format` 若指定了 `min_move` 而未指定 `precision`，则自动推导精度（参考实现 `precisionByMinMove`）；`price_format.tick_ladder` 价格区间（交易所价差表，例如 HKEX 表）将每个标签舍入到所属区间的价位刻度并按区间设置精度，使坐标轴刻度保持在可见区间的公共网格上，并驱动该系列所在比例尺上的交易磁吸；系列选项 `autoscale_info_provider`（参考实现 `autoscaleInfoProvider`）替换系列的自动缩放信息（它在渲染期间运行：在其内部调用图表 API 会抛出 `unsupported_operation` 且不改动图表，抛出异常的 provider 在该次渲染 pass 中被忽略）；比例尺选项 `tick_mark_density` 和 `ensure_edge_tick_marks_visible` 遵循参考实现，Aeris 扩展项 `base_value`（与绘图共享的显式百分比/指数化基准）、`autoscale_center`（对称自动缩放）和 `stable_auto_scale`（可选启用的迟滞；默认保持与参考实现完全一致）都是普通的 `price_scale_options`；格式错误的价位梯和扩展值会抛出 `invalid_options`，并使价格格式或比例尺保持不变。指数化到 100 的标签使用参考实现的固定两位小数格式化器；
- [分时图](intraday.md#分时图)一节所述的分时构建模块：`session_slot_times()`、显式的时间轴 `tick_marks`、`histogram_updown_rule` 的前收盘价成交量着色（配合宿主提供的 `up_color`/`down_color`）、显示其首根有成交的柱的基线系列、新增的 `break_on_trading_day` 线/面积/基线选项（默认 `false`），以及在每次重置时重新开始的 VWAP/枢轴线；收盘时间显示标签 `time_scale_options.bar_time_label` 按柱的收盘时间显示柱，而柱本身仍保持开盘时间戳（参见 [收盘时间标签](coordinates-and-time.md#收盘时间标签)）；
- 通过系列选项 `time_alignment: "as_of"` 和 `as_of_max_staleness` 实现的多日历叠加层（参见 [多日历叠加层](coordinates-and-time.md#时间交易所时区与交易时段)）；
- 这些句柄所声明的内置系列、指标、绘图类型、选项、主题、数据写入、交互、订阅、截图和生命周期操作；
- 通过规范的 `drawing_kind` 值 `"long_position"` 和 `"short_position"` 提供的多头头寸与空头头寸绘图；每个绘图按入场、目标、止损的顺序存储三个可编辑锚点，绘制目标/入场/止损信息，将三个价格都投影到所属 Y 轴，并使用共享的绘图历史、持久化、命中测试和后端帧路径。统计数据使用两个持久化的绘图选项：`position_account_size`（假设的余额，默认 1,000）和 `position_risk_percent`（在止损处承担风险的占比，0–100，默认 25），与券商订单无关；
- 通过规范的 `drawing_kind` 值 `"price_range"`、`"date_range"` 和 `"date_and_price_range"` 提供的测量绘图（早先的拼写 `"date_price_range"` 在导入、模板、剪贴板和同步时仍会被读取，但绝不会写出）；每个绘图存储一个可编辑的起始锚点和结束锚点，吸附到整根柱和价格刻度，标注带符号的价格变化、百分比、刻度数（按品种刻度或价格区间价位梯计数）、柱数和经过的时间（`labels` 选项决定显示哪些度量项），并以绘图颜色绘制；
- 内置指针处理中的 Shift 点击快速测量：在图表空白区域 Shift + 按下，会启动一次临时的日期与价格测量，该测量跟随指针（上涨使用绘图默认颜色，下跌使用市场下跌颜色），在拖动后松开时或下一次点击时冻结，并由随后的点击或 Escape 取消。它绝不是绘图、历史条目或持久化对象；
- 通过 `chart.add_volume_profile(prices, volume, options)` 提供的可见范围成交量分布，返回带有 `options()`、`apply_options()`、`snapshot()` 和 `remove()` 的分布句柄；
- KLineChart 的 27 个指标模板，通过 `chart.add_klinechart_indicator(source, indicator, volume_source?, options?)` 提供，详见 [KLineChart 指标](indicators.md#klinechart-指标)；
- 一等的、由 Tick 驱动的足迹图 / Numbers Bars 系列，通过 `chart.add_series("footprint")` 提供，包括对象与类型化列的成交写入、显式、报价和 tick 规则的主动方处理、每价位的 Bid × Ask/总量/delta、控制点（POC）、最终/最大/最小/交易时段 delta、可配置的对角失衡与堆叠失衡、密度细节层级（LOD），以及派生的柱/价位查询；通用 OHLC setter 会被拒绝，因为它们无法提供订单流真值；
- 一个图表级回放时钟，加上共享的规范成交流、类型化批量写入、普通的 K 线/柱绑定、精确的 seek 工作量遥测、`chart.trade_stream_stats()` 中的实时末端依赖工作计数器（`dependent_rows_computed`、`bar_rows_projected`、`bubble_trades_scanned`、`bubble_markers_sized`），以及通过 `configure_synthetic_bar_series`、`set_synthetic_bar_source[_typed]` 和 `update_synthetic_bar_source` 提供的固定/ATR Renko、Line Break、Kagi 和 Point & Figure 变换；合成数据源/历史仍归宿主所有，不属于图表状态持久化的一部分；
- [Tick 转 K 线与重采样](aggregation.md#tick-转-k-线与重采样)一节所述的由 Tick 构建的普通 K 线与 OHLCV 重采样：以交易所交易时段为锚点的成交流时间柱（`chart.set_trade_stream_sessions()`）、成交流成交量直方图（`chart.add_trade_volume_series()`），以及通过 `chart.configure_resampled_series()`、`chart.resampled_bars()`、`chart.resample_stats()` 和由交易时段派生的 `resample_boundaries()` 辅助函数提供的引擎重采样；重采样配置仅在运行时存在；
- 由引擎解析的辅助点击上下文，通过 `chart.subscribe_chart_context()` 提供，包括窗格、时间、逻辑索引、坐标、命中的系列，以及其比例尺上的精确价格；菜单、剪贴板操作和订单操作由宿主拥有；
- 图表范围的引擎值查询，通过 `chart.value_snapshot(logical_index?)` 提供，包括每个存活系列的句柄/ID、当前类型、窗格/比例尺归属位置、由引擎拥有的精确值或各系列独立的最新值、前驱值，以及格式化字段；`mouse_event_params.value_snapshot` 携带相同的记录，并在十字光标离开时恢复最新值，而旧版 `series_data` 仍只包含有值的系列；
- 新增的完整指标谱系元数据：稳定的绑定 ID、结构化参数、数据源与可选的 VWAP 成交量数据源，以及稳定的输出名称/索引/数量，同时保留旧字段；
- [指标约定](indicators.md#指标约定)一节所述的指标计算约定、KDJ、对空白数据安全的指标数据源、预热查询和成交额加权平均价；
- 五输出 EMA 色带，通过 `chart.add_ema_ribbon()` 提供，默认周期为 `5/10/20/50/200`，默认颜色为 `#335cff/#FF9800/#7d52f4/#fb4ba3/#fb3748`，以及通过 `chart.set_ema_ribbon_periods()` 进行的原子的就地周期修改；
- 自有的、与券商无关的交易展示、本地预览、意图与回滚、命中测试、语义样式，以及类型化的意图订阅（确认流程由宿主拥有），均通过 `chart.trading()` 暴露；
- 以宿主为权威的警报线指标，以及十字光标加号徽标创建请求，通过 `chart.alerts()` 暴露；条件/频率作为配置元数据被保留，而对话框、评估、持久化、限额、过期、后台投递和通知由宿主拥有；
- 默认的图表无障碍、其新增的 `chart.accessibility()` 单例句柄、用于兼容的 `enable_accessibility()`、无障碍选项，以及键盘数据/绘图操作；
- 新增的 `wheel_behavior` 图表选项（`auto`、`pan` 或 `zoom`）；现有的手势选项名称保持兼容；
- 时间比例尺视口契约：数据更新（历史前插、乱序插入、缺口回填、保留期修剪）绝不会移动已向后滚动的视图，而实时边缘按 `shift_visible_range_on_new_bar` 跟随新柱；`set_visible_logical_range()` 保留小数边界；`scroll_to_real_time()` 以动画滚动到已配置的 `right_offset`；键盘时间比例尺移动遵循 `handle_scroll`/`handle_scale`；在处理函数同步修改数据之后，可见范围订阅者始终以最终范围收尾；
- 新增的 `lock_visible_logical_range` 时间比例尺选项（默认 `false`），用于固定的整交易时段视图，例如分时图：将每个交易时段槽位安装为空白数据，调用 `set_visible_logical_range({ from: 0, to: slots - 1 })`，该范围从开盘前状态直到收盘，在数据更新和尺寸调整期间始终保持精确；
- `AerisChartsError` 及其机器可读的错误码；
- 通过 `chart.export_state()` 和 `chart.import_state()` 实现的版本化图表状态持久化：V1 金融布局、V2 通用状态与 V3 金融研究，详见[持久化契约](compatibility.md#持久化-v1)。
- 面向常见 JavaScript 生命周期的驼峰命名别名（`createChart`、`initWasm`、图表/系列/比例尺创建与数据方法），同时每个现有的蛇形命名入口在相同句柄上仍受支持；
- 通过 `chart.reset_style_to_defaults()` 实现的规范展示重置。它为图表所选主题恢复由 Aeris 拥有的图表与系列视觉默认值，包括语义上的未设置/跟随状态，同时保留数据、窗格、绘图、指标、系列可见性/元数据、价格格式、比例尺绑定以及比例尺/视图状态。它有意与 `chart.reset_view()` 分离，后者会更改时间/价格比例尺的视图状态。
- 通过 `chart.backend_status()` 提供的只读后端诊断，包括请求的后端和活动后端、稳定的回退阶段/原因、安全上下文与 `navigator.gpu` 的暴露情况，以及可选的不稳定平台细节。`chart.backend()` 保留其现有的活动后端返回值。

`./react` 入口导出 `AerisChart`、`FinancialSeries`、`GeneralPane` 和 `useAerisChart`，以及它们的配置类型。它是位于根命令式 API 之上的编写适配器：普通的重新渲染会保留图表/系列的标识，数据变化会修改这些已有句柄，结构性的通用坐标轴或系列变化只替换受影响的引擎对象，卸载则使用规范的销毁路径。它不会独立于 Rust 引擎定义图表语义。导入该模块在 SSR 下是安全的；DOM/WASM 图表创建从已挂载组件的 effect 开始。`FinancialSeries` 以流式方式更新：新的 `data` 数组若与先前已应用的数组相比仅有被替换的最后一个点和/或按升序追加的点（通过标识或浅相等识别，至多 1,024 个变化点），则通过 `series.update()` 应用；其他任何变化均为一次 `setData`。
