import { test, expect } from "@playwright/test";
import { PNG } from "pngjs";

// `crosshair.shadeRight` through the public TS API: with the veil on, the pane region right of the
// hovered bar takes the configured tint while the region left of it is untouched. This is a
// region-color assertion on the executed frame (the screenshot runs the retained frame through the
// Canvas2D executor), never a cross-backend hash.

async function wait_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function settle(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function capture(page) {
  const data_url = await page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  return PNG.sync.read(Buffer.from(data_url.split(",")[1], "base64"));
}

function px(png, x, y) {
  const o = (y * png.width + x) * 4;
  return [png.data[o], png.data[o + 1], png.data[o + 2]];
}

/** Device-pixel sample columns two bar spacings left and right of the pointer, inside pane 0. */
async function sample_geometry(page, pointer_css_x, pointer_css_y) {
  return page.evaluate(([cx, cy]) => {
    const dpr = window.devicePixelRatio || 1;
    const scale = window.__chart.time_scale();
    const data = window.__main.data();
    const spacing = Math.abs(
      scale.logical_to_coordinate(data.length - 1) - scale.logical_to_coordinate(data.length - 2),
    );
    const pane_h = window.__chart.wasm.pane_height(0);
    return {
      left: Math.round((cx - 2 * spacing) * dpr),
      right: Math.round((cx + 2 * spacing) * dpr),
      edge: Math.round(scale.width() * dpr) - 2,
      // Rows away from the horizontal crosshair line (which passes through the pointer row).
      rows: [0.15, 0.3, 0.7, 0.85]
        .map((f) => Math.round(pane_h * f * dpr))
        .filter((y) => Math.abs(y - cy * dpr) > 4 * dpr),
    };
  }, [pointer_css_x, pointer_css_y]);
}

const SHADE = "rgba(255, 0, 0, 0.5)";

/**
 * A 50% red tint over any color raises red (unless already saturated) and lowers green and blue
 * (unless already zero); at least one channel must actually move, so an unpainted frame never passes.
 */
function tinted(before, after) {
  const moved = after[0] !== before[0] || after[1] !== before[1] || after[2] !== before[2];
  return (
    moved &&
    (after[0] > before[0] || before[0] === 255) &&
    (after[1] < before[1] || before[1] === 0) &&
    (after[2] < before[2] || before[2] === 0)
  );
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/");
  await wait_chart(page);
});

test("the veil tints the pane right of the hovered bar and nothing left of it", async ({ page }) => {
  const canvas = await page.locator("#chart_container canvas").last().boundingBox();
  const pointer = { x: canvas.x + canvas.width * 0.4, y: canvas.y + canvas.height * 0.5 };
  await page.mouse.move(pointer.x, pointer.y);
  await settle(page);
  const before = await capture(page);

  await page.evaluate((color) => window.__chart.apply_options({
    crosshair: { shadeRight: { visible: true, color } },
  }), SHADE);
  await settle(page);
  const after = await capture(page);
  expect(after.width).toBe(before.width);
  expect(after.height).toBe(before.height);

  const geometry = await sample_geometry(page, canvas.width * 0.4, canvas.height * 0.5);
  expect(geometry.rows.length).toBeGreaterThanOrEqual(3);
  for (const y of geometry.rows) {
    expect(tinted(px(before, geometry.right, y), px(after, geometry.right, y)),
      `right of the bar at y=${y} is tinted`).toBe(true);
    expect(tinted(px(before, geometry.edge, y), px(after, geometry.edge, y)),
      `the pane's right edge at y=${y} is tinted`).toBe(true);
    expect(px(after, geometry.left, y), `left of the bar at y=${y} is unchanged`)
      .toEqual(px(before, geometry.left, y));
  }

  // Turning the veil off restores the frame.
  await page.evaluate(() => window.__chart.apply_options({ crosshair: { shadeRight: { visible: false } } }));
  await settle(page);
  const off = await capture(page);
  for (const y of geometry.rows) {
    expect(px(off, geometry.right, y)).toEqual(px(before, geometry.right, y));
  }
});

test("the option round-trips and a style reset restores the color but keeps visible", async ({ page }) => {
  const result = await page.evaluate((color) => {
    const chart = window.__chart;
    const initial = chart.options().crosshair.shadeRight;
    chart.apply_options({ crosshair: { shadeRight: { visible: true, color } } });
    const applied = chart.options().crosshair.shadeRight;
    chart.reset_style_to_defaults();
    const reset = chart.options().crosshair.shadeRight;
    return { initial, applied, reset };
  }, SHADE);
  expect(result.initial).toEqual({ visible: false, color: "rgba(74, 74, 74, 0.12)" });
  expect(result.applied).toEqual({ visible: true, color: SHADE });
  expect(result.reset).toEqual({ visible: true, color: "rgba(74, 74, 74, 0.12)" });
});
