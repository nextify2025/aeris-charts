import { test, expect } from "@playwright/test";

// Real touch input (Chromium DevTools `Input.dispatchTouchEvent`, so the browser builds the
// Touch Events, pointer events, and compatibility-click decisions itself) through the browser
// recognizer's touch path (`gestures.ts` touch handlers): one-finger pan, direction arbitration
// against the page, long-press crosshair tracking and both exit modes, a centroid-anchored pinch
// that ignores a third finger and honors its switch, exact tap/double-tap callback counts, and a
// real touchcancel rolling back a drawing drag. Timers (long press, tap window) run on the
// Playwright clock so no assertion depends on machine speed.

const OVERLAY = "#chart_container canvas:last-of-type";

test.use({ hasTouch: true });

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
  await page.evaluate(() => {
    // A coast after release would move the view on the machine's own clock.
    window.__chart.apply_options({ kinetic_scroll: { touch: false } });
    window.__clicks = 0;
    window.__double_clicks = 0;
    window.__crosshair = [];
    window.__touchmoves = [];
    window.__chart.subscribe_click(() => { window.__clicks += 1; });
    window.__chart.subscribe_dbl_click(() => { window.__double_clicks += 1; });
    window.__chart.subscribe_crosshair_move((params) => window.__crosshair.push(params.point));
    document.addEventListener("touchmove", (event) => window.__touchmoves.push(event.defaultPrevented));
  });
  // Freeze the page clock: the long-press and tap-window timers then advance only through an
  // explicit `page.clock.runFor`, never by however long the machine takes between touch calls.
  await page.clock.install();
  await page.clock.pauseAt((await page.evaluate(() => Date.now())) + 1000);
}

/** Client-space pane geometry: pane x = 0 sits at `left`, chart y = 0 at `top`. */
async function geometry(page) {
  return page.evaluate((selector) => {
    const rect = document.querySelector(selector).getBoundingClientRect();
    const wasm = window.__chart.wasm;
    return {
      left: rect.left + wasm.pane_left(),
      top: rect.top,
      width: wasm.time_scale_width(),
      pane_height: wasm.pane_height(0),
    };
  }, OVERLAY);
}

async function touchscreen(page) {
  const cdp = await page.context().newCDPSession(page);
  const send = (type, touchPoints) => cdp.send("Input.dispatchTouchEvent", { type, touchPoints });
  return {
    start: (...points) => send("touchStart", points),
    move: (...points) => send("touchMove", points),
    end: () => send("touchEnd", []),
    cancel: () => send("touchCancel", []),
    async tap(point) {
      await send("touchStart", [point]);
      await send("touchEnd", []);
    },
  };
}

const view = (page) => page.evaluate(() => ({
  spacing: window.__chart.wasm.bar_spacing(),
  offset: window.__chart.wasm.scroll_position(),
}));

const last_crosshair = (page) => page.evaluate(() => window.__crosshair.at(-1) ?? null);

test("a one-finger horizontal swipe pans the time scale by exactly the dragged distance", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const x = Math.round(g.left + g.width * 0.6);
  const y = Math.round(g.top + g.pane_height * 0.4);
  const before = await view(page);

  await touch.start({ x, y, id: 1 });
  for (let step = 1; step <= 6; step += 1) await touch.move({ x: x - step * 20, y, id: 1 });
  await touch.end();

  // The first move crosses the 5 px slop and opens the scroll; movement starts on the next sample,
  // so the five following 20 px samples scroll 100 px.
  const after = await view(page);
  expect(after.spacing).toBe(before.spacing);
  expect(after.offset - before.offset).toBeCloseTo(100 / before.spacing, 6);
  expect(await page.evaluate(() => window.__touchmoves)).toEqual(Array(6).fill(true));
  expect(await page.evaluate(() => window.__clicks)).toBe(0);
  const crosshair = await page.evaluate(() => window.__crosshair);
  expect(crosshair.length).toBeGreaterThan(0);
  expect(crosshair.at(-1), "a lifted pan finger leaves no crosshair").toBeNull();
});

test("touch direction arbitration leaves a gated swipe direction to the page", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const x = Math.round(g.left + g.width * 0.6);
  const y = Math.round(g.top + g.pane_height * 0.8);
  // A mostly vertical swipe with a 30 px horizontal component.
  const swipe = async () => {
    await page.evaluate(() => { window.__touchmoves = []; });
    await touch.start({ x, y, id: 1 });
    for (let step = 1; step <= 10; step += 1) await touch.move({ x: x - step * 3, y: y - step * 20, id: 1 });
    await touch.end();
    return page.evaluate(() => window.__touchmoves);
  };

  // Vertical touch drag on: the chart owns the swipe and pans by its horizontal part. The first
  // sample opens the scroll, so 27 px of the 30 px move the view.
  let before = await view(page);
  expect(await swipe()).toEqual(Array(10).fill(true));
  expect((await view(page)).offset - before.offset).toBeCloseTo(27 / before.spacing, 6);

  await page.evaluate(() => window.__chart.apply_options({ handle_scroll: { vert_touch_drag: false } }));
  before = await view(page);
  const moves = await swipe();
  expect(moves.length).toBeGreaterThan(0);
  expect(moves.every((prevented) => prevented === false), "the page keeps the vertical swipe").toBe(true);
  expect(await view(page)).toEqual(before);

  // The same arbitration holds horizontally, with the vertical gate open again so only the
  // horizontal switch can leave this swipe to the page.
  await page.evaluate(() => window.__chart.apply_options({
    handle_scroll: { vert_touch_drag: true, horz_touch_drag: false },
  }));
  await page.evaluate(() => { window.__touchmoves = []; });
  await touch.start({ x, y, id: 2 });
  for (let step = 1; step <= 6; step += 1) await touch.move({ x: x - step * 20, y, id: 2 });
  await touch.end();
  const horizontal = await page.evaluate(() => window.__touchmoves);
  expect(horizontal.length).toBeGreaterThan(0);
  expect(horizontal.every((prevented) => prevented === false), "the page keeps the horizontal swipe").toBe(true);
  expect(await view(page)).toEqual(before);
});

test("a long press starts crosshair tracking that a drag steers and the next tap ends", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const press = { x: Math.round(g.left + g.width * 0.5), y: Math.round(g.top + g.pane_height * 0.5) };
  const before = await view(page);

  await touch.start({ ...press, id: 1 });
  await page.clock.runFor(239);
  expect(await last_crosshair(page), "no crosshair before the long-press delay").toBeNull();
  await page.clock.runFor(1);
  expect(await last_crosshair(page)).toEqual({ x: press.x - g.left, y: press.y - g.top });

  // Tracking moves the crosshair by the finger's offset instead of panning.
  await touch.move({ x: press.x + 30, y: press.y + 12, id: 1 });
  await touch.move({ x: press.x + 60, y: press.y + 24, id: 1 });
  expect(await last_crosshair(page)).toEqual({ x: press.x + 60 - g.left, y: press.y + 24 - g.top });
  await touch.end();
  expect(await view(page)).toEqual(before);
  expect(await page.evaluate(() => window.__clicks), "a long press is not a tap").toBe(0);
  // `on_next_tap` (the default) keeps tracking after the finger lifts...
  expect(await last_crosshair(page)).toEqual({ x: press.x + 60 - g.left, y: press.y + 24 - g.top });

  // ...and a following drag steers the crosshair relative to where it was left.
  await page.clock.runFor(600);
  await touch.start({ x: press.x - 100, y: press.y, id: 2 });
  await touch.move({ x: press.x - 140, y: press.y + 10, id: 2 });
  expect(await last_crosshair(page)).toEqual({ x: press.x + 20 - g.left, y: press.y + 34 - g.top });
  await touch.end();
  expect(await view(page)).toEqual(before);

  // The next tap ends tracking.
  await page.clock.runFor(600);
  await touch.tap({ x: press.x - 200, y: press.y, id: 3 });
  expect(await last_crosshair(page)).toBeNull();

  // Tracking is over: a swipe pans again.
  await page.clock.runFor(600);
  await touch.start({ x: press.x, y: press.y, id: 4 });
  for (let step = 1; step <= 4; step += 1) await touch.move({ x: press.x - step * 20, y: press.y, id: 4 });
  await touch.end();
  expect((await view(page)).offset - before.offset).toBeCloseTo(60 / before.spacing, 6);
});

test("exit_mode on_touch_end ends crosshair tracking when the finger lifts", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ tracking_mode: { exit_mode: "on_touch_end" } }));
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const press = { x: Math.round(g.left + g.width * 0.5), y: Math.round(g.top + g.pane_height * 0.5) };

  await touch.start({ ...press, id: 1 });
  await page.clock.runFor(240);
  expect(await last_crosshair(page)).toEqual({ x: press.x - g.left, y: press.y - g.top });
  await touch.move({ x: press.x + 30, y: press.y + 12, id: 1 });
  expect(await last_crosshair(page)).toEqual({ x: press.x + 30 - g.left, y: press.y + 12 - g.top });
  await touch.end();
  expect(await last_crosshair(page)).toBeNull();

  // Tracking ended with the lift: the next drag pans instead of steering a crosshair.
  const before = await view(page);
  await page.clock.runFor(600);
  await touch.start({ ...press, id: 2 });
  for (let step = 1; step <= 4; step += 1) await touch.move({ x: press.x - step * 20, y: press.y, id: 2 });
  await touch.end();
  expect((await view(page)).offset - before.offset).toBeCloseTo(60 / before.spacing, 6);
});

test("a two-finger pinch zooms around its starting centroid and ignores a third finger", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const cx = Math.round(g.left + g.width * 0.4);
  const cy = Math.round(g.top + g.pane_height * 0.5);
  const logical_at = (x) => page.evaluate((x) => window.__chart.time_scale().coordinate_to_logical(x), x);
  const before = await view(page);
  const anchor = await logical_at(cx - g.left);

  const a = { x: cx - 60, y: cy, id: 1 };
  await touch.start(a);
  await touch.start(a, { x: cx + 60, y: cy, id: 2 });
  // Spread while the pair drifts right: drift never pans, the start centroid stays anchored.
  for (let step = 1; step <= 5; step += 1) {
    await touch.move({ x: cx - 60 + step * 4, y: cy, id: 1 }, { x: cx + 60 + step * 36, y: cy, id: 2 });
  }
  const pinched = await view(page);
  expect(pinched.spacing).toBeGreaterThan(before.spacing * 1.5);
  expect(await logical_at(cx - g.left)).toBeCloseTo(anchor, 5);
  expect(await page.evaluate(() => window.__touchmoves.every(Boolean))).toBe(true);

  // A third finger is rejected: moving it changes nothing, and the original pair keeps pinching.
  const one = { x: cx - 40, y: cy, id: 1 };
  const two = { x: cx + 240, y: cy, id: 2 };
  const third = { x: cx, y: cy + 150, id: 3 };
  await touch.start(one, two, third);
  await touch.move(one, two, { ...third, x: cx - 200, y: cy + 50 });
  await touch.move(one, two, { ...third, x: cx + 300, y: cy - 100 });
  expect(await view(page)).toEqual(pinched);
  await touch.move({ ...one, x: cx - 80 }, { ...two, x: cx + 300 }, { ...third, x: cx + 300, y: cy - 100 });
  expect((await view(page)).spacing).toBeGreaterThan(pinched.spacing);
  expect(await logical_at(cx - g.left)).toBeCloseTo(anchor, 5);
  await touch.end();
  expect(await page.evaluate(() => window.__clicks)).toBe(0);
});

test("handle_scale pinch off leaves a real pinch without effect", async ({ page }) => {
  await goto_fixture(page);
  await page.evaluate(() => window.__chart.apply_options({ handle_scale: { pinch: false } }));
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const cx = Math.round(g.left + g.width * 0.4);
  const cy = Math.round(g.top + g.pane_height * 0.5);
  const before = await view(page);

  const a = { x: cx - 60, y: cy, id: 1 };
  await touch.start(a);
  await touch.start(a, { x: cx + 60, y: cy, id: 2 });
  for (let step = 1; step <= 5; step += 1) {
    await touch.move({ x: cx - 60 + step * 4, y: cy, id: 1 }, { x: cx + 60 + step * 36, y: cy, id: 2 });
  }
  await touch.end();
  expect(await view(page)).toEqual(before);
});

test("a tap emits exactly one click and a double tap one click plus one double-click", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const spot = { x: Math.round(g.left + g.width * 0.5), y: Math.round(g.top + g.pane_height * 0.3), id: 1 };
  const counts = () => page.evaluate(() => ({ clicks: window.__clicks, double_clicks: window.__double_clicks }));

  await touch.tap(spot);
  expect(await counts(), "no compatibility click doubles the tap").toEqual({ clicks: 1, double_clicks: 0 });

  // Past the 500 ms double-tap window a second tap is a new single tap.
  await page.clock.runFor(501);
  await touch.tap(spot);
  expect(await counts()).toEqual({ clicks: 2, double_clicks: 0 });

  await page.clock.runFor(501);
  await touch.tap(spot);
  await touch.tap({ ...spot, x: spot.x + 10 });
  expect(await counts()).toEqual({ clicks: 3, double_clicks: 1 });
});

test("a real touchcancel mid drawing drag restores the drawing", async ({ page }) => {
  await goto_fixture(page);
  const touch = await touchscreen(page);
  const g = await geometry(page);
  const center = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    const price = window.__chart.price_scale("right").get_visible_range();
    const level = price.from + (price.to - price.from) * 0.8;
    const l0 = Math.floor(range.from + (range.to - range.from) * 0.3);
    const l1 = Math.floor(range.from + (range.to - range.from) * 0.6);
    window.__drawing = window.__chart.add_drawing("trend_line", [
      { logical: l0, price: level },
      { logical: l1, price: level },
    ]);
    const scale = window.__chart.time_scale();
    return {
      x: (scale.logical_to_coordinate(l0) + scale.logical_to_coordinate(l1)) / 2,
      y: window.__main.price_to_coordinate(level),
    };
  });
  const original = await page.evaluate(() => window.__drawing.points());
  const x = Math.round(g.left + center.x);
  const y = Math.round(g.top + center.y);
  const before = await view(page);

  await touch.start({ x, y, id: 1 });
  for (let step = 1; step <= 5; step += 1) await touch.move({ x: x + step * 10, y: y + step * 8, id: 1 });
  expect(await page.evaluate(() => window.__chart.wasm.drawing_drag_active())).toBe(true);
  expect(await page.evaluate(() => window.__drawing.points())).not.toEqual(original);
  await touch.cancel();
  expect(await page.evaluate(() => ({
    active: window.__chart.wasm.drawing_drag_active(),
    points: window.__drawing.points(),
  }))).toEqual({ active: false, points: original });
  expect(await view(page)).toEqual(before);
  expect(await page.evaluate(() => window.__clicks)).toBe(0);

  // The recognizer is free again: a fresh touch drag moves the drawing and commits it.
  await touch.start({ x, y, id: 2 });
  for (let step = 1; step <= 5; step += 1) await touch.move({ x: x + step * 10, y: y + step * 8, id: 2 });
  await touch.end();
  expect(await page.evaluate(() => window.__chart.wasm.drawing_drag_active())).toBe(false);
  expect(await page.evaluate(() => window.__drawing.points())).not.toEqual(original);
});
