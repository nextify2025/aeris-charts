import { test, expect } from "@playwright/test";

test("host profile periods, tape levels, TPO, and profile drawings use the engine", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const base = 1_700_000_000;
    const candles = chart.add_series("candlestick");
    const volume = chart.add_series("histogram", { visible: false });
    candles.set_data([
      { time: base, open: 100, high: 101, low: 99, close: 100 },
      { time: base + 30, open: 100, high: 102, low: 100, close: 101 },
      { time: base + 60, open: 101, high: 103, low: 101, close: 102 },
    ]);
    volume.set_data([0, 30, 60].map((offset, index) => ({
      time: base + offset, value: index + 1,
    })));
    const candle_source = { kind: "candles", price_series: candles.id, volume_series: volume.id };
    const boundary = [{ startTime: base, endTime: base + 120, sessionId: 7 }];
    const candle_profiles = chart.periodic_volume_profiles(candle_source, boundary, 1, 8, 70);
    const candle_composite = chart.periodic_volume_profiles(candle_source, [
      { startTime: base, endTime: base + 30, sessionId: 8 },
      { startTime: base + 60, endTime: base + 90, sessionId: 8 },
    ], 1, 8, 70);
    const candle_naked = chart.periodic_naked_profile_levels(candle_source, boundary, 1, 8, 70);
    const tpo = chart.tpo_profiles({
      priceSeries: candles.id,
      boundaries: boundary,
      periodSeconds: 30,
      tickSize: 1,
      valueAreaPercent: 70,
      initialBalancePeriods: 2,
    });
    const vwap = chart.anchored_vwap(candle_source, base * 1_000_000, (base + 120) * 1_000_000, 1);
    const options = {
      source: candle_source,
      tickSize: 1,
      rowCount: 8,
      valueAreaPercent: 70,
      bandMultiplier: 1,
      widthPercent: 30,
    };
    const fixed = chart.add_drawing("fixed_range_volume_profile", [
      { logical: 0, price: 100 }, { logical: 2, price: 100 },
    ]);
    chart.configure_profile_drawing(fixed, options);
    const fixed_snapshot = chart.profile_drawing_snapshot(fixed);
    const anchored_profile = chart.add_drawing("anchored_volume_profile", [{ logical: 0, price: 100 }]);
    chart.configure_profile_drawing(anchored_profile, options);
    const anchored_profile_snapshot = chart.profile_drawing_snapshot(anchored_profile);
    const anchored = chart.add_drawing("anchored_vwap", [{ logical: 0, price: 100 }]);
    chart.configure_profile_drawing(anchored, options);
    const anchored_snapshot = chart.profile_drawing_snapshot(anchored);
    candles.update({ time: base + 300, open: 103, high: 103, low: 103, close: 103 });
    candles.update({ time: base + 330, open: 104, high: 104, low: 104, close: 104 });
    const merged_boundaries = [
      boundary[0],
      { startTime: base + 300, endTime: base + 420, sessionId: 7 },
    ];
    const tpo_merged = chart.tpo_profiles({
      priceSeries: candles.id,
      boundaries: merged_boundaries,
      periodSeconds: 30,
      tickSize: 1,
      valueAreaPercent: 70,
      initialBalancePeriods: 2,
    });
    const tpo_split = chart.tpo_profiles({
      priceSeries: candles.id,
      boundaries: [merged_boundaries[0], { ...merged_boundaries[1], sessionId: 8 }],
      periodSeconds: 30,
      tickSize: 1,
      valueAreaPercent: 70,
      initialBalancePeriods: 2,
    });

    const stream = chart.add_trade_stream("B7:PROFILE", { tick_size: 0.1 });
    const micros = base * 1_000_000;
    chart.set_trade_stream_trades(stream, [
      { timestamp_micros: micros + 100, price: 100.3, volume: 5, aggressor: "buy" },
      { timestamp_micros: micros + 1_000_100, price: 100.4, volume: 2, aggressor: "sell" },
      { timestamp_micros: micros + 2_000_100, price: 100.3, volume: 1, aggressor: "buy" },
    ]);
    const tape_source = { kind: "tape", stream_id: stream };
    const tape_profile = chart.volume_profile_snapshot({
      source: tape_source,
      startTimestampMicros: micros,
      endTimestampMicros: micros + 2_000_000,
      tickSize: 0.1,
      rowCount: 8,
      valueAreaPercent: 70,
    });
    const naked = chart.periodic_naked_profile_levels(
      tape_source,
      [{ startTime: base, endTime: base + 2, sessionId: 9 }],
      0.1, 8, 70,
    );
    const tape_composite = chart.periodic_volume_profiles(tape_source, [
      { startTime: base, endTime: base + 1, sessionId: 8 },
      { startTime: base + 2, endTime: base + 3, sessionId: 8 },
    ], 0.1, 8, 70);
    return { candle_profiles, candle_composite, candle_naked, tpo, tpo_merged, tpo_split, vwap, fixed_snapshot, anchored_profile_snapshot, anchored_snapshot, tape_profile, naked, tape_composite };
  });

  expect(result.candle_profiles).toHaveLength(1);
  expect(result.candle_profiles[0].sessionId).toBe(7);
  expect(result.candle_profiles[0].totalVolume).toBe(6);
  expect(result.candle_profiles[0].candleApproximation).toBe(true);
  expect(result.candle_profiles[0].developing).toHaveLength(3);
  expect(result.candle_profiles[0].developing.at(-1).poc).toBe(result.candle_profiles[0].poc);
  expect(result.candle_naked.length).toBeGreaterThan(0);
  expect(result.candle_composite).toHaveLength(1);
  expect(result.candle_composite[0].totalVolume).toBe(4);
  expect(result.tpo).toHaveLength(1);
  expect(result.tpo[0].initialBalanceLow).toBe(99);
  expect(result.tpo[0].initialBalanceHigh).toBe(102);
  expect(result.tpo_merged).toHaveLength(1);
  expect(result.tpo_merged[0].rows.at(-1).periods).toEqual([5]);
  expect(result.tpo_split).toHaveLength(2);
  expect(result.vwap).toHaveLength(3);
  expect(result.fixed_snapshot.kind).toBe("volume");
  expect(result.anchored_profile_snapshot.kind).toBe("volume");
  expect(result.anchored_snapshot.kind).toBe("vwap");
  expect(result.tape_profile.candleApproximation).toBe(false);
  expect(result.tape_profile.totalVolume).toBe(7);
  expect(result.tape_profile.rows.map((row) => row.delta)).toEqual([5, -2]);
  expect(result.tape_composite).toHaveLength(1);
  expect(result.tape_composite[0].totalVolume).toBe(6);
  expect(result.naked.find((level) => level.kind === "poc").touchedTimestampMicros)
    .toBe(1_700_000_002_000_100);
});

test("TPO blocks are painted by the shared frame and removed with their handle", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const base = 1_700_000_000;
    const source = chart.add_series("candlestick");
    source.set_data([0, 60, 120, 180].map((offset, index) => ({
      time: base + offset,
      open: 100 + index,
      high: 101 + index,
      low: 99 + index,
      close: 100 + index,
    })));
    chart.time_scale().fit_content();
    const magenta_pixels = () => {
      const canvas = chart.take_screenshot();
      const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      let count = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        if (pixels[index] === 255 && pixels[index + 1] === 0 && pixels[index + 2] === 255) count++;
      }
      return count;
    };
    const before = magenta_pixels();
    const id = chart.add_tpo_presentation({
      priceSeries: source.id,
      boundaries: [{ startTime: base, endTime: base + 240, sessionId: 1 }],
      periodSeconds: 60,
      tickSize: 1,
      valueAreaPercent: 70,
      initialBalancePeriods: 2,
    }, {
      mode: "blocks",
      color: "#ff00ff",
      valueAreaColor: "#ff00ff",
      singlePrintColor: "#ff00ff",
      pocColor: "#ff00ff",
      initialBalanceColor: "#ff00ff",
    });
    const with_tpo = magenta_pixels();
    const removed = chart.remove_tpo_presentation(id);
    const after = magenta_pixels();
    return { before, with_tpo, removed, after };
  });
  expect(result.with_tpo).toBeGreaterThan(result.before);
  expect(result.removed).toBe(true);
  expect(result.after).toBe(result.before);
});

test("periodic bid/ask and delta profiles paint and refresh from tape", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const base = 1_700_000_000;
    const anchor = chart.add_series("candlestick");
    anchor.set_data([0, 60, 120].map((offset) => ({
      time: base + offset, open: 100, high: 101, low: 99, close: 100,
    })));
    chart.time_scale().fit_content();
    const stream = chart.add_trade_stream("B7:PERIODIC", { tick_size: 1 });
    const micros = base * 1_000_000;
    chart.set_trade_stream_trades(stream, [
      { timestamp_micros: micros + 100, price: 100, volume: 5, aggressor: "buy" },
      { timestamp_micros: micros + 60_000_100, price: 101, volume: 2, aggressor: "sell" },
    ]);
    const pixel_count = () => {
      const canvas = chart.take_screenshot();
      const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      let count = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        if (pixels[i] === 255 && pixels[i + 1] === 0 && pixels[i + 2] === 255) count++;
      }
      return count;
    };
    const request = {
      source: { kind: "tape", stream_id: stream },
      boundaries: [{ startTime: base, endTime: base + 180, sessionId: 1 }],
      tickSize: 1, rowCount: 8, valueAreaPercent: 70,
    };
    let foreign_anchor_rejected = false;
    try {
      chart.add_periodic_profile_presentation({ id: anchor.id }, request);
    } catch (error) {
      foreign_anchor_rejected = error.code === "invalid_handle";
    }
    const before = pixel_count();
    const id = chart.add_periodic_profile_presentation(anchor, request, {
      mode: "bid_ask", bidColor: "#ff00ff", askColor: "#ff00ff",
      unknownColor: "#ff00ff", pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    });
    const first = pixel_count();
    chart.update_trade_stream_trades(stream, [
      { timestamp_micros: micros + 120_000_100, price: 99, volume: 20, aggressor: "buy" },
    ]);
    const corrected = pixel_count();
    const removed = chart.remove_periodic_profile_presentation(id);
    const after = pixel_count();
    const delta_id = chart.add_periodic_profile_presentation(anchor, request, {
      mode: "delta", bidColor: "#ff00ff", askColor: "#ff00ff",
      pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    });
    const delta = pixel_count();
    chart.remove_periodic_profile_presentation(delta_id);
    const short_request = {
      ...request,
      boundaries: [{ startTime: base, endTime: base + 60, sessionId: 1 }],
    };
    const short_options = {
      mode: "delta", bidColor: "#ff00ff", askColor: "#ff00ff",
      pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    };
    const short_id = chart.add_periodic_profile_presentation(anchor, short_request, short_options);
    const short = pixel_count();
    chart.remove_periodic_profile_presentation(short_id);
    const extended_id = chart.add_periodic_profile_presentation(anchor, short_request, {
      mode: "delta", extendNakedLevels: true,
      bidColor: "#ff00ff", askColor: "#ff00ff",
      pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    });
    const extended = pixel_count();
    chart.remove_periodic_profile_presentation(extended_id);
    const baseline_id = chart.add_periodic_profile_presentation(anchor, request, {
      mode: "total", bidColor: "#ff00ff", askColor: "#ff00ff",
      pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    });
    const baseline_image = chart.take_screenshot().toDataURL();
    chart.remove_periodic_profile_presentation(baseline_id);
    const developing_id = chart.add_periodic_profile_presentation(anchor, request, {
      mode: "total", showDeveloping: true,
      bidColor: "#ff00ff", askColor: "#ff00ff",
      pocColor: "#ff00ff", valueAreaColor: "#ff00ff",
    });
    const developing_changed = chart.take_screenshot().toDataURL() !== baseline_image;
    chart.remove_periodic_profile_presentation(developing_id);
    return { before, first, corrected, removed, after, delta, short, extended, developing_changed, foreign_anchor_rejected };
  });
  expect(result.first).toBeGreaterThan(result.before);
  expect(result.corrected).toBeGreaterThan(result.first);
  expect(result.removed).toBe(true);
  expect(result.after).toBe(result.before);
  expect(result.delta).toBeGreaterThan(result.before);
  expect(result.extended).toBeGreaterThan(result.short);
  expect(result.developing_changed).toBe(true);
  expect(result.foreign_anchor_rejected).toBe(true);
});
