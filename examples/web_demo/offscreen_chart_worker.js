import { create_offscreen_chart } from "./dist/aeris_charts_financial.js";

// Canvas text the worker's own engine paints (Canvas2D backend), so tests can read the axis strings
// a worker chart draws: the OffscreenCanvas cannot be read back from the page.
const painted_text = [];
{
  const prototype = self.OffscreenCanvasRenderingContext2D?.prototype;
  const original = prototype?.fillText;
  if (original) {
    prototype.fillText = function (text, ...args) {
      painted_text.push(String(text));
      return original.call(this, text, ...args);
    };
  }
}

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

// A 32-bit FNV-1a hash of the pixels Canvas2D last painted into the fallback surface.
function canvas_hash() {
  const context = fallback_canvas.getContext("2d");
  const { data } = context.getImageData(0, 0, fallback_canvas.width, fallback_canvas.height);
  let hash = 2166136261;
  for (let i = 0; i < data.length; i += 1) hash = Math.imul(hash ^ data[i], 16777619);
  return hash >>> 0;
}

function state(type = "state") {
  const crosshair = new Float64Array(2);
  chart.wasm.controller_crosshair_into(crosshair);
  postMessage({
    type,
    backend: chart.backend(),
    stats: chart.frame_stats(),
    range: chart.visible_logical_range(),
    logical_320: chart.wasm.coordinate_to_logical(320 - chart.wasm.pane_left()),
    price_range: chart.wasm.price_scale_visible_range(0, 0),
    bar_spacing: chart.wasm.bar_spacing(),
    crosshair: Number.isFinite(crosshair[0]) && Number.isFinite(crosshair[1])
      ? [crosshair[0], crosshair[1]] : null,
    axis_x: chart.wasm.pane_left() + chart.wasm.time_scale_width() + 12,
    axis_y: chart.wasm.pane_height(0) / 2,
    time_axis_y: chart.wasm.pane_height(0) + chart.wasm.time_scale_height() / 2,
    series_ids: JSON.parse(chart.wasm.series_order_json()),
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
    else if (message.type === "options") chart.apply_options(message.options);
    else if (message.type === "select_series") chart.wasm.set_selected_series(message.id);
    else if (message.type === "add_series") chart.add_series(message.kind);
    else if (message.type === "bar_spacing") chart.wasm.set_bar_spacing(message.value);
    else if (message.type === "price_range") chart.wasm.set_price_scale_visible_range(0, 0, message.from, message.to);
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
        bar_time_label: options.bar_time_label,
        local: chart.wasm.exchange_local_seconds(message.probe),
        printed: chart.wasm.bar_label_time(message.probe),
      });
      // One reply per request: a trailing state message would race the reply in `send`.
      return;
    } else if (message.type === "axis_texts") {
      // Hover a point of the pane and report the canvas text the frame paints.
      painted_text.length = 0;
      chart.inject_pointer_event({ type: "move", x: message.x, y: message.y });
      chart.render();
      postMessage({ type: "axis_texts", texts: [...painted_text] });
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
      const time_alignment = chart.series_options(id).time_alignment;
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
    } else if (message.type === "as_of_overlay_update") {
      // Changing `time_alignment` after creation through `apply_series_options`: one union overlay
      // joins the primary's calendar, goes as-of and back, then the refusals, the id checks, and
      // disposal run. Everything is reported in a single reply.
      const main = columns(message.bars ?? 5_000);
      const count = Math.floor(main.times.length / 7);
      const overlay = columns(count);
      for (let i = 0; i < count; i += 1) overlay.times[i] = main.times[i * 7] + 30;
      const failure = (run) => {
        try {
          run();
          return null;
        } catch (error) {
          return { code: error.code, message: error.message };
        }
      };
      const code_of = (run) => failure(run)?.code ?? null;
      const fitted = () => {
        chart.fit_content();
        return chart.visible_logical_range();
      };
      const primary = chart.add_series("candlestick"); // adopts the primary that `init` filled
      const primary_range = fitted();
      const id = chart.add_series("line"); // a union overlay: its 30 s-offset minutes join the axis
      chart.set_data_typed(overlay, id);
      const union_range = fitted();
      const alignment = (series_id = id) => {
        const { time_alignment, as_of_max_staleness } = chart.series_options(series_id);
        return { time_alignment, as_of_max_staleness };
      };
      const results = { primary, primary_range, union_range };

      // Repaint: a call presents its own frame before it returns. No fit or render follows the
      // calls below, so a dropped repaint leaves the last frame on the canvas and the counter
      // where it was. The pixels are read from the fallback surface, the one Canvas2D paints.
      const presented = () => chart.frame_stats().presented_frames;
      const frames = [presented()];
      const pixels_before = canvas_hash();
      chart.apply_series_options({ time_alignment: "as_of" }, id); // a change
      frames.push(presented());
      const pixels_after = canvas_hash();
      chart.apply_series_options({ time_alignment: "as_of" }, id); // equal to the current values
      frames.push(presented());
      code_of(() => chart.apply_series_options({ as_of_max_staleness: -1 }, id)); // refused
      frames.push(presented());
      results.repaint = { frames, pixels_changed: pixels_after !== pixels_before };

      results.as_of = { range: fitted(), options: alignment() };
      chart.apply_series_options({ as_of_max_staleness: 0 }, id);
      results.staleness = alignment();
      chart.apply_series_options({ as_of_max_staleness: null }, id);
      results.unbounded = alignment();

      // A request equal to the current values makes no engine call; a change makes exactly one.
      // TypeScript `private` is erased in the bundle: the wasm handle is wrapped to count calls.
      const engine = chart.wasm;
      const engine_set = engine.set_series_time_alignment;
      let engine_calls = 0;
      engine.set_series_time_alignment = function (...args) {
        engine_calls += 1;
        return engine_set.apply(this, args);
      };
      const calls_for = (patch) => {
        const before = engine_calls;
        chart.apply_series_options(patch, id);
        return engine_calls - before;
      };
      try {
        results.noop_calls = [
          calls_for({ time_alignment: "as_of" }),
          calls_for({ time_alignment: "as_of", as_of_max_staleness: null, color: undefined }),
          calls_for({}),
        ];
        results.change_calls = [calls_for({ as_of_max_staleness: 60 }), calls_for({ as_of_max_staleness: 60 })];
      } finally {
        delete engine.set_series_time_alignment;
      }
      results.bounded = alignment();
      chart.apply_series_options({ time_alignment: "union" }, id);
      results.union_again = { range: fitted(), options: alignment() };

      // The refusal table from the spec: each patch is refused in the named state, and nothing
      // about the series changes.
      results.refusals = message.refusals.map(({ patch, on }) => {
        chart.apply_series_options({ time_alignment: on }, id);
        const before = JSON.stringify(chart.series_options(id));
        const code = code_of(() => chart.apply_series_options(patch, id));
        return { code, unchanged: JSON.stringify(chart.series_options(id)) === before };
      });
      chart.apply_series_options({ time_alignment: "union" }, id);

      // Atomicity: a refusal applies none of the call's options, and nothing is silently dropped.
      const color_before = chart.series_options(id).color;
      results.atomic = {
        invalid: code_of(() => chart.apply_series_options({ time_alignment: "as_of", as_of_max_staleness: -1 }, id)),
        invalid_alignment: alignment().time_alignment,
        unsupported: failure(() => chart.apply_series_options({ color: "#ff0000" }, id)),
        color_unchanged: chart.series_options(id).color === color_before,
        mixed: code_of(() => chart.apply_series_options({ time_alignment: "as_of", color: "#ff0000" }, id)),
        mixed_alignment: alignment().time_alignment,
        null_options: code_of(() => chart.apply_series_options(null, id)),
      };

      // A footprint series, added after the adopted primary, owns no calendar to align.
      const footprint = chart.add_series("footprint");
      const footprint_before = JSON.stringify(chart.series_options(footprint));
      results.footprint = {
        code: code_of(() => chart.apply_series_options({ time_alignment: "as_of" }, footprint)),
        unchanged: JSON.stringify(chart.series_options(footprint)) === footprint_before,
        alignment: alignment(footprint).time_alignment,
      };

      // Ids: a worker chart has no public `remove_series`, so the removed id uses the private hook.
      // A patch that visibly changes whichever series an id would wrongly reach is used throughout.
      const removed = chart.add_series("line");
      chart.wasm.remove_series_tracked(removed);
      const changing = { time_alignment: "as_of" };
      const coerced = [Number.NaN, 1.5, 2 ** 32, -1];
      results.ids = {
        unknown: code_of(() => chart.apply_series_options(changing, 12_345)),
        removed: code_of(() => chart.apply_series_options(changing, removed)),
        removed_empty: code_of(() => chart.apply_series_options({}, removed)),
        unknown_read: code_of(() => chart.series_options(12_345)),
        removed_read: code_of(() => chart.series_options(removed)),
        coerced: coerced.map((bad) => code_of(() => chart.apply_series_options(changing, bad))),
        coerced_read: coerced.map((bad) => code_of(() => chart.series_options(bad))),
        alignments: [alignment(0).time_alignment, alignment(id).time_alignment],
      };

      // Disposal: the same handler, before the chart reference is dropped.
      if (timer !== null) clearInterval(timer);
      timer = null;
      chart.remove();
      results.disposed = {
        apply: code_of(() => chart.apply_series_options(changing, id)),
        read: code_of(() => chart.series_options(id)),
      };
      chart = null;
      postMessage({ type: "as_of_overlay_update", results });
      return;
    } else if (message.type === "custom_study_unsupported") {
      const errors = [];
      for (const method of ["register_custom_study", "add_custom_study"]) {
        try { chart[method](); } catch (error) { errors.push({ name: error.name, code: error.code }); }
      }
      postMessage({ type: "custom_study_unsupported", errors, series_ids: JSON.parse(chart.wasm.series_order_json()) });
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
