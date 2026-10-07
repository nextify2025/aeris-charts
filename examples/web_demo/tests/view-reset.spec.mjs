import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { PNG } from "pngjs";

// The demo's price-line style select and industry-standard "reset view" button.

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

/** Colored pixel runs in the live-price line's row band. */
function price_line_runs(png, y, is_line_pixel) {
  const runs = [];
  let cur = null;
  for (let x = 0; x < png.width; x++) {
    const o = (y * png.width + x) * 4;
    if (is_line_pixel([png.data[o], png.data[o + 1], png.data[o + 2]])) {
      if (!cur) cur = { s: x, e: x };
      cur.e = x;
    } else if (cur) {
      runs.push(cur);
      cur = null;
    }
  }
  if (cur) runs.push(cur);
  return runs;
}

/**
 * The price line's device row: the raw coordinate's rounding can drift a device px when the
 * pane's fractional-dpr height rounds the bitmap (vpr ≠ dpr), so scan the neighborhood and
 * take the row with the most line pixels.
 */
function best_line_row(png, row, is_line_pixel) {
  let best = row;
  let best_count = -1;
  for (let y = Math.max(row - 2, 0); y <= Math.min(row + 2, png.height - 1); y++) {
    let count = 0;
    for (let x = 0; x < png.width; x++) {
      const o = (y * png.width + x) * 4;
      if (is_line_pixel([png.data[o], png.data[o + 1], png.data[o + 2]])) count += 1;
    }
    if (count > best_count) {
      best_count = count;
      best = y;
    }
  }
  return best;
}

test("native partial price line is default and shares full-line style and width semantics", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  // Give the partial line a visible right-side runway and pin a unique color so series pixels can
  // never be mistaken for it. Hide the last-value chip so only the line contributes magenta.
  await page.evaluate(() => {
    window.__chart.time_scale().fit_content();
    window.__chart.time_scale().apply_options({ bar_spacing: 20, right_offset: 5 });
    window.__main.apply_options({
      price_line_color: "#ff00ff",
      price_line_width: 3,
      last_value_visible: false,
    });
  });
  await wait_for_chart(page);
  const probe = await page.evaluate(() => {
    const last = window.__data[window.__data.length - 1];
    return {
      y: window.__main.price_to_coordinate(last.close),
      x: window.__chart.time_scale().time_to_coordinate(last.time),
      pane_right: window.__chart.wasm.pane_left() + window.__chart.wasm.time_scale_width(),
      extent: window.__main.options().price_line_extent,
      width: window.__main.options().price_line_width,
    };
  });
  expect(probe.extent).toBe("partial");
  expect(probe.width).toBe(3);
  const is_line = (c) => Math.abs(c[0] - 255) < 40 && Math.abs(c[1] - 0) < 40 && Math.abs(c[2] - 255) < 40;
  const css_w = await page.evaluate(() => document.querySelector("#chart_container").getBoundingClientRect().width);

  await page.selectOption("#price_line_style", "0"); // solid
  await wait_for_chart(page);
  const solid_png = await capture(page);
  const scale = solid_png.width / css_w;
  const row = best_line_row(solid_png, Math.round(probe.y * scale), is_line);
  const pane_right = Math.round(probe.pane_right * scale);
  const pane_runs = (png, y) => price_line_runs(png, y, is_line)
    .filter((run) => run.s < pane_right)
    .map((run) => ({ s: run.s, e: Math.min(run.e, pane_right - 1) }));
  const solid = pane_runs(solid_png, row);
  expect(solid.length, "solid partial style: one continuous run").toBe(1);
  expect(Math.abs(solid[0].s - Math.round(probe.x * scale)), "partial line starts at tracked bar").toBeLessThanOrEqual(3);
  expect(solid[0].e - solid[0].s, "partial line reaches toward the price axis").toBeGreaterThan(80);

  await page.selectOption("#price_line_style", "1"); // dotted
  await wait_for_chart(page);
  const dotted_png = await capture(page);
  const dotted = pane_runs(dotted_png, best_line_row(dotted_png, row, is_line));
  expect(dotted.length, "dotted style: repeated short runs").toBeGreaterThan(6);

  await page.selectOption("#price_line_style", "2"); // dashed
  await wait_for_chart(page);
  const dashed_png = await capture(page);
  const dashed = pane_runs(dashed_png, best_line_row(dashed_png, row, is_line));
  expect(dashed.length, "dashed style: multiple long runs").toBeGreaterThan(3);
  expect(dashed.length).toBeLessThan(dotted.length);

  await page.selectOption("#price_line_style", "0");
  await page.selectOption("#price_line_extent", "full");
  await wait_for_chart(page);
  const full_png = await capture(page);
  const full = pane_runs(full_png, best_line_row(full_png, row, is_line));
  expect(full.length).toBe(1);
  expect(full[0].s, "full extent remains explicitly available").toBeLessThanOrEqual(2);
});

test("reset view button restores time defaults and re-fits a contracted price scale", async ({ page }) => {
  await page.goto("/?backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);

  // Zoom the time scale at RUNTIME (like a wheel zoom — the option default stays 6) and
  // contract the price scale to a manual sliver.
  await page.evaluate(() => {
    window.__chart.wasm.set_bar_spacing(20);
    const last = window.__data[window.__data.length - 1];
    window.__chart.price_scale("right").set_visible_range({ from: last.close - 0.05, to: last.close + 0.05 });
  });
  await wait_for_chart(page);
  expect(await page.evaluate(() => window.__chart.wasm.bar_spacing())).toBe(20);

  await page.click("#reset_view_btn");
  await wait_for_chart(page);
  // Time scale restored to the configured defaults…
  expect(await page.evaluate(() => window.__chart.wasm.bar_spacing())).toBe(6);
  expect(await page.evaluate(() => window.__chart.time_scale().options().bar_spacing)).toBe(6);
  // …and the price scale autoscales the data into view again.
  const fits = await page.evaluate(() => {
    let min = Infinity, max = -Infinity;
    const range = window.__chart.time_scale().get_visible_logical_range();
    for (let i = Math.ceil(range.from); i <= Math.floor(range.to); i++) {
      const bar = window.__main.data_by_index(i);
      if (bar) { min = Math.min(min, bar.low); max = Math.max(max, bar.high); }
    }
    const top = window.__main.price_to_coordinate(max);
    const bottom = window.__main.price_to_coordinate(min);
    const pane_h = document.querySelector("#chart_container").getBoundingClientRect().height;
    return top > 0 && bottom < pane_h && bottom - top > pane_h * 0.3;
  });
  expect(fits, "candles fill the pane again after reset").toBe(true);
});
