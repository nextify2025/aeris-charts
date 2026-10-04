# 成交构柱与重采样 API

[文档导航](../README.md) · [架构总览](../Architecture.md) · [API 入口](README.md)

## Tick 转 K 线与重采样

两种做法都在图表的交易所时间中构建普通 K 线，因此应先设置交易所时区（夜盘还需设置 `session_start`）。K 线以其开盘时间作为时间戳，即 Aeris 的规范柱时间。若要改为输出每根 K 线的收盘时间（A 股分钟线的 09:31 … 15:00），请设置 `bar_time_label`（[收盘时间标签](coordinates-and-time.md#收盘时间标签)）：K 线、其成交量、回放、倒计时以及宿主传入或读回的所有时间仍保持以开盘时间为时间戳。

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

**源行。** 源行必须以柱开盘时间作为时间戳；位于所有边界之外的行会被省略，因此宿主在重采样之前，仍须通过减去一个间隔，将数据提供方的收盘时间戳（09:31 … 15:00）转换为开盘时间戳。若数据同时带有单独的开盘集合竞价分钟（241 根柱的数据把它标为 09:30，与收盘时间戳 09:31 并存），则必须在重采样之前将该行合并进第一分钟：若对其做平移，它会落在第一个窗口之前，并连同其成交量一起被省略。直接绘制而不做重采样的 241 根柱数据，则可以改为同样将集合竞价行前移：它落在 09:29，位于所有窗口之外，而 `bar_time_label` 将其打印为 09:30（参见 [收盘时间标签](coordinates-and-time.md#收盘时间标签)）。空白数据行（`session_slot_times` 预留）为其桶预留位置但不含价格：未成交的桶是空白柱，正在形成的桶截止于其最后一个已成交行。重采样需要时间轴：在坐标轴为非时间柱序列（成交笔数流、成交量流或区间流，合成柱）的图表上会被拒绝，并且这类序列不能加入已有重采样系列的图表。

**实时更新。** 对源系列或其成交量调用 `update`、`merge` 以及类型化批量，仅刷新受影响的尾部：派生柱中未变化的前缀被保留，尾部从第一个可能含有变化行的桶开始重建，因此实时的一分钟只会重读一个桶，绝不会重读历史，并且超出最后一个源行的边界（提前配置的日期）不产生任何开销。`pop`、源系列头部的保留上限裁剪、会丢失某根派生柱的向后回放，以及完整的 `set_data`，都会触发一次重建。`chart.resample_stats(target)` 报告 `rebuilds`、`tail_refreshes` 和 `rows_scanned`。在回放时钟下，正在形成的柱仅聚合时钟当时或之前的行。要到达已配置边界之后的日期，请再次调用 `configure_resampled_series`（一次重建）；也可以提前包含新日期，因为没有数据的日期不会产生柱。重新配置会保留绑定的源（换用其他源会抛出 `invalid_options`）。无论配置顺序如何，绑定都绝不链式连接：目标（或成交量目标）不得是另一个绑定的源（或成交量源）或输出，且绑定的成交量源不得是其自身的成交量目标；每种情况都会抛出 `invalid_options`（"resampling dependencies may not be chained or cyclic"）且不做任何更改。

目标由引擎持有：对其调用 `set_data`、`update`、`update_typed`、`merge` 和 `merge_typed` 会被拒绝（`last_ingestion_diagnostics()` 报告 `status: "rejected"` 及 `code: "derived_series"`）且不做任何更改，`pop` 记录同样的拒绝。目标不得是足迹图、绑定到成交流的 K 线、成交研究或合成柱系列（`invalid_options`）；不过，绑定到成交流的 K 线或成交量研究可以作为该绑定的源。移除绑定中的任一系列（源、成交量源或目标），会连同其目标系列一并移除该绑定，与指标输出相同。`chart.resampled_bars(target)` 返回派生柱，附带其 `session_id` 以及聚合的源行数量。Rust 宿主调用 `ChartEngine::configure_resampled_series(source, volume_source, target, volume_target, ResampleOptions { interval_seconds, boundaries })` 和 `aeris_charts_engine::resample_boundaries(&days, &windows, chart.exchange_time(), ResampleSpan::Window)`。
