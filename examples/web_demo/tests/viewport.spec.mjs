import { test, expect } from "@playwright/test";

// Viewport contract through the public browser API: data changes never move a scrolled-back view,
// the live edge still follows, Ctrl/Cmd wheel zoom is pointer-anchored while a plain wheel pins the
// right edge, range subscriptions converge after re-entrant data loads, fractional ranges restore
// exactly, `scroll_to_real_time` animates to the configured right offset, and a locked
// full-session view survives data, resizes, and keyboard.

const T0 = 1_700_000_000;

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
});

test.afterEach(async ({ page }) => {
  await page.evaluate(() => {
    window.__vp?.chart.remove();
    window.__vp?.host.remove();
    window.__vp = undefined;
  });
});

/** A fresh 800x400 line chart outside the demo grid, with page helpers on `window.__vp`. */
async function make_chart(page, options = {}) {
  await page.evaluate(async ({ options, t0 }) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText =
      "position:fixed;left:0;top:0;width:800px;height:400px;z-index:1000;background:white";
    document.body.append(host);
    const chart = await create_chart(host, { autoSize: false, backend: "canvas2d", ...options });
    chart.resize(800, 400);
    const series = chart.add_series("line");
    const row = (i) => ({ time: t0 + i * 60, value: 100 + Math.sin(i / 7) });
    const rows = (from, to, step = 1) => {
      const out = [];
      for (let i = from; i <= to; i += step) out.push(row(i));
      return out;
    };
    const scale = chart.time_scale();
    // Content at the visible edges: the times of the first/last whole bars plus the fractional
    // border offsets, which must all survive a data change that keeps the view.
    const edges = () => {
      const range = scale.get_visible_logical_range();
      const left = Math.ceil(range.from);
      const right = Math.floor(range.to);
      const time_at = (index) => scale.coordinate_to_time(scale.logical_to_coordinate(index));
      return {
        left_time: time_at(left),
        right_time: time_at(right),
        from_fraction: +(left - range.from).toFixed(9),
        to_fraction: +(range.to - right).toFixed(9),
      };
    };
    window.__vp = { chart, host, series, scale, row, rows, edges };
  }, { options, t0: T0 });
}

test("history loads, backfills, and appends keep a scrolled-back view; the live edge follows", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(() => {
    const { series, scale, row, rows, edges } = window.__vp;
    // Every other minute exists, so later odd-minute updates backfill gaps left of the view.
    series.set_data(rows(0, 598, 2));
    scale.apply_options({ bar_spacing: 8 });
    scale.scroll_to_position(-60.5, false);
    const before = edges();

    // Prepend 100 older bars and append 3 newer ones in one replacement.
    series.set_data([...rows(-200, -2, 2), ...rows(0, 598, 2), ...rows(600, 604, 2)]);
    const after_replace = edges();
    // Out-of-order streaming insert far left of the view.
    series.update(row(11));
    const after_backfill = edges();
    // Scrolled-back tail append.
    series.update(row(606));
    const after_append = edges();

    scale.scroll_to_position(0, false);
    series.update(row(608));
    return {
      before,
      after_replace,
      after_backfill,
      after_append,
      live_position: scale.scroll_position(),
      live_right_time: edges().right_time,
      live_time: row(608).time,
    };
  });
  expect(result.after_replace).toEqual(result.before);
  expect(result.after_backfill).toEqual(result.before);
  expect(result.after_append).toEqual(result.before);
  expect(result.live_position).toBe(0);
  expect(result.live_right_time).toBe(result.live_time);
});

test("off-center Ctrl wheel zoom keeps the logical point under the pointer", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(async () => {
    const { chart, series, scale, rows } = window.__vp;
    series.set_data(rows(0, 999));
    scale.scroll_to_position(-100, false);
    const float_index = (x) => {
      const range = scale.get_visible_logical_range();
      return range.to - (scale.width() - 1 - x) / chart.wasm.bar_spacing();
    };
    const pointer_x = 150;
    const before = { at_pointer: float_index(pointer_x), at_center: float_index(scale.width() / 2) };
    const spacing_before = chart.wasm.bar_spacing();
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    overlay.dispatchEvent(new WheelEvent("wheel", {
      deltaX: 0,
      deltaY: -120,
      deltaMode: 0,
      ctrlKey: true,
      clientX: rect.left + chart.wasm.pane_left() + pointer_x,
      clientY: rect.top + chart.wasm.pane_height(0) / 2,
      bubbles: true,
      cancelable: true,
    }));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    return {
      before,
      after: { at_pointer: float_index(pointer_x), at_center: float_index(scale.width() / 2) },
      spacing_before,
      spacing_after: chart.wasm.bar_spacing(),
    };
  });
  expect(result.spacing_after).toBeGreaterThan(result.spacing_before);
  expect(Math.abs(result.after.at_pointer - result.before.at_pointer)).toBeLessThan(1e-6);
  // A center-anchored zoom would have kept the center instead.
  expect(Math.abs(result.after.at_center - result.before.at_center)).toBeGreaterThan(1);
});

test("off-center plain wheel zoom pins the right edge instead of the pointer", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(async () => {
    const { chart, series, scale, rows } = window.__vp;
    series.set_data(rows(0, 999));
    scale.scroll_to_position(-100, false);
    const float_index = (x) => {
      const range = scale.get_visible_logical_range();
      return range.to - (scale.width() - 1 - x) / chart.wasm.bar_spacing();
    };
    const pointer_x = 150;
    const before = { at_pointer: float_index(pointer_x), right: scale.get_visible_logical_range().to };
    const spacing_before = chart.wasm.bar_spacing();
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    const rect = overlay.getBoundingClientRect();
    overlay.dispatchEvent(new WheelEvent("wheel", {
      deltaX: 0,
      deltaY: -120,
      deltaMode: 0,
      clientX: rect.left + chart.wasm.pane_left() + pointer_x,
      clientY: rect.top + chart.wasm.pane_height(0) / 2,
      bubbles: true,
      cancelable: true,
    }));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    return {
      before,
      after: { at_pointer: float_index(pointer_x), right: scale.get_visible_logical_range().to },
      spacing_before,
      spacing_after: chart.wasm.bar_spacing(),
    };
  });
  expect(result.spacing_after).toBeGreaterThan(result.spacing_before);
  // The right edge (the bar offset past the latest bar) stays put; the bar under the pointer moves.
  expect(Math.abs(result.after.right - result.before.right)).toBeLessThan(1e-6);
  expect(Math.abs(result.after.at_pointer - result.before.at_pointer)).toBeGreaterThan(1);
});

test("range subscriptions fire on pan and data, and converge after a re-entrant history load", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(() => {
    const { series, scale, rows } = window.__vp;
    series.set_data(rows(0, 299));
    scale.apply_options({ bar_spacing: 8 });
    scale.scroll_to_position(-100, false);

    let loads = 0;
    let seen_at_load = null;
    const load_more = (range) => {
      // The load-more recipe: prepend history synchronously when the left edge nears the start.
      if (range !== null && range.from < 20 && loads === 0) {
        loads += 1;
        seen_at_load = range;
        series.set_data(rows(-200, 299));
      }
    };
    const logical_events = [];
    const time_events = [];
    const record_logical = (range) => logical_events.push(range);
    const record_time = (range) => time_events.push(range);
    scale.subscribe_visible_logical_range_change(load_more);
    scale.subscribe_visible_logical_range_change(record_logical);
    scale.subscribe_visible_time_range_change(record_time);

    scale.scroll_to_position(-150, false);
    const panned = logical_events.length;
    scale.scroll_to_position(-270, false);
    const final_logical = scale.get_visible_logical_range();
    const final_time = scale.get_visible_range();
    scale.unsubscribe_visible_logical_range_change(load_more);
    scale.unsubscribe_visible_logical_range_change(record_logical);
    scale.unsubscribe_visible_time_range_change(record_time);
    return {
      panned,
      loads,
      seen_at_load,
      final_logical,
      final_time,
      last_logical: logical_events.at(-1),
      last_time: time_events.at(-1),
    };
  });
  expect(result.panned).toBeGreaterThan(0);
  expect(result.loads).toBe(1);
  // The prepend rebased the logical range by exactly +200 without moving the visible bars.
  expect(result.final_logical.from).toBeCloseTo(result.seen_at_load.from + 200, 9);
  expect(result.final_logical.to).toBeCloseTo(result.seen_at_load.to + 200, 9);
  // Handlers after the mutating one end on the final range rather than the superseded one.
  expect(result.last_logical).toEqual(result.final_logical);
  expect(result.last_time).toEqual(result.final_time);
});

test("a fractional logical range restores exactly", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(() => {
    const { series, scale, rows } = window.__vp;
    series.set_data(rows(0, 499));
    scale.set_visible_logical_range({ from: 120.25, to: 239.75 });
    const exact = scale.get_visible_logical_range();
    scale.scroll_to_position(-37.4, false);
    const saved = scale.get_visible_logical_range();
    scale.set_visible_logical_range({ from: 0, to: 50 });
    scale.set_visible_logical_range(saved);
    return { exact, saved, restored: scale.get_visible_logical_range() };
  });
  expect(result.exact.from).toBeCloseTo(120.25, 9);
  expect(result.exact.to).toBeCloseTo(239.75, 9);
  expect(result.restored.from).toBeCloseTo(result.saved.from, 9);
  expect(result.restored.to).toBeCloseTo(result.saved.to, 9);
});

test("scroll_to_real_time animates back to the configured right offset", async ({ page }) => {
  await make_chart(page);
  const result = await page.evaluate(async () => {
    const { series, scale, rows } = window.__vp;
    series.set_data(rows(0, 499));
    scale.apply_options({ right_offset: 5 });
    scale.scroll_to_position(-80, false);
    scale.scroll_to_real_time();
    const started = scale.scroll_position();
    await new Promise((resolve) => setTimeout(resolve, 700));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    return { started, final: scale.scroll_position() };
  });
  expect(result.started).toBeLessThan(0);
  expect(result.final).toBe(5);
});

test("keyboard time-scale input respects handle_scroll and handle_scale being off", async ({ page }) => {
  await make_chart(page, { handle_scroll: false, handle_scale: false });
  const result = await page.evaluate(() => {
    const { chart, series, scale, rows } = window.__vp;
    series.set_data(rows(0, 499));
    scale.set_visible_logical_range({ from: 300, to: 420 });
    const fixed = scale.get_visible_logical_range();
    const press = (target, key) => {
      target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      target.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
    };
    const a11y = chart.accessibility();
    a11y.focus(0);
    for (const key of ["+", "-", "Home", "End", "PageDown"]) press(document.activeElement, key);
    const pane = scale.get_visible_logical_range();
    a11y.focus_target("time-axis");
    for (const key of ["Home", "ArrowLeft"]) press(document.activeElement, key);
    const time_axis = scale.get_visible_logical_range();
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    // A gated key does nothing, so the overlay leaves its default (page scrolling) alone.
    const prevented = [];
    for (const key of ["Home", "+", "-", "ArrowLeft", "ArrowRight"]) {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      overlay.dispatchEvent(event);
      overlay.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
      if (event.defaultPrevented) prevented.push(key);
    }
    return { fixed, pane, time_axis, overlay: scale.get_visible_logical_range(), prevented };
  });
  expect(result.pane).toEqual(result.fixed);
  expect(result.time_axis).toEqual(result.fixed);
  expect(result.overlay).toEqual(result.fixed);
  expect(result.prevented).toEqual([]);
});

test("removing the chart from a range handler or mid scroll_to_real_time raises nothing", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await make_chart(page);
  const thrown = await page.evaluate(async () => {
    const { chart, series, scale, rows } = window.__vp;
    series.set_data(rows(0, 499));
    scale.scroll_to_position(-80, false);
    // The animated return to the live edge must end quietly when the chart goes away.
    scale.scroll_to_real_time();
    chart.remove();
    await new Promise((resolve) => setTimeout(resolve, 600));

    // A range handler that disposes the chart ends the dispatch instead of reading a freed engine.
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:800px;height:400px";
    document.body.append(host);
    const other = await create_chart(host, { autoSize: false, backend: "canvas2d" });
    other.resize(800, 400);
    other.add_series("line").set_data(rows(0, 499));
    other.time_scale().subscribe_visible_logical_range_change(() => other.remove());
    try {
      other.time_scale().scroll_to_position(-50, false);
      return null;
    } catch (error) {
      return String(error);
    } finally {
      host.remove();
    }
  });
  expect(thrown).toBeNull();
  expect(errors).toEqual([]);
});

test("a locked full-session view survives the open, streaming, resizes, and keyboard input", async ({ page }) => {
  await make_chart(page, { handle_scroll: false, handle_scale: false });
  const result = await page.evaluate(() => {
    const { chart, series, scale, row } = window.__vp;
    const slots = 241;
    const full = { from: 0, to: slots - 1 };
    const minute = (slot) => row(0).time + slot * 60;
    // Every session minute is a whitespace slot until it trades.
    series.set_data(Array.from({ length: slots }, (_, slot) => ({ time: minute(slot) })));
    scale.apply_options({ lock_visible_logical_range: true });
    scale.set_visible_logical_range(full);
    const states = { pre_open: scale.get_visible_logical_range() };

    series.update({ time: minute(0), value: 10 });
    states.first_minute = scale.get_visible_logical_range();
    for (let slot = 1; slot < 90; slot += 1) series.update({ time: minute(slot), value: 10 + slot / 100 });
    series.update({ time: minute(89), value: 11 });
    states.streaming = scale.get_visible_logical_range();

    chart.resize(520, 400);
    states.narrow = scale.get_visible_logical_range();
    chart.resize(1000, 400);
    states.wide = scale.get_visible_logical_range();

    const press = (target, key) => {
      target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      target.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true }));
    };
    const a11y = chart.accessibility();
    a11y.focus(0);
    for (const key of ["+", "=", "-", "Home", "End", "ArrowLeft", "ArrowRight", "PageUp", "PageDown"]) {
      press(document.activeElement, key);
    }
    states.pane_keys = scale.get_visible_logical_range();
    a11y.focus_target("time-axis");
    for (const key of ["Home", "ArrowLeft", "ArrowRight"]) press(document.activeElement, key);
    states.time_axis_keys = scale.get_visible_logical_range();
    const overlay = chart.chart_element().querySelector("canvas:last-of-type");
    for (const key of ["ArrowLeft", "ArrowRight", "Home", "+", "-"]) press(overlay, key);
    states.overlay_keys = scale.get_visible_logical_range();
    return { full, states };
  });
  for (const [name, range] of Object.entries(result.states)) {
    expect(range, name).toEqual(result.full);
  }
});
