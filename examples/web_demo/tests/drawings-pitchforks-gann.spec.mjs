import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Pitchforks & Gann family (Andrews, Schiff, modified Schiff, and inside pitchforks, pitchfan,
// Gann box, Gann square, Gann square fixed, Gann fan) through the public API and real pointer
// input: armed placement, hover and selection along geometry beyond the anchors, anchor and body
// drags with undo, flat level options and atomic rejection, persistence,
// clipboard, and sync round trips, the demo toolbar entries, and WebGPU == Canvas2D parity. Every
// geometry decision is engine-owned; these specs only drive the package API and pointer.

const TOOLS = [
  "andrews_pitchfork",
  "schiff_pitchfork",
  "modified_schiff_pitchfork",
  "inside_pitchfork",
  "pitchfan",
  "gann_box",
  "gann_square",
  "gann_square_fixed",
  "gann_fan",
];
const ANCHOR_COUNT = {
  andrews_pitchfork: 3,
  schiff_pitchfork: 3,
  modified_schiff_pitchfork: 3,
  inside_pitchfork: 3,
  pitchfan: 3,
  gann_box: 2,
  gann_square: 2,
  gann_square_fixed: 2,
  gann_fan: 2,
};
const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;

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

/** Visible logical indexes and prices for deterministic anchors: a low, a high, and a pullback. */
async function anchor_spots(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
    const l0 = at(0.2);
    const lm = at(0.35);
    const l1 = at(0.45);
    const bars = [l0, lm, l1].map((logical) => window.__main.data_by_index(logical));
    const low = Math.min(...bars.map((bar) => bar.low));
    const high = Math.max(...bars.map((bar) => bar.high));
    return { l0, lm, l1, p_lo: low, p_hi: high, p_mid: (low + high) / 2 };
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

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

async function selected(page) {
  return page.evaluate(() => window.__chart.selected_drawing()?.id ?? null);
}

async function points_of(page, id) {
  return page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
}

function pixel_diff(a, b) {
  return pixelmatch(a.data, b.data, null, a.width, a.height, { threshold: 0 });
}

test("every Pitchforks & Gann tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);
  const spots = [[s.l0, s.p_lo], [s.lm, s.p_hi], [s.l1, s.p_mid]];
  for (const kind of TOOLS) {
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    const clicks = spots.slice(0, ANCHOR_COUNT[kind]);
    for (const [index, [logical, price]] of clicks.entries()) {
      const point = await spot(page, logical, price);
      if (index === 1 && clicks.length === 3) {
        // Between clicks the placed anchor and the pointer join in a guide.
        const before = await capture(page);
        await page.mouse.move(point.x, point.y, { steps: 3 });
        await settle_frames(page);
        expect(pixel_diff(before, await capture(page)), `${kind} placement guide`).toBeGreaterThan(20);
      }
      await page.mouse.click(point.x, point.y);
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(1);
    expect(list[0].kind).toBe(kind);
    expect(list[0].points).toHaveLength(clicks.length);
    expect(list[0].options.color).toBe("#e91e63");
    for (const point of list[0].points) expect(Number.isFinite(point.time)).toBe(true);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    expect(pixel_diff(clean, await capture(page)), `${kind} paints`).toBeGreaterThan(200);
    await page.evaluate(() => window.__chart.clear_drawings());
    await settle_frames(page);
  }
});

test("a pitchfork hovers and selects along its tines, drags one anchor, and undoes", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("andrews_pitchfork", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.lm, price: s.p_hi },
    { logical: s.l1, price: s.p_mid },
  ], { color: "#e91e63" }).id, { s });
  await settle_frames(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.lm, s.p_hi);
  const c = await spot(page, s.l1, s.p_mid);
  const center = { x: (b.x + c.x) / 2, y: (b.y + c.y) / 2 };
  const direction = { x: center.x - a.x, y: center.y - a.y };
  // Along the median past the handle, far from every anchor.
  const along = { x: a.x + direction.x * 1.7, y: a.y + direction.y * 1.7 };
  await page.mouse.move(along.x, along.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.click(along.x, along.y);
  expect(await selected(page)).toBe(id);
  // The upper tine through B, past the handle.
  const tine = { x: b.x + direction.x * 0.6, y: b.y + direction.y * 0.6 };
  await page.mouse.move(tine.x, tine.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");

  // Drag the third anchor with the real pointer: only it moves, as one undo step.
  const before = await points_of(page, id);
  await page.mouse.move(c.x, c.y);
  await page.mouse.down();
  await page.mouse.move(c.x + 20, c.y + 25, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);
  const after = await points_of(page, id);
  expect(after[0]).toEqual(before[0]);
  expect(after[1]).toEqual(before[1]);
  expect(after[2].logical).toBeGreaterThan(before[2].logical);
  expect(after[2].price).toBeLessThan(before[2].price);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  const restored = await points_of(page, id);
  expect(restored[2].logical).toBeCloseTo(before[2].logical, 9);
  expect(restored[2].price).toBeCloseTo(before[2].price, 9);

  // Level lists are typed kind options; showing another level repaints.
  const shown = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    const initial = window.__chart.drawing_kind_options(drawing);
    const levels = drawing.options().levels.map((level) => ({ ...level, visible: level.value !== 0.5 }));
    drawing.apply_options({ levels: [...levels, { ...levels[0], value: 0.75 }] });
    return { initial, next: window.__chart.drawing_kind_options(drawing) };
  }, id);
  expect(shown.initial.kind).toBe("levels");
  expect(shown.initial.levels.filter((level) => level.visible).map((level) => level.value)).toEqual([0, 0.5, 1]);
  expect(shown.next.levels.filter((level) => level.visible).map((level) => level.value)).toEqual([0, 1, 0.75]);
});

test("extreme levels, sizes, and scale ratios render bounded frames on both executors", async ({ page }) => {
  for (const backend of ["canvas2d", "auto"]) {
    await goto_fixture(page, backend);
    const s = await anchor_spots(page);
    const level = (value, style) => ({ value, color: "#e91e63", visible: true, style, fill_between: true, label_visible: true });
    const clean = await capture(page);
    const elapsed = await page.evaluate(({ s, levels }) => {
      const chart = window.__chart;
      // A dotted pitchfan ray a million half handles out, and a fixed square grown down by 20 bars
      // at a billion price units per bar with dashed grid lines and a dotted full-size arc.
      chart.add_drawing("pitchfan", [
        { logical: s.l0, price: s.p_lo }, { logical: s.lm, price: s.p_hi }, { logical: s.l1, price: s.p_mid },
      ], { style: "dotted", levels: [levels.ray] });
      chart.add_drawing("gann_square_fixed", [
        { logical: s.l0, price: s.p_mid }, { logical: s.l0 + 20, price: s.p_mid - 2e10 },
      ], { levels: levels.grid, level_reverse: true, gann_arcs: [levels.arc] });
      const start = performance.now();
      chart.render();
      return performance.now() - start;
    }, { s, levels: { ray: level(1e6, "dotted"), grid: [level(0, "dashed"), level(1, "dashed")], arc: level(1, "dotted") } });
    await settle_frames(page);
    expect(elapsed, `${backend} frame time`).toBeLessThan(1000);
    expect(pixel_diff(clean, await capture(page)), `${backend} paints`).toBeGreaterThan(50);
  }
});

test("a Gann box selects and drags from its level lines and rejects invalid options atomically", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("gann_box", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.l1, price: s.p_hi },
  ]).id, { s });
  await settle_frames(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.l1, s.p_hi);
  // Between two grid lines the box lets the pointer through.
  const between = { x: a.x + (b.x - a.x) * 0.19, y: a.y + (b.y - a.y) * 0.19 };
  await page.mouse.move(between.x, between.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  // The 0.5 price level is a body target.
  const level = { x: a.x + (b.x - a.x) * 0.3, y: a.y + (b.y - a.y) * 0.5 };
  await page.mouse.move(level.x, level.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.click(level.x, level.y);
  expect(await selected(page)).toBe(id);
  const before = await points_of(page, id);
  await page.mouse.down();
  await page.mouse.move(level.x + 30, level.y - 20, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await points_of(page, id);
  expect(moved[0].logical).toBeGreaterThan(before[0].logical);
  expect(moved[1].logical - moved[0].logical).toBeCloseTo(before[1].logical - before[0].logical, 6);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);

  const plain = await capture(page);
  const result = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    // Earlier builds' `tool_options.gann.reverse` moves onto `level_reverse`.
    drawing.apply_options({ tool_options: { gann: { reverse: true } } });
    const kind_options = window.__chart.drawing_kind_options(drawing);
    let code = null;
    try {
      drawing.apply_options({ width: 5, level_label_align: "sideways" });
    } catch (error) {
      code = error.code;
    }
    return { kind_options, code, width: drawing.options().width, level_reverse: drawing.options().level_reverse };
  }, id);
  expect(result.kind_options.kind).toBe("levels");
  expect(result.kind_options.reverse).toBe(true);
  expect(result.kind_options.levels.map((level) => level.value)).toEqual([0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1]);
  expect(result.code).toBe("invalid_options");
  expect(result.width).toBe(1);
  expect(result.level_reverse).toBe(true);
  await settle_frames(page);
  expect(pixel_diff(plain, await capture(page)), "reversed levels repaint").toBeGreaterThan(0);
});

test("a fixed Gann square sizes from its two anchors and a Gann fan projects its ratio rays", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const ids = await page.evaluate(({ s }) => {
    const square = window.__chart.add_drawing("gann_square_fixed", [
      { logical: s.l0, price: s.p_lo }, { logical: s.l0 + 10, price: s.p_hi },
    ]);
    return { square: square.id, schema: window.__chart.drawing_property_schema(square).properties.map((property) => property.name) };
  }, { s });
  expect(ids.schema).toContain("gann_fans");
  expect(ids.schema).toContain("gann_arcs");
  // The anchors carry the size; the stats box and the scale ratio are the square's options.
  expect(ids.schema).not.toContain("tool_options.gann.size_bars");
  expect(ids.schema).toContain("tool_options.gann.show_stats");
  expect(ids.schema).toContain("tool_options.gann.scale_ratio");
  await settle_frames(page);
  const before = await capture(page);
  // Moving the second anchor resizes the square.
  await page.evaluate(({ id, s }) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).set_points([
      { logical: s.l0, price: s.p_lo }, { logical: s.l0 + 5, price: s.p_mid },
    ]);
  }, { id: ids.square, s });
  await settle_frames(page);
  expect(pixel_diff(before, await capture(page)), "the square resizes").toBeGreaterThan(50);
  await page.evaluate(() => window.__chart.clear_drawings());

  const fan = await page.evaluate(({ s }) => {
    const drawing = window.__chart.add_drawing("gann_fan", [
      { logical: s.l0, price: s.p_lo },
      { logical: s.l1, price: s.p_hi },
    ]);
    return { id: drawing.id, kind_options: window.__chart.drawing_kind_options(drawing) };
  }, { s });
  expect(fan.kind_options.kind).toBe("levels");
  expect(fan.kind_options.levels.map((level) => level.value)).toEqual([0.125, 0.25, 0.333333333333, 0.5, 1, 2, 3, 4, 8]);
});

test("Pitchforks & Gann tools round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
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
    const three = [{ logical: 1, price: 10 }, { logical: 3, price: 12 }, { logical: 5, price: 11 }];
    const two = [{ logical: 1, price: 10 }, { logical: 4, price: 12 }];
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const additions = [
      ["andrews_pitchfork", three, { extend_right: true }],
      ["schiff_pitchfork", three, { fill_enabled: false }],
      ["modified_schiff_pitchfork", three, {}],
      ["inside_pitchfork", three, { style: "dashed" }],
      ["pitchfan", three, { levels: [] }],
      ["gann_box", two, { level_reverse: true, tool_options: { gann: { show_angles: true } } }],
      ["gann_square", two, { tool_options: { gann: { show_stats: false } } }],
      ["gann_square_fixed", [{ logical: 2, price: 11 }, { logical: 14, price: 14 }], { tool_options: { gann: { size_bars: 12, scale_ratio: 0.25 } } }],
      ["gann_fan", two, { extend_right: true }],
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
  expect(styles[0].levels).toBeUndefined();
  expect(styles[1].fill_enabled).toBe(false);
  expect(styles[2].tool_options).toBeUndefined();
  expect(styles[4].levels).toEqual([]);
  expect(styles[5].level_reverse).toBe(true);
  // Gann options without a flat counterpart stay stored (the box paints its angles).
  expect(styles[5].tool_options.gann.show_angles).toBe(true);
  expect(styles[7].tool_options.gann.size_bars).toBe(12);
  expect(styles[8].extend_right).toBe(true);
  const semantic = (list) => list.map(({ kind, options }) => ({
    kind,
    color: options.color,
    extend_left: options.extend_left,
    extend_right: options.extend_right,
    fill_enabled: options.fill_enabled,
    levels: options.levels,
    level_reverse: options.level_reverse,
    tool_options: options.tool_options,
    style: options.style,
    width: options.width,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("a pitchfork drags its base midpoint and a fixed Gann square resizes from its painted corner", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const fork = await page.evaluate(({ s }) => window.__chart.add_drawing("andrews_pitchfork", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.lm, price: s.p_hi },
    { logical: s.l1, price: s.p_mid },
  ]).id, { s });
  await settle_frames(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.lm, s.p_hi);
  const c = await spot(page, s.l1, s.p_mid);
  const base = { x: (b.x + c.x) / 2, y: (b.y + c.y) / 2 };
  // Select through the tine past B; the base midpoint is then the fourth handle.
  const direction = { x: base.x - a.x, y: base.y - a.y };
  await page.mouse.click(b.x + direction.x * 0.6, b.y + direction.y * 0.6);
  expect(await selected(page)).toBe(fork);
  expect(await page.evaluate((id) => window.__chart.drawing_handle_count(id), fork)).toBe(4);
  await page.mouse.move(base.x, base.y);
  await expect.poll(() => overlay_cursor(page)).toBe("pointer");
  const before = await points_of(page, fork);
  await page.mouse.down();
  await page.mouse.move(base.x + 24, base.y - 18, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);
  const after = await points_of(page, fork);
  expect(after[0]).toEqual(before[0]);
  // B and C move together by whole bars, the bars the pointer's move crossed (within one bar of
  // its 24 px), and by its vertical move in price.
  const spacing = (c.x - b.x) / (s.l1 - s.lm);
  const shift = after[1].logical - before[1].logical;
  expect(Number.isInteger(shift)).toBe(true);
  expect(Math.abs(shift * spacing - 24)).toBeLessThanOrEqual(spacing);
  const moved_y = await page.evaluate((price) => window.__main.price_to_coordinate(price), after[1].price);
  expect(moved_y - b.y).toBeCloseTo(-18, 0);
  expect(after[2].logical - before[2].logical).toBeCloseTo(after[1].logical - before[1].logical, 6);
  expect(after[2].price - before[2].price).toBeCloseTo(after[1].price - before[1].price, 6);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points_of(page, fork)).toEqual(before);
  await page.evaluate(() => window.__chart.clear_drawings());

  // A tall fixed square: its time side wins, so its handle sits on the painted corner, not on
  // the second anchor.
  const square = await page.evaluate(({ s }) => window.__chart.add_drawing("gann_square_fixed", [
    { logical: s.l0, price: s.p_lo }, { logical: s.l0 + 60, price: s.p_hi + (s.p_hi - s.p_lo) * 10 },
  ]).id, { s });
  await settle_frames(page);
  const pivot = await spot(page, s.l0, s.p_lo);
  const corner_at = async (bars) => {
    const x = await page.evaluate((logical) => window.__chart.time_scale().logical_to_coordinate(logical), s.l0 + bars);
    return { x, y: pivot.y - (x - pivot.x) };
  };
  const corner = await corner_at(60);
  await page.mouse.click(corner.x, (pivot.y + corner.y) / 2);
  expect(await selected(page)).toBe(square);
  await page.mouse.move(corner.x, corner.y);
  await expect.poll(() => overlay_cursor(page)).toBe("nesw-resize");
  // The corner's larger distance sets the side in whole bars.
  const target = await corner_at(90);
  await page.mouse.down();
  await page.mouse.move(target.x, target.y, { steps: 6 });
  await page.mouse.up();
  await settle_frames(page);
  const resized = await points_of(page, square);
  expect(resized[1].logical - resized[0].logical).toBe(90);
  expect(resized[1].price).toBeGreaterThan(s.p_hi);
  const moved_corner = await corner_at(90);
  await page.mouse.move(moved_corner.x, moved_corner.y);
  await expect.poll(() => overlay_cursor(page)).toBe("nesw-resize");
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect((await points_of(page, square))[1].logical - resized[0].logical).toBe(60);
});

test("Gann options paint: box time levels and angles, the square's stats box, and scale ratios", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("gann_box", [
    { logical: s.l0, price: s.p_lo }, { logical: s.l1, price: s.p_hi },
  ]).id, { s });
  await settle_frames(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.l1, s.p_hi);
  const plain = await capture(page);
  const level = (value) => ({ value, color: "#e91e63", visible: true, style: "solid", fill_between: true, label_visible: false });
  await page.evaluate(({ id, levels }) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({
      tool_options: { gann: { time_levels: levels, show_angles: true } },
    });
  }, { id, levels: [level(0), level(0.5), level(1)] });
  await settle_frames(page);
  expect(pixel_diff(plain, await capture(page)), "time levels and angles repaint").toBeGreaterThan(200);
  // A vertical line only at the time levels: 0.25 across lets the pointer through between the
  // price levels, 0.5 is a body target.
  const between = a.y + (b.y - a.y) * 0.5625;
  await page.mouse.move(a.x + (b.x - a.x) * 0.25, between);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  await page.mouse.move(a.x + (b.x - a.x) * 0.5, between);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.evaluate(() => window.__chart.clear_drawings());

  const square = await page.evaluate(({ s }) => window.__chart.add_drawing("gann_square", [
    { logical: s.l0, price: s.p_lo }, { logical: s.l1, price: s.p_hi },
  ]).id, { s });
  await settle_frames(page);
  const without = await capture(page);
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ tool_options: { gann: { show_stats: true } } });
  }, square);
  await settle_frames(page);
  expect(pixel_diff(without, await capture(page)), "the stats box paints").toBeGreaterThan(200);
  // The box sits right of the far corner and is a body target.
  const far = await spot(page, s.l1, s.p_hi);
  await page.mouse.move(far.x + 24, far.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.evaluate(() => window.__chart.clear_drawings());

  const fan = await page.evaluate(({ s }) => window.__chart.add_drawing("gann_fan", [
    { logical: s.l0, price: s.p_lo }, { logical: s.l1, price: s.p_hi },
  ]).id, { s });
  await settle_frames(page);
  const anchor_slope = await capture(page);
  await page.evaluate(({ id, ratio }) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ tool_options: { gann: { scale_ratio: ratio } } });
  }, { id: fan, ratio: (s.p_hi - s.p_lo) / (s.l1 - s.l0) / 2 });
  await settle_frames(page);
  expect(pixel_diff(anchor_slope, await capture(page)), "the ratio sets the 1x1 slope").toBeGreaterThan(200);
});

test("the demo toolbar arms every Pitchforks & Gann tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of TOOLS) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  await page.click("#drawings_group [data-tool='gann_fan']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Pitchforks & Gann tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
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
      chart.add_drawing("andrews_pitchfork", [
        { logical: at(0.05), price: up(0.1) }, { logical: at(0.15), price: up(0.6) }, { logical: at(0.2), price: up(0.3) },
      ], { width: 2 });
      chart.add_drawing("schiff_pitchfork", [
        { logical: at(0.25), price: up(0.9) }, { logical: at(0.32), price: up(0.4) }, { logical: at(0.36), price: up(0.7) },
      ], { color: "#7b1fa2", fill_enabled: false });
      chart.add_drawing("pitchfan", [
        { logical: at(0.4), price: up(0.2) }, { logical: at(0.5), price: up(0.55) }, { logical: at(0.55), price: up(0.35) },
      ]);
      chart.add_drawing("gann_box", [{ logical: at(0.6), price: up(0.1) }, { logical: at(0.72), price: up(0.45) }]);
      chart.add_drawing("gann_square", [{ logical: at(0.75), price: up(0.55) }, { logical: at(0.9), price: up(0.95) }], { width: 2 });
      chart.add_drawing("gann_fan", [{ logical: at(0.62), price: up(0.6) }, { logical: at(0.7), price: up(0.8) }]);
      chart.add_drawing("gann_square_fixed", [{ logical: at(0.8), price: up(0.1) }, { logical: at(0.84), price: up(0.3) }]);
      // The restored Gann options: a stats box, a split-axis box with angles, and a ratio fan.
      chart.add_drawing("gann_square", [{ logical: at(0.1), price: up(0.6) }, { logical: at(0.18), price: up(0.85) }], {
        tool_options: { gann: { show_stats: true } },
      });
      chart.add_drawing("gann_box", [{ logical: at(0.45), price: up(0.6) }, { logical: at(0.55), price: up(0.9) }], {
        tool_options: { gann: { show_angles: true, time_levels: [
          { value: 0, color: "#00897b", visible: true, style: "solid", fill_between: true, label_visible: true },
          { value: 0.5, color: "#00897b", visible: true, style: "dashed", fill_between: true, label_visible: true },
          { value: 1, color: "#00897b", visible: true, style: "solid", fill_between: true, label_visible: true },
        ] } },
      });
      chart.add_drawing("gann_fan", [{ logical: at(0.02), price: up(0.05) }, { logical: at(0.08), price: up(0.15) }], {
        tool_options: { gann: { scale_ratio: (hi - lo) / 40 } },
      });
      // Dashed and dotted strokes reach both executors as the same solid dash runs.
      chart.add_drawing("inside_pitchfork", [
        { logical: at(0.3), price: up(0.15) }, { logical: at(0.38), price: up(0.45) }, { logical: at(0.45), price: up(0.25) },
      ], { color: "#ff6d00", style: "dashed", width: 2 });
      chart.add_drawing("modified_schiff_pitchfork", [
        { logical: at(0.85), price: up(0.2) }, { logical: at(0.9), price: up(0.5) }, { logical: at(0.95), price: up(0.3) },
      ], { style: "dotted", fill_enabled: false });
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
  console.log(`pitchforks & gann parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels (pixel ratio ${PR})`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["canvas2d.png", canvas.png], ["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "family geometry and paint order match across executors").toBe(0);
});
