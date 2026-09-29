import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Projection & Annotations family through the public API and real pointer input: armed
// placement and painting of every tool, the demo toolbar, forecast outcomes, bars-pattern capture
// and body drags, range stats boxes and fills as hit targets, pane-anchored text that stays put
// while the chart scrolls, persistence/clipboard/sync round trips, and WebGPU == Canvas2D parity.
// Every geometry decision is engine-owned; these specs only drive the package API and pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const TOOLS = [
  "forecast",
  "bars_pattern",
  "price_range",
  "date_range",
  "date_and_price_range",
  "projection",
  "anchored_text",
  "note",
  "price_note",
  "callout",
  "comment",
  "price_label",
  "signpost",
  "flag_mark",
  "arrow_mark_up",
  "arrow_mark_down",
  "arrow_mark_left",
  "arrow_mark_right",
  "icon",
];
const ANCHORS = { projection: 3, forecast: 2, bars_pattern: 2, price_range: 2, date_range: 2, date_and_price_range: 2, price_note: 2, callout: 2 };
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
        box = box === null
          ? { left: x, right: x, top: y, bottom: y }
          : { left: Math.min(box.left, x), right: Math.max(box.right, x), top: Math.min(box.top, y), bottom: Math.max(box.bottom, y) };
      }
    }
  }
  return box;
}

/** Mean RGB of the device-px rectangle around a CSS-px point. */
function mean_color(png, css_x, css_y, css_w, css_h) {
  const sum = [0, 0, 0];
  let count = 0;
  for (let y = Math.round(css_y * PR); y < Math.round((css_y + css_h) * PR); y += 1) {
    for (let x = Math.round(css_x * PR); x < Math.round((css_x + css_w) * PR); x += 1) {
      const o = (y * png.width + x) * 4;
      for (let channel = 0; channel < 3; channel += 1) sum[channel] += png.data[o + channel];
      count += 1;
    }
  }
  return sum.map((value) => value / count);
}

async function spot(page, logical, price) {
  return page.evaluate(({ logical, price }) => ({
    x: window.__chart.time_scale().logical_to_coordinate(logical),
    y: window.__main.price_to_coordinate(price),
  }), { logical, price });
}

async function bar(page, logical) {
  return page.evaluate((logical) => window.__main.data_by_index(logical), logical);
}

async function visible_range(page) {
  return page.evaluate(() => window.__chart.time_scale().get_visible_logical_range());
}

async function drawings(page) {
  return page.evaluate(() => window.__chart.drawings().map((drawing) => ({
    id: drawing.id, kind: drawing.kind(), points: drawing.points(), options: drawing.options(),
  })));
}

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

test("every Projection & Annotations tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  let previous = await capture(page);
  for (const [index, kind] of TOOLS.entries()) {
    const l0 = Math.floor(range.from + (range.to - range.from) * (0.06 + index * 0.046));
    const b0 = await bar(page, l0);
    const b1 = await bar(page, l0 + 4);
    const count = ANCHORS[kind] ?? 1;
    const spots = count === 1
      ? [[l0, b0.high]]
      : count === 2
        ? [[l0, b0.low], [l0 + 4, b1.high]]
        : [[l0, b0.close], [l0 + 5, b0.close], [l0 + 4, b1.high]];
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    for (const [logical, price] of spots) {
      const point = await spot(page, logical, price);
      await page.mouse.click(point.x, point.y);
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(index + 1);
    const drawing = list[index];
    expect(drawing.kind).toBe(kind);
    expect(drawing.points).toHaveLength(count);
    expect(drawing.options.color).toBe("#e91e63");
    for (const point of drawing.points) {
      if (kind === "anchored_text") {
        // Pane fractions carry no time identity.
        expect(point.time).toBeUndefined();
        expect(point.logical).toBeGreaterThanOrEqual(0);
        expect(point.logical).toBeLessThanOrEqual(1);
        expect(point.price).toBeGreaterThanOrEqual(0);
        expect(point.price).toBeLessThanOrEqual(1);
      } else {
        expect(Number.isFinite(point.time)).toBe(true);
      }
    }
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const pixels = await capture(page);
    const diff = pixelmatch(previous.data, pixels.data, null, pixels.width, pixels.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
    previous = pixels;
  }
});

test("the demo toolbar arms every Projection & Annotations tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of TOOLS) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  await page.click("#drawings_group [data-tool='icon']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("the demo toolbar keeps each tool's catalog defaults and templates only edited settings", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await settle_frames(page);
  const offset = await page.evaluate(() => {
    const rect = document.getElementById("chart_container").getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  });
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.4);
  const b0 = await bar(page, l0);
  const b1 = await bar(page, l0 + 4);
  const spots = {
    1: [[l0, b0.high]],
    2: [[l0, b0.low], [l0 + 4, b1.high]],
    3: [[l0, b0.close], [l0 + 5, b0.close], [l0 + 4, b1.high]],
  };
  const clicks = { arrow_mark_up: 1, arrow_mark_down: 1, anchored_text: 1, callout: 2, fib_retracement: 2, andrews_pitchfork: 3 };
  const place = async (kind, arm) => {
    await arm();
    for (const [logical, price] of spots[clicks[kind]]) {
      const point = await spot(page, logical, price);
      await page.mouse.click(point.x + offset.left, point.y + offset.top);
    }
    await settle_frames(page);
    const created = (await drawings(page)).at(-1);
    await page.evaluate(() => window.__chart.clear_drawings());
    expect(created?.kind, kind).toBe(kind);
    const o = created.options;
    return { color: o.color, width: o.width, style: o.style, text_size: o.text_size, text_h_align: o.text_h_align, text_v_align: o.text_v_align };
  };
  const toolbar = (kind) => () => page.click(`#drawings_group [data-tool='${kind}']`);
  const bare = (kind) => () => page.evaluate((kind) => window.__chart.set_drawing_tool(kind), kind);
  for (const kind of Object.keys(clicks)) {
    expect(await place(kind, toolbar(kind)), kind).toEqual(await place(kind, bare(kind)));
  }
  // Spot checks of the catalog defaults the untouched toolbar used to overwrite.
  const up = await place("arrow_mark_up", toolbar("arrow_mark_up"));
  const down = await place("arrow_mark_down", toolbar("arrow_mark_down"));
  expect(up.color).not.toBe(down.color);
  expect((await place("fib_retracement", toolbar("fib_retracement"))).style).toBe("dashed");
  expect((await place("anchored_text", toolbar("anchored_text"))).text_h_align).toBe("left");

  // An explicitly edited setting templates the next drawing; the rest stay the tool's own.
  await page.evaluate(() => {
    const input = document.getElementById("drawing_color");
    input.value = "#ff00ff";
    input.dispatchEvent(new Event("change"));
  });
  const edited = await place("arrow_mark_up", toolbar("arrow_mark_up"));
  expect(edited).toEqual({ ...up, color: "#ff00ff" });
});

test("a forecast turns its target box into the evaluated outcome and selects from it", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
  const source = await bar(page, l0);
  let reachable = -Infinity;
  let lowest = Infinity;
  for (let offset = 1; offset <= 10; offset += 1) {
    const next = await bar(page, l0 + offset);
    reachable = Math.max(reachable, next.high);
    lowest = Math.min(lowest, next.low);
  }
  // A blue forecast: pending boxes stay blue, outcomes turn green or red.
  const id = await page.evaluate(({ l0, source, reachable }) => window.__chart.add_drawing("forecast", [
    { logical: l0, price: source.close },
    { logical: l0 + 10, price: reachable },
  ], { color: "#2962ff" }).id, { l0, source, reachable });
  await settle_frames(page);
  const target = await spot(page, l0 + 10, reachable);
  const success = mean_color(await capture(page), target.x + 12, target.y - 3, 14, 6);
  expect(success[1], `success box is green: ${success}`).toBeGreaterThan(success[0]);
  expect(success[1], `success box is green: ${success}`).toBeGreaterThan(success[2]);

  // Unreachable within the window: the data passes the target bar, so the box reports failure.
  const unreachable = reachable + (reachable - lowest) * 0.3;
  await page.evaluate(({ id, l0, source, unreachable }) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).set_points([
      { logical: l0, price: source.close },
      { logical: l0 + 10, price: unreachable },
    ]);
  }, { id, l0, source, unreachable });
  await settle_frames(page);
  const failed_target = await spot(page, l0 + 10, unreachable);
  expect(failed_target.y, "the failed target stays on the pane").toBeGreaterThan(12);
  const failure = mean_color(await capture(page), failed_target.x + 12, failed_target.y - 3, 14, 6);
  expect(failure[0], `failure box is red: ${failure}`).toBeGreaterThan(failure[1]);
  expect(failure[0], `failure box is red: ${failure}`).toBeGreaterThan(failure[2]);

  // A target beyond the data stays pending in the forecast's own color.
  const future = (await visible_range(page)).to + 30;
  await page.evaluate(({ id, l0, source, reachable, future }) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).set_points([
      { logical: l0, price: source.close },
      { logical: future, price: reachable },
    ]);
    window.__chart.time_scale().set_visible_logical_range({ from: l0 - 10, to: future + 20 });
  }, { id, l0, source, reachable, future });
  await settle_frames(page);
  const pending_target = await spot(page, future, reachable);
  const pending = mean_color(await capture(page), pending_target.x + 12, pending_target.y - 3, 14, 6);
  expect(pending[2], `pending box is blue: ${pending}`).toBeGreaterThan(pending[0]);

  // The target box is a selection target.
  await page.mouse.click(pending_target.x + 24, pending_target.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
});

test("a bars pattern copies its source bars on placement and drags as a rigid ghost", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.35);
  // Zoom in so the copied sticks sit well apart from the anchor handles.
  await page.evaluate((l0) => window.__chart.time_scale().set_visible_logical_range({ from: l0 - 20, to: l0 + 40 }), l0);
  await settle_frames(page);
  const first = await bar(page, l0);
  const last = await bar(page, l0 + 8);
  await page.evaluate(() => window.__chart.set_drawing_tool("bars_pattern", { color: "#e91e63" }));
  for (const [logical, price] of [[l0, first.high], [l0 + 8, last.low]]) {
    const point = await spot(page, logical, price);
    await page.mouse.click(point.x, point.y);
  }
  await settle_frames(page);
  const created = await page.evaluate(() => {
    const drawing = window.__chart.drawings()[0];
    return {
      id: drawing.id,
      points: drawing.points(),
      bars: drawing.options().tool_options.projection_annotation.bars,
      kind_options: window.__chart.drawing_kind_options(drawing),
    };
  });
  expect(created.bars).toHaveLength(9);
  expect(created.bars[0]).toEqual([first.open, first.high, first.low, first.close]);
  expect(created.bars[8][3]).toBe(last.close);
  // The anchors span the copy's box (its first bar at the highest value, its last bar at the
  // lowest), so it starts exactly over its source.
  const values = created.bars.flat();
  expect(created.points[0].logical).toBe(l0);
  expect(created.points[0].price).toBe(Math.max(...values));
  expect(created.points[1].logical).toBe(l0 + 8);
  expect(created.points[1].price).toBe(Math.min(...values));
  expect(created.kind_options).toEqual({
    kind: "projection_annotation", bars_mode: "hl_bars", mirrored: false, flipped: false, pattern_bars: 9, icon: "star", icon_size: 24,
    always_show_text: false,
  });

  // Grab the middle stick and drag the ghost to the right: it moves rigidly and keeps its copy.
  const middle = await bar(page, l0 + 4);
  const grab = await spot(page, l0 + 4, (middle.high + middle.low) / 2);
  await page.mouse.move(grab.x, grab.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  const before = color_extent(await capture(page), PINK);
  await page.mouse.down();
  await page.mouse.move(grab.x + 60, grab.y - 20, { steps: 4 });
  await page.mouse.move(grab.x + 120, grab.y - 40, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    return { points: drawing.points(), bars: drawing.options().tool_options.projection_annotation.bars };
  }, created.id);
  expect(moved.bars).toEqual(created.bars);
  expect(moved.points[0].logical).toBeGreaterThan(l0 + 3);
  const after = color_extent(await capture(page), PINK);
  expect(Math.abs(after.left - before.left - 120 * PR)).toBeLessThanOrEqual(2);
  expect(Math.abs(after.top - before.top + 40 * PR)).toBeLessThanOrEqual(2);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);

  // Line modes repaint the same copy as one polyline.
  const hl = await capture(page);
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id)
      .apply_options({ tool_options: { projection_annotation: { bars_mode: "line_close" } } });
  }, created.id);
  await settle_frames(page);
  const line = await capture(page);
  expect(pixelmatch(hl.data, line.data, null, hl.width, hl.height, { threshold: 0 })).toBeGreaterThan(20);
  const options = await page.evaluate((id) => window.__chart.drawing_kind_options(id), created.id);
  expect(options.bars_mode).toBe("line_close");
  expect(options.pattern_bars).toBe(9);

  // A named template restyles another pattern and keeps that pattern's own copy.
  const templated = await page.evaluate(({ id, from }) => {
    const chart = window.__chart;
    const template = chart.drawing_template(id, "ghost");
    const other = chart.add_drawing("bars_pattern", [{ logical: from, price: 0 }, { logical: from + 3, price: 0 }], {});
    const own = other.options().tool_options.projection_annotation.bars;
    chart.apply_drawing_template(other, template);
    const applied = other.options().tool_options.projection_annotation;
    return { carries_bars: "bars" in (template.options.tool_options?.projection_annotation ?? {}), own, applied };
  }, { id: created.id, from: l0 + 12 });
  expect(templated.carries_bars).toBe(false);
  expect(templated.applied.bars).toEqual(templated.own);
  expect(templated.applied.bars).toHaveLength(4);
  expect(templated.applied.bars_mode).toBe("line_close");
});

test("a projection shows its placed apex and a provisional line between clicks", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
  const b0 = await bar(page, l0);
  const apex = await spot(page, l0, b0.close);
  const before = await capture(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("projection", { color: "#e91e63" }));
  await page.mouse.click(apex.x, apex.y);
  await page.mouse.move(apex.x + 160, apex.y - 60, { steps: 4 });
  await settle_frames(page);
  // The provisional line spans the apex to the pointer in the drawing color.
  const band = color_extent(await capture(page), PINK);
  expect(band).not.toBeNull();
  expect(band.left).toBeLessThanOrEqual(Math.round((apex.x + 10) * PR));
  expect(band.right).toBeGreaterThanOrEqual(Math.round((apex.x + 150) * PR));
  const pending = await capture(page);
  expect(pixelmatch(before.data, pending.data, null, before.width, before.height, { threshold: 0 })).toBeGreaterThan(20);
  // The second click previews the sector and the third commits it.
  await page.mouse.click(apex.x + 160, apex.y - 60);
  await page.mouse.click(apex.x + 120, apex.y - 120);
  await settle_frames(page);
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect(list[0].kind).toBe("projection");
  expect(list[0].points).toHaveLength(3);
});

test("a group move leaves anchored text pinned and the layout restorable", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.4);
  const b0 = await bar(page, l0);
  const result = await page.evaluate(({ l0, price }) => {
    const chart = window.__chart;
    const text = chart.add_drawing("anchored_text", [{ logical: 0.9, price: 0.9 }], { group_id: "g" });
    const note = chart.add_drawing("note", [{ logical: l0, price }], { group_id: "g" });
    const changed = chart.move_drawing_group("g", 5, 1);
    const state = chart.export_state();
    return { changed, text: text.points(), note: note.points(), state };
  }, { l0, price: b0.high });
  expect(result.changed).toBe(1);
  expect(result.text[0].logical).toBe(0.9);
  expect(result.text[0].price).toBe(0.9);
  expect(result.note[0].logical).toBe(l0 + 5);
  // The exported layout restores into a fresh chart (an off-pane fraction would be invalid_data).
  const restored = await page.evaluate(async (state) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
    document.body.append(host);
    const fresh = await create_chart(host, { backend: "canvas2d", autoSize: false });
    fresh.import_state(state);
    const list = fresh.drawings().map((drawing) => ({ kind: drawing.kind(), points: drawing.points() }));
    fresh.remove();
    host.remove();
    return list;
  }, result.state);
  const text = restored.find((drawing) => drawing.kind === "anchored_text");
  expect(text.points[0].logical).toBe(0.9);
  expect(text.points[0].price).toBe(0.9);
});

test("price range stats and fills are selection targets with typed options", async ({ page }) => {
  await goto_fixture(page);
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
  const l1 = Math.floor(range.from + (range.to - range.from) * 0.5);
  const b0 = await bar(page, l0);
  const b1 = await bar(page, l1);
  const low = Math.min(b0.low, b1.low);
  const high = Math.max(b0.high, b1.high);
  const info = await page.evaluate(({ l0, l1, low, high }) => {
    const drawing = window.__chart.add_drawing("price_range", [
      { logical: l0, price: low },
      { logical: l1, price: high },
    ], { color: "#e91e63" });
    return {
      id: drawing.id,
      labels: drawing.options().labels.map((label) => label.metric),
      fill: drawing.options().fill_enabled,
      stroke_end: drawing.options().stroke_end,
    };
  }, { l0, l1, low, high });
  expect(info.labels).toEqual(["price_change", "percent_change", "ticks"]);
  expect(info.fill).toBe(true);
  expect(info.stroke_end).toBe("arrow");
  await settle_frames(page);
  const top = await spot(page, (l0 + l1) / 2, high);
  // The stats box sits above the rising range's upper edge.
  await page.mouse.move(top.x, top.y - 16);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.click(top.x, top.y - 16);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(info.id);
  // The fill is a body target until it is switched off.
  const inside = await spot(page, l0 + (l1 - l0) * 0.2, (low + high) / 2);
  await page.mouse.move(inside.x, inside.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ fill_enabled: false });
  }, info.id);
  await settle_frames(page);
  await page.mouse.move(inside.x + 1, inside.y + 1);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  const schema = await page.evaluate((id) => window.__chart.drawing_property_schema(id), info.id);
  expect(schema.kind).toBe("price_range");
  expect(schema.properties.find((property) => property.name === "fill_enabled").default).toBe(true);
  const icon_schema = await page.evaluate(() => {
    const icon = window.__chart.add_drawing("icon", [{ logical: 5, price: 100 }]);
    return window.__chart.drawing_property_schema(icon);
  });
  const size = icon_schema.properties.find((property) => property.name === "tool_options.projection_annotation.icon_size");
  expect([size.min, size.max, size.default]).toEqual([8, 128, 24]);
});

test("anchored text stays pinned to its pane position while the chart scrolls and drags in pane fractions", async ({ page }) => {
  await goto_fixture(page);
  const id = await page.evaluate(() => window.__chart.add_drawing("anchored_text", [
    { logical: 0.1, price: 0.12 },
  ], { text: "Pinned note", box_color: "#e91e63", text_color: "#ffffff" }).id);
  await settle_frames(page);
  const before = color_extent(await capture(page), PINK);
  expect(before).not.toBeNull();
  const size = await page.evaluate(() => ({ width: window.__chart.time_scale().width() }));
  expect(Math.abs(before.left / PR - 0.1 * size.width)).toBeLessThanOrEqual(2);

  // Scrolling the time scale leaves it exactly in place.
  await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    window.__chart.time_scale().set_visible_logical_range({ from: range.from - 40, to: range.to - 25 });
  });
  await settle_frames(page);
  expect(color_extent(await capture(page), PINK)).toEqual(before);
  const anchors = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  expect(anchors).toEqual([{ logical: 0.1, price: 0.12 }]);
  // Pane fractions outside the pane clamp into it, so the text stays reachable.
  const clamped = await page.evaluate(() => {
    const drawing = window.__chart.add_drawing("anchored_text", [{ logical: 1.5, price: -0.2 }]);
    const points = drawing.points();
    drawing.remove();
    return points;
  });
  expect(clamped).toEqual([{ logical: 1, price: 0 }]);

  // Dragging it moves it by pane fractions.
  const grab = { x: (before.left + before.right) / 2 / PR, y: (before.top + before.bottom) / 2 / PR };
  await page.mouse.move(grab.x, grab.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(grab.x + 60, grab.y + 20, { steps: 4 });
  await page.mouse.move(grab.x + 120, grab.y + 40, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points()[0], id);
  expect(moved.logical).toBeCloseTo(0.1 + 120 / size.width, 6);
  expect(moved.time).toBeUndefined();
  const after = color_extent(await capture(page), PINK);
  expect(Math.abs(after.left - before.left - 120 * PR)).toBeLessThanOrEqual(2);
});

test("a callout edits its tip by pointer and its box by keyboard, and honors hide and lock", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const range = await visible_range(page);
  const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
  const l1 = Math.floor(range.from + (range.to - range.from) * 0.5);
  const b0 = await bar(page, l0);
  const b1 = await bar(page, l1);
  const id = await page.evaluate(({ l0, l1, b0, b1 }) => {
    const chart = window.__chart;
    const drawing = chart.add_drawing("callout", [
      { logical: l0, price: b0.high },
      { logical: l1, price: b1.high },
    ], { color: "#e91e63" });
    chart.wasm.set_selected_drawing(drawing.id);
    chart.render();
    return drawing.id;
  }, { l0, l1, b0, b1 });
  await settle_frames(page);
  const points = () => page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  // Media px of the anchors: x is linear in the (possibly fractional) logical position.
  const spacing = await page.evaluate((l0) => {
    const scale = window.__chart.time_scale();
    return scale.logical_to_coordinate(l0 + 1) - scale.logical_to_coordinate(l0);
  }, l0);
  const x0 = await page.evaluate((l0) => window.__chart.time_scale().logical_to_coordinate(l0), l0);
  const px = () => page.evaluate(({ id, l0, x0, spacing }) => window.__chart.drawings()
    .find((drawing) => drawing.id === id).points().map((point) => ({
      x: x0 + (point.logical - l0) * spacing,
      y: window.__main.price_to_coordinate(point.price),
    })), { id, l0, x0, spacing });
  const start = await px();
  const start_points = await points();

  // Pointer: the tip handle follows the pointer and the box stays; one undo restores it.
  await page.mouse.move(start[0].x, start[0].y);
  await page.mouse.down();
  await page.mouse.move(start[0].x - 20, start[0].y + 15, { steps: 4 });
  await page.mouse.move(start[0].x - 40, start[0].y + 30, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const dragged = await px();
  expect(dragged[0].x).toBeCloseTo(start[0].x - 40, 2);
  expect(dragged[0].y).toBeCloseTo(start[0].y + 30, 2);
  expect((await points())[1]).toEqual(start_points[1]);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points()).toEqual(start_points);

  // Keyboard: Enter edits, Tab cycles to the box handle, and each arrow is one undo step.
  const handles = await page.evaluate((id) => {
    const chart = window.__chart;
    chart.accessibility().focus_target(`drawing:${id}`);
    const layer = document.activeElement;
    const key = (name, shift = false) => layer.dispatchEvent(
      new KeyboardEvent("keydown", { key: name, shiftKey: shift, bubbles: true, cancelable: true }),
    );
    key("Enter");
    key("Tab"); // handle 1: the tip
    key("Tab"); // handle 2: the box
    key("ArrowUp", true); // 10 CSS px
    key("ArrowRight"); // 1 CSS px
    key("Enter");
    return chart.drawing_handle_count(id);
  }, id);
  expect(handles).toBe(2);
  const keyed = await px();
  expect(keyed[1].x).toBeCloseTo(start[1].x + 1, 2);
  expect(keyed[1].y).toBeCloseTo(start[1].y - 10, 2);
  expect((await points())[0]).toEqual(start_points[0]);
  expect(await page.evaluate(() => window.__chart.undo_drawing() && window.__chart.undo_drawing())).toBe(true);
  expect(await points()).toEqual(start_points);

  // Hidden: no paint and no hover target; the drawing stays in the list.
  expect(color_extent(await capture(page), PINK)).not.toBeNull();
  await page.evaluate((id) => {
    const chart = window.__chart;
    chart.wasm.set_selected_drawing(undefined);
    chart.drawings().find((drawing) => drawing.id === id).apply_options({ visible: false });
    chart.render();
  }, id);
  await settle_frames(page);
  expect(color_extent(await capture(page), PINK)).toBeNull();
  await page.mouse.move(start[1].x, start[1].y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  expect(await page.evaluate(() => window.__chart.drawings().length)).toBe(1);

  // Locked: pressing on the box selects it, and a drag never moves it.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ visible: true, locked: true });
  }, id);
  await settle_frames(page);
  await page.mouse.move(start[1].x + 1, start[1].y + 1);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(start[1].x + 60, start[1].y + 40, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  expect(await points()).toEqual(start_points);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
});

test("Projection & Annotations tools round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
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
    const bars = [[10, 12, 9, 11], [11, 13, 10, 12.5], [12.5, 13, 11, 11.5]];
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const additions = [
      ["forecast", [{ logical: 1, price: 10 }, { logical: 4, price: 12 }], { width: 3 }],
      ["bars_pattern", [{ logical: 1, price: 11 }, { logical: 3, price: 11.5 }], { tool_options: { projection_annotation: { bars, bars_mode: "oc_bars", mirrored: true } } }],
      ["price_range", [{ logical: 2, price: 10 }, { logical: 5, price: 13 }], { fill_enabled: false, labels: [] }],
      ["date_and_price_range", [{ logical: 2, price: 11 }, { logical: 6, price: 12 }], { stroke_end: "none" }],
      ["projection", [{ logical: 1, price: 10 }, { logical: 4, price: 10 }, { logical: 3, price: 12 }], { fill_color: "#ff000055" }],
      ["anchored_text", [{ logical: 0.4, price: 0.2 }], { text: "pinned", box_color: "#202020" }],
      ["comment", [{ logical: 3, price: 11 }], { text: "" }],
      ["callout", [{ logical: 2, price: 11 }, { logical: 4, price: 12.5 }], { text: "two\nlines", text_h_align: "left" }],
      ["icon", [{ logical: 5, price: 11 }], { tool_options: { projection_annotation: { icon: "heart", icon_size: 40 } } }],
      ["arrow_mark_down", [{ logical: 6, price: 12 }], { text: "sell" }],
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
    return { state, canonical, expected, restored, pasted, applied, synced, bars };
  });

  expect(result.canonical).toEqual(result.state);
  const styles = result.state.drawings.map((drawing) => drawing.style);
  expect(styles[1].tool_options).toEqual({ projection_annotation: { bars_mode: "oc_bars", mirrored: true, flipped: false, bars: result.bars, icon: "star", icon_size: 24 } });
  expect(styles[2].fill_enabled).toBe(false);
  expect(styles[3].fill_enabled).toBeUndefined();
  expect(styles[6].text).toBe("");
  expect(styles[7].text).toBe("two\nlines");
  expect(result.state.drawings[5].anchors[0].time).toBeUndefined();
  const semantic = (list) => list.map(({ kind, options }) => ({
    kind,
    text: options.text,
    fill_enabled: options.fill_enabled,
    fill_color: options.fill_color,
    stroke_end: options.stroke_end,
    labels: options.labels,
    tool_options: options.tool_options,
    box_color: options.box_color,
    width: options.width,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("Projection & Annotations tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
  const run_scenario = async (backend) => {
    await goto_fixture(page, backend);
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await page.evaluate(() => {
      const chart = window.__chart;
      const range = chart.time_scale().get_visible_logical_range();
      const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
      const b = (fraction) => window.__main.data_by_index(at(fraction));
      const lo = Math.min(b(0.2).low, b(0.6).low);
      const hi = Math.max(b(0.2).high, b(0.6).high);
      const up = (fraction) => lo + (hi - lo) * fraction;
      chart.add_drawing("forecast", [{ logical: at(0.1), price: up(0.2) }, { logical: at(0.22), price: up(0.6) }], { color: "#e91e63" });
      chart.add_drawing("bars_pattern", [{ logical: at(0.3), price: 0 }, { logical: at(0.4), price: 0 }], { color: "#7b1fa2" });
      chart.add_drawing("price_range", [{ logical: at(0.45), price: up(0.1) }, { logical: at(0.55), price: up(0.5) }]);
      chart.add_drawing("date_and_price_range", [{ logical: at(0.6), price: up(0.7) }, { logical: at(0.72), price: up(0.4) }], { color: "#ff6d00" });
      chart.add_drawing("projection", [{ logical: at(0.75), price: up(0.2) }, { logical: at(0.9), price: up(0.2) }, { logical: at(0.85), price: up(0.6) }], { color: "#089981" });
      chart.add_drawing("anchored_text", [{ logical: 0.05, price: 0.05 }], { text: "Plan A", box_color: "#fff3e0" });
      chart.add_drawing("callout", [{ logical: at(0.2), price: up(0.9) }, { logical: at(0.3), price: up(1.05) }]);
      chart.add_drawing("comment", [{ logical: at(0.5), price: up(0.95) }], { color: "#7b1fa2" });
      chart.add_drawing("note", [{ logical: at(0.62), price: up(0.95) }]);
      chart.add_drawing("signpost", [{ logical: at(0.8), price: up(0.9) }], { color: "#ff6d00" });
      chart.add_drawing("arrow_mark_up", [{ logical: at(0.35), price: lo }], { text: "buy" });
      chart.add_drawing("arrow_mark_down", [{ logical: at(0.4), price: hi }]);
      chart.add_drawing("flag_mark", [{ logical: at(0.58), price: hi }], { color: "#e91e63" });
      chart.add_drawing("icon", [{ logical: at(0.68), price: up(0.15) }], { color: "#ffb300" });
      chart.add_drawing("icon", [{ logical: at(0.7), price: up(0.3) }], { tool_options: { projection_annotation: { icon: "heart", icon_size: 30 } } });
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
  expect(differs(clean, canvas.png), "the family paints on Canvas2D").toBeGreaterThan(1000);
  expect(differs(clean, gpu.png), "the family paints on WebGPU").toBeGreaterThan(1000);

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
  console.log(`projection-annotations parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
  const canvas_path = test_info.outputPath("canvas2d.png");
  writeFileSync(canvas_path, PNG.sync.write(canvas.png));
  await test_info.attach("canvas2d.png", { path: canvas_path, contentType: "image/png" });
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "family geometry and paint order match across executors").toBe(0);
});
