import { test, expect } from "@playwright/test";

// Real pointer input through the demo hosts: Playwright's mouse and viewport drive the browser's own
// event pipeline (no synthetic dispatch), and every hover readout is checked against the data the
// host fed the chart. Covers the financial OHLC hover readout across pan, Ctrl+wheel zoom, and window
// resize, and the general dashboard's hover tooltip, series visibility (the legend's hidden state),
// and resize reprojection.

const settle = (page) => page.evaluate(() => new Promise((resolve) => {
  requestAnimationFrame(() => requestAnimationFrame(resolve));
}));

// ---- financial demo -------------------------------------------------------------------------------

async function open_financial(page) {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__grid !== undefined && window.__main !== undefined
    && window.__chart?.backend?.() === "canvas2d");
  // Pin a fitted, static viewport so the probe bars sit away from the streaming live edge.
  await page.evaluate(() => window.__chart.time_scale().fit_content());
  await settle(page);
}

/** Pane geometry in page coordinates plus the plot-local view state. */
const financial_view = (page) => page.evaluate(() => {
  const rect = document.getElementById("chart_container").getBoundingClientRect();
  const chart = window.__chart;
  return {
    plot_left: rect.left + chart.wasm.pane_left(),
    top: rect.top,
    plot_width: chart.time_scale().width(),
    pane_height: chart.wasm.pane_height(0),
    spacing: chart.wasm.bar_spacing(),
  };
});

/** The OHLC legend text the demo must show for the bar nearest plot-local `x`. */
const expected_readout = (page, x) => page.evaluate((x) => {
  const time = window.__chart.time_scale().coordinate_to_time(x);
  const bar = window.__main.data().find((row) => row.time === time);
  const fmt = (v) => v.toFixed(2);
  return {
    time,
    logical: window.__chart.time_scale().coordinate_to_logical(x),
    text: `O ${fmt(bar.open)}  H ${fmt(bar.high)}  L ${fmt(bar.low)}  C ${fmt(bar.close)}`,
  };
}, x);

const legend = (page) => page.locator("#legend");

test("financial hover readout tracks the bar under the pointer through pan, zoom, and resize", async ({ page }) => {
  const page_errors = [];
  page.on("pageerror", (error) => page_errors.push(error.message));
  await open_financial(page);

  let view = await financial_view(page);
  const probe = { x: view.plot_width * 0.35, y: view.pane_height * 0.5 };
  const client = (v) => ({ x: v.plot_left + probe.x, y: v.top + probe.y });

  // Hover: the readout names exactly the bar under the pointer.
  await page.mouse.move(client(view).x, client(view).y);
  const hovered = await expected_readout(page, probe.x);
  await expect(legend(page)).toHaveText(hovered.text);

  // Pan: dragging the pane right brings older bars under the same pointer position. The engine
  // opens the pan on the sample that crosses the click slop and scrolls from there, so the view
  // moves by the 200px travelled after that first 10px sample.
  const visible_from = () => page.evaluate(() => window.__chart.time_scale().get_visible_logical_range().from);
  await page.mouse.down();
  await page.mouse.move(client(view).x + 10, client(view).y);
  const before_pan = await visible_from();
  await page.mouse.move(client(view).x + 210, client(view).y, { steps: 8 });
  await page.mouse.up();
  await settle(page);
  expect(before_pan - await visible_from()).toBeCloseTo(200 / view.spacing, 1);
  await page.mouse.move(client(view).x, client(view).y);
  const panned = await expected_readout(page, probe.x);
  expect(panned.time).toBeLessThan(hovered.time);
  await expect(legend(page)).toHaveText(panned.text);

  // Zoom: Ctrl+wheel zooms in around the pointer, so the same bar stays under it.
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -240);
  await page.keyboard.up("Control");
  await settle(page);
  const zoomed_view = await financial_view(page);
  expect(zoomed_view.spacing).toBeGreaterThan(view.spacing);
  const zoomed = await expected_readout(page, probe.x);
  expect(Math.abs(zoomed.logical - panned.logical)).toBeLessThan(0.5);
  await page.mouse.move(client(zoomed_view).x + 1, client(zoomed_view).y);
  await page.mouse.move(client(zoomed_view).x, client(zoomed_view).y);
  await expect(legend(page)).toHaveText(zoomed.text);

  // Resize: a narrower window shrinks the plot, and hover maps through the new geometry.
  await page.setViewportSize({ width: 1000, height: 640 });
  await expect.poll(async () => (await financial_view(page)).plot_width).toBeLessThan(view.plot_width);
  view = await financial_view(page);
  const resized_probe = { x: view.plot_width * 0.6, y: view.pane_height * 0.5 };
  await page.mouse.move(view.plot_left + resized_probe.x, view.top + resized_probe.y);
  const resized = await expected_readout(page, resized_probe.x);
  await expect(legend(page)).toHaveText(resized.text);

  // Leaving the chart clears the readout.
  await page.mouse.move(1, 1);
  await expect(legend(page)).toHaveText("O — H — L — C —");
  expect(page_errors).toEqual([]);
});

// ---- general dashboard ----------------------------------------------------------------------------

async function open_dashboard(page) {
  await page.goto("/?theme=dark&demo=general&backend=canvas2d");
  await page.waitForFunction(() => window.__generalDashboard?.ready === true);
}

function card(page, title) {
  return page.locator("#general_workspace .general-chart-card", { has: page.getByRole("heading", { name: title }) });
}

async function mount_card(page, title) {
  const locator = card(page, title);
  await locator.scrollIntoViewIfNeeded();
  await expect(locator).toHaveAttribute("data-mounted", "true");
  await settle(page);
  return locator;
}

/**
 * Page coordinates of a point inside the `row` mark of the series titled `series_title` on the card
 * titled `card_title`, found by scanning the engine's exact hit test over the host. `null` when the
 * mark is not hittable (for example, its series is hidden).
 */
const mark_point = (page, card_title, series_title, row) => page.evaluate(({ card_title, series_title, row }) => {
  const entry = window.__generalDashboard.active_entries().find((e) => e.example.title === card_title);
  const series = entry.series.find((s) => s.options().title === series_title);
  const host = entry.card.querySelector(".general-chart-host");
  const rect = host.getBoundingClientRect();
  const hits = [];
  for (let y = 2; y < rect.height; y += 3) {
    for (let x = 2; x < rect.width; x += 3) {
      const hit = entry.chart.general_hit_test(entry.pane.pane_index(), x, y);
      if (hit !== null && hit.series === series.id && hit.row === row) hits.push([x, y]);
    }
  }
  if (hits.length === 0) return null;
  // The centroid of a column, area, or point mark lies inside it; fall back to a sampled hit.
  const cx = hits.reduce((sum, [x]) => sum + x, 0) / hits.length;
  const cy = hits.reduce((sum, [, y]) => sum + y, 0) / hits.length;
  const [x, y] = hits.reduce((best, h) => (Math.hypot(h[0] - cx, h[1] - cy) < Math.hypot(best[0] - cx, best[1] - cy) ? h : best));
  return { x: rect.left + x, y: rect.top + y, host_width: rect.width };
}, { card_title, series_title, row });

const tooltip = (locator) => locator.locator(".general-chart-tooltip");

test("general dashboard hover tooltip names the hovered mark and hides on leave", async ({ page }) => {
  const page_errors = [];
  page.on("pageerror", (error) => page_errors.push(error.message));
  await open_dashboard(page);
  const revenue = await mount_card(page, "Channel revenue");

  await expect(tooltip(revenue)).not.toHaveAttribute("data-visible", "true");
  for (const [row, category, value] of [[4, "API", "83"], [1, "Partner", "51"]]) {
    const point = await mark_point(page, "Channel revenue", "Revenue", row);
    expect(point).not.toBeNull();
    await page.mouse.move(point.x, point.y);
    await expect(tooltip(revenue)).toHaveAttribute("data-visible", "true");
    await expect(tooltip(revenue).locator("strong")).toHaveText(category);
    await expect(tooltip(revenue)).toContainText(`value ${value}`);
  }

  await page.mouse.move(1, 1);
  await expect(tooltip(revenue)).toHaveAttribute("data-visible", "false");
  expect(page_errors).toEqual([]);
});

test("hiding a series updates the legend state and removes its marks from hover until shown again", async ({ page }) => {
  await open_dashboard(page);
  const regional = await mount_card(page, "Regional comparison");
  const series = (title) => `window.__generalDashboard.active_entries()
    .find((e) => e.example.title === "Regional comparison").series.find((s) => s.options().title === ${JSON.stringify(title)})`;
  const legend_state = () => page.evaluate(() => {
    const entry = window.__generalDashboard.active_entries().find((e) => e.example.title === "Regional comparison");
    return entry.chart.general_legend_snapshot(entry.pane.pane_index()).items.map((item) => [item.title, item.visible]);
  });

  expect(await legend_state()).toEqual([["North", true], ["South", true]]);
  const north_q4 = await mark_point(page, "Regional comparison", "North", 3);
  expect(north_q4).not.toBeNull();
  await page.mouse.move(north_q4.x, north_q4.y);
  await expect(tooltip(regional)).toContainText("value 71");

  // Toggle North off: the legend keeps the entry as hidden, and its marks leave hit testing.
  await page.mouse.move(1, 1);
  await page.evaluate(`${series("North")}.set_visible(false)`);
  await settle(page);
  expect(await legend_state()).toEqual([["North", false], ["South", true]]);
  expect(await mark_point(page, "Regional comparison", "North", 3)).toBeNull();
  await page.mouse.move(north_q4.x, north_q4.y);
  await settle(page);
  await expect(tooltip(regional)).not.toContainText("value 71");
  const south_q4 = await mark_point(page, "Regional comparison", "South", 3);
  expect(south_q4).not.toBeNull();
  await page.mouse.move(south_q4.x, south_q4.y);
  await expect(tooltip(regional)).toHaveAttribute("data-visible", "true");
  await expect(tooltip(regional)).toContainText("value 65");

  // Toggle North back on: the legend and hover both return to the original state.
  await page.mouse.move(1, 1);
  await page.evaluate(`${series("North")}.set_visible(true)`);
  await settle(page);
  expect(await legend_state()).toEqual([["North", true], ["South", true]]);
  const restored = await mark_point(page, "Regional comparison", "North", 3);
  expect(restored).not.toBeNull();
  await page.mouse.move(restored.x, restored.y);
  await expect(tooltip(regional)).toContainText("value 71");
});

test("resizing the window reflows dashboard charts and hover follows the reprojected marks", async ({ page }) => {
  await open_dashboard(page);
  const revenue = await mount_card(page, "Channel revenue");
  const before = await mark_point(page, "Channel revenue", "Revenue", 4);
  expect(before).not.toBeNull();

  await page.setViewportSize({ width: 820, height: 720 });
  await revenue.scrollIntoViewIfNeeded();
  await expect.poll(async () => (await mark_point(page, "Channel revenue", "Revenue", 4))?.host_width)
    .not.toBe(before.host_width);
  // The chart canvas follows its host at the new size.
  const sizes = await revenue.evaluate((element) => {
    const host = element.querySelector(".general-chart-host");
    const canvas = host.querySelector("canvas");
    return { host: host.getBoundingClientRect().width, canvas: canvas.getBoundingClientRect().width };
  });
  expect(Math.abs(sizes.canvas - sizes.host)).toBeLessThan(1);

  const after = await mark_point(page, "Channel revenue", "Revenue", 4);
  expect(after).not.toBeNull();
  await page.mouse.move(after.x, after.y);
  await expect(tooltip(revenue)).toHaveAttribute("data-visible", "true");
  await expect(tooltip(revenue).locator("strong")).toHaveText("API");
  await expect(tooltip(revenue)).toContainText("value 83");
});
