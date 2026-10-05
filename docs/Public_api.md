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
  and `"date_price_range"` (the spelling `"date_and_price_range"` of earlier builds of this line is
  still read on import, templates, clipboard, and sync, and never written); each stores an editable start and end anchor
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

`configure_resampled_series(source, target, options, volume_source?, volume_target?)` derives a
candlestick or bar `target` (and an optional volume histogram) from a source series in the engine.
`resample_boundaries()` derives the periods from the session windows, the exchange time zone, and
the host's trading dates:

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
chart.configure_resampled_series(minute, hour, { intervalSeconds: 3600, boundaries },
  minute_volume, hour_volume);
```

The earlier form `configure_resampled_series(target, { source, volume_source?, volume_target?,
interval_seconds, boundaries })` is a deprecated overload with the same engine binding; it still
reads boundary rows written as `{ start_time, end_time, session_id }`, typed as the deprecated
`legacy_resample_boundary`.

**Periods.** `span: "window"` (the default) returns one boundary per session window, so 5-, 15-,
30- and 60-minute bars restart at every window open (A-share 60-minute bars at 09:30, 10:30, 13:00,
14:00; a window whose length is not a multiple of the interval ends with a shorter bar).
`span: "day"` returns one boundary per trading date from its first open to its last close; with
`intervalSeconds: 86400` that is one daily bar per date, stamped at the session open, so US daily
bars built from extended-hours minutes (04:00–20:00 Eastern) stay one bar per day across DST even
though winter sessions run past UTC midnight. Every boundary carries the requested date as
`sessionId` (`YYYYMMDD`): the trading date for a market whose sessions start on it, the evening
date for a Sunday-open market (below). Dates are strictly ascending and use `session_slot_times`
placement (night sessions with a negative `session_start` included); at most 20 000 boundaries and
32 resampled series per chart. Boundaries are `resample_boundary` rows,
`{ startTime, endTime, sessionId }` (UTC seconds, start inclusive, end exclusive), and hosts may
also pass their own (for weeks or months, for example).

`resample_boundaries` has its own `session_start`, default `0` and independent of the chart's, so
China futures pass `-10800` explicitly. For a Sunday-open market (CME Globex) pass `session_start:
0`, the evening dates, and `windows: [["17:00", "16:00"]]`; the `sessionId` of each boundary is then
the requested evening date (`20240107` for the session that opens Sunday 2024-01-07 and is Monday's
trading day), or build the `{ startTime, endTime, sessionId }` periods yourself. Never pass
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
closes at its last traded row. The derived volume sums every finite volume row inside the bucket's
span, from its start to its end (the earlier of the start plus the interval and the boundary's end,
exclusive), including volume rows after the last priced row of a forming bucket; a bucket with no
traded price has no volume. Resampling needs a time axis: it is rejected on a chart whose axis is
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
with its target series, like indicator outputs. `chart.resampled_bars(target)` (a handle or a
series id) returns the derived bars as `resampled_bar` rows: `timestamp`, `sessionId`, `open`,
`high`, `low`, `close`, and `volume` (each `null` for a whitespace bucket), and `sourceRows`, the
aggregated source-row count. Rust hosts call
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
it: the text tool, a trend line's label, the text annotations (`note`, `comment`, `callout`,
`price_note`, `anchored_text`), the text of every other catalog tool (one line, rotated along the
stroke when the label follows a segment), and the `simple_annotation` box (several lines). Level,
vertex, and wave labels, ratios, and stats are engine-formatted text and stay options-only. The
measuring tools and `simple_tag` (whose `text` is its price-axis tag) accept `text` but never paint
or edit it on the chart. A double-click on a selected
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
the rest of the patch, and typing stops at the bound); the text tool, trend labels, the text
annotations, and every other run label stay on one line (line breaks become one space), while the
`simple_annotation` box takes several lines (Shift+Enter adds one, paste inserts plain text). The
editor is a labeled text box that announces opening and closing through the accessibility live
region and returns focus to where it was opened from. Placing the text tool, a text annotation, or
`simple_annotation` opens the editor at once, and a single click on an already selected text tool
or text annotation reopens it when it is empty or the click lands on its text. Committing or
Escape keeps the drawing even emptied, except the text tool and the text annotations, which remove
themselves when left empty.

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
   Position levels use the entry anchor's segment; only anchor prices are rescaled. This is a
   data-basis change, not an edit: it
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

The B8 drawing catalog extends `drawing_kind` with AerisTerminal upstream's tools plus seven tools of
this line's own. Every tool uses the same placement, selection, handles, drags, magnet, keyboard
editing, anchor time identity, history, persistence, clipboard, sync, and schema APIs as every other
drawing, and each drag, and each keyboard edit session, is one undo step. Kind defaults (for example
a ray's `extend_right`) are the schema defaults and are omitted from persistence.

**Names.** `drawing_kind` holds the canonical names below. The legacy spellings of earlier builds of
this line, `drawing_kind_alias`, stay in the union so no export disappears, but they are input only:
`add_drawing`, `set_drawing_tool`, templates, clipboard and sync payloads, and restored documents
accept them, normalize them through `DRAWING_KIND_ALIASES`, and every output (`drawings()`, the
`kind` of a handle, exported documents, payloads) carries the canonical name. `DRAWING_KIND_TO_U8`
holds canonical rows only (`Record<Exclude<drawing_kind, drawing_kind_alias>, number>`), so a wire
id always maps back to a canonical name; normalize an alias before a wire-id lookup.

| Legacy spelling (`drawing_kind_alias`) | Canonical kind |
| --- | --- |
| `date_and_price_range` | `date_price_range` |
| `fib_retracement`, `trend_based_fib_extension`, `fib_channel`, `fib_time_zone`, `trend_based_fib_time` | `fibonacci_retracement`, `fibonacci_extension`, `fibonacci_channel`, `fibonacci_time_zones`, `fibonacci_trend_time` |
| `fib_speed_resistance_fan`, `fib_speed_resistance_arcs`, `fib_circles`, `fib_spiral`, `fib_wedge` | `fibonacci_speed_fan`, `fibonacci_speed_arcs`, `fibonacci_circles`, `fibonacci_spiral`, `fibonacci_wedge` |
| `xabcd_pattern`, `cypher_pattern`, `abcd_pattern`, `head_and_shoulders`, `triangle_pattern`, `three_drives_pattern` | `pattern_xabcd`, `pattern_cypher`, `pattern_abcd`, `pattern_head_shoulders`, `pattern_triangle`, `pattern_three_drives` |
| `elliott_impulse_wave`, `elliott_correction_wave`, `elliott_triangle_wave`, `elliott_double_combo`, `elliott_triple_combo` | `elliott_impulse`, `elliott_correction`, `elliott_triangle`, `elliott_double_combination`, `elliott_triple_combination` |
| `arrow_mark_up`, `arrow_mark_down`, `arrow_mark_left`, `arrow_mark_right` | `arrow_marker_up`, `arrow_marker_down`, `arrow_marker_left`, `arrow_marker_right` |
| `icon` | `icon_stamp` (a restored document sets `icon_name` to its built-in glyph name, `"star"` by default) |
| `flat_top_bottom` | `flat_top_channel` (a restored document picks `flat_top_channel` or `flat_bottom_channel` from its anchors) |

**Wire ids.** Ids 0 to 84 follow upstream's table (`price_range` 13, `date_range` 14,
`date_price_range` 15, `ray` 16 through `bars_pattern` 84); this line's own tools take 240 to 246
(`horizontal_segment`, `vertical_ray`, `vertical_segment`, `price_line`, `price_channel`,
`simple_tag`, `simple_annotation`). Aliases have no id. Ids are an in-process detail of the
JS/WASM boundary: documents and payloads carry names.

**Options.** Upstream's tools keep their options as flat drawing options, listed in the tool's
`drawing_property_schema`, validated by every patch, persisted, and carried by templates, clipboard,
and sync payloads:

| Option | Tools | Values and default |
| --- | --- | --- |
| `levels` | Fibonacci, pitchforks, pitchfan, Gann box, squares, fan | the common level list (`value`, `color`, `visible`, `style`, `fill_between`, `fill_color`, `label_visible`), at most 64 |
| `level_reverse` | the level tools | `false`; mirrors normalized levels, sends time-zone levels to the other side of their start, and reciprocates positive Gann fan ratios |
| `level_show_prices`, `level_show_values`, `level_show_percents` | the level tools | prices on for retracement, extension, and channel; values on for time zones and trend time; percents on for every level tool but those two |
| `level_label_align` | the level tools | `"left"`, `"center"`, `"right"`; `"left"` for time zones and trend time, `"center"` for arcs, circles, spiral, wedge, pitchforks, pitchfan, Gann box, and squares, `"right"` otherwise |
| `level_log_scale` | retracement, extension, channel | `false`; interpolate positive prices geometrically |
| `gann_fans`, `gann_arcs` | `gann_square`, `gann_square_fixed` | level lists; fans 1/8, 1/4, 1/2, 1, 2, 4, 8, arcs 0.25, 0.5, 0.75, 1 |
| `wave_degree` | Elliott waves | `"subminuette"`, `"minuette"`, `"minute"`, `"minor"` (default), `"intermediate"`, `"primary"`, `"cycle"`, `"supercycle"`, `"grand_supercycle"`, `"submillennium"`, `"millennium"`, `"supermillennium"` |
| `screen_x`, `screen_y` | `anchored_text` | pane fractions 0 to 1, default 0.5 |
| `icon_name`, `icon_size` | `icon_stamp` | a registered image name (empty by default); 8 to 96 CSS px, default 24 |
| `bars_pattern_mode`, `bars_pattern_mirror_x`, `bars_pattern_mirror_y` | `bars_pattern` | `"bars"` (default), `"oc_bars"`, `"line_open"`, `"line_high"`, `"line_low"`, `"line_close"` (`"hl_bars"` is read as `"bars"`); mirrors `false` |
| `regression_source_id`, `regression_deviations` | `regression_trend` | a series id or `null` (the default source below); 0 to 10, default 2 |

This line's own tools and the measuring tools keep one typed block per family under
`options.tool_options` (`tool_options.line`, `tool_options.channel`,
`tool_options.projection_annotation`); a patch deep-merges it (absent keys keep their values, `null`
resets a block, an invalid block rejects the whole patch with `invalid_options`), and schema
descriptors name those options with dotted paths such as `tool_options.line.stats_position`. The
other blocks of earlier builds (`tool_options.fibonacci`, `gann`, `pattern`, `shape`, and the
regression and bars-pattern keys) are still accepted and stored. A key that has a flat counterpart
moves onto it, by key presence, wherever options enter (patches, templates, paste, and restored
documents), and an explicit flat option in the same patch wins: Fibonacci `reverse`, `log_scale`,
`show_prices`, `show_levels`, `levels_as_percent`, and `label_h_align` become `level_reverse`,
`level_log_scale`, `level_show_prices`, `level_show_values`, `level_show_percents`, and
`level_label_align` (`reverse` keeps its meaning: it maps as is on the extension, channel, and time
zones and inverted on the retracement and the speed fan, whose level 0 earlier builds put on the
second anchor; the spiral's counterclockwise `reverse` and a `reverse` on a tool that never read it
are kept but change nothing); Gann `reverse`, `angles`, and `arcs` become `level_reverse`, `gann_fans`, and
`gann_arcs`; pattern `degree` becomes `wave_degree`; bars-pattern `bars_mode`, `mirrored`,
`flipped`, and `bars` become `bars_pattern_mode`, `bars_pattern_mirror_x`, `bars_pattern_mirror_y`,
and the snapshot; icon `icon` and `icon_size` become `icon_name` and `icon_size` (clamped to 96);
and the regression deviations become `regression_deviations`. Keys without a counterpart (for
example `tool_options.channel.middle_line` on a parallel channel or `tool_options.fibonacci.grid`)
are kept and persisted but change nothing on an upstream tool, and its schema does not list them.

`drawing_kind_options()` returns `{ kind: "levels", levels, reverse, log_scale, show_prices,
show_values, show_percents, label_align }` for the Fibonacci, pitchfork, pitchfan, Gann box, and
Gann fan tools, `{ kind: "gann_square", levels, fans, arcs, reverse, show_prices, show_values,
show_percents, label_align }` for the squares, `{ kind: "regression_trend", source_id, deviations }`,
`{ kind: "elliott", wave_degree }`, `{ kind: "anchored_text", screen_x, screen_y, box_color,
box_border_color, box_border_width }`, `{ kind: "icon_stamp", icon_name, icon_size }`,
`{ kind: "bars_pattern", mirror_x, mirror_y, mode, bar_count }`, `{ kind: "text", ... }` for the
note, comment, callout, and price note, `{ kind: "line", stats_position }`, `{ kind: "channel",
middle_line, middle_color }`, and `{ kind: "projection_annotation", ... }` for this line's own tools
and the measuring tools, and `{ kind: "generic" }` for every other tool.

### Lines

- `ray`, `extended_line`, `info_line`, `trend_angle`, and `arrow_line` place two anchors. A ray
  keeps the direction from the first anchor and an extended line runs both ways; the engine projects
  them to the pane edge. `arrow_line` defaults `stroke_end` to `"arrow"`. `info_line` shows its
  visible `labels` (default price change, percent change, bar count, and angle) and `trend_angle`
  its angle. Label values are engine-formatted: `date_time_range` prints the anchors' times through
  the bar label and `duration` the elapsed time (on an axis without time, the bar span). The `text`
  label follows the segment like a trend line's and edits in place the same way; only a trend line
  prompts `+ Add text` on hover.
- `cross_line` places one anchor and paints full horizontal and vertical arms through it, with the
  horizontal line's price tag on the axis.
- `horizontal_segment` keeps both anchors on one price, and `vertical_ray` and `vertical_segment`
  keep both on one bar. Placing, dragging, or supplying an anchor moves the shared coordinate on the
  other, taken from the anchor placed or dragged last, so a supplied or imported pair that disagrees
  is repaired the same way. `extend_left` and `extend_right` extend them beyond the first and second
  anchor; the vertical ray defaults to `extend_right`, which runs it from the first anchor through
  the second to the pane edge. Visible `labels` render as one stats box (price, price change,
  percent change, and ticks; bar count, time range, and duration; screen angle and CSS-px
  distance), placed by `tool_options.line.stats_position` (`"start"`, `"middle"`, `"end"`; default
  `"end"`).
- `price_line` places one anchor and paints a crisp line from it to the right pane edge, with the
  anchor's price printed above the line's start and tagged on the price axis (KLineChart's price
  line). Its body is the ray. A `text` of its own is the generic line label and does not replace
  the price.

### Channels

- `parallel_channel`, `flat_top_channel`, and `flat_bottom_channel` place three anchors and
  `disjoint_channel` four; the engine resolves their boundaries once for hit testing, fill, and
  stroke, and their anchors stay the editable, persisted geometry. Their fill is on by default, and
  `extend_left`/`extend_right` run both lines and the fill to the pane edges.
- `regression_trend` places two anchors that choose a bar window: the bars whose positions lie
  between them. The engine fits the finite closes of its source over that window and paints the fit
  with bands `regression_deviations` population residual deviations away. A window with fewer than
  two closes (the future area, data not loaded yet, a replay clock before it) has no fit and paints
  the dashed segment between the anchors, which stays selectable. The source is `regression_source_id` while that series is live on the
  drawing's pane and price scale (an id that is not live there leaves the drawing without a fit),
  otherwise the first ordinary series added to the drawing's pane and price scale that is still live
  (indicator outputs and custom series never qualify, footprint and feature series do through
  their OHLC projection; reordering or hiding a series does not change the source). The fit reads
  only the rows the replay clock shows, reads each bar of an as-of (`time_alignment: "as_of"`)
  source once, and follows streaming updates of the source; replacing the latest bar or appending
  bars costs the changed rows, not the window.
- `price_channel` is KLineChart's price channel: the base line through the first two anchors is the
  centre, the second line passes through the third anchor parallel to it, and the third line mirrors
  the second on the other side of the base. It defaults to `extend_left` and `extend_right` with no
  fill (`fill_enabled: true` shades the whole band), the third anchor's handle sits on the second
  line's midpoint, and placement previews the base line after the first click.

### Fibonacci

| Tool | Anchors | Default levels |
| --- | --- | --- |
| `fibonacci_retracement` | 2 | 0, 0.236, 0.382, 0.5, 0.618, 0.786, 1 (level 0 on the first anchor, 1 on the second) |
| `fibonacci_extension` | 3 | 0, 0.618, 1, 1.618, 2, 2.618: the first leg's move projected from the third anchor |
| `fibonacci_channel` | 3 | as the retracement: lines parallel to the first leg, offset toward the third anchor |
| `fibonacci_time_zones` | 2 | 0, 1, 2, 3, 5, 8, 13, 21, 34 times the anchors' time distance |
| `fibonacci_trend_time` | 3 | as time zones, projected from the third anchor |
| `fibonacci_speed_fan` | 2 | as the retracement: rays from the first anchor |
| `fibonacci_speed_arcs`, `fibonacci_circles` | 2 | as the retracement: concentric levels around the second anchor |
| `fibonacci_spiral` | 2 | as the retracement: bounded logarithmic turns from the first anchor |
| `fibonacci_wedge` | 3 | as the retracement: concentric arcs between the two side rays |

Each level controls visibility, color, stroke style, the fill to the previous level, and its label;
`fill_enabled` is on by default. Price and time levels beyond the anchors keep a drawing visible and
hittable while its anchors are scrolled away.

### Pitchforks and Gann

- `andrews_pitchfork`, `schiff_pitchfork`, `modified_schiff_pitchfork`, and `inside_pitchfork`
  place three anchors and derive distinct median origins and parallel tines; `pitchfan` draws the
  same levels as rays through the outer anchors. Their `levels` place the tines along the handle
  between the second and third anchors (defaults 0, 0.5, and 1 for the pitchforks, 0, 0.25, 0.5,
  0.75, and 1 for the pitchfan).
- `gann_box` places two anchors and resolves a price and time grid with styled `levels` (default
  0 to 1 in eighths). `gann_square` adds the `gann_fans` angle rays and `gann_arcs` quarter arcs to
  that grid; `gann_square_fixed` resolves a square box on screen from its two anchors. The three
  level families have separate schema, patch, persistence, and fill-between controls.
- `gann_fan` places two anchors and projects nine proportional angle rays (`levels` 1/8, 1/4, 1/3,
  1/2, 1, 2, 3, 4, 8) from its pivot to the pane edge.

### Projection and annotations

- `projection` (apex, target): the target's time and price set the horizon and the projected
  height together, filled as a triangle.
- `forecast` (entry, target): the label at the target shows the move in percent and `target
  reached` once a bar after the entry bar (the entry bar itself never counts) reaches the target
  price (a high for a rising target, a low for a falling one) by the target bar, `expired` once a
  traded bar after the target bar exists without that (whitespace rows such as future session slots
  are not bars), and `pending` otherwise; on a time axis the target bar's time, through the bar
  label, sits one line above it. The source is the regression trend's
  default source, read the same way (replay clock, as-of bars), and the status follows its
  streaming updates; it is derived and never persisted.
- `bars_pattern` (source start, source end, target): placing it copies at most 512 finite OHLC bars
  between the first two anchors into a snapshot, each at its bar offset so gaps survive. The third
  anchor moves the frozen copy without reading the source again; moving a source anchor recaptures
  the copy when the edit commits. `bars_pattern_mode` draws bars, open-close sticks (`"oc_bars"`),
  or a line through the chosen price, and the two mirrors flip it. Persistence, clipboard, and sync
  carry the snapshot, so a destination chart needs no source data.
- `price_range`, `date_range`, and `date_price_range` (two anchors): a fill between the anchors
  (`fill_enabled` defaults on; `fill_color` or the drawing color at 20%), the edge lines of the
  measured axis, arrowed measures through the middle toward the second anchor (`stroke_end`
  defaults to `"arrow"`), and a stats box beyond the measured end (below a date range). Default
  `labels`: price change, percent change, and ticks; bar count and duration; or all five. Anchors
  snap to whole bars and price ticks (also while dragging, nudging with the keyboard, or moving the
  body); ticks count on the instrument tick or price-band ladder, falling back to the scale's
  `min_move`. The Shift-click quick measure draws a transient date-and-price range.
- `anchored_text` (one anchor): text at a fixed pane position, `screen_x`/`screen_y`, taken from the
  placement click (or from the anchor when the drawing is added or its anchors are set), so time and
  price scale changes do not move it; drags, undo, and coordinate patches edit that position.
- `note` and `comment` (one anchor), `callout` (tip, box; a leader to the tip, `stroke_start`
  defaults to `"arrow"`), and `price_note` (one priced point with a pane-wide guide) are text
  annotations: they edit in the inline editor like the text tool and open it on placement. The
  callout's tip and box each have a handle; the others move as one body.
- `price_label` (one anchor): a badge at the pane edge with the engine-formatted price unless
  `text` overrides it.
- `arrow_marker_up`, `arrow_marker_down`, `arrow_marker_left`, `arrow_marker_right`, and `flag_mark`
  (one anchor) and `signpost` (two anchors: foot and plate) are filled markers with optional stems.
- `icon_stamp` (one anchor): the image registered under `icon_name`, `icon_size` CSS px across. A
  host registers RGBA8 images with `chart.register_drawing_icon(name, width, height, pixels)` (at
  most 32 images of up to 96 by 96 pixels, names up to 64 bytes; anything else throws
  `invalid_options`, and registering a name again replaces its pixels) and removes one with
  `chart.remove_drawing_icon(name)`; Rust hosts call `ChartEngine::set_drawing_icon(name, width,
  height, pixels)` and `remove_drawing_icon(name)`, which return `bool`. Persistence keeps only the
  name, so register the images again after a restore. A name without a registered image paints the built-in vector glyph when it is
  `"star"`, `"heart"`, `"check"`, `"cross"`, `"circle"`, `"square"`, `"diamond"`, `"triangle_up"`,
  or `"triangle_down"`, and a colored placeholder otherwise.
- `simple_tag` (one anchor): KLineChart's simple tag: a dashed line across the whole pane at the
  anchor's price, tagged on the price axis. The tag shows the drawing's `text` when it has any and
  the price otherwise; the text is not painted on the chart, so it has no inline editor.
- `simple_annotation` (one anchor): KLineChart's simple annotation: a dashed stem rising from the
  anchor to a small head, with the `text` in a box above the head (it starts empty, may span
  several lines, and edits in place). Placing it opens the editor.

### Patterns, Elliott waves, and cycles

| Tool | Anchors (vertex labels) |
| --- | --- |
| `pattern_xabcd`, `pattern_cypher` | X, A, B, C, D |
| `pattern_abcd` | A, B, C, D |
| `pattern_head_shoulders` | N, LS, N, H, N, RS, N |
| `pattern_triangle` | A, B, C, D, E |
| `pattern_three_drives` | 0, 1, A, 2, B, 3 |
| `elliott_impulse` | 0, 1, 2, 3, 4, 5 |
| `elliott_correction` | 0, A, B, C |
| `elliott_triangle` | 0, A, B, C, D, E |
| `elliott_double_combination` | 0, W, X, Y |
| `elliott_triple_combination` | 0, W, X, Y, X, Z |
| `cyclic_lines`, `time_cycles`, `sine_line` | 2 |

Patterns and Elliott waves are ordered, editable anchor paths with engine-owned vertex labels; an
Elliott label reads `label (degree)` with the drawing's `wave_degree`. Cyclic lines and time cycles
repeat vertical marks from their two anchors (at most 256 visible), and the sine line samples the
visible pane into at most 512 segments.

### Shapes

`rotated_rectangle` (an edge and a depth point), `ellipse` (two corners), `circle` (center and rim),
`triangle` (three vertices), `arc` (three anchors), `curve` (start, control point, end), and
`double_curve` (start, two control points, end) resolve into shared screen geometry for painting and
hit testing; the rotated rectangle, ellipse, circle, and triangle fill by default. `polyline` places
vertices like `path` (click to add, double-click or Enter to finish, Backspace removes the latest,
Escape cancels). `highlighter` is a freehand drag like `brush`, a translucent 12 px stroke painted once per pixel
(as the region it covers) where it overlaps itself.

### Equivalents of KLineChart's overlays

A host moving from KLineChart finds each of its drawing overlays here. Seven are tools of their own
(wire ids 240 to 246); the others are an existing tool with options, and the table says how.

| KLineChart overlay | Aeris tool |
|---|---|
| `straightLine` | `extended_line` |
| `rayLine` | `ray` |
| `horizontalSegment` | `horizontal_segment` |
| `verticalRayLine` | `vertical_ray` |
| `verticalSegment` | `vertical_segment` |
| `parallelStraightLine` | `parallel_channel` with `fill_enabled: false` |
| `priceChannelLine` | `price_channel` |
| `fibonacciLine` | `fibonacci_retracement` |
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
- the top-level `drawing_catalog` marker (`2`, written by every export, V2 and V3 included), which
  says that the drawings follow AerisTerminal upstream's B8 catalog.

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

Drawing style fields are optional and restore the kind's own defaults when omitted; the export
writes a field only when it differs from them (an emptied `labels` or `levels` list is written as
`[]`; `gann_fans`, `gann_arcs`, and the `level_*` options are always written for their tools).
Family option blocks travel in the optional `style.tool_options` object (at most 16 KiB
serialized); upstream's tools persist their options as ordinary style fields.

Documents written by earlier builds of this line keep loading. A document without
`drawing_catalog` is treated as one of them when it carries a legacy kind name or one of this
line's own tools, an anchor `time`, a `style.tool_options` object, a `drawing_price_basis`, an
`anchored_text` without `screen_x`, or a drawing of the B8 catalog without a field upstream writes
for every drawing of that tool (the `level_*` options of a level tool, `regression_deviations`,
`icon_size`, `bars_pattern`, `wave_degree`), which recognizes such a document written on a
tick, volume, or range-bar axis, where it carries no anchor `time`; a document from an upstream pin
carries none of these and loads as is. (A marker-less tick, volume, or range-bar document of this
line whose only B8 drawings are unleveled shapes such as arcs, curves, rotated rectangles, or sine
lines carries no sign either and loads unconverted.) Restore converts such drawings
deterministically before validating them: legacy names map to their canonical kinds; drawings with
the old anchor counts get upstream's anchors (`disjoint_channel` 3 to 4, `gann_square_fixed` 1 to
2, `projection` 3 to 2, `price_note` 2 to 1, `signpost` 1 to 2, `bars_pattern` 2 to 3,
`pattern_triangle` 4 to 5, `pattern_three_drives` 7 to 6); in a document of this line `arc`,
`curve`, `double_curve`, `rotated_rectangle`, `fibonacci_speed_arcs` (centered on the first anchor,
now the second), `fibonacci_circles` (centered between the anchors, now on the second), and
`sine_line` (two opposite extremes, now a zero crossing and an extreme) anchors are converted to
their new meaning, an `anchored_text` takes its pane-fraction anchor as its screen position, omitted
values take the defaults of the build that wrote them, and `tool_options` keys with a flat
counterpart move onto it (a retracement's or speed fan's `reverse` inverted, since this line put
their level 0 on the second anchor). Clipboard and sync payloads of earlier builds get the
anchor-count conversions, a bars pattern's snapshot from its `tool_options` bars, and an anchored
text's screen position from its pane-fraction anchor; the same-count conversions need a document.
Some conversions lose detail: a three-drives pattern drops its last leg, a projection its sector
radius, a price note its label offset, a signpost starts with a zero-height pole, a bars pattern
loses its box fit, a `flat_top_bottom` its crossing split, a fixed Gann square without
`scale_ratio` gets a second anchor that may sit far from the square, a rotated rectangle may slide
along its axis on screen, speed arcs open toward their other anchor rather than up or down, a sine
wave is drawn only from its zero crossing on, a converted circle center sits at the price midpoint
(on a log scale the pixel midpoint differs), a reversed spiral loses its counterclockwise turn, and
a regression's two deviation sides and their switches fold into one symmetric
`regression_deviations` at the wider enabled side (a +3/-1 band becomes ±3, a one-sided band
two-sided). The next export writes the converted drawings with the marker.

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
  character boundary. A run label stays on one line; family text boxes keep line breaks (since the
  B8 catalog merge below only `simple_annotation` has one; the annotations are one-line runs).
  Native hosts get click-to-caret placement
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
three range tools independently, and the merge kept the own line's implementation. The later B8
catalog merge (below) kept that implementation but adopted upstream's spelling and ids, so the
names and ids in this group describe pins between the two merges. Which spelling a pin has depends
on its side:

- An upstream pin from `5a2e6e8` on has `DrawingKind::DatePriceRange`, the kind name
  `date_price_range`, and the wire ids 13 (`PriceRange`), 14 (`DateRange`), and 15
  (`DatePriceRange`). An own-line pin from `36c9f09` on (`36c9f09` itself, for example) already has
  `DrawingKind::DateAndPriceRange`, the name `date_and_price_range`, and the ids 130, 131, and 132,
  which main kept until the B8 catalog merge. For an own-line pin there is no rename and no id
  remap; only the grid snap below applies. A pin on either side before those commits has no range tools.
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

**B8 drawing catalog and B7 resampling** (both lines, from the merge that took upstream's
`57e00de feat(charts): complete B7 profiles and resampling` and `1b81852 B8: complete professional
drawing catalog expansion` onto the own line's `ace49b5`; see [Drawing families](#drawing-families),
[Persistence V1](#persistence-v1), and [Resampling](#resampling)). Upstream's `1b81852` and the own
line's `36c9f09 feat(charts): B8 drawing catalog, multi-calendar overlays, bounded ticks, tick-built
candles, and resampling` had built the same drawing catalog independently. The merge adopted
upstream's catalog, names, wire ids, anchor contracts, option fields, and renderers, and kept the
own line's seven own tools, its measuring-tool implementation, its data readers, and its id-based
text editing. An upstream pin from `1b81852` on already has upstream's catalog, so only the items
for upstream pins apply to it; an own-line pin (any pin up to `ace49b5`) takes the items for
own-line pins.

- Kind names (own-line pins). The 28 kinds in the table under [Drawing families](#drawing-families)
  are renamed. The old names are still read (a Rust `#[serde(alias)]` and `DrawingKind::from_name`;
  the TS `drawing_kind_alias` normalized through `DRAWING_KIND_ALIASES`) and never written:
  `DrawingKind::name`, `drawings()`, handles, payloads, and exported documents return the canonical
  name, so a host that compares kind strings compares canonical names. In Rust,
  `DateAndPriceRange` is `DatePriceRange`, `FlatTopBottom` is `FlatTopChannel` or
  `FlatBottomChannel`, `Icon` is `IconStamp`, the `Fib*` and `TrendBasedFib*` variants are the
  `Fibonacci*` variants, `XabcdPattern`, `CypherPattern`, `AbcdPattern`, `HeadAndShoulders`,
  `TrianglePattern`, and `ThreeDrivesPattern` are `PatternXabcd`, `PatternCypher`, `PatternAbcd`,
  `PatternHeadShoulders`, `PatternTriangle`, and `PatternThreeDrives`, the `Elliott*Wave` variants
  drop `Wave` and `Elliott*Combo` becomes `Elliott*Combination`, and `ArrowMark*` is
  `ArrowMarker*`. `DrawingKind` follows upstream's order, with the own tools after
  `BarsPattern`.
- New kinds (upstream pins). `DrawingKind` gains `HorizontalSegment`, `VerticalRay`,
  `VerticalSegment`, `PriceLine`, `PriceChannel`, `SimpleTag`, and `SimpleAnnotation`; the enum is
  not `#[non_exhaustive]`, so an exhaustive `match` needs the arms. A document that uses them does
  not load on an upstream pin.
- Wire ids. For own-line pins every B8 id moved: the own line's per-family ids 32 to 200 are now
  upstream's 0 to 84 (for example `ray` 32 is 16, `fib_retracement` 64 is `fibonacci_retracement`
  36) or, for the own tools, 240 to 246 (`horizontal_segment` 38 is 240, `vertical_ray` 39 is 241,
  `vertical_segment` 40 is 242, `price_line` 41 is 243, `price_channel` 52 is 244, `simple_tag` 147
  is 245, `simple_annotation` 148 is 246), and the ranges 130 to 132 are 13 to 15. A host that kept
  numeric ids remaps them through `DRAWING_KIND_TO_U8`; documents and payloads carry names and need
  nothing. For upstream pins ids 0 to 84 are unchanged and 240 to 246 are new.
- Anchor contracts (own-line pins). Thirteen tools take upstream's anchors: `disjoint_channel` has
  four independent anchors (was three with a mirrored slope), `gann_square_fixed` two corners (was
  one anchor plus `size_bars` and `scale_ratio`), `projection` two (apex and target, was apex,
  radius point, and price point), `price_note` one (was two), `signpost` two (foot and plate, was
  one), `bars_pattern` three (two source anchors and a target, was two box anchors),
  `pattern_triangle` five (was four), `pattern_three_drives` six (was seven), `arc` takes its point
  of passage second (was last), `curve` and `double_curve` take Bezier control points (were points
  on the curve), `rotated_rectangle` takes an edge and a depth point (was the short sides' midpoints
  and a width point), `anchored_text` is placed by `screen_x`/`screen_y` (was a pane-fraction
  anchor), and `flat_top_bottom` is two tools. A host that builds anchors for these tools builds
  upstream's. Restored documents convert, keyed by the `drawing_catalog` marker as described under
  [Persistence V1](#persistence-v1), with the recorded losses listed there; every export writes the
  marker, which an upstream pin ignores.
- Options (own-line pins). Upstream's tools read flat options (`levels` with the `level_*` options,
  `gann_fans`, `gann_arcs`, `wave_degree`, `screen_x`, `screen_y`, `icon_name`, `icon_size`,
  `bars_pattern_*`, `regression_source_id`, `regression_deviations`) instead of
  `tool_options.fibonacci`, `gann`, `pattern`, `shape`, and the regression, bars-pattern, and icon
  keys. Those keys are still accepted: a key with a flat counterpart moves onto it (an explicit flat
  option wins), and the others are kept and persisted but change nothing. New drawings take
  upstream's defaults (seven Fibonacci levels, wave degree `minor`, four info-line labels, centered
  labels on the line and channel tools), while restored documents of this line keep the defaults
  they were written with.
- `DrawingKindOptions` (own-line pins). The `Fibonacci`, `Pitchfork`, `Gann`, `Pattern`,
  `ElliottWave`, and `Shape` variants are removed, `RegressionTrend` is
  `{ source_id: Option<u32>, deviations: f64 }`, and `Levels`, `GannSquare`, `Elliott`,
  `AnchoredText`, `IconStamp`, and `BarsPattern` are new; the TS `drawing_kind_options` union drops
  the `"fibonacci"`, `"pitchfork"`, `"gann"`, `"pattern"`, `"elliott_wave"`, and `"shape"` arms in
  the same way. The option types (`FibonacciToolOptions`, `GannToolOptions`, `PatternToolOptions`,
  `ShapeToolOptions`, and the rest) stay exported for the stored blocks. For upstream pins the
  `Line`, `Channel`, and `ProjectionAnnotation` variants are new, as are the `Drawing::tool_options`
  field and the root exports `DrawingAnchor`, `DRAWING_WEAK_MAGNET_DISTANCE`, and
  `MAX_BARS_PATTERN_BARS`.
- Value domains. `wave_degree` adds `submillennium`, `millennium`, and `supermillennium` (upstream
  pins); `bars_pattern_mode` adds `"oc_bars"` and reads `"hl_bars"` as `"bars"` (upstream pins), and
  `"hl_bars"` is no longer a mode of its own (own-line pins). `MAX_BARS_PATTERN_BARS` is 512 finite
  rows (own-line pins: was 128 aggregated buckets). `icon_size` is 8 to 96 (own-line pins: was 8 to
  128; larger stored values clamp).
- Icons (own-line pins). The glyph tool `icon` is `icon_stamp`, which paints a host-registered RGBA8
  image: `chart.register_drawing_icon(name, width, height, pixels)` and
  `chart.remove_drawing_icon(name)` (Rust `ChartEngine::set_drawing_icon` and `remove_drawing_icon`).
  A name from the built-in set (`star`, `heart`, `check`, `cross`, `circle`, `square`, `diamond`,
  `triangle_up`, `triangle_down`) without a registered image paints the glyph, so converted
  documents look as before. For upstream pins that fallback is new; other unregistered names keep
  the placeholder.
- Anchored text. Own-line pins: the pane-fraction anchor model is gone; `screen_x` and `screen_y`
  hold the position, and the anchor is an ordinary data anchor from which the position is derived
  when the drawing is added or its anchors are set. Upstream pins: the placement preview now paints where the click will land rather
  than at the default position.
- Text annotations. Own-line pins: the note, comment, callout, price note, and anchored text edit as
  one-line runs (no longer multi-line boxes; only `simple_annotation` keeps a box), a single click on
  a selected annotation reopens its editor, and an annotation left empty is removed on commit or
  Escape (it used to stay). Upstream pins: they edit through the engine session and the id-based
  `drawing_text_edit_layout(id)` described under **Drawing text editing** above.
- Data-reading drawings. Upstream pins: a regression trend, forecast, and bars pattern read only the
  rows the replay clock shows, read each bar of an as-of source once, and follow streaming updates
  of their source (earlier upstream revisions left a regression or forecast stale after a live
  bar). Without `regression_source_id` the source is the first live ordinary series on the
  drawing's pane and price scale, in creation order, where upstream took the scale's primary
  series. The regression band uses population deviations
  (`regression_deviations`), and the forecast's labels are `target reached`, `expired`, and
  `pending`. A window with fewer than two closes paints the dashed segment between the regression's
  anchors, still selectable, where upstream painted nothing and could not be hit. A forecast counts
  bars after its entry bar (upstream also counted the entry bar, so a target the entry bar already
  touched read `target reached`), expires once a traded bar after its target bar exists (upstream:
  once the data reached the target bar), and prints the target bar's time one line above its label.
  Own-line pins: the regression fit uses closes with one symmetric deviation over the bars whose
  positions lie between the anchors (was the bars at the rounded anchor positions, so anchors at
  2.4 and 7.6 fitted bars 2 to 8 and now fit 3 to 7), and a window with one close has no fit (was a
  flat line); asymmetric or toggled deviations, `source`, and Pearson's R are retired, and a
  regression moves on both axes like every upstream tool (was time only: its anchors' prices are
  now its handles' positions). The forecast's source and target boxes are retired (see below).
- Labels and text (upstream pins). The line and channel tools built from upstream's `line_spec` and
  `channel_spec` (`ray`, `extended_line`, `info_line`, `trend_angle`, `arrow_line`,
  `parallel_channel`, `flat_top_channel`, `flat_bottom_channel`, `disjoint_channel`) lay their
  `text` out along the first two anchors like the trend line (`text_layout` `Segment`): the label
  rotates with the segment, takes the stroke color, and a middle-aligned label splits the stroke,
  where upstream painted an unrotated label in the text color against the line's box. This line's
  behavior was kept over upstream's. A `date_time_range` label prints the anchors' times through the
  bar label and `duration` the elapsed time (upstream printed `range` and a bar count), and a
  `bars_pattern` in `"oc_bars"` mode is hit on its sticks (upstream's hit test read the close line,
  which that mode does not paint).
- TypeScript types (upstream pins). `drawing_kind` also holds the 28 `drawing_kind_alias`
  spellings, so a `Record<drawing_kind, T>` table or an exhaustive `switch` over it needs them, or
  narrow to `Exclude<drawing_kind, drawing_kind_alias>` (what every output returns).
  `DRAWING_KIND_TO_U8` is `Record<Exclude<drawing_kind, drawing_kind_alias>, number>` (was
  `Record<drawing_kind, number>`): index it with a canonical name, normalizing an input first
  through `DRAWING_KIND_ALIASES`. `resampled_bar`'s `open`, `high`, `low`,
  `close`, and `volume` are `number | null` (`null` for a whitespace bucket), which a
  `strictNullChecks` host must handle.
- Annotations (own-line pins). Placing a `signpost` no longer opens the text editor (upstream's
  signpost is a two-anchor marker), and a forecast's label is upstream's one line plus the target
  time. The line tools' visible `labels` print one per line through upstream's label path instead of
  one engine-formatted stats box (the values stay engine-formatted). Fork-era clipboard and sync
  payloads with the old anchor counts paste (see [Persistence V1](#persistence-v1)).
- Persistence export (upstream pins). An emptied `labels` or `levels` list is written as `[]` and
  restores empty (upstream wrote nothing, so the defaults came back), and `stroke_start`,
  `stroke_end`, `extend_*`, and `fill_enabled` are written relative to the kind's defaults.
- Resampling (TS). Own-line pins: `resample_boundary` is `{ startTime, endTime, sessionId }`, so
  `resample_boundaries()` returns camelCase rows; `resampled_bars()` returns `resampled_bar` rows
  (`timestamp`, `sessionId`, `open`, `high`, `low`, `close`, `volume` with `null` for whitespace,
  `sourceRows`); `configure_resampled_series(source, target, options, volume_source?,
  volume_target?)` is the primary form, and the `(target, options)` form is a deprecated overload
  that still reads snake_case boundaries (typed `legacy_resample_boundary`). The output rows have no
  snake_case aliases: a host that read `boundary.start_time`, `bar.time`, or `bar.source_rows` reads
  `startTime`, `timestamp`, and `sourceRows`. Upstream pins: `resample_boundaries()`,
  `resample_stats()`, the deprecated overload, and a numeric `target` in `resampled_bars` are new.
  On both lines the derived volume sums the bucket's whole span and skips whitespace rows.
- Resampling (Rust, upstream pins). The engine takes the own line's tail refresh, whitespace bucket
  reservation, replay-clock cutoff, time-axis requirement, and chain refusal (see
  [Resampling](#resampling)); `resample_boundaries`, `ResampleSpan`, `ResampleStats`, and the
  `ResampleError` variants `InvalidSessions` and `TimeAxisRequired` are new.
- Retired own-line rendering (own-line pins). Upstream's renderer replaced the own line's for every
  shared tool, and these extras did not carry over: the one stats box the line tools' `labels`
  rendered as (the info line's five statistics and `stats_position` included), a ray's
  `extend_left` and `extend_right` toggles (a ray is always a ray; use an extended or trend line), the trend angle's arc and reference line, and the own arrowheads; the parallel
  channel's middle line, `flat_top_bottom`'s crossing split, and the regression extras above; the
  Fibonacci per-level palette lines, dashed trend line, fan grid, full circles, vertical label
  alignment, phi spiral, and level labels and selected bands as hit targets; the pitchforks' zone
  fills as hit targets and base-midpoint handle, the Gann box time levels and
  angles, the square stats box, the fan `scale_ratio`, the fixed square's size and corner handle,
  and price-basis rescaling of Gann options; harmonic ratio connectors and labels, point labels as
  hit targets, shaded XABCD
  triangles, the head-and-shoulders neckline, the triangle apex extension, the twelve-degree Elliott
  notation (labels now read `label (degree)`), `show_wave`, and progressive previews; the
  projection sector, the note pin and reveal on focus, the price-note leader, speech bubbles, the
  default texts of new annotations (they start empty), the
  signpost pole and its editor on placement, arrow-marker text, multi-line annotation boxes, the
  bars-pattern box fit and aggregation, and the forecast's source and target boxes (the absolute
  change, `Success`/`Failure` on market colors, and the box as a hit target; the target time stays
  above upstream's label); and the symmetric rotated rectangle and its width handles, ellipse bounds handles,
  on-curve anchors with tangent extension and chord fills, closed polylines, and clip-aware curve
  flattening (the highlighter is now 12 px wide). Their options stay stored but inert. Kept on
  upstream's renderer: channel `extend_left`/`extend_right` (upstream pins: new), the callout's
  tip and box handles (upstream pins: new), the highlighter painted once per pixel as its stroke's
  region (upstream pins: a self-overlapping highlighter no longer blends twice on WebGPU), and the
  regression's dashed anchor segment without a fit.

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
