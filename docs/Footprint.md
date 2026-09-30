# Footprint / Numbers Bars Design

This document is the durable design contract for GitHub issue #23. It describes the tick-truth
model that Aeris Charts uses for professional footprint series. `Architecture.md` remains the
authority for crate ownership and dependency direction.

## 1. Trade event and ordering model

A footprint series ingests trades, never synthetic OHLC clusters. Each event carries:

- a signed Unix timestamp at microsecond resolution;
- tick-aligned price and positive volume;
- host-provided aggressor side (`buy`, `sell`, or `unknown`);
- optional contemporaneous bid and ask;
- optional provider sequence and stable trade ID;
- opaque condition bits for later microstructure extensions; and
- an optional host-defined session ID.

Microseconds preserve exchange ordering while remaining exactly representable for contemporary
dates in JavaScript numbers. The browser boundary rejects unsafe integers. Equal timestamps sort by
provider sequence, then stable input order. A trade ID makes replay idempotent and permits a provider
correction to replace the prior event. A batch containing the same trade ID twice is rejected
atomically rather than choosing an undocumented winner.

The retained trade tape is authoritative. Derived bars may be discarded and rebuilt. Tip events use
an incremental path. A late event or correction is inserted in canonical order; the shipped path
rebuilds the complete retained series once and reports that work. It never patches final bar totals
while leaving the Max/Min Delta path stale. A measured suffix-checkpoint path may replace that full
reconstruction later without changing the public result.

Prices must lie on the configured integer tick grid. Aeris rejects off-grid data rather than
rounding financial truth silently. Volume is finite and positive. A session-ID change closes the
current bar and resets session cumulative delta.

## 2. Aggressor-side classification

Classification is deterministic and ordered by authority:

1. A host-provided `buy` or `sell` side is accepted as authoritative.
2. For `unknown`, a trade at or above the supplied ask is a buy; at or below the supplied bid it is
   a sell.
3. Otherwise the tick rule compares with the previous canonical trade: higher is buy, lower is
   sell, and an unchanged price carries the previous classified side.
4. If none of those rules resolves the event, it remains unknown.

Unknown volume contributes to price-level, bar, and POC total volume, but not bid volume, ask volume,
delta, imbalance, or cumulative delta. Aeris never guesses a side merely to make a cluster look
complete. Historical reconstruction reruns classification in canonical event order, so inserting a
late event can correctly change the classification of later ambiguous events.

## 3. Aggregation and accounting

The aggregation model supports aligned time bars, fixed trade-count bars, whole-trade volume bars,
and tick-grid range bars. A trade is never split to hit an exact volume or range threshold. A new session always starts a new
bar. Time bars align to the configured anchor, or, once the host anchors the stream to exchange
session windows, restart at every window open in the chart's exchange time with out-of-window
prints folded into the nearest bar of their trading day or excluded (see `Public_api.md`, "Ticks to
candles"). Time bars retain their whole-second display projection; trade-count, volume, and range bars
are chart-integrated through a chart-local logical row key and an engine-owned sequence sidecar.
That sidecar carries each bar's full-resolution open/close microsecond bounds for labels, crosshair,
and visible-range lookup, so several bars in one second are never assigned false timestamps.
`BarSequenceMapping` provides deterministic anchor rebasing when a sequence is prepended or rebuilt.
The chart retires the sidecar when its last non-time footprint and dependent are removed, so a
later time-only series cannot inherit stale logical labels.
Non-time tip updates replace only the affected suffix and keep derived delta studies on the same
logical row keys, including capped series: retention drops row keys from the front without re-keying,
so later tips, studies, and bubble markers continue from the projection's first retained key, and a
footprint, candle/bar, or study bound after a trim installs from that same key. Trade
bubbles also use logical bar indices while their aggregation windows retain microsecond comparison
precision.
Chart value snapshots and series queries expose the corresponding UTC-second label instead of the
internal row key.
Incremental replay and release performance evidence remain part of the B5 performance exit.

Each bar retains OHLC, bid/ask/unknown/total volume, trade count, final delta, delta percentage,
session cumulative delta, and sorted price levels. Each level retains bid, ask, unknown, total, and delta. Trades are
validated against the instrument `tick_size`; integer row identity is
`floor(round(price / tick_size) / ticks_per_row)`, avoiding repeated floating-point price comparisons.
`ticks_per_row` (default 1) groups adjacent ticks into one row so dense instruments stay legible; a
level's `price` is the lowest tick of its row, and bar OHLC keeps exact trade prices.

Running bar delta begins at zero. Each classified buy adds volume and each classified sell subtracts
volume. Max Delta is the highest running value observed after each event (including initial zero);
Min Delta is the lowest. They are retained during live aggregation and recomputed from the tape after
historical mutation. Final delta is not used to approximate either extreme.

POC is the level with maximum total volume. Ties select the level nearest the bar close, then the
lower level. This rule is stable across live and historical construction.

The professional diagonal imbalance rule compares:

- ask at level `p` against bid at `p - 1 tick`; and
- bid at level `p` against ask at `p + 1 tick`.

The dominant side must meet the configured minimum volume and the configured ratio. Zero opposite
volume qualifies once the minimum is met. Missing intermediate levels break a stack. Every member of
an adjacent run at least `consecutive_levels` long is marked as a stacked bid or ask imbalance.
Horizontal and alternative imbalance modes are visual projections only; they cannot alter the stored
bid/ask truth.

The visual aggregation mode is independent of bar construction: hosts can request Bid × Ask, Total,
Delta, profile-in-bar, volume-ladder, horizontal-imbalance, or bid/ask-histogram cells from the same
data without rebuilding the tape.

### Shared chart tape and derived studies

`ChartEngine::add_trade_stream` creates one bounded stream keyed by a host instrument identity.
The stream owns classification, canonical ordering, corrections, retention, and a monotonic revision;
footprints, CVD, delta histograms, and bubble markers bind to that identity rather than retaining a
second provider-event tape. CVD supports session, continuous, and anchored resets. Retention never
rewrites the CVD values of retained bars: the stream carries the evicted bars' session and continuous
cumulative delta as seeds, and an anchored CVD keeps the base its evicted bars established (a CVD
created after its anchor bar was evicted anchors at the first retained bar). Delta dependents
read final delta, Max/Min Delta, delta percentage, and bid/ask/unknown volumes from the same bars,
and the volume dependent reads each bar's total volume for ordinary tick-built candles.

A stream is the only writer of its dependents. A trade-bound candle or bar and the CVD, delta, and
volume studies refuse every host data write (set, install, update, batch, merge, sequenced update,
per-point colors, pop), exactly like a footprint, so a stray write can no longer rewrite the row
keys that every other presentation of a non-time stream continues from. One series has one engine
writer: `configure_footprint_series` returns `FootprintError::SeriesOwned` for a series that a
stream, a study, a resampler, or synthetic bars already write, and
`bind_trade_bar_series_to_stream` returns it for a candlestick or bar that a resampler, synthetic
bars, or a study converted to a candle writes (a footprint or scalar study fails the candle-kind
check first, with `UnsupportedTradeBarSeries`; a bound candle rebinds to another stream freely).
Resampling and synthetic-bar configuration refuse such a series as their target. The tip and
rebuild paths write through the engine's internal installers, not the host entry points, so the
guard costs a tip nothing. Retention still follows the stream: a bound candle refuses a `max_points`
cap, while a study keeps its own.

Large-trade bubbles are bounded marker dependents: translucent circles centred on the traded price,
colored by aggressor side, with area proportional to volume relative to the largest retained bubble.
They support minimum-volume filtering, optional same-side same-price consecutive-print aggregation
within one bar (merged volume sets the size), and a hard marker cap that retains the newest prints.
On time bars a bubble carries the open of the bar holding its print, so it paints on that bar even
when session anchoring folds an auction or lunch print into it; an excluded print has no bubble, and
an id-less marker is named by its print's own second. Rebuilds and live tips run the same resumable
fold, so both place, merge, and size bubbles identically. Late events refresh all dependents after
one canonical rebuild; stream telemetry reports revision, retained capacity, dependent count,
dependent rebuilds and incremental updates, and lifetime work counters for study rows computed, bar
rows projected, bubble trades folded, and bubble marker sizes computed.

## 4. Rendering and LOD

Footprint geometry is constructed in `aeris_charts_engine` as ordinary backend-neutral primitives.
Backends preserve the resulting order, clipping, alpha, text alignment, and pixel coordinates; no
executor recalculates POC, delta, or imbalance.

At readable density, each visible bar paints:

1. optional delta-tinted bar background;
2. per-level bid and ask cells (or the selected Total/Delta layout);
3. POC emphasis;
4. bid/ask imbalance and stacked-imbalance emphasis;
5. aligned volume text; and
6. a bounded two-line bar summary containing final, Max, and Min Delta plus total, bid, and ask
   volume.

Text uses the chart's resolved font family and centered columns. Colors are resolved before frame
execution, and the default text color follows live chart-theme changes. Max/Min Delta are visible in
the summary and queryable even when LOD hides text.

LOD is selected from horizontal bar width and vertical tick-row height:

- **Detailed:** bid/ask (or selected mode) text, cells, POC, imbalance, and summary.
  The two-line summary is omitted while the bar is too narrow to fit it without
  overprinting neighboring bars (`bar_spacing < 9 × font_size`).
- **Cells:** colored level cells and POC/stacked emphasis, without glyphs.
- **Summary:** one delta/volume body plus POC marker per bar.
Frame work is bounded to the visible logical range. At the densest zoom, Summary mode is already one
body and one POC marker per visible bar; hidden text does not enter the draw list or text atlas.
Every backend therefore receives exactly the same chosen LOD.

## 5. Storage, invalidation, and recovery

One chart-level stream owns one canonical trade tape and one derived bar vector. A footprint series
owns only visual options and a stream handle; CVD, delta, volume, and bubble dependents own no
provider tape.
A bar owns sorted price levels;
there is no renderer-side cluster cache. Tip append mutates only the active bar or appends one bar,
projects only that changed suffix into the footprint and any bound candles/bars (also under a
retention ceiling), and invalidates those series. CVD/delta/volume studies recompute only that suffix
(CVD from a cached running fold), and bubble markers fold only the appended trades (no work without bubble
dependents): they size only new or merged bubbles and rescale every retained marker only when the
peak bubble volume changes. `trade_stream_stats` reports each as a work counter, and every tip
result equals a clean rebuild of that dependent. The tip that crosses a retention ceiling (once per
hysteresis margin, 1/32 of the cap) evicts the leading bars, exactly the trades they aggregated, and
the bubbles made only of those trades in place: it reconstructs nothing and never scans the
retained tape, so its work is proportional to the evicted trades plus the retained rows (renumbering
the retained bars and the data layer's own trim of the affected rows).
Closed bars are immutable on the
live path. Historical insertion/correction reconstructs canonical state once after the final tape is
known and replaces the projection once. The current reconstruction is intentionally full-series;
work statistics expose that cost so suffix checkpoints can be added when measurements justify them.

Vectors, the tape deque, and the trade-ID index reuse their allocated capacity. Series retention
evicts complete old bars and the trades they aggregated together (counted per bar, so a trade sharing
its microsecond with the next bar's open stays with its own bar, and a session-anchored bar's folded
opening-auction print, stamped before the bar's open, leaves with that bar; prints the session policy
excludes join no bar, so eviction steps over them and takes those stamped before the first retained
bar's first print) while preserving the
classification/session/cumulative-delta seed needed by the remaining tape; no orphan tape or derived
history survives. The ID index keeps absolute tape positions, so eviction removes only the evicted
IDs, and replay checkpoints inside the retained suffix are re-addressed rather than rebuilt. Every
footprint bound to the stream drops the same rows, since footprint geometry reads stream bar `i` for
row `i`. Bubble folds materialize markers only for retained bubbles, so a refold over a long tape
holds at most `max_markers` markers. Engine memory telemetry includes tape, levels, the ID index,
bubble folds, and retained capacities.

Device loss is irrelevant to this model: the headless tape and derived bars remain intact while the
browser executor falls back. Renderer caches are rebuilt from the same frame contract.

## 6. API surface

The Rust engine owns typed commands to create/configure a footprint series, atomically replace a
trade tape, append/correct one trade, ingest an ordered batch, query a bar and price level, and read
work/memory statistics. The WebAssembly boundary accepts typed columns for historical and live batch
ingest; object conversion is reserved for the low-frequency single-event API.

The TypeScript package exposes a dedicated `footprint_series_api` from
`chart.add_series("footprint", options)`. Its input is `footprint_trade`, not `series_data`; generic
OHLC `set_data` is rejected for this kind. Queries expose bar OHLC, price levels, POC, final/Max/Min
Delta, delta percentage, bid/ask/unknown/total volume, session delta, and stacked flags. Chart-level
methods create CVD, delta, and bounded bubble dependents and query stream revision/telemetry. Data-change notifications use
`full` for replacement/historical reconstruction and `update` for a true tip update.

Options cover tick size, ticks per row, time-bar interval/anchor or trade-count/volume/range construction, imbalance
ratio/minimum/consecutive count, visual cell modes, colors, text size, summaries, and generic series
retention. Changing tick size, ticks per row, or time aggregation rebuilds from the tape atomically, keeping
prints the replay clock hides, the retention seed, and session anchoring; bound candles/bars follow. Visual-only
options invalidate only the series frame layer.

Host callbacks receive derived snapshots only through ordinary chart query/event paths. Aeris Terminal
and other hosts remain authoritative for feed subscription, exchange calendars, and choosing
session IDs; none of those concerns enter the renderer.

## 7. Verification and performance evidence

### Reference fixture and release baseline

The dense order-flow reference view is deterministic: 20 one-minute bars, 11 price levels per
bar, a 0.25 tick size, and paired buy/sell prints at every level. It is used by the GPUI
`plan_bench` fixture at 1600×900 CSS pixels, DPR 1.5, and 72 px bar spacing, where detailed LOD
must emit the cell text runs. The WebGPU `perf_gate` Target J consumes the same frame contract
with a resolved atlas quad for every text primitive and guards a 2 ms p99 CPU-side encoding
budget; this measures scheduling and upload preparation, not device present time.

The sustained native release baseline remains Target D: 2,500 retained one-minute bars with 100
trades per bar, a 100-bar live batch, and a 10-bar correction batch. Its budgets are 300 ms for
historical load, 50 ms for the live batch, 300 ms for correction, and 16.67 ms for frame
construction, with retention bounded to the configured history. It then streams 9,000 single-trade
live tips (crossing the retention ceiling once) into the chart with bound candles, CVD, delta, and
bubbles, requires every tip's work counters to stay within the changed bar suffix and the new
trade with no tape reconstruction, budgets the tip p99 at 0.25 ms, and budgets the slowest tip (the
one crossing the ceiling) at one 16.67 ms frame. Commands and thresholds are kept in the release
examples so a clean `--release` run can be compared without importing machine-specific timings into
the repository; `perf_gate` prints the measured tip p99 and slowest tip against these budgets.

The finite GPUI real-window probe was also exercised on the current Windows display with the
footprint fixture: 30 frames at DPR 1.25 and 500 source bars produced 24 cached text runs (zero
misses), with adapter p50/p99 of 2.005/4.508 ms and GPUI paint p50/p95/p99 of 2.005/2.369/4.341 ms.
These numbers are an observed host run, not a portable release budget; the probe does not expose
native WebGPU device-present timing.

The 2026-09-26 release gate measured the same dense fixture through the native WebGPU CPU-side
encoding path: 120 resolved text primitives produced 120 atlas instances, with a 0.00 ms p99
encoding sample against the 2.00 ms Target J budget. The Chromium footprint suite also passed all
six cases, including the WebGPU shared-frame path. These checks cover executor scheduling and
browser integration; native device-present timing and the GPUI display-specific numbers above
remain diagnostic rather than portable budgets.

Order-flow milestone evidence was captured on 2026-09-26. The GPUI pane capture used
`AERIS_CHARTS_GPUI_FEATURE=footprint` at DPR 1.25 and produced a 1543×873 image for the dense
12-bar fixture; the image was visually inspected for readable cell text, stable column alignment,
and unclipped pane content. The browser accessibility review passed the focused
`unified-interaction-accessibility.spec.mjs` contract: one bounded application surface, hidden
canvas pixels, a silent live region during streaming, and keyboard drawing edits that roll back.
The capture image remains a transient milestone artifact; the command and metadata are recorded
here so the evidence can be reproduced without adding binary fixtures to the repository.

Deterministic synthetic tapes cover grid boundaries, unknown-side handling, quote/tick-rule
classification, equal timestamps and sequences, late events, corrections, session resets,
session-anchored time bars through live tips and retention, all bar modes, bid/ask/total/delta levels, POC ties, mean-reverting Max/Min Delta paths, both imbalance sides,
and stacked-run breaks. Frame tests cover every shipped LOD and primitive ordering. Browser tests
exercise the same engine-built frame through Canvas2D and WebGPU as well as typed ingest and public
queries; GPUI and native consume those existing backend-neutral primitive kinds without footprint
math or footprint-specific executor branches.

The release evidence fixture measures a large historical tape plus sustained live append. It records
aggregation/rebuild work, frame CPU, draw count, upload bytes, retained memory, and allocations.
Acceptance requires tip updates not to rebuild prior bars, stable memory under configured retention,
and bounded visible-frame work as history grows.
