import { test, expect } from "@playwright/test";

const start = 1_700_000_000;
const candles = [
  { time: start, open: 9, high: 10, low: 8, close: 9 },
  { time: start + 60, open: 12, high: 14, low: 9, close: 13 },
  { time: start + 120, open: 12, high: 12, low: 10, close: 11 },
  { time: start + 180, open: 16, high: 18, low: 15, close: 17 },
  { time: start + 240, open: 16, high: 16, low: 15, close: 15 },
  { time: start + 300, open: 18, high: 20, low: 8, close: 19 },
  { time: start + 360, open: 19, high: 21, low: 16, close: 20 },
];

test("replay seeks in both directions match fresh prefix loads without a replay clock", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
  const cursors = [3, 6, 2, 5, 1, 4, 0, 6];
  const replay = await page.evaluate(({ candles, cursors }) => {
    const chart = window.__chart;
    chart.remove_series(window.__main);
    const source = chart.add_series("candlestick");
    source.set_data(candles);
    const market = chart.add_market_structure(source, 1, 1);
    const gaps = chart.add_fair_value_gaps(source);
    const blocks = chart.add_order_blocks(source, { left: 1, right: 1 });
    const levels = chart.add_session_levels(source);
    const snapshot = () => ({
      market: chart.study_annotations(market),
      gaps: chart.study_annotations(gaps),
      blocks: chart.study_annotations(blocks),
      levels: levels.map((output) => output.data().map(({ time, value }) => ({ time, value: value ?? null }))),
    });
    const complete = snapshot();
    const sought = cursors.map((cursor) => {
      chart.set_replay_clock_micros(candles[cursor].time * 1_000_000);
      return { cursor, clock: chart.replay_clock_micros(), snapshot: snapshot() };
    });
    chart.set_replay_clock_micros(null);
    return { complete, sought, resumed: snapshot() };
  }, { candles, cursors });

  expect(replay.complete.market.markers.some(({ row }) => row > 3)).toBe(true);
  expect(replay.complete.blocks.zones.some(({ confirm_row }) => confirm_row > 3)).toBe(true);
  expect(replay.complete.levels[0]).toHaveLength(candles.length);
  expect(replay.complete.levels[0].at(-1).value).toBe(21);
  expect(replay.resumed).toEqual(replay.complete);

  for (const { cursor, clock, snapshot } of replay.sought) {
    expect(clock).toBe(candles[cursor].time * 1_000_000);
    for (const { markers, zones } of [snapshot.market, snapshot.gaps, snapshot.blocks]) {
      expect(markers.every(({ row, confirm_row }) => row <= cursor && confirm_row <= cursor)).toBe(true);
      expect(zones.every(({ start_row, confirm_row, end_row }) =>
        start_row <= cursor && confirm_row <= cursor && (end_row === null || end_row <= cursor))).toBe(true);
    }
    expect(snapshot.levels.every((output) => output.length === cursor + 1)).toBe(true);

    await page.reload();
    await page.waitForFunction(() => window.__chart?.backend?.() !== undefined);
    const fresh = await page.evaluate(({ candles, cursor }) => {
      const chart = window.__chart;
      chart.remove_series(window.__main);
      const source = chart.add_series("candlestick");
      source.set_data(candles.slice(0, cursor + 1));
      const market = chart.add_market_structure(source, 1, 1);
      const gaps = chart.add_fair_value_gaps(source);
      const blocks = chart.add_order_blocks(source, { left: 1, right: 1 });
      const levels = chart.add_session_levels(source);
      return {
        clock: chart.replay_clock_micros(),
        snapshot: {
          market: chart.study_annotations(market),
          gaps: chart.study_annotations(gaps),
          blocks: chart.study_annotations(blocks),
          levels: levels.map((output) => output.data().map(({ time, value }) => ({ time, value: value ?? null }))),
        },
      };
    }, { candles, cursor });
    expect(fresh.clock).toBeNull();
    expect(snapshot).toEqual(fresh.snapshot);
  }
});
