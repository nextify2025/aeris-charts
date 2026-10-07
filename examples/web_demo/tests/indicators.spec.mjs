import { expect } from "@playwright/test";
import { PNG } from "pngjs";
import { test, wait_for_chart } from "./page-ready.mjs";

// Engine-native indicators: Bollinger band fill, oscillator separate panes with channel strips,
// MACD four-state histogram colors, and the full native set's placement/lineage.

async function wait_grid(page) {
  await wait_for_chart(page, { grid: true });
}

test("page readiness reports a failed static import instead of timing out", async ({ page }) => {
  await page.route("**/fixture_features.js", (route) => route.abort("internetdisconnected"));
  await page.goto("/");
  await expect(wait_grid(page)).rejects.toThrow(/fixture_features\.js.*ERR_INTERNET_DISCONNECTED/);
});

test("page readiness reports a module error instead of timing out", async ({ page }) => {
  await page.route("**/fixture_features.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: "export const marker_fixture = null; export const timeline_mark_fixture = null; export const volume_fixture = null; throw new Error('fixture module crashed')" }));
  await page.goto("/");
  await expect(wait_grid(page)).rejects.toThrow(/fixture module crashed/);
});

function count_color(png, target, tol = 10) {
  let n = 0;
  for (let o = 0; o < png.data.length; o += 4) {
    if (
      Math.abs(png.data[o] - target[0]) <= tol &&
      Math.abs(png.data[o + 1] - target[1]) <= tol &&
      Math.abs(png.data[o + 2] - target[2]) <= tol
    ) n += 1;
  }
  return n;
}

test("breadth studies expose engine values and metadata through the browser host", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    source.set_data(Array.from({ length: 40 }, (_, index) => ({
      time: 1_700_000_000 + index * 60,
      open: 100 + index,
      high: 101 + index,
      low: 99 + index,
      close: 100 + index,
    })));
    const aroon = chart.add_aroon(source, 3);
    const ao = chart.add_awesome_oscillator(source);
    const dpo = chart.add_dpo(source, 5);
    const cmo = chart.add_chande_momentum(source, 5);
    const metrics = chart.add_bollinger_metrics(source, 5, 2);
    const envelopes = chart.add_envelopes(source, 5, 10, true);
    const alma = chart.add_alma(source, 5);
    const outputs = [...aroon, ao, dpo, cmo, ...metrics, ...envelopes, alma];
    const schema = chart.indicator_schema("envelopes", 5, 10);
    return { outputs: outputs.map((output) => ({
      kind: output.indicator_info().kind,
      value: output.data().at(-1)?.value,
      pane: output.pane_index(),
      exponential: output.indicator_info().parameters.exponential,
      offset: output.indicator_info().parameters.offset,
      sigma: output.indicator_info().parameters.sigma,
    })), schema };
  });
  const outputs = result.outputs;
  expect(outputs.slice(0, 5).map(({ kind, value }) => ({ kind, value }))).toEqual([
    { kind: "aroon", value: 100 },
    { kind: "aroon", value: 0 },
    { kind: "awesome_oscillator", value: 14.5 },
    { kind: "dpo", value: -1 },
    { kind: "chande_momentum", value: 100 },
  ]);
  expect(outputs.slice(5, 7).map(({ kind }) => kind)).toEqual(["bollinger_metrics", "bollinger_metrics"]);
  expect(outputs[5].value).toBeCloseTo((2 + 2 * Math.SQRT2) / (4 * Math.SQRT2), 8);
  expect(outputs[6].value).toBeCloseTo(4 * Math.SQRT2 / 137 * 100, 8);
  expect(outputs[5].pane).not.toBe(outputs[6].pane);
  expect(outputs.slice(7, 10).map(({ kind, exponential }) => ({ kind, exponential }))).toEqual(Array(3).fill({ kind: "envelopes", exponential: true }));
  for (const [index, expected] of [150.7, 137, 123.3].entries()) {
    expect(outputs[7 + index].value).toBeCloseTo(expected, 8);
  }
  expect(result.schema.parameters.find(({ name }) => name === "exponential")).toMatchObject({ parameter_type: "boolean", default: false });
  expect(outputs[10]).toMatchObject({ kind: "alma", offset: 0.85, sigma: 6 });
  expect(outputs[10].value).toBeGreaterThan(137);
  expect(outputs[10].value).toBeLessThan(139);
});

test("cumulative volume studies align sparse volume and repair historical insertions", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const volume = chart.add_series("histogram", { visible: false });
    const start = 1_700_000_000;
    source.set_data([
      { time: start, open: 10, high: 12, low: 8, close: 11 },
      { time: start + 60, open: 11, high: 14, low: 10, close: 13 },
      { time: start + 120, open: 13, high: 15, low: 11, close: 15 },
    ]);
    volume.set_data([{ time: start, value: 10 }, { time: start + 120, value: 30 }]);
    const adl = chart.add_accumulation_distribution(source, volume);
    const pvt = chart.add_price_volume_trend(source, volume);
    const chaikin = chart.add_chaikin_oscillator(source, 2, 3, volume);
    const relative = chart.add_relative_volume(source, 2, volume);
    const elder = chart.add_elder_force(source, 2, volume);
    const ease = chart.add_ease_of_movement(source, 2, volume, 100);
    const studies = [adl, pvt, chaikin, relative, elder, ease];
    const before = studies.map((output) => output.data().at(-1)?.value);
    volume.update({ time: start + 60, value: 20 });
    return {
      before,
      after: studies.map((output) => output.data().at(-1)?.value),
      kinds: studies.map((output) => output.indicator_info().kind),
      volume_ids: studies.map((output) => output.indicator_info().volume_source.id),
      volume_id: volume.id,
    };
  });
  expect(result.kinds).toEqual(["accumulation_distribution", "price_volume_trend", "chaikin_oscillator", "relative_volume", "elder_force", "ease_of_movement"]);
  expect(result.volume_ids).toEqual(Array(6).fill(result.volume_id));
  expect(result.before[0]).toBe(35);
  expect(result.before[1]).toBeCloseTo(60 / 13, 8);
  expect(result.before[2]).toBeCloseTo(10, 8);
  expect(result.before[3]).toBeCloseTo(6, 8);
  expect(result.before[4]).toBeCloseTo(30, 8);
  expect(result.before[5]).toBeUndefined();
  expect(result.after[0]).toBe(45);
  expect(result.after[1]).toBeCloseTo(40 / 11 + 60 / 13, 8);
  expect(result.after[2]).toBeCloseTo(35 / 3, 8);
  expect(result.after[3]).toBeCloseTo(2, 8);
  expect(result.after[4]).toBeCloseTo(50, 8);
  expect(result.after[5]).toBeCloseTo(80 / 3, 8);
});

test("volume oscillator keeps three ordered outputs across historical volume repair", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const volume = chart.add_series("histogram", { visible: false });
    const start = 1_700_100_000;
    source.set_data([0, 1, 2].map((i) => ({
      time: start + i * 60, open: 10 + i, high: 12 + i, low: 9 + i, close: 11 + i,
    })));
    volume.set_data([{ time: start, value: 10 }, { time: start + 120, value: 30 }]);
    const outputs = chart.add_volume_oscillator(source, 1, 2, 2, volume);
    const before = outputs.map((output) => output.data().at(-1)?.value);
    const kinds = outputs.map((output) => output.indicator_info().kind);
    const output_indices = outputs.map((output) => output.indicator_info().output_index);
    const binding_ids = outputs.map((output) => output.indicator_info().binding_id);
    volume.update({ time: start + 60, value: 20 });
    return {
      before,
      after: outputs.map((output) => output.data().at(-1)?.value),
      kinds,
      output_indices,
      binding_ids,
      ids: outputs.map((output) => output.id),
      volume_id: outputs[0].indicator_info().volume_source.id,
      expected_volume_id: volume.id,
      schema: chart.indicator_schema("volume_oscillator"),
    };
  });
  expect(result.kinds).toEqual(Array(3).fill("volume_oscillator"));
  expect(result.output_indices).toEqual([0, 1, 2]);
  expect(result.binding_ids).toEqual(Array(3).fill(result.ids[0]));
  expect(result.volume_id).toBe(result.expected_volume_id);
  expect(result.schema.parameters.map(({ name }) => name)).toContain("signal");
  expect(result.before[0]).toBeCloseTo(500 / 13, 8);
  expect(result.before[1]).toBeCloseTo(-400 / 13, 8);
  expect(result.before[2]).toBeCloseTo(900 / 13, 8);
  expect(result.after[0]).toBeCloseTo(20, 8);
  expect(result.after[1]).toBeCloseTo(80 / 3, 8);
  expect(result.after[2]).toBeCloseTo(-20 / 3, 8);
});

test("Klinger, KAMA, McGinley and regression expose ordered browser values and repair history", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const start = 1_702_000_000;
    const price = chart.add_series("candlestick", { visible: false });
    const volume = chart.add_series("histogram", { visible: false });
    price.set_data([2, 3, 2, 1].map((close, i) => ({
      time: start + i * 60, open: close, high: close + 1, low: close - 1, close,
    })));
    volume.set_data([0, 1, 2, 3].map((i) => ({ time: start + i * 60, value: 1 })));
    const adaptive = chart.add_series("line", { visible: false });
    adaptive.set_data([1, 2, 3, 4, 3, 4].map((value, i) => ({ time: start + i * 60, value })));
    const dynamic = chart.add_series("line", { visible: false });
    dynamic.set_data([2, 4, 4].map((value, i) => ({ time: start + i * 60, value })));
    const regression = chart.add_series("line", { visible: false });
    regression.set_data([1, 3, 2, 5].map((value, i) => ({ time: start + i * 60, value })));

    const bindings = [
      chart.add_klinger(price, 1, 2, 2, volume),
      [chart.add_kama(adaptive, 3, 2, 5)],
      [chart.add_mcginley(dynamic, 2)],
      chart.add_linear_regression(regression, 3, 2),
    ];
    const before = [
      bindings[0].map((s) => s.data().find((row) => row.time === start + 120)?.value),
      [bindings[1][0].data().find((row) => row.time === start + 240)?.value],
      [bindings[2][0].data().find((row) => row.time === start + 60)?.value],
      bindings[3].map((s) => s.data().find((row) => row.time === start + 120)?.value),
    ];
    const signalAtLast = bindings[0][1].data().at(-1)?.value;
    const metadata = bindings.map((group) => group.map((s) => ({
      id: s.id,
      kind: s.indicator_info().kind,
      parameters: s.indicator_info().parameters,
      index: s.indicator_info().output_index,
      count: s.indicator_info().output_count,
      binding_id: s.indicator_info().binding_id,
      volume_id: s.indicator_info().volume_source?.id,
      pane: s.pane_index(),
    })));
    const schemas = ["klinger", "kama", "mcginley", "linear_regression"].map((kind) =>
      chart.indicator_schema(kind).parameters.map(({ name, default: value }) => [name, value]));
    const panesBefore = chart.panes().length;
    const seriesOrderBefore = chart.series_order().map((series) => series.id);
    const invalid = [
      () => chart.add_klinger(price, 2, 2, 2, volume),
      () => chart.add_klinger(price, 1, 2, 2, null),
      () => chart.add_klinger(price, 1, 2, 1.5, volume),
      () => chart.add_kama(adaptive, 0, 2, 5),
      () => chart.add_kama(adaptive, 3, 5, 2),
      () => chart.add_kama(adaptive, 3.5, 2, 5),
      () => chart.add_kama(adaptive, 3, 0, 5),
      () => chart.add_kama(adaptive, 3, 2, Infinity),
      () => chart.add_mcginley(dynamic, Infinity),
      () => chart.add_mcginley(dynamic, 0),
      () => chart.add_mcginley(dynamic, 2.5),
      () => chart.add_mcginley(dynamic, 1_000_001),
      () => chart.add_linear_regression(regression, 0, 2),
      () => chart.add_linear_regression(regression, 3, -1),
      () => chart.add_linear_regression(regression, 3, NaN),
      () => chart.add_linear_regression(regression, 2.5, 2),
      () => chart.add_linear_regression(regression, 3, Infinity),
      () => chart.add_linear_regression(regression, 1_000_001, 2),
    ].map((add) => {
      try { add(); return false; } catch (error) {
        return error.name === "AerisChartsError" && error.code === "invalid_options";
      }
    });
    const panesAfter = chart.panes().length;
    const seriesOrderAfter = chart.series_order().map((series) => series.id);
    volume.update({ time: start + 60, value: 2 });
    adaptive.update({ time: start + 180, value: 5 });
    dynamic.update({ time: start + 60, value: 3 });
    regression.update({ time: start + 60, value: 4 });
    const after = bindings.map((group) => group.map((s) => s.data().at(-1)?.value));
    const fresh = [
      chart.add_klinger(price, 1, 2, 2, volume),
      [chart.add_kama(adaptive, 3, 2, 5)],
      [chart.add_mcginley(dynamic, 2)],
      chart.add_linear_regression(regression, 3, 2),
    ].map((group) => group.map((s) => s.data().at(-1)?.value));
    const defaults = [
      chart.add_klinger(price, undefined, undefined, undefined, volume)[0],
      chart.add_kama(adaptive),
      chart.add_mcginley(dynamic),
      chart.add_linear_regression(regression)[0],
    ].map((s) => s.indicator_info().parameters);
    return { before, signalAtLast, after, fresh, defaults, metadata, schemas, invalid, panesBefore, panesAfter,
      seriesOrderBefore, seriesOrderAfter,
      pricePane: price.pane_index(), volumeId: volume.id };
  });
  expect(result.before[0][0]).toBeCloseTo(-50 / 3, 8);
  expect(result.before[0][1]).toBeCloseTo(-100 / 3, 8);
  expect(result.signalAtLast).toBeCloseTo(-200 / 9, 8);
  expect(result.before[1][0]).toBeCloseTo(2 + 8 / 9 + (3 - 2 - 8 / 9) * 16 / 81, 8);
  expect(result.before[2][0]).toBeCloseTo(2.0625, 8);
  expect(result.before[3][0]).toBeCloseTo(2.5, 8);
  expect(result.before[3][1]).toBeCloseTo(2.5 + Math.SQRT2, 8);
  expect(result.before[3][2]).toBeCloseTo(2.5 - Math.SQRT2, 8);
  expect(result.invalid).toEqual(Array(18).fill(true));
  expect(result.panesAfter).toBe(result.panesBefore);
  expect(result.seriesOrderAfter).toEqual(result.seriesOrderBefore);
  expect(result.metadata.map((group) => group.map(({ kind }) => kind))).toEqual([
    ["klinger", "klinger"], ["kama"], ["mcginley"],
    ["linear_regression", "linear_regression", "linear_regression"],
  ]);
  for (const [i, group] of result.metadata.entries()) {
    expect(group.map(({ index }) => index)).toEqual([...group.keys()]);
    expect(group.map(({ count }) => count)).toEqual(Array(group.length).fill(group.length));
    expect(group.map(({ binding_id }) => binding_id)).toEqual(Array(group.length).fill(group[0].id));
    expect(new Set(group.map(({ pane }) => pane)).size).toBe(1);
    for (let j = 0; j < group.length; j++) expect(result.after[i][j]).toBeCloseTo(result.fresh[i][j], 8);
  }
  expect(result.metadata[0][0]).toMatchObject({
    parameters: { fast: 1, slow: 2, signal: 2 }, volume_id: result.volumeId,
  });
  expect(result.metadata[0][0].pane).not.toBe(result.pricePane);
  expect(result.metadata.slice(1).every((group) => group[0].pane === result.pricePane)).toBe(true);
  expect(result.metadata[1][0].parameters).toMatchObject({ period: 3, fast: 2, slow: 5 });
  expect(result.metadata[2][0].parameters).toMatchObject({ period: 2 });
  expect(result.metadata[3][0].parameters).toMatchObject({ period: 3, deviation: 2 });
  expect(result.defaults).toEqual([
    expect.objectContaining({ fast: 34, slow: 55, signal: 13 }),
    expect.objectContaining({ period: 10, fast: 2, slow: 30 }),
    expect.objectContaining({ period: 14 }),
    expect.objectContaining({ period: 20, deviation: 2 }),
  ]);
  expect(result.schemas).toEqual([
    [["source", "close"], ["fast", 34], ["slow", 55], ["signal", 13], ["volume_source", null]],
    [["source", "close"], ["period", 10], ["fast", 2], ["slow", 30]],
    [["source", "close"], ["period", 14]],
    [["source", "close"], ["period", 20], ["deviation", 2]],
  ]);
});

test("historical volatility annualizes sample log returns and repairs corrected history", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line", { visible: false });
    const start = 1_700_200_000;
    source.set_data([1, 2, 4, 16].map((value, row) => ({
      time: start + row * 60, value,
    })));
    const output = chart.add_historical_volatility(source, 2, 4);
    const before = output.data().at(-1)?.value;
    const info = output.indicator_info();
    source.update({ time: start + 120, value: 2 });
    return {
      before,
      after: output.data().at(-1)?.value,
      kind: info.kind,
      annualization: info.parameters.annualization,
      schema: chart.indicator_schema("historical_volatility"),
    };
  });
  expect(result.kind).toBe("historical_volatility");
  expect(result.annualization).toBe(4);
  expect(result.schema.parameters.map(({ name }) => name)).toContain("annualization");
  expect(result.before).toBeCloseTo(200 * Math.log(2) / Math.SQRT2, 8);
  expect(result.after).toBeCloseTo(200 * Math.log(8) / Math.SQRT2, 8);
});

test("TRIX returns ordered line and signal outputs after historical repair", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line", { visible: false });
    const start = 1_700_300_000;
    source.set_data([1, 2, 3, 4, 5, 6].map((value, row) => ({
      time: start + row * 60, value,
    })));
    const outputs = chart.add_trix(source, 2, 2);
    const before = outputs.map((output) => output.data().at(-1)?.value);
    const info = outputs.map((output) => output.indicator_info());
    source.update({ time: start + 120, value: 2 });
    return {
      before,
      after: outputs.map((output) => output.data().at(-1)?.value),
      kinds: info.map((entry) => entry.kind),
      indices: info.map((entry) => entry.output_index),
      binding_ids: info.map((entry) => entry.binding_id),
      schema: chart.indicator_schema("trix"),
    };
  });
  expect(result.kinds).toEqual(["trix", "trix"]);
  expect(result.indices).toEqual([0, 1]);
  expect(result.binding_ids[0]).toBe(result.binding_ids[1]);
  expect(result.schema.parameters.map(({ name }) => name)).toContain("signal");
  expect(result.before[0]).toBeCloseTo(200 / 7, 8);
  expect(result.before[1]).toBeCloseTo(240 / 7, 8);
  expect(result.after[0]).not.toBe(result.before[0]);
  expect(result.after[1]).not.toBe(result.before[1]);
});

test("KST, TSI, Mass Index, and Vortex expose exact browser values and repair history", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const start = 1_701_000_000;
    const source = chart.add_series("candlestick", { visible: false });
    const bars = [1, 2, 4, 3, 5].map((value, i) => ({
      time: start + i * 60, open: value, high: value, low: 0, close: value,
    }));
    source.set_data(bars);
    const kst = chart.add_kst(source, [1, 1, 1, 1], [1, 1, 1, 1], 2);
    const tsi = chart.add_tsi(source, 2, 2, 2);
    const mass = chart.add_mass_index(source, 2, 2);
    const vortexSource = chart.add_series("candlestick", { visible: false });
    vortexSource.set_data([1, 3, 5].map((close, i) => ({
      time: start + i * 60, open: close, high: close + 1, low: close - 1, close,
    })));
    const vortex = chart.add_vortex(vortexSource, 2);
    const groups = [kst, tsi, [mass], vortex];
    const before = groups.map((group) => group.map((s) => s.data().at(-1)?.value));
    const metadata = groups.map((group) => group.map((s) => ({
      kind: s.indicator_info().kind,
      params: s.indicator_info().parameters,
      index: s.indicator_info().output_index,
      count: s.indicator_info().output_count,
      pane: s.pane_index(),
    })));
    const schemas = ["kst", "tsi", "mass_index", "vortex"].map((kind) =>
      chart.indicator_schema(kind).parameters.map(({ name, default: value }) => [name, value]));
    const panesBefore = chart.panes().length;
    const invalid = [
      () => chart.add_kst(source, [1, 1, 0, 1], [1, 1, 1, 1], 2),
      () => chart.add_kst(source, [1, 1, 1, 1], [1, 1, 1, 1], 1.5),
      () => chart.add_tsi(source, 2, -1, 2),
      () => chart.add_mass_index(source, 2, Infinity),
      () => chart.add_vortex(vortexSource, 0),
    ].map((add) => { try { add(); return false; } catch (error) { return error.code === "invalid_options"; } });
    const panesAfter = chart.panes().length;
    source.update({ ...bars[3], close: 2, high: 2, open: 2 });
    vortexSource.update({ time: start + 60, open: 4, high: 5, low: 2, close: 4 });
    const after = groups.map((group) => group.map((s) => s.data().at(-1)?.value));
    // A fresh binding over the corrected history must agree with each live repaired binding.
    const recreated = [
      chart.add_kst(source, [1, 1, 1, 1], [1, 1, 1, 1], 2),
      chart.add_tsi(source, 2, 2, 2),
      [chart.add_mass_index(source, 2, 2)],
      chart.add_vortex(vortexSource, 2),
    ].map((group) => group.map((s) => s.data().at(-1)?.value));
    return { before, after, recreated, metadata, schemas, invalid, panesBefore, panesAfter };
  });
  expect(result.before[0][0]).toBeCloseTo(2000 / 3, 8);
  expect(result.before[0][1]).toBeCloseTo(625 / 3, 8); // mean of -250 and 2000/3
  expect(result.before[1][0]).toBeCloseTo(2900 / 43, 8);
  expect(result.before[1][1]).toBeCloseTo(2525 / 43, 8);
  expect(result.before[2][0]).toBeCloseTo(165 / 152 + 705 / 622, 8);
  expect(result.before[3][0]).toBeCloseTo(4 / 3, 8);
  expect(result.before[3][1]).toBe(0);
  expect(result.invalid).toEqual([true, true, true, true, true]);
  expect(result.panesAfter).toBe(result.panesBefore);
  expect(new Set(result.metadata.map((group) => group[0].pane)).size).toBe(4);
  expect(result.after[0][0]).toBeCloseTo(1500, 8);
  expect(result.after[0][1]).toBeCloseTo(500, 8);
  expect(result.after[1][0]).not.toBe(result.before[1][0]);
  expect(result.after[2][0]).not.toBe(result.before[2][0]);
  expect(result.after[3][0]).not.toBe(result.before[3][0]);
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < result.after[i].length; j++) {
      expect(result.after[i][j]).toBeCloseTo(result.recreated[i][j], 8);
    }
    expect(result.metadata[i].map(({ index }) => index)).toEqual([...result.metadata[i].keys()]);
    expect(new Set(result.metadata[i].map(({ pane }) => pane)).size).toBe(1);
  }
  expect(result.metadata.map((group) => group.map(({ kind }) => kind))).toEqual([
    ["kst", "kst"], ["tsi", "tsi"], ["mass_index"], ["vortex", "vortex"],
  ]);
  expect(result.metadata[0][0].params).toMatchObject({ roc: [1, 1, 1, 1], smoothing_periods: [1, 1, 1, 1], signal: 2 });
  expect(result.metadata[1][0].params).toMatchObject({ long_period: 2, short_period: 2, signal: 2 });
  expect(result.metadata[2][0].params).toMatchObject({ ema_period: 2, sum_period: 2 });
  expect(result.metadata[3][0].params).toMatchObject({ period: 2 });
  expect(result.schemas).toEqual([
    [["source", "close"], ["roc_1", 10], ["roc_2", 15], ["roc_3", 20], ["roc_4", 30],
      ["smoothing_1", 10], ["smoothing_2", 10], ["smoothing_3", 10], ["smoothing_4", 15], ["signal", 9]],
    [["source", "close"], ["long", 25], ["short", 13], ["signal", 13]],
    [["source", "close"], ["ema_period", 9], ["sum_period", 25]],
    [["source", "close"], ["period", 14]],
  ]);
});

test("Coppock Curve weights two rates of change and repairs historical prices", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("line", { visible: false });
    const start = 1_700_400_000;
    source.set_data([1, 2, 4, 8, 16].map((value, row) => ({
      time: start + row * 60, value,
    })));
    const output = chart.add_coppock_curve(source, 2, 1, 2);
    const before = output.data().at(-1)?.value;
    const info = output.indicator_info();
    source.update({ time: start + 120, value: 3 });
    return {
      before,
      after: output.data().at(-1)?.value,
      kind: info.kind,
      parameters: info.parameters,
      schema: chart.indicator_schema("coppock_curve"),
    };
  });
  expect(result.kind).toBe("coppock_curve");
  expect(result.parameters).toMatchObject({ long_period: 2, short_period: 1, smoothing: 2 });
  expect(result.schema.parameters.map(({ name }) => name)).toEqual(["source", "long_period", "short_period", "smoothing"]);
  expect(result.before).toBeCloseTo(400, 8);
  expect(result.after).toBeCloseTo(4600 / 9, 8);
});

test("Fisher Transform shares ordered line and trigger outputs and repairs extrema", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const start = 1_700_500_000;
    source.set_data([0, 1, 2].map((row) => ({
      time: start + row * 60, open: row * 2 + 1, high: row * 2 + 2,
      low: row * 2, close: row * 2 + 1,
    })));
    const outputs = chart.add_fisher_transform(source, 2);
    const before = outputs.map((output) => output.data().at(-1)?.value);
    const info = outputs.map((output) => output.indicator_info());
    source.update({ time: start + 60, open: 4, high: 5, low: 2, close: 4 });
    return {
      before,
      after: outputs.map((output) => output.data().at(-1)?.value),
      kinds: info.map((entry) => entry.kind),
      indices: info.map((entry) => entry.output_index),
      binding_ids: info.map((entry) => entry.binding_id),
      schema: chart.indicator_schema("fisher_transform"),
    };
  });
  const first = 0.5 * Math.log((1 + 0.165) / (1 - 0.165));
  const secondValue = 0.165 + 0.67 * 0.165;
  const second = 0.5 * Math.log((1 + secondValue) / (1 - secondValue)) + 0.5 * first;
  expect(result.kinds).toEqual(["fisher_transform", "fisher_transform"]);
  expect(result.indices).toEqual([0, 1]);
  expect(result.binding_ids[0]).toBe(result.binding_ids[1]);
  expect(result.schema.parameters.map(({ name }) => name)).toEqual(["source", "period"]);
  expect(result.before[0]).toBeCloseTo(second, 8);
  expect(result.before[1]).toBeCloseTo(first, 8);
  expect(result.after[0]).not.toBe(result.before[0]);
  expect(result.after[1]).not.toBe(result.before[1]);
});

test("Ultimate Oscillator weights buying pressure across three windows", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const start = 1_700_600_000;
    source.set_data([0, 1, 2].map((row) => ({
      time: start + row * 60, open: row * 4 + 4,
      high: row * 2 + 10, low: row * 2, close: row * 4 + 5,
    })));
    const output = chart.add_ultimate_oscillator(source, 1, 2, 3);
    const before = output.data().at(-1)?.value;
    const info = output.indicator_info();
    source.update({ time: start + 60, open: 8, high: 12, low: 2, close: 8 });
    return {
      before, after: output.data().at(-1)?.value,
      kind: info.kind, parameters: info.parameters,
      schema: chart.indicator_schema("ultimate_oscillator"),
    };
  });
  expect(result.kind).toBe("ultimate_oscillator");
  expect(result.parameters).toMatchObject({ short_period: 1, period: 2, long_period: 3 });
  expect(result.schema.parameters.map(({ name }) => name)).toEqual(["source", "short_period", "medium_period", "long_period"]);
  expect(result.before).toBeCloseTo(590 / 7, 8);
  expect(result.after).not.toBe(result.before);
});

test("EMA ribbon owns five colored outputs and updates periods in place", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const ribbon = window.__chart.add_ema_ribbon(window.__main);
    const before_ids = ribbon.map((series) => series.id);
    const before = ribbon.map((series) => {
      const info = series.indicator_info();
      return {
        color: series.options().color,
        line_width: series.options().line_width,
        title: series.options().title,
        binding_id: info.binding_id,
        kind: info.kind,
        period: info.period,
        periods: info.parameters.periods,
        output_name: info.output_name,
        output_index: info.output_index,
        output_count: info.output_count,
      };
    });

    ribbon[1].apply_options({ title: "Custom EMA" });
    const updated = window.__chart.set_ema_ribbon_periods(ribbon[2], [6, 12, 24, 60, 120]);
    const after = ribbon.map((series) => ({
      id: series.id,
      color: series.options().color,
      title: series.options().title,
      period: series.indicator_info().period,
      periods: series.indicator_info().parameters.periods,
    }));
    return { before_ids, before, updated, after };
  });

  expect(result.before.map((output) => output.color)).toEqual([
    "#335cff",
    "#FF9800",
    "#7d52f4",
    "#fb4ba3",
    "#fb3748",
  ]);
  expect(result.before.map((output) => output.title)).toEqual([
    "EMA 5",
    "EMA 10",
    "EMA 20",
    "EMA 50",
    "EMA 200",
  ]);
  expect(result.before.map((output) => output.kind)).toEqual(Array(5).fill("ema_ribbon"));
  expect(result.before.map((output) => output.line_width)).toEqual(Array(5).fill(1));
  expect(result.before.map((output) => output.period)).toEqual([5, 10, 20, 50, 200]);
  expect(result.before.map((output) => output.output_name)).toEqual([
    "EMA 1",
    "EMA 2",
    "EMA 3",
    "EMA 4",
    "EMA 5",
  ]);
  expect(result.before.map((output) => output.output_index)).toEqual([0, 1, 2, 3, 4]);
  expect(result.before.map((output) => output.output_count)).toEqual(Array(5).fill(5));
  expect(new Set(result.before.map((output) => output.binding_id)).size).toBe(1);
  expect(result.before.every((output) => JSON.stringify(output.periods) === "[5,10,20,50,200]")).toBe(true);

  expect(result.updated).toBe(true);
  expect(result.after.map((output) => output.id)).toEqual(result.before_ids);
  expect(result.after.map((output) => output.color)).toEqual(result.before.map((output) => output.color));
  expect(result.after.map((output) => output.title)).toEqual([
    "EMA 6",
    "Custom EMA",
    "EMA 24",
    "EMA 60",
    "EMA 120",
  ]);
  expect(result.after.map((output) => output.period)).toEqual([6, 12, 24, 60, 120]);
  expect(result.after.every((output) => JSON.stringify(output.periods) === "[6,12,24,60,120]")).toBe(true);
});

test("indicator sources recover from empty, short, and retention-trimmed warm-up", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const rows = Array.from({ length: 20 }, (_, index) => ({
      time: 1_700_000_000 + index * 60,
      value: 100 + index,
    }));
    const repopulate = (replacement_rows) => {
      const source = window.__chart.add_series("line", { visible: false });
      source.set_data(rows);
      const sma = window.__chart.add_sma(source, 20);
      source.set_data(rows.slice(0, replacement_rows));
      const empty_rows = sma.data().length;
      for (const row of rows.slice(replacement_rows)) source.update(row);
      const output = sma.data();
      window.__chart.remove_series(source);
      return { empty_rows, output };
    };

    const retained = window.__chart.add_series("line", { visible: false });
    retained.set_data(rows);
    const retained_sma = window.__chart.add_sma(retained, 20);
    retained.apply_options({ max_points: 1 });
    for (let index = 20; index < 40; index += 1) {
      retained.update({ time: 1_700_000_000 + index * 60, value: 100 + index });
    }
    return {
      empty: repopulate(0),
      short: repopulate(10),
      retained_rows: retained_sma.data().length,
    };
  });

  for (const replacement of [result.empty, result.short]) {
    expect(replacement.empty_rows).toBe(0);
    expect(replacement.output).toEqual([{ time: 1_700_001_140, value: 109.5 }]);
  }
  expect(result.retained_rows).toBe(0);
});

/** Chart screenshot decoded to a PNG plus a row-crop counter (geometry is CSS px, shots are device px). */
async function shot(page) {
  const { url, chart_width } = await page.evaluate(() => ({
    url: window.__chart.take_screenshot().toDataURL("image/png"),
    chart_width: document.getElementById("chart_container").getBoundingClientRect().width,
  }));
  const png = PNG.sync.read(Buffer.from(url.split(",")[1], "base64"));
  // The demo has a dockable inspector, so the chart bitmap no longer necessarily spans the
  // viewport. Pane geometry is in chart-local CSS px; derive its scale from the actual chart.
  const dsf = png.width / chart_width;
  const crop = (top_css, bottom_css) => {
    const top = Math.max(0, Math.floor(top_css * dsf));
    const bottom = Math.min(png.height, Math.ceil(bottom_css * dsf));
    const sub = new PNG({ width: png.width, height: bottom - top });
    PNG.bitblt(png, sub, 0, top, png.width, bottom - top, 0, 0);
    return sub;
  };
  return { png, dsf, crop };
}

test("bollinger bands paint their background fill between upper and lower", async ({ page }) => {
  // Pin the light theme: the expected fill is #2196f3 at 0.2 alpha over white.
  await page.goto("/?theme=light");
  await wait_grid(page);
  await page.evaluate(() => {
    window.__bands = window.__chart.add_bollinger(window.__main, 20, 2);
  });
  await wait_grid(page);
  const { png } = await shot(page);
  // Band color #2196f3 at 0.2 alpha over the white background blends to ~(211, 234, 253).
  expect(count_color(png, [211, 234, 253], 8), "band fill pixels").toBeGreaterThan(200);
});

test("rsi and macd stack their own panes with channel strip and four-state histogram", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);

  const added = await page.evaluate(() => {
    const events = [];
    window.__chart.subscribe_series_added((e) => events.push({ id: e.series.id, pane: e.pane_index }));
    window.__rsi = window.__chart.add_rsi(window.__main, 14);
    window.__macd = window.__chart.add_macd(window.__main, 12, 26, 9);
    return {
      events: events.length,
      rsi_pane: events[0].pane,
      macd_panes: events.slice(1).map((e) => e.pane),
      rsi_kind: window.__rsi.indicator_info().kind,
      hist_type: window.__macd[2].series_type(),
      hist_slot: window.__macd[2].indicator_info().output_index,
      panes: window.__chart.panes().length,
    };
  });
  expect(added.events).toBe(4); // 1 rsi + 3 macd outputs
  expect(added.rsi_pane).toBe(1);
  expect(added.macd_panes).toEqual([2, 2, 2]);
  expect(added.rsi_kind).toBe("rsi");
  expect(added.hist_type).toBe("histogram");
  expect(added.hist_slot).toBe(2);
  expect(added.panes).toBe(3);

  await wait_grid(page);
  const geo = await page.evaluate(() => window.__chart.panes().map((p) => p.get_geometry()));
  const { crop } = await shot(page);
  // RSI channel strip rgba(120,123,134,0.2) over the canonical #1f1f1f dark surface.
  const rsi_strip = count_color(crop(geo[1].top, geo[1].top + geo[1].height), [49, 49, 52], 6);
  expect(rsi_strip, "rsi 30/70 channel strip pixels").toBeGreaterThan(500);
  // MACD histogram strong-up columns paint the opaque candle green.
  const macd_green = count_color(crop(geo[2].top, geo[2].top + geo[2].height), [8, 153, 129], 10);
  expect(macd_green, "macd histogram strong-state pixels").toBeGreaterThan(20);
});

test("indicator chips: auto-name on, no countdown, 2px default, style overrides", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const out = await page.evaluate(() => {
    const rsi = window.__chart.add_rsi(window.__main, 14);
    const before = rsi.options();
    // The platform surfaces its own chip: custom name, chip visible, dotted 1px override.
    rsi.apply_options({ title: "RSI(14) 1h", title_visible: true, line_style: 1, line_width: 1 });
    const after = rsi.options();
    return {
      before: {
        title: before.title,
        title_visible: before.title_visible,
        countdown: before.countdown_visible,
        width: before.line_width,
        price_line_visible: before.price_line_visible,
        price_line_extent: before.price_line_extent,
        price_line_width: before.price_line_width,
        price_line_style: before.price_line_style,
      },
      after: {
        title: after.title,
        title_visible: after.title_visible,
        line_style: after.line_style,
        line_width: after.line_width,
      },
    };
  });
  expect(out.before).toEqual({
    title: "RSI 14",
    title_visible: true,
    countdown: false,
    width: 2,
    price_line_visible: true,
    price_line_extent: "partial",
    price_line_width: 1,
    price_line_style: 1,
  });
  expect(out.after).toEqual({ title: "RSI(14) 1h", title_visible: true, line_style: 1, line_width: 1 });
});

test("demo-created indicators and feature companions never enable candle countdowns", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const outputs = await page.evaluate(() => {
    window.__demo_catalogs.series.activate("shaded-background");
    window.__demo_catalogs.lab.activate("overlay-scale");
    document.getElementById("sma_toggle").click(); // Catalog indicators are created on demand.
    document.getElementById("rsi_toggle").click();
    return window.__chart.series_order()
      .filter((series) => series !== window.__main)
      .map((series) => ({
        type: series.series_type(),
        title: series.options().title,
        countdown_visible: series.options().countdown_visible,
      }));
  });
  expect(outputs.length).toBeGreaterThanOrEqual(5);
  expect(outputs.every((series) => series.countdown_visible === false)).toBe(true);
});

test("crosshair hover marker paints on an engine-created indicator line and remains configurable", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const probe = await page.evaluate(() => {
    const default_sma = window.__chart.add_sma(window.__main, 5);
    const default_visible = default_sma.options().crosshair_marker_visible;
    window.__chart.remove_series(default_sma);
    window.__hover_sma = window.__chart.add_sma(window.__main, 5, {
      color: "#ff9900",
      crosshair_marker_visible: true,
      crosshair_marker_radius: 6,
      crosshair_marker_border_width: 3,
      crosshair_marker_border_color: "#010203",
      crosshair_marker_background_color: "#040506",
    });
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor((range.from + range.to) / 2);
    const point = window.__hover_sma.data_by_index(index);
    window.__chart.set_crosshair_position(point.value, point.time, window.__hover_sma);
    const options = window.__hover_sma.options();
    return {
      x: window.__chart.time_scale().logical_to_coordinate(index),
      y: window.__hover_sma.price_to_coordinate(point.value),
      default_visible,
      options: {
        visible: options.crosshair_marker_visible,
        radius: options.crosshair_marker_radius,
        border_width: options.crosshair_marker_border_width,
        border: options.crosshair_marker_border_color,
        fill: options.crosshair_marker_background_color,
      },
    };
  });
  expect(probe.default_visible).toBe(false);
  expect(probe.options).toEqual({
    visible: true,
    radius: 6,
    border_width: 3,
    border: "#010203",
    fill: "#040506",
  });
  await wait_grid(page);

  const local_color_count = async (target) => {
    const { png, dsf } = await shot(page);
    const cx = Math.round(probe.x * dsf);
    const cy = Math.round(probe.y * dsf);
    const reach = Math.ceil(11 * dsf);
    let count = 0;
    for (let y = cy - reach; y <= cy + reach; y += 1) {
      for (let x = cx - reach; x <= cx + reach; x += 1) {
        const offset = (y * png.width + x) * 4;
        if (
          png.data[offset] === target[0]
          && png.data[offset + 1] === target[1]
          && png.data[offset + 2] === target[2]
        ) count += 1;
      }
    }
    return count;
  };
  expect(await local_color_count([1, 2, 3]), "marker border pixels").toBeGreaterThan(10);
  expect(await local_color_count([4, 5, 6]), "marker fill pixels").toBeGreaterThan(10);

  await page.evaluate(() => window.__hover_sma.apply_options({ crosshair_marker_visible: false }));
  await wait_grid(page);
  expect(await local_color_count([1, 2, 3]), "disabled marker border").toBe(0);
  expect(await local_color_count([4, 5, 6]), "disabled marker fill").toBe(0);
});

test("indicator values inherit source precision at creation", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const values = await page.evaluate(() => {
    window.__main.apply_options({ price_format: { type: "price", precision: 0, min_move: 1 } });
    const [upper] = window.__chart.add_bollinger(window.__main, 20, 2);
    const inherited = upper.price_formatter()(65475.46);
    upper.apply_options({ price_format: { type: "price", precision: 4, min_move: 0.0001 } });
    return { inherited, overridden: upper.price_formatter()(65475.46) };
  });
  expect(values).toEqual({ inherited: "65,475", overridden: "65,475.4600" });
});

for (const backend of ["canvas2d", "webgpu"]) {
  test(`${backend}: hiding the sole SMA preserves scale precision and spacing`, async ({ page }) => {
    await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
    await wait_grid(page);
    const result = await page.evaluate(async () => {
      const chart = window.__chart;
      window.__main.apply_options({
        price_format: { type: "price", precision: 0, min_move: 1 },
      });
      const scale = chart.add_price_scale({ id: "sma-only", side: "right", order: 0 });
      const sma = chart.add_sma(window.__main, 20);
      sma.apply_options({
        price_scale_id: "sma-only",
        last_value_visible: false,
        price_line_visible: false,
        title_visible: false,
      });
      chart.time_scale().fit_content();
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const range = chart.time_scale().get_visible_logical_range();
      const logical = Math.floor((range.from + range.to) / 2);
      const point = sma.data_by_index(logical);
      const snapshot = () => ({
        pane_width: chart.time_scale().width(),
        scale_width: scale.width(),
        range: scale.get_visible_range(),
        coordinate: sma.price_to_coordinate(point.value),
      });
      const initial = snapshot();

      sma.apply_options({ visible: false });
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const hidden = snapshot();

      sma.apply_options({ visible: true });
      await new Promise((resolve) => requestAnimationFrame(resolve));
      const shown = snapshot();
      return { actual_backend: chart.backend(), initial, hidden, shown };
    });

    expect(result.actual_backend).toBe(backend);
    expect(result.initial.scale_width).toBeGreaterThan(0);
    expect(result.hidden).toEqual(result.initial);
    expect(result.shown).toEqual(result.initial);
  });
}

test("stochastic, atr, vwap, and wma register with lineage and placement", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const out = await page.evaluate(() => {
    const panes_before = window.__chart.panes().length;
    const stoch = window.__chart.add_stochastic(window.__main, 14, 3);
    const atr = window.__chart.add_atr(window.__main, 14);
    const vwap = window.__chart.add_vwap(window.__main, null);
    const wma = window.__chart.add_wma(window.__main, 20);
    const info = (s) => {
      const i = s.indicator_info();
      return { kind: i.kind, period: i.period, deviation: i.deviation, output_index: i.output_index };
    };
    return {
      panes_before,
      panes_after: window.__chart.panes().length,
      stoch: [info(stoch[0]), info(stoch[1])],
      atr: info(atr),
      vwap: info(vwap),
      wma: info(wma),
      overlays_on_price_pane: [vwap, wma].map((s) =>
        window.__chart.panes()[0].get_series().some((p) => p.id === s.id),
      ),
      stoch_same_pane: window.__chart
        .panes()
        .map((p) => p.get_series().some((s) => s.id === stoch[0].id || s.id === stoch[1].id)),
    };
  });
  expect(out.panes_after).toBe(out.panes_before + 2); // stochastic + atr each own a pane
  expect(out.stoch[0]).toEqual({ kind: "stochastic", period: 14, deviation: 3, output_index: 0 });
  expect(out.stoch[1].output_index).toBe(1);
  expect(out.atr.kind).toBe("atr");
  expect(out.vwap).toEqual({ kind: "vwap", period: 0, deviation: null, output_index: 0 });
  expect(out.wma).toEqual({ kind: "wma", period: 20, deviation: null, output_index: 0 });
  expect(out.overlays_on_price_pane).toEqual([true, true]);
  // Both stochastic lines share one pane (and it is not the price pane).
  const stoch_pane = out.stoch_same_pane.findIndex(Boolean);
  expect(stoch_pane).toBeGreaterThan(0);
});

test("Choppiness computes short true-range windows in an oscillator pane", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const bars = [
      [12, 8, 10], [14, 9, 12], [13, 10, 11], [16, 11, 14], [15, 12, 13],
    ];
    source.set_data(bars.map(([high, low, close], index) => ({
      time: 1_705_000_000 + index * 60, open: close, high, low, close,
    })));
    const panes_before = chart.panes().length;
    const output = chart.add_choppiness(source, 2);
    return {
      data: output.data(),
      info: {
        ...output.indicator_info(),
        source: { id: output.indicator_info().source.id },
      },
      source_id: source.id,
      panes_before,
      panes_after: chart.panes().length,
      pane: output.pane_index(),
      pane_has_output: chart.panes()[output.pane_index()].get_series().some((series) => series.id === output.id),
    };
  });
  expect(result.data.map(({ value }) => value)).toHaveLength(3);
  // At rows 2–4: TR pairs (5,3), (3,5), (5,3), and high-low spans 5, 6, 5.
  expect(result.data.map(({ time }) => time)).toEqual([1_705_000_120, 1_705_000_180, 1_705_000_240]);
  for (const [index, expected] of [8 / 5, 8 / 6, 8 / 5].entries()) {
    expect(result.data[index].value).toBeCloseTo(100 * Math.log(expected) / Math.log(2), 8);
  }
  expect(result.panes_after).toBe(result.panes_before + 1);
  expect(result.pane).toBeGreaterThan(0);
  expect(result.pane_has_output).toBe(true);
  expect(result.info).toMatchObject({
    kind: "choppiness", period: 2, output_index: 0, source: { id: result.source_id },
    parameters: { period: 2 },
  });
});

test("ATR bands follow close plus or minus Wilder ATR on the price pane", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const bars = [
      [12, 8, 10], [14, 9, 12], [13, 10, 11], [16, 11, 14], [15, 12, 13],
    ];
    source.set_data(bars.map(([high, low, close], index) => ({
      time: 1_705_100_000 + index * 60, open: close, high, low, close,
    })));
    const panes_before = chart.panes().length;
    const outputs = chart.add_atr_bands(source, 2, 1.5);
    return {
      values: outputs.map((series) => series.data().map(({ value }) => value)),
      info: outputs.map((series) => ({
        ...series.indicator_info(),
        source: { id: series.indicator_info().source.id },
      })),
      panes_before,
      panes_after: chart.panes().length,
      pane_indices: outputs.map((series) => series.pane_index()),
      on_price_pane: outputs.map((series) =>
        chart.panes()[0].get_series().some((candidate) => candidate.id === series.id)),
      source_id: source.id,
      output_ids: outputs.map((series) => series.id),
    };
  });
  // First ATR = (TR[1] + TR[2]) / 2 = (5 + 3) / 2 = 4;
  // subsequent Wilder ATRs are (4 + 5) / 2 = 4.5, (4.5 + 3) / 2 = 3.75.
  const closes = [11, 14, 13];
  const atrs = [4, 4.5, 3.75];
  expect(result.values).toHaveLength(3);
  for (const [slot, sign] of [[0, 1], [1, 0], [2, -1]]) {
    expect(result.values[slot]).toHaveLength(3);
    for (let row = 0; row < 3; row += 1) {
      expect(result.values[slot][row]).toBeCloseTo(closes[row] + sign * 1.5 * atrs[row], 8);
    }
  }
  expect(result.panes_after).toBe(result.panes_before);
  expect(result.pane_indices).toEqual([0, 0, 0]);
  expect(result.on_price_pane).toEqual([true, true, true]);
  expect(result.info.map(({ kind, output_index, output_name }) => ({ kind, output_index, output_name }))).toEqual([
    { kind: "atr_bands", output_index: 0, output_name: "Upper" },
    { kind: "atr_bands", output_index: 1, output_name: "Basis" },
    { kind: "atr_bands", output_index: 2, output_name: "Lower" },
  ]);
  for (const info of result.info) {
    expect(info).toMatchObject({
      period: 2, deviation: 1.5, source: { id: result.source_id },
      parameters: { period: 2, multiplier: 1.5 },
    });
    expect(info.binding_id).toBe(result.output_ids[0]);
  }
});

test("invalid Choppiness and ATR bands parameters reject without allocating outputs or panes", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    source.set_data([10, 11, 12].map((close, index) => ({
      time: 1_705_200_000 + index * 60, open: close, high: close + 2, low: close - 2, close,
    })));
    const snapshot = () => chart.panes().map((pane) => pane.get_series().map((series) => series.id));
    const before = snapshot();
    const attempts = [
      () => chart.add_choppiness(source, 1),
      () => chart.add_choppiness(source, 2.5),
      () => chart.add_atr_bands(source, 0, 1.5),
      () => chart.add_atr_bands(source, 2, -1),
      () => chart.add_atr_bands(source, 2, Number.NaN),
    ];
    return attempts.map((attempt) => {
      let code;
      try { attempt(); } catch (error) { code = error.code; }
      return { code, panes: snapshot(), unchanged: JSON.stringify(snapshot()) === JSON.stringify(before) };
    });
  });
  expect(result.map(({ code }) => code)).toEqual(Array(5).fill("invalid_options"));
  expect(result.every(({ unchanged }) => unchanged)).toBe(true);
});

test("browser schemas retain every multi-parameter Rust canonical default", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  // Explicit browser contract, independent of the engine's schema builder and indicator functions.
  // Includes volume-source descriptors when they are the second parameter.
  const defaults = {
    swing_points: { left: 5, right: 5 },
    market_structure: { left: 5, right: 5, break_on: "close" },
    fair_value_gaps: {
      min_size: 0, mitigation: "touch", mitigation_price: "wick", max_active: 20, show_mitigated: false,
    },
    order_blocks: {
      left: 5, right: 5, break_on: "close", zone: "wick",
      mitigation: "touch", mitigation_price: "wick", max_active: 20, show_mitigated: false,
    },
    previous_period_levels: { period: "day", calendar: "utc" },
    opening_range: { duration_seconds: 1800, calendar: "utc" },
    stochastic_rsi: { rsi_period: 14, stochastic_period: 14 },
    bollinger_metrics: { period: 14, deviation: 2 },
    envelopes: { period: 14, percent: 2, exponential: false },
    alma: { period: 14, offset: 0.85, sigma: 6 },
    keltner: { period: 14, multiplier: 2 },
    supertrend: { period: 14, multiplier: 3 },
    ema_ribbon: { period_1: 5, period_2: 10, period_3: 20, period_4: 50, period_5: 200 },
    // The fork's seed, histogram multiplier and estimator descriptors default to the textbook forms.
    bollinger: { period: 14, deviation: 2, estimator: "population" },
    macd: { fast: 12, slow: 26, signal: 9, seed: "sma", histogram_multiplier: 1 },
    kdj: { period: 9, k_smoothing: 3, d_smoothing: 3, seed: "fifty" },
    stochastic: { k_period: 14, d_period: 3 },
    chaikin_oscillator: { fast: 3, slow: 10, volume_source: null },
    klinger: { fast: 34, slow: 55, signal: 13, volume_source: null },
    kama: { period: 10, fast: 2, slow: 30 },
    linear_regression: { period: 20, deviation: 2 },
    atr_bands: { period: 14, multiplier: 2 },
    relative_volume: { period: 14, volume_source: null },
    elder_force: { period: 14, volume_source: null },
    ease_of_movement: { period: 14, divisor: 100_000_000, volume_source: null },
    historical_volatility: { period: 14, annualization: 252 },
    trix: { period: 14, signal: 9 },
    kst: {
      roc_1: 10, roc_2: 15, roc_3: 20, roc_4: 30,
      smoothing_1: 10, smoothing_2: 10, smoothing_3: 10, smoothing_4: 15, signal: 9,
    },
    tsi: { long: 25, short: 13, signal: 13 },
    mass_index: { ema_period: 9, sum_period: 25 },
    coppock_curve: { long_period: 14, short_period: 11, smoothing: 10 },
    ultimate_oscillator: { short_period: 7, medium_period: 14, long_period: 28 },
    volume_oscillator: { fast: 12, slow: 26, signal: 9, volume_source: null },
    cmf: { period: 14, volume_source: null },
    mfi: { period: 14, volume_source: null },
    volume: { period: 14, volume_source: null },
    vwma: { period: 14, volume_source: null },
    vwap_bands: { reset: "session", standard_deviation: 1, percent: 10, volume_source: null },
  };
  const actual = await page.evaluate((names) => Object.fromEntries(names.map((name) => {
    const schema = window.__chart.indicator_schema(name);
    return [name, {
      kind: schema.kind,
      parameters: Object.fromEntries(schema.parameters
        .filter(({ name: parameter }) => parameter !== "source")
        .map(({ name: parameter, default: value }) => [parameter, value])),
    }];
  })), Object.keys(defaults));
  for (const [kind, expected] of Object.entries(defaults)) {
    expect(actual[kind].kind, kind).toBe(kind);
    expect(actual[kind].parameters, kind).toEqual(expected);
  }
});
