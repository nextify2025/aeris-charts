import { create_offscreen_chart } from "./dist/aeris_charts_financial.js";

let chart = null;
let gpu_canvas = null;
let fallback_canvas = null;
let timer = null;
let frame = 0;

function columns(count) {
  const times = new Float64Array(count);
  const open = new Float64Array(count);
  const high = new Float64Array(count);
  const low = new Float64Array(count);
  const close = new Float64Array(count);
  let price = 100;
  for (let i = 0; i < count; i += 1) {
    const next = price + Math.sin(i * 0.037) * 0.45;
    times[i] = 1_577_836_800 + i * 60;
    open[i] = price;
    high[i] = Math.max(price, next) + 0.2;
    low[i] = Math.min(price, next) - 0.2;
    close[i] = next;
    price = next;
  }
  return { times, open, high, low, close };
}

function state(type = "state") {
  postMessage({
    type,
    backend: chart.backend(),
    stats: chart.frame_stats(),
    range: chart.visible_logical_range(),
    size: [gpu_canvas.width, gpu_canvas.height],
    frame,
  });
}

self.onmessage = async (event) => {
  try {
    const message = event.data;
    if (message.type === "init") {
      gpu_canvas = message.gpu_canvas;
      fallback_canvas = message.fallback_canvas;
      chart = await create_offscreen_chart(gpu_canvas, fallback_canvas, {
        width: message.width,
        height: message.height,
        dpr: message.dpr,
        options: {
          backend: message.backend,
          ...(message.time_scale ? { timeScale: message.time_scale } : {}),
        },
        force_fallback_adapter: message.force_fallback_adapter,
      });
      chart.set_data_typed(columns(message.bars ?? 5_000));
      chart.fit_content();
      chart.subscribe_backend_change(() => state("backend_change"));
      state("ready");
      return;
    }
    if (chart === null) throw new Error("offscreen chart is not initialized");
    if (message.type === "pointer") chart.inject_pointer_event(message.event);
    else if (message.type === "wheel") chart.inject_wheel_event(message.event);
    else if (message.type === "key") chart.inject_key_event(message.event);
    else if (message.type === "resize") chart.resize(message.width, message.height, message.dpr);
    else if (message.type === "start") {
      if (timer !== null) clearInterval(timer);
      timer = setInterval(() => {
        frame += 1;
        const x = 30 + (frame * 11) % Math.max(40, message.width - 120);
        chart.inject_pointer_event({ type: "move", x, y: message.height * 0.45 });
      }, 16);
    } else if (message.type === "stop") {
      if (timer !== null) clearInterval(timer);
      timer = null;
    } else if (message.type === "simulate_loss") {
      // Test fixture only: TypeScript `private` is erased in the bundle; this invokes the wasm
      // hook and lets subscribe_backend_change publish the post-render backend.
      chart.wasm.simulate_device_loss_for_test();
      return;
    } else if (message.type === "invalid_timestamp") {
      chart.update_typed({
        times: new Float64Array([1_725_000_000_000]),
        open: new Float64Array([100]),
        high: new Float64Array([101]),
        low: new Float64Array([99]),
        close: new Float64Array([100]),
      });
      postMessage({
        type: "timestamp_diagnostics",
        diagnostics: chart.last_ingestion_diagnostics(),
      });
      return;
    } else if (message.type === "time_zone") {
      // Declarative exchange time zone: IANA resolution runs inside the worker.
      try {
        if (message.options) chart.apply_options(message.options);
      } catch (error) {
        postMessage({ type: "time_zone_error", code: error.code, message: error.message });
        return;
      }
      const options = JSON.parse(chart.wasm.time_scale_options_json());
      postMessage({
        type: "time_zone",
        time_zone: options.time_zone,
        session_start: options.session_start,
        tick_marks: options.tick_marks,
        local: chart.wasm.exchange_local_seconds(message.probe),
      });
      // One reply per request: a trailing state message would race the reply in `send`.
      return;
    } else if (message.type === "sequenced_stream") {
      // Worker-side streaming: the sequence guard on typed updates and typed partial merges.
      const last = 1_577_836_800 + ((message.bars ?? 5_000) - 1) * 60;
      const row = (time, value) => ({
        times: new Float64Array([time]),
        open: new Float64Array([value]),
        high: new Float64Array([value]),
        low: new Float64Array([value]),
        close: new Float64Array([value]),
      });
      const results = {};
      chart.update_typed(row(last, 101), 0, { sequence: 3 });
      results.applied = chart.last_ingestion_diagnostics();
      chart.update_typed(row(last, 102), 0, { sequence: 3 });
      results.stale = chart.last_ingestion_diagnostics();
      chart.merge_typed({ times: new Float64Array([last + 60]), close: new Float64Array([99]) }, 0, { sequence: 4 });
      results.merged = chart.last_ingestion_diagnostics();
      chart.merge_typed({ times: new Float64Array([last + 60]), close: new Float64Array([98]) }, 0, { sequence: 4 });
      results.merge_stale = chart.last_ingestion_diagnostics();
      chart.merge_typed({ times: new Float64Array([last + 60]) }, 0);
      results.empty = chart.last_ingestion_diagnostics();
      postMessage({ type: "sequenced_stream", results });
      return;
    } else if (message.type === "as_of_overlay") {
      // A second market calendar: every seventh minute, 30 s off the primary's minutes. As-of it
      // adds no time point; as a union series it would add one per row.
      const main = columns(message.bars ?? 5_000);
      const count = Math.floor(main.times.length / 7);
      const overlay = columns(count);
      for (let i = 0; i < count; i += 1) overlay.times[i] = main.times[i * 7] + 30;
      let first_invalid = null;
      try {
        chart.add_series("candlestick", { time_alignment: "asof" });
      } catch (error) {
        first_invalid = error.code;
      }
      const primary = chart.add_series("candlestick"); // adopts the primary that `init` filled
      chart.fit_content();
      const range_before = chart.visible_logical_range();
      const id = chart.add_series("line", { time_alignment: "as_of" });
      chart.set_data_typed(overlay, id);
      chart.fit_content();
      const range_after = chart.visible_logical_range();
      const time_alignment = JSON.parse(chart.wasm.series_options_json(id)).time_alignment;
      const union = chart.add_series("line");
      chart.set_data_typed(overlay, union);
      chart.fit_content();
      const union_points_grew = chart.visible_logical_range().to > range_before.to;
      const ids_before_invalid = Array.from(chart.wasm.pane_series_ids(0));
      let invalid = null;
      try {
        chart.add_series("line", { as_of_max_staleness: -5 });
      } catch (error) {
        invalid = error.code;
      }
      try {
        chart.add_series("line", { time_alignment: "union", as_of_max_staleness: 60 });
      } catch {
        // Refused like the call above.
      }
      const ids_after_invalid = Array.from(chart.wasm.pane_series_ids(0));
      postMessage({
        type: "as_of_overlay", range_before, range_after, time_alignment, union_points_grew, invalid,
        first_invalid, primary, ids_before_invalid, ids_after_invalid,
      });
      return;
    } else if (message.type === "remove") {
      if (timer !== null) clearInterval(timer);
      timer = null;
      chart.remove();
      chart = null;
      postMessage({ type: "removed" });
      return;
    }
    state();
  } catch (error) {
    postMessage({ type: "error", message: error instanceof Error ? error.stack ?? error.message : String(error) });
  }
};
