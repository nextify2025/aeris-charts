import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";

// Engine-owned interaction models (`aeris_charts_engine::interaction`): the TypeScript
// recognizer only classifies events and forwards samples — the axis drag-to-scale, vertical
// price pan, wheel/pinch zoom increments, kinetic coast, and eased scroll animations all
// compute in Rust. These specs drive the real gestures in the browser and assert the same
// behavior the headless engine tests pin down, including the public reference wheel sensitivity.

async function wait_grid(page) {
  await wait_for_chart(page, { grid: true });
}

const state = (page) =>
  page.evaluate(() => ({
    spacing: window.__chart.wasm.bar_spacing(),
    offset: window.__chart.wasm.scroll_position(),
    range: window.__chart.wasm.price_scale_visible_range(0, 0),
    width: window.__chart.wasm.time_scale_width(),
    axis_h: window.__chart.wasm.time_scale_height(),
    min_spacing: window.__chart.time_scale().options().min_bar_spacing,
    auto_scale: window.__chart.price_scale("right").options().auto_scale,
  }));

async function chart_box(page) {
  return page.evaluate(() => {
    const r = document.getElementById("chart_container").getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height };
  });
}

test("interaction models run engine-side with canonical behavior", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  // Keep this gesture fixture on its historical fitted viewport without making fit-content a demo
  // startup policy. The production demo now leaves the engine's native time-scale state untouched.
  await page.evaluate(() => window.__chart.time_scale().fit_content());
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const box = await chart_box(page);
  const s0 = await state(page);
  const pane_left = await page.evaluate(() => window.__chart.wasm.pane_left());
  console.log("initial:", JSON.stringify(s0), "pane_left:", pane_left);

  // 1) time-axis drag-to-scale: drag RIGHT on the time axis -> zoom out (spacing shrinks).
  const time_axis_y = box.y + box.h - s0.axis_h / 2;
  const ax_x = box.x + pane_left + s0.width * 0.6;
  await page.mouse.move(ax_x, time_axis_y);
  await page.mouse.down();
  await page.mouse.move(ax_x + s0.width * 0.2, time_axis_y, { steps: 5 });
  let s = await state(page);
  console.log("time-axis drag right:", s0.spacing.toFixed(4), "->", s.spacing.toFixed(4));
  expect(s.spacing).toBeLessThan(s0.spacing);
  // reference ratio: spacing * (width - x_now) / (width - x_start) = spacing0 * (0.8w-?) ...
  // start length from right = w - 0.6w = 0.4w; current = w - 0.8w = 0.2w -> x0.5
  // A docked inspector can make fit-content spacing small enough for this gesture to reach the
  // engine's configured floor. The ratio still applies, bounded by that public option.
  expect(s.spacing).toBeCloseTo(Math.max(s0.min_spacing, s0.spacing * (0.2 * s0.width) / (0.4 * s0.width)), 3);
  await page.mouse.up();
  // ...and drag LEFT: zoom in by the inverse ratio relative to THIS drag's start spacing.
  const s1 = s.spacing;
  await page.mouse.move(ax_x, time_axis_y);
  await page.mouse.down();
  await page.mouse.move(ax_x - s0.width * 0.2, time_axis_y, { steps: 5 });
  s = await state(page);
  expect(s.spacing).toBeCloseTo(s1 * (0.6 * s0.width) / (0.4 * s0.width), 3);
  await page.mouse.up();

  // 2) price-axis drag-to-scale: drag on the right axis -> range scales around its center.
  const price_axis_x = box.x + pane_left + s0.width + 10;
  const pane_mid_y = box.y + box.h * 0.4;
  await page.mouse.move(price_axis_x, pane_mid_y);
  await page.mouse.down();
  await page.mouse.move(price_axis_x, pane_mid_y + 60, { steps: 5 });
  s = await state(page);
  const [f0, t0] = s0.range, [f1, t1] = s.range;
  const mid0 = (f0 + t0) / 2, mid1 = (f1 + t1) / 2;
  console.log("price-axis drag down: span", (t0 - f0).toFixed(3), "->", (t1 - f1).toFixed(3));
  expect(t1 - f1).not.toBeCloseTo(t0 - f0, 3);
  expect(mid1).toBeCloseTo(mid0, 6); // center-pinned
  await page.mouse.up();

  // 3) vertical price pan: grabbing an autoscaled candle preserves the lock. Once the scale is
  // explicitly manual, the same grab shifts its range with a constant span.
  const grab = await page.evaluate(async () => {
    window.__chart.price_scale("right").set_auto_scale(true);
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const range = window.__chart.time_scale().get_visible_logical_range();
    // Stay clear of the pane-facing alert-create control attached to the crosshair price label at
    // the right edge; this probe is specifically a series price-pan gesture.
    const logical = Math.floor((range.from + range.to) / 2);
    const bar = window.__main.data_by_index(logical, -1);
    return {
      x: window.__chart.time_scale().logical_to_coordinate(logical),
      y: window.__main.price_to_coordinate((bar.high + bar.low) / 2),
    };
  });
  const locked_before = await state(page);
  expect(locked_before.auto_scale).toBe(true);
  const grab_x = box.x + pane_left + grab.x;
  const candle_y = box.y + grab.y;
  await page.mouse.move(grab_x, candle_y);
  await page.mouse.down();
  await page.mouse.move(grab_x, candle_y + 40, { steps: 6 });
  await page.mouse.up();
  const locked_after = await state(page);
  expect(locked_after.auto_scale).toBe(true);
  expect(locked_after.range).toEqual(locked_before.range);

  await page.evaluate(() => window.__chart.price_scale("right").set_auto_scale(false));
  const pre_pan = await state(page);
  await page.mouse.move(grab_x, candle_y);
  await page.mouse.down();
  await page.mouse.move(grab_x, candle_y + 40, { steps: 6 });
  const post_pan = await state(page);
  console.log("price pan +40px:", JSON.stringify(pre_pan.range), "->", JSON.stringify(post_pan.range));
  const span_pre = pre_pan.range[1] - pre_pan.range[0];
  const span_post = post_pan.range[1] - post_pan.range[0];
  expect(span_post).toBeCloseTo(span_pre, 6);
  expect(post_pan.range[0]).toBeGreaterThan(pre_pan.range[0]); // dragged down -> range up
  expect(post_pan.auto_scale).toBe(false);
  await page.mouse.up();
  await page.evaluate(() => window.__chart.wasm.set_price_scale_auto_scale(0, 0, true));

  // 4) horizontal pan (time scroll): drag left -> view moves to older data (offset grows).
  const cx = box.x + pane_left + s0.width / 2;
  const o0 = (await state(page)).offset;
  await page.mouse.move(cx, pane_mid_y);
  await page.mouse.down();
  await page.mouse.move(cx - 120, pane_mid_y, { steps: 6 });
  let o1 = (await state(page)).offset;
  console.log("pane drag left 120px: offset", o0.toFixed(2), "->", o1.toFixed(2));
  expect(o1).toBeGreaterThan(o0);
  await page.mouse.up();

  // 5) wheel zoom at pane center: deltaY<0 zooms in.
  await page.evaluate(() => window.__chart.apply_options({ wheel_behavior: "zoom" }));
  const z0 = (await state(page)).spacing;
  await page.mouse.move(cx, pane_mid_y);
  await page.mouse.wheel(0, -120);
  await wait_grid(page);
  const z1 = (await state(page)).spacing;
  console.log("wheel zoom in:", z0.toFixed(4), "->", z1.toFixed(4));
  expect(z1).toBeGreaterThan(z0 * 1.04);

  // 6) wheel scroll (deltaX): offset moves by 80px/spacing bars.
  await page.evaluate(() => window.__chart.apply_options({ wheel_behavior: "pan" }));
  const w0 = (await state(page)).offset;
  await page.mouse.wheel(120, 0);
  await wait_grid(page);
  const w1 = (await state(page)).offset;
  console.log("wheel scroll: offset", w0.toFixed(2), "->", w1.toFixed(2));
  expect(Math.abs(w1 - w0)).toBeGreaterThan(0.5);

  // 7) kinetic coast: a fast flick keeps scrolling after release (no further input). Reference
  // sampling is in logical rightOffset units with its px thresholds divided by bar spacing, so the
  // same flick remains consistent across zoom levels. Enable mouse kinetic explicitly first.
  await page.evaluate(() => window.__chart.apply_options({ kinetic_scroll: { mouse: true, touch: true } }));
  await page.clock.install();
  // Freeze the fake clock so CDP dispatch latency cannot skew the flick's per-segment speed;
  // time now advances only on explicit runFor. The
  // freeze target is in the fake Date domain (pauseAt's input), slightly ahead of "now".
  await page.clock.pauseAt((await page.evaluate(() => Date.now())) + 1000);
  await page.mouse.move(cx + 140, pane_mid_y);
  await page.mouse.down();
  for (let i = 1; i <= 7; i++) {
    await page.mouse.move(cx + 140 - i * 30, pane_mid_y);
    await page.clock.runFor(8);
  }
  const at_release = (await state(page)).offset;
  // The engine refuses a coast whose release lags the last sample by >50 ms, and CDP dispatch
  // latency would eat that window under a fake clock — dispatch the final move and the release
  // in ONE in-page turn so both share the same fake timestamp (mouse pointerId is always 1).
  await page.evaluate(({ final_x, final_y }) => {
    const overlay = document.querySelector("#chart_container canvas:last-of-type");
    overlay.dispatchEvent(
      new PointerEvent("pointermove", {
        pointerId: 1,
        pointerType: "mouse",
        buttons: 1,
        clientX: final_x,
        clientY: final_y,
        bubbles: true,
      }),
    );
    overlay.dispatchEvent(
      new PointerEvent("pointerup", { pointerId: 1, pointerType: "mouse", button: 0, bubbles: true }),
    );
  }, { final_x: cx + 140 - 7 * 30 - 30, final_y: pane_mid_y });
  await page.clock.runFor(150); // coast frames fire without further input
  const after_coast = (await state(page)).offset;
  console.log("kinetic: release at", at_release.toFixed(2), "coast to", after_coast.toFixed(2));
  expect(after_coast).toBeGreaterThan(at_release + 0.3);
  // the coast settles (finished): a long tick lands the end position and nothing moves after
  await page.clock.runFor(10000);
  const settled = (await state(page)).offset;
  await page.clock.runFor(500);
  expect((await state(page)).offset).toBeCloseTo(settled, 6);

  // 8) animated scroll_to_position: the engine eases from start to target (cubic ease-out)
  // instead of jumping. Samples are recorded in-page with their fake-clock timestamps, so the
  // easing curve is asserted without any driver-side timing races.
  await page.evaluate(() => {
    window.__anim = { t0: performance.now(), samples: [] };
    window.__chart.time_scale().scroll_to_position(0, false);
    window.__anim.t0 = performance.now();
    window.__chart.time_scale().scroll_to_position(6, true);
    const sampler = () => {
      window.__anim.samples.push([performance.now(), window.__chart.wasm.scroll_position()]);
      requestAnimationFrame(sampler);
    };
    requestAnimationFrame(sampler);
  });
  await page.clock.runFor(500);
  const anim = await page.evaluate(() => window.__anim);
  const points = anim.samples.map(([t, pos]) => ({ progress: (t - anim.t0) / 300, pos }));
  const final = points[points.length - 1];
  console.log("animated scroll:", points.slice(0, 8).map((p) => `${p.progress.toFixed(2)}:${p.pos.toFixed(2)}`).join(" "));
  expect(final.pos).toBeCloseTo(6, 5);
  // eased intermediates (not a jump): some sample lands strictly inside (0.05, 5.95)
  expect(points.some((p) => p.pos > 0.05 && p.pos < 5.95)).toBe(true);
  // cubic ease-out is fast early: the sample nearest 50% progress is past 70% of the way
  const half = points.reduce((a, b) => (Math.abs(b.progress - 0.5) < Math.abs(a.progress - 0.5) ? b : a));
  expect(half.pos).toBeGreaterThan(4.2);

  // 9) keyboard arrows are low-friction velocity motion while held. The engine owns repeat cadence,
  // so it launches immediately, builds speed smoothly, ignores OS-repeat jitter, and key-up stops
  // immediately instead of coasting or finishing a destination tween.
  await page.evaluate(() => document.querySelector("#chart_container canvas:last-of-type").focus());
  await page.clock.runFor(50); // let the step-8 settle fully out of flight
  const k0 = (await state(page)).offset;
  await page.evaluate(() => {
    window.__key_anim = { t0: performance.now(), samples: [] };
    const sampler = () => {
      window.__key_anim.samples.push([performance.now(), window.__chart.wasm.scroll_position()]);
      requestAnimationFrame(sampler);
    };
    requestAnimationFrame(sampler);
  });
  await page.keyboard.down("Control");
  await page.keyboard.down("ArrowLeft");
  await page.clock.runFor(50);
  const k50 = (await state(page)).offset;
  await page.clock.runFor(50);
  const k100 = (await state(page)).offset;
  await page.clock.runFor(50);
  const k150 = (await state(page)).offset;
  await page.clock.runFor(150);
  const k300 = (await state(page)).offset;
  expect(k50).toBeLessThan(k0);
  expect(k100).toBeLessThan(k50);
  expect(k150).toBeLessThan(k100);
  expect(k0 - k50).toBeGreaterThan(1.5, "keyboard kinetic launch should not feel sticky or delayed");
  expect(k50 - k100).toBeGreaterThan(k0 - k50, "held motion should build velocity after launch");

  // Inject the repeat event that browsers receive while a key stays physically down. It must not
  // cancel the engine-owned kinetic session; the chart should continue until the real key-up.
  await page.evaluate(() => {
    document.querySelector("#chart_container canvas:last-of-type").dispatchEvent(new KeyboardEvent("keydown", {
      key: "ArrowLeft",
      ctrlKey: true,
      repeat: true,
      bubbles: true,
      cancelable: true,
    }));
  });
  await page.clock.runFor(150);
  const k450 = (await state(page)).offset;
  expect(k450).toBeLessThan(k300, "keyboard kinetic motion must survive OS repeat while held");
  await page.keyboard.up("ArrowLeft");
  const at_key_up = (await state(page)).offset;
  await page.clock.runFor(300);
  const after_release = (await state(page)).offset;
  await page.keyboard.up("Control");
  console.log(
    "keyboard Ctrl+ArrowLeft held:",
    k0.toFixed(2), "->", k50.toFixed(2), "->", k100.toFixed(2), "->", k150.toFixed(2), "->", k300.toFixed(2),
  );
  expect(after_release).toBeCloseTo(at_key_up, 6);

  // 10) +/- keyboard zoom still works through the same zoom path.
  const kb0 = (await state(page)).spacing;
  await page.keyboard.press("+");
  await page.clock.runFor(50);
  expect((await state(page)).spacing).toBeGreaterThan(kb0);
});

test("the demo pans its displayed manual price scale from empty pane space", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  const box = await chart_box(page);
  const geometry = await page.evaluate(() => ({
    pane_left: window.__chart.wasm.pane_left(),
    pane_width: window.__chart.wasm.time_scale_width(),
    pane_height: window.__chart.wasm.pane_height(0),
  }));

  // A real axis drag is the demo's unlock gesture.
  const axis_x = box.x + geometry.pane_left + geometry.pane_width + 10;
  const axis_y = box.y + geometry.pane_height * 0.4;
  await page.mouse.move(axis_x, axis_y);
  await page.mouse.down();
  await page.mouse.move(axis_x, axis_y + 60, { steps: 6 });
  await page.mouse.up();

  const before = await page.evaluate(() => ({
    auto_scale: window.__chart.price_scale("right").options().auto_scale,
    range: window.__chart.price_scale("right").get_visible_range(),
    anchors: JSON.parse(window.__chart.wasm.selection_anchor_identities_json()).length,
  }));
  expect(before.auto_scale).toBe(false);
  expect(before.anchors).toBe(0);

  // Deliberately miss all series geometry: the demo still knows its displayed primary series and
  // routes this vertical pan to that manual scale without leaving a visible series selection.
  const empty = await page.evaluate(() => {
    const range = window.__chart.time_scale().get_visible_logical_range();
    let max_high = -Infinity;
    for (let index = Math.ceil(range.from); index <= Math.floor(range.to); index += 1) {
      const bar = window.__main.data_by_index(index);
      if (bar !== null) max_high = Math.max(max_high, bar.high);
    }
    return {
      x: window.__chart.time_scale().logical_to_coordinate(Math.floor((range.from + range.to) / 2)),
      y: Math.max(10, window.__main.price_to_coordinate(max_high) - 20),
    };
  });
  const pane_x = box.x + geometry.pane_left + empty.x;
  const pane_y = box.y + empty.y;
  await page.mouse.move(pane_x, pane_y);
  await page.mouse.down();
  await page.mouse.move(pane_x, pane_y + 44, { steps: 6 });
  await page.mouse.up();

  const after = await page.evaluate(() => ({
    auto_scale: window.__chart.price_scale("right").options().auto_scale,
    range: window.__chart.price_scale("right").get_visible_range(),
    anchors: JSON.parse(window.__chart.wasm.selection_anchor_identities_json()).length,
  }));
  expect(after.auto_scale).toBe(false);
  expect(after.range).not.toEqual(before.range);
  expect(after.range.to - after.range.from).toBeCloseTo(before.range.to - before.range.from, 6);
  expect(after.anchors).toBe(0);
});
