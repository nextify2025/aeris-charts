import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { readFileSync } from "node:fs";
import { PNG } from "pngjs";

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
}

/** Subscribe a page-side collector to crosshair moves, capturing the Phase C-d hover fields. */
async function collect_hover_events(page) {
  await page.evaluate(() => {
    window.__hits = [];
    window.__chart.subscribe_crosshair_move((p) => {
      window.__hits.push({
        obj: p.hovered_object_id,
        series: p.hovered_series ? p.hovered_series.id : null,
      });
    });
  });
}

async function last_hover(page) {
  return page.evaluate(() => window.__hits[window.__hits.length - 1] ?? null);
}

/** The overlay (input/axis) canvas — the element the gesture layer styles with cursors. */
async function overlay_cursor(page) {
  // The overlay is the topmost (last) of the package's stacked canvases.
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

/** A visible bar's center coordinates plus its mid high/low y, via the public handles. */
async function visible_bar_spot(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor((range.from + range.to) / 2);
    const x = window.__chart.time_scale().logical_to_coordinate(index);
    const bar = window.__main.data_by_index(index);
    return { x, y: window.__main.price_to_coordinate((bar.high + bar.low) / 2) };
  });
}

/** A pane point above every visible candle's high (plus the 3px hit tolerance): a certain miss. */
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

test("series primitive hit_test drives hovered_object_id, hovered_series, and the cursor", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__set_position_band(true));
  await settle_frames(page);
  await collect_hover_events(page);

  // The band is ±2% around the middle bar's close; its center is safely inside.
  const band_y = await page.evaluate(() => {
    const entry = window.__data[Math.floor(window.__data.length / 2)].close;
    return window.__main.price_to_coordinate(entry);
  });
  await page.mouse.move(400, band_y);
  const hit = await last_hover(page);
  expect(hit.obj).toBe("position-band");
  // A series primitive's hit source IS the owning series (reference parity).
  const main_id = await page.evaluate(() => window.__main.id);
  expect(hit.series).toBe(main_id);
  expect(await overlay_cursor(page)).toBe("move");

  // Moving off the band clears the object id and restores the region cursor.
  await page.mouse.move(400, Math.min(band_y + 250, 650));
  expect((await last_hover(page)).obj).toBeNull();
  expect(await overlay_cursor(page)).toBe("crosshair");
});

test("engine series hit test sets hovered_series; leaving the geometry clears it", async ({ page }) => {
  await goto_fixture(page);
  await collect_hover_events(page);

  const main_id = await page.evaluate(() => window.__main.id);
  const spot = await visible_bar_spot(page);
  await page.mouse.move(spot.x, spot.y);
  const hit = await last_hover(page);
  expect(hit.series).toBe(main_id);
  expect(hit.obj).toBeNull();

  const empty = await empty_spot(page);
  await page.mouse.move(empty.x, empty.y);
  const cleared = await last_hover(page);
  expect(cleared.series).toBeNull();
  expect(cleared.obj).toBeNull();
  expect(await overlay_cursor(page)).toBe("crosshair");
});

test("secondary click emits pane- and scale-correct chart context without selecting", async ({ page }) => {
  await goto_fixture(page);
  const target = await page.evaluate(async () => {
    const chart = window.__chart;
    const scale = chart.add_price_scale({ id: "context-scale", side: "left", order: 0 });
    const series = chart.add_series("line", {
      color: "#ff00ff",
      line_width: 6,
      price_scale_id: "context-scale",
    });
    const range = chart.time_scale().get_visible_logical_range();
    const logical = Math.floor((range.from + range.to) / 2);
    const time = window.__data[logical].time;
    series.set_data(window.__data.map((bar) => ({ time: bar.time, value: 1_000 })));
    scale.set_visible_range({ from: 900, to: 1_100 });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    window.__contexts = [];
    window.__clicks = 0;
    chart.subscribe_click(() => { window.__clicks += 1; });
    chart.subscribe_chart_context((context) => {
      window.__contexts.push({
        price: context.price,
        time: context.time,
        logical: context.logical,
        point: context.point,
        pane_index: context.pane_index,
        hovered_series: context.hovered_series?.id ?? null,
        hovered_object_id: context.hovered_object_id,
      });
    });
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    const x = chart.time_scale().logical_to_coordinate(logical);
    const y = series.price_to_coordinate(1_000);
    return {
      client_x: bounds.left + pane.left + x,
      client_y: bounds.top + pane.top + y,
      x,
      y,
      time,
      logical,
      series_id: series.id,
    };
  });
  await settle_frames(page);

  const dispatch = await page.evaluate(({ client_x, client_y }) => {
    window.__chart.clear_hover();
    const canvases = document.querySelectorAll("#chart_container canvas");
    const overlay = canvases[canvases.length - 1];
    const accepted = overlay.dispatchEvent(new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: client_x,
      clientY: client_y,
    }));
    return { default_prevented: !accepted, hover_series: window.__chart.hover_series_id() };
  }, target);
  const result = await page.evaluate(() => {
    const context = window.__contexts[0];
    const hovered = window.__chart.series_order().find((series) => series.id === context.hovered_series);
    return {
      contexts: window.__contexts,
      clicks: window.__clicks,
      selection_anchors: JSON.parse(window.__chart.wasm.selection_anchor_identities_json()),
      expected_price: hovered.coordinate_to_price(context.point.y),
      expected_time: window.__chart.time_scale().coordinate_to_time(context.point.x),
      expected_logical: window.__chart.time_scale().coordinate_to_logical(context.point.x),
      default_price: window.__chart.coordinate_to_price(context.point.y),
    };
  });
  expect(dispatch).toEqual({ default_prevented: true, hover_series: null });
  expect(result.clicks).toBe(0);
  expect(result.selection_anchors).toEqual([]);
  expect(result.contexts).toHaveLength(1);
  expect(result.contexts[0]).toMatchObject({
    pane_index: 0,
    hovered_series: target.series_id,
    hovered_object_id: null,
  });
  expect(result.contexts[0].price).toBeCloseTo(result.expected_price, 9);
  expect(result.contexts[0].time).toBe(result.expected_time);
  expect(result.contexts[0].logical).toBe(result.expected_logical);
  expect(Math.abs(result.contexts[0].price - result.default_price)).toBeGreaterThan(100);
  expect(Math.abs(result.contexts[0].point.x - target.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(result.contexts[0].point.y - target.y)).toBeLessThanOrEqual(1);
});

test("hoveredSeriesOnTop repaints the hovered series above an overlapping one", async ({ page }) => {
  await goto_fixture(page);
  // No crosshair pixels in the sampled band.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));

  // Two flat lines, red below blue, strokes overlapping. They sit ABOVE every visible
  // candle high so the hover probe has exactly one distance-0 hit — browser-delivered
  // pointer coordinates are float32, which lands a hair off the exact line y, and an
  // in-range candle (distance 0) would legitimately out-arbitrate that epsilon (reference
  // distance decides). Blue's value is derived on the settled scale (adding blue nudges
  // the autoscale, so the second read is the exact one).
  const setup = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    let max_high = -Infinity;
    for (let i = Math.ceil(range.from); i <= Math.floor(range.to); i++) {
      const bar = window.__main.data_by_index(i);
      if (bar) max_high = Math.max(max_high, bar.high);
    }
    const line = max_high * 1.03;
    const red = window.__chart.add_series("line", { color: "#ff0000", line_width: 6 });
    red.set_data(window.__data.map((d) => ({ time: d.time, value: line })));
    const blue = window.__chart.add_series("line", { color: "#0000ff", line_width: 6 });
    blue.set_data(window.__data.map((d) => ({ time: d.time, value: line })));
    return {
      red: red.id,
      blue: blue.id,
      line,
      x: window.__chart.time_scale().logical_to_coordinate(Math.floor((range.from + range.to) / 2)),
      order_before: window.__chart.series_order().map((s) => s.id),
    };
  });
  // Lift blue ~4 css px above red on the current scale; re-read red's y after the repaint
  // so the probe/sample coordinates match the settled frame exactly.
  setup.y_red = await page.evaluate(({ line, red, blue }) => {
    const red_api = window.__chart.series_order().find((series) => series.id === red);
    const blue_api = window.__chart.series_order().find((series) => series.id === blue);
    const y_red = red_api.price_to_coordinate(line);
    blue_api.set_data(window.__data.map((d) => ({ time: d.time, value: red_api.coordinate_to_price(y_red - 4) })));
    return red_api.price_to_coordinate(line);
  }, setup);
  await settle_frames(page);

  const pixel_ratio = fixture.pixel_ratio;
  const sample = async () => {
    const png = PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
    const dx = Math.round(setup.x * pixel_ratio);
    const dy = Math.round((setup.y_red - 2) * pixel_ratio); // inside both strokes
    const idx = (dy * png.width + dx) * 4;
    return { r: png.data[idx], g: png.data[idx + 1], b: png.data[idx + 2] };
  };
  const is_red = (p) => p.r > 180 && p.g < 100 && p.b < 100;
  const is_blue = (p) => p.b > 180 && p.r < 100 && p.g < 100;

  // Blue was added later: it paints on top of the overlap band.
  expect(is_blue(await sample()), "blue on top before any hover").toBe(true);

  // Hovering the red line bumps it above blue for the render only.
  await page.mouse.move(setup.x, setup.y_red);
  await settle_frames(page);
  expect(is_red(await sample()), "red on top while hovered").toBe(true);
  // The stable paint order is untouched by the bump (reference render-order-only semantics).
  const order_during = await page.evaluate(() => window.__chart.series_order().map((s) => s.id));
  expect(order_during).toEqual(setup.order_before);

  // Moving far above both strokes (still inside the pane) releases the bump: blue returns.
  await page.mouse.move(setup.x, 5);
  await settle_frames(page);
  expect(is_blue(await sample()), "blue back on top after the hover cleared").toBe(true);
});

test("a top-layer pane primitive hit beats the series hit tests", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__pw_handle = window.__chart.panes()[0].attach_primitive({
      pane_views: () => [],
      hit_test: () => ({ external_id: "pw-pane-hit", cursor_style: "grab", z_order: "top" }),
    });
  });
  await collect_hover_events(page);

  // Straight over a candle the primitive still wins, and a pane primitive reports no series.
  const spot = await visible_bar_spot(page);
  await page.mouse.move(spot.x, spot.y);
  const hit = await last_hover(page);
  expect(hit.obj).toBe("pw-pane-hit");
  expect(hit.series).toBeNull();
  expect(await overlay_cursor(page)).toBe("grab");
});

test("a bottom-layer pane primitive hit loses to any series hit", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__pw_handle = window.__chart.panes()[0].attach_primitive({
      pane_views: () => [],
      hit_test: () => ({ external_id: "pw-bottom-hit", z_order: "bottom" }),
    });
  });
  await collect_hover_events(page);

  // Over a candle the series wins (reference: bottom-layer primitive hits are last resort).
  const spot = await visible_bar_spot(page);
  await page.mouse.move(spot.x, spot.y);
  const hit = await last_hover(page);
  const main_id = await page.evaluate(() => window.__main.id);
  expect(hit.series).toBe(main_id);
  expect(hit.obj).toBeNull();

  // Over empty space the bottom-layer hit surfaces.
  const empty = await empty_spot(page);
  await page.mouse.move(empty.x, empty.y);
  const cleared = await last_hover(page);
  expect(cleared.obj).toBe("pw-bottom-hit");
  expect(cleared.series).toBeNull();
});
