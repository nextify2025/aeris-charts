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
  `false`), and VWAP/pivot lines that restart at every reset;
- multi-calendar overlays through the series options `time_alignment: "as_of"` and
  `as_of_max_staleness` (see [Multi-calendar overlays](#time-exchange-time-zone-and-trading-sessions));
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
`add_series` options. Rust hosts call
`ChartEngine::set_series_time_alignment(id, TimeAlignment::AsOf { max_staleness })`. Like other
financial series options, the setting is host-owned and not persisted.

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
open time, the canonical Aeris bar time; platforms that label a minute by its close (同花顺, 富途)
show the same bars one interval later.

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
though winter sessions run past UTC midnight. Every boundary carries the trading date as
`session_id` (`YYYYMMDD`). Dates are strictly ascending and use `session_slot_times` placement
(night sessions with a negative `session_start` included); at most 20 000 boundaries and 32
resampled series per chart. Hosts may also pass their own `{ start_time, end_time, session_id }`
periods (for weeks or months, for example).

**Source rows.** Source rows must be stamped with bar-open times; rows outside every boundary are
omitted, so shift close-stamped minutes (09:31 … 15:00) back by one interval first. A feed that also
carries a separate opening-auction minute (241-bar feeds stamp it 09:30 beside the close-stamped
09:31) must merge that row into the first minute before resampling: shifted, it falls before the
first window and is omitted with its volume. Whitespace rows (`session_slot_times` reservations)
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
(an API call, a placement, a freehand stroke, a pointer drag or keyboard nudge that moved something,
a text edit) advances the sync revision, so an already synced cell accepts the next payload.
Clipboard payloads are bounded like a persisted drawing document (at most 10,000 drawings, 250,000
anchors, and 8 MiB): `copy_drawings` throws `resource_limit` past them and `invalid_data` when no
listed drawing exists, and `clone_drawing` copies any drawing the chart holds. Named templates
(`drawing_template`, `apply_drawing_template`) carry style only: never a drawing's name, group,
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
handle by the nudge distance. `drawing_handle_count()` counts them. A nudge that moves nothing (a
locked drawing, an axis the drawing cannot move along, a clamp at the pane edge) records no undo
step and is announced as such, so Escape rolls back only the nudges that moved the drawing.

A drawing's own text is edited in place in the chart's inline editor: the text tool, a trend line's
label, and the text boxes of the Projection & Annotations tools listed below. A double-click on a
selected drawing, or Enter or F2 while the chart has focus and the drawing is selected (F2 on its
accessibility drawing target, where Enter keeps geometry editing), opens the editor; locked,
hidden, and interval-hidden drawings do not open it. Typing repaints live, Enter or leaving the
editor commits, and Escape restores the text. The whole edit is one undo step and reaches
`drawing_sync_payload` once, on commit. Family text boxes take several lines (Shift+Enter adds one,
paste inserts plain text); the text tool and trend labels stay single-line. The editor is a
labeled text box that announces opening and closing through the accessibility live region and
returns focus to where it was opened from.

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
anchor handles, and each drag or nudge is one undo step, including any option it edits. Family-specific options live in one block
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
  line's; inline hover editing remains a trend-line feature.
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
3×3 box label of other tools) and renders any visible `labels` as engine-formatted stats.

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
  `labels`: price change, percent change, and ticks; bar count and duration; or all five.
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
- `icon` (one anchor): `tool_options.projection_annotation.icon` — `"star"` (default), `"heart"`,
  `"check"`, `"cross"`, `"circle"`, `"square"`, `"diamond"`, `"triangle_up"`, or
  `"triangle_down"` — centered on the anchor, `icon_size` CSS px across (8..128, default 24), in
  the drawing color.
- Kind defaults (fills, arrows, stats, default texts, arrow colors) are schema defaults and are
  omitted from persistence; a cleared default text persists as `""`.
- The text of `anchored_text`, `note`, `price_note`, `callout`, `comment`, `price_label`,
  `signpost`, and the arrow marks edits in place (see inline text editing above); the price note
  and price label keep their price line above it. An emptied box keeps one caret line while it is
  edited. Placing one of these tools does not open the editor.
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
  enclosed region by the nonzero rule.
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
