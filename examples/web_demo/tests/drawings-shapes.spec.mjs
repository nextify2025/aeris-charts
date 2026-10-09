import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Shapes family (rotated rectangle, ellipse, circle, triangle, arc, curve, double curve,
// polyline, highlighter) through the public API and real pointer input: armed placement for every
// placement class (clicks with a progress preview from the first click, the multi-click polyline
// finished with Enter, the freehand highlighter), outline hover and selected-only fill hits, body
// and handle drags with undo, persistence, clipboard and sync round trips, the demo toolbar and
// its live style edits, WebGPU == Canvas2D parity, and the translucent highlighter blending once
// per pixel on WebGPU. Every geometry decision is engine-owned; these specs only drive the package
// API and the pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const SHAPES = [
  "rotated_rectangle",
  "ellipse",
  "circle",
  "triangle",
  "arc",
  "curve",
  "double_curve",
  "polyline",
  "highlighter",
];

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
    return { l0, l1, p_lo: low, p_hi: high, p_mid: (low + high) / 2 };
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

async function focus_overlay(page) {
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
}

/** Anchor clicks (logical, price) for each click-placed shape. */
function clicks_for(kind, s) {
  const mid = Math.floor((s.l0 + s.l1) / 2);
  const third = Math.floor(s.l0 + (s.l1 - s.l0) / 3);
  const two_thirds = Math.floor(s.l0 + (2 * (s.l1 - s.l0)) / 3);
  switch (kind) {
    case "rotated_rectangle": return [[s.l0, s.p_mid], [s.l1, s.p_mid], [mid, s.p_hi]];
    case "ellipse": return [[s.l0, s.p_lo], [s.l1, s.p_hi]];
    case "circle": return [[mid, s.p_mid], [s.l1, s.p_mid]];
    case "double_curve": return [[s.l0, s.p_mid], [s.l1, s.p_mid], [third, s.p_hi], [two_thirds, s.p_lo]];
    case "polyline": return [[s.l0, s.p_lo], [mid, s.p_hi], [s.l1, s.p_mid]];
    default: return [[s.l0, s.p_lo], [s.l1, s.p_lo], [mid, s.p_hi]];
  }
}

/** Drag a freehand stroke from `from` to `to` bending through `bend`. */
async function drag_stroke(page, from, bend, to, steps = 24) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  for (let i = 1; i <= steps; i += 1) {
    const t = i / steps;
    await page.mouse.move(
      (1 - t) * (1 - t) * from.x + 2 * (1 - t) * t * bend.x + t * t * to.x,
      (1 - t) * (1 - t) * from.y + 2 * (1 - t) * t * bend.y + t * t * to.y,
    );
  }
  await page.mouse.up();
}

test("every Shapes tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  for (const [index, kind] of SHAPES.entries()) {
    const before = await capture(page);
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    let expected_points = null;
    if (kind === "highlighter") {
      await drag_stroke(
        page,
        await spot(page, s.l0, s.p_hi),
        await spot(page, Math.floor((s.l0 + s.l1) / 2), s.p_mid),
        await spot(page, s.l1, s.p_hi),
      );
    } else {
      const clicks = clicks_for(kind, s);
      for (const [click, [logical, price]] of clicks.entries()) {
        const point = await spot(page, logical, price);
        await page.mouse.click(point.x, point.y);
        if (click === 0 && clicks.length >= 3) {
          // Before its last click a three- or four-anchor tool already shows its progress: the
          // placed anchor joined to the pointer (below every earlier shape, so its pixels are new).
          const placed = await capture(page);
          await page.mouse.move(point.x + 60, point.y + 50, { steps: 2 });
          await settle_frames(page);
          const moved = await capture(page);
          const preview = pixelmatch(placed.data, moved.data, null, placed.width, placed.height, { threshold: 0 });
          expect(preview, `${kind} previews from the first click`).toBeGreaterThan(20);
        }
      }
      if (kind === "polyline") {
        expect(await drawings(page), "a polyline waits for its finish").toHaveLength(index);
        await focus_overlay(page);
        await page.keyboard.press("Enter");
      }
      expected_points = clicks.length;
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(index + 1);
    expect(list[index].kind).toBe(kind);
    if (expected_points === null) expect(list[index].points.length).toBeGreaterThan(5);
    else expect(list[index].points).toHaveLength(expected_points);
    expect(list[index].options.color).toBe("#e91e63");
    for (const point of list[index].points) expect(Number.isFinite(point.time)).toBe(true);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const pixels = await capture(page);
    const diff = pixelmatch(before.data, pixels.data, null, before.width, before.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
  }
});

test("a circle hovers on its rim, hits its fill once selected, and drags as one undo step", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const mid = Math.floor((s.l0 + s.l1) / 2);
  const id = await page.evaluate(({ s, mid }) => window.__chart.add_drawing("circle", [
    { logical: mid, price: s.p_mid },
    { logical: s.l1, price: s.p_mid },
  ], { color: "#e91e63" }).id, { s, mid });
  await settle_frames(page);
  const center = await spot(page, mid, s.p_mid);
  const rim = await spot(page, s.l1, s.p_mid);
  const radius = Math.hypot(rim.x - center.x, rim.y - center.y);
  // The rim below the center is on the outline; the point halfway there is inside.
  const bottom = { x: center.x, y: center.y + radius };
  const inside = { x: center.x, y: center.y + radius / 2 };
  await page.mouse.move(bottom.x, bottom.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.move(inside.x, inside.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");

  await page.mouse.click(bottom.x, bottom.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
  await page.mouse.move(inside.x, inside.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  const before = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  await page.mouse.down();
  await page.mouse.move(inside.x + 20, inside.y - 10, { steps: 4 });
  await page.mouse.move(inside.x + 40, inside.y - 20, { steps: 4 });
  await page.mouse.up();
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  expect(moved[0].logical).toBeGreaterThan(before[0].logical);
  expect(moved[0].price).toBeGreaterThan(before[0].price);
  expect(moved[1].logical - moved[0].logical).toBeCloseTo(before[1].logical - before[0].logical, 6);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  const restored = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  expect(restored[0].logical).toBeCloseTo(before[0].logical, 9);
  expect(restored[0].price).toBeCloseTo(before[0].price, 9);
});

test("a rotated rectangle's depth handle resizes it and an edge handle turns it, one undo step each", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const depth_price = s.p_mid + (s.p_hi - s.p_mid) / 2;
  // Upstream's rotated rectangle: an edge (the first two anchors) and a point on the opposite edge.
  const id = await page.evaluate(({ s, depth_price }) => window.__chart.add_drawing("rotated_rectangle", [
    { logical: s.l0, price: s.p_mid },
    { logical: s.l1, price: s.p_mid },
    { logical: s.l0, price: depth_price },
  ], { color: "#e91e63" }).id, { s, depth_price });
  await settle_frames(page);
  const points = (id) => page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  const a = await spot(page, s.l0, s.p_mid);
  const b = await spot(page, s.l1, s.p_mid);
  const depth = await spot(page, s.l0, depth_price);
  // Select through the edge, then grab the depth handle.
  await page.mouse.click((a.x + b.x) / 2, a.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(id);
  expect(await page.evaluate((id) => window.__chart.drawing_handle_count(id), id)).toBe(3);
  const before = await points(id);
  await page.mouse.move(depth.x, depth.y);
  await expect.poll(() => overlay_cursor(page)).toBe("pointer");
  await page.mouse.down();
  await page.mouse.move(depth.x, depth.y - 20, { steps: 4 });
  await page.mouse.move(depth.x, depth.y - 30, { steps: 4 });
  await page.mouse.up();
  const after = await points(id);
  expect(after[2].price).toBeGreaterThan(before[2].price);
  expect(after[0]).toEqual(before[0]);
  expect(after[1]).toEqual(before[1]);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points(id)).toEqual(before);

  // Dragging the edge's end turns the rectangle; one undo restores it.
  await page.mouse.move(b.x, b.y);
  await page.mouse.down();
  await page.mouse.move(b.x - 20, b.y + 30, { steps: 4 });
  await page.mouse.move(b.x - 40, b.y + 60, { steps: 4 });
  await page.mouse.up();
  const turned = await points(id);
  expect(turned[1].price).toBeLessThan(before[1].price);
  expect(turned[0]).toEqual(before[0]);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  expect(await points(id)).toEqual(before);
});

test("a polyline places by clicks and finishes with Enter", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const mid = Math.floor((s.l0 + s.l1) / 2);
  const vertices = [
    await spot(page, s.l0, s.p_lo),
    await spot(page, mid, s.p_hi),
    await spot(page, s.l1, s.p_lo),
  ];
  await page.evaluate(() => window.__chart.set_drawing_tool("polyline", { color: "#e91e63" }));
  for (const vertex of vertices) await page.mouse.click(vertex.x, vertex.y);
  expect(await drawings(page)).toHaveLength(0);
  await focus_overlay(page);
  await page.keyboard.press("Enter");
  const open = await drawings(page);
  expect(open).toHaveLength(1);
  expect(open[0].points).toHaveLength(3);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Shapes tools round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
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
      ["rotated_rectangle", [{ logical: 1, price: 10 }, { logical: 5, price: 12 }, { logical: 3, price: 13 }], { text: "zone" }],
      ["ellipse", [{ logical: 1, price: 9 }, { logical: 4, price: 11 }], { fill_enabled: false }],
      ["circle", [{ logical: 2, price: 10 }, { logical: 4, price: 10 }], { fill_color: "rgba(1, 2, 3, 0.5)" }],
      ["triangle", [{ logical: 1, price: 9 }, { logical: 5, price: 9 }, { logical: 3, price: 12 }], { style: "dashed" }],
      ["arc", [{ logical: 1, price: 10 }, { logical: 5, price: 10 }, { logical: 3, price: 12 }], { stroke_end: "arrow" }],
      ["curve", [{ logical: 1, price: 10 }, { logical: 5, price: 10 }, { logical: 3, price: 12 }], { extend_right: true, fill_enabled: true }],
      ["double_curve", [{ logical: 1, price: 10 }, { logical: 7, price: 10 }, { logical: 3, price: 12 }, { logical: 5, price: 8 }], { stroke_start: "circle" }],
      ["polyline", [{ logical: 1, price: 10 }, { logical: 3, price: 12 }, { logical: 5, price: 9 }, { logical: 6, price: 11 }], { tool_options: { shape: { closed: true } } }],
      ["highlighter", [{ logical: 1, price: 10 }, { logical: 2, price: 10.5 }, { logical: 3, price: 10.2 }], { width: 14 }],
    ];
    for (const [kind, anchors, style] of additions) first.add_drawing(kind, anchors, style);
    const state = first.export_state();
    const copied = first.copy_drawings(first.drawings().map((drawing) => drawing.id));
    const sync = first.drawing_sync_payload("cell-a");
    const expected = first.drawings().map((drawing) => ({ kind: drawing.kind(), points: drawing.points().length, options: drawing.options() }));
    first.remove();
    first_host.remove();

    const second_host = host();
    const second = await create_chart(second_host, { backend: "canvas2d", autoSize: false });
    second.import_state(state);
    const canonical = second.export_state();
    const restored = second.drawings().map((drawing) => ({ kind: drawing.kind(), points: drawing.points().length, options: drawing.options() }));
    second.remove();
    second_host.remove();

    const third_host = host();
    const third = await create_chart(third_host, { backend: "canvas2d", autoSize: false });
    const pasted = third.paste_drawings(copied).map((drawing) => ({ kind: drawing.kind(), points: drawing.points().length, options: drawing.options() }));
    third.remove();
    third_host.remove();

    const fourth_host = host();
    const fourth = await create_chart(fourth_host, { backend: "canvas2d", autoSize: false });
    const applied = fourth.apply_drawing_sync_payload(sync);
    const synced = fourth.drawings().map((drawing) => ({ kind: drawing.kind(), points: drawing.points().length, options: drawing.options() }));
    fourth.remove();
    fourth_host.remove();
    return { state, canonical, expected, restored, pasted, applied, synced };
  });

  expect(result.canonical).toEqual(result.state);
  const styles = result.state.drawings.map((drawing) => drawing.style);
  expect(styles[0].fill_enabled, "default fills are not written").toBeUndefined();
  expect(styles[1].fill_enabled).toBe(false);
  expect(styles[5].fill_enabled).toBe(true);
  expect(styles[7].tool_options).toEqual({ shape: { closed: true } });
  expect(styles[8].tool_options).toBeUndefined();
  const semantic = (list) => list.map(({ kind, points, options }) => ({
    kind,
    points,
    fill_enabled: options.fill_enabled,
    fill_color: options.fill_color,
    extend_right: options.extend_right,
    stroke_start: options.stroke_start,
    stroke_end: options.stroke_end,
    tool_options: options.tool_options,
    style: options.style,
    width: options.width,
    color: options.color,
    text: options.text,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
  expect(result.expected[8].options.width).toBe(14);
});

test("the demo toolbar arms every Shapes tool and keeps the highlighter's marker defaults", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of SHAPES) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  // The highlighter button is armed: draw with it and keep the engine's wide marker (the
  // translucency is applied when it paints).
  const s = await anchor_spots(page);
  await drag_stroke(
    page,
    await spot(page, s.l0, s.p_mid),
    await spot(page, Math.floor((s.l0 + s.l1) / 2), s.p_hi),
    await spot(page, s.l1, s.p_mid),
  );
  const list = await drawings(page);
  const created = list[list.length - 1];
  expect(created.kind).toBe("highlighter");
  expect(created.options.width).toBe(12);
  // Editing the selected marker's width from the form keeps its color.
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.kind())).toBe("highlighter");
  await expect.poll(() => page.evaluate(() => document.getElementById("drawing_width").value)).toBe("12");
  await page.evaluate(() => {
    const input = document.getElementById("drawing_width");
    input.value = "16";
    input.dispatchEvent(new Event("change"));
  });
  const edited = await page.evaluate(() => window.__chart.selected_drawing().options());
  expect(edited.width).toBe(16);
  expect(edited.color).toBe(created.options.color);
  await page.click("#drawings_group [data-tool='polyline']");
  await page.click("#drawings_group [data-tool='polyline']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Shapes tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
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
      chart.add_drawing("rotated_rectangle", [{ logical: at(0.05), price: up(0.2) }, { logical: at(0.2), price: up(0.4) }, { logical: at(0.1), price: up(0.45) }], { color: "#e91e63" });
      chart.add_drawing("ellipse", [{ logical: at(0.22), price: up(0.1) }, { logical: at(0.36), price: up(0.35) }], { color: "#089981" });
      chart.add_drawing("circle", [{ logical: at(0.46), price: up(0.25) }, { logical: at(0.52), price: up(0.25) }], { color: "#7b1fa2", width: 3 });
      chart.add_drawing("triangle", [{ logical: at(0.6), price: up(0.1) }, { logical: at(0.75), price: up(0.1) }, { logical: at(0.68), price: up(0.4) }], { color: "#ff6d00", style: "dashed" });
      chart.add_drawing("arc", [{ logical: at(0.05), price: up(0.6) }, { logical: at(0.2), price: up(0.6) }, { logical: at(0.12), price: up(0.8) }], { color: "#2962ff" });
      chart.add_drawing("curve", [{ logical: at(0.25), price: up(0.6) }, { logical: at(0.4), price: up(0.6) }, { logical: at(0.32), price: up(0.85) }], { color: "#00bcd4", fill_enabled: true, stroke_end: "arrow" });
      chart.add_drawing("double_curve", [{ logical: at(0.45), price: up(0.7) }, { logical: at(0.62), price: up(0.7) }, { logical: at(0.5), price: up(0.9) }, { logical: at(0.56), price: up(0.55) }], { color: "#f44336", fill_enabled: true });
      chart.add_drawing("polyline", [{ logical: at(0.66), price: up(0.55) }, { logical: at(0.72), price: up(0.9) }, { logical: at(0.78), price: up(0.6) }, { logical: at(0.84), price: up(0.85) }], { color: "#4caf50", tool_options: { shape: { closed: true } } });
      chart.add_drawing("highlighter", Array.from({ length: 12 }, (_, index) => ({ logical: at(0.8 + index * 0.012), price: up(0.2 + 0.1 * Math.sin(index / 3)) })));
      chart.wasm.set_selected_drawing(chart.drawings()[2].id);
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
  expect(differs(clean, canvas.png), "Shapes paint on Canvas2D").toBeGreaterThan(1000);
  expect(differs(clean, gpu.png), "Shapes paint on WebGPU").toBeGreaterThan(1000);

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
  console.log(`shapes parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["canvas2d.png", canvas.png], ["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "Shapes geometry and paint order match across executors").toBe(0);
});

test("a translucent highlighter blends once per pixel on WebGPU as on Canvas2D", async ({ page }) => {
  // A jittery freehand path with 1.5 px samples under a 20 px stroke: every sample turns inside
  // the stroke width, where overlapping stroke triangles would paint the amber twice on the GPU.
  const run_scenario = async (backend, with_marker) => {
    await goto_fixture(page, backend);
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    if (with_marker) {
      await page.evaluate(() => {
        const time_scale = window.__chart.time_scale();
        const points = Array.from({ length: 300 }, (_, index) => {
          const x = 300 + index * 1.5;
          const y = 300 + 40 * Math.sin(index / 12) + ((index * 7919) % 5) - 2;
          return { logical: time_scale.coordinate_to_logical(x), price: window.__main.coordinate_to_price(y) };
        });
        window.__chart.add_drawing("highlighter", points, { width: 20 });
        window.__chart.render();
      });
    }
    await settle_frames(page);
    return {
      backend: await page.evaluate(() => window.__chart.backend()),
      png: PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false })),
    };
  };
  const clean = await run_scenario("canvas2d", false);
  const canvas = await run_scenario("canvas2d", true);
  const gpu = await run_scenario("auto", true);
  expect(gpu.backend).toBe("webgpu");
  let marker = 0;
  let strong = 0;
  for (let offset = 0; offset < canvas.png.data.length; offset += 4) {
    let painted = 0;
    let delta = 0;
    for (let channel = 0; channel < 3; channel += 1) {
      painted = Math.max(painted, Math.abs(canvas.png.data[offset + channel] - clean.png.data[offset + channel]));
      delta = Math.max(delta, Math.abs(canvas.png.data[offset + channel] - gpu.png.data[offset + channel]));
    }
    if (painted > 0) marker += 1;
    if (delta > 40) strong += 1;
  }
  console.log(`highlighter overlap: ${strong} of ${marker} marker pixels differ by more than 40`);
  expect(marker).toBeGreaterThan(20_000);
  // Only the anti-aliased rim may differ; double-blended overlaps covered about half the marker.
  expect(strong / marker).toBeLessThan(0.1);
});
