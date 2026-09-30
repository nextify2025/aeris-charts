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
    /// B8 family option blocks; omitted when they equal the kind's defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_options: Option<crate::DrawingToolOptions>,
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
        | IndicatorKind::Cmf { .. }
        | IndicatorKind::Mfi { .. }
        | IndicatorKind::Wma { .. } => 1,
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
        IndicatorKind::Macd { .. } => 3,
        IndicatorKind::Stochastic { .. } => 2,
        IndicatorKind::Kdj { .. } => 3,
        IndicatorKind::VwapBands { .. } => 5,
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
        if let Some(color) = color {
            if color.len() > MAX_COLOR_BYTES || Color::parse_css(color).is_none() {
                return Err(field);
            }
        }
    }
    Ok(())
}

fn indicator_kind_is_valid(kind: &IndicatorKind) -> bool {
    match kind {
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
        } => *period > 0 && deviation.is_finite(),
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
        IndicatorKind::Cmf { period } => *period > 0,
        IndicatorKind::Mfi { period } => *period > 0,
        IndicatorKind::Volume { period } => *period > 0,
        IndicatorKind::VwapBands {
            standard_deviation,
            percent,
            ..
        } => standard_deviation.is_finite() && percent.is_finite(),
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
        let base_json = self.export_state_v1_json()?;
        let base: StateV1 = serde_json::from_str(&base_json)
            .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))?;
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
                        price_scale_id: (drawing.price_scale != DrawingPriceScale::Right)
                            .then(|| drawing.price_scale.name().to_string()),
                        color: Some(drawing.color.clone()),
                        width: Some(drawing.width),
                        line_style: Some(line_style_name(drawing.style).to_string()),
                        fill_color: drawing.fill_color.clone(),
                        preview_fill_color: drawing.preview_fill_color.clone(),
                        border_visible: (!drawing.border_visible).then_some(false),
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
                        tool_options: (drawing.tool_options != defaults.tool_options)
                            .then(|| drawing.tool_options.clone()),
                    },
                })
            })
            .collect::<Result<Vec<_>, ChartError>>()?;
        serde_json::to_string(&StateV1 {
            schema: "aeris_charts-state".to_string(),
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            panes,
            drawings,
            drawing_price_basis: self.drawing_price_basis().map(str::to_string),
        })
        .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))
    }

    fn export_state_v2_json(&self) -> Result<String, ChartError> {
        let base: StateV1 = serde_json::from_str(&self.export_state_v1_json()?)
            .map_err(|error| ChartError::new(ErrorCode::SerializationError, error.to_string()))?;
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
            let kind = DrawingKind::from_name(&item.kind)
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
            total_points = total_points
                .checked_add(item.anchors.len())
                .ok_or_else(|| resource("drawing anchor count overflow"))?;
            if total_points > PERSISTENCE_MAX_TOTAL_POINTS {
                return Err(resource(format!(
                    "V1 supports at most {PERSISTENCE_MAX_TOTAL_POINTS} total anchors"
                )));
            }
            if !kind.valid_point_count(item.anchors.len()) {
                return Err(invalid(format!(
                    "drawing {} has an invalid anchor count",
                    item.id
                )));
            }
            // Pane-anchored anchors are pane fractions with no time identity.
            let pane_anchored = kind.pane_anchored();
            if !item.anchor_times_micros.is_empty() && !pane_anchored {
                drawing_anchor_times.insert(item.id, item.anchor_times_micros.clone());
            }
            let bounded = |value: f64| value.is_finite() && value.abs() <= MAX_SAFE_VALUE;
            let fraction = |value: f64| (0.0..=1.0).contains(&value);
            if item.anchors.iter().any(|anchor| {
                !bounded(anchor.price)
                    || anchor.logical.is_some_and(|logical| !bounded(logical))
                    || anchor.time.is_some_and(|time| !bounded(time))
                    || (anchor.logical.is_none() && anchor.time.is_none())
                    || (pane_anchored
                        && !(anchor.logical.is_some_and(fraction) && fraction(anchor.price)))
            }) {
                return Err(invalid(format!(
                    "drawing {} has an invalid anchor",
                    item.id
                )));
            }
            // Anchor times are restored as pending identity and resolved against the host's
            // data on install (immediately when data is present, otherwise when it arrives).
            let pending_times = item
                .anchors
                .iter()
                .map(|anchor| anchor.time)
                .collect::<Vec<_>>();
            let anchors = item
                .anchors
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
            let style = item.style;
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
            if let Some(tool_options) = style.tool_options {
                if !tool_options.validate() {
                    return Err(invalid(format!(
                        "drawing {} has invalid tool options",
                        item.id
                    )));
                }
                drawing.tool_options = tool_options;
            }
            max_drawing_id = max_drawing_id.max(item.id);
            drawings.push(drawing);
        }
        let drawing_price_basis = state.drawing_price_basis.filter(|basis| !basis.is_empty());
        if drawing_price_basis
            .as_ref()
            .is_some_and(|basis| basis.len() > crate::MAX_DRAWING_GROUP_BYTES)
        {
            return Err(resource("drawing price basis is too large"));
        }
        Ok(ValidatedStateV1 {
            panes,
            drawings,
            drawing_anchor_times,
            drawing_price_basis,
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
        self.text_edit = None;
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
        })?;
        let mut resolved = Vec::with_capacity(state.indicators.len());
        let mut expected_outputs = Vec::with_capacity(state.indicators.len());
        for (study, indicator) in state.indicators.iter().enumerate() {
            if !indicator_kind_is_valid(&indicator.kind) {
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
            {
                if !matches!(indicator.kind, IndicatorKind::Vwap)
                    || source_refs_equal(&indicator.source, amount_ref)
                    || indicator
                        .volume_source
                        .as_ref()
                        .is_none_or(|volume| source_refs_equal(volume, amount_ref))
                    || (matches!(amount_ref, IndicatorSourceV3::Series { .. })
                        && !source_is_scalar(self, amount_source))
                {
                    return Err(invalid(format!(
                        "indicator {study} amount source must be a distinct scalar VWAP input"
                    )));
                }
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
        serde_json::from_value::<aeris_charts_core::options::ChartOptions>(
            state.chart_options.clone(),
        )
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
        let options_json = serde_json::to_string(&state.chart_options)
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        staged
            .apply_options(&options_json)
            .map_err(|error| invalid(format!("invalid V2 chart_options: {error}")))?;
        // Everything the live chart's own state can reject (an installed bar time label that the
        // document's session start does not fit) is decided before the first field is replaced.
        let prepared_options = self
            .prepare_options_patch(&state.chart_options)
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
        self.apply_prepared_options(&state.chart_options, prepared_options);
        // A document without exchange-time keys keeps the chart's installed zone and session
        // start; mirror a non-default one so the replaced options store still describes the live
        // chart (absent keys already mean UTC, keeping default documents byte-stable).
        if !self.exchange_time().is_utc_identity() {
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
        self.text_edit = None;
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
        assert!(chart.drawings[..3]
            .iter()
            .all(|drawing| drawing.pane_index == 0));
        assert!(chart.drawings[3..]
            .iter()
            .all(|drawing| drawing.pane_index == 1));
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
}
