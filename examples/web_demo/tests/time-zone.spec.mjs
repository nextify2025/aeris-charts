import { test, expect } from "@playwright/test";

// Exchange time zone, trading-day session start, calendar-date context, host time formatters,
// and the host countdown clock, exercised through the public browser API.

async function open_chart(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  await next_frames(page);
}

async function next_frames(page) {
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
});

test("time_zone and session_start move tick boundaries seen by tick_mark_formatter", async ({ page }) => {
  await open_chart(page);
  await page.evaluate(() => {
    // China futures: Tuesday day session, Tuesday night session, Wednesday morning (CST, 30 min).
    const cst = (day, hour, minute) => Date.UTC(2024, 0, day, hour - 8, minute) / 1000;
    const times = [];
    const session = (day, from, to) => {
      for (let time = cst(day, ...from); time <= cst(day, ...to); time += 1_800) times.push(time);
    };
    session(2, [13, 30], [15, 0]);
    session(2, [21, 0], [23, 0]);
    session(3, [9, 0], [11, 30]);
    window.__tz_times = times;
    window.__main.set_data(times.map((time, index) => ({
      time, open: 100 + index, high: 101 + index, low: 99 + index, close: 100.5 + index,
    })));
    window.__tick_calls = [];
    window.__chart.time_scale().apply_options({
      time_visible: true,
      tick_mark_formatter: (time, type, locale, context) => {
        window.__tick_calls.push({ time, type, locale, business_day: context.business_day });
        return "";
      },
    });
    window.__chart.time_scale().fit_content();
  });
  await next_frames(page);
  const day_marks = () => page.evaluate(() => {
    const marks = [...new Set(window.__tick_calls.filter((call) => call.type <= 2).map((call) => call.time))];
    const calls = window.__tick_calls;
    window.__tick_calls = [];
    return { marks, sample: calls[0] ?? null, times: window.__tz_times };
  });
  const utc = await day_marks();
  // UTC: the new day starts at 00:00 UTC, i.e. at the 09:00 CST bar.
  const nine_am = utc.times[utc.times.length - 6];
  const night = utc.times[4];
  expect(utc.marks).toContain(nine_am);
  expect(utc.marks).not.toContain(night);
  expect(typeof utc.sample.locale).toBe("string");
  expect(utc.sample.business_day).toBeNull();

  await page.evaluate(() => {
    window.__chart.time_scale().apply_options({ time_zone: "Asia/Shanghai", session_start: -3 * 3_600 });
  });
  await next_frames(page);
  const exchange = await day_marks();
  // The 21:00 night session opens the next trading day; 09:00 continues it.
  expect(exchange.marks).toContain(night);
  expect(exchange.marks).not.toContain(nine_am);
  const options = await page.evaluate(() => window.__chart.time_scale().options());
  expect(options.time_zone).toBe("Asia/Shanghai");
  expect(options.session_start).toBe(-10_800);

  // An unknown zone is rejected atomically with a stable error code.
  const rejected = await page.evaluate(() => {
    try {
      window.__chart.time_scale().apply_options({ time_zone: "Mars/Olympus_Mons", bar_spacing: 3 });
      return null;
    } catch (error) {
      return { code: error.code, spacing: window.__chart.time_scale().options().bar_spacing };
    }
  });
  expect(rejected?.code).toBe("invalid_options");
  expect(rejected.spacing).not.toBe(3);
});

test("time_formatter receives calendar-date context and drives the tooltip", async ({ page }) => {
  await open_chart(page);
  const target = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    window.__formatter_contexts = [];
    chart.apply_options({
      timeScale: { timeZone: "America/New_York" },
      localization: {
        locale: "en-US",
        time_formatter: (time, context) => {
          window.__formatter_contexts.push(context.business_day);
          return `T${time}`;
        },
      },
    });
    window.__main.set_data(Array.from({ length: 30 }, (_, index) => ({
      time: { year: 2024, month: 1, day: index + 1 },
      open: 100 + index, high: 102 + index, low: 99 + index, close: 101 + index,
    })));
    chart.time_scale().fit_content();
    window.__tooltip = api.create_tooltip(chart, { series: window.__main });
    const logical = 7;
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    return {
      x: bounds.left + pane.left + chart.time_scale().logical_to_coordinate(logical),
      y: bounds.top + pane.top + pane.height * 0.5,
      time: Date.UTC(2024, 0, 8) / 1000,
    };
  });
  await page.mouse.move(target.x, target.y);
  await expect.poll(() => page.locator(".aeris_charts-tooltip").evaluate((element) => element.style.opacity)).toBe("1");
  const hosted = await page.evaluate(() => ({
    timestamp: document.querySelector(".aeris_charts-tooltip__timestamp")?.textContent,
    contexts: window.__formatter_contexts,
  }));
  expect(hosted.timestamp).toBe(`T${target.time}`);
  // The crosshair label and the tooltip both asked the host formatter with the calendar date.
  expect(hosted.contexts).toContainEqual({ year: 2024, month: 1, day: 8 });
  expect(hosted.contexts.every((context) => context !== null)).toBe(true);

  // Built-in text: the calendar date stays Jan 8 (and prints no clock time) even though UTC midnight is Jan 7 in New York.
  await page.evaluate(() => window.__chart.apply_options({ localization: { time_formatter: null } }));
  await page.mouse.move(target.x + 1, target.y);
  await expect.poll(() => page.locator(".aeris_charts-tooltip__timestamp").textContent()).toBe("08 Jan '24");
});

test("tooltip and accessibility show exchange-local intraday times", async ({ page }) => {
  await open_chart(page);
  const target = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    // A-share session with the lunch break: 09:30..11:30 and 13:00..15:00 CST, 30-minute bars.
    const cst = (hour, minute) => Date.UTC(2024, 0, 2, hour - 8, minute) / 1000;
    const times = [];
    for (let time = cst(9, 30); time <= cst(11, 30); time += 1_800) times.push(time);
    for (let time = cst(13, 0); time <= cst(15, 0); time += 1_800) times.push(time);
    window.__main.set_data(times.map((time, index) => ({
      time, open: 10 + index, high: 11 + index, low: 9 + index, close: 10.5 + index,
    })));
    chart.apply_options({ localization: { locale: "en-US" }, timeScale: { timeZone: "Asia/Shanghai" } });
    chart.time_scale().fit_content();
    window.__tooltip = api.create_tooltip(chart, { series: window.__main });
    const lunch_close = 4; // 11:30
    const afternoon = 5; // 13:00
    const x = (logical) => chart.time_scale().logical_to_coordinate(logical);
    const pane = chart.panes()[0].get_geometry();
    const bounds = chart.chart_element().getBoundingClientRect();
    return {
      spacing: x(afternoon) - x(lunch_close),
      bar_spacing: x(1) - x(0),
      x: bounds.left + pane.left + x(afternoon),
      y: bounds.top + pane.top + pane.height * 0.5,
    };
  });
  // No gap across the lunch break.
  expect(Math.abs(target.spacing - target.bar_spacing)).toBeLessThan(1e-6);
  await page.mouse.move(target.x, target.y);
  await expect.poll(() => page.locator(".aeris_charts-tooltip__timestamp").textContent()).toBe("02 Jan '24   13:00");

  const announced = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const accessibility = api.enable_accessibility(window.__chart, { data_scope: "all" });
    accessibility.focus(0);
    const layer = document.activeElement;
    layer.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return window.__chart.chart_element().querySelector(".aeris_charts-a11y-live-region")?.textContent ?? "";
  });
  expect(announced).toContain("02 Jan '24   15:00");
});

test("the host clock drives the countdown, which hides outside the forming bar", async ({ page }) => {
  await open_chart(page);
  const last = await page.evaluate(() => {
    const start = Date.UTC(2024, 0, 2, 1, 30) / 1000;
    window.__main.set_data(Array.from({ length: 40 }, (_, index) => ({
      time: start + index * 3_600, open: 100, high: 101, low: 99, close: 100 + (index % 3),
    })));
    window.__main.apply_options({ countdown_visible: true, price_line_visible: false });
    window.__chart.time_scale().fit_content();
    return start + 39 * 3_600;
  });
  const shot = async (clock, countdown = true) => {
    await page.evaluate(({ clock, countdown }) => {
      window.__main.apply_options({ countdown_visible: countdown });
      window.__chart.set_clock(() => clock);
    }, { clock, countdown });
    await next_frames(page);
    return page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  };
  const hidden = await shot(last + 60, false);
  const inside = await shot(last + 60);
  // After the forming bar's interval (an overnight gap) the countdown hides instead of cycling.
  const after = await shot(last + 3_600 + 5);
  const before = await shot(last - 5);
  expect(inside).not.toBe(hidden);
  expect(after).toBe(hidden);
  expect(before).toBe(hidden);
  await page.evaluate(() => window.__chart.set_clock(null));
});

test("session highlighting accepts fractional exchange-local hours", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    const start = Date.UTC(2024, 0, 2, 13, 0) / 1000; // 08:00 ET
    window.__main.set_data(Array.from({ length: 20 }, (_, index) => ({
      time: start + index * 1_800, open: 100, high: 101, low: 99, close: 100,
    })));
    chart.time_scale().fit_content();
    const shot = async () => {
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return chart.take_screenshot().toDataURL("image/png");
    };
    const options = { start_hour: 9.5, end_hour: 16, weekday_color: "rgba(0, 0, 255, 0.4)" };
    const utc_handle = api.create_session_highlighting(window.__main, options);
    const utc = await shot();
    chart.time_scale().apply_options({ time_zone: "America/New_York" });
    const eastern = await shot();
    utc_handle.detach();
    let rejected = null;
    try {
      api.create_session_highlighting(window.__main, { start_hour: 9.5 });
    } catch (error) {
      rejected = error.code;
    }
    return { differs: utc !== eastern, rejected };
  });
  // The same 09:30..16:00 gate shades different bars once hours are exchange-local.
  expect(result.differs).toBe(true);
  expect(result.rejected).toBe("invalid_data");
});

test("session highlighting callbacks stay incremental under max_points retention", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(async () => {
    const api = await import("/dist/aeris_charts_financial.js");
    const series = window.__main;
    const start = Date.UTC(2024, 0, 2) / 1000;
    series.set_data(Array.from({ length: 40 }, (_, index) => ({
      time: start + index * 60, open: 100, high: 101, low: 99, close: 100,
    })));
    series.apply_options({ max_points: 40 });
    let calls = 0;
    const handle = api.create_session_highlighting(series, () => {
      calls += 1;
      return "rgba(0, 0, 255, 0.2)";
    });
    const initial = calls;
    // Each append now evicts the oldest row; only the appended row is evaluated.
    for (let index = 40; index < 45; index += 1) {
      series.update({ time: start + index * 60, open: 100, high: 101, low: 99, close: 100 });
    }
    const after = calls;
    const first = series.data()[0].time;
    handle.detach();
    return { initial, after, first, start, length: series.data().length };
  });
  expect(result.initial).toBe(40);
  // Retention evicted history (the cap trims with hysteresis), yet only appended rows ran.
  expect(result.first).toBeGreaterThan(result.start);
  expect(result.length).toBeLessThanOrEqual(40);
  expect(result.after).toBe(result.initial + 5);
});

test("a bar that opens after a quiet gap shows its countdown on arrival", async ({ page }) => {
  await open_chart(page);
  const last = await page.evaluate(() => {
    const start = Date.UTC(2024, 0, 2, 1, 30) / 1000;
    window.__main.set_data(Array.from({ length: 40 }, (_, index) => ({
      time: start + index * 3_600, open: 100, high: 101, low: 99, close: 100 + (index % 3),
    })));
    window.__main.apply_options({ countdown_visible: true, price_line_visible: false });
    window.__chart.time_scale().fit_content();
    return start + 39 * 3_600;
  });
  const shot = () => page.evaluate(() => window.__chart.take_screenshot().toDataURL("image/png"));
  // Two hours after the last bar's interval: nothing is forming, so no countdown.
  await page.evaluate((now) => {
    window.__clock_now = now;
    window.__chart.set_clock(() => window.__clock_now);
  }, last + 2 * 3_600);
  await next_frames(page);
  // The next bar opens three hours after the last one; the clock moves with it. The countdown
  // must use the clock at data arrival rather than the value pinned by the last 1 s tick.
  await page.evaluate((bar) => {
    window.__clock_now = bar + 5;
    window.__main.update({ time: bar, open: 100, high: 101, low: 99, close: 100 });
  }, last + 3 * 3_600);
  await next_frames(page);
  const arrived = await shot();
  await page.evaluate(() => window.__main.apply_options({ countdown_visible: false }));
  await next_frames(page);
  const hidden = await shot();
  expect(arrived).not.toBe(hidden);
  await page.evaluate(() => window.__chart.set_clock(null));
});
