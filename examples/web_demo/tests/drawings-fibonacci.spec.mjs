import { test, expect } from "@playwright/test";
import { readFileSync, writeFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// B8 Fibonacci family (retracement, trend-based extension, channel, time zones, trend-based time,
// speed resistance fan and arcs, circles, spiral, wedge) through the public API and real pointer
// input: armed placement of every tool, the three-anchor first-leg preview, level lines and their
// hits, level-list and flat level-option edits, body drags with undo, the stored tool options
// (trend line, label placement, label and selected-band hits, the golden spiral),
// persistence/clipboard/sync round trips, the demo toolbar, and WebGPU == Canvas2D parity. Every
// geometry decision is engine-owned; these specs only drive the package API and pointer.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const FIBONACCI = [
  "fibonacci_retracement",
  "fibonacci_extension",
  "fibonacci_channel",
  "fibonacci_time_zones",
  "fibonacci_trend_time",
  "fibonacci_speed_fan",
  "fibonacci_speed_arcs",
  "fibonacci_circles",
  "fibonacci_spiral",
  "fibonacci_wedge",
];
const THREE_ANCHORS = new Set(["fibonacci_extension", "fibonacci_channel", "fibonacci_trend_time", "fibonacci_wedge"]);
const GREEN = [76, 175, 80]; // #4caf50
const PINK = [233, 30, 99]; // #e91e63

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

/** Device-px rows holding at least `min_run` horizontally adjacent `target`-colored pixels. */
function color_rows(png, target, min_run = 12, tol = 10) {
  const rows = [];
  for (let y = 0; y < png.height; y += 1) {
    let run = 0;
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      const hit = Math.abs(png.data[o] - target[0]) <= tol
        && Math.abs(png.data[o + 1] - target[1]) <= tol
        && Math.abs(png.data[o + 2] - target[2]) <= tol;
      run = hit ? run + 1 : 0;
      if (run >= min_run) {
        rows.push(y);
        break;
      }
    }
  }
  return rows;
}

/**
 * Mid-range visible logical indexes and prices 200 CSS px apart, so neighbouring levels sit well
 * beyond the pointer's hit tolerance whatever the fixture's price range.
 */
async function anchor_spots(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const at = (fraction) => Math.floor(range.from + (range.to - range.from) * fraction);
    const price = (y) => window.__main.coordinate_to_price(y);
    return { l0: at(0.3), l1: at(0.5), l2: at(0.62), p_lo: price(460), p_hi: price(260), p_mid: price(360) };
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

test("every Fibonacci tool places through the armed-tool flow and paints", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);
  for (const kind of FIBONACCI) {
    await page.evaluate((kind) => {
      window.__chart.set_drawing_tool(kind, { color: "#e91e63" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`${kind} not armed`);
    }, kind);
    const clicks = [[s.l0, s.p_lo], [s.l1, s.p_hi], [s.l2, s.p_mid]].slice(0, THREE_ANCHORS.has(kind) ? 3 : 2);
    for (const [logical, price] of clicks) {
      const point = await spot(page, logical, price);
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
    const pixels = await capture(page);
    const diff = pixelmatch(clean.data, pixels.data, null, clean.width, clean.height, { threshold: 0 });
    expect(diff, `${kind} paints`).toBeGreaterThan(20);
    // Remove it again so each tool is measured against the clean chart.
    await page.evaluate(() => window.__chart.clear_drawings());
  }
  // Earlier builds' spellings arm the canonical tool.
  expect(await page.evaluate(() => {
    window.__chart.set_drawing_tool("fib_retracement");
    const armed = window.__chart.active_drawing_tool();
    window.__chart.set_drawing_tool(null);
    return armed;
  })).toBe("fibonacci_retracement");
});

test("three-anchor tools preview their first leg before the second click", async ({ page }) => {
  await goto_fixture(page);
  // Hide the crosshair so only drawing geometry changes the pixels.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("fibonacci_extension", { color: "#e91e63" }));
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.l1, s.p_hi);
  await page.mouse.click(a.x, a.y);
  await settle_frames(page);
  const before = await capture(page);
  await page.mouse.move(b.x, b.y, { steps: 4 });
  await settle_frames(page);
  const preview = await capture(page);
  const diff = pixelmatch(before.data, preview.data, null, before.width, before.height, { threshold: 0 });
  expect(diff, "the dashed first leg follows the pointer").toBeGreaterThan(20);
  await page.mouse.click(b.x, b.y);
  const c = await spot(page, s.l2, s.p_mid);
  await page.mouse.click(c.x, c.y);
  await settle_frames(page);
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect(list[0].points).toHaveLength(3);
});

test("the retracement paints ratio levels and hits its level lines", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const info = await page.evaluate(({ s }) => {
    const drawing = window.__chart.add_drawing("fibonacci_retracement", [
      { logical: s.l0, price: s.p_lo },
      { logical: s.l1, price: s.p_hi },
    ], { color: "#e91e63" });
    return {
      id: drawing.id,
      levels: drawing.options().levels.map((level) => level.value),
      kind_options: window.__chart.drawing_kind_options(drawing),
      schema: window.__chart.drawing_property_schema(drawing).properties
        .filter((property) => property.name.startsWith("level_"))
        .map((property) => [property.name, property.default]),
    };
  }, { s });
  expect(info.levels).toEqual([0, 0.236, 0.382, 0.5, 0.618, 0.786, 1]);
  expect(info.kind_options).toMatchObject({ kind: "levels", reverse: false, log_scale: false, label_align: "right" });
  expect(info.schema).toContainEqual(["level_log_scale", false]);
  await settle_frames(page);

  // Level 0 sits on the first anchor: the 0.236 level paints a quarter of the way up from it.
  const rows = color_rows(await capture(page), PINK);
  const low = await spot(page, (s.l0 + s.l1) / 2, s.p_lo + (s.p_hi - s.p_lo) * 0.236);
  expect(rows.some((row) => Math.abs(row - low.y * PR) <= 2), `rows ${rows} near ${low.y * PR}`).toBe(true);

  // A level line hovers and selects the drawing; the band between two levels does not (it is
  // unfilled here, and a band is a drag surface only while selected). The spot sits clear of
  // the level labels, which hover like the lines.
  const half = (s.p_lo + s.p_hi) / 2;
  const level = await spot(page, (s.l0 + s.l1) / 2, half);
  const band = await spot(page, s.l0 + (s.l1 - s.l0) * 0.15, s.p_lo + (s.p_hi - s.p_lo) * 0.7);
  await page.mouse.move(band.x, band.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  await page.mouse.move(level.x, level.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.click(level.x, level.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(info.id);

  // Flat level options: reverse moves level 0 to the second anchor; an invalid value rejects the
  // whole patch. Earlier builds' `tool_options.fibonacci` keys move onto the flat options (their
  // `reverse` had level 0 on the second anchor already, so it inverts).
  const edited = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    drawing.apply_options({ level_reverse: true, level_show_percents: true });
    let code = null;
    try {
      drawing.apply_options({ width: 7, level_label_align: "sideways" });
    } catch (error) {
      code = error.code;
    }
    const flat = {
      code,
      width: drawing.options().width,
      kind_options: window.__chart.drawing_kind_options(drawing),
    };
    drawing.apply_options({ tool_options: { fibonacci: { reverse: true, label_h_align: "left" } } });
    return { ...flat, legacy: window.__chart.drawing_kind_options(drawing), tool_options: drawing.options().tool_options };
  }, info.id);
  expect(edited.code).toBe("invalid_options");
  expect(edited.width).toBe(1);
  expect(edited.kind_options).toMatchObject({ kind: "levels", reverse: true, show_percents: true });
  expect(edited.legacy).toMatchObject({ reverse: false, label_align: "left" });
  expect(edited.tool_options).toEqual({});
  await settle_frames(page);
  const reversed = color_rows(await capture(page), PINK);
  const high = await spot(page, (s.l0 + s.l1) / 2, s.p_hi - (s.p_hi - s.p_lo) * 0.236);
  expect(reversed.some((row) => Math.abs(row - high.y * PR) <= 2), `rows ${reversed} near ${high.y * PR}`).toBe(false);
});

test("level lists edit values, visibility, colors, and fills", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("fibonacci_retracement", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.l1, price: s.p_hi },
  ]).id, { s });
  await settle_frames(page);
  // Level 0 sits on the first anchor, so the 0.25 level is a quarter of the way up from it.
  const target = await spot(page, (s.l0 + s.l1) / 2, s.p_lo + (s.p_hi - s.p_lo) * 0.25);
  expect(color_rows(await capture(page), [233, 30, 99]).length).toBe(0);
  const levels = await page.evaluate((id) => {
    const drawing = window.__chart.drawings().find((candidate) => candidate.id === id);
    drawing.apply_options({
      levels: [
        { value: 0, color: "#787b86", visible: true, style: "solid", fill_between: false, label_visible: true },
        { value: 0.25, color: "#e91e63", visible: true, style: "solid", fill_between: true, label_visible: true },
        { value: 0.5, color: "#4caf50", visible: false, style: "solid", fill_between: true, label_visible: true },
        { value: 1, color: "#787b86", visible: true, style: "dashed", fill_between: false, label_visible: false },
      ],
    });
    return drawing.options().levels;
  }, id);
  expect(levels.map((level) => level.value)).toEqual([0, 0.25, 0.5, 1]);
  await settle_frames(page);
  const png = await capture(page);
  const pink = color_rows(png, [233, 30, 99]);
  expect(pink.some((row) => Math.abs(row - target.y * PR) <= 2), `pink rows ${pink}`).toBe(true);
  expect(color_rows(png, GREEN).length, "hidden levels paint nothing").toBe(0);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  const restored = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).options().levels.length, id);
  expect(restored).toBe(7);
});

test("a trend-based extension drags from a level line and undoes as one step", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ s }) => window.__chart.add_drawing("fibonacci_extension", [
    { logical: s.l0, price: s.p_lo },
    { logical: s.l1, price: s.p_hi },
    { logical: s.l2, price: s.p_mid },
  ], { tool_options: { fibonacci: { trend_line: false } } }).id, { s });
  await settle_frames(page);
  // Level 1 sits one full first-leg move above the third anchor, spanning from it.
  const level = await spot(page, s.l2 + (s.l1 - s.l0) / 2, s.p_mid + (s.p_hi - s.p_lo));
  await page.mouse.move(level.x, level.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(level.x + 20, level.y + 15, { steps: 4 });
  await page.mouse.move(level.x + 40, level.y + 30, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  expect(moved[0].logical).toBeGreaterThan(s.l0);
  expect(moved[2].price).toBeLessThan(s.p_mid);
  expect(await page.evaluate(() => window.__chart.undo_drawing())).toBe(true);
  const restored = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), id);
  expect(restored[0].logical).toBeCloseTo(s.l0, 9);
  expect(restored[2].price).toBeCloseTo(s.p_mid, 9);
});

test("an already synced cell accepts pointer drags and armed placements", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const initial = await page.evaluate(async ({ s }) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;top:0;width:800px;height:500px";
    document.body.append(host);
    const mirror = await create_chart(host, { backend: "canvas2d", autoSize: false });
    mirror.add_series("candlestick").set_data(window.__main.data());
    window.__mirror = mirror;
    window.__sync = () => mirror.apply_drawing_sync_payload(window.__chart.drawing_sync_payload("cell-a"));
    const line = window.__chart.add_drawing("trend_line", [
      { logical: s.l0, price: s.p_lo },
      { logical: s.l2, price: s.p_hi },
    ]);
    return { applied: window.__sync(), id: line.id, revision: line.options().revision };
  }, { s });
  expect(initial.applied).toBe(true);
  await settle_frames(page);

  // A pointer drag of the trend line's body on the first cell.
  const mid = await spot(page, (s.l0 + s.l2) / 2, (s.p_lo + s.p_hi) / 2);
  await page.mouse.move(mid.x, mid.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(mid.x + 20, mid.y + 15, { steps: 4 });
  await page.mouse.move(mid.x + 40, mid.y + 30, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const dragged = await page.evaluate((id) => {
    const source = window.__chart.drawings().find((drawing) => drawing.id === id);
    const applied = window.__sync();
    const mirrored = window.__mirror.drawings().find((drawing) => drawing.id === id);
    return { applied, revision: source.options().revision, source: source.points(), mirrored: mirrored.points() };
  }, initial.id);
  expect(dragged.applied).toBe(true);
  expect(dragged.revision).toBeGreaterThan(initial.revision);
  expect(dragged.source[0].logical).toBeGreaterThan(s.l0);
  for (const [index, point] of dragged.mirrored.entries()) {
    expect(point.time).toBe(dragged.source[index].time);
    expect(point.price).toBeCloseTo(dragged.source[index].price, 9);
  }

  // An armed retracement placed with real clicks on the first cell.
  await page.evaluate(() => window.__chart.set_drawing_tool("fibonacci_retracement"));
  for (const [logical, price] of [[s.l1, s.p_hi], [s.l2, s.p_mid]]) {
    const point = await spot(page, logical, price);
    await page.mouse.click(point.x, point.y);
  }
  await settle_frames(page);
  const placed = await page.evaluate(() => ({
    applied: window.__sync(),
    kinds: window.__mirror.drawings().map((drawing) => drawing.kind()).sort(),
  }));
  expect(placed).toEqual({ applied: true, kinds: ["fibonacci_retracement", "trend_line"] });
  await page.evaluate(() => window.__mirror.remove());
});

test("Fibonacci tools round-trip through persistence, clipboard, and sync with their levels and options", async ({ page }) => {
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
    const two = [{ logical: 1, price: 10 }, { logical: 4, price: 12 }];
    const three = [...two, { logical: 6, price: 11 }];
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const additions = [
      ["fibonacci_retracement", two, { extend_right: true, level_reverse: true, level_label_align: "left" }],
      ["fibonacci_extension", three, { level_log_scale: true }],
      ["fibonacci_channel", three, { levels: [] }],
      ["fibonacci_time_zones", two, { fill_enabled: true }],
      ["fibonacci_trend_time", three, {}],
      ["fibonacci_speed_fan", two, { tool_options: { fibonacci: { grid: false } } }],
      ["fibonacci_speed_arcs", two, { tool_options: { fibonacci: { full_circles: true } } }],
      ["fibonacci_circles", two, { levels: [{ value: 0.5, color: "#abcdef", visible: true, style: "dashed", fill_between: false, label_visible: false }] }],
      ["fibonacci_spiral", two, { level_reverse: true, style: "dotted" }],
      ["fibonacci_wedge", three, { fill_enabled: false }],
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
  expect(styles[0].level_reverse).toBe(true);
  expect(styles[0].level_label_align).toBe("left");
  expect(styles[0].tool_options).toBeUndefined();
  expect(styles[1].level_log_scale).toBe(true);
  expect(styles[0].extend_right).toBe(true);
  expect(styles[0].levels, "default levels are omitted").toBeUndefined();
  expect(styles[2].levels, "a cleared level list persists").toEqual([]);
  expect(styles[4].tool_options).toBeUndefined();
  const semantic = (list) => list.map(({ kind, options }) => ({
    kind,
    levels: options.levels,
    fill_enabled: options.fill_enabled,
    extend_right: options.extend_right,
    tool_options: options.tool_options,
    style: options.style,
    color: options.color,
    level_reverse: options.level_reverse,
    level_log_scale: options.level_log_scale,
    level_label_align: options.level_label_align,
  }));
  expect(semantic(result.restored)).toEqual(semantic(result.expected));
  expect(semantic(result.pasted)).toEqual(semantic(result.expected));
  expect(result.applied).toBe(true);
  expect(semantic(result.synced)).toEqual(semantic(result.expected));
});

test("Fibonacci tool options paint and hit through real pointer input", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  const levels = [0, 0.5, 1].map((value) => ({
    value, color: "#4caf50", visible: true, style: "solid", fill_between: true, label_visible: true,
  }));
  const info = await page.evaluate(({ s, levels }) => {
    const drawing = window.__chart.add_drawing("fibonacci_retracement", [
      { logical: s.l0, price: s.p_lo },
      { logical: s.l1, price: s.p_hi },
    ], { color: "#e91e63", levels, level_label_align: "left" });
    return {
      id: drawing.id,
      schema: window.__chart.drawing_property_schema(drawing).properties
        .filter((property) => property.name.startsWith("tool_options.fibonacci."))
        .map((property) => [property.name, property.default]),
    };
  }, { s, levels });
  expect(info.schema).toEqual([
    ["tool_options.fibonacci.trend_line", false],
    ["tool_options.fibonacci.label_v_align", "top"],
  ]);
  await settle_frames(page);
  const plain = await capture(page);
  const a = await spot(page, s.l0, s.p_lo);
  const b = await spot(page, s.l1, s.p_hi);
  // A tenth of the way along the anchors: between levels 0 and 0.5, off the labels.
  const on_trend = { x: a.x + (b.x - a.x) * 0.1, y: a.y + (b.y - a.y) * 0.1 };
  await page.mouse.move(on_trend.x, on_trend.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");

  // The stored options: the trend line through the anchors and labels beside the lines' left
  // ends, centered on them.
  await page.evaluate((id) => {
    window.__chart.drawings().find((drawing) => drawing.id === id).apply_options({
      tool_options: { fibonacci: { trend_line: true, label_v_align: "middle" } },
    });
  }, info.id);
  await settle_frames(page);
  const styled = await capture(page);
  const diff = pixelmatch(plain.data, styled.data, null, plain.width, plain.height, { threshold: 0 });
  expect(diff, "the trend line and the moved labels paint").toBeGreaterThan(50);
  await page.mouse.move(on_trend.x + 1, on_trend.y + 1);
  await page.mouse.move(on_trend.x, on_trend.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");

  // A label left of the 0.5 line hovers like the line.
  const level_half = await spot(page, s.l0, (s.p_lo + s.p_hi) / 2);
  await page.mouse.move(level_half.x - 20, level_half.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");

  // The band between levels 0 and 0.5 pans the chart until the drawing is selected, then drags it.
  const band = await spot(page, s.l0 + (s.l1 - s.l0) * 0.7, s.p_lo + (s.p_hi - s.p_lo) * 0.3);
  await page.mouse.move(band.x, band.y);
  await expect.poll(() => overlay_cursor(page)).not.toBe("move");
  await page.mouse.click(on_trend.x, on_trend.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id)).toBe(info.id);
  await page.mouse.move(band.x + 1, band.y);
  await page.mouse.move(band.x, band.y);
  await expect.poll(() => overlay_cursor(page)).toBe("move");
  await page.mouse.down();
  await page.mouse.move(band.x + 20, band.y + 15, { steps: 4 });
  await page.mouse.move(band.x + 40, band.y + 30, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = await page.evaluate((id) => window.__chart.drawings().find((drawing) => drawing.id === id).points(), info.id);
  expect(moved[0].logical).toBeGreaterThan(s.l0);
  expect(moved[0].price).toBeLessThan(s.p_lo);

  // An empty spiral paints the golden spiral.
  const before = await capture(page);
  await page.evaluate(({ s }) => {
    window.__chart.add_drawing("fibonacci_spiral", [
      { logical: s.l2, price: s.p_mid },
      { logical: s.l2 + 2, price: s.p_mid },
    ], { color: "#7b1fa2", levels: [] });
  }, { s });
  await settle_frames(page);
  const spiral = await capture(page);
  const painted = pixelmatch(before.data, spiral.data, null, before.width, before.height, { threshold: 0 });
  expect(painted, "the golden spiral paints").toBeGreaterThan(200);
});

test("the demo toolbar arms every Fibonacci tool", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const kind of FIBONACCI) {
    await page.click(`#drawings_group [data-tool='${kind}']`);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe(kind);
  }
  await page.click("#drawings_group [data-tool='fibonacci_wedge']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("Fibonacci tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
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
      chart.add_drawing("fibonacci_retracement", [{ logical: at(0.1), price: up(0.1) }, { logical: at(0.3), price: up(0.9) }]);
      chart.add_drawing("fibonacci_time_zones", [{ logical: at(0.05), price: up(0.5) }, { logical: at(0.08), price: up(0.5) }], { fill_enabled: true });
      chart.add_drawing("fibonacci_speed_fan", [{ logical: at(0.4), price: up(0.2) }, { logical: at(0.5), price: up(0.6) }]);
      chart.add_drawing("fibonacci_circles", [{ logical: at(0.62), price: up(0.4) }, { logical: at(0.66), price: up(0.55) }]);
      chart.add_drawing("fibonacci_spiral", [{ logical: at(0.8), price: up(0.5) }, { logical: at(0.82), price: up(0.5) }], { color: "#7b1fa2" });
      // Extended both ways: the bands are the pane clipped between neighbouring level lines.
      chart.add_drawing("fibonacci_channel", [
        { logical: at(0.35), price: up(0.9) }, { logical: at(0.55), price: up(1.0) }, { logical: at(0.45), price: up(0.75) },
      ], { extend_left: true, extend_right: true });
      // Centered left of the pane with dashed rings: only the pane's angular window is
      // tessellated, each ring starting on its own dash phase.
      const ring = (value, color) => ({ value, color, visible: true, style: "dashed", fill_between: true, label_visible: true });
      chart.add_drawing("fibonacci_circles", [
        { logical: range.from - (range.to - range.from) * 0.3, price: up(0.5) },
        { logical: range.from - (range.to - range.from) * 0.1, price: up(0.5) },
      ], { levels: [ring(1.5, "#2962ff"), ring(2, "#f23645"), ring(2.5, "#089981")] });
      // The same dashed rings centered right of the pane as precise rings (a stored block): the
      // engine splits their dashes over the pane's window of each ring.
      chart.add_drawing("fibonacci_circles", [
        { logical: range.to + (range.to - range.from) * 0.3, price: up(0.5) },
        { logical: range.to + (range.to - range.from) * 0.1, price: up(0.5) },
      ], {
        levels: [ring(1.5, "#2962ff"), ring(2, "#f23645"), ring(2.5, "#089981")],
        tool_options: { fibonacci: { trend_line: false } },
      });
      chart.add_drawing("fibonacci_speed_arcs", [{ logical: at(0.12), price: up(0.25) }, { logical: at(0.16), price: up(0.4) }], { style: "dotted" });
      chart.add_drawing("fibonacci_extension", [
        { logical: at(0.5), price: up(0.05) }, { logical: at(0.56), price: up(0.25) }, { logical: at(0.6), price: up(0.15) },
      ], { extend_right: true });
      chart.add_drawing("fibonacci_trend_time", [
        { logical: at(0.86), price: up(0.9) }, { logical: at(0.89), price: up(0.75) }, { logical: at(0.91), price: up(0.85) },
      ]);
      chart.add_drawing("fibonacci_wedge", [
        { logical: at(0.7), price: up(0.1) }, { logical: at(0.78), price: up(0.3) }, { logical: at(0.78), price: up(0.0) },
      ]);
      // The stored options: the dashed trend line, labels beside the lines' ends, the fan grid,
      // full circles over precise rings, and the golden spiral of an empty spiral.
      const fill = (value, color) => ({ value, color, visible: true, style: "solid", fill_between: true, label_visible: true });
      chart.add_drawing("fibonacci_retracement", [{ logical: at(0.66), price: up(0.6) }, { logical: at(0.74), price: up(0.95) }], {
        style: "dashed", level_label_align: "left", levels: [fill(0, "#787b86"), fill(0.5, "#4caf50"), fill(1, "#2962ff")],
        tool_options: { fibonacci: { trend_line: true, label_v_align: "middle" } },
      });
      chart.add_drawing("fibonacci_speed_fan", [{ logical: at(0.24), price: up(0.6) }, { logical: at(0.32), price: up(0.95) }], {
        tool_options: { fibonacci: { grid: true } },
      });
      chart.add_drawing("fibonacci_speed_arcs", [{ logical: at(0.92), price: up(0.2) }, { logical: at(0.95), price: up(0.35) }], {
        tool_options: { fibonacci: { full_circles: true, trend_line: true } },
      });
      chart.add_drawing("fibonacci_spiral", [{ logical: at(0.45), price: up(0.45) }, { logical: at(0.47), price: up(0.5) }], {
        color: "#00897b", levels: [], tool_options: { fibonacci: { trend_line: true } },
      });
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
  expect(differs(clean, canvas.png), "Fibonacci tools paint on Canvas2D").toBeGreaterThan(1000);
  expect(differs(clean, gpu.png), "Fibonacci tools paint on WebGPU").toBeGreaterThan(1000);

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
  console.log(`fibonacci parity: ${edge_diff} AA-edge pixels, ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    for (const [name, png] of [["canvas2d.png", canvas.png], ["webgpu.png", gpu.png], ["diff.png", visual]]) {
      const path = test_info.outputPath(name);
      writeFileSync(path, PNG.sync.write(png));
      await test_info.attach(name, { path, contentType: "image/png" });
    }
  }
  expect(ordering_diff, "Fibonacci geometry and paint order match across executors").toBe(0);
});
