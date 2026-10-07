//! Versioned persistence for stable semantic chart state.
//!
//! V1 deliberately contains pane topology and built-in drawings only. Market history, series and
//! indicator definitions, custom extensions, runtime caches, retained frames, and renderer state
//! remain host-owned or derived.

use std::collections::{HashMap, HashSet};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;

use crate::drawings::{DrawingAnchorTime, DrawingPriceScale, DrawingTextHAlign, DrawingTextVAlign};
use crate::{
    ChartEngine, ChartError, Drawing, DrawingAnchor, DrawingKind, DrawingPoint, ErrorCode,
    IndicatorInputSource, IndicatorKind, IndicatorOutputStyle, Pane, PaneId, SeriesId,
};

pub const PERSISTENCE_SCHEMA_VERSION: u32 = 1;
pub const PERSISTENCE_SCHEMA_VERSION_GENERAL: u32 = 2;
pub const PERSISTENCE_SCHEMA_VERSION_STUDIES: u32 = 3;
pub const PERSISTENCE_MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const PERSISTENCE_MAX_GENERAL_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
pub const PERSISTENCE_MAX_PANES: usize = 64;
pub const PERSISTENCE_MAX_DRAWINGS: usize = 10_000;
pub const PERSISTENCE_MAX_POINTS_PER_DRAWING: usize = crate::drawings::MAX_DRAWING_POINTS;
pub const PERSISTENCE_MAX_TOTAL_POINTS: usize = 250_000;
pub const PERSISTENCE_MAX_INDICATORS: usize = 256;
const MAX_TEXT_BYTES: usize = crate::MAX_DRAWING_TEXT_BYTES;
const MAX_TOTAL_TEXT_BYTES: usize = 1_048_576;
const MAX_COLOR_BYTES: usize = 256;
const MAX_STYLE_NUMBER: f64 = 1_000.0;
/// Revision of the drawing catalog a document's anchors and kind names follow: 2 is
/// AerisTerminal's B8 catalog (upstream kind names and anchor contracts). Every export writes it
/// as `drawing_catalog`; a document without it was written before the fork adopted that catalog
/// (by the fork, whose B8 tools are converted on load, or by an upstream pin, which loads as is).
const DRAWING_CATALOG_REVISION: u32 = 2;
/// The fork's own-line tools, which AerisTerminal's catalog does not have: a document without the
/// catalog marker that names one was written by the fork.
const OWN_LINE_KIND_NAMES: [&str; 7] = [
    "horizontal_segment",
    "vertical_ray",
    "vertical_segment",
    "price_line",
    "price_channel",
    "simple_tag",
    "simple_annotation",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistenceRestoreResult {
    pub schema_version: u32,
    pub panes: usize,
    pub drawings: usize,
    pub points: usize,
}

/// Release-benchmark evidence for the bounded restore stages. This is not a stable product API.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub struct PersistenceRestoreProfile {
    pub restore: PersistenceRestoreResult,
    pub parse_ns: u64,
    pub validation_ns: u64,
    pub semantic_install_ns: u64,
    pub index_rebuild_ns: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct InstallProfile {
    semantic_install_ns: u64,
    index_rebuild_ns: u64,
}

/// Fully parsed and validated V1 state. Fields are private so installation cannot bypass
/// validation. This split is also the release benchmark seam for parse/validation vs install.
#[derive(Debug)]
#[doc(hidden)]
pub struct ValidatedStateV1 {
    panes: Vec<ValidatedPane>,
    drawings: Vec<Drawing>,
    drawing_anchor_times: HashMap<u32, Vec<Option<DrawingAnchorTime>>>,
    drawing_price_basis: Option<String>,
    hidden_mark_groups: Vec<String>,
    max_drawing_id: u32,
    max_persistent_pane_id: u32,
    points: usize,
}

#[derive(Debug)]
struct ValidatedPane {
    persistent_id: u32,
    stretch_factor: f64,
    preserve_empty: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StateV1 {
    schema: String,
    schema_version: u32,
    panes: Vec<PaneV1>,
    drawings: Vec<DrawingV1>,
    /// Host-defined price basis of the drawing prices (optional; no schema bump).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_price_basis: Option<String>,
    /// [`DRAWING_CATALOG_REVISION`] of the drawings; absent in documents written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_catalog: Option<u32>,
    /// Hidden timeline-mark groups (optional; no schema bump). Marks themselves never persist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden_mark_groups: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StateV2 {
    schema: String,
    schema_version: u32,
    panes: Vec<PaneV2>,
    drawings: Vec<DrawingV1>,
    axes: Vec<crate::GeneralAxisOptions>,
    datasets: Vec<DatasetV2>,
    series: Vec<SeriesV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    references: Vec<crate::GeneralReferenceOptions>,
    chart_options: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_price_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_catalog: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden_mark_groups: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StateV3 {
    schema: String,
    schema_version: u32,
    panes: Vec<PaneV1>,
    drawings: Vec<DrawingV1>,
    #[serde(default)]
    indicators: Vec<IndicatorV3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_price_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    drawing_catalog: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden_mark_groups: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IndicatorSourceV3 {
    Series { id: SeriesId },
    Output { study: usize, output: usize },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct IndicatorV3 {
    kind: IndicatorKind,
    source: IndicatorSourceV3,
    source_input: IndicatorInputSource,
    #[serde(default)]
    volume_source: Option<IndicatorSourceV3>,
    /// Turnover source of an amount-weighted VWAP; absent in documents without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    amount_source: Option<IndicatorSourceV3>,
    styles: Vec<IndicatorOutputStyle>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PaneV2 {
    #[serde(flatten)]
    pane: PaneV1,
    horizontal_domain: crate::HorizontalDomain,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DatasetV2 {
    id: String,
    input: crate::GeneralXyInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<Option<String>>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SeriesV2 {
    kind: crate::GeneralSeriesKind,
    pane: usize,
    dataset: String,
    x_axis_id: String,
    y_axis_id: String,
    visible: bool,
    title: String,
    color: Option<String>,
    point_radius: f64,
    #[serde(default)]
    point_markers: bool,
    #[serde(default)]
    point_symbol: crate::GeneralPointSymbol,
    #[serde(default = "default_general_line_width")]
    line_width: f64,
    #[serde(default)]
    line_style: crate::GeneralLineStyle,
    #[serde(default)]
    interpolation: crate::GeneralInterpolation,
    #[serde(default)]
    connect_missing: bool,
    #[serde(default = "default_general_fill_opacity")]
    fill_opacity: f64,
    #[serde(default)]
    baseline_value: Option<f64>,
    data_labels: bool,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    stack_id: Option<String>,
    #[serde(default)]
    stack_mode: crate::GeneralStackMode,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PaneV1 {
    id: String,
    #[serde(default = "default_stretch")]
    stretch_factor: f64,
    #[serde(default)]
    preserve_empty: bool,
}

fn default_stretch() -> f64 {
    1.0
}

fn default_general_line_width() -> f64 {
    2.0
}

fn default_general_fill_opacity() -> f64 {
    crate::DEFAULT_GENERAL_FILL_OPACITY
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DrawingV1 {
    id: u32,
    kind: String,
    pane_id: String,
    /// `{logical, price, time?}`: `time` is the anchor time identity (UTC seconds) on ordinary
    /// time charts; it is authoritative on restore whenever the chart can place it.
    anchors: Vec<DrawingAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    anchor_times_micros: Vec<Option<DrawingAnchorTime>>,
    #[serde(default)]
    style: DrawingStyleV1,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct DrawingStyleV1 {
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<crate::ProfileDrawingOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_account_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_risk_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    regression_source_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    regression_deviations: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wave_degree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bars_pattern: Option<Vec<crate::drawings::BarsPatternBar>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bars_pattern_mirror_x: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bars_pattern_mirror_y: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bars_pattern_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    locked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    z_order: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interval_visibility: Option<crate::DrawingIntervalVisibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stroke_start: Option<crate::DrawingLineCap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stroke_end: Option<crate::DrawingLineCap>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extend_left: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extend_right: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    magnet: Option<crate::DrawingMagnetMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<Vec<crate::DrawingLabelOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    levels: Option<Vec<crate::DrawingLevel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gann_fans: Option<Vec<crate::DrawingLevel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gann_arcs: Option<Vec<crate::DrawingLevel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_reverse: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_log_scale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_show_prices: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_show_values: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_show_percents: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level_label_align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price_scale_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    border_visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_labels: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    axis_bands_visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label_text_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snap_time_to_data: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_italic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_h_align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_v_align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    box_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    box_border_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    box_border_width: Option<f64>,
    /// The fork's B8 option blocks ([`crate::DrawingToolOptions`]); omitted when they equal the
    /// kind's defaults. Kept as JSON so restore sees which keys a document carried: a key that
    /// overlaps an upstream flat field moves onto that field (`take_legacy_flat_options`) before
    /// the rest is parsed, and is never written back.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_options: Option<serde_json::Value>,
}

fn pane_wire_id(id: u32) -> String {
    format!("pane-{id}")
}

fn parse_pane_wire_id(value: &str) -> Option<u32> {
    value
        .strip_prefix("pane-")?
        .parse::<u32>()
        .ok()
        .filter(|&id| id != 0)
}

fn line_style_name(style: LineStyle) -> &'static str {
    match style {
        LineStyle::Solid => "solid",
        LineStyle::Dotted => "dotted",
        LineStyle::Dashed => "dashed",
    }
}

fn parse_line_style(value: &str) -> Option<LineStyle> {
    Some(match value {
        "solid" => LineStyle::Solid,
        "dotted" => LineStyle::Dotted,
        "dashed" => LineStyle::Dashed,
        _ => return None,
    })
}

fn invalid(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

fn resource(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::ResourceLimit, message)
}

fn validate_color(value: &str, field: &str) -> Result<(), ChartError> {
    if value.len() > MAX_COLOR_BYTES {
        return Err(resource(format!("{field} exceeds {MAX_COLOR_BYTES} bytes")));
    }
    if !value.is_empty() && Color::parse_css(value).is_none() {
        return Err(invalid(format!("{field} is not a supported CSS color")));
    }
    Ok(())
}

fn validate_positive_number(value: f64, field: &str) -> Result<(), ChartError> {
    if !value.is_finite() || value <= 0.0 || value > MAX_STYLE_NUMBER {
        return Err(invalid(format!(
            "{field} must be finite and in (0, {MAX_STYLE_NUMBER}]"
        )));
    }
    Ok(())
}

fn incremental_output_count(kind: &IndicatorKind) -> usize {
    match kind {
        IndicatorKind::SwingPoints { .. } | IndicatorKind::SessionLevels { .. } => 2,
        IndicatorKind::PreviousPeriodLevels { .. } | IndicatorKind::OpeningRange { .. } => 3,
        IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. } => 1,
        IndicatorKind::Aroon { .. } => 2,
        IndicatorKind::AwesomeOscillator => 1,
        IndicatorKind::Dpo { .. } => 1,
        IndicatorKind::ChandeMomentum { .. } => 1,
        IndicatorKind::Sma { .. }
        | IndicatorKind::Ema { .. }
        | IndicatorKind::Dema { .. }
        | IndicatorKind::Tema { .. }
        | IndicatorKind::Smma { .. }
        | IndicatorKind::Hma { .. }
        | IndicatorKind::Vwma { .. }
        | IndicatorKind::StandardDeviation { .. }
        | IndicatorKind::Cci { .. }
        | IndicatorKind::WilliamsR { .. }
        | IndicatorKind::StochasticRsi { .. }
        | IndicatorKind::Momentum { .. }
        | IndicatorKind::RateOfChange { .. }
        | IndicatorKind::Rsi { .. }
        | IndicatorKind::Atr { .. }
        | IndicatorKind::Vwap
        | IndicatorKind::Obv
        | IndicatorKind::AccumulationDistribution
        | IndicatorKind::PriceVolumeTrend
        | IndicatorKind::ChaikinOscillator { .. }
        | IndicatorKind::Kama { .. }
        | IndicatorKind::McGinley { .. }
        | IndicatorKind::Choppiness { .. }
        | IndicatorKind::RelativeVolume { .. }
        | IndicatorKind::ElderForce { .. }
        | IndicatorKind::EaseOfMovement { .. }
        | IndicatorKind::HistoricalVolatility { .. }
        | IndicatorKind::MassIndex { .. }
        | IndicatorKind::CoppockCurve { .. }
        | IndicatorKind::UltimateOscillator { .. }
        | IndicatorKind::Cmf { .. }
        | IndicatorKind::Mfi { .. }
        | IndicatorKind::Wma { .. } => 1,
        IndicatorKind::VolumeOscillator { .. } => 3,
        IndicatorKind::Trix { .. }
        | IndicatorKind::Kst { .. }
        | IndicatorKind::Klinger { .. }
        | IndicatorKind::Tsi { .. }
        | IndicatorKind::Vortex { .. }
        | IndicatorKind::FisherTransform { .. } => 2,
        IndicatorKind::Volume { .. } => 2,
        IndicatorKind::Donchian { .. }
        | IndicatorKind::Keltner { .. }
        | IndicatorKind::AdxDmi { .. } => 3,
        IndicatorKind::PivotPoints { .. } => 5,
        IndicatorKind::ZigZag { .. } => 1,
        IndicatorKind::ParabolicSar => 1,
        IndicatorKind::SuperTrend { .. } => 1,
        IndicatorKind::Ichimoku => 5,
        IndicatorKind::EmaRibbon { .. } => aeris_charts_indicators::MAX_OUTPUTS,
        IndicatorKind::Bollinger { .. } => 3,
        IndicatorKind::LinearRegression { .. } => 3,
        IndicatorKind::AtrBands { .. } => 3,
        IndicatorKind::BollingerMetrics { .. } => 2,
        IndicatorKind::Envelopes { .. } => 3,
        IndicatorKind::Alma { .. } => 1,
        IndicatorKind::Macd { .. } => 3,
        IndicatorKind::Stochastic { .. } => 2,
        IndicatorKind::Kdj { .. } => 3,
        IndicatorKind::VwapBands { .. } => 5,
        IndicatorKind::KLineChart(indicator) => indicator.output_count(),
    }
}

fn validate_indicator_style(style: &IndicatorOutputStyle) -> Result<(), &'static str> {
    if style
        .line_width
        .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return Err("line width is invalid");
    }
    if style.line_style > 4 {
        return Err("line style is invalid");
    }
    for (field, color) in [
        ("line color", style.line_color.as_deref()),
        ("up color", style.up_color.as_deref()),
        ("down color", style.down_color.as_deref()),
        ("area top color", style.area_top_color.as_deref()),
        ("area bottom color", style.area_bottom_color.as_deref()),
    ] {
        if let Some(color) = color
            && (color.len() > MAX_COLOR_BYTES || Color::parse_css(color).is_none())
        {
            return Err(field);
        }
    }
    Ok(())
}

fn indicator_kind_is_valid(kind: &IndicatorKind) -> bool {
    match kind {
        IndicatorKind::SwingPoints { .. }
        | IndicatorKind::MarketStructure { .. }
        | IndicatorKind::FairValueGaps { .. }
        | IndicatorKind::OrderBlocks { .. } => super::indicators::structure_kind_is_valid(kind),
        IndicatorKind::SessionLevels { .. } | IndicatorKind::PreviousPeriodLevels { .. } => true,
        IndicatorKind::OpeningRange {
            duration_seconds, ..
        } => *duration_seconds > 0,
        IndicatorKind::Aroon { period } => *period > 0,
        IndicatorKind::AwesomeOscillator => true,
        IndicatorKind::Dpo { period } => *period > 0,
        IndicatorKind::ChandeMomentum { period } => *period > 0,
        IndicatorKind::Sma { period }
        | IndicatorKind::Ema { period, .. }
        | IndicatorKind::Dema { period, .. }
        | IndicatorKind::Tema { period, .. }
        | IndicatorKind::Smma { period }
        | IndicatorKind::Hma { period }
        | IndicatorKind::Vwma { period }
        | IndicatorKind::StandardDeviation { period }
        | IndicatorKind::Cci { period }
        | IndicatorKind::WilliamsR { period }
        | IndicatorKind::Donchian { period }
        | IndicatorKind::Rsi { period, .. }
        | IndicatorKind::Atr { period }
        | IndicatorKind::Wma { period } => *period > 0,
        IndicatorKind::PivotPoints { .. } => true,
        IndicatorKind::ZigZag { deviation_percent } => {
            deviation_percent.is_finite() && *deviation_percent > 0.0
        }
        IndicatorKind::Keltner { period, multiplier } => {
            *period > 0 && multiplier.is_finite() && *multiplier >= 0.0
        }
        IndicatorKind::AdxDmi { period } => *period > 0,
        IndicatorKind::StochasticRsi {
            rsi_period,
            stochastic_period,
        } => *rsi_period > 0 && *stochastic_period > 0,
        IndicatorKind::Momentum { period } | IndicatorKind::RateOfChange { period } => *period > 0,
        IndicatorKind::ParabolicSar => true,
        IndicatorKind::SuperTrend { period, multiplier } => {
            *period > 0 && multiplier.is_finite() && *multiplier >= 0.0
        }
        IndicatorKind::Ichimoku => true,
        IndicatorKind::EmaRibbon { periods } => periods.iter().all(|period| *period > 0),
        IndicatorKind::Bollinger {
            period, deviation, ..
        }
        | IndicatorKind::BollingerMetrics { period, deviation } => {
            *period > 0 && deviation.is_finite() && *deviation >= 0.0
        }
        IndicatorKind::Envelopes {
            period, percent, ..
        } => *period > 0 && percent.is_finite() && *percent >= 0.0,
        IndicatorKind::Alma {
            period,
            offset,
            sigma,
        } => {
            *period > 0
                && offset.is_finite()
                && (0.0..=1.0).contains(offset)
                && sigma.is_finite()
                && (0.01..=1_000_000.0).contains(sigma)
        }
        IndicatorKind::Macd {
            fast,
            slow,
            signal,
            histogram_multiplier,
            ..
        } => {
            *fast > 0
                && *slow > 0
                && *signal > 0
                && histogram_multiplier.is_finite()
                && *histogram_multiplier > 0.0
        }
        IndicatorKind::Kdj {
            period,
            k_smoothing,
            d_smoothing,
            ..
        } => *period > 0 && *k_smoothing > 0 && *d_smoothing > 0,
        IndicatorKind::Stochastic { k_period, d_period } => *k_period > 0 && *d_period > 0,
        IndicatorKind::Vwap => true,
        IndicatorKind::Obv => true,
        IndicatorKind::AccumulationDistribution | IndicatorKind::PriceVolumeTrend => true,
        IndicatorKind::ChaikinOscillator { fast, slow } => *fast > 0 && *slow > 0 && *fast < *slow,
        IndicatorKind::Klinger { fast, slow, signal } => *fast > 0 && *fast < *slow && *signal > 0,
        IndicatorKind::Kama { period, fast, slow } => *period > 0 && *fast > 0 && *fast < *slow,
        IndicatorKind::McGinley { period } => *period > 0,
        IndicatorKind::LinearRegression { period, deviation } => {
            *period > 0 && deviation.is_finite() && *deviation >= 0.0
        }
        IndicatorKind::Choppiness { period } => *period >= 2,
        IndicatorKind::AtrBands { period, multiplier } => {
            *period > 0 && multiplier.is_finite() && *multiplier >= 0.0
        }
        IndicatorKind::RelativeVolume { period } => *period > 0,
        IndicatorKind::ElderForce { period } => *period > 0,
        IndicatorKind::EaseOfMovement { period, divisor } => {
            *period > 0 && divisor.is_finite() && *divisor > 0.0
        }
        IndicatorKind::HistoricalVolatility {
            period,
            annualization,
        } => *period >= 2 && annualization.is_finite() && *annualization > 0.0,
        IndicatorKind::Trix { period, signal } => *period > 0 && *signal > 0,
        IndicatorKind::Kst {
            roc,
            smoothing,
            signal,
        } => {
            roc.iter().all(|&period| period > 0)
                && smoothing.iter().all(|&period| period > 0)
                && *signal > 0
        }
        IndicatorKind::Tsi {
            long,
            short,
            signal,
        } => *long > 0 && *short > 0 && *signal > 0,
        IndicatorKind::MassIndex {
            ema_period,
            sum_period,
        } => *ema_period > 0 && *sum_period > 0,
        IndicatorKind::Vortex { period } => *period > 0,
        IndicatorKind::CoppockCurve {
            long,
            short,
            smoothing,
        } => *long > 0 && *short > 0 && *smoothing > 0,
        IndicatorKind::FisherTransform { period } => *period > 0,
        IndicatorKind::UltimateOscillator {
            short,
            medium,
            long,
        } => *short > 0 && *medium > 0 && *long > 0,
        IndicatorKind::VolumeOscillator { fast, slow, signal } => {
            *fast > 0 && *fast < *slow && *signal > 0
        }
        IndicatorKind::Cmf { period } => *period > 0,
        IndicatorKind::Mfi { period } => *period > 0,
        IndicatorKind::Volume { period } => *period > 0,
        IndicatorKind::VwapBands {
            standard_deviation,
            percent,
            ..
        } => standard_deviation.is_finite() && percent.is_finite(),
        IndicatorKind::KLineChart(indicator) => indicator.is_valid(),
    }
}

fn source_refs_equal(left: &IndicatorSourceV3, right: &IndicatorSourceV3) -> bool {
    match (left, right) {
        (IndicatorSourceV3::Series { id: left }, IndicatorSourceV3::Series { id: right }) => {
            left == right
        }
        (
            IndicatorSourceV3::Output {
                study: left_study,
                output: left_output,
            },
            IndicatorSourceV3::Output {
                study: right_study,
                output: right_output,
            },
        ) => left_study == right_study && left_output == right_output,
        _ => false,
    }
}

fn source_is_scalar(chart: &ChartEngine, source: SeriesId) -> bool {
    chart.series_entry(source).is_some_and(|series| {
        matches!(
            series.kind,
            crate::SeriesKind::Line
                | crate::SeriesKind::Area
                | crate::SeriesKind::Histogram
                | crate::SeriesKind::Baseline
        )
    })
}

fn resolve_indicator_source(
    source: &IndicatorSourceV3,
    study: usize,
    expected_outputs: &[Vec<SeriesId>],
    chart: &ChartEngine,
) -> Result<SeriesId, ChartError> {
    match source {
        IndicatorSourceV3::Series { id } => {
            if chart.series_entry(*id).is_none() {
                return Err(invalid(format!("indicator source {id} is not live")));
            }
            Ok(*id)
        }
        IndicatorSourceV3::Output {
            study: source_study,
            output,
        } => {
            if *source_study >= study || *source_study >= expected_outputs.len() {
                return Err(invalid(
                    "indicator output source must reference an earlier study",
                ));
            }
            if *output >= expected_outputs[*source_study].len() {
                return Err(invalid("indicator output source index is out of range"));
            }
            Ok(u32::MAX)
        }
    }
}

fn remap_indicator_source(
    source: &IndicatorSourceV3,
    remapped_outputs: &[Vec<SeriesId>],
    plain_source: SeriesId,
) -> SeriesId {
    match source {
        IndicatorSourceV3::Series { .. } => plain_source,
        IndicatorSourceV3::Output { study, output } => remapped_outputs
            .get(*study)
            .and_then(|outputs| outputs.get(*output))
            .copied()
            .unwrap_or(u32::MAX),
    }
}

/// Fill a style field the document left out with a value read from elsewhere in the document.
fn fill_absent<T>(field: &mut Option<T>, value: Option<T>) {
    if field.is_none() {
        *field = value;
    }
}

/// A drawing's `tool_options` as written: omitted when they equal the kind's defaults, and without
/// the keys written as an upstream flat field instead (`take_legacy_flat_options`).
fn exported_tool_options(
    drawing: &Drawing,
    defaults: &Drawing,
) -> Result<Option<serde_json::Value>, ChartError> {
    if drawing.tool_options == defaults.tool_options {
        return Ok(None);
    }
    let mut value = serde_json::to_value(&drawing.tool_options)
        .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))?;
    let taken = crate::drawing_contract::take_legacy_flat_options(drawing.kind, &mut value, false)
        != crate::drawing_contract::LegacyFlatOptions::default();
    let emptied = taken && value.as_object().is_some_and(serde_json::Map::is_empty);
    Ok((!emptied).then_some(value))
}

/// Whether a document was written by the fork before it adopted AerisTerminal's B8 catalog. Only
/// a document without the catalog marker can be, and only one that carries something the fork
/// wrote and no upstream pin does: a fork tool name, an anchor time, a `tool_options` block, a
/// drawing price basis, an anchored text positioned by its anchor alone, or a B8 drawing without
/// a field upstream writes for every drawing of its kind ([`lacks_upstream_b8_fields`]). Such a
/// document gets the anchor conversions whose stored anchor count is the same in both catalogs,
/// and the fork's defaults for every value it omitted.
fn written_by_legacy_fork(state: &StateV1) -> bool {
    state.drawing_catalog.is_none()
        && (state.drawing_price_basis.is_some()
            || state.drawings.iter().any(|item| {
                crate::drawings::LEGACY_DRAWING_KIND_NAMES
                    .iter()
                    .any(|(name, _)| *name == item.kind)
                    || OWN_LINE_KIND_NAMES.contains(&item.kind.as_str())
                    || item.anchors.iter().any(|anchor| anchor.time.is_some())
                    || item.style.tool_options.is_some()
                    || (item.kind == DrawingKind::AnchoredText.name()
                        && item.style.screen_x.is_none())
                    || lacks_upstream_b8_fields(item)
            }))
}

/// Whether `item` is a B8 drawing stored without a field upstream's exporter writes for every
/// drawing of its kind: the level flags of a leveled tool, a regression's deviations, an icon
/// stamp's size, a bars pattern's snapshot, an Elliott wave's degree. Upstream pins before the B8
/// catalog had none of these kinds, and the fork never wrote these fields (it kept their values
/// in `tool_options` or left them at its defaults), so this tells a fork document apart even when
/// nothing else does: on a sequence axis (trade-count, volume, or range bars) the fork wrote no
/// anchor `time`, and a drawing with default options carried no `tool_options`.
fn lacks_upstream_b8_fields(item: &DrawingV1) -> bool {
    let Some(kind) = DrawingKind::from_name(&item.kind) else {
        return false;
    };
    let style = &item.style;
    (kind.has_levels() && style.level_reverse.is_none())
        || (kind == DrawingKind::RegressionTrend && style.regression_deviations.is_none())
        || (kind == DrawingKind::IconStamp && style.icon_size.is_none())
        || (kind == DrawingKind::BarsPattern && style.bars_pattern.is_none())
        || (kind.is_elliott() && style.wave_degree.is_none())
}

/// The kind a stored drawing names. The fork's `flat_top_bottom` was one tool for both upstream
/// channels: a level line through the third anchor's price across the base's bars. A level at or
/// above both base anchors is a flat top and one at or below both a flat bottom, which upstream
/// draws at that price on a non-inverted price scale (it keeps the flat line outside the base
/// anchors in screen y). A level between them crosses the base, which no flat channel draws: it
/// becomes the disjoint channel whose second line is that level ([`fork_flat_crossing_anchors`]).
/// A base on one bar keeps the flat top or bottom its third anchor is above or below.
fn stored_drawing_kind(name: &str, anchors: &[DrawingAnchor]) -> Option<DrawingKind> {
    if name == "flat_top_bottom"
        && let [a, b, c] = anchors
        && let (Some(a_logical), Some(b_logical), Some(_)) = (a.logical, b.logical, c.logical)
    {
        let (low, high) = (a.price.min(b.price), a.price.max(b.price));
        return Some(if c.price >= high {
            DrawingKind::FlatTopChannel
        } else if c.price <= low || a_logical == b_logical {
            DrawingKind::FlatBottomChannel
        } else {
            DrawingKind::DisjointChannel
        });
    }
    DrawingKind::from_name(name)
}

/// The time identity of a derived anchor at `logical`: the stored anchors' times interpolated
/// there, when every stored anchor has a time, those times are one affine function of their
/// logicals (evenly spaced bars between them, to a millionth of a bar), and `logical` lies
/// between them. Otherwise `None`, which keeps the derived anchor at its logical: time
/// arithmetic across a session gap, or past the anchors where no bar times are known, would put
/// it on another bar than the bar arithmetic its logical came from.
fn interpolated_anchor_time(anchors: &[DrawingAnchor], logical: f64) -> Option<f64> {
    let pairs = anchors
        .iter()
        .map(|anchor| Some((anchor.logical?, anchor.time?)))
        .collect::<Option<Vec<(f64, f64)>>>()?;
    let first = pairs.iter().copied().min_by(|a, b| a.0.total_cmp(&b.0))?;
    let last = pairs.iter().copied().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if !(first.0..=last.0).contains(&logical) || last.0 <= first.0 {
        return None;
    }
    let per_bar = (last.1 - first.1) / (last.0 - first.0);
    if !(per_bar.is_finite() && per_bar > 0.0) {
        return None;
    }
    let time_at = |at: f64| first.1 + (at - first.0) * per_bar;
    pairs
        .iter()
        .all(|&(at, time)| ((time - time_at(at)) / per_bar).abs() <= 1e-6)
        .then(|| time_at(logical))
}

/// One anchor of a fork drawing converted to the upstream anchor contract.
struct MigratedAnchor {
    anchor: DrawingAnchor,
    /// The stored anchor whose time identity (`time` and `anchor_times_micros`) this anchor
    /// keeps; it sits at that anchor's logical position. `None` for an anchor placed elsewhere.
    identity: Option<usize>,
}

/// A fork `flat_top_bottom` whose level crosses its base ([`stored_drawing_kind`]) as the disjoint
/// channel that draws the same two lines: the base, then the level across the base's bars at the
/// third anchor's price, each end keeping the time identity of the base anchor on its bar.
fn fork_flat_crossing_anchors(anchors: &[DrawingAnchor]) -> Vec<MigratedAnchor> {
    let at = |index: usize, price: f64| MigratedAnchor {
        anchor: DrawingAnchor {
            price,
            ..anchors[index]
        },
        identity: Some(index),
    };
    let level = anchors[2].price;
    vec![
        at(0, anchors[0].price),
        at(1, anchors[1].price),
        at(0, level),
        at(1, level),
    ]
}

/// The upstream anchors of a drawing the fork stored under its own anchor contract, or `None`
/// when the stored anchors already follow the upstream one. Conversions keyed by an anchor count
/// that upstream never stores apply to any document; those whose count is the same in both
/// catalogs apply only to a `legacy_fork` document. A conversion that needs a logical position
/// the stored anchor lacks (a time-only anchor) does not apply.
fn migrate_fork_anchors(
    kind: DrawingKind,
    anchors: &[DrawingAnchor],
    tool_options: Option<&serde_json::Value>,
    legacy_fork: bool,
) -> Option<Vec<MigratedAnchor>> {
    let keep = |index: usize| MigratedAnchor {
        anchor: anchors[index],
        identity: Some(index),
    };
    // At stored anchor `index`'s logical position (and time identity), at another price.
    let beside = |index: usize, price: f64| MigratedAnchor {
        anchor: DrawingAnchor {
            price,
            ..anchors[index]
        },
        identity: Some(index),
    };
    let placed = |logical: f64, price: f64, time: Option<f64>| MigratedAnchor {
        anchor: DrawingAnchor {
            logical: Some(logical),
            price,
            time,
        },
        identity: None,
    };
    let logicals = || {
        anchors
            .iter()
            .map(|anchor| anchor.logical)
            .collect::<Option<Vec<f64>>>()
    };
    let price = |index: usize| anchors[index].price;
    // Halfway between the first two stored anchors (times by the same weights).
    let midpoint = || {
        let logical = logicals()?;
        let time = anchors[0]
            .time
            .zip(anchors[1].time)
            .map(|(first, second)| (first + second) / 2.0);
        Some(placed(
            (logical[0] + logical[1]) / 2.0,
            (price(0) + price(1)) / 2.0,
            time,
        ))
    };
    match (kind, anchors.len()) {
        // The mirrored-slope second line through the third anchor becomes two explicit anchors.
        (DrawingKind::DisjointChannel, 3) => {
            let logical = logicals()?;
            Some(if logical[0] == logical[1] {
                vec![keep(0), keep(1), beside(2, price(0)), beside(2, price(1))]
            } else {
                let slope = (price(1) - price(0)) / (logical[1] - logical[0]);
                vec![
                    keep(0),
                    keep(1),
                    beside(0, price(2) - slope * (logical[0] - logical[2])),
                    beside(1, price(2) - slope * (logical[1] - logical[2])),
                ]
            })
        }
        // The fixed square's side came from its options; upstream stores the opposite corner.
        (DrawingKind::GannSquareFixed, 1) => {
            let logical = anchors[0].logical?;
            let gann = tool_options.and_then(|options| options.get("gann"));
            let option = |key: &str| gann.and_then(|gann| gann.get(key));
            let size = option("size_bars")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(20.0)
                .clamp(1.0, crate::MAX_GANN_SQUARE_BARS);
            let direction = if option("reverse").and_then(serde_json::Value::as_bool) == Some(true)
            {
                -1.0
            } else {
                1.0
            };
            let ratio = option("scale_ratio")
                .and_then(serde_json::Value::as_f64)
                .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
            let corner = match ratio {
                Some(ratio) => price(0) + direction * size * ratio,
                // Without a scale ratio the fork kept the square square on screen, which no price
                // per bar reproduces: the corner sits far beyond the square (the corner drag's
                // rule), so the square's time side is normally its smaller one.
                None => crate::drawings::kinds::pitchforks_gann::fixed_square_far_price(
                    price(0),
                    size,
                    direction,
                ),
            };
            Some(vec![keep(0), placed(logical + size, corner, None)])
        }
        // The fork's reversed square measured from the first anchor's time and the second's
        // price; upstream's reverse pivots on the second anchor. Swapping the anchors' prices
        // puts upstream's pivot on the fork's (the caller drops the `reverse` the swap consumed).
        (DrawingKind::GannSquare, 2) if legacy_fork && legacy_gann_reverse(tool_options) => {
            Some(vec![beside(0, price(1)), beside(1, price(0))])
        }
        // Apex and projected price; the time horizon anchor (the sector radius) has no place.
        (DrawingKind::Projection, 3) => Some(vec![keep(0), keep(2)]),
        // The priced point; the label offset anchor has no place.
        (DrawingKind::PriceNote, 2) => Some(vec![keep(0)]),
        // The pole's top starts at its foot.
        (DrawingKind::Signpost, 1) => Some(vec![keep(0), keep(0)]),
        // The box between the two anchors stays; the target places the first copied bar's close
        // where the fork's box fit drew it.
        (DrawingKind::BarsPattern, 2) => {
            let logical = logicals()?;
            let bars = tool_options
                .and_then(|options| options.get("projection_annotation"))
                .and_then(|block| block.get("bars"))
                .and_then(|bars| serde_json::from_value::<Vec<[f64; 4]>>(bars.clone()).ok())
                .map(|bars| crate::drawing_contract::legacy_bars_pattern(&bars))
                .unwrap_or_default();
            let (box_low, box_high) = (price(0).min(price(1)), price(0).max(price(1)));
            let source_low = bars.iter().map(|bar| bar.low).fold(f64::INFINITY, f64::min);
            let source_high = bars
                .iter()
                .map(|bar| bar.high)
                .fold(f64::NEG_INFINITY, f64::max);
            let target = match bars.first() {
                Some(first) if source_high > source_low => {
                    box_low
                        + (first.close - source_low) * (box_high - box_low)
                            / (source_high - source_low)
                }
                _ => box_low,
            };
            let earlier = usize::from(logical[1] < logical[0]);
            Some(vec![keep(0), keep(1), beside(earlier, target)])
        }
        // E extends the A-C side by the D-C span past D.
        (DrawingKind::PatternTriangle, 4) => {
            let logical = logicals()?;
            let e_logical = logical[3] + (logical[3] - logical[2]);
            let e_price = if logical[0] == logical[2] {
                price(2)
            } else {
                price(0)
                    + (price(2) - price(0)) * (e_logical - logical[0]) / (logical[2] - logical[0])
            };
            Some(vec![
                keep(0),
                keep(1),
                keep(2),
                keep(3),
                placed(e_logical, e_price, None),
            ])
        }
        // Upstream's three drives end at the third drive; the fork's last leg has no place.
        (DrawingKind::PatternThreeDrives, 7) => Some((0..6).map(keep).collect()),
        // The fork stored the arc's ends first and its through point last.
        (DrawingKind::Arc, 3) if legacy_fork => Some(vec![keep(0), keep(2), keep(1)]),
        // The fork's on-curve midpoint becomes the quadratic Bezier control point, an affine
        // combination of the stored anchors' bars (and of their times where those are evenly
        // spaced, [`interpolated_anchor_time`]).
        (DrawingKind::Curve, 3) if legacy_fork => {
            let logical = logicals()?;
            let control = 2.0 * logical[2] - (logical[0] + logical[1]) / 2.0;
            Some(vec![
                keep(0),
                placed(
                    control,
                    2.0 * price(2) - (price(0) + price(1)) / 2.0,
                    interpolated_anchor_time(anchors, control),
                ),
                keep(1),
            ])
        }
        // The fork's on-curve points at a third and two thirds become the cubic Bezier control
        // points, affine combinations of the stored anchors' bars (and of their times where
        // those are evenly spaced).
        (DrawingKind::DoubleCurve, 4) if legacy_fork => {
            let logical = logicals()?;
            let prices = [price(0), price(1), price(2), price(3)];
            // Weights over the stored [a, b, p, q].
            let combine = |weights: [f64; 4], values: &[f64]| {
                weights
                    .iter()
                    .zip(values)
                    .map(|(weight, value)| weight * value)
                    .sum::<f64>()
            };
            let control = |weights: [f64; 4]| {
                let at = combine(weights, &logical);
                placed(
                    at,
                    combine(weights, &prices),
                    interpolated_anchor_time(anchors, at),
                )
            };
            Some(vec![
                keep(0),
                control([-5.0 / 6.0, 1.0 / 3.0, 3.0, -1.5]),
                control([1.0 / 3.0, -5.0 / 6.0, -1.5, 3.0]),
                keep(1),
            ])
        }
        // The fork's symmetric rectangle (an axis and a half-width point) becomes an edge plus a
        // depth handle on the opposite edge.
        (DrawingKind::RotatedRectangle, 3) if legacy_fork => {
            let logical = logicals()?;
            Some(if logical[0] != logical[1] {
                let slope = (price(1) - price(0)) / (logical[1] - logical[0]);
                let delta = price(2) - (price(0) + slope * (logical[2] - logical[0]));
                vec![
                    beside(0, price(0) + delta),
                    beside(1, price(1) + delta),
                    beside(0, price(0) - delta),
                ]
            } else {
                // A vertical axis: the first edge lies on the width point's bar (and keeps its
                // time identity), the depth handle as many bars on the other side. That mirror
                // lies past the stored anchors, where their times say nothing about the bars (a
                // session gap on either side moves it), so it keeps its logical without a time.
                vec![
                    beside(2, price(0)),
                    beside(2, price(1)),
                    placed(2.0 * logical[0] - logical[2], price(0), None),
                ]
            })
        }
        // The fork turned its speed arcs around the first anchor; upstream's center is the
        // second, so the anchors swap (the half circles now open toward the other anchor rather
        // than up or down).
        (DrawingKind::FibonacciSpeedArcs, 2) if legacy_fork => Some(vec![keep(1), keep(0)]),
        // The fork centered its circles between the anchors with half their distance as the unit
        // radius; upstream centers them on the second anchor at the full distance, so the second
        // anchor moves to the midpoint.
        (DrawingKind::FibonacciCircles, 2) if legacy_fork => Some(vec![keep(0), midpoint()?]),
        // The fork's sine anchors were opposite extremes half a period apart; upstream's are a
        // zero crossing and the next extreme a quarter period on. The midpoint is that zero
        // crossing, so the wave is the same (upstream draws it from there on).
        (DrawingKind::SineLine, 2) if legacy_fork => Some(vec![midpoint()?, keep(1)]),
        _ => None,
    }
}

/// Whether a fork Gann block (`tool_options.gann`) stores `reverse: true`.
fn legacy_gann_reverse(tool_options: Option<&serde_json::Value>) -> bool {
    tool_options
        .and_then(|options| options.get("gann"))
        .and_then(|gann| gann.get("reverse"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Drop the fork `reverse` of a Gann square whose anchor conversion consumed it (the fixed
/// square's grows-downward corner, the reversed square's swapped prices), so it does not also
/// reach `level_reverse` and pivot the fans and arcs on the far corner.
fn drop_consumed_gann_reverse(kind: DrawingKind, tool_options: Option<&mut serde_json::Value>) {
    if matches!(kind, DrawingKind::GannSquare | DrawingKind::GannSquareFixed)
        && let Some(gann) = tool_options
            .and_then(|options| options.get_mut("gann"))
            .and_then(serde_json::Value::as_object_mut)
    {
        gann.remove("reverse");
    }
}

/// Convert a clipboard or drawing-sync item a fork build wrote to upstream's contracts: anchors
/// stored under an anchor count upstream never stores take the same conversions a document gets
/// (only those; the same-count ones need a document's provenance), a fork bars pattern without a
/// snapshot takes its `tool_options` bars, and a fork anchored text (its options carry no
/// `screen_x`; every upstream payload writes one) takes its pane-fraction anchor as its screen
/// position. A converted item is provably the fork's, so like a fork document it also takes the
/// fork's unstored option defaults ([`kinds::merge_legacy_fork_tool_options`], the annotations'
/// fork-form marker included; the triangle pattern's apex sides), and a fixed square's `reverse`
/// stays in its converted corner. A fork info line, whose payload carries the fork's default
/// labels (which no upstream info line has) and no `line` key, takes the `line` block that draws
/// them as one stats box; this build writes an absent block as `"line": null`, which the merge
/// keeps, so a restored fork info line whose box was removed stays without it. Items already on
/// upstream's contracts are left as they are.
///
/// [`kinds::merge_legacy_fork_tool_options`]: crate::drawings::kinds::merge_legacy_fork_tool_options
pub(crate) fn migrate_fork_payload_item(item: &mut crate::DrawingClipboardItem) {
    let mut converted = false;
    if !item.kind.valid_point_count(item.points.len()) {
        let tool_options = item.options.get("tool_options");
        if let Some(migrated) = migrate_fork_anchors(item.kind, &item.points, tool_options, false) {
            item.points = migrated.into_iter().map(|anchor| anchor.anchor).collect();
            converted = true;
        }
    }
    if item.kind == DrawingKind::BarsPattern && item.bars_pattern.is_none() {
        item.bars_pattern = item
            .options
            .get("tool_options")
            .and_then(|options| options.get("projection_annotation"))
            .and_then(|block| block.get("bars"))
            .and_then(|bars| serde_json::from_value::<Vec<[f64; 4]>>(bars.clone()).ok())
            .map(|bars| crate::drawing_contract::legacy_bars_pattern(&bars))
            .filter(|bars| !bars.is_empty());
    }
    let fork_info_line = item.kind == DrawingKind::InfoLine
        && item
            .options
            .get("labels")
            .and_then(|labels| {
                serde_json::from_value::<Vec<crate::DrawingLabelOptions>>(labels.clone()).ok()
            })
            .is_some_and(|labels| crate::drawings::kinds::lines::is_legacy_info_stats(&labels));
    let Some(options) = item.options.as_object_mut() else {
        return;
    };
    if item.kind == DrawingKind::AnchoredText
        && !options.contains_key("screen_x")
        && let Some(anchor) = item.points.first_mut()
    {
        anchor.time = None;
        options.insert(
            "screen_x".to_string(),
            serde_json::json!(anchor.logical.unwrap_or(0.5).clamp(0.0, 1.0)),
        );
        options.insert(
            "screen_y".to_string(),
            serde_json::json!(anchor.price.clamp(0.0, 1.0)),
        );
    }
    if converted || fork_info_line {
        let tool_options = options
            .entry("tool_options")
            .or_insert_with(|| serde_json::json!({}));
        drop_consumed_gann_reverse(item.kind, Some(&mut *tool_options));
        crate::drawings::kinds::merge_legacy_fork_tool_options(item.kind, tool_options);
    }
    if converted && item.kind == DrawingKind::PatternTriangle {
        options.insert("extend_left".to_string(), serde_json::json!(true));
        options.insert("extend_right".to_string(), serde_json::json!(true));
    }
}

impl ChartEngine {
    /// Deterministic JSON of stable chart state. Financial-only charts retain the V1 wire format.
    pub fn export_state_json(&self) -> Result<String, ChartError> {
        if !self.indicators.is_empty()
            && self
                .panes
                .iter()
                .all(|pane| pane.general_horizontal_domain.is_none())
        {
            return self.export_state_v3_json();
        }
        if self
            .panes
            .iter()
            .any(|pane| pane.general_horizontal_domain.is_some())
            || self.general_dataset_count() != 0
            || self.general_series_count() != 0
            || !self.general_axes(None).is_empty()
        {
            return self.export_state_v2_json();
        }
        self.export_state_v1_json()
    }

    fn export_state_v3_json(&self) -> Result<String, ChartError> {
        if self.indicators.len() > PERSISTENCE_MAX_INDICATORS {
            return Err(resource("indicator count exceeds the persistence limit"));
        }
        let base = self.export_state_v1()?;
        let indicators = self
            .indicators
            .iter()
            .enumerate()
            .map(|(study, binding)| {
                let source = self.indicator_source_ref(binding.source, study)?;
                let volume_source = binding
                    .volume_source
                    .map(|id| self.indicator_source_ref(id, study))
                    .transpose()?;
                let amount_source = binding
                    .amount_source
                    .map(|id| self.indicator_source_ref(id, study))
                    .transpose()?;
                let styles = binding
                    .outputs
                    .iter()
                    .map(|&id| {
                        self.series_entry(id)
                            .map(crate::indicators::indicator_output_style)
                            .ok_or_else(|| {
                                ChartError::new(
                                    ErrorCode::SerializationError,
                                    format!("indicator output {id} is not live"),
                                )
                            })
                    })
                    .collect::<Result<Vec<_>, ChartError>>()?;
                Ok(IndicatorV3 {
                    kind: binding.kind.clone(),
                    source,
                    source_input: binding.source_input,
                    volume_source,
                    amount_source,
                    styles,
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let document = serde_json::to_string(&StateV3 {
            schema: base.schema,
            schema_version: PERSISTENCE_SCHEMA_VERSION_STUDIES,
            panes: base.panes,
            drawings: base.drawings,
            indicators,
            drawing_price_basis: base.drawing_price_basis,
            drawing_catalog: base.drawing_catalog,
            hidden_mark_groups: base.hidden_mark_groups,
        })
        .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))?;
        if document.len() > PERSISTENCE_MAX_DOCUMENT_BYTES {
            return Err(resource(
                "study persistence document exceeds the size limit",
            ));
        }
        Ok(document)
    }

    fn indicator_source_ref(
        &self,
        source: SeriesId,
        study: usize,
    ) -> Result<IndicatorSourceV3, ChartError> {
        for (source_study, binding) in self.indicators.iter().enumerate().take(study) {
            if let Some(output) = binding.outputs.iter().position(|&id| id == source) {
                return Ok(IndicatorSourceV3::Output {
                    study: source_study,
                    output,
                });
            }
        }
        if self.series_entry(source).is_none() {
            return Err(ChartError::new(
                ErrorCode::SerializationError,
                format!("indicator source {source} is not live"),
            ));
        }
        Ok(IndicatorSourceV3::Series { id: source })
    }

    fn export_state_v1_json(&self) -> Result<String, ChartError> {
        serde_json::to_string(&self.export_state_v1()?)
            .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))
    }

    /// The V1 state, which the V2 and V3 documents embed as a value. Reparsing the V1 JSON instead
    /// would deserialize export's own output and compile a second (string-reader) deserializer of
    /// the whole drawing tree into the module.
    fn export_state_v1(&self) -> Result<StateV1, ChartError> {
        let panes = self
            .panes
            .iter()
            .map(|pane| {
                let id = pane.persistent_id().ok_or_else(|| {
                    ChartError::new(
                        ErrorCode::SerializationError,
                        "chart pane has no persistence id",
                    )
                })?;
                Ok(PaneV1 {
                    id: pane_wire_id(id),
                    stretch_factor: pane.stretch_factor,
                    preserve_empty: pane.preserve_empty,
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let drawings = self
            .drawings
            .iter()
            .map(|drawing| {
                let pane = self.panes.get(drawing.pane_index).ok_or_else(|| {
                    ChartError::new(
                        ErrorCode::SerializationError,
                        format!("drawing {} is not attached to a live pane", drawing.id),
                    )
                })?;
                let persistent_id = pane.persistent_id().ok_or_else(|| {
                    ChartError::new(
                        ErrorCode::SerializationError,
                        "drawing pane has no persistence id",
                    )
                })?;
                // Omitted style fields restore the kind's own defaults (a ray's `extend_right`,
                // an arrow line's end cap), so each optional field is written when it differs
                // from them.
                let defaults = Drawing::new(0, drawing.kind, 0, Vec::new());
                Ok(DrawingV1 {
                    id: drawing.id,
                    kind: drawing.kind.name().to_string(),
                    pane_id: pane_wire_id(persistent_id),
                    anchors: self.drawing_anchors_of(drawing),
                    anchor_times_micros: self.drawing_anchor_times_for(drawing),
                    style: DrawingStyleV1 {
                        profile: drawing.profile.clone(),
                        position_account_size: matches!(
                            drawing.kind,
                            DrawingKind::LongPosition | DrawingKind::ShortPosition
                        )
                        .then_some(drawing.position_account_size),
                        position_risk_percent: matches!(
                            drawing.kind,
                            DrawingKind::LongPosition | DrawingKind::ShortPosition
                        )
                        .then_some(drawing.position_risk_percent),
                        regression_source_id: (drawing.kind == DrawingKind::RegressionTrend)
                            .then_some(drawing.regression_source_id)
                            .flatten(),
                        regression_deviations: (drawing.kind == DrawingKind::RegressionTrend)
                            .then_some(drawing.regression_deviations),
                        wave_degree: drawing
                            .kind
                            .is_elliott()
                            .then(|| drawing.wave_degree.clone()),
                        screen_x: (drawing.kind == DrawingKind::AnchoredText)
                            .then_some(drawing.screen_x),
                        screen_y: (drawing.kind == DrawingKind::AnchoredText)
                            .then_some(drawing.screen_y),
                        icon_name: (drawing.kind == DrawingKind::IconStamp)
                            .then(|| drawing.icon_name.clone())
                            .flatten(),
                        icon_size: (drawing.kind == DrawingKind::IconStamp)
                            .then_some(drawing.icon_size),
                        bars_pattern: (drawing.kind == DrawingKind::BarsPattern)
                            .then(|| drawing.bars_pattern.clone()),
                        bars_pattern_mirror_x: (drawing.kind == DrawingKind::BarsPattern
                            && drawing.bars_pattern_mirror_x)
                            .then_some(true),
                        bars_pattern_mirror_y: (drawing.kind == DrawingKind::BarsPattern
                            && drawing.bars_pattern_mirror_y)
                            .then_some(true),
                        bars_pattern_mode: (drawing.kind == DrawingKind::BarsPattern
                            && drawing.bars_pattern_mode != "bars")
                            .then(|| drawing.bars_pattern_mode.clone()),
                        name: (!drawing.name.is_empty()).then(|| drawing.name.clone()),
                        group_id: drawing.group_id.clone(),
                        revision: (drawing.revision != 1).then_some(drawing.revision),
                        visible: (!drawing.visible).then_some(false),
                        locked: drawing.locked.then_some(true),
                        z_order: (drawing.z_order != drawing.id as i32).then_some(drawing.z_order),
                        interval_visibility: drawing
                            .interval_visibility
                            .enabled
                            .then_some(drawing.interval_visibility.clone()),
                        stroke_start: (drawing.stroke_start != defaults.stroke_start)
                            .then_some(drawing.stroke_start),
                        stroke_end: (drawing.stroke_end != defaults.stroke_end)
                            .then_some(drawing.stroke_end),
                        extend_left: (drawing.extend_left != defaults.extend_left)
                            .then_some(drawing.extend_left),
                        extend_right: (drawing.extend_right != defaults.extend_right)
                            .then_some(drawing.extend_right),
                        fill_enabled: (drawing.fill_enabled != defaults.fill_enabled)
                            .then_some(drawing.fill_enabled),
                        magnet: (drawing.magnet != crate::DrawingMagnetMode::Off)
                            .then_some(drawing.magnet),
                        labels: (drawing.labels != defaults.labels).then(|| drawing.labels.clone()),
                        levels: (drawing.levels != defaults.levels).then(|| drawing.levels.clone()),
                        gann_fans: matches!(
                            drawing.kind,
                            DrawingKind::GannSquare | DrawingKind::GannSquareFixed
                        )
                        .then(|| drawing.gann_fans.clone()),
                        gann_arcs: matches!(
                            drawing.kind,
                            DrawingKind::GannSquare | DrawingKind::GannSquareFixed
                        )
                        .then(|| drawing.gann_arcs.clone()),
                        level_reverse: drawing.kind.has_levels().then_some(drawing.level_reverse),
                        level_log_scale: drawing
                            .kind
                            .has_levels()
                            .then_some(drawing.level_log_scale),
                        level_show_prices: drawing
                            .kind
                            .has_levels()
                            .then_some(drawing.level_show_prices),
                        level_show_values: drawing
                            .kind
                            .has_levels()
                            .then_some(drawing.level_show_values),
                        level_show_percents: drawing
                            .kind
                            .has_levels()
                            .then_some(drawing.level_show_percents),
                        level_label_align: drawing
                            .kind
                            .has_levels()
                            .then(|| drawing.level_label_align.clone()),
                        price_scale_id: (drawing.price_scale != DrawingPriceScale::Right)
                            .then(|| drawing.price_scale.name().to_string()),
                        color: Some(drawing.color.clone()),
                        width: Some(drawing.width),
                        line_style: Some(line_style_name(drawing.style).to_string()),
                        fill_color: drawing.fill_color.clone(),
                        preview_fill_color: drawing.preview_fill_color.clone(),
                        border_visible: (drawing.kind == DrawingKind::Rectangle
                            || !drawing.border_visible)
                            .then_some(drawing.border_visible),
                        show_labels: drawing.show_labels.then_some(true),
                        axis_bands_visible: drawing.axis_bands_visible.then_some(true),
                        label_color: drawing.label_color.clone(),
                        label_text_color: drawing.label_text_color.clone(),
                        snap_time_to_data: drawing.snap_time_to_data.then_some(true),
                        text: (drawing.text != defaults.text).then(|| drawing.text.clone()),
                        text_color: drawing.text_color.clone(),
                        text_size: drawing.text_size,
                        text_weight: drawing.text_weight,
                        text_italic: drawing.text_italic.then_some(true),
                        text_h_align: Some(drawing.text_h_align.name().to_string()),
                        text_v_align: Some(drawing.text_v_align.name().to_string()),
                        box_color: drawing.box_color.clone(),
                        box_border_color: drawing.box_border_color.clone(),
                        box_border_width: Some(drawing.box_border_width),
                        tool_options: exported_tool_options(drawing, &defaults)?,
                    },
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        Ok(StateV1 {
            schema: "aeris_charts-state".to_string(),
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            panes,
            drawings,
            drawing_price_basis: self.drawing_price_basis().map(str::to_string),
            drawing_catalog: Some(DRAWING_CATALOG_REVISION),
            hidden_mark_groups: self.hidden_timeline_groups(),
        })
    }

    fn export_state_v2_json(&self) -> Result<String, ChartError> {
        let base = self.export_state_v1()?;
        let panes = base
            .panes
            .into_iter()
            .enumerate()
            .map(|(index, pane)| {
                Ok(PaneV2 {
                    pane,
                    horizontal_domain: self
                        .pane_horizontal_domain(index)
                        .ok_or_else(|| invalid(format!("pane {index} has no horizontal domain")))?,
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let axes = self
            .general_axes
            .iter()
            .map(|axis| {
                let pane = self
                    .pane_index_for_id(axis.pane_id())
                    .ok_or_else(|| invalid(format!("axis {:?} has no live pane", axis.id())))?;
                Ok(crate::GeneralAxisOptions {
                    id: axis.id().to_string(),
                    pane,
                    dimension: axis.dimension(),
                    position: axis.position(),
                    scale: axis.scale(),
                    domain: axis.domain().clone(),
                    reverse: axis.reverse(),
                    visible: axis.visible(),
                    title: axis.title().map(str::to_string),
                    tick_count: axis.tick_count(),
                    ticks: axis.ticks().map(<[_]>::to_vec),
                    min_tick_gap: axis.min_tick_gap(),
                    band_padding_inner: axis.band_padding_inner(),
                    band_padding_outer: axis.band_padding_outer(),
                    zero_line: axis.zero_line(),
                    grid_visible: axis.grid_visible(),
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let mut dataset_ids = HashMap::new();
        let datasets = self
            .general_data
            .iter()
            .flat_map(|store| store.iter())
            .enumerate()
            .map(|(index, dataset)| {
                let id = format!("dataset-{}", index + 1);
                dataset_ids.insert(dataset.id(), id.clone());
                let ids = (0..dataset.len())
                    .map(|row| match dataset.row_identity(row) {
                        Some(crate::GeneralRowIdentity::Explicit(id)) => id.clone(),
                        _ => crate::GeneralRowId::Generated,
                    })
                    .collect::<Vec<_>>();
                let ids = ids
                    .iter()
                    .any(|id| !matches!(id, crate::GeneralRowId::Generated))
                    .then_some(ids);
                let y = dataset.y().to_vec();
                let y_valid = (0..dataset.len())
                    .any(|row| !dataset.y_is_valid(row))
                    .then(|| {
                        (0..dataset.len())
                            .map(|row| u8::from(dataset.y_is_valid(row)))
                            .collect()
                    });
                let size = dataset.size().map(ToOwned::to_owned);
                let size_valid = size.as_ref().and_then(|_| {
                    (0..dataset.len())
                        .any(|row| !dataset.size_is_valid(row))
                        .then(|| {
                            (0..dataset.len())
                                .map(|row| u8::from(dataset.size_is_valid(row)))
                                .collect()
                        })
                });
                let low = dataset.low().map(ToOwned::to_owned);
                let low_valid = low.as_ref().and_then(|_| {
                    (0..dataset.len())
                        .any(|row| !dataset.low_is_valid(row))
                        .then(|| {
                            (0..dataset.len())
                                .map(|row| u8::from(dataset.low_is_valid(row)))
                                .collect()
                        })
                });
                let high = dataset.high().map(ToOwned::to_owned);
                let high_valid = high.as_ref().and_then(|_| {
                    (0..dataset.len())
                        .any(|row| !dataset.high_is_valid(row))
                        .then(|| {
                            (0..dataset.len())
                                .map(|row| u8::from(dataset.high_is_valid(row)))
                                .collect()
                        })
                });
                let x_low = dataset.x_low().map(ToOwned::to_owned);
                let x_low_valid = x_low.as_ref().and_then(|_| {
                    (0..dataset.len())
                        .any(|row| !dataset.x_low_is_valid(row))
                        .then(|| {
                            (0..dataset.len())
                                .map(|row| u8::from(dataset.x_low_is_valid(row)))
                                .collect()
                        })
                });
                let x_high = dataset.x_high().map(ToOwned::to_owned);
                let x_high_valid = x_high.as_ref().and_then(|_| {
                    (0..dataset.len())
                        .any(|row| !dataset.x_high_is_valid(row))
                        .then(|| {
                            (0..dataset.len())
                                .map(|row| u8::from(dataset.x_high_is_valid(row)))
                                .collect()
                        })
                });
                let heatmap_y_numeric = dataset.heatmap_y_numeric().map(ToOwned::to_owned);
                debug_assert!(
                    low.is_none() || size.is_none(),
                    "range and bubble channels are mutually exclusive"
                );
                let input = match dataset.x_kind() {
                    crate::GeneralXKind::Numeric if heatmap_y_numeric.is_some() => {
                        crate::GeneralXyInput::HeatmapNumericNumeric {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            y_coordinate: heatmap_y_numeric
                                .expect("numeric heatmap Y-coordinate channel"),
                            value: y,
                            value_valid: y_valid,
                        }
                    }
                    crate::GeneralXKind::Numeric if high.is_some() => {
                        crate::GeneralXyInput::ErrorNumeric {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            y,
                            y_valid,
                            x_low: x_low.expect("error-bar X-low channel"),
                            x_low_valid,
                            x_high: x_high.expect("error-bar X-high channel"),
                            x_high_valid,
                            y_low: low.expect("error-bar Y-low channel"),
                            y_low_valid: low_valid,
                            y_high: high.expect("error-bar Y-high channel"),
                            y_high_valid: high_valid,
                        }
                    }
                    crate::GeneralXKind::Numeric => match (low, size) {
                        (Some(low), None) => crate::GeneralXyInput::RangeNumeric {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            low,
                            low_valid,
                            high: y,
                            high_valid: y_valid,
                        },
                        (None, Some(size)) => crate::GeneralXyInput::Bubble {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            y,
                            y_valid,
                            size,
                            size_valid,
                        },
                        (None, None) => crate::GeneralXyInput::Numeric {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            y,
                            y_valid,
                        },
                        (Some(low), Some(_)) => crate::GeneralXyInput::RangeNumeric {
                            ids,
                            x: dataset.numeric_x().unwrap_or_default().to_vec(),
                            low,
                            low_valid,
                            high: y,
                            high_valid: y_valid,
                        },
                    },
                    crate::GeneralXKind::Temporal if heatmap_y_numeric.is_some() => {
                        crate::GeneralXyInput::HeatmapTemporalNumeric {
                            ids,
                            x_epoch_ms: dataset.temporal_x_epoch_ms().unwrap_or_default().to_vec(),
                            y_coordinate: heatmap_y_numeric
                                .expect("temporal heatmap Y-coordinate channel"),
                            value: y,
                            value_valid: y_valid,
                        }
                    }
                    crate::GeneralXKind::Temporal if high.is_some() => {
                        crate::GeneralXyInput::ErrorTemporal {
                            ids,
                            x_epoch_ms: dataset.temporal_x_epoch_ms().unwrap_or_default().to_vec(),
                            y,
                            y_valid,
                            x_low_epoch_ms: x_low.expect("temporal error-bar X-low channel"),
                            x_low_valid,
                            x_high_epoch_ms: x_high.expect("temporal error-bar X-high channel"),
                            x_high_valid,
                            y_low: low.expect("temporal error-bar Y-low channel"),
                            y_low_valid: low_valid,
                            y_high: high.expect("temporal error-bar Y-high channel"),
                            y_high_valid: high_valid,
                        }
                    }
                    crate::GeneralXKind::Temporal => match low {
                        Some(low) => crate::GeneralXyInput::RangeTemporal {
                            ids,
                            x_epoch_ms: dataset.temporal_x_epoch_ms().unwrap_or_default().to_vec(),
                            low,
                            low_valid,
                            high: y,
                            high_valid: y_valid,
                        },
                        None => crate::GeneralXyInput::Temporal {
                            ids,
                            x_epoch_ms: dataset.temporal_x_epoch_ms().unwrap_or_default().to_vec(),
                            y,
                            y_valid,
                        },
                    },
                    crate::GeneralXKind::Category
                        if dataset.heatmap_y_categories().is_some()
                            && dataset.heatmap_y_category_indices().is_some() =>
                    {
                        crate::GeneralXyInput::HeatmapCategoryCategory {
                            ids,
                            x_categories: dataset.categories().unwrap_or_default().to_vec(),
                            x_category_indices: dataset
                                .category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            y_categories: dataset
                                .heatmap_y_categories()
                                .unwrap_or_default()
                                .to_vec(),
                            y_category_indices: dataset
                                .heatmap_y_category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            value: y,
                            value_valid: y_valid,
                        }
                    }
                    crate::GeneralXKind::Category
                        if high.is_some() && x_low.is_some() && x_high.is_some() =>
                    {
                        crate::GeneralXyInput::BoxCategory {
                            ids,
                            categories: dataset.categories().unwrap_or_default().to_vec(),
                            category_indices: dataset
                                .category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            min: low.expect("box-plot min channel"),
                            min_valid: low_valid,
                            q1: x_low.expect("box-plot q1 channel"),
                            q1_valid: x_low_valid,
                            median: y,
                            median_valid: y_valid,
                            q3: x_high.expect("box-plot q3 channel"),
                            q3_valid: x_high_valid,
                            max: high.expect("box-plot max channel"),
                            max_valid: high_valid,
                        }
                    }
                    crate::GeneralXKind::Category if high.is_some() => {
                        crate::GeneralXyInput::ErrorCategory {
                            ids,
                            categories: dataset.categories().unwrap_or_default().to_vec(),
                            category_indices: dataset
                                .category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            y,
                            y_valid,
                            y_low: low.expect("error-bar Y-low channel"),
                            y_low_valid: low_valid,
                            y_high: high.expect("error-bar Y-high channel"),
                            y_high_valid: high_valid,
                        }
                    }
                    crate::GeneralXKind::Category => match low {
                        Some(low) => crate::GeneralXyInput::RangeCategory {
                            ids,
                            categories: dataset.categories().unwrap_or_default().to_vec(),
                            category_indices: dataset
                                .category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            low,
                            low_valid,
                            high: y,
                            high_valid: y_valid,
                        },
                        None => crate::GeneralXyInput::Category {
                            ids,
                            categories: dataset.categories().unwrap_or_default().to_vec(),
                            category_indices: dataset
                                .category_indices()
                                .unwrap_or_default()
                                .to_vec(),
                            y,
                            y_valid,
                        },
                    },
                };
                let labels = (0..dataset.len())
                    .map(|row| dataset.row_label(row).map(str::to_string))
                    .collect::<Vec<_>>();
                let labels = labels.iter().any(Option::is_some).then_some(labels);
                DatasetV2 { id, input, labels }
            })
            .collect::<Vec<_>>();
        let series = self
            .general_series_iter()
            .map(|series| {
                Ok(SeriesV2 {
                    kind: series.kind(),
                    pane: self.pane_index_for_id(series.pane_id()).ok_or_else(|| {
                        invalid(format!("series {} has no live pane", series.id().get()))
                    })?,
                    dataset: dataset_ids.get(&series.dataset()).cloned().ok_or_else(|| {
                        invalid(format!("series {} has no live dataset", series.id().get()))
                    })?,
                    x_axis_id: series.x_axis_id().to_string(),
                    y_axis_id: series.y_axis_id().to_string(),
                    visible: series.visible(),
                    title: series.title().to_string(),
                    color: series.color().map(str::to_string),
                    point_radius: series.point_radius(),
                    point_markers: series.point_markers(),
                    point_symbol: series.point_symbol(),
                    line_width: series.line_width(),
                    line_style: series.line_style(),
                    interpolation: series.interpolation(),
                    connect_missing: series.connect_missing(),
                    fill_opacity: series.fill_opacity(),
                    baseline_value: series.baseline_value(),
                    data_labels: series.data_labels(),
                    group_id: series.group_id().map(str::to_string),
                    stack_id: series.stack_id().map(str::to_string),
                    stack_mode: series.stack_mode(),
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let references = self
            .general_reference_iter()
            .map(|reference| {
                self.general_reference_options(reference.id())
                    .ok_or_else(|| {
                        invalid(format!(
                            "general reference {} has no live pane",
                            reference.id().get()
                        ))
                    })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        let document = serde_json::to_string(&StateV2 {
            schema: base.schema,
            schema_version: PERSISTENCE_SCHEMA_VERSION_GENERAL,
            panes,
            drawings: base.drawings,
            axes,
            datasets,
            series,
            references,
            chart_options: self.options.value().clone(),
            drawing_price_basis: base.drawing_price_basis,
            drawing_catalog: base.drawing_catalog,
            hidden_mark_groups: base.hidden_mark_groups,
        })
        .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))?;
        if document.len() > PERSISTENCE_MAX_GENERAL_DOCUMENT_BYTES {
            return Err(resource("V2 persistence document exceeds the size limit"));
        }
        Ok(document)
    }

    /// Parse and validate untrusted state without mutating the chart.
    #[doc(hidden)]
    pub fn validate_state_json(json: &str) -> Result<ValidatedStateV1, ChartError> {
        Self::validate_state_v1(Self::parse_state_json(json)?)
    }

    fn parse_state_json(json: &str) -> Result<StateV1, ChartError> {
        if json.len() > PERSISTENCE_MAX_DOCUMENT_BYTES {
            return Err(resource(format!(
                "persistence document exceeds {PERSISTENCE_MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        let envelope: serde_json::Value = serde_json::from_str(json).map_err(|error| {
            ChartError::new(
                ErrorCode::SerializationError,
                format!("malformed JSON: {error}"),
            )
        })?;
        let object = envelope.as_object().ok_or_else(|| {
            ChartError::new(
                ErrorCode::SerializationError,
                "persistence document must be an object",
            )
        })?;
        let schema = object.get("schema").and_then(serde_json::Value::as_str);
        if schema != Some("aeris_charts-state") {
            return Err(ChartError::new(
                ErrorCode::SerializationError,
                "unsupported or missing persistence schema",
            ));
        }
        let version = object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ChartError::new(
                    ErrorCode::SerializationError,
                    "schema_version must be an integer",
                )
            })?;
        if version != u64::from(PERSISTENCE_SCHEMA_VERSION) {
            return Err(ChartError::new(
                ErrorCode::PersistenceVersionError,
                format!("unsupported persistence schema version {version}"),
            ));
        }
        serde_json::from_value(envelope).map_err(|error| {
            ChartError::new(
                ErrorCode::SerializationError,
                format!("invalid V1 document: {error}"),
            )
        })
    }

    fn validate_state_v1(state: StateV1) -> Result<ValidatedStateV1, ChartError> {
        if state.panes.is_empty() {
            return Err(invalid("V1 requires at least one pane"));
        }
        if state.panes.len() > PERSISTENCE_MAX_PANES {
            return Err(resource(format!(
                "V1 supports at most {PERSISTENCE_MAX_PANES} panes"
            )));
        }
        if state.drawings.len() > PERSISTENCE_MAX_DRAWINGS {
            return Err(resource(format!(
                "V1 supports at most {PERSISTENCE_MAX_DRAWINGS} drawings"
            )));
        }

        // Read before the panes move out of the document.
        let legacy_fork = written_by_legacy_fork(&state);
        let mut pane_ids = HashSet::with_capacity(state.panes.len());
        let mut panes = Vec::with_capacity(state.panes.len());
        let mut max_persistent_pane_id = 0;
        for pane in state.panes {
            let id = parse_pane_wire_id(&pane.id)
                .ok_or_else(|| invalid(format!("invalid pane id {:?}", pane.id)))?;
            if !pane_ids.insert(id) {
                return Err(invalid(format!("duplicate pane id {:?}", pane.id)));
            }
            if !pane.stretch_factor.is_finite()
                || pane.stretch_factor <= 0.0
                || pane.stretch_factor > 1_000_000.0
            {
                return Err(invalid(format!(
                    "pane {:?} has invalid stretch_factor",
                    pane.id
                )));
            }
            max_persistent_pane_id = max_persistent_pane_id.max(id);
            panes.push(ValidatedPane {
                persistent_id: id,
                stretch_factor: pane.stretch_factor,
                preserve_empty: pane.preserve_empty,
            });
        }
        if max_persistent_pane_id == u32::MAX {
            return Err(resource("pane persistence identity space is exhausted"));
        }

        let pane_positions = panes
            .iter()
            .enumerate()
            .map(|(index, pane)| (pane.persistent_id, index))
            .collect::<std::collections::HashMap<_, _>>();
        let mut drawing_ids = HashSet::with_capacity(state.drawings.len());
        let mut drawings = Vec::with_capacity(state.drawings.len());
        let mut drawing_anchor_times = HashMap::new();
        let mut max_drawing_id = 0;
        let mut total_points = 0usize;
        let mut total_text = 0usize;
        for item in state.drawings {
            if item.id == 0 || item.id == u32::MAX || !drawing_ids.insert(item.id) {
                return Err(invalid(format!(
                    "drawing id {} is invalid or duplicated",
                    item.id
                )));
            }
            let kind = stored_drawing_kind(&item.kind, &item.anchors)
                .ok_or_else(|| invalid(format!("unknown drawing kind {:?}", item.kind)))?;
            if item.anchors.len() > PERSISTENCE_MAX_POINTS_PER_DRAWING {
                return Err(resource(format!(
                    "drawing {} exceeds {PERSISTENCE_MAX_POINTS_PER_DRAWING} anchors",
                    item.id
                )));
            }
            if !item.anchor_times_micros.is_empty()
                && item.anchor_times_micros.len() != item.anchors.len()
            {
                return Err(invalid(format!(
                    "drawing {} has an anchor-time count different from its anchors",
                    item.id
                )));
            }
            let mut anchors = item.anchors;
            let mut anchor_times_micros = item.anchor_times_micros;
            let mut style = item.style;
            let mut tool_options = style.tool_options.take();
            // A drawing the fork stored under its own anchor contract converts to upstream's
            // before the anchor count is checked: restore is atomic, so one such drawing would
            // otherwise reject the whole layout.
            let migrated = if item.kind == "flat_top_bottom" && kind == DrawingKind::DisjointChannel
            {
                Some(fork_flat_crossing_anchors(&anchors))
            } else {
                migrate_fork_anchors(kind, &anchors, tool_options.as_ref(), legacy_fork)
            };
            let converted_bars_pattern = migrated.is_some() && kind == DrawingKind::BarsPattern;
            if migrated.is_some() {
                drop_consumed_gann_reverse(kind, tool_options.as_mut());
            }
            if let Some(migrated) = migrated {
                if !anchor_times_micros.is_empty() {
                    anchor_times_micros = migrated
                        .iter()
                        .map(|anchor| anchor.identity.and_then(|index| anchor_times_micros[index]))
                        .collect();
                }
                anchors = migrated.into_iter().map(|anchor| anchor.anchor).collect();
            }
            // The fork placed an anchored text by its anchor alone, a pane fraction. Upstream
            // places it by `screen_x`/`screen_y`; the anchor stays, inert, without time identity.
            let pane_position = if kind == DrawingKind::AnchoredText && style.screen_x.is_none() {
                anchors.first_mut().map(|anchor| {
                    anchor.time = None;
                    (
                        anchor.logical.unwrap_or(0.5).clamp(0.0, 1.0),
                        anchor.price.clamp(0.0, 1.0),
                    )
                })
            } else {
                None
            };
            if pane_position.is_some()
                && let Some(time) = anchor_times_micros.first_mut()
            {
                *time = None;
            }
            total_points = total_points
                .checked_add(anchors.len())
                .ok_or_else(|| resource("drawing anchor count overflow"))?;
            if total_points > PERSISTENCE_MAX_TOTAL_POINTS {
                return Err(resource(format!(
                    "V1 supports at most {PERSISTENCE_MAX_TOTAL_POINTS} total anchors"
                )));
            }
            if !kind.valid_point_count(anchors.len()) {
                return Err(invalid(format!(
                    "drawing {} has an invalid anchor count",
                    item.id
                )));
            }
            if !anchor_times_micros.is_empty() {
                drawing_anchor_times.insert(item.id, anchor_times_micros);
            }
            let bounded = |value: f64| value.is_finite() && value.abs() <= MAX_SAFE_VALUE;
            if anchors.iter().any(|anchor| {
                !bounded(anchor.price)
                    || anchor.logical.is_some_and(|logical| !bounded(logical))
                    || anchor.time.is_some_and(|time| !bounded(time))
                    || (anchor.logical.is_none() && anchor.time.is_none())
            }) {
                return Err(invalid(format!(
                    "drawing {} has an invalid anchor",
                    item.id
                )));
            }
            // Anchor times are restored as pending identity and resolved against the host's
            // data on install (immediately when data is present, otherwise when it arrives).
            let pending_times = anchors.iter().map(|anchor| anchor.time).collect::<Vec<_>>();
            let anchors = anchors
                .iter()
                .map(|anchor| DrawingPoint {
                    logical: anchor.logical.unwrap_or(0.0),
                    price: anchor.price,
                })
                .collect::<Vec<_>>();
            let pane_id = parse_pane_wire_id(&item.pane_id).ok_or_else(|| {
                invalid(format!("drawing {} has an invalid pane reference", item.id))
            })?;
            let pane_index = pane_positions.get(&pane_id).copied().ok_or_else(|| {
                invalid(format!(
                    "drawing {} references unknown pane {:?}",
                    item.id, item.pane_id
                ))
            })?;
            let mut drawing = Drawing::new(item.id, kind, pane_index, anchors);
            drawing.set_pending_times(pending_times);
            if legacy_fork {
                // The fork omitted every value equal to its own defaults.
                crate::drawings::kinds::apply_legacy_fork_defaults(&mut drawing);
            }
            if let Some((screen_x, screen_y)) = pane_position {
                drawing.screen_x = screen_x;
                drawing.screen_y = screen_y;
            }
            if kind == DrawingKind::Rectangle && style.border_visible.is_none() {
                // Older documents omitted the then-visible default. Preserve their appearance.
                drawing.border_visible = true;
            }
            // Fork option keys that overlap an upstream flat field stand in for that field where
            // the document carries no flat key of its own, and pass the same checks below.
            let stored_tool_options = tool_options.is_some();
            if stored_tool_options || legacy_fork {
                let options = tool_options.get_or_insert_with(|| serde_json::json!({}));
                let legacy =
                    crate::drawing_contract::take_legacy_flat_options(kind, options, legacy_fork);
                fill_absent(&mut style.level_reverse, legacy.level_reverse);
                fill_absent(&mut style.level_log_scale, legacy.level_log_scale);
                fill_absent(&mut style.level_show_prices, legacy.level_show_prices);
                fill_absent(&mut style.level_show_values, legacy.level_show_values);
                fill_absent(&mut style.level_show_percents, legacy.level_show_percents);
                fill_absent(&mut style.level_label_align, legacy.level_label_align);
                fill_absent(&mut style.gann_fans, legacy.gann_fans);
                fill_absent(&mut style.gann_arcs, legacy.gann_arcs);
                fill_absent(&mut style.wave_degree, legacy.wave_degree);
                // A fork bars pattern without copied bars has no snapshot to carry.
                fill_absent(
                    &mut style.bars_pattern,
                    legacy.bars_pattern.filter(|bars| !bars.is_empty()),
                );
                fill_absent(
                    &mut style.bars_pattern_mirror_x,
                    legacy.bars_pattern_mirror_x,
                );
                fill_absent(
                    &mut style.bars_pattern_mirror_y,
                    legacy.bars_pattern_mirror_y,
                );
                fill_absent(&mut style.bars_pattern_mode, legacy.bars_pattern_mode);
                fill_absent(&mut style.icon_name, legacy.icon_name);
                fill_absent(&mut style.icon_size, legacy.icon_size);
                fill_absent(
                    &mut style.regression_deviations,
                    legacy.regression_deviations,
                );
            }
            if let Some(value) = style.position_account_size {
                if !value.is_finite() || value <= 0.0 || value > 1e15 {
                    return Err(invalid(format!(
                        "drawing {} has invalid position account size",
                        item.id
                    )));
                }
                drawing.position_account_size = value;
            }
            if let Some(value) = style.position_risk_percent {
                if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                    return Err(invalid(format!(
                        "drawing {} has invalid position risk percent",
                        item.id
                    )));
                }
                drawing.position_risk_percent = value;
            }
            if let Some(source_id) = style.regression_source_id {
                drawing.regression_source_id = Some(source_id);
            }
            if let Some(deviations) = style.regression_deviations {
                if !deviations.is_finite() || !(0.0..=10.0).contains(&deviations) {
                    return Err(invalid(format!(
                        "drawing {} has invalid regression deviations",
                        item.id
                    )));
                }
                drawing.regression_deviations = deviations;
            }
            if let Some(degree) = style.wave_degree {
                if !drawing.kind.is_elliott() || !DrawingKind::valid_wave_degree(&degree) {
                    return Err(invalid(format!(
                        "drawing {} has invalid wave degree",
                        item.id
                    )));
                }
                drawing.wave_degree = degree;
            }
            if let Some(value) = style.screen_x {
                if drawing.kind != DrawingKind::AnchoredText
                    || !value.is_finite()
                    || !(0.0..=1.0).contains(&value)
                {
                    return Err(invalid(format!("drawing {} has invalid screen x", item.id)));
                }
                drawing.screen_x = value;
            }
            if let Some(value) = style.screen_y {
                if drawing.kind != DrawingKind::AnchoredText
                    || !value.is_finite()
                    || !(0.0..=1.0).contains(&value)
                {
                    return Err(invalid(format!("drawing {} has invalid screen y", item.id)));
                }
                drawing.screen_y = value;
            }
            if let Some(name) = style.icon_name {
                if drawing.kind != DrawingKind::IconStamp
                    || name.is_empty()
                    || name.len() > crate::drawings::MAX_DRAWING_ICON_NAME_BYTES
                {
                    return Err(invalid(format!(
                        "drawing {} has invalid icon name",
                        item.id
                    )));
                }
                drawing.icon_name = Some(name);
            }
            if let Some(size) = style.icon_size {
                if drawing.kind != DrawingKind::IconStamp
                    || !size.is_finite()
                    || !(8.0..=96.0).contains(&size)
                {
                    return Err(invalid(format!(
                        "drawing {} has invalid icon size",
                        item.id
                    )));
                }
                drawing.icon_size = size;
            }
            if let Some(bars) = style.bars_pattern {
                if drawing.kind != DrawingKind::BarsPattern
                    || bars.is_empty()
                    || bars.len() > crate::drawings::MAX_BARS_PATTERN_BARS
                    || bars.iter().any(|bar| !bar.valid())
                    || bars.windows(2).any(|pair| pair[0].offset >= pair[1].offset)
                    || bars.last().is_some_and(|bar| {
                        usize::from(bar.offset) >= crate::drawings::MAX_BARS_PATTERN_BARS
                    })
                {
                    return Err(invalid(format!(
                        "drawing {} has invalid bars pattern",
                        item.id
                    )));
                }
                drawing.bars_pattern = bars;
            }
            if let Some(value) = style.bars_pattern_mirror_x {
                if drawing.kind != DrawingKind::BarsPattern {
                    return Err(invalid(format!(
                        "drawing {} has invalid bars pattern mirror",
                        item.id
                    )));
                }
                drawing.bars_pattern_mirror_x = value;
            }
            if let Some(value) = style.bars_pattern_mirror_y {
                if drawing.kind != DrawingKind::BarsPattern {
                    return Err(invalid(format!(
                        "drawing {} has invalid bars pattern mirror",
                        item.id
                    )));
                }
                drawing.bars_pattern_mirror_y = value;
            }
            if let Some(mode) = style.bars_pattern_mode {
                if drawing.kind != DrawingKind::BarsPattern
                    || !crate::drawings::valid_bars_pattern_mode(&mode)
                {
                    return Err(invalid(format!(
                        "drawing {} has invalid bars pattern mode",
                        item.id
                    )));
                }
                // `hl_bars` is the fork's spelling of the high-low bars mode.
                drawing.bars_pattern_mode = if mode == "hl_bars" {
                    "bars".to_string()
                } else {
                    mode
                };
            }
            if let Some(profile) = style.profile {
                if !profile.valid() {
                    return Err(invalid(format!(
                        "drawing {} has invalid profile options",
                        item.id
                    )));
                }
                drawing.profile = Some(profile);
            }
            if let Some(name) = style.name {
                if name.len() > crate::MAX_DRAWING_NAME_BYTES {
                    return Err(resource(format!("drawing {} name is too large", item.id)));
                }
                drawing.name = name;
            }
            if let Some(group_id) = style.group_id {
                if group_id.len() > crate::MAX_DRAWING_GROUP_BYTES {
                    return Err(resource(format!(
                        "drawing {} group id is too large",
                        item.id
                    )));
                }
                drawing.group_id = (!group_id.is_empty()).then_some(group_id);
            }
            if let Some(revision) = style.revision {
                drawing.revision = revision.max(1);
            }
            if let Some(visible) = style.visible {
                drawing.visible = visible;
            }
            if let Some(locked) = style.locked {
                drawing.locked = locked;
            }
            if let Some(z_order) = style.z_order {
                drawing.z_order = z_order;
            }
            if let Some(interval_visibility) = style.interval_visibility {
                if !interval_visibility.validate() {
                    return Err(invalid(format!(
                        "drawing {} has invalid interval visibility",
                        item.id
                    )));
                }
                drawing.interval_visibility = interval_visibility;
            }
            if let Some(stroke_start) = style.stroke_start {
                drawing.stroke_start = stroke_start;
            }
            if let Some(stroke_end) = style.stroke_end {
                drawing.stroke_end = stroke_end;
            }
            if let Some(extend_left) = style.extend_left {
                drawing.extend_left = extend_left;
            }
            if let Some(extend_right) = style.extend_right {
                drawing.extend_right = extend_right;
            }
            if let Some(fill_enabled) = style.fill_enabled {
                drawing.fill_enabled = fill_enabled;
            }
            if let Some(magnet) = style.magnet {
                drawing.magnet = magnet;
            }
            if let Some(labels) = style.labels {
                if labels.len() > crate::MAX_DRAWING_LABELS
                    || !labels.iter().all(|label| label.validate())
                {
                    return Err(invalid(format!("drawing {} has invalid labels", item.id)));
                }
                drawing.labels = labels;
            }
            if let Some(levels) = style.levels {
                if levels.len() > crate::MAX_DRAWING_LEVELS
                    || !levels.iter().all(|level| level.validate())
                {
                    return Err(invalid(format!("drawing {} has invalid levels", item.id)));
                }
                drawing.levels = levels;
            }
            for (family, destination, fan) in [
                (style.gann_fans, &mut drawing.gann_fans, true),
                (style.gann_arcs, &mut drawing.gann_arcs, false),
            ] {
                if let Some(levels) = family {
                    if !matches!(
                        drawing.kind,
                        DrawingKind::GannSquare | DrawingKind::GannSquareFixed
                    ) || !crate::drawings::valid_gann_family(&levels, fan)
                    {
                        return Err(invalid(format!(
                            "drawing {} has invalid Gann levels",
                            item.id
                        )));
                    }
                    *destination = levels;
                }
            }
            let has_level_options = style.level_reverse.is_some()
                || style.level_log_scale.is_some()
                || style.level_show_prices.is_some()
                || style.level_show_values.is_some()
                || style.level_show_percents.is_some()
                || style.level_label_align.is_some();
            if has_level_options
                && (!drawing.kind.has_levels()
                    || (style.level_log_scale == Some(true) && !drawing.kind.supports_log_levels())
                    || style
                        .level_label_align
                        .as_deref()
                        .is_some_and(|align| !matches!(align, "left" | "center" | "right")))
            {
                return Err(invalid(format!(
                    "drawing {} has invalid level options",
                    item.id
                )));
            }
            if let Some(value) = style.level_reverse {
                drawing.level_reverse = value;
            }
            if let Some(value) = style.level_log_scale {
                drawing.level_log_scale = value;
            }
            if let Some(value) = style.level_show_prices {
                drawing.level_show_prices = value;
            }
            if let Some(value) = style.level_show_values {
                drawing.level_show_values = value;
            }
            if let Some(value) = style.level_show_percents {
                drawing.level_show_percents = value;
            }
            if let Some(value) = style.level_label_align {
                drawing.level_label_align = value;
            }
            if let Some(scale) = style.price_scale_id {
                drawing.price_scale = DrawingPriceScale::from_name(&scale)
                    .ok_or_else(|| invalid(format!("unknown drawing price scale {scale:?}")))?;
            }
            if let Some(color) = style.color {
                validate_color(&color, "drawing color")?;
                if !color.is_empty() {
                    drawing.color = color;
                }
            }
            if let Some(width) = style.width {
                validate_positive_number(width, "drawing width")?;
                drawing.width = width;
            }
            if let Some(value) = style.line_style {
                drawing.style = parse_line_style(&value)
                    .ok_or_else(|| invalid(format!("unknown line style {value:?}")))?;
            }
            if let Some(color) = style.fill_color {
                validate_color(&color, "fill_color")?;
                drawing.fill_color = (!color.is_empty()).then_some(color);
            }
            if let Some(color) = style.preview_fill_color {
                validate_color(&color, "preview_fill_color")?;
                drawing.preview_fill_color = (!color.is_empty()).then_some(color);
            }
            if let Some(visible) = style.border_visible {
                drawing.border_visible = visible;
            }
            if let Some(visible) = style.show_labels {
                drawing.show_labels = visible;
            }
            if let Some(visible) = style.axis_bands_visible {
                drawing.axis_bands_visible = visible;
            }
            if let Some(color) = style.label_color {
                validate_color(&color, "label_color")?;
                drawing.label_color = (!color.is_empty()).then_some(color);
            }
            if let Some(color) = style.label_text_color {
                validate_color(&color, "label_text_color")?;
                drawing.label_text_color = (!color.is_empty()).then_some(color);
            }
            if let Some(snap) = style.snap_time_to_data {
                drawing.snap_time_to_data = snap;
            }
            if let Some(text) = style.text {
                if text.len() > MAX_TEXT_BYTES {
                    return Err(resource(format!("drawing {} text is too large", item.id)));
                }
                total_text = total_text
                    .checked_add(text.len())
                    .ok_or_else(|| resource("drawing text size overflow"))?;
                if total_text > MAX_TOTAL_TEXT_BYTES {
                    return Err(resource("drawing text exceeds the document limit"));
                }
                drawing.text = text;
            }
            if let Some(color) = style.text_color {
                validate_color(&color, "text_color")?;
                drawing.text_color = (!color.is_empty()).then_some(color);
            }
            if let Some(size) = style.text_size {
                validate_positive_number(size, "text_size")?;
                drawing.text_size = Some(size);
            }
            if let Some(weight) = style.text_weight {
                if !(100..=900).contains(&weight) {
                    return Err(invalid("text_weight must be in 100..=900"));
                }
                drawing.text_weight = Some(weight);
            }
            if let Some(italic) = style.text_italic {
                drawing.text_italic = italic;
            }
            if let Some(align) = style.text_h_align {
                drawing.text_h_align = DrawingTextHAlign::from_name(&align).ok_or_else(|| {
                    invalid(format!("unknown horizontal text alignment {align:?}"))
                })?;
            }
            if let Some(align) = style.text_v_align {
                drawing.text_v_align = DrawingTextVAlign::from_name(&align)
                    .ok_or_else(|| invalid(format!("unknown vertical text alignment {align:?}")))?;
            }
            // Every exporter writes the box colors whenever they are set, so an absent key is a
            // box the user (or the fork's defaults) cleared, not the kind's tinted default.
            if matches!(
                kind,
                DrawingKind::Note | DrawingKind::Comment | DrawingKind::Callout
            ) {
                if style.box_color.is_none() {
                    drawing.box_color = None;
                }
                if style.box_border_color.is_none() {
                    drawing.box_border_color = None;
                }
            }
            if let Some(color) = style.box_color {
                validate_color(&color, "box_color")?;
                drawing.box_color = (!color.is_empty()).then_some(color);
            }
            if let Some(color) = style.box_border_color {
                validate_color(&color, "box_border_color")?;
                drawing.box_border_color = (!color.is_empty()).then_some(color);
            }
            if let Some(width) = style.box_border_width {
                validate_positive_number(width, "box_border_width")?;
                drawing.box_border_width = width;
            }
            // What remains of the stored blocks after the flat-field keys moved out, over the
            // fork's option defaults its documents never stored.
            if let Some(mut tool_options) =
                tool_options.filter(|_| stored_tool_options || legacy_fork)
            {
                if legacy_fork {
                    crate::drawings::kinds::merge_legacy_fork_tool_options(kind, &mut tool_options);
                }
                let tool_options =
                    serde_json::from_value::<crate::DrawingToolOptions>(tool_options)
                        .ok()
                        .filter(crate::DrawingToolOptions::validate)
                        .ok_or_else(|| {
                            invalid(format!("drawing {} has invalid tool options", item.id))
                        })?;
                drawing.tool_options = tool_options;
            }
            if legacy_fork {
                // With its style in place: the median takes the drawing's line style.
                crate::drawings::kinds::pitchforks_gann::legacy_levels_to_upstream(&mut drawing);
            }
            max_drawing_id = max_drawing_id.max(item.id);
            // A bars pattern converted from a fork drawing that had copied no bars restores
            // without a snapshot: it paints nothing until an edit of its box recaptures one.
            if drawing.kind == DrawingKind::BarsPattern
                && drawing.bars_pattern.is_empty()
                && !converted_bars_pattern
            {
                return Err(invalid(format!(
                    "drawing {} has no bars pattern snapshot",
                    item.id
                )));
            }
            drawings.push(drawing);
        }
        let drawing_price_basis = state.drawing_price_basis.filter(|basis| !basis.is_empty());
        if drawing_price_basis
            .as_ref()
            .is_some_and(|basis| basis.len() > crate::MAX_DRAWING_GROUP_BYTES)
        {
            return Err(resource("drawing price basis is too large"));
        }
        crate::timeline_marks::validate_hidden_groups(&state.hidden_mark_groups)?;
        Ok(ValidatedStateV1 {
            panes,
            drawings,
            drawing_anchor_times,
            drawing_price_basis,
            hidden_mark_groups: state.hidden_mark_groups,
            max_drawing_id,
            max_persistent_pane_id,
            points: total_points,
        })
    }

    /// Install already-validated state as one logical transaction.
    #[doc(hidden)]
    pub fn install_validated_state(
        &mut self,
        state: ValidatedStateV1,
    ) -> Result<PersistenceRestoreResult, ChartError> {
        #[cfg(target_arch = "wasm32")]
        return self.install_validated_state_inner(state);
        #[cfg(not(target_arch = "wasm32"))]
        self.install_validated_state_inner(state, None)
    }

    fn install_validated_state_inner(
        &mut self,
        state: ValidatedStateV1,
        #[cfg(not(target_arch = "wasm32"))] mut profile: Option<&mut InstallProfile>,
    ) -> Result<PersistenceRestoreResult, ChartError> {
        if self.panes.len() != 1
            || !self.drawings.is_empty()
            || self.next_drawing_id != 1
            || self.general_dataset_count() != 0
            || self.general_series_count() != 0
        {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "state import requires a fresh chart before drawing handles have been issued",
            ));
        }
        #[cfg(not(target_arch = "wasm32"))]
        let semantic_started = profile.as_ref().map(|_| Instant::now());
        let pane_count = u32::try_from(state.panes.len())
            .map_err(|_| resource("pane count cannot be represented"))?;
        let next_runtime = self
            .next_pane_id
            .checked_add(pane_count)
            .ok_or_else(|| resource("pane handle identity space is exhausted"))?;
        let mut panes = Vec::with_capacity(state.panes.len());
        for (offset, persisted) in state.panes.iter().enumerate() {
            let offset = u32::try_from(offset).map_err(|_| resource("pane index overflow"))?;
            let runtime_id = PaneId::try_from(self.next_pane_id + offset)?;
            let mut pane = Pane::with_chart_ids(runtime_id, persisted.persistent_id);
            pane.stretch_factor = persisted.stretch_factor;
            pane.preserve_empty = persisted.preserve_empty;
            self.apply_chart_scale_options(&mut pane);
            panes.push(pane);
        }

        let drawing_count = state.drawings.len();
        let points = state.points;
        self.panes = panes;
        self.general_horizontal_domains = crate::domains::HorizontalDomainRegistry::new();
        self.general_axes = crate::general_axes::GeneralAxisRegistry::new();
        self.general_data = None;
        self.general_series = None;
        self.drawings = state.drawings;
        self.drawing_anchor_times = state.drawing_anchor_times;
        self.restore_drawing_time_identity(state.drawing_price_basis);
        self.timeline_marks
            .install_hidden_groups(state.hidden_mark_groups);
        self.next_pane_id = next_runtime;
        self.next_persistent_pane_id = state.max_persistent_pane_id + 1;
        self.next_drawing_id = state.max_drawing_id + 1;
        for series in &mut self.series {
            if !series.removed {
                series.pane_index = 0;
            }
        }
        self.selected_drawing = None;
        self.selected_drawings.clear();
        self.drawing_drag = None;
        self.drawing_history = crate::DrawingHistory::default();
        // Persistence replaces committed semantic state, but an armed host tool is transient UI
        // state and historically survived import. Abort only the in-flight placement/capture.
        self.drawing_controller.pending = None;
        self.drawing_controller.brush = None;
        self.drawing_controller.measure = None;
        self.drawing_text_edit = None;
        self.hovered_drawing = None;
        self.hovered_text = None;
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), semantic_started) {
            profile.semantic_install_ns =
                started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let index_started = profile.as_ref().map(|_| Instant::now());
        self.drawing_runtime
            .borrow_mut()
            .rebuild_all(&self.drawings, self.panes.len());
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(profile), Some(started)) = (profile, index_started) {
            profile.index_rebuild_ns =
                started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        }
        self.layout_panes(self.pane_h);
        self.invalidate_frame_all();
        Ok(PersistenceRestoreResult {
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            panes: self.panes.len(),
            drawings: drawing_count,
            points,
        })
    }

    /// Validate first, then install atomically. Any error leaves the chart unchanged.
    pub fn import_state_json(
        &mut self,
        json: &str,
    ) -> Result<PersistenceRestoreResult, ChartError> {
        if json.len() > PERSISTENCE_MAX_GENERAL_DOCUMENT_BYTES {
            return Err(resource("persistence document exceeds the size limit"));
        }
        let envelope: serde_json::Value = match serde_json::from_str(json) {
            Ok(envelope) => envelope,
            Err(_) if json.len() > PERSISTENCE_MAX_DOCUMENT_BYTES => {
                return Err(resource("persistence document exceeds the V1 size limit"));
            }
            Err(error) => {
                return Err(ChartError::new(
                    ErrorCode::SerializationError,
                    format!("malformed JSON: {error}"),
                ));
            }
        };
        let is_v2 = envelope
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(u64::from(PERSISTENCE_SCHEMA_VERSION_GENERAL));
        let is_v3 = envelope
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            == Some(u64::from(PERSISTENCE_SCHEMA_VERSION_STUDIES));
        if !is_v2 && json.len() > PERSISTENCE_MAX_DOCUMENT_BYTES {
            return Err(resource("persistence document exceeds the V1 size limit"));
        }
        if is_v3 {
            if envelope.get("schema").and_then(serde_json::Value::as_str)
                != Some("aeris_charts-state")
            {
                return Err(ChartError::new(
                    ErrorCode::SerializationError,
                    "unsupported or missing persistence schema",
                ));
            }
            let state: StateV3 = serde_json::from_value(envelope).map_err(|error| {
                ChartError::new(
                    ErrorCode::SerializationError,
                    format!("invalid study persistence document: {error}"),
                )
            })?;
            return self.import_state_v3(state);
        }
        if is_v2 {
            if envelope.get("schema").and_then(serde_json::Value::as_str)
                != Some("aeris_charts-state")
            {
                return Err(ChartError::new(
                    ErrorCode::SerializationError,
                    "unsupported or missing persistence schema",
                ));
            }
            let state: StateV2 = serde_json::from_value(envelope).map_err(|error| {
                ChartError::new(
                    ErrorCode::SerializationError,
                    format!("invalid V2 document: {error}"),
                )
            })?;
            return self.import_state_v2(state);
        }
        let state = Self::validate_state_json(json)?;
        self.install_validated_state(state)
    }

    fn import_state_v3(&mut self, state: StateV3) -> Result<PersistenceRestoreResult, ChartError> {
        if state.indicators.len() > PERSISTENCE_MAX_INDICATORS {
            return Err(resource("indicator count exceeds the persistence limit"));
        }
        if self.panes.len() != 1
            || !self.drawings.is_empty()
            || self.next_drawing_id != 1
            || !self.indicators.is_empty()
            || self.general_dataset_count() != 0
            || self.general_series_count() != 0
        {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "study state import requires a fresh financial chart",
            ));
        }
        let validated = Self::validate_state_v1(StateV1 {
            schema: state.schema.clone(),
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            panes: state.panes,
            drawings: state.drawings,
            drawing_price_basis: state.drawing_price_basis.clone(),
            drawing_catalog: state.drawing_catalog,
            hidden_mark_groups: state.hidden_mark_groups,
        })?;
        let mut resolved = Vec::with_capacity(state.indicators.len());
        let mut expected_outputs = Vec::with_capacity(state.indicators.len());
        for (study, indicator) in state.indicators.iter().enumerate() {
            if !indicator_kind_is_valid(&indicator.kind)
                || matches!(
                    indicator.kind,
                    IndicatorKind::SwingPoints { .. }
                        | IndicatorKind::MarketStructure { .. }
                        | IndicatorKind::FairValueGaps { .. }
                        | IndicatorKind::OrderBlocks { .. }
                ) && indicator.source_input != IndicatorInputSource::Close
            {
                return Err(invalid(format!("indicator {study} has invalid parameters")));
            }
            if indicator.styles.len() > aeris_charts_indicators::MAX_OUTPUTS {
                return Err(resource(format!(
                    "indicator {study} has too many output styles"
                )));
            }
            let source =
                resolve_indicator_source(&indicator.source, study, &expected_outputs, self)?;
            let volume_source = indicator
                .volume_source
                .as_ref()
                .map(|source| resolve_indicator_source(source, study, &expected_outputs, self))
                .transpose()?;
            if let Some(volume_source) = volume_source {
                if source_refs_equal(&indicator.source, indicator.volume_source.as_ref().unwrap()) {
                    return Err(invalid(format!(
                        "indicator {study} volume source duplicates its price source"
                    )));
                }
                if matches!(
                    indicator.volume_source.as_ref(),
                    Some(IndicatorSourceV3::Series { .. })
                ) && !source_is_scalar(self, volume_source)
                {
                    return Err(invalid(format!(
                        "indicator {study} volume source must be scalar"
                    )));
                }
            }
            let amount_source = indicator
                .amount_source
                .as_ref()
                .map(|source| resolve_indicator_source(source, study, &expected_outputs, self))
                .transpose()?;
            if let (Some(amount_source), Some(amount_ref)) =
                (amount_source, indicator.amount_source.as_ref())
                && (!matches!(indicator.kind, IndicatorKind::Vwap)
                    || source_refs_equal(&indicator.source, amount_ref)
                    || indicator
                        .volume_source
                        .as_ref()
                        .is_none_or(|volume| source_refs_equal(volume, amount_ref))
                    || (matches!(amount_ref, IndicatorSourceV3::Series { .. })
                        && !source_is_scalar(self, amount_source)))
            {
                return Err(invalid(format!(
                    "indicator {study} amount source must be a distinct scalar VWAP input"
                )));
            }
            let expected = incremental_output_count(&indicator.kind);
            if indicator.styles.len() != expected {
                return Err(invalid(format!(
                    "indicator {study} style count does not match its output count"
                )));
            }
            for (output, style) in indicator.styles.iter().enumerate() {
                validate_indicator_style(style).map_err(|message| {
                    invalid(format!("indicator {study} output {output}: {message}"))
                })?;
            }
            resolved.push((
                source,
                indicator.source_input,
                indicator.kind.clone(),
                volume_source,
                amount_source,
                indicator.styles.clone(),
            ));
            expected_outputs.push(vec![u32::MAX; expected]);
        }
        let mut result = self.install_validated_state(validated)?;
        let mut remapped_outputs = Vec::with_capacity(resolved.len());
        self.study_restore_pane_cursor = Some(1);
        for (study, (source, source_input, kind, volume_source, amount_source, styles)) in
            resolved.into_iter().enumerate()
        {
            let source =
                remap_indicator_source(&state.indicators[study].source, &remapped_outputs, source);
            let volume_source = state.indicators[study]
                .volume_source
                .as_ref()
                .map(|source| {
                    remap_indicator_source(
                        source,
                        &remapped_outputs,
                        volume_source.unwrap_or_default(),
                    )
                });
            let amount_source = state.indicators[study]
                .amount_source
                .as_ref()
                .map(|source| {
                    remap_indicator_source(
                        source,
                        &remapped_outputs,
                        amount_source.unwrap_or_default(),
                    )
                });
            let outputs = self.add_indicator_kind_with_sources(
                source,
                source_input,
                kind,
                volume_source,
                amount_source,
            );
            if outputs.len() != styles.len() {
                return Err(invalid(format!("indicator {study} could not be restored")));
            }
            for (&output, style) in outputs.iter().zip(styles) {
                if !self.set_indicator_output_style(output, style) {
                    return Err(invalid(format!(
                        "indicator {study} style could not be restored"
                    )));
                }
            }
            remapped_outputs.push(outputs);
        }
        self.study_restore_pane_cursor = None;
        result.schema_version = PERSISTENCE_SCHEMA_VERSION_STUDIES;
        Ok(result)
    }

    fn import_state_v2(&mut self, state: StateV2) -> Result<PersistenceRestoreResult, ChartError> {
        if self.panes.len() != 1
            || self.panes[0].general_horizontal_domain.is_some()
            || !self.drawings.is_empty()
            || self.next_drawing_id != 1
            || self.general_dataset_count() != 0
            || self.general_series_count() != 0
            || self.general_reference_count() != 0
            || self.general_axes.has_issued_handles()
        {
            return Err(ChartError::new(
                ErrorCode::UnsupportedOperation,
                "state import requires a fresh chart before general or drawing handles have been issued",
            ));
        }
        if !state.chart_options.is_object() {
            return Err(invalid("V2 chart_options must be an object"));
        }
        let chart_options =
            crate::exchange_time_api::importable_chart_options(&state.chart_options);
        serde_json::from_value::<aeris_charts_core::options::ChartOptions>(chart_options.clone())
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        let domains = state
            .panes
            .iter()
            .map(|pane| pane.horizontal_domain)
            .collect::<Vec<_>>();
        let validated = Self::validate_state_v1(StateV1 {
            schema: state.schema,
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            panes: state.panes.into_iter().map(|pane| pane.pane).collect(),
            drawings: state.drawings,
            drawing_price_basis: state.drawing_price_basis,
            drawing_catalog: state.drawing_catalog,
            hidden_mark_groups: state.hidden_mark_groups,
        })?;
        let mut staged = ChartEngine::new(self.css_width, self.css_height, self.dpr);
        staged.next_pane_id = self.next_pane_id;
        let mut result = staged.install_validated_state(validated)?;
        for (pane, domain) in staged.panes.iter_mut().zip(domains) {
            pane.general_horizontal_domain = staged.general_horizontal_domains.register(domain)?;
        }
        for axis in state.axes {
            staged.add_general_axis(axis)?;
        }
        for reference in state.references {
            staged.add_general_reference(reference)?;
        }
        let mut datasets = HashMap::new();
        for dataset in state.datasets {
            if datasets.contains_key(&dataset.id) {
                return Err(invalid(format!("duplicate dataset id {:?}", dataset.id)));
            }
            let id = staged.create_general_xy_dataset(dataset.input.clone())?;
            if let Some(labels) = dataset.labels {
                staged.replace_general_xy_dataset_labeled(id, dataset.input, Some(labels))?;
            }
            datasets.insert(dataset.id, id);
        }
        for series in state.series {
            let dataset = *datasets.get(&series.dataset).ok_or_else(|| {
                invalid(format!(
                    "series references unknown dataset {:?}",
                    series.dataset
                ))
            })?;
            staged.add_general_series(crate::GeneralSeriesOptions {
                kind: series.kind,
                pane: series.pane,
                dataset,
                x_axis_id: series.x_axis_id,
                y_axis_id: series.y_axis_id,
                visible: series.visible,
                title: series.title,
                color: series.color,
                point_radius: series.point_radius,
                point_markers: series.point_markers,
                point_symbol: series.point_symbol,
                line_width: series.line_width,
                line_style: series.line_style,
                interpolation: series.interpolation,
                connect_missing: series.connect_missing,
                fill_opacity: series.fill_opacity,
                baseline_value: series.baseline_value,
                data_labels: series.data_labels,
                group_id: series.group_id,
                stack_id: series.stack_id,
                stack_mode: series.stack_mode,
            })?;
        }
        // The document's exchange-time keys are optional and an absent key keeps the chart's
        // installed value, so both the staged chart and the live chart judge the options against
        // the exchange time the live chart will run with.
        staged.exchange_time = self.exchange_time.clone();
        let options_json = serde_json::to_string(&chart_options)
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        staged
            .apply_options(&options_json)
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        // Everything the live chart's own state can reject (an installed bar time label that the
        // document's session start does not fit) is decided before the first field is replaced.
        let prepared_options = self
            .prepare_options_patch(&chart_options)
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        self.panes = staged.panes;
        self.general_horizontal_domains = staged.general_horizontal_domains;
        self.general_axes = staged.general_axes;
        self.general_data = staged.general_data;
        self.general_series = staged.general_series;
        let price_basis = staged.drawing_settings.price_basis.take();
        self.drawings = staged.drawings;
        self.restore_drawing_time_identity(price_basis);
        self.next_pane_id = staged.next_pane_id;
        self.next_persistent_pane_id = staged.next_persistent_pane_id;
        self.next_drawing_id = staged.next_drawing_id;
        self.options = staged.options;
        self.timeline_marks
            .install_hidden_groups(std::mem::take(&mut staged.timeline_marks.hidden_groups));
        self.apply_prepared_options(&chart_options, prepared_options);
        // A document without exchange-time keys keeps the chart's installed zone and session
        // start; mirror a non-default one so the replaced options store still describes the live
        // chart (absent keys already mean UTC, keeping default documents byte-stable).
        if !self.exchange_time().is_utc_identity()
            || self.time_zone != crate::ChartTimeZone::default()
        {
            self.mirror_exchange_time_options();
        }
        // Likewise explicit time-axis marks the document does not carry stay installed.
        if self.time_tick_marks().is_some() {
            self.mirror_time_tick_marks_option();
        }
        // And a close-time bar label a document without the key does not carry.
        if *self.bar_time_label() != crate::BarTimeLabel::Open {
            self.mirror_bar_time_label_option();
        }
        self.selected_drawing = None;
        self.selected_drawings.clear();
        self.drawing_drag = None;
        self.drawing_history = crate::DrawingHistory::default();
        self.drawing_controller.pending = None;
        self.drawing_controller.brush = None;
        self.drawing_controller.measure = None;
        self.drawing_text_edit = None;
        self.hovered_drawing = None;
        self.hovered_text = None;
        for series in &mut self.series {
            if !series.removed {
                series.pane_index = 0;
            }
        }
        self.drawing_runtime
            .borrow_mut()
            .rebuild_all(&self.drawings, self.panes.len());
        self.layout_panes(self.pane_h);
        self.invalidate_frame_all();
        result.schema_version = PERSISTENCE_SCHEMA_VERSION_GENERAL;
        Ok(result)
    }

    /// Import with per-stage timings for the repository's release evidence harness.
    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    pub fn import_state_json_profiled(
        &mut self,
        json: &str,
    ) -> Result<PersistenceRestoreProfile, ChartError> {
        let parse_started = Instant::now();
        let state = Self::parse_state_json(json)?;
        let parse_ns = parse_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let validation_started = Instant::now();
        let state = Self::validate_state_v1(state)?;
        let validation_ns = validation_started
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let mut install = InstallProfile::default();
        let restore = self.install_validated_state_inner(state, Some(&mut install))?;
        Ok(PersistenceRestoreProfile {
            restore,
            parse_ns,
            validation_ns,
            semantic_install_ns: install.semantic_install_ns,
            index_rebuild_ns: install.index_rebuild_ns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AggressorSide, FootprintAggregationOptions, FootprintBarAggregation,
        FootprintSeriesOptions, FootprintTrade,
    };

    const MINIMAL: &str = include_str!("../fixtures/persistence/minimal-v1.json");
    const VALID: &str = include_str!("../fixtures/persistence/valid-v1.json");
    const ALL_DRAWINGS: &str =
        include_str!("../fixtures/persistence/all-drawings-multipane-v1.json");
    const MALFORMED: &str = include_str!("../fixtures/persistence/malformed.json");
    const UNKNOWN_VERSION: &str = include_str!("../fixtures/persistence/unknown-version.json");
    const UNKNOWN_KIND: &str = include_str!("../fixtures/persistence/unknown-kind-v1.json");

    fn settled_chart() -> ChartEngine {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        chart.build_frame();
        chart
    }

    #[test]
    fn historical_volatility_persists_annualization_and_output_identity() {
        let mut chart = settled_chart();
        let output = chart.add_historical_volatility(0, 3, 365.0).unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::HistoricalVolatility {
                period: 3,
                annualization: 365.0,
            }
        );
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn trix_persists_two_ordered_outputs() {
        let mut chart = settled_chart();
        let outputs = chart.add_trix(0, 2, 3);
        assert_eq!(outputs.len(), 2);
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::Trix {
                period: 2,
                signal: 3
            }
        );
        assert_eq!(binding.outputs, outputs);
    }

    #[test]
    fn negative_bollinger_deviation_documents_are_rejected_atomically() {
        for add in [
            ChartEngine::add_bollinger,
            ChartEngine::add_bollinger_metrics,
        ] {
            let mut chart = settled_chart();
            assert!(!add(&mut chart, 0, 3, 2.0).is_empty());
            let document = chart.export_state_json().unwrap();
            assert_eq!(document.matches(r#""deviation":2.0"#).count(), 1);
            let negative = document.replace(r#""deviation":2.0"#, r#""deviation":-1.0"#);
            let mut target = settled_chart();
            let before = target.export_state_json().unwrap();
            assert_eq!(
                target.import_state_json(&negative).unwrap_err().code(),
                ErrorCode::InvalidData
            );
            assert_eq!(target.export_state_json().unwrap(), before);
        }
    }

    #[test]
    fn structure_kinds_round_trip_without_annotations_and_reject_invalid_parameters() {
        let mut chart = settled_chart();
        let kinds = [
            crate::IndicatorKind::SwingPoints { left: 2, right: 3 },
            crate::IndicatorKind::MarketStructure {
                left: 2,
                right: 3,
                break_on: crate::indicators::StructureBreakOn::Wick,
            },
            crate::IndicatorKind::FairValueGaps {
                min_size: 0.5,
                mitigation: crate::indicators::StructureMitigation::Full,
                mitigation_price: crate::indicators::StructureMitigationPrice::Close,
                max_active: 12,
                show_mitigated: true,
            },
            crate::IndicatorKind::OrderBlocks {
                left: 2,
                right: 3,
                break_on: crate::indicators::StructureBreakOn::Close,
                zone: crate::indicators::OrderBlockZone::Body,
                mitigation: crate::indicators::StructureMitigation::Touch,
                mitigation_price: crate::indicators::StructureMitigationPrice::Wick,
                max_active: 16,
                show_mitigated: false,
            },
        ];
        for kind in &kinds {
            assert!(!chart.add_indicator_kind(0, kind.clone(), None).is_empty());
        }
        let document = chart.export_state_json().unwrap();
        assert!(!document.contains("\"annotations\""));
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(
            restored
                .indicator_bindings()
                .iter()
                .map(|binding| &binding.kind)
                .collect::<Vec<_>>(),
            kinds.iter().collect::<Vec<_>>(),
        );
        for (index, field, value) in [
            (0, "left", serde_json::json!(0)),
            (1, "right", serde_json::json!(51)),
            (2, "max_active", serde_json::json!(0)),
            (3, "max_active", serde_json::json!(65)),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
            invalid["indicators"][index]["kind"][field] = value;
            assert!(
                settled_chart()
                    .import_state_json(&invalid.to_string())
                    .is_err()
            );
        }
        let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
        invalid["indicators"][0]["source_input"] = serde_json::json!("hlc3");
        assert!(
            settled_chart()
                .import_state_json(&invalid.to_string())
                .is_err()
        );
    }

    #[test]
    fn choppiness_and_atr_bands_v3_round_trip_and_old_layout() {
        let mut chart = settled_chart();
        let chop = chart.add_choppiness(0, 3).unwrap();
        let bands = chart.add_atr_bands(0, 4, 2.75);
        assert_eq!(bands.len(), 3);
        assert!(chart.set_indicator_output_style(
            bands[1],
            crate::IndicatorOutputStyle {
                visible: false,
                line_color: Some("#123456".into()),
                ..Default::default()
            },
        ));
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(
            bindings[0].kind,
            crate::IndicatorKind::Choppiness { period: 3 }
        );
        assert_eq!(bindings[0].outputs, [chop]);
        assert_eq!(
            bindings[1].kind,
            crate::IndicatorKind::AtrBands {
                period: 4,
                multiplier: 2.75
            }
        );
        assert_eq!(bindings[1].outputs, bands);
        assert_eq!(bindings[1].styles[1].line_color.as_deref(), Some("#123456"));
        for output in [chop].into_iter().chain(bands) {
            let before = chart.data.series_data(output).unwrap();
            let after = restored.data.series_data(output).unwrap();
            assert_eq!(before.0, after.0);
            for (left, right) in before.1.into_iter().zip(after.1) {
                assert!(
                    left.iter()
                        .zip(right)
                        .all(|(a, b)| a == b || a.is_nan() && b.is_nan())
                );
            }
        }
        for (index, field, value) in [
            (0, "period", serde_json::json!(1)),
            (1, "multiplier", serde_json::json!(-1.0)),
            (1, "period", serde_json::json!(0)),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
            invalid["indicators"][index]["kind"][field] = value;
            assert!(
                settled_chart()
                    .import_state_json(&invalid.to_string())
                    .is_err()
            );
        }

        // An earlier V3 document with only legacy kind tags still imports unchanged.
        let mut legacy = settled_chart();
        let sma = legacy.add_sma(0, 3).unwrap();
        let old_document = legacy.export_state_json().unwrap();
        let mut target = settled_chart();
        target.import_state_json(&old_document).unwrap();
        assert_eq!(
            target.indicator_bindings()[0].kind,
            crate::IndicatorKind::Sma { period: 3 }
        );
        assert_eq!(target.indicator_bindings()[0].outputs, [sma]);
    }

    #[test]
    fn adaptive_regression_and_klinger_round_trip_and_reject_invalid_parameters() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..40).map(|row| row as f64 * 3600.0).collect::<Vec<_>>();
        let values = (0..40).map(|row| 10.0 + row as f64).collect::<Vec<_>>();
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let cases = [
            (
                crate::IndicatorKind::Klinger {
                    fast: 3,
                    slow: 7,
                    signal: 4,
                },
                chart.add_klinger(0, volume, 3, 7, 4),
            ),
            (
                crate::IndicatorKind::Kama {
                    period: 5,
                    fast: 2,
                    slow: 10,
                },
                vec![chart.add_kama(0, 5, 2, 10).unwrap()],
            ),
            (
                crate::IndicatorKind::McGinley { period: 5 },
                vec![chart.add_mcginley(0, 5).unwrap()],
            ),
            (
                crate::IndicatorKind::LinearRegression {
                    period: 5,
                    deviation: 2.0,
                },
                chart.add_linear_regression(0, 5, 2.0),
            ),
        ];
        for (kind, outputs) in &cases {
            assert_eq!(outputs.len(), incremental_output_count(kind));
        }
        for (index, (_, outputs)) in cases.iter().enumerate() {
            for (slot, &output) in outputs.iter().enumerate() {
                assert!(chart.set_indicator_output_style(
                    output,
                    crate::IndicatorOutputStyle {
                        visible: (index + slot) % 2 == 0,
                        line_color: Some("#336699".to_string()),
                        line_width: Some(1.5 + slot as f64),
                        ..crate::IndicatorOutputStyle::default()
                    }
                ));
            }
        }
        let original_bindings = chart.indicator_bindings();
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        for (((kind, outputs), before), binding) in cases
            .iter()
            .zip(&original_bindings)
            .zip(restored.indicator_bindings())
        {
            assert_eq!(&binding.kind, kind);
            assert_eq!(&binding.outputs, outputs);
            assert_eq!(binding.styles, before.styles);
            assert_eq!(
                binding.volume_source,
                matches!(kind, crate::IndicatorKind::Klinger { .. }).then_some(restored_volume)
            );
            for &output in outputs {
                let original = chart.data.series_data(output).unwrap();
                let round_trip = restored.data.series_data(output).unwrap();
                assert_eq!(original.0, round_trip.0);
                for (left, right) in original.1.into_iter().zip(round_trip.1) {
                    assert_eq!(left.len(), right.len());
                    for (&left, &right) in left.iter().zip(right) {
                        assert!(left == right || left.is_nan() && right.is_nan());
                    }
                }
            }
        }
        for (index, field, value) in [
            (0, "signal", serde_json::json!(0)),
            (0, "fast", serde_json::json!(7)),
            (1, "period", serde_json::json!(0)),
            (1, "slow", serde_json::json!(2)),
            (2, "period", serde_json::json!(0)),
            (3, "period", serde_json::json!(0)),
            (3, "deviation", serde_json::json!(-1.0)),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
            invalid["indicators"][index]["kind"][field] = value;
            let mut target = settled_chart();
            let baseline = target.export_state_json().unwrap();
            assert!(target.import_state_json(&invalid.to_string()).is_err());
            assert_eq!(target.export_state_json().unwrap(), baseline);
        }
    }

    #[test]
    fn momentum_studies_v3_round_trip_parameters_outputs_and_styles() {
        let mut chart = settled_chart();
        let cases = [
            (
                crate::IndicatorKind::Kst {
                    roc: [2, 3, 4, 5],
                    smoothing: [3, 2, 4, 2],
                    signal: 3,
                },
                chart.add_kst(0, [2, 3, 4, 5], [3, 2, 4, 2], 3),
            ),
            (
                crate::IndicatorKind::Tsi {
                    long: 4,
                    short: 2,
                    signal: 3,
                },
                chart.add_tsi(0, 4, 2, 3),
            ),
            (
                crate::IndicatorKind::MassIndex {
                    ema_period: 2,
                    sum_period: 3,
                },
                vec![chart.add_mass_index(0, 2, 3).unwrap()],
            ),
            (
                crate::IndicatorKind::Vortex { period: 4 },
                chart.add_vortex(0, 4),
            ),
        ];
        for (index, (kind, outputs)) in cases.iter().enumerate() {
            let expected = incremental_output_count(kind);
            assert_eq!(outputs.len(), expected);
            for (slot, &output) in outputs.iter().enumerate() {
                let style = crate::IndicatorOutputStyle {
                    visible: (index + slot) % 2 == 0,
                    line_color: Some(format!("#{:06x}", 0x224466 + index * 0x1100 + slot)),
                    line_width: Some(1.25 + index as f64 + slot as f64),
                    line_style: (index + slot) as u8 % 5,
                    point_markers: slot == 1,
                    ..crate::IndicatorOutputStyle::default()
                };
                assert!(chart.set_indicator_output_style(output, style));
            }
        }
        let expected = chart.indicator_bindings();
        let document = chart.export_state_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(parsed["schema_version"], PERSISTENCE_SCHEMA_VERSION_STUDIES);
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let actual = restored.indicator_bindings();
        assert_eq!(actual.len(), cases.len());
        for (before, after) in expected.iter().zip(&actual) {
            assert_eq!(after.kind, before.kind);
            assert_eq!(after.outputs.len(), before.outputs.len());
            assert_eq!(after.styles, before.styles);
            for (&old, &new) in before.outputs.iter().zip(&after.outputs) {
                let original = chart.data.series_data(old).unwrap();
                let round_trip = restored.data.series_data(new).unwrap();
                assert_eq!(original.0, round_trip.0);
                for (left, right) in original.1.into_iter().zip(round_trip.1) {
                    assert_eq!(left.len(), right.len());
                    for (&left, &right) in left.iter().zip(right) {
                        assert!(left == right || left.is_nan() && right.is_nan());
                    }
                }
            }
        }
        for (study, field) in [
            (0, "roc"),
            (0, "smoothing"),
            (0, "signal"),
            (1, "long"),
            (1, "short"),
            (1, "signal"),
            (2, "ema_period"),
            (2, "sum_period"),
            (3, "period"),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
            if matches!(field, "roc" | "smoothing") {
                invalid["indicators"][study]["kind"][field][2] = serde_json::json!(0);
            } else {
                invalid["indicators"][study]["kind"][field] = serde_json::json!(0);
            }
            let mut untouched = settled_chart();
            let baseline = untouched.export_state_json().unwrap();
            assert!(
                untouched
                    .import_state_json(&serde_json::to_string(&invalid).unwrap())
                    .is_err(),
                "study {study} {field}"
            );
            assert_eq!(untouched.export_state_json().unwrap(), baseline);
        }
    }

    #[test]
    fn coppock_curve_persists_all_three_periods() {
        let mut chart = settled_chart();
        let output = chart.add_coppock_curve(0, 4, 3, 2).unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::CoppockCurve {
                long: 4,
                short: 3,
                smoothing: 2,
            }
        );
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn fisher_transform_persists_two_ordered_outputs() {
        let mut chart = settled_chart();
        let outputs = chart.add_fisher_transform(0, 5);
        assert_eq!(outputs.len(), 2);
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::FisherTransform { period: 5 }
        );
        assert_eq!(binding.outputs, outputs);
    }

    #[test]
    fn ultimate_oscillator_persists_periods_and_output() {
        let mut chart = settled_chart();
        let output = chart.add_ultimate_oscillator(0, 3, 5, 7).unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::UltimateOscillator {
                short: 3,
                medium: 5,
                long: 7
            }
        );
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn study_persistence_round_trips_dependencies_inputs_volume_and_styles() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let sma = chart
            .add_indicator_kind_with_input(
                0,
                crate::IndicatorInputSource::Hlc3,
                crate::IndicatorKind::Sma { period: 2 },
                None,
            )
            .into_iter()
            .next()
            .unwrap();
        let bands = chart.add_bollinger(sma, 3, 2.0);
        let vwap = chart.add_vwap(0, Some(volume)).unwrap();
        let style = IndicatorOutputStyle {
            visible: false,
            line_color: Some("#123456".into()),
            line_width: Some(3.0),
            line_style: 2,
            point_markers: true,
            up_color: Some("#00ff00".into()),
            down_color: Some("#ff0000".into()),
            area_top_color: Some("rgba(1, 2, 3, 0.4)".into()),
            area_bottom_color: Some("rgba(4, 5, 6, 0.2)".into()),
        };
        assert!(chart.set_indicator_output_style(vwap, style.clone()));
        let document = chart.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["schema_version"], PERSISTENCE_SCHEMA_VERSION_STUDIES);
        assert_eq!(value["indicators"].as_array().unwrap().len(), 3);

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].source_input, crate::IndicatorInputSource::Hlc3);
        assert_eq!(bindings[1].source, bindings[0].outputs[0]);
        assert_eq!(bindings[2].volume_source, Some(restored_volume));
        assert_eq!(bindings[2].styles[0], style);
        assert_eq!(bands.len(), 3);

        let mut invalid_document: serde_json::Value = serde_json::from_str(&document).unwrap();
        invalid_document["indicators"][0]["kind"]["period"] = serde_json::json!(0);
        let invalid_document = serde_json::to_string(&invalid_document).unwrap();
        let mut untouched = settled_chart();
        untouched.add_series(crate::SeriesKind::Histogram);
        let baseline = untouched.export_state_json().unwrap();
        assert!(untouched.import_state_json(&invalid_document).is_err());
        assert_eq!(untouched.export_state_json().unwrap(), baseline);
    }

    #[test]
    fn f4_exit_fixture_round_trips_hlc3_rsi_sma_and_bollinger_fill() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let times = (0..16)
            .map(|index| (index * 3_600) as f64)
            .collect::<Vec<_>>();
        let close = (0..16)
            .map(|index| 100.0 + (index as f64 * 0.7).sin() * 3.0 + index as f64 * 0.2)
            .collect::<Vec<_>>();
        let high = close.iter().map(|value| value + 1.25).collect::<Vec<_>>();
        let low = close.iter().map(|value| value - 1.0).collect::<Vec<_>>();
        let open = close.iter().map(|value| value - 0.15).collect::<Vec<_>>();
        chart
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        let rsi = chart
            .add_indicator_kind_with_input(
                0,
                crate::IndicatorInputSource::Hlc3,
                crate::IndicatorKind::Rsi {
                    period: 3,
                    seed: crate::IndicatorSeed::Sma,
                },
                None,
            )
            .into_iter()
            .next()
            .unwrap();
        let sma = chart.add_sma(rsi, 2).unwrap();
        let bands = chart.add_bollinger(sma, 2, 2.0);
        let fill_style = IndicatorOutputStyle {
            area_top_color: Some("rgba(20, 120, 220, 0.24)".into()),
            area_bottom_color: Some("rgba(20, 120, 220, 0.04)".into()),
            ..IndicatorOutputStyle::default()
        };
        assert!(chart.set_indicator_output_style(bands[0], fill_style.clone()));
        let document = chart.export_state_json().unwrap();
        let original_frame = chart.build_frame();
        let original_values = chart
            .indicator_bindings()
            .iter()
            .flat_map(|binding| {
                binding
                    .outputs
                    .iter()
                    .map(|&id| chart.data.series_data(id).unwrap().1[3].to_vec())
            })
            .collect::<Vec<_>>();

        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored
            .set_series_data(0, &times, &open, &high, &low, &close)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[0].source_input, crate::IndicatorInputSource::Hlc3);
        assert_eq!(bindings[1].source, bindings[0].outputs[0]);
        assert_eq!(bindings[2].source, bindings[1].outputs[0]);
        assert_eq!(bindings[2].styles[0], fill_style);
        let restored_values = bindings
            .iter()
            .flat_map(|binding| {
                binding
                    .outputs
                    .iter()
                    .map(|&id| restored.data.series_data(id).unwrap().1[3].to_vec())
            })
            .collect::<Vec<_>>();
        assert_eq!(restored_values, original_values);
        assert_eq!(restored.build_frame(), original_frame);
    }

    #[test]
    fn obv_persistence_round_trips_volume_source() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let output = chart.add_obv(0, volume).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::Obv);
        assert_eq!(binding.volume_source, Some(restored_volume));
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn cumulative_volume_studies_persist_volume_binding_and_output_identity() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let adl = chart.add_accumulation_distribution(0, volume).unwrap();
        let pvt = chart.add_price_volume_trend(0, volume).unwrap();
        let chaikin = chart.add_chaikin_oscillator(0, volume, 3, 7).unwrap();
        let relative = chart.add_relative_volume(0, volume, 3).unwrap();
        let oscillator = chart.add_volume_oscillator(0, volume, 2, 4, 3);
        assert_eq!(oscillator.len(), 3);
        let elder = chart.add_elder_force(0, volume, 3).unwrap();
        let ease = chart.add_ease_of_movement(0, volume, 3, 100.0).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(
            bindings[0].kind,
            crate::IndicatorKind::AccumulationDistribution
        );
        assert_eq!(bindings[1].kind, crate::IndicatorKind::PriceVolumeTrend);
        assert_eq!(bindings[0].volume_source, Some(restored_volume));
        assert_eq!(bindings[1].volume_source, Some(restored_volume));
        assert_eq!(bindings[0].outputs, vec![adl]);
        assert_eq!(bindings[1].outputs, vec![pvt]);
        assert_eq!(
            bindings[2].kind,
            crate::IndicatorKind::ChaikinOscillator { fast: 3, slow: 7 }
        );
        assert_eq!(bindings[2].volume_source, Some(restored_volume));
        assert_eq!(bindings[2].outputs, vec![chaikin]);
        assert_eq!(
            bindings[3].kind,
            crate::IndicatorKind::RelativeVolume { period: 3 }
        );
        assert_eq!(bindings[3].volume_source, Some(restored_volume));
        assert_eq!(bindings[3].outputs, vec![relative]);
        assert_eq!(
            bindings[4].kind,
            crate::IndicatorKind::VolumeOscillator {
                fast: 2,
                slow: 4,
                signal: 3
            }
        );
        assert_eq!(bindings[4].volume_source, Some(restored_volume));
        assert_eq!(bindings[4].outputs, oscillator);
        assert_eq!(
            bindings[5].kind,
            crate::IndicatorKind::ElderForce { period: 3 }
        );
        assert_eq!(bindings[5].volume_source, Some(restored_volume));
        assert_eq!(bindings[5].outputs, vec![elder]);
        assert_eq!(
            bindings[6].kind,
            crate::IndicatorKind::EaseOfMovement {
                period: 3,
                divisor: 100.0
            }
        );
        assert_eq!(bindings[6].volume_source, Some(restored_volume));
        assert_eq!(bindings[6].outputs, vec![ease]);
    }

    #[test]
    fn cmf_persistence_round_trips_period_and_volume_source() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let output = chart.add_cmf(0, volume, 3).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::Cmf { period: 3 });
        assert_eq!(binding.volume_source, Some(restored_volume));
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn mfi_persistence_round_trips_period_and_volume_source() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let output = chart.add_mfi(0, volume, 3).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::Mfi { period: 3 });
        assert_eq!(binding.volume_source, Some(restored_volume));
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn volume_study_persistence_round_trips_outputs_and_volume_source() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        let outputs = chart.add_volume(0, volume, 3);
        assert_eq!(outputs.len(), 2);
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::Volume { period: 3 });
        assert_eq!(binding.volume_source, Some(restored_volume));
        assert_eq!(binding.outputs, outputs);
    }

    #[test]
    fn keltner_study_persistence_round_trips_three_outputs() {
        let mut chart = settled_chart();
        let outputs = chart.add_keltner(0, 3, 1.5);
        assert_eq!(outputs.len(), 3);
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0].kind,
            crate::IndicatorKind::Keltner {
                period: 3,
                multiplier: 1.5,
            }
        );
        assert_eq!(bindings[0].outputs.len(), 3);
    }

    #[test]
    fn adx_dmi_study_persistence_round_trips_three_outputs() {
        let mut chart = settled_chart();
        let outputs = chart.add_adx_dmi(0, 3);
        assert_eq!(outputs.len(), 3);
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].kind, crate::IndicatorKind::AdxDmi { period: 3 });
        assert_eq!(bindings[0].outputs.len(), 3);
    }

    #[test]
    fn parabolic_sar_persistence_round_trips_output() {
        let mut chart = settled_chart();
        let output = chart.add_parabolic_sar(0).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].kind, crate::IndicatorKind::ParabolicSar);
        assert_eq!(bindings[0].outputs, vec![output]);
    }

    #[test]
    fn supertrend_persistence_round_trips_parameters() {
        let mut chart = settled_chart();
        chart.add_supertrend(0, 3, 2.5).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(
            restored.indicator_bindings()[0].kind,
            crate::IndicatorKind::SuperTrend {
                period: 3,
                multiplier: 2.5,
            }
        );
    }

    #[test]
    fn ichimoku_persistence_round_trips_five_outputs() {
        let mut chart = settled_chart();
        let outputs = chart.add_ichimoku(0);
        assert_eq!(outputs.len(), 5);
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].kind, crate::IndicatorKind::Ichimoku);
        assert_eq!(bindings[0].outputs.len(), 5);
    }

    #[test]
    fn cci_persistence_round_trips_period_and_output() {
        let mut chart = settled_chart();
        let output = chart.add_cci(0, 3).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::Cci { period: 3 });
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn williams_r_persistence_round_trips_period_and_output() {
        let mut chart = settled_chart();
        let output = chart.add_williams_r(0, 3).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(binding.kind, crate::IndicatorKind::WilliamsR { period: 3 });
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn stochastic_rsi_persistence_round_trips_two_periods() {
        let mut chart = settled_chart();
        let output = chart.add_stochastic_rsi(0, 3, 4).unwrap();
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::StochasticRsi {
                rsi_period: 3,
                stochastic_period: 4,
            }
        );
        assert_eq!(binding.outputs, vec![output]);
    }

    #[test]
    fn momentum_and_roc_persistence_round_trip_periods() {
        for (kind, output) in [
            (crate::IndicatorKind::Momentum { period: 3 }, "momentum"),
            (crate::IndicatorKind::RateOfChange { period: 4 }, "roc"),
        ] {
            let mut chart = settled_chart();
            let id = match &kind {
                crate::IndicatorKind::Momentum { period } => {
                    chart.add_momentum(0, *period).unwrap()
                }
                crate::IndicatorKind::RateOfChange { period } => chart.add_roc(0, *period).unwrap(),
                _ => unreachable!("test kind"),
            };
            let document = chart.export_state_json().unwrap();
            let mut restored = settled_chart();
            restored.import_state_json(&document).unwrap();
            assert_eq!(restored.indicator_bindings()[0].kind, kind);
            assert_eq!(restored.indicator_bindings()[0].outputs, vec![id]);
            assert_eq!(restored.indicator_info(id).unwrap().kind, output);
        }
    }

    #[test]
    fn breadth_indicators_persist_output_identity_and_style() {
        let mut chart = settled_chart();
        let aroon = chart.add_aroon(0, 3);
        let awesome = chart.add_awesome_oscillator(0).unwrap();
        let dpo = chart.add_dpo(0, 5).unwrap();
        let cmo = chart.add_chande_momentum(0, 5).unwrap();
        let metrics = chart.add_bollinger_metrics(0, 5, 2.0);
        let envelopes = chart.add_envelopes(0, 5, 10.0, true);
        let alma = chart.add_alma(0, 5, 0.85, 6.0).unwrap();
        assert_eq!(aroon.len(), 2);
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        let bindings = restored.indicator_bindings();
        assert!(matches!(
            bindings[0].kind,
            crate::IndicatorKind::Aroon { period: 3 }
        ));
        assert_eq!(bindings[0].outputs, aroon);
        assert!(matches!(
            bindings[1].kind,
            crate::IndicatorKind::AwesomeOscillator
        ));
        assert_eq!(bindings[1].outputs, vec![awesome]);
        assert!(matches!(
            bindings[2].kind,
            crate::IndicatorKind::Dpo { period: 5 }
        ));
        assert_eq!(bindings[2].outputs, vec![dpo]);
        assert!(matches!(
            bindings[3].kind,
            crate::IndicatorKind::ChandeMomentum { period: 5 }
        ));
        assert_eq!(bindings[3].outputs, vec![cmo]);
        assert!(matches!(
            bindings[4].kind,
            crate::IndicatorKind::BollingerMetrics {
                period: 5,
                deviation: 2.0
            }
        ));
        assert_eq!(bindings[4].outputs, metrics);
        assert!(matches!(
            bindings[5].kind,
            crate::IndicatorKind::Envelopes {
                period: 5,
                percent: 10.0,
                exponential: true
            }
        ));
        assert_eq!(bindings[5].outputs, envelopes);
        assert!(matches!(
            bindings[6].kind,
            crate::IndicatorKind::Alma {
                period: 5,
                offset: 0.85,
                sigma: 6.0
            }
        ));
        assert_eq!(bindings[6].outputs, vec![alma]);
        assert_eq!(
            restored.series_kind(awesome),
            Some(crate::SeriesKind::Histogram)
        );
    }

    #[test]
    fn pivot_points_persistence_round_trips_formula_family() {
        let mut chart = settled_chart();
        let outputs = chart.add_pivot_points(0, aeris_charts_indicators::PivotKind::Camarilla);
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored
            .set_series_data(
                0,
                &[
                    0.0, 3_600.0, 7_200.0, 10_800.0, 14_400.0, 18_000.0, 21_600.0, 25_200.0,
                    28_800.0, 32_400.0,
                ],
                &[11.0; 10],
                &[11.0; 10],
                &[11.0; 10],
                &[11.0; 10],
            )
            .unwrap();
        restored.import_state_json(&document).unwrap();
        let binding = &restored.indicator_bindings()[0];
        assert_eq!(
            binding.kind,
            crate::IndicatorKind::PivotPoints {
                variant: aeris_charts_indicators::PivotKind::Camarilla,
            }
        );
        assert_eq!(binding.outputs.len(), 5);
        assert_eq!(
            restored.indicator_info(outputs[0]).unwrap().kind,
            "pivot_points"
        );
    }

    #[test]
    fn zigzag_persistence_round_trips_deviation() {
        let mut chart = settled_chart();
        let output = chart.add_zigzag(0, 4.5);
        let document = chart.export_state_json().unwrap();
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(
            restored.indicator_bindings()[0].kind,
            crate::IndicatorKind::ZigZag {
                deviation_percent: 4.5
            }
        );
        assert_eq!(restored.indicator_bindings()[0].outputs, vec![output]);
        assert_eq!(restored.indicator_info(output).unwrap().kind, "zigzag");
        assert_eq!(
            restored
                .indicator_info(output)
                .unwrap()
                .parameters
                .deviation_percent,
            Some(4.5)
        );
    }

    #[test]
    fn vwap_bands_persistence_round_trips_reset_and_parameters() {
        let mut chart = settled_chart();
        let outputs = chart.add_vwap_bands(
            0,
            None,
            aeris_charts_indicators::VwapReset::Monthly,
            1.25,
            7.5,
        );
        assert_eq!(outputs.len(), 5);
        let document = chart.export_state_json().unwrap();

        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(
            restored.indicator_bindings()[0].kind,
            crate::IndicatorKind::VwapBands {
                reset: aeris_charts_indicators::VwapReset::Monthly,
                standard_deviation: 1.25,
                percent: 7.5,
            }
        );
        assert_eq!(restored.indicator_bindings()[0].outputs, outputs);
    }

    #[test]
    fn supported_fixtures_restore_and_canonical_output_round_trips() {
        for fixture in [MINIMAL, VALID, ALL_DRAWINGS] {
            let mut first = ChartEngine::new(800.0, 500.0, 1.0);
            first.import_state_json(fixture).unwrap();
            let canonical = first.export_state_json().unwrap();
            let mut second = ChartEngine::new(800.0, 500.0, 1.0);
            second.import_state_json(&canonical).unwrap();
            assert_eq!(second.export_state_json().unwrap(), canonical);
        }
    }

    #[test]
    fn b2_common_drawing_state_persists_losslessly() {
        let mut chart = settled_chart();
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 1.0,
                        price: 10.0,
                    },
                    DrawingPoint {
                        logical: 6.0,
                        price: 12.0,
                    },
                ],
                None,
            )
            .unwrap();
        chart
            .drawing_apply_options(
                id,
                r#"{"name":"release","group_id":"macro","visible":false,"locked":true,"z_order":-4,"stroke_end":"arrow","extend_right":true,"labels":[{"metric":"price","visible":true,"position":"above"}]}"#,
            );
        let document = chart.export_state_json().unwrap();
        assert!(document.contains("release"));
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        let drawing = restored.drawing(id).unwrap();
        assert_eq!(drawing.name, "release");
        assert_eq!(drawing.group_id.as_deref(), Some("macro"));
        assert!(!drawing.visible);
        assert!(drawing.locked);
        assert_eq!(drawing.stroke_end, crate::DrawingLineCap::Arrow);
        assert_eq!(drawing.labels.len(), 1);
    }

    #[test]
    fn time_chart_anchor_times_restore_into_a_shifted_window_before_or_after_data() {
        const BASE: f64 = 1_704_067_200.0;
        let hours = |from: f64, count: usize| {
            (0..count)
                .map(|index| from + index as f64 * 3_600.0)
                .collect::<Vec<_>>()
        };
        let install = |chart: &mut ChartEngine, times: &[f64]| {
            let values = vec![10.0; times.len()];
            chart
                .set_series_data(0, times, &values, &values, &values, &values)
                .unwrap();
        };
        let mut source = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut source, &hours(BASE, 100));
        let id = source
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 50.0,
                        price: 10.0,
                    },
                    DrawingPoint {
                        logical: 60.25,
                        price: 11.0,
                    },
                ],
                None,
            )
            .unwrap();
        source.set_drawing_price_basis(Some("qfq")).unwrap();
        let document = source.export_state_json().unwrap();
        assert!(document.contains(&format!("\"time\":{}", BASE + 50.0 * 3_600.0)));
        assert!(document.contains("\"drawing_price_basis\":\"qfq\""));

        // Grid-workspace order: restore first, then the host loads the latest bars, which now
        // start twenty hours later than when the layout was saved.
        let mut before_data = ChartEngine::new(800.0, 500.0, 1.0);
        before_data.import_state_json(&document).unwrap();
        assert_eq!(before_data.drawing_price_basis(), Some("qfq"));
        assert_eq!(
            before_data.export_state_json().unwrap(),
            document,
            "unresolved anchors round-trip their time identity unchanged"
        );
        install(&mut before_data, &hours(BASE + 20.0 * 3_600.0, 100));
        let points = &before_data.drawing(id).unwrap().points;
        assert_eq!(points[0].logical, 30.0);
        assert!((points[1].logical - 40.25).abs() < 1e-9);

        // Restoring after data (more history loaded) resolves immediately.
        let mut after_data = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut after_data, &hours(BASE - 30.0 * 3_600.0, 200));
        after_data.import_state_json(&document).unwrap();
        let points = &after_data.drawing(id).unwrap().points;
        assert_eq!(points[0].logical, 80.0);
        assert!((points[1].logical - 90.25).abs() < 1e-9);

        // Legacy documents without anchor times keep their logical anchors.
        let first_time = serde_json::to_string(&(BASE + 50.0 * 3_600.0)).unwrap();
        let legacy = document.replace(&format!(",\"time\":{first_time}"), "");
        assert_ne!(legacy, document);
        let mut legacy_chart = ChartEngine::new(800.0, 500.0, 1.0);
        install(&mut legacy_chart, &hours(BASE - 30.0 * 3_600.0, 200));
        legacy_chart.import_state_json(&legacy).unwrap();
        assert_eq!(legacy_chart.drawing(id).unwrap().points[0].logical, 50.0);
    }

    #[test]
    fn non_time_drawing_anchor_times_restore_against_a_rebased_sequence() {
        let trade = |timestamp_micros, price| FootprintTrade {
            timestamp_micros,
            price,
            volume: 1.0,
            aggressor: AggressorSide::Buy,
            bid: None,
            ask: None,
            sequence: None,
            trade_id: None,
            conditions: 0,
            session_id: Some(1),
        };
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let footprint = chart
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        chart
            .set_footprint_trades(
                footprint,
                vec![trade(2_000_001, 100.0), trade(3_000_001, 101.0)],
            )
            .unwrap();
        let id = chart
            .add_drawing(
                DrawingKind::TrendLine,
                0,
                vec![
                    DrawingPoint {
                        logical: 0.5,
                        price: 100.0,
                    },
                    DrawingPoint {
                        logical: 1.0,
                        price: 101.0,
                    },
                ],
                None,
            )
            .unwrap();
        let document = chart.export_state_json().unwrap();
        assert!(document.contains("anchor_times_micros"));
        // Non-time sequence rows are not UTC times, so no seconds-based anchor time is emitted.
        assert!(!document.contains("\"time\""));

        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        let restored_footprint = restored
            .add_footprint_series(FootprintSeriesOptions {
                aggregation: FootprintAggregationOptions {
                    tick_size: 1.0,
                    ticks_per_row: 1,
                    bars: FootprintBarAggregation::Trades { trades_per_bar: 1 },
                    ..FootprintAggregationOptions::default()
                },
                ..FootprintSeriesOptions::default()
            })
            .unwrap();
        restored
            .set_footprint_trades(
                restored_footprint,
                vec![
                    trade(1_000_001, 99.0),
                    trade(2_000_001, 100.0),
                    trade(3_000_001, 101.0),
                ],
            )
            .unwrap();

        let points = &restored.drawing(id).unwrap().points;
        assert_eq!(points[0].logical, 1.5);
        assert_eq!(points[1].logical, 2.0);
    }

    #[test]
    fn v2_round_trip_preserves_general_panes_axes_data_and_series() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        let mut category_axis = crate::GeneralAxisOptions::new(
            "category-x",
            pane,
            crate::AxisDimension::X,
            crate::GeneralScaleType::Band,
        );
        category_axis.ticks = Some(vec![
            crate::GeneralAxisTick::Category {
                value: "Jan".into(),
                label: Some("January".into()),
            },
            crate::GeneralAxisTick::Category {
                value: "Feb".into(),
                label: None,
            },
        ]);
        chart.add_general_axis(category_axis).unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "category-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::Category {
                ids: Some(vec![crate::GeneralRowId::Text("jan".into())]),
                categories: vec!["Jan".into()],
                category_indices: vec![0],
                y: vec![42.0],
                y_valid: None,
            })
            .unwrap();
        chart
            .replace_general_xy_dataset_labeled(
                dataset,
                crate::GeneralXyInput::Category {
                    ids: Some(vec![crate::GeneralRowId::Text("jan".into())]),
                    categories: vec!["Jan".into()],
                    category_indices: vec![0],
                    y: vec![42.0],
                    y_valid: None,
                },
                Some(vec![Some("January".into())]),
            )
            .unwrap();
        let mut series =
            crate::GeneralSeriesOptions::column(pane, dataset, "category-x", "category-y");
        series.data_labels = true;
        series.group_id = Some("sales".into());
        series.stack_id = Some("share".into());
        series.stack_mode = crate::GeneralStackMode::Percent;
        chart.add_general_series(series).unwrap();
        let mut line =
            crate::GeneralSeriesOptions::xy_line(pane, dataset, "category-x", "category-y");
        line.title = "Category trend".into();
        line.point_markers = true;
        line.point_symbol = crate::GeneralPointSymbol::Diamond;
        line.point_radius = 7.0;
        line.interpolation = crate::GeneralInterpolation::Curved;
        line.connect_missing = true;
        chart.add_general_series(line).unwrap();
        let mut area =
            crate::GeneralSeriesOptions::xy_area(pane, dataset, "category-x", "category-y");
        area.title = "Category area".into();
        area.fill_opacity = 0.5;
        area.stack_id = Some("area-share".into());
        area.stack_mode = crate::GeneralStackMode::Percent;
        chart.add_general_series(area).unwrap();

        let document = chart.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["axes"][0]["ticks"][0]["label"], "January");
        assert!(value["axes"][0]["ticks"][1].get("label").is_none());
        assert_eq!(value["series"][0]["group_id"], "sales");
        assert_eq!(value["series"][0]["stack_id"], "share");
        assert_eq!(value["series"][0]["stack_mode"], "Percent");
        assert_eq!(value["series"][1]["point_markers"], true);
        assert_eq!(value["series"][1]["point_symbol"], "Diamond");
        assert_eq!(value["series"][1]["point_radius"], 7.0);
        assert_eq!(value["series"][1]["interpolation"], "Curved");
        assert_eq!(value["series"][1]["connect_missing"], true);
        assert_eq!(value["series"][2]["stack_id"], "area-share");
        assert_eq!(value["series"][2]["fill_opacity"], 0.5);
        assert_eq!(value["series"][2]["stack_mode"], "Percent");
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        let result = restored.import_state_json(&document).unwrap();
        assert_eq!(result.schema_version, 2);
        assert_eq!(restored.export_state_json().unwrap(), document);
    }

    #[test]
    fn v2_round_trip_preserves_horizontal_bar_axes_and_stack_options() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "bar-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "bar-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Band,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::Category {
                ids: Some(vec![crate::GeneralRowId::Text("a".into())]),
                categories: vec!["A".into()],
                category_indices: vec![0],
                y: vec![25.0],
                y_valid: None,
            })
            .unwrap();
        let mut series =
            crate::GeneralSeriesOptions::horizontal_bar(pane, dataset, "bar-x", "bar-y");
        series.group_id = Some("bars".into());
        series.stack_id = Some("share".into());
        series.stack_mode = crate::GeneralStackMode::Percent;
        chart.add_general_series(series).unwrap();

        let document = chart.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["series"][0]["kind"], "HorizontalBar");
        assert_eq!(value["series"][0]["group_id"], "bars");
        assert_eq!(value["series"][0]["stack_id"], "share");
        assert_eq!(value["series"][0]["stack_mode"], "Percent");
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        let result = restored.import_state_json(&document).unwrap();
        assert_eq!(result.schema_version, 2);
        assert_eq!(restored.export_state_json().unwrap(), document);
    }

    #[test]
    fn v2_round_trip_preserves_bubble_size_channel_and_missingness() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "bubble-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "bubble-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::Bubble {
                ids: Some(vec![
                    crate::GeneralRowId::Text("small".into()),
                    crate::GeneralRowId::Text("missing".into()),
                ]),
                x: vec![1.0, 2.0],
                y: vec![3.0, 4.0],
                y_valid: None,
                size: vec![25.0, 0.0],
                size_valid: Some(vec![1, 0]),
            })
            .unwrap();
        let series = crate::GeneralSeriesOptions::bubble(pane, dataset, "bubble-x", "bubble-y");
        chart.add_general_series(series).unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let restored_series = restored.general_series_ids_in_pane(pane)[0];
        assert_eq!(
            restored
                .general_tooltip_snapshot(restored_series, 0)
                .unwrap()
                .size,
            Some(25.0)
        );
        assert_eq!(
            restored
                .general_tooltip_snapshot(restored_series, 1)
                .unwrap()
                .size,
            None
        );
    }

    #[test]
    fn v2_round_trip_preserves_range_area_bounds_and_missingness() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "range-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "range-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::RangeNumeric {
                ids: Some(vec![
                    crate::GeneralRowId::Text("first".into()),
                    crate::GeneralRowId::Text("gap".into()),
                ]),
                x: vec![1.0, 2.0],
                low: vec![10.0, 0.0],
                low_valid: Some(vec![1, 0]),
                high: vec![20.0, 0.0],
                high_valid: Some(vec![1, 0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::range_area(
                pane, dataset, "range-x", "range-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let restored_series = restored.general_series_ids_in_pane(pane)[0];
        let first = restored
            .general_tooltip_snapshot(restored_series, 0)
            .unwrap();
        assert_eq!(first.low, Some(10.0));
        assert_eq!(first.high, Some(20.0));
        let gap = restored
            .general_tooltip_snapshot(restored_series, 1)
            .unwrap();
        assert_eq!(gap.low, None);
        assert_eq!(gap.high, None);
    }

    #[test]
    fn v2_round_trip_preserves_error_bar_bounds_and_missingness() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        for (id, dimension) in [
            ("error-x", crate::AxisDimension::X),
            ("error-y", crate::AxisDimension::Y),
        ] {
            chart
                .add_general_axis(crate::GeneralAxisOptions::new(
                    id,
                    pane,
                    dimension,
                    crate::GeneralScaleType::Linear,
                ))
                .unwrap();
        }
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::ErrorNumeric {
                ids: Some(vec![crate::GeneralRowId::Text("first".into())]),
                x: vec![10.0],
                y: vec![20.0],
                y_valid: None,
                x_low: vec![8.0],
                x_low_valid: None,
                x_high: vec![0.0],
                x_high_valid: Some(vec![0]),
                y_low: vec![15.0],
                y_low_valid: None,
                y_high: vec![0.0],
                y_high_valid: Some(vec![0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::error_bar(
                pane, dataset, "error-x", "error-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let series = restored.general_series_ids_in_pane(pane)[0];
        let snapshot = restored.general_tooltip_snapshot(series, 0).unwrap();
        assert_eq!(snapshot.value, Some(20.0));
        assert_eq!(snapshot.x_low, Some(8.0));
        assert_eq!(snapshot.x_high, None);
        assert_eq!(snapshot.low, Some(15.0));
        assert_eq!(snapshot.high, None);
    }

    #[test]
    fn v2_round_trip_preserves_category_error_bar_y_bounds() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "error-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Band,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "error-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::ErrorCategory {
                ids: Some(vec![crate::GeneralRowId::Text("quarter".into())]),
                categories: vec!["Q1".into()],
                category_indices: vec![0],
                y: vec![20.0],
                y_valid: None,
                y_low: vec![15.0],
                y_low_valid: None,
                y_high: vec![0.0],
                y_high_valid: Some(vec![0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::error_bar(
                pane, dataset, "error-x", "error-y",
            ))
            .unwrap();
        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let series = restored.general_series_ids_in_pane(pane)[0];
        let snapshot = restored.general_tooltip_snapshot(series, 0).unwrap();
        assert_eq!(snapshot.x_label, "Q1");
        assert_eq!(
            (snapshot.value, snapshot.low, snapshot.high),
            (Some(20.0), Some(15.0), None)
        );
    }

    #[test]
    fn v2_round_trip_preserves_category_box_plot_statistics_and_missingness() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "box-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Band,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "box-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::BoxCategory {
                ids: Some(vec![
                    crate::GeneralRowId::Text("full".into()),
                    crate::GeneralRowId::Text("missing".into()),
                ]),
                categories: vec!["A".into(), "B".into()],
                category_indices: vec![0, 1],
                min: vec![5.0, 10.0],
                min_valid: None,
                q1: vec![10.0, 15.0],
                q1_valid: None,
                median: vec![15.0, 20.0],
                median_valid: Some(vec![1, 0]),
                q3: vec![20.0, 25.0],
                q3_valid: None,
                max: vec![30.0, 35.0],
                max_valid: None,
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::box_plot(
                pane, dataset, "box-x", "box-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let series = restored.general_series_ids_in_pane(pane)[0];
        let first = restored.general_tooltip_snapshot(series, 0).unwrap();
        assert_eq!(first.x_label, "A");
        assert_eq!(
            (
                first.low,
                first.q1,
                first.value,
                first.q3,
                first.high,
                first.x_low,
                first.x_high,
            ),
            (
                Some(5.0),
                Some(10.0),
                Some(15.0),
                Some(20.0),
                Some(30.0),
                None,
                None,
            )
        );
        let missing = restored.general_tooltip_snapshot(series, 1).unwrap();
        assert_eq!(missing.value, None);
        assert_eq!(missing.q1, Some(15.0));
        assert_eq!(missing.q3, Some(25.0));
    }

    #[test]
    fn v2_round_trip_preserves_category_heatmap_axes_values_and_missingness() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Category {
                    scale: crate::CategoryScaleType::Band,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "heat-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Band,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "heat-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Band,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::HeatmapCategoryCategory {
                ids: Some(vec![
                    crate::GeneralRowId::Text("a".into()),
                    crate::GeneralRowId::Text("b".into()),
                ]),
                x_categories: vec!["Jan".into(), "Feb".into()],
                x_category_indices: vec![0, 1],
                y_categories: vec!["North".into(), "South".into()],
                y_category_indices: vec![0, 1],
                value: vec![10.0, 20.0],
                value_valid: Some(vec![1, 0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::heatmap_grid(
                pane, dataset, "heat-x", "heat-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let series = restored.general_series_ids_in_pane(pane)[0];
        let first = restored.general_tooltip_snapshot(series, 0).unwrap();
        assert_eq!(first.x_label, "Jan");
        assert_eq!(first.y_label.as_deref(), Some("North"));
        assert_eq!(first.value, Some(10.0));
        let missing = restored.general_tooltip_snapshot(series, 1).unwrap();
        assert_eq!(missing.x_label, "Feb");
        assert_eq!(missing.y_label.as_deref(), Some("South"));
        assert_eq!(missing.value, None);
    }

    #[test]
    fn v2_round_trip_preserves_general_reference_components() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "reference-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "reference-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let expected = vec![
            crate::GeneralReferenceOptions::Line {
                pane,
                axis_id: "reference-x".into(),
                value: crate::GeneralReferenceValue::Numeric(5.0),
                color: Some("#112233".into()),
                line_width: 2.0,
                extend_domain: true,
            },
            crate::GeneralReferenceOptions::Dot {
                pane,
                x_axis_id: "reference-x".into(),
                y_axis_id: "reference-y".into(),
                x: crate::GeneralReferenceValue::Numeric(2.0),
                y: crate::GeneralReferenceValue::Numeric(3.0),
                color: Some("#445566".into()),
                radius: 6.0,
                extend_domain: false,
            },
            crate::GeneralReferenceOptions::Region {
                pane,
                x_axis_id: "reference-x".into(),
                y_axis_id: "reference-y".into(),
                x_from: crate::GeneralReferenceValue::Numeric(1.0),
                x_to: crate::GeneralReferenceValue::Numeric(4.0),
                y_from: crate::GeneralReferenceValue::Numeric(2.0),
                y_to: crate::GeneralReferenceValue::Numeric(8.0),
                fill_color: Some("rgba(10,20,30,0.25)".into()),
                extend_domain: true,
            },
        ];
        for options in &expected {
            chart.add_general_reference(options.clone()).unwrap();
        }

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let restored_options = restored
            .general_reference_ids(Some(pane))
            .into_iter()
            .map(|id| restored.general_reference_options(id).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(restored_options, expected);
    }

    #[test]
    fn v2_round_trip_preserves_numeric_and_temporal_heatmap_coordinates() {
        let mut chart = ChartEngine::new(800.0, 600.0, 1.0);
        let numeric_pane = chart
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "numeric-heat-x",
                numeric_pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "numeric-heat-y",
                numeric_pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let numeric_data = chart
            .create_general_xy_dataset(crate::GeneralXyInput::HeatmapNumericNumeric {
                ids: Some(vec![
                    crate::GeneralRowId::Text("numeric-a".into()),
                    crate::GeneralRowId::Text("numeric-b".into()),
                ]),
                x: vec![1.0, 2.0],
                y_coordinate: vec![10.0, 20.0],
                value: vec![5.0, 6.0],
                value_valid: Some(vec![1, 0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::heatmap_grid(
                numeric_pane,
                numeric_data,
                "numeric-heat-x",
                "numeric-heat-y",
            ))
            .unwrap();

        let temporal_pane = chart
            .add_pane_with_domain(true, crate::HorizontalDomain::Temporal)
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "temporal-heat-x",
                temporal_pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Temporal,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "temporal-heat-y",
                temporal_pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let temporal_data = chart
            .create_general_xy_dataset(crate::GeneralXyInput::HeatmapTemporalNumeric {
                ids: Some(vec![
                    crate::GeneralRowId::Text("temporal-a".into()),
                    crate::GeneralRowId::Text("temporal-b".into()),
                ]),
                x_epoch_ms: vec![1_700_000_000_000, 1_700_000_060_000],
                y_coordinate: vec![15.0, 25.0],
                value: vec![50.0, 60.0],
                value_valid: None,
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::heatmap_grid(
                temporal_pane,
                temporal_data,
                "temporal-heat-x",
                "temporal-heat-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 600.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);

        let numeric_series = restored.general_series_ids_in_pane(numeric_pane)[0];
        let numeric_first = restored
            .general_tooltip_snapshot(numeric_series, 0)
            .unwrap();
        assert_eq!(numeric_first.x_label, "1");
        assert_eq!(numeric_first.y_label.as_deref(), Some("10"));
        assert_eq!(numeric_first.value, Some(5.0));
        assert_eq!(
            restored
                .general_tooltip_snapshot(numeric_series, 1)
                .unwrap()
                .value,
            None
        );

        let temporal_series = restored.general_series_ids_in_pane(temporal_pane)[0];
        let temporal_second = restored
            .general_tooltip_snapshot(temporal_series, 1)
            .unwrap();
        assert_eq!(temporal_second.x_label, "1700000060000");
        assert_eq!(temporal_second.y_label.as_deref(), Some("25"));
        assert_eq!(temporal_second.value, Some(60.0));
    }

    #[test]
    fn v2_round_trip_preserves_temporal_error_bar_xy_bounds() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = chart
            .add_pane_with_domain(true, crate::HorizontalDomain::Temporal)
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "error-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Temporal,
            ))
            .unwrap();
        chart
            .add_general_axis(crate::GeneralAxisOptions::new(
                "error-y",
                pane,
                crate::AxisDimension::Y,
                crate::GeneralScaleType::Linear,
            ))
            .unwrap();
        let dataset = chart
            .create_general_xy_dataset(crate::GeneralXyInput::ErrorTemporal {
                ids: Some(vec![crate::GeneralRowId::Text("observation".into())]),
                x_epoch_ms: vec![1_700_000_000_000],
                y: vec![20.0],
                y_valid: None,
                x_low_epoch_ms: vec![1_699_999_970_000.0],
                x_low_valid: None,
                x_high_epoch_ms: vec![1_700_000_030_000.0],
                x_high_valid: None,
                y_low: vec![15.0],
                y_low_valid: None,
                y_high: vec![0.0],
                y_high_valid: Some(vec![0]),
            })
            .unwrap();
        chart
            .add_general_series(crate::GeneralSeriesOptions::error_bar(
                pane, dataset, "error-x", "error-y",
            ))
            .unwrap();

        let document = chart.export_state_json().unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let series = restored.general_series_ids_in_pane(pane)[0];
        let snapshot = restored.general_tooltip_snapshot(series, 0).unwrap();
        assert_eq!(snapshot.x_label, "1700000000000");
        assert_eq!(snapshot.x_low, Some(1_699_999_970_000.0));
        assert_eq!(snapshot.x_high, Some(1_700_000_030_000.0));
        assert_eq!(
            (snapshot.value, snapshot.low, snapshot.high),
            (Some(20.0), Some(15.0), None)
        );
    }

    #[test]
    fn v2_invalid_series_reference_leaves_target_unchanged() {
        let mut source = ChartEngine::new(800.0, 500.0, 1.0);
        source
            .add_pane_with_domain(
                true,
                crate::HorizontalDomain::Continuous {
                    scale: crate::ContinuousScaleType::Linear,
                },
            )
            .unwrap();
        let mut document: serde_json::Value =
            serde_json::from_str(&source.export_state_json().unwrap()).unwrap();
        document["series"] = serde_json::json!([{
            "kind": "Scatter",
            "pane": 1,
            "dataset": "dataset-999",
            "x_axis_id": "x",
            "y_axis_id": "y",
            "visible": true,
            "title": "",
            "color": null,
            "point_radius": 3.0,
            "data_labels": false
        }]);
        let mut target = ChartEngine::new(800.0, 500.0, 1.0);
        let before = target.export_state_json().unwrap();
        assert!(target.import_state_json(&document.to_string()).is_err());
        assert_eq!(target.export_state_json().unwrap(), before);
    }

    #[test]
    fn v2_import_rejects_a_chart_after_general_axis_handles_were_issued() {
        let mut source = ChartEngine::new(800.0, 500.0, 1.0);
        source
            .add_pane_with_domain(true, crate::HorizontalDomain::Temporal)
            .unwrap();
        let document = source.export_state_json().unwrap();

        let mut target = ChartEngine::new(800.0, 500.0, 1.0);
        let pane = target
            .add_pane_with_domain(true, crate::HorizontalDomain::Temporal)
            .unwrap();
        target
            .add_general_axis(crate::GeneralAxisOptions::new(
                "stale-x",
                pane,
                crate::AxisDimension::X,
                crate::GeneralScaleType::Temporal,
            ))
            .unwrap();
        assert!(target.remove_general_axis("stale-x"));
        assert!(target.remove_pane(pane));
        let before = target.export_state_json().unwrap();

        let error = target.import_state_json(&document).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedOperation);
        assert_eq!(target.export_state_json().unwrap(), before);
    }

    #[test]
    fn unknown_optional_v1_fields_are_ignored_deterministically() {
        let mut document: serde_json::Value = serde_json::from_str(VALID).unwrap();
        document["future_metadata"] = serde_json::json!({ "safe_to_ignore": true });
        document["panes"][0]["future_pane_option"] = serde_json::json!(42);
        document["drawings"][0]["style"]["future_style_option"] = serde_json::json!("x");

        let mut baseline = ChartEngine::new(800.0, 500.0, 1.0);
        baseline.import_state_json(VALID).unwrap();
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document.to_string()).unwrap();
        assert_eq!(restored.export_state_json(), baseline.export_state_json());
    }

    #[test]
    fn all_ten_kinds_and_multi_pane_associations_restore() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let restored = chart.import_state_json(ALL_DRAWINGS).unwrap();
        assert_eq!(restored.panes, 2);
        assert_eq!(restored.drawings, 10);
        assert_eq!(
            chart
                .drawings
                .iter()
                .map(|drawing| drawing.kind)
                .collect::<Vec<_>>(),
            [
                DrawingKind::TrendLine,
                DrawingKind::HorizontalLine,
                DrawingKind::HorizontalRay,
                DrawingKind::VerticalLine,
                DrawingKind::Rectangle,
                DrawingKind::Text,
                DrawingKind::Brush,
                DrawingKind::Path,
                DrawingKind::LongPosition,
                DrawingKind::ShortPosition,
            ]
        );
        assert!(
            chart.drawings[..3]
                .iter()
                .all(|drawing| drawing.pane_index == 0)
        );
        assert!(
            chart.drawings[3..]
                .iter()
                .all(|drawing| drawing.pane_index == 1)
        );
        assert_eq!(chart.panes[0].persistent_id(), Some(3));
        assert_eq!(chart.panes[1].persistent_id(), Some(9));
    }

    #[test]
    fn restore_issues_fresh_live_pane_ids_and_stales_old_handles() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let old = chart.pane_stable_id(0).unwrap();
        chart.import_state_json(ALL_DRAWINGS).unwrap();
        assert_eq!(chart.pane_index_for_id(old), None);
        assert_ne!(chart.pane_stable_id(0), Some(old));
    }

    #[test]
    fn malformed_unknown_and_semantically_invalid_documents_are_atomic() {
        for (document, code) in [
            (MALFORMED, ErrorCode::SerializationError),
            (r#"{}"#, ErrorCode::SerializationError),
            (
                r#"{"schema":"aeris_charts-state","schema_version":"1","panes":[],"drawings":[]}"#,
                ErrorCode::SerializationError,
            ),
            (UNKNOWN_VERSION, ErrorCode::PersistenceVersionError),
            (UNKNOWN_KIND, ErrorCode::InvalidData),
            (
                r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"text","pane_id":"pane-2","anchors":[{"logical":1,"price":1}]}]}"#,
                ErrorCode::InvalidData,
            ),
            (
                r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"},{"id":"pane-1"}],"drawings":[]}"#,
                ErrorCode::InvalidData,
            ),
            (
                r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"text","pane_id":"pane-1","anchors":[{"logical":1,"price":1}]},{"id":1,"kind":"text","pane_id":"pane-1","anchors":[{"logical":2,"price":2}]}]}"#,
                ErrorCode::InvalidData,
            ),
            (
                r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"text","pane_id":"pane-1","anchors":[{"logical":1e999,"price":1}]}]}"#,
                ErrorCode::SerializationError,
            ),
            (
                r#"{"schema":"aeris_charts-state","schema_version":1,"panes":[{"id":"pane-1"}],"drawings":[{"id":1,"kind":"rectangle","pane_id":"pane-1","anchors":[{"logical":1,"price":1}]}]}"#,
                ErrorCode::InvalidData,
            ),
        ] {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let before = chart.export_state_json().unwrap();
            let error = chart.import_state_json(document).unwrap_err();
            assert_eq!(error.code(), code);
            assert_eq!(chart.export_state_json().unwrap(), before);
        }
    }

    #[test]
    fn document_and_anchor_resource_limits_fail_before_install() {
        let too_large = " ".repeat(PERSISTENCE_MAX_DOCUMENT_BYTES + 1);
        assert_eq!(
            ChartEngine::validate_state_json(&too_large)
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        assert_eq!(
            chart.import_state_json(&too_large).unwrap_err().code(),
            ErrorCode::ResourceLimit,
            "V2 capacity must not weaken the legacy oversized-document contract"
        );
        let anchors = (0..=PERSISTENCE_MAX_POINTS_PER_DRAWING)
            .map(|index| serde_json::json!({ "logical": index, "price": 1 }))
            .collect::<Vec<_>>();
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{ "id": "pane-1" }],
            "drawings": [{ "id": 1, "kind": "brush", "pane_id": "pane-1", "anchors": anchors }]
        })
        .to_string();
        assert_eq!(
            ChartEngine::validate_state_json(&document)
                .unwrap_err()
                .code(),
            ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn import_is_fresh_chart_only_and_drawing_ids_are_not_reused() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let issued = chart
            .add_drawing(
                DrawingKind::Text,
                0,
                vec![DrawingPoint {
                    logical: 1.0,
                    price: 1.0,
                }],
                None,
            )
            .unwrap();
        assert!(chart.remove_drawing(issued));
        let error = chart.import_state_json(VALID).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedOperation);

        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(VALID).unwrap();
        assert!(restored.remove_drawing(12));
        let next = restored
            .add_drawing(
                DrawingKind::Text,
                0,
                vec![DrawingPoint {
                    logical: 1.0,
                    price: 1.0,
                }],
                None,
            )
            .unwrap();
        assert_eq!(next, 13);
    }

    #[test]
    fn one_thousand_drawing_restore_rebuilds_m6_runtime_once_and_remains_candidate_bounded() {
        let mut source = settled_chart();
        for index in 0..1_000 {
            let logical = if index < 10 {
                2.0 + index as f64 * 0.4
            } else {
                1_000.0 + index as f64 * 10.0
            };
            source
                .add_drawing(
                    DrawingKind::TrendLine,
                    0,
                    vec![
                        DrawingPoint {
                            logical,
                            price: 10.5,
                        },
                        DrawingPoint {
                            logical: logical + 0.25,
                            price: 11.0,
                        },
                    ],
                    None,
                )
                .unwrap();
        }
        let document = source.export_state_json().unwrap();
        drop(source);

        let mut restored = settled_chart();
        let result = restored.import_state_json(&document).unwrap();
        assert_eq!(result.drawings, 1_000);
        restored.reset_drawing_work_stats();
        restored.build_frame();
        let work = restored.drawing_work_stats();
        assert_eq!(work.drawings_total, 1_000);
        assert_eq!(work.candidates, 10);
        assert_eq!(work.visible, 10);
        assert_eq!(work.geometry_rebuilds, 10);
    }

    #[test]
    fn study_persistence_round_trips_convention_parameters_kdj_and_amount_source() {
        let mut chart = settled_chart();
        let volume = chart.add_series(crate::SeriesKind::Histogram);
        let amount = chart.add_series(crate::SeriesKind::Line);
        let times = (0..10).map(|i| (i * 3600) as f64).collect::<Vec<_>>();
        let values = [11.0, 12.0, 11.0, 10.0, 11.0, 12.0, 13.0, 12.0, 11.0, 10.0];
        let amounts = values.map(|value| value * 11.5);
        chart
            .set_series_data(volume, &times, &values, &values, &values, &values)
            .unwrap();
        chart
            .set_series_data(amount, &times, &amounts, &amounts, &amounts, &amounts)
            .unwrap();
        let china = crate::IndicatorConvention::China;
        let kinds = [
            crate::IndicatorKind::Ema {
                period: 3,
                seed: crate::IndicatorSeed::Sma,
            }
            .with_convention(china),
            crate::IndicatorKind::Rsi {
                period: 3,
                seed: crate::IndicatorSeed::Sma,
            }
            .with_convention(china),
            crate::IndicatorKind::Macd {
                fast: 2,
                slow: 4,
                signal: 3,
                seed: crate::IndicatorSeed::Sma,
                histogram_multiplier: 1.0,
            }
            .with_convention(china),
            crate::IndicatorKind::Bollinger {
                period: 3,
                deviation: 2.0,
                estimator: crate::DeviationEstimator::Population,
            }
            .with_convention(china),
            crate::IndicatorKind::Kdj {
                period: 3,
                k_smoothing: 3,
                d_smoothing: 3,
                seed: aeris_charts_indicators::KdjSeed::Fifty,
            }
            .with_convention(china),
        ];
        for kind in &kinds {
            assert!(!chart.add_indicator_kind(0, kind.clone(), None).is_empty());
        }
        chart.add_vwap_with_amount(0, volume, amount).unwrap();
        let document = chart.export_state_json().unwrap();
        // Explicit parameters are persisted; the preset name never is.
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["indicators"][2]["kind"]["seed"], "first_value");
        assert_eq!(value["indicators"][2]["kind"]["histogram_multiplier"], 2.0);
        assert_eq!(value["indicators"][3]["kind"]["estimator"], "sample");
        assert_eq!(value["indicators"][4]["kind"]["seed"], "first_value");
        assert!(!document.contains("china"));

        let mut restored = settled_chart();
        let restored_volume = restored.add_series(crate::SeriesKind::Histogram);
        let restored_amount = restored.add_series(crate::SeriesKind::Line);
        restored
            .set_series_data(restored_volume, &times, &values, &values, &values, &values)
            .unwrap();
        restored
            .set_series_data(
                restored_amount,
                &times,
                &amounts,
                &amounts,
                &amounts,
                &amounts,
            )
            .unwrap();
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.export_state_json().unwrap(), document);
        let bindings = restored.indicator_bindings();
        for (binding, kind) in bindings.iter().zip(&kinds) {
            assert_eq!(&binding.kind, kind);
        }
        assert_eq!(bindings[5].volume_source, Some(restored_volume));
        assert_eq!(bindings[5].amount_source, Some(restored_amount));
        assert_eq!(
            restored.data.series_data(bindings[5].outputs[0]).unwrap().1[3],
            chart
                .data
                .series_data(chart.indicator_bindings()[5].outputs[0])
                .unwrap()
                .1[3]
        );

        // An amount source on a kind that cannot use it is rejected before mutation.
        let mut invalid: serde_json::Value = serde_json::from_str(&document).unwrap();
        invalid["indicators"][0]["amount_source"] =
            invalid["indicators"][5]["amount_source"].clone();
        let invalid = serde_json::to_string(&invalid).unwrap();
        let mut untouched = settled_chart();
        for series in [crate::SeriesKind::Histogram, crate::SeriesKind::Line] {
            let id = untouched.add_series(series);
            untouched
                .set_series_data(id, &times, &values, &values, &values, &values)
                .unwrap();
        }
        let baseline = untouched.export_state_json().unwrap();
        let error = untouched.import_state_json(&invalid).unwrap_err();
        assert!(error.message().contains("amount source"), "{error:?}");
        assert_eq!(untouched.export_state_json().unwrap(), baseline);
        assert!(untouched.import_state_json(&document).is_ok());
    }

    #[test]
    fn study_documents_without_convention_fields_restore_the_tradingview_defaults() {
        let mut chart = settled_chart();
        chart.add_macd(0, 2, 4, 3);
        chart.add_bollinger(0, 3, 2.0);
        chart.add_rsi(0, 3);
        chart.add_indicator_kind(
            0,
            crate::IndicatorKind::Kdj {
                period: 3,
                k_smoothing: 3,
                d_smoothing: 3,
                seed: aeris_charts_indicators::KdjSeed::FirstValue,
            },
            None,
        );
        let mut legacy: serde_json::Value =
            serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
        // Documents written before these parameters existed carry none of them.
        for indicator in legacy["indicators"].as_array_mut().unwrap() {
            let kind = indicator["kind"].as_object_mut().unwrap();
            for field in ["seed", "histogram_multiplier", "estimator"] {
                kind.remove(field);
            }
        }
        let mut restored = settled_chart();
        restored
            .import_state_json(&serde_json::to_string(&legacy).unwrap())
            .unwrap();
        let kinds = restored
            .indicator_bindings()
            .into_iter()
            .map(|binding| binding.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                crate::IndicatorKind::Macd {
                    fast: 2,
                    slow: 4,
                    signal: 3,
                    seed: crate::IndicatorSeed::Sma,
                    histogram_multiplier: 1.0,
                },
                crate::IndicatorKind::Bollinger {
                    period: 3,
                    deviation: 2.0,
                    estimator: crate::DeviationEstimator::Population,
                },
                crate::IndicatorKind::Rsi {
                    period: 3,
                    seed: crate::IndicatorSeed::Sma,
                },
                crate::IndicatorKind::Kdj {
                    period: 3,
                    k_smoothing: 3,
                    d_smoothing: 3,
                    seed: aeris_charts_indicators::KdjSeed::Fifty,
                },
            ]
        );
    }

    /// The `(logical, price)` anchors of drawing `id`.
    fn anchor_pairs(chart: &ChartEngine, id: u32) -> Vec<(f64, f64)> {
        chart
            .drawing(id)
            .unwrap()
            .points
            .iter()
            .map(|point| (point.logical, point.price))
            .collect()
    }

    fn assert_anchors(chart: &ChartEngine, id: u32, expected: &[(f64, f64)]) {
        let actual = anchor_pairs(chart, id);
        assert_eq!(actual.len(), expected.len(), "drawing {id}: {actual:?}");
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual.0 - expected.0).abs() < 1e-9 && (actual.1 - expected.1).abs() < 1e-9,
                "drawing {id}: {actual:?} != {expected:?}"
            );
        }
    }

    /// The written anchor times of drawing `id` in an exported document.
    fn exported_times(document: &serde_json::Value, id: u32) -> Vec<Option<f64>> {
        document["drawings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|drawing| drawing["id"] == id)
            .unwrap()["anchors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|anchor| anchor["time"].as_f64())
            .collect()
    }

    #[test]
    fn fork_written_documents_convert_to_the_upstream_catalog_on_load() {
        const T: f64 = 1_700_000_000.0;
        let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price});
        let timed = |logical: f64, price: f64| {
            let mut anchor = at(logical, price);
            anchor["time"] = serde_json::json!(T + logical * 60.0);
            anchor
        };
        let drawing = |id: u32, kind: &str, anchors: Vec<serde_json::Value>| {
            let mut drawing = serde_json::json!({"id": id, "kind": kind, "pane_id": "pane-1"});
            drawing["anchors"] = serde_json::Value::Array(anchors);
            drawing
        };
        let styled =
            |id: u32, kind: &str, anchors: Vec<serde_json::Value>, style: serde_json::Value| {
                let mut drawing = drawing(id, kind, anchors);
                drawing["style"] = style;
                drawing
            };
        // A document the fork wrote: no catalog marker, fork tool names and anchor contracts,
        // anchor times, and `tool_options` blocks, with values equal to the fork's defaults left
        // out.
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{"id": "pane-1"}],
            "drawings": [
                styled(1, "fib_retracement", vec![at(1.0, 10.0), at(6.0, 12.0)],
                    serde_json::json!({"tool_options": {"fibonacci": {
                        "reverse": true, "levels_as_percent": true
                    }}})),
                drawing(2, "flat_top_bottom", vec![at(1.0, 10.0), at(5.0, 12.0), at(3.0, 13.0)]),
                drawing(3, "flat_top_bottom", vec![at(1.0, 10.0), at(5.0, 12.0), at(3.0, 9.0)]),
                drawing(4, "disjoint_channel",
                    vec![timed(2.0, 10.0), timed(6.0, 12.0), at(4.0, 8.0)]),
                styled(5, "gann_square_fixed", vec![at(3.0, 10.0)],
                    serde_json::json!({"tool_options": {"gann": {
                        "size_bars": 10.0, "scale_ratio": 0.5, "reverse": true
                    }}})),
                styled(6, "bars_pattern", vec![at(10.0, 20.0), at(14.0, 30.0)],
                    serde_json::json!({"tool_options": {"projection_annotation": {
                        "bars_mode": "oc_bars",
                        "mirrored": true,
                        "bars": [
                            [5.0, 6.0, 4.0, 5.5],
                            [1.0, 0.5, 2.0, 1.0],
                            [5.5, 8.0, 5.0, 7.0],
                            [7.0, 7.5, 2.0, 3.0]
                        ]
                    }}})),
                drawing(7, "projection", vec![at(20.0, 10.0), at(25.0, 12.0), at(22.0, 14.0)]),
                drawing(8, "price_note", vec![at(5.0, 11.0), at(8.0, 13.0)]),
                drawing(9, "signpost", vec![timed(7.0, 12.0)]),
                drawing(10, "triangle_pattern",
                    vec![at(0.0, 10.0), at(2.0, 14.0), at(4.0, 11.0), at(6.0, 13.0)]),
                drawing(11, "three_drives_pattern", (0..7)
                    .map(|index| at(f64::from(index), 10.0 + f64::from(index % 2)))
                    .collect()),
                drawing(12, "arc", vec![timed(1.0, 10.0), timed(5.0, 10.0), timed(3.0, 12.0)]),
                drawing(13, "curve", vec![at(0.0, 10.0), at(4.0, 10.0), at(2.0, 12.0)]),
                drawing(14, "double_curve",
                    vec![timed(0.0, 10.0), timed(3.0, 10.0), timed(1.0, 11.0), timed(2.0, 11.0)]),
                drawing(15, "rotated_rectangle", vec![at(0.0, 10.0), at(4.0, 12.0), at(2.0, 14.0)]),
                styled(16, "anchored_text", vec![timed(0.25, 0.75)],
                    serde_json::json!({"text": "pinned"})),
                styled(17, "icon", vec![at(9.0, 11.0)],
                    serde_json::json!({"tool_options": {"projection_annotation": {
                        "icon": "heart", "icon_size": 120.0
                    }}})),
                drawing(18, "elliott_impulse_wave", (0..6)
                    .map(|index| at(f64::from(index), 10.0 + f64::from(index % 2)))
                    .collect()),
            ]
        })
        .to_string();

        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let restored = chart.import_state_json(&document).unwrap();
        assert_eq!(restored.drawings, 18);
        assert_eq!(
            chart
                .drawings
                .iter()
                .map(|drawing| drawing.kind)
                .collect::<Vec<_>>(),
            [
                DrawingKind::FibonacciRetracement,
                DrawingKind::FlatTopChannel,
                DrawingKind::FlatBottomChannel,
                DrawingKind::DisjointChannel,
                DrawingKind::GannSquareFixed,
                DrawingKind::BarsPattern,
                DrawingKind::Projection,
                DrawingKind::PriceNote,
                DrawingKind::Signpost,
                DrawingKind::PatternTriangle,
                DrawingKind::PatternThreeDrives,
                DrawingKind::Arc,
                DrawingKind::Curve,
                DrawingKind::DoubleCurve,
                DrawingKind::RotatedRectangle,
                DrawingKind::AnchoredText,
                DrawingKind::IconStamp,
                DrawingKind::ElliottImpulse,
            ]
        );

        // Omitted values keep the fork's defaults; fork option keys map onto the flat fields.
        let fibonacci = chart.drawing(1).unwrap();
        assert_eq!(fibonacci.levels.len(), crate::FIBONACCI_RATIOS.len());
        assert_eq!(fibonacci.color, "#787b86");
        assert_eq!(fibonacci.style, LineStyle::Dashed);
        // The fork's reversed retracement put level 0 on its first anchor: upstream's
        // unreversed one.
        assert!(!fibonacci.level_reverse);
        assert!(fibonacci.level_show_percents);
        assert!(!fibonacci.level_show_values);
        assert_eq!(fibonacci.level_label_align, "left");
        assert_eq!(chart.drawing(18).unwrap().wave_degree, "intermediate");
        let icon = chart.drawing(17).unwrap();
        assert_eq!(icon.icon_name.as_deref(), Some("heart"));
        assert_eq!(icon.icon_size, 96.0);
        // The fixed square's `reverse` lives in its downward corner: the fans and arcs keep
        // pivoting on the anchor, as the fork's did.
        assert!(!chart.drawing(5).unwrap().level_reverse);
        let bars = chart.drawing(6).unwrap();
        assert_eq!(
            bars.bars_pattern
                .iter()
                .map(|bar| bar.offset)
                .collect::<Vec<_>>(),
            [0, 2, 3],
            "invalid bars drop out and the rest keep their positions"
        );
        assert_eq!(bars.bars_pattern_mode, "oc_bars");
        assert!(bars.bars_pattern_mirror_x);
        let anchored = chart.drawing(16).unwrap();
        assert_eq!((anchored.screen_x, anchored.screen_y), (0.25, 0.75));

        // Anchors follow the upstream contracts.
        assert_anchors(
            &chart,
            4,
            &[(2.0, 10.0), (6.0, 12.0), (2.0, 9.0), (6.0, 7.0)],
        );
        assert_anchors(&chart, 5, &[(3.0, 10.0), (13.0, 5.0)]);
        assert_anchors(
            &chart,
            6,
            &[(10.0, 20.0), (14.0, 30.0), (10.0, 20.0 + 3.5 * 10.0 / 6.0)],
        );
        assert_anchors(&chart, 7, &[(20.0, 10.0), (22.0, 14.0)]);
        assert_anchors(&chart, 8, &[(5.0, 11.0)]);
        assert_anchors(&chart, 9, &[(7.0, 12.0), (7.0, 12.0)]);
        assert_anchors(
            &chart,
            10,
            &[
                (0.0, 10.0),
                (2.0, 14.0),
                (4.0, 11.0),
                (6.0, 13.0),
                (8.0, 12.0),
            ],
        );
        assert_eq!(anchor_pairs(&chart, 11).len(), 6);
        assert_anchors(&chart, 12, &[(1.0, 10.0), (3.0, 12.0), (5.0, 10.0)]);
        assert_anchors(&chart, 13, &[(0.0, 10.0), (2.0, 14.0), (4.0, 10.0)]);
        assert_anchors(
            &chart,
            14,
            &[(0.0, 10.0), (1.0, 11.5), (2.0, 11.5), (3.0, 10.0)],
        );
        assert_anchors(&chart, 15, &[(0.0, 13.0), (4.0, 15.0), (0.0, 7.0)]);

        // The export writes upstream names and the catalog marker; derived anchors keep the time
        // identity of the anchor they sit beside.
        let exported = chart.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(value["drawing_catalog"], DRAWING_CATALOG_REVISION);
        assert_eq!(
            value["drawings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|drawing| drawing["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "fibonacci_retracement",
                "flat_top_channel",
                "flat_bottom_channel",
                "disjoint_channel",
                "gann_square_fixed",
                "bars_pattern",
                "projection",
                "price_note",
                "signpost",
                "pattern_triangle",
                "pattern_three_drives",
                "arc",
                "curve",
                "double_curve",
                "rotated_rectangle",
                "anchored_text",
                "icon_stamp",
                "elliott_impulse",
            ]
        );
        let time = |logical: f64| Some(T + logical * 60.0);
        assert_eq!(
            exported_times(&value, 4),
            [time(2.0), time(6.0), time(2.0), time(6.0)]
        );
        assert_eq!(exported_times(&value, 9), [time(7.0), time(7.0)]);
        assert_eq!(
            exported_times(&value, 12),
            [time(1.0), time(3.0), time(5.0)]
        );
        let controls = exported_times(&value, 14);
        assert_eq!(controls[0], time(0.0));
        assert!(
            (controls[1].unwrap() - (T + 60.0)).abs() < 1e-3,
            "{controls:?}"
        );
        assert!(
            (controls[2].unwrap() - (T + 120.0)).abs() < 1e-3,
            "{controls:?}"
        );
        assert_eq!(controls[3], time(3.0));
        assert_eq!(exported_times(&value, 16), [None]);
        assert!(!exported.contains("\"levels_as_percent\""));
        assert!(!exported.contains("\"bars_mode\""));
        // The projection, the price note, and the signpost carry the fork-form marker, written
        // as an empty block.
        for id in [7, 8, 9] {
            assert_eq!(
                written_tool_options(&value, id),
                serde_json::json!({"projection_annotation": {}}),
                "drawing {id}"
            );
        }

        // The written document is upstream-format: it restores unchanged.
        let mut again = ChartEngine::new(800.0, 500.0, 1.0);
        again.import_state_json(&exported).unwrap();
        assert_eq!(again.export_state_json().unwrap(), exported);
    }

    #[test]
    fn fork_fibonacci_and_sine_drawings_keep_the_geometry_the_fork_drew() {
        const T: f64 = 1_700_000_000.0;
        let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price});
        let timed = |logical: f64, price: f64| {
            let mut anchor = at(logical, price);
            anchor["time"] = serde_json::json!(T + logical * 60.0);
            anchor
        };
        let drawing = |id: u32, kind: &str, anchors: Vec<serde_json::Value>| serde_json::json!({"id": id, "kind": kind, "pane_id": "pane-1", "anchors": anchors});
        let reversed = |id: u32, kind: &str, anchors: Vec<serde_json::Value>| {
            let mut drawing = drawing(id, kind, anchors);
            drawing["style"] =
                serde_json::json!({"tool_options": {"fibonacci": {"reverse": true}}});
            drawing
        };
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{"id": "pane-1"}],
            "drawings": [
                drawing(1, "fib_retracement", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                drawing(2, "fib_speed_resistance_fan", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                reversed(3, "fib_speed_resistance_fan", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                reversed(4, "trend_based_fib_extension",
                    vec![at(1.0, 10.0), at(6.0, 12.0), at(8.0, 11.0)]),
                reversed(5, "fib_time_zone", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                reversed(6, "fib_spiral", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                reversed(7, "fib_circles", vec![at(1.0, 10.0), at(6.0, 12.0)]),
                drawing(8, "fib_speed_resistance_arcs", vec![timed(1.0, 10.0), timed(6.0, 12.0)]),
                drawing(9, "fib_circles", vec![timed(2.0, 10.0), timed(6.0, 14.0)]),
                drawing(10, "sine_line", vec![timed(2.0, 14.0), timed(6.0, 10.0)]),
            ]
        })
        .to_string();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.import_state_json(&document).unwrap();
        let reverse = |id: u32| chart.drawing(id).unwrap().level_reverse;
        // The fork's level 0 sat on the retracement's and the fan's second anchor: upstream's
        // reversed placement, and the fork's `reverse` is upstream's unreversed one.
        assert!(reverse(1));
        assert!(reverse(2));
        assert!(!reverse(3));
        // The extension and the time zones reversed the way upstream does.
        assert!(reverse(4));
        assert!(reverse(5));
        // The spiral's counterclockwise turn and a `reverse` the circles never read map to
        // nothing; the key stays stored.
        assert!(!reverse(6));
        assert!(!reverse(7));
        let exported: serde_json::Value =
            serde_json::from_str(&chart.export_state_json().unwrap()).unwrap();
        let tool_options = |id: u32| {
            exported["drawings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|drawing| drawing["id"] == id)
                .unwrap()["style"]["tool_options"]
                .clone()
        };
        assert_eq!(tool_options(6)["fibonacci"]["reverse"], true);
        // The fork's unstored trend line and label placement are written out, the flat aliases
        // are not.
        assert_eq!(
            tool_options(1),
            serde_json::json!({"fibonacci": {
                "trend_line": true, "grid": false, "full_circles": false, "label_v_align": "middle"
            }})
        );
        // Speed arcs turned around the first anchor: upstream's center is the second.
        assert_anchors(&chart, 8, &[(6.0, 12.0), (1.0, 10.0)]);
        // Circles centered between the anchors at half their distance: upstream centers them on
        // the second anchor at the full distance.
        assert_anchors(&chart, 9, &[(2.0, 10.0), (4.0, 12.0)]);
        // A sine through two opposite extremes: upstream's zero crossing and extreme.
        assert_anchors(&chart, 10, &[(4.0, 12.0), (6.0, 10.0)]);
        let time = |logical: f64| Some(T + logical * 60.0);
        assert_eq!(exported_times(&exported, 8), [time(6.0), time(1.0)]);
        assert_eq!(exported_times(&exported, 9), [time(2.0), time(4.0)]);
        assert_eq!(exported_times(&exported, 10), [time(4.0), time(6.0)]);
    }

    #[test]
    fn fork_documents_from_sequence_axes_are_recognized_by_missing_upstream_fields() {
        // On a trade-count, volume, or range-bar axis the fork wrote no anchor time; with default
        // options it wrote no `tool_options` either, and it used upstream's names for these
        // tools. Upstream writes the level flags of every leveled tool, which the fork never did.
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{"id": "pane-1"}],
            "drawings": [
                {"id": 1, "kind": "andrews_pitchfork", "pane_id": "pane-1", "anchors": [
                    {"logical": 0.0, "price": 10.0},
                    {"logical": 4.0, "price": 14.0},
                    {"logical": 6.0, "price": 9.0}
                ]},
                {"id": 2, "kind": "arc", "pane_id": "pane-1", "anchors": [
                    {"logical": 1.0, "price": 10.0},
                    {"logical": 5.0, "price": 10.0},
                    {"logical": 3.0, "price": 12.0}
                ]}
            ]
        })
        .to_string();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.import_state_json(&document).unwrap();
        let mut fork = Drawing::new(0, DrawingKind::AndrewsPitchfork, 0, Vec::new());
        crate::drawings::kinds::apply_legacy_fork_defaults(&mut fork);
        let upstream = Drawing::new(0, DrawingKind::AndrewsPitchfork, 0, Vec::new());
        assert_ne!(
            fork.levels, upstream.levels,
            "the fixture tells the defaults apart"
        );
        // The fork's pitchfork levels, in upstream's meaning.
        crate::drawings::kinds::pitchforks_gann::legacy_levels_to_upstream(&mut fork);
        assert_eq!(chart.drawing(1).unwrap().levels, fork.levels);
        assert_anchors(&chart, 2, &[(1.0, 10.0), (3.0, 12.0), (5.0, 10.0)]);

        // The same pitchfork from an upstream pin carries its level flags and loads unchanged.
        let mut upstream_document: serde_json::Value = serde_json::from_str(&document).unwrap();
        upstream_document["drawings"][0]["style"] = serde_json::json!({"level_reverse": false});
        let mut pinned = ChartEngine::new(800.0, 500.0, 1.0);
        pinned
            .import_state_json(&upstream_document.to_string())
            .unwrap();
        assert_eq!(pinned.drawing(1).unwrap().levels, upstream.levels);
        assert_anchors(&pinned, 2, &[(1.0, 10.0), (5.0, 10.0), (3.0, 12.0)]);
    }

    /// A document as the fork wrote it (no catalog marker) holding `drawings`.
    fn fork_document(drawings: Vec<serde_json::Value>) -> String {
        serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{"id": "pane-1"}],
            "drawings": drawings,
        })
        .to_string()
    }

    /// One stored drawing; `style` `null` leaves the style out.
    fn stored(
        id: u32,
        kind: &str,
        anchors: &[serde_json::Value],
        style: serde_json::Value,
    ) -> serde_json::Value {
        let mut drawing = serde_json::json!({
            "id": id, "kind": kind, "pane_id": "pane-1", "anchors": anchors,
        });
        if !style.is_null() {
            drawing["style"] = style;
        }
        drawing
    }

    /// An anchor with the time the fork wrote (on hourly bars from the epoch), which marks the
    /// document as the fork's.
    fn timed(logical: f64, price: f64) -> serde_json::Value {
        serde_json::json!({"logical": logical, "price": price, "time": logical * 3_600.0})
    }

    fn untimed(logical: f64, price: f64) -> serde_json::Value {
        serde_json::json!({"logical": logical, "price": price})
    }

    /// The `tool_options` an export wrote for drawing `id`.
    fn written_tool_options(document: &serde_json::Value, id: u32) -> serde_json::Value {
        document["drawings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|drawing| drawing["id"] == id)
            .unwrap()["style"]["tool_options"]
            .clone()
    }

    /// Restore `document`, then check that its export restores to the same export.
    fn restore_round_trip(document: &str) -> (ChartEngine, serde_json::Value) {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.import_state_json(document).unwrap();
        let exported = chart.export_state_json().unwrap();
        let mut again = ChartEngine::new(800.0, 500.0, 1.0);
        again.import_state_json(&exported).unwrap();
        assert_eq!(again.export_state_json().unwrap(), exported);
        for drawing in &chart.drawings {
            assert_eq!(
                again.drawing(drawing.id),
                Some(drawing),
                "{:?}",
                drawing.kind
            );
        }
        (chart, serde_json::from_str(&exported).unwrap())
    }

    #[test]
    fn fork_flat_channels_keep_their_level_line() {
        let base = [timed(1.0, 10.0), timed(5.0, 12.0)];
        let flat = |id: u32, level: serde_json::Value| {
            stored(
                id,
                "flat_top_bottom",
                &[base[0].clone(), base[1].clone(), level],
                serde_json::Value::Null,
            )
        };
        let document = fork_document(vec![
            // A level between the base anchors crosses the base: the disjoint channel whose
            // second line is that level.
            flat(1, untimed(3.0, 11.5)),
            // Below both base anchors, whatever its bar: a flat bottom at its price.
            flat(2, untimed(-3.0, 9.5)),
            flat(3, untimed(3.0, 12.0)),
            flat(4, untimed(3.0, 10.0)),
            // A base on one bar keeps its side of the base.
            stored(
                5,
                "flat_top_bottom",
                &[timed(2.0, 10.0), timed(2.0, 12.0), untimed(2.0, 11.0)],
                serde_json::Value::Null,
            ),
        ]);
        let (chart, exported) = restore_round_trip(&document);
        let kinds = (1..=5)
            .map(|id| chart.drawing(id).unwrap().kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                DrawingKind::DisjointChannel,
                DrawingKind::FlatBottomChannel,
                DrawingKind::FlatTopChannel,
                DrawingKind::FlatBottomChannel,
                DrawingKind::FlatBottomChannel,
            ]
        );
        assert_anchors(
            &chart,
            1,
            &[(1.0, 10.0), (5.0, 12.0), (1.0, 11.5), (5.0, 11.5)],
        );
        assert!(chart.drawing(1).unwrap().fill_enabled);
        // Each end of the level keeps the time identity of the base anchor on its bar.
        let time = |logical: f64| Some(logical * 3_600.0);
        assert_eq!(
            exported_times(&exported, 1),
            [time(1.0), time(5.0), time(1.0), time(5.0)]
        );
        assert_anchors(&chart, 2, &[(1.0, 10.0), (5.0, 12.0), (-3.0, 9.5)]);
        assert_anchors(&chart, 5, &[(2.0, 10.0), (2.0, 12.0), (2.0, 11.0)]);
    }

    #[test]
    fn fork_channels_keep_their_unstored_defaults_and_regression_sides() {
        let two = [timed(1.0, 10.0), timed(6.0, 12.0)];
        let document = fork_document(vec![
            stored(
                1,
                "regression_trend",
                &two,
                serde_json::json!({"tool_options": {"channel": {
                    "upper_deviation": 3.0, "lower_deviation": -1.0
                }}}),
            ),
            stored(
                2,
                "regression_trend",
                &two,
                serde_json::json!({"tool_options": {"channel": {"use_lower_deviation": false}}}),
            ),
            stored(3, "regression_trend", &two, serde_json::Value::Null),
            stored(
                4,
                "parallel_channel",
                &[two[0].clone(), two[1].clone(), untimed(3.0, 13.0)],
                serde_json::Value::Null,
            ),
            stored(
                5,
                "parallel_channel",
                &[two[0].clone(), two[1].clone(), untimed(3.0, 13.0)],
                serde_json::json!({"tool_options": {"channel": {"middle_line": false}}}),
            ),
            stored(
                6,
                "disjoint_channel",
                &[
                    two[0].clone(),
                    two[1].clone(),
                    untimed(1.0, 8.0),
                    untimed(6.0, 9.0),
                ],
                serde_json::Value::Null,
            ),
        ]);
        let (mut chart, exported) = restore_round_trip(&document);
        let channel =
            |chart: &ChartEngine, id: u32| chart.drawing(id).unwrap().tool_options.channel.clone();
        let options = |value: serde_json::Value| {
            serde_json::from_value::<crate::ChannelToolOptions>(value).unwrap()
        };
        // Asymmetric and one-sided bands keep their sides (an omitted side at the fork's
        // default); the flat band is the fold, for upstream's readers.
        assert_eq!(chart.drawing(1).unwrap().regression_deviations, 3.0);
        assert_eq!(
            channel(&chart, 1),
            Some(options(serde_json::json!({
                "middle_line": true, "show_pearsons": true,
                "upper_deviation": 3.0, "lower_deviation": -1.0
            })))
        );
        assert_eq!(chart.drawing(2).unwrap().regression_deviations, 2.0);
        assert_eq!(
            channel(&chart, 2),
            Some(options(serde_json::json!({
                "middle_line": true, "show_pearsons": true, "use_lower_deviation": false,
                "upper_deviation": 2.0, "lower_deviation": -2.0
            })))
        );
        // The fork's on-by-default centre line and Pearson's R, and the parallel channel's
        // middle line, which its documents never stored; a stored value wins.
        assert_eq!(
            channel(&chart, 3),
            Some(options(
                serde_json::json!({"middle_line": true, "show_pearsons": true})
            ))
        );
        assert_eq!(
            channel(&chart, 4),
            Some(options(serde_json::json!({"middle_line": true})))
        );
        assert_eq!(
            channel(&chart, 5),
            Some(options(serde_json::json!({"middle_line": false})))
        );
        assert_eq!(channel(&chart, 6), None);
        assert_eq!(
            written_tool_options(&exported, 1),
            serde_json::json!({"channel": {
                "middle_line": true, "upper_deviation": 3.0, "lower_deviation": -1.0,
                "show_pearsons": true
            }})
        );
        // A side override is a patch of its own side: the flat band stays.
        assert!(
            chart.drawing_apply_options(
                3,
                r#"{"tool_options":{"channel":{"upper_deviation":1.0}}}"#
            )
        );
        let regression = chart.drawing(3).unwrap();
        assert_eq!(regression.regression_deviations, 2.0);
        assert_eq!(
            regression
                .tool_options
                .channel
                .as_ref()
                .unwrap()
                .upper_deviation,
            Some(1.0)
        );
        // Upstream documents keep upstream's defaults.
        let mut marked: serde_json::Value = exported.clone();
        marked["drawings"][3]["style"]
            .as_object_mut()
            .unwrap()
            .remove("tool_options");
        let mut upstream = ChartEngine::new(800.0, 500.0, 1.0);
        upstream.import_state_json(&marked.to_string()).unwrap();
        assert_eq!(channel(&upstream, 4), None);
    }

    #[test]
    fn fork_fibonacci_documents_keep_their_trend_lines_grid_and_label_placement() {
        let two = [untimed(1.0, 10.0), untimed(6.0, 12.0)];
        let three = [untimed(1.0, 10.0), untimed(6.0, 12.0), untimed(8.0, 11.0)];
        let fibonacci = |id: u32, kind: &str, anchors: &[serde_json::Value]| {
            stored(id, kind, anchors, serde_json::Value::Null)
        };
        let document = fork_document(vec![
            fibonacci(1, "fib_retracement", &two),
            fibonacci(2, "trend_based_fib_extension", &three),
            fibonacci(3, "fib_channel", &three),
            fibonacci(4, "fib_time_zone", &two),
            fibonacci(5, "trend_based_fib_time", &three),
            fibonacci(6, "fib_speed_resistance_fan", &two),
            fibonacci(7, "fib_speed_resistance_arcs", &two),
            fibonacci(8, "fib_circles", &two),
            fibonacci(9, "fib_spiral", &two),
            fibonacci(10, "fib_wedge", &three),
            // A stored block (the fork wrote whole blocks) keeps its values.
            stored(
                11,
                "fib_retracement",
                &two,
                serde_json::json!({"tool_options": {"fibonacci": {
                    "trend_line": false, "label_v_align": "top", "reverse": true
                }}}),
            ),
            stored(
                12,
                "fib_time_zone",
                &two,
                serde_json::json!({"tool_options": {"fibonacci": {"label_h_align": "left"}}}),
            ),
        ]);
        let (chart, _) = restore_round_trip(&document);
        let block = |id: u32| chart.drawing(id).unwrap().tool_options.fibonacci;
        let v_align = |align| Some(align);
        for (id, trend_line, grid, label_v_align) in [
            (1, true, false, v_align(crate::FibonacciLabelVAlign::Middle)),
            (2, true, false, v_align(crate::FibonacciLabelVAlign::Middle)),
            (
                3,
                false,
                false,
                v_align(crate::FibonacciLabelVAlign::Middle),
            ),
            (4, true, false, v_align(crate::FibonacciLabelVAlign::Bottom)),
            (5, true, false, v_align(crate::FibonacciLabelVAlign::Bottom)),
            (6, false, true, None),
            (7, true, false, None),
            (8, true, false, None),
            (9, true, false, None),
            (11, false, false, v_align(crate::FibonacciLabelVAlign::Top)),
        ] {
            let block = block(id).unwrap_or_else(|| panic!("drawing {id} has its block"));
            assert_eq!(
                (block.trend_line, block.grid, block.label_v_align),
                (trend_line, grid, label_v_align),
                "drawing {id}"
            );
        }
        // The wedge has no unstored option value, but its block selects the fork's precise
        // rings (R5, owner decision T1), as every fork ring tool's does.
        assert_eq!(
            block(10),
            Some(crate::FibonacciToolOptions::default()),
            "the wedge carries the empty block"
        );
        // The fork labelled time levels right of their lines: upstream's `left`. An explicit
        // fork `left` is upstream's `right`.
        assert_eq!(chart.drawing(4).unwrap().level_label_align, "left");
        assert_eq!(chart.drawing(5).unwrap().level_label_align, "left");
        assert_eq!(chart.drawing(12).unwrap().level_label_align, "right");
        assert_eq!(chart.drawing(1).unwrap().level_label_align, "left");
        // New drawings keep upstream's look: the option type's defaults switch nothing on.
        let defaults = crate::FibonacciToolOptions::default();
        assert!(!defaults.trend_line && !defaults.grid);
    }

    #[test]
    fn fork_time_zone_labels_stay_right_of_their_lines() {
        let document = fork_document(vec![stored(
            1,
            "fib_time_zone",
            &[untimed(1.0, 11.0), untimed(2.0, 12.0)],
            serde_json::Value::Null,
        )]);
        let mut chart = settled_chart();
        chart.import_state_json(&document).unwrap();
        let frame = chart.build_frame();
        let pane = &frame.panes[0];
        let lines = pane
            .main
            .iter()
            .filter_map(|prim| match prim {
                aeris_charts_render::draw_list::Prim::VLine { x, .. } => Some(f64::from(*x)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let labels = pane
            .main
            .iter()
            .filter_map(|prim| match prim {
                aeris_charts_render::draw_list::Prim::Text { x, text, align, .. }
                    if text == "0" || text == "1" =>
                {
                    Some((f64::from(*x), *align))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!labels.is_empty(), "the zones are labelled");
        for (x, align) in labels {
            assert_eq!(align, aeris_charts_render::draw_list::TextAlign::Left);
            assert!(
                lines.iter().any(|line| x > *line && x - *line <= 6.0),
                "label at {x} runs right from a line of {lines:?}"
            );
        }
    }

    #[test]
    fn fork_pitchforks_and_gann_tools_keep_the_geometry_the_fork_drew() {
        let three = [timed(8.0, 101.0), timed(16.0, 105.0), timed(20.0, 102.0)];
        let two = [timed(3.0, 10.0), timed(7.0, 12.0)];
        let document = fork_document(vec![
            stored(1, "andrews_pitchfork", &three, serde_json::Value::Null),
            stored(
                2,
                "pitchfan",
                &three,
                serde_json::json!({"line_style": "dashed", "levels": [
                    {"value": 0.5, "color": "#ff0000", "visible": true, "style": "solid",
                     "fill_between": true, "fill_color": "#ff000040", "label_visible": true},
                    {"value": 1.0, "color": "#00ff00", "visible": true, "style": "solid",
                     "fill_between": true, "label_visible": false},
                    {"value": 0.75, "color": "#0000ff", "visible": false, "style": "solid",
                     "fill_between": true, "label_visible": false},
                    {"value": 0.0, "color": "#000000", "visible": true, "style": "solid",
                     "fill_between": false, "label_visible": false}
                ]}),
            ),
            stored(3, "gann_box", &two, serde_json::Value::Null),
            stored(4, "gann_square", &two, serde_json::Value::Null),
            stored(
                5,
                "gann_square",
                &two,
                serde_json::json!({"tool_options": {"gann": {"reverse": true}}}),
            ),
            stored(
                6,
                "gann_fan",
                &two,
                serde_json::json!({"tool_options": {"gann": {"reverse": true, "scale_ratio": 0.5}}}),
            ),
            stored(
                7,
                "gann_square_fixed",
                &[timed(3.0, 10.0)],
                serde_json::json!({"tool_options": {"gann": {"size_bars": 20.0, "reverse": true}}}),
            ),
            stored(
                8,
                "gann_square_fixed",
                &[timed(3.0, 10.0)],
                serde_json::json!({"tool_options": {"gann": {"size_bars": 2.0}}}),
            ),
        ]);
        let (chart, _) = restore_round_trip(&document);
        let level = |value: f64, color: &str, visible: bool| crate::DrawingLevel {
            value,
            color: color.to_string(),
            visible,
            style: "solid".to_string(),
            fill_between: true,
            fill_color: None,
            label_visible: false,
        };
        // The fork's defaults: tines at 0.5 and 1 half-handle each side of an always-drawn
        // median. Upstream places the tines along the handle (0.25/0.75 and 0/1), the visible
        // ones first; each lower band keeps the fill of the fork level outside it.
        let andrews = chart.drawing(1).unwrap();
        let visible = andrews
            .levels
            .iter()
            .filter(|level| level.visible)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            [
                crate::DrawingLevel {
                    fill_between: false,
                    ..level(0.0, "#2962ff", true)
                },
                crate::DrawingLevel {
                    fill_color: Some("#2962ff23".to_string()),
                    ..level(0.25, "#089981", true)
                },
                crate::DrawingLevel {
                    fill_color: Some("#08998123".to_string()),
                    ..level(0.5, "", true)
                },
                level(0.75, "#089981", true),
                level(1.0, "#2962ff", true),
            ]
        );
        assert_eq!(andrews.levels.len(), 19);
        assert!(andrews.levels[5..].iter().all(|level| !level.visible));
        assert!(
            andrews.levels[5..]
                .windows(2)
                .all(|pair| pair[0].value <= pair[1].value)
        );
        // Stored levels convert the same way; the median takes the drawing's line style, a
        // zero level (never drawn) drops out, and a stored fill colour moves with its band.
        let fan = chart.drawing(2).unwrap();
        let values = fan
            .levels
            .iter()
            .map(|level| (level.value, level.visible))
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            [
                (0.0, true),
                (0.25, true),
                (0.5, true),
                (0.75, true),
                (1.0, true),
                (0.125, false),
                (0.875, false)
            ]
        );
        assert_eq!(fan.levels[1].fill_color.as_deref(), Some("#00ff0023"));
        assert_eq!(fan.levels[2].fill_color.as_deref(), Some("#ff000040"));
        assert_eq!(fan.levels[2].style, "dashed");
        assert_eq!(fan.levels[3].fill_color.as_deref(), Some("#ff000040"));
        // The box's own time levels and the squares' stats box.
        let gann = |id: u32| {
            chart
                .drawing(id)
                .unwrap()
                .tool_options
                .gann
                .clone()
                .unwrap()
        };
        assert_eq!(gann(3).time_levels.len(), 7);
        assert!(!gann(3).show_stats);
        assert!(gann(4).show_stats);
        assert!(gann(4).time_levels.is_empty());
        // A reversed square measured from the first anchor's time and the second's price:
        // upstream's pivot sits on the first anchor once the prices swap.
        assert_anchors(&chart, 5, &[(3.0, 12.0), (7.0, 10.0)]);
        assert!(!chart.drawing(5).unwrap().level_reverse);
        assert!(!chart.drawing(4).unwrap().level_reverse);
        // The fork's fan never read `reverse`; its ratio stays.
        assert!(!chart.drawing(6).unwrap().level_reverse);
        assert_eq!(gann(6).scale_ratio, Some(0.5));
        assert!(!gann(6).reverse);
        // A downward fixed square without a ratio keeps a positive corner below its anchor
        // (one a logarithmic scale can place); an upward one keeps the additive offset.
        assert!(!chart.drawing(7).unwrap().level_reverse);
        assert_anchors(&chart, 7, &[(3.0, 10.0), (23.0, 10.0 / 21.0)]);
        assert_anchors(&chart, 8, &[(3.0, 10.0), (5.0, 30.0)]);
        assert!(gann(7).show_stats);
    }

    #[test]
    fn fork_annotations_restore_in_their_fork_form() {
        use crate::drawings::{
            DrawingBodyGeometry, DrawingGeometryOptions, resolve_drawing_geometry,
        };
        let document = fork_document(vec![
            // The fork's three-anchor projection (pivot, radius point, price point).
            stored(
                1,
                "projection",
                &[timed(1.0, 10.0), timed(3.0, 10.0), timed(4.0, 12.0)],
                serde_json::Value::Null,
            ),
            // The fork's one-anchor signpost.
            stored(2, "signpost", &[timed(2.0, 11.0)], serde_json::Value::Null),
            stored(3, "note", &[timed(2.0, 11.0)], serde_json::Value::Null),
            stored(
                4,
                "price_label",
                &[timed(2.0, 11.0)],
                serde_json::Value::Null,
            ),
        ]);
        let (chart, exported) = restore_round_trip(&document);
        fn body<'a>(chart: &ChartEngine, id: u32, px: &'a [(f64, f64)]) -> DrawingBodyGeometry<'a> {
            let drawing = chart.drawing(id).unwrap();
            resolve_drawing_geometry(
                drawing.kind,
                px,
                800.0,
                0.0,
                500.0,
                DrawingGeometryOptions::for_drawing(drawing, 1.0),
            )
            .unwrap()
            .body
        }
        // The projection resolves its sector, the note its pin, the price label its tail.
        assert!(matches!(
            body(&chart, 1, &[(100.0, 300.0), (200.0, 200.0)]),
            DrawingBodyGeometry::Sector(_)
        ));
        assert!(matches!(
            body(&chart, 3, &[(100.0, 300.0)]),
            DrawingBodyGeometry::NotePin(_)
        ));
        assert!(matches!(
            body(&chart, 4, &[(100.0, 300.0)]),
            DrawingBodyGeometry::SpeechTail { .. }
        ));
        // The signpost's anchors coincide, so it stands the fork's 40 CSS px pole.
        let points = &chart.drawing(2).unwrap().points;
        assert_eq!(points[0], points[1]);
        let DrawingBodyGeometry::Marker(marker) =
            body(&chart, 2, &[(100.0, 300.0), (100.0, 300.0)])
        else {
            panic!("signpost marker");
        };
        assert_eq!(marker.anchor, (100.0, 260.0));
        // Each keeps its marker, written as an empty block.
        for id in 1..=4 {
            assert_eq!(
                written_tool_options(&exported, id),
                serde_json::json!({"projection_annotation": {}}),
                "{id}"
            );
        }
    }

    #[test]
    fn fork_lines_annotations_patterns_and_shapes_keep_their_fork_options() {
        let two = [timed(1.0, 10.0), timed(4.0, 12.0)];
        let document = fork_document(vec![
            stored(1, "ray", &two, serde_json::Value::Null),
            stored(2, "info_line", &two, serde_json::Value::Null),
            stored(3, "trend_angle", &two, serde_json::Value::Null),
            stored(
                4,
                "arrow_line",
                &two,
                serde_json::json!({"tool_options": {"line": {"stats_position": "middle"}}}),
            ),
            stored(5, "trend_line", &two, serde_json::Value::Null),
            stored(6, "note", &[timed(2.0, 11.0)], serde_json::Value::Null),
            stored(7, "comment", &[timed(2.0, 11.0)], serde_json::Value::Null),
            stored(
                8,
                "callout",
                &two,
                serde_json::json!({"box_color": "#ff000033"}),
            ),
            stored(
                9,
                "price_label",
                &[timed(2.0, 11.0)],
                serde_json::Value::Null,
            ),
            stored(10, "forecast", &two, serde_json::Value::Null),
            stored(
                11,
                "arrow_mark_left",
                &[timed(2.0, 11.0)],
                serde_json::Value::Null,
            ),
            stored(
                12,
                "note",
                &[timed(2.0, 11.0)],
                serde_json::json!({"tool_options": {"projection_annotation": {"always_show_text": true}}}),
            ),
            stored(
                13,
                "anchored_text",
                &[timed(2.0, 11.0)],
                serde_json::json!({"screen_x": 0.5, "screen_y": 0.5}),
            ),
            stored(
                14,
                "triangle_pattern",
                &[
                    timed(0.0, 10.0),
                    timed(2.0, 14.0),
                    timed(4.0, 11.0),
                    timed(6.0, 13.0),
                ],
                serde_json::Value::Null,
            ),
            // The fork's curve passes through its third anchor halfway along; upstream's middle
            // control point takes a time identity where the stored times are evenly spaced.
            stored(
                15,
                "curve",
                &[timed(0.0, 10.0), timed(4.0, 10.0), timed(2.0, 12.0)],
                serde_json::Value::Null,
            ),
            // A rotated rectangle around a vertical axis.
            stored(
                16,
                "rotated_rectangle",
                &[timed(2.0, 10.0), timed(2.0, 14.0), timed(5.0, 12.0)],
                serde_json::Value::Null,
            ),
        ]);
        let (chart, exported) = restore_round_trip(&document);
        let drawing = |id: u32| chart.drawing(id).unwrap();
        // The line tools draw their stats as one box: the block selects it.
        for id in 1..=3 {
            assert_eq!(
                drawing(id).tool_options.line,
                Some(Default::default()),
                "{id}"
            );
        }
        assert_eq!(
            drawing(4).tool_options.line.unwrap().stats_position,
            crate::DrawingStatsPosition::Middle
        );
        assert_eq!(drawing(5).tool_options, Default::default());
        assert_eq!(
            written_tool_options(&exported, 1),
            serde_json::json!({"line": {"stats_position": "end"}})
        );
        // The annotations' fork-form marker, written as an empty block; a stored block keeps
        // its keys.
        for id in [6, 7, 9, 10, 11] {
            assert_eq!(
                drawing(id).tool_options.projection_annotation,
                Some(Default::default()),
                "{id}"
            );
            assert_eq!(
                written_tool_options(&exported, id),
                serde_json::json!({"projection_annotation": {}}),
                "{id}"
            );
        }
        assert!(
            drawing(12)
                .tool_options
                .projection_annotation
                .as_ref()
                .unwrap()
                .always_show_text
        );
        assert_eq!(drawing(8).tool_options, Default::default());
        assert_eq!(drawing(13).tool_options, Default::default());
        // The fork's boxes had no fill or border unless set, and an export writes a set one.
        assert_eq!(drawing(6).box_color, None);
        assert_eq!(drawing(6).box_border_color, None);
        assert_eq!(drawing(8).box_color.as_deref(), Some("#ff000033"));
        assert_eq!(drawing(8).box_border_color, None);
        // The triangle pattern's apex sides.
        assert!(drawing(14).extend_left && drawing(14).extend_right);
        assert_anchors(&chart, 15, &[(0.0, 10.0), (2.0, 14.0), (4.0, 10.0)]);
        assert_eq!(
            exported_times(&exported, 15),
            [Some(0.0), Some(2.0 * 3_600.0), Some(4.0 * 3_600.0)]
        );
        assert_anchors(&chart, 16, &[(5.0, 10.0), (5.0, 14.0), (-1.0, 10.0)]);
        assert_eq!(
            exported_times(&exported, 16),
            [Some(5.0 * 3_600.0), Some(5.0 * 3_600.0), None]
        );
    }

    #[test]
    fn fork_derived_anchors_keep_their_bars_across_a_session_gap() {
        // Hourly bars with a weekend after the fourth: a curve from L0 through L3 to L6, a double
        // curve over L0..L6, and a rotated rectangle around a vertical axis at L2 straddle it.
        let times = (0..10)
            .map(|i| f64::from(if i < 4 { i } else { i + 48 }) * 3_600.0)
            .collect::<Vec<_>>();
        let gapped = |logical: f64, price: f64| {
            let time = times[logical as usize];
            serde_json::json!({"logical": logical, "price": price, "time": time})
        };
        let document = fork_document(vec![
            stored(
                1,
                "curve",
                &[gapped(0.0, 10.0), gapped(6.0, 10.0), gapped(3.0, 12.0)],
                serde_json::Value::Null,
            ),
            stored(
                2,
                "double_curve",
                &[
                    gapped(0.0, 10.0),
                    gapped(6.0, 10.0),
                    gapped(2.0, 11.0),
                    gapped(4.0, 11.0),
                ],
                serde_json::Value::Null,
            ),
            stored(
                3,
                "rotated_rectangle",
                &[gapped(2.0, 10.0), gapped(2.0, 14.0), gapped(5.0, 12.0)],
                serde_json::Value::Null,
            ),
        ]);
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let values = [11.0; 10];
        chart
            .set_series_data(0, &times, &values, &values, &values, &values)
            .unwrap();
        chart.import_state_json(&document).unwrap();
        // The derived points keep the bars the fork's geometry put them on (time arithmetic
        // across the gap would move them about two days of bars left).
        assert_anchors(&chart, 1, &[(0.0, 10.0), (3.0, 14.0), (6.0, 10.0)]);
        assert_anchors(
            &chart,
            2,
            &[(0.0, 10.0), (2.0, 11.5), (4.0, 11.5), (6.0, 10.0)],
        );
        assert_anchors(&chart, 3, &[(5.0, 10.0), (5.0, 14.0), (-1.0, 10.0)]);
    }

    #[test]
    fn cleared_annotation_boxes_survive_a_round_trip() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        let points = vec![DrawingPoint {
            logical: 2.0,
            price: 11.0,
        }];
        let note = chart
            .add_drawing(
                DrawingKind::Note,
                0,
                points.clone(),
                Some(r#"{"box_color":"","box_border_color":""}"#),
            )
            .unwrap();
        let tinted = chart
            .add_drawing(DrawingKind::Comment, 0, points, None)
            .unwrap();
        assert_eq!(chart.drawing(note).unwrap().box_color, None);
        let exported = chart.export_state_json().unwrap();
        let mut again = ChartEngine::new(800.0, 500.0, 1.0);
        again.import_state_json(&exported).unwrap();
        assert_eq!(again.drawing(note).unwrap().box_color, None);
        assert_eq!(again.drawing(note).unwrap().box_border_color, None);
        assert_eq!(
            again.drawing(tinted).unwrap().box_color,
            chart.drawing(tinted).unwrap().box_color
        );
        assert_eq!(again.export_state_json().unwrap(), exported);
    }

    /// Shapes whose anchor count is the same in both catalogs but whose anchors meant something
    /// else to the fork.
    fn same_count_shapes() -> Vec<serde_json::Value> {
        let at = |logical: f64, price: f64| serde_json::json!({"logical": logical, "price": price});
        [
            ("rotated_rectangle", [(0.0, 10.0), (4.0, 12.0), (2.0, 14.0)]),
            ("arc", [(1.0, 10.0), (5.0, 10.0), (3.0, 12.0)]),
            ("curve", [(0.0, 10.0), (4.0, 10.0), (2.0, 12.0)]),
        ]
        .into_iter()
        .zip(1u32..)
        .map(|((kind, anchors), id)| {
            serde_json::json!({
                "id": id,
                "kind": kind,
                "pane_id": "pane-1",
                "anchors": anchors
                    .iter()
                    .map(|&(logical, price)| at(logical, price))
                    .collect::<Vec<_>>(),
            })
        })
        .collect()
    }

    #[test]
    fn documents_with_the_catalog_marker_are_never_converted() {
        let mut drawings = same_count_shapes();
        // An anchor time alone marks a fork document only without the catalog marker.
        drawings[0]["anchors"][0]["time"] = serde_json::json!(1_700_000_000.0);
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "drawing_catalog": DRAWING_CATALOG_REVISION,
            "panes": [{"id": "pane-1"}],
            "drawings": drawings,
        })
        .to_string();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.import_state_json(&document).unwrap();
        assert_anchors(&chart, 1, &[(0.0, 10.0), (4.0, 12.0), (2.0, 14.0)]);
        assert_anchors(&chart, 2, &[(1.0, 10.0), (5.0, 10.0), (3.0, 12.0)]);
        assert_anchors(&chart, 3, &[(0.0, 10.0), (4.0, 10.0), (2.0, 12.0)]);
    }

    #[test]
    fn upstream_pin_documents_load_unconverted() {
        // An upstream pin writes neither the catalog marker nor anything only the fork wrote.
        let mut drawings = same_count_shapes();
        drawings.push(serde_json::json!({
            "id": 4,
            "kind": "anchored_text",
            "pane_id": "pane-1",
            "anchors": [{"logical": 12.0, "price": 30.0}],
            "style": {"text": "pinned", "screen_x": 0.2, "screen_y": 0.4}
        }));
        drawings.push(serde_json::json!({
            "id": 5,
            "kind": "fibonacci_retracement",
            "pane_id": "pane-1",
            "anchors": [{"logical": 1.0, "price": 10.0}, {"logical": 6.0, "price": 12.0}],
            "style": {"level_reverse": false, "level_label_align": "right"}
        }));
        let document = serde_json::json!({
            "schema": "aeris_charts-state",
            "schema_version": 1,
            "panes": [{"id": "pane-1"}],
            "drawings": drawings,
        })
        .to_string();
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.import_state_json(&document).unwrap();
        assert_anchors(&chart, 1, &[(0.0, 10.0), (4.0, 12.0), (2.0, 14.0)]);
        assert_anchors(&chart, 2, &[(1.0, 10.0), (5.0, 10.0), (3.0, 12.0)]);
        assert_anchors(&chart, 3, &[(0.0, 10.0), (4.0, 10.0), (2.0, 12.0)]);
        let anchored = chart.drawing(4).unwrap();
        assert_eq!((anchored.screen_x, anchored.screen_y), (0.2, 0.4));
        assert_anchors(&chart, 4, &[(12.0, 30.0)]);
        // Omitted values take upstream's defaults, not the fork's.
        let fibonacci = chart.drawing(5).unwrap();
        let defaults = Drawing::new(0, DrawingKind::FibonacciRetracement, 0, Vec::new());
        assert_eq!(fibonacci.levels, defaults.levels);
        assert_eq!(fibonacci.color, defaults.color);
        assert_eq!(fibonacci.level_label_align, "right");
    }

    #[test]
    fn emptied_label_and_level_lists_survive_a_round_trip() {
        let mut chart = settled_chart();
        let points = vec![
            DrawingPoint {
                logical: 1.0,
                price: 10.0,
            },
            DrawingPoint {
                logical: 6.0,
                price: 12.0,
            },
        ];
        let info = chart
            .add_drawing(DrawingKind::InfoLine, 0, points.clone(), None)
            .unwrap();
        let fibonacci = chart
            .add_drawing(DrawingKind::FibonacciRetracement, 0, points, None)
            .unwrap();
        assert!(!chart.drawing(info).unwrap().labels.is_empty());
        assert!(!chart.drawing(fibonacci).unwrap().levels.is_empty());
        assert!(chart.drawing_apply_options(info, r#"{"labels":[]}"#));
        assert!(chart.drawing_apply_options(fibonacci, r#"{"levels":[]}"#));
        let document = chart.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(
            value["drawings"][0]["style"]["labels"],
            serde_json::json!([])
        );
        assert_eq!(
            value["drawings"][1]["style"]["levels"],
            serde_json::json!([])
        );

        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert!(restored.drawing(info).unwrap().labels.is_empty());
        assert!(restored.drawing(fibonacci).unwrap().levels.is_empty());
        assert_eq!(restored.export_state_json().unwrap(), document);
    }
    #[test]
    fn hidden_mark_groups_round_trip_v1_before_marks_exist() {
        let mut source = ChartEngine::new(800.0, 500.0, 1.0);
        source.set_timeline_group_hidden("news", true).unwrap();
        source.set_timeline_group_hidden("dividends", true).unwrap();
        let document = source.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(
            value["hidden_mark_groups"],
            serde_json::json!(["dividends", "news"])
        );
        // A chart without hidden groups writes no key (byte-stable default documents).
        assert!(
            !ChartEngine::new(800.0, 500.0, 1.0)
                .export_state_json()
                .unwrap()
                .contains("hidden_mark_groups")
        );

        // Hosts import first and set marks later: the hidden set is live before any mark.
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.hidden_timeline_groups(), ["dividends", "news"]);
        assert!(restored.timeline_marks().marks.is_empty());
        restored
            .set_timeline_marks(crate::TimelineMarksSnapshot {
                marks: vec![crate::TimelineMark {
                    id: "n".into(),
                    time: 3_600,
                    group: "news".into(),
                    glyph: crate::TimelineMarkGlyph::default(),
                    title: String::new(),
                }],
                groups: Vec::new(),
            })
            .unwrap();
        assert_eq!(restored.hidden_timeline_groups(), ["dividends", "news"]);
        assert_eq!(restored.export_state_json().unwrap(), document);
    }

    #[test]
    fn hidden_mark_groups_round_trip_v2_and_v3_documents() {
        // V2: a general pane makes the chart export schema 2 through the staged importer.
        let mut general = ChartEngine::new(800.0, 500.0, 1.0);
        general
            .add_pane_with_domain(true, crate::HorizontalDomain::Temporal)
            .unwrap();
        general.set_timeline_group_hidden("news", true).unwrap();
        let document = general.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["hidden_mark_groups"], serde_json::json!(["news"]));
        let mut restored = ChartEngine::new(800.0, 500.0, 1.0);
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.hidden_timeline_groups(), ["news"]);
        assert_eq!(restored.export_state_json().unwrap(), document);

        // V3: an engine indicator makes the chart export schema 3.
        let mut studies = settled_chart();
        studies.add_indicator_kind(0, crate::IndicatorKind::Sma { period: 2 }, None);
        studies.set_timeline_group_hidden("earnings", true).unwrap();
        let document = studies.export_state_json().unwrap();
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        assert_eq!(value["schema_version"], 3);
        assert_eq!(value["hidden_mark_groups"], serde_json::json!(["earnings"]));
        let mut restored = settled_chart();
        restored.import_state_json(&document).unwrap();
        assert_eq!(restored.hidden_timeline_groups(), ["earnings"]);
        assert_eq!(restored.export_state_json().unwrap(), document);
    }

    #[test]
    fn invalid_hidden_mark_groups_fail_structurally_and_atomically() {
        let too_many: Vec<String> = (0..=crate::MAX_TIMELINE_GROUPS)
            .map(|i| format!("g{i}"))
            .collect();
        let too_long = "x".repeat(129);
        for (groups, code) in [
            (serde_json::json!(too_many), ErrorCode::ResourceLimit),
            (serde_json::json!([too_long]), ErrorCode::InvalidData),
            (serde_json::json!([""]), ErrorCode::InvalidData),
            (serde_json::json!("news"), ErrorCode::SerializationError),
        ] {
            let document = serde_json::json!({
                "schema": "aeris_charts-state",
                "schema_version": 1,
                "panes": [{"id": "pane-1"}],
                "drawings": [],
                "hidden_mark_groups": groups,
            })
            .to_string();
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            let before = chart.export_state_json().unwrap();
            let error = chart.import_state_json(&document).unwrap_err();
            assert_eq!(error.code(), code, "{document}");
            assert_eq!(chart.export_state_json().unwrap(), before);
            assert!(chart.hidden_timeline_groups().is_empty());
        }
    }
}
