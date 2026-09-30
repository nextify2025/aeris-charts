/**
 * Handle implementations over the wasm engine: series, time scale, price scale, pane, chart.
 * Extracted from `index.ts`.
 */

// @ts-ignore -- pkg is a build artifact, present after build:wasm
import init, {
  AerisChart, resample_boundaries as wasm_resample_boundaries, session_slot_times as wasm_session_slot_times,
} from "../pkg/aeris_charts_wasm.js";

import { install_gestures } from "./gestures.js";
import type { pane_primitive, pane_primitive_handle, series_primitive, series_primitive_handle } from "./primitives.js";
import type { canvas_primitive, canvas_primitive_handle, canvas_pane_view } from "./canvas_plugins.js";
import { create_canvas_render_target } from "./canvas_plugins.js";
import type { custom_series_item, custom_series_pane_view } from "./custom_series.js";
import {
  enable_accessibility,
  type accessibility_handle,
  type accessibility_options,
} from "./accessibility.js";
import { AerisChartsError } from "./errors.js";
import type { AerisChartsErrorCode } from "./errors.js";
import type {
  alert_api, alert_condition, alert_frequency, alert_line, alert_price_scale, alert_snapshot,
  autoscale_info,
  crosshair_action_request, crosshair_action_request_handler,
  any_series_options, backend_status, bars_info, chart_api, chart_context_handler, chart_context_params, chart_options, chart_state, chart_value_snapshot, comparison_legend_entry, data_changed_handler, dbl_click_handler,
  deep_partial, drawing_api, drawing_created_handler, drawing_info, drawing_kind, drawing_options,
  depth_event_columns, depth_event_layer_options, depth_heatmap_options, depth_ladder_row, depth_options, depth_snapshot_columns, depth_study_snapshot, depth_update_columns,
  drawing_point, drawing_point_input, drawing_points_update, drawing_price_segment, drawing_magnet_mode,
  drawing_tool_change_handler, drawing_interval, drawing_property_schema, drawing_kind_options, drawing_template,
  ema_ribbon_options, ema_ribbon_periods,
  feature_series_kind, frame_stats,
  footprint_bar, footprint_series_api, footprint_series_options, footprint_trade, footprint_trade_columns,
  general_accessibility_snapshot, general_axis_api, general_axis_options, general_axis_presentation_options, general_brush_snapshot, general_legend_snapshot, general_pane_options, general_reference_api, general_reference_options, general_reference_value, general_series_api, general_series_hit,
  general_series_kind, general_series_options, general_shared_tooltip_snapshot, general_tooltip_snapshot, general_update_options, general_xy_row,
  box_plot_row, bubble_columns, bubble_row, category_box_columns, category_error_columns, category_heatmap_columns, category_range_columns, category_xy_columns, numeric_error_columns, numeric_heatmap_columns, numeric_range_columns, numeric_xy_columns,
  error_bar_row, heatmap_grid_row, range_area_row, temporal_error_columns, temporal_heatmap_columns, temporal_range_columns, temporal_xy_columns,
  ingestion_diagnostics,
  handle_scale_options, handle_scroll_options, indicator_info, indicator_input_source, indicator_kind, indicator_output_style, indicator_schema, kinetic_scroll_options, pivot_kind, vwap_reset,
  bollinger_parameters, indicator_convention, indicator_seed_parameters, macd_parameters, vwap_parameters,
  last_value_data, localization_options, logical_range,
  mismatch_direction, mouse_event_handler, mouse_event_params, ohlc_columns, ohlc_data, options_change_handler, pane_api, pane_geometry, price_line_api, price_line_options,
  persistence_restore_result, price_range, price_scale_api, price_scale_create_options,
  price_scale_info, price_scale_options, ring_source_layout,
  series_api, series_change_handler, series_data, series_kind, series_merge_columns, series_merge_data, series_update_options,
  series_marker, series_marker_options, series_options, single_value_data, size_change_handler, time, time_range,
  replay_clock_stats, replay_seek_stats, synthetic_bar_options, trade_stream_stats,
  time_and_sales_options, time_and_sales_row, time_scale_api, time_scale_options, tracking_mode_options, trading_api, trading_execution, trading_hit,
  chart_sync_event, crosshair_sync_position,
  trading_intent, trading_intent_handler, trading_position, trading_preview, trading_snapshot,
  trading_style_options, instrument_metadata, working_order, host_overlay_snapshot, host_event_hit,
  visible_logical_range_handler, visible_time_range_handler,
  volume_profile_indicator_api, volume_profile_indicator_options, volume_profile_indicator_snapshot,
  time_label_context, time_zone,
  histogram_updown_rule, kdj_parameters, session_slot_options, time_tick_mark, time_alignment,
  resample_boundary, resample_boundary_options, resample_series_options, resample_stats, resampled_bar,
  trade_session_options, business_day,
} from "./types.js";
import {
  DRAWING_KIND_TO_U8, FEATURE_KIND_TO_U8, KIND_TO_U8, LINE_STYLE_TO_U8, LINE_TYPE_TO_U8,
  is_feature_series_kind, is_footprint_series_kind,
} from "./types.js";
import { default_theme_name, theme_options, theme_palette, type theme_name } from "./theme.js";
import {
  exchange_time_json, format_exchange_seconds, resolve_time_zone, split_exchange_time_options,
  time_label_context_for, type engine_time_tick_marks,
} from "./time_zone.js";

// ---------------------------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------------------------

let init_promise: Promise<unknown> | null = null;

// Wire ids are sparse (reserved per drawing family), so the reverse map derives from the single
// DRAWING_KIND_TO_U8 list instead of an index-ordered array.
const DRAWING_KIND_FROM_U8: ReadonlyMap<number, drawing_kind> = new Map(
  (Object.entries(DRAWING_KIND_TO_U8) as [drawing_kind, number][]).map(([kind, wire]) => [wire, kind]),
);

type persistence_error_result = {
  ok: false;
  error: { code: AerisChartsErrorCode; message: string };
};

function throw_persistence_error(result: persistence_error_result): never {
  throw new AerisChartsError(result.error.code, result.error.message);
}

/** Parse an engine `{ok, ...}` / `{ok:false, error}` drawing result envelope, throwing on error. */
function drawing_result<T extends object>(json: string): T {
  const result = JSON.parse(json) as ({ ok: true } & T) | persistence_error_result;
  if (!result.ok) throw_persistence_error(result);
  return result;
}

const DRAWING_MAGNET_TO_U8: Record<drawing_magnet_mode, number> = { off: 0, weak: 1, strong: 2 };
const DRAWING_MAGNET_FROM_U8: readonly drawing_magnet_mode[] = ["off", "weak", "strong"];
/**
 * Instantiate the wasm module once per page. `wasm_url` overrides the default asset resolution
 * (`new URL("aeris_charts_wasm_bg.wasm", import.meta.url)` beside the bundle) — the escape hatch for
 * bundlers that relocate the JS away from the .wasm (e.g. Vite's dev pre-bundler). Only the
 * first call's argument takes effect.
 */
export function ensure_init(wasm_url?: string | URL): Promise<unknown> {
  if (init_promise === null) {
    const initializing = (wasm_url !== undefined ? init(wasm_url) : init()).then((module: unknown) => {
      wasm_ready = true;
      return module;
    });
    init_promise = initializing;
    return initializing;
  }
  return init_promise;
}

/** Set once the engine module is instantiated, for synchronous engine helpers. */
let wasm_ready = false;

/**
 * UTC seconds of every bar slot of one trading date's session windows, for reserving whitespace
 * slots before an intraday (分时) session trades. The engine owns the calculation (validation,
 * exchange offsets across DST, windows crossing midnight, at most 100 000 slots); the time zone
 * is an IANA name resolved here or an explicit schedule. Needs the engine module, so call it after
 * `init_wasm()` or `create_chart()` resolved. Invalid input throws `invalid_options`.
 */
export function session_slot_times(options: session_slot_options): number[] {
  if (!wasm_ready) {
    throw new AerisChartsError(
      "unsupported_operation",
      "session_slot_times needs the engine: await init_wasm() or create_chart() first",
    );
  }
  const date = options.date;
  const text = typeof date === "string"
    ? date
    : date !== null && typeof date === "object"
      ? `${String(date.year).padStart(4, "0")}-${String(date.month).padStart(2, "0")}-${String(date.day).padStart(2, "0")}`
      : "";
  const request = JSON.stringify({
    date: text,
    windows: options.windows,
    interval_seconds: options.interval_seconds,
    time_zone: resolve_time_zone(options.time_zone ?? "UTC"),
    session_start: options.session_start ?? 0,
    convention: options.convention ?? "bar_open",
  });
  try {
    return Array.from(wasm_session_slot_times(request));
  } catch (error) {
    throw new AerisChartsError("invalid_options", `invalid session: ${String(error)}`);
  }
}

function trading_date_text(date: string | business_day): string {
  return typeof date === "string"
    ? date
    : date !== null && typeof date === "object"
      ? `${String(date.year).padStart(4, "0")}-${String(date.month).padStart(2, "0")}-${String(date.day).padStart(2, "0")}`
      : "";
}

/**
 * Resampling boundaries for the host's trading dates: each date's exchange-local session windows
 * placed in the exchange time zone (offset in force on that date, so DST is respected), one
 * boundary per window (`span: "window"`, intraday bars restart at every window open) or per date
 * (`span: "day"`, daily bars). Dates without data produce no bars, so a live host may include the
 * dates it is about to stream. Needs the engine module, like {@link session_slot_times}. Invalid
 * input throws `invalid_options`.
 */
export function resample_boundaries(options: resample_boundary_options): resample_boundary[] {
  if (!wasm_ready) {
    throw new AerisChartsError(
      "unsupported_operation",
      "resample_boundaries needs the engine: await init_wasm() or create_chart() first",
    );
  }
  const request = JSON.stringify({
    dates: Array.from(options.dates ?? [], trading_date_text),
    windows: options.windows,
    time_zone: resolve_time_zone(options.time_zone ?? "UTC"),
    session_start: options.session_start ?? 0,
    span: options.span ?? "window",
  });
  let flat: Float64Array;
  try {
    flat = wasm_resample_boundaries(request);
  } catch (error) {
    throw new AerisChartsError("invalid_options", `invalid resample sessions: ${String(error)}`);
  }
  const boundaries: resample_boundary[] = [];
  for (let offset = 0; offset + 2 < flat.length; offset += 3) {
    boundaries.push({
      start_time: flat[offset] ?? 0,
      end_time: flat[offset + 1] ?? 0,
      session_id: flat[offset + 2] ?? 0,
    });
  }
  return boundaries;
}

/**
 * Convert public explicit time-axis marks to the engine form (UTC seconds). The engine validates
 * order, count, and label length. `null` restores automatic ticks.
 */
export function normalize_time_tick_marks(marks: unknown): engine_time_tick_marks {
  if (marks === null) return null;
  if (!Array.isArray(marks)) {
    throw new AerisChartsError("invalid_options", "tick_marks must be an array of { time, label? } or null");
  }
  return (marks as (time_tick_mark | null)[]).map((mark, index) => {
    const time = mark === null || typeof mark !== "object" ? NaN : time_to_utc_seconds(mark.time);
    const label = mark?.label;
    if (!Number.isFinite(time) || (label !== undefined && typeof label !== "string")) {
      throw new AerisChartsError("invalid_options", `tick mark ${index} needs a valid time and an optional string label`);
    }
    return label === undefined ? { time } : { time, label };
  });
}

/** Accepted `histogram_updown_rule` values. */
const HISTOGRAM_UPDOWN_RULES: readonly histogram_updown_rule[] = ["open_close", "previous_close"];

/** Accepted `time_alignment` values. */
const TIME_ALIGNMENTS: readonly time_alignment[] = ["union", "as_of"];

/**
 * The pure value checks of `time_alignment` / `as_of_max_staleness`, run before anything touches
 * the engine: the enum, a non-negative whole number of seconds (or null), and no staleness bound
 * on a union series. `current` is the series' alignment when the call omits `time_alignment`
 * (`"union"` for a series about to be created). Throws `invalid_options`.
 */
export function validate_series_time_alignment(
  options: Partial<Pick<series_options, "time_alignment" | "as_of_max_staleness">>,
  current: time_alignment = "union",
): void {
  const { time_alignment, as_of_max_staleness } = options;
  if (time_alignment !== undefined && !TIME_ALIGNMENTS.includes(time_alignment)) {
    throw new AerisChartsError("invalid_options", `time_alignment must be "union" or "as_of"`);
  }
  if (as_of_max_staleness !== undefined && as_of_max_staleness !== null
    && !(Number.isSafeInteger(as_of_max_staleness) && as_of_max_staleness >= 0)) {
    throw new AerisChartsError(
      "invalid_options",
      "as_of_max_staleness must be a non-negative whole number of seconds or null",
    );
  }
  if ((time_alignment ?? current) === "union" && as_of_max_staleness !== undefined && as_of_max_staleness !== null) {
    throw new AerisChartsError("invalid_options", `as_of_max_staleness requires time_alignment "as_of"`);
  }
}

/**
 * Validate and apply `time_alignment` / `as_of_max_staleness` (shared by main-thread and worker
 * charts). An omitted key keeps the current value, except that switching to `"union"` clears the
 * staleness bound. Throws `invalid_options` for a bad value (before touching the series) and the
 * engine's `unsupported_operation` for a series without an own calendar. Returns whether the
 * alignment or staleness changed; a request equal to the current values is a no-op.
 */
export function apply_series_time_alignment(
  wasm: AerisChart,
  id: number,
  options: Partial<Pick<series_options, "time_alignment" | "as_of_max_staleness">>,
): boolean {
  const { time_alignment, as_of_max_staleness } = options;
  if (time_alignment === undefined && as_of_max_staleness === undefined) return false;
  const current = JSON.parse(wasm.series_options_json(id)) as Partial<series_options>;
  const current_alignment = current.time_alignment ?? "union";
  validate_series_time_alignment(options, current_alignment);
  const alignment = time_alignment ?? current_alignment;
  const staleness = as_of_max_staleness !== undefined
    ? as_of_max_staleness
    : alignment === "as_of" ? current.as_of_max_staleness ?? null : null;
  if (alignment === current_alignment && staleness === (current.as_of_max_staleness ?? null)) return false;
  assert_trading_result(wasm.set_series_time_alignment(id, alignment === "as_of", staleness ?? undefined));
  return true;
}

/**
 * Throw unless `id` names a live series of the chart behind `wasm`; worker charts address series
 * by raw numeric id. A number that is not a `u32` is refused with `invalid_handle` before it
 * reaches the engine, because the wasm boundary would wrap it onto a real series (`NaN` and `2**32`
 * to 0, `1.5` to 1). A dead id gets the engine's own answer, `stale_handle` for a removed series
 * and `invalid_handle` for one never issued.
 */
export function assert_live_series(wasm: AerisChart, id: number): void {
  if (!Number.isInteger(id) || id < 0 || id > 0xffffffff) {
    throw new AerisChartsError("invalid_handle", `series id ${id} is not a valid series handle`);
  }
  if (wasm.series_kind(id) !== undefined) return;
  // `set_series_time_alignment` validates the id before it reads anything else, so this union
  // request on a dead id changes nothing and reports why the id is dead.
  assert_trading_result(wasm.set_series_time_alignment(id, false, undefined));
  throw new AerisChartsError("invalid_handle", `series ${id} does not exist`);
}

function undef_to_null<T>(v: T | undefined): T | null {
  return v === undefined ? null : v;
}

function same_logical_range(a: logical_range | null, b: logical_range | null): boolean {
  return a === b || (a !== null && b !== null && a.from === b.from && a.to === b.to);
}

function same_time_range(a: time_range | null, b: time_range | null): boolean {
  return a === b || (a !== null && b !== null && a.from === b.from && a.to === b.to);
}

/** Duration of the animated `scroll_to_position` ease (matches the reference smooth-scroll feel). */
const SCROLL_ANIM_MS = 300;
const EMPTY_UINT32 = new Uint32Array();
/** reference `scrollToRealTime` duration (`DefaultAnimationDuration`, time-scale.ts:31). */
const REAL_TIME_SCROLL_ANIM_MS = 400;
/** Synchronous re-reads after handlers mutate the view; later changes wait for the next frame. */
const MAX_VISIBLE_RANGE_EMIT_PASSES = 8;

/**
 * Whether the candle-close countdown timer should run: any series with `countdown_visible`
 * and data. Factored pure so the start/stop logic is testable without a chart.
 */
function countdown_timer_needed(
  series: readonly { countdown_visible?: boolean; has_data: boolean }[],
): boolean {
  return series.some((s) => s.countdown_visible === true && s.has_data);
}

/**
 * Series style keys without a dedicated wasm setter, forwarded to `series_apply_options_json`
 * as one snake_case JSON patch (the engine ignores unknown keys).
 */
const SERIES_JSON_OPTION_KEYS = [
  "last_value_visible",
  "title",
  "title_visible",
  "countdown_visible",
  "price_line_visible",
  "price_line_source",
  "price_line_extent",
  "price_line_width",
  "price_line_color",
  "price_line_style",
  "bid_ask_visible",
  "bid_color",
  "ask_color",
  "bid_ask_line_width",
  "bid_ask_line_style",
  "line_style",
  "line_visible",
  "point_markers_radius",
  "crosshair_marker_visible",
  "crosshair_marker_radius",
  "crosshair_marker_border_color",
  "crosshair_marker_background_color",
  "crosshair_marker_border_width",
  "top_fill_color1",
  "top_fill_color2",
  "top_line_color",
  "top_line_width",
  "top_line_style",
  "bottom_fill_color1",
  "bottom_fill_color2",
  "bottom_line_color",
  "bottom_line_width",
  "bottom_line_style",
  "base",
  "invert_filled_area",
  "break_on_trading_day",
  "open_visible",
  "close_visible",
  "thin_bars",
  "heikin_ashi",
  "histogram_updown_rule",
] as const;

/** Reject malformed Aeris price-scale extension values before any option is applied. */
function validate_price_scale_options(options: deep_partial<price_scale_options>): void {
  const { tick_mark_density, base_value, autoscale_center } = options;
  if (tick_mark_density !== undefined && !(Number.isFinite(tick_mark_density) && tick_mark_density > 0)) {
    throw new AerisChartsError("invalid_options", "tick_mark_density must be a finite positive number");
  }
  if (base_value !== undefined && base_value !== null && !(Number.isFinite(base_value) && base_value !== 0)) {
    throw new AerisChartsError("invalid_options", "base_value must be a finite non-zero price or null");
  }
  if (autoscale_center !== undefined && autoscale_center !== null && !Number.isFinite(autoscale_center)) {
    throw new AerisChartsError("invalid_options", "autoscale_center must be a finite price or null");
  }
  for (const key of ["ensure_edge_tick_marks_visible", "stable_auto_scale"] as const) {
    const value = options[key];
    if (value !== undefined && typeof value !== "boolean") {
      throw new AerisChartsError("invalid_options", `${key} must be a boolean`);
    }
  }
}

/**
 * Price-scale style keys without a dedicated wasm setter, forwarded to
 * `price_scale_apply_options_json` as one snake_case JSON patch (the engine ignores unknown keys).
 */
const PRICE_SCALE_JSON_OPTION_KEYS = [
  "align_labels",
  "ticks_visible",
  "entire_text_only",
  "minimum_width",
  "text_color",
  "bold_round_labels",
  "tick_mark_density",
  "ensure_edge_tick_marks_visible",
  "base_value",
  "autoscale_center",
  "stable_auto_scale",
  "visible",
] as const;

/** Engine kind ordinal → public kind name (index-aligned with `KIND_TO_U8`). */
const KIND_NAMES = ["candlestick", "bar", "line", "area", "histogram", "baseline", "custom", undefined, "footprint"] as const;
const FEATURE_KIND_NAMES = [
  undefined,
  undefined,
  "grouped_bars",
  "heatmap",
  "hlc_area",
  "pretty_histogram",
  undefined,
  "background_shade",
  "stacked_area",
  "stacked_bars",
  "whisker_box",
] as const satisfies readonly (feature_series_kind | undefined)[];

/**
 * Slot layout of the `frame_stats_into` f64 buffer. Must match `crate::telemetry::slot` in
 * `aeris_charts_wasm` exactly — append only, never reorder (the engine and the package version
 * together, but a stale bundle against a newer .wasm must still read the same slots).
 */
const FRAME_STATS_SLOT = {
  cpu_ms: 0,
  gpu_ms: 1,
  draw_calls: 2,
  dropped_frames: 3,
  presented_frames: 4,
  memory_bytes: 5,
  canvas2d_ops: 6,
  ring_overruns: 7,
  gpu_buffer_allocations: 8,
  gpu_write_calls: 9,
  gpu_uploaded_bytes: 10,
  layout_rebuilds: 11,
  autoscale_runs: 12,
  series_rebuilds: 13,
  drawing_rebuilds: 14,
  grid_rebuilds: 15,
  overlay_rebuilds: 16,
  axis_rebuilds: 17,
  text_resolutions: 18,
  trading_rebuilds: 19,
  ring_dropped_rows: 20,
} as const;

/**
 * Convert a `time` input to the engine's UTC-seconds form. Business days and strict `"YYYY-MM-DD"`
 * strings are taken at UTC midnight. A malformed or normalized date yields `NaN`, which causes the
 * ingestion transaction to be rejected.
 */
export function time_to_utc_seconds(t: time): number {
  if (typeof t === "number") return t;
  let year: number;
  let month: number;
  let day: number;
  if (typeof t === "string") {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(t);
    if (match === null) return NaN;
    year = Number(match[1]);
    month = Number(match[2]);
    day = Number(match[3]);
  } else {
    if (t === null || typeof t !== "object") return NaN;
    ({ year, month, day } = t);
  }
  if (![year, month, day].every(Number.isInteger)
    || year < 0 || year > 9999 || month < 1 || month > 12 || day < 1 || day > 31) return NaN;

  // Date.UTC remaps years 0..99 to 1900..1999. setUTCFullYear applies the astronomical year
  // directly, then the round-trip check rejects rollover such as 2024-02-31.
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1 || date.getUTCDate() !== day) return NaN;
  return date.getTime() / 1_000;
}

/**
 * Input form of a replacement batch: `true` when every row carries a calendar date
 * (`business_day` or `"YYYY-MM-DD"`), `false` when any row is a numeric instant, `null` for an
 * empty batch (the series no longer constrains the chart's calendar-date axis).
 */
function calendar_input(data: readonly { time: time }[]): boolean | null {
  if (data.length === 0) return null;
  return data.every((item) => typeof item.time !== "number");
}

const MIN_TIMESTAMP_SECONDS = -62_167_219_200;
const MAX_TIMESTAMP_SECONDS = 253_402_300_799;

function timestamp_rejection_reason(time: number): string | null {
  const expected = `expected a finite whole number of UTC seconds in the inclusive range ${MIN_TIMESTAMP_SECONDS}..${MAX_TIMESTAMP_SECONDS}`;
  if (!Number.isFinite(time)) return `invalid timestamp: ${expected}; received a non-finite value`;
  if (!Number.isInteger(time)) return `invalid timestamp: ${expected}; received fractional seconds`;
  if (time >= MIN_TIMESTAMP_SECONDS && time <= MAX_TIMESTAMP_SECONDS) return null;
  const likely = [
    [1_000, 1_000_000_000_000, "milliseconds"],
    [1_000_000, 1_000_000_000_000_000, "microseconds"],
    [1_000_000_000, 1_000_000_000_000_000_000, "nanoseconds"],
  ] as const;
  const unit = likely.find(([scale, minimum_magnitude]) => {
    const seconds = time / scale;
    return Math.abs(time) >= minimum_magnitude
      && seconds >= MIN_TIMESTAMP_SECONDS && seconds <= MAX_TIMESTAMP_SECONDS;
  })?.[2];
  const hint = unit === undefined
    ? ""
    : `; the value appears to be ${unit}, convert it to UTC seconds before ingestion (timestamps are not auto-converted)`;
  return `invalid timestamp: ${expected}; received a value outside the supported range${hint}`;
}

function pack(data: readonly series_data[]): {
  times: Float64Array;
  open: Float64Array;
  high: Float64Array;
  low: Float64Array;
  close: Float64Array;
  body_colors?: Uint32Array;
  wick_colors?: Uint32Array;
  border_colors?: Uint32Array;
} {
  const n = data.length;
  const times = new Float64Array(n);
  const open = new Float64Array(n);
  const high = new Float64Array(n);
  const low = new Float64Array(n);
  const close = new Float64Array(n);
  // Per-point color channels (reference `color`/`wickColor`/`borderColor` on data items). A channel is
  // only allocated when at least one item carries the field; rows without it pad as 0, which the
  // engine treats as "no override" within a passed channel.
  let body_colors: Uint32Array | undefined;
  let wick_colors: Uint32Array | undefined;
  let border_colors: Uint32Array | undefined;
  for (let i = 0; i < n; i++) {
    const d = data[i] as series_data;
    times[i] = time_to_utc_seconds(d.time);
    if ("value" in d) {
      open[i] = high[i] = low[i] = close[i] = d.value;
      body_colors = pack_color_channel(body_colors, n, i, "color" in d ? d.color : undefined);
    } else if ("open" in d) {
      open[i] = d.open;
      high[i] = d.high;
      low[i] = d.low;
      close[i] = d.close;
      body_colors = pack_color_channel(body_colors, n, i, "color" in d ? d.color : undefined);
      wick_colors = pack_color_channel(wick_colors, n, i, "wick_color" in d ? d.wick_color : undefined);
      border_colors = pack_color_channel(border_colors, n, i, "border_color" in d ? d.border_color : undefined);
    } else {
      // Whitespace (reference `WhitespaceData`): an explicit empty slot, packed all-NaN. The engine
      // keeps the row as whitespace instead of dropping it.
      open[i] = high[i] = low[i] = close[i] = NaN;
    }
  }
  return { times, open, high, low, close, body_colors, wick_colors, border_colors };
}

/** Parse a CSS hex or rgb()/rgba() color to 8-bit RGBA channels (mirrors the Rust `Color::parse_css`). */
function parse_rgba(css: string): [number, number, number, number] | null {
  const s = css.trim();
  if (s.startsWith("#")) {
    const h = s.slice(1);
    const expand = (c: string) => parseInt(c + c, 16);
    if (h.length === 3 || h.length === 4) {
      return [expand(h[0]!), expand(h[1]!), expand(h[2]!), h.length === 4 ? expand(h[3]!) : 255];
    }
    if (h.length === 6 || h.length === 8) {
      return [
        parseInt(h.slice(0, 2), 16),
        parseInt(h.slice(2, 4), 16),
        parseInt(h.slice(4, 6), 16),
        h.length === 8 ? parseInt(h.slice(6, 8), 16) : 255,
      ];
    }
    return null;
  }
  const m = s.match(/^rgba?\(([^)]+)\)$/i);
  if (m) {
    const parts = m[1]!.split(",").map((p) => parseFloat(p.trim()));
    if (parts.length >= 3 && parts.every((p) => !Number.isNaN(p))) {
      const alpha = parts.length >= 4 ? Math.round(parts[3]! * 255) : 255;
      return [Math.round(parts[0]!), Math.round(parts[1]!), Math.round(parts[2]!), alpha];
    }
  }
  return null;
}

/** Parse a CSS hex or rgb()/rgba() color to 8-bit channels (mirrors the Rust `Color::parse_css`). */
function parse_rgb(css: string): [number, number, number] | null {
  const rgba = parse_rgba(css);
  return rgba === null || rgba.slice(0, 3).some(Number.isNaN) ? null : [rgba[0], rgba[1], rgba[2]];
}

/**
 * Parse a CSS color to the engine's packed per-point color word, 0xRRGGBBAA (alpha preserved).
 * Returns `null` for unparseable input; callers warn and skip that item's color, matching how
 * the engine sanitizer warns on bad data.
 */
function parse_css_to_u32(css: string): number | null {
  const rgba = parse_rgba(css);
  if (rgba === null || rgba.some(Number.isNaN)) return null;
  return ((rgba[0] << 24) | (rgba[1] << 16) | (rgba[2] << 8) | rgba[3]) >>> 0;
}

/**
 * Fold one data item's optional per-point color field into a channel array, allocating it lazily
 * on first use. The engine treats a channel as present only when the array is passed, and every
 * row of a passed channel as a custom color — so rows without the field pad as 0, which the
 * engine reads as "no override" (0x00000000 would otherwise be a valid transparent-black).
 * Unparseable colors warn and pad as 0, matching the engine sanitizer's warn-and-skip.
 */
function pack_color_channel(
  channel: Uint32Array | undefined,
  n: number,
  i: number,
  css: string | undefined,
): Uint32Array | undefined {
  const packed = point_color_to_u32(css);
  if (packed === undefined) return channel;
  const out = channel ?? new Uint32Array(n);
  out[i] = packed;
  return out;
}

/** Parse an optional per-point color field to 0xRRGGBBAA; `undefined` = no custom color. */
function point_color_to_u32(css: string | undefined): number | undefined {
  if (css === undefined) return undefined;
  const packed = parse_css_to_u32(css);
  if (packed === null) {
    console.warn(`aeris_charts: ignoring unparseable data point color "${css}"`);
    return undefined;
  }
  return packed;
}

type general_engine_result<T> =
  | { ok: true; result: T }
  | { ok: false; error: { code: AerisChartsErrorCode; message: string } };

function parse_general_result<T>(json: string): T {
  const result = JSON.parse(json) as general_engine_result<T>;
  if (!result.ok) throw new AerisChartsError(result.error.code, result.error.message);
  return result.result;
}

type packed_numeric_xy_columns = Omit<numeric_xy_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_bubble_columns = Omit<bubble_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_numeric_range_columns = Omit<numeric_range_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_numeric_error_columns = Omit<numeric_error_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_category_xy_columns = Omit<category_xy_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_temporal_xy_columns = Omit<temporal_xy_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_temporal_range_columns = Omit<temporal_range_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_temporal_error_columns = Omit<temporal_error_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_category_range_columns = Omit<category_range_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_category_error_columns = Omit<category_error_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_category_box_columns = Omit<category_box_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_category_heatmap_columns = Omit<category_heatmap_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_numeric_heatmap_columns = Omit<numeric_heatmap_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type packed_temporal_heatmap_columns = Omit<temporal_heatmap_columns, "ids"> & {
  ids?: readonly (string | number | null)[];
};
type general_columns_input =
  | packed_numeric_xy_columns
  | packed_bubble_columns
  | packed_numeric_range_columns
  | packed_numeric_error_columns
  | packed_temporal_range_columns
  | packed_temporal_error_columns
  | packed_category_range_columns
  | packed_category_error_columns
  | packed_category_box_columns
  | packed_category_heatmap_columns
  | packed_numeric_heatmap_columns
  | packed_temporal_heatmap_columns
  | packed_temporal_xy_columns
  | packed_category_xy_columns;

function general_ids_json(ids: readonly (string | number | null)[] | undefined, rows: number): string {
  if (ids === undefined) return "";
  if (ids.length !== rows) {
    throw new AerisChartsError("invalid_data", "general row IDs and value columns must have equal lengths");
  }
  return JSON.stringify(ids);
}

function general_labels_value(labels: readonly (string | null)[] | undefined, rows: number): readonly (string | null)[] | null {
  if (labels === undefined) return null;
  if (labels.length !== rows) {
    throw new AerisChartsError("invalid_data", "general row labels and value columns must have equal lengths");
  }
  return labels;
}

function general_value_metadata_json(
  columns: { ids?: readonly (string | number | null)[]; labels?: readonly (string | null)[] },
  rows: number,
): string {
  const ids = general_ids_json(columns.ids, rows) || "null";
  const labels = JSON.stringify(general_labels_value(columns.labels, rows));
  return `{"ids":${ids},"labels":${labels}}`;
}

function pack_general_rows(
  kind: general_series_kind,
  data: readonly (general_xy_row | bubble_row | range_area_row | error_bar_row | box_plot_row | heatmap_grid_row)[],
  x_scale?: general_axis_options["scale"],
): general_columns_input {
  const has_explicit = data.some((row) => row.id !== undefined);
  const ids = has_explicit ? data.map((row) => row.id ?? null) : undefined;
  const has_labels = data.some((row) => row.label !== undefined);
  const labels = has_labels ? data.map((row) => row.label ?? null) : undefined;
  if (kind === "heatmap_grid") {
    if (x_scale === "temporal") {
      const x_epoch_ms = new Float64Array(data.length);
      const y_coordinate = new Float64Array(data.length);
      const value = new Float64Array(data.length);
      let value_valid: Uint8Array | undefined;
      for (let index = 0; index < data.length; index += 1) {
        const row = data[index] as heatmap_grid_row;
        const x = row.x instanceof Date ? row.x.getTime() : row.x;
        if (typeof x !== "number" || typeof row.y !== "number") {
          throw new AerisChartsError(
            "invalid_data",
            "temporal heatmap_grid rows require Date/epoch-millisecond X and numeric Y coordinates",
          );
        }
        x_epoch_ms[index] = x;
        y_coordinate[index] = row.y;
        if (row.value === undefined) {
          throw new AerisChartsError("invalid_data", "heatmap_grid rows require a value field");
        }
        if (row.value === null) {
          value_valid ??= new Uint8Array(data.length).fill(1);
          value_valid[index] = 0;
        } else {
          value[index] = row.value;
        }
      }
      return { ids, labels, x_epoch_ms, y_coordinate, value, value_valid };
    }
    if (x_scale !== "band" && x_scale !== "point") {
      const x = new Float64Array(data.length);
      const y_coordinate = new Float64Array(data.length);
      const value = new Float64Array(data.length);
      let value_valid: Uint8Array | undefined;
      for (let index = 0; index < data.length; index += 1) {
        const row = data[index] as heatmap_grid_row;
        if (typeof row.x !== "number" || typeof row.y !== "number") {
          throw new AerisChartsError(
            "invalid_data",
            "continuous heatmap_grid rows require numeric X and Y coordinates",
          );
        }
        x[index] = row.x;
        y_coordinate[index] = row.y;
        if (row.value === undefined) {
          throw new AerisChartsError("invalid_data", "heatmap_grid rows require a value field");
        }
        if (row.value === null) {
          value_valid ??= new Uint8Array(data.length).fill(1);
          value_valid[index] = 0;
        } else {
          value[index] = row.value;
        }
      }
      return { ids, labels, x, y_coordinate, value, value_valid };
    }
    const x_categories: string[] = [];
    const y_categories: string[] = [];
    const x_lookup = new Map<string, number>();
    const y_lookup = new Map<string, number>();
    const x_category_indices = new Uint32Array(data.length);
    const y_category_indices = new Uint32Array(data.length);
    const value = new Float64Array(data.length);
    let value_valid: Uint8Array | undefined;
    for (let index = 0; index < data.length; index += 1) {
      const row = data[index] as heatmap_grid_row;
      if (typeof row.x !== "string" || typeof row.y !== "string") {
        throw new AerisChartsError("invalid_data", "heatmap_grid X and Y categories must be strings");
      }
      let x_category = x_lookup.get(row.x);
      if (x_category === undefined) {
        x_category = x_categories.length;
        x_categories.push(row.x);
        x_lookup.set(row.x, x_category);
      }
      let y_category = y_lookup.get(row.y);
      if (y_category === undefined) {
        y_category = y_categories.length;
        y_categories.push(row.y);
        y_lookup.set(row.y, y_category);
      }
      x_category_indices[index] = x_category;
      y_category_indices[index] = y_category;
      if (row.value === undefined) {
        throw new AerisChartsError("invalid_data", "heatmap_grid rows require a value field");
      }
      if (row.value === null) {
        value_valid ??= new Uint8Array(data.length).fill(1);
        value_valid[index] = 0;
      } else {
        value[index] = row.value;
      }
    }
    return {
      ids, labels, x_categories, x_category_indices, y_categories, y_category_indices, value, value_valid,
    };
  }
  if (kind === "box_plot") {
    const categories: string[] = [];
    const category_lookup = new Map<string, number>();
    const category_indices = new Uint32Array(data.length);
    const min = new Float64Array(data.length);
    const q1 = new Float64Array(data.length);
    const median = new Float64Array(data.length);
    const q3 = new Float64Array(data.length);
    const max = new Float64Array(data.length);
    let min_valid: Uint8Array | undefined;
    let q1_valid: Uint8Array | undefined;
    let median_valid: Uint8Array | undefined;
    let q3_valid: Uint8Array | undefined;
    let max_valid: Uint8Array | undefined;
    for (let index = 0; index < data.length; index += 1) {
      const row = data[index] as box_plot_row;
      if (typeof row.x !== "string") {
        throw new AerisChartsError("invalid_data", "box_plot category X values must be strings");
      }
      let category = category_lookup.get(row.x);
      if (category === undefined) {
        category = categories.length;
        categories.push(row.x);
        category_lookup.set(row.x, category);
      }
      category_indices[index] = category;
      for (const [value, values, validity] of [
        [row.min, min, () => { min_valid ??= new Uint8Array(data.length).fill(1); return min_valid; }],
        [row.q1, q1, () => { q1_valid ??= new Uint8Array(data.length).fill(1); return q1_valid; }],
        [row.median, median, () => { median_valid ??= new Uint8Array(data.length).fill(1); return median_valid; }],
        [row.q3, q3, () => { q3_valid ??= new Uint8Array(data.length).fill(1); return q3_valid; }],
        [row.max, max, () => { max_valid ??= new Uint8Array(data.length).fill(1); return max_valid; }],
      ] as const) {
        if (value === undefined) {
          throw new AerisChartsError("invalid_data", "box_plot rows require min, q1, median, q3, and max fields");
        }
        if (value === null) {
          validity()[index] = 0;
        } else {
          values[index] = value;
        }
      }
    }
    return {
      ids, labels, categories, category_indices,
      min, min_valid, q1, q1_valid, median, median_valid, q3, q3_valid, max, max_valid,
    };
  }
  const is_range = kind === "range_area" || kind === "range_bar";
  const y = new Float64Array(data.length);
  let y_valid: Uint8Array | undefined;
  const low = is_range ? new Float64Array(data.length) : undefined;
  let low_valid: Uint8Array | undefined;
  for (let index = 0; index < data.length; index += 1) {
    const row = data[index]!;
    const value = is_range ? (row as range_area_row).high : (row as general_xy_row).y;
    if (value === undefined) {
      throw new AerisChartsError("invalid_data", `${kind} rows require low and high fields`);
    }
    if (value === null) {
      y_valid ??= new Uint8Array(data.length).fill(1);
      y_valid[index] = 0;
    } else {
      y[index] = value;
    }
    if (is_range) {
      const low_value = (row as range_area_row).low;
      if (low_value === undefined) {
        throw new AerisChartsError("invalid_data", `${kind} rows require low and high fields`);
      }
      if (low_value === null) {
        low_valid ??= new Uint8Array(data.length).fill(1);
        low_valid[index] = 0;
      } else {
        low![index] = low_value;
      }
    }
  }
  const x_mode = kind === "scatter"
    || kind === "bubble"
    ? "numeric"
    : kind === "column" || kind === "horizontal_bar"
      ? "category"
      : x_scale === "temporal"
        ? "temporal"
        : x_scale === "band" || x_scale === "point"
          ? "category"
          : "numeric";
  if (x_mode === "numeric") {
    const x = new Float64Array(data.length);
    for (let index = 0; index < data.length; index += 1) {
      const value = data[index]!.x;
      if (typeof value !== "number") {
        throw new AerisChartsError("invalid_data", `${kind} numeric X values must be numbers`);
      }
      x[index] = value;
    }
    if (kind === "bubble") {
      const size = new Float64Array(data.length);
      let size_valid: Uint8Array | undefined;
      for (let index = 0; index < data.length; index += 1) {
        const value = (data[index] as bubble_row).size;
        if (value === undefined) {
          throw new AerisChartsError("invalid_data", "bubble rows require a size field");
        }
        if (value === null) {
          size_valid ??= new Uint8Array(data.length).fill(1);
          size_valid[index] = 0;
        } else {
          size[index] = value;
        }
      }
      return { ids, labels, x, y, y_valid, size, size_valid };
    }
    if (kind === "error_bar") {
      const x_low = new Float64Array(data.length);
      const x_high = new Float64Array(data.length);
      const y_low = new Float64Array(data.length);
      const y_high = new Float64Array(data.length);
      const x_low_valid = new Uint8Array(data.length);
      const x_high_valid = new Uint8Array(data.length);
      const y_low_valid = new Uint8Array(data.length);
      const y_high_valid = new Uint8Array(data.length);
      for (let index = 0; index < data.length; index += 1) {
        const row = data[index] as error_bar_row;
        for (const [value, values, validity] of [
          [row.x_low, x_low, x_low_valid],
          [row.x_high, x_high, x_high_valid],
        ] as const) {
          if (value === undefined || value === null) continue;
          if (typeof value !== "number") {
            throw new AerisChartsError("invalid_data", "numeric error_bar X bounds must be numbers");
          }
          values[index] = value;
          validity[index] = 1;
        }
        for (const [value, values, validity] of [
          [row.y_low, y_low, y_low_valid],
          [row.y_high, y_high, y_high_valid],
        ] as const) {
          if (value === undefined || value === null) continue;
          values[index] = value;
          validity[index] = 1;
        }
      }
      return {
        ids, labels, x, y, y_valid, x_low, x_low_valid, x_high, x_high_valid,
        y_low, y_low_valid, y_high, y_high_valid,
      };
    }
    if (is_range) return { ids, labels, x, low: low!, low_valid, high: y, high_valid: y_valid };
    return { ids, labels, x, y, y_valid };
  }
  if (x_mode === "temporal") {
    const x_epoch_ms = new Float64Array(data.length);
    for (let index = 0; index < data.length; index += 1) {
      const value = data[index]!.x;
      if (value instanceof Date) {
        x_epoch_ms[index] = value.getTime();
      } else if (typeof value === "number") {
        x_epoch_ms[index] = value;
      } else {
        throw new AerisChartsError(
          "invalid_data",
          `${kind} temporal X values must be Date objects or epoch-millisecond numbers`,
        );
      }
    }
    if (kind === "error_bar") {
      const x_low_epoch_ms = new Float64Array(data.length);
      const x_high_epoch_ms = new Float64Array(data.length);
      const y_low = new Float64Array(data.length);
      const y_high = new Float64Array(data.length);
      const x_low_valid = new Uint8Array(data.length);
      const x_high_valid = new Uint8Array(data.length);
      const y_low_valid = new Uint8Array(data.length);
      const y_high_valid = new Uint8Array(data.length);
      for (let index = 0; index < data.length; index += 1) {
        const row = data[index] as error_bar_row;
        for (const [value, values, validity] of [
          [row.x_low, x_low_epoch_ms, x_low_valid],
          [row.x_high, x_high_epoch_ms, x_high_valid],
        ] as const) {
          if (value === undefined || value === null) continue;
          if (value instanceof Date) {
            values[index] = value.getTime();
          } else if (typeof value === "number") {
            values[index] = value;
          } else {
            throw new AerisChartsError(
              "invalid_data",
              "temporal error_bar X bounds must be Date objects or epoch-millisecond numbers",
            );
          }
          validity[index] = 1;
        }
        if (row.y_low !== undefined && row.y_low !== null) {
          y_low[index] = row.y_low;
          y_low_valid[index] = 1;
        }
        if (row.y_high !== undefined && row.y_high !== null) {
          y_high[index] = row.y_high;
          y_high_valid[index] = 1;
        }
      }
      return {
        ids, labels, x_epoch_ms, y, y_valid, x_low_epoch_ms, x_low_valid,
        x_high_epoch_ms, x_high_valid, y_low, y_low_valid, y_high, y_high_valid,
      };
    }
    if (is_range) return { ids, labels, x_epoch_ms, low: low!, low_valid, high: y, high_valid: y_valid };
    return { ids, labels, x_epoch_ms, y, y_valid };
  }
  const categories: string[] = [];
  const category_lookup = new Map<string, number>();
  const category_indices = new Uint32Array(data.length);
  for (let index = 0; index < data.length; index += 1) {
    const value = data[index]!.x;
    if (typeof value !== "string") {
      throw new AerisChartsError("invalid_data", `${kind} category X values must be strings`);
    }
    let category = category_lookup.get(value);
    if (category === undefined) {
      category = categories.length;
      categories.push(value);
      category_lookup.set(value, category);
    }
    category_indices[index] = category;
  }
  if (is_range) return { ids, labels, categories, category_indices, low: low!, low_valid, high: y, high_valid: y_valid };
  if (kind === "error_bar") {
    const y_low = new Float64Array(data.length);
    const y_high = new Float64Array(data.length);
    const y_low_valid = new Uint8Array(data.length);
    const y_high_valid = new Uint8Array(data.length);
    for (let index = 0; index < data.length; index += 1) {
      const row = data[index] as error_bar_row;
      if (row.x_low !== undefined || row.x_high !== undefined) {
        throw new AerisChartsError("invalid_data", "category error bars do not accept X bounds");
      }
      if (row.y_low !== undefined && row.y_low !== null) {
        y_low[index] = row.y_low;
        y_low_valid[index] = 1;
      }
      if (row.y_high !== undefined && row.y_high !== null) {
        y_high[index] = row.y_high;
        y_high_valid[index] = 1;
      }
    }
    return { ids, labels, categories, category_indices, y, y_valid, y_low, y_low_valid, y_high, y_high_valid };
  }
  return { ids, labels, categories, category_indices, y, y_valid };
}

function normalize_general_axis_options(options: general_axis_options): general_axis_options {
  const normalized = { ...options } as general_axis_options & { domain?: unknown };
  if (Array.isArray(options.domain) && options.scale === "temporal") {
    normalized.domain = options.domain.map((value) => value instanceof Date ? value.getTime() : value);
  }
  if (Array.isArray(options.ticks)) {
    normalized.ticks = options.ticks.map((tick) => tick.type === "temporal" && tick.value instanceof Date
      ? { ...tick, value: tick.value.getTime() }
      : tick);
  }
  return normalized as general_axis_options;
}

class general_axis_impl implements general_axis_api {
  applyOptions(...args: Parameters<general_axis_api["apply_options"]>): void { this.apply_options(...args); }
  resetView(): void { this.reset_view(); }
  setVisible(...args: Parameters<general_axis_api["set_visible"]>): void { this.set_visible(...args); }
  constructor(
    readonly id: string,
    private readonly handle_token: number,
    private readonly chart: chart_impl,
  ) {}

  private current(): general_axis_options {
    if (this.chart.wasm.general_axis_handle_token(this.id) !== this.handle_token) {
      throw new AerisChartsError("stale_handle", "this general axis has been removed");
    }
    const value = JSON.parse(this.chart.wasm.general_axis_json(this.id)) as general_axis_options | null;
    if (value === null) throw new AerisChartsError("stale_handle", "this general axis has been removed");
    return value;
  }

  options(): general_axis_options {
    return this.current();
  }

  apply_options(patch: Partial<general_axis_presentation_options>): void {
    const options = { ...this.current(), ...patch };
    parse_general_result<null>(
      this.chart.wasm.update_general_axis_result_json(
        JSON.stringify(normalize_general_axis_options(options)),
      ),
    );
    this.chart.repaint();
  }

  set_visible(visible: boolean): void {
    this.current();
    if (!this.chart.wasm.set_general_axis_visible(this.id, visible)) {
      throw new AerisChartsError("stale_handle", "this general axis has been removed");
    }
    this.chart.repaint();
  }

  pan(fraction: number): void {
    this.current();
    parse_general_result<null>(this.chart.wasm.pan_general_axis_result_json(this.id, fraction));
    this.chart.repaint();
  }

  zoom(factor: number, anchor_value: number | string): void {
    const options = this.current();
    const category = options.scale === "band" || options.scale === "point";
    if (category !== (typeof anchor_value === "string")) {
      throw new AerisChartsError(
        "invalid_options",
        category ? "category axis zoom requires a string anchor" : "continuous axis zoom requires a numeric anchor",
      );
    }
    const result = category
      ? this.chart.wasm.zoom_general_category_axis_result_json(this.id, factor, anchor_value as string)
      : this.chart.wasm.zoom_general_axis_result_json(this.id, factor, anchor_value as number);
    parse_general_result<null>(result);
    this.chart.repaint();
  }

  reset_view(): void {
    this.current();
    this.chart.wasm.reset_general_axis_view(this.id);
    this.chart.repaint();
  }

  remove(): boolean {
    this.current();
    const removed = this.chart.wasm.remove_general_axis(this.id);
    if (removed) this.chart.repaint();
    return removed;
  }
}

type general_reference_wire_value =
  | { type: "numeric"; value: number }
  | { type: "temporal"; value: number }
  | { type: "category"; value: string };

type general_reference_wire_options =
  | (Omit<Extract<general_reference_options, { kind: "line" }>, "value"> & {
      value: general_reference_wire_value;
    })
  | (Omit<Extract<general_reference_options, { kind: "dot" }>, "x" | "y"> & {
      x: general_reference_wire_value;
      y: general_reference_wire_value;
    })
  | (Omit<
      Extract<general_reference_options, { kind: "region" }>,
      "x_from" | "x_to" | "y_from" | "y_to"
    > & {
      x_from: general_reference_wire_value;
      x_to: general_reference_wire_value;
      y_from: general_reference_wire_value;
      y_to: general_reference_wire_value;
    });

function general_reference_wire_value(
  chart: chart_impl,
  axis_id: string,
  value: general_reference_value,
): general_reference_wire_value {
  const axis = chart.axis(axis_id);
  if (axis === null) {
    throw new AerisChartsError(
      "invalid_options",
      "general reference axis \"" + axis_id + "\" does not exist",
    );
  }
  const scale = axis.options().scale;
  if (scale === "temporal") {
    const epoch_ms = value instanceof Date ? value.getTime() : value;
    if (typeof epoch_ms !== "number" || !Number.isSafeInteger(epoch_ms)) {
      throw new AerisChartsError(
        "invalid_options",
        "temporal reference values must be Date objects or whole epoch-millisecond numbers",
      );
    }
    return { type: "temporal", value: epoch_ms };
  }
  if (scale === "band" || scale === "point") {
    if (typeof value !== "string") {
      throw new AerisChartsError("invalid_options", "category reference values must be strings");
    }
    return { type: "category", value };
  }
  if (scale === "linear" || scale === "log" || scale === "symlog") {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new AerisChartsError("invalid_options", "numeric reference values must be finite numbers");
    }
    return { type: "numeric", value };
  }
  throw new AerisChartsError("invalid_options", "general references require Cartesian axes");
}

function encode_general_reference_options(
  chart: chart_impl,
  options: general_reference_options,
): general_reference_wire_options {
  switch (options.kind) {
    case "line":
      return {
        ...options,
        value: general_reference_wire_value(chart, options.axis_id, options.value),
      };
    case "dot":
      return {
        ...options,
        x: general_reference_wire_value(chart, options.x_axis_id, options.x),
        y: general_reference_wire_value(chart, options.y_axis_id, options.y),
      };
    case "region":
      return {
        ...options,
        x_from: general_reference_wire_value(chart, options.x_axis_id, options.x_from),
        x_to: general_reference_wire_value(chart, options.x_axis_id, options.x_to),
        y_from: general_reference_wire_value(chart, options.y_axis_id, options.y_from),
        y_to: general_reference_wire_value(chart, options.y_axis_id, options.y_to),
      };
  }
}

function decode_general_reference_value(value: general_reference_wire_value): number | string {
  return value.value;
}

function decode_general_reference_options(
  options: general_reference_wire_options,
): general_reference_options {
  switch (options.kind) {
    case "line":
      return { ...options, value: decode_general_reference_value(options.value) };
    case "dot":
      return {
        ...options,
        x: decode_general_reference_value(options.x),
        y: decode_general_reference_value(options.y),
      };
    case "region":
      return {
        ...options,
        x_from: decode_general_reference_value(options.x_from),
        x_to: decode_general_reference_value(options.x_to),
        y_from: decode_general_reference_value(options.y_from),
        y_to: decode_general_reference_value(options.y_to),
      };
  }
}

class general_reference_impl implements general_reference_api {
  constructor(readonly id: number, private readonly chart: chart_impl) {}

  private current(): general_reference_wire_options {
    const value = JSON.parse(
      this.chart.wasm.general_reference_options_json(this.id),
    ) as general_reference_wire_options | null;
    if (value === null) {
      throw new AerisChartsError("stale_handle", "this general reference has been removed");
    }
    return value;
  }

  options(): general_reference_options {
    return decode_general_reference_options(this.current());
  }

  remove(): boolean {
    this.current();
    const removed = this.chart.wasm.remove_general_reference(this.id);
    if (removed) this.chart.repaint();
    return removed;
  }
}

class general_series_impl implements general_series_api {
  applyOptions(...args: Parameters<general_series_api["apply_options"]>): void { this.apply_options(...args); }
  setVisible(...args: Parameters<general_series_api["set_visible"]>): void { this.set_visible(...args); }
  setData(...args: Parameters<general_series_api["set_data"]>): void { this.set_data(...args); }
  setDataTyped(...args: Parameters<general_series_api["set_data_typed"]>): void { this.set_data_typed(...args); }
  updateData(...args: Parameters<general_series_api["update_data"]>): void { this.update_data(...args); }
  updateDataTyped(...args: Parameters<general_series_api["update_data_typed"]>): void { this.update_data_typed(...args); }
  dataAt(...args: Parameters<general_series_api["data_at"]>): general_tooltip_snapshot | null { return this.data_at(...args); }
  selectedHit(): general_series_hit | null { return this.selected_hit(); }
  accessibilitySnapshot(...args: Parameters<general_series_api["accessibility_snapshot"]>): general_accessibility_snapshot {
    return this.accessibility_snapshot(...args);
  }
  private readonly data_changed_subs = new Set<data_changed_handler>();
  private removed = false;

  constructor(
    readonly id: number,
    private readonly dataset: number,
    readonly kind: general_series_kind,
    private x_axis_id: string,
    private y_axis_id: string,
    private readonly chart: chart_impl,
  ) {}

  mark_removed(): void {
    this.removed = true;
    this.data_changed_subs.clear();
  }

  private assert_live(): void {
    void this.chart.wasm;
    if (this.removed) throw new AerisChartsError("stale_handle", "this general series has been removed");
  }

  private x_scale(): general_axis_options["scale"] {
    const axis = this.chart.axis(this.x_axis_id);
    if (axis === null) throw new AerisChartsError("stale_handle", "this general series X axis has been removed");
    return axis.options().scale;
  }

  options(): general_series_options {
    this.assert_live();
    const options = JSON.parse(this.chart.wasm.general_series_options_json(this.id)) as general_series_options | null;
    if (options === null) {
      throw new AerisChartsError("stale_handle", "this general series has been removed");
    }
    return options;
  }

  apply_options(patch: Partial<general_series_options>): void {
    const options = { ...this.options(), ...patch };
    parse_general_result<null>(
      this.chart.wasm.update_general_series_options_result_json(this.id, JSON.stringify(options)),
    );
    this.x_axis_id = options.x_axis_id;
    this.y_axis_id = options.y_axis_id;
    this.chart.repaint();
  }

  set_visible(visible: boolean): void {
    this.assert_live();
    if (!this.chart.wasm.set_general_series_visible(this.id, visible)) {
      throw new AerisChartsError("stale_handle", "this general series has been removed");
    }
    this.chart.repaint();
  }

  set_data(data: readonly (general_xy_row | bubble_row | range_area_row | error_bar_row | box_plot_row | heatmap_grid_row)[]): void {
    this.install_data(pack_general_rows(this.kind, data, this.x_scale()));
  }

  set_data_typed(columns: numeric_xy_columns | temporal_xy_columns | category_xy_columns | bubble_columns | numeric_range_columns | temporal_range_columns | category_range_columns | numeric_error_columns | temporal_error_columns | category_error_columns | category_box_columns | category_heatmap_columns | numeric_heatmap_columns | temporal_heatmap_columns): void {
    this.install_data(columns);
  }

  update_data(data: readonly (general_xy_row | bubble_row | range_area_row | error_bar_row | box_plot_row | heatmap_grid_row)[], options: general_update_options = {}): void {
    if (data.some((row) => row.id === undefined)) {
      throw new AerisChartsError("invalid_data", "general incremental updates require explicit row IDs");
    }
    this.upsert_data(pack_general_rows(this.kind, data, this.x_scale()), options);
  }

  update_data_typed(
    columns: numeric_xy_columns | temporal_xy_columns | category_xy_columns | bubble_columns | numeric_range_columns | temporal_range_columns | category_range_columns | numeric_error_columns | temporal_error_columns | category_error_columns | category_box_columns | category_heatmap_columns | numeric_heatmap_columns | temporal_heatmap_columns,
    options: general_update_options = {},
  ): void {
    this.upsert_data(columns, options);
  }

  private upsert_data(columns: general_columns_input, options: general_update_options): void {
    this.assert_live();
    if (columns.ids === undefined) {
      throw new AerisChartsError("invalid_data", "general incremental updates require explicit row IDs");
    }
    const max_rows = options.max_rows;
    if (max_rows !== undefined && (!Number.isSafeInteger(max_rows) || max_rows <= 0 || max_rows > 0xffff_ffff)) {
      throw new AerisChartsError("invalid_options", "general max_rows must be a positive safe 32-bit integer");
    }
    if (this.kind === "scatter" && !("x" in columns)) {
      throw new AerisChartsError("invalid_data", "scatter requires numeric XY columns");
    }
    if (this.kind === "bubble" && (!("x" in columns) || !("size" in columns))) {
      throw new AerisChartsError("invalid_data", "bubble requires numeric XY columns with a size channel");
    }
    if ((this.kind === "range_area" || this.kind === "range_bar") && !("low" in columns)) {
      throw new AerisChartsError("invalid_data", `${this.kind} requires low and high columns`);
    }
    if (this.kind === "error_bar" && !("y_low" in columns)) {
      throw new AerisChartsError("invalid_data", "error_bar requires numeric, temporal, or category error-bound columns");
    }
    if ((this.kind === "column" || this.kind === "horizontal_bar") && !("category_indices" in columns)) {
      throw new AerisChartsError("invalid_data", `${this.kind} requires category/value columns`);
    }
    if (this.kind === "box_plot" && !("median" in columns)) {
      throw new AerisChartsError("invalid_data", "box_plot requires category box columns");
    }
    if (this.kind === "heatmap_grid" && !("value" in columns)) {
      throw new AerisChartsError("invalid_data", "heatmap_grid requires heatmap coordinate/value columns");
    }
    let result: string;
    if ("value" in columns) {
      if ("x_category_indices" in columns) {
        result = this.chart.wasm.upsert_general_heatmap_category_data_typed(
          this.dataset,
          general_ids_json(columns.ids, columns.value.length),
          JSON.stringify({
            x_categories: columns.x_categories,
            y_categories: columns.y_categories,
            labels: general_labels_value(columns.labels, columns.value.length),
            max_rows: max_rows ?? 0,
          }),
          columns.x_category_indices,
          columns.y_category_indices,
          columns.value,
          columns.value_valid,
        );
      } else if ("x_epoch_ms" in columns) {
        result = this.chart.wasm.upsert_general_heatmap_temporal_data_typed(
          this.dataset,
          general_value_metadata_json(columns, columns.value.length),
          columns.x_epoch_ms,
          columns.y_coordinate,
          columns.value,
          columns.value_valid,
          max_rows ?? 0,
        );
      } else {
        result = this.chart.wasm.upsert_general_heatmap_numeric_data_typed(
          this.dataset,
          general_value_metadata_json(columns, columns.value.length),
          columns.x,
          columns.y_coordinate,
          columns.value,
          columns.value_valid,
          max_rows ?? 0,
        );
      }
    } else if ("median" in columns) {
      result = this.chart.wasm.upsert_general_box_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({
          categories: columns.categories,
          labels: general_labels_value(columns.labels, columns.category_indices.length),
          max_rows: max_rows ?? 0,
        }),
        columns.category_indices,
        columns.min, columns.min_valid,
        columns.q1, columns.q1_valid,
        columns.median, columns.median_valid,
        columns.q3, columns.q3_valid,
        columns.max, columns.max_valid,
      );
    } else if ("x_low" in columns) {
      result = this.chart.wasm.upsert_general_error_numeric_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x.length), columns.x,
        columns.y, columns.y_valid, columns.x_low, columns.x_low_valid,
        columns.x_high, columns.x_high_valid, columns.y_low, columns.y_low_valid,
        columns.y_high, columns.y_high_valid, max_rows ?? 0,
      );
    } else if ("x_low_epoch_ms" in columns) {
      result = this.chart.wasm.upsert_general_error_temporal_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x_epoch_ms.length), columns.x_epoch_ms,
        columns.y, columns.y_valid, columns.x_low_epoch_ms, columns.x_low_valid,
        columns.x_high_epoch_ms, columns.x_high_valid, columns.y_low, columns.y_low_valid,
        columns.y_high, columns.y_high_valid, max_rows ?? 0,
      );
    } else if ("y_low" in columns) {
      result = this.chart.wasm.upsert_general_error_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({ categories: columns.categories, labels: general_labels_value(columns.labels, columns.category_indices.length), max_rows: max_rows ?? 0 }),
        columns.category_indices, columns.y, columns.y_valid,
        columns.y_low, columns.y_low_valid, columns.y_high, columns.y_high_valid,
      );
    } else if ("low" in columns && "x" in columns) {
      result = this.chart.wasm.upsert_general_range_numeric_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x.length), columns.x,
        columns.low, columns.low_valid, columns.high, columns.high_valid, max_rows ?? 0,
      );
    } else if ("low" in columns && "x_epoch_ms" in columns) {
      result = this.chart.wasm.upsert_general_range_temporal_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x_epoch_ms.length), columns.x_epoch_ms,
        columns.low, columns.low_valid, columns.high, columns.high_valid, max_rows ?? 0,
      );
    } else if ("low" in columns) {
      result = this.chart.wasm.upsert_general_range_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({ categories: columns.categories, labels: general_labels_value(columns.labels, columns.category_indices.length), max_rows: max_rows ?? 0 }),
        columns.category_indices, columns.low, columns.low_valid, columns.high, columns.high_valid,
      );
    } else if ("size" in columns) {
      result = this.chart.wasm.upsert_general_bubble_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x.length),
        columns.x,
        columns.y,
        columns.y_valid,
        columns.size,
        columns.size_valid,
        max_rows ?? 0,
      );
    } else if ("x" in columns) {
      result = this.chart.wasm.upsert_general_numeric_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x.length),
        columns.x,
        columns.y,
        columns.y_valid,
        max_rows ?? 0,
      );
    } else if ("x_epoch_ms" in columns) {
      result = this.chart.wasm.upsert_general_temporal_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x_epoch_ms.length),
        columns.x_epoch_ms,
        columns.y,
        columns.y_valid,
        max_rows ?? 0,
      );
    } else {
      result = this.chart.wasm.upsert_general_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({
          categories: columns.categories,
          labels: general_labels_value(columns.labels, columns.category_indices.length),
          max_rows: max_rows ?? 0,
        }),
        columns.category_indices,
        columns.y,
        columns.y_valid,
      );
    }
    parse_general_result<null>(result);
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  private install_data(columns: general_columns_input): void {
    this.assert_live();
    if (this.kind === "scatter" && !("x" in columns)) {
      throw new AerisChartsError("invalid_data", "scatter requires numeric XY columns");
    }
    if (this.kind === "bubble" && (!("x" in columns) || !("size" in columns))) {
      throw new AerisChartsError("invalid_data", "bubble requires numeric XY columns with a size channel");
    }
    if ((this.kind === "range_area" || this.kind === "range_bar") && !("low" in columns)) {
      throw new AerisChartsError("invalid_data", `${this.kind} requires low and high columns`);
    }
    if (this.kind === "error_bar" && !("y_low" in columns)) {
      throw new AerisChartsError("invalid_data", "error_bar requires numeric, temporal, or category error-bound columns");
    }
    if ((this.kind === "column" || this.kind === "horizontal_bar") && !("category_indices" in columns)) {
      throw new AerisChartsError("invalid_data", `${this.kind} requires category/value columns`);
    }
    if (this.kind === "box_plot" && !("median" in columns)) {
      throw new AerisChartsError("invalid_data", "box_plot requires category box columns");
    }
    if (this.kind === "heatmap_grid" && !("value" in columns)) {
      throw new AerisChartsError("invalid_data", "heatmap_grid requires heatmap coordinate/value columns");
    }
    let result: string;
    if ("value" in columns) {
      if ("x_category_indices" in columns) {
        result = this.chart.wasm.set_general_heatmap_category_data_typed(
          this.dataset,
          general_ids_json(columns.ids, columns.value.length),
          JSON.stringify({
            x_categories: columns.x_categories,
            y_categories: columns.y_categories,
            labels: general_labels_value(columns.labels, columns.value.length),
          }),
          columns.x_category_indices,
          columns.y_category_indices,
          columns.value,
          columns.value_valid,
        );
      } else if ("x_epoch_ms" in columns) {
        result = this.chart.wasm.set_general_heatmap_temporal_data_typed(
          this.dataset,
          general_value_metadata_json(columns, columns.value.length),
          columns.x_epoch_ms,
          columns.y_coordinate,
          columns.value,
          columns.value_valid,
        );
      } else {
        result = this.chart.wasm.set_general_heatmap_numeric_data_typed(
          this.dataset,
          general_value_metadata_json(columns, columns.value.length),
          columns.x,
          columns.y_coordinate,
          columns.value,
          columns.value_valid,
        );
      }
    } else if ("median" in columns) {
      result = this.chart.wasm.set_general_box_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({
          categories: columns.categories,
          labels: general_labels_value(columns.labels, columns.category_indices.length),
        }),
        columns.category_indices,
        columns.min, columns.min_valid,
        columns.q1, columns.q1_valid,
        columns.median, columns.median_valid,
        columns.q3, columns.q3_valid,
        columns.max, columns.max_valid,
      );
    } else if ("x_low" in columns) {
      result = this.chart.wasm.set_general_error_numeric_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x.length), columns.x,
        columns.y, columns.y_valid, columns.x_low, columns.x_low_valid,
        columns.x_high, columns.x_high_valid, columns.y_low, columns.y_low_valid,
        columns.y_high, columns.y_high_valid,
      );
    } else if ("x_low_epoch_ms" in columns) {
      result = this.chart.wasm.set_general_error_temporal_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x_epoch_ms.length), columns.x_epoch_ms,
        columns.y, columns.y_valid, columns.x_low_epoch_ms, columns.x_low_valid,
        columns.x_high_epoch_ms, columns.x_high_valid, columns.y_low, columns.y_low_valid,
        columns.y_high, columns.y_high_valid,
      );
    } else if ("y_low" in columns) {
      result = this.chart.wasm.set_general_error_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({ categories: columns.categories, labels: general_labels_value(columns.labels, columns.category_indices.length) }),
        columns.category_indices, columns.y, columns.y_valid,
        columns.y_low, columns.y_low_valid, columns.y_high, columns.y_high_valid,
      );
    } else if ("low" in columns && "x" in columns) {
      result = this.chart.wasm.set_general_range_numeric_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x.length), columns.x,
        columns.low, columns.low_valid, columns.high, columns.high_valid,
      );
    } else if ("low" in columns && "x_epoch_ms" in columns) {
      result = this.chart.wasm.set_general_range_temporal_data_typed(
        this.dataset, general_value_metadata_json(columns, columns.x_epoch_ms.length), columns.x_epoch_ms,
        columns.low, columns.low_valid, columns.high, columns.high_valid,
      );
    } else if ("low" in columns) {
      result = this.chart.wasm.set_general_range_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({ categories: columns.categories, labels: general_labels_value(columns.labels, columns.category_indices.length) }),
        columns.category_indices, columns.low, columns.low_valid, columns.high, columns.high_valid,
      );
    } else if ("size" in columns) {
      result = this.chart.wasm.set_general_bubble_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x.length),
        columns.x,
        columns.y,
        columns.y_valid,
        columns.size,
        columns.size_valid,
      );
    } else if ("x" in columns) {
      result = this.chart.wasm.set_general_numeric_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x.length),
        columns.x,
        columns.y,
        columns.y_valid,
      );
    } else if ("x_epoch_ms" in columns) {
      result = this.chart.wasm.set_general_temporal_data_typed(
        this.dataset,
        general_value_metadata_json(columns, columns.x_epoch_ms.length),
        columns.x_epoch_ms,
        columns.y,
        columns.y_valid,
      );
    } else {
      result = this.chart.wasm.set_general_category_data_typed(
        this.dataset,
        general_ids_json(columns.ids, columns.category_indices.length),
        JSON.stringify({
          categories: columns.categories,
          labels: general_labels_value(columns.labels, columns.category_indices.length),
        }),
        columns.category_indices,
        columns.y,
        columns.y_valid,
      );
    }
    parse_general_result<null>(result);
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  data_at(row: number): general_tooltip_snapshot | null {
    this.assert_live();
    if (!Number.isSafeInteger(row) || row < 0) {
      throw new AerisChartsError("invalid_options", "general data row must be a non-negative safe integer");
    }
    return JSON.parse(this.chart.wasm.general_tooltip_json(this.id, row)) as general_tooltip_snapshot | null;
  }

  selected_hit(): general_series_hit | null {
    this.assert_live();
    const hit = this.chart.general_selected_hit();
    return hit?.series === this.id ? hit : null;
  }

  accessibility_snapshot(offset = 0, limit = 512): general_accessibility_snapshot {
    this.assert_live();
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 0) {
      throw new AerisChartsError(
        "invalid_options",
        "general accessibility offset and limit must be non-negative safe integers",
      );
    }
    const snapshot = JSON.parse(
      this.chart.wasm.general_accessibility_json(this.id, offset, limit),
    ) as general_accessibility_snapshot | null;
    if (snapshot === null) {
      throw new AerisChartsError("stale_handle", "this general series has been removed");
    }
    return snapshot;
  }

  subscribe_data_changed(handler: data_changed_handler): void {
    this.assert_live();
    this.data_changed_subs.add(handler);
  }

  unsubscribe_data_changed(handler: data_changed_handler): void {
    this.data_changed_subs.delete(handler);
  }

  remove(): void {
    this.chart.remove_general_series_handle(this);
  }

  remove_from_engine(): boolean {
    this.assert_live();
    return this.chart.wasm.remove_general_series(this.id, this.dataset);
  }
}

/** Point keys that never carry a price: a payload with only these (plus `time`) is whitespace. */
const NON_PRICE_POINT_KEYS = new Set(["time", "color", "wick_color", "border_color", "custom_values", "customValues"]);
const OHLC_POINT_KEYS = ["open", "high", "low", "close"] as const;

/** A rejected streaming record in the shared `ingestion_diagnostics` shape. */
function rejected_ingestion(
  reason: string,
  code?: ingestion_diagnostics["code"],
  non_finite = 0,
  out_of_range = 0,
): ingestion_diagnostics {
  return {
    status: "rejected",
    accepted: 0,
    dropped_invalid: non_finite + out_of_range > 0 ? 1 : 0,
    dropped_non_finite: non_finite,
    dropped_out_of_range: out_of_range,
    deduplicated: 0,
    reordered: false,
    semantic_anomalies: 0,
    reason,
    ...(code === undefined ? {} : { code }),
  };
}

/** A host write to a series the engine owns (a footprint, a trade-bound candle or bar, a CVD,
 *  delta, or volume study, resampled or synthetic bars). The wasm boundary reports the same
 *  record for the typed, sequenced, and merge entries. */
function derived_series_ingestion(id: number): ingestion_diagnostics {
  return rejected_ingestion(
    `series ${id} is derived by the engine; write to its trade stream, resampler source, or synthetic-bar source instead`,
    "derived_series",
  );
}

const INVALID_SEQUENCE = "sequence must be a non-negative safe integer";

/** Custom and advanced series own their payloads outside the OHLC streaming path, so a sequence
 *  passed to them would silently guard nothing. */
function reject_sequence_option(options: series_update_options | undefined, series: string): void {
  if (options?.sequence !== undefined) {
    throw new AerisChartsError(
      "unsupported_operation",
      `the sequence guard applies to built-in OHLC and value series, not to ${series}`,
    );
  }
}

class series_impl implements series_api {
  setData(...args: Parameters<series_api["set_data"]>): void { this.set_data(...args); }
  setDataTyped(...args: Parameters<series_api["set_data_typed"]>): void { this.set_data_typed(...args); }
  updateTyped(...args: Parameters<series_api["update_typed"]>): void { this.update_typed(...args); }
  applyOptions(...args: Parameters<series_api["apply_options"]>): void { this.apply_options(...args); }
  moveToPane(...args: Parameters<series_api["move_to_pane"]>): void { this.move_to_pane(...args); }
  priceScale(): price_scale_api { return this.price_scale(); }
  protected readonly data_changed_subs = new Set<data_changed_handler>();
  private removed = false;
  private last_ingestion: ingestion_diagnostics | null = null;
  /** Accepted-with-hint streaming codes already warned for this handle (bounded by the code set). */
  private readonly warned_codes = new Set<string>();

  constructor(
    readonly id: number,
    readonly kind: series_kind,
    protected readonly chart: chart_impl,
  ) {}

  /** Called by `chart_impl.remove_series`; makes every subsequent method on this handle throw. */
  mark_removed(): void {
    this.removed = true;
    this.data_changed_subs.clear();
  }

  /**
   * Fire this series' `data_changed` after a ring drain delivered rows to it. Scope is `"update"`:
   * a drain appends or replace-lasts, exactly like `update`/`update_typed`. Called by the chart's
   * drain loop, which already owns the repaint, so this only notifies.
   */
  emit_ring_data_changed(): void {
    if (this.removed) return;
    this.chart.note_series_update(this.id, false);
    for (const handler of this.data_changed_subs) handler("update");
  }
  protected assert_live(): void {
    void this.chart.wasm;
    if (this.removed) throw new AerisChartsError("stale_handle", "this series has been removed from the chart");
  }

  /** The wasm sequence argument (`NaN` = none), or `null` after recording and warning about an
   *  invalid one; the call is then dropped like any other rejected ingestion. */
  private streaming_sequence(options: series_update_options | undefined, call: string): number | null {
    const sequence = options?.sequence;
    if (sequence === undefined) return NaN;
    if (Number.isSafeInteger(sequence) && sequence >= 0) return sequence;
    this.last_ingestion = rejected_ingestion(INVALID_SEQUENCE);
    console.warn(`aeris_charts: ${call} rejected — ${INVALID_SEQUENCE}`);
    return null;
  }

  set_data(data: readonly series_data[], options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "set_data");
    if (sequence === null) return;
    const p = pack(data);
    const accepted = this.record_ingestion(
      this.chart.wasm.set_series_data_typed(this.id, p.times, p.open, p.high, p.low, p.close),
    );
    if (!accepted) {
      this.warn_rejected("set_data");
      return;
    }
    // A full replace cleared the guard; a snapshot sequence becomes the new baseline.
    if (!Number.isNaN(sequence)) this.chart.wasm.set_series_update_sequence(this.id, sequence);
    // set_series_data resets point colors, so per-point channels must be applied after it.
    if (p.body_colors !== undefined || p.wick_colors !== undefined || p.border_colors !== undefined) {
      this.chart.wasm.set_series_point_colors(this.id, p.body_colors, p.wick_colors, p.border_colors);
    }
    this.chart.note_series_times(this.id, calendar_input(data));
    this.chart.sync_countdown_timer();
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  /**
   * Columnar fast path: already-packed typed arrays go straight to the engine, skipping
   * the per-object JS packing of `set_data`, which is avoidable work for feed handlers that
   * already hold columnar data. `times` are UTC seconds
   * (the engine's time unit, see `time_to_utc_seconds`); single-value series repeat
   * their value across all four price channels. Per-point colors are not carried —
   * apply them after with the usual options/colors path. The engine's sort/dedupe/
   * sanitize rules apply exactly as with `set_data`.
   */
  set_data_typed(columns: ohlc_columns, options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "set_data_typed");
    if (sequence === null) return;
    const accepted = this.record_ingestion(this.chart.wasm.set_series_data_typed(
      this.id, columns.times, columns.open, columns.high, columns.low, columns.close,
    ));
    if (!accepted) {
      this.warn_rejected("set_data_typed");
      return;
    }
    this.chart.note_series_times(this.id, columns.times.length === 0 ? null : false);
    if (!Number.isNaN(sequence)) this.chart.wasm.set_series_update_sequence(this.id, sequence);
    this.chart.sync_countdown_timer();
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  /**
   * Columnar streaming fast path: the batch goes straight to the engine as five typed arrays,
   * so a feed running at tens of thousands of ticks per second allocates no per-point JS object
   * at the boundary. One `data_changed("update")` per call regardless of batch size — the
   * notification describes the call, not the rows, exactly as it does for `update`.
   */
  update_typed(columns: ohlc_columns, options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "update_typed");
    if (sequence === null) return;
    const accepted = this.record_ingestion(Number.isNaN(sequence)
      ? this.chart.wasm.update_series_bars_typed(
        this.id, columns.times, columns.open, columns.high, columns.low, columns.close,
      )
      : this.chart.wasm.update_series_bars_typed_sequenced(
        this.id, columns.times, columns.open, columns.high, columns.low, columns.close, sequence,
      ));
    if (!accepted) {
      this.warn_rejected("update_typed");
      return;
    }
    if (columns.times.length > 0) this.chart.note_series_update(this.id, false);
    // Same post-update bookkeeping as `update`: data arriving on a countdown-enabled series can
    // start the timer, and repaints coalesce onto the next frame rather than painting per batch.
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  /**
   * Bind (or with `null`, unbind) a `SharedArrayBuffer` ring the engine drains on its own frame
   * tick. The two typed-array views are built here, once, and handed to the engine — it holds them
   * for the lifetime of the binding so no view is constructed per frame.
   */
  set_ring_source(buffer: SharedArrayBuffer | null, layout?: ring_source_layout): void {
    this.assert_live();
    if (buffer === null) {
      this.chart.wasm.clear_ring_source(this.id);
      this.chart.sync_ring_drain_loop();
      return;
    }
    // Unbinding stays allowed above: a ring bound before the series became derived must be
    // releasable. Binding one now would only feed rows the engine drops on every frame.
    if (this.chart.wasm.series_is_derived(this.id)) {
      throw new AerisChartsError(
        "unsupported_operation",
        `series ${this.id} is derived by the engine; feed its trade stream or source instead of binding a ring`,
      );
    }
    if (layout === undefined) {
      throw new AerisChartsError("invalid_options", "set_ring_source requires a layout when a buffer is given");
    }
    // A plain ArrayBuffer would work for the reads but defeats the point (the producer is a worker),
    // and `Atomics.load` on non-shared memory is a footgun rather than an error. Reject it here
    // where the message can say why.
    if (typeof SharedArrayBuffer !== "undefined" && !(buffer instanceof SharedArrayBuffer)) {
      throw new AerisChartsError(
        "invalid_data",
        "set_ring_source expects a SharedArrayBuffer (is the page cross-origin isolated?)",
      );
    }
    const reason = this.chart.wasm.set_ring_source(
      this.id, new Uint8Array(buffer), new Int32Array(buffer), JSON.stringify(layout),
    );
    if (reason !== "") throw new AerisChartsError("invalid_options", `set_ring_source rejected — ${reason}`);
    this.chart.sync_ring_drain_loop();
  }

  update(point: series_data, options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "update");
    if (sequence === null) return;
    // A whitespace point (`{time}` only) streams as an all-NaN bar; the engine keeps the slot.
    const o = "value" in point ? point.value : "open" in point ? point.open : NaN;
    const h = "value" in point ? point.value : "high" in point ? point.high : NaN;
    const l = "value" in point ? point.value : "low" in point ? point.low : NaN;
    const c = "value" in point ? point.value : "close" in point ? point.close : NaN;
    // undefined = no custom color; on a replace of the last bar this also clears a previously
    // set custom color for that channel. Whitespace points carry no color channels.
    const body = "color" in point ? point_color_to_u32(point.color) : undefined;
    const wick = "wick_color" in point ? point_color_to_u32(point.wick_color) : undefined;
    const border = "border_color" in point ? point_color_to_u32(point.border_color) : undefined;
    // Series-scoped streaming: append a new time point or replace the last on this series.
    const time = time_to_utc_seconds(point.time);
    if (!this.record_single_ingestion(time, [o, h, l, c])) {
      const present = OHLC_POINT_KEYS.filter((key) => key in point).length;
      if (!("value" in point) && present > 0 && present < OHLC_POINT_KEYS.length && this.last_ingestion !== null) {
        this.last_ingestion = {
          ...this.last_ingestion,
          code: "partial_ohlc",
          reason: "partial OHLC point (missing open/high/low/close); use merge() for partial ticks",
        };
      }
      // Same signal as update_typed: a dropped streaming point is never silent.
      console.warn(`aeris_charts: update rejected — ${this.last_ingestion?.reason ?? "invalid or out-of-range values"}`);
      return;
    }
    if (Number.isNaN(sequence)) {
      // A refused write is also what invalid values look like, so ask the engine whether the
      // series is derived (a footprint, trade-bound candle or study, synthetic or resampled bars)
      // rather than guessing from the refusal.
      if (!this.chart.wasm.update_series_bar_styled(this.id, time, o, h, l, c, body, wick, border)) {
        this.last_ingestion = this.chart.wasm.series_is_derived(this.id)
          ? derived_series_ingestion(this.id)
          : rejected_ingestion("the engine rejected the point (unknown series or invalid values)");
        this.warn_rejected("update");
        return;
      }
    } else {
      const json = this.chart.wasm.update_series_bar_sequenced(
        this.id, time, o, h, l, c, body, wick, border, sequence,
      );
      // `undefined` = applied; keep any semantic-anomaly record from the boundary check.
      if (json !== undefined && !this.record_ingestion(json)) {
        this.warn_rejected("update");
        return;
      }
    }
    this.chart.note_series_update(this.id, typeof point.time !== "number");
    this.note_reference_hint(point);
    // Data arriving on a countdown-enabled series can start the timer (cheap flag check).
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  merge(point: series_merge_data, options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "merge");
    if (sequence === null) return;
    const time = time_to_utc_seconds(point.time);
    const timestamp_reason = timestamp_rejection_reason(time);
    if (timestamp_reason !== null) {
      this.last_ingestion = rejected_ingestion(
        timestamp_reason,
        undefined,
        Number.isFinite(time) ? 0 : 1,
        Number.isFinite(time) && Number.isInteger(time) ? 1 : 0,
      );
      console.warn(`aeris_charts: merge rejected — ${timestamp_reason}`);
      return;
    }
    // NaN is the wasm encoding of an absent channel, so a present field must be a finite number.
    const channels = [point.open, point.high, point.low, point.close ?? point.value];
    const limit = Number.MAX_SAFE_INTEGER / 100;
    for (const value of channels) {
      if (value === undefined) continue;
      if (typeof value !== "number" || !Number.isFinite(value) || Math.abs(value) > limit) {
        const non_finite = typeof value !== "number" || !Number.isFinite(value);
        this.last_ingestion = rejected_ingestion(
          "merge fields must be finite numbers within the supported range",
          undefined,
          non_finite ? 1 : 0,
          non_finite ? 0 : 1,
        );
        console.warn(`aeris_charts: merge rejected — ${this.last_ingestion.reason}`);
        return;
      }
    }
    const [open, high, low, close] = channels.map((value) => value ?? NaN) as [number, number, number, number];
    const body = point.color === undefined ? undefined : point_color_to_u32(point.color);
    const wick = point.wick_color === undefined ? undefined : point_color_to_u32(point.wick_color);
    const border = point.border_color === undefined ? undefined : point_color_to_u32(point.border_color);
    const json = this.chart.wasm.merge_series_bar(
      this.id, time, open, high, low, close, body, wick, border, sequence,
    );
    if (!this.record_ingestion(json)) {
      this.warn_rejected("merge");
      return;
    }
    this.chart.note_series_update(this.id, typeof point.time !== "number");
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  /**
   * Columnar partial merge: row `i` merges exactly like `merge({ time: times[i], ... })`, with
   * `NaN` entries and omitted columns absent. Rows apply in input order (a later row for the same
   * time merges into the earlier result) with one engine synchronization and one
   * `data_changed("update")`; any invalid row rejects the whole batch.
   */
  merge_typed(columns: series_merge_columns, options?: series_update_options): void {
    this.assert_live();
    const sequence = this.streaming_sequence(options, "merge_typed");
    if (sequence === null) return;
    const json = this.chart.wasm.merge_series_bars_typed(
      this.id, columns.times, columns.open, columns.high, columns.low, columns.close, sequence,
    );
    if (!this.record_ingestion(json)) {
      this.warn_rejected("merge_typed");
      return;
    }
    if (columns.times.length > 0) this.chart.note_series_update(this.id, false);
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  /** A dropped streaming call is never silent; stale deliveries are expected and stay quiet. */
  private warn_rejected(call: string): void {
    if (this.last_ingestion?.code === "stale_sequence") return;
    console.warn(`aeris_charts: ${call} rejected — ${this.last_ingestion?.reason ?? "invalid data"}`);
  }

  /**
   * Reference `update` semantics that silently rewrite a bar get an accepted diagnostic pointing
   * at `merge()`: a value-only point flattens a candlestick/bar, and a price-less payload such as
   * `{ time, volume }` becomes whitespace. Each code warns once per handle.
   */
  private note_reference_hint(point: series_data): void {
    let code: ingestion_diagnostics["code"];
    let reason: string;
    if ("value" in point) {
      const kind = KIND_NAMES[this.chart.wasm.series_kind(this.id) ?? KIND_TO_U8[this.kind]];
      if (kind !== "candlestick" && kind !== "bar") return;
      code = "value_on_ohlc_series";
      reason = "{ time, value } replaced the OHLC bar with a flat O=H=L=C bar; use merge({ time, close }) to update only the close";
    } else if (!OHLC_POINT_KEYS.some((key) => key in point)) {
      const extra = Object.keys(point).filter((key) => !NON_PRICE_POINT_KEYS.has(key));
      if (extra.length === 0) return;
      code = "price_less_payload";
      reason = `a point without price fields (${extra.join(", ")}) replaced the bar with whitespace; use merge() for partial ticks and update volume on its own series`;
    } else {
      return;
    }
    this.last_ingestion = {
      ...(this.last_ingestion ?? {
        status: "accepted_with_diagnostics",
        accepted: 1,
        dropped_invalid: 0,
        dropped_non_finite: 0,
        dropped_out_of_range: 0,
        deduplicated: 0,
        reordered: false,
        semantic_anomalies: 0,
      }),
      code,
      reason,
    };
    if (!this.warned_codes.has(code)) {
      this.warned_codes.add(code);
      console.warn(`aeris_charts: update() ${reason}`);
    }
  }

  last_ingestion_diagnostics(): ingestion_diagnostics | null {
    this.assert_live();
    return this.last_ingestion;
  }

  protected record_ingestion(json: string | undefined): boolean {
    this.last_ingestion = json === undefined ? null : JSON.parse(json) as ingestion_diagnostics;
    return this.last_ingestion?.status !== "rejected";
  }

  private record_single_ingestion(time: number, values: [number, number, number, number]): boolean {
    const whitespace = values.every(Number.isNaN);
    const timestamp_reason = timestamp_rejection_reason(time);
    const value_non_finite = !whitespace && values.some((value) => !Number.isFinite(value));
    const non_finite = timestamp_reason !== null || value_non_finite;
    const limit = Number.MAX_SAFE_INTEGER / 100;
    const out_of_range = !non_finite
      && !whitespace
      && values.some((value) => Math.abs(value) > limit);
    const [open, high, low, close] = values;
    const semantic = !whitespace && !non_finite && !out_of_range
      && (high < low || high < open || high < close || low > open || low > close);
    if (!non_finite && !out_of_range && !semantic) {
      this.last_ingestion = null;
      return true;
    }
    this.last_ingestion = {
      status: non_finite || out_of_range ? "rejected" : "accepted_with_diagnostics",
      accepted: semantic ? 1 : 0,
      dropped_invalid: non_finite || out_of_range ? 1 : 0,
      dropped_non_finite: value_non_finite || !Number.isFinite(time) ? 1 : 0,
      dropped_out_of_range: out_of_range
        || (timestamp_reason !== null && Number.isInteger(time)) ? 1 : 0,
      deduplicated: 0,
      reordered: false,
      semantic_anomalies: semantic ? 1 : 0,
      ...(timestamp_reason === null ? {} : { reason: timestamp_reason }),
    };
    return semantic;
  }

  pop(count = 1): void {
    this.assert_live();
    // Popping is a data-path write: an engine-owned series keeps its rows, so this records the
    // rejection like any other dropped write instead of repainting or firing `data_changed`.
    if (this.chart.wasm.series_is_derived(this.id)) {
      this.last_ingestion = derived_series_ingestion(this.id);
      this.warn_rejected("pop");
      return;
    }
    this.chart.wasm.series_pop(this.id, count);
    this.chart.repaint();
    // Like set_data, popping is a full-range change, not an incremental update.
    for (const handler of this.data_changed_subs) handler("full");
  }

  last_value_data(global_last = false): last_value_data | null {
    this.assert_live();
    const json = this.chart.wasm.series_last_value_data(this.id, global_last);
    return json === "" ? null : JSON.parse(json) as last_value_data;
  }

  price_formatter(): (price: number) => string {
    this.assert_live();
    const wasm = this.chart.wasm;
    const id = this.id;
    return (price: number) => wasm.series_format_price(id, price);
  }

  apply_options(options: Partial<any_series_options>): void {
    this.assert_live();
    const rule = (options as Partial<series_options>).histogram_updown_rule;
    if (rule !== undefined && !HISTOGRAM_UPDOWN_RULES.includes(rule)) {
      throw new AerisChartsError("invalid_options", `histogram_updown_rule must be "open_close" or "previous_close"`);
    }
    // First, so a rejected alignment leaves every other option of the call unapplied. A changed
    // alignment may change the chart's time points, which is a full-range change for handlers;
    // re-applying the current alignment (a React re-render) notifies nothing.
    if (apply_series_time_alignment(this.chart.wasm, this.id, options)) {
      for (const handler of this.data_changed_subs) handler("full");
    }
    if (options.max_points !== undefined) {
      // 0 (and anything below 1) clears the cap back to unbounded, matching the option's docs.
      this.chart.wasm.set_series_max_points(this.id, options.max_points);
      // Applying a cap may evict points, so this is a full-range data change even though it
      // arrived as an option. Emitted unconditionally rather than only when rows actually left:
      // over-notifying is safe (handlers re-read), and this is a configuration call, not a hot
      // path, so it is not worth a round trip to find out.
      this.chart.repaint();
      for (const handler of this.data_changed_subs) handler("full");
    }
    if (options.color !== undefined) {
      // CSS string passed through verbatim so the engine keeps any alpha channel.
      this.chart.wasm.set_series_color_css(this.id, options.color);
    }
    if (options.visible !== undefined) {
      this.chart.wasm.set_series_visible(this.id, options.visible);
    }
    if (options.render_before_time !== undefined) {
      this.chart.wasm.set_series_render_before_time(
        this.id,
        options.render_before_time ?? undefined,
      );
    }
    if (options.up_color !== undefined || options.down_color !== undefined) {
      // Pass each direction through unchanged: undefined = keep, "" = clear, CSS = pin (alpha
      // preserved, so "transparent" yields a hollow body). A plain `?? ""` here would wrongly
      // reset the direction the caller left unspecified back to the engine default.
      this.chart.wasm.set_series_updown_colors(this.id, options.up_color, options.down_color);
    }
    if (options.wick_up_color !== undefined || options.wick_down_color !== undefined) {
      // Pass each direction through unchanged: undefined = keep, "" = clear (follow body), CSS = pin.
      // (A plain `?? ""` here would wrongly clear the direction the caller left unspecified.)
      this.chart.wasm.set_series_wick_colors(this.id, options.wick_up_color, options.wick_down_color);
    }
    if (options.border_up_color !== undefined || options.border_down_color !== undefined) {
      this.chart.wasm.set_series_border_colors(this.id, options.border_up_color, options.border_down_color);
    }
    if (options.wick_visible !== undefined) {
      this.chart.wasm.set_series_wick_visible(this.id, options.wick_visible);
    }
    if (options.border_visible !== undefined) {
      this.chart.wasm.set_series_border_visible(this.id, options.border_visible);
    }
    if (options.line_width !== undefined) {
      this.chart.wasm.set_series_line_width(this.id, options.line_width);
    }
    if (options.area_top_color !== undefined || options.area_bottom_color !== undefined) {
      this.chart.wasm.set_series_area_colors(this.id, options.area_top_color ?? "", options.area_bottom_color ?? "");
    }
    if (options.histogram_updown !== undefined) {
      this.chart.wasm.set_series_histogram_updown(this.id, options.histogram_updown);
    }
    if (options.line_type !== undefined) {
      this.chart.wasm.set_series_line_type(this.id, LINE_TYPE_TO_U8[options.line_type]);
    }
    if (options.point_markers !== undefined) {
      this.chart.wasm.set_series_point_markers(this.id, options.point_markers);
    }
    if (options.close_visible !== undefined) {
      this.chart.wasm.set_series_close_visible(this.id, options.close_visible);
    }
    if (options.baseline_value !== undefined) {
      this.chart.wasm.set_series_baseline(this.id, options.baseline_value);
    }
    const requested_scale = options.overlay
      ? ""
      : options.priceScaleId ?? options.price_scale_id;
    if (options.pane !== undefined || requested_scale !== undefined) {
      const pane = options.pane ?? this.pane_index();
      const scale = requested_scale ?? this.price_scale_id();
      if (!this.chart.wasm.try_set_series_pane_and_scale(
        this.id,
        pane,
        options.pane_stretch ?? 1,
        scale,
      )) {
        throw new AerisChartsError(
          "invalid_options",
          `price scale '${scale}' does not exist in pane ${pane}`,
        );
      }
      if (scale === "") {
        const m = options.scale_margins ?? { top: 0.8, bottom: 0 };
        this.chart.wasm.set_series_overlay(this.id, m.top, m.bottom);
      }
    }
    if (options.last_price_animation !== undefined) {
      this.chart.wasm.set_series_last_price_animation(this.id, options.last_price_animation);
      this.chart.sync_animation();
    }
    if (options.price_format !== undefined) {
      const pf = options.price_format;
      if (pf.type === "custom") {
        // The callback crosses into wasm; min_move (when given) rides the JSON patch.
        this.chart.wasm.set_series_price_formatter(this.id, pf.formatter);
        if (pf.min_move !== undefined) {
          this.chart.wasm.series_apply_price_format_json(
            this.id, JSON.stringify({ type: "custom", min_move: pf.min_move }),
          );
        }
      } else {
        // An omitted precision stays omitted so the engine derives it from `min_move`; a
        // `null` ladder survives serialization and clears the bands. The engine validates the
        // whole patch before changing anything, so a rejected ladder leaves the format intact.
        const applied = this.chart.wasm.series_apply_price_format_json(
          this.id,
          JSON.stringify({
            type: pf.type,
            precision: pf.precision,
            min_move: pf.min_move,
            tick_ladder: pf.tick_ladder,
          }),
        );
        if (!applied) {
          throw new AerisChartsError(
            "invalid_options",
            "price_format was rejected: tick_ladder needs 1 to 64 bands with finite non-negative, "
              + "strictly ascending `from` bounds, positive `min_move` ticks, precision 0-15, and "
              + "each bound on both adjacent bands' tick grids",
          );
        }
      }
    }
    if ("autoscale_info_provider" in options) {
      const provider = options.autoscale_info_provider;
      if (provider !== undefined && provider !== null && typeof provider !== "function") {
        throw new AerisChartsError("invalid_options", "autoscale_info_provider must be a function");
      }
      // The engine hands the adapter the series' own info; the user provider sees it through the
      // reference-style `base_implementation()` callback.
      const chart = this.chart;
      this.chart.wasm.set_series_autoscale_info_provider(
        this.id,
        typeof provider === "function"
          ? (base: autoscale_info | null) =>
            chart.run_render_callback("autoscale_info_provider", () => provider(() => base))
          : undefined,
      );
    }
    // Style keys without a dedicated setter go to the engine as a single JSON patch.
    const json_patch: Record<string, unknown> = {};
    for (const key of SERIES_JSON_OPTION_KEYS) {
      const value = options[key];
      if (value !== undefined) json_patch[key] = value;
    }
    if (Object.keys(json_patch).length > 0) {
      this.chart.wasm.series_apply_options_json(this.id, JSON.stringify(json_patch));
    }
    this.chart.sync_countdown_timer();
    this.chart.repaint();
  }

  /** Push the current bid/ask quotes (`bid_ask_visible` renders them); `null` hides a side. */
  set_bid_ask(bid: number | null, ask: number | null): void {
    this.assert_live();
    this.chart.wasm.set_series_bid_ask(this.id, bid ?? Number.NaN, ask ?? Number.NaN);
    this.chart.repaint();
  }

  options(): any_series_options {
    this.assert_live();
    return JSON.parse(this.chart.wasm.series_options_json(this.id)) as series_options;
  }

  set_type(kind: series_kind): void {
    this.assert_live();
    if (kind === "custom" || is_feature_series_kind(kind) || is_footprint_series_kind(kind)) {
      throw new AerisChartsError(
        "unsupported_operation",
        "set_type() only converts built-in series; remove and re-add custom or advanced series",
      );
    }
    if (!this.chart.wasm.set_series_kind(this.id, KIND_TO_U8[kind])) {
      throw new AerisChartsError("stale_handle", "this series has been removed from the chart");
    }
    this.chart.repaint();
  }

  move_to_pane(pane_index: number, stretch = 1): void {
    this.assert_live();
    if (!this.chart.wasm.try_set_series_pane(this.id, pane_index, stretch)) {
      throw new AerisChartsError(
        "invalid_options",
        `pane ${pane_index} does not contain price scale '${this.price_scale_id()}'`,
      );
    }
    this.chart.repaint();
  }

  create_price_line(options: price_line_options): price_line_api {
    this.assert_live();
    const rgb = parse_rgb(options.color ?? "#2196f3") ?? [0x21, 0x96, 0xf3];
    const style = LINE_STYLE_TO_U8[options.line_style ?? "solid"];
    const id = this.chart.wasm.create_price_line(
      this.id,
      options.price,
      rgb[0],
      rgb[1],
      rgb[2],
      options.line_width ?? 1,
      style,
      options.title ?? "",
    );
    // Extras the positional constructor doesn't take go through the JSON patch path.
    const extras: Partial<price_line_options> = {};
    if (options.line_visible !== undefined) extras.line_visible = options.line_visible;
    if (options.axis_label_visible !== undefined) extras.axis_label_visible = options.axis_label_visible;
    if (options.axis_label_color !== undefined) extras.axis_label_color = options.axis_label_color;
    if (options.axis_label_text_color !== undefined) extras.axis_label_text_color = options.axis_label_text_color;
    if (Object.keys(extras).length > 0) {
      this.chart.wasm.price_line_apply_options(id, JSON.stringify(extras));
    }
    this.chart.repaint();
    const chart = this.chart;
    return {
      id,
      remove() {
        chart.wasm.remove_price_line(id);
        chart.repaint();
      },
      apply_options(patch: Partial<price_line_options>) {
        chart.wasm.price_line_apply_options(id, JSON.stringify(patch));
        chart.repaint();
      },
      options() {
        return JSON.parse(chart.wasm.price_line_options_json(id)) as price_line_options;
      },
    };
  }

  set_markers(markers: readonly series_marker[], options?: Partial<series_marker_options>): void {
    this.assert_live();
    if (options?.auto_scale !== undefined) {
      this.chart.wasm.set_series_markers_auto_scale(this.id, options.auto_scale);
    }
    if (options?.z_order !== undefined) {
      const z_order = options.z_order === "aboveSeries" ? 1 : options.z_order === "top" ? 2 : 0;
      if (!this.chart.wasm.set_series_markers_z_order(this.id, z_order)) {
        throw new Error("Aeris rejected the series-marker z-order");
      }
    }
    // Normalize marker times to UTC seconds so business-day/string forms match their data points
    // (the engine's marker JSON expects a numeric time).
    const normalized = markers.map((mk) => ({ ...mk, time: time_to_utc_seconds(mk.time) }));
    if (!this.chart.wasm.set_series_markers(this.id, JSON.stringify(normalized))) {
      throw new Error("Aeris rejected invalid series markers");
    }
    this.chart.repaint();
  }

  price_scale(): price_scale_api {
    const pane = undef_to_null(this.chart.wasm.series_pane_index(this.id)) ?? 0;
    return new price_scale_impl(this.chart, pane, this.price_scale_id());
  }
  price_scale_id(): string {
    this.assert_live();
    return this.chart.wasm.series_price_scale_name(this.id);
  }
  move_to_price_scale(id: string): void {
    this.assert_live();
    if (!this.chart.wasm.set_series_price_scale_by_name(this.id, id)) {
      throw new AerisChartsError(
        "invalid_options",
        `price scale '${id}' does not exist in pane ${this.pane_index()}`,
      );
    }
    this.chart.repaint();
  }
  pane_index(): number {
    this.assert_live();
    return undef_to_null(this.chart.wasm.series_pane_index(this.id)) ?? 0;
  }
  price_to_coordinate(price: number): number | null {
    return undef_to_null(this.chart.wasm.series_price_to_coordinate(this.id, price));
  }
  coordinate_to_price(coordinate: number): number | null {
    return undef_to_null(this.chart.wasm.series_coordinate_to_price(this.id, coordinate));
  }
  bars_in_logical_range(range: logical_range): bars_info | null {
    const info = this.chart.wasm.series_bars_in_logical_range(this.id, range.from, range.to);
    if (info.length < 2) return null;
    return info.length === 4
      ? { bars_before: info[0]!, bars_after: info[1]!, from: info[2]!, to: info[3]! }
      : { bars_before: info[0]!, bars_after: info[1]! };
  }
  private unpack_point(values: Float64Array | number[], offset = 0): series_data | null {
    if (values.length < offset + 5) return null;
    const time = values[offset]!;
    const [o, h, l, c] = [values[offset + 1]!, values[offset + 2]!, values[offset + 3]!, values[offset + 4]!];
    // A whitespace row round-trips all-NaN; return it as `{time}` with no value keys.
    if (Number.isNaN(o) && Number.isNaN(h) && Number.isNaN(l) && Number.isNaN(c)) {
      return { time };
    }
    if (this.series_type() === "candlestick" || this.series_type() === "bar") {
      return { time, open: o, high: h, low: l, close: c };
    }
    return { time, value: c };
  }
  data_by_index(logical_index: number, mismatch_direction: mismatch_direction = 0): series_data | null {
    return this.unpack_point(
      this.chart.wasm.series_data_by_index(this.id, logical_index, mismatch_direction),
    );
  }
  data(): readonly series_data[] {
    const values = this.chart.wasm.series_data(this.id);
    const output: series_data[] = [];
    for (let offset = 0; offset + 4 < values.length; offset += 5) {
      const point = this.unpack_point(values, offset);
      if (point !== null) output.push(point);
    }
    return output;
  }
  series_type(): series_kind {
    const kind = this.chart.wasm.series_kind(this.id) ?? KIND_TO_U8[this.kind];
    return KIND_NAMES[kind] ?? "candlestick";
  }
  indicator_info(): indicator_info | null {
    const raw = JSON.parse(this.chart.wasm.series_indicator_info_json(this.id)) as
      | Omit<indicator_info, "source" | "volume_source" | "amount_source"> & { source: number; volume_source: number | null; amount_source: number | null }
      | null;
    if (raw === null) return null;
    return {
      binding_id: raw.binding_id,
      kind: raw.kind,
      parameters: raw.parameters,
      period: raw.period,
      deviation: raw.deviation,
      source: this.chart.series_handle(raw.source),
      source_input: raw.source_input,
      volume_source: raw.volume_source === null ? null : this.chart.series_handle(raw.volume_source),
      amount_source: raw.amount_source === null ? null : this.chart.series_handle(raw.amount_source),
      style: raw.style,
      output_name: raw.output_name,
      output_index: raw.output_index,
      output_count: raw.output_count,
      warmup_bars: raw.warmup_bars,
      convergence_bars: raw.convergence_bars,
    };
  }
  indicator_output_style(): indicator_output_style | null {
    return this.indicator_info()?.style ?? null;
  }
  set_indicator_output_style(patch: Partial<indicator_output_style>): boolean {
    const current = this.indicator_output_style();
    if (current === null) return false;
    const updated = this.chart.wasm.set_indicator_output_style(
      this.id,
      JSON.stringify({ ...current, ...patch }),
    );
    if (updated) this.chart.repaint();
    return updated;
  }
  subscribe_data_changed(handler: data_changed_handler): void {
    this.data_changed_subs.add(handler);
  }
  unsubscribe_data_changed(handler: data_changed_handler): void {
    this.data_changed_subs.delete(handler);
  }

  attach_primitive(primitive: series_primitive): series_primitive_handle {
    this.assert_live();
    // Bind the hooks the plugin actually implements into a plain object (the host reads own
    // properties; binding also pins `this` for class-instance primitives), then register.
    const adapted: Record<string, unknown> = {};
    for (const key of [
      "detached",
      "update_all_views",
      "pane_views",
      "price_axis_views",
      "time_axis_views",
      "text_views",
      "autoscale_info",
      "hit_test",
    ] as const) {
      const hook = primitive[key];
      if (typeof hook === "function") adapted[key] = hook.bind(primitive);
    }
    if (typeof primitive.attached === "function") {
      const attached = primitive.attached;
      // The host supplies `{series_id, pane_index}`; inject `request_update` (a repaint
      // scheduler, reference `requestUpdate`) before the plugin sees the params.
      adapted.attached = (params: { series_id: number; pane_index?: number; request_update?: () => void }) => {
        params.request_update = () => this.chart.repaint();
        attached.call(primitive, params);
      };
    }
    const id = this.chart.wasm.attach_series_primitive(this.id, adapted);
    this.chart.repaint();
    return new series_primitive_handle_impl(this.chart, id);
  }

  /** Package-internal boundary for the first-class Rust primitive helpers. */
  native_add_image_watermark(
    width: number,
    height: number,
    pixels: Uint8Array,
    options_json: string,
  ): number {
    this.assert_live();
    return this.chart.wasm.add_native_image_watermark(
      this.id,
      width,
      height,
      pixels,
      options_json,
    );
  }
  native_add_anchored_text(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_anchored_text(this.id, options_json);
  }
  native_set_anchored_text_options(id: number, options_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_anchored_text_options(id, options_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_bands_indicator(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_bands_indicator(this.id, options_json);
  }
  native_set_bands_indicator_options(primitive_id: number, options_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_bands_indicator_options(primitive_id, options_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_overlay_price_scale(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_overlay_price_scale(this.id, options_json);
  }
  native_set_overlay_price_scale_options(primitive_id: number, options_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_overlay_price_scale_options(primitive_id, options_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_accessibility_focus(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_accessibility_focus(this.id, options_json);
  }
  native_set_accessibility_focus(primitive_id: number, time: number | null, options_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_accessibility_focus(
      primitive_id,
      time ?? Number.NaN,
      options_json,
    );
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_session_highlighting(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_session_highlighting(this.id, options_json);
  }
  native_set_session_highlighting_data(primitive_id: number, highlights_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_session_highlighting_data(primitive_id, highlights_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_upsert_session_highlighting_data(primitive_id: number, highlights_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.upsert_native_session_highlighting_data(primitive_id, highlights_json);
    if (changed) this.chart.schedule_repaint();
    return changed;
  }
  /** Source times at or after `since` (UTC seconds), for incremental per-row host callbacks. */
  native_times_since(since: number): Float64Array {
    this.assert_live();
    return Float64Array.from(this.chart.wasm.series_times_since(this.id, since));
  }
  native_add_crosshair_highlight(color?: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_crosshair_highlight(this.id, color);
  }
  native_add_vertical_line(time: number, options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_vertical_line(this.id, time, options_json);
  }
  native_add_delta_tooltip(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_delta_tooltip(this.id, options_json);
  }
  native_set_area_brush_state(state_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_series_area_brush_state(this.id, state_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_tooltip(options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_tooltip(this.id, options_json);
  }
  native_set_tooltip_options(primitive_id: number, options_json: string): boolean {
    this.assert_live();
    const accepted = this.chart.wasm.set_native_tooltip_options(primitive_id, options_json);
    if (accepted) this.chart.repaint();
    return accepted;
  }
  native_tooltip_snapshot_json(primitive_id: number): string {
    this.assert_live();
    return this.chart.wasm.native_tooltip_snapshot_json(primitive_id);
  }
  native_delta_tooltip_active_range_json(primitive_id: number): string {
    this.assert_live();
    return this.chart.wasm.native_delta_tooltip_active_range_json(primitive_id);
  }
  native_clear_delta_tooltip(primitive_id: number): boolean {
    this.assert_live();
    const changed = this.chart.wasm.clear_native_delta_tooltip(primitive_id);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_add_trend_line(
    first_time: number,
    first_price: number,
    second_time: number,
    second_price: number,
    options_json: string,
  ): number {
    this.assert_live();
    return this.chart.wasm.add_native_trend_line(
      this.id,
      first_time,
      first_price,
      second_time,
      second_price,
      options_json,
    );
  }
  native_add_volume_profile(data_json: string, options_json: string): number {
    this.assert_live();
    return this.chart.wasm.add_native_volume_profile(this.id, data_json, options_json);
  }
  native_set_volume_profile_data(id: number, data_json: string): boolean {
    this.assert_live();
    const changed = this.chart.wasm.set_native_volume_profile_data(id, data_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_remove_primitive(id: number): void {
    if (this.chart.wasm.remove_native_primitive(id)) this.chart.repaint();
  }
  native_repaint(): void {
    this.chart.repaint();
  }
}

function assert_trading_result(json: string): void {
  const result = JSON.parse(json) as
    | { ok: true }
    | { ok: false; error: { code: AerisChartsErrorCode; message: string } };
  if (!result.ok) throw new AerisChartsError(result.error.code, result.error.message);
}

function parse_engine_result<T extends object>(json: string): T {
  const result = JSON.parse(json) as
    | ({ ok: true } & T)
    | { ok: false; error: { code: AerisChartsErrorCode; message: string } };
  if (!result.ok) throw new AerisChartsError(result.error.code, result.error.message);
  return result;
}

export interface native_primitive_handle {
  detach(): void;
}

export interface native_bands_indicator_handle extends native_primitive_handle {
  set_options_json(options_json: string): boolean;
}

export interface native_overlay_price_scale_handle extends native_primitive_handle {
  set_options_json(options_json: string): boolean;
}

export interface native_volume_profile_handle extends native_primitive_handle {
  set_data_json(data_json: string): boolean;
}

export interface native_accessibility_focus_handle extends native_primitive_handle {
  set(time: number | null, options_json: string): boolean;
}

export interface native_session_highlighting_handle extends native_primitive_handle {
  set_data_json(data_json: string): boolean;
  /** Merge records for rows appended or replaced at the tail; false when unaligned. */
  upsert_data_json(data_json: string): boolean;
  /** Source times at or after `since`. */
  times_since(since: number): Float64Array;
}

export interface native_anchored_text_handle extends native_primitive_handle {
  set_options_json(options_json: string): boolean;
}

export interface native_text_watermark_handle extends native_primitive_handle {
  set_options_json(options_json: string): boolean;
}

export interface native_delta_tooltip_handle extends native_primitive_handle {
  active_range_json(): string;
  clear(): boolean;
}

export interface native_tooltip_handle extends native_primitive_handle {
  set_options_json(options_json: string): boolean;
  snapshot_json(): string;
}

function native_series(series: series_api): series_impl {
  if (!(series instanceof series_impl)) {
    throw new AerisChartsError(
      "invalid_handle",
      "engine-owned primitives require a series created by this Aeris chart",
    );
  }
  return series;
}

/** Package-internal controller for transient brush styling on a built-in Area series. */
export function set_native_area_brush_state(series: series_api, state_json: string): boolean {
  return native_series(series).native_set_area_brush_state(state_json);
}

function native_handle(series: series_impl, id: number): native_primitive_handle {
  if (id === 0) throw new AerisChartsError("invalid_data", "engine rejected native primitive data or options");
  series.native_repaint();
  let attached = true;
  return {
    detach() {
      if (!attached) return;
      attached = false;
      series.native_remove_primitive(id);
    },
  };
}

export function attach_native_bands_indicator(
  series: series_api,
  options_json: string,
): native_bands_indicator_handle {
  const owner = native_series(series);
  const id = owner.native_add_bands_indicator(options_json);
  const base = native_handle(owner, id);
  return {
    set_options_json(next) {
      return owner.native_set_bands_indicator_options(id, next);
    },
    detach: base.detach,
  };
}

export function attach_native_overlay_price_scale(
  series: series_api,
  options_json: string,
): native_overlay_price_scale_handle {
  const owner = native_series(series);
  const id = owner.native_add_overlay_price_scale(options_json);
  const base = native_handle(owner, id);
  return {
    set_options_json(next) {
      return owner.native_set_overlay_price_scale_options(id, next);
    },
    detach: base.detach,
  };
}

export function attach_native_accessibility_focus(
  series: series_api,
  options_json: string,
): native_accessibility_focus_handle {
  const owner = native_series(series);
  const id = owner.native_add_accessibility_focus(options_json);
  const base = native_handle(owner, id);
  return {
    detach: base.detach,
    set(time, next_options_json) {
      return owner.native_set_accessibility_focus(id, time, next_options_json);
    },
  };
}

/** Exchange-time text for a chart timestamp (package-owned DOM surfaces). */
export function chart_time_text(chart: chart_api, seconds: number): string {
  if (chart instanceof chart_impl) return chart.format_time_text(seconds);
  return format_exchange_seconds(seconds, undefined, seconds % 86_400 !== 0, false);
}

export function attach_native_session_highlighting(
  series: series_api,
  options_json: string,
): native_session_highlighting_handle {
  const owner = native_series(series);
  const id = owner.native_add_session_highlighting(options_json);
  const base = native_handle(owner, id);
  return {
    detach: base.detach,
    set_data_json(data_json) {
      return owner.native_set_session_highlighting_data(id, data_json);
    },
    upsert_data_json(data_json) {
      return owner.native_upsert_session_highlighting_data(id, data_json);
    },
    times_since(since) {
      return owner.native_times_since(since);
    },
  };
}

export function attach_native_crosshair_highlight(
  series: series_api,
  color?: string,
): native_primitive_handle {
  const owner = native_series(series);
  return native_handle(owner, owner.native_add_crosshair_highlight(color));
}

export function attach_native_vertical_line(
  series: series_api,
  time: number,
  options_json: string,
): native_primitive_handle {
  const owner = native_series(series);
  return native_handle(owner, owner.native_add_vertical_line(time, options_json));
}

export function attach_native_delta_tooltip(
  series: series_api,
  options_json: string,
): native_delta_tooltip_handle {
  const owner = native_series(series);
  const id = owner.native_add_delta_tooltip(options_json);
  const base = native_handle(owner, id);
  return {
    active_range_json() {
      return owner.native_delta_tooltip_active_range_json(id);
    },
    clear() {
      return owner.native_clear_delta_tooltip(id);
    },
    detach: base.detach,
  };
}

export function attach_native_tooltip(
  series: series_api,
  options_json: string,
): native_tooltip_handle {
  const owner = native_series(series);
  const id = owner.native_add_tooltip(options_json);
  const base = native_handle(owner, id);
  return {
    set_options_json(next) {
      return owner.native_set_tooltip_options(id, next);
    },
    snapshot_json() {
      return owner.native_tooltip_snapshot_json(id);
    },
    detach: base.detach,
  };
}

export function attach_native_trend_line(
  series: series_api,
  first_time: number,
  first_price: number,
  second_time: number,
  second_price: number,
  options_json: string,
): native_primitive_handle {
  const owner = native_series(series);
  return native_handle(owner, owner.native_add_trend_line(
    first_time,
    first_price,
    second_time,
    second_price,
    options_json,
  ));
}

export function attach_native_volume_profile(
  series: series_api,
  data_json: string,
  options_json: string,
): native_volume_profile_handle {
  const owner = native_series(series);
  const id = owner.native_add_volume_profile(data_json, options_json);
  const base = native_handle(owner, id);
  return {
    detach: base.detach,
    set_data_json(next) {
      return owner.native_set_volume_profile_data(id, next);
    },
  };
}

function native_pane(pane: pane_api): pane_impl {
  if (!(pane instanceof pane_impl)) {
    throw new AerisChartsError(
      "invalid_handle",
      "engine-owned primitives require a pane created by this Aeris chart",
    );
  }
  return pane;
}

export function attach_native_image_watermark(
  series: series_api,
  width: number,
  height: number,
  pixels: Uint8Array,
  options_json: string,
): native_primitive_handle {
  const owner = native_series(series);
  const id = owner.native_add_image_watermark(width, height, pixels, options_json);
  if (id === 0) throw new AerisChartsError("invalid_data", "engine rejected image watermark data or options");
  owner.native_repaint();
  let attached = true;
  return {
    detach() {
      if (!attached) return;
      attached = false;
      owner.native_remove_primitive(id);
    },
  };
}

export function attach_native_anchored_text(
  series: series_api,
  options_json: string,
): native_anchored_text_handle {
  const owner = native_series(series);
  const id = owner.native_add_anchored_text(options_json);
  if (id === 0) throw new AerisChartsError("invalid_data", "engine rejected anchored-text options");
  owner.native_repaint();
  let attached = true;
  return {
    set_options_json(next) {
      if (!attached) return false;
      return owner.native_set_anchored_text_options(id, next);
    },
    detach() {
      if (!attached) return;
      attached = false;
      owner.native_remove_primitive(id);
    },
  };
}

export function attach_native_text_watermark(
  pane: pane_api,
  options_json: string,
): native_text_watermark_handle {
  const owner = native_pane(pane);
  const id = owner.native_add_text_watermark(options_json);
  if (id === 0) throw new AerisChartsError("invalid_data", "engine rejected text-watermark options");
  owner.native_repaint();
  let attached = true;
  return {
    set_options_json(next) {
      if (!attached) return false;
      return owner.native_set_text_watermark_options(id, next);
    },
    detach() {
      if (!attached) return;
      attached = false;
      owner.native_remove_primitive(id);
    },
  };
}

/**
 * The handle for a custom series (plugin platform Phase C-c; the reference's `ISeriesApi<'Custom'>`).
 * Shares the built-in handle's options/coordinate/primitive surface; the data methods work on
 * the raw plugin items (the engine rows carry times only, so `data()`/`data_by_index()` come
 * from the host's aligned item store, and `last_value_data` resolves through the engine's
 * host-recorded frame values).
 */
class custom_series_impl extends series_impl {
  constructor(id: number, chart: chart_impl) {
    super(id, "custom", chart);
  }

  /** Replace the series' items (reference `setData`). Times convert to UTC seconds here (the same
   *  boundary conversion as the built-ins); sort/dedupe happens engine-side with the items
   *  carried along, so `data()` returns the aligned raw items. */
  set_data(data: readonly custom_series_item[], options?: series_update_options): void {
    this.assert_live();
    reject_sequence_option(options, "custom series");
    const converted = data.map((item) => ({ ...item, time: time_to_utc_seconds(item.time) }));
    if (!this.record_ingestion(this.chart.wasm.set_custom_series_data(this.id, converted))) return;
    this.chart.note_series_times(this.id, calendar_input(data));
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  /** Append a new item or replace the one at an existing time (reference `update`). */
  update(item: custom_series_item, options?: series_update_options): void {
    this.assert_live();
    reject_sequence_option(options, "custom series");
    if (!this.record_ingestion(this.chart.wasm.update_custom_series_item(
      this.id,
      { ...item, time: time_to_utc_seconds(item.time) },
    ))) return;
    this.chart.note_series_update(this.id, typeof item.time !== "number");
    // Custom series compute their price values through the JS pane view during render,
    // so this path must stay synchronous: last_value_data and friends read that
    // render-computed state immediately after an update.
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  merge(): void {
    // Custom items are opaque plugin payloads; there are no engine-owned fields to merge.
    this.assert_live();
    throw new AerisChartsError("unsupported_operation", "merge() does not apply to a custom series");
  }

  merge_typed(): void {
    this.merge();
  }

  set_data_typed(): void {
    // A custom series carries raw plugin items aligned by time, not OHLC columns.
    this.assert_live();
    throw new AerisChartsError("unsupported_operation", "set_data_typed() does not apply to a custom series");
  }

  update_typed(): void {
    // Same reason as `set_data_typed`: no OHLC columns to append.
    this.assert_live();
    throw new AerisChartsError("unsupported_operation", "update_typed() does not apply to a custom series");
  }

  set_ring_source(): void {
    // A ring carries OHLC rows; a custom series' values live in its host-side pane view.
    this.assert_live();
    throw new AerisChartsError("unsupported_operation", "set_ring_source() does not apply to a custom series");
  }

  /** The raw items aligned with the engine rows (sorted, last-wins deduped). */
  data(): readonly custom_series_item[] {
    this.assert_live();
    return this.chart.wasm.custom_series_data(this.id) as custom_series_item[];
  }

  data_by_index(logical_index: number, mismatch_direction: mismatch_direction = 0): custom_series_item | null {
    this.assert_live();
    return undef_to_null(
      this.chart.wasm.custom_series_data_by_index(this.id, logical_index, mismatch_direction),
    ) as custom_series_item | null;
  }

  series_type(): series_kind {
    return "custom";
  }

  set_type(): void {
    // A custom series' type IS the pane view; change it by removing and re-adding the series.
    this.assert_live();
    throw new AerisChartsError("unsupported_operation", "set_type() does not apply to a custom series");
  }
}

/** A public handle whose payload and renderer are both owned by the Rust feature-series state. */
class feature_series_impl extends series_impl {
  private heatmap_shader: ((amount: number) => string) | null = null;

  constructor(id: number, kind: feature_series_kind, chart: chart_impl) {
    super(id, kind, chart);
  }

  private engine_item(item: series_data): series_data {
    if (this.kind !== "heatmap" || this.heatmap_shader === null || !("cells" in item)) return item;
    const shader = this.heatmap_shader;
    return {
      ...item,
      cells: item.cells.map((cell) => ({ ...cell, color: shader(cell.amount) })),
    } as series_data;
  }

  set_data(data: readonly series_data[], options?: series_update_options): void {
    this.assert_live();
    reject_sequence_option(options, "advanced series");
    const converted = data.map((item) => this.engine_item({
      ...item,
      time: time_to_utc_seconds(item.time),
    } as series_data));
    if (!this.record_ingestion(this.chart.wasm.set_feature_series_data(this.id, converted))) return;
    this.chart.note_series_times(this.id, calendar_input(data));
    this.chart.sync_countdown_timer();
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  update(item: series_data, options?: series_update_options): void {
    this.assert_live();
    reject_sequence_option(options, "advanced series");
    if (!this.record_ingestion(this.chart.wasm.update_feature_series_item(
      this.id,
      this.engine_item({ ...item, time: time_to_utc_seconds(item.time) } as series_data),
    ))) return;
    this.chart.note_series_update(this.id, typeof item.time !== "number");
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler("update");
  }

  merge(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "merge() does not apply to structured advanced-series payloads",
    );
  }

  merge_typed(): void {
    this.merge();
  }

  set_data_typed(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "set_data_typed() does not apply to structured advanced-series payloads",
    );
  }

  update_typed(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "update_typed() does not apply to structured advanced-series payloads",
    );
  }

  set_ring_source(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "set_ring_source() does not apply to structured advanced-series payloads",
    );
  }

  data(): readonly series_data[] {
    this.assert_live();
    return this.chart.wasm.feature_series_data(this.id) as series_data[];
  }

  data_by_index(logical_index: number, mismatch_direction: mismatch_direction = 0): series_data | null {
    this.assert_live();
    return undef_to_null(
      this.chart.wasm.feature_series_data_by_index(this.id, logical_index, mismatch_direction),
    ) as series_data | null;
  }

  apply_options(options: Partial<any_series_options>): void {
    const { cell_shader, ...engine_options } = options;
    const shader_changed = this.kind === "heatmap" && cell_shader !== undefined;
    const current_data = shader_changed ? this.data() : [];
    if (shader_changed) {
      if (typeof cell_shader !== "function") {
        throw new AerisChartsError("invalid_options", "cell_shader must be a function");
      }
      this.heatmap_shader = cell_shader;
    }
    super.apply_options(engine_options);
    this.chart.wasm.apply_feature_series_options(this.id, JSON.stringify(engine_options));
    if (current_data.length > 0) {
      this.record_ingestion(this.chart.wasm.set_feature_series_data(
        this.id,
        current_data.map((item) => this.engine_item(item)),
      ));
    }
    this.chart.repaint();
  }

  options(): any_series_options {
    const options = {
      ...super.options(),
      ...(JSON.parse(this.chart.wasm.feature_series_options_json(this.id)) as Partial<any_series_options>),
    };
    return this.heatmap_shader === null ? options : { ...options, cell_shader: this.heatmap_shader };
  }

  series_type(): feature_series_kind {
    return this.kind as feature_series_kind;
  }

  set_type(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "set_type() does not change an advanced series schema; remove and re-add the series",
    );
  }
}

function footprint_side(side: footprint_trade["aggressor"], index = 0): number {
  if (side === undefined || side === "unknown") return 0;
  if (side === "buy") return 1;
  if (side === "sell") return 2;
  throw new AerisChartsError(
    "invalid_data",
    `invalid footprint trade at index ${index}: aggressor must be 'buy', 'sell', or 'unknown'`,
  );
}

function pack_footprint_trades(trades: readonly footprint_trade[]): footprint_trade_columns {
  const length = trades.length;
  const columns: footprint_trade_columns = {
    timestamps_micros: new Float64Array(length),
    prices: new Float64Array(length),
    volumes: new Float64Array(length),
    aggressors: new Uint8Array(length),
    bids: new Float64Array(length).fill(Number.NaN),
    asks: new Float64Array(length).fill(Number.NaN),
    sequences: new Float64Array(length).fill(Number.NaN),
    trade_ids: new Float64Array(length).fill(Number.NaN),
    conditions: new Uint32Array(length),
    session_ids: new Float64Array(length).fill(Number.NaN),
  };
  for (let index = 0; index < length; index += 1) {
    const trade = trades[index]!;
    columns.timestamps_micros[index] = trade.timestamp_micros;
    columns.prices[index] = trade.price;
    columns.volumes[index] = trade.volume;
    columns.aggressors[index] = footprint_side(trade.aggressor, index);
    if (trade.bid !== undefined) columns.bids[index] = trade.bid;
    if (trade.ask !== undefined) columns.asks[index] = trade.ask;
    if (trade.sequence !== undefined) columns.sequences[index] = trade.sequence;
    if (trade.trade_id !== undefined) columns.trade_ids[index] = trade.trade_id;
    if (trade.conditions !== undefined) columns.conditions[index] = trade.conditions;
    if (trade.session_id !== undefined) columns.session_ids[index] = trade.session_id;
  }
  return columns;
}

function throw_footprint_error(result: string): never {
  let message = result;
  try {
    const parsed = JSON.parse(result) as { error?: string };
    message = parsed.error ?? result;
  } catch {
    // Preserve the engine string when it is not an error envelope.
  }
  throw new AerisChartsError("invalid_data", message);
}

/** A public handle whose authoritative payload is a raw trade tape in the Rust engine. */
class footprint_series_impl extends series_impl implements footprint_series_api {
  constructor(id: number, chart: chart_impl) {
    super(id, "footprint", chart);
  }

  set_trades(trades: readonly footprint_trade[]): void {
    this.set_trades_typed(pack_footprint_trades(trades));
  }

  set_trades_typed(columns: footprint_trade_columns): void {
    this.assert_live();
    const result = this.chart.wasm.set_footprint_trades_typed(
      this.id,
      columns.timestamps_micros,
      columns.prices,
      columns.volumes,
      columns.aggressors,
      columns.bids,
      columns.asks,
      columns.sequences,
      columns.trade_ids,
      columns.conditions,
      columns.session_ids,
    );
    if (result !== "") throw_footprint_error(result);
    this.chart.sync_countdown_timer();
    this.chart.repaint();
    for (const handler of this.data_changed_subs) handler("full");
  }

  update_trades(trades: readonly footprint_trade[]): "tip" | "historical" {
    return this.update_trades_typed(pack_footprint_trades(trades));
  }

  update_trades_typed(columns: footprint_trade_columns): "tip" | "historical" {
    this.assert_live();
    const result = this.chart.wasm.update_footprint_trades_typed(
      this.id,
      columns.timestamps_micros,
      columns.prices,
      columns.volumes,
      columns.aggressors,
      columns.bids,
      columns.asks,
      columns.sequences,
      columns.trade_ids,
      columns.conditions,
      columns.session_ids,
    );
    if (result !== "tip" && result !== "historical") throw_footprint_error(result);
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler(result === "tip" ? "update" : "full");
    return result;
  }

  update_trade(trade: footprint_trade): "tip" | "historical" {
    this.assert_live();
    const result = this.chart.wasm.update_footprint_trade_typed(
      this.id,
      trade.timestamp_micros,
      trade.price,
      trade.volume,
      footprint_side(trade.aggressor),
      trade.bid ?? Number.NaN,
      trade.ask ?? Number.NaN,
      trade.sequence ?? Number.NaN,
      trade.trade_id ?? Number.NaN,
      trade.conditions ?? 0,
      trade.session_id ?? Number.NaN,
    );
    if (result !== "tip" && result !== "historical") throw_footprint_error(result);
    if (this.chart.countdown_series_present) this.chart.sync_countdown_timer();
    this.chart.schedule_repaint();
    for (const handler of this.data_changed_subs) handler(result === "tip" ? "update" : "full");
    return result;
  }

  footprint_bars(): readonly footprint_bar[] {
    this.assert_live();
    return JSON.parse(this.chart.wasm.footprint_bars_json(this.id)) as footprint_bar[];
  }

  footprint_bar(index: number): footprint_bar | null {
    this.assert_live();
    return JSON.parse(this.chart.wasm.footprint_bar_json(this.id, index)) as footprint_bar | null;
  }

  set_data(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "footprint series require set_trades() or set_trades_typed(); OHLC data cannot supply order-flow truth",
    );
  }

  set_data_typed(): void {
    this.set_data();
  }

  update(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "footprint series require update_trade(); OHLC updates cannot supply order-flow truth",
    );
  }

  update_typed(): void {
    this.update();
  }

  merge(): void {
    this.update();
  }

  merge_typed(): void {
    this.update();
  }

  set_ring_source(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "the OHLC shared ring does not apply to footprint trades",
    );
  }

  pop(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "pop() cannot remove a derived footprint bar independently of its trade tape",
    );
  }

  apply_options(options: Partial<any_series_options> & Partial<footprint_series_options>): void {
    this.assert_live();
    super.apply_options(options);
    const current = JSON.parse(this.chart.wasm.footprint_options_json(this.id)) as footprint_series_options;
    const result = this.chart.wasm.apply_footprint_options(
      this.id,
      JSON.stringify({ ...current, ...options }),
    );
    if (result !== "") throw_footprint_error(result);
    this.chart.repaint();
  }

  options(): any_series_options & footprint_series_options {
    return {
      ...super.options(),
      ...(JSON.parse(this.chart.wasm.footprint_options_json(this.id)) as footprint_series_options),
    };
  }

  series_type(): "footprint" {
    return "footprint";
  }

  set_type(): void {
    this.assert_live();
    throw new AerisChartsError(
      "unsupported_operation",
      "set_type() does not change a footprint trade schema; remove and re-add the series",
    );
  }
}

class time_scale_impl implements time_scale_api {
  fitContent(): void { this.fit_content(); }
  scrollToRealTime(): void { this.scroll_to_real_time(); }
  setVisibleRange(...args: Parameters<time_scale_api["set_visible_range"]>): void { this.set_visible_range(...args); }
  getVisibleRange(): time_range | null { return this.get_visible_range(); }
  setVisibleLogicalRange(...args: Parameters<time_scale_api["set_visible_logical_range"]>): void { this.set_visible_logical_range(...args); }
  getVisibleLogicalRange(): logical_range | null { return this.get_visible_logical_range(); }
  constructor(private readonly chart: chart_impl) {}

  scroll_position(): number {
    return this.chart.wasm.scroll_position();
  }
  /** Invalidate any in-flight animated scroll (a new scroll call or user gesture takes over). */
  cancel_scroll_animation(): void {
    this.chart.wasm.cancel_scroll_animation();
    this.chart.wasm.cancel_keyboard_scroll();
  }
  scroll_to_position(position: number, animated: boolean): void {
    if (!animated || this.chart.prefers_reduced_motion()) {
      this.cancel_scroll_animation();
      this.chart.wasm.scroll_to_position(position);
      this.chart.repaint();
      return;
    }
    // The engine owns the cubic ease-out easing and applies every tick; this RAF loop is just
    // host-side frame scheduling (a newer scroll or a user gesture supersedes engine-side).
    if (this.chart.wasm.scroll_position() === position) {
      this.cancel_scroll_animation();
      return;
    }
    this.chart.wasm.cancel_keyboard_scroll();
    this.chart.wasm.start_scroll_animation(position, SCROLL_ANIM_MS, performance.now());
    this.drive_scroll_animation();
  }
  /** Host frame scheduling for the engine-owned scroll animation (a newer scroll or a user
   *  gesture supersedes it engine-side, which ends this loop). */
  private drive_scroll_animation(): void {
    const step = () => {
      // A chart removed mid-animation has freed its engine; the loop simply ends.
      if (this.chart.is_removed()) return;
      const done = Number.isNaN(this.chart.wasm.scroll_animation_tick(performance.now()));
      this.chart.repaint();
      if (!done) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }
  scroll_to_real_time(): void {
    // reference `scrollToRealTime` animates to `options.rightOffset` (time-scale.ts:824-826).
    if (this.chart.prefers_reduced_motion()) {
      this.cancel_scroll_animation();
      this.chart.wasm.scroll_to_real_time();
      this.chart.repaint();
      return;
    }
    this.chart.wasm.cancel_keyboard_scroll();
    this.chart.wasm.start_real_time_scroll_animation(REAL_TIME_SCROLL_ANIM_MS, performance.now());
    this.drive_scroll_animation();
  }
  reset_time_scale(): void {
    this.cancel_scroll_animation();
    this.chart.wasm.reset_time_scale();
    this.chart.repaint();
  }
  fit_content(): void {
    this.cancel_scroll_animation();
    this.chart.wasm.fit_content();
    this.chart.repaint();
  }
  apply_options(options: Partial<time_scale_options>): void {
    // Exchange time and explicit marks validate first, together, so a rejected zone or mark list
    // leaves every other option untouched.
    if (
      options.time_zone !== undefined || options.session_start !== undefined ||
      options.tick_marks !== undefined || options.bar_time_label !== undefined
    ) {
      this.chart.apply_exchange_time(
        options.time_zone,
        options.session_start,
        options.tick_marks,
        options.bar_time_label,
      );
    }
  if (options.bar_spacing !== undefined) this.chart.wasm.apply_bar_spacing_option(options.bar_spacing);
  if (options.right_offset !== undefined) this.chart.wasm.apply_right_offset_option(options.right_offset);
    if (options.min_bar_spacing !== undefined) this.chart.wasm.set_min_bar_spacing(options.min_bar_spacing);
    if (options.max_bar_spacing !== undefined) this.chart.wasm.set_max_bar_spacing(options.max_bar_spacing);
    if (options.right_offset_pixels !== undefined) this.chart.wasm.set_right_offset_pixels(options.right_offset_pixels);
    if (options.time_visible !== undefined) this.chart.wasm.set_time_visible(options.time_visible);
    if (options.seconds_visible !== undefined) this.chart.wasm.set_seconds_visible(options.seconds_visible);
    if (options.fix_left_edge !== undefined) this.chart.wasm.set_fix_left_edge(options.fix_left_edge);
    if (options.fix_right_edge !== undefined) this.chart.wasm.set_fix_right_edge(options.fix_right_edge);
    if (options.lock_visible_time_range_on_resize !== undefined)
      this.chart.wasm.set_lock_visible_time_range_on_resize(options.lock_visible_time_range_on_resize);
    if (options.right_bar_stays_on_scroll !== undefined)
      this.chart.wasm.set_right_bar_stays_on_scroll(options.right_bar_stays_on_scroll);
    if (options.lock_visible_logical_range !== undefined)
      this.chart.wasm.set_lock_visible_logical_range(options.lock_visible_logical_range);
    if (options.shift_visible_range_on_new_bar !== undefined)
      this.chart.wasm.set_shift_visible_range_on_new_bar(options.shift_visible_range_on_new_bar);
    if (options.allow_shift_visible_range_on_whitespace_replacement !== undefined)
      this.chart.wasm.set_allow_shift_visible_range_on_whitespace_replacement(
        options.allow_shift_visible_range_on_whitespace_replacement,
      );
    if (options.allow_bold_labels !== undefined)
      this.chart.wasm.set_allow_bold_labels(options.allow_bold_labels);
    if (options.ticks_visible !== undefined) this.chart.wasm.set_time_ticks_visible(options.ticks_visible);
    if (options.minimum_height !== undefined) this.chart.wasm.set_time_axis_minimum_height(options.minimum_height);
    if (options.tick_mark_max_character_length !== undefined)
      this.chart.wasm.set_tick_mark_max_character_length(options.tick_mark_max_character_length);
    if (options.visible !== undefined) this.chart.wasm.set_time_axis_visible(options.visible);
    if (options.tick_mark_formatter !== undefined)
      this.chart.set_tick_mark_formatter(options.tick_mark_formatter);
    this.chart.repaint();
  }
  options(): time_scale_options {
    const options = JSON.parse(this.chart.wasm.time_scale_options_json()) as time_scale_options;
    const name = this.chart.time_zone_name(options.time_zone);
    if (name !== null) options.time_zone = name;
    return options;
  }
  get_visible_logical_range(): logical_range | null {
    const r = this.chart.wasm.visible_logical_range();
    return r.length === 2 ? { from: r[0]!, to: r[1]! } : null;
  }
  set_visible_logical_range(range: logical_range): void {
    this.cancel_scroll_animation();
    this.chart.wasm.set_visible_logical_range(range.from, range.to);
    this.chart.repaint();
  }
  get_visible_range(): time_range | null {
    const r = this.chart.wasm.visible_time_range();
    return r.length === 2 ? { from: r[0]!, to: r[1]! } : null;
  }
  set_visible_range(range: time_range): void {
    this.cancel_scroll_animation();
    this.chart.wasm.set_visible_time_range(range.from, range.to);
    this.chart.repaint();
  }
  subscribe_visible_logical_range_change(handler: visible_logical_range_handler): void {
    this.chart.subscribe_visible_logical_range_change(handler);
  }
  unsubscribe_visible_logical_range_change(handler: visible_logical_range_handler): void {
    this.chart.unsubscribe_visible_logical_range_change(handler);
  }
  subscribe_visible_time_range_change(handler: visible_time_range_handler): void {
    this.chart.subscribe_visible_time_range_change(handler);
  }
  unsubscribe_visible_time_range_change(handler: visible_time_range_handler): void {
    this.chart.unsubscribe_visible_time_range_change(handler);
  }
  subscribe_size_change(handler: size_change_handler): void {
    this.chart.subscribe_size_change(handler);
  }
  unsubscribe_size_change(handler: size_change_handler): void {
    this.chart.unsubscribe_size_change(handler);
  }
  time_to_coordinate(time: number): number | null {
    return undef_to_null(this.chart.wasm.time_to_coordinate(time));
  }
  coordinate_to_time(x: number): number | null {
    return undef_to_null(this.chart.wasm.coordinate_to_time(x));
  }
  logical_to_coordinate(logical: number): number | null {
    return undef_to_null(this.chart.wasm.logical_to_coordinate(logical));
  }
  coordinate_to_logical(x: number): number | null {
    return undef_to_null(this.chart.wasm.coordinate_to_logical(x));
  }
  time_to_index(time: number, find_nearest = false): number | null {
    const index = undef_to_null(this.chart.wasm.time_to_index(time, find_nearest));
    return index === null ? null : Number(index);
  }
  width(): number {
    return this.chart.wasm.time_scale_width();
  }
  height(): number {
    return this.chart.wasm.time_scale_height();
  }
}

class price_scale_impl implements price_scale_api {
  applyOptions(...args: Parameters<price_scale_api["apply_options"]>): void { this.apply_options(...args); }
  setVisibleRange(...args: Parameters<price_scale_api["set_visible_range"]>): void { this.set_visible_range(...args); }
  getVisibleRange(): price_range | null { return this.get_visible_range(); }
  private readonly pane_id: number;

  constructor(
    private readonly chart: chart_impl,
    pane: number,
    private readonly id: string,
  ) {
    const pane_id = undef_to_null(chart.wasm.pane_stable_id(pane));
    if (pane_id === null) {
      throw new AerisChartsError("invalid_handle", `pane index ${pane} does not identify a live price scale`);
    }
    this.pane_id = pane_id;
    if (undef_to_null(chart.wasm.price_scale_target_by_id(pane, id)) === null) {
      throw new AerisChartsError("invalid_handle", `price scale '${id}' does not exist in pane ${pane}`);
    }
  }

  private pane(): number {
    const pane = undef_to_null(this.chart.wasm.pane_index_for_id(this.pane_id));
    if (pane === null) throw new AerisChartsError("stale_handle", "price-scale pane has been removed");
    return pane;
  }

  private target(): number {
    const target = undef_to_null(this.chart.wasm.price_scale_target_by_id(this.pane(), this.id));
    if (target === null) throw new AerisChartsError("stale_handle", "price scale has been removed");
    return target;
  }

  apply_options(options: deep_partial<price_scale_options>): void {
    validate_price_scale_options(options);
    if (options.mode !== undefined) {
      this.chart.wasm.set_price_scale_mode(this.pane(), this.target(), options.mode);
    }
    if (options.auto_scale !== undefined) {
      this.chart.wasm.set_price_scale_auto_scale(this.pane(), this.target(), options.auto_scale);
    }
    if (options.invert_scale !== undefined) {
      this.chart.wasm.set_price_scale_inverted(this.pane(), this.target(), options.invert_scale);
    }
    if (options.scale_margins !== undefined) {
      const current = this.options().scale_margins;
      this.chart.wasm.set_price_scale_margins(
        this.pane(),
        this.target(),
        options.scale_margins.top ?? current.top,
        options.scale_margins.bottom ?? current.bottom,
      );
    }
    // Style keys without a dedicated setter go to the engine as a single JSON patch.
    const json_patch: Record<string, unknown> = {};
    for (const key of PRICE_SCALE_JSON_OPTION_KEYS) {
      const value = options[key];
      if (value !== undefined) json_patch[key] = value;
    }
    if (Object.keys(json_patch).length > 0) {
      this.chart.wasm.price_scale_apply_options_json(this.pane(), this.target(), JSON.stringify(json_patch));
    }
    this.chart.repaint();
  }

  options(): price_scale_options {
    return JSON.parse(this.chart.wasm.price_scale_options_json(this.pane(), this.target())) as price_scale_options;
  }

  width(): number {
    return this.chart.wasm.price_scale_width(this.pane(), this.target());
  }

  set_visible_range(range: price_range): void {
    this.chart.wasm.set_price_scale_visible_range(this.pane(), this.target(), range.from, range.to);
    this.chart.repaint();
  }

  get_visible_range(): price_range | null {
    const range = this.chart.wasm.price_scale_visible_range(this.pane(), this.target());
    return range.length === 2 ? { from: range[0]!, to: range[1]! } : null;
  }

  set_auto_scale(on: boolean): void {
    this.chart.wasm.set_price_scale_auto_scale(this.pane(), this.target(), on);
    this.chart.repaint();
  }
}

class pane_impl implements pane_api {
  paneIndex(): number { return this.pane_index(); }
  private readonly stable_id: number;

  constructor(private readonly chart: chart_impl, index: number) {
    const stable_id = undef_to_null(chart.wasm.pane_stable_id(index));
    if (stable_id === null) throw new AerisChartsError("invalid_handle", `pane index ${index} is not live`);
    this.stable_id = stable_id;
  }

  private index(): number {
    const index = undef_to_null(this.chart.wasm.pane_index_for_id(this.stable_id));
    if (index === null) throw new AerisChartsError("stale_handle", "pane has been removed");
    return index;
  }

  pane_index(): number {
    return this.index();
  }
  get_height(): number {
    return this.chart.wasm.pane_height(this.index());
  }
  get_geometry(): pane_geometry {
    // `{}` answers a stale index (e.g. after a pane removal) — report zeros.
    const g = JSON.parse(this.chart.wasm.pane_geometry_json(this.index())) as Partial<pane_geometry>;
    return { left: g.left ?? 0, top: g.top ?? 0, width: g.width ?? 0, height: g.height ?? 0 };
  }
  set_height(height: number): void {
    this.chart.wasm.set_pane_height(this.index(), height);
    this.chart.repaint();
  }
  get_stretch_factor(): number {
    return this.chart.wasm.pane_stretch(this.index());
  }
  set_stretch_factor(factor: number): void {
    this.chart.wasm.set_pane_stretch(this.index(), factor);
    this.chart.repaint();
  }
  move_to(target: number): boolean {
    // The engine answers false for a rejected move (e.g. a stale index after remove_pane);
    // on success this handle follows the pane to its new index.
    if (!this.chart.wasm.pane_move_to(this.index(), target)) return false;
    this.chart.repaint();
    return true;
  }
  preserve_empty_pane(): boolean {
    // The engine answers false for a stale index (e.g. after remove_pane).
    return this.chart.wasm.pane_preserve_empty(this.index());
  }
  set_preserve_empty_pane(flag: boolean): void {
    this.chart.wasm.pane_set_preserve_empty(this.index(), flag);
    this.chart.repaint();
  }
  get_series(): (series_api | general_series_api)[] {
    // Live handles from the engine's id list (empty for a stale index after remove_pane).
    const ids = this.chart.wasm.pane_series_ids(this.index());
    const out: (series_api | general_series_api)[] = [];
    for (const id of ids) {
      out.push(this.chart.series_handle(id));
    }
    for (const id of this.chart.wasm.general_series_ids(this.index())) {
      const series = this.chart.general_series_handle(id);
      if (series !== null) out.push(series);
    }
    return out;
  }
  price_scale(id: string): price_scale_api {
    return this.chart.price_scale(id, this.index());
  }

  attach_primitive(primitive: pane_primitive): pane_primitive_handle {
    // Bind the hooks the plugin actually implements into a plain object (the host reads own
    // properties; binding also pins `this` for class-instance primitives), then register.
    const adapted: Record<string, unknown> = {};
    for (const key of [
      "attached",
      "detached",
      "update_all_views",
      "pane_views",
      "price_axis_views",
      "time_axis_views",
      "text_views",
      "hit_test",
    ] as const) {
      const hook = primitive[key];
      if (typeof hook === "function") adapted[key] = hook.bind(primitive);
    }
    const id = this.chart.wasm.attach_pane_primitive(this.index(), adapted);
    this.chart.repaint();
    return new pane_primitive_handle_impl(this.chart, id);
  }

  attach_canvas_primitive(primitive: canvas_primitive): canvas_primitive_handle {
    // No wasm involvement: the package owns the plugin canvas and the per-frame pass.
    return this.chart.attach_canvas_primitive(this.index(), primitive);
  }

  /** Package-internal boundary for pane-scoped Rust primitives. */
  native_add_text_watermark(options_json: string): number {
    return this.chart.wasm.add_native_text_watermark(this.index(), options_json);
  }
  native_set_text_watermark_options(id: number, options_json: string): boolean {
    const changed = this.chart.wasm.set_native_text_watermark_options(id, options_json);
    if (changed) this.chart.repaint();
    return changed;
  }
  native_remove_primitive(id: number): void {
    if (this.chart.wasm.remove_native_primitive(id)) this.chart.repaint();
  }
  native_repaint(): void {
    this.chart.repaint();
  }
}

/** Detach handle for a registered pane primitive (the wasm registry owns the lifecycle). */
class pane_primitive_handle_impl implements pane_primitive_handle {
  constructor(private readonly chart: chart_impl, private readonly id: number) {}

  detach(): void {
    // The engine answers false for an unknown/already-detached id — repaint once, on change.
    if (this.chart.wasm.detach_pane_primitive(this.id)) {
      this.chart.repaint();
    }
  }
}

/** Detach handle for a registered series primitive (the wasm registry owns the lifecycle). */
class series_primitive_handle_impl implements series_primitive_handle {
  constructor(private readonly chart: chart_impl, private readonly id: number) {}

  detach(): void {
    // The engine answers false for an unknown/already-detached id — repaint once, on change.
    // (Detaching after the owning series was removed is a no-op: the removal auto-detached.)
    if (this.chart.wasm.detach_series_primitive(this.id)) {
      this.chart.repaint();
    }
  }
}

/**
 * Where the engine paints a drawing's own text (`drawing_text_edit_layout_json`), in overlay CSS
 * px. A `multiline` layout is a family text box: lines left-aligned at `x`, line `i` centered at
 * `y + i * line_height`, never rotated. Otherwise it is one run: `x`, `y` are its start (left
 * edge, vertical center), rotated clockwise by `angle` radians about that point, and `rect`
 * bounds its padded box. The host only presents the editor the layout describes.
 */
interface text_edit_layout {
  x: number;
  y: number;
  line_height: number;
  size: number;
  font_family: string;
  weight: number;
  italic: boolean;
  color: string;
  rect: [number, number, number, number];
  angle: number;
  multiline: boolean;
}

/** One registered canvas primitive (Phase C-e) in the package-side registry. */
interface canvas_primitive_entry {
  primitive: canvas_primitive;
  pane_index: number;
  detached: boolean;
}

/** Detach handle for a registered canvas primitive (the TS registry owns the lifecycle). */
/**
 * A live drawing handle (engine-owned drawing objects). `kind`/`pane_index` are cached at
 * creation — they never change over a drawing's lifetime; everything else queries the engine.
 */
class drawing_impl implements drawing_api {
  private removed = false;

  constructor(
    private readonly chart: chart_impl,
    readonly id: number,
    private readonly drawing_kind: drawing_kind,
    private readonly pane: number,
  ) {}
  private assert_live(): string {
    void this.chart.wasm;
    if (this.removed) throw new AerisChartsError("stale_handle", "drawing has been removed");
    const options = this.chart.wasm.drawing_options_json(this.id);
    if (options === "") throw new AerisChartsError("stale_handle", "drawing has been removed");
    return options;
  }
  kind(): drawing_kind {
    this.assert_live();
    return this.drawing_kind;
  }
  pane_index(): number {
    this.assert_live();
    return this.pane;
  }
  points(): drawing_point[] {
    this.assert_live();
    return JSON.parse(this.chart.wasm.drawing_points_json(this.id)) as drawing_point[];
  }
  set_points(points: drawing_point_input[]): void {
    this.assert_live();
    drawing_result(this.chart.wasm.drawing_set_points_result_json(this.id, JSON.stringify(points)));
    this.chart.repaint();
  }
  options(): drawing_options {
    return JSON.parse(this.assert_live()) as drawing_options;
  }
  apply_options(options: Partial<drawing_options>): void {
    this.assert_live();
    if (!this.chart.wasm.drawing_apply_options(this.id, JSON.stringify(options))) {
      throw new AerisChartsError("invalid_options", "drawing options are malformed");
    }
    this.chart.repaint();
  }
  remove(): void {
    this.assert_live();
    if (!this.chart.wasm.remove_drawing(this.id)) {
      throw new AerisChartsError("stale_handle", "drawing has been removed");
    }
    this.removed = true;
    this.chart.repaint();
  }
}

class canvas_primitive_handle_impl implements canvas_primitive_handle {  private detached = false;

  constructor(private readonly chart: chart_impl, private readonly entry: canvas_primitive_entry) {}

  detach(): void {
    // Idempotent-ish: detaching twice is a no-op (mirrors the wasm-registry handles).
    if (this.detached) return;
    this.detached = true;
    this.chart.detach_canvas_primitive(this.entry);
  }
}

/** Gesture toggles resolved to concrete values the recognizer reads on each event. */
export interface resolved_gestures {
  pan: boolean;
  pan_horz_touch: boolean;
  pan_vert_touch: boolean;
  wheel_scroll: boolean;
  wheel_zoom: boolean;
  pinch_zoom: boolean;
  axis_dblclick_reset_time: boolean;
  axis_dblclick_reset_price: boolean;
  axis_scale_price: boolean;
  axis_scale_time: boolean;
  kinetic_touch: boolean;
  kinetic_mouse: boolean;
  wheel_behavior: "auto" | "pan" | "zoom";
  panes_resize: boolean;
  tracking_exit_mode: "on_next_tap" | "on_touch_end";
}

function apply_scroll(v: boolean | handle_scroll_options, cfg: resolved_gestures): void {
  if (typeof v === "boolean") {
    // reference migrateHandleScaleScrollOptions: a boolean expands to all four scroll flags.
    cfg.pan = v;
    cfg.pan_horz_touch = v;
    cfg.pan_vert_touch = v;
    cfg.wheel_scroll = v;
    return;
  }
  // Object form merges over the current config; `pan` (the mouse-drag / generic pan) tracks
  // pressed_mouse_move, and the touch axes track their own flags.
  cfg.pan = v.pressed_mouse_move ?? cfg.pan;
  cfg.pan_horz_touch = v.horz_touch_drag ?? cfg.pan_horz_touch;
  cfg.pan_vert_touch = v.vert_touch_drag ?? cfg.pan_vert_touch;
  cfg.wheel_scroll = v.mouse_wheel ?? cfg.wheel_scroll;
}

function apply_scale(v: boolean | handle_scale_options, cfg: resolved_gestures): void {
  if (typeof v === "boolean") {
    // reference migrateHandleScaleScrollOptions: a boolean expands to every scale flag.
    cfg.wheel_zoom = v;
    cfg.pinch_zoom = v;
    cfg.axis_dblclick_reset_time = v;
    cfg.axis_dblclick_reset_price = v;
    cfg.axis_scale_price = v;
    cfg.axis_scale_time = v;
    return;
  }
  // Object form merges over the current config (reference applyOptions semantics).
  cfg.wheel_zoom = v.mouse_wheel ?? cfg.wheel_zoom;
  cfg.pinch_zoom = v.pinch ?? cfg.pinch_zoom;
  const adr = v.axis_double_click_reset;
  if (typeof adr === "boolean") {
    // reference migrateHandleScaleScrollOptions: a boolean expands to both axes.
    cfg.axis_dblclick_reset_time = adr;
    cfg.axis_dblclick_reset_price = adr;
  } else if (adr) {
    cfg.axis_dblclick_reset_time = adr.time ?? cfg.axis_dblclick_reset_time;
    cfg.axis_dblclick_reset_price = adr.price ?? cfg.axis_dblclick_reset_price;
  }
  const apm = v.axis_pressed_mouse_move;
  if (typeof apm === "boolean") {
    cfg.axis_scale_price = apm;
    cfg.axis_scale_time = apm;
  } else if (apm) {
    cfg.axis_scale_price = apm.price ?? cfg.axis_scale_price;
    cfg.axis_scale_time = apm.time ?? cfg.axis_scale_time;
  }
}

function apply_kinetic(v: boolean | kinetic_scroll_options, cfg: resolved_gestures): void {
  if (typeof v === "boolean") {
    cfg.kinetic_touch = v;
    cfg.kinetic_mouse = v;
    return;
  }
  cfg.kinetic_touch = v.touch ?? cfg.kinetic_touch;
  cfg.kinetic_mouse = v.mouse ?? cfg.kinetic_mouse;
}

function apply_tracking(v: tracking_mode_options, cfg: resolved_gestures): void {
  cfg.tracking_exit_mode = v.exit_mode ?? cfg.tracking_exit_mode;
}

class trading_impl implements trading_api {
  private readonly intent_handlers = new Set<trading_intent_handler>();

  constructor(private readonly chart: chart_impl) {}

  apply_snapshot(snapshot: trading_snapshot): void {
    assert_trading_result(this.chart.wasm.set_trading_snapshot_json(JSON.stringify(snapshot)));
    this.chart.repaint();
  }

  state(): Required<trading_snapshot> {
    return JSON.parse(this.chart.wasm.trading_snapshot_json()) as Required<trading_snapshot>;
  }

  set_visible_account(account_id: string | null): void {
    assert_trading_result(this.chart.wasm.set_trading_visible_account(account_id));
    this.chart.repaint();
  }

  set_host_overlay(overlay: host_overlay_snapshot): void {
    assert_trading_result(this.chart.wasm.set_host_overlay_json(JSON.stringify(overlay)));
    this.chart.repaint();
  }

  host_overlay(): host_overlay_snapshot {
    return JSON.parse(this.chart.wasm.host_overlay_json()) as host_overlay_snapshot;
  }

  host_event_hit_at(x: number, y: number): host_event_hit | null {
    return JSON.parse(this.chart.wasm.host_event_hit_json(x, y)) as host_event_hit | null;
  }

  update_position(position: trading_position): void {
    assert_trading_result(this.chart.wasm.update_trading_position_json(JSON.stringify(position)));
    this.chart.repaint();
  }

  remove_position(id: string): boolean {
    const changed = this.chart.wasm.remove_trading_position(id);
    if (changed) this.chart.repaint();
    return changed;
  }

  update_order(order: working_order): void {
    assert_trading_result(this.chart.wasm.update_working_order_json(JSON.stringify(order)));
    this.chart.repaint();
  }

  remove_order(id: string): boolean {
    const changed = this.chart.wasm.remove_working_order(id);
    if (changed) this.chart.repaint();
    return changed;
  }

  apply_execution(execution: trading_execution): void {
    assert_trading_result(this.chart.wasm.apply_trading_execution_json(JSON.stringify(execution)));
    this.chart.repaint();
  }

  remove_execution(id: string): boolean {
    const changed = this.chart.wasm.remove_trading_execution(id);
    if (changed) this.chart.repaint();
    return changed;
  }

  set_instrument(instrument: instrument_metadata): void {
    assert_trading_result(this.chart.wasm.set_instrument_metadata_json(JSON.stringify(instrument)));
    this.chart.repaint();
  }

  apply_options(options: Partial<trading_style_options>): void {
    assert_trading_result(this.chart.wasm.apply_trading_style_json(JSON.stringify(options)));
    this.chart.repaint();
  }

  place_bracket_order(drawing_id: number, quantity: number): void {
    assert_trading_result(this.chart.wasm.place_bracket_order_from_drawing(drawing_id, quantity));
    this.dispatch_pending_intents();
  }

  hit_at(x: number, y: number): trading_hit | null {
    return JSON.parse(this.chart.wasm.trading_hit_json(x, y)) as trading_hit | null;
  }

  preview(): trading_preview | null {
    return JSON.parse(this.chart.wasm.trading_preview_json()) as trading_preview | null;
  }

  take_intents(): trading_intent[] {
    return JSON.parse(this.chart.wasm.take_trading_intents_json()) as trading_intent[];
  }

  resolve_intent(sequence: number, accepted: boolean): boolean {
    const changed = this.chart.wasm.resolve_trading_intent(sequence, accepted);
    if (changed) this.chart.repaint();
    return changed;
  }

  subscribe_intents(handler: trading_intent_handler): void {
    this.intent_handlers.add(handler);
  }

  unsubscribe_intents(handler: trading_intent_handler): void {
    this.intent_handlers.delete(handler);
  }

  dispatch_pending_intents(): void {
    for (const intent of this.take_intents()) {
      this.chart.announce_trading_intent(intent);
      for (const handler of this.intent_handlers) handler(intent);
    }
  }
}

type engine_alert_create_request = {
  sequence: number;
  pane_index: number;
  price_scale: alert_price_scale;
  price: number;
  condition: alert_condition;
  frequency: alert_frequency;
};

class alert_impl implements alert_api {
  constructor(private readonly chart: chart_impl) {}

  apply_snapshot(snapshot: alert_snapshot): void {
    assert_trading_result(this.chart.wasm.set_alert_snapshot_json(JSON.stringify(snapshot)));
    this.chart.repaint();
  }

  state(): Required<alert_snapshot> {
    return JSON.parse(this.chart.wasm.alert_snapshot_json()) as Required<alert_snapshot>;
  }

  update_line(line: alert_line): void {
    assert_trading_result(this.chart.wasm.update_alert_line_json(JSON.stringify(line)));
    this.chart.repaint();
  }

  remove_line(id: string): boolean {
    const changed = this.chart.wasm.remove_alert_line(id);
    if (changed) this.chart.repaint();
    return changed;
  }

}

export class chart_impl implements chart_api {
  addSeries(kind: "footprint", options?: Partial<any_series_options> & Partial<footprint_series_options>): footprint_series_api;
  addSeries(kind: general_series_kind, options: general_series_options): general_series_api;
  addSeries(kind: series_kind, options?: Partial<any_series_options>): series_api;
  addSeries(kind: series_kind | general_series_kind, options?: Partial<any_series_options> | general_series_options): series_api | general_series_api {
    return this.add_series(kind as series_kind, options as Partial<any_series_options>);
  }
  removeSeries(...args: Parameters<chart_api["remove_series"]>): void { this.remove_series(...args); }
  addPane(preserve_empty?: boolean): pane_api;
  addPane(options: general_pane_options): pane_api;
  addPane(options?: boolean | general_pane_options): pane_api { return this.add_pane(options); }
  addAxis(...args: Parameters<chart_api["add_axis"]>): general_axis_api { return this.add_axis(...args); }
  removeAxis(...args: Parameters<chart_api["remove_axis"]>): boolean { return this.remove_axis(...args); }
  applyOptions(...args: Parameters<chart_api["apply_options"]>): void { this.apply_options(...args); }
  timeScale(): time_scale_api { return this.time_scale(); }
  priceScale(...args: Parameters<chart_api["price_scale"]>): price_scale_api { return this.price_scale(...args); }
  takeScreenshot(...args: Parameters<chart_api["take_screenshot"]>): HTMLCanvasElement { return this.take_screenshot(...args); }
  exportState(): chart_state { return this.export_state(); }
  importState(...args: Parameters<chart_api["import_state"]>): persistence_restore_result { return this.import_state(...args); }
  private wasm_instance: AerisChart | null;
  private next_extra_series = false;
  private readonly gestures_cfg: resolved_gestures = {
    pan: true,
    pan_horz_touch: true,
    pan_vert_touch: true,
    wheel_scroll: true,
    wheel_zoom: true,
    pinch_zoom: true,
    axis_dblclick_reset_time: true,
    axis_dblclick_reset_price: true,
    axis_scale_price: true,
    axis_scale_time: true,
    kinetic_touch: true,
    kinetic_mouse: false,
    wheel_behavior: "auto",
    panes_resize: true,
    tracking_exit_mode: "on_next_tap",
  };
  private accessibility_handle: accessibility_handle | null = null;
  /** DPR used by the engine/canvas, including an explicit manual-resize override. */
  private pixel_ratio = window.devicePixelRatio || 1;
  private readonly ts = new time_scale_impl(this);
  private readonly trading_handle = new trading_impl(this);
  private readonly alert_handle = new alert_impl(this);
  private readonly crosshair_action_handlers = new Set<crosshair_action_request_handler>();
  private observer: ResizeObserver | null = null;
  private detach_gestures: (() => void) | null = null;
  private removed = false;
  private readonly series_by_id = new Map<number, series_impl>();
  private readonly general_series_by_id = new Map<number, general_series_impl>();
  private readonly crosshair_subs = new Set<mouse_event_handler>();
  private readonly click_subs = new Set<mouse_event_handler>();
  private readonly chart_context_subs = new Set<chart_context_handler>();
  private readonly dbl_click_subs = new Set<dbl_click_handler>();
  private readonly visible_logical_range_subs = new Set<visible_logical_range_handler>();
  private readonly visible_time_range_subs = new Set<visible_time_range_handler>();
  private readonly size_change_subs = new Set<size_change_handler>();
  private readonly series_added_subs = new Set<series_change_handler>();
  private readonly series_removed_subs = new Set<series_change_handler>();
  private readonly options_change_subs = new Set<options_change_handler>();
  private readonly delta_tooltip_range_listeners = new Set<() => void>();
  private last_visible_logical_range: logical_range | null;
  private last_visible_time_range: time_range | null;
  private visible_range_emitting = false;
  private visible_range_emit_pending = false;
  private last_ts_width: number;
  private last_ts_height: number;
  private auto_size: boolean;
  private dpr_query: MediaQueryList | null = null;
  private readonly dpr_change_handler = (): void => {
    if (this.removed || !this.auto_size) return;
    const bounds = this.container.getBoundingClientRect();
    if (bounds.width >= 2 && bounds.height >= 2) {
      this.apply_size(bounds.width, bounds.height, window.devicePixelRatio || 1);
    }
    this.bind_dpr_watcher();
  };
  /** Crosshair position tracked TS-side (for crosshair-less screenshots); `null` when hidden. */
  private last_crosshair: { x: number; y: number } | null = null;
  /** Last hover hit-test result (Phase C-d), refreshed on crosshair moves; feeds event params. */
  private hover: {
    series_id: number | null;
    object_id: string | null;
    cursor: string | null;
    general_hit: general_series_hit | null;
  } | null = null;
  /** Host-side serialization cache for the engine-owned armed drawing template. */
  private tool_options_json = "{}";
  private tool_listener: ((tool: drawing_kind | null) => void) | null = null;
  private readonly tool_change_subs = new Set<drawing_tool_change_handler>();
  private readonly drawing_created_subs = new Set<drawing_created_handler>();
  /** Borderless caret surface shared by the explicitly separate product edit modes. */
  private text_editor: HTMLElement | null = null;
  private text_editor_id = 0;
  /** Snapshot of the drawing's text when the editor opened — restored on Escape. */
  private text_editor_original = "";
  private text_editor_mode: "standalone_text" | "run_label" | "part_label" | null = null;
  /** The edited drawing's kind in words, for accessibility announcements. */
  private text_editor_label = "";
  /** The element focused when the editor opened (an accessibility target regains focus). */
  private text_editor_return_focus: Element | null = null;
  /**
   * The drawing selection snapshotted at pointer-DOWN, before the engine's drag grab selects
   * the hit (gestures.ts calls `note_drawing_press`). `emit_click` reads it for the public reference's
   * two-step text editing: a click opens typing mode only when the text drawing was already
   * selected when the press began; the first click just selects (focus border). A double-click
   * (`activate_drawing_double_click`) needs it too, plus the engine's word that the point is on
   * that drawing: presses a trading object or the alert widget consumed never refresh it.
   */
  private text_press_selected: number | null = null;
  /**
   * Re-anchor the open editor after any repaint-driving change (wheel zoom/scroll, pinch,
   * resize, data update): it re-queries the anchor's coordinates from the settled engine
   * scales and repositions the box, so the editor tracks its drawing instead of displacing.
   */
  private text_editor_reposition: (() => void) | null = null;
  private anim_frame: number | null = null;
  /** The 1s candle-close countdown interval; `null` while no countdown is visible. */
  private countdown_timer: ReturnType<typeof setInterval> | null = null;
  /** True while any pointer/touch is down — pauses the countdown tick so it can't repaint mid-gesture. */
  private interacting = false;
  /** Pending rAF handle for a coalesced repaint; `null` when no repaint is scheduled. */
  private repaint_raf: number | null = null;
  /** Pending rAF handle for the ring-drain loop; `null` while no ring source is bound. */
  private ring_raf: number | null = null;
  /** Reused `[pair_count, series_id, rows, ...]` report from `drain_ring_sources`. */
  private ring_report: Float64Array | null = null;
  /**
   * Reusable transfer buffer for `frame_stats()`. Sized by the engine itself so an engine that
   * appends a slot needs no matching bundle change, and allocated once per chart so a host
   * polling telemetry every frame adds no JS-heap allocation of its own (the acceptance bar is
   * that reading stats for 60s does not itself raise `cpu_ms`). Each read still copies these
   * few dozen bytes across the wasm boundary — fixed size, no growth.
   */
  private readonly stats_scratch = new Float64Array(AerisChart.frame_stats_len());

  /** The gesture recognizer marks pointer/touch activity (down = true, all-up = false). */
  set_interacting(active: boolean): void {
    this.interacting = active;
  }

  trading(): trading_api {
    return this.trading_handle;
  }

  alerts(): alert_api {
    return this.alert_handle;
  }

  set_crosshair_action_button_visible(visible: boolean): void {
    if (this.wasm.set_alert_create_button_visible(visible)) this.repaint();
  }

  subscribe_crosshair_action(handler: crosshair_action_request_handler): void {
    this.crosshair_action_handlers.add(handler);
  }

  unsubscribe_crosshair_action(handler: crosshair_action_request_handler): void {
    this.crosshair_action_handlers.delete(handler);
  }

  accessibility(): accessibility_handle {
    if (this.accessibility_handle === null) {
      throw new AerisChartsError("unsupported_operation", "accessibility is disabled for this chart");
    }
    return this.accessibility_handle;
  }

  value_snapshot(logical_index?: number): chart_value_snapshot[] {
    const raw = JSON.parse(this.wasm.value_snapshot_json(logical_index ?? Number.NaN)) as Array<
      Omit<chart_value_snapshot, "series" | "kind"> & {
        kind: series_kind | "feature";
        feature_kind: feature_series_kind | null;
      }
    >;
    return raw.map(({ feature_kind, ...entry }) => ({
      ...entry,
      series: this.series_handle(entry.series_id),
      kind: entry.kind === "feature" ? feature_kind! : entry.kind,
    }));
  }

  set_comparison_anchor(time: number | null): boolean {
    if (time !== null && (!Number.isFinite(time) || !Number.isSafeInteger(time))) {
      throw new AerisChartsError("invalid_options", "comparison anchor must be a finite integer UTC second or null");
    }
    const changed = this.wasm.set_comparison_anchor(time ?? Number.NaN);
    if (changed) this.repaint();
    return changed;
  }

  comparison_anchor(): number | null {
    const value = this.wasm.comparison_anchor();
    return Number.isNaN(value) ? null : value;
  }

  comparison_legend_snapshot(): comparison_legend_entry[] {
    return JSON.parse(this.wasm.comparison_legend_json()) as comparison_legend_entry[];
  }

  /** Internal package hook used by the singleton accessibility controller. */
  set_accessibility_handle(handle: accessibility_handle | null): void {
    this.accessibility_handle = handle;
  }

  trading_hover_at(x: number, y: number): boolean {
    return this.wasm.trading_hover_at(x, y);
  }

  alert_create_hit_at(x: number, y: number): boolean {
    return this.wasm.alert_create_hit_at(x, y);
  }

  activate_alert_create_at(x: number, y: number): boolean {
    const activated = this.wasm.activate_alert_create_at(x, y);
    const requests = JSON.parse(this.wasm.take_alert_create_requests_json()) as engine_alert_create_request[];
    for (const request of requests) {
      const action: crosshair_action_request = {
        sequence: request.sequence,
        pane_index: request.pane_index,
        price_scale_id: request.price_scale === "overlay" ? "" : request.price_scale,
        price: request.price,
      };
      for (const handler of this.crosshair_action_handlers) handler(action);
    }
    return activated;
  }

  trading_hit_at(x: number, y: number): trading_hit | null {
    return this.trading_handle.hit_at(x, y);
  }

  trading_hit_at_device(x: number, y: number, device: number): trading_hit | null {
    return JSON.parse(this.wasm.trading_hit_json_device(x, y, device)) as trading_hit | null;
  }

  trading_cursor_at(x: number, y: number): string | null {
    const cursor = this.wasm.trading_cursor_at(x, y);
    return cursor === 2 ? "grab" : cursor === 1 ? "pointer" : null;
  }

  arm_trading_tooltip(): boolean {
    return this.wasm.arm_trading_tooltip();
  }

  clear_trading_hover(): boolean {
    return this.wasm.clear_trading_hover();
  }

  trading_pressed_at(x: number, y: number): boolean {
    return this.wasm.trading_pressed_at(x, y);
  }

  trading_pressed_at_device(x: number, y: number, device: number): boolean {
    return this.wasm.trading_pressed_at_device(x, y, device);
  }

  clear_trading_pressed(): boolean {
    return this.wasm.clear_trading_pressed();
  }

  deactivate_trading_group(): boolean {
    return this.wasm.deactivate_trading_group();
  }

  trading_drag_start_at(x: number, y: number): boolean {
    return this.wasm.trading_drag_start_at(x, y);
  }

  trading_drag_start_at_device(x: number, y: number, device: number): boolean {
    return this.wasm.trading_drag_start_at_device(x, y, device);
  }

  trading_keyboard_start_order(id: string): boolean {
    const started = this.wasm.trading_keyboard_start_order(id);
    if (started) this.repaint();
    return started;
  }

  trading_keyboard_adjust(ticks: number): boolean {
    const changed = this.wasm.trading_keyboard_adjust(ticks);
    if (changed) this.repaint();
    return changed;
  }

  trading_keyboard_commit(): void {
    this.wasm.trading_keyboard_commit_json();
    this.trading_handle.dispatch_pending_intents();
    this.repaint();
  }

  trading_keyboard_cancel(): boolean {
    const changed = this.discard_trading_interaction();
    if (changed) this.repaint();
    return changed;
  }

  trading_drag_to(y: number): boolean {
    return this.wasm.trading_drag_to(y);
  }

  trading_drag_end(): void {
    this.wasm.trading_drag_end_json();
    this.trading_handle.dispatch_pending_intents();
  }

  cancel_trading_drag(): void {
    this.wasm.cancel_trading_drag();
  }

  discard_trading_interaction(): boolean {
    return this.wasm.discard_trading_interaction();
  }

  trading_activate_at(x: number, y: number): boolean {
    const activated = this.wasm.trading_activate_at(x, y);
    this.trading_handle.dispatch_pending_intents();
    return activated;
  }

  /** Standard gesture forwarding for the engine-owned delta-tooltip interaction model. */
  native_delta_tooltip_mouse_down(x: number, shift: boolean): boolean {
    return this.wasm.native_delta_tooltip_mouse_down(x, shift);
  }

  native_delta_tooltip_mouse_move(x: number): void {
    if (this.wasm.native_delta_tooltip_mouse_move(x)) this.notify_delta_tooltip_ranges();
  }

  native_delta_tooltip_mouse_up(): void {
    if (this.wasm.native_delta_tooltip_mouse_up()) this.notify_delta_tooltip_ranges();
  }

  native_delta_tooltip_touch_move(xs: Float64Array): boolean {
    const changed = this.wasm.native_delta_tooltip_touch_move(xs);
    if (changed) this.notify_delta_tooltip_ranges();
    return this.wasm.native_delta_tooltip_active();
  }

  native_delta_tooltip_touch_active(): boolean {
    return this.wasm.native_delta_tooltip_active();
  }

  native_delta_tooltip_leave(): boolean {
    const changed = this.wasm.native_delta_tooltip_leave();
    if (changed) this.notify_delta_tooltip_ranges();
    return changed;
  }

  add_delta_tooltip_range_listener(listener: () => void): () => void {
    this.delta_tooltip_range_listeners.add(listener);
    return () => this.delta_tooltip_range_listeners.delete(listener);
  }

  private notify_delta_tooltip_ranges(): void {
    for (const listener of this.delta_tooltip_range_listeners) listener();
  }
  /**
   * Cached "any live series has countdown_visible" flag (refreshed by `sync_countdown_timer`)
   * so streaming `update` calls can cheaply decide whether data-arrival may start the timer.
   */
  countdown_series_present = false;
  /** Host clock (UTC seconds) for the countdown; `null` uses `Date.now()`. */
  private clock: (() => number) | null = null;
  /**
   * IANA name of the last applied exchange time zone and the engine schedule it resolved to, so
   * `time_scale().options()` reports the name only while the engine still uses that schedule (a
   * V2 import or an explicit schedule replaces it).
   */
  private applied_zone: { name: string; schedule: string } | null = null;
  /** Whether every financial series with data was given calendar-date (`business_day`) times. */
  private calendar_date_axis = false;
  /** Per-series input form of the latest data (`true` = calendar dates) for the axis flag. */
  private readonly calendar_series = new Map<number, boolean>();
  /** The unwrapped host `localization.time_formatter` for package-owned time text. */
  private host_time_formatter: localization_options["time_formatter"] | undefined = undefined;
  private locale_setting: string | undefined = undefined;
  /** The Phase C-e plugin overlay canvas and its 2D context (package-owned host DOM). */
  private readonly plugin_ctx: CanvasRenderingContext2D;
  private readonly canvas_primitives: canvas_primitive_entry[] = [];
  private plugin_resize_observer: ResizeObserver | null = null;
  private backend_loss_count = 0;
  private readonly backend_loss_handler = (): void => {
    if (this.removed) return;
    this.backend_loss_count += 1;
    // The wgpu callback may arrive from a promise microtask. Defer the repaint once more so the
    // callback stack is fully unwound before Rust drops GPU resources and paints the warm 2D pane.
    queueMicrotask(() => this.repaint());
  };

  constructor(
    wasm: AerisChart,
    private readonly container: HTMLElement,
    private readonly gpu_pane: HTMLCanvasElement,
    private readonly fallback_pane: HTMLCanvasElement,
    private readonly plugin_canvas: HTMLCanvasElement,
    private readonly overlay: HTMLCanvasElement,
    auto_size: boolean,
    private selected_theme: theme_name = default_theme_name,
    initial_general = false,
  ) {
    this.wasm_instance = wasm;
    const plugin_ctx = plugin_canvas.getContext("2d");
    if (plugin_ctx === null) {
      throw new AerisChartsError("renderer_platform_error", "plugin canvas 2D context is unavailable");
    }
    this.plugin_ctx = plugin_ctx;
    window.addEventListener("aeris_charts-chart-backend-lost", this.backend_loss_handler);
    this.last_visible_logical_range = this.read_visible_logical_range();
    this.last_visible_time_range = this.read_visible_time_range();
    this.last_ts_width = this.wasm.time_scale_width();
    this.last_ts_height = this.wasm.time_scale_height();
    this.auto_size = auto_size;
    this.container.setAttribute("role", "group");
    if (!this.container.hasAttribute("aria-label")) {
      this.container.setAttribute("aria-label", initial_general ? "General chart" : "Financial chart");
    }
    this.detach_gestures = install_gestures(this);
    if (auto_size) {
      this.wasm.enable_auto_resize(container);
      this.bind_dpr_watcher();
    }
    // Canvas primitives (Phase C-e): the engine's own ResizeObserver (registered first, above)
    // re-renders on container resizes; this one re-runs the package-side canvas pass on the
    // settled frame so plugin content tracks the new size/DPR. The microtask defers past ALL
    // observer callbacks (microtasks drain after the notification task, before paint), so the
    // engine's auto-resize render has always completed by the time the pass runs.
    this.plugin_resize_observer = new ResizeObserver(() => {
      queueMicrotask(() => this.run_canvas_primitives());
    });
    this.plugin_resize_observer.observe(container);
  }

  /** Internal package boundary. Every post-disposal operation fails with one stable error. */
  get wasm(): AerisChart {
    if (this.wasm_instance === null) {
      throw new AerisChartsError("disposed", "this chart has been disposed");
    }
    if (this.render_callback !== null) {
      // The engine is mid-frame: re-entering it would alias its state (and abort the instance).
      throw new AerisChartsError(
        "unsupported_operation",
        `chart APIs cannot be called from ${this.render_callback}, which runs while the chart renders`,
      );
    }
    return this.wasm_instance;
  }

  /** The render-time host callback currently on the stack, if any. */
  private render_callback: string | null = null;

  /**
   * Run a host callback that the engine invokes during frame construction. Chart APIs called from
   * it throw `unsupported_operation` instead of re-entering the engine; the engine then keeps its
   * own result for that pass.
   */
  run_render_callback<T>(name: string, callback: () => T): T {
    const outer = this.render_callback;
    this.render_callback = name;
    try {
      return callback();
    } finally {
      this.render_callback = outer;
    }
  }

  /** Deterministic browser-test hook; intentionally absent from `chart_api`. */
  backend_loss_count_for_test(): number {
    return this.backend_loss_count;
  }

  /**
   * Coalesce renders onto the next animation frame. The streaming hot path
   * (series `update` on built-in series) must not pay a full render per tick: N calls
   * inside one frame produce exactly one repaint, which is what the engine's
   * invalidate-mask design assumes (reference behavior: mutations invalidate, the
   * frame flush renders). One-shot mutations (set_data, pop, options, gestures,
   * resize, explicit render(), take_screenshot) still repaint synchronously — axis
   * sizing hysteresis and custom-series render-time price computation make their
   * render counts observable. Any synchronous repaint cancels a pending scheduled
   * frame and paints immediately.
   */
  schedule_repaint(): void {
    if (this.removed || this.repaint_raf !== null) return;
    this.repaint_raf = requestAnimationFrame(() => {
      this.repaint_raf = null;
      this.repaint();
    });
  }

  /**
   * Start or stop the ring-drain frame loop to match the number of bound ring sources.
   *
   * The engine has no clock of its own — the package owns the frame loop — so a bound ring needs
   * something to tick it. While at least one ring is bound this runs one `requestAnimationFrame`
   * loop for the whole chart (not per series), and it stops as soon as the last ring unbinds, so a
   * chart with no rings pays nothing.
   *
   * Called on every bind/unbind rather than tracking a count: `ring_source_count()` is the engine's
   * own answer, which cannot drift from it.
   */
  sync_ring_drain_loop(): void {
    const rings = this.removed ? 0 : this.wasm.ring_source_count();
    if (rings === 0) {
      if (this.ring_raf !== null) {
        cancelAnimationFrame(this.ring_raf);
        this.ring_raf = null;
      }
      this.ring_report = null;
      return;
    }
    // `[pair_count, series_id, rows, ...]` — resized only when the ring count grows.
    const slots = 1 + rings * 2;
    if (this.ring_report === null || this.ring_report.length < slots) {
      this.ring_report = new Float64Array(slots);
    }
    if (this.ring_raf === null) this.ring_raf = requestAnimationFrame(this.ring_tick);
  }

  /**
   * One frame's ring drain. This is the whole per-frame cost of a ring source when the producer is
   * idle: one engine call that does an atomic load per ring. A repaint and `data_changed` only
   * happen when rows actually arrived, so a bound-but-quiet ring does not force the chart to
   * re-render at frame rate.
   */
  private readonly ring_tick = (): void => {
    this.ring_raf = null;
    if (this.removed) return;
    const report = this.ring_report;
    if (report === null) return;
    const rows = this.wasm.drain_ring_sources(report);
    if (rows > 0) {
      this.repaint();
      // Notify per series that actually received rows, using the engine's own report rather than
      // the set of bound rings — a quiet ring must not emit a spurious data change.
      const pairs = report[0] as number;
      for (let i = 0; i < pairs; i += 1) {
        const series = this.series_by_id.get(report[1 + i * 2] as number);
        series?.emit_ring_data_changed();
      }
      if (this.countdown_series_present) this.sync_countdown_timer();
    }
    this.ring_raf = requestAnimationFrame(this.ring_tick);
  };

  /** Repaint unless torn down. Named distinctly from the public `render` for internal use. */
  repaint(): void {
    if (this.repaint_raf !== null) {
      cancelAnimationFrame(this.repaint_raf);
      this.repaint_raf = null;
    }
    if (!this.removed) {
      this.wasm.render();
      // The text editor tracks its anchor through the change that drove this repaint
      // (wheel zoom/scroll, pinch, resize, data update) — before plugin passes composite.
      this.text_editor_reposition?.();
      this.run_canvas_primitives();
      this.emit_visible_range_changes();
      // The engine owns whether a pulse is drawn (line/area default on, data required), so any
      // repaint-driving change — new series, type change, data, removal — re-syncs the loop.
      this.sync_animation();
    }
  }

  /**
   * Canvas primitives (plugin platform Phase C-e): after the engine frame, paint every
   * attached canvas primitive onto the package-owned DOM canvas. The canvas remains above pane
   * content for plugin z-order, but each renderer is clipped to its owning engine pane so it cannot
   * cover the shared price/time-axis chrome now rendered by WebGPU/Canvas2D.
   */
  private run_canvas_primitives(): void {
    if (this.removed) return;
    const canvas = this.plugin_canvas;
    if (canvas.width !== this.overlay.width) canvas.width = this.overlay.width;
    if (canvas.height !== this.overlay.height) canvas.height = this.overlay.height;
    const ctx = this.plugin_ctx;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    if (this.canvas_primitives.length === 0) return;
    // One target per frame, shared by every view (the reference shares one target per widget).
    // Media size is the canvas' own CSS box — pinned to the container, like the overlay's.
    const rect = canvas.getBoundingClientRect();
    const target = create_canvas_render_target(
      ctx,
      { width: rect.width, height: rect.height },
      { width: canvas.width, height: canvas.height },
    );
    // update_all_views before the frame's views (like C-a); views bucket by z_order so all
    // `normal` views paint before all `top` views (attach order within a pass).
    const normal: { renderer: canvas_pane_view["renderer"]; pane_index: number }[] = [];
    const top: { renderer: canvas_pane_view["renderer"]; pane_index: number }[] = [];
    for (const entry of this.canvas_primitives) {
      const primitive = entry.primitive;
      try {
        primitive.update_all_views?.();
      } catch (error) {
        console.warn(`aeris_charts: canvas primitive \`update_all_views\` threw — ${error}`);
      }
      let views;
      try {
        views = primitive.pane_views?.();
      } catch (error) {
        console.warn(`aeris_charts: canvas primitive \`pane_views\` threw — ${error}`);
        continue;
      }
      for (const view of views ?? []) {
        (view.z_order === "top" ? top : normal).push({
          renderer: view.renderer,
          pane_index: entry.pane_index,
        });
      }
    }
    const hpr = rect.width > 0 ? canvas.width / rect.width : 1;
    const vpr = rect.height > 0 ? canvas.height / rect.height : 1;
    for (const { renderer, pane_index } of [...normal, ...top]) {
      // Canvas primitives are pane-scoped. Keep their DOM layer above pane content while clipping
      // it to the owning engine pane, so the shared GPU/Canvas axis strips remain top-layer chrome.
      const geometry = JSON.parse(this.wasm.pane_geometry_json(pane_index)) as Partial<pane_geometry>;
      const left = (geometry.left ?? 0) * hpr;
      const top_px = (geometry.top ?? 0) * vpr;
      const width = (geometry.width ?? 0) * hpr;
      const height = (geometry.height ?? 0) * vpr;
      if (width <= 0 || height <= 0) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(left, top_px, width, height);
      ctx.clip();
      try {
        renderer(target);
      } catch (error) {
        console.warn(`aeris_charts: canvas primitive renderer threw — ${error}`);
      } finally {
        ctx.restore();
      }
    }
  }

  /** Register a canvas primitive (Phase C-e) on behalf of `pane_impl`. */
  attach_canvas_primitive(pane_index: number, primitive: canvas_primitive): canvas_primitive_handle {
    const entry: canvas_primitive_entry = { primitive, pane_index, detached: false };
    this.canvas_primitives.push(entry);
    try {
      primitive.attached?.({ pane_index });
    } catch (error) {
      console.warn(`aeris_charts: canvas primitive \`attached\` threw — ${error}`);
    }
    this.repaint();
    return new canvas_primitive_handle_impl(this, entry);
  }

  /** Drop a canvas primitive; the repaint clears its paint (the pass re-clears the canvas). */
  detach_canvas_primitive(entry: canvas_primitive_entry): void {
    if (entry.detached) return;
    entry.detached = true;
    const index = this.canvas_primitives.indexOf(entry);
    if (index >= 0) this.canvas_primitives.splice(index, 1);
    try {
      entry.primitive.detached?.();
    } catch (error) {
      console.warn(`aeris_charts: canvas primitive \`detached\` threw — ${error}`);
    }
    this.repaint();
  }

  private read_visible_logical_range(): logical_range | null {
    const r = this.wasm.visible_logical_range();
    return r.length === 2 ? { from: r[0]!, to: r[1]! } : null;
  }

  private read_visible_time_range(): time_range | null {
    const r = this.wasm.visible_time_range();
    return r.length === 2 ? { from: r[0]!, to: r[1]! } : null;
  }

  /**
   * Diff-and-notify for the visible range and size subscriptions. A handler may mutate the chart
   * synchronously (the load-more-history recipe calls `set_data` from its range handler), which
   * repaints and re-enters here. Nested calls only mark the diff dirty; the outer dispatch
   * finishes delivering its value to every handler and then re-reads, so handlers that ran
   * after the mutating one always receive a corrective event with the final range. The re-read
   * is bounded; a handler that keeps changing the range defers the rest to the next frame.
   */
  private emit_visible_range_changes(): void {
    if (this.visible_range_emitting) {
      this.visible_range_emit_pending = true;
      return;
    }
    this.visible_range_emitting = true;
    try {
      for (let pass = 0; pass < MAX_VISIBLE_RANGE_EMIT_PASSES; pass += 1) {
        this.visible_range_emit_pending = false;
        this.emit_visible_range_diff();
        if (!this.visible_range_emit_pending || this.removed) return;
      }
      this.schedule_repaint();
    } finally {
      this.visible_range_emitting = false;
    }
  }

  private emit_visible_range_diff(): void {
    const logical = this.read_visible_logical_range();
    const time = this.read_visible_time_range();
    const logical_changed = !same_logical_range(this.last_visible_logical_range, logical);
    const time_changed = !same_time_range(this.last_visible_time_range, time);
    this.last_visible_logical_range = logical;
    this.last_visible_time_range = time;

    if (logical_changed) {
      for (const handler of this.visible_logical_range_subs) {
        handler(logical ? { ...logical } : null);
      }
    }
    if (time_changed) {
      for (const handler of this.visible_time_range_subs) {
        handler(time ? { ...time } : null);
      }
    }
    // A handler may remove the chart; its engine is gone, so there is no size left to diff.
    if (this.removed) return;

    // Size-change diff (reference `subscribeSizeChange`), fired on change only.
    const width = this.wasm.time_scale_width();
    const height = this.wasm.time_scale_height();
    if (width !== this.last_ts_width || height !== this.last_ts_height) {
      this.last_ts_width = width;
      this.last_ts_height = height;
      for (const handler of this.size_change_subs) {
        handler(width, height);
      }
    }
  }

  /** Start or stop the animation rAF loop to match whether any series wants the last-price pulse. */
  sync_animation(): void {
    if (this.removed) return;
    if (this.wasm.wants_animation()) {
      this.start_animation();
    } else {
      this.stop_animation();
    }
  }

  /**
   * Start or stop the 1s candle-close countdown interval to match whether any live series has
   * `countdown_visible` and data (industry-standard countdown row in the last-value cluster).
   * Central rebuild point: called from the series apply-options/set-data/remove paths and on
   * chart teardown. Ticks pin the engine clock and repaint; ticks are skipped while the
   * document is hidden.
   */
  sync_countdown_timer(): void {
    if (this.removed) return;
    const candidates: { countdown_visible?: boolean; has_data: boolean }[] = [];
    this.countdown_series_present = false;
    for (const series of this.series_by_id.values()) {
      if (series.options().countdown_visible === true) {
        this.countdown_series_present = true;
        candidates.push({
          countdown_visible: true,
          has_data: this.wasm.series_last_value_data(series.id, true) !== "",
        });
      }
    }
    if (countdown_timer_needed(candidates)) {
      // Re-pin the clock on every data arrival too: the countdown hides while the clock is
      // before the forming bar's open, so a clock pinned by the last 1 s tick would hide a bar
      // that just opened until the next tick.
      this.wasm.set_now_seconds(this.now_seconds());
      if (this.countdown_timer === null) {
        this.countdown_timer = setInterval(() => {
          // Skip while hidden or while the user is mid-gesture — a mid-drag repaint is the
          // visible lag/flicker when moving the chart.
          if (document.hidden || this.interacting) return;
          this.wasm.set_now_seconds(this.now_seconds());
          this.repaint();
        }, 1000);
      }
    } else if (this.countdown_timer !== null) {
      clearInterval(this.countdown_timer);
      this.countdown_timer = null;
    }
  }
  /**
   * Install (or with `null` remove) the host clock used by the candle-close countdown. The clock
   * returns UTC seconds and replaces `Date.now()` for every countdown tick and repaint.
   */
  set_clock(clock: (() => number) | null): void {
    if (clock !== null && typeof clock !== "function") {
      throw new AerisChartsError("invalid_options", "set_clock expects a function returning UTC seconds, or null");
    }
    this.clock = clock;
    // NaN unpins the engine clock so the render path falls back to the system time.
    this.wasm.set_now_seconds(clock === null ? Number.NaN : this.now_seconds());
    this.repaint();
  }

  /** Current UTC seconds from the host clock, or the system clock when none is installed. */
  now_seconds(): number {
    if (this.clock !== null) {
      try {
        const now = this.clock();
        if (Number.isFinite(now)) return now;
      } catch (error) {
        console.error("aeris_charts: host clock threw", error);
      }
    }
    return Date.now() / 1000;
  }

  /** Resolve and apply `time_zone` / `session_start` / explicit `tick_marks` / `bar_time_label` as
   *  one validated step; throws without changing anything on error. */
  apply_exchange_time(
    zone: time_zone | undefined,
    session_start: number | undefined,
    tick_marks?: unknown,
    bar_time_label?: unknown,
  ): void {
    const marks = tick_marks === undefined ? undefined : normalize_time_tick_marks(tick_marks);
    const reason = this.wasm.set_exchange_time_json(
      exchange_time_json(zone, session_start, marks, bar_time_label),
    );
    if (reason !== "") throw new AerisChartsError("invalid_options", reason);
    if (typeof zone === "string") {
      const engine = JSON.parse(this.wasm.time_scale_options_json()) as { time_zone: unknown };
      this.applied_zone = { name: zone, schedule: JSON.stringify(engine.time_zone) };
    }
    this.repaint();
  }

  /** The IANA name for the engine's current schedule, when it was applied by name. */
  time_zone_name(schedule: unknown): string | null {
    const applied = this.applied_zone;
    return applied !== null && JSON.stringify(schedule) === applied.schedule ? applied.name : null;
  }

  /** Record a replacement batch's input form (`null` when the series has no rows). */
  note_series_times(id: number, calendar: boolean | null): void {
    if (calendar === null) this.calendar_series.delete(id);
    else this.calendar_series.set(id, calendar);
    this.sync_calendar_date_axis();
  }

  /** Record a streamed row's input form: one numeric instant makes the series instant-based. */
  note_series_update(id: number, calendar: boolean): void {
    const current = this.calendar_series.get(id);
    if (current === calendar || current === false) return;
    this.calendar_series.set(id, current === undefined ? calendar : false);
    this.sync_calendar_date_axis();
  }

  private sync_calendar_date_axis(): void {
    let calendar = this.calendar_series.size > 0;
    for (const value of this.calendar_series.values()) {
      if (!value) {
        calendar = false;
        break;
      }
    }
    if (calendar === this.calendar_date_axis) return;
    this.calendar_date_axis = calendar;
    this.wasm.set_calendar_date_axis(calendar);
  }

  /** Formatter context; reads package state only, so engine-driven callbacks may use it. */
  time_label_context(seconds: number): time_label_context {
    return time_label_context_for(seconds, this.calendar_date_axis);
  }

  private formatter_locale(): string {
    return this.locale_setting ?? (typeof navigator !== "undefined" ? navigator.language : "en-US");
  }

  /** Install the host tick formatter with the reference `(time, type, locale)` arguments plus context. */
  set_tick_mark_formatter(formatter: time_scale_options["tick_mark_formatter"] | null): void {
    this.wasm.set_tick_mark_formatter(formatter === null || formatter === undefined
      ? null
      : (time: number, tick_mark_type: number) =>
        formatter(time, tick_mark_type, this.formatter_locale(), this.time_label_context(time)));
  }

  /**
   * Time text for package-owned surfaces (tooltip, accessibility): the host `time_formatter`
   * when installed, else a locale date — plus time of day for intraday rows — in the chart's
   * exchange time. `identity_seconds` is a bar's open time; the text prints its label time (the
   * close under `time_scale.bar_time_label`). Calendar-date rows always show their own date.
   */
  format_time_text(identity_seconds: number): string {
    const seconds = this.wasm.bar_label_time(identity_seconds);
    const formatter = this.host_time_formatter;
    if (formatter !== undefined) {
      try {
        const text = formatter(seconds, this.time_label_context(seconds));
        if (typeof text === "string") return text;
      } catch (error) {
        console.error("aeris_charts: localization.time_formatter threw", error);
      }
    }
    const flags = this.wasm.time_label_flags();
    const local = this.wasm.exchange_local_seconds(seconds);
    const calendar = (flags & 4) !== 0;
    const with_time = !calendar && ((flags & 1) !== 0 || local % 86_400 !== 0);
    const with_seconds = with_time && ((flags & 2) !== 0 || local % 60 !== 0);
    return format_exchange_seconds(local, this.locale_setting, with_time, with_seconds);
  }

  private stop_countdown_timer(): void {
    if (this.countdown_timer !== null) {
      clearInterval(this.countdown_timer);
      this.countdown_timer = null;
    }
  }
  private start_animation(): void {
    if (this.anim_frame !== null || this.removed) return;
    const tick = () => {
      if (this.removed || !this.wasm.wants_animation()) {
        this.anim_frame = null;
        return;
      }
      this.wasm.set_animation_time(performance.now());
      this.wasm.render();
      this.run_canvas_primitives();
      this.anim_frame = requestAnimationFrame(tick);
    };
    this.anim_frame = requestAnimationFrame(tick);
  }
  private stop_animation(): void {
    if (this.anim_frame !== null) {
      cancelAnimationFrame(this.anim_frame);
      this.anim_frame = null;
    }
  }

  add_trade_stream(key: string, options: Partial<footprint_series_options> = {}): number {
    const id = this.wasm.add_trade_stream(key, JSON.stringify(options));
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "trade stream options were rejected by the engine");
    }
    return id;
  }

  add_depth_stream(key: string, options: Partial<depth_options> = {}): number {
    const id = this.wasm.add_depth_stream(key, JSON.stringify(options));
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "depth stream options were rejected by the engine");
    }
    return id;
  }

  depth_stream_id(key: string): number | null {
    const id = this.wasm.depth_stream_id(key);
    return id === 0 ? null : id;
  }

  remove_depth_stream(stream_id: number): boolean {
    const removed = this.wasm.remove_depth_stream(stream_id);
    if (removed) this.repaint();
    return removed;
  }

  set_depth_snapshot_typed(stream_id: number, columns: depth_snapshot_columns): void {
    const result = this.wasm.set_depth_snapshot_typed(
      stream_id,
      columns.timestamp_micros,
      columns.sequence_high,
      columns.sequence_low,
      columns.bids.prices,
      columns.bids.sizes,
      columns.bids.order_counts ?? EMPTY_UINT32,
      columns.asks.prices,
      columns.asks.sizes,
      columns.asks.order_counts ?? EMPTY_UINT32,
    );
    if (result !== "") throw new AerisChartsError("invalid_data", result);
    this.repaint();
  }

  update_depth_typed(stream_id: number, columns: depth_update_columns): void {
    const result = this.wasm.update_depth_typed(
      stream_id,
      columns.timestamps_micros,
      columns.sequences.high,
      columns.sequences.low,
      columns.previous_sequences.high,
      columns.previous_sequences.low,
      columns.sides,
      columns.prices,
      columns.sizes,
      columns.order_counts ?? EMPTY_UINT32,
    );
    if (result !== "") throw new AerisChartsError("invalid_data", result);
    this.repaint();
  }

  depth_ladder(
    stream_id: number,
    levels_per_side: number,
    minimum_size = 0,
    max_distance_ticks?: number,
  ): readonly depth_ladder_row[] | null {
    return JSON.parse(this.wasm.depth_ladder_json(
      stream_id,
      levels_per_side,
      minimum_size,
      max_distance_ticks ?? 0xffffffff,
    )) as depth_ladder_row[] | null;
  }

  depth_study(
    stream_id: number,
    levels_per_side: number,
    minimum_size = 0,
    max_distance_ticks?: number,
  ): depth_study_snapshot | null {
    return JSON.parse(this.wasm.depth_study_json(
      stream_id,
      levels_per_side,
      minimum_size,
      max_distance_ticks ?? 0xffffffff,
    )) as depth_study_snapshot | null;
  }

  add_depth_heatmap(
    stream_id: number,
    options: Partial<depth_heatmap_options> & Pick<depth_heatmap_options, "price_min" | "price_max">,
  ): number {
    const id = this.wasm.add_depth_heatmap(stream_id, JSON.stringify(options));
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "depth heatmap options were rejected by the engine");
    }
    this.repaint();
    return id;
  }

  remove_depth_heatmap(id: number): boolean {
    const removed = this.wasm.remove_depth_heatmap(id);
    if (removed) this.repaint();
    return removed;
  }

  set_depth_events_typed(stream_id: number, columns: depth_event_columns): void {
    const result = this.wasm.set_depth_events_typed(
      stream_id,
      columns.timestamps_micros,
      columns.prices,
      columns.sizes,
      columns.sides,
      columns.kinds,
      columns.labels === undefined ? "" : JSON.stringify(columns.labels),
    );
    if (result !== "") throw new AerisChartsError("invalid_data", result);
    this.repaint();
  }

  add_depth_event_layer(
    stream_id: number,
    options: Partial<depth_event_layer_options> = {},
  ): number {
    const id = this.wasm.add_depth_event_layer(stream_id, JSON.stringify(options));
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "depth event layer options were rejected by the engine");
    }
    this.repaint();
    return id;
  }

  remove_depth_event_layer(id: number): boolean {
    const removed = this.wasm.remove_depth_event_layer(id);
    if (removed) this.repaint();
    return removed;
  }

  time_and_sales(
    stream_id: number,
    options: Partial<time_and_sales_options> = {},
  ): readonly time_and_sales_row[] | null {
    return JSON.parse(
      this.wasm.time_and_sales_json(stream_id, JSON.stringify(options)),
    ) as time_and_sales_row[] | null;
  }

  replay_clock_micros(): number | null {
    const clock = this.wasm.replay_clock_micros();
    return Number.isNaN(clock) ? null : clock;
  }

  set_replay_clock_micros(clock_micros: number | null): replay_clock_stats {
    const result = this.wasm.set_replay_clock_micros(clock_micros ?? Number.NaN);
    try {
      const parsed = JSON.parse(result) as Record<string, unknown>;
      if (typeof parsed.error === "string") throw_footprint_error(result);
      this.repaint();
      return parsed as unknown as replay_clock_stats;
    } catch (error) {
      if (error instanceof AerisChartsError) throw error;
      throw_footprint_error(result);
    }
  }

  configure_synthetic_bar_series(
    series: series_api | number,
    options: synthetic_bar_options,
  ): void {
    const series_id = typeof series === "number" ? series : series.id;
    const result = this.wasm.configure_synthetic_bar_series(series_id, JSON.stringify(options));
    if (result !== "") throw new AerisChartsError("invalid_options", result);
    this.repaint();
  }

  set_synthetic_bar_source(series: series_api | number, data: readonly series_data[]): void {
    const columns = pack(data);
    this.set_synthetic_bar_source_typed(series, columns);
  }

  set_synthetic_bar_source_typed(
    series: series_api | number,
    columns: ohlc_columns,
  ): void {
    const series_id = typeof series === "number" ? series : series.id;
    const result = this.wasm.set_synthetic_bar_source_typed(
      series_id,
      columns.times,
      columns.open,
      columns.high,
      columns.low,
      columns.close,
    );
    if (result !== "") throw new AerisChartsError("invalid_data", result);
    this.sync_countdown_timer();
    this.repaint();
  }

  update_synthetic_bar_source(series: series_api | number, data: ohlc_data): void {
    const series_id = typeof series === "number" ? series : series.id;
    const columns = pack([data]);
    const time = columns.times[0];
    const open = columns.open[0];
    const high = columns.high[0];
    const low = columns.low[0];
    const close = columns.close[0];
    if (time === undefined || open === undefined || high === undefined || low === undefined || close === undefined) {
      throw new AerisChartsError("invalid_data", "synthetic source update requires one complete OHLC row");
    }
    const result = this.wasm.update_synthetic_bar_source(
      series_id,
      time,
      open,
      high,
      low,
      close,
    );
    if (result !== "") throw new AerisChartsError("invalid_data", result);
    this.sync_countdown_timer();
    this.repaint();
  }

  trade_stream_id(key: string): number | null {
    const id = this.wasm.trade_stream_id(key);
    return id === 0 ? null : id;
  }

  trade_stream_revision(stream_id: number): number | null {
    const revision = this.wasm.trade_stream_revision(stream_id);
    return revision === 0xffffffff ? null : revision;
  }

  trade_stream_stats(stream_id: number): trade_stream_stats | null {
    return JSON.parse(this.wasm.trade_stream_stats_json(stream_id)) as trade_stream_stats | null;
  }

  trade_stream_replay_clock_micros(stream_id: number): number | null {
    const clock = this.wasm.trade_stream_replay_clock_micros(stream_id);
    return Number.isNaN(clock) ? null : clock;
  }

  set_trade_stream_replay_clock_micros(
    stream_id: number,
    clock_micros: number | null,
  ): replay_seek_stats {
    const result = this.wasm.set_trade_stream_replay_clock_micros(
      stream_id,
      clock_micros ?? Number.NaN,
    );
    try {
      const parsed = JSON.parse(result) as Record<string, unknown>;
      if (typeof parsed.error === "string") throw_footprint_error(result);
      this.repaint();
      return parsed as unknown as replay_seek_stats;
    } catch (error) {
      if (error instanceof AerisChartsError) throw error;
      throw_footprint_error(result);
    }
  }

  set_trade_stream_trades(stream_id: number, trades: readonly footprint_trade[]): void {
    this.set_trade_stream_trades_typed(stream_id, pack_footprint_trades(trades));
  }

  set_trade_stream_trades_typed(stream_id: number, columns: footprint_trade_columns): void {
    const result = this.wasm.set_trade_stream_trades_typed(
      stream_id,
      columns.timestamps_micros,
      columns.prices,
      columns.volumes,
      columns.aggressors,
      columns.bids,
      columns.asks,
      columns.sequences,
      columns.trade_ids,
      columns.conditions,
      columns.session_ids,
    );
    if (result !== "") throw_footprint_error(result);
    this.sync_countdown_timer();
    this.repaint();
  }

  update_trade_stream_trades(
    stream_id: number,
    trades: readonly footprint_trade[],
  ): "tip" | "historical" {
    return this.update_trade_stream_trades_typed(stream_id, pack_footprint_trades(trades));
  }

  update_trade_stream_trades_typed(
    stream_id: number,
    columns: footprint_trade_columns,
  ): "tip" | "historical" {
    const result = this.wasm.update_trade_stream_trades_typed(
      stream_id,
      columns.timestamps_micros,
      columns.prices,
      columns.volumes,
      columns.aggressors,
      columns.bids,
      columns.asks,
      columns.sequences,
      columns.trade_ids,
      columns.conditions,
      columns.session_ids,
    );
    if (result !== "tip" && result !== "historical") throw_footprint_error(result);
    if (this.countdown_series_present) this.sync_countdown_timer();
    this.schedule_repaint();
    return result;
  }

  bind_footprint_series_to_stream(series: footprint_series_api | number, stream_id: number): void {
    const id = typeof series === "number" ? series : series.id;
    if (!this.wasm.bind_footprint_series_to_stream(id, stream_id)) {
      throw new AerisChartsError("invalid_options", "trade stream binding was rejected by the engine");
    }
    this.repaint();
  }

  bind_trade_bar_series_to_stream(series: series_api | number, stream_id: number): void {
    const id = typeof series === "number" ? series : series.id;
    if (!this.wasm.bind_trade_bar_series_to_stream(id, stream_id)) {
      throw new AerisChartsError(
        "invalid_options",
        "trade bar stream binding requires a candlestick or bar series without a max_points cap that no other engine feature (a footprint, trade study, resampler, or synthetic bars) already writes",
      );
    }
    this.repaint();
  }

  add_cvd_series(
    stream_id: number,
    pane = 1,
    reset: "session" | "continuous" | "anchored" = "session",
    anchor_timestamp_micros = Number.NaN,
  ): series_api {
    const reset_code = reset === "session" ? 0 : reset === "continuous" ? 1 : 2;
    const id = this.wasm.add_cvd_series(stream_id, pane, reset_code, anchor_timestamp_micros);
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "CVD series options were rejected by the engine");
    }
    const series = new series_impl(id, "line", this);
    this.series_by_id.set(id, series);
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    this.repaint();
    return series;
  }

  add_delta_series(stream_id: number, pane = 1): series_api {
    const id = this.wasm.add_delta_series(stream_id, pane);
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "delta series options were rejected by the engine");
    }
    const series = new series_impl(id, "histogram", this);
    this.series_by_id.set(id, series);
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    this.repaint();
    return series;
  }

  add_trade_bubbles(
    series: series_api | number,
    stream_id: number,
    options: { minimum_volume?: number; max_markers?: number; aggregation_window_micros?: number } = {},
  ): void {
    const series_id = typeof series === "number" ? series : series.id;
    if (!this.wasm.add_trade_bubbles(
      stream_id,
      series_id,
      options.minimum_volume ?? 0,
      options.max_markers ?? 2048,
      options.aggregation_window_micros ?? 0,
    )) {
      throw new AerisChartsError("invalid_options", "trade bubbles were rejected by the engine");
    }
    this.repaint();
  }

  set_trade_stream_sessions(stream_id: number, sessions: trade_session_options | null): void {
    const request = sessions === null
      ? null
      : { windows: sessions.windows, outside: sessions.outside ?? "fold" };
    const result = this.wasm.set_trade_stream_sessions(stream_id, JSON.stringify(request));
    if (result !== "") throw new AerisChartsError("invalid_options", result);
    this.sync_countdown_timer();
    this.repaint();
  }

  add_trade_volume_series(stream_id: number, pane = 1): series_api {
    const id = this.wasm.add_trade_volume_series(stream_id, pane);
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "trade volume series options were rejected by the engine");
    }
    const series = new series_impl(id, "histogram", this);
    this.series_by_id.set(id, series);
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    this.repaint();
    return series;
  }

  configure_resampled_series(target: series_api | number, options: resample_series_options): void {
    const id_of = (series: series_api | number | undefined) =>
      series === undefined ? undefined : typeof series === "number" ? series : series.id;
    const request = JSON.stringify({
      source: id_of(options.source),
      volume_source: id_of(options.volume_source),
      volume_target: id_of(options.volume_target),
      interval_seconds: options.interval_seconds,
      boundaries: Array.from(options.boundaries ?? [], (boundary) => ({
        start_time: boundary.start_time,
        end_time: boundary.end_time,
        session_id: boundary.session_id,
      })),
    });
    const target_id = typeof target === "number" ? target : target.id;
    const result = this.wasm.configure_resampled_series(target_id, request);
    if (result !== "") throw new AerisChartsError("invalid_options", result);
    this.sync_countdown_timer();
    this.repaint();
  }

  resampled_bars(target: series_api | number): readonly resampled_bar[] | null {
    const id = typeof target === "number" ? target : target.id;
    return JSON.parse(this.wasm.resampled_bars_json(id)) as resampled_bar[] | null;
  }

  resample_stats(target: series_api | number): resample_stats | null {
    const id = typeof target === "number" ? target : target.id;
    return JSON.parse(this.wasm.resample_stats_json(id)) as resample_stats | null;
  }

  add_series(
    kind: "footprint",
    options?: Partial<any_series_options> & Partial<footprint_series_options>,
  ): footprint_series_api;
  add_series(kind: general_series_kind, options: general_series_options): general_series_api;
  add_series(kind: series_kind, options?: Partial<any_series_options>): series_api;
  add_series(
    kind: series_kind | general_series_kind,
    options?: Partial<any_series_options> | general_series_options,
  ): series_api | general_series_api {
    if (
      kind === "xy_line"
      || kind === "xy_area"
      || kind === "range_area"
      || kind === "range_bar"
      || kind === "error_bar"
      || kind === "column"
      || kind === "horizontal_bar"
      || kind === "box_plot"
      || kind === "heatmap_grid"
      || kind === "scatter"
      || kind === "bubble"
    ) {
      if (options === undefined || !("x_axis_id" in options) || !("y_axis_id" in options)) {
        throw new AerisChartsError(
          "invalid_options",
          `${kind} requires pane, x_axis_id, and y_axis_id options`,
        );
      }
      const created = parse_general_result<{ series: number; dataset: number }>(
        this.wasm.add_general_series_result_json(kind, JSON.stringify(options)),
      );
      const series = new general_series_impl(
        created.series,
        created.dataset,
        kind,
        options.x_axis_id,
        options.y_axis_id,
        this,
      );
      this.general_series_by_id.set(created.series, series);
      this.emit_series_change(this.series_added_subs, series, options.pane);
      this.repaint();
      return series;
    }
    if (kind === "custom") {
      throw new AerisChartsError(
        "invalid_options",
        "add_series does not accept 'custom'; use add_custom_series(pane_view)",
      );
    }
    if (is_footprint_series_kind(kind)) {
      const financial_options = options as (Partial<any_series_options> & Partial<footprint_series_options>) | undefined;
      const requested_scale = financial_options?.overlay
        ? ""
        : financial_options?.priceScaleId ?? financial_options?.price_scale_id;
      const pane = financial_options?.pane ?? 0;
      if (requested_scale !== undefined
        && !["left", "right", ""].includes(requested_scale)
        && undef_to_null(this.wasm.price_scale_target_by_id(pane, requested_scale)) === null) {
        throw new AerisChartsError(
          "invalid_options",
          `price scale '${requested_scale}' does not exist in pane ${pane}`,
        );
      }
      const adopt_primary = !this.next_extra_series;
      const id = this.wasm.add_footprint_series(adopt_primary, JSON.stringify(financial_options ?? {}));
      if (id === 0xffffffff) {
        throw new AerisChartsError("invalid_options", "footprint series options were rejected by the engine");
      }
      const series = new footprint_series_impl(id, this);
      if (financial_options) series.apply_options(financial_options);
      this.next_extra_series = true;
      this.series_by_id.set(id, series);
      this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
      return series;
    }
    if (is_feature_series_kind(kind)) {
      const adopt_primary = !this.next_extra_series;
      this.next_extra_series = true;
      const id = this.wasm.add_feature_series(FEATURE_KIND_TO_U8[kind], adopt_primary, "{}");
      if (id === 0xffffffff) {
        throw new AerisChartsError("invalid_options", `advanced series '${kind}' was rejected by the engine`);
      }
      const series = new feature_series_impl(id, kind, this);
      this.series_by_id.set(id, series);
      if (options) series.apply_options(options as Partial<any_series_options>);
      this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
      return series;
    }
    // Option values that need no engine state are checked before the series exists, so a bad
    // value creates or adopts nothing.
    if (options) {
      const rule = (options as Partial<series_options>).histogram_updown_rule;
      if (rule !== undefined && !HISTOGRAM_UPDOWN_RULES.includes(rule)) {
        throw new AerisChartsError("invalid_options", `histogram_updown_rule must be "open_close" or "previous_close"`);
      }
      validate_series_time_alignment(options as Partial<series_options>);
    }
    // Series 0 is created by the engine at construction; the first add_series adopts it so the
    // common "one chart, one series" path matches reference (add_series returns the primary series).
    const adopted = !this.next_extra_series;
    const adopted_kind = adopted ? this.wasm.series_kind(0) : undefined;
    let id: number;
    if (adopted) {
      this.next_extra_series = true;
      this.wasm.set_series_type(KIND_TO_U8[kind]);
      id = 0;
    } else {
      id = this.wasm.add_series(KIND_TO_U8[kind]);
    }
    const series = new series_impl(id, kind, this);
    this.series_by_id.set(id, series);
    if (options) {
      try {
        series.apply_options(options as Partial<any_series_options>);
      } catch (error) {
        // An engine refusal (an as-of alignment on a non-time axis) leaves no series behind:
        // the caller never receives this handle.
        this.series_by_id.delete(id);
        series.mark_removed();
        if (adopted) {
          this.next_extra_series = false;
          if (adopted_kind !== undefined) this.wasm.set_series_type(adopted_kind);
        } else {
          this.wasm.remove_series_tracked(id);
        }
        throw error;
      }
    }
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    return series;
  }

  add_custom_series(pane_view: custom_series_pane_view, options?: Partial<series_options>): series_api {
    // Bind the hooks the view actually implements into a plain object (the host reads own
    // properties; binding also pins `this` for class-instance views), then register — the same
    // adaptation the primitive handles use.
    const adapted: Record<string, unknown> = {};
    for (const key of ["price_value_builder", "is_whitespace", "render", "destroy"] as const) {
      const hook = pane_view[key];
      if (typeof hook === "function") adapted[key] = hook.bind(pane_view);
    }
    if (typeof adapted.price_value_builder !== "function" || typeof adapted.render !== "function") {
      throw new AerisChartsError(
        "invalid_options",
        "add_custom_series needs a pane view with `price_value_builder` and `render`",
      );
    }
    // The first-series adoption mirrors add_series (the engine's construction-time series 0
    // converts to Custom instead of leaving an empty built-in behind).
    let id: number;
    if (!this.next_extra_series) {
      this.next_extra_series = true;
      id = this.wasm.add_custom_series(adapted, true);
    } else {
      id = this.wasm.add_custom_series(adapted, false);
    }
    if (id === 0xffffffff) {
      throw new AerisChartsError("invalid_options", "add_custom_series was rejected by the engine");
    }
    const series = new custom_series_impl(id, this);
    this.series_by_id.set(id, series);
    // reference createCustomSeriesDefinition: the view's defaultOptions merge UNDER the caller's.
    const merged = { ...(pane_view.default_options ?? {}), ...(options ?? {}) } as Partial<series_options>;
    if (Object.keys(merged).length > 0) series.apply_options(merged);
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    return series;
  }

  remove_series(series: series_api | general_series_api): void {
    if (series instanceof general_series_impl) {
      this.remove_general_series_handle(series);
      return;
    }
    const impl = this.series_by_id.get(series.id);
    // Ignore a foreign handle or one already removed (idempotent, matching reference leniency).
    if (!impl) return;
    // Pane indices are captured BEFORE the engine tombstones: a removed series reports no pane,
    // and the removal can cascade to derived indicator outputs living on other panes.
    const pane_of = new Map<number, number>();
    for (const id of this.series_by_id.keys()) pane_of.set(id, this.pane_of_series(id));
    // The engine tombstones the primary series (id 0) safely, so no id is refused here. The
    // tracked removal reports the series plus every derived indicator output dropped with it.
    const dropped = this.wasm.remove_series_tracked(series.id);
    if (dropped.length === 0) return;
    for (const id of dropped) {
      const handle = this.series_by_id.get(id);
      if (!handle) continue;
      handle.mark_removed();
      this.series_by_id.delete(id);
      this.note_series_times(id, null);
      this.emit_series_change(this.series_removed_subs, handle, pane_of.get(id) ?? 0);
    }
    this.sync_countdown_timer();
    // The engine drops a removed series' ring with it, so the drain loop may now have nothing left
    // to do.
    this.sync_ring_drain_loop();
    this.repaint();
  }

  series_order(): series_api[] {
    const ids = JSON.parse(this.wasm.series_order_json()) as number[];
    // Only ids with a live TS handle are returned; a series the package no longer tracks is skipped.
    return ids
      .map((id) => this.series_by_id.get(id))
      .filter((s): s is series_impl => s !== undefined);
  }

  /**
   * The live handle for an engine series id: the registered handle when one exists, otherwise a
   * fresh handle adopted into the registry (the way `panes()`/`price_scale()` build handles on
   * demand). Backs `pane_api.get_series`.
   */
  series_handle(id: number): series_impl {
    let series = this.series_by_id.get(id);
    if (series === undefined) {
      const kind = this.wasm.series_kind(id) ?? KIND_TO_U8.candlestick;
      if (kind === 7) {
        const feature_kind = FEATURE_KIND_NAMES[this.wasm.feature_series_kind(id) ?? -1];
        series = feature_kind === undefined
          ? new series_impl(id, "candlestick", this)
          : new feature_series_impl(id, feature_kind, this);
      } else if (kind === 8) {
        series = new footprint_series_impl(id, this);
      } else {
        series = new series_impl(id, KIND_NAMES[kind] ?? "candlestick", this);
      }
      this.series_by_id.set(id, series);
    }
    return series;
  }

  general_series_handle(id: number): general_series_impl | null {
    return this.general_series_by_id.get(id) ?? null;
  }

  general_series_order(pane?: number): general_series_api[] {
    if (pane !== undefined && (!Number.isInteger(pane) || pane < 0)) {
      throw new AerisChartsError("invalid_options", "general series order pane must be a non-negative integer");
    }
    const ids = JSON.parse(this.wasm.general_series_order_json(pane ?? -1)) as number[];
    return ids
      .map((id) => this.general_series_by_id.get(id))
      .filter((series): series is general_series_impl => series !== undefined);
  }

  set_general_series_order(ordered: general_series_api[], pane?: number): boolean {
    if (pane !== undefined && (!Number.isInteger(pane) || pane < 0)) return false;
    if (ordered.some((series) => this.general_series_by_id.get(series.id) !== series)) return false;
    const ids = new Uint32Array(ordered.map((series) => series.id));
    if (!this.wasm.set_general_series_order(pane ?? -1, ids)) return false;
    this.repaint();
    return true;
  }

  set_series_order(ordered: series_api[]): boolean {
    const ids = new Uint32Array(ordered.map((s) => s.id));
    if (!this.wasm.set_series_order(ids)) return false;
    this.repaint();
    return true;
  }

  private indicator_series(id: number, options?: Partial<series_options>): series_api {
    if (id === 0xffffffff) throw new AerisChartsError("invalid_options", "invalid indicator configuration");
    const series = new series_impl(id, "line", this);
    this.series_by_id.set(id, series);
    if (options) series.apply_options(options);
    this.emit_series_change(this.series_added_subs, series, this.pane_of_series(id));
    // Separate-pane indicators create their pane engine-side: relayout now so the new pane
    // gets real bounds on this frame instead of waiting for the next incidental repaint.
    this.repaint();
    return series;
  }

  add_sma(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_sma(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_sma_with_source(source: series_api, input: indicator_input_source, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_sma_with_source(source.id, input, Math.max(1, Math.floor(period))), options);
  }

  /**
   * Create a built-in indicator from its typed engine definition. Undefined fields are omitted, so
   * the engine applies its defaults, then `convention`, then the explicit fields.
   */
  private add_indicator_definition(
    source: series_api,
    input: indicator_input_source,
    definition: Record<string, unknown>,
    convention: indicator_convention | undefined,
    volume_source?: series_api | null,
    amount_source?: series_api | null,
  ): number[] {
    if (convention !== undefined && convention !== "tradingview" && convention !== "china") {
      throw new AerisChartsError("invalid_options", `unknown indicator convention: ${String(convention)}`);
    }
    return Array.from(this.wasm.add_indicator(
      source.id,
      input,
      JSON.stringify(definition),
      convention ?? "",
      volume_source?.id ?? -1,
      amount_source?.id ?? -1,
    ));
  }

  private add_seeded_average(kind: "ema" | "dema" | "tema" | "rsi", source: series_api, input: indicator_input_source, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    const ids = this.add_indicator_definition(source, input, { kind, period: Math.max(1, Math.floor(period)), seed: parameters?.seed }, parameters?.convention);
    return this.indicator_series(ids[0] ?? 0xffffffff, options);
  }

  add_ema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    return this.add_seeded_average("ema", source, "close", period, options, parameters);
  }

  add_dema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    return this.add_seeded_average("dema", source, "close", period, options, parameters);
  }

  add_tema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    return this.add_seeded_average("tema", source, "close", period, options, parameters);
  }

  add_smma(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_smma(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_rma(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_rma(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_hma(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_hma(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_ema_ribbon(
    source: series_api,
    periods: ema_ribbon_periods = [5, 10, 20, 50, 200],
    options?: ema_ribbon_options,
  ): [series_api, series_api, series_api, series_api, series_api] {
    const normalized = periods.map((period) => Math.max(1, Math.floor(period))) as [number, number, number, number, number];
    const ids = this.wasm.add_ema_ribbon(source.id, ...normalized);
    if (ids.length !== 5) throw new AerisChartsError("invalid_options", "invalid EMA ribbon configuration");
    return [
      this.indicator_series(ids[0]!, options?.[0]),
      this.indicator_series(ids[1]!, options?.[1]),
      this.indicator_series(ids[2]!, options?.[2]),
      this.indicator_series(ids[3]!, options?.[3]),
      this.indicator_series(ids[4]!, options?.[4]),
    ];
  }

  set_ema_ribbon_periods(indicator: series_api, periods: ema_ribbon_periods): boolean {
    const normalized = periods.map((period) => Math.max(1, Math.floor(period))) as [number, number, number, number, number];
    const updated = this.wasm.set_ema_ribbon_periods(indicator.id, ...normalized);
    if (updated) this.repaint();
    return updated;
  }

  add_bollinger(source: series_api, period: number, deviation = 2, options?: Partial<series_options>, parameters?: bollinger_parameters): [series_api, series_api, series_api] {
    return this.add_bollinger_with_source(source, "close", period, deviation, options, parameters);
  }

  add_bollinger_with_source(source: series_api, input: indicator_input_source, period: number, deviation = 2, options?: Partial<series_options>, parameters?: bollinger_parameters): [series_api, series_api, series_api] {
    const ids = this.add_indicator_definition(source, input, {
      kind: "bollinger",
      period: Math.max(1, Math.floor(period)),
      deviation,
      estimator: parameters?.estimator,
    }, parameters?.convention);
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid Bollinger configuration");
    return [this.indicator_series(ids[0]!, options), this.indicator_series(ids[1]!, options), this.indicator_series(ids[2]!, options)];
  }

  add_rsi(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    return this.add_seeded_average("rsi", source, "close", period, options, parameters);
  }

  add_rsi_with_source(source: series_api, input: indicator_input_source, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api {
    return this.add_seeded_average("rsi", source, input, period, options, parameters);
  }

  set_indicator_input_source(indicator: series_api, input: indicator_input_source): boolean {
    const updated = this.wasm.set_indicator_input_source(indicator.id, input);
    if (updated) this.repaint();
    return updated;
  }

  indicator_schema(kind: indicator_kind, period = 14, deviation = 2): indicator_schema {
    const raw = this.wasm.indicator_schema_json(kind, Math.max(1, Math.floor(period)), deviation);
    const schema = JSON.parse(raw) as indicator_schema | null;
    if (schema === null) throw new AerisChartsError("invalid_options", "unknown indicator kind");
    return schema;
  }

  add_macd(source: series_api, fast: number, slow: number, signal: number, options?: Partial<series_options>, parameters?: macd_parameters): [series_api, series_api, series_api] {
    const ids = this.add_indicator_definition(source, "close", {
      kind: "macd",
      fast: Math.max(1, Math.floor(fast)),
      slow: Math.max(1, Math.floor(slow)),
      signal: Math.max(1, Math.floor(signal)),
      seed: parameters?.seed,
      histogram_multiplier: parameters?.histogram_multiplier,
    }, parameters?.convention);
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid MACD configuration");
    return [this.indicator_series(ids[0]!, options), this.indicator_series(ids[1]!, options), this.indicator_series(ids[2]!, options)];
  }

  add_kdj(source: series_api, period = 9, k_smoothing = 3, d_smoothing = 3, options?: Partial<series_options>, parameters?: kdj_parameters): [series_api, series_api, series_api] {
    const whole = (value: number) => Math.max(1, Math.floor(value));
    const ids = this.add_indicator_definition(source, "close", {
      kind: "kdj",
      period: whole(period),
      k_smoothing: whole(k_smoothing),
      d_smoothing: whole(d_smoothing),
      seed: parameters?.seed,
    }, parameters?.convention);
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid KDJ configuration");
    return [this.indicator_series(ids[0]!, options), this.indicator_series(ids[1]!, options), this.indicator_series(ids[2]!, options)];
  }

  add_stochastic(source: series_api, k_period: number, d_period: number, options?: Partial<series_options>): [series_api, series_api] {
    const ids = this.wasm.add_stochastic(source.id, Math.max(1, Math.floor(k_period)), Math.max(1, Math.floor(d_period)));
    if (ids.length !== 2) throw new AerisChartsError("invalid_options", "invalid Stochastic configuration");
    return [this.indicator_series(ids[0]!, options), this.indicator_series(ids[1]!, options)];
  }

  add_atr(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_atr(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_vwap(source: series_api, volume_source?: series_api | null, options?: Partial<series_options>, parameters?: vwap_parameters): series_api {
    const ids = this.add_indicator_definition(source, "close", { kind: "vwap" }, undefined, volume_source, parameters?.amount_source);
    return this.indicator_series(ids[0] ?? 0xffffffff, options);
  }

  add_obv(source: series_api, volume_source: series_api, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_obv(source.id, volume_source.id), options);
  }

  add_cmf(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_cmf(source.id, volume_source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_mfi(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_mfi(source.id, volume_source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_volume(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): [series_api, series_api] {
    const ids = this.wasm.add_volume(source.id, volume_source.id, Math.max(1, Math.floor(period)));
    if (ids.length !== 2) throw new AerisChartsError("invalid_options", "invalid volume study configuration");
    return [this.indicator_series(ids[0]!, options), this.indicator_series(ids[1]!, options)];
  }

  add_vwap_bands(
    source: series_api,
    reset: vwap_reset = "session",
    standard_deviation = 1,
    percent = 10,
    volume_source?: series_api | null,
    options?: Partial<series_options>,
  ): [series_api, series_api, series_api, series_api, series_api] {
    const ids = this.wasm.add_vwap_bands(
      source.id,
      volume_source?.id ?? -1,
      reset,
      standard_deviation,
      percent,
    );
    if (ids.length !== 5) throw new AerisChartsError("invalid_options", "invalid VWAP bands configuration");
    return [
      this.indicator_series(ids[0]!, options),
      this.indicator_series(ids[1]!, options),
      this.indicator_series(ids[2]!, options),
      this.indicator_series(ids[3]!, options),
      this.indicator_series(ids[4]!, options),
    ];
  }

  add_volume_profile(source: series_api, volume_source: series_api, options: Partial<volume_profile_indicator_options> = {}): volume_profile_indicator_api {
    const wasm = this.wasm;
    if (this.series_by_id.get(source.id) !== source || this.series_by_id.get(volume_source.id) !== volume_source) {
      throw new AerisChartsError("invalid_handle", "volume profile requires two live series from this chart");
    }
    const id = wasm.add_volume_profile_indicator(source.id, volume_source.id, JSON.stringify(options));
    if (id === 0) throw new AerisChartsError("invalid_options", "invalid volume-profile sources, options, or indicator limit (16)");
    let removed = false;
    const read_options = (): volume_profile_indicator_options => {
      const result = removed ? null : JSON.parse(this.wasm.volume_profile_indicator_options(id)) as volume_profile_indicator_options | null;
      if (result === null) throw new AerisChartsError("stale_handle", "volume-profile indicator has been removed");
      return result;
    };
    this.repaint();
    return {
      id,
      options: read_options,
      apply_options: (patch) => {
        const merged = { ...read_options(), ...patch };
        if (!this.wasm.set_volume_profile_indicator_options(id, JSON.stringify(merged))) {
          throw new AerisChartsError("invalid_options", "invalid volume-profile options");
        }
        this.repaint();
      },
      snapshot: () => {
        const result = removed ? null : JSON.parse(this.wasm.volume_profile_indicator_snapshot(id)) as volume_profile_indicator_snapshot | null;
        if (result === null) throw new AerisChartsError("stale_handle", "volume-profile indicator has been removed");
        return result;
      },
      remove: () => {
        if (removed) return;
        if (this.wasm.remove_native_primitive(id)) this.repaint();
        removed = true;
      },
    };
  }

  add_wma(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_wma(source.id, Math.max(1, Math.floor(period))), options);
  }

  add_vwma(source: series_api, period: number, volume_source?: series_api | null, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_vwma(source.id, volume_source?.id ?? -1, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_standard_deviation(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_standard_deviation(source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_cci(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_cci(source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_williams_r(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_williams_r(source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_stochastic_rsi(source: series_api, rsi_period: number, stochastic_period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_stochastic_rsi(
        source.id,
        Math.max(1, Math.floor(rsi_period)),
        Math.max(1, Math.floor(stochastic_period)),
      ),
      options,
    );
  }

  add_momentum(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_momentum(source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_roc(source: series_api, period: number, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_roc(source.id, Math.max(1, Math.floor(period))),
      options,
    );
  }

  add_donchian(source: series_api, period: number, options?: Partial<series_options>): [series_api, series_api, series_api] {
    const ids = this.wasm.add_donchian(source.id, Math.max(1, Math.floor(period)));
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid Donchian configuration");
    return [
      this.indicator_series(ids[0]!, options),
      this.indicator_series(ids[1]!, options),
      this.indicator_series(ids[2]!, options),
    ];
  }

  add_pivot_points(source: series_api, variant: pivot_kind = "standard", options?: Partial<series_options>): [series_api, series_api, series_api, series_api, series_api] {
    const index = ({ standard: 1, fibonacci: 2, camarilla: 3, woodie: 4, demark: 5 } as const)[variant];
    const ids = this.wasm.add_pivot_points(source.id, index);
    if (ids.length !== 5) throw new AerisChartsError("invalid_options", "invalid pivot-point configuration");
    return [
      this.indicator_series(ids[0]!, options),
      this.indicator_series(ids[1]!, options),
      this.indicator_series(ids[2]!, options),
      this.indicator_series(ids[3]!, options),
      this.indicator_series(ids[4]!, options),
    ];
  }

  add_zigzag(source: series_api, deviation_percent = 5, options?: Partial<series_options>): series_api {
    const id = this.wasm.add_zigzag(source.id, deviation_percent);
    if (id === 0xffffffff) throw new AerisChartsError("invalid_options", "invalid ZigZag deviation");
    return this.indicator_series(id, options);
  }

  add_keltner(source: series_api, period: number, multiplier = 2, options?: Partial<series_options>): [series_api, series_api, series_api] {
    const ids = this.wasm.add_keltner(source.id, Math.max(1, Math.floor(period)), multiplier);
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid Keltner configuration");
    return [
      this.indicator_series(ids[0]!, options),
      this.indicator_series(ids[1]!, options),
      this.indicator_series(ids[2]!, options),
    ];
  }

  add_adx_dmi(source: series_api, period: number, options?: Partial<series_options>): [series_api, series_api, series_api] {
    const ids = this.wasm.add_adx_dmi(source.id, Math.max(1, Math.floor(period)));
    if (ids.length !== 3) throw new AerisChartsError("invalid_options", "invalid ADX/DMI configuration");
    return [
      this.indicator_series(ids[0]!, options),
      this.indicator_series(ids[1]!, options),
      this.indicator_series(ids[2]!, options),
    ];
  }

  add_parabolic_sar(source: series_api, options?: Partial<series_options>): series_api {
    return this.indicator_series(this.wasm.add_parabolic_sar(source.id), options);
  }

  add_supertrend(source: series_api, period: number, multiplier = 3, options?: Partial<series_options>): series_api {
    return this.indicator_series(
      this.wasm.add_supertrend(source.id, Math.max(1, Math.floor(period)), multiplier),
      options,
    );
  }

  add_ichimoku(source: series_api, options?: Partial<series_options>): [series_api, series_api, series_api, series_api, series_api] {
    const ids = this.wasm.add_ichimoku(source.id);
    if (ids.length !== 5) throw new AerisChartsError("invalid_options", "invalid Ichimoku configuration");
    const outputs = Array.from(ids).map((id) => this.indicator_series(id, options));
    return outputs as [series_api, series_api, series_api, series_api, series_api];
  }

  subscribe_crosshair_move(handler: mouse_event_handler): void {
    this.crosshair_subs.add(handler);
  }
  unsubscribe_crosshair_move(handler: mouse_event_handler): void {
    this.crosshair_subs.delete(handler);
  }
  subscribe_click(handler: mouse_event_handler): void {
    this.click_subs.add(handler);
  }
  unsubscribe_click(handler: mouse_event_handler): void {
    this.click_subs.delete(handler);
  }
  subscribe_chart_context(handler: chart_context_handler): void {
    this.chart_context_subs.add(handler);
  }
  unsubscribe_chart_context(handler: chart_context_handler): void {
    this.chart_context_subs.delete(handler);
  }
  subscribe_dbl_click(handler: dbl_click_handler): void {
    this.dbl_click_subs.add(handler);
  }
  unsubscribe_dbl_click(handler: dbl_click_handler): void {
    this.dbl_click_subs.delete(handler);
  }
  subscribe_visible_logical_range_change(handler: visible_logical_range_handler): void {
    this.visible_logical_range_subs.add(handler);
  }
  unsubscribe_visible_logical_range_change(handler: visible_logical_range_handler): void {
    this.visible_logical_range_subs.delete(handler);
  }
  subscribe_visible_time_range_change(handler: visible_time_range_handler): void {
    this.visible_time_range_subs.add(handler);
  }
  unsubscribe_visible_time_range_change(handler: visible_time_range_handler): void {
    this.visible_time_range_subs.delete(handler);
  }
  subscribe_size_change(handler: size_change_handler): void {
    this.size_change_subs.add(handler);
  }
  unsubscribe_size_change(handler: size_change_handler): void {
    this.size_change_subs.delete(handler);
  }
  subscribe_series_added(handler: series_change_handler): void {
    this.series_added_subs.add(handler);
  }
  unsubscribe_series_added(handler: series_change_handler): void {
    this.series_added_subs.delete(handler);
  }
  subscribe_series_removed(handler: series_change_handler): void {
    this.series_removed_subs.add(handler);
  }
  unsubscribe_series_removed(handler: series_change_handler): void {
    this.series_removed_subs.delete(handler);
  }
  subscribe_options_change(handler: options_change_handler): void {
    this.options_change_subs.add(handler);
  }
  unsubscribe_options_change(handler: options_change_handler): void {
    this.options_change_subs.delete(handler);
  }

  /** A series' current pane index (0 fallback for a tombstoned/unknown id). */
  private pane_of_series(id: number): number {
    return undef_to_null(this.wasm.series_pane_index(id)) ?? 0;
  }

  /** Notify series-lifecycle subscribers (added and removed share the payload shape). */
  private emit_series_change(
    subs: Set<series_change_handler>,
    series: series_api | general_series_api,
    pane_index: number,
  ): void {
    if (subs.size === 0) return;
    for (const h of subs) h({ series, pane_index });
  }

  /** Invalidate any in-flight animated `scroll_to_position` (user gestures take over scrolling). */
  cancel_scroll_animation(): void {
    this.ts.cancel_scroll_animation();
  }

  /**
   * Which pane contains the pane-relative CSS point (x, y), or `null` when it falls on an axis
   * strip (price/time) or outside the pane area. Backs `mouse_event_params.pane_index`.
   */
  pane_index_at(x: number, y: number): number | null {
    if (x < 0 || x > this.wasm.time_scale_width()) return null;
    const pane_bottom = this.overlay.getBoundingClientRect().height - this.wasm.time_scale_height();
    if (y < 0 || y > pane_bottom) return null;
    return this.wasm.pane_index_at_y(y);
  }

  /** Build event params for a cursor at (x, y) in pane CSS px. */
  private build_params(x: number, y: number): mouse_event_params {
    const time = undef_to_null(this.wasm.coordinate_to_time(x));
    const logical = undef_to_null(this.wasm.coordinate_to_logical(x));
    const value_snapshot = logical === null ? [] : this.value_snapshot(logical);
    const series_data = new Map<series_api, ohlc_data | single_value_data>();
    // Preserve the compatibility map's historical topmost-first insertion order while the rich
    // snapshot itself stays in canonical bottom-to-top series order.
    for (let index = value_snapshot.length - 1; index >= 0; index -= 1) {
      const entry = value_snapshot[index]!;
      if (entry.time === null) continue;
      if (entry.open !== null && entry.high !== null && entry.low !== null && entry.close !== null) {
        series_data.set(entry.series, {
          time: entry.time,
          open: entry.open,
          high: entry.high,
          low: entry.low,
          close: entry.close,
        });
      } else if (entry.value !== null) {
        series_data.set(entry.series, { time: entry.time, value: entry.value });
      }
    }
    const hovered_series = this.hover?.series_id != null
      ? this.series_handle(this.hover.series_id)
      : this.hover?.general_hit != null
        ? this.general_series_by_id.get(this.hover.general_hit.series) ?? null
        : null;
    return {
      time, logical, point: { x, y }, pane_index: this.pane_index_at(x, y), series_data, value_snapshot,
      hovered_series, general_hit: this.hover?.general_hit ?? null,
      hovered_object_id: this.hover?.object_id ?? null,
    };
  }

  /**
   * Refresh the hover hit-test state (Phase C-d) for a crosshair at pane CSS px (x, y): runs
   * the engine's series hit test plus the primitives' `hit_test`, stashes the result for
   * `build_params`, and updates the engine's hovered series for `hoveredSeriesOnTop` (the
   * caller repaints, so the z-bump lands on the next frame). Called by the gesture
   * recognizer on every crosshair move.
   */
  update_hover(x: number, y: number): void {
    const previous_object = this.hover?.object_id ?? null;
    this.hover = JSON.parse(this.wasm.hover_at(x, y)) as chart_impl["hover"];
    // The hover ring (text drawings' dimmed focus border) changes with the hovered object:
    // repaint exactly on transitions, even when the crosshair itself is hidden.
    if ((this.hover?.object_id ?? null) !== previous_object) this.repaint();
  }

  /** Clear the hover state (cursor left the chart) and release the z-bump; caller repaints. */
  clear_hover(): void {
    const had_object = this.hover?.object_id != null;
    this.hover = null;
    this.wasm.clear_hover();
    // A visible hover ring (drawing under the cursor) must paint out even if the caller
    // skips its own repaint.
    if (had_object) this.repaint();
  }

  /** The cursor a primitive's `hit_test` reports for the current hover, or `null`. */
  hover_cursor(): string | null {
    return this.hover?.cursor ?? null;
  }

  /** The engine-hit series under the current hover, or `null` (drives the pointer cursor). */
  hover_series_id(): number | null {
    return this.hover?.series_id ?? this.hover?.general_hit?.series ?? null;
  }

  /** Emit a crosshair-move event (called by the gesture recognizer). */
  emit_crosshair(x: number, y: number): void {
    this.last_crosshair = { x, y };
    if (this.crosshair_subs.size === 0) return;
    const params = this.build_params(x, y);
    for (const h of this.crosshair_subs) h(params);
  }
  /** Emit the "cursor left the chart" crosshair event (empty params). */
  emit_crosshair_left(): void {
    this.last_crosshair = null;
    if (this.crosshair_subs.size === 0) return;
    const params: mouse_event_params = {
      time: null, logical: null, point: null, pane_index: null, series_data: new Map(),
      value_snapshot: this.value_snapshot(),
      hovered_series: null, general_hit: null, hovered_object_id: null,
    };
    for (const h of this.crosshair_subs) h(params);
  }
  /**
   * Snapshot the drawing selection at pointer-down (gestures.ts, before the engine's drag
   * grab selects the hit). Drives the two-step text-editing rule in `emit_click`.
   */
  note_drawing_press(): void {
    this.text_press_selected = this.wasm.selected_drawing() ?? null;
  }

  /** Apply the chart-owned selection/editing work shared by click and drawing-owned double-click. */
  private apply_primary_click(x: number, y: number): void {
    // industry-standard click-to-select: select the series under the click (the frame build
    // paints anchor points on it) and clear the selection on empty pane space. The hover
    // hit-test refreshes at the click point first, so a click without a preceding move still
    // arbitrates correctly.
    this.update_hover(x, y);
    // industry-standard click-to-select, drawings first: a drawing hit selects it and clears
    // the series selection; a miss clears the drawing selection and falls through to the
    // series under the click (or clears that on empty pane space).
    // A drawing's own text under the click selects it (the engine answers for a trend line's
    // label and prompt and for the text of every line, channel, Fibonacci, pitchfork, pattern,
    // and shape tool, which an unselected shape's interior would otherwise not hit).
    const label_hit = Number(this.wasm.drawing_text_hit_at(x, y));
    const drawing_hit = label_hit > 0 || this.wasm.select_drawing_at(x, y);
    if (label_hit > 0) this.wasm.set_selected_drawing(label_hit);
    const general_hit = !drawing_hit && this.hover?.general_hit != null;
    if (general_hit) this.wasm.select_general_hovered();
    else this.wasm.clear_general_selection();
    this.wasm.set_selected_series(
      drawing_hit || general_hit ? undefined : (this.hover?.series_id ?? undefined),
    );
    // Text drawings: empty labels open typing mode on the first click (there is no ink to
    // "focus" otherwise). Non-empty labels follow the public reference's two-step model — first click
    // selects (focus border), a click opens typing mode only when already selected at press.
    // Every other drawing's text opens on a double-click, Enter, or F2 (`edit_drawing_text`).
    if (drawing_hit) {
      const selected = this.selected_drawing();
      if (selected !== null && selected.kind() === "trend_line" && selected.id === label_hit) {
        this.open_trend_label_editor(selected);
      } else if (selected !== null && selected.kind() === "text") {
        const empty = !(selected.options().text ?? "").trim();
        if (empty || this.text_press_selected === selected.id) {
          this.open_text_editor(selected);
        }
      }
    }
    this.repaint();
  }

  remove_general_series_handle(series: general_series_impl): void {
    const live = this.general_series_by_id.get(series.id);
    if (live !== series) return;
    const pane_index = this.panes().find((pane) => pane.get_series().includes(series))?.pane_index() ?? 0;
    if (!series.remove_from_engine()) return;
    series.mark_removed();
    this.general_series_by_id.delete(series.id);
    this.emit_series_change(this.series_removed_subs, series, pane_index);
    this.repaint();
  }

  /** Emit a click event (called by the gesture recognizer). */
  emit_click(x: number, y: number): void {
    // A click/tap is a discrete, intentional action, so it is a good moment to announce the point
    // to assistive tech (unlike mouse hover, which would flood the live region).
    this.announce(x, y);
    this.apply_primary_click(x, y);
    if (this.click_subs.size === 0) return;
    const params = this.build_params(x, y);
    for (const h of this.click_subs) h(params);
  }

  /**
   * Let an explicitly hit Aeris drawing consume the second click without a pane click event: the
   * text tool and trend labels re-run their click activation, and any other selected drawing
   * whose text the engine edits in place opens the inline editor. The first click of a pair on an
   * unselected drawing's text selects it (`apply_primary_click`), so a double-click on the text
   * of a line, channel, Fibonacci, pitchfork, pattern, or shape tool reaches the editor. Host
   * `dbl_click` subscribers still run afterwards.
   */
  activate_drawing_double_click(x: number, y: number): void {
    const selected = this.selected_drawing();
    if (selected === null || selected.id !== this.text_press_selected) return;
    // The press snapshot says the drawing was selected, not that this click is on it: a press
    // another owner consumed (a trading object, the alert widget) leaves the selection and the
    // snapshot behind. The engine says whether the point still belongs to the selected drawing.
    if (this.wasm.drawing_at(x, y) !== selected.id) return;
    if (selected.kind() === "text" || selected.kind() === "trend_line") {
      this.apply_primary_click(x, y);
    } else {
      this.edit_drawing_text(selected.id);
    }
  }

  /** Enter or F2 on the chart: edit the selected drawing's text in place, when it has one. */
  edit_selected_drawing_text(): boolean {
    const id = this.wasm.selected_drawing();
    return id !== undefined && this.edit_drawing_text(id);
  }

  /** Emit engine-resolved context without running primary-click selection or activation paths. */
  emit_chart_context(x: number, y: number): boolean {
    if (this.chart_context_subs.size === 0) return false;
    const context = this.wasm.chart_context_at(x, y);
    if (context.length !== 7) return false;
    const params = this.build_params(context[0]!, context[1]!) as chart_context_params;
    params.pane_index = context[2]!;
    params.time = Number.isNaN(context[3]!) ? null : context[3]!;
    params.logical = Number.isNaN(context[4]!) ? null : context[4]!;
    params.price = context[5]!;
    const series_id = context[6]!;
    params.hovered_series = Number.isNaN(series_id) ? null : this.series_handle(series_id);
    params.hovered_object_id = null;
    for (const handler of this.chart_context_subs) handler(params);
    return true;
  }

  /** Announce the current visible time range to assistive tech (used after keyboard navigation). */
  announce_view(): void {
    const r = this.wasm.visible_time_range();
    this.accessibility_handle?.announce(
      r.length === 2 ? `Showing time ${r[0]} to ${r[1]}` : "No data",
    );
  }

  emit_dbl_click(x: number, y: number): void {
    if (this.dbl_click_subs.size === 0) return;
    const params = this.build_params(x, y);
    for (const h of this.dbl_click_subs) h(params);
  }

  // ---------------------------------------------------------------------------------------------
  // Drawing tools (engine-owned drawing objects; the gesture layer routes presses/clicks/moves
  // through the creation helpers below, everything else is thin wasm delegation)
  // ---------------------------------------------------------------------------------------------

  add_drawing(
    kind: drawing_kind,
    points: drawing_point_input[],
    options?: Partial<drawing_options>,
    pane_index = 0,
  ): drawing_api {
    const wire_kind = DRAWING_KIND_TO_U8[kind];
    if (wire_kind === undefined) throw new AerisChartsError("invalid_data", `unknown drawing kind ${String(kind)}`);
    const { id } = drawing_result<{ id: number }>(this.wasm.add_drawing_result_json(
      wire_kind,
      pane_index,
      JSON.stringify(points),
      JSON.stringify(options ?? {}),
    ));
    this.repaint();
    return new drawing_impl(this, id, kind, pane_index);
  }

  set_drawings_points(updates: readonly drawing_points_update[]): number {
    const wire = updates.map((update) => ({
      id: typeof update.drawing === "number" ? update.drawing : update.drawing.id,
      points: update.points,
    }));
    const { changed } = drawing_result<{ changed: number }>(
      this.wasm.set_drawings_points_result_json(JSON.stringify(wire)),
    );
    if (changed > 0) this.repaint();
    return changed;
  }

  rescale_drawing_prices(segments: readonly drawing_price_segment[], price_basis?: string): number {
    const { changed } = drawing_result<{ changed: number }>(
      this.wasm.rescale_drawing_prices_result_json(JSON.stringify(segments), price_basis),
    );
    this.repaint();
    return changed;
  }

  set_drawing_price_basis(basis: string | null): void {
    if (!this.wasm.set_drawing_price_basis(basis ?? undefined)) {
      throw new AerisChartsError("invalid_options", "drawing price basis must be at most 128 bytes");
    }
  }

  drawing_price_basis(): string | null {
    return this.wasm.drawing_price_basis() ?? null;
  }

  set_drawing_magnet_mode(mode: drawing_magnet_mode): void {
    const wire = DRAWING_MAGNET_TO_U8[mode];
    if (wire === undefined || !this.wasm.set_drawing_magnet_mode(wire)) {
      throw new AerisChartsError("invalid_options", `unknown drawing magnet mode ${String(mode)}`);
    }
    this.repaint();
  }

  drawing_magnet_mode(): drawing_magnet_mode {
    return DRAWING_MAGNET_FROM_U8[this.wasm.drawing_magnet_mode()] ?? "off";
  }

  /** Whether the armed tool's next placement snaps strongly (crosshair magnet mirror). */
  armed_drawing_magnet_strong(toggle: boolean): boolean {
    return this.wasm.armed_drawing_magnet(toggle) === 2;
  }

  drawings(): drawing_api[] {
    const list = JSON.parse(this.wasm.drawings_json()) as drawing_info[];
    return list.map((d) => new drawing_impl(this, d.id, d.kind, d.pane_index));
  }

  drawing_property_schema(drawing: drawing_api | number): drawing_property_schema {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    const value = this.wasm.drawing_property_schema_json(id);
    if (value === "") throw new AerisChartsError("stale_handle", "drawing has been removed");
    return JSON.parse(value) as drawing_property_schema;
  }

  drawing_kind_options(drawing: drawing_api | number): drawing_kind_options {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    const value = this.wasm.drawing_kind_options_json(id);
    if (value === "") throw new AerisChartsError("stale_handle", "drawing has been removed");
    return JSON.parse(value) as drawing_kind_options;
  }

  drawing_object_tree(): unknown[] {
    return JSON.parse(this.wasm.drawing_object_tree_json()) as unknown[];
  }

  set_drawing_interval(interval: drawing_interval | null): void {
    if (!this.wasm.set_drawing_interval(interval === null ? "" : JSON.stringify(interval))) {
      throw new AerisChartsError("invalid_options", "drawing interval metadata is invalid");
    }
    this.repaint();
  }

  copy_drawings(ids: readonly number[] = []): string {
    return parse_engine_result<{ payload: string }>(this.wasm.copy_drawings_json(JSON.stringify(ids))).payload;
  }

  paste_drawings(payload: string, pane_index = 0, logical_offset = 0, price_offset = 0): drawing_api[] {
    const ids = JSON.parse(this.wasm.paste_drawings_json(payload, pane_index, logical_offset, price_offset)) as number[];
    if (ids.length === 0) throw new AerisChartsError("invalid_data", "drawing payload was rejected");
    const list = JSON.parse(this.wasm.drawings_json()) as drawing_info[];
    this.repaint();
    return ids
      .map((id) => list.find((drawing) => drawing.id === id))
      .filter((drawing): drawing is drawing_info => drawing !== undefined)
      .map((drawing) => new drawing_impl(this, drawing.id, drawing.kind, drawing.pane_index));
  }

  clone_drawing(drawing: drawing_api | number, logical_offset = 0, price_offset = 0): drawing_api {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    const cloned = this.wasm.clone_drawing(id, logical_offset, price_offset);
    if (cloned === 0) throw new AerisChartsError("invalid_data", "drawing could not be cloned");
    const info = (JSON.parse(this.wasm.drawings_json()) as drawing_info[]).find((entry) => entry.id === cloned);
    if (info === undefined) throw new AerisChartsError("renderer_platform_error", "cloned drawing is missing");
    this.repaint();
    return new drawing_impl(this, info.id, info.kind, info.pane_index);
  }

  move_drawing_z_order(drawing: drawing_api | number, delta: number): boolean {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    const changed = this.wasm.move_drawing_z_order(id, delta);
    if (changed) this.repaint();
    return changed;
  }

  set_drawing_group_visibility(group_id: string, visible: boolean): number {
    const changed = this.wasm.set_drawing_group_visibility(group_id, visible);
    if (changed !== 0) this.repaint();
    return changed;
  }

  set_drawing_group_locked(group_id: string, locked: boolean): number {
    const changed = this.wasm.set_drawing_group_locked(group_id, locked);
    if (changed !== 0) this.repaint();
    return changed;
  }

  move_drawing_group(group_id: string, logical_delta: number, price_delta: number): number {
    const changed = this.wasm.move_drawing_group(group_id, logical_delta, price_delta);
    if (changed !== 0) this.repaint();
    return changed;
  }

  apply_drawing_template(drawing: drawing_api | number, template: drawing_template): void {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    if (!this.wasm.apply_drawing_template_json(id, JSON.stringify(template))) {
      throw new AerisChartsError("invalid_options", "drawing template was rejected");
    }
    this.repaint();
  }

  drawing_template(drawing: drawing_api | number, name: string): drawing_template {
    const id = typeof drawing === "number" ? drawing : drawing.id;
    const value = this.wasm.drawing_template_json(id, name);
    if (value === "") throw new AerisChartsError("invalid_options", "drawing template is invalid");
    return JSON.parse(value) as drawing_template;
  }

  drawing_sync_payload(source: string): string {
    const payload = this.wasm.drawing_sync_payload_json(source);
    if (payload === "") throw new AerisChartsError("invalid_options", "drawing sync source is invalid");
    return payload;
  }

  apply_drawing_sync_payload(payload: string): boolean {
    const changed = this.wasm.apply_drawing_sync_payload_json(payload);
    if (changed) this.repaint();
    return changed;
  }

  export_state(): chart_state {
    const result = JSON.parse(this.wasm.export_state_result_json()) as
      | { ok: true; document: string }
      | persistence_error_result;
    if (!result.ok) throw_persistence_error(result);
    return JSON.parse(result.document) as chart_state;
  }

  import_state(state: chart_state | string): persistence_restore_result {
    let document: string;
    try {
      document = typeof state === "string" ? state : JSON.stringify(state);
    } catch (error) {
      throw new AerisChartsError("serialization_error", `state is not JSON-serializable: ${error}`);
    }
    const response = JSON.parse(this.wasm.import_state_result_json(document)) as
      | { ok: true; result: persistence_restore_result }
      | persistence_error_result;
    if (!response.ok) throw_persistence_error(response);
    if (response.result.schema_version === 2) {
      const catalog = JSON.parse(this.wasm.general_series_catalog_json()) as
        { id: number; dataset: number; kind: general_series_kind; x_axis_id: string; y_axis_id: string }[];
      this.general_series_by_id.clear();
      for (const entry of catalog) {
        this.general_series_by_id.set(
          entry.id,
          new general_series_impl(
            entry.id,
            entry.dataset,
            entry.kind,
            entry.x_axis_id,
            entry.y_axis_id,
            this,
          ),
        );
      }
    }
    this.repaint();
    return response.result;
  }

  clear_drawings(): void {
    this.wasm.clear_drawings();
    this.repaint();
  }

  announce_trading_intent(intent: trading_intent): void {
    const target = intent.order_id ?? intent.position_id ?? "trading object";
    const price = intent.price === undefined ? "" : ` at ${intent.price}`;
    this.accessibility_handle?.announce(
      `Trading request ${intent.action.replaceAll("_", " ")} for ${target}${price}, awaiting confirmation`,
    );
  }

  undo_drawing(): boolean {
    this.close_text_editor(true);
    const changed = this.wasm.undo_drawing();
    if (changed) this.repaint();
    return changed;
  }

  redo_drawing(): boolean {
    this.close_text_editor(true);
    const changed = this.wasm.redo_drawing();
    if (changed) this.repaint();
    return changed;
  }

  can_undo_drawing(): boolean {
    return this.wasm.can_undo_drawing();
  }

  can_redo_drawing(): boolean {
    return this.wasm.can_redo_drawing();
  }

  set_drawing_tool(
    tool: drawing_kind | null,
    options?: Partial<drawing_options>,
    pane_index?: number,
  ): void {
    const previous_tool = this.active_drawing_tool();
    const previous_pane_wire = Number(this.wasm.active_drawing_tool_pane());
    const previous_pane = previous_pane_wire >= 0 ? previous_pane_wire : null;
    const next_pane = tool === null
      ? null
      : (pane_index ?? (previous_tool === tool ? previous_pane : null));
    const changed = previous_tool !== tool || previous_pane !== next_pane;
    if (changed) this.close_text_editor(true); // arming another tool commits the edit
    if (options !== undefined) {
      this.tool_options_json = JSON.stringify(options);
    }
    if (!changed && tool !== null && options !== undefined) {
      if (!this.wasm.drawing_tool_apply_options(this.tool_options_json)) {
        throw new AerisChartsError("invalid_options", "drawing tool options are malformed");
      }
    } else if (changed || tool === null) {
      const wire_kind = tool === null ? -1 : DRAWING_KIND_TO_U8[tool];
      if (!this.wasm.set_drawing_tool(wire_kind, this.tool_options_json, next_pane ?? -1)) {
        throw new AerisChartsError("invalid_options", "drawing tool options are malformed");
      }
    }
    if (changed) {
      this.tool_listener?.(tool);
      for (const handler of this.tool_change_subs) handler(tool);
    }
  }

  active_drawing_tool(): drawing_kind | null {
    const wire = Number(this.wasm.active_drawing_tool());
    return wire >= 0 ? (DRAWING_KIND_FROM_U8.get(wire) ?? null) : null;
  }

  set_drawing_tool_listener(listener: ((tool: drawing_kind | null) => void) | null): void {
    this.tool_listener = listener;
  }

  subscribe_drawing_tool_change(handler: drawing_tool_change_handler): void {
    this.tool_change_subs.add(handler);
  }

  unsubscribe_drawing_tool_change(handler: drawing_tool_change_handler): void {
    this.tool_change_subs.delete(handler);
  }

  subscribe_drawing_created(handler: drawing_created_handler): void {
    this.drawing_created_subs.add(handler);
  }

  unsubscribe_drawing_created(handler: drawing_created_handler): void {
    this.drawing_created_subs.delete(handler);
  }

  selected_drawing(): drawing_api | null {
    const id = this.wasm.selected_drawing();
    if (id === undefined) return null;
    const info = (JSON.parse(this.wasm.drawings_json()) as drawing_info[]).find((d) => d.id === id);
    return info ? new drawing_impl(this, info.id, info.kind, info.pane_index) : null;
  }

  /** Whether an interactive drawing tool is armed (the recognizer routes pane clicks to creation). */
  creation_armed(): boolean {
    return this.active_drawing_tool() !== null;
  }

  /** Whether an interactive creation is mid-placement/capture in the engine controller. */
  creation_active(): boolean {
    return this.wasm.drawing_create_active() || this.wasm.drawing_tool_capture_active();
  }

  /** Whether the current creation owns a captured pointer stream (freehand today, extensible). */
  creation_capture_active(): boolean {
    return this.wasm.drawing_tool_capture_active();
  }

  /** Whether the armed placement is a variable sequence requiring explicit finish. */
  creation_sequence_active(): boolean {
    return this.wasm.drawing_tool_sequence_active();
  }

  private drawing_created(created_id: number): boolean {
    if (created_id <= 0) return false;
    const info = (JSON.parse(this.wasm.drawings_json()) as drawing_info[]).find((drawing) => drawing.id === created_id);
    if (info === undefined) return false;
    const created = new drawing_impl(this, info.id, info.kind, info.pane_index);
    for (const handler of this.drawing_created_subs) handler(created);
    // One-shot disarming happened inside the engine controller; mirror that public state change.
    this.tool_listener?.(null);
    for (const handler of this.tool_change_subs) handler(null);
    // The engine decides which tools start in the editor (the text tool and the annotation boxes
    // that begin from a default text); the host presents the one its layout describes.
    if (this.wasm.drawing_requests_text_edit(created_id)) this.edit_drawing_text(created_id);
    return true;
  }

  /** Forward an armed-tool pointer press. Returns true only when placement committed on press. */
  creation_pointer_down(x: number, y: number, magnet = false, straighten = false): boolean {
    if (!this.creation_armed() || this.pane_index_at(x, y) === null) return false;
    return this.drawing_created(Number(this.wasm.drawing_tool_pointer_down(x, y, magnet, straighten)));
  }

  /** Forward an armed-tool pointer move; `pressed` identifies an active captured drag stream. */
  creation_pointer_move(
    x: number,
    y: number,
    magnet = false,
    straighten = false,
    pressed = false,
  ): boolean {
    if (!this.creation_armed()) return false;
    return this.wasm.drawing_tool_pointer_move(x, y, magnet, straighten, pressed);
  }

  /** Forward pointer release; returns true when the release committed a drawing. */
  creation_pointer_up(x: number, y: number, magnet = false, straighten = false): boolean {
    if (!this.creation_armed()) return false;
    return this.drawing_created(Number(this.wasm.drawing_tool_pointer_up(x, y, magnet, straighten)));
  }

  /** Route one click/tap activation into the canonical placement state machine. */
  creation_click(x: number, y: number, magnet = false, straighten = false): boolean {
    const pane = this.pane_index_at(x, y);
    if (!this.creation_armed() || pane === null) return false;
    const pinned_pane = Number(this.wasm.active_drawing_tool_pane());
    if (pinned_pane >= 0 && pane !== pinned_pane) return false;
    this.drawing_created(Number(this.wasm.drawing_tool_activate(x, y, magnet, straighten)));
    return true;
  }

  /** Commit any active variable-sequence drawing (double-click/Enter). */
  creation_finish(): boolean {
    return this.drawing_created(Number(this.wasm.drawing_tool_finish()));
  }

  /** Remove the latest placed vertex from an active variable-sequence drawing. */
  creation_pop_anchor(): boolean {
    return this.wasm.drawing_tool_pop_anchor();
  }

  /** Abort an interrupted pointer capture while leaving the selected tool armed. */
  cancel_active_drawing_creation(): void {
    this.wasm.cancel_drawing_creation();
  }

  /** Escape: disarm the tool (cancelling any pending creation) and deselect any drawing. */
  cancel_drawing_interaction(): void {
    const changed = this.active_drawing_tool() !== null;
    this.wasm.cancel_drawing_tool();
    if (changed) {
      this.tool_listener?.(null);
      for (const handler of this.tool_change_subs) handler(null);
    }
    this.wasm.set_selected_drawing(undefined);
  }

  // ---------------------------------------------------------------------------------------------
  // Inline drawing editors. Every drawing that paints text edits it through one engine session
  // and the borderless caret surface: live text repaints without history, commit records one undo
  // step, and cancel restores the pre-edit text. The engine's `drawing_text_edit_layout` says
  // where the text sits and whether it is a run (the text tool, trend labels, and the text of
  // lines, channels, Fibonacci tools, pitchforks, patterns, and shapes: one line, possibly
  // rotated) or a family text box (several lines); the host only presents the matching surface.
  // The text tool alone owns an empty lifecycle (leaving it empty removes the drawing).
  // ---------------------------------------------------------------------------------------------

  /**
   * Open the typing-mode editor for a text drawing (the public reference's overlay-caret model): a
   * borderless wrap whose glyphs are TRANSPARENT — the engine keeps painting both the label
   * and the focus border underneath, so entering edit cannot lift the text or shift the
   * outline. Each keystroke pushes the text into the engine live; Enter/blur commits, Escape
   * restores the pre-edit text. Leaving empty removes the drawing.
   */
  open_text_editor(drawing: drawing_api): void {
    this.open_inline_editor(drawing, "standalone_text");
  }

  private open_trend_label_editor(drawing: drawing_api): void {
    this.open_inline_editor(drawing, "run_label");
  }

  /**
   * Open the inline editor on a drawing's own text (Enter or F2 on a selected drawing, a
   * double-click on it, or placement of a tool that starts in the editor): whatever the engine
   * reports editable, presented as the surface its layout describes. Refuses a drawing whose
   * text is entirely outside the chart, so no invisible editor captures the keys. Returns whether
   * an editor opened.
   */
  edit_drawing_text(id: number): boolean {
    if (!this.wasm.drawing_text_editable(id)) return false;
    const layout = this.text_edit_layout(id);
    if (layout === null || !this.text_in_view(layout)) return false;
    const info = (JSON.parse(this.wasm.drawings_json()) as drawing_info[]).find((d) => d.id === id);
    if (info === undefined) return false;
    const drawing = new drawing_impl(this, info.id, info.kind, info.pane_index);
    if (layout.multiline) this.open_part_label_editor(drawing);
    else this.open_inline_editor(drawing, info.kind === "text" ? "standalone_text" : "run_label");
    return this.text_editor !== null && this.text_editor_id === id;
  }

  /** Whether any part of the layout's text box is inside the chart's visible area. */
  private text_in_view(layout: text_edit_layout): boolean {
    const width = this.overlay.clientWidth;
    const height = this.overlay.clientHeight;
    if (width <= 0 || height <= 0) return true;
    const [left, top, right, bottom] = layout.rect;
    return right > 0 && left < width && bottom > 0 && top < height;
  }

  /**
   * The caret surface every editor mode shares: a borderless wrap holding a fully transparent
   * editable run (glyphs, IME composition, and browser selection all invisible, so the engine's
   * canvas label stays the only ink) and one explicit caret in the label's ink. Single-line text
   * edits a content-editable run; multi-line text a native `textarea`, which keeps a caret
   * position after a trailing line break and pastes plain text.
   */
  private text_editor_surface(
    text: string,
    font: string,
    font_size: number,
    line_height: number,
    ink: string,
    label: string,
    multiline: boolean,
  ): { wrap: HTMLDivElement; editor: HTMLElement; caret: HTMLSpanElement } {
    const wrap = document.createElement("div");
    wrap.id = "aeris_charts-text-editor";
    // The accessibility layer leaves this class reachable (it hides every other chart control).
    wrap.className = "aeris_charts-text-editor";
    wrap.style.position = "absolute";
    wrap.style.zIndex = "10";
    // Borderless: the engine paints the focus border and any box continuously (selected +
    // editing), so the outline never swaps owners or shifts when entering typing mode. This wrap
    // only carries the transparent caret overlay.
    wrap.style.border = "none";
    wrap.style.borderRadius = "0";
    wrap.style.background = "transparent";
    wrap.style.padding = "0";
    wrap.style.outline = "none";

    let editor: HTMLElement;
    if (multiline) {
      const area = document.createElement("textarea");
      area.value = text;
      area.wrap = "off";
      area.spellcheck = false;
      area.style.resize = "none";
      area.style.overflow = "hidden";
      editor = area;
    } else {
      editor = document.createElement("div");
      editor.contentEditable = "true";
      editor.setAttribute("role", "textbox");
      editor.textContent = text;
    }
    editor.id = "aeris_charts-text-input";
    editor.setAttribute("aria-label", label);
    editor.style.font = font;
    editor.style.lineHeight = `${line_height}px`;
    // The editing surface is fully transparent, including IME composition glyphs. Chromium can
    // otherwise repaint composing text in `caret-color` despite transparent text fill, producing
    // a second unrotated label. A dedicated one-pixel caret below is the only DOM ink.
    editor.style.color = "transparent";
    editor.style.caretColor = "transparent";
    (editor.style as CSSStyleDeclaration & { webkitTextFillColor?: string }).webkitTextFillColor =
      "transparent";
    editor.style.opacity = "0";
    editor.style.background = "transparent";
    editor.style.border = "none";
    editor.style.outline = "none";
    editor.style.padding = "0";
    editor.style.margin = "0";
    editor.style.display = "block";
    editor.style.whiteSpace = multiline ? "pre" : "nowrap";
    editor.style.minWidth = `${font_size}px`;

    const caret = document.createElement("span");
    caret.id = "aeris_charts-text-caret";
    caret.setAttribute("aria-hidden", "true");
    caret.style.position = "absolute";
    caret.style.top = "0";
    caret.style.width = "1px";
    caret.style.height = `${font_size * 1.2}px`;
    caret.style.background = ink;
    caret.style.pointerEvents = "none";

    // Selection is painted by the browser in a separate phase and can remain visible even when
    // the editable element itself is transparent. Suppress it locally so a selected/composing
    // label cannot place theme-colored blocks over the canonical canvas glyphs.
    const selection_style = document.createElement("style");
    selection_style.textContent =
      "#aeris_charts-text-input::selection{background:transparent!important;color:transparent!important;-webkit-text-fill-color:transparent!important;text-shadow:none!important}";

    wrap.appendChild(editor);
    wrap.appendChild(caret);
    wrap.appendChild(selection_style);
    return { wrap, editor, caret };
  }

  /** The editor surface's text. */
  private static text_editor_value(editor: HTMLElement): string {
    return editor instanceof HTMLTextAreaElement ? editor.value : (editor.textContent ?? "");
  }

  /** The editor's caret offset in its text (the text length when the selection is elsewhere). */
  private static text_editor_caret_offset(editor: HTMLElement): number {
    if (editor instanceof HTMLTextAreaElement) return editor.selectionEnd;
    const selection = window.getSelection();
    if (selection?.anchorNode && editor.contains(selection.anchorNode)) {
      const prefix = document.createRange();
      prefix.selectNodeContents(editor);
      prefix.setEnd(selection.anchorNode, selection.anchorOffset);
      return prefix.toString().length;
    }
    return (editor.textContent ?? "").length;
  }

  /**
   * Hand the mounted surface the session: Enter (outside IME composition) and blur commit and
   * Escape cancels; a multi-line editor keeps Shift+Enter for its native line break. The caret
   * starts at the end of the text and focus returns where it was on close.
   */
  private attach_text_editor(
    drawing: drawing_api,
    mode: "standalone_text" | "run_label" | "part_label",
    editor: HTMLElement,
    return_focus: Element | null,
    on_input: () => void,
  ): void {
    const multiline = mode === "part_label";
    editor.addEventListener("input", on_input);
    editor.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.isComposing) return;
      if (e.key === "Enter" && !(multiline && e.shiftKey)) {
        e.preventDefault();
        this.close_text_editor(true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        this.close_text_editor(false);
      }
    });
    // Focus that moved to another element (a host panel or control) stays there: the editor
    // commits, and the chart does not pull focus back.
    editor.addEventListener("blur", (e) => this.close_text_editor(true, e.relatedTarget instanceof Element));

    this.text_editor = editor;
    this.text_editor_id = drawing.id;
    this.text_editor_original = drawing.options().text ?? "";
    this.text_editor_mode = mode;
    this.text_editor_label = drawing.kind().replaceAll("_", " ");
    this.text_editor_return_focus = return_focus;
    this.repaint();
    editor.focus();
    if (editor instanceof HTMLTextAreaElement) {
      editor.setSelectionRange(editor.value.length, editor.value.length);
    } else {
      const selection = window.getSelection();
      if (selection !== null) {
        const range = document.createRange();
        range.selectNodeContents(editor);
        range.collapse(false);
        selection.removeAllRanges();
        selection.addRange(range);
      }
    }
    this.accessibility_handle?.announce(
      `${this.text_editor_label} text editing. Enter commits${multiline ? ", Shift+Enter adds a line" : ""}, Escape cancels.`,
    );
  }

  /**
   * Open the single-line editor on a drawing's text run. The engine's layout gives the run's
   * start point (left edge, vertical center), rotation, glyph size, font, and ink, and is read
   * again after every keystroke, so the caret overlay cannot drift from the painted label: the
   * host holds no placement, alignment, or color rule of its own.
   */
  private open_inline_editor(
    drawing: drawing_api,
    mode: "standalone_text" | "run_label",
  ): void {
    this.close_text_editor(true);
    const return_focus = document.activeElement;
    // The engine session keeps the canvas label under the caret overlay and records the edit as
    // one undo step; it refuses a locked, hidden, or otherwise uneditable drawing.
    if (!this.wasm.begin_drawing_text_edit(drawing.id)) return;
    let layout = this.text_edit_layout(drawing.id);
    if (layout === null || layout.multiline) {
      this.wasm.end_drawing_text_edit(false);
      return;
    }
    const options = drawing.options();
    const font_of = (edit: text_edit_layout) =>
      `${edit.italic ? "italic " : ""}${edit.weight} ${edit.size}px ${edit.font_family}`;
    const font = font_of(layout);
    const font_size = layout.size;
    const { wrap, editor, caret } = this.text_editor_surface(
      options.text,
      font,
      font_size,
      layout.line_height,
      layout.color,
      `${drawing.kind().replaceAll("_", " ")} text`,
      false,
    );
    // Only the text tool paints a container behind its run.
    if (mode === "standalone_text") wrap.style.background = options.box_color || "transparent";

    const dpr = window.devicePixelRatio || 1;
    const measure_ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
    let baseline_drop = 0;
    if (measure_ctx !== null) {
      const device_font = `${layout.italic ? "italic " : ""}${layout.weight} ${font_size * dpr}px ${layout.font_family}`;
      const side = Math.ceil(font_size * dpr) + 16;
      const probe_canvas = measure_ctx.canvas;
      probe_canvas.width = side;
      probe_canvas.height = side;
      measure_ctx.font = device_font;
      measure_ctx.textBaseline = "middle";
      measure_ctx.fillStyle = "#fff";
      measure_ctx.fillText("H", 8, side / 2);
      const pixels = measure_ctx.getImageData(0, 0, side, side).data;
      let bottom_row = -1;
      let bottom_alpha = 0;
      for (let row = side - 1; row >= 0; row -= 1) {
        let row_alpha = 0;
        for (let col = 0; col < side; col += 1) {
          const alpha = pixels[(row * side + col) * 4 + 3]!;
          if (alpha > row_alpha) row_alpha = alpha;
        }
        if (row_alpha >= 24) {
          bottom_row = row;
          bottom_alpha = row_alpha;
          break;
        }
      }
      if (bottom_row >= 0) {
        baseline_drop = (bottom_row + Math.min(bottom_alpha / 255, 1) - side / 2) / dpr;
      }
      probe_canvas.width = 0;
      probe_canvas.height = 0;
    }
    let baseline_in_editor = 0;
    const position_caret = () => {
      const text = editor.textContent ?? "";
      const before_caret = text.slice(0, chart_impl.text_editor_caret_offset(editor));
      if (measure_ctx !== null) measure_ctx.font = font;
      const x = measure_ctx === null ? 0 : measure_ctx.measureText(before_caret).width;
      caret.style.left = `${Math.ceil(x)}px`;
    };
    // The wrap's left-middle sits on the run's start point and rotates about it, so the DOM run
    // lies exactly on the painted one whatever its alignment or slope.
    const position_editor = () => {
      const edit = layout!;
      const middle = baseline_in_editor - baseline_drop;
      wrap.style.left = `${edit.x}px`;
      wrap.style.top = `${edit.y - middle}px`;
      wrap.style.transformOrigin = `0px ${middle}px`;
      wrap.style.transform = `rotate(${edit.angle}rad)`;
      caret.style.background = edit.color;
      position_caret();
    };
    // Re-read the engine's layout: the run may grow, shift, or slide along its stroke as the text
    // changes. A layout that vanished (drawing removed or replaced) ends the session.
    const relayout = () => {
      const fresh = this.wasm.editing_drawing() === drawing.id ? this.text_edit_layout(drawing.id) : null;
      if (fresh === null || fresh.multiline) {
        this.close_text_editor(false);
        return;
      }
      layout = fresh;
      position_editor();
    };
    const push_live_text = () => {
      // The engine owns the single-line rule and the length bound.
      if (!this.wasm.set_drawing_edit_text(editor.textContent ?? "")) {
        this.close_text_editor(false);
        return;
      }
      relayout();
      this.repaint();
    };
    const set_width = () => {
      const text = editor.textContent ?? "";
      if (measure_ctx !== null) {
        measure_ctx.font = font;
        const w = text === "" ? font_size : measure_ctx.measureText(text).width;
        editor.style.width = `${Math.ceil(w) + 1}px`;
      }
      push_live_text();
    };
    editor.addEventListener("keyup", position_caret);
    editor.addEventListener("pointerup", position_caret);

    this.container.appendChild(wrap);
    const probe = document.createElement("span");
    probe.style.display = "inline-block";
    probe.style.width = "0";
    probe.style.height = "0";
    editor.appendChild(probe);
    baseline_in_editor = probe.getBoundingClientRect().top - editor.getBoundingClientRect().top;
    probe.remove();

    // Width without a live push yet (avoids a redundant apply of the same text).
    if (measure_ctx !== null) {
      measure_ctx.font = font;
      const original = options.text ?? "";
      const w = original === "" ? font_size : measure_ctx.measureText(original).width;
      editor.style.width = `${Math.ceil(w) + 1}px`;
    }
    position_editor();

    this.text_editor_reposition = () => {
      if (this.text_editor !== null) relayout();
    };
    this.attach_text_editor(drawing, mode, editor, return_focus, set_width);
    position_caret();
  }

  /**
   * Open the editor on a family text box: the engine resolves where the box's own text sits (its
   * left edge, the first text line's center, and the line advance, below any engine line such as
   * a price label's price) and relays it out after every keystroke, since a box can grow in any
   * direction (a comment grows upward, a centered callout both ways). The text may span lines.
   */
  private open_part_label_editor(drawing: drawing_api): void {
    this.close_text_editor(true);
    const return_focus = document.activeElement;
    if (!this.wasm.begin_drawing_text_edit(drawing.id)) return;
    let layout = this.text_edit_layout(drawing.id);
    if (layout === null) {
      this.wasm.end_drawing_text_edit(false);
      return;
    }
    const font_of = (edit: text_edit_layout) =>
      `${edit.italic ? "italic " : ""}${edit.weight} ${edit.size}px ${edit.font_family}`;
    const { wrap, editor, caret } = this.text_editor_surface(
      drawing.options().text ?? "",
      font_of(layout),
      layout.size,
      layout.line_height,
      layout.color,
      `${drawing.kind().replaceAll("_", " ")} text`,
      true,
    );
    const measure_ctx = document.createElement("canvas").getContext("2d");
    const measure = (text: string) => {
      if (measure_ctx === null) return 0;
      measure_ctx.font = font_of(layout!);
      return measure_ctx.measureText(text).width;
    };
    const text_of = () => chart_impl.text_editor_value(editor).replace(/\r\n?/g, "\n");
    const position = () => {
      const edit = layout!;
      const font = font_of(edit);
      editor.style.font = font;
      editor.style.lineHeight = `${edit.line_height}px`;
      editor.style.minWidth = `${edit.size}px`;
      const text = text_of();
      const lines = text.split("\n");
      const widest = lines.reduce((width, line) => Math.max(width, measure(line)), 0);
      editor.style.width = `${Math.ceil(Math.max(widest, edit.size)) + 1}px`;
      editor.style.height = `${lines.length * edit.line_height}px`;
      wrap.style.left = `${edit.x}px`;
      wrap.style.top = `${edit.y - edit.line_height / 2}px`;
      const before = text.slice(0, chart_impl.text_editor_caret_offset(editor)).split("\n");
      const row = before.length - 1;
      caret.style.left = `${Math.ceil(measure(before[row] ?? ""))}px`;
      caret.style.top = `${row * edit.line_height + (edit.line_height - edit.size * 1.2) / 2}px`;
      caret.style.height = `${edit.size * 1.2}px`;
      caret.style.background = edit.color;
    };
    const push_live_text = () => {
      if (!this.wasm.set_drawing_edit_text(text_of())) {
        this.close_text_editor(false);
        return;
      }
      // The repaint relays the box out through `text_editor_reposition`.
      this.repaint();
    };
    editor.addEventListener("keyup", position);
    editor.addEventListener("pointerup", position);
    this.container.appendChild(wrap);
    position();
    this.text_editor_reposition = () => {
      if (this.text_editor === null) return;
      const fresh = this.wasm.editing_drawing() === drawing.id ? this.text_edit_layout(drawing.id) : null;
      if (fresh === null) {
        this.close_text_editor(false);
        return;
      }
      layout = fresh;
      position();
    };
    this.attach_text_editor(drawing, "part_label", editor, return_focus, push_live_text);
    position();
  }

  /** The engine's editor layout of a family text box, or null when it paints none. */
  private text_edit_layout(id: number): text_edit_layout | null {
    const json = this.wasm.drawing_text_edit_layout_json(id);
    return json === "" ? null : (JSON.parse(json) as text_edit_layout);
  }

  /**
   * Close the typing-mode editor. Commit keeps the typed text as one undo step; cancel restores
   * the pre-edit text. A standalone text left empty is removed. Focus returns to where the
   * editor was opened from inside the chart (an accessibility target), else the overlay, unless
   * `focus_moved`: the editor lost focus to another element, which keeps it (taking focus back
   * would cancel the host's own `focus()` call, as a blur handler moving focus does).
   */
  close_text_editor(commit: boolean, focus_moved = false): void {
    const editor = this.text_editor;
    if (editor === null) return;
    const mode = this.text_editor_mode;
    this.text_editor = null;
    this.text_editor_reposition = null;
    const wrap = this.container.querySelector("#aeris_charts-text-editor");
    wrap?.remove();
    const id = this.text_editor_id;
    const raw = chart_impl.text_editor_value(editor);
    const text = (mode === "part_label" ? raw.replace(/\r\n?/g, "\n") : raw.replace(/\s*\n\s*/g, " ")).trim();
    if (commit) this.wasm.set_drawing_edit_text(text);
    // A session the engine already ended (the drawing was removed, restored, or replaced by a
    // sync payload) is not this editor's to finish.
    const ended = this.wasm.end_drawing_text_edit(commit);
    if (ended && mode === "standalone_text" && !(commit ? text : this.text_editor_original.trim())) {
      this.wasm.remove_drawing(id);
    }
    const label = this.text_editor_label;
    const return_focus = this.text_editor_return_focus;
    this.text_editor_original = "";
    this.text_editor_mode = null;
    this.text_editor_label = "";
    this.text_editor_return_focus = null;
    this.repaint();
    if (ended) this.accessibility_handle?.announce(`${label} text ${commit ? "committed" : "edit cancelled"}.`);
    // An accessibility target rebuilt while editing is found again by its target key.
    const target = return_focus instanceof HTMLElement ? return_focus.dataset.a11yTarget : undefined;
    const back = return_focus instanceof HTMLElement && return_focus.isConnected
      ? return_focus
      : target === undefined
        ? null
        : this.container.querySelector<HTMLElement>(`[data-a11y-target="${CSS.escape(target)}"]`);
    if (focus_moved) return;
    if (back !== null && back !== this.overlay_el() && this.container.contains(back)) back.focus();
    else this.overlay_el().focus();
  }

  apply_options(options: deep_partial<chart_options>): void {
    // handle_scroll / handle_scale / kinetic_scroll / tracking_mode (gestures), the pane-resize
    // toggle, and localization (JS callbacks) are package-level; intercept and strip them so only
    // engine-owned, JSON-serializable options reach the wasm store.
    const { theme, handle_scroll, handle_scale, kinetic_scroll, wheel_behavior, tracking_mode, localization, accessibility, ...rest } =
      options as deep_partial<chart_options> & {
        theme?: theme_name;
        handle_scroll?: boolean | handle_scroll_options;
        handle_scale?: boolean | handle_scale_options;
        kinetic_scroll?: boolean | kinetic_scroll_options;
        wheel_behavior?: "auto" | "pan" | "zoom";
        tracking_mode?: tracking_mode_options;
        localization?: localization_options;
        accessibility?: boolean | accessibility_options;
      };
    // IANA time-zone names resolve here (the engine accepts only explicit schedules). Validate
    // and apply them before any other key so a rejected zone changes nothing.
    const exchange = split_exchange_time_options(rest as Record<string, unknown>);
    if (
      exchange.zone !== undefined || exchange.session_start !== undefined ||
      exchange.tick_marks !== undefined || exchange.bar_time_label !== undefined
    ) {
      this.apply_exchange_time(exchange.zone, exchange.session_start, exchange.tick_marks, exchange.bar_time_label);
    }
    let engine_options: Record<string, unknown> = exchange.engine;
    // layout.panes.enableResize (reference) drives the separator drag here, not the engine; strip it
    // alongside the other gesture keys before forwarding.
    const panes = (engine_options.layout as { panes?: { enableResize?: boolean } } | undefined)?.panes;
    if (panes?.enableResize !== undefined) {
      const { enableResize, ...panes_rest } = panes;
      engine_options = { ...engine_options, layout: { ...(engine_options.layout as object), panes: panes_rest } };
      this.gestures_cfg.panes_resize = enableResize;
    }
    // Theme selection is package-owned state. Apply its canonical projection first so explicit
    // engine options in the same patch can still override individual visual fields, matching
    // create_chart() ordering. Retaining the identity lets reset_style_to_defaults() return to
    // the selected theme even after arbitrary color customizations.
    if (theme !== undefined) {
      this.selected_theme = theme;
      this.wasm.apply_options(JSON.stringify(theme_options(theme_palette(theme))));
    }
    if (
      handle_scroll !== undefined || handle_scale !== undefined || kinetic_scroll !== undefined ||
      tracking_mode !== undefined
    ) {
      this.apply_gesture_options(handle_scroll, handle_scale, kinetic_scroll, tracking_mode);
    }
    if (wheel_behavior !== undefined) this.gestures_cfg.wheel_behavior = wheel_behavior;
    this.sync_touch_action();
    if (localization !== undefined) this.apply_localization(localization);
    if (accessibility !== undefined) {
      if (accessibility === false) this.accessibility_handle?.detach();
      else if (this.accessibility_handle !== null) {
        this.accessibility_handle.apply_options(accessibility === true ? {} : accessibility);
      } else {
        enable_accessibility(this, accessibility === true ? {} : accessibility);
      }
    }
    // autoSize stays in `rest` (the engine stores it); the active flag is tracked TS-side.
    if (options.autoSize !== undefined) this.set_auto_size(options.autoSize);
    // Gesture-only patches strip down to an empty object; an empty patch is a no-op for the
    // engine, so don't forward it (it would still trigger the full relayout that real patches
    // get — reference applyOptions(fullUpdate) semantics — with nothing to apply).
    if (Object.keys(engine_options).some((key) => {
      const value = (engine_options as Record<string, unknown>)[key];
      return value !== undefined && (typeof value !== "object" || value === null || Object.keys(value).length > 0);
    })) {
      this.wasm.apply_options(JSON.stringify(engine_options));
    }
    this.repaint();
    for (const h of this.options_change_subs) h(options);
  }

  /** reference `autoSize`: the engine owns container sizing only while this option is active. */
  private set_auto_size(on: boolean): void {
    if (on === this.auto_size || this.removed) return;
    if (on) {
      this.wasm.enable_auto_resize(this.container);
      this.auto_size = true;
      this.bind_dpr_watcher();
    } else {
      this.wasm.disable_auto_resize();
      this.auto_size = false;
      this.unbind_dpr_watcher();
    }
  }

  nudge_selected_drawing(dx: number, dy: number, handle: number | null): boolean {
    const changed = this.wasm.nudge_selected_drawing(dx, dy, handle ?? -1);
    if (changed) this.repaint();
    return changed;
  }

  /** Keyboard-reachable handle count of a drawing (0 when unknown or not placeable). */
  drawing_handle_count(id: number): number {
    return Math.max(0, this.wasm.drawing_handle_count(id));
  }

  select_drawing_for_accessibility(id: number): void {
    this.wasm.set_selected_drawing(id);
    this.repaint();
  }

  /** DPR-only display transitions do not reliably resize CSS bounds on every WebKit host. */
  private bind_dpr_watcher(): void {
    this.unbind_dpr_watcher();
    this.dpr_query = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    this.dpr_query.addEventListener("change", this.dpr_change_handler, { once: true });
    window.addEventListener("orientationchange", this.dpr_change_handler);
    document.addEventListener("fullscreenchange", this.dpr_change_handler);
  }

  private unbind_dpr_watcher(): void {
    this.dpr_query?.removeEventListener("change", this.dpr_change_handler);
    this.dpr_query = null;
    window.removeEventListener("orientationchange", this.dpr_change_handler);
    document.removeEventListener("fullscreenchange", this.dpr_change_handler);
  }

  auto_size_active(): boolean {
    return this.auto_size;
  }

  chart_element(): HTMLElement {
    return this.container;
  }

  /** Install the host price/time formatters (reference `localization`). Callbacks cross into wasm. */
  apply_localization(loc: localization_options): void {
    if (loc.price_formatter !== undefined) this.wasm.set_price_formatter(loc.price_formatter);
    if (loc.time_formatter !== undefined) {
      const formatter = loc.time_formatter as localization_options["time_formatter"] | null;
      this.host_time_formatter = formatter ?? undefined;
      // The engine calls this during frame construction, so the wrapper reads only package
      // state (never the wasm object) to build the calendar-date context.
      this.wasm.set_time_formatter(formatter === null || formatter === undefined
        ? null
        : (time: number) => formatter(time, this.time_label_context(time)));
    }
    if (loc.locale !== undefined) {
      this.locale_setting = loc.locale;
      this.wasm.set_locale(loc.locale);
    }
    if (loc.date_format !== undefined) this.wasm.set_date_format(loc.date_format);
    this.repaint();
  }

  /** Resolve and store the gesture toggles; the recognizer reads the result live. */
  apply_gesture_options(
    scroll?: boolean | handle_scroll_options,
    scale?: boolean | handle_scale_options,
    kinetic?: boolean | kinetic_scroll_options,
    tracking?: tracking_mode_options,
  ): void {
    if (scroll !== undefined) apply_scroll(scroll, this.gestures_cfg);
    if (scale !== undefined) apply_scale(scale, this.gestures_cfg);
    if (kinetic !== undefined) apply_kinetic(kinetic, this.gestures_cfg);
    if (tracking !== undefined) apply_tracking(tracking, this.gestures_cfg);
    this.sync_interaction_disabled();
    this.sync_touch_action();
  }

  private sync_touch_action(): void {
    // Cancellable Touch Events arbitrate direction after the shared 5 px slop.
    this.overlay.style.touchAction = "auto";
  }

  /** Resolve the reference `layout.panes.enableResize` toggle (separator drag + hover cursor). */
  apply_panes_resize(enabled: boolean): void {
    this.gestures_cfg.panes_resize = enabled;
  }

  /** Mirror the engine's master interaction switch: off only when every scroll+scale flag is. */
  private sync_interaction_disabled(): void {
    const c = this.gestures_cfg;
    const all_off =
      !c.pan && !c.pan_horz_touch && !c.pan_vert_touch && !c.wheel_scroll && !c.wheel_zoom &&
      !c.pinch_zoom && !c.axis_dblclick_reset_time && !c.axis_dblclick_reset_price &&
      !c.axis_scale_time && !c.axis_scale_price;
    this.wasm.set_interaction_disabled(all_off);
  }

  /** Update the aria-live region with a compact description of the point under the cursor. */
  announce(x: number, y: number): void {
    const params = this.build_params(x, y);
    if (params.time === null || params.series_data.size === 0) return;
    const parts = [];
    for (const [, point] of params.series_data) {
      parts.push("value" in point ? `${point.value}` : `O ${point.open} H ${point.high} L ${point.low} C ${point.close}`);
    }
    this.accessibility_handle?.announce(`Time ${params.time}: ${parts.join("; ")}`);
  }

  /** Whether the user has requested reduced motion (gates kinetic scroll). */
  prefers_reduced_motion(): boolean {
    return this.container.ownerDocument.defaultView?.matchMedia?.("(prefers-reduced-motion: reduce)")
      .matches === true;
  }

  /** Current resolved gesture toggles (read by the gesture recognizer). */
  gesture_config(): resolved_gestures {
    return this.gestures_cfg;
  }

  /**
   * Keyboard time-scale motion honors the same host switches as pointer gestures, so a fixed
   * view (`handle_scroll: false`, `handle_scale: false`) cannot be moved from the keyboard:
   * arrow panning needs a horizontal scroll gesture, +/- zoom follows wheel zoom, and Home
   * (fit/reset) follows the time-axis reset gesture.
   */
  keyboard_time_scroll_enabled(): boolean {
    const c = this.gestures_cfg;
    return c.pan || c.pan_horz_touch || c.wheel_scroll;
  }
  keyboard_time_zoom_enabled(): boolean {
    return this.gestures_cfg.wheel_zoom;
  }
  keyboard_time_reset_enabled(): boolean {
    return this.gestures_cfg.axis_dblclick_reset_time;
  }

  /** Whether `remove()` ran; host frame loops stop instead of touching the freed engine. */
  is_removed(): boolean {
    return this.removed;
  }

  options(): unknown {
    return JSON.parse(this.wasm.options_json());
  }

  backend(): "webgpu" | "canvas2d" {
    return this.wasm.backend_kind() as "webgpu" | "canvas2d";
  }

  backend_status(): Readonly<backend_status> {
    return Object.freeze(JSON.parse(this.wasm.backend_status_json()) as backend_status);
  }

  /**
   * Read the engine's last-frame record into the per-chart scratch array and shape it as
   * `frame_stats`. The scratch array is the whole point: the engine writes f64 slots in place,
   * so a host polling this every frame for minutes never grows the JS heap and never triggers a
   * GC pause that would itself corrupt the measurement.
   */
  frame_stats(): frame_stats {
    const out = this.stats_scratch;
    this.wasm.frame_stats_into(out);
    const gpu = out[FRAME_STATS_SLOT.gpu_ms] as number;
    return {
      cpu_ms: out[FRAME_STATS_SLOT.cpu_ms] as number,
      // NaN is the engine's encoding for "unavailable" (no WebGPU / no timestamp-query / no
      // readback resolved yet) — see the `frame_stats.gpu_ms` docs.
      gpu_ms: Number.isNaN(gpu) ? null : gpu,
      draw_calls: out[FRAME_STATS_SLOT.draw_calls] as number,
      dropped_frames: out[FRAME_STATS_SLOT.dropped_frames] as number,
      presented_frames: out[FRAME_STATS_SLOT.presented_frames] as number,
      memory_bytes: out[FRAME_STATS_SLOT.memory_bytes] as number,
      canvas2d_ops: out[FRAME_STATS_SLOT.canvas2d_ops] as number,
      ring_overruns: out[FRAME_STATS_SLOT.ring_overruns] as number,
      gpu_buffer_allocations: out[FRAME_STATS_SLOT.gpu_buffer_allocations] as number,
      gpu_write_calls: out[FRAME_STATS_SLOT.gpu_write_calls] as number,
      gpu_uploaded_bytes: out[FRAME_STATS_SLOT.gpu_uploaded_bytes] as number,
      layout_rebuilds: out[FRAME_STATS_SLOT.layout_rebuilds] as number,
      autoscale_runs: out[FRAME_STATS_SLOT.autoscale_runs] as number,
      series_rebuilds: out[FRAME_STATS_SLOT.series_rebuilds] as number,
      drawing_rebuilds: out[FRAME_STATS_SLOT.drawing_rebuilds] as number,
      grid_rebuilds: out[FRAME_STATS_SLOT.grid_rebuilds] as number,
      overlay_rebuilds: out[FRAME_STATS_SLOT.overlay_rebuilds] as number,
      axis_rebuilds: out[FRAME_STATS_SLOT.axis_rebuilds] as number,
      text_resolutions: out[FRAME_STATS_SLOT.text_resolutions] as number,
      trading_rebuilds: out[FRAME_STATS_SLOT.trading_rebuilds] as number,
      ring_dropped_rows: out[FRAME_STATS_SLOT.ring_dropped_rows] as number,
    };
  }

  time_scale(): time_scale_api {
    return this.ts;
  }

  price_scale(price_scale_id = "right", pane_index = 0): price_scale_api {
    return new price_scale_impl(this, pane_index, price_scale_id);
  }

  add_price_scale(options: price_scale_create_options, pane_index = 0): price_scale_api {
    // Same contract as `price_scale().apply_options`: a malformed extension value throws before
    // any scale is created.
    validate_price_scale_options(options);
    parse_engine_result<{ target: number }>(
      this.wasm.add_price_scale_result_json(pane_index, JSON.stringify(options)),
    );
    this.repaint();
    return new price_scale_impl(this, pane_index, options.id);
  }

  price_scales(pane_index = 0): price_scale_info[] {
    return JSON.parse(this.wasm.price_scales_json(pane_index)) as price_scale_info[];
  }

  move_price_scale(id: string, side: "left" | "right", order: number, pane_index = 0): void {
    const target = undef_to_null(this.wasm.price_scale_target_by_id(pane_index, id));
    if (target === null) {
      throw new AerisChartsError("invalid_handle", `price scale '${id}' does not exist in pane ${pane_index}`);
    }
    parse_engine_result<object>(
      this.wasm.move_price_scale_result_json(pane_index, target, side, order),
    );
    this.repaint();
  }

  remove_price_scale(id: string, pane_index = 0): void {
    const target = undef_to_null(this.wasm.price_scale_target_by_id(pane_index, id));
    if (target === null) {
      throw new AerisChartsError("invalid_handle", `price scale '${id}' does not exist in pane ${pane_index}`);
    }
    parse_engine_result<object>(this.wasm.remove_price_scale_result_json(pane_index, target));
    this.repaint();
  }

  panes(): pane_api[] {
    const n = this.wasm.pane_count();
    const out: pane_api[] = [];
    for (let i = 0; i < n; i++) {
      out.push(new pane_impl(this, i));
    }
    return out;
  }

  add_pane(options?: boolean | general_pane_options): pane_api {
    if (typeof options === "object") {
      const { pane } = parse_general_result<{ pane: number }>(
        this.wasm.add_general_pane_result_json(JSON.stringify(options)),
      );
      this.repaint();
      return new pane_impl(this, pane);
    }
    const index = undef_to_null(this.wasm.add_pane(options ?? false));
    if (index === null) throw new AerisChartsError("resource_limit", "pane identity space is exhausted");
    this.repaint();
    return new pane_impl(this, index);
  }

  add_axis(options: general_axis_options): general_axis_api {
    const normalized = normalize_general_axis_options(options);
    const created = parse_general_result<{ id: string; handle_token: number }>(
      this.wasm.add_general_axis_result_json(JSON.stringify(normalized)),
    );
    this.repaint();
    return new general_axis_impl(created.id, created.handle_token, this);
  }

  axis(id: string): general_axis_api | null {
    const handle_token = this.wasm.general_axis_handle_token(id);
    return handle_token === 0 ? null : new general_axis_impl(id, handle_token, this);
  }

  axes(pane?: number): general_axis_api[] {
    const ids = JSON.parse(this.wasm.general_axis_ids_json(pane ?? -1)) as string[];
    return ids.map((id) => new general_axis_impl(id, this.wasm.general_axis_handle_token(id), this));
  }

  remove_axis(id: string): boolean {
    const removed = this.wasm.remove_general_axis(id);
    if (removed) this.repaint();
    return removed;
  }

  add_general_reference(options: general_reference_options): general_reference_api {
    const encoded = encode_general_reference_options(this, options);
    const created = parse_general_result<{ id: number }>(
      this.wasm.add_general_reference_result_json(JSON.stringify(encoded)),
    );
    this.repaint();
    return new general_reference_impl(created.id, this);
  }

  general_references(pane?: number): general_reference_api[] {
    if (pane !== undefined && (!Number.isSafeInteger(pane) || pane < 0)) {
      throw new AerisChartsError(
        "invalid_options",
        "general reference pane must be a non-negative safe integer",
      );
    }
    const ids = JSON.parse(this.wasm.general_reference_ids_json(pane ?? -1)) as number[];
    return ids.map((id) => new general_reference_impl(id, this));
  }

  general_legend_snapshot(pane?: number): general_legend_snapshot {
    if (pane !== undefined && (!Number.isSafeInteger(pane) || pane < 0)) {
      throw new AerisChartsError(
        "invalid_options",
        "general legend pane must be a non-negative safe integer",
      );
    }
    return JSON.parse(this.wasm.general_legend_snapshot_json(pane ?? -1)) as general_legend_snapshot;
  }

  general_shared_tooltip(
    series: number | general_series_api,
    row: number,
  ): general_shared_tooltip_snapshot | null {
    if (!Number.isSafeInteger(row) || row < 0) {
      throw new AerisChartsError(
        "invalid_options",
        "general shared-tooltip row must be a non-negative safe integer",
      );
    }
    const series_id = typeof series === "number" ? series : series.id;
    if (!Number.isSafeInteger(series_id) || series_id <= 0) {
      throw new AerisChartsError(
        "invalid_options",
        "general shared-tooltip series must have a positive safe integer id",
      );
    }
    return JSON.parse(
      this.wasm.general_shared_tooltip_json(series_id, row),
    ) as general_shared_tooltip_snapshot | null;
  }

  set_general_brush(
    axis: string | general_axis_api,
    from_coordinate: number,
    to_coordinate: number,
  ): general_brush_snapshot {
    if (!Number.isFinite(from_coordinate) || !Number.isFinite(to_coordinate)) {
      throw new AerisChartsError(
        "invalid_options",
        "general brush coordinates must be finite CSS-pixel values",
      );
    }
    const axis_id = typeof axis === "string" ? axis : axis.id;
    if (axis_id.length === 0) {
      throw new AerisChartsError("invalid_options", "general brush axis id must not be empty");
    }
    const snapshot = parse_general_result<general_brush_snapshot>(
      this.wasm.set_general_brush_result_json(axis_id, from_coordinate, to_coordinate),
    );
    this.repaint();
    return snapshot;
  }

  general_brush_snapshot(): general_brush_snapshot | null {
    return JSON.parse(this.wasm.general_brush_snapshot_json()) as general_brush_snapshot | null;
  }

  clear_general_brush(): void {
    this.wasm.clear_general_brush();
    this.repaint();
  }

  general_hit_test(
    pane: number,
    x: number,
    y: number,
    max_distance?: number,
  ): general_series_hit | null {
    if (max_distance !== undefined && (!Number.isFinite(max_distance) || max_distance < 0)) {
      throw new AerisChartsError(
        "invalid_options",
        "general hit-test max_distance must be a finite non-negative number",
      );
    }
    return JSON.parse(
      this.wasm.general_hit_test_json(pane, x, y, max_distance ?? -1),
    ) as general_series_hit | null;
  }

  general_selected_hit(): general_series_hit | null {
    return JSON.parse(this.wasm.general_selected_hit_json()) as general_series_hit | null;
  }

  general_accessibility_focused_hit(): general_series_hit | null {
    return JSON.parse(this.wasm.general_accessibility_focused_hit_json()) as general_series_hit | null;
  }

  set_general_accessibility_focus(series: number, row: number): boolean {
    const accepted = this.wasm.set_general_accessibility_focus(series, row);
    if (accepted) this.repaint();
    return accepted;
  }

  clear_general_accessibility_focus(): void {
    this.wasm.clear_general_accessibility_focus();
    this.repaint();
  }

  remove_pane(index: number): boolean {
    if (!this.wasm.remove_pane(index)) return false;
    this.repaint();
    return true;
  }

  swap_panes(first: number, second: number): boolean {
    if (!this.wasm.swap_panes(first, second)) return false;
    this.repaint();
    return true;
  }

  reset_style_to_defaults(): void {
    this.wasm.reset_style_to_defaults(this.selected_theme === "light");
    this.sync_countdown_timer();
    this.repaint();
  }

  reset_view(): void {
    this.wasm.reset_view();
    this.repaint();
  }

  price_to_coordinate(price: number): number | null {
    return undef_to_null(this.wasm.price_to_coordinate(price));
  }
  coordinate_to_price(y: number): number | null {
    return undef_to_null(this.wasm.coordinate_to_price(y));
  }

  set_crosshair_position(price: number, time: time, series: series_api): void {
    const seconds = time_to_utc_seconds(time);
    // false = the engine refused the position (e.g. unknown series); reference throws, we no-op.
    if (!this.wasm.set_crosshair_position(price, seconds, series.id)) return;
    this.repaint();
    // Emit the crosshair-move at the coordinates the position resolved to, on the given series'
    // own price scale.
    const x = undef_to_null(this.wasm.time_to_coordinate(seconds));
    const y = undef_to_null(this.wasm.series_price_to_coordinate(series.id, price));
    if (x === null || y === null) return;
    this.emit_crosshair(x, y);
  }

  clear_crosshair_position(): void {
    this.wasm.clear_crosshair_position();
    this.repaint();
    this.emit_crosshair_left();
  }

  crosshair_sync_position(): crosshair_sync_position | null {
    return JSON.parse(this.wasm.crosshair_sync_json()) as crosshair_sync_position | null;
  }

  apply_external_crosshair(position: crosshair_sync_position | null): void {
    if (this.wasm.apply_external_crosshair_json(JSON.stringify(position))) this.repaint();
  }

  take_sync_events(): chart_sync_event[] {
    return JSON.parse(this.wasm.take_sync_events_json()) as chart_sync_event[];
  }

  resize(width: number, height: number, dpr?: number): void {
    if (this.auto_size) return;
    this.apply_size(width, height, dpr ?? window.devicePixelRatio ?? 1);
  }

  private apply_size(width: number, height: number, dpr: number): void {
    this.pixel_ratio = Math.max(dpr, Number.EPSILON);
    const bitmap_width = Math.max(1, Math.round(width * this.pixel_ratio));
    const bitmap_height = Math.max(1, Math.round(height * this.pixel_ratio));
    for (const canvas of [this.gpu_pane, this.fallback_pane, this.plugin_canvas, this.overlay]) {
      canvas.width = bitmap_width;
      canvas.height = bitmap_height;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
    }
    this.wasm.resize(width, height, this.pixel_ratio);
    this.repaint();
  }

  render(): void {
    this.repaint();
  }

  take_screenshot(add_top_layer = true, include_crosshair = true): HTMLCanvasElement {
    // Browser WebGPU canvases are presentable but are not synchronously readable through
    // CanvasRenderingContext2D.drawImage (Chromium returns transparent pixels). Repaint the current
    // engine frame, then execute that same retained frame through the already-warm Canvas2D pane.
    // This keeps the reference-style synchronous API deterministic without duplicating chart state.
    this.repaint();
    // Hiding the crosshair for the capture is a clear → snapshot → restore cycle; the whole call
    // is synchronous, so the on-screen canvases never present the crosshair-less frame.
    const restore = include_crosshair ? null : this.last_crosshair;
    if (restore !== null) {
      this.wasm.clear_crosshair();
      this.wasm.render();
    }
    this.wasm.render_canvas2d_snapshot(add_top_layer);
    const output = document.createElement("canvas");
    output.width = this.overlay.width;
    output.height = this.overlay.height;
    const ctx = output.getContext("2d");
    if (ctx === null) {
      throw new AerisChartsError("renderer_platform_error", "screenshot Canvas2D context is unavailable");
    }
    ctx.drawImage(this.fallback_pane, 0, 0);
    // Canvas primitives composite at pane level (the reference paints primitives on the pane
    // canvas), so they are captured regardless of `add_top_layer`. The repaint above re-ran
    // the pass, so the plugin canvas is fresh.
    ctx.drawImage(this.plugin_canvas, 0, 0);
    if (add_top_layer) {
      ctx.drawImage(this.overlay, 0, 0);
    }
    // `render_canvas2d_snapshot(false)` executes into the warm fallback surface. When Canvas2D is
    // the live backend that surface is visible, so restore its full shared chrome after copying the
    // requested pane-only bitmap. (On WebGPU this simply keeps the warm fallback current.)
    if (!add_top_layer) {
      this.wasm.render_canvas2d_snapshot(true);
    }
    if (restore !== null) {
      this.wasm.set_crosshair(restore.x, restore.y);
      this.repaint();
    }
    return output;
  }

  overlay_el(): HTMLCanvasElement {
    return this.overlay;
  }

  remove(): void {
    if (this.removed) return;
    const wasm = this.wasm;
    // Package-owned canvas extensions live outside wasm; detach each independently so one
    // throwing hook cannot prevent the rest of the chart from being released.
    for (const entry of this.canvas_primitives.splice(0)) {
      if (entry.detached) continue;
      entry.detached = true;
      try {
        entry.primitive.detached?.();
      } catch (error) {
        console.warn(`aeris_charts: canvas primitive \`detached\` threw — ${error}`);
      }
    }
    this.removed = true;
    if (this.repaint_raf !== null) {
      cancelAnimationFrame(this.repaint_raf);
      this.repaint_raf = null;
    }
    // Stop the ring-drain loop and drop its report buffer; the engine's views over any bound
    // shared buffer go with the chart itself.
    if (this.ring_raf !== null) {
      cancelAnimationFrame(this.ring_raf);
      this.ring_raf = null;
    }
    this.ring_report = null;
    this.close_text_editor(false);
    this.stop_animation();
    this.stop_countdown_timer();
    window.removeEventListener("aeris_charts-chart-backend-lost", this.backend_loss_handler);
    this.unbind_dpr_watcher();
    this.detach_gestures?.();
    this.observer?.disconnect();
    this.plugin_resize_observer?.disconnect();
    this.observer = null;
    this.detach_gestures = null;
    this.plugin_resize_observer = null;
    for (const series of this.series_by_id.values()) series.mark_removed();
    this.series_by_id.clear();
    for (const series of this.general_series_by_id.values()) series.mark_removed();
    this.general_series_by_id.clear();
    this.crosshair_subs.clear();
    this.click_subs.clear();
    this.chart_context_subs.clear();
    this.dbl_click_subs.clear();
    this.visible_logical_range_subs.clear();
    this.visible_time_range_subs.clear();
    this.size_change_subs.clear();
    this.series_added_subs.clear();
    this.series_removed_subs.clear();
    this.options_change_subs.clear();
    this.delta_tooltip_range_listeners.clear();
    this.tool_listener = null;
    this.tool_change_subs.clear();
    this.drawing_created_subs.clear();
    this.accessibility_handle?.detach();
    this.accessibility_handle = null;
    wasm.dispose();
    wasm.free();
    this.wasm_instance = null;
    this.gpu_pane.remove();
    this.fallback_pane.remove();
    this.plugin_canvas.remove();
    this.overlay.remove();
  }
}
