import { test, expect } from "@playwright/test";

test("browser structure studies expose values, typed annotations and parameter validation", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const time = 1_700_000_000;
    source.set_data([
      { time, open: 9, high: 10, low: 8, close: 9 },
      { time: time + 60, open: 12, high: 14, low: 9, close: 13 },
      { time: time + 120, open: 12, high: 12, low: 10, close: 11 },
      { time: time + 180, open: 16, high: 18, low: 15, close: 17 },
      { time: time + 240, open: 16, high: 16, low: 15, close: 15 },
      { time: time + 300, open: 18, high: 20, low: 8, close: 19 },
      { time: time + 360, open: 19, high: 21, low: 16, close: 20 },
    ]);
    const swings = chart.add_swing_points(source, 1, 1);
    const market = chart.add_market_structure(source, 1, 1, "close");
    const gaps = chart.add_fair_value_gaps(source, { min_size: 0.5, show_mitigated: true });
    const blocks = chart.add_order_blocks(source, { left: 1, right: 1, zone: "body", show_mitigated: true });
    const studies = [...swings, market, gaps, blocks];
    const beforeInvalid = chart.series_order().length;
    const errors = [
      () => chart.add_swing_points(source, 0, 1),
      () => chart.add_swing_points(source, 1, 51),
      () => chart.add_market_structure(source, 1, 1, "invalid"),
      () => chart.add_market_structure(source, 0, 1),
      () => chart.add_market_structure(source, 1, 51),
      () => chart.add_fair_value_gaps(source, { min_size: -1 }),
      () => chart.add_fair_value_gaps(source, { min_size: Infinity }),
      () => chart.add_fair_value_gaps(source, { min_size: NaN }),
      () => chart.add_fair_value_gaps(source, { mitigation: "invalid" }),
      () => chart.add_fair_value_gaps(source, { max_active: 65 }),
      () => chart.add_fair_value_gaps(source, { max_active: 0 }),
      () => chart.add_fair_value_gaps(source, { mitigation_price: "invalid" }),
      () => chart.add_fair_value_gaps(source, { mitigation: null }),
      () => chart.add_fair_value_gaps(source, { show_mitigated: null }),
      () => chart.add_fair_value_gaps(source, { max_active: 1.5 }),
      () => chart.add_order_blocks(source, { zone: "invalid" }),
      () => chart.add_order_blocks(source, { mitigation_price: "invalid" }),
      () => chart.add_order_blocks(source, { break_on: "invalid" }),
      () => chart.add_order_blocks(source, { mitigation: "invalid" }),
      () => chart.add_order_blocks(source, { max_active: 0 }),
      () => chart.add_order_blocks(source, { left: 0 }),
      () => chart.add_order_blocks(source, { right: 51 }),
    ].map((invoke) => {
      try { invoke(); return null; } catch (error) {
        return { name: error.name, code: error.code };
      }
    });
    const afterInvalid = chart.series_order().length;
    const snapshot = studies.map((output) => ({
      kind: output.indicator_info().kind,
      parameters: output.indicator_info().parameters,
      data: output.data(),
      annotations: chart.study_annotations(output),
    }));
    const schemas = ["swing_points", "market_structure", "fair_value_gaps", "order_blocks"]
      .map((kind) => chart.indicator_schema(kind));
    let plainError = null;
    try { chart.study_annotations(source); } catch (error) { plainError = error.code; }
    chart.remove_series(source);
    let removedError = null;
    try { chart.study_annotations(market); } catch (error) { removedError = error.code; }
    return { snapshot, schemas, errors, beforeInvalid, afterInvalid, plainError, removedError };
  });
  expect(result.snapshot.map(({ kind }) => kind)).toEqual([
    "swing_points", "swing_points", "market_structure", "fair_value_gaps", "order_blocks",
  ]);
  expect(result.snapshot[0].annotations.markers).toContainEqual(expect.objectContaining({
    row: 1, confirm_row: 2, price: 14, kind: "swing_high",
  }));
  expect(result.snapshot[0].data[2].value).toBe(14);
  expect(result.snapshot[0].parameters).toMatchObject({ left: 1, right: 1 });
  expect(result.snapshot[2].annotations.markers).toContainEqual(expect.objectContaining({
    row: 3, confirm_row: 3, price: 14, from_row: 1, kind: { bos: { up: true } },
  }));
  expect(result.snapshot[3].annotations.zones).toContainEqual(expect.objectContaining({
    start_row: 2, confirm_row: 3, top: 15, bottom: 14, bullish: true,
  }));
  expect(result.snapshot[4].annotations.zones).toContainEqual(expect.objectContaining({
    start_row: 2, confirm_row: 3, top: 12, bottom: 11, bullish: true,
  }));
  expect(result.snapshot.slice(2).every(({ data }) => data.every(({ value }) => value === undefined))).toBe(true);
  expect(result.snapshot[2].parameters).toMatchObject({ left: 1, right: 1, break_on: "close" });
  expect(result.snapshot[3].parameters).toMatchObject({
    min_size: 0.5, mitigation: "touch", mitigation_price: "wick", max_active: 20, show_mitigated: true,
  });
  expect(result.snapshot[4].parameters).toMatchObject({
    left: 1, right: 1, break_on: "close", zone: "body",
    mitigation: "touch", mitigation_price: "wick", max_active: 20, show_mitigated: true,
  });
  // The fork's schema revision 4 (upstream 2): choice lists travel as `options`.
  expect(result.schemas.map(({ revision }) => revision)).toEqual([4, 4, 4, 4]);
  for (const [kind, name, defaultValue, options] of [
    ["market_structure", "break_on", "close", ["close", "wick"]],
    ["fair_value_gaps", "mitigation_price", "wick", ["wick", "close"]],
    ["order_blocks", "break_on", "close", ["close", "wick"]],
    ["order_blocks", "zone", "wick", ["wick", "body"]],
    ["order_blocks", "mitigation", "touch", ["touch", "half", "full"]],
    ["order_blocks", "mitigation_price", "wick", ["wick", "close"]],
  ]) {
    expect(result.schemas.find((schema) => schema.kind === kind).parameters.find((param) => param.name === name))
      .toMatchObject({ parameter_type: "choice", default: defaultValue, options });
  }
  expect(result.schemas[2].parameters.find(({ name }) => name === "mitigation")).toMatchObject({
    parameter_type: "choice", default: "touch", options: ["touch", "half", "full"],
  });
  expect(result.errors).toEqual(Array(22).fill({ name: "AerisChartsError", code: "invalid_options" }));
  expect(result.afterInvalid).toBe(result.beforeInvalid);
  expect(result.plainError).toBe("unsupported_operation");
  expect(result.removedError).toBe("invalid_handle");
});

test("max active retires zones without deleting their browser history", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const zones = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    source.set_data(Array.from({ length: 30 }, (_, row) => ({
      time: 1_700_100_000 + row * 60,
      open: row * 3 + 10, high: row * 3 + 11,
      low: row * 3 + 9, close: row * 3 + 10,
    })));
    const gap = chart.add_fair_value_gaps(source, {
      max_active: 1, mitigation: "full", mitigation_price: "close", show_mitigated: true,
    });
    return chart.study_annotations(gap).zones;
  });
  expect(zones).toHaveLength(28);
  expect(zones[0]).toMatchObject({
    start_row: 1, confirm_row: 2, end_row: 3, retired: true,
  });
  expect(zones.at(-1)).toMatchObject({ end_row: null, retired: false });
});

test("whitespace structure anchors align to source rows without expanding price autoscale", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const source = chart.add_series("candlestick");
    source.set_data([
      { time: 1_700_000_000, open: 10, high: 12, low: 9, close: 11 },
      { time: 1_700_000_060, open: 12, high: 15, low: 10, close: 14 },
      { time: 1_700_000_120, open: 11, high: 13, low: 9, close: 10 },
      { time: 1_700_000_180, open: 16, high: 18, low: 14, close: 17 },
    ]);
    chart.time_scale().fit_content();
    source.price_scale().set_auto_scale(true);
    const before = source.price_scale().get_visible_range();
    const outputs = [
      chart.add_market_structure(source, 1, 1),
      chart.add_fair_value_gaps(source),
      chart.add_order_blocks(source, { left: 1, right: 1 }),
    ];
    const after = source.price_scale().get_visible_range();
    const outlier = chart.add_series("line");
    outlier.set_data([{ time: 1_700_000_180, value: 1000 }]);
    return {
      before,
      after,
      withOutlier: source.price_scale().get_visible_range(),
      times: source.data().map(({ time }) => time),
      anchors: outputs.map((output) => output.data().map(({ time, value }) => ({
        time, hasValue: value !== undefined,
      }))),
    };
  });
  expect(result.before).not.toBeNull();
  expect(result.before.from).toBeLessThanOrEqual(9);
  expect(result.before.to).toBeGreaterThanOrEqual(18);
  expect(result.after).toEqual(result.before);
  expect(result.withOutlier.to).toBeGreaterThan(result.after.to);
  expect(result.withOutlier.to).toBeGreaterThanOrEqual(1000);
  for (const anchor of result.anchors) {
    expect(anchor.map(({ time }) => time)).toEqual(result.times);
    expect(anchor.every(({ hasValue }) => !hasValue)).toBe(true);
  }
});
