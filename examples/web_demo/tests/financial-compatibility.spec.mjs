import { test, expect } from "@playwright/test";

async function open_chart(page) {
  await page.goto("/?runtimeTest=presentedFrame&backend=canvas2d&forceFallbackAdapter=1");
  await page.waitForFunction(() => window.__chart?.backend?.() === "canvas2d");
}

test("CVD and delta start at the same small height as oscillator panes", async ({ page }) => {
  await open_chart(page);
  const factors = await page.evaluate(() => {
    const chart = window.__chart;
    const stream = chart.add_trade_stream("pane-sizing");
    chart.add_cvd_series(stream, 1);
    chart.add_delta_series(stream, 2);
    chart.add_rsi(window.__main, 5);
    return chart.panes().map((pane) => pane.get_stretch_factor());
  });
  expect(factors.slice(1)).toEqual([0.3, 0.3, 0.3]);
  expect(factors[0]).toBeGreaterThan(0.3);
});

test("financial product composes through one public chart and ordered frame", async ({ page }) => {
  await open_chart(page);

  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const source = window.__main;
    const bars = window.__data.slice(0, 64);
    const scalar = bars.map((bar) => ({ time: bar.time, value: bar.close }));

    const canonical = [
      ["bar", bars],
      ["line", scalar],
      ["area", scalar],
      ["histogram", scalar],
      ["baseline", scalar],
    ].map(([kind, data]) => {
      const series = chart.add_series(kind, {
        price_line_visible: false,
        last_value_visible: false,
      });
      series.set_data(data);
      return series;
    });

    const feature_definitions = [
      ["grouped_bars", bars.map((bar, index) => ({ time: bar.time, values: [index + 1, index + 3, index + 2] }))],
      ["heatmap", bars.map((bar, index) => ({
        time: bar.time,
        cells: [
          { low: bar.low, high: bar.close, amount: index },
          { low: bar.close, high: bar.high, amount: 100 - index },
        ],
      }))],
      ["hlc_area", bars.map((bar) => ({ time: bar.time, high: bar.high, low: bar.low, close: bar.close }))],
      ["pretty_histogram", scalar],
      ["background_shade", scalar],
      ["stacked_area", bars.map((bar, index) => ({ time: bar.time, values: [10 + index, 5, 3] }))],
      ["stacked_bars", bars.map((bar, index) => ({ time: bar.time, values: [10 + index, 5, 3] }))],
      ["whisker_box", bars.map((bar) => ({
        time: bar.time,
        quartiles: [bar.low - 1, bar.low, bar.close, bar.high, bar.high + 1],
        outliers: [bar.high + 2],
      }))],
    ];
    const features = feature_definitions.map(([kind, data]) => {
      const series = chart.add_series(kind, {
        price_line_visible: false,
        last_value_visible: false,
      });
      series.set_data(data);
      return series;
    });

    const footprint = chart.add_series("footprint", {
      tick_size: 0.25,
      interval_seconds: 60,
      price_line_visible: false,
      last_value_visible: false,
    });
    const footprint_second = Math.floor(bars[0].time / 60) * 60;
    footprint.set_trades([
      { timestamp_micros: footprint_second * 1_000_000 + 1, price: 100, volume: 4, aggressor: "buy" },
      { timestamp_micros: footprint_second * 1_000_000 + 2, price: 100.25, volume: 3, aggressor: "sell" },
    ]);

    const comparison_pane = chart.add_pane(true);
    const comparison_scale = chart.add_price_scale(
      { id: "phase-zero-comparison", side: "left", order: 0 },
      comparison_pane.pane_index(),
    );
    const comparison = canonical.find((series) => series.series_type() === "line");
    comparison.move_to_pane(comparison_pane.pane_index(), 0.6);
    comparison.move_to_price_scale("phase-zero-comparison");

    const indicator_groups = [
      [chart.add_sma(source, 10)],
      [chart.add_ema(source, 12)],
      chart.add_ema_ribbon(source, [3, 5, 8, 13, 21]),
      chart.add_bollinger(source, 10, 2),
      [chart.add_rsi(source, 5)],
      chart.add_macd(source, 3, 6, 3),
      chart.add_stochastic(source, 5, 3),
      [chart.add_atr(source, 5)],
      [chart.add_vwap(source)],
      [chart.add_wma(source, 4)],
    ];
    const indicators = indicator_groups.flat();

    chart.time_scale().fit_content();
    const low = Math.min(...bars.slice(8, 25).map((bar) => bar.low));
    const high = Math.max(...bars.slice(8, 25).map((bar) => bar.high));
    const middle = (low + high) / 2;
    const point = (logical, price) => ({ logical, price });
    const drawing_definitions = [
      ["trend_line", [point(8, low), point(24, high)], { text: "trend" }],
      ["horizontal_line", [point(0, middle)]],
      ["horizontal_ray", [point(12, middle + 1)]],
      ["vertical_line", [point(18, 0)]],
      ["rectangle", [point(10, low), point(20, high)]],
      ["text", [point(22, high)], { text: "phase zero" }],
      ["brush", [point(26, low), point(30, high), point(34, middle)]],
      ["path", [point(36, low), point(40, high), point(44, middle)]],
      ["long_position", [point(46, middle), point(50, high + 1), point(50, low - 1)]],
      ["short_position", [point(52, middle), point(56, low - 1), point(56, high + 1)]],
    ];
    for (const [kind, points, options] of drawing_definitions) {
      chart.add_drawing(kind, points, options);
    }

    const before_range = chart.time_scale().get_visible_logical_range();
    chart.time_scale().set_visible_logical_range({
      from: before_range.from + 1,
      to: before_range.to + 1,
    });
    chart.time_scale().scroll_to_position(0, false);
    chart.set_crosshair_position(bars[32].close, bars[32].time, source);
    comparison_scale.set_auto_scale(false);
    comparison_scale.set_visible_range({ from: low - 2, to: high + 2 });
    chart.render();
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

    const named_scale = chart.price_scales(comparison_pane.pane_index())
      .find((scale) => scale.id === "phase-zero-comparison");
    return {
      backend: chart.backend(),
      canonical: [source, ...canonical].map((series) => series.series_type()),
      features: features.map((series) => series.series_type()),
      footprint: {
        type: footprint.series_type(),
        bars: footprint.footprint_bars().length,
      },
      indicator_kinds: indicators.map((series) => series.indicator_info().kind),
      drawing_kinds: chart.drawings().map((drawing) => drawing.kind()),
      pane_count: chart.panes().length,
      named_scale,
      comparison: {
        pane: comparison.pane_index(),
        scale: comparison.price_scale_id(),
      },
      snapshot_count: chart.value_snapshot().length,
      screenshot_length: chart.take_screenshot().toDataURL("image/png").length,
    };
  });

  expect(result.backend).toBe("canvas2d");
  expect(result.canonical).toEqual([
    "candlestick", "bar", "line", "area", "histogram", "baseline",
  ]);
  expect(new Set(result.features)).toEqual(new Set([
    "grouped_bars", "heatmap", "hlc_area", "pretty_histogram",
    "background_shade", "stacked_area", "stacked_bars", "whisker_box",
  ]));
  expect(result.footprint).toEqual({ type: "footprint", bars: 1 });
  expect(new Set(result.indicator_kinds)).toEqual(new Set([
    "sma", "ema", "ema_ribbon", "bollinger", "rsi",
    "macd", "stochastic", "atr", "vwap", "wma",
  ]));
  expect(result.drawing_kinds).toEqual([
    "trend_line", "horizontal_line", "horizontal_ray", "vertical_line", "rectangle",
    "text", "brush", "path", "long_position", "short_position",
  ]);
  expect(result.pane_count).toBeGreaterThanOrEqual(6);
  expect(result.named_scale).toMatchObject({
    id: "phase-zero-comparison",
    side: "left",
    visible: true,
    built_in: false,
  });
  expect(result.named_scale.series_ids).toHaveLength(1);
  expect(result.comparison).toEqual({ pane: 1, scale: "phase-zero-comparison" });
  expect(result.snapshot_count).toBeGreaterThanOrEqual(30);
  expect(result.screenshot_length).toBeGreaterThan(10_000);
});

test("bar close visibility reaches the browser frame for high-low bars", async ({ page }) => {
  await open_chart(page);

  const result = await page.evaluate(async () => {
    const chart = window.__chart;
    const bar = chart.add_series("bar", { price_line_visible: false, last_value_visible: false });
    bar.set_data([
      { time: 1, open: 100, high: 108, low: 96, close: 104 },
      { time: 2, open: 104, high: 111, low: 101, close: 107 },
      { time: 3, open: 107, high: 114, low: 103, close: 105 },
    ]);
    chart.time_scale().apply_options({ bar_spacing: 24 });
    const settle = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    await settle();
    bar.apply_options({ open_visible: false });
    await settle();
    bar.apply_options({ close_visible: false });
    await settle();
    return {
      options: bar.options(),
      data: bar.data(),
    };
  });

  expect(result.options).toMatchObject({ open_visible: false, close_visible: false });
  expect(result.data).toHaveLength(3);
});

test("stepped lines and point markers round-trip through the public series API", async ({ page }) => {
  await open_chart(page);

  const options = await page.evaluate(() => {
    const line = window.__chart.add_series("line", { price_line_visible: false, last_value_visible: false });
    line.set_data([
      { time: 1, value: 100 },
      { time: 2, value: 103 },
      { time: 3, value: 101 },
    ]);
    line.apply_options({ line_type: "stepped", point_markers: true, point_markers_radius: 5 });
    return line.options();
  });

  expect(options).toMatchObject({ line_type: "stepped", point_markers: true, point_markers_radius: 5 });
});

test("comparison overlays share one anchor and expose legend values", async ({ page }) => {
  await open_chart(page);

  const legend = await page.evaluate(() => {
    const chart = window.__chart;
    const first = chart.add_series("line", { title: "first", price_line_visible: false });
    const second = chart.add_series("line", { title: "second", price_line_visible: false });
    first.set_data([
      { time: 1, value: 100 },
      { time: 2, value: 110 },
      { time: 3, value: 120 },
    ]);
    second.set_data([
      { time: 1, value: 200 },
      { time: 2, value: 180 },
      { time: 3, value: 220 },
    ]);
    const changed = chart.set_comparison_anchor(2);
    return {
      changed,
      anchor: chart.comparison_anchor(),
      values: chart.comparison_legend_snapshot(),
    };
  });

  expect(legend.changed).toBe(true);
  expect(legend.anchor).toBe(2);
  expect(legend.values).toEqual(expect.arrayContaining([
    expect.objectContaining({ title: "first", anchor_value: 110, latest_value: 120 }),
    expect.objectContaining({ title: "second", anchor_value: 180, latest_value: 220 }),
  ]));
});

test("Heikin Ashi is a presentation projection while data stays raw", async ({ page }) => {
  await open_chart(page);

  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const candles = chart.add_series("candlestick", {
      heikin_ashi: true,
      price_line_visible: false,
      last_value_visible: false,
    });
    const raw = [
      { time: 1, open: 10, high: 14, low: 8, close: 12 },
      { time: 2, open: 12, high: 16, low: 10, close: 14 },
    ];
    candles.set_data(raw);
    const options = candles.options();
    const data = candles.data();
    chart.render();
    return { options, data };
  });

  expect(result.options).toMatchObject({ heikin_ashi: true });
  expect(result.data).toEqual([
    { time: 1, open: 10, high: 14, low: 8, close: 12 },
    { time: 2, open: 12, high: 16, low: 10, close: 14 },
  ]);
});
