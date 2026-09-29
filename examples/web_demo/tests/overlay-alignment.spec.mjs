import { test, expect } from "@playwright/test";

// Multi-calendar overlays (`series_options.time_alignment: "as_of"`) through the public browser
// API and the rendered frame: an overlay from another market calendar adds no time point to the
// primary series, each primary slot shows the overlay's last row at or before it, live ticks on
// either side follow, and the option validates, reads back, and is honored by worker charts.

const OVERLAY = [255, 0, 255];

async function open_chart(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  await settle(page);
}

async function settle(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

test("an as-of overlay keeps the primary calendar gapless and shows its last row per slot", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const day = (date) => Date.UTC(2024, 0, date) / 1000;
    // HK trades Jan 2, 3, 4, 8, 9; the US index trades Jan 2, 3, 5 (an HK holiday), 8, 10.
    const hk = [2, 3, 4, 8, 9];
    window.__main.set_data(hk.map((date, index) => ({
      time: day(date), open: 100 + index, high: 102 + index, low: 99 + index, close: 101 + index,
    })));
    const us = chart.add_series("line", {
      time_alignment: "as_of",
      price_scale_id: "left",
      color: "#ff00ff",
      line_width: 3,
      last_price_animation: false,
      price_line_visible: false,
      last_value_visible: false,
    });
    us.set_data([
      { time: day(2), value: 4000 },
      { time: day(3), value: 4040 },
      { time: day(5), value: 4100 },
      { time: day(8), value: 4120 },
      { time: day(10), value: 4200 },
    ]);
    window.__us = us;
    window.__day = day;
    const scale = chart.time_scale();
    scale.fit_content();
    const value_at = (logical) => chart.value_snapshot(logical)
      .find((entry) => entry.series === us)?.value ?? null;
    return {
      options: (({ time_alignment, as_of_max_staleness }) => ({ time_alignment, as_of_max_staleness }))(us.options()),
      us_only_slot: scale.time_to_coordinate(day(5)),
      us_future_slot: scale.time_to_coordinate(day(10)),
      hk_slots: hk.map((date) => scale.time_to_coordinate(day(date)) !== null),
      values: [0, 1, 2, 3, 4].map(value_at),
      snapshot_time: chart.value_snapshot(2).find((entry) => entry.series === us).time,
      own_rows: us.data().map((row) => row.time),
    };
  });
  expect(result.options).toEqual({ time_alignment: "as_of", as_of_max_staleness: null });
  // No US-only day became a slot: the HK candles stay gapless.
  expect(result.us_only_slot).toBeNull();
  expect(result.us_future_slot).toBeNull();
  expect(result.hk_slots).toEqual([true, true, true, true, true]);
  // Jan 4 (US closed) repeats Jan 3; Jan 5 collapses into Jan 8; Jan 9 repeats Jan 8.
  expect(result.values).toEqual([4000, 4040, 4040, 4120, 4120]);
  expect(result.snapshot_time).toBe(Date.UTC(2024, 0, 4) / 1000);
  expect(result.own_rows).toEqual([2, 3, 5, 8, 10].map((date) => Date.UTC(2024, 0, date) / 1000));

  // The stroke runs flat from Jan 3 to the US-holiday slot instead of sloping toward Jan 5.
  await settle(page);
  const painted = await page.evaluate((color) => {
    const chart = window.__chart;
    const { __us: us, __day: day } = window;
    const scale = chart.time_scale();
    const pane = chart.panes()[0].get_geometry();
    const shot = chart.take_screenshot(true, false);
    const ratio = shot.width / chart.chart_element().clientWidth;
    const pixels = shot.getContext("2d").getImageData(0, 0, shot.width, shot.height).data;
    const mean_y = (x_media) => {
      const ys = [];
      const x0 = Math.floor((pane.left + x_media - 1) * ratio);
      const x1 = Math.ceil((pane.left + x_media + 1) * ratio);
      for (let y = Math.ceil(pane.top * ratio); y < Math.floor((pane.top + pane.height) * ratio); y += 1) {
        for (let x = x0; x < x1; x += 1) {
          const i = (y * shot.width + x) * 4;
          if (Math.abs(pixels[i] - color[0]) + Math.abs(pixels[i + 1] - color[1])
            + Math.abs(pixels[i + 2] - color[2]) < 60) ys.push(y / ratio);
        }
      }
      return ys.length === 0 ? null : ys.reduce((sum, y) => sum + y, 0) / ys.length;
    };
    const x3 = scale.time_to_coordinate(day(3));
    const x4 = scale.time_to_coordinate(day(4));
    return {
      flat_y: mean_y((x3 + x4) / 2),
      level_y: pane.top + us.price_to_coordinate(4040),
      next_level_y: pane.top + us.price_to_coordinate(4100),
    };
  }, OVERLAY);
  expect(painted.flat_y).not.toBeNull();
  expect(Math.abs(painted.flat_y - painted.level_y)).toBeLessThan(2);
  expect(Math.abs(painted.flat_y - painted.next_level_y)).toBeGreaterThan(4);

  // Comparison anchors and the legend read the as-of values.
  const comparison = await page.evaluate(() => {
    const chart = window.__chart;
    chart.set_comparison_anchor(window.__day(4));
    const entry = chart.comparison_legend_snapshot().find((row) => row.series_id === window.__us.id);
    chart.set_comparison_anchor(null);
    return entry;
  });
  expect(comparison).toMatchObject({ anchor_value: 4040, latest_value: 4120 });

  // Live ticks on both sides follow without adding overlay slots.
  const live = await page.evaluate(() => {
    const chart = window.__chart;
    const { __us: us, __day: day } = window;
    const value_at = (logical) => chart.value_snapshot(logical)
      .find((entry) => entry.series === us)?.value ?? null;
    const out = {};
    window.__main.update({ time: day(11), open: 106, high: 107, low: 105, close: 106 });
    out.primary_tick = value_at(5);
    us.update({ time: day(10), value: 4210 });
    out.overlay_tick = value_at(5);
    us.update({ time: day(12), value: 4300 });
    out.ahead_slot = chart.time_scale().time_to_coordinate(day(12));
    out.ahead_value = value_at(5);
    window.__main.update({ time: day(12), open: 107, high: 108, low: 106, close: 107 });
    out.caught_up = value_at(6);
    return out;
  });
  expect(live).toEqual({
    primary_tick: 4200,
    overlay_tick: 4210,
    ahead_slot: null,
    ahead_value: 4210,
    caught_up: 4300,
  });
  expect(errors).toEqual([]);
});

test("time_alignment validates, bounds staleness, and switches back to the union", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const day = (date) => Date.UTC(2024, 0, date) / 1000;
    window.__main.set_data([2, 3, 4, 8, 9].map((date) => ({
      time: day(date), open: 100, high: 101, low: 99, close: 100,
    })));
    const us = chart.add_series("line", { time_alignment: "as_of", price_scale_id: "left" });
    us.set_data([
      { time: day(2), value: 1 },
      { time: day(3), value: 2 },
      { time: day(5), value: 3 },
      { time: day(8), value: 4 },
    ]);
    const code = (apply) => {
      try {
        apply();
        return null;
      } catch (error) {
        return error.code;
      }
    };
    const value_at = (logical) => chart.value_snapshot(logical)
      .find((entry) => entry.series === us)?.value ?? null;
    const sma = chart.add_sma(us, 2);
    // Re-applying the current alignment (a React re-render) notifies nothing; a change notifies
    // once.
    const changes = [];
    us.subscribe_data_changed((scope) => changes.push(scope));
    us.apply_options({ time_alignment: "as_of" });
    us.apply_options({ time_alignment: "as_of", as_of_max_staleness: null, color: "#ff00ff" });
    const unchanged = changes.length;
    // A bad value creates no series and announces none.
    const series_ids = () => chart.panes()[0].get_series().map((series) => series.id);
    const ids_before = series_ids();
    const added = [];
    chart.subscribe_series_added((series) => added.push(series.id));
    const refused_adds = [
      code(() => chart.add_series("line", { time_alignment: "asof" })),
      code(() => chart.add_series("line", { as_of_max_staleness: -1 })),
      code(() => chart.add_series("line", { time_alignment: "union", as_of_max_staleness: 60 })),
      code(() => chart.add_series("line", { as_of_max_staleness: 60 })),
    ];
    const out = {
      unchanged,
      refused_adds,
      leaked: JSON.stringify(series_ids()) !== JSON.stringify(ids_before) || added.length !== 0,
      bogus: code(() => us.apply_options({ time_alignment: "calendar" })),
      negative: code(() => us.apply_options({ as_of_max_staleness: -1 })),
      fractional: code(() => us.apply_options({ as_of_max_staleness: 1.5 })),
      union_bound: code(() => us.apply_options({ time_alignment: "union", as_of_max_staleness: 60 })),
      study: code(() => sma.apply_options({ time_alignment: "union" })),
      study_alignment: sma.options().time_alignment,
      still_as_of: us.options().time_alignment,
    };
    // Only rows exactly at a slot: the US holiday Jan 4 and Jan 9 stay empty.
    us.apply_options({ as_of_max_staleness: 0 });
    out.changed = changes.length - unchanged;
    out.exact = [0, 1, 2, 3, 4].map(value_at);
    out.exact_options = us.options().as_of_max_staleness;
    us.apply_options({ as_of_max_staleness: 86_400 });
    out.one_day = [0, 1, 2, 3, 4].map(value_at);
    us.apply_options({ time_alignment: "union" });
    out.union = {
      options: us.options(),
      us_only_slot: chart.time_scale().time_to_coordinate(day(5)) !== null,
    };
    return out;
  });
  expect(result.unchanged).toBe(0);
  expect(result.changed).toBe(1);
  expect(result.refused_adds).toEqual(["invalid_options", "invalid_options", "invalid_options", "invalid_options"]);
  expect(result.leaked).toBe(false);
  expect(result.bogus).toBe("invalid_options");
  expect(result.negative).toBe("invalid_options");
  expect(result.fractional).toBe("invalid_options");
  expect(result.union_bound).toBe("invalid_options");
  expect(result.study).toBe("unsupported_operation");
  expect(result.study_alignment).toBe("as_of");
  expect(result.still_as_of).toBe("as_of");
  expect(result.exact).toEqual([1, 2, null, 4, null]);
  expect(result.exact_options).toBe(0);
  expect(result.one_day).toEqual([1, 2, 2, 4, 4]);
  expect(result.union.options).toMatchObject({ time_alignment: "union", as_of_max_staleness: null });
  expect(result.union.us_only_slot).toBe(true);
});

test("a synthetic transform returns an as-of series to the union and its axis refuses as-of", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const series = chart.add_series("candlestick", { time_alignment: "as_of" });
    const before = series.options().time_alignment;
    chart.configure_synthetic_bar_series(series, { kind: "renko_fixed", box_size: 1 });
    const after = series.options().time_alignment;
    const start = Math.floor(window.__data[0].time);
    chart.set_synthetic_bar_source(series, [100, 103, 99].map((close, index) => ({
      time: start + index, open: close, high: close, low: close, close,
    })));
    // Renko row keys are not times: no overlay can be joined as-of on that axis.
    const line = chart.add_series("line");
    let refused = null;
    try {
      line.apply_options({ time_alignment: "as_of" });
    } catch (error) {
      refused = error.code;
    }
    // Refused at creation, an as-of series is not left behind.
    const ids_before = chart.panes()[0].get_series().map((entry) => entry.id);
    let refused_add = null;
    try {
      chart.add_series("line", { time_alignment: "as_of" });
    } catch (error) {
      refused_add = error.code;
    }
    const ids_after = chart.panes()[0].get_series().map((entry) => entry.id);
    return {
      before,
      after,
      bars: series.data().length,
      refused,
      line_alignment: line.options().time_alignment,
      refused_add,
      left_behind: JSON.stringify(ids_after) !== JSON.stringify(ids_before),
    };
  });
  expect(result).toEqual({
    before: "as_of",
    after: "union",
    bars: 6,
    refused: "unsupported_operation",
    line_alignment: "union",
    refused_add: "unsupported_operation",
    left_behind: false,
  });
});

test("worker charts take time_alignment when adding a series", async ({ page }) => {
  await page.goto("/");
  const supported = await page.evaluate(() =>
    typeof OffscreenCanvas !== "undefined"
    && "transferControlToOffscreen" in HTMLCanvasElement.prototype,
  );
  test.skip(!supported, "OffscreenCanvas transfer is unavailable in this browser");
  const result = await page.evaluate(() => new Promise((resolve) => {
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:0;top:0;width:640px;height:360px;z-index:1000";
    const gpu = document.createElement("canvas");
    const fallback = document.createElement("canvas");
    host.append(gpu, fallback);
    document.body.appendChild(host);
    const worker = new Worker("/offscreen_chart_worker.js", { type: "module" });
    worker.onerror = (event) => resolve({ type: "error", message: event.message });
    worker.onmessage = (event) => {
      if (event.data.type === "ready") worker.postMessage({ type: "as_of_overlay", bars: 600 });
      else if (event.data.type === "as_of_overlay" || event.data.type === "error") {
        worker.terminate();
        resolve(event.data);
      }
    };
    const gpu_canvas = gpu.transferControlToOffscreen();
    const fallback_canvas = fallback.transferControlToOffscreen();
    worker.postMessage({
      type: "init",
      gpu_canvas,
      fallback_canvas,
      width: 640,
      height: 360,
      dpr: 1,
      backend: "canvas2d",
      bars: 600,
      force_fallback_adapter: true,
    }, [gpu_canvas, fallback_canvas]);
  }));
  expect(result.type, result.message).toBe("as_of_overlay");
  // The overlay's 30 s-offset minutes would double the points if they joined the union.
  expect(result.range_after).toEqual(result.range_before);
  expect(result.time_alignment).toBe("as_of");
  expect(result.union_points_grew).toBe(true);
  expect(result.invalid).toBe("invalid_options");
  // A refused first call adopts nothing: the retry still adopts the primary, and later refused
  // adds leave no series behind.
  expect(result.first_invalid).toBe("invalid_options");
  expect(result.primary).toBe(0);
  expect(result.ids_after_invalid).toEqual(result.ids_before_invalid);
});
