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
  previous-close volume tint with host `up_color`/`down_color`, a baseline series that shows
  its first traded bar, the additive `break_on_trading_day` line/area/baseline option (default
  `false`), and VWAP/pivot lines that restart at every reset; the close-time display label
  `time_scale_options.bar_time_label` prints bars by their close while they stay open-stamped
  (see [Close-time labels](#close-time-labels));
- multi-calendar overlays through the series options `time_alignment: "as_of"` and
  `as_of_max_staleness` (see [Multi-calendar overlays](#time-exchange-time-zone-and-trading-sessions));
- built-in series, indicators, drawing kinds, options, themes, data ingestion, interactions,
  subscriptions, screenshots, and lifecycle operations declared by those handles;
- Long Position and Short Position drawings through the canonical `drawing_kind` values
  `"long_position"` and `"short_position"`; each stores three editable anchors in entry, target,
  stop order, paints target/entry/stop information, projects all three prices onto the owning Y-axis,
  and uses the shared drawing history, persistence, hit testing, and backend frame path. The
  statistics use two persisted drawing options, `position_account_size` (a hypothetical balance,
  default 1,000) and `position_risk_percent` (the share of it risked at the stop, 0–100, default 25),
  independent of broker orders;
- measuring drawings through the canonical `drawing_kind` values `"price_range"`, `"date_range"`,
  and `"date_and_price_range"` (the earlier spelling `"date_price_range"` is still read on import,
  templates, clipboard, and sync, and never written); each stores an editable start and end anchor
  snapped to whole bars and price ticks, labels the signed price change, percentage, ticks (counted
  on the instrument tick or price-band ladder), bar count, and elapsed time (the `labels` option
  chooses the metrics), and paints in the drawing color;
- the Shift-click quick measure in the built-in pointer handling: Shift + press on empty chart
  space starts a transient date-and-price measurement that follows the pointer (drawing default
  color for a rise, market-down color for a fall), freezes on release after a drag or on the next
  click, and is dismissed by the following click or Escape. It is never a drawing, history entry,
  or persisted object;
- visible-range volume profiles through `chart.add_volume_profile(prices, volume, options)`,
  returning a distribution handle with `options()`, `apply_options()`, `snapshot()` and `remove()`;
- KLineChart's 27 indicator templates through `chart.add_klinechart_indicator(source, indicator,
  volume_source?, options?)`, described under [KLineChart indicators](#klinechart-indicators);
- first-class tick-driven footprint / numbers-bar series through `chart.add_series("footprint")`,
  including object and typed-column trade ingestion, explicit/quote/tick-rule aggressor handling,
  per-level Bid × Ask/total/delta, POC, final/Max/Min/session delta, configurable diagonal and
  stacked imbalances, density LOD, and derived bar/level queries; generic OHLC setters are rejected
  because they cannot supply order-flow truth;
- one chart-level replay clock plus shared canonical trade streams, typed batch ingestion, ordinary
  candle/bar bindings, exact seek-work telemetry, live-tip dependent work counters in
  `chart.trade_stream_stats()` (`dependent_rows_computed`, `bar_rows_projected`,
  `bubble_trades_scanned`, `bubble_markers_sized`), and fixed/ATR Renko, Line Break, Kagi, and Point &
  Figure transforms through `configure_synthetic_bar_series`, `set_synthetic_bar_source[_typed]`,
  and `update_synthetic_bar_source`; synthetic source/history remains host-owned and is not part of
  chart-state persistence;
- ordinary candles built from ticks and OHLCV resampling as described under
  [Ticks to candles and resampling](#ticks-to-candles-and-resampling): exchange-session anchored
  trade-stream time bars (`chart.set_trade_stream_sessions()`), the stream volume histogram
  (`chart.add_trade_volume_series()`), and engine resampling through
  `chart.configure_resampled_series()`, `chart.resampled_bars()`, `chart.resample_stats()`, and the
  session-derived `resample_boundaries()` helper; resampling configuration is runtime-only;
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
- the additive `crosshair.shadeRight` chart option (`{ visible, color }`, default off): a
  translucent veil over the pane region right of the hovered bar, described under
  [Crosshair shade](#crosshair-shade);
- the additive baseline-series options `baseline_mode` (`"visible_midpoint"` or
  `"close_before_visible_range"`), `baseline_line_visible`, `baseline_line_color`,
  `baseline_line_width`, `baseline_line_style`, and the read-only `series.baseline_price()`
  (`baselinePrice()`) query, described under [Baseline reference line](#baseline-reference-line);
- the additive per-series `live_bar_easing_ms` option (default `0` = off): a same-time replacement
  of the drawn last bar glides its displayed high/low/close toward the new values while every
  query keeps the real ones, described under [Live-bar easing](#live-bar-easing);
- the engine-owned timeline-mark lane exposed by `chart.timeline_marks()` (`set`, `state`,
  `set_visible`, `set_group_hidden`, `hidden_groups`, `hit_at`) and the
  `subscribe_timeline_mark_click` outcome carrying the resolved hit, described under
  [Timeline marks](#timeline-marks); hidden groups persist as the optional V1/V2/V3
  `hidden_mark_groups` field;
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
declaration file. CI runs `bun run check:api`; after deliberate review, update it with
`bun run update:api`.

Grid lines are engine-owned and default to visible dashed lines. Demo hosts may hide grid
visibility without replacing the canonical grid style/color; that presentation choice is not a
library default.

`chart.reset_style_to_defaults()` is the canonical host action for returning presentation to shipped
Aeris defaults. It does not reconstruct defaults from `options()` output: the engine restores
semantic follow states such as unpinned series colors and price-scale text. Watermark content and
visibility, scale modes/ranges/margins/layout constraints, viewport zoom/scroll, and indicator/data
semantics survive the reset; only their engine-owned visual styling is restored.

Default mouse-wheel zoom follows measurements of TradingView: a saturated vertical step changes bar
spacing by exactly 10% (smaller trackpad deltas stay proportional) and keeps the right edge pinned,
because `right_bar_stays_on_scroll` defaults to `true`: the gap after the latest bar stays constant
while history compresses or expands. Ctrl/Cmd + wheel, macOS trackpad pinch (delivered as Ctrl +
wheel), and touch pinch zoom around the pointer instead. Setting `right_bar_stays_on_scroll: false`
restores cursor-anchored ordinary zoom. Vertical and horizontal deltas independently zoom and pan
the time scale on the pane, time axis, or price axis; Shift does not change routing.
`wheel_behavior: "pan"` and `"zoom"` are explicit Aeris extensions; explicit zoom retains
price-axis wheel zoom.

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

| Parameter | Default (`convention: "tradingview"`) | `convention: "china"` (通达信/同花顺 formula language) |
| --- | --- | --- |
| `seed` (EMA, DEMA, TEMA, MACD, RSI) | `"sma"`: mean of the first N samples; EMA N starts at bar N-1, RSI N at bar N, MACD 12/26/9 at bar 25/33 | `"first_value"`: `Y0 = X0`, so values start at bar 0 (RSI at bar 1) |
| `histogram_multiplier` (MACD) | `1`: `MACD - signal` | `2`: `(DIF - DEA) * 2` |
| `estimator` (Bollinger) | `"population"` (divide by N) | `"sample"` (`STD`, divide by N-1) |
| `seed` (KDJ) | `"fifty"`: a full RSV window, then K and D start from the textbook 50 (first value at bar N-1) | `"first_value"`: RSV over the bars available while fewer than N exist, and `SMA(X,N,1)` starts at its first input, so K = D = J = RSV at bar 0 |

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
`Y = (X + (N-1) * Y') / N`. J is not clipped to 0–100. A flat window (`HHV == LLV`) repeats the
previous RSV (50 before the first) instead of producing NaN. `seed` chooses how the study starts:

- `"fifty"` (default): the textbook KDJ, which 通达信's own help calls "KDJ传统版": the first value
  waits for a full N-bar RSV window (bar N-1) and uses 50 for the missing previous K and D ("若无前一日
  K值与D值，则可分别用50来代替"; also 百度百科/东方财富百科).
- `"first_value"` (`{ convention: "china" }` selects it): the formula-language KDJ, which 通达信's help
  calls "KDJ普通版" (`RSV:=...; K:SMA(RSV,M1,1); D:SMA(K,M2,1); J:3*K-2*D`). While fewer than N bars
  exist, `HHV`/`LLV` take the bars available, so RSV exists from the first bar, and `SMA(X,N,M)`
  starts at its first input (`Y0 = X0`). K, D, and J all equal the first bar's RSV, and every bar
  has a value (`warmup_bars` 0).

The two differ only near the start of the loaded history. They agree after `convergence_bars` (26
bars for K and 44 for D/J at 9/3/3, the same for both seeds), so load that much history before the
first visible bar when the exact terminal value matters. For a newly listed instrument whose whole
history is loaded, `"first_value"` follows the formula from the first session, as 东方财富's and
新浪's web charts compute it (apart from flat windows, see below). Retention trims and prepended
history restart the partial window at the new first bar, exactly as if that history had been loaded.
`indicator_info().parameters.kdj_seed` reports the choice, and V3 persistence stores it (documents
without it restore `"fifty"`). A document saved with `"first_value"` before this rule was verified
restores the same parameter and now also shows values on the first N-1 bars.

**How the conventions were checked (2026-09-28).** No live 通达信, 同花顺, 东方财富, or 富途
terminal was available, so these results come from published definitions, the platforms' own web
chart code, and one platform's published values. The published values were compared outside the
repository and are not committed; `crates/aeris_charts_indicators/tests/platform_values.rs` pins the
verified rules on a deterministic synthetic series instead:

| Study | Evidence | Result |
| --- | --- | --- |
| KDJ `"fifty"` | 通达信 help "通达信指标公式算法释疑" (help.tdx.com.cn/gspt) describes KDJ传统版 with the 50 rule; textbook encyclopedias | Documentation only |
| KDJ `"first_value"` | 通达信 help: KDJ普通版 formula; function reference: `EMA` returns values before N bars (unlike `EXPMEMA`), and `TMA`/`AMA` start at X. Web chart code: 东方财富 (emcharts 3.18.1, quotekchart 1.0.6) computes RSV over `min(9, i + 1)` bars with K = D = J = RSV at bar 0; 新浪财经's formula runtime computes `HHV`/`LLV` over the bars available and starts `SMA` at `Y0 = X0`. Values: 雪球's server-computed KDJ for two A-shares from their listing day and the full histories of four older listings, compared out of tree | 雪球's RSV equals the available-bar RSV exactly from the third session, and the same rule reproduced all six histories to 5e-5. 雪球 starts K and D from 100 and clips J to 0–100, which no terminal formula does, so its first sessions differ; after `convergence_bars` they agree |
| MACD `{ convention: "china" }` | Same out-of-tree 雪球 comparison; 东方财富, 同花顺, and 新浪 web chart code | Exact from the first session: `EMA` from the first close, DIF = DEA = 0 on bar 0, `(DIF-DEA)*2` |
| BOLL `estimator` | Same out-of-tree 雪球 comparison; web chart code; 通达信 help: `BOLL` is `MA(C,M) ± 2*STD(C,M)`, and the function reference defines `STD` as the estimated (sample) σ and `STDP` as the population σ | Conflicting. 雪球, 东方财富 web, and 同花顺 web use the population σ (`"population"`, exact match with 雪球). `{ convention: "china" }` selects the sample σ of 通达信's `STD` definition, as 新浪's web chart does, but 通达信's own BOLL help page illustrates σ with 1/N, and published user comparisons disagree (a 2008 test found population, a 2018 formula with N-1 matched). Pass `estimator: "population"` to match 雪球 or 东方财富; the terminal estimator needs a live 通达信/同花顺 check |
| RSI `"first_value"` | Same out-of-tree 雪球 comparison; 东方财富 and 新浪 web chart code | Not matched near the start: those sources start `SMA` from 0, and 雪球 also measures the listing day's change from the issue price, while `"first_value"` starts at the first change. The gap shrinks as history grows (RSI6 on one listing: 0.65 at bar 39). Unchanged pending a terminal check |

Not verified: exact 通达信/同花顺/富途 terminal output (so the China preset is not claimed to match
富途), how terminals treat a flat window (东方财富's and 新浪's web charts use RSV 0 instead of
repeating the previous RSV; 通达信's formula help does not document division by zero), and 富途's web
chart, whose page is behind a bot challenge. 同花顺's legacy web charts use other KDJ starts (100 with
clipping, or a running mean for the first N bars) and are not treated as the terminal definition.

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

**Lines restart at resets.** Every VWAP line (typical-price or amount-weighted), each VWAP band
output, and each pivot level ends its line where its period resets: the first drawn row of a new
session (or week or month for VWAP bands) starts a new run, with no segment, fill, or hit area joining
it to the previous period. Periods follow the chart's exchange trading day (`time_zone`,
`session_start`), exactly as the values reset, after full installs and live updates alike. A period
whose only drawn row is its first draws a one-bar horizontal segment, so a session VWAP on daily
bars shows one short segment per bar. Ordinary line, area, and baseline series connect across days
unless `break_on_trading_day: true` asks them to break at each exchange trading day (on a non-time
bar axis such as Renko or tick bars, the day of each bar's open time); whitespace rows never break
a line.

### KLineChart indicators

`chart.add_klinechart_indicator(source, indicator, volume_source?, options?)` adds one of KLineChart's
27 indicator templates, with KLineChart's formulas and presentation, and returns one `series_api` per
output in output order. `indicator` is a `klinechart_indicator`: the template name in `indicator` plus
that template's parameters, all spelled out (nothing is defaulted, so a missing field is rejected):

```ts
const [dif, dea, histogram] = chart.add_klinechart_indicator(candles, { indicator: "macd", short: 12, long: 26, signal: 9 });
const [volume_bars, ma5, ma10] = chart.add_klinechart_indicator(candles, { indicator: "vol", periods: [5, 10] }, volume);
const [average_price] = chart.add_klinechart_indicator(turnover, { indicator: "avp" }, volume);
```

`options` (the last argument, after `volume_source`, as in `add_vwap`) is a `Partial<series_options>` applied to every output; style one output
through its returned handle. The 27 names are the `klinechart_indicator_name` union, price overlays first:
`ma`, `ema`, `sma`, `boll`, `sar`, `bbi`, `avp`, `vol`, `macd`, `kdj`, `rsi`, `bias`, `brar`, `cci`, `dmi`,
`cr`, `psy`, `dma`, `trix`, `obv`, `vr`, `wr`, `mtm`, `emv`, `roc`, `pvt`, `ao`. Each row below gives the
parameters at KLineChart's defaults, the outputs (`key`, drawn as a line unless noted), and what the
template needs:

| `indicator` | Parameters (KLineChart default) | Outputs | Needs |
| --- | --- | --- | --- |
| `ma` | `periods` `[5, 10, 30, 60]` | `ma1`..`ma4`, one per period | |
| `ema` | `periods` `[6, 12, 20]` | `ema1`..`ema3` | |
| `sma` | `period` 12, `weight` 2 | `sma` | |
| `boll` | `period` 20, `multiplier` 2 | `up`, `mid`, `dn` | |
| `sar` | `start` 2, `step` 2, `max` 20 (percent) | `sar` (dots) | |
| `bbi` | `periods` `[3, 6, 12, 24]`, exactly four | `bbi` | |
| `avp` | none | `avp` | a scalar source series holding turnover, and volume |
| `vol` | `periods` `[5, 10, 20]`, at most four | `volume` (bars), `ma1`..`ma3` | volume |
| `macd` | `short` 12, `long` 26, `signal` 9 | `dif`, `dea`, `macd` (bars) | |
| `kdj` | `period` 9, `k_smoothing` 3, `d_smoothing` 3 | `k`, `d`, `j` | |
| `rsi` | `periods` `[6, 12, 24]` | `rsi1`..`rsi3` | |
| `bias` | `periods` `[6, 12, 24]` | `bias1`..`bias3` | |
| `brar` | `period` 26 | `br`, `ar` | |
| `cci` | `period` 20 | `cci` | |
| `dmi` | `period` 14, `adxr_period` 6 | `pdi`, `mdi`, `adx`, `adxr` | |
| `cr` | `period` 26, `ma_periods` `[10, 20, 40, 60]`, exactly four | `cr`, `ma1`..`ma4` | |
| `psy` | `period` 12, `ma_period` 6 | `psy`, `maPsy` | |
| `dma` | `short` 10, `long` 50, `signal` 10 | `dma`, `ama` | |
| `trix` | `period` 12, `ma_period` 9 | `trix`, `maTrix` | |
| `obv` | `ma_period` 30 | `obv`, `maObv` | volume |
| `vr` | `period` 26, `ma_period` 6 | `vr`, `maVr` | volume |
| `wr` | `periods` `[6, 10, 14]` | `wr1`..`wr3` | |
| `mtm` | `period` 12, `ma_period` 6 | `mtm`, `maMtm` | |
| `emv` | `period` 14 | `emv`, `maEmv` | volume |
| `roc` | `period` 12, `ma_period` 6 | `roc`, `maRoc` | |
| `pvt` | none | `pvt` | volume |
| `ao` | `short` 5, `long` 34 | `ao` (bars) | |

A list of periods (`periods`) holds one to five entries, and one output line per entry (`vol`: one to
four, because the volume bars take the first output). Periods are whole numbers from 1 to 1,000,000;
the `sma` weight and the `sar` factors are positive and the `boll` multiplier is not negative.
`indicator_schema("klinechart_<name>")` reports the same defaults (a list as `period_1`, `period_2`,
...) and output names from the engine, so a settings editor can read them instead of copying this table.
KLineChart lists a second `emv` parameter, 9, that its formula never reads; it is not part of the
definition.

**Sources.** `source` is the OHLC series the formulas read. `avp` is the exception: its source is a
scalar series holding the traded value (turnover) per bar, typically hidden, and it divides the running
sum of that series by the running volume. A scalar series (a line, area, baseline, or histogram) is also
accepted as the source of every other template and is read as open = high = low = close = its value, so
`macd` over a line series computes the MACD of that line. The templates marked "volume" need a scalar `volume_source`
(a scalar series such as a histogram or a line, never the source itself); volume pairs with source rows by exact timestamp,
and a bar without a volume uses KLineChart's own default (1 for `pvt`, 0 for the others). Every other
template must not be given a volume series. `add_klinechart_indicator` has no `amount_source`: the
engine accepts a turnover series only for VWAP, so a KLineChart binding carries turnover in `avp`'s
source instead.

**Invalid input.** An unknown template name (the name is case-sensitive and lower case), a missing,
non-whole, out-of-range, or non-numeric parameter, too many or too few periods, a source series that
no longer exists, an OHLC source given to `avp`, or a volume series that is missing, extra, equal to the
source, or not scalar throws `AerisChartsError` with code `invalid_options` and leaves the chart
unchanged. The engine decides every one of these, and the message names the template that was passed. Nothing is rounded: `{ short: 2.5 }` is rejected
instead of floored, unlike the period arguments of the other `add_*` methods.

**Presentation.** The engine owns the look, as KLineChart draws it. Line outputs are 1px lines in
KLineChart's five-color palette in output order, with no title chip, last-value label, or price line.
`vol`, `macd`, and `ao` draw their bar output as a histogram, and `sar` draws marker-only dots. Bars and
dots are colored per row by the engine, from the source candles' up and down colors: `vol` bars by
candle direction (grey when flat), `macd` bars by sign and by whether they are rising, `ao` bars by
whether they are rising, and `sar` dots by their position against the candle's midpoint. KLineChart
outlines a rising `macd` or `ao` column; Aeris histograms have no outline style, so those columns are
filled at a lighter alpha instead. Price templates (`ma`, `ema`, `sma`, `boll`, `sar`, `bbi`, `avp`)
draw over the candles on the main pane, and every other template in a pane of its own below it.

**Lineage, warm-up, and persistence.** Every output's `indicator_info()` has `kind:
"klinechart_<name>"` (a member of `indicator_kind`), the whole definition in `parameters.klinechart`,
`period` set to the first period (0 for `avp`, `pvt`, and `sar`), `deviation: null`, and the bound
`volume_source`. Rows before an output's `warmup_bars` hold no value and are not returned by `data()`;
`convergence_bars` is `null` for `ema`, `sma`, `macd`, `kdj`, `rsi`, `dmi`, `trix`, `obv`, `pvt`, `avp`,
and `sar`, whose values depend on the whole loaded history (recursive smoothing, running totals, or a
path state). A binding steps each formula one row at a time from checkpointed state, so a live
tick costs the formula's window rather than the history, and publishes the changed suffix. A
whitespace row (a missing bar, or a pre-installed session slot) emits no value and never enters a
window: every value equals the one computed on the chart without that row. V3 chart state stores the definition as `{"kind": "klinechart",
"indicator": "macd", "short": 12, "long": 26, "signal": 9 }` with the source, volume-source, and
per-output style references, and restores into a fresh chart like every other study. The formulas are
translated from KLineChart v10.0.3 and match its output bit for bit (see `docs/Architecture.md` and
`NOTICE`).

## Crosshair shade

```ts
chart.apply_options({
  crosshair: { shadeRight: { visible: true, color: "rgba(74, 74, 74, 0.12)" } },
});
```

**What it paints.** With `crosshair.shadeRight.visible` the engine veils the pane region to the
right of the hovered bar: one filled rectangle per stacked pane, from the snapped bar's right edge
to the pane's right edge and over the pane's full height. The edge is the same device-pixel bar
rule the `HighlightBarCrosshair` primitive uses, so a highlight and the veil abut exactly; the veil
follows the vertical line onto the empty right-offset slots, and when the bar's right edge rounds
onto the pane edge (the last bar at right offset 0 with an odd device bar width) nothing is drawn.
The veil is painted in every pane like the vertical line, not only in the pane under the pointer,
and it is independent of `vertLine.visible`. It sits in the crosshair overlay above series,
drawings, chrome, and trading objects and below the crosshair lines, and it follows the crosshair's
own gates: hidden mode, and the suppression while an interactive object (drawing, trading control)
is hovered or dragged. Moving the pointer rebuilds only the crosshair overlay, as without the veil.

**Color.** `color` is a CSS color whose alpha carries the opacity; the default
`rgba(74, 74, 74, 0.12)` is the crosshair line token at 12% alpha and is the same in both themes.
An unparsable value (a named color, `hsl()`) falls back to that default instead of dropping the
veil. The veil is an ordinary translucent `Rect`, so it inherits each executor's existing alpha
compositing with no shade-specific path. The WebGPU executor's Canvas2D-exact source-over
(`crates/aeris_charts_render_wgpu/src/blend.rs`) documents a ±1/255 residual only for tints so faint
that a channel's premultiplied 8-bit value rounds to 0, `(c * a + 127) / 255 == 0` with `a` the
8-bit alpha; the default tint (`a = 31`, premultiplied channel 9) does not reach it. GPUI
translucent fills are covered by the reported, not gated, translucent-rects parity fixture.

**Persistence and reset.** The option lives in the chart options store, so a V2 document (general
panes) carries it with `vertLine`/`horzLine`; V1 and V3 documents carry no chart options.
`chart.reset_style_to_defaults()` restores `color` and keeps `visible`, the same split the
watermark uses. Rust hosts read `CrosshairOptions::shade_right` (`CrosshairShadeOptions { visible,
color }`); there is no separate setter, the option flows through `apply_options` like every other
crosshair key.

## Baseline reference line

```ts
const price = chart.add_series("baseline", {
  baseline_mode: "close_before_visible_range",
  baseline_line_visible: true,                 // dashed #4a4a4a, 1 CSS px by default
  baseline_line_color: "#4a4a4a", baseline_line_width: 1, baseline_line_style: LINE_STYLE_TO_U8.dashed,
});
price.baseline_price();                        // the price the quadrants compare against, or null
```

**Which price is the baseline.** A baseline series without a pinned `baseline_value` resolves its
baseline from the visible window per `baseline_mode`. `"visible_midpoint"` (the default, today's
behavior) is the midpoint of the minimum and maximum visible close. `"close_before_visible_range"`
is the last finite close strictly before the first visible bar, so the window reads as change
against where it started; whitespace rows before the window are skipped, and when no finite row
precedes it the first visible finite close stands in, so the first bar reads as unchanged and the
series never disappears. Both modes follow the visible window and change while scrolling or
zooming; a host that knows the true prior-session close pins `baseline_value`, which wins over every
mode. One resolution feeds everything: the quadrant fills and strokes, the reference line, the
live price line color, the last-value axis chip, the crosshair marker, and `baseline_price()`. The
mode does not change the `histogram_updown_rule: "previous_close"` first-bar reference, which still
reads only a pinned `baseline_value` (or the scale's `base_value`). `"previous_close"` is not a
`baseline_mode` value (it is reserved for a future session-anchored mode); a bad value throws
`invalid_options` before any option of the call is applied, like `histogram_updown_rule`.

**The line.** `baseline_line_visible` (default `false`) draws the resolved baseline as one
full-pane-width horizontal line painted between the quadrant fills and the quadrant strokes, so
the strokes stay on top of it and it stays above the fills. `baseline_line_color` is a CSS string
stored verbatim; unset or `""` follows the neutral chrome tint `#4a4a4a` (the crosshair line token,
the same in both themes), and an unparsable value falls back to that tint instead of dropping the
line. `baseline_line_width` is CSS px (default `1`, positive; floored to whole device pixels with
the vertical ratio, at least one) and `baseline_line_style` a `LINE_STYLE_TO_U8` value (default
`2`, dashed). The line is drawn only while the series itself reaches the pane: a bar in view, or
a segment bridging a window between two off-screen bars; once the series has scrolled entirely
off one side, the line goes with its fills and strokes. It replaces
the `create_price_line({ price: prev_close, line_style: "dashed" })` workaround of the intraday
example when the previous close is pinned, and needs no host bookkeeping when it is not.

**The query.** `series.baseline_price()` (`baselinePrice()`) returns the resolved baseline of a
baseline series as a number, or `null` for every other series type, a removed series, a chart
without a visible range yet, and a `"visible_midpoint"` series none of whose bars is in the
visible window (another series keeps the window). `"close_before_visible_range"` is still defined
then and reports the last close before the window, which is the series' own last close once it
has scrolled entirely off to the left. Rust hosts call `ChartEngine::series_baseline_price(id)`;
the wasm export is `AerisChart.series_baseline_price(id)`.

**Reset, persistence, workers.** The line options are style: `chart.reset_style_to_defaults()`
restores them. `baseline_mode` is semantic and survives the reset, like `break_on_trading_day`.
Like every other financial series style option none of them is persisted. Worker (offscreen)
charts cannot set them: `apply_series_options` on a worker chart accepts only the alignment keys
after creation, so they are main-thread options (see [Intraday (分时) charts](#intraday-分时-charts)).

## Live-bar easing

```ts
const price = chart.add_series("candlestick", { live_bar_easing_ms: 120 });
price.update({ time: last.time, open, high, low, close });   // same time: the drawn bar glides
price.data().at(-1).close === close;                         // true at once: queries read real values
```

**What glides.** `live_bar_easing_ms` is the time constant `tau` in milliseconds (`0`, the
default, is off; values above `1000` clamp to it; a negative or non-finite value throws
`invalid_options` before any option of the call is applied). When `update()`, `update_typed()`,
`merge()` or a merge batch replaces the drawn last bar in place — the same canonical row and the
same time — the displayed high, low and close move toward the new values on every presented frame
with `x += (target - x) * (1 - exp(-dt / tau))`, where `dt` is the host clock delta capped at
100 ms per frame, so a tab that stopped presenting resumes with a glide instead of a jump. The
open never eases. The glide settles exactly on the target when every channel is within
`max(1e-9, |target| * 1e-7)` of it or six time constants after the last change, and the
animation loop stops; a feed that keeps moving the target keeps gliding without ever freezing.
A brand-new bar, `set_data()`/`setData()` with the same times, `pop()`, a moved replay clock, a
retention trim and the `reduced_motion` interaction option snap to the real values at once. A
write to another bar while the drawn bar glides (a historical correction, a replay bar past the
clock) leaves the glide running, since the drawn bar did not change. A last bar hidden by
`render_before_time` never eases. The first frame after a tick still shows the previous values
(it stamps the clock), so the glide becomes visible on the second presented frame.

**What reads the eased values.** Only what is drawn for that bar: the candle, bar, histogram
column, line, area or baseline geometry (a `histogram_updown` volume column takes its tint from
the primary's drawn direction, so it flips when the eased close crosses the open), the series'
last-value line and axis chip, the pulse, the crosshair marker on that bar (so it sits on the
drawn line) and the series hit test (so the drawn wick is hittable). Heikin Ashi candles
recompute their last row from the eased raw values with the previous canonical row, so they
glide too. Everything else reads the real values throughout: `data()`, `data_by_index()`,
`last_value_data()`, `bars_in_logical_range()`, the value snapshot behind the legend, tooltip
and data window, the crosshair magnet (in magnet mode the horizontal line snaps to the real close
and therefore sits apart from the marker by the glide while it runs — a documented one-bar
divergence), trading geometry, the percentage/indexed base value, `baseline_price()` and
autoscale, which follows the real range at once.

**Hosts.** Browser charts need no host code: `wants_animation` now covers the glide and the
package's rAF loop, which restarts after every repaint, runs until it settles. Worker (offscreen)
charts cannot set the option after creation and run no animation loop, so they never ease (see
[Intraday (分时) charts](#intraday-分时-charts)). Rust hosts drive `ChartEngine` directly:
`set_animation_time(ms)` advances every glide and the pulse clock (browser loop),
`advance_live_bar_easing(now_ms) -> bool` advances only the glides (what the GPUI adapter calls
from `GpuiChartInput::prepare_frame`), `animation_active()` is the browser loop predicate and
`animation_frame_requested()` the Rust-host frame-request predicate (an input animation or a
glide; the pulse is browser-only). `live_bar_easing_active()` answers whether any glide is
unsettled.

**Reset, persistence, parity.** The option is style, like `last_price_animation`:
`chart.reset_style_to_defaults()` turns it off and it is not persisted. A settled frame is
bit-identical to a clean rebuild and a frame built from the real values alone; mid-glide frames
depend on the host clock, so pixel-parity fixtures keep the option off.

## Timeline marks

```ts
chart.timeline_marks().set({
  groups: [{ id: "earnings", label: "Earnings" }],
  marks: [
    { id: "q3", time: 1_700_000_000, group: "earnings", glyph: { shape: "circle", color: "#2962ff", letter: "E" }, title: "Q3 report" },
    { id: "call", time: 1_700_000_000, group: "earnings", title: "Call" },      // same bar: one token with the count 2
    { id: "up", time: 1_700_300_000, group: "news", glyph: { shape: "diamond", color: "#f7525f" } }, // a group missing from `groups` labels as its id
  ],
});
chart.subscribe_timeline_mark_click((hit) => open_popup(hit.mark_ids, hit.label, hit.title));
chart.timeline_marks().set_group_hidden("news", true);  // persists with export_state()
```

**What the lane is.** A row of glyph tokens along the bottom of the primary series' pane: one
token per bar slot (or per cluster of near slots), separate from series markers and from the
trading host overlay. A mark has a unique `id` (1..=128 bytes), a unix-second `time`, a `group`
id, a glyph (`shape`: `circle` (default), `square`, `diamond` or `pin`; a CSS `color`; a
`letter` of at most two characters) and a `title` (at most 128 bytes). Groups carry a `label`
(at most 64 bytes) shown in the tooltip; a mark may name a group missing from `groups`, which
then labels as its id. `set()` replaces marks and groups atomically and throws `resource_limit`
above 4096 marks or 64 distinct groups and `invalid_data` on duplicate ids, over-long strings or
an unparsable color, leaving the lane unchanged. `set_visible(false)` hides the whole lane
(runtime, default shown).

**Where a mark lands.** The engine resolves the bar slot: the bar whose span `open .. open +
min(step, next_open - open)` holds the time, where `step` is the installed session bar grid's
interval (close-time labels), else the `set_future_time_projection` cadence, else the prevailing
bar interval. A time inside a gap — an overnight or a weekend — lands on the next bar, never on
the bar before the gap; a time past the last bar projects into the right-side whitespace by
whole steps and draws only while the scale shows that whitespace; a time before the first bar,
or after the replay clock, draws nothing. Tick, volume and range bars map by the bar's
open..close span without projection. Tokens whose centers come within 20 CSS px of a cluster's
earliest slot fold into one token: a single-group cluster keeps that group's glyph with the
count in place of the letter, a mixed cluster paints a neutral bordered square with the count
(`99+` from a hundred). Clustering is computed in CSS px, so it is identical at every device
pixel ratio, and token geometry derives from constants, never from measured text, so the lane is
pixel-identical on every backend. The lane hides on panes shorter than 96 CSS px and shows again
above 108 CSS px (hysteresis), and while shown with any mark it reserves 27 CSS px below the data
on every auto-scaled scale of its pane — independent of the view and of hidden groups, so
panning or toggling never moves the scale; a manual scale can overlap the lane like series
markers, and a bottom-pinned volume overlay lifts with it.

**Interaction.** Hovering a token answers the pointer cursor and draws a 1 px ring; after the
same 450 ms dwell as trading controls the engine draws a title tooltip (`label · title` for one
mark, `label · N` for a single-group cluster, `N marks` for a mixed one) in the theme's surface,
border and text tokens. A click (press and release on the same token without a drag) fires
`subscribe_timeline_mark_click` once with a `timeline_mark_hit` — the anchor `logical` slot and
`projected` flag, the earliest `time` and `title`, the `count`, the distinct `groups`, every
`mark_ids` entry and the `label` — and never selects, pans or reaches a drawing under it; a
double-click on a token never opens a drawing editor. `hit_at(x, y)` answers the same precision hit
for any chart-content point: a token answers across its own box (16 CSS px, 18 for a cluster) and
the rest of the lane belongs to the pane. Only a touch press on the chart widens that box by the
touch hit tolerance, inside the engine input controller; `hit_at` itself always uses the precision
box. The rich click popup stays host UI. Accessibility exposes one `mark:` focus target per mark of a shown group, on the
pane the engine shows the lane on (the primary series' pane while the lane is enabled, non-empty
and tall enough) and none while the lane is hidden, with the tooltip text as its label; Enter
activates the token through the same click subscription when the mark is in view.

**Hidden groups and persistence.** `set_group_hidden(group, hidden)` hides or shows one group's
marks (at most 64 hidden groups, 128-byte ids; throws `resource_limit`/`invalid_data`). The set
is independent of the snapshot, so a host may import a document first and set marks later, and a
new `set()` never prunes it. Hidden groups are the only persisted part of the lane: the optional
`hidden_mark_groups: string[]` field on V1, V2 and V3 documents (omitted when empty, no schema
bump; documents with more than 64 entries or over-long ids fail structurally and atomically).
Marks never persist.

**Hosts.** Rust hosts call `ChartEngine::set_timeline_marks`, `timeline_marks`,
`set_timeline_marks_visible`, `timeline_marks_visible`, `set_timeline_group_hidden -> Result<bool,
ChartError>`, `hidden_timeline_groups`, `timeline_mark_hit_at`, `timeline_mark_hit_at_with_profile(x,
y, HitProfile)` (touch tolerance), `timeline_mark_hit_for_id`, `timeline_lane_pane -> Option<usize>`
(the pane showing the lane now; keyboard targets for marks follow it) and read a click's
`ChartInputEvent::TimelineMarkActivated(seq)` back through `timeline_mark_activation(seq)` (a
32-entry ring). The wasm exports are `set_timeline_marks_json`, `timeline_marks_json`,
`set_timeline_marks_visible`, `set_timeline_group_hidden`, `hidden_timeline_groups_json`,
`timeline_mark_hit_json`, `timeline_mark_hit_for_id_json` and `timeline_lane_pane`; the controller
event `timeline_mark_activated` carries the resolved `hit`. Worker (offscreen) charts do not proxy
the lane handle in this revision.

## Coordinates and panes

Every public coordinate lives in one chart-content space, and it is never pane-local. `x` is CSS px
from the plot-area left edge (right of the left price strip), so the container `x` is
`pane.get_geometry().left + x`. `y` is CSS px from the top of the stacked pane area, which is pane 0's
top. A pane spans `[geometry.top, geometry.top + geometry.height]` in that `y`; its pane-local `y` is
`y - geometry.top`. The same space carries `series.price_to_coordinate`/`coordinate_to_price`,
`chart.price_to_coordinate`/`coordinate_to_price`, `time_scale()` conversions,
`mouse_event_params.point`, the crosshair, hit tests, drawings, and trading geometry, so a `y` from one
API can be handed to any other. All values reflect the last layout pass: after `pane.set_height`, a
separator drag, or a pane move, read them again once the chart has laid out.

**Choosing a pane.** A series handle converts on its own pane and price scale, in that scale's mode and
base. A host that needs a lower pane's coordinates keeps a handle to a series in that pane:

```ts
// `rsi` is a series handle in pane 1, for example from `chart.add_rsi(candles, 14)`.
const pane = chart.panes()[1].get_geometry();
const y = rsi.price_to_coordinate(70); // in pane 1's [top, top + height] while 70 is in range; never pane-local
const paneLocalY = y! - pane.top;      // pane-relative chrome subtracts the pane top itself
rsi.coordinate_to_price(y!);           // 70
```

`chart.price_to_coordinate(price)` converts on pane 0's default scale (the first visible non-overlay
series' scale, else the right scale), and `chart.coordinate_to_price(y)` uses the default scale of the
pane containing `y`: a separator belongs to the pane above and a `y` below the panes to the last pane.
That is the scale the crosshair label reads in that pane, so the two never disagree. Neither follows
series creation order or an overlay's scale. Time, logical, and `x` conversions are the same for every
pane.

**Two other spaces.** Plugin draw-context converters (`price_to_y`, `time_to_x`, `logical_to_x`) return
bitmap px of the whole chart with `x` including `pane_left`, the space the plugin canvas draws in;
subtract `pane_left` and divide by `dpr` to compare them with the CSS-px converters above. A plugin
axis-label descriptor's `coordinate` is pane-local (price: px from the pane top; time: px from the
plot-area left) unless a series primitive supplies `price`, which is converted on the series' scale.

**Linked crosshairs.** `crosshair_sync_position()` and the `crosshair` events from `take_sync_events()`
carry `pane_index` and a `price` on that pane's default scale, with the pane picked from the crosshair
`y` (a separator counts as the pane above). `set_crosshair_position(price, time, series)` places the
line through the given series' scale; the emitted price is the raw `price` when that series is on the
pane's default scale (and, in percentage and indexed modes, shares its base), otherwise it is
re-expressed on the default scale so a linked chart lands on the same line. `apply_external_crosshair`
converts the price on the requested pane's default scale and holds the line inside that pane: a price
outside the pane's visible range sits on the pane's edge instead of drawing in a neighbouring pane.

## Time, exchange time zone, and trading sessions

Canonical chart time is whole UTC seconds. `business_day` values and strict `"YYYY-MM-DD"` strings
are taken at UTC midnight; values returned by the chart are always numeric UTC seconds. The
financial time axis is ordinal: bars are spaced by index, so weekends, holidays, lunch breaks, and
half days take no width. Hosts own exchange calendars and send only the bars that exist; holiday
calendars are not an engine feature. By default every series' timestamps join the axis, so an
overlay from another market calendar adds its own bars as slots; opt it into `time_alignment:
"as_of"` instead (see **Multi-calendar overlays**).

**Multi-calendar overlays.** `series_options.time_alignment` is `"union"` (the default, reference
behavior) or `"as_of"`. An as-of series (an index over a stock from another exchange, crypto over
equities, a futures night session over its underlying) adds no time point, so the other series keep
a gapless axis. Each point, up to the last real bar of the other series, shows the overlay's last
row at or before that point's time: rows between two points collapse into the later one, a point
with no newer row repeats the previous row, and rows after the last point wait until the other
series reach them (a pre-installed intraday session never shows the overlay in its future slots).
`as_of_max_staleness` (whole seconds, `null` by default) leaves a point empty instead of repeating
a row older than that; `0` shows only rows exactly at a point. The points come from the union
series, so an as-of series alone on a chart shows nothing. Everything reading the series follows
the same points: rendering, hit testing, crosshair and legend values, `value_snapshot` (which
reports the point's time), comparison anchors and percentage bases, autoscale, markers (placed at
the first point at or after their time; a marker newer than every point waits with its row, and a
point the staleness bound leaves empty hides it), session highlighting (each point is colored as
the row it shows), the accessibility focus ring (on the first point showing the focused row), and
last-value chrome. `data()`, `data_by_index`, and `last_value_data` return the overlay's own rows
with their own times. Studies bound to an as-of series compute on its own rows and are shown the
same way; their `time_alignment` reads back as the source's and cannot be set. Live ticks on
either side stay proportional to the points they change. Only line, area, baseline, histogram, bar,
and candlestick series that own their rows accept it: custom, advanced, footprint, trade-bound,
trade-study, and synthetic series, and every series on a non-time (tick, volume, range, or
synthetic) bar axis, throw `unsupported_operation`. Converting an as-of series to a custom,
advanced, or footprint series, binding it to a trade stream, or configuring it as a synthetic
transform returns it to the union. A bad value or a staleness without `"as_of"` throws
`invalid_options` before the call's other options apply; `add_series` checks both keys before it
creates (or adopts) the series, and a refusal leaves no series behind on the main thread or in a
worker. Re-applying the current alignment is a no-op that notifies no `subscribe_data_changed`
handler; a change notifies each once with `"full"`. Worker charts take both keys in
`add_series` options and change them later with `offscreen_chart.apply_series_options(patch,
series_id)`; `offscreen_chart.series_options(series_id)` reads the options back. A worker chart
addresses series by the numeric id `add_series` returned (`0` is the primary). The patch follows the
same rules and throws the same codes as `apply_options` (an omitted key keeps its value, switching
to `"union"` clears the staleness bound, an unchanged request is a no-op) and applies nothing when
it throws. It accepts only `time_alignment` and `as_of_max_staleness`: any other key with a value
throws `unsupported_operation` naming it, and a non-object patch throws `invalid_options`. An id
that is not a whole number in `0..=4294967295` or names no live series throws `invalid_handle`
(`stale_handle` for a series that was removed), even for an empty patch, and a removed chart throws
`disposed`. A call that succeeds repaints the worker canvas before it returns, like the other
worker mutations (an unchanged request too); a call that throws paints nothing. Worker charts have no
series handle, so no `subscribe_data_changed` notification; read `visible_logical_range()` after the
call. Rust hosts call
`ChartEngine::set_series_time_alignment(id, TimeAlignment::AsOf { max_staleness })`. Like other
financial series options, the setting is host-owned and not persisted.

**Exchange time zone.** `chart.time_scale().apply_options({ time_zone, session_start })` or the
declarative chart option `timeScale: { timeZone, sessionStart }` (also accepted by worker charts)
sets how instants are grouped and labelled. `time_zone` is `"UTC"` (the default), an IANA name such
as `"Asia/Shanghai"` or `"America/New_York"`, or an explicit schedule of
`{ from_utc_seconds, offset_seconds }` transitions (strictly ascending, at most 1024, offsets within
±18 h; the first offset also applies before its entry). The package resolves an IANA name once per
zone with `Intl.DateTimeFormat` over 1970–2100 (at most ~262 DST transitions) and keeps at most 32
resolved zones; outside that span the nearest offset applies. The engine itself never reads the
browser's time zone: `timeScale.timeZone` takes only `"UTC"` or an explicit schedule. A zone from the
TradingView parity list (`TRADINGVIEW_TIME_ZONES`; WASM `supported_time_zones_json()`) can instead be
named to the engine with `ChartEngine::set_time_zone(&str)` (`Ok(true)` when it changed; WASM
`set_time_zone`) or the top-level engine option `timezone`; it is resolved once into the same schedule
(1970–2100, about 4 ms natively), so grouping and labels match the explicit schedule, and the name
also localizes the general (non-financial) temporal axes and
`ChartEngine::time_zone_clock_text(utc_seconds, show_seconds)`. `ChartEngine::time_zone_id()` (WASM
`time_zone()`) returns the named zone, `Etc/UTC` by default, or `custom` while an explicit schedule is
installed. An unknown zone, malformed schedule, out-of-range session start, or session start that the
windows of an installed close-time label do not fit ([Close-time labels](#close-time-labels)) throws
`invalid_options` before any other key in the same call is applied; an unsupported or non-string
`timezone` rejects the patch the same way. `time_scale().options()` reports the IANA name (or the
schedule) and `session_start`. Rust hosts call `ChartEngine::set_exchange_offsets(UtcOffsetSchedule)`
for an explicit schedule and `set_session_start_seconds(i32)`; the engine JSON options accept
`timeScale.timeZone` (`"UTC"` or a transition array), `timeScale.sessionStart`, and `timezone`. V2
persistence round-trips them: the schedule always, and the `timezone` name while a named zone is
installed (an explicit schedule clears it). When one patch carries both a schedule and a name, the
schedule drives grouping and labels and the name the general axes and the clock. Importing a document
that predates these keys keeps the chart's installed zone and session start.

Display-only time projection: `ChartEngine::set_future_time_projection(cadence_seconds, points)` and
`set_past_time_projection(cadence_seconds, points)` (each bounded at 4,096 points; `None` or zero points
clears it; `has_future_time_projection` / `has_past_time_projection` read it back) label the whitespace
after the last and before the first bar of a time axis. Projected points are labels only (no data rows,
base index or point count) and are not persisted.

**Trading day.** `session_start` is the offset in seconds from exchange-local midnight at which a
trading day begins (default `0`, range ±86 399). A negative value assigns an evening session to the
next trading day, e.g. `-3 * 3600` makes a 21:00 China futures night session start the next day;
with a negative start a day that would fall on Saturday or Sunday rolls forward to Monday, so a
Friday-night session belongs to Monday. Day/Month/Year tick marks, VWAP `session`/`weekly`/`monthly`
resets, and pivot sessions use trading days. Weekly periods start on Monday.

A market whose week opens on Sunday evening (CME Globex, 17:00 Central) sets `session_start:
-25200`: Sunday 17:00 belongs to Monday's trading day and Monday 17:00 to Tuesday's, so Day marks,
session and weekly VWAP resets, and pivots align with the session. With `0` the Sunday evening is its
own trading day: a Day mark and a session VWAP reset appear at midnight in the middle of the
session, and the weekly VWAP keeps the Sunday evening bars in the previous week and resets at that
midnight. With a negative start every Saturday or Sunday instant belongs to Monday, and window
placement (`session_slot_times`, `resample_boundaries`, `set_trade_stream_sessions`)
assumes the week opens on Friday evening: right for China futures, but a Sunday-open market places
its evening windows with a per-call `session_start` of `0` (see *Intraday (分时) charts* and *Ticks
to candles and resampling*).

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
their own `time_formatter`). Without it, `create_tooltip` and accessibility print the crosshair label
(`chart.format_time_label()`): `localization.date_format` and `localization.locale` in the chart time
zone, adding the time of day for intraday rows and none for calendar-date rows. Rust `TickMarkFormatterFn`
and `TimeFormatterFn` signatures are unchanged. With a close-time label configured
([Close-time labels](#close-time-labels)) both callbacks receive the LABEL instant (the bar's
close), not the bar's open time; a host that adds one interval inside its formatter must drop that
addition when it adopts the option.

**Countdown clock.** The candle-close countdown shows only while the clock is inside the forming
bar's interval `[last_bar_time, last_bar_time + bar_interval)`; outside it — lunch breaks,
overnight, weekends, after an early close — it hides instead of cycling. Calendar-date bars form
during the exchange trading day(s) of their date (with a negative `session_start`, Monday's bar
starts on Friday evening at the session-start time of day: Friday's night session for China
futures, Friday 17:00 Central for a Sunday-open market such as CME Globex), and bars 28 or more
days apart run to the end of their calendar month(s). `chart.set_clock(() => utc_seconds)` and
`offscreen_chart.set_clock(...)`
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
DST. `session_start` is this call's own and defaults to `0`; it is not read from the chart, so a
China futures host must pass `session_start: -10800` explicitly (omitted, a 21:00 night window is
placed on the calendar date, Monday 21:00, instead of Friday 21:00, and nothing reports it). With a
negative `session_start`, windows starting at or after the session-start time of day belong to the
evening before (Friday evening for Monday), matching the chart's trading days and the night
sessions of Chinese futures. That placement assumes the week opens on Friday evening.

A market whose week reopens on Sunday evening (CME Globex) passes `session_start: 0` and makes one
call per evening date with `windows: [["17:00", "16:00"]]`. The date is the evening the session
opens: the Sunday date places Monday's trading day (1380 one-minute slots, Sunday 17:00 to Monday
16:00), the Monday date places Tuesday's, and so on, so with `0` every call is keyed by the
evening's calendar date rather than by trading date. Two calls give the same slots: the Sunday date
with `[["17:00", "24:00"]]` and `session_start: 0`, then the Monday date with `[["00:00", "16:00"]]`.
Do not give the Monday date a window that starts at or after 17:00 under a negative `session_start`:
that places Friday 17:00 to Saturday 16:00. Which dates trade, holidays, and early closes are host
calendar data; pass the windows that apply to each date (a date without a night session is a
window list without it). It needs the engine module: call it after `init_wasm()` or
`create_chart()`. Rust hosts call
`aeris_charts_engine::session_slot_times(day, &windows, interval, chart.exchange_time(), convention)`.

`convention` decides which instant names a slot. Aeris bars are stamped with their open time, so
the default `"bar_open"` gives 240 one-minute slots for an A-share day (09:30..11:29, 13:00..14:59).
同花顺 and 富途 instead label a minute by its close and show the opening-auction print as its own
first point: `"bar_close_with_open"` reproduces their 241 points (09:30, 09:31..11:30, 13:01..15:00);
`"bar_close"` is the close-labelled form without the opening point. Use the convention your data
provider stamps its minutes with. The lunch break takes no width either way: the last morning slot
and the first afternoon slot are neighbours.

The two families of chart use the conventions differently. An instant-sampled line (the classic
time-sharing price line, one price per minute end) uses close-stamped slots: its points ARE those
instants, and 241 points are a property of the line. Interval bars (candles built from ticks or
resampled minutes) use `"bar_open"` slots and open-stamped rows, and print their close time through
the `bar_time_label` option ([Close-time labels](#close-time-labels)); they do not grow a separate
241st auction bar, because the 09:25 auction print folds into the first bar.

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
bar-wide segment, and `baseline_line_visible` draws the previous close as a dashed reference line
(see [Baseline reference line](#baseline-reference-line)). Center its scale on the previous close,
and put the same prices on a second scale in percentage mode based on the previous close:

```ts
const price = chart.add_series("baseline", {
  price_scale_id: "left", baseline_value: prev_close, baseline_line_visible: true,
  top_line_color: "#f7525f", bottom_line_color: "#089981",
});
const percent = chart.add_series("line", { price_scale_id: "right", line_visible: false });
chart.price_scale("left").apply_options({ autoscale_center: prev_close, scale_margins: { top: 0.08, bottom: 0.08 } });
chart.price_scale("right").apply_options({
  mode: 2, base_value: prev_close, autoscale_center: prev_close, scale_margins: { top: 0.08, bottom: 0.08 },
});
price.set_data(slots.map((time, i) => row(time, closes[i])));   // closes[i] undefined for future minutes
percent.set_data(slots.map((time, i) => row(time, closes[i])));
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
The average line restarts at every session reset, so on a multi-day chart no segment joins one
day's last average to the next day's first. To separate the days' price lines the same way, give
the price series (and a percentage mirror) `break_on_trading_day: true`; the five-day demo does.

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

## Ticks to candles and resampling

Both recipes build ordinary candles on the time axis in the chart's exchange time, so set the
exchange time zone (and `session_start` for night sessions) first. Candles are stamped with their
open time, the canonical Aeris bar time. To print each candle's close time (09:31 … 15:00 for A-share
minutes) instead, set `bar_time_label` ([Close-time labels](#close-time-labels)): the candles, their
volume, replay, countdown, and every time your host passes in or reads back stay open-stamped.

### Ticks to candles

A chart-level trade stream owns the tick tape; an ordinary candlestick (or bar) series bound to it
presents the stream's time bars, and a volume histogram is derived from the same bars:

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

**Session anchoring.** Without sessions, time bars align to the `anchor_seconds` grid (default 0,
the UTC grid), which suits 24-hour markets. `set_trade_stream_sessions()` places the exchange-local
windows on every trading day in the chart's `time_zone` and `session_start` (the offset in force on
that date, so DST is respected), and each window restarts the bar grid at its open: A-share
60-minute bars open at 09:30, 10:30, 13:00 and 14:00, US 60-minute bars at 09:30 … 15:30 Eastern on
both sides of a DST change, and the lunch break takes no width. An `interval_seconds` of 86 400 gives
one bar per trading day, opening at its first window. Changing the chart's time zone or session start
re-places the windows; `null` restores the plain grid. Only whole-second time bars accept sessions;
invalid windows or other bar types throw `invalid_options` and change nothing.

`set_trade_stream_sessions()` always uses the chart's own `session_start` (there is no per-call
override) and takes one window list for all dates, so a date after a break cannot drop its night
window. For a Sunday-open market (CME Globex) keep the chart's `session_start` at `0` and pass one
crossing window, `[["17:00", "16:00"]]`: the previous-day lookup places the Sunday evening, at the
cost of midnight trading-day semantics for Day marks and VWAP resets. A chart at `-25200` places
Monday's window on Friday evening, so Sunday and Monday prints fall after it: `fold` sends them to
that window's last bar (Saturday 15:xx) and `exclude` drops them.

**Prints outside the windows.** `outside: "fold"` (the default) keeps every print: the 09:25
opening auction opens the 09:30 bar, and the 11:30:00 and 15:00:00 closing prints close the last bar
of their window, as Chinese platforms show them. `outside: "exclude"` leaves pre-market and
after-hours prints out of every bar (a US regular-hours chart) but keeps prints stamped in a window's
closing second, such as the 16:00:00 closing cross. A print that is folded or excluded still takes
part in aggressor classification. A trade `session_id` change always starts a new bar and resets
session delta; the windows already split the morning and afternoon, so a per-date id is enough.

**Engine-owned series.** A bound candle or bar and the volume, CVD, and delta studies are written
only by their trade stream, and one series has one engine writer: `bind_trade_bar_series_to_stream`
throws `invalid_options` for a series that is not a candlestick or bar, carries a `max_points`
cap, or is already a footprint, a study, a resampled target, or a synthetic-bar series (rebinding a
bound candle to another stream stays allowed), and a footprint, resampled target, or synthetic
series cannot be created from a series that a stream writes. Their `set_data`, `set_data_typed`,
`update`, `update_typed`, `merge`, `merge_typed` (with or without `{ sequence }`), `pop`, and
`set_ring_source` are rejected: the data calls record `last_ingestion_diagnostics()` as
`{ status: "rejected", code: "derived_series" }`, warn, and change nothing, `pop` records the same
rejection without repainting or firing `data_changed`, and `set_ring_source` throws
`unsupported_operation` (unbinding with `null` still works, and a ring bound before the series
became derived keeps draining into `frame_stats().ring_dropped_rows` until unbound). Styling, pane
moves, visibility, `histogram_updown_rule`, and a study's own `max_points` still apply; a bound
candle refuses `max_points` because it follows the stream's retention. Feed the stream instead.

Rust hosts get the same refusals from the ordinary write entries (`false`, `0`, `None`,
`Err(UnsupportedSeriesData)`, or `Rejected(UnsupportedSeries)`), which also mean an unknown id or
invalid data, so `ChartEngine::series_is_source_owned(id)` tells an engine-owned series apart, and
`ChartEngine::apply_momentum_histogram_colors` returns `false` for the delta and volume studies.
`FootprintError::SeriesOwned` is what `bind_trade_bar_series_to_stream` returns for a candlestick
or bar that a resampler, synthetic bars, or a study (converted to a candle) already writes, and what
`configure_footprint_series` returns for any series a stream, study, resampler, or synthetic bars
write. `bind_trade_bar_series_to_stream` checks the series kind and `max_points` first, so a
footprint or a scalar study gets `UnsupportedTradeBarSeries` or `InvalidAggregation` instead.

**Live, corrections, and replay.** In-order prints update the forming bar in place and the first
print at or after a bar boundary opens the next bar (`"tip"`); late or corrected prints rebuild the
stream once (`"historical"`). The volume histogram, CVD/delta studies, and footprints on the same
stream follow every change. `chart.set_replay_clock_micros(clock)` shows exactly the bars of the tape
up to the clock; a bar that a folded pre-open print opens ahead of the clock appears when the clock
reaches its open. The volume columns take the up/down tint of the chart's primary (first) price
series (`histogram_updown`); restyle them like any histogram.

Rust hosts call `ChartEngine::set_trade_stream_sessions(stream, Some(TradeSessionOptions { windows,
outside: OutOfSessionPolicy::Fold }))` and `add_trade_volume_series(stream, pane)`;
`SessionBarGrid` exposes the same placement for other tick consumers.

### Resampling

`configure_resampled_series(target, options)` derives a candlestick or bar `target` (and an optional
volume histogram) from a source series in the engine. `resample_boundaries()` derives the periods
from the session windows, the exchange time zone, and the host's trading dates:

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

**Periods.** `span: "window"` (the default) returns one boundary per session window, so 5-, 15-,
30- and 60-minute bars restart at every window open (A-share 60-minute bars at 09:30, 10:30, 13:00,
14:00; a window whose length is not a multiple of the interval ends with a shorter bar).
`span: "day"` returns one boundary per trading date from its first open to its last close; with
`interval_seconds: 86400` that is one daily bar per date, stamped at the session open, so US daily
bars built from extended-hours minutes (04:00–20:00 Eastern) stay one bar per day across DST even
though winter sessions run past UTC midnight. Every boundary carries the requested date as
`session_id` (`YYYYMMDD`): the trading date for a market whose sessions start on it, the evening
date for a Sunday-open market (below). Dates are strictly ascending and use `session_slot_times`
placement (night sessions with a negative `session_start` included); at most 20 000 boundaries and
32 resampled series per chart. Hosts may also pass their own `{ start_time, end_time, session_id }`
periods (for weeks or months, for example).

`resample_boundaries` has its own `session_start`, default `0` and independent of the chart's, so
China futures pass `-10800` explicitly. For a Sunday-open market (CME Globex) pass `session_start:
0`, the evening dates, and `windows: [["17:00", "16:00"]]`; the `session_id` of each boundary is then
the requested evening date (`20240107` for the session that opens Sunday 2024-01-07 and is Monday's
trading day), or build the `{ start_time, end_time, session_id }` periods yourself. Never pass
`-25200` with the Monday date: it places that session on Friday 17:00 to Saturday 16:00, so Sunday
and Monday rows fall outside every boundary and are omitted.

The window list applies to every date of a call. Hosts with a calendar call `resample_boundaries`
once per window set and concatenate the arrays, which only need to be ordered and disjoint: night
and day windows for normal dates, day windows only for a date whose night session does not trade,
such as the first trading day after a break. With `span: "day"` the bar is stamped at the first
window's open, so a shared night-plus-day list would stamp that date's daily bar on the night
session that never traded.

**Source rows.** Source rows must be stamped with bar-open times; rows outside every boundary are
omitted, so the host still converts provider close stamps (09:31 … 15:00) to open stamps by
subtracting one interval before resampling. A feed that also carries a separate opening-auction
minute (241-bar feeds stamp it 09:30 beside the close-stamped 09:31) must merge that row into the
first minute before RESAMPLING: shifted, it falls before the first window and is omitted with its
volume. A 241-bar feed that is drawn directly and not resampled may instead shift the auction row
back too: it lands at 09:29, outside every window, and `bar_time_label` prints it 09:30 (see
[Close-time labels](#close-time-labels)). Whitespace rows (`session_slot_times` reservations)
reserve their bucket without prices: an untraded bucket is a whitespace bar, and the forming bucket
closes at its last traded row. Resampling needs a time axis: it is rejected on a chart whose axis is
a non-time bar sequence (trade-count, volume, or range streams, synthetic bars), and such a sequence
cannot join a chart that has resampled series.

**Live updates.** `update`, `merge`, and typed batches on the source or its volume refresh only the
affected tail: the unchanged prefix of derived bars is kept and the tail is rebuilt from the first
bucket that can hold a changed row, so a live minute re-reads one bucket, never the history, and
boundaries past the last source row (dates configured ahead) cost nothing. A `pop`, a
retention-cap trim of the source's head, a backward replay that loses a derived bar, and a complete
`set_data` rebuild once.
`chart.resample_stats(target)` reports `rebuilds`, `tail_refreshes`, and `rows_scanned`. Under the
replay clock the forming bar aggregates only rows at or before the clock. To reach a date past the
configured boundaries, call `configure_resampled_series` again (one rebuild); new dates can also be
included ahead of time because dates without data produce no bars. A reconfigure keeps the
binding's source (another source throws `invalid_options`). Bindings never chain, in either
configuration order: a target (or volume target) may not be another binding's source (or volume
source) or output, and a binding's volume source may not be its own volume target; each throws
`invalid_options` ("resampling dependencies may not be chained or cyclic") and changes nothing.

The targets are engine-owned: `set_data`, `update`, `update_typed`, `merge`, and `merge_typed` on
them are rejected (`last_ingestion_diagnostics()` reports `status: "rejected"` with
`code: "derived_series"`) and change nothing, and `pop` records the same rejection. A target must
not be a footprint, a trade-bound candle, a trade study, or a synthetic-bar series (`invalid_options`);
a trade-bound candle or the trade volume study may be the binding's source, however.
Removing any series of a binding (source, volume source, or a target) removes the binding together
with its target series, like indicator outputs. `chart.resampled_bars(target)` returns the derived
bars with their `session_id` and aggregated source-row count. Rust hosts call
`ChartEngine::configure_resampled_series(source, volume_source, target, volume_target,
ResampleOptions { interval_seconds, boundaries })` and
`aeris_charts_engine::resample_boundaries(&days, &windows, chart.exchange_time(),
ResampleSpan::Window)`.

## Close-time labels

Every bar is stamped with its OPEN time, the canonical Aeris bar time, and keeps that stamp as its
identity: rows, series data, crosshair events, snapshots, the countdown, replay, sessions, trading
days, drawings, markers, alerts, resampling, and every time a host passes in or reads back are
open-stamped. An A-share minute chart therefore holds 09:30 … 14:59. A user who reads bars by the
time they close expects 09:31 … 15:00. `time_scale_options.bar_time_label` (declaratively
`timeScale.barTimeLabel`, also accepted by worker charts) changes only the TEXT the chart prints for
a bar:

```ts
chart.time_scale().apply_options({
  time_zone: "Asia/Shanghai", time_visible: true,
  bar_time_label: {
    anchor: "close", interval_seconds: 60,
    windows: [["09:30", "11:30"], ["13:00", "15:00"]],   // optional, exchange-local
  },
});
```

The default `"open"` changes nothing. With `{ anchor: "close", … }` the printed time of a bar is its
open plus `interval_seconds` (1 to 86 399), or the end of the session window that contains its open
when the bar is that window's short last bar. Set `"open"` to restore the open text. `time_scale().options().bar_time_label` reports `"open"` or `{ anchor,
interval_seconds, windows }`. An invalid label (an interval outside 1..86 399, more than 32,
unordered, or zero-length windows for the chart's `session_start`, unknown keys) throws
`invalid_options` and changes nothing; the label is validated together with `time_zone`,
`session_start`, and `tick_marks` of the same call, against the session start that call installs.

**What prints the label.** The crosshair time label, automatic tick labels, the default text of
explicit `tick_marks`, drawing axis tags, drawing statistics (`date_time_range`) and the forecast
target time, the delta tooltip's time line, `create_tooltip`, and accessibility text. The host
`tick_mark_formatter` and `localization.time_formatter` receive the label instant. Hour and minute
tick weights follow the printed time, so the "10:00" tick sits on the bar that closes on the hour
(the bar opened 09:59) and never on the bar opened 10:00; Day, Month, and Year weights and every
trading-day reset keep following the bar's own trading day, so the last bar of a window that ends
at midnight prints 00:00 of the next date and still belongs to its own trading day. Worker charts
print the label on every engine-drawn surface; the package-owned tooltip and accessibility text are
main-thread surfaces. Labels are ordinary text: nothing in the frame contract, the draw list, or any
backend changes.

**What stays open-stamped.** All times in and out: `series_data`, `bars_in_logical_range`,
crosshair and click events, series snapshots, `coordinate_to_time`, visible ranges, the crosshair
sync position, markers, executions, alerts, drawing anchors, `tick_marks[].time`, the countdown,
the replay clock, session highlighting, and resampling. A host whose provider stamps bars by close
still converts them to open stamps on the way in (subtract one interval) and reads open stamps on the
way out. Native vertical-line labels keep their host text. The `time_formatter` of the
accessibility options receives the host's own data time, not the label. An explicit tick mark
names its bar by identity: to label the bar opened 11:29 (printing 11:30) pass `11:29`; a mark at
the label-only instant 11:30 matches no bar and draws nothing.

**One interval per chart.** `interval_seconds` is the chart's primary bar interval. A resampled
target and its source share one time axis, so a one-minute source with an hourly target cannot be
labelled per series: pick the interval of the bars the user reads, and update the option in the same
step as the timeframe switch (until then the labels use the old interval).

**Windows and short last bars.** `windows` are exchange-local `["HH:MM", "HH:MM"]` pairs placed in
the chart's `time_zone` and `session_start` exactly like `session_slot_times` (at most 32; an end at
or before its start crosses midnight, `"24:00"` ends at midnight). They end a window's last bar
exactly: a US hourly session 09:30–16:00 has a short 15:30 bar that prints 16:00 on both sides of a
DST change, and an HK morning window 09:30–12:00 prints 12:00 for its 11:30 bar. Without windows a
short last bar prints its open plus the interval (16:30). A bar whose open lies in no window prints
its open plus the interval as well, which is how a host-fed 241-bar feed that shifts the auction row
back one interval (09:29) prints 09:30, 09:31 … 15:00 with no engine-built auction bar. The windows
and the `session_start` must fit each other: while a label with windows is installed, a
`session_start` (through `apply_options`, `timeScale.sessionStart`, a V2 import, or
`set_session_start_seconds`) that the windows cannot be placed on throws `invalid_options`
(`ExchangeTimeError::BarTimeLabelWindows` in Rust) and changes nothing, so a saved document always
imports again. To move both, send them in one `apply_options` call (the label is checked against the
start of that call) or set the label to `"open"` first. A `time_zone` change never conflicts with the
windows; only an instant whose windows a DST transition collapses prints open plus the interval.
How 同花顺 and 富途 label a short last hourly bar could not be verified (no live terminal was
available), so compare the printed window end with your reference terminal before relying on it.

**Where it does not apply.** Calendar-date axes and non-time bar sequences (trade-count, volume,
and range streams, synthetic bars) print their own times and ignore the option. Interval bars do not
grow a separate 241st auction bar: engine-built candles fold the 09:25 auction print into the first
bar, as tick-built candles already do, and the 241 points of a time-sharing LINE remain a property
of close-stamped instant slots (`session_slot_times` with `"bar_close_with_open"`).

The label lives in the options store, so V2 persistence carries it while it is not `"open"`;
importing a document without the key keeps the installed label (a document whose `sessionStart` the
installed windows do not fit is rejected whole), and a default chart's document is unchanged. Rust
hosts call `ChartEngine::set_bar_time_label(BarTimeLabel::Close { interval_seconds, windows })`,
`bar_time_label()`, and `bar_label_time(open_time)` (the instant a bar prints); the engine JSON
option is `timeScale.barTimeLabel`, and the WASM export `bar_label_time(seconds)`.

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
`instanceof` checks when migrating. Rust consumers use the repository-only `aeris_charts_*` crates.

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
rejected `partial_ohlc`. A write to an engine-owned series (a trade-bound candle or study,
resampled or synthetic bars) is rejected with `derived_series` on every data path; a footprint
handle throws `unsupported_operation` instead. `series.merge(point, options?)` is the engine-owned
partial path: present open/high/low/close/value fields overwrite, absent fields keep the existing
bar, and candlestick/bar results are normalized so `high >= max(open, close)` and
`low <= min(open, close)` (a close-only tick for a new time creates O=H=L=C; scalar series take
`value`). A merge without a price field is
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
receiving chart resolves them on its own interval and history window. Every committed drawing change
(an API call, a placement, a freehand stroke, a pointer drag or keyboard edit that moved something,
a text edit) advances the sync revision, so an already synced cell accepts the next payload.
Clipboard payloads are bounded like a persisted drawing document (at most 10,000 drawings, 250,000
anchors, and 8 MiB): `copy_drawings` throws `resource_limit` past them and `invalid_data` when no
listed drawing exists, and `clone_drawing` copies any drawing the chart holds. Named templates
(`drawing_template`, `apply_drawing_template`) carry style only (plus a position tool's
`position_account_size` and `position_risk_percent`): never a drawing's name, group,
revision, visibility, lock, z-order, interval visibility, price scale, or text, so applying one
restyles the target and keeps its identity and its own text.

`chart.set_drawing_magnet_mode("off" | "weak" | "strong")` is the persistent toolbar magnet
(default `"off"`, which keeps the historical Ctrl/Cmd-only magnet). `"strong"` always snaps a placed
or edited anchor to the nearest rendered OHLC value of the bar under the pointer; `"weak"` snaps only
within 12 CSS px (`DRAWING_WEAK_MAGNET_DISTANCE`). A drawing's own `magnet` option raises the mode for
that drawing. Holding Ctrl/Cmd toggles the effective magnet (inactive becomes strong, active becomes
off). Touch input has no modifier and uses the chart mode. Keyboard nudges never snap.

`chart.add_drawing()` throws `invalid_options` for a malformed or out-of-range options patch instead of
dropping the options. Undo/redo during an active drag cancels the drag first. Keyboard editing
(`Enter`, `Tab`, arrows) cycles the drawing's editable handles (every anchor, a rectangle's eight
bounds handles, a Long/Short Position's target, entry, width, and stop controls, or the handles a
drawing family places on its geometry, listed with each family below) and moves the focused
handle by the nudge distance. `drawing_handle_count()` counts them. Every nudge applies live; `Enter`
commits the whole keyboard edit as one undo step and `Escape` restores the drawing as it was when the
edit began. A nudge that moves nothing (a locked drawing, an axis the drawing cannot move along, a
clamp at the pane edge) changes nothing and is announced as such.

A drawing's own text is edited in place in the chart's inline editor, for every drawing that paints
it: the text tool, a trend line's label, the text of every line, channel, Fibonacci, pitchfork,
pattern, and shape tool (one line, rotated along the stroke when the label follows a segment), and
the text boxes of the Projection & Annotations tools listed below (several lines). Level, point,
and wave labels, ratios, and stats are engine-formatted text and stay options-only. Nine tools
accept `text` but never paint or edit it on the chart: `forecast`, `bars_pattern`, `price_range`,
`date_range`, `date_and_price_range`, `projection`, `flag_mark`, `icon`, and `simple_tag` (whose
`text` is its price-axis tag). A double-click on a selected
drawing, or on the text of an unselected one (its first click selects it), or Enter or F2 while
the chart has focus and the drawing is selected (F2 on its accessibility drawing target, where
Enter keeps geometry editing), opens the editor; locked, hidden, and interval-hidden drawings do
not open it, nor does a drawing whose text lies wholly outside its pane's plot (the engine applies
this on every host and path: double-click, Enter, F2, placement, and a direct begin). The engine
decides which text is edited and where it sits, so an unselected drawing with no text has no label to
double-click: select it and double-click it, press Enter or F2, or use its options to add the first
label (only a trend line prompts `+ Add text` on hover). An unselected drawing's text answers hover
with the text cursor and a click with a selection, unless a higher drawing or the selected
drawing's anchor handle is at that point.
Typing repaints live, Enter or leaving the editor commits, and Escape restores the text. The whole
edit is one undo step and reaches `drawing_sync_payload` once, on commit. Text is bounded by
`MAX_DRAWING_TEXT_BYTES` (65,536 bytes: longer `text` in options is rejected without applying
the rest of the patch, and typing stops at the bound); the text tool, trend labels, and every
other run label stay on one line (line breaks become one space), while family text boxes take
several lines (Shift+Enter adds one, paste inserts plain text). The editor is a labeled text box
that announces opening and closing through the accessibility live region and returns focus to
where it was opened from. Placing the text tool, `anchored_text`, `note`, `callout`, `comment`,
`signpost`, or `simple_annotation` opens the editor at once with the caret after the default text; committing or Escape
keeps the drawing, even emptied (only the text tool removes itself when left empty). Placing a
`price_note`, `price_label`, or arrow mark, which start with no text of their own, opens nothing.

A double-click acts on the selected drawing only where a click would select it: on its text, its
body, or one of its handles. A pair whose first click landed on a trading object or the alert
widget acts on no drawing, and neither does a double-click elsewhere while a drawing stays
selected.

Host `dbl_click` subscribers still run after a double-click opened the editor. A host that binds
double-click to its own settings panel therefore sees both: the editor is open when the handler
runs, and calling `focus()` on a panel control closes it (the editor commits its text unchanged,
which records no undo step and no sync revision) and leaves focus on that control, so the drawing
is exactly as it was. A click on a host control while an editor is open closes it the same way and
leaves focus on that control; only Enter and Escape return focus to where the editor opened from
inside the chart.

A drawing's dashed or dotted `style` paints the same dashes on WebGPU, Canvas2D, GPUI, and native
rendering, and so does a general series' `line_style`: the engine splits those strokes into dash
runs before any backend draws them.

### Price-basis (复权) switches

The engine has no adjustment-factor model; the host computes adjusted OHLC. Drawings follow a basis
switch through three calls:

1. Replace the series data in the new basis (`series.set_data()`); indicators recompute.
2. Call `chart.rescale_drawing_prices(segments, basis_label)`. Each segment is
   `{from_time?, to_time?, factor}` (UTC seconds, `[from, to)`, non-overlapping, factor 1e-6..1e6).
   Each anchor price whose time falls in a segment is multiplied by that factor (on tick, volume,
   or range bar charts an anchor's time is the open time of the bar it sits on); Long/Short
   Position levels use the entry anchor's segment, and a Gann fan's or fixed square's
   `scale_ratio` (price per bar) scales with its first anchor's segment. This is a data-basis
   change, not an edit: it
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

## Drawing families

B8 drawing families extend the `drawing_kind` catalog. Their tools use the same placement,
selection, handles, drags, magnet, keyboard editing, anchor time identity, history, persistence,
clipboard, sync, and schema APIs as every other drawing. Some tools add handles on their geometry
beyond their anchors (listed with each family); those drag, magnet-snap, and keyboard-nudge like
anchor handles, and each drag, and each keyboard edit session, is one undo step, including any option it edits. Family-specific options live in one block
per family under `options.tool_options`; a patch deep-merges it (absent keys keep their values,
`null` resets a block, an invalid block rejects the whole patch with `invalid_options`). Schema
descriptors name those options with dotted paths such as `tool_options.line.stats_position`, and
`drawing_kind_options()` returns the resolved block. Kind defaults (for example a ray's
`extend_right`) are the schema defaults and are omitted from persistence.

<!-- B8: lines — begin -->
### Lines

- `ray`, `extended_line`, `info_line`, `trend_angle`, and `arrow_line` place two anchors. On these
  tools `extend_left` extends beyond the first anchor and `extend_right` beyond the second, each to
  the pane edge in the line's own direction; a ray defaults to `extend_right`, an extended line to
  both. End caps (`stroke_start`, `stroke_end`) paint only on ends that are not extended; the arrow
  line defaults `stroke_end` to `"arrow"`. The `text` label follows the segment like a trend
  line's and edits in place the same way; only a trend line prompts `+ Add text` on hover.
- Visible `labels` render as one stats box: price, price change, percent change, and ticks on one
  line; bar count, time range, and duration on the next; screen angle and CSS-px distance last.
  Values use the drawing scale's price formatter and the anchors' time identity. `info_line`
  enables price change, percent change, bar count, duration, and angle by default.
  `tool_options.line.stats_position` (`"start"`, `"middle"`, `"end"`; default `"end"`) places the
  box beyond the first anchor, below the midpoint, or beyond the second anchor. The box is a body
  target for selection and drags. `volume_in_range` renders nothing because drawings carry no
  volume source.
- `trend_angle` adds a dashed horizontal reference toward the second anchor, the arc to the
  segment, and the screen angle in degrees (rising positive, -90 to 90).
- `cross_line` places one anchor and paints full-span horizontal and vertical lines through it,
  with the horizontal line's price tag on the axis. Its body drags on both axes.
- `horizontal_segment` keeps both anchors on one price, and `vertical_ray` and `vertical_segment`
  keep both on one bar. Placing, dragging, or supplying an anchor moves the shared coordinate on the
  other, taken from the anchor placed or dragged last, so a supplied or imported pair that
  disagrees is repaired the same way. The vertical ray defaults to `extend_right`, which runs it
  from the first anchor through the second to the pane edge on the second anchor's side.
- `price_line` places one anchor and paints a crisp line from it to the right pane edge, with the
  anchor's price printed above the line's start and tagged on the price axis (KLineChart's price
  line). Its body is the ray. A `text` of its own is the generic line label, placed like a
  horizontal ray's, and does not replace the price.
- `drawing_kind_options()` returns `{ kind: "line", stats_position }` for every Lines tool.
<!-- B8: lines — end -->
<!-- B8: channels — begin -->
### Channels

- `parallel_channel`, `flat_top_bottom`, and `disjoint_channel` place three anchors. The first two
  define the base line. The second line spans the same bars and lies on the line through the third
  anchor, whichever bar that anchor sits on: the base line moved vertically on screen for the
  parallel channel (parallel on every scale mode), a horizontal line at the third anchor's price for
  flat top/bottom, and the base line's slope mirrored for the disjoint channel. `extend_left` and
  `extend_right` extend both lines and the fill to the pane edge beyond the first and second
  anchor. Placement previews the base line after the first click and the whole channel after the
  second.
- `price_channel` is KLineChart's price channel: the base line through the first two anchors is the
  centre, the second line passes through the third anchor parallel to it, and the third line mirrors
  the second on the other side of the base. It defaults to `extend_left` and `extend_right` with no
  fill (`fill_enabled: true` shades the whole band), and has no middle line.
- The fill between the lines is on by default (`fill_enabled`); `fill_color` defaults to the stroke
  color at 20% alpha. Where the lines cross (flat top/bottom, disjoint channel) the fill meets at
  the crossing. Lines are body targets; the fill is a drag surface only while the drawing is
  selected, like the rectangle's. The `text` label follows the base line like a trend line's.
  Channel lines ignore `stroke_start` and `stroke_end`.
- `tool_options.channel.middle_line` paints a dashed 1 px line halfway between the two lines (default
  on for the parallel channel, off for the others) in `middle_color` (`""` follows `color`).
- `regression_trend` places two anchors that choose a bar range (rounded positions, inclusive); its
  body and handles move along time only. The engine fits a least-squares line to the source series
  over those bars — the first ordinary series added to the drawing's pane and price scale that is
  still live (indicator outputs and custom series never qualify, footprint and feature series do
  through their OHLC projection; reordering or hiding a series does not change the source, and
  neither does removing and re-adding other series) — and paints it dashed (`middle_line`, `middle_color`) with lines `upper_deviation` (default 2) and
  `lower_deviation` (default -2) residual standard deviations away (sample deviation, `n − 1`),
  each toggled by `use_upper_deviation` and `use_lower_deviation`, the zones between them filled,
  and Pearson's R (the signed correlation of bar position and value, four decimals) below the start
  unless `show_pearsons` is false. `source` selects the bar value (`indicator_input_source`, default
  `"close"`). The lines follow streaming updates of the source; replacing the latest bar or appending
  bars costs the changed rows, not the anchored range. On an as-of (`time_alignment: "as_of"`)
  source the fit reads each of the source's own bars in the range once, not the repeated axis
  points. A range without source bars paints
  the dashed anchor segment. The anchors' prices are stored but do not shape the lines, and the
  `text` label sits in the anchors' box. Default width 1; the other channels default to 2.
- Handles sit on the painted lines, one per anchor in anchor order (`drawing_handle_count` 3 or
  2): the base line's two ends, the second line's midpoint for the third anchor (dragging or
  nudging it moves the second line), and the regression line's two ends, which follow the fit
  (while placing too). Shift-dragging a base-line end straightens the base line like a trend
  line's.
- A template from `drawing_template()` replaces the drawing's `tool_options.channel` when applied,
  so options the template leaves at their defaults reset as well.
- `drawing_kind_options()` returns `{ kind: "channel", middle_line, middle_color }` for the three
  click-placed channels and `{ kind: "regression_trend", ... }` with every resolved regression
  option. `tool_options.channel` stores only the fields that were set.
<!-- B8: channels — end -->
<!-- B8: fibonacci — begin -->
### Fibonacci

| Tool | Anchors | Geometry |
| --- | --- | --- |
| `fib_retracement` | 2 | Horizontal levels between the anchors' prices: level 0 on the second anchor, 1 on the first, extensions beyond. Levels span the anchors' times. |
| `trend_based_fib_extension` | 3 | The first leg's move projected from the third anchor (level 0 at the third anchor, 1 one full move away), spanning the first leg's width from the third anchor. |
| `fib_channel` | 3 | Lines parallel to the first leg; level 1 passes through the third anchor. |
| `fib_time_zone` | 2 | Full-height lines at 0, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89 times the anchors' time distance from the first anchor. |
| `trend_based_fib_time` | 3 | Full-height lines at ratio multiples of the first leg's duration from the third anchor. |
| `fib_speed_resistance_fan` | 2 | Rays from the first anchor through the second anchor's time at each price ratio and through its price at each time ratio, to the pane edge, plus the ratio grid inside the anchors' box. |
| `fib_speed_resistance_arcs` | 2 | Arcs around the first anchor, radius ratio × the anchors' screen distance, on the second anchor's side (full circles optional). |
| `fib_circles` | 2 | Circles around the anchors' midpoint; level 1 passes through both anchors. |
| `fib_spiral` | 2 | A golden spiral around the first anchor through the second, growing by φ every quarter turn, clockwise on screen. |
| `fib_wedge` | 3 | Ratio arcs around the first anchor between the edges toward the second and third anchors; level 1 at the second anchor's distance. |

- The level list is the common `levels` option: `value`, `color`, `visible`, `style`
  (`"solid"`, `"dotted"`, `"dashed"`), `fill_between`, optional `fill_color`, and
  `label_visible`, at most 64 levels. Retracement, extension, and channel default to
  TradingView's visible retracement levels 0, 0.236, 0.382, 0.5, 0.618, 0.786, 1, 1.618, 2.618,
  3.618, 4.236 and its palette (0 and 1 gray, 0.236 red, 0.382 orange, 0.5 green, 0.618 teal,
  0.786 cyan, 1.618 blue, 2.618 red, 3.618 purple, 4.236 pink). The other tools use the
  conventional tables: trend-based time 0, 0.382, 0.5, 0.618, 1, 1.382, 1.618, 2, 2.382, 2.618, 3;
  the fan 0, 0.25, 0.382, 0.5, 0.618, 0.75, 1; arcs and circles 0.236 through 4.236; the wedge
  0.236 through 1. Values from the retracement table keep its colors; other values take the
  palette in list order. The spiral has no levels.
- Visible levels sort by value. `fill_enabled` is the background switch (on by default except
  for time zones and the spiral); the band between two neighbouring levels takes the upper
  level's `fill_color`, or its color at 20% opacity, when that level's `fill_between` is on. Bands
  select and drag the drawing only while it is selected; level lines, the trend line, and labels
  always do.
- The drawing's own `color`, `width`, and `style` (default `#787b86`, 1 px, dashed) are the
  trend line through the anchors, the fan's grid, and the wedge's edges (solid); the spiral is
  drawn in them. Level lines use each level's color and style at the drawing's width.
- `extend_left` and `extend_right` extend the retracement's, extension's, and channel's levels to
  the pane's left and right edges.
- Level labels show the value and, for the retracement and extension, the price through the
  drawing scale's formatter, for example `0.618 (102.53)`, in the level's color.
- `tool_options.fibonacci` (`fibonacci_tool_options`): `reverse` (swap the ends levels 0 and 1
  sit at; time zones project backward; the spiral turns counterclockwise), `show_levels`,
  `show_prices`, `levels_as_percent` (`61.8%`), `log_scale` (price levels interpolate in log
  space), `trend_line`, `grid` (fan), `full_circles` (arcs), `label_h_align` and `label_v_align`
  (price levels default to `"left"`/`"middle"`: beyond the left end, centered on the line; time
  levels to `"right"`/`"bottom"`). Each tool's schema lists only the fields it reads, with its
  resolved defaults. `drawing_kind_options()` returns `{ kind: "fibonacci", ... }` with every field
  resolved.
- Culling follows the painted levels: price and time levels beyond the anchors keep a drawing
  visible and hittable while its anchors are scrolled away. The fan, arcs, circles, spiral, and
  wedge depend on the screen distance between their anchors, so they are culled by the pane only.
<!-- B8: fibonacci — end -->
<!-- B8: pitchforks_gann — begin -->
### Pitchforks and Gann

- `andrews_pitchfork`, `schiff_pitchfork`, `modified_schiff_pitchfork`, and `inside_pitchfork`
  place three anchors: the pivot, then the two ends of the handle. The median starts at the pivot
  (Schiff: at the pivot's time, halfway between the first two anchors' prices; modified Schiff and
  inside: at the midpoint of the first two anchors). It runs through the handle's midpoint, or for
  the inside pitchfork through the third anchor, whose tines pass through the second anchor and
  its reflection about the third. Between clicks a guide joins the placed anchors and the pointer.
  Besides one handle per anchor, the pitchforks and the pitchfan have a fourth handle on the
  midpoint between the second and third anchors, which moves both together
  (`drawing_handle_count` 4).
- A pitchfork's `levels` are median offsets in half-handle widths: level `v` is a tine on each
  side of the median, and level 1 passes through the handle's ends. Defaults: 0.25, 0.382, 0.5,
  0.618, 0.75, 1, 1.5, 1.75, and 2, with 0.5 and 1 visible; `fill_between` zones filled at 20% of
  the level color (`fill_enabled`); level labels off; median `color` `#f23645`; width 1.
  Unextended lines reach one median length past the handle; `extend_left` and `extend_right` run
  every line to the pane edge. The shifted-pivot variants add a dashed guide between the first two
  anchors. Like a rectangle's interior, the zone fills of every tool in this family (pitchfork and
  fan zones, Gann box zones, square arcs) are drag targets only while the drawing is selected.
- `pitchfan` places three anchors and draws the same levels as rays from the first anchor through
  the level points on the handle between the other two.
- `gann_box` places two corners. Its `levels` are horizontal price levels and
  `tool_options.gann.time_levels` vertical time levels, both as fractions of the box from the first
  corner (defaults 0, 0.25, 0.382, 0.5, 0.618, 0.75, and 1, filled and labeled on all four sides).
  `show_angles` adds the `angles` fan from the pivot corner. It has eight bounds handles, and
  Shift squares it on screen.
- `gann_square` places two corners and draws the `levels` grid (default fifths of each side), the
  `angles` fan (1×8 through 8×1, the 1×1 on the diagonal), quarter `arcs` around the pivot corner
  (fifths of the side, filled), and a box with the price range, bar count, and price per bar. Like
  the box, it has eight bounds handles and Shift squares it on screen.
  `gann_square_fixed` places one anchor: the square is `size_bars` wide and `size_bars ×
  scale_ratio` tall in price, or square on screen without a ratio. Its second handle, the far
  corner, resizes it: the corner's bar sets `size_bars` in whole bars (at least 1), dragging it
  below the anchor sets `reverse`, and with a `scale_ratio` the corner's price sets the ratio
  (Shift keeps it); without one the square stays square on screen and follows the corner's larger
  distance from the anchor. A keyboard nudge of the corner moves the side at least one whole bar
  the way the arrow points (without a ratio, sized by the arrow's axis), so repeated presses keep
  resizing however wide a bar is.
- `gann_fan` places two anchors. Its `levels` are multiples of the 1×1 slope (defaults 1/8, 1/4,
  1/3, 1/2, 1, 2, 3, 4, and 8, labeled `8x1` through `1x8`, zones filled). The 1×1 passes through
  the second anchor, or with `scale_ratio` rises that many price units per bar. Lines are rays by
  default (`extend_right`); unextended, they stop at the anchors' box. Shift straightens the
  second anchor to 45°.
- `tool_options.gann` (absent fields keep their defaults):

  | Field | Tools | Default |
  | --- | --- | --- |
  | `time_levels` | Gann box | 0 … 1 as above |
  | `angles` | Gann box (with `show_angles`), squares | 1/8 … 8, positive values |
  | `arcs` | squares | 0.2, 0.4, 0.6, 0.8, 1, positive values |
  | `reverse` | Gann box, squares | `false`; `true` measures from the second anchor's price (the box also counts time from it) and grows the fixed square down |
  | `show_angles` | Gann box | `false` |
  | `show_stats` | squares | `true` |
  | `scale_ratio` | Gann fan, fixed square | `null`: price per bar of the 1×1, positive |
  | `size_bars` | fixed square | 20, from 1 to 100000 |

  Each tool's schema lists only the fields it uses. `drawing_kind_options()` returns
  `{ kind: "pitchfork", levels }` for the pitchforks and the pitchfan, and `{ kind: "gann", levels,
  time_levels, angles, arcs, reverse, show_angles, show_stats, scale_ratio, size_bars }` for the
  Gann tools.
<!-- B8: pitchforks_gann — end -->
<!-- B8: projection_annotations — begin -->
### Projection and annotations

Defaults follow the conventional professional-platform look: the drawing `color` (the canonical
primary unless noted) paints markers, leaders, and box backgrounds; box text is `text_color` or
black/white contrast against the box; `box_border_color` frames annotation boxes; text uses the
chart font size unless `text_size` is set. Every tool owns its `text` (it does not follow the
3×3 box label of other tools) and renders any visible `labels` as engine-formatted stats. Eight of
the tools paint no text: their `text` is accepted and kept, never shown or edited in place.

- `forecast` (source, target): a segment with end caps from `stroke_start`/`stroke_end`, a source
  dot and a source-price box on the far side, and a target box with the change and percent, the
  target time, and the outcome. The outcome comes from the drawing's source series (the regression
  trend's rule: the first live ordinary series added to its pane and price scale) and follows its
  streaming updates: `Success` (market-up box) once a bar after the source bar reaches the target
  price (a high for a rising target, a low for a falling one) by the target bar (an as-of source's
  own bars, including those that collapse between two axis points; a point that repeats the source
  bar is not a later bar), `Failure`
  (market-down box) once a traded bar after the target bar exists without that (a target on the
  latest, possibly still-forming bar stays pending, and whitespace rows such as future session
  slots are not bars), and no outcome (the drawing color) while pending.
- `bars_pattern` (two anchors): when placed with the armed tool (which previews the copy in place)
  or created by `add_drawing` without `bars`, it copies the bars between its anchors' bar indexes
  (at most 128; a longer range aggregates into 128 OHLC buckets) into
  `tool_options.projection_annotation.bars` and pins its anchors on the copy's box — the first
  copied bar at the copy's highest value and the last at its lowest — so the ghost starts exactly
  over its source. The ghost always fills the box between its anchors: moving it moves the copy,
  and the anchors stretch it in time and scale it in proportion to the copy's full range (a price
  basis rescale scales it exactly). `bars_mode` is `"hl_bars"` (default), `"oc_bars"`,
  `"line_open"`, `"line_high"`, `"line_low"`, or `"line_close"`; `mirrored` reverses the copy in
  time and `flipped` turns it upside down within the box. Paste, sync, and persistence carry the
  copied bars; a named template keeps only the style, so applying one never replaces a pattern's
  copy and the armed tool always copies its own range. On an as-of source it copies the source's
  own bars, each once. A pattern created before any data has no
  copy and paints a dashed box.
- `price_range`, `date_range`, `date_and_price_range` (two anchors): a fill between the anchors
  (`fill_enabled` defaults on; `fill_color` or the drawing color at 20%), the edge lines of the
  measured axis, arrowed measures through the middle toward the second anchor (`stroke_end`
  defaults to `"arrow"`), and a stats box beyond the measured end (below a date range). Default
  `labels`: price change, percent change, and ticks; bar count and duration; or all five. Anchors
  snap to whole bars and price ticks (also while dragging, nudging with the keyboard, or moving the
  body); ticks count on the instrument tick or price-band ladder, falling back to the scale's
  `min_move`. `"date_price_range"`, the spelling of earlier builds, is read as
  `"date_and_price_range"` and never written. The Shift-click quick measure draws a transient
  date-and-price range.
- `projection` (apex, radius point, price point): the circular sector around the apex from
  the ray through the radius point to the ray through the price point (the shorter turn), filled
  (`fill_enabled` defaults on) and outlined. Visible `labels` measure from the apex to the price
  point. While placing, the first click shows the apex as a handle with a provisional line to the
  pointer; after the second click the sector previews through the pointer, and the third click
  commits it.
- `anchored_text` (one anchor): text pinned to a pane position. Its anchor is a pane fraction —
  `logical` is x / pane width and `price` is y / pane height from the pane top — so it stays put
  while the chart scrolls, zooms, rescales, or changes interval. Its anchors carry no `time`
  (`time` inputs are ignored and a time-only anchor is rejected), fractions clamp into `0..=1`
  (in `add_drawing`, `set_points`, paste, sync, and drags, so it always stays reachable) while an
  `import_state` document with a fraction outside `0..=1` is `invalid_data`, and magnets and paste
  offsets and group moves do not apply. Default text `"Text"`, aligned left/top on the
  anchor; `box_color`/`box_border_color` box it like the text tool.
- `note` (one anchor): a pin whose tip is the anchor, with the text (default `"Note"`) in a box
  beside the head. The box shows while the note is hovered, selected, or being edited;
  `tool_options.projection_annotation.always_show_text` (default `false`) keeps it visible.
- `price_note` (two anchors): a leader from the priced point to a box with its price (and any
  text) at the second anchor.
- `callout` (tip, box): a text box (default `"Callout"`) placed on the second anchor by
  `text_h_align`/`text_v_align` (default centered), with a pointer to the first anchor.
- `comment` and `price_label` (one anchor): a speech bubble whose tail tip is the anchor; the
  comment shows its text (default `"Comment"`), the price label the anchor's price and any text.
- `signpost` (one anchor): a pole from the anchor up to a text plate (default `"Signpost"`).
- `flag_mark` (one anchor): a flag standing on the anchor.
- `arrow_mark_up`, `arrow_mark_down`, `arrow_mark_left`, `arrow_mark_right` (one anchor): a block
  arrow whose tip is the anchor, with any text past its tail in `text_color` or the arrow color.
  Up defaults to the market-up color and down to the market-down color.
- `simple_tag` (one anchor): KLineChart's simple tag: a dashed line across the whole pane at the
  anchor's price, tagged on the price axis. The tag shows the drawing's `text` when it has any
  and the price otherwise; the text is not painted on the chart, so it has no inline editor (set
  it through `text` in the options).
- `simple_annotation` (one anchor): KLineChart's simple annotation: a dashed stem rising from the
  anchor to a small head, with the `text` in a box above the head (it starts empty). Placing it
  opens the editor.
- `icon` (one anchor): `tool_options.projection_annotation.icon` — `"star"` (default), `"heart"`,
  `"check"`, `"cross"`, `"circle"`, `"square"`, `"diamond"`, `"triangle_up"`, or
  `"triangle_down"` — centered on the anchor, `icon_size` CSS px across (8..128, default 24), in
  the drawing color.
- Kind defaults (fills, arrows, stats, default texts, arrow colors) are schema defaults and are
  omitted from persistence; a cleared default text persists as `""`.
- The text of `anchored_text`, `note`, `price_note`, `callout`, `comment`, `price_label`,
  `signpost`, `simple_annotation`, and the arrow marks edits in place (see inline text editing
  above); the price note and price label keep their price line above it. An emptied box keeps one
  caret line while it is edited. Placing `anchored_text`, `note`, `callout`, `comment`, `signpost`,
  or `simple_annotation` opens the editor (on the default text, where the tool has one); placing
  `price_note`, `price_label`, or an arrow mark does not.
- `drawing_kind_options()` returns `{ kind: "projection_annotation", bars_mode, mirrored, flipped,
  pattern_bars, icon, icon_size, always_show_text }` for every tool of the family.
<!-- B8: projection_annotations — end -->
<!-- B8: patterns_elliott_cycles — begin -->
### Patterns, Elliott waves, and cycles

Every tool places a fixed number of anchors by clicking, previews the legs placed so far while it is
being placed, and exposes one handle per anchor. Defaults follow TradingView's: colors per tool,
width 2 (1 for cyclic lines), and region fills at 15% of the drawing color when `fill_color` is
empty. Point and ratio labels use the drawing's text size, weight, and italic (`text_color`
overrides their contrasting default); they are body targets.

| Tool | Anchors | Default color | Paints |
| --- | --- | --- | --- |
| `xabcd_pattern` | X, A, B, C, D | `#2962FF` | Legs, shaded XAB and BCD, ratios AB/XA, BC/AB, CD/BC, AD/XA |
| `cypher_pattern` | X, A, B, C, D | `#2962FF` | Legs, shaded XAB and BCD, ratios AB/XA, XC/XA, CD/XC |
| `abcd_pattern` | A, B, C, D | `#089981` | Legs, ratios BC/AB and CD/BC |
| `head_and_shoulders` | base, left shoulder, neck, head, neck, right shoulder, base | `#089981` | Legs, neckline between the outer legs, shaded shoulders and head, part labels |
| `triangle_pattern` | A, B, C, D (alternating highs and lows) | `#673AB7` | Legs, A–C and B–D sides extended to their apex when it lies ahead within one pattern width, shaded triangle |
| `three_drives_pattern` | start, drive 1, retracement, drive 2, retracement, drive 3, end | `#673AB7` | Legs, drives labeled 1–3, each leg's ratio to the leg before it |
| `elliott_impulse_wave` | 0, 1, 2, 3, 4, 5 | `#3D85C6` | Waves labeled 1–5 |
| `elliott_correction_wave` | 0, A, B, C | `#3D85C6` | Waves labeled A–C |
| `elliott_triangle_wave` | 0, A, B, C, D, E | `#FF9800` | Waves labeled A–E |
| `elliott_double_combo` | 0, W, X, Y | `#6AA84F` | Waves labeled W, X, Y |
| `elliott_triple_combo` | 0, W, X, Y, X, Z | `#6AA84F` | Waves labeled W, X, Y, X, Z |
| `cyclic_lines` | cycle start, cycle end | `#80CCDB` | Dashed connector and full-height vertical lines every interval from the earlier anchor to the right edge |
| `time_cycles` | cycle start (base), cycle end (sets the arch height) | `#159980` | Half-ellipse arches of the anchors' width and height, repeated both ways, shaded |
| `sine_line` | a peak or trough, the next opposite extreme | `#159980` | A sine through both anchors across the pane |

- Ratios are price ratios printed with three decimals on dashed connectors;
  `tool_options.pattern.show_ratios: false` hides connectors and ratios.
- `tool_options.pattern.degree` selects the Elliott wave degree: `"supermillennium"`,
  `"millennium"`, `"submillennium"`, `"grand_supercycle"`, `"supercycle"`, `"cycle"`,
  `"primary"`, `"intermediate"` (default), `"minor"`, `"minute"`, `"minuette"`, or
  `"subminuette"`. Wave 3 and wave C read `{III}`/`{c}`, `[III]`/`[c]`, `<III>`/`<c>`, ringed
  `III`/`c`, `(III)`/`(c)`, `III`/`c`, ringed `3`/`C`, `(3)`/`(C)`, `3`/`C`, ringed `iii`/`c`,
  `(iii)`/`(c)`, and `iii`/`c` in that order. `tool_options.pattern.show_wave: false` leaves only
  the labels.
- Cycle repeats closer than 3 CSS px collapse to the defining cycle.
- `fill_enabled` and `fill_color` shade XABCD, cypher, head and shoulders, triangle pattern, and
  time cycles; region fills are a body target only while the drawing is selected. `extend_left`,
  `extend_right`, `stroke_start`, and `stroke_end` do not apply to these tools, and fill does not
  apply to the other tools. The `text` label is placed against the anchors' box.
- `drawing_kind_options()` returns `{ kind: "pattern", show_ratios }` for XABCD, cypher, ABCD, and
  three drives, `{ kind: "elliott_wave", degree, show_wave }` for the Elliott tools, and
  `{ kind: "generic" }` for head and shoulders, the triangle pattern, and the cycle tools.
<!-- B8: patterns_elliott_cycles — end -->
<!-- B8: shapes — begin -->
### Shapes

- `rotated_rectangle` places three anchors: the midpoints of the two short sides, then a point on a
  long side, whose distance from that axis sets the width. It stays right-angled on screen at any
  zoom. Its handles are the two axis ends and a width handle at the midpoint of each long side
  (`drawing_handle_count` 4); the third anchor has no handle of its own. A width handle sets the
  width to its distance from the axis, and dragging an axis end keeps the width on screen, so
  turning the rectangle never flattens it. After a width drag the third anchor sits at its long
  side's midpoint.
- `ellipse` places two box corners and is inscribed in their box; it edits with the rectangle's
  eight handles, and Shift keeps the box square (a circle). `circle` places its center and a point
  on its rim. `triangle` places three vertices.
- `arc` places its start, its end, and a point it passes through (collinear points give the
  straight chord). `curve` places its start, its end, and the point it passes at its middle;
  `double_curve` places its start, its end, and the points it passes at one and two thirds. Every
  handle sits on the curve, and `extend_left`/`extend_right` continue a curve's end tangents to the
  pane edge. While a three- or four-anchor shape is placed, the anchors clicked so far and the
  pointer show as a polyline in the drawing's stroke until every anchor but the last is placed; the
  shape itself then previews through the pointer until the last click commits it.
- `polyline` places vertices like `path`: click to add, double-click or Enter to finish, Backspace
  removes the latest vertex, Escape cancels. Once three vertices are placed, clicking the first
  vertex again finishes the polyline closed (the preview snaps shut while the pointer is over it).
  `tool_options.shape.closed` (default `false`) joins the last vertex to the first and fills the
  enclosed region by the nonzero rule. The fill is bounded work: a closed polyline of more than
  2,048 vertices, or one so heavily self-intersecting that its fill exceeds the tessellation
  bounds, paints its outline only, with no fill and no interior selection target (its stroke still
  selects it). This is not an error, every vertex is kept, and it is identical on every backend; a
  region that follows thousands of bars of chart data belongs in a series rather than in a
  drawing polyline.
- `highlighter` is a freehand drag like `brush`: a 20 px marker stroke in 40% amber with round
  ends. It keeps one opacity where it overlaps itself on every backend, and ignores `style`, end
  caps, and fill.
- Strokes default to 2 px. The rotated rectangle, ellipse, circle, triangle, arc, and polyline
  default `fill_enabled` to `true` and fill with `fill_color`, or the stroke color at 20% opacity
  when unset; the arc fills the segment between the arc and its chord, curves fill the region
  between the curve and its chord once enabled, and an open polyline never fills. A shape's fill
  selects and drags it only while it is selected, so an unselected shape's interior keeps panning
  the chart. End caps (`stroke_start`, `stroke_end`) apply to the open shapes: arc, curves, and the
  open polyline.
- Box text (`text`) aligns against the shape's own box, such as a circle's rather than the box of
  its center and rim anchors.
- `drawing_kind_options()` returns `{ kind: "shape", closed }` for every Shapes tool; the
  `tool_options.shape.closed` schema descriptor is listed for `polyline` only.
<!-- B8: shapes — end -->

### Equivalents of KLineChart's overlays

A host moving from KLineChart finds each of its drawing overlays here. Seven are tools of their own
(wire ids 38..=41, 52, 147, and 148); the others are an existing tool with options, and the table
says how.

| KLineChart overlay | Aeris tool |
|---|---|
| `straightLine` | `extended_line` |
| `rayLine` | `ray` |
| `horizontalSegment` | `horizontal_segment` |
| `verticalRayLine` | `vertical_ray` |
| `verticalSegment` | `vertical_segment` |
| `parallelStraightLine` | `parallel_channel` with `extend_left` and `extend_right`, `fill_enabled: false`, and `tool_options.channel.middle_line: false` |
| `priceChannelLine` | `price_channel` |
| `fibonacciLine` | `fib_retracement` with `extend_left` and `extend_right` (levels span the pane) |
| `priceLine` | `price_line` |
| `simpleTag` | `simple_tag` |
| `simpleAnnotation` | `simple_annotation` |

## Persistence V1

Persistence schema versioning is independent of the npm package version. V1 contains only:

- ordered pane identities, stretch factors, and preserve-empty flags;
- ordered built-in drawings with persistent ID, kind, pane reference, semantic anchors
  (`{logical, price, time?}`, plus the `anchor_times_micros` sidecar on non-time bar charts), and
  style;
- the optional top-level `drawing_price_basis` label (the host-defined price basis of the drawing
  prices);
- the optional top-level `hidden_mark_groups` list (hidden timeline-mark groups, at most 64 ids of
  at most 128 bytes; omitted when empty). Timeline marks themselves are never persisted.

Host market history, series and indicator definitions, chart options, trading positions/orders/
executions/previews/intents, alert lines/create requests, custom extensions, callbacks,
subscriptions, selections, interaction sessions, generations, LOD, drawing bounds/indexes,
retained frames, and GPU resources are not persisted. (V2 documents, written for charts with
general panes, additionally carry the chart options store — crosshair `vertLine`, `horzLine`, and
`shadeRight` included; V3 documents, like V1, carry no chart options.) Hosts restore V1 into a fresh chart, then
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

Drawing style fields are optional and restore the kind's own defaults when omitted; the export
writes a field only when it differs from them. B8 family options travel in the optional
`style.tool_options` object (at most 16 KiB serialized), so older documents need no migration.

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

The Rust crates are repository-only (`publish = false`); nothing is published to crates.io, and the
browser package is the only published artifact. Hosts such as Aeris Terminal consume the
`aeris_charts_*` crates through pinned Git revisions or local paths. The Rust API is below 1.0 and
may change in any revision, so a host reviews the notes below when it moves its pin.

`aeris_charts_render_gpui` is experimental. It pins `gpui-pre` 0.3.7, the GPUI snapshot gpui-kit
0.7.0 depends on, with an exact version requirement, so a host that draws the chart must use that
same `gpui` (a host on another GPUI build, such as a Zed Git revision, holds two incompatible copies
of its types). GPUI upgrades are explicit manifest and lockfile changes.

On macOS a host must build its GPUI platform crate (`gpui-pre-platform`, or `gpui-pre-macos`
directly) with the `font-kit` feature, which is GPUI's macOS text system. Without it GPUI substitutes
a no-op text system: the chart paints no text (axes, labels, legends, drawing text) and measures
every string as zero width, and the only signal is a `log::warn!` at startup. Linux and Windows are
unaffected.

A host that binds wheel events through `GpuiChartInput::scroll_wheel` gets browser-equivalent
scrolling. Hosts that pinned an earlier revision panned the time scale the wrong way on a horizontal
wheel or trackpad swipe: the adapter passed GPUI's horizontal delta through unflipped, though GPUI
reports content motion (positive reveals the left) and the engine, like a browser, takes positive
as a move to the right. The adapter now flips the horizontal axis; a host that compensated for the
old sign itself must remove its compensation.

### Moving the pinned revision

These notes list the host-visible changes a pinned-revision move carries, so call sites can be
reviewed once. Each group names the revision-level change and the call sites it affects.

Each group is keyed to the commit that carries its change. A group applies to a pin that does not
contain that commit (`git merge-base --is-ancestor <commit> <pin>` exits non-zero); a pin that
already contains it has taken the change. Hashes are this repository's. "Upstream" marks commits
that came from `AerisTerminal/aeris-charts` main and keep their hashes here, because upstream was
merged and never rebased; "own line" marks commits that exist only in this repository. The two
histories part at `ed2910d`, and a group whose old spelling lived on one side only says which side.

**Sub-pane coordinates** (own line, from `84b85e6 fix(engine): sub-pane crosshair sync,
chart-level pane selection, coordinate contract`; see [Coordinates and panes](#coordinates-and-panes)).
Three behaviours changed:

- The chart-level `price_to_coordinate` and `coordinate_to_price` no longer follow the first visible
  series in creation order. The price converts on pane 0's default scale and the coordinate on the
  default scale of the pane containing `y`, so a call that relied on an overlay-first or hidden main
  series must use that series' own handle instead. Single-pane charts whose main series is created
  first are unchanged, and series-handle conversions never changed.
- Linked crosshairs on a lower pane (`pane_index` of 1 or more) now round-trip. Earlier revisions
  applied the pane offset twice and read the wrong scale, so `crosshair_sync_position` and
  `apply_external_crosshair` disagreed on any pane but the first. A synced price is now a price on
  the pane's default scale, and an off-range price holds the line inside its pane.
- `ChartEngine::pane_index_at_y` (and the browser package's `pane_index_at_y`) now returns the pane
  above for a separator and pane 0 for a `y` above the content, where both used to resolve to the
  last pane. GPUI hosts that pick a pane for price-axis hit-testing pick up the corrected mapping.

**Trade-stream-derived series** (own line, from `a51242c fix(engine): guard trade-stream-derived
series against host writes`; see [Ticks to candles](#ticks-to-candles)). Two reviews, and the first
cannot be checked from this repository:

- Terminal must not write to a candle bound with `bind_trade_bar_series_to_stream` or to a CVD,
  delta, or trade-volume series (`add_cvd_series`, `add_delta_series`, `add_trade_volume_series`).
  Every host data write to them is now refused like a footprint's (`false`, `0`, `None`,
  `Err(UnsupportedSeriesData)`, or `Rejected(UnsupportedSeries)`; `series_is_source_owned(id)` tells
  the refusal from an unknown id), and `apply_momentum_histogram_colors` returns `false` for the
  delta and volume studies. The browser package rejects them with `code: "derived_series"`.
- `FootprintError` gains the `SeriesOwned(SeriesId)` variant, so an exhaustive `match` on it needs
  an arm. `bind_trade_bar_series_to_stream` returns it for a candlestick or bar that a resampler,
  synthetic bars, or a study converted to a candle already writes (a footprint or scalar study still
  gets `UnsupportedTradeBarSeries`, because the candle-kind check runs first), and
  `configure_footprint_series` returns it for a series a stream, study, resampler, or synthetic bars
  write. Resampling targets and synthetic-bar series refuse a trade-bound candle
  (`ResampleError::UnsupportedTarget`, `SyntheticBarError::UnsupportedSeries`).

**Batched period-reset study lines** (own line, from `fafecda perf(render): batch period-reset
study segments into one Segments primitive`). This is a compile-time break for Rust code that
matches `aeris_charts_render::draw_list::Prim` exhaustively (a custom executor, a frame inspector,
a point-pool rebase):

- `Prim` gains `Segments { first_point, segment_count, width, color }`, a batch of `segment_count`
  independent two-point strokes over `points[first_point .. first_point + 2 * segment_count]`, each
  stroked like a solid simple two-point `Polyline` (dashes already expanded into one pair per dash).
  The engine emits it in place of one two-point `Polyline` per bar for session VWAP, VWAP bands, and
  pivot lines on bars of a day or longer, so a `_ => {}` arm that compiles silently stops drawing
  those studies. Take the pair window from `draw_list::segment_points` (a range outside the pool is
  a dropped prim), and move `first_point` with every other pool index when rebasing a layer. The
  Canvas2D, WebGPU, GPUI, and native executors in this repository already handle it. Because hosts
  take the change by moving their pin, it is breaking for exhaustive matchers.

**Named time zones** (both lines, from the merge `2e7d19f merge: sync with
AerisTerminal/aeris-charts main`, which took upstream's `f796529 feat(time): add selectable IANA
chart time zones` onto the own line's exchange-time clock; see
[Time, exchange time zone, and trading sessions](#time-exchange-time-zone-and-trading-sessions)).
Behaviours to review:

- `ChartEngine::set_time_zone` takes an IANA id from `TRADINGVIEW_TIME_ZONES`
  (`Result<bool, String>`; `Ok(false)` when it is already installed). The setter that takes a
  `UtcOffsetSchedule` is `set_exchange_offsets`, so a call site written against a revision where
  `set_time_zone` took a schedule must use the new name. Only an own-line pin before the merge took
  a schedule; an upstream pin from `f796529` on already passes an id.
- An upstream pin from `f796529` on has zone-aware helpers in `aeris_charts_core`, each of which
  takes a `ChartTimeZone`: `format_tick_label_with_time_zone`, `format_date_pattern_with_time_zone`,
  `format_crosshair_time_with_time_zone`, `weight_by_time_in_time_zone`, and
  `fill_weights_for_points_in_time_zone`. The merge deleted them and kept the own line's
  `ExchangeTime` forms as the one clock (an own-line pin already has these); they take
  `&ExchangeTime` instead of a zone: `format_tick_label_in`, `format_crosshair_time_in`,
  `weight_by_time_in`, and `fill_weights_for_points_in`. Read the engine's own clock with
  `ChartEngine::exchange_time()`, or build one with `ExchangeTime::new(zone.offset_schedule()?, 0)?`.
  There is no `ExchangeTime` form of `format_date_pattern`: shift the timestamp with
  `ExchangeTime::local_seconds` and format that, as `format_crosshair_time_in` does. Only an
  upstream pin from `f796529` on had the removed helpers; an own-line pin never did.
- A named zone is resolved once into the explicit schedule, so tick weights, labels, VWAP and pivot
  period keys, sessions, and the countdown follow it exactly like `set_exchange_offsets`. The name
  also localizes the general temporal axes and `time_zone_clock_text`; an explicit schedule does
  not, and `time_zone_id()` then returns `custom` instead of a TradingView id.
- A top-level `timezone` option that is not a string, or names an id outside the parity list, now
  rejects the whole options patch (earlier revisions ignored it silently). A host that forwards a
  TradingView placeholder such as `exchange` must filter it before the patch. Importing a saved V2
  document is the exception: an unresolvable or non-string `timezone` in its options (a raw value an
  earlier build stored) is dropped so the rest of the layout still restores.
- `time_scale_options_json()["time_zone"]` reports the exchange schedule (`"UTC"` or the transition
  array), not the TradingView id an earlier revision printed there; read the named zone through
  `time_zone_id()`.
- A V2 document can carry an additive `timezone` string beside `timeScale.timeZone`, written only
  while a named zone is installed. A consumer that takes `aeris_charts_core` by Git does not read
  this repository's `.cargo/config.toml`, so it compiles the complete tz tables rather than the 98
  parity zones the repository's artifacts keep.

**Drawing text editing** (both lines, from the merge `2e7d19f merge: sync with
AerisTerminal/aeris-charts main`, which made upstream's `7518e7e feat(drawings): engine-owned text
typing session for every host` the one session and kept the own line's layout, hit testing, and
editable tools; see
[Drawing anchors, magnet, and price basis](#drawing-anchors-magnet-and-price-basis)).
Each item says which pins it applies to:

- On main one engine session is the only text-editing state. Open it with
  `begin_drawing_text_edit(id, paint_caret)` (`false` for a host that paints its own caret, as the
  browser does); mirror a host's editable surface with `set_drawing_text_edit(text, caret)`; end it
  with `commit_drawing_text_edit()` or `cancel_drawing_text_edit()`; `editing_drawing()` reads the
  open session. Native hosts use `drawing_text_edit_insert`, `drawing_text_edit_key`,
  `drawing_text_edit_select_all`, and `drawing_text_edit_caret_at`. Live text records no undo step;
  a commit records one. A GPUI host that forwards its key events to `GpuiChartInput::key_down` does
  not need to call these: the adapter routes the keys, with the platform's word and line motion and
  the clipboard shortcuts (see the engine input controller below).
- `ChartEngine::set_editing_drawing` is gone. Upstream pins have it (alongside the session), and so
  do own-line pins before `36c9f09`; an own-line pin from `36c9f09` on does not.
- An own-line pin from `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded
  ticks, tick-built candles, and resampling` up to the merge used a three-call session of its own,
  which the merge removed: `begin_drawing_text_edit(id)`, `set_drawing_edit_text(text)`, and
  `end_drawing_text_edit(commit)`. The new calls are `begin_drawing_text_edit(id, paint_caret)`,
  `set_drawing_text_edit(text, caret)` (the mirrored value now carries the caret), and
  `commit_drawing_text_edit()` or `cancel_drawing_text_edit()` for `commit` true or false. It is not
  a plain rename: the old call neither trimmed the text nor removed a text tool, while a commit now
  trims the text and removes a text tool left empty, and a cancel removes a text tool that began
  empty. An upstream pin never had the three-call form.
- An upstream pin that contains `7518e7e` but not `b75f092 feat(drawings): text-field selection in
  the drawing typing session` had `drawing_text_edit_key(key)`. That commit changed it to
  `drawing_text_edit_key(key, extend_selection)` (Shift extends the selection) and gave
  `DrawingTextEditKey` the variants `DeleteWordBackward`, `DeleteWordForward`, `WordLeft`, and
  `WordRight`, so such a pin adds the argument and, for an exhaustive `match` on the key, the arms.
  An own-line pin never had the one-argument form.
- `begin_drawing_text_edit` accepts every drawing that paints its own text, and refuses (leaving an
  open session alone) a locked, hidden, interval-hidden, or non-text drawing, or one whose anchors
  cannot convert yet. Upstream's session opened only the text tool and trend lines, and closed an
  open session even when it refused; an own-line pin before `d438dab` stopped at those and the
  family drawings. A chart that has not been laid out has no price scale to convert the anchors, so
  a host that begins a session before its first frame sees `false`.
- An upstream pin bounded text at 256 bytes; main bounds it by `MAX_DRAWING_TEXT_BYTES` (65,536
  bytes): an insert that would exceed it is refused whole, and a mirrored value is clamped at a
  character boundary. A run label stays on one line; family text boxes (`comment`, `callout`,
  `note`, `signpost`, `anchored_text`) keep line breaks. Native hosts get click-to-caret placement
  and typing in a box but not Up/Down line navigation yet.
- `drawing_text_hit_at` answers for the label of every tool that paints a text run (lines,
  channels, Fibonacci, shapes), not only the trend line, and arbitrates against higher drawing
  bodies; upstream pins and own-line pins before `d438dab` answered for the trend line only. Which
  click starts typing is one rule for every host: the first click opens it only for the text
  tool's two-step click and a trend label, and placing a tool that requests an editor opens it on
  placement; a double-click on the selected drawing, Enter, or F2 opens the editor of every
  text-bearing drawing. The browser gesture layer and the native input controller both apply it; a
  host that drives the engine API directly applies it itself.
- Upstream's `3527136 Default rectangle borders off and EMA strokes to one pixel` (an own-line pin
  before the merge `2e7d19f` and an upstream pin before `3527136` lack it) makes rectangles default
  to no border (`border_visible: false`). A saved document that omits the key imports with the
  border visible, so earlier documents keep their look.

**Engine input controller** (upstream, from `17a591f feat(input): engine-owned interaction
controller for every native host`, which the own line took with the merge `3eef45e`; the design is
in [Architecture.md](Architecture.md#aeris_charts_engine)). The engine now owns pointer, wheel, and
key routing: a host translates platform events into `PointerInput`, `WheelSample`, and `ChartKey` and
calls `ChartEngine::input_*`. Press arbitration, drag lifecycles, click and double-click, key
bindings, hover, cursor choice, and kinetic motion belong to the engine. Review these call sites:

- Which pins had the removed gesture API. `begin_financial_drag`, `update_financial_drag`,
  `update_financial_crosshair`, `end_financial_drag`, `apply_financial_wheel`,
  `apply_financial_navigation`, `financial_drag`, and the types `FinancialDrag` and
  `FinancialNavigation` existed only on upstream's side, from `f9b052f refactor(engine): own native
  financial gestures` (`apply_financial_navigation` and `FinancialNavigation` from `4aaaf78
  refactor(engine): own financial scale commands`), and on own-line revisions from the merge
  `2e7d19f` until the merge `3eef45e` took `17a591f`, which removed them with no compatibility shim.
  An own-line pin before `2e7d19f` never had them, so it has no host gesture API to delete: it adopts
  the `input_*` calls, and the table below only says what each call now does. Coordinates are
  unchanged (pane space: x from the plot's left edge, y from the chart's top).

  | Removed | Now |
  | --- | --- |
  | `begin_financial_drag(x, y, click_count, radius)` | `input_pointer_down(PointerInput, click_count)`; `click_count` is `u32` (it was `usize`); the 4 px separator radius is the constant `PANE_SEPARATOR_HIT`; a double-click axis reset follows `InteractionOptions::axis_double_click_reset_time` and `axis_double_click_reset_price` |
  | `update_financial_drag(x, y)`, `update_financial_crosshair(x, y, radius)` | `input_pointer_move(PointerInput, primary_pressed)`, which also promotes drawing, series, and trading hover and keeps the crosshair tracking (clamped into the plot) while a press is captured; `input_pointer_leave()` clears the crosshair while no press is open |
  | `end_financial_drag()` | `input_pointer_up(PointerInput)` to finish, `input_cancel()` to abandon |
  | `apply_financial_wheel(x, y, normalized_x, normalized_y)` | `input_wheel(WheelSample)`, which returns whether the chart consumed the wheel. With the default `WheelBehavior::Auto`, `delta_y` goes through `wheel_zoom_scale` as `normalized_y` did and `delta_x` through `WHEEL_SCROLL_PX_PER_DELTA` as `normalized_x` did, so the values passed as `normalized_x` and `normalized_y` go to `delta_x` and `delta_y` unchanged. It is not behaviour-equivalent: see the differences below |
  | `apply_financial_navigation(action, accelerated)` and `FinancialNavigation` | `input_key_down(ChartKey, InputModifiers, repeat, now_ms)` and `input_key_up(ChartKey)`: `PageUp`/`PageDown` are `PreviousPage`/`NextPage`, `ZoomIn`/`ZoomOut` keep their names, and `accelerated` is Control or Shift in `InputModifiers`. `PreviousBar`/`NextBar` map to `ChartKey::ArrowLeft`/`ArrowRight` in direction only: see the differences below |
  | `financial_drag()` and `FinancialDrag` | none: the open gesture is engine-private, and the pointer feedback is `input_cursor()` |

- Where a replacement does not behave like the call it replaces. These were compared in the code,
  not by signature, and none of the removed functions changed between its introduction and
  `17a591f`:
  - Wheel over a price axis. `apply_financial_wheel` zoomed a price scale whenever the pointer was
    over that scale's axis strip, whatever the wheel behaviour. `input_wheel` zooms it only when
    `InteractionOptions::wheel_behavior` is `WheelBehavior::Zoom` or
    `InteractionOptions::price_axis_wheel_zoom` is `true` (default `false`); with the default
    `WheelBehavior::Auto`, the same wheel over an axis strip zooms the time scale. To get the old
    behaviour back, call `set_interaction_options` with `price_axis_wheel_zoom: true`.
    `WheelBehavior::Zoom` also zooms the price scale there, but it turns every wheel into a zoom, so
    a horizontal wheel stops panning.
  - Wheel zoom over the plot. The old call zoomed the time scale around the pointer. A plain wheel
    now follows `right_bar_stays_on_scroll` (default `true`; see the follow-ups below), so the gap
    after the newest bar stays and only Ctrl/Cmd zooms around the pointer. `input_wheel` also
    honours `wheel_zoom` and `wheel_scroll` (both default `true`), and it stops a kinetic coast, a
    held arrow pan, or an animated scroll that is in progress.
  - Drag threshold. The old calls acted from the first `update_financial_drag` (a pane pan, an axis
    scale, or a separator resize). Now pane pans, axis scales, and separator drags begin at the
    shared 5 px threshold, and an unmoved release is a click. A plain press on a price axis
    therefore no longer switches that scale to manual: the old `begin_financial_drag` turned
    autoscale off on the press, and now the first scale step does. A pane drag pans a manual price
    scale only when it is the scale of the series (or the pane's default) under the press; the old
    call also fell back to the pane's first manual right or left scale.
  - Double-click on a price axis. `begin_financial_drag` ran `reset_price_scales()`, every price
    scale in the chart; `input_pointer_down` resets only the pressed scale (`reset_price_scale(pane,
    target)`). A host that wants the chart-wide reset keeps `reset_price_scales()` or `reset_view()`
    for its own command.
  - Release and cancel. `end_financial_drag` only ended the scale and scroll sessions. An unmoved
    `input_pointer_up` also selects or activates what is under the pointer, and `input_cancel()` ends
    the same sessions but also restores an open drawing or trading drag and clears hover, a live
    measure, and the cursor. `input_pointer_move` with `primary_pressed` false while a press is open
    abandons that press.
  - Arrow and zoom keys. The old call jumped 1 bar (10 accelerated) per call; `ArrowLeft` and
    `ArrowRight` start a velocity-owned pan (see the clock item). `ZoomIn` and `ZoomOut` anchored
    at the plot centre; with the new `right_bar_stays_on_scroll` default they keep the gap after
    the newest bar instead. Keys also follow the `InteractionOptions` switches (`pan` or
    `wheel_scroll` for the scrolling keys, `wheel_zoom` for the zoom keys; all default `true`), and
    a gated key stays unconsumed.
- The host supplies the clock. Arrow-key panning is velocity-owned, so deliver `input_key_up`, pass
  the platform's key-repeat flag, call `input_tick(now_ms)` once per prepared frame, and request
  another frame only while `input_animating()` holds. `input_wake_deadline_ms()` is the one
  deferred deadline (the trading-tooltip dwell): schedule a wake for it and repaint.
  `flush_coalesced_input()` forwards the newest captured drawing sample once per prepaint.
- The engine hands host-only work back as `ChartInputEvent`s (`ContextMenu`, `DrawingCreated`,
  `RemoveSeries`); drain them with `take_input_events()` after each input call. Hosts keep event
  translation, pointer capture, applying `input_cursor()`, timers and frame scheduling, menus,
  clipboard, and persistence. Persist drawings on `drawing_revision()` instead of tracking
  gestures. The reference `handleScroll`/`handleScale` switches are `InteractionOptions`
  (`interaction_options()` and `set_interaction_options`).
- The lower-level gesture operations (`drawing_tool_pointer_*`, `measure_pointer_*`,
  `delta_tooltip_mouse_*`, `kinetic_*`, `start_keyboard_scroll`, `keyboard_scroll_tick`,
  `cancel_keyboard_scroll`, the `time_axis_*` and `price_axis_*` scale and scroll steps,
  `drag_pane_separator`) are still public engine operations, but the controller now sequences them.
  A host that keeps driving them beside `input_*` bypasses the controller's press arbitration and
  cursor, so host routing that sequenced them should be deleted.
- Frame preparation changed with it: `prepare_financial_frame_with_measure` rebuilds after any
  layer invalidation or input change and relayouts after a pane-resize drag. A host that cleared its
  frame to force a rebuild, or forced a layout when `update_financial_drag` reported a pane resize,
  can stop doing so.
- GPUI hosts (feature `gpui-backend`) bind through `aeris_charts_render_gpui::input` with one
  adapter call per listener: `GpuiChartInput::mouse_down`, `mouse_move`, `mouse_up` (bound for
  releases outside the chart too), `context_menu`, `scroll_wheel`, `pinch`, `modifiers_changed`,
  `key_down`, and `key_up`. `scroll_wheel` converts GPUI's delta itself (pixels divided by 100,
  lines at `WHEEL_LINE_HEIGHT`, 32 px). Prepaint calls `set_canvas_bounds(bounds)` and
  `prepare_frame(&mut engine)`, `wake_delay(&engine)` schedules the deferred wake,
  `cursor_style(engine.input_cursor())` is the one cursor mapping, and
  `install_text_metrics(&mut engine, window)` runs before a frame is prepared so drawing labels
  measure as they paint. The host's own key tables, cursor priority, and text-edit routing are
  redundant beside the adapter and should be deleted rather than kept.
- Two follow-ups after `17a591f`. `1869773 feat(input): TradingView wheel zoom anchoring,
  engine-owned on every host` (upstream) renamed `GpuiChartInput::set_origin(Point<Pixels>)` to
  `set_canvas_bounds(Bounds<Pixels>)` (only a pin at `17a591f` itself had the old name), added
  `input_pinch` with `GpuiChartInput::pinch`, and made
  `TimeScaleOptions::right_bar_stays_on_scroll` default to `true`: a plain wheel or keyboard zoom
  keeps the gap after the newest bar, and only Ctrl/Cmd wheel and pinch zoom around the pointer.
  Call `ChartEngine::set_right_bar_stays_on_scroll(false)` for the earlier zoom around the pointer
  (wheel) or the plot centre (keys). The merge `3eef45e merge: sync with AerisTerminal/aeris-charts
  main (range tools, input controller)` (own line) added `ChartKey::EditText` (F2), so an upstream
  pin's exhaustive `match` on `ChartKey` needs an arm.

**GPUI dependency** (own line, from `9ae1c58 gpui: build the executor on gpui-pre 0.3.6, the GPUI
gpui-kit pins`; see [Rust distribution](#rust-distribution)). `aeris_charts_render_gpui` stopped
depending on a Zed Git revision:

- Before, it used `gpui` 0.2.2 from `https://github.com/zed-industries/zed` at revision
  `1057c2cf3d5b4aefd04755e1387c7826a4d7fba6` (its manifest sourced GPUI from Zed to track the
  current pre-1.0 API and pinned `gpui` and `gpui_platform` to that one reviewed commit). Now the
  `gpui-backend` feature enables `gpui = { package = "gpui-pre", version = "=0.3.6" }` and the
  parity harness uses `gpui-pre-platform` at `=0.3.6`. The manifest records this as the `gpui` that
  gpui-kit 0.6.6 (`gpui-component`) pins, so a host on gpui-kit 0.6.6 should already resolve it;
  that pin is not checked from this repository.
- The host must resolve to that same package and version: replace its Zed Git dependency with the
  `package = "gpui-pre"`, `version = "=0.3.6"` form (and `gpui-pre-platform` where it uses the
  platform crate). A host left on the Zed revision holds two GPUI copies, and every adapter and
  executor call that takes or returns a GPUI type (`input.mouse_down(&mut engine, &MouseDownEvent)`,
  `install_text_metrics(&mut engine, &Window)`, `cursor_style` returning a `CursorStyle`) fails to
  type-check against the host's copy.
- Keep the `windows-manifest` feature on Windows: GPUI imports `comctl32!TaskDialogIndirect`, which
  resolves only under the comctl32 v6 activation context the feature's manifest embeds, so an
  executable without it fails to load (`STATUS_ENTRYPOINT_NOT_FOUND`) before `main`.
- `9ae1c58` changes only the manifest, the lockfile, and documentation: the executor and adapter
  source compiled unchanged against `gpui-pre` 0.3.6, so Aeris's own API did not change. The
  host's own GPUI code is not checked from this repository: moving to `gpui-pre` 0.3.6 is the
  host's change to make and verify.

**GPUI snapshot 0.3.7** (own line, the commit that moves `gpui-pre` to 0.3.7 and every other
dependency to its latest release; find it with `git log -S'=0.3.7' --
crates/aeris_charts_render_gpui/Cargo.toml`). It follows gpui-kit, which moved from 0.6.6 to 0.7.0
and pins `gpui-pre =0.3.7` together with `gpui-pre-platform`, `gpui-pre-web`, `gpui-pre-macros`
and `gpui-pre-sum-tree` at the same version:

- A host that consumes this executor must be on one `gpui-pre` version, so a host still on
  gpui-kit 0.6.6 (`gpui-pre =0.3.6`) moves to gpui-kit 0.7.0 in the same change as the Aeris pin;
  with one on each side Cargo resolves two incompatible `gpui` copies and every adapter and
  executor call that takes or returns a GPUI type fails to type-check, as with the earlier Zed
  revision above.
- Aeris's own API did not change: the executor and adapter source compiled unchanged against
  0.3.7 and the complete GPUI test suite passed unchanged. The host's own GPUI code is not checked
  from this repository.
- The parity harness's X11 capture moved to `x11rb` 0.14 (a Linux dev-dependency of the examples
  only). GPUI's Linux platform still depends on `x11rb` 0.13, so the example build holds both;
  nothing a host links is affected.

**Measuring tools** (both lines, from the merge `3eef45e merge: sync with
AerisTerminal/aeris-charts main (range tools, input controller)`). Upstream's `5a2e6e8 feat(drawings): add price/date range
measuring tools and Shift-click measure` and the own line's `36c9f09 feat(charts): B8 drawing
catalog, multi-calendar overlays, bounded ticks, tick-built candles, and resampling` had built the
three range tools independently, and the merge kept the own line's implementation. Which spelling a
pin has depends on its side:

- An upstream pin from `5a2e6e8` on has `DrawingKind::DatePriceRange`, the kind name
  `date_price_range`, and the wire ids 13 (`PriceRange`), 14 (`DateRange`), and 15
  (`DatePriceRange`). An own-line pin from `36c9f09` on (`36c9f09` itself, for example) already has
  `DrawingKind::DateAndPriceRange`, the name `date_and_price_range`, and the ids 130, 131, and 132,
  which is what main keeps. For an own-line pin there is no rename and no id remap; only the grid
  snap below applies. A pin on either side before those commits has no range tools.
- For an upstream pin, `DrawingKind::DatePriceRange` is now `DrawingKind::DateAndPriceRange`
  (`PriceRange` and `DateRange` keep their names). The numeric wire ids of `DrawingKind::to_u8` and
  `from_u8` moved: `PriceRange` is 130, `DateRange` 131, and `DateAndPriceRange` 132, where they
  were 13, 14, and 15. Ids 13 to 15 are now unassigned and ids 0 to 12 are unchanged, so a host that
  stored numeric ids from an upstream pin remaps them.
- The kind name `date_price_range` is still read (a serde alias and `DrawingKind::from_name`) and
  never written, so documents saved by an upstream pin still load; saved documents, templates, and
  clipboard payloads write `date_and_price_range`.
- Grid snap. Creation, anchor drags, body drags, and keyboard nudges of the three range tools and of
  the long and short position tools snap to whole bars and to the instrument tick or price-band
  ladder. An own-line pin before the merge has no such snap on these tools, so it takes this change
  for all five (the position tools' price snapping came earlier, with the merge `2e7d19f`). An
  upstream pin from `5a2e6e8` on already snapped them to bars and the price tick; it gains the
  price-band ladder (`SeriesPriceFormat::tick_ladder`).
- The Shift-click quick measure came with `5a2e6e8` (an own-line pin before the merge never had
  it). Main drives it from the input controller (a Shift press on the pane), so `measure_pointer_*`
  need not be called for it.

**Crosshair shade, baseline mode, live-bar easing, and timeline marks** (own line, the commit that
adds `live_bar_easing_ms`; find it with `git log -S'live_bar_easing_ms' --
crates/aeris_charts_engine/src/lib.rs`; see [Crosshair shade](#crosshair-shade), [Baseline reference
line](#baseline-reference-line), [Live-bar easing](#live-bar-easing), and [Timeline
marks](#timeline-marks)). Every addition is off or empty by default, so a host that adopts nothing
renders as before; the review points are the frame-request predicate and the added enum variants and
struct fields:

- Animation clock. `ChartEngine::set_animation_time(ms)` is new on the engine (the browser shell used
  to stamp a clock of its own): it advances every live-bar glide and writes the `pub animation_time`
  field only when a last-price pulse is drawn or a glide advanced, so a stamp-only tick changes no
  frame key. `advance_live_bar_easing(now_ms) -> bool` advances only the glides;
  `live_bar_easing_active()`, `animation_active()` (pulse or glide, what the browser `wants_animation`
  returns), and `animation_frame_requested()` (`input_animating() || live_bar_easing_active()`, the
  Rust-host frame-request predicate) are new. A host that requests another frame only while
  `input_animating()` holds, as the input-controller group above described, moves to
  `animation_frame_requested()` or `GpuiChartInput::animating(&engine)`; otherwise a glide shows its
  first frame and stalls, because no further frame is requested. `GpuiChartInput::prepare_frame` now
  also calls `advance_live_bar_easing` on the adapter clock and returns `true` when a glide moved;
  the adapter still never runs the pulse clock.
- Baseline. `ChartEngine::series_baseline_price(id) -> Option<f64>` and the public enum
  `BaselineMode` (`VisibleMidpoint`, `CloseBeforeVisibleRange`, with `as_str` and `parse`) are new.
  `SeriesEntry` gains the `pub` fields `baseline_mode`, `baseline_line_visible`,
  `baseline_line_color: Option<String>`, `baseline_line_width`, `baseline_line_style`, and
  `live_bar_easing_ms`; a struct literal that lists the fields adds them, and the defaults keep
  today's rendering. A host that writes `live_bar_easing_ms` directly gets the JSON path's semantics:
  a non-finite or non-positive value is off and larger values clamp to `MAX_LIVE_BAR_EASING_MS`
  (1000).
- Crosshair. `aeris_charts_core::options::CrosshairOptions` gains `shade_right:
  CrosshairShadeOptions { visible, color }` (wire key `shadeRight`, `#[serde(default)]`, so V2
  documents saved before it still read); a struct literal that lists the fields adds it.
- Timeline marks. New `ChartEngine` methods: `set_timeline_marks(snapshot) -> Result<(),
  ChartError>`, `timeline_marks()`, `set_timeline_marks_visible(bool) -> bool`,
  `timeline_marks_visible()`, `set_timeline_group_hidden(group, hidden) -> Result<bool, ChartError>`,
  `hidden_timeline_groups()`, `timeline_mark_hit_at(x, y)`, `timeline_mark_hit_at_with_profile(x, y,
  HitProfile)`, `timeline_mark_hit_for_id(id)`, `timeline_lane_pane()`, and
  `timeline_mark_activation(seq)`; new public types `TimelineMark`, `TimelineMarkGlyph`,
  `TimelineGlyphShape`, `TimelineMarkGroup`, `TimelineMarksSnapshot`, and `TimelineMarkHit`, and the
  caps `MAX_TIMELINE_MARKS` (4,096) and `MAX_TIMELINE_GROUPS` (64). New variants, each a compile-time
  break for an exhaustive `match`: `ChartInputEvent::TimelineMarkActivated(u32)` (drain it with the
  other events and read the hit through `timeline_mark_activation(seq)`), `ChartHover::TimelineMark`,
  and `InputTarget::TimelineMark`. `EngineMemoryUsage` gains `timeline_marks_capacity_bytes`.
- Persistence. V1, V2, and V3 documents gain the optional `hidden_mark_groups` list (omitted when
  empty; no schema bump). The document structs ignore unknown fields, so a document written by this
  revision with hidden groups still loads on an older pin, which drops the list.
- Core. `PlotListView::with_row_override(source_row, [open, high, low, close])` and
  `overridden_values(row)` are new, and `value_at` and `is_whitespace_row` honor the override; no
  existing signature changed. The browser package's additions are additive `.d.ts` members (the
  `crosshair.shadeRight` and baseline/easing series options, `series_api.baseline_price`,
  `chart_api.timeline_marks`, and the `subscribe_timeline_mark_click` pair) and the npm version is
  unchanged in that commit.

**Other source-level changes.** Each item names the commit that carries it. None of the public
enums involved is `#[non_exhaustive]`, so every added variant is a compile-time break for an
exhaustive `match`, and every added field is one for a struct literal that lists the fields.

- `a565efc fix(kline): close K-line engine pitfalls across time, indicators, drawings, streaming,
  viewport, price axis, and intraday charts` (own line):
  - `PriceScaleCore::build_tick_marks` and `price_tick_span_calculator::composite_tick_span` take
    `min_move: f64`, the price grid every tick lies on, where they took `base: i64`, the formatter
    base. A base of 100 is a `min_move` of `0.01`, which is also the engine's grid on percentage
    and indexed scales.
  - `SessionHighlightingOptions::start_hour_utc` and `end_hour_utc` (`Option<u8>`) are now
    `start_hour` and `end_hour` (`Option<f64>`): fractional, exchange-local hours (`9.5` is 09:30;
    `start_hour > end_hour` wraps midnight). `Some(9)` becomes `Some(9.0)` and means the same
    while the exchange time is UTC.
  - `IndicatorKind::Ema`, `Dema`, `Tema`, and `Rsi` gain `seed: IndicatorSeed`, `Macd` gains
    `seed` and `histogram_multiplier: f64`, and `Bollinger` gains `estimator:
    DeviationEstimator`. `IndicatorSeed::Sma`, `1.0`, and `DeviationEstimator::Population` are the
    earlier behaviour (and the serde defaults, so saved documents still read); a pattern that lists
    the fields needs `..`.
  - `DrawingClipboardItem::points` is `Vec<DrawingAnchor>` instead of `Vec<DrawingPoint>`
    (`DrawingAnchor { logical: Option<f64>, price, time: Option<f64> }`, and
    `DrawingAnchor::from(point)` converts).
  - New variants: `IndicatorKind::Kdj` and `IndicatorParameterType::Choice`. New fields:
    `TimeScaleOptions::lock_visible_logical_range`, `PriceScaleCoreOptions::{
    ensure_edge_tick_marks_visible, base_value, autoscale_center, stable_auto_scale}`,
    `PriceMark::edge`, `IndicatorInput::amount`, `SeriesPriceFormat::tick_ladder`,
    `SeriesEntry::histogram_updown_rule`, `IndicatorBindingInfo::amount_source`,
    `IndicatorParameterDescriptor::choices`, and `price_basis` on `DrawingClipboardPayload` and
    `DrawingSyncPayload`.
- `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built
  candles, and resampling` (own line): `ChartEngine::copy_drawings_json` returns
  `Result<String, ChartError>` instead of `Option<String>` (`ErrorCode::InvalidData` when no known
  drawing could be copied; `ErrorCode::ResourceLimit` past `MAX_DRAWING_CLIPBOARD_POINTS` anchors,
  checked first, and past `MAX_DRAWING_CLIPBOARD_BYTES` bytes). An upstream pin has the
  `Option<String>` form too. New variants:
  `DrawingKind` (the B8 catalog; later own-line commits add `HorizontalSegment`, `VerticalRay`,
  `VerticalSegment`, `PriceChannel`, `PriceLine`, `SimpleTag`, and `SimpleAnnotation`), ten
  `DrawingKindOptions` variants, `DrawingDragPart::Handle(usize)`,
  `FootprintError::InvalidSessions`, `ResampleError::{InvalidSessions, TimeAxisRequired}`,
  `TradeStudyKind::Volume`, and
  `aeris_charts_core::model::plot_list::PlotValues::AsOf`. New fields: `Drawing::tool_options`,
  `SeriesEntry::break_on_trading_day`, and `TradeStreamStats::{dependent_rows_computed,
  bar_rows_projected, bubble_trades_scanned, bubble_markers_sized}`.
- Later own-line additions: `ExchangeTimeError::BarTimeLabelWindows` (`78d7d59 feat(time):
  close-time display labels for open-stamped bars`), `IndicatorKind::KLineChart` (`c2d837e engine:
  bind KLineChart indicators as chart studies`), and `angle` and `multiline` on
  `DrawingTextEditLayout`, a type the own line has had since `36c9f09` (`d438dab feat(drawings):
  edit the text of every text-bearing tool and open the editor on placement`).
- `960e011 feat(drawings): add complete position statistics and adaptive borders` (upstream):
  `DrawingKindOptions::Position` gains `account_size: f64` and `risk_percent: f64`, and `Drawing`
  gains `position_account_size` and `position_risk_percent`.
- `5c62071 feat(engine): own native workspace identity` (upstream):
  `prepare_financial_frame_with_measure` takes one `FinancialFrameRequest { width, height, dpr,
  force_layout, fit_content, frame, axis_primitives }` followed by `measure` and
  `countdown_measure`, where it took nine positional arguments (`width`, `height`, `dpr`,
  `force_layout`, `fit_content`, `measure`, `countdown_measure`, `frame`, `axis_primitives`); it
  still returns `FinancialFramePreparation`.
- Upstream commits between the parting point `ed2910d` and `1869773` also changed these existing
  public types, so a pin that does not contain them diffs the declarations it matches or
  constructs: `TradingHitKind` gains `TakeProfitButton` and `StopLossButton` (`24e8e2d fix(trading):
  dedicated TP/SL buttons with pixel-exact, optically centered controls`), and `TradingStyle` and
  `TradingStyleOptions` gain `execution_buy` and `execution_sell`, while `ExecutionMarkerShape`'s
  default is `Arrow` where it was `Circle` (`6f56736 fix(trading): bar-anchored execution arrows
  with stacked multi-fill marks`). The range kinds and the `Position` fields are covered above. A
  pin older than `ed2910d` also diffs the types changed before it, which this section does not
  list.

## Release policy

Tag publication depends on required Rust, package, and portable Chromium/Firefox/WebKit jobs. It also
checks persistence fixtures, public declarations, Node/SSR import, package contents, and configured
performance budgets. `continue-on-error` is forbidden for portable correctness.

Hardware- and machine-sensitive screenshot hashes, GPU timing, heap sampling, and wall-clock evidence
are calibration diagnostics. They remain non-authoritative and may not be approved merely to make one
runner green. Shared draw-stream parity, clipping/order/frame-contract tests, replay determinism, and
portable browser behavior are the authoritative gates. Scenarios without a configured benchmark
budget remain explicitly report-only.
