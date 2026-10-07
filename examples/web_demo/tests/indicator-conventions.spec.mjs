import { test, expect } from "@playwright/test";

// Indicator conventions through the public TypeScript API: whitespace-safe engine state, the
// China/TradingView presets, KDJ, the warm-up query, and the amount-weighted 分时 average price.

async function wait_grid(page) {
  await page.waitForFunction(() => window.__grid !== undefined && window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test("whitespace source rows never poison indicator outputs through the package API", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const rows = Array.from({ length: 48 }, (_, index) => ({
      time: 1_700_000_000 + index * 60,
      value: 100 + Math.sin(index * 0.45) * 5 + index * 0.08,
    }));
    // Row 2 falls inside the first KDJ window, where the China start uses the bars available.
    const blank = new Set([2, 6, 17, 18, 30]);
    const trailing = [48, 49, 50].map((index) => ({ time: 1_700_000_000 + index * 60 }));
    const build = (source) => [
      chart.add_sma(source, 4),
      chart.add_ema(source, 5),
      chart.add_rsi(source, 5),
      chart.add_rsi(source, 6, undefined, { convention: "china" }),
      ...chart.add_macd(source, 3, 6, 4, undefined, { convention: "china" }),
      ...chart.add_bollinger(source, 5, 2, undefined, { estimator: "sample" }),
      ...chart.add_kdj(source, 5, 3, 3),
      ...chart.add_kdj(source, 5, 3, 3, undefined, { convention: "china" }),
    ];

    // Whitespace present when the studies are created...
    const spaced = chart.add_series("line", { visible: false });
    spaced.set_data([...rows.map((row, index) => (blank.has(index) ? { time: row.time } : row)), ...trailing]);
    const spaced_outputs = build(spaced);
    // ...and arriving later through streaming updates, including trailing future slots.
    const streamed = chart.add_series("line", { visible: false });
    streamed.set_data(rows.slice(0, 5).map((row, index) => (blank.has(index) ? { time: row.time } : row)));
    const streamed_outputs = build(streamed);
    rows.slice(5).forEach((row, offset) => streamed.update(blank.has(offset + 5) ? { time: row.time } : row));
    for (const slot of trailing) streamed.update(slot);
    // The reference never sees the whitespace rows at all.
    const compact = chart.add_series("line", { visible: false });
    compact.set_data(rows.filter((_, index) => !blank.has(index)));
    const compact_outputs = build(compact);

    const blank_times = new Set([...[...blank].map((index) => rows[index].time), ...trailing.map((slot) => slot.time)]);
    const mismatches = [];
    const compare = (label, outputs) => outputs.forEach((output, index) => {
      const expected = new Map(compact_outputs[index].data().filter((point) => "value" in point).map((point) => [point.time, point.value]));
      for (const point of output.data()) {
        const has_value = "value" in point;
        if (blank_times.has(point.time)) {
          if (has_value) mismatches.push(`${label} ${index} whitespace ${point.time} = ${point.value}`);
        } else if (expected.has(point.time)) {
          if (!has_value || Math.abs(point.value - expected.get(point.time)) > 1e-9) {
            mismatches.push(`${label} ${index} ${point.time}: ${point.value} != ${expected.get(point.time)}`);
          }
        } else if (has_value) {
          mismatches.push(`${label} ${index} ${point.time}: unexpected ${point.value}`);
        }
      }
      const finite_tail = output.data().filter((point) => "value" in point).slice(-3);
      if (finite_tail.length !== 3 || !finite_tail.every((point) => Number.isFinite(point.value))) {
        mismatches.push(`${label} ${index}: poisoned tail`);
      }
    });
    compare("full", spaced_outputs);
    compare("streamed", streamed_outputs);
    for (const series of [spaced, streamed, compact]) chart.remove_series(series);
    return { mismatches, outputs: spaced_outputs.length };
  });
  expect(result.outputs).toBe(16);
  expect(result.mismatches).toEqual([]);
});

test("convention presets expand to explicit parameters, KDJ binds three outputs, and warm-up is queryable", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const rows = Array.from({ length: 60 }, (_, index) => ({
      time: 1_700_000_000 + index * 86_400,
      open: 100 + Math.sin(index * 0.3) * 4,
      high: 102 + Math.sin(index * 0.3) * 4 + (index % 3) * 0.2,
      low: 98 + Math.sin(index * 0.3) * 4 - (index % 4) * 0.2,
      close: 100 + Math.sin(index * 0.3 + 0.4) * 4,
    }));
    const source = chart.add_series("candlestick", { visible: false });
    source.set_data(rows);
    const tradingview = chart.add_macd(source, 12, 26, 9);
    const china = chart.add_macd(source, 12, 26, 9, undefined, { convention: "china" });
    const override = chart.add_macd(source, 12, 26, 9, undefined, { convention: "china", histogram_multiplier: 1 });
    const kdj = chart.add_kdj(source);
    const kdj_china = chart.add_kdj(source, 9, 3, 3, undefined, { convention: "china" });
    const chained = chart.add_sma(chart.add_rsi(source, 14), 5);
    let rejected = null;
    try {
      chart.add_macd(source, 12, 26, 9, undefined, { convention: "hk" });
    } catch (error) {
      rejected = error.code;
    }
    const info = (series) => series.indicator_info();
    const values = (series) => series.data().filter((point) => "value" in point);
    const summary = {
      tradingview: { parameters: info(tradingview[2]).parameters, first: values(tradingview[0]).length, warmup: info(tradingview[1]).warmup_bars, convergence: info(tradingview[1]).convergence_bars },
      china: { parameters: info(china[2]).parameters, first: values(china[0]).length, warmup: info(china[1]).warmup_bars },
      override_multiplier: info(override[2]).parameters.histogram_multiplier,
      override_seed: info(override[2]).parameters.seed,
      histogram_ratio: values(china[2]).at(-1).value / (values(china[0]).at(-1).value - values(china[1]).at(-1).value),
      kdj: kdj.map((series) => ({ name: info(series).output_name, rows: values(series).length, warmup: info(series).warmup_bars })),
      kdj_j: values(kdj[2]).at(-1).value - (3 * values(kdj[0]).at(-1).value - 2 * values(kdj[1]).at(-1).value),
      // The formula-language KDJ: RSV over the bars available so far, SMA started at its input.
      kdj_china: kdj_china.map((series) => ({ rows: values(series).length, warmup: info(series).warmup_bars, convergence: info(series).convergence_bars, seed: info(series).parameters.kdj_seed })),
      kdj_china_start: (() => {
        const rsv = (row, from) => {
          const window = rows.slice(from, row + 1);
          const high = Math.max(...window.map((bar) => bar.high));
          const low = Math.min(...window.map((bar) => bar.low));
          return (rows[row].close - low) / (high - low) * 100;
        };
        const k0 = rsv(0, 0);
        const k1 = (rsv(1, 0) + 2 * k0) / 3;
        const [k, d] = [values(kdj_china[0]), values(kdj_china[1])];
        return [k[0].value - k0, d[0].value - k0, k[1].value - k1, d[1].value - (k1 + 2 * k0) / 3];
      })(),
      kdj_schema: chart.indicator_schema("kdj", 9).outputs.map((output) => output.name),
      seed_schema: chart.indicator_schema("macd", 12).parameters.find((parameter) => parameter.name === "seed"),
      chained: { warmup: info(chained).warmup_bars, convergence: info(chained).convergence_bars },
      rejected,
      exported: JSON.stringify(chart.export_state()),
    };
    chart.remove_series(source);
    return summary;
  });
  expect(result.tradingview.parameters).toMatchObject({ seed: "sma", histogram_multiplier: 1 });
  expect(result.tradingview.first).toBe(60 - 25);
  expect(result.tradingview.warmup).toBe(33);
  expect(result.tradingview.convergence).toBe(154);
  expect(result.china.parameters).toMatchObject({ seed: "first_value", histogram_multiplier: 2 });
  expect(result.china.first).toBe(60);
  expect(result.china.warmup).toBe(0);
  expect(result.override_multiplier).toBe(1);
  expect(result.override_seed).toBe("first_value");
  expect(result.histogram_ratio).toBeCloseTo(2, 9);
  expect(result.kdj.map((output) => output.name)).toEqual(["K", "D", "J"]);
  expect(result.kdj.every((output) => output.rows === 52 && output.warmup === 8)).toBe(true);
  expect(Math.abs(result.kdj_j)).toBeLessThan(1e-9);
  expect(result.kdj_china).toEqual([
    { rows: 60, warmup: 0, convergence: 26, seed: "first_value" },
    { rows: 60, warmup: 0, convergence: 44, seed: "first_value" },
    { rows: 60, warmup: 0, convergence: 44, seed: "first_value" },
  ]);
  expect(result.kdj_china_start.every((error) => Math.abs(error) < 1e-9)).toBe(true);
  expect(result.kdj_schema).toEqual(["K", "D", "J"]);
  expect(result.seed_schema).toMatchObject({ parameter_type: "choice", default: "sma", options: ["sma", "first_value"] });
  expect(result.chained).toEqual({ warmup: 18, convergence: 112 });
  expect(result.rejected).toBe("invalid_options");
  // Persistence carries the expanded parameters, never the preset name.
  expect(result.exported).toContain("\"first_value\"");
  expect(result.exported).not.toContain("china");
});

test("VWAP with an amount source is the 分时 average price and skips empty minutes", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const start = 1_700_006_400;
    const minutes = [0, 1, 2, 3, 4].map((index) => start + index * 60);
    const price = chart.add_series("line", { visible: false });
    price.set_data([
      { time: minutes[0], value: 10 },
      { time: minutes[1], value: 11 },
      { time: minutes[2] },
      { time: minutes[3], value: 12 },
      { time: minutes[4], value: 13 },
    ]);
    // The weight columns also carry the previous session's last minutes, so their row indices
    // run ahead of the price rows; live updates must still land on the matching timestamp.
    const volume = chart.add_series("histogram", { visible: false });
    volume.set_data([
      { time: start - 120, value: 1 },
      { time: start - 60, value: 1 },
      { time: minutes[0], value: 10 },
      { time: minutes[1], value: 0 },
      { time: minutes[3], value: 30 },
      { time: minutes[4], value: 20 },
    ]);
    const amount = chart.add_series("line", { visible: false });
    amount.set_data([
      { time: start - 120, value: 9 },
      { time: start - 60, value: 9 },
      { time: minutes[0], value: 100 },
      { time: minutes[1], value: 0 },
      { time: minutes[3], value: 330 },
      { time: minutes[4], value: 270 },
    ]);
    const average = chart.add_vwap(price, volume, undefined, { amount_source: amount });
    const before = average.data().map((point) => point.value ?? null);
    amount.update({ time: minutes[4], value: 290 });
    const after = average.data().at(-1).value;
    const info = average.indicator_info();
    const linked = { amount: info.amount_source?.id === amount.id, volume: info.volume_source?.id === volume.id };
    let rejected = null;
    try {
      chart.add_vwap(price, null, undefined, { amount_source: amount });
    } catch (error) {
      rejected = error.code;
    }
    chart.remove_series(amount);
    const removed = average.indicator_info() === null;
    for (const series of [price, volume]) chart.remove_series(series);
    return { before, after, linked, rejected, removed };
  });
  // 100/10, zero-volume minute skipped, whitespace minute blank, (100+330)/40, (430+270)/60.
  expect(result.before).toEqual([10, 10, null, 10.75, 700 / 60]);
  expect(result.after).toBeCloseTo(720 / 60, 12);
  expect(result.linked).toEqual({ amount: true, volume: true });
  expect(result.rejected).toBe("invalid_options");
  expect(result.removed).toBe(true);
});
