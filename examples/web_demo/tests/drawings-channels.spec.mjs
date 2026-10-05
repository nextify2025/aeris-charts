import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Channels family (parallel channel, regression trend, flat top and flat bottom channels,
// disjoint channel, price channel) through the public API and real pointer input: armed placement
// (three clicks, two for the regression, four for the disjoint channel), the parallel line through
// the third anchor as a hover and drag target, the selection-only fill surface, the engine's
// regression fit and its flat options, persistence, clipboard and sync round trips, the demo
// toolbar entries, and WebGPU == Canvas2D parity. Every geometry and statistics decision is
// engine-owned; these specs only drive the package API and pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const CHANNELS = [
  "parallel_channel",
  "regression_trend",
  "flat_top_channel",
  "flat_bottom_channel",
  "disjoint_channel",
  "price_channel",
];
const PINK = [233, 30, 99]; // #e91e63 — collides with no fixture pixel

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page, backend = "canvas2d") {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await settle_frames(page);
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

/** Device-px bounding box of `target`-colored pixels, or null when none. */
function color_extent(png, target, tol = 12) {
  let box = null;
  for (let y = 0; y < png.height; y += 1) {
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol &&
        Math.abs(png.data[o + 1] - target[1]) <= tol &&
        Math.abs(png.data[o + 2] - target[2]) <= tol
      ) {
        box = box
          ? { min_x: Math.min(box.min_x, x), max_x: Math.max(box.max_x, x), min_y: Math.min(box.min_y, y), max_y: Math.max(box.max_y, y) }
          : { min_x: x, max_x: x, min_y: y, max_y: y };
      }
    }
  }
  return box;
}

/** First device-px row in column `x` holding a pink pixel, or null. */
function pink_row(png, x, tol = 12) {
  for (let y = 0; y < png.height; y += 1) {
    const o = (y * png.width + x) * 4;
    if (
      Math.abs(png.data[o] - PINK[0]) <= tol &&
      Math.abs(png.data[o + 1] - PINK[1]) <= tol &&
      Math.abs(png.data[o + 2] - PINK[2]) <= tol
    ) {
      return y;
    }
  }
  return null;
}

/** Vertical center of the first pink run in device column `x`, or null. */
function pink_center(png, x, tol = 12) {
  const top = pink_row(png, x, tol);
  if (top === null) return null;
  let bottom = top;
  for (let y = top + 1; y < png.height; y += 1) {
    const o = (y * png.width + x) * 4;
    if (
      Math.abs(png.data[o] - PINK[0]) > tol ||
      Math.abs(png.data[o + 1] - PINK[1]) > tol ||
      Math.abs(png.data[o + 2] - PINK[2]) > tol
    ) break;
    bottom = y;
  }
  return (top + bottom + 1) / 2;
}

/** CSS-px y at CSS-px `x` of the only pink line, fitted through its dashes right of `x`. */
function pink_line_y(png, x) {
  const samples = [];
  const first = Math.ceil(x * PR) + 1;
  for (let column = first; column < first + 80 * PR; column += 1) {
    const y = pink_center(png, column);
    if (y !== null) samples.push([column + 0.5, y]);
  }
  const n = samples.length;
  const mx = samples.reduce((sum, [sx]) => sum + sx, 0) / n;
  const my = samples.reduce((sum, [, sy]) => sum + sy, 0) / n;
  const sxx = samples.reduce((sum, [sx]) => sum + (sx - mx) ** 2, 0);
  const sxy = samples.reduce((sum, [sx, sy]) => sum + (sx - mx) * (sy - my), 0);
  return (my + (sxy / sxx) * (x * PR - mx)) / PR;
}

/** Mid-range visible logical indexes and prices for deterministic anchors. */
async function anchor_spots(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
    const l1 = Math.floor(range.from + (range.to - range.from) * 0.55);
    const b0 = window.__main.data_by_index(l0);
    const b1 = window.__main.data_by_index(l1);
    const low = Math.min(b0.low, b1.low);
    const high = Math.max(b0.high, b1.high);
    return { l0, l1, lm: Math.round((l0 + l1) / 2), p_lo: low, p_hi: high, p_mid: (low + high) / 2 };
  });
}

async function spot(page, logical, price) {
  return page.evaluate(({ logical, price }) => ({
    x: window.__chart.time_scale().logical_to_coordinate(logical),
    y: window.__main.price_to_coordinate(price),
  }), { logical, price });
}

async function drawings(page) {
  return page.evaluate(() => window.__chart.drawings().map((drawing) => ({
    id: drawing.id, kind: drawing.kind(), points: drawing.points(), options: drawing.options(),
  })));
}

async function points_of(page, id) {
  return page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
}

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

async function selected_id(page) {
  return page.evaluate(() => window.__chart.selected_drawing()?.id ?? null);
}

test("every Channels tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);
  for (const [index, kind] of CHANNELS.entries()) {
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    const clicks = {
      regression_trend: [[s.l0, s.p_lo], [s.l1, s.p_hi]],
      flat_bottom_channel: [[s.l0, s.p_mid], [s.l1, s.p_hi], [s.lm, s.p_lo]],
      disjoint_channel: [[s.l0, s.p_lo], [s.l1, s.p_mid], [s.l0, s.p_hi], [s.l1, s.p_mid]],
    }[kind] ?? [[s.l0, s.p_lo], [s.l1, s.p_hi], [s.lm, s.p_hi]];
    for (const [step, [logical, price]] of clicks.entries()) {
      const point = await spot(page, logical, price);
      if (step === 1 && index === 0) {
        // Between the first and second click the base line previews to the pointer (checked on
        // the first tool, before any other pink drawing exists).
        await page.mouse.move(point.x, point.y, { steps: 2 });
        await settle_frames(page);
        const png = await capture(page);
        const preview = color_extent(png, PINK);
        const first = await spot(page, clicks[0][0], clicks[0][1]);
        expect(preview, `${kind} previews its base line`).not.toBeNull();
        expect(preview.max_x).toBeGreaterThanOrEqual(Math.floor(point.x * PR) - 2);
        // The first anchor's handle covers the line's start; its middle is the segment's.
        const middle_row = pink_row(png, Math.round(((first.x + point.x) / 2) * PR));
        expect(middle_row).not.toBeNull();
        expect(Math.abs(middle_row / PR - (first.y + point.y) / 2)).toBeLessThan(3);
      }
      await page.mouse.click(point.x, point.y);
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(index + 1);
    expect(list[index].kind).toBe(kind);
    expect(list[index].points).toHaveLength(clicks.length);
    expect(list[index].options.color).toBe("#e91e63");
    // Every channel fills by default except KLineChart's price channel, which is three bare lines.
    expect(list[index].options.fill_enabled).toBe(kind !== "price_channel");
    for (const point of list[index].points) expect(Number.isFinite(point.time)).toBe(true);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const pixels = await capture(page);
    const diff = pixelmatch(clean.data, pixels.data, null, clean.width, clean.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
  }
  // Earlier builds' `flat_top_bottom` spelling arms the flat top channel.
  expect(await page.evaluate(() => {
    window.__chart.set_drawing_tool("flat_top_channel");
    const armed = window.__chart.active_drawing_tool();
    window.__chart.set_drawing_tool(null);
    return armed;
  })).toBe("flat_top_channel");
});

test("a parallel channel hovers and drags by its parallel line and selects its fill only once selected", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("parallel_channel", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.l1, price: s.p_mid },
    { logical: s.lm, price: s.p_hi },
  ], { color: "#e91e63" }).id, { s });
  await settle_frames(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.l1, s.p_mid);
  const c = await spot(page, s.lm, s.p_hi);
  const slope = (b.y - a.y) / (b.x - a.x);
  const offset = c.y - (a.y + slope * (c.x - a.x));
  expect(Math.abs(offset)).toBeGreaterThan(12);
  const on_parallel = (x) => ({ x, y: a.y + slope * (x - a.x) + offset });
  const inside = (x) => ({ x, y: a.y + slope * (x - a.x) + offset * 0.25 });

  // The parallel line through the third anchor is a hover target away from every anchor.
  const hover = on_parallel(a.x + (b.x - a.x) * 0.2);
  await page.mouse.move(hover.x, hover.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");

  // Inside the fill: an unselected channel lets the press through; selected, it is a body.
  const middle = inside(a.x + (b.x - a.x) * 0.3);
  await page.mouse.click(middle.x, middle.y);
  expect(await selected_id(page)).toBeNull();
  await page.waitForTimeout(700);
  await page.mouse.click(hover.x, hover.y);
  expect(await selected_id(page)).toBe(id);
  const before = await points_of(page, id);
  await page.mouse.move(middle.x, middle.y);
  await page.mouse.down();
  await page.mouse.move(middle.x + 20, middle.y - 15, { steps: 4 });
  await page.mouse.move(middle.x + 40, middle.y - 30, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await points_of(page, id);
  for (const [index, point] of moved.entries()) {
    expect(point.logical).toBeGreaterThan(before[index].logical);
    expect(point.price).toBeGreaterThan(before[index].price);
  }
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  await settle_frames(page);

  // The third anchor's handle sits on the parallel line's midpoint; dragging it moves only the
  // parallel line, as one undo step.
  const handle = on_parallel((a.x + b.x) / 2);
  await page.mouse.move(handle.x, handle.y);
  await expect.poll(() => overlay_cursor(page)).toBe("pointer");
  await page.mouse.down();
  await page.mouse.move(handle.x, handle.y + 10, { steps: 3 });
  await page.mouse.move(handle.x, handle.y + 24, { steps: 3 });
  await page.mouse.up();
  await settle_frames(page);
  const dragged = await points_of(page, id);
  expect(dragged[0]).toEqual(before[0]);
  expect(dragged[1]).toEqual(before[1]);
  expect(dragged[2].price).toBeLessThan(before[2].price);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points_of(page, id)).toEqual(before);

  // Extended both ways, the channel's lines reach the pane edges.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id)
      .apply_options({ extend_left: true, extend_right: true });
  }, id);
  await settle_frames(page);
  const width = await page.evaluate(() => window.__chart.time_scale().width());
  const extent = color_extent(await capture(page), PINK);
  expect(extent.min_x).toBeLessThanOrEqual(2);
  expect(extent.max_x).toBeGreaterThanOrEqual(Math.floor((width - 2) * PR));
});

test("a regression trend paints its fit with flat, atomic options", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const info = await page.evaluate(({ s }) => {
    const drawing = window.__chart.add_drawing("regression_trend", [
      { logical: s.l0, price: s.p_mid },
      { logical: s.l1, price: s.p_mid },
    ], { color: "#e91e63" });
    return { id: drawing.id, kind_options: window.__chart.drawing_kind_options(drawing) };
  }, { s });
  expect(info.kind_options).toEqual({ kind: "regression_trend", source_id: null, deviations: 2 });
  await settle_frames(page);
  const wide = color_extent(await capture(page), PINK);
  expect(wide).not.toBeNull();

  // Narrower deviations pull the outer lines in; the anchors never move vertically.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id)
      .apply_options({ regression_deviations: 0.5 });
  }, info.id);
  await settle_frames(page);
  const narrow = color_extent(await capture(page), PINK);
  expect(narrow.max_y - narrow.min_y).toBeLessThan(wide.max_y - wide.min_y);
  const options = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    return { options: drawing.options(), kind_options: window.__chart.drawing_kind_options(drawing) };
  }, info.id);
  expect(options.options.regression_deviations).toBe(0.5);
  expect(options.kind_options.deviations).toBe(0.5);

  // Earlier builds' deviation keys fold into the flat option (the wider side).
  const legacy = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    drawing.apply_options({ tool_options: { channel: { upper_deviation: 1.5, lower_deviation: -1 } } });
    return { deviations: window.__chart.drawing_kind_options(drawing).deviations, tool_options: drawing.options().tool_options };
  }, info.id);
  expect(legacy.deviations).toBe(1.5);
  expect(legacy.tool_options).toEqual({});

  const schema = await page.evaluate((id) => window.__chart.drawing_property_schema(
    window.__chart.drawings().find((drawing) => drawing.id === id),
  ), info.id);
  const deviations = schema.properties.find((property) => property.name === "regression_deviations");
  expect(deviations.default).toBe(2);
  expect(schema.properties.some((property) => property.name.startsWith("tool_options.channel."))).toBe(false);

  // An invalid value is rejected atomically.
  const rejected = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    try {
      drawing.apply_options({ width: 7, regression_deviations: 1000 });
    } catch (error) {
      return { code: error.code, width: drawing.options().width };
    }
    return { code: null, width: drawing.options().width };
  }, info.id);
  expect(rejected.code).toBe("invalid_options");
  expect(rejected.width).toBe(1);
});

test("a regression trend drags as one body, edits its window by its anchors, and follows a streaming update", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const last = await page.evaluate(() => {
    const data = window.__main.data();
    return { index: data.length - 1, bar: data[data.length - 1] };
  });
  // Zero deviations and no fill: the fitted center line is the only pink.
  const id = await page.evaluate(({ s, last }) => window.__chart.add_drawing("regression_trend", [
    { logical: s.l0, price: s.p_mid },
    { logical: last.index, price: s.p_mid },
  ], { color: "#e91e63", regression_deviations: 0, fill_enabled: false }).id, { s, last });
  await settle_frames(page);
  const before = await points_of(page, id);
  const png = await capture(page);
  const extent = color_extent(png, PINK);
  expect(extent).not.toBeNull();
  const probe = Math.round(extent.min_x + (extent.max_x - extent.min_x) * 0.35);
  let grab = null;
  for (let x = probe; !grab && x < probe + 16; x += 1) {
    const y = pink_row(png, x);
    if (y !== null) grab = { x: x / PR, y: y / PR };
  }
  expect(grab).not.toBeNull();
  await page.mouse.move(grab.x, grab.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  const bar = await page.evaluate(() => {
    const scale = window.__chart.time_scale();
    return scale.logical_to_coordinate(1) - scale.logical_to_coordinate(0);
  });
  // A body drag moves both anchors on both axes (upstream's free anchors), as one undo step.
  await page.mouse.down();
  await page.mouse.move(grab.x - bar * 2, grab.y - 40, { steps: 4 });
  await page.mouse.move(grab.x - bar * 4, grab.y - 80, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await points_of(page, id);
  for (const [index, point] of moved.entries()) {
    expect(point.logical).toBeLessThan(before[index].logical);
    expect(point.price).toBeGreaterThan(before[index].price);
  }
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points_of(page, id)).toEqual(before);
  await settle_frames(page);

  // Selected, its handles sit on its anchors; dragging the start handle shortens the window.
  const start = await spot(page, s.l0, s.p_mid);
  await page.evaluate((id) => {
    window.__chart.wasm.set_selected_drawing(id);
    window.__chart.render();
  }, id);
  await settle_frames(page);
  await page.mouse.move(start.x, start.y);
  await expect.poll(() => overlay_cursor(page)).toBe("pointer");
  await page.mouse.down();
  await page.mouse.move(start.x + bar, start.y, { steps: 3 });
  await page.mouse.move(start.x + bar * 3, start.y, { steps: 3 });
  await page.mouse.up();
  await settle_frames(page);
  const shortened = await points_of(page, id);
  expect(shortened[0].logical).toBeGreaterThan(before[0].logical);
  expect(shortened[1]).toEqual(before[1]);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points_of(page, id)).toEqual(before);
  await page.evaluate(() => {
    window.__chart.wasm.set_selected_drawing(undefined);
    window.__chart.render();
  });
  await settle_frames(page);

  // A streaming update of the last bar re-fits the line.
  const fitted = color_extent(await capture(page), PINK);
  await page.evaluate(({ last }) => {
    const bar = last.bar;
    const high = bar.high + (bar.high - bar.low) * 40;
    window.__main.update({ time: bar.time, open: bar.open, high, low: bar.low, close: high });
  }, { last });
  await settle_frames(page);
  const refit = color_extent(await capture(page), PINK);
  expect(refit).not.toEqual(fitted);
});

test("Channels round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = () => {
      const element = document.createElement("div");
      element.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
      document.body.append(element);
      return element;
    };
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const additions = [
      ["parallel_channel", [{ logical: 1, price: 10 }, { logical: 5, price: 12 }, { logical: 3, price: 13 }], { extend_right: true, tool_options: { channel: { middle_line: false } } }],
      ["regression_trend", [{ logical: 0, price: 10 }, { logical: 6, price: 10 }], { regression_deviations: 1.5, tool_options: { channel: { source: "hlc3", show_pearsons: false } } }],
      ["flat_bottom_channel", [{ logical: 2, price: 9 }, { logical: 6, price: 12 }, { logical: 4, price: 8 }], { fill_enabled: false, style: "dashed" }],
      ["disjoint_channel", [{ logical: 1, price: 11 }, { logical: 4, price: 12 }, { logical: 1, price: 9 }, { logical: 4, price: 10 }], { fill_color: "#00ff0040", tool_options: { channel: { middle_line: true, middle_color: "#ff9800" } } }],
    ];
    for (const [kind, anchors, style] of additions) first.add_drawing(kind, anchors, style);
    const state = first.export_state();
    const copied = first.copy_drawings(first.drawings().map((drawing) => drawing.id));
    const sync = first.drawing_sync_payload("cell-a");
    const expected = first.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options() }));
    first.remove();
    first_host.remove();

    const second_host = host();
    const second = await create_chart(second_host, { backend: "canvas2d", autoSize: false });
    second.import_state(state);
    const canonical = second.export_state();
    const restored = second.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options() }));
    second.remove();
    second_host.remove();

    const third_host = host();
    const third = await create_chart(third_host, { backend: "canvas2d", autoSize: false });
    const pasted = third.paste_drawings(copied).map((drawing) => ({ kind: drawing.kind(), options: drawing.options() }));
    third.remove();
    third_host.remove();

    const fourth_host = host();
    const fourth = await create_chart(fourth_host, { backend: "canvas2d", autoSize: false });
    const applied = fourth.apply_drawing_sync_payload(sync);
    const synced = fourth.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options() }));
    fourth.remove();
    fourth_host.remove();
    return { state, canonical, expected, restored, pasted, applied, synced };
  });

  expect(result.canonical).toEqual(result.state);
  const styles = result.state.drawings.map((drawing) => drawing.style);
  expect(styles[0].extend_right).toBe(true);
  expect(styles[0].fill_enabled).toBeUndefined();
  expect(styles[0].tool_options).toEqual({ channel: { middle_line: false } });
  expect(styles[1].regression_deviations).toBe(1.5);
  expect(styles[1].tool_options).toEqual({ channel: { source: "hlc3", show_pearsons: false } });
  expect(styles[2].fill_enabled).toBe(false);
  expect(styles[3].tool_options).toEqual({ channel: { middle_line: true, middle_color: "#ff9800" } });
  const semantic = (list) => list.map(({ kind, options }) => ({
    kind,
    extend_left: options.extend_left,
    extend_right: options.extend_right,
    fill_enabled: options.fill_enabled,
    fill_color: options.fill_color,
    tool_options: options.tool_options,
    style: options.style,
    width: options.width,
    regression_deviations: options.regression_deviations,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("a named template replaces a regression trend's options", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const result = await page.evaluate(({ s }) => {
    const chart = window.__chart;
    const anchors = [{ logical: s.l0, price: s.p_mid }, { logical: s.l1, price: s.p_mid }];
    const custom = chart.add_drawing("regression_trend", anchors, {
      width: 3,
      regression_deviations: 3,
      tool_options: { channel: { source: "hl2" } },
    });
    const quiet = chart.add_drawing("regression_trend", anchors, {
      regression_deviations: 1,
      tool_options: { channel: { show_pearsons: false } },
    });
    const plain = chart.add_drawing("regression_trend", anchors);
    const read = (drawing) => {
      const current = chart.drawings().find((candidate) => candidate.id === drawing.id);
      return { width: current.options().width, tool_options: current.options().tool_options, kind_options: chart.drawing_kind_options(current) };
    };
    chart.apply_drawing_template(custom, chart.drawing_template(plain, "Plain"));
    const reset = read(custom);
    chart.apply_drawing_template(custom, chart.drawing_template(quiet, "Quiet"));
    const replaced = read(custom);
    return { reset, plain: read(plain), replaced };
  }, { s });
  expect(result.reset.width).toBe(1);
  expect(result.reset.tool_options).toEqual({});
  expect(result.reset.kind_options).toEqual(result.plain.kind_options);
  expect(result.replaced.tool_options).toEqual({ channel: { show_pearsons: false } });
  expect(result.replaced.kind_options.deviations).toBe(1);
});

test("the demo toolbar arms every Channels tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of CHANNELS) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  // Clicking the armed tool again disarms it.
  await page.click(`#drawings_group [data-tool='${CHANNELS[CHANNELS.length - 1]}']`);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Channels render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
  const run_scenario = async (backend) => {
    await goto_fixture(page, backend);
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await page.evaluate(() => {
      const chart = window.__chart;
      const range = chart.time_scale().get_visible_logical_range();
      const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
      const b0 = window.__main.data_by_index(at(0.2));
      const b1 = window.__main.data_by_index(at(0.6));
      const lo = Math.min(b0.low, b1.low);
      const hi = Math.max(b0.high, b1.high);
      const up = (fraction) => lo + (hi - lo) * fraction;
      chart.add_drawing("parallel_channel", [{ logical: at(0.1), price: up(0.1) }, { logical: at(0.3), price: up(0.4) }, { logical: at(0.2), price: up(0.6) }], { color: "#e91e63", width: 2 });
      chart.add_drawing("regression_trend", [{ logical: at(0.35), price: up(0.5) }, { logical: at(0.6), price: up(0.5) }], { color: "#089981" });
      chart.add_drawing("flat_top_channel", [{ logical: at(0.62), price: up(0.2) }, { logical: at(0.8), price: up(0.6) }, { logical: at(0.7), price: up(0.4) }], { color: "#7b1fa2" });
      chart.add_drawing("disjoint_channel", [{ logical: at(0.82), price: up(0.5) }, { logical: at(0.95), price: up(0.7) }, { logical: at(0.82), price: up(0.3) }, { logical: at(0.95), price: up(0.35) }], { color: "#ff6d00", style: "dotted" });
      chart.add_drawing("price_channel", [{ logical: at(0.1), price: up(0.55) }, { logical: at(0.3), price: up(0.65) }, { logical: at(0.2), price: up(0.4) }], { color: "#2962ff", width: 2 });
      const first = chart.drawings()[0];
      chart.wasm.set_selected_drawing(first.id);
      chart.render();
    });
    await settle_frames(page);
    return {
      backend: await page.evaluate(() => window.__chart.backend()),
      png: PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false })),
    };
  };

  const canvas = await run_scenario("canvas2d");
  expect(canvas.backend).toBe("canvas2d");
  const gpu = await run_scenario("auto");
  expect(gpu.backend).toBe("webgpu");
  expect([canvas.png.width, canvas.png.height]).toEqual([gpu.png.width, gpu.png.height]);

  const clean = await (async () => {
    await goto_fixture(page, "canvas2d");
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await settle_frames(page);
    return PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  })();
  const differs = (a, b) => pixelmatch(a.data, b.data, null, a.width, a.height, { threshold: 0, includeAA: true });
  expect(differs(clean, canvas.png), "Channels paint on Canvas2D").toBeGreaterThan(1000);
  expect(differs(clean, gpu.png), "Channels paint on WebGPU").toBeGreaterThan(1000);

  // The repository ordering contract (drawings.spec.mjs): only anti-aliasing coverage steps may
  // differ; anything above 128 per channel is wrong geometry or paint order.
  let ordering_diff = 0;
  let edge_diff = 0;
  for (let offset = 0; offset < canvas.png.data.length; offset += 4) {
    let delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      delta = Math.max(delta, Math.abs(canvas.png.data[offset + channel] - gpu.png.data[offset + channel]));
    }
    if (delta > 128) ordering_diff += 1;
    else if (delta !== 0) edge_diff += 1;
  }
  console.log(`channels parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["canvas2d.png", canvas.png], ["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "Channels geometry and paint order match across executors").toBe(0);
});
