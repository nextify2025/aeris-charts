import { test, expect } from "@playwright/test";

// Price-axis semantics through the public browser API: min-move tick grids, tick-size ladders,
// explicit percentage bases, symmetric autoscale, and stable autoscale during horizontal pans.
// Axis text is read from the chart's own Canvas2D `fillText` calls.

const HKEX_LADDER = [
  [0, 0.001], [0.25, 0.005], [0.5, 0.01], [10, 0.02], [20, 0.05], [100, 0.1],
  [200, 0.2], [500, 0.5], [1000, 1], [2000, 2], [5000, 5],
].map(([from, min_move]) => ({ from, min_move }));

async function open_page(page) {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
}

/**
 * Create an isolated Canvas2D chart with one candlestick series. `bars` are `[open, high, low,
 * close]` rows. `series_options` and `scale_options` apply after the first settled frame, and the
 * axis text of the frame they trigger is returned.
 */
async function axis_fixture(page, { bars, series_options = {}, scale_options = {} }) {
  return page.evaluate(async ({ bars, series_options, scale_options }) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const frames = () => new Promise((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(resolve));
    });
    window.__price_axis?.chart.remove();
    window.__price_axis = undefined;
    window.__price_axis_host?.remove();
    const host = document.createElement("div");
    host.style.cssText = "width:640px;height:420px;position:absolute;left:0;top:0;z-index:100";
    document.body.append(host);
    window.__price_axis_host = host;
    const texts = [];
    const prototype = CanvasRenderingContext2D.prototype;
    if (window.__price_axis_restore === undefined) {
      const original = prototype.fillText;
      prototype.fillText = function (text, ...args) {
        if (window.__price_axis_host?.contains(this.canvas)) window.__price_axis_texts?.push(String(text));
        return original.call(this, text, ...args);
      };
      window.__price_axis_restore = () => { prototype.fillText = original; };
    }
    window.__price_axis_texts = texts;
    const chart = await create_chart(host, {
      backend: "canvas2d",
      autoSize: false,
      width: 640,
      height: 420,
      accessibility: false,
    });
    const series = chart.add_series("candlestick", {
      last_value_visible: false,
      price_line_visible: false,
    });
    const day = 86_400;
    series.set_data(bars.map(([open, high, low, close], index) => ({
      time: 1_704_067_200 + index * day,
      open,
      high,
      low,
      close,
    })));
    chart.time_scale().fit_content();
    await frames();
    // The option writes invalidate the axis, so the next settled frame repaints every label.
    texts.length = 0;
    if (Object.keys(series_options).length > 0) series.apply_options(series_options);
    if (Object.keys(scale_options).length > 0) chart.price_scale("right").apply_options(scale_options);
    await frames();
    window.__price_axis = { chart, series, frames };
    const price_texts = texts.filter((text) => /^[+−-]?[\d,]+\.\d+%?$/.test(text));
    return {
      texts: price_texts,
      range: chart.price_scale("right").get_visible_range(),
      options: chart.price_scale("right").options(),
    };
  }, { bars, series_options, scale_options });
}

const parse = (text) => Number(text.replace(/,/g, "").replace("−", "-").replace("%", ""));
const on_grid = (value, step) => Math.abs(value / step - Math.round(value / step)) < 1e-6;

function hk_bars(start, step, count) {
  return Array.from({ length: count }, (_, index) => {
    const close = start + index * step + 0.13 * Math.sin(index);
    return [close - step, close + 0.07, close - 0.09, close];
  });
}

test.afterEach(async ({ page }) => {
  await page.evaluate(() => {
    window.__price_axis?.chart.remove();
    window.__price_axis = undefined;
    window.__price_axis_host?.remove();
    window.__price_axis_restore?.();
    window.__price_axis_restore = undefined;
  });
});

test("a 0.02 minimum move keeps every tick label on the tradable grid", async ({ page }) => {
  await open_page(page);
  for (const count of [30, 55]) {
    const result = await axis_fixture(page, {
      bars: hk_bars(14.2, 1.43 / count, count),
      series_options: { price_format: { type: "price", min_move: 0.02 } },
    });
    expect(result.texts.length).toBeGreaterThanOrEqual(3);
    for (const text of result.texts) {
      expect(on_grid(parse(text), 0.02), `off-grid tick label ${text} in ${result.texts}`).toBe(true);
      expect(text.split(".")[1]).toHaveLength(2);
    }
    expect(new Set(result.texts).size).toBe(result.texts.length);
  }
});

test("an HKEX tick ladder rounds labels per band across the HK$10 boundary", async ({ page }) => {
  await open_page(page);
  const bars = hk_bars(9.72, 0.016, 40);
  const result = await axis_fixture(page, {
    bars,
    series_options: { price_format: { type: "price", tick_ladder: HKEX_LADDER } },
  });
  const values = result.texts.map(parse);
  expect(values.some((value) => value < 10)).toBe(true);
  expect(values.some((value) => value > 10)).toBe(true);
  for (const value of values) expect(on_grid(value, 0.02), `${result.texts}`).toBe(true);

  // The last-value label rounds the final close to its own band tick.
  const last_close = bars.at(-1)[3];
  const label = await page.evaluate(async () => {
    const { series, frames } = window.__price_axis;
    window.__price_axis_texts.length = 0;
    series.apply_options({ last_value_visible: true });
    await frames();
    return window.__price_axis_texts.slice();
  });
  const expected = (Math.round(last_close / 0.02) * 0.02).toFixed(2);
  expect(last_close).toBeGreaterThan(10);
  expect(label).toContain(expected);
  const options = await page.evaluate(() => window.__price_axis.series.options().price_format);
  expect(options.tick_ladder).toHaveLength(11);
});

test("an explicit percentage base holds the axis steady while panning", async ({ page }) => {
  await open_page(page);
  const bars = Array.from({ length: 60 }, (_, index) => {
    const close = 102 + 5 * Math.sin(index / 6);
    return [close - 0.5, close + 1, close - 1, close];
  });
  const result = await axis_fixture(page, {
    bars,
    scale_options: { mode: 2, base_value: 100 },
  });
  expect(result.options.base_value).toBe(100);
  expect(result.texts.length).toBeGreaterThanOrEqual(3);
  expect(result.texts.every((text) => text.endsWith("%"))).toBe(true);
  // With every bar in view the percentage range is exactly the data against the 100 base.
  const lowest = Math.min(...bars.map((bar) => bar[2]));
  const highest = Math.max(...bars.map((bar) => bar[1]));
  expect(result.range.from).toBeCloseTo(lowest - 100, 9);
  expect(result.range.to).toBeCloseTo(highest - 100, 9);
  const ranges = await page.evaluate(async () => {
    const { chart, frames } = window.__price_axis;
    chart.time_scale().apply_options({ bar_spacing: 20 });
    const out = [];
    for (const position of [-5, -12, -20, -27]) {
      chart.time_scale().scroll_to_position(position, false);
      await frames();
      out.push(chart.price_scale("right").get_visible_range());
    }
    return out;
  });
  // Panning changes which bars are visible but never the base: every range stays inside the
  // full data range measured against 100 (a first-visible base would re-scale it).
  expect(new Set(ranges.map((range) => `${range.from}:${range.to}`)).size).toBeGreaterThan(1);
  for (const range of ranges) {
    expect(range.from).toBeGreaterThanOrEqual(lowest - 100 - 1e-9);
    expect(range.to).toBeLessThanOrEqual(highest - 100 + 1e-9);
  }
});

test("symmetric autoscale centers the range on the previous close", async ({ page }) => {
  await open_page(page);
  const bars = Array.from({ length: 30 }, (_, index) => {
    const close = 101 + 3 * (index / 29);
    return [close - 0.2, close + 0.4, close - 0.4, close];
  });
  const normal = await axis_fixture(page, {
    bars,
    scale_options: { autoscale_center: 100 },
  });
  expect((normal.range.from + normal.range.to) / 2).toBeCloseTo(100, 9);
  expect(normal.range.to).toBeGreaterThanOrEqual(104.4 - 1e-9);
  await page.evaluate(() => {
    window.__price_axis.chart.remove();
    window.__price_axis = undefined;
  });
  const percent = await axis_fixture(page, {
    bars,
    scale_options: { mode: 2, base_value: 100, autoscale_center: 100 },
  });
  expect((percent.range.from + percent.range.to) / 2).toBeCloseTo(0, 9);
});

test("stable autoscale does not flip on sub-bar pans across a spike", async ({ page }) => {
  await open_page(page);
  const bars = Array.from({ length: 80 }, (_, index) => {
    const base = 100 + (index % 3);
    return [base, index === 50 ? 140 : base + 1, base - 1, base];
  });
  const pan_ranges = async (stable) => {
    await page.evaluate(() => {
      window.__price_axis?.chart.remove();
      window.__price_axis = undefined;
    });
    await axis_fixture(page, {
      bars,
      scale_options: { stable_auto_scale: stable },
    });
    return page.evaluate(async () => {
      const { chart, frames } = window.__price_axis;
      chart.time_scale().apply_options({ bar_spacing: 20, right_offset: 0 });
      await frames();
      const distinct = [];
      for (let step = 0; step < 16; step += 1) {
        // Right edge alternates between 48.8 (spike bar excluded) and 49.2 (spike bar enters).
        const right = (step % 2 === 0 ? 48.8 : 49.2) + Math.floor(step / 2) * 0.01;
        chart.time_scale().scroll_to_position(right - 79, false);
        await frames();
        const range = chart.price_scale("right").get_visible_range();
        const last = distinct.at(-1);
        if (last === undefined || last.from !== range.from || last.to !== range.to) distinct.push(range);
      }
      return distinct;
    });
  };
  const exact = await pan_ranges(false);
  expect(exact.length, "reference-exact autoscale flips with the edge bar").toBeGreaterThan(8);
  const stable = await pan_ranges(true);
  expect(stable.length, JSON.stringify(stable)).toBeLessThanOrEqual(2);
  expect(stable.at(-1).to).toBeGreaterThanOrEqual(140);
});

test("autoscale_info_provider replaces the series range", async ({ page }) => {
  await open_page(page);
  const bars = Array.from({ length: 30 }, (_, index) => [105, 106 + (index % 2), 104, 105]);
  await axis_fixture(page, { bars });
  const result = await page.evaluate(async () => {
    const { chart, series, frames } = window.__price_axis;
    let seen = null;
    series.apply_options({
      autoscale_info_provider: (original) => {
        seen = original();
        return { price_range: { min_value: 0, max_value: 200 } };
      },
    });
    await frames();
    const replaced = chart.price_scale("right").get_visible_range();
    series.apply_options({ autoscale_info_provider: null });
    await frames();
    return { seen, replaced, restored: chart.price_scale("right").get_visible_range() };
  });
  expect(result.seen.price_range.min_value).toBe(104);
  expect(result.seen.price_range.max_value).toBe(107);
  expect(result.replaced).toEqual({ from: 0, to: 200 });
  expect(result.restored).toEqual({ from: 104, to: 107 });
});

test("malformed ladders and price-scale extension values are rejected atomically", async ({ page }) => {
  await open_page(page);
  await axis_fixture(page, { bars: hk_bars(14.2, 0.05, 30) });
  const result = await page.evaluate(() => {
    const { chart, series } = window.__price_axis;
    const code = (apply) => {
      try {
        apply();
        return null;
      } catch (error) {
        return error.code ?? String(error);
      }
    };
    const scale = chart.price_scale("right");
    series.apply_options({ price_format: { type: "price", min_move: 0.02 } });
    const before = JSON.stringify(series.options().price_format);
    return {
      descending: code(() => series.apply_options({
        price_format: { type: "price", tick_ladder: [{ from: 1, min_move: 0.01 }, { from: 0.5, min_move: 0.01 }] },
      })),
      off_grid: code(() => series.apply_options({
        price_format: { type: "price", min_move: 0.05, tick_ladder: [{ from: 0, min_move: 0.02 }, { from: 10.01, min_move: 0.05 }] },
      })),
      format_kept: JSON.stringify(series.options().price_format) === before,
      provider: code(() => series.apply_options({ autoscale_info_provider: 5 })),
      zero_base: code(() => scale.apply_options({ base_value: 0 })),
      density: code(() => scale.apply_options({ tick_mark_density: -1 })),
      center: code(() => scale.apply_options({ autoscale_center: Number.NaN })),
      stable: code(() => scale.apply_options({ stable_auto_scale: "yes" })),
      scale_kept: scale.options().base_value === null && scale.options().tick_mark_density === 2.5,
      cleared: code(() => scale.apply_options({ base_value: null, autoscale_center: null })),
    };
  });
  expect(result).toEqual({
    descending: "invalid_options",
    off_grid: "invalid_options",
    format_kept: true,
    provider: "invalid_options",
    zero_base: "invalid_options",
    density: "invalid_options",
    center: "invalid_options",
    stable: "invalid_options",
    scale_kept: true,
    cleared: null,
  });
});
