//! Engine-owned advanced series.
//!
//! These series use the same ordered `ChartFrame` as every built-in series. Hosts only translate
//! typed data and options; price semantics, autoscale projections, geometry, and lifecycle stay in
//! the headless engine so WebGPU, Canvas2D, GPUI, and native rendering cannot diverge.

use crate::{ChartEngine, SeriesKind};
use aeris_charts_core::model::data_layer::{SeriesId, SeriesIdError};
use aeris_charts_core::model::data_validation::{
    MAX_SAFE_VALUE, MIN_SAFE_VALUE, ValidationError, ValidationReport,
};
use aeris_charts_core::model::plot_list::MismatchDirection;
use aeris_charts_core::style::{MARKET_DOWN_RGB, MARKET_UP_RGB};
use aeris_charts_render::color::Color;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureSeriesKind {
    GroupedBars,
    Heatmap,
    HlcArea,
    PrettyHistogram,
    BackgroundShade,
    StackedArea,
    StackedBars,
    WhiskerBox,
}

impl FeatureSeriesKind {
    pub fn from_u8(kind: u8) -> Option<Self> {
        Some(match kind {
            2 => Self::GroupedBars,
            3 => Self::Heatmap,
            4 => Self::HlcArea,
            5 => Self::PrettyHistogram,
            7 => Self::BackgroundShade,
            8 => Self::StackedArea,
            9 => Self::StackedBars,
            10 => Self::WhiskerBox,
            _ => return None,
        })
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::GroupedBars => 2,
            Self::Heatmap => 3,
            Self::HlcArea => 4,
            Self::PrettyHistogram => 5,
            Self::BackgroundShade => 7,
            Self::StackedArea => 8,
            Self::StackedBars => 9,
            Self::WhiskerBox => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeatmapCell {
    pub low: f64,
    pub high: f64,
    pub amount: f64,
    /// Host-resolved `cellShader` output. `None` uses the official default shader.
    pub color: Option<Color>,
}

impl HeatmapCell {
    pub(crate) fn rendered_color(&self) -> Color {
        self.color.unwrap_or_else(|| {
            let amount = self.amount.clamp(0.0, 100.0);
            Color::rgba(
                0,
                (100.0 + amount * 1.55).round().clamp(0.0, 255.0) as u8,
                amount.round().clamp(0.0, 255.0) as u8,
                ((0.2 + amount * 0.8).clamp(0.0, 1.0) * 255.0).round() as u8,
            )
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StackedAreaColor {
    pub line: Color,
    pub area: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    GroupedBars {
        values: Vec<f64>,
    },
    Heatmap {
        cells: Vec<HeatmapCell>,
    },
    HlcArea {
        high: f64,
        low: f64,
        close: f64,
    },
    PrettyHistogram {
        value: f64,
        color: Option<Color>,
    },
    BackgroundShade {
        value: f64,
    },
    StackedArea {
        values: Vec<f64>,
    },
    StackedBars {
        values: Vec<f64>,
    },
    WhiskerBox {
        quartiles: [f64; 5],
        outliers: Vec<f64>,
    },
}

impl FeatureValue {
    fn kind(&self) -> FeatureSeriesKind {
        match self {
            Self::GroupedBars { .. } => FeatureSeriesKind::GroupedBars,
            Self::Heatmap { .. } => FeatureSeriesKind::Heatmap,
            Self::HlcArea { .. } => FeatureSeriesKind::HlcArea,
            Self::PrettyHistogram { .. } => FeatureSeriesKind::PrettyHistogram,
            Self::BackgroundShade { .. } => FeatureSeriesKind::BackgroundShade,
            Self::StackedArea { .. } => FeatureSeriesKind::StackedArea,
            Self::StackedBars { .. } => FeatureSeriesKind::StackedBars,
            Self::WhiskerBox { .. } => FeatureSeriesKind::WhiskerBox,
        }
    }

    fn finite_and_safe(&self) -> bool {
        let safe =
            |value: f64| value.is_finite() && (MIN_SAFE_VALUE..=MAX_SAFE_VALUE).contains(&value);
        match self {
            Self::PrettyHistogram { value, .. } | Self::BackgroundShade { value } => safe(*value),
            Self::GroupedBars { values }
            | Self::StackedArea { values }
            | Self::StackedBars { values } => {
                !values.is_empty() && values.iter().copied().all(safe)
            }
            Self::Heatmap { cells } => {
                !cells.is_empty()
                    && cells
                        .iter()
                        .all(|cell| safe(cell.low) && safe(cell.high) && safe(cell.amount))
            }
            Self::HlcArea { high, low, close } => safe(*high) && safe(*low) && safe(*close),
            Self::WhiskerBox {
                quartiles,
                outliers,
            } => quartiles.iter().copied().all(safe) && outliers.iter().copied().all(safe),
        }
    }

    fn semantic_anomaly(&self) -> bool {
        match self {
            Self::HlcArea { high, low, close } => high < low || close < low || close > high,
            Self::Heatmap { cells } => cells.iter().any(|cell| cell.high < cell.low),
            Self::WhiskerBox { quartiles, .. } => {
                quartiles.windows(2).any(|pair| pair[0] > pair[1])
            }
            _ => false,
        }
    }

    /// OHLC-shaped projection used by the canonical scale/query layer. The complete payload stays
    /// in [`FeatureSeriesState`]; this projection is a derived index, not renderer-owned data.
    pub(crate) fn projection(&self) -> [f64; 4] {
        match self {
            Self::PrettyHistogram { value, .. } => [*value; 4],
            Self::GroupedBars { values } => {
                range_projection(values, *values.last().unwrap_or(&0.0))
            }
            Self::Heatmap { cells } => {
                let low = cells
                    .iter()
                    .map(|cell| cell.low)
                    .fold(f64::INFINITY, f64::min);
                let high = cells
                    .iter()
                    .map(|cell| cell.high)
                    .fold(f64::NEG_INFINITY, f64::max);
                let mid = low + (high - low) / 2.0;
                [mid, high, low, mid]
            }
            Self::HlcArea { high, low, close } => [*close, *high, *low, *close],
            // The reference intentionally returns NaN so this visual never owns a price scale.
            Self::BackgroundShade { .. } => [f64::NAN; 4],
            Self::StackedArea { values } | Self::StackedBars { values } => {
                let total = values.iter().sum::<f64>();
                [0.0, total.max(0.0), total.min(0.0), total]
            }
            Self::WhiskerBox { quartiles, .. } => {
                [quartiles[2], quartiles[4], quartiles[0], quartiles[2]]
            }
        }
    }
}

fn range_projection(values: &[f64], close: f64) -> [f64; 4] {
    let low = values.iter().copied().fold(0.0, f64::min);
    let high = values.iter().copied().fold(0.0, f64::max);
    [0.0, high, low, close]
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureDataPoint {
    pub time: f64,
    /// `None` is an explicit whitespace row.
    pub value: Option<FeatureValue>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureSeriesOptionsPatch {
    pub color: Option<Color>,
    pub colors: Option<Vec<Color>>,
    pub stacked_area_colors: Option<Vec<StackedAreaColor>>,
    pub line_color: Option<Color>,
    pub top_color: Option<Color>,
    pub bottom_color: Option<Color>,
    pub line_width: Option<f64>,
    pub base_price: Option<f64>,
    pub cell_border_width: Option<f64>,
    pub cell_border_color: Option<Color>,
    pub high_line_color: Option<Color>,
    pub low_line_color: Option<Color>,
    pub close_line_color: Option<Color>,
    pub area_top_color: Option<Color>,
    pub area_bottom_color: Option<Color>,
    pub high_line_width: Option<f64>,
    pub low_line_width: Option<f64>,
    pub close_line_width: Option<f64>,
    pub width_percent: Option<f64>,
    pub radius: Option<f64>,
    pub low_color: Option<Color>,
    pub high_color: Option<Color>,
    pub low_value: Option<f64>,
    pub high_value: Option<f64>,
    pub opacity: Option<f64>,
    pub whisker_color: Option<Color>,
    pub lower_quartile_fill: Option<Color>,
    pub upper_quartile_fill: Option<Color>,
    pub outlier_color: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FeatureSeriesOptions {
    pub color: Color,
    pub colors: Vec<Color>,
    pub stacked_area_colors: Vec<StackedAreaColor>,
    pub line_color: Color,
    pub top_color: Color,
    pub bottom_color: Color,
    pub line_width: f64,
    pub base_price: f64,
    pub cell_border_width: f64,
    pub cell_border_color: Color,
    pub high_line_color: Color,
    pub low_line_color: Color,
    pub close_line_color: Color,
    pub area_top_color: Color,
    pub area_bottom_color: Color,
    pub high_line_width: f64,
    pub low_line_width: f64,
    pub close_line_width: f64,
    pub width_percent: f64,
    pub radius: Option<f64>,
    pub low_color: Color,
    pub high_color: Color,
    pub low_value: f64,
    pub high_value: f64,
    pub opacity: f64,
    pub whisker_color: Color,
    pub lower_quartile_fill: Color,
    pub upper_quartile_fill: Color,
    pub outlier_color: Color,
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::rgba(r, g, b, a)
}

fn market_rgb(rgb: (u8, u8, u8)) -> Color {
    Color::rgb(rgb.0, rgb.1, rgb.2)
}

fn market_rgba(rgb: (u8, u8, u8), alpha: u8) -> Color {
    Color::rgba(rgb.0, rgb.1, rgb.2, alpha)
}

impl Default for FeatureSeriesOptions {
    fn default() -> Self {
        let palette = vec![
            rgb(0x29, 0x62, 0xff),
            rgb(0xe1, 0x57, 0x5a),
            rgb(0xf2, 0x8e, 0x2c),
            rgb(0xa4, 0x59, 0xd1),
            rgb(0x1b, 0x9c, 0x85),
        ];
        Self {
            color: rgb(0xd6, 0x38, 0x64),
            colors: palette.clone(),
            stacked_area_colors: palette
                .iter()
                .copied()
                .map(|line| StackedAreaColor {
                    line,
                    area: Color::rgba(line.r(), line.g(), line.b(), 51),
                })
                .collect(),
            line_color: rgb(40, 98, 255),
            top_color: rgba(40, 98, 255, 102),
            bottom_color: rgba(40, 98, 255, 0),
            line_width: 2.0,
            base_price: 0.0,
            cell_border_width: 1.0,
            cell_border_color: rgba(0, 0, 0, 0),
            high_line_color: market_rgb(MARKET_UP_RGB),
            low_line_color: market_rgb(MARKET_DOWN_RGB),
            close_line_color: rgb(0x87, 0x89, 0x93),
            area_top_color: market_rgba(MARKET_UP_RGB, 51),
            area_bottom_color: market_rgba(MARKET_DOWN_RGB, 51),
            high_line_width: 2.0,
            low_line_width: 2.0,
            close_line_width: 2.0,
            width_percent: 50.0,
            radius: None,
            low_color: rgb(50, 50, 255),
            high_color: rgb(255, 50, 50),
            low_value: 0.0,
            high_value: 100.0,
            opacity: 0.8,
            whisker_color: rgb(106, 27, 154),
            lower_quartile_fill: rgb(103, 58, 183),
            upper_quartile_fill: rgb(233, 30, 99),
            outlier_color: rgb(149, 152, 161),
        }
    }
}

impl FeatureSeriesOptions {
    pub(crate) fn reset_style_to_defaults(&mut self) {
        let defaults = Self::default();
        self.color = defaults.color;
        self.colors = defaults.colors;
        self.stacked_area_colors = defaults.stacked_area_colors;
        self.line_color = defaults.line_color;
        self.top_color = defaults.top_color;
        self.bottom_color = defaults.bottom_color;
        self.line_width = defaults.line_width;
        self.cell_border_width = defaults.cell_border_width;
        self.cell_border_color = defaults.cell_border_color;
        self.high_line_color = defaults.high_line_color;
        self.low_line_color = defaults.low_line_color;
        self.close_line_color = defaults.close_line_color;
        self.area_top_color = defaults.area_top_color;
        self.area_bottom_color = defaults.area_bottom_color;
        self.high_line_width = defaults.high_line_width;
        self.low_line_width = defaults.low_line_width;
        self.close_line_width = defaults.close_line_width;
        self.radius = defaults.radius;
        self.low_color = defaults.low_color;
        self.high_color = defaults.high_color;
        self.opacity = defaults.opacity;
        self.whisker_color = defaults.whisker_color;
        self.lower_quartile_fill = defaults.lower_quartile_fill;
        self.upper_quartile_fill = defaults.upper_quartile_fill;
        self.outlier_color = defaults.outlier_color;
    }

    fn apply(&mut self, patch: FeatureSeriesOptionsPatch) {
        macro_rules! set {
            ($field:ident) => {
                if let Some(value) = patch.$field {
                    self.$field = value;
                }
            };
        }
        set!(color);
        if patch
            .colors
            .as_ref()
            .is_some_and(|colors| !colors.is_empty())
        {
            self.colors = patch.colors.unwrap_or_default();
        }
        if patch
            .stacked_area_colors
            .as_ref()
            .is_some_and(|colors| !colors.is_empty())
        {
            self.stacked_area_colors = patch.stacked_area_colors.unwrap_or_default();
        }
        set!(line_color);
        set!(top_color);
        set!(bottom_color);
        if patch
            .line_width
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            self.line_width = patch.line_width.unwrap_or(self.line_width);
        }
        if patch.base_price.is_some_and(f64::is_finite) {
            self.base_price = patch.base_price.unwrap_or(self.base_price);
        }
        if patch
            .cell_border_width
            .is_some_and(|value| value.is_finite() && value >= 0.0)
        {
            self.cell_border_width = patch.cell_border_width.unwrap_or(self.cell_border_width);
        }
        set!(cell_border_color);
        set!(high_line_color);
        set!(low_line_color);
        set!(close_line_color);
        set!(area_top_color);
        set!(area_bottom_color);
        if patch
            .high_line_width
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            self.high_line_width = patch.high_line_width.unwrap_or(self.high_line_width);
        }
        if patch
            .low_line_width
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            self.low_line_width = patch.low_line_width.unwrap_or(self.low_line_width);
        }
        if patch
            .close_line_width
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            self.close_line_width = patch.close_line_width.unwrap_or(self.close_line_width);
        }
        if patch
            .width_percent
            .is_some_and(|value| value.is_finite() && value > 0.0 && value <= 100.0)
        {
            self.width_percent = patch.width_percent.unwrap_or(self.width_percent);
        }
        if let Some(radius) = patch.radius
            && radius.is_finite()
            && radius >= 0.0
        {
            self.radius = Some(radius);
        }
        set!(low_color);
        set!(high_color);
        if patch.low_value.is_some_and(f64::is_finite) {
            self.low_value = patch.low_value.unwrap_or(self.low_value);
        }
        if patch.high_value.is_some_and(f64::is_finite) {
            self.high_value = patch.high_value.unwrap_or(self.high_value);
        }
        if patch.opacity.is_some_and(f64::is_finite) {
            self.opacity = patch.opacity.unwrap_or(self.opacity).clamp(0.0, 1.0);
        }
        set!(whisker_color);
        set!(lower_quartile_fill);
        set!(upper_quartile_fill);
        set!(outlier_color);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FeatureRow {
    pub time: i64,
    pub value: Option<FeatureValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FeatureSeriesState {
    pub kind: FeatureSeriesKind,
    pub options: FeatureSeriesOptions,
    pub rows: Vec<FeatureRow>,
    stacked_area_layers: Option<usize>,
    stacked_area_value_rows: usize,
}

impl FeatureSeriesState {
    fn new(kind: FeatureSeriesKind, patch: FeatureSeriesOptionsPatch) -> Self {
        let mut options = FeatureSeriesOptions::default();
        options.apply(patch);
        Self {
            kind,
            options,
            rows: Vec::new(),
            stacked_area_layers: None,
            stacked_area_value_rows: 0,
        }
    }

    pub(crate) fn capacity_bytes(&self) -> usize {
        let row_payload = self
            .rows
            .iter()
            .filter_map(|row| row.value.as_ref())
            .map(|value| match value {
                FeatureValue::GroupedBars { values }
                | FeatureValue::StackedArea { values }
                | FeatureValue::StackedBars { values } => values.capacity() * size_of::<f64>(),
                FeatureValue::Heatmap { cells } => cells.capacity() * size_of::<HeatmapCell>(),
                FeatureValue::WhiskerBox { outliers, .. } => outliers.capacity() * size_of::<f64>(),
                _ => 0,
            })
            .sum::<usize>();
        self.rows.capacity() * size_of::<FeatureRow>()
            + row_payload
            + self.options.colors.capacity() * size_of::<Color>()
            + self.options.stacked_area_colors.capacity() * size_of::<StackedAreaColor>()
    }
}

impl ChartEngine {
    pub(crate) fn feature_bar_color(&self, id: SeriesId, row: usize) -> Option<Color> {
        let series = self.series_entry(id)?;
        let feature = series.feature.as_ref()?;
        let value = feature.rows.get(row)?.value.as_ref()?;
        let options = &feature.options;
        Some(match value {
            FeatureValue::GroupedBars { values } => {
                options.colors[values.len().saturating_sub(1) % options.colors.len()]
            }
            FeatureValue::Heatmap { cells } => {
                let projection = value.projection()[3];
                cells
                    .iter()
                    .rev()
                    .find(|cell| {
                        let low = cell.low.min(cell.high);
                        let high = cell.low.max(cell.high);
                        projection >= low && projection <= high
                    })
                    .or_else(|| cells.last())?
                    .rendered_color()
            }
            FeatureValue::HlcArea { .. } => options.close_line_color,
            FeatureValue::PrettyHistogram { color, .. } => color.unwrap_or(options.color),
            FeatureValue::BackgroundShade { .. } => options.color,
            FeatureValue::StackedArea { values } => {
                options.stacked_area_colors
                    [values.len().saturating_sub(1) % options.stacked_area_colors.len()]
                .line
            }
            FeatureValue::StackedBars { values } => {
                options.colors[values.len().saturating_sub(1) % options.colors.len()]
            }
            FeatureValue::WhiskerBox { .. } => options.whisker_color,
        })
    }

    pub fn add_feature_series(
        &mut self,
        kind: FeatureSeriesKind,
        options: FeatureSeriesOptionsPatch,
    ) -> SeriesId {
        let id = self.add_series(SeriesKind::Feature);
        self.configure_feature_series(id, kind, options);
        id
    }

    pub fn configure_feature_series(
        &mut self,
        id: SeriesId,
        kind: FeatureSeriesKind,
        options: FeatureSeriesOptionsPatch,
    ) -> bool {
        let had_data = !self.data.plot(id).is_empty();
        let Some(series) = self
            .series
            .iter_mut()
            .find(|series| series.id == id && !series.removed)
        else {
            return false;
        };
        series.kind = SeriesKind::Feature;
        series.feature = Some(FeatureSeriesState::new(kind, options));
        series.custom_frame = Default::default();
        self.data.set_rows_count_as_data(id, true);
        if had_data {
            let cleared = self.install_series_data(
                id,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            debug_assert!(cleared, "validated series must accept an empty replacement");
        }
        self.invalidate_frame_series(id);
        true
    }

    pub fn feature_series_kind(&self, id: SeriesId) -> Option<FeatureSeriesKind> {
        self.series_entry(id)?
            .feature
            .as_ref()
            .map(|feature| feature.kind)
    }

    pub fn feature_series_options_json(&self, id: SeriesId) -> Option<String> {
        let feature = self.series_entry(id)?.feature.as_ref()?;
        let options = &feature.options;
        let colors = if feature.kind == FeatureSeriesKind::StackedArea {
            serde_json::Value::Array(
                options
                    .stacked_area_colors
                    .iter()
                    .map(|color| {
                        serde_json::json!({
                            "line": color.line.to_css(),
                            "area": color.area.to_css(),
                        })
                    })
                    .collect(),
            )
        } else {
            serde_json::Value::Array(
                options
                    .colors
                    .iter()
                    .map(|color| serde_json::Value::String(color.to_css()))
                    .collect(),
            )
        };
        let mut value = serde_json::json!({
            "color": options.color.to_css(),
            "colors": colors,
            "line_color": options.line_color.to_css(),
            "top_color": options.top_color.to_css(),
            "bottom_color": options.bottom_color.to_css(),
            "line_width": options.line_width,
            "base_price": options.base_price,
            "cell_border_width": options.cell_border_width,
            "cell_border_color": options.cell_border_color.to_css(),
            "high_line_color": options.high_line_color.to_css(),
            "low_line_color": options.low_line_color.to_css(),
            "close_line_color": options.close_line_color.to_css(),
            "area_top_color": options.area_top_color.to_css(),
            "area_bottom_color": options.area_bottom_color.to_css(),
            "high_line_width": options.high_line_width,
            "low_line_width": options.low_line_width,
            "close_line_width": options.close_line_width,
            "width_percent": options.width_percent,
            "low_color": options.low_color.to_css(),
            "high_color": options.high_color.to_css(),
            "low_value": options.low_value,
            "high_value": options.high_value,
            "opacity": options.opacity,
            "whisker_color": options.whisker_color.to_css(),
            "lower_quartile_fill": options.lower_quartile_fill.to_css(),
            "upper_quartile_fill": options.upper_quartile_fill.to_css(),
            "outlier_color": options.outlier_color.to_css(),
        });
        if let Some(radius) = options.radius {
            value["radius"] = serde_json::json!(radius);
        }
        serde_json::to_string(&value).ok()
    }

    pub fn apply_feature_series_options(
        &mut self,
        id: SeriesId,
        patch: FeatureSeriesOptionsPatch,
    ) -> bool {
        let Some(feature) = self
            .series_entry_mut(id)
            .and_then(|series| series.feature.as_mut())
        else {
            return false;
        };
        feature.options.apply(patch);
        self.invalidate_frame_series(id);
        true
    }

    pub fn set_feature_series_data(
        &mut self,
        id: SeriesId,
        input: Vec<FeatureDataPoint>,
    ) -> Result<ValidationReport, ValidationError> {
        let kind = self
            .series_entry(id)
            .and_then(|series| series.feature.as_ref())
            .map(|feature| feature.kind)
            .ok_or_else(|| match self.data.validate_series_id(id) {
                Err(SeriesIdError::Stale(id)) => ValidationError::StaleSeries(id),
                _ => ValidationError::UnknownSeries(id),
            })?;
        let input_was_empty = input.is_empty();
        let (mut rows, mut report) = sanitize_feature_rows(kind, input)?;
        if kind == FeatureSeriesKind::StackedArea {
            let expected_layers = rows.iter().find_map(|row| match row.value.as_ref()? {
                FeatureValue::StackedArea { values } => Some(values.len()),
                _ => None,
            });
            if let Some(expected_layers) = expected_layers {
                let before = rows.len();
                rows.retain(|row| match row.value.as_ref() {
                    Some(FeatureValue::StackedArea { values }) => values.len() == expected_layers,
                    _ => true,
                });
                report.dropped_invalid += before - rows.len();
                report.accepted = rows.len();
            }
        }
        // An explicitly empty set clears the series. A non-empty payload with no valid rows is a
        // rejected transaction and must not erase previously accepted chart data.
        if !input_was_empty && rows.is_empty() && report.dropped_invalid > 0 {
            return Ok(report);
        }
        let mut times = Vec::with_capacity(rows.len());
        let mut open = Vec::with_capacity(rows.len());
        let mut high = Vec::with_capacity(rows.len());
        let mut low = Vec::with_capacity(rows.len());
        let mut close = Vec::with_capacity(rows.len());
        for row in &rows {
            let values = row
                .value
                .as_ref()
                .map_or([f64::NAN; 4], FeatureValue::projection);
            times.push(row.time);
            open.push(values[0]);
            high.push(values[1]);
            low.push(values[2]);
            close.push(values[3]);
        }
        if !self.install_series_data(id, times, open, high, low, close) {
            return Err(ValidationError::UnknownSeries(id));
        }
        let retained = self
            .data
            .series_data(id)
            .map_or(0, |(times, _)| times.len());
        if retained < rows.len() {
            rows.drain(..rows.len() - retained);
        }
        if let Some(feature) = self
            .series_entry_mut(id)
            .and_then(|series| series.feature.as_mut())
        {
            feature.stacked_area_layers = rows.iter().find_map(|row| match row.value.as_ref()? {
                FeatureValue::StackedArea { values } => Some(values.len()),
                _ => None,
            });
            feature.stacked_area_value_rows = rows
                .iter()
                .filter(|row| matches!(row.value, Some(FeatureValue::StackedArea { .. })))
                .count();
            feature.rows = rows;
        }
        self.invalidate_frame_series(id);
        Ok(report)
    }

    /// Append or replace one engine-owned advanced-series row. The raw payload and its canonical
    /// OHLC projection are updated together, so queries, autoscale, indicators, and every backend
    /// observe the same state.
    pub fn update_feature_series_data(
        &mut self,
        id: SeriesId,
        point: FeatureDataPoint,
    ) -> Result<ValidationReport, ValidationError> {
        let kind = self
            .series_entry(id)
            .and_then(|series| series.feature.as_ref())
            .map(|feature| feature.kind)
            .ok_or_else(|| match self.data.validate_series_id(id) {
                Err(SeriesIdError::Stale(id)) => ValidationError::StaleSeries(id),
                _ => ValidationError::UnknownSeries(id),
            })?;
        let (mut rows, mut report) = sanitize_feature_rows(kind, vec![point])?;
        let Some(row) = rows.pop() else {
            return Ok(report);
        };
        let stacked_update = if kind == FeatureSeriesKind::StackedArea {
            let feature = self
                .series_entry(id)
                .and_then(|series| series.feature.as_ref())
                .expect("validated feature series");
            let replacing_value = feature
                .rows
                .binary_search_by_key(&row.time, |existing| existing.time)
                .ok()
                .is_some_and(|position| {
                    matches!(
                        feature.rows[position].value,
                        Some(FeatureValue::StackedArea { .. })
                    )
                });
            let incoming_layers = match row.value.as_ref() {
                Some(FeatureValue::StackedArea { values }) => Some(values.len()),
                _ => None,
            };
            let other_value_rows = feature
                .stacked_area_value_rows
                .saturating_sub(usize::from(replacing_value));
            if other_value_rows > 0
                && incoming_layers.is_some_and(|layers| {
                    feature
                        .stacked_area_layers
                        .is_some_and(|expected| layers != expected)
                })
            {
                report.accepted = 0;
                report.dropped_invalid += 1;
                return Ok(report);
            }
            let value_rows = other_value_rows + usize::from(incoming_layers.is_some());
            Some((incoming_layers.or(feature.stacked_area_layers), value_rows))
        } else {
            None
        };
        let projection = row
            .value
            .as_ref()
            .map_or([f64::NAN; 4], FeatureValue::projection);
        let previous_generation = self.data.series_generation(id).unwrap_or(0);
        let from = self
            .data
            .series_data(id)
            .map(|(times, _)| {
                times
                    .binary_search(&row.time)
                    .unwrap_or_else(|position| position)
            })
            .unwrap_or_default();
        self.data.update_styled(id, row.time, projection, [None; 3]);
        if let Some(feature) = self
            .series_entry_mut(id)
            .and_then(|series| series.feature.as_mut())
        {
            match feature
                .rows
                .binary_search_by_key(&row.time, |item| item.time)
            {
                Ok(position) => feature.rows[position] = row,
                Err(position) => feature.rows.insert(position, row),
            }
            if let Some((layers, value_rows)) = stacked_update {
                feature.stacked_area_layers = if value_rows > 0 { layers } else { None };
                feature.stacked_area_value_rows = value_rows;
            }
        }
        let trimmed = self.enforce_series_cap(id);
        self.sync_time_points();
        self.update_indicators_after_change(
            id,
            crate::IndicatorChange {
                from: if trimmed { 0 } else { from },
                previous_generation,
                full_replace: trimmed,
            },
        );
        self.invalidate_frame_series(id);
        Ok(report)
    }

    /// Canonical feature payloads in post-sanitize engine order.
    pub fn feature_series_data(&self, id: SeriesId) -> Option<Vec<FeatureDataPoint>> {
        let feature = self.series_entry(id)?.feature.as_ref()?;
        Some(
            feature
                .rows
                .iter()
                .map(|row| FeatureDataPoint {
                    time: row.time as f64,
                    value: row.value.clone(),
                })
                .collect(),
        )
    }

    /// Canonical feature payload at a logical index, honoring the standard mismatch direction.
    pub fn feature_series_data_by_index(
        &self,
        id: SeriesId,
        logical_index: i64,
        mismatch: MismatchDirection,
    ) -> Option<FeatureDataPoint> {
        let feature = self.series_entry(id)?.feature.as_ref()?;
        let row = self.data.plot(id).search(logical_index, mismatch)?;
        let value = feature.rows.get(row)?;
        Some(FeatureDataPoint {
            time: value.time as f64,
            value: value.value.clone(),
        })
    }

    pub(crate) fn feature_series_capacity_bytes(&self) -> usize {
        self.series
            .iter()
            .filter_map(|series| series.feature.as_ref())
            .map(FeatureSeriesState::capacity_bytes)
            .sum()
    }

    pub(crate) fn truncate_feature_rows(&mut self, id: SeriesId, len: usize) {
        if let Some(feature) = self
            .series_entry_mut(id)
            .and_then(|series| series.feature.as_mut())
        {
            let removed_values = feature.rows[len.min(feature.rows.len())..]
                .iter()
                .filter(|row| matches!(row.value, Some(FeatureValue::StackedArea { .. })))
                .count();
            feature.stacked_area_value_rows = feature
                .stacked_area_value_rows
                .saturating_sub(removed_values);
            if feature.stacked_area_value_rows == 0 {
                feature.stacked_area_layers = None;
            }
            feature.rows.truncate(len);
        }
    }

    pub(crate) fn trim_feature_rows_front(&mut self, id: SeriesId, keep: usize) {
        if let Some(feature) = self
            .series_entry_mut(id)
            .and_then(|series| series.feature.as_mut())
        {
            let drop = feature.rows.len().saturating_sub(keep);
            let removed_values = feature.rows[..drop]
                .iter()
                .filter(|row| matches!(row.value, Some(FeatureValue::StackedArea { .. })))
                .count();
            feature.stacked_area_value_rows = feature
                .stacked_area_value_rows
                .saturating_sub(removed_values);
            if feature.stacked_area_value_rows == 0 {
                feature.stacked_area_layers = None;
            }
            feature.rows.drain(..drop);
        }
    }
}

fn sanitize_feature_rows(
    kind: FeatureSeriesKind,
    input: Vec<FeatureDataPoint>,
) -> Result<(Vec<FeatureRow>, ValidationReport), ValidationError> {
    let times = input
        .iter()
        .enumerate()
        .map(|(index, point)| {
            aeris_charts_core::model::data_validation::validate_timestamp(point.time)
                .map_err(|error| ValidationError::InvalidTimestamp { index, error })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = ValidationReport::default();
    let mut rows = Vec::with_capacity(input.len());
    for (source, point) in input.into_iter().enumerate() {
        if let Some(value) = &point.value {
            if value.kind() != kind || !value.finite_and_safe() {
                report.dropped_invalid += 1;
                report.dropped_non_finite += 1;
                continue;
            }
            if value.semantic_anomaly() {
                report.semantic_anomalies += 1;
            }
        }
        rows.push((times[source], source, point.value));
    }
    report.reordered = rows.windows(2).any(|pair| pair[0].0 > pair[1].0);
    rows.sort_by_key(|(time, _, _)| *time);
    let mut sanitized: Vec<FeatureRow> = Vec::with_capacity(rows.len());
    for (time, _, value) in rows {
        if sanitized.last().is_some_and(|row| row.time == time) {
            sanitized.pop();
            report.dropped_duplicate += 1;
        }
        sanitized.push(FeatureRow { time, value });
    }
    report.accepted = sanitized.len();
    Ok((sanitized, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_kind_codes_preserve_surviving_public_ids() {
        let expected = [
            (2, FeatureSeriesKind::GroupedBars),
            (3, FeatureSeriesKind::Heatmap),
            (4, FeatureSeriesKind::HlcArea),
            (5, FeatureSeriesKind::PrettyHistogram),
            (7, FeatureSeriesKind::BackgroundShade),
            (8, FeatureSeriesKind::StackedArea),
            (9, FeatureSeriesKind::StackedBars),
            (10, FeatureSeriesKind::WhiskerBox),
        ];
        for (code, kind) in expected {
            assert_eq!(FeatureSeriesKind::from_u8(code), Some(kind));
            assert_eq!(kind.to_u8(), code);
        }
        assert_eq!(FeatureSeriesKind::from_u8(0), None);
        assert_eq!(FeatureSeriesKind::from_u8(1), None);
        assert_eq!(FeatureSeriesKind::from_u8(6), None);
    }

    fn sample_value(kind: FeatureSeriesKind, index: usize) -> FeatureValue {
        let value = 10.0 + index as f64;
        match kind {
            FeatureSeriesKind::GroupedBars => FeatureValue::GroupedBars {
                values: vec![value, value + 2.0, value - 2.0],
            },
            FeatureSeriesKind::Heatmap => FeatureValue::Heatmap {
                cells: vec![HeatmapCell {
                    low: value - 2.0,
                    high: value + 2.0,
                    amount: index as f64 / 2.0,
                    color: None,
                }],
            },
            FeatureSeriesKind::HlcArea => FeatureValue::HlcArea {
                high: value + 2.0,
                low: value - 2.0,
                close: value,
            },
            FeatureSeriesKind::PrettyHistogram => {
                FeatureValue::PrettyHistogram { value, color: None }
            }
            FeatureSeriesKind::BackgroundShade => FeatureValue::BackgroundShade { value },
            FeatureSeriesKind::StackedArea => FeatureValue::StackedArea {
                values: vec![value, value / 2.0, value / 4.0],
            },
            FeatureSeriesKind::StackedBars => FeatureValue::StackedBars {
                values: vec![value, value / 2.0, value / 4.0],
            },
            FeatureSeriesKind::WhiskerBox => FeatureValue::WhiskerBox {
                quartiles: [value - 4.0, value - 2.0, value, value + 2.0, value + 4.0],
                outliers: vec![value + 5.0],
            },
        }
    }

    #[test]
    fn feature_data_is_engine_owned_sorted_and_last_wins() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::HlcArea,
            FeatureSeriesOptionsPatch::default(),
        );
        let report = chart
            .set_feature_series_data(
                0,
                vec![
                    FeatureDataPoint {
                        time: 2.0,
                        value: Some(FeatureValue::HlcArea {
                            high: 12.0,
                            low: 8.0,
                            close: 10.0,
                        }),
                    },
                    FeatureDataPoint {
                        time: 1.0,
                        value: Some(FeatureValue::HlcArea {
                            high: 11.0,
                            low: 7.0,
                            close: 9.0,
                        }),
                    },
                    FeatureDataPoint {
                        time: 2.0,
                        value: Some(FeatureValue::HlcArea {
                            high: 14.0,
                            low: 6.0,
                            close: 13.0,
                        }),
                    },
                ],
            )
            .unwrap();
        assert!(report.reordered);
        assert_eq!(report.dropped_duplicate, 1);
        let feature = chart.series_entry(0).unwrap().feature.as_ref().unwrap();
        assert_eq!(feature.rows.len(), 2);
        assert!(matches!(
            feature.rows[1].value,
            Some(FeatureValue::HlcArea { close: 13.0, .. })
        ));
        let (times, values) = chart.data.series_data(0).unwrap();
        assert_eq!(times, [1, 2]);
        assert_eq!(values[3], [9.0, 13.0]);
    }

    #[test]
    fn feature_live_colors_and_snapshots_follow_the_rendered_projection() {
        let close_color = Color::rgb(1, 2, 3);
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::HlcArea,
            FeatureSeriesOptionsPatch {
                close_line_color: Some(close_color),
                ..FeatureSeriesOptionsPatch::default()
            },
        );
        chart
            .set_feature_series_data(
                0,
                vec![
                    FeatureDataPoint {
                        time: 1.0,
                        value: Some(FeatureValue::HlcArea {
                            high: 12.0,
                            low: 8.0,
                            close: 10.0,
                        }),
                    },
                    FeatureDataPoint {
                        time: 2.0,
                        value: Some(FeatureValue::HlcArea {
                            high: 13.0,
                            low: 9.0,
                            close: 11.0,
                        }),
                    },
                ],
            )
            .unwrap();

        assert_eq!(chart.feature_bar_color(0, 1), Some(close_color));
        let snapshot = chart.value_snapshot(None).remove(0);
        assert_eq!(snapshot.value, Some(11.0));
        assert_eq!(
            (snapshot.open, snapshot.high, snapshot.low, snapshot.close),
            (None, None, None, None)
        );
        chart.set_price_scale_visible_range_for(0, crate::PriceScaleTarget::Right, 0.0, 20.0);
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let frame = chart.build_frame();
        assert!(
            frame.panes[0].main.iter().any(|primitive| matches!(
                primitive,
                aeris_charts_render::draw_list::Prim::HLine { color, .. }
                    if *color == close_color
            )),
            "primitives: {:?}",
            frame.panes[0].main
        );
        let axis = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        assert!(
            axis.labels
                .iter()
                .any(|label| matches!(label.background, Some((.., color)) if color == close_color))
        );
    }

    #[test]
    fn heatmap_live_color_uses_the_rendered_cell_shader() {
        let shader_color = Color::rgb(20, 220, 120);
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::Heatmap,
            FeatureSeriesOptionsPatch::default(),
        );
        chart
            .set_feature_series_data(
                0,
                [1.0, 2.0]
                    .into_iter()
                    .map(|time| FeatureDataPoint {
                        time,
                        value: Some(FeatureValue::Heatmap {
                            cells: vec![HeatmapCell {
                                low: 8.0,
                                high: 12.0,
                                amount: 50.0,
                                color: Some(shader_color),
                            }],
                        }),
                    })
                    .collect(),
            )
            .unwrap();
        assert_eq!(chart.feature_bar_color(0, 1), Some(shader_color));

        chart.set_price_scale_visible_range_for(0, crate::PriceScaleTarget::Right, 0.0, 20.0);
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let frame = chart.build_frame();
        assert!(frame.panes[0].main.iter().any(|primitive| matches!(
            primitive,
            aeris_charts_render::draw_list::Prim::HLine { color, .. }
                if *color == shader_color
        )));
        let axis = chart.build_axis_frame(
            80.0,
            |text, _bold| text.len() as f64 * 7.0,
            |text, _bold| text.len() as f64 * 6.0,
        );
        assert!(
            axis.labels.iter().any(
                |label| matches!(label.background, Some((.., color)) if color == shader_color)
            )
        );
    }

    #[test]
    fn stacked_area_rejects_rows_that_cannot_share_rendered_layers() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::StackedArea,
            FeatureSeriesOptionsPatch::default(),
        );
        let report = chart
            .set_feature_series_data(
                0,
                vec![
                    FeatureDataPoint {
                        time: 1.0,
                        value: Some(FeatureValue::StackedArea {
                            values: vec![1.0, 2.0, 3.0],
                        }),
                    },
                    FeatureDataPoint {
                        time: 2.0,
                        value: Some(FeatureValue::StackedArea {
                            values: vec![4.0, 5.0],
                        }),
                    },
                    FeatureDataPoint {
                        time: 3.0,
                        value: None,
                    },
                ],
            )
            .unwrap();
        assert_eq!(report.accepted, 2);
        assert_eq!(report.dropped_invalid, 1);
        assert_eq!(chart.feature_series_data(0).unwrap().len(), 2);
        assert_eq!(chart.value_snapshot(None)[0].value, Some(6.0));

        let report = chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 4.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![6.0, 7.0],
                    }),
                },
            )
            .unwrap();
        assert_eq!(report.accepted, 0);
        assert_eq!(report.dropped_invalid, 1);
        assert_eq!(chart.feature_series_data(0).unwrap().len(), 2);

        assert_eq!(chart.series_pop(0, 1), Some(1));
        let report = chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 1.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![8.0, 9.0],
                    }),
                },
            )
            .unwrap();
        assert_eq!(
            report.accepted, 1,
            "the sole valued row can define a new schema"
        );
        assert_eq!(chart.series_pop(0, 1), Some(0));

        let report = chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 5.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![1.0, 2.0, 3.0, 4.0],
                    }),
                },
            )
            .unwrap();
        assert_eq!(report.accepted, 1, "pop resets an empty series' schema");
        chart.set_series_max_points(0, Some(1));
        chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 6.0,
                    value: None,
                },
            )
            .unwrap();
        let report = chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 7.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![10.0, 11.0],
                    }),
                },
            )
            .unwrap();
        assert_eq!(
            report.accepted, 1,
            "retention resets an empty series' schema"
        );
    }

    #[test]
    fn feature_reconfiguration_clears_payload_and_canonical_projection_together() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::StackedArea,
            FeatureSeriesOptionsPatch::default(),
        );
        chart
            .set_feature_series_data(
                0,
                vec![FeatureDataPoint {
                    time: 1.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![1.0, 2.0, 3.0],
                    }),
                }],
            )
            .unwrap();

        assert!(chart.configure_feature_series(
            0,
            FeatureSeriesKind::StackedArea,
            FeatureSeriesOptionsPatch::default(),
        ));
        assert!(chart.feature_series_data(0).unwrap().is_empty());
        assert!(chart.data.plot(0).is_empty());
        assert_eq!(chart.value_snapshot(None)[0].value, None);

        let report = chart
            .update_feature_series_data(
                0,
                FeatureDataPoint {
                    time: 2.0,
                    value: Some(FeatureValue::StackedArea {
                        values: vec![4.0, 5.0],
                    }),
                },
            )
            .unwrap();
        assert_eq!(report.accepted, 1);
        assert_eq!(chart.value_snapshot(None)[0].value, Some(9.0));
    }

    #[test]
    fn wrong_payload_kind_is_rejected_without_poisoning_scale_data() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::Heatmap,
            FeatureSeriesOptionsPatch::default(),
        );
        let report = chart
            .set_feature_series_data(
                0,
                vec![FeatureDataPoint {
                    time: 1.0,
                    value: Some(FeatureValue::PrettyHistogram {
                        value: 10.0,
                        color: None,
                    }),
                }],
            )
            .unwrap();
        assert_eq!(report.dropped_invalid, 1);
        assert!(chart.data.plot(0).is_empty());
    }

    #[test]
    fn feature_streaming_query_and_retention_stay_aligned() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::GroupedBars,
            FeatureSeriesOptionsPatch::default(),
        );
        chart.set_series_max_points(0, Some(2));
        for index in 0..3 {
            chart
                .update_feature_series_data(
                    0,
                    FeatureDataPoint {
                        time: index as f64,
                        value: Some(sample_value(FeatureSeriesKind::GroupedBars, index)),
                    },
                )
                .unwrap();
        }
        let data = chart.feature_series_data(0).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].time, 1.0);
        assert_eq!(data[1].time, 2.0);
        let point = chart
            .feature_series_data_by_index(0, 1, MismatchDirection::None)
            .unwrap();
        assert_eq!(point.time, 2.0);
        assert!(chart.memory_usage().feature_series_capacity_bytes > 0);
    }

    #[test]
    fn every_feature_kind_builds_shared_frame_geometry() {
        let kinds = [
            FeatureSeriesKind::GroupedBars,
            FeatureSeriesKind::Heatmap,
            FeatureSeriesKind::HlcArea,
            FeatureSeriesKind::PrettyHistogram,
            FeatureSeriesKind::BackgroundShade,
            FeatureSeriesKind::StackedArea,
            FeatureSeriesKind::StackedBars,
            FeatureSeriesKind::WhiskerBox,
        ];
        for kind in kinds {
            let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
            chart.configure_feature_series(0, kind, FeatureSeriesOptionsPatch::default());
            chart
                .set_feature_series_data(
                    0,
                    (0..3)
                        .map(|index| FeatureDataPoint {
                            time: index as f64,
                            value: Some(sample_value(kind, index)),
                        })
                        .collect(),
                )
                .unwrap();
            chart.time_scale.set_width(800.0);
            chart.fit_content();
            let frame = chart.build_frame();
            let segment = chart
                .frame_series_segments(0)
                .iter()
                .find(|segment| segment.series_id == Some(0))
                .copied()
                .unwrap_or_else(|| panic!("{kind:?} emitted no retained series segment"));
            assert!(segment.end > segment.start, "{kind:?} emitted no geometry");
            assert!(frame.panes[0].main.len() >= segment.end);
        }
    }

    #[test]
    fn heatmap_emits_every_full_width_price_cell_with_host_shader_colors() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::Heatmap,
            FeatureSeriesOptionsPatch::default(),
        );
        let color = Color::rgb(12, 34, 56);
        chart
            .set_feature_series_data(
                0,
                (0..4)
                    .map(|time| FeatureDataPoint {
                        time: time as f64,
                        value: Some(FeatureValue::Heatmap {
                            cells: (0..3)
                                .map(|cell| HeatmapCell {
                                    low: cell as f64 * 10.0,
                                    high: (cell + 1) as f64 * 10.0,
                                    amount: 50.0,
                                    color: Some(color),
                                })
                                .collect(),
                        }),
                    })
                    .collect(),
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.fit_content();
        let frame = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .copied()
            .unwrap();
        let cells = frame.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                aeris_charts_render::draw_list::Prim::Rect { rect, color: fill }
                    if *fill == color =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(cells.len(), 12);
        assert!(cells.iter().all(|cell| cell.w > 1 && cell.h > 1));
        let time_columns = cells
            .as_chunks::<3>()
            .0
            .iter()
            .map(|group| group[0].x)
            .collect::<Vec<_>>();
        assert_eq!(time_columns.len(), 4);
        assert!(time_columns.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn background_shade_matches_official_bar_geometry() {
        let mut background = ChartEngine::new(800.0, 500.0, 2.0);
        background.configure_feature_series(
            0,
            FeatureSeriesKind::BackgroundShade,
            FeatureSeriesOptionsPatch {
                low_value: Some(0.0),
                high_value: Some(100.0),
                ..FeatureSeriesOptionsPatch::default()
            },
        );
        background
            .set_feature_series_data(
                0,
                vec![
                    FeatureDataPoint {
                        time: 0.0,
                        value: Some(FeatureValue::BackgroundShade { value: 0.0 }),
                    },
                    FeatureDataPoint {
                        time: 1.0,
                        value: Some(FeatureValue::BackgroundShade { value: 50.0 }),
                    },
                    FeatureDataPoint {
                        time: 2.0,
                        value: Some(FeatureValue::BackgroundShade { value: 100.0 }),
                    },
                    FeatureDataPoint {
                        time: 3.0,
                        value: None,
                    },
                    FeatureDataPoint {
                        time: 4.0,
                        value: Some(FeatureValue::BackgroundShade { value: 25.0 }),
                    },
                ],
            )
            .unwrap();
        background.time_scale.set_width(800.0);
        background.fit_content();
        background.build_frame();
        assert!(background.series_base_value(0, 0).is_none());
        assert!(background.panes[0].price_scale.price_range().is_none());
        assert!(background.hit_test_one_series(0, 400.0, 250.0).is_none());

        let line = background.add_series(SeriesKind::Line);
        let times = [0.0, 1.0, 2.0, 3.0, 4.0];
        let values = [0.0, 50.0, 100.0, 75.0, 25.0];
        background
            .set_series_data(line, &times, &values, &values, &values, &values)
            .unwrap();
        background.time_scale.set_width(800.0);
        background.fit_content();
        let frame = background.build_frame();
        let segment = background
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .copied()
            .unwrap();
        let fields = frame.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                aeris_charts_render::draw_list::Prim::Rect { rect, color } => Some((*rect, *color)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            fields.len(),
            4,
            "one full-height strip per non-whitespace bar"
        );
        assert_eq!(
            fields.iter().map(|(_, color)| *color).collect::<Vec<_>>(),
            vec![
                Color::rgb(50, 50, 255),
                Color::rgb(153, 50, 153),
                Color::rgb(255, 50, 50),
                Color::rgb(101, 50, 204),
            ]
        );
        assert!(
            fields
                .iter()
                .all(|(rect, _)| rect.y == 0 && rect.h == 1_000)
        );
        let spacing = background.time_scale.bar_spacing();
        let expected = [0, 1, 2, 4]
            .map(|logical| {
                let x = background.time_scale.index_to_coordinate(logical);
                let left = ((x - spacing / 2.0) * 2.0).round() as i32;
                let right = ((x + spacing / 2.0) * 2.0).round() as i32;
                (left, (right - left).max(1))
            })
            .to_vec();
        assert_eq!(
            fields
                .iter()
                .map(|(rect, _)| (rect.x, rect.w))
                .collect::<Vec<_>>(),
            expected,
            "each value owns exactly its upstream full-bar-width interval"
        );
        assert!(
            fields[2].0.x + fields[2].0.w < fields[3].0.x,
            "whitespace must remain unshaded"
        );
        assert!(background.series_base_value(0, 0).is_none());
    }

    #[test]
    fn background_shade_sparse_rows_stay_time_aligned_after_pan() {
        let mut chart = ChartEngine::new(480.0, 260.0, 2.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::BackgroundShade,
            FeatureSeriesOptionsPatch::default(),
        );
        let values = (0..60)
            .map(|index| match index {
                0 => 0.0,
                1 => 50.0,
                2 => 100.0,
                _ => ((index * 37 + (index % 5) * 11) % 101) as f64,
            })
            .collect::<Vec<_>>();
        chart
            .set_feature_series_data(
                0,
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| FeatureDataPoint {
                        time: index as f64,
                        value: (![9, 31].contains(&index))
                            .then_some(FeatureValue::BackgroundShade { value: *value }),
                    })
                    .collect(),
            )
            .unwrap();
        let line = chart.add_series(SeriesKind::Line);
        let times = (0..60).map(|index| index as f64).collect::<Vec<_>>();
        chart
            .set_series_data(line, &times, &values, &values, &values, &values)
            .unwrap();
        chart.time_scale.set_width(480.0);
        chart.set_visible_logical_range(20.0, 35.0);
        let frame = chart.build_frame();
        let segment = chart
            .frame_series_segments(0)
            .iter()
            .find(|segment| segment.series_id == Some(0))
            .copied()
            .unwrap();
        let colors = frame.panes[0].main[segment.start..segment.end]
            .iter()
            .filter_map(|primitive| match primitive {
                aeris_charts_render::draw_list::Prim::Rect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected = (20..=35)
            .filter(|index| *index != 31)
            .map(|index| {
                let amount = values[index] / 100.0;
                Color::rgb(
                    (50.0 + 205.0 * amount).round() as u8,
                    50,
                    (255.0 - 205.0 * amount).round() as u8,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(colors, expected);
    }

    #[test]
    fn background_shade_extrapolates_channels_like_the_reference_css_color() {
        let mut chart = ChartEngine::new(200.0, 100.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::BackgroundShade,
            FeatureSeriesOptionsPatch {
                low_color: Some(Color::rgb(100, 100, 100)),
                high_color: Some(Color::rgb(150, 150, 150)),
                low_value: Some(0.0),
                high_value: Some(100.0),
                ..FeatureSeriesOptionsPatch::default()
            },
        );
        chart
            .set_feature_series_data(
                0,
                vec![FeatureDataPoint {
                    time: 0.0,
                    value: Some(FeatureValue::BackgroundShade { value: 200.0 }),
                }],
            )
            .unwrap();
        chart.time_scale.set_width(200.0);
        chart.fit_content();
        assert!(chart.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, aeris_charts_render::draw_list::Prim::Rect { color, .. }
                if *color == Color::rgb(200, 200, 200))
        }));
    }

    #[test]
    fn official_feature_defaults_and_shader_colors_are_preserved() {
        let mut heatmap = ChartEngine::new(800.0, 500.0, 1.0);
        heatmap.configure_feature_series(
            0,
            FeatureSeriesKind::Heatmap,
            FeatureSeriesOptionsPatch::default(),
        );
        heatmap
            .set_feature_series_data(
                0,
                (0..2)
                    .map(|time| FeatureDataPoint {
                        time: time as f64,
                        value: Some(FeatureValue::Heatmap {
                            cells: vec![HeatmapCell {
                                low: 10.0,
                                high: 11.0,
                                amount: 0.5,
                                color: (time == 1).then_some(Color::rgb(12, 34, 56)),
                            }],
                        }),
                    })
                    .collect(),
            )
            .unwrap();
        heatmap.time_scale.set_width(800.0);
        heatmap.fit_content();
        assert!(heatmap.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, aeris_charts_render::draw_list::Prim::Rect { color, .. }
                if *color == Color::rgba(0, 101, 1, 153))
        }));
        assert!(heatmap.build_frame().panes[0].main.iter().any(|primitive| {
            matches!(primitive, aeris_charts_render::draw_list::Prim::Rect { color, .. }
                if *color == Color::rgb(12, 34, 56))
        }));

        let mut background = ChartEngine::new(800.0, 500.0, 1.0);
        background.configure_feature_series(
            0,
            FeatureSeriesKind::BackgroundShade,
            FeatureSeriesOptionsPatch {
                opacity: Some(0.01),
                ..FeatureSeriesOptionsPatch::default()
            },
        );
        background
            .set_feature_series_data(
                0,
                (0..2)
                    .map(|time| FeatureDataPoint {
                        time: time as f64,
                        value: Some(FeatureValue::BackgroundShade { value: 50.0 }),
                    })
                    .collect(),
            )
            .unwrap();
        background.time_scale.set_width(800.0);
        background.fit_content();
        assert!(
            background.build_frame().panes[0]
                .main
                .iter()
                .any(|primitive| {
                    matches!(primitive, aeris_charts_render::draw_list::Prim::Rect { color, .. }
                if color.a() == 255 && color.r() == 153 && color.g() == 50 && color.b() == 153)
                })
        );
    }

    #[test]
    fn pretty_histogram_paints_the_official_one_bar_right_edge_overscan() {
        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::PrettyHistogram,
            FeatureSeriesOptionsPatch::default(),
        );
        chart
            .set_feature_series_data(
                0,
                (0..8)
                    .map(|time| FeatureDataPoint {
                        time: time as f64,
                        value: Some(FeatureValue::PrettyHistogram {
                            value: 10.0 + time as f64,
                            color: None,
                        }),
                    })
                    .collect(),
            )
            .unwrap();
        chart.time_scale.set_width(800.0);
        chart.set_visible_logical_range(2.0, 4.0);
        let frame = chart.build_frame();
        let columns = frame.panes[0]
            .main
            .iter()
            .filter(|primitive| {
                matches!(primitive, aeris_charts_render::draw_list::Prim::RoundRect {
                    fill,
                    ..
                } if *fill == Color::rgb(0xd6, 0x38, 0x64))
            })
            .count();
        assert_eq!(
            columns, 4,
            "strict bars 2..=4 plus the official right overscan"
        );
    }

    #[test]
    fn invalid_feature_timestamps_reject_atomically() {
        use aeris_charts_core::model::data_validation::{
            MAX_TIMESTAMP, MIN_TIMESTAMP, TimestampErrorCategory,
        };

        let mut chart = ChartEngine::new(800.0, 500.0, 1.0);
        chart.configure_feature_series(
            0,
            FeatureSeriesKind::PrettyHistogram,
            FeatureSeriesOptionsPatch::default(),
        );
        let point = |time| FeatureDataPoint {
            time,
            value: Some(FeatureValue::PrettyHistogram {
                value: time,
                color: None,
            }),
        };
        chart
            .set_feature_series_data(
                0,
                vec![point(MIN_TIMESTAMP as f64), point(MAX_TIMESTAMP as f64)],
            )
            .unwrap();
        let before = chart.feature_series_data(0).unwrap();

        for invalid in [
            f64::NAN,
            1.5,
            MAX_TIMESTAMP as f64 + 1.0,
            1_725_000_000_000.0,
        ] {
            let error = chart
                .set_feature_series_data(0, vec![point(10.0), point(invalid)])
                .unwrap_err();
            assert!(matches!(
                error,
                ValidationError::InvalidTimestamp { index: 1, .. }
            ));
            assert_eq!(chart.feature_series_data(0).unwrap(), before);
        }

        let error = chart.update_feature_series_data(0, point(2.5)).unwrap_err();
        assert!(matches!(
            error,
            ValidationError::InvalidTimestamp {
                error: aeris_charts_core::model::data_validation::TimestampError {
                    category: TimestampErrorCategory::Fractional,
                    ..
                },
                ..
            }
        ));
        assert_eq!(chart.feature_series_data(0).unwrap(), before);
    }
}
