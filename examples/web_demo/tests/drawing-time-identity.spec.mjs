import { test, expect } from "@playwright/test";

// Drawing anchors keep their TIME identity through the public TypeScript API: interval
// switches and history prepends via set_data, persistence restore into a shifted data window
// (import before and after data), the split-grid workspace restore order, sync payloads,
// price-basis rescaling, the chart magnet modes through real pointer input, and keyboard nudges
// of a rectangle's edge handle through the accessibility layer.

const BASE = 1_704_067_200; // 2024-01-01T00:00:00Z
const MINUTE = 60;
const HOUR = 3_600;
const DAY = 86_400;

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

async function wait_for_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

async function open_isolated(page) {
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);
}

test("anchors follow their time through interval switches and history prepends", async ({ page }) => {
  await open_isolated(page);
  const result = await page.evaluate(async ({ BASE, MINUTE, HOUR, DAY }) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
    document.body.append(host);
    const chart = await create_chart(host, { backend: "canvas2d", autoSize: false });
    const series = chart.add_series("line");
    const bars = (from, step, count) => Array.from({ length: count }, (_, index) => ({
      time: from + index * step,
      value: 100 + (index % 7),
    }));
    const start = BASE + 10 * HOUR; // Monday 10:00
    series.set_data(bars(start, MINUTE, 120)); // 10:00..11:59
    const drawing = chart.add_drawing("trend_line", [{ logical: 37, price: 101 }, { logical: 130, price: 102 }]);
    const minute = drawing.points();
    series.set_data(bars(BASE + 6 * HOUR, HOUR, 10)); // 06:00..15:00
    const hourly = drawing.points();
    series.set_data(bars(BASE - 5 * DAY, DAY, 10)); // midnight-stamped daily bars
    const daily = drawing.points();
    series.set_data(bars(start, MINUTE, 120));
    const back = drawing.points();
    // History prepend: one more hour of earlier minutes.
    series.set_data(bars(start - HOUR, MINUTE, 180));
    const prepended = drawing.points();
    // Time-only anchors are accepted too.
    const by_time = chart.add_drawing("trend_line", [
      { time: start + 20 * MINUTE, price: 100 },
      { time: start + 50 * MINUTE, price: 101 },
    ]);
    const time_points = by_time.points();

    // Daily history is longer than intraday history: a drawing older than the hourly window keeps
    // its date while the host pages in more hourly bars, and returns to its day on daily bars.
    chart.clear_drawings();
    const weekdays = [];
    for (let day = 0; weekdays.length < 130; day += 1) {
      if (day % 7 < 5) weekdays.push(BASE + day * DAY); // BASE is a Monday
    }
    const session_hours = (days) => days.flatMap((day) =>
      Array.from({ length: 7 }, (_, hour) => ({ time: day + 9.5 * HOUR + hour * HOUR, value: 100 })));
    series.set_data(weekdays.map((time) => ({ time, value: 100 })));
    const long_line = chart.add_drawing("trend_line", [{ logical: 10, price: 101 }, { logical: 129, price: 102 }]);
    const saved_time = long_line.points()[0].time;
    series.set_data(session_hours(weekdays.slice(120)));
    series.set_data(session_hours(weekdays.slice(100))); // scroll back: prepend 20 trading days
    const paged_time = long_line.points()[0].time;
    series.set_data(weekdays.map((time) => ({ time, value: 100 })));
    const long_back = long_line.points();
    chart.remove();
    host.remove();
    return { minute, hourly, daily, back, prepended, time_points, saved_time, paged_time, long_back };
  }, { BASE, MINUTE, HOUR, DAY });

  expect(result.minute[0]).toEqual({ logical: 37, price: 101, time: BASE + 10 * HOUR + 37 * MINUTE });
  expect(result.minute[1].time).toBe(BASE + 12 * HOUR + 10 * MINUTE);
  expect(result.hourly[0].logical).toBeCloseTo(4 + 37 / 60, 9);
  expect(result.hourly[1].logical).toBeCloseTo(6 + 10 / 60, 9);
  expect(result.hourly[0].time).toBeCloseTo(result.minute[0].time, 3);
  expect(result.daily[0].logical).toBeCloseTo(5 + (10 * HOUR + 37 * MINUTE) / DAY, 9);
  expect(result.back[0].logical).toBeCloseTo(37, 6);
  expect(result.back[1].logical).toBeCloseTo(130, 6);
  expect(result.prepended[0].logical).toBeCloseTo(97, 6);
  expect(result.prepended[0].time).toBeCloseTo(result.minute[0].time, 3);
  expect(result.time_points.map((point) => point.logical)).toEqual([80, 110]);
  expect(result.saved_time).toBe(BASE + 14 * DAY); // the 11th weekday
  expect(result.paged_time).toBeCloseTo(result.saved_time, 3);
  expect(result.long_back[0].logical).toBeCloseTo(10, 6);
  expect(result.long_back[1].logical).toBeCloseTo(129, 6);
});

test("restores land on the saved times in a shifted window, standalone and in the grid", async ({ page }) => {
  await open_isolated(page);
  const result = await page.evaluate(async ({ BASE, HOUR }) => {
    const { create_chart, create_chart_grid } = await import("/dist/aeris_charts_financial.js");
    const make_host = () => {
      const host = document.createElement("div");
      host.style.cssText = "position:absolute;left:-10000px;top:0;width:800px;height:500px";
      document.body.append(host);
      return host;
    };
    const bars = (from, count) => Array.from({ length: count }, (_, index) => ({
      time: from + index * HOUR,
      value: 100 + (index % 5),
    }));
    const source_host = make_host();
    const source = await create_chart(source_host, { backend: "canvas2d", autoSize: false });
    source.add_series("line").set_data(bars(BASE, 100));
    source.add_drawing("trend_line", [{ logical: 50, price: 101 }, { logical: 60.25, price: 102 }]);
    source.set_drawing_price_basis("qfq");
    const state = source.export_state();
    source.remove();
    source_host.remove();

    // Import before data (the grid workspace order), then load bars starting 20 hours later.
    const before_host = make_host();
    const before = await create_chart(before_host, { backend: "canvas2d", autoSize: false });
    before.import_state(state);
    const basis = before.drawing_price_basis();
    before.add_series("line").set_data(bars(BASE + 20 * HOUR, 100));
    const before_points = before.drawings()[0].points();
    before.remove();
    before_host.remove();

    // Import after data with 30 more hours of history.
    const after_host = make_host();
    const after = await create_chart(after_host, { backend: "canvas2d", autoSize: false });
    after.add_series("line").set_data(bars(BASE - 30 * HOUR, 200));
    after.import_state(state);
    const after_points = after.drawings()[0].points();
    after.remove();
    after_host.remove();

    // Split-grid workspace: cells restore their chart documents before the host seeds data.
    const seed_original = (cell) => cell.chart.add_series("line").set_data(bars(BASE, 100));
    const grid_host = make_host();
    const grid = await create_chart_grid(grid_host, { shortcuts: false, on_cell_added: seed_original });
    seed_original(grid.cells()[0]);
    grid.cells()[0].chart.add_drawing("vertical_line", [{ logical: 40, price: 0 }]);
    const workspace = grid.export_state();
    grid.destroy();
    grid_host.remove();
    const restored_host = make_host();
    const restored = await create_chart_grid(restored_host, {
      shortcuts: false,
      initial_state: workspace,
      on_cell_restored: (cell) => cell.chart.add_series("line").set_data(bars(BASE + 10 * HOUR, 100)),
    });
    const grid_points = restored.cells()[0].chart.drawings()[0].points();
    restored.destroy();
    restored_host.remove();
    return { state, basis, before_points, after_points, grid_points };
  }, { BASE, HOUR });

  expect(result.state.drawing_price_basis).toBe("qfq");
  expect(result.state.drawings[0].anchors[0]).toEqual({ logical: 50, price: 101, time: BASE + 50 * HOUR });
  expect(result.basis).toBe("qfq");
  expect(result.before_points[0]).toEqual({ logical: 30, price: 101, time: BASE + 50 * HOUR });
  expect(result.before_points[1].logical).toBeCloseTo(40.25, 9);
  expect(result.after_points[0].logical).toBe(80);
  expect(result.after_points[1].logical).toBeCloseTo(90.25, 9);
  expect(result.grid_points[0]).toMatchObject({ logical: 30, time: BASE + 40 * HOUR });
});

test("sync payloads, price-basis rescale, batch rewrites and option errors through the TS API", async ({ page }) => {
  await open_isolated(page);
  const result = await page.evaluate(async ({ BASE, HOUR, DAY }) => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const make_host = () => {
      const host = document.createElement("div");
      host.style.cssText = "position:absolute;left:-10000px;top:0;width:800px;height:500px";
      document.body.append(host);
      return host;
    };
    const bars = (from, step, count) => Array.from({ length: count }, (_, index) => ({
      time: from + index * step,
      value: 100,
    }));
    const hourly_host = make_host();
    const hourly = await create_chart(hourly_host, { backend: "canvas2d", autoSize: false });
    hourly.add_series("line").set_data(bars(BASE, HOUR, 48));
    const line = hourly.add_drawing("trend_line", [{ logical: 10, price: 100 }, { logical: 30.5, price: 120 }]);
    const locked = hourly.add_drawing("horizontal_line", [{ logical: 5, price: 90 }], { locked: true });
    const payload = hourly.drawing_sync_payload("cell-a");

    const daily_host = make_host();
    const daily = await create_chart(daily_host, { backend: "canvas2d", autoSize: false });
    daily.add_series("line").set_data(bars(BASE - 3 * DAY, DAY, 10));
    const applied = daily.apply_drawing_sync_payload(payload);
    const synced = daily.drawings()[0].points();

    // Rescale: prices before a mid-series ex-date are halved; locked drawings follow the basis.
    const ex_date = BASE + 24 * HOUR;
    const changed = hourly.rescale_drawing_prices([{ to_time: ex_date, factor: 0.5 }], "qfq");
    const rescaled = { line: line.points().map((point) => point.price), locked: locked.points()[0].price };
    const basis = hourly.drawing_price_basis();
    const undo_after_rescale = hourly.undo_drawing(); // undoes the locked line's creation, not the rescale
    hourly.redo_drawing();

    // One undo step for a batch rewrite of several drawings.
    const batch = hourly.set_drawings_points([
      { drawing: line, points: [{ logical: 11, price: 60 }, { logical: 31, price: 130 }] },
      { drawing: hourly.drawings()[1], points: [{ time: BASE + 41 * HOUR, price: 50 }] },
    ]);
    hourly.undo_drawing();
    const after_batch_undo = hourly.drawings().map((drawing) => drawing.points()[0].logical);

    let option_error = null;
    try {
      hourly.add_drawing("trend_line", [{ logical: 1, price: 100 }, { logical: 2, price: 101 }], { width: "thick" });
    } catch (error) {
      option_error = error.code;
    }
    let anchor_error = null;
    try {
      hourly.add_drawing("trend_line", [{ logical: 1, price: 100 }]);
    } catch (error) {
      anchor_error = error.code;
    }
    hourly.remove();
    hourly_host.remove();
    daily.remove();
    daily_host.remove();
    return {
      applied, synced, changed, rescaled, basis, undo_after_rescale, batch, after_batch_undo,
      option_error, anchor_error,
    };
  }, { BASE, HOUR, DAY });

  expect(result.applied).toBe(true);
  expect(result.synced[0].logical).toBeCloseTo(3 + 10 / 24, 9);
  expect(result.synced[1].logical).toBeCloseTo(3 + 30.5 / 24, 9);
  expect(result.changed).toBe(2);
  expect(result.rescaled).toEqual({ line: [50, 120], locked: 45 });
  expect(result.basis).toBe("qfq");
  expect(result.undo_after_rescale).toBe(true);
  expect(result.batch).toBe(2);
  expect(result.after_batch_undo).toEqual([10, 5]);
  expect(result.option_error).toBe("invalid_options");
  expect(result.anchor_error).toBe("invalid_data");
});

async function goto_fixture(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await wait_for_chart(page);
  await page.evaluate(() => window.__chart.time_scale().set_visible_logical_range({ from: 350, to: 400 }));
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test("chart magnet modes snap pointer placement without a modifier and Ctrl toggles them", async ({ page }) => {
  await goto_fixture(page);
  const probe = await page.evaluate(() => {
    const scale = window.__chart.time_scale();
    const index = 370;
    const bar = window.__main.data_by_index(index);
    const x = scale.logical_to_coordinate(index) + (scale.logical_to_coordinate(index + 1) - scale.logical_to_coordinate(index)) * 0.2;
    const prices = [bar.open, bar.high, bar.low, bar.close];
    const top = Math.min(...prices.map((price) => window.__main.price_to_coordinate(price)));
    return {
      index,
      prices,
      x,
      near_y: window.__main.price_to_coordinate(bar.close) + 3,
      far_y: top - 40,
    };
  });
  const place = async (mode, y, ctrl) => {
    await page.evaluate((mode) => {
      window.__chart.clear_drawings();
      window.__chart.set_drawing_magnet_mode(mode);
      window.__chart.set_drawing_tool("horizontal_line");
    }, mode);
    // macOS turns Ctrl+click into a secondary click; Cmd is the same engine toggle there.
    const toggle_key = process.platform === "darwin" ? "Meta" : "Control";
    if (ctrl) await page.keyboard.down(toggle_key);
    await page.mouse.click(probe.x, y);
    if (ctrl) await page.keyboard.up(toggle_key);
    return page.evaluate(() => window.__chart.drawings()[0]?.points()[0] ?? null);
  };

  const strong = await place("strong", probe.near_y, false);
  expect(probe.prices).toContainEqual(strong.price);
  const strong_far = await place("strong", probe.far_y, false);
  expect(probe.prices).toContainEqual(strong_far.price);
  const toggled = await place("strong", probe.near_y, true);
  expect(probe.prices).not.toContainEqual(toggled.price);
  const weak_near = await place("weak", probe.near_y, false);
  expect(probe.prices).toContainEqual(weak_near.price);
  const weak_far = await place("weak", probe.far_y, false);
  expect(probe.prices).not.toContainEqual(weak_far.price);
  const off = await place("off", probe.near_y, false);
  expect(probe.prices).not.toContainEqual(off.price);
  expect(await page.evaluate(() => window.__chart.drawing_magnet_mode())).toBe("off");
});

test("keyboard nudges move a rectangle's edge handle by the nudge distance", async ({ page }) => {
  await goto_fixture(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const bar_a = window.__main.data_by_index(360);
    const bar_b = window.__main.data_by_index(380);
    const low = Math.min(bar_a.low, bar_b.low);
    const high = Math.max(bar_a.high, bar_b.high);
    const rectangle = chart.add_drawing("rectangle", [{ logical: 360, price: low }, { logical: 380, price: high }]);
    const y = (price) => window.__main.price_to_coordinate(price);
    const before = rectangle.points().map((point) => ({ ...point, y: y(point.price) }));
    chart.wasm.set_selected_drawing(rectangle.id);
    chart.accessibility().focus_target(`drawing:${rectangle.id}`);
    const layer = document.activeElement;
    const key = (name, shift = false) => layer.dispatchEvent(
      new KeyboardEvent("keydown", { key: name, shiftKey: shift, bubbles: true, cancelable: true }),
    );
    key("Enter");
    key("Tab"); // handle 1: top-left corner
    key("Tab"); // handle 2: top edge
    key("ArrowDown", true); // 10 CSS px
    key("Enter");
    const after = rectangle.points().map((point) => ({ ...point, y: y(point.price) }));
    return { before, after };
  });
  // The top edge (the high-price anchor) moved down by 10 CSS px; nothing jumped to the pane's top-left corner.
  expect(result.after[1].y).toBeCloseTo(result.before[1].y + 10, 3);
  expect(result.after[1].logical).toBe(result.before[1].logical);
  expect(result.after[0]).toEqual(result.before[0]);
});
