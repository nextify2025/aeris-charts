import { test, expect } from "@playwright/test";

async function open_chart(page, backend = "canvas2d") {
  await page.goto(`/?runtimeTest=presentedFrame&backend=${backend}&forceFallbackAdapter=1`);
  await page.waitForFunction((expected) => window.__chart?.backend?.() === expected, backend);
}

test("tick-driven footprint API preserves delta path, POC, and stacked imbalances", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const footprint = chart.add_series("footprint", {
      tick_size: 1,
      interval_seconds: 60,
      imbalance_ratio: 3,
      imbalance_minimum_volume: 20,
      stacked_imbalance_levels: 2,
      font_size: 9,
      cell_mode: "bid_ask",
      price_line_visible: false,
      last_value_visible: false,
    });
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    footprint.set_trades([
      { timestamp_micros: micros + 1, price: 100, volume: 10, aggressor: "sell", session_id: 1 },
      { timestamp_micros: micros + 2, price: 101, volume: 40, aggressor: "buy", session_id: 1 },
      { timestamp_micros: micros + 3, price: 101, volume: 10, aggressor: "sell", session_id: 1 },
      { timestamp_micros: micros + 4, price: 102, volume: 50, aggressor: "buy", session_id: 1 },
      // Mean reversion proves extrema come from the tick path, not final delta.
      { timestamp_micros: micros + 5, price: 102, volume: 80, aggressor: "sell", session_id: 1 },
    ]);
    chart.time_scale().fit_content();
    chart.time_scale().apply_options({ bar_spacing: 100 });
    const first = footprint.footprint_bar(0);
    const generic_set_error = (() => {
      try {
        footprint.set_data([{ time: second, open: 1, high: 2, low: 0, close: 1 }]);
        return null;
      } catch (error) {
        return { code: error.code, message: error.message };
      }
    })();
    const tip = footprint.update_trade({
      timestamp_micros: micros + 6,
      price: 101,
      volume: 5,
      aggressor: "buy",
      session_id: 1,
    });
    const historical = footprint.update_trade({
      timestamp_micros: micros + 3,
      price: 101,
      volume: 2,
      aggressor: "buy",
      sequence: 1,
      session_id: 1,
    });
    const final = footprint.footprint_bar(0);
    return {
      type: footprint.series_type(),
      options: footprint.options(),
      first,
      final,
      bars: footprint.footprint_bars().length,
      tip,
      historical,
      generic_set_error,
    };
  });

  expect(result.type).toBe("footprint");
  expect(result.options.ask_color).toMatch(/^rgba\(8,153,129,/);
  expect(result.options.positive_delta_color).toMatch(/^rgba\(8,153,129,/);
  expect(result.options.stacked_ask_color).toBe("#089981");
  expect(result.bars).toBe(1);
  expect(result.first).toMatchObject({
    logical_index: 0,
    bid_volume: 100,
    ask_volume: 90,
    total_volume: 190,
    delta: -10,
    max_delta: 70,
    min_delta: -10,
    poc_price: 102,
  });
  expect(result.first.levels.filter((level) => level.stacked_ask_imbalance).map((level) => level.price))
    .toEqual([101, 102]);
  expect(result.tip).toBe("tip");
  expect(result.historical).toBe("historical");
  expect(result.final.delta).toBe(-3);
  expect(result.final.max_delta).toBe(72);
  expect(result.final.min_delta).toBe(-10);
  expect(result.generic_set_error).toMatchObject({ code: "unsupported_operation" });

  const screenshot = await page.screenshot();
  expect(screenshot.byteLength).toBeGreaterThan(10_000);
});

test("footprint shared-frame semantics execute through WebGPU", async ({ page }) => {
  await open_chart(page, "webgpu");
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const footprint = chart.add_series("footprint", {
      tick_size: 0.25,
      interval_seconds: 60,
      cell_mode: "delta",
    });
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    footprint.set_trades([
      { timestamp_micros: micros + 1, price: 100, volume: 9, aggressor: "buy" },
      { timestamp_micros: micros + 2, price: 100.25, volume: 4, aggressor: "sell" },
    ]);
    chart.time_scale().fit_content();
    chart.time_scale().apply_options({ bar_spacing: 100 });
    return { backend: chart.backend(), bar: footprint.footprint_bar(0) };
  });
  expect(result.backend).toBe("webgpu");
  expect(result.bar).toMatchObject({ logical_index: 0, total_volume: 13, delta: 5, max_delta: 9, min_delta: 0 });
  const screenshot = await page.screenshot();
  expect(screenshot.byteLength).toBeGreaterThan(10_000);
});

test("typed footprint columns reject off-grid data without replacing accepted bars", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const footprint = chart.add_series("footprint", { tick_size: 0.25, interval_seconds: 60 });
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const columns = (price) => ({
      timestamps_micros: new Float64Array([second * 1_000_000 + 1]),
      prices: new Float64Array([price]),
      volumes: new Float64Array([4]),
      aggressors: new Uint8Array([1]),
      bids: new Float64Array([NaN]),
      asks: new Float64Array([NaN]),
      sequences: new Float64Array([NaN]),
      trade_ids: new Float64Array([NaN]),
      conditions: new Uint32Array([0]),
      session_ids: new Float64Array([1]),
    });
    footprint.set_trades_typed(columns(100.25));
    const before = footprint.footprint_bars();
    let error = null;
    try {
      footprint.set_trades_typed(columns(100.30));
    } catch (caught) {
      error = { code: caught.code, message: caught.message };
    }
    return { before, after: footprint.footprint_bars(), error };
  });
  expect(result.error).toMatchObject({ code: "invalid_data" });
  expect(result.error.message).toContain("tick_size");
  expect(result.after).toEqual(result.before);
});

test("footprint validation rejects malformed aggressors and infinities atomically", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const footprint = chart.add_series("footprint", { tick_size: 0.25, interval_seconds: 60 });
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const columns = (aggressor = 1, bid = Number.NaN) => ({
      timestamps_micros: new Float64Array([second * 1_000_000 + 1]),
      prices: new Float64Array([100.25]),
      volumes: new Float64Array([4]),
      aggressors: new Uint8Array([aggressor]),
      bids: new Float64Array([bid]),
      asks: new Float64Array([Number.NaN]),
      sequences: new Float64Array([Number.NaN]),
      trade_ids: new Float64Array([1]),
      conditions: new Uint32Array([0]),
      session_ids: new Float64Array([1]),
    });
    footprint.set_trades_typed(columns());
    const before = footprint.footprint_bars();
    const errors = [];
    for (const invalid of [columns(9), columns(1, Number.POSITIVE_INFINITY)]) {
      try {
        footprint.set_trades_typed(invalid);
      } catch (error) {
        errors.push({ code: error.code, message: error.message });
      }
    }
    try {
      footprint.update_trade({
        timestamp_micros: second * 1_000_000 + 2,
        price: 100.25,
        volume: 1,
        aggressor: "crossed",
      });
    } catch (error) {
      errors.push({ code: error.code, message: error.message });
    }
    return { before, after: footprint.footprint_bars(), errors };
  });
  expect(result.errors).toHaveLength(3);
  expect(result.errors.every((error) => error.code === "invalid_data")).toBe(true);
  expect(result.errors[0].message).toContain("index 0");
  expect(result.errors[0].message).toContain("aggressor");
  expect(result.errors[1].message).toContain("infinity");
  expect(result.errors[2].message).toContain("aggressor");
  expect(result.after).toEqual(result.before);
});

test("historical correction batches and session replacements use final canonical truth", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const footprint = chart.add_series("footprint", { tick_size: 1, interval_seconds: 60 });
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    footprint.set_trades([
      { timestamp_micros: micros + 1, price: 100, volume: 4, aggressor: "buy", trade_id: 1, session_id: 1 },
      { timestamp_micros: micros + 2, price: 101, volume: 2, aggressor: "sell", trade_id: 2, session_id: 1 },
    ]);
    const batch = footprint.update_trades([
      { timestamp_micros: micros + 1, price: 100, volume: 6, aggressor: "sell", trade_id: 1, session_id: 2 },
      { timestamp_micros: micros + 2, price: 101, volume: 3, aggressor: "buy", trade_id: 2, session_id: 2 },
    ]);
    return { batch, bars: footprint.footprint_bars() };
  });
  expect(result.batch).toBe("historical");
  expect(result.bars).toHaveLength(1);
  expect(result.bars[0]).toMatchObject({ session_id: 2, bid_volume: 6, ask_volume: 3, delta: -3 });
});

test("non-time footprint bars use the sequence axis and round-trip construction options", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    const trades = [
      { timestamp_micros: micros + 1, price: 100, volume: 3, aggressor: "buy", session_id: 1 },
      { timestamp_micros: micros + 2, price: 101, volume: 2, aggressor: "sell", session_id: 1 },
      { timestamp_micros: micros + 3, price: 102, volume: 4, aggressor: "buy", session_id: 1 },
      { timestamp_micros: micros + 4, price: 103, volume: 1, aggressor: "sell", session_id: 1 },
    ];
    const read = (options) => {
      const series = chart.add_series("footprint", options);
      series.set_trades(trades);
      const snapshot = {
        bars: series.footprint_bars(),
        options: series.options(),
        data_times: series.data().map((point) => point.time),
        snapshot_time: chart.value_snapshot(1).find((entry) => entry.series_id === series.id)?.time,
      };
      chart.remove_series(series);
      return snapshot;
    };
    return {
      trade: read({ tick_size: 1, bar_type: "trades", trades_per_bar: 2 }),
      volume: read({ tick_size: 1, bar_type: "volume", volume_per_bar: 5 }),
      range: read({ tick_size: 1, bar_type: "range", range_ticks: 2 }),
      first_open: micros + 1,
    };
  });

  expect(result.trade.bars).toHaveLength(2);
  expect(result.volume.bars).toHaveLength(2);
  expect(result.range.bars).toHaveLength(2);
  expect(result.trade.bars.map((bar) => bar.logical_index)).toEqual([0, 1]);
  expect(result.volume.bars.map((bar) => bar.logical_index)).toEqual([0, 1]);
  expect(result.range.bars.map((bar) => bar.logical_index)).toEqual([0, 1]);
  expect(result.trade.options).toMatchObject({ bar_type: "trades", trades_per_bar: 2 });
  expect(result.volume.options).toMatchObject({ bar_type: "volume", volume_per_bar: 5 });
  expect(result.range.options).toMatchObject({ bar_type: "range", range_ticks: 2 });
  expect(result.trade.data_times[0]).toBe(Math.floor(result.first_open / 1_000_000));
  expect(result.trade.snapshot_time).toBe(Math.floor(result.first_open / 1_000_000));
  expect(result.trade.bars[0].start_timestamp_micros).toBe(result.first_open);
});

test("ordinary candles consume one canonical non-time trade stream", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const candles = chart.add_series("candlestick");
    const scalar = chart.add_series("line");
    const stream = chart.add_trade_stream("CME:ES:browser", {
      tick_size: 1,
      bar_type: "trades",
      trades_per_bar: 2,
    });
    let scalar_error = "";
    try {
      chart.bind_trade_bar_series_to_stream(scalar, stream);
    } catch (error) {
      scalar_error = String(error);
    }
    chart.bind_trade_bar_series_to_stream(candles, stream);
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    chart.set_trade_stream_trades(stream, [
      { timestamp_micros: micros + 1, price: 100, volume: 1, aggressor: "buy" },
      { timestamp_micros: micros + 2, price: 102, volume: 1, aggressor: "sell" },
      { timestamp_micros: micros + 3, price: 101, volume: 1, aggressor: "buy" },
    ]);
    const before = candles.data();
    const update = chart.update_trade_stream_trades(stream, [
      { timestamp_micros: micros + 4, price: 103, volume: 1, aggressor: "buy" },
    ]);
    return {
      before,
      after: candles.data(),
      update,
      stats: chart.trade_stream_stats(stream),
      snapshots: [chart.value_snapshot(0), chart.value_snapshot(1)],
      scalar_error,
    };
  });

  expect(result.before.map(({ open, high, low, close }) => ({ open, high, low, close }))).toEqual([
    { open: 100, high: 102, low: 100, close: 102 },
    { open: 101, high: 101, low: 101, close: 101 },
  ]);
  expect(result.after[1]).toMatchObject({ open: 101, high: 103, low: 101, close: 103 });
  expect(result.update).toBe("tip");
  expect(result.stats).toMatchObject({ dependent_count: 1, dependent_incremental_updates: 1 });
  expect(result.snapshots).toHaveLength(2);
  expect(result.scalar_error).toContain("candlestick or bar");
});

test("live trade-stream tips do suffix-bounded dependent work and match a clean load", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    const trade = (index) => ({
      timestamp_micros: micros + index * 7_000_000 + 1,
      price: 100 + ((index * 7) % 11),
      volume: 1 + ((index * 13) % 9) + (index % 29 === 0 ? 30 : 0),
      aggressor: index % 5 === 2 ? "unknown" : index % 2 ? "sell" : "buy",
      trade_id: index + 1,
      session_id: 1,
    });
    // Two streams on one time axis: one advances by live tips, the other loads the final tape.
    const build = (key) => {
      const stream = chart.add_trade_stream(key, { tick_size: 1, interval_seconds: 60 });
      const footprint = chart.add_series("footprint", { tick_size: 1, interval_seconds: 60 });
      chart.bind_footprint_series_to_stream(footprint, stream);
      const candles = chart.add_series("candlestick");
      chart.bind_trade_bar_series_to_stream(candles, stream);
      const cvd = chart.add_cvd_series(stream, 1, "continuous");
      const delta = chart.add_delta_series(stream, 1);
      const big_trades = chart.add_big_trades(footprint, stream, { filter: { mode: "fixed", minimum_volume: 3 } });
      return { stream, footprint, candles, cvd, delta, big_trades };
    };
    const live = build("TEST:TIP:LIVE");
    const clean = build("TEST:TIP:CLEAN");
    chart.set_trade_stream_trades(live.stream, Array.from({ length: 400 }, (_, index) => trade(index)));
    const tips = [];
    for (let index = 400; index < 520; index += 1) {
      const before = chart.trade_stream_stats(live.stream);
      const bars_before = live.footprint.footprint_bars().length;
      const kind = chart.update_trade_stream_trades(live.stream, [trade(index)]);
      const after = chart.trade_stream_stats(live.stream);
      tips.push({
        kind,
        changed_bars: live.footprint.footprint_bars().length + 1 - bars_before,
        study_rows: after.dependent_rows_computed - before.dependent_rows_computed,
        bar_rows: after.bar_rows_projected - before.bar_rows_projected,
        big_trades_prints: after.big_trades_prints_scanned - before.big_trades_prints_scanned,
        big_trades_replays: after.big_trades_replays - before.big_trades_replays,
      });
    }
    chart.set_trade_stream_trades(clean.stream, Array.from({ length: 520 }, (_, index) => trade(index)));
    const read = (handles) => ({
      bars: handles.footprint.footprint_bars(),
      candles: handles.candles.data(),
      cvd: handles.cvd.data(),
      delta: handles.delta.data(),
      big_trades: handles.big_trades.snapshot(),
    });
    return { tips, live: read(live), clean: read(clean) };
  });

  expect(result.tips).toHaveLength(120);
  for (const tip of result.tips) {
    expect(tip.kind).toBe("tip");
    // CVD and delta recompute, and the footprint and candles project, only the changed bars.
    expect(tip.study_rows).toBe(2 * tip.changed_bars);
    expect(tip.bar_rows).toBe(2 * tip.changed_bars);
    // Big trades fold only the new trade and never replay the tape.
    expect(tip.big_trades_prints).toBe(1);
    expect(tip.big_trades_replays).toBe(0);
  }
  expect(result.tips.some((tip) => tip.changed_bars === 2)).toBe(true);
  expect(result.live.bars.length).toBeGreaterThan(20);
  expect(result.live.big_trades.bubbles.length).toBeGreaterThan(0);
  expect(result.live).toEqual(result.clean);
});

test("retained live trade-stream tips evict history in place and keep the unretained tail", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const second = Math.floor(window.__data[0].time / 60) * 60;
    const micros = second * 1_000_000;
    const trade = (index) => ({
      timestamp_micros: micros + index * 7_000_000 + 1,
      price: 100 + ((index * 7) % 11),
      volume: 1 + ((index * 13) % 9) + (index % 29 === 0 ? 30 : 0),
      aggressor: index % 5 === 2 ? "unknown" : index % 2 ? "sell" : "buy",
      trade_id: index + 1,
      session_id: 1,
    });
    // The same tape streams into a retained and an unretained stream on one time axis.
    const build = (key, max_points) => {
      const stream = chart.add_trade_stream(key, { tick_size: 1, interval_seconds: 60 });
      const footprint = chart.add_series("footprint", { tick_size: 1, interval_seconds: 60 });
      chart.bind_footprint_series_to_stream(footprint, stream);
      if (max_points) footprint.apply_options({ max_points });
      const candles = chart.add_series("candlestick");
      chart.bind_trade_bar_series_to_stream(candles, stream);
      const cvd = chart.add_cvd_series(stream, 1, "continuous");
      const delta = chart.add_delta_series(stream, 1);
      const big_trades = chart.add_big_trades(footprint, stream, { filter: { mode: "fixed", minimum_volume: 3 } });
      return { stream, footprint, candles, cvd, delta, big_trades };
    };
    const retained = build("TEST:TIP:RETAINED", 20);
    const unretained = build("TEST:TIP:UNRETAINED", null);
    const history = Array.from({ length: 200 }, (_, index) => trade(index));
    chart.set_trade_stream_trades(retained.stream, history);
    chart.set_trade_stream_trades(unretained.stream, history);
    const first_open = () => retained.footprint.footprint_bars()[0].start_timestamp_micros;
    const tips = [];
    for (let index = 200; index < 500; index += 1) {
      const before = chart.trade_stream_stats(retained.stream);
      const opened_before = first_open();
      chart.update_trade_stream_trades(retained.stream, [trade(index)]);
      chart.update_trade_stream_trades(unretained.stream, [trade(index)]);
      const after = chart.trade_stream_stats(retained.stream);
      tips.push({
        trimmed: first_open() !== opened_before,
        big_trades_prints: after.big_trades_prints_scanned - before.big_trades_prints_scanned,
        big_trades_replays: after.big_trades_replays - before.big_trades_replays,
        bar_rows: after.bar_rows_projected - before.bar_rows_projected,
      });
    }
    const read = (handles) => ({
      bars: handles.footprint.footprint_bars().map(({ logical_index, ...bar }) => bar),
      candles: handles.candles.data(),
      cvd: handles.cvd.data(),
      delta: handles.delta.data(),
      big_trades: handles.big_trades.snapshot(),
    });
    return { tips, retained: read(retained), unretained: read(unretained) };
  });

  expect(result.tips.filter((tip) => tip.trimmed).length).toBeGreaterThan(2);
  for (const tip of result.tips) {
    // Every tip folds only its own trade and replays nothing, also the tips that cross the
    // retention ceiling: big trades evict the orders of evicted bars in place.
    expect(tip.big_trades_prints).toBe(1);
    expect(tip.big_trades_replays).toBe(0);
    // The footprint and candles project only the active bar and a bar the tip opened.
    expect(tip.bar_rows).toBeLessThanOrEqual(4);
  }
  const retained_bars = result.retained.bars.length;
  expect(retained_bars).toBeLessThanOrEqual(20);
  expect(result.unretained.bars.length).toBeGreaterThan(retained_bars);
  const tail = (rows) => rows.slice(rows.length - retained_bars);
  expect(result.retained.bars).toEqual(tail(result.unretained.bars));
  expect(result.retained.candles).toEqual(tail(result.unretained.candles));
  // Evicting history never rewrites the cumulative delta of the bars that remain.
  expect(result.retained.cvd).toEqual(tail(result.unretained.cvd));
  expect(result.retained.delta).toEqual(tail(result.unretained.delta));
  // Retention drops the orders of evicted bars and never changes another one.
  const first_bar = Math.floor(result.retained.bars[0].start_timestamp_micros / 1_000_000);
  expect(result.retained.big_trades.bubbles.length).toBeGreaterThan(0);
  expect(result.retained.big_trades).toEqual({
    ...result.unretained.big_trades,
    bubbles: result.unretained.big_trades.bubbles.filter((order) => order.bar_time >= first_bar),
  });
});

test("big trades rebuild split prints into one order over ordinary candles", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const stream = chart.add_trade_stream("CME:ES:big-trades", { tick_size: 0.25 });
    const big_trades = chart.add_big_trades(window.__main, stream, {
      filter: { mode: "fixed", minimum_volume: 10 },
    });
    const micros = window.__data[0].time * 1_000_000;
    chart.set_trade_stream_trades(stream, [
      // One sell order sweeping two levels, reported as two 6-lot prints.
      { timestamp_micros: micros + 1, price: 100, volume: 6, aggressor: "sell", session_id: 1 },
      { timestamp_micros: micros + 1, price: 99.75, volume: 6, aggressor: "sell", session_id: 1 },
      { timestamp_micros: micros + 2, price: 100, volume: 3, aggressor: "buy", session_id: 1 },
    ]);
    const snapshot = big_trades.snapshot();
    const defaults = big_trades.options();
    big_trades.apply_options({ size: "large", show_volume: false });
    const restyled = big_trades.options();
    const invalid = (() => {
      try {
        big_trades.apply_options({ grouping_window_micros: -1 });
        return null;
      } catch (error) {
        return error.code;
      }
    })();
    const in_use = chart.trade_stream_stats(stream).dependent_count;
    big_trades.remove();
    const stale = (() => {
      try {
        big_trades.snapshot();
        return null;
      } catch (error) {
        return error.code;
      }
    })();
    return {
      snapshot,
      defaults,
      restyled,
      invalid,
      in_use,
      released: chart.trade_stream_stats(stream).dependent_count,
      stale,
    };
  });

  expect(result.snapshot.threshold).toBe(10);
  expect(result.snapshot.bubbles).toHaveLength(1);
  expect(result.snapshot.bubbles[0]).toMatchObject({
    side: "sell",
    volume: 12,
    prints: 2,
    low: 99.75,
    high: 100,
    vwap: 99.875,
  });
  expect(result.defaults).toMatchObject({
    grouping_window_micros: 1000,
    size: "medium",
    show_volume: true,
    visible: true,
    text_color: null,
  });
  expect(result.restyled).toMatchObject({ size: "large", show_volume: false });
  expect(result.invalid).toBe("invalid_options");
  expect(result.in_use).toBe(1);
  expect(result.released).toBe(0);
  expect(result.stale).toBe("stale_handle");
});

test("big trades rejections carry typed error codes", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const code = (action) => {
      try {
        action();
        return null;
      } catch (error) {
        return error.code;
      }
    };
    const stream = chart.add_trade_stream("CME:ES:big-trades-errors", { tick_size: 0.25 });
    const histogram = chart.add_series("histogram");
    const unknown_stream = code(() => chart.add_big_trades(window.__main, 999_999));
    const unknown_series = code(() => chart.add_big_trades(999_999, stream));
    const unsupported = code(() => chart.add_big_trades(histogram, stream));
    const invalid = code(() => chart.add_big_trades(window.__main, stream, { grouping_window_micros: -1 }));
    const added = [];
    let limit = null;
    while (limit === null && added.length < 32) {
      limit = code(() => added.push(chart.add_big_trades(window.__main, stream)));
    }
    const indicators = added.length;
    for (const handle of added) handle.remove();
    return { unknown_stream, unknown_series, unsupported, invalid, limit, indicators };
  });

  expect(result).toEqual({
    unknown_stream: "invalid_handle",
    unknown_series: "invalid_handle",
    unsupported: "unsupported_operation",
    invalid: "invalid_options",
    limit: "resource_limit",
    indicators: 16,
  });
});

test("trade replay clock masks retained future events and reports seek work", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const candles = chart.add_series("candlestick");
    const reference = chart.add_series("line");
    reference.apply_options({ line_type: "stepped", title: "Weekly fundamentals" });
    const footprint = chart.add_series("footprint", {
      tick_size: 1,
      bar_type: "time",
      interval_seconds: 1,
    });
    const stream = chart.add_trade_stream("CME:ES:replay-browser", {
      tick_size: 1,
      bar_type: "time",
      interval_seconds: 1,
    });
    chart.bind_footprint_series_to_stream(footprint, stream);
    chart.bind_trade_bar_series_to_stream(candles, stream);
    const micros = Math.floor(window.__data[0].time) * 1_000_000;
    const second = Math.floor(micros / 1_000_000);
    reference.set_data([
      { time: second, value: 10 },
      { time: second + 1, value: 20 },
      { time: second + 2, value: 30 },
    ]);
    chart.set_trade_stream_trades(stream, [1, 1_000_001, 2_000_001].map((offset, index) => ({
      timestamp_micros: micros + offset,
      price: 100 + index,
      volume: 1,
      aggressor: "buy",
    })));
    const backward = chart.set_replay_clock_micros(micros + 1);
    const masked = { candles: candles.data(), bars: footprint.footprint_bars(), reference: reference.data() };
    chart.update_trade_stream_trades(stream, [{
      timestamp_micros: micros + 3_000_001,
      price: 103,
      volume: 1,
      aggressor: "buy",
    }]);
    const still_masked = candles.data().length;
    const forward = chart.set_replay_clock_micros(micros + 3_000_001);
    return {
      backward,
      masked,
      still_masked,
      forward,
      visible: candles.data(),
      reference: reference.data(),
      clock: chart.replay_clock_micros(),
    };
  });

  expect(result.backward).toMatchObject({ stream_count: 1, visible_trades: 1, rebuilt_trades: 1 });
  expect(result.masked.candles).toHaveLength(1);
  expect(result.masked.bars).toHaveLength(1);
  expect(result.masked.reference).toHaveLength(1);
  expect(result.still_masked).toBe(1);
  expect(result.forward).toMatchObject({ visible_trades: 4, rebuilt_trades: 0, incremental_trades: 3 });
  expect(result.visible).toHaveLength(4);
  expect(result.reference).toHaveLength(3);
  expect(result.clock).toBe(result.forward.clock_micros);
});

test("synthetic price-action charts share the logical sequence and replay clock", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const series = chart.add_series("candlestick");
    const start = Math.floor(window.__data[0].time);
    const rows = (values, spread = 0) => values.map((close, index) => ({
      time: start + index,
      open: close,
      high: close + spread,
      low: close - spread,
      close,
    }));
    const read = (options, data) => {
      chart.configure_synthetic_bar_series(series, options);
      chart.set_synthetic_bar_source(series, data);
      return series.data().map(({ open, high, low, close }) => ({ open, high, low, close }));
    };
    const renko = read({ kind: "renko_fixed", box_size: 1 }, rows([100, 103.2, 102.1, 100.8]));
    chart.update_synthetic_bar_source(series, {
      time: start + 4,
      open: 105,
      high: 105,
      low: 105,
      close: 105,
    });
    const renko_updated = series.data();
    const atr = read({ kind: "renko_atr", period: 3 }, rows([100, 101, 102, 105], 0.5));
    const line_break = read({ kind: "line_break", lines: 3 }, rows([100, 101, 102, 101.5, 99]));
    const kagi = read({ kind: "kagi", reversal_size: 2 }, rows([100, 103, 104, 103, 101.5, 99]));
    const point_and_figure = read(
      { kind: "point_and_figure", box_size: 1, reversal_boxes: 3 },
      rows([100, 104.2, 102, 100.5, 103.8]),
    );
    chart.configure_synthetic_bar_series(series, { kind: "renko_fixed", box_size: 1 });
    chart.set_synthetic_bar_source(series, rows([100, 103, 99]));
    chart.set_replay_clock_micros((start + 1) * 1_000_000);
    const replay_masked = series.data().length;
    chart.set_replay_clock_micros(null);
    let second_error = "";
    try {
      const second = chart.add_series("candlestick");
      chart.configure_synthetic_bar_series(second, { kind: "line_break", lines: 3 });
    } catch (error) {
      second_error = String(error);
    }
    return {
      renko,
      renko_updated_length: renko_updated.length,
      renko_updated_close: renko_updated.at(-1)?.close,
      atr,
      line_break,
      kagi,
      point_and_figure,
      replay_masked,
      second_error,
    };
  });

  expect(result.renko.map(({ open, close }) => ({ open, close }))).toEqual([
    { open: 100, close: 101 },
    { open: 101, close: 102 },
    { open: 102, close: 103 },
    { open: 102, close: 101 },
  ]);
  expect(result.renko_updated_length).toBe(7);
  expect(result.renko_updated_close).toBe(105);
  expect(result.atr).toHaveLength(1);
  expect(result.line_break.map(({ close }) => close)).toEqual([100, 101, 102, 99]);
  expect(result.kagi.map(({ open, close }) => ({ open, close }))).toEqual([
    { open: 100, close: 104 },
    { open: 104, close: 99 },
  ]);
  expect(result.point_and_figure.map(({ open, close }) => ({ open, close }))).toEqual([
    { open: 100, close: 104 },
    { open: 103, close: 101 },
  ]);
  expect(result.replay_masked).toBe(3);
  expect(result.second_error).toContain("one independent non-time bar sequence");
});

test("failed footprint creation leaves engine order, handles, scale membership, and notifications unchanged", async ({ page }) => {
  await open_chart(page);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const before = {
      handles: chart.series_order().map((series) => series.id),
      snapshots: chart.value_snapshot().map((entry) => entry.series_id),
    };
    const added = [];
    chart.subscribe_series_added((event) => added.push(event.series.id));
    let failure = null;
    try {
      chart.add_series("footprint", { price_scale_id: "footprint-dedicated", tick_size: 0.25 });
    } catch (error) {
      failure = { code: error.code, message: error.message };
    }
    const rejected = {
      handles: chart.series_order().map((series) => series.id),
      snapshots: chart.value_snapshot().map((entry) => entry.series_id),
      added: [...added],
      scales: chart.price_scales().map((scale) => ({ id: scale.id, series_ids: scale.series_ids })),
    };
    chart.add_price_scale({ id: "footprint-dedicated", side: "right" });
    const footprint = chart.add_series("footprint", {
      price_scale_id: "footprint-dedicated",
      tick_size: 0.25,
    });
    return {
      before,
      failure,
      rejected,
      accepted: {
        id: footprint.id,
        scale_id: footprint.price_scale_id(),
        added,
        members: chart.price_scales().find((scale) => scale.id === "footprint-dedicated")?.series_ids,
      },
    };
  });
  expect(result.failure).toMatchObject({ code: "invalid_options" });
  expect(result.rejected.handles).toEqual(result.before.handles);
  expect(result.rejected.snapshots).toEqual(result.before.snapshots);
  expect(result.rejected.added).toEqual([]);
  expect(result.rejected.scales.some((scale) => scale.id === "footprint-dedicated")).toBe(false);
  expect(result.accepted.scale_id).toBe("footprint-dedicated");
  expect(result.accepted.added).toEqual([result.accepted.id]);
  expect(result.accepted.members).toEqual([result.accepted.id]);
});
