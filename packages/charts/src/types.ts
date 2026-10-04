/**
 * Public data, option, and handle types for `@aeristerminal/aeris-charts`. The original snake-case
 * methods remain canonical and supported; common browser lifecycle methods also expose camel-case
 * aliases on the same handles. Extracted from `index.ts`.
 */

import type { pane_primitive, pane_primitive_handle, series_primitive, series_primitive_handle } from "./primitives.js";
import type { canvas_primitive, canvas_primitive_handle } from "./canvas_plugins.js";
import type { custom_series_pane_view } from "./custom_series.js";
import type { accessibility_handle, accessibility_options } from "./accessibility.js";

// ---------------------------------------------------------------------------------------------
// Data & option types
// ---------------------------------------------------------------------------------------------

/**
 * A series kind. Maps to the engine's numeric kind at the boundary. `"custom"` is only ever
 * REPORTED (by {@link series_api.series_type} for a custom series); it is not accepted by
 * {@link chart_api.add_series} — custom series are created with {@link chart_api.add_custom_series}.
 */
export type feature_series_kind =
  | "grouped_bars"
  | "heatmap"
  | "hlc_area"
  | "pretty_histogram"
  | "background_shade"
  | "stacked_area"
  | "stacked_bars"
  | "whisker_box";

export type series_kind =
  | "candlestick"
  | "bar"
  | "line"
  | "area"
  | "histogram"
  | "baseline"
  | "footprint"
  /** @deprecated Use an ordinary `"area"` series with `enable_brushable_area_interaction()`. */
  | "brushable_area"
  | feature_series_kind
  | "custom";

/** General Cartesian series currently available through the shared chart engine. */
export type general_series_kind = "xy_line" | "xy_area" | "range_area" | "range_bar" | "error_bar" | "column" | "horizontal_bar" | "box_plot" | "heatmap_grid" | "scatter" | "bubble";
export type general_row_id = string | number;

export interface general_xy_row {
  id?: general_row_id;
  x: string | number | Date;
  y: number | null;
  /** Optional custom text for an enabled data label; omitted labels use the numeric Y value. */
  label?: string;
}

export interface bubble_row extends general_xy_row {
  /** Bubble area channel. Missing values remain queryable but emit no mark. */
  size: number | null;
}

export interface range_area_row {
  id?: general_row_id;
  x: string | number | Date;
  low: number | null;
  high: number | null;
  /** Optional custom text for an enabled data label; omitted labels use the high value. */
  label?: string;
}

/** Numeric/temporal/category observation with error bounds supported by the bound X domain. */
export interface error_bar_row extends general_xy_row {
  x: number | string | Date;
  x_low?: number | Date | null;
  x_high?: number | Date | null;
  y_low?: number | null;
  y_high?: number | null;
}

export interface box_plot_row {
  id?: general_row_id;
  x: string;
  min: number | null;
  q1: number | null;
  median: number | null;
  q3: number | null;
  max: number | null;
  label?: string;
}

export interface heatmap_grid_row {
  id?: general_row_id;
  x: string | number | Date;
  y: string | number;
  value: number | null;
  label?: string;
}

export interface numeric_xy_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  x: Float64Array;
  y: Float64Array;
  y_valid?: Uint8Array;
}

export interface bubble_columns extends numeric_xy_columns {
  size: Float64Array;
  size_valid?: Uint8Array;
}

export interface numeric_range_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  x: Float64Array;
  low: Float64Array;
  low_valid?: Uint8Array;
  high: Float64Array;
  high_valid?: Uint8Array;
}

/** Each bound has its own missing-value mask; a missing center Y emits no mark. */
export interface numeric_error_columns extends numeric_xy_columns {
  x_low: Float64Array;
  x_low_valid?: Uint8Array;
  x_high: Float64Array;
  x_high_valid?: Uint8Array;
  y_low: Float64Array;
  y_low_valid?: Uint8Array;
  y_high: Float64Array;
  y_high_valid?: Uint8Array;
}

export interface temporal_xy_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  /** Whole epoch-millisecond values carried as JS-safe numbers. */
  x_epoch_ms: Float64Array;
  y: Float64Array;
  y_valid?: Uint8Array;
}

export interface temporal_range_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  /** Whole epoch-millisecond values carried as JS-safe numbers. */
  x_epoch_ms: Float64Array;
  low: Float64Array;
  low_valid?: Uint8Array;
  high: Float64Array;
  high_valid?: Uint8Array;
}

/** Temporal error bounds are whole epoch-millisecond values carried as JS-safe numbers. */
export interface temporal_error_columns extends temporal_xy_columns {
  x_low_epoch_ms: Float64Array;
  x_low_valid?: Uint8Array;
  x_high_epoch_ms: Float64Array;
  x_high_valid?: Uint8Array;
  y_low: Float64Array;
  y_low_valid?: Uint8Array;
  y_high: Float64Array;
  y_high_valid?: Uint8Array;
}

export interface category_xy_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  categories: readonly string[];
  category_indices: Uint32Array;
  y: Float64Array;
  y_valid?: Uint8Array;
}

export interface category_range_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  categories: readonly string[];
  category_indices: Uint32Array;
  low: Float64Array;
  low_valid?: Uint8Array;
  high: Float64Array;
  high_valid?: Uint8Array;
}

/** Category-centered error bars have Y bounds only; X uncertainty requires numeric X. */
export interface category_error_columns extends category_xy_columns {
  y_low: Float64Array;
  y_low_valid?: Uint8Array;
  y_high: Float64Array;
  y_high_valid?: Uint8Array;
}

export interface category_box_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  categories: readonly string[];
  category_indices: Uint32Array;
  min: Float64Array;
  min_valid?: Uint8Array;
  q1: Float64Array;
  q1_valid?: Uint8Array;
  median: Float64Array;
  median_valid?: Uint8Array;
  q3: Float64Array;
  q3_valid?: Uint8Array;
  max: Float64Array;
  max_valid?: Uint8Array;
}

export interface category_heatmap_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  x_categories: readonly string[];
  x_category_indices: Uint32Array;
  y_categories: readonly string[];
  y_category_indices: Uint32Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}

export interface numeric_heatmap_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  x: Float64Array;
  y_coordinate: Float64Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}

export interface temporal_heatmap_columns {
  ids?: readonly general_row_id[];
  labels?: readonly (string | null)[];
  x_epoch_ms: Float64Array;
  y_coordinate: Float64Array;
  value: Float64Array;
  value_valid?: Uint8Array;
}

export type horizontal_domain_options =
  | { type: "financial_time" }
  | { type: "continuous"; scale?: "linear" | "log" | "symlog" }
  | { type: "temporal" }
  | { type: "category"; scale?: "band" | "point" }
  | { type: "polar" };

export interface general_pane_options {
  preserve_empty?: boolean;
  horizontal_domain: horizontal_domain_options;
}

/** Creation-time topology for the chart's canonical first pane. */
export interface initial_pane_options {
  horizontal_domain: horizontal_domain_options;
}

export type axis_dimension = "x" | "y" | "angle" | "radius";
export type axis_position = "top" | "bottom" | "left" | "right";
export type general_scale_type =
  | "linear"
  | "log"
  | "symlog"
  | "temporal"
  | "band"
  | "point"
  | "radial_linear"
  | "angular_category";

export type general_axis_tick =
  | { type: "numeric"; value: number; label?: string }
  | { type: "temporal"; value: number | Date; label?: string }
  | { type: "category"; value: string; label?: string };

export interface general_axis_options {
  id: string;
  pane: number;
  dimension: axis_dimension;
  position?: axis_position;
  scale: general_scale_type;
  domain?: "auto" | readonly [number | Date, number | Date] | readonly string[];
  reverse?: boolean;
  visible?: boolean;
  title?: string;
  tick_count?: number;
  /** Explicit tick values. Optional labels are retained by the engine and shared by every backend. */
  ticks?: readonly general_axis_tick[];
  min_tick_gap?: number;
  band_padding_inner?: number;
  band_padding_outer?: number;
  /** Draw a solid zero rule when zero is inside a numeric domain. */
  zero_line?: boolean;
  /** Draw this axis's ticks as plot grid rules, subject to the chart-wide grid direction. */
  grid_visible?: boolean;
}

/** Options that can change without replacing an axis or its scale/domain family. */
export type general_axis_presentation_options = Omit<
  general_axis_options,
  "id" | "pane" | "dimension" | "scale"
>;

export type general_reference_value = number | string | Date;

export type general_reference_options =
  | {
      kind: "line";
      pane: number;
      axis_id: string;
      value: general_reference_value;
      color?: string;
      line_width?: number;
      /** Include the reference value in an automatic axis domain. Default false. */
      extend_domain?: boolean;
    }
  | {
      kind: "dot";
      pane: number;
      x_axis_id: string;
      y_axis_id: string;
      x: general_reference_value;
      y: general_reference_value;
      color?: string;
      radius?: number;
      /** Include both coordinates in automatic axis domains. Default false. */
      extend_domain?: boolean;
    }
  | {
      kind: "region";
      pane: number;
      x_axis_id: string;
      y_axis_id: string;
      x_from: general_reference_value;
      x_to: general_reference_value;
      y_from: general_reference_value;
      y_to: general_reference_value;
      fill_color?: string;
      /** Include all region bounds in automatic axis domains. Default false. */
      extend_domain?: boolean;
    };

export interface general_reference_api {
  readonly id: number;
  options(): general_reference_options;
  remove(): boolean;
}

export interface general_series_options {
  pane: number;
  x_axis_id: string;
  y_axis_id: string;
  visible?: boolean;
  title?: string;
  color?: string;
  /** Path-marker/scatter radius or error-bar cap half-width in CSS pixels. Bubble radii come from sqrt(size). */
  point_radius?: number;
  /** Draw markers at line, area, or range-area data points (default false). */
  point_markers?: boolean;
  /** Marker shape for scatter and opt-in path markers (default `circle`). */
  point_symbol?: "circle" | "square" | "diamond" | "triangle";
  /** Stroke width for line, area, and range-area paths in CSS pixels (default 2). */
  line_width?: number;
  /** Stroke pattern for line, area, and range-area paths (default `solid`). */
  line_style?: "solid" | "dotted" | "dashed";
  /** Path interpolation for line, area, and range-area boundaries (default `linear`). */
  interpolation?: "linear" | "step" | "curved";
  /** Bridge missing rows in line, area, and range-area paths; transform-invalid rows remain gaps. */
  connect_missing?: boolean;
  /** Area fill opacity from 0 through 1 (default `72 / 255`). */
  fill_opacity?: number;
  /** Explicit numeric fill baseline for `xy_area`; omitted uses zero when visible, otherwise the edge. */
  baseline_value?: number;
  /** Show bounded, engine-placed value labels beside visible marks. */
  data_labels?: boolean;
  /** Bar-only grouping key. Matching columns or horizontal bars share their category band side-by-side. */
  group_id?: string;
  /** Column/horizontal_bar/xy_area stack key. Bars accumulate by category; areas by exact X identity. */
  stack_id?: string;
  /** Bar/xy_area stack normalization. `percent` requires `stack_id`. */
  stack_mode?: "normal" | "percent";
}

/** Presentation-only subset retained for callers that do not need compatible axis rebinding. */
export type general_series_presentation_options = Omit<
  general_series_options,
  "pane" | "x_axis_id" | "y_axis_id"
>;

export interface general_update_options {
  /** Keep only the newest rows after this transaction. Omit to retain the full dataset. */
  max_rows?: number;
}

export interface general_tooltip_snapshot {
  series: number;
  row: number;
  row_id: general_row_id | { generated: string };
  x_label: string;
  /** Heatmap Y category label, or null for non-heatmap series. */
  y_label: string | null;
  /** Custom row label, or null when the numeric value supplies the visible label. */
  label: string | null;
  value: number | null;
  /** Lower range bound, or null for a missing bound/non-range series. */
  low: number | null;
  /** Upper range bound, or null for a missing bound/non-range series. */
  high: number | null;
  /** Numeric X error bounds, or null when missing/not an error-bar series. */
  x_low: number | null;
  x_high: number | null;
  /** Box-plot quartiles, or null for missing quartiles/non-box series. */
  q1: number | null;
  q3: number | null;
  /** Bubble size channel, or null for missing size/non-bubble series. */
  size: number | null;
  title: string;
}

export interface general_shared_tooltip_snapshot {
  /** Pane containing the anchor and every returned visible item. */
  pane: number;
  anchor_series: number;
  anchor_row: number;
  /**
   * Visible rows whose exact engine-owned horizontal datum matches the anchor, in stable
   * series order then row order. A heatmap may contribute multiple cells for one X datum.
   */
  items: readonly general_tooltip_snapshot[];
}

export type general_brush_range =
  | { type: "numeric"; from: number; to: number }
  | { type: "temporal"; from: number; to: number }
  | { type: "category"; from: string; to: string };

export interface general_brush_snapshot {
  pane: number;
  axis_id: string;
  dimension: "x" | "y";
  range: general_brush_range;
  /** Bounded stable series/row-order identities whose axis value is inside the selected range. */
  items: readonly general_series_hit[];
}

export interface general_series_hit {
  series: number;
  row: number;
  row_id: general_row_id | { generated: string };
  distance: number;
}

export interface general_accessibility_snapshot {
  series: number;
  title: string;
  total_rows: number;
  offset: number;
  items: readonly {
    row: number;
    row_id: general_row_id | { generated: string };
    x_label: string;
    y_label: string | null;
    label: string | null;
    value: number | null;
    low: number | null;
    high: number | null;
    x_low: number | null;
    x_high: number | null;
    q1: number | null;
    q3: number | null;
    size: number | null;
  }[];
}

export interface general_legend_item {
  series: number;
  pane: number;
  kind: general_series_kind;
  title: string;
  color: string | null;
  visible: boolean;
}

export interface general_legend_snapshot {
  items: readonly general_legend_item[];
}

export interface general_axis_api {
  applyOptions: general_axis_api["apply_options"];
  resetView: general_axis_api["reset_view"];
  setVisible: general_axis_api["set_visible"];
  readonly id: string;
  options(): general_axis_options;
  apply_options(options: Partial<general_axis_presentation_options>): void;
  set_visible(visible: boolean): void;
  /** Shift a numeric, temporal, or category view by a fraction of its current visible span. */
  pan(fraction: number): void;
  /** Zoom around a domain value; temporal anchors are epoch milliseconds and category anchors are identities. */
  zoom(factor: number, anchor_value: number | string): void;
  reset_view(): void;
  remove(): boolean;
}

export interface general_series_api {
  applyOptions: general_series_api["apply_options"];
  setVisible: general_series_api["set_visible"];
  readonly id: number;
  readonly kind: general_series_kind;
  options(): general_series_options;
  /** Mutate presentation and compatible pane/axis bindings while retaining identity and data. */
  apply_options(options: Partial<general_series_options>): void;
  set_visible(visible: boolean): void;
  set_data(data: readonly (general_xy_row | bubble_row | range_area_row | error_bar_row | box_plot_row | heatmap_grid_row)[]): void;
  set_data_typed(columns: numeric_xy_columns | temporal_xy_columns | category_xy_columns | bubble_columns | numeric_range_columns | temporal_range_columns | category_range_columns | numeric_error_columns | temporal_error_columns | category_error_columns | category_box_columns | category_heatmap_columns | numeric_heatmap_columns | temporal_heatmap_columns): void;
  /** Update existing rows and append missing rows by explicit `id`, atomically. */
  update_data(data: readonly (general_xy_row | bubble_row | range_area_row | error_bar_row | box_plot_row | heatmap_grid_row)[], options?: general_update_options): void;
  /** Typed-column form of {@link update_data}; `ids` is required at runtime. */
  update_data_typed(
    columns: numeric_xy_columns | temporal_xy_columns | category_xy_columns | bubble_columns | numeric_range_columns | temporal_range_columns | category_range_columns | numeric_error_columns | temporal_error_columns | category_error_columns | category_box_columns | category_heatmap_columns | numeric_heatmap_columns | temporal_heatmap_columns,
    options?: general_update_options,
  ): void;
  data_at(row: number): general_tooltip_snapshot | null;
  /** This series' currently selected mark, or `null` when another mark/series is selected. */
  selected_hit(): general_series_hit | null;
  accessibility_snapshot(offset?: number, limit?: number): general_accessibility_snapshot;
  remove(): void;
  /** Camel-case aliases; both naming styles operate on this same series handle. */
  setData: general_series_api["set_data"];
  setDataTyped: general_series_api["set_data_typed"];
  updateData: general_series_api["update_data"];
  updateDataTyped: general_series_api["update_data_typed"];
  dataAt: general_series_api["data_at"];
  selectedHit: general_series_api["selected_hit"];
  accessibilitySnapshot: general_series_api["accessibility_snapshot"];
}

/** Calendar day (reference `BusinessDay`), interpreted at UTC midnight. `month`/`day` are 1-based. */
export interface business_day {
  year: number;
  month: number;
  day: number;
}

/**
 * One entry of an explicit exchange time-zone schedule: from `from_utc_seconds` (inclusive) until
 * the next entry, exchange wall-clock time is `utc + offset_seconds`. The first offset also
 * applies before its entry. Entries must be strictly ascending (at most 1024, offsets within
 * ±18 h).
 */
export interface utc_offset_transition {
  from_utc_seconds: number;
  offset_seconds: number;
}

/**
 * Exchange time zone of the financial time axis: `"UTC"` (default), an IANA name such as
 * `"Asia/Shanghai"` or `"America/New_York"` (resolved once per zone with `Intl.DateTimeFormat`
 * over 1970–2100), or an explicit {@link utc_offset_transition} schedule. The chart never reads
 * the browser's own time zone.
 */
export type time_zone = string | readonly utc_offset_transition[];

/** Extra context passed to host time formatters. */
export interface time_label_context {
  /**
   * The calendar date when the chart's financial time points are calendar dates (every
   * financial series was given `business_day` or `"YYYY-MM-DD"` times), else `null` for
   * instants. The first argument remains the UTC-midnight seconds of that date.
   */
  business_day: business_day | null;
}

/**
 * A point in time (reference `Time`). Accepted forms at the input boundary:
 * - `number` — a finite whole UTC timestamp in seconds since the epoch;
 * - `business_day` — `{ year, month, day }`, taken at UTC midnight;
 * - `string` — `"YYYY-MM-DD"`, taken at UTC midnight.
 *
 * Numeric timestamps are never auto-converted and must be in the inclusive range
 * `-62167219200..253402300799` (years 0000..9999). Out-of-range values that look like milliseconds,
 * microseconds, or nanoseconds are rejected with a conversion hint. Business-day/string inputs are
 * strictly validated and converted at the boundary. Values the engine returns (e.g. `data()`,
 * crosshair params) are always the numeric UTC-seconds form.
 */
export type time = number | business_day | string;

/** OHLC bar for candlestick/bar series. */
export interface ohlc_data {
  time: time;
  open: number;
  high: number;
  low: number;
  close: number;
  /**
   * Optional body color for this bar (reference `BarData.color`/`CandlestickData.color`); when missed,
   * the color from the series options is used. snake_case per the package API convention.
   */
  color?: string;
  /**
   * Optional wick color for this bar (reference `CandlestickData.wickColor`); when missed, the color
   * from the series options is used. snake_case per the package API convention.
   */
  wick_color?: string;
  /**
   * Optional border color for this bar (reference `CandlestickData.borderColor`); when missed, the
   * color from the series options is used. snake_case per the package API convention.
   */
  border_color?: string;
}

/** Single-value point for line/area/histogram series. */
export interface single_value_data {
  time: time;
  value: number;
  /**
   * Optional color for this point (reference `LineData.color`/`HistogramData.color`); when missed, the
   * color from the series options is used.
   */
  color?: string;
}

/**
 * A whitespace point (reference `WhitespaceData`): reserves a time slot without carrying a value.
 * The engine keeps the row as an explicit empty slot instead of dropping it, so it can later be
 * replaced by a real bar via {@link series_api.update}.
 */
export interface whitespace_data {
  time: time;
}

/** @deprecated Brushable Area now uses ordinary scalar Area data (`single_value_data`). */
export interface brushable_area_data { time: time; value: number }
export interface grouped_bars_data { time: time; values: readonly number[] }
export interface heatmap_cell { low: number; high: number; amount: number }
export interface heatmap_data { time: time; cells: readonly heatmap_cell[] }
export type heatmap_cell_shader = (amount: number) => string;
export interface hlc_area_data { time: time; high: number; low: number; close: number }
export interface pretty_histogram_data { time: time; value: number; color?: string }
export interface background_shade_data { time: time; value: number }
/** Every non-whitespace row in one stacked-area series must carry the same number of layers. */
export interface stacked_area_data { time: time; values: readonly number[] }
export interface stacked_bars_data { time: time; values: readonly number[] }
export interface whisker_box_data {
  time: time;
  /** `[low whisker, lower quartile, median, upper quartile, high whisker]`. */
  quartiles: readonly [number, number, number, number, number];
  outliers?: readonly number[];
}

export type feature_series_data =
  | brushable_area_data
  | grouped_bars_data
  | heatmap_data
  | hlc_area_data
  | pretty_histogram_data
  | background_shade_data
  | stacked_area_data
  | stacked_bars_data
  | whisker_box_data;

export type series_data = ohlc_data | single_value_data | feature_series_data | whitespace_data;

/**
 * A partial bar for {@link series_api.merge}. Present fields overwrite the bar at `time`; absent
 * (or `undefined`) fields keep that bar's current values. `value` is an alias of `close` (use it for
 * line/area/baseline/histogram series). Volume and turnover are not bar fields: merge them into
 * their own series (for example the volume histogram) with `{ time, value }`.
 */
export interface series_merge_data {
  time: time;
  open?: number;
  high?: number;
  low?: number;
  close?: number;
  value?: number;
  /** Per-bar color overrides; absent keeps the bar's current override. */
  color?: string;
  wick_color?: string;
  border_color?: string;
}

/**
 * Columns for {@link series_api.merge_typed}: row `i` is the partial bar
 * `{ time: times[i], open: open?.[i], ... }`. A `NaN` entry, or an omitted column, is an absent
 * field that keeps the bar's current value. `close` is the value of line/area/baseline/histogram
 * series. Every present column must have the length of `times`.
 */
export interface series_merge_columns {
  /** UTC seconds, as for {@link ohlc_columns.times}. */
  times: Float64Array;
  open?: Float64Array;
  high?: Float64Array;
  low?: Float64Array;
  close?: Float64Array;
}

/**
 * Options for streaming ingestion ({@link series_api.update}, {@link series_api.merge},
 * {@link series_api.update_typed}, {@link series_api.merge_typed}) and full replaces
 * ({@link series_api.set_data}). Custom and advanced series throw `unsupported_operation` when a
 * `sequence` is supplied, because their payloads bypass the guarded OHLC path.
 */
export interface series_update_options {
  /**
   * Optional monotonic per-series sequence (a non-negative safe integer), for example the feed's
   * message sequence. On `update`/`merge`/`update_typed`/`merge_typed`, a sequence that is not
   * greater than the last one applied to this series is rejected as stale: nothing changes and
   * {@link series_api.last_ingestion_diagnostics} reports `code: "stale_sequence"` with
   * `last_sequence`. Calls without a sequence always apply and leave the guard untouched. On
   * `set_data`/`set_data_typed` it installs the snapshot's sequence as the new baseline; a full
   * replace without one clears the guard. The guard is O(1), runtime-only, and never persisted.
   * An invalid value is rejected (and warned) like invalid data, leaving the series unchanged.
   */
  sequence?: number;
}

export type footprint_aggressor_side = "buy" | "sell" | "unknown";

/** Bounded chart-owned level-two order-book configuration. */
export interface depth_options {
  tick_size: number;
  max_levels_per_side: number;
  history_bucket_micros: number;
  max_history_buckets: number;
  max_history_cells: number;
  max_event_markers: number;
}

/** Exact unsigned 64-bit values split into high/low words for a portable typed-array boundary. */
export interface uint64_columns {
  high: Uint32Array;
  low: Uint32Array;
}

/** One side of a depth snapshot; order-count 0xffffffff means unavailable. */
export interface depth_level_columns {
  prices: Float64Array;
  sizes: Float64Array;
  order_counts?: Uint32Array;
}

export interface depth_snapshot_columns {
  timestamp_micros: number;
  sequence_high: number;
  sequence_low: number;
  bids: depth_level_columns;
  asks: depth_level_columns;
}

/** Allocation-conscious incremental depth rows. Side 0 is bid and side 1 is ask. */
export interface depth_update_columns {
  timestamps_micros: Float64Array;
  sequences: uint64_columns;
  previous_sequences: uint64_columns;
  sides: Uint8Array;
  prices: Float64Array;
  sizes: Float64Array;
  order_counts?: Uint32Array;
}

export interface depth_ladder_row {
  price: number;
  bid_size: number | null;
  ask_size: number | null;
  bid_order_count: number | null;
  ask_order_count: number | null;
  distance_from_touch_ticks: number;
}

export interface depth_study_snapshot {
  /** Exact provider sequence encoded as decimal text. */
  sequence: string | null;
  best_bid: { price: number; size: number; order_count: number | null } | null;
  best_ask: { price: number; size: number; order_count: number | null } | null;
  bid_cumulative: readonly { price: number; size: number; order_count: number | null }[];
  ask_cumulative: readonly { price: number; size: number; order_count: number | null }[];
  imbalance: number | null;
}

export interface depth_heatmap_options {
  pane_index: number;
  price_min: number;
  price_max: number;
  minimum_size: number;
  maximum_size: number;
  bid_rgba: readonly [number, number, number, number];
  ask_rgba: readonly [number, number, number, number];
  opacity: number;
}

export interface depth_event_columns {
  timestamps_micros: Float64Array;
  prices: Float64Array;
  sizes: Float64Array;
  /** -1 unknown, 0 bid, 1 ask. */
  sides: Int8Array;
  /** 0 iceberg refill, 1 pulled liquidity, 2 size cluster, 3 sweep. */
  kinds: Uint8Array;
  /** Optional bounded host labels aligned one-for-one with the numeric columns. */
  labels?: readonly (string | null)[];
}

export interface depth_event_layer_options {
  pane_index: number;
  max_markers: number;
}

export interface time_and_sales_options {
  minimum_volume: number;
  /** Omit to include every classified aggressor side. */
  side: footprint_aggressor_side | null;
  /** Bounded to 4096 rows; results are newest first. */
  max_rows: number;
}

export interface time_and_sales_row {
  timestamp_micros: number;
  price: number;
  volume: number;
  aggressor: footprint_aggressor_side;
  /** Exact provider identity encoded as decimal text. */
  trade_id: string | null;
  conditions: number;
}

/** One raw trade consumed by a tick-driven footprint series. */
export interface footprint_trade {
  /** Signed Unix timestamp in integer microseconds; must be a JavaScript safe integer. */
  timestamp_micros: number;
  price: number;
  volume: number;
  aggressor?: footprint_aggressor_side;
  /** Contemporaneous quote used only when `aggressor` is unknown. */
  bid?: number;
  ask?: number;
  sequence?: number;
  trade_id?: number;
  conditions?: number;
  /** Host-defined session identity. A change resets session cumulative delta. */
  session_id?: number;
}

/** Allocation-conscious historical footprint input. Every column must have equal length. */
export interface footprint_trade_columns {
  timestamps_micros: Float64Array;
  prices: Float64Array;
  volumes: Float64Array;
  /** 0 unknown, 1 buy, 2 sell. */
  aggressors: Uint8Array;
  /** Optional numeric columns use NaN for a missing value. */
  bids: Float64Array;
  asks: Float64Array;
  sequences: Float64Array;
  trade_ids: Float64Array;
  conditions: Uint32Array;
  session_ids: Float64Array;
}

export interface footprint_level {
  level: number;
  price: number;
  bid_volume: number;
  ask_volume: number;
  unknown_volume: number;
  total_volume: number;
  delta: number;
  delta_percent: number;
  bid_imbalance: boolean;
  ask_imbalance: boolean;
  stacked_bid_imbalance: boolean;
  stacked_ask_imbalance: boolean;
}

export interface footprint_bar {
  /** Logical bar position; unlike display time, this remains unique for sub-second bars. */
  logical_index: number;
  start_timestamp_micros: number;
  end_timestamp_micros: number;
  session_id: number | null;
  open: number;
  high: number;
  low: number;
  close: number;
  bid_volume: number;
  ask_volume: number;
  unknown_volume: number;
  total_volume: number;
  delta: number;
  max_delta: number;
  min_delta: number;
  session_delta: number;
  trade_count: number;
  poc_level: number;
  poc_price: number;
  levels: footprint_level[];
}

/**
 * Columnar input for {@link series_api.set_data_typed} and {@link series_api.update_typed}: one
 * `Float64Array` per channel, all of equal length. `times` are finite whole UTC seconds in the
 * inclusive range `-62167219200..253402300799`; they are never auto-converted. Convert business
 * days / "YYYY-MM-DD" strings to UTC midnight before using this typed API. Single-value series
 * repeat their value in all four price channels.
 * Whitespace slots are all-NaN rows.
 *
 * The engine treats these arrays as read-only inputs — it neither mutates nor retains them — so
 * the same view may be passed as several channels, and views over a `SharedArrayBuffer` are safe.
 * See {@link series_api.set_data_typed} for the full guarantee.
 */
export interface ohlc_columns {
  times: Float64Array;
  open: Float64Array;
  high: Float64Array;
  low: Float64Array;
  close: Float64Array;
}

/** Structured result for an ingestion that was repaired, rejected, or semantically suspicious.
 * `null` from {@link series_api.last_ingestion_diagnostics} means every supplied row was accepted
 * without repair or anomaly. A rejected set/update preserves the series' current state. Timestamp
 * reasons identify the whole-seconds/range failure and likely millisecond/microsecond/nanosecond
 * units when applicable. Financial anomalies are accepted unchanged; hosts choose policy. */
export interface ingestion_diagnostics {
  status: "accepted_with_diagnostics" | "rejected";
  accepted: number;
  dropped_invalid: number;
  dropped_non_finite: number;
  dropped_out_of_range: number;
  deduplicated: number;
  reordered: boolean;
  semantic_anomalies: number;
  reason?: string;
  /**
   * Machine-readable cause for streaming diagnostics:
   * - `stale_sequence` — rejected: the `sequence` option was not newer than `last_sequence`;
   * - `partial_ohlc` — rejected: an `update()` point carried only some OHLC fields (use `merge()`);
   * - `value_on_ohlc_series` — accepted: `{ time, value }` replaced a candlestick/bar with a flat
   *   O=H=L=C bar (reference behavior; use `merge()` to move only the close);
   * - `price_less_payload` — accepted: a point without price fields (for example
   *   `{ time, volume }`) replaced the bar with whitespace (reference behavior; use `merge()`, and
   *   update volume on its own series);
   * - `empty_merge` — rejected: a `merge()` carried no price field;
   * - `derived_series` — rejected: the series is engine-owned (a trade-bound candle or bar, a
   *   CVD, delta, or volume study, or resampled or synthetic bars), so a host data write changed
   *   nothing. Feed its trade stream or source instead. `pop()` on such a series records the same
   *   rejection. A footprint handle throws `unsupported_operation` instead.
   */
  code?: "stale_sequence" | "partial_ohlc" | "value_on_ohlc_series" | "price_less_payload" | "empty_merge" | "derived_series";
  /** Last sequence applied to the series, reported with `code: "stale_sequence"`. */
  last_sequence?: number;
}

/**
 * Last-frame render telemetry, read with {@link chart_api.frame_stats}.
 *
 * Every field describes the **most recent frame** except `dropped_frames` and
 * `presented_frames`, which are lifetime counters since chart create. The engine keeps a single
 * fixed-size record — there is no history buffer — and the façade reuses one scratch array per
 * chart, so polling every frame allocates nothing and costs two `performance.now()` reads
 * inside the engine.
 */
export interface frame_stats {
  /** CPU time in ms for the last frame: layout, axis-frame construction, engine frame build,
   *  plugin passes, and command encoding. */
  cpu_ms: number;
  /** GPU time in ms for the last frame via WebGPU timestamp queries; `null` on `canvas2d`, on a
   *  device without the `timestamp-query` feature, or before the first readback resolves.
   *  Collection is armed by the first `frame_stats()` call, so expect one or two `null` reads
   *  after chart create even where the feature is available. Readback is asynchronous, so this
   *  is the most recently *resolved* frame rather than strictly the last presented one. */
  gpu_ms: number | null;
  /** Draw calls issued for the last frame. WebGPU: render-pass draw calls. Canvas2D: paint ops
   *  issued by the pane executor (the nearest equivalent). */
  draw_calls: number;
  /** Frames the engine began but did not present, since chart create (surface acquisition
   *  failed or timed out, so the previous frame stayed on screen). */
  dropped_frames: number;
  /** Frames presented since chart create, on whichever backend was active. */
  presented_frames: number;
  /** Wasm linear memory currently reserved, in bytes. Never shrinks — see the series retention
   *  notes on {@link series_options.max_points}. */
  memory_bytes: number;
  /** Canvas2D paint ops the **engine** issued for the last frame: the axis/crosshair overlay,
   *  plus the pane executor on the Canvas2D backend. Excludes plugin canvas primitives, which
   *  are package-side (see `canvas_plugins`). Extension beyond the consumer's requested shape:
   *  it is how "the WebGPU path does no per-frame Canvas2D work" is asserted. */
  canvas2d_ops: number;
  /** Producer overruns observed across all ring sources since chart create
   *  (see {@link series_api.set_ring_source}); 0 while no ring is bound. */
  ring_overruns: number;
  /** Ring rows dropped since chart create because their timestamp or values were invalid
   *  (non-finite, fractional, or out of range); 0 while no ring is bound. */
  ring_dropped_rows: number;
  /** WebGPU vertex-buffer allocations made for the most recent frame. A warmed, unchanged chart
   *  reports 0; capacity grows geometrically and is retained until chart removal. */
  gpu_buffer_allocations: number;
  /** WebGPU queue buffer-write calls made for the most recent frame. */
  gpu_write_calls: number;
  /** Vertex bytes uploaded to WebGPU for the most recent frame. */
  gpu_uploaded_bytes: number;
  /** Retained layout recomputations performed for the most recent frame. */
  layout_rebuilds: number;
  /** Autoscale passes performed for the most recent frame. */
  autoscale_runs: number;
  /** Individual retained series layers rebuilt for the most recent frame. */
  series_rebuilds: number;
  /** Retained drawing layers rebuilt for the most recent frame. */
  drawing_rebuilds: number;
  /** Retained first-party trading layers rebuilt for the most recent frame. */
  trading_rebuilds: number;
  /** Retained grid/underlay layers rebuilt for the most recent frame. */
  grid_rebuilds: number;
  /** Retained interaction-overlay layers rebuilt for the most recent frame. */
  overlay_rebuilds: number;
  /** Browser axis/top-layer primitive rebuilds for the most recent frame. */
  axis_rebuilds: number;
  /** Browser text runs rasterized and resolved into atlas slots for the most recent frame. */
  text_resolutions: number;
}

/**
 * Byte layout of a `SharedArrayBuffer` ring bound with {@link series_api.set_ring_source}. Every
 * offset is in **bytes**, relative to the start of the buffer except the per-channel offsets, which
 * are relative to the start of a row.
 *
 * The channel offsets are independent, so a row may be a packed `f64[5]`, a wider struct with the
 * price channels next to unrelated fields, or a single-value layout where all four price offsets
 * point at the same 8 bytes.
 */
export interface ring_source_layout {
  /** Byte offset of the first row. */
  data_offset: number;
  /** Bytes per row. Must be at least the end of the furthest channel. */
  row_stride: number;
  /** Maximum rows the ring holds before wrapping. */
  capacity: number;
  /** Byte offsets, within a row, of each `f64` channel. `time` is UTC seconds, as in
   *  {@link ohlc_columns}. */
  time_offset: number;
  open_offset: number;
  high_offset: number;
  low_offset: number;
  close_offset: number;
  /**
   * Byte offset of an `Int32` monotonic write cursor — the count of rows the producer has **ever**
   * written, not a ring slot. Must be 4-byte aligned. The engine reads it with `Atomics.load`.
   *
   * The producer's base contract: write the row's bytes **first**, then publish the incremented
   * count with `Atomics.store`. The cursor may overflow `Int32`; the engine handles the wrap.
   */
  write_cursor_offset: number;
  /**
   * Optional byte offset, within every row, of an aligned `Int32` sequence word. Set this for a
   * producer that can wrap while the engine is draining; it upgrades the base cursor handshake to
   * a per-slot seqlock and guarantees the engine never accepts a torn generation.
   *
   * For logical row count `next = previous_cursor + 1`, the producer must:
   * 1. `Atomics.store(sequence, ~next)` before touching the row;
   * 2. write all `f64` channels;
   * 3. `Atomics.store(sequence, next)`;
   * 4. `Atomics.store(write_cursor, next)`.
   *
   * The engine checks every selected slot before and after its bulk copy and retries a frame when
   * a sequence changed. Omit only for backwards compatibility with producers that cannot lap the
   * consumer during a copy; a double cursor check still rejects fully-published overlap, but cannot
   * detect a producer currently midway through an unpublished overwrite.
   */
  sequence_offset?: number;
}

/** Inclusive logical (bar-index) range. */
export interface logical_range {
  from: number;
  to: number;
}

/** Inclusive time range (UTC seconds). */
export interface time_range {
  from: number;
  to: number;
}

export interface bars_info extends Partial<time_range> {
  bars_before: number;
  bars_after: number;
}

/** reference mismatch-direction values used by `data_by_index`. */
export type mismatch_direction = -1 | 0 | 1;
export type data_changed_scope = "full" | "update";
export type data_changed_handler = (scope: data_changed_scope) => void;

/**
 * The last value of a series as reported by the engine (cf. reference `LastValueDataResult`, which
 * instead returns `{ noData, price, color, ... }`). `formatted` is the value rendered with the
 * series' price format; `time` is the UTC-second timestamp of the bar the value came from.
 */
export interface last_value_data {
  value: number;
  formatted: string;
  time: number;
}

/**
 * One live series in {@link chart_api.value_snapshot}. Numeric and formatted fields are null when
 * the series is missing or whitespace at an exact logical index. `value` is the current scalar
 * value; OHLC series instead populate `open`/`high`/`low`/`close`. Experimental custom series have
 * null exact-index values; latest mode can expose only the last value recorded by a visible frame.
 */
export interface chart_value_snapshot {
  series: series_api;
  series_id: number;
  kind: series_kind;
  pane_index: number;
  price_scale_id: string;
  logical_index: number | null;
  /** UTC-second timestamp selected independently per series in latest mode. */
  time: number | null;
  open: number | null;
  high: number | null;
  low: number | null;
  close: number | null;
  value: number | null;
  /** Previous same-series non-whitespace close/scalar value. */
  previous_value: number | null;
  formatted_open: string | null;
  formatted_high: string | null;
  formatted_low: string | null;
  formatted_close: string | null;
  formatted_value: string | null;
  formatted_previous_value: string | null;
}

/** Parameters delivered to crosshair-move and click subscribers (mirrors reference `MouseEventParams`). */
export interface mouse_event_params {
  /** UTC seconds of the bar under the cursor, or `null` off the data. */
  time: number | null;
  /** Float logical (bar) index under the cursor, or `null` when there is no data. */
  logical: number | null;
  /**
   * Cursor position in CSS px in the chart's shared coordinate space (see {@link pane_geometry}):
   * `x` from the plot-area left edge (right of the left price strip), `y` from the top of the
   * stacked pane area. `y` is not pane-local; subtract the hovered pane's `get_geometry().top`
   * for that. `null` when the cursor left the chart.
   */
  point: { x: number; y: number } | null;
  /** Index of the pane under the cursor, or `null` over an axis strip or outside the panes. */
  pane_index: number | null;
  /** Per-series value at the hovered bar, keyed by the series handle. */
  series_data: Map<series_api, ohlc_data | single_value_data>;
  /** Rich engine snapshot at the crosshair index; on crosshair leave this is the latest snapshot. */
  value_snapshot: chart_value_snapshot[];
  /**
   * The series under the cursor (reference `MouseEventParams.hoveredSeries`), from the engine's
   * per-kind hit tests (candle/bar high-low range, histogram column, line stroke) — or a
   * series primitive's hit, whose owning series reports here. `null` when nothing is hit.
   */
  hovered_series: series_api | general_series_api | null;
  /** Exact engine-owned mark hit for a general series, otherwise `null`. */
  general_hit: general_series_hit | null;
  /**
   * The `external_id` a primitive's `hit_test` reported for the hovered object (reference
   * `MouseEventParams.hoveredObjectId`), or `null` when no primitive is hit.
   */
  hovered_object_id: string | null;
}

export type mouse_event_handler = (params: mouse_event_params) => void;
/** Engine-resolved context delivered for a secondary click inside a pane. */
export interface chart_context_params extends mouse_event_params {
  /** Exact price at the click Y on the hit series' scale, or the pane's canonical default scale. */
  price: number;
}
export type chart_context_handler = (params: chart_context_params) => void;
export type dbl_click_handler = (params: mouse_event_params) => void;
export type visible_logical_range_handler = (range: logical_range | null) => void;
export type visible_time_range_handler = (range: time_range | null) => void;
/** Receives the time scale's new media size in px (reference `SizeChangeEventHandler`). */
export type size_change_handler = (width: number, height: number) => void;

/**
 * Geometry of a pane's content area in CSS px relative to the chart container's top-left —
 * the anchor for platform-rendered per-pane chrome (indicator chips, legends). It also defines
 * the chart's public coordinate space, shared by every `price_to_coordinate` /
 * `coordinate_to_price` / `time_to_coordinate` / `coordinate_to_time` / `logical_to_coordinate` /
 * `coordinate_to_logical` conversion, pointer `point`, crosshair, hit-test, drawing and trading
 * position: `x` is CSS px from the plot-area left edge (container x = `left` + x) and `y` is CSS
 * px from the top of the stacked pane area (pane 0's `top`). A pane spans
 * `[top, top + height]` in that `y`; pane-local `y` is `y - top`. A `y` is never pane-local, so a
 * lower pane's price maps to a `y` at or below that pane's `top`. All values reflect the last
 * layout pass.
 */
export interface pane_geometry {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** One live overlay value resolved against the chart-wide comparison anchor. */
export interface comparison_legend_entry {
  series_id: number;
  title: string;
  anchor_time: number | null;
  anchor_value: number | null;
  latest_time: number | null;
  latest_value: number | null;
  change: number | null;
  percent_change: number | null;
}

/** Scalar input accepted by a built-in indicator. The source series may itself be an indicator output. */
export type indicator_input_source = "open" | "high" | "low" | "close" | "hl2" | "hlc3" | "ohlc4" | "hlcc4";
export type indicator_kind = "sma" | "ema" | "dema" | "tema" | "smma" | "hma" | "vwma" | "standard_deviation" | "cci" | "williams_r" | "stochastic_rsi" | "momentum" | "roc" | "donchian" | "pivot_points" | "zigzag" | "keltner" | "adx_dmi" | "parabolic_sar" | "supertrend" | "ichimoku" | "ema_ribbon" | "bollinger" | "rsi" | "macd" | "stochastic" | "atr" | "vwap" | "obv" | "cmf" | "mfi" | "volume" | "vwap_bands" | "wma" | "kdj" | klinechart_indicator_kind;
/**
 * One KLineChart indicator template with its parameters, as {@link chart_api.add_klinechart_indicator}
 * takes it and `indicator_info().parameters.klinechart` reports it. `indicator` names the template and
 * every other field is that template's `calcParams`, spelled out: nothing is defaulted, so a definition
 * missing a field is rejected. Periods are whole numbers from 1 to 1,000,000 and the `SMA` weight and
 * `SAR` factors are positive; the `BOLL` multiplier is not negative. A list of periods holds one to five
 * entries (`VOL`: one to four), one output line each.
 *
 * The doc comment of each template gives KLineChart's default parameters.
 */
export type klinechart_indicator =
  /** Rolling mean of close per period. Default `[5, 10, 30, 60]`. Outputs `ma1`..`ma5`. */
  | { indicator: "ma"; periods: number[] }
  /** Exponential moving average of close per period. Default `[6, 12, 20]`. Outputs `ema1`..`ema5`. */
  | { indicator: "ema"; periods: number[] }
  /** Weighted `SMA(CLOSE, N, M)` smoothing. Default `period` 12, `weight` 2. Output `sma`. */
  | { indicator: "sma"; period: number; weight: number }
  /** Bollinger bands. Default `period` 20, `multiplier` 2. Outputs `up`, `mid`, `dn`. */
  | { indicator: "boll"; period: number; multiplier: number }
  /** Parabolic stop-and-reverse, with `start`, `step`, and `max` in percent. Default 2, 2, 20. Output `sar`, drawn as dots. */
  | { indicator: "sar"; start: number; step: number; max: number }
  /** Bull and bear index: the average of four moving averages. Default `[3, 6, 12, 24]`. Output `bbi`. */
  | { indicator: "bbi"; periods: [number, number, number, number] }
  /** Average traded price, `SUM(TURNOVER) / SUM(VOLUME)`. No parameters. Output `avp`. The source series carries turnover; needs a volume series. */
  | { indicator: "avp" }
  /** Volume bars plus a moving average of volume per period. Default `[5, 10, 20]`. Outputs `volume` (bars), `ma1`..`ma4`. Needs a volume series. */
  | { indicator: "vol"; periods: number[] }
  /** Moving average convergence divergence. Default `short` 12, `long` 26, `signal` 9. Outputs `dif`, `dea`, `macd` (bars). */
  | { indicator: "macd"; short: number; long: number; signal: number }
  /** Stochastic KDJ. Default `period` 9, `k_smoothing` 3, `d_smoothing` 3. Outputs `k`, `d`, `j`. */
  | { indicator: "kdj"; period: number; k_smoothing: number; d_smoothing: number }
  /** Relative strength index, one line per period. Default `[6, 12, 24]`. Outputs `rsi1`..`rsi5`. */
  | { indicator: "rsi"; periods: number[] }
  /** Bias ratio, one line per period. Default `[6, 12, 24]`. Outputs `bias1`..`bias5`. */
  | { indicator: "bias"; periods: number[] }
  /** Buying and selling momentum (BR and AR). Default `period` 26. Outputs `br`, `ar`. */
  | { indicator: "brar"; period: number }
  /** Commodity channel index. Default `period` 20. Output `cci`. */
  | { indicator: "cci"; period: number }
  /** Directional movement index. Default `period` 14, `adxr_period` 6. Outputs `pdi`, `mdi`, `adx`, `adxr`. */
  | { indicator: "dmi"; period: number; adxr_period: number }
  /** Current ratio plus four shifted moving averages of it. Default `period` 26, `ma_periods` `[10, 20, 40, 60]`. Outputs `cr`, `ma1`..`ma4`. */
  | { indicator: "cr"; period: number; ma_periods: [number, number, number, number] }
  /** Psychological line. Default `period` 12, `ma_period` 6. Outputs `psy`, `maPsy`. */
  | { indicator: "psy"; period: number; ma_period: number }
  /** Different of moving average. Default `short` 10, `long` 50, `signal` 10. Outputs `dma`, `ama`. */
  | { indicator: "dma"; short: number; long: number; signal: number }
  /** Triple exponentially smoothed average. Default `period` 12, `ma_period` 9. Outputs `trix`, `maTrix`. */
  | { indicator: "trix"; period: number; ma_period: number }
  /** On-balance volume. Default `ma_period` 30. Outputs `obv`, `maObv`. Needs a volume series. */
  | { indicator: "obv"; ma_period: number }
  /** Volume ratio. Default `period` 26, `ma_period` 6. Outputs `vr`, `maVr`. Needs a volume series. */
  | { indicator: "vr"; period: number; ma_period: number }
  /** Williams %R, one line per period. Default `[6, 10, 14]`. Outputs `wr1`..`wr5`. */
  | { indicator: "wr"; periods: number[] }
  /** Momentum. Default `period` 12, `ma_period` 6. Outputs `mtm`, `maMtm`. */
  | { indicator: "mtm"; period: number; ma_period: number }
  /** Ease of movement value. Default `period` 14 (KLineChart lists a second parameter, 9, that its formula never reads). Outputs `emv`, `maEmv`. Needs a volume series. */
  | { indicator: "emv"; period: number }
  /** Rate of change. Default `period` 12, `ma_period` 6. Outputs `roc`, `maRoc`. */
  | { indicator: "roc"; period: number; ma_period: number }
  /** Price and volume trend. No parameters. Output `pvt`. Needs a volume series. */
  | { indicator: "pvt" }
  /** Awesome oscillator. Default `short` 5, `long` 34. Output `ao` (bars). */
  | { indicator: "ao"; short: number; long: number };
/**
 * KLineChart's 27 template names, price overlays first: `ma`, `ema`, `sma`, `boll`, `sar`, `bbi`, `avp`,
 * `vol`, `macd`, `kdj`, `rsi`, `bias`, `brar`, `cci`, `dmi`, `cr`, `psy`, `dma`, `trix`, `obv`, `vr`, `wr`,
 * `mtm`, `emv`, `roc`, `pvt`, `ao`. The discriminant of {@link klinechart_indicator}.
 */
export type klinechart_indicator_name = klinechart_indicator["indicator"];
/** The {@link indicator_kind} of a KLineChart binding (and the `kind` {@link chart_api.indicator_schema} takes): `klinechart_` followed by the template name. */
export type klinechart_indicator_kind = `klinechart_${klinechart_indicator_name}`;
export type pivot_kind = "standard" | "fibonacci" | "camarilla" | "woodie" | "demark";
/** VWAP reset period in exchange trading days: one day, a Monday-start week, or a calendar month. */
export type vwap_reset = "session" | "weekly" | "monthly";
/**
 * Seed of a recursive average. `"sma"` seeds with the mean of the first N samples, so values start
 * at sample N-1 (TradingView `ta.ema`/`ta.rma`, TA-Lib). `"first_value"` seeds with the first sample,
 * so values start immediately (通达信/同花顺 `EMA(X,N)` and `SMA(X,N,M)`).
 */
export type indicator_seed = "sma" | "first_value";
/** Bollinger standard deviation: `"population"` divides by N (TradingView), `"sample"` by N-1 (通达信/同花顺 `STD`). */
export type deviation_estimator = "population" | "sample";
/**
 * Convenience preset expanded by the engine into explicit parameters. `"tradingview"` (the default)
 * selects SMA seeds, a `MACD - signal` histogram, and population deviation; `"china"` selects
 * first-value seeds, the `(DIF-DEA)*2` histogram, and sample deviation. Explicit fields override
 * the preset, and only the expanded parameters are reported and persisted.
 */
export type indicator_convention = "tradingview" | "china";
export interface indicator_convention_parameters {
  convention?: indicator_convention;
}
/** Calculation parameters for {@link chart_api.add_ema}, `add_dema`, `add_tema`, and RSI. */
export interface indicator_seed_parameters extends indicator_convention_parameters {
  seed?: indicator_seed;
}
/** Calculation parameters for {@link chart_api.add_macd}. */
export interface macd_parameters extends indicator_seed_parameters {
  /** Histogram scale: 1 is `MACD - signal` (default), 2 is 通达信/同花顺/富途 `(DIF-DEA)*2`. */
  histogram_multiplier?: number;
}
/**
 * How KDJ starts: `"fifty"` (default) is the textbook start (通达信 KDJ传统版), which waits for a full
 * RSV window and uses 50 for the missing previous K and D; `"first_value"` is the 通达信 formula KDJ
 * (KDJ普通版), whose RSV uses the bars available while fewer than `period` exist and whose `SMA`
 * starts at its first input, so K = D = J = RSV on the first bar. They converge after
 * `convergence_bars`.
 */
export type kdj_seed = "fifty" | "first_value";
/** Calculation parameters for {@link chart_api.add_kdj}; `convention: "china"` selects `"first_value"`. */
export interface kdj_parameters extends indicator_convention_parameters {
  seed?: kdj_seed;
}
/** Calculation parameters for {@link chart_api.add_bollinger}. */
export interface bollinger_parameters extends indicator_convention_parameters {
  estimator?: deviation_estimator;
}
/** Calculation parameters for {@link chart_api.add_vwap}. */
export interface vwap_parameters {
  /**
   * Scalar turnover series. When present the line is the 分时 average price
   * `sum(amount) / sum(volume)` per reset period; `volume_source` is then required. Both columns align
   * by timestamp, and rows without positive volume or finite turnover contribute nothing.
   */
  amount_source?: series_api | null;
}
export type indicator_parameter_type = "integer" | "number" | "source" | "series" | "choice";
export interface indicator_parameter_descriptor {
  name: string;
  parameter_type: indicator_parameter_type;
  default: unknown;
  min: number | null;
  max: number | null;
  /** Allowed values of a `"choice"` parameter. */
  choices?: string[];
}
export interface indicator_output_descriptor {
  name: string;
  index: number;
  supports_style: boolean;
}
/** Engine-owned presentation state for one output of a study binding. */
export interface indicator_output_style {
  visible: boolean;
  line_color: string | null;
  line_width: number | null;
  line_style: number;
  point_markers: boolean;
  up_color: string | null;
  down_color: string | null;
  area_top_color: string | null;
  area_bottom_color: string | null;
}
export interface indicator_schema {
  revision: number;
  kind: indicator_kind;
  parameters: indicator_parameter_descriptor[];
  outputs: indicator_output_descriptor[];
}

/**
 * An indicator output series' lineage (engine `IndicatorInfo`): which binding it belongs to
 * (kind + params), the source series it derives from, and which output slot it is — everything
 * a platform needs to render its own industry-standard indicator chip (title, params, source,
 * hide/remove actions) without the engine owning any UI. Bollinger slots: 0 = upper,
 * 1 = middle, 2 = lower; EMA ribbon slots follow its five configured periods; SMA/EMA: always 0.
 */
export interface indicator_info {
  /** Stable identity shared by all outputs in one indicator binding. */
  binding_id: number;
  kind: indicator_kind;
  /** Complete structured parameters. Fields not used by this kind are `null`. */
  parameters: {
    period: number | null;
    periods: [number, number, number, number, number] | null;
    pivot_kind: pivot_kind | null;
    deviation: number | null;
    fast: number | null;
    slow: number | null;
    signal: number | null;
    k_period: number | null;
    d_period: number | null;
    reset: vwap_reset | null;
    standard_deviation: number | null;
    percent: number | null;
    /** EMA/DEMA/TEMA/MACD/RSI seed convention. */
    seed: indicator_seed | null;
    /** MACD histogram scale. */
    histogram_multiplier: number | null;
    /** Bollinger standard-deviation estimator. */
    estimator: deviation_estimator | null;
    /** KDJ K and D smoothing (`SMA(X,N,1)` lengths). */
    k_smoothing: number | null;
    d_smoothing: number | null;
    /** KDJ K/D start (the `seed` of `add_kdj` parameters). */
    kdj_seed: kdj_seed | null;
    /** The complete KLineChart definition of a `klinechart_*` binding, as passed to
     *  {@link chart_api.add_klinechart_indicator}. Absent for every other kind. */
    klinechart?: klinechart_indicator;
  };
  /** For a KLineChart template, its first period (0 for `avp`, `pvt`, and `sar`, which have none). */
  period: number;
  /** Second parameter when the kind has one: Bollinger deviation, MACD signal period,
   *  Stochastic %D period; otherwise `null`. KLineChart bindings always report `null`. */
  deviation: number | null;
  source: series_api;
  /** Scalar OHLC/aggregate input selected for the binding. */
  source_input: indicator_input_source;
  /** The bound volume series (VWAP, the volume studies, and KLineChart's `vol`, `obv`, `pvt`, `emv`,
   *  `vr`, and `avp`), otherwise `null`. */
  volume_source: series_api | null;
  /** The turnover series of an amount-weighted VWAP, otherwise `null`. */
  amount_source: series_api | null;
  /** Current engine-owned presentation state for this output. */
  style: indicator_output_style;
  /** Stable display name for this output, preserving binding output order. */
  output_name: string;
  /** Bollinger: 0 = upper, 1 = middle, 2 = lower. EMA ribbon: fastest-to-slowest configured
   *  period. MACD: 0 = line, 1 = signal, 2 = histogram. Stochastic: 0 = %K, 1 = %D.
   *  KDJ: 0 = K, 1 = D, 2 = J. Single-output indicators: 0. */
  output_index: number;
  output_count: number;
  /** Bars of the root price source before this output's first value, including every chained
   *  indicator source (assuming no whitespace rows). */
  warmup_bars: number;
  /** Recommended bars of history before this output no longer depends on where loaded history
   *  begins: the warm-up for windowed formulas, plus the bars for every recursive seed's weight
   *  to fall below 0.1%. `null` when no bar count suffices (session VWAP, pivots, OBV, SAR,
   *  SuperTrend, ZigZag). Load at least this many bars before the first visible bar. */
  convergence_bars: number | null;
}

/** Five EMA periods in fastest-to-slowest output order. */
export type ema_ribbon_periods = readonly [number, number, number, number, number];

/** Per-output style overrides in the same order as {@link ema_ribbon_periods}. */
export type ema_ribbon_options = readonly [
  Partial<series_options>?,
  Partial<series_options>?,
  Partial<series_options>?,
  Partial<series_options>?,
  Partial<series_options>?,
];

/** Series lifecycle event (platform chrome: legend chips, indicator counts). */
export interface series_change_event {
  series: series_api | general_series_api;
  pane_index: number;
}
export type series_change_handler = (event: series_change_event) => void;
/** Receives the patch passed to {@link chart_api.apply_options} (theme/border retokening). */
export type options_change_handler = (options: deep_partial<chart_options>) => void;

export interface time_scale_options {
  /** Distance between adjacent bars in CSS pixels. */
  bar_spacing: number;
  /** Empty logical bars between the final data point and the right edge. */
  right_offset: number;
  /** Minimum bar spacing in CSS pixels (reference `minBarSpacing`, default 0.5). */
  min_bar_spacing?: number;
  /** Maximum bar spacing in CSS pixels (reference `maxBarSpacing`); omit for unlimited. */
  max_bar_spacing?: number;
  /** Right margin after the last bar in CSS pixels (reference `rightOffsetPixels`). */
  right_offset_pixels?: number;
  /** Show the time of day (not just the date) in axis/crosshair labels (reference `timeVisible`). */
  time_visible?: boolean;
  /** Include seconds when the time is shown (reference `secondsVisible`). */
  seconds_visible?: boolean;
  /** Prevent scrolling past the first data point on the left (reference `fixLeftEdge`). */
  fix_left_edge?: boolean;
  /** Prevent scrolling past the last data point on the right (reference `fixRightEdge`). */
  fix_right_edge?: boolean;
  /** Keep the visible range constant across chart resizes (reference `lockVisibleTimeRangeOnResize`). */
  lock_visible_time_range_on_resize?: boolean;
  /**
   * Keep the right-most bar pinned during ordinary time-scale zoom. Defaults to `true`, matching
   * measured TradingView wheel zoom: the gap after the latest bar stays constant while history
   * compresses or expands. Ctrl/Cmd + wheel and pinch always zoom around the pointer. Set `false`
   * for cursor-anchored ordinary zoom (the Lightweight Charts default).
   */
  right_bar_stays_on_scroll?: boolean;
  /**
   * Shift the visible range to the right (into the future) by the number of new bars when new
   * data is added. Only applies when the last bar is visible (reference `shiftVisibleRangeOnNewBar`,
   * default `true`).
   */
  shift_visible_range_on_new_bar?: boolean;
  /**
   * Allow the visible range to shift right when a new bar replaces an existing whitespace time
   * point. Only applies when the last bar is visible and `shift_visible_range_on_new_bar` is
   * enabled (reference `allowShiftVisibleRangeOnWhitespaceReplacement`, default `false`).
   */
  allow_shift_visible_range_on_whitespace_replacement?: boolean;
  /** reference `timeScale.allowBoldLabels` (default true): bold the major time tick labels. */
  allow_bold_labels?: boolean;
  /**
   * Hold the visible logical range exactly across data updates and resizes (Aeris extension,
   * default `false`). New bars never shift the view, resizes rescale the bar spacing, and a
   * range passed to `set_visible_logical_range` is applied without the reference scroll clamps.
   * Use it for fixed full-session intraday (time-sharing) views: install every session slot as
   * whitespace rows, then call `set_visible_logical_range({ from: 0, to: slots - 1 })`; the
   * view stays exact from the pre-open state through the close. Explicit scrolls and zooms still
   * apply and become the held range.
   */
  lock_visible_logical_range?: boolean;
  /**
   * Show the whole time-scale strip (reference `timeScale.visible`, default `true`). Distinct from
   * `time_visible`, which only controls whether the labels show the time of day.
   */
  visible?: boolean;
  /** Draw small vertical lines on the time axis labels (reference `ticksVisible`, default `false`). */
  ticks_visible?: boolean;
  /**
   * Minimum height of the time scale in CSS px (reference `minimumHeight`, default 0 = auto, i.e.
   * ~28 px). Exceeded when the scale needs more space; useful to align horizontally stacked
   * charts' scale heights.
   */
  minimum_height?: number;
  /**
   * Maximum tick-mark label length in characters, overriding the built-in cap
   * (reference `tickMarkMaxCharacterLength`, default 8).
   */
  tick_mark_max_character_length?: number;
  /** Custom time-axis tick formatter (reference `tickMarkFormatter`). Receives `(timeSeconds, tickMarkType,
   *  locale, context)` where tickMarkType is 0 Year, 1 Month, 2 DayOfMonth, 3 Time, 4 TimeWithSeconds
   *  and `context.business_day` identifies calendar-date rows. */
  tick_mark_formatter?: (
    time: number,
    tick_mark_type: number,
    locale: string,
    context: time_label_context,
  ) => string;
  /**
   * Exchange time zone for tick boundaries and built-in time labels (default `"UTC"`). Canonical
   * data times stay UTC seconds; see {@link time_zone}.
   */
  time_zone?: time_zone;
  /**
   * Seconds from exchange-local midnight at which a trading day begins (default `0`). Negative
   * values assign an evening session to the next trading day (e.g. `-3 * 3600` makes a 21:00
   * night session start the next day; a Friday-night session then belongs to Monday, and
   * `-25200` makes the Sunday 17:00 open of a Globex-style market belong to Monday). With a
   * negative start every Saturday or Sunday instant belongs to Monday, and window placement
   * (`session_slot_times`, `resample_boundaries`, `set_trade_stream_sessions`) assumes the week
   * opens on Friday evening. Drives Day/Month/Year tick marks, VWAP session/weekly/monthly
   * resets, and pivot sessions.
   */
  session_start?: number;
  /**
   * Explicit time-axis marks (Aeris extension, default `null`): the listed anchors replace the
   * automatic tick selection for both the axis labels and the vertical grid; `null` restores it.
   * Times must be strictly ascending (at most 512 marks, labels at most 64 bytes). A mark draws
   * only where a time point has exactly its time, including whitespace slots reserved for bars
   * that have not traded; without a `label` the built-in exchange-time label for that point is
   * used (set `time_visible` for `HH:MM`), an empty label keeps just the grid line, and labels stay
   * inside the axis and skip one that would overlap its predecessor.
   */
  tick_marks?: readonly time_tick_mark[] | null;
  /**
   * Which instant of a bar its time TEXT prints (Aeris extension, default `"open"`). A bar keeps
   * its open time as its identity everywhere (rows, series data, crosshair events, snapshots,
   * countdown, replay, sessions, drawings, markers, alerts, `tick_marks[].time`, visible ranges);
   * `{ anchor: "close", ... }` only changes what the chart prints for a bar: the crosshair label,
   * automatic and default explicit tick labels, drawing axis tags and statistics, and the tooltip
   * and accessibility time text. A one-minute bar opened 09:30 then reads 09:31. See
   * {@link bar_time_label}. Ignored on calendar-date axes and non-time bar sequences. While a
   * label with windows is set, a `session_start` those windows do not fit is rejected with
   * `invalid_options` (send both in one call, or set `"open"` first).
   */
  bar_time_label?: bar_time_label;
}

/**
 * Close-time display labels (see {@link time_scale_options.bar_time_label}). `interval_seconds`
 * is the chart's primary bar interval, 1 to 86 399: the printed time of a bar is its open plus
 * that interval. Optional exchange-local `windows` (the `["HH:MM", "HH:MM"]` form of
 * {@link session_slot_options.windows}, at most 32, valid for the chart's `session_start`) end
 * each window's short last bar exactly, so a 15:30 hourly bar of a 16:00 session prints 16:00
 * rather than 16:30. A bar whose open lies in no window (for example a host-shifted 09:29
 * auction row) prints open plus the interval. Every bar time the host passes in or reads back
 * stays open-stamped. The windows must fit the chart's `session_start`: the same call may change
 * both, and a later `session_start` the windows cannot be placed on is rejected.
 */
export type bar_time_label =
  | "open"
  | {
      anchor: "close";
      interval_seconds: number;
      windows?: readonly (readonly [string, string])[];
    };

/** One explicit time-axis mark (see {@link time_scale_options.tick_marks}). */
export interface time_tick_mark {
  time: time;
  label?: string;
}

/** Which instant of each bar names its session slot. */
export type session_slot_convention = "bar_open" | "bar_close" | "bar_close_with_open";

/** Input of {@link session_slot_times}. */
export interface session_slot_options {
  /** Trading date, `"YYYY-MM-DD"` or a business day. */
  date: string | business_day;
  /**
   * Exchange-local `["HH:MM", "HH:MM"]` windows in chronological order (at most 32). An end at or
   * before its start crosses midnight; `"24:00"` ends at midnight.
   */
  windows: readonly (readonly [string, string])[];
  /** Bar interval in seconds (1..86 400). */
  interval_seconds: number;
  /** Exchange time zone: an IANA name or an explicit schedule (default `"UTC"`). */
  time_zone?: time_zone;
  /**
   * Trading-day start relative to local midnight (default 0). It is this call's own value and is
   * not read from the chart, so pass the chart's `session_start` (`-10800` for China futures) to
   * place night windows on the evening before. A negative start places windows assuming the week
   * opens on Friday evening; a Sunday-open market (CME Globex) passes `0` with one call per
   * evening date and `[["17:00", "16:00"]]`.
   */
  session_start?: number;
  /**
   * `"bar_open"` (default, the canonical bar time): 09:30..11:29 and 13:00..14:59 for an A-share
   * day. `"bar_close"`: 09:31..11:30 and 13:01..15:00. `"bar_close_with_open"` adds the opening
   * instant as its own slot: the 241 points 同花顺/富途 show (09:30, 09:31..11:30, 13:01..15:00).
   */
  convention?: session_slot_convention;
}

/**
 * Where a print outside every session window goes: `"fold"` keeps it (a print before the trading
 * day's first window, such as the opening auction, joins that window's first bar; a later one,
 * such as the 11:30 or closing print, joins the preceding window's last bar); `"exclude"` leaves it
 * out of every bar except prints stamped in a window's closing second (the closing auction).
 */
export type out_of_session_policy = "fold" | "exclude";

/** Session anchoring of a trade stream's time bars (see {@link chart_api.set_trade_stream_sessions}). */
export interface trade_session_options {
  /**
   * Exchange-local `["HH:MM", "HH:MM"]` windows in chronological order (at most 32), placed in the
   * chart's `time_zone` and `session_start` like {@link session_slot_options.windows}. Each window
   * restarts the bar grid at its open; an interval of one day gives one bar per trading day. The
   * one list applies to every date, and there is no per-call start: a Sunday-open market (CME
   * Globex) keeps the chart's `session_start` at `0` with one crossing window
   * `[["17:00", "16:00"]]`, because a negative chart start places Monday's window on Friday evening.
   */
  windows: readonly (readonly [string, string])[];
  /** Default `"fold"`. */
  outside?: out_of_session_policy;
}

/** One resampling period: bars restart at `start_time` and never cross `end_time` (UTC seconds). */
export interface resample_boundary {
  start_time: number;
  /** Exclusive. */
  end_time: number;
  /**
   * Opaque session identity; {@link resample_boundaries} uses the requested date as `YYYYMMDD`
   * (the trading date, or the evening date for a Sunday-open market passed with `session_start: 0`).
   */
  session_id: number;
}

/** Input of {@link resample_boundaries}. */
export interface resample_boundary_options {
  /** Trading dates (`"YYYY-MM-DD"` or business days), strictly ascending; host calendar data. */
  dates: readonly (string | business_day)[];
  /** Exchange-local `["HH:MM", "HH:MM"]` session windows, as in {@link session_slot_options.windows}. */
  windows: readonly (readonly [string, string])[];
  /** Exchange time zone: an IANA name or an explicit schedule (default `"UTC"`). */
  time_zone?: time_zone;
  /**
   * Trading-day start relative to local midnight (default 0). It is this call's own value and is
   * not read from the chart: pass `-10800` for China futures. A Sunday-open market (CME Globex)
   * passes `0`, the evening dates, and `[["17:00", "16:00"]]`; `-25200` with the Monday date places
   * that session on Friday evening and omits the Sunday and Monday rows. `windows` apply to every
   * date of the call, so call once per window set and concatenate the boundaries when dates differ
   * (for example the first trading day after a break has no night window).
   */
  session_start?: number;
  /**
   * `"window"` (default): one boundary per session window, so intraday bars restart at every
   * window open. `"day"`: one boundary per date from its first open to its last close, for daily
   * bars (use an interval of at least that span, e.g. 86 400).
   */
  span?: "window" | "day";
}

/** Options of {@link chart_api.configure_resampled_series}. */
export interface resample_series_options {
  /** Candlestick or bar series whose bar-open-stamped rows are resampled. */
  source: series_api | number;
  /** Histogram of the source's volume, summed per derived bar into `volume_target`. */
  volume_source?: series_api | number;
  /** Histogram that receives the derived volume. */
  volume_target?: series_api | number;
  /** Width of one derived bar in seconds; buckets restart at each boundary. */
  interval_seconds: number;
  /** At most 20 000 ordered, disjoint periods; source rows outside every boundary are omitted. */
  boundaries: readonly resample_boundary[];
}

/** One derived bar of a resampled series; whitespace bars carry `null` prices and volume. */
export interface resampled_bar {
  time: number;
  session_id: number;
  open: number | null;
  high: number | null;
  low: number | null;
  close: number | null;
  volume: number | null;
  /** Source rows aggregated into the bar, whitespace rows included. */
  source_rows: number;
}

/** Lifetime work counters of a resampled series. */
export interface resample_stats {
  rebuilds: number;
  tail_refreshes: number;
  rows_scanned: number;
}

export interface price_scale_options {
  /** 0 normal, 1 logarithmic, 2 percentage, 3 indexed-to-100 (reference values). */
  mode: 0 | 1 | 2 | 3;
  auto_scale: boolean;
  invert_scale: boolean;
  scale_margins: { top: number; bottom: number };
  /** Align price scale labels to prevent them from overlapping (reference `alignLabels`, default `true`). */
  align_labels?: boolean;
  /** Draw small horizontal lines on the price axis labels (reference `ticksVisible`, default `false`). */
  ticks_visible?: boolean;
  /**
   * Show the top and bottom corner labels only when their text is fully visible
   * (reference `entireTextOnly`, default `false`).
   */
  entire_text_only?: boolean;
  /**
   * Minimum width of the price scale in CSS px (reference `minimumWidth`, default 0 = auto). Exceeded
   * when the scale needs more space; useful to align vertically stacked charts' scale widths.
   */
  minimum_width?: number;
  /**
   * Price scale text color (reference `textColor`); when unset, the scale follows `layout.textColor`.
   */
  text_color?: string;
  /**
   * Aeris extension (industry-standard, default true): draw round-figure tick labels in the bold
   * font — multiples of step×10 on uniform ticks, exact powers of ten on log ticks.
   */
  bold_round_labels?: boolean;
  /**
   * Tick mark label density (reference `tickMarkDensity`, default `2.5`): tick spacing in font
   * heights. Higher values produce fewer tick marks.
   */
  tick_mark_density?: number;
  /**
   * Keep a rounded tick mark at the very top and bottom of an autoscaled scale (reference
   * `ensureEdgeTickMarksVisible`, default `false`); adds half a font height of edge padding.
   */
  ensure_edge_tick_marks_visible?: boolean;
  /**
   * Aeris extension: explicit base price for the percentage (`mode: 2`) and indexed-to-100
   * (`mode: 3`) modes, e.g. the previous close. Every series and drawing on this scale converts
   * against it instead of its first visible bar or the comparison anchor, so horizontal panning
   * never re-bases the axis. `null` (default) restores the first-visible base.
   */
  base_value?: number | null;
  /**
   * Aeris extension: center the autoscaled range on this price, `center ± max|price − center|`
   * (e.g. a time-sharing chart centered on the previous close). Works in every mode; in
   * percentage/indexed modes the center converts through the scale's base. Scale margins still
   * apply, so equal top/bottom margins put the center at the pane's middle. `null` (default)
   * autoscales to the plain data range.
   */
  autoscale_center?: number | null;
  /**
   * Aeris extension (default `false`, reference-exact): stable autoscale. The range grows at once
   * when visible data exceeds it but shrinks only when the data one bar beyond the visible edges
   * leaves more than 20% of it unused, so sub-bar pans and kinetic scrolling never flip the range
   * back and forth. Restarts from the exact range on data replacement, series visibility,
   * removal, or scale changes, mode/base/center changes, and autoscale resets.
   */
  stable_auto_scale?: boolean;
  /** Reserve and render this scale's axis strip while retaining its state when hidden. */
  visible?: boolean;
}

export interface price_scale_create_options extends Partial<price_scale_options> {
  /** Pane-local, case-sensitive id (1-128 UTF-8 bytes; `left`, `right`, and `""` are reserved). */
  id: string;
  side: "left" | "right";
  /** Dense position on the side; 0 is nearest the plot. Omitted appends outward. */
  order?: number;
}

export interface price_scale_info {
  id: string;
  side: "left" | "right" | null;
  order: number | null;
  visible: boolean;
  built_in: boolean;
  pane_index: number;
  series_ids: number[];
}

/** The visible raw-value range of a price scale. */
export interface price_range {
  from: number;
  to: number;
}

/** Deeply-partial chart options; forwarded to the engine and deep-merged there (reference semantics). */
export type deep_partial<T> = { [K in keyof T]?: deep_partial<T[K]> };

export interface grid_line_options {
  color: string;
  style: number;
  /** Show the grid lines (Aeris default `true`; the canonical default style is dashed). */
  visible: boolean;
}

/**
 * One crosshair line (reference `CrosshairLineOptions`). `labelVisible`/`labelBackgroundColor` keep
 * the reference's camelCase names, matching the engine's serde keys.
 */
export interface crosshair_line_options {
  color?: string;
  /** Stroke width in CSS px. */
  width?: number;
  /** Line style (`line_style` value; default Dashed). */
  style?: number;
  visible?: boolean;
  /** Display the crosshair label on the relevant scale (reference `labelVisible`, default `true`). */
  labelVisible?: boolean;
  /** Crosshair label background color (reference `labelBackgroundColor`). */
  labelBackgroundColor?: string;
}

/**
 * `crosshair.shadeRight` (Aeris extension): a translucent veil over the pane region to the right
 * of the hovered bar. It starts at the snapped bar's right edge, reaches the pane's right edge, and
 * paints in every stacked pane like the vertical line, independently of `vertLine.visible`. It
 * follows the crosshair gates (hidden mode, and suppression while an interactive object is hovered
 * or dragged). Part of the chart options store, so V2 persistence carries it.
 */
export interface crosshair_shade_options {
  /** Default `false`. */
  visible?: boolean;
  /**
   * CSS color; opacity travels in its alpha. Default `rgba(74, 74, 74, 0.12)` (the crosshair line
   * token at 12% alpha, both themes). An unparsable value falls back to that default.
   */
  color?: string;
}
/** Custom label formatters (reference `localization`). Each receives numbers and returns a string. */
export interface localization_options {
  /**
   * Current locale used to format dates. Uses the browser's language settings by default
   * (reference `localization.locale`, default `navigator.language`).
   */
  locale?: string;
  /**
   * Date formatting string. Can contain `yyyy`, `yy`, `MMMM`, `MMM`, `MM` and `dd` literals
   * which will be replaced with the corresponding date's value. Ignored when `time_formatter`
   * is specified (reference `localization.dateFormat`, default `'dd MMM \'yy'`).
   */
  date_format?: string;
  /** Format any non-percentage price label (axis ticks, last-value badge, crosshair, price lines). */
  price_formatter?: (price: number) => string;
  /**
   * Format a point in time wherever the chart prints one (crosshair, rectangle axis tags, delta
   * tooltip, tooltip, accessibility). Receives the UTC-second timestamp and a context whose
   * `business_day` identifies calendar-date rows.
   */
  time_formatter?: (time: number, context: time_label_context) => string;
}

/** Pan/scroll gesture toggles (reference `handleScroll`). `false` disables all scrolling. */
export interface handle_scroll_options {
  /** Drag inside the pane to pan the time scale. */
  pressed_mouse_move?: boolean;
  /** Horizontal wheel/trackpad scroll pans the time scale (reference `handleScroll.mouseWheel`). */
  mouse_wheel?: boolean;
  /** Horizontal one-finger touch drag pans the time scale. */
  horz_touch_drag?: boolean;
  /** Vertical one-finger touch drag participates in panning. */
  vert_touch_drag?: boolean;
}

/** Zoom/scale gesture toggles (reference `handleScale`). `false` disables all zooming. */
export interface handle_scale_options {
  /** Mouse-wheel zoom on the time scale. Modifiers do not change the default routing. */
  mouse_wheel?: boolean;
  /** Two-finger touch pinch zoom. */
  pinch?: boolean;
  /**
   * Double-clicking a price/time axis resets it (reference `handleScale.axisDoubleClickReset`).
   * `true`/`false` toggles both axes; the object form toggles each independently.
   */
  axis_double_click_reset?: boolean | { time?: boolean; price?: boolean };
  /**
   * Press-and-drag on an axis strip scales it (reference `axisPressedMouseMove`): vertical drag on a
   * price axis scales that price scale (disabling autoscale), horizontal drag on the time axis
   * scales bar spacing. `true`/`false` toggles both; the object form toggles each independently.
   */
  axis_pressed_mouse_move?: boolean | { time?: boolean; price?: boolean };
}

/** Momentum ("kinetic") scroll after a pan flick (reference `kineticScroll`). */
export interface kinetic_scroll_options {
  /** Coast after a one-finger touch flick. Default `true`. */
  touch?: boolean;
  /** Coast after a mouse-drag flick. Default `false`. */
  mouse?: boolean;
}

/**
 * Chart-level cosmetics of one visible price axis (reference `leftPriceScale`/`rightPriceScale`).
 * Keys keep the reference's camelCase names, matching the engine's serde keys; the engine routes them to
 * the corresponding scale.
 */
export interface chart_price_scale_options {
  visible: boolean;
  borderVisible: boolean;
  borderColor: string;
  /** Align price scale labels to prevent them from overlapping (reference `alignLabels`, default `true`). */
  alignLabels?: boolean;
  /** Draw small horizontal lines on the price axis labels (reference `ticksVisible`, default `false`). */
  ticksVisible?: boolean;
  /**
   * Show the top and bottom corner labels only when their text is fully visible
   * (reference `entireTextOnly`, default `false`).
   */
  entireTextOnly?: boolean;
  /**
   * Minimum width of the price scale in CSS px (reference `minimumWidth`, default 0 = auto). Exceeded
   * when the scale needs more space; useful to align vertically stacked charts' scale widths.
   */
  minimumWidth?: number;
  /** Price scale text color (reference `textColor`); when unset, the scale follows `layout.textColor`. */
  textColor?: string;
  /** Bold round-figure tick labels (Aeris extension, industry-standard, default `true`). */
  boldRoundLabels?: boolean;
  /** Tick mark label density in font heights (reference `tickMarkDensity`, default `2.5`). */
  tickMarkDensity?: number;
  /** Rounded tick marks at both edges while autoscaled (reference `ensureEdgeTickMarksVisible`, default `false`). */
  ensureEdgeTickMarksVisible?: boolean;
}

/** Crosshair "tracking mode" behavior on touch (reference `trackingMode`). Package-level. */
export interface tracking_mode_options {
  /**
   * How tracking mode exits (reference `trackingMode.exitMode`). `"on_touch_end"` (reference
   * `TrackingModeExitMode.OnTouchEnd`) clears the crosshair when the finger lifts;
   * `"on_next_tap"` (default, reference `TrackingModeExitMode.OnNextTap`) keeps it until the next
   * tap ends.
   */
  exit_mode?: "on_next_tap" | "on_touch_end";
}

export interface chart_options {
  /**
   * Domain of the canonical first pane. Omit for the compatible financial-time chart with its
   * primary candlestick series. A general domain creates one preserved pane with no hidden
   * financial series or transient pane removal.
   */
  initialPane: initial_pane_options;
  layout: {
    background: { type: string; color: string };
    textColor: string;
    /** Secondary unboxed chart text. Boxed live labels derive black/white text from their fill. */
    mutedTextColor: string;
    /** Theme fallback for candlestick and bar up geometry when the series color is unpinned. */
    bullishColor: string;
    /** Theme fallback for candlestick and bar down geometry when the series color is unpinned. */
    bearishColor: string;
    fontSize: number;
    fontFamily: string;
    panes: {
      separatorColor: string;
      /** Hover band for an interactive pane separator. */
      separatorHoverColor: string;
      /**
       * Allow dragging pane separators to resize panes (reference `layout.panes.enableResize`,
       * default `true`). Package-level: drives the separator drag and its hover cursor; it is
       * stripped before options reach the engine.
       */
      enableResize: boolean;
    };
  };
  grid: { vertLines: grid_line_options; horzLines: grid_line_options };
  crosshair: {
    vertLine: crosshair_line_options;
    horzLine: crosshair_line_options;
    /** Veil right of the hovered bar; see `crosshair_shade_options`. */
    shadeRight: crosshair_shade_options;
    mode: number;
  };
  leftPriceScale: chart_price_scale_options;
  rightPriceScale: chart_price_scale_options;
  /**
   * Time-axis strip cosmetics (reference `timeScale.borderVisible`/`borderColor`) plus the
   * declarative exchange time zone, trading-day start, explicit marks, and bar time label (same
   * semantics as {@link time_scale_options.time_zone} / {@link time_scale_options.session_start} /
   * {@link time_scale_options.tick_marks} / {@link time_scale_options.bar_time_label}); these
   * also work for worker charts.
   */
  timeScale: {
    borderVisible: boolean;
    borderColor: string;
    timeZone?: time_zone;
    sessionStart?: number;
    /** Declarative {@link time_scale_options.tick_marks} (also for worker charts). */
    tickMarks?: readonly time_tick_mark[] | null;
    /** Declarative {@link time_scale_options.bar_time_label} (also for worker charts). */
    barTimeLabel?: bar_time_label;
  };
  /**
   * Large text label painted inside the pane (reference v4 `watermark`). `color` is any CSS color
   * (include alpha for a faint mark; the default is fully transparent). Aeris draws it on the shared
   * overlay above the series — a deliberate divergence needed to stay pixel-identical across the
   * WebGPU and Canvas2D backends.
   */
  watermark: {
    visible: boolean;
    text: string;
    color: string;
    fontSize: number;
    fontFamily: string;
    fontStyle: string;
    horzAlign: "left" | "center" | "right";
    vertAlign: "top" | "center" | "bottom";
  };
  /** Install a ResizeObserver so the chart tracks its container's size. Default `false` (reference parity). */
  autoSize: boolean;
  /**
   * Temporarily promote hovered chart content above idle (default `true`). When on, hovering a
   * series promotes its whole indicator group (all Bollinger/ribbon outputs together, internal
   * order kept) and hovering a drawing promotes it above ordinary price; dragging/editing tops
   * hovered tops selected. Idle default paints indicators below idle drawings below ordinary
   * price; explicit `set_series_order` overrides idle series grouping. Stable saved order and
   * hit-test ties never change, so promotion cannot oscillate hover.
   */
  hoveredSeriesOnTop: boolean;
  /** Custom label formatters (reference `localization`). Package-level; carries JS callbacks. */
  localization: localization_options;
  /** Enable/disable panning gestures (reference `handleScroll`). Default `true`. Package-level. */
  handle_scroll: boolean | handle_scroll_options;
  /** Enable/disable zoom gestures (reference `handleScale`). Default `true`. Package-level. */
  handle_scale: boolean | handle_scale_options;
  /** Momentum scroll after a pan flick (reference `kineticScroll`). Default touch-only. Package-level. */
  kinetic_scroll: boolean | kinetic_scroll_options;
  /**
   * Wheel/trackpad policy. `auto` uses behavior measured from the pinned public reference fixture:
   * vertical deltas zoom time
   * and horizontal deltas pan time independently on every chart surface, without modifier
   * routing. `pan` and `zoom` are explicit Aeris extensions.
   */
  wheel_behavior: "auto" | "pan" | "zoom";
  /** Zoom the hovered price axis in auto wheel mode. Default `false`. Package-level. */
  price_axis_wheel_zoom: boolean;
  /** Chart-owned keyboard and assistive-technology surface. Enabled by default. */
  accessibility: boolean | accessibility_options;
  /** Touch crosshair tracking-mode behavior (reference `trackingMode`). Package-level. */
  tracking_mode: tracking_mode_options;
  /** Backend override for capability testing; defaults to automatic WebGPU → Canvas2D fallback. */
  backend: "auto" | "canvas2d";
  /**
   * Style preset from `theme.ts` (the package's style settings file). At creation it is applied
   * under explicit options; later `apply_options({ theme })` switches the selected preset live.
   * The selected identity is retained so {@link chart_api.reset_style_to_defaults} can restore the
   * correct light/dark canonical defaults after further customization. Package-level only — the
   * raw `theme` key is never forwarded to the engine.
   */
  theme: "light" | "dark";
}

/** Direction rule of a `histogram_updown` volume histogram. */
export type histogram_updown_rule = "open_close" | "previous_close";

/**
 * How a baseline series without a pinned `baseline_value` resolves its baseline price (see
 * `series_options.baseline_mode`).
 */
export type baseline_mode = "visible_midpoint" | "close_before_visible_range";

/** How a series' timestamps land on the shared time axis (see `series_options.time_alignment`). */
export type time_alignment = "union" | "as_of";

/** Options accepted when adding a series. */
export interface series_options {
  /**
   * Retention ceiling: the series holds at most this many data points, evicting the **oldest**
   * first. Omit (or pass `0`) for the default, which is **unbounded** — a series grows for as long
   * as the host appends to it, and wasm linear memory never shrinks, so an open-ended live session
   * with no cap grows monotonically. Set this to make memory flat over a long session; watch it
   * with {@link frame_stats.memory_bytes}.
   *
   * `max_points` is a hard ceiling — the series never holds more. Eviction is amortized rather
   * than per-point, because trimming rebuilds the shared time axis: once the count exceeds
   * `max_points` the engine trims back to `max_points - max_points / 32`, so the count sits in
   * `[max_points - max_points / 32, max_points]` and the cost per appended point stays constant.
   * `data()` reflects the retained rows, so it can return slightly fewer than `max_points`.
   *
   * Eviction is per series, and only affects the shared time axis where the evicted timestamps
   * were not also held by another series. Applying a cap trims the series' existing points
   * immediately; a full `set_data`/`set_data_typed` install is trimmed to the ceiling too.
   *
   * Under a replay clock the ceiling counts and evicts only the points up to the clock (what a
   * clean load to that clock holds), and a seek that reveals points trims them the same way.
   * Points ingested past the clock are the replay's pending data: they stay, uncounted, until the
   * clock reveals them.
   *
   * Note this is a *point* count, not a time window: the retained span depends on the bar interval.
   */
  max_points: number;
  /**
   * UTC-seconds time from which rows keep their data but are not drawn, so another
   * presentation (for example a live footprint) can take over the series' tail. `null` draws
   * every row.
   */
  render_before_time: number | null;
  /**
   * How this series' timestamps land on the shared time axis. `"union"` (the default,
   * reference behavior) adds every timestamp to the chart's time points. `"as_of"` is for an
   * overlay from another market calendar (an index over a stock from another exchange, crypto
   * over equities, a futures night session over its underlying): the series adds no time point,
   * so the other series keep a gapless axis, and each point up to the last real bar of the
   * other series shows this series' last row at or before that point's time. Rows between two
   * points collapse into the later one; a point with no newer row repeats the previous row (see
   * `as_of_max_staleness`); rows after the last point wait for it. Studies bound to the series
   * compute on its own rows and follow the same points, `data()` keeps its own rows, and value
   * snapshots report the point's time. Line, area, baseline, histogram, bar, and candlestick
   * series that own their rows, on a time axis, only; others (and any series on a non-time bar
   * axis) throw `unsupported_operation`. Worker charts take it in `add_series` options and change
   * it with `offscreen_chart.apply_series_options`.
   */
  time_alignment: time_alignment;
  /**
   * With `time_alignment: "as_of"`, leave a point empty instead of repeating a row older than
   * this many seconds (a non-negative whole number). `null` (the default) repeats without limit;
   * `0` shows only rows exactly at a point's time.
   */
  as_of_max_staleness: number | null;
  /** Overrides the kind default color (line/area/histogram). */
  color: string;
  /**
   * Candlestick/bar up (close ≥ open) body color. Any CSS color the engine parses. On a
   * `histogram_updown` histogram it is the up-column tint instead of the translucent market up
   * color (e.g. red for the A-share red-up convention).
   */
  up_color: string;
  /** Candlestick/bar down (close < open) body color; the down-column tint of a `histogram_updown` histogram. */
  down_color: string;
  /** Render candlesticks from a bounded Heikin Ashi presentation projection while keeping raw OHLC in data(). */
  heikin_ashi: boolean;
  /** Candlestick up-bar wick color. Until set, follows `up_color` (reference parity). Pass `""` to
   *  clear a previously-pinned color and go back to following the body color. */
  wick_up_color: string;
  /** Candlestick down-bar wick color. Until set, follows `down_color`. `""` clears the override. */
  wick_down_color: string;
  /** Candlestick up-bar border color. Until set, follows `up_color` (reference parity). `""` clears it. */
  border_up_color: string;
  /** Candlestick down-bar border color. Until set, follows `down_color`. `""` clears the override. */
  border_down_color: string;
  /** Candlestick wick visibility (default true; ignored by bar series). */
  wick_visible: boolean;
  /** Candlestick body-border visibility (default true; ignored by bar series). */
  border_visible: boolean;
  /** Line/area stroke width in CSS px (default 2; EMA-family indicators default to 1). */
  line_width: number;
  /** Area fill color at the line (top of the gradient). */
  area_top_color: string;
  /** Area fill color at the base (bottom of the gradient; usually fully transparent). */
  area_bottom_color: string;
  /**
   * Histogram only: color each bar by the main price series' up/down direction at that time
   * (translucent green/red), matching industry-standard volume. Default false (solid `color`).
   */
  histogram_updown: boolean;
  /**
   * How `histogram_updown` decides a column's direction from the primary price series (the first
   * series added): `"open_close"` (default) compares that bar's close with its open;
   * `"previous_close"` compares it with the previous traded close (the A-share/HK time-sharing
   * (分时) volume convention; an unchanged close counts as up). The first traded bar compares
   * with the primary's previous-close reference: a baseline series' `baseline_value`, else its
   * price scale's `base_value`, else its own open. Whitespace bars keep the solid `color`.
   */
  histogram_updown_rule?: histogram_updown_rule;
  /**
   * Place the series on the bottom-band overlay price scale (volume-style): its magnitude is
   * excluded from the main price axis autoscale. Mirrors the reference's `priceScaleId: ''` + scaleMargins.
   */
  overlay: boolean;
  /** Pane-local price-scale id; `left`/`right` are built-ins and `""` is the overlay scale. */
  price_scale_id: string;
  /** Camel-case reference alias of `price_scale_id`. */
  priceScaleId: string;
  /** Overlay band as fractions of pane height (default `{ top: 0.8, bottom: 0 }` ⇒ bottom fifth). */
  scale_margins: { top: number; bottom: number };
  /** Stacked pane index (0 = top/price pane). A new pane is created on demand (roadmap Phase B1). */
  pane: number;
  /** Relative height of a newly-created pane (default 1; the price pane is 3). */
  pane_stretch: number;
  /** Line/area join type (roadmap Phase B3). */
  line_type: "simple" | "stepped" | "curved";
  /** Draw a disc at each data point (shown when bars are spaced enough), roadmap Phase B3. */
  point_markers: boolean;
  /** Baseline price for a baseline series (omit for auto, resolved per `baseline_mode`). */
  baseline_value: number;
  /**
   * Baseline: how the baseline price resolves while `baseline_value` is unset (default
   * `"visible_midpoint"`, the midpoint of the visible closes). `"close_before_visible_range"`
   * uses the last finite close before the first visible bar, so the view reads as change against
   * where it started; when nothing precedes the window the first visible close stands in (the
   * first bar reads as unchanged). Both values follow the visible window and change while
   * scrolling; a host that knows the true prior-session close pins `baseline_value` instead.
   * Semantic state: survives `reset_style_to_defaults()`. Does not change the
   * `histogram_updown_rule: "previous_close"` first-bar reference, which stays pinned-only.
   */
  baseline_mode?: baseline_mode;
  /**
   * Baseline: draw the resolved baseline price as a full-pane-width horizontal reference line
   * between the quadrant fills and the quadrant strokes (default `false`).
   */
  baseline_line_visible?: boolean;
  /**
   * Baseline: reference line color (CSS). `""` or omitted follows the neutral chrome tint
   * `#4a4a4a` (the crosshair line token, identical in both themes).
   */
  baseline_line_color?: string;
  /** Baseline: reference line width in CSS px (default `1`; positive). */
  baseline_line_width?: number;
  /** Baseline: reference line style, a `LINE_STYLE_TO_U8` value (default `2`, dashed). */
  baseline_line_style?: number;
  /**
   * Pulse an expanding ring at the last value (drives an rAF loop while visible). Default `true`
   * for line and area series and `false` for every other type; set `false` to disable. A value
   * equal to the current type's default keeps following the default when the series type
   * changes; a value that differs from it (an opt-out on a line, an opt-in on candles) is kept.
   */
  last_price_animation: boolean;
  /**
   * Live-bar easing time constant in milliseconds (default `0` = off; values above `1000` clamp).
   * When a `series.update()` / `merge()` replaces the drawn last bar in place (same time), the
   * displayed high, low and close glide toward the new values with
   * `x += (target - x) * (1 - exp(-dt / tau))`; the open never eases. A brand-new bar, a reinstall
   * and `reduced_motion` snap. Only the drawn geometry, its last-value line and axis chip, the
   * pulse and the crosshair marker on that bar follow the glide: `data()`, `options()`, the
   * value snapshot (legend, tooltip, data window), the magnet, autoscale and `baseline_price()`
   * read the real values throughout. Negative or non-finite values throw `invalid_options`.
   * Style class like `last_price_animation`: reset by `reset_style_to_defaults()`, not persisted,
   * not available on worker (offscreen) charts.
   */
  live_bar_easing_ms?: number;
  /** Keep the series in the engine while toggling its visibility. */
  visible: boolean;
  /** Show the last-value badge on the price scale (reference `lastValueVisible`, default `true`). */
  last_value_visible?: boolean;
  /**
   * reference `title` (default `""`): the series' display name, shown as a chip in a darker
   * shade of the label color at the front of the last-value label cluster when `title_visible`
   * holds (industry-standard).
   */
  title?: string;
  /** Show the `title` chip in the last-value cluster (industry-standard; default `true`). */
  title_visible?: boolean;
  /**
   * Stack a candle-close countdown row below the price inside the last-value cluster. The
   * canonical primary market series defaults to `true`; subsequently added series default to
   * `false` and must opt in when they own a market interval. The package ticks a 1s timer while
   * any visible series with data has this on.
   */
  countdown_visible?: boolean;
  /** Show the series price line at the last value (reference `priceLineVisible`, default `true`). */
  price_line_visible?: boolean;
  /** Value the price line tracks (reference `PriceLineSource`): 0 LastBar (default), 1 LastVisible. */
  price_line_source?: 0 | 1;
  /**
   * Built-in live-price line extent. `"partial"` (default) draws from the tracked bar/value to the
   * pane's right edge; `"full"` draws the conventional full-pane horizontal line.
   */
  price_line_extent?: "partial" | "full";
  /** Price line width in CSS px (reference `priceLineWidth`, default 1). */
  price_line_width?: number;
  /**
   * Built-in live-line and complete last-value-cluster color (reference `priceLineColor`);
   * default `""` follows the resolved series/bar/point color.
   */
  price_line_color?: string;
  /** Price line style, a `LINE_STYLE_TO_U8` value (reference `priceLineStyle`, default 1 Dotted). */
  price_line_style?: number;
  /**
   * industry-standard bid/ask lines + "Bid"/"Ask" axis chips (default `false` — platforms opt
   * in). Push the live quotes with {@link series_api.set_bid_ask}; each side with a value
   * draws a line across the pane and a title chip on the scale.
   */
  bid_ask_visible?: boolean;
  /** Bid line/chip color (default: the theme's primary token). */
  bid_color?: string;
  /** Ask line/chip color (default: the market-loss token). */
  ask_color?: string;
  /** Bid/ask line width in CSS px (default 1, mirrors `price_line_width`). */
  bid_ask_line_width?: number;
  /** Bid/ask line style, a `LINE_STYLE_TO_U8` value (default 1 Dotted, mirrors `price_line_style`). */
  bid_ask_line_style?: number;
  /** Line stroke style 0-4, a `LINE_STYLE_TO_U8` value (reference `lineStyle`, default 0 Solid). */
  line_style?: number;
  /** Draw the line itself on line/area/baseline series (reference `lineVisible`, default `true`). */
  line_visible?: boolean;
  /** Point-marker disc radius in CSS px (reference `pointMarkersRadius`); unset = auto. */
  point_markers_radius?: number;
  /** Show the crosshair marker on this series or indicator output (Aeris default `false`). */
  crosshair_marker_visible?: boolean;
  /** Crosshair marker radius in CSS px (reference `crosshairMarkerRadius`, default 4). */
  crosshair_marker_radius?: number;
  /** Crosshair marker border color; default `""` follows the chart background. */
  crosshair_marker_border_color?: string;
  /** Crosshair marker fill color; default `""` follows the series value color. */
  crosshair_marker_background_color?: string;
  /** Crosshair marker border width in CSS px (reference `crosshairMarkerBorderWidth`, default 2). */
  crosshair_marker_border_width?: number;
  /** Baseline: first gradient fill color above the baseline (reference `topFillColor1`). */
  top_fill_color1?: string;
  /** Baseline: second gradient fill color above the baseline (reference `topFillColor2`). */
  top_fill_color2?: string;
  /** Baseline: line color above the baseline (reference `topLineColor`). */
  top_line_color?: string;
  /** Baseline: line width above the baseline in CSS px (reference `topLineWidth`). */
  top_line_width?: number;
  /** Baseline: line style above the baseline, a `LINE_STYLE_TO_U8` value (reference `topLineStyle`). */
  top_line_style?: number;
  /** Baseline: first gradient fill color below the baseline (reference `bottomFillColor1`). */
  bottom_fill_color1?: string;
  /** Baseline: second gradient fill color below the baseline (reference `bottomFillColor2`). */
  bottom_fill_color2?: string;
  /** Baseline: line color below the baseline (reference `bottomLineColor`). */
  bottom_line_color?: string;
  /** Baseline: line width below the baseline in CSS px (reference `bottomLineWidth`). */
  bottom_line_width?: number;
  /** Baseline: line style below the baseline, a `LINE_STYLE_TO_U8` value (reference `bottomLineStyle`). */
  bottom_line_style?: number;
  /** Histogram base value the bars grow from (reference `base`, default 0); autoscale always includes it. */
  base?: number;
  /** Area: invert the filled area (fill above the line) (reference `invertFilledArea`, default `false`). */
  invert_filled_area?: boolean;
  /**
   * Line/area/baseline: end the line at each exchange trading-day boundary (default `false`).
   * The first drawn row of a trading day starts a new run with no connecting segment, fill, or
   * hit area from the previous day; trading days follow the time scale's `time_zone` and
   * `session_start`. A run of one row draws a one-bar horizontal segment. Multi-day intraday
   * (分时) charts use it to separate days. VWAP, VWAP bands, and pivot outputs always break at
   * their own reset periods.
   */
  break_on_trading_day?: boolean;
  /** Bar: draw the open tick on each bar (reference `openVisible`, default `true`). */
  open_visible?: boolean;
  /** Bar: draw the close tick on each bar (default `true`). Set false with `open_visible` false for high-low bars. */
  close_visible?: boolean;
  /** Bar: draw thin bars when the bar spacing is small (reference `thinBars`, default `true`). */
  thin_bars?: boolean;
  /**
   * Per-series price formatting (reference `priceFormat`). Built-in types (`"price"`/`"volume"`/
   * `"percent"`, reference `PriceFormatBuiltIn`) take `precision` and `min_move` (reference
   * `precision`/`minMove`, snake_case per the package API convention); `"custom"` (reference
   * `PriceFormatCustom`) installs a JS formatter callback with an optional `min_move`.
   *
   * Axis ticks always lie on the `min_move` grid (a 0.02 tick never labels 15.25). When
   * `min_move` is given without `precision`, the precision derives from it (reference
   * `precisionByMinMove`): `{ type: "price", min_move: 0.0001 }` prints 4 decimals.
   *
   * `tick_ladder` (Aeris extension, `"price"` type) installs an exchange spread table: each label
   * rounds to its own band tick with that band's precision, axis ticks lie on the common grid of
   * the visible bands, and trading order snapping on this series' scale uses the same bands.
   * `null` clears it. A malformed ladder throws `invalid_options` and leaves the format unchanged.
   */
  price_format?:
    | {
        type: "price" | "volume" | "percent";
        precision?: number;
        min_move?: number;
        tick_ladder?: readonly price_tick_band[] | null;
      }
    | { type: "custom"; formatter: (price: number) => string; min_move?: number };
  /**
   * Replace this series' autoscale contribution (reference `autoscaleInfoProvider`). Called
   * during every autoscale pass with `base_implementation`, which returns the series' own info for
   * the visible bars; the returned info REPLACES it (`null` removes the series from autoscale).
   * The provider runs while the chart renders: a chart API called from inside it throws
   * `unsupported_operation` without touching the chart, and a provider that throws is ignored for
   * that pass (the series keeps its own info). Read chart state before rendering and close over
   * it instead. `null` clears it.
   */
  autoscale_info_provider?: autoscale_info_provider | null;
}

/** One band of a price-format tick-size ladder (exchange spread table). */
export interface price_tick_band {
  /** Inclusive lower bound of the band's absolute price; the first band also covers lower prices. */
  from: number;
  /** Tick size inside the band. Band bounds must lie on the grids of both adjacent bands. */
  min_move: number;
  /** Label decimals inside the band; derived from `min_move` when omitted. */
  precision?: number;
}

/** reference `AutoscaleInfo`: a series' autoscale range in prices plus optional pixel margins. */
export interface autoscale_info {
  price_range: { min_value: number; max_value: number } | null;
  margins?: { above: number; below: number };
}

/** reference `AutoscaleInfoProvider`: receives the default implementation, returns the replacement. */
export type autoscale_info_provider = (
  base_implementation: () => autoscale_info | null,
) => autoscale_info | null;

export interface feature_brush_style {
  line_color: string;
  top_color: string;
  bottom_color: string;
  line_width: number;
}

export interface feature_brush_range {
  /** Logical range; `from` is inclusive and `to` is exclusive. */
  range: logical_range;
  style: feature_brush_style;
}

/** Rust-engine options shared by the advanced financial series. Irrelevant keys are ignored. */
export interface feature_series_options {
  colors: readonly string[] | readonly { line: string; area: string }[];
  line_color: string;
  top_color: string;
  bottom_color: string;
  base_price: number;
  /**
   * @deprecated Brush ranges are transient presentation state owned by
   * `enable_brushable_area_interaction()` on an ordinary Area series.
   */
  brush_ranges: readonly feature_brush_range[];
  cell_border_width: number;
  cell_border_color: string;
  /** Official Heat Map `cellShader`; evaluated at the host boundary and retained as engine color. */
  cell_shader: heatmap_cell_shader;
  high_line_color: string;
  low_line_color: string;
  close_line_color: string;
  high_line_width: number;
  low_line_width: number;
  close_line_width: number;
  width_percent: number;
  radius: number;
  low_color: string;
  high_color: string;
  low_value: number;
  high_value: number;
  opacity: number;
  whisker_color: string;
  lower_quartile_fill: string;
  upper_quartile_fill: string;
  outlier_color: string;
}

export interface footprint_series_options {
  /** Exact exchange price increment. Off-grid trades are rejected. */
  tick_size: number;
  /** Adjacent ticks aggregated into one footprint row. Defaults to 1 (one row per tick). */
  ticks_per_row?: number;
  /** Bar construction policy shared by footprint and its chart-level trade stream. */
  bar_type: "time" | "trades" | "volume" | "range";
  /** Whole-second aligned time-bar period when bar_type is time. */
  interval_seconds: number;
  anchor_seconds: number;
  /** Number of trades per bar when bar_type is trades. */
  trades_per_bar: number;
  /** Total volume per bar when bar_type is volume. */
  volume_per_bar: number;
  /** Tick span when bar_type is range. */
  range_ticks: number;
  imbalance_ratio: number;
  imbalance_minimum_volume: number;
  stacked_imbalance_levels: number;
  cell_mode: "bid_ask" | "total" | "delta" | "profile_in_bar" | "volume_ladder" | "horizontal_imbalance" | "bid_ask_histogram";
  font_size: number;
  bid_color: string;
  ask_color: string;
  positive_delta_color: string;
  negative_delta_color: string;
  /** `null` follows the live chart layout foreground. */
  text_color: string | null;
  poc_color: string;
  stacked_bid_color: string;
  stacked_ask_color: string;
  show_bar_summary: boolean;
}

/** Engine-owned price-action transform applied to canonical host OHLC source bars. */
export type synthetic_bar_options =
  | { kind: "renko_fixed"; box_size: number }
  | { kind: "renko_atr"; period: number }
  | { kind: "line_break"; lines: number }
  | { kind: "kagi"; reversal_size: number }
  | { kind: "point_and_figure"; box_size: number; reversal_boxes: number };

export type any_series_options = series_options & Partial<feature_series_options> & Partial<footprint_series_options>;

export const LINE_TYPE_TO_U8: Record<NonNullable<series_options["line_type"]>, number> = {
  simple: 0,
  stepped: 1,
  curved: 2,
};

/** Style of a price line / crosshair line. */
export type line_style = "solid" | "dotted" | "dashed" | "large_dashed" | "sparse_dotted";
export const LINE_STYLE_TO_U8: Record<line_style, number> = {
  solid: 0,
  dotted: 1,
  dashed: 2,
  large_dashed: 3,
  sparse_dotted: 4,
};

/** Options for {@link series_api.create_price_line}. */
export interface price_line_options {
  price: number;
  color?: string;
  line_width?: number;
  line_style?: line_style;
  /** Axis label text; defaults to the formatted price. */
  title?: string;
  /** Draw the line itself (reference `lineVisible`, default `true`). */
  line_visible?: boolean;
  /** Show the price label on the axis (reference `axisLabelVisible`, default `true`). */
  axis_label_visible?: boolean;
  /** Axis label background color; defaults to the line color (reference `axisLabelColor`). */
  axis_label_color?: string;
  /** Axis label text color (reference `axisLabelTextColor`). */
  axis_label_text_color?: string;
}

/** A handle to a created price line. */
export interface price_line_api {
  remove(): void;
  /** Deep-merge a patch onto this line's options (reference `IPriceLine.applyOptions`). */
  apply_options(options: Partial<price_line_options>): void;
  /** The current (deep-merged) options of this price line (reference `IPriceLine.options`). */
  options(): price_line_options;
  readonly id: number;
}

/** A per-bar marker on a series (roadmap Phase B4), informed by common public chart APIs. */
export interface series_marker {
  /** Bar time (must match a data point's time). Accepts the same forms as data `time`. */
  time: time;
  /** Placement relative to the bar. reference names are canonical; short aliases remain compatible. */
  position?: "aboveBar" | "belowBar" | "inBar" | "atPriceTop" | "atPriceBottom" | "atPriceMiddle" | "above" | "below";
  /** Marker shape. Default `"circle"`. */
  shape?: "circle" | "square" | "arrowUp" | "arrowDown";
  /** Fill color (any CSS color the engine parses). Default series color. */
  color?: string;
  /** Optional label rendered beside the marker. */
  text?: string;
  /** Optional object ID reported by marker hit testing. */
  id?: string;
  /** Marker-size multiplier. Default `1`; negative values clamp to zero like the reference. */
  size?: number;
  /** Exact price, required by `atPriceTop`, `atPriceBottom`, and `atPriceMiddle`. */
  price?: number;
}

export interface series_marker_options {
  /** Expand price-scale pixel margins so marker shapes remain visible. Default `true` (reference). */
  auto_scale: boolean;
  /** Official marker stacking order. Default `normal`. */
  z_order: "normal" | "aboveSeries" | "top";
}

export const KIND_TO_U8: Record<series_kind, number> = {
  candlestick: 0,
  bar: 1,
  line: 2,
  area: 3,
  histogram: 4,
  baseline: 5,
  footprint: 8,
  // Compatibility alias only: brushable area is an ordinary Area series in the engine.
  brushable_area: 3,
  grouped_bars: 7,
  heatmap: 7,
  hlc_area: 7,
  pretty_histogram: 7,
  background_shade: 7,
  stacked_area: 7,
  stacked_bars: 7,
  whisker_box: 7,
  custom: 6,
};

export const FEATURE_KIND_TO_U8: Record<feature_series_kind, number> = {
  grouped_bars: 2,
  heatmap: 3,
  hlc_area: 4,
  pretty_histogram: 5,
  background_shade: 7,
  stacked_area: 8,
  stacked_bars: 9,
  whisker_box: 10,
};

export function is_feature_series_kind(kind: series_kind): kind is feature_series_kind {
  return kind !== "custom" && KIND_TO_U8[kind] === 7;
}

export function is_footprint_series_kind(kind: series_kind): kind is "footprint" {
  return kind === "footprint";
}

// ---------------------------------------------------------------------------------------------
// Drawing tools (engine-owned drawing objects; aeris_charts_engine drawings.rs)
// ---------------------------------------------------------------------------------------------

/**
 * The drawing-tool kinds. Each tool is an engine-owned drawing object with defining anchor
 * points: trend line (2), rectangle (2), Long Position / Short Position tools (3: entry, target,
 * stop), horizontal line/ray, vertical line, and text (1 each), a multi-click arrow-ended
 * straight-segment path (variable length, every vertex editable), the freehand brush (a
 * variable-length curve, anchor handles at the two ends), and the price range, date range, and
 * date-and-price range measuring tools (2: start, end; the measured sign follows start → end).
 * The measuring tools snap both anchors to whole bars and price ticks. Holding Shift while
 * clicking an empty pane starts a transient date-and-price range (the quick measure; it is never a
 * drawing, history entry, or persisted object).
 *
 * Lines family (B8): `ray`, `extended_line`, `info_line`, `trend_angle`, and `arrow_line` place
 * two anchors; `extend_left` extends beyond the first anchor and `extend_right` beyond the second
 * (a ray defaults to `extend_right`, an extended line to both). `cross_line` places one anchor.
 * `horizontal_segment` keeps both anchors on one price, and `vertical_ray` and `vertical_segment`
 * keep both on one bar (the shared coordinate follows the anchor placed or dragged last); the
 * vertical ray defaults to `extend_right`, which runs it through its second anchor to the pane edge.
 * `price_line` places one anchor: a line from it to the right pane edge with its price printed on
 * the line and on the price axis.
 *
 * Channels family (B8): `price_channel` places three anchors: the base line through the first two,
 * its parallel through the third, and the base line mirrored on the other side. It defaults to
 * extending both ways, with no fill.
 */
export type drawing_kind =
  | "trend_line"
  | "horizontal_line"
  | "horizontal_ray"
  | "vertical_line"
  | "rectangle"
  | "text"
  | "brush"
  | "path"
  | "long_position"
  | "short_position"
  // B8: lines — begin
  | "ray"
  | "extended_line"
  | "info_line"
  | "trend_angle"
  | "cross_line"
  | "arrow_line"
  | "horizontal_segment"
  | "vertical_ray"
  | "vertical_segment"
  | "price_line"
  // B8: lines — end
  // B8: channels — begin
  | "parallel_channel"
  | "regression_trend"
  | "flat_top_bottom"
  | "disjoint_channel"
  | "price_channel"
  // B8: channels — end
  // B8: fibonacci — begin
  // Fibonacci family: two anchors (retracement, time zone, speed resistance fan and arcs,
  // circles, spiral) or three (trend-based extension and time, channel, wedge); levels come
  // from `levels`, options from `tool_options.fibonacci`.
  | "fib_retracement"
  | "trend_based_fib_extension"
  | "fib_channel"
  | "fib_time_zone"
  | "trend_based_fib_time"
  | "fib_speed_resistance_fan"
  | "fib_speed_resistance_arcs"
  | "fib_circles"
  | "fib_spiral"
  | "fib_wedge"
  // B8: fibonacci — end
  // B8: pitchforks_gann — begin
  | "andrews_pitchfork"
  | "schiff_pitchfork"
  | "modified_schiff_pitchfork"
  | "inside_pitchfork"
  | "pitchfan"
  | "gann_box"
  | "gann_square"
  | "gann_square_fixed"
  | "gann_fan"
  // B8: pitchforks_gann — end
  // B8: projection_annotations — begin
  // Projection & Annotations: `projection` places three anchors; `forecast`, `bars_pattern`,
  // the three ranges, `price_note`, and `callout` two; the other annotations one.
  // `simple_tag` is a dashed line across the pane whose price-axis tag shows the drawing's `text`
  // (the price when it has none); `simple_annotation` is a dashed stem with a head under boxed text.
  // `anchored_text` anchors are pane fractions (`logical` = x / pane width, `price` = y / pane
  // height) and carry no `time`.
  | "forecast"
  | "bars_pattern"
  | "price_range"
  | "date_range"
  | "date_and_price_range"
  | "projection"
  | "anchored_text"
  | "note"
  | "price_note"
  | "callout"
  | "comment"
  | "price_label"
  | "signpost"
  | "flag_mark"
  | "arrow_mark_up"
  | "arrow_mark_down"
  | "arrow_mark_left"
  | "arrow_mark_right"
  | "icon"
  | "simple_tag"
  | "simple_annotation"
  // B8: projection_annotations — end
  // B8: patterns_elliott_cycles — begin
  // Patterns (boxed point labels; XABCD, cypher, ABCD, and three drives add ratio connectors),
  // Elliott waves (degree-notation labels), and cycles (repeats across the pane).
  | "xabcd_pattern"
  | "cypher_pattern"
  | "abcd_pattern"
  | "head_and_shoulders"
  | "triangle_pattern"
  | "three_drives_pattern"
  | "elliott_impulse_wave"
  | "elliott_correction_wave"
  | "elliott_triangle_wave"
  | "elliott_double_combo"
  | "elliott_triple_combo"
  | "cyclic_lines"
  | "time_cycles"
  | "sine_line"
  // B8: patterns_elliott_cycles — end
  // B8: shapes — begin
  // Shapes family: `rotated_rectangle` (axis ends + a point on a long side), `ellipse` (box
  // corners), `circle` (center + rim), `triangle`, `arc` (start, end, a point on the arc),
  // `curve` (start, end, the curve's midpoint), `double_curve` (start, end, the points at one and
  // two thirds), multi-click `polyline`, and the freehand `highlighter`.
  | "rotated_rectangle"
  | "ellipse"
  | "circle"
  | "triangle"
  | "arc"
  | "curve"
  | "double_curve"
  | "polyline"
  | "highlighter"
  // B8: shapes — end
  ;

export const DRAWING_KIND_TO_U8: Record<drawing_kind, number> = {
  trend_line: 0,
  horizontal_line: 1,
  horizontal_ray: 2,
  vertical_line: 3,
  rectangle: 4,
  text: 5,
  brush: 6,
  path: 7,
  long_position: 8,
  short_position: 9,
  // B8: lines — begin (wire ids 32..=47)
  ray: 32,
  extended_line: 33,
  info_line: 34,
  trend_angle: 35,
  cross_line: 36,
  arrow_line: 37,
  horizontal_segment: 38,
  vertical_ray: 39,
  vertical_segment: 40,
  price_line: 41,
  // B8: lines — end
  // B8: channels — begin (wire ids 48..=63)
  parallel_channel: 48,
  regression_trend: 49,
  flat_top_bottom: 50,
  disjoint_channel: 51,
  price_channel: 52,
  // B8: channels — end
  // B8: fibonacci — begin (wire ids 64..=95)
  fib_retracement: 64,
  trend_based_fib_extension: 65,
  fib_channel: 66,
  fib_time_zone: 67,
  trend_based_fib_time: 68,
  fib_speed_resistance_fan: 69,
  fib_speed_resistance_arcs: 70,
  fib_circles: 71,
  fib_spiral: 72,
  fib_wedge: 73,
  // B8: fibonacci — end
  // B8: pitchforks_gann — begin (wire ids 96..=127)
  andrews_pitchfork: 96,
  schiff_pitchfork: 97,
  modified_schiff_pitchfork: 98,
  inside_pitchfork: 99,
  pitchfan: 100,
  gann_box: 101,
  gann_square: 102,
  gann_square_fixed: 103,
  gann_fan: 104,
  // B8: pitchforks_gann — end
  // B8: projection_annotations — begin (wire ids 128..=159)
  forecast: 128,
  bars_pattern: 129,
  price_range: 130,
  date_range: 131,
  date_and_price_range: 132,
  projection: 133,
  anchored_text: 134,
  note: 135,
  price_note: 136,
  callout: 137,
  comment: 138,
  price_label: 139,
  signpost: 140,
  flag_mark: 141,
  arrow_mark_up: 142,
  arrow_mark_down: 143,
  arrow_mark_left: 144,
  arrow_mark_right: 145,
  icon: 146,
  simple_tag: 147,
  simple_annotation: 148,
  // B8: projection_annotations — end
  // B8: patterns_elliott_cycles — begin (wire ids 160..=191)
  xabcd_pattern: 160,
  cypher_pattern: 161,
  abcd_pattern: 162,
  head_and_shoulders: 163,
  triangle_pattern: 164,
  three_drives_pattern: 165,
  elliott_impulse_wave: 166,
  elliott_correction_wave: 167,
  elliott_triangle_wave: 168,
  elliott_double_combo: 169,
  elliott_triple_combo: 170,
  cyclic_lines: 171,
  time_cycles: 172,
  sine_line: 173,
  // B8: patterns_elliott_cycles — end
  // B8: shapes — begin (wire ids 192..=223)
  rotated_rectangle: 192,
  ellipse: 193,
  circle: 194,
  triangle: 195,
  arc: 196,
  curve: 197,
  double_curve: 198,
  polyline: 199,
  highlighter: 200,
  // B8: shapes — end
};

/**
 * One defining anchor of a drawing: a fractional logical bar index (integer values sit at bar
 * centers — the engine's `logical_to_coordinate` space) plus a price. The unused coordinate of
 * the full-span kinds is stored but never read (a horizontal line's `logical`, a vertical
 * line's `price`).
 *
 * `time` is the anchor's time identity in UTC seconds (fractional between bars, extrapolated
 * with the prevailing bar interval beyond the data). It is what survives an interval switch, a
 * data reload, persistence restore into a different history window, and cross-chart sync. It is
 * absent on non-time (tick/volume/range bar) charts, for `logical` anchors while the chart has too
 * few time points to derive one, and for positions so far beyond the data that the time would
 * leave the supported value range. An anchor supplied by `time` keeps reporting it while pending.
 */
export interface drawing_point {
  logical: number;
  price: number;
  time?: number;
}

/**
 * An anchor supplied to {@link chart_api.add_drawing} or {@link drawing_api.set_points}: a
 * `logical` index, a `time` (UTC seconds), or both — `time` wins when both are present and
 * disagree. A `time` supplied before the chart has data stays pending and resolves when data
 * arrives. `points()` output is accepted unchanged.
 */
export type drawing_point_input =
  | { logical: number; price: number; time?: number }
  | { time: number; price: number; logical?: number };

/** One anchor rewrite in {@link chart_api.set_drawings_points}. */
export interface drawing_points_update {
  drawing: drawing_api | number;
  points: drawing_point_input[];
}

/**
 * A multiplicative price-basis segment for {@link chart_api.rescale_drawing_prices}: anchors whose
 * time lies in `[from_time, to_time)` (UTC seconds; omitted = unbounded) are multiplied by
 * `factor` (1e-6..1e6). Segments must not overlap.
 */
export interface drawing_price_segment {
  from_time?: number;
  to_time?: number;
  factor: number;
}

/** Horizontal label alignment shared by every tool's text (canvas `textAlign` keywords). */
export type drawing_text_h_align = "left" | "center" | "right";
/** Vertical label alignment: above / inline with / below the tool at the selected horizontal slot. */
export type drawing_text_v_align = "top" | "middle" | "bottom";
export type drawing_line_cap = "none" | "arrow" | "circle";
export type drawing_magnet_mode = "off" | "weak" | "strong";
export type drawing_interval_unit = "seconds" | "minutes" | "hours" | "days" | "weeks" | "months" | "ticks" | "ranges";
export interface drawing_interval { unit: drawing_interval_unit; value: number }
export interface drawing_interval_visibility { enabled: boolean; intervals: drawing_interval[] }
export type drawing_label_metric = "price" | "price_change" | "percent_change" | "ticks" | "bar_count" | "date_time_range" | "duration" | "angle" | "distance" | "volume_in_range";
export type drawing_label_position = "above" | "on" | "below" | "inside" | "outside";
export interface drawing_label_options { metric: drawing_label_metric; visible: boolean; position: drawing_label_position; text?: string }
export interface drawing_level { value: number; color: string; visible: boolean; style: string; fill_between: boolean; fill_color?: string; label_visible: boolean }
export type drawing_property_type = "boolean" | "number" | "integer" | "string" | "color" | "enum" | "points" | "levels" | "interval_set";
export interface drawing_property_descriptor { name: string; property_type: drawing_property_type; default: unknown; min?: number; max?: number; enum_values: string[] }
export interface drawing_property_schema { revision: number; kind: drawing_kind; properties: drawing_property_descriptor[] }
export interface drawing_template { name: string; kind: drawing_kind; options: Partial<drawing_options> }
export type drawing_kind_options =
  | { kind: "rectangle"; fill_color?: string; preview_fill_color?: string; border_visible: boolean; show_labels: boolean; axis_bands_visible: boolean; label_color?: string; label_text_color?: string; snap_time_to_data: boolean }
  | { kind: "text"; box_color?: string; box_border_color?: string; box_border_width: number }
  | { kind: "position"; levels: drawing_level[]; account_size: number; risk_percent: number }
  | { kind: "generic" }
  // B8: lines — begin
  | { kind: "line"; stats_position: drawing_stats_position }
  // B8: lines — end
  // B8: channels — begin
  | { kind: "channel"; middle_line: boolean; middle_color: string | null }
  | {
    kind: "regression_trend";
    middle_line: boolean;
    middle_color: string | null;
    upper_deviation: number;
    lower_deviation: number;
    use_upper_deviation: boolean;
    use_lower_deviation: boolean;
    source: indicator_input_source;
    show_pearsons: boolean;
  }
  // B8: channels — end
  // B8: fibonacci — begin
  | ({ kind: "fibonacci" } & Required<fibonacci_tool_options>)
  // B8: fibonacci — end
  // B8: pitchforks_gann — begin
  | { kind: "pitchfork"; levels: drawing_level[] }
  | {
    kind: "gann";
    levels: drawing_level[];
    time_levels: drawing_level[];
    angles: drawing_level[];
    arcs: drawing_level[];
    reverse: boolean;
    show_angles: boolean;
    show_stats: boolean;
    scale_ratio: number | null;
    size_bars: number;
  }
  // B8: pitchforks_gann — end
  // B8: projection_annotations — begin
  | {
    kind: "projection_annotation";
    bars_mode: bars_pattern_mode;
    mirrored: boolean;
    flipped: boolean;
    /** Number of bars a `bars_pattern` copied (0 for other tools). */
    pattern_bars: number;
    icon: drawing_icon;
    icon_size: number;
    always_show_text: boolean;
  }
  // B8: projection_annotations — end
  // B8: patterns_elliott_cycles — begin
  | { kind: "pattern"; show_ratios: boolean }
  | { kind: "elliott_wave"; degree: elliott_wave_degree; show_wave: boolean }
  // B8: patterns_elliott_cycles — end
  // B8: shapes — begin
  | { kind: "shape"; closed: boolean }
  // B8: shapes — end
  ;

// B8: lines — begin
/** Where a Lines-family stats box sits: beyond the first anchor, below the midpoint, or beyond the second anchor. */
export type drawing_stats_position = "start" | "middle" | "end";
/** Lines-family options (`tool_options.line`); absent fields keep their defaults. */
export interface line_tool_options {
  /** Stats box position along the anchor segment (default `"end"`). */
  stats_position?: drawing_stats_position;
}
// B8: lines — end
// B8: channels — begin
/**
 * Channels-family options (`tool_options.channel`). Absent fields take the tool's own default
 * and `null` resets one field. The deviation, source, and Pearson fields apply to
 * `regression_trend` only.
 */
export interface channel_tool_options {
  /** Dashed middle line; the regression line on a regression trend (default on for the parallel channel and the regression trend, off for the others). */
  middle_line?: boolean | null;
  /** Middle-line CSS color; `""` follows the stroke color (default). */
  middle_color?: string | null;
  /** Upper line offset in residual standard deviations (default 2, range -100..100). */
  upper_deviation?: number | null;
  /** Lower line offset in residual standard deviations (default -2, range -100..100). */
  lower_deviation?: number | null;
  /** Paint the upper deviation line and its zone (default true). */
  use_upper_deviation?: boolean | null;
  /** Paint the lower deviation line and its zone (default true). */
  use_lower_deviation?: boolean | null;
  /** Bar value the regression fits (default `"close"`). */
  source?: indicator_input_source | null;
  /** Paint Pearson's R below the regression's start (default true). */
  show_pearsons?: boolean | null;
}
// B8: channels — end
// B8: fibonacci — begin
/**
 * Fibonacci-family options (`tool_options.fibonacci`); absent fields keep their defaults. Each
 * tool's property schema lists the fields it reads.
 */
export interface fibonacci_tool_options {
  /**
   * Swap the ends levels 0 and 1 sit at (retracement, extension, channel, fan), project time
   * zones backward, or turn the spiral counterclockwise (default `false`).
   */
  reverse?: boolean;
  /** Show level values in labels (default `true`). */
  show_levels?: boolean;
  /** Show level prices in labels: retracement and extension (default `true`). */
  show_prices?: boolean;
  /** Show level values as percents, `61.8%` instead of `0.618` (default `false`). */
  levels_as_percent?: boolean;
  /** Interpolate price levels in log space: retracement, extension, channel (default `false`). */
  log_scale?: boolean;
  /** Show the dashed trend line through the anchors (default `true`). */
  trend_line?: boolean;
  /** Show the speed resistance fan's grid (default `true`). */
  grid?: boolean;
  /** Draw speed resistance arcs as full circles (default `false`). */
  full_circles?: boolean;
  /**
   * Level label placement: beyond the left end, centered, or beyond the right end of price
   * levels; left of, on, or right of time levels. Absent: `"left"` for price levels, `"right"`
   * for time levels.
   */
  label_h_align?: drawing_text_h_align;
  /**
   * Level label placement: above, on, or below price levels; at the top, middle, or bottom of
   * time levels. Absent: `"middle"` for price levels, `"bottom"` for time levels.
   */
  label_v_align?: drawing_text_v_align;
}
// B8: fibonacci — end
// B8: pitchforks_gann — begin
/**
 * Gann-tool options (`tool_options.gann`); absent fields keep their defaults. Each field applies
 * to the Gann tools named on it. Pitchforks and the pitchfan use only the common options: their
 * `levels` are median offsets in half-handle widths (level 1 passes through the handle's ends).
 */
export interface gann_tool_options {
  /** Gann box vertical levels as fractions of the box width (its `levels` are the price levels). */
  time_levels?: drawing_level[];
  /**
   * Gann box (with `show_angles`) and Gann squares: angle lines from the pivot corner as positive
   * multiples of the 1×1 slope (`2` is 1×2, `0.5` is 2×1).
   */
  angles?: drawing_level[];
  /** Gann squares: quarter arcs around the pivot corner, radii as positive fractions of the side. */
  arcs?: drawing_level[];
  /** Gann box and squares: measure from the second anchor's price; the fixed square grows down. */
  reverse?: boolean;
  /** Gann box: paint `angles` from the pivot corner (default `false`). */
  show_angles?: boolean;
  /** Gann squares: the price range, bars, and price-per-bar box (default `true`). */
  show_stats?: boolean;
  /**
   * Gann fan and fixed square: price units per bar of the 1×1 angle. `null` makes the fan's 1×1
   * pass through its second anchor and the fixed square a square on screen.
   */
  scale_ratio?: number | null;
  /** Fixed square: side length in bars, 1..=100000 (default 20). */
  size_bars?: number;
}
// B8: pitchforks_gann — end
// B8: projection_annotations — begin
/** How a `bars_pattern` paints its copied bars: high–low or open–close sticks, or a line through one field. */
export type bars_pattern_mode = "hl_bars" | "oc_bars" | "line_open" | "line_high" | "line_low" | "line_close";
/** The bounded built-in icon set of the `icon` tool. */
export type drawing_icon =
  | "star"
  | "heart"
  | "check"
  | "cross"
  | "circle"
  | "square"
  | "diamond"
  | "triangle_up"
  | "triangle_down";
/** Projection & Annotations options (`tool_options.projection_annotation`); absent fields keep their defaults. */
export interface projection_annotation_tool_options {
  /** `bars_pattern` paint mode (default `"hl_bars"`). */
  bars_mode?: bars_pattern_mode;
  /** `bars_pattern`: reverse the copied bars in time (default `false`). */
  mirrored?: boolean;
  /** `bars_pattern`: turn the copied bars upside down within the box between its two anchors (default `false`). */
  flipped?: boolean;
  /**
   * `bars_pattern`: the copied `[open, high, low, close]` bars, oldest first (at most 128). The
   * engine captures them when the pattern is created; paste, sync, and persistence carry them,
   * while named templates keep only the style.
   */
  bars?: [number, number, number, number][];
  /** `icon` shape (default `"star"`). */
  icon?: drawing_icon;
  /** `icon` size in CSS px, 8..128 (default 24). */
  icon_size?: number;
  /**
   * `note`: paint the text box while the note is neither hovered, selected, nor edited (default
   * `false`: only the pin shows then).
   */
  always_show_text?: boolean;
}
// B8: projection_annotations — end
// B8: patterns_elliott_cycles — begin
/**
 * Elliott wave degree, largest first. Labels follow the Frost–Prechter notation: supercycle-scale
 * degrees use upper Roman numerals and lowercase letters, primary to minor Arabic numerals and
 * uppercase letters, minute to subminuette lower Roman numerals and lowercase letters; within each
 * triad the degrees are ringed, parenthesized, and bare. The millennium degrees wrap upper Roman
 * numerals in braces, brackets, and angle brackets.
 */
export type elliott_wave_degree =
  | "supermillennium"
  | "millennium"
  | "submillennium"
  | "grand_supercycle"
  | "supercycle"
  | "cycle"
  | "primary"
  | "intermediate"
  | "minor"
  | "minute"
  | "minuette"
  | "subminuette";
/** Patterns, Elliott waves, and cycles options (`tool_options.pattern`); absent fields keep their defaults. */
export interface pattern_tool_options {
  /** XABCD, cypher, ABCD, and three drives: dashed ratio connectors and their ratios (default `true`). */
  show_ratios?: boolean;
  /** Elliott waves: the degree whose notation labels the waves (default `"intermediate"`). */
  degree?: elliott_wave_degree;
  /** Elliott waves: the wave polyline; `false` leaves only the labels (default `true`). */
  show_wave?: boolean;
}
// B8: patterns_elliott_cycles — end
// B8: shapes — begin
/** Shapes-family options (`tool_options.shape`); absent fields keep their defaults. */
export interface shape_tool_options {
  /**
   * Polyline only: join the last vertex back to the first and, while `fill_enabled`, fill the
   * enclosed region by the nonzero rule (default `false`). Other shapes ignore it. The fill is
   * bounded work: more than 2,048 vertices, or a polygon so heavily self-intersecting that its
   * fill exceeds the tessellation bounds, paints the outline only (no fill, no interior selection
   * target, no error).
   */
  closed?: boolean;
}
// B8: shapes — end

/**
 * Family-specific option blocks (B8), one optional block per drawing family. Patches deep-merge:
 * absent keys keep their values and `null` resets a block to its defaults.
 */
export interface drawing_tool_options {
  // B8: lines — begin
  line?: line_tool_options | null;
  // B8: lines — end
  // B8: channels — begin
  channel?: channel_tool_options | null;
  // B8: channels — end
  // B8: fibonacci — begin
  fibonacci?: fibonacci_tool_options | null;
  // B8: fibonacci — end
  // B8: pitchforks_gann — begin
  gann?: gann_tool_options | null;
  // B8: pitchforks_gann — end
  // B8: projection_annotations — begin
  projection_annotation?: projection_annotation_tool_options | null;
  // B8: projection_annotations — end
  // B8: patterns_elliott_cycles — begin
  pattern?: pattern_tool_options | null;
  // B8: patterns_elliott_cycles — end
  // B8: shapes — begin
  shape?: shape_tool_options | null;
  // B8: shapes — end
}

/**
 * A drawing's options (engine `Drawing`). Every tool can carry a text label placed by the
 * 3×3 `text_h_align`/`text_v_align` against the tool's geometry, except the nine tools that
 * paint no text on the chart (`forecast`, `bars_pattern`, `price_range`, `date_range`,
 * `date_and_price_range`, `projection`, `flag_mark`, `icon`, `simple_tag`), which keep `text`
 * without showing it (the simple tag shows it as its price-axis tag). A painted label is
 * edited in place: double-click it (or select the drawing and press Enter or F2), and tools that
 * start from a default text open the editor when placed. Colors parse per the engine's
 * CSS rules; `""` for optional colors means "follow the default" (the border color at
 * 20% alpha for a rectangle's fill, the chart's `layout.textColor` for labels), and
 * `text_size: null` follows `layout.fontSize`.
 */
export interface drawing_options {
  /** Hypothetical balance for Long/Short Position statistics (default 1,000); independent of broker orders. */
  position_account_size: number;
  /** Percentage of the hypothetical balance risked at the stop, 0–100 (default 25). */
  position_risk_percent: number;
  name: string;
  group_id: string;
  revision: number;
  visible: boolean;
  locked: boolean;
  z_order: number;
  interval_visibility: drawing_interval_visibility;
  stroke_start: drawing_line_cap;
  stroke_end: drawing_line_cap;
  extend_left: boolean;
  extend_right: boolean;
  fill_enabled: boolean;
  magnet: drawing_magnet_mode;
  labels: drawing_label_options[];
  levels: drawing_level[];
  /** Price scale used for price-coordinate conversion (`overlay` is the pane's overlay scale). */
  price_scale_id: "left" | "right" | "overlay";
  /** Line/border color (default: the canonical primary token). */
  color: string;
  /** Stroke width in CSS px (default 2; 1 for a rectangle's border). */
  width: number;
  /** Stroke style (default `"solid"`). */
  style: line_style;
  /**
   * Region fill of the rectangle and of family tools that fill (default `""` = the stroke color
   * washed out, 20% alpha for the rectangle). Unused by the line kinds and the text tool.
   */
  fill_color: string;
  /** Interactive rectangle preview fill (`""` = `fill_color`). */
  preview_fill_color: string;
  /** Rectangle outline visibility (default false). */
  border_visible: boolean;
  /** Rectangle endpoint labels on the price and time axes. */
  show_labels: boolean;
  /** Official 15 CSS px rectangle shading in the price/time axis panes. */
  axis_bands_visible: boolean;
  /** Rectangle endpoint-label background (`""` = drawing color). */
  label_color: string;
  /** Rectangle endpoint-label text (`""` = automatic black/white contrast against `label_color`). */
  label_text_color: string;
  /** Snap rectangle x anchors to canonical data times. */
  snap_time_to_data: boolean;
  /** The tool's text label (`""` = none). */
  text: string;
  /** Label color (default `""` = the chart's `layout.textColor`). */
  text_color: string;
  /** Label glyph size in CSS px (`null` = the chart's `layout.fontSize`). */
  text_size: number | null;
  /** Label font weight (numeric CSS weight 100–900; `null` = normal 400, 700 = bold). */
  text_weight: number | null;
  /** Italic label glyphs (default `false`). */
  text_italic: boolean;
  /**
   * Legacy boolean view of the label weight (`true` = semibold or heavier, i.e.
   * `text_weight >= 600`). In patches, `text_bold: true` maps to weight 700 and `false`
   * resets to normal when no explicit `text_weight` is given.
   */
  text_bold: boolean;
  text_h_align: drawing_text_h_align;
  text_v_align: drawing_text_v_align;
  /** Text-tool container background (the public reference's text-box background; default `""` = none). */
  box_color: string;
  /** Text-tool container border color (default `""` = none). */
  box_border_color: string;
  /** Text-tool container border width in CSS px (default 1). */
  box_border_width: number;
  /**
   * Family-specific option blocks (B8). The Lines family's stats (`info_line` shows them by
   * default) are the common `labels` list; `tool_options.line.stats_position` places their box.
   */
  tool_options: drawing_tool_options;
}

/** A drawing as listed by {@link chart_api.drawings}. This inspection shape is not persistence. */
export interface drawing_info extends drawing_options {
  id: number;
  kind: drawing_kind;
  pane_index: number;
  points: drawing_point[];
}

/** Pane topology persisted by schema V1. Array order is pane order; IDs survive reordering. */
export interface persisted_pane_v1 {
  id: `pane-${number}`;
  stretch_factor: number;
  preserve_empty: boolean;
}

/** Stable semantic drawing style persisted by schema V1. Omitted fields restore defaults. */
export interface persisted_drawing_style_v1 {
  position_account_size?: number;
  position_risk_percent?: number;
  name?: string;
  group_id?: string;
  revision?: number;
  visible?: boolean;
  locked?: boolean;
  z_order?: number;
  interval_visibility?: drawing_interval_visibility;
  stroke_start?: drawing_line_cap;
  stroke_end?: drawing_line_cap;
  extend_left?: boolean;
  extend_right?: boolean;
  fill_enabled?: boolean;
  magnet?: drawing_magnet_mode;
  labels?: drawing_label_options[];
  levels?: drawing_level[];
  price_scale_id?: "left" | "right" | "overlay";
  color?: string;
  width?: number;
  line_style?: line_style;
  fill_color?: string;
  preview_fill_color?: string;
  border_visible?: boolean;
  show_labels?: boolean;
  axis_bands_visible?: boolean;
  label_color?: string;
  label_text_color?: string;
  snap_time_to_data?: boolean;
  text?: string;
  text_color?: string;
  text_size?: number;
  text_weight?: number;
  text_italic?: boolean;
  text_h_align?: drawing_text_h_align;
  text_v_align?: drawing_text_v_align;
  box_color?: string;
  box_border_color?: string;
  box_border_width?: number;
  tool_options?: drawing_tool_options;
}

/** One semantic drawing in the durable V1 persistence contract. */
export interface persisted_drawing_v1 {
  id: number;
  kind: drawing_kind;
  pane_id: `pane-${number}`;
  /** `{logical, price, time?}`; a present `time` is authoritative when the restoring chart can place it. */
  anchors: drawing_point[];
  /** Non-time (tick/volume/range bar) charts: full-resolution bar identity per anchor. */
  anchor_times_micros?: ({ open_timestamp_micros: number; close_timestamp_micros: number } | null)[];
  style?: persisted_drawing_style_v1;
}

/** Versioned chart persistence V1: pane topology and built-in drawings only. */
export interface chart_state_v1 {
  schema: "aeris_charts-state";
  schema_version: 1;
  panes: persisted_pane_v1[];
  drawings: persisted_drawing_v1[];
  /** Host-defined price basis of the drawing prices (see {@link chart_api.set_drawing_price_basis}). */
  drawing_price_basis?: string;
  /** Hidden timeline-mark groups (see {@link timeline_marks_api.set_group_hidden}); marks never persist. */
  hidden_mark_groups?: string[];
}

export interface trade_stream_stats {
  revision: number;
  stream_capacity_bytes: number;
  dependent_count: number;
  dependent_rebuilds: number;
  dependent_incremental_updates: number;
  /** Lifetime CVD/delta rows computed by study refreshes; a live tip adds only its active bars. */
  dependent_rows_computed: number;
  /** Lifetime footprint and bound candle/bar rows projected; a live tip adds only its active bars. */
  bar_rows_projected: number;
  /** Lifetime tape trades folded into bubble markers; a live tip adds only its new trades. */
  bubble_trades_scanned: number;
  /**
   * Lifetime bubble marker sizes computed; a live tip sizes only its new or merged bubbles unless
   * the peak retained bubble volume changes, which rescales every retained bubble once.
   */
  bubble_markers_sized: number;
}

export interface replay_seek_stats {
  previous_clock_micros: number | null;
  clock_micros: number | null;
  visible_trades: number;
  rebuilt_trades: number;
  incremental_trades: number;
}

export interface replay_clock_stats extends replay_seek_stats {
  stream_count: number;
  depth_stream_count: number;
  visible_depth_events: number;
  rebuilt_depth_events: number;
}

/** V2 adds engine-owned general pane, axis, dataset, and series state. */
export interface chart_state_v2 {
  schema: "aeris_charts-state";
  schema_version: 2;
  panes: (persisted_pane_v1 & { horizontal_domain: unknown })[];
  drawings: persisted_drawing_v1[];
  axes: Record<string, unknown>[];
  datasets: { id: `dataset-${number}`; input: Record<string, unknown>; labels?: (string | null)[] }[];
  series: {
    kind:
      | "XyLine"
      | "XyArea"
      | "RangeArea"
      | "RangeBar"
      | "ErrorBar"
      | "Column"
      | "HorizontalBar"
      | "BoxPlot"
      | "HeatmapGrid"
      | "Scatter"
      | "Bubble";
    pane: number;
    dataset: `dataset-${number}`;
    x_axis_id: string;
    y_axis_id: string;
    visible: boolean;
    title: string;
    color: string | null;
    point_radius: number;
    line_width: number;
    line_style: "solid" | "dotted" | "dashed";
    baseline_value: number | null;
    data_labels: boolean;
    group_id: string | null;
    stack_id: string | null;
    stack_mode: "Normal" | "Percent";
  }[];
  chart_options: Record<string, unknown>;
  drawing_price_basis?: string;
  hidden_mark_groups?: string[];
}

/** Indicator source reference persisted by schema V3: a host series or an earlier study output. */
export type persisted_indicator_source_v3 =
  | { kind: "series"; id: number }
  | { kind: "output"; study: number; output: number };

/** V3 adds engine-owned study bindings to the V1 financial document. */
export interface chart_state_v3 {
  schema: "aeris_charts-state";
  schema_version: 3;
  panes: persisted_pane_v1[];
  drawings: persisted_drawing_v1[];
  indicators: {
    /** Engine indicator definition: `{ kind: indicator_kind, ...parameters }`. A KLineChart study stores
     *  `kind: "klinechart"` beside its {@link klinechart_indicator} fields. */
    kind:
      | ({ kind: "klinechart" } & klinechart_indicator)
      | ({ kind: Exclude<indicator_kind, klinechart_indicator_kind> } & Record<string, unknown>);
    source: persisted_indicator_source_v3;
    source_input: indicator_input_source;
    volume_source?: persisted_indicator_source_v3 | null;
    /** Turnover series of an amount-weighted VWAP; absent when the study has none. */
    amount_source?: persisted_indicator_source_v3 | null;
    styles: indicator_output_style[];
  }[];
  drawing_price_basis?: string;
  hidden_mark_groups?: string[];
}

export type chart_state = chart_state_v1 | chart_state_v2 | chart_state_v3;

/** Counts returned after one validated, atomic state restore. */
export interface persistence_restore_result {
  schema_version: 1 | 2 | 3;
  panes: number;
  drawings: number;
  points: number;
}

/** A live handle to an engine-owned drawing. */
export interface drawing_api {
  readonly id: number;
  kind(): drawing_kind;
  pane_index(): number;
  /** The defining anchors, each with its `time` identity on ordinary time charts. */
  points(): drawing_point[];
  /** Replace the anchors (validated against the kind's anchor count; `time` anchors accepted). */
  set_points(points: drawing_point_input[]): void;
  /** The current options (reference `options()`). */
  options(): drawing_options;
  /** Deep-merge a patch onto this drawing's options (reference `applyOptions`). */
  apply_options(options: Partial<drawing_options>): void;
  /** Remove the drawing from the chart. */
  remove(): void;
}

export type drawing_created_handler = (drawing: drawing_api) => void;
export type drawing_tool_change_handler = (tool: drawing_kind | null) => void;


// ---------------------------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------------------------

/** A single data series on the chart. */
export interface series_api {
  /** Camel-case aliases; both naming styles operate on this same series handle. */
  setData: series_api["set_data"];
  setDataTyped: series_api["set_data_typed"];
  updateTyped: series_api["update_typed"];
  applyOptions: series_api["apply_options"];
  moveToPane: series_api["move_to_pane"];
  priceScale: series_api["price_scale"];
  baselinePrice: series_api["baseline_price"];
  /**
   * Replace the series' data. Accepts OHLC or single-value points; packed to typed arrays here.
   * A full replace clears the series' sequence guard, or installs `options.sequence` as the new
   * baseline (see {@link series_update_options.sequence}).
   */
  set_data(data: readonly series_data[], options?: series_update_options): void;
  /** Diagnostics from the most recent set/update call; `null` is the allocation-free clean case. */
  last_ingestion_diagnostics(): ingestion_diagnostics | null;
  /**
   * Replace the series' data from already-packed columns, skipping `set_data`'s per-object
   * JS packing. `times` are UTC seconds (the engine's time unit); single-value series
   * (line/area/histogram) repeat their value in all four price channels. All arrays must
   * share a length; the engine's usual sort/dedupe/sanitize rules apply.
   *
   * **Input arrays are never mutated and never retained.** The engine copies each column into
   * its own storage on the way in and does all sorting, deduping and sanitizing on those copies.
   * Two consequences callers can rely on:
   * - Passing the **same** view as several channels is safe. A single-value series may pass one
   *   array as all four of `open`/`high`/`low`/`close`, which is the documented way to express
   *   the "repeat their value in all four price channels" contract.
   * - Views over a `SharedArrayBuffer` are safe, and the buffer may be rewritten by the producer
   *   as soon as this call returns.
   *
   * Like {@link set_data}, a full replace clears the sequence guard or installs
   * `options.sequence` as the baseline.
   */
  set_data_typed(columns: ohlc_columns, options?: series_update_options): void;
  /**
   * Streaming update with reference `series.update` semantics: the point replaces the **whole**
   * bar at its time. A time equal to the last bar replaces it and a newer time appends, both in
   * O(1). An older time corrects that historical bar in place (only its level-of-detail path and
   * autoscale chunk are repaired) or inserts a new bar there, which reindexes the series and, for a
   * time no series holds, merges the shared time axis.
   *
   * Partial payloads are not merged: an OHLC point missing some prices is rejected (and warned),
   * `{ time, value }` on a candlestick/bar series flattens the bar to O=H=L=C, and a point without
   * price fields (for example `{ time, volume }`) becomes whitespace. Each case is reported through
   * {@link last_ingestion_diagnostics} with a `code`; use {@link merge} for partial ticks.
   *
   * `options.sequence` enables the per-series stale-delivery guard
   * (see {@link series_update_options.sequence}).
   */
  update(point: series_data, options?: series_update_options): void;
  /**
   * Merge a partial tick into the bar at `point.time` (engine-owned, O(log n) lookup plus the
   * ordinary streaming update). Present fields overwrite, absent fields keep the existing bar. For
   * candlestick/bar series the result is normalized so `high >= max(open, close)` and
   * `low <= min(open, close)`, so a close-only tick extends the high/low as needed; a close-only
   * tick for a new time creates an O=H=L=C bar. Line/area/baseline/histogram series take
   * `value` (or `close`) as their value. A merge without any price field is rejected with
   * `code: "empty_merge"`. Volume and turnover merge into their own series.
   *
   * Emits one `data_changed("update")`. Throws `unsupported_operation` on custom, advanced, and
   * footprint series. An engine-owned series (a bound candle, a trade study, resampled or
   * synthetic bars) rejects it with `code: "derived_series"` and changes nothing.
   */
  merge(point: series_merge_data, options?: series_update_options): void;
  /**
   * Columnar counterpart to {@link merge} for high-rate partial ticks: row `i` merges exactly like
   * `merge({ time: times[i], ... })` with `NaN` entries and omitted columns absent. Rows apply in
   * input order, so a later row for the same time merges into the earlier result, and the engine
   * synchronizes time state and indicators once for the batch. Each row costs what the same
   * {@link merge} would: O(log n) for an existing or tail bar, a reindex for a new historical
   * time. One invalid row (bad timestamp,
   * non-finite or out-of-range value, or no price field) rejects the whole batch; `sequence`
   * guards the batch as one delivery. Emits one `data_changed("update")`. Throws
   * `unsupported_operation` on custom, advanced, and footprint series.
   */
  merge_typed(columns: series_merge_columns, options?: series_update_options): void;
  /**
   * Streaming counterpart to {@link set_data_typed}: append (or replace-last) a **batch** of
   * points from already-packed columns, so a high-rate feed never allocates a JS object per tick.
   * Same column layout and same `times` unit (UTC seconds); single-value series repeat their value
   * in all four price channels; all-NaN rows are whitespace.
   *
   * Rows whose time equals the series' current last point replace it; later rows append. The batch
   * is first repaired exactly as `set_data_typed` repairs a full replace — non-finite rows dropped,
   * stable sort by time, duplicate times collapsed last-wins — and then applied in ascending time
   * order. A row that lands *before* the series' last point is a mid-history insert, handled the
   * same way `update` handles one.
   *
   * A one-row batch is observably identical to {@link update} with the equivalent point: same
   * rendered output, same `data()`, and one `data_changed` notification with scope `"update"`. A
   * 500-row batch is also exactly one notification, not 500.
   *
   * Cost for the streaming shape — every row at or past the chart's last timestamp — is linear in
   * the batch and independent of series length, because each row takes the engine's single-append
   * fast path. When the rows before the series' last bar only correct bars that already exist,
   * they are applied in place (logarithmic lookup per row, no reindex) and the batch's tail rows
   * keep the append path. A batch that inserts new historical times is merged
   * into the series in one linear pass and reindexes the shared time axis once. Use
   * {@link set_data_typed} to rewrite history.
   *
   * The input arrays are never mutated or retained — see {@link set_data_typed} for the aliasing
   * guarantee this shares.
   *
   * `options.sequence` guards the whole batch with one sequence
   * (see {@link series_update_options.sequence}).
   */
  update_typed(columns: ohlc_columns, options?: series_update_options): void;
  /**
   * Bind a `SharedArrayBuffer` ring that the engine drains **once per frame**, or pass `null` to
   * unbind and return to explicit {@link update}/{@link update_typed} calls.
   *
   * This decouples tick delivery from engine calls structurally rather than by convention: there
   * is **no engine call per tick** and the host never decides when to flush. Total frame CPU still
   * includes applying every delivered row and is reported by {@link frame_stats.cpu_ms}; use an
   * appropriate capacity and {@link series_options.max_points} for sustained high-rate streams.
   *
   * Rows are not materialized as JS values. Each drain copies the contiguous run(s) of new row
   * bytes straight into engine memory (at most two `TypedArray.set` calls per attempt — a ring wrap
   * splits the window) and parses them there, with a staging buffer sized once at bind time. Drain
   * planning and copying allocate nothing per row or frame.
   *
   * Rows apply in ring order, each appending or replacing the series' last point exactly as
   * {@link update} would; a row with an invalid timestamp or non-finite value is dropped and counted
   * in {@link frame_stats.ring_dropped_rows}. Unlike {@link update_typed} there is no
   * per-batch sort or dedupe — a ring is a stream, and its producer is expected to write in
   * ascending time.
   *
   * **Producer overrun.** If the producer wrote more than `capacity` rows between two drains, the
   * oldest of them have already been overwritten. The engine then targets the newest `capacity`
   * rows and adds the shortfall to {@link frame_stats.ring_overruns}. With
   * {@link ring_source_layout.sequence_offset}, every selected slot is validated before and after
   * copying, so a concurrent wrap is retried instead of rendering a torn mixed-generation window.
   *
   * Binding starts from the producer's current cursor, so it picks up new rows rather than
   * replaying whatever is already sitting in the ring. Binding a second ring to the same series
   * replaces the first. `set_ring_source(null)` releases the engine's views over the buffer, as
   * does removing the series.
   *
   * The cost model of {@link update_typed} applies to the drained rows as well: rows at or past the
   * chart's last timestamp take the engine's single-append fast path; a row correcting an existing
   * earlier bar is repaired in place, while a row with a new earlier time costs a reindex of the
   * shared time axis. On a chart with several series, keep the ring-fed one at the global tip.
   *
   * Composes with {@link series_options.max_points}: a ring-fed series with a cap holds its window
   * and plateaus in memory, which is the shape a long live session wants.
   *
   * Requires the page to be cross-origin isolated, since that is what makes `SharedArrayBuffer`
   * available at all. Throws if the layout is unusable (a channel that overruns `row_stride`, a
   * misaligned cursor, a ring that does not fit the buffer, zero capacity), and throws
   * `unsupported_operation` for an engine-owned series (a bound candle, a trade study, resampled
   * or synthetic bars), which only its trade stream or source feeds. A ring bound before its
   * series became engine-owned is not unbound: unbind it with `set_ring_source(null)`, or its
   * drained rows are dropped and counted in {@link frame_stats.ring_dropped_rows}.
   */
  set_ring_source(buffer: SharedArrayBuffer | null, layout?: ring_source_layout): void;
  /**
   * Push the current bid/ask quotes (industry-standard; render with `bid_ask_visible: true`).
   * Pass `null` to hide a side.
   */
  set_bid_ask(bid: number | null, ask: number | null): void;
  /**
   * Remove `count` data items from the end of the series (reference `ISeriesApi.pop`, default
   * `count: 1`). Divergence: reference returns the removed items; here the engine drops them and the
   * method returns nothing. On an engine-owned series (a bound candle, a trade study, resampled or
   * synthetic bars) it removes nothing: it records a `derived_series` rejection in
   * {@link last_ingestion_diagnostics}, warns, and fires no `data_changed`.
   */
  pop(count?: number): void;
  /**
   * The last value data of the series (reference `ISeriesApi.lastValueData`). `global_last: false`
   * reads the last value in the current visible range, `true` the absolute last value. Returns
   * `null` when the series has no value.
   */
  last_value_data(global_last?: boolean): last_value_data | null;
  /**
   * The current price formatter of this series (reference `ISeriesApi.priceFormatter`). Divergence:
   * reference returns an `IPriceFormatter` object with a `format` method; this method returns
   * the bare format function `(price) => string`.
   */
  price_formatter(): (price: number) => string;
  /** Current pane index of this live series. */
  pane_index(): number;
  /** Apply series options (currently: `color`). */
  apply_options(options: Partial<any_series_options>): void;
  /** The current (deep-merged) options of this series (reference `ISeriesApi.options`). */
  options(): any_series_options;
  /** Change how the primary series is drawn (candlestick/bar/line/area/histogram). */
  set_type(kind: series_kind): void;
  /** Move this series into stacked pane `pane_index` (0 = price pane), creating it if needed. */
  move_to_pane(pane_index: number, stretch?: number): void;
  /** Add a horizontal price line on this series; returns a handle with `.remove()`. */
  create_price_line(options: price_line_options): price_line_api;
  /** Replace this series' per-bar markers (pass `[]` to clear). Roadmap Phase B4. */
  set_markers(markers: readonly series_marker[], options?: Partial<series_marker_options>): void;
  /** Price-scale handle currently used by this series. */
  price_scale(): price_scale_api;
  /** Pane-local id of the price scale currently owning this series. */
  price_scale_id(): string;
  /** Rebind to an existing price scale in this pane without recreating the series. */
  move_to_price_scale(id: string): void;
  /**
   * Chart-content `y` (CSS px from the top of the stacked pane area, see {@link pane_geometry}) for
   * a price on this series' own pane and price scale, in that scale's mode and base. For a series
   * in a lower pane the result lies inside that pane's `[top, top + height]`, not pane-local: this
   * is how a host targets a sub-pane. `null` when the scale has no range yet or the price is not
   * finite.
   */
  price_to_coordinate(price: number): number | null;
  /** Inverse of {@link series_api.price_to_coordinate}: price on this series' scale at chart-content `y`. */
  coordinate_to_price(coordinate: number): number | null;
  /**
   * The baseline price a baseline series currently compares against: its pinned `baseline_value`,
   * else its `baseline_mode` resolved over the visible range (the same value the fills, quadrant
   * strokes, reference line, live price line, axis chip and crosshair marker share). It moves
   * with the visible window unless pinned. `null` for every other series type and before the
   * chart has a visible range.
   */
  baseline_price(): number | null;
  bars_in_logical_range(range: logical_range): bars_info | null;
  data_by_index(logical_index: number, mismatch_direction?: mismatch_direction): series_data | null;
  data(): readonly series_data[];
  series_type(): series_kind;
  /** Current engine-owned style snapshot for this output, or `null` for a plain series. */
  indicator_output_style(): indicator_output_style | null;
  /** Atomically update an indicator output's persisted presentation style. */
  set_indicator_output_style(patch: Partial<indicator_output_style>): boolean;
  subscribe_data_changed(handler: data_changed_handler): void;
  unsubscribe_data_changed(handler: data_changed_handler): void;
  /**
   * Attach a series primitive (reference `ISeriesApi.attachPrimitive`, plugin platform Phase C-b)
   * and repaint. The primitive records backend-neutral draw commands (no raw canvas), so its
   * output is identical on the WebGPU and Canvas2D backends; its price converter and price-axis
   * labels resolve on this series' price scale, and its `autoscale_info` hook can expand that
   * scale's range. Divergence: reference returns `void`; here the returned handle detaches. Removing
   * the series auto-detaches its primitives (the `detached` hook fires).
   */
  /** @experimental Host primitive contract; extension persistence is host-owned. */
  attach_primitive(primitive: series_primitive): series_primitive_handle;
  /**
   * The indicator lineage of this series when it is an output of
   * {@link chart_api.add_sma}/{@link chart_api.add_ema}/{@link chart_api.add_bollinger}, or
   * `null` for a plain (or source) series. Combined with {@link pane_api.get_series} and
   * {@link pane_api.get_geometry} this is the enumeration half of platform indicator chrome:
   * which indicators are active, where they live, and what to label them.
   */
  indicator_info(): indicator_info | null;
  /** The engine-side series id. */
  readonly id: number;
}

/** A first-class tick-driven footprint handle. Generic OHLC setters are rejected at runtime. */
export interface footprint_series_api extends series_api {
  set_trades(trades: readonly footprint_trade[]): void;
  set_trades_typed(columns: footprint_trade_columns): void;
  update_trades(trades: readonly footprint_trade[]): "tip" | "historical";
  update_trades_typed(columns: footprint_trade_columns): "tip" | "historical";
  update_trade(trade: footprint_trade): "tip" | "historical";
  footprint_bars(): readonly footprint_bar[];
  footprint_bar(index: number): footprint_bar | null;
  apply_options(options: Partial<any_series_options> & Partial<footprint_series_options>): void;
  options(): any_series_options & footprint_series_options;
  series_type(): "footprint";
}

/** The horizontal (time) scale. */
export interface time_scale_api {
  fitContent: time_scale_api["fit_content"];
  scrollToRealTime: time_scale_api["scroll_to_real_time"];
  setVisibleRange: time_scale_api["set_visible_range"];
  getVisibleRange: time_scale_api["get_visible_range"];
  setVisibleLogicalRange: time_scale_api["set_visible_logical_range"];
  getVisibleLogicalRange: time_scale_api["get_visible_logical_range"];
  /** Distance in logical bars between the latest point and the right edge. */
  scroll_position(): number;
  /**
   * Scroll to a logical right-edge position. `animated: true` eases over ~300 ms with a cubic
   * ease-out (suppressed under prefers-reduced-motion); falsy applies immediately.
   */
  scroll_to_position(position: number, animated: boolean): void;
  /**
   * Return the latest point to the real-time edge at the configured `right_offset` (reference
   * `scrollToRealTime`): animated over ~400 ms, immediate under prefers-reduced-motion.
   */
  scroll_to_real_time(): void;
  /** Restore configured default spacing and right offset. */
  reset_time_scale(): void;
  fit_content(): void;
  apply_options(options: Partial<time_scale_options>): void;
  options(): time_scale_options;
  get_visible_logical_range(): logical_range | null;
  /**
   * Show this logical range. Fractional borders are kept exactly (reference
   * `setVisibleLogicalRange`), so restoring a saved `get_visible_logical_range()` is jump-free.
   */
  set_visible_logical_range(range: logical_range): void;
  get_visible_range(): time_range | null;
  set_visible_range(range: time_range): void;
  /** Fire after the visible logical range changes. */
  subscribe_visible_logical_range_change(handler: visible_logical_range_handler): void;
  unsubscribe_visible_logical_range_change(handler: visible_logical_range_handler): void;
  /** Fire after the visible time range changes. */
  subscribe_visible_time_range_change(handler: visible_time_range_handler): void;
  unsubscribe_visible_time_range_change(handler: visible_time_range_handler): void;
  /** Fire after the time scale's media size changes (reference `subscribeSizeChange`). */
  subscribe_size_change(handler: size_change_handler): void;
  unsubscribe_size_change(handler: size_change_handler): void;
  /**
   * `x` (CSS px from the plot-area left edge; add {@link pane_geometry.left} for container x) of an
   * exact bar timestamp. Time, logical and `x` conversions are the same for every pane.
   */
  time_to_coordinate(time: number): number | null;
  /** Timestamp of the bar nearest `x` (CSS px from the plot-area left edge). */
  coordinate_to_time(x: number): number | null;
  /** `x` (CSS px from the plot-area left edge) of a possibly fractional logical bar index. */
  logical_to_coordinate(logical: number): number | null;
  /** Logical bar index at `x` (CSS px from the plot-area left edge). */
  coordinate_to_logical(x: number): number | null;
  /** Exact timestamp lookup, or reference-compatible lower-bound lookup when `find_nearest` is true. */
  time_to_index(time: number, find_nearest?: boolean): number | null;
  /** Current media-coordinate width of the horizontal scale. */
  width(): number;
  /** Current media-coordinate height of the horizontal axis, or zero when hidden. */
  height(): number;
}

/** A pane price scale. The handle becomes stale when its pane or named scale is removed. */
export interface price_scale_api {
  applyOptions: price_scale_api["apply_options"];
  setVisibleRange: price_scale_api["set_visible_range"];
  getVisibleRange: price_scale_api["get_visible_range"];
  apply_options(options: deep_partial<price_scale_options>): void;
  options(): price_scale_options;
  width(): number;
  set_visible_range(range: price_range): void;
  get_visible_range(): price_range | null;
  set_auto_scale(on: boolean): void;
}

/** A stacked pane (roadmap Phase B1), informed by common public chart APIs. */
export interface pane_api {
  paneIndex: pane_api["pane_index"];
  /** This pane's current index (0 = top/price pane). Throws after this pane is removed. */
  pane_index(): number;
  /** Current CSS height in px (from the last layout pass). */
  get_height(): number;
  /**
   * This pane's content-area geometry in CSS px relative to the chart container's top-left
   * (from the last layout pass). Absolutely-position platform chrome against it — e.g. a
   * industry-standard indicator chip pinned at `{ left, top }` of the pane. Operations on a
   * removed pane handle throw instead of silently targeting a replacement pane.
   */
  get_geometry(): pane_geometry;
  /** Resize this pane to `height` CSS px, absorbing the delta from its neighbour. */
  set_height(height: number): void;
  /** This pane's relative stretch factor (height weight). */
  get_stretch_factor(): number;
  /** Set this pane's relative stretch factor. */
  set_stretch_factor(factor: number): void;
  /**
   * Move this pane to the `target` index (reference `IPaneApi.moveTo`). Returns `false` without
   * changing anything when the target index is rejected. A removed handle throws.
   * Divergence: reference returns `void`.
   */
  move_to(target: number): boolean;
  /** Whether this pane is kept while it has no series (reference `IPaneApi.preserveEmptyPane`). */
  preserve_empty_pane(): boolean;
  /** Set whether to keep this pane while it has no series (reference `IPaneApi.setPreserveEmptyPane`). */
  set_preserve_empty_pane(flag: boolean): void;
  /** The series attached to this pane, as live handles (reference `IPaneApi.getSeries`). */
  get_series(): (series_api | general_series_api)[];
  /**
   * Attach a pane primitive (reference `IPaneApi.attachPrimitive`, plugin platform Phase C-a) and
   * repaint. The primitive records backend-neutral draw commands (no raw canvas), so its
   * output is identical on the WebGPU and Canvas2D backends. Divergence: reference returns `void`;
   * here the returned handle detaches. The pane binding is by index — it does not follow
   * later pane moves/removals (a removed pane's primitives draw nowhere until detached).
   */
  /** @experimental Host primitive contract; extension persistence is host-owned. */
  attach_primitive(primitive: pane_primitive): pane_primitive_handle;
  /**
   * Attach a canvas primitive (plugin platform Phase C-e — the Canvas2D escape hatch) and
   * repaint. The primitive paints with arbitrary Canvas2D calls on the plugin overlay canvas
   * through a reference-style `CanvasRenderingTarget2D` mirror, so reference plugin renderers
   * port near-verbatim. Plugin limits remain explicit: plugin
   * content is Canvas2D-only, always above the whole pane (no pane scissor, no z-ordering
   * between engine layers — `normal`/`top` only order among canvas views) and below the axis
   * chrome/crosshair. Divergence: reference returns `void`; here the returned handle detaches.
   */
  /** @experimental Package-side Canvas2D extension; persistence is host-owned. */
  attach_canvas_primitive(primitive: canvas_primitive): canvas_primitive_handle;
  /**
   * This pane's price scale by id (reference `IPaneApi.priceScale`): `"left"`/`"right"` for the
   * built-ins, `""` for overlay, or a host-created pane-local named scale. Unknown IDs throw.
   */
  price_scale(id: string): price_scale_api;
}

export type alert_price_scale = "right" | "left" | "overlay";
export type alert_condition =
  | "crossing"
  | "crossing_up"
  | "crossing_down"
  | "greater_than"
  | "less_than";
/**
 * Alert evaluation frequency retained by the chart for visual fidelity. Regular live-price
 * alerts normally use `only_once` or `every_time`; interval-dependent hosts may also expose the
 * per-bar and per-minute modes. Aeris does not evaluate or deliver alerts.
 */
export type alert_frequency =
  | "only_once"
  | "every_time"
  | "once_per_bar"
  | "once_per_bar_close"
  | "once_per_minute";
export type alert_line_status = "active" | "triggered" | "expired";

/** Host-authoritative alert indicator rendered by the shared engine frame. */
export interface alert_line {
  id: string;
  pane_index?: number;
  price_scale?: alert_price_scale;
  price: number;
  condition?: alert_condition;
  frequency?: alert_frequency;
  status?: alert_line_status;
  /** Optional host metadata. The chart's axis tag always renders the formatted `price`. */
  label?: string;
}

export interface alert_snapshot {
  lines?: alert_line[];
}

/** Exact chart location emitted when the crosshair's multipurpose action button is clicked. */
export interface crosshair_action_request {
  sequence: number;
  pane_index: number;
  price_scale_id: string;
  price: number;
}

export type crosshair_action_request_handler = (request: crosshair_action_request) => void;

/**
 * Alert presentation boundary. The host owns dialogs, persistence, condition evaluation,
 * expiration, notifications, and server/background delivery; Aeris owns the backend-neutral
 * line indicators. The multipurpose crosshair action button belongs to {@link chart_api}.
 */
export interface alert_api {
  apply_snapshot(snapshot: alert_snapshot): void;
  state(): Required<alert_snapshot>;
  update_line(line: alert_line): void;
  remove_line(id: string): boolean;
}

export type position_side = "long" | "short";
export type order_side = "buy" | "sell";
export type order_kind = "market" | "limit" | "stop" | "stop_limit";
export type order_role = "working" | "stop_loss" | "take_profit";
export type order_status =
  | "pending_submit"
  | "working"
  | "pending_modify"
  | "partially_filled"
  | "filled"
  | "pending_cancel"
  | "cancelled"
  | "rejected"
  | "expired";
export type trading_price_scale = "right" | "left" | "overlay";

export interface instrument_metadata {
  tick_size?: number;
  price_precision?: number;
  quantity_precision?: number;
  minimum_quantity?: number;
  point_value?: number;
  currency?: string;
}

export interface trading_position {
  id: string;
  account_id?: string;
  pane_index?: number;
  price_scale?: trading_price_scale;
  side: position_side;
  average_price: number;
  quantity: number;
  display_pnl?: number;
  currency?: string;
  annotations?: trading_annotation[];
}

export type trading_annotation_tone = "neutral" | "info" | "warning" | "danger";
export type trading_annotation_placement = "inline" | "above" | "below";
export interface trading_annotation {
  id: string;
  text: string;
  tone?: trading_annotation_tone;
  tooltip?: string;
  placement?: trading_annotation_placement;
}

export interface working_order {
  id: string;
  account_id?: string;
  pane_index?: number;
  price_scale?: trading_price_scale;
  side: order_side;
  kind: order_kind;
  role?: order_role;
  status: order_status;
  price: number;
  stop_price?: number;
  trailing_trigger_price?: number;
  break_even_trigger_price?: number;
  quantity: number;
  filled_quantity?: number;
  position_id?: string;
  parent_order_id?: string;
  bracket_id?: string;
  oco_group_id?: string;
  revision?: number;
  annotations?: trading_annotation[];
}

export interface trading_execution {
  id: string;
  account_id?: string;
  pane_index?: number;
  price_scale?: trading_price_scale;
  side: order_side;
  kind: "entry" | "partial_fill" | "exit";
  time: number;
  price: number;
  quantity: number;
  order_id?: string;
  position_id?: string;
  /**
   * Mark drawn outside the bar that contains `time`: buys below its rendered low, sells above
   * its rendered high (the plotted line for line-type series). Fills of one side on one bar
   * share one mark (an arrow with stacked chevrons when there are several) that clears a line
   * series across its width; hovering it marks each fill's exact price. Defaults to `"arrow"`.
   */
  marker_shape?: "circle" | "arrow" | "triangle";
  size_by_quantity?: boolean;
}

export interface trading_round_trip {
  id: string;
  entry_execution_id: string;
  exit_execution_id: string;
  result_label: string;
  outcome: "profit" | "loss" | "flat";
}

export interface host_event_marker {
  id: string;
  time: number;
  importance?: number;
  label: string;
  icon?: string;
}
export interface host_time_window {
  id: string;
  start_time: number;
  end_time: number;
  label?: string;
}
export interface host_overlay_snapshot {
  events?: host_event_marker[];
  windows?: host_time_window[];
}
export interface host_event_hit {
  id: string;
  window: boolean;
}

/** Token shape of a timeline mark (default `"circle"`). */
export type timeline_glyph_shape = "circle" | "square" | "diamond" | "pin";

export interface timeline_mark_glyph {
  shape?: timeline_glyph_shape;
  /** CSS color of the token fill (default: the primary brand color). */
  color?: string;
  /** At most two characters printed inside a single-mark token; may be empty. */
  letter?: string;
}

/**
 * One engine-owned timeline mark: a glyph token in the lane along the bottom of the primary
 * series' pane at the bar slot that holds `time` (a time inside a gap lands on the next bar; a
 * future time projects into the right-side whitespace by whole bar steps).
 */
export interface timeline_mark {
  /** Unique id, 1..=128 UTF-8 bytes. */
  id: string;
  /** Unix seconds, like bar times. */
  time: number;
  /** Group id (1..=128 bytes); it may name a group missing from `groups` (label = id). */
  group: string;
  glyph?: timeline_mark_glyph;
  /** Tooltip title (at most 128 bytes). */
  title?: string;
}

export interface timeline_mark_group {
  id: string;
  /** Tooltip label (at most 64 bytes); the id when empty. */
  label?: string;
}

/** At most 4096 marks and 64 distinct groups; ids unique; letters at most two characters. */
export interface timeline_marks_snapshot {
  marks?: timeline_mark[];
  groups?: timeline_mark_group[];
}

/** The lane token under a point or behind a click: its anchor slot and every mark it folds. */
export interface timeline_mark_hit {
  /** Anchor bar slot (logical index; past the last bar when `projected`). */
  logical: number;
  /** Time of the earliest mark in the token. */
  time: number;
  projected: boolean;
  count: number;
  /** Distinct group ids in mark order. */
  groups: string[];
  mark_ids: string[];
  /** Title of the earliest mark. */
  title: string;
  /** Group label of a single-group token, or `"N marks"` for a mixed one. */
  label: string;
}

export type timeline_mark_click_handler = (hit: timeline_mark_hit) => void;

/** The engine-owned timeline-mark lane; see `docs/Public_api.md` "Timeline marks". */
export interface timeline_marks_api {
  /** Replace marks and groups atomically; throws `invalid_data`/`resource_limit`. */
  set(snapshot: timeline_marks_snapshot): void;
  state(): Required<timeline_marks_snapshot>;
  /** Show or hide the whole lane (default shown). */
  set_visible(visible: boolean): void;
  /** Hide or show one group's marks; persists with the chart state. Returns whether it changed. */
  set_group_hidden(group: string, hidden: boolean): boolean;
  hidden_groups(): string[];
  /** The token under chart-content CSS px `(x, y)` or `null`. */
  hit_at(x: number, y: number): timeline_mark_hit | null;
}

/**
 * A linked-chart crosshair position. `pane_index` selects the pane and `price` is a price on that
 * pane's default price scale (the scale its crosshair label reads: the first visible non-overlay
 * series' scale, else the right scale), not on the price scale of whichever series a host used to
 * place it. `time` is an exact bar timestamp. Applying it puts the horizontal line at that price
 * inside the requested pane, held on the pane's edge when the price is outside its visible range.
 */
export interface crosshair_sync_position {
  time: number;
  price: number;
  pane_index: number;
}
export type chart_sync_event =
  | { source: string; revision: number; kind: "crosshair"; position: crosshair_sync_position }
  | { source: string; revision: number; kind: "clear_crosshair" }
  | { source: string; revision: number; kind: "visible_time_range"; range: { from: number; to: number } };

export interface trading_snapshot {
  instrument?: instrument_metadata;
  positions?: trading_position[];
  orders?: working_order[];
  executions?: trading_execution[];
  round_trips?: trading_round_trip[];
}

export interface trading_hit {
  object_type: "position" | "order" | "execution";
  id: string;
  kind:
    | "position_line"
    | "order_line"
    | "take_profit_button"
    | "stop_loss_button"
    | "cancel_button"
    | "execution_marker"
    | "annotation";
  distance: number;
  annotation_id?: string;
}

export type trading_intent_action =
  | "place_bracket_order"
  | "modify_order"
  | "cancel_order"
  | "create_stop_loss"
  | "create_take_profit"
  | "close_position";

export interface trading_intent {
  sequence: number;
  action: trading_intent_action;
  account_id?: string;
  drawing_id?: number;
  order_id?: string;
  position_id?: string;
  pane_index?: number;
  price_scale?: trading_price_scale;
  side?: order_side;
  kind?: order_kind;
  role?: order_role;
  price?: number;
  price_tick_index?: number;
  stop_price?: number;
  take_profit_price?: number;
  stop_loss_price?: number;
  quantity?: number;
  bracket_id?: string;
  oco_group_id?: string;
  base_revision: number;
}

export interface trading_preview {
  source: "order" | "stop_loss" | "take_profit" | "order_stop_loss" | "order_take_profit";
  order_id?: string;
  position_id?: string;
  pane_index: number;
  price_scale: trading_price_scale;
  price: number;
  quantity: number;
  side: order_side;
  role: order_role;
  base_revision: number;
}

export type trading_intent_handler = (intent: trading_intent) => void;

export interface trading_style_options {
  position: string;
  working_order: string;
  buy: string;
  sell: string;
  profit: string;
  risk: string;
  take_profit: string;
  stop_loss: string;
  pending: string;
  rejected: string;
  control: string;
  label: string;
  /** Execution arrow colors; default blue buy and red sell. */
  execution_buy: string;
  execution_sell: string;
}

/** First-party, broker-neutral runtime trading state. Live objects are never chart-persisted. */
export interface trading_api {
  apply_snapshot(snapshot: trading_snapshot): void;
  state(): Required<trading_snapshot>;
  set_visible_account(account_id: string | null): void;
  set_host_overlay(overlay: host_overlay_snapshot): void;
  host_overlay(): host_overlay_snapshot;
  host_event_hit_at(x: number, y: number): host_event_hit | null;
  update_position(position: trading_position): void;
  remove_position(id: string): boolean;
  update_order(order: working_order): void;
  remove_order(id: string): boolean;
  apply_execution(execution: trading_execution): void;
  remove_execution(id: string): boolean;
  set_instrument(instrument: instrument_metadata): void;
  apply_options(options: Partial<trading_style_options>): void;
  /** Emit one atomic host-authoritative bracket request from a complete Long/Short Position
   * drawing. The host supplies quantity, broker submission, IDs, and the resulting snapshot. */
  place_bracket_order(drawing_id: number, quantity: number): void;
  hit_at(x: number, y: number): trading_hit | null;
  /** The live drag preview, or `null` when no drag is in flight. A released change is already
   * applied to the chart's own state, so nothing lingers here waiting on the host. */
  preview(): trading_preview | null;
  take_intents(): trading_intent[];
  /** Answer an emitted intent. Accepting releases the rollback the chart kept; rejecting undoes
   * the change — restoring a closed order or position, or moving a dragged line back. */
  resolve_intent(sequence: number, accepted: boolean): boolean;
  subscribe_intents(handler: trading_intent_handler): void;
  unsubscribe_intents(handler: trading_intent_handler): void;
}

/** Immutable diagnostic snapshot for backend selection and fallback. */
export interface backend_status {
  readonly requested_backend: "auto" | "canvas2d";
  readonly active_backend: "webgpu" | "canvas2d";
  readonly stage:
    | "backend_selection"
    | "adapter_acquisition"
    | "device_acquisition"
    | "surface_configuration"
    | "initialization"
    | "ready"
    | "runtime";
  readonly reason:
    | "canvas2d_requested"
    | "adapter_unavailable"
    | "device_unavailable"
    | "surface_unavailable"
    | "webgpu_initialization_failed"
    | "webgpu_ready"
    | "device_lost"
    | "surface_acquisition_failed";
  /** Browser secure-context state at chart construction, or `null` when the host cannot expose it. */
  readonly secure_context: boolean | null;
  /** Whether `navigator.gpu` was exposed at construction, without making another adapter request. */
  readonly navigator_gpu: boolean | null;
  /** Unstable platform detail for debugging. Branch on `stage` and `reason`, not this text. */
  readonly detail?: string;
}

/** Visible-range volume profile. OHLCV volume is spread uniformly across each bar's
 * low/high range; these bins estimate activity and are not exact trade-at-price data. */
export interface volume_profile_indicator_options {
  /** Price rows, 1-512. Default 48. */
  rows: number;
  /** Contiguous volume coverage around POC, >0-100. Default 70. */
  value_area_percent: number;
  /** Width as a percentage of the pane, >0-50. Default 25. */
  width_percent: number;
  visible: boolean;
  show_poc: boolean;
  show_value_area: boolean;
  up_color: string;
  down_color: string;
  value_area_up_color: string;
  value_area_down_color: string;
  poc_color: string;
}
export interface volume_profile_indicator_snapshot {
  rows: readonly { low: number; high: number; volume: number; up_volume: number; down_volume: number }[];
  total_volume: number;
  bar_count: number;
  poc: number | null;
  value_area_low: number | null;
  value_area_high: number | null;
  calculation_revision: number;
  error: string | null;
  method: "ohlcv_uniform";
}
export interface volume_profile_indicator_api {
  readonly id: number;
  options(): volume_profile_indicator_options;
  apply_options(options: Partial<volume_profile_indicator_options>): void;
  snapshot(): volume_profile_indicator_snapshot;
  /** Whether the profile is the chart's selection (click-to-select, cleared by Escape). */
  selected(): boolean;
  /** Select the profile, or clear it with `false`; selecting clears other chart selections. */
  select(selected?: boolean): void;
  remove(): void;
}

/** The chart. Create with {@link create_chart}. */
export interface chart_api {
  /** Format a time with the chart's time zone, date pattern, and crosshair time formatter. */
  format_time_label(value: time): string;
  /** Camel-case aliases; both naming styles operate on this same chart handle. */
  addSeries: chart_api["add_series"];
  removeSeries: chart_api["remove_series"];
  addPane: chart_api["add_pane"];
  addAxis: chart_api["add_axis"];
  removeAxis: chart_api["remove_axis"];
  applyOptions: chart_api["apply_options"];
  timeScale: chart_api["time_scale"];
  priceScale: chart_api["price_scale"];
  takeScreenshot: chart_api["take_screenshot"];
  exportState: chart_api["export_state"];
  importState: chart_api["import_state"];
  /** Active pane backend: `webgpu` when available, otherwise the shared `canvas2d` fallback. */
  backend(): "webgpu" | "canvas2d";
  /** Structured backend selection/fallback diagnostics for this chart. */
  backend_status(): Readonly<backend_status>;
  /**
   * Render telemetry for the last frame — the surface for holding a frame-time budget and
   * detecting regressions. Cheap enough to poll every frame: the returned object is freshly
   * built but the underlying transfer reuses one scratch array per chart, and the engine's
   * collection is a fixed-size record rather than a retained history.
   *
   * The first call arms WebGPU GPU-time collection; see {@link frame_stats.gpu_ms}.
   */
  frame_stats(): frame_stats;
  /** The chart-local first-party trading domain. Broker state remains host-authoritative. */
  trading(): trading_api;
  /** The engine-owned timeline-mark lane along the bottom of the primary series' pane. */
  timeline_marks(): timeline_marks_api;
  /** Fire once per click on a lane token with the resolved hit; the popup is host UI. */
  subscribe_timeline_mark_click(handler: timeline_mark_click_handler): void;
  unsubscribe_timeline_mark_click(handler: timeline_mark_click_handler): void;
  /** Host-authoritative price-alert indicators. */
  alerts(): alert_api;
  /** Show or hide the neutral crosshair action button. */
  set_crosshair_action_button_visible(visible: boolean): void;
  /** Subscribe to requests for a host-owned action menu at the crosshair price. */
  subscribe_crosshair_action(handler: crosshair_action_request_handler): void;
  unsubscribe_crosshair_action(handler: crosshair_action_request_handler): void;
  /** The singleton accessibility controller installed for this chart. */
  accessibility(): accessibility_handle;
  /**
   * Return every live series as one engine-owned snapshot and one WASM transfer. With no argument,
   * each series resolves its own latest non-whitespace row. With an index, lookup is exact: gaps
   * and whitespace remain present with null data and never borrow a neighboring value.
   */
  value_snapshot(logical_index?: number): chart_value_snapshot[];
  /** Set or clear the shared UTC-second anchor used by percentage/indexed comparison overlays. */
  set_comparison_anchor(time: number | null): boolean;
  /** Current shared comparison anchor, or `null` when unset. */
  comparison_anchor(): number | null;
  /** Engine-owned per-series values resolved against the shared comparison anchor. */
  comparison_legend_snapshot(): comparison_legend_entry[];
  /** Create or reuse a chart-level canonical trade stream for tape-derived studies. */
  add_trade_stream(key: string, options?: Partial<footprint_series_options>): number;
  /** Create or reuse the canonical bounded level-two book for one host instrument. */
  add_depth_stream(key: string, options?: Partial<depth_options>): number;
  depth_stream_id(key: string): number | null;
  remove_depth_stream(stream_id: number): boolean;
  /** Atomically replace a depth book with an exact sequenced snapshot. */
  set_depth_snapshot_typed(stream_id: number, columns: depth_snapshot_columns): void;
  /** Apply an atomic ordered batch; a sequence gap fences mutation and requests host resync. */
  update_depth_typed(stream_id: number, columns: depth_update_columns): void;
  depth_ladder(stream_id: number, levels_per_side: number, minimum_size?: number, max_distance_ticks?: number): readonly depth_ladder_row[] | null;
  depth_study(stream_id: number, levels_per_side: number, minimum_size?: number, max_distance_ticks?: number): depth_study_snapshot | null;
  add_depth_heatmap(stream_id: number, options: Partial<depth_heatmap_options> & Pick<depth_heatmap_options, "price_min" | "price_max">): number;
  remove_depth_heatmap(id: number): boolean;
  set_depth_events_typed(stream_id: number, columns: depth_event_columns): void;
  add_depth_event_layer(stream_id: number, options?: Partial<depth_event_layer_options>): number;
  remove_depth_event_layer(id: number): boolean;
  time_and_sales(stream_id: number, options?: Partial<time_and_sales_options>): readonly time_and_sales_row[] | null;
  replay_clock_micros(): number | null;
  set_replay_clock_micros(clock_micros: number | null): replay_clock_stats;
  /** Configure one candlestick/bar series as the chart's exclusive non-time price transform. */
  configure_synthetic_bar_series(series: series_api | number, options: synthetic_bar_options): void;
  /** Replace the transform's canonical OHLC source through the columnar boundary. */
  set_synthetic_bar_source_typed(series: series_api | number, columns: ohlc_columns): void;
  /** Object-row convenience wrapper over {@link set_synthetic_bar_source_typed}. */
  set_synthetic_bar_source(series: series_api | number, data: readonly series_data[]): void;
  /** Append or replace the current canonical source bar, then refresh the synthetic sequence. */
  update_synthetic_bar_source(series: series_api | number, data: ohlc_data): void;
  trade_stream_id(key: string): number | null;
  trade_stream_revision(stream_id: number): number | null;
  trade_stream_stats(stream_id: number): trade_stream_stats | null;
  trade_stream_replay_clock_micros(stream_id: number): number | null;
  set_trade_stream_replay_clock_micros(stream_id: number, clock_micros: number | null): replay_seek_stats;
  set_trade_stream_trades(stream_id: number, trades: readonly footprint_trade[]): void;
  set_trade_stream_trades_typed(stream_id: number, columns: footprint_trade_columns): void;
  update_trade_stream_trades(stream_id: number, trades: readonly footprint_trade[]): "tip" | "historical";
  update_trade_stream_trades_typed(stream_id: number, columns: footprint_trade_columns): "tip" | "historical";
  bind_footprint_series_to_stream(series: footprint_series_api | number, stream_id: number): void;
  /**
   * Present one canonical trade stream as ordinary candlesticks or OHLC bars.
   *
   * The series becomes engine-owned and read-only: feed the trade stream, not the series. Host
   * `set_data`, `update`, `merge`, their typed forms, `pop`, and `set_ring_source` are rejected
   * with `code: "derived_series"` (see {@link series_api.last_ingestion_diagnostics}) and change
   * nothing; styling, pane moves, visibility, and `histogram_updown_rule` still work. It throws
   * `invalid_options` for a series that is not a candlestick or bar, carries a `max_points` cap
   * (retention follows the stream), or is already written by another engine feature (a
   * footprint, a CVD, delta, or volume study, or resampled or synthetic bars). Rebinding to
   * another stream is allowed.
   */
  bind_trade_bar_series_to_stream(series: series_api | number, stream_id: number): void;
  /**
   * Cumulative volume delta study of the stream. Engine-owned and read-only like a bound
   * candle: feed the trade stream. Styling, pane moves, visibility, and the study's own
   * `max_points` still work.
   */
  add_cvd_series(stream_id: number, pane?: number, reset?: "session" | "continuous" | "anchored", anchor_timestamp_micros?: number): series_api;
  /**
   * Delta histogram of the stream's bars. Engine-owned and read-only like a bound candle: feed
   * the trade stream. Styling, pane moves, visibility, and the study's own `max_points` still
   * work.
   */
  add_delta_series(stream_id: number, pane?: number): series_api;
  add_trade_bubbles(series: series_api | number, stream_id: number, options?: { minimum_volume?: number; max_markers?: number; aggregation_window_micros?: number }): void;
  /**
   * Anchor a time-bar trade stream to exchange-local session windows in the chart's `time_zone`
   * and `session_start`: each window restarts the bar grid (A-share 60-minute bars open at 09:30,
   * 10:30, 13:00 and 14:00), and `outside` places auction, lunch, and after-hours prints. `null`
   * restores the plain `anchor_seconds` grid. The stream rebuilds once and every dependent follows;
   * changing the chart's time zone or session start re-places the windows.
   */
  set_trade_stream_sessions(stream_id: number, sessions: trade_session_options | null): void;
  /**
   * Volume histogram derived from the stream's bars (total traded volume per bar), tinted by the
   * primary price series' direction (`histogram_updown`); restyle it like any histogram. It is
   * engine-owned and read-only like a bound candle: feed the trade stream. Styling, pane moves,
   * visibility, `histogram_updown_rule`, and the study's own `max_points` still work.
   */
  add_trade_volume_series(stream_id: number, pane?: number): series_api;
  /**
   * Derive `target` (a candlestick or bar series) and an optional volume histogram from a source
   * series by engine resampling; both follow every source change, refreshing only the affected
   * tail. The targets are engine-owned: host writes to them are rejected (see
   * {@link series_api.last_ingestion_diagnostics}). Reconfigure to extend the boundaries, e.g. when a
   * new trading date starts; source rows outside every boundary are omitted.
   */
  configure_resampled_series(target: series_api | number, options: resample_series_options): void;
  /** The derived bars of a resampled series, or `null` when it is not resampled. */
  resampled_bars(target: series_api | number): readonly resampled_bar[] | null;
  resample_stats(target: series_api | number): resample_stats | null;
  add_series(kind: "footprint", options?: Partial<any_series_options> & Partial<footprint_series_options>): footprint_series_api;
  add_series(kind: general_series_kind, options: general_series_options): general_series_api;
  add_series(kind: series_kind, options?: Partial<any_series_options>): series_api;
  /**
   * Add a custom series (plugin platform Phase C-c; reference `IChartApi.addCustomSeries`): a
   * user-defined series type rendered by the pane view's `render(ctx)` through backend-neutral
   * draw commands, so its output is pixel-identical on the WebGPU and Canvas2D backends. The
   * engine owns the time mapping and autoscale (via the view's `price_value_builder`); the
   * returned handle's `set_data`/`update`/`data` work on the raw plugin items. The view's
   * `default_options` merge under the caller's `options` (reference `createCustomSeriesDefinition`);
   * the series options that make sense for a plugin-drawn series apply (`visible`,
   * `price_scale_id`/overlay, `pane`/`move_to_pane`, `last_value_visible`, the `price_line_*`
   * family, `price_format`) and unsupported style keys are ignored. Removing the series fires
   * the view's `destroy` hook.
   */
  /** @experimental Custom-series lifecycle is supported but its exact type surface is not frozen. */
  add_custom_series(pane_view: custom_series_pane_view, options?: Partial<series_options>): series_api;
  /**
   * Remove a series (and any indicators derived from it). No-op for an already-removed or
   * foreign handle. The primary series (the first one created, engine id 0) may also be removed;
   * the engine tombstones it safely.
   */
  remove_series(series: series_api | general_series_api): void;
  /** General series in stable paint, legend, hit-test, and persistence order (bottom first). */
  general_series_order(pane?: number): general_series_api[];
  /** Atomically reorder every general series in the selected pane, or globally when omitted. */
  set_general_series_order(ordered: general_series_api[], pane?: number): boolean;
  /**
   * The chart's series in stable saved (z-)order (bottom first), as live handles. A series
   * whose handle the package no longer tracks is omitted. Cf. the reference's per-series
   * `ISeriesApi.seriesOrder`. The frame derives pane-local paint from it — default idle
   * indicators below idle drawings below ordinary price with active hover/selection/drag
   * promoted on top (indicator outputs move as one group) — without rewriting it; hit-test
   * ties break on it so promotion cannot oscillate hover.
   */
  series_order(): series_api[];
  /**
   * Override default series grouping with an explicit idle order (bottom first; cf. the
   * reference's per-series `ISeriesApi.setSeriesOrder`, elevated here to a whole-chart call).
   * Idle drawings stay below price series; active hover/selection/drag promotion still applies
   * above the explicit idle order with indicator groups moving together. Returns `false`
   * without changing anything when the engine rejects the order.
   */
  set_series_order(ordered: series_api[]): boolean;
  /** Add a Rust-native simple moving-average line derived from an existing series. */
  add_sma(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add an SMA using an explicit OHLC/aggregate input from the source series. */
  add_sma_with_source(source: series_api, input: indicator_input_source, period: number, options?: Partial<series_options>): series_api;
  /** Add a Rust-native exponential moving-average line derived from an existing series.
   *  `parameters` selects the seed convention (default SMA seed; `{ convention: "china" }` or
   *  `{ seed: "first_value" }` starts at the first bar). */
  add_ema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api;
  add_dema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api;
  add_tema(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api;
  add_smma(source: series_api, period: number, options?: Partial<series_options>): series_api;
  add_rma(source: series_api, period: number, options?: Partial<series_options>): series_api;
  add_hma(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add one five-output EMA ribbon on the source pane. Defaults to 5/10/20/50/200; the third
   *  output (EMA 20 by default) uses violet `#7d52f4`. */
  add_ema_ribbon(source: series_api, periods?: ema_ribbon_periods, options?: ema_ribbon_options): [series_api, series_api, series_api, series_api, series_api];
  /** Atomically update all EMA ribbon periods without replacing its five series handles.
   *  `indicator` may be any output returned by {@link add_ema_ribbon}. */
  set_ema_ribbon_periods(indicator: series_api, periods: ema_ribbon_periods): boolean;
  /** Add upper, middle, and lower Rust-native Bollinger-band lines (with the industry-standard
   *  background fill between the bands). */
  add_bollinger(source: series_api, period: number, deviation?: number, options?: Partial<series_options>, parameters?: bollinger_parameters): [series_api, series_api, series_api];
  /** Add Bollinger bands using an explicit OHLC/aggregate input from the source series. */
  add_bollinger_with_source(source: series_api, input: indicator_input_source, period: number, deviation?: number, options?: Partial<series_options>, parameters?: bollinger_parameters): [series_api, series_api, series_api];
  /** Add a Rust-native Wilder RSI line in its own oscillator pane (dotted 30/70 band lines and
   *  the translucent channel strip between them). */
  add_rsi(source: series_api, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api;
  /** Add RSI using an explicit OHLC/aggregate input from the source series. */
  add_rsi_with_source(source: series_api, input: indicator_input_source, period: number, options?: Partial<series_options>, parameters?: indicator_seed_parameters): series_api;
  /** Change an existing indicator binding's scalar input while retaining its output handle. */
  set_indicator_input_source(indicator: series_api, input: indicator_input_source): boolean;
  /** Return the bounded typed editor schema for a built-in indicator kind. A `klinechart_*` kind
   *  reports the template's KLineChart default parameters (a list of periods as `period_1`, `period_2`, ...)
   *  and its output names; `period` and `deviation` are ignored for it. */
  indicator_schema(kind: indicator_kind, period?: number, deviation?: number): indicator_schema;
  /**
   * Add one of KLineChart's 27 indicator templates, with KLineChart's formulas and presentation, and
   * return its output series in output order (the keys listed on {@link klinechart_indicator}; one
   * handle per output, so `ma` returns one per period and `macd` returns `dif`, `dea`, `macd`).
   * Price templates (`ma`, `ema`, `sma`, `boll`, `sar`, `bbi`, `avp`) draw over the candles on the main pane
   * and every other template in a pane of its own. Lines use KLineChart's line palette; the `vol`, `macd`, and `ao`
   * bars and the `sar` dots are colored per row by the engine. `options` applies to every output.
   *
   * `source` is the OHLC series the formulas read, except for `avp`, whose source is a scalar series
   * holding the traded value (turnover) per bar. A scalar source series (a line, area, baseline, or histogram)
   * is accepted for every other template too and is read as open = high = low = close = its value.
   * `vol`, `obv`, `pvt`, `emv`, `vr`, and `avp` require a scalar `volume_source` distinct from
   * `source`; every other template must not be given one.
   *
   * Throws {@link AerisChartsError} `invalid_options` for an unknown template name (names are case-sensitive)
   * or any definition the engine rejects (a missing or non-whole period, a period outside 1 to 1,000,000,
   * too many periods, a `source` series that no longer exists, an OHLC `source` given to `avp`, or a
   * volume series that is missing, extra, equal to `source`, or not scalar), leaving the chart unchanged.
   */
  add_klinechart_indicator(source: series_api, indicator: klinechart_indicator, volume_source?: series_api | null, options?: Partial<series_options>): series_api[];
  /** Add MACD line, signal line, and histogram in their own oscillator pane; the histogram's
   *  per-bar color follows four conventional states (strong/weak × above/below zero). */
  add_macd(source: series_api, fast: number, slow: number, signal: number, options?: Partial<series_options>, parameters?: macd_parameters): [series_api, series_api, series_api];
  /** Add KDJ K, D, and J lines in their own oscillator pane (dotted 20/80 bands). RSV uses the last
   *  `period` bars; `K = SMA(RSV, k_smoothing, 1)`, `D = SMA(K, d_smoothing, 1)`, `J = 3K - 2D`,
   *  with K and D starting from 50 unless `parameters` selects `{ seed: "first_value" }` (or
   *  `{ convention: "china" }`), which starts at the first bar over the bars available.
   *  Defaults 9/3/3. */
  add_kdj(source: series_api, period?: number, k_smoothing?: number, d_smoothing?: number, options?: Partial<series_options>, parameters?: kdj_parameters): [series_api, series_api, series_api];
  /** Add Stochastic %K and %D lines in their own oscillator pane (dotted 20/80 band lines and
   *  the translucent channel strip between them). */
  add_stochastic(source: series_api, k_period: number, d_period: number, options?: Partial<series_options>): [series_api, series_api];
  /** Add a Rust-native Wilder ATR line in its own oscillator pane. */
  add_atr(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add a session-anchored (exchange trading-day reset; UTC by default) VWAP line on the source's pane. `volume_source`
   *  supplies per-bar volume (e.g. the volume histogram series); `null`/omitted = unit weights. */
  add_vwap(source: series_api, volume_source?: series_api | null, options?: Partial<series_options>, parameters?: vwap_parameters): series_api;
  add_obv(source: series_api, volume_source: series_api, options?: Partial<series_options>): series_api;
  add_cmf(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): series_api;
  add_mfi(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): series_api;
  add_volume(source: series_api, period: number, volume_source: series_api, options?: Partial<series_options>): [series_api, series_api];
  /** Add VWAP basis, standard-deviation bands, and percentage bands with an explicit reset. */
  add_vwap_bands(source: series_api, reset?: vwap_reset, standard_deviation?: number, percent?: number, volume_source?: series_api | null, options?: Partial<series_options>): [series_api, series_api, series_api, series_api, series_api];
  /** Create a Rust-calculated visible-range volume profile. Source must initially be OHLC;
   * volume_source must be a scalar series on this chart. Missing timestamps, whitespace,
   * nonpositive volume, and invalid price intervals contribute nothing. Source removal
   * removes the indicator. Distribution handles are runtime-only, not V1 scalar bindings. */
  add_volume_profile(source: series_api, volume_source: series_api, options?: Partial<volume_profile_indicator_options>): volume_profile_indicator_api;
  /** Add a Rust-native weighted moving-average line (linear weights, recent heaviest). */
  add_wma(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add a volume-weighted moving average. `volume_source` is optional and defaults to unit weights. */
  add_vwma(source: series_api, period: number, volume_source?: series_api | null, options?: Partial<series_options>): series_api;
  add_standard_deviation(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add a Commodity Channel Index line in its own oscillator pane with ±100 bands. */
  add_cci(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add Williams %R in its own oscillator pane with conventional -80/-20 bands. */
  add_williams_r(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add Stochastic RSI in its own oscillator pane with conventional 20/80 bands. */
  add_stochastic_rsi(source: series_api, rsi_period: number, stochastic_period: number, options?: Partial<series_options>): series_api;
  /** Add Momentum in its own oscillator pane. */
  add_momentum(source: series_api, period: number, options?: Partial<series_options>): series_api;
  /** Add Rate of Change (percentage) in its own oscillator pane. */
  add_roc(source: series_api, period: number, options?: Partial<series_options>): series_api;
  add_donchian(source: series_api, period: number, options?: Partial<series_options>): [series_api, series_api, series_api];
  /** Add previous-session pivot, R1, S1, R2, and S2 levels. */
  add_pivot_points(source: series_api, variant?: pivot_kind, options?: Partial<series_options>): [series_api, series_api, series_api, series_api, series_api];
  /** Add a ZigZag line using a minimum reversal percentage. */
  add_zigzag(source: series_api, deviation_percent?: number, options?: Partial<series_options>): series_api;
  /** Add Keltner channel upper, middle, and lower lines using an EMA center and Wilder ATR envelope. */
  add_keltner(source: series_api, period: number, multiplier?: number, options?: Partial<series_options>): [series_api, series_api, series_api];
  /** Add +DI, -DI, and ADX lines in an oscillator pane. */
  add_adx_dmi(source: series_api, period: number, options?: Partial<series_options>): [series_api, series_api, series_api];
  /** Add a conventional Parabolic SAR overlay (0.02 acceleration step, 0.20 cap). */
  add_parabolic_sar(source: series_api, options?: Partial<series_options>): series_api;
  /** Add a SuperTrend line using an ATR period and multiplier. */
  add_supertrend(source: series_api, period: number, multiplier?: number, options?: Partial<series_options>): series_api;
  add_ichimoku(source: series_api, options?: Partial<series_options>): [series_api, series_api, series_api, series_api, series_api];
  apply_options(options: deep_partial<chart_options>): void;
  options(): unknown;
  /**
   * Install the host clock (UTC seconds, fractional allowed) used by the candle-close countdown
   * instead of `Date.now()`; `null` restores the system clock. The countdown shows only while the
   * clock is inside the forming bar's interval.
   */
  set_clock(clock: (() => number) | null): void;
  time_scale(): time_scale_api;
  price_scale(price_scale_id?: string, pane_index?: number): price_scale_api;
  add_price_scale(options: price_scale_create_options, pane_index?: number): price_scale_api;
  price_scales(pane_index?: number): price_scale_info[];
  move_price_scale(id: string, side: "left" | "right", order: number, pane_index?: number): void;
  remove_price_scale(id: string, pane_index?: number): void;
  /** The stacked panes, top to bottom (roadmap Phase B1). At least one always exists. */
  panes(): pane_api[];
  /**
   * Add a stacked pane (reference `IChartApi.addPane`) and return the handle for the new (last) index.
   * `preserve_empty` keeps the pane alive while it has no series (reference `preserveEmptyPane`,
   * default `false`).
   */
  add_pane(preserve_empty?: boolean): pane_api;
  /** Add a pane bound to an explicit engine-owned horizontal domain. */
  add_pane(options: general_pane_options): pane_api;
  add_axis(options: general_axis_options): general_axis_api;
  axis(id: string): general_axis_api | null;
  axes(pane?: number): general_axis_api[];
  remove_axis(id: string): boolean;
  /** Add an engine-owned reference line, point, or region over general Cartesian axes. */
  add_general_reference(options: general_reference_options): general_reference_api;
  /** General references in stable insertion order, optionally filtered to one pane. */
  general_references(pane?: number): general_reference_api[];
  /**
   * Engine-owned legend metadata in stable series order. Hidden series remain present with
   * `visible: false`; pass a pane index to filter without changing chart state.
   */
  general_legend_snapshot(pane?: number): general_legend_snapshot;
  /**
   * Engine-owned cross-series tooltip values for the exact horizontal datum of `series`/`row`.
   * Hidden series and other panes are excluded. Returns `null` for a stale series/row.
   */
  general_shared_tooltip(series: number | general_series_api, row: number): general_shared_tooltip_snapshot | null;
  /**
   * Create/replace the transient engine-owned range brush for a Cartesian general axis.
   * X coordinates are pane plot-local CSS pixels; Y coordinates are chart CSS pixels.
   * The engine immediately converts them to semantic axis values, so resize/zoom reprojects the band.
   */
  set_general_brush(
    axis: string | general_axis_api,
    from_coordinate: number,
    to_coordinate: number,
  ): general_brush_snapshot;
  general_brush_snapshot(): general_brush_snapshot | null;
  clear_general_brush(): void;
  /** Engine-owned exact hit when `max_distance` is omitted, nearest hit otherwise. */
  general_hit_test(pane: number, x: number, y: number, max_distance?: number): general_series_hit | null;
  /** The mark selected by the most recent primary click/tap in a general-domain pane. */
  general_selected_hit(): general_series_hit | null;
  /**
   * Remove the pane at `index` (reference `IChartApi.removePane`). Returns `false` without changing
   * anything when the engine refuses (e.g. an out-of-range index or a non-empty last pane). An
   * empty preserved last pane is retired and replaced by a fresh default pane so the chart retains
   * one layout slot. Divergence: reference returns `void`. Live pane handles follow index shifts;
   * a handle for the removed pane becomes explicitly invalid and can never retarget a replacement.
   */
  remove_pane(index: number): boolean;
  /**
   * Swap the panes at `first` and `second` (reference `IChartApi.swapPanes`). Returns `false` without
   * changing anything when the engine rejects the swap. Divergence: reference returns `void`. Pane
   * live handles follow their pane identities across the swap.
   */
  swap_panes(first: number, second: number): boolean;
  /**
   * Restore Aeris-owned chart and series styling to the canonical defaults for this chart's
   * selected theme. This does not reset the view: data, panes, drawings, indicators, series
   * visibility/metadata, price formatting, scale bindings/ranges/modes/margins, zoom, and scroll
   * position are preserved. Semantic follow/unset states are restored instead of pinning effective
   * theme colors.
   */
  reset_style_to_defaults(): void;
  /**
   * industry-standard "reset view" in one action: the time scale returns to its configured
   * defaults (reference `resetTimeScale`) and every pane's price scales re-enable autoscale
   * (reference pane `resetPriceScale`, the price-axis double-click). A manually contracted or
   * over-zoomed scale fits the data again on the next frame.
   */
  reset_view(): void;
  /**
   * Chart-content `y` (CSS px from the top of the stacked pane area, see {@link pane_geometry}) for
   * a price on the top pane's (pane 0) default price scale. To convert on another pane or scale
   * use that series' {@link series_api.price_to_coordinate}. `null` when the scale has no range yet.
   */
  price_to_coordinate(price: number): number | null;
  /**
   * Price for a chart-content `y` on the default price scale of the pane containing `y` (the scale
   * its crosshair label reads): a separator belongs to the pane above and a `y` below the panes to
   * the last pane. `null` when that scale has no range yet.
   */
  coordinate_to_price(y: number): number | null;
  /**
   * Set the crosshair position within the chart (reference `IChartApi.setCrosshairPosition`). The
   * crosshair normally follows the user's cursor; setting it explicitly is useful to synchronise
   * the crosshairs of two separate charts. `time` accepts the same forms as data times.
   * Divergence: reference throws on an unknown series; here the call is a silent no-op when the
   * position cannot be applied. The line is placed through `series`' own price scale; the sync
   * event it queues carries a price on the pane's default scale (see {@link crosshair_sync_position}).
   */
  set_crosshair_position(price: number, time: time, series: series_api): void;
  /** Clear the crosshair position within the chart (reference `IChartApi.clearCrosshairPosition`). */
  clear_crosshair_position(): void;
  /**
   * Read the semantic crosshair state for linked-chart coordinators: the pane under the crosshair
   * (a separator counts as the pane above) and the price on that pane's default scale.
   */
  crosshair_sync_position(): crosshair_sync_position | null;
  /** Apply a coordinator-provided crosshair without generating local pointer input. */
  apply_external_crosshair(position: crosshair_sync_position | null): void;
  /** Drain bounded semantic synchronization events; callers route them with source/revision. */
  take_sync_events(): chart_sync_event[];
  /** Fire on every crosshair move (and once with `point: null` when the cursor leaves). */
  subscribe_crosshair_move(handler: mouse_event_handler): void;
  unsubscribe_crosshair_move(handler: mouse_event_handler): void;
  /** Fire on a click/tap inside the pane. */
  subscribe_click(handler: mouse_event_handler): void;
  unsubscribe_click(handler: mouse_event_handler): void;
  /** Fire once for a secondary click inside a pane. Hosts own any menu or action UI. */
  subscribe_chart_context(handler: chart_context_handler): void;
  unsubscribe_chart_context(handler: chart_context_handler): void;
  /** Fire on a double-click inside the pane (the default fit-content action still runs). */
  subscribe_dbl_click(handler: dbl_click_handler): void;
  unsubscribe_dbl_click(handler: dbl_click_handler): void;
  /**
   * Fire when a series appears on the chart: {@link chart_api.add_series},
   * {@link chart_api.add_custom_series}, and every output of
   * {@link chart_api.add_sma}/{@link chart_api.add_ema}/{@link chart_api.add_bollinger} (one
   * event per output, so an added indicator is observable the moment it exists). Pair with
   * {@link series_api.indicator_info} to tell indicator outputs apart from plain series.
   */
  subscribe_series_added(handler: series_change_handler): void;
  unsubscribe_series_added(handler: series_change_handler): void;
  /**
   * Fire when a series leaves the chart via {@link chart_api.remove_series} — one event per
   * tombstoned series, so removing a source series also reports its derived indicator outputs.
   */
  subscribe_series_removed(handler: series_change_handler): void;
  unsubscribe_series_removed(handler: series_change_handler): void;
  /**
   * Fire after {@link chart_api.apply_options} applies a patch, receiving that patch. This is
   * the retokening signal for platform chrome that follows chart options — e.g. the split-grid
   * divider tracking `rightPriceScale.borderColor` live.
   */
  subscribe_options_change(handler: options_change_handler): void;
  unsubscribe_options_change(handler: options_change_handler): void;
  /**
   * Add a drawing (engine-owned drawing object) to a pane (default 0) from its defining anchor
   * points and repaint. Anchors may be `{logical, price}` or `{time, price}` (UTC seconds).
   * Returns the live handle. Throws `invalid_data` when the engine rejects the placement (stale
   * pane, wrong anchor count for the kind, non-finite anchors) and `invalid_options` for a
   * malformed or out-of-range options patch.
   */
  add_drawing(kind: drawing_kind, points: drawing_point_input[], options?: Partial<drawing_options>, pane_index?: number): drawing_api;
  /**
   * Rewrite the anchors of many drawings atomically as ONE undo step (every update is validated
   * first). Returns the number of drawings that changed.
   */
  set_drawings_points(updates: readonly drawing_points_update[]): number;
  /**
   * Rescale drawing prices for a data price-basis switch (for example 前复权 ↔ 不复权): each
   * anchor whose time falls in a segment is multiplied by that segment's factor (Long/Short
   * Position levels use the entry's segment; on tick/volume/range bar charts an anchor's time is
   * the open time of its bar). This is a data-basis change, not an edit: it also
   * applies to locked drawings, rewrites the undo/redo history in the new basis, and records no
   * undo step. `price_basis` (when given) sets the basis label in the same step; `""` clears it.
   * Atomic: throws `invalid_data` with nothing changed for invalid segments or when a factor would
   * move any price outside the supported value range. Returns the number of drawings that changed.
   */
  rescale_drawing_prices(segments: readonly drawing_price_segment[], price_basis?: string): number;
  /**
   * Label the price basis the drawings use (host-defined, e.g. `"qfq"` or `"raw"`; at most 128
   * bytes; `null` clears it). The label is persisted and carried by sync/clipboard payloads so a
   * host can detect a mismatch on restore.
   */
  set_drawing_price_basis(basis: string | null): void;
  drawing_price_basis(): string | null;
  /**
   * Persistent chart drawing magnet (the toolbar magnet, default `"off"`): `"weak"` snaps anchor
   * placement and editing to the nearest OHLC value only within a small pixel distance,
   * `"strong"` always snaps. A drawing's own `magnet` option can raise it for that drawing, and
   * holding Ctrl/Cmd toggles the effective magnet temporarily. Touch input uses this mode.
   */
  set_drawing_magnet_mode(mode: drawing_magnet_mode): void;
  drawing_magnet_mode(): drawing_magnet_mode;
  /** Every drawing as live handles, in z-order (bottom first). */
  drawings(): drawing_api[];
  /** Return the typed common property schema for a live drawing. */
  drawing_property_schema(drawing: drawing_api | number): drawing_property_schema;
  drawing_kind_options(drawing: drawing_api | number): drawing_kind_options;
  /** Return the object-tree snapshot as stable JSON-compatible records. */
  drawing_object_tree(): unknown[];
  /** Set host-supplied interval metadata used by interval visibility. */
  set_drawing_interval(interval: drawing_interval | null): void;
  /**
   * Copy selected or explicit drawings into a payload bounded like a persisted drawing document.
   * Throws `resource_limit` past that bound and `invalid_data` when no listed drawing exists.
   */
  copy_drawings(ids?: readonly number[]): string;
  /** Paste a payload into a pane and return newly allocated drawing handles. */
  paste_drawings(payload: string, pane_index?: number, logical_offset?: number, price_offset?: number): drawing_api[];
  /** Clone one drawing with a semantic anchor offset. */
  clone_drawing(drawing: drawing_api | number, logical_offset?: number, price_offset?: number): drawing_api;
  /** Move a drawing by a bounded relative z-order delta. */
  move_drawing_z_order(drawing: drawing_api | number, delta: number): boolean;
  set_drawing_group_visibility(group_id: string, visible: boolean): number;
  set_drawing_group_locked(group_id: string, locked: boolean): number;
  move_drawing_group(group_id: string, logical_delta: number, price_delta: number): number;
  /**
   * Apply or export a typed style template. A template carries style only: never the name, group,
   * revision, visibility, lock, z-order, interval visibility, price scale, or text, whether it was
   * exported or written by the host.
   */
  drawing_template(drawing: drawing_api | number, name: string): drawing_template;
  apply_drawing_template(drawing: drawing_api | number, template: drawing_template): void;
  /** Export/import revisioned cross-cell drawing payloads. */
  drawing_sync_payload(source: string): string;
  apply_drawing_sync_payload(payload: string): boolean;
  /** Export V1 financial state or V2 general state; neither includes financial market data or runtime caches. */
  export_state(): chart_state;
  /** Validate and atomically restore V1 into a fresh chart. Throws {@link AerisChartsError} on failure. */
  import_state(state: chart_state | string): persistence_restore_result;
  /** Remove every drawing (the "clear all" action) and repaint. */
  clear_drawings(): void;
  /** Undo one committed drawing create/delete/move/style operation in this chart only. */
  undo_drawing(): boolean;
  /** Redo one previously undone drawing operation in this chart only. */
  redo_drawing(): boolean;
  /** Whether this chart currently has a drawing operation to undo. */
  can_undo_drawing(): boolean;
  /** Whether this chart currently has a drawing operation to redo. */
  can_redo_drawing(): boolean;
  /**
   * Arm an interactive drawing tool (industry-standard), or disarm with `null`. While armed,
   * pane clicks place the tool's anchors through the engine's creation flow — one click for the
   * single-anchor kinds, two for `trend_line`/`rectangle`, and repeated clicks for `path` until
   * double-click or Enter — the mouse previews the pending anchor, Backspace removes the latest
   * path vertex, and Escape cancels. `options` templates the drawing created this way. One-shot:
   * the tool disarms after each commit (listen with {@link chart_api.set_drawing_tool_listener}
   * to sync a toolbar).
   */
  set_drawing_tool(
    tool: drawing_kind | null,
    options?: Partial<drawing_options>,
    pane_index?: number,
  ): void;
  /** The armed interactive tool, or `null`. */
  active_drawing_tool(): drawing_kind | null;
  /** Register a listener for armed-tool changes (including the auto-disarm after a commit). */
  set_drawing_tool_listener(listener: ((tool: drawing_kind | null) => void) | null): void;
  /** Additive drawing-tool state subscription used by toolbar/controller features. */
  subscribe_drawing_tool_change(handler: drawing_tool_change_handler): void;
  unsubscribe_drawing_tool_change(handler: drawing_tool_change_handler): void;
  /** Fire after an engine-owned interactive drawing is committed. */
  subscribe_drawing_created(handler: drawing_created_handler): void;
  unsubscribe_drawing_created(handler: drawing_created_handler): void;
  /** The currently selected drawing (click-to-select; Delete/Backspace removes it), or `null`. */
  selected_drawing(): drawing_api | null;
  /** Fire after the visible logical range changes. */
  subscribe_visible_logical_range_change(handler: visible_logical_range_handler): void;
  unsubscribe_visible_logical_range_change(handler: visible_logical_range_handler): void;
  /** Fire after the visible time range changes. */
  subscribe_visible_time_range_change(handler: visible_time_range_handler): void;
  unsubscribe_visible_time_range_change(handler: visible_time_range_handler): void;
  /** Manually set the CSS size (and optional devicePixelRatio). Ignored while `autoSize` is on. */
  resize(width: number, height: number, dpr?: number): void;
  /** Force a repaint. Normally unnecessary — mutating calls repaint themselves. */
  render(): void;
  /**
   * Snapshot the composed pane and axis layers at their current device-pixel resolution.
   * `add_top_layer: false` composites the pane only (no axis/input overlay);
   * `include_crosshair: false` hides the crosshair for the capture (both default `true`).
   */
  take_screenshot(add_top_layer?: boolean, include_crosshair?: boolean): HTMLCanvasElement;
  /** Whether the `autoSize` option is enabled and active (reference `autoSizeActive`). */
  auto_size_active(): boolean;
  /** The container element passed to {@link create_chart} (reference `chartElement`). */
  chart_element(): HTMLElement;
  /**
   * Idempotently dispose the chart: stop scheduling, detach listeners/extensions, remove canvases,
   * release per-chart GPU state, and explicitly free the underlying WASM chart. Later operations
   * throw a disposed-state error; retaining this JavaScript object does not retain a live engine.
   */
  remove(): void;
}
