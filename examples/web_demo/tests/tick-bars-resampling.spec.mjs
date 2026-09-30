import { test, expect } from "@playwright/test";

// Ticks to ordinary candles and OHLCV resampling through the public package API: exchange-session
// anchoring in the chart's time zone, auction/lunch/closing prints, live roll-over, the stream
// volume histogram, replay, session-derived resampling boundaries, bounded live tail refreshes,
// and US daily bars across DST. The browser runs in New York so every exchange-local result must
// come from the chart's zone, not the host's.

test.use({ timezoneId: "America/New_York" });

async function open_chart(page) {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
  return errors;
}

async function settle(page) {
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

const A_SHARE = [["09:30", "11:30"], ["13:00", "15:00"]];

test("trades build A-share session candles and a volume histogram, live and replayed", async ({ page }) => {
  const errors = await open_chart(page);
  const result = await page.evaluate((windows) => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    chart.time_scale().apply_options({ time_zone: "Asia/Shanghai", time_visible: true });
    // Exchange-local Shanghai wall clock to UTC microseconds.
    const micros = (date, clock) => {
      const [year, month, day] = date.split("-").map(Number);
      const [hour, minute, second] = clock.split(":").map(Number);
      return Date.UTC(year, month - 1, day, hour - 8, minute, second) * 1000;
    };
    const local = (seconds) => new Date((seconds + 8 * 3600) * 1000).toISOString().slice(11, 16);
    const day = (date) => [
      ["09:25:00", 10.00, 500],
      ["09:30:01", 10.02, 100],
      ["10:29:59", 10.05, 100],
      ["10:30:00", 10.04, 100],
      ["11:29:59", 10.03, 100],
      ["11:30:00", 10.01, 50],
      ["13:00:02", 10.06, 100],
      ["14:00:00", 10.07, 100],
      ["14:59:59", 10.08, 100],
      ["15:00:00", 10.10, 800],
    ].map(([clock, price, volume]) => ({
      timestamp_micros: micros(date, clock), price, volume, aggressor: "buy", session_id: 1,
    }));

    const candles = chart.add_series("candlestick");
    const options = { tick_size: 0.01, bar_type: "time", interval_seconds: 3600 };
    const stream = chart.add_trade_stream("SSE:600000", options);
    chart.bind_trade_bar_series_to_stream(candles, stream);
    const volume = chart.add_trade_volume_series(stream, 1);
    chart.set_trade_stream_sessions(stream, { windows });

    const tape = day("2026-09-25");
    chart.set_trade_stream_trades(stream, tape.slice(0, 3));
    const live = tape.slice(3).map((print) => ({
      kind: chart.update_trade_stream_trades(stream, [print]),
      bars: candles.data().length,
    }));
    const bars = candles.data();
    const volume_rows = volume.data();
    chart.time_scale().fit_content();

    // Replay: seeking equals loading the tape up to the clock (a second stream holds that prefix).
    const two_days = [...day("2026-09-24"), ...tape];
    chart.set_trade_stream_trades(stream, two_days);
    const prefix_candles = chart.add_series("candlestick");
    const prefix = chart.add_trade_stream("SSE:600000:prefix", options);
    chart.bind_trade_bar_series_to_stream(prefix_candles, prefix);
    chart.set_trade_stream_sessions(prefix, { windows });
    const replay = [];
    for (const clock of [micros("2026-09-25", "11:00:00"), micros("2026-09-24", "14:30:00"), micros("2026-09-25", "15:00:00")]) {
      chart.set_trade_stream_trades(prefix, two_days.filter((print) => print.timestamp_micros <= clock));
      chart.set_replay_clock_micros(clock);
      replay.push({ seek: candles.data(), fresh: prefix_candles.data() });
    }
    chart.set_replay_clock_micros(null);
    chart.remove_series(prefix_candles);

    const errors = [];
    for (const attempt of [
      () => chart.set_trade_stream_sessions(stream, { windows: [["13:00", "15:00"], ["09:30", "11:30"]] }),
      () => chart.set_trade_stream_sessions(stream, { windows, outside: "drop" }),
      () => chart.set_trade_stream_sessions(chart.add_trade_stream("SSE:600000:ticks", { tick_size: 0.01, bar_type: "trades", trades_per_bar: 5 }), { windows }),
    ]) {
      try {
        attempt();
        errors.push(null);
      } catch (error) {
        errors.push(error.code);
      }
    }
    return {
      times: bars.map((bar) => local(bar.time)),
      ohlc: bars.map(({ open, high, low, close }) => [open, high, low, close]),
      volume: volume_rows.map((row) => row.value),
      volume_options: volume.options(),
      live,
      replay,
      after_replay: candles.data().length,
      stats: chart.trade_stream_stats(stream),
      errors,
    };
  }, A_SHARE);

  expect(errors).toEqual([]);
  expect(result.times).toEqual(["09:30", "10:30", "13:00", "14:00"]);
  expect(result.ohlc).toEqual([
    [10.00, 10.05, 10.00, 10.05],
    [10.04, 10.04, 10.01, 10.01],
    [10.06, 10.06, 10.06, 10.06],
    [10.07, 10.10, 10.07, 10.10],
  ]);
  expect(result.volume).toEqual([700, 250, 100, 1000]);
  expect(result.volume_options).toMatchObject({ histogram_updown: true });
  expect(result.live.map((entry) => entry.kind)).toEqual(Array(7).fill("tip"));
  expect(result.live.map((entry) => entry.bars)).toEqual([2, 2, 2, 3, 4, 4, 4]);
  for (const { seek, fresh } of result.replay) {
    expect(seek).toEqual(fresh);
    expect(seek.length).toBeGreaterThan(0);
  }
  expect(result.after_replay).toBe(8);
  expect(result.stats.dependent_count).toBeGreaterThanOrEqual(2);
  expect(result.errors).toEqual(["invalid_options", "invalid_options", "invalid_options"]);

  // The shared frame paints the candles and the stream volume columns.
  await settle(page);
  const painted = await page.evaluate(() => {
    const chart = window.__chart;
    chart.time_scale().fit_content();
    const shot = chart.take_screenshot(true, false);
    const ratio = shot.width / chart.chart_element().clientWidth;
    const context = shot.getContext("2d");
    return chart.panes().map((pane) => {
      const { left, top, width, height } = pane.get_geometry();
      const pixels = context.getImageData(
        Math.round(left * ratio), Math.round(top * ratio), Math.round(width * ratio), Math.round(height * ratio),
      ).data;
      let colored = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        const [red, green, blue] = [pixels[index], pixels[index + 1], pixels[index + 2]];
        if (Math.max(red, green, blue) - Math.min(red, green, blue) > 40) colored += 1;
      }
      return colored;
    });
  });
  expect(painted.length).toBe(2);
  expect(painted[0]).toBeGreaterThan(50);
  expect(painted[1]).toBeGreaterThan(50);
});

test("minute bars resample to A-share 60-minute bars and follow live updates", async ({ page }) => {
  const errors = await open_chart(page);
  const result = await page.evaluate(async (windows) => {
    const { resample_boundaries, session_slot_times } = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.remove_series(window.__main);
    chart.time_scale().apply_options({ time_zone: "Asia/Shanghai" });
    const dates = ["2026-09-24", "2026-09-25"];
    const slots = dates.flatMap((date) => session_slot_times({
      date, windows, interval_seconds: 60, time_zone: "Asia/Shanghai",
    }));
    const price = (index) => 10 + ((index * 7) % 23) / 100;
    const rows = slots.map((time, index) => ({
      time, open: price(index), high: price(index) + 0.05, low: price(index) - 0.03, close: price(index) + 0.01,
    }));
    const volumes = slots.map((time, index) => ({ time, value: 100 + (index % 7) }));
    const minute = chart.add_series("candlestick");
    const minute_volume = chart.add_series("histogram", { pane: 1 });
    // Everything but the last five minutes is history; those stream in live.
    minute.set_data(rows.slice(0, -5));
    minute_volume.set_data(volumes.slice(0, -5));
    const hour = chart.add_series("candlestick");
    const hour_volume = chart.add_series("histogram", { pane: 1 });
    const boundaries = resample_boundaries({ dates, windows, time_zone: "Asia/Shanghai" });
    chart.configure_resampled_series(hour, {
      source: minute, volume_source: minute_volume, volume_target: hour_volume,
      interval_seconds: 3600, boundaries,
    });
    const configured = chart.resample_stats(hour);
    const live = [];
    for (let index = rows.length - 5; index < rows.length; index += 1) {
      const before = chart.resample_stats(hour);
      minute.update(rows[index]);
      minute_volume.update(volumes[index]);
      const after = chart.resample_stats(hour);
      live.push({
        rebuilds: after.rebuilds - before.rebuilds,
        scanned: after.rows_scanned - before.rows_scanned,
        close: hour.data().at(-1).close,
      });
    }
    // Refining the forming minute only touches its bar.
    minute.update({ ...rows.at(-1), high: 12, close: 11.5 });
    const refined = hour.data().at(-1);
    // Every host write path to an engine-owned target is rejected and changes nothing; a merge
    // used to write straight into the derived rows.
    const derived_before = { hour: hour.data(), volume: hour_volume.data() };
    const writes = {};
    const codes = {};
    const one = (value) => Float64Array.of(value);
    for (const [name, target, attempt] of [
      ["set_data", hour, () => hour.set_data([{ time: slots[0], open: 1, high: 2, low: 0, close: 1 }])],
      ["update", hour, () => hour.update({ time: slots[0], open: 1, high: 2, low: 0, close: 1 })],
      ["update_typed", hour, () => hour.update_typed({
        times: one(slots[0]), open: one(1), high: one(2), low: one(0), close: one(1),
      })],
      ["merge", hour, () => hour.merge({ time: slots[0], high: 99 })],
      ["merge_sequenced", hour, () => hour.merge({ time: slots[0], close: 5 }, { sequence: 9 })],
      ["volume_update", hour_volume, () => hour_volume.update({ time: slots[0], value: 1 })],
      ["volume_merge", hour_volume, () => hour_volume.merge({ time: slots[0], value: 1 })],
    ]) {
      attempt();
      writes[name] = target.last_ingestion_diagnostics()?.status ?? null;
      codes[name] = target.last_ingestion_diagnostics()?.code ?? null;
    }
    hour.pop(1);
    codes.pop = hour.last_ingestion_diagnostics()?.code ?? null;
    const derived_after = { hour: hour.data(), volume: hour_volume.data() };
    const expected = boundaries.flatMap((boundary) => {
      const bars = [];
      for (let open = boundary.start_time; open < boundary.end_time; open += 3600) {
        const inside = rows.filter((row) => row.time >= open && row.time < Math.min(open + 3600, boundary.end_time));
        if (inside.length === 0) continue;
        bars.push({ time: open, rows: inside.length });
      }
      return bars;
    });
    return {
      local: hour.data().map((bar) => new Date((bar.time + 8 * 3600) * 1000).toISOString().slice(11, 16)),
      boundaries,
      bars: chart.resampled_bars(hour),
      expected,
      volume: hour_volume.data().map((row) => row.value),
      first_hour_volume: volumes.slice(0, 60).reduce((sum, row) => sum + row.value, 0),
      configured,
      live,
      last_close: rows.at(-1).close,
      refined,
      writes,
      codes,
      derived_unchanged: JSON.stringify(derived_before) === JSON.stringify(derived_after),
      unbound: chart.resampled_bars(minute),
    };
  }, A_SHARE);

  expect(errors).toEqual([]);
  expect(result.boundaries).toHaveLength(4);
  expect(result.boundaries[0].session_id).toBe(20260924);
  expect(result.local).toEqual(["09:30", "10:30", "13:00", "14:00", "09:30", "10:30", "13:00", "14:00"]);
  expect(result.bars.map((bar) => ({ time: bar.time, rows: bar.source_rows }))).toEqual(result.expected);
  expect(result.bars.every((bar) => bar.source_rows === 60)).toBe(true);
  expect(result.volume[0]).toBe(result.first_hour_volume);
  expect(result.configured).toMatchObject({ rebuilds: 1, tail_refreshes: 0 });
  for (const step of result.live) {
    expect(step.rebuilds).toBe(0);
    // Source and volume each re-read at most one 60-minute bucket.
    expect(step.scanned).toBeLessThanOrEqual(2 * 61);
  }
  expect(result.live.at(-1).close).toBeCloseTo(result.last_close, 10);
  expect(result.refined).toMatchObject({ high: 12, close: 11.5 });
  expect(result.writes).toEqual({
    set_data: "rejected",
    update: "rejected",
    update_typed: "rejected",
    merge: "rejected",
    merge_sequenced: "rejected",
    volume_update: "rejected",
    volume_merge: "rejected",
  });
  // The rejection names its cause, so a host can tell it from an invalid payload.
  expect(Object.values(result.codes)).toEqual(Array(8).fill("derived_series"));
  expect(result.derived_unchanged).toBe(true);
  expect(result.local).toHaveLength(8);
  expect(result.unbound).toBeNull();
});

test("trade-bound candles and studies reject every host write and stay fed by their stream", async ({ page }) => {
  const errors = await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const second = (index) => 1_700_000_000 + index;
    const print = (index) => ({
      timestamp_micros: second(index * 7) * 1_000_000,
      price: 100 + ((index * 7) % 11) / 4,
      volume: 1 + (index % 5),
      aggressor: index % 3 === 0 ? "sell" : "buy",
      session_id: 1,
    });
    const tape = Array.from({ length: 260 }, (_, index) => print(index));
    const options = { tick_size: 0.25, bar_type: "time", interval_seconds: 60 };
    const build = (key) => {
      const candles = chart.add_series("candlestick");
      const stream = chart.add_trade_stream(key, options);
      chart.bind_trade_bar_series_to_stream(candles, stream);
      return {
        stream,
        candles,
        volume: chart.add_trade_volume_series(stream, 1),
        cvd: chart.add_cvd_series(stream, 2, "continuous"),
        delta: chart.add_delta_series(stream, 2),
      };
    };
    const live = build("TEST:LIVE");
    chart.set_trade_stream_trades(live.stream, tape.slice(0, 200));

    const warnings = [];
    const original_warn = console.warn;
    console.warn = (...args) => { warnings.push(args.join(" ")); };
    const one = (value) => Float64Array.of(value);
    const time = second(60 * 3);
    const attempts = {
      set_data: (handle, scalar) => handle.set_data([scalar ? { time, value: 1 } : { time, open: 1, high: 2, low: 0, close: 1 }]),
      set_data_typed: (handle) => handle.set_data_typed({ times: one(time), open: one(1), high: one(2), low: one(0), close: one(1) }),
      update: (handle, scalar) => handle.update(scalar ? { time, value: 1 } : { time, open: 1, high: 2, low: 0, close: 1 }),
      update_typed: (handle) => handle.update_typed({ times: one(time), open: one(1), high: one(2), low: one(0), close: one(1) }),
      update_sequenced: (handle, scalar) => handle.update(scalar ? { time, value: 1 } : { time, open: 1, high: 2, low: 0, close: 1 }, { sequence: 3 }),
      update_typed_sequenced: (handle) => handle.update_typed({ times: one(time), open: one(1), high: one(2), low: one(0), close: one(1) }, { sequence: 3 }),
      merge: (handle) => handle.merge({ time, close: 99 }),
      merge_typed: (handle) => handle.merge_typed({ times: one(time), close: one(99) }),
      merge_sequenced: (handle) => handle.merge({ time, close: 5 }, { sequence: 9 }),
      pop: (handle) => handle.pop(1),
    };
    const outcomes = {};
    for (const [name, handle] of Object.entries({ candles: live.candles, volume: live.volume, cvd: live.cvd, delta: live.delta })) {
      const scalar = name !== "candles";
      const before = JSON.stringify(handle.data());
      let notified = 0;
      const on_change = () => { notified += 1; };
      handle.subscribe_data_changed(on_change);
      const seen = {};
      for (const [attempt, run] of Object.entries(attempts)) {
        const warned = warnings.length;
        run(handle, scalar);
        const diagnostics = handle.last_ingestion_diagnostics();
        seen[attempt] = { status: diagnostics?.status, code: diagnostics?.code, warned: warnings.length > warned };
      }
      handle.unsubscribe_data_changed(on_change);
      outcomes[name] = { seen, notified, unchanged: JSON.stringify(handle.data()) === before };
    }

    // A ring cannot feed them either, but unbinding stays allowed.
    const ring = {};
    for (const [name, handle] of Object.entries({ candles: live.candles, volume: live.volume })) {
      try {
        handle.set_ring_source(new SharedArrayBuffer(4096), {
          data_offset: 64, row_stride: 48, capacity: 8, time_offset: 0, open_offset: 8, high_offset: 16,
          low_offset: 24, close_offset: 32, sequence_offset: 40, write_cursor_offset: 0,
        });
        ring[name] = "bound";
      } catch (error) {
        ring[name] = error.code ?? String(error);
      }
      handle.set_ring_source(null);
    }

    // Presentation still works on the read-only series.
    live.volume.apply_options({ histogram_updown_rule: "previous_close" });
    live.volume.apply_options({ color: "#336699" });
    live.delta.apply_options({ visible: false });
    live.cvd.apply_options({ visible: true });
    live.candles.move_to_pane(0);

    // After every refused write, a live tip equals the same tape loaded fresh.
    for (let index = 200; index < 260; index += 1) chart.update_trade_stream_trades(live.stream, [print(index)]);
    const fresh = build("TEST:FRESH");
    chart.set_trade_stream_trades(fresh.stream, tape);
    const rows = (group) => JSON.stringify({
      candles: group.candles.data(), volume: group.volume.data(), cvd: group.cvd.data(), delta: group.delta.data(),
    });
    const tip_equals_fresh = rows(live) === rows(fresh);
    const candle_count = live.candles.data().length;
    // One series has one engine writer: a bound candle is no resampling target, and it may still
    // rebind to another stream.
    const plain = chart.add_series("candlestick");
    const refusals = {};
    try {
      chart.configure_resampled_series(live.candles, {
        source: plain, interval_seconds: 300, boundaries: [{ start_time: 0, end_time: 4_000_000_000, session_id: 1 }],
      });
      refusals.resampled_target = "configured";
    } catch (error) {
      refusals.resampled_target = error.code ?? String(error);
    }
    const other = chart.add_trade_stream("TEST:OTHER", options);
    chart.bind_trade_bar_series_to_stream(live.candles, other);
    console.warn = original_warn;
    return {
      outcomes,
      ring,
      warned_pop: warnings.some((text) => text.includes("pop rejected")),
      refusals,
      candles: candle_count,
      tip_equals_fresh,
    };
  });

  expect(errors).toEqual([]);
  for (const name of ["candles", "volume", "cvd", "delta"]) {
    const outcome = result.outcomes[name];
    for (const [attempt, seen] of Object.entries(outcome.seen)) {
      expect(seen, `${name} ${attempt}`).toEqual({ status: "rejected", code: "derived_series", warned: true });
    }
    expect(outcome.unchanged, `${name} data`).toBe(true);
    expect(outcome.notified, `${name} data_changed`).toBe(0);
  }
  expect(result.ring).toEqual({ candles: "unsupported_operation", volume: "unsupported_operation" });
  expect(result.refusals).toEqual({ resampled_target: "invalid_options" });
  expect(result.candles).toBeGreaterThan(3);
  expect(result.tip_equals_fresh).toBe(true);
});

test("reconfiguring a volume binding adds a trading date; self-feeding volume is refused", async ({ page }) => {
  const errors = await open_chart(page);
  const result = await page.evaluate(async (windows) => {
    const { resample_boundaries, session_slot_times } = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.remove_series(window.__main);
    chart.time_scale().apply_options({ time_zone: "Asia/Shanghai" });
    const dates = ["2026-09-24", "2026-09-25"];
    const slots = (date) => session_slot_times({ date, windows, interval_seconds: 60, time_zone: "Asia/Shanghai" });
    const price = (index) => 10 + ((index * 7) % 23) / 100;
    const bars = (times, from) => times.map((time, index) => ({
      time, open: price(from + index), high: price(from + index) + 0.05, low: price(from + index) - 0.03, close: price(from + index) + 0.01,
    }));
    const minute = chart.add_series("candlestick");
    const minute_volume = chart.add_series("histogram", { pane: 1 });
    const first = slots(dates[0]);
    minute.set_data(bars(first, 0));
    minute_volume.set_data(first.map((time) => ({ time, value: 100 })));
    const hour = chart.add_series("candlestick");
    const hour_volume = chart.add_series("histogram", { pane: 1 });
    const binding = (days) => ({
      source: minute, volume_source: minute_volume, volume_target: hour_volume,
      interval_seconds: 3600, boundaries: resample_boundaries({ dates: days, windows, time_zone: "Asia/Shanghai" }),
    });
    chart.configure_resampled_series(hour, binding([dates[0]]));
    const before = { bars: hour.data().length, volumes: hour_volume.data().length };
    // A new trading date starts: the same call with extended boundaries, then its minutes.
    chart.configure_resampled_series(hour, binding(dates));
    const second = slots(dates[1]);
    for (const [index, bar] of bars(second, first.length).entries()) {
      minute.update(bar);
      minute_volume.update({ time: second[index], value: 50 });
    }
    let refused = null;
    const other = chart.add_series("candlestick");
    try {
      chart.configure_resampled_series(other, {
        source: minute, volume_source: minute_volume, volume_target: minute_volume,
        interval_seconds: 120, boundaries: binding(dates).boundaries,
      });
    } catch (error) {
      refused = error.code;
    }
    return {
      before,
      after: { bars: hour.data().length, volumes: hour_volume.data().map((row) => row.value) },
      refused,
      still_live: (() => { minute.update({ ...bars(second, first.length).at(-1), close: 12 }); return hour.data().at(-1).close; })(),
    };
  }, A_SHARE);
  expect(errors).toEqual([]);
  expect(result.before).toEqual({ bars: 4, volumes: 4 });
  expect(result.after.bars).toBe(8);
  expect(result.after.volumes.slice(4)).toEqual([3000, 3000, 3000, 3000]);
  expect(result.refused).toBe("invalid_options");
  expect(result.still_live).toBe(12);
});

test("US daily bars from extended-hours minutes follow the exchange day across DST", async ({ page }) => {
  const errors = await open_chart(page);
  const result = await page.evaluate(async () => {
    const { resample_boundaries, session_slot_times } = await import("/dist/aeris_charts_financial.js");
    const chart = window.__chart;
    chart.remove_series(window.__main);
    chart.time_scale().apply_options({ time_zone: "America/New_York" });
    const windows = [["04:00", "20:00"]];
    const dates = ["2024-03-08", "2024-03-11"];
    const slots = dates.flatMap((date) => session_slot_times({
      date, windows, interval_seconds: 60, time_zone: "America/New_York",
    }));
    const minute = chart.add_series("candlestick");
    minute.set_data(slots.map((time, index) => ({
      time, open: 100 + index / 100, high: 101 + index / 100, low: 99 + index / 100, close: 100.5 + index / 100,
    })));
    const daily = chart.add_series("candlestick");
    chart.configure_resampled_series(daily, {
      source: minute,
      interval_seconds: 86400,
      boundaries: resample_boundaries({ dates, windows, time_zone: "America/New_York", span: "day" }),
    });
    return {
      minutes: slots.length,
      bars: chart.resampled_bars(daily),
      friday_last_minute: slots[959],
    };
  });

  expect(errors).toEqual([]);
  expect(result.minutes).toBe(1920);
  expect(result.bars).toHaveLength(2);
  // 04:00 Eastern opens each day: 09:00 UTC in winter, 08:00 UTC after the change.
  expect(result.bars[0].time).toBe(Date.UTC(2024, 2, 8, 9) / 1000);
  expect(result.bars[1].time).toBe(Date.UTC(2024, 2, 11, 8) / 1000);
  expect(result.bars.map((bar) => bar.source_rows)).toEqual([960, 960]);
  // Friday's session runs past UTC midnight (19:59 EST is 00:59 UTC Saturday) inside one bar.
  expect(result.friday_last_minute).toBe(Date.UTC(2024, 2, 9, 0, 59) / 1000);
  expect(result.bars[0].close).toBeCloseTo(100.5 + 959 / 100, 10);
});
