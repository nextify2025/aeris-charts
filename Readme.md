# Aeris Charts

Aeris Charts is the Rust chart engine for [Aeris Terminal](https://aeristerminal.com) and browser
hosts. One deterministic chart model powers WebGPU, Canvas2D, GPUI, and native rendering.

The project includes professional chart interactions, drawings, technical indicators, multiple panes and scales, custom series, primitives, shared-memory market-data input, and backend parity tooling.

## Rust crates

Rust hosts depend on the `aeris_charts_*` crates from this repository through Git or path
dependencies; they are not published to crates.io. `aeris_charts_engine` owns chart state,
interactions, drawings, indicators, and frame construction, and hosts pair it with a renderer such
as `aeris_charts_render_wgpu` or `aeris_charts_native` (see [Repository layout](#repository-layout)).

## Browser package

The browser SDK is published through GitHub Packages as `@aeristerminal/aeris-charts`. Configure
the Aeris Terminal scope before installing:

```text
@aeristerminal:registry=https://npm.pkg.github.com
```

```sh
npm install @aeristerminal/aeris-charts
```

Version tags publish automatically after release credentials are configured and the tag matches
`packages/charts/package.json` exactly
(for example, package version `0.9.0` is released from tag `v0.9.0`).

Create a chart with the asynchronous camel-case API:

```ts
import { createChart } from "@aeristerminal/aeris-charts";

const container = document.querySelector<HTMLElement>("#chart");
if (!container) throw new Error("missing chart container");

const chart = await createChart(container, { autoSize: true });
const candles = chart.addSeries("candlestick");

candles.setData([
  { time: 1735689600, open: 100, high: 108, low: 98, close: 105 },
  { time: 1735776000, open: 105, high: 112, low: 103, close: 110 },
]);

chart.timeScale().fitContent();
```

The original snake-case names remain available on the same chart and series handles. Options and
data fields retain their documented names; the camel-case aliases apply to the common method calls.

Financial and general series can share one chart lifecycle while occupying panes with compatible
coordinate domains:

```ts
const summary = chart.addPane({
  preserve_empty: true,
  horizontal_domain: { type: "category", scale: "band" },
});
const pane = summary.paneIndex();
chart.addAxis({ id: "month", pane, dimension: "x", scale: "band" });
chart.addAxis({ id: "revenue", pane, dimension: "y", scale: "linear" });
const revenue = chart.addSeries("column", {
  pane, x_axis_id: "month", y_axis_id: "revenue", title: "Revenue",
});
revenue.setData([{ id: "jan", x: "Jan", y: 42 }, { id: "feb", x: "Feb", y: 57 }]);

// Both panes render through the same engine and chart.render() lifecycle.
chart.render();
```

The same engine is available as an optional React authoring layer. Install React in applications that
use it, then import the adapter from the package subpath; framework-neutral applications do not load
or depend on React:

```sh
npm install @aeristerminal/aeris-charts react
```

```tsx
import { FinancialSeries, GeneralPane, AerisChart } from "@aeristerminal/aeris-charts/react";

const axes = [
  { id: "month", dimension: "x", scale: "band" },
  { id: "revenue", dimension: "y", scale: "linear" },
] as const;

export function Dashboard({ candles, revenue }) {
  return (
    <AerisChart options={{ autoSize: true }} style={{ width: "100%", height: 560 }}>
      <FinancialSeries kind="candlestick" data={candles} />
      <GeneralPane
        options={{ horizontal_domain: { type: "category", scale: "band" } }}
        axes={axes}
        series={[{
          key: "revenue",
          kind: "column",
          options: { x_axis_id: "month", y_axis_id: "revenue", title: "Revenue" },
          data: revenue,
        }]}
      />
    </AerisChart>
  );
}
```

The adapter creates the ordinary imperative chart once, reconciles data/configuration onto retained
engine handles, and calls the same `chart.remove()` lifecycle on unmount. Its module is safe to import
during SSR because chart creation and DOM access begin only after the component mounts. Complete
framework-neutral and React combined examples live in `examples/all_in_one/`.

The optimized WASM binary is shipped beside the ESM entry and resolves there automatically. Bundlers
that require an explicit asset URL may import `@aeristerminal/aeris-charts/wasm` (or their normal URL-loader
form of that export) and pass the resulting URL to `initWasm()` before creating a chart; no `pkg/`,
`crates/`, demo, or repository path is part of the consumer contract.

Numeric times are finite whole UTC seconds in the exact inclusive range
`-62167219200..253402300799` (years 0000..9999). Aeris never auto-converts numeric timestamps;
rejections include a likely milliseconds, microseconds, or nanoseconds hint when applicable.
Direct set/update batches reject atomically on any invalid timestamp, and invalid single updates
leave existing data unchanged. Inspect `series.last_ingestion_diagnostics()` for the reason.

## Advanced chart features

The advanced financial series are first-class Rust-engine series. Their data, autoscale projection,
geometry, lifecycle, and rendering are shared by every backend; the browser package only translates
public data and options at the WASM boundary:

```ts
import { create_volume_profile } from "@aeristerminal/aeris-charts";

const heatmap = chart.add_series("heatmap", {
  cell_border_width: 1,
  cell_border_color: "rgba(255,255,255,.08)",
  cell_shader: (amount) => `rgba(80,0,255,${Math.min(1, amount / 100)})`,
});
heatmap.set_data(heatmap_data);

const profile = create_volume_profile(candles, {
  time: 1735689600,
  profile: [{ price: 100, vol: 12 }, { price: 101, vol: 28 }],
  width: 10, // time-scale bar slots
});
// profile.set_data(next_time_anchored_profile); profile.detach();
```

The engine-owned feature set includes brushable area, grouped bars, heatmap, HLC area,
pretty histogram, shaded background, stacked area,
stacked bars, and box-and-whisker series. Primitive helpers include accessibility, anchored text,
official ±10% price bands, delta and ordinary tooltips, highlighted-bar
crosshair, image watermark, overlay price scale, partial price line, rectangle/trend/vertical
drawings, session highlighting, volume profile, and user-defined price lines.

Heatmap-around-line and shaded-background examples are composed beneath a normal line series.

Features that Aeris already owns—drawings (including Long Position and Short Position tools, the price range, date range, and date-and-price range measuring tools, and the Shift-click quick measure), bands, price lines, overlay scales, partial-last-price
lines, session shading, highlighted bar slots, and time-anchored volume profiles—are thin helpers
over those engine APIs. Accessibility is enabled by default; `chart.accessibility()` returns its
singleton controller and `enable_accessibility(chart, options)` configures the same instance for
compatibility. Keyboard/ARIA nodes and announcements remain browser DOM chrome, while bounded data
queries, focus geometry, drawing edits, and rendering primitives use the shared engine. Streaming
market updates are silent unless `announce_data_updates` is enabled. Every returned feature handle
with `detach()` releases its engine and host state.

Browser input uses Pointer Events for mouse and pen, plus cancellable Touch Events for dynamic
page-scroll arbitration. The engine owns the bounded gesture state, 5 px drag threshold,
fixed-start-centroid cumulative pinch behavior, primary-touch continuation, cancellation, and
device-aware hit tolerances. Wheel policy is configurable with
`wheel_behavior: "auto" | "pan" | "zoom"`; informed by measured behavior from the pinned public
reference fixture, auto zooms time from
vertical deltas and pans time from horizontal deltas independently on the pane or either axis, with
no Ctrl/Shift special case. The explicit `pan` and `zoom` values retain Aeris extension routing.

## Trading and order management

Trading objects are a separate first-party engine domain. The application supplies authoritative
positions, working orders, bracket/OCO relationships, executions, and instrument metadata; Aeris
owns their deterministic visualization, native axis labels, hit testing, risk/reward regions, and
local interaction previews. A drag never rewrites confirmed broker state. Instant mode emits one
typed, broker-neutral intent on release; manual mode holds the preview behind inline Confirm and
Discard controls. The host reconciles a confirmed preview with an accepted state update or rejects
it explicitly. Risk/reward fills belong only to active previews, never confirmed orders.

```ts
const trading = chart.trading();
trading.set_confirmation_mode("manual"); // Optional; the default is "instant".
trading.apply_snapshot({
  instrument: { tick_size: 0.25, price_precision: 2, point_value: 50, currency: "USD" },
  positions: [{ id: "position-1", side: "long", average_price: 5230, quantity: 2 }],
  orders: [{
    id: "target-1", side: "sell", kind: "limit", role: "take_profit", status: "working",
    price: 5240, quantity: 2, position_id: "position-1", oco_group_id: "bracket-1", revision: 4,
  }],
});

trading.subscribe_intents(async (intent) => {
  const accepted = await route_to_broker(intent);
  trading.resolve_intent(intent.sequence, accepted);
  // On acceptance, push the resulting authoritative order/position update through this API.
});

// Convert a completed Long/Short Position drawing into one atomic bracket request. Quantity is
// deliberately host-owned; the intent carries the drawing's tick-snapped entry, TP, and SL.
const plan = chart.selected_drawing();
if (plan && (plan.kind() === "long_position" || plan.kind() === "short_position")) {
  trading.place_bracket_order(plan.id, quantity_from_host);
}
```

Live trading objects, previews, and intent queues are chart-local runtime state and are deliberately
excluded from `chart.export_state()`.

Give the container an explicit size; the chart canvases fill it.

Import the portable design system once in browser hosts:

```ts
import "@aeristerminal/aeris-charts/design.css";
```

Light is the CSS default. Set `data-theme="dark"` (or class `dark`) on a root element for dark
mode, and apply `theme_options("dark")` to the chart. Host chrome and chart labels default to the
system UI font stack. Chart font remains an explicit layout option so a host webfont cannot shift
financial labels until the host sets it.
Chart defaults use the same semantic roles directly: foreground for axes and value text, and muted
for crosshair-label surfaces. Crosshair lines use the theme-independent `crosshair_line` token
(`#4a4a4a`) in both themes.

## Repository layout

- `crates/aeris_charts_core` — validated data, scales, options, formatting, and shared math.
- `crates/aeris_charts_indicators` — platform-free indicator calculations.
- `crates/aeris_charts_engine` — chart state, interactions, drawings, panes, and frame construction.
- `crates/aeris_charts_render` — backend-neutral primitives and the ordered draw list.
- `crates/aeris_charts_render_wgpu` — WebGPU executor.
- `crates/aeris_charts_render_gpui` — GPUI executor.
- `crates/aeris_charts_wasm` — browser and WebAssembly boundary.
- `crates/aeris_charts_native` — deterministic native rendering and performance verification.
- `packages/charts` — TypeScript browser package.
- `examples/web_demo` — browser integration and parity test host; it is not a published package.
- `docs` — architecture, public API, domain-model, and contribution documentation.
- `plan` — active product and expansion plans.

See [Architecture.md](docs/Architecture.md) for ownership, data flow, and backend boundaries.
See [Public_api.md](docs/Public_api.md) for supported/experimental surfaces, persistence, errors, and
version policy.

## Development

Prerequisites: stable Rust, the `wasm32-unknown-unknown` target, `wasm-pack`, Bun, and Node.js 18 or newer
(the `node` scripts and Playwright run on Node).

```sh
cargo test --workspace

cd packages/charts
bun install --frozen-lockfile
bun run build
bun run lint
bun run typecheck
bun run test:pack
```

The complete verification gates are documented in [AGENTS.md](AGENTS.md) and enforced by CI.
Contribution requirements are documented in [CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Performance evidence

Reproducible release-package benchmarks live in [`benchmarks/`](benchmarks/README.md). The harness records deterministic workloads, raw samples, statistical summaries, build and machine provenance, capability limits, package sizes, browser CPU/GPU timing, memory, lifecycle, scaling, and soak behavior. Shared CI results are diagnostics; only clean runs from the controlled benchmark environment may produce public claims or release baselines.

## License

Aeris Charts is open-source software licensed under the
[GNU Affero General Public License v3.0](LICENSE), identified by the SPDX expression
`AGPL-3.0-only`. The AGPL permits commercial use, modification, and redistribution subject to its
copyleft and corresponding-source requirements, including its network-interaction provisions.

Organizations that cannot comply with the AGPL may obtain a separate Aeris Terminal Commercial License
for proprietary integration, redistribution, OEM/embedded use, white-label use, support, and custom
engineering. The commercial option is a separate agreement; it does not add restrictions to the
public AGPL grant. See [COMMERCIAL_LICENSE.md](COMMERCIAL_LICENSE.md).

## Independent development and third-party references

Aeris Charts is independently designed and implemented. Public documentation, public examples,
and observed behavior from established charting products are used to learn common user expectations
and to build development-only compatibility comparisons. Those references do not share Aeris's
engine, rendering, or state-management implementation.

The KLineChart indicator port is the exception: its formulas are translated from
[KLineChart](https://github.com/klinecharts/KLineChart) v10.0.3 (Apache-2.0) and credited in
[NOTICE](NOTICE) and the module documentation.

Development tests use Lightweight Charts as a pinned Apache-2.0 dependency through its public API.
That dependency is not included in the published `@aeristerminal/aeris-charts` package. TradingView and
Lightweight Charts are trademarks of their respective owners; Aeris Charts is not affiliated with
or endorsed by TradingView. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
