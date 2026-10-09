import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { PNG } from "pngjs";

// industry-standard series selection: hovering a series shows the pointer (click affordance)
// cursor; clicking selects it and paints anchor points on its drawn data points — theme-derived
// fill (white on light backgrounds, black on dark) with the accent-blue border.

const BLUE = [0, 145, 255]; // semantic primary #0091ff — the anchor border

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
}

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

/** A visible bar's center x plus its close y (a certain series hit). */
async function bar_close_spot(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor((range.from + range.to) / 2);
    const bar = window.__main.data_by_index(index);
    return { x: window.__chart.time_scale().logical_to_coordinate(index), y: window.__main.price_to_coordinate(bar.close) };
  });
}

/** A pane point above every visible candle's high (plus the hit tolerance): a certain miss. */
async function empty_spot(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    let max_high = -Infinity;
    for (let i = Math.ceil(range.from); i <= Math.floor(range.to); i++) {
      const bar = window.__main.data_by_index(i);
      if (bar) max_high = Math.max(max_high, bar.high);
    }
    const x = window.__chart.time_scale().logical_to_coordinate(Math.floor((range.from + range.to) / 2));
    return { x, y: window.__main.price_to_coordinate(max_high) - 20 };
  });
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

function count_color(png, target, tol = 30) {
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

/** The anchor disc's fill pixel at a projected point (screenshot px == CSS px here). */
function fill_pixel_at(png, x, y) {
  const o = (Math.round(y) * png.width + Math.round(x)) * 4;
  return [png.data[o], png.data[o + 1], png.data[o + 2]];
}

function has_color_near(png, x, y, target, radius = 6, tol = 30) {
  for (let py = Math.max(0, Math.floor(y - radius)); py <= Math.min(png.height - 1, Math.ceil(y + radius)); py++) {
    for (let px = Math.max(0, Math.floor(x - radius)); px <= Math.min(png.width - 1, Math.ceil(x + radius)); px++) {
      const o = (py * png.width + px) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol
        && Math.abs(png.data[o + 1] - target[1]) <= tol
        && Math.abs(png.data[o + 2] - target[2]) <= tol
      ) return true;
    }
  }
  return false;
}

async function selection_state(page) {
  return page.evaluate(() => {
    const bounds = document.getElementById("chart_container").getBoundingClientRect();
    const identities = JSON.parse(window.__chart.wasm.selection_anchor_identities_json());
    const points = identities.map((time) => {
      const index = window.__chart.time_scale().time_to_index(time);
      const bar = index === null ? null : window.__main.data_by_index(index);
      return {
        time,
        x: window.__chart.time_scale().time_to_coordinate(time),
        y: bar ? window.__main.price_to_coordinate((bar.open + bar.close) / 2) : null,
      };
    });
    return {
      identities,
      points,
      width: window.__chart.wasm.time_scale_width(),
      height: bounds.height - window.__chart.wasm.time_scale_height(),
      chart_width: bounds.width,
      chart_height: bounds.height,
      pane_left: window.__chart.wasm.pane_left(),
    };
  });
}

async function next_frame(page) {
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test("series hover shows the pointer cursor; off the geometry it stays crosshair", async ({ page }) => {
  await goto_fixture(page);
  const spot = await bar_close_spot(page);
  await page.mouse.move(spot.x, spot.y);
  expect(await overlay_cursor(page)).toBe("pointer");

  const empty = await empty_spot(page);
  await page.mouse.move(empty.x, empty.y);
  expect(await overlay_cursor(page)).toBe("crosshair");
});

test("click selects a series with theme-derived anchors; empty click deselects", async ({ page }) => {
  await goto_fixture(page);
  const spot = await bar_close_spot(page);
  const before = await capture(page);
  expect(count_color(before, BLUE), "no anchors before selection").toBe(0);

  // Click the bar: accent-blue anchor borders appear, with WHITE fills on the light chart.
  await page.mouse.click(spot.x, spot.y);
  const selected = await selection_state(page);
  const body = selected.points.find((point) => point.x !== null && point.y !== null && point.x >= 0 && point.x <= selected.width);
  expect(body, "a selected candle body midpoint is visible").toBeTruthy();
  const light = await capture(page);
  expect(count_color(light, BLUE), "anchor borders after click").toBeGreaterThan(50);
  expect(fill_pixel_at(light, body.x, body.y), "light-theme anchor fill").toEqual([255, 255, 255]);

  // Dark background: fills track the background luminance to black, blue borders stay.
  await page.evaluate(() => window.__chart.apply_options({ layout: { background: { color: "#0d0d0d" } } }));
  await page.waitForTimeout(100);
  const dark = await capture(page);
  expect(count_color(dark, BLUE), "anchor borders persist on dark").toBeGreaterThan(50);
  const fill = fill_pixel_at(dark, body.x, body.y);
  expect(fill[0] < 60 && fill[1] < 60 && fill[2] < 60, `dark-theme anchor fill ${fill}`).toBe(true);

  // Click empty pane space: the selection (and its anchors) clears.
  await page.evaluate(() => window.__chart.apply_options({ layout: { background: { color: "#ffffff" } } }));
  const empty = await empty_spot(page);
  await page.mouse.click(empty.x, empty.y);
  const cleared = await capture(page);
  expect(count_color(cleared, BLUE), "anchors gone after empty click").toBe(0);
});

test("selected anchor identities survive real wheel zoom, pan, and resize while reprojecting", async ({ page }) => {
  await goto_fixture(page);
  const spot = await bar_close_spot(page);
  await page.mouse.click(spot.x, spot.y);
  const selected = await selection_state(page);
  expect(selected.identities.length).toBeGreaterThanOrEqual(24);
  expect(selected.identities.length).toBeLessThanOrEqual(128);

  await page.mouse.move(spot.x, spot.y);
  await page.mouse.wheel(0, -120);
  await next_frame(page);
  const zoomed = await selection_state(page);
  expect(zoomed.identities).toEqual(selected.identities);
  expect(zoomed.points.some((point, index) => point.x !== selected.points[index].x)).toBe(true);

  await page.mouse.move(spot.x, spot.y);
  await page.mouse.down();
  await page.mouse.move(spot.x - 120, spot.y);
  await page.mouse.move(spot.x - 121, spot.y);
  await page.mouse.up();
  await next_frame(page);
  const panned = await selection_state(page);
  expect(panned.identities).toEqual(selected.identities);
  expect(panned.points.some((point, index) => point.x !== zoomed.points[index].x)).toBe(true);

  const visible = panned.points.find((point) =>
    point.x !== null && point.y !== null
    && point.x >= 0 && point.x <= panned.width
    && point.y >= 0 && point.y <= panned.height
  );
  expect(visible, "at least one retained identity remains visible").toBeTruthy();
  const rendered = await capture(page);
  expect(
    has_color_near(
      rendered,
      (panned.pane_left + visible.x) * rendered.width / panned.chart_width,
      visible.y * rendered.height / panned.chart_height,
      BLUE,
    ),
    "retained anchor is painted at its source candle's current projection",
  ).toBe(true);

  await page.setViewportSize({ width: 1100, height: 720 });
  await next_frame(page);
  const resized = await selection_state(page);
  expect(resized.identities).toEqual(selected.identities);
});
