# Aeris Charts evidence benchmarks

This subsystem is the source of truth for Aeris Charts performance, artifact-size, and memory claims. It measures the production `@aeristerminal/aeris-charts` package through its public browser API and keeps every number tied to source, environment, scenario, dataset, and raw samples. It does not optimize the product and it does not manufacture unsupported values.

## Requirements

- The repository's configured Rust toolchain and `wasm-pack` 0.15.0 for the production WASM build. Every workflow installs exactly that version (`release_gate_guard.mjs` enforces it). wasm-pack runs a `wasm-opt` found on `PATH` and otherwise downloads its own binaryen `version_117`, so a locally installed `wasm-opt` silently changes the artifact; the provenance below records which one ran.
- Node.js 18 or newer.
- Chromium installed for Playwright (`cd examples/web_demo && npx playwright install chromium`).
- For official release results, a clean checkout on a controlled runner with `AERIS_CHARTS_BENCH_ENV_CLASS=official-benchmark-runner` and a stable `AERIS_CHARTS_BENCH_ENV_ID`.

The CLI runs `npm ci` when the package or demo dependencies are absent. Browser scenarios reuse the existing demo server and Playwright dependency. No benchmark package is shipped to consumers.

## Commands

```text
node benchmarks/benchmark.mjs test
node benchmarks/benchmark.mjs smoke
node benchmarks/benchmark.mjs nightly
node benchmarks/benchmark.mjs release
node benchmarks/benchmark.mjs soak --duration-ms 600000
node benchmarks/benchmark.mjs scenario pan-candlestick-100k
node benchmarks/benchmark.mjs size
node benchmarks/benchmark.mjs rebudget <size-result.json> --tradeoff <text> [--lever <text>]... [--headroom-percent 7]
node benchmarks/benchmark.mjs native
node benchmarks/benchmark.mjs compare <baseline.json> <current.json>
node benchmarks/benchmark.mjs report <result.json>
node benchmarks/benchmark.mjs public <official-release-result.json>
node benchmarks/benchmark.mjs baseline <official-release-result.json>
node benchmarks/benchmark.mjs claims
```

`release` includes the configured 60-minute soak and refuses a dirty worktree. `--duration-ms` may shorten a local validation run, but a changed duration is part of the scenario configuration and must not be compared as if it were the canonical release run.

## Profiles and authority

- `smoke` is fast shared-CI validation. It detects broken scenarios and gross regressions but is never public evidence.
- `nightly` collects broader shared-runner trends. Its absolute timings are non-authoritative.
- `release` is the full clean-worktree suite for a controlled machine. Only this profile on an `official-benchmark-runner` can generate `benchmark-public.json` or become a baseline.
- `soak` runs the long-lived workload alone. The canonical release duration is 60 minutes; a 10-minute smoke or 30-minute engineering run is selected explicitly with `--duration-ms`.

Environment classification defaults to `local`; shared workflows set `shared-ci`. Set a stable environment ID, for example `official-win-nvidia-01`. The result captures OS, architecture, CPU, logical cores, total RAM, runtime/browser, viewport, DPR, and GPU information where applicable. Viewport and DPR are `null` for size-only and native runs; unknown GPU driver, physical-core, and refresh-rate values also remain `null`. Build provenance records the exact package or native build command, Rust, wasm-pack, Node, npm, and esbuild versions, Cargo/package lock hashes, and applicable wasm-opt arguments. Hostname, username, home directory, IP addresses, environment variables, and local repository paths are never serialized.

## Deterministic data and scenarios

`shared.mjs` uses a versioned xorshift32 generator. A generator version, unsigned seed, start time, interval, start price, volatility, point count, series count, and pane count identify the dataset. Generated rows always satisfy `high >= open/close`, `low <= open/close`, and `high >= low`. Core sizes are 1K, 10K, 100K, 500K, and 1M candlesticks. Dataset creation occurs before timed installs.

Canonical scenario definitions live in `scenarios.json`; this is intentionally a small manifest rather than a benchmark DSL. Scenario IDs and versions make methodology changes explicit. A material change creates a new scenario version instead of silently rewriting history. Workloads cover startup, historical loading, current-candle and append streaming, pan/zoom/crosshair input, lifecycle retention, multi-chart, multi-series/multi-pane scaling, a five-series/100K-row Phase 2 general dashboard, retained current-candle updates across 1/2/4/8/16 series and one/four panes, and soak stability. Retained scenarios record semantic rebuild counts plus WebGPU allocation, write, and upload volume alongside frame CPU percentiles. Native evidence additionally measures every representative indicator on 10K/100K/1M histories, typed-equivalent batches of 1/10/100/1K/10K rows, 1/4/8/16 mixed indicators on one source, and one active source across 1/2/4/8/16 source/indicator-pane pairs.

## Timing and statistics

Short durations use `performance.now()` in the browser and Node's monotonic timing facilities outside it. Wall time is metadata only. Cold startup uses a fresh browser context per sample. Steady-state scenarios declare warm-up and measured runs independently.

Raw samples are retained. `shared.mjs` is the sole statistical implementation: min, max, arithmetic mean, p50, p90, p95, p99, population standard deviation, and count. Percentiles use nearest rank: sort ascending and select `ceil(p * n)`, clamped to the first element. No outliers are removed.

`set_data_api_ms` measures the public synchronous typed-data call, which includes validation, engine install, frame construction, command encoding/submission, and the backend present call. `first_raf_after_*` ends at the next browser animation-frame callback and is labeled as a compositor opportunity, not photon-visible proof. The system records actual presentation counters exposed by the engine but reports visible presentation latency as unsupported because the browser supplies no reliable completion timestamp here.

Interaction traces use Playwright pointer and wheel input on the package's top overlay canvas. `raf_frame_interval_ms` includes display cadence and browser scheduling; `frame_cpu_ms` is the existing engine-to-command-encoding measurement. Effective FPS is derived from observed rAF callbacks and is always presented with frame durations and refresh-rate limitations.

## GPU and CPU methodology

On WebGPU, `gpu_render_pass_ms` is accepted only when the existing `frame_stats().gpu_ms` produces a resolved hardware timestamp-query sample. The value spans the WebGPU render pass and is asynchronously read back. On Canvas2D or an adapter without `timestamp-query`, it is `unsupported`, never zero and never replaced with CPU submission time.

Browser CPU is the Chromium DevTools `Performance.TaskDuration` delta across a scenario. It is a page main-thread task measurement, not whole-system CPU and not solely attributable to Aeris Charts. Frame CPU comes from the existing bounded WASM telemetry record.

## Memory and lifecycle methodology

Memory labels stay distinct:

- `wasm_linear_memory_bytes` is reserved WASM linear memory. It is global to the module, grows in 64 KiB pages, does not shrink, and is not total RAM.
- `browser_js_heap_used_bytes` is Chromium's whole-page JavaScript heap after a documented DevTools GC.
- `browser_page_memory_*` uses `measureUserAgentSpecificMemory` when available and is whole-page memory, not exact Aeris ownership.

Lifecycle scenarios first complete and discard one create/load/render/destroy warm-up so WASM initialization and initial allocator growth precede the baseline. They then repeat the same deterministic fixture, recording WASM linear memory on every loaded cycle. Because whole-page memory collection is disruptive, smoke takes one loaded sample while 50- and 100-cycle profiles take five and ten evenly spaced loaded samples; initial and final samples bracket every run. Results record whether GC was forced. Retained delta is final whole-page memory minus the warmed initial baseline; retained delta per cycle divides that value by completed cycles. Unsupported page-memory APIs remain explicit. Multi-chart and multi-series scaling load every configuration in a fresh page realm so non-shrinking WASM linear-memory high-water marks remain comparable; fixture creation and disruptive page-memory collection stay outside startup timing.

Soak sampling performs continuous current-candle updates, periodic appends, time-scale movement, crosshair movement, and periodic sampling without per-frame file writes. Lightweight frame, update-latency, and WASM-memory samples are taken every second. The more disruptive whole-page memory API is sampled once per minute so it does not dominate the workload. The result reports first-to-last memory growth normalized per hour and second-half versus first-half frame CPU degradation when enough observations exist; otherwise that derived metric is explicitly unsupported. Raw periodic samples remain in the result.

## Artifact sizes

The size scenario runs the same production `npm run build` used before publication and reads `npm pack --json --dry-run`. Metrics are unambiguous:

- npm tarball and unpacked bytes for exactly the files npm would publish;
- production JavaScript raw/gzip-9/Brotli bytes;
- production optimized WASM raw/gzip-9/Brotli bytes;
- total TypeScript declaration bytes;
- minimal, typical, and full minified consumer JavaScript bundles built by the package's existing esbuild dependency.

Consumer JavaScript bundle metrics explicitly exclude the separately shipped WASM asset, whose sizes are reported independently.

A size result is only meaningful if the optimized module was measured, so the step fails when the build log shows that wasm-opt did not run (wasm-pack prints `Skipping wasm-opt` on a platform it cannot fetch binaryen for). Every result records `build.cargo_profile`, `build.wasm_opt_args` (read from the crate metadata wasm-pack itself reads, not a copy) and `build.wasm_opt_version` (the `wasm-opt` on `PATH`, else the newest one in the wasm-pack cache, else `null`) next to the rustc and wasm-pack versions. `build.profile` stays `release`, the evidence channel, and `compare` treats a result with different build metadata as incompatible.

## Results, baselines, budgets, and reports

The versioned JSON contract is `schema/result-v1.schema.json`. Runtime validation rejects missing metadata, failed scenarios disguised as success, non-finite samples, and impossible negative durations. Scenario failures carry `status: failed` and an error; public summaries exclude them.

Local raw results are immutable files under `benchmarks/results/v<version>/<environment-id>/` and are gitignored. CI uploads them as artifacts. Release results should be attached immutably to the matching release. `baseline` copies only a clean official release result into `benchmarks/baselines/v<version>/<environment-id>.json` and refuses overwrite. The selected policy is an explicit previous-release baseline; it never rolls silently.

Comparison requires the same scenario version, generator version, seed, dataset configuration, point/series/pane counts, stable environment ID, OS, architecture, CPU, runtime/browser version, GPU identity, viewport, DPR, and refresh-rate metadata. Output includes baseline, current, absolute difference, percentage difference, direction, scenario/environment compatibility, and status. Budgets live only in `budgets.json`. Relative timing thresholds remain empty until controlled baseline evidence exists; adding one requires both warning and failure percentages keyed as `<scenario>.<metric>.p50`.

Blocking `absolute_maximums` use the same key convention. Deterministic artifact ceilings do not require a
machine baseline; the machine-sensitive general-dashboard startup/upload ceilings are evaluated only on the
official release benchmark runner, whose stable environment identity is part of the release evidence. Every
benchmark run containing a named scenario evaluates its configured maxima; an exceeded, unavailable, or failed
metric exits non-zero. The initial package ceilings were set
from a clean production build at commit `813230b` and rounded above its measured output:

| Metric | Observed bytes | Blocking maximum |
| --- | ---: | ---: |
| npm tarball | 960,989 | 1,050,000 |
| npm unpacked | 2,811,610 | 3,000,000 |
| JavaScript raw | 577,227 | 620,000 |
| JavaScript Brotli | 87,948 | 95,000 |
| WASM raw | 1,975,672 | 2,100,000 |
| WASM Brotli | 583,569 | 625,000 |

Budget policy v3 records the deliberate Phase 2 package-size reset after the complete Cartesian API landed.
Before changing ceilings, the published ESM build was switched to minification; that reduced JavaScript raw
from 697,316 bytes to 343,161 and Brotli from 96,233 bytes to 64,038, so both original JavaScript ceilings remain
unchanged. The irreducible optimized WASM and package-container growth is captured with modest release headroom:

| Phase 2 metric | Observed bytes | Blocking maximum |
| --- | ---: | ---: |
| npm tarball | 1,197,880 | 1,300,000 |
| npm unpacked | 3,435,987 | 3,700,000 |
| JavaScript raw | 343,161 | 620,000 |
| JavaScript Brotli | 64,038 | 95,000 |
| WASM raw | 2,812,727 | 3,000,000 |
| WASM Brotli | 761,514 | 810,000 |

This reset is tied to the Phase 2 engine-owned Cartesian surface (additional data channels, reference/brush/
shared-tooltip APIs, heatmap variants, persistence, and WASM bindings). Future growth is again blocked at the v3
ceilings rather than inheriting an open-ended exception.

### WASM size levers and re-baselining

Once B1-B8 landed, the optimized WASM no longer fit the v3 ceilings. Before any ceiling moves, the bytes were measured and the lossless levers were priced. All numbers below come from one machine (rustc 1.98.1, wasm-pack 0.15.0, wasm-bindgen 0.2.127, `wasm-opt` version_117 from the `binaryen@117.0.0` npm package on `PATH`, Chromium 141 headless, Canvas2D forced, a shared four-core sandbox) at base commit `78d7d59`. The build route was also run by hand (`cargo build -p aeris_charts_wasm --release --target wasm32-unknown-unknown`, `wasm-bindgen --target web --out-name aeris_charts_wasm`, `wasm-opt` with the crate-metadata flags) and produced a module byte-identical to `wasm-pack build`. They are engineering evidence, not CI-runner or public performance claims.

| Package metric | `78d7d59` | With `IndicatorKind` serde out of line | v3 ceiling |
| --- | ---: | ---: | ---: |
| npm tarball | 1,876,748 | 1,860,741 | 1,300,000 |
| npm unpacked | 5,636,275 | 5,503,213 | 3,700,000 |
| JavaScript raw | 412,622 | 412,622 | 620,000 |
| JavaScript Brotli | 73,630 | 73,635 | 95,000 |
| WASM raw | 4,739,804 | 4,606,742 | 3,000,000 |
| WASM gzip-9 | 1,663,479 | 1,647,743 | - |
| WASM Brotli | 1,175,083 | 1,172,054 | 810,000 |

(Both tarball and unpacked columns already exclude `dist/react.js.map`, which the same change stops shipping: 8,165 tarball and 27,149 unpacked bytes. Ceilings are not changed by measuring: derive them with `rebudget` below on the final code.)

The module is 93% code (4.40 MB), 6% data (0.28 MB, of which the crosshair mask is 100,368 bytes) and under 1% imports and exports. The code is a long tail, not one hotspot. Twiggy on a names-preserving `-Oz -g` build of the same module attributes it (percent of code) to `aeris_charts_engine` 31%, the wasm crate 11% plus its wasm-bindgen export shims, libcore and alloc generics instantiated for those crates 25% (`slice::sort` alone 6%, across about fifteen element types), serde-derived deserialization about 25% across the engine and wasm crates (`serde_json` itself 3%), `aeris_charts_core` 3%, indicators 2%, render, render_wgpu and wgpu about 1% each. The largest function, the per-frame `ChartInner::render_inner`, is 6.6% of code; the ten largest are 20%. Dependencies were already at `opt-level = "z"`.

| Lever (workspace crates not listed stay at `opt-level` 3) | WASM raw | Brotli | raw vs current | pan / stream frame CPU p50, paired ratio vs current |
| --- | ---: | ---: | ---: | --- |
| current | 4,739,804 | 1,175,083 | - | 1.00 / 1.00 |
| `IndicatorKind` serde out of line (shipped) | 4,606,742 | 1,172,054 | -2.8% | 1.00 / 1.06 (pan 3 of 7 rounds slower, stream 6 of 7: the stream figure is this machine's noise floor on a change that is off the frame path) |
| `wasm-opt` binaryen 132 instead of 117 | 4,717,722 | 1,174,150 | -0.5% | not timed |
| `wasm-opt --converge` | 4,738,993 | 1,173,017 | -0.02% | not timed |
| without `+simd128` | 4,803,949 | 1,179,862 | +1.4% | not timed (SIMD costs no bytes) |
| engine `s` | 4,268,389 | 1,115,992 | -10.0% | 0.99-1.03 / 1.01-1.06 |
| engine `z` | 3,895,650 | 1,057,463 | -17.8% | 1.06-1.08 / 1.00-1.07; zoom 1.18, crosshair 1.11 |
| wasm crate `z` | 4,515,650 | 1,172,435 | -4.7% | 0.99 / 1.17 |
| `aeris_charts_core` `z` | 4,656,252 | 1,170,339 | -1.8% | not timed alone |
| indicators `z` | 4,707,715 | 1,171,271 | -0.7% | not timed alone |
| render and render_wgpu `z` | 4,718,680 | 1,172,342 | -0.4% | not timed alone |
| engine + wasm crate `z` | 3,464,533 | 987,566 | -26.9% | 1.09 / 1.18 |
| engine + wasm + core + indicators `z` | 3,292,048 | 944,719 | -30.5% | 1.42 / 1.19 |
| every workspace crate `s` | 3,741,299 | 1,025,898 | -21.1% | one screening round: 1.09 / 1.14 |
| every workspace crate `z` | 3,250,690 | 933,355 | -31.4% | 1.49 / 1.30 |

Only the first row is shipped: it is the one lever whose effect is pure code volume (the roughly 110 KB internally tagged `IndicatorKind` deserializer existed twice, once for `from_value` and once for struct fields, and Brotli already hid most of the duplicate), on a cold path, with byte-identical rendered frames. Every `opt-level` row trades frame time for bytes and stays unshipped pending a product decision.

Timing rows are medians of per-round paired ratios from interleaved A/B runs (at least seven alternating runs per variant, a fresh browser per run, `pan-candlestick-100k`, `stream-current-candle-60hz`, `zoom-candlestick-100k`, `crosshair-candlestick-100k`); the range is across independent sets. The noise floor on this machine is large: the current build's own run-to-run p50 spread is about 15-22% for pan and 16-54% for stream, and the 100k dashboard p50 (five samples per run) varies by about 30%, so no dashboard difference in any row is distinguishable. Fifteen interleaved pan and stream rounds of engine `s` gave p50 ratios 1.03 (pan, 10 of 15 rounds slower) and 1.01 (stream, 9 of 15), pooled p95 +5% and -3%: a cost of up to a few percent on pan cannot be excluded, which is why the release profile was not changed. Rendered frames of the current and engine-`s` builds were byte-identical for five deterministic Canvas2D demo captures. Adding `aeris_charts_core` and indicators at `z` to the engine and wasm-crate build raised pan from 1.09 to 1.42, so the per-frame loops live mainly in `aeris_charts_core`; the wasm crate at `z` showed +17% on stream with a wide spread (0.76-1.37 per round).

Reproduce a variant without editing the release profile: build with `--config 'profile.release.package.<crate>.opt-level="z"'` (Cargo environment variables cannot express per-package overrides), run `wasm-bindgen` and `wasm-opt` as above, and rebundle `dist/index.js` from that variant's `aeris_charts_wasm.js` (the glue names closure shims by per-build index, so a glue file from another build does not match).

To re-baseline after a deliberate size increase, run `node benchmarks/benchmark.mjs size` on the final code (it writes the result JSON under `benchmarks/results/` and exits non-zero while a ceiling is exceeded), then

```text
node benchmarks/benchmark.mjs rebudget <that result.json> --tradeoff "<the product capability that added the bytes>" --lever "<lever applied and its measured effect>"
```

It prints the proposed `budgets.json` without writing it: only ceilings the run exceeds move, to the observed p50 plus 7% rounded up to 10,000 bytes (the Phase 2 reset carried 6.4-8.5%), `policy_version` increments, and a `rationale` entry is appended holding the commit, observed bytes, raised ceilings, toolchain (rustc, wasm-pack, wasm-opt version, Cargo profile, wasm-opt flags), levers and tradeoff. `budgets.json` ignores keys it does not evaluate, so the evidence stays beside the numbers. Review the diff, commit it with the code that caused the growth, and confirm `ci.yml`, `metrics-smoke.yml` and the nightly and release workflows pass against it.

Phase 2 adds release-blocking maxima for `general-dashboard-100k`: p50 startup through the first following rAF
must stay at or below 2,000 ms, and first-frame WebGPU vertex uploads must stay at or below 96 MiB. These are
guardrails for catastrophic host regressions, not cross-machine performance claims.

These byte counts are reproducible filesystem/compression evidence, not an official wall-clock
benchmark or a public performance claim. A deliberate size increase must explain the product
tradeoff and update the central ceiling; it must not bypass the evaluator.

`report` generates a human-readable Markdown artifact beside the raw result. `public` generates a stable `benchmark-public.json` containing only measured `public_candidate` metrics from a clean official release. The trace is: website field → public summary → immutable raw result → raw samples → versioned scenario → deterministic dataset/environment → commit.

## CI and limitations

PR smoke runs harness tests, production artifact sizes, and a short browser workload on shared CI. Nightly runs broader non-authoritative scenarios. The release workflow targets a future self-hosted Windows runner labeled `benchmark` and produces raw JSON, a readable report, and the public summary. Credentials and runner provisioning are deliberately outside this repository.

The `native` command writes a separate result for headless Rust ingestion, retained frame construction, and current-candle replacement. Native diagnostics are never mixed with browser product results or public summaries.

Competitor comparisons are deliberately not implemented here. A future comparison must use a separate result namespace, pin every library and browser version, disclose backend and feature configuration, use equivalent visible chart features and datasets, apply the same warm-up/statistics/environment rules, and publish methodology beside the numbers. Existing demo dependencies are not treated as benchmark competitors.

Not currently measurable with trustworthy semantics: photon-visible presentation completion, a separate first-`queue.submit` timestamp, GPU upload-only duration, GPU resource byte attribution, browser GPU driver version, physical core count, and native process RSS. They remain unsupported or `null`; no substitute number is inferred. Existing interaction and GPUI scene-plan examples remain useful internal diagnostics.
