# Public API and compatibility policy

## Supported product surface

The supported product is the pre-1.0 browser package `@aeristerminal/aeris-charts`. Its framework-neutral
root ESM entry point, optional `./react` adapter, `./wasm` asset, and `./design.css` stylesheet are the
supported npm export paths. React is an optional peer dependency and is not loaded by root consumers.
The supported root surface is:

- chart creation and initialization;
- chart, series, time-scale, pane, price-scale, price-line, and drawing handles declared in
  `packages/charts/src/types.ts`;
- pane-local named price scales through `chart.add_price_scale()`, `chart.price_scales()`,
  `chart.move_price_scale()`, `chart.remove_price_scale()`, arbitrary string IDs in
  `chart.price_scale()`/`pane.price_scale()`, and series scale identity/rebinding;
- price-axis semantics: tick labels always lie on the series `min_move` grid; a built-in
  `price_format` naming `min_move` without `precision` derives the precision (reference
  `precisionByMinMove`); `price_format.tick_ladder` price bands (exchange spread tables such as the
  HKEX table) round every label to its band tick with per-band precision, keep axis ticks on the
  common grid of the visible bands, and drive trading snapping on that series' scale; the series
  option `autoscale_info_provider` (reference `autoscaleInfoProvider`) replaces a series' autoscale
  info (it runs during rendering: a chart API called from inside it throws `unsupported_operation`
  without touching the chart, and a provider that throws is ignored for that pass); price-scale
  options `tick_mark_density` and `ensure_edge_tick_marks_visible` follow the
  reference, and the Aeris extensions `base_value` (explicit percentage/indexed base shared with
  drawings), `autoscale_center` (symmetric autoscale), and `stable_auto_scale` (opt-in hysteresis;
  the default stays reference-exact) are ordinary `price_scale_options`; malformed ladders and
  extension values throw `invalid_options` and leave the price format or scale unchanged. Indexed-to-100 labels use
  the reference fixed two-decimal formatter;
- the intraday (分时) building blocks described under [Intraday (分时) charts](#intraday-分时-charts):
  `session_slot_times()`, explicit time-axis `tick_marks`, the `histogram_updown_rule`
  previous-close volume tint with host `up_color`/`down_color`, and a baseline series that shows
  its first traded bar;
- built-in series, indicators, drawing kinds, options, themes, data ingestion, interactions,
  subscriptions, screenshots, and lifecycle operations declared by those handles;
- Long Position and Short Position drawings through the canonical `drawing_kind` values
  `"long_position"` and `"short_position"`; each stores three editable anchors in entry, target,
  stop order, paints target/entry/stop information, projects all three prices onto the owning Y-axis,
  and uses the shared drawing history, persistence, hit testing, and backend frame path;
- visible-range volume profiles through `chart.add_volume_profile(prices, volume, options)`,
  returning a distribution handle with `options()`, `apply_options()`, `snapshot()` and `remove()`;
- first-class tick-driven footprint / numbers-bar series through `chart.add_series("footprint")`,
  including object and typed-column trade ingestion, explicit/quote/tick-rule aggressor handling,
  per-level Bid × Ask/total/delta, POC, final/Max/Min/session delta, configurable diagonal and
  stacked imbalances, density LOD, and derived bar/level queries; generic OHLC setters are rejected
  because they cannot supply order-flow truth;
- one chart-level replay clock plus shared canonical trade streams, typed batch ingestion, ordinary
  candle/bar bindings, exact seek-work telemetry, and fixed/ATR Renko, Line Break, Kagi, and Point &
  Figure transforms through `configure_synthetic_bar_series`, `set_synthetic_bar_source[_typed]`,
  and `update_synthetic_bar_source`; synthetic source/history remains host-owned and is not part of
  chart-state persistence;
- engine-resolved secondary-click context through `chart.subscribe_chart_context()`, including
  pane, time, logical index, coordinates, hit series, and the exact price on its scale; hosts own
  menus, clipboard operations, and order actions;
- chart-wide engine value queries through `chart.value_snapshot(logical_index?)`, including every
  live series' handle/ID, current kind, pane/scale placement, engine-owned exact or independently
  latest values, predecessor value, and formatted fields; `mouse_event_params.value_snapshot`
  carries the same records and restores latest values on crosshair leave while legacy `series_data`
  remains valued-only;
- additive complete indicator lineage metadata: stable binding ID, structured parameters, source and
  optional VWAP volume source, and stable output name/index/count, while legacy fields remain;
- indicator calculation conventions, KDJ, whitespace-safe indicator sources, the warm-up query, and
  the amount-weighted average price described under [Indicator conventions](#indicator-conventions);
- the five-output EMA ribbon through `chart.add_ema_ribbon()`, defaulting to periods
  `5/10/20/50/200` and colors `#335cff/#FF9800/#7d52f4/#fb4ba3/#fb3748`, plus atomic in-place
  period changes through `chart.set_ema_ribbon_periods()`;
- first-party broker-neutral trading state, instant/manual confirmation, previews, hit testing,
  semantic style, and typed intent subscriptions exposed by `chart.trading()`;
- host-authoritative alert-line indicators and crosshair plus-chip creation requests exposed by
  `chart.alerts()`; conditions/frequencies are retained configuration metadata while the host owns
  dialogs, evaluation, persistence, limits, expiration, background delivery, and notifications;
- default chart accessibility, its additive `chart.accessibility()` singleton handle, compatibility
  `enable_accessibility()`, accessibility options, and keyboard data/drawing operation;
- the additive `wheel_behavior` chart option (`auto`, `pan`, or `zoom`); existing gesture option
  names remain compatible;
- the time-scale viewport contract: data updates (history prepends, out-of-order inserts, gap
  backfills, retention trims) never move a scrolled-back view while the live edge follows new bars
  per `shift_visible_range_on_new_bar`; `set_visible_logical_range()` keeps fractional borders;
  `scroll_to_real_time()` animates to the configured `right_offset`; keyboard time-scale motion
  honors `handle_scroll`/`handle_scale`; and visible-range subscribers always end on the final range
  after a handler mutates data synchronously;
- the additive `lock_visible_logical_range` time-scale option (default `false`) for fixed
  full-session views such as intraday time-sharing charts: install every session slot as whitespace,
  call `set_visible_logical_range({ from: 0, to: slots - 1 })`, and the range stays exact from the
  pre-open state through the close, across data updates and resizes;
- `AerisChartsError` and its machine-readable error codes;
- chart-state persistence V1 through `chart.export_state()` and `chart.import_state()`.
- camel-case aliases for the common JavaScript lifecycle (`createChart`, `initWasm`, chart/series/scale
  creation and data methods) while every existing snake-case entry remains supported on the same handles;
- canonical presentation reset through `chart.reset_style_to_defaults()`. It restores Aeris-owned
  chart and series visual defaults for the chart's selected theme, including semantic unset/follow
  states, while preserving data, panes, drawings, indicators, series visibility/metadata, price
  formatting, scale bindings and scale/view state. It is deliberately separate from
  `chart.reset_view()`, which changes time/price scale view state.
- read-only backend diagnostics through `chart.backend_status()`, including the requested and active
  backend, stable fallback stage/reason, secure-context and `navigator.gpu` exposure, and optional
  unstable platform detail. `chart.backend()` retains its existing active-backend return value.

The `./react` entry exports `AerisChart`, `FinancialSeries`, `GeneralPane`, and `useAerisChart` plus
their configuration types. It is an authoring adapter over the root imperative API: ordinary rerenders
retain chart/series identities, data changes mutate those existing handles, structural general-axis or
series changes replace only the affected engine objects, and unmount uses the canonical disposal path.
It does not define chart semantics independently of the Rust engine. Importing the module is SSR-safe;
DOM/WASM chart creation starts from the mounted component effect. `FinancialSeries` streams: a new
`data` array that differs from the previously applied one only by a replaced last point and/or
ascending appended points (recognized by identity or shallow equality, at most 1,024 changed points)
is applied through `series.update()`; any other change is one `setData`.

### Volume profile

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

The price source must initially be a candlestick/bar series and volume a scalar series from the
same chart. Volume is matched by exact timestamp; missing, whitespace, nonfinite, zero and negative
volume contribute nothing. Each valid bar's volume is distributed uniformly over its high/low
interval and classified as bullish when close is at or above open, otherwise bearish. Each row
stacks the green bullish and red bearish shares and ends flush at the pane's right edge. Flat bars
contribute to one bin. This OHLCV estimate is not exact traded volume at each price, buy/sell volume,
profit/loss, or order flow. Demo volume is synthetic.

POC is the center of the largest-volume bin (lowest price wins ties) and uses one solid marker. The
contiguous value area expands from POC toward the larger adjacent bin, choosing the lower bin on ties,
until it reaches the requested fraction; stronger row colors show membership without boundary lines.
Rows are limited to 1–512, area to >0–100%, width to >0–50% of the pane, and
live profiles to 16 per chart. `snapshot().error` reports unrepresentable arithmetic; empty/missing
volume produces empty rows and null levels. Invalid option updates leave the prior options intact.

Profiles follow the source pane/scale and current visible time range. Source data updates and range
changes recompute bins; color/width changes and pointer movement reuse them. Removing either source
invalidates the handle. Profiles are runtime-only distributions, not scalar output series, and are
not included in V1 state exports; recreate their bindings after restoring the source data.
The older `create_volume_profile()` helper remains a drawing of caller-supplied bins.

Generated `wasm-bindgen` classes, methods reachable only through implementation objects, telemetry,
benchmark counters, demo globals, fixtures, and test hooks are internal even when JavaScript can
inspect them at runtime. `drawings_json()` is an internal inspection shape, not persistence.

`value_snapshot()` performs no history export. With no argument it resolves each engine-owned
series' own latest non-whitespace logical index/time. With an integer argument it performs exact
merged-logical lookup; every live series remains in the array, and a missing or whitespace value has
null data rather than borrowing a neighbor. OHLC series populate `open`, `high`, `low`, and `close`;
scalar series populate `value`; `previous_value` is the prior same-series non-whitespace close/value.
Matching formatted fields use that series' current formatter. Host applications remain responsible
for symbol/exchange metadata, volume-series association outside a VWAP binding, bar/day changes,
session calendars, visibility settings, and legend DOM.

Engine-owned advanced feature series expose their documented scalar price projection through
`value`, preserving the legacy scalar `series_data` shape. Experimental custom-series values are
computed by arbitrary host callbacks during rendering rather than stored as canonical engine data.
Their snapshot record is therefore null for exact-index queries and until a frame records a value;
latest mode exposes only the most recently recorded visible-frame value and may remain stale while
the series is not rendered.

The declaration manifest at `packages/charts/api/public-api-v1.json` records every supported
declaration file. CI runs `npm run check:api`; after deliberate review, update it with
`npm run update:api`.

Grid lines are engine-owned and default to visible dashed lines. Demo hosts may hide grid
visibility without replacing the canonical grid style/color; that presentation choice is not a
library default.

`chart.reset_style_to_defaults()` is the canonical host action for returning presentation to shipped
Aeris defaults. It does not reconstruct defaults from `options()` output: the engine restores
semantic follow states such as unpinned series colors and price-scale text. Watermark content and
visibility, scale modes/ranges/margins/layout constraints, viewport zoom/scroll, and indicator/data
semantics survive the reset; only their engine-owned visual styling is restored.

Default mouse-wheel behavior is informed by measurements from the pinned public reference fixture:
a saturated vertical step uses a 1.0 zoom increment, smaller trackpad deltas stay proportional, and the logical point under
the cursor remains anchored because `right_bar_stays_on_scroll` defaults to `false`. Vertical and
horizontal deltas independently zoom and pan the time scale on the pane, time axis, or price axis;
Ctrl and Shift do not change routing. `wheel_behavior: "pan"` and `"zoom"` are explicit Aeris
extensions; explicit zoom retains price-axis wheel zoom and focused Ctrl zoom.

The built-in series live-price line is engine-owned. `price_line_extent` defaults to `"partial"`
(tracked bar/value to the pane's right edge); `"full"` preserves the conventional pane-wide line.
Both extents use the same `price_line_source`, color, width, and line-style options, including solid,
dotted, and dashed modes. Indicator outputs inherit the same default because they are ordinary engine
series. `create_partial_price_line()` remains only as a compatibility controller over these canonical
series options, not a separate primitive or rendering implementation.

Chart surfaces do not inject product attribution or branding. `layout` contains only visual chart
configuration; there is no attribution-logo option in the public chart contract.

`create_chart(container, { initialPane: { horizontal_domain } })` constructs the canonical first
pane with continuous, temporal, category, or polar semantics. General-first charts contain no hidden
financial series and need no add-then-remove cleanup. Omitting `initialPane` retains the compatible
financial-time pane and primary candlestick series. Invalid creation options remove every canvas the
attempt installed before rejecting.

General axis and series handles expose `set_visible(boolean)` and `setVisible(boolean)`. Visibility
changes preserve the handle and its data while the engine updates domains, interaction snapshots,
legends, persistence, and the next rendered frame.

Numeric and temporal general-axis handles expose `pan`, `zoom`, and `reset_view`/`resetView`.
Temporal anchors use whole JavaScript-safe epoch milliseconds, and temporal ticks are selected and
formatted as bounded UTC intervals by the engine.

Category general-axis handles use the same view lifecycle. `zoom` anchors on a visible category
identity, `pan` shifts by a fraction of the visible category window, and `reset_view` restores the
configured or automatic category registry. Automatic registry changes clamp the retained index
window instead of keeping stale category strings.

Executable Cartesian general axes accept bounded typed explicit ticks with optional portable labels. Explicit numeric,
temporal, and category values drive the same labels and grid coordinates in Canvas2D, WebGPU, GPUI,
native frames, persistence, and screenshots; omitted labels use the built-in formatter. Explicit
ticks outside the current view are clipped, and combining `ticks` with `tick_count` is rejected.

General-axis `grid_visible` projects tick rules into the clipped pane underlay and respects the
chart-wide horizontal or vertical grid setting. Numeric `zero_line` draws an independent solid rule
when zero is visible; shared pixel coordinates are deduplicated across multiple axes.

The same handles expose atomic `apply_options` / `applyOptions` methods for mutable axis configuration,
series presentation, and compatible general-series pane/axis rebinding. Invalid colors, tick policies,
grouping, stacking, or bindings leave the complete prior object unchanged. General series expose
`general_series_order(pane?)` and exact-permutation `set_general_series_order(...)`; this engine order is
shared by rendering, legends, hit testing, React keyed arrays, and persistence. React `GeneralPane` applies
these ordinary prop and order changes to existing handles and rolls back a newly acquired pane or series
when initial installation or a readiness callback fails. Removing an owned empty final pane retires its
stable identity and leaves one fresh default layout slot, so React cleanup does not create a temporary pane.

### Indicator conventions

Built-in studies default to the TradingView/TA-Lib definitions. The optional last `parameters`
argument of `add_ema`, `add_dema`, `add_tema`, `add_rsi`, `add_rsi_with_source`, `add_macd`,
`add_bollinger`, and `add_bollinger_with_source` selects the other widespread convention:

| Parameter | Default (`convention: "tradingview"`) | `convention: "china"` (通达信/同花顺/富途) |
| --- | --- | --- |
| `seed` (EMA, DEMA, TEMA, MACD, RSI) | `"sma"`: mean of the first N samples; EMA N starts at bar N-1, RSI N at bar N, MACD 12/26/9 at bar 25/33 | `"first_value"`: `Y0 = X0`, so values start at bar 0 (RSI at bar 1) |
| `histogram_multiplier` (MACD) | `1`: `MACD - signal` | `2`: `(DIF - DEA) * 2` |
| `estimator` (Bollinger) | `"population"` (divide by N) | `"sample"` (`STD`, divide by N-1) |
| `seed` (KDJ) | `"fifty"`: K and D start from the textbook 50 | `"first_value"`: `SMA(X,N,1)` starts at its first input, so the first K is the first RSV |

```ts
const [dif, dea, bars] = chart.add_macd(candles, 12, 26, 9, undefined, { convention: "china" });
const rsi6 = chart.add_rsi(candles, 6, undefined, { convention: "china" });
const boll = chart.add_bollinger(candles, 20, 2, undefined, { convention: "china", estimator: "population" });
const [k, d, j] = chart.add_kdj(candles, 9, 3, 3);
```

The preset only fills parameters; explicit fields override it, `indicator_info().parameters` reports
the expanded `seed`/`histogram_multiplier`/`estimator` values, and chart-state persistence stores those
explicit values, never the preset name. Documents written before these parameters existed restore the
TradingView defaults. Rust hosts use `IndicatorKind::with_convention(IndicatorConvention::China)`.

`add_kdj(source, period = 9, k_smoothing = 3, d_smoothing = 3, options?, parameters?)` adds K, D,
and J in one oscillator pane: `RSV = (C - LLV(L, N)) / (HHV(H, N) - LLV(L, N)) * 100`,
`K = SMA(RSV, M1, 1)`, `D = SMA(K, M2, 1)`, `J = 3K - 2D`, where `SMA(X, N, 1)` is
`Y = (X + (N-1) * Y') / N`. The first value lands at bar N-1 either way; a flat window
(`HHV == LLV`) repeats the previous RSV (50 before the first) instead of producing NaN. `seed`
chooses the missing previous K and D:

- `"fifty"` (default): the textbook definition (百度百科/东方财富百科: "若无前一日K值与D值，则可分别用50来
  代替").
- `"first_value"` (`{ convention: "china" }` selects it): the 通达信/同花顺 formula language's
  `SMA(X,N,M)` semantics, which start at their first input (`Y0 = X0`), as the MyTT-style ports that
  reproduce those terminals do. The first K equals the first RSV and the first D the first K.

The two differ only near the start of the loaded history and agree after `convergence_bars` (26
bars for K and 44 for D/J at 9/3/3), so load that much history before the first visible bar when the
exact terminal value matters. Neither choice has been compared against a live terminal build here.
`indicator_info().parameters.kdj_seed` reports the choice, and V3 persistence stores it (documents
without it restore `"fifty"`).

`indicator_schema(kind)` has revision 2: convention parameters appear as `"choice"` parameters with a
`choices` list, and VWAP lists an optional `amount_source` series.

**Whitespace sources.** A whitespace row (`{ time }`) in an indicator source keeps its time slot but
never enters calculation state. Every study emits a whitespace output row there and continues exactly
as if the row did not exist: a suspended session or a pre-filled future slot does not reset or poison
EMA/RSI/MACD recursion, and window studies use the last N real bars. Filling a whitespace slot later
recalculates from that row.

**Warm-up query and history loading.** `indicator_info()` reports, for each output, `warmup_bars` (bars
of the root price source before the first value) and `convergence_bars` (bars of history after which
the value no longer depends on where loaded history begins: the warm-up for window studies, plus the
bars for every recursive seed's weight to fall below 0.1%). Both include chained sources, so an SMA of
an RSI reports the sum. `convergence_bars` is `null` when no bar count suffices: session VWAP and
pivots depend on a time anchor, OBV, Parabolic SAR, SuperTrend, and ZigZag on the whole path.

To show converged values from the first visible bar, request history before it:

```ts
const studies = [...chart.add_macd(candles, 12, 26, 9), chart.add_rsi(candles, 14)];
const needed = Math.max(...studies.map((series) => series.indicator_info()?.convergence_bars ?? 0));
// Host-owned market data: fetch `needed` extra bars before the first bar the user should see.
const history = await provider.bars({ to: first_visible_time, count: visible_bars + needed });
candles.set_data(history); // indicators rebuild from the new first bar
chart.time_scale().set_visible_logical_range({ from: needed, to: needed + visible_bars - 1 });
```

MACD 12/26/9 reports warm-up 33 and convergence 154 with the default seed; RSI 14 reports 14 and 108.
Series retention (`max_points`) must also keep at least that many bars, because trimming re-seeds
recursive studies at the new first bar.

**Average price (分时 均价).** `add_vwap(price, volume, options, { amount_source: turnover })` reports
`sum(amount) / sum(volume)` per VWAP reset period instead of weighting the typical price. Volume and
turnover align to the price rows by timestamp; minutes without positive volume or finite turnover, and
whitespace price rows, contribute nothing, and the line is blank until the period's first trade.
`indicator_info().amount_source` identifies the turnover series; removing it removes the study. The
reset period follows the VWAP session key.

## Time, exchange time zone, and trading sessions

Canonical chart time is whole UTC seconds. `business_day` values and strict `"YYYY-MM-DD"` strings
are taken at UTC midnight; values returned by the chart are always numeric UTC seconds. The
financial time axis is ordinal: bars are spaced by index, so weekends, holidays, lunch breaks, and
half days take no width. Hosts own exchange calendars and send only the bars that exist; holiday
calendars and multi-calendar overlay alignment are not engine features.

**Exchange time zone.** `chart.time_scale().apply_options({ time_zone, session_start })` or the
declarative chart option `timeScale: { timeZone, sessionStart }` (also accepted by worker charts)
sets how instants are grouped and labelled. `time_zone` is `"UTC"` (the default), an IANA name such
as `"Asia/Shanghai"` or `"America/New_York"`, or an explicit schedule of
`{ from_utc_seconds, offset_seconds }` transitions (strictly ascending, at most 1024, offsets within
±18 h; the first offset also applies before its entry). The package resolves an IANA name once per
zone with `Intl.DateTimeFormat` over 1970–2100 (at most ~262 DST transitions) and keeps at most 32
resolved zones; outside that span the nearest offset applies. The engine itself is platform-free,
accepts only explicit schedules, and never reads the browser's time zone. An unknown zone, malformed
schedule, or out-of-range session start throws `invalid_options` before any other key in the same
call is applied. `time_scale().options()` reports the IANA name (or the schedule) and
`session_start`. Rust hosts call `ChartEngine::set_time_zone(UtcOffsetSchedule)` and
`set_session_start_seconds(i32)`; the engine JSON options accept `timeScale.timeZone` (`"UTC"` or a
transition array) and `timeScale.sessionStart`, and V2 persistence round-trips both. Importing a
document that predates these keys keeps the chart's installed zone and session start.

**Trading day.** `session_start` is the offset in seconds from exchange-local midnight at which a
trading day begins (default `0`, range ±86 399). A negative value assigns an evening session to the
next trading day, e.g. `-3 * 3600` makes a 21:00 China futures night session start the next day;
with a negative start a day that would fall on Saturday or Sunday rolls forward to Monday, so a
Friday-night session belongs to Monday. Day/Month/Year tick marks, VWAP `session`/`weekly`/`monthly`
resets, and pivot sessions use trading days. Weekly periods start on Monday.

**What follows exchange time.** Tick boundaries (Day/Month/Year from trading days; hour and minute
marks on exchange wall-clock time, so they stay on exchange hours across DST and non-hour offsets),
built-in labels on every surface (axis ticks, crosshair, rectangle drawing axis tags, the delta
tooltip, `create_tooltip`, and accessibility text), VWAP and pivot resets, the session-highlighting
hour gate and weekend test, and the countdown window. Day-mark tick labels name the trading date;
crosshair and tooltip text show the instant's exchange wall-clock date and time.

**Calendar-date data.** When every financial series with data was given `business_day` or
`"YYYY-MM-DD"` times, the chart treats its time points as calendar dates: they keep their own date
in every zone and are never shifted by the time zone or `session_start`. One numeric time on any
financial series makes the time points instants again. Send daily and longer bars as calendar dates;
numeric UTC-midnight daily bars are instants and show the previous evening in zones west of UTC.
Typed column input is always numeric instants. Rust hosts declare the same state with
`ChartEngine::set_calendar_date_axis(bool)`.

**Formatter hooks.** `time_scale_options.tick_mark_formatter(time, tick_mark_type, locale, context)`
and `localization.time_formatter(time, context)` receive UTC seconds plus a `time_label_context`
whose `business_day` is the calendar date for calendar-date rows and `null` for instants. The host
`time_formatter` overrides every surface that prints a point in time: crosshair label, rectangle
axis tags, delta tooltip, `create_tooltip`, and accessibility (unless the accessibility options set
their own `time_formatter`). Without it, `create_tooltip` and accessibility format in the chart time
zone with `localization.locale`, adding the time of day for intraday rows. Rust `TickMarkFormatterFn`
and `TimeFormatterFn` signatures are unchanged.

**Countdown clock.** The candle-close countdown shows only while the clock is inside the forming
bar's interval `[last_bar_time, last_bar_time + bar_interval)`; outside it — lunch breaks,
overnight, weekends, after an early close — it hides instead of cycling. Calendar-date bars form
during the exchange trading day(s) of their date (with a negative `session_start`, Monday's bar
starts with Friday's night session), and bars 28 or more days apart run to the end of their
calendar month(s). `chart.set_clock(() => utc_seconds)` and `offscreen_chart.set_clock(...)`
replace `Date.now()` for countdown ticks; `null` (or a clock that throws or returns a non-finite
value) falls back to the system clock.

**Session highlighting.** `create_session_highlighting(series, { start_hour, end_hour })` accepts
fractional exchange-local hours (`9.5` is 09:30; `start_hour > end_hour` wraps midnight); both or
neither must be set. `start_hour_utc`/`end_hour_utc` remain deprecated aliases (identical on the
default UTC chart). The callback overload evaluates only rows appended by a live `update` (also
when `max_points` retention evicts the oldest rows) and re-evaluates the whole series only on full
replacement or when history changed underneath. The Rust `SessionHighlightingOptions` fields are
`start_hour`/`end_hour: Option<f64>`.

## Intraday (分时) charts

A time-sharing chart shows one trading session (or several) at fixed width from the open: every
minute of the session has a slot before it trades, the price is drawn against the previous close,
the average price is turnover over volume, and the view never scrolls. It is composed from ordinary
series and options; `examples/web_demo/intraday.html` is the complete reference host (one A-share
day in Asia/Shanghai, and a five-day variant with `?days=5`).

**1. Session slots.** `session_slot_times({ date, windows, interval_seconds, time_zone,
session_start?, convention? })` returns the UTC seconds of every bar of one trading date. `windows`
are exchange-local `["HH:MM", "HH:MM"]` pairs in chronological order (at most 32; an end at or before
its start crosses midnight, `"24:00"` ends at midnight), `time_zone` is an IANA name or an explicit
schedule, and the result is bounded to 100 000 slots and validated (`invalid_options`). Each window
converts with the offset in force on that date, so the same windows stay on exchange hours across
DST. With a negative `session_start`, windows starting at or after the session-start time of day
belong to the evening before (Friday evening for Monday), matching the chart's trading days and the
night sessions of Chinese futures. A market whose week reopens on Sunday evening (CME Globex)
requests that evening separately: the Sunday date with `session_start: 0` and
`[["17:00", "24:00"]]`, then the Monday date with its remaining windows. It
needs the engine module: call it after `init_wasm()` or `create_chart()`. Rust hosts call
`aeris_charts_engine::session_slot_times(day, &windows, interval, chart.exchange_time(), convention)`.

`convention` decides which instant names a slot. Aeris bars are stamped with their open time, so
the default `"bar_open"` gives 240 one-minute slots for an A-share day (09:30..11:29, 13:00..14:59).
同花顺 and 富途 instead label a minute by its close and show the opening-auction print as its own
first point: `"bar_close_with_open"` reproduces their 241 points (09:30, 09:31..11:30, 13:01..15:00);
`"bar_close"` is the close-labelled form without the opening point. Use the convention your data
provider stamps its minutes with. The lunch break takes no width either way: the last morning slot
and the first afternoon slot are neighbours.

**2. Reserve the session.** Install every slot, traded or not; untraded minutes are whitespace rows
(`{ time }`). Lock the whole session in view and disable gestures (keyboard motion follows the same
switches). Hold the range half a bar beyond the first and last slots: slot centres then sit half a
bar in from the pane edges, so the opening and closing columns stay inside the pane on narrow
screens instead of straddling its left edge, where the reference coordinate mapping puts an
unpadded first slot:

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

**3. Price against the previous close.** A baseline series with `baseline_value: prev_close` is red
above and green below (set `top_*`/`bottom_*` colors); with only the first minute traded it draws a
bar-wide segment. Center its scale on the previous close, and put the same prices on a second scale
in percentage mode based on the previous close:

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

Equal top and bottom margins put the previous close in the middle of the pane on both axes.

**4. Average price (均价).** Volume and turnover are separate series aligned by time; the average is
VWAP with a turnover source, reset each trading session (`session_start` defines the session):

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

Keep units consistent: with turnover in yuan, volume must be in shares (multiply lots by 100).

**5. Volume colors.** `histogram_updown_rule: "previous_close"` colors each column by the primary
price series' close against the previous traded close (an unchanged close counts as up); the first
traded minute compares with the previous close given as the primary's `baseline_value` (or its
scale's `base_value`). The primary is the first series added to the chart. `up_color`/`down_color`
replace the translucent market palette; the default `"open_close"` rule keeps the reference
behavior. As in the reference, a histogram's autoscale range always includes its `base` (0), so
column heights stay proportional to volume, the opening minute included.

**6. Session anchors on the time axis.** `tick_marks` replaces the automatic tick selection for the
axis labels and the vertical grid; marks on future whitespace slots work, `null` restores automatic
ticks, and labels default to the exchange-time label of their slot:

```ts
// slot("10:30"): the slot whose exchange wall-clock time is 10:30.
chart.time_scale().apply_options({ tick_marks: [
  { time: slot("09:30") }, { time: slot("10:30") },
  { time: slot("11:30"), label: "11:30/13:00" },
  { time: slot("14:00") }, { time: slot("15:00") },
] });
```

A mark whose time is not a slot (13:00 under the 241-point convention) draws nothing, so label the
junction explicitly. For several days, mark each day's first slot with a date label and install the
days' slots back to back; the marks' grid lines separate the days. Without labels, day-open marks
show the trading date in bold, the chart's first slot included once the chart spans several
trading days (a single-day chart's first mark shows its time). Explicit marks are at most 512,
strictly ascending, with labels of at most 64 bytes; labels stay inside the axis and a label that
would overlap its left neighbour is skipped (its grid line stays). Worker charts accept the same
list as `timeScale.tickMarks`, and V2 persistence carries it.

**7. Live minutes.** Fill the next slot with `update()` and refine the forming minute with `merge()`;
a `{ sequence }` makes each delivery idempotent (a stale or replayed one is rejected with
`stale_sequence`). Filling a whitespace slot never moves the locked view:

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

Before the first trade the time axis, its anchors, and the vertical grid already show the session,
while the price axes stay empty: as in the reference, a series without data does not autoscale.
The opening trade then draws at once (a baseline series with one traded row paints a bar-wide
segment). Open `intraday.html?traded=0` to see that state.

## Experimental surfaces

Custom series, pane/series/canvas primitives, the exported custom-series/primitive feature packs,
built-in plugin helpers, offscreen-worker charts, split-grid helpers, and shortcut helpers are
public experimental APIs. Their current lifecycle and containment behavior is tested, but their
exact types may change in a pre-1.0 minor release.
`create_delta_tooltip()` is intentionally unavailable on candlestick series; candles use the normal
hover `create_tooltip()` instead. Delta Tooltip remains available on non-candlestick series such as
area/line/bar and is composed directly into the brushable-area interaction. Brushable Area is a
composition over the ordinary `area` series rather than a separate data-bearing series kind; the
legacy `brushable_area` input spelling remains a compatibility alias that normalizes to `area`.
The helper reserves primary mouse/pen pane-drag for the comparison brush while it is attached; axis
drags keep their ordinary auto/manual-scale behavior and the helper does not globally disable chart
scroll or scale options. Removing the helper restores the ordinary Area pane-drag path immediately.
The normal `create_tooltip()` is the canonical structured market-data inspector. Its engine snapshot
retains Open/High/Low/Close for every ordinary series presentation, including area and line; scalar
rows naturally report the same value in all four fields, while area/line series fed retained OHLC
rows can render Close and still inspect the full bar. Hosts may bind an explicit `volume_series` to
add a timestamp-aligned Volume row; the chart never guesses which histogram represents volume.
Convenience helpers such as `create_rectangle_drawing()` and `create_rectangle_drawing_tool()` are
controllers over the canonical engine-owned drawing kind; they do not define a separate rectangle
feature or persistence identity. Likewise, primitive helpers whose visual shape resembles a drawing
remain primitives and should be presented as such by demos and hosts.
Extensions run at host render time, must not re-enter a chart mutation from a render callback, own
their external objects and persistence, and receive teardown exactly once. Callback failures are
contained at the host boundary so one extension cannot prevent other teardown. Arbitrary extension
objects or executable callbacks are never reconstructed from persisted JSON.

## Errors and lifecycle

Predictable failures throw `AerisChartsError`, an `Error` subclass with one of these stable
codes: `disposed`, `invalid_handle`, `stale_handle`, `invalid_data`, `invalid_options`,
`unsupported_operation`, `serialization_error`, `persistence_version_error`, `extension_error`,
`renderer_platform_error`, or `resource_limit`.

## Brand rename

The browser package is `@aeristerminal/aeris-charts` (with `@aeristerminal/aeris-charts/react`). The former branded error
exports were renamed to `AerisChartsError` and `AerisChartsErrorCode`; update imports and
`instanceof` checks when migrating. Rust consumers use the `aeris_charts_*` crates listed in
`Crates.md`.

Every former brand-bearing public identifier was hard renamed:

| Public surface | New identifier |
| --- | --- |
| Browser error class | `AerisChartsError` |
| Browser error-code type | `AerisChartsErrorCode` |
| React chart component | `AerisChart` |
| React chart props | `AerisChartProps` |
| React chart hook | `useAerisChart` |
| WebAssembly chart class | `AerisChart` |
| WebAssembly workspace class | `AerisWorkspace` |
| GPUI prepared-frame type | `PreparedAerisFrame` |
| GPUI viewport type | `AerisViewport` |

Persisted chart and workspace schema identifiers, browser event names, generated WebAssembly
asset names, DOM IDs/classes, CSS selectors, and benchmark environment variables now use the
`aeris_charts` prefix. There are no compatibility aliases for the retired brand.

`chart.remove()` is idempotent. Every operation that needs live chart state throws `disposed`
after removal. Identity fields already held by the caller may still be read. Removed series,
drawings, panes, and price scales throw `stale_handle`; a stale handle never targets a replacement.
Extension cleanup exceptions remain contained and are reported as development warnings.

Named price-scale creation rejects empty/reserved/duplicate/overlong IDs, missing panes, and the
per-pane resource limit without partial mutation. Rebinding to an unknown scale throws
`invalid_options`; removing a built-in or populated scale throws `unsupported_operation`. Named
scale IDs are case-sensitive, pane-local UTF-8 strings of 1-128 bytes, with at most 16 per pane.

Clean batch/current-bar ingestion retains its allocation-free/null diagnostic path. Repaired,
dropped, reordered, deduplicated, rejected, or semantically anomalous input is available through
`series.last_ingestion_diagnostics()`; OHLC anomalies are reported without rewriting values.
Numeric times must be finite whole UTC seconds in the inclusive range
`-62167219200..253402300799` (years 0000..9999) and are never auto-converted. Rejection reasons
suggest milliseconds, microseconds, or nanoseconds when scaling would produce an in-range value.
Any invalid timestamp rejects a direct set/update batch atomically, and an invalid single update
leaves the current series unchanged and warns on the console, as `update_typed` does. Shared-ring
drains reject malformed rows individually and count them in `frame_stats().ring_dropped_rows`.
Worker charts expose the most recent result through `offscreen_chart.last_ingestion_diagnostics()`.

Streaming ingestion keeps reference `series.update` semantics: a point replaces the whole bar at its
time. `update()` reports the payloads that silently rewrite a bar with a machine-readable
diagnostics `code` pointing to `merge()`: `value_on_ohlc_series` (`{ time, value }` flattens a
candlestick/bar), `price_less_payload` (for example `{ time, volume }` becomes whitespace), and the
rejected `partial_ohlc`. `series.merge(point, options?)` is the engine-owned partial path: present
open/high/low/close/value fields overwrite, absent fields keep the existing bar, and candlestick/bar
results are normalized so `high >= max(open, close)` and `low <= min(open, close)` (a close-only tick
for a new time creates O=H=L=C; scalar series take `value`). A merge without a price field is
rejected with `empty_merge`; volume and turnover merge into their own series. `series.merge_typed(columns,
options?)` is the columnar form: row `i` merges like `merge()` with `NaN` entries and omitted columns
absent, rows apply in input order with one engine synchronization, and one invalid row rejects the
batch (`offscreen_chart.merge_typed` is the worker form). Custom, advanced, and footprint series throw
`unsupported_operation`. A streamed point's explicit `color`/`wick_color`/`border_color` colors that bar
even when no earlier point carried a color, exactly as the same item does in `set_data`.

`update`, `merge`, `update_typed`, and `merge_typed` (including the `offscreen_chart` typed forms) accept
`{ sequence }`, a non-negative safe integer; an invalid value is rejected and warned like invalid data. When present, a sequence not greater than the last one
applied to that series is rejected as stale (`code: "stale_sequence"`, `last_sequence`) without
changing data or emitting `data_changed`; calls without a sequence always apply. A full
`set_data`/`set_data_typed` clears the guard or installs its `{ sequence }` as the baseline. The
guard is O(1) per series, runtime-only, and not persisted. Custom and advanced series throw
`unsupported_operation` when given a sequence. Rust hosts use `ChartEngine::merge_series_bar`,
`merge_series_bars`, `update_series_bar_sequenced`, `update_series_bars_sanitized_sequenced`, and
`set_series_update_sequence`.

## Drawing anchors, magnet, and price basis

Every drawing anchor has a time identity. `drawing.points()` returns `{logical, price, time}`,
where `time` is UTC seconds: fractional positions interpolate between the neighbouring bar times
and positions beyond the data extrapolate with the prevailing bar interval (the future-area
rectangle time tag shows that extrapolated date). `chart.add_drawing()` and `drawing.set_points()`
accept `{logical, price}`, `{time, price}`, or both; `time` wins when both are present and disagree,
and `points()` output round-trips exactly. A time anchor added before the chart has data stays
pending and resolves when data arrives. Non-time bar charts report no `time`.

When data changes, anchors follow the merged time axis. Timestamps shared by the old and new data
keep the exact mapping, and a same-interval retention trim or window shift keeps its bar-count
extrapolation. Prepended history re-places anchors left of the old data by their time, so a drawing
older than a short intraday history keeps its date while the host pages in more bars. When the
data shares no timestamps, or the interval changes (1m to 1h to 1D, a clear-then-set, a symbol
reload), each anchor resolves from its time on the new axis: 10:37
lands 37/60 of the way from the 10:00 hourly bar to the next one, and inside that day's bar on
daily data. Undo/redo history and in-flight creation or drag state follow the same rules. Sync
payloads (`drawing_sync_payload`) and clipboard payloads (`copy_drawings`) carry anchor times, so the
receiving chart resolves them on its own interval and history window.

`chart.set_drawing_magnet_mode("off" | "weak" | "strong")` is the persistent toolbar magnet
(default `"off"`, which keeps the historical Ctrl/Cmd-only magnet). `"strong"` always snaps a placed
or edited anchor to the nearest rendered OHLC value of the bar under the pointer; `"weak"` snaps only
within 12 CSS px (`DRAWING_WEAK_MAGNET_DISTANCE`). A drawing's own `magnet` option raises the mode for
that drawing. Holding Ctrl/Cmd toggles the effective magnet (inactive becomes strong, active becomes
off). Touch input has no modifier and uses the chart mode. Keyboard nudges never snap.

`chart.add_drawing()` throws `invalid_options` for a malformed or out-of-range options patch instead of
dropping the options. Undo/redo during an active drag cancels the drag first. Keyboard editing
(`Enter`, `Tab`, arrows) cycles the drawing's editable handles (every anchor, a rectangle's eight
bounds handles, or a Long/Short Position's target, entry, width, and stop controls) and moves the
focused handle by the nudge distance.

### Price-basis (复权) switches

The engine has no adjustment-factor model; the host computes adjusted OHLC. Drawings follow a basis
switch through three calls:

1. Replace the series data in the new basis (`series.set_data()`); indicators recompute.
2. Call `chart.rescale_drawing_prices(segments, basis_label)`. Each segment is
   `{from_time?, to_time?, factor}` (UTC seconds, `[from, to)`, non-overlapping, factor 1e-6..1e6).
   Each anchor price whose time falls in a segment is multiplied by that factor (on tick, volume,
   or range bar charts an anchor's time is the open time of the bar it sits on); Long/Short
   Position levels use the entry anchor's segment. This is a data-basis change, not an edit: it
   also applies to locked drawings, rewrites the undo/redo history in the new basis, and records no
   undo step, so undo never restores old-basis prices. The rescale is atomic: invalid segments, or
   a factor that would move any price outside the supported value range, change nothing. The
   label argument sets the basis in the same step. Position progress re-evaluates against the new
   candles.
3. Keep `chart.drawing_price_basis()` in sync (`set_drawing_price_basis()` also sets it on its own).
   The label is persisted and carried by sync and clipboard payloads. After a restore or sync, compare
   it with the data basis and rescale when they differ.

For a 前复权 ↔ 不复权 switch, the segments are the ex-date intervals with the cumulative factor of
each interval (for example `{to_time: ex_date, factor: 0.5}` after a 2-for-1 split).
`chart.set_drawings_points([{drawing, points}])` rewrites the anchors of many drawings atomically as
one undo step, for host-computed edits. Price lines, alerts, markers, and trading objects stay
host-owned; the host rewrites them itself. Trading intents emitted from the chart (bracket orders from
Long/Short Positions, order drags) carry display-basis prices, so under a non-raw basis the host must
convert them to raw prices before it submits them to a broker.

## Persistence V1

Persistence schema versioning is independent of the npm package version. V1 contains only:

- ordered pane identities, stretch factors, and preserve-empty flags;
- ordered built-in drawings with persistent ID, kind, pane reference, semantic anchors
  (`{logical, price, time?}`, plus the `anchor_times_micros` sidecar on non-time bar charts), and
  style;
- the optional top-level `drawing_price_basis` label (the host-defined price basis of the drawing
  prices).

Host market history, series and indicator definitions, chart options, trading positions/orders/
executions/previews/intents, alert lines/create requests, custom extensions, callbacks,
subscriptions, selections, interaction sessions, generations, LOD, drawing bounds/indexes,
retained frames, and GPU resources are not persisted. Hosts restore V1 into a fresh chart, then
reinstall host-owned data, series/indicator configuration, trading state, alert state, options, and
extensions.

Named price-scale descriptors and series-to-scale bindings are also host-owned. Hosts recreate
named scales in each pane before restoring comparison-series bindings; chart-state V1 is unchanged.

Import checks the whole document before mutation and installs it as one transaction. Imported panes
receive fresh live handle IDs while retaining separate persistent pane IDs; pre-import pane and
price-scale handles therefore become stale. Import is accepted only before drawing IDs have been
issued and while the chart still has its initial pane topology. This prevents an old drawing handle
from retargeting a restored drawing with the same persistent ID.

Drawing anchors are stored as `{logical, price, time?}` plus the optional top-level
`drawing_price_basis` label; both fields are optional and need no schema version change. On an
ordinary time chart `time` is authoritative on restore: importing after data resolves each anchor
by its time on the loaded window, and importing before data (the grid workspace order) keeps the
anchors pending by time until the host installs data. Documents without anchor times keep their
logical anchors. Non-time (tick/volume/range) charts keep using `anchor_times_micros`.

Limits for untrusted input are 8 MiB per document, 64 panes, 10,000 drawings, 100,000 anchors per
drawing, 250,000 total anchors, 64 KiB text per drawing, and 1 MiB total drawing text. Unknown
optional V1 fields are ignored. Unknown schema versions, drawing kinds, pane references, duplicate
IDs, invalid anchor counts, non-finite/unsafe numbers, and limit violations fail structurally and
leave the chart unchanged. V1 fixtures are compatibility inputs; future versions must retain an
explicit V1 migration path for the documented compatibility window.

## Persistence V3 studies

Financial charts with engine-owned indicators export schema version 3. V3 keeps market history and
ordinary series data host-owned, but persists ordered study bindings, scalar input selection, typed
indicator parameters (including explicit seed, histogram, and estimator values), timestamp-aligned
volume- and turnover-source references, and per-output styles. Chained
sources are encoded as references to an earlier study output so restore does not depend on old live
series identities. Hosts must recreate the source series and their data before importing V3; import
validates every dependency, parameter, output-style count, and resource limit before mutation.
V1 and V2 documents remain accepted unchanged, and a V3 document restores into a fresh financial
chart only.

## Version policy

While the browser package is below 1.0:

- patch: compatible correctness, security, performance, documentation, and packaging fixes;
- minor: additive stable API, explicitly reviewed experimental API changes, or compatible behavior
  additions;
- major (including the eventual 1.0 boundary): removal/rename/signature change to stable API,
  incompatible stable behavior, or ending a documented persistence compatibility window.

Adding a drawing/series kind is normally a minor package feature, but changing the meaning of an
existing kind is incompatible. Adding a new persistence schema does not invalidate V1; removing V1
support follows the separately documented persistence window and is a major compatibility event.

## Rust distribution

The Rust crates are prepared as one coordinated release family. Version `0.3.0`
publishes `aeris_charts_core`, `aeris_charts_indicators`, `aeris_charts_render`,
`aeris_charts_engine`, `aeris_charts_render_wgpu`, `aeris_charts_native`, and
`aeris_charts_wasm`. Workspace manifests retain local path dependencies with the same explicit
version, so repository builds exercise the same dependency boundaries used by registry consumers.

The Rust API is below 1.0 and may evolve between minor releases. Patch releases preserve the public
API within their minor line except where a correctness or security repair cannot do so safely; minor
releases may add, change, or remove pre-1.0 Rust APIs. All published Aeris crates in one release use
the same version, and consumers should keep direct Aeris dependencies aligned.

`aeris_charts_render_gpui` remains repository-only and experimental because it tracks a reviewed
Zed Git revision whose API differs from the crates.io `gpui` release. Exact Git revisions are
required for that backend; floating Git dependencies are unsupported.

## Release policy

Tag publication depends on required Rust, package, and portable Chromium/Firefox/WebKit jobs. It also
checks persistence fixtures, public declarations, Node/SSR import, package contents, and configured
performance budgets. `continue-on-error` is forbidden for portable correctness.

Hardware- and machine-sensitive screenshot hashes, GPU timing, heap sampling, and wall-clock evidence
are calibration diagnostics. They remain non-authoritative and may not be approved merely to make one
runner green. Shared draw-stream parity, clipping/order/frame-contract tests, replay determinism, and
portable browser behavior are the authoritative gates. Scenarios without a configured benchmark
budget remain explicitly report-only.
