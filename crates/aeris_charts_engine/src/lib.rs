//! Headless Aeris chart engine.
//!
//! This crate owns chart state and behavior without depending on WASM, the DOM, WebGPU, or a
//! native windowing system. Hosts provide input and a viewport; rendering backends consume the
//! frame produced from this state. During the architecture recovery, frame construction is being
//! migrated here incrementally from `aeris_charts_wasm`.

mod alerts;
mod axis_metrics;
mod axis_primitives;
mod depth;
mod domains;
mod drawing_contract;
mod drawings;
mod exchange_time_api;
mod feature_series;
mod footprint;
mod frame;
mod general_axes;
mod general_data;
mod general_series;
mod heikin_ashi;
mod hit_test;
mod host_layout;
mod indicators;
mod interaction;
mod native_primitives;
mod synthetic_bars;
mod volume_profile;
pub use volume_profile::{
    VolumeProfileIndicatorOptions, VolumeProfileIndicatorSnapshot, MAX_VOLUME_PROFILE_INDICATORS,
};
mod ordering;
mod persistence;
#[cfg(test)]
mod price_axis_tests;
mod price_line_api;
mod price_scale_api;
mod profiles;
mod resampling;
mod series_query_api;
mod series_update_api;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tick_bar_tests;
mod time_alignment_api;
mod time_tick_marks_api;
mod trading;
mod viewport;
mod workspace;

use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};

pub use aeris_charts_indicators::{
    DeviationEstimator, IndicatorConvention, IndicatorSeed, KdjSeed, PivotKind, VwapReset,
};
pub use alerts::{
    AlertCondition, AlertCreateRequest, AlertFrequency, AlertId, AlertLine, AlertLineStatus,
    AlertPriceScale, AlertSnapshot, MAX_ALERT_LINES,
};
pub use depth::{
    DepthBook, DepthBucket, DepthError, DepthEventCluster, DepthEventKind, DepthEventLayerOptions,
    DepthHeatmapOptions, DepthLadderRow, DepthLevel, DepthMicrostructureEvent, DepthOptions,
    DepthReplayStats, DepthResyncRequest, DepthSide, DepthSnapshot, DepthStudySnapshot,
    DepthUpdate, MAX_DEPTH_BATCH_UPDATES, MAX_DEPTH_EVENT_LABEL_BYTES, MAX_DEPTH_EVENT_LAYERS,
    MAX_DEPTH_EVENT_MARKERS, MAX_DEPTH_HEATMAPS, MAX_DEPTH_HEATMAP_ROWS, MAX_DEPTH_HISTORY_BUCKETS,
    MAX_DEPTH_HISTORY_CELLS, MAX_DEPTH_LEVELS_PER_SIDE, MAX_DEPTH_REPLAY_UPDATES,
    MAX_DEPTH_STREAMS, MAX_DEPTH_STREAM_KEY_BYTES,
};
pub use domains::{
    CategoryScaleType, ContinuousScaleType, HorizontalDomain, MAX_GENERAL_HORIZONTAL_DOMAINS,
};
pub use drawing_contract::drawing_property_schema;
pub use drawing_contract::{
    drawing_levels_from_ratios, DrawingToolOptions, FIBONACCI_RATIOS, FIBONACCI_TIME_ZONES,
    MAX_DRAWING_TOOL_OPTIONS_BYTES,
};
pub use drawing_contract::{
    DrawingClipboardItem, DrawingClipboardPayload, DrawingCommonSnapshot, DrawingInterval,
    DrawingIntervalUnit, DrawingIntervalVisibility, DrawingKindOptions, DrawingLabelMetric,
    DrawingLabelOptions, DrawingLabelPosition, DrawingLevel, DrawingLineCap, DrawingMagnetMode,
    DrawingPriceSegment, DrawingPropertyDescriptor, DrawingPropertySchema, DrawingPropertyType,
    DrawingSyncPayload, DrawingTemplate, DRAWING_CONTRACT_REVISION, MAX_DRAWING_CLIPBOARD_BYTES,
    MAX_DRAWING_CLIPBOARD_POINTS, MAX_DRAWING_GROUP_BYTES, MAX_DRAWING_LABELS, MAX_DRAWING_LEVELS,
    MAX_DRAWING_NAME_BYTES, MAX_DRAWING_OBJECTS, MAX_DRAWING_PRICE_SEGMENTS, MAX_DRAWING_TEMPLATES,
    MAX_DRAWING_TEMPLATE_BYTES,
};
// B8: lines — begin
pub use drawings::kinds::lines::{DrawingStatsPosition, LineToolOptions};
// B8: lines — end
// B8: channels — begin
pub use drawings::kinds::channels::ChannelToolOptions;
// B8: channels — end
// B8: fibonacci — begin
pub use drawings::kinds::fibonacci::{
    FibonacciLabelHAlign, FibonacciLabelVAlign, FibonacciToolOptions,
};
// B8: fibonacci — end
// B8: pitchforks_gann — begin
pub use drawings::kinds::pitchforks_gann::{GannToolOptions, MAX_GANN_SQUARE_BARS};
// B8: pitchforks_gann — end
// B8: projection_annotations — begin
pub use drawings::kinds::projection_annotations::{
    BarsPatternMode, DrawingIcon, ProjectionAnnotationToolOptions, MAX_BARS_PATTERN_BARS,
};
// B8: projection_annotations — end
// B8: patterns_elliott_cycles — begin
pub use drawings::kinds::patterns_elliott_cycles::{ElliottWaveDegree, PatternToolOptions};
// B8: patterns_elliott_cycles — end
// B8: shapes — begin
pub use drawings::kinds::shapes::ShapeToolOptions;
// B8: shapes — end
pub use drawings::{
    Drawing, DrawingAnchor, DrawingCreationUpdate, DrawingDragPart, DrawingHit, DrawingId,
    DrawingKind, DrawingModifiers, DrawingPoint, DrawingPriceScale, DrawingTextEditLayout,
    DrawingWorkStats, TextMeasureFn, DRAWING_DEFAULT_COLOR, DRAWING_WEAK_MAGNET_DISTANCE,
};
pub(crate) use drawings::{
    DrawingAnchorTime, DrawingChartSettings, DrawingController, DrawingDrag, DrawingHistory,
    DrawingRuntime, DrawingTextEdit,
};
pub use feature_series::{
    FeatureDataPoint, FeatureSeriesKind, FeatureSeriesOptionsPatch, FeatureValue, HeatmapCell,
    StackedAreaColor,
};
pub use footprint::{
    AggressorSide, BarSequence, BarSequenceMapping, BarSequencePoint, CumulativeDeltaReset,
    FootprintAggregationOptions, FootprintAggregator, FootprintBar, FootprintBarAggregation,
    FootprintCellMode, FootprintError, FootprintImbalanceOptions, FootprintLevel,
    FootprintSeriesOptions, FootprintTrade, FootprintUpdateKind, FootprintVisualOptions,
    FootprintWorkStats, ReplayClockStats, ReplaySeekStats, TimeAndSalesOptions, TimeAndSalesRow,
    TradeBubbleOptions, TradeSessionOptions, TradeStreamStats, TradeStudyKind, TradeStudyOptions,
    MAX_TIME_AND_SALES_ROWS, MAX_TRADE_STREAMS, MAX_TRADE_STREAM_KEY_BYTES,
};
pub use frame::{
    AxisBand, AxisFrame, AxisIcon, AxisLabel, AxisLabelCorners, AxisRotatedLabel, AxisTextAlign,
    AxisTextMidpoint, ChartFrame, FrameBuildStats, FrameDrawingSegment, FramePane,
    FramePaneSegments, FrameSeriesSegment,
};
pub use general_axes::{
    AxisDimension, AxisPosition, GeneralAxis, GeneralAxisDomain, GeneralAxisOptions,
    GeneralAxisTick, GeneralScaleType, MAX_GENERAL_AXES, MAX_GENERAL_AXIS_CATEGORIES,
    MAX_GENERAL_AXIS_CATEGORY_BYTES, MAX_GENERAL_AXIS_ID_BYTES, MAX_GENERAL_AXIS_TICKS,
    MAX_GENERAL_AXIS_TICK_BYTES, MAX_GENERAL_AXIS_TITLE_BYTES, MAX_GENERAL_TEMPORAL_MILLISECONDS,
};
#[doc(hidden)]
pub use general_data::{
    GeneralDataset, GeneralDatasetId, GeneralRowId, GeneralRowIdentity, GeneralXKind,
    GeneralXyInput, MAX_GENERAL_DATASETS, MAX_GENERAL_DATASET_CATEGORIES,
    MAX_GENERAL_DATASET_CATEGORY_BYTES, MAX_GENERAL_DATASET_ROWS, MAX_GENERAL_ROW_ID_BYTES,
    MAX_GENERAL_ROW_ID_BYTES_TOTAL,
};
#[doc(hidden)]
pub use general_series::{
    GeneralAccessibilityItem, GeneralAccessibilitySnapshot, GeneralBrushRange,
    GeneralBrushSnapshot, GeneralHitMode, GeneralInterpolation, GeneralLegendItem,
    GeneralLegendSnapshot, GeneralLineStyle, GeneralPointSymbol, GeneralReference,
    GeneralReferenceId, GeneralReferenceOptions, GeneralReferenceValue, GeneralSeries,
    GeneralSeriesHit, GeneralSeriesId, GeneralSeriesKind, GeneralSeriesOptions,
    GeneralSharedTooltipSnapshot, GeneralStackMode, GeneralTooltipSnapshot,
    DEFAULT_GENERAL_FILL_OPACITY, MAX_GENERAL_ACCESSIBILITY_ITEMS, MAX_GENERAL_BRUSH_ITEMS,
    MAX_GENERAL_POINT_RADIUS, MAX_GENERAL_REFERENCES, MAX_GENERAL_SERIES,
    MAX_GENERAL_SERIES_COLOR_BYTES, MAX_GENERAL_SERIES_GROUP_ID_BYTES,
    MAX_GENERAL_SERIES_STACK_ID_BYTES, MAX_GENERAL_SERIES_TITLE_BYTES,
    MAX_GENERAL_SHARED_TOOLTIP_ITEMS, MIN_GENERAL_POINT_RADIUS,
};
pub use hit_test::{SeriesHit, SeriesHitKind};
pub(crate) use indicators::{IndicatorBinding, IndicatorChange};
pub use indicators::{
    IndicatorBindingInfo, IndicatorInputSource, IndicatorKind, IndicatorOutputDescriptor,
    IndicatorOutputStyle, IndicatorParameterDescriptor, IndicatorParameterType, IndicatorSchema,
    EMA_RIBBON_DEFAULT_COLORS, EMA_RIBBON_DEFAULT_PERIODS, INDICATOR_SCHEMA_REVISION,
};
pub use interaction::{
    pinch_zoom_scale, wheel_zoom_scale, CancelReason, ChartContext, GestureResolver, GestureState,
    GestureUpdate, GestureUpdateKind, HitProfile, InputDevice, InputEvent, InputModifiers,
    InputTarget, PointerSample, ScrollAnimation, WheelBehavior, WheelDeltaMode, WheelIntent,
    WheelSample, KINETIC_DUMPING, KINETIC_MAX_SPEED, KINETIC_MIN_MOVE, KINETIC_MIN_SPEED,
    MAX_ACTIVE_POINTERS, PINCH_ZOOM_INTENSITY, WHEEL_SCROLL_PX_PER_DELTA,
};
pub use native_primitives::{
    AccessibilityFocusOptions, AnchoredTextHorizontalAlign, AnchoredTextOptions,
    AnchoredTextVerticalAlign, BandsIndicatorOptions, DeltaTooltipActiveRange, DeltaTooltipOptions,
    DeltaTooltipPoint, ImageWatermarkOptions, NativePrimitiveId, OverlayPriceScaleOptions,
    OverlayPriceScaleSide, SessionHighlightingData, SessionHighlightingOptions, TextWatermarkLine,
    TextWatermarkOptions, TooltipOptions, TooltipSnapshot, TrendLineOptions, VerticalLineOptions,
    VolumeProfileData, VolumeProfileOptions, VolumeProfilePoint, MAX_RASTER_IMAGE_DIMENSION,
};
#[cfg(not(target_arch = "wasm32"))]
pub use persistence::PersistenceRestoreProfile;
pub use persistence::{
    PersistenceRestoreResult, ValidatedStateV1, PERSISTENCE_MAX_DOCUMENT_BYTES,
    PERSISTENCE_MAX_DRAWINGS, PERSISTENCE_MAX_INDICATORS, PERSISTENCE_MAX_PANES,
    PERSISTENCE_MAX_POINTS_PER_DRAWING, PERSISTENCE_MAX_TOTAL_POINTS, PERSISTENCE_SCHEMA_VERSION,
    PERSISTENCE_SCHEMA_VERSION_GENERAL, PERSISTENCE_SCHEMA_VERSION_STUDIES,
};
pub use profiles::{
    AnchoredVwapPoint, DevelopingValueArea, NakedProfileLevel, NakedProfileLevelKind,
    ProfileDrawingOptions, ProfileDrawingSnapshot, ProfileError, ProfileRequest,
    ProfileRowSnapshot, ProfileSnapshot, ProfileSource, TpoRequest, TpoRowSnapshot, TpoSnapshot,
    MAX_PROFILE_DEVELOPING_POINTS, MAX_PROFILE_PERIODS, MAX_PROFILE_ROWS, MAX_TPO_PERIODS,
};
pub use resampling::{
    resample_boundaries, ResampleBoundary, ResampleError, ResampleOptions, ResampleSpan,
    ResampleStats, ResampledBar, MAX_RESAMPLED_SERIES, MAX_RESAMPLE_BOUNDARIES,
};
pub use series_update_api::{SeriesBarPatch, SeriesUpdateOutcome, SeriesUpdateRejection};
pub use synthetic_bars::{
    SyntheticBar, SyntheticBarAggregator, SyntheticBarError, SyntheticBarOptions,
    SyntheticSourceBar, MAX_SYNTHETIC_BARS, MAX_SYNTHETIC_SOURCE_BARS,
};
pub use time_tick_marks_api::{
    TimeTickMark, TimeTickMarksError, MAX_TIME_TICK_LABEL_BYTES, MAX_TIME_TICK_MARKS,
};
pub use trading::{
    AccountId, ExecutionId, ExecutionKind, ExecutionMarkerShape, HostEventHit, HostEventMarker,
    HostOverlaySnapshot, HostTimeWindow, InstrumentMetadata, OrderId, OrderKind, OrderRole,
    OrderSide, OrderStatus, PositionId, PositionSide, TradingAnnotation,
    TradingAnnotationPlacement, TradingAnnotationTone, TradingExecution, TradingGroupId,
    TradingHit, TradingHitKind, TradingIntent, TradingIntentAction, TradingObjectId,
    TradingPosition, TradingPreview, TradingPreviewSource, TradingPriceScale, TradingRoundTrip,
    TradingRoundTripOutcome, TradingSnapshot, TradingStyle, TradingStyleOptions, WorkingOrder,
    MAX_HOST_EVENTS, MAX_HOST_WINDOWS, MAX_TRADING_ANNOTATIONS, MAX_TRADING_OBJECTS,
    MAX_TRADING_ROUND_TRIPS,
};
pub use workspace::{SplitDirection, Workspace, WorkspaceError, WorkspaceLayout};

use aeris_charts_core::format::price_formatter::PriceFormatter;
pub use aeris_charts_core::format::price_tick_ladder::{
    PriceTickBand, PriceTickLadder, MAX_PRICE_TICK_BANDS,
};
use aeris_charts_core::format::time_formatter::{MonthNames, DEFAULT_DATE_FORMAT};
pub use aeris_charts_core::model::data_layer::TimeAlignment;
use aeris_charts_core::model::data_layer::{
    DataLayer, DataLayerMemoryUsage, MergedTimeMapping, PointColorChannel, SeriesId, SeriesIdError,
};
use aeris_charts_core::model::data_validation::{
    sanitize_ohlc, sanitize_ohlc_styled, sanitize_point, validate_timestamp, ValidationError,
    ValidationReport,
};
use aeris_charts_core::model::magnet::CrosshairMode;
use aeris_charts_core::model::plot_list::{MismatchDirection, PlotValueIndex};
use aeris_charts_core::model::price_range::PriceRange;
use aeris_charts_core::model::range::{LogicalRange, StrictRange};
pub use aeris_charts_core::options::ChartTheme;
use aeris_charts_core::options::{chart_theme_patch, ChartOptionsStore};
pub use aeris_charts_core::scale::exchange_time::{
    ExchangeTime, ExchangeTimeError, UtcOffsetSchedule, UtcOffsetTransition,
};
use aeris_charts_core::scale::price_scale_core::{
    PriceScaleCore, PriceScaleCoreOptions, PriceScaleMargins, PriceScaleMode,
};
pub use aeris_charts_core::scale::session_slots::{
    parse_iso_date, parse_wall_clock, session_slot_times, SessionSlotConvention, SessionSlotError,
    SessionWindow, MAX_SESSION_SLOTS, MAX_SESSION_WINDOWS,
};
pub use aeris_charts_core::scale::session_slots::{
    session_window_bounds, OutOfSessionPolicy, SessionBarGrid,
};
use aeris_charts_core::scale::time_scale_core::{TimeScaleCore, TimeScaleOptions};
use aeris_charts_core::scale::time_tick_marks::TimeTickMarks;
use aeris_charts_core::TimePointIndex;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, LineType};

/// Host formatting callbacks (reference localization / `tickMarkFormatter`). Each returns `None` to fall
/// back to the built-in formatter. Boxed so the headless engine carries them without a js dependency.
pub type PriceFormatterFn = Box<dyn Fn(f64) -> Option<String>>;
pub type TickMarkFormatterFn = Box<dyn Fn(i64, u8) -> Option<String>>;
pub type TimeFormatterFn = Box<dyn Fn(i64) -> Option<String>>;

pub(crate) type SequenceProjectionColumns = (
    Vec<BarSequencePoint>,
    Vec<f64>,
    Vec<f64>,
    Vec<f64>,
    Vec<f64>,
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineMemoryUsage {
    pub data: DataLayerMemoryUsage,
    pub tick_payload_bytes: usize,
    pub tick_capacity_bytes: usize,
    pub indicator_runtime_bytes: usize,
    pub indicator_transfer_capacity_bytes: usize,
    pub retained_frame_capacity_bytes: usize,
    pub drawing_runtime_capacity_bytes: usize,
    pub feature_series_capacity_bytes: usize,
    pub footprint_capacity_bytes: usize,
    pub depth_capacity_bytes: usize,
    pub resampling_capacity_bytes: usize,
    pub native_primitive_capacity_bytes: usize,
    pub trading_capacity_bytes: usize,
    pub alert_capacity_bytes: usize,
    pub general_domain_capacity_bytes: usize,
    pub general_axis_bytes: usize,
    pub general_data_capacity_bytes: usize,
    pub general_series_capacity_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct LodWorkStats {
    pub selected_level: usize,
    pub summary_nodes: usize,
    pub raw_rows: usize,
    pub candidates: usize,
}

impl EngineMemoryUsage {
    pub fn estimated_live_bytes(self) -> usize {
        self.data.logical_payload_bytes()
            + self.tick_payload_bytes
            + self.indicator_runtime_bytes
            + self.retained_frame_capacity_bytes
            + self.drawing_runtime_capacity_bytes
            + self.feature_series_capacity_bytes
            + self.footprint_capacity_bytes
            + self.depth_capacity_bytes
            + self.resampling_capacity_bytes
            + self.native_primitive_capacity_bytes
            + self.trading_capacity_bytes
            + self.alert_capacity_bytes
            + self.general_domain_capacity_bytes
            + self.general_axis_bytes
            + self.general_data_capacity_bytes
            + self.general_series_capacity_bytes
    }
}

/// reference `PriceFormat` kind (model/series-options.ts): the built-in `price` (precision/minMove
/// decimals), `volume` (K/M/B suffixes), `percent` (% sign), or a host `custom` formatter fn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceFormatKind {
    Price,
    Volume,
    Percent,
    Custom,
}

/// Per-series price format (reference series option `priceFormat`; series-options-defaults.ts:26-30
/// defaults to `{type:'price', precision:2, minMove:0.01}`). The boxed host formatter is
/// consulted only for [`PriceFormatKind::Custom`] (reference `priceFormat.formatter`), with a `None`
/// return falling back to the built-in price formatter.
pub struct SeriesPriceFormat {
    pub kind: PriceFormatKind,
    pub precision: u32,
    pub min_move: f64,
    pub formatter: Option<PriceFormatterFn>,
    /// Aeris extension: price-band tick sizes (an exchange spread table). For the `price` kind the
    /// ladder owns label rounding and per-band precision, the axis tick grid (the LCM of the
    /// visible bands' ticks), and trading price snapping on this series' scale; `min_move` and
    /// `precision` remain the scalar fallback for autoscale padding.
    pub tick_ladder: Option<PriceTickLadder>,
}

impl Default for SeriesPriceFormat {
    fn default() -> Self {
        Self {
            kind: PriceFormatKind::Price,
            precision: 2,
            min_move: 0.01,
            formatter: None,
            tick_ladder: None,
        }
    }
}

impl SeriesPriceFormat {
    /// Whether the format still holds the reference's factory default — such a series defers to the
    /// chart-level `localization.priceFormatter`/built-in formatter, exactly like a series
    /// that never set `priceFormat`.
    pub fn is_reference_default(&self) -> bool {
        self.kind == PriceFormatKind::Price
            && self.precision == 2
            && self.min_move == 0.01
            && self.tick_ladder.is_none()
    }

    /// Reference series `base()`: the price-scale tick base is the reciprocal of `minMove`.
    #[cfg(test)]
    pub(crate) fn base(&self) -> i64 {
        aeris_charts_core::scale::price_tick_span_calculator::tick_base_for_min_move(self.min_move)
    }

    /// The price-band ladder when it governs this format (built-in `price` kind only).
    pub fn active_tick_ladder(&self) -> Option<&PriceTickLadder> {
        self.tick_ladder
            .as_ref()
            .filter(|_| self.kind == PriceFormatKind::Price)
    }

    /// Nearest price on this format's tick grid (the ladder band tick, else `min_move`).
    pub fn snap_price(&self, price: f64) -> f64 {
        match self.active_tick_ladder() {
            Some(ladder) => ladder.snap(price),
            None if self.min_move.is_finite() && self.min_move > 0.0 => {
                (price / self.min_move).round() * self.min_move
            }
            None => price,
        }
    }

    /// The grid every axis tick over raw prices `[low, high]` must lie on.
    pub(crate) fn tick_grid(&self, low: f64, high: f64) -> f64 {
        match self.active_tick_ladder() {
            Some(ladder) if low.is_finite() && high.is_finite() => ladder.grid_step(low, high),
            _ => self.min_move,
        }
    }
}

/// reference `AutoscaleInfo` (series-options.ts): a series' autoscale range in raw prices plus
/// optional pixel margins. `price_range: None` contributes no range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AutoscaleInfo {
    /// `(min, max)` raw prices.
    pub price_range: Option<(f64, f64)>,
    /// `(above, below)` extra pixel margins.
    pub margins: Option<(f64, f64)>,
}

/// reference `autoscaleInfoProvider`: receives the series' own autoscale info for the visible bars
/// (`None` when the series has no data) and returns the info that REPLACES it (`None` removes the
/// series from autoscale). Hosts must not call back into the chart from the provider.
pub type AutoscaleInfoProviderFn = Box<dyn Fn(Option<AutoscaleInfo>) -> Option<AutoscaleInfo>>;

/// Apply the chart-level `leftPriceScale`/`rightPriceScale` tick keys (reference
/// `tickMarkDensity`/`ensureEdgeTickMarksVisible`) present in `group` to one scale.
fn apply_chart_tick_mark_options(
    scale: &mut PriceScaleCore,
    group: &serde_json::Map<String, serde_json::Value>,
) {
    if let Some(density) = group
        .get("tickMarkDensity")
        .and_then(serde_json::Value::as_f64)
    {
        scale.set_tick_mark_density(density);
    }
    if let Some(visible) = group
        .get("ensureEdgeTickMarksVisible")
        .and_then(serde_json::Value::as_bool)
    {
        scale.set_ensure_edge_tick_marks_visible(visible);
    }
}

/// the reference's shared line-family default color (line/area/baseline `lineColor`, histogram `color`,
/// and the custom-series `color` — custom-series.ts `customStyleDefaults`).
pub const DEFAULT_LINE_COLOR: Color = Color::rgb(0x21, 0x96, 0xf3);

/// Hysteresis for `max_points` eviction: a series over its ceiling is trimmed back to
/// `max_points - max_points / CAP_TRIM_MARGIN_DIVISOR`, so the O(total) trim runs once per that
/// many appends instead of once per append. 32 keeps the post-trim floor within ~3% of the cap
/// (a 28,800-point 8-hour window trims every 900 bars) while making the amortized cost constant.
pub const CAP_TRIM_MARGIN_DIVISOR: usize = 32;

/// Stable machine-readable categories for failures caused by public input or handle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    Disposed,
    InvalidHandle,
    StaleHandle,
    InvalidData,
    InvalidOptions,
    UnsupportedOperation,
    SerializationError,
    PersistenceVersionError,
    ExtensionError,
    RendererPlatformError,
    ResourceLimit,
}

impl ErrorCode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Disposed => "disposed",
            Self::InvalidHandle => "invalid_handle",
            Self::StaleHandle => "stale_handle",
            Self::InvalidData => "invalid_data",
            Self::InvalidOptions => "invalid_options",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::SerializationError => "serialization_error",
            Self::PersistenceVersionError => "persistence_version_error",
            Self::ExtensionError => "extension_error",
            Self::RendererPlatformError => "renderer_platform_error",
            Self::ResourceLimit => "resource_limit",
        }
    }
}

/// Public failure with a stable category and a human-readable message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartError {
    code: ErrorCode,
    message: String,
}

impl ChartError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for ChartError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ChartError {}

/// Opaque chart-local pane identity. IDs are monotonic and never reused by a chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(NonZeroU32);

impl PaneId {
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for PaneId {
    type Error = ChartError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or_else(|| ChartError::new(ErrorCode::InvalidHandle, "pane id zero is invalid"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesKind {
    Candlestick,
    Bar,
    Line,
    Area,
    Histogram,
    Baseline,
    /// A plugin-defined series type (plugin platform Phase C-c; reference `addCustomSeries`). Its
    /// data-layer rows carry times only (whitespace-style); the host renders each item through
    /// the plugin's pane view and records the frame values the built-in chrome needs.
    Custom,
    /// An engine-owned advanced series. Unlike [`Self::Custom`], its typed data, scaling,
    /// geometry, and backend-neutral frame primitives never execute host renderer callbacks.
    Feature,
    /// A first-class tick-driven footprint / numbers-bar series. Its OHLC projection participates
    /// in shared scales and queries while the authoritative tape and clusters remain engine-owned.
    Footprint,
}

/// Horizontal extent of a series' built-in live-price line.
///
/// `Partial` is Aeris's default: draw from the tracked bar/value to the pane's right edge.
/// `Full` preserves the conventional full-pane horizontal line for hosts that explicitly want it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PriceLineExtent {
    #[default]
    Partial,
    Full,
}

impl PriceLineExtent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "partial" => Some(Self::Partial),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

/// How a `histogram_updown` column takes its direction from the chart's primary price series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistogramUpDownRule {
    /// Up when the primary's close is at or above the same bar's open (the reference volume
    /// convention, and the default).
    #[default]
    OpenClose,
    /// Up when the primary's close is at or above its previous non-whitespace close (the
    /// A-share/HK time-sharing (分时) convention). The primary's first row compares with its
    /// reference price: its explicit `baseline_value`, else its price scale's explicit
    /// `base_value` (the host's previous close), else that bar's own open.
    PreviousClose,
}

impl HistogramUpDownRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenClose => "open_close",
            Self::PreviousClose => "previous_close",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open_close" => Some(Self::OpenClose),
            "previous_close" => Some(Self::PreviousClose),
            _ => None,
        }
    }
}

/// One custom series' last-value record (Phase C-c): the plugin's current value for the item
/// (the LAST element of `priceValueBuilder`, mirroring the Close slot of the reference's
/// `[last, max, min, last]` custom plot-row mapping — get-series-plot-row-creator.ts), the bar
/// color the reference's custom barColorer resolves (data item `color` ?? series `color`), and the
/// item's UTC-seconds time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CustomSeriesLastValue {
    pub value: f64,
    pub color: Color,
    pub time: i64,
}

/// Custom-series frame values (plugin platform Phase C-c), recorded by the host each frame
/// before any layout/frame pass consumes them — the same per-frame host-recording pattern as
/// [`PrimitiveAutoscaleContribution`]. A custom series' data rows are time-only, so the
/// values the built-in chrome needs — the percentage/indexed scale anchor (reference `firstValue`),
/// the built-in last-price line, the last-value axis label — arrive here, computed host-side
/// from the plugin's `priceValueBuilder` over its stored items.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CustomSeriesFrameValues {
    /// First visible non-whitespace item's current value (reference `firstValue()`).
    pub first_value: Option<f64>,
    /// Last non-whitespace item (reference `lastValueData(true)`).
    pub last: Option<CustomSeriesLastValue>,
    /// Last non-whitespace item at or left of the visible right edge (reference
    /// `lastValueData(false)`).
    pub last_visible: Option<CustomSeriesLastValue>,
}

/// Opaque pane-local identity for a host-created price scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PriceScaleId(NonZeroU32);

impl PriceScaleId {
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for PriceScaleId {
    type Error = ChartError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        NonZeroU32::new(value).map(Self).ok_or_else(|| {
            ChartError::new(ErrorCode::InvalidHandle, "price scale id zero is invalid")
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PriceScaleSide {
    Left,
    Right,
}

/// The price scale that owns a series. Named identities are stable across side/order changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PriceScaleTarget {
    Right,
    Left,
    Overlay,
    Named(PriceScaleId),
}

pub const MAX_NAMED_PRICE_SCALES_PER_PANE: usize = 16;
pub const MAX_PRICE_SCALE_ID_BYTES: usize = 128;

/// One series primitive's autoscale contribution for the frame being built (plugin platform
/// Phase C-b; reference `ISeriesPrimitiveBase.autoscaleInfo` merged into the owning series' price
/// scale range, series.ts `_autoscaleInfoImpl`). Hosts record these between frames; the next
/// autoscale pass unions each into the owning scale's range (gated on the owning series being
/// visible with a first value, exactly like the reference's per-source autoscale gate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrimitiveAutoscaleContribution {
    /// The primitive's owning series.
    pub series: SeriesId,
    /// Pane of the owning series at record time.
    pub pane: usize,
    /// Price scale of the owning series at record time.
    pub target: PriceScaleTarget,
    /// Raw price bounds to union into the scale's range.
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceScaleInfo {
    pub id: String,
    pub side: Option<PriceScaleSide>,
    pub order: Option<usize>,
    pub visible: bool,
    pub built_in: bool,
    pub pane_index: usize,
    pub series_ids: Vec<SeriesId>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesDataPoint {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// One engine-resolved value for a comparison overlay legend. The raw series remains the
/// canonical market data; these values are derived from the chart's shared comparison anchor.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ComparisonLegendEntry {
    pub series_id: SeriesId,
    pub title: String,
    pub anchor_time: Option<i64>,
    pub anchor_value: Option<f64>,
    pub latest_time: Option<i64>,
    pub latest_value: Option<f64>,
    pub change: Option<f64>,
    pub percent_change: Option<f64>,
}

/// One live series in a chart-wide value query. Latest mode resolves `logical_index` and `time`
/// independently for each series; exact mode retains an entry with null values for gaps and
/// whitespace. All formatting is resolved by the engine through the series' current formatter.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SeriesValueSnapshot {
    pub series_id: SeriesId,
    pub kind: SeriesKind,
    pub feature_kind: Option<FeatureSeriesKind>,
    pub pane_index: usize,
    pub price_scale_id: String,
    pub logical_index: Option<i64>,
    pub time: Option<i64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub value: Option<f64>,
    pub previous_value: Option<f64>,
    pub formatted_open: Option<String>,
    pub formatted_high: Option<String>,
    pub formatted_low: Option<String>,
    pub formatted_close: Option<String>,
    pub formatted_value: Option<String>,
    pub formatted_previous_value: Option<String>,
}

// the public reference keeps selected plots visibly studded with compact handles. Sample densely enough
// that a normal-width pane shows dozens of anchors, while retaining a strict per-selection bound.
const SELECTION_ANCHOR_SPACING_CSS: f64 = 24.0;
const MAX_SELECTION_ANCHORS: usize = 128;
// A selection group is host-authored interaction metadata, not an open-ended data store. Keep the
// retained member snapshots strictly bounded even if a hostile adapter supplies arbitrary ids.
const MAX_SELECTION_MEMBERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectionAnchorMemberSnapshot {
    series: SeriesId,
    times: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectionAnchorSnapshot {
    series: SeriesId,
    members: Vec<SelectionAnchorMemberSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarsInLogicalRange {
    pub bars_before: f64,
    pub bars_after: f64,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl SeriesKind {
    pub fn from_u8(kind: u8) -> Self {
        match kind {
            1 => Self::Bar,
            2 => Self::Line,
            3 => Self::Area,
            4 => Self::Histogram,
            5 => Self::Baseline,
            6 => Self::Custom,
            7 => Self::Feature,
            8 => Self::Footprint,
            _ => Self::Candlestick,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::Candlestick => 0,
            Self::Bar => 1,
            Self::Line => 2,
            Self::Area => 3,
            Self::Histogram => 4,
            Self::Baseline => 5,
            Self::Custom => 6,
            Self::Feature => 7,
            Self::Footprint => 8,
        }
    }

    fn stores_scalar_values(self) -> bool {
        matches!(
            self,
            Self::Line | Self::Area | Self::Histogram | Self::Baseline
        )
    }
}

pub fn line_style_from_u8(style: u8) -> LineStyle {
    match style {
        1 => LineStyle::Dotted,
        2 => LineStyle::Dashed,
        // The reference's retired variants (3 LargeDashed, 4 SparseDotted) fold into their
        // renamed equivalents — 3 renders exactly what `Dashed` renders, 4 what `Dotted` does.
        3 => LineStyle::Dashed,
        4 => LineStyle::Dotted,
        _ => LineStyle::Solid,
    }
}

/// reference `CrosshairMode` from its numeric wire form (unknown values fall back to Normal).
pub fn crosshair_mode_from_u8(mode: u8) -> CrosshairMode {
    use aeris_charts_core::options::crosshair_mode as wire;
    match mode {
        wire::MAGNET => CrosshairMode::Magnet,
        wire::HIDDEN => CrosshairMode::Hidden,
        wire::MAGNET_OHLC => CrosshairMode::MagnetOhlc,
        _ => CrosshairMode::Normal,
    }
}

pub mod marker_pos {
    pub const ABOVE: u8 = 0;
    pub const BELOW: u8 = 1;
    pub const IN_BAR: u8 = 2;
    pub const AT_PRICE_TOP: u8 = 3;
    pub const AT_PRICE_BOTTOM: u8 = 4;
    pub const AT_PRICE_MIDDLE: u8 = 5;
}

pub mod marker_z_order {
    pub const NORMAL: u8 = 0;
    pub const ABOVE_SERIES: u8 = 1;
    pub const TOP: u8 = 2;
}

pub mod marker_shape {
    pub const CIRCLE: u8 = 0;
    pub const SQUARE: u8 = 1;
    pub const ARROW_UP: u8 = 2;
    pub const ARROW_DOWN: u8 = 3;
}

#[derive(Clone)]
pub struct Marker {
    pub time: i64,
    pub position: u8,
    pub shape: u8,
    pub color: Color,
    pub text: String,
    pub id: String,
    pub size: f64,
    pub price: Option<f64>,
}

#[derive(Clone)]
pub struct PriceLine {
    pub id: u32,
    pub price: f64,
    pub color: Color,
    pub width: i32,
    pub style: LineStyle,
    pub title: String,
    /// reference `lineVisible` (default true): draw the horizontal line across the pane.
    pub line_visible: bool,
    /// reference `axisLabelVisible` (default true): show the boxed label on the price axis.
    pub axis_label_visible: bool,
    /// reference `axisLabelColor` (default `''`): label background; `None` follows the line color.
    pub axis_label_color: Option<String>,
    /// reference `axisLabelTextColor` (default `''`): label text; `None` automatically selects
    /// black or white for contrast against the effective label background.
    pub axis_label_text_color: Option<String>,
}

/// Transient presentation style used by the brushable-area interaction. The source remains an
/// ordinary [`SeriesKind::Area`] series; brushing never owns or duplicates market data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushStyle {
    pub line_color: Color,
    pub top_color: Color,
    pub bottom_color: Color,
    pub line_width: f64,
}

/// Engine-owned default brush styles for one Area series; hosts override individual fields only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaBrushDefaults {
    pub outside: BrushStyle,
    pub positive: BrushStyle,
    pub negative: BrushStyle,
}

/// One logical half-open range styled by the brushable-area interaction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushRange {
    pub from: f64,
    pub to: f64,
    pub style: BrushStyle,
}

/// One fixed-value oscillator channel owned by a scalar series.
///
/// Hosts describe the semantic lower/upper levels only. Aeris owns scale conversion, the
/// translucent fill, and the canonical dotted boundary lines, so callers never receive or retain
/// renderer primitives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesThresholdRegion {
    pub lower: f64,
    pub upper: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AreaBrushState {
    pub outside: BrushStyle,
    pub ranges: Vec<BrushRange>,
}

pub struct SeriesEntry {
    pub id: SeriesId,
    pub kind: SeriesKind,
    /// reference line/area/baseline `lineColor` (and the histogram `color`). Stored verbatim as a
    /// CSS string (reference `series.options()` returns the applied string); `None` is the
    /// kind-default placeholder [`DEFAULT_LINE_COLOR`], parsed only at render time.
    pub line_color: Option<String>,
    /// Candlestick/bar up & down body colors; `None` = reference default (follows the engine UP/DOWN
    /// palette). Stored verbatim as CSS strings; parsed at render time.
    pub up_color: Option<String>,
    pub down_color: Option<String>,
    /// Candlestick wick colors per direction; `None` falls back to the body color (reference parity).
    /// Stored verbatim as CSS strings; parsed at render time.
    pub wick_up_color: Option<String>,
    pub wick_down_color: Option<String>,
    /// Candlestick border colors per direction; `None` falls back to the body color (reference parity).
    /// Stored verbatim as CSS strings; parsed at render time.
    pub border_up_color: Option<String>,
    pub border_down_color: Option<String>,
    /// Candlestick part visibility; `None` = visible (reference parity).
    pub wick_visible: Option<bool>,
    pub border_visible: Option<bool>,
    pub line_width: Option<f64>,
    /// Area fill gradient colors; `None` = engine defaults. Stored verbatim as CSS strings;
    /// parsed at render time.
    pub area_top_color: Option<String>,
    pub area_bottom_color: Option<String>,
    /// Optional transient range styling for an ordinary Area series. This is interaction/presentation
    /// state only; canonical rows, hit testing, ingestion, LOD, and scale ownership remain unchanged.
    pub(crate) area_brush: Option<AreaBrushState>,
    /// Optional fixed-value channel painted behind this line/area series.
    pub threshold_region: Option<SeriesThresholdRegion>,
    /// Tint histogram columns by the primary price series' direction. `up_color`/`down_color`
    /// override the translucent market palette of the tint.
    pub histogram_updown: bool,
    /// Which comparison decides a `histogram_updown` column's direction.
    pub histogram_updown_rule: HistogramUpDownRule,
    pub price_scale_target: PriceScaleTarget,
    pub pane_index: usize,
    pub line_type: LineType,
    pub point_markers: bool,
    pub visible: bool,
    pub baseline: Option<f64>,
    pub last_price_animation: bool,
    /// Set once a host chooses the pulse explicitly; a kind change then keeps that choice instead
    /// of adopting the new kind's default.
    pub(crate) last_price_animation_explicit: bool,
    /// reference `SeriesOptionsCommon.lastValueVisible` (series-options-defaults.ts: true): draw this
    /// series' last-value label on its price scale.
    pub last_value_visible: bool,
    /// reference `title` (series-options-defaults.ts: `''`): the series' display name. Shown as a
    /// chip in a darker shade of the label color at the front of the last-value label cluster
    /// when `title_visible` holds (industry-standard).
    pub title: String,
    /// industry-standard title-chip toggle (default true): include the series' `title` as the
    /// darker chip of the last-value cluster. The chip renders even when the price label itself
    /// is off (`last_value_visible: false`).
    pub title_visible: bool,
    /// industry-standard candle-close countdown: stack a countdown row below the price inside the
    /// last-value cluster. The canonical primary market series starts enabled; subsequently added
    /// series start disabled because a derived value does not own the market bar close. Hosts may
    /// opt any series in explicitly. Hidden when the series has no usable bar interval or the host
    /// installed no clock (`now_override`).
    pub countdown_visible: bool,
    /// Draw the built-in last-price line for this series (default true).
    pub price_line_visible: bool,
    /// reference `priceLineSource` (PriceLineSource): 0 = LastBar (default), 1 = LastVisible.
    pub price_line_source: u8,
    /// Aeris live-line extent (default `Partial`): from the tracked bar/value to the pane's right
    /// edge. `Full` preserves the conventional full-pane horizontal price line.
    pub price_line_extent: PriceLineExtent,
    /// reference `priceLineWidth` in CSS px (default 1).
    pub price_line_width: f64,
    /// reference `priceLineColor` (default `''`): a valid color controls both the built-in live
    /// line and complete last-value cluster; `None` follows the relevant bar/custom value color.
    /// The CSS string is stored verbatim (reference `series.options()` returns the applied string);
    /// it is parsed only at render time, falling back to the follow behavior when unparseable.
    pub price_line_color: Option<String>,
    /// reference `priceLineStyle` (default 1 = Dotted; the reference LineStyle numbering).
    pub price_line_style: u8,
    /// industry-standard bid/ask lines + axis chips (default OFF — platforms opt in). Values
    /// are pushed by the host via `set_bid_ask`; when visible, each side with a value draws a
    /// horizontal line across the pane and a "Bid"/"Ask"-titled chip on the series' scale.
    pub bid_ask_visible: bool,
    /// Current bid price (`None` hides the bid side).
    pub bid: Option<f64>,
    /// Current ask price (`None` hides the ask side).
    pub ask: Option<f64>,
    /// Bid line/chip color (default: the canonical primary token). Stored verbatim.
    pub bid_color: String,
    /// Ask line/chip color (default: the canonical loss token). Stored verbatim.
    pub ask_color: String,
    /// Bid/ask line width in CSS px (default 1, mirrors `price_line_width`).
    pub bid_ask_line_width: f64,
    /// Bid/ask line style (default 1 = Dotted, mirrors `price_line_style`).
    pub bid_ask_line_style: u8,
    /// reference line/area/baseline `lineStyle` (default 0 = Solid; reference LineStyle numbering).
    pub line_style: u8,
    /// reference `lineVisible` (default true): hides the line stroke; an area keeps its fill and a
    /// line series keeps only its point markers.
    pub line_visible: bool,
    /// reference `pointMarkersRadius` (default `undefined`): `None` = auto (`lineWidth / 2 + 2`,
    /// line-pane-view.ts).
    pub point_markers_radius: Option<f64>,
    /// Host-configurable crosshair marker visibility (Aeris default false).
    pub crosshair_marker_visible: bool,
    /// reference `crosshairMarkerRadius` in CSS px (default 4).
    pub crosshair_marker_radius: f64,
    /// reference `crosshairMarkerBorderColor` (default `''`): `None` follows the chart background.
    pub crosshair_marker_border_color: Option<String>,
    /// reference `crosshairMarkerBackgroundColor` (default `''`): `None` follows the bar color.
    pub crosshair_marker_background_color: Option<String>,
    /// reference `crosshairMarkerBorderWidth` in CSS px (default 2).
    pub crosshair_marker_border_width: f64,
    /// reference baseline `topFillColor1` (default `rgba(38, 166, 154, 0.28)`); `None` = reference default.
    /// Stored verbatim as a CSS string; parsed at render time.
    pub top_fill_color1: Option<String>,
    /// reference baseline `topFillColor2` (default `rgba(38, 166, 154, 0.05)`); `None` = reference default.
    /// Stored verbatim as a CSS string; parsed at render time.
    pub top_fill_color2: Option<String>,
    /// reference baseline `topLineColor` (default `rgba(38, 166, 154, 1)`); `None` = reference default.
    /// Stored verbatim as a CSS string; parsed at render time.
    pub top_line_color: Option<String>,
    /// Baseline top-quadrant line width in CSS px; `None` follows `line_width` (the reference's single
    /// baseline `lineWidth`, default 3). reference has no per-quadrant width; the option is an
    /// engine extension mirroring the quadrant colors.
    pub top_line_width: Option<f64>,
    /// Baseline top-quadrant line style (default 0 = Solid; the reference's shared `lineStyle`).
    pub top_line_style: u8,
    /// reference baseline `bottomFillColor1` (default `rgba(239, 83, 80, 0.05)`); `None` = reference
    /// default. Stored verbatim as a CSS string; parsed at render time.
    pub bottom_fill_color1: Option<String>,
    /// reference baseline `bottomFillColor2` (default `rgba(239, 83, 80, 0.28)`); `None` = reference
    /// default. Stored verbatim as a CSS string; parsed at render time.
    pub bottom_fill_color2: Option<String>,
    /// reference baseline `bottomLineColor` (default `rgba(239, 83, 80, 1)`); `None` = reference default.
    /// Stored verbatim as a CSS string; parsed at render time.
    pub bottom_line_color: Option<String>,
    /// Baseline bottom-quadrant line width; `None` follows `line_width` (see `top_line_width`).
    pub bottom_line_width: Option<f64>,
    /// Baseline bottom-quadrant line style (default 0 = Solid).
    pub bottom_line_style: u8,
    /// reference histogram `base` (default 0): the price level columns grow from.
    pub base: f64,
    /// reference area `invertFilledArea` (default false): fill above the line instead of below.
    pub invert_filled_area: bool,
    /// reference bar `openVisible` (default true): draw the open tick on OHLC bars.
    pub open_visible: bool,
    /// High-low bar mode: draw the close tick when enabled (default true). Set both
    /// `open_visible` and this flag false to render a vertical high-low bar.
    pub close_visible: bool,
    /// reference bar `thinBars` (default true): bar body width capped to the crisp line width.
    pub thin_bars: bool,
    /// Presentation-only Heikin Ashi projection. Raw OHLC remains canonical in the data layer.
    pub heikin_ashi: bool,
    pub(crate) heikin_ashi_cache: RefCell<heikin_ashi::HeikinAshiCache>,
    /// reference `priceFormat` (series-options-defaults.ts: `{type:'price', precision:2, minMove:0.01}`):
    /// drives this series' last-value label, its price-line labels, the crosshair price label
    /// when this series is the label source, and the axis ticks when it is the scale's primary
    /// source.
    pub price_format: SeriesPriceFormat,
    pub price_lines: Vec<PriceLine>,
    pub markers: Vec<Marker>,
    pub markers_auto_scale: bool,
    pub markers_z_order: u8,
    /// Tombstone flag (reference `removeSeries`). The backing slot may later hold another opaque ID.
    /// and this vector, so a removed series keeps its slot (data emptied, hidden) rather than being
    /// compacted; every other series keeps its id. Removed slots are inert in every draw/scale path
    /// because they carry no data and are not visible.
    pub removed: bool,
    /// Custom series (Phase C-c): host-recorded frame values (first/last values for the scale
    /// anchor, the last-value label, and the built-in last-price line). Refreshed per frame by
    /// the host before any layout/frame pass consumes them; unused by other kinds.
    pub custom_frame: CustomSeriesFrameValues,
    /// Engine-owned advanced-series state. Present only when `kind == SeriesKind::Feature`.
    pub(crate) feature: Option<feature_series::FeatureSeriesState>,
    /// Tick-truth footprint state. Present only when `kind == SeriesKind::Footprint`.
    pub(crate) footprint: Option<footprint::FootprintSeriesState>,
    /// First-class financial primitives whose state and geometry live in the shared engine.
    pub(crate) native_primitives: Vec<native_primitives::NativeSeriesPrimitive>,
    /// Retention ceiling: the series holds at most this many rows, oldest evicted first.
    /// `None` (the default) is unbounded — a series grows for as long as the host appends to it.
    /// See [`ChartEngine::set_series_max_points`] for the eviction schedule.
    pub max_points: Option<usize>,
    /// Rows at or after this UTC-seconds time keep their data but are not drawn. A host uses it
    /// to hand the tail of a series to another presentation (candles before a live footprint).
    pub render_before_time: Option<i64>,
    /// Line/area/baseline runs end at each exchange trading-day boundary (default false): the
    /// first drawn row of a trading day starts a new run with no connecting segment, fill, or
    /// hit area from the previous day. Engine indicator outputs with period resets (VWAP, VWAP
    /// bands, pivots) break at their reset keys without this option.
    pub break_on_trading_day: bool,
    /// Last host sequence applied by a sequence-guarded update or merge (runtime-only, O(1)).
    /// `None` accepts any sequence; a full data install clears it.
    pub(crate) update_sequence: Option<u64>,
    /// reference `autoscaleInfoProvider`: replaces this series' autoscale contribution.
    pub(crate) autoscale_info_provider: Option<AutoscaleInfoProviderFn>,
}

impl SeriesEntry {
    pub fn new(id: SeriesId, kind: SeriesKind) -> Self {
        Self {
            id,
            kind,
            line_color: None,
            up_color: None,
            down_color: None,
            wick_up_color: None,
            wick_down_color: None,
            border_up_color: None,
            border_down_color: None,
            wick_visible: None,
            border_visible: None,
            line_width: None,
            area_top_color: None,
            area_bottom_color: None,
            area_brush: None,
            threshold_region: None,
            histogram_updown: false,
            histogram_updown_rule: HistogramUpDownRule::OpenClose,
            price_scale_target: PriceScaleTarget::Right,
            pane_index: 0,
            line_type: LineType::Simple,
            point_markers: false,
            visible: true,
            baseline: None,
            // Aeris product default: line and area series pulse their last price. Hosts opt out
            // per series; every other kind stays static unless a host opts in.
            last_price_animation: Self::default_last_price_animation(kind),
            last_price_animation_explicit: false,
            // Reference-compatible defaults except for explicit Aeris product choices: the live
            // price line defaults to partial extent, and crosshair markers stay disabled until the
            // host opts in per series or indicator output.
            last_value_visible: true,
            title: String::new(),
            title_visible: true,
            countdown_visible: true,
            price_line_visible: true,
            price_line_source: 0,
            price_line_extent: PriceLineExtent::Partial,
            price_line_width: 1.0,
            price_line_color: None,
            price_line_style: 1,
            bid_ask_visible: false,
            bid: None,
            ask: None,
            bid_color: aeris_charts_core::style::DEFAULT_PRIMARY_CSS.to_string(),
            ask_color: aeris_charts_core::style::MARKET_DOWN_CSS.to_string(),
            bid_ask_line_width: 1.0,
            bid_ask_line_style: 1,
            line_style: 0,
            line_visible: true,
            point_markers_radius: None,
            crosshair_marker_visible: false,
            crosshair_marker_radius: 4.0,
            crosshair_marker_border_color: None,
            crosshair_marker_background_color: None,
            crosshair_marker_border_width: 2.0,
            top_fill_color1: None,
            top_fill_color2: None,
            top_line_color: None,
            top_line_width: None,
            top_line_style: 0,
            bottom_fill_color1: None,
            bottom_fill_color2: None,
            bottom_line_color: None,
            bottom_line_width: None,
            bottom_line_style: 0,
            base: 0.0,
            invert_filled_area: false,
            open_visible: true,
            close_visible: true,
            thin_bars: true,
            heikin_ashi: false,
            heikin_ashi_cache: RefCell::new(heikin_ashi::HeikinAshiCache::default()),
            price_format: SeriesPriceFormat::default(),
            price_lines: Vec::new(),
            markers: Vec::new(),
            markers_auto_scale: true,
            markers_z_order: marker_z_order::NORMAL,
            removed: false,
            custom_frame: CustomSeriesFrameValues::default(),
            feature: None,
            footprint: None,
            native_primitives: Vec::new(),
            max_points: None,
            render_before_time: None,
            break_on_trading_day: false,
            update_sequence: None,
            autoscale_info_provider: None,
        }
    }

    pub(crate) fn default_last_price_animation(kind: SeriesKind) -> bool {
        matches!(kind, SeriesKind::Line | SeriesKind::Area)
    }

    /// Restore engine-owned visual styling without replacing the live series or its semantic/runtime
    /// state. Data, visibility, title metadata, pane/scale binding, price formatting, quotes,
    /// marker payloads, indicator semantics, retention, and transient interaction state survive.
    fn reset_style_to_defaults(&mut self) {
        let defaults = Self::new(self.id, self.kind);
        self.line_color = defaults.line_color;
        self.up_color = defaults.up_color;
        self.down_color = defaults.down_color;
        self.wick_up_color = defaults.wick_up_color;
        self.wick_down_color = defaults.wick_down_color;
        self.border_up_color = defaults.border_up_color;
        self.border_down_color = defaults.border_down_color;
        self.wick_visible = defaults.wick_visible;
        self.border_visible = defaults.border_visible;
        self.line_width = defaults.line_width;
        self.area_top_color = defaults.area_top_color;
        self.area_bottom_color = defaults.area_bottom_color;
        self.histogram_updown = defaults.histogram_updown;
        self.histogram_updown_rule = defaults.histogram_updown_rule;
        self.line_type = defaults.line_type;
        self.point_markers = defaults.point_markers;
        self.last_price_animation = defaults.last_price_animation;
        self.last_price_animation_explicit = false;
        self.last_value_visible = defaults.last_value_visible;
        self.title_visible = defaults.title_visible;
        // Countdown ownership is semantic, not visual styling. A theme/style reset must never
        // turn a derived series into a market-bar countdown owner.
        self.price_line_visible = defaults.price_line_visible;
        self.price_line_source = defaults.price_line_source;
        self.price_line_extent = defaults.price_line_extent;
        self.price_line_width = defaults.price_line_width;
        self.price_line_color = defaults.price_line_color;
        self.price_line_style = defaults.price_line_style;
        self.bid_ask_visible = defaults.bid_ask_visible;
        self.bid_color = defaults.bid_color;
        self.ask_color = defaults.ask_color;
        self.bid_ask_line_width = defaults.bid_ask_line_width;
        self.bid_ask_line_style = defaults.bid_ask_line_style;
        self.line_style = defaults.line_style;
        self.line_visible = defaults.line_visible;
        self.point_markers_radius = defaults.point_markers_radius;
        self.crosshair_marker_visible = defaults.crosshair_marker_visible;
        self.crosshair_marker_radius = defaults.crosshair_marker_radius;
        self.crosshair_marker_border_color = defaults.crosshair_marker_border_color;
        self.crosshair_marker_background_color = defaults.crosshair_marker_background_color;
        self.crosshair_marker_border_width = defaults.crosshair_marker_border_width;
        self.top_fill_color1 = defaults.top_fill_color1;
        self.top_fill_color2 = defaults.top_fill_color2;
        self.top_line_color = defaults.top_line_color;
        self.top_line_width = defaults.top_line_width;
        self.top_line_style = defaults.top_line_style;
        self.bottom_fill_color1 = defaults.bottom_fill_color1;
        self.bottom_fill_color2 = defaults.bottom_fill_color2;
        self.bottom_line_color = defaults.bottom_line_color;
        self.bottom_line_width = defaults.bottom_line_width;
        self.bottom_line_style = defaults.bottom_line_style;
        self.invert_filled_area = defaults.invert_filled_area;
        self.open_visible = defaults.open_visible;
        self.close_visible = defaults.close_visible;
        self.thin_bars = defaults.thin_bars;
        self.heikin_ashi = defaults.heikin_ashi;
        self.heikin_ashi_cache = RefCell::new(heikin_ashi::HeikinAshiCache::default());

        if let Some(feature) = self.feature.as_mut() {
            feature.options.reset_style_to_defaults();
        }
        if let Some(footprint) = self.footprint.as_mut() {
            footprint.visual.reset_style_to_defaults();
        }
    }
}

/// Canonical owner of series presentation state.
///
/// Read access behaves like the former `Vec<SeriesEntry>`. Any mutable access advances the
/// revision before exposing the entries, so retained-frame invalidation cannot be bypassed by a
/// Rust host that edits a public series entry directly.
pub struct SeriesStore {
    entries: Vec<SeriesEntry>,
    revision: u64,
}

impl SeriesStore {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
    }
}

impl From<Vec<SeriesEntry>> for SeriesStore {
    fn from(entries: Vec<SeriesEntry>) -> Self {
        Self {
            entries,
            revision: 1,
        }
    }
}

impl Deref for SeriesStore {
    type Target = Vec<SeriesEntry>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl DerefMut for SeriesStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.changed();
        &mut self.entries
    }
}

impl<'a> IntoIterator for &'a SeriesStore {
    type Item = &'a SeriesEntry;
    type IntoIter = std::slice::Iter<'a, SeriesEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<'a> IntoIterator for &'a mut SeriesStore {
    type Item = &'a mut SeriesEntry;
    type IntoIter = std::slice::IterMut<'a, SeriesEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.changed();
        self.entries.iter_mut()
    }
}

/// Stable pane-boundary layout slot in CSS pixels. The visible separator rule is thinner and uses
/// the canonical design-system border width during axis-frame lowering; hover/hit geometry is also
/// independently expanded for usability.
pub const PANE_SEPARATOR: f64 = 1.0;

/// `pane_index` sentinel for a series whose pane was removed (reference `removePane` orphans the
/// pane's series — `paneForSource` turns null): the series keeps its data but renders and
/// scales nowhere until re-assigned to a live pane.
pub(crate) const PANELESS: usize = usize::MAX;

pub struct Pane {
    /// Chart-local identity that survives index changes and is never reused. Zero is reserved for
    /// standalone/default panes that are not owned by a [`ChartEngine`].
    stable_id: Option<PaneId>,
    /// Persistence identity is separate from the live handle identity. Imports preserve this
    /// value while issuing fresh live IDs, so pre-import handles become stale rather than
    /// retargeting restored panes.
    persistent_id: Option<u32>,
    /// `None` is the allocation-free binding to the chart's established financial-time domain.
    /// General panes resolve this opaque identity through the chart-owned domain registry.
    general_horizontal_domain: Option<domains::GeneralHorizontalDomainId>,
    pub price_scale: PriceScaleCore,
    pub left_scale: PriceScaleCore,
    pub overlay_scale: PriceScaleCore,
    pub(crate) named_scales: Vec<NamedPriceScale>,
    next_price_scale_id: u32,
    right_scale_order: usize,
    left_scale_order: usize,
    pub stretch_factor: f64,
    pub overlay_top: f64,
    pub overlay_bottom: f64,
    /// reference pane.ts `_preserveEmptyPane` (default false): an empty pane collapses on the next
    /// series removal/move-out unless this holds it open (chart-model.ts
    /// `_cleanupIfPaneIsEmpty`).
    pub preserve_empty: bool,
    pub marker_margin_above: f64,
    pub marker_margin_below: f64,
    pub left_marker_margin_above: f64,
    pub left_marker_margin_below: f64,
    pub overlay_marker_margin_above: f64,
    pub overlay_marker_margin_below: f64,
    pub top: f64,
    pub height: f64,
}

impl Pane {
    pub fn new() -> Self {
        Self::with_ids(None, None)
    }

    fn with_chart_ids(stable_id: PaneId, persistent_id: u32) -> Self {
        Self::with_ids(Some(stable_id), Some(persistent_id))
    }

    fn with_ids(stable_id: Option<PaneId>, persistent_id: Option<u32>) -> Self {
        let main_scale = PriceScaleCore::new(PriceScaleCoreOptions::default());
        let overlay_scale = PriceScaleCore::new(PriceScaleCoreOptions {
            scale_margins: PriceScaleMargins {
                top: 0.8,
                bottom: 0.0,
            },
            ..PriceScaleCoreOptions::default()
        });
        Self {
            stable_id,
            persistent_id,
            general_horizontal_domain: None,
            price_scale: main_scale,
            left_scale: PriceScaleCore::new(PriceScaleCoreOptions::default()),
            overlay_scale,
            named_scales: Vec::new(),
            next_price_scale_id: 1,
            right_scale_order: 0,
            left_scale_order: 0,
            stretch_factor: 1.0,
            overlay_top: 0.8,
            overlay_bottom: 0.0,
            preserve_empty: false,
            marker_margin_above: 0.0,
            marker_margin_below: 0.0,
            left_marker_margin_above: 0.0,
            left_marker_margin_below: 0.0,
            overlay_marker_margin_above: 0.0,
            overlay_marker_margin_below: 0.0,
            top: 0.0,
            height: 0.0,
        }
    }

    pub fn stable_id(&self) -> Option<PaneId> {
        self.stable_id
    }

    pub(crate) fn persistent_id(&self) -> Option<u32> {
        self.persistent_id
    }

    /// Give every scale this pane owns its own axis geometry: the scale height is the pane's
    /// slot height and its offset is the pane's top edge, so autoscale, margins, ticks, and
    /// gestures resolve inside the pane alone. Panes share no axis coordinate space; only the
    /// final pane-offset transform maps a scale coordinate into chart-content space.
    pub fn layout(&mut self) {
        let (top, height) = (self.top, self.height);
        for scale in self.scales_mut() {
            scale.set_height(height);
            scale.set_pane_offset(top);
        }
        self.refresh_internal_margins();
    }

    /// Pixel margins requested by autoscale marker providers. Pane placement is carried by the
    /// pane offset, so these are only the marker reservations.
    pub fn refresh_internal_margins(&mut self) {
        self.price_scale
            .set_internal_margins(self.marker_margin_above, self.marker_margin_below);
        self.left_scale
            .set_internal_margins(self.left_marker_margin_above, self.left_marker_margin_below);
        self.overlay_scale.set_internal_margins(
            self.overlay_marker_margin_above,
            self.overlay_marker_margin_below,
        );
        for entry in &mut self.named_scales {
            entry
                .scale
                .set_internal_margins(entry.marker_margin_above, entry.marker_margin_below);
        }
    }

    /// Every price scale this pane owns, in axis-declaration order.
    fn scales_mut(&mut self) -> impl Iterator<Item = &mut PriceScaleCore> {
        [
            &mut self.price_scale,
            &mut self.left_scale,
            &mut self.overlay_scale,
        ]
        .into_iter()
        .chain(self.named_scales.iter_mut().map(|entry| &mut entry.scale))
    }
}

pub(crate) struct NamedPriceScale {
    pub id: PriceScaleId,
    pub public_id: String,
    pub side: PriceScaleSide,
    pub order: usize,
    pub visible: bool,
    pub width: f64,
    pub marker_margin_above: f64,
    pub marker_margin_below: f64,
    pub scale: PriceScaleCore,
}

impl Pane {
    pub(crate) fn scale(&self, target: PriceScaleTarget) -> Option<&PriceScaleCore> {
        match target {
            PriceScaleTarget::Right => Some(&self.price_scale),
            PriceScaleTarget::Left => Some(&self.left_scale),
            PriceScaleTarget::Overlay => Some(&self.overlay_scale),
            PriceScaleTarget::Named(id) => self
                .named_scales
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| &entry.scale),
        }
    }

    pub(crate) fn scale_mut(&mut self, target: PriceScaleTarget) -> Option<&mut PriceScaleCore> {
        match target {
            PriceScaleTarget::Right => Some(&mut self.price_scale),
            PriceScaleTarget::Left => Some(&mut self.left_scale),
            PriceScaleTarget::Overlay => Some(&mut self.overlay_scale),
            PriceScaleTarget::Named(id) => self
                .named_scales
                .iter_mut()
                .find(|entry| entry.id == id)
                .map(|entry| &mut entry.scale),
        }
    }

    pub(crate) fn target_for_public_id(&self, id: &str) -> Option<PriceScaleTarget> {
        match id {
            "right" => Some(PriceScaleTarget::Right),
            "left" => Some(PriceScaleTarget::Left),
            "" => Some(PriceScaleTarget::Overlay),
            _ => self
                .named_scales
                .iter()
                .find(|entry| entry.public_id == id)
                .map(|entry| PriceScaleTarget::Named(entry.id)),
        }
    }

    pub(crate) fn public_id_for_target(&self, target: PriceScaleTarget) -> Option<&str> {
        match target {
            PriceScaleTarget::Right => Some("right"),
            PriceScaleTarget::Left => Some("left"),
            PriceScaleTarget::Overlay => Some(""),
            PriceScaleTarget::Named(id) => self
                .named_scales
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.public_id.as_str()),
        }
    }

    pub(crate) fn named_scale(&self, id: PriceScaleId) -> Option<&NamedPriceScale> {
        self.named_scales.iter().find(|entry| entry.id == id)
    }

    pub(crate) fn named_scale_mut(&mut self, id: PriceScaleId) -> Option<&mut NamedPriceScale> {
        self.named_scales.iter_mut().find(|entry| entry.id == id)
    }

    pub(crate) fn scale_targets(&self) -> impl Iterator<Item = PriceScaleTarget> + '_ {
        [
            PriceScaleTarget::Right,
            PriceScaleTarget::Left,
            PriceScaleTarget::Overlay,
        ]
        .into_iter()
        .chain(
            self.named_scales
                .iter()
                .map(|entry| PriceScaleTarget::Named(entry.id)),
        )
    }

    pub(crate) fn scale_revisions(&self) -> Vec<u64> {
        self.scale_targets()
            .filter_map(|target| self.scale(target).map(PriceScaleCore::revision))
            .collect()
    }

    pub(crate) fn scale_side(&self, target: PriceScaleTarget) -> Option<PriceScaleSide> {
        match target {
            PriceScaleTarget::Right => Some(PriceScaleSide::Right),
            PriceScaleTarget::Left => Some(PriceScaleSide::Left),
            PriceScaleTarget::Overlay => None,
            PriceScaleTarget::Named(id) => self.named_scale(id).map(|entry| entry.side),
        }
    }

    pub(crate) fn scale_order(&self, target: PriceScaleTarget) -> Option<usize> {
        match target {
            PriceScaleTarget::Right => Some(self.right_scale_order),
            PriceScaleTarget::Left => Some(self.left_scale_order),
            PriceScaleTarget::Overlay => None,
            PriceScaleTarget::Named(id) => self.named_scale(id).map(|entry| entry.order),
        }
    }

    fn set_scale_order(&mut self, target: PriceScaleTarget, order: usize) {
        match target {
            PriceScaleTarget::Right => self.right_scale_order = order,
            PriceScaleTarget::Left => self.left_scale_order = order,
            PriceScaleTarget::Overlay => {}
            PriceScaleTarget::Named(id) => {
                if let Some(entry) = self.named_scale_mut(id) {
                    entry.order = order;
                }
            }
        }
    }

    pub(crate) fn ordered_side_targets(&self, side: PriceScaleSide) -> Vec<PriceScaleTarget> {
        let mut targets = Vec::with_capacity(self.named_scales.len() + 1);
        targets.push(match side {
            PriceScaleSide::Left => PriceScaleTarget::Left,
            PriceScaleSide::Right => PriceScaleTarget::Right,
        });
        targets.extend(
            self.named_scales
                .iter()
                .filter(|entry| entry.side == side)
                .map(|entry| PriceScaleTarget::Named(entry.id)),
        );
        targets.sort_by_key(|target| self.scale_order(*target).unwrap_or(usize::MAX));
        targets
    }

    pub(crate) fn move_axis_target(
        &mut self,
        target: PriceScaleTarget,
        side: PriceScaleSide,
        requested_order: usize,
    ) -> bool {
        let Some(old_side) = self.scale_side(target) else {
            return false;
        };
        if matches!(target, PriceScaleTarget::Right) && side != PriceScaleSide::Right
            || matches!(target, PriceScaleTarget::Left) && side != PriceScaleSide::Left
        {
            return false;
        }
        if self.scale_order(target).is_none() {
            return false;
        }
        let mut old_targets = self.ordered_side_targets(old_side);
        old_targets.retain(|candidate| *candidate != target);
        for (order, candidate) in old_targets.into_iter().enumerate() {
            self.set_scale_order(candidate, order);
        }
        if let PriceScaleTarget::Named(id) = target {
            if let Some(entry) = self.named_scale_mut(id) {
                entry.side = side;
            }
        }
        let mut targets = self.ordered_side_targets(side);
        targets.retain(|candidate| *candidate != target);
        let order = requested_order.min(targets.len());
        targets.insert(order, target);
        for (index, candidate) in targets.into_iter().enumerate() {
            self.set_scale_order(candidate, index);
        }
        true
    }
}

impl Default for Pane {
    fn default() -> Self {
        Self::new()
    }
}

/// A linked-crosshair position. `price` is a price on `pane_index`'s default price scale (the
/// scale the crosshair label reads) and `time` an exact merged chart time. Applying it puts the
/// horizontal line at that price's chart-content y, held on the pane's edge when the price is
/// outside the pane's visible range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrosshairSyncPosition {
    pub time: f64,
    pub price: f64,
    pub pane_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibleTimeRangeSync {
    pub from: f64,
    pub to: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChartSyncEventKind {
    Crosshair { position: CrosshairSyncPosition },
    ClearCrosshair,
    VisibleTimeRange { range: VisibleTimeRangeSync },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartSyncEvent {
    pub source: String,
    pub revision: u64,
    #[serde(flatten)]
    pub kind: ChartSyncEventKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMismatchPolicy {
    #[default]
    Nearest,
    Clear,
}

/// Platform-independent state for one chart instance.
pub struct ChartEngine {
    pub time_scale: TimeScaleCore,
    pub panes: Vec<Pane>,
    pub price_formatter: PriceFormatter,
    data: DataLayer,
    /// Full-resolution temporal labels for a logical bar sequence. The data layer's integer row
    /// keys remain chart-local positions; this sidecar prevents non-time bars from being encoded
    /// as synthetic UTC timestamps.
    sequence_points: Option<Vec<BarSequencePoint>>,
    synthetic_series: HashMap<SeriesId, SyntheticBarAggregator>,
    resampled_series: HashMap<SeriesId, resampling::ResampleBinding>,
    depth_streams: HashMap<u64, DepthBook>,
    depth_stream_keys: HashMap<String, u64>,
    next_depth_stream_id: u64,
    depth_heatmaps: HashMap<u64, depth::DepthHeatmap>,
    next_depth_heatmap_id: u64,
    depth_event_layers: HashMap<u64, depth::DepthEventLayer>,
    next_depth_event_layer_id: u64,
    /// Sequence identity mapping waiting for the data-layer synchronization triggered by a
    /// non-time footprint rebuild. It is consumed before ordinary timestamp rebasing so drawing
    /// anchors follow the same full-resolution bars even when row keys are reused.
    pending_sequence_mapping: Option<BarSequenceMapping>,
    /// Persisted non-time drawing identities waiting for the host to install a matching bar
    /// sequence. Entries are bounded by the persistence anchor limits and are consumed when they
    /// resolve to a live sequence point.
    drawing_anchor_times: HashMap<DrawingId, Vec<Option<DrawingAnchorTime>>>,
    pub series: SeriesStore,
    tick_marks: TimeTickMarks,
    /// Host-supplied time-axis marks replacing the automatic selection (`None` = automatic).
    time_tick_marks: Option<Vec<time_tick_marks_api::TimeTickMark>>,
    next_pane_id: u32,
    next_persistent_pane_id: u32,
    general_horizontal_domains: domains::HorizontalDomainRegistry,
    general_axes: general_axes::GeneralAxisRegistry,
    general_data: Option<general_data::GeneralDataStore>,
    general_series: Option<general_series::GeneralSeriesRegistry>,
    pub options: ChartOptionsStore,
    theme: ChartTheme,
    pub crosshair_mode: CrosshairMode,
    /// the public reference's Ctrl-held magnet: while set, a Normal-mode crosshair snaps to the hovered
    /// bar's rendered prices exactly like `CrosshairMode::MagnetOhlc` (OHLC for candles/bars,
    /// close/value for scalar series; frame/crosshair.rs `crosshair_snap`). The gesture layer
    /// forwards the live modifier state; the configured `crosshair_mode` is untouched
    /// (Magnet/MagnetOhlc stay as configured, Hidden stays hidden).
    pub crosshair_ohlc_magnet: bool,
    pub animation_time: f64,
    pub next_price_line_id: u32,
    next_native_primitive_id: NativePrimitiveId,
    native_pane_primitives: Vec<native_primitives::NativePanePrimitive>,
    trading_state: trading::TradingState,
    alert_state: alerts::AlertState,
    /// reference `timeScale.timeVisible` — label semantics only: whether axis/crosshair time labels
    /// include the time of day. Strip reservation is [`Self::time_axis_visible`].
    pub time_visible: bool,
    /// reference `timeScale.visible` (default true): reserve and paint the whole time-axis strip.
    /// When false the strip collapses to zero height and its labels/ticks/border vanish.
    pub time_axis_visible: bool,
    /// reference `timeScale.ticksVisible` (default false): tick marks on the time axis.
    pub time_ticks_visible: bool,
    /// reference `timeScale.minimumHeight` (default 0 = the metrics-derived auto height): floor
    /// for the time-axis strip height.
    pub time_axis_minimum_height: f64,
    /// reference `timeScale.tickMarkMaxCharacterLength` (default 8): tick-label width cap in
    /// characters. 0 restores the default (the reference's `|| defaultTickMarkMaxCharacterLength`).
    pub tick_mark_max_character_length: u32,
    /// Hovered pane separator index for the hover band (reference pane-separator.ts
    /// `separatorHoverColor` handle); `None` paints nothing. Mirrored into the axis frame.
    pub separator_hover: Option<usize>,
    /// reference `timeScale.secondsVisible` — include seconds in axis/crosshair time labels when
    /// `time_visible` is set. Defaults to false (reference default).
    pub seconds_visible: bool,
    pub css_width: f64,
    pub css_height: f64,
    pub dpr: f64,
    /// Host-installed clock (UTC seconds) for the candle-close countdown rows of the last-value
    /// label clusters (industry-standard extension). The engine is headless: countdown rows stay
    /// hidden until a host supplies the time — the wasm render path feeds the browser's system
    /// time every frame unless a value is pinned; tests pin one here for determinism.
    pub now_override: Option<f64>,
    /// Host-owned replay clock. Canonical rows remain retained while the data layer and canonical
    /// trade streams expose only facts at or before this microsecond boundary.
    replay_clock_micros: Option<i64>,
    pub crosshair: Option<(f64, f64)>,
    /// Optional chart-wide time anchor used by percentage/indexed comparison overlays and their
    /// legend. The anchor is a time identity only; each series resolves its own value at that
    /// time and continues to own its canonical rows.
    comparison_anchor: Option<i64>,
    /// Host-supplied interval metadata used by drawing visibility ranges. `None` means no
    /// interval filter is active and keeps legacy drawings visible.
    pub drawing_interval: Option<DrawingInterval>,
    sync_events: VecDeque<ChartSyncEvent>,
    sync_revision: u64,
    sync_mismatch_policy: SyncMismatchPolicy,
    pub pane_w: f64,
    pub pane_h: f64,
    /// Media-coordinate x offset of the pane after reserving a visible left axis.
    pub pane_left: f64,
    pub left_axis_w: f64,
    pub axis_w: f64,
    pub(crate) left_builtin_axis_w: f64,
    pub(crate) right_builtin_axis_w: f64,
    indicators: Vec<IndicatorBinding>,
    /// During study-state restore, persisted oscillator panes are empty until their studies are
    /// recreated. The cursor lets the normal placement path reuse those panes without changing
    /// interactive study creation semantics.
    study_restore_pane_cursor: Option<usize>,
    indicator_changes: Vec<(SeriesId, IndicatorChange)>,
    /// Canonical chart-level trade streams. Footprint and future tape-derived studies refer to a
    /// stream identity instead of retaining a second provider-event tape.
    trade_streams: HashMap<u64, footprint::FootprintAggregator>,
    trade_stream_keys: HashMap<String, u64>,
    trade_bar_dependents: HashMap<u64, Vec<footprint::TradeBarDependent>>,
    trade_dependents: HashMap<u64, Vec<footprint::TradeStudyDependent>>,
    trade_bubbles: HashMap<u64, Vec<footprint::TradeBubbleDependent>>,
    next_trade_stream_id: u64,
    synced_points_len: usize,
    synced_time_points_generation: u64,
    synced_last_time: Option<i64>,
    synced_first_time: Option<i64>,
    /// reference `localization.dateFormat` (default `dd MMM \'yy`): drives the crosshair time label.
    pub date_format: String,
    /// Per-locale month-name tables (reference `localization.locale`) used by the date-format
    /// `MMM`/`MMMM` tokens and the month tick labels. Hosts inject locale-derived names (the
    /// wasm host builds them from `Intl.DateTimeFormat`); the headless default is English.
    pub month_names: MonthNames,
    /// Exchange time zone, trading-day session start, and calendar-date axis flag. Drives tick
    /// weights, built-in time labels, trading-day indicator resets, session highlighting, and
    /// the countdown window. Defaults to UTC with a midnight session start.
    pub(crate) exchange_time: ExchangeTime,
    /// Series ids in stable order, bottom to top (topmost LAST — the reference's z-order, pane.ts
    /// `orderedSources`/`setSeriesOrder`). Live series only: removed slots leave the list.
    /// This is the saved ordering: insertion order until an explicit `set_series_order`
    /// override. Frame assembly derives the pane-local paint order from it (default idle
    /// indicators below idle drawings below ordinary series, active objects on top) without
    /// rewriting it; hit testing tie-breaks on it so promotion cannot oscillate hover.
    series_order: Vec<SeriesId>,
    /// Whether `set_series_order` has overridden the default series grouping. Idle explicit
    /// order paints verbatim (drawings still below price series); active indicator groups
    /// still promote together. Never reset except by constructing a new chart.
    series_order_explicit: bool,
    /// The series under the cursor (reference `ChartModel._hoveredSource`), refreshed by hosts
    /// from their hover pipeline. When `hoveredSeriesOnTop` holds, the frame build paints
    /// this series topmost (reference `hoveredSourceOnTopOrder`) without touching `series_order`.
    hovered_series: Option<SeriesId>,
    /// The series the host last clicked plus the canonical source timestamps sampled on the
    /// unselected -> selected transition. Coordinate changes only reproject this snapshot.
    selection: Option<SelectionAnchorSnapshot>,
    /// Series-primitive autoscale contributions for the current frame build (Phase C-b).
    /// Hosts clear and re-record them per frame, before any layout/autoscale pass runs;
    /// `autoscale_for_frame` unions them into the owning scales.
    primitive_autoscale: Vec<PrimitiveAutoscaleContribution>,
    /// Engine-owned drawing objects (drawing tools: trend/horizontal/vertical lines, rectangle,
    /// Long/Short Position,
    /// text) in z-order, bottom first. See drawings.rs.
    drawings: Vec<Drawing>,
    /// Derived, chart-local drawing bounds, pane candidates, and coordinate geometry. Semantic
    /// anchors and styles in `drawings` remain authoritative and are the only serialized state.
    drawing_runtime: RefCell<DrawingRuntime>,
    /// Next chart-unique drawing id (never reused; starts at 1 — 0 is the "no drawing" sentinel).
    next_drawing_id: DrawingId,
    /// The drawing the host last clicked (industry-standard selection): while set, the frame
    /// build paints anchor handles at its defining points and its anchors accept drags.
    selected_drawing: Option<DrawingId>,
    /// Additional object-tree selections. `selected_drawing` remains the compatibility primary.
    selected_drawings: Vec<DrawingId>,
    /// Active anchor/body drag session on a drawing (drawings.rs; the interaction.rs session
    /// pattern — the engine owns the start snapshot and the math).
    drawing_drag: Option<DrawingDrag>,
    /// A merged-time rebase refreshed live interaction pixels immediately; refresh once more after
    /// the next frame's layout and autoscale settle their final coordinate transforms.
    drawing_baselines_need_frame_refresh: bool,
    /// Bounded chart-local semantic history for committed drawing mutations. Runtime-only:
    /// persistence stores the current drawings, never this stack.
    drawing_history: DrawingHistory,
    /// One engine-owned drawing-tool controller: armed tool/template, anchored placement and
    /// freehand capture. Hosts forward normalized actions and never own per-tool creation logic.
    drawing_controller: DrawingController,
    /// Last accepted cross-cell drawing sync envelope, used to reject stale revisions and echo
    /// loops without making the host coordinator stateful inside the renderer.
    drawing_sync_source: String,
    drawing_sync_revision: u64,
    /// Chart-level drawing settings (magnet mode, price-basis label) and anchor time-identity
    /// bookkeeping; drawings.rs owns every field.
    drawing_settings: DrawingChartSettings,
    /// The open inline text-edit session (drawings.rs): the drawing whose host editor owns
    /// text input, with the text it began from. Frame construction keeps committed glyphs for
    /// the transparent overlay-caret model, keeps an empty trend label's measured middle gap,
    /// and keeps an empty family text box's caret line while it is open. Runtime only.
    text_edit: Option<DrawingTextEdit>,
    /// The text drawing under the host's pointer (drawings.rs): the overlay frame paints its
    /// focus border at hover opacity (the public reference's hover ring). Only the text tool has hover
    /// chrome — other kinds show nothing until selected.
    hovered_text: Option<DrawingId>,
    /// The drawing of any kind under the host's pointer (ordering seam): drives temporary
    /// hover promotion in frame assembly alongside `hovered_series`. `hovered_text` remains
    /// the text-only ring state; this tracks every kind so overlaps stay selectable and
    /// return to stable order on hover leave. Never persisted; cleared like other hover.
    hovered_drawing: Option<DrawingId>,
    /// Optional host text-measure callback for drawing-label hit boxes (drawings.rs
    /// [`TextMeasureFn`]); without one the engine estimates widths by character count.
    text_measure_fn: Option<TextMeasureFn>,
    /// Kinetic (momentum) scroll sampler/coast for the active drag (engine interaction module,
    /// reference `KineticAnimation`); the host feeds samples and drives the coast per frame.
    kinetic: Option<aeris_charts_core::model::kinetic_animation::KineticAnimation>,
    /// Velocity-owned keyboard pan. This is separate from public `scroll_to_position(..., true)`:
    /// a held arrow receives bounded engine-timed velocity kicks with light drag; key-up stops it.
    keyboard_scroll_animation: Option<interaction::KeyboardKineticScroll>,
    /// In-flight eased scroll-to-position (engine interaction module); the host schedules the
    /// ticks, the engine owns the easing and applies each step.
    scroll_animation: Option<interaction::ScrollAnimation>,
    /// Optional host formatting callbacks (reference `localization.priceFormatter`/`timeFormatter` and
    /// `timeScale.tickMarkFormatter`). The engine stays headless — the host supplies plain boxed
    /// closures; each returns `None` to fall back to the built-in formatter (e.g. the callback
    /// threw at the boundary). Kept as trait objects, so `ChartEngine` is intentionally not
    /// `Clone`/`Debug`/`Send`.
    price_formatter_fn: Option<PriceFormatterFn>,
    tick_mark_formatter_fn: Option<TickMarkFormatterFn>,
    time_formatter_fn: Option<TimeFormatterFn>,
    frame_invalidation: frame::FrameInvalidation,
    retained_frame: frame::RetainedFrame,
    frame_build_stats: FrameBuildStats,
    lod_work: Cell<LodWorkStats>,
}

impl ChartEngine {
    pub fn new(css_width: f64, css_height: f64, dpr: f64) -> Self {
        Self::new_with_initial_domain(css_width, css_height, dpr, HorizontalDomain::FinancialTime)
            .expect("the default financial domain requires no bounded registry entry")
    }

    /// Construct the chart with its canonical first-pane domain. Financial charts retain the
    /// historical primary candlestick series; general charts start with one preserved general pane
    /// and no hidden financial series or transient financial topology.
    pub fn new_with_initial_domain(
        css_width: f64,
        css_height: f64,
        dpr: f64,
        initial_domain: HorizontalDomain,
    ) -> Result<Self, ChartError> {
        let mut general_horizontal_domains = domains::HorizontalDomainRegistry::new();
        let initial_binding = general_horizontal_domains.register(initial_domain)?;
        let mut initial_pane = Pane::with_chart_ids(PaneId(NonZeroU32::MIN), 1);
        initial_pane.general_horizontal_domain = initial_binding;
        initial_pane.preserve_empty = !initial_domain.is_financial_time();
        let mut data = DataLayer::new();
        let (series, series_order) = if initial_domain.is_financial_time() {
            let main = data.add_series();
            (
                vec![SeriesEntry::new(main, SeriesKind::Candlestick)].into(),
                vec![main],
            )
        } else {
            (Vec::<SeriesEntry>::new().into(), Vec::new())
        };
        data.begin_merged_time_transaction();
        Ok(Self {
            time_scale: TimeScaleCore::new(TimeScaleOptions::default()),
            panes: vec![initial_pane],
            price_formatter: PriceFormatter::default(),
            data,
            sequence_points: None,
            synthetic_series: HashMap::new(),
            resampled_series: HashMap::new(),
            depth_streams: HashMap::new(),
            depth_stream_keys: HashMap::new(),
            next_depth_stream_id: 1,
            depth_heatmaps: HashMap::new(),
            next_depth_heatmap_id: 1,
            depth_event_layers: HashMap::new(),
            next_depth_event_layer_id: 1,
            pending_sequence_mapping: None,
            drawing_anchor_times: HashMap::new(),
            series,
            tick_marks: TimeTickMarks::new(),
            time_tick_marks: None,
            next_pane_id: 2,
            next_persistent_pane_id: 2,
            general_horizontal_domains,
            general_axes: general_axes::GeneralAxisRegistry::new(),
            general_data: None,
            general_series: None,
            options: ChartOptionsStore::new(),
            theme: ChartTheme::default(),
            crosshair_mode: CrosshairMode::Normal,
            crosshair_ohlc_magnet: false,
            animation_time: 0.0,
            next_price_line_id: 1,
            next_native_primitive_id: 1,
            native_pane_primitives: Vec::new(),
            trading_state: trading::TradingState::default(),
            alert_state: alerts::AlertState::default(),
            time_visible: true,
            time_axis_visible: true,
            time_ticks_visible: false,
            time_axis_minimum_height: 0.0,
            tick_mark_max_character_length: 8,
            separator_hover: None,
            seconds_visible: false,
            css_width,
            css_height,
            dpr,
            now_override: None,
            replay_clock_micros: None,
            crosshair: None,
            comparison_anchor: None,
            drawing_interval: None,
            sync_events: VecDeque::new(),
            sync_revision: 0,
            sync_mismatch_policy: SyncMismatchPolicy::Nearest,
            pane_w: css_width,
            pane_h: css_height,
            pane_left: 0.0,
            left_axis_w: 0.0,
            axis_w: 0.0,
            left_builtin_axis_w: 0.0,
            right_builtin_axis_w: 0.0,
            indicators: Vec::new(),
            study_restore_pane_cursor: None,
            indicator_changes: Vec::new(),
            trade_streams: HashMap::new(),
            trade_stream_keys: HashMap::new(),
            trade_bar_dependents: HashMap::new(),
            trade_dependents: HashMap::new(),
            trade_bubbles: HashMap::new(),
            next_trade_stream_id: 1,
            synced_points_len: 0,
            synced_time_points_generation: 0,
            synced_last_time: None,
            synced_first_time: None,
            date_format: DEFAULT_DATE_FORMAT.to_string(),
            month_names: MonthNames::default(),
            exchange_time: ExchangeTime::default(),
            series_order,
            series_order_explicit: false,
            hovered_series: None,
            selection: None,
            primitive_autoscale: Vec::new(),
            drawings: Vec::new(),
            drawing_runtime: RefCell::new(DrawingRuntime::default()),
            next_drawing_id: 1,
            selected_drawing: None,
            selected_drawings: Vec::new(),
            drawing_drag: None,
            drawing_baselines_need_frame_refresh: false,
            drawing_history: DrawingHistory::default(),
            drawing_controller: DrawingController::default(),
            drawing_sync_source: String::new(),
            drawing_sync_revision: 0,
            drawing_settings: DrawingChartSettings::default(),
            text_edit: None,
            hovered_text: None,
            hovered_drawing: None,
            text_measure_fn: None,
            kinetic: None,
            keyboard_scroll_animation: None,
            scroll_animation: None,
            price_formatter_fn: None,
            tick_mark_formatter_fn: None,
            time_formatter_fn: None,
            frame_invalidation: frame::FrameInvalidation::default(),
            retained_frame: frame::RetainedFrame::default(),
            frame_build_stats: FrameBuildStats::default(),
            lod_work: Cell::new(LodWorkStats::default()),
        })
    }

    #[doc(hidden)]
    pub fn lod_work_stats(&self) -> LodWorkStats {
        self.lod_work.get()
    }

    pub(crate) fn reset_lod_work(&self) {
        self.lod_work.set(LodWorkStats::default());
    }

    pub(crate) fn record_lod_work(
        &self,
        selected_level: usize,
        summary_nodes: usize,
        raw_rows: usize,
        candidates: usize,
    ) {
        let mut work = self.lod_work.get();
        work.selected_level = work.selected_level.max(selected_level);
        work.summary_nodes += summary_nodes;
        work.raw_rows += raw_rows;
        work.candidates += candidates;
        self.lod_work.set(work);
    }

    /// Read-only access to canonical series data. All mutation must use engine commands so time,
    /// scale, indicator, retention, and invalidation invariants remain synchronized.
    pub fn data_layer(&self) -> &DataLayer {
        &self.data
    }

    pub(crate) fn axis_time_seconds_at(&self, index: usize) -> Option<f64> {
        if let Some(points) = self.sequence_points() {
            return points
                .get(index)
                .map(|point| point.open_timestamp_micros as f64 / 1_000_000.0);
        }
        self.data.merged_times().get(index).map(|&time| time as f64)
    }

    pub(crate) fn axis_time_key_at(&self, index: usize) -> Option<i64> {
        if let Some(points) = self.sequence_points() {
            return points
                .get(index)
                .map(|point| point.open_timestamp_micros.div_euclid(1_000_000));
        }
        self.data.merged_times().get(index).copied()
    }

    pub(crate) fn axis_index_for_time(&self, time: i64) -> Option<usize> {
        if self.sequence_points().is_some() {
            return self
                .time_to_index(time as f64, true)
                .and_then(|index| usize::try_from(index).ok());
        }
        let times = self.data.merged_times();
        if times.is_empty() {
            return None;
        }
        Some(
            times
                .binary_search(&time)
                .unwrap_or_else(|index| index.min(times.len() - 1)),
        )
    }

    /// Structure-level memory attribution for engineering evidence. This reports logical payload
    /// and vector capacity, not allocator metadata, committed WASM pages, or browser memory.
    pub fn memory_usage(&self) -> EngineMemoryUsage {
        let (indicator_runtime_bytes, indicator_transfer_capacity_bytes) =
            self.indicator_memory_usage();
        EngineMemoryUsage {
            data: self.data.memory_usage(),
            tick_payload_bytes: self.tick_marks.payload_bytes() + self.time_tick_marks_bytes().0,
            tick_capacity_bytes: self.tick_marks.capacity_bytes() + self.time_tick_marks_bytes().1,
            indicator_runtime_bytes,
            indicator_transfer_capacity_bytes,
            retained_frame_capacity_bytes: self.retained_frame.capacity_bytes(),
            drawing_runtime_capacity_bytes: self.drawing_runtime.borrow().capacity_bytes(),
            feature_series_capacity_bytes: self.feature_series_capacity_bytes(),
            footprint_capacity_bytes: self.footprint_capacity_bytes(),
            depth_capacity_bytes: self.depth_capacity_bytes(),
            resampling_capacity_bytes: self.resampling_capacity_bytes(),
            native_primitive_capacity_bytes: self.native_primitive_capacity_bytes(),
            trading_capacity_bytes: self.trading_state.estimated_bytes(),
            alert_capacity_bytes: self.alert_state.estimated_bytes(),
            general_domain_capacity_bytes: self.general_horizontal_domains.capacity_bytes(),
            general_axis_bytes: self.general_axes.estimated_bytes(),
            general_data_capacity_bytes: self
                .general_data
                .as_ref()
                .map_or(0, general_data::GeneralDataStore::estimated_bytes),
            general_series_capacity_bytes: self
                .general_series
                .as_ref()
                .map_or(0, general_series::GeneralSeriesRegistry::estimated_bytes),
        }
    }

    /// Staging seam for the engine-owned general column store. Browser support is not exposed until
    /// a concrete general series owns the dataset and the public package manifest includes it.
    #[doc(hidden)]
    pub fn create_general_xy_dataset(
        &mut self,
        input: GeneralXyInput,
    ) -> Result<GeneralDatasetId, ChartError> {
        let id = if let Some(store) = self.general_data.as_mut() {
            store.insert(input)?
        } else {
            let mut store = general_data::GeneralDataStore::new();
            let id = store.insert(input)?;
            self.general_data = Some(store);
            id
        };
        self.invalidate_frame_scene();
        Ok(id)
    }

    #[doc(hidden)]
    pub fn replace_general_xy_dataset(
        &mut self,
        id: GeneralDatasetId,
        input: GeneralXyInput,
    ) -> Result<(), ChartError> {
        self.replace_general_xy_dataset_labeled(id, input, None)
    }

    #[doc(hidden)]
    pub fn replace_general_xy_dataset_labeled(
        &mut self,
        id: GeneralDatasetId,
        input: GeneralXyInput,
        labels: Option<Vec<Option<String>>>,
    ) -> Result<(), ChartError> {
        let bound = self.general_series_uses_dataset(id);
        if bound {
            self.validate_general_dataset_replacement(id, &input)?;
        }
        let store = self.general_data.as_mut().ok_or_else(|| {
            ChartError::new(ErrorCode::InvalidHandle, "general dataset handle is stale")
        })?;
        if let Some(labels) = labels {
            store.replace_labeled(id, input, Some(labels))?;
        } else {
            store.replace(id, input)?;
        }
        if bound {
            self.reconcile_general_interaction_for_dataset(id, 0);
            self.invalidate_frame_all();
        } else {
            self.invalidate_frame_scene();
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn upsert_general_xy_dataset(
        &mut self,
        id: GeneralDatasetId,
        input: GeneralXyInput,
        max_rows: Option<usize>,
    ) -> Result<(), ChartError> {
        self.upsert_general_xy_dataset_labeled(id, input, None, max_rows)
    }

    #[doc(hidden)]
    pub fn upsert_general_xy_dataset_labeled(
        &mut self,
        id: GeneralDatasetId,
        input: GeneralXyInput,
        labels: Option<Vec<Option<String>>>,
        max_rows: Option<usize>,
    ) -> Result<(), ChartError> {
        let bound = self.general_series_uses_dataset(id);
        if bound {
            self.validate_general_dataset_replacement(id, &input)?;
        }
        let store = self.general_data.as_mut().ok_or_else(|| {
            ChartError::new(ErrorCode::InvalidHandle, "general dataset handle is stale")
        })?;
        let removed_front = if let Some(labels) = labels {
            store.upsert_labeled(id, input, Some(labels), max_rows)?
        } else {
            store.upsert(id, input, max_rows)?
        };
        if bound {
            self.reconcile_general_interaction_for_dataset(id, removed_front);
            self.invalidate_frame_all();
        } else {
            self.invalidate_frame_scene();
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn general_dataset(&self, id: GeneralDatasetId) -> Option<&GeneralDataset> {
        self.general_data.as_ref()?.get(id)
    }

    #[doc(hidden)]
    pub fn general_dataset_count(&self) -> usize {
        self.general_data
            .as_ref()
            .map_or(0, general_data::GeneralDataStore::len)
    }

    #[doc(hidden)]
    pub fn remove_general_dataset(&mut self, id: GeneralDatasetId) -> bool {
        if self.general_series_uses_dataset(id) {
            return false;
        }
        let Some(store) = self.general_data.as_mut() else {
            return false;
        };
        if !store.remove(id) {
            return false;
        }
        if store.is_empty() {
            self.general_data = None;
        }
        self.invalidate_frame_scene();
        true
    }

    /// Read-only time tick state derived from the canonical timestamp sequence.
    pub fn tick_marks(&self) -> &TimeTickMarks {
        &self.tick_marks
    }

    /// Read-only live/storage entries. Series mutation remains staged through existing engine
    /// commands; the public field is retained temporarily for host option APIs that still need a
    /// controlled migration.
    pub fn series_entries(&self) -> &[SeriesEntry] {
        &self.series
    }

    /// Install (or clear with `None`) the host price formatter (reference `localization.priceFormatter`).
    /// Applied to non-percentage price labels; a `None` return from the callback falls back to the
    /// built-in formatter.
    pub fn set_price_formatter(&mut self, f: Option<PriceFormatterFn>) {
        self.price_formatter_fn = f;
        self.invalidate_frame_all();
    }

    /// Install (or clear) the host time-axis tick formatter (reference `timeScale.tickMarkFormatter`).
    /// The callback receives the UTC-second timestamp and the tick-mark type (0 Year, 1 Month,
    /// 2 DayOfMonth, 3 Time, 4 TimeWithSeconds). Replacement text can change strip widths, so
    /// this invalidates layout as well as the scene.
    pub fn set_tick_mark_formatter(&mut self, f: Option<TickMarkFormatterFn>) {
        self.tick_mark_formatter_fn = f;
        self.invalidate_frame_scene();
        self.invalidate_frame_layout_and_axis();
    }

    /// Pin the engine clock (UTC seconds) used by the candle-close countdown rows of the
    /// last-value label clusters. Hosts with a ticking countdown call this on every tick;
    /// per frame the wasm render path also feeds the system time when nothing was pinned.
    pub fn set_now_seconds(&mut self, now: f64) {
        if now.is_finite() {
            let changed_second = self.now_override.map(f64::floor) != Some(now.floor());
            if changed_second {
                let previous_now = self.now_override;
                let mut countdown_active = false;
                let layout_changed = self
                    .series
                    .iter()
                    .filter(|series| series.visible && series.countdown_visible)
                    .any(|series| {
                        let previous = self.series_countdown_layout_key_at(series.id, previous_now);
                        let next = self.series_countdown_layout_key_at(series.id, Some(now));
                        countdown_active |= next.is_some();
                        previous != next
                    });
                self.now_override = Some(now);
                if layout_changed {
                    self.invalidate_frame_layout_and_axis();
                } else if countdown_active {
                    self.invalidate_frame_axis();
                }
            } else {
                self.now_override = Some(now);
            }
        }
    }

    /// Install (or clear) the host crosshair time formatter (reference `localization.timeFormatter`).
    pub fn set_time_formatter(&mut self, f: Option<TimeFormatterFn>) {
        self.time_formatter_fn = f;
        self.invalidate_frame_overlay();
    }

    /// reference `localization.dateFormat` (default `dd MMM \'yy`): the pattern driving the
    /// crosshair time label. Ignored while a host `timeFormatter` is installed (reference parity).
    pub fn set_date_format(&mut self, pattern: &str) {
        self.date_format = pattern.to_string();
        self.invalidate_frame_overlay();
    }

    /// Inject per-locale month-name tables (reference `localization.locale`): the 12 short and 12
    /// long month names used by the date-format `MMM`/`MMMM` tokens and the month tick
    /// labels. The engine stays headless — hosts derive the names (the wasm host uses
    /// `Intl.DateTimeFormat`); the default is English.
    pub fn set_month_names(&mut self, short: [String; 12], long: [String; 12]) {
        self.month_names = MonthNames { short, long };
        self.invalidate_frame_scene();
    }

    /// Add a series to the headless chart. The returned id is stable for the instance lifetime.
    pub fn add_series(&mut self, kind: SeriesKind) -> SeriesId {
        let id = self.data.add_series();
        let slot = self
            .data
            .series_slot(id)
            .expect("newly allocated series must have a storage slot");
        if slot == self.series.len() {
            self.series.push(SeriesEntry::new(id, kind));
        } else {
            self.series[slot] = SeriesEntry::new(id, kind);
        }
        // Only the canonical primary market series created with the chart owns the bar-close
        // countdown by default. Every later series is an overlay, study, companion, or an
        // explicitly host-owned source and must opt in if it truly owns a market interval.
        self.series[slot].countdown_visible = false;
        if kind == SeriesKind::Footprint {
            let stream_id = self.next_trade_stream_id;
            self.next_trade_stream_id = self.next_trade_stream_id.saturating_add(1).max(1);
            self.trade_streams.insert(
                stream_id,
                footprint::FootprintAggregator::new(Default::default())
                    .expect("default footprint options must remain valid"),
            );
            self.series[slot].footprint = Some(footprint::FootprintSeriesState {
                trade_stream_id: stream_id,
                visual: Default::default(),
            });
        }
        // new series paint on top (reference appends to the pane's data sources)
        self.series_order.push(id);
        // Custom time-only rows and footprint scale-projection rows both anchor the base index.
        self.data.set_rows_count_as_data(
            id,
            matches!(kind, SeriesKind::Custom | SeriesKind::Footprint),
        );
        self.invalidate_frame_scene();
        id
    }

    /// Change a live series' kind (host `setSeriesType` and custom-series adoption, Phase C-c).
    /// Keeps the data layer's custom-series bookkeeping in sync: a Custom series' rows carry
    /// times only, yet still count as data rows for the time-scale base index.
    pub fn convert_series_kind(&mut self, id: SeriesId, kind: SeriesKind) {
        if let Some(s) = self.series.iter_mut().find(|s| s.id == id && !s.removed) {
            // A footprint is not an OHLC presentation variant. It requires raw trades plus
            // aggregation options, which `configure_footprint_series` installs atomically.
            if kind == SeriesKind::Footprint {
                return;
            }
            // The last-price pulse follows the new kind's default unless the host chose it
            // explicitly, so an opt-out or opt-in survives any chain of conversions.
            if !s.last_price_animation_explicit {
                s.last_price_animation = SeriesEntry::default_last_price_animation(kind);
            }
            s.kind = kind;
            if kind != SeriesKind::Area {
                s.area_brush = None;
            }
            if kind == SeriesKind::Candlestick {
                s.native_primitives.retain(|primitive| {
                    !matches!(
                        &primitive.kind,
                        native_primitives::NativeSeriesPrimitiveKind::DeltaTooltip(_)
                    )
                });
            }
            if kind != SeriesKind::Feature {
                s.feature = None;
            }
            if kind != SeriesKind::Footprint {
                s.footprint = None;
            }
            self.data
                .set_rows_count_as_data(id, kind == SeriesKind::Custom);
            // Host-valued kinds own no engine rows to join as-of; they rejoin the union.
            if matches!(kind, SeriesKind::Custom | SeriesKind::Feature) {
                self.rejoin_time_union(id);
            }
            self.clear_sequence_axis_if_unused();
            self.invalidate_frame_scene();
        }
    }

    /// Record a custom series' frame values (Phase C-c; hosts refresh them per frame, before
    /// the layout/autoscale passes consume them). Ignored for an unknown, removed, or
    /// non-custom id.
    pub fn set_custom_frame_values(&mut self, id: SeriesId, values: CustomSeriesFrameValues) {
        if let Some(s) = self
            .series
            .iter_mut()
            .find(|s| s.id == id && !s.removed && s.kind == SeriesKind::Custom)
        {
            s.custom_frame = values;
            self.invalidate_frame_scene();
        }
    }

    /// Remove a series (reference `removeSeries`, which accepts any series including the first).
    /// Returns false for an unknown or already-removed id. Any indicators bound to (or
    /// derived from) the series are dropped with it. Consumers that anchor on the "primary"
    /// series (crosshair defaults, the volume up/down reference, the last-price pulse, the
    /// wasm coordinate API) fall back to the first visible non-removed series.
    ///
    /// Removal permanently invalidates the opaque identity and releases its backing slot for a
    /// future series. A stale identity can therefore never alias the replacement series.
    pub fn remove_series(&mut self, id: SeriesId) -> bool {
        !self.remove_series_tracked(id).is_empty()
    }

    /// `remove_series` that reports every tombstoned id: the series itself plus any indicator
    /// output series dropped with it (empty when the id is unknown or already removed). Hosts
    /// use the report to fire per-series removal events and drop their own per-series state.
    pub fn remove_series_tracked(&mut self, id: SeriesId) -> Vec<SeriesId> {
        if !self.series.iter().any(|s| s.id == id && !s.removed) {
            return Vec::new();
        }
        // The pane losing the series may collapse afterwards (reference `_cleanupIfPaneIsEmpty`).
        let home_pane = self
            .series
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.pane_index);
        // Drop indicator bindings touching this series and collect their output series to tombstone
        // alongside it (a removed source leaves no derived data behind).
        let mut tombstones = self.drop_indicators_touching(id);
        for binding in self.resampled_series.values() {
            if binding.source == id
                || binding.volume_source == Some(id)
                || binding.target == id
                || binding.volume_target == Some(id)
            {
                if !tombstones.contains(&binding.target) {
                    tombstones.push(binding.target);
                }
                if let Some(volume_target) = binding.volume_target {
                    if !tombstones.contains(&volume_target) {
                        tombstones.push(volume_target);
                    }
                }
            }
        }
        if !tombstones.contains(&id) {
            tombstones.push(id);
        }
        for rid in &tombstones {
            let rid = *rid;
            self.synthetic_series.remove(&rid);
            // The scale losing this source refits exactly under stable autoscale.
            self.reset_series_scale_stabilization(rid);
            self.drop_volume_profiles_using(rid);
            if let Some(entry) = self.series.iter_mut().find(|s| s.id == rid) {
                entry.removed = true;
                entry.visible = false;
                entry.price_lines.clear();
                entry.markers.clear();
                entry.feature = None;
                entry.footprint = None;
                entry.native_primitives.clear();
                entry.autoscale_info_provider = None;
            }
            // Release the data slot; its opaque identity is invalid forever and the storage may
            // be reused by a different identity.
            let removed = self.data.remove_series(rid);
            debug_assert!(removed, "tracked live series must own a data slot");
        }
        self.resampled_series.retain(|_, binding| {
            !tombstones.contains(&binding.source)
                && binding
                    .volume_source
                    .is_none_or(|source| !tombstones.contains(&source))
                && !tombstones.contains(&binding.target)
                && binding
                    .volume_target
                    .is_none_or(|target| !tombstones.contains(&target))
        });
        let mut live_streams = self
            .series
            .iter()
            .filter_map(|series| series.footprint.as_ref().map(|state| state.trade_stream_id))
            .collect::<std::collections::HashSet<_>>();
        live_streams.extend(self.trade_stream_keys.values().copied());
        live_streams.extend(self.trade_bar_dependents.keys().copied());
        live_streams.extend(self.trade_dependents.keys().copied());
        live_streams.extend(self.trade_bubbles.keys().copied());
        self.trade_streams
            .retain(|stream_id, _| live_streams.contains(stream_id));
        self.trade_stream_keys
            .retain(|_, stream_id| live_streams.contains(stream_id));
        for dependents in self.trade_bar_dependents.values_mut() {
            dependents.retain(|dependent| !tombstones.contains(&dependent.series_id));
        }
        self.trade_bar_dependents.retain(|stream_id, dependents| {
            live_streams.contains(stream_id) && !dependents.is_empty()
        });
        for dependents in self.trade_dependents.values_mut() {
            dependents.retain(|dependent| !tombstones.contains(&dependent.series_id));
        }
        self.trade_dependents.retain(|stream_id, dependents| {
            live_streams.contains(stream_id) && !dependents.is_empty()
        });
        for dependents in self.trade_bubbles.values_mut() {
            dependents.retain(|dependent| !tombstones.contains(&dependent.series_id));
        }
        self.trade_bubbles.retain(|stream_id, dependents| {
            live_streams.contains(stream_id) && !dependents.is_empty()
        });
        self.clear_sequence_axis_if_unused();
        self.series_order.retain(|sid| !tombstones.contains(sid));
        // A hovered series leaving the chart releases the hovered-on-top z-bump with it.
        if self
            .hovered_series
            .is_some_and(|hovered| tombstones.contains(&hovered))
        {
            self.hovered_series = None;
        }
        // Removing the primary selection clears the group. Removing another member leaves the
        // remaining indicator outputs selected; sync_time_points prunes its retired snapshot.
        if self
            .selection
            .as_ref()
            .is_some_and(|selection| tombstones.contains(&selection.series))
        {
            self.selection = None;
        }
        self.sync_time_points();
        // reference chart-model.ts `removeSeries`: prune the pane the series left when it is empty
        // and not preserved (a pane-less index — after an explicit `remove_pane` — prunes
        // nothing).
        if let Some(pane_index) = home_pane {
            self.cleanup_if_pane_is_empty(pane_index);
        }
        tombstones
    }

    /// Record a series primitive's autoscale contribution for the next autoscale pass (plugin
    /// platform Phase C-b; reference `ISeriesPrimitiveBase.autoscaleInfo`). Non-finite bounds are
    /// rejected here so a misbehaving plugin cannot poison the scale range.
    pub fn add_autoscale_contribution(&mut self, contribution: PrimitiveAutoscaleContribution) {
        if !contribution.min.is_finite() || !contribution.max.is_finite() {
            return;
        }
        self.primitive_autoscale.push(contribution);
        self.invalidate_frame_scene();
    }

    /// Drop all recorded series-primitive autoscale contributions. Hosts call this at frame
    /// build start, before re-collecting the current frame's contributions.
    pub fn clear_autoscale_contributions(&mut self) {
        if !self.primitive_autoscale.is_empty() {
            self.primitive_autoscale.clear();
            self.invalidate_frame_scene();
        }
    }

    /// reference chart-api.ts `addPane(preserveEmptyPane)` → chart-model.ts `_addPane`: append a
    /// pane and return its index. The new pane's scales inherit the chart-level
    /// `leftPriceScale`/`rightPriceScale` cosmetics, exactly like the reference's `Pane` constructor.
    pub fn add_pane(&mut self, preserve_empty: bool) -> Option<usize> {
        self.add_pane_with_domain(preserve_empty, HorizontalDomain::FinancialTime)
            .ok()
    }

    /// Append a pane with explicit horizontal coordinate semantics. Existing `add_pane` callers
    /// continue to use financial time. General-domain state is allocated only for a non-financial
    /// pane and remains outside frame construction until a compatible general series is installed.
    pub fn add_pane_with_domain(
        &mut self,
        preserve_empty: bool,
        domain: HorizontalDomain,
    ) -> Result<usize, ChartError> {
        let (stable_id, persistent_id) = self.take_pane_ids().ok_or_else(|| {
            ChartError::new(ErrorCode::ResourceLimit, "pane identity space is exhausted")
        })?;
        let general_horizontal_domain = match self.general_horizontal_domains.register(domain) {
            Ok(binding) => binding,
            Err(error) => {
                // No observable handle was issued. Restore the adjacent counters so failure is an
                // atomic topology mutation and repeated capacity failures cannot consume pane IDs.
                self.next_pane_id = stable_id.get();
                self.next_persistent_pane_id = persistent_id;
                return Err(error);
            }
        };
        let mut pane = Pane::with_chart_ids(stable_id, persistent_id);
        pane.general_horizontal_domain = general_horizontal_domain;
        pane.preserve_empty = preserve_empty;
        self.apply_chart_scale_options(&mut pane);
        self.panes.push(pane);
        self.drawing_runtime
            .borrow_mut()
            .rebuild_panes(&self.drawings, self.panes.len());
        self.invalidate_frame_all();
        Ok(self.panes.len() - 1)
    }

    fn take_pane_ids(&mut self) -> Option<(PaneId, u32)> {
        let stable_id = PaneId::try_from(self.next_pane_id).ok()?;
        let next_stable = self.next_pane_id.checked_add(1)?;
        let persistent_id = self.next_persistent_pane_id;
        let next_persistent = persistent_id.checked_add(1)?;
        self.next_pane_id = next_stable;
        self.next_persistent_pane_id = next_persistent;
        Some((stable_id, persistent_id))
    }

    /// Stable chart-local identity for the pane currently at `index`.
    pub fn pane_stable_id(&self, index: usize) -> Option<PaneId> {
        self.panes.get(index)?.stable_id()
    }

    /// Current index of a live pane identity. Removed pane identities never resolve again.
    pub fn pane_index_for_id(&self, stable_id: PaneId) -> Option<usize> {
        self.panes
            .iter()
            .position(|pane| pane.stable_id() == Some(stable_id))
    }

    /// Horizontal coordinate semantics bound to the live pane at `index`.
    pub fn pane_horizontal_domain(&self, index: usize) -> Option<HorizontalDomain> {
        let binding = self.panes.get(index)?.general_horizontal_domain;
        let domain = self.general_horizontal_domains.resolve(binding);
        debug_assert!(
            domain.is_some(),
            "live pane must resolve its domain binding"
        );
        domain
    }

    pub(crate) fn pane_uses_financial_time(&self, index: usize) -> bool {
        self.pane_horizontal_domain(index) == Some(HorizontalDomain::FinancialTime)
    }

    /// reference chart-model.ts `removePane`: rejects out-of-range indices and a non-empty last
    /// pane. An empty preserved last pane is retired in place and replaced by a fresh default pane,
    /// preserving the engine's one-pane invariant without keeping the removed identity alive.
    /// The removed pane's financial series are NOT moved or removed — they become pane-less
    /// (reference leaves them with `paneForSource` → null): they keep their data but render and
    /// scale nowhere until re-assigned. Series below shift one pane up.
    pub fn remove_pane(&mut self, index: usize) -> bool {
        if index >= self.panes.len() {
            return false;
        }
        let replacing_last = self.panes.len() == 1;
        if replacing_last
            && (!self.panes[index].preserve_empty
                || !self.panes[index].named_scales.is_empty()
                || self
                    .series
                    .iter()
                    .any(|series| !series.removed && series.pane_index == index))
        {
            return false;
        }
        let removed_id = self.panes[index].stable_id();
        if removed_id.is_some_and(|pane_id| self.general_series_uses_pane(pane_id)) {
            return false;
        }
        let replacement = if replacing_last {
            let Some((stable_id, persistent_id)) = self.take_pane_ids() else {
                return false;
            };
            let mut pane = Pane::with_chart_ids(stable_id, persistent_id);
            self.apply_chart_scale_options(&mut pane);
            Some(pane)
        } else {
            None
        };
        let removed = match replacement {
            Some(replacement) => std::mem::replace(&mut self.panes[index], replacement),
            None => self.panes.remove(index),
        };
        self.general_horizontal_domains
            .remove(removed.general_horizontal_domain);
        self.general_axes.remove_pane(removed_id);
        self.native_pane_primitives
            .retain(|primitive| Some(primitive.pane_id) != removed_id);
        for s in &mut self.series {
            if s.pane_index == index {
                s.pane_index = PANELESS;
            } else if !replacing_last && s.pane_index != PANELESS && s.pane_index > index {
                s.pane_index -= 1;
            }
        }
        for drawing in &mut self.drawings {
            if drawing.pane_index == index {
                drawing.pane_index = PANELESS;
            } else if !replacing_last
                && drawing.pane_index != PANELESS
                && drawing.pane_index > index
            {
                drawing.pane_index -= 1;
            }
        }
        self.remove_trading_pane(index);
        self.remove_alert_pane(index);
        self.drawing_runtime
            .borrow_mut()
            .rebuild_panes(&self.drawings, self.panes.len());
        self.drawing_history.clear();
        self.invalidate_frame_all();
        true
    }

    /// reference chart-model.ts `swapPanes`: the two panes trade places; their series assignments,
    /// stretch factors, scales, and preserve flags ride along with them.
    pub fn swap_panes(&mut self, first: usize, second: usize) -> bool {
        if first >= self.panes.len() || second >= self.panes.len() {
            return false;
        }
        if first == second {
            return true;
        }
        self.panes.swap(first, second);
        for s in &mut self.series {
            if s.pane_index == first {
                s.pane_index = second;
            } else if s.pane_index == second {
                s.pane_index = first;
            }
        }
        for drawing in &mut self.drawings {
            if drawing.pane_index == first {
                drawing.pane_index = second;
            } else if drawing.pane_index == second {
                drawing.pane_index = first;
            }
        }
        self.swap_trading_panes(first, second);
        self.swap_alert_panes(first, second);
        self.drawing_runtime
            .borrow_mut()
            .rebuild_panes(&self.drawings, self.panes.len());
        self.drawing_history.clear();
        self.invalidate_frame_all();
        true
    }

    /// reference chart-model.ts `movePane` (pane-api.ts `moveTo`): relocate the pane to a new index
    /// with its series; the panes in between shift one slot.
    pub fn move_pane(&mut self, from: usize, to: usize) -> bool {
        if from >= self.panes.len() || to >= self.panes.len() {
            return false;
        }
        if from == to {
            return true;
        }
        let pane = self.panes.remove(from);
        self.panes.insert(to, pane);
        for s in &mut self.series {
            let p = s.pane_index;
            if p == PANELESS {
                continue;
            }
            s.pane_index = if p == from {
                to
            } else if from < to && p > from && p <= to {
                p - 1
            } else if to < from && p >= to && p < from {
                p + 1
            } else {
                p
            };
        }
        for drawing in &mut self.drawings {
            let pane = drawing.pane_index;
            if pane == PANELESS {
                continue;
            }
            drawing.pane_index = if pane == from {
                to
            } else if from < to && pane > from && pane <= to {
                pane - 1
            } else if to < from && pane >= to && pane < from {
                pane + 1
            } else {
                pane
            };
        }
        self.move_trading_pane(from, to);
        self.move_alert_pane(from, to);
        self.drawing_runtime
            .borrow_mut()
            .rebuild_panes(&self.drawings, self.panes.len());
        self.drawing_history.clear();
        self.invalidate_frame_all();
        true
    }

    /// reference pane-api.ts `preserveEmptyPane()` (false for a stale index).
    pub fn pane_preserve_empty(&self, index: usize) -> bool {
        self.panes
            .get(index)
            .map(|p| p.preserve_empty)
            .unwrap_or(false)
    }

    /// reference pane-api.ts `setPreserveEmptyPane(preserve)` (ignored for a stale index).
    pub fn pane_set_preserve_empty(&mut self, index: usize, flag: bool) {
        if let Some(pane) = self.panes.get_mut(index) {
            pane.preserve_empty = flag;
        }
    }

    /// reference pane-api.ts `getSeries()`: the pane's live series in render order (bottom first,
    /// matching the chart z-order). Empty for a stale index.
    pub fn pane_series_ids(&self, index: usize) -> Vec<SeriesId> {
        self.series_order
            .iter()
            .copied()
            .filter(|&id| {
                self.series_entry(id)
                    .is_some_and(|series| series.pane_index == index)
            })
            .collect()
    }

    /// Move a series into pane `pane_index`, creating panes (with the given stretch factor
    /// for a newly-created pane) as needed — reference `moveSeriesToPane` with `_getOrCreatePane`.
    /// The pane the series left collapses when empty and not preserved (reference
    /// `_cleanupIfPaneIsEmpty`, chart-model.ts:1135).
    pub fn set_series_pane(&mut self, id: SeriesId, pane_index: usize, stretch_factor: f64) {
        self.try_set_series_pane(id, pane_index, stretch_factor);
    }

    pub fn try_set_series_pane(
        &mut self,
        id: SeriesId,
        pane_index: usize,
        stretch_factor: f64,
    ) -> bool {
        if pane_index < self.panes.len() && !self.pane_uses_financial_time(pane_index) {
            return false;
        }
        let Some((from, current_target)) = self
            .series_entry(id)
            .map(|series| (series.pane_index, series.price_scale_target))
        else {
            return false;
        };
        let named_public_id = match current_target {
            PriceScaleTarget::Named(_) => self
                .panes
                .get(from)
                .and_then(|pane| pane.public_id_for_target(current_target))
                .map(str::to_string),
            _ => None,
        };
        if named_public_id.is_some() && pane_index >= self.panes.len() {
            return false;
        }
        let destination_target = named_public_id
            .as_deref()
            .map(|public_id| {
                self.panes
                    .get(pane_index)
                    .and_then(|pane| pane.target_for_public_id(public_id))
            })
            .unwrap_or(Some(current_target));
        let Some(destination_target) = destination_target else {
            return false;
        };
        while self.panes.len() <= pane_index {
            let Some((stable_id, persistent_id)) = self.take_pane_ids() else {
                return false;
            };
            let mut pane = Pane::with_chart_ids(stable_id, persistent_id);
            pane.stretch_factor = stretch_factor.max(0.01);
            self.apply_chart_scale_options(&mut pane);
            self.panes.push(pane);
        }
        let Some(series) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        if from == pane_index {
            return true;
        }
        series.pane_index = pane_index;
        series.price_scale_target = destination_target;
        // Both the scale the series left and the one it joined refit exactly.
        self.reset_scale_stabilization_at(from, current_target);
        self.reset_scale_stabilization_at(pane_index, destination_target);
        if from != PANELESS {
            self.cleanup_if_pane_is_empty(from);
        }
        self.invalidate_frame_all();
        true
    }

    pub fn try_set_series_pane_and_scale(
        &mut self,
        id: SeriesId,
        pane_index: usize,
        stretch_factor: f64,
        price_scale_id: &str,
    ) -> bool {
        if pane_index < self.panes.len() && !self.pane_uses_financial_time(pane_index) {
            return false;
        }
        let Some((from, from_target)) = self
            .series_entry(id)
            .map(|series| (series.pane_index, series.price_scale_target))
        else {
            return false;
        };
        let built_in = matches!(price_scale_id, "left" | "right" | "");
        if !built_in && pane_index >= self.panes.len() {
            return false;
        }
        while self.panes.len() <= pane_index {
            let Some((stable_id, persistent_id)) = self.take_pane_ids() else {
                return false;
            };
            let mut pane = Pane::with_chart_ids(stable_id, persistent_id);
            pane.stretch_factor = stretch_factor.max(0.01);
            self.apply_chart_scale_options(&mut pane);
            self.panes.push(pane);
        }
        let Some(target) = self.panes[pane_index].target_for_public_id(price_scale_id) else {
            return false;
        };
        let Some(series) = self.series_entry_mut(id) else {
            return false;
        };
        series.pane_index = pane_index;
        series.price_scale_target = target;
        if (from, from_target) != (pane_index, target) {
            // Both the scale the series left and the one it joined refit exactly.
            self.reset_scale_stabilization_at(from, from_target);
            self.reset_scale_stabilization_at(pane_index, target);
        }
        if from != pane_index && from != PANELESS {
            self.cleanup_if_pane_is_empty(from);
        }
        self.invalidate_frame_all();
        true
    }

    /// Port of reference chart-model.ts `_cleanupIfPaneIsEmpty`: a pane left without any live
    /// series collapses unless it is preserved or the last remaining pane. Series below
    /// shift one pane up. Returns true when the pane was removed.
    fn cleanup_if_pane_is_empty(&mut self, pane_index: usize) -> bool {
        if pane_index >= self.panes.len() || self.panes.len() <= 1 {
            return false;
        }
        if self.panes[pane_index].preserve_empty {
            return false;
        }
        if self.panes[pane_index]
            .stable_id()
            .is_some_and(|pane_id| self.general_series_uses_pane(pane_id))
        {
            return false;
        }
        if !self.panes[pane_index].named_scales.is_empty() {
            return false;
        }
        // reference checks `pane.dataSources().length === 0`: hidden series still occupy their
        // pane; removed (tombstoned) ones are detached from it.
        if self
            .series
            .iter()
            .any(|s| !s.removed && s.pane_index == pane_index)
        {
            return false;
        }
        let removed = self.panes.remove(pane_index);
        self.general_horizontal_domains
            .remove(removed.general_horizontal_domain);
        self.general_axes.remove_pane(removed.stable_id());
        for s in &mut self.series {
            if s.pane_index != PANELESS && s.pane_index > pane_index {
                s.pane_index -= 1;
            }
        }
        true
    }

    /// Copy the chart-level `leftPriceScale`/`rightPriceScale` scale-held cosmetics onto a
    /// new pane's scales (reference pane.ts constructor `_createPriceScale` from the chart options).
    fn apply_chart_scale_options(&self, pane: &mut Pane) {
        let options = self.options.get();
        let apply = |scale: &mut PriceScaleCore,
                     group: &aeris_charts_core::options::PriceAxisOptions| {
            scale.set_align_labels(group.align_labels);
            scale.set_ticks_visible(group.ticks_visible);
            scale.set_entire_text_only(group.entire_text_only);
            scale.set_minimum_width(group.minimum_width);
            scale.set_text_color(group.text_color.clone());
        };
        apply(&mut pane.left_scale, &options.left_price_scale);
        apply(&mut pane.price_scale, &options.right_price_scale);
        // Tick density and edge marks are scale-core options carried only in the raw chart
        // options, like `boldRoundLabels`.
        let raw = self.options.value();
        for (key, scale) in [
            ("leftPriceScale", &mut pane.left_scale),
            ("rightPriceScale", &mut pane.price_scale),
        ] {
            if let Some(group) = raw.get(key).and_then(serde_json::Value::as_object) {
                apply_chart_tick_mark_options(scale, group);
            }
        }
    }

    /// Whether `id` names a tombstoned (removed) series. Data mutations on such a slot are ignored
    /// so a removed series can never be silently revived.
    pub fn is_series_removed(&self, id: SeriesId) -> bool {
        matches!(
            self.data.validate_series_id(id),
            Err(SeriesIdError::Stale(_))
        )
    }

    /// Validate a public series identity without touching chart state.
    pub fn validate_series_id(&self, id: SeriesId) -> Result<(), SeriesIdError> {
        self.data.validate_series_id(id)
    }

    /// Resolve a live opaque series identity to its current storage entry.
    pub(crate) fn series_entry(&self, id: SeriesId) -> Option<&SeriesEntry> {
        let slot = self.data.series_slot(id)?;
        self.series
            .get(slot)
            .filter(|series| series.id == id && !series.removed)
    }

    pub(crate) fn series_entry_mut(&mut self, id: SeriesId) -> Option<&mut SeriesEntry> {
        let slot = self.data.series_slot(id)?;
        self.series
            .get_mut(slot)
            .filter(|series| series.id == id && !series.removed)
    }

    /// Return one lazily rebuilt Heikin Ashi row for a candlestick presentation.
    /// The cache is invalidated by the canonical data generation and never replaces raw columns.
    pub(crate) fn heikin_ashi_row(&self, id: SeriesId, row: usize) -> Option<[f64; 4]> {
        let series = self.series_entry(id)?;
        if !series.heikin_ashi {
            return None;
        }
        let generation = self.data.series_generation(id)?;
        let (_, columns) = self.data.series_data(id)?;
        // `row` is a plot row; the projection is over canonical rows (they differ as-of).
        let row = self.data.plot(id).source_row(row);
        series
            .heikin_ashi_cache
            .borrow_mut()
            .row(generation, columns, row)
    }

    /// Toggle a series without destroying its data or indicator binding. A removed slot can
    /// never be revived, so visibility changes on it are ignored.
    /// Stops drawing a series' rows at or after `time` (UTC seconds) while keeping its data,
    /// scale participation, and last-value chrome. `None` draws every row.
    pub fn set_series_render_before_time(&mut self, id: SeriesId, time: Option<i64>) {
        if let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed)
        {
            if series.render_before_time == time {
                return;
            }
            series.render_before_time = time;
            self.invalidate_frame_scene();
        }
    }

    /// Last logical index a series draws inside `to`, honoring its render cutoff.
    pub(crate) fn series_render_end(&self, id: SeriesId, to: i64) -> i64 {
        let Some(cutoff) = self
            .series
            .iter()
            .find(|series| series.id == id && !series.removed)
            .and_then(|series| series.render_before_time)
        else {
            return to;
        };
        let first_hidden = match self.sequence_points() {
            Some(points) => {
                let cutoff_micros = cutoff.saturating_mul(1_000_000);
                points.partition_point(|point| point.open_timestamp_micros < cutoff_micros)
            }
            None => self
                .data
                .merged_times()
                .partition_point(|&time| time < cutoff),
        };
        to.min(first_hidden as i64 - 1)
    }

    pub fn set_series_visible(&mut self, id: SeriesId, visible: bool) {
        if let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed)
        {
            if series.visible == visible {
                return;
            }
            series.visible = visible;
            // Showing or hiding a source refits a stable scale exactly, like the default mode.
            self.reset_series_scale_stabilization(id);
            self.invalidate_frame_scene();
            self.invalidate_frame_layout_and_axis();
        }
    }

    /// The effective primary series: the first visible, non-removed entry. reference lets any
    /// series be removed (`removeSeries`), so every "first series" anchor resolves through
    /// this fallback instead of assuming id 0 is alive.
    pub(crate) fn primary_series(&self) -> Option<&SeriesEntry> {
        self.series.iter().find(|s| !s.removed && s.visible)
    }

    /// Host choice for a series' last-price pulse. Line and area default on; this records an
    /// explicit opt-out (or opt-in for other kinds) that later kind changes preserve.
    pub fn set_series_last_price_animation(&mut self, id: SeriesId, enabled: bool) -> bool {
        let Some(series) = self.series.iter_mut().find(|s| s.id == id && !s.removed) else {
            return false;
        };
        series.last_price_animation = enabled;
        series.last_price_animation_explicit = true;
        true
    }

    /// Whether the frame draws a last-price pulse, which is exactly when a host must keep its
    /// animation clock running. Mirrors `build_last_pulse_frame`: the primary series owns the
    /// pulse and needs data, so an empty or opted-out chart never runs an animation loop.
    pub fn last_price_pulse_active(&self) -> bool {
        self.primary_series().is_some_and(|series| {
            series.last_price_animation && !self.data.plot(series.id).is_empty()
        })
    }

    /// Series ids in stable saved order (bottom to top; topmost LAST), live series only.
    /// This is the underlying order frame assembly derives the pane-local paint order from:
    /// default idle indicators (stable) below idle drawings below ordinary series with active
    /// objects promoted on top; an explicit `set_series_order` override paints idle series
    /// verbatim. Hit-test ties break on this stable order so promotion cannot oscillate hover.
    pub fn series_order(&self) -> &[SeriesId] {
        &self.series_order
    }

    /// The stable saved order as a JSON array of series ids (the reference's z-order; the last
    /// id is topmost in the saved order — the frame may temporarily promote hovered/selected
    /// objects above it without rewriting it). Backs the TS `chart.seriesOrder()`.
    pub fn series_order_json(&self) -> String {
        serde_json::to_string(&self.series_order).unwrap_or_else(|_| "[]".to_string())
    }

    /// reference `chart.setSeriesOrder`: override the default series ordering with an explicit
    /// paint order for idle series (bottom to top). The patch must name every live series id
    /// exactly once (a bad permutation — wrong length, duplicates, unknown or missing ids —
    /// is rejected with false and no state change). Active hover/selection promotion still
    /// applies above the explicit idle order, and idle drawings remain below price series.
    pub fn set_series_order(&mut self, ids: Vec<SeriesId>) -> bool {
        self.invalidate_frame_scene();
        if ids.len() != self.series_order.len() {
            return false;
        }
        let mut requested = ids.clone();
        requested.sort_unstable();
        let mut current = self.series_order.clone();
        current.sort_unstable();
        if requested != current {
            return false;
        }
        self.series_order = ids;
        self.series_order_explicit = true;
        true
    }

    /// Remove the last `count` data points of a series (reference v5.2 `ISeriesApi.pop`,
    /// iseries-api.ts:203): `count` 0 is a no-op, larger counts clamp to the data length.
    /// Per-point color channels truncate with their rows. Returns the new data length, or
    /// `None` for an unknown/removed id.
    pub fn series_pop(&mut self, id: SeriesId, count: usize) -> Option<usize> {
        if self.is_series_removed(id) || !self.series.iter().any(|s| s.id == id) {
            return None;
        }
        if self.is_source_owned_series(id) {
            return None;
        }
        self.invalidate_frame_series(id);
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        let len = self.data.pop(id, count)?;
        self.truncate_feature_rows(id, len);
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            IndicatorChange {
                from: len,
                previous_generation,
                full_replace: true,
            },
        );
        Some(len)
    }

    pub fn set_series_markers(&mut self, id: SeriesId, markers: Vec<Marker>) {
        self.invalidate_frame_series(id);
        // A trade-bubble fold writing this series must refold on its next refresh.
        self.invalidate_trade_bubble_folds(id, None);
        if let Some(series) = self.series_entry_mut(id) {
            series.markers = markers;
        }
    }

    pub fn set_series_markers_auto_scale(&mut self, id: SeriesId, enabled: bool) {
        self.invalidate_frame_series(id);
        if let Some(series) = self.series_entry_mut(id) {
            series.markers_auto_scale = enabled;
        }
    }

    pub fn set_series_markers_z_order(&mut self, id: SeriesId, z_order: u8) -> bool {
        if !matches!(
            z_order,
            marker_z_order::NORMAL | marker_z_order::ABOVE_SERIES | marker_z_order::TOP
        ) {
            return false;
        }
        self.invalidate_frame_series(id);
        let Some(series) = self.series_entry_mut(id) else {
            return false;
        };
        series.markers_z_order = z_order;
        true
    }

    /// Apply one streaming OHLC update after validating its time and values.
    pub fn update_series_bar(&mut self, id: SeriesId, time: f64, values: [f64; 4]) -> bool {
        self.update_series_bar_styled(id, time, values, [None; 3])
    }

    /// Apply ordered streaming rows without coordinator allocation, then synchronize shared chart
    /// state and dependent indicators once. Used by the bounded shared-ring drain; each valid row
    /// keeps the single-update append/replace semantics.
    pub fn update_series_bars<I>(&mut self, id: SeriesId, rows: I) -> usize
    where
        I: IntoIterator<Item = (f64, [f64; 4])>,
    {
        if self.validate_series_id(id).is_err() || self.is_source_owned_series(id) {
            return 0;
        }
        self.invalidate_frame_series(id);
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        let mut from = usize::MAX;
        let mut accepted = 0;
        for (time, values) in rows {
            let Some((time, values)) = sanitize_point(time, values) else {
                continue;
            };
            let row = self
                .data
                .series_data(id)
                .map(|(times, _)| {
                    times
                        .binary_search(&time)
                        .unwrap_or_else(|position| position)
                })
                .unwrap_or_default();
            from = from.min(row);
            self.data.update_styled(id, time, values, [None; 3]);
            accepted += 1;
        }
        if accepted == 0 {
            return 0;
        }
        let trimmed = self.enforce_series_cap(id);
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            IndicatorChange {
                from: if trimmed { 0 } else { from },
                previous_generation,
                full_replace: trimmed,
            },
        );
        accepted
    }

    /// Install an already sanitized ascending/unique batch. This is the WASM typed-array fast
    /// path: the data layer merges once, time state synchronizes once, and each dependent
    /// indicator advances once from the earliest affected row.
    pub fn update_series_bars_sanitized(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> usize {
        if self.validate_series_id(id).is_err()
            || self.is_source_owned_series(id)
            || times.is_empty()
        {
            return 0;
        }
        self.update_series_bars_sanitized_inner(id, times, open, high, low, close)
    }

    fn update_series_bars_sanitized_inner(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> usize {
        self.invalidate_frame_series(id);
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        let Some(from) = self
            .data
            .update_many(id, &times, [&open, &high, &low, &close])
        else {
            return 0;
        };
        let accepted = times.len();
        let trimmed = self.enforce_series_cap(id);
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            IndicatorChange {
                from,
                previous_generation,
                full_replace: trimmed,
            },
        );
        accepted
    }

    /// [`update_series_bar`] plus the target bar's per-point color channels (reference
    /// `series.update` with data-item colors; `None` = no custom color for that channel).
    /// Mirrors the plain update's semantics exactly: append-new-time vs replace-last.
    pub fn update_series_bar_styled(
        &mut self,
        id: SeriesId,
        time: f64,
        values: [f64; 4],
        colors: [Option<u32>; 3],
    ) -> bool {
        if self.validate_series_id(id).is_err() || self.is_source_owned_series(id) {
            return false;
        }
        self.update_series_bar_styled_inner(id, time, values, colors)
    }

    fn update_series_bar_styled_inner(
        &mut self,
        id: SeriesId,
        time: f64,
        values: [f64; 4],
        colors: [Option<u32>; 3],
    ) -> bool {
        let Some((time, values)) = sanitize_point(time, values) else {
            return false;
        };
        self.invalidate_frame_series(id);
        let from = self
            .data
            .series_data(id)
            .map(|(times, _)| {
                times
                    .binary_search(&time)
                    .unwrap_or_else(|position| position)
            })
            .unwrap_or_default();
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        self.data.update_styled(id, time, values, colors);
        // Retention (`max_points`): usually a no-op flag check, and an O(total) trim once every
        // `margin` appends. Runs before `sync_time_points` so the scale sees the final row set.
        if self.enforce_series_cap(id) {
            self.sync_time_points();
            self.update_indicators_after_change(
                id,
                IndicatorChange {
                    from: 0,
                    previous_generation,
                    full_replace: true,
                },
            );
            return true;
        }
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            IndicatorChange {
                from,
                previous_generation,
                full_replace: false,
            },
        );
        true
    }

    /// Install per-row color overrides for a series (reference data-item colors, packed RGBA
    /// `0xRRGGBBAA`). Channels: body = candle/bar body, line/area stroke + point marker,
    /// histogram column; wick/border = candlestick parts. Each channel is `None`/empty for
    /// absent, or must match the series' row count exactly — a mismatch rejects the whole call
    /// (false, no partial state). `set_series_data`/`install_series_data` reset these colors,
    /// so hosts install them right after setting data.
    pub fn set_series_point_colors(
        &mut self,
        id: SeriesId,
        body: Option<Vec<u32>>,
        wick: Option<Vec<u32>>,
        border: Option<Vec<u32>>,
    ) -> bool {
        if self.validate_series_id(id).is_err() || self.is_source_owned_series(id) {
            return false;
        }
        let changed = self.data.set_point_colors(id, [body, wick, border]);
        if changed {
            self.invalidate_frame_series(id);
        }
        changed
    }

    /// Sets or clears one fixed-value background channel for a scalar line/area series.
    ///
    /// Invalid/non-finite bounds, reversed bounds, unknown series, and non-line/area series are
    /// rejected without mutating the current presentation.
    pub fn set_series_threshold_region(
        &mut self,
        id: SeriesId,
        region: Option<SeriesThresholdRegion>,
    ) -> bool {
        let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed)
        else {
            return false;
        };
        if !matches!(series.kind, SeriesKind::Line | SeriesKind::Area) {
            return false;
        }
        if region.is_some_and(|region| {
            !region.lower.is_finite() || !region.upper.is_finite() || region.lower >= region.upper
        }) {
            return false;
        }
        if series.threshold_region == region {
            return false;
        }
        series.threshold_region = region;
        self.invalidate_frame_series(id);
        true
    }

    /// Full (re)assignment with per-row color channels run through the same repair pipeline as
    /// the OHLC columns: the colors follow their row through invalid-row drops and the stable
    /// sort, and the last-wins dedupe keeps the winning row's channels.
    #[allow(clippy::too_many_arguments)] // mirrors set_series_data plus the three reference color slots
    pub fn set_series_data_styled(
        &mut self,
        id: SeriesId,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
        colors: [Option<Vec<u32>>; 3],
    ) -> Result<ValidationReport, ValidationError> {
        self.validate_series_id(id).map_err(|error| match error {
            SeriesIdError::Unknown(id) => ValidationError::UnknownSeries(id),
            SeriesIdError::Stale(id) => ValidationError::StaleSeries(id),
        })?;
        if self.is_source_owned_series(id) {
            return Err(ValidationError::UnsupportedSeriesData(id));
        }
        let s = sanitize_ohlc_styled(times, open, high, low, close, colors)?;
        self.invalidate_frame_series(id);
        let report = s.data.report.clone();
        self.install_series_columns(
            id,
            s.data.times,
            s.data.open,
            s.data.high,
            s.data.low,
            s.data.close,
        );
        let [body, wick, border] = s.colors;
        let installed = self
            .data
            .set_point_colors(id, [Some(body), Some(wick), Some(border)]);
        debug_assert!(installed, "sanitized channels are aligned by construction");
        // Retention (`max_points`): trim after the colors land so the eviction shifts rows and
        // color channels together.
        self.enforce_series_cap(id);
        self.sync_time_points();
        self.recompute_indicators_for(id);
        self.restart_selection_anchor_snapshot_after_replacement(id);
        Ok(report)
    }

    /// Validate and install one series' parallel OHLC columns without involving a host runtime.
    /// The returned report lets browser, native, and server callers expose identical diagnostics.
    pub fn set_series_data(
        &mut self,
        id: SeriesId,
        times: &[f64],
        open: &[f64],
        high: &[f64],
        low: &[f64],
        close: &[f64],
    ) -> Result<ValidationReport, ValidationError> {
        // A removed slot must stay empty; ignore the data (the TS series handle rejects the call
        // before it reaches here, so this is defense-in-depth) and report a clean no-op.
        self.validate_series_id(id).map_err(|error| match error {
            SeriesIdError::Unknown(id) => ValidationError::UnknownSeries(id),
            SeriesIdError::Stale(id) => ValidationError::StaleSeries(id),
        })?;
        if self.is_source_owned_series(id) {
            return Err(ValidationError::UnsupportedSeriesData(id));
        }
        let sanitized = sanitize_ohlc(times, open, high, low, close)?;
        self.invalidate_frame_series(id);
        let report = sanitized.report.clone();
        self.install_series_columns(
            id,
            sanitized.times,
            sanitized.open,
            sanitized.high,
            sanitized.low,
            sanitized.close,
        );
        // Retention (`max_points`): a full install can exceed the ceiling; trim before the scale
        // and the indicators index the rows. The report still describes the caller's input.
        self.enforce_series_cap(id);
        self.sync_time_points();
        self.recompute_indicators_for(id);
        self.restart_selection_anchor_snapshot_after_replacement(id);
        Ok(report)
    }

    /// Install columns that have already crossed the validation boundary (used by adapters that
    /// need to report the sanitization details before handing ownership to the engine).
    pub fn install_series_data(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> bool {
        if self.validate_series_id(id).is_err() || self.is_source_owned_series(id) {
            return false;
        }
        self.install_series_data_inner(id, times, open, high, low, close)
    }

    fn install_series_data_inner(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> bool {
        self.invalidate_frame_series(id);
        if !self.install_series_columns(id, times, open, high, low, close) {
            return false;
        }
        // A full install can land more rows than the retention ceiling allows; trim before the
        // scale and the indicators see the row set, so nothing downstream indexes evicted rows.
        self.enforce_series_cap(id);
        self.sync_time_points();
        self.recompute_indicators_for(id);
        self.restart_selection_anchor_snapshot_after_replacement(id);
        true
    }

    fn is_footprint_series(&self, id: SeriesId) -> bool {
        self.series.iter().any(|series| {
            series.id == id && !series.removed && series.kind == SeriesKind::Footprint
        })
    }

    /// Source-owned series may only be mutated through their canonical footprint/trade or
    /// synthetic-bar ingestion API. Generic OHLC writes would desynchronize the visible
    /// projection from the state that owns replay, sequence identity, and incremental updates.
    fn is_source_owned_series(&self, id: SeriesId) -> bool {
        self.is_footprint_series(id)
            || self.synthetic_series.contains_key(&id)
            || self
                .resampled_series
                .values()
                .any(|binding| binding.target == id || binding.volume_target == Some(id))
    }

    pub(crate) fn install_footprint_projection(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> bool {
        debug_assert!(self.is_footprint_series(id));
        self.sequence_points = None;
        self.pending_sequence_mapping = None;
        self.install_series_data_inner(id, times, open, high, low, close)
    }

    /// `key_base` is the row key of the first bar: the trade stream's retained key base, so a
    /// footprint installed after retention trims stays aligned with the stream's other rows.
    pub(crate) fn install_footprint_sequence_projection(
        &mut self,
        id: SeriesId,
        key_base: i64,
        projection: SequenceProjectionColumns,
    ) -> bool {
        debug_assert!(self.is_footprint_series(id));
        self.install_sequence_projection_inner(id, Some(key_base), projection)
    }

    /// Install a complete sequence-axis projection keyed contiguously from `key_base`, or, when
    /// `None`, from the series' own first key while the sequence axis is live (zero otherwise).
    fn install_sequence_projection_inner(
        &mut self,
        id: SeriesId,
        key_base: Option<i64>,
        projection: SequenceProjectionColumns,
    ) -> bool {
        let (mut points, open, high, low, close) = projection;
        if points.len() != open.len()
            || points.len() != high.len()
            || points.len() != low.len()
            || points.len() != close.len()
        {
            return false;
        }
        // Keep the row-key base across reinstalls so studies and markers written against the
        // current sequence axis stay aligned (retention trims drop keys without re-keying).
        let key_base = key_base.unwrap_or_else(|| {
            if self.sequence_points.is_some() {
                self.data
                    .series_data(id)
                    .and_then(|(times, _)| times.first().copied())
                    .unwrap_or(0)
            } else {
                0
            }
        });
        let times = (0..points.len())
            .map(|index| key_base + index as i64)
            .collect::<Vec<_>>();
        // The data layer synchronizes immediately during installation. Clear any prior
        // sequence first so that synchronization cannot interpret the new logical rows with an
        // unrelated sidecar; install the new sidecar before the second sync below.
        let previous_sequence = self.sequence_points.take();
        let previous_pending = self.pending_sequence_mapping.take();
        // The aggregator keeps absolute bar identities across retention, while the chart data
        // layer and drawing geometry address the retained rows from zero. Preserve the absolute
        // identity through the full-resolution times, but normalize the chart-local sidecar
        // indices before mapping/rebasing anchors. Retention trims the installed rows inside the
        // install, so map the prior sequence onto the rows that remain: the viewport and drawing
        // rebases in that synchronization then address the final chart-local indices.
        let retained = self.rows_retained_after_cap(id, points.len());
        points.drain(..points.len() - retained);
        for (index, point) in points.iter_mut().enumerate() {
            point.logical_index = index as u64;
        }
        self.pending_sequence_mapping = previous_sequence
            .as_deref()
            .map(|old| BarSequenceMapping::between_points(old, &points));
        // The row keys installed below are not UTC times; drawing time identity must not read them.
        self.drawing_settings.sequence_install = true;
        let installed = self.install_series_data_inner(id, times, open, high, low, close);
        self.drawing_settings.sequence_install = false;
        if installed {
            let retained_len = self
                .data
                .series_data(id)
                .map_or(0, |(retained_times, _)| retained_times.len());
            if points.len() > retained_len {
                points.drain(..points.len() - retained_len);
            }
            self.sequence_points = Some(points);
            self.apply_persisted_drawing_anchor_times();
            // Recompute tick weights and axis endpoints from the full-resolution sequence times,
            // not from the chart-local row keys used by the data layer. The data-layer generation
            // is unchanged at this point, so the ordinary sync path would otherwise retain the
            // row-key weights.
            self.sync_sequence_axis_times();
        } else {
            self.sequence_points = previous_sequence;
            self.pending_sequence_mapping = previous_pending;
        }
        installed
    }

    /// `key_base` is the row key of the first bar, as in
    /// [`Self::install_footprint_sequence_projection`].
    pub(crate) fn install_trade_bar_sequence_projection(
        &mut self,
        id: SeriesId,
        key_base: i64,
        projection: SequenceProjectionColumns,
    ) -> bool {
        debug_assert!(self.series_entry(id).is_some_and(|series| {
            matches!(series.kind, SeriesKind::Candlestick | SeriesKind::Bar)
        }));
        self.install_sequence_projection_inner(id, Some(key_base), projection)
    }

    pub(crate) fn sequence_points(&self) -> Option<&[BarSequencePoint]> {
        self.sequence_points.as_deref()
    }

    pub(crate) fn drawing_anchor_times_for(
        &self,
        drawing: &Drawing,
    ) -> Vec<Option<DrawingAnchorTime>> {
        let Some(points) = self
            .sequence_points()
            .filter(|_| !drawing.kind.pane_anchored())
        else {
            return Vec::new();
        };
        drawing
            .points
            .iter()
            .map(|point| {
                let index = point.logical.round();
                if !index.is_finite() || index < 0.0 {
                    return None;
                }
                points
                    .get(index as usize)
                    .map(|sequence| DrawingAnchorTime {
                        open_timestamp_micros: sequence.open_timestamp_micros,
                        close_timestamp_micros: sequence.close_timestamp_micros,
                    })
            })
            .collect()
    }

    fn apply_persisted_drawing_anchor_times(&mut self) {
        let Some(sequence) = self.sequence_points() else {
            return;
        };
        if self.drawing_anchor_times.is_empty() {
            return;
        }
        let lookup = sequence
            .iter()
            .enumerate()
            .map(|(index, point)| {
                (
                    (point.open_timestamp_micros, point.close_timestamp_micros),
                    index,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut resolved_ids = Vec::new();
        let mut changed = false;
        for drawing in &mut self.drawings {
            let Some(anchor_times) = self.drawing_anchor_times.get(&drawing.id) else {
                continue;
            };
            if anchor_times.len() != drawing.points.len() {
                continue;
            }
            let mut resolved_all = true;
            for (point, anchor_time) in drawing.points.iter_mut().zip(anchor_times) {
                let Some(anchor_time) = anchor_time else {
                    resolved_all = false;
                    continue;
                };
                let Some(&index) = lookup.get(&(
                    anchor_time.open_timestamp_micros,
                    anchor_time.close_timestamp_micros,
                )) else {
                    resolved_all = false;
                    continue;
                };
                let offset = point.logical - point.logical.round();
                let logical = index as f64 + offset;
                changed |= logical != point.logical;
                point.logical = logical;
            }
            if resolved_all {
                resolved_ids.push(drawing.id);
            }
        }
        for id in resolved_ids {
            self.drawing_anchor_times.remove(&id);
        }
        if changed {
            self.drawing_runtime
                .borrow_mut()
                .rebuild_all(&self.drawings, self.panes.len());
            self.invalidate_frame_drawings();
        }
    }

    pub(crate) fn update_footprint_projection_bars(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> usize {
        debug_assert!(self.is_footprint_series(id));
        self.update_series_bars_sanitized_inner(id, times, open, high, low, close)
    }

    pub(crate) fn update_footprint_sequence_projection_bars(
        &mut self,
        id: SeriesId,
        from: usize,
        projection: SequenceProjectionColumns,
    ) -> usize {
        debug_assert!(self.is_footprint_series(id));
        self.update_sequence_projection_bars_inner(id, from, projection)
    }

    pub(crate) fn update_trade_bar_sequence_projection_bars(
        &mut self,
        id: SeriesId,
        from: usize,
        projection: SequenceProjectionColumns,
    ) -> usize {
        debug_assert!(self.series_entry(id).is_some_and(|series| {
            matches!(series.kind, SeriesKind::Candlestick | SeriesKind::Bar)
        }));
        self.update_sequence_projection_bars_inner(id, from, projection)
    }

    fn update_sequence_projection_bars_inner(
        &mut self,
        id: SeriesId,
        from: usize,
        projection: SequenceProjectionColumns,
    ) -> usize {
        let (mut points, open, high, low, close) = projection;
        if points.is_empty()
            || points.len() != open.len()
            || points.len() != high.len()
            || points.len() != low.len()
            || points.len() != close.len()
        {
            return 0;
        }
        // Rows are keyed contiguously from the projection's first retained key (retention trims
        // drop keys from the front without re-keying).
        let key_base = self
            .data
            .series_data(id)
            .and_then(|(times, _)| times.first().copied())
            .unwrap_or(0);
        let Some(sequence) = self.sequence_points.as_mut() else {
            return 0;
        };
        if from > sequence.len() {
            return 0;
        }
        // A live tip rewrites only the active suffix. The prefix keeps its identities, so logical
        // anchors map to themselves and no rebase mapping is needed. The sidecar changes before
        // the rows so a retention trim inside the update drains the matching prefix, and the
        // shared time sync then reads the final sidecar in place.
        let replaced = sequence.split_off(from);
        for (index, point) in points.iter_mut().enumerate() {
            point.logical_index = (from + index) as u64;
        }
        sequence.extend_from_slice(&points);
        let times = (from..from + points.len())
            .map(|index| key_base + index as i64)
            .collect::<Vec<_>>();
        let accepted = self.update_series_bars_sanitized_inner(id, times, open, high, low, close);
        if accepted != points.len() {
            if let Some(sequence) = self.sequence_points.as_mut() {
                sequence.truncate(from);
                sequence.extend(replaced);
            }
        }
        accepted
    }

    fn install_series_columns(
        &mut self,
        id: SeriesId,
        times: Vec<i64>,
        open: Vec<f64>,
        high: Vec<f64>,
        low: Vec<f64>,
        close: Vec<f64>,
    ) -> bool {
        // A full replace is a resync: the provider's sequence space may have restarted.
        if let Some(series) = self.series_entry_mut(id) {
            series.update_sequence = None;
        }
        // A full replacement restarts stable autoscale from the new data's exact range, on this
        // series' scale and on every derived indicator output's scale.
        self.reset_replaced_series_stabilization(id);
        let scalar = self
            .series_entry(id)
            .is_some_and(|series| series.kind.stores_scalar_values())
            && open == high
            && open == low
            && open == close;
        if scalar {
            self.data.set_single_data(id, times, close)
        } else {
            self.data.set_data(id, times, open, high, low, close)
        }
    }

    /// Set a series' retention ceiling: at most `max_points` rows, oldest evicted first. `None`
    /// restores the default unbounded behavior. Applied immediately to the series' current rows.
    ///
    /// **Eviction schedule.** Trimming is `O(total rows)` (a row shift plus a rebuild of the shared
    /// time axis), so evicting on every append would make a long streaming session quadratic.
    /// Instead the engine trims with hysteresis: once the row count exceeds `max_points` it drops
    /// back to `max_points - margin`, where `margin` is [`CAP_TRIM_MARGIN_DIVISOR`]-th of the cap.
    /// `max_points` is therefore a **hard ceiling** — the series never holds more — while the
    /// floor right after a trim is `max_points - margin`. Amortized cost per appended point is
    /// constant.
    ///
    /// **Replay.** Under a replay clock the ceiling counts and evicts only the rows up to the
    /// clock, so the chart holds what a clean load to that clock with the same cap would hold,
    /// and a seek that reveals rows trims them the same way. Rows ingested past the clock are the
    /// replay's pending source truth: they are retained, uncounted, until the clock reveals them
    /// (hidden rows never push out visible ones).
    ///
    /// An unknown or removed id is ignored.
    pub fn set_series_max_points(&mut self, id: SeriesId, max_points: Option<usize>) -> bool {
        if self.is_series_removed(id) {
            return false;
        }
        // Non-time candle/bar dependents share the stream's exact logical sequence and retention
        // boundary. A series-local cap would silently misalign it from footprint and studies.
        if max_points.is_some()
            && (self.is_trade_bar_dependent(id) || self.synthetic_series.contains_key(&id))
        {
            return false;
        }
        let Some(entry) = self.series_entry_mut(id) else {
            return false;
        };
        entry.max_points = max_points;
        if self.enforce_series_cap(id) {
            self.sync_time_points();
            self.recompute_indicators_for(id);
        }
        true
    }

    /// This series' retention ceiling (`None` = unbounded).
    pub fn series_max_points(&self, id: SeriesId) -> Option<usize> {
        self.series_entry(id).and_then(|series| series.max_points)
    }

    /// Evict oldest rows if the series is over its ceiling. Returns whether anything was dropped,
    /// so callers can skip the follow-up scale/indicator sync in the overwhelmingly common case
    /// where nothing needed evicting. The ceiling counts the rows the series exposes (up to the
    /// replay clock) and, except for a footprint projection, evicts only from their front; rows
    /// past the clock stay.
    fn enforce_series_cap(&mut self, id: SeriesId) -> bool {
        let Some(max_points) = self
            .series
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.max_points)
        else {
            return false;
        };
        let visible = self
            .data
            .series_data(id)
            .map_or(0, |(times, _)| times.len());
        if visible <= max_points {
            return false;
        }
        let keep = self.rows_retained_after_cap(id, visible);
        // A footprint projection is rebuilt from its stream with the ceiling applied to every bar
        // it projects, so its trim keeps that many rows in all, as that rebuild does. Any other
        // series keeps its rows past the clock and evicts only from the front of the rows up to
        // it; the data layer and the feature sidecar hold every canonical row.
        let footprint = self
            .series_entry(id)
            .is_some_and(|series| series.footprint.is_some());
        let hidden = if footprint {
            0
        } else {
            self.data.series_rows(id).unwrap_or(visible) - visible
        };
        self.data.trim_front(id, keep + hidden);
        self.trim_feature_rows_front(id, keep + hidden);
        self.trim_footprint_rows_front(id, keep);
        true
    }

    /// Rows a series keeps after its retention cap processes `rows` rows.
    fn rows_retained_after_cap(&self, id: SeriesId, rows: usize) -> usize {
        match self.series_max_points(id) {
            // Trim past the ceiling by the hysteresis margin so the next `margin` appends are
            // free. A cap of 0 means "hold nothing"; guard the divisor rather than
            // special-casing it.
            Some(max_points) if rows > max_points => {
                max_points - (max_points / CAP_TRIM_MARGIN_DIVISOR).min(max_points)
            }
            _ => rows,
        }
    }

    /// Fit the horizontal scale to the current union of series timestamps.
    pub fn fit_content(&mut self) {
        self.time_scale.fit_content();
        self.invalidate_frame_scene();
    }

    /// Apply the public horizontal-scale spacing while keeping ownership in the headless model.
    pub fn set_bar_spacing(&mut self, spacing: f64) {
        if spacing.is_finite() && spacing > 0.0 {
            self.time_scale.set_bar_spacing(spacing);
            self.invalidate_frame_scene();
        }
    }

    /// Apply the public horizontal-scale right offset in logical bars.
    pub fn set_right_offset(&mut self, offset: f64) {
        if offset.is_finite() {
            self.time_scale.set_right_offset(offset);
            self.invalidate_frame_scene();
        }
    }

    /// reference `timeScale.timeVisible`: show the time of day in axis/crosshair labels.
    pub fn set_time_visible(&mut self, visible: bool) {
        self.time_visible = visible;
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.visible`: reserve/collapse the whole time-axis strip. Distinct from
    /// [`Self::set_time_visible`], which only governs label content (reference
    /// time-scale-options-defaults.ts keeps the two flags separate).
    pub fn set_time_axis_visible(&mut self, visible: bool) {
        self.time_axis_visible = visible;
        self.invalidate_frame_all();
    }

    /// reference `timeScale.ticksVisible`: tick marks beside the time-axis labels.
    pub fn set_time_ticks_visible(&mut self, visible: bool) {
        self.time_ticks_visible = visible;
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.minimumHeight` (CSS px; non-negative, finite): floor for the strip
    /// height — chart-widget.ts `Math.max(optimalHeight(), minimumHeight)`.
    pub fn set_time_axis_minimum_height(&mut self, height: f64) {
        if height.is_finite() && height >= 0.0 {
            self.time_axis_minimum_height = height;
            self.invalidate_frame_all();
        }
    }

    /// reference `timeScale.tickMarkMaxCharacterLength`: 0 restores the default 8, matching the reference's
    /// `tickMarkMaxCharacterLength || defaultTickMarkMaxCharacterLength` (time-scale.ts:635).
    pub fn set_tick_mark_max_character_length(&mut self, n: u32) {
        self.tick_mark_max_character_length = if n == 0 { 8 } else { n };
        self.invalidate_frame_scene();
    }

    /// The reserved time-axis strip height in media px (reference chart-widget.ts
    /// `_adjustSizeImpl`): zero when the strip is hidden, else the shared-metrics auto height
    /// (axis text plus border, tick allowance, and vertical padding, even-snapped — 22 CSS px
    /// at the default font) floored at `timeScale.minimumHeight`. Hosts subtract this from the
    /// chart height for the pane content area and report it from their `time_scale_height()`
    /// gestures getter.
    pub fn time_axis_height(&self) -> f64 {
        if self.time_axis_visible {
            self.axis_metrics()
                .time_strip_height()
                .max(self.time_axis_minimum_height)
        } else {
            0.0
        }
    }

    /// Set/clear the hovered pane separator (reference pane-separator.ts hover handle). Mirrored
    /// into the next axis frame; hosts repaint to show the band.
    pub fn set_separator_hover(&mut self, index: Option<usize>) {
        self.separator_hover = index;
        self.invalidate_frame_overlay();
    }

    /// Drag the separator below pane `index` by `delta_css` logical pixels. Positive deltas grow
    /// the pane above and shrink the pane below. Current heights become stretch factors first so
    /// unaffected panes hold their size; both adjacent panes retain the reference 24px minimum.
    pub fn drag_pane_separator(&mut self, index: usize, delta_css: f64) {
        if index + 1 >= self.panes.len() || !delta_css.is_finite() {
            return;
        }
        const MIN_PANE_HEIGHT: f64 = 24.0;
        for pane in &mut self.panes {
            pane.stretch_factor = pane.height.max(1.0);
        }
        let top = self.panes[index].height;
        let bottom = self.panes[index + 1].height;
        let combined = top + bottom;
        let mut new_top = (top + delta_css).clamp(
            MIN_PANE_HEIGHT,
            (combined - MIN_PANE_HEIGHT).max(MIN_PANE_HEIGHT),
        );
        let mut new_bottom = combined - new_top;
        // The top clamp alone can leave the bottom a float-dust epsilon below the minimum
        // (`combined - new_top`), violating the invariant the overlap and tiling asserts rely
        // on. Pin sub-epsilon violations back to the bound; anything larger keeps the
        // reference behavior (in particular the tiny-content path where the top wins).
        if new_bottom < MIN_PANE_HEIGHT && new_bottom > MIN_PANE_HEIGHT - 1e-9 {
            new_bottom = MIN_PANE_HEIGHT;
            new_top = combined - new_bottom;
        }
        self.panes[index].stretch_factor = new_top;
        self.panes[index + 1].stretch_factor = new_bottom;
        self.invalidate_frame_all();
    }

    /// reference `timeScale.secondsVisible`: include seconds when `time_visible` is set.
    pub fn set_seconds_visible(&mut self, visible: bool) {
        self.seconds_visible = visible;
        self.invalidate_frame_scene();
    }

    /// industry-standard bid/ask: push the current quotes for a series. `None` hides that
    /// side. Lines and chips render only while the series' `bid_ask_visible` option holds.
    pub fn set_bid_ask(&mut self, id: SeriesId, bid: Option<f64>, ask: Option<f64>) {
        if let Some(series) = self.series.iter_mut().find(|s| s.id == id && !s.removed) {
            series.bid = bid.filter(|v| v.is_finite());
            series.ask = ask.filter(|v| v.is_finite());
            self.invalidate_frame_scene();
        }
    }

    /// reference `timeScale.minBarSpacing`.
    pub fn set_min_bar_spacing(&mut self, spacing: f64) {
        self.time_scale.set_min_bar_spacing(spacing);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.maxBarSpacing` (CSS px; 0 restores the default half-width cap).
    pub fn set_max_bar_spacing(&mut self, spacing: f64) {
        self.time_scale.set_max_bar_spacing(spacing);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale().applyOptions({ barSpacing })`: write the option and apply it live.
    pub fn apply_bar_spacing_option(&mut self, spacing: f64) {
        self.time_scale.apply_bar_spacing_option(spacing);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale().applyOptions({ rightOffset })`: write the option and apply it live.
    pub fn apply_right_offset_option(&mut self, offset: f64) {
        self.time_scale.apply_right_offset_option(offset);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.rightOffsetPixels`: pin the right offset in pixels (converted to bars
    /// through the current bar spacing, then preserved across zoom).
    pub fn set_right_offset_pixels(&mut self, pixels: f64) {
        self.time_scale.set_right_offset_pixels(pixels);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.fixLeftEdge`.
    pub fn set_fix_left_edge(&mut self, fix: bool) {
        self.time_scale.set_fix_left_edge(fix);
    }

    /// reference `timeScale.fixRightEdge`.
    pub fn set_fix_right_edge(&mut self, fix: bool) {
        self.time_scale.set_fix_right_edge(fix);
    }

    /// reference `timeScale.lockVisibleTimeRangeOnResize`.
    pub fn set_lock_visible_time_range_on_resize(&mut self, lock: bool) {
        self.time_scale.set_lock_visible_time_range_on_resize(lock);
    }

    /// reference `timeScale.rightBarStaysOnScroll`.
    pub fn set_right_bar_stays_on_scroll(&mut self, stays: bool) {
        self.time_scale.set_right_bar_stays_on_scroll(stays);
    }

    /// Aeris `timeScale.lockVisibleLogicalRange` (default false): hold the visible logical range
    /// exactly across data updates and resizes, e.g. a fixed full-session intraday view.
    pub fn set_lock_visible_logical_range(&mut self, lock: bool) {
        self.time_scale.set_lock_visible_logical_range(lock);
        self.invalidate_frame_scene();
    }

    /// reference `timeScale.shiftVisibleRangeOnNewBar` (default true): when the last bar is
    /// visible, the visible range follows newly appended bars instead of compensating the
    /// right offset (chart-model.ts:968-983).
    pub fn set_shift_visible_range_on_new_bar(&mut self, shift: bool) {
        self.time_scale.set_shift_visible_range_on_new_bar(shift);
    }

    /// reference `timeScale.allowShiftVisibleRangeOnWhitespaceReplacement` (default false): also
    /// shift when the new bar replaces an existing whitespace time point.
    pub fn set_allow_shift_visible_range_on_whitespace_replacement(&mut self, allow: bool) {
        self.time_scale
            .set_allow_shift_visible_range_on_whitespace_replacement(allow);
    }

    /// reference `timeScale.allowBoldLabels` (default true): bold the major time tick labels.
    pub fn set_allow_bold_labels(&mut self, allow: bool) {
        self.time_scale.set_allow_bold_labels(allow);
    }

    /// reference `chart.setCrosshairPosition(price, time, series)` (chart-model.ts
    /// `setAndSaveSyntheticPosition`): position the crosshair at a data point without a DOM
    /// event. The time must land exactly on a merged time point (false otherwise); x is that
    /// bar's coordinate and y the price converted through the given series' price scale (the
    /// queued sync event carries a price on the pane's default scale, see
    /// [`CrosshairSyncPosition`]).
    /// Works headless — the next built frame draws it; hosts emit their crosshair event.
    pub fn set_crosshair_position(&mut self, price: f64, time: f64, series_id: SeriesId) -> bool {
        if !price.is_finite() || self.is_series_removed(series_id) {
            return false;
        }
        if !self.series.iter().any(|s| s.id == series_id) {
            return false;
        }
        // Ordinary time bars require validated whole-second timestamps. A logical bar sequence
        // accepts full-resolution seconds because its identity is carried by the sequence sidecar.
        if self.sequence_points().is_none() && validate_timestamp(time).is_err() {
            return false;
        }
        let Some(index) = self.time_to_index(time, false) else {
            return false;
        };
        let Some(y) = self.series_price_to_coordinate(series_id, price) else {
            return false;
        };
        let x = self.time_scale.index_to_coordinate(index);
        self.crosshair = Some((x, y));
        let pane_index = self
            .series_entry(series_id)
            .map_or(0, |series| series.pane_index);
        // The synced price is a price on the pane's default scale, which is what a linked chart
        // converts. The raw host price is kept only when it already is one; a series on another
        // scale (or on another percentage/indexed base) is re-read from the crosshair y so the
        // receiver lands on the same line.
        let sync_price = if self.series_price_is_on_pane_default_scale(series_id) {
            price
        } else {
            self.pane_coordinate_to_price(pane_index, y)
                .unwrap_or(price)
        };
        self.queue_sync_event(ChartSyncEventKind::Crosshair {
            position: CrosshairSyncPosition {
                time,
                price: sync_price,
                pane_index,
            },
        });
        self.invalidate_frame_overlay();
        true
    }

    /// Whether a price on `series_id` is already a price on its pane's default scale: the series
    /// sits on that scale and, in the modes that read a per-series base (percentage and indexed),
    /// shares the default series' base.
    fn series_price_is_on_pane_default_scale(&self, series_id: SeriesId) -> bool {
        let Some(series) = self.series_entry(series_id) else {
            return false;
        };
        let pane_index = series.pane_index;
        if series.price_scale_target != self.pane_default_scale_target(pane_index) {
            return false;
        }
        let series_based = self
            .price_scale_for(pane_index, series.price_scale_target)
            .is_some_and(|scale| {
                matches!(
                    scale.mode(),
                    PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100
                )
            });
        if !series_based {
            return true;
        }
        let Some((from, _)) = self.visible_range_for_frame() else {
            return false;
        };
        self.series_base_value(series_id, from) == Some(self.pane_default_scale(pane_index, from).1)
    }

    /// reference `chart.clearCrosshairPosition`. The engine keeps a single stored position — the
    /// reference offset coords *are* the position here (a synthetic set saves (x, y) and the frame
    /// builder re-derives the snapped index from that stored x, exactly like the reference's
    /// `updateCrosshair` re-deriving from the saved offset) — so clearing it leaves nothing
    /// a scale change could resurrect.
    pub fn clear_crosshair_position(&mut self) {
        self.clear_crosshair_at();
        self.queue_sync_event(ChartSyncEventKind::ClearCrosshair);
    }

    fn queue_sync_event(&mut self, kind: ChartSyncEventKind) {
        self.sync_revision = self.sync_revision.wrapping_add(1).max(1);
        if self.sync_events.len() >= 64 {
            self.sync_events.pop_front();
        }
        self.sync_events.push_back(ChartSyncEvent {
            source: "local".to_string(),
            revision: self.sync_revision,
            kind,
        });
    }

    pub fn set_sync_mismatch_policy(&mut self, policy: SyncMismatchPolicy) {
        self.sync_mismatch_policy = policy;
    }

    #[must_use]
    pub fn crosshair_sync_position(&self) -> Option<CrosshairSyncPosition> {
        let (x, y) = self.crosshair?;
        let logical = self.time_scale.coordinate_to_index(x);
        let time = self.axis_time_seconds_at(logical as usize)?;
        // The crosshair y is chart-content space and every scale already applies its own pane
        // offset, so y goes to the scale untouched. A separator resolves to the pane above.
        let pane_index = self.pane_index_at_y(y);
        Some(CrosshairSyncPosition {
            time,
            price: self.pane_coordinate_to_price(pane_index, y)?,
            pane_index,
        })
    }

    pub fn apply_external_crosshair(&mut self, position: Option<CrosshairSyncPosition>) -> bool {
        let Some(position) = position else {
            let changed = self.crosshair.take().is_some();
            if changed {
                self.invalidate_frame_overlay();
            }
            return changed;
        };
        let Some(index) = self.time_to_index(
            position.time,
            matches!(self.sync_mismatch_policy, SyncMismatchPolicy::Nearest),
        ) else {
            return false;
        };
        let Some(pane) = self.panes.get(position.pane_index) else {
            return false;
        };
        let (top, bottom) = (pane.top, pane.top + pane.height);
        let Some(y) = self.pane_price_to_coordinate(position.pane_index, position.price) else {
            return false;
        };
        // The crosshair y is one chart-content value: keep it inside the requested pane so a
        // price outside that pane's range sits on its edge instead of drawing in a neighbour.
        let next = (
            self.time_scale.index_to_coordinate(index),
            y.max(top).min(bottom),
        );
        let changed = self.crosshair != Some(next);
        self.crosshair = Some(next);
        if changed {
            self.invalidate_frame_overlay();
        }
        changed
    }

    #[must_use]
    pub fn take_sync_events(&mut self) -> Vec<ChartSyncEvent> {
        self.sync_events.drain(..).collect()
    }

    /// Host-pushed "all scaling and scrolling disabled" aggregate (reference
    /// `_isAllScalingAndScrollingDisabled`, time-scale.ts:975-986): label alignment only in the
    /// reference — the scale math keeps reading the raw fix-edge options, so a non-interactive
    /// chart never reacts to resizes.
    pub fn set_interaction_disabled(&mut self, disabled: bool) {
        self.time_scale.set_interaction_disabled(disabled);
    }

    pub fn bar_spacing(&self) -> f64 {
        self.time_scale.bar_spacing()
    }

    pub fn right_offset(&self) -> f64 {
        self.time_scale.right_offset()
    }

    /// Current distance, in logical bars, from the latest data point to the right edge.
    pub fn scroll_position(&self) -> f64 {
        self.time_scale.right_offset()
    }

    /// Move the latest data point to `position` logical bars from the right edge. Animation is a
    /// host scheduling concern; this headless operation applies the target state immediately.
    pub fn scroll_to_position(&mut self, position: f64) {
        self.set_right_offset(position);
    }

    /// The real-time edge position: the configured `right_offset` option, exactly the target of
    /// reference `scrollToRealTime` (time-scale.ts:824-826). Like the reference, a configured
    /// `right_offset_pixels` is not consulted here.
    pub fn real_time_scroll_position(&self) -> f64 {
        self.time_scale.options().right_offset
    }

    /// Restore the real-time edge at the configured right offset. Headless callers get the
    /// target state immediately; browser hosts animate through
    /// [`Self::start_real_time_scroll_animation`] like the reference.
    pub fn scroll_to_real_time(&mut self) {
        self.cancel_scroll_animation();
        self.set_right_offset(self.real_time_scroll_position());
    }

    /// Animate to the real-time edge (reference `scrollToRealTime` animates over
    /// `DefaultAnimationDuration`, time-scale.ts:31, 824-848). The host drives
    /// [`Self::scroll_animation_tick`] from its frame scheduler.
    pub fn start_real_time_scroll_animation(&mut self, duration_ms: f64, now_ms: f64) {
        self.start_scroll_animation(self.real_time_scroll_position(), duration_ms, now_ms);
    }

    /// Restore the configured default bar spacing and right offset.
    pub fn reset_time_scale(&mut self) {
        self.time_scale.restore_default();
        self.invalidate_frame_scene();
    }

    /// Restore autoscale on every price scale in the chart. Comparison scales are one visual
    /// group from the user's perspective, so a price reset never leaves a neighboring symbol in
    /// a stale manual range.
    pub fn reset_price_scales(&mut self) {
        for pane in &mut self.panes {
            pane.price_scale.set_auto_scale(true);
            pane.left_scale.set_auto_scale(true);
            pane.overlay_scale.set_auto_scale(true);
            for entry in &mut pane.named_scales {
                entry.scale.set_auto_scale(true);
            }
        }
        // Autoscale changes affect ranges, marker margins, coordinates, axes, and retained
        // geometry. Rebuild the complete frame contract on the next repaint.
        self.invalidate_frame_all();
    }

    /// Restore autoscale only on the requested pane-local scale. Axis double-click uses this
    /// targeted reset; the explicit reset-view command deliberately retains the chart-wide reset.
    pub fn reset_price_scale(&mut self, pane: usize, target: PriceScaleTarget) {
        let Some(scale) = self.price_scale_for_mut(pane, target) else {
            return;
        };
        scale.set_auto_scale(true);
        self.invalidate_frame_all();
    }

    /// industry-standard "reset view" button semantics in one action: the time scale returns
    /// to its configured defaults (reference `resetTimeScale`) AND every pane's price scales
    /// re-enable autoscale. Axis double-click deliberately uses the targeted method above.
    /// The next frame's autoscale pass recalculates the visible ranges, so a manually
    /// contracted or over-zoomed price scale fits the data again.
    pub fn reset_view(&mut self) {
        self.reset_time_scale();
        self.apply_reset_right_margin();
        self.reset_price_scales();
    }

    /// Leave breathing room after the last bar on a view reset.
    ///
    /// `reset_time_scale` restores the reference default right offset (0), which pins the newest
    /// bar against the price axis and puts the live-price cluster on top of the data. The public reference
    /// resets to a visible right margin instead, so the product-level reset adds one worth
    /// [`RESET_RIGHT_MARGIN_FRACTION`] of the plot width — a fraction rather than a bar count, so
    /// the gap looks the same at any window size or zoom. The reference `resetTimeScale`
    /// semantics stay untouched for hosts that call it directly.
    fn apply_reset_right_margin(&mut self) {
        /// Share of the plot width left empty after the last bar by a view reset.
        const RESET_RIGHT_MARGIN_FRACTION: f64 = 0.10;
        let spacing = self.time_scale.bar_spacing();
        if spacing <= 0.0 || self.pane_w <= 0.0 {
            return;
        }
        let offset = self.pane_w * RESET_RIGHT_MARGIN_FRACTION / spacing;
        if offset.is_finite() && offset > 0.0 {
            self.set_right_offset(offset);
        }
    }

    /// Deep-merge a JSON options patch into the chart options store (reference `applyOptions`
    /// semantics) and apply the runtime-affecting fields: the crosshair mode, plus any
    /// behavioral `timeScale` keys routed to the core scale through the same setters as the
    /// public time-scale API. Returns the parse error for a malformed patch.
    pub fn apply_options(&mut self, patch_json: &str) -> Result<(), serde_json::Error> {
        let patch: serde_json::Value = serde_json::from_str(patch_json)?;
        // Exchange-time keys validate before anything mutates so a rejected schedule never
        // reaches the options store (and therefore persistence).
        let exchange_time = Self::parse_exchange_time_patch(&patch)
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        let tick_marks = time_tick_marks_api::parse_time_tick_marks_patch(&patch)
            .map_err(<serde_json::Error as serde::de::Error>::custom)?;
        self.options.apply(&patch);
        // Re-derive runtime state that isn't read straight from the store each frame.
        self.crosshair_mode = crosshair_mode_from_u8(self.options.get().crosshair.mode);
        self.route_time_scale_patch(&patch);
        self.route_price_scale_patch(&patch);
        self.route_localization_patch(&patch);
        self.apply_exchange_time_patch(exchange_time);
        if let Some(marks) = tick_marks {
            self.set_time_tick_marks(marks)
                .expect("time tick marks were validated before the options patch applied");
        }
        self.invalidate_frame_all();
        Ok(())
    }

    /// Switch all chart cosmetics using Aeris's canonical style-token source.
    pub fn set_theme(&mut self, theme: ChartTheme) {
        self.theme = theme;
        let patch = chart_theme_patch(theme);
        self.options.apply(&patch);
        self.route_price_scale_patch(&patch);
        self.invalidate_frame_all();
    }

    /// Restore Aeris-owned chart and series styling without touching view/runtime state.
    ///
    /// This is deliberately distinct from [`Self::reset_view`]: live time/price ranges, scale
    /// modes/margins/autoscale state, data, panes, drawings, indicators, series visibility/title,
    /// pane/scale bindings, price formatting, quotes, and footprint aggregation all survive. Visual
    /// defaults come from the current canonical theme and semantic follow/unset states are restored.
    pub fn reset_style_to_defaults(&mut self) {
        self.reset_style_to_theme_defaults(self.theme);
    }

    /// Theme-aware form used by hosts whose selected theme lives outside the headless engine.
    pub fn reset_style_to_theme_defaults(&mut self, theme: ChartTheme) {
        self.theme = theme;
        self.options.reset_style_to_defaults(theme);

        for pane in &mut self.panes {
            for scale in pane.scales_mut() {
                scale.reset_style_to_defaults();
            }
        }
        for series in &mut self.series {
            if !series.removed {
                series.reset_style_to_defaults();
            }
        }
        self.reset_indicator_output_styles_to_defaults();

        self.invalidate_frame_all();
    }

    /// Route the behavioral keys of a `timeScale` options patch to the core scale (reference
    /// `applyOptions({ timeScale })`, time-scale.ts:381-420). Only keys present in this patch
    /// are applied — the merged store is never re-read here, so an unrelated patch leaves the
    /// live scale state untouched.
    fn route_time_scale_patch(&mut self, patch: &serde_json::Value) {
        let Some(time_scale) = patch
            .get("timeScale")
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };
        let number = |key: &str| time_scale.get(key).and_then(serde_json::Value::as_f64);
        let flag = |key: &str| time_scale.get(key).and_then(serde_json::Value::as_bool);
        // reference ordering (time-scale.ts:384-407): edge fixes first, then bar spacing, right
        // offset, and rightOffsetPixels (which converts through the just-applied spacing);
        // the spacing constraints come last since each re-corrects spacing and offset.
        if let Some(fix) = flag("fixLeftEdge") {
            self.set_fix_left_edge(fix);
        }
        if let Some(fix) = flag("fixRightEdge") {
            self.set_fix_right_edge(fix);
        }
        if let Some(spacing) = number("barSpacing") {
            self.apply_bar_spacing_option(spacing);
        }
        if let Some(offset) = number("rightOffset") {
            self.apply_right_offset_option(offset);
        }
        if let Some(pixels) = number("rightOffsetPixels") {
            self.set_right_offset_pixels(pixels);
        }
        if let Some(spacing) = number("minBarSpacing") {
            self.set_min_bar_spacing(spacing);
        }
        if let Some(spacing) = number("maxBarSpacing") {
            self.set_max_bar_spacing(spacing);
        }
        if let Some(visible) = flag("timeVisible") {
            self.set_time_visible(visible);
        }
        if let Some(visible) = flag("secondsVisible") {
            self.set_seconds_visible(visible);
        }
        // Strip cosmetics (reference timeScale options distinct from the label flags): `visible`
        // reserves the whole strip; the others shape its height and tick chrome.
        if let Some(visible) = flag("visible") {
            self.set_time_axis_visible(visible);
        }
        if let Some(visible) = flag("ticksVisible") {
            self.set_time_ticks_visible(visible);
        }
        if let Some(height) = number("minimumHeight") {
            self.set_time_axis_minimum_height(height);
        }
        if let Some(n) = time_scale
            .get("tickMarkMaxCharacterLength")
            .and_then(serde_json::Value::as_u64)
        {
            self.set_tick_mark_max_character_length(n.min(u32::MAX as u64) as u32);
        }
        if let Some(lock) = flag("lockVisibleTimeRangeOnResize") {
            self.set_lock_visible_time_range_on_resize(lock);
        }
        if let Some(stays) = flag("rightBarStaysOnScroll") {
            self.set_right_bar_stays_on_scroll(stays);
        }
        if let Some(lock) = flag("lockVisibleLogicalRange") {
            self.set_lock_visible_logical_range(lock);
        }
        if let Some(shift) = flag("shiftVisibleRangeOnNewBar") {
            self.set_shift_visible_range_on_new_bar(shift);
        }
        if let Some(allow) = flag("allowShiftVisibleRangeOnWhitespaceReplacement") {
            self.set_allow_shift_visible_range_on_whitespace_replacement(allow);
        }
        if let Some(allow) = flag("allowBoldLabels") {
            self.set_allow_bold_labels(allow);
        }
    }

    /// Route a `localization` options patch: `dateFormat` drives the crosshair time label
    /// (reference chart-options-defaults.ts:34-37). `locale` is handled by hosts that can resolve
    /// month names (the wasm layer intercepts it before delegating here); the store keeps
    /// both keys for the options round-trip either way.
    /// Route the scale-held keys of a `leftPriceScale`/`rightPriceScale` patch to every
    /// pane's corresponding scale — reference pane.ts `applyScaleOptions` applies the chart-level
    /// groups to all panes. The strip cosmetics (`visible`, borders) stay in the options
    /// store and are read at render time. Only keys present in this patch are applied.
    fn route_price_scale_patch(&mut self, patch: &serde_json::Value) {
        for (group_key, target) in [
            ("leftPriceScale", PriceScaleTarget::Left),
            ("rightPriceScale", PriceScaleTarget::Right),
        ] {
            let Some(group) = patch.get(group_key).and_then(serde_json::Value::as_object) else {
                continue;
            };
            let flag = |key: &str| group.get(key).and_then(serde_json::Value::as_bool);
            for pane in &mut self.panes {
                let scale = match target {
                    PriceScaleTarget::Left => &mut pane.left_scale,
                    PriceScaleTarget::Right => &mut pane.price_scale,
                    PriceScaleTarget::Overlay | PriceScaleTarget::Named(_) => continue,
                };
                if let Some(align) = flag("alignLabels") {
                    scale.set_align_labels(align);
                }
                if let Some(visible) = flag("ticksVisible") {
                    scale.set_ticks_visible(visible);
                }
                if let Some(entire) = flag("entireTextOnly") {
                    scale.set_entire_text_only(entire);
                }
                if let Some(width) = group
                    .get("minimumWidth")
                    .and_then(serde_json::Value::as_f64)
                {
                    scale.set_minimum_width(width);
                }
                if let Some(value) = group.get("textColor") {
                    if value.is_null() {
                        scale.set_text_color(None);
                    } else if let Some(css) = value.as_str() {
                        scale.set_text_color((!css.is_empty()).then(|| css.to_string()));
                    }
                }
                if let Some(bold) = flag("boldRoundLabels") {
                    scale.set_bold_round_labels(bold);
                }
                apply_chart_tick_mark_options(scale, group);
            }
        }
    }

    fn route_localization_patch(&mut self, patch: &serde_json::Value) {
        let Some(localization) = patch
            .get("localization")
            .and_then(serde_json::Value::as_object)
        else {
            return;
        };
        if let Some(pattern) = localization.get("dateFormat").and_then(|v| v.as_str()) {
            self.set_date_format(pattern);
        }
    }

    /// All time-scale options as a snake_case JSON object: the `TimeScaleOptions` fields from
    /// the core scale plus the engine-held `timeVisible`/`secondsVisible` label flags and the
    /// strip cosmetics (`visible`, `ticks_visible`, `minimum_height`,
    /// `tick_mark_max_character_length`). Backs
    /// the TS time-scale handle's `options()`. Values are the *configured* options (reference
    /// `timeScale().options()` semantics): `applyOptions` writes them, scroll/zoom gestures
    /// only move the live scale.
    pub fn time_scale_options_json(&self) -> String {
        let options = self.time_scale.options();
        serde_json::json!({
            "bar_spacing": options.bar_spacing,
            "right_offset": options.right_offset,
            "min_bar_spacing": options.min_bar_spacing,
            "max_bar_spacing": options.max_bar_spacing,
            "right_offset_pixels": options.right_offset_pixels,
            "time_visible": self.time_visible,
            "seconds_visible": self.seconds_visible,
            "visible": self.time_axis_visible,
            "ticks_visible": self.time_ticks_visible,
            "minimum_height": self.time_axis_minimum_height,
            "tick_mark_max_character_length": self.tick_mark_max_character_length,
            "fix_left_edge": options.fix_left_edge,
            "fix_right_edge": options.fix_right_edge,
            "lock_visible_time_range_on_resize": options.lock_visible_time_range_on_resize,
            "right_bar_stays_on_scroll": options.right_bar_stays_on_scroll,
            "shift_visible_range_on_new_bar": options.shift_visible_range_on_new_bar,
            "allow_shift_visible_range_on_whitespace_replacement": options.allow_shift_visible_range_on_whitespace_replacement,
            "allow_bold_labels": options.allow_bold_labels,
            "time_zone": self.time_zone_json(),
            "session_start": self.exchange_time.session_start_seconds(),
            "lock_visible_logical_range": options.lock_visible_logical_range,
            "tick_marks": self.time_tick_marks_json(),
        })
        .to_string()
    }

    /// X coordinate for an integer logical index, or `None` when the scale has no points.
    pub fn logical_to_coordinate(&self, logical: f64) -> Option<f64> {
        if !logical.is_finite() || self.data.merged_times().is_empty() {
            return None;
        }
        // the reference's internal indexToCoordinate returns zero for non-integer runtime input. The public
        // Logical nominal type normally prevents this, but preserving it makes the JS boundary
        // deterministic for untyped callers too.
        if logical.fract() != 0.0 {
            return Some(0.0);
        }
        Some(
            self.time_scale
                .index_to_coordinate(logical as TimePointIndex),
        )
    }

    /// Integer logical bar owning an X coordinate. Values may extend outside the data.
    pub fn coordinate_to_logical(&self, x: f64) -> Option<f64> {
        if !x.is_finite() || self.data.merged_times().is_empty() {
            return None;
        }
        Some(self.time_scale.coordinate_to_index(x) as f64)
    }

    /// Logical index for a UTC-seconds timestamp. With `find_nearest`, select the first point at
    /// or after the timestamp and clamp timestamps beyond the last point to that final point,
    /// matching the reference's lower-bound behavior.
    pub fn time_to_index(&self, time: f64, find_nearest: bool) -> Option<TimePointIndex> {
        if let Some(points) = self.sequence_points() {
            if !time.is_finite() {
                return None;
            }
            if points.is_empty() {
                return None;
            }
            let micros = (time * 1_000_000.0).round();
            if !micros.is_finite() || micros < i64::MIN as f64 || micros > i64::MAX as f64 {
                return None;
            }
            let index = points.partition_point(|point| point.open_timestamp_micros < micros as i64);
            if index < points.len() && points[index].open_timestamp_micros == micros as i64 {
                return Some(index as TimePointIndex);
            }
            return find_nearest.then(|| index.min(points.len()).saturating_sub(1) as i64);
        }
        let time = validate_timestamp(time).ok()?;
        let times = self.data.merged_times();
        if times.is_empty() {
            return None;
        }
        let index = times.partition_point(|&point| point < time);
        if index < times.len() && times[index] == time {
            return Some(index as TimePointIndex);
        }
        if !find_nearest {
            return None;
        }
        Some(index.min(times.len() - 1) as TimePointIndex)
    }

    /// X coordinate for an exact UTC-seconds timestamp.
    pub fn time_to_coordinate(&self, time: f64) -> Option<f64> {
        let index = self.time_to_index(time, false)?;
        Some(self.time_scale.index_to_coordinate(index))
    }

    /// UTC-seconds timestamp at the rounded logical index under X.
    pub fn coordinate_to_time(&self, x: f64) -> Option<f64> {
        if !x.is_finite() {
            return None;
        }
        let index = self.time_scale.coordinate_to_index(x);
        (index >= 0).then(|| self.axis_time_seconds_at(index as usize))?
    }

    pub fn visible_logical_range(&self) -> Option<(f64, f64)> {
        self.time_scale
            .visible_logical_range()
            .map(|range| (range.left(), range.right()))
    }

    pub fn set_visible_logical_range(&mut self, from: f64, to: f64) {
        if from.is_finite() && to.is_finite() && from <= to {
            self.time_scale
                .set_logical_range(LogicalRange::new(from, to));
            self.invalidate_frame_scene();
        }
    }

    /// Visible data timestamps nearest the logical window edges.
    pub fn visible_time_range(&self) -> Option<(f64, f64)> {
        let range = self.time_scale.visible_strict_range()?;
        if self.data.merged_times().is_empty() {
            return None;
        }
        let last = self.data.merged_times().len() as i64 - 1;
        let left = range.left().clamp(0, last) as usize;
        let right = range.right().clamp(0, last) as usize;
        Some((
            self.axis_time_seconds_at(left)?,
            self.axis_time_seconds_at(right)?,
        ))
    }

    /// Set the visible window to the points bracketing a UTC-seconds range.
    pub fn set_visible_time_range(&mut self, from: f64, to: f64) {
        if !from.is_finite() || !to.is_finite() || from > to {
            return;
        }
        let point_count = self.data.merged_times().len();
        if point_count == 0 {
            return;
        }
        let (left, right) = if let Some(points) = self.sequence_points() {
            let left = points
                .partition_point(|point| point.open_timestamp_micros as f64 / 1_000_000.0 < from);
            let right = points
                .partition_point(|point| point.open_timestamp_micros as f64 / 1_000_000.0 <= to);
            (left, right)
        } else {
            let times = self.data.merged_times();
            (
                times.partition_point(|&time| (time as f64) < from),
                times.partition_point(|&time| (time as f64) <= to),
            )
        };
        if right == 0 || left >= point_count {
            return;
        }
        let last = point_count - 1;
        let left = left.min(last) as i64;
        let right = (right - 1).min(last) as i64;
        if left <= right {
            self.time_scale
                .set_visible_range(StrictRange::new(left, right), false);
            self.queue_sync_event(ChartSyncEventKind::VisibleTimeRange {
                range: VisibleTimeRangeSync { from, to },
            });
            self.invalidate_frame_scene();
        }
    }

    /// Lay out stacked panes inside the chart content area. This is shared by hosts that need
    /// pane bounds before frame submission (for example, to draw axis separators).
    pub fn layout_panes(&mut self, content_h: f64) {
        let usable =
            (content_h - PANE_SEPARATOR * self.panes.len().saturating_sub(1) as f64).max(1.0);
        let total: f64 = self.panes.iter().map(|p| p.stretch_factor.max(0.01)).sum();
        let mut top = 0.0;
        let pane_count = self.panes.len();
        for (i, pane) in self.panes.iter_mut().enumerate() {
            pane.top = top;
            pane.height = usable * pane.stretch_factor.max(0.01) / total;
            pane.layout();
            top += pane.height;
            if i + 1 < pane_count {
                top += PANE_SEPARATOR;
            }
        }
    }

    fn sync_time_points(&mut self) {
        let merged_time_mapping = self.data.take_merged_time_mapping();
        let sequence_changed =
            self.data.time_points_generation() != self.synced_time_points_generation;
        if sequence_changed {
            self.invalidate_frame_scene();
        }
        // As-of overlays whose points moved with another series' data (a new bar or a moved data
        // extent) repaint without a time-point change of their own.
        for id in self.data.take_realigned() {
            self.invalidate_frame_series(id);
        }
        // Viewport compensation is decided against the scale state before the new points land
        // (viewport.rs), through the same exact mappings the drawings rebase with below.
        let viewport_right_offset =
            self.viewport_right_offset_after_sync(merged_time_mapping.as_ref());
        if let Some(mapping) = self.pending_sequence_mapping.take() {
            self.rebase_drawing_logicals_sequence(&mapping);
        } else if let Some(mapping) = merged_time_mapping.as_ref() {
            self.rebase_drawing_logicals(mapping);
        }
        self.sync_drawing_time_identity(sequence_changed);
        // Non-time footprint axes label ticks by full-resolution bar open times. Read them in
        // place; only a full weight rebuild materializes the column, so a live tip stays O(1).
        let sequence = self.sequence_points.as_deref();
        let times = self.data.merged_times();
        let tick_len = sequence.map_or(times.len(), <[BarSequencePoint]>::len);
        let tick_time = |index: usize| {
            sequence.map_or_else(
                || times[index],
                |points| points[index].open_timestamp_micros.div_euclid(1_000_000),
            )
        };
        let time_points_changed =
            self.data.time_points_generation() != self.synced_time_points_generation;
        // Only a pure tail append may extend the weights incrementally. Any union rebuild in this
        // transaction (a retention trim, a history insert, a replacement) records a mapping, and
        // the surviving weights then no longer line up with their indices.
        let appended = time_points_changed
            && merged_time_mapping.is_none()
            && tick_len > self.synced_points_len
            && self.synced_points_len > 0
            && self.synced_last_time.is_some_and(|last| {
                let time = tick_time(self.synced_points_len);
                // Several non-time bars may open within one second.
                time > last || (sequence.is_some() && time == last)
            });
        if appended {
            for index in self.synced_points_len..tick_len {
                let weight = aeris_charts_core::scale::time_tick_marks::weight_by_time_in(
                    tick_time(index),
                    tick_time(index - 1),
                    &self.exchange_time,
                ) as u8;
                self.tick_marks.push_weight(index as i64, weight);
            }
        } else if time_points_changed {
            let tick_times = (0..tick_len).map(tick_time).collect::<Vec<_>>();
            let mut weights = vec![0u8; tick_len];
            aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_in(
                &tick_times,
                &mut weights,
                0,
                &self.exchange_time,
            );
            self.tick_marks.set_weights(&weights);
        }
        self.synced_points_len = tick_len;
        self.synced_time_points_generation = self.data.time_points_generation();
        self.synced_last_time = tick_len.checked_sub(1).map(tick_time);
        let points_len = tick_len;
        // Data-layer units (chart-local row keys on a non-time sequence axis), matching the
        // unit of the compensation fallback's first-time comparison.
        self.synced_first_time = self.data.merged_times().first().copied();
        let rebase =
            self.time_scale
                .sync_points(points_len, self.data.base_index(), viewport_right_offset);
        self.rebase_view_motion(rebase);
        if merged_time_mapping.is_some() {
            self.refresh_drawing_pixel_baselines();
            self.drawing_baselines_need_frame_refresh = true;
        }
        self.prune_selection_anchor_snapshot();
        self.data.begin_merged_time_transaction();
    }

    fn sync_sequence_axis_times(&mut self) {
        let Some(points) = self.sequence_points() else {
            return;
        };
        let times = points
            .iter()
            .map(|point| point.open_timestamp_micros.div_euclid(1_000_000))
            .collect::<Vec<_>>();
        let mut weights = vec![0u8; times.len()];
        aeris_charts_core::scale::time_tick_marks::fill_weights_for_points_in(
            &times,
            &mut weights,
            0,
            &self.exchange_time,
        );
        self.tick_marks.set_weights(&weights);
        self.synced_points_len = times.len();
        self.synced_last_time = times.last().copied();
        // Keep the data-layer row-key unit; open seconds here made every later sync look like
        // a history prepend and disabled compensation on the sequence axis.
        self.synced_first_time = self.data.merged_times().first().copied();
        let rebase = self
            .time_scale
            .sync_points(times.len(), self.data.base_index(), None);
        self.rebase_view_motion(rebase);
    }

    fn clear_sequence_axis_if_unused(&mut self) {
        let has_sequence_series = !self.synthetic_series.is_empty()
            || self.series.iter().any(|series| {
                series.footprint.as_ref().is_some_and(|state| {
                    self.trade_stream(state.trade_stream_id)
                        .is_some_and(|stream| {
                            !matches!(stream.options().bars, FootprintBarAggregation::Time { .. })
                        })
                })
            })
            || self
                .trade_bar_dependents
                .iter()
                .any(|(stream_id, dependents)| {
                    !dependents.is_empty()
                        && self.trade_stream(*stream_id).is_some_and(|stream| {
                            !matches!(stream.options().bars, FootprintBarAggregation::Time { .. })
                        })
                })
            || self.trade_dependents.iter().any(|(stream_id, dependents)| {
                !dependents.is_empty()
                    && self.trade_stream(*stream_id).is_some_and(|stream| {
                        !matches!(stream.options().bars, FootprintBarAggregation::Time { .. })
                    })
            });
        if !has_sequence_series && self.sequence_points.take().is_some() {
            // The data-layer generation does not change when the sidecar is retired. Force one
            // ordinary sync so tick weights and axis endpoints stop referring to sequence labels.
            self.synced_time_points_generation = self.data.time_points_generation().wrapping_sub(1);
            self.sync_time_points();
        }
    }
}
