import React, { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import {
  FinancialSeries,
  GeneralPane,
  AerisChart,
  type GeneralAxisSpec,
  type GeneralSeriesSpec,
} from "../../packages/charts/dist/react.js";
import type { chart_api, general_series_api, pane_api, series_api } from "../../packages/charts/dist/types.js";

const axes: readonly GeneralAxisSpec[] = [
  { id: "month", dimension: "x", position: "bottom", scale: "band" },
  { id: "revenue", dimension: "y", position: "left", scale: "linear" },
  { id: "revenue-alt", dimension: "y", position: "right", scale: "linear" },
];

const pane_options = { horizontal_domain: { type: "category" as const, scale: "band" as const } };

function wait_until(predicate: () => boolean, timeout_ms = 10_000): Promise<void> {
  const started = performance.now();
  return new Promise((resolve, reject) => {
    const poll = () => {
      if (predicate()) {
        resolve();
        return;
      }
      if (performance.now() - started > timeout_ms) {
        reject(new Error("timed out waiting for React chart fixture"));
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
}

function safely(predicate: () => boolean): boolean {
  try {
    return predicate();
  } catch {
    return false;
  }
}

/** Browser-only Phase 3 evidence used by Playwright; not part of the published package. */
export async function exerciseReactAdapter(): Promise<Record<string, unknown>> {
  const react_errors: string[] = [];
  const window_errors: string[] = [];
  const original_console_error = console.error;
  const on_window_error = (event: ErrorEvent) => {
    window_errors.push(event.error instanceof Error ? event.error.message : event.message);
  };
  window.addEventListener("error", on_window_error);
  console.error = (...args: unknown[]) => {
    react_errors.push(args.map((value) => String(value)).join(" "));
    original_console_error(...args);
  };
  const host = document.createElement("div");
  host.style.width = "720px";
  host.style.height = "480px";
  document.body.appendChild(host);
  const root = createRoot(host);

  let chart: chart_api | null = null;
  let financial: series_api | null = null;
  let general: general_series_api | null = null;
  let forecast: general_series_api | null = null;
  let pane: pane_api | null = null;

  const render = (
    financial_close: number,
    revenue_value: number,
    show_children = true,
    title = "Revenue",
    use_alternate_axis = false,
    reverse_general_order = false,
  ) => {
    const render_axes = axes.map((axis) => axis.id === "revenue"
      ? { ...axis, title: title === "Revenue" ? "Revenue axis" : "Updated revenue axis" }
      : axis);
    const general_series: readonly GeneralSeriesSpec[] = [{
      key: "revenue",
      kind: "column",
      options: {
        x_axis_id: "month",
        y_axis_id: use_alternate_axis ? "revenue-alt" : "revenue",
        title,
        color: title === "Revenue" ? "#2563eb" : "#dc2626",
      },
      data: [{ id: "jan", x: "Jan", y: revenue_value }],
    }, {
      key: "forecast",
      kind: "xy_line",
      options: { x_axis_id: "month", y_axis_id: "revenue", title: "Forecast", color: "#16a34a" },
      data: [{ id: "jan-forecast", x: "Jan", y: revenue_value + 5 }],
    }];
    const ordered_general_series = reverse_general_order
      ? [...general_series].reverse()
      : general_series;
    root.render(
      <StrictMode>
        <AerisChart
          options={{ backend: "canvas2d", autoSize: false, accessibility: false }}
          style={{ width: "720px", height: "480px" }}
          onChartReady={(value) => { chart = value; }}
        >
          {show_children ? <>
            <FinancialSeries
              kind="candlestick"
              data={[{ time: 1735689600, open: 1, high: 4, low: 1, close: financial_close }]}
              onSeriesReady={(value) => { financial = value; }}
            />
            <GeneralPane
              options={pane_options}
              axes={render_axes}
              series={ordered_general_series}
              onPaneReady={(value) => { pane = value; }}
              onSeriesReady={(key, value) => {
                if (key === "revenue") general = value;
                if (key === "forecast") forecast = value;
              }}
            />
          </> : null}
        </AerisChart>
      </StrictMode>,
    );
  };

  render(2, 42);
  try {
    await wait_until(() => chart !== null && financial !== null && general !== null && forecast !== null && pane !== null
      && safely(() => (chart as chart_api).backend().length > 0)
      && safely(() => (financial as series_api).data()[0]?.close === 2)
      && safely(() => (general as general_series_api).dataAt(0)?.value === 42)
      && safely(() => (pane as pane_api).paneIndex() >= 0));
  } catch {
    console.error = original_console_error;
    window.removeEventListener("error", on_window_error);
    throw new Error(
      `React fixture readiness: chart=${chart !== null} financial=${financial !== null} general=${general !== null} forecast=${forecast !== null} pane=${pane !== null} children=${host.childElementCount} errors=${react_errors.join(" | ")} window=${window_errors.join(" | ")}`,
    );
  }
  const first_chart = chart as chart_api;
  const first_financial = financial as series_api;
  const first_general = general as general_series_api;
  const first_forecast = forecast as general_series_api;
  const first_pane = pane as pane_api;
  const financial_id = first_financial.id;
  const general_id = first_general.id;
  const pane_index = first_pane.paneIndex();

  render(3, 57, true, "Updated revenue", true, true);
  await wait_until(() => safely(() => (financial as series_api).data()[0]?.close === 3));
  await wait_until(() => safely(() => (general as general_series_api).dataAt(0)?.value === 57));
  await wait_until(() => safely(() => (general as general_series_api).options().title === "Updated revenue"));
  await wait_until(() => safely(() => (general as general_series_api).options().y_axis_id === "revenue-alt"));
  await wait_until(() => safely(() => first_chart.general_series_order(pane_index)[0] === first_forecast));
  await wait_until(() => safely(() => first_chart.axis("revenue")?.options().title === "Updated revenue axis"));

  const result = {
    same_chart: chart === first_chart,
    same_financial_handle: financial === first_financial,
    same_general_handle: general === first_general,
    same_forecast_handle: forecast === first_forecast,
    same_financial_id: (financial as series_api).id === financial_id,
    same_general_id: (general as general_series_api).id === general_id,
    updated_general_title: (general as general_series_api).options().title,
    updated_general_color: (general as general_series_api).options().color,
    updated_general_axis: (general as general_series_api).options().y_axis_id,
    updated_general_order: (
      first_chart.general_series_order(pane_index)[0] === first_forecast
      && first_chart.general_series_order(pane_index)[1] === first_general
    ),
    updated_axis_title: first_chart.axis("revenue")?.options().title,
    pane_index_stable: (pane as pane_api).paneIndex() === pane_index,
    pane_count: first_chart.panes().length,
    axis_count: first_chart.axes(pane_index).length,
    general_legend_count: first_chart.general_legend_snapshot(pane_index).items.length,
  };

  render(3, 57, false);
  await wait_until(() => first_chart.panes().length === 1);
  await wait_until(() => first_chart.axes().length === 0);
  await wait_until(() => first_chart.series_order().length === 0);
  const child_cleanup = first_chart.general_legend_snapshot().items.length === 0;
  const general_pane_stale = !safely(() => first_pane.pane_index() >= 0);

  root.unmount();
  await new Promise((resolve) => setTimeout(resolve, 0));
  let disposed = false;
  try {
    first_chart.backend();
  } catch {
    disposed = true;
  }
  host.remove();
  console.error = original_console_error;
  window.removeEventListener("error", on_window_error);
  return { ...result, child_cleanup, general_pane_stale, disposed };
}

class FailureBoundary extends React.Component<{
  children: React.ReactNode;
  onFailure: (error: Error) => void;
}, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  componentDidCatch(error: Error): void {
    this.props.onFailure(error);
  }

  render(): React.ReactNode {
    return this.state.failed ? null : this.props.children;
  }
}

/** Prove a rejected first data install cannot strand an untracked engine series or pane. */
export async function exerciseReactFailureCleanup(): Promise<Record<string, unknown>> {
  const host = document.createElement("div");
  host.style.cssText = "width:720px;height:480px";
  document.body.appendChild(host);
  const root = createRoot(host);
  let chart: chart_api | null = null;
  let failure: Error | null = null;
  const invalid_series: readonly GeneralSeriesSpec[] = [{
    key: "invalid",
    kind: "column",
    options: { x_axis_id: "month", y_axis_id: "revenue" },
    data: [{ id: "bad", x: 7 as unknown as string, y: 42 }],
  }];
  const original_console_error = console.error;
  console.error = () => {};
  try {
    root.render(
      <AerisChart
        options={{ backend: "canvas2d", autoSize: false, accessibility: false }}
        onChartReady={(value) => { chart = value; }}
      >
        <FailureBoundary onFailure={(error) => { failure = error; }}>
          <GeneralPane options={pane_options} axes={axes} series={invalid_series} />
        </FailureBoundary>
      </AerisChart>,
    );
    await wait_until(() => chart !== null && failure !== null);
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    const live = chart as chart_api;
    const result = {
      failure: (failure as Error).message,
      pane_count: live.panes().length,
      axis_count: live.axes().length,
      legend_count: live.general_legend_snapshot().items.length,
    };
    root.unmount();
    await new Promise((resolve) => setTimeout(resolve, 0));
    return result;
  } finally {
    console.error = original_console_error;
    root.unmount();
    host.remove();
  }
}

/**
 * Declarative streaming evidence: a `FinancialSeries` whose `data` prop changes only by a replaced
 * last bar and/or appended bars must stream through `update()` (one `data_changed("update")` per
 * changed bar) instead of a full `setData`; any other change falls back to one `setData`.
 */
export async function exerciseReactStreaming(): Promise<Record<string, unknown>> {
  const host = document.createElement("div");
  host.style.cssText = "width:720px;height:480px";
  document.body.appendChild(host);
  const root = createRoot(host);
  let series: series_api | null = null;
  let scopes: string[] = [];
  const t0 = 1735689600;
  const bar = (index: number, close = 10.5 + index) => ({
    time: t0 + index * 60,
    open: 10 + index,
    high: Math.max(11 + index, close),
    low: Math.min(9 + index, close),
    close,
  });
  type bar_data = ReturnType<typeof bar>;
  const render = (data: readonly bar_data[]) => {
    root.render(
      <AerisChart options={{ backend: "canvas2d", autoSize: false, accessibility: false }}>
        <FinancialSeries
          kind="candlestick"
          data={data}
          onSeriesReady={(value) => {
            series = value;
            value.subscribe_data_changed((scope) => { scopes.push(scope); });
          }}
        />
      </AerisChart>,
    );
  };
  const step = async (data: readonly bar_data[], ready: (live: series_api) => boolean) => {
    scopes = [];
    render(data);
    await wait_until(() => series !== null && safely(() => ready(series as series_api)));
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    return [...scopes];
  };
  const close_at = (live: series_api, index: number) =>
    (live.data()[index] as { close?: number } | undefined)?.close;

  try {
    const initial = Array.from({ length: 100 }, (_, index) => bar(index));
    const initial_scopes = await step(initial, (live) => live.data().length === 100);

    const replaced = [...initial.slice(0, -1), bar(99, 250)];
    const replace_scopes = await step(replaced, (live) => close_at(live, 99) === 250);

    const appended = [...replaced, bar(100)];
    const append_scopes = await step(appended, (live) => live.data().length === 101);

    const replaced_and_appended = [...appended.slice(0, -1), bar(100, 300), bar(101), bar(102)];
    const mixed_scopes = await step(replaced_and_appended, (live) => live.data().length === 103
      && close_at(live, 100) === 300);

    // Re-created objects with identical values still stream (shallow equality), then append.
    const recreated = [...replaced_and_appended.map((point) => ({ ...point })), bar(103)];
    const recreated_scopes = await step(recreated, (live) => live.data().length === 104);

    const history_edit = recreated.map((point, index) => index === 10 ? bar(10, 999) : point);
    const history_scopes = await step(history_edit, (live) => close_at(live, 10) === 999);

    const removed = history_edit.slice(0, -1);
    const removal_scopes = await step(removed, (live) => live.data().length === 103);

    // Streaming hosts often mutate the forming bar and push onto the array they passed last time,
    // then pass a copy. The applied snapshot still sees the replaced bar and the appended one.
    const forming = removed[removed.length - 1]!;
    forming.high = 777;
    forming.close = 777;
    removed.push(bar(103));
    const final_data = [...removed];
    const in_place_scopes = await step(final_data, (live) => live.data().length === 104
      && close_at(live, 102) === 777);

    const live = series as unknown as series_api;
    const rows = live.data() as unknown as readonly bar_data[];
    const final_matches = rows.length === final_data.length
      && final_data.every((point, index) => {
        const row = rows[index]!;
        return row.time === point.time && row.open === point.open && row.high === point.high
          && row.low === point.low && row.close === point.close;
      });
    return {
      initial_scopes,
      replace_scopes,
      append_scopes,
      mixed_scopes,
      recreated_scopes,
      history_scopes,
      removal_scopes,
      in_place_scopes,
      final_matches,
    };
  } finally {
    root.unmount();
    host.remove();
  }
}
