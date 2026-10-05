import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Patterns, Elliott waves, and cycles family through the public API and real pointer input:
// armed multi-click placement of every tool with its progressive preview, handle and body edits
// with undo, the flat Elliott degree and its effect on paint, cycle repeats across the pane,
// persistence/clipboard/sync round trips, the demo toolbar, and WebGPU == Canvas2D parity. Every
// geometry decision is engine-owned; these specs only drive the package API and the pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const ANCHORS = {
  pattern_xabcd: 5,
  pattern_cypher: 5,
  pattern_abcd: 4,
  pattern_head_shoulders: 7,
  pattern_triangle: 5,
  pattern_three_drives: 6,
  elliott_impulse: 6,
  elliott_correction: 4,
  elliott_triangle: 6,
  elliott_double_combination: 4,
  elliott_triple_combination: 6,
  cyclic_lines: 2,
  time_cycles: 2,
  sine_line: 2,
};
const KINDS = Object.keys(ANCHORS);
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

/** Device-px x extent and pixel count of `target`-colored pixels, or null when none. */
function color_extent(png, target, tol = 12) {
  let min = Infinity;
  let max = -Infinity;
  let count = 0;
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
        count += 1;
      }
    }
  }
  return Number.isFinite(min) ? { min, max, count } : null;
}

/**
 * `count` deterministic anchors alternating between the lows and highs of the visible bars, spread
 * across `[from, to]` of the visible logical range.
 */
async function zigzag(page, count, from = 0.15, to = 0.75) {
  return page.evaluate(({ count, from, to }) => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const logical = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
    let low = Infinity;
    let high = -Infinity;
    for (let index = logical(from); index <= logical(to); index += 1) {
      const bar = window.__main.data_by_index(index);
      low = Math.min(low, bar.low);
      high = Math.max(high, bar.high);
    }
    return Array.from({ length: count }, (_, index) => {
      const fraction = count === 1 ? from : from + ((to - from) * index) / (count - 1);
      const up = index % 2 === 1;
      return {
        logical: logical(fraction),
        price: up ? high - (high - low) * (0.1 + 0.02 * index) : low + (high - low) * (0.1 + 0.02 * index),
      };
    });
  }, { count, from, to });
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

test("every pattern, wave, and cycle tool places through the armed-tool flow and paints", async ({ page }) => {
  // Fourteen tools, each on a fresh chart so its pixels are its own.
  test.setTimeout(60_000);
  for (const kind of KINDS) {
    await goto_fixture(page);
    const clean = await capture(page);
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    const anchors = await zigzag(page, ANCHORS[kind]);
    for (const [step, anchor] of anchors.entries()) {
      const point = await spot(page, anchor.logical, anchor.price);
      await page.mouse.click(point.x, point.y);
      if (step + 1 < anchors.length) {
        expect(await page.evaluate(() => window.__chart.active_drawing_tool()), `${kind} stays armed`).toBe(kind);
      }
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${kind}`).toHaveLength(1);
    const [created] = list;
    expect(created.kind).toBe(kind);
    expect(created.points).toHaveLength(ANCHORS[kind]);
    expect(created.options.color).toBe("#e91e63");
    for (const point of created.points) expect(Number.isFinite(point.time)).toBe(true);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const pixels = await capture(page);
    const diff = pixelmatch(clean.data, pixels.data, null, pixels.width, pixels.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
    expect(color_extent(pixels, PINK), `${kind} strokes in its color`).not.toBeNull();
  }
});

test("multi-anchor placement previews progressively and edits by handle and body", async ({ page }) => {
  await goto_fixture(page);
  const anchors = await zigzag(page, 5);
  await page.evaluate(() => window.__chart.set_drawing_tool("pattern_xabcd", { color: "#e91e63" }));
  const first = await spot(page, anchors[0].logical, anchors[0].price);
  const second = await spot(page, anchors[1].logical, anchors[1].price);
  const third = await spot(page, anchors[2].logical, anchors[2].price);
  await page.mouse.click(first.x, first.y);
  await page.mouse.click(second.x, second.y);
  await page.mouse.move(third.x, third.y, { steps: 3 });
  await settle_frames(page);
  // X-A plus the leg to the pointer paint before the pattern exists (the placed X handle may
  // cover the first few pixels of the leg).
  const preview = color_extent(await capture(page), PINK);
  expect(preview).not.toBeNull();
  expect(preview.min).toBeLessThanOrEqual(Math.ceil((first.x + 6) * PR));
  expect(preview.max).toBeGreaterThanOrEqual(Math.floor(third.x * PR) - 2);
  expect(await drawings(page)).toHaveLength(0);
  for (const anchor of anchors.slice(2)) {
    const point = await spot(page, anchor.logical, anchor.price);
    await page.mouse.click(point.x, point.y);
  }
  await settle_frames(page);
  const [created] = await drawings(page);
  expect(created.kind).toBe("pattern_xabcd");
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(created.id);

  // Drag the B handle: only that anchor moves, and undo restores it as one step.
  await page.waitForTimeout(700);
  await page.mouse.move(third.x, third.y);
  await expect.poll(() => overlay_cursor(page)).toBe("pointer");
  await page.mouse.down();
  await page.mouse.move(third.x + 20, third.y + 15, { steps: 4 });
  await page.mouse.move(third.x + 40, third.y + 25, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  let [edited] = await drawings(page);
  expect(edited.points[2].logical).toBeGreaterThan(created.points[2].logical);
  for (const index of [0, 1, 3, 4]) expect(edited.points[index]).toEqual(created.points[index]);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  [edited] = await drawings(page);
  expect(edited.points).toEqual(created.points);

  // A body drag from the middle of the X-A leg moves every anchor.
  const middle = { x: (first.x + second.x) / 2, y: (first.y + second.y) / 2 };
  await page.mouse.move(middle.x, middle.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(middle.x + 15, middle.y - 10, { steps: 4 });
  await page.mouse.move(middle.x + 30, middle.y - 20, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  [edited] = await drawings(page);
  for (const [index, point] of edited.points.entries()) {
    expect(point.logical).toBeGreaterThan(created.points[index].logical);
    expect(point.price).toBeGreaterThan(created.points[index].price);
  }
});

test("the Elliott degree is a flat, atomic option that relabels the wave", async ({ page }) => {
  await goto_fixture(page);
  const anchors = await zigzag(page, 6, 0.2, 0.7);
  const wave = await page.evaluate(({ anchors }) => {
    const drawing = window.__chart.add_drawing("elliott_impulse", anchors, { color: "#e91e63" });
    const schema = window.__chart.drawing_property_schema(drawing);
    const degree = schema.properties.find((property) => property.name === "wave_degree");
    return {
      id: drawing.id,
      kind_options: window.__chart.drawing_kind_options(drawing),
      degree_default: degree.default,
      degrees: degree.enum_values,
    };
  }, { anchors });
  expect(wave.kind_options).toEqual({ kind: "elliott", wave_degree: "minor" });
  expect(wave.degree_default).toBe("minor");
  expect([...wave.degrees].sort()).toEqual([
    "cycle", "grand_supercycle", "intermediate", "millennium", "minor", "minuette", "minute",
    "primary", "submillennium", "subminuette", "supercycle", "supermillennium",
  ]);
  await settle_frames(page);
  const minor = await capture(page);

  // Another degree relabels the wave; the change is a flat option.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({ wave_degree: "primary" });
  }, wave.id);
  await settle_frames(page);
  const primary = await capture(page);
  expect(pixelmatch(minor.data, primary.data, null, primary.width, primary.height, { threshold: 0 })).toBeGreaterThan(20);
  const options = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    return { wave_degree: drawing.options().wave_degree, kind_options: window.__chart.drawing_kind_options(drawing) };
  }, wave.id);
  expect(options).toEqual({ wave_degree: "primary", kind_options: { kind: "elliott", wave_degree: "primary" } });

  // Earlier builds' `tool_options.pattern.degree` moves onto the flat option.
  const legacy = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    drawing.apply_options({ tool_options: { pattern: { degree: "minute" } } });
    return drawing.options().wave_degree;
  }, wave.id);
  expect(legacy).toBe("minute");

  // An invalid degree is rejected atomically.
  const rejected = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    const width = drawing.options().width;
    try {
      drawing.apply_options({ width: width + 5, wave_degree: "tiny" });
    } catch (error) {
      return { code: error.code, unchanged: drawing.options().width === width };
    }
    return { code: null, unchanged: drawing.options().width === width };
  }, wave.id);
  expect(rejected).toEqual({ code: "invalid_options", unchanged: true });

  // The wave leg hovers.
  const a = await spot(page, anchors[2].logical, anchors[2].price);
  const b = await spot(page, anchors[3].logical, anchors[3].price);
  await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
});

test("cycles repeat across the pane from their anchors and hit on a far repeat", async ({ page }) => {
  await goto_fixture(page);
  const width = await page.evaluate(() => window.__chart.time_scale().width());
  const [start, end] = await zigzag(page, 2, 0.3, 0.36);
  await page.evaluate(({ start, end }) => window.__chart.add_drawing("cyclic_lines", [
    start, { logical: end.logical, price: start.price },
  ], { color: "#e91e63" }), { start, end });
  await settle_frames(page);
  const extent = color_extent(await capture(page), PINK);
  const first = await spot(page, start.logical, start.price);
  const second = await spot(page, end.logical, start.price);
  const spacing = second.x - first.x;
  // Cyclic lines repeat both ways across the whole pane.
  expect(extent.min).toBeLessThanOrEqual(Math.ceil(spacing * PR) + 2);
  expect(extent.max).toBeGreaterThanOrEqual(Math.floor((width - spacing - 2) * PR));
  // A repeat far right of both anchors is a hover target; the gap between two repeats is not.
  const repeat_x = first.x + spacing * Math.floor((width - first.x) / spacing - 1);
  await page.mouse.move(repeat_x, 60);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.move(repeat_x - spacing / 2, 60);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");

  // Time cycles and the sine line run from their first anchor to the right edge.
  for (const kind of ["time_cycles", "sine_line"]) {
    await goto_fixture(page);
    await page.evaluate(({ kind, start, end }) => window.__chart.add_drawing(kind, [start, end], { color: "#e91e63" }), { kind, start, end });
    await settle_frames(page);
    const right = color_extent(await capture(page), PINK);
    expect(right.min, `${kind} starts at its first anchor`).toBeGreaterThanOrEqual(Math.floor(first.x * PR) - 4);
    expect(right.max, `${kind} reaches the right edge`).toBeGreaterThanOrEqual(Math.floor((width - spacing) * PR));
  }
});

test("a time-cycle line selects the drawing and drags it as one body", async ({ page }) => {
  await goto_fixture(page);
  const [base, top] = await zigzag(page, 2, 0.3, 0.42);
  const cycles = await page.evaluate(({ base, top }) => window.__chart.add_drawing("time_cycles", [base, top], { color: "#e91e63" }).id, { base, top });
  await settle_frames(page);
  const b = await spot(page, top.logical, top.price);
  // The second cycle line runs through the second anchor's bar, full height.
  const line = { x: b.x, y: 200 };
  await page.mouse.click(line.x, line.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(cycles);
  await page.waitForTimeout(700);
  await page.mouse.move(line.x, line.y + 40);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(line.x + 20, line.y + 40, { steps: 4 });
  await page.mouse.move(line.x + 40, line.y + 40, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), cycles);
  expect(moved[0].logical).toBeGreaterThan(base.logical);
  expect(moved[1].logical).toBeGreaterThan(top.logical);
});

test("family tools round-trip through persistence, clipboard, and sync with their options", async ({ page }) => {
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
    const zig = (count) => Array.from({ length: count }, (_, index) => ({ logical: 1 + index * 2, price: index % 2 ? 12 : 10 + index * 0.1 }));
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const additions = [
      ["pattern_xabcd", zig(5), { fill_enabled: true, tool_options: { pattern: { show_ratios: false } } }],
      ["pattern_head_shoulders", zig(7), {}],
      ["pattern_triangle", zig(5), { fill_color: "#ff000033" }],
      ["elliott_impulse", zig(6), { wave_degree: "minute" }],
      ["elliott_triple_combination", zig(6), { tool_options: { pattern: { show_wave: false } }, style: "dashed" }],
      ["cyclic_lines", zig(2), { width: 3 }],
      ["time_cycles", zig(2), { fill_enabled: false }],
      ["sine_line", zig(2), { text: "cycle" }],
    ];
    for (const [kind, anchors, style] of additions) first.add_drawing(kind, anchors, style);
    const state = first.export_state();
    const copied = first.copy_drawings(first.drawings().map((drawing) => drawing.id));
    const sync = first.drawing_sync_payload("cell-a");
    const expected = first.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options(), points: drawing.points().length }));
    first.remove();
    first_host.remove();

    const second_host = host();
    const second = await create_chart(second_host, { backend: "canvas2d", autoSize: false });
    second.import_state(state);
    const canonical = second.export_state();
    const restored = second.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options(), points: drawing.points().length }));
    second.remove();
    second_host.remove();

    const third_host = host();
    const third = await create_chart(third_host, { backend: "canvas2d", autoSize: false });
    const pasted = third.paste_drawings(copied).map((drawing) => ({ kind: drawing.kind(), options: drawing.options(), points: drawing.points().length }));
    third.remove();
    third_host.remove();

    const fourth_host = host();
    const fourth = await create_chart(fourth_host, { backend: "canvas2d", autoSize: false });
    const applied = fourth.apply_drawing_sync_payload(sync);
    const synced = fourth.drawings().map((drawing) => ({ kind: drawing.kind(), options: drawing.options(), points: drawing.points().length }));
    fourth.remove();
    fourth_host.remove();
    return { state, canonical, expected, restored, pasted, applied, synced };
  });

  expect(result.canonical).toEqual(result.state);
  const styles = result.state.drawings.map((drawing) => drawing.style);
  expect(styles[0].fill_enabled).toBe(true);
  // Earlier builds' pattern options stay stored, inert.
  expect(styles[0].tool_options.pattern.show_ratios).toBe(false);
  expect(styles[1].fill_enabled).toBeUndefined();
  expect(styles[1].tool_options).toBeUndefined();
  expect(styles[3].wave_degree).toBe("minute");
  const semantic = (list) => list.map(({ kind, options, points }) => ({
    kind,
    points,
    color: options.color,
    width: options.width,
    style: options.style,
    fill_enabled: options.fill_enabled,
    fill_color: options.fill_color,
    text: options.text,
    tool_options: options.tool_options,
    wave_degree: options.wave_degree,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("the demo toolbar arms every pattern, wave, and cycle tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of KINDS) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  await page.click("#drawings_group [data-tool='sine_line']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("patterns, waves, and cycles render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
  const run_scenario = async (backend) => {
    await goto_fixture(page, backend);
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await page.evaluate(() => {
      const chart = window.__chart;
      const range = chart.time_scale().get_visible_logical_range();
      const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
      let lo = Infinity;
      let hi = -Infinity;
      for (let index = at(0.05); index <= at(0.95); index += 1) {
        const bar = window.__main.data_by_index(index);
        lo = Math.min(lo, bar.low);
        hi = Math.max(hi, bar.high);
      }
      const up = (fraction) => lo + (hi - lo) * fraction;
      const zig = (from, to, count, low, high) => Array.from({ length: count }, (_, index) => ({
        logical: at(from + ((to - from) * index) / (count - 1)),
        price: up(index % 2 ? high : low),
      }));
      chart.add_drawing("pattern_xabcd", zig(0.05, 0.3, 5, 0.55, 0.85), { color: "#e91e63" });
      chart.add_drawing("pattern_head_shoulders", zig(0.35, 0.65, 7, 0.6, 0.8));
      chart.add_drawing("pattern_triangle", [
        { logical: at(0.7), price: up(0.95) }, { logical: at(0.74), price: up(0.6) },
        { logical: at(0.8), price: up(0.88) }, { logical: at(0.84), price: up(0.68) },
        { logical: at(0.88), price: up(0.8) },
      ]);
      chart.add_drawing("elliott_impulse", zig(0.05, 0.4, 6, 0.1, 0.4), { wave_degree: "primary" });
      chart.add_drawing("elliott_correction", zig(0.45, 0.6, 4, 0.35, 0.15));
      chart.add_drawing("cyclic_lines", [{ logical: at(0.62), price: up(0.2) }, { logical: at(0.68), price: up(0.2) }], { style: "dotted" });
      chart.add_drawing("time_cycles", [{ logical: at(0.5), price: up(0.02) }, { logical: at(0.58), price: up(0.25) }]);
      chart.add_drawing("sine_line", [{ logical: at(0.2), price: up(0.5) }, { logical: at(0.26), price: up(0.42) }], { color: "#ff6d00" });
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
  console.log(`patterns/waves/cycles parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
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
