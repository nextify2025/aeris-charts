import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

// Multi-chart split grid in the MAIN demo: the primary chart is the first cell, splits are
// independent, dividers drag, the usage signal meters, the cap enforces, and closes collapse.

async function wait_grid(page) {
  await page.waitForFunction(() => window.__grid !== undefined && window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test("header comparison controls add and clear independently scaled symbols", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);

  await page.selectOption("#comparison_symbol", "MOON");
  await page.click("#comparison_add");
  await page.selectOption("#comparison_symbol", "ORBIT");
  await page.click("#comparison_add");

  const added = await page.evaluate(() => ({
    count: window.__chart.__demo_comparisons.size,
    scales: window.__chart.price_scales().filter((scale) => scale.id.startsWith("demo-comparison-")),
    moon_type: window.__chart.__demo_comparisons.get("MOON").series.series_type(),
    orbit_type: window.__chart.__demo_comparisons.get("ORBIT").series.series_type(),
    moon_margins: window.__chart.price_scale("demo-comparison-moon").options().scale_margins,
    orbit_margins: window.__chart.price_scale("demo-comparison-orbit").options().scale_margins,
    moon_minimum_width: window.__chart.price_scale("demo-comparison-moon").options().minimum_width,
    orbit_minimum_width: window.__chart.price_scale("demo-comparison-orbit").options().minimum_width,
  }));
  expect(added.count).toBe(2);
  expect(added.scales.map((scale) => [scale.id, scale.side, scale.series_ids.length])).toEqual([
    ["demo-comparison-moon", "left", 1],
    ["demo-comparison-orbit", "right", 1],
  ]);
  expect(added.moon_type).toBe("candlestick");
  expect(added.orbit_type).toBe("candlestick");
  expect(added.moon_margins).toEqual({ top: 0, bottom: 0 });
  expect(added.orbit_margins).toEqual({ top: 0, bottom: 0 });
  expect(added.moon_minimum_width).toBe(0);
  expect(added.orbit_minimum_width).toBe(0);
  await expect(page.locator("#comparison_clear")).toContainText("2");

  const narrowed = await page.evaluate(async () => {
    const chart = window.__chart;
    const series = chart.__demo_comparisons.get("MOON").series;
    const rows = series.data();
    chart.time_scale().set_visible_logical_range({ from: rows.length - 80, to: rows.length - 1 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const logical = chart.time_scale().get_visible_logical_range();
    const visible = rows.slice(Math.max(0, Math.floor(logical.from)), Math.ceil(logical.to) + 1);
    return {
      data_min: Math.min(...visible.map((row) => row.low)),
      data_max: Math.max(...visible.map((row) => row.high)),
      scale: chart.price_scale("demo-comparison-moon").get_visible_range(),
      logical,
    };
  });
  console.log("narrowed comparison autoscale:", JSON.stringify(narrowed));
  expect(narrowed.scale.from).toBeLessThanOrEqual(narrowed.data_min);
  expect(narrowed.scale.to).toBeGreaterThanOrEqual(narrowed.data_max);

  await page.click("#comparison_clear");
  expect(await page.evaluate(() => window.__chart.__demo_comparisons.size)).toBe(0);
  expect(await page.evaluate(() => window.__chart.price_scales()
    .filter((scale) => scale.id.startsWith("demo-comparison-"))
    .every((scale) => !scale.visible && scale.series_ids.length === 0))).toBe(true);
  await expect(page.locator("#comparison_clear")).toBeDisabled();

  await page.selectOption("#comparison_symbol", "MOON");
  await page.click("#comparison_add");
  expect(await page.evaluate(() => window.__chart.__demo_comparisons.size)).toBe(1);
});

test("demo chrome and controls follow the chart theme", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);

  const theme_state = () => page.evaluate(() => ({
    root: document.documentElement.dataset.theme,
    control: document.getElementById("theme_select").value,
    surface: getComputedStyle(document.documentElement).getPropertyValue("--surface").trim(),
    header: getComputedStyle(document.getElementById("bar")).backgroundColor,
    chart: window.__chart.options().layout.background.color,
    axis_text: window.__chart.options().layout.textColor,
    axis_border: window.__chart.options().rightPriceScale.borderColor,
    grid: window.__chart.options().grid.vertLines.color,
    border_control: document.getElementById("axis_border_color").value,
    crosshair_control: document.getElementById("cross_color").value,
    crosshair_label_control: document.getElementById("cross_label_bg").value,
    bullish: window.__chart.options().layout.bullishColor,
    bearish: window.__chart.options().layout.bearishColor,
  }));

  // d2c9b95 ("pin crosshair to dark chrome tokens") pins the crosshair line to the dark border
  // (#333333) and its label to the dark muted surface (#222222) in both themes.
  expect(await theme_state()).toEqual({
    root: "dark",
    control: "dark",
    surface: "#1f1f1f",
    header: "rgb(31, 31, 31)",
    chart: "#1f1f1f",
    axis_text: "#f5f5f5",
    axis_border: "#333333",
    grid: "#333333",
    border_control: "#333333",
    crosshair_control: "#333333",
    crosshair_label_control: "#222222",
    bullish: "#089981",
    bearish: "#f7525f",
  });

  // Both themes share the crosshair tokens, so customize them first: the theme switch below must
  // restore the canonical crosshair and resync its controls rather than keep the custom values.
  await page.locator("#cross_color").fill("#ff0000");
  await page.locator("#cross_label_bg").fill("#00ff00");
  expect(await page.evaluate(() => {
    const line = window.__chart.options().crosshair.vertLine;
    return [line.color, line.labelBackgroundColor];
  })).toEqual(["#ff0000", "#00ff00"]);

  await page.selectOption("#theme_select", "light");
  await wait_grid(page);
  expect(await theme_state()).toEqual({
    root: "light",
    control: "light",
    surface: "#ffffff",
    header: "rgb(255, 255, 255)",
    chart: "#ffffff",
    axis_text: "#222222",
    axis_border: "#e5e5e5",
    grid: "#e5e5e5",
    border_control: "#e5e5e5",
    crosshair_control: "#333333",
    crosshair_label_control: "#222222",
    bullish: "#089981",
    bearish: "#f7525f",
  });
});

test("portable design tokens and disabled controls match the brand contract", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);

  const brand_tokens = [
    "surface", "surface-secondary", "surface-subtle", "surface-raised", "surface-overlay", "surface-inverse",
    "border", "border-secondary", "border-subtle", "border-softer", "border-strong", "border-inverse", "border-width",
    "text-primary", "text-default", "text-secondary", "text-muted", "text-positive", "text-negative", "text-danger",
    "text-warning", "text-interactive", "text-hover", "text-active", "hover-bg", "active-bg", "disabled-bg", "icon",
    "icon-active", "positive", "positive-subtle", "negative", "negative-subtle", "warning", "warning-subtle", "indigo",
    "indigo-subtle", "purple", "purple-subtle", "primary", "primary-hover", "primary-active", "primary-disabled",
    "primary-disabled-foreground", "primary-ring", "primary-subtle", "primary-foreground", "danger", "danger-disabled",
    "danger-disabled-foreground", "danger-ring", "danger-foreground", "button-fill", "button-fill-hover", "button-fill-active",
    "button-fill-foreground", "button-fill-subtle", "ring", "radius-default", "radius-medium", "radius-small", "radius-large",
    "radius-button", "shadow-1", "shadow-2", "shadow-3", "shadow-dialog", "bullish", "bearish",
  ];
  const tokens = () => page.evaluate((names) => {
    const style = getComputedStyle(document.documentElement);
    return Object.fromEntries(names.map((name) => [name, style.getPropertyValue(`--${name}`).trim().toLowerCase()]));
  }, brand_tokens);

  const dark = await tokens();
  expect(Object.values(dark).every(Boolean)).toBe(true);
  expect(dark).toMatchObject({
    surface: "#1f1f1f", border: "#333333", "text-primary": "#f5f5f5", "text-secondary": "#c2c2c2",
    "text-muted": "#808080", "text-positive": "#089981", "text-negative": "#f7525f", primary: "#0091ff",
    "primary-hover": "#0077fa", positive: "#089981", negative: "#f7525f", bullish: "#089981", bearish: "#f7525f",
    "button-fill": "#f5f5f5", "radius-button": "6px",
  });

  await page.selectOption("#theme_select", "light");
  const light = await tokens();
  expect(Object.values(light).every(Boolean)).toBe(true);
  expect(light).toMatchObject({
    surface: "#ffffff", border: "#e5e5e5", "text-primary": "#222222", "text-secondary": "#646465",
    "text-muted": "#c2c2c2", "text-positive": "#089981", "text-negative": "#f7525f", primary: "#0091ff",
    "primary-hover": "#0077fa", positive: "#089981", negative: "#f7525f", bullish: "#089981", bearish: "#f7525f",
    "button-fill": "#333333", "radius-button": "6px",
  });

  await page.evaluate(() => {
    const button = document.createElement("button");
    button.id = "disabled_brand_button";
    button.className = "nc-button nc-button--outline";
    button.disabled = true;
    button.textContent = "Disabled";
    document.body.append(button);
  });
  const disabled = page.locator("#disabled_brand_button");
  await expect(disabled).toHaveCSS("cursor", "not-allowed");
  await expect(disabled).toHaveCSS("pointer-events", "auto");
  const background = await disabled.evaluate((button) => getComputedStyle(button).backgroundColor);
  await disabled.hover({ force: true });
  await expect(disabled).toHaveCSS("background-color", background);
});

/** Every cell's chart handle has resolved (a split rebuilds the layout once the chart is in). */
async function wait_cell_charts(page) {
  await page.waitForFunction(() => window.__grid.cells().every((c) => !!c.chart));
  await wait_grid(page);
}

const canvas_count = (page) => page.evaluate(() => document.querySelectorAll("#chart_container canvas").length);

/** Per-cell screenshots as PNGs (public take_screenshot per chart). */
async function cell_shots(page) {
  const urls = await page.evaluate(() =>
    window.__grid.cells().map((c) => c.chart.take_screenshot().toDataURL("image/png")),
  );
  return urls.map((u) => PNG.sync.read(Buffer.from(u.split(",")[1], "base64")));
}

function count_color(png, target, tol = 40) {
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

/** Click a cell's center to make it the active one (toolbar acts on it). */
async function activate_cell(page, index) {
  const point = await page.evaluate((i) => {
    const rect = window.__grid.cells()[i].element.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }, index);
  await page.mouse.click(point.x, point.y);
}

test("main demo splits into independent charts, drags dividers, meters, caps, and collapses", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  expect(await canvas_count(page)).toBe(4); // the primary chart alone: four stacked canvases
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(1);

  // Split horizontally: two independent charts, both rendering candles.
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  let shots = await cell_shots(page);
  for (const shot of shots) {
    const bullish = count_color(shot, [124, 141, 176]);
    const bearish = count_color(shot, [152, 97, 92]);
    expect(bullish + bearish, "each cell renders its own candles").toBeGreaterThan(200);
  }

  // The divider is visible and draggable (col-resize), and the drag resizes the cells.
  const divider = page.locator(".aeris_charts-grid-divider >> nth=0");
  await expect(divider).toHaveCSS("cursor", "col-resize");
  const widths = () => page.evaluate(() => window.__grid.cells().map((c) => c.element.getBoundingClientRect().width));
  const before = await widths();
  const box = await divider.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const after = await widths();
  expect(after[0], "dragging grows the left cell").toBeGreaterThan(before[0] + 60);
  expect(after[1], "and shrinks the right cell").toBeLessThan(before[1] - 60);

  // Independence: the primary cell (the main demo chart) recolors alone.
  await page.evaluate(() => window.__main.apply_options({ up_color: "#0000ff" }));
  shots = await cell_shots(page);
  expect(count_color(shots[0], [0, 0, 255], 40), "primary cell recolored").toBeGreaterThan(50);
  expect(count_color(shots[1], [0, 0, 255], 40), "second cell untouched").toBe(0);

  // Recursive split (vertical on the second cell) → three charts, row-resize divider inside.
  await activate_cell(page, 1);
  await page.click("#split_v");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 12);
  await wait_cell_charts(page);
  await expect(page.locator(".aeris_charts-grid-divider >> nth=1")).toHaveCSS("cursor", "row-resize");
  let usage = await page.evaluate(() => window.__grid.usage());
  expect(usage.chart_count).toBe(3);
  expect(usage.split_count).toBe(2);
  expect(usage.cells.length).toBe(3);

  // Paywall cap: at max 3 the next split is rejected, the grid unchanged.
  await page.selectOption("#max_charts", "3");
  await activate_cell(page, 0);
  await page.click("#split_h");
  await page.waitForTimeout(400);
  expect(await canvas_count(page)).toBe(12);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(3);

  // Close the active (non-primary) cell: the sibling absorbs the space.
  await activate_cell(page, 1);
  await page.click("#close_cell");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  usage = await page.evaluate(() => window.__grid.usage());
  expect(usage.chart_count).toBe(2);

  // The primary cell cannot be closed from the toolbar (all wiring lives on it).
  await activate_cell(page, 0);
  await page.click("#close_cell");
  await page.waitForTimeout(300);
  expect(await canvas_count(page), "primary cell refuses to close").toBe(8);

  // …but closing the remaining secondary cell returns to the single primary chart.
  await activate_cell(page, 1);
  await page.click("#close_cell");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 4);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(1);
});

test("the toolbar's series type and style act on the ACTIVE cell, not always the primary", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  const type_of = (i) => page.evaluate((idx) => window.__grid.cells()[idx].chart.__seed_series.series_type(), i);

  // Activate the second chart and switch it to area: only it changes.
  await activate_cell(page, 1);
  await expect(page.locator('input[name="series"][value="candlestick"]')).toBeChecked();
  await page.locator('input[name="series"][value="area"]').check();
  expect(await type_of(1), "active cell switched to area").toBe("area");
  expect(await type_of(0), "primary cell untouched").toBe("candlestick");
  // The area fill style group appears for the area-typed active chart.
  await expect(page.locator("#area_style")).toBeVisible();
  await expect(page.locator("#area_top")).toHaveValue("#089981");
  expect(await page.evaluate(() => {
    const options = window.__grid.cells()[1].chart.__seed_series.options();
    return [options.color, options.area_top_color, options.area_bottom_color];
    // The toolbar sets the series color; the engine derives the fill at the canonical strength, so
    // no opaque explicit fill is written.
  })).toEqual(["#089981", "", ""]);

  // The toolbar tracks the active cell: back on the primary, its type is selected again and
  // changes land there instead.
  await activate_cell(page, 0);
  await expect(page.locator('input[name="series"][value="candlestick"]')).toBeChecked();
  await expect(page.locator("#area_style")).toBeHidden();
  await page.locator('input[name="series"][value="line"]').check();
  expect(await type_of(0), "primary switched to line").toBe("line");
  expect(await type_of(1), "second cell keeps its area type").toBe("area");

  // Style options follow the active cell too: recolor the primary's line only.
  await page.evaluate(() => {
    const el = document.getElementById("line_color");
    el.value = "#ff00ff";
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const color_of = (i) => page.evaluate((idx) => window.__grid.cells()[idx].chart.__seed_series.options().color, i);
  expect(await color_of(0)).toBe("#ff00ff");
  expect(await color_of(1)).not.toBe("#ff00ff");
});

test("demo theme persists across reloads while runtime fixtures stay deterministic", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.selectOption("#theme_select", "light");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(await page.evaluate(() => localStorage.getItem("aeris_charts.demo.theme"))).toBe("light");

  await page.reload();
  await wait_grid(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("#theme_select")).toHaveValue("light");
  expect(await page.evaluate(() => window.__chart.options().layout.background.color)).toBe("#ffffff");

  await page.selectOption("#theme_select", "dark");
  await page.reload();
  await wait_grid(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(await page.evaluate(() => window.__chart.options().layout.background.color)).toBe("#1f1f1f");

  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("changing a split cell series does not clear the root chart's advanced series", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.evaluate(() => window.__demo_catalogs.series.activate("hlc-area"));
  expect(await page.evaluate(() => window.__demo_catalogs.series.active_id())).toBe("hlc-area");

  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  await activate_cell(page, 1);
  await page.locator('input[name="series"][value="area"]').check();

  expect(await page.evaluate(() => window.__demo_catalogs.series.active_id())).toBe("hlc-area");
  expect(await page.evaluate(() => window.__main.options().visible)).toBe(false);
  expect(await page.evaluate(() => window.__grid.cells()[1].chart.__seed_series.series_type())).toBe("area");
});

test("divider drags never disturb a cell's candle spacing (even with interactions off)", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_grid(page);

  // Dashboard-style embeds: all scroll/scale gestures off. The engine's all-interactions-off
  // aggregate feeds label alignment only (reference time-scale.ts:975-986) — it must not force
  // fix-edge semantics, so a resize leaves bar spacing and the right range edge untouched.
  await page.evaluate(() => {
    for (const cell of window.__grid.cells()) {
      cell.chart.apply_options({ handle_scroll: false, handle_scale: false });
      cell.chart.time_scale().fit_content();
    }
  });
  const probe = (i) =>
    page.evaluate((idx) => {
      const chart = window.__grid.cells()[idx].chart;
      const r = chart.wasm.visible_logical_range();
      return { spacing: chart.wasm.bar_spacing(), right: r.length === 2 ? r[1] : null };
    }, i);
  const left_before = await probe(0);
  const right_before = await probe(1);

  const divider = page.locator(".aeris_charts-grid-divider >> nth=0");
  const box = await divider.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 - 120, box.y + box.height / 2, { steps: 8 });
  await page.mouse.up();
  await wait_grid(page);

  const left_after = await probe(0);
  const right_after = await probe(1);
  expect(right_after.spacing, "growing cell keeps its bar spacing").toBe(right_before.spacing);
  expect(right_after.right, "growing cell keeps its right range edge").toBe(right_before.right);
  expect(left_after.spacing, "shrinking cell keeps its bar spacing").toBe(left_before.spacing);
  expect(left_after.right, "shrinking cell keeps its right range edge").toBe(left_before.right);
});

test("the divider keeps its canonical border line under a global border-box reset", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  // Platform pages commonly reset `box-sizing: border-box` globally (even `!important`); the
  // divider paints its line as a gradient stop across a fixed 5px box, so no content box can
  // collapse under it.
  await page.addStyleTag({ content: "*, *::before, *::after { box-sizing: border-box !important; }" });
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  const divider = page.locator(".aeris_charts-grid-divider >> nth=0");
  const box = await divider.boundingBox();
  expect(box.width).toBeGreaterThanOrEqual(5); // the full hit area

  const projected = await divider.evaluate((el) => ({
    dpr: window.devicePixelRatio,
    width: Number.parseFloat(el.firstElementChild.style.width),
  }));
  expect(projected.width).toBeCloseTo(
    Math.max(1, Math.round(0.5 * projected.dpr)) / projected.dpr,
    6,
  );

  // The canonical line actually paints: screenshot the divider strip and count border-color pixels.
  const png = PNG.sync.read(await page.screenshot({ clip: box }));
  const border_hex = await page.evaluate(() => window.__chart.options().rightPriceScale.borderColor);
  const target = [1, 3, 5].map((i) => parseInt(border_hex.slice(i, i + 2), 16));
  expect(count_color(png, target, 10), "the line paints under the reset").toBeGreaterThan(100);

  // The drag hit area survives too.
  const widths = () => page.evaluate(() => window.__grid.cells().map((c) => c.element.getBoundingClientRect().width));
  const before = await widths();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + 100, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  const after = await widths();
  expect(after[0], "drag still resizes under the reset").toBeGreaterThan(before[0] + 40);
});

test("split dividers follow the axis border token (theme and explicit changes)", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);

  const divider_rgb = () =>
    page.locator(".aeris_charts-grid-divider >> nth=0").evaluate((el) => {
      // The line color is the solid inner strip's background.
      const inner = el.firstElementChild;
      return inner ? getComputedStyle(inner).backgroundColor : "";
    });
  const border_hex = () => page.evaluate(() => window.__chart.options().rightPriceScale.borderColor);
  const to_rgb = (hex) => `rgb(${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(", ")})`;

  // Default: the divider paints in the active theme's axis border color.
  expect(await divider_rgb()).toBe(to_rgb(await border_hex()));

  // Theme switch: the border token changes and the divider tracks it.
  await page.selectOption("#theme_select", "light");
  await wait_grid(page);
  const light_border = await border_hex();
  expect(light_border.toLowerCase()).toBe("#e5e5e5");
  expect(await divider_rgb()).toBe(to_rgb(light_border));

  // An explicit axis border change re-resolves the divider too.
  await page.locator("#axis_border_color").evaluate((el) => {
    el.value = "#ff8800";
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await wait_grid(page);
  expect(await divider_rgb()).toBe("rgb(255, 136, 0)");
});

test("Ctrl+H/V split the active cell; typing fields and the cap still hold", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  expect(await canvas_count(page)).toBe(4);

  // Ctrl+H splits the (initially active) primary cell horizontally.
  await page.keyboard.press("Control+h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(2);

  // Ctrl+V splits the SAME active cell again (still the primary) vertically.
  await page.keyboard.press("Control+v");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 12);
  await wait_cell_charts(page);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(3);

  // Pressing another cell retargets the shortcut: it splits, the primary stays put.
  await activate_cell(page, 2);
  await page.keyboard.press("Control+h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 16);
  await wait_cell_charts(page);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(4);

  // Typing in an input does NOT split.
  await page.locator("#drawing_text").click();
  await page.keyboard.press("Control+v");
  await page.waitForTimeout(300);
  expect(await canvas_count(page)).toBe(16);

  // The paywall cap still gates the shortcut (max 4): the next Ctrl+H is a no-op.
  await page.selectOption("#max_charts", "4");
  await page.keyboard.press("Control+h");
  await page.waitForTimeout(300);
  expect(await canvas_count(page)).toBe(16);
  expect((await page.evaluate(() => window.__grid.usage())).chart_count).toBe(4);
});

test("shortcut splits come up seeded (the on_cell_added hook fires)", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.keyboard.press("Control+h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);
  // Both cells render candles � the shortcut path runs the same seeding hook as the button.
  const shots = await cell_shots(page);
  for (const shot of shots) {
    const bullish = count_color(shot, [124, 141, 176]);
    const bearish = count_color(shot, [152, 97, 92]);
    expect(bullish + bearish, "shortcut-created cell renders its seeded candles").toBeGreaterThan(200);
  }
});

test("the shortcut registry accepts combo overrides", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  // A scratch grid with the vertical split re-keyed: ctrl+shift+x, default ctrl+v disabled.
  await page.evaluate(async () => {
    const mod = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:640px;height:400px;z-index:50;background:white;";
    host.id = "scratch_grid";
    document.body.appendChild(host);
    window.__scratch = await mod.create_chart_grid(host, {
      shortcuts: { "grid.split_vertical": "ctrl+shift+x" },
      on_cell_added: (cell) => {
        const s = cell.chart.add_series("line");
        s.set_data([{ time: 1, value: 1 }, { time: 2, value: 2 }]);
      },
    });
  });
  // The overridden combo splits; the default combo does not.
  await page.keyboard.press("Control+Shift+x");
  await page.waitForFunction(() => window.__scratch.chart_count() === 2);
  await page.keyboard.press("Control+v");
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.__scratch.chart_count())).toBe(2);
  await page.evaluate(() => { window.__scratch.destroy(); document.getElementById("scratch_grid").remove(); });
});

test("Ctrl+click maximizes a cell to full container and restores", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);

  const widths = () => page.evaluate(() => window.__grid.cells().map((c) => {
    const r = c.element.getBoundingClientRect();
    // Hidden = detached from the container (the grid mounts only the maximized slot; a
    // detached slot never collapses its chart through a 0-size resize).
    return { attached: c.element.isConnected, width: r.width };
  }));
  const before = await widths();
  expect(before[1].width).toBeGreaterThan(100);

  // Ctrl+click the second cell: it takes the container, the first cell and dividers hide.
  const point = await page.evaluate(() => {
    const rect = window.__grid.cells()[1].element.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  });
  await page.keyboard.down("Control");
  await page.mouse.click(point.x, point.y);
  await page.keyboard.up("Control");
  const maximized = await widths();
  expect(maximized[0].attached).toBe(false);
  expect(maximized[1].attached).toBe(true);
  expect(maximized[1].width).toBeGreaterThan(before[1].width + before[0].width - 20);
  expect(await page.evaluate(() => window.__grid.maximized_cell()?.id ?? null)).toBe(2);

  // Ctrl+click again: everything reappears.
  await page.keyboard.down("Control");
  await page.mouse.click(point.x, point.y);
  await page.keyboard.up("Control");
  const restored = await widths();
  expect(restored[0].attached).toBe(true);
  expect(restored[1].attached).toBe(true);
  expect(restored[1].width).toBeLessThan(restored[0].width + 20);
  expect(await page.evaluate(() => window.__grid.maximized_cell())).toBeNull();
});

test("drawing tools and history route only to the stable active cell", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.evaluate(async () => {
    const grid = window.__grid;
    let active = grid.active_cell();
    for (const direction of ["horizontal", "vertical", "horizontal"]) {
      const created = await active.split(direction);
      if (created === null) throw new Error("split rejected");
      active = created;
      grid.set_active_cell(active);
    }
  });
  await page.waitForFunction(() => window.__grid.chart_count() === 4);
  await wait_cell_charts(page);

  const draw_in = async (index, tool) => {
    const points = await page.evaluate((i) => {
      const grid = window.__grid;
      const cell = grid.cells()[i];
      grid.set_active_cell(cell);
      const rect = cell.element.getBoundingClientRect();
      return [
        { x: rect.left + rect.width * 0.3, y: rect.top + rect.height * 0.35 },
        { x: rect.left + rect.width * 0.65, y: rect.top + rect.height * 0.65 },
      ];
    }, index);
    await page.click(`#drawings_group [data-tool='${tool}']`);
    await page.mouse.click(points[0].x, points[0].y);
    await page.mouse.click(points[1].x, points[1].y);
    await wait_grid(page);
  };

  await draw_in(1, "trend_line");
  expect(await page.evaluate(() => window.__grid.cells().map((cell) => cell.chart.drawings().length)))
    .toEqual([0, 1, 0, 0]);

  await draw_in(3, "rectangle");
  expect(await page.evaluate(() => window.__grid.cells().map((cell) => cell.chart.drawings().map((d) => d.kind()))))
    .toEqual([[], ["trend_line"], [], ["rectangle"]]);

  // Remove a different stable id and split the root; the retained Chart 4 handle remains the
  // target, and undo affects only its own history.
  const stable = await page.evaluate(async () => {
    const grid = window.__grid;
    const chart4 = grid.cells()[3];
    grid.cells()[2].remove();
    await grid.cells()[0].split("vertical");
    grid.set_active_cell(chart4);
    grid.undo_drawing();
    return {
      active: grid.active_cell().id,
      chart4: chart4.id,
      counts: grid.cells().map((cell) => [cell.id, cell.chart.drawings().length]),
      chart2_can_undo: grid.cells().find((cell) => cell.id === 2).chart.can_undo_drawing(),
    };
  });
  expect(stable.active).toBe(stable.chart4);
  expect(stable.counts.find(([id]) => id === stable.chart4)[1]).toBe(0);
  expect(stable.counts.find(([id]) => id === 2)[1]).toBe(1);
  expect(stable.chart2_can_undo).toBe(true);
  await expect(page.locator("#undo_drawing_btn")).toBeVisible();
  await expect(page.locator("#redo_drawing_btn")).toBeVisible();
  await page.click("#redo_drawing_btn");
  expect(await page.evaluate(() => window.__grid.active_cell().chart.drawings()[0].kind())).toBe("rectangle");
  await page.click("#undo_drawing_btn");
  expect(await page.evaluate(() => window.__grid.active_cell().chart.drawings().length)).toBe(0);
  await page.click("#redo_drawing_btn");
  expect(await page.evaluate(() => window.__grid.active_cell().chart.drawings()[0].kind())).toBe("rectangle");
});

test("reset view targets only the active chart", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await wait_cell_charts(page);

  await page.evaluate(() => {
    const grid = window.__grid;
    const [first, second] = grid.cells();
    first.chart.wasm.set_bar_spacing(18);
    second.chart.wasm.set_bar_spacing(22);
    grid.set_active_cell(second);
  });
  await page.click("#reset_view_btn");

  expect(await page.evaluate(() => window.__grid.cells().map((cell) => cell.chart.wasm.bar_spacing())))
    .toEqual([18, 6]);
});

test("workspace state composes chart persistence V1 and restores stable ownership at a new size", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const result = await page.evaluate(async () => {
    const { create_chart_grid } = await import("/dist/aeris_charts_financial.js");
    const make_host = (width, height) => {
      const host = document.createElement("div");
      host.style.cssText = `position:absolute;left:-10000px;top:0;width:${width}px;height:${height}px`;
      document.body.append(host);
      return host;
    };
    const bars = Array.from({ length: 30 }, (_, i) => ({ time: i + 1, value: 100 + i }));
    const seed = (cell) => {
      const series = cell.chart.add_series("line");
      series.set_data(bars);
      cell.chart.time_scale().fit_content();
      cell.chart.__workspace_series = series;
    };

    const first_host = make_host(900, 540);
    const first = await create_chart_grid(first_host, { shortcuts: false, on_cell_added: seed });
    seed(first.cells()[0]);
    const second = await first.cells()[0].split("horizontal");
    const third = await second.split("vertical");
    first.cells()[0].set_host_chart_identity("BTC-USD");
    second.set_host_chart_identity("ETH-USD");
    third.set_host_chart_identity("SOL-USD");
    first.cells()[0].chart.add_drawing("horizontal_line", [{ logical: 6, price: 106 }], { color: "#ff0000" });
    second.chart.add_drawing("trend_line", [
      { logical: 4, price: 104 }, { logical: 12, price: 112 },
    ], { style: "dashed", width: 4 });
    third.chart.add_drawing("rectangle", [
      { logical: 8, price: 108 }, { logical: 18, price: 118 },
    ], { color: "#00aa00", fill_color: "rgba(0,170,0,0.2)" });
    first.set_active_cell(third);
    const first_projection = {
      x: third.chart.time_scale().logical_to_coordinate(8),
      y: third.chart.__workspace_series.price_to_coordinate(108),
    };
    const state = first.export_state();
    first.destroy();
    first_host.remove();

    const second_host = make_host(620, 360);
    const restored = await create_chart_grid(second_host, {
      shortcuts: false,
      initial_state: state,
      on_cell_restored: seed,
    });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const roundtrip = restored.export_state();
    const restored_cell = restored.cells().find((cell) => cell.id === state.active_cell);
    const second_projection = {
      x: restored_cell.chart.time_scale().logical_to_coordinate(8),
      y: restored_cell.chart.__workspace_series.price_to_coordinate(108),
    };
    const out = {
      state,
      roundtrip,
      active: restored.active_cell().id,
      identities: restored.cells().map((cell) => cell.host_chart_identity),
      drawings: restored.cells().map((cell) => cell.chart.drawings().map((drawing) => ({
        kind: drawing.kind(), points: drawing.points(), options: drawing.options(),
      }))),
      first_projection,
      second_projection,
      histories_empty: restored.cells().every((cell) => !cell.chart.can_undo_drawing() && !cell.chart.can_redo_drawing()),
    };
    restored.destroy();
    second_host.remove();
    return out;
  });

  expect(result.roundtrip).toEqual(result.state);
  expect(result.active).toBe(result.state.active_cell);
  expect(result.identities).toEqual(["BTC-USD", "ETH-USD", "SOL-USD"]);
  expect(result.drawings.map((drawings) => drawings.map((drawing) => drawing.kind)))
    .toEqual([["horizontal_line"], ["trend_line"], ["rectangle"]]);
  // points() also reports each anchor's time identity (the seeded bars use times 1..30, so
  // logical 4 is time 5); the restore resolves that time against the re-seeded data.
  expect(result.drawings[1][0].points).toEqual([
    { logical: 4, price: 104, time: 5 }, { logical: 12, price: 112, time: 13 },
  ]);
  expect(result.drawings[1][0].options).toMatchObject({ style: "dashed", width: 4 });
  expect(result.histories_empty).toBe(true);
  expect(result.second_projection.x).not.toBeCloseTo(result.first_projection.x, 3);
  // Price projection may remain similar under autoscale, but semantic anchors are exact above.
  expect(Number.isFinite(result.second_projection.y)).toBe(true);
});
