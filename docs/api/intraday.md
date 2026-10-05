# 分时图接入

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

时间单位与时区契约见[坐标与时间](coordinates-and-time.md)，成交构柱见[聚合](aggregation.md)。

## 分时图

分时图在从开盘起的固定宽度内显示一个（或多个）交易时段：交易时段内的每一分钟在成交之前就有一个槽位，价格相对前收盘价绘制，均价为成交额除以成交量，并且视图从不滚动。它由普通系列和选项组合而成；`examples/web_demo/intraday.html` 是完整的参考宿主（一个 Asia/Shanghai 的 A 股交易日，以及通过 `?days=5` 得到的五日变体）。

**1. 交易时段槽位**。`session_slot_times({ date, windows, interval_seconds, time_zone, session_start?, convention? })` 返回某一个交易日期每根柱的 UTC 秒数。`windows` 是交易所本地的 `["HH:MM", "HH:MM"]` 数对，按时间顺序排列（至多 32 个；结束时间等于或早于开始时间表示跨越午夜，`"24:00"` 表示结束于午夜），`time_zone` 是 IANA 名称或显式时间表，结果上限为 100 000 个槽位并经过校验（`invalid_options`）。每个窗口使用该日期当日生效的偏移量进行转换，因此同一组窗口在跨越 DST 后仍保持在交易所交易时间内。`session_start` 是本次调用自带的，默认为 `0`；它不会从图表读取，因此中国期货宿主必须显式传入 `session_start: -10800`（若省略，21:00 的夜盘窗口会被放在日历日期（周一 21:00）而不是周五 21:00，且没有任何提示）。`session_start` 为负时，开始于交易时段起始时刻或其之后的窗口属于前一晚（周一对应周五晚间），与图表的交易日和中国期货夜盘一致。该放置方式假定一周在周五晚间开盘。

周日晚间重新开盘的市场（CME Globex）传入 `session_start: 0`，并对每个晚间日期各调用一次，使用 `windows: [["17:00", "16:00"]]`。该日期是时段开盘的那个晚上：周日日期放置周一的交易日（1380 个一分钟槽位，周日 17:00 至周一 16:00），周一日期放置周二的交易日，依此类推，因此在 `0` 下每次调用都以晚间的日历日期而不是交易日期为键。两次调用可得到相同的槽位：周日日期搭配 `[["17:00", "24:00"]]` 与 `session_start: 0`，然后周一日期搭配 `[["00:00", "16:00"]]`。在负的 `session_start` 下，不要给周一日期一个开始于 17:00 或之后的窗口：那会放置周五 17:00 至周六 16:00。哪些日期交易、节假日和提前收盘都是宿主的日历数据；为每个日期传入适用的窗口（没有夜盘的日期，其窗口列表中不含夜盘窗口）。它需要引擎模块：在 `init_wasm()` 或 `create_chart()` 之后调用。Rust 宿主调用 `aeris_charts_engine::session_slot_times(day, &windows, interval, chart.exchange_time(), convention)`。

`convention` 决定由哪个时刻来命名一个槽位。Aeris 的柱以开盘时间作为时间戳，因此默认的 `"bar_open"` 为 A 股交易日给出 240 个一分钟槽位（09:30..11:29、13:00..14:59）。同花顺和富途则以收盘时间标注一分钟，并把开盘集合竞价成交显示为其自己的第一个点：`"bar_close_with_open"` 复现它们的 241 个点（09:30、09:31..11:30、13:01..15:00）；`"bar_close"` 是不含开盘点的收盘标注形式。请使用数据提供方给分钟打时间戳所采用的约定。无论哪种方式，午休都不占宽度：上午最后一个槽位与下午第一个槽位相邻。

两类图表对这些约定的用法不同。按时刻采样的折线（经典分时价格线，每分钟末一个价格）使用以收盘为时间戳的槽位：其点就是那些时刻，241 个点是该折线自身的属性。区间柱（由 Tick 构建的 K 线或重采样的分钟）使用 `"bar_open"` 槽位和以开盘为时间戳的行，并通过 `bar_time_label` 选项（[收盘时间标签](coordinates-and-time.md#收盘时间标签)）输出其收盘时间；它们不会多出单独的第 241 根集合竞价柱，因为 09:25 的集合竞价成交并入第一根柱。

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

上下边距相等时，前收盘价在两个坐标轴上都位于窗格正中。`baseline_line_visible: true` 让基线系列在其解析出的基线处（固定了 `baseline_value` 时即前收盘价）自行绘制虚线参考线，无需宿主维护单独的价格线；示例保留 `create_price_line`，因为它还在左侧坐标轴上标出该价格。未固定 `baseline_value` 时，`baseline_mode: "close_before_visible_range"` 以窗口之前的最后一个收盘价为基线。两者见[基线参考线与基线模式](presentation.md#基线参考线与基线模式)。

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
