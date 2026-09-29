import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Lines family (ray, extended line, info line, trend angle, cross line, arrow line) through the
// public API and real pointer input: armed placement, edge-reaching extensions and their hit
// testing, the engine-formatted info stats box and its typed tool options, cross-line body drags,
// persistence and clipboard round trips, the demo toolbar entries, and WebGPU == Canvas2D parity.
// Every geometry decision is engine-owned; these specs only drive the package API and pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const LINES = ["ray", "extended_line", "info_line", "trend_angle", "cross_line", "arrow_line"];
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

/** Device-px x extent of `target`-colored pixels, or null when none. */
function color_x_extent(png, target, tol = 12) {
  let min = Infinity;
  let max = -Infinity;
  for (let y = 0; y < png.height; y += 1) {
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol &&
        Math.abs(png.data[o + 1] - target[1]) <= tol &&
        Math.abs(png.data[o + 2] - target[2]) <= tol
      ) {
        min = Math.min(min, x);
        max = Math.max(max, x);
      }
    }
  }
  return Number.isFinite(min) ? { min, max } : null;
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

test("every Lines tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);
  for (const [index, kind] of LINES.entries()) {
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    const clicks = kind === "cross_line"
      ? [[s.l1 + 3, s.p_mid]]
      : [[s.l0, s.p_lo], [s.l1, s.p_hi]];
    for (const [logical, price] of clicks) {
      const point = await spot(page, logical, price);
      await page.mouse.click(point.x, point.y);
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(index + 1);
    expect(list[index].kind).toBe(kind);
    expect(list[index].points).toHaveLength(clicks.length);
    expect(list[index].options.color).toBe("#e91e63");
    for (const point of list[index].points) expect(Number.isFinite(point.time)).toBe(true);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const pixels = await capture(page);
    const diff = pixelmatch(clean.data, pixels.data, null, clean.width, clean.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
  }
});

test("rays and extended lines reach the pane edges and hit along the extension", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const width = await page.evaluate(() => window.__chart.time_scale().width());
  const ids = await page.evaluate(({ s }) => {
    const ray = window.__chart.add_drawing("ray", [
      { logical: s.l0, price: s.p_mid },
      { logical: s.l1, price: s.p_mid },
    ], { color: "#e91e63" });
    return { ray: ray.id };
  }, { s });
  await settle_frames(page);
  let extent = color_x_extent(await capture(page), PINK);
  const start = await spot(page, s.l0, s.p_mid);
  expect(extent.min).toBeGreaterThanOrEqual(Math.floor(start.x * PR) - 2);
  expect(extent.max).toBeGreaterThanOrEqual(Math.floor((width - 2) * PR));

  // The extension is a hover and selection target far beyond the second anchor.
  const far_x = width - 12;
  const far = await page.evaluate(({ id, far_x }) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    const [a, b] = drawing.points();
    const time_scale = window.__chart.time_scale();
    const ax = time_scale.logical_to_coordinate(a.logical);
    const bx = time_scale.logical_to_coordinate(b.logical);
    const ay = window.__main.price_to_coordinate(a.price);
    const by = window.__main.price_to_coordinate(b.price);
    return { x: far_x, y: ay + (by - ay) * (far_x - ax) / (bx - ax) };
  }, { id: ids.ray, far_x });
  await page.mouse.move(far.x, far.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.click(far.x, far.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(ids.ray);

  // Extending beyond the first anchor too makes it an extended line spanning both edges.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ extend_left: true });
  }, ids.ray);
  await settle_frames(page);
  extent = color_x_extent(await capture(page), PINK);
  expect(extent.min).toBeLessThanOrEqual(2);
  expect(extent.max).toBeGreaterThanOrEqual(Math.floor((width - 2) * PR));

  // With both extensions off it is the anchor segment: paint stops at the anchors and the old
  // extension spot no longer hovers the drawing.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id)
      .apply_options({ extend_left: false, extend_right: false });
  }, ids.ray);
  await settle_frames(page);
  extent = color_x_extent(await capture(page), PINK);
  const end = await spot(page, s.l1, s.p_mid);
  expect(extent.min).toBeGreaterThanOrEqual(Math.floor(start.x * PR) - 4);
  expect(extent.max).toBeLessThanOrEqual(Math.ceil(end.x * PR) + 4);
  await page.mouse.move(far.x, far.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  const schema = await page.evaluate(() => {
    const list = window.__chart.drawings();
    return window.__chart.drawing_property_schema(list[list.length - 1]);
  });
  expect(schema.kind).toBe("ray");
  expect(schema.properties.find((property) => property.name === "extend_right").default).toBe(true);
});

test("the info line shows engine stats in a selectable box with typed tool options", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const info = await page.evaluate(({ s }) => {
    const drawing = window.__chart.add_drawing("info_line", [
      { logical: s.l0, price: s.p_lo },
      { logical: s.l1, price: s.p_hi },
    ], { color: "#e91e63" });
    return {
      id: drawing.id,
      labels: drawing.options().labels.map((label) => label.metric),
      kind_options: window.__chart.drawing_kind_options(drawing),
    };
  }, { s });
  expect(info.labels).toEqual(["price_change", "percent_change", "bar_count", "duration", "angle"]);
  expect(info.kind_options).toEqual({ kind: "line", stats_position: "end" });
  await settle_frames(page);

  // The stats box sits beyond the second anchor and selects the drawing.
  const end = await spot(page, s.l1, s.p_hi);
  await page.mouse.click(end.x + 30, end.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(info.id);

  // Moving the box to the start: the old spot no longer hits, the new one does.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id)
      .apply_options({ tool_options: { line: { stats_position: "start" } } });
  }, info.id);
  await settle_frames(page);
  const start = await spot(page, s.l0, s.p_lo);
  // Outside the double-click interval, so the second press is an ordinary selection click.
  await page.waitForTimeout(700);
  await page.mouse.click(end.x + 30, end.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBeNull();
  await page.mouse.click(start.x - 30, start.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(info.id);
  const options = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    return { tool_options: drawing.options().tool_options, kind_options: window.__chart.drawing_kind_options(drawing) };
  }, info.id);
  expect(options.tool_options).toEqual({ line: { stats_position: "start" } });
  expect(options.kind_options).toEqual({ kind: "line", stats_position: "start" });

  // An invalid block is rejected atomically.
  const rejected = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    try {
      drawing.apply_options({ width: 7, tool_options: { line: { stats_position: "sideways" } } });
    } catch (error) {
      return { code: error.code, width: drawing.options().width };
    }
    return { code: null, width: drawing.options().width };
  }, info.id);
  expect(rejected.code).toBe("invalid_options");
  expect(rejected.width).toBe(2);
});

test("a cross line drags on both axes from either line and undoes as one step", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("cross_line", [
    { logical: s.l1, price: s.p_mid },
  ], { color: "#e91e63" }).id, { s });
  await settle_frames(page);
  const anchor = await spot(page, s.l1, s.p_mid);
  // Grab the horizontal line far from the crossing and drag diagonally.
  await page.mouse.move(anchor.x - 120, anchor.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(anchor.x - 100, anchor.y - 30, { steps: 4 });
  await page.mouse.move(anchor.x - 80, anchor.y - 40, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points()[0], id);
  expect(moved.logical).toBeGreaterThan(s.l1);
  expect(moved.price).toBeGreaterThan(s.p_mid);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  const restored = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points()[0], id);
  expect(restored.logical).toBeCloseTo(s.l1, 9);
  expect(restored.price).toBeCloseTo(s.p_mid, 9);
});

test("Lines tools round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
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
      ["ray", [{ logical: 1, price: 10 }, { logical: 3, price: 12 }], { extend_right: false, extend_left: true }],
      ["extended_line", [{ logical: 1, price: 9 }, { logical: 4, price: 11 }], {}],
      ["info_line", [{ logical: 2, price: 10 }, { logical: 6, price: 13 }], { tool_options: { line: { stats_position: "middle" } } }],
      ["trend_angle", [{ logical: 2, price: 11 }, { logical: 5, price: 12 }], { style: "dashed" }],
      ["cross_line", [{ logical: 3, price: 11.5 }], { width: 3 }],
      ["arrow_line", [{ logical: 1, price: 12 }, { logical: 5, price: 10 }], { stroke_end: "none", stroke_start: "arrow" }],
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
  const ray_style = result.state.drawings[0].style;
  expect(ray_style.extend_left).toBe(true);
  expect(ray_style.extend_right).toBe(false);
  expect(result.state.drawings[1].style.extend_right).toBeUndefined();
  expect(result.state.drawings[2].style.tool_options).toEqual({ line: { stats_position: "middle" } });
  const semantic = (list) => list.map(({ kind, options }) => ({
    kind,
    extend_left: options.extend_left,
    extend_right: options.extend_right,
    stroke_start: options.stroke_start,
    stroke_end: options.stroke_end,
    labels: options.labels,
    tool_options: options.tool_options,
    style: options.style,
    width: options.width,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("the demo toolbar arms every Lines tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of LINES) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  await page.click("#drawings_group [data-tool='arrow_line']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Lines tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
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
      const mid = (lo + hi) / 2;
      const up = (fraction) => lo + (hi - lo) * fraction;
      chart.add_drawing("ray", [{ logical: at(0.1), price: up(0.1) }, { logical: at(0.25), price: up(0.3) }], { color: "#e91e63", width: 2 });
      chart.add_drawing("extended_line", [{ logical: at(0.3), price: hi }, { logical: at(0.45), price: mid }]);
      chart.add_drawing("info_line", [{ logical: at(0.2), price: mid }, { logical: at(0.5), price: hi }], { color: "#089981" });
      chart.add_drawing("trend_angle", [{ logical: at(0.55), price: lo }, { logical: at(0.7), price: mid }], { color: "#7b1fa2" });
      chart.add_drawing("cross_line", [{ logical: at(0.8), price: up(0.25) }], { style: "dotted" });
      chart.add_drawing("arrow_line", [{ logical: at(0.15), price: mid }, { logical: at(0.35), price: lo }], { color: "#ff6d00", width: 3, stroke_start: "circle" });
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
  expect(differs(clean, canvas.png), "Lines paint on Canvas2D").toBeGreaterThan(1000);
  expect(differs(clean, gpu.png), "Lines paint on WebGPU").toBeGreaterThan(1000);

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
  console.log(`lines parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["canvas2d.png", canvas.png], ["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "Lines geometry and paint order match across executors").toBe(0);
});
