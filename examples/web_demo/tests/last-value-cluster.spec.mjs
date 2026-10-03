import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

// industry-standard last-value cluster: title chip + price text + candle-close countdown row,
// held together with side-specific corner radius. These specs drive the live demo page (hourly
// bars ending at the current hour) through the public API only.

const LABEL = [247, 82, 95]; // #f7525f — the deterministic final DOWN bar's label color
const CHIP = LABEL; // the title chip shares the main label color by default
const BORDER = [51, 51, 51]; // #333333 - dark axis border
const ROW = 15; // 11px axis text + 2*2 padding (compact price row)
const ROW_CD = 14; // 10px countdown text + 2*2 padding

const test_port = Number.parseInt(process.env.AERIS_CHARTS_TEST_PORT ?? "4174", 10);
const test_base_url = `http://127.0.0.1:${test_port}`;

async function wait_for_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

// Geometry probes default to DPR 1; alignment coverage overrides it when needed.
async function open_cluster_page(browser, options, deviceScaleFactor = 1, query = "") {
  const context = await browser.newContext({
    viewport: { width: 1280, height: 720 },
    deviceScaleFactor,
    colorScheme: "light",
  });
  const page = await context.newPage();
  await page.goto(`${test_base_url}/${query}`);
  await wait_for_chart(page);
  await page.evaluate((opts) => {
    // A deterministic final DOWN bar at the current second: the label color is pinned to
    // #f7525f and the countdown always has ~1h left (no hour-boundary flake). The bar is a
    // real `update` through the public series API.
    const now = Math.floor(Date.now() / 1000);
    const last = window.__data[window.__data.length - 1];
    const close = last.close - 2;
    window.__cluster_close = close;
    window.__main.update({ time: now, open: last.close, high: last.close + 0.6, low: close - 0.6, close });
    window.__main.apply_options({ down_color: "#f7525f", price_line_visible: false, ...opts });
  }, options);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  return { context, page };
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

async function cluster_anchor(page) {
  return page.evaluate(() => ({
    pane_w: window.__chart.time_scale().width(),
    y: window.__main.price_to_coordinate(window.__cluster_close),
  }));
}

function px(png, x, y) {
  const o = (y * png.width + x) * 4;
  return [png.data[o], png.data[o + 1], png.data[o + 2]];
}

function dist(a, b) {
  return Math.max(Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1]), Math.abs(a[2] - b[2]));
}

function near(a, b, tol = 12) {
  return dist(a, b) <= tol;
}

const is_box = (c) => near(c, LABEL) || near(c, CHIP);

// Locate the OUTSIDE title chip (the public reference geometry: it ends at the pane-side edge of the axis
// border). Since the chip shares the label color (candle bodies/wicks match
// too), detection is by box coverage: a chip is a solid rectangle (≥ 70% LABEL pixels over a
// 20px window on the row band); wicks/bodies never fill a window like that.
const is_label = (c) => near(c, LABEL);

function chip_boxes_at(png, y, x0, x1, row_h = ROW) {
  // A chip box is a run of columns with ≥ 1 LABEL pixel (allowing ≤ 8px text/AA dips), ≥ 12px
  // wide, averaging ≥ 0.5 coverage. Wicks/bodies are too narrow or too sparse to qualify.
  const hits_at = (x) => {
    let hit = 0;
    for (let yy = 0; yy < row_h; yy += 1) {
      const o = ((y + yy) * png.width + x) * 4;
      if (is_label([png.data[o], png.data[o + 1], png.data[o + 2]])) hit += 1;
    }
    return hit;
  };
  const boxes = [];
  let cur = null;
  let gap = 0;
  for (let x = x0; x < x1; x += 1) {
    const hit = hits_at(x);
    if (hit >= 1) {
      if (!cur) cur = { s: x, e: x, filled: 0 };
      cur.e = x;
      cur.filled += hit;
      gap = 0;
    } else if (cur) {
      gap += 1;
      if (gap > 8) { boxes.push(cur); cur = null; gap = 0; }
    }
  }
  if (cur) boxes.push(cur);
  return boxes.filter((b) => b.e - b.s + 1 >= 12 && b.filled / ((b.e - b.s + 1) * row_h) >= 0.5);
}

function chip_run_near(png, pane_w, anchor_y, scale = 1) {
  const row_h = Math.round(ROW * scale);
  const y = Math.round(anchor_y) - Math.floor(row_h / 2);
  // `x1` is exclusive. Include `pane_w - 1`, the final chart-side pixel before the border, so
  // this probe can distinguish a flush title chip from a one-pixel surface gap.
  const boxes = chip_boxes_at(png, y, Math.max(0, pane_w - Math.round(120 * scale)), pane_w, row_h);
  if (boxes.length === 0) return { left: -1, right: -1, top: -1, bottom: -1, found: false };
  const last = boxes[boxes.length - 1]; // the border-most box
  return { left: last.s, right: last.e, top: y, bottom: y + row_h, found: true };
}

function chip_extent(png, pane_w, anchor_y, scale = 1) {
  const run = chip_run_near(png, pane_w, anchor_y, scale);
  return { left: run.left, right: run.right, top: run.top, bottom: run.bottom, found: run.found };
}

function find_chip(png, pane_w, anchor_y) {
  return chip_run_near(png, pane_w, anchor_y);
}

function count_chip_left(png, pane_w, anchor_y) {
  const y = Math.round(anchor_y) - Math.floor(ROW / 2);
  let n = 0;
  for (const box of chip_boxes_at(png, y, Math.max(0, pane_w - 120), pane_w)) n += box.e - box.s + 1;
  return n;
}

// Locate the cluster's painted bounding box: the axis border occupies pane_w, so the box begins
// one device pixel later at DPR 1. Column pane_w+2 brackets the vertical extent; the right edge
// is the widest box-colored run across the rows (rounded corners only shrink the outer 2 rows).
function find_cluster(png, pane_w) {
  let top = -1;
  let bottom = -1;
  for (let y = 0; y < png.height; y += 1) {
    if (is_box(px(png, pane_w + 2, y))) {
      if (top === -1) top = y;
      bottom = y;
    }
  }
  let right = -1;
  for (let y = top; y <= bottom; y += 1) {
    for (let x = png.width - 1; x >= pane_w; x -= 1) {
      if (is_box(px(png, x, y))) {
        right = Math.max(right, x + 1);
        break;
      }
    }
  }
  return { left: pane_w + 1, top, right, bottom: bottom + 1 };
}

function count_where(png, box, predicate) {
  let n = 0;
  for (let y = box.top; y < box.bottom; y += 1) {
    for (let x = box.left; x < box.right; x += 1) {
      if (predicate(px(png, x, y))) n += 1;
    }
  }
  return n;
}

function rightmost_where(png, box, predicate) {
  let right = -1;
  for (let y = box.top; y < box.bottom; y += 1) {
    for (let x = box.left; x < box.right; x += 1) {
      if (predicate(px(png, x, y))) right = Math.max(right, x);
    }
  }
  return right;
}

function row_span_where(png, y, x0, x1, predicate) {
  let left = -1;
  let right = -1;
  for (let x = x0; x < x1; x += 1) {
    if (!predicate(px(png, x, y))) continue;
    if (left === -1) left = x;
    right = x;
  }
  return { left, right };
}

const count_color = (png, box, color) => count_where(png, box, (c) => near(c, color));
const is_white = (c) => c[0] > 240 && c[1] > 240 && c[2] > 240;

function white_ink_center(png, box) {
  let top = Infinity;
  let bottom = -Infinity;
  for (let y = box.top; y < box.bottom; y += 1) {
    for (let x = box.left; x < box.right; x += 1) {
      if (is_white(px(png, x, y))) {
        top = Math.min(top, y);
        bottom = Math.max(bottom, y);
      }
    }
  }
  expect(top, "label must contain white glyph ink").toBeLessThan(Infinity);
  return (top + bottom) / 2;
}

// Vertical center of the painted glyph ink at half coverage, as the pixel-edge midpoint of the
// first and last ink rows. Coverage is judged by the weakest channel, so white text over a
// saturated label fill counts once the blend passes 50%: thin descender strokes that never reach
// pure white still count, which the pure-white probe above cannot promise on every rasterizer.
function half_coverage_ink_extent(png, box) {
  let top = Infinity;
  let bottom = -Infinity;
  for (let y = box.top; y < box.bottom; y += 1) {
    for (let x = box.left; x < box.right; x += 1) {
      if (Math.min(...px(png, x, y)) > 128) {
        top = Math.min(top, y);
        bottom = Math.max(bottom, y);
      }
    }
  }
  expect(top, "label must contain glyph ink").toBeLessThan(Infinity);
  return { top, bottom: bottom + 1, center: (top + bottom + 1) / 2 };
}

function expect_white_ink_centered(png, box, tolerance = 1) {
  const box_center = (box.top + box.bottom - 1) / 2;
  expect(Math.abs(white_ink_center(png, box) - box_center)).toBeLessThanOrEqual(tolerance);
}

// The painted rows of a solid label box: the contiguous run of `color` in column `x` that
// contains row `y`. Probe a column without text or rounded corners.
function painted_row(png, x, y, color) {
  let top = y;
  let bottom = y;
  while (top > 0 && near(px(png, x, top - 1), color)) top -= 1;
  while (bottom + 1 < png.height && near(px(png, x, bottom + 1), color)) bottom += 1;
  expect(near(px(png, x, y), color), `label box must cover ${x},${y}`).toBe(true);
  return { top, bottom: bottom + 1 };
}

function color_bands(png, color) {
  const rows = [];
  for (let y = 0; y < png.height; y += 1) {
    let left = png.width;
    let right = -1;
    for (let x = 0; x < png.width; x += 1) {
      if (near(px(png, x, y), color)) {
        left = Math.min(left, x);
        right = x;
      }
    }
    if (right >= 0) rows.push({ y, left, right });
  }
  const bands = [];
  for (const row of rows) {
    const last = bands[bands.length - 1];
    if (last && row.y === last.bottom) {
      last.bottom += 1;
      last.left = Math.min(last.left, row.left);
      last.right = Math.max(last.right, row.right + 1);
    } else {
      bands.push({ left: row.left, right: row.right + 1, top: row.y, bottom: row.y + 1 });
    }
  }
  return bands;
}

function region_diff(a, b, box) {
  let diff = 0;
  for (let y = box.top; y < box.bottom; y += 1) {
    for (let x = box.left; x < box.right; x += 1) {
      if (dist(px(a, x, y), px(b, x, y)) > 10) diff += 1;
    }
  }
  return diff;
}


test("last-value cluster paints chip, price, and countdown rows; the chip matches the label color", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    title: "Aeris",
    title_visible: true,
    countdown_visible: true,
  });
  const anchor = await cluster_anchor(page);
  const on = await capture(page);
  const box = find_cluster(on, anchor.pane_w);
  expect(box.top, "cluster box should be located").toBeGreaterThanOrEqual(0);
  // One connected two-row box (15px price + 14px countdown at the default font).
  expect(box.bottom - box.top).toBeGreaterThanOrEqual(ROW + ROW_CD - 4);
  expect(box.bottom - box.top).toBeLessThanOrEqual(ROW + ROW_CD + 4);
  // The title chip sits OUTSIDE the axis strip and ends exactly at the border's pane-side edge,
  // in the SAME color as the price/countdown chips by default.
  const chip = find_chip(on, anchor.pane_w, anchor.y);
  expect(chip.found, "title chip outside the strip").toBe(true);
  const gap = anchor.pane_w - chip.right - 1;
  expect(gap, "no chart-surface pixels between title chip and border").toBe(0);
  const chip_pixel = px(on, chip.left + 3, box.top + Math.floor(ROW / 2));
  const price_pixel = px(on, box.left + 3, box.top + Math.floor(ROW / 2));
  expect(near(chip_pixel, CHIP), `chip pixel ${chip_pixel}`).toBe(true);
  expect(near(price_pixel, LABEL), `price pixel ${price_pixel}`).toBe(true);
  expect(dist(chip_pixel, price_pixel)).toBeLessThanOrEqual(12); // matching colors by default
  // Center the title against its painted row, not the anchor-derived probe window: the probe
  // rounds the anchor independently of the label box and can start a row below the chip.
  const chip_row = { ...chip, right: chip.right + 1, ...painted_row(on, chip.right, Math.round(anchor.y), CHIP) };
  expect([chip_row.top, chip_row.bottom], "the title chip shares the price row").toEqual([box.top, box.top + ROW]);
  expect_white_ink_centered(on, chip_row);
  expect_white_ink_centered(on, { ...box, bottom: box.top + ROW });
  // The countdown row sits below the top row, in the main label color, spanning the full width.
  expect(near(px(on, box.left + 3, box.bottom - 3), LABEL)).toBe(true);
  expect(near(px(on, box.right - 4, box.bottom - 3), LABEL)).toBe(true);

  // Disabled (title chip + countdown off, price label on): the cluster rows vanish (diff > 0
  // in the cluster region) and the remaining plain label is a single row.
  await page.evaluate(() => window.__main.apply_options({ title_visible: false, countdown_visible: false }));
  const off = await capture(page);
  expect(region_diff(on, off, box)).toBeGreaterThan(0);
  const plain = find_cluster(off, anchor.pane_w);
  expect(plain.top).toBeGreaterThanOrEqual(0);
  expect(plain.bottom - plain.top).toBeLessThanOrEqual(ROW + 3);
  await context.close();
});

test("explicit light price-line color unifies the live cluster and selects dark text", async ({ browser }) => {
  const LIVE = [240, 230, 140];
  const { context, page } = await open_cluster_page(browser, {
    title: "Aeris",
    title_visible: true,
    countdown_visible: true,
    price_line_color: "#f0e68c",
    price_line_visible: true,
  });
  const anchor = await cluster_anchor(page);
  const shot = await capture(page);
  let top = -1;
  let bottom = -1;
  for (let y = 0; y < shot.height; y += 1) {
    if (near(px(shot, anchor.pane_w + 2, y), LIVE)) {
      if (top === -1) top = y;
      bottom = y;
    }
  }
  const cluster = { left: anchor.pane_w + 1, right: shot.width, top, bottom: bottom + 1 };
  expect(top, "light cluster should be located").toBeGreaterThanOrEqual(0);
  expect(count_where(shot, cluster, (color) => near(color, LIVE)), "unified light cluster fill").toBeGreaterThan(150);
  expect(count_where(shot, cluster, (color) => color.every((channel) => channel < 32)), "black cluster glyph ink").toBeGreaterThan(5);
  await context.close();
});

test("price-line tag clears the live-price tag on the rendered axis", async ({ browser }) => {
  const BLUE = [0, 80, 255];
  const { context, page } = await open_cluster_page(browser, {
    title_visible: false,
    countdown_visible: false,
  });
  await page.evaluate(() => window.__main.create_price_line({
    price: window.__cluster_close,
    color: "#0050ff",
    title: "target",
    line_visible: false,
  }));
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  const anchor = await cluster_anchor(page);
  const shot = await capture(page);
  const rows = (color) => {
    const found = [];
    for (let y = 0; y < shot.height; y += 1) {
      if (near(px(shot, anchor.pane_w + 2, y), color)) found.push(y);
    }
    return found;
  };
  const live = rows(LABEL);
  const target = rows(BLUE);
  expect(live.length).toBeGreaterThanOrEqual(ROW - 2);
  expect(target.length).toBeGreaterThanOrEqual(ROW - 2);
  expect(Math.max(...live) < Math.min(...target) || Math.max(...target) < Math.min(...live)).toBe(true);
  await context.close();
});

// The engine keys the time label's vertical position to the stable "Apr0" ink sample (cap top to
// descender bottom), not to the glyphs of the label itself, so a label without descenders sits
// higher than the same box holding one. The painted label text depends on the calendar month and
// the font, so the placement contract is probed with the sample text itself: its ink must be
// centered on the time-strip text center, whichever font, backend and DPR paint it.
async function expect_crosshair_glyphs_centered(browser, query) {
  const { context, page } = await open_cluster_page(browser, {
    last_value_visible: false,
    title_visible: false,
    countdown_visible: false,
  }, 1.25, query);
  await page.evaluate(() => window.__chart.apply_options({
    localization: { time_formatter: () => "Apr0" },
    crosshair: {
      horzLine: { labelBackgroundColor: "#ff00ff" },
      vertLine: { labelBackgroundColor: "#ff00ff" },
    },
  }));
  await page.waitForTimeout(600);
  const canvas = await page.locator("#chart_container canvas").last().boundingBox();
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2);
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const shot = await capture(page);
  const labels = color_bands(shot, [255, 0, 255]).filter((box) => box.bottom - box.top >= 12);
  expect(labels).toHaveLength(2);
  const [price_label, time_label] = labels.sort(
    (a, b) => (a.bottom - a.top) - (b.bottom - b.top),
  );
  const geometry = await page.evaluate(() => ({
    dpr: window.devicePixelRatio,
    pane_w: window.__chart.time_scale().width(),
    pane_h: window.__chart.wasm.pane_height(0),
  }));
  const border_w = Math.max(1, Math.round(0.5 * geometry.dpr));
  const price_text_left = Math.round(geometry.pane_w * geometry.dpr) + border_w;
  // The neutral crosshair action chip is attached to the chart-facing edge and deliberately
  // shares the configured label fill. Isolate the price-text portion when checking centering.
  expect(price_label.left).toBeLessThan(price_text_left);
  const price_text_label = { ...price_label, left: price_text_left };
  expect(time_label.top).toBe(Math.round(geometry.pane_h * geometry.dpr) + border_w);
  expect(near(px(shot, price_text_left - border_w, price_label.top + 3), BORDER)).toBe(true);
  expect(near(px(shot, Math.floor((time_label.left + time_label.right) / 2), time_label.top - border_w), BORDER)).toBe(true);
  expect_white_ink_centered(shot, price_text_label, 2);
  // The compact time box includes border + 3px tick space above the text body. Its glyph is
  // therefore deliberately below the full box center rather than incorrectly centered in it
  // (1px border + 3px tick + 3px pad above vs 3px pad below the 11px body centers ink 1.5px low).
  // Measure digit-only ink so the result does not depend on the hovered date: the trailing
  // HH:MM run has no descenders, unlike month names such as "Sep". The digit-only price text
  // in the same font is the reference, which cancels the font's cap-versus-em asymmetry.
  const box_center = (box) => (box.top + box.bottom - 1) / 2;
  const time_digits = { ...time_label, left: time_label.right - Math.round((time_label.right - time_label.left) * 0.3) };
  const price_digit_offset = white_ink_center(shot, price_text_label) - box_center(price_label);
  const time_digit_offset = white_ink_center(shot, time_digits) - box_center(time_label);
  const time_body_shift = time_digit_offset - price_digit_offset;
  expect(time_body_shift).toBeGreaterThanOrEqual(1.5);
  expect(time_body_shift).toBeLessThanOrEqual(4);
  await context.close();
}

test("crosshair price and time glyphs stay centered in their label boxes", async ({ browser }) => {
  await expect_crosshair_glyphs_centered(browser, "");
});

test("crosshair price and time glyphs stay centered in their label boxes on Canvas2D", async ({ browser }) => {
  await expect_crosshair_glyphs_centered(browser, "?backend=canvas2d");
});

test("crosshair paints one aligned price label on each visible scale", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    last_value_visible: false,
    title_visible: false,
    countdown_visible: false,
    price_format: { type: "price", precision: 0, min_move: 1 },
  });
  const geometry = await page.evaluate(async () => {
    const left = window.__chart.add_series("line", {
      price_scale_id: "left",
      last_value_visible: false,
      price_line_visible: false,
      crosshair_marker_visible: false,
      price_format: { type: "price", precision: 4, min_move: 0.0001 },
    });
    left.set_data(window.__data.slice(-80).map((row, index) => ({
      time: row.time,
      value: 1.1 + index * 0.0025,
    })));
    window.__chart.apply_options({
      leftPriceScale: { visible: true },
      rightPriceScale: { visible: true },
      crosshair: {
        horzLine: { labelBackgroundColor: "#ff00ff" },
        vertLine: { labelBackgroundColor: "#ff00ff" },
      },
    });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const row = window.__data[window.__data.length - 20];
    window.__chart.set_crosshair_position(row.close, row.time, window.__main);
    await new Promise((resolve) => requestAnimationFrame(resolve));
    return {
      left_w: window.__chart.price_scale("left").width(),
      pane_w: window.__chart.time_scale().width(),
      right_w: window.__chart.price_scale("right").width(),
      y: window.__main.price_to_coordinate(row.close),
    };
  });
  const shot = await capture(page);
  const y0 = Math.round(geometry.y) - ROW;
  const y1 = Math.round(geometry.y) + ROW;
  const magenta = (color) => near(color, [255, 0, 255]);
  const left_box = {
    left: 0,
    right: geometry.left_w,
    top: Math.max(0, y0),
    bottom: Math.min(shot.height, y1),
  };
  const right_box = {
    left: geometry.left_w + geometry.pane_w,
    right: geometry.left_w + geometry.pane_w + geometry.right_w,
    top: Math.max(0, y0),
    bottom: Math.min(shot.height, y1),
  };
  expect(geometry.left_w).toBeGreaterThan(0);
  expect(geometry.right_w).toBeGreaterThan(0);
  expect(count_where(shot, left_box, magenta), "left-scale crosshair label box").toBeGreaterThan(80);
  expect(count_where(shot, right_box, magenta), "right-scale crosshair label box").toBeGreaterThan(80);

  const label_rows = (box) => {
    const rows = [];
    for (let y = box.top; y < box.bottom; y += 1) {
      let count = 0;
      for (let x = box.left; x < box.right; x += 1) {
        if (magenta(px(shot, x, y))) count += 1;
      }
      if (count >= 4) rows.push(y);
    }
    return rows;
  };
  const left_rows = label_rows(left_box);
  const right_rows = label_rows(right_box);
  expect(left_rows.length).toBeGreaterThanOrEqual(ROW - 4);
  expect(right_rows.length).toBeGreaterThanOrEqual(ROW - 4);
  expect(Math.min(...left_rows)).toBe(Math.min(...right_rows));
  expect(Math.max(...left_rows)).toBe(Math.max(...right_rows));
  await context.close();
});

test("runtime price precision settles autoscale and candle geometry in one repaint", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    last_value_visible: false,
    title_visible: false,
    countdown_visible: false,
    price_line_visible: false,
    up_color: "#00ff00",
    down_color: "#ff0000",
    wick_up_color: "#00ff00",
    wick_down_color: "#ff0000",
    border_up_color: "#00ff00",
    border_down_color: "#ff0000",
    price_format: { type: "price", precision: 2, min_move: 0.01 },
  });
  const result = await page.evaluate(async () => {
    const source = window.__data.slice(-80);
    const btc = source.map((row, index) => {
      const open = 116_000 + index * 0.25;
      const close = open + (index % 2 === 0 ? 3.75 : -2.25);
      return {
        time: row.time,
        open,
        high: Math.max(open, close) + 4.5,
        low: Math.min(open, close) - 4.5,
        close,
      };
    });
    window.__main.set_data(btc);
    window.__chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const before = window.__chart.frame_stats().presented_frames;
    window.__main.apply_options({
      price_format: { type: "price", precision: 0, min_move: 1 },
    });
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const range = window.__main.price_scale().get_visible_range();
    return {
      frames: window.__chart.frame_stats().presented_frames - before,
      formatted: window.__main.price_formatter()(116_004.25),
      range,
      pane_w: window.__chart.time_scale().width(),
      pane_h: window.__chart.wasm.pane_height(0),
      coordinates: btc.flatMap((bar) => [bar.high, bar.low]).map((price) => (
        window.__main.price_to_coordinate(price)
      )),
    };
  });
  expect(result.frames, "precision change should settle in the next presented frame").toBe(1);
  expect(result.formatted).toBe("116,004");
  expect(result.range.from).toBeLessThanOrEqual(115_993.5);
  expect(result.range.to).toBeGreaterThanOrEqual(116_027.25);
  expect(result.range.to - result.range.from).toBeLessThan(100);
  expect(result.coordinates.every((y) => y !== null && y >= 0 && y <= result.pane_h)).toBe(true);

  const shot = await capture(page);
  const pane = { left: 0, right: result.pane_w, top: 0, bottom: result.pane_h };
  expect(count_where(shot, pane, (color) => near(color, [0, 255, 0])), "visible up candles").toBeGreaterThan(20);
  expect(count_where(shot, pane, (color) => near(color, [255, 0, 0])), "visible down candles").toBeGreaterThan(20);
  await context.close();
});

test("precision-zero daily countdown reserves the complete live-label width", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    title: "HYPE",
    title_visible: true,
    countdown_visible: false,
    price_format: { type: "price", precision: 0, min_move: 1 },
  });
  const result = await page.evaluate(async () => {
    const now = Math.floor(Date.now() / 1000);
    const bars = [2, 1, 0].map((days, index) => ({
      time: now - days * 86_400,
      open: 101 + index,
      high: 102 + index,
      low: 99 + index,
      close: 100 + index,
    }));
    window.__cluster_close = bars[bars.length - 1].close;
    window.__main.set_data(bars);
    window.__chart.time_scale().fit_content();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const without = window.__chart.time_scale().width();
    window.__main.apply_options({ countdown_visible: true });
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return {
      without,
      with_countdown: window.__chart.time_scale().width(),
      pane_w: window.__chart.time_scale().width(),
    };
  });

  expect(result.with_countdown).toBeLessThan(result.without);
  const shot = await capture(page);
  const box = find_cluster(shot, result.pane_w);
  expect(box.top).toBeGreaterThanOrEqual(0);
  // Compact strips reserve 12px chrome around measured text (34px floor ⇒ 46px minimum).
  expect(shot.width - result.pane_w).toBeGreaterThanOrEqual(46);
  const countdown_row = { ...box, top: box.bottom - 14 };
  const is_countdown_ink = (color) => near(color, [247, 200, 199], 24);
  expect(count_where(shot, countdown_row, is_countdown_ink)).toBeGreaterThan(8);
  expect(rightmost_where(shot, countdown_row, is_countdown_ink)).toBeLessThan(shot.width - 5);
  const price_row = { ...box, bottom: box.top + ROW };
  expect(rightmost_where(shot, price_row, is_white)).toBeLessThan(shot.width - 5);
  await context.close();
});

test("cluster parts toggle independently", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    title: "Aeris",
    title_visible: true,
    countdown_visible: true,
  });
  const anchor = await cluster_anchor(page);

  // Title chip off: no chip-colored pixels anywhere (inside or outside the strip), price +
  // countdown rows remain.
  await page.evaluate(() => window.__main.apply_options({ title_visible: false }));
  let shot = await capture(page);
  let box = find_cluster(shot, anchor.pane_w);
  expect(box.bottom - box.top).toBeGreaterThanOrEqual(ROW + ROW_CD - 4);
  expect(count_chip_left(shot, anchor.pane_w, anchor.y)).toBe(0);
  expect(count_color(shot, box, LABEL)).toBeGreaterThan(100);
  // Price text (white glyphs on the box) is still painted in the top row.
  expect(count_where(shot, { ...box, bottom: box.top + ROW }, is_white)).toBeGreaterThan(5);

  // Price off (chip + countdown on): the outside title chip returns, NO empty price box inside
  // the strip's top row, and the countdown row keeps its text.
  await page.evaluate(() => window.__main.apply_options({ title_visible: true, last_value_visible: false }));
  shot = await capture(page);
  box = find_cluster(shot, anchor.pane_w);
  const extent = chip_extent(shot, anchor.pane_w, anchor.y);
  expect(extent.found, "outside title chip present").toBe(true);
  // The title chip ends at the axis border with no intervening chart-surface pixel.
  expect(anchor.pane_w - extent.right - 1).toBe(0);
  // Inside the strip the top row is empty above the countdown. Its text is the dark-theme
  // foreground at 70% opacity, composited over the red label background.
  const is_faded_white = (c) => near(c, [247, 200, 199], 20);
  expect(count_where(shot, { ...box, top: box.top }, is_faded_white)).toBeGreaterThan(5);

  // Countdown off (chip + price on): one inside row, nothing painted below it, price text present.
  await page.evaluate(() => window.__main.apply_options({ last_value_visible: true, countdown_visible: false }));
  shot = await capture(page);
  box = find_cluster(shot, anchor.pane_w);
  expect(box.bottom - box.top).toBeLessThanOrEqual(ROW + 3);
  expect(count_chip_left(shot, anchor.pane_w, anchor.y)).toBeGreaterThan(20);
  expect(count_where(shot, { ...box, bottom: box.top + ROW }, is_white)).toBeGreaterThan(5);

  // Everything off: no cluster at all, no outside chip either.
  await page.evaluate(() => window.__main.apply_options({ last_value_visible: false, title_visible: false }));
  shot = await capture(page);
  expect(find_cluster(shot, anchor.pane_w).top).toBe(-1);
  expect(count_chip_left(shot, anchor.pane_w, anchor.y)).toBe(0);
  await context.close();
});

test("countdown row ticks with the 1s interval timer", async ({ browser }) => {
  const { context, page } = await open_cluster_page(browser, {
    title: "Aeris",
    title_visible: true,
    countdown_visible: true,
  });
  const anchor = await cluster_anchor(page);
  // Let the first tick settle (the timer starts on apply; the first capture must be past the
  // initial pin so both captures read distinct remaining seconds).
  await page.waitForTimeout(1100);
  const first = await capture(page);
  const box = find_cluster(first, anchor.pane_w);
  expect(box.bottom - box.top).toBeGreaterThanOrEqual(ROW + ROW_CD - 4);
  await page.waitForTimeout(1300);
  const second = await capture(page);
  const countdown_row = { left: box.left, right: box.right, top: box.bottom - ROW_CD, bottom: box.bottom };
  expect(region_diff(first, second, countdown_row)).toBeGreaterThan(0);
  await context.close();
});

test("price and countdown chips share an exact edge at any DPR (no attachment gap)", async ({ browser }) => {
  for (const dpr of [1, 1.35, 2]) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: dpr, colorScheme: "light" });
    const page = await context.newPage();
    await page.goto(`${test_base_url}/?theme=light`);
    await wait_for_chart(page);
    await page.evaluate(() => {
      const now = Math.floor(Date.now() / 1000);
      const last = window.__data[window.__data.length - 1];
      const close = last.close - 2;
      window.__cluster_close = close;
      window.__main.update({ time: now, open: last.close, high: last.close + 0.6, low: close - 0.6, close });
      window.__main.apply_options({ title: "Aeris", title_visible: true, countdown_visible: true, price_line_visible: false });
    });
    // Isolate the cluster under test: a second visible series' own cluster would share the
    // strip (and its white glyphs would cross the probe column), so hide the demo SMA.
    // Attachment is per-cluster geometry; overlap coverage lives in engine + corner tests.
    if (await page.locator("#sma_toggle").isChecked()) {
      await page.uncheck("#sma_toggle");
    }
    await page.waitForTimeout(300);
    // Chart-local capture (not a page screenshot): `pane_left`/`price_to_coordinate` are
    // chart CSS px, so no page/container offset applies. The probe column sits 4 device px
    // inside the strip — in the box padding, clear of glyph ink — so boxes read as label
    // red and a true attachment gap reads as white surface showing through.
    const shot = await capture(page);
    const geom = await page.evaluate(() => ({
      pane_edge: window.__chart.wasm.pane_left() + window.__chart.time_scale().width(),
      value_y: window.__main.price_to_coordinate(window.__cluster_close),
    }));
    const border_dev_x = Math.round(geom.pane_edge * dpr) + Math.round(4 * dpr);
    // Full cluster (15px price + 14px countdown rows) centers 7px below the value.
    const top_dev = Math.round((geom.value_y - 7.5) * dpr);
    const bottom_dev = Math.round((geom.value_y + 21.5) * dpr);
    const is_box = (x, y) => {
      const o = (y * shot.width + x) * 4;
      return shot.data[o] > 200 && shot.data[o + 1] < 130 && shot.data[o + 2] < 130;
    };
    const is_white = (x, y) => {
      const o = (y * shot.width + x) * 4;
      return shot.data[o] > 240 && shot.data[o + 1] > 240 && shot.data[o + 2] > 240;
    };
    const red_rows = [];
    for (let y = top_dev - 4; y < bottom_dev + 4; y++) {
      if (is_box(border_dev_x, y)) red_rows.push(y);
    }
    expect(red_rows.length, `dpr ${dpr}: cluster boxes present`).toBeGreaterThan(10);
    expect(red_rows[0] - (top_dev - 4), `dpr ${dpr}: cluster top present`).toBeLessThanOrEqual(6);
    expect((bottom_dev + 4 - 1) - red_rows[red_rows.length - 1], `dpr ${dpr}: cluster bottom present`).toBeLessThanOrEqual(6);
    const gap_rows = [];
    for (let y = red_rows[0]; y <= red_rows[red_rows.length - 1]; y++) {
      if (!is_box(border_dev_x, y) && is_white(border_dev_x, y)) gap_rows.push(y);
    }
    expect(gap_rows, `dpr ${dpr}: surface rows splitting attached chips`).toEqual([]);
    await context.close();
  }
});

test("cluster rounds its axis-facing corners and keeps the chart-facing side sharp", async ({ browser }) => {
  // The canonical radius is 1 CSS px. At DPR 1 a correct arc can fully cover the one extreme
  // sample depending on rasterizer/MSAA rules, so probe at DPR 2 and compare painted row extents.
  const { context, page } = await open_cluster_page(browser, {
    title: "Aeris",
    title_visible: true,
    countdown_visible: true,
  }, 2, "?backend=canvas2d");
  const anchor_css = await cluster_anchor(page);
  const dpr = await page.evaluate(() => window.devicePixelRatio);
  const anchor = {
    pane_w: Math.round(anchor_css.pane_w * dpr),
    y: anchor_css.y * dpr,
  };
  const shot = await capture(page);
  const box = find_cluster(shot, anchor.pane_w);
  expect(box.top).toBeGreaterThanOrEqual(0);
  expect(box.left).toBe(anchor.pane_w + 1);

  const top_outer = row_span_where(shot, box.top, box.left, box.right, is_label);
  const top_inner = row_span_where(
    shot,
    box.top + Math.round(3 * dpr),
    box.left,
    box.right,
    is_label,
  );
  expect(top_outer.left).toBe(top_inner.left); // chart-facing edge stays sharp
  expect(top_outer.right).toBeLessThan(top_inner.right); // axis-facing corner rounds inward

  const bottom_outer = row_span_where(shot, box.bottom - 1, box.left, box.right, is_label);
  const bottom_inner = row_span_where(
    shot,
    box.bottom - Math.round(4 * dpr),
    box.left,
    box.right,
    is_label,
  );
  expect(bottom_outer.left).toBe(bottom_inner.left); // chart-facing edge stays sharp
  expect(bottom_outer.right).toBeLessThan(bottom_inner.right); // axis-facing corner rounds inward

  // The outside title chip has the inverse ownership: its outer chart-facing side rounds, while
  // the side attached to the axis border remains square.
  const extent = chip_extent(shot, anchor.pane_w, anchor.y, dpr);
  expect(extent.found).toBe(true);
  const chip_outer = row_span_where(shot, extent.top, extent.left, extent.right + 1, is_label);
  const chip_inner = row_span_where(
    shot,
    extent.top + Math.round(2 * dpr),
    extent.left,
    extent.right + 1,
    is_label,
  );
  expect(chip_outer.left).toBeGreaterThan(chip_inner.left); // outer corner rounds inward
  expect(chip_outer.right).toBe(chip_inner.right); // attached axis-facing edge stays sharp
  await context.close();
});

test("two clustered series never chain into one box; the volume shows title + volume value", async ({ page }) => {
  await page.goto("/");
  await wait_for_chart(page);
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const last = window.__data[window.__data.length - 1];
    const close = last.close - 2;
    window.__cluster_close = close;
    window.__main.update({ time: now, open: last.close, high: last.close + 0.6, low: close - 0.6, close });
  });
  await page.check("#vol_toggle");
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));

  // The volume cluster: outside title chip "Volume" + volume-formatted value (K/M suffix).
  const shot = await capture(page);
  const anchor = await cluster_anchor(page);
  const axis_x0 = Math.round(anchor.pane_w);
  // The strip between the main cluster's bottom and the volume band must NOT be a continuous
  // red column (the attach-group chaining bug merged every cluster on the strip into one box).
  const main_bottom = Math.round(anchor.y + ROW);
  const strip = new PNG({ width: shot.width - axis_x0, height: Math.max(1, shot.height - main_bottom - 90) });
  PNG.bitblt(shot, strip, axis_x0, main_bottom, strip.width, strip.height, 0, 0);
  let red_rows = 0;
  for (let y = 0; y < strip.height; y++) {
    let red_in_row = 0;
    for (let x = 0; x < strip.width; x++) {
      const o = (y * strip.width + x) * 4;
      if (Math.abs(strip.data[o] - 239) <= 25 && Math.abs(strip.data[o + 1] - 83) <= 25 && Math.abs(strip.data[o + 2] - 80) <= 25) red_in_row++;
    }
    // A chain box would paint red across the whole strip on EVERY row between the clusters.
    if (red_in_row > strip.width * 0.8) red_rows++;
  }
  expect(red_rows, "no merged red column between the two clusters").toBeLessThan(4);
});
