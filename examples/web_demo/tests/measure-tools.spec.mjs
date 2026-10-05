import { test, expect } from "@playwright/test";

// The measuring tools and the Shift-click quick measure through real pointer and keyboard input.
// Geometry, snapping, statistics, and colors are engine-owned; these tests prove both browser
// backends route the same gestures into that one implementation. The quick measure is a transient
// date-and-price range, so it prints the same statistics lines as the committed tool: a price line
// (`<change>  <percent>  <ticks>`) and a time line (`<bars>  <duration>`).

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  // Statistic labels reach both backends as whole text runs (WebGPU rasterizes its glyph atlas
  // through the same Canvas2D call), so recording them is backend-neutral.
  await page.addInitScript(() => {
    const original = CanvasRenderingContext2D.prototype.fillText;
    window.__measure_text = [];
    CanvasRenderingContext2D.prototype.fillText = function (text, ...args) {
      if (/ bars| ticks|%/.test(String(text))) window.__measure_text.push(String(text));
      return original.call(this, text, ...args);
    };
  });
});

async function open_chart(page, backend) {
  await page.goto(`/?backend=${backend}&forceFallbackAdapter=1`);
  await page.waitForFunction(() => window.__main && window.__chart?.time_scale().get_visible_logical_range());
  await page.evaluate(() => document.fonts.ready);
  await frames(page);
  return page.evaluate(() => {
    const box = document.getElementById("chart_container").getBoundingClientRect();
    const left = box.left + window.__chart.wasm.pane_left();
    const point = (fx, fy) => ({ x: left + box.width * fx, y: box.top + box.height * fy });
    return {
      backend: window.__chart.backend(),
      start: point(0.3, 0.35),
      below: point(0.55, 0.6),
      above: point(0.5, 0.15),
    };
  });
}

async function frames(page) {
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

// WebGPU caches each rasterized run in its glyph atlas, so a label is recorded the first time it
// paints. `text` is therefore the cumulative log since the last `reset_text`.
async function measure_state(page) {
  await frames(page);
  return page.evaluate(() => ({
    active: window.__chart.wasm.measure_active(),
    points: JSON.parse(window.__chart.wasm.measure_points_json()),
    drawings: window.__chart.drawings().length,
    text: [...window.__measure_text],
  }));
}

async function reset_text(page) {
  await page.evaluate(() => { window.__measure_text.length = 0; });
}

for (const backend of ["auto", "canvas2d"]) {
  test(`shift-drag measures falls and rises, then dismisses on the next click (${backend})`, async ({ page }) => {
    const at = await open_chart(page, backend);
    expect(at.backend).toBe(backend === "auto" ? "webgpu" : "canvas2d");
    await page.evaluate(() => {
      window.__pane_clicks = 0;
      window.__chart.subscribe_click(() => { window.__pane_clicks += 1; });
    });

    // Shift + press, pull down, release: a negative measurement frozen at the release point.
    await page.mouse.move(at.start.x, at.start.y);
    await page.keyboard.down("Shift");
    await page.mouse.down();
    await page.mouse.move(at.below.x, at.below.y, { steps: 8 });
    await page.mouse.up();
    await page.keyboard.up("Shift");
    let state = await measure_state(page);
    expect(state.active).toBe(true);
    expect(state.drawings).toBe(0);
    const [start, end] = state.points;
    expect(end.price).toBeLessThan(start.price);
    expect(end.logical).toBeGreaterThan(start.logical);
    expect(Number.isInteger(start.logical) && Number.isInteger(end.logical)).toBe(true);
    expect(state.text.some((text) => /^−[\d.,]+ {2}-[\d.,]+%/.test(text))).toBe(true);
    expect(state.text.some((text) => /^\d+ bars {2}\S/.test(text))).toBe(true);

    // Moving no longer changes a frozen measure; the next click only dismisses it.
    await page.mouse.move(at.above.x, at.above.y);
    expect((await measure_state(page)).points).toEqual([start, end]);
    await page.mouse.click(at.above.x, at.above.y);
    state = await measure_state(page);
    expect(state.active).toBe(false);
    expect(await page.evaluate(() => window.__pane_clicks)).toBe(0);

    // Shift + click, move upward without a button, click: a positive measurement.
    await reset_text(page);
    await page.mouse.move(at.start.x, at.start.y);
    await page.keyboard.down("Shift");
    await page.mouse.click(at.start.x, at.start.y);
    await page.keyboard.up("Shift");
    await page.mouse.move(at.above.x, at.above.y, { steps: 6 });
    const following = await measure_state(page);
    expect(following.points[1].price).toBeGreaterThan(following.points[0].price);
    await page.mouse.click(at.above.x, at.above.y);
    await page.mouse.move(at.below.x, at.below.y);
    state = await measure_state(page);
    expect(state.active).toBe(true);
    expect(state.points).toEqual(following.points);
    expect(state.text.some((text) => /^\+[\d.,]+ {2}\+[\d.,]+%/.test(text))).toBe(true);

    // Escape dismisses it, and nothing was ever committed or clicked through.
    await page.keyboard.press("Escape");
    state = await measure_state(page);
    expect(state.active).toBe(false);
    expect(state.drawings).toBe(0);
    expect(await page.evaluate(() => window.__pane_clicks)).toBe(0);
  });

  test(`price, date, and date-and-price range tools place snapped editable drawings (${backend})`, async ({ page }) => {
    const at = await open_chart(page, backend);
    for (const [index, kind] of ["price_range", "date_range", "date_price_range"].entries()) {
      await reset_text(page);
      await page.click(`#drawings_group [data-tool="${kind}"]`);
      // Each tool ends at a different point so its labels are new text runs.
      const end = { x: at.below.x + index * 23, y: at.below.y + index * 17 };
      // Shift during placement stays the tool's own modifier rather than starting a measure.
      await page.keyboard.down("Shift");
      await page.mouse.click(at.start.x, at.start.y);
      await page.keyboard.up("Shift");
      await page.mouse.move(end.x, end.y, { steps: 4 });
      await page.mouse.click(end.x, end.y);
      const drawing = await page.evaluate(() => {
        const drawing = window.__chart.drawings().at(-1);
        return {
          kind: drawing.kind(),
          points: drawing.points(),
          tick: window.__main.options().price_format.min_move,
          measuring: window.__chart.wasm.measure_active(),
          tool: window.__chart.active_drawing_tool(),
        };
      });
      expect(drawing.kind).toBe(kind);
      expect(drawing.measuring).toBe(false);
      expect(drawing.tool).toBe(null);
      expect(drawing.points).toHaveLength(2);
      for (const point of drawing.points) {
        expect(Number.isInteger(point.logical)).toBe(true);
        expect(point.price / drawing.tick).toBeCloseTo(Math.round(point.price / drawing.tick), 6);
      }
      const state = await measure_state(page);
      // The end sits below the start, so the price line reads as a fall; ASCII signs follow the
      // percent and ticks, the price keeps the formatter's typographic minus.
      const expected = kind === "date_range" ? /^\d+ bars {2}\S/ : /^−[\d.,]+ {2}-[\d.,]+%/;
      expect(state.text.some((text) => expected.test(text)), `${kind}: ${state.text}`).toBe(true);
      await page.keyboard.press("Escape");
    }
  });
}
