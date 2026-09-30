import { generate_ohlcv } from "/benchmarks/shared.mjs";

const root = document.querySelector("#bench-root");
let package_module = null;
const live = [];
let frame_recording = null;
let lifecycle_fixture = null;

const next_frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

async function load_package() {
  package_module ??= await import("/dist/aeris_charts_financial.js");
  return package_module;
}

function host() {
  const element = document.createElement("div");
  element.className = "bench-host";
  root.append(element);
  root.style.setProperty("--columns", String(Math.ceil(Math.sqrt(root.childElementCount))));
  return element;
}

async function create(points = 0, seed = 0x02f6e2b1, options = {}, fixture = null, configuration = {}) {
  const api = await load_package();
  const container = host();
  const chart = await api.create_chart(container, { autoSize: true, ...options });
  const series = chart.add_series("candlestick");
  const columns = fixture ?? generate_ohlcv(points, seed, configuration);
  if (points > 0) series.set_data_typed(columns);
  const entry = { chart, series, container, columns };
  live.push(entry);
  return entry;
}

function remove_entry(entry) {
  entry.chart.remove();
  entry.container.remove();
  const index = live.indexOf(entry);
  if (index >= 0) live.splice(index, 1);
}

function reset() {
  while (live.length > 0) remove_entry(live.at(-1));
  root.replaceChildren();
  root.style.setProperty("--columns", "1");
}

async function page_memory() {
  if (typeof performance.measureUserAgentSpecificMemory !== "function") return null;
  try {
    return (await performance.measureUserAgentSpecificMemory()).bytes;
  } catch {
    return null;
  }
}

async function memory_snapshot(chart = live[0]?.chart ?? null) {
  if (typeof globalThis.gc === "function") globalThis.gc();
  await next_frame();
  return {
    page_bytes: await page_memory(),
    wasm_linear_memory_bytes: chart?.frame_stats().memory_bytes ?? null,
    forced_gc: typeof globalThis.gc === "function",
  };
}

async function environment() {
  let adapter_info = null;
  try {
    const adapter = await navigator.gpu?.requestAdapter();
    adapter_info = adapter?.info ?? null;
  } catch {
    adapter_info = null;
  }
  const probe = live[0]?.chart ?? null;
  return {
    user_agent: navigator.userAgent,
    device_pixel_ratio: devicePixelRatio,
    viewport: { width: innerWidth, height: innerHeight },
    gpu: adapter_info?.device ?? adapter_info?.description ?? null,
    gpu_vendor: adapter_info?.vendor ?? null,
    gpu_architecture: adapter_info?.architecture ?? null,
    backend: probe?.backend() ?? null,
    gpu_timestamp_supported: navigator.gpu === undefined ? false : null,
    page_memory_supported: typeof performance.measureUserAgentSpecificMemory === "function",
  };
}

async function startup(points, seed) {
  reset();
  package_module = null;
  const columns = generate_ohlcv(points, seed);
  const started = performance.now();
  const import_started = performance.now();
  const api = await load_package();
  const module_import_ms = performance.now() - import_started;
  const wasm_started = performance.now();
  await api.init_wasm();
  const wasm_init_ms = performance.now() - wasm_started;
  const container = host();
  const create_started = performance.now();
  const chart = await api.create_chart(container, { autoSize: true });
  const chart_create_api_ms = performance.now() - create_started;
  const series_started = performance.now();
  const series = chart.add_series("candlestick");
  const series_create_api_ms = performance.now() - series_started;
  let set_data_api_ms = 0;
  if (points > 0) {
    const data_started = performance.now();
    series.set_data_typed(columns);
    set_data_api_ms = performance.now() - data_started;
  }
  const stats = chart.frame_stats();
  await next_frame();
  const first_raf_after_ready_ms = performance.now() - started;
  live.push({ chart, series, container, columns });
  return { module_import_ms, wasm_init_ms, chart_create_api_ms, series_create_api_ms, set_data_api_ms, first_raf_after_ready_ms, first_frame_cpu_ms: stats.cpu_ms, presented_frames_at_ready: stats.presented_frames, backend: chart.backend() };
}

async function historical(points, seed, warmup_runs, measured_runs) {
  reset();
  const entry = await create(0, seed);
  const columns = generate_ohlcv(points, seed);
  const api_samples = [];
  const ready_samples = [];
  const cpu_samples = [];
  for (let run = 0; run < warmup_runs + measured_runs; run += 1) {
    const started = performance.now();
    entry.series.set_data_typed(columns);
    const api_ms = performance.now() - started;
    const cpu_ms = entry.chart.frame_stats().cpu_ms;
    await next_frame();
    const ready_ms = performance.now() - started;
    if (run >= warmup_runs) {
      api_samples.push(api_ms);
      ready_samples.push(ready_ms);
      cpu_samples.push(cpu_ms);
    }
  }
  return { api_samples, ready_samples, cpu_samples, stats: entry.chart.frame_stats(), backend: entry.chart.backend() };
}

async function realtime(points, seed, mode, update_rate_hz, duration_ms) {
  reset();
  const entry = await create(points, seed);
  const interval = 1000 / update_rate_hz;
  const deadline = performance.now() + duration_ms;
  const api_samples = [];
  const frame_cpu_samples = [];
  const frame_stats_samples = [];
  let updates = 0;
  let last_time = entry.columns.times.at(-1);
  let value = entry.columns.close.at(-1);
  let update_latency_total = 0;
  let update_latency_count = 0;
  const before = entry.chart.frame_stats();
  const started = performance.now();
  while (performance.now() < deadline) {
    const target = started + updates * interval;
    const wait = target - performance.now();
    if (wait > 0) await delay(wait);
    value += Math.sin(updates * 0.17) * 0.02;
    if (mode === "append") last_time += 60;
    const call_started = performance.now();
    entry.series.update({ time: last_time, open: value, high: value + 0.2, low: value - 0.2, close: value });
    api_samples.push(performance.now() - call_started);
    await next_frame();
    const frame_stats = entry.chart.frame_stats();
    frame_cpu_samples.push(frame_stats.cpu_ms);
    frame_stats_samples.push(frame_stats);
    updates += 1;
  }
  const elapsed_ms = performance.now() - started;
  const after = entry.chart.frame_stats();
  return {
    api_samples,
    frame_cpu_samples,
    frame_stats_samples,
    elapsed_ms,
    updates,
    presented_frames: after.presented_frames - before.presented_frames,
    dropped_frames: after.dropped_frames - before.dropped_frames,
    ring_overruns: after.ring_overruns - before.ring_overruns,
    backend: entry.chart.backend(),
  };
}

// The studies that restart at every trading-day boundary. On daily bars every bar is its own period,
// so each of the 11 outputs (session VWAP, its 5 bands, 5 standard pivot levels) draws one bar-wide
// segment per visible bar.
function add_session_reset_studies({ chart, series, columns }) {
  const volume = chart.add_series("histogram", { visible: false });
  volume.set_data_typed({ times: columns.times, open: columns.volume, high: columns.volume, low: columns.volume, close: columns.volume });
  chart.add_vwap(series, volume);
  chart.add_vwap_bands(series, "session", 1, 1, volume);
  chart.add_pivot_points(series, "standard");
}

async function prepare_interaction(points, seed, { interval_seconds, studies, backend } = {}) {
  reset();
  const entry = await create(points, seed, backend === undefined ? {} : { backend }, null, { interval_seconds });
  if (studies === "session_reset") add_session_reset_studies(entry);
  else if (studies !== undefined) throw new Error(`unknown study set ${studies}`);
  const scale = entry.chart.time_scale();
  scale.apply_options({ min_bar_spacing: 1280 / Math.max(points, 1) / 2 });
  scale.set_visible_logical_range({ from: 0, to: Math.max(points - 1, 0) });
  entry.chart.render();
  await next_frame();
  return { backend: entry.chart.backend() };
}

function start_frame_recording() {
  if (frame_recording !== null) throw new Error("frame recording already active");
  const record = { active: true, last: performance.now(), last_presented: null, frame_ms: [], cpu_ms: [], gpu_ms: [], draw_calls: [], canvas2d_ops: [], gpu_buffer_allocations: [], gpu_uploaded_bytes: [], layout_rebuilds: [], autoscale_runs: [], series_rebuilds: [], drawing_rebuilds: [], grid_rebuilds: [], axis_rebuilds: [], overlay_rebuilds: [], text_resolutions: [], long_task_ms: [], long_task_observer: null, started: performance.now() };
  if (PerformanceObserver.supportedEntryTypes?.includes("longtask")) {
    record.long_task_observer = new PerformanceObserver((entries) => {
      for (const entry of entries.getEntries()) record.long_task_ms.push(entry.duration);
    });
    record.long_task_observer.observe({ type: "longtask" });
  }
  frame_recording = record;
  const tick = (time) => {
    if (!record.active) return;
    const stats = live[0]?.chart.frame_stats();
    record.frame_ms.push(time - record.last);
    record.last = time;
    if (stats && stats.presented_frames !== record.last_presented) {
      record.cpu_ms.push(stats.cpu_ms);
      if (stats.gpu_ms !== null) record.gpu_ms.push(stats.gpu_ms);
      record.draw_calls.push(stats.draw_calls);
      record.canvas2d_ops.push(stats.canvas2d_ops);
      record.gpu_buffer_allocations.push(stats.gpu_buffer_allocations);
      record.gpu_uploaded_bytes.push(stats.gpu_uploaded_bytes);
      record.layout_rebuilds.push(stats.layout_rebuilds);
      record.autoscale_runs.push(stats.autoscale_runs);
      record.series_rebuilds.push(stats.series_rebuilds);
      record.drawing_rebuilds.push(stats.drawing_rebuilds);
      record.grid_rebuilds.push(stats.grid_rebuilds);
      record.axis_rebuilds.push(stats.axis_rebuilds);
      record.overlay_rebuilds.push(stats.overlay_rebuilds);
      record.text_resolutions.push(stats.text_resolutions);
      record.last_presented = stats.presented_frames;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

async function stop_frame_recording() {
  const record = frame_recording;
  if (record === null) throw new Error("frame recording is not active");
  if (record.long_task_observer !== null) {
    for (const entry of record.long_task_observer.takeRecords()) record.long_task_ms.push(entry.duration);
    record.long_task_observer.disconnect();
  }
  record.active = false;
  frame_recording = null;
  await next_frame();
  return { ...Object.fromEntries(Object.entries(record).filter(([, value]) => Array.isArray(value)).map(([key, value]) => [key, key === "long_task_ms" ? value : value.slice(2)])), duration_ms: performance.now() - record.started, stats: live[0].chart.frame_stats() };
}

async function prepare_lifecycle(points, seed) {
  reset();
  lifecycle_fixture = { points, seed, columns: generate_ohlcv(points, seed) };
  const warmup = await create(points, seed, {}, lifecycle_fixture.columns);
  warmup.chart.time_scale().fit_content();
  warmup.chart.render();
  await next_frame();
  remove_entry(warmup);
}

async function lifecycle(points, seed, cycles) {
  reset();
  if (lifecycle_fixture?.points !== points || lifecycle_fixture?.seed !== seed) throw new Error("lifecycle warm-up fixture is missing");
  const baseline = await memory_snapshot(null);
  const wasm_samples = [];
  let peak_page_bytes = baseline.page_bytes;
  let backend = null;
  const page_sample_target = cycles <= 10 ? 1 : Math.min(10, Math.ceil(cycles / 10));
  const page_sample_stride = Math.ceil(cycles / page_sample_target);
  for (let cycle = 0; cycle < cycles; cycle += 1) {
    const entry = await create(points, seed, {}, lifecycle_fixture.columns);
    backend ??= entry.chart.backend();
    entry.chart.time_scale().fit_content();
    entry.chart.render();
    await next_frame();
    wasm_samples.push(entry.chart.frame_stats().memory_bytes);
    if ((cycle + 1) % page_sample_stride === 0) {
      const loaded = await memory_snapshot(entry.chart);
      if (loaded.page_bytes !== null) peak_page_bytes = Math.max(peak_page_bytes ?? 0, loaded.page_bytes);
    }
    remove_entry(entry);
  }
  const final = await memory_snapshot(null);
  return { baseline, final, peak_page_bytes, wasm_linear_memory_samples: wasm_samples, cycles, loaded_page_memory_sample_count: page_sample_target, backend };
}

async function multi_chart(points, seed, chart_counts) {
  reset();
  const rows = [];
  for (const count of chart_counts) {
    reset();
    const fixtures = Array.from({ length: count }, (_, index) => generate_ohlcv(points, seed + index));
    const started = performance.now();
    for (let index = 0; index < count; index += 1) await create(points, seed + index, {}, fixtures[index]);
    await next_frame();
    for (const entry of live) {
      const scale = entry.chart.time_scale();
      scale.apply_options({ min_bar_spacing: entry.container.clientWidth / Math.max(points, 1) / 2 });
      scale.set_visible_logical_range({ from: 0, to: Math.max(points - 1, 0) });
      entry.chart.render();
    }
    await next_frame();
    const startup_ms = performance.now() - started;
    const repaint_started = performance.now();
    for (const entry of live) entry.chart.render();
    await next_frame();
    const active_repaint_ms = performance.now() - repaint_started;
    const stats = live.map((entry) => entry.chart.frame_stats());
    rows.push({ count, startup_ms, active_repaint_ms, wasm_linear_memory_bytes: stats[0]?.memory_bytes ?? null, frame_cpu_ms: stats.map((entry) => entry.cpu_ms), gpu_buffer_allocations: stats.reduce((sum, entry) => sum + entry.gpu_buffer_allocations, 0), gpu_uploaded_bytes: stats.reduce((sum, entry) => sum + entry.gpu_uploaded_bytes, 0) });
  }
  return { rows, backend: live[0]?.chart.backend() ?? null };
}

async function multi_series(points, seed, series_counts, pane_counts) {
  reset();
  const rows = [];
  const columns = generate_ohlcv(points, seed);
  for (const pane_count of pane_counts) {
    for (const series_count of series_counts) {
      reset();
      const api = await load_package();
      const container = host();
      const started = performance.now();
      const chart = await api.create_chart(container, { autoSize: true });
      for (let pane = 1; pane < pane_count; pane += 1) chart.add_pane(true);
      for (let index = 0; index < series_count; index += 1) {
        const series = chart.add_series(index === 0 ? "candlestick" : "line", { pane: index % pane_count });
        series.set_data_typed(columns);
      }
      const scale = chart.time_scale();
      scale.apply_options({ min_bar_spacing: container.clientWidth / Math.max(points, 1) / 2 });
      scale.set_visible_logical_range({ from: 0, to: Math.max(points - 1, 0) });
      chart.render();
      await next_frame();
      const startup_ms = performance.now() - started;
      live.push({ chart, series: null, container, columns });
      const stats = chart.frame_stats();
      rows.push({ pane_count, series_count, startup_ms, wasm_linear_memory_bytes: stats.memory_bytes, frame_cpu_ms: stats.cpu_ms });
    }
  }
  return { rows, backend: live[0]?.chart.backend() ?? null };
}

async function general_dashboard(points) {
  reset();
  const api = await load_package();
  const container = host();
  const rows_per_series = Math.max(1, Math.floor(points / 5));
  const x = new Float64Array(rows_per_series);
  const y = new Float64Array(rows_per_series);
  const low = new Float64Array(rows_per_series);
  const high = new Float64Array(rows_per_series);
  const size = new Float64Array(rows_per_series);
  for (let index = 0; index < rows_per_series; index += 1) {
    const xv = index;
    const yv = 50 + Math.sin(index * 0.017) * 20 + Math.cos(index * 0.003) * 5;
    x[index] = xv;
    y[index] = yv;
    low[index] = yv - 4;
    high[index] = yv + 4;
    size[index] = 4 + (index % 25);
  }

  const started = performance.now();
  const chart = await api.create_chart(container, { autoSize: true });
  const pane = chart.add_pane({
    preserve_empty: true,
    horizontal_domain: { type: "continuous", scale: "linear" },
  });
  chart.add_axis({ id: "bench-general-x", pane: pane.pane_index(), dimension: "x", scale: "linear" });
  chart.add_axis({ id: "bench-general-y", pane: pane.pane_index(), dimension: "y", scale: "linear" });
  const common = { pane: pane.pane_index(), x_axis_id: "bench-general-x", y_axis_id: "bench-general-y" };
  const line = chart.add_series("xy_line", common);
  const area = chart.add_series("xy_area", common);
  const range = chart.add_series("range_area", common);
  const scatter = chart.add_series("scatter", common);
  const bubble = chart.add_series("bubble", common);
  line.set_data_typed({ x, y });
  area.set_data_typed({ x, y });
  range.set_data_typed({ x, low, high });
  scatter.set_data_typed({ x, y });
  bubble.set_data_typed({ x, y, size });
  chart.render();
  await next_frame();
  const startup_ms = performance.now() - started;
  const stats = chart.frame_stats();
  live.push({ chart, series: line, container, columns: null });
  return {
    startup_ms,
    frame_cpu_ms: stats.cpu_ms,
    wasm_linear_memory_bytes: stats.memory_bytes,
    gpu_buffer_allocations: stats.gpu_buffer_allocations,
    gpu_write_calls: stats.gpu_write_calls,
    gpu_uploaded_bytes: stats.gpu_uploaded_bytes,
    backend: chart.backend(),
    rows_per_series,
    series_count: 5,
  };
}

async function retained_updates(points, seed, series_counts, pane_counts, iterations) {
  reset();
  const rows = [];
  const columns = generate_ohlcv(points, seed);
  for (const pane_count of pane_counts) {
    for (const series_count of series_counts) {
      reset();
      const api = await load_package();
      const container = host();
      const chart = await api.create_chart(container, { autoSize: true });
      for (let pane = 1; pane < pane_count; pane += 1) chart.add_pane(true);
      const series = [];
      for (let index = 0; index < series_count; index += 1) {
        const item = chart.add_series(index === 0 ? "candlestick" : "line", { pane: index % pane_count });
        item.set_data_typed(columns);
        series.push(item);
      }
      chart.time_scale().fit_content();
      chart.render();
      await next_frame();
      chart.render();
      await next_frame();
      const stable = chart.frame_stats();
      const target = series[Math.floor(series.length / 2)];
      const last_time = columns.times.at(-1);
      const base = columns.close.at(-1);
      const samples = [];
      for (let update = 0; update < iterations; update += 1) {
        const value = base + Math.sin(update * 0.17) * 0.01;
        target.update({ time: last_time, open: value, high: value + 0.02, low: value - 0.02, close: value });
        await next_frame();
        samples.push(chart.frame_stats());
      }
      live.push({ chart, series: null, container, columns });
      rows.push({ pane_count, series_count, stable, samples });
    }
  }
  return { rows, backend: live[0]?.chart.backend() ?? null };
}

async function soak(points, seed, duration_ms, sample_interval_ms, memory_sample_interval_ms) {
  reset();
  const entry = await create(points, seed);
  entry.chart.time_scale().fit_content();
  const started = performance.now();
  const samples = [];
  let next_sample = started;
  let next_memory_sample = started + memory_sample_interval_ms;
  let updates = 0;
  let last_time = entry.columns.times.at(-1);
  let value = entry.columns.close.at(-1);
  let update_latency_total = 0;
  let update_latency_count = 0;
  while (performance.now() - started < duration_ms) {
    value += Math.sin(updates * 0.17) * 0.02;
    if (updates > 0 && updates % 60 === 0) last_time += 60;
    const update_started = performance.now();
    entry.series.update({ time: last_time, open: value, high: value + 0.2, low: value - 0.2, close: value });
    update_latency_total += performance.now() - update_started;
    update_latency_count += 1;
    if (updates % 120 === 0) entry.chart.time_scale().scroll_to_position((updates / 120) % 20, false);
    if (updates % 180 === 0) entry.chart.set_crosshair_position(value, last_time, entry.series);
    if (updates > 0 && updates % 240 === 0) entry.chart.resize(updates % 480 === 0 ? 1280 : 1200, 720, 1);
    await next_frame();
    updates += 1;
    if (performance.now() >= next_sample) {
      let page_bytes = null;
      if (performance.now() >= next_memory_sample) {
        page_bytes = (await memory_snapshot(entry.chart)).page_bytes;
        next_memory_sample += memory_sample_interval_ms;
      }
      const stats = entry.chart.frame_stats();
      samples.push({ elapsed_ms: performance.now() - started, page_bytes, wasm_linear_memory_bytes: stats.memory_bytes, update_api_ms: update_latency_total / update_latency_count, ...stats });
      update_latency_total = 0;
      update_latency_count = 0;
      next_sample = performance.now() + sample_interval_ms;
    }
  }
  return { duration_ms: performance.now() - started, updates, samples, backend: entry.chart.backend() };
}

globalThis.__Aeris_bench = { environment, general_dashboard, historical, lifecycle, memory_snapshot, multi_chart, multi_series, prepare_interaction, prepare_lifecycle, realtime, reset, retained_updates, soak, start_frame_recording, startup, stop_frame_recording };
globalThis.__Aeris_bench_ready = true;
