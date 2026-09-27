import {
  createContext,
  createElement,
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import { createChart } from "./index.js";
import type {
  any_series_options,
  chart_api,
  chart_options,
  deep_partial,
  general_axis_api,
  general_axis_options,
  general_pane_options,
  general_series_api,
  general_series_kind,
  general_series_options,
  pane_api,
  series_api,
  series_data,
  series_kind,
  time,
} from "./types.js";

const chart_context = createContext<chart_api | null>(null);

/** Return the live Aeris chart owned by the nearest {@link AerisChart}. */
export function useAerisChart(): chart_api {
  const chart = useContext(chart_context);
  if (chart === null) throw new Error("useAerisChart must be used inside <AerisChart>");
  return chart;
}

export interface AerisChartProps {
  options?: deep_partial<chart_options>;
  className?: string;
  style?: CSSProperties;
  children?: ReactNode;
  onChartReady?: (chart: chart_api) => void;
}

/**
 * React lifecycle adapter over the ordinary browser chart. The chart is created once per mounted
 * component, option changes are applied to the same handle, and cleanup calls the canonical
 * `chart.remove()` path. No DOM work runs during server rendering.
 */
export function AerisChart({ options, className, style, children, onChartReady }: AerisChartProps) {
  const host_ref = useRef<HTMLDivElement | null>(null);
  const on_ready_ref = useRef(onChartReady);
  const initial_options_ref = useRef(options);
  const [chart, set_chart] = useState<chart_api | null>(null);
  const [failure, set_failure] = useState<unknown>(null);
  on_ready_ref.current = onChartReady;

  useEffect(() => {
    const host = host_ref.current;
    if (host === null) return;
    let live = true;
    let owned: chart_api | null = null;

    void createChart(host, initial_options_ref.current).then((created) => {
      if (!live) {
        created.remove();
        return;
      }
      owned = created;
      set_chart(created);
      on_ready_ref.current?.(created);
    }, (error: unknown) => {
      if (live) set_failure(error);
    });

    return () => {
      live = false;
      owned?.remove();
    };
  }, []);

  useEffect(() => {
    if (chart !== null && options !== undefined) chart.applyOptions(options);
  }, [chart, options]);

  if (failure !== null) throw failure;

  const contents = chart === null
    ? null
    : createElement(chart_context.Provider, { value: chart }, children);
  return createElement("div", {
    ref: host_ref,
    className,
    style: { position: "relative", ...style },
  }, contents);
}

export type react_financial_series_kind = Exclude<series_kind, "custom">;

/** Longest changed tail streamed through `update()`; a larger change is one full `setData`. */
const STREAM_TAIL_LIMIT = 1_024;

/** Order two input times of the same form, or `null` when the forms differ. */
function time_order(a: time, b: time): number | null {
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "string" && typeof b === "string") return a < b ? -1 : a > b ? 1 : 0;
  if (typeof a === "object" && typeof b === "object" && a !== null && b !== null) {
    return a.year - b.year || a.month - b.month || a.day - b.day;
  }
  return null;
}

/** Shallow point equality: the same object, or the same own keys with identical values. */
function same_point(a: series_data, b: series_data): boolean {
  if (a === b) return true;
  const a_record = a as unknown as Record<string, unknown>;
  const b_record = b as unknown as Record<string, unknown>;
  const keys = Object.keys(a_record);
  if (keys.length !== Object.keys(b_record).length) return false;
  return keys.every((key) => a_record[key] === b_record[key]
    || (key === "time" && time_order(a.time, b.time) === 0));
}

/**
 * What a `FinancialSeries` last applied. React state is immutable by contract, but streaming hosts
 * often mutate the forming bar (or push onto their array) in place before passing a new array; the
 * applied length and a shallow copy of the applied last point keep that common case diffed against
 * what the chart actually holds rather than against the mutated objects.
 */
interface applied_series_data {
  series: series_api;
  data: readonly series_data[];
  length: number;
  last: series_data;
}

function applied_state(series: series_api, data: readonly series_data[]): applied_series_data | null {
  const last = data[data.length - 1];
  return last === undefined ? null : { series, data, length: data.length, last: { ...last } };
}

/**
 * The points to stream when `next` differs from the `applied` data only by a replaced last point
 * and/or an ascending appended tail, in the order `update()` must apply them. `null` means the
 * change is anything else and needs a full replace.
 */
function streaming_tail(
  applied: applied_series_data,
  next: readonly series_data[],
): readonly series_data[] | null {
  const last = applied.length - 1;
  if (next.length < applied.length || next.length - last > STREAM_TAIL_LIMIT) return null;
  for (let index = 0; index < last; index += 1) {
    if (!same_point(applied.data[index]!, next[index]!)) return null;
  }
  const tail: series_data[] = [];
  if (!same_point(applied.last, next[last]!)) {
    if (time_order(applied.last.time, next[last]!.time) !== 0) return null;
    tail.push(next[last]!);
  }
  for (let index = applied.length; index < next.length; index += 1) {
    const order = time_order(next[index - 1]!.time, next[index]!.time);
    if (order === null || order >= 0) return null;
    tail.push(next[index]!);
  }
  return tail;
}

export interface FinancialSeriesProps {
  kind: react_financial_series_kind;
  data: readonly series_data[];
  options?: Partial<any_series_options>;
  onSeriesReady?: (series: series_api) => void;
}

/**
 * A financial series whose engine identity is retained across ordinary React rerenders.
 *
 * Declarative streaming: when a new `data` array differs from the previously applied one only by
 * a replaced last point and/or appended later points (the usual immutable
 * `[...prev.slice(0, -1), bar]` / `[...prev, bar]` updates), the changed points go through
 * `series.update()` in O(1) each instead of a full `setData`. Unchanged points are recognized by
 * identity or shallow equality (one linear identity scan per change); the last point is compared
 * with a copy taken when it was applied, so mutating the forming bar in place still streams. Any
 * other change (history edits, removals, reordering, a new series, or a previously rejected
 * install) falls back to one `setData`. Edits made by mutating older points in place are not
 * detected, as React props are immutable by contract.
 */
export function FinancialSeries({ kind, data, options, onSeriesReady }: FinancialSeriesProps) {
  const chart = useAerisChart();
  const options_ref = useRef(options);
  const ready_ref = useRef(onSeriesReady);
  const applied_ref = useRef<applied_series_data | null>(null);
  const [series, set_series] = useState<series_api | null>(null);
  options_ref.current = options;
  ready_ref.current = onSeriesReady;

  useEffect(() => {
    const created = chart.addSeries(kind as series_kind, options_ref.current);
    set_series(created);
    ready_ref.current?.(created);
    return () => chart.removeSeries(created);
  }, [chart, kind]);

  useEffect(() => {
    if (series === null) return;
    const applied = applied_ref.current;
    const tail = applied !== null && applied.series === series ? streaming_tail(applied, data) : null;
    let streamed = tail !== null;
    for (const point of tail ?? []) {
      series.update(point);
      if (series.last_ingestion_diagnostics()?.status === "rejected") {
        streamed = false;
        break;
      }
    }
    if (!streamed) series.setData(data);
    applied_ref.current = series.last_ingestion_diagnostics()?.status === "rejected"
      ? null
      : applied_state(series, data);
  }, [series, data]);

  useEffect(() => {
    if (series !== null && options !== undefined) series.applyOptions(options);
  }, [series, options]);

  return null;
}

export type GeneralAxisSpec = Omit<general_axis_options, "pane">;

export interface GeneralSeriesSpec {
  /** Stable React-side identity. Changing this value creates a different engine series. */
  key: string;
  kind: general_series_kind;
  options: Omit<general_series_options, "pane">;
  data: Parameters<general_series_api["set_data"]>[0];
}

export interface GeneralPaneProps {
  /** The horizontal-domain contract. The adapter retains this pane while mounted, even when empty. */
  options: Omit<general_pane_options, "preserve_empty">;
  axes: readonly GeneralAxisSpec[];
  series: readonly GeneralSeriesSpec[];
  onPaneReady?: (pane: pane_api) => void;
  onSeriesReady?: (key: string, series: general_series_api) => void;
}

interface axis_runtime {
  handle: general_axis_api;
  structure_signature: string;
  presentation_signature: string;
}

interface series_runtime {
  handle: general_series_api;
  binding_signature: string;
  presentation_signature: string;
  data: GeneralSeriesSpec["data"];
}

interface pane_runtime {
  pane: pane_api;
  axes: Map<string, axis_runtime>;
  series: Map<string, series_runtime>;
}

function stable_signature(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => item instanceof Date ? item.getTime() : item);
}

function general_series_bindings(options: GeneralSeriesSpec["options"]): object {
  return { x_axis_id: options.x_axis_id, y_axis_id: options.y_axis_id };
}

function general_axis_structure(axis: GeneralAxisSpec): object {
  return { dimension: axis.dimension, scale: axis.scale };
}

function general_axis_presentation(
  axis: GeneralAxisSpec,
): Omit<GeneralAxisSpec, "id" | "dimension" | "scale"> {
  const { id, dimension, scale, ...presentation } = axis;
  void id;
  void dimension;
  void scale;
  return presentation;
}

function general_series_presentation(
  options: GeneralSeriesSpec["options"],
): Omit<GeneralSeriesSpec["options"], "x_axis_id" | "y_axis_id"> {
  const { x_axis_id, y_axis_id, ...presentation } = options;
  void x_axis_id;
  void y_axis_id;
  return presentation;
}

function dispose_pane_runtime(chart: chart_api, runtime: pane_runtime): void {
  try {
    chart.backend();
  } catch {
    // React may dispose the parent chart before child effect cleanup under Strict Mode/unmount.
    return;
  }
  for (const item of runtime.series.values()) {
    item.handle.remove();
  }
  runtime.series.clear();
  for (const item of Array.from(runtime.axes.values()).reverse()) {
    if (!item.handle.remove()) throw new Error(`general axis ${item.handle.id} could not be removed during cleanup`);
  }
  runtime.axes.clear();
  const removed = chart.remove_pane(runtime.pane.pane_index());
  if (!removed) throw new Error("general pane could not be removed during cleanup");
}

/**
 * Own one general-domain pane, its axes, and its general series. Axes and series are reconciled by
 * stable IDs/keys; unchanged series keep their engine identity and a changed data reference calls
 * `set_data` on that same handle. Compatible series binding changes retain handles; structural axis
 * or kind changes replace only the affected general series, never the chart.
 */
export function GeneralPane({ options, axes, series, onPaneReady, onSeriesReady }: GeneralPaneProps) {
  const chart = useAerisChart();
  const runtime_ref = useRef<pane_runtime | null>(null);
  const ready_ref = useRef(onPaneReady);
  const series_ready_ref = useRef(onSeriesReady);
  const [generation, set_generation] = useState(0);
  const domain_signature = stable_signature(options.horizontal_domain);
  ready_ref.current = onPaneReady;
  series_ready_ref.current = onSeriesReady;

  const axis_ids = new Set<string>();
  for (const axis of axes) {
    if (axis_ids.has(axis.id)) throw new Error(`duplicate general axis id: ${axis.id}`);
    axis_ids.add(axis.id);
  }
  const series_keys = new Set<string>();
  for (const item of series) {
    if (series_keys.has(item.key)) throw new Error(`duplicate general series key: ${item.key}`);
    series_keys.add(item.key);
  }

  useEffect(() => {
    const pane = chart.addPane({ preserve_empty: true, horizontal_domain: options.horizontal_domain });
    const runtime: pane_runtime = { pane, axes: new Map(), series: new Map() };
    runtime_ref.current = runtime;
    try {
      ready_ref.current?.(pane);
    } catch (error) {
      runtime_ref.current = null;
      dispose_pane_runtime(chart, runtime);
      throw error;
    }
    set_generation((value) => value + 1);
    return () => {
      if (runtime_ref.current === runtime) runtime_ref.current = null;
      dispose_pane_runtime(chart, runtime);
    };
    // A horizontal-domain change is structural and intentionally replaces this one pane.
  }, [chart, domain_signature]);

  useEffect(() => {
    const runtime = runtime_ref.current;
    if (runtime === null) return;
    const pane = runtime.pane.pane_index();
    const desired_axes = new Map(axes.map((axis) => [axis.id, axis] as const));
    const desired_series = new Map(series.map((item) => [item.key, item] as const));
    const changed_axes = new Set<string>();

    for (const [id, current] of runtime.axes) {
      const desired = desired_axes.get(id);
      if (
        desired === undefined
        || stable_signature(general_axis_structure(desired)) !== current.structure_signature
      ) {
        changed_axes.add(id);
      } else {
        const presentation = general_axis_presentation(desired);
        const presentation_signature = stable_signature(presentation);
        if (presentation_signature !== current.presentation_signature) {
          current.handle.applyOptions(presentation);
          current.presentation_signature = presentation_signature;
        }
      }
    }

    for (const [key, current] of runtime.series) {
      const desired = desired_series.get(key);
      const depends_on_changed_axis = desired !== undefined
        && (changed_axes.has(desired.options.x_axis_id) || changed_axes.has(desired.options.y_axis_id));
      const changed = desired === undefined
        || desired.kind !== current.handle.kind
        || depends_on_changed_axis;
      if (changed) {
        current.handle.remove();
        runtime.series.delete(key);
      } else {
        const binding_signature = stable_signature(general_series_bindings(desired.options));
        const presentation = general_series_presentation(desired.options);
        const presentation_signature = stable_signature(presentation);
        if (
          binding_signature !== current.binding_signature
          || presentation_signature !== current.presentation_signature
        ) {
          current.handle.applyOptions({
            ...presentation,
            pane,
            x_axis_id: desired.options.x_axis_id,
            y_axis_id: desired.options.y_axis_id,
          });
          current.binding_signature = binding_signature;
          current.presentation_signature = presentation_signature;
        }
      }
    }

    for (const id of changed_axes) {
      const current = runtime.axes.get(id);
      if (current !== undefined) {
        if (!current.handle.remove()) throw new Error(`general axis ${id} could not be reconciled`);
        runtime.axes.delete(id);
      }
    }

    for (const axis of axes) {
      if (runtime.axes.has(axis.id)) continue;
      const handle = chart.addAxis({ ...axis, pane });
      runtime.axes.set(axis.id, {
        handle,
        structure_signature: stable_signature(general_axis_structure(axis)),
        presentation_signature: stable_signature(general_axis_presentation(axis)),
      });
    }

    for (const desired of series) {
      let current = runtime.series.get(desired.key);
      if (current === undefined) {
        const handle = chart.addSeries(desired.kind, { ...desired.options, pane });
        try {
          handle.setData(desired.data);
          current = {
            handle,
            binding_signature: stable_signature(general_series_bindings(desired.options)),
            presentation_signature: stable_signature(general_series_presentation(desired.options)),
            data: desired.data,
          };
          runtime.series.set(desired.key, current);
          series_ready_ref.current?.(desired.key, handle);
        } catch (error) {
          runtime.series.delete(desired.key);
          handle.remove();
          throw error;
        }
      } else if (current.data !== desired.data) {
        current.handle.setData(desired.data);
        current.data = desired.data;
      }
    }

    const desired_order = series
      .map((item) => runtime.series.get(item.key)?.handle)
      .filter((handle): handle is general_series_api => handle !== undefined);
    const current_order = chart.general_series_order(pane);
    if (
      desired_order.length === current_order.length
      && desired_order.some((handle, index) => handle !== current_order[index])
    ) {
      chart.set_general_series_order(desired_order, pane);
    }
  }, [chart, generation, axes, series]);

  return null;
}
