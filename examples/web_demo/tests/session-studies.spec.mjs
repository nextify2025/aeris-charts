import { test, expect } from "@playwright/test";

test("session studies expose exchange, UTC and host-boundary values with choice schemas", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    const start = Date.UTC(2024, 0, 1) / 1000;
    source.set_data([
      { time: start, open: 9, high: 10, low: 8, close: 9 },
      { time: start + 600, open: 10, high: 15, low: 7, close: 14 },
      { time: start + 1200, open: 12, high: 13, low: 9, close: 12 },
      { time: start + 86400, open: 19, high: 20, low: 18, close: 19 },
      { time: start + 87000, open: 20, high: 25, low: 17, close: 22 },
    ]);
    // The UTC policy is explicit: the default calendar is the chart's exchange calendar, which
    // on this UTC chart (midnight session start) yields the same values.
    const utc = {
      session: chart.add_session_levels(source, "utc"),
      previous: chart.add_previous_period_levels(source, "day", "utc"),
      opening: chart.add_opening_range(source, 900, "utc"),
    };
    const exchange = {
      session: chart.add_session_levels(source),
      previous: chart.add_previous_period_levels(source),
      opening: chart.add_opening_range(source, 900),
    };
    chart.set_study_calendar([
      { startTime: start, endTime: start + 900, sessionId: 1 },
      { startTime: start + 900, endTime: start + 2 * 86400, sessionId: 2 },
    ]);
    const host = {
      session: chart.add_session_levels(source, "host"),
      previous: chart.add_previous_period_levels(source, "day", "host"),
      opening: chart.add_opening_range(source, 900, "host"),
    };
    const values = (outputs) => outputs.map((output) => ({
      kind: output.indicator_info().kind,
      parameters: output.indicator_info().parameters,
      values: output.data().map(({ value }) => value ?? null),
      times: output.data().map(({ time }) => time),
    }));
    const schemas = ["session_levels", "previous_period_levels", "opening_range"]
      .map((kind) => chart.indicator_schema(kind));
    const snapshot = {
      utc: Object.fromEntries(Object.entries(utc).map(([kind, outputs]) => [kind, values(outputs)])),
      exchange: Object.fromEntries(Object.entries(exchange).map(([kind, outputs]) => [kind, values(outputs)])),
      host: Object.fromEntries(Object.entries(host).map(([kind, outputs]) => [kind, values(outputs)])),
      schemas,
    };
    chart.remove_series(source);
    return snapshot;
  });
  const start = Date.UTC(2024, 0, 1) / 1000;
  expect(result.utc.session.map(({ values }) => values)).toEqual([
    [10, 15, 15, 20, 25], [8, 7, 7, 18, 17],
  ]);
  expect(result.utc.previous.map(({ values }) => values)).toEqual([
    [15, 15], [7, 7], [12, 12],
  ]);
  expect(result.utc.previous.map(({ times }) => times)).toEqual(
    Array.from({ length: 3 }, () => [start + 86400, start + 87000]),
  );
  expect(result.utc.opening.map(({ values }) => values)).toEqual([
    [10, 15, 15, 20, 25], [8, 7, 7, 18, 17], [9, 11, 11, 19, 21],
  ]);
  expect(result.host.session.map(({ values }) => values)).toEqual([
    [10, 15, 13, 20, 25], [8, 7, 9, 9, 9],
  ]);
  expect(result.host.previous.map(({ values }) => values)).toEqual([
    [15, 15, 15], [7, 7, 7], [14, 14, 14],
  ]);
  expect(result.host.previous.map(({ times }) => times)).toEqual(
    Array.from({ length: 3 }, () => [start + 1200, start + 86400, start + 87000]),
  );
  expect(result.host.opening.map(({ values }) => values)).toEqual([
    [10, 15, 13, 13, 13], [8, 7, 9, 9, 9], [9, 11, 11, 11, 11],
  ]);
  expect(result.utc.session[0].parameters).toMatchObject({ calendar: "utc" });
  expect(result.utc.previous[0].parameters).toMatchObject({ calendar: "utc", previous_period: "day" });
  expect(result.utc.opening[0].parameters).toMatchObject({ calendar: "utc", duration_seconds: 900 });
  for (const kind of ["session", "previous", "opening"]) {
    expect(result.exchange[kind].map(({ values }) => values)).toEqual(result.utc[kind].map(({ values }) => values));
    expect(result.exchange[kind][0].parameters).toMatchObject({ calendar: "exchange" });
  }
  expect(result.host.session[0].parameters).toMatchObject({ calendar: "host" });
  expect(result.host.previous[0].parameters).toMatchObject({ calendar: "host", previous_period: "day" });
  expect(result.host.opening[0].parameters).toMatchObject({ calendar: "host", duration_seconds: 900 });
  for (const [kind, count] of [["session_levels", 2], ["previous_period_levels", 3], ["opening_range", 3]]) {
    const schema = result.schemas.find((item) => item.kind === kind);
    expect(schema.outputs).toHaveLength(count);
    expect(schema.parameters.find(({ name }) => name === "calendar")).toMatchObject({
      parameter_type: "choice", default: "exchange", options: ["exchange", "utc", "host"],
    });
  }
  expect(result.schemas[1].parameters.find(({ name }) => name === "period")).toMatchObject({
    parameter_type: "choice", default: "day", options: ["day", "week", "month"],
  });
});

test("exchange calendar counts night sessions in the next trading week and follows time zone changes", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick", { visible: false });
    // SHFE-style bars in CST (UTC+8); the Friday 21:00 night session trades Monday's day.
    const cst = (month, day, hour, minute) => Date.UTC(2024, month - 1, day, hour - 8, minute) / 1000;
    const rows = [
      [cst(1, 31, 9, 0), 10, 5], [cst(1, 31, 21, 0), 20, 15], [cst(2, 1, 0, 30), 22, 14],
      [cst(2, 1, 9, 0), 21, 16], [cst(2, 2, 21, 0), 30, 25], [cst(2, 3, 1, 0), 31, 24],
      [cst(2, 5, 9, 0), 32, 26],
    ];
    source.set_data(rows.map(([time, high, low]) => ({ time, open: (high + low) / 2, high, low, close: (high + low) / 2 })));
    const week = chart.add_previous_period_levels(source, "week");
    const month = chart.add_previous_period_levels(source, "month", "exchange");
    const utc_week = chart.add_previous_period_levels(source, "week", "utc");
    const read = (output) => {
      const by_time = new Map(output.data().map(({ time, value }) => [time, value ?? null]));
      return rows.map(([time]) => by_time.get(time) ?? null);
    };
    const before = { week: read(week[0]), month: read(month[0]) };
    chart.time_scale().apply_options({ time_zone: "Asia/Shanghai", session_start: -3 * 3_600 });
    const shanghai = { week: read(week[0]), month: read(month[0]), utc_week: read(utc_week[0]) };
    chart.time_scale().apply_options({ time_zone: "Etc/UTC", session_start: 0 });
    const restored = { week: read(week[0]), month: read(month[0]) };
    chart.remove_series(source);
    return { before, shanghai, restored };
  });
  // UTC: the week turns on Monday 09:00 CST and the month on Thursday 09:00 CST.
  expect(result.before.week).toEqual([null, null, null, null, null, null, 31]);
  expect(result.before.month).toEqual([null, null, null, 22, 22, 22, 22]);
  // Exchange: Friday night opens Monday's week and Wednesday night already trades February.
  expect(result.shanghai.week).toEqual([null, null, null, null, 22, 22, 22]);
  expect(result.shanghai.month).toEqual([null, 10, 10, 10, 10, 10, 10]);
  expect(result.shanghai.utc_week).toEqual(result.before.week);
  expect(result.restored).toEqual(result.before);
});

test("session study arguments reject invalid choices before engine mutation", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const source = chart.add_series("candlestick");
    const count = chart.series_order().length;
    const errors = [
      () => chart.add_session_levels(source, "local"),
      () => chart.add_session_levels(source, null),
      () => chart.add_previous_period_levels(source, "year"),
      () => chart.add_previous_period_levels(source, "day", "local"),
      () => chart.add_opening_range(source, 0),
      () => chart.add_opening_range(source, -1),
      () => chart.add_opening_range(source, 1.5),
      () => chart.add_opening_range(source, 4294967296),
      () => chart.add_opening_range(source, 300, "local"),
    ].map((invoke) => {
      try { invoke(); return null; } catch (error) { return error.code; }
    });
    return { errors, count, after: chart.series_order().length };
  });
  expect(result.errors).toEqual(Array(9).fill("invalid_options"));
  expect(result.after).toBe(result.count);
});

test("demo catalog creates and clears every session study output", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  for (const [kind, count] of [["session_levels", 2], ["previous_period_levels", 3], ["opening_range", 3]]) {
    const toggle = page.locator(`#${kind}_toggle`);
    await toggle.check();
    expect(await page.evaluate((id) => window.__demo_indicators.get(id)?.outputs.length, kind)).toBe(count);
    await toggle.uncheck();
    expect(await page.evaluate((id) => window.__demo_indicators.has(id), kind)).toBe(false);
  }
  await expect(page.locator("#indicator_error")).toBeEmpty();
});

test("all seven study kinds inherit the source pane without creating oscillator panes", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const pane = chart.add_pane();
    const source = chart.add_series("candlestick");
    source.move_to_pane(pane.pane_index());
    source.set_data([
      { time: 1_700_000_000, open: 9, high: 10, low: 8, close: 9 },
      { time: 1_700_000_060, open: 10, high: 15, low: 9, close: 14 },
      { time: 1_700_000_120, open: 12, high: 13, low: 10, close: 12 },
    ]);
    const groups = [
      chart.add_session_levels(source),
      chart.add_previous_period_levels(source),
      chart.add_opening_range(source, 60),
      chart.add_swing_points(source, 1, 1),
      [chart.add_market_structure(source, 1, 1)],
      [chart.add_fair_value_gaps(source)],
      [chart.add_order_blocks(source, { left: 1, right: 1 })],
    ];
    return {
      sourcePane: source.pane_index(),
      paneCount: chart.panes().length,
      outputs: groups.map((group) => group.map((output) => ({
        pane: output.pane_index(), kind: output.indicator_info().kind,
      }))),
    };
  });
  expect(result.sourcePane).toBe(1);
  expect(result.paneCount).toBe(2);
  expect(result.outputs.map((group) => group.length)).toEqual([2, 3, 3, 2, 1, 1, 1]);
  expect(result.outputs.map((group) => group[0].kind)).toEqual([
    "session_levels", "previous_period_levels", "opening_range",
    "swing_points", "market_structure", "fair_value_gaps", "order_blocks",
  ]);
  expect(result.outputs.flat().every(({ pane }) => pane === result.sourcePane), JSON.stringify(result)).toBe(true);
});
