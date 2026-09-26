# Aeris Charts Architecture

## Purpose

Aeris Charts is a high-performance financial chart engine. It provides chart state, interaction behavior, drawing tools, indicators, frame construction, and multiple rendering backends for [Aeris Terminal](https://aeristerminal.com) and browser hosts.

The engine is backend-neutral and host-neutral. One canonical state must produce equivalent frames across GPUI, WebGPU, Canvas2D, and native test rendering. Performance, visual parity, deterministic behavior, and bounded resource use are product requirements.

Source code, tests, and measured release behavior are the implementation truth. This file must change in the same commit whenever the architecture changes.

## Data flow

```text
Host API and market data
    -> aeris_charts_core validation, data, scales, and options
    -> aeris_charts_engine chart state and interaction
    -> ChartFrame and aeris_charts_render DrawList
    -> GPUI | WebGPU | Canvas2D | tiny-skia executor
    -> pixels and frame metrics
```

Browser hosts enter through `packages/charts`, which translates the supported public TypeScript API into typed arrays and WebAssembly calls. Rust hosts use the versioned `aeris_charts_engine` crate directly and select a renderer from the coordinated crates.io release, first published as version `0.1.0`. Repository development still resolves those crates through local paths, with the same explicit version used for registry consumers. The GPUI executor remains repository-only because it tracks a reviewed Zed commit whose API differs from the crates.io `gpui` release. Rendering backends consume prepared frame data; they do not own chart semantics.

The web demo exposes all built-in calculation APIs in a searchable Indicators catalog. Entries create their engine bindings on demand and remove all owned outputs and synthetic volume dependencies when cleared. RSI uses the same engine calculation and oscillator pane as package consumers; no separate demo formula is maintained.

Canonical series data uses opaque chart-local `u32` identities mapped to reusable storage slots. Identities are never reused, removed identities are classified as stale, and slot-backed vectors remain bounded by peak concurrent series rather than lifetime add/remove count. Each ordinary series owns one timestamp column and either one scalar value column or four OHLC columns. `PlotList` owns only a dense range or sparse logical-index mapping plus its chunked autoscale cache; allocation-free views join that mapping to the canonical values for queries and frame construction. Dense aligned mappings carry no per-row index allocation. Indicator outputs own one scalar value column and alias a contiguous source-time range by identity, so they duplicate neither source timestamps nor plot values. The merged timestamp union remains independently owned because ordinary source series are independently mutable and may diverge or carry whitespace. It carries a generation that changes only when its contents change; time weights use that generation, while value-only current-bar updates retain the O(1) fast path.

Each canonical built-in series also owns an eager fanout-16 row-summary pyramid. A summary stores only six `u32` source-row identities: chronological endpoints, OHLC low/high, and close minimum/maximum. Values are always dereferenced from the canonical columns, so the hierarchy adds no second value owner and preserves whitespace. Point and tail mutations repair one node per affected level; typed batches repair the affected range once; historical insertion, replacement, and retention rebuild or repair the exact affected hierarchy before the mutation is visible. The same endpoint summaries bound latest and predecessor lookup to at most fanout work per hierarchy level even across pathological whitespace; no parallel predecessor index or value history is retained. Series removal releases the hierarchy with the canonical storage slot. Custom-series host geometry is not summarized because its semantics are not engine-owned.

Each canonical series also carries a data generation. An ascending typed batch is sanitized once at the host boundary, merged into its source in one data-layer operation, then synchronizes merged time points, tick weights, dependent indicators, and frame generations once. Tail batches append weights incrementally; historical batches merge in `O(n + k)` and reindex once rather than once per input row. A data-layer transaction that actually rebuilds the timestamp union temporarily captures its prior union and emits one old-to-final logical mapping through common timestamps when the engine synchronizes. Current-bar replacements and pure tail appends capture and map nothing, and no second merged timeline survives the synchronization boundary.

One core validator defines canonical numeric time: a finite, integral count of whole UTC seconds in
the inclusive range `-62167219200..253402300799` (years 0000..9999). Hosts do not auto-convert
numeric timestamps. Out-of-range errors include a likely milliseconds, microseconds, or nanoseconds
hint when dividing by that unit would enter the supported range. Direct set/update batches validate
all timestamps before repair or mutation and reject atomically; single updates likewise preserve
state. Shared-ring drains may reject individual rows because producer drains cannot be rolled back.

## Crate boundaries

### `aeris_charts_core`

Platform-free chart fundamentals: validated canonical columnar data, compact plot index/view storage, ranges, options, formatting, price scales, time scales, tick marks, and shared math. It also exposes structure-level payload and capacity attribution for memory evidence; these counters are not allocator, WASM-page, or browser-memory measurements. Media-space calculations remain `f64`; conversion to backend coordinate formats happens at rendering boundaries.

General Cartesian scale foundations live beside, rather than inside, the financial scales. `LinearScale`,
`LogScale`, and `SymLogScale` map continuous numeric domains and emit bounded deterministic ticks;
`BandScale` and `PointScale` map caller-owned category indices without retaining labels or allocating
category state. All five keep their math in `f64`, accept reversed ranges, and have no host or renderer
dependency. Linear normalization, interpolation, and tick selection remain finite for every pair of
distinct finite domain endpoints, including spans whose direct subtraction overflows. General axes with
explicit numeric, temporal, band, or point domains use these scales during
shared layout and axis-frame construction; temporal coordinates reuse the linear transform over validated
JavaScript-safe epoch milliseconds while the engine owns UTC calendar interval selection and formatting.
The financial coordinate path does not dispatch through them.
Existing financial charts therefore continue to instantiate only `TimeScaleCore` and `PriceScaleCore`
and pay no retained-memory cost for these foundations.

Each pane has one immutable horizontal-domain binding. Absence of a general binding means
`financial_time` and continues to use the chart's established `TimeScaleCore`; this is the initial
pane and every legacy `add_pane` call. Non-financial continuous, temporal, category, and polar
declarations live in a chart-owned registry that allocates only on first use, is capped at 64 live
entries, uses monotonic internal identities, and releases entries with their panes. Pane moves and
swaps carry the binding. Until compatible general series and axes are installed, financial series
cannot move into a general pane, and V1 persistence rejects rather than silently reinterprets a
general pane. The existing financial frame path never dispatches through the registry.

General axes are chart-owned objects with unique case-sensitive UTF-8 IDs and monotonic internal
handles. Their options retain dimension, resolved placement, scale type, automatic or explicit
domain, direction, visibility, title, bounded tick policy, band padding, zero-line policy, and grid
policy. Validation is atomic: Cartesian X axes must match the pane domain; Cartesian Y axes are
numeric; polar panes accept only angular-category and radial-linear axes; explicit domains must
match the scale and category labels must be unique. Axis count, identity/title bytes, tick count,
category count, and category bytes are bounded and included in engine memory attribution. Pane
moves preserve axis ownership through stable pane IDs. Explicit temporal bounds are ascending epoch
milliseconds within JavaScript's exactly representable integer range. Pane removal releases its
axes. Visible Cartesian axes reserve engine-owned top/bottom plot space and measured left/right strips;
multiple axes stack in insertion order, vertical widths use the existing grow-fast/shrink-on-full-layout
policy, and the resulting rules, titles, and collision-filtered ticks are emitted through the common
`AxisFrame`. Each side may reserve at most 45% of the space remaining after financial axes; complete
strips that do not fit are omitted, preserving a nonzero plot and keeping unscissored axis chrome inside
the chart. Category selection and numeric tick generation are capped at 512 candidates. Explicit
Cartesian numeric, temporal, and category ticks share that cap, reject duplicate or scale-incompatible values,
and retain optional preformatted labels as portable engine state; out-of-view ticks are clipped by
the same transform that places generated ticks and grid rules. Automatic band
and numeric domains now resolve from visible bound general series without rewriting configured axis
options; hidden series stop contributing immediately. A single extreme numeric value expands inward
when outward padding would overflow; logarithmic domains use the adjacent positive value when a
percentage expansion rounds back to the same endpoint. Continuous X/Y axes execute linear, logarithmic,
or symmetric-log transforms consistently for ticks, geometry, hit testing, and runtime pan/zoom; a
runtime view is independent of the configured/automatic base domain and can be reset without rewriting
axis options. Temporal axes use that same runtime-view contract with whole epoch-millisecond anchors and
emit bounded UTC millisecond-through-calendar-year ticks through the shared `AxisFrame`; locale injection
supplies month names without moving date math into a host or backend. Polar tick execution remains deferred
until its owning transform slice is implemented. Cartesian grid and numeric zero-line policies execute
from the same effective domains into the retained pane underlay, below references and series. Axis-local
grid visibility combines with the chart-wide direction style, coincident device-pixel rules are deduplicated,
and an enabled zero line replaces a coincident ordinary grid rule. Financial panes allocate no general
axis storage and retain their established price/time axis output unchanged.

Category runtime views are bounded index windows over the current configured or automatic registry.
Zoom anchors use a category identity in the visible window, pan advances by a rounded visible-window
fraction, and registry changes clamp the window without retaining stale category strings.

General Cartesian data has a separate engine-owned typed-column store beside `DataLayer`. The first
storage slice accepts numeric, epoch-millisecond temporal, and interned-category X columns plus numeric Y
values, explicit validity, and stable generated or caller-provided row identities. Installation and
replacement validate the complete batch before mutation; NaN/infinity, duplicate explicit IDs, invalid
category indices, mismatched columns, and over-limit dictionaries are rejected atomically. Dataset and
row counts, category bytes, and ID bytes are bounded, and retained capacity is attributed separately in
engine memory evidence. A financial-only chart keeps the store absent and therefore retains zero general
dataset capacity.

The released Phase 2 Cartesian bindings are category-band columns, horizontal bars, box plots, category/category
plus numeric/numeric and temporal/numeric heatmaps, numeric XY scatter/bubble marks, numeric/temporal/category
error bars, and `xy_line`, `xy_area`, `range_area`, and category-band `range_bar`. General
series have monotonic chart-local identities, stable pane/axis/dataset ownership, bounded title/color
state, and lazy registry allocation. Populated axes and datasets cannot be removed out from under a
series, and a pane containing a general series cannot be removed until that series is detached. Visible
column series contribute their category union and finite valid Y values to automatic domains; the zero baseline participates in the Y
domain. Horizontal bars reuse the same category/value dataset but bind the numeric value scale to X and the
category band scale to Y; category Y autoscale is engine-owned and the numeric X zero baseline participates in
autoscale. Phase 2 bar layout options add bounded `group_id`/`stack_id` state without creating renderer-specific
primitives. Visible members of one group subdivide each category band, while one stack consumes one group
slot. Normal stacks accumulate positive and negative values independently from zero and contribute their
summed category extents to the oriented numeric-axis autoscale; percent stacks normalize each category independently to `+1` and
`-1`. Horizontal stacks apply the same rules on numeric X; vertical stacks apply them on numeric Y.
Stack membership requires the same pane, group, axes, orientation, and stack mode. Missing rows remain queryable
and accessible but emit no mark or stack contribution. Bar geometry is computed once
in shared CSS-space semantics, reused by frame painting and exact/nearest hit testing, then lowered to
ordinary ordered `Rect` primitives. Bounded tooltip and accessibility snapshots come from the same rows.
Scatter binds independent continuous numeric axes, validates logarithmic positivity, clips geometry to
the runtime view, and lowers persisted circle, square, diamond, or triangle symbols to existing ordered
primitives. Path point markers share the same symbol contract; bubbles remain area-scaled circles. Exact
hits follow each symbol boundary. The lazily rebuilt scatter screen-space grid is keyed by dataset
generation, plot geometry, axis domains/transforms, direction, and point radius; grid
cell count is capped, retained capacity is attributed to engine memory, and exact/nearest hits inspect
only intersecting cells while preserving stable series/row tie-breaking. Bubble reuses that point/index
contract with a required typed size channel, square-root area-to-radius mapping clamped to the shared
point-radius bound, and queryable zero/missing sizes that emit no mark. The size channel participates in
atomic replacement, explicit-ID updates, bounded retention, accessibility, tooltip snapshots, memory
accounting, and V2 persistence. `xy_line` and `xy_area` reuse the same
general dataset/axis ownership across continuous numeric, temporal epoch-millisecond, and category
band/point X domains. Missing rows split path runs by default, while persisted `connect_missing`
can bridge them without removing their queryable identity. Transform-invalid rows always remain hard gaps.
Their persisted `linear`, horizontal-then-vertical `step`, and Catmull-Rom `curved` interpolation policy
travels on the ordered frame primitive and drives both shared lowering and exact/nearest hit geometry.
`xy_line` lowers each run to the shared point pool plus ordered `Polyline` primitives. `xy_area` adds an
ordered `AreaFill` before the matching stroke; its zero baseline is clamped into linear/symlog plots and
falls back to the lower-domain plot edge when a logarithmic Y axis has no zero coordinate. A persisted,
bounded fill opacity preserves the shared 3:1 top-to-baseline gradient and scales stacked/range bands from
the same value. Line hits use
segment distance, while area hits include the filled trapezoid and both preserve the closest endpoint row
identity. When `xy_area` has a `stack_id`, visible members with the same pane, X/Y axes, stack ID, and stack
mode, interpolation, and missing-row connection policy align by exact numeric, epoch-millisecond, or
category X identity rather than row position. Positive and negative values accumulate independently;
percent mode normalizes each sign independently to `+1`/`-1`.
Cumulative extents participate in Y autoscale, and each layer becomes a variable-bound `BandFill` between
the preceding stack boundary and the new cumulative boundary while retaining the upper area stroke and row
interaction identity. `range_area` adds bounded typed low/high columns beside the same X domains, rejects inverted
complete bounds atomically, treats either missing or transform-invalid bound as a run break, and emits one
ordered `BandFill` plus its two boundary polylines from shared geometry. Band hit testing returns the nearest
contributing row, while tooltip/accessibility snapshots expose both bounds. Replacement, explicit-ID updates,
retention, memory accounting, and V2 persistence keep both channels aligned. `BandFill` carries the same
interpolation policy as its boundary strokes; shared coupled expansion chooses one bounded subdivision sequence
for both edges so Canvas2D, WebGPU, GPUI, native painting, and band hit testing cannot open seams or disagree.
Numeric and temporal
`error_bar` support four independent optional bound channels around center XY values; temporal X
centers and bounds are validated whole JavaScript-safe epoch milliseconds and contribute to temporal
autoscale. Category band/point error bars center on a category and keep only the two Y-bound channels.
The engine validates bound ordering atomically, excludes absent-center rows from marks and bound autoscale,
and computes stems, caps, center circles, hits, labels, snapshots, and accessibility from one shared geometry
path. Ordered `HLine`, `VLine`, and `Circle` frame primitives keep executor semantics identical; bound
validity, explicit-ID updates, retention, memory accounting, and V2 persistence remain aligned.
Category-band `box_plot` reuses the aligned general dataset with five ordered numeric statistics:
`min`, `q1`, `median`, `q3`, and `max`. Complete rows validate that order atomically; incomplete rows
remain queryable/accessibility-visible but emit no mark and do not affect autoscale. The outer whiskers drive
numeric-Y autoscale, including logarithmic positivity checks across all present statistics. One shared CSS-space
geometry path computes the IQR rectangle, median, whiskers, caps, labels, and exact/nearest hits, then lowers them
to existing `Rect`, `HLine`, and `VLine` primitives for every backend. Tooltip/accessibility snapshots expose
quartiles separately, explicit-ID updates and bounded retention preserve aligned statistics, and V2 persistence
round-trips the complete five-number summary without reinterpretation.
`heatmap_grid` keeps one aligned dataset across all supported Cartesian coordinate variants. Category/category
heatmaps add a second engine-owned category dictionary/index column for Y; the existing category X column remains
the X registry and the ordinary numeric value/validity column remains cell intensity data. Continuous numeric and
temporal X heatmaps instead add one aligned numeric Y-coordinate column while reusing the ordinary numeric/temporal
X column. Category registries validate, merge, remap, trim, and compact inside the same atomic update transaction.
Automatic domains union category registries or numeric/temporal coordinate extents as appropriate. One shared cell
geometry path maps band grids directly and infers numeric/temporal cell boundaries from neighboring coordinate
centers, derives deterministic normalized value intensity from visible valid cells, drives exact/nearest rectangle
hits and labels, and lowers every cell to the ordered `Rect` primitive. Missing values remain queryable and
accessible but emit no cell. Tooltip/accessibility snapshots expose both X and Y labels, explicit-ID updates and
retention keep coordinate/value channels aligned, and V2 persistence round-trips every heatmap coordinate shape.

General interaction/configuration state stays in the same engine registry. Shared tooltip snapshots group visible
rows by the anchor row's exact horizontal datum in stable series/row order, including duplicate-X heatmap cells.
The transient general brush converts host CSS-pixel endpoints immediately into semantic numeric, temporal, or
category ranges, returns bounded selected row identities, and reprojects from semantic values after view changes.
General reference lines, dots, and regions bind explicit axes and lower into existing shared primitives; each
reference independently declares whether its values extend automatic domains. Reference lifecycle and V2
persistence are engine-owned, while brush/hover/selection remain transient and are not serialized.
The release perf harness
includes a 100k-point general-only line target with a 16.67 ms frame budget and 8 ms nearest-hit budget.
It also keeps the current line, area, range, scatter, and bubble paths in one 100k-row mixed-general
target with the same frame/hit budgets and a 12 MiB retained-memory ceiling, then measures one engine
containing 50k financial bars plus a 50k-point general range pane against the frame budget and a 16 MiB
retained-memory ceiling. A separate 100k-row numeric error-bar target enforces the same frame/hit budgets
and a 16 MiB retained-memory ceiling. Financial-only, general-only, and combined execution therefore have separate
enforced evidence rather than unbudgeted performance claims. The official release browser benchmark also includes
a five-series/100k-row representative Phase 2 general dashboard. Its first-frame host startup has an absolute
2,000 ms p50 ceiling and its first-frame WebGPU vertex upload volume has a 96 MiB p50 ceiling; the release workflow
evaluates these through the same versioned `budgets.json` policy as artifact-size ceilings.
Dataset replacement remains atomic against every bound series and cannot change a bound path/scatter/bubble
X kind or drop a bound bubble size or range low channel. Canvas2D, retained
WebGPU, GPUI, and the native tiny-skia rasterizer consume the same frame contract; grouped/stacked columns
reuse the already-covered ordered `Rect` executor path, bubble reuses scatter's already-covered ordered
`Circle` executor path with per-row radii, stacked area reuses the range-band `BandFill` path, and column,
scatter, line/area, and range-band paths have direct executor
coverage.

The browser package exposes these general slices through the common chart lifecycle. Domain-aware pane and
axis handles remain thin mutations over engine state. General-axis browser handles carry the engine's
monotonic handle token as well as the user-visible axis ID, so removing and recreating an axis with the
same ID stales the old handle instead of retargeting it; V2 restore likewise rejects charts that already
issued a general-axis handle rather than recycling that identity. Object rows are normalized once into numeric or
epoch-millisecond temporal, or interned-category columns; typed input crosses the WASM boundary as bulk arrays, while optional string or
numeric identities cross as one bounded JSON vector. A general-series handle owns one engine dataset and
removes it transactionally after detaching the series. Pane enumeration and chart series-lifecycle events
include general handles without making financial primitive helpers reinterpret them. Tooltip, shared-tooltip,
bounded accessibility, exact/nearest hit, brush, reference-component, and legend state come back from Rust. The legend snapshot is derived
directly from the live series registry in stable engine order, optionally filtered by pane, and retains hidden
series with their visibility state rather than maintaining a parallel host registry. Shared tooltip grouping,
semantic brush selection, and reference domain extension likewise do not create browser-owned semantic mirrors.
Axis and series handles mutate visibility in place through the engine registry, preserving handle, data, view,
selection, and ordering identity while shared invalidation updates domains, hits, legends, persistence, and frames.
Their browser `apply_options` transactions also update mutable axis configuration and series presentation in
place after validating the complete candidate. General-series rebinding commits through that same engine
transaction only when the target pane has equivalent horizontal-domain semantics and its X/Y scale types match;
kind and dataset identity remain structural. Invalid candidates leave the live object unchanged. The general
registry is also the single ordering owner: exact global or pane-local permutations update paint, legend,
hit-test, React keyed-array, and persistence order while leaving other panes' relative order intact. React uses
these mutations for ordinary prop and order changes and releases a new pane or series if initial data installation
or a readiness callback throws.
The shared browser accessibility
controller recognizes financial and general handles but keeps their navigation math separate: financial
series continue to query the time scale, while general series page through at most 512 Rust-owned
accessibility rows at a time. General keyboard focus is a distinct engine interaction target rather than
an alias for hover or primary selection; explicit row identities follow reordered replacement batches,
generated batch-local identities clear, and the shared frame paints the same focus chrome for every
executor. Scatter and continuous-numeric `xy_line`/`xy_area` keyboard zoom mutate their bound general X
axis rather than the financial time scale; category/temporal path navigation remains row-oriented without
inventing an unsupported axis zoom.
Generated row
identities are encoded as decimal strings at the JavaScript boundary so their full `u64` identity is not
rounded. The ordinary browser pointer path feeds exact general hits back into engine-owned transient
hover and primary selection. Interaction targets retain row identity rather than formatted coordinates:
explicit identities follow reordered replacement batches, while generated batch-local identities clear.
Explicit-ID incremental batches update existing rows and append new rows in the shared dataset store;
an optional per-call row limit trims the oldest rows and prunes unused category labels. Validation
precedes mutation, and interaction targets reconcile against the retained identities.
The shared frame emits the corresponding mark chrome, so Canvas2D, WebGPU, GPUI, and native executors
receive the same presentation without host overlays. Opt-in numeric value labels are placed in
the shared frame with deterministic collision rejection and per-pane emission/work ceilings; executors
receive ordinary ordered text primitives. Sparse, bounded custom row labels live in the dataset
store and participate in the same validated replacement/upsert/retention transaction as X/Y data;
tooltip and accessibility snapshots expose their text without replacing raw numeric values. General
chart persistence uses schema V2 for pane domains, axes, datasets, labels, series bindings, and chart
options while financial-only exports remain V1-compatible; restore rehydrates browser general-series
handles without persisting transient hover, selection, or keyboard focus.

`ChartOptionsStore` keeps typed options canonical for engine and frame reads and retains the raw JSON
object only for boundary-compatible deep merges and serialization. An option patch is merged and
validated once at mutation time; frame construction borrows the typed value without cloning or
deserializing JSON.

Style reset is also owned at this shared boundary. `ChartEngine::reset_style_to_defaults()` restores
canonical chart and live-series presentation in place, including semantic unset/follow values, while
preserving data, pane/scale topology, drawings, indicator bindings, price formatting, and all
time/price view state. Price-scale mode, ranges, margins and layout constraints are not style reset.
Advanced-series semantic geometry and footprint aggregation/representation likewise survive while
their colors, strokes, fills and other visual styling return to Aeris defaults. The browser passes
its selected light/dark theme through the WASM boundary because theme selection is package state.

`aeris_charts_core` must not depend on a window system, browser, GPU, or host application.

### `aeris_charts_indicators`

Pure technical-indicator calculations over numeric slices. Warm-up gaps are explicit. Alongside clean full-recomputation functions, it owns the explicit per-formula rolling state used for append, current-bar replacement, and rebuild-from-index. Bounded-window formulas retain no source-length state; recursive formulas retain tail state and one checkpoint per 1,024 source rows, then recompute from the nearest prior checkpoint after a historical correction. Sparse checkpoint vectors are copy-on-write so hosts can transactionally clone recursive state without deep-copying retained history during ordinary tail work. Host-neutral indexed EMA, ATR, session-VWAP, RSI, MACD, and Stochastic states accept callback-provided optional samples so non-chart Rust hosts can lazily convert only the canonical rows replayed for a dirty suffix. `None` is a hard reset; recursive replay may begin at an earlier sparse checkpoint while writers receive only the requested suffix. Stochastic additionally retains only bounded tail `%K` windows needed for `%D` tail replacement; its windowed high/low scan remains bounded by the configured `%K` period rather than source-history length. Derived values use short-lived transfer buffers that move into or update the engine's canonical output series and are capped after partial repairs. This crate does not know about charts, panes, rendering, WebAssembly, or GPUI.

The `klinechart` module ports KLineChart v10.0.3's 27 built-in indicators (MA, EMA, SMA, BBI, VOL, MACD, BOLL, KDJ, RSI, BIAS, BRAR, CCI, CR, DMA, DMI, EMV, MTM, OBV, PVT, PSY, ROC, SAR, TRIX, VR, WR, AO, AVP) with the conventions of mainland-China and Hong Kong charting software: a doubled MACD histogram, SMA-seeded EMAs, the weighted `SMA(X, N, M)`, and KDJ seeded at 50. Each port repeats KLineChart's arithmetic in the same order, so `tests/klinechart_parity.rs` compares every value bit for bit against KLineChart's own `calc` output (`tools/klinechart_parity` regenerates the fixture). `klinechart::Indicator` is the bindable form: template plus `calcParams`, output keys and titles, figure kinds, default placement, value format, volume needs, and the parameter-only warm-up row of every output. Its runtime recomputes from the first row on each update, as KLineChart does; outputs are causal, so only the requested suffix is emitted, and rows left unset after warm-up become NaN whitespace. The formulas and their tests are Apache-2.0 KLineChart derivatives, credited in the module documentation.

Visible-range volume profiles use a pure two-pass OHLCV bin calculation in this crate: uniform high/low overlap, bullish/bearish volume split by candle direction, deterministic point of control and contiguous value area, `O(visible bars + rows)` work and at most 512 rows. It does not claim tick-at-price accuracy.

### `aeris_charts_engine`

The headless owner of chart behavior and mutable chart state. It owns series, panes, scales, workspace layout, drawings, hit testing, interaction models, indicator bindings, price lines, and frame construction.

Volume-profile indicators bind an OHLC price series to a separate scalar volume series by exact timestamp. The engine owns at most 16 distribution handles, their options and bounded bin caches in native primitive state. Before frame construction (or an explicit snapshot read), it refreshes only profiles whose source generations, visible source-row interval, bin parameters or minimum price move changed. Removing either dependency removes the handle. Shared frame geometry stacks bullish and bearish volume within each row, anchors every row flush to the source pane's right edge, uses stronger row colors for the value area, and draws only the solid POC marker without extending autoscale; every executor consumes those same ordered primitives. These price distributions have no synthetic time-series output and are runtime-only, outside V1 scalar-indicator workspace persistence. Hosts recreate them after restoring data.

Each pane owns one unified price-scale collection: reserved `left`, `right`, and overlay (`""`)
scales plus at most sixteen host-created named scales. Named IDs are case-sensitive and pane-local;
each visible side scale retains its own options, range, formatter source, autoscale, inversion,
dense plot-outward order, measured width, and gesture state. Layout reserves the sum of visible
strips on each side while keeping every pane and the shared time scale aligned. Axis ticks,
last-value and crosshair labels, primitives, coordinates, and gestures resolve through the exact
owning scale. Width negotiation measures every axis-side row in a live-value cluster, including the
scaled countdown row; the pane-side title chip is fitted to the available pane instead of inflating
the strip. The horizontal grid uses only the innermost visible populated scale, preferring the right
side when equal orders meet. Hidden and empty named scales retain state without consuming layout or
receiving labels and input.

Axis chrome is engine-owned and compact: axis-attached text resolves to 11/12 of `layout.fontSize`
(11 CSS px at the 12 px default) with the configured family, countdown text to 10/12 of layout
(10 px), scaling proportionally with larger fonts. The price strip keeps a stable 1 px border slot,
3 px tick, and 4 px padding on each side while the visible border inside that slot uses the canonical
design-system width; the time strip likewise keeps its existing border slot, text, tick, and vertical padding,
tick, and vertical padding, snapped to an even CSS-pixel height (22 px by default). Price tags are
axis text plus 2 px padding above and below (15 px), while the crosshair Y-axis tag alone adds 2 px
per side (19 px); countdown rows are countdown text plus 2 px padding per side (14 px), and time tags
fit the strip height with 6 px horizontal padding per side. Axis-attached price, time, drawing,
alert, and live-value chips share a 1 CSS-pixel corner radius. Tick
density, collision spacing, drag bounds, and crosshair placement derive from the same metrics, and
hosts measure axis strings at the axis size and countdown strings at the countdown size with matching
weight. Font, DPR, formatter, and minimum-dimension changes invalidate measurements, retained labels,
and layout together.

Pane scale geometry is pane-local. Every scale a pane owns is laid out against that pane's own slot
height and carries the pane's top edge as its single explicit transform into chart-content space, so
autoscale, margins, internal height, tick marks, hit testing, and axis gestures resolve inside the
owning pane alone and never against the stacked content height. Resizing one pane therefore cannot
move another pane's range or coordinates. The pane divider is structural rather than plot chrome: its
resting line, hover band, hit test, and drag geometry all describe the same boundary spanning the
full chart width, including every visible left and right price-scale strip. Tick labels reserve their full line-box height plus clearance at internal pane edges even when `entireTextOnly` is disabled; an off-edge label is omitted rather than allowed to paint into its neighbor.

Series pane/scale rebinding is one validated engine mutation. An unknown destination leaves pane,
scale, data, type, style, visibility, streaming state, and handle identity unchanged. Percentage
and indexed geometry uses each series' own first visible value by default; a chart-level comparison
anchor may replace that base for every overlay without copying rows. The same anchor resolution
feeds the bounded engine-owned comparison legend snapshot, so browser and native hosts present
per-symbol values from one canonical time identity.

Hosts send input and data to the engine. The engine returns query results and a prepared `ChartFrame`. Browser and GPUI adapters normalize native events into CSS-logical `PointerSample`/`WheelSample` values and feed the same fixed-capacity `GestureResolver`. The resolver owns pointer membership, the 5 px Manhattan drag threshold, explicit gesture state, fixed starting pinch centroid/distance, cumulative pinch scale, primary-touch continuation/termination, and cancellation; it retains at most two pointers and performs no move-sample allocation. Pinch moves zoom only around the starting centroid and cannot begin after a one-finger move or long press. Hosts still own platform capture, cursor application, event-default policy, and frame/timer scheduling. Normalization includes pointer cadence: browsers already coalesce pointer motion to display frames, and a native host must do the same for brush capture — Wayland delivers per-HID-report motion (~1000 Hz, often one axis per event), and feeding every sample to `brush_create_add` records that axis-alternating staircase as stroke knots. The GPUI host therefore retains only the newest brush sample per painted frame and flushes it once during prepaint (and on pointer-up before commit), so stroke knots sample the drag trajectory at display cadence on every host. Horizontal kinetic scroll follows the reference domain exactly: drag samples are the time scale's logical `rightOffset`, while the reference 0.2/7 px-per-ms speed limits and 15 px minimum move are divided by the bar spacing captured when the drag starts. The resulting coast is therefore zoom-invariant instead of being tuned in raw pointer pixels. Browser keyboard Left/Right pan is velocity-owned rather than destination-owned: key-down gives an immediate bounded velocity kick, the engine adds further low-friction kicks at a fixed cadence while the key remains held, and browser key-repeat is ignored except when it changes the requested Ctrl/Shift speed. Key-up cancels the kinetic state immediately. Ctrl/Shift retain the existing 10x strength relationship to plain arrows. Zoom, scroll, kinetic motion, snapping, selection, drawing/trading preview semantics, and rollback belong here.

Secondary clicks use the engine-owned Chart Context query. It resolves chart-space coordinates,
pane, time, logical index, hit series, and price on that series' exact scale (or the pane's canonical
default scale on empty space) without running primary-click selection or activation. Browser and
native hosts may use the payload to build menus, clipboard actions, or order UI, but those side
effects remain outside the engine.

Chart-wide value snapshots are assembled once by the engine from canonical plots, current series kind, pane/scale placement, and each series formatter. Exact logical mode retains every live series and leaves gaps or whitespace null; latest mode independently selects each series' own last non-whitespace logical index and time. The snapshot also carries the previous same-series non-whitespace close/value. WASM only serializes this bounded result, while the TypeScript package maps opaque series IDs to existing handles. Crosshair compatibility `series_data` is filtered from the same snapshot, and crosshair leave exposes a rich latest snapshot while retaining an empty compatibility map.

Sparse host fundamentals use the existing general-series contract: temporal release rows remain sparse, `GeneralInterpolation::Step` lowers to `LineType::WithSteps` (step-after at the release timestamp), and `GeneralSeriesKind::Column` provides the independent-pane histogram form. Row labels are host-supplied as-of release text; the engine neither fetches nor interprets fundamentals, and no value is projected before its release row. The same timestamp/label rows are retained through history gaps and resampling, so replay can mask future releases without a second fundamental-data model.

All interaction hit tests use an engine `HitProfile`. Mouse and pen retain precision tolerances; touch expands semantic anchors and actionable trading controls to an effective 44 CSS-pixel target without changing visual geometry. Cancellation from pointer cancellation/capture loss, host focus or visibility loss, resize, backend loss, or disposal closes scale/scroll sessions without inertia and restores drawing/trading previews rather than committing them.

Built-in frame geometry and series hit testing share one viewport-density query. Resolvable spacing uses the raw rows unchanged. Below one physical pixel per row, the query chooses the deepest summary level whose group fits the average pixel density, uses aligned summary nodes for pixel-bucket interiors, and refines partial boundaries through lower levels or raw rows. The existing per-kind conflation then preserves chronological line endpoints and close extrema, candle first-open/high/low/last-close semantics, and histogram greatest absolute value. The resulting ordered `ChartFrame` remains the only backend contract. Crosshair and trading data lookup remain exact raw/cached canonical queries rather than LOD approximations. Heikin Ashi candlesticks use a generation-keyed engine presentation cache over canonical OHLC: frame geometry, autoscale, and candle chrome may consume the derived values, while `series_data`, crosshair, and trading paths continue to expose raw OHLC.

The official advanced-series examples are engine-owned feature series, not browser drawing callbacks. Each retains its complete validated payload beside an OHLC-shaped canonical projection used by the shared time/price-scale and query machinery. Grouped bars, heatmap, HLC area, pretty histogram, background shade, stacked area/bars, and whisker boxes construct backend-neutral primitives in the same ordered series layer as built-in geometry. Their official defaults, visible-range rules, pixel snapping, autoscale semantics, and source-data lifecycle are therefore identical in browser and native hosts. Brushable Area is deliberately not an advanced-series data type: it is an ordinary built-in Area series plus transient engine-owned range styling, so data ingestion, retention, LOD, hit testing, price-scale ownership, and all ordinary Area APIs remain on the canonical Area path. The legacy browser input name `brushable_area` is only a compatibility alias and normalizes to `area` immediately.

Professional footprint / numbers-bar data has a chart-level tick-truth owner described in
`Footprint.md`. `ChartEngine::add_trade_stream` retains one bounded keyed canonical microsecond tape;
footprints, CVD, delta histograms, and bounded large-trade bubble markers hold dependent handles, not
provider-event copies. The stream derives integer tick-grid levels, bid/ask/unknown/total volume, POC,
final/session delta, delta percentage, running Max/Min Delta, and diagonal stacked imbalances. CVD
supports session, continuous, and anchored resets, and every dependent carries the stream revision
through tip, correction, and retention updates. Stream telemetry attributes retained tape capacity
and dependent rebuild work.
Live tip events update only the active derived bar; a late-event or provider-correction batch merges
atomically into the final canonical tape, validates its final session/bar projection, and reconstructs
exactly once.
Each derived bar also carries an engine-owned logical index plus its full-resolution open and close
microsecond times. `FootprintAggregator::bar_sequence` exposes those bounds without collapsing them
to whole-second labels, so several non-time bars in one second and long gaps remain distinct.
Chart-integrated trade-count, volume, and range footprint projections use chart-local row keys plus
an engine-owned sequence sidecar; axis labels, crosshair lookup, and visible ranges resolve against
the sidecar's full-resolution open times, never synthetic UTC timestamps. `BarSequenceMapping`
matches ordered full-resolution bounds and rebases logical anchors across prepend/rebuild operations
without collapsing duplicate second labels. Non-time tip updates now replace only the affected
suffix (falling back to a full projection when retention can shift the prefix), and derived delta
studies and trade-bubble markers use the same logical row keys. Bubble aggregation windows compare
the original microsecond trade times; value snapshots and series queries resolve their time labels
through the same sidecar. Trading executions, host events, and round-trip geometry resolve their
timestamp anchors through the same index helper. The broader B5 performance exit remains open for replay and
release benchmarks. The sidecar is retired when the last live non-time footprint or dependent leaves the chart,
preventing stale sequence labels from affecting later time series.
The configured tick size owns the series min-move/formatter and the shared autoscale, frame, and hit
paths use complete half-tick outer cell bounds on the series' ordinary pane-local price scale.
Footprint bars ultimately emit the same ordered `ChartFrame` as every other series, and no backend
may infer order flow from OHLC or recalculate footprint math.

The upstream heatmap-around-line and background-shade examples are compositions: the specialized engine series is ordered beneath an ordinary line series rather than duplicating that base-series geometry. Heatmap `cell_shader` callbacks are the one styling boundary in this group; the browser evaluates the callback while normalizing input, and Rust retains the resolved color with each bounded cell so every renderer executes the same prepared frame.

Trading is a first-party engine domain, not a drawing, series, primitive, or plugin. Each chart owns host-supplied typed position, order, group, and execution identities; broker relationships and instrument metadata; semantic trading style; dedicated hit state; and a bounded intent queue. The host remains authoritative for broker state. Pointer movement changes only a chart-local snapped preview. Release emits one broker-neutral typed intent directly, and the chart offers no inline confirmation step of its own: a host that gates modifications runs its own confirmation around the intent before answering it, which keeps that policy where the host's instant-order-placement setting already lives. The chart also APPLIES the change as it emits — a closed order or position leaves the chart, a dragged line stays where it was dropped — and keeps only a rollback, so rejecting the intent restores the object exactly as it was. Nothing is parked in a pending tint waiting on an answer, because closing means the object is gone and moving means it has moved. Confirmed objects change only through a subsequent host snapshot or incremental update. Accepted previews remain visibly dotted and pending until that authoritative update arrives; rejected or discarded previews disappear without mutating the confirmed object. Trading state, previews, intents, and executions are runtime-only and never enter drawing persistence.

The trading contract is explicitly multi-account and host-authoritative: every runtime object may carry a bounded validated account ID, and one engine-owned visible-account filter gates both rendering and hit-testing without removing hidden objects from the snapshot. Host annotations on positions and orders are capped, validated atomically, and rendered as shared chip geometry with deterministic overflow; their tone and tooltip are presentation metadata only. Trailing and break-even trigger lines use host-supplied prices, while price-bearing intents also carry an exact integer tick index when instrument tick metadata permits it. Execution markers accept circle/arrow/triangle and quantity sizing, and bounded host round trips add outcome-colored connectors and labels. Host event markers and risk windows use a separate transient overlay layer with bounded IDs, deterministic pixel-column LOD collapse, and dedicated host hit results; they never enter drawings, undo history, or persistence. Linked charts use semantic crosshair and visible-time-range events carrying source and monotonic revision; external application resolves values against local data without re-emitting, preventing echo loops.

Price alerts use the same host-authoritative boundary. The engine retains at most 4,096 typed alert-line indicators and paints them through the canonical pane/axis frame; it does not evaluate conditions, persist alerts, enforce account limits, run background timers, or deliver notifications. An alert line's price tag always shows its formatted price, exactly like every other axis tag; an optional host label remains metadata and never replaces that price. What names the line visually is a badge chip attached to the tag's pane-facing edge, carrying a bell drawn from prims rather than a font glyph, rounded on its outer edge and square against the tag so the pair reads as one control. Active alert chrome derives from the theme-aware muted-text token rather than the primary blue accent; triggered and expired states retain warning and darker-neutral colors. The crosshair price label exposes one engine-rendered multipurpose action chip on its primary price scale: an attached button, rounded on its outer edge and square against the tag with no radius on the tag side, carrying the original circular-plus SVG from `packages/charts/src/assets/icons/add.svg`. Its alpha masks for integer sizes 1 through 96 are generated by the pinned browser rasterizer (`node examples/web_demo/build_crosshair_icon.mjs`, with `--check` for verification) and embedded in `aeris_charts_render` as a bounded run-length asset. Each engine retains only the current size as immutable RGBA pixels; an axis frame shares those pixels and the shared converter emits an integer-aligned image primitive for every backend, including workers. Font/DPR changes select the matching mask; device recovery reuses the retained image. No runtime SVG parser or renderer-specific icon shape is involved. The chip stays visible whenever the crosshair is; hovering it lifts the fill a step with no blue fill. Activating that exact hit zone emits a bounded chart-level action request carrying pane, scale, and price; the browser package forwards it to host subscribers so the host can offer alert, limit-order, horizontal-line, or other context-appropriate actions. The request does not choose an action or carry alert defaults. Alert metadata represents the regular-price `crossing`, directional crossing, greater/less operators and the `only_once`/`every_time` frequencies, plus interval-dependent per-bar, bar-close, and per-minute frequencies. These values are display/configuration metadata only until the host returns an authoritative line snapshot or update. Alert lines and pending action requests are runtime-only and never enter chart persistence.

Official primitives with chart semantics are likewise retained by the engine. Series primitives follow the source across panes and own their bounded data, hit state, autoscale contribution, and pane/axis views; pane-only primitives retain a stable `PaneId`. Delta Tooltip is a non-candlestick interaction: the engine rejects attachment to candlestick series and removes an attached Delta Tooltip if a convertible built-in series later becomes candlesticks, while the ordinary Tooltip remains available for candle inspection. The ordinary Tooltip snapshot is a structured bar inspector rather than a one-value DOM guess: Rust resolves the exact hovered source row and returns its retained Open/High/Low/Close for candlestick, bar, area, line, baseline, histogram, and other ordinary presentations. Scalar host rows already normalize the same value through all four canonical columns, so scalar area/line data has coherent OHLC while an area/line presentation over retained OHLC can inspect the complete bar even though it paints Close. Optional volume is explicitly host-associated through a timestamp-aligned `volume_series`; the engine never guesses which independent histogram means volume. Tooltip chrome reads the chart's resolved surface, foreground, muted text, border, and font at the browser boundary so light/dark theme changes cannot drift from the chart. Brushable Area composes an ordinary Area series with Delta Tooltip and transient `SeriesEntry.area_brush` presentation state. While that helper is attached, primary mouse/pen pane-drag belongs to the comparison gesture instead of starting a competing canvas pan; price/time-axis drags and manual scale unlocks remain the ordinary Area behavior, and the helper never globally changes `handle_scroll`, `handle_scale`, or chart crosshair options. One-finger touch can still pan normally, while the Delta Tooltip's existing two-point touch interaction remains available. Delta Tooltip owns its own comparison guides, and clearing/detaching the interaction simply drops the transient brush state so the untouched Area renderer is restored. A browser may decode an image or evaluate a user-supplied color/format callback at the platform boundary, but it sends the bounded result back to Rust. The engine stores image watermarks as RGBA8 `RasterImage` values and emits one shared `Image` primitive; Canvas2D, WebGPU, GPUI, and native executors only upload/cache and paint that prepared image.

An indicator binding keeps its public definition, compact private runtime, and ordinary canonical output series separate. Sparse runtime checkpoints are tied to source row positions and to the source and optional volume-series generations. A tail mutation advances only bindings that depend on that source and installs only changed output rows; a historical mutation resumes from the nearest valid checkpoint and replaces the affected output suffix, while truncation or complete replacement performs a clean rebuild. The five-output EMA ribbon is one binding with one independently checkpointed recursive EMA state per configured period. Its atomic period update retains all output identities and presentation, rebuilds the five value columns once, and propagates the resulting changes through dependent indicators. Removed source/output series drop the binding and its runtime state together. VWAP bands use the same sparse checkpoint boundary for weighted basis, population deviation, and percentage bands, with an engine-owned session/weekly/monthly reset key.

KLineChart bindings (`IndicatorKind::KLineChart`, persisted as `{"kind": "klinechart", "indicator": ...}`) reuse the same binding, alias, persistence, and schema paths. Their presentation is engine-owned: 1px lines in KLineChart's five-color palette, `VOL`/`MACD`/`AO` columns as histograms and `SAR` as marker-only dots with per-row colors recomputed for the changed suffix, price templates on the source pane and every other template in its own oscillator pane, and no price-axis value labels. KLineChart outlines rising MACD and AO columns; Aeris histograms have no outline style, so those columns are filled at a lighter alpha instead. `VOL`, `OBV`, `PVT`, `EMV`, and `VR` require a distinct scalar volume series (missing timestamps use KLineChart's own default, 1 for PVT and 0 otherwise); `AVP` reads turnover as the value of a scalar source series and volume from its volume series.

Indicator output metadata is additive and binding-complete: the first output's monotonic series identity is the stable binding identity; every output reports the full structured parameters, source and optional VWAP/VWMA volume source, stable output name, index, and count. Native hosts may also enumerate one typed definition per live binding in deterministic creation/dependency order and recreate it through the generic `IndicatorKind` entry point, remapping source, volume-source, and ordered output identities as they go. This definition snapshot excludes runtime calculation state. Scalar series can also carry renderer-neutral semantic presentation owned by the engine: fixed threshold regions lower into the canonical translucent oscillator channel plus dotted boundary lines, and momentum histograms reuse the canonical four-state market palette while treating whitespace as a reset. Hosts declare those semantics but never receive or retain render primitives or palette logic. The host groups and renders legends from binding metadata; indicator values still come through the ordinary chart value snapshot. Study bindings additionally carry a typed scalar input (`open`, `high`, `low`, `close`, `hl2`, `hlc3`, `ohlc4`, or `hlcc4`) selected at the engine boundary; aggregate inputs are materialized only for the bounded rebuild and never become duplicate canonical series storage. Multi-input bindings align VWAP and VWMA volume inputs by exact timestamp rather than row position, using the documented unit-weight fallback for missing timestamps. Each output also exposes a compact engine-owned style snapshot (visibility, line, marker, area, and directional colors) and accepts an atomic validated style replacement, so per-output styling survives host persistence without kind-specific reconstruction. WASM hosts can query the same bounded parameter/output schema by indicator kind, so property panels do not duplicate engine definitions.

Drawing anchors, kinds, styles, pane association, stable z-order, and metadata remain the only authoritative committed drawing state (temporary hover/selection/drag/edit promotion never rewrites it). Built-in tool semantics are described by one compile-time engine catalog: stable wire/name identity, placement class, point-count rule, handle policy, movement-axis restriction, straighten behavior, semantic bounds extent, defaults, and platform-edit requests. Trend-line labels resolve their 3×3 left/center/right and top/middle/bottom positions against the actual segment rather than its bounding box; an inline middle label splits the shared stroke around its measured text extent so no backend paints through the glyphs. The catalog includes Long Position and Short Position as single-click preset tools whose committed semantic points are entry, target/width, and stop. Target and stop are normalized to opposite sides of entry, stop shares the origin edge, and editing uses four dedicated controls (target, entry/origin, width, stop) rather than generic anchor behavior. Their one-click presets open asymmetrically at 2:1 reward/risk, with fill-only entry→target profit and entry→stop loss zones, a thin neutral-gray center entry line, target/stop boundary labels only, and filled neutral/green/red owning-scale Y-axis price tags; no PnL or risk/reward summary block is painted inside the position. Position run progress is a derived three-state model bounded by the position's horizontal lifetime. Pending positions emit no progress until the first post-placement candle reaches or crosses entry; that candle becomes the progress origin at the exact entry price. While filled and active, the endpoint is the latest in-box candle's Close/current value, so the active reward/risk side follows current position state rather than a historical or current wick extreme. The first candle after fill to touch target or stop completes the run: target-first freezes at the exact target price and stop-first at the exact stop price; a same-candle target+stop touch is conservatively stop-first because OHLC cannot determine intrabar ordering. The stronger opacity covers only the traveled x/y rectangle from first-fill entry to current/terminal price, never the whole TP/SL zone or untouched empty area, and neither overlay nor connector projects beyond the position rectangle. Reward and risk use the same explicit progress-emphasis opacity, stronger than the untouched base-zone opacity, so an SL run is emphasized exactly like a TP run rather than depending on subtle repeated alpha compositing. The connector remains dashed neutral gray. LOD extrema summaries plus prefix binary search keep entry-cross and first-boundary discovery bounded for long-lived positions; an OHLC gap across entry is deterministically treated as a cross and drawn at the semantic entry level because no exact intrabar path is available. The run overlay lowers through pane chrome so series updates do not rebuild retained drawing geometry, and the boundary labels are emitted after it so the dashed trend never paints over their text. The catalog is not a runtime plugin registry; adding a built-in tool extends this deterministic engine-owned definition rather than teaching each host or renderer how the tool behaves. One chart-local `DrawingController` owns the armed tool/template plus pending anchored placement or captured freehand state. Browser and GPUI hosts forward generic press/move/release/activation/finish/cancel actions and retain only platform duties such as pointer capture, event coalescing, editor surfaces, and repaint scheduling; hosts do not branch on concrete drawing kinds to decide creation behavior.

A chart-local derived runtime maps the existing monotonic `DrawingId` values to conservative logical/price bounds, coordinate-keyed media-space anchor geometry, and pane-local z-ordered candidate lists. Candidate queries first reject drawings in semantic space, then test cached conservative screen bounds; only viewport or pointer candidates rebuild coordinate geometry and reach canonical primitive emission or precise hit testing. Tool anchors resolve into one backend-neutral drawing-geometry vocabulary before either body hit-testing or `Prim` emission, so the interactive body and the rendered body share the same segment/ray/full-line/rectangle/polyline geometry and terminal decorations. Frame construction alone lowers that resolved geometry into the shared render IR; Canvas2D, WebGPU, GPUI, and native/headless executors never receive a drawing kind and cannot fork tool semantics. Full-span horizontal lines, full-height vertical lines, and half-infinite horizontal rays retain explicit unbounded dimensions rather than fake finite extents. The multi-click Path stores two to 100,000 vertices, emits one straight polyline with an open terminal chevron, exposes every vertex for editing, treats the two arrowhead wings as body-movement targets, and commits one history command only when double-click or Enter finishes it; Backspace removes the latest pending vertex and Escape discards the pending path. Brush remains a press-drag freehand placement class: its bounds are computed once on semantic mutation, padded for curved interpolation, and remain conservatively unbounded during an active capture before one exact pointer-up rebuild. Capture decimates pointer samples by distance and leaves the frame untouched when a sample is rejected, so rapid input invalidates the drawings layer only per accepted point. Runtime bounds, resolved geometry, controller state, counters, and index entries are never serialized.

When a historical insertion, removal, series replacement, or retention trim changes merged logical indices, the engine rebases every drawing-semantic logical snapshot through the data layer's common-timestamp mapping: committed anchors, pending creation anchors and preview, active brush points, active drag start and current snapshots, and both drawing-history stacks. Non-time footprint rebuilds use the bar sequence's full-resolution open/close microsecond identity mapping instead of row keys or truncated seconds, including fractional anchors between bars. Common timestamps map exactly, fractional positions interpolate between them, positions outside their extent extrapolate with slope one, and a replacement with no common timestamp is identity. The transient mapping retains only slope-change breakpoints. Pixel drag/brush baselines refresh immediately for input continuity and once more after the next frame settles layout and autoscale. This maintenance mutation creates no drawing-history command. Non-time drawings persist an optional bounded anchor-time sidecar (open/close microseconds) alongside their logical/price anchors; legacy documents omit it and retain their existing behavior, while restored sidecars resolve when the host installs the matching sequence.

Each chart also owns a bounded runtime-only drawing history of the last 100 committed semantic
create, delete, anchor, style, and clear operations. Pointer-move samples mutate the active drag
snapshot without adding commands; pointer-up records one start-to-end update. Undo/redo rebuilds
only the affected drawing runtime state, a new mutation clears the redo branch, and persistence
never contains either history stack.

B2 extends that owner boundary with a versioned typed drawing contract. `drawing_contract.rs`
defines bounded property descriptors, interval visibility, line caps, magnet modes, labels,
levels, templates, clipboard payloads, and revisioned sync payloads. The live `Drawing` remains
the sole source of truth; its common snapshot and discriminated kind-option projection are
computed views, so a property panel cannot create a second state model. Patches validate all
bounded contract fields on a clone before installation and record one undo entry per semantic
change. The shared resolved-geometry path applies line extensions and is consumed by both frame
emission and hit testing. Hidden or interval-ineligible drawings remain in persistence and the
object tree but are excluded from rendering and hit testing; locked drawings remain selectable
but cannot be edited. Selection, clone/copy/paste, z-order, group operations, bulk removal, and
sync are chart-owned and bounded, with sync IDs/revisions preventing stale or echoed updates.
Named templates are validated data rather than host-side drawing copies. V1/V2 persistence keeps
these fields optional for lossless migration of existing layouts, while browser/WASM exposes the
same schema, template, object-tree, and payload operations as the native engine.

Versioned persistence is an engine-owned semantic DTO boundary, never serialization of live engine
structs. Financial-only charts continue to export V1 with ordered pane topology and built-in
drawings only. V2 adds pane horizontal domains, general axes, typed general datasets (including
row identities and labels), general series bindings, and chart options. V1 restoration and its
fixtures remain unchanged; V2 validates on a detached engine before committing general state and
rebuilds browser series handles from an engine catalog. Pane persistence identity is
separate from live `PaneId`: import preserves document references while issuing fresh monotonic live
IDs, so pre-import pane and price-scale handles become stale. The complete document is size-bounded,
parsed, and validated before one transactional install; drawing bounds, candidates, and geometry are
rebuilt once from anchors. Financial market history and series/indicator definitions, extensions,
callbacks, and every runtime cache remain host-owned or derived. Unknown versions and semantic kinds
fail structurally without mutation.

Named price-scale descriptors and series bindings remain host-owned configuration and are not added
to chart-state V1. A restoring host recreates pane-local named scales before reinstalling or rebinding
its series.

The browser split grid is the active-chart router. Its stable workspace cell ID decides which
independent chart receives a global drawing tool, document shortcut, or view reset; the receiving
chart retains all drawing selection, hit testing, mutation semantics, and history. An armed toolbar
tool migrates between active cells, but drawing selection never does. The grid's host-facing
workspace state is a small composition of the validated generic split layout, optional active/stable
cell identity, and one unchanged chart persistence V1 document per cell. Optional instrument identities are opaque
host strings. The host stores this composition and restores market history, subscriptions, and
host-owned series/indicator definitions after the grid restores each Aeris chart document.

### `aeris_charts_render`

Backend-neutral drawing primitives, colors, geometry, bar-width rules, and the ordered `DrawList`. This is the contract shared by every renderer. Pixel snapping, primitive ordering, clipping intent, and geometry must be decided before backend execution whenever possible. Curved polylines expand their Catmull-Rom spline adaptively by device-px interval length — intervals already a few pixels long render as their chord (dense freehand brush samples), long sparse intervals keep up to 16 segments — and round joins are emitted only where a turn opens a visible wedge, so tessellation volume stays proportional to what the pixels can show on every backend.

### `aeris_charts_render_gpui`

The native GPUI executor. It converts the prepared primitive stream into GPUI scene operations and owns GPUI-specific text, image caches, geometry conversion, backend metrics, and fixtures. It must not fork chart behavior or recalculate engine geometry. The repository's interactive Linux probes enable GPUI's Wayland and X11 platforms; macOS and Windows continue through GPUI's native platform selection. CI compiles and tests the GPUI backend on all three operating systems.

GPUI's path pass cannot rely on MSAA — its sample count is picked from the surface and can fall back to 1x on Linux — so stroke, disc, and ring meshes carry a per-vertex Loop-Blinn signed-distance encoding in the path shader's `st` coordinates. Polyline strokes keep `s` constant and encode signed device-pixel distance in `t`, which is compatible with GPUI's Windows solid-triangle branch; their one-pixel coverage transition is centered on the nominal edge so integrated coverage remains the requested width. Ring strokes use the same constant-s, centered coverage encoding as polylines, preventing Windows from treating the antialiasing fringe as solid stroke. Filled discs retain their shape-specific exterior encoding. A mesh larger than a bounded chunk is split into multiple GPUI paths so one stroke cannot overflow GPUI's fixed path instance buffer and trigger its grow-and-redraw retry loop; the mesh is a triangle soup, so coverage and paint order are unchanged.

An interactive GPUI host requests another animation frame only for active engine animation or an explicit finite measurement run. Idle charts stop scheduling frames. The executor retains its lowered `ScenePlan`; a host presentation that does not change the canonical engine frame can repaint that plan without lowering every primitive again.

The interactive `gpui_probe` example and `examples/web_demo` keep demo controls in a separate,
scrolling inspector so adding control groups does not reduce chart height. Section navigation,
inspector visibility, and responsive shell layout belong to these example hosts. GPUI uses its
native scroll and keyboard-focus facilities; the browser uses semantic headings, labeled controls,
and a dismissible compact inspector. These shells retain the existing engine/API action paths;
the finite GPUI probe and browser runtime fixtures keep their dedicated measurement layouts.

### `aeris_charts_render_wgpu`

The WebGPU executor. It owns quad, triangle, textured-label, atlas, blend, multisample, scissor, and GPU timing resources. GPU objects are reused across frames and rebuilt only when their actual invalidation inputs change.

### `aeris_charts_wasm`

The browser boundary. It exposes the engine through `wasm-bindgen`, decodes typed input, selects WebGPU or Canvas2D policy, executes browser frames, handles shared ring input, text measurement, workspace APIs, and browser telemetry.

The browser boundary translates data and platform events and serializes engine-owned value snapshots. It must not become a second chart engine.

### `aeris_charts_native`

The headless native executor and verification support. It uses tiny-skia for deterministic raster output, golden comparisons, examples, and release performance gates. Text follows the host system UI sans-serif face. It is evidence infrastructure, not a competing product model.

## TypeScript package

`packages/charts` publishes the `@aeristerminal/aeris-charts` browser API through GitHub Packages. It owns WebAssembly initialization, TypeScript chart handles, DOM canvas lifecycle, resize observation, Pointer Event translation for mouse/pen, cancellable Touch Event translation for direction-dependent page-scroll arbitration, platform capture/default policy, host callbacks, themes, shortcuts, offscreen support, and grid helpers. The root entry remains framework-neutral. The optional `@aeristerminal/aeris-charts/react` entry is a thin lifecycle/reconciliation adapter over those same public chart handles: React mounts one ordinary chart, applies option/data changes to retained engine objects, and disposes through `chart.remove()`; it owns no scale, geometry, hit-test, persistence, or rendering semantics. Its module performs no DOM work at import time, so SSR can import it without constructing a browser chart. Drawing-tool arming and pointer events cross the browser boundary through the generic engine drawing controller; the package does not classify a tool as single-point, multi-point, sequence, or freehand, nor duplicate tool-specific placement state. Touch Events normalize into the same engine resolver rather than a parallel gesture state machine; static `touch-action` stays `auto`, and the host applies the reference-informed vertical-priority direction rule after the shared slop. Wheel samples retain floating-point deltas; informed by measurements from the pinned public reference fixture, `wheel_behavior: "auto"` independently maps vertical deltas to time zoom and horizontal deltas to time pan on every chart surface without modifier routing. Explicit `"pan"`/`"zoom"` modes remain host overrides, including price-axis wheel zoom only in explicit zoom mode.

Browser accessibility is chart-owned, enabled by default, and represented by one singleton controller exposed through `chart.accessibility()`. `enable_accessibility(chart, options)` configures that same controller for compatibility. The chart container is a named group; canvases are hidden from assistive technology and each pane has one complete application-style keyboard surface. Default streaming announcements are off, user-driven navigation/actions remain announced, visible data queries are capped at 512 on-demand logical points, and only the active series owns one engine-rendered focus primitive. Accessibility focus/edit state is runtime-only and is never persisted. Pointer interaction updates ordinary chart selection and hover without moving DOM focus into the application surface; keyboard traversal and explicit accessibility API calls own its visible focus. Forced colors, higher contrast, reduced motion, locale, host names, and visible focus are resolved at the host boundary; the shared engine retains exact focus geometry and keyboard drawing mutations use the same drawing history/rollback path as pointer input.

Auto-size keeps `ResizeObserver`'s exact device-pixel path. A resolution media-query watcher plus orientation/fullscreen fallbacks re-run sizing when DPR changes without a CSS-bounds change; resize reprojects semantic state and does not create new object identities. While auto-size is active, manual `resize` calls are ignored. Disabling it disconnects the engine-owned observer and returns authority to manual sizing; re-enabling immediately adopts the current container. Hidden or detached containers retain the last usable size and adopt their new bounds when revealed.

The package also ships `design.css` as the portable host design system. Its complete surface, control, interaction, icon, action, focus, radius, and market palette remains host-owned CSS; host chrome uses the system UI font stack and may use `color-mix` and `oklch`. The published package does not include a webfont. Native CPU text uses the host system UI sans-serif face; scene goldens that contain no text stay machine-independent. Backend-facing roles — surface, axis text, axis and grid border, canonical border width, muted text, separator interaction, focus/primary interaction, market up/down, and the small/default/large radii — have deterministic projections in `crates/aeris_charts_core/style_tokens.json`; colors are opaque sRGB composited over the theme surface while border width and radii are CSS-logical pixels. `aeris_charts_core` owns and compiles that file into the defaults used by every engine and backend. Axis borders and visible pane separators both project the shared border-width token onto the device-pixel grid in the engine; the separator hover target stays independently expanded for interaction. The TypeScript package imports the same source at build time for host theming and workspace divider projection. The core crate therefore remains independently packageable without reaching into a browser-package directory, and the tokens resolve before frame construction rather than through demo or renderer overrides. Canonical engine grid lines are visible and dashed by default. The interactive WebGPU/Canvas2D and GPUI demos intentionally override only grid visibility to off at startup; every other engine-owned option and series presentation starts from its canonical default, while explicit user actions and deterministic test/parity fixtures may apply their own requested state. A `v*` tag matching the package version publishes the verified artifact to GitHub Packages.

The package preserves its complete `snake_case` surface and adds camel-case aliases for the common JavaScript chart/series/scale lifecycle without creating parallel state or handles. Financial and general series use the same chart object and ordered frame. Data crosses into WebAssembly in typed columns or bounded shared-ring layouts rather than per-point object calls on hot paths. Typed update batches transfer their sanitized owned columns to the engine's batch entry point; the browser wrapper never loops through the single-row engine API. The published artifact exports the optimized WASM asset explicitly and the generated glue also resolves that sibling asset by `import.meta.url`; source-tree `pkg/`, crate, benchmark, and demo paths are not runtime dependencies. `examples/web_demo` remains an integration and parity test host, while `examples/all_in_one` contains consumer-facing framework-neutral and React compositions.

`chart.value_snapshot(logical_index?)` crosses WebAssembly once and returns all live series. The package adds live handles to the engine records and derives legacy crosshair `series_data` by retaining only valued entries. Engine-owned feature series expose their scalar scale projection and retain the legacy scalar event shape. Arbitrary custom-series callbacks remain host-owned: exact snapshots are null, while latest snapshots can expose only the last value recorded during a visible frame and are explicitly render-state-dependent. Symbol/exchange metadata, volume association outside VWAP bindings, bar/day change math, session calendars, visibility settings, and legend DOM remain host-owned.

The supported, experimental, internal-but-exposed, and legacy surfaces are classified in
`Public_api.md`. Predictable browser failures use `AerisChartsError` with stable category codes;
clean ingestion retains a null diagnostics fast path. The generated WASM surface and benchmark/test
hooks are internal even when visible to developer tools. A deterministic declaration manifest makes
supported TypeScript surface changes explicit in CI.

`chart.remove()` is the single public browser lifecycle operation. It is idempotent and transitions the retained TypeScript handle to a disposed state after cancelling scheduling, detaching browser resources and extensions, releasing per-chart GPU state, explicitly disposing the Rust object, and calling the generated `free()`. Later operations fail with a stable disposed-state error. Offscreen charts use the same explicit dispose-then-free ordering.

## State and frame ownership

Each chart has one engine owner. Mutations invalidate only the state that changed. A frame is a deterministic snapshot of engine state for a viewport and device scale.

Coordinate-authoritative scale objects advance canonical revisions inside their mutating methods. Browser and GPUI hosts express gestures through `ChartEngine` commands; legacy direct Rust access remains coherent because it cannot bypass the scale-owned revision. `SeriesStore` likewise advances its canonical presentation revision whenever a Rust host takes mutable access, replacing read-side hashing of every style field. Retained coordinate-dependent layers are derived caches stamped with the engine's current coordinate revision. Frame assembly asserts that the grid, visible series, chrome, drawings, and interaction overlay all carry that same revision, so a frame cannot mix transforms.

Frame invalidation is an engine-owned generation graph. Layout, coordinates/autoscale, grid and underlay, each series, drawings (with per-drawing prim/point segments plus a trailing controller-owned creation-preview block), and interaction overlays have independent generations. Coordinate-range changes fan out to coordinate-dependent layers; a value-only current-bar update stays on its source series when autoscale bounds do not change. Ordering-only promotion (hover/selection/drag/edit) reassembles retained series layers and drawing segments without rebuilding geometry; drawing drag rebuilds the drawings layer with fresh segments while reusing the runtime per-entry cache. Public option and series-style mutation are included in the generation inputs, so direct native callers cannot bypass retention accidentally.

Series and indicator selection owns one transient engine snapshot with a single primary command target and at most 64 related output members. Engine-owned indicator bindings expand automatically; hosts may supply the bounded member identities for study groups they author outside the built-in indicator registry. Each member retains at most 128 canonical output timestamps sampled from its own full canonical start-to-end extent only on the unselected-to-selected transition. Selection-time projection determines sparse density, while endpoint-inclusive logical spacing prevents a partial-series selection treatment. Overlay rebuilds resolve every member's identities against its current canonical values and coordinates, place candlestick handles at the current body midpoint, clip offscreen handles without replacement, and discard the snapshot on deselection; LOD geometry, screen coordinates, and persistence never own selection-anchor membership.

Drawing semantic mutations reuse this graph: add/remove/style/anchor changes invalidate the drawing layer and update only the affected derived entry, while selection/hover/drag/edit promotion reassembles retained drawing segments without rebuilding geometry and selection changes additionally invalidate the overlay and axis frame for handles. Drawing selection handles are assembled at the beginning of the overlay, preserving their prior canonical order immediately after drawing bodies and before crosshair/series overlays without rebuilding unrelated drawing geometry. Temporary promotion (dragging/editing → hovered → selected → idle, hover gated by `hoveredSeriesOnTop`) never rewrites saved drawing z-order; deselection, hover leave, cancellation, or removal restores it. A selected price-spanning rectangle also emits primary-colored extent tags and a territory band on its bound price scale; those axis views follow creation, drag, and resize coordinates and disappear on deselection unless the drawing explicitly requests persistent axis views. The text tool is an exception: it emits no anchor discs — selection and hover paint the same focus border box (hover at reduced opacity), empty text paints nothing on the chart, and leaving the host editor without typed text removes the drawing. While typing, the host wrap is borderless with transparent glyphs; the engine keeps painting both the label and the focus border underneath, so edit entry cannot lift the text or shift the outline. Crosshair movement and unchanged-coordinate market-data updates do not invalidate drawing geometry. Pane add/remove/swap/move rebuilds pane membership because pane ownership itself changed; ordinary drawing drag updates one entry, and structural removal repairs the canonical vector's id-to-position map.

The engine-owned crosshair overlay can paint the same configurable hover marker for every visible line, area, baseline, and line-shaped indicator output at the snapped logical index. Markers ship disabled and hosts opt in per series or indicator output; when enabled, marker coordinates, per-series colors, borders, pane ownership, and scale conversion are resolved before the shared frame reaches any backend. Crosshair and drawing magnets share one pixel-space candidate path: candle, bar, and footprint series expose their rendered OHLC fields, while line, area, histogram, baseline, and other scalar projections expose only the close/value they paint, so hidden storage columns cannot attract an anchor. An empty hovered trend line emits a low-opacity, borderless `+ Add text` run at its configured segment-relative slot; the engine owns its measured hit box and exact caret anchor, including the middle-slot stroke gap, while a browser host owns only the transparent content-editable caret overlay. Clicking either this affordance or existing trend text enters inline editing, and leaving an empty trend edit preserves the drawing. Bar-slot highlights and tooltip guides resolve their default tint from the current chart surface, using a light lift on dark surfaces and a dark tint on light surfaces; overlay price-scale text follows the current layout foreground. Explicit host colors remain authoritative, while implicit colors retokenize with chart options. Chrome that stands for a bar itself — the built-in live price line, its last-value axis chip, and the crosshair marker — follows one shared bar-color resolution. The built-in live price line is one canonical series feature: its default `partial` extent starts at the tracked bar/value and reaches the pane's right edge, while `full` is an explicit per-series option. Both extents use the same source, color, width, and solid/dotted/dashed line-style state, so ordinary series and engine indicator outputs cannot drift in thickness or dash semantics. Explicit user-created horizontal price-line objects remain full-width independent chart objects. For candlesticks that resolution walks the parts in paint order, body then border then wick, skipping any part that is transparent or switched off, so a hollow candle (a transparent body over a visible border frame, industry-standard) keeps its bullish or bearish color instead of resolving to an invisible fill. Bar presentation remains engine-owned as well: OHLC bars keep one vertical high/low body and independently gate the open and close ticks, so setting both visibility flags false produces an explicit high-low bar without a host-side geometry fork.

The engine retains semantic pane layers and their ordered primitive/point ranges, then assembles the same canonical `ChartFrame` contract from clean and rebuilt layers. The retained boundaries are underlay/grid, individual series, per-drawing segments plus a trailing creation-preview block, pane chrome, transient trading risk/reward preview regions, financial-action lines/controls (trading and alerts), and overlay. Frame assembly is the single pane-local ordering owner: grid/background → idle indicators → idle drawings → ordinary price series → active objects (dragging/editing → hovered → selected, series before drawings within a tier, previews trailing active) → chrome → trading regions → trading/alerts → overlay → top. Indicator outputs move as one visual group with internal ordering preserved (bindings own grouping, never series type or title); explicit `set_series_order` overrides default idle series grouping while idle drawings stay below price series and indicator-only panes keep stable internal order. Axes, crosshair, and financial-action controls keep their protected layers above all chart content; active chart content stays clipped to its owning pane. No public z-index API or renderer-specific policy exists. Preview regions sit above chart content with financially actionable lines, alert indicators, and exact control hit zones above them and below crosshair transients. A series' live-price cluster stays filled while its value is live, and is outlined — chart-surface fill, semantic color as an inside border and text — once the series' final bar scrolls out of view, so a stale value never reads as the current one. Colliding series clusters are spaced by the overlap pass rather than restyled. Otherwise-solid trading tags that meet the primary cluster's raw axis region take that same outlined treatment, but financial-action tags stay at their exact price coordinate instead of being collision-shifted away from the line they identify. They emit before the primary cluster so the live price remains visually authoritative if exact coordinates overlap. Confirmed orders never manufacture persistent risk/reward fills. Positions and orders use a 304 CSS-pixel bounded marker beside the price scale. Its control cluster and line start move left together, so the rule begins flush at the container rather than exposing an empty lead-in. Only that visible marker span is interactive. Each marker is a readout chip — a solid quantity cell and then P&L or order type inside ONE outline, with no inner border or divider, so the quantity block's own edge is the seam — followed after a gap by the detached close/cancel chip. The readout, quantity fill, and close chip are square; a destructive control never shares an edge with the readout it would destroy. Outline, quantity fill, and close mark all carry the object's semantic color, so the whole marker reads as one color from the line to the price tag; only the P&L text keeps its own profit/loss tint. The close mark is stroked geometry rather than a font glyph, because a host `font_family` is not guaranteed to carry the multiplication sign. Protection role takes precedence over broker side and kind: TP is profit green and SL is warning yellow. For ordinary entries, resting buy limits use the neutral blue `working_order` accent while sell limits and other sell orders use bearish red; non-limit buy orders and fills use their directional buy color. Rejected, cancelled, and expired markers use the rejected color, while pending broker operations use the pending color. A price tag is solid only once its order is actually filled — a resting order stays outlined, so a working intention never reads as an executed one. Every marker chip is outlined with its own semantic hairline so a hollow chip reads as a rule rather than a heavy frame — on the pane labels and on the axis price tag alike; that object outline is independent of the design-system axis/pane border-width token. Hover and press tint the addressed control, and the close chip answers hits across the chip's full height rather than the line tolerance. A hovered working-order rule changes to compact hairline dashes without increasing stroke width, keeping dash metrics consistent across device-pixel ratios. Those tints are pre-blended against the chip surface and stay OPAQUE: a control sits on top of its own marker line, and a translucent fill would let that line read through the button the pointer is on. For the same reason the host suppresses the crosshair lines while the pointer is over a trading control — the control is not a price to read. Action tooltips are host-timed: the engine owns no clock, so it reveals one only once the host arms it after a hover dwell, and a changed hover disarms it. Sweeping across stacked markers therefore never flashes a tooltip per marker. An action tooltip is chart chrome rather than part of the object it describes: it takes the active theme's surface, border, text, and radius tokens, never the order's buy/sell color, so it reads identically on every line and in both themes. Confirmed TP and SL orders remain ordinary host-authoritative order markers with quantity and projected P&L, endpoint nodes, and action tooltips. Trading, alerts, and axis labels share the canonical pane price transforms; WebGPU, Canvas2D, GPUI, screenshots, and native/headless consumers receive no separate financial-action geometry. Retention never gives a backend permission to change ordering or semantics. Host/plugin primitive callbacks use the canonical frame but conservatively rebuild the affected pane stream because their output is not engine-owned. Incremental frames are tested against forced clean rebuilds across data, interaction, scale, drawing, theme, and resize mutations.

Trading interaction is one engine-owned state machine: idle, hovering, dragging through a local preview, or awaiting a host answer while holding that change's rollback. These states are mutually exclusive. Bounded hover and pressed hits are retained separately as visual feedback, so actionable buttons continue to respond while the semantic state is awaiting confirmation without those visuals becoming broker state. Hovering a working entry changes its marker line to dashed; dragging that line creates attached protection rather than moving the entry. The engine classifies the new level from the entry side and its current price displacement (buy: lower SL / higher TP; sell: higher SL / lower TP), emits `create_stop_loss` or `create_take_profit` on release, and leaves identity assignment and the authoritative child order to the host. Limit and market entries share this behavior, including filled market entries. Once supplied by the host, an SL or TP is an ordinary protection order whose role stays fixed even when dragged across its entry; its modify intent preserves the authoritative kind and stop-limit trigger price. Pointer movement changes only the local preview; release applies an existing-order move or emits a protection creation intent and holds the appropriate rollback until the host answers. Position markers do not start drags. Escape discards a live drag. Rejecting an emitted intent runs its rollback — reinserting a closed order or position, moving a modified order back, or doing nothing for an unmaterialized protection request — while acceptance releases it and the host's snapshot remains the last word. A separate active/inactive trading-group visual state owns confirmed bracket connector chrome. Successful protection acknowledgement activates the related bracket, position, or parent-order group; an empty-canvas press deactivates only that visual state, leaving broker relationships and confirmed entry, TP, and SL objects intact.

Trading quantity cells use the host's canonical text measurement plus bounded horizontal padding, so the visible cell and close-control hit geometry respond to the formatted quantity without a fixed empty allotment.

Panes expose opaque, monotonic chart-local identities at the browser boundary. A live pane or
price-scale handle resolves its current index after moves or swaps; removal permanently invalidates
that handle, so later index reuse cannot retarget it to another pane or scale. Persistence uses a
separate stable pane identity and intentionally issues fresh live IDs during restore.

The ordered frame contract contains pane backgrounds and grids, idle indicator geometry, idle drawings, ordinary series geometry, active series/drawings/previews, custom-series contributions spliced at their paint marks, pane chrome, trading regions, trading/alerts, crosshair overlays, axes, labels, and text, plus per-series and per-drawing segment ranges for retained backend groups. `series_order`/`drawings` stay the stable saved orders; the frame derives the effective paint order without rewriting them, and series/drawing hit tests tie-break on stable order so promotion cannot oscillate hover. Backends preserve ordering, clipping, blending, and coordinate conversion. A backend may batch compatible adjacent primitives only when visible output is unchanged.

Trend-line labels are owned by the trend-line feature rather than by `DrawingKind::Text`: the
engine owns their text state, dedicated hover affordance and hit region, edit-session identity,
segment-local transform, and middle-stroke cutout. New trend labels default to the top-right slot;
their 3×3 slots resolve along and perpendicular to the actual segment. The direction is normalized
into the readable half-plane, including a
deterministic vertical orientation, so endpoint crossing preserves visual left/right and never
turns glyphs upside down. Pointer hits are inverse-transformed into the measured local text
rectangle. An unset trend-label text color follows the drawing stroke dynamically; an explicit text
color remains independent. Empty labels use that same resolved RGB at reduced alpha for
`+ Add text`; entering or
leaving the dedicated trend-label editor never converts or deletes the trend line. Middle labels
split the stroke in segment-parameter space using measured advance plus padding. Hover reserves the
prompt advance; editing starts with a compact one-em caret opening and expands from shaped text
advance as the user types. Top and bottom slots never cut the stroke. The browser uses a fully
transparent borderless editing surface (including native caret and IME composition paint) plus one
explicit colored caret at the engine's exact anchor and angle, leaving the frame as the sole glyph
owner. Its selection pseudo-element is transparent as well, preventing browser selection/IME paint
from leaking theme-colored duplicate glyphs during live transforms.
Standalone Text retains its separate create/remove lifecycle and explicit toolbar text input.

Segment-following text is an explicit `RotatedText` frame primitive carrying the final aligned
anchor, clockwise angle, font, weight, italics, size, color, and text; no executor reconstructs
trend geometry or silently ignores the angle. Canvas2D translates and rotates around the anchor
before `fillText`; WebGPU sends only rotated runs through a dedicated vertex pipeline that rotates
and linearly samples the unchanged cached glyph-atlas quad while preserving the ordinary-text
instance/shader contract;
tiny-skia resamples a local glyph-coverage raster around the same pivot; GPUI uses its transformed
monochrome-sprite path backed by its atlas. Browser and GPUI caches key glyph-dependent inputs and
subpixel phase but deliberately exclude angle, so endpoint motion reuses glyph coverage. Their
fixed-capacity/LRU or atlas budgets bound retained entries, and font, DPR, device, or atlas-generation
invalidation drops stale resources.

One-click bracket placement crosses the drawing/trading boundary only through an explicit engine command. A host passes a Long/Short Position drawing identity plus its own quantity; the engine reads the drawing's semantic entry, target, stop, pane, and price scale, snaps all prices to instrument ticks, and emits one atomic `place_bracket_order` intent. It creates no speculative order or position. The broker host owns submission, venue-specific entry interpretation, generated order/bracket/OCO identities, acceptance or rejection, and the authoritative snapshot that materializes the resulting lines. The web demo's quantity input and intent handler are an example host, not account-sizing or broker policy inside Aeris.

## Plugins and host extensions

User-defined custom series and primitives remain explicit host boundaries. The engine owns their identity, layout participation, hit-test context, autoscale contribution, and built-in chrome integration. A host may execute an arbitrary user callback, then records the values the engine needs for the next canonical frame. The official plugin implementations above do not use that callback path.

Extensions must not receive unrestricted engine internals or create a second scene graph. Add extension surfaces only for current consumers with a stable semantic need.

Disposal invokes every registered extension teardown exactly once; one failing JavaScript cleanup hook cannot prevent the remaining hooks from running.

Extension rendering is host-timed, non-reentrant with chart mutation, and error-contained at the
host boundary. Extension runtime objects and callbacks are never persisted by the engine; hosts own
their configuration and restoration. The current custom-series and primitive APIs are experimental,
not a second plugin framework.

The browser package's official-feature modules are thin lifecycle and platform adapters over these
engine owners. They normalize public data/options, translate pointer or keyboard events, decode
browser images, and create optional DOM chrome; they do not simulate financial geometry. Tooltip
guides/value lookup, accessibility focus geometry, drawings, bands, price lines, overlay
labels, image placement, and every specialized series frame are constructed in Rust. The engine and
its browser/native hosts do not inject product attribution or branding into chart surfaces. Feature
handles release their engine primitive
plus any host subscription,
timer, or DOM node exactly once; none of that runtime state enters engine persistence.

The GPUI demo uses the shared crosshair-action hit test for its pointing-hand cursor and `Alert` input target. Its dedicated press/release state prevents chart pan and selection underneath the button; an unmoved release within the hit area emits the shared action request and displays its pane and price in the demo status line. Pointer cancellation discards the pending press.

Interactive chart objects own pointer feedback: the shared frame suppresses the complete visual crosshair (lines, markers, and axis labels) while any trading object or drawing is hovered, created, or dragged. The engine retains the crosshair position for snapping and host callbacks, while each host continues to show the object's pointer, click, grab, or drag cursor.

## Performance contract

Performance comes from avoiding work:

1. Recompute only invalidated state.
2. Keep hot data columnar and transfers bounded.
3. Reuse GPU, text, image, and geometry resources.
4. Conflate replaceable frame requests while preserving the newest state.
5. Keep rendering and input queues bounded.
6. Measure release builds before changing algorithms or adding caches.

Large-history geometry and hit testing are bounded by physical viewport density plus hierarchy-boundary refinement rather than visible source-row count. The hierarchy is a compact canonical-data auxiliary index, not a renderer cache: GPUI, WebGPU, Canvas2D, native rendering, retained rebuilds, and forced clean rebuilds all consume the same selected geometry. Native evidence records selected level, summary-node operations, raw boundary rows, and candidate rows; browser evidence records the resulting frame CPU, backend work, allocations, and upload bytes without exposing LOD controls through the public chart API.

Drawing work is independently bounded before frame emission. Charts with at most twenty drawings use the direct stable z-ordered render path to avoid index overhead. Larger charts scan only the hovered pane's compact bounds entries, use semantic-domain rejection before coordinate work, preserve canonical pane stable z-order in the candidate list, and run exact per-tool hit tests only for pointer candidates. Retained per-drawing segments reassemble idle-below / active-above without rebuilding geometry on promotion; hit tests stay on stable order so promotion cannot oscillate hover. This intentionally small local structure has no spatial-tree dependency: its cheap bounds pass is linear in drawings owned by the pane; text extents are measured once per semantic/font generation; and coordinate conversion, brush traversal, primitive emission, and precise hit testing follow the candidate count. Pathological complete overlap therefore remains an explicit linear candidate worst case. Cache memory is bounded by live drawing entries, retained path-point capacity, pane membership, and one reusable candidate scratch vector; removal releases the entry and no historical geometry is retained.

Trading state is capped at 4,096 live positions, orders, and executions per chart, and pending intent delivery is capped at 256 entries. Expected terminal workloads (10, 50, 100, and 500 trading objects) use a direct topmost-first pane scan for hits and one retained trading rebuild per semantic/preview mutation; unchanged frames reuse both trading layers. This deliberately avoids a spatial tree until measurements justify one. Engine memory telemetry includes trading vector, intent-queue, and retained string capacity.

Track CPU frame time, GPU time where available, draw calls, dropped and presented frames, memory, ring overruns, interaction latency, and steady-state allocation. Device loss or unavailable WebGPU must fail over without losing headless chart state.

Each browser chart owns reusable WebGPU vertex buffers for its retained semantic draw groups. Buffers grow geometrically to a high-water capacity, upload only when the corresponding group revision changes, never shrink during the chart lifetime, and are released with the chart's GPU state. The public last-frame telemetry also reports buffer allocations, buffer writes, uploaded bytes, and retained-layer rebuild counts so stable and localized-update behavior is directly testable.

The shared text atlas treats one render as a transaction: slots referenced or inserted in the current frame cannot be recycled until that frame completes. Atlas pressure after the frame has accepted text defers the reset to the next frame; the browser renders the pressured frame through Canvas2D rather than submit stale UVs. A reset increments the atlas epoch, invalidating retained textured groups and text-cache entries before the next WebGPU submission.

Browser WebGPU shares one page-wide adapter/device/queue and atlas while retaining per-chart surfaces. Device loss is therefore a shared generation event, not ownership of the chart that first created the device: every live chart listener wakes and falls back, while disposed charts have no listener. Headless chart data is preserved through fallback.

The browser package's default `auto` backend prefers WebGPU but keeps Canvas2D available when the browser exposes no usable adapter; `navigator.gpu` alone is not proof of adapter availability. A failed adapter request is cached for that page session so independently mounted charts and viewport remounts do not repeatedly probe an unavailable adapter. Device-initialization failures remain retryable, explicit fallback-adapter diagnostics are isolated from the ordinary adapter result, and a reload permits a new adapter probe after browser or driver settings change. The General dashboard reports the actual backend and fallback reason rather than rejecting charts when WebGPU is unavailable.

Chart construction accepts an explicit first-pane horizontal domain. The engine creates either the
compatible financial pane plus primary candlestick series or one preserved general pane with no
financial series; browser hosts do not add a temporary financial pane and remove it afterward.
Rejected construction removes the canvases installed by that attempt before control returns to the
caller. The engine retains one layout slot at all times. Explicit removal of an empty preserved final
pane retires its stable and persistence identities, releases its general-domain/axis state, and installs
a fresh unpreserved financial-time pane in the same slot. The removed handle therefore stales normally,
and declarative cleanup never needs a temporary keeper pane.

## Evidence benchmark subsystem

`benchmarks/` is development and release evidence infrastructure outside every production crate and the published package. Its single Node entry point builds the actual release package, drives the public browser API through the existing Playwright demo host, generates deterministic versioned OHLCV data, validates versioned JSON results, compares explicit baselines, applies centralized budgets, and emits human- and website-readable artifacts. The browser page is served by `examples/web_demo/test_server.mjs` only for automation; it is not part of the npm package.

The subsystem reuses `chart_api.frame_stats()` for bounded CPU, real capability-detected WebGPU timestamp, draw, presentation, dropped-frame, ring-overrun, and WASM-linear-memory observations. It does not add production instrumentation, dependencies, imports, feature flags, logging, or runtime branches. Browser page/heap memory is labeled as whole-page memory, and unsupported presentation or GPU measurements remain unsupported rather than inferred.

Raw local results are ignored and CI results are artifacts. Public summaries and committed release baselines require a clean `release` profile result classified as `official-benchmark-runner`; shared CI timings are smoke/trend evidence only. Scenario, dataset-generator, schema, and baseline versions preserve historical comparability.

Benchmark comparisons enforce only explicitly configured budgets. An empty policy is reported as `NO ENFORCED BUDGET`, and configured keys must match comparable metrics so a typo cannot silently disable a hard threshold. Publication requires the portable browser runtime/parity suite; machine-calibrated pixel, GPU, and wall-clock evidence remains a separate non-blocking result.

## Correctness and parity

Chart math must be deterministic for the same state, viewport, and device scale. Validate malformed data at the input boundary. Preserve whitespace rows, time ordering, logical ranges, primitive order, and explicit warm-up gaps.

OHLC ingestion preserves structurally valid numeric input rather than silently rewriting financial values. Impossible relationships are accepted for compatibility but counted in structured diagnostics alongside accepted, dropped, deduplicated, reordered, non-finite, and out-of-range rows. Invalid timestamps reject a direct transaction before value-row repair; accepted batches retain the existing value repair semantics. Clean ingestion returns no diagnostic object on the browser hot path. Predictable boundary failures carry stable error categories rather than relying on console text.

The generic `Workspace` engine type owns only split-tree topology, stable cell identities, ratios,
and bounded validation of a restored layout. Subscription caps, billing-tier vetoes, cumulative
split usage, storage, provider identity, and cell-age metering live in the browser grid host; the
shared engine has no commercial-policy or account knowledge.
Workspace divider mutations reject non-finite deltas without changing the layout, and splits
reject exhausted `u32` cell identities before mutation so browser handles remain addressable.

Changes to geometry, snapping, scales, interactions, or execution require the narrowest relevant combination of unit tests, frame-contract tests, golden images, draw-stream parity, replay stability, browser tests, and release performance evidence. A backend-specific screenshot alone is not proof of shared-engine correctness.

## Dependency direction

Lower layers never import a host API to bypass their boundary. The headless path is `aeris_charts_core` and `aeris_charts_indicators` into `aeris_charts_engine`, then `aeris_charts_render`; GPUI, WebGPU, native, and WASM/browser code sit at execution boundaries. Avoid new crates, traits, and feature flags unless they enforce a real current dependency or platform boundary.

## Repository documentation

Markdown documentation may live at the root or beside the component it explains when it has a durable repository purpose. Keep the root README focused on product orientation and contributor setup, and keep architectural ownership and data flow in this file. Do not commit transient work notes, generated reports, or duplicate documentation.

## Verification

The standard gates mirror CI:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p aeris_charts_wasm --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
cargo run -p aeris_charts_native --example perf_gate --release

cd packages/charts
npm ci
npm run lint
npm run build
npm run typecheck
npm run test:pack
```

The release performance gate also measures a 100,000-visible-bar volume-profile refresh through frame construction and verifies that unchanged frames retain the calculation revision.

Run Playwright for browser behavior, rendering, interaction, packaging, or parity changes. Run GPUI parity and replay checks for GPUI executor changes. The pixel-parity harness enforces the crosshair icon image against native rendering with at most one channel value of blending-rounding difference. Changes to the icon source or masks also run `node examples/web_demo/build_crosshair_icon.mjs --check`.

The evidence harness has one entry point:

```text
node benchmarks/benchmark.mjs test
node benchmarks/benchmark.mjs smoke
node benchmarks/benchmark.mjs release
```

Tag publication requires the Rust, package, and portable Chromium/Firefox/WebKit jobs. Public
declaration and release-policy guards, V1 fixtures, Node import, and pack smoke are portable blocking
checks. Configured `perf_gate` budgets run strictly. Machine-calibrated screenshots, GPU timings,
heap sampling, and wall-clock evidence stay in separate non-blocking diagnostic steps; approved hashes
are never changed merely to satisfy a different host.

Indicator multi-input validation is engine-owned: VWAP and VWAP-band bindings require a distinct
live scalar volume series, while missing volume remains the explicit unit-weight fallback. Financial
study persistence V3 stores binding definitions, dependency references, scalar inputs, volume inputs,
and output styles while leaving market history and ordinary series data host-owned. Trade, quote, and
depth study inputs remain owned by the host market runtime: it supplies typed stream requirements and
generation-fenced borrowed views, while Charts receives only bounded scalar study output publications.
Charts must not retain a second tape/book or infer provider stream state from a rendered series. The
Terminal bridge now carries the transitive stream requirements as bounded output metadata and persists
only the validated host binding; this keeps the runtime-to-chart boundary explicit while allowing
downstream output presentation to retain its typed input contract.
