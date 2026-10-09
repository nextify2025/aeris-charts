import { expect } from "@playwright/test";
import { test, wait_for_chart } from "./page-ready.mjs";

test("persistence V1 round-trips all drawing kinds across stable multi-pane references", async ({ page }) => {
  page.on("console", (message) => console.log(`[browser:${message.type()}] ${message.text()}`));
  page.on("pageerror", (error) => console.log(`[browser:pageerror] ${error.message}`));
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = () => {
      const element = document.createElement("div");
      element.style.cssText = "position:absolute;left:-10000px;width:800px;height:500px";
      document.body.append(element);
      return element;
    };
    const first_host = host();
    const first = await create_chart(first_host, { backend: "canvas2d", autoSize: false });
    const second_pane = first.add_pane(true).pane_index();
    const additions = [
      ["trend_line", [{ logical: 1, price: 10 }, { logical: 3, price: 12 }], 0, { color: "#ff0000" }],
      ["horizontal_line", [{ logical: 0, price: 11 }], 0, { text: "level" }],
      ["horizontal_ray", [{ logical: 2, price: 11.5 }], 0, { style: "dashed" }],
      ["vertical_line", [{ logical: 4, price: 0 }], second_pane, { color: "#195bd6" }],
      ["rectangle", [{ logical: 1, price: 8 }, { logical: 5, price: 13 }], second_pane, { fill_color: "rgba(41,98,255,0.2)" }],
      ["text", [{ logical: 3, price: 10 }], second_pane, { text: "earnings", text_italic: true }],
      ["brush", [{ logical: 1, price: 9 }, { logical: 1.5, price: 10 }, { logical: 2, price: 9.5 }], second_pane, { width: 4 }],
    ];
    for (const [kind, anchors, pane, style] of additions) first.add_drawing(kind, anchors, style, pane);
    const state = first.export_state();
    first.remove();
    first_host.remove();

    const second_host = host();
    const second = await create_chart(second_host, { backend: "canvas2d", autoSize: false });
    const pre_import_pane = second.panes()[0];
    const restored = second.import_state(state);
    let stale_code = null;
    try { pre_import_pane.get_height(); } catch (error) { stale_code = error.code; }
    const canonical = second.export_state();
    const kinds = second.drawings().map((drawing) => drawing.kind());
    const pane_indexes = second.drawings().map((drawing) => drawing.pane_index());
    second.remove();
    second_host.remove();
    return { state, canonical, restored, kinds, pane_indexes, stale_code };
  });

  expect(result.restored).toEqual({ schema_version: 1, panes: 2, drawings: 7, points: 11 });
  expect(result.kinds).toEqual([
    "trend_line", "horizontal_line", "horizontal_ray", "vertical_line", "rectangle", "text", "brush",
  ]);
  expect(result.pane_indexes).toEqual([0, 0, 0, 1, 1, 1, 1]);
  expect(result.stale_code).toBe("stale_handle");
  expect(result.canonical).toEqual(result.state);
});

test("persistence failures are structured, bounded, and atomic", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const host = document.createElement("div");
    host.style.cssText = "position:absolute;left:-10000px;width:400px;height:300px";
    document.body.append(host);
    const chart = await create_chart(host, { backend: "canvas2d", autoSize: false });
    const before = chart.export_state();
    const errors = [];
    for (const document of [
      "{",
      { schema: "aeris_charts-state", schema_version: 99, panes: [], drawings: [] },
      {
        schema: "aeris_charts-state",
        schema_version: 1,
        panes: [{ id: "pane-1", stretch_factor: 1, preserve_empty: false }],
        drawings: [{ id: 1, kind: "future_tool", pane_id: "pane-1", anchors: [{ logical: 1, price: 1 }] }],
      },
      " ".repeat(8 * 1024 * 1024 + 1),
    ]) {
      try { chart.import_state(document); } catch (error) {
        errors.push({ name: error.name, code: error.code });
      }
    }
    const after = chart.export_state();
    chart.remove();
    host.remove();
    return { before, after, errors };
  });

  expect(result.errors).toEqual([
    { name: "AerisChartsError", code: "serialization_error" },
    { name: "AerisChartsError", code: "persistence_version_error" },
    { name: "AerisChartsError", code: "invalid_data" },
    { name: "AerisChartsError", code: "resource_limit" },
  ]);
  expect(result.after).toEqual(result.before);
});

test("V3 session studies persist the exchange calendar and a missing calendar reads as exchange", async ({ page }) => {
  await page.goto("/?backend=canvas2d");
  await wait_for_chart(page);
  const result = await page.evaluate(async () => {
    const { create_chart } = await import("/dist/aeris_charts_financial.js");
    const start = Date.UTC(2024, 0, 1) / 1000;
    const bars = [0, 600, 86_400, 7 * 86_400].map((offset, index) => ({
      time: start + offset, open: 10 + index, high: 12 + index, low: 8 + index, close: 11 + index,
    }));
    const build = async () => {
      const host = document.createElement("div");
      host.style.cssText = "position:absolute;left:-10000px;width:600px;height:400px";
      document.body.append(host);
      const chart = await create_chart(host, { backend: "canvas2d", autoSize: false });
      const source = chart.add_series("candlestick");
      source.set_data(bars);
      return { chart, host, source };
    };
    const studies = (chart) => chart.panes()
      .flatMap((pane) => pane.get_series())
      .filter((series) => series.indicator_info() !== null)
      .map((series) => ({
        kind: series.indicator_info().kind,
        calendar: series.indicator_info().parameters.calendar,
        values: series.data().map(({ value }) => value ?? null),
      }));
    const first = await build();
    first.chart.add_session_levels(first.source);
    first.chart.add_previous_period_levels(first.source, "week", "utc");
    const state = first.chart.export_state();
    const before = studies(first.chart);
    // A document written without the calendar field (a host-built or older document).
    const legacy = structuredClone(state);
    for (const study of legacy.indicators) delete study.kind.calendar;
    const second = await build();
    second.chart.import_state(legacy);
    const after = studies(second.chart);
    for (const built of [first, second]) {
      built.chart.remove();
      built.host.remove();
    }
    return { calendars: state.indicators.map((study) => study.kind.calendar), before, after };
  });
  expect(result.calendars).toEqual(["exchange", "utc"]);
  // Both studies read back as the exchange calendar, which equals UTC on this UTC chart.
  expect(result.after.map(({ calendar }) => calendar)).toEqual(Array(result.after.length).fill("exchange"));
  expect(result.after.map(({ values }) => values)).toEqual(result.before.map(({ values }) => values));
});
