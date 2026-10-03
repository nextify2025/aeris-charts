import type {
  chart_api,
  feature_brush_style,
  series_api,
} from "./types.js";
import { attach_native_brushable_area } from "./impl.js";

/**
 * Optional overrides for the brushable area. Every unset field keeps the engine default, which uses
 * the same fill strength as ordinary area and baseline series.
 */
export interface brushable_area_interaction_options {
  base_style?: Partial<feature_brush_style>;
  /** Overrides for the de-emphasized area outside the selection. */
  faded_style?: Partial<feature_brush_style>;
  /** Shared overrides for both selected states. */
  selected_style?: Partial<feature_brush_style>;
  positive_style?: Partial<feature_brush_style>;
  negative_style?: Partial<feature_brush_style>;
}

export interface brushable_area_interaction_handle {
  active_range(): import("./primitive_features.js").delta_tooltip_active_range | null;
  clear(): void;
  detach(): void;
}

/**
 * Compose Delta Tooltip range selection with an ordinary Area series. While attached, primary
 * pane-drag belongs to the comparison brush instead of canvas pan; axis gestures remain ordinary.
 * The brush is transient presentation state, so the Area series keeps its normal data, hit
 * testing, scales, LOD, and ingestion behavior.
 */
export function enable_brushable_area_interaction(
  chart: chart_api,
  series: series_api,
  options: brushable_area_interaction_options = {},
): brushable_area_interaction_handle {
  if (series.series_type() !== "area") {
    throw new Error("enable_brushable_area_interaction requires an area series");
  }
  const native = attach_native_brushable_area(series, JSON.stringify({
    outside: { ...options.base_style, ...options.faded_style },
    positive: { ...options.selected_style, ...options.positive_style },
    negative: { ...options.selected_style, ...options.negative_style },
  }));
  let detached = false;
  const clear = (): void => {
    if (detached) return;
    native.clear();
  };
  return {
    active_range: () => JSON.parse(native.active_range_json()) as import("./primitive_features.js").delta_tooltip_active_range | null,
    clear,
    detach() {
      if (detached) return;
      detached = true;
      native.detach();
    },
  };
}
