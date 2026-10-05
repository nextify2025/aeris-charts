import { test, expect } from "@playwright/test";

test("host UTC boundaries drive an overlay and its higher-timeframe study", async ({ page }) => {
  await page.goto("/");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const result = await page.evaluate(() => {
    const chart = window.__chart;
    const base = 1_700_000_000;
    const source = chart.add_series("candlestick", { visible: false });
    const overlay = chart.add_series("candlestick");
    const volume_source = chart.add_series("histogram", { visible: false });
    const volume_target = chart.add_series("histogram", { visible: false });
    const options = {
      intervalSeconds: 120,
      boundaries: [
        { startTime: base, endTime: base + 240, sessionId: 11 },
        { startTime: base + 300, endTime: base + 540, sessionId: 12 },
      ],
    };
    chart.configure_resampled_series(source, overlay, options, volume_source, volume_target);
    const rsi = chart.add_rsi(overlay, 2);
    source.set_data([0, 60, 120, 180, 240, 300, 360, 420]
      .map((offset, index) => ({
        time: base + offset,
        open: index + 1,
        high: index + 2,
        low: index + 0.5,
        close: index + 1.5,
      })));
    volume_source.set_data([0, 30, 90, 120, 180, 300, 390, 450]
      .map((offset, index) => ({ time: base + offset, value: index + 1 })));
    const before = chart.resampled_bars(overlay);
    const volume_before = volume_target.data();
    source.update({ time: base + 420, open: 8, high: 10, low: 7.5, close: 9 });
    const after = chart.resampled_bars(overlay);
    const study = rsi.data();
    chart.remove_series(source);
    let overlay_removed = false;
    try {
      chart.resampled_bars(overlay);
    } catch {
      overlay_removed = true;
    }
    return { before, after, volume_before, study, overlay_removed };
  });

  expect(result.before.map((bar) => bar.timestamp)).toEqual([
    1_700_000_000, 1_700_000_120, 1_700_000_300, 1_700_000_420,
  ]);
  expect(result.before.map((bar) => bar.sessionId)).toEqual([11, 11, 12, 12]);
  expect(result.before.map((bar) => bar.close)).toEqual([2.5, 4.5, 7.5, 8.5]);
  expect(result.before.map((bar) => bar.volume)).toEqual([6, 9, 13, 8]);
  expect(result.volume_before.map((bar) => bar.value)).toEqual([6, 9, 13, 8]);
  expect(result.after.at(-1).close).toBe(9);
  expect(result.after.at(-1).high).toBe(10);
  expect(result.study.length).toBeGreaterThan(0);
  expect(result.overlay_removed).toBe(true);
});
