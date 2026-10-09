import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";

// Platform indicator-chrome building blocks: series lifecycle events, indicator lineage,
// per-pane geometry anchors (top-left chip placement), hover values for the main series and
// indicator outputs, and the split divider live-tracking the axis border token. The engine
// owns no chips — it only supplies these APIs.

async function wait_grid(page) {
  await wait_for_chart(page, { grid: true });
}

async function wait_cell_charts(page) {
  await page.waitForFunction(() => window.__grid.cells().every((c) => !!c.chart));
  await wait_grid(page);
}

test("indicator lifecycle events, lineage, pane geometry, and hover values feed platform chips", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);

  // Added event + lineage: add_sma fires one event per output with its birth pane.
  const setup = await page.evaluate(() => {
    const events = [];
    window.__chart.subscribe_series_added((e) => events.push({ id: e.series.id, pane: e.pane_index }));
    const sma = window.__chart.add_sma(window.__main, 3);
    window.__sma = sma;
    const info = sma.indicator_info();
    return {
      events,
      sma_id: sma.id,
      main_id: window.__main.id,
      info: { kind: info.kind, period: info.period, deviation: info.deviation, source: info.source.id, output_index: info.output_index },
      main_info: window.__main.indicator_info(), // a plain series reports null
    };
  });
  expect(setup.events).toEqual([{ id: setup.sma_id, pane: 0 }]);
  expect(setup.info).toEqual({ kind: "sma", period: 3, deviation: null, source: setup.main_id, output_index: 0 });
  expect(setup.main_info).toBeNull();

  // Hover values: the crosshair-move params carry the main series' OHLC and the indicator's
  // value at the same bar — the platform legend/chip text.
  await page.evaluate(() => {
    window.__hover = null;
    window.__chart.subscribe_crosshair_move((p) => {
      window.__hover = { values: [...p.series_data.entries()].map(([s, d]) => ({ id: s.id, ...d })) };
    });
  });
  const canvas = await page.locator("#chart_container canvas").first().boundingBox();
  // The renderer (SwiftShader WebGPU init) drops CDP pointer moves for a while after load —
  // keep walking the cursor until the hover stream starts instead of waiting on one move.
  let hover = null;
  for (let i = 0; i < 25 && hover === null; i++) {
    await page.mouse.move(canvas.x + canvas.width / 2 - 60 + (i % 5) * 30, canvas.y + canvas.height / 2);
    await page.waitForTimeout(250);
    hover = await page.evaluate(() => window.__hover);
  }
  expect(hover, "crosshair-move emitted per-series values").not.toBeNull();
  const main_bar = hover.values.find((v) => v.id === setup.main_id);
  const sma_bar = hover.values.find((v) => v.id === setup.sma_id);
  expect(main_bar.open, "main series reports OHLC").toBeGreaterThan(0);
  expect(main_bar.high).toBeGreaterThan(0);
  expect(main_bar.low).toBeGreaterThan(0);
  expect(main_bar.close).toBeGreaterThan(0);
  expect(sma_bar.value, "indicator output reports its value at the hovered bar").toBeGreaterThan(0);

  // Separate pane + geometry: the indicator moves to pane 1, whose geometry anchors a
  // platform-rendered chip at the pane's top-left corner.
  const geo = await page.evaluate(() => {
    window.__sma.move_to_pane(1, 0.3);
    window.__chart.render();
    const g = window.__chart.panes()[1].get_geometry();
    const chip = document.createElement("div");
    chip.id = "test_indicator_chip";
    chip.style.cssText = `position:absolute;left:${g.left}px;top:${g.top}px;width:48px;height:14px;z-index:10;background:magenta;`;
    window.__grid.cells()[0].element.appendChild(chip); // the cell slot is the chart's offset parent
    return g;
  });
  expect(geo.top, "pane 1 starts below pane 0").toBeGreaterThan(100);
  expect(geo.width).toBeGreaterThan(100);
  expect(geo.height).toBeGreaterThan(20);
  const chip_box = await page.locator("#test_indicator_chip").boundingBox();
  const slot_box = await page.evaluate(() => window.__grid.cells()[0].element.getBoundingClientRect().toJSON());
  expect(chip_box.x, "chip left edge lands at the pane's left").toBeCloseTo(slot_box.x + geo.left, 0);
  expect(chip_box.y, "chip top edge lands at the pane's top").toBeCloseTo(slot_box.y + geo.top, 0);

  // Per-pane indicator enumeration: pane 1 holds exactly the SMA output.
  const pane_series = await page.evaluate(() => window.__chart.panes()[1].get_series().map((s) => ({ id: s.id, indicator: s.indicator_info()?.kind ?? null })));
  expect(pane_series).toEqual([{ id: setup.sma_id, indicator: "sma" }]);

  // Removal cascade: removing a source fires one removed event per tombstoned series —
  // the source AND its derived indicator output (with the pane it lived on).
  const removed = await page.evaluate(() => {
    const events = [];
    window.__chart.subscribe_series_removed((e) => events.push({ id: e.series.id, pane: e.pane_index }));
    const extra = window.__chart.add_series("line");
    extra.set_data([
      { time: 1, value: 10 }, { time: 2, value: 11 }, { time: 3, value: 12 },
    ]);
    const sma2 = window.__chart.add_sma(extra, 2);
    window.__chart.remove_series(extra);
    return { events, extra_id: extra.id, sma2_id: sma2.id };
  });
  expect(removed.events).toContainEqual({ id: removed.extra_id, pane: 0 });
  expect(removed.events).toContainEqual({ id: removed.sma2_id, pane: 0 });
});

test("split divider live-tracks the axis border token on apply_options (no topology change)", async ({ page }) => {
  await page.goto("/");
  await wait_grid(page);
  await page.click("#split_h");
  await page.waitForFunction(() => document.querySelectorAll("#chart_container canvas").length === 8);
  await wait_cell_charts(page);

  const divider_rgb = () =>
    page.locator(".aeris_charts-grid-divider >> nth=0").evaluate((el) => {
      // The line color is the solid inner strip's background.
      const inner = el.firstElementChild;
      return inner ? getComputedStyle(inner).backgroundColor : "";
    });

  // Direct apply_options on the primary chart — no demo input, no split/remove reconcile.
  const patches = await page.evaluate(() => {
    const seen = [];
    window.__chart.subscribe_options_change((o) => seen.push(o));
    window.__chart.apply_options({ rightPriceScale: { borderColor: "#ff8800" } });
    return seen;
  });
  expect(patches).toEqual([{ rightPriceScale: { borderColor: "#ff8800" } }]);
  await expect.poll(divider_rgb).toBe("rgb(255, 136, 0)");

  // And back: the divider keeps following the same token, not a pinned color.
  await page.evaluate(() => window.__chart.apply_options({ rightPriceScale: { borderColor: "#2B2B43" } }));
  await expect.poll(divider_rgb).toBe("rgb(43, 43, 67)");
});
