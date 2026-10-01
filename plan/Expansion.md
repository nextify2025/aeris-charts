# Aeris Charts Trading Expansion Plan

Aeris Charts will become a complete **headless** professional trading chart engine: order flow,
market depth, non-time bars, a professional indicator catalog, and a drawing system whose every
tool is as configurable as the tools in mature trading platforms. The primary consumer is the
Aeris Terminal GPUI platform; browser hosts consume the same engine through WASM.

This is the **active program** (since 2026-09-25). [plan.md](plan.md) covers general
(non-financial) chart families and is paused. Both plans share one `ChartEngine` and one frame
contract.

How to read this file:

1. **Status at a glance**: where every batch stands.
2. **How work is delivered**: the batch, gate and commit rules.
3. **Batches B1–B9**: the work itself, as checklists with exit criteria.
4. **Scope and ownership**: what the engine owns, what hosts own, what is out of scope.
5. **Current baseline**: what exists today.
6. **Architecture principles**: standing rules for every batch.
7. **Specifications**: detailed requirements for foundations F1–F6 and platform contracts PD1–PD11.
8. **Catalogs**: order-flow (OF), chart-type (CT), indicator (I) and drawing items.
9. **Verification and completion**: evidence required per item and for the whole plan.

Item IDs (F, OF, CT, I, PD) are stable and referenced by the platform roadmap
(`plan/trading_platform_feature_roadmap.md` in the Aeris Terminal repository). Batches group those
items; they do not renumber them.

## Status at a glance

Updated 2026-09-27. Baseline source-confirmed 2026-09-24.

| Batch | Scope | Unblocks on the platform | Status |
| --- | --- | --- | --- |
| B1 | Platform chart contracts: PD11, PD1, PD3, PD4, PD5, PD6, PD7 | Multi-account chart trading, trailing and break-even stops, risk warnings on order lines, economic events and risk windows, trade review markers, linked charts, journal images, fundamentals | **Complete** |
| B2 | Drawing model and customization: F5, schema conventions, existing tools | Configurable drawings, templates, drawing sync across cells | **Complete** |
| B3 | Shared tape and order flow: F2, OF1, OF2, OF11, OF12, PD10 | Footprint, CVD, delta, big-trade bubbles | **Complete** |
| B4 | Study inputs and core indicators: F4, OF9, CT1, CT2, CT6, I1 | Professional indicator set, VWAP bands, Heikin Ashi, comparisons | **Complete** |
| B5 | Non-time bars and replay: F1, OF14, CT3, CT4, PD2 | Tick/volume/range charts, session replay, trade review playback | **Complete** |
| B6 | Depth: F3, OF15–OF18, PD8, PD9 | Liquidity heatmap, order-level markers, depth studies | **Complete** |
| B7 | Profiles and resampling: F6, OF3–OF8, OF10, CT5 | Session/composite profiles, TPO, anchored VWAP, multi-timeframe studies | Open |
| B8 | Drawing catalog expansion | Full professional drawing toolset | Open |
| B9 | Breadth and extension: I2, I3, I4, OF13 | Remaining indicators, custom studies, auction markers | Open |

Ordering: B1–B3 serve the platform's first phase and are independent of each other. B4 must land
before B7 (OF10 needs F4), B2 before B7 and B8 (they need F5), B3 before B5 and B6 (replay and the
heatmap reuse the shared tape), and B5 before B6 (depth extends the replay checkpoints). Change the
order only when platform priorities change, and record it here.

## How work is delivered

Work proceeds in **large batches**, as defined in **Work cadence** in [AGENTS.md](../AGENTS.md). A
batch is one whole row of the status table, not one item or option.

- **Implement the whole batch first.** Build every checklist item in the batch, with its regression
  tests and fixtures written as each item is built. Do not stop between items for full gates,
  commits or pushes.
- **Focused checks while implementing.** `cargo check`, unit tests and `cargo clippy` for touched
  crates, and frame fixtures for the affected families. Nothing broader.
- **One full gate at the end.** Run the complete gates from AGENTS.md once, plus Playwright when the
  batch changes browser-facing behavior and GPUI parity/replay when it changes GPUI execution. Fix
  every failure and rerun until green. If the cause is unclear, rerun focused checks item by item.
- **One commit and push per batch.** Commit with a structured message listing delivered item IDs and
  verification, then push `main`. Never commit a batch with a failing or skipped required gate.
- **Update this file in the same commit.** Tick the batch's checklist, set its status, and update
  `docs/Architecture.md` when ownership, data flow or execution paths changed.
- **Manual evidence at milestones, not per batch.** Themed and overflow screenshots, accessibility
  review, competitor comparisons and recorded benchmarks are collected when B3, B6 and B9 close.

A batch may be split into two commits only when it is too large to review as one, and each part
must pass the full gate on its own.

## Batches

Each checklist item refers to its specification or catalog entry for the detailed requirement.
An item is ticked only when it works through the real host and executor paths (Rust, WASM and
TypeScript, and the GPUI host), not when engine unit tests alone pass.

### B1 — Platform chart contracts

**Scope:** PD11, PD1, PD3, PD4, PD5, PD6, PD7. **Depends on:** the existing trading layer,
workspace and series paths. **Status:** next.

These extend existing layers without new foundations, and the platform needs them first. The
trading layer (`trading.rs`, `frame/trading_geometry.rs`) already renders positions, working
orders and executions, supports drag and keyboard modify, brackets from drawings, and
      host-resolved intents. Aeris Terminal can wire basic chart trading against it today; B1 corrects
      it for the platform's multi-account runtime rather than rebuilding it. **Status: complete.**

- [x] **PD11** Trading layer alignment: optional account identifier on trading objects and
      intents with a host-set visible-account filter, trailing-stop and break-even presentation
      from host-supplied trigger prices, exact tick-index prices on intents, and a documented
      order-state contract with fixtures.
- [x] **PD1** Host annotations on working orders and positions: bounded list, semantic tones,
      tooltip text, shared layout with the existing chips, deterministic overflow collapse, exact
      hit-testing, caps and atomic rejection of invalid annotations.
- [x] **PD4** Execution marker variants (circle, arrow, triangle, optional size by quantity) and
      round-trip connectors with host result labels colored by outcome, identifier hit-testing and
      caps.
- [x] **PD3** Host event layer: typed event markers and shaded time windows on price and study
      panes, LOD collapse, hit-testing to host identifiers, caps, and exclusion from drawing
      persistence and undo history.
- [x] **PD7** Sparse fundamental series: confirm or extend `LineType::WithSteps` step-after
      semantics, add column/histogram presentation in its own pane, and add as-of release labels.
      Replay no-look-ahead is verified in B5.
- [x] **PD5** Cross-chart synchronization: read and set external crosshair and visible time range,
      semantic events with source and revision, echo-loop prevention, and subscription removal on
      disposal. Built once so plan.md R4 can reuse it for general charts.
- [x] **PD6** Native and GPUI image export: RGBA output at a requested size and scale, optional
      crosshair and trading layer, the same composition rules as the browser `take_screenshot`, and
      no disturbance to live state or frame pacing. Works for financial and general panes.
- [x] `docs/Architecture.md` updated for the new host contracts.
- [x] Full gate green; batch committed and pushed.

**Exit:** every PD exit criterion above passes on GPUI and in the browser, and live-rate updates to
annotations do not rebuild unrelated trading geometry.

### B2 — Drawing model and customization

**Scope:** F5 and the typed schema conventions shared with F4. **Depends on:** nothing new.
**Status:** complete (2026-09-25).

This fixes the biggest customization gap before new tools are added, so B8 builds on the final
model.

- [x] Record reference behavior for the drawing family and a release performance baseline for
      drawings.
- [x] Typed schema conventions (parameter and property descriptors: name, type, range, default)
      shared by drawings and studies.
- [x] Drawing split into a common core plus a typed per-kind option block; property exceptions
      recorded per tool in the catalog.
- [x] Identity and state: stable ID, name, group, revision, visible, locked, z-order operations,
      per-interval visibility.
- [x] Stroke, line ends, extension and fill properties from the F5 contract.
- [x] One shared text layout path (measurement, alignment, placement, box, clipping, wrap) used by
      every tool; the existing trend-line text becomes one instance of it.
- [x] Toggleable labels and statistics per tool, with label positions.
- [x] Numeric anchor read/write, scale and pane binding, and magnet modes (off, weak, strong).
- [x] Level-list contract (values, colors, visibility, styles, fills between levels) ready for B8
      level tools.
- [x] Atomic property patches validated against the schema; each property change is one undo/redo
      entry.
- [x] Style templates as data: per-tool default overrides, named templates, validation,
      export/import.
- [x] Management: object tree snapshot, multi-select, group move/lock/hide, clone, copy/paste
      payloads, bulk remove, and cross-cell sync through revisioned payloads without echo loops.
- [x] All ten existing tools migrated to the contract.
- [x] Lossless persistence migration from V1/V2 drawings.
- [x] Executor parity fixtures for text on lines, shapes and level tools.
- [x] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

Evidence: the bounded `thousand_mostly_offscreen_drawings_bound_frame_and_hit_work` fixture is
the release drawing-work baseline; typed schema/state, clipboard/z-order, persistence, label and
frame parity fixtures are in the engine test suite. Native and WASM builds expose the same typed
schema, templates, object tree and sync payload operations.

**Exit:** the F5 exit criterion passes: every existing tool supports the common contract, a host
builds a generic property panel from schemas alone, and old layouts migrate.

### B3 — Shared tape and order flow

**Scope:** F2, OF1, OF2, OF11, OF12, PD10. **Depends on:** the existing footprint.
**Status:** complete (2026-09-26).

- [x] Record reference behavior and release baselines for footprint and tape-derived studies.
- [x] **F2** Chart-level trade stream handle keyed by host instrument stream; footprint rebound to
      it; classification once per event; revisions; per-dependent incremental state with a rebuild
      path; memory telemetry per stream and per dependent. Checkpoints are designed so F1 bars and
      PD2 seeks can use them.
- [x] **OF1** Cumulative volume delta pane (candles or line; session, continuous and anchored
      reset).
- [x] **OF2** Bar delta histogram, delta %, max/min delta and buy/sell/unknown volume split.
- [x] **OF11** Large-trade bubbles and volume dots with size by volume, color by side, threshold
      filters and consecutive-print aggregation, on a bounded marker primitive path.
- [x] **OF12** Footprint variants: profile-in-bar, volume ladder, horizontal imbalance, delta-only
      and bid/ask histogram cells.
- [x] **PD10** Release benchmarks for dense footprint text on GPUI and WebGPU; shared caching of
      repeated numeric runs where measurement shows shaping dominates; budgets added to `perf_gate`.
- [x] Early F1 design note in `docs/Architecture.md` so later work does not assume the second-based
      axis.
- [x] Full gate green; benchmark gate changes committed and pushed; milestone evidence recorded.
- [x] Milestone evidence: screenshots, accessibility review and recorded benchmarks for order flow.

Implementation evidence so far: `chart_trade_stream_is_shared_by_bound_footprint_dependents`,
`cvd_and_delta_dependents_follow_late_corrections_and_report_rebuilds`,
`trade_bubbles_are_bounded_and_rebuilt_from_the_shared_tape`, and
`footprint_retention_evicts_shared_studies_with_the_same_bar_boundary` cover shared revisions,
derived-study updates, bounded markers, and retention. Rust, WASM, and TypeScript APIs expose the
same stream/dependent contracts. The native release `perf_gate` now exercises the shared-study tape,
tip/correction paths, retention, and dependent incremental work. The release
`aeris_charts_render_gpui/examples/plan_bench` also includes a deterministic detailed-LOD footprint
fixture and reports primitive/text counts plus p50/p95/p99 scene-lowering cost. That benchmark
stops at GPUI scene construction, while native `perf_gate` Target J covers WebGPU CPU-side
frame encoding and verifies every resolved dense text run is scheduled. Neither benchmark covers
native window shaping or actual GPU present time; `gpui_probe` now accepts
`AERIS_CHARTS_PROBE_FEATURE=footprint` for that real-window capture (it prints no frame data). The screenshot and accessibility milestone is recorded in `docs/Footprint.md`. The
screenshot harness accepts
`AERIS_CHARTS_GPUI_FEATURE=footprint` and emits a DPR-aware PNG plus metadata for the dense
12-bar fixture; the capture has been exercised on the current Windows display after fixing the
harness to pass the configured frame background through the GPUI prepared frame. The observed
30-frame footprint probe run is recorded in `docs/Footprint.md`; it remains machine-specific
evidence, not a portable budget. The current release gate also verifies the WebGPU executor path:
Target J schedules all 120 resolved dense text runs and measures 0.00 ms p99 CPU-side frame
encoding against the 2.00 ms budget; the browser footprint suite passes its six Chromium cases,
including the WebGPU shared-frame case. The GPUI probe remains machine-specific evidence, while
the portable release budget is now covered for both executor sides.

**Exit:** the F2 exit criterion passes (footprint and CVD share one tape, a late trade updates both,
retention evicts both), and PD10 budgets hold on GPUI.

### B4 — Study inputs and core indicators

**Scope:** F4, OF9, CT1, CT2, CT6, indicator tier I1. **Depends on:** B2 schema conventions.
**Status:** complete (2026-09-26).

- [x] **F4** `IndicatorInput` gains open; selectable sources (open, high, low, close, hl2, hlc3,
      ohlc4, hlcc4, any indicator output); multi-input bindings with typed validation; typed
      parameter schemas and output descriptors; per-output style persisted; study bindings in the
      next persistence schema version.
- [x] **F4 exit fixture slice:** RSI of hlc3, SMA of RSI and Bollinger fill values/styles and the
      resulting engine frame round-trip through persistence; Canvas2D/WebGPU and Canvas2D/GPUI
      draw-stream parity fixtures cover the chain, while full render parity remains part of the
      aggregate F4 exit.
- [x] **OF9** VWAP standard-deviation and percent bands with session, weekly and monthly reset.
- [x] **CT1** Hollow candles, columns, high-low bars, step line, line with markers.
- [x] **CT2** Heikin Ashi with real OHLC exposed separately for trading and crosshair.
- [x] **CT6** Symbol comparison overlays with a shared comparison anchor and per-symbol legend
      values.
- [x] **I1 moving-average catalog:** HMA, VWMA, DEMA, TEMA, SMMA/RMA.
- [x] **I1 Hull moving-average slice:** HMA with pure, incremental, schema, persistence and package coverage.
- [x] **I1 moving average slice:** DEMA with pure, incremental, schema, persistence and package coverage.
- [x] **I1 Wilder moving-average slice:** SMMA/RMA with pure, incremental, schema, persistence and package coverage.
- [x] **I1 trend:** Ichimoku.
- [x] **I1 trend slice:** ADX/DMI with pure, incremental, schema, persistence and package coverage.
- [x] **I1 trend slice:** Parabolic SAR with pure, incremental, schema, persistence and package coverage.
- [x] **I1 trend slice:** SuperTrend with pure, incremental, schema, persistence and package coverage.
- [x] **I1 trend slice:** Ichimoku with pure, incremental, schema, persistence and package coverage.
- [x] **I1 channels and volatility:** Keltner Channels.
- [x] **I1 volatility slice:** population standard deviation with pure, incremental, schema, persistence and package coverage.
- [x] **I1 channel slice:** Donchian Channels with pure, incremental, schema, persistence and package coverage.
- [x] **I1 channel slice:** Keltner Channels with EMA/ATR pure, incremental, schema, persistence and package coverage.
- [x] **I1 oscillator slice:** CCI with pure, incremental, schema, persistence and package coverage.
- [x] **I1 oscillator slice:** Williams %R with pure, incremental, schema, persistence and package coverage.
- [x] **I1 oscillator slice:** Stochastic RSI with pure, incremental, schema, persistence and package coverage.
- [x] **I1 oscillator slice:** ROC and Momentum with pure, incremental, schema, persistence and package coverage.
- [x] **I1 volume slice:** OBV with pure, incremental, schema, persistence and package coverage.
- [x] **I1 volume slice:** CMF with pure, incremental, schema, persistence and package coverage.
- [x] **I1 volume slice:** MFI with pure, incremental, schema, persistence and package coverage.
- [x] **I1 oscillators:** CCI, Williams %R, Stochastic RSI, ROC/Momentum, MFI.
- [x] **I1 volume:** Volume study with MA.
- [x] **I1 levels slice:** daily UTC previous-session pivot points (standard, Fibonacci, Camarilla,
      Woodie, DeMark) with pure, incremental, schema, persistence and package coverage.
- [x] **I1 levels slice:** ZigZag with percentage-deviation turning points, pure, incremental,
      schema, persistence and package coverage.
- [x] **I1 levels:** pivot points (standard, Fibonacci, Camarilla, Woodie, DeMark), ZigZag.
- [x] Every indicator has incremental state, rebuild equivalence, a typed schema, persistence and an
      independently computed reference fixture.
- [x] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** the F4 exit criterion passes (RSI of hlc3, SMA of that RSI and a Bollinger band fill
round-trip through persistence and render identically on every executor) and every I1 fixture
matches its reference.

Implementation evidence so far: the F4 foundation now carries the open OHLC column through
`IndicatorInput`, exposes close/open/high/low plus hl2/hlc3/ohlc4/hlcc4 scalar sources, retains
output identities when a binding is rebound, and publishes typed parameter/output schemas. VWAP
multi-input bindings reject missing, duplicate, non-scalar, and stale volume sources before any
state is created. The engine, WASM shell and TypeScript package expose the explicit-source path and
bounded schema query; existing convenience methods remain close-based. Indicator outputs now also expose and atomically
replace a compact engine-owned style snapshot, preserving per-output presentation independently
of binding kind. VWAP's optional volume input now aligns by exact timestamp and keeps missing rows
on the documented unit-weight fallback. OF9 now adds five engine-owned VWAP-band outputs with
session, weekly and monthly reset keys, weighted population-deviation bands and percentage bands;
pure-math and incremental rebuild tests cover the monthly reference path. Typed multi-input
validation now rejects invalid VWAP volume bindings atomically. Financial persistence V3 now
round-trips ordered study dependencies, scalar inputs, volume references and per-output styles while
leaving market data host-owned. The Terminal host bridge carries each
runtime study's transitive typed trade/quote/depth stream requirements beside
the bounded scalar publication, so downstream chart presentation can retain
binding metadata without a second tape or book. The final B4 docs/full-gate
closure is now verified. DEMA, TEMA, SMMA/RMA, HMA, VWMA,
standard deviation, CCI, Williams %R, Stochastic RSI, ROC, Momentum, Donchian Channels, Keltner Channels, ADX/DMI, Parabolic SAR, SuperTrend, Ichimoku are
OBV, CMF, MFI, the volume/MA study, daily previous-session pivot points and percentage-deviation
ZigZag are now exposed through the engine, WASM and TypeScript APIs, with pure and incremental
rebuild coverage. The all-runtime-mutation fixture exercises every current indicator state,
including pivot, ZigZag and VWAP bands, and the fixed-value reference fixture covers every output
family and pivot variant. Engine persistence and full-recompute fixtures cover the same catalog;
the completed B4 closure includes the host-owned F4 trade/depth binding bridge. CT6 now
uses one bounded chart-level comparison anchor for percentage/indexed geometry and exposes an
engine-owned per-series legend snapshot; the Rust fixture and browser public API path cover exact
anchor values, latest values, and percent changes without duplicating canonical rows. CT1's
histogram columns, transparent-body hollow candles, stepped lines, point markers, and high-low
bars are covered by shared frame fixtures plus the browser package path; the bar path exposes a
typed `close_visible` style flag through Rust, WASM and TypeScript so disabling both OHLC ticks
produces a high-low bar while retaining its vertical range body. Browser compatibility coverage
round-trips the high-low, stepped-line, and point-marker options; existing backend parity and GPUI
matrix fixtures cover marker execution across Canvas2D, WebGPU, and GPUI.
CT2 keeps raw OHLC canonical for `series_data`, crosshair, and trading while an engine-owned,
generation-keyed Heikin Ashi projection feeds candlestick geometry, autoscale, last-value chrome,
and candle direction colors; engine and browser fixtures verify the projection and raw-data split.

### B5 — Non-time bars and replay

**Scope:** F1, OF14, CT3, CT4, PD2 for bars and tape. **Depends on:** B3. **Status:** complete (2026-09-27).

This is the largest architectural change in the plan.

- [x] **F1** Bar-sequence domain: each logical index is a bar with open and close time in
      microseconds; labels, crosshair, ticks and gaps derive from bar times; drawings, alerts,
      trading lines and markers store bar plus time and rebase on prepend and rebuild; declared
      rules for which series may share a non-time pane.
- [x] One shared tick, volume and range aggregator used by candles and footprint; footprint
      trade-count and volume policies become chart-integrated.
- [x] **OF14 / CT3** Tick, volume and range candles, with footprint on the same bars.
- [x] **CT4** Renko (fixed box or ATR), Line Break, Kagi, Point & Figure.
- [x] **PD2** Replay clock supplied by the host; replay cursor; masking of everything after the
      clock in every series, study, footprint cell and marker; checkpoint-based seek backward with
      reported cost; bulk ordered ingest for trades and bars; live and replay share code paths.
- [x] PD7 no-look-ahead verified in replay fixtures.
- [x] `perf_gate` covers tip append without rebuilding closed bars and 100× replay with flat memory.
- [x] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

Current F1 slice (2026-09-26): the existing engine-owned footprint aggregator now publishes a
logical bar index with each bar's full-resolution open/close microsecond bounds through a
read-only `bar_sequence` view. `BarSequenceMapping` now rebases anchors across ordered
prepend/rebuild sequences without collapsing duplicate second labels. This records the identity
boundary for the non-time axis, and the shared footprint aggregator now validates tick-grid range
bar boundaries. The chart engine now projects trade-count, volume, and range footprint bars through
chart-local logical row keys with a full-resolution sequence sidecar; the WASM and TypeScript APIs
round-trip those policies, and native/browser fixtures cover labels, crosshair lookup, and logical
keys. Non-time tip updates now replace only the affected suffix (with a full path when retention
can shift the prefix), and derived delta studies and trade-bubble markers use the same logical keys.
Value queries and transient trading/event overlays now resolve timestamp labels through the same
sidecar. Non-time sequence rebuilds also rebase committed, pending, drag, brush, and drawing-history
logical anchors through the full-resolution bar mapping, and persistence now carries an optional
bounded open/close-microsecond sidecar for those drawing anchors. Ordinary candlestick and OHLC-bar
series can now bind to the same chart-level stream as footprint and studies; stream-identity bulk
replacement and live batches classify and aggregate once, then update every dependent through the
same full or incremental path. The binding rejects scalar presentations and independent retention
caps that would misalign the shared logical domain. Engine and packaged-browser fixtures cover
trade-count and volume candles, stream-only ingestion, logical-axis growth, and dependent telemetry.
The same sequence owner now builds fixed-box and Wilder-ATR Renko, N-line Break, Kagi, and
Point & Figure projections from bounded canonical OHLC source rows. Ordered source updates replace
only the affected projection suffix, replay rebuilds from the eligible source prefix, and Kagi and
Point & Figure lower to the shared draw-list contract without backend-specific state.
The chart-wide PD2 clock now masks ordinary host rows, indicators, every shared-tape dependent,
sparse stepped releases, and transient host/trading events without discarding canonical future
input. One shared replay cursor is emitted through the ordered frame. Forward movement uses the
ordinary live update path; backward seeks restore the nearest 1,024-trade checkpoint from a bounded
64-checkpoint set and report only replayed suffix work. Columnar `update_typed` and typed trade
batches are the bulk replay boundaries. Engine and packaged-browser fixtures verify no-look-ahead,
checkpoint equivalence, future-ingest isolation, and clipped host windows. The release `perf_gate`
passes 100× replay at 1.25 ms per clock-advance/frame on the measured machine with flat
steady-state retained memory. The final B5 run passed the complete Rust, WASM/package, GPUI
parity/replay, Chromium, and release-performance gates.

**Exit:** the F1 and PD2 exit criteria pass: many-bars-per-second and gap fixtures render on every
executor, drawings survive prepend and rebuild, and seek-back equals a fresh load to the same
clock.

### B6 — Depth

**Scope:** F3, OF15, OF16, OF17, OF18, PD8, PD9, PD2 for depth. **Depends on:** B3 and B5.
**Status:** complete (2026-09-27).

- [x] **F3** Order-book model: snapshot and incremental level ingest with tick-grid validation,
      sequence-gap detection with typed resync requests, bounded live book, time-bucketed history
      ring, queries (best bid/ask, size at price, cumulative depth, imbalance), and typed columnar
      WASM ingest.
- [x] **PD8** Optional per-level order counts in depth ingest; typed microstructure event markers
      (iceberg refill, pulled liquidity, size cluster, sweep) with caps and LOD collapse. Detection
      stays in the platform.
- [x] **OF15 / PD9** Liquidity heatmap with color scaling, thresholds and trades overlaid, lowered to
      a texture/image primitive with incremental live-edge column updates on every executor.
- [x] **OF16** DOM ladder data model for non-Aeris hosts.
- [x] **OF17** Depth studies: book imbalance, cumulative depth curve, minimum-size and
      distance-from-touch filters.
- [x] **OF18** Time-and-sales view model for non-Aeris hosts.
- [x] **PD2** Replay extended to depth checkpoints and heatmap buckets.
- [x] `perf_gate` covers depth-update soak, heatmap frame and upload budgets.
- [x] `docs/Architecture.md` updated; full gate green; batch committed and pushed.
- [x] Milestone evidence: screenshots, accessibility review and recorded benchmarks for depth.

Implementation evidence: one keyed `DepthBook` owns fixed-grid bid/ask levels, exact provider
sequence continuity, optional order counts, bounded near-touch retention, a bucket/cell-capped
history ring, typed resync fencing, host event markers, replay tape and checkpoints. Ladder,
cumulative-depth and imbalance queries derive from that owner; time and sales derives newest-first
from the existing classified trade tape. The browser boundary ingests snapshots and batches through
parallel typed arrays with split `u64` words and returns exact identities as decimal strings.

The heatmap applies thresholds and color scaling before upload, packs finalized history into stable
absolute 32-column images, and replaces only its one-column live edge on book updates. The shared
underlay image and top-layer event primitives execute unchanged through Canvas2D, WebGPU, native,
and GPUI; ordinary trade series remain above the heatmap. Replay restores the nearest capped
1,024-event checkpoint, masks future ladder/study/heatmap/marker state without discarding live
input, and reports suffix work.

The final release Target L ran two 1.2-million-update passes: the worst 100,000-row batch was
10.05 ms, frame construction was 0.34 ms, the 512-bucket view used 17 images, its live-edge payload
was 512 bytes, and retained depth memory stayed flat at 66.03 MiB. The GPUI release adapter gate
lowered the same dense heatmap's 17 image runs at 0.053 ms p99 against 2 ms. The focused browser
fixture captured the WebGPU heatmap/marker view, and the unified accessibility contract remained
green because depth adds no DOM focus target or live announcement. `docs/Depth.md` records the
reproducible milestone details. The completed gate passed Rust fmt/clippy/tests, WASM lint,
package build/type/API/namespace/release/pack checks, GPUI parity/replay, 302 Chromium tests with
three intentional machine-only skips, and both release performance gates.

**Exit:** the F3, PD8 and PD9 exit criteria pass: deterministic book replay, gap fixtures request
resync, flat memory under soak, and the heatmap holds the target refresh rate on GPUI within parity
tolerance of the rectangle reference.

### B7 — Profiles and resampling

**Scope:** F6, OF3–OF8, OF10, CT5. **Depends on:** B2, B3 and B4. **Status:** open.

- [ ] **F6** Engine-owned OHLCV resampling with host-supplied session boundaries and explicit
      timezone policy.
- [ ] **OF3** Session, daily, weekly and composite volume profiles with developing POC/VAH/VAL.
- [ ] **OF4** Fixed-range volume profile drawing.
- [ ] **OF5** Anchored volume profile drawing.
- [ ] **OF6** Naked POC and value-area extension until touched.
- [ ] **OF7** Delta profile and bid/ask split profile.
- [ ] **OF8** TPO / Market Profile: letters or blocks, initial balance, single prints, POC, value
      area, split/merge sessions.
- [ ] **OF10** Anchored VWAP drawing with bands.
- [ ] **CT5** Higher-timeframe overlay candles.
- [ ] Multi-timeframe study inputs (for example a daily RSI on a 5-minute chart).
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.

**Exit:** tape and candle-mode profiles match reference fixtures, session boundaries come only from
the host, and multi-timeframe studies rebuild deterministically.

### B8 — Drawing catalog expansion

**Scope:** every tool in the drawing catalog not yet delivered. **Depends on:** B2.
**Status:** open.

Every tool implements the F5 contract with schema, persistence, hit-testing and executor parity.

- [ ] Lines: ray, extended line, info line, trend angle, cross line, arrow line.
- [ ] Channels: parallel, regression trend, flat top/bottom, disjoint.
- [ ] Fibonacci: retracement, trend-based extension, channel, time zones, trend-based time, speed
      resistance fan and arcs, circles, spiral, wedge.
- [ ] Pitchforks: Andrews, Schiff, modified Schiff, inside, pitchfan.
- [ ] Projection and measuring: forecast, bars pattern, price range, date range, date and price
      range, projection.
- [ ] Annotations: anchored text, note, price note, callout, comment, price label, signpost, flag,
      arrow markers, bounded icon stamps.
- [ ] Gann: box, square, square fixed, fan.
- [ ] Patterns: XABCD, cypher, ABCD, head and shoulders, triangle, three drives.
- [ ] Elliott waves: impulse, correction, triangle, double and triple combinations with degree
      labels.
- [ ] Cycles: cyclic lines, time cycles, sine line.
- [ ] Shapes: rotated rectangle, ellipse, circle, triangle, arc, curve, double curve, polyline,
      highlighter, with shared geometry on every executor.
- [ ] Full gate green; batch committed and pushed.

**Exit:** every catalog tool is placeable, editable through its schema, persisted and identical on
every executor.

### B9 — Breadth and extension

**Scope:** I2, I3, I4, OF13. **Depends on:** B3 and B4. **Status:** open.

- [ ] **I2** Breadth indicator tier (see Indicator catalog).
- [ ] **I3** Structure tier: swing points, structure breaks, fair value gaps, order blocks,
      session and previous-period levels, opening range.
- [ ] **I4** Typed custom study API in Rust and TypeScript: inputs, parameters, outputs,
      incremental update and rebuild; the engine owns scheduling, bounds, styles, persistence and
      rendering.
- [ ] **OF13** Unfinished auctions, absorption and exhaustion markers with documented,
      parameterized, deterministic rules.
- [ ] `docs/Architecture.md` updated; full gate green; batch committed and pushed.
- [ ] Milestone evidence: screenshots, accessibility review, competitor comparison and recorded
      benchmarks for the whole plan.

**Exit:** the Definition of completion below is met.

## Scope and ownership

Headless means Aeris Charts owns semantics and pixels inside the chart, never application chrome:

| Aeris Charts owns | Hosts own |
| --- | --- |
| Validated data models (trades, depth, bars), aggregation, classification and derived studies | Market-data subscriptions, provider normalization, reconnection and resync requests |
| Indicator and order-flow math, incremental updates, bounded caches | Symbol search, watchlists, exchange calendars and session definitions |
| Drawing geometry, placement, handles, hit testing, snapping, text layout, undo/redo | Toolbars, settings dialogs, property panels, color pickers, context menus |
| Typed option schemas, defaults, validation and style templates as data | Where templates are stored and how users pick them |
| Persistence schemas and migrations for everything above | Account/cloud storage, cross-device sync, sharing |
| Ordered backend-neutral frame output for every executor | Window, DOM or GPUI layout around the chart |

A feature is not delivered until a host can build its complete UI from typed engine APIs without
reimplementing chart math, and every executor (GPUI, WebGPU, Canvas2D, native) renders it from the
same ordered frame. [Architecture.md](../docs/Architecture.md) remains the authority for current
ownership.

Aeris Terminal's `market_runtime` is the canonical owner of order books, trades and order-level
(market-by-order) state. Engine stores such as F2 and F3 are chart-side projections of the
platform's publications, never a second canonical market model. Aeris Terminal also owns these
outside the chart, so they are not engine work for that host:

- **DOM ladder.** A GPUI widget fed by the platform's canonical order book. OF16 remains for other
  hosts, such as browser consumers.
- **Time and sales.** Rendered by the platform from its own trade tape. OF18 remains for other
  hosts.
- **Trading lock for risk lockouts.** The host stops forwarding trading gestures
  (`trading_drag_start_at` and related calls), rejects drained `take_trading_intents`, and shows the
  lock in its own chrome. No engine state is required.

Out of scope: a Pine-style scripting language, a bundled UI kit, broker connectivity, datafeed
adapters, and news/fundamental data fetching. Custom studies are covered by the typed extension
API (I4) rather than an interpreter.

## Current baseline

Source-confirmed on 2026-09-24. This is the starting point, not a claim of completeness.

| Area | Present today | Evidence |
| --- | --- | --- |
| Footprint | Trade tape per series; aggressor classification (host side → quote → tick rule); bid/ask/unknown/total per level; Bid×Ask, Total and Delta cell modes; POC; diagonal and stacked imbalances; final/max/min delta; session cumulative delta per bar; three LODs; late-event and correction rebuild | `engine/src/footprint.rs`, `frame/footprint_geometry.rs`, [Footprint.md](../docs/Footprint.md) |
| Footprint bar policies | Time, trade-count and volume aggregation in Rust; only whole-second time bars are chart-integrated | `FootprintBarAggregation`, Footprint.md §3 |
| Volume profile | Visible-range profile computed from OHLCV candles; rows, value area, POC; at most 16 per chart; runtime-only | `engine/src/volume_profile.rs`, `indicators/src/volume_profile.rs` |
| Series types | Candlestick, bar, line, area, histogram, baseline, custom, feature (grouped/stacked bars, heatmap, HLC area, pretty histogram, background shade, stacked area, whisker box), footprint | `SeriesKind`, `FeatureSeriesKind` |
| Indicators | SMA, EMA, DEMA, TEMA, SMMA/RMA, HMA, VWMA, standard deviation, Donchian Channels, Keltner Channels, ADX/DMI, Parabolic SAR, SuperTrend, Ichimoku, EMA ribbon, WMA, Bollinger, RSI, MACD, Stochastic, ATR, VWAP; incremental state; outputs are ordinary series, so indicator-on-indicator chaining already works | `engine/src/indicators.rs`, `indicators/src/lib.rs` |
| Indicator input | `IndicatorInput` carries times, high, low, close and volume only; no open, no selectable price source (hl2, hlc3, ohlc4) | `IndicatorInput` |
| Drawing tools | Trend line, horizontal line, horizontal ray, vertical line, rectangle, text, brush, path, long position, short position; static tool catalog; magnet; straighten; bounded undo/redo | `drawings/tools.rs`, `drawings.rs` |
| Drawing styling | Common drawing contract with typed kind-option projections, stroke caps/extensions/fill, shared text/label layout, interval visibility, magnet modes and bounded level lists | `drawing_contract.rs`, `Drawing`, `frame/drawings.rs` |
| Drawing management | Selection, multi-select, lock/hide, z-order/group operations, naming, templates, clone/copy/paste, bulk removal, bounded undo/redo and revisioned sync payloads | `drawings.rs`, `drawing_contract.rs` |
| Trading and alerts | Positions, orders, brackets/OCO, drag intents, bracket from position drawing; alert lines (host evaluates) | `trading.rs`, `alerts.rs` |
| Trading labels | Order and position chips are engine-formatted (quantity, kind, PnL); no host-supplied label or badge text | `trading_geometry.rs` |
| Markers and executions | Series markers (circle, square, arrow up/down with optional text, size and price); point markers on line/area; trading executions drawn as B/S circles | `Marker`, `set_series_markers`, `frame/series_geometry.rs`, `TradingExecution`, `frame/trading_geometry.rs` |
| Line and scale variants | Stepped lines (`LineType::WithSteps`); hollow candles through transparent body colors; percentage and indexed-to-100 price scales; price lines with host titles | `draw_list.rs`, `price_scale_core.rs`, `PriceLine` |
| Native primitives | Series-attached vertical line, text and image watermarks, volume-profile handle | `native_primitives.rs` |
| General charts | Step interpolation, bubble, heatmap grid, column, axis-bound reference regions (general panes only, not the financial time axis) | `general_series.rs` |
| Workspace | Split-grid of chart cells with stable identities | `workspace.rs` |
| Persistence | V1 panes/drawings, V2 general datasets/series, V3 financial study bindings/styles; profiles remain host-recreated | `persistence.rs` |
| Telemetry | WASM `frame_stats` (CPU/GPU ms, draw calls, rebuild counters, buffer traffic); `ChartEngine::memory_usage` structural attribution | `wasm/src/telemetry.rs`, `EngineMemoryUsage` |
| Image export | Browser `take_screenshot` only; no native or GPUI image export | `packages/charts/src/types.ts` |
| Order book / depth | **Absent.** No Level 2 model, DOM, or liquidity heatmap. The feature heatmap accepts only host-precomputed cells and lowers each cell to its own rectangle primitive | `FeatureSeriesKind::Heatmap`, `HeatmapCell` |
| Non-time bars | **Absent on chart.** The shared time axis has one logical row per UTC second | Footprint.md §3 |
| Cross-chart sync | Revisioned drawing payload export/import with source identity and stable drawing IDs; host routes payloads between cells without echoing | `drawing_contract.rs`, `drawings.rs` |
| Replay | **Absent.** No playback cursor or future masking; hosts can only replace and append data | — |

## Architecture principles

- **One source of truth per market fact.** A trade tape or depth book is stored once per
  instrument stream and shared by every study and series derived from it. No study copies raw
  trades.
- **Derived state is disposable.** Bars, profiles, delta series and heatmap buckets can always be
  rebuilt from the retained source. Tip updates use incremental paths; historical corrections
  rebuild from a documented checkpoint and report the work done.
- **Bounded everything.** Every tape, ring, profile, level list, cache and drawing collection has an
  explicit cap, eviction rule and memory telemetry. Frame work is bounded by what is visible.
- **Typed, not stringly.** Each study and drawing kind has a typed option struct with validation and
  defaults. Hosts receive typed schemas (name, type, range, default) to generate their property
  panels; the engine never renders dialogs.
- **Shared geometry, not per-backend features.** New shapes (ellipses, arcs, arrows, level fills,
  bubbles) are lowered to existing or new backend-neutral primitives implemented by every executor
  before a feature is complete.
- **No invented data.** Nothing guesses aggressor sides, fabricates timestamps for non-time bars, or
  rounds off-grid prices. Unknowns stay unknown and are reported.
- Follow [AGENTS.md](../AGENTS.md): no speculative crates, traits, plugin registries or feature
  flags. Extract modules only when a real responsibility justifies it.

## Specifications

### Foundations

#### F1 — Logical bar identity for non-time bars

**Problem.** The time axis maps one logical row to one UTC second. Tick, volume, range, Renko,
Kagi, Point & Figure and Line Break bars can produce several bars within one second, or bars
whose position is not a function of time at all. Footprint.md already forbids faking timestamps.

**Required outcome.**

- A bar-sequence domain where each logical index is a bar with its own open and close time
  (microseconds), independent of whole-second alignment.
- Time labels, crosshair time, tick marks and gaps derive from bar open/close times, including
  many bars in one second and long gaps between bars.
- Drawings, alerts, trading lines and markers anchored to a non-time chart store both the logical
  bar and the time, and rebase deterministically when history is prepended or rebuilt.
- Define which series can share a pane with a non-time primary series. Recommended rule: series in a
  non-time pane must be derived from the same bar sequence (studies, footprint, delta), not
  independent time series. Mixing arbitrary time series requires an explicit mapping and is not
  silently aligned.
- Bar construction for tick, volume and range bars moves into one shared aggregator used by
  candles and footprint alike; footprint's existing trade-count and volume policies become
  chart-integrated.

**Exit.** Tick/volume/range candles and footprint render on every executor; many-bars-per-second
and gap fixtures pass; drawings survive history prepend and rebuild; performance gate shows tip
append without rebuilding closed bars.

#### F2 — Shared trade tape and tape-derived studies

**Problem.** The footprint series owns its tape. CVD, delta histograms, trade bubbles, tape-based
volume profiles and VWAP-from-trades must read the same classified trades without duplicating them.

**Required outcome.**

- A chart-level trade stream handle, keyed by host-defined instrument stream, retaining the
  canonical classified tape with the existing ordering, correction and retention rules.
- Footprint, candles built from trades (F1), and every order-flow study bind to the stream by
  identity. Removing the stream removes or invalidates dependents explicitly.
- Classification runs once per event; dependents receive the classified event and a revision.
- Per-dependent incremental state plus a documented rebuild path on historical mutation.
- Memory telemetry per stream and per dependent.

**Exit.** A footprint series and a CVD study share one tape (verified by memory telemetry), a late
trade updates both consistently, and retention evicts tape and derived state together.

#### F3 — Order-book (Level 2) depth model

**Problem.** A liquidity heatmap and DOM ladder need historical and live resting liquidity per
price level. No such model exists; the current heatmap series only accepts precomputed cells.

**Required outcome.**

- Ingest a full book snapshot and incremental level updates (price, side, size, sequence,
  timestamp). Prices validated against the tick grid.
- Sequence-gap detection that marks the book stale and emits a typed resync request to the host;
  the engine never invents missing levels.
- Live book state (bounded by a configurable number of levels around the touch) plus a
  time-bucketed history ring for heatmap rendering, with explicit bucket interval and caps.
- Queries: best bid/ask, size at price, cumulative depth, book imbalance over N levels.
- Typed columnar ingest at the WASM boundary; no per-update object conversion on the hot path.

**Exit.** Deterministic replay of a recorded snapshot-plus-update stream yields identical book
states; gap fixtures request resync; memory stays flat under a sustained update soak.

#### F4 — Study input model

**Problem.** Indicators only receive high, low, close and volume, and always use close as the price
source. Professional studies need open, selectable sources and multi-input bindings.

**Required outcome.**

- `IndicatorInput` gains open; bindings select a source: open, high, low, close, hl2, hlc3, ohlc4,
  hlcc4, or any existing indicator output (chaining already works through series identity).
- Studies may bind several inputs (price series, volume series, trade stream, depth book) with
  typed validation.
- Every study exposes a typed parameter schema and typed output descriptors (name, kind: line,
  histogram, band fill, markers, levels) so hosts generate settings UI and legends without
  hard-coded knowledge.
- Per-output style (color, width, line style, visibility, histogram colors, band fill) remains
  engine state and persists.
- Study bindings join the persistence schema (next version) instead of being host-recreated.

**Exit.** An RSI of hlc3, an SMA of that RSI, and a Bollinger band fill round-trip through
persistence and render identically on every executor.

#### F5 — Drawing model and customization contract

**Problem.** One flat struct serves every tool; fill, border, labels and bands are
rectangle-specific; text layout is hard-coded to the trend line and text tool. This cannot scale to
~80 tools with per-tool customization.

**Required outcome.** A drawing becomes a common core plus a typed per-kind option block. The
common contract applies to **every** tool unless a property is meaningless for its geometry, and
that exception is recorded in the tool's catalog entry.

| Group | Properties |
| --- | --- |
| Identity | Stable ID, user name, optional group ID, creation/modification revision |
| State | Visible, locked (no drag/edit, still selectable), z-order within the drawing layer (bring forward/backward/front/back) |
| Interval visibility | Show on selected interval ranges (seconds, minutes, hours, days, weeks, months, ticks, ranges), using host-supplied interval metadata |
| Stroke | Color with alpha, width, line style (solid, dotted, dashed, large dashed, sparse dotted), line cap/end style per end (none, arrow, circle) where the geometry has ends |
| Extension | Extend left/right for line-like tools; extend levels for level tools |
| Fill | Background enabled, color with alpha; per-zone fills for multi-zone tools |
| Text | Content (multi-line), font size, bold, italic, color, horizontal alignment, vertical alignment, placement relative to the geometry (above/below/on line, inside/outside shape), optional background box with border color/width and padding, wrap width |
| Labels and stats | Per tool: price, price change, percent change, ticks/pips, bar count, date/time range, duration, angle, distance, volume in range; each individually toggleable with label position |
| Coordinates | Numeric read/write of every anchor (time or bar plus price) through the typed API, so hosts can offer coordinate editors |
| Scale binding | Price scale (left/right/overlay), pane, magnet mode (off, weak, strong) |

Level-based tools (Fibonacci, Gann, pitchfork, position) add a level list: value, color, visible,
line style, fill-between-levels toggle and color, plus tool options such as reverse, log-scale
levels, show prices, show level values or percents, and label alignment.

Engine responsibilities around the contract:

- Text layout for every tool uses one shared layout path (measurement hook, alignment, placement,
  box, clipping) so hit boxes, editing caret anchors and rendering never disagree. The existing
  trend-line text behavior becomes one instance of it.
- Hosts receive a typed property schema per tool and read/write properties through
  `drawing_apply_options`-style atomic patches; invalid patches leave the drawing unchanged.
- Style templates are data: per-tool default overrides and named templates that the engine
  validates, applies at creation, and exports/imports. The host decides where templates live.
- Multi-select, clone, copy/paste as a serialized drawing payload, group move/lock/hide, and
  undo/redo of every property change as one history entry.
- Cross-chart sync (same symbol in several workspace cells) uses export/import of drawing payloads
  with revision tracking; the host coordinator routes changes and the engine prevents echo loops.
- Persistence migrates existing V1/V2 drawings into the new model losslessly.

**Exit.** Every existing tool supports the common text, stroke, state and label contract; a host
builds a generic property panel from schemas alone; persistence migrates old layouts; executor
parity fixtures cover text placement on lines, shapes and level tools.

#### F6 — Engine-owned OHLCV resampling

Hosts own feeds, but higher-timeframe views and multi-timeframe studies need deterministic
aggregation of a lower-timeframe source into higher-timeframe bars with host-supplied session
boundaries. This powers multi-timeframe study inputs, session and weekly profiles, and derived
chart types without host-side re-aggregation. Timezone and session policy come from the host
explicitly; the engine never uses the browser timezone.

### Platform contracts

The Aeris Terminal roadmap adds risk controls, session replay, trade review, order-level analytics
and fundamentals context. Rule evaluation, recording storage, data fetching and every panel or
dialog stay in the platform. These items are what the engine must provide so the platform renders
those features from typed APIs without reimplementing chart math.

#### PD1 — Host annotations on trading objects

**Problem.** Order and position chips show only engine-formatted quantity, kind and PnL. The
platform needs to show estimated queue position, fill likelihood, rule warnings ("breaks daily loss
limit at stop") and copier status on the same objects.

**Required outcome.**

- `WorkingOrder` and `TradingPosition` accept a bounded list of host annotations: short text,
  semantic tone (neutral, info, warning, danger), optional tooltip text and placement.
- Annotations are laid out with the existing chips by the shared trading geometry, clipped and
  hit-tested consistently; overflow collapses deterministically.
- Explicit caps on annotation count and text bytes per object; invalid annotations are rejected
  without changing the object.
- Annotations are presentation only. The engine never computes queue position or rule state.

**Exit.** An order line with two annotations renders identically on every executor, hit-tests to
the right annotation, and updates at live tick rates without rebuilding unrelated trading geometry.

#### PD2 — Replay playback contract

**Problem.** Session replay feeds recorded trades, depth and bars into charts at 1× to 100× speed,
seeks backward, and must never show data from after the replay clock.

**Required outcome.**

- A replay clock supplied by the host (microseconds). The engine draws a replay cursor and masks
  or omits everything after it in every series, study, footprint cell, heatmap bucket and marker.
- Seek backward resets derived state from the nearest documented checkpoint (F2, F3 and indicator
  checkpoints) rather than rebuilding full history; seek cost is reported.
- Bulk ordered ingest paths for trades, depth and bars sized for high-speed replay without
  per-event object conversion at the WASM or GPUI boundary.
- Live and replay charts use the same series and study code paths; replay is a data-source mode,
  not a forked renderer.

**Exit.** A recorded session replays at 100× with bounded frame work and flat memory in
`perf_gate`, seek-back equals a fresh load to the same clock, and no fixture shows post-clock data.

#### PD3 — Host event layer on the time axis

**Problem.** Economic releases, platform risk windows (no trading two minutes around a release),
session opens and contract roll dates must appear on charts. Drawings are user-editable and
persisted, so they are the wrong owner for host-generated context.

**Required outcome.**

- A non-persisted, non-editable host overlay layer with typed event markers (time, importance,
  short label, optional icon from a bounded host image set) and shaded time windows.
- Markers and windows render on price and study panes, collapse by LOD when dense, and hit-test to
  a host event identifier for tooltips.
- Explicit caps on markers and windows per chart.

**Exit.** Event markers and windows render identically on every executor, survive history prepend
and resampling, and never enter drawing persistence or undo history.

#### PD4 — Execution markers and round trips

**Problem.** Trade review needs every fill, grouped into entry-to-exit round trips with their result,
directly on the chart.

**Required outcome.**

- Execution marker variants: circle (current), arrow and triangle, optional size by quantity.
- Round-trip connectors from entry executions to exit executions with a host-supplied result label
  (for example "+3.25 pts, +$162.50"), colored by outcome.
- Hit-testing returns the execution or round-trip identifier; caps on executions and connectors
  per chart.

**Exit.** A day with many round trips renders at every LOD without overlapping labels beyond the
documented collapse rule, and hit-testing selects the correct round trip on every executor.

#### PD5 — Cross-chart synchronization

**Problem.** Linked charts (same symbol at several timeframes, or linked symbol groups) must share
crosshair position and optionally visible time range. `workspace.rs` deliberately shares no state.

**Shared contract.** This is the same capability as linked-chart synchronization in
[plan.md](plan.md) R4. It is built once in the shared engine layer, delivered here first for
financial charts, and later extended by R4 to general domains rather than duplicated.

**Required outcome.**

- APIs to read the local crosshair (time, price, pane) and to set an external crosshair that
  renders without being treated as local pointer input.
- APIs to read and set the visible time range.
- Synchronization events carry semantic values (financial time and price here; general domain
  values or declared index matching in plan.md R4) with an explicit mismatch policy, plus a source
  and revision so a bounded host coordinator can route events between independent charts without
  echo loops. Each receiving engine resolves the values against its own data, and disposal removes
  its subscriptions.
- Symbol linking remains host-owned; the engine exposes only crosshair and range primitives.

**Exit.** Two charts with different timeframes track one crosshair and one time range with no
feedback oscillation, on GPUI and in the browser. The contract needs no financial-only fields that
would block its reuse in plan.md R4.

#### PD6 — Native and GPUI image export

**Problem.** The platform journal attaches chart images to trades. Only the browser package can
export images today.

**Shared contract.** Image export is one frame-level capability for every chart kind and also
serves plan.md's frame-rendered export (R4 and the "equivalent frame exports" coverage row). State
exports (persistence) remain a separate gate.

**Required outcome.**

- Render a chart frame to an RGBA buffer at a requested size and scale on the native and GPUI
  paths, including or excluding the crosshair and trading layer, with the same composition rules as
  the browser `take_screenshot`.
- The export works from the ordered frame regardless of whether panes are financial or general,
  and documents which built-in chrome is included.
- Export never disturbs the live chart's state or frame pacing.

**Exit.** Exported images match on-screen output within the existing parity tolerances for a
financial chart and a general Cartesian chart.

#### PD7 — Sparse fundamental series on intraday charts

**Problem.** Weekly and monthly context (EIA inventories, CFTC Commitments of Traders, USDA
reports) must be shown beside intraday prices without look-ahead: a value becomes visible only from
its release time.

**Required outcome.**

- Step-after rendering from host-supplied release timestamps, with documented behavior for sparse
  points on second-based axes and across history gaps. Confirm whether `LineType::WithSteps`
  already provides these semantics and extend it only if it does not.
- Column or histogram presentation of the same series in its own pane, and value labels that show
  the as-of release.
- The engine never fetches or interprets fundamental data; it renders host series.

**Exit.** A weekly series on a one-minute chart changes value exactly at each release bar, with no
interpolation and no look-ahead in fixtures and replay (PD2).

#### PD8 — Order-level depth inputs and microstructure events on the chart

**Problem.** Rithmic supplies CME market-by-order data. Aeris Terminal's adapter already assembles
an order-level book but publishes aggregated levels. Queue position, iceberg detection, pulled
liquidity and order-size clustering are computed platform-side. The DOM shows them in the
platform's own widget; the chart must show them on price panes and the liquidity heatmap.

**Required outcome.**

- F3 depth ingest accepts the optional per-level order count the platform already carries, so
  heatmap cells and tooltips can show order counts beside size.
- A typed microstructure event marker (kind: iceberg refill, pulled liquidity, size cluster, sweep;
  price, time, size and host label) rendered in price panes and on the heatmap (OF15), with caps and
  LOD collapse.
- The engine does not implement detection rules. Deterministic detection lives in the platform,
  consistent with the OF13 rule that detections are never heuristic black boxes. Queue position on
  the chart is shown through PD1 order-line annotations.

**Exit.** A recorded order-level stream renders heatmap order counts and event markers identically
on every executor, and markers stay aligned with heatmap buckets after replay seeks.

#### PD9 — Depth heatmap rendering budget

**Problem.** OF15 at full resolution means thousands of price rows by hundreds of time buckets.
Lowering each cell to a rectangle, as the feature heatmap does today, will not hold high refresh
rates on dense books.

**Required outcome.**

- OF15 lowers visible buckets to a texture or image primitive (`Prim::Image`, the WebGPU
  textured-quad pipeline and its GPUI and Canvas2D equivalents), with incremental column updates
  for the live edge.
- Color scaling, thresholds and minimum-size filters remain engine semantics applied before
  upload.
- Frame and upload budgets are measured in release builds and added to `perf_gate`.

**Exit.** A dense recorded book renders at the target refresh rate on GPUI with bounded upload per
frame, and results match the rectangle-based reference within parity tolerance.

#### PD10 — Dense footprint text budget

**Problem.** Detailed footprint cells update many numeric text runs per frame during fast markets.
GPUI glyph shaping and atlas cost at that density is unmeasured. (The Aeris Terminal DOM ladder is
a platform widget, so its text performance is platform work.)

**Required outcome.**

- Measured release benchmarks for footprint text density on GPUI and WebGPU.
- Shared caching of repeated numeric runs where measurement shows shaping dominates, without
  changing text metrics or parity.

**Exit.** Documented frame budgets for a reference footprint view are met on GPUI and guarded by
`perf_gate`.

#### PD11 — Trading layer alignment with the platform runtime

**Problem.** The existing trading layer was built for a single implicit account with four order
kinds. Aeris Terminal's trading runtime has several simulated and live accounts, a trade copier,
fixed-point prices, and bracket templates with trailing and break-even stops. Without these
corrections the host must encode accounts in identifiers and approximate trailing stops as plain
stops.

**Required outcome.**

- `WorkingOrder`, `TradingPosition`, `TradingExecution` and `TradingIntent` carry an optional
  host account identifier (bounded, validated like other identifiers). The host sets a visible
  account filter (one, several or all); hidden objects are neither rendered nor hit-tested.
  Intents created from drawings or empty space carry the host-set active account; intents on an
  existing object carry that object's account.
- Trailing stops and break-even-armed stops are presentation variants of stop orders: the host
  supplies the current trigger price and the trail offset or arm state; the engine renders the
  variant and never computes trailing or arming itself. Local-versus-server management is shown
  through PD1 annotations.
- Price-bearing intents also report the price as an integer tick index from the instrument tick
  size, so fixed-point hosts convert without floating-point rounding. Behavior without a tick size
  is unchanged.
- The order-state contract is documented: pending modify shows the requested price until the host
  resolves the intent, rejected intents return to the last host-confirmed price, and partially
  filled and pending-cancel states render distinctly. Fixtures cover each transition.
- All additions are optional fields with serde defaults; existing snapshots and hosts keep
  working unchanged.

**Exit.** A chart showing two accounts' orders filters to one account, hit-tests only visible
objects, renders a trailing stop from host-supplied trigger updates, and emits drag intents whose
tick index round-trips exactly to the host's fixed-point price on every executor.

#### Platform feature to engine prerequisite map

| Platform feature | Engine prerequisites | Batch |
| --- | --- | --- |
| Basic chart trading (orders, positions, drag to modify, brackets from drawings) | Existing trading layer; host wiring only | — |
| Multi-account chart trading, copier, trailing and break-even stops | PD11 | B1 |
| Prop-firm rules, pre-trade checks, lockouts | PD1 for warnings on order lines; the lock itself is host-owned | B1 |
| Economic calendar and risk windows | PD3 | B1 |
| Fundamentals dashboards on charts | PD7; existing panes, histogram and stepped lines | B1 |
| Linked charts and symbol groups | PD5 | B1 |
| Footprint, volume profile and CVD panels | Existing footprint and profile; F2, OF1–OF3, OF12, PD10 | B3, B7 |
| Big-trade bubbles and sweeps | F2, OF11; PD8 for sweep markers | B3, B6 |
| Custom studies and study scene objects | I4, F4; platform study roadmap Phases F–G | B4, B9 |
| Tick, volume and range charts | F1, OF14, CT3 | B5 |
| Session replay and trade review | PD2, PD4, PD6; F1 for sub-second tape display | B1, B5 |
| Liquidity heatmap | F3, OF15, PD8, PD9 | B6 |
| Queue position, icebergs, pulled liquidity on charts | PD8; PD1 for queue position on the order line | B1, B6 |
| DOM ladder and time and sales | None; Aeris Terminal platform widgets | — |

## Catalogs

### Order-flow catalog

Each item names its data source and dependency. "Candle" means the item also works in a
candle-only approximation mode that is clearly labeled as such; tape items never silently fall back.

| ID | Item | Source | Depends on | Batch | Notes |
| --- | --- | --- | --- | --- | --- |
| OF1 | Cumulative volume delta (CVD) pane: candles or line, session/continuous/anchored reset | Tape | F2 | B3 | Session delta already exists inside footprint bars; expose it as a proper series |
| OF2 | Bar delta histogram, delta %, max/min delta, volume split (buy/sell/unknown) histogram | Tape | F2 | B3 | |
| OF3 | Session, daily, weekly, composite volume profiles with developing POC/VAH/VAL lines | Tape (candle mode available) | F2, F6 | B7 | Replaces candle-only approximation for tape users |
| OF4 | Fixed-range volume profile drawing | Tape or candle | F2, F5 | B7 | Drawing whose statistics come from the engine |
| OF5 | Anchored volume profile drawing | Tape or candle | F2, F5 | B7 | |
| OF6 | Naked (virgin) POC and value-area level extension until touched | Tape | OF3 | B7 | |
| OF7 | Delta profile and bid/ask split profile | Tape | OF3 | B7 | |
| OF8 | TPO / Market Profile: letters or blocks, initial balance, single prints, POC, value area, split/merge sessions | Candle or tape | F6 | B7 | Period and session boundaries are host-supplied |
| OF9 | VWAP standard-deviation and percent bands; session/weekly/monthly reset | Candle or tape | F4 | B4 | Extends existing VWAP |
| OF10 | Anchored VWAP drawing with bands | Candle or tape | F4, F5 | B7 | |
| OF11 | Large-trade bubbles and volume dots: size by volume, color by side, threshold filters, aggregation of consecutive prints | Tape | F2 | B3 | New bounded marker primitive path |
| OF12 | Footprint variants: profile-in-bar, volume ladder, horizontal imbalance mode, delta-only, bid/ask histogram cells | Tape | F2 | B3 | Extends existing footprint LOD |
| OF13 | Unfinished auctions, absorption and exhaustion markers with explicit documented rules | Tape | OF12 | B9 | Rules must be deterministic and parameterized, never heuristic black boxes |
| OF14 | Tick, volume and range candles; footprint on the same bars | Tape | F1 | B5 | Trade-count and volume aggregators already exist |
| OF15 | Liquidity heatmap (resting depth over time) with color scaling, thresholds, and trades overlaid | Depth + tape | F3, OF11 | B6 | Bounded by visible time buckets × visible price rows |
| OF16 | DOM ladder data model: price ladder, bid/ask size, recent volume at price, own orders | Depth + trading | F3 | B6 | For non-Aeris hosts; chart-side panel primitive |
| OF17 | Depth-derived studies: book imbalance, cumulative depth curve, minimum-size and distance-from-touch filters | Depth | F3 | B6 | |
| OF18 | Time and sales data view model (bounded recent prints with filters) | Tape | F2 | B6 | For non-Aeris hosts; host renders the list |

### Chart types

| ID | Type | Depends on | Batch |
| --- | --- | --- | --- |
| CT1 | Hollow candles, columns, high-low bars, step line, line with markers | — | B4 |
| CT2 | Heikin Ashi (derived series with real OHLC exposed separately for trading and crosshair) | F4 | B4 |
| CT3 | Tick, volume and range bars | F1 | B5 |
| CT4 | Renko (box size fixed or ATR), Line Break, Kagi, Point & Figure | F1 | B5 |
| CT5 | Higher-timeframe overlay candles on a lower-timeframe chart | F6 | B7 |
| CT6 | Symbol comparison overlays: several instruments on one pane, shared comparison anchor bar, per-symbol legend values | — (percentage and indexed-to-100 scale modes already exist in `price_scale_core.rs`) | B4 |

### Indicator catalog

Current: SMA, EMA, DEMA, TEMA, SMMA/RMA, HMA, VWMA, standard deviation, CCI, Williams %R, Stochastic RSI, ROC, Momentum, OBV, CMF, MFI, Volume/MA, Donchian Channels, Keltner Channels, ADX/DMI, Parabolic SAR, SuperTrend, Ichimoku, EMA ribbon, WMA, Bollinger, RSI, MACD, Stochastic, ATR, VWAP. Each new indicator
ships with incremental state, rebuild tests, typed schema, persistence and an independently
computed reference-value fixture.

| Tier | Batch | Indicators |
| --- | --- | --- |
| I1 — core professional set | B4 | Volume (as a study with MA), OBV, ADX/DMI, Parabolic SAR, SuperTrend, Ichimoku, Keltner Channels, Donchian Channels, CCI, Williams %R, Stochastic RSI, ROC/Momentum, MFI, CMF, HMA, VWMA, DEMA, TEMA, SMMA/RMA, standard deviation, pivot points (standard, Fibonacci, Camarilla, Woodie, DeMark), ZigZag |
| I2 — breadth | B9 | Aroon, Awesome Oscillator, Chande Momentum, Chaikin Oscillator, Coppock, DPO, Elder Force, Ease of Movement, Fisher Transform, Historical Volatility, KST, Klinger, Linear Regression channel/curve, Mass Index, Ultimate Oscillator, TRIX, TSI, Vortex, Envelopes, ALMA, KAMA, McGinley Dynamic, Chop Zone/Choppiness, Bollinger %B and Bandwidth, ATR bands, Accumulation/Distribution, Price Volume Trend, Volume Oscillator, Relative Volume |
| I3 — structure | B9 | Swing highs/lows, market structure breaks, fair value gaps, order blocks, session highs/lows, previous day/week/month levels, opening range |
| I4 — extension API | B9 | Typed custom study API in Rust and TypeScript: declare inputs, parameters and outputs; provide incremental update and rebuild functions; engine owns scheduling, bounds, styles, persistence and rendering |

Multi-timeframe inputs (a daily RSI on a 5-minute chart) depend on F6 (B7). Alert conditions on
study outputs remain host-evaluated; the engine exposes the values and crossing snapshots.

### Drawing tool catalog

Every tool implements the F5 common contract. Placement types refer to `DrawingPlacement` in
`drawings/tools.rs`; new placement kinds are added only when a tool genuinely needs one. ✓ marks
tools that exist today and are migrated in B2; everything else is delivered in B8, except the
volume-based tools, which are delivered in B7.

| Family | Tools |
| --- | --- |
| Lines | Trend line ✓, ray, extended line, info line (price/bars/percent/angle), trend angle, horizontal line ✓, horizontal ray ✓, vertical line ✓, cross line, arrow line |
| Channels | Parallel channel, regression trend (with deviation settings), flat top/bottom, disjoint channel |
| Pitchforks | Andrews, Schiff, modified Schiff, inside pitchfork, pitchfan |
| Fibonacci | Retracement, trend-based extension, channel, time zones, trend-based time, speed resistance fan, speed resistance arcs, circles, spiral, wedge |
| Gann | Gann box, square, square fixed, fan |
| Patterns | XABCD, cypher, ABCD, head and shoulders, triangle pattern, three drives |
| Elliott waves | Impulse (12345), correction (ABC), triangle (ABCDE), double and triple combination, with degree labels |
| Cycles | Cyclic lines, time cycles, sine line |
| Projection and measuring | Long position ✓, short position ✓, forecast, bars pattern (ghost copy), price range, date range, date and price range, projection |
| Volume-based | Fixed-range volume profile (OF4), anchored volume profile (OF5), anchored VWAP (OF10) |
| Shapes | Rectangle ✓, rotated rectangle, ellipse, circle, triangle, arc, curve, double curve, polyline, path ✓, brush ✓, highlighter |
| Annotations | Text ✓, anchored text (screen-anchored), note, price note, callout, comment, price label, signpost, flag mark, arrow markers (up/down/left/right), icon/emoji stamp from a host-provided bounded image set |

Management features (object tree, multi-select, group operations, clone, copy/paste, templates,
cross-cell sync, bulk remove) belong to F5 and ship in B2, not per tool.

## Verification and evidence

Every checklist item needs, before its batch closes:

- Deterministic engine tests for math, including an independently computed reference fixture,
  edge cases (empty, one bar, gaps, unknown sides, off-grid rejections, corrections) and rebuild
  equivalence (incremental result equals full rebuild).
- Frame fixtures for ordering, clipping and LOD; parity across GPUI, WebGPU, Canvas2D and native;
  GPUI replay for executor changes.
- Browser Playwright tests through the published package for browser-facing APIs.
- Persistence round trip and migration from the previous schema version.
- Performance evidence in release builds added to `perf_gate`: tip update cost independent of
  history length, frame work bounded by visible bars/levels/buckets, steady-state allocation,
  retained memory under retention caps, and depth-update soak for F3.

The complete gates in [AGENTS.md](../AGENTS.md) run once at the end of each batch, before its commit.
`docs/Architecture.md` is updated in the same commit as any ownership or data-flow change.
Reference behavior comes from public documentation and observed behavior only; no copied
implementation code or assets (see the licensing rule in AGENTS.md).

## Definition of completion

Aeris Charts is a complete headless trading chart engine for this plan when a host can build a
professional order-flow and technical-analysis workstation using only typed engine APIs. That means
footprint, CVD, profiles, TPO, liquidity heatmap, DOM, non-time bars, the I1–I3 indicator catalog,
the full drawing catalog with per-tool customization, and the platform contracts PD1–PD11, with
identical results across every backend, bounded resources, deterministic persistence, and measured
performance evidence. Until then, report delivered items and remaining gaps precisely against the
batch checklists and catalog IDs above.
