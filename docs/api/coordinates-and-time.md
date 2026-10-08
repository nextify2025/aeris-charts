# 坐标与时间 API

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

- [坐标与窗格](#坐标与窗格)
- [时间、交易所时区与交易时段](#时间交易所时区与交易时段)
- [收盘时间标签](#收盘时间标签)

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

**多日历叠加层**。`series_options.time_alignment` 为 `"union"`（默认值，参考实现的行为）或 `"as_of"`。as-of 系列（例如叠加在另一交易所股票之上的指数、叠加在股票之上的加密货币、叠加在其标的之上的期货夜盘）不会增加时间点，因此其他系列的时间轴保持无间断。每个时间点——直到其他系列的最后一根真实柱为止——显示该叠加层在不晚于该时间点的时间处的最后一行：位于两个时间点之间的行会合并到后一个时间点，没有更新行的时间点重复上一行，最后一个时间点之后的行则等待其他系列到达它们（预先安装的日内交易时段绝不会在其未来槽位中显示叠加层）。`as_of_max_staleness`（整数秒，默认 `null`）会让时间点留空，而不是重复比该值更旧的行；`0` 只显示恰好位于某个时间点上的行。时间点来自 union 系列，因此图表上单独存在的 as-of 系列不显示任何内容。所有读取该系列的功能都遵循同样的时间点：渲染、命中测试、十字光标和图例的值、`value_snapshot`（报告该时间点的时间）、比较锚点和百分比基准、自动缩放、标记（放置在不早于其时间的第一个时间点上；比所有时间点都新的标记随其行一同等待，被陈旧度上限留空的时间点会隐藏它）、交易时段高亮（每个时间点按其所显示的行着色）、无障碍焦点环（位于显示该焦点行的第一个时间点上），以及最新值界面元素。

`data()`、`data_by_index` 和 `last_value_data` 返回叠加层自身的行及其自身的时间。绑定到 as-of 系列的研究在其自身的行上计算，并以同样方式显示；它们的 `time_alignment` 读回的是源的值，且无法设置。任一侧的实时 Tick 仍与其所改变的时间点成比例。仅拥有自身行数据的折线、面积、基线、直方图、柱和 K 线系列接受它：自定义、高级、足迹图、成交绑定、成交研究和合成系列，以及非时间（Tick、成交量、区间或合成）柱轴上的每个系列，都会抛出 `unsupported_operation`。把 as-of 系列转换为自定义、高级或足迹图系列、将其绑定到成交流，或将其配置为合成变换，会使它回到 union。错误的值，或在没有 `"as_of"` 时给出陈旧度上限，会在该调用的其他选项生效之前抛出 `invalid_options`；`add_series` 在创建（或接管）该系列之前检查这两个键，被拒绝时不会在主线程或 worker 中留下任何系列。

重复应用当前的对齐方式是空操作，不会通知任何 `subscribe_data_changed` 处理器；发生变化时会以 `"full"` 通知每个处理器一次。worker 图表在 `add_series` 选项中接受这两个键，之后可通过 `offscreen_chart.apply_series_options(patch, series_id)` 更改它们；`offscreen_chart.series_options(series_id)` 读回这些选项。worker 图表通过 `add_series` 返回的数值 id 来定位系列（`0` 为主系列）。该补丁遵循与 `apply_options` 相同的规则并抛出相同的错误码（省略的键保持其值，切换到 `"union"` 会清除陈旧度上限，未变化的请求是空操作），抛出错误时不应用任何内容。它只接受 `time_alignment` 和 `as_of_max_staleness`：任何其他带值的键都会抛出点名该键的 `unsupported_operation`，非对象补丁则抛出 `invalid_options`。

不是 `0..=4294967295` 范围内整数的 id，或指向没有存活系列的 id，会抛出 `invalid_handle`（对于已被移除的系列则抛出 `stale_handle`），即使补丁为空也是如此；已被移除的图表则抛出 `disposed`。调用成功时，会在返回之前重绘 worker 画布，与其他 worker 变更一致（未变化的请求也是如此）；抛出错误的调用不会绘制任何内容。worker 图表没有系列句柄，因此没有 `subscribe_data_changed` 通知；请在调用之后读取 `visible_logical_range()`。Rust 宿主调用 `ChartEngine::set_series_time_alignment(id, TimeAlignment::AsOf { max_staleness })`。与其他金融系列选项一样，该设置由宿主拥有，且不被持久化。

**交易所时区**。`chart.time_scale().apply_options({ time_zone, session_start })`，或声明式图表选项 `timeScale: { timeZone, sessionStart }`（worker 图表同样接受），用于设定时刻如何分组和标注。`time_zone` 可以是 `"UTC"`（默认值）、IANA 名称（如 `"Asia/Shanghai"` 或 `"America/New_York"`），或由 `{ from_utc_seconds, offset_seconds }` 转换点构成的显式时间表（严格升序，至多 1024 个，偏移量在 ±18 h 以内；第一个偏移量也适用于其条目之前的时间）。该包对每个时区仅用 `Intl.DateTimeFormat` 在 1970–2100 年间解析一次 IANA 名称（至多约 262 次 DST 转换），并至多保留 32 个已解析的时区；超出该区间时采用最近的偏移量。引擎本身绝不读取浏览器的时区：`timeScale.timeZone` 只接受 `"UTC"` 或显式时间表。

TradingView 对标列表中的时区（`TRADINGVIEW_TIME_ZONES`；WASM 的 `supported_time_zones_json()`）则可以通过 `ChartEngine::set_time_zone(&str)`（发生变化时返回 `Ok(true)`；WASM 的 `set_time_zone`）或顶层引擎选项 `timezone` 以名称传给引擎；它会被一次性解析为同一种时间表（1970–2100 年，原生约 4 ms），因此分组和标签与显式时间表一致，并且该名称还会本地化通用（非金融）时间坐标轴以及 `ChartEngine::time_zone_clock_text(utc_seconds, show_seconds)`。`ChartEngine::time_zone_id()`（WASM 的 `time_zone()`）返回已命名的时区，默认为 `Etc/UTC`，在安装了显式时间表期间返回 `custom`。未知时区、格式错误的时间表、超出范围的交易时段起点，或与已安装收盘时间标签（[收盘时间标签](#收盘时间标签)）的窗口不适配的交易时段起点，会在同一次调用中应用任何其他键之前抛出 `invalid_options`；不受支持或非字符串的 `timezone` 以同样方式拒绝该补丁。

`time_scale().options()` 报告 IANA 名称（或时间表）和 `session_start`。Rust 宿主通过 `ChartEngine::set_exchange_offsets(UtcOffsetSchedule)` 设置显式时间表，通过 `set_session_start_seconds(i32)` 设置交易时段起点；引擎 JSON 选项接受 `timeScale.timeZone`（`"UTC"` 或转换数组）、`timeScale.sessionStart` 和 `timezone`。V2 持久化会往返保存它们：时间表始终保存，而 `timezone` 名称仅在已安装命名时区期间保存（显式时间表会将其清除）。当同一个补丁同时带有时间表和名称时，时间表决定分组和标签，名称决定通用坐标轴和时钟。导入早于这些键的文档时，会保留图表已安装的时区和交易时段起点。

仅显示的时间投影：`ChartEngine::set_future_time_projection(cadence_seconds, points)` 和 `set_past_time_projection(cadence_seconds, points)`（各自上限为 4,096 个点；传入 `None` 或 0 个点会将其清除；`has_future_time_projection` / `has_past_time_projection` 可读回）为时间轴上最后一根柱之后和第一根柱之前的空白区域标注标签。投影点仅是标签（没有数据行、基础索引或点数），且不会被持久化。

**交易日**。`session_start` 是交易日开始时刻相对交易所本地午夜的偏移秒数（默认 `0`，范围 ±86 399）。负值会把夜盘时段归入下一个交易日，例如 `-3 * 3600` 使 21:00 的中国期货夜盘从次日开始；起点为负时，本应落在周六或周日的交易日会顺延到周一，因此周五夜盘属于周一。Day/Month/Year 刻度标记、VWAP 的 `session`/`weekly`/`monthly` 重置、枢轴点交易时段以及默认 `exchange` 日历的时段研究（交易时段高低点、上一日/周/月水平与开盘区间，见[时段日历](../features/studies.md#时段日历)）均使用交易日。每周周期从周一开始。

周日晚间开盘的市场（CME Globex，美国中部时间 17:00）将 `session_start: -25200`：周日 17:00 属于周一的交易日，周一 17:00 属于周二的交易日，因此 Day 标记、交易时段 VWAP 与每周 VWAP 重置以及枢轴点都与交易时段对齐。取 `0` 时，周日晚间自成一个交易日：在交易时段中间的午夜会出现 Day 标记和交易时段 VWAP 重置，而每周 VWAP 把周日晚间的柱留在上一周，并在该午夜重置。起点为负时，每个周六或周日的时刻都属于周一，而窗口放置（`session_slot_times`、`resample_boundaries`、`set_trade_stream_sessions`）假定一周在周五晚间开盘：这对中国期货是正确的，但周日开盘的市场应在每次调用中以 `session_start` 为 `0` 来放置其晚间窗口（参见 *“分时图”* 和 *“Tick 转 K 线与重采样”*）。

**遵循交易所时间的内容**。刻度边界（Day/Month/Year 来自交易日；小时和分钟标记基于交易所墙上时钟时间，因此在 DST 和非整点偏移下仍保持在交易所交易时间内）、每个界面上的内置标签（坐标轴刻度、十字光标、矩形绘图的坐标轴标签、delta 提示框、`create_tooltip` 以及无障碍文本）、VWAP 和枢轴点重置、交易时段高亮的小时门限与周末判断，以及倒计时窗口。Day 标记刻度标签标注交易日期；十字光标和提示框文本显示该时刻的交易所墙上时钟日期和时间。

**日历日期数据**。当每个有数据的金融系列都以 `business_day` 或 `"YYYY-MM-DD"` 时间给出时，图表将其时间点视为日历日期：它们在每个时区中保持自己的日期，绝不会被时区或 `session_start` 移位。任一金融系列中只要有一个数值时间，时间点就重新成为时刻。日线及更长周期的柱应以日历日期发送；以 UTC 午夜为时间的数值日线柱是时刻，在 UTC 以西的时区会显示为前一天晚上。类型化列输入始终是数值时刻。Rust 宿主通过 `ChartEngine::set_calendar_date_axis(bool)` 声明相同状态。

**格式化钩子**。`time_scale_options.tick_mark_formatter(time, tick_mark_type, locale, context)` 和 `localization.time_formatter(time, context)` 接收 UTC 秒数以及一个 `time_label_context`，其 `business_day` 对日历日期行为该日历日期，对时刻则为 `null`。宿主的 `time_formatter` 会覆盖所有输出时间点的界面：十字光标标签、矩形坐标轴标签、delta 提示框、`create_tooltip` 以及无障碍（除非无障碍选项设置了自己的 `time_formatter`）。没有它时，`create_tooltip` 和无障碍会输出十字光标标签（`chart.format_time_label()`）：按图表时区使用 `localization.date_format` 和 `localization.locale`，日内行附加当日时间，日历日期行则不附加。Rust 的 `TickMarkFormatterFn` 和 `TimeFormatterFn` 签名保持不变。配置了收盘时间标签（[收盘时间标签](#收盘时间标签)）时，两个回调收到的都是标签时刻（柱的收盘时间），而不是柱的开盘时间；在自己的格式化函数中加上一个周期的宿主，在采用该选项时必须去掉这一加法。

**倒计时时钟**。K 线收盘倒计时仅在时钟处于正在形成的柱的区间 `[last_bar_time, last_bar_time + bar_interval)` 内时显示；在区间之外——午休、隔夜、周末、提前收盘之后——它会隐藏而不是循环。日历日期柱在其日期对应的交易所交易日内形成（`session_start` 为负时，周一的柱从周五晚间的交易时段起始时刻开始：中国期货为周五的夜盘，CME Globex 这类周日开盘的市场为美国中部时间周五 17:00），而相隔 28 天或更多天的柱持续到各自所在日历月的月末。`chart.set_clock(() => utc_seconds)` 和 `offscreen_chart.set_clock(...)` 替换 `Date.now()` 用于倒计时跳动；`null`（或抛出异常或返回非有限值的时钟）回退到系统时钟。Rust 宿主用 `ChartEngine::set_now_seconds` 固定时钟，`ChartEngine::countdown_shown()` 报告在已固定的时钟下是否有倒计时行显示；GPUI 适配器在每次 prepaint 时固定系统时钟，并在其成立期间每秒重绘一次（见 [GPUI 刷新契约](rust.md#gpui-刷新契约)）。

**交易时段高亮**。`create_session_highlighting(series, { start_hour, end_hour })` 接受交易所本地时间的小数小时（`9.5` 即 09:30；`start_hour > end_hour` 时跨越午夜）；两者必须同时设置或同时不设置。`start_hour_utc`/`end_hour_utc` 仍是已弃用的别名（在默认 UTC 图表上结果相同）。回调重载仅对由实时 `update` 追加的行求值（`max_points` 保留策略淘汰最旧行时同样如此），并且仅在整体替换或历史在底层发生变化时才对整个系列重新求值。Rust 的 `SessionHighlightingOptions` 字段为 `start_hour`/`end_hour: Option<f64>`。

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

**窗口与较短的最后一根柱。** `windows` 是交易所本地的 `["HH:MM", "HH:MM"]` 对，按图表的 `time_zone` 和 `session_start` 放置，与 `session_slot_times` 完全一致（至多 32 个；结束时间不晚于开始时间表示跨越午夜，`"24:00"` 表示在午夜结束）。它们使窗口的最后一根柱精确结束：US 09:30–16:00 的小时线交易时段有一根较短的 15:30 柱，在 DST 变更前后都打印 16:00，HK 09:30–12:00 的上午窗口，其 11:30 的柱打印 12:00。没有 windows 时，较短的最后一根柱打印其开盘时间加间隔（16:30）。开盘时间不在任何窗口内的柱同样打印其开盘时间加间隔，这就是宿主提供的 241 根柱数据（将集合竞价行前移一个间隔至 09:29）在没有引擎构建的集合竞价柱的情况下，打印为 09:30、09:31 … 15:00 的原因。windows 与 `session_start` 必须相互匹配：当安装了带 windows 的标签时，若某个 `session_start`（通过 `apply_options`、`timeScale.sessionStart`、V2 导入或 `set_session_start_seconds`）使这些窗口无法被放置，则会抛出 `invalid_options`（Rust 中为 `ExchangeTimeError::BarTimeLabelWindows`）且不做任何更改，因此已保存的文档始终可以再次导入。

要同时移动两者，请在一次 `apply_options` 调用中一并发送（标签依据该次调用的起点校验），或先将标签设为 `"open"`。`time_zone` 变更绝不会与 windows 冲突；仅当某个时刻的窗口被 DST 切换折叠时，该时刻才打印开盘时间加间隔。同花顺和富途如何标注较短的最后一根小时线柱，未能验证（没有可用的实时终端），因此在依赖它之前，请将打印出的窗口结束时间与你的参考终端比对。

**不适用的场景。** 日历日期坐标轴和非时间柱序列（成交笔数流、成交量流和区间流，合成柱）打印它们自己的时间，并忽略该选项。间隔柱不会多出单独的第 241 根集合竞价柱：引擎构建的 K 线将 09:25 的集合竞价成交并入第一根柱，正如由 Tick 构建的 K 线已经做的那样；而分时折线的 241 个点仍然是按收盘时间戳标注的时刻槽位的属性（`session_slot_times` 与 `"bar_close_with_open"`）。

标签存放在选项存储中，因此只要它不是 `"open"`，V2 持久化就会携带它；导入不带该键的文档会保留已安装的标签（若文档的 `sessionStart` 与已安装的 windows 不匹配，则整份文档被拒绝），而默认图表的文档保持不变。Rust 宿主调用 `ChartEngine::set_bar_time_label(BarTimeLabel::Close { interval_seconds, windows })`、`bar_time_label()` 和 `bar_label_time(open_time)`（柱所打印的时刻）；引擎 JSON 选项为 `timeScale.barTimeLabel`，WASM 导出为 `bar_label_time(seconds)`。
