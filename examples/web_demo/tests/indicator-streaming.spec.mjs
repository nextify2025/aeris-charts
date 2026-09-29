import { test, expect } from "@playwright/test";

// Live ticks advance every built-in study with bounded work: window studies re-evaluate only the
// forming bar, Stochastic RSI, pivots and ZigZag resume from retained recursive state, aggregate
// inputs derive only the changed row, and a volume column pairs with the candle by timestamp in
// either streaming order. A typed batch that closes the current bar and opens the next resumes
// from the state before the closed bar. The history crosses the runtimes' first sparse checkpoint
// (row 1,023). Through the public package this must leave every study exactly where a fresh
// install over the final data puts it.

async function wait_chart(page) {
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  await page.evaluate(() => new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  }));
}

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/");
  await wait_chart(page);
});

test("streamed ticks leave bounded incremental studies equal to a fresh install", async ({ page }) => {
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const start = 1_700_006_400;
    const bar = (index, revision) => {
      const close = 100 + Math.sin(index * 0.35) * 8 + revision * 0.013;
      const open = close - Math.cos(index * 0.9) * 0.6;
      return {
        time: start + index * 3_600,
        open,
        high: Math.max(open, close) + 0.7,
        low: Math.min(open, close) - 0.5,
        close,
      };
    };
    const volume_at = (index, revision) => ((index * 7 + revision) % 11) + 1;

    // One price/volume pair with every bounded study kind bound to it.
    const install = (bars, volumes) => {
      const price = chart.add_series("candlestick", { visible: false });
      const volume = chart.add_series("histogram", { visible: false });
      price.set_data(bars);
      volume.set_data(volumes);
      const hidden = { visible: false };
      const studies = [
        chart.add_zigzag(price, 3, hidden),
        ...chart.add_pivot_points(price, "camarilla", hidden),
        chart.add_stochastic_rsi(price, 5, 5, hidden),
        ...chart.add_ichimoku(price, hidden),
        chart.add_cci(price, 5, hidden),
        chart.add_williams_r(price, 5, hidden),
        chart.add_standard_deviation(price, 5, hidden),
        chart.add_momentum(price, 5, hidden),
        chart.add_roc(price, 5, hidden),
        ...chart.add_donchian(price, 5, hidden),
        chart.add_cmf(price, 5, volume, hidden),
        chart.add_mfi(price, 5, volume, hidden),
        chart.add_obv(price, volume, hidden),
        chart.add_vwap(price, volume, hidden),
        chart.add_vwma(price, 5, volume, hidden),
        ...chart.add_volume(price, 5, volume, hidden),
        chart.add_rsi_with_source(price, "hlc3", 5, hidden),
        ...chart.add_bollinger_with_source(price, "ohlc4", 5, 2, hidden),
      ];
      const hl2 = chart.add_stochastic_rsi(price, 5, 5, hidden);
      chart.set_indicator_input_source(hl2, "hl2");
      studies.push(hl2);
      return { price, volume, studies };
    };

    const history = 1_030;
    const bars = Array.from({ length: history }, (_, index) => bar(index, 0));
    const volumes = bars.map((row, index) => ({ time: row.time, value: volume_at(index, 0) }));
    const live = install(bars, volumes);
    const tick = (index, revision, volume_first) => {
      const row = bar(index, revision);
      const volume = { time: row.time, value: volume_at(index, revision) };
      if (volume_first) {
        live.volume.update(volume);
        live.price.update(row);
      } else {
        live.price.update(row);
        live.volume.update(volume);
      }
      bars[index] = row;
      volumes[index] = volume;
    };
    for (let revision = 1; revision <= 4; revision += 1) {
      tick(history - 1, revision, revision % 2 === 0);
    }
    for (let index = history; index < history + 30; index += 1) {
      tick(index, 0, index % 2 === 0);
      tick(index, 1, index % 3 === 0);
      tick(index, 2, false);
    }
    // Close the current bar and open the next one in a single typed batch, then stream their
    // volumes.
    for (let last = bars.length - 1, step = 0; step < 3; last += 1, step += 1) {
      const rows = [bar(last, 5), bar(last + 1, 0)];
      const column = (key) => Float64Array.from(rows, (row) => row[key]);
      live.price.update_typed({
        times: column("time"),
        open: column("open"),
        high: column("high"),
        low: column("low"),
        close: column("close"),
      });
      bars[last] = rows[0];
      bars[last + 1] = rows[1];
      for (const [offset, row] of rows.entries()) {
        const volume = { time: row.time, value: volume_at(last + offset, 5 - offset * 5) };
        live.volume.update(volume);
        volumes[last + offset] = volume;
      }
    }

    const fresh = install(bars, volumes);
    return {
      count: live.studies.length,
      outputs: live.studies.map((series, index) => ({
        live: series.data(),
        fresh: fresh.studies[index].data(),
      })),
    };
  });
  expect(result.count).toBeGreaterThan(30);
  for (const [index, { live, fresh }] of result.outputs.entries()) {
    expect(live.length, `study output ${index} rows`).toBe(fresh.length);
    expect(live, `study output ${index} values`).toEqual(fresh);
  }
});
