import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";
import { crop_png, count_different, max_channel_delta } from "./parity-pixels.mjs";

test("custom SMA matches built-in values and dispatches tail updates", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const calls = [];
    chart.register_custom_study({
      type: "test.sma", version: 1, title: "Custom SMA",
      parameters: [{ name: "period", parameter_type: "integer", default: 3, min: 1, max: 100 }],
      outputs: [{ name: "average", plot: "line", pane: "price" }],
      init: (params) => ({ period: params.period }),
      update: (state, ctx) => {
        calls.push(["update", ctx.from, ctx.length, ctx.tail,
          [ctx.time, ctx.open, ctx.high, ctx.low, ctx.close, ctx.volume]
            .every((column) => column instanceof Float64Array && column.length === ctx.length),
          ctx.outputs[0].length === ctx.length - ctx.from]);
        for (let i = ctx.from; i < ctx.length; i++) {
          let sum = 0;
          for (let j = i - state.period + 1; j <= i; j++) sum += ctx.close[j];
          ctx.outputs[0][i - ctx.from] = i >= state.period - 1 ? sum / state.period : NaN;
        }
      },
      rebuild: (state, ctx) => {
        calls.push(["rebuild", ctx.from, ctx.length, ctx.tail,
          [ctx.time, ctx.open, ctx.high, ctx.low, ctx.close, ctx.volume]
            .every((column) => column instanceof Float64Array && column.length === ctx.length),
          ctx.outputs[0].length === ctx.length - ctx.from]);
        for (let i = ctx.from; i < ctx.length; i++) {
          let sum = 0;
          for (let j = Math.max(0, i - state.period + 1); j <= i; j++) sum += ctx.close[j];
          ctx.outputs[0][i - ctx.from] = i >= state.period - 1 ? sum / state.period : NaN;
        }
      },
    });
    const source = chart.add_series("line");
    source.set_data(Array.from({ length: 8 }, (_, i) => ({ time: 1701000000 + 60 * i, value: i + 1 })));
    const custom = chart.add_custom_study("test.sma", source)[0];
    const builtin = chart.add_sma(source, 3);
    const snapshots = [custom.data(), builtin.data()];
    source.update({ time: 1701000480, value: 9 });
    source.update({ time: 1701000480, value: 12 });
    source.update({ time: 1701000180, value: 5 });
    return { calls, snapshots, after: [custom.data(), builtin.data()],
      info: { kind: custom.indicator_info().kind, params: custom.indicator_info().parameters.custom } };
  });
  expect(result.snapshots[0].filter(({ value }) => value !== undefined)).toEqual(result.snapshots[1]);
  expect(result.after[0].filter(({ value }) => value !== undefined)).toEqual(result.after[1]);
  expect(result.info).toEqual({ kind: "test.sma", params: { period: 3 } });
  expect(result.calls.some(([kind, , , tail]) => kind === "update" && tail)).toBe(true);
  expect(result.calls.some(([kind, , , tail]) => kind === "rebuild" && !tail)).toBe(true);
  expect(result.calls.every(([, , , , inputs, outputs]) => inputs && outputs)).toBe(true);
});

test("chained RSI, EMA and Bollinger respect custom warm-up and fault whitespace", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    let broken = false;
    chart.register_custom_study({
      type: "test.chained-whitespace", version: 1, title: "Chained",
      parameters: [], outputs: [{ name: "sma", plot: "line", pane: "price" }],
      init: () => null,
      rebuild: (_state, ctx) => {
        if (broken) throw new Error("source fault");
        for (let i = ctx.from; i < ctx.length; i++) {
          ctx.outputs[0][i - ctx.from] = i < 2 ? NaN
            : (ctx.close[i - 2] + ctx.close[i - 1] + ctx.close[i]) / 3;
        }
      },
    });
    const source = chart.add_series("line");
    const bars = Array.from({ length: 15 }, (_, i) => ({
      time: 1701000000 + i * 60, value: [10, 13, 12, 15, 17, 16, 19, 21, 18, 20, 22, 19, 23, 24, 25][i],
    }));
    source.set_data(bars);
    const custom = chart.add_custom_study("test.chained-whitespace", source)[0];
    const builtin = chart.add_sma(source, 3);
    const pairs = [
      [chart.add_rsi(custom, 5), chart.add_rsi(builtin, 5)],
      [chart.add_ema(custom, 4), chart.add_ema(builtin, 4)],
      [chart.add_bollinger(custom, 4, 2)[0], chart.add_bollinger(builtin, 4, 2)[0]],
    ];
    const snapshot = () => pairs.map(([actual, expected]) => [actual.data(), expected.data()]);
    const stages = [snapshot()];
    source.update({ time: 1701000900, value: 26 });
    stages.push(snapshot());
    source.update({ time: 1701000900, value: 27 });
    stages.push(snapshot());
    source.update({ time: 1701000420, value: 28 });
    stages.push(snapshot());
    broken = true;
    source.update({ time: 1701000960, value: 29 });
    return { stages, fault: pairs[0][0].data() };
  });
  for (const stage of result.stages) {
    for (const [actual, expected] of stage) expect(actual).toEqual(expected);
  }
  expect(result.fault.every(({ value }) => value === undefined)).toBe(true);
});

test("missing update rebuilds every tail and invalid outputs fault", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    source.set_data([{ time: 1701000000, value: 4 }]);
    const tails = [];
    const faults = [];
    chart.subscribe_custom_study_fault((event) => faults.push(event));
    chart.register_custom_study({
      type: "test.no-update", version: 1, title: "No update",
      parameters: [], outputs: [{ name: "value", plot: "line", pane: "price" }],
      init: () => null, rebuild: (_state, ctx) => {
        tails.push(ctx.tail);
        ctx.outputs[0].fill(tails.length === 2 ? Infinity : 5);
      },
    });
    const output = chart.add_custom_study("test.no-update", source)[0];
    source.update({ time: 1701000060, value: 5 });
    source.update({ time: 1701000120, value: 6 });
    return { tails, faults, values: output.data(), id: output.id };
  });
  expect(result.tails).toEqual([false, true]);
  expect(result.faults).toEqual([{ binding: result.id, message: "invalid custom study output" }]);
  expect(result.values.every(({ value }) => value === undefined)).toBe(true);
});

test("callback errors fault once; re-entrant chart calls return a typed error", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    source.set_data([{ time: 1701000000, value: 1 }, { time: 1701000060, value: 2 }]);
    const faults = [];
    chart.subscribe_custom_study_fault((event) => faults.push(event));
    let calls = 0;
    let reentrant = null;
    const reentrantCalls = [];
    chart.register_custom_study({
      type: "test.fault", version: 1, title: "Fault",
      parameters: [], outputs: [{ name: "signal", plot: "line", pane: "price" }],
      init: () => {
        for (const invoke of [() => chart.series_order(), () => chart.time_scale(), () => source.data()]) {
          try { invoke(); } catch (error) {
            reentrant = { name: error.name, code: error.code };
            reentrantCalls.push(error.code);
          }
        }
        return {};
      },
      rebuild: (_state, ctx) => {
        calls++;
        try { chart.backend(); } catch (error) { reentrant = { name: error.name, code: error.code }; }
        if (calls > 1) throw new Error("study failed");
        ctx.outputs[0].fill(7);
      },
    });
    const output = chart.add_custom_study("test.fault", source)[0];
    source.update({ time: 1701000120, value: 3 });
    source.update({ time: 1701000180, value: 4 });
    return { faults, calls, reentrant, reentrantCalls, values: output.data(), usable: chart.backend(), binding: output.id };
  });
  expect(result.reentrant).toEqual({ name: "AerisChartsError", code: "reentrant_call" });
  expect(result.reentrantCalls).toEqual(Array(3).fill("reentrant_call"));
  expect(result.faults).toEqual([{ binding: result.binding, message: "study failed" }]);
  expect(result.calls).toBe(2);
  expect(result.values.every(({ value }) => value === undefined)).toBe(true);
  expect(result.usable).toBe("canvas2d");
});

test("series pop delivers a custom study fault after the wasm call", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    source.set_data([{ time: 1701000000, value: 1 }, { time: 1701000060, value: 2 }]);
    const faults = [];
    chart.subscribe_custom_study_fault((event) => faults.push(event));
    chart.register_custom_study({
      type: "test.pop-fault", version: 1, title: "Pop",
      parameters: [], outputs: [{ name: "signal", plot: "line", pane: "price" }],
      init: () => null,
      rebuild: (_state, ctx) => {
        if (ctx.length === 1) throw new Error("pop failed");
        ctx.outputs[0].fill(1);
      },
    });
    const output = chart.add_custom_study("test.pop-fault", source)[0];
    source.pop();
    return { faults, binding: output.id, values: output.data() };
  });
  expect(result.faults).toEqual([{ binding: result.binding, message: "pop failed" }]);
  expect(result.values).toEqual([]);
});

test("ring drain delivers a custom study fault within the same frame", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    source.set_data([{ time: 4_000_000_000, value: 1 }]);
    const faults = [];
    chart.subscribe_custom_study_fault((event) => faults.push(event));
    chart.register_custom_study({
      type: "test.ring-fault", version: 1, title: "Ring",
      parameters: [], outputs: [{ name: "signal", plot: "line", pane: "price" }],
      init: () => null,
      rebuild: (_state, ctx) => {
        if (ctx.length > 1) throw new Error("ring failed");
        ctx.outputs[0].fill(1);
      },
    });
    const output = chart.add_custom_study("test.ring-fault", source)[0];
    const layout = {
      data_offset: 64, row_stride: 48, capacity: 8, time_offset: 0,
      open_offset: 8, high_offset: 16, low_offset: 24, close_offset: 32,
      sequence_offset: 40, write_cursor_offset: 0,
    };
    const buffer = new SharedArrayBuffer(64 + 48 * layout.capacity);
    source.set_ring_source(buffer, layout);
    const bytes = new DataView(buffer);
    for (const [offset, value] of [[0, 4_000_000_060], [8, 2], [16, 2], [24, 2], [32, 2]]) {
      bytes.setFloat64(64 + offset, value, true);
    }
    Atomics.store(new Int32Array(buffer), (64 + 40) / 4, 1);
    Atomics.store(new Int32Array(buffer), 0, 1);
    // Isolate the ring drain from repaint's separate wasm calls.
    const repaint = chart.repaint;
    Object.defineProperty(chart, "repaint", { configurable: true, value: () => {} });
    for (let i = 0; i < 10 && source.data().length < 2; i++) {
      await new Promise((resolve) => requestAnimationFrame(resolve));
    }
    // Clearing the ring is another wasm mutation, so freeze the observation first.
    const observed = { faults: [...faults], binding: output.id, sourceRows: source.data().length, values: output.data() };
    Object.defineProperty(chart, "repaint", { configurable: true, value: repaint });
    source.set_ring_source(null);
    return observed;
  });
  expect(result.sourceRows).toBe(2);
  expect(result.faults).toEqual([{ binding: result.binding, message: "ring failed" }]);
  expect(result.values.every(({ value }) => value === undefined)).toBe(true);
});

test("configuring synthetic bars delivers faults before the next chart call", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick");
    const options = { kind: "renko_fixed", box_size: 1 };
    chart.configure_synthetic_bar_series(source, options);
    chart.set_synthetic_bar_source(source, [
      { time: 1701000000, open: 100, high: 100, low: 100, close: 100 },
      { time: 1701000060, open: 102, high: 102, low: 102, close: 102 },
    ]);
    const faults = [];
    chart.subscribe_custom_study_fault((event) => faults.push(event));
    chart.register_custom_study({
      type: "test.synthetic-fault", version: 1, title: "Synthetic",
      parameters: [], outputs: [{ name: "signal", plot: "line", pane: "price" }],
      init: () => null,
      rebuild: (_state, ctx) => {
        if (ctx.length === 0) throw new Error("synthetic reconfiguration failed");
        ctx.outputs[0].fill(1);
      },
    });
    const output = chart.add_custom_study("test.synthetic-fault", source)[0];
    const before = [...faults];
    // Isolate the configuring wasm call from repaint's other wasm calls.
    const repaint = chart.repaint;
    Object.defineProperty(chart, "repaint", { configurable: true, value: () => {} });
    chart.configure_synthetic_bar_series(source, options);
    Object.defineProperty(chart, "repaint", { configurable: true, value: repaint });
    // Do not call another wasm method (including output.data()) before snapshotting.
    return { before, faults: [...faults], binding: output.id };
  });
  expect(result.before).toEqual([]);
  expect(result.faults).toEqual([{ binding: result.binding, message: "synthetic reconfiguration failed" }]);
});

test("definitions, parameters, and worker API reject invalid input", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const source = chart.add_series("line");
    const errors = [];
    const definition = {
      type: "test.valid", version: 1, title: "Valid",
      parameters: [{ name: "period", parameter_type: "integer", default: 3, min: 1, max: 10 }],
      outputs: [{ name: "value", plot: "line", pane: "price" }],
      init: () => ({}), rebuild: (_state, ctx) => ctx.outputs[0].fill(1),
    };
    for (const patch of [{ init: undefined }, { rebuild: undefined }, { type: "Bad" }, { outputs: [] },
      { outputs: Array(6).fill(definition.outputs[0]) }]) {
      try { chart.register_custom_study({ ...definition, ...patch }); } catch (error) { errors.push(error.code); }
    }
    chart.register_custom_study(definition);
    for (const params of [{ period: 0 }, { period: 2.5 }, { missing: 2 }]) {
      try { chart.add_custom_study("test.valid", source, params); } catch (error) { errors.push(error.code); }
    }
    const { offscreen_chart } = await import("/dist/aeris_charts_financial.js");
    // Unsupported methods do not access worker state or mutate its chart.
    const worker = Object.create(offscreen_chart.prototype);
    for (const method of ["register_custom_study", "add_custom_study"]) {
      try { worker[method](definition, source); } catch (error) { errors.push(error.code); }
    }
    return { errors, count: chart.series_order().length };
  });
  expect(result.errors).toEqual([...Array(8).fill("invalid_options"), "unsupported", "unsupported"]);
  expect(result.count).toBeGreaterThan(0);
});

test("live offscreen worker rejects custom studies without mutating its chart", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  const result = await page.evaluate(async () => {
    const worker = new Worker("/offscreen_chart_worker.js", { type: "module" });
    const gpu = new OffscreenCanvas(480, 320);
    const fallback = new OffscreenCanvas(480, 320);
    const receive = () => new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error("worker did not respond")), 15_000);
      worker.addEventListener("message", function onMessage(event) {
        worker.removeEventListener("message", onMessage);
        clearTimeout(timeout);
        resolve(event.data);
      });
    });
    try {
      worker.postMessage({
        type: "init", gpu_canvas: gpu, fallback_canvas: fallback,
        width: 480, height: 320, dpr: 1, bars: 4, backend: "canvas2d",
      }, [gpu, fallback]);
      const ready = await receive();
      if (ready.type !== "ready") throw new Error(JSON.stringify(ready));
      worker.postMessage({ type: "custom_study_unsupported" });
      const response = await receive();
      worker.postMessage({ type: "remove" });
      await receive();
      return { response, before: ready.series_ids };
    } finally {
      worker.terminate();
    }
  });
  expect(result.response.errors).toEqual(Array(2).fill({ name: "AerisChartsError", code: "unsupported" }));
  expect(result.response.series_ids).toEqual(result.before);
});

test("V3 import reports pending custom studies and activates after registration", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const original = window.__chart;
    const definition = {
      type: "test.persist", version: 1, title: "Persistent",
      parameters: [], outputs: [{ name: "value", plot: "line", pane: "price" }],
      init: () => ({}), rebuild: (_state, ctx) => {
        for (let i = ctx.from; i < ctx.length; i++) ctx.outputs[0][i - ctx.from] = ctx.close[i] * 2;
      },
    };
    const source = original.add_series("line");
    source.set_data([{ time: 1701000000, value: 2 }, { time: 1701000060, value: 3 }]);
    original.register_custom_study(definition);
    const output = original.add_custom_study("test.persist", source)[0];
    const state = original.export_state();
    const container = document.createElement("div");
    container.style.cssText = "width:600px;height:400px";
    document.body.append(container);
    const restored = await create_chart(container, { backend: "canvas2d" });
    // The V3 document carries bindings, not market data; recreate the same source identity.
    restored.add_series("line");
    const restoredSource = restored.add_series("line");
    restoredSource.set_data(source.data());
    const pending = restored.import_state(state);
    const before = restored.series_order().find((series) => series.indicator_info()?.kind === "test.persist");
    const pendingValues = before?.data();
    const roundTrip = restored.export_state();
    restored.register_custom_study(definition);
    const after = restored.series_order().find((series) => series.indicator_info()?.kind === "test.persist");
    const values = after?.data();
    const order = restored.series_order().map((series) => ({ id: series.id, kind: series.indicator_info()?.kind }));
    restored.remove();
    container.remove();
    return { schema: state.schema_version, pending, pendingValues, order, sameDefinition: JSON.stringify(roundTrip.indicators) === JSON.stringify(state.indicators), values, original: output.data() };
  });
  expect(result.schema).toBe(3);
  expect(result.pending.unresolved_custom_studies).toHaveLength(1);
  expect(result.pendingValues).toBeDefined();
  expect(result.pendingValues.every(({ value }) => value === undefined)).toBe(true);
  expect(result.sameDefinition).toBe(true);
  expect(result.values).toEqual(result.original);
});

test("I2, I3 and custom studies retain definitions and values in one layout", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const source = chart.add_series("candlestick");
    const rows = Array.from({ length: 40 }, (_, i) => ({
      time: 1701100000 + i * 60,
      open: i === 3 ? 116 : 100 + i,
      high: i === 3 ? 118 : 103 + i,
      low: i === 3 ? 114 : 99 + i,
      close: i === 3 ? 117 : 101 + i,
    }));
    source.set_data(rows);
    const definition = {
      type: "test.cross", version: 1, title: "Cross",
      parameters: [], outputs: [{ name: "doubled", plot: "line", pane: "price" }],
      init: () => ({}), rebuild: (_state, ctx) => {
        for (let i = ctx.from; i < ctx.length; i++) ctx.outputs[0][i - ctx.from] = ctx.close[i] * 2;
      },
    };
    chart.register_custom_study(definition);
    const i2 = chart.add_kama(source, 10, 2, 30);
    const i3 = chart.add_swing_points(source, 2, 2);
    const zones = chart.add_fair_value_gaps(source, { mitigation: "half", show_mitigated: true });
    const custom = chart.add_custom_study("test.cross", source)[0];
    const state = chart.export_state();
    const host = document.createElement("div");
    host.style.cssText = "width:600px;height:400px";
    document.body.append(host);
    const restored = await create_chart(host, { backend: "canvas2d" });
    restored.add_series("line");
    const restoredSource = restored.add_series("candlestick");
    restoredSource.set_data(rows);
    restored.register_custom_study(definition);
    const imported = restored.import_state(state);
    const outputs = restored.series_order();
    const kinds = outputs.map((entry) => entry.indicator_info()?.kind).filter(Boolean);
    const restoredKama = outputs.find((entry) => entry.indicator_info()?.kind === "kama");
    const restoredSwing = outputs.find((entry) => entry.indicator_info()?.kind === "swing_points");
    const restoredZones = outputs.find((entry) => entry.indicator_info()?.kind === "fair_value_gaps");
    const restoredCustom = outputs.find((entry) => entry.indicator_info()?.kind === "test.cross");
    const comparison = {
      kama: [i2.data(), restoredKama?.data()],
      swing: [i3[0].data(), restoredSwing?.data()],
      custom: [custom.data(), restoredCustom?.data()],
      markers: [chart.study_annotations(i3[0]).markers, restored.study_annotations(restoredSwing).markers],
      zones: [chart.study_annotations(zones).zones, restored.study_annotations(restoredZones).zones],
      styles: [[i2, i3[0], zones, custom].map((entry) => entry.indicator_info().style),
        [restoredKama, restoredSwing, restoredZones, restoredCustom].map((entry) => entry.indicator_info().style)],
      params: [[i2, i3[0], zones, custom].map((entry) => entry.indicator_info().parameters),
        [restoredKama, restoredSwing, restoredZones, restoredCustom].map((entry) => entry.indicator_info().parameters)],
    };
    restored.remove();
    host.remove();
    return { schema: state.schema_version, imported, kinds, comparison };
  });
  expect(result.schema).toBe(3);
  expect(result.imported.unresolved_custom_studies).toEqual([]);
  expect(result.kinds).toContain("kama");
  expect(result.kinds).toContain("swing_points");
  expect(result.kinds).toContain("fair_value_gaps");
  expect(result.kinds).toContain("test.cross");
  expect(result.comparison.zones[0].length).toBeGreaterThan(0);
  expect(result.comparison.params[0][2].mitigation).toBe("half");
  for (const [left, right] of Object.values(result.comparison)) expect(right).toEqual(left);
});

test("structure zones and auction marks share Canvas2D and WebGPU geometry", async ({ browser }) => {
  const snapshots = [];
  for (const backend of ["canvas2d", "webgpu"]) {
    const page = await browser.newPage({ viewport: { width: 1100, height: 720 }, deviceScaleFactor: 1 });
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await page.waitForFunction((expected) => window.__chart?.backend?.() === expected, backend);
    const geometry = await page.evaluate(() => {
      const chart = window.__chart;
      chart.remove_series(window.__main);
      const source = chart.add_series("candlestick");
      const base = Math.floor(window.__data[0].time / 60) * 60;
      const rows = [
        [9, 10, 8, 9], [12, 14, 9, 13], [12, 12, 10, 11],
        [16, 18, 15, 17], [16, 16, 15, 15], [18, 20, 8, 19],
        [19, 21, 16, 20],
      ].map(([open, high, low, close], i) => ({
        time: base + i * 60, open, high, low, close,
      }));
      source.set_data(rows);
      const swings = chart.add_swing_points(source, 1, 1);
      const market = chart.add_market_structure(source, 1, 1);
      const gaps = chart.add_fair_value_gaps(source, { show_mitigated: true });
      const blocks = chart.add_order_blocks(source, { left: 1, right: 1, show_mitigated: true });
      chart.register_custom_study({
        type: "test.parity-marker", version: 1, title: "Marker",
        parameters: [],
        outputs: [{ name: "event", plot: "marker", pane: "price",
          default_style: { line_color: "#ff00ff", point_markers: true } }],
        init: () => null,
        rebuild: (_state, ctx) => {
          for (let i = ctx.from; i < ctx.length; i++) {
            ctx.outputs[0][i - ctx.from] = i === 3 || i === 5 ? ctx.close[i] : NaN;
          }
        },
      });
      const customMarker = chart.add_custom_study("test.parity-marker", source)[0];
      const footprint = chart.add_series("footprint", { tick_size: 1, interval_seconds: 60 });
      const stream = chart.add_trade_stream("test:parity", { tick_size: 1, interval_seconds: 60 });
      chart.bind_footprint_series_to_stream(footprint, stream);
      chart.set_trade_stream_trades(stream, [
        { timestamp_micros: base * 1_000_000 + 1, price: 10, volume: 100, aggressor: "sell", session_id: 1 },
        { timestamp_micros: base * 1_000_000 + 2, price: 10, volume: 25, aggressor: "buy", session_id: 1 },
        { timestamp_micros: (base + 60) * 1_000_000 + 1, price: 11, volume: 12, aggressor: "buy", session_id: 1 },
      ]);
      const auction = chart.add_auction_markers(footprint, stream);
      chart.time_scale().fit_content();
      chart.time_scale().apply_options({ bar_spacing: 38 });
      return {
        swing: chart.study_annotations(swings[0]),
        market: chart.study_annotations(market),
        gaps: chart.study_annotations(gaps),
        blocks: chart.study_annotations(blocks),
        customMarkerValues: customMarker.data().filter(({ value }) => value !== undefined),
        marks: auction.snapshot(),
      };
    });
    await page.evaluate(() => document.fonts.ready);
    snapshots.push({ backend, geometry, image: PNG.sync.read(await page.screenshot()) });
    await page.close();
  }
  expect(snapshots[0].geometry).toEqual(snapshots[1].geometry);
  expect(snapshots[0].geometry.gaps.zones.length).toBeGreaterThan(0);
  expect(snapshots[0].geometry.blocks.zones.length).toBeGreaterThan(0);
  expect(snapshots[0].geometry.market.markers.length).toBeGreaterThan(0);
  expect(snapshots[0].geometry.swing.markers.length).toBeGreaterThan(0);
  expect(snapshots[0].geometry.customMarkerValues).toHaveLength(2);
  expect(snapshots[0].geometry.marks.length).toBeGreaterThan(0);
  const [canvas, gpu] = snapshots.map(({ image }) => image);
  const chart_canvas = crop_png(canvas, 75, 65, 665, 655);
  const chart_gpu = crop_png(gpu, 75, 65, 665, 655);
  const data_a = chart_canvas.data;
  const data_b = chart_gpu.data;
  let rounding_pixels = 0;
  let coverage_pixels = 0;
  let coverage_max = 0;
  let interior_max = 0;
  for (let y = 0; y < chart_canvas.height; y++) {
    for (let x = 0; x < chart_canvas.width; x++) {
      const i = 4 * (y * chart_canvas.width + x);
      let delta = 0;
      for (let c = 0; c < 4; c++) delta = Math.max(delta, Math.abs(data_a[i + c] - data_b[i + c]));
      if (delta === 0) continue;
      if (delta <= 2) {
        rounding_pixels++;
        interior_max = Math.max(interior_max, delta);
        continue;
      }
      coverage_pixels++;
      coverage_max = Math.max(coverage_max, delta);
      // A difference larger than the two-channel-unit fill/blend allowance must be
      // adjacent to an actual contour or glyph in at least one backend. This catches
      // a shifted or differently blended *interior* even when the residual count fits.
      let local_contrast = 0;
      for (let dy = -2; dy <= 2; dy++) {
        for (let dx = -2; dx <= 2; dx++) {
          const nx = x + dx;
          const ny = y + dy;
          if (nx < 0 || ny < 0 || nx >= chart_canvas.width || ny >= chart_canvas.height) continue;
          const neighbor = 4 * (ny * chart_canvas.width + nx);
          for (let c = 0; c < 3; c++) {
            local_contrast = Math.max(local_contrast,
              Math.abs(data_a[i + c] - data_a[neighbor + c]),
              Math.abs(data_b[i + c] - data_b[neighbor + c]));
          }
        }
      }
      expect(local_contrast, `non-edge fill mismatch at chart pixel (${x}, ${y})`).toBeGreaterThanOrEqual(delta);
    }
  }
  expect(rounding_pixels, "zone, overlapping fill and body rounding must be exercised").toBeGreaterThan(0);
  expect(coverage_pixels, "primitive AA and text must be exercised").toBeGreaterThan(0);
  expect(count_different(chart_canvas, chart_gpu)).toBe(rounding_pixels + coverage_pixels);
  expect(interior_max, "all fill and alpha-blend interiors stay within two channel units").toBeLessThanOrEqual(2);
  // Windows SwiftShader, 1100x720 DPR 1: 4,342 strict pixels: 4,060 are 1-2 unit
  // rect/zone/overlap rounding (4,032 at 1, 28 at 2); 282 are localized contour/glyph
  // coverage, max 84 on triangle/label edges. The 2,600/40 baseline from
  // primitives.spec.mjs covers a different scene without this many translucent rects.
  // backend-parity.spec.mjs also accepts marker AA up to 66 when paint order is exact.
  // Require the actual non-rounding residual with ~13% count and 6-unit edge margin,
  // rather than hiding a real fill/geometry divergence behind pixelmatch threshold 0.1.
  expect(coverage_pixels, "whole-chart AA/text residual").toBeLessThanOrEqual(320);
  expect(coverage_max, "whole-chart AA/text maximum channel delta").toBeLessThanOrEqual(90);
  expect(max_channel_delta(chart_canvas, chart_gpu)).toBe(coverage_max);
});
