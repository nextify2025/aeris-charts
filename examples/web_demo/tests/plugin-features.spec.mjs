import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

function count_near(image, expected, tolerance = 10) {
  let count = 0;
  for (let offset = 0; offset < image.data.length; offset += 4) {
    if (
      Math.abs(image.data[offset] - expected[0]) <= tolerance
      && Math.abs(image.data[offset + 1] - expected[1]) <= tolerance
      && Math.abs(image.data[offset + 2] - expected[2]) <= tolerance
      && image.data[offset + 3] > 200
    ) count += 1;
  }
  return count;
}

function count_near_point(image, point, viewport, expected, tolerance = 10, radius = 7) {
  const scale_x = image.width / viewport.width;
  const scale_y = image.height / viewport.height;
  const center_x = Math.round(point.x * scale_x);
  const center_y = Math.round(point.y * scale_y);
  const pixel_radius = Math.ceil(radius * Math.max(scale_x, scale_y));
  let count = 0;
  for (let y = Math.max(0, center_y - pixel_radius); y <= Math.min(image.height - 1, center_y + pixel_radius); y += 1) {
    for (let x = Math.max(0, center_x - pixel_radius); x <= Math.min(image.width - 1, center_x + pixel_radius); x += 1) {
      const offset = (y * image.width + x) * 4;
      if (
        Math.abs(image.data[offset] - expected[0]) <= tolerance
        && Math.abs(image.data[offset + 1] - expected[1]) <= tolerance
        && Math.abs(image.data[offset + 2] - expected[2]) <= tolerance
        && image.data[offset + 3] > 200
      ) count += 1;
    }
  }
  return count;
}

function pixel_at(buffer, point, viewport) {
  const image = PNG.sync.read(buffer);
  const x = Math.round(point.x * image.width / viewport.width);
  const y = Math.round(point.y * image.height / viewport.height);
  const offset = (y * image.width + x) * 4;
  return [...image.data.subarray(offset, offset + 3)];
}

async function open_chart(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("all advanced series render through the shared Rust engine", async ({ page }) => {
  const warnings = [];
  page.on("console", (message) => {
    if (message.type() === "warning") warnings.push(message.text());
  });
  await open_chart(page);
  const before = await page.screenshot();
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const bars = window.__data.slice(0, 20);
    const definitions = [
      ["grouped_bars", {}, bars.map((bar, index) => ({ time: bar.time, values: [index + 1, index + 3, index + 2] }))],
      ["heatmap", {}, bars.map((bar, index) => ({ time: bar.time, cells: [{ low: bar.low, high: bar.close, amount: index * 5 }, { low: bar.close, high: bar.high, amount: 100 - index * 5 }] }))],
      ["hlc_area", {}, bars.map((bar) => ({ time: bar.time, high: bar.high, low: bar.low, close: bar.close }))],
      ["pretty_histogram", { base_price: 90 }, bars.map((bar) => ({ time: bar.time, value: bar.close }))],
      ["background_shade", { low_value: 90, high_value: 120 }, bars.map((bar) => ({ time: bar.time, value: bar.close }))],
      ["stacked_area", {}, bars.map((bar, index) => ({ time: bar.time, values: [10 + index, 5 + index / 2, 3] }))],
      ["stacked_bars", {}, bars.map((bar, index) => ({ time: bar.time, values: [10 + index, 5, 3] }))],
      ["whisker_box", {}, bars.map((bar) => ({ time: bar.time, quartiles: [bar.low - 1, bar.low, bar.close, bar.high, bar.high + 1], outliers: [bar.high + 2] }))],
    ];
    const series = definitions.map(([kind, options, data]) => {
      const feature = window.__chart.add_series(kind, { ...options, price_line_visible: false, last_value_visible: false });
      feature.set_data(data);
      return feature;
    });
    window.__chart.time_scale().fit_content();
    window.__chart.render();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return { count: series.length, kinds: series.map((item) => item.series_type()) };
  });
  const after = await page.screenshot();
  expect(result.count).toBe(8);
  expect(new Set(result.kinds)).toEqual(new Set([
    "grouped_bars", "heatmap", "hlc_area",
    "pretty_histogram", "background_shade", "stacked_area",
    "stacked_bars", "whisker_box",
  ]));
  expect(after.equals(before)).toBe(false);
  expect(warnings.filter((warning) => warning.includes("advanced series") || warning.includes("skipped"))).toEqual([]);
});

test("advanced-series data, updates, options, and diagnostics round-trip from Rust", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const feature = window.__chart.add_series("grouped_bars", {
      colors: ["#2962ff", "#e1575a"],
      price_line_visible: false,
    });
    feature.set_data([
      { time: 3, values: [3, 6] },
      { time: 1, values: [1, 2] },
      { time: 3, values: [30, 60] },
      { time: 2 },
    ]);
    const replace_diagnostics = feature.last_ingestion_diagnostics();
    feature.update({ time: 2, values: [20, 40] });
    const after_update = feature.data();
    const nearest = feature.data_by_index(1, 1);
    feature.apply_options({ colors: ["#089981", "#f23645"] });
    const options = feature.options();
    feature.update({ time: Number.NaN, values: [1, 2] });
    const invalid = feature.last_ingestion_diagnostics();
    return {
      type: feature.series_type(),
      replace_diagnostics,
      after_update,
      nearest,
      colors: options.colors,
      invalid,
    };
  });
  expect(result.type).toBe("grouped_bars");
  expect(result.replace_diagnostics).toMatchObject({ accepted: 3, deduplicated: 1, reordered: true });
  expect(result.after_update).toEqual([
    { time: 1, values: [1, 2] },
    { time: 2, values: [20, 40] },
    { time: 3, values: [30, 60] },
  ]);
  expect(result.nearest).toEqual({ time: 2, values: [20, 40] });
  expect(result.colors).toEqual(["#089981", "#f23645"]);
  expect(result.invalid).toMatchObject({ status: "rejected", accepted: 0, dropped_invalid: 1 });
});

test("heatmap cell_shader resolves colors at the host boundary and keeps data engine-owned", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    let initial_calls = 0;
    let replacement_calls = 0;
    const heatmap = window.__chart.add_series("heatmap", {
      cell_shader: () => {
        initial_calls += 1;
        return "#0c2238";
      },
    });
    heatmap.set_data([
      { time: 1, cells: [{ low: 10, high: 11, amount: 25 }, { low: 11, high: 12, amount: 75 }] },
    ]);
    const data = heatmap.data();
    heatmap.apply_options({
      cell_shader: () => {
        replacement_calls += 1;
        return "rgba(155,0,255,.8)";
      },
    });
    return {
      initial_calls,
      replacement_calls,
      has_shader: typeof heatmap.options().cell_shader === "function",
      data,
    };
  });
  expect(result).toMatchObject({ initial_calls: 2, replacement_calls: 2, has_shader: true });
  expect(result.data).toEqual([
    { time: 1, cells: [{ low: 10, high: 11, amount: 25 }, { low: 11, high: 12, amount: 75 }] },
  ]);
});

test("primitive feature helpers compose existing engine and host boundaries", async ({ page }) => {
  const page_errors = [];
  page.on("pageerror", (error) => page_errors.push(error.message));
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const series = window.__main;
    const bars = window.__data;
    const start = bars[10];
    const end = bars[30];
    const handles = [];

    const anchored = api.create_anchored_text(series, { text: "Anchor", font: "bold 20px Arial" });
    anchored.apply_options({ text: "Updated anchor", horz_align: "right", vert_align: "bottom" });
    handles.push(anchored);
    api.create_rectangle_drawing(chart, [{ logical: 14, price: start.low }, { logical: 20, price: end.high }]);
    handles.push(api.create_trend_line(series, [{ time: start.time, price: start.low }, { time: end.time, price: end.high }]));
    handles.push(api.create_vertical_line(series, bars[24].time, { label_text: "Event", show_label: true }));
    api.create_user_price_line(series, { price: start.close, title: "Level" });
    handles.push(api.create_partial_price_line(series));
    handles.push(api.create_session_highlighting(series, { start_hour_utc: 0, end_hour_utc: 24 }));
    handles.push(api.create_highlight_bar_crosshair(chart, series));
    const profile = bars.slice(0, 15).map((bar, index) => ({ price: bar.close, vol: index + 1 }));
    const volume_profile = api.create_volume_profile(series, { time: start.time, profile, width: 10 });
    volume_profile.set_data({ time: start.time, profile: profile.map((point) => ({ ...point, vol: point.vol + 1 })), width: 12 });
    handles.push(volume_profile);
    handles.push(api.create_bands_indicator(series));

    const overlay = chart.add_series("line", { price_scale_id: "" });
    overlay.set_data(bars.slice(0, 20).map((bar) => ({ time: bar.time, value: bar.close })));
    const overlay_labels = api.create_overlay_price_scale(overlay);
    const overlay_scale = overlay_labels.price_scale();
    handles.push(overlay_labels);

    const image = document.createElement("canvas");
    image.width = 16;
    image.height = 16;
    image.getContext("2d").fillRect(0, 0, 16, 16);
    handles.push(api.create_image_watermark(series, image.toDataURL(), { maxWidth: 32, maxHeight: 32, alpha: 0.4 }));
    handles.push(api.create_tooltip(chart));
    handles.push(api.create_delta_tooltip(chart, { series: overlay }));

    const a11y = api.enable_accessibility(chart, {
      chart_title: "Test financial chart",
      announce_data_updates: "active",
    });
    a11y.focus();
    document.activeElement.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    chart.set_crosshair_position(start.close, start.time, series);
    chart.render();
    await new Promise((resolve) => setTimeout(resolve, 40));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

    const state = {
      drawings: chart.drawings().map((drawing) => drawing.kind()),
      overlay_has_range: overlay_scale.get_visible_range() !== null,
      tooltips: chart.chart_element().querySelectorAll(".aeris_charts-tooltip").length,
      role: chart.chart_element().querySelector(".aeris_charts-a11y-layer")?.getAttribute("role"),
      label: chart.chart_element().querySelector(".aeris_charts-a11y-layer")?.getAttribute("aria-label"),
      announcement: chart.chart_element().querySelector(".aeris_charts-a11y-live-region")?.textContent ?? "",
    };
    a11y.detach();
    handles.forEach((handle) => handle.detach());
    return state;
  });
  expect(result.drawings).toEqual(expect.arrayContaining(["rectangle"]));
  expect(result.overlay_has_range).toBe(true);
  expect(result.tooltips).toBe(1);
  expect(result.role).toBe("application");
  expect(result.label).toContain("Test financial chart");
  expect(result.announcement).toContain("Point");
  expect(page_errors).toEqual([]);
});

test("legacy brushable_area input normalizes to the built-in Area series", async ({ page }) => {
  await open_chart(page);
  expect(await page.evaluate(() => {
    const legacy = window.__chart.add_series("brushable_area", {
      price_line_visible: false,
      last_value_visible: false,
    });
    legacy.set_data(window.__data.slice(0, 20).map((bar) => ({ time: bar.time, value: bar.close })));
    return {
      kind: legacy.series_type(),
      data: legacy.data().length,
    };
  })).toEqual({ kind: "area", data: 20 });
});

test("Delta Tooltip rejects candlesticks, normal Tooltip still works, and type conversion removes Delta Tooltip", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const candles = window.__main;
    let candle_error = "";
    try {
      api.create_delta_tooltip(chart, { series: candles });
    } catch (error) {
      candle_error = String(error?.message ?? error);
    }

    const tooltip = api.create_tooltip(chart, { series: candles });
    tooltip.detach();

    const area = chart.add_series("area", {
      price_line_visible: false,
      last_value_visible: false,
      countdown_visible: false,
    });
    area.set_data(window.__data.slice(0, 40).map((bar) => ({ time: bar.time, value: bar.close })));
    const delta = api.create_delta_tooltip(chart, { series: area });
    const active_before_conversion = chart.wasm.native_delta_tooltip_active();

    area.set_type("candlestick");
    const active_after_conversion = chart.wasm.native_delta_tooltip_active();
    const range_after_conversion = delta.active_range();

    let converted_error = "";
    try {
      api.create_delta_tooltip(chart, { series: area });
    } catch (error) {
      converted_error = String(error?.message ?? error);
    }
    delta.detach();

    return {
      candle_error,
      converted_error,
      active_before_conversion,
      active_after_conversion,
      range_after_conversion,
    };
  });

  expect(result.candle_error).toContain("not supported on candlestick series");
  expect(result.converted_error).toContain("not supported on candlestick series");
  expect(result.active_before_conversion).toBe(true);
  expect(result.active_after_conversion).toBe(false);
  expect(result.range_after_conversion).toBe(null);
});

test("bar highlight default follows dark and light chart surfaces", async ({ page }) => {
  await open_chart(page);
  const point = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.apply_options(api.theme_options("dark"));
    window.__bar_highlight = api.create_highlight_bar_crosshair(chart, window.__main);
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const range = chart.time_scale().get_visible_logical_range();
    const logical = Math.round((range.from + range.to) * 0.5);
    return {
      x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(logical),
      y: bounds.top + pane.top + 12,
    };
  });
  const viewport = page.viewportSize();

  await page.evaluate(() => window.__chart.clear_crosshair_position());
  await page.waitForTimeout(40);
  const dark_base = pixel_at(await page.screenshot(), point, viewport);
  await page.mouse.move(point.x, point.y);
  await page.waitForTimeout(40);
  const dark_highlight = pixel_at(await page.screenshot(), point, viewport);
  expect(dark_highlight[0] + dark_highlight[1] + dark_highlight[2])
    .toBeGreaterThan(dark_base[0] + dark_base[1] + dark_base[2] + 30);

  await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    window.__chart.apply_options(api.theme_options("light"));
    window.__chart.clear_crosshair_position();
  });
  await page.waitForTimeout(40);
  const light_base = pixel_at(await page.screenshot(), point, viewport);
  await page.mouse.move(point.x + 1, point.y);
  await page.waitForTimeout(40);
  const light_highlight = pixel_at(await page.screenshot(), point, viewport);
  expect(light_highlight[0] + light_highlight[1] + light_highlight[2])
    .toBeLessThan(light_base[0] + light_base[1] + light_base[2] - 60);

  await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    window.__bar_highlight.detach();
    window.__chart.apply_options(api.theme_options("dark"));
    window.__chart.clear_crosshair_position();
    window.__explicit_bar_highlight = api.create_highlight_bar_crosshair(
      window.__chart,
      window.__main,
      { color: "#ff0000" },
    );
  });
  await page.mouse.move(point.x, point.y);
  await page.waitForTimeout(40);
  expect(count_near_point(PNG.sync.read(await page.screenshot()), point, viewport, [255, 0, 0]))
    .toBeGreaterThan(5);
  await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    window.__chart.apply_options(api.theme_options("light"));
  });
  await page.waitForTimeout(40);
  expect(count_near_point(PNG.sync.read(await page.screenshot()), point, viewport, [255, 0, 0]))
    .toBeGreaterThan(5);
  await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    window.__chart.remove_series(window.__main);
    window.__chart.apply_options(api.theme_options("dark"));
    window.__explicit_bar_highlight.detach();
  });
});

test("rectangle tool uses official two-click preview, data-time snapping, and engine axis views", async ({ page }) => {
  await open_chart(page);
  const setup = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const series = window.__main;
    series.apply_options({ price_scale_id: "left" });
    chart.apply_options({
      leftPriceScale: { visible: true },
      rightPriceScale: { visible: false },
    });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const toolbar = document.createElement("div");
    toolbar.id = "rectangle-test-toolbar";
    document.body.appendChild(toolbar);
    window.__rectangle_tool = api.create_rectangle_drawing_tool(chart, series, toolbar);
    window.__rectangle_tool.start_drawing();
    const range = chart.time_scale().get_visible_logical_range();
    const first_logical = Math.ceil(range.from + (range.to - range.from) * 0.3);
    const second_logical = Math.floor(range.from + (range.to - range.from) * 0.7);
    const spacing = chart.time_scale().logical_to_coordinate(first_logical + 1) - chart.time_scale().logical_to_coordinate(first_logical);
    const first = series.data_by_index(first_logical);
    const second = series.data_by_index(second_logical);
    const pane_offset = chart.price_scale("left").width();
    return {
      first: {
        x: pane_offset + chart.time_scale().logical_to_coordinate(first_logical) + spacing * 0.36,
        y: series.price_to_coordinate(first.low),
      },
      second: {
        x: pane_offset + chart.time_scale().logical_to_coordinate(second_logical) + spacing * 0.41,
        y: series.price_to_coordinate(second.high),
      },
      toolbar_children: toolbar.childElementCount,
      logicals: [first_logical, second_logical],
    };
  });
  expect(setup.toolbar_children).toBe(2);
  await page.mouse.click(setup.first.x, setup.first.y);
  await page.mouse.move(setup.second.x, setup.second.y);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));

  const preview = await page.evaluate(() => {
    const chart = window.__chart;
    const canvas = chart.take_screenshot();
    const ctx = canvas.getContext("2d");
    let band_pixels = 0;
    const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let index = 0; index < pixels.length; index += 4) {
      if (Math.abs(pixels[index] - 197) <= 3 && Math.abs(pixels[index + 1] - 227) <= 3 && Math.abs(pixels[index + 2] - 253) <= 3 && pixels[index + 3] === 255) {
        band_pixels += 1;
      }
    }
    return {
      drawing_count: chart.drawings().length,
      active: window.__rectangle_tool.is_drawing(),
      pending: chart.creation_active(),
      band_pixels,
    };
  });
  expect(preview.drawing_count).toBe(0);
  expect(preview.active).toBe(true);
  expect(preview.pending).toBe(true);
  expect(preview.band_pixels).toBeGreaterThan(20);

  await page.mouse.click(setup.second.x, setup.second.y);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const committed = await page.evaluate(() => {
    const chart = window.__chart;
    const drawing = chart.drawings()[0];
    const canvas = chart.take_screenshot();
    const ctx = canvas.getContext("2d");
    const options = drawing.options();
    const points = drawing.points();
    let label_pixels = 0;
    let band_pixels = 0;
    const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
    for (let index = 0; index < pixels.length; index += 4) {
      if (Math.abs(pixels[index] - 22) <= 3 && Math.abs(pixels[index + 1] - 142) <= 3 && Math.abs(pixels[index + 2] - 247) <= 3 && pixels[index + 3] === 255) {
        label_pixels += 1;
      }
      if (Math.abs(pixels[index] - 197) <= 3 && Math.abs(pixels[index + 1] - 227) <= 3 && Math.abs(pixels[index + 2] - 253) <= 3 && pixels[index + 3] === 255) {
        band_pixels += 1;
      }
    }
    return {
      count: chart.drawings().length,
      active: window.__rectangle_tool.is_drawing(),
      points,
      options,
      label_pixels,
      band_pixels,
    };
  });
  expect(committed.count).toBe(1);
  expect(committed.active).toBe(false);
  expect(committed.points.map((point) => point.logical)).toEqual(setup.logicals);
  expect(committed.options).toMatchObject({
    price_scale_id: "left",
    fill_color: "rgba(200, 50, 100, 0.75)",
    preview_fill_color: "rgba(200, 50, 100, 0.25)",
    border_visible: false,
    show_labels: false,
    axis_bands_visible: false,
    snap_time_to_data: true,
  });
  expect(committed.band_pixels).toBeGreaterThan(20);
  expect(committed.label_pixels).toBeGreaterThan(20);

  const removed = await page.evaluate(() => {
    window.__rectangle_tool.remove();
    return {
      drawings: window.__chart.drawings().length,
      toolbar: document.querySelector("#rectangle-test-toolbar").childElementCount,
    };
  });
  expect(removed).toEqual({ drawings: 0, toolbar: 0 });
});

test("session highlighting follows the official callback contract and refreshes on source data", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const series = window.__main;
    const source_count = series.data().length;
    let calls = 0;
    let last_type = "";
    const handle = api.create_session_highlighting(series, (time) => {
      calls += 1;
      last_type = typeof time;
      return calls % 2 === 0 ? "rgba(1, 2, 3, 0.2)" : "rgba(4, 5, 6, 0.3)";
    });
    const initial_calls = calls;
    const last = series.data().at(-1);
    series.update({
      time: last.time + 86_400,
      open: last.close,
      high: last.close + 2,
      low: last.close - 2,
      close: last.close + 1,
    });
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const update_calls = calls;
    handle.detach();
    series.update({
      time: last.time + 2 * 86_400,
      open: last.close + 1,
      high: last.close + 3,
      low: last.close - 1,
      close: last.close + 2,
    });
    return { source_count, initial_calls, update_calls, detached_calls: calls, last_type };
  });
  expect(result.initial_calls).toBe(result.source_count);
  // A live update evaluates only the appended row. The callback used to re-run over the whole
  // series on every update (initial + source_count + 1), which made each tick O(n).
  expect(result.update_calls).toBe(result.initial_calls + 1);
  expect(result.detached_calls).toBe(result.update_calls);
  expect(result.last_type).toBe("number");
});

test("accessibility provides per-pane semantics, official keyboard help, summaries, updates, and engine focus", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const series = window.__main;
    const before_canvas = [...chart.chart_element().querySelectorAll("canvas")].map((canvas) => canvas.getAttribute("aria-hidden"));
    const accessibility = api.enable_accessibility(chart, {
      chart_title: "Accessible price history",
      data_scope: "all",
      show_shortcuts: true,
      announce_data_updates: "active",
    });
    accessibility.focus(0);
    const layer = document.activeElement;
    layer.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const point_text = chart.chart_element().querySelector(".aeris_charts-a11y-live-region")?.textContent ?? "";
    const last = series.data().at(-1);
    const x = chart.time_scale().time_to_coordinate(last.time);
    const y = series.price_to_coordinate(last.close);
    const screenshot = chart.take_screenshot(true, false);
    const context = screenshot.getContext("2d");
    const ratio = screenshot.width / chart.chart_element().clientWidth;
    const pixels = context.getImageData(
      Math.max(0, Math.floor((x - 16) * ratio)),
      Math.max(0, Math.floor((y - 16) * ratio)),
      Math.ceil(32 * ratio),
      Math.ceil(32 * ratio),
    ).data;
    let focus_pixels = 0;
    for (let index = 0; index < pixels.length; index += 4) {
      if (pixels[index] < 90 && pixels[index + 1] > 60 && pixels[index + 1] < 145 && pixels[index + 2] > 180 && pixels[index + 3] > 180) focus_pixels++;
    }
    layer.dispatchEvent(new KeyboardEvent("keydown", { key: "H", bubbles: true }));
    const panel_visible = getComputedStyle(chart.chart_element().querySelector(".aeris_charts-a11y-shortcuts-panel")).display !== "none";
    const before_zoom = chart.time_scale().get_visible_logical_range();
    layer.dispatchEvent(new KeyboardEvent("keydown", { key: "+", bubbles: true }));
    const after_zoom = chart.time_scale().get_visible_logical_range();
    layer.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const summary = chart.chart_element().querySelector(".aeris_charts-a11y-live-region")?.textContent ?? "";
    series.update({ ...last, close: last.close + 1 });
    await new Promise((resolve) => setTimeout(resolve, 220));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const update = chart.chart_element().querySelector(".aeris_charts-a11y-shared-status-region")?.textContent ?? "";
    const semantic = {
      layers: chart.chart_element().querySelectorAll(".aeris_charts-a11y-layer").length,
      role: layer.getAttribute("role"),
      role_description: layer.getAttribute("aria-roledescription"),
      label: layer.getAttribute("aria-label"),
      canvases_hidden: [...chart.chart_element().querySelectorAll("canvas")].every((canvas) => canvas.getAttribute("aria-hidden") === "true"),
    };
    accessibility.detach();
    const after_canvas = [...chart.chart_element().querySelectorAll("canvas")].map((canvas) => canvas.getAttribute("aria-hidden"));
    return { semantic, point_text, focus_pixels, panel_visible, before_zoom, after_zoom, summary, update, before_canvas, after_canvas };
  });
  expect(result.semantic).toMatchObject({
    layers: 1,
    role: "application",
    role_description: "Interactive chart pane",
    canvases_hidden: true,
  });
  expect(result.semantic.label).toContain("Accessible price history");
  expect(result.point_text).toContain("Point");
  expect(result.point_text).toContain("open");
  expect(result.focus_pixels).toBeGreaterThan(0);
  expect(result.panel_visible).toBe(true);
  expect(result.after_zoom.to - result.after_zoom.from).toBeLessThan(result.before_zoom.to - result.before_zoom.from);
  expect(result.summary).toContain("data points");
  expect(result.update).toContain("Chart data updated");
  expect(result.before_canvas.every((value) => value === "true")).toBe(true);
  expect(result.after_canvas.every((value) => value === null)).toBe(true);
});

test("accessibility creates an independently named focus target for every live pane", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const accessibility = api.enable_accessibility(chart, {
      chart_title: (pane_index) => pane_index === 0 ? "Price pane" : "Volume pane",
    });
    const volume = chart.add_series("histogram", { pane: 1, title: "Volume" });
    volume.set_data(window.__data.map((bar, index) => ({ time: bar.time, value: 1_000 + index })));
    await new Promise((resolve) => queueMicrotask(resolve));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    accessibility.focus(1);
    const layers = [...chart.chart_element().querySelectorAll(".aeris_charts-a11y-layer")];
    const state = {
      count: layers.length,
      labels: layers.map((layer) => layer.getAttribute("aria-label")),
      focused_label: document.activeElement?.getAttribute("aria-label"),
    };
    accessibility.detach();
    return state;
  });
  expect(result.count).toBe(2);
  expect(result.labels[0]).toContain("Price pane");
  expect(result.labels[1]).toContain("Volume pane");
  expect(result.focused_label).toContain("Volume pane");
});

test("tooltip presents themed OHLC market data with explicit volume", async ({ page }) => {
  await open_chart(page);
  const target = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.apply_options(api.theme_options("dark"));
    const volume = chart.add_series("histogram", {
      visible: false,
      price_format: { type: "volume" },
    });
    volume.set_data(window.__data.map((bar, index) => ({ time: bar.time, value: 1_000 + index * 25 })));
    window.__official_tooltip = api.create_tooltip(chart, {
      series: window.__main,
      volume_series: volume,
      title: "AAPL",
      follow_mode: "top",
      top_offset: 20,
    });
    const range = chart.time_scale().get_visible_logical_range();
    const logical = Math.round((range.from + range.to) * 0.5);
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const price = window.__main.data_by_index(logical);
    const volume_point = volume.data_by_index(logical);
    return {
      x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(logical),
      y: bounds.top + pane.top + pane.height * 0.5,
      expected: {
        close: window.__main.price_formatter()(price.close),
        open: window.__main.price_formatter()(price.open),
        high: window.__main.price_formatter()(price.high),
        low: window.__main.price_formatter()(price.low),
        volume: volume.price_formatter()(volume_point.value),
      },
    };
  });
  await page.waitForFunction(() => performance.now() > 600);
  const before = await page.screenshot();
  await page.mouse.move(target.x, target.y);
  await expect.poll(() => page.locator(".aeris_charts-tooltip").evaluate((element) => element.style.opacity)).toBe("1");
  const content = await page.locator(".aeris_charts-tooltip").evaluate((element) => ({
    timestamp: element.querySelector(".aeris_charts-tooltip__timestamp")?.textContent,
    title: element.querySelector(".aeris_charts-tooltip__title")?.textContent,
    rows: [...element.querySelectorAll(".aeris_charts-tooltip__row:not([hidden])")].map((row) => ({
      label: row.querySelector(".aeris_charts-tooltip__label")?.textContent,
      value: row.querySelector(".aeris_charts-tooltip__value")?.textContent,
    })),
    background: getComputedStyle(element).backgroundColor,
    color: getComputedStyle(element).color,
    borderRadius: getComputedStyle(element).borderRadius,
    shadow: getComputedStyle(element).boxShadow,
    transform: element.style.transform,
  }));
  expect(content.timestamp).toMatch(/\w{3} \d{1,2}.*\d{1,2}:\d{2}/);
  expect(content.title).toBe("AAPL");
  expect(content.rows).toEqual([
    { label: "Close", value: target.expected.close },
    { label: "Open", value: target.expected.open },
    { label: "High", value: target.expected.high },
    { label: "Low", value: target.expected.low },
    { label: "Volume", value: target.expected.volume },
  ]);
  expect(content.background).toBe("rgb(31, 31, 31)");
  expect(content.color).toBe("rgb(240, 240, 240)");
  expect(content.borderRadius).toBe("8px");
  expect(content.shadow).toBe("none");
  expect(content.transform).toContain("20px");

  await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    window.__chart.apply_options(api.theme_options("light"));
  });
  await expect.poll(() => page.locator(".aeris_charts-tooltip").evaluate((element) => getComputedStyle(element).backgroundColor)).toBe("rgb(255, 255, 255)");
  expect(await page.locator(".aeris_charts-tooltip").evaluate((element) => getComputedStyle(element).color)).toBe("rgb(51, 51, 51)");

  const after = await page.screenshot();
  expect(after.equals(before)).toBe(false);
  await page.evaluate(() => window.__official_tooltip.detach());
  await expect(page.locator(".aeris_charts-tooltip")).toHaveCount(0);
});

test("tooltip preserves OHLC inspection on area and line presentations", async ({ page }) => {
  await open_chart(page);
  const target = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const series = chart.add_series("area", {
      price_line_visible: false,
      last_value_visible: false,
      countdown_visible: false,
    });
    series.set_data(window.__data.slice(0, 80));
    window.__structured_series = series;
    window.__structured_tooltip = api.create_tooltip(chart, { series });
    chart.time_scale().fit_content();
    const logical = 40;
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const point = window.__data[logical];
    return {
      x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(logical),
      y: bounds.top + pane.top + pane.height * 0.5,
      expected: [point.close, point.open, point.high, point.low].map(series.price_formatter()),
    };
  });

  const read_values = () => page.locator(".aeris_charts-tooltip__row:not([hidden]) .aeris_charts-tooltip__value").allTextContents();
  await page.mouse.move(target.x, target.y);
  await expect.poll(read_values).toEqual(target.expected);
  await expect(page.locator(".aeris_charts-tooltip__row").filter({ hasText: "Volume" })).toBeHidden();

  await page.evaluate(() => window.__structured_series.set_type("line"));
  await page.mouse.move(target.x + 1, target.y);
  await page.mouse.move(target.x, target.y);
  await expect.poll(read_values).toEqual(target.expected);

  await page.evaluate(() => window.__structured_tooltip.detach());
});

test("brushable area keeps ordinary Area data/axes while pane drag is reserved for comparison", async ({ page }) => {
  await open_chart(page);
  const setup = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.apply_options({
      crosshair: {
        mode: 0,
        vertLine: { visible: true, labelVisible: false, color: "#aa2244", width: 3, style: 2 },
        horzLine: { visible: true, labelVisible: true, color: "#22aa44", width: 2, style: 2 },
      },
    });
    const before_options = {
      handle_scroll: structuredClone(chart.options().handle_scroll),
      handle_scale: structuredClone(chart.options().handle_scale),
      crosshair: structuredClone(chart.options().crosshair),
    };
    const brush = chart.add_series("area", {
      price_line_visible: false,
      last_value_visible: false,
    });
    brush.set_data(window.__main.data().map((bar) => ({ time: bar.time, value: bar.close })));
    window.__delta_brush = brush;
    window.__delta_tooltip = api.enable_brushable_area_interaction(chart, brush);
    window.__delta_options_before = before_options;
    chart.apply_options({
      layout: {
        background: { color: "#102030" },
        textColor: "#f1e2d3",
        mutedTextColor: "#c3b2a1",
        fontSize: 15,
        fontFamily: "Courier New",
      },
      rightPriceScale: { borderColor: "#456789" },
    });
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    return {
      left: bounds.left + pane.left,
      top: bounds.top + pane.top,
      width: pane.width,
      height: pane.height,
      y: bounds.top + pane.top + pane.height * 0.5,
      price_axis_x: bounds.left + pane.left + chart.wasm.time_scale_width() + 10,
      scroll_position: chart.wasm.scroll_position(),
    };
  });
  expect(await page.evaluate(() => window.__delta_brush.series_type())).toBe("area");
  expect(await page.evaluate(() => ({
    handle_scroll: window.__chart.options().handle_scroll,
    handle_scale: window.__chart.options().handle_scale,
    crosshair: window.__chart.options().crosshair,
  }))).toEqual({
    ...await page.evaluate(() => window.__delta_options_before),
  });

  // While the helper is attached, primary pane-drag is the comparison gesture. It must not start
  // a competing canvas pan, but it also must not globally mutate the chart's scroll/scale options.
  const pan_start_x = setup.left + setup.width * 0.6;
  await page.mouse.move(pan_start_x, setup.y);
  await page.mouse.down();
  await page.mouse.move(pan_start_x - 120, setup.y, { steps: 6 });
  expect(await page.evaluate(() => window.__chart.wasm.scroll_position())).toBeCloseTo(setup.scroll_position, 6);
  await expect.poll(() => page.evaluate(() => window.__delta_tooltip.active_range())).not.toBe(null);
  await page.mouse.up();
  await page.evaluate(() => window.__delta_tooltip.clear());

  // Price-axis drag retains the canonical scale interaction too: it leaves auto-scale and becomes
  // a manual scale exactly as it does on every ordinary series.
  await page.evaluate(() => window.__chart.price_scale("right").set_auto_scale(true));
  expect(await page.evaluate(() => window.__chart.price_scale("right").options().auto_scale)).toBe(true);
  await page.mouse.move(setup.price_axis_x, setup.y);
  await page.mouse.down();
  await page.mouse.move(setup.price_axis_x, setup.y + 60, { steps: 5 });
  await page.mouse.up();
  expect(await page.evaluate(() => window.__chart.price_scale("right").options().auto_scale)).toBe(false);
  await page.evaluate(async () => {
    window.__chart.price_scale("right").set_auto_scale(true);
    window.__chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });

  const geometry = await page.evaluate(() => {
    const chart = window.__chart;
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const range = chart.time_scale().get_visible_logical_range();
    const span = range.to - range.from;
    const first = Math.ceil(range.from + span * 0.25);
    const second = Math.floor(range.from + span * 0.75);
    const middle = Math.round((first + second) * 0.5);
    return {
      left: bounds.left + pane.left,
      top: bounds.top + pane.top,
      width: pane.width,
      height: pane.height,
      y: bounds.top + pane.top + pane.height * 0.5,
      first,
      second,
      x_first: chart.time_scale().logical_to_coordinate(first),
      x_second: chart.time_scale().logical_to_coordinate(second),
      x_middle: chart.time_scale().logical_to_coordinate(middle),
    };
  });
  const before = await page.screenshot();
  await page.mouse.move(geometry.left + geometry.x_first, geometry.y);
  const hover = await page.screenshot();
  expect(hover.equals(before)).toBe(false);
  await page.mouse.down();
  await page.mouse.move(geometry.left + geometry.x_second, geometry.y, { steps: 4 });
  await expect.poll(() => page.evaluate(() => window.__delta_tooltip.active_range())).toEqual({
    from: geometry.first + 1,
    to: geometry.second + 1,
    positive: expect.any(Boolean),
  });
  const comparison = await page.screenshot();
  expect(comparison.equals(hover)).toBe(false);
  const tooltip_clip = PNG.sync.read(await page.screenshot({
    clip: {
      x: geometry.left,
      y: geometry.top,
      width: geometry.width,
      height: Math.min(100, geometry.height),
    },
  }));
  expect(count_near(tooltip_clip, [16, 32, 48], 2), "themed tooltip surface").toBeGreaterThan(100);
  expect(count_near(tooltip_clip, [241, 226, 211], 12), "themed primary text").toBeGreaterThan(5);
  expect(count_near(tooltip_clip, [195, 178, 161], 12), "themed muted text").toBeGreaterThan(5);
  expect(count_near(tooltip_clip, [69, 103, 137], 6), "standard themed border").toBeGreaterThan(10);
  expect(await page.evaluate(() => window.__chart.chart_element().querySelectorAll(".aeris_charts-delta-tooltip").length)).toBe(0);

  await page.mouse.up();
  expect(await page.evaluate(() => window.__delta_tooltip.active_range())).toMatchObject({
    from: geometry.first + 1,
    to: geometry.second + 1,
  });
  await page.mouse.move(geometry.left + geometry.x_middle, geometry.y);
  await expect.poll(() => page.evaluate(() => window.__delta_tooltip.active_range())).toMatchObject({
    from: geometry.first + 1,
    to: geometry.second + 1,
  });

  await page.evaluate(() => window.__delta_tooltip.clear());
  expect(await page.evaluate(() => window.__delta_tooltip.active_range())).toBe(null);

  await page.mouse.move(geometry.left + geometry.x_first, geometry.y);
  await page.mouse.down();
  await page.mouse.move(geometry.left + geometry.x_second, geometry.y, { steps: 4 });
  await page.mouse.up();
  expect(await page.evaluate(() => window.__delta_tooltip.active_range())).not.toBe(null);
  await page.evaluate(() => {
    window.__delta_tooltip.detach();
    window.__delta_tooltip.detach();
  });
  expect(await page.evaluate(() => window.__chart.options().crosshair)).toEqual(
    await page.evaluate(() => window.__delta_options_before.crosshair),
  );
});

test("brushable area guides reproject and Escape or double click clears", async ({ page }) => {
  await open_chart(page);
  const selection = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const brush = chart.add_series("area", {
      price_line_visible: false,
      last_value_visible: false,
    });
    brush.set_data(window.__main.data().map((bar) => ({ time: bar.time, value: bar.close })));
    window.__reproject_brush = brush;
    window.__reproject_interaction = api.enable_brushable_area_interaction(chart, brush);
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const range = chart.time_scale().get_visible_logical_range();
    const span = range.to - range.from;
    const from = Math.ceil(range.from + span * 0.3);
    const to = Math.floor(range.from + span * 0.7);
    return {
      from,
      to,
      start: {
        x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(from),
        y: bounds.top + pane.top + pane.height * 0.5,
      },
      end: {
        x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(to),
        y: bounds.top + pane.top + pane.height * 0.5,
      },
    };
  });

  await page.mouse.move(selection.start.x, selection.start.y);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(selection.end.x, selection.end.y, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  const committed = await page.evaluate(() => window.__reproject_interaction.active_range());
  expect(committed).toMatchObject({ from: selection.from + 1, to: selection.to + 1 });

  const probes = () => page.evaluate(() => {
    const chart = window.__chart;
    const brush = window.__reproject_brush;
    const active = window.__reproject_interaction.active_range();
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    return [active.from - 1, active.to - 1].map((logical) => {
      const point = brush.data()[logical];
      return {
        x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(logical),
        y: bounds.top + pane.top + brush.price_to_coordinate(point.value),
      };
    });
  });
  const expect_handles = async (points, viewport) => {
    const image = PNG.sync.read(await page.screenshot());
    for (const point of points) {
      expect(count_near_point(image, point, viewport, [136, 136, 136], 12), "guide handle at semantic boundary")
        .toBeGreaterThan(12);
    }
  };

  const initial_probes = await probes();
  await expect_handles(initial_probes, { width: 1280, height: 720 });

  await page.locator("#chart_container canvas:last-of-type").focus();
  const initial_scroll = await page.evaluate(() => window.__chart.wasm.scroll_position());
  await page.keyboard.down("Control");
  await page.keyboard.down("ArrowLeft");
  await expect.poll(() => page.evaluate(() => window.__chart.wasm.scroll_position()))
    .not.toBeCloseTo(initial_scroll, 8);
  await page.keyboard.up("ArrowLeft");
  await page.keyboard.up("Control");
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__reproject_interaction.active_range())).toEqual(committed);
  const panned_probes = await probes();
  expect(panned_probes.map((point) => point.x)).not.toEqual(initial_probes.map((point) => point.x));
  await expect_handles(panned_probes, { width: 1280, height: 720 });

  await page.setViewportSize({ width: 1500, height: 840 });
  await page.waitForFunction(() => window.__chart.chart_element().getBoundingClientRect().width > 1400);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__reproject_interaction.active_range())).toEqual(committed);
  const resized_probes = await probes();
  await expect_handles(resized_probes, { width: 1500, height: 840 });

  await page.keyboard.press("Escape");
  await expect.poll(() => page.evaluate(() => window.__reproject_interaction.active_range())).toBe(null);

  await page.mouse.move(selection.start.x, selection.start.y);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(selection.end.x, selection.end.y, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  expect(await page.evaluate(() => window.__reproject_interaction.active_range())).not.toBe(null);

  const pane_center = await page.evaluate(() => {
    const chart = window.__chart;
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    return { x: bounds.left + pane.left + pane.width * 0.5, y: bounds.top + pane.top + pane.height * 0.5 };
  });
  await page.mouse.dblclick(pane_center.x, pane_center.y);
  await expect.poll(() => page.evaluate(() => window.__reproject_interaction.active_range())).toBe(null);
});
