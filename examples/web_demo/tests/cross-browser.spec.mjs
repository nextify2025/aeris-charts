// Cross-browser reach: the WebGPU pane is Chromium+SwiftShader only (see backend-parity.spec.mjs),
// so this suite verifies the shared **Canvas2D fallback** actually initializes and renders on every
// engine in the project matrix (Chromium, Firefox, WebKit). Pixel-exact parity is deliberately not
// asserted here — each engine's Canvas2D rasterizer differs — only that a real chart is drawn.

import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";
import { PNG } from "pngjs";

test.beforeEach(async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

test("Canvas2D fallback initializes and renders a chart", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);

  // The forced fallback must actually select Canvas2D on this engine.
  expect(await page.evaluate(() => window.__chart.backend())).toBe("canvas2d");

  // A rendered chart (candles, grid, axis labels, background) shows many distinct colors; a blank
  // or failed canvas would show ~1. This is engine-agnostic and robust to rasterizer differences.
  const buffer = await page.locator("#chart_container").screenshot();
  const png = PNG.sync.read(buffer);
  const colors = new Set();
  for (let offset = 0; offset < png.data.length; offset += 4) {
    colors.add((png.data[offset] << 16) | (png.data[offset + 1] << 8) | png.data[offset + 2]);
  }
  expect(colors.size, `expected a rendered chart, saw ${colors.size} distinct colors`).toBeGreaterThan(20);
});
