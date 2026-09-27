import { test, expect } from "@playwright/test";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// Real-time ingestion through the public TS API: out-of-order corrections of historical bars,
// engine-owned partial merges, the optional per-series sequence guard, diagnostics for reference
// `update` payloads that silently rewrite a bar, ring drop accounting, and React streaming.

const T0 = 1_577_836_800;

async function wait_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/");
  await wait_chart(page);
  await page.evaluate(define_install_bars, T0);
});

/** Page helper: a hidden candlestick series with `count` one-minute bars and its scope log. */
function define_install_bars(t0) {
  window.__install_bars = (count) => {
    const bars = [];
    for (let i = 0; i < count; i += 1) {
      bars.push({ time: t0 + i * 60, open: 100 + i, high: 101 + i, low: 99 + i, close: 100.5 + i });
    }
    const series = window.__chart.add_series("candlestick", { visible: false });
    series.set_data(bars);
    const scopes = [];
    series.subscribe_data_changed((scope) => scopes.push(scope));
    return { series, bars, scopes };
  };
}

test("a single out-of-order update corrects bar N-2 in place", async ({ page }) => {
  const result = await page.evaluate(() => {
    const install = window.__install_bars;
    const { series, bars, scopes } = install(500);
    const n2 = bars[bars.length - 3];
    series.update({ time: n2.time, open: 1, high: 900, low: 0.5, close: 450 });
    const data = series.data();
    const single = {
      length: data.length,
      corrected: data[data.length - 3],
      last: data[data.length - 1],
      neighbor: data[data.length - 4],
      scopes: [...scopes],
      diagnostics: series.last_ingestion_diagnostics(),
    };
    // The usual feed batch: a late N-1 correction, the current bar, and a new bar in one call.
    const last = bars[bars.length - 1].time;
    const column = (values) => new Float64Array(values);
    series.update_typed({
      times: column([last - 60, last, last + 60]),
      open: column([2, 3, 4]),
      high: column([20, 30, 40]),
      low: column([1, 2, 3]),
      close: column([10, 11, 12]),
    });
    const after = series.data();
    return {
      single,
      batch: {
        length: after.length,
        corrected: after[498],
        replaced: after[499],
        appended: after[500],
        untouched: after[497],
        scopes: [...scopes],
      },
    };
  });

  expect(result.single.length).toBe(500);
  expect(result.single.corrected).toEqual({ time: T0 + 497 * 60, open: 1, high: 900, low: 0.5, close: 450 });
  expect(result.single.last).toEqual({ time: T0 + 499 * 60, open: 599, high: 600, low: 598, close: 599.5 });
  expect(result.single.neighbor.close).toBe(596.5);
  expect(result.single.scopes).toEqual(["update"]);
  expect(result.single.diagnostics).toBeNull();
  expect(result.batch.length).toBe(501);
  expect(result.batch.corrected).toEqual({ time: T0 + 498 * 60, open: 2, high: 20, low: 1, close: 10 });
  expect(result.batch.replaced).toEqual({ time: T0 + 499 * 60, open: 3, high: 30, low: 2, close: 11 });
  expect(result.batch.appended).toEqual({ time: T0 + 500 * 60, open: 4, high: 40, low: 3, close: 12 });
  expect(result.batch.untouched).toEqual({ time: T0 + 497 * 60, open: 1, high: 900, low: 0.5, close: 450 });
  expect(result.batch.scopes).toEqual(["update", "update"]);
});

test("merge applies partial fields and normalizes the bar envelope", async ({ page }) => {
  const result = await page.evaluate(() => {
    const install = window.__install_bars;
    const { series, bars, scopes } = install(20);
    const last = bars[bars.length - 1];
    const snapshot = (index) => series.data()[index];

    series.merge({ time: last.time, close: 200 });
    const close_above = snapshot(19);
    series.merge({ time: last.time, value: 50 });
    const value_below = snapshot(19);
    series.merge({ time: bars[5].time, high: 1 });
    const history = snapshot(5);
    series.merge({ time: last.time + 60, close: 77 });
    const new_bar = snapshot(20);

    const volume = window.__chart.add_series("histogram", { visible: false });
    volume.set_data(bars.map((bar) => ({ time: bar.time, value: 10 })));
    volume.merge({ time: last.time, value: 1234 });
    const volume_value = volume.data()[19];

    const warnings = [];
    const original_warn = console.warn;
    console.warn = (...args) => { warnings.push(args.join(" ")); };
    let empty;
    try {
      series.merge({ time: last.time, volume: 5 });
      empty = series.last_ingestion_diagnostics();
    } finally {
      console.warn = original_warn;
    }
    let custom_error = null;
    try {
      window.__chart.add_series("heatmap", { visible: false }).merge({ time: last.time, close: 1 });
    } catch (error) {
      custom_error = error.code ?? String(error);
    }
    return {
      close_above,
      value_below,
      history,
      new_bar,
      volume_value,
      empty,
      warnings,
      scopes,
      length: series.data().length,
      custom_error,
    };
  });

  const last_time = T0 + 19 * 60;
  expect(result.close_above).toEqual({ time: last_time, open: 119, high: 200, low: 118, close: 200 });
  expect(result.value_below).toEqual({ time: last_time, open: 119, high: 200, low: 50, close: 50 });
  // An explicit high below the body is normalized up to the body.
  expect(result.history).toEqual({ time: T0 + 5 * 60, open: 105, high: 105.5, low: 104, close: 105.5 });
  expect(result.new_bar).toEqual({ time: last_time + 60, open: 77, high: 77, low: 77, close: 77 });
  expect(result.volume_value).toEqual({ time: last_time, value: 1234 });
  expect(result.empty).toMatchObject({ status: "rejected", code: "empty_merge" });
  expect(result.warnings.some((line) => line.includes("merge rejected"))).toBe(true);
  expect(result.scopes).toEqual(["update", "update", "update", "update"]);
  expect(result.length).toBe(21);
  expect(result.custom_error).toBe("unsupported_operation");
});

test("merge_typed merges partial columns in order and rejects the batch atomically", async ({ page }) => {
  const result = await page.evaluate(() => {
    const { series, bars, scopes } = window.__install_bars(10);
    const last = bars[9].time;
    const next = last + 60;
    const column = (values) => new Float64Array(values);
    series.merge_typed({
      times: column([bars[2].time, last, last, next, next]),
      high: column([NaN, NaN, NaN, NaN, 125]),
      close: column([50, 300, 90, 120, NaN]),
    }, { sequence: 5 });
    const data = series.data();
    const merged = { n2: data[2], last: data[9], next: data[10], length: data.length, scopes: [...scopes] };

    const before = JSON.stringify(series.data());
    const warnings = [];
    const original_warn = console.warn;
    console.warn = (...args) => { warnings.push(args.join(" ")); };
    const out = { merged };
    try {
      series.merge_typed({ times: column([last, next]), close: column([1, Infinity]) }, { sequence: 6 });
      out.invalid = series.last_ingestion_diagnostics();
      series.merge_typed({ times: column([next]), close: column([1]) }, { sequence: 5 });
      out.stale = series.last_ingestion_diagnostics();
      series.merge_typed({ times: column([next]), close: column([NaN]) });
      out.empty = series.last_ingestion_diagnostics();
      series.merge_typed({ times: column([next, last]), close: column([1]) });
      out.mismatch = series.last_ingestion_diagnostics();
    } finally {
      console.warn = original_warn;
    }
    out.unchanged = JSON.stringify(series.data()) === before;
    out.scopes = [...scopes];
    out.warnings = warnings;

    const heatmap = window.__chart.add_series("heatmap", { visible: false });
    for (const [name, call] of [
      ["merge_typed", () => heatmap.merge_typed({ times: column([last]), close: column([1]) })],
      ["update", () => heatmap.update({ time: last, value: 1 }, { sequence: 1 })],
      ["set_data", () => heatmap.set_data([], { sequence: 1 })],
    ]) {
      try {
        call();
        out[`heatmap_${name}`] = "no error";
      } catch (error) {
        out[`heatmap_${name}`] = error.code ?? String(error);
      }
    }
    return out;
  });

  expect(result.merged.n2).toEqual({ time: T0 + 2 * 60, open: 102, high: 103, low: 50, close: 50 });
  expect(result.merged.last).toEqual({ time: T0 + 9 * 60, open: 109, high: 300, low: 90, close: 90 });
  expect(result.merged.next).toEqual({ time: T0 + 10 * 60, open: 120, high: 125, low: 120, close: 120 });
  expect(result.merged.length).toBe(11);
  expect(result.merged.scopes).toEqual(["update"]);
  expect(result.invalid).toMatchObject({ status: "rejected" });
  // The rejected batch did not consume sequence 6, and 5 is stale against the applied 5.
  expect(result.stale).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 5 });
  expect(result.empty).toMatchObject({ status: "rejected", code: "empty_merge" });
  expect(result.mismatch).toMatchObject({ status: "rejected" });
  expect(result.mismatch.reason).toContain("columns");
  expect(result.unchanged).toBe(true);
  expect(result.scopes).toEqual(["update"]);
  // Every non-stale rejection warns; the stale delivery stays quiet.
  expect(result.warnings.filter((line) => line.includes("merge_typed rejected"))).toHaveLength(3);
  expect(result.heatmap_merge_typed).toBe("unsupported_operation");
  expect(result.heatmap_update).toBe("unsupported_operation");
  expect(result.heatmap_set_data).toBe("unsupported_operation");
});

test("a streamed colored bar renders exactly like the same item in a full install", async ({ page }) => {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_chart(page);
  const settle = () => page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  const capture = async () => {
    await settle();
    return PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  };
  const colors = { color: "#2962ff", wick_color: "#ff9800", border_color: "#e91e63" };
  const different = (a, b) => {
    expect([a.width, a.height]).toEqual([b.width, b.height]);
    return pixelmatch(a.data, b.data, null, a.width, a.height, { threshold: 0, includeAA: true });
  };

  // Reference: the colored last bar arrives in a full data install.
  await page.evaluate((style) => {
    const data = window.__data;
    window.__main.set_data(data.map((bar, index) => (index === data.length - 1 ? { ...bar, ...style } : bar)));
  }, colors);
  const installed = await capture();

  await page.evaluate(() => window.__main.set_data(window.__data));
  const plain = await capture();
  expect(different(installed, plain), "the colors must be visible").toBeGreaterThan(0);

  // The same colors streamed onto a series that had no color channel yet.
  await page.evaluate((style) => {
    const data = window.__data;
    window.__main.update({ ...data[data.length - 1], ...style });
  }, colors);
  expect(different(installed, await capture())).toBe(0);

  await page.evaluate((style) => {
    const data = window.__data;
    const last = data[data.length - 1];
    window.__main.set_data(data);
    window.__main.merge({ time: last.time, close: last.close, ...style });
  }, colors);
  expect(different(installed, await capture())).toBe(0);
});

test("stale sequences are rejected without mutating or notifying", async ({ page }) => {
  const result = await page.evaluate(() => {
    const install = window.__install_bars;
    const { series, bars, scopes } = install(10);
    const last = bars[bars.length - 1];
    const bar = (close) => ({ time: last.time, open: 1, high: Math.max(2, close), low: 0.5, close });
    const columns = (time, close) => ({
      times: new Float64Array([time]),
      open: new Float64Array([close]),
      high: new Float64Array([close]),
      low: new Float64Array([close]),
      close: new Float64Array([close]),
    });
    const out = {};
    series.update(bar(5), { sequence: 7 });
    out.applied = series.data()[9].close;
    series.update(bar(6), { sequence: 7 });
    out.equal = series.last_ingestion_diagnostics();
    series.merge({ time: last.time, close: 8 }, { sequence: 3 });
    out.older_merge = series.last_ingestion_diagnostics();
    series.update_typed(columns(last.time + 60, 9), { sequence: 6 });
    out.stale_batch = series.last_ingestion_diagnostics();
    out.after_stale = { close: series.data()[9].close, length: series.data().length, scopes: [...scopes] };

    series.update(bar(10));
    out.unsequenced = series.data()[9].close;
    series.merge({ time: last.time, close: 11 }, { sequence: 8 });
    series.update_typed(columns(last.time + 60, 12), { sequence: 9 });
    out.advanced = { close: series.data()[9].close, appended: series.data()[10]?.close };

    series.update(bar(13), { sequence: -1 });
    out.invalid = series.last_ingestion_diagnostics();

    // A full replace is a resync: it clears the guard, or installs the snapshot's sequence.
    series.set_data(bars);
    series.update(bar(14), { sequence: 1 });
    out.after_reset = series.data()[9].close;
    series.set_data(bars, { sequence: 100 });
    series.update(bar(15), { sequence: 50 });
    out.after_baseline = series.last_ingestion_diagnostics();
    series.update(bar(16), { sequence: 101 });
    out.after_newer = series.data()[9].close;
    return out;
  });

  expect(result.applied).toBe(5);
  expect(result.equal).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 7 });
  expect(result.older_merge).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 7 });
  expect(result.stale_batch).toMatchObject({ status: "rejected", code: "stale_sequence", last_sequence: 7 });
  expect(result.after_stale).toEqual({ close: 5, length: 10, scopes: ["update"] });
  expect(result.unsequenced).toBe(10);
  expect(result.advanced).toEqual({ close: 11, appended: 12 });
  expect(result.invalid).toMatchObject({ status: "rejected" });
  expect(result.invalid.reason).toContain("sequence");
  expect(result.after_reset).toBe(14);
  expect(result.after_baseline).toMatchObject({ code: "stale_sequence", last_sequence: 100 });
  expect(result.after_newer).toBe(16);
});

test("reference update payloads that rewrite a bar report diagnostics pointing to merge()", async ({ page }) => {
  const result = await page.evaluate(() => {
    const install = window.__install_bars;
    const { series, bars } = install(10);
    const last = bars[bars.length - 1];
    const warnings = [];
    const original_warn = console.warn;
    console.warn = (...args) => { warnings.push(args.join(" ")); };
    const out = {};
    try {
      series.update({ time: last.time, value: 42 });
      out.value = { diagnostics: series.last_ingestion_diagnostics(), bar: series.data()[9] };
      series.update({ time: last.time, value: 43 });
      series.update({ time: last.time, close: 44 });
      out.partial = { diagnostics: series.last_ingestion_diagnostics(), bar: series.data()[9] };
      series.update({ time: last.time, volume: 1000 });
      out.price_less = { diagnostics: series.last_ingestion_diagnostics(), length: series.data().length };
      series.update({ time: last.time + 60, open: 1, high: 2, low: 0.5, close: 1.5 });
      out.clean = series.last_ingestion_diagnostics();
      const line = window.__chart.add_series("line", { visible: false });
      line.update({ time: last.time, value: 7 });
      out.line = line.last_ingestion_diagnostics();
    } finally {
      console.warn = original_warn;
    }
    out.warnings = warnings;
    return out;
  });

  expect(result.value.diagnostics).toMatchObject({ status: "accepted_with_diagnostics", code: "value_on_ohlc_series" });
  expect(result.value.diagnostics.reason).toContain("merge(");
  // Reference behavior is preserved: the bar is flattened.
  expect(result.value.bar).toEqual({ time: T0 + 9 * 60, open: 42, high: 42, low: 42, close: 42 });
  expect(result.partial.diagnostics).toMatchObject({ status: "rejected", code: "partial_ohlc" });
  expect(result.partial.bar.close).toBe(43);
  expect(result.price_less.diagnostics).toMatchObject({ status: "accepted_with_diagnostics", code: "price_less_payload" });
  expect(result.price_less.diagnostics.reason).toContain("volume");
  expect(result.clean).toBeNull();
  expect(result.line).toBeNull();
  // One hint per code per handle, and every dropped point warns.
  expect(result.warnings.filter((line) => line.includes("flat O=H=L=C"))).toHaveLength(1);
  expect(result.warnings.filter((line) => line.includes("update rejected"))).toHaveLength(1);
  expect(result.warnings.filter((line) => line.includes("without price fields"))).toHaveLength(1);
});

test("ring rows with invalid values are dropped and counted in frame stats", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const layout = {
      data_offset: 64,
      row_stride: 40,
      capacity: 16,
      time_offset: 0,
      open_offset: 8,
      high_offset: 16,
      low_offset: 24,
      close_offset: 32,
      write_cursor_offset: 0,
    };
    const buffer = new SharedArrayBuffer(layout.data_offset + layout.row_stride * layout.capacity);
    const cursor = new Int32Array(buffer);
    const rows = new DataView(buffer);
    const series = window.__chart.add_series("line", { visible: false });
    series.set_ring_source(buffer, layout);
    const before = window.__chart.frame_stats().ring_dropped_rows;
    const write = (slot, time, value) => {
      const base = layout.data_offset + slot * layout.row_stride;
      rows.setFloat64(base, time, true);
      for (const offset of [8, 16, 24, 32]) rows.setFloat64(base + offset, value, true);
    };
    write(0, 4_000_000_000, 10);
    write(1, 4_000_000_060, Number.POSITIVE_INFINITY);
    write(2, 4_000_000_120.5, 12);
    write(3, 4_000_000_180, 13);
    Atomics.store(cursor, 0, 4);
    await new Promise((resolve) => {
      let frames = 6;
      const tick = () => (frames-- <= 0 ? resolve() : requestAnimationFrame(tick));
      tick();
    });
    const stats = window.__chart.frame_stats();
    const data = series.data();
    series.set_ring_source(null);
    return { before, dropped: stats.ring_dropped_rows, overruns: stats.ring_overruns, values: data.map((point) => point.value) };
  });

  expect(result.before).toBe(0);
  expect(result.dropped).toBe(2);
  expect(result.overruns).toBe(0);
  expect(result.values).toEqual([10, 13]);
});

test("React FinancialSeries streams tail changes through update() and replaces otherwise", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  const result = await page.evaluate(async () => {
    const fixture = await import("/dist/react_phase3_fixture.js");
    return fixture.exerciseReactStreaming();
  });
  expect(result).toEqual({
    initial_scopes: ["full"],
    replace_scopes: ["update"],
    append_scopes: ["update"],
    mixed_scopes: ["update", "update", "update"],
    recreated_scopes: ["update"],
    history_scopes: ["full"],
    removal_scopes: ["full"],
    in_place_scopes: ["update", "update"],
    final_matches: true,
  });
});
