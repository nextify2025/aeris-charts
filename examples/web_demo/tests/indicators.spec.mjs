import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

// Engine-native indicators: Bollinger band fill, oscillator separate panes with channel strips,
// MACD four-state histogram colors, and the full native set's placement/lineage.

async function wait_grid(page) {
  await page.waitForFunction(() => window.__grid !== undefined && window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

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
