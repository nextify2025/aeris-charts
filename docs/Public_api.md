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
DOM/WASM chart creation starts from the mounted component effect.

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
leaves the current series unchanged. Shared-ring drains reject malformed rows individually.
Worker charts expose the most recent result through `offscreen_chart.last_ingestion_diagnostics()`.

## Persistence V1

Persistence schema versioning is independent of the npm package version. V1 contains only:

- ordered pane identities, stretch factors, and preserve-empty flags;
- ordered built-in drawings with persistent ID, kind, pane reference, semantic anchors, and style.

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

Limits for untrusted input are 8 MiB per document, 64 panes, 10,000 drawings, 100,000 anchors per
drawing, 250,000 total anchors, 64 KiB text per drawing, and 1 MiB total drawing text. Unknown
optional V1 fields are ignored. Unknown schema versions, drawing kinds, pane references, duplicate
IDs, invalid anchor counts, non-finite/unsafe numbers, and limit violations fail structurally and
leave the chart unchanged. V1 fixtures are compatibility inputs; future versions must retain an
explicit V1 migration path for the documented compatibility window.

## Persistence V3 studies

Financial charts with engine-owned indicators export schema version 3. V3 keeps market history and
ordinary series data host-owned, but persists ordered study bindings, scalar input selection, typed
indicator parameters, timestamp-aligned volume-source references, and per-output styles. Chained
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

`aeris_charts_render_gpui` remains repository-only and experimental. It pins `gpui-pre` 0.3.6,
the GPUI snapshot gpui-kit 0.6.6 depends on, with an exact version requirement; GPUI upgrades are
explicit manifest and lockfile changes.

## Release policy

Tag publication depends on required Rust, package, and portable Chromium/Firefox/WebKit jobs. It also
checks persistence fixtures, public declarations, Node/SSR import, package contents, and configured
performance budgets. `continue-on-error` is forbidden for portable correctness.

Hardware- and machine-sensitive screenshot hashes, GPU timing, heap sampling, and wall-clock evidence
are calibration diagnostics. They remain non-authoritative and may not be approved merely to make one
runner green. Shared draw-stream parity, clipping/order/frame-contract tests, replay determinism, and
portable browser behavior are the authoritative gates. Scenarios without a configured benchmark
budget remain explicitly report-only.
