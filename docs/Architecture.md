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

Canonical series data uses opaque chart-local `u32` identities mapped to reusable storage slots. Identities are never reused, removed identities are classified as stale, and slot-backed vectors remain bounded by peak concurrent series rather than lifetime add/remove count. Each ordinary series owns one timestamp column and either one scalar value column or four OHLC columns. `PlotList` owns only a dense range or sparse logical-index mapping plus its chunked autoscale cache; allocation-free views join that mapping to the canonical values for queries and frame construction. Dense aligned mappings carry no per-row index allocation. Indicator outputs own one scalar value column and alias a contiguous source-time range by identity, so they duplicate neither source timestamps nor plot values. An output still before its warm-up holds no rows, so its range starts at the first value an update computes, also when that update rewrites earlier source rows. The merged timestamp union remains independently owned because ordinary source series are independently mutable and may diverge or carry whitespace. It carries a generation that changes only when its contents change; time weights use that generation, while value-only current-bar updates retain the O(1) fast path.

A multi-calendar overlay opts out of that union per owned series (`TimeAlignment::AsOf`, the browser's `series_options.time_alignment: "as_of"`): it adds no merged time point, so the union-timed series keep a gapless axis. Its `PlotList` holds a dense or sparse logical-index mapping onto the merged points up to the union's data extent (the last point where a host-owned union series holds a real row, so an overlay never runs into host-installed future session slots) plus a non-decreasing plot-row to canonical-row map. Each point shows the last canonical row at or before its time; rows between two points collapse into the later one, a point with no newer row repeats the previous one unless that row is older than the optional `max_staleness`, and rows after the last point wait for it. Values are never copied: `PlotValues::AsOf` resolves every value, whitespace test, autoscale chunk, and per-point color through the map, and the pyramid of a mapped plot summarizes plot rows. Readers that need canonical rows (studies, `data()`, merges, bar times, selection anchors, visible-range volume profiles, Heikin Ashi, session highlighting, the accessibility focus ring, and the data-reading drawings: regression trend, forecast, and bars pattern) translate through `source_row`, `row_for_source`, or `source_range` (the drawings through `ChartEngine::drawing_source_window`, which places each canonical row at the first point at or after its time); markers resolve to the first point at or after their time and wait, like their rows, past the last point. A value tick of an as-of row repairs only the points showing it; a new as-of row rewrites only the points at or after its time; a new union point or a moved data extent appends or truncates only the tail; a historical as-of insert re-derives that series' map; any union rebuild re-derives every map in `O(points + rows)`. Indicator outputs of an as-of source compute on its own rows and alias its map with their row offset, rewriting only the changed tail. A series whose points move because of another series' data (a tail resync or any union reindex, including a reinstall or pop inside pre-installed session slots that leaves the time points unchanged) is reported once through the data layer's realigned list, which time synchronization drains into series-frame invalidation. Only engine-valued time series (line, area, baseline, histogram, bar, candlestick) that own their rows may be as-of; custom, advanced, footprint, trade-bound, trade-study, and synthetic series, indicator outputs, and any series while a non-time sequence axis is installed are refused, and a conversion to a host-valued kind or a footprint, a trade-bound binding, or a synthetic configuration rejoins the union first. An output that is re-aliased after owning its rows gives up its own alignment and follows its source again. The policy is host-owned series configuration and, like every financial series definition, is not persisted.

Each canonical built-in series also owns an eager fanout-16 row-summary pyramid. A summary stores only six `u32` source-row identities: chronological endpoints, OHLC low/high, and close minimum/maximum. Values are always dereferenced from the canonical columns, so the hierarchy adds no second value owner and preserves whitespace. Point and tail mutations repair one node per affected level; typed batches repair the affected range once; historical insertion, replacement, and retention rebuild or repair the exact affected hierarchy before the mutation is visible. The same endpoint summaries bound latest and predecessor lookup to at most fanout work per hierarchy level even across pathological whitespace; no parallel predecessor index or value history is retained. Series removal releases the hierarchy with the canonical storage slot. Custom-series host geometry is not summarized because its semantics are not engine-owned.

Each canonical series also carries a data generation. An ascending typed batch is sanitized once at the host boundary, merged into its source in one data-layer operation, then synchronizes merged time points, tick weights, dependent indicators, and frame generations once. Tail batches append weights incrementally; historical batches merge in `O(n + k)` and reindex once rather than once per input row. A single update, or a batch whose rows before the series' last bar only correct bars whose timestamps the series already holds (the late N-1/N-2 revision), is applied in place: each corrected row repairs its summary path and autoscale chunk, the batch's tail rows stream through the append/replace path, and nothing merges the timestamp union or reindexes a plot. A new historical row at a time the union already holds reindexes only its own series (and aliased outputs); only a timestamp new to the chart inserts into the union and reindexes every series. A data-layer transaction that actually rebuilds the timestamp union temporarily captures its prior union and emits one old-to-final logical mapping through common timestamps when the engine synchronizes. Current-bar replacements and pure tail appends capture and map nothing, and no second merged timeline survives the synchronization boundary. The union is a linear merge of the union series' already-sorted time columns (up to 16 series; a wider layer sorts the concatenation instead), and a retention that trims several series at once (`DataLayer::trim_fronts`) merges it and reindexes every plot once, not once per trimmed series. On a time axis the synchronization after such a trim keeps the surviving points' tick weights, drops the evicted ones, and re-weighs only the new first point and the appended tail; a non-time axis, whose tick times come from the bar sidecar, re-weighs every point.

Streaming keeps reference `series.update` semantics (the point replaces the whole bar). Two real-time policies are engine-owned rather than host-rebuilt: `ChartEngine::merge_series_bars` (and its single-row form `merge_series_bar`) merges partial bars field-wise into the stored bars in input order (absent fields keep their values and color overrides; candlestick/bar results are normalized so high/low envelope open and close; scalar series take the value), validates the whole batch before mutating, applies each row through the streaming data-layer path, and synchronizes time state and indicators once. An optional per-series monotonic sequence (`SeriesEntry::update_sequence`, O(1), runtime-only, cleared by a full data install) rejects stale deliveries for sequenced updates, merges, and typed batches before any mutation; custom and advanced series, whose payloads bypass this path, refuse a sequence instead of ignoring it. A streamed row's explicit color creates that series' color channel on first use, so streaming and full installs color bars identically. Volume and turnover stay independent series; the engine never infers them from a price series. The browser package only encodes absent fields and the optional sequence at the WASM boundary and reports diagnostics; it keeps no bar cache or sequence state of its own.

One core validator defines canonical numeric time: a finite, integral count of whole UTC seconds in
the inclusive range `-62167219200..253402300799` (years 0000..9999). Hosts do not auto-convert
numeric timestamps. Out-of-range errors include a likely milliseconds, microseconds, or nanoseconds
hint when dividing by that unit would enter the supported range. Direct set/update batches validate
all timestamps before repair or mutation and reject atomically; single updates likewise preserve
state. Shared-ring drains may reject individual rows because producer drains cannot be rolled back.

Presentation and grouping of those instants use one chart-level `ExchangeTime`
(`aeris_charts_core::scale::exchange_time`): a validated, bounded (≤1024 transitions) UTC-offset
schedule, a trading-day session start relative to local midnight (negative starts assign evening
sessions to the next trading day and roll weekend days forward to Monday), and a calendar-date flag
for business-day input, whose rows are never shifted. The default is UTC with a midnight start, which
reproduces the historical behavior exactly. Tick weights (trading days for Day/Month/Year, exchange
wall clock for intraday boundaries, including the non-time sequence axis), built-in labels, VWAP and
pivot period keys, session highlighting, and the countdown window all read it. The engine accepts
only explicit schedules and never consults a platform time zone; the browser package resolves IANA
names with `Intl` and derives the calendar-date flag from its series' input forms, while Rust hosts
pass both. Changing it rebuilds tick weights and period-keyed indicator bindings once (the
calendar-date flag alone is a no-op on the UTC identity). The zone and session start live in the
options store, so V2 persistence carries them; importing a document without them keeps the
installed exchange time.

Session slots for intraday (time-sharing) charts are generated by one platform-free core function,
`aeris_charts_core::scale::session_slots::session_slot_times` (re-exported by the engine): a trading
date, chronological exchange-local windows (which may cross midnight), a bar interval, and an
`ExchangeTime` produce the UTC bar times, each window converted with the offset in force on that
date and placed inside the trading day the chart's session start defines (weekend roll included).
Input is validated and the slot count is checked against a 100 000 bound before allocation. The
browser package resolves the IANA zone and calls it through a free WASM function; which dates trade
remains host-owned calendar data. Hosts install the slots as whitespace rows.

The same placement (`session_window_bounds`, the UTC `(open, close)` of each window of one trading
date) drives session-anchored bars. Core `SessionBarGrid` maps an instant to the open of its bar:
the windows of the instant's trading day (or a previous-day window that reaches past the session
start) each restart the bar grid at their open, the last bar of a window ends at its close, and a
one-day interval spans the whole trading day. Prints outside every window either fold (before the
day's first window into its first bar, later into the preceding window's last bar) or are excluded
except in a window's closing second. A lookup places at most two trading days on stack buffers and
allocates nothing. Trade streams opt into it per stream (below) in the chart's exchange time, and
`resample_boundaries` turns host trading dates plus the same windows into resampling boundaries.

Explicit time-axis marks (`timeScale.tickMarks`) are engine state beside the automatic tick
weights: at most 512 strictly ascending UTC times with optional labels of at most 64 bytes. While
set, `ChartEngine::time_marks` resolves them to exact time points (whitespace slots included; marks
without a point are skipped), so the vertical grid and the axis labels, which both read that one
owner, follow the anchors; labels default to the exchange-time label of their point's weight, are
edge-aligned inside the axis strip, and a label overlapping its predecessor is dropped while its grid
line stays. A mark on the chart's first point is weighed like the automatic first mark (against
one average spacing), except that on a chart spanning several trading days it weighs at least a
day, so every day-open mark shares one label style. The list is validated with the exchange-time
keys of the same patch, mirrored into the options store (so V2 persistence and worker option
patches carry it), and attributed to tick memory.

## Crate boundaries

### `aeris_charts_core`

Platform-free chart fundamentals: validated canonical columnar data, compact plot index/view storage, ranges, options, formatting, price scales, time scales, tick marks, and shared math. It also exposes structure-level payload and capacity attribution for memory evidence; these counters are not allocator, WASM-page, or browser-memory measurements. Media-space calculations remain `f64`; conversion to backend coordinate formats happens at rendering boundaries.

Price tick marks are built on the caller's price grid. The reference span search runs unchanged and
its result is then widened to the smallest nice span (`{1, 2, 2.5, 4, 5} × 10^k`, else
`min_move × {1, 2, 5} × 10^k`) that is an integer multiple of `min_move`, so every tick is a
tradable price; decimal and binary minimum moves already satisfy this and keep reference output.
The grid may depend on the price interval (a tick ladder): the view's grid picks the first span, and
a log scale re-derives each following span from the grid of the interval below the last mark, so
low-priced regions keep their own finer ticks.
`PriceTickLadder` (at most 64 validated ascending bands whose bounds lie on both adjacent grids)
owns band lookup, snapping, per-band precision and labels, cumulative tick indices, and the grid of
a price span (the LCM of the touched bands' ticks). `PriceScaleCore` owns the autoscale shaping that
follows source merging: an optional symmetric center, the opt-in stable mode (grow immediately,
shrink only when data one bar beyond the visible edges leaves more than 20% of the range unused;
reset by mode/base/center changes and autoscale re-enable; the engine also resets it when a source
changes structurally: full data replacement, including indicator outputs rebuilt from it, and
series visibility, removal, or pane/scale rebinding), degenerate
padding, and log refit. `ensureEdgeTickMarksVisible` boundary marks and their half-font padding
apply only while autoscaled, as in the reference.

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

Pure technical-indicator calculations over numeric slices. Warm-up gaps are explicit. Convention-dependent formulas take typed parameters owned here: an EMA-family/RSI seed (`Sma`, the TradingView/TA-Lib default, or `FirstValue`, the 通达信/同花顺 `EMA(X,N)`/`SMA(X,N,M)` convention), the MACD histogram multiplier (1, or 2 for `(DIF-DEA)*2`), and the Bollinger deviation estimator (population or sample); `IndicatorConvention` only maps a preset to those values. KDJ uses windowed RSV with `SMA(X,N,1)` K/D smoothing started from a typed `KdjSeed`: the textbook 50 after a full N-row window (default), or the formula-language start, which the China preset selects: RSV over the rows available while fewer than N exist and `Y0 = X0`, so values start at row 0. `tests/platform_values.rs` pins the verified platform rules (China MACD, population-σ Bollinger, the formula-language KDJ start, and the textbook start's convergence after `convergence_rows`) on a deterministic synthetic series with independently computed expected values; the comparison with a platform's published values for real bars was done out of tree, and that data is not committed. Each runtime also reports, per output, its warm-up rows and a convergence horizon (rows until every recursive seed's weight is below 0.1%, or none for time-anchored and path-dependent formulas); the engine sums them along indicator chains. Whitespace source rows (NaN close/high/low) never enter formula state: recursive runtimes skip them in place and emit NaN, while stateless window formulas that meet whitespace in the rows they read evaluate over a bounded compacted window (their lookback plus the changed suffix) and scatter NaN back to the whitespace rows, so every value equals the value computed without those rows. Alongside clean full-recomputation functions, it owns the explicit per-formula rolling state used for append, current-bar replacement, and rebuild-from-index. Bounded-window formulas retain no source-length state; recursive formulas retain tail state and one checkpoint per 1,024 source rows, then recompute from the nearest prior checkpoint after a historical correction. The retained state before the last row resumes a replacement of that row whether or not the same rebuild also appends rows, so closing the current bar and opening the next in one batch never falls back to a checkpoint. Every built-in kind therefore bounds a current-bar replacement, an append, or both in one rebuild by its window rather than the history: window formulas (moving averages, standard deviation, CCI, Williams %R, momentum, rate of change, Donchian, Ichimoku, CMF, MFI, volume) evaluate only the changed output rows, each over its own period, while Stochastic RSI (Wilder RSI state plus a retained tail window of its last RSI values, like Stochastic `%D`), pivots (the running and previous trading-day sessions), and ZigZag (its anchor, direction, and provisional extreme) are recursive states that skip whitespace in place. ZigZag writes turning points back to earlier rows, so a tail rebuild re-emits rows back to its one open turning point (the provisional endpoint it moves or restores, or the first-direction anchor it writes or clears) only when it changes that point's value, which bounds it by the rows since the last confirmed turning point; a tail revision that leaves that point alone touches only the forming bar, and a historical repair re-emits from the open turning point of the state it resumes. `last_work_rows` reports the rows each rebuild actually evaluated. Volume and turnover columns may be shorter than the source rows; rows past their end take each formula's missing-weight fallback (unit weight for VWAP, VWAP bands and VWMA, zero for OBV, CMF, MFI and volume, no trade for amount-weighted VWAP), which equals the engine's timestamp-alignment fallback. Sparse checkpoint vectors are copy-on-write so hosts can transactionally clone recursive state without deep-copying retained history during ordinary tail work. Host-neutral indexed EMA, ATR, session-VWAP, RSI, MACD, and Stochastic states accept callback-provided optional samples so non-chart Rust hosts can lazily convert only the canonical rows replayed for a dirty suffix. `None` is a hard reset; recursive replay may begin at an earlier sparse checkpoint while writers receive only the requested suffix. Stochastic additionally retains only bounded tail `%K` windows needed for `%D` tail replacement (Stochastic RSI likewise retains its last RSI values), and a rebuild that cannot resume from them, a truncation included, replays enough earlier rows to refill the window a later replacement of the new last row reads; its windowed high/low scan remains bounded by the configured `%K` period rather than source-history length. Derived values use short-lived transfer buffers that move into or update the engine's canonical output series and are capped after partial repairs. This crate does not know about charts, panes, rendering, WebAssembly, or GPUI.

Calendar periods (VWAP session/weekly/monthly resets and pivot sessions) key on a caller-supplied
trading-day mapping; the crate knows no time zones, and weekly periods start on Monday. The chart
runtime passes the engine's exchange trading day, while the full-recomputation functions keep UTC days.
`VwapReset::period_key` exposes the reset key itself, so chart geometry breaks lines exactly where
the values reset.

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
design-system width; the time strip likewise keeps its existing border slot, text, tick, and vertical
padding, snapped to an even CSS-pixel height (22 px by default). Price tags are
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

Public coordinates are chart-content space, never pane-local: `x` is CSS px from the plot-area left edge and `y` is CSS px from the top of the stacked pane area, so a lower pane's scale returns a `y` inside that pane's `[top, top + height]` and every conversion, pointer position, crosshair, drawing, and trading round trip shares that one space. Two plugin surfaces differ deliberately and are documented as such: the draw-context converters return bitmap px of the whole chart, and an axis-label descriptor's `coordinate` is pane-local. Series conversions use the series' own pane and scale. The chart-level pair is a shared engine query on the pane default scale (the scale the crosshair label reads): a price converts on pane 0, and a coordinate converts on the pane containing `y` (a separator belongs to the pane above, a `y` below the panes to the last pane); neither follows series creation order. All of it reflects the last layout pass.

Series pane/scale rebinding is one validated engine mutation. An unknown destination leaves pane,
scale, data, type, style, visibility, streaming state, and handle identity unchanged. Percentage
and indexed geometry uses each series' own first visible value by default; a chart-level comparison
anchor may replace that base for every overlay without copying rows. A scale-level explicit
`base_value` (for example a host-supplied previous close) replaces both for every source, label,
primitive, trading line, and drawing on that scale, so horizontal panning never re-bases it. The same anchor resolution
feeds the bounded engine-owned comparison legend snapshot, so browser and native hosts present
per-symbol values from one canonical time identity.

Autoscale is one engine pass per invalidated frame. Each visible series contributes its own raw
range over the strict visible bars (data with Heikin-Ashi/footprint bounds and, for histograms,
their `base` as in the reference, engine-owned native primitives while data is visible, host
series-primitive contributions) plus marker margins. A series `autoscale_info_provider`
(reference `autoscaleInfoProvider`) receives that info and its
answer replaces the series' range and margins; hosts record no state for it and it runs during the
pass. The browser package runs it inside a render-callback guard at its single WASM boundary, so a
chart API called from the provider throws `unsupported_operation` instead of re-entering the
borrowed engine (which would otherwise abort the WebAssembly instance from resize-observer frames),
and a throwing provider leaves the series' own info for that pass. Stable scales additionally merge each series' range one bar beyond both visible edges. The
merged logical ranges, the scale's center converted through its base, and the formatter source's
`min_move` then go to `PriceScaleCore`. Axis ticks, the horizontal grid, and axis-width negotiation
share one tick-mark set built by `ChartEngine::scale_tick_marks`: the formatter source's
`min_move`, or its tick ladder's grid (the LCM of the band ticks) over each price interval the
builder spans, with the reference's fixed `0.01` grid on percentage/indexed scales. Tick,
crosshair, last-value, and price-line labels of a laddered series round to each price's band tick;
trading snapping, keyboard steps, and tick indices on that scale use the same ladder in place of
the scalar instrument tick, so an intent's `price_tick_index` then counts cumulative band ticks
from zero. Indexed-to-100 labels use the reference fixed two-decimal formatter.

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

Line runs break at period boundaries, not at whitespace (whitespace rows connect, as in the reference). Line, area, and baseline geometry, built-in series and indicator line outputs alike, starts a new run wherever two consecutive drawn rows have different period keys: a series with the host `break_on_trading_day` option keys the exchange trading day; VWAP (typical-price and amount-weighted) and pivot outputs key their trading-day session, and VWAP bands their session/weekly/monthly reset, through `VwapReset::period_key`, the same function over the same `ExchangeTime` trading day the runtime resets on. Keys are derived during frame construction and hit testing, so no break state is stored. An indicator output keys each drawn row's canonical timestamp, the time its runtime resets on; the host option keys the time axis's own bar time, which on a non-time sequence axis is the bar's open time rather than the data layer's row key. Full rebuilds, incremental tail updates, historical corrections, retention trims, and conflated row selections (keys are monotonic in time) break identically, and an exchange-time change, which invalidates every layer, moves them. Each run emits its own stroke, color runs, area fill (keeping its slice of the unbroken gradient), baseline quadrant runs, and Bollinger band fill; a run of one visible row draws the reference's one-bar horizontal segment, while a lone edge neighbour beyond the pane draws nothing (its real segments are off-screen). Hit testing evaluates each run alone, so the omitted connector is not hittable. The breaks are ordinary ordered primitives, identical on every executor, and a series without a period keeps its single-run geometry primitive for primitive.

The official advanced-series examples are engine-owned feature series, not browser drawing callbacks. Each retains its complete validated payload beside an OHLC-shaped canonical projection used by the shared time/price-scale and query machinery. Grouped bars, heatmap, HLC area, pretty histogram, background shade, stacked area/bars, and whisker boxes construct backend-neutral primitives in the same ordered series layer as built-in geometry. Their official defaults, visible-range rules, pixel snapping, autoscale semantics, and source-data lifecycle are therefore identical in browser and native hosts. Brushable Area is deliberately not an advanced-series data type: it is an ordinary built-in Area series plus transient engine-owned range styling, so data ingestion, retention, LOD, hit testing, price-scale ownership, and all ordinary Area APIs remain on the canonical Area path. The legacy browser input name `brushable_area` is only a compatibility alias and normalizes to `area` immediately. Area-like fills share one design token (`market.area_fill_strong_alpha` → `area_fill_faint_alpha`): an unset Area fill, both unset baseline halves, and the brushable range defaults all derive their gradient from their own stroke color at that strength, strong at the series extreme and faint at its base. Brush default styles are engine-owned (`area_brush_defaults`); hosts send only the fields they override plus each range's positive/negative tone.

Professional footprint / numbers-bar data has a chart-level tick-truth owner described in
`Footprint.md`. `ChartEngine::add_trade_stream` retains one bounded keyed canonical microsecond tape;
footprints, CVD, delta and volume histograms, and bounded large-trade bubble markers hold dependent
handles, not provider-event copies. The stream derives integer tick-grid levels, bid/ask/unknown/total volume, POC,
final/session delta, delta percentage, running Max/Min Delta, and diagonal stacked imbalances. CVD
supports session, continuous, and anchored resets, and every dependent carries the stream revision
through tip, correction, and retention updates. Stream telemetry attributes retained tape capacity
and dependent rebuild work. The stream is the only writer of its dependents: a trade-bound
candle/bar and the CVD, delta, and volume studies are source-owned like a footprint, so every host
data write (install, update, batch, merge, sequenced update, per-point colors, pop) is refused at
the shared engine layer and reaches wasm, TypeScript, native, and GPUI unchanged. The stream's own
tip and rebuild paths write through the unguarded `install_series_data_inner` and
`update_series_bars_sanitized_inner` internals. A series has one engine writer
(`ChartEngine::series_owner`, computed from the footprint, synthetic, resampling, trade-bar, and
trade-study registries, so removal never leaves a stale answer), and every attach path
(`bind_trade_bar_series_to_stream`, `configure_footprint_series`, `configure_resampled_series`
targets, `configure_synthetic_bar_series`) checks it before mutating anything; without that check
the unguarded internals would let two writers share one series.
Time bars align to the stream's `anchor_micros` grid unless the host anchors the stream to
exchange sessions (`set_trade_stream_sessions`): the stream then owns a `SessionBarGrid` in the
chart's exchange time (calendar-date flag cleared), so ordinary trade-bound candles open at every
window open (A-share 60-minute bars at 09:30, 10:30, 13:00, 14:00) and on exchange hours across
DST. Configuring sessions validates and rebuilds a candidate stream once; a chart exchange-time
change re-places every session-anchored stream once. Changing a footprint's aggregation
(`apply_footprint_series_options`) re-aggregates its stream in place, keeping the retained tape
(hidden prints included), replay clock, retention seed, and session anchoring, re-placed on the new
interval; bound candles/bars follow it, and a switch onto the sequence axis is refused while
resampling or synthetic bars share the chart. Excluded prints still feed aggressor
classification but no bar or session delta. The chart clock also masks time-domain rows after it,
so a bar opened ahead of the clock by a folded pre-open print appears when the clock reaches its
open. A volume trade study (`add_trade_volume_series`) projects each derived bar's total volume
into a `histogram_updown` histogram with the same tip, correction, replay, and retention lifecycle
as the delta histogram, so hosts need no second volume source for tick-built candles. On time bars
a large-trade bubble carries the open of the bar holding its print
(`FootprintAggregator::print_bar_time`: markers snap an off-grid time to the next bar, and folded
auction or lunch prints are stamped outside their bar) while an id-less marker keeps its print's
own second as its name; a print the session policy excludes has no bubble. Non-time bubbles sit on
their bar's row key. Rebuilds and live tips run the same fold, so both place bubbles identically.
Live tip events update only the active derived bar. The footprint projection advances first and
writes only the changed bar suffix straight from the stream, also under a `max_points` ceiling;
trade-bound candles/bars project the same suffix, CVD/delta/volume studies recompute only that
suffix (CVD resumes a cached running fold that steps exactly like a clean rebuild), and bubble
markers advance a resumable fold over the newly appended trades only. The fold sizes only new or
merged bubbles, rescales every retained marker only when the peak bubble volume changes (a
sliding-window maximum tracks it), and does no work at all when the stream has no bubble
dependents. Nothing on the tip path clones the retained bars or tape, and a tip's result equals a
clean rebuild of every dependent, on session-anchored bars too. The tip that crosses a retention
ceiling (once per hysteresis margin) evicts complete bars from the stream front together with
exactly the trades they aggregated, counted per bar, and reconstructs nothing. Counting subsumes a
bar-key cutoff on session-anchored bars: a bar counts the prints it folds, so a folded
opening-auction print stamped before the bar's open leaves with its bar, and the walk steps over
excluded prints (which join no bar), evicting those stamped before the first retained bar's first
print with the evicted history. The tape is a deque, the trade-ID index keeps absolute positions so
only evicted IDs leave it, replay checkpoints inside the retained suffix are re-addressed, and each
bubble fold drops only the bubbles made of evicted trades (refolding once only if a merged bubble
straddled the boundary). Every presentation, including every other footprint bound to the stream,
then drops exactly the rows keyed before the footprint's first retained row, all of them (the
footprint's own rows included) in one data-layer transaction, so the timestamp union merges and
every plot reindexes once however many presentations the stream has. The indicators and
resampled series reading a trimmed presentation recompute from its retained rows, as after a
retention trim of that series itself. Beyond work
proportional to the evicted trades, only that data-layer trim and renumbering the retained bars
scale with the retained rows; nothing scans the retained tape. The stream carries the evicted bars'
cumulative delta, and an anchored CVD the base its evicted bars established, so evicting history
never rewrites retained CVD values. Stream stats expose `dependent_rows_computed`,
`bar_rows_projected`, `bubble_trades_scanned`, and `bubble_markers_sized` as lifetime work
counters. Tape replacement, session changes, late-event, correction, and replay-seek paths rebuild
each dependent once; a bubble refold materializes markers only for the retained bubbles, so it
never holds more than `max_markers` markers. A late-event or provider-correction batch merges
atomically into the final canonical tape, validates its final session/bar projection, and reconstructs
exactly once. Time-bar rows are keyed by bar open, so every tape path (replacement, tip, correction,
session or aggregation change) rejects a tape on which a session change falls inside one bar
interval (`ProjectionTimeCollision`) before anything changes. The check covers prints the replay
clock still hides, so no clock move has to refuse a reveal; a tip checks only its batch against the
tape's last bar key.
Each derived bar also carries an engine-owned logical index plus its full-resolution open and close
microsecond times. `FootprintAggregator::bar_sequence` exposes those bounds without collapsing them
to whole-second labels, so several non-time bars in one second and long gaps remain distinct.
Chart-integrated trade-count, volume, and range footprint projections use chart-local row keys plus
an engine-owned sequence sidecar; axis labels, crosshair lookup, and visible ranges resolve against
the sidecar's full-resolution open times, never synthetic UTC timestamps. `BarSequenceMapping`
matches ordered full-resolution bounds and rebases logical anchors across prepend/rebuild operations
without collapsing duplicate second labels. Ordinary candlestick and OHLC-bar presentations bind
to that same chart-level stream and consume the aggregator's canonical OHLC bars; stream-identity
replacement and live batches update footprint, ordinary bars, studies, and bubbles together without
copying or reclassifying the tape. Bound ordinary bars reject independent retention caps because all
presentations in a non-time domain must retain the same logical rows. Non-time tip updates
replace only the affected suffix, also under retention: the sidecar changes in place before the rows,
so a trim inside the update drains the matching prefix. The shared time sync reads the sidecar's open times in place instead of materializing a
tick-time column, and extends tick weights incrementally only for a pure tail append. Derived delta studies and trade-bubble markers use the same logical row keys, continuing from the projection's first retained key after a trim, and a footprint, candle/bar, or study bound after a trim installs from that key too. Bubble aggregation windows compare
the original microsecond trade times; value snapshots and series queries resolve their time labels
through the same sidecar. Trading executions, host events, and round-trip geometry resolve their
timestamp anchors through the same index helper. The sidecar is retired when the last live
non-time footprint, candle/bar, or study dependent leaves the chart,
preventing stale sequence labels from affecting later time series.

Level-two depth uses the parallel chart-side projection boundary documented in `Depth.md`.
`ChartEngine::add_depth_stream` owns one keyed, bounded book per host instrument publication. A
validated full snapshot establishes sequence and tick-grid identity; incremental updates are
atomic, and a gap fences all later deltas behind a typed resync request until the host supplies a
new snapshot. The current bid/ask maps, optional per-level order counts, time-bucketed history,
host-detected microstructure events, replay tape, and checkpoints have explicit independent caps.
DOM ladder rows, cumulative curves, imbalance, and time-and-sales are disposable read models over
the canonical depth or classified trade stream, never additional mutable books or tapes.

Liquidity heatmaps retain immutable 32-column RGBA chunks plus one replaceable one-column live
edge. Fixed absolute bucket alignment lets history eviction rebuild only affected edge chunks while
stable image keys preserve executor caches. The ordered frame emits the same `Prim::Image` contract
to Canvas2D, WebGPU, native, and GPUI, followed by ordinary trade/series geometry and explicitly
bound, capped microstructure markers. Hosts own provider decoding, recovery, event detection,
tooltips/panels, and the Terminal's authoritative market book; this engine state is a bounded chart
projection of those publications. The typed WASM boundary uses parallel numeric arrays, split
high/low words for exact `u64` sequences, an optional aligned label vector, and decimal strings for
exact sequence/trade identifiers returned to JavaScript.

The chart owns one host-supplied replay clock in microseconds and applies it to every canonical
trade stream, depth projection, and the ordinary time-domain data layer. Source rows and future
events remain retained once while series queries, studies, footprint cells, heatmap buckets,
depth markers, sparse stepped releases, trading
executions, round trips, and host-event geometry expose only the eligible prefix. A host window
crossing the clock is clipped; a wholly future window is omitted. The ordered frame draws one
engine-owned dashed replay cursor in every pane, so Canvas2D, WebGPU, native, and GPUI executors do
not reconstruct replay state. Non-time dependents use their shared sequence projection rather than
interpreting logical row keys as UTC seconds; arbitrary independently timed series are not valid on
that domain.

Moving the clock forward applies newly revealed canonical events through the ordinary live path,
classifying each at reveal time against its canonical predecessor, so an unknown-side trade ingested
or reconstructed behind the clock uses the same tick rule as a fresh load. Backward trade seeks restore the nearest retained aggregation checkpoint, replay only the reported
suffix, and produce the same bars as a fresh load to that clock. Checkpoints are recorded every
1,024 eligible trades and capped at 64 per stream; an older seek starts from the retained tape's
rebuild seed. Depth uses the same 1,024-event interval and 64-checkpoint cap, restores the nearest
book snapshot, and replays only the reported suffix; its ladder, studies, heatmap, and marker
queries all read the replay projection. Retention keeps the checkpoints inside the surviving suffix, re-addressed to it. A seek exposes the new clock's ordinary rows before trade-stream projections reinstall, so their retention ceilings count the rows a clean rebuild counts. An ordinary series' `max_points` likewise counts and evicts only the rows up to the clock, so it holds what a clean load to that clock holds, and a seek trims revealed rows the same way; rows ingested past the clock are pending source truth, retained uncounted until the clock reveals them, so hidden rows never push revealed ones out. A clock move refreshes each indicator from its dependencies' previous visible length (a forward move costs the revealed rows, like a tail append) and rebuilds it after a backward move; a dependency its owner rewrote during the move (a trade-derived or resampled series) already refreshed its bindings. Ingest wholly beyond
the clock changes only source truth and performs no dependent work; a correction that moves a
revealed print past the clock hides it and refreshes every dependent. The existing columnar
`update_typed` path is the bulk ordered bar boundary, while trade batches cross as parallel typed
arrays and update all stream dependents once. The release `perf_gate` advances a shared
footprint/candle chart through 6,000 recorded seconds at 100×, builds every frame, and requires
steady-state retained memory not to grow across complete passes. Its Target D also streams 9,000
single-trade tips into a retained 250,000-trade footprint with bound candles, CVD, delta, and
bubbles, requires the work counters to stay within the changed suffix and the new trade with no
tape reconstruction, and budgets the tip p99 and the slowest (retention-crossing) tip, which must
run one union merge and one reindex for every presentation. Its report-only Target D2 prints the
data-layer retention trim across series counts and retained rows.

Renko, Line Break, Kagi, and Point & Figure are engine-owned price-action transforms over one
canonical host OHLC source. Fixed-box Renko requires a two-box reversal; ATR Renko uses Wilder true
range and begins only after its configured warm-up; Line Break compares a source close with the
high/low of the last configured lines; Kagi reverses only by its configured absolute amount; Point
& Figure uses fixed boxes and a configured reversal count. Ordered tip input updates only the
active transform state, while current-source replacement rebuilds deterministically and is tested
against the incremental result. Source and output are each capped at 1,000,000 rows/bars and reject
overflow atomically. Their output installs through the same full-resolution bar-sequence sidecar as
trade-count/volume/range charts, so replay, labels, crosshair lookup, drawing rebasing, indicators,
and every backend share one logical identity. A chart permits only one independent non-time source;
derived indicators may share it, but another synthetic transform, arbitrary independently timed
series, or a non-time trade stream must use another chart. Renko and Line Break use canonical candle
or OHLC-bar geometry, Kagi lowers to shared horizontal/vertical line primitives, and Point & Figure
lowers bounded X/O text (with a dense-column line fallback), so executors contain no transform math.
Synthetic market source remains host-owned and is intentionally excluded from chart-state
persistence, consistent with every financial series definition and market-history payload.
The configured tick size owns the series min-move/formatter and the shared autoscale, frame, and hit
paths use complete row bounds (whole `ticks_per_row` rows padded half a tick) on the series' ordinary
pane-local price scale. Any series may carry a `render_before_time` cutoff: rows at or after it keep
their data, scale participation, and last-value chrome but are not drawn, so a host can hand a
price series' tail to a live footprint without a second price model.
Footprint bars ultimately emit the same ordered `ChartFrame` as every other series, and no backend
may infer order flow from OHLC or recalculate footprint math.

OHLCV resampling (`configure_resampled_series`) derives one candlestick/bar target and an optional
volume histogram from a candlestick/bar source and its volume histogram over ordered, disjoint UTC
boundaries (at most 32 bindings and 20 000 boundaries; hosts pass their own or derive them with
`resample_boundaries`). Buckets restart at each boundary, rows outside every boundary are omitted,
and whitespace rows reserve their bucket without prices (an all-whitespace bucket is a whitespace
bar). Targets are source-owned, like trade-bound candles and bars, trade studies, and synthetic
bars: every host write path (install, update, typed batches, and merges) is rejected, and a
footprint, trade-bound, trade-study, or synthetic series cannot be a target. Resampling buckets UTC
seconds, so it and a non-time bar sequence (trade-count, volume, or range streams, synthetic bars)
never share a chart axis; whichever arrives second is rejected. A
source or volume mutation reports its first changed row like an indicator change; bars whose bucket
closes by the last unchanged row's time plus one second are kept, the rest is rebuilt from the first
affected bucket and reaches the target through the ordinary tail-update path, so a live minute costs
one bucket of rows, and the scan stops at the last source row rather than visiting boundaries
configured ahead. With the source's row count unchanged (a tick filling a pre-installed session
slot), rows after the last priced row before and after the change are whitespace both times, so
the scan also stops at the bucket holding that row and the reserved whitespace buckets after it
are kept rather than rebuilt and rewritten. Resampling configuration refuses chained bindings in
either configuration order (a target may not be another binding's source or output) and a binding
whose volume source is its own volume target; a reconfigure keeps its source and may name its own
targets again. A pop, a retention trim of the source head, a tail that loses a bar (backward
replay), and a complete source replacement rebuild once. Replay-clock moves refresh from the earlier
cutoff, so a forming bar never aggregates rows after the clock.
`resample_stats` reports rebuilds, tail refreshes, and rows scanned. Resampling configuration, like
every series definition, is runtime-only and outside chart-state persistence.

The upstream heatmap-around-line and background-shade examples are compositions: the specialized engine series is ordered beneath an ordinary line series rather than duplicating that base-series geometry. Heatmap `cell_shader` callbacks are the one styling boundary in this group; the browser evaluates the callback while normalizing input, and Rust retains the resolved color with each bounded cell so every renderer executes the same prepared frame.

Trading is a first-party engine domain, not a drawing, series, primitive, or plugin. Each chart owns host-supplied typed position, order, group, and execution identities; broker relationships and instrument metadata; semantic trading style; dedicated hit state; and a bounded intent queue. The host remains authoritative for broker state. Pointer movement changes only a chart-local snapped preview. Release emits one broker-neutral typed intent directly, and the chart offers no inline confirmation step of its own: a host that gates modifications runs its own confirmation around the intent before answering it, which keeps that policy where the host's instant-order-placement setting already lives. The chart also APPLIES the change as it emits — a closed order or position leaves the chart, a dragged line stays where it was dropped — and keeps only a rollback, so rejecting the intent restores the object exactly as it was. Nothing is parked in a pending tint waiting on an answer, because closing means the object is gone and moving means it has moved. Confirmed objects change only through a subsequent host snapshot or incremental update. Accepted previews remain visibly dotted and pending until that authoritative update arrives; rejected or discarded previews disappear without mutating the confirmed object. Trading state, previews, intents, and executions are runtime-only and never enter drawing persistence.

The trading contract is explicitly multi-account and host-authoritative: every runtime object may carry a bounded validated account ID, and one engine-owned visible-account filter gates both rendering and hit-testing without removing hidden objects from the snapshot. Host annotations on positions and orders are capped, validated atomically, and rendered as shared chip geometry with deterministic overflow; their tone and tooltip are presentation metadata only. Trailing and break-even trigger lines use host-supplied prices, while price-bearing intents also carry an exact integer tick index when instrument tick metadata permits it. Execution markers accept circle/arrow/triangle and quantity sizing, and bounded host round trips add outcome-colored connectors and labels. Host event markers and risk windows use a separate transient overlay layer with bounded IDs, deterministic pixel-column LOD collapse, and dedicated host hit results; they never enter drawings, undo history, or persistence. Linked charts use semantic crosshair and visible-time-range events carrying source and monotonic revision; external application resolves values against local data without re-emitting, preventing echo loops. A crosshair event carries the pane picked from the chart-content `y` and a price on that pane's default scale (a series on another scale, or on another percentage/indexed base, is re-expressed from the crosshair `y`), and applying it converts on the same scale and holds the line inside the requested pane.

Price alerts use the same host-authoritative boundary. The engine retains at most 4,096 typed alert-line indicators and paints them through the canonical pane/axis frame; it does not evaluate conditions, persist alerts, enforce account limits, run background timers, or deliver notifications. An alert line's price tag always shows its formatted price, exactly like every other axis tag; an optional host label remains metadata and never replaces that price. What names the line visually is a badge chip attached to the tag's pane-facing edge, carrying a bell drawn from prims rather than a font glyph, rounded on its outer edge and square against the tag so the pair reads as one control. Active alert chrome derives from the theme-aware muted-text token rather than the primary blue accent; triggered and expired states retain warning and darker-neutral colors. The crosshair price label exposes one engine-rendered multipurpose action chip on its primary price scale: an attached button, rounded on its outer edge and square against the tag with no radius on the tag side, carrying the original circular-plus SVG from `packages/charts/src/assets/icons/add.svg`. Its alpha masks for integer sizes 1 through 96 are generated by the pinned browser rasterizer (`node examples/web_demo/build_crosshair_icon.mjs`, with `--check` for verification) and embedded in `aeris_charts_render` as a bounded run-length asset. Each engine retains only the current size as immutable RGBA pixels; an axis frame shares those pixels and the shared converter emits an integer-aligned image primitive for every backend, including workers. Font/DPR changes select the matching mask; device recovery reuses the retained image. No runtime SVG parser or renderer-specific icon shape is involved. The chip stays visible whenever the crosshair is; hovering it lifts the fill a step with no blue fill. Activating that exact hit zone emits a bounded chart-level action request carrying pane, scale, and price; the browser package forwards it to host subscribers so the host can offer alert, limit-order, horizontal-line, or other context-appropriate actions. The request does not choose an action or carry alert defaults. Alert metadata represents the regular-price `crossing`, directional crossing, greater/less operators and the `only_once`/`every_time` frequencies, plus interval-dependent per-bar, bar-close, and per-minute frequencies. These values are display/configuration metadata only until the host returns an authoritative line snapshot or update. Alert lines and pending action requests are runtime-only and never enter chart persistence.

Official primitives with chart semantics are likewise retained by the engine. Series primitives follow the source across panes and own their bounded data, hit state, autoscale contribution, and pane/axis views; pane-only primitives retain a stable `PaneId`. Delta Tooltip is a non-candlestick interaction: the engine rejects attachment to candlestick series and removes an attached Delta Tooltip if a convertible built-in series later becomes candlesticks, while the ordinary Tooltip remains available for candle inspection. The ordinary Tooltip snapshot is a structured bar inspector rather than a one-value DOM guess: Rust resolves the exact hovered source row and returns its retained Open/High/Low/Close for candlestick, bar, area, line, baseline, histogram, and other ordinary presentations. Scalar host rows already normalize the same value through all four canonical columns, so scalar area/line data has coherent OHLC while an area/line presentation over retained OHLC can inspect the complete bar even though it paints Close. Optional volume is explicitly host-associated through a timestamp-aligned `volume_series`; the engine never guesses which independent histogram means volume. Tooltip chrome reads the chart's resolved surface, foreground, muted text, border, and font at the browser boundary so light/dark theme changes cannot drift from the chart. Brushable Area composes an ordinary Area series with Delta Tooltip and transient `SeriesEntry.area_brush` presentation state. While that helper is attached, primary mouse/pen pane-drag belongs to the comparison gesture instead of starting a competing canvas pan; price/time-axis drags and manual scale unlocks remain the ordinary Area behavior, and the helper never globally changes `handle_scroll`, `handle_scale`, or chart crosshair options. One-finger touch can still pan normally, while the Delta Tooltip's existing two-point touch interaction remains available. Delta Tooltip owns its own comparison guides, and clearing/detaching the interaction simply drops the transient brush state so the untouched Area renderer is restored. A browser may decode an image or evaluate a user-supplied color/format callback at the platform boundary, but it sends the bounded result back to Rust. The engine stores image watermarks as RGBA8 `RasterImage` values and emits one shared `Image` primitive; Canvas2D, WebGPU, GPUI, and native executors only upload/cache and paint that prepared image.

Session highlighting evaluates its optional fractional-hour gate and weekend test in exchange wall-clock time. Its callback records are merged at the tail: a live update sends only appended rows, the engine drops records for rows that retention evicted from the source's head, and it accepts the merge only while the record count and both endpoints still align with the source, otherwise the host re-sends the full aligned set once. The candle-close countdown shows only while the host clock lies inside the last bar's interval (for calendar-date bars, the exchange trading days of its date, or its calendar months for monthly and longer bars) and hides outside it; hosts supply the clock, and the engine never reads one.

An indicator binding keeps its public definition, compact private runtime, and ordinary canonical output series separate. Its runtime covers the source through the data end (one past the last real row, kept at least as far as the previous rebuild's), so trailing whitespace rows such as pre-installed session slots are neither evaluated nor rewritten: their outputs stay whitespace, recursive states resume from the retained tail state when the forming slot fills or changes, and `DataLayer::update_single_aligned_within` writes only the changed rows of an aligned output, so filling or revising a slot costs the window like an append. Sparse runtime checkpoints are tied to source row positions and to the source and optional volume/turnover-series generations. Volume and turnover columns pair with source rows by timestamp, so a change to one of them resumes at the first source row after its last unchanged timestamp rather than at its own row index. A tail mutation advances only bindings that depend on that source and installs only changed output rows; a historical mutation resumes from the nearest valid checkpoint and replaces the affected output suffix, while truncation or complete replacement performs a clean rebuild. The five-output EMA ribbon is one binding with one independently checkpointed recursive EMA state per configured period. Its atomic period update retains all output identities and presentation, rebuilds the five value columns once, and propagates the resulting changes through dependent indicators. Removed source/output series drop the binding and its runtime state together. VWAP bands use the same sparse checkpoint boundary for weighted basis, population deviation, and percentage bands, with an engine-owned session/weekly/monthly reset key derived from the chart's exchange trading day. A VWAP binding may also carry a timestamp-aligned turnover (amount) series; it then reports the 分时 average price `sum(amount) / sum(volume)` over the same reset key, skipping rows without positive volume or finite turnover. Convention presets are expanded into explicit `IndicatorKind` parameters at the host boundary, so bindings, metadata, and persistence never hold a preset name.

Indicator output metadata is additive and binding-complete: the first output's monotonic series identity is the stable binding identity; every output reports the full structured parameters, source, optional VWAP/VWMA volume source and VWAP turnover source, stable output name, index, and count, plus its warm-up and convergence rows measured on the root price source through any chained indicator sources. Native hosts may also enumerate one typed definition per live binding in deterministic creation/dependency order and recreate it through the generic `IndicatorKind` entry point (`add_indicator_kind_with_sources`), remapping source, volume-source, turnover-source, and ordered output identities as they go. This definition snapshot excludes runtime calculation state. Scalar series can also carry renderer-neutral semantic presentation owned by the engine: fixed threshold regions lower into the canonical translucent oscillator channel plus dotted boundary lines, and momentum histograms reuse the canonical four-state market palette while treating whitespace as a reset. Hosts declare those semantics but never receive or retain render primitives or palette logic. The host groups and renders legends from binding metadata; indicator values still come through the ordinary chart value snapshot. Study bindings additionally carry a typed scalar input (`open`, `high`, `low`, `close`, `hl2`, `hlc3`, `ohlc4`, or `hlcc4`) selected at the engine boundary; aggregate inputs never become canonical series: each binding keeps its aggregate column as private runtime state (counted in indicator runtime memory) and re-derives it only from the rebuild's first changed row, so a tick derives one row rather than the history. Multi-input bindings pair volume and turnover inputs with source rows by exact timestamp rather than row position. While one timeline is a prefix of the other (identical timelines, or a candle and its volume streaming a new bar in either order) the weight column is borrowed as is and only the newly shared rows are compared; a diverging timeline keeps a binding-owned aligned column with the documented fallback for missing timestamps (unit weight for VWAP, VWAP bands and VWMA, zero for OBV, CMF, MFI and volume, no trade for turnover), also re-derived only from the first changed row. `last_indicator_work_rows` sums, per binding, the formula rows and the derived input rows of its latest rebuild. Each output also exposes a compact engine-owned style snapshot (visibility, line, marker, area, and directional colors) and accepts an atomic validated style replacement, so per-output styling survives host persistence without kind-specific reconstruction. WASM hosts can query the same bounded parameter/output schema by indicator kind, so property panels do not duplicate engine definitions.

Drawing anchors, kinds, styles, pane association, stable z-order, and metadata remain the only authoritative committed drawing state (temporary hover/selection/drag/edit promotion never rewrites it). Built-in tool semantics are described by one compile-time engine catalog: stable wire/name identity, placement class, point-count rule, handle policy, movement-axis restriction, straighten behavior, semantic bounds extent, defaults, and platform-edit requests. Trend-line labels resolve their 3×3 left/center/right and top/middle/bottom positions against the actual segment rather than its bounding box; an inline middle label splits the shared stroke around its measured text extent so no backend paints through the glyphs. The catalog includes Long Position and Short Position as single-click preset tools whose committed semantic points are entry, target/width, and stop. Target and stop are normalized to opposite sides of entry, stop shares the origin edge, and editing uses four dedicated controls (target, entry/origin, width, stop) rather than generic anchor behavior. Their one-click presets open asymmetrically at 2:1 reward/risk, with fill-only entry→target profit and entry→stop loss zones, a thin neutral-gray center entry line, target/stop boundary labels only, and filled neutral/green/red owning-scale Y-axis price tags; no PnL or risk/reward summary block is painted inside the position. Position run progress is a derived three-state model bounded by the position's horizontal lifetime. Pending positions emit no progress until the first post-placement candle reaches or crosses entry; that candle becomes the progress origin at the exact entry price. While filled and active, the endpoint is the latest in-box candle's Close/current value, so the active reward/risk side follows current position state rather than a historical or current wick extreme. The first candle after fill to touch target or stop completes the run: target-first freezes at the exact target price and stop-first at the exact stop price; a same-candle target+stop touch is conservatively stop-first because OHLC cannot determine intrabar ordering. The stronger opacity covers only the traveled x/y rectangle from first-fill entry to current/terminal price, never the whole TP/SL zone or untouched empty area, and neither overlay nor connector projects beyond the position rectangle. Reward and risk use the same explicit progress-emphasis opacity, stronger than the untouched base-zone opacity, so an SL run is emphasized exactly like a TP run rather than depending on subtle repeated alpha compositing. The connector remains dashed neutral gray. LOD extrema summaries plus prefix binary search keep entry-cross and first-boundary discovery bounded for long-lived positions; an OHLC gap across entry is deterministically treated as a cross and drawn at the semantic entry level because no exact intrabar path is available. The run overlay lowers through pane chrome so series updates do not rebuild retained drawing geometry, and the boundary labels are emitted after it so the dashed trend never paints over their text. The catalog is not a runtime plugin registry; adding a built-in tool extends this deterministic engine-owned definition rather than teaching each host or renderer how the tool behaves. One chart-local `DrawingController` owns the armed tool/template plus pending anchored placement or captured freehand state. Browser and GPUI hosts forward generic press/move/release/activation/finish/cancel actions and retain only platform duties such as pointer capture, event coalescing, editor surfaces, and repaint scheduling; hosts do not branch on concrete drawing kinds to decide creation behavior.

A chart-local derived runtime maps the existing monotonic `DrawingId` values to conservative logical/price bounds, coordinate-keyed media-space anchor geometry, and pane-local z-ordered candidate lists. Candidate queries first reject drawings in semantic space, then test cached conservative screen bounds; only viewport or pointer candidates rebuild coordinate geometry and reach canonical primitive emission or precise hit testing. Tool anchors resolve into one backend-neutral drawing-geometry vocabulary before either body hit-testing or `Prim` emission, so the interactive body and the rendered body share the same segment/ray/full-line/rectangle/polyline geometry and terminal decorations (core tools through `drawings/geometry.rs`, B8 family tools through the shared part vocabulary described under "Drawing families (B8)"). Frame construction alone lowers that resolved geometry into the shared render IR; Canvas2D, WebGPU, GPUI, and native/headless executors never receive a drawing kind and cannot fork tool semantics. A dashed or dotted core stroke (trend lines, the path, the curved brush) reaches executors as solid dash runs through `push_styled_stroke` (`frame/series_geometry.rs`, beside the series lines' `push_line_stroke`): the run is expanded with its line type first, then clipped to the pane grown by the stroke's reach and split, so every executor paints the same dashes (the WebGPU stroker ignores `Polyline.style`) and a line reaching far past the pane splits only its visible reach; a solid stroke stays one polyline. General-series lines lower their dashes the same way. Full-span horizontal lines, full-height vertical lines, and half-infinite horizontal rays retain explicit unbounded dimensions rather than fake finite extents. The multi-click Path stores two to 100,000 vertices, emits one straight polyline with an open terminal chevron, exposes every vertex for editing, treats the two arrowhead wings as body-movement targets, and commits one history command only when double-click or Enter finishes it; Backspace removes the latest pending vertex and Escape discards the pending path. Brush remains a press-drag freehand placement class: its bounds are computed once on semantic mutation, padded for curved interpolation, and remain conservatively unbounded during an active capture before one exact pointer-up rebuild. Capture decimates pointer samples by distance and leaves the frame untouched when a sample is rejected, so rapid input invalidates the drawings layer only per accepted point. Runtime bounds, resolved geometry, controller state, counters, and index entries are never serialized.

When a historical insertion, removal, series replacement, or retention trim changes merged logical indices, the engine rebases every drawing-semantic logical snapshot through the data layer's common-timestamp mapping: committed anchors, pending creation anchors and preview, active brush points, active drag start and current snapshots, and both drawing-history stacks. Non-time footprint rebuilds use the bar sequence's full-resolution open/close microsecond identity mapping instead of row keys or truncated seconds, including fractional anchors between bars. Common timestamps map exactly and fractional positions interpolate between them. The core mapping also reports its common extent, whether it is a pure index translation, and the old merged union it was built from. A translation (same-interval prepend, retention trim, or window shift) keeps slope-one bar-count extrapolation outside the common extent so live drawings never jump across session gaps while retention trims stream; the one exception is prepended history, where anchors left of the old data resolve their extrapolated time on the new bars, so an anchor an interval switch placed before a short history keeps its moment when the host pages in more. Otherwise (an interval switch, or a replacement with no common timestamp) every anchor outside the exact extent resolves its time on the old axis onto the new axis (`drawings/time_anchor.rs`). Derived times and logical positions outside the persisted value range are reported as unplaceable, and a pending time that already matches its placeholder keeps that logical bit-exact, so export and import round-trip deterministically. On ordinary time axes an anchor's time identity is derived, not stored: logical `i` sits at merged time `i`, fractional positions interpolate between neighbouring bar times, and positions beyond the data extrapolate with the prevailing bar interval (the most frequent of the last 16 spacings, so session and weekend gaps are never the step). Only an anchor that cannot be placed keeps an explicit pending time on its `Drawing` snapshot: a clear-then-set parks every committed and history anchor's time (cancelling in-flight creation and drag state), and time anchors supplied by the API, a restored document, or a sync/clipboard payload before data exists stay pending until the next time-point change resolves them. A bounded flag skips that walk in steady state, and row keys of non-time bar sequences are never read as times. The transient mapping compresses its common-timestamp breakpoints to slope changes and carries the moved-out old merged union only for the one synchronization in `sync_time_points`; both are dropped there, so no second merged timeline outlives the synchronization boundary. Pixel drag/brush baselines refresh immediately for input continuity and once more after the next frame settles layout and autoscale. This maintenance mutation creates no drawing-history command. Non-time drawings persist an optional bounded anchor-time sidecar (open/close microseconds) alongside their logical/price anchors; legacy documents omit it and retain their existing behavior, while restored sidecars resolve when the host installs the matching sequence. Ordinary time charts persist each anchor's time inline (`{logical, price, time}`), and that time is authoritative on import: it resolves immediately against loaded data or stays pending until the host installs data, so restoring into a shifted history window, another interval, or the grid workspace (which imports before data) lands on the saved moments.

The time-scale view follows the same mappings. Before new points land, the engine maps the view's right border through the transaction's common-timestamp mapping (or, on a non-time axis, the bar-identity mapping) and lands the point count, base index, and right offset atomically, so a scrolled-back view keeps the same bars across out-of-order inserts, gap backfills, prepend-plus-append replacements, and retention trims. An active drag snapshot, kinetic coast, held keyboard pan, and animated scroll shift by the same rebase instead of overwriting it. The reference follow-latest rule is unchanged: while the latest bar is visible and `shift_visible_range_on_new_bar` is on (for a whitespace replacement only with `allow_shift_visible_range_on_whitespace_replacement`), the offset stays relative to the newest bar. A transaction without any common timestamp or bar identity keeps the reference first-time heuristic, compared in data-layer row units on every axis. Non-time retention computes its identity mapping against the rows that survive the cap. `set_visible_logical_range` keeps fractional borders like the reference. The opt-in `lock_visible_logical_range` time-scale option holds the visible logical range exactly across data synchronization and resizes (in-flight drag, coast, and animated motion rebase with it) and applies a range passed to `set_visible_logical_range` without the reference scroll clamps; the reference `maxRightOffset` rule would otherwise shift a full `[0, N - 1]` session one bar left while fewer than two bars have traded. Scroll, zoom, fit, and option mutations still apply their ordinary clamps and become the held range. This is the fixed full-session (intraday time-sharing) view contract; session slots are host-installed whitespace rows, generated with the engine's session-slot function. The base index (the last real bar across the union series) is found through each series' LOD pyramid, so the trailing slots add no work to a tick's time synchronization.

Each chart also owns a bounded runtime-only drawing history of the last 100 committed semantic
create, delete, anchor, style, and clear operations. Pointer-move samples mutate the active drag
snapshot without adding commands; pointer-up records one start-to-end update. Undo/redo cancels an
active drag first, rebuilds only the affected drawing runtime state, a new mutation clears the redo
branch, and persistence never contains either history stack. A host batch anchor rewrite records one
`BatchUpdate` command. Every committed history step (an API mutation, an interactive placement
or freehand stroke, a drag or keyboard nudge that changed something, a text-edit commit) advances
the chart's drawing sync revision once, as undo, redo, and price-basis changes do; a drag that ends
where it started records nothing and advances nothing. A price-basis rescale (multiplicative
factors over anchor-time segments, non-time bars dated by their open time, Long/Short Position
levels on the entry's segment, and tool options measured in price units, such as a Gann fan's or
fixed square's `scale_ratio`, on the first anchor's segment through the family's
`rescale_price_options` hook) is a data-basis change like the time rebase: it applies to locked
drawings, rewrites both history stacks and in-flight creation/drag state in the new basis, records
no command, and is rejected as a whole when any rescaled price or price-unit option would leave
its valid range. The chart-level price-basis label is metadata carried by persistence and
sync/clipboard payloads.

B2 extends that owner boundary with a versioned typed drawing contract. `drawing_contract.rs`
defines bounded property descriptors, interval visibility, line caps, magnet modes, labels,
levels, templates, clipboard payloads (with anchor times and the price-basis label, bounded like a
persisted drawing document: at most `MAX_DRAWING_OBJECTS` drawings, `MAX_DRAWING_CLIPBOARD_POINTS`
anchors, and `MAX_DRAWING_CLIPBOARD_BYTES` bytes, the byte bound checked before a paste parses;
clone stages from the live drawing, so any drawing the chart holds can be cloned), and revisioned
sync payloads. Magnet snapping resolves one effective mode from the stronger of the drawing's own
mode and the chart's persistent mode, with the Ctrl/Cmd modifier as a temporary toggle; weak snaps
only within a fixed vertical CSS-pixel distance, and keyboard nudges start at the focused handle's
own position and never snap. The live `Drawing` remains
the sole source of truth; its common snapshot and discriminated kind-option projection are
computed views, so a property panel cannot create a second state model. Patches validate all
bounded contract fields on a clone before installation and record one undo entry per semantic
change. The shared resolved-geometry path applies line extensions and is consumed by both frame
emission and hit testing. Hidden or interval-ineligible drawings remain in persistence and the
object tree but are excluded from rendering and hit testing; locked drawings remain selectable
but cannot be edited. Selection, clone/copy/paste, z-order, group operations, bulk removal, and
sync are chart-owned and bounded, with sync IDs/revisions preventing stale or echoed updates.
Named templates are validated style data rather than host-side drawing copies: a template never
carries identity (name, group, revision, z-order), placement (price scale), visibility (visible,
locked, interval visibility), or text content, neither when exported nor when applied (a
host-written template's such keys are dropped, in either spelling), and a profile template keeps
the target's own series source. V1/V2 persistence keeps
these fields optional for lossless migration of existing layouts, while browser/WASM exposes the
same schema, template, object-tree, and payload operations as the native engine.

#### Drawing families (B8)

B8 tools live in drawing families. The core tools (trend, horizontal, and vertical lines, horizontal
ray, rectangle, text, brush, path, Long/Short Position, and the profile drawings) keep
`family: None` in their catalog spec and resolve through `drawings/geometry.rs` rather than the
family part vocabulary; their dashed and dotted strokes lower to solid dash runs through
`push_styled_stroke`, as described with the drawing-geometry vocabulary above. Each family owns one module, `drawings/kinds/<family>.rs`, holding its tool specs, one
`static FAMILY: DrawingFamily` hook table that every spec references, its typed option block, and
its tests (`kinds/<family>/tests.rs`). The hook table is a closed compile-time table, not a plugin
registry. `DrawingFamily::new` takes the two required hooks, `build_parts` and `kind_options`, and
starts every optional hook at a neutral default that the family overrides by assignment in its
`static` initializer: `apply_defaults` (kind defaults applied by `Drawing::new`, so creation,
templates, restore, paste, and schema defaults agree), `decoration_extent` (conservative CSS-px
culling pad for boxes and labels beyond the anchors), `extend_schema` (`tool_options.*`
descriptors appended after the common ones, whose defaults the engine already takes from the
kind's template drawing), and `owns_labels` (the family renders the common `labels` itself). A
hook added later gets its default in `DrawingFamily::new`, so the other families compile
unchanged; a hook more than one family sets, or one the foundation adds for every family, sits
outside the family blocks. Those shared hooks are `paint_bounds`, `reads_series_data`,
`partial_preview`, `handles`, `drag`, and `close_placement` (a foundation hook; today only the
shapes polyline sets it). `paint_bounds` is the
one family culling hook: a conservative media-px box of everything a drawing paints except text,
computed from its anchors' media px whenever its coordinate key changes. A tool whose reach is
screen-derived (pitchfork tines and levels, a circle through its rim anchor, a fixed-size square)
declares `Full` logical and price extents, so no semantic box culls it, and while its bounds stay
unbounded in both dimensions this box replaces the whole pane as its screen culling and
hit-candidate box (`None`, or an extended drawing, keeps the pane). `reads_series_data` marks a
drawing whose geometry reads series data (a regression's fit, a forecast's outcome): every such
drawing measures `ChartEngine::drawing_source_series`, the first live ordinary series in creation
order (the smallest live `SeriesId`; identities are monotonic, storage slots are reused) on its
pane and price scale (indicator outputs and custom series never qualify, footprint and feature
series do through their OHLC projection, and neither paint order nor visibility moves it), and
`invalidate_frame_series` rebuilds the retained drawings layer only when the changed series is
such a drawing's source (a scan of the drawing list per data mutation; structural source changes
invalidate the whole scene). These readers work in the source's canonical rows: an as-of
source's plot points repeat and skip canonical rows, so a regression, a forecast, and a bars
pattern read each canonical row once, at the first axis point at or after its time (see
`ChartEngine::drawing_source_window`). `partial_preview` lets a placement preview resolve parts from the second
anchor on. `handles` and `drag` are the derived-handle foundation: `handles` edits the handle
set `drawings/handles.rs` builds in media px from the spec's handle mode (moving a handle onto
derived geometry, dropping one, or appending handles that drive `DrawingDragPart::Handle(index)`),
and selected-handle painting, placement previews (the placed anchors' handles only), handle hit
testing, keyboard handle cycling, and drag starts all read that one set. Every drag sample, pointer
drag or keyboard nudge, then runs the generic part drag (an anchor re-anchored with time snap,
magnet, and straighten; a body translated; a derived handle's baseline media px moved by the
delta along the movement axis, time- and magnet-snapped like an anchor) and hands the result to
`drag` as a `HandleDrag` sample (baseline anchors and px, the dragged point as an anchor and in
px, Shift, and a keyboard nudge's step, so a hook that quantizes its target, such as the fixed
Gann square's whole bars, moves at least one unit per key press). A nudge reports success only
when it recorded an undoable change. The hook rewrites the anchors from that baseline alone and may return replacement
tool options, which the drag session's history snapshot restores on cancel and records in the
same undo step; a data-driven rebaseline also rebases a derived handle's baseline onto its
current position. `close_placement` lets a multi-click tool close on its first vertex: with at
least three vertices placed, a click within the precision anchor hit radius of the first calls
the hook (a polyline sets `closed`) and commits without adding a vertex, and hovering there snaps
the preview onto that vertex; Enter, double-click, and Escape keep finishing open and cancelling.
Hooks run while the drawing runtime cache is borrowed during frame construction and hit
testing, so they read the engine but never call candidate queries or cached anchor-geometry
accessors. Spec fields replace per-kind checks in shared code: `text_layout` (`Box`, or
`Segment`: the label follows, rotates with, and takes the stroke color of the first two anchors
and a middle label splits the stroke) and `axis_price_label` (the horizontal-line axis tag).

A family resolves one drawing into the shared part vocabulary of `drawings/parts.rs` in the
caller's space (bitmap px at render, media px at hit test): anti-aliased strokes, crisp full-pixel
horizontal and vertical lines, ribbon fills between paired chains (convex polygons via
`fill_convex`, any polygon by the nonzero rule via `fill_polygon`), discs, wide strokes painted
once per pixel (`Tube`, lowered to one band fill), and boxed text blocks laid out by one
`PartLabel::layout`. Derived
logical/price points (level lines, time zones, data-driven points) map into that space through
`PartContext::point_px`, which applies the frame's separate horizontal and vertical bitmap ratios;
frame construction debug-asserts that it reproduces every anchor. Frame construction lowers
parts into existing `Prim`s in `frame/drawings.rs` (`build_family_prims`; every stroke run goes
through `push_clipped_stroke`, which clips it to the pane grown by the stroke's reach with
`shape::clip_polyline_to_rect`, so work and coordinates stay bounded however far the geometry reaches,
and splits a dashed or dotted run into solid dash runs through `push_line_stroke`, like every
engine-owned path stroke, because the WebGPU tessellator has no dash concept; clipped parts keep the
unclipped run's dash phase, so dashes never shift while panning) and precise hit testing tests
the same parts (`DrawingParts::hit`, where a dashed stroke stays one continuous body), so family
tools cannot fork executor behavior and the painted and interactive shapes cannot drift. While a
tool is being placed, the frame's pending path builds the tool's own parts once every anchor is
placed or previewed (with `partial_preview`, from the second anchor on); before that, a tool of
three or more anchors joins the placed anchors and the pointer in one guide polyline in the
drawing's stroke, lowered through `push_clipped_stroke`, with handles on the placed anchors, so
every click leaves visible ink. Shared helpers sit at their owners: pure geometry (segment
extension, midpoints, and clipping to the pane, ray clipping, parallel offsets, arc/ellipse
tessellation and clip-aware curve flattening, nonzero polygon ribbons, polyline/polygon/ribbon
hit predicates, and `Rect` inflation, intersection, and bounding boxes) in
`aeris_charts_render::shape`; line caps (`capped_segment` is the two-point `capped_polyline`, and
`cap_radius` sizes every disc and arrowhead), arrow trimming, label layout, the stats box
(`PartContext::stats_label` with the shared `STATS_*` gap, padding, and alpha and `text_on`'s
black-or-white text), and the fill convention (`PartContext::fills_hit`: region fills are body
targets only while the drawing is selected, like the rectangle's interior) in `parts.rs`; color
resolution on the contract types (`Drawing::stroke_color` with the canonical primary fallback,
`Drawing::fill_or_wash`, and `DrawingLevel::stroke_color`, `zone_fill`, and `line_style`, whose
style names fold through `line_style_from_name` like a drawing's own `style`); the property
descriptor builder `drawing_contract::descriptor`; engine-formatted measurement text (price
through the drawing scale's formatter, percent, ticks, bars, time range, duration, screen angle,
distance) and the text and stats glyph sizes (`drawing_text_size`, `drawing_stats_size`) in
`drawings/stats.rs`; the one editable handle set used by painting, handle hit
testing, keyboard cycling, and drags, with the `HandleDrag` sample and the `drawing_anchor_at`
conversion family drags use, in `drawings/handles.rs`; the media-px `PartContext::media` of hit
testing and handle hooks in `parts.rs`; and level lists as contract data
(`FIBONACCI_RATIOS`, `FIBONACCI_TIME_ZONES`, `drawing_levels_from_ratios`,
`DrawingLevel::price_between`, `DrawingLevel::label`). A drawing with `extend_left` or
`extend_right` uses unbounded semantic bounds, so extensions stay visible and hittable when the
anchors scroll away, while a ray or extended line whose extensions are switched off culls like any
finite segment.

Inline text editing is one engine session for every editable drawing, the text tool and trend
labels included: `ChartEngine::begin_drawing_text_edit` (refused for a locked, hidden, or
interval-hidden drawing, or one that is not `drawing_text_editable`) records the text it began
from, `set_drawing_edit_text` replaces the text live (repainting and relaying out, with no undo
step and no sync revision), and `end_drawing_text_edit` commits the edit as one `Update` undo step
and one sync revision or cancels it back to the text it began from. Undo and redo commit an open
session first (the browser editor's order); removal, clearing, a sync payload, and a restore end
it. The session is runtime-only. A family marks the label that holds a drawing's own `text` with
`DrawingParts::text_label` (lines from `first_line` on; earlier lines are engine text such as a
formatted price), filled from `PartContext::text_lines`, which keeps one empty caret line while
`PartContext::text_editing` is set, so an emptied box keeps its place. Such a family sets
`owns_text`, so the generic text pass does not paint the same `text` a second time; a family that
sets no `owns_text` gets the generic box or segment label (placed by `text_box` when set). A family drawing is
text-editable exactly when its parts, resolved as while editing, mark such a label, and
`ChartEngine::drawing_text_edit_layout` returns that label's layout in media px from the same
`PartLabel::layout` the frame and the hit test use (the lines' left edge, the first text line's
center, the line advance, glyph size, weight, italics, and painted color), so the host's caret
overlay cannot drift from the painted text as the box grows in any direction.

Family-specific options live in `Drawing.tool_options: DrawingToolOptions`, one optional block per
family carried under `tool_options` by options JSON, templates, clipboard and sync payloads, and V1
persistence. Patches deep-merge (absent keys keep their values, `null` resets a block) on a copy
that is validated and size-bounded before one undoable install. A template that carries
`tool_options` replaces the drawing's family style instead (`DrawingToolOptions::replacement_patch`
against the drawing's `template_style`), so applying it also resets the family options it leaves
at their defaults, while data the drawing captured (a bars pattern's copy) is not style and
stays. Persistence writes each optional style field only when it differs from the kind's own
defaults, so a restored ray keeps `extend_right` and a user-cleared one stays cleared.

Each family documents its semantics in its own block below.

<!-- B8: lines — begin -->
The Lines family (`kinds/lines.rs`, wire ids 32..=47) delivers `ray`, `extended_line`,
`info_line`, `trend_angle`, `cross_line`, and `arrow_line`. The five segment tools share one
geometry: the first two anchors extended beyond the first by `extend_left` and beyond the second by
`extend_right` (a ray and an extended line are these defaults), clipped to the pane in their own
direction, with end caps on the ends that are not extended. Visible `labels` render as one stats
box whose position is `tool_options.line.stats_position`; the info line enables price change,
percent change, bar count, duration, and angle by default. The trend angle adds a dashed horizontal
reference, the arc to the segment, and the screen angle. The cross line is full-span crisp
horizontal and vertical lines with the horizontal line's axis price tag.
<!-- B8: lines — end -->
<!-- B8: channels — begin -->
The Channels family (`kinds/channels.rs`, wire ids 48..=51) delivers `parallel_channel`,
`regression_trend`, `flat_top_bottom`, and `disjoint_channel`. The three-anchor channels share one
construction: the first two anchors are the base line, and the second line spans the same bars
(vertical sides) on the line through the third anchor — translated vertically in px (parallel on
every scale mode), horizontal at the third anchor's price, or with the base slope mirrored. Their
price extent is `Full` except flat top/bottom's, because the second line's ends can leave the
anchors' price box. Fills run between two lines over a common parameter span, split at the one
crossing so each piece is convex, and are clipped to the pane with `shape::clip_polygon_to_rect`;
they are body targets only while the drawing is selected. The family's shared `handles` hook
moves the anchor handles onto the painted lines (the third anchor's handle to the second line's
midpoint, the regression's to its fitted line's ends), so handle painting (placement previews
included), handle hit testing, and keyboard nudges read them there, and each handle still drives
its own anchor by pointer deltas.
Shift straightens a dragged base-line end against the other like a trend line's (the anchor drag
straightens the first two anchors of any tool with a straighten mode). With `partial_preview`
set, placement previews the base line while the second anchor is placed, then the whole channel
through the pointer.

`regression_trend` fits its source series (`drawing_source_series`) over its canonical rows
between its rounded anchor bars in one allocation-free pass of shifted sums
(least-squares slope, sample residual deviation, Pearson's R). `RegressionMemo`
(`DrawingChartSettings::regression_memo`) keeps each drawing's latest fit keyed by the series, the
merged points' positions (`time_index_generation`; an as-of source also keys every time point),
the replay clock, the bar range, and the source, together with the source data generation it read
and the sums of every fitted row but the last. The engine reports each series data change it
routes to indicators (`update_indicators_after_change`) to the memo with its first changed row, so
a change that leaves every row before a fit's last row untouched (a live replacement of the latest
bar, or appended bars) extends the fit by the changed rows, adding them in order so the result is
bitwise identical to a full pass; a change at an earlier row, an unreported change, a moved point,
or an as-of source pays the pass proportional to the anchored range. Panning, zooming, and pointer
hit tests reuse the fit however many regressions the chart holds, and the release `perf_gate`
Target N times live ticks plus frames with five regressions across a 1,000,000-row source. Fits of
removed drawings are dropped once the memo exceeds the drawing count by 16 entries. Its anchors
move along time only, and it sets the shared `reads_series_data` hook. The optional `tool_options.channel` block stores only fields that
were set; per-tool defaults resolve at use.
<!-- B8: channels — end -->
<!-- B8: fibonacci — begin -->
The Fibonacci family (`kinds/fibonacci.rs`, wire ids 64..=73 of 64..=95) delivers
`fib_retracement`, `trend_based_fib_extension`, `fib_channel`, `fib_time_zone`,
`trend_based_fib_time`, `fib_speed_resistance_fan`, `fib_speed_resistance_arcs`, `fib_circles`,
`fib_spiral`, and `fib_wedge`. Every tool but the spiral paints the common level list
(`Drawing::levels`, defaults set by `apply_defaults`): visible levels sort by value, bands between
neighbours take the upper level's fill under the `fill_enabled` switch, and labels take the level
color. The drawing's own stroke is the auxiliary line (trend line, fan grid, wedge edges, or the
spiral). Price levels (retracement, extension, channel) are computed in price space, or log space
with `tool_options.fibonacci.log_scale`, and mapped through `PartContext::point_px`, so they sit on
exact prices on every scale mode; time levels interpolate the anchors' px because the time axis is
affine in logical position; the fan, arcs, circles, spiral, and wedge are screen-space geometry
from the anchors' px. Band fills are body targets only while the drawing is selected.

The family adds the `bounds` hook (default in its block of `DrawingFamily::new`), which returns a
drawing's complete semantic reach (`FamilyBounds`: logical and price ranges,
`None` when unbounded) and replaces the anchor-derived culling bounds in
`DrawingBounds::for_drawing`, so levels beyond the anchors stay visible and hittable while the
anchors scroll away; screen-space tools keep unbounded spec extents and skip curves outside the
pane. Arcs, circles, and the wedge set the shared `paint_bounds` to the anchors plus the square
around their center reaching the largest visible level's radius (at least the unit), and their
`decoration_extent` pads it by the level labels, so a ring tool far from the pane culls and skips
hit testing like any finite drawing; the fan's rays and the spiral reach the pane edge and keep the
pane. It sets the shared `partial_preview`, so a three-anchor tool shows its first leg before its
second click. Level lists persist only when they
differ from the kind's defaults, so a user-cleared list stays cleared.

Every emitted coordinate stays within the pane's reach whatever the level values: fan rays end at
the pane edge, channel lines that miss the pane are skipped, and ring radii beyond the farthest
pane point close bands and wedge edges at that distance. Fan wedges and channel bands clip the
pane polygon by half-planes (`aeris_charts_render::shape::clip_to_half_plane`), so an extended
channel band covers the pane corner its lines leave through. Labels are culled by their own box
(one glyph size per character), not by their level line, so a label whose line sits just outside
the pane still paints. The spiral grows by φ per quarter turn from a sub-pixel radius until its
radius passes the farthest pane point, at most 128 quarter turns. Arcs, circles, wedge arcs, and
spiral turns skip what cannot reach the pane and, when their center lies outside it, tessellate
only the angular window the pane subtends, so curves far beyond the pane stay within the curve
tolerance at the capped segment count; one tool's arcs share a single table of unit angles, which
also pairs its band chains. A windowed dashed or dotted curve starts at the last dash-period
boundary before the window (arc length from the curve's own start, off the pane), and a full
circle restarts its pattern where the whole circle does, so dashes stay put while the pane
scrolls.
<!-- B8: fibonacci — end -->
<!-- B8: pitchforks_gann — begin -->
The Pitchforks & Gann family (`kinds/pitchforks_gann.rs`, wire ids 96..=127) delivers
`andrews_pitchfork`, `schiff_pitchfork`, `modified_schiff_pitchfork`, `inside_pitchfork`,
`pitchfan`, `gann_box`, `gann_square`, `gann_square_fixed`, and `gann_fan`. A pitchfork resolves
one frame from its anchors A, B, C: the median pivot (A; Schiff: A's time at the midpoint of A's
and B's prices; modified Schiff and inside: the midpoint of A and B), the base center (the
midpoint of B and C; inside: C), and the half handle (toward C; inside: back to B). Level `v` of
the drawing's `levels` is the pair of tines parallel to the median through `center ± v · half`,
so level 1 passes through the handle ends. Unextended lines reach one median length past the
base; `extend_left`/`extend_right` run every line to the pane edge, and extended zone fills clip
to the pane through `shape::clip_polygon_to_rect`. The shifted-pivot variants add a dashed A–B
guide, and the pitchfan draws the levels as rays from A through the Andrews base. The Gann box
divides its corners' box by the price `levels` and `tool_options.gann.time_levels`, with zone
fills, ratio labels on all four sides, and optional `angles` from the pivot corner. Like the
rectangle's interior, every zone fill of the family (pitchfork strips, fan sectors, Gann box
zones, square arcs) is a drag target only while its drawing is selected. The Gann square draws
the `levels` grid, the `angles` fan, and quarter-ellipse `arcs` around its pivot corner, plus an
engine-formatted price range, bar count, and price-per-bar box. The fixed square is one anchor
plus `size_bars` and an optional `scale_ratio` (without one it is square on screen). The Gann fan's
`levels` are multiples of the 1×1 slope, which passes through the second anchor or rises
`scale_ratio` price per bar. Its lines are rays by default, labeled `8x1` through `1x8`. Every
tool of the family reaches past its anchors by a viewport-dependent amount, so the specs declare
`Full` extents and the family's `paint_bounds` resolves the same corners and line ends in media
px (an extended drawing keeps the pane). Crisp horizontal and vertical lines clamp to the pane
with the dash-phase rule of `push_clipped_stroke` (executors dash them from their start pixel by
pixel), and label boxes off the pane emit nothing, so an extreme level, size, or zoom never grows
frame work with the geometry's length or leaves non-finite coordinates. Through the shared
`handles` and `drag` hooks, the pitchforks and the pitchfan add a fourth handle on the base
midpoint of B and C, which translates both by the midpoint's (magnet-snapped) move, and the fixed
square adds its far corner, which resizes it: the corner's time sets `size_bars` in whole bars
(1 at least) and its side of the anchor sets `reverse`; with a `scale_ratio` its price sets the
ratio (Shift keeps it), and without one the square stays square on screen, sized by the corner's
larger distance from the anchor. A pointer rounds the side to the nearest bar; a keyboard step
moves it at least one whole bar the way the key moved (sized by the moved axis without a ratio), so
sub-bar key presses accumulate. The option edit is part of the drag's single undo step. The
fan's and the fixed square's `scale_ratio` is price per bar, so the family's
`rescale_price_options` scales it with a price-basis rescale of the pivot anchor.
<!-- B8: pitchforks_gann — end -->
<!-- B8: projection_annotations — begin -->
The Projection & Annotations family (`kinds/projection_annotations.rs`, wire ids 128..=159)
delivers `forecast`, `bars_pattern`, `price_range`, `date_range`, `date_and_price_range`,
`projection`, `anchored_text`, `note`, `price_note`, `callout`, `comment`, `price_label`,
`signpost`, `flag_mark`, `arrow_mark_up`/`down`/`left`/`right`, and `icon`. Its options live in
`tool_options.projection_annotation` (bars-pattern mode, mirror, flip, and copied bars; icon and
icon size). It adds four hooks to `DrawingFamily`, each neutral by default: `on_create` (called
before a new drawing is stored, by the armed tool's placement commit, which always captures since
its options come from a tool template, and by `add_drawing`, which paste also uses and which keeps
state its options already carry; sync and restore never call it), `owns_text` (the generic text
pass skips the family's tools; they lay the common `text` out in their own parts),
`pane_anchored` (per kind), and `reveals_on_focus` (a drawing that paints some parts only while
hovered, selected, or edited; see the note below). A pane-anchored kind (`anchored_text`) stores its anchor as pane
fractions (`logical` = x / pane width, `price` = y / pane height): every anchor conversion goes
through `ChartEngine::drawing_anchor_px`/`drawing_anchor_from_px` (and `drawing_point_px`), so
frames, hit tests, handles, drags, nudges, creation, stats, and `PartContext::point_px` agree. Its
anchors carry no time (`set_pending_times` drops them, `resolve_drawing_anchors` ignores `time`,
and restore ignores a non-time anchor-time sidecar), anchor resolution and drags clamp its
fractions into `0..=1` (restore rejects a document outside that range), and it is never rebased
on data changes, price-rescaled, magnet-snapped, or offset by paste or group moves. Culling uses
full extents for it; the projection sector (a screen-px circle) declares full extents too, and the
family's `paint_bounds` bounds it by the square around its apex reaching the radius point, padded
by its stats box through `decoration_extent`.

`forecast` evaluates its outcome from its source series (`drawing_source_series`, followed
through the shared `reads_series_data` hook, so a tick of that series that moves no scale still
updates it) through the LOD extrema and latest-bar queries (logarithmic in the range): success once
a bar after the source bar reaches the target by the target bar, failure once a traded bar after
the target bar exists (whitespace rows such as future session slots are not bars). An as-of source
has no canonical-row pyramid (its LOD summarizes plot points), so its outcome scans its canonical
rows in the window once per data, axis, clock, or anchor change and is memoized per drawing like
the regression fit.
`bars_pattern` copies at most 128 OHLC bars once at creation (a longer range aggregates into 128
buckets, each read from its LOD summary rows, so the capture and the placement preview that
repeats it per frame stay logarithmic in the range; an as-of source's buckets scan its canonical
rows, bounded by the capture window), pins its anchors on the copy's box (the first
bar at the highest value, the last bar at the lowest), fits the copy into the anchors' box
(divided by the copy's full range, so small anchor drags scale it proportionally and a price-basis
rescale scales it exactly; flip turns it upside down within the box, mirror reverses time), and
converts only the columns inside the pane each frame. The ghost never leaves the anchors' box, so
it culls like any finite drawing. After its first click the projection shows the shared placement
guide; after its second it previews the sector through the pointer. Named templates carry
style only: `DrawingToolOptions::template_style` drops the copied bars, so applying a template
restyles a pattern without replacing its copy, and like every template it never carries identity,
placement, visibility, or text content (see the drawing contract above). Filled markers are single
regions: convex polygons, paired-chain outlines (arrow marks), or triangle fans around a kernel
for star-shaped outlines (star and heart icons), so paint and hit test cover exactly the same
area. Persistence compares `text` against the kind default, so a cleared default label stays
cleared. The text boxes of anchored text, the note, price note, callout, comment, price label,
signpost, and arrow marks are shared text labels (`DrawingParts::text_label`; the price note's and
price label's text follows their price line), so the host's inline editor edits them in place;
placement never opens it, since each tool starts from its default text. The flag, the icon, and
the projection and measuring tools paint no text of their own. The note paints only its pin until
it is hovered, selected, or edited, like the reference platform's note, unless
`tool_options.projection_annotation.always_show_text` is set (serialized only when set); the
family's `reveals_on_focus` hook names such a drawing, so frame construction rebuilds the retained
drawings layer when it gains or loses hover or selection, while every other hover and selection
change still only reassembles retained geometry. The culling pad counts an empty text as one line,
the caret line its editor keeps.
<!-- B8: projection_annotations — end -->
<!-- B8: patterns_elliott_cycles — begin -->
The Patterns, Elliott waves, and cycles family (`kinds/patterns_elliott_cycles.rs`, wire ids
160..=191) delivers `xabcd_pattern`, `cypher_pattern`, `abcd_pattern`, `head_and_shoulders`,
`triangle_pattern`, `three_drives_pattern`, `elliott_impulse_wave`, `elliott_correction_wave`,
`elliott_triangle_wave`, `elliott_double_combo`, `elliott_triple_combo`, `cyclic_lines`,
`time_cycles`, and `sine_line`, all `ClickAnchors` tools with one handle per anchor. Patterns are a
zigzag through the anchors with boxed point labels placed above highs and below lows; XABCD, cypher,
ABCD, and three drives add dashed connectors labeled with the conventional price ratios of their
legs (`tool_options.pattern.show_ratios`), every connector painted beneath every label, and the
culling pad measures the drawing's actual point, ratio, and wave labels; XABCD and cypher shade
their two triangles, head and shoulders draws the neckline between the outer legs and shades the
shoulders and head against it, and the triangle pattern extends its A–C and B–D sides to their apex
when it lies ahead within one pattern width (its spec pads the logical bounds by that width and
never culls on price). Elliott waves label each wave in the notation of
`tool_options.pattern.degree`; ringed degrees draw the ring as stroke geometry instead of a circled
glyph, so no executor depends on font coverage. Cycles resolve repeats only across the visible pane:
cyclic lines from the earlier anchor rightward, time-cycle arches and the sine wave in both
directions; repeats closer than 3 CSS px collapse to the defining cycle, and the arches and the wave
each stay within a 16,384-point tessellation budget. Fills are body targets only while the drawing
is selected (the rectangle convention). Every part resolves from any anchor prefix, so the family
sets the shared `partial_preview` and previews multi-anchor placement from the second anchor on.
It also adds the generic `aeris_charts_render::shape::line_intersection`.
<!-- B8: patterns_elliott_cycles — end -->
<!-- B8: shapes — begin -->
The Shapes family (`kinds/shapes.rs`, wire ids 192..=223) delivers `rotated_rectangle`, `ellipse`,
`circle`, `triangle`, `arc`, `curve`, `double_curve`, `polyline`, and `highlighter`. Every shape
resolves in caller px from its anchors, so a circle stays round and a rotated rectangle keeps its
right angles at any zoom and bitmap ratio. The rotated rectangle's first two anchors are its short
sides' midpoints and the third lies on a long side; the ellipse is inscribed in its two corners and
edits with the rectangle's eight bounds handles; the circle is its center and a rim point; the arc
runs from the first anchor to the second through the third (their chord when collinear); the curve
and double curve pass through every anchor (the quadratic's third at t = 1/2, the cubic's third and
fourth at 1/3 and 2/3), and `extend_left`/`extend_right` continue their end tangents to the pane
edge. Closed outlines start mid-edge so their butt ends meet collinearly; open strokes (arc, curves,
open polyline) carry end caps through the shared `capped_polyline`, pointing along the exact end
tangents. Fills use `fill_color` or the stroke color at 20% (the rectangle's wash), are body targets
only while the drawing is selected, and never overlap themselves: convex regions through
`fill_convex`, concave or self-crossing ones (a closed polyline, a cubic's chord region) through the
shared `fill_polygon` over `shape::nonzero_ribbon`. Curves and circles flatten through clip-aware
helpers (`shape::flatten_quadratic`/`flatten_cubic`, `EllipseArc::append_clipped_points`) that
refine only where the curve can be visible and spend at most 1,024 points, so a zoomed-in circle
thousands of px wide stays within 0.25 px on screen (a wholly visible arc takes the cheaper uniform
chords). The rotated rectangle, circle, and arc reach screen-derived distances that no semantic box
bounds and keep full extents; the curves pad their logical span by the interpolation overshoot. One
shape box serves two hooks: the family's own `text_box` gives box-layout text the shape's box (a
circle's rather than its center-to-rim anchors'), and the shared `paint_bounds` makes it the exact
screen culling box of those full-extent tools, so they reach part building and precise hit testing only
near the viewport or pointer. The highlighter is a freehand
capture painted as the shared `Tube` part: the region within half its width of the path with round
joins and caps, built by `shape::tube_ribbon` from the runs that can reach the pane, simplified
within the chord tolerance in one pass, outlined so the nonzero rule yields exactly the union, and
tessellated into non-overlapping strips. Its 40% amber therefore blends once per pixel on every
executor instead of darkening where GPU stroke triangles overlap; the tolerance doubles when a
stroke exceeds the fill bounds, and only past the coarsest attempt does it fall back to a plain
stroke. Its hit test is the stroke distance. The multi-anchor shapes show the shared placement
guide until every anchor but the last is placed, then preview their own parts through the pointer. Through the shared `handles` and `drag` hooks, the rotated rectangle's
handles are its axis ends plus derived width handles at its long sides' midpoints (the third anchor
has no handle of its own): a width handle sets the half width to its target's distance from the
axis, and an axis-end drag re-places the third anchor at the baseline width about the new axis, so
rotating the axis never collapses it; the width point is stored on its long side's midpoint
without time snapping. The polyline sets the shared `close_placement`: clicking its first vertex
once three are placed closes and commits it.
<!-- B8: shapes — end -->

Wire ids are reserved per family: core 0..=31, lines 32..=47, channels 48..=63, fibonacci 64..=95,
pitchforks_gann 96..=127, projection_annotations 128..=159, patterns_elliott_cycles 160..=191,
shapes 192..=223; 224..=255 are unassigned. A test asserts every spec sits in its family's range
with a unique wire id and name, and that the serde and catalog names agree.

Single-list registries carry one `// B8: <family> — begin/end` block per family (`<!-- -->` in
HTML). A family edits only inside its own blocks, so parallel family work merges additively:

| File | Blocks |
| --- | --- |
| `drawings.rs` | `DrawingKind` variants |
| `drawings/tools.rs` | `DRAWING_TOOL_SPECS` entries; `DrawingKind::spec()` arms |
| `drawings/kinds/mod.rs` | `mod` declaration; new family-specific `DrawingFamily` hooks and their defaults in `DrawingFamily::new` (a hook a second family needs, or one the foundation adds for every family, moves out of the blocks); wire-range test table |
| `drawing_contract.rs` | `DrawingKindOptions` variants; `DrawingToolOptions` fields, `validate` checks, and `template_style` clears of captured data |
| `lib.rs` | public re-exports of family option types |
| `packages/charts/src/types.ts` | `drawing_kind` union; `DRAWING_KIND_TO_U8`; `drawing_kind_options`; family option types; `drawing_tool_options` |
| `examples/web_demo/index.html` | drawing toolbar buttons |
| `docs/Public_api.md` | drawing family catalog |
| `docs/Architecture.md` | family semantics paragraph (above) |

The `drawing_perf` native example measures every tool with a wire id of 32 or more in its
`families` mix and places each with its catalog anchor count, so family work never edits it.

Recipe for a family:

1. Add the `DrawingKind` variants (the serde name is the spec name) and, in
   `kinds/<family>.rs`, the specs with wire ids inside the family's range plus
   `pub(crate) static FAMILY`, built with `DrawingFamily::new(build_parts, kind_options)` and
   assigning the optional hooks it needs. Declare the module and list the specs and `spec()` arms
   in their blocks.
2. Resolve geometry only through `DrawingParts` using the shared helpers; add pure geometry to
   `aeris_charts_render::shape` when it is generic. Never emit `Prim`s or branch on kinds in the
   frame, hit tester, executors, WASM, or hosts. Map derived points with `PartContext::point_px`
   and size strokes, glyphs, and gaps by the `PartContext.scale` ratio. Use the existing placement
   classes and the `Anchors`, `Endpoints`, `RectangleBounds`, or `Position` handle modes. Derived
   (non-anchor) handles and their drags go through the shared `handles` and `drag` hooks, and a
   multi-click close through `close_placement`, never through per-family drag or placement code.
   Paint a drawing's own `text` as `DrawingParts::text_label` over `PartContext::text_lines`, which
   makes it editable in place with no host or WASM code; such a family sets `owns_text` so the
   generic text pass does not paint the same `text` again. `owns_text` still lives in the
   projection_annotations block of `kinds/mod.rs`, so the second family to set it first moves the
   field and its default out of those blocks to the shared hooks (and its description from the
   projection paragraph to the shared hook list).
3. Put family options in one serde-default struct in the module, add its field to
   `DrawingToolOptions` (plus an `&& ...` check in `validate` for lists, strings, or numbers), a
   `DrawingKindOptions` variant, a `lib.rs` re-export, and schema descriptors named
   `tool_options.<block>.<field>`. Persistence, clipboard, sync, templates, and WASM need no
   family code, except that data a drawing captures (not style) is cleared for named templates
   in `template_style`.
4. Add the TypeScript union members, wire ids, kind-option and option types, the demo toolbar
   buttons, and the `Public_api.md` entry in their blocks; `impl.ts` derives its reverse wire map.
   `packages/charts/api/public-api-v1.json` is a generated hash that every stream changes;
   regenerate it (`npm run update:api`) after merging instead of merging it.
5. Tests: the wire-range table entry; family engine tests for defaults, armed placement, frame
   parts at DPR 1 and 2 and at a fractional DPR whose bitmap ratios differ, hit testing including indexed against brute force with more than 20
   drawings, anchor and body drags with straighten and magnet, keyboard handle count and nudge, time
   identity across an interval switch, schema and kind options, atomic and undoable option
   patches, persistence round trip with default omission, clipboard, and sync, and that the
   drawing's own text paints exactly once; and a Playwright
   spec (`drawings-<family>.spec.mjs`) for armed placement, hover and hit, options, persistence,
   clipboard and sync, the toolbar, and WebGPU against Canvas2D parity.

Versioned persistence is an engine-owned semantic DTO boundary, never serialization of live engine
structs. Financial-only charts continue to export V1 with ordered pane topology, built-in
drawings (with optional inline anchor times), and the optional drawing price-basis label. V2 adds pane horizontal domains, general axes, typed general datasets (including
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
host-owned series/indicator definitions after the grid restores each Aeris chart document; the
restored drawings' anchor times resolve when that data arrives.

### `aeris_charts_render`

Backend-neutral drawing primitives, colors, geometry, bar-width rules, and the ordered `DrawList`. This is the contract shared by every renderer. The `shape` module holds the pure `f64` drawing-tool geometry that engine drawing families share: segment extension, line/segment clipping, polyline clipping (`clip_polyline_to_rect`), polygon clipping (`clip_polygon_to_rect`, `clip_to_half_plane`), parallel offsets, uniform arc/ellipse tessellation bounded to 256 chords (`EllipseArc::append_points`), clip-aware curve flattening that refines only where the curve can be visible under a 1,024-point budget (`EllipseArc::append_clipped_points`, `flatten_quadratic`, `flatten_cubic`), nonzero-winding ribbon fills (`nonzero_ribbon`), tube outlines and ribbons, polyline simplification, and polyline/polygon/ribbon hit predicates. It never snaps to pixels. Pixel snapping, primitive ordering, clipping intent, and geometry must be decided before backend execution whenever possible. Curved polylines expand their Catmull-Rom spline adaptively by device-px interval length — intervals already a few pixels long render as their chord (dense freehand brush samples), long sparse intervals keep up to 16 segments — and round joins are emitted only where a turn opens a visible wedge, so tessellation volume stays proportional to what the pixels can show on every backend. Polyline strokes for the GPU backends come from one shared anti-aliased stroker (`line::stroke_aa`): a solid core ending half a device pixel inside the nominal edge, a centered one-pixel coverage transition, faded butt caps, and round joins that fill only the outer wedge of a turn. It emits each vertex with a signed edge distance, and each executor chooses the encoding — WebGPU multiplies vertex alpha by coverage on top of MSAA, GPUI writes the path shader `st` channel — so both backends tessellate identical geometry. Line points are never snapped to the pixel grid: sub-pixel positions plus coverage are what keep diagonals smooth. The Canvas2D contract strokes with round joins and butt caps, matching the reference line renderer. `line::round_rect_polygon` is likewise the single rounded-rectangle tessellation for WebGPU and GPUI, with corner chords scaled to the device radius. The WebGPU stroker ignores `Polyline.style`, so engine frame construction never emits a dashed or dotted polyline: series lines lower dashes through `push_line_stroke`, drawings and general series through `push_styled_stroke`/`push_clipped_stroke`, so every executor receives the same solid dash runs for engine-built strokes.

### `aeris_charts_render_gpui`

The native GPUI executor. It converts the prepared primitive stream into GPUI scene operations and owns GPUI-specific text, image caches, geometry conversion, backend metrics, and fixtures. It must not fork chart behavior or recalculate engine geometry. The repository's interactive Linux probes enable GPUI's Wayland and X11 platforms; macOS and Windows continue through GPUI's native platform selection. CI compiles and tests the GPUI backend on all three operating systems.

GPUI's path pass cannot rely on MSAA — its sample count is picked from the surface and can fall back to 1x on Linux — so stroke, disc, and ring meshes carry a per-vertex Loop-Blinn signed-distance encoding in the path shader's `st` coordinates. Polyline geometry comes from the shared `line::stroke_aa` stroker; GPUI only maps its signed distances onto `st`. Polyline strokes keep `s` constant and encode signed device-pixel distance in `t`, which is compatible with GPUI's Windows solid-triangle branch; their one-pixel coverage transition is centered on the nominal edge so integrated coverage remains the requested width. Ring strokes use the same constant-s, centered coverage encoding as polylines, preventing Windows from treating the antialiasing fringe as solid stroke. Filled discs retain their shape-specific exterior encoding. A mesh larger than a bounded chunk is split into multiple GPUI paths so one stroke cannot overflow GPUI's fixed path instance buffer and trigger its grow-and-redraw retry loop; the mesh is a triangle soup, so coverage and paint order are unchanged.

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

The browser boundary translates data and platform events and serializes engine-owned value snapshots. It must not become a second chart engine. Exchange-session requests (session slots, trade-stream sessions, resampling boundaries and configuration) are parsed in the host-testable `session_slots` module into engine types; a host write to an engine-owned series (footprint, synthetic, or resampled) through `set_data`, `update`, `update_typed`, or a merge returns rejected ingestion diagnostics instead of a silent no-op (the engine itself rejects merges into those series).

### `aeris_charts_native`

The headless native executor and verification support. It uses tiny-skia for deterministic raster output, golden comparisons, examples, and release performance gates. Text follows the host system UI sans-serif face. It is evidence infrastructure, not a competing product model.

## TypeScript package

`packages/charts` publishes the `@aeristerminal/aeris-charts` browser API through GitHub Packages. It owns WebAssembly initialization, TypeScript chart handles, DOM canvas lifecycle, resize observation, Pointer Event translation for mouse/pen, cancellable Touch Event translation for direction-dependent page-scroll arbitration, platform capture/default policy, host callbacks, themes, shortcuts, offscreen support, and grid helpers. The root entry remains framework-neutral. The optional `@aeristerminal/aeris-charts/react` entry is a thin lifecycle/reconciliation adapter over those same public chart handles: React mounts one ordinary chart, applies option/data changes to retained engine objects (a `FinancialSeries` data change that only replaces the last point and/or appends later points streams through `series.update()`; anything else is one `setData`), and disposes through `chart.remove()`; it owns no scale, geometry, hit-test, persistence, or rendering semantics. Its module performs no DOM work at import time, so SSR can import it without constructing a browser chart. Drawing-tool arming and pointer events cross the browser boundary through the generic engine drawing controller; the package does not classify a tool as single-point, multi-point, sequence, or freehand, nor duplicate tool-specific placement state. Touch Events normalize into the same engine resolver rather than a parallel gesture state machine; static `touch-action` stays `auto`, and the host applies the reference-informed vertical-priority direction rule after the shared slop. Wheel samples retain floating-point deltas; informed by measurements from the pinned public reference fixture, `wheel_behavior: "auto"` independently maps vertical deltas to time zoom and horizontal deltas to time pan on every chart surface without modifier routing. Explicit `"pan"`/`"zoom"` modes remain host overrides, including price-axis wheel zoom only in explicit zoom mode.

The package resolves IANA time-zone names to explicit offset schedules with `Intl.DateTimeFormat` over a bounded 1970–2100 span (cached per page for at most 32 zones), on the main thread and in workers, and never passes the browser's own zone to the engine; the free `session_slot_times()` and `resample_boundaries()` helpers pass their resolved schedule, while trade-stream sessions use the chart's installed zone. It derives the engine's calendar-date flag from the input form of its financial series (business days versus numeric instants), wraps host time formatters with a calendar-date context without re-entering WebAssembly during frame construction, formats package-owned tooltip and accessibility time text from engine-supplied exchange-local seconds, and lets hosts inject the countdown clock (`set_clock`) instead of `Date.now()`.

Wheel routing is one worker-safe function shared by the DOM recognizer and the OffscreenCanvas worker façade, including the reference delta-mode and Windows-Chromium device-pixel speed corrections; worker charts expose no `handle_scroll`/`handle_scale` options, so both wheel gestures stay enabled there. Keyboard time-scale motion from the input overlay and the accessibility surface honors the host gesture switches: arrow panning and accessibility point scrolling need a horizontal scroll gesture, +/- zoom follows wheel zoom, and Home fit/reset follows the time-axis reset gesture, so a view fixed with `handle_scroll: false` and `handle_scale: false` cannot be moved from the keyboard, and a gated key keeps its default browser action. Visible-range and size subscriptions never dispatch re-entrantly: a handler that mutates the chart marks the diff dirty, the current value finishes delivery, and a bounded re-read (eight passes, then the next frame) delivers the final range to every handler. `time_scale().scroll_to_real_time()` animates through the engine scroll animation to the configured `right_offset` over the reference 400 ms (with the engine's cubic ease-out rather than the reference's linear curve), immediately under reduced motion; the headless engine call applies the same target at once.

Browser accessibility is chart-owned, enabled by default, and represented by one singleton controller exposed through `chart.accessibility()`. `enable_accessibility(chart, options)` configures that same controller for compatibility. The chart container is a named group; canvases are hidden from assistive technology and each pane has one complete application-style keyboard surface. Default streaming announcements are off, user-driven navigation/actions remain announced, visible data queries are capped at 512 on-demand logical points, and only the active series owns one engine-rendered focus primitive. Accessibility focus/edit state is runtime-only and is never persisted. Pointer interaction updates ordinary chart selection and hover without moving DOM focus into the application surface; keyboard traversal and explicit accessibility API calls own its visible focus. Forced colors, higher contrast, reduced motion, locale, host names, and visible focus are resolved at the host boundary; the shared engine retains exact focus geometry and keyboard drawing mutations use the same drawing history/rollback path as pointer input.

Auto-size keeps `ResizeObserver`'s exact device-pixel path. A resolution media-query watcher plus orientation/fullscreen fallbacks re-run sizing when DPR changes without a CSS-bounds change; resize reprojects semantic state and does not create new object identities. While auto-size is active, manual `resize` calls are ignored. Disabling it disconnects the engine-owned observer and returns authority to manual sizing; re-enabling immediately adopts the current container. Hidden or detached containers retain the last usable size and adopt their new bounds when revealed.

The package also ships `aeris_charts.css` as the portable host design system. Its complete brand token contract remains intact even when a token is currently consumed only by Terminal or the website; Charts consumes the applicable surface, border, text, status, control, interaction, icon, action, focus, radius, shadow, and market roles without renaming them. Host chrome uses the system UI font stack and may use `color-mix`. The published package does not include a webfont. Native CPU text uses the host system UI sans-serif face; scene goldens that contain no text stay machine-independent. Backend-facing roles — the primary surface for chart panes, primary text for axes, border for axis rules and pane separators, canonical border width, muted text, separator interaction, focus/primary interaction, positive/negative market semantics, and shared radii — have deterministic opaque-sRGB projections in `crates/aeris_charts_core/style_tokens.json`. `aeris_charts_core` owns and compiles that file into the defaults used by every engine and backend. Axis borders and visible pane separators both project the shared border-width token onto the device-pixel grid in the engine; the separator hover target stays independently expanded for interaction. The TypeScript package imports the same source at build time for host theming and workspace divider projection. The core crate therefore remains independently packageable without reaching into a browser-package directory, and the tokens resolve before frame construction rather than through demo or renderer overrides. Canonical engine grid lines retain their dashed style and border color but ship disabled; hosts and deterministic parity fixtures may opt either family in explicitly. A `v*` tag matching the package version publishes the verified artifact to GitHub Packages.

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

Frame invalidation is an engine-owned generation graph. Layout, coordinates/autoscale, grid and underlay, each series, drawings (with per-drawing prim/point segments plus a trailing controller-owned creation-preview block), and interaction overlays have independent generations. Coordinate-range changes fan out to coordinate-dependent layers; a value-only current-bar update stays on its source series when autoscale bounds do not change. A `histogram_updown` histogram layer is also keyed by the primary price series' generation, because its column tint reads the primary's rows (open versus close, or the previous traded close for time-sharing volume, whose first row compares with the primary's explicit baseline or percentage base) and takes host `up_color`/`down_color`. Ordering-only promotion (hover/selection/drag/edit) reassembles retained series layers and drawing segments without rebuilding geometry; drawing drag rebuilds the drawings layer with fresh segments while reusing the runtime per-entry cache. Public option and series-style mutation are included in the generation inputs, so direct native callers cannot bypass retention accidentally.

Series and indicator selection owns one transient engine snapshot with a single primary command target and at most 64 related output members. Engine-owned indicator bindings expand automatically; hosts may supply the bounded member identities for study groups they author outside the built-in indicator registry. Each member retains at most 128 canonical output timestamps sampled from its own full canonical start-to-end extent only on the unselected-to-selected transition. Selection-time projection determines sparse density, while endpoint-inclusive logical spacing prevents a partial-series selection treatment. Overlay rebuilds resolve every member's identities against its current canonical values and coordinates, place candlestick handles at the current body midpoint, clip offscreen handles without replacement, and discard the snapshot on deselection; LOD geometry, screen coordinates, and persistence never own selection-anchor membership.

Drawing semantic mutations reuse this graph: add/remove/style/anchor changes invalidate the drawing layer and update only the affected derived entry, while selection/hover/drag/edit promotion reassembles retained drawing segments without rebuilding geometry and selection changes additionally invalidate the overlay and axis frame for handles. The one exception is a family drawing that paints some parts only while focused (a note's text, `DrawingFamily::reveals_on_focus`): frame construction keys the hovered and selected drawing among those and rebuilds the drawings layer when that key changes; opening or closing a text-edit session rebuilds it too. Drawing selection handles are assembled at the beginning of the overlay, preserving their prior canonical order immediately after drawing bodies and before crosshair/series overlays without rebuilding unrelated drawing geometry. Temporary promotion (dragging/editing → hovered → selected → idle, hover gated by `hoveredSeriesOnTop`) never rewrites saved drawing z-order; deselection, hover leave, cancellation, or removal restores it. A selected price-spanning rectangle also emits primary-colored extent tags and a territory band on its bound price scale; those axis views follow creation, drag, and resize coordinates and disappear on deselection unless the drawing explicitly requests persistent axis views. The text tool is an exception: it emits no anchor discs — selection and hover paint the same focus border box (hover at reduced opacity), empty text paints nothing on the chart, and leaving the host editor without typed text removes the drawing. While typing, the host wrap is borderless with transparent glyphs; the engine keeps painting both the label and the focus border underneath, so edit entry cannot lift the text or shift the outline. Crosshair movement and unchanged-coordinate market-data updates do not invalidate drawing geometry. Pane add/remove/swap/move rebuilds pane membership because pane ownership itself changed; ordinary drawing drag updates one entry, and structural removal repairs the canonical vector's id-to-position map.

The engine-owned crosshair overlay can paint the same configurable hover marker for every visible line, area, baseline, and line-shaped indicator output at the snapped logical index. Markers ship disabled and hosts opt in per series or indicator output; when enabled, marker coordinates, per-series colors, borders, pane ownership, and scale conversion are resolved before the shared frame reaches any backend. Crosshair and drawing magnets share one pixel-space candidate path: candle, bar, and footprint series expose their rendered OHLC fields, while line, area, histogram, baseline, and other scalar projections expose only the close/value they paint, so hidden storage columns cannot attract an anchor. An empty hovered trend line emits a low-opacity, borderless `+ Add text` run at its configured segment-relative slot; the engine owns its measured hit box and exact caret anchor, including the middle-slot stroke gap, while a browser host owns only the transparent content-editable caret overlay. Clicking either this affordance or existing trend text enters inline editing, and leaving an empty trend edit preserves the drawing. Bar-slot highlights and tooltip guides resolve their default tint from the current chart surface, using a light lift on dark surfaces and a dark tint on light surfaces; overlay price-scale text follows the current layout foreground. Explicit host colors remain authoritative, while implicit colors retokenize with chart options. Chrome that stands for a bar itself — the built-in live price line, its last-value axis chip, and the crosshair marker — follows one shared bar-color resolution. The built-in live price line is one canonical series feature: its default `partial` extent starts at the tracked bar/value and reaches the pane's right edge, while `full` is an explicit per-series option. Both extents use the same source, color, width, and solid/dotted/dashed line-style state, so ordinary series and engine indicator outputs cannot drift in thickness or dash semantics. Explicit user-created horizontal price-line objects remain full-width independent chart objects. For candlesticks that resolution walks the parts in paint order, body then border then wick, skipping any part that is transparent or switched off, so a hollow candle (a transparent body over a visible border frame, industry-standard) keeps its bullish or bearish color instead of resolving to an invisible fill. Bar presentation remains engine-owned as well: OHLC bars keep one vertical high/low body and independently gate the open and close ticks, so setting both visibility flags false produces an explicit high-low bar without a host-side geometry fork.

The engine retains semantic pane layers and their ordered primitive/point ranges, then assembles the same canonical `ChartFrame` contract from clean and rebuilt layers. The retained boundaries are underlay/grid, individual series, per-drawing segments plus a trailing creation-preview block, pane chrome, transient trading risk/reward preview regions, financial-action lines/controls (trading and alerts), and overlay. Frame assembly is the single pane-local ordering owner: grid/background → idle indicators → idle drawings → ordinary price series → active objects (dragging/editing → hovered → selected, series before drawings within a tier, previews trailing active) → chrome → trading regions → trading/alerts → overlay → top. Indicator outputs move as one visual group with internal ordering preserved (bindings own grouping, never series type or title); explicit `set_series_order` overrides default idle series grouping while idle drawings stay below price series and indicator-only panes keep stable internal order. Axes, crosshair, and financial-action controls keep their protected layers above all chart content; active chart content stays clipped to its owning pane. No public z-index API or renderer-specific policy exists. Preview regions sit above chart content with financially actionable lines, alert indicators, and exact control hit zones above them and below crosshair transients. A series' live-price cluster stays filled while its value is live, and is outlined — chart-surface fill, semantic color as an inside border and text — once the series' final bar scrolls out of view, so a stale value never reads as the current one. Colliding series clusters are spaced by the overlap pass rather than restyled. Otherwise-solid trading tags that meet the primary cluster's raw axis region take that same outlined treatment, but financial-action tags stay at their exact price coordinate instead of being collision-shifted away from the line they identify. They emit before the primary cluster so the live price remains visually authoritative if exact coordinates overlap. Confirmed orders never manufacture persistent risk/reward fills. Positions and orders use a 304 CSS-pixel bounded marker beside the price scale. Each order and position rule extends by default from the pane's left edge to the marker end and passes beneath the opaque marker container. Only the visible marker span is interactive. Each marker is a taller readout chip — a solid quantity cell and then P&L or order type inside ONE outline, with no inner border or divider, so the quantity block's own edge is the seam — followed immediately by an integrated close/cancel cell on the P&L/readout's right. The single main container and its edge cells use the shared large `999px` radius token, which the engine resolves with CSS clamping semantics to form one pill. Outline and quantity fill carry the object's semantic color, so the marker reads as one color from the line to the price tag; only the P&L text keeps its own profit/loss tint. Every close icon uses its marker's semantic line color in idle, hover, and pressed states, so line, outline, quantity fill, and close icon read as one color. The close icon is two correctly stroked, round-capped diagonal lines in shared primitive geometry rather than a rotated font glyph or skewed filled bars, because a host `font_family` is not guaranteed to carry a suitable symbol. Protection role takes precedence over broker side and kind: TP is positive green and SL is warning yellow. Every ordinary buy order, including a resting buy limit, uses the positive token; every ordinary sell order, including a sell limit, uses the negative token. Positive and negative P&L text uses those same semantic roles. Rejected, cancelled, and expired markers use the rejected color, while pending broker operations use the pending color. A price tag is solid only once its order is actually filled — a resting order stays outlined, so a working intention never reads as an executed one. Every main marker pill uses a solid semantic outline at the design-system `--border-width`, snapped like a browser border and emitted as the container's `RoundRect` border — every executor (WebGPU, GPUI, and the shared Canvas2D executor behind browser Canvas2D and native CPU output) paints `RoundRect` borders inside the rect, never centred across its edge — with a deliberate chip-surface gap between it and the edge cell fills, while the integrated close cell keeps its full-height hit target but paints no resting surface of its own; its icon sits directly on the container surface. Hover and press fill the addressed control (for the close cell, a smaller inset, fully rounded pill) with the brand `--hover-bg` / `--active-bg` surfaces rather than the order color, and the close cell answers hits across the pill's full height rather than the line tolerance. A hovered working-order rule changes to compact hairline dashes without increasing stroke width, keeping dash metrics consistent across device-pixel ratios. Those fills stay OPAQUE: a control sits on top of its own marker line, and a translucent fill would let that line read through the button the pointer is on. For the same reason the host suppresses the crosshair lines while the pointer is over a trading control — the control is not a price to read. Action tooltips are host-timed: the engine owns no clock, so it reveals one only once the host arms it after a hover dwell, and a changed hover disarms it. Sweeping across stacked markers therefore never flashes a tooltip per marker. An action tooltip is chart chrome rather than part of the object it describes: it takes the active theme's surface, border, text, and radius tokens, never the order's buy/sell color, so it reads identically on every line and in both themes. Confirmed TP and SL orders remain ordinary host-authoritative order markers with quantity and projected P&L, endpoint nodes, and action tooltips. Trading, alerts, and axis labels share the canonical pane price transforms; WebGPU, Canvas2D, GPUI, screenshots, and native/headless consumers receive no separate financial-action geometry. Retention never gives a backend permission to change ordering or semantics. Host/plugin primitive callbacks use the canonical frame but conservatively rebuild the affected pane stream because their output is not engine-owned. Incremental frames are tested against forced clean rebuilds across data, interaction, scale, drawing, theme, and resize mutations.

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
Family text boxes open the same transparent surface, as a native `textarea` because their text
may span lines (Shift+Enter inserts a line; paste inserts plain text), laid out from the engine's
`drawing_text_edit_layout`: lines left-aligned at the box's text edge and a caret positioned by
line and column. After every keystroke the host relays the surface out from the engine, because a
box can grow upward (a comment) or both ways (a centered callout). All three modes share the engine text-edit session,
so a whole edit is one undo step and Escape restores the text without a history entry. A
double-click on a selected drawing, or Enter or F2 on the chart or its accessibility drawing target
(the target keeps Enter for geometry editing), opens the editor on whatever the engine reports
`drawing_text_editable`; the editor is a labeled `textbox`, announces opening and closing through
the accessibility live region, and returns focus to the element it was opened from inside the
chart.

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

Track CPU frame time, GPU time where available, draw calls, dropped and presented frames, memory, ring overruns and dropped invalid ring rows, interaction latency, and steady-state allocation. Device loss or unavailable WebGPU must fail over without losing headless chart state.

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

The standard gates mirror CI (`.github/workflows/ci.yml`, in its order):

```text
node packages/charts/scripts/namespace_guard.mjs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p aeris_charts_wasm --target wasm32-unknown-unknown --locked -- -D warnings
cargo test --workspace --locked
AERIS_CHARTS_PERF_STRICT=1 cargo run -p aeris_charts_native --example perf_gate --release

cd packages/charts
npm ci
npm run lint
npm run build
node ../../benchmarks/benchmark.mjs size   # CI runs it from the repository root
npm run typecheck
npm run check:api
npm run check:release-gates
npm run test:pack
```

An intentional public-API change regenerates the snapshot with `npm run update:api` (a write
step, not a gate) before `check:api`. CI also requires the portable browser suite
(`AERIS_CHARTS_PORTABLE_BROWSER=1 npx playwright test` in `examples/web_demo` after
`npm ci && npm run build` there) and the native GPUI tests
(`cargo test -p aeris_charts_render_gpui --features gpui-backend --all-targets --locked`).

The release performance gate also measures a 100,000-visible-bar volume-profile refresh through frame construction and verifies that unchanged frames retain the calculation revision. Its Target M binds every built-in indicator kind plus aggregate-input studies to one 1,000,000-row candle source and its volume series and times current-bar replacements and candle-then-volume appends through the public engine path against a 1 ms per-tick budget. Its Target N times the same live ticks, each followed by one frame, with five regression trends anchored across a 1,000,000-row source against the same budget.

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
live scalar volume series, while missing volume remains the explicit unit-weight fallback. An
amount-weighted VWAP additionally requires a volume series and a distinct live scalar turnover series.
Financial study persistence V3 stores binding definitions (including explicit seed, histogram, and
estimator parameters, which default to the TradingView convention when absent), dependency references,
scalar inputs, volume and turnover inputs, and output styles while leaving market history and ordinary
series data host-owned. Trade, quote, and
depth study inputs remain owned by the host market runtime: it supplies typed stream requirements and
generation-fenced borrowed views, while Charts receives only bounded scalar study output publications.
Charts must not retain a second tape/book or infer provider stream state from a rendered series. The
Terminal bridge now carries the transitive stream requirements as bounded output metadata and persists
only the validated host binding; this keeps the runtime-to-chart boundary explicit while allowing
downstream output presentation to retain its typed input contract.
