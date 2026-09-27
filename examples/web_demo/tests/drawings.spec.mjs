import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

// Engine-owned drawing tools: creation via the armed-tool click flow, click-to-select with
// anchor handles, anchor re-anchor drags, whole-body moves, per-tool text labels with 3×3
// alignment, Delete/Backspace removal, and WebGPU == Canvas2D pixel parity. The engine owns
// all state and drag math (drawings.rs); the specs drive the public API + pointer only.

const fixture = JSON.parse(readFileSync(new URL("../fixtures/d1/candles.json", import.meta.url), "utf8"));
const PR = fixture.pixel_ratio;
const BLUE = [0, 145, 255]; // semantic primary #0091ff — anchor border and drawing default
const PURPLE = [123, 31, 162]; // #7b1fa2 — text label color (collides with no fixture pixel)
// Ctrl/Cmd is the drawing magnet toggle. macOS Chromium turns Ctrl+CLICK into a secondary click
// (no placement), so click-based magnet tests hold Cmd there; press-drag and hover keep Ctrl.
const MAGNET_CLICK_KEY = process.platform === "darwin" ? "Meta" : "Control";

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function wait_for_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function settle_frames(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function goto_fixture(page, backend = "canvas2d") {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await wait_for_chart(page);
}

/** The chart composite (pane + axes) at device-pixel resolution. */
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

/** The pixel centroid of `target`-colored pixels (device px), or null when none. */
function color_centroid(png, target, tol = 10) {
  let n = 0;
  let sx = 0;
  let sy = 0;
  for (let y = 0; y < png.height; y += 1) {
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - target[0]) <= tol &&
        Math.abs(png.data[o + 1] - target[1]) <= tol &&
        Math.abs(png.data[o + 2] - target[2]) <= tol
      ) {
        n += 1;
        sx += x;
        sy += y;
      }
    }
  }
  return n === 0 ? null : { x: sx / n, y: sy / n, count: n };
}

function pixel_diff(a, b) {
  expect([a.width, a.height]).toEqual([b.width, b.height]);
  return pixelmatch(a.data, b.data, null, a.width, a.height, { threshold: 0, includeAA: true });
}

/** Crop a (w×h) region centered on device px (cx, cy), clamped into the image. */
function crop_around(png, cx, cy, w, h) {
  const x = Math.max(0, Math.min(Math.round(cx - w / 2), png.width - w));
  const y = Math.max(0, Math.min(Math.round(cy - h / 2), png.height - h));
  const out = new PNG({ width: w, height: h });
  PNG.bitblt(png, out, x, y, w, h, 0, 0);
  return out;
}

/** Count dark-ish pixels (muted placeholder text on the light fixture background). */
function count_dark(png, max_lum = 170) {
  let n = 0;
  for (let o = 0; o < png.data.length; o += 4) {
    if (png.data[o] < max_lum && png.data[o + 1] < max_lum && png.data[o + 2] < max_lum) n += 1;
  }
  return n;
}

/** The visible mid-range logical indexes and prices to anchor drawings deterministically. */
async function anchor_spots(page) {
  return page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const l0 = Math.floor(range.from + (range.to - range.from) * 0.25);
    const l1 = Math.floor(range.from + (range.to - range.from) * 0.7);
    const b0 = window.__main.data_by_index(l0);
    const b1 = window.__main.data_by_index(l1);
    const low = Math.min(b0.low, b1.low);
    const high = Math.max(b0.high, b1.high);
    return {
      l0,
      l1,
      p_lo: low,
      p_hi: high,
      p_mid: (low + high) / 2,
      to_x: (l) => window.__chart.time_scale().logical_to_coordinate(l),
      to_y: (p) => window.__main.price_to_coordinate(p),
    };
  });
}

/** CSS-px pane coordinates of a (logical, price) anchor. */
async function spot(page, logical, price) {
  return page.evaluate(
    ({ logical, price }) => ({
      x: window.__chart.time_scale().logical_to_coordinate(logical),
      y: window.__main.price_to_coordinate(price),
    }),
    { logical, price },
  );
}

async function drawings(page) {
  return page.evaluate(() => window.__chart.drawings().map((d) => ({
    id: d.id, kind: d.kind(), pane_index: d.pane_index(), points: d.points(),
  })));
}

async function overlay_cursor(page) {
  return page.evaluate(() => {
    const canvases = document.querySelectorAll("#chart_container canvas");
    return canvases[canvases.length - 1].style.cursor;
  });
}

/** Focus the input overlay so keyboard gestures reach the recognizer. */
async function focus_overlay(page) {
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
}

/** The EFFECTIVE bar spacing in CSS px (fit-content adjusts it; `options().bar_spacing` is the configured value). */
async function bar_spacing(page) {
  return page.evaluate(() => {
    const x0 = window.__chart.time_scale().logical_to_coordinate(100);
    const x1 = window.__chart.time_scale().logical_to_coordinate(101);
    return x1 - x0;
  });
}

/**
 * CSS-px x for a possibly fractional logical (the public `logical_to_coordinate` is
 * integer-only — reference parity — so interpolate with the effective bar spacing).
 */
async function x_of(page, logical) {
  const spacing = await bar_spacing(page);
  return page.evaluate(
    ({ logical, spacing }) => {
      const base = Math.floor(logical);
      return window.__chart.time_scale().logical_to_coordinate(base) + spacing * (logical - base);
    },
    { logical, spacing },
  );
}

/** A pane point above every visible candle's high: a certain miss for both drawings and series. */
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

test("click-placed drawing tools create through the armed-tool flow", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);

  const cases = [
    { kind: "trend_line", clicks: [[s.l0, s.p_lo], [s.l1, s.p_hi]] },
    { kind: "horizontal_line", clicks: [[s.l0, s.p_mid]] },
    { kind: "horizontal_ray", clicks: [[s.l0, s.p_lo]] },
    { kind: "vertical_line", clicks: [[Math.floor((s.l0 + s.l1) / 2), s.p_mid]] },
    { kind: "rectangle", clicks: [[s.l0, s.p_lo], [s.l1, s.p_hi]] },
    { kind: "long_position", clicks: [[s.l0, s.p_mid]], point_count: 3 },
    { kind: "short_position", clicks: [[s.l0, s.p_mid]], point_count: 3 },
    { kind: "text", clicks: [[s.l1, s.p_hi]] },
  ];
  for (const [index, entry] of cases.entries()) {
    await page.evaluate(({ kind }) => {
      window.__chart.set_drawing_tool(kind, { color: "#2962ff", text: "" });
      if (window.__chart.active_drawing_tool() !== kind) throw new Error(`tool ${kind} not armed`);
    }, { kind: entry.kind });
    for (const [logical, price] of entry.clicks) {
      const p = await spot(page, logical, price);
      await page.mouse.click(p.x, p.y);
    }
    await settle_frames(page);
    const list = await drawings(page);
    expect(list, `after ${entry.kind}`).toHaveLength(index + 1);
    expect(list[index].kind).toBe(entry.kind);
    expect(list[index].points).toHaveLength(entry.point_count ?? entry.clicks.length);
    if (entry.kind === "long_position") {
      expect(list[index].points[1].price).toBeGreaterThan(list[index].points[0].price);
      expect(list[index].points[2].price).toBeLessThan(list[index].points[0].price);
      expect(list[index].points[2].logical).toBeCloseTo(list[index].points[0].logical, 8);
      const reward = Math.abs(list[index].points[1].price - list[index].points[0].price);
      const risk = Math.abs(list[index].points[0].price - list[index].points[2].price);
      expect(reward / risk).toBeCloseTo(2, 8);
    }
    if (entry.kind === "short_position") {
      expect(list[index].points[1].price).toBeLessThan(list[index].points[0].price);
      expect(list[index].points[2].price).toBeGreaterThan(list[index].points[0].price);
      expect(list[index].points[2].logical).toBeCloseTo(list[index].points[0].logical, 8);
      const reward = Math.abs(list[index].points[1].price - list[index].points[0].price);
      const risk = Math.abs(list[index].points[0].price - list[index].points[2].price);
      expect(reward / risk).toBeCloseTo(2, 8);
    }
    for (const point of list[index].points) {
      expect(Number.isFinite(point.logical)).toBe(true);
      expect(Number.isFinite(point.price)).toBe(true);
    }
    // One-shot: the tool disarmed itself after the commit.
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
    const after = await capture(page);
    expect(pixel_diff(clean, after), `${entry.kind} paints`).toBeGreaterThan(20);
  }
});

test("rectangle price chips follow a runtime y-scale precision change", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    const prototype = CanvasRenderingContext2D.prototype;
    const original = prototype.fillText;
    window.__drawing_axis_text = [];
    prototype.fillText = function (text, ...args) {
      window.__drawing_axis_text.push(String(text));
      return original.call(this, text, ...args);
    };
    window.__restore_fill_text = () => {
      prototype.fillText = original;
    };
    window.__chart.add_drawing("rectangle", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ], { show_labels: true });
  }, s);
  await settle_frames(page);

  await page.evaluate(() => {
    window.__drawing_axis_text.length = 0;
    window.__main.apply_options({
      price_format: { type: "price", precision: 4, min_move: 0.0001 },
    });
  });
  await settle_frames(page);

  const text = await page.evaluate(() => {
    const captured = [...window.__drawing_axis_text];
    window.__restore_fill_text();
    return captured;
  });
  expect(text).toContain(s.p_lo.toFixed(4));
  expect(text).toContain(s.p_hi.toFixed(4));
});

test("two-anchor creation previews the pending drawing; Escape cancels it", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const clean = await capture(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("rectangle", { color: "#2962ff" }));

  const first = await spot(page, s.l0, s.p_lo);
  await page.mouse.click(first.x, first.y);
  expect(await drawings(page)).toHaveLength(0);
  // The preview follows the mouse before the second click.
  const hover = await spot(page, s.l1, s.p_hi);
  await page.mouse.move(hover.x, hover.y);
  await settle_frames(page);
  const preview = await capture(page);
  expect(pixel_diff(clean, preview), "preview paints before commit").toBeGreaterThan(20);

  // Escape abandons the pending drawing and disarms the tool.
  await focus_overlay(page);
  await page.keyboard.press("Escape");
  await settle_frames(page);
  expect(pixel_diff(clean, await capture(page)), "cancel restores the clean frame").toBe(0);
  expect(await drawings(page)).toHaveLength(0);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

  // A click afterwards places nothing (the tool is no longer armed).
  await page.mouse.click(first.x, first.y);
  expect(await drawings(page)).toHaveLength(0);
});

test("path clicks add vertices, Backspace pops, and Enter or double-click finishes", async ({ page }) => {
  for (const backend of ["canvas2d", "auto"]) {
    await goto_fixture(page, backend);
    expect(await page.evaluate(() => window.__chart.backend())).toBe(backend === "auto" ? "webgpu" : backend);
    const s = await anchor_spots(page);
    const first = await spot(page, s.l0, s.p_lo);
    const second = await spot(page, Math.floor((s.l0 + s.l1) / 2), s.p_hi);
    const removed = await spot(page, s.l1, s.p_mid);
    const replacement = await spot(page, s.l1, s.p_lo);

    await page.evaluate(() => window.__chart.set_drawing_tool("path", { color: "#2962ff" }));
    await page.mouse.click(first.x, first.y);
    await page.mouse.click(second.x, second.y);
    await page.mouse.click(removed.x, removed.y);
    expect(await drawings(page)).toHaveLength(0);
    await focus_overlay(page);
    await page.keyboard.press("Backspace");
    await page.mouse.click(replacement.x, replacement.y);
    await page.keyboard.press("Enter");

    let list = await drawings(page);
    expect(list).toHaveLength(1);
    expect(list[0].kind).toBe("path");
    expect(list[0].points).toHaveLength(3);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

    // Probe one open chevron wing. This verifies that each real executor paints the arrowhead and
    // that the same visible geometry is a body-movement target. Click-to-logical rounding
    // can shift the committed tip ~1 CSS px from the click spot, so the probe searches a
    // small device-px neighborhood instead of a single pixel.
    const dx = replacement.x - second.x;
    const dy = replacement.y - second.y;
    const distance = Math.hypot(dx, dy);
    const direction = { x: dx / distance, y: dy / distance };
    const angle = 40 * Math.PI / 180;
    const wing_length = 16;
    const wing_end = {
      x: replacement.x - direction.x * wing_length * Math.cos(angle) - direction.y * wing_length * Math.sin(angle),
      y: replacement.y - direction.y * wing_length * Math.cos(angle) + direction.x * wing_length * Math.sin(angle),
    };
    const arrow_wing = {
      x: (replacement.x + wing_end.x) / 2,
      y: (replacement.y + wing_end.y) / 2,
    };
    const painted = await capture(page);
    const dist_to_blue = (css_x, css_y) => {
      const cx = Math.round(css_x * PR);
      const cy = Math.round(css_y * PR);
      let best = Infinity;
      for (let oy = -6; oy <= 6; oy++) {
        for (let ox = -6; ox <= 6; ox++) {
          const x = cx + ox;
          const y = cy + oy;
          if (x < 0 || y < 0 || x >= painted.width || y >= painted.height) continue;
          const o = (y * painted.width + x) * 4;
          const d = Math.max(
            Math.abs(painted.data[o] - 41),
            Math.abs(painted.data[o + 1] - 98),
            Math.abs(painted.data[o + 2] - 255),
          );
          if (d < best) best = d;
        }
      }
      return best;
    };
    expect(
      dist_to_blue(arrow_wing.x, arrow_wing.y),
      `${backend} paints the path arrowhead`,
    ).toBeLessThanOrEqual(64);
    const open_gap = {
      x: replacement.x - direction.x * 8 - direction.y * 3,
      y: replacement.y - direction.y * 8 + direction.x * 3,
    };
    const gap_offset = (Math.round(open_gap.y * PR) * painted.width + Math.round(open_gap.x * PR)) * 4;
    const gap_pixel = [painted.data[gap_offset], painted.data[gap_offset + 1], painted.data[gap_offset + 2]];
    expect(
      Math.max(...gap_pixel.map((channel, index) => Math.abs(channel - [41, 98, 255][index]))),
      `${backend} keeps the path arrowhead open`,
    ).toBeGreaterThan(64);
    await page.mouse.click(arrow_wing.x, arrow_wing.y);
    expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(list[0].id);

    await page.evaluate(() => window.__chart.set_drawing_tool("path", { color: "#e91e63" }));
    await page.mouse.click(first.x, first.y);
    await page.mouse.dblclick(replacement.x, replacement.y);
    list = await drawings(page);
    expect(list).toHaveLength(2);
    expect(list[1].kind).toBe("path");
    expect(list[1].points).toHaveLength(2);
    expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
  }
});

test("click selects with anchor handles and part cursors; empty click deselects", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({
    crosshair: {
      vertLine: { color: "#ff00ff", labelBackgroundColor: "#ff00ff" },
      horzLine: { color: "#ff00ff", labelBackgroundColor: "#ff00ff" },
    },
  }));
  const s = await anchor_spots(page);
  // Black stroke: the blue handle discs are unambiguous against it.
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ], { color: "#000000" });
  }, s);
  await settle_frames(page);
  const clean = await capture(page);
  expect(count_color(clean, BLUE), "no handles while unselected").toBe(0);
  const empty = await empty_spot(page);
  await page.mouse.move(empty.x, empty.y);
  await settle_frames(page);
  expect(count_color(await capture(page), [255, 0, 255]), "ordinary pane hover paints the crosshair")
    .toBeGreaterThan(100);

  // Hovering the body away from the dedicated middle-label slot reports the drawing with the
  // move cursor. The exact midpoint belongs to the empty trend-label prompt.
  const body = await page.evaluate(({ l0, l1, p_lo, p_hi }) => ({
    x: window.__chart.time_scale().logical_to_coordinate(l0) * 0.75 + window.__chart.time_scale().logical_to_coordinate(l1) * 0.25,
    y: window.__main.price_to_coordinate(p_lo) * 0.75 + window.__main.price_to_coordinate(p_hi) * 0.25,
  }), s);
  await page.evaluate(() => {
    window.__hits = [];
    window.__chart.subscribe_crosshair_move((p) => window.__hits.push(p.hovered_object_id));
  });
  await page.mouse.move(body.x, body.y);
  const id = (await drawings(page))[0].id;
  expect(await page.evaluate(() => window.__hits[window.__hits.length - 1])).toBe(`drawing:${id}`);
  expect(await overlay_cursor(page)).toBe("move");
  await settle_frames(page);
  expect(count_color(await capture(page), [255, 0, 255]), "drawing hover suppresses crosshair chrome").toBe(0);

  // Click selects: anchor handles (blue discs) appear at both defining points.
  await page.mouse.click(body.x, body.y);
  await settle_frames(page);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(id);
  const selected = await capture(page);
  expect(count_color(selected, BLUE), "anchor handles after selection").toBeGreaterThan(50);

  // Hovering an anchor handle switches to the pointer cursor.
  const anchor = await spot(page, s.l0, s.p_lo);
  await page.mouse.move(anchor.x, anchor.y);
  expect(await overlay_cursor(page)).toBe("pointer");

  // Clicking empty pane space deselects (handles gone; nothing else selects).
  await page.mouse.click(empty.x, empty.y);
  await settle_frames(page);
  expect(await page.evaluate(() => window.__chart.selected_drawing())).toBeNull();
  expect(count_color(await capture(page), BLUE), "handles gone after deselect").toBe(0);
});

test("anchor drag re-anchors one point; body drag moves the whole drawing", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ]);
  }, s);
  await settle_frames(page);
  const before = (await drawings(page))[0];

  // Select with a body click, then drag anchor 0 by a known pixel delta.
  const mid = await page.evaluate(({ l0, l1, p_lo, p_hi }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(l0) + window.__chart.time_scale().logical_to_coordinate(l1)) / 2,
    y: (window.__main.price_to_coordinate(p_lo) + window.__main.price_to_coordinate(p_hi)) / 2,
  }), s);
  await page.mouse.click(mid.x, mid.y);
  const anchor = await spot(page, s.l0, s.p_lo);
  const spacing = await bar_spacing(page);
  const drop = { x: anchor.x + 4 * spacing, y: anchor.y - 40 };
  const expected_price = await page.evaluate((y) => window.__main.coordinate_to_price(y), drop.y);
  await page.mouse.move(anchor.x, anchor.y);
  await page.mouse.down();
  await page.mouse.move(drop.x, drop.y, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);

  const reanchored = (await drawings(page))[0];
  expect(reanchored.points[0].logical).toBeCloseTo(before.points[0].logical + 4, 1);
  expect(reanchored.points[0].price).toBeCloseTo(expected_price, 6);
  // Anchor 1 untouched.
  expect(reanchored.points[1].logical).toBeCloseTo(before.points[1].logical, 6);
  expect(reanchored.points[1].price).toBeCloseTo(before.points[1].price, 6);

  // Body drag: grab the segment's middle, move it — both anchors translate together.
  const seg = {
    x: (await x_of(page, reanchored.points[0].logical) + await x_of(page, reanchored.points[1].logical)) / 2,
    y: await page.evaluate(({ points }) => (
      window.__main.price_to_coordinate(points[0].price) +
      window.__main.price_to_coordinate(points[1].price)
    ) / 2, { points: reanchored.points }),
  };
  const body_drop = { x: seg.x - 2 * spacing, y: seg.y + 24 };
  const expected_body_price0 = await page.evaluate(
    ({ points, dy }) => window.__main.coordinate_to_price(window.__main.price_to_coordinate(points[0].price) + dy),
    { points: reanchored.points, dy: 24 },
  );
  await page.mouse.move(seg.x, seg.y);
  await page.mouse.down();
  await page.mouse.move(body_drop.x, body_drop.y, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);

  const moved = (await drawings(page))[0];
  expect(moved.points[0].logical).toBeCloseTo(reanchored.points[0].logical - 2, 1);
  expect(moved.points[1].logical).toBeCloseTo(reanchored.points[1].logical - 2, 1);
  expect(moved.points[0].price).toBeCloseTo(expected_body_price0, 6);
  // The shape (anchor spacing) is preserved exactly by the coordinate-space translation.
  expect(moved.points[1].logical - moved.points[0].logical)
    .toBeCloseTo(reanchored.points[1].logical - reanchored.points[0].logical, 6);
  // Each multi-sample pointer drag is one semantic history entry.
  await page.evaluate(() => window.__chart.undo_drawing());
  expect((await drawings(page))[0].points).toEqual(reanchored.points);
  await page.evaluate(() => window.__chart.undo_drawing());
  expect((await drawings(page))[0].points).toEqual(before.points);
  // The drawing stayed selected through the drags.
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(moved.id);
});

test("full-span kinds freeze the unused axis in body drags", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid, l1 }) => {
    window.__chart.add_drawing("horizontal_line", [{ logical: l0, price: p_mid }]);
    window.__chart.add_drawing("vertical_line", [{ logical: l1, price: 0 }]);
  }, s);
  await settle_frames(page);
  const [hline, vline] = await drawings(page);

  // Diagonal drag on the horizontal line: only the price changes.
  const h = await spot(page, s.l0, s.p_mid);
  await page.mouse.move(h.x, h.y);
  await page.mouse.down();
  await page.mouse.move(h.x + 60, h.y - 30, { steps: 4 });
  await page.mouse.up();
  const [hline_after] = await drawings(page);
  expect(hline_after.points[0].logical).toBeCloseTo(hline.points[0].logical, 6);
  expect(hline_after.points[0].price).not.toBeCloseTo(hline.points[0].price, 3);

  // Diagonal drag on the vertical line: only the logical changes.
  const v = await spot(page, s.l1, s.p_mid);
  await page.mouse.move(v.x, v.y);
  await page.mouse.down();
  await page.mouse.move(v.x + 40, v.y - 30, { steps: 4 });
  await page.mouse.up();
  const list = await drawings(page);
  const vline_after = list.find((d) => d.id === vline.id);
  expect(vline_after.points[0].price).toBeCloseTo(vline.points[0].price, 6);
  expect(vline_after.points[0].logical).not.toBeCloseTo(vline.points[0].logical, 3);
});

test("the 3×3 text alignment places labels around the tool's anchor", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  // No crosshair pixels near the probes.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const anchor_logical = Math.floor((s.l0 + s.l1) / 2);
  const create = (h, v) => page.evaluate(
    ({ logical, price, h, v }) => window.__chart.add_drawing("text", [{ logical, price }], {
      text: "aligned", text_color: "#7b1fa2", text_size: 20, text_h_align: h, text_v_align: v,
    }).id,
    { logical: anchor_logical, price: s.p_mid, h, v },
  );

  const anchor = await spot(page, anchor_logical, s.p_mid);
  const ax = anchor.x * PR;
  const ay = anchor.y * PR;

  const h_id = await create("center", "middle");
  await settle_frames(page);
  const centered = color_centroid(await capture(page), PURPLE);
  expect(centered, "label paints").not.toBeNull();
  expect(Math.abs(centered.x - ax), "center-aligned centroid on the anchor x").toBeLessThan(3 * PR);
  expect(Math.abs(centered.y - ay), "middle-aligned centroid on the anchor y").toBeLessThan(4 * PR);

  await page.evaluate(({ id, h }) => {
    const d = window.__chart.drawings().find((x) => x.id === id);
    d.apply_options({ text_h_align: h });
  }, { id: h_id, h: "left" });
  await settle_frames(page);
  const left = color_centroid(await capture(page), PURPLE);
  expect(left.x, "left-aligned label extends right of the anchor").toBeGreaterThan(ax + 8 * PR);

  await page.evaluate(({ id, h }) => {
    const d = window.__chart.drawings().find((x) => x.id === id);
    d.apply_options({ text_h_align: h });
  }, { id: h_id, h: "right" });
  await settle_frames(page);
  const right = color_centroid(await capture(page), PURPLE);
  expect(right.x, "right-aligned label extends left of the anchor").toBeLessThan(ax - 8 * PR);

  await page.evaluate(({ id, h, v }) => {
    const d = window.__chart.drawings().find((x) => x.id === id);
    d.apply_options({ text_h_align: h, text_v_align: v });
  }, { id: h_id, h: "center", v: "top" });
  await settle_frames(page);
  const top = color_centroid(await capture(page), PURPLE);
  expect(top.y, "top-aligned label sits above the anchor").toBeLessThan(ay - 8 * PR);

  await page.evaluate(({ id, v }) => {
    const d = window.__chart.drawings().find((x) => x.id === id);
    d.apply_options({ text_v_align: v });
  }, { id: h_id, v: "bottom" });
  await settle_frames(page);
  const bottom = color_centroid(await capture(page), PURPLE);
  expect(bottom.y, "bottom-aligned label sits below the anchor").toBeGreaterThan(ay + 8 * PR);

  // The options round-trip reports the applied alignment.
  const options = await page.evaluate(({ id }) => window.__chart.drawings().find((x) => x.id === id).options(), { id: h_id });
  expect(options.text_h_align).toBe("center");
  expect(options.text_v_align).toBe("bottom");
});

test("a trend line carries an aligned text label", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ], { text: "trend label", text_color: "#7b1fa2", text_v_align: "top" });
  }, s);
  await settle_frames(page);
  const with_label = color_centroid(await capture(page), PURPLE);
  expect(with_label, "label paints above the segment").not.toBeNull();
  const line_mid = await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    const ys = [window.__main.price_to_coordinate(p_lo), window.__main.price_to_coordinate(p_hi)];
    return (ys[0] + ys[1]) / 2;
  }, s);
  // Top-center is local to the actual segment, not its axis-aligned bounding box: the run sits
  // above the line where the horizontal center slot intersects it.
  expect(with_label.y, "above the segment at its center slot").toBeLessThan(line_mid * PR - 8);
});

test("an empty trend line offers direct inline text entry at its configured slot", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const id = await page.evaluate(({ l0, l1, p_lo }) => window.__chart.add_drawing("trend_line", [
    { logical: l0, price: p_lo },
    { logical: l1, price: p_lo },
  ], {
    color: "#f59e0b",
    text_color: "#7b1fa2",
    text_size: 14,
    text_h_align: "center",
    text_v_align: "middle",
  }).id, s);
  const slot = {
    x: (await x_of(page, s.l0) + await x_of(page, s.l1)) / 2,
    y: await page.evaluate((price) => window.__main.price_to_coordinate(price), s.p_lo),
  };

  const idle = await capture(page);
  await page.mouse.move(slot.x, slot.y);
  await settle_frames(page);
  const hovered = await capture(page);
  expect(pixel_diff(idle, hovered), "hover paints the low-opacity Add text affordance").toBeGreaterThan(20);

  const gap_width = (png) => {
    const center = slot.x * PR;
    const candidates = [];
    for (let y = Math.round(slot.y * PR) - 2; y <= Math.round(slot.y * PR) + 2; y += 1) {
      let left = -1;
      let right = png.width;
      for (let x = 0; x < png.width; x += 1) {
        const o = (y * png.width + x) * 4;
        const orange = Math.abs(png.data[o] - 245) < 35 && Math.abs(png.data[o + 1] - 158) < 35 && png.data[o + 2] < 55;
        if (!orange) continue;
        if (x < center) left = Math.max(left, x);
        else right = Math.min(right, x);
      }
      if (left >= 0 && right < png.width) candidates.push(right - left - 1);
    }
    return Math.min(...candidates);
  };
  const prompt_gap = gap_width(hovered);

  await page.mouse.click(slot.x, slot.y);
  const editor = page.locator("#aeris_charts-text-input");
  await expect(editor).toBeVisible();
  await settle_frames(page);
  const caret_gap = gap_width(await capture(page));
  expect(caret_gap, "empty editing uses a compact one-character gap").toBeLessThan(50);
  expect(caret_gap, "clicking the prompt releases its full-width opening").toBeLessThan(prompt_gap);
  await page.keyboard.type("Dev");
  await expect.poll(() => page.evaluate(({ id }) => (
    window.__chart.drawings().find((drawing) => drawing.id === id)?.options().text
  ), { id })).toBe("Dev");
  await settle_frames(page);
  expect(gap_width(await capture(page)), "the line gap expands with measured typed text").toBeGreaterThan(caret_gap);
  const editor_box = await editor.boundingBox();
  expect(editor_box).not.toBeNull();
  expect(Math.abs((editor_box.x + editor_box.width / 2) - slot.x)).toBeLessThan(3);
  expect(Math.abs((editor_box.y + editor_box.height / 2) - slot.y)).toBeLessThan(3);

  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(({ id }) => (
    window.__chart.drawings().find((drawing) => drawing.id === id)?.options().text
  ), { id })).toBe("Dev");
});

test("Delete and Backspace remove the selected drawing", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const add = () => page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ]);
  }, s);
  const body = async () => {
    const points = (await drawings(page))[0].points;
    return {
      x: await x_of(page, points[0].logical) * 0.75 + await x_of(page, points[1].logical) * 0.25,
      y: await page.evaluate(({ points }) => (
        window.__main.price_to_coordinate(points[0].price) * 0.75 +
        window.__main.price_to_coordinate(points[1].price) * 0.25
      ), { points }),
    };
  };
  await focus_overlay(page);

  await add();
  let grab = await body();
  await page.mouse.click(grab.x, grab.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing())).not.toBeNull();
  await page.keyboard.press("Delete");
  expect(await drawings(page)).toHaveLength(0);

  // Backspace works the same way; with nothing selected both keys are no-ops.
  await add();
  grab = await body();
  await page.mouse.click(grab.x, grab.y);
  await page.keyboard.press("Backspace");
  expect(await drawings(page)).toHaveLength(0);
  await page.keyboard.press("Delete");
  await page.keyboard.press("Backspace");
  expect(await drawings(page)).toHaveLength(0);
});

test("public drawing history reverses create, delete, anchors, and style with conventional shortcuts", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  const initial = [
    { logical: s.l0, price: s.p_lo },
    { logical: s.l1, price: s.p_hi },
  ];
  await page.evaluate(({ initial }) => {
    window.__chart.add_drawing("trend_line", initial, { color: "#112233", width: 2 });
  }, { initial });
  await focus_overlay(page);
  expect(await page.evaluate(() => window.__chart.can_undo_drawing())).toBe(true);

  await page.keyboard.press("Control+z");
  expect(await drawings(page)).toHaveLength(0);
  await page.keyboard.press("Control+Shift+z");
  // points() also reports each anchor's `time` identity; the history must restore the exact
  // logical/price anchors, so match those fields and require the time to be present.
  expect((await drawings(page))[0].points).toMatchObject(initial);
  expect((await drawings(page))[0].points.every((point) => Number.isFinite(point.time))).toBe(true);

  await page.evaluate(() => window.__chart.drawings()[0].apply_options({ color: "#ff00ff", width: 5 }));
  expect((await page.evaluate(() => window.__chart.drawings()[0].options())).width).toBe(5);
  await page.evaluate(() => window.__chart.undo_drawing());
  expect(await page.evaluate(() => window.__chart.drawings()[0].options())).toMatchObject({ color: "#112233", width: 2 });
  await page.evaluate(() => window.__chart.redo_drawing());
  expect(await page.evaluate(() => window.__chart.drawings()[0].options())).toMatchObject({ color: "#ff00ff", width: 5 });

  const moved = initial.map((point) => ({ logical: point.logical + 2, price: point.price + 1 }));
  await page.evaluate(({ moved }) => window.__chart.drawings()[0].set_points(moved), { moved });
  await page.evaluate(() => window.__chart.undo_drawing());
  expect((await drawings(page))[0].points).toMatchObject(initial);
  await page.evaluate(() => window.__chart.redo_drawing());
  expect((await drawings(page))[0].points).toMatchObject(moved);

  await page.evaluate(() => window.__chart.drawings()[0].remove());
  expect(await drawings(page)).toHaveLength(0);
  await page.evaluate(() => window.__chart.undo_drawing());
  expect((await drawings(page))[0].points).toMatchObject(moved);
  await page.evaluate(() => {
    window.__chart.drawings()[0].apply_options({ width: 3 });
  });
  expect(await page.evaluate(() => window.__chart.can_redo_drawing())).toBe(false);
});

test("drawing tools render pixel-identical on WebGPU and Canvas2D (AA coverage steps aside)", async ({ page }, test_info) => {
  const run_scenario = async (backend) => {
    await goto_fixture(page, backend);
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await page.evaluate(() => {
      const chart = window.__chart;
      const range = chart.time_scale().get_visible_logical_range();
      const l0 = Math.floor(range.from + (range.to - range.from) * 0.2);
      const l1 = Math.floor(range.from + (range.to - range.from) * 0.75);
      const b0 = window.__main.data_by_index(l0);
      const b1 = window.__main.data_by_index(l1);
      const lo = Math.min(b0.low, b1.low);
      const hi = Math.max(b0.high, b1.high);
      chart.add_drawing("trend_line", [
        { logical: l0, price: lo },
        { logical: l1, price: hi },
      ], { text: "trend", text_h_align: "center", text_v_align: "top", width: 3 });
      chart.add_drawing("horizontal_line", [{ logical: l0, price: (lo + hi) / 2 }], {
        style: "dashed", text: "h-line", text_h_align: "left",
      });
      chart.add_drawing("horizontal_ray", [{ logical: Math.floor((l0 + l1) / 2), price: lo - 1 }], { width: 2 });
      chart.add_drawing("vertical_line", [{ logical: Math.floor((l0 + l1) / 2), price: 0 }], { style: "dotted" });
      chart.add_drawing("rectangle", [
        { logical: l0 + 2, price: lo },
        { logical: l0 + 14, price: lo + (hi - lo) / 2 },
      ], { color: "#e91e63" });
      chart.add_drawing("text", [{ logical: l1 - 4, price: hi + 0.5 }], {
        text: "parity label", text_size: 16, text_bold: true, text_h_align: "right", text_v_align: "bottom",
        text_color: "#7b1fa2",
      });
      // A boxed text (container background + border) and an empty text (muted placeholder).
      chart.add_drawing("text", [{ logical: l0 + 8, price: hi }], {
        text: "boxed", box_color: "rgba(41, 98, 255, 0.85)", box_border_color: "#e91e63",
        box_border_width: 2, text_color: "#ffffff",
      });
      // A styled label (heavy weight + italic) — its own atlas/font-spec path.
      chart.add_drawing("text", [{ logical: Math.floor((l0 + l1) / 2), price: lo - 2 }], {
        text: "styled 800 italic", text_weight: 800, text_italic: true, text_color: "#e91e63",
      });
      // A smooth brush stroke (curved polyline through the captured path).
      chart.add_drawing("brush", [
        { logical: l0 + 4, price: lo - 0.5 },
        { logical: Math.floor((l0 + l1) / 2), price: hi + 0.8 },
        { logical: l1 - 2, price: lo - 0.5 },
      ], { color: "#7b1fa2", width: 3 });
      chart.add_drawing("path", [
        { logical: l0 + 1, price: hi + 0.2 },
        { logical: Math.floor((l0 + l1) / 2), price: lo - 0.8 },
        { logical: l1 - 1, price: hi + 0.2 },
      ], { color: "#089981", width: 2 });
      chart.add_drawing("long_position", [
        { logical: l0 + 3, price: (lo + hi) / 2 },
        { logical: l0 + 12, price: hi + 0.6 },
        { logical: l0 + 12, price: lo - 0.6 },
      ]);
      chart.add_drawing("short_position", [
        { logical: l1 - 14, price: (lo + hi) / 2 },
        { logical: l1 - 4, price: lo - 0.9 },
        { logical: l1 - 4, price: hi + 0.9 },
      ]);
      // One interactive creation through the click flow shares the engine path too.
      chart.set_drawing_tool("trend_line", { color: "#089981" });
    });
    const s = await anchor_spots(page);
    const p1 = await spot(page, s.l0, s.p_mid);
    const p2 = await spot(page, s.l1, s.p_lo);
    await page.mouse.click(p1.x, p1.y);
    await page.mouse.click(p2.x, p2.y);
    // Anchor handles: select the first drawing.
    await page.evaluate(() => {
      const first = window.__chart.drawings()[0];
      window.__chart.wasm.set_selected_drawing(first.id);
      window.__chart.render();
    });
    await settle_frames(page);
    const trend_label = await page.evaluate(() => {
      const first = window.__chart.drawings()[0];
      const [x, y, angle] = window.__chart.wasm.drawing_text_transform(first.id);
      const options = first.options();
      const layout = window.__chart.options().layout;
      const size = options.text_size ?? layout.fontSize;
      const context = document.createElement("canvas").getContext("2d");
      context.font = `${options.text_italic ? "italic " : ""}${options.text_weight ?? 400} ${size}px ${layout.fontFamily}`;
      return { x, y, angle, size, width: context.measureText(options.text).width };
    });
    return {
      backend: await page.evaluate(() => window.__chart.backend()),
      png: PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false })),
      trend_label,
    };
  };

  const canvas = await run_scenario("canvas2d");
  expect(canvas.backend).toBe("canvas2d");
  const gpu = await run_scenario("auto");
  expect(gpu.backend).toBe("webgpu");
  expect([canvas.png.width, canvas.png.height]).toEqual([gpu.png.width, gpu.png.height]);
  expect(gpu.trend_label.x).toBeCloseTo(canvas.trend_label.x, 6);
  expect(gpu.trend_label.y).toBeCloseTo(canvas.trend_label.y, 6);
  expect(gpu.trend_label.angle).toBeCloseTo(canvas.trend_label.angle, 6);

  // Vacuousness guard: the scenario paints substantially on EACH backend.
  const clean_probe = await (async () => {
    await goto_fixture(page, "canvas2d");
    await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
    await settle_frames(page);
    return PNG.sync.read(await page.screenshot({ animations: "disabled", fullPage: false }));
  })();
  expect(pixel_diff(clean_probe, canvas.png), "drawings paint on Canvas2D").toBeGreaterThan(1000);
  expect(pixel_diff(clean_probe, gpu.png), "drawings paint on WebGPU").toBeGreaterThan(1000);

  // The repo's ordering contract (backend-parity.spec.mjs markers gate): zero pixels may differ
  // by more than an AA coverage step. Measurements across local and CI SwiftShader put isolated
  // diagonal-stroke coverage deltas at up to 121; paint-order swaps remain far above this band
  // (the regression fixture measured 201), so 128 separates raster coverage from wrong paint.
  // Rotated browser text is the one intentional exception: Canvas2D rotates during fillText,
  // while WebGPU rotates the cached unrotated glyph-atlas quad. Verify the canonical transform
  // exactly above, then bound high-coverage differences to that measured glyph rectangle only.
  let ordering_diff = 0;
  let rotated_glyph_diff = 0;
  let edge_diff = 0;
  let maximum_channel_delta = 0;
  for (let offset = 0; offset < canvas.png.data.length; offset += 4) {
    let pixel_delta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      pixel_delta = Math.max(pixel_delta, Math.abs(canvas.png.data[offset + channel] - gpu.png.data[offset + channel]));
    }
    maximum_channel_delta = Math.max(maximum_channel_delta, pixel_delta);
    if (pixel_delta > 128) {
      const pixel = offset / 4;
      const dx = (pixel % canvas.png.width) / PR - canvas.trend_label.x;
      const dy = Math.floor(pixel / canvas.png.width) / PR - canvas.trend_label.y;
      const cos = Math.cos(canvas.trend_label.angle);
      const sin = Math.sin(canvas.trend_label.angle);
      const local_x = dx * cos + dy * sin;
      const local_y = -dx * sin + dy * cos;
      const in_rotated_glyph =
        Math.abs(local_x) <= canvas.trend_label.width / 2 + 3 &&
        Math.abs(local_y) <= canvas.trend_label.size / 2 + 3;
      if (in_rotated_glyph) rotated_glyph_diff += 1;
      else ordering_diff += 1;
    }
    else if (pixel_delta !== 0) edge_diff += 1;
  }
  console.log(`drawings parity: ${edge_diff} AA-edge pixels (max step ${maximum_channel_delta}), ${rotated_glyph_diff} rotated-glyph pixels, ${ordering_diff} ordering pixels`);
  if (ordering_diff !== 0) {
    const visual = new PNG({ width: canvas.png.width, height: canvas.png.height });
    pixelmatch(canvas.png.data, gpu.png.data, visual.data, canvas.png.width, canvas.png.height, { threshold: 0, includeAA: true });
    await test_info.attach("canvas2d.png", { body: PNG.sync.write(canvas.png), contentType: "image/png" });
    await test_info.attach("webgpu.png", { body: PNG.sync.write(gpu.png), contentType: "image/png" });
    await test_info.attach("diff.png", { body: PNG.sync.write(visual), contentType: "image/png" });
  }
  expect(rotated_glyph_diff, "rotated glyph raster residual stays bounded").toBeLessThanOrEqual(128);
  expect(ordering_diff, "drawing geometry/paint order must match (only AA coverage steps may differ)").toBe(0);
});

test("the demo toolbar arms tools, creates, and clears all", async ({ page }) => {  // The full demo (toolbar visible, grid-native chart) — the drawing group drives the same API.
  await page.goto("/");
  await wait_for_chart(page);
  await page.waitForFunction(() => performance.now() > 600);
  const offset = await page.evaluate(() => {
    const r = document.getElementById("chart_container").getBoundingClientRect();
    return { left: r.left, top: r.top };
  });
  const spots = await anchor_spots(page);
  const at = async (logical, price) => {
    const p = await spot(page, logical, price);
    return { x: p.x + offset.left, y: p.y + offset.top };
  };

  await page.click("#drawings_group [data-tool='trend_line']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe("trend_line");
  const armed = await page.evaluate(() => window.__chart.active_drawing_tool());
  expect(armed).toBe("trend_line");
  const a = await at(spots.l0, spots.p_lo);
  const b = await at(spots.l1, spots.p_hi);
  await page.mouse.click(a.x, a.y);
  await page.mouse.click(b.x, b.y);
  expect(await drawings(page)).toHaveLength(1);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

  // Clicking the button again without placing toggles the tool off.
  await page.click("#drawings_group [data-tool='horizontal_line']");
  await page.click("#drawings_group [data-tool='horizontal_line']");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

  // The label/color inputs template the next drawing; clear all empties the store.
  await page.fill("#drawing_text", "toolbar note");
  await page.click("#drawings_group [data-tool='text']");
  const t = await at(spots.l1, spots.p_mid);
  await page.mouse.click(t.x, t.y);
  const list = await drawings(page);
  expect(list).toHaveLength(2);
  const text_options = await page.evaluate(() => window.__chart.drawings()[1].options());
  expect(text_options.text).toBe("toolbar note");
  await page.click("#clear_drawings");
  expect(await drawings(page)).toHaveLength(0);
});

test("Ctrl magnet snaps placement to the nearest bar's OHLC", async ({ page }) => {
  await goto_fixture(page);
  // Widen bars so the 70%-vs-50% slot distinction is several device px, not sub-pixel:
  // the default 1000-bar fit leaves ~1.2px spacing where any click rounds to both bars.
  await page.evaluate(() => {
    window.__chart.time_scale().set_visible_logical_range({ from: 350, to: 400 });
  });
  await settle_frames(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("trend_line"));
  // Click between two bars, nearer to one bar's close, with Ctrl held: the anchor snaps to
  // that bar's center and to the closest of its open/high/low/close.
  const probe = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor(range.from + (range.to - range.from) * 0.4);
    const bar = window.__main.data_by_index(index + 1);
    // 70% of the way from bar `index` to bar `index + 1`: clearly inside the +1 bar's slot.
    const x = window.__chart.time_scale().logical_to_coordinate(index) +
      (window.__chart.time_scale().logical_to_coordinate(index + 1) -
        window.__chart.time_scale().logical_to_coordinate(index)) * 0.7;
    // Aim just off the close so the snap target is unambiguous among the four prices.
    const y = window.__main.price_to_coordinate(bar.close) + 3;
    return { index, x, y, prices: [bar.open, bar.high, bar.low, bar.close] };
  });
  await page.keyboard.down(MAGNET_CLICK_KEY);
  await page.mouse.click(probe.x, probe.y);
  await page.keyboard.up(MAGNET_CLICK_KEY);
  // Second click WITHOUT Ctrl: stays raw (fractional logical).
  const free = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor(range.from + (range.to - range.from) * 0.75);
    const bar = window.__main.data_by_index(index);
    return {
      x: (window.__chart.time_scale().logical_to_coordinate(index) +
        window.__chart.time_scale().logical_to_coordinate(index + 1)) / 2,
      y: window.__main.price_to_coordinate(bar.close) + 3,
    };
  });
  await page.mouse.click(free.x, free.y);
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  const [first, second] = list[0].points;
  expect(first.logical).toBeCloseTo(probe.index + 1, 6);
  expect(probe.prices).toContainEqual(first.price);
  expect(Number.isInteger(second.logical)).toBe(false);
  expect(probe.prices).not.toContainEqual(second.price);
});

test("Ctrl magnet ignores derived indicator lines", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__magnet_sma = window.__chart.add_sma(window.__main, 20);
    window.__chart.time_scale().set_visible_logical_range({ from: 350, to: 400 });
    window.__chart.set_drawing_tool("horizontal_line");
  });
  await settle_frames(page);

  const probe = await page.evaluate(() => {
    for (let index = 350; index <= 400; index += 1) {
      const indicator = window.__magnet_sma.data_by_index(index);
      const bar = window.__main.data_by_index(index);
      if (!indicator || !bar) continue;
      const prices = [bar.open, bar.high, bar.low, bar.close];
      if (prices.every((price) => Math.abs(price - indicator.value) > 1e-6)) {
        return {
          index,
          indicator: indicator.value,
          prices,
          x: window.__chart.time_scale().logical_to_coordinate(index),
          y: window.__main.price_to_coordinate(indicator.value),
        };
      }
    }
    return null;
  });
  expect(probe).not.toBeNull();

  await page.keyboard.down(MAGNET_CLICK_KEY);
  await page.mouse.click(probe.x, probe.y);
  await page.keyboard.up(MAGNET_CLICK_KEY);

  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect(list[0].points[0].logical).toBeCloseTo(probe.index, 6);
  expect(probe.prices).toContainEqual(list[0].points[0].price);
  expect(list[0].points[0].price).not.toBeCloseTo(probe.indicator, 9);
});

test("Ctrl magnet ignores hidden OHLC fields after switching to an area series", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => {
    window.__main.set_type("area");
    window.__chart.time_scale().fit_content();
    // Widen bars for the same sub-pixel reason as the placement test above.
    window.__chart.time_scale().set_visible_logical_range({ from: 350, to: 400 });
    window.__chart.set_drawing_tool("horizontal_line");
  });
  await settle_frames(page);
  const probe = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const index = Math.floor(range.from + (range.to - range.from) * 0.45);
    const bar = window.__data[index];
    return {
      index,
      close: bar.close,
      hidden_high: bar.high,
      x: window.__chart.time_scale().logical_to_coordinate(index),
      y: window.__main.price_to_coordinate(bar.high),
    };
  });
  expect(probe.hidden_high).not.toBe(probe.close);
  await settle_frames(page);

  await page.keyboard.down(MAGNET_CLICK_KEY);
  await page.mouse.click(probe.x, probe.y);
  await page.keyboard.up(MAGNET_CLICK_KEY);

  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect(list[0].points[0].logical).toBeCloseTo(probe.index, 6);
  expect(list[0].points[0].price).toBeCloseTo(probe.close, 9);
  expect(list[0].points[0].price).not.toBeCloseTo(probe.hidden_high, 9);
});

test("Ctrl magnets an anchor drag to a bar's OHLC (and never straightens)", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ]);
  }, s);
  await settle_frames(page);
  // Select with a body click, then drag anchor 1 near a different bar with Ctrl held: the
  // anchor snaps to that bar's center and to the closest of its OHLC — it does NOT straighten
  // to an angle.
  const mid = await page.evaluate(({ l0, l1, p_lo, p_hi }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(l0) + window.__chart.time_scale().logical_to_coordinate(l1)) / 2,
    y: (window.__main.price_to_coordinate(p_lo) + window.__main.price_to_coordinate(p_hi)) / 2,
  }), s);
  await page.mouse.click(mid.x, mid.y);
  const anchor = await spot(page, s.l1, s.p_hi);
  const probe = await page.evaluate(({ l0 }) => {
    const index = l0 + 3;
    const bar = window.__main.data_by_index(index);
    return {
      index,
      prices: [bar.open, bar.high, bar.low, bar.close],
      x: window.__chart.time_scale().logical_to_coordinate(index),
      y: window.__main.price_to_coordinate(bar.close) + 2,
    };
  }, s);
  await page.mouse.move(anchor.x, anchor.y);
  await page.keyboard.down("Control");
  await page.mouse.down();
  await page.mouse.move(probe.x, probe.y, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Control");
  const points = (await drawings(page))[0].points;
  // Magnet proof: snapped to the bar's center and to one of its OHLC prices.
  expect(points[1].logical).toBeCloseTo(probe.index, 6);
  expect(probe.prices).toContainEqual(points[1].price);
  // Straighten would have forced the dragged anchor onto a 0°/45°/90° ray from anchor 0 —
  // the magnet snap must not be overridden by it (Ctrl never straightens).
  expect(points[1].price).not.toBeCloseTo(points[0].price, 6);
});

test("Shift straightens a trend anchor drag to 45°", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ]);
  }, s);
  await settle_frames(page);
  let points = (await drawings(page))[0].points;
  // Select with a body click at the segment midpoint.
  const mid = await page.evaluate(({ l0, l1, p_lo, p_hi }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(l0) + window.__chart.time_scale().logical_to_coordinate(l1)) / 2,
    y: (window.__main.price_to_coordinate(p_lo) + window.__main.price_to_coordinate(p_hi)) / 2,
  }), s);
  await page.mouse.click(mid.x, mid.y);
  // Near-diagonal anchor drag with Shift: the segment straightens to 45° (|dx| == |dy| in px).
  const current = {
    x: await x_of(page, points[1].logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), points[1].price),
  };
  const fixed = {
    x: await x_of(page, points[0].logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), points[0].price),
  };
  await page.mouse.move(current.x, current.y);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(fixed.x + 55, fixed.y - 48, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  points = (await drawings(page))[0].points;
  const pa = {
    x: await x_of(page, points[0].logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), points[0].price),
  };
  const pb = {
    x: await x_of(page, points[1].logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), points[1].price),
  };
  expect(Math.abs(Math.abs(pb.x - pa.x) - Math.abs(pb.y - pa.y)), "45°: |dx| == |dy|").toBeLessThan(2);
});

test("Shift constrains a body drag to the dominant axis", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("trend_line", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ]);
  }, s);
  await settle_frames(page);
  const before = (await drawings(page))[0];
  const mid = await page.evaluate(({ l0, l1, p_lo, p_hi }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(l0) + window.__chart.time_scale().logical_to_coordinate(l1)) / 2,
    y: (window.__main.price_to_coordinate(p_lo) + window.__main.price_to_coordinate(p_hi)) / 2,
  }), s);
  // Mostly-horizontal body drag with Shift: prices frozen, logicals moved.
  await page.mouse.move(mid.x, mid.y);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(mid.x + 50, mid.y + 8, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  const after = (await drawings(page))[0];
  expect(after.points[0].logical).not.toBeCloseTo(before.points[0].logical, 3);
  expect(after.points[0].price).toBeCloseTo(before.points[0].price, 6);
  expect(after.points[1].price).toBeCloseTo(before.points[1].price, 6);
});

test("Shift squares a rectangle on the second placement click", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("rectangle"));
  const first = await spot(page, s.l0, s.p_lo);
  await page.mouse.click(first.x, first.y);
  const spacing = await bar_spacing(page);
  await page.keyboard.down("Shift");
  await page.mouse.click(first.x + 3 * spacing, first.y - 2.1 * spacing);
  await page.keyboard.up("Shift");
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  const [a, b] = list[0].points;
  const pa = {
    x: await x_of(page, a.logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), a.price),
  };
  const pb = {
    x: await x_of(page, b.logical),
    y: await page.evaluate((p) => window.__main.price_to_coordinate(p), b.price),
  };
  expect(Math.abs(Math.abs(pb.x - pa.x) - Math.abs(pb.y - pa.y)), "square: |dx| == |dy|").toBeLessThan(2);
});

test("Ctrl magnets the crosshair to the hovered bar's OHLC", async ({ page }) => {
  await goto_fixture(page);
  // The Ctrl magnet engages only for drawing work: arm a tool first (plain browsing never
  // price-snaps on Ctrl).
  await page.evaluate(() => window.__chart.set_drawing_tool("trend_line"));
  // The crosshair's horizontal line is the default crosshair gray — find the pane row with
  // the most of it (the dashed line covers the pane width).
  const CROSS = [194, 194, 194]; // light border-strong crosshair line #c2c2c2
  const crosshair_row = async () => {
    const png = await capture(page);
    const pane_bottom = Math.round((fixture.css_height - fixture.time_axis_height) * PR);
    let best_row = null;
    let best_count = 0;
    for (let y = 0; y < pane_bottom; y += 1) {
      let count = 0;
      for (let x = 0; x < png.width; x += 1) {
        const o = (y * png.width + x) * 4;
        if (
          Math.abs(png.data[o] - CROSS[0]) <= 4 &&
          Math.abs(png.data[o + 1] - CROSS[1]) <= 4 &&
          Math.abs(png.data[o + 2] - CROSS[2]) <= 4
        ) count += 1;
      }
      if (count > best_count) {
        best_count = count;
        best_row = y;
      }
    }
    return best_row;
  };
  // Hover a spot on a bar with a real high/close spread, 30% of the way from the high (an
  // unambiguous nearest-OHLC target): Normal mode leaves the crosshair at the cursor y.
  const probe = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    let index = Math.floor(range.from + (range.to - range.from) * 0.45);
    let bar = null;
    for (let i = index; i < Math.floor(range.to) - 1; i += 1) {
      const candidate = window.__main.data_by_index(i);
      const span = window.__main.price_to_coordinate(candidate.close) - window.__main.price_to_coordinate(candidate.high);
      if (span > 10) {
        index = i;
        bar = candidate;
        break;
      }
    }
    const high_y = window.__main.price_to_coordinate(bar.high);
    const y = high_y + (window.__main.price_to_coordinate(bar.close) - high_y) * 0.3;
    return { index, bar, x: window.__chart.time_scale().logical_to_coordinate(index), y };
  });
  await page.mouse.move(probe.x, probe.y);
  await settle_frames(page);
  const free_row = await crosshair_row();
  expect(free_row).not.toBeNull();
  expect(Math.abs(free_row - probe.y * PR), "Normal mode: crosshair at the cursor").toBeLessThan(2 * PR);

  // Ctrl held: the line snaps to the bar's high (the nearest of its OHLC in px).
  await page.keyboard.down("Control");
  await settle_frames(page);
  const snapped_row = await crosshair_row();
  const high_y = await page.evaluate((h) => window.__main.price_to_coordinate(h), probe.bar.high);
  expect(Math.abs(snapped_row - high_y * PR), "Ctrl: snapped to the bar's high").toBeLessThan(2 * PR);
  expect(Math.abs(snapped_row - probe.y * PR)).toBeGreaterThan(2 * PR);

  // Released: back to the raw cursor position (the configured mode is untouched).
  await page.keyboard.up("Control");
  await settle_frames(page);
  const released_row = await crosshair_row();
  expect(Math.abs(released_row - probe.y * PR), "released: raw again").toBeLessThan(2 * PR);
});

test("brush: freehand drag draws a smooth stroke that survives commit unchanged", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("brush", { color: "#000000", width: 2 }));
  // Drag an arc across the pane in small steps (the engine decimates the raw moves by distance).
  const from = await spot(page, s.l0, s.p_lo);
  const bend = await spot(page, Math.floor((s.l0 + s.l1) / 2), s.p_hi);
  const to = await spot(page, s.l1, s.p_lo);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  const steps = 24;
  for (let i = 1; i <= steps; i += 1) {
    const t = i / steps;
    // Quadratic bezier through the bend for a smooth arc gesture.
    const x = (1 - t) * (1 - t) * from.x + 2 * (1 - t) * t * bend.x + t * t * to.x;
    const y = (1 - t) * (1 - t) * from.y + 2 * (1 - t) * t * bend.y + t * t * to.y;
    await page.mouse.move(x, y);
  }
  await page.mouse.up();
  await settle_frames(page);
  // Committed, selected: every decimated capture point is kept — the committed curve is
  // exactly what the live drag painted (no commit-time thinning or reshaping).
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect(list[0].kind).toBe("brush");
  const point_count = list[0].points.length;
  expect(point_count).toBeGreaterThanOrEqual(2);
  expect(point_count).toBeLessThanOrEqual(steps + 1);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(list[0].id);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();

  // The selection shows anchor handles at the TWO ENDS only — not one per path point. A
  // two-anchor trend line's handles give the same blue footprint.
  const with_handles = await capture(page);
  const brush_blue = count_color(with_handles, BLUE);
  expect(brush_blue, "end anchors visible").toBeGreaterThan(40);
  expect(brush_blue, "not one handle per path point").toBeLessThan(200);
  await focus_overlay(page);
  await page.keyboard.press("Escape"); // deselect

  // Body hit: clicking the stroke selects it again; dragging an END anchor re-anchors only it.
  const end = (await drawings(page))[0].points.at(-1);
  const end_px = { x: await x_of(page, end.logical), y: await page.evaluate((p) => window.__main.price_to_coordinate(p), end.price) };
  await page.mouse.click(end_px.x, end_px.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing()?.id ?? null)).toBe(list[0].id);
  const before = (await drawings(page))[0].points;
  await page.mouse.move(end_px.x, end_px.y);
  await page.mouse.down();
  await page.mouse.move(end_px.x + 30, end_px.y - 20, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const after = (await drawings(page))[0].points;
  expect(after.length).toBe(before.length);
  // Only the last endpoint moved; the first is untouched.
  expect(after[0].logical).toBeCloseTo(before[0].logical, 6);
  expect(after[0].price).toBeCloseTo(before[0].price, 6);
  const moved = after.at(-1);
  expect(Math.hypot(moved.logical - before.at(-1).logical, moved.price - before.at(-1).price)).toBeGreaterThan(0.01);
});

test("brush: a click without a drag discards the stroke", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("brush"));
  const p = await spot(page, s.l0, s.p_lo);
  await page.mouse.click(p.x, p.y);
  await settle_frames(page);
  expect(await drawings(page)).toHaveLength(0);
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBe("brush");
  // Escape disarms cleanly (no pending state leaks).
  await focus_overlay(page);
  await page.keyboard.press("Escape");
  expect(await page.evaluate(() => window.__chart.active_drawing_tool())).toBeNull();
});

test("changing style mid-edit never wipes the typed text; font size control applies", async ({ page }) => {
  // The full demo (toolbar visible) — the regression: typing in the editor, then changing a
  // style input, used to write the toolbar's EMPTY label field into the drawing, wiping the
  // typed text back to the placeholder.
  await page.goto("/");
  await wait_for_chart(page);
  await page.waitForFunction(() => performance.now() > 600);
  const offset = await page.evaluate(() => {
    const r = document.getElementById("chart_container").getBoundingClientRect();
    return { left: r.left, top: r.top };
  });
  const spots = await anchor_spots(page);
  const p = await spot(page, spots.l0, spots.p_mid);

  await page.click("#drawings_group [data-tool='text']");
  await page.mouse.click(p.x + offset.left, p.y + offset.top);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  await editor.fill("keep me");
  // Change the weight like a real user (focusing the toolbar input blurs the editor, which
  // commits the text first; the change then applies the style) — the text must survive.
  await page.evaluate(() => document.getElementById("drawing_weight").focus());
  await page.selectOption("#drawing_weight", "700");
  await expect(page.locator("#chart_container #aeris_charts-text-editor")).toHaveCount(0);
  const options = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(options.text).toBe("keep me");
  expect(options.text_weight).toBe(700);

  // The font-size control templates/applies too.
  await page.evaluate(() => {
    const el = document.getElementById("drawing_text_size");
    el.value = "24";
    el.dispatchEvent(new Event("change"));
  });
  const sized = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(sized.text_size).toBe(24);
  expect(sized.text).toBe("keep me");

  // The drawing is still selected after the style edits — the toolbar mirrors it on pointerup.
  await page.mouse.click(p.x + offset.left + 200, p.y + offset.top); // empty pane: deselect
  await settle_frames(page);
  await page.mouse.click(p.x + offset.left, p.y + offset.top); // first click: select only
  await settle_frames(page);
  expect(await page.evaluate(() => window.__chart.selected_drawing())).not.toBeNull();
  await expect(page.locator("#chart_container #aeris_charts-text-editor")).toHaveCount(0);
  const synced = await page.evaluate(() => ({
    weight: document.getElementById("drawing_weight").value,
    size: document.getElementById("drawing_text_size").value,
    text: document.getElementById("drawing_text").value,
  }));
  expect(synced.weight).toBe("700");
  expect(synced.size).toBe("24");
  expect(synced.text).toBe("keep me");
});

test("tool customization templates new drawings and applies live to the selected one", async ({ page }) => {  // The full demo (toolbar visible) — style/width/color are the drawing customization settings.
  await page.goto("/");
  await wait_for_chart(page);
  await page.waitForFunction(() => performance.now() > 600);
  const offset = await page.evaluate(() => {
    const r = document.getElementById("chart_container").getBoundingClientRect();
    return { left: r.left, top: r.top };
  });
  const spots = await anchor_spots(page);
  const at = async (logical, price) => {
    const p = await spot(page, logical, price);
    return { x: p.x + offset.left, y: p.y + offset.top };
  };

  // Template: dotted, width 5, magenta — the next drawing carries them.
  await page.selectOption("#drawing_style", "dotted");
  await page.evaluate(() => {
    const width = document.getElementById("drawing_width");
    width.value = "5";
    width.dispatchEvent(new Event("change"));
    const color = document.getElementById("drawing_color");
    color.value = "#ff00ff";
    color.dispatchEvent(new Event("change"));
  });
  await page.click("#drawings_group [data-tool='trend_line']");
  const a = await at(spots.l0, spots.p_lo);
  const b = await at(spots.l1, spots.p_hi);
  await page.mouse.click(a.x, a.y);
  await page.mouse.click(b.x, b.y);
  const created = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(created.style).toBe("dotted");
  expect(created.width).toBe(5);
  expect(created.color).toBe("#ff00ff");
  expect(created.text_color, "fresh trend labels inherit their line color").toBe("");
  expect([created.text_h_align, created.text_v_align], "fresh trend labels default to top-right").toEqual(["right", "top"]);

  const [label_x, label_y] = await page.evaluate(() => (
    window.__chart.wasm.drawing_text_transform(window.__chart.drawings()[0].id)
  ));
  await page.mouse.move(label_x + offset.left, label_y + offset.top);
  await page.mouse.click(label_x + offset.left, label_y + offset.top);
  const editor = page.locator("#aeris_charts-text-input");
  await expect(editor).toBeFocused();
  expect(await editor.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");
  expect(await page.locator("#aeris_charts-text-caret").evaluate((el) => getComputedStyle(el).backgroundColor)).toBe("rgb(255, 0, 255)");
  await page.keyboard.press("Escape");

  // Live-apply: with the drawing selected, changing the settings updates it in place.
  await page.mouse.click(a.x, a.y);
  expect(await page.evaluate(() => window.__chart.selected_drawing())).not.toBeNull();
  await page.selectOption("#drawing_style", "dashed");
  await page.evaluate(() => {
    const el = document.getElementById("drawing_width");
    el.value = "3";
    el.dispatchEvent(new Event("change"));
  });
  const updated = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(updated.style).toBe("dashed");
  expect(updated.width).toBe(3);
});

test("trend labels rotate, reverse, template alignment, and never inherit text-tool content", async ({ page }) => {
  await page.goto("/");
  await wait_for_chart(page);
  await page.waitForFunction(() => performance.now() > 600);
  const offset = await page.evaluate(() => {
    const rect = document.getElementById("chart_container").getBoundingClientRect();
    return { left: rect.left, top: rect.top };
  });
  const spots = await anchor_spots(page);
  const a = await spot(page, spots.l0, spots.p_lo);
  const b = await spot(page, spots.l1, spots.p_hi);

  await page.fill("#drawing_text", "standalone only");
  await page.evaluate(() => {
    const input = document.getElementById("drawing_text_color");
    input.value = "#d32f2f";
    input.dispatchEvent(new Event("change"));
  });
  await page.selectOption("#drawing_text_h_align", "right");
  await page.selectOption("#drawing_text_v_align", "bottom");
  await page.click("#drawings_group [data-tool='trend_line']");
  await page.mouse.click(a.x + offset.left, a.y + offset.top);
  await page.mouse.click(b.x + offset.left, b.y + offset.top);

  let first = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(first.text, "trend templates never consume the standalone text field").toBe("");
  expect([first.text_h_align, first.text_v_align]).toEqual(["right", "bottom"]);
  expect(first.text_size, "drawing text defaults to 14 CSS px").toBe(14);
  expect(first.text_color, "the visible text-color control templates trend labels").toBe("#d32f2f");

  const [slot_x, slot_y, slot_angle] = await page.evaluate(() => (
    window.__chart.wasm.drawing_text_transform(window.__chart.drawings()[0].id)
  ));
  const slot = { x: slot_x, y: slot_y, angle: slot_angle };
  await page.mouse.move(slot.x + offset.left, slot.y + offset.top);
  await settle_frames(page);
  expect(await overlay_cursor(page)).toBe("text");
  const prompt = crop_around(await capture(page), slot.x * PR, slot.y * PR, 140, 80);
  let configured_red_pixels = 0;
  for (let o = 0; o < prompt.data.length; o += 4) {
    if (prompt.data[o] > prompt.data[o + 1] + 20 && prompt.data[o] > prompt.data[o + 2] + 20) {
      configured_red_pixels += 1;
    }
  }
  expect(configured_red_pixels, "the real WebGPU prompt uses configured text RGB, not gray").toBeGreaterThan(5);
  await page.mouse.click(slot.x + offset.left, slot.y + offset.top);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeFocused();
  const wrap = page.locator("#chart_container #aeris_charts-text-editor");
  expect(await editor.evaluate((el) => getComputedStyle(el).fontSize)).toBe("14px");
  expect(await editor.evaluate((el) => getComputedStyle(el).opacity)).toBe("0");
  expect(await page.locator("#aeris_charts-text-caret")).toBeVisible();
  const editor_angle = () => wrap.evaluate((el) => Number(el.style.transform.match(/rotate\(([-\d.e]+)rad\)/)?.[1]));
  expect(await editor_angle()).toBeCloseTo(slot.angle, 6);
  await editor.fill("owned trend label");

  // A live DOM selection must never paint a second theme-colored copy over the rotated frame
  // glyphs. Compare the real composed host surface with and without the editor wrapper: only the
  // explicit one-pixel caret may differ.
  await editor.evaluate((el) => {
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(el);
    selection.removeAllRanges();
    selection.addRange(range);
  });
  const selection_paint = await editor.evaluate((el) => ({
    background: getComputedStyle(el, "::selection").backgroundColor,
    color: getComputedStyle(el, "::selection").color,
  }));
  expect(selection_paint.background).toBe("rgba(0, 0, 0, 0)");
  expect(selection_paint.color).toBe("rgba(0, 0, 0, 0)");
  const with_caret = crop_around(
    PNG.sync.read(await page.locator("#chart_container").screenshot({ animations: "disabled" })),
    slot.x * PR,
    slot.y * PR,
    300,
    120,
  );
  await wrap.evaluate((el) => { el.style.opacity = "0"; });
  const frame_only = crop_around(
    PNG.sync.read(await page.locator("#chart_container").screenshot({ animations: "disabled" })),
    slot.x * PR,
    slot.y * PR,
    300,
    120,
  );
  await wrap.evaluate((el) => { el.style.opacity = "1"; });
  expect(pixel_diff(with_caret, frame_only), "editing adds only the explicit caret, never duplicate glyph or selection paint").toBeLessThan(80);

  await page.evaluate(() => {
    const drawing = window.__chart.drawings()[0];
    drawing.set_points([...drawing.points()].reverse());
  });
  await settle_frames(page);
  expect(await editor_angle()).toBeCloseTo(slot.angle, 6);
  await page.keyboard.press("Enter");
  first = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(first.text).toBe("owned trend label");

  await page.selectOption("#drawing_text_h_align", "left");
  await page.selectOption("#drawing_text_v_align", "top");
  first = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect([first.text_h_align, first.text_v_align]).toEqual(["left", "top"]);
  expect(first.text, "alignment changes do not wipe an existing trend label").toBe("owned trend label");

  await page.click("#drawings_group [data-tool='trend_line']");
  await page.mouse.click(a.x + offset.left, a.y + offset.top);
  await page.mouse.click(b.x + offset.left, b.y + offset.top);
  const second = await page.evaluate(() => window.__chart.drawings()[1].options());
  expect(second.text, "subsequent trend lines start empty").toBe("");
  expect([second.text_h_align, second.text_v_align]).toEqual(["left", "top"]);

  await page.click("#drawings_group [data-tool='text']");
  await page.mouse.click((a.x + b.x) / 2 + offset.left, a.y + offset.top - 50);
  await expect(editor).toHaveText("standalone only");
  await page.keyboard.press("Enter");
  const standalone = await page.evaluate(() => window.__chart.drawings().at(-1).options());
  expect(standalone.text).toBe("standalone only");
});

test("text tool: press places and opens typing mode; typing commits; leaving empty removes", async ({ page }) => {
  await goto_fixture(page);
  // No crosshair pixels near the probes.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  const p = await spot(page, s.l0, s.p_mid);
  const clean = await capture(page);
  await page.evaluate(() => window.__chart.set_drawing_tool("text"));
  await page.mouse.click(p.x, p.y);
  // Typing mode: borderless caret overlay; the engine's focus border is the only outline.
  const wrap = page.locator("#chart_container #aeris_charts-text-editor");
  await expect(wrap).toBeVisible();
  const editor = wrap.locator("#aeris_charts-text-input");
  await expect(editor).toBeFocused();
  await expect(page.locator("#aeris_charts-text-preview")).toHaveCount(0);
  expect(await wrap.evaluate((el) => getComputedStyle(el).borderStyle)).toBe("none");
  // Focus border stays on the canvas (primary blue ring) — not a DOM handoff.
  const blue_ring = (png) => count_color(crop_around(png, p.x * PR, p.y * PR, 130, 40), BLUE, 40);
  expect(blue_ring(await capture(page)), "engine focus border while editing").toBeGreaterThan(10);
  // Engine keeps painting the label under the transparent editor (no lift). Empty still paints nothing.
  const dark_clean = count_dark(crop_around(clean, p.x * PR, p.y * PR, 130, 26));
  const dark_editing = count_dark(crop_around(await capture(page), p.x * PR, p.y * PR, 130, 26));
  expect(dark_editing, "empty edit paints no ghost ink").toBeLessThanOrEqual(dark_clean + 40);

  await editor.fill("engine label");
  await page.keyboard.press("Enter");
  await settle_frames(page);
  await expect(page.locator("#chart_container #aeris_charts-text-editor")).toHaveCount(0);
  const list = await drawings(page);
  expect(list).toHaveLength(1);
  expect((await page.evaluate(() => window.__chart.drawings()[0].options())).text).toBe("engine label");

  // Leaving empty (Escape on a fresh placement) removes the drawing — no "Add text" left behind.
  await page.evaluate(() => window.__chart.set_drawing_tool("text"));
  await page.mouse.click(p.x + 80, p.y);
  await expect(page.locator("#chart_container #aeris_charts-text-input")).toBeVisible();
  await page.keyboard.press("Escape");
  await settle_frames(page);
  expect(await drawings(page)).toHaveLength(1);
});

test("text tool: first click selects (focus border), a second click opens typing mode; Escape cancels", async ({ page }) => {
  await goto_fixture(page);
  // No crosshair pixels near the probes.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "source" });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  const editor = page.locator("#chart_container #aeris_charts-text-input");

  // the public reference's two-step model: the first click only SELECTS — no editor, the drawing
  // becomes selected, and the engine's focus border (primary blue ring) paints around the
  // label. No anchor discs exist for text (covered engine-side in frame/tests.rs).
  const before_select = await capture(page);
  await page.mouse.click(p.x, p.y);
  await settle_frames(page);
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => window.__chart.selected_drawing())).not.toBeNull();
  const blue_ring = (png) => count_color(crop_around(png, p.x * PR, p.y * PR, 130, 40), BLUE, 40);
  expect(blue_ring(await capture(page)), "focus border paints on select").toBeGreaterThan(blue_ring(before_select) + 10);

  // The second click on the selected label opens typing mode, prefilled.
  await page.mouse.click(p.x, p.y);
  await expect(editor).toBeVisible();
  // Overlay caret: editor glyphs are transparent; the canvas label is the ink.
  const ink = await editor.evaluate((el) => ({
    color: getComputedStyle(el).color,
    fill: getComputedStyle(el).webkitTextFillColor,
    caret: getComputedStyle(el).caretColor,
    opacity: getComputedStyle(el).opacity,
  }));
  expect(ink.color).toBe("rgba(0, 0, 0, 0)");
  expect(ink.fill).toBe("rgba(0, 0, 0, 0)");
  expect(ink.caret).toBe("rgba(0, 0, 0, 0)");
  expect(ink.opacity).toBe("0");
  await expect(page.locator("#aeris_charts-text-caret")).toBeVisible();
  // Escape discards the edit.
  await editor.fill("discarded");
  await page.keyboard.press("Escape");
  await settle_frames(page);
  await expect(page.locator("#chart_container #aeris_charts-text-editor")).toHaveCount(0);
  expect((await page.evaluate(() => window.__chart.drawings()[0].options())).text).toBe("source");
  // Still selected after the cancelled edit: ONE click re-enters typing mode.
  await page.mouse.click(p.x, p.y);
  await expect(editor).toBeVisible();
  await page.keyboard.press("Escape");
});

test("hovering a text drawing shows the focus border at reduced opacity; leaving clears it", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "hoverable" });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  // The hover ring blends the primary blue at ~45% over the light fixture background: a
  // distinctly blue-but-not-full-strength tint the plain chart never contains.
  const bluish = (png) => {
    const crop = crop_around(png, p.x * PR, p.y * PR, 140, 44);
    let n = 0;
    for (let o = 0; o < crop.data.length; o += 4) {
      const r = crop.data[o], g = crop.data[o + 1], b = crop.data[o + 2];
      if (b > r + 30 && b > g + 20) n += 1;
    }
    return n;
  };
  const resting = bluish(await capture(page));
  await page.mouse.move(p.x, p.y);
  await settle_frames(page);
  expect(bluish(await capture(page)), "hover ring paints").toBeGreaterThan(resting + 10);
  // Moving off the label removes the ring.
  await page.mouse.move(p.x, p.y + 120);
  await settle_frames(page);
  expect(bluish(await capture(page)), "hover ring clears").toBeLessThanOrEqual(resting + 4);
});

test("empty text paints nothing on the chart; leaving edit without typing removes it", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  const clean = await capture(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }]);
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  // No "Add text" ghost on the canvas for an empty drawing.
  expect(
    pixel_diff(crop_around(clean, p.x * PR, p.y * PR, 130, 26), crop_around(await capture(page), p.x * PR, p.y * PR, 130, 26)),
    "empty text paints nothing",
  ).toBe(0);
  // Empty text: first click opens typing mode (no ink to focus). Leaving without typing removes it.
  await page.mouse.click(p.x, p.y);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  await expect(editor).toHaveText("");
  await expect(page.locator("#aeris_charts-text-preview")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await settle_frames(page);
  expect(await drawings(page)).toHaveLength(0);
});

test("text drawing moves freely in both directions with a body drag", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "movable" });
  }, s);
  await settle_frames(page);
  const before = (await drawings(page))[0].points[0];
  const p = await spot(page, s.l0, s.p_mid);
  // Drag the label diagonally: both the logical and the price follow.
  await page.mouse.move(p.x, p.y);
  await page.mouse.down();
  await page.mouse.move(p.x + 45, p.y - 25, { steps: 4 });
  await page.mouse.up();
  await settle_frames(page);
  const after = (await drawings(page))[0].points[0];
  expect(after.logical).not.toBeCloseTo(before.logical, 3);
  expect(after.price).not.toBeCloseTo(before.price, 3);
  // Clicking the label afterwards still opens typing mode (movement does not eat the click).
  const moved_p = { x: p.x + 45, y: p.y - 25 };
  await page.mouse.click(moved_p.x, moved_p.y);
  await expect(page.locator("#chart_container #aeris_charts-text-input")).toBeVisible();
  await page.keyboard.press("Escape");
});

test("the editor tracks its anchor through wheel zoom and scroll (no displacement)", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "anchored" });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  // Two-step: first click selects, second opens typing mode.
  await page.mouse.click(p.x, p.y);
  await page.mouse.click(p.x, p.y);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  const text_center = () => page.evaluate(() => {
    const editor = document.querySelector("#aeris_charts-text-input");
    const range = document.createRange();
    range.selectNodeContents(editor);
    const r = range.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  });
  const anchor_of = () => page.evaluate(() => {
    const c = window.__chart.wasm.drawing_point_to_coordinate(window.__chart.drawings()[0].id, 0);
    return { x: c[0], y: c[1] };
  });
  // Wheel-zoom over the pane (does NOT blur the editor): the visible range and the anchor's
  // pixel position change — the editor must follow.
  await page.mouse.move(p.x + 100, p.y);
  await page.mouse.wheel(0, -240);
  await settle_frames(page);
  await expect(editor).toBeVisible();
  let a = await anchor_of();
  let c = await text_center();
  expect(Math.abs(c.x - a.x), "editor follows the anchor through zoom").toBeLessThanOrEqual(2);
  expect(Math.abs(c.y - a.y)).toBeLessThanOrEqual(2);

  // Wheel-scroll pans the chart: same tracking contract.
  await page.mouse.wheel(240, 0);
  await settle_frames(page);
  await expect(editor).toBeVisible();
  a = await anchor_of();
  c = await text_center();
  expect(Math.abs(c.x - a.x), "editor follows the anchor through scroll").toBeLessThanOrEqual(2);
  expect(Math.abs(c.y - a.y)).toBeLessThanOrEqual(2);
  await page.keyboard.press("Escape");
});

test("typing mode keeps the text pixel-anchored (no shift, same size) as it grows", async ({ page }) => {  await goto_fixture(page);
  // No crosshair pixels near the probes.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "devraj" });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  // Two-step: first click selects, second opens typing mode.
  await page.mouse.click(p.x, p.y);
  await page.mouse.click(p.x, p.y);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  // The editable's REAL text rect (Range) vs the engine anchor — the calibration's contract.
  const metrics = () => page.evaluate(() => {
    const editor = document.querySelector("#aeris_charts-text-input");
    const range = document.createRange();
    range.selectNodeContents(editor);
    const r = range.getBoundingClientRect();
    const cs = getComputedStyle(editor);
    return {
      center: { x: r.left + r.width / 2, y: r.top + r.height / 2 },
      font_size: cs.fontSize,
      font_style: cs.fontStyle,
      font_weight: cs.fontWeight,
    };
  });
  // Exactly the engine's committed size: text_size, else the text tool's own 14px default
  // (TEXT_TOOL_DEFAULT_SIZE, drawings.rs) — NOT layout.fontSize, and no editor-side floor.
  const expected_size = 14;
  let m = await metrics();
  // Pixel-band bounds quantize both glyph edges independently, so their inferred center can
  // differ from the vector anchor by up to two physical pixels at fractional DPR.
  expect(Math.abs(m.center.x - p.x), "text x on the anchor").toBeLessThanOrEqual(2);
  expect(Math.abs(m.center.y - p.y), "text y on the anchor").toBeLessThanOrEqual(2);
  // Same font size/style/weight as the engine's committed text.
  expect(m.font_size).toBe(`${expected_size}px`);
  expect(m.font_style).toBe("normal");
  expect(m.font_weight).toBe("400");
  // Growing the text keeps the anchor (no drifting/lifting while typing).
  await editor.fill("devraj the great king of everything");
  m = await metrics();
  expect(Math.abs(m.center.x - p.x), "grown text x still on the anchor").toBeLessThanOrEqual(2);
  expect(Math.abs(m.center.y - p.y), "grown text y still on the anchor").toBeLessThanOrEqual(2);
  await page.keyboard.press("Escape");
});

test("typing mode matches the committed render: exact size and baseline (no jump on edit/commit)", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  // A sub-12px size: the editor must NOT floor it — the engine rasterizes committed text at
  // the exact size (only the empty-text placeholder is floored at 12, drawings.rs). Flat-bottom
  // glyphs (no descenders/overshoot) so the committed ink's bottom edge IS the baseline.
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], { text: "HAHA", text_size: 10, text_color: "#7b1fa2" });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  // The committed canvas run's baseline from real pixels: the last purple ink row (+1)
  // (the fixture page renders the chart at the viewport top-left, so capture px are comparable
  // to viewport CSS px × the device ratio).
  const png = await capture(page);
  let run_baseline = -1;
  for (let y = 0; y < png.height; y += 1) {
    for (let x = 0; x < png.width; x += 1) {
      const o = (y * png.width + x) * 4;
      if (
        Math.abs(png.data[o] - PURPLE[0]) <= 40 &&
        Math.abs(png.data[o + 1] - PURPLE[1]) <= 40 &&
        Math.abs(png.data[o + 2] - PURPLE[2]) <= 40 &&
        y + 1 > run_baseline
      ) run_baseline = y + 1;
    }
  }
  expect(run_baseline, "committed label paints").toBeGreaterThan(0);
  // Two-step: first click selects, second opens typing mode.
  await page.mouse.click(p.x, p.y);
  await page.mouse.click(p.x, p.y);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  const probe = await page.evaluate(() => {
    const editor = document.querySelector("#aeris_charts-text-input");
    // The editor's own baseline, via the same zero-size inline-probe the package uses.
    const span = document.createElement("span");
    span.style.display = "inline-block";
    span.style.width = "0";
    span.style.height = "0";
    editor.appendChild(span);
    const baseline = span.getBoundingClientRect().top;
    span.remove();
    return { font_size: getComputedStyle(editor).fontSize, baseline };
  });
  expect(probe.font_size, "editor size == committed size (no 12px floor)").toBe("10px");
  expect(
    Math.abs(probe.baseline * PR - run_baseline),
    "editor baseline on the committed run's ink baseline",
  ).toBeLessThanOrEqual(1.6);
  await page.keyboard.press("Escape");
});

test("empty typing mode is a blank caret box (no Add text ghost)", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }]);
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  await page.mouse.click(p.x, p.y);
  const editor = page.locator("#chart_container #aeris_charts-text-input");
  await expect(editor).toBeVisible();
  await expect(editor).toHaveText("");
  await expect(page.locator("#aeris_charts-text-preview")).toHaveCount(0);
  const font_size = await editor.evaluate((el) => getComputedStyle(el).fontSize);
  expect(font_size).toBe("14px");
  await page.keyboard.press("Escape");
  expect(await drawings(page)).toHaveLength(0);
});

test("text styling: weight, italic, and color flow through options and pixels", async ({ page }) => {  await goto_fixture(page);
  // No crosshair pixels near the probes.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { mode: 2 } }));
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], {
      text: "styled",
      text_weight: 800,
      text_italic: true,
      text_color: "#7b1fa2",
    });
  }, s);
  await settle_frames(page);
  const options = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(options.text_weight).toBe(800);
  expect(options.text_italic).toBe(true);
  expect(options.text_color).toBe("#7b1fa2");
  // Derived legacy boolean (semibold and up reads bold).
  expect(options.text_bold).toBe(true);
  // The styled label paints (purple pixels at the anchor).
  const p = await spot(page, s.l0, s.p_mid);
  expect(color_centroid(await capture(page), PURPLE), "styled label paints").not.toBeNull();

  // Patching down to normal weight + non-italic changes the footprint measurably.
  const heavy = await capture(page);
  await page.evaluate(() => window.__chart.drawings()[0].apply_options({ text_weight: 400, text_italic: false }));
  await settle_frames(page);
  const normal = await capture(page);
  expect(pixel_diff(
    crop_around(heavy, p.x * PR, p.y * PR, 160, 30),
    crop_around(normal, p.x * PR, p.y * PR, 160, 30),
  ), "weight/italic change restyles the label").toBeGreaterThan(50);
  const after = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(after.text_weight).toBe(400);
  expect(after.text_italic).toBe(false);
  expect(after.text_bold).toBe(false);
});

test("text tool container: background and border make it a box", async ({ page }) => {  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, p_mid }) => {
    window.__chart.add_drawing("text", [{ logical: l0, price: p_mid }], {
      text: "boxed",
      box_color: "rgba(41, 98, 255, 0.85)",
      box_border_color: "#e91e63",
      box_border_width: 2,
      text_color: "#ffffff",
    });
  }, s);
  await settle_frames(page);
  const p = await spot(page, s.l0, s.p_mid);
  const crop = crop_around(await capture(page), p.x * PR, p.y * PR, 120, 40);
  expect(count_color(crop, [41, 98, 255], 60), "box background paints").toBeGreaterThan(50);
  expect(count_color(crop, [233, 30, 99], 60), "box border paints").toBeGreaterThan(10);
  // The options round-trip exposes the container settings.
  const options = await page.evaluate(() => window.__chart.drawings()[0].options());
  expect(options.box_color).toBe("rgba(41, 98, 255, 0.85)");
  expect(options.box_border_color).toBe("#e91e63");
  expect(options.box_border_width).toBe(2);
});

test("rectangle: middle pans unselected, drags selected, 8 anchors from the first click", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l0, l1, p_lo, p_hi }) => {
    window.__chart.add_drawing("rectangle", [
      { logical: l0, price: p_lo },
      { logical: l1, price: p_hi },
    ], { color: "#2962ff" });
  }, s);
  await settle_frames(page);
  const before = (await drawings(page))[0];
  const center_of = (points) => page.evaluate(({ points }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(points[0].logical) + window.__chart.time_scale().logical_to_coordinate(points[1].logical)) / 2,
    y: (window.__main.price_to_coordinate(points[0].price) + window.__main.price_to_coordinate(points[1].price)) / 2,
  }), { points });
  const range = () => page.evaluate(() => window.__chart.time_scale().get_visible_logical_range());

  // Unselected: a middle grab pans the chart � the rectangle does not move.
  const r0 = await range();
  const center = await center_of(before.points);
  await page.mouse.move(center.x, center.y);
  await page.mouse.down();
  await page.mouse.move(center.x + 120, center.y, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);
  const r1 = await range();
  expect(r1.from, "the middle grab panned the chart").not.toBeCloseTo(r0.from, 3);
  expect((await drawings(page))[0].points[0].logical).toBeCloseTo(before.points[0].logical, 6);
  await page.evaluate((r) => window.__chart.time_scale().set_visible_logical_range(r), r0);
  await settle_frames(page);

  // Select with a border click: all 8 anchors paint.
  const border = await page.evaluate(({ points }) => ({
    x: (window.__chart.time_scale().logical_to_coordinate(points[0].logical) + window.__chart.time_scale().logical_to_coordinate(points[1].logical)) / 2,
    y: window.__main.price_to_coordinate(points[1].price),
  }), { points: before.points });
  await page.mouse.click(border.x, border.y);
  await settle_frames(page);
  expect(count_color(await capture(page), BLUE), "8 anchors once selected").toBeGreaterThan(40);

  // Selected: the middle now drags the whole rectangle.
  const spacing = await bar_spacing(page);
  const center2 = await center_of(before.points);
  await page.mouse.move(center2.x, center2.y);
  await page.mouse.down();
  await page.mouse.move(center2.x + 4 * spacing, center2.y, { steps: 5 });
  await page.mouse.up();
  await settle_frames(page);
  const moved = (await drawings(page))[0];
  expect(moved.points[0].logical).toBeCloseTo(before.points[0].logical + 4, 1);
  expect(moved.points[1].logical).toBeCloseTo(before.points[1].logical + 4, 1);

  // Creation: the 8 anchors show from the first click (before the second commits).
  await page.keyboard.press("Delete");
  await page.evaluate(() => window.__chart.set_drawing_tool("rectangle", { color: "#2962ff" }));
  const c1 = await spot(page, s.l0, s.p_lo);
  await page.mouse.click(c1.x, c1.y);
  const c2 = await spot(page, s.l1, s.p_hi);
  await page.mouse.move(c2.x, c2.y);
  await settle_frames(page);
  expect(count_color(await capture(page), BLUE), "8 anchors during the draw").toBeGreaterThan(40);
  await page.keyboard.press("Escape");
});

test("a vertical line body-drag with Ctrl snaps to the bar center (the public reference magnet)", async ({ page }) => {
  await goto_fixture(page);
  const s = await anchor_spots(page);
  await page.evaluate(({ l1 }) => {
    window.__chart.add_drawing("vertical_line", [{ logical: l1, price: 0 }]);
  }, s);
  await settle_frames(page);
  // Grab the line's body (not its anchor handle) with Ctrl and drag between two bars � the
  // single-anchor line's body drag IS the anchor drag, so the magnet snaps it to a bar center.
  const grab = await spot(page, s.l1, s.p_mid);
  const between = await page.evaluate(({ l0, l1 }) => {
    const a = window.__chart.time_scale().logical_to_coordinate(l0);
    const b = window.__chart.time_scale().logical_to_coordinate(l1);
    return (a + b) / 2;
  }, s);
  await page.keyboard.down("Control");
  await page.mouse.move(grab.x, grab.y);
  await page.mouse.down();
  await page.mouse.move(between, grab.y, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.up("Control");
  await settle_frames(page);
  const points = (await drawings(page))[0].points;
  expect(Number.isInteger(points[0].logical), "snapped onto a bar center").toBe(true);
});

test("Ctrl magnet engages only while a drawing tool is armed", async ({ page }) => {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.waitForFunction(() => performance.now() > 600);
  const box = await page.locator("#chart_container canvas").first().boundingBox();
  const hover = async () => {
    for (let i = 0; i < 4; i++) {
      await page.mouse.move(box.x + box.width / 2 - 30 + i * 20, box.y + box.height / 2);
      await page.waitForTimeout(120);
    }
  };
  const magnet = () => page.evaluate(() => window.__chart.wasm.crosshair_ohlc_magnet());

  // Plain browsing: Ctrl held, no tool armed � the crosshair must NOT price-snap.
  await page.keyboard.down("Control");
  await hover();
  expect(await magnet()).toBe(false);

  // Armed tool: the same Ctrl hover engages the magnet for the drawing flow.
  await page.evaluate(() => window.__chart.set_drawing_tool("vertical_line"));
  await hover();
  expect(await magnet()).toBe(true);

  // Disarming (Escape) drops it back off even with Ctrl still held.
  await focus_overlay(page);
  await page.keyboard.press("Escape");
  await hover();
  expect(await magnet()).toBe(false);
  await page.keyboard.up("Control");
});
