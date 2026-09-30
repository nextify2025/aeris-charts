import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";
import { benchmark_root, repository_root } from "./core.mjs";
import { dataset_metadata, metric } from "./shared.mjs";

const require_from_demo = createRequire(path.join(repository_root, "examples", "web_demo", "package.json"));
const { chromium } = require_from_demo("@playwright/test");
const seed = 0x02f6e2b1;

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function wait_for_server(url, process) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (process.exitCode !== null) throw new Error(`benchmark server exited with code ${process.exitCode}`);
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    await delay(100);
  }
  throw new Error("timed out waiting for benchmark server");
}

async function collect_cdp(session) {
  const performance_metrics = await session.send("Performance.getMetrics");
  const values = Object.fromEntries(performance_metrics.metrics.map(({ name, value }) => [name, value]));
  const heap = await session.send("Runtime.getHeapUsage");
  return { task_duration_seconds: values.TaskDuration ?? null, js_heap_used_bytes: heap.usedSize ?? null };
}

function scalar(name, value, unit, direction, visibility, methodology) {
  return { [name]: metric([value], unit, direction, visibility, methodology) };
}

function samples(name, values, unit, direction, visibility, methodology, availability = "measured") {
  return { [name]: metric(values, unit, direction, visibility, methodology, availability) };
}

function scenario_dataset(scenario) {
  const dataset = dataset_metadata(scenario.points ?? 0, seed, { series_count: scenario.series_counts?.at(-1) ?? 1, pane_count: scenario.pane_counts?.at(-1) ?? 1, interval_seconds: scenario.interval_seconds, studies: scenario.studies });
  dataset.configuration.scenario = Object.fromEntries(Object.entries(scenario).filter(([key]) => !["id", "description", "profiles", "visibility", "warmup_runs", "measured_runs", "points", "version", "kind", "interval_seconds", "studies"].includes(key)));
  return dataset;
}

async function page_environment(page) {
  return page.evaluate(() => globalThis.__Aeris_bench.environment());
}

function report_browser_errors(page) {
  page.on("console", (message) => {
    if (message.type() === "error") console.error(`browser console: ${message.text()}`);
  });
}

async function cold_start(browser, base_url, scenario) {
  const rows = [];
  for (let run = 0; run < scenario.measured_runs; run += 1) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });
    const page = await context.newPage();
    report_browser_errors(page);
    await page.goto(`${base_url}/benchmark.html`);
    await page.waitForFunction(() => globalThis.__Aeris_bench_ready === true);
    rows.push(await page.evaluate(({ points, seed_value }) => globalThis.__Aeris_bench.startup(points, seed_value), { points: scenario.points, seed_value: seed }));
    rows.at(-1).browser_environment = await page_environment(page);
    await context.close();
  }
  const values = (key) => rows.map((row) => row[key]);
  return {
    environment: rows.at(-1).browser_environment,
    metrics: {
      ...samples("module_import_ms", values("module_import_ms"), "ms", "lower_is_better", "public_candidate", "Cold dynamic import of the shipped production ESM bundle in a fresh browser context."),
      ...samples("wasm_init_ms", values("wasm_init_ms"), "ms", "lower_is_better", "public_candidate", "Cold public init_wasm() completion in a fresh browser context; includes fetch, compile, and instantiation as performed by wasm-bindgen."),
      ...samples("chart_create_api_ms", values("chart_create_api_ms"), "ms", "lower_is_better", "public_candidate", "Public create_chart() completion after WASM initialization; includes backend acquisition, chart construction, and its documented first drawn frame."),
      ...samples("series_create_api_ms", values("series_create_api_ms"), "ms", "lower_is_better", "internal", "Public add_series() completion."),
      ...(scenario.points > 0 ? samples("set_data_api_ms", values("set_data_api_ms"), "ms", "lower_is_better", "public_candidate", "Public typed historical install including the synchronous render/submit path.") : { set_data_api_ms: metric([], "ms", "lower_is_better", "public_candidate", "No data install occurs in the empty-chart startup scenario.", "not_applicable") }),
      ...samples("first_raf_after_ready_ms", values("first_raf_after_ready_ms"), "ms", "lower_is_better", "public_candidate", "Elapsed time from scenario start until the first requestAnimationFrame callback after the APIs returned; a compositor opportunity, not proof of photon-visible presentation."),
      ...samples("first_frame_cpu_ms", values("first_frame_cpu_ms"), "ms", "lower_is_better", "internal", "Existing frame_stats CPU measurement for the most recent frame at API readiness."),
      first_gpu_submission_ms: metric([], "ms", "lower_is_better", "internal", "No production hook separately timestamps queue.submit; unsupported rather than inferred from API return.", "unsupported"),
      visible_presentation_ms: metric([], "ms", "lower_is_better", "public_candidate", "The browser exposes no reliable photon/presentation-completion timestamp for this canvas path.", "unsupported"),
    },
    backend: rows.at(-1).backend,
  };
}

function degradation(values) {
  if (values.length < 4) return null;
  const midpoint = Math.floor(values.length / 2);
  const first = values.slice(0, midpoint).reduce((sum, value) => sum + value, 0) / midpoint;
  const second_values = values.slice(midpoint);
  const second = second_values.reduce((sum, value) => sum + value, 0) / second_values.length;
  return first === 0 ? null : (second - first) / Math.abs(first) * 100;
}

async function run_page_scenario(page, scenario) {
  if (scenario.kind === "historical") {
    const result = await page.evaluate((input) => globalThis.__Aeris_bench.historical(input.points, input.seed, input.warmup_runs, input.measured_runs), { ...scenario, seed });
    return { backend: result.backend, metrics: {
      ...samples("set_data_api_ms", result.api_samples, "ms", "lower_is_better", "public_candidate", "Public set_data_typed() call; includes validation, engine install, layout/frame construction, GPU command encoding/submission, and present call."),
      ...samples("first_raf_after_set_data_ms", result.ready_samples, "ms", "lower_is_better", "public_candidate", "Time from set_data_typed start to the next rAF callback; not a photon-visible timestamp."),
      ...samples("frame_cpu_ms", result.cpu_samples, "ms", "lower_is_better", "internal", "Existing frame_stats CPU cost for layout through command encoding."),
      ...scalar("wasm_linear_memory_bytes", result.stats.memory_bytes, "bytes", "lower_is_better", "public_candidate", "WASM linear memory reserved after the final install; this is not total page RAM."),
    } };
  }
  if (scenario.kind === "realtime") {
    const rows = [];
    for (let run = 0; run < scenario.warmup_runs + scenario.measured_runs; run += 1) {
      const result = await page.evaluate((input) => globalThis.__Aeris_bench.realtime(input.points, input.seed, input.mode, input.update_rate_hz, input.duration_ms), { ...scenario, seed });
      if (run >= scenario.warmup_runs) rows.push(result);
    }
  return { backend: rows.at(-1).backend, metrics: {
      ...samples("update_api_latency_ms", rows.flatMap((row) => row.api_samples), "ms", "lower_is_better", "public_candidate", `Public update() latency across ${scenario.measured_runs} deterministic ${scenario.mode}-candle runs after ${scenario.warmup_runs} discarded warm-up run(s); render requests coalesce through package rAF.`),
      ...samples("frame_cpu_ms", rows.flatMap((row) => row.frame_cpu_samples), "ms", "lower_is_better", "public_candidate", "Existing frame_stats CPU cost observed after each scheduled update frame across measured runs."),
      ...samples("series_rebuilds", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.series_rebuilds)), "count", "lower_is_better", "internal", "Retained series layers rebuilt per streaming frame."),
      ...samples("layout_rebuilds", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.layout_rebuilds)), "count", "lower_is_better", "internal", "Retained layout rebuilds per streaming frame."),
      ...samples("autoscale_runs", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.autoscale_runs)), "count", "lower_is_better", "internal", "Autoscale passes per streaming frame."),
      ...samples("grid_rebuilds", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.grid_rebuilds)), "count", "lower_is_better", "internal", "Retained grid layers rebuilt per streaming frame."),
      ...samples("gpu_buffer_allocations", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.gpu_buffer_allocations)), "count", "lower_is_better", "internal", "WebGPU vertex-buffer allocations per streaming frame."),
      ...samples("gpu_uploaded_bytes", rows.flatMap((row) => row.frame_stats_samples.map((stats) => stats.gpu_uploaded_bytes)), "bytes", "lower_is_better", "internal", "WebGPU vertex bytes uploaded per streaming frame."),
      ...samples("updates_processed_per_second", rows.map((row) => row.updates / row.elapsed_ms * 1000), "updates/s", "higher_is_better", "public_candidate", "Delivered public API calls divided by the measured interval for each measured run."),
      ...samples("presented_frames", rows.map((row) => row.presented_frames), "count", "higher_is_better", "internal", "Difference in engine presented_frames lifetime counter for each measured run."),
      ...samples("dropped_frames", rows.map((row) => row.dropped_frames), "count", "lower_is_better", "internal", "Difference in engine dropped_frames counter for each measured run."),
      ...samples("ring_overruns", rows.map((row) => row.ring_overruns), "count", "lower_is_better", "internal", "Ring overrun counter per measured run; expected zero because this scenario uses explicit public updates."),
    } };
  }
  if (scenario.kind === "lifecycle") {
    const result = await page.evaluate((input) => globalThis.__Aeris_bench.lifecycle(input.points, input.seed, input.cycles), { ...scenario, seed });
    const metrics = {
      ...samples("wasm_linear_memory_bytes", result.wasm_linear_memory_samples, "bytes", "lower_is_better", "internal", "Global WASM linear memory after each loaded chart; linear memory does not shrink and is not chart-attributable RAM."),
      ...scalar("loaded_page_memory_probe_count", result.loaded_page_memory_sample_count, "count", "informational", "internal", "Configured evenly spaced whole-page memory probes while charts are loaded; initial and final probes are additional."),
    };
    if (result.baseline.page_bytes !== null && result.final.page_bytes !== null) {
      Object.assign(metrics,
        scalar("browser_page_memory_baseline_bytes", result.baseline.page_bytes, "bytes", "informational", "public_candidate", "measureUserAgentSpecificMemory page total after documented forced GC where available."),
        scalar("browser_page_memory_peak_bytes", result.peak_page_bytes, "bytes", "lower_is_better", "public_candidate", "Peak measured browser page total across lifecycle cycles."),
        scalar("browser_page_memory_retained_delta_bytes", result.final.page_bytes - result.baseline.page_bytes, "bytes", "lower_is_better", "public_candidate", "Final minus initial whole-page memory after repeated create/load/destroy cycles and documented forced GC."),
        scalar("retained_delta_per_cycle_bytes", (result.final.page_bytes - result.baseline.page_bytes) / result.cycles, "bytes", "lower_is_better", "public_candidate", "Whole-page retained delta divided by lifecycle cycles."));
    } else {
      metrics.browser_page_memory_retained_delta_bytes = metric([], "bytes", "lower_is_better", "public_candidate", "measureUserAgentSpecificMemory is unavailable in this browser execution.", "unsupported");
    }
    return { backend: result.backend, forced_gc: result.baseline.forced_gc, metrics };
  }
  if (scenario.kind === "multi_chart") {
    const rows = [];
    let backend = null;
    for (const count of scenario.chart_counts) {
      if (rows.length > 0) {
        await page.reload();
        await page.waitForFunction(() => globalThis.__Aeris_bench_ready === true);
      }
      const result = await page.evaluate((input) => globalThis.__Aeris_bench.multi_chart(input.points, input.seed, [input.count]), { ...scenario, seed, count });
      rows.push(result.rows[0]);
      backend = result.backend;
    }
    return { backend, metrics: {
      ...samples("chart_count", rows.map((row) => row.count), "count", "informational", "internal", "Independent variable for aligned multi-chart scaling samples."),
      ...samples("startup_ms", rows.map((row) => row.startup_ms), "ms", "lower_is_better", "public_candidate", "Sequential public creation and 10K install for N charts through the first following rAF."),
      ...samples("active_repaint_ms", rows.map((row) => row.active_repaint_ms), "ms", "lower_is_better", "public_candidate", "One warmed public render request for every live chart through the following rAF."),
      ...samples("gpu_buffer_allocations", rows.map((row) => row.gpu_buffer_allocations), "count", "lower_is_better", "internal", "Total WebGPU vertex-buffer allocations across one warmed active repaint of every chart."),
      ...samples("gpu_uploaded_bytes", rows.map((row) => row.gpu_uploaded_bytes), "bytes", "lower_is_better", "internal", "Total WebGPU vertex bytes uploaded across one warmed active repaint of every chart."),
      ...samples("wasm_linear_memory_bytes", rows.map((row) => row.wasm_linear_memory_bytes), "bytes", "lower_is_better", "public_candidate", "Global WASM linear memory for each chart-count workload; not per-chart RAM."),
    } };
  }
  if (scenario.kind === "multi_series") {
    const rows = [];
    let backend = null;
    for (const pane_count of scenario.pane_counts) {
      for (const series_count of scenario.series_counts) {
        if (rows.length > 0) {
          await page.reload();
          await page.waitForFunction(() => globalThis.__Aeris_bench_ready === true);
        }
        const result = await page.evaluate((input) => globalThis.__Aeris_bench.multi_series(input.points, input.seed, [input.series_count], [input.pane_count]), { ...scenario, seed, series_count, pane_count });
        rows.push(result.rows[0]);
        backend = result.backend;
      }
    }
    return { backend, metrics: {
      ...samples("series_count", rows.map((row) => row.series_count), "count", "informational", "internal", "Independent variable for aligned multi-series samples."),
      ...samples("pane_count", rows.map((row) => row.pane_count), "count", "informational", "internal", "Independent variable for aligned multi-pane samples."),
      ...samples("startup_ms", rows.map((row) => row.startup_ms), "ms", "lower_is_better", "public_candidate", "Public chart, pane, series, data, and render path through the following rAF."),
      ...samples("frame_cpu_ms", rows.map((row) => row.frame_cpu_ms), "ms", "lower_is_better", "public_candidate", "Existing frame_stats CPU duration for each series/pane combination."),
      ...samples("wasm_linear_memory_bytes", rows.map((row) => row.wasm_linear_memory_bytes), "bytes", "lower_is_better", "public_candidate", "Global WASM linear memory for each series/pane combination."),
    } };
  }
  if (scenario.kind === "general_dashboard") {
    const rows = [];
    for (let run = 0; run < scenario.warmup_runs + scenario.measured_runs; run += 1) {
      if (run > 0) {
        await page.reload();
        await page.waitForFunction(() => globalThis.__Aeris_bench_ready === true);
      }
      const result = await page.evaluate(
        (input) => globalThis.__Aeris_bench.general_dashboard(input.points),
        { ...scenario, seed },
      );
      if (run >= scenario.warmup_runs) rows.push(result);
    }
    return {
      backend: rows.at(-1).backend,
      metrics: {
        ...samples("startup_ms", rows.map((row) => row.startup_ms), "ms", "lower_is_better", "public_candidate", "Five-series Phase 2 general dashboard creation, 100k aligned rows, and first following rAF."),
        ...samples("frame_cpu_ms", rows.map((row) => row.frame_cpu_ms), "ms", "lower_is_better", "public_candidate", "Engine/frame CPU cost reported for the first completed representative general-dashboard frame."),
        ...samples("wasm_linear_memory_bytes", rows.map((row) => row.wasm_linear_memory_bytes), "bytes", "lower_is_better", "public_candidate", "WASM linear memory after the representative Phase 2 general dashboard reaches its first frame."),
        ...samples("gpu_buffer_allocations", rows.map((row) => row.gpu_buffer_allocations), "count", "lower_is_better", "internal", "WebGPU vertex-buffer allocations on the representative general-dashboard first frame."),
        ...samples("gpu_write_calls", rows.map((row) => row.gpu_write_calls), "count", "lower_is_better", "internal", "WebGPU queue buffer writes on the representative general-dashboard first frame."),
        ...samples("gpu_uploaded_bytes", rows.map((row) => row.gpu_uploaded_bytes), "bytes", "lower_is_better", "internal", "WebGPU vertex bytes uploaded on the representative general-dashboard first frame."),
      },
    };
  }
  if (scenario.kind === "retained_updates") {
    const result = await page.evaluate((input) => globalThis.__Aeris_bench.retained_updates(input.points, input.seed, input.series_counts, input.pane_counts, input.iterations), { ...scenario, seed });
    const frames = result.rows.flatMap((row) => row.samples);
    return { backend: result.backend, metrics: {
      ...samples("series_count", result.rows.map((row) => row.series_count), "count", "informational", "internal", "Independent variable for aligned retained-update samples."),
      ...samples("pane_count", result.rows.map((row) => row.pane_count), "count", "informational", "internal", "Independent variable for aligned retained-update samples."),
      ...samples("frame_cpu_ms", frames.map((stats) => stats.cpu_ms), "ms", "lower_is_better", "public_candidate", "Current-candle update through the next animation frame, measured by frame_stats CPU timing."),
      ...samples("series_rebuilds", frames.map((stats) => stats.series_rebuilds), "count", "lower_is_better", "internal", "Retained series layers rebuilt per current-candle frame."),
      ...samples("layout_rebuilds", frames.map((stats) => stats.layout_rebuilds), "count", "lower_is_better", "internal", "Retained layout rebuilds per current-candle frame."),
      ...samples("autoscale_runs", frames.map((stats) => stats.autoscale_runs), "count", "lower_is_better", "internal", "Autoscale passes per current-candle frame."),
      ...samples("grid_rebuilds", frames.map((stats) => stats.grid_rebuilds), "count", "lower_is_better", "internal", "Grid layers rebuilt per current-candle frame."),
      ...samples("gpu_buffer_allocations", frames.map((stats) => stats.gpu_buffer_allocations), "count", "lower_is_better", "internal", "WebGPU vertex-buffer allocations per current-candle frame after warm-up."),
      ...samples("gpu_write_calls", frames.map((stats) => stats.gpu_write_calls), "count", "lower_is_better", "internal", "WebGPU queue buffer writes per current-candle frame."),
      ...samples("gpu_uploaded_bytes", frames.map((stats) => stats.gpu_uploaded_bytes), "bytes", "lower_is_better", "internal", "WebGPU vertex bytes uploaded per current-candle frame."),
      ...samples("stable_gpu_buffer_allocations", result.rows.map((row) => row.stable.gpu_buffer_allocations), "count", "lower_is_better", "internal", "WebGPU vertex-buffer allocations on a warmed unchanged frame."),
      ...samples("stable_gpu_uploaded_bytes", result.rows.map((row) => row.stable.gpu_uploaded_bytes), "bytes", "lower_is_better", "internal", "WebGPU vertex bytes uploaded on a warmed unchanged frame."),
    } };
  }
  if (scenario.kind === "soak") {
    const result = await page.evaluate((input) => globalThis.__Aeris_bench.soak(input.points, input.seed, input.duration_ms, input.sample_interval_ms, input.memory_sample_interval_ms), { ...scenario, seed });
    const memory = result.samples.map((sample) => sample.page_bytes).filter((value) => value !== null);
    const memory_elapsed = result.samples.filter((sample) => sample.page_bytes !== null).map((sample) => sample.elapsed_ms);
    const frame = result.samples.map((sample) => sample.cpu_ms);
    const update_latency = result.samples.map((sample) => sample.update_api_ms);
    const wasm_memory = result.samples.map((sample) => sample.wasm_linear_memory_bytes);
    const sample_elapsed = result.samples.map((sample) => sample.elapsed_ms);
    const metrics = {
      ...samples("sample_elapsed_ms", sample_elapsed, "ms", "informational", "internal", "Elapsed monotonic time for each aligned periodic frame, update-latency, and WASM-memory sample."),
      ...(memory_elapsed.length > 0 ? samples("page_memory_sample_elapsed_ms", memory_elapsed, "ms", "informational", "internal", "Elapsed monotonic time for each aligned whole-page memory sample.") : { page_memory_sample_elapsed_ms: metric([], "ms", "informational", "internal", "No whole-page memory samples were collected.", "unsupported") }),
      ...samples("frame_cpu_ms", frame, "ms", "lower_is_better", "public_candidate", "Periodic existing frame_stats CPU samples during deterministic updates, pan, crosshair, and resize workload."),
      ...samples("update_api_latency_ms", update_latency, "ms", "lower_is_better", "public_candidate", "Per-period mean public update() call latency during the soak workload."),
      ...samples("wasm_linear_memory_bytes", wasm_memory, "bytes", "lower_is_better", "public_candidate", "Periodic global WASM linear-memory samples."),
      ...(memory.length > 0 ? samples("browser_page_memory_bytes", memory, "bytes", "lower_is_better", "public_candidate", "Periodic measureUserAgentSpecificMemory whole-page samples.") : { browser_page_memory_bytes: metric([], "bytes", "lower_is_better", "public_candidate", "measureUserAgentSpecificMemory unavailable.", "unsupported") }),
      ...scalar("updates_processed_per_second", result.updates / result.duration_ms * 1000, "updates/s", "higher_is_better", "public_candidate", "Completed deterministic updates divided by run duration."),
    };
    if (wasm_memory.length >= 2) Object.assign(metrics, scalar("wasm_memory_growth_bytes_per_hour", (wasm_memory.at(-1) - wasm_memory[0]) / (sample_elapsed.at(-1) - sample_elapsed[0]) * 3_600_000, "bytes/hour", "lower_is_better", "public_candidate", "First-to-last WASM linear-memory change normalized by the exact interval between those samples."));
    else metrics.wasm_memory_growth_bytes_per_hour = metric([], "bytes/hour", "lower_is_better", "public_candidate", "Fewer than two periodic WASM-memory samples were collected.", "unsupported");
    const frame_degradation = degradation(frame);
    if (frame_degradation !== null) Object.assign(metrics, scalar("frame_cpu_degradation_percent", frame_degradation, "percent", "lower_is_better", "public_candidate", "Second-half mean versus first-half mean of periodic frame CPU samples."));
    else metrics.frame_cpu_degradation_percent = metric([], "percent", "lower_is_better", "public_candidate", "At least four periodic samples are required for a degradation estimate.", "unsupported");
    const latency_degradation = degradation(update_latency);
    if (latency_degradation !== null) Object.assign(metrics, scalar("update_latency_degradation_percent", latency_degradation, "percent", "lower_is_better", "public_candidate", "Second-half mean versus first-half mean of periodic update API latency samples."));
    else metrics.update_latency_degradation_percent = metric([], "percent", "lower_is_better", "public_candidate", "At least four periodic samples are required for a degradation estimate.", "unsupported");
    if (memory.length >= 2) Object.assign(metrics, scalar("browser_page_memory_growth_bytes_per_hour", (memory.at(-1) - memory[0]) / (memory_elapsed.at(-1) - memory_elapsed[0]) * 3_600_000, "bytes/hour", "lower_is_better", "public_candidate", "First-to-last whole-page memory slope normalized by the exact interval between those samples."));
    else metrics.browser_page_memory_growth_bytes_per_hour = metric([], "bytes/hour", "lower_is_better", "public_candidate", "measureUserAgentSpecificMemory unavailable or returned too few samples.", "unsupported");
    return { backend: result.backend, metrics };
  }
  throw new Error(`unsupported browser scenario kind ${scenario.kind}`);
}

async function interaction(page, scenario) {
  const rows = [];
  let backend = null;
  for (let run = 0; run < scenario.warmup_runs + scenario.measured_runs; run += 1) {
    const prepared = await page.evaluate((input) => globalThis.__Aeris_bench.prepare_interaction(input.points, input.seed, { interval_seconds: input.interval_seconds, studies: input.studies, backend: input.backend }), { ...scenario, seed });
    backend = prepared.backend;
    const canvas = page.locator("#bench-root canvas").last();
    const box = await canvas.boundingBox();
    if (!box) throw new Error("benchmark canvas has no bounding box");
    await page.evaluate(() => globalThis.__Aeris_bench.start_frame_recording());
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    if (scenario.interaction === "pan") {
      await page.mouse.move(cx - box.width * 0.3, cy);
      await page.mouse.down();
      await page.mouse.move(cx + box.width * 0.3, cy, { steps: scenario.steps });
      await page.mouse.up();
    } else if (scenario.interaction === "zoom") {
      await page.mouse.move(cx, cy);
      for (let step = 0; step < scenario.steps; step += 1) await page.mouse.wheel(0, step % 2 === 0 ? -60 : 60);
    } else {
      await page.mouse.move(box.x + 10, cy);
      await page.mouse.move(box.x + box.width - 10, cy, { steps: scenario.steps });
    }
    const result = await page.evaluate(() => globalThis.__Aeris_bench.stop_frame_recording());
    if (run >= scenario.warmup_runs) rows.push(result);
  }
  const frame_ms = rows.flatMap((row) => row.frame_ms);
  const cpu_ms = rows.flatMap((row) => row.cpu_ms);
  const gpu_ms = rows.flatMap((row) => row.gpu_ms);
  const draw_calls = rows.flatMap((row) => row.draw_calls);
  const metrics = {
    ...samples("raf_frame_interval_ms", frame_ms, "ms", "lower_is_better", "public_candidate", `requestAnimationFrame callback intervals across ${scenario.measured_runs} actual Playwright pointer/wheel traces after ${scenario.warmup_runs} discarded trace(s); includes display cadence and scheduling.`),
    ...samples("frame_cpu_ms", cpu_ms, "ms", "lower_is_better", "public_candidate", "Existing frame_stats CPU duration sampled during measured interaction frames."),
      ...samples("draw_calls", draw_calls, "count", "lower_is_better", "internal", "Existing frame_stats draw-call count during measured interaction frames."),
      ...samples("canvas2d_ops", rows.flatMap((row) => row.canvas2d_ops), "count", "lower_is_better", "internal", "Visible Canvas2D fallback operations during measured interaction frames; WebGPU dense rendering should remain zero."),
      ...samples("gpu_buffer_allocations", rows.flatMap((row) => row.gpu_buffer_allocations), "count", "lower_is_better", "internal", "WebGPU vertex-buffer allocations during measured interaction frames."),
      ...samples("gpu_uploaded_bytes", rows.flatMap((row) => row.gpu_uploaded_bytes), "bytes", "lower_is_better", "internal", "WebGPU vertex bytes uploaded during measured interaction frames."),
      ...samples("layout_rebuilds", rows.flatMap((row) => row.layout_rebuilds), "count", "lower_is_better", "internal", "Retained layout rebuilds during measured interaction frames."),
      ...samples("autoscale_runs", rows.flatMap((row) => row.autoscale_runs), "count", "lower_is_better", "internal", "Autoscale passes during measured interaction frames."),
      ...samples("series_rebuilds", rows.flatMap((row) => row.series_rebuilds), "count", "lower_is_better", "internal", "Retained series rebuilds during measured interaction frames."),
      ...samples("drawing_rebuilds", rows.flatMap((row) => row.drawing_rebuilds), "count", "lower_is_better", "internal", "Retained drawing rebuilds during measured interaction frames."),
      ...samples("grid_rebuilds", rows.flatMap((row) => row.grid_rebuilds), "count", "lower_is_better", "internal", "Retained grid rebuilds during measured interaction frames."),
      ...samples("axis_rebuilds", rows.flatMap((row) => row.axis_rebuilds), "count", "lower_is_better", "internal", "Browser axis/top-layer rebuilds during measured interaction frames."),
      ...samples("overlay_rebuilds", rows.flatMap((row) => row.overlay_rebuilds), "count", "lower_is_better", "internal", "Retained overlay rebuilds during measured interaction frames."),
      ...samples("text_resolutions", rows.flatMap((row) => row.text_resolutions), "count", "lower_is_better", "internal", "Text atlas rasterizations during measured interaction frames."),
    ...samples("long_task_count", rows.map((row) => row.long_task_ms.length), "count", "lower_is_better", "public_candidate", "PerformanceObserver long-task entries recorded per measured interaction trace."),
    ...samples("long_task_total_ms", rows.map((row) => row.long_task_ms.reduce((sum, value) => sum + value, 0)), "ms", "lower_is_better", "public_candidate", "Total PerformanceObserver long-task duration per measured interaction trace."),
    ...samples("frames_over_8_33_ms", rows.map((row) => row.frame_ms.filter((value) => value > 8.33).length), "count", "lower_is_better", "public_candidate", "Recorded rAF intervals above the 120 Hz frame budget per measured trace."),
    ...samples("frames_over_16_67_ms", rows.map((row) => row.frame_ms.filter((value) => value > 16.67).length), "count", "lower_is_better", "public_candidate", "Recorded rAF intervals above the 60 Hz frame budget per measured trace."),
    ...samples("frames_over_33_33_ms", rows.map((row) => row.frame_ms.filter((value) => value > 33.33).length), "count", "lower_is_better", "public_candidate", "Recorded rAF intervals above two 60 Hz frame periods per measured trace."),
    ...samples("effective_presentation_rate_fps", rows.map((row) => row.frame_ms.length / row.frame_ms.reduce((sum, value) => sum + value, 0) * 1000), "frames/s", "higher_is_better", "public_candidate", "Observed rAF callbacks divided by the exact retained interval samples for each trace; capped by display/browser scheduling and not unconstrained renderer FPS."),
  };
  if (gpu_ms.length > 0) Object.assign(metrics, samples("gpu_render_pass_ms", gpu_ms, "ms", "lower_is_better", "public_candidate", "Actual WebGPU timestamp-query duration from render-pass beginning to end; asynchronous and capability-detected."));
  else metrics.gpu_render_pass_ms = metric([], "ms", "lower_is_better", "public_candidate", "No WebGPU timestamp query sample resolved on this adapter.", "unsupported");
  return { backend, metrics, gpu_timestamp_supported: gpu_ms.length > 0 };
}

export async function run_browser_scenarios(scenarios) {
  const port = Number(process.env.AERIS_CHARTS_BENCH_PORT ?? 4191);
  const base_url = `http://127.0.0.1:${port}`;
  const server = spawn(process.execPath, ["test_server.mjs"], { cwd: path.join(repository_root, "examples", "web_demo"), env: { ...process.env, AERIS_CHARTS_TEST_PORT: String(port) }, stdio: ["ignore", "ignore", "inherit"], windowsHide: true });
  await wait_for_server(`${base_url}/benchmark.html`, server);
  const browser = await chromium.launch({ channel: "chromium", headless: true, args: ["--enable-unsafe-webgpu", "--enable-dawn-features=allow_unsafe_apis", "--enable-webgpu-developer-features", "--use-gpu-in-tests", "--js-flags=--expose-gc"] });
  try {
    let browser_metadata = null;
    const outputs = [];
    for (const scenario of scenarios) {
      const started = performance.now();
      try {
        let result;
        let cpu_seconds = null;
        let js_heap_used_bytes = null;
        if (scenario.kind === "startup") {
          result = await cold_start(browser, base_url, scenario);
          browser_metadata ??= { ...result.environment, runtime: "Chromium", runtime_version: browser.version() };
        } else {
          const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1 });
          const page = await context.newPage();
          report_browser_errors(page);
          await page.goto(`${base_url}/benchmark.html`);
          await page.waitForFunction(() => globalThis.__Aeris_bench_ready === true);
          const session = await context.newCDPSession(page);
          await session.send("Performance.enable");
          await session.send("HeapProfiler.collectGarbage");
          if (scenario.kind === "lifecycle") {
            await page.evaluate((input) => globalThis.__Aeris_bench.prepare_lifecycle(input.points, input.seed), { ...scenario, seed });
            await session.send("HeapProfiler.collectGarbage");
          }
          const before = await collect_cdp(session);
          result = scenario.kind === "interaction" ? await interaction(page, scenario) : await run_page_scenario(page, scenario);
          await session.send("HeapProfiler.collectGarbage");
          const after = await collect_cdp(session);
          cpu_seconds = before.task_duration_seconds === null || after.task_duration_seconds === null || ["multi_chart", "multi_series"].includes(scenario.kind) ? null : after.task_duration_seconds - before.task_duration_seconds;
          js_heap_used_bytes = after.js_heap_used_bytes;
          if (scenario.kind === "lifecycle" && before.js_heap_used_bytes !== null && after.js_heap_used_bytes !== null) {
            Object.assign(result.metrics, scalar("browser_js_heap_retained_delta_bytes", after.js_heap_used_bytes - before.js_heap_used_bytes, "bytes", "lower_is_better", "public_candidate", "Chromium whole-page JS heap after minus before repeated lifecycle cycles; both samples follow DevTools forced GC."));
            result.forced_gc = true;
          }
          browser_metadata ??= { ...await page_environment(page), runtime: "Chromium", runtime_version: browser.version() };
          await context.close();
        }
        if (cpu_seconds !== null) Object.assign(result.metrics, scalar("browser_main_thread_task_cpu_ms", cpu_seconds * 1000, "ms", "lower_is_better", "internal", "Chromium DevTools Performance.TaskDuration delta for the page over the scenario interval."));
        if (js_heap_used_bytes !== null) Object.assign(result.metrics, scalar("browser_js_heap_used_bytes", js_heap_used_bytes, "bytes", "lower_is_better", "internal", "Chromium Runtime.getHeapUsage usedSize after a documented forced GC; whole-page JS heap, not Aeris-only."));
        outputs.push({
          id: scenario.id,
          version: scenario.version,
          status: "passed",
          error: null,
          reproduction_command: `node benchmarks/benchmark.mjs scenario ${scenario.id}`,
          dataset: scenario_dataset(scenario),
          execution: { warmup_runs: scenario.warmup_runs ?? 0, measured_runs: scenario.measured_runs ?? 1, benchmark_duration_ms: performance.now() - started, sampling_method: "automated Chromium via Playwright on the public package API", forced_gc: result.forced_gc ?? false },
          capabilities: { backend: result.backend ?? null, gpu_timestamp_supported: result.gpu_timestamp_supported ?? false, actual_visible_presentation_timestamp: false },
          metrics: result.metrics,
        });
      } catch (error) {
        outputs.push({ id: scenario.id, version: scenario.version, status: "failed", error: error.stack ?? String(error), reproduction_command: `node benchmarks/benchmark.mjs scenario ${scenario.id}`, dataset: scenario_dataset(scenario), execution: { warmup_runs: scenario.warmup_runs ?? 0, measured_runs: scenario.measured_runs ?? 0, benchmark_duration_ms: performance.now() - started, sampling_method: "automated Chromium via Playwright on the public package API", forced_gc: false }, capabilities: {}, metrics: {} });
      }
    }
    return { scenarios: outputs, browser: browser_metadata ?? { runtime: "Chromium", runtime_version: browser.version(), viewport: { width: 1280, height: 720 }, device_pixel_ratio: 1, gpu: null, gpu_vendor: null, gpu_architecture: null, backend: null, gpu_timestamp_supported: null, page_memory_supported: null } };
  } finally {
    await browser.close();
    server.kill();
  }
}
