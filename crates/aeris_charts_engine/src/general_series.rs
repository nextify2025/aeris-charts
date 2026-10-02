use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;

use aeris_charts_core::scale::general_scale::{BandScale, LinearScale, PointScale};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{LineStyle, LineType};
use aeris_charts_render::line::{expand_band_into, expand_line_into, LinePoint};

use crate::general_axes::NumericAxisScale;
use crate::{
    AxisDimension, ChartEngine, ChartError, ErrorCode, GeneralAxisDomain, GeneralDatasetId,
    GeneralRowIdentity, GeneralScaleType, GeneralXKind, GeneralXyInput, HorizontalDomain, PaneId,
};

pub const MAX_GENERAL_SERIES: usize = 1_024;
pub const MAX_GENERAL_REFERENCES: usize = 1_024;
pub const MAX_GENERAL_SERIES_TITLE_BYTES: usize = 4_096;
pub const MAX_GENERAL_SERIES_COLOR_BYTES: usize = 256;
pub const MAX_GENERAL_SERIES_GROUP_ID_BYTES: usize = 128;
pub const MAX_GENERAL_SERIES_STACK_ID_BYTES: usize = 128;
pub const MAX_GENERAL_ACCESSIBILITY_ITEMS: usize = 512;
pub const MAX_GENERAL_BRUSH_ITEMS: usize = 4_096;
pub const MAX_GENERAL_SHARED_TOOLTIP_ITEMS: usize = 512;
pub const MIN_GENERAL_POINT_RADIUS: f64 = 1.0;
pub const MAX_GENERAL_POINT_RADIUS: f64 = 64.0;
pub const DEFAULT_GENERAL_FILL_OPACITY: f64 = 72.0 / 255.0;
const SCATTER_GRID_BASE_CELL_CSS: f64 = 32.0;
const MAX_SCATTER_GRID_CELLS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GeneralSeriesKind {
    XyLine,
    XyArea,
    RangeArea,
    RangeBar,
    ErrorBar,
    Column,
    HorizontalBar,
    BoxPlot,
    HeatmapGrid,
    Scatter,
    Bubble,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum GeneralStackMode {
    #[default]
    Normal,
    Percent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GeneralLineStyle {
    #[default]
    Solid,
    Dotted,
    Dashed,
}

impl GeneralLineStyle {
    pub(crate) fn render_style(self) -> LineStyle {
        match self {
            Self::Solid => LineStyle::Solid,
            Self::Dotted => LineStyle::Dotted,
            Self::Dashed => LineStyle::Dashed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GeneralInterpolation {
    #[default]
    Linear,
    Step,
    Curved,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GeneralPointSymbol {
    #[default]
    Circle,
    Square,
    Diamond,
    Triangle,
}

impl GeneralInterpolation {
    pub(crate) fn render_type(self) -> LineType {
        match self {
            Self::Linear => LineType::Simple,
            Self::Step => LineType::WithSteps,
            Self::Curved => LineType::Curved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GeneralSeriesId(NonZeroU32);

impl GeneralSeriesId {
    pub fn get(self) -> u32 {
        self.0.get()
    }

    #[doc(hidden)]
    pub fn from_raw(value: u32) -> Option<Self> {
        NonZeroU32::new(value).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralSeriesOptions {
    pub kind: GeneralSeriesKind,
    pub pane: usize,
    pub dataset: GeneralDatasetId,
    pub x_axis_id: String,
    pub y_axis_id: String,
    pub visible: bool,
    pub title: String,
    pub color: Option<String>,
    pub point_radius: f64,
    pub point_markers: bool,
    pub point_symbol: GeneralPointSymbol,
    pub line_width: f64,
    pub line_style: GeneralLineStyle,
    pub interpolation: GeneralInterpolation,
    pub connect_missing: bool,
    pub fill_opacity: f64,
    pub baseline_value: Option<f64>,
    pub data_labels: bool,
    pub group_id: Option<String>,
    pub stack_id: Option<String>,
    pub stack_mode: GeneralStackMode,
}

impl GeneralSeriesOptions {
    pub fn xy_line(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::XyLine,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn xy_area(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::XyArea,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn range_area(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::RangeArea,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn range_bar(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        let mut options = Self::range_area(pane, dataset, x_axis_id, y_axis_id);
        options.kind = GeneralSeriesKind::RangeBar;
        options
    }

    pub fn error_bar(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::ErrorBar,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 4.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn column(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::Column,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn horizontal_bar(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::HorizontalBar,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn box_plot(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::BoxPlot,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn heatmap_grid(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::HeatmapGrid,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn scatter(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::Scatter,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }

    pub fn bubble(
        pane: usize,
        dataset: GeneralDatasetId,
        x_axis_id: impl Into<String>,
        y_axis_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: GeneralSeriesKind::Bubble,
            pane,
            dataset,
            x_axis_id: x_axis_id.into(),
            y_axis_id: y_axis_id.into(),
            visible: true,
            title: String::new(),
            color: None,
            point_radius: 3.0,
            point_markers: false,
            point_symbol: GeneralPointSymbol::Circle,
            line_width: 2.0,
            line_style: GeneralLineStyle::Solid,
            interpolation: GeneralInterpolation::Linear,
            connect_missing: false,
            fill_opacity: DEFAULT_GENERAL_FILL_OPACITY,
            baseline_value: None,
            data_labels: false,
            group_id: None,
            stack_id: None,
            stack_mode: GeneralStackMode::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralSeries {
    id: GeneralSeriesId,
    kind: GeneralSeriesKind,
    pane_id: PaneId,
    dataset: GeneralDatasetId,
    x_axis_id: String,
    y_axis_id: String,
    visible: bool,
    title: String,
    color: Option<String>,
    point_radius: f64,
    point_markers: bool,
    point_symbol: GeneralPointSymbol,
    line_width: f64,
    line_style: GeneralLineStyle,
    interpolation: GeneralInterpolation,
    connect_missing: bool,
    fill_opacity: f64,
    baseline_value: Option<f64>,
    data_labels: bool,
    group_id: Option<String>,
    stack_id: Option<String>,
    stack_mode: GeneralStackMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralColumnGeometry {
    pub(crate) row: usize,
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) top: f64,
    pub(crate) bottom: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralScatterGeometry {
    pub(crate) row: usize,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) radius: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralLinePointGeometry {
    pub(crate) row: usize,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) starts_new_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralRangePointGeometry {
    pub(crate) row: usize,
    pub(crate) x: f64,
    pub(crate) low_y: f64,
    pub(crate) high_y: f64,
    pub(crate) starts_new_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum GeneralStackXKey {
    Numeric(u64),
    Temporal(i64),
    Category(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralErrorBarGeometry {
    pub(crate) row: usize,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) x_low: Option<f64>,
    pub(crate) x_high: Option<f64>,
    pub(crate) y_low: Option<f64>,
    pub(crate) y_high: Option<f64>,
    pub(crate) cap_half_size: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralBoxPlotGeometry {
    pub(crate) row: usize,
    pub(crate) center_x: f64,
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) min_y: f64,
    pub(crate) q1_y: f64,
    pub(crate) median_y: f64,
    pub(crate) q3_y: f64,
    pub(crate) max_y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GeneralHeatmapGeometry {
    pub(crate) row: usize,
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) top: f64,
    pub(crate) bottom: f64,
    pub(crate) value: f64,
    pub(crate) intensity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScatterGeometryKey {
    dataset_generation: u64,
    plot_width: u64,
    plot_y: u64,
    plot_height: u64,
    x_domain: [u64; 2],
    y_domain: [u64; 2],
    x_scale: GeneralScaleType,
    y_scale: GeneralScaleType,
    x_reverse: bool,
    y_reverse: bool,
    radius: u64,
}

#[derive(Clone, Copy)]
struct ScatterGeometryContext {
    key: ScatterGeometryKey,
    x_scale: NumericAxisScale,
    y_scale: NumericAxisScale,
    plot_width: f64,
    plot_y: f64,
    plot_bottom: f64,
}

/// Memoized O(rows) contribution of one series to one automatic numeric axis domain. It is valid
/// while the series keeps the same dataset generation, kind, and stacking; hit tests and frame
/// construction resolve auto domains repeatedly, so rescanning every row per call is not bounded.
#[derive(Clone, Copy)]
struct AxisScanEntry {
    dimension: AxisDimension,
    scale: GeneralScaleType,
    dataset: GeneralDatasetId,
    generation: u64,
    kind: GeneralSeriesKind,
    stacked: bool,
    value: (Option<(f64, f64)>, bool),
}

/// A series binds one X and one Y axis; a small bound still covers scale-type changes.
const MAX_AXIS_SCANS_PER_SERIES: usize = 4;

struct ScatterSpatialIndex {
    key: ScatterGeometryKey,
    origin_y: f64,
    cell_size: f64,
    columns: usize,
    rows: usize,
    offsets: Vec<usize>,
    point_indices: Vec<u32>,
    points: Vec<GeneralScatterGeometry>,
}

impl ScatterSpatialIndex {
    fn new(
        key: ScatterGeometryKey,
        origin_y: f64,
        plot_width: f64,
        plot_height: f64,
        points: Vec<GeneralScatterGeometry>,
    ) -> Self {
        let mut cell_size = SCATTER_GRID_BASE_CELL_CSS;
        let (mut columns, mut rows) = grid_dimensions(plot_width, plot_height, cell_size);
        while columns.saturating_mul(rows) > MAX_SCATTER_GRID_CELLS {
            cell_size *= 2.0;
            (columns, rows) = grid_dimensions(plot_width, plot_height, cell_size);
        }
        let cell_count = columns.saturating_mul(rows).max(1);
        let mut counts = vec![0usize; cell_count];
        for point in &points {
            let cell = scatter_cell(point.x, point.y, origin_y, cell_size, columns, rows);
            counts[cell] += 1;
        }
        let mut offsets = vec![0usize; cell_count + 1];
        for (index, count) in counts.into_iter().enumerate() {
            offsets[index + 1] = offsets[index] + count;
        }
        let mut cursors = offsets[..cell_count].to_vec();
        let mut point_indices = vec![0u32; points.len()];
        for (point_index, point) in points.iter().enumerate() {
            let cell = scatter_cell(point.x, point.y, origin_y, cell_size, columns, rows);
            let slot = cursors[cell];
            point_indices[slot] = u32::try_from(point_index)
                .expect("general scatter rows stay below the u32 index ceiling");
            cursors[cell] += 1;
        }
        Self {
            key,
            origin_y,
            cell_size,
            columns,
            rows,
            offsets,
            point_indices,
            points,
        }
    }

    fn estimated_bytes(&self) -> usize {
        self.offsets.capacity() * std::mem::size_of::<usize>()
            + self.point_indices.capacity() * std::mem::size_of::<u32>()
            + self.points.capacity() * std::mem::size_of::<GeneralScatterGeometry>()
    }

    fn visit_candidates<F>(&self, x: f64, y: f64, expansion: f64, mut visit: F)
    where
        F: FnMut(GeneralScatterGeometry),
    {
        let Some((min_col, max_col)) = grid_query_range(
            x - expansion,
            x + expansion,
            0.0,
            self.cell_size,
            self.columns,
        ) else {
            return;
        };
        let Some((min_row, max_row)) = grid_query_range(
            y - expansion,
            y + expansion,
            self.origin_y,
            self.cell_size,
            self.rows,
        ) else {
            return;
        };
        for row in min_row..=max_row {
            for col in min_col..=max_col {
                let cell = row * self.columns + col;
                for &point_index in &self.point_indices[self.offsets[cell]..self.offsets[cell + 1]]
                {
                    if let Some(point) = self.points.get(point_index as usize) {
                        visit(*point);
                    }
                }
            }
        }
    }
}

fn grid_dimensions(width: f64, height: f64, cell_size: f64) -> (usize, usize) {
    let columns = (width.max(1.0) / cell_size).ceil().max(1.0) as usize;
    let rows = (height.max(1.0) / cell_size).ceil().max(1.0) as usize;
    (columns, rows)
}

fn scatter_cell(
    x: f64,
    y: f64,
    origin_y: f64,
    cell_size: f64,
    columns: usize,
    rows: usize,
) -> usize {
    let col = ((x / cell_size).floor() as isize).clamp(0, columns as isize - 1) as usize;
    let row = (((y - origin_y) / cell_size).floor() as isize).clamp(0, rows as isize - 1) as usize;
    row * columns + col
}

fn grid_query_range(
    from: f64,
    to: f64,
    range_start: f64,
    cell_size: f64,
    count: usize,
) -> Option<(usize, usize)> {
    let first = ((from - range_start) / cell_size).floor() as isize;
    let last = ((to - range_start) / cell_size).floor() as isize;
    if last < 0 || first >= count as isize {
        return None;
    }
    Some((
        first.clamp(0, count as isize - 1) as usize,
        last.clamp(0, count as isize - 1) as usize,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GeneralHitMode {
    Exact,
    Nearest { max_distance: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralSeriesHit {
    pub series: GeneralSeriesId,
    pub row: usize,
    pub row_id: GeneralRowIdentity,
    pub distance: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralInteractionTarget {
    series: GeneralSeriesId,
    row: usize,
    row_id: GeneralRowIdentity,
    distance: f64,
}

impl From<&GeneralSeriesHit> for GeneralInteractionTarget {
    fn from(hit: &GeneralSeriesHit) -> Self {
        Self {
            series: hit.series,
            row: hit.row,
            row_id: hit.row_id.clone(),
            distance: hit.distance,
        }
    }
}

impl GeneralInteractionTarget {
    fn hit(&self) -> GeneralSeriesHit {
        GeneralSeriesHit {
            series: self.series,
            row: self.row,
            row_id: self.row_id.clone(),
            distance: self.distance,
        }
    }

    fn estimated_bytes(&self) -> usize {
        match &self.row_id {
            GeneralRowIdentity::Explicit(crate::GeneralRowId::Text(value)) => value.capacity(),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralTooltipSnapshot {
    pub series: GeneralSeriesId,
    pub row: usize,
    pub row_id: GeneralRowIdentity,
    pub x_label: String,
    pub y_label: Option<String>,
    pub label: Option<String>,
    pub value: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub x_low: Option<f64>,
    pub x_high: Option<f64>,
    pub q1: Option<f64>,
    pub q3: Option<f64>,
    pub size: Option<f64>,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralSharedTooltipSnapshot {
    pub pane: usize,
    pub anchor_series: GeneralSeriesId,
    pub anchor_row: usize,
    pub items: Vec<GeneralTooltipSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralAccessibilityItem {
    pub row: usize,
    pub row_id: GeneralRowIdentity,
    pub x_label: String,
    pub y_label: Option<String>,
    pub label: Option<String>,
    pub value: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub x_low: Option<f64>,
    pub x_high: Option<f64>,
    pub q1: Option<f64>,
    pub q3: Option<f64>,
    pub size: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralAccessibilitySnapshot {
    pub series: GeneralSeriesId,
    pub title: String,
    pub total_rows: usize,
    pub offset: usize,
    pub items: Vec<GeneralAccessibilityItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralLegendItem {
    pub series: GeneralSeriesId,
    pub pane: usize,
    pub kind: GeneralSeriesKind,
    pub title: String,
    pub color: Option<String>,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralLegendSnapshot {
    pub items: Vec<GeneralLegendItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeneralBrushRange {
    Numeric([f64; 2]),
    Temporal([i64; 2]),
    Category([String; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralBrushSnapshot {
    pub pane: usize,
    pub axis_id: String,
    pub dimension: AxisDimension,
    pub range: GeneralBrushRange,
    pub items: Vec<GeneralSeriesHit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GeneralReferenceId(NonZeroU32);

impl GeneralReferenceId {
    pub fn get(self) -> u32 {
        self.0.get()
    }

    #[doc(hidden)]
    pub fn from_raw(value: u32) -> Option<Self> {
        NonZeroU32::new(value).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum GeneralReferenceValue {
    Numeric(f64),
    Temporal(i64),
    Category(String),
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GeneralReferenceOptions {
    Line {
        pane: usize,
        axis_id: String,
        value: GeneralReferenceValue,
        #[serde(default)]
        color: Option<String>,
        #[serde(default = "default_reference_line_width")]
        line_width: f64,
        #[serde(default)]
        extend_domain: bool,
    },
    Dot {
        pane: usize,
        x_axis_id: String,
        y_axis_id: String,
        x: GeneralReferenceValue,
        y: GeneralReferenceValue,
        #[serde(default)]
        color: Option<String>,
        #[serde(default = "default_reference_dot_radius")]
        radius: f64,
        #[serde(default)]
        extend_domain: bool,
    },
    Region {
        pane: usize,
        x_axis_id: String,
        y_axis_id: String,
        x_from: GeneralReferenceValue,
        x_to: GeneralReferenceValue,
        y_from: GeneralReferenceValue,
        y_to: GeneralReferenceValue,
        #[serde(default)]
        fill_color: Option<String>,
        #[serde(default)]
        extend_domain: bool,
    },
}

fn default_reference_line_width() -> f64 {
    1.0
}

fn default_reference_dot_radius() -> f64 {
    4.0
}

impl GeneralReferenceOptions {
    pub fn pane(&self) -> usize {
        match self {
            Self::Line { pane, .. } | Self::Dot { pane, .. } | Self::Region { pane, .. } => *pane,
        }
    }

    fn set_pane(&mut self, pane: usize) {
        match self {
            Self::Line { pane: value, .. }
            | Self::Dot { pane: value, .. }
            | Self::Region { pane: value, .. } => *value = pane,
        }
    }

    pub fn extend_domain(&self) -> bool {
        match self {
            Self::Line { extend_domain, .. }
            | Self::Dot { extend_domain, .. }
            | Self::Region { extend_domain, .. } => *extend_domain,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeneralReference {
    id: GeneralReferenceId,
    pane_id: PaneId,
    options: GeneralReferenceOptions,
}

impl GeneralReference {
    pub fn id(&self) -> GeneralReferenceId {
        self.id
    }

    pub fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    pub fn options(&self) -> &GeneralReferenceOptions {
        &self.options
    }

    fn estimated_bytes(&self) -> usize {
        fn value_bytes(value: &GeneralReferenceValue) -> usize {
            match value {
                GeneralReferenceValue::Category(value) => value.capacity(),
                GeneralReferenceValue::Numeric(_) | GeneralReferenceValue::Temporal(_) => 0,
            }
        }
        match &self.options {
            GeneralReferenceOptions::Line {
                axis_id,
                value,
                color,
                ..
            } => {
                axis_id.capacity() + value_bytes(value) + color.as_ref().map_or(0, String::capacity)
            }
            GeneralReferenceOptions::Dot {
                x_axis_id,
                y_axis_id,
                x,
                y,
                color,
                ..
            } => {
                x_axis_id.capacity()
                    + y_axis_id.capacity()
                    + value_bytes(x)
                    + value_bytes(y)
                    + color.as_ref().map_or(0, String::capacity)
            }
            GeneralReferenceOptions::Region {
                x_axis_id,
                y_axis_id,
                x_from,
                x_to,
                y_from,
                y_to,
                fill_color,
                ..
            } => {
                x_axis_id.capacity()
                    + y_axis_id.capacity()
                    + value_bytes(x_from)
                    + value_bytes(x_to)
                    + value_bytes(y_from)
                    + value_bytes(y_to)
                    + fill_color.as_ref().map_or(0, String::capacity)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct GeneralBrushSelection {
    pane_id: PaneId,
    axis_id: String,
    dimension: AxisDimension,
    range: GeneralBrushRange,
}

impl GeneralSeries {
    pub fn id(&self) -> GeneralSeriesId {
        self.id
    }

    pub fn kind(&self) -> GeneralSeriesKind {
        self.kind
    }

    pub fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    pub fn dataset(&self) -> GeneralDatasetId {
        self.dataset
    }

    pub fn x_axis_id(&self) -> &str {
        &self.x_axis_id
    }

    pub fn y_axis_id(&self) -> &str {
        &self.y_axis_id
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    pub fn point_radius(&self) -> f64 {
        self.point_radius
    }

    pub fn point_markers(&self) -> bool {
        self.point_markers
    }

    pub fn point_symbol(&self) -> GeneralPointSymbol {
        self.point_symbol
    }

    pub fn line_width(&self) -> f64 {
        self.line_width
    }

    pub fn line_style(&self) -> GeneralLineStyle {
        self.line_style
    }

    pub fn interpolation(&self) -> GeneralInterpolation {
        self.interpolation
    }

    pub fn connect_missing(&self) -> bool {
        self.connect_missing
    }

    pub fn fill_opacity(&self) -> f64 {
        self.fill_opacity
    }

    pub fn baseline_value(&self) -> Option<f64> {
        self.baseline_value
    }

    pub fn data_labels(&self) -> bool {
        self.data_labels
    }

    pub fn group_id(&self) -> Option<&str> {
        self.group_id.as_deref()
    }

    pub fn stack_id(&self) -> Option<&str> {
        self.stack_id.as_deref()
    }

    pub fn stack_mode(&self) -> GeneralStackMode {
        self.stack_mode
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.x_axis_id.capacity()
            + self.y_axis_id.capacity()
            + self.title.capacity()
            + self.color.as_ref().map_or(0, String::capacity)
            + self.group_id.as_ref().map_or(0, String::capacity)
            + self.stack_id.as_ref().map_or(0, String::capacity)
    }
}

pub(crate) struct GeneralSeriesRegistry {
    series: Vec<GeneralSeries>,
    next_id: u32,
    references: Vec<GeneralReference>,
    next_reference_id: u32,
    scatter_spatial: RefCell<HashMap<GeneralSeriesId, ScatterSpatialIndex>>,
    axis_scans: RefCell<HashMap<GeneralSeriesId, Vec<AxisScanEntry>>>,
    hovered: Option<GeneralInteractionTarget>,
    selected: Option<GeneralInteractionTarget>,
    accessibility_focused: Option<GeneralInteractionTarget>,
    brush: Option<GeneralBrushSelection>,
}

impl GeneralSeriesRegistry {
    pub(crate) fn new() -> Self {
        Self {
            series: Vec::new(),
            next_id: 1,
            references: Vec::new(),
            next_reference_id: 1,
            scatter_spatial: RefCell::new(HashMap::new()),
            axis_scans: RefCell::new(HashMap::new()),
            hovered: None,
            selected: None,
            accessibility_focused: None,
            brush: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.series.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.series.is_empty() && self.references.is_empty() && self.brush.is_none()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &GeneralSeries> {
        self.series.iter()
    }

    pub(crate) fn get(&self, id: GeneralSeriesId) -> Option<&GeneralSeries> {
        self.series.iter().find(|series| series.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: GeneralSeriesId) -> Option<&mut GeneralSeries> {
        self.series.iter_mut().find(|series| series.id == id)
    }

    fn ids(&self, pane_id: Option<PaneId>) -> Vec<GeneralSeriesId> {
        self.series
            .iter()
            .filter(|series| pane_id.is_none_or(|pane_id| series.pane_id == pane_id))
            .map(GeneralSeries::id)
            .collect()
    }

    fn set_order(&mut self, pane_id: Option<PaneId>, ids: &[GeneralSeriesId]) -> bool {
        let positions = self
            .series
            .iter()
            .enumerate()
            .filter(|(_, series)| pane_id.is_none_or(|pane_id| series.pane_id == pane_id))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if positions.len() != ids.len()
            || ids.iter().copied().collect::<HashSet<_>>().len() != ids.len()
        {
            return false;
        }
        let mut current = positions
            .iter()
            .map(|&index| {
                let series = self.series[index].clone();
                (series.id, series)
            })
            .collect::<HashMap<_, _>>();
        if ids.iter().any(|id| !current.contains_key(id)) {
            return false;
        }

        let ordered = ids
            .iter()
            .map(|id| {
                current
                    .remove(id)
                    .expect("a validated general series order contains only live ids")
            })
            .collect::<Vec<_>>();
        for (index, series) in positions.into_iter().zip(ordered) {
            self.series[index] = series;
        }
        true
    }

    pub(crate) fn reference_iter(&self) -> impl Iterator<Item = &GeneralReference> {
        self.references.iter()
    }

    pub(crate) fn reference_get(&self, id: GeneralReferenceId) -> Option<&GeneralReference> {
        self.references.iter().find(|reference| reference.id == id)
    }

    pub(crate) fn reference_insert(
        &mut self,
        pane_id: PaneId,
        options: GeneralReferenceOptions,
    ) -> Result<GeneralReferenceId, ChartError> {
        if self.references.len() >= MAX_GENERAL_REFERENCES {
            return Err(resource(format!(
                "a chart supports at most {MAX_GENERAL_REFERENCES} general references"
            )));
        }
        let id = NonZeroU32::new(self.next_reference_id)
            .map(GeneralReferenceId)
            .ok_or_else(|| resource("general reference identity space is exhausted"))?;
        self.next_reference_id = self
            .next_reference_id
            .checked_add(1)
            .ok_or_else(|| resource("general reference identity space is exhausted"))?;
        self.references.push(GeneralReference {
            id,
            pane_id,
            options,
        });
        Ok(id)
    }

    pub(crate) fn reference_remove(&mut self, id: GeneralReferenceId) -> bool {
        let Some(index) = self
            .references
            .iter()
            .position(|reference| reference.id == id)
        else {
            return false;
        };
        self.references.remove(index);
        true
    }

    pub(crate) fn insert(
        &mut self,
        pane_id: PaneId,
        options: GeneralSeriesOptions,
    ) -> Result<GeneralSeriesId, ChartError> {
        if self.series.len() >= MAX_GENERAL_SERIES {
            return Err(resource(format!(
                "a chart supports at most {MAX_GENERAL_SERIES} general series"
            )));
        }
        validate_presentation(&options)?;
        let id = NonZeroU32::new(self.next_id)
            .map(GeneralSeriesId)
            .ok_or_else(|| resource("general series identity space is exhausted"))?;
        let next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| resource("general series identity space is exhausted"))?;
        self.series.push(GeneralSeries {
            id,
            kind: options.kind,
            pane_id,
            dataset: options.dataset,
            x_axis_id: options.x_axis_id,
            y_axis_id: options.y_axis_id,
            visible: options.visible,
            title: options.title,
            color: options.color,
            point_radius: options.point_radius,
            point_markers: options.point_markers,
            point_symbol: options.point_symbol,
            line_width: options.line_width,
            line_style: options.line_style,
            interpolation: options.interpolation,
            connect_missing: options.connect_missing,
            fill_opacity: options.fill_opacity,
            baseline_value: options.baseline_value,
            data_labels: options.data_labels,
            group_id: options.group_id,
            stack_id: options.stack_id,
            stack_mode: options.stack_mode,
        });
        self.next_id = next_id;
        Ok(id)
    }

    pub(crate) fn remove(&mut self, id: GeneralSeriesId) -> bool {
        let Some(index) = self.series.iter().position(|series| series.id == id) else {
            return false;
        };
        self.series.remove(index);
        self.scatter_spatial.get_mut().remove(&id);
        self.axis_scans.get_mut().remove(&id);
        if self
            .hovered
            .as_ref()
            .is_some_and(|target| target.series == id)
        {
            self.hovered = None;
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|target| target.series == id)
        {
            self.selected = None;
        }
        if self
            .accessibility_focused
            .as_ref()
            .is_some_and(|target| target.series == id)
        {
            self.accessibility_focused = None;
        }
        true
    }

    pub(crate) fn uses_axis(&self, id: &str) -> bool {
        self.series
            .iter()
            .any(|series| series.x_axis_id == id || series.y_axis_id == id)
            || self
                .references
                .iter()
                .any(|reference| match &reference.options {
                    GeneralReferenceOptions::Line { axis_id, .. } => axis_id == id,
                    GeneralReferenceOptions::Dot {
                        x_axis_id,
                        y_axis_id,
                        ..
                    }
                    | GeneralReferenceOptions::Region {
                        x_axis_id,
                        y_axis_id,
                        ..
                    } => x_axis_id == id || y_axis_id == id,
                })
            || self.brush.as_ref().is_some_and(|brush| brush.axis_id == id)
    }

    pub(crate) fn uses_dataset(&self, id: GeneralDatasetId) -> bool {
        self.series.iter().any(|series| series.dataset == id)
    }

    pub(crate) fn uses_pane(&self, pane_id: PaneId) -> bool {
        self.series.iter().any(|series| series.pane_id == pane_id)
            || self
                .references
                .iter()
                .any(|reference| reference.pane_id == pane_id)
            || self
                .brush
                .as_ref()
                .is_some_and(|brush| brush.pane_id == pane_id)
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        let scatter_bytes = self.scatter_spatial.try_borrow().map_or(0, |cache| {
            cache.capacity()
                * (std::mem::size_of::<GeneralSeriesId>()
                    + std::mem::size_of::<ScatterSpatialIndex>())
                + cache
                    .values()
                    .map(ScatterSpatialIndex::estimated_bytes)
                    .sum::<usize>()
        });
        let axis_scan_bytes = self.axis_scans.try_borrow().map_or(0, |cache| {
            cache.capacity() * std::mem::size_of::<(GeneralSeriesId, Vec<AxisScanEntry>)>()
                + cache
                    .values()
                    .map(|entries| entries.capacity() * std::mem::size_of::<AxisScanEntry>())
                    .sum::<usize>()
        });
        let brush_bytes = self.brush.as_ref().map_or(0, |brush| {
            brush.axis_id.capacity()
                + match &brush.range {
                    GeneralBrushRange::Category(values) => {
                        values[0].capacity() + values[1].capacity()
                    }
                    GeneralBrushRange::Numeric(_) | GeneralBrushRange::Temporal(_) => 0,
                }
        });
        self.series.capacity() * std::mem::size_of::<GeneralSeries>()
            + self
                .series
                .iter()
                .map(GeneralSeries::estimated_bytes)
                .sum::<usize>()
            + scatter_bytes
            + axis_scan_bytes
            + self.references.capacity() * std::mem::size_of::<GeneralReference>()
            + self
                .references
                .iter()
                .map(GeneralReference::estimated_bytes)
                .sum::<usize>()
            + self
                .hovered
                .as_ref()
                .map_or(0, GeneralInteractionTarget::estimated_bytes)
            + self
                .selected
                .as_ref()
                .map_or(0, GeneralInteractionTarget::estimated_bytes)
            + self
                .accessibility_focused
                .as_ref()
                .map_or(0, GeneralInteractionTarget::estimated_bytes)
            + brush_bytes
    }
}

impl Default for GeneralSeriesRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartEngine {
    #[doc(hidden)]
    pub fn add_general_series(
        &mut self,
        options: GeneralSeriesOptions,
    ) -> Result<GeneralSeriesId, ChartError> {
        let pane_id = self
            .pane_stable_id(options.pane)
            .ok_or_else(|| invalid("general series references a stale pane"))?;
        let pane_domain = self
            .pane_horizontal_domain(options.pane)
            .ok_or_else(|| invalid("general series references a stale pane"))?;
        let dataset = self.general_dataset(options.dataset).ok_or_else(|| {
            ChartError::new(ErrorCode::InvalidHandle, "general dataset handle is stale")
        })?;
        let dataset_kind = dataset.x_kind();
        let x_axis = self
            .general_axis(&options.x_axis_id)
            .cloned()
            .ok_or_else(|| invalid("general series X axis does not exist"))?;
        let y_axis = self
            .general_axis(&options.y_axis_id)
            .cloned()
            .ok_or_else(|| invalid("general series Y axis does not exist"))?;

        if x_axis.pane_id() != pane_id || y_axis.pane_id() != pane_id {
            return Err(invalid("general series axes must belong to its pane"));
        }
        if x_axis.dimension() != AxisDimension::X || y_axis.dimension() != AxisDimension::Y {
            return Err(invalid("general series axis dimensions are incompatible"));
        }
        match options.kind {
            GeneralSeriesKind::XyLine
            | GeneralSeriesKind::XyArea
            | GeneralSeriesKind::RangeArea => {
                let compatible_x = matches!(
                    (pane_domain, dataset_kind, x_axis.scale()),
                    (
                        HorizontalDomain::Continuous { .. },
                        GeneralXKind::Numeric,
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    ) | (
                        HorizontalDomain::Temporal,
                        GeneralXKind::Temporal,
                        GeneralScaleType::Temporal
                    ) | (
                        HorizontalDomain::Category {
                            scale: crate::CategoryScaleType::Band
                        },
                        GeneralXKind::Category,
                        GeneralScaleType::Band
                    ) | (
                        HorizontalDomain::Category {
                            scale: crate::CategoryScaleType::Point
                        },
                        GeneralXKind::Category,
                        GeneralScaleType::Point
                    )
                );
                if !compatible_x
                    || !matches!(
                        y_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                {
                    return Err(invalid(
                        "xy_line/xy_area/range_area requires X data/axis semantics matching its continuous, temporal, or category pane and a numeric Y axis",
                    ));
                }
            }
            GeneralSeriesKind::RangeBar => {
                if pane_domain
                    != (HorizontalDomain::Category {
                        scale: crate::CategoryScaleType::Band,
                    })
                    || dataset_kind != GeneralXKind::Category
                    || x_axis.scale() != GeneralScaleType::Band
                    || !matches!(
                        y_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                {
                    return Err(invalid(
                        "range_bar requires a category-band pane, category X data, a band X axis, and a numeric Y axis",
                    ));
                }
            }
            GeneralSeriesKind::Column => {
                if pane_domain
                    != (HorizontalDomain::Category {
                        scale: crate::CategoryScaleType::Band,
                    })
                    || dataset_kind != GeneralXKind::Category
                    || x_axis.scale() != GeneralScaleType::Band
                    || y_axis.scale() != GeneralScaleType::Linear
                {
                    return Err(invalid(
                        "the initial column series requires a category-band pane, category X data, a band X axis, and a linear Y axis",
                    ));
                }
            }
            GeneralSeriesKind::HorizontalBar => {
                if pane_domain
                    != (HorizontalDomain::Continuous {
                        scale: crate::ContinuousScaleType::Linear,
                    })
                    || dataset_kind != GeneralXKind::Category
                    || x_axis.scale() != GeneralScaleType::Linear
                    || y_axis.scale() != GeneralScaleType::Band
                {
                    return Err(invalid(
                        "horizontal_bar requires a continuous-linear pane, category/value data, a linear X axis, and a band Y axis",
                    ));
                }
            }
            GeneralSeriesKind::BoxPlot => {
                if pane_domain
                    != (HorizontalDomain::Category {
                        scale: crate::CategoryScaleType::Band,
                    })
                    || dataset_kind != GeneralXKind::Category
                    || x_axis.scale() != GeneralScaleType::Band
                    || !matches!(
                        y_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                {
                    return Err(invalid(
                        "box_plot requires a category-band pane, category data, a band X axis, and a numeric Y axis",
                    ));
                }
            }
            GeneralSeriesKind::HeatmapGrid => {
                let compatible = match dataset_kind {
                    GeneralXKind::Category => {
                        pane_domain
                            == (HorizontalDomain::Category {
                                scale: crate::CategoryScaleType::Band,
                            })
                            && x_axis.scale() == GeneralScaleType::Band
                            && y_axis.scale() == GeneralScaleType::Band
                            && dataset.heatmap_y_categories().is_some()
                    }
                    GeneralXKind::Numeric => {
                        matches!(pane_domain, HorizontalDomain::Continuous { .. })
                            && matches!(
                                x_axis.scale(),
                                GeneralScaleType::Linear
                                    | GeneralScaleType::Logarithmic
                                    | GeneralScaleType::SymmetricLog
                            )
                            && matches!(
                                y_axis.scale(),
                                GeneralScaleType::Linear
                                    | GeneralScaleType::Logarithmic
                                    | GeneralScaleType::SymmetricLog
                            )
                            && dataset.heatmap_y_numeric().is_some()
                    }
                    GeneralXKind::Temporal => {
                        pane_domain == HorizontalDomain::Temporal
                            && x_axis.scale() == GeneralScaleType::Temporal
                            && matches!(
                                y_axis.scale(),
                                GeneralScaleType::Linear
                                    | GeneralScaleType::Logarithmic
                                    | GeneralScaleType::SymmetricLog
                            )
                            && dataset.heatmap_y_numeric().is_some()
                    }
                };
                if !compatible {
                    return Err(invalid(
                        "heatmap_grid requires category/category band axes, numeric/numeric axes, or temporal-X/numeric-Y axes matching its data",
                    ));
                }
            }
            GeneralSeriesKind::Scatter | GeneralSeriesKind::Bubble => {
                if !matches!(pane_domain, HorizontalDomain::Continuous { .. })
                    || dataset_kind != GeneralXKind::Numeric
                    || !matches!(
                        x_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                    || !matches!(
                        y_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                {
                    return Err(invalid(
                        "scatter/bubble requires a continuous pane, numeric X data, and numeric X/Y axes",
                    ));
                }
            }
            GeneralSeriesKind::ErrorBar => {
                let compatible_x = matches!(
                    (pane_domain, dataset_kind, x_axis.scale()),
                    (
                        HorizontalDomain::Continuous { .. },
                        GeneralXKind::Numeric,
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    ) | (
                        HorizontalDomain::Temporal,
                        GeneralXKind::Temporal,
                        GeneralScaleType::Temporal
                    ) | (
                        HorizontalDomain::Category {
                            scale: crate::CategoryScaleType::Band
                        },
                        GeneralXKind::Category,
                        GeneralScaleType::Band
                    ) | (
                        HorizontalDomain::Category {
                            scale: crate::CategoryScaleType::Point
                        },
                        GeneralXKind::Category,
                        GeneralScaleType::Point
                    )
                );
                if !compatible_x
                    || !matches!(
                        y_axis.scale(),
                        GeneralScaleType::Linear
                            | GeneralScaleType::Logarithmic
                            | GeneralScaleType::SymmetricLog
                    )
                {
                    return Err(invalid(
                        "error_bar requires matching numeric/temporal/category X pane, data, and axis semantics with a numeric Y axis",
                    ));
                }
            }
        }
        validate_dataset_for_series(options.kind, dataset, &x_axis, &y_axis)?;
        validate_presentation(&options)?;
        self.validate_layout_compatibility(pane_id, &options, None)?;
        let id = if let Some(registry) = self.general_series.as_mut() {
            registry.insert(pane_id, options)?
        } else {
            let mut registry = GeneralSeriesRegistry::new();
            let id = registry.insert(pane_id, options)?;
            self.general_series = Some(registry);
            id
        };
        self.invalidate_frame_all();
        Ok(id)
    }

    fn validate_layout_compatibility(
        &self,
        pane_id: PaneId,
        options: &GeneralSeriesOptions,
        ignored: Option<GeneralSeriesId>,
    ) -> Result<(), ChartError> {
        if matches!(
            options.kind,
            GeneralSeriesKind::Column | GeneralSeriesKind::HorizontalBar
        ) {
            if let Some(group_id) = options.group_id.as_deref() {
                for sibling in self.general_series_iter().filter(|series| {
                    Some(series.id) != ignored
                        && series.kind == options.kind
                        && series.pane_id == pane_id
                        && series.group_id() == Some(group_id)
                }) {
                    let same_category_axis = match options.kind {
                        GeneralSeriesKind::Column => sibling.x_axis_id == options.x_axis_id,
                        GeneralSeriesKind::HorizontalBar => sibling.y_axis_id == options.y_axis_id,
                        _ => true,
                    };
                    if !same_category_axis {
                        return Err(invalid(
                            "grouped bar series must share the same category axis",
                        ));
                    }
                }
            }
        }

        if let Some(stack_id) = options.stack_id.as_deref() {
            for sibling in self.general_series_iter().filter(|series| {
                Some(series.id) != ignored
                    && series.kind == options.kind
                    && series.pane_id == pane_id
                    && series.group_id() == options.group_id.as_deref()
                    && series.stack_id() == Some(stack_id)
            }) {
                if sibling.x_axis_id != options.x_axis_id
                    || sibling.y_axis_id != options.y_axis_id
                    || sibling.stack_mode != options.stack_mode
                    || (options.kind == GeneralSeriesKind::XyArea
                        && (sibling.interpolation != options.interpolation
                            || sibling.connect_missing != options.connect_missing))
                {
                    return Err(invalid(
                        "stacked series must share X/Y axes, group ID, stack mode, and area path policy",
                    ));
                }
            }
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn update_general_series_options(
        &mut self,
        id: GeneralSeriesId,
        options: GeneralSeriesOptions,
    ) -> Result<(), ChartError> {
        let current = self.general_series(id).cloned().ok_or_else(|| {
            ChartError::new(ErrorCode::InvalidHandle, "general series handle is stale")
        })?;
        let pane = self
            .pane_index_for_id(current.pane_id)
            .ok_or_else(|| invalid("general series references a stale pane"))?;
        if options.kind != current.kind || options.dataset != current.dataset {
            return Err(invalid("general series kind and dataset are structural"));
        }
        let current_domain = self
            .pane_horizontal_domain(pane)
            .ok_or_else(|| invalid("general series references a stale pane"))?;
        let target_pane_id = self
            .pane_stable_id(options.pane)
            .ok_or_else(|| invalid("general series references a stale target pane"))?;
        let target_domain = self
            .pane_horizontal_domain(options.pane)
            .ok_or_else(|| invalid("general series target pane has no horizontal domain"))?;
        let current_x = self
            .general_axis(&current.x_axis_id)
            .ok_or_else(|| invalid("general series current X axis does not exist"))?;
        let current_y = self
            .general_axis(&current.y_axis_id)
            .ok_or_else(|| invalid("general series current Y axis does not exist"))?;
        let target_x = self
            .general_axis(&options.x_axis_id)
            .ok_or_else(|| invalid("general series target X axis does not exist"))?;
        let target_y = self
            .general_axis(&options.y_axis_id)
            .ok_or_else(|| invalid("general series target Y axis does not exist"))?;
        if target_x.pane_id() != target_pane_id
            || target_y.pane_id() != target_pane_id
            || target_x.dimension() != AxisDimension::X
            || target_y.dimension() != AxisDimension::Y
        {
            return Err(invalid(
                "general series target axes must be X/Y axes in its target pane",
            ));
        }
        if target_domain != current_domain
            || target_x.scale() != current_x.scale()
            || target_y.scale() != current_y.scale()
        {
            return Err(invalid(
                "general series rebinding requires equivalent pane and axis scale semantics",
            ));
        }
        validate_presentation(&options)?;
        self.validate_layout_compatibility(target_pane_id, &options, Some(id))?;

        let registry = self
            .general_series
            .as_mut()
            .expect("a resolved general series has a registry");
        let series = registry
            .get_mut(id)
            .expect("a resolved general series remains live during one mutation");
        let geometry_binding_changed = series.pane_id != target_pane_id
            || series.x_axis_id != options.x_axis_id
            || series.y_axis_id != options.y_axis_id
            || series.point_radius != options.point_radius;
        series.pane_id = target_pane_id;
        series.x_axis_id = options.x_axis_id;
        series.y_axis_id = options.y_axis_id;
        series.visible = options.visible;
        series.title = options.title;
        series.color = options.color;
        series.point_radius = options.point_radius;
        series.point_markers = options.point_markers;
        series.point_symbol = options.point_symbol;
        series.line_width = options.line_width;
        series.line_style = options.line_style;
        series.interpolation = options.interpolation;
        series.connect_missing = options.connect_missing;
        series.fill_opacity = options.fill_opacity;
        series.baseline_value = options.baseline_value;
        series.data_labels = options.data_labels;
        series.group_id = options.group_id;
        series.stack_id = options.stack_id;
        series.stack_mode = options.stack_mode;
        if geometry_binding_changed {
            registry.scatter_spatial.get_mut().remove(&id);
        }
        self.invalidate_frame_all();
        Ok(())
    }

    #[doc(hidden)]
    pub fn general_series(&self, id: GeneralSeriesId) -> Option<&GeneralSeries> {
        self.general_series.as_ref()?.get(id)
    }

    #[doc(hidden)]
    pub fn general_series_pane_index(&self, id: GeneralSeriesId) -> Option<usize> {
        self.pane_index_for_id(self.general_series(id)?.pane_id)
    }

    #[doc(hidden)]
    pub fn general_series_count(&self) -> usize {
        self.general_series
            .as_ref()
            .map_or(0, GeneralSeriesRegistry::len)
    }

    #[doc(hidden)]
    pub fn general_series_ids_in_pane(&self, pane_index: usize) -> Vec<GeneralSeriesId> {
        let Some(pane_id) = self.pane_stable_id(pane_index) else {
            return Vec::new();
        };
        self.general_series
            .as_ref()
            .into_iter()
            .flat_map(GeneralSeriesRegistry::iter)
            .filter(|series| series.pane_id == pane_id)
            .map(GeneralSeries::id)
            .collect()
    }

    #[doc(hidden)]
    pub fn general_series_order(&self, pane_index: Option<usize>) -> Vec<GeneralSeriesId> {
        let pane_id = match pane_index {
            Some(index) => match self.pane_stable_id(index) {
                Some(pane_id) => Some(pane_id),
                None => return Vec::new(),
            },
            None => None,
        };
        self.general_series
            .as_ref()
            .map_or_else(Vec::new, |registry| registry.ids(pane_id))
    }

    #[doc(hidden)]
    pub fn set_general_series_order(
        &mut self,
        pane_index: Option<usize>,
        ids: Vec<GeneralSeriesId>,
    ) -> bool {
        let pane_id = match pane_index {
            Some(index) => match self.pane_stable_id(index) {
                Some(pane_id) => Some(pane_id),
                None => return false,
            },
            None => None,
        };
        let accepted = match self.general_series.as_mut() {
            Some(registry) => registry.set_order(pane_id, &ids),
            None => ids.is_empty(),
        };
        if accepted {
            self.invalidate_frame_all();
        }
        accepted
    }

    #[doc(hidden)]
    pub fn remove_general_series(&mut self, id: GeneralSeriesId) -> bool {
        let Some(registry) = self.general_series.as_mut() else {
            return false;
        };
        if !registry.remove(id) {
            return false;
        }
        if registry.is_empty() {
            self.general_series = None;
        }
        self.invalidate_frame_all();
        true
    }

    #[doc(hidden)]
    pub fn set_general_series_visible(&mut self, id: GeneralSeriesId, visible: bool) -> bool {
        let Some(registry) = self.general_series.as_mut() else {
            return false;
        };
        let Some(series) = registry.get_mut(id) else {
            return false;
        };
        if series.visible == visible {
            return true;
        }
        series.visible = visible;
        self.invalidate_frame_all();
        true
    }

    pub(crate) fn general_series_iter(&self) -> impl Iterator<Item = &GeneralSeries> {
        self.general_series
            .as_ref()
            .into_iter()
            .flat_map(GeneralSeriesRegistry::iter)
    }

    #[doc(hidden)]
    pub fn add_general_reference(
        &mut self,
        options: GeneralReferenceOptions,
    ) -> Result<GeneralReferenceId, ChartError> {
        let pane_id = self
            .pane_stable_id(options.pane())
            .ok_or_else(|| invalid("general reference references a stale pane"))?;
        validate_general_reference_options(self, pane_id, &options)?;
        let id = if let Some(registry) = self.general_series.as_mut() {
            registry.reference_insert(pane_id, options)?
        } else {
            let mut registry = GeneralSeriesRegistry::new();
            let id = registry.reference_insert(pane_id, options)?;
            self.general_series = Some(registry);
            id
        };
        self.invalidate_frame_all();
        Ok(id)
    }

    #[doc(hidden)]
    pub fn general_reference_options(
        &self,
        id: GeneralReferenceId,
    ) -> Option<GeneralReferenceOptions> {
        let reference = self.general_series.as_ref()?.reference_get(id)?;
        let pane = self.pane_index_for_id(reference.pane_id)?;
        let mut options = reference.options.clone();
        options.set_pane(pane);
        Some(options)
    }

    #[doc(hidden)]
    pub fn general_reference_ids(&self, pane: Option<usize>) -> Vec<GeneralReferenceId> {
        let pane_id = pane.and_then(|pane| self.pane_stable_id(pane));
        if pane.is_some() && pane_id.is_none() {
            return Vec::new();
        }
        self.general_series
            .as_ref()
            .into_iter()
            .flat_map(GeneralSeriesRegistry::reference_iter)
            .filter(|reference| pane_id.is_none_or(|pane_id| reference.pane_id == pane_id))
            .map(GeneralReference::id)
            .collect()
    }

    #[doc(hidden)]
    pub fn general_reference_count(&self) -> usize {
        self.general_series
            .as_ref()
            .map_or(0, |registry| registry.references.len())
    }

    #[doc(hidden)]
    pub fn remove_general_reference(&mut self, id: GeneralReferenceId) -> bool {
        let Some(registry) = self.general_series.as_mut() else {
            return false;
        };
        if !registry.reference_remove(id) {
            return false;
        }
        if registry.is_empty() {
            self.general_series = None;
        }
        self.invalidate_frame_all();
        true
    }

    pub(crate) fn general_reference_iter(&self) -> impl Iterator<Item = &GeneralReference> {
        self.general_series
            .as_ref()
            .into_iter()
            .flat_map(GeneralSeriesRegistry::reference_iter)
    }

    pub(crate) fn general_series_uses_axis(&self, id: &str) -> bool {
        self.general_series
            .as_ref()
            .is_some_and(|registry| registry.uses_axis(id))
    }

    pub(crate) fn general_series_uses_dataset(&self, id: GeneralDatasetId) -> bool {
        self.general_series
            .as_ref()
            .is_some_and(|registry| registry.uses_dataset(id))
    }

    pub(crate) fn general_series_uses_pane(&self, pane_id: PaneId) -> bool {
        self.general_series
            .as_ref()
            .is_some_and(|registry| registry.uses_pane(pane_id))
    }

    pub(crate) fn validate_general_dataset_replacement(
        &self,
        dataset_id: GeneralDatasetId,
        input: &GeneralXyInput,
    ) -> Result<(), ChartError> {
        let Some(registry) = self.general_series.as_ref() else {
            return Ok(());
        };
        for series in registry
            .series
            .iter()
            .filter(|series| series.dataset == dataset_id)
        {
            let x_axis = self
                .general_axis(&series.x_axis_id)
                .ok_or_else(|| invalid("bound general series X axis is stale"))?;
            let y_axis = self
                .general_axis(&series.y_axis_id)
                .ok_or_else(|| invalid("bound general series Y axis is stale"))?;
            validate_input_for_series(series.kind, input, x_axis, y_axis)?;
        }
        Ok(())
    }

    pub(crate) fn visit_general_columns<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralColumnGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::Column {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (Some(categories), Some(category_indices)) =
            (dataset.categories(), dataset.category_indices())
        else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(GeneralAxisDomain::Category(axis_categories)) =
            self.effective_general_axis_domain(x_axis)
        else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric([y_from, y_to])) =
            self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let Ok(x_scale) = BandScale::new(
            axis_categories.len(),
            x_range.0,
            x_range.1,
            x_axis.band_padding_inner(),
            x_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Ok(y_scale) = LinearScale::new(y_from, y_to, y_range.0, y_range.1) else {
            return;
        };
        let axis_lookup: HashMap<&str, usize> = axis_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();

        let (slot_index, slot_count) = self.general_column_group_slot(series);
        let mut stack_base = series
            .stack_id
            .is_some()
            .then(|| vec![(0.0_f64, 0.0_f64); axis_categories.len()]);
        let mut stack_totals = (series.stack_mode == GeneralStackMode::Percent)
            .then(|| vec![(0.0_f64, 0.0_f64); axis_categories.len()]);
        if series.stack_id.is_some() {
            if let Some(registry) = self.general_series.as_ref() {
                if let Some(totals) = stack_totals.as_mut() {
                    for sibling in registry.series.iter().filter(|candidate| {
                        candidate.visible && column_stack_matches(series, candidate)
                    }) {
                        accumulate_column_values(
                            self,
                            sibling,
                            &axis_lookup,
                            |axis_index, value| {
                                if value >= 0.0 {
                                    totals[axis_index].0 += value;
                                } else {
                                    totals[axis_index].1 += -value;
                                }
                            },
                        );
                    }
                }
                for sibling in registry.series.iter().filter(|candidate| {
                    candidate.visible && column_stack_matches(series, candidate)
                }) {
                    if sibling.id == series.id {
                        break;
                    }
                    let Some(base) = stack_base.as_mut() else {
                        break;
                    };
                    accumulate_column_values(self, sibling, &axis_lookup, |axis_index, value| {
                        if value >= 0.0 {
                            base[axis_index].0 += value;
                        } else {
                            base[axis_index].1 += value;
                        }
                    });
                }
                if let (Some(base), Some(totals)) = (stack_base.as_mut(), stack_totals.as_ref()) {
                    for (base, total) in base.iter_mut().zip(totals) {
                        if total.0 > 0.0 {
                            base.0 /= total.0;
                        }
                        if total.1 > 0.0 {
                            base.1 /= total.1;
                        }
                    }
                }
            }
        }

        for (row, &category_index) in category_indices.iter().enumerate() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let Some(category) = usize::try_from(category_index)
                .ok()
                .and_then(|index| categories.get(index))
            else {
                continue;
            };
            let Some(&axis_index) = axis_lookup.get(category.as_str()) else {
                continue;
            };
            let Some((left, right)) = x_scale.bounds(axis_index) else {
                continue;
            };
            let full_left = left.min(right).clamp(0.0, plot.width);
            let full_right = left.max(right).clamp(0.0, plot.width);
            let slot_width = (full_right - full_left) / slot_count.max(1) as f64;
            let left = full_left + slot_width * slot_index as f64;
            let right = if slot_index + 1 == slot_count {
                full_right
            } else {
                left + slot_width
            };
            let raw_value = dataset.y()[row];
            let (from_value, to_value) = if series.stack_id.is_some() {
                let normalized_value = match stack_totals.as_ref() {
                    Some(totals) if raw_value >= 0.0 => {
                        let total = totals[axis_index].0;
                        if total > 0.0 {
                            raw_value / total
                        } else {
                            0.0
                        }
                    }
                    Some(totals) => {
                        let total = totals[axis_index].1;
                        if total > 0.0 {
                            raw_value / total
                        } else {
                            0.0
                        }
                    }
                    None => raw_value,
                };
                let Some(stack_base) = stack_base.as_mut() else {
                    continue;
                };
                let base = if raw_value >= 0.0 {
                    &mut stack_base[axis_index].0
                } else {
                    &mut stack_base[axis_index].1
                };
                let from = *base;
                let to = from + normalized_value;
                *base = to;
                (from, to)
            } else {
                (0.0, raw_value)
            };
            let Some(from_y) = y_scale.coordinate_clamped(from_value) else {
                continue;
            };
            let Some(to_y) = y_scale.coordinate_clamped(to_value) else {
                continue;
            };
            let top = from_y.min(to_y).clamp(plot.y, plot_bottom);
            let bottom = from_y.max(to_y).clamp(plot.y, plot_bottom);
            if right <= left || bottom <= top {
                continue;
            }
            visit(GeneralColumnGeometry {
                row,
                left,
                right,
                top,
                bottom,
            });
        }
    }

    pub(crate) fn visit_general_horizontal_bars<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralColumnGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::HorizontalBar {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (Some(categories), Some(category_indices)) =
            (dataset.categories(), dataset.category_indices())
        else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric([x_from, x_to])) =
            self.effective_general_axis_domain(x_axis)
        else {
            return;
        };
        let Some(GeneralAxisDomain::Category(axis_categories)) =
            self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let Ok(x_scale) = LinearScale::new(x_from, x_to, x_range.0, x_range.1) else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Ok(y_scale) = BandScale::new(
            axis_categories.len(),
            y_range.0,
            y_range.1,
            y_axis.band_padding_inner(),
            y_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let axis_lookup: HashMap<&str, usize> = axis_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();

        let (slot_index, slot_count) = self.general_horizontal_bar_group_slot(series);
        let mut stack_base = series
            .stack_id
            .is_some()
            .then(|| vec![(0.0_f64, 0.0_f64); axis_categories.len()]);
        let mut stack_totals = (series.stack_mode == GeneralStackMode::Percent)
            .then(|| vec![(0.0_f64, 0.0_f64); axis_categories.len()]);
        if series.stack_id.is_some() {
            if let Some(registry) = self.general_series.as_ref() {
                if let Some(totals) = stack_totals.as_mut() {
                    for sibling in registry.series.iter().filter(|candidate| {
                        candidate.visible && horizontal_bar_stack_matches(series, candidate)
                    }) {
                        accumulate_column_values(
                            self,
                            sibling,
                            &axis_lookup,
                            |axis_index, value| {
                                if value >= 0.0 {
                                    totals[axis_index].0 += value;
                                } else {
                                    totals[axis_index].1 += -value;
                                }
                            },
                        );
                    }
                }
                for sibling in registry.series.iter().filter(|candidate| {
                    candidate.visible && horizontal_bar_stack_matches(series, candidate)
                }) {
                    if sibling.id == series.id {
                        break;
                    }
                    let Some(base) = stack_base.as_mut() else {
                        break;
                    };
                    accumulate_column_values(self, sibling, &axis_lookup, |axis_index, value| {
                        if value >= 0.0 {
                            base[axis_index].0 += value;
                        } else {
                            base[axis_index].1 += value;
                        }
                    });
                }
                if let (Some(base), Some(totals)) = (stack_base.as_mut(), stack_totals.as_ref()) {
                    for (base, total) in base.iter_mut().zip(totals) {
                        if total.0 > 0.0 {
                            base.0 /= total.0;
                        }
                        if total.1 > 0.0 {
                            base.1 /= total.1;
                        }
                    }
                }
            }
        }

        for (row, &category_index) in category_indices.iter().enumerate() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let Some(category) = usize::try_from(category_index)
                .ok()
                .and_then(|index| categories.get(index))
            else {
                continue;
            };
            let Some(&axis_index) = axis_lookup.get(category.as_str()) else {
                continue;
            };
            let Some((from_y, to_y)) = y_scale.bounds(axis_index) else {
                continue;
            };
            let full_top = from_y.min(to_y).clamp(plot.y, plot_bottom);
            let full_bottom = from_y.max(to_y).clamp(plot.y, plot_bottom);
            let slot_height = (full_bottom - full_top) / slot_count.max(1) as f64;
            let top = full_top + slot_height * slot_index as f64;
            let bottom = if slot_index + 1 == slot_count {
                full_bottom
            } else {
                top + slot_height
            };
            let raw_value = dataset.y()[row];
            let (from_value, to_value) = if series.stack_id.is_some() {
                let normalized_value = match stack_totals.as_ref() {
                    Some(totals) if raw_value >= 0.0 => {
                        let total = totals[axis_index].0;
                        if total > 0.0 {
                            raw_value / total
                        } else {
                            0.0
                        }
                    }
                    Some(totals) => {
                        let total = totals[axis_index].1;
                        if total > 0.0 {
                            raw_value / total
                        } else {
                            0.0
                        }
                    }
                    None => raw_value,
                };
                let Some(stack_base) = stack_base.as_mut() else {
                    continue;
                };
                let base = if raw_value >= 0.0 {
                    &mut stack_base[axis_index].0
                } else {
                    &mut stack_base[axis_index].1
                };
                let from = *base;
                let to = from + normalized_value;
                *base = to;
                (from, to)
            } else {
                (0.0, raw_value)
            };
            let Some(from_x) = x_scale.coordinate_clamped(from_value) else {
                continue;
            };
            let Some(to_x) = x_scale.coordinate_clamped(to_value) else {
                continue;
            };
            let left = from_x.min(to_x).clamp(0.0, plot.width);
            let right = from_x.max(to_x).clamp(0.0, plot.width);
            if right <= left || bottom <= top {
                continue;
            }
            visit(GeneralColumnGeometry {
                row,
                left,
                right,
                top,
                bottom,
            });
        }
    }

    pub(crate) fn visit_general_range_bars<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralColumnGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::RangeBar {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (Some(categories), Some(category_indices), Some(low)) = (
            dataset.categories(),
            dataset.category_indices(),
            dataset.low(),
        ) else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(GeneralAxisDomain::Category(axis_categories)) =
            self.effective_general_axis_domain(x_axis)
        else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric([y_from, y_to])) =
            self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let Ok(x_scale) = BandScale::new(
            axis_categories.len(),
            x_range.0,
            x_range.1,
            x_axis.band_padding_inner(),
            x_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) =
            NumericAxisScale::new(y_axis.scale(), [y_from, y_to], y_range.0, y_range.1)
        else {
            return;
        };
        let axis_lookup: HashMap<&str, usize> = axis_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();
        for row in 0..dataset.len() {
            if !dataset.low_is_valid(row) || !dataset.y_is_valid(row) || low[row] > dataset.y()[row]
            {
                continue;
            }
            let Some(category) = usize::try_from(category_indices[row])
                .ok()
                .and_then(|i| categories.get(i))
            else {
                continue;
            };
            let Some(&axis_index) = axis_lookup.get(category.as_str()) else {
                continue;
            };
            let Some((left, right)) = x_scale.bounds(axis_index) else {
                continue;
            };
            let bar_left = left.min(right).clamp(0.0, plot.width);
            let bar_right = left.max(right).clamp(0.0, plot.width);
            let Some(low_y) = y_scale.coordinate(low[row]) else {
                continue;
            };
            let Some(high_y) = y_scale.coordinate(dataset.y()[row]) else {
                continue;
            };
            let top = low_y.min(high_y).clamp(plot.y, plot_bottom);
            let bottom = low_y.max(high_y).clamp(plot.y, plot_bottom);
            if bar_right > bar_left && bottom > top {
                visit(GeneralColumnGeometry {
                    row,
                    left: bar_left,
                    right: bar_right,
                    top,
                    bottom,
                });
            }
        }
    }

    fn general_horizontal_bar_group_slot(&self, series: &GeneralSeries) -> (usize, usize) {
        let Some(group_id) = series.group_id() else {
            return (0, 1);
        };
        let Some(registry) = self.general_series.as_ref() else {
            return (0, 1);
        };
        let mut target_slot = 0usize;
        let mut slot_count = 0usize;
        for (index, sibling) in registry.series.iter().enumerate() {
            if !sibling.visible
                || sibling.kind != GeneralSeriesKind::HorizontalBar
                || sibling.pane_id != series.pane_id
                || sibling.y_axis_id != series.y_axis_id
                || sibling.group_id() != Some(group_id)
            {
                continue;
            }
            let representative = sibling.stack_id().is_none()
                || !registry.series[..index].iter().any(|previous| {
                    previous.visible
                        && previous.kind == GeneralSeriesKind::HorizontalBar
                        && previous.pane_id == series.pane_id
                        && previous.y_axis_id == series.y_axis_id
                        && previous.group_id() == Some(group_id)
                        && previous.stack_id() == sibling.stack_id()
                });
            if !representative {
                continue;
            }
            if sibling.id == series.id
                || (series.stack_id().is_some() && sibling.stack_id() == series.stack_id())
            {
                target_slot = slot_count;
            }
            slot_count += 1;
        }
        (target_slot, slot_count.max(1))
    }

    fn general_column_group_slot(&self, series: &GeneralSeries) -> (usize, usize) {
        let Some(group_id) = series.group_id() else {
            return (0, 1);
        };
        let Some(registry) = self.general_series.as_ref() else {
            return (0, 1);
        };
        let mut target_slot = 0usize;
        let mut slot_count = 0usize;
        for (index, sibling) in registry.series.iter().enumerate() {
            if !sibling.visible
                || sibling.kind != GeneralSeriesKind::Column
                || sibling.pane_id != series.pane_id
                || sibling.x_axis_id != series.x_axis_id
                || sibling.group_id() != Some(group_id)
            {
                continue;
            }
            let representative = sibling.stack_id().is_none()
                || !registry.series[..index].iter().any(|previous| {
                    previous.visible
                        && previous.kind == GeneralSeriesKind::Column
                        && previous.pane_id == series.pane_id
                        && previous.x_axis_id == series.x_axis_id
                        && previous.group_id() == Some(group_id)
                        && previous.stack_id() == sibling.stack_id()
                });
            if !representative {
                continue;
            }
            if sibling.id == series.id
                || (series.stack_id().is_some() && sibling.stack_id() == series.stack_id())
            {
                target_slot = slot_count;
            }
            slot_count += 1;
        }
        (target_slot, slot_count.max(1))
    }

    pub(crate) fn general_column_axis_bounds(
        &self,
        pane_id: PaneId,
        y_axis_id: &str,
    ) -> Option<(f64, f64)> {
        let registry = self.general_series.as_ref()?;
        let mut bounds: Option<(f64, f64)> = None;
        let mut processed_stacks = HashSet::new();

        for series in registry.series.iter().filter(|series| {
            series.visible
                && series.kind == GeneralSeriesKind::Column
                && series.pane_id == pane_id
                && series.y_axis_id == y_axis_id
        }) {
            if let Some(stack_id) = series.stack_id() {
                let key = (
                    series.x_axis_id.clone(),
                    series.group_id.clone(),
                    stack_id.to_owned(),
                    series.stack_mode,
                );
                if !processed_stacks.insert(key) {
                    continue;
                }
                let mut category_totals: HashMap<String, (f64, f64)> = HashMap::new();
                for sibling in registry.series.iter().filter(|candidate| {
                    candidate.visible
                        && candidate.y_axis_id == y_axis_id
                        && column_stack_matches(series, candidate)
                }) {
                    let Some(dataset) = self.general_dataset(sibling.dataset) else {
                        continue;
                    };
                    let (Some(categories), Some(indices)) =
                        (dataset.categories(), dataset.category_indices())
                    else {
                        continue;
                    };
                    for (row, &category_index) in indices.iter().enumerate() {
                        if !dataset.y_is_valid(row) {
                            continue;
                        }
                        let Some(category) = usize::try_from(category_index)
                            .ok()
                            .and_then(|index| categories.get(index))
                        else {
                            continue;
                        };
                        let value = dataset.y()[row];
                        let totals = category_totals.entry(category.clone()).or_default();
                        if value >= 0.0 {
                            totals.0 += value;
                        } else {
                            totals.1 += value;
                        }
                    }
                }
                for (positive, negative) in category_totals.into_values() {
                    match series.stack_mode {
                        GeneralStackMode::Normal => {
                            extend_numeric_pair(&mut bounds, positive);
                            extend_numeric_pair(&mut bounds, negative);
                        }
                        GeneralStackMode::Percent => {
                            if positive > 0.0 {
                                extend_numeric_pair(&mut bounds, 1.0);
                            }
                            if negative < 0.0 {
                                extend_numeric_pair(&mut bounds, -1.0);
                            }
                        }
                    }
                }
            } else if let Some(dataset) = self.general_dataset(series.dataset) {
                for (row, &value) in dataset.y().iter().enumerate() {
                    if dataset.y_is_valid(row) {
                        extend_numeric_pair(&mut bounds, value);
                    }
                }
            }
        }
        if bounds.is_some() {
            extend_numeric_pair(&mut bounds, 0.0);
        }
        bounds
    }

    pub(crate) fn general_horizontal_bar_axis_bounds(
        &self,
        pane_id: PaneId,
        x_axis_id: &str,
    ) -> Option<(f64, f64)> {
        let registry = self.general_series.as_ref()?;
        let mut bounds: Option<(f64, f64)> = None;
        let mut processed_stacks = HashSet::new();

        for series in registry.series.iter().filter(|series| {
            series.visible
                && series.kind == GeneralSeriesKind::HorizontalBar
                && series.pane_id == pane_id
                && series.x_axis_id == x_axis_id
        }) {
            if let Some(stack_id) = series.stack_id() {
                let key = (
                    series.y_axis_id.clone(),
                    series.group_id.clone(),
                    stack_id.to_owned(),
                    series.stack_mode,
                );
                if !processed_stacks.insert(key) {
                    continue;
                }
                let mut category_totals: HashMap<String, (f64, f64)> = HashMap::new();
                for sibling in registry.series.iter().filter(|candidate| {
                    candidate.visible
                        && candidate.x_axis_id == x_axis_id
                        && horizontal_bar_stack_matches(series, candidate)
                }) {
                    let Some(dataset) = self.general_dataset(sibling.dataset) else {
                        continue;
                    };
                    let (Some(categories), Some(indices)) =
                        (dataset.categories(), dataset.category_indices())
                    else {
                        continue;
                    };
                    for (row, &category_index) in indices.iter().enumerate() {
                        if !dataset.y_is_valid(row) {
                            continue;
                        }
                        let Some(category) = usize::try_from(category_index)
                            .ok()
                            .and_then(|index| categories.get(index))
                        else {
                            continue;
                        };
                        let value = dataset.y()[row];
                        let totals = category_totals.entry(category.clone()).or_default();
                        if value >= 0.0 {
                            totals.0 += value;
                        } else {
                            totals.1 += value;
                        }
                    }
                }
                for (positive, negative) in category_totals.into_values() {
                    match series.stack_mode {
                        GeneralStackMode::Normal => {
                            extend_numeric_pair(&mut bounds, positive);
                            extend_numeric_pair(&mut bounds, negative);
                        }
                        GeneralStackMode::Percent => {
                            if positive > 0.0 {
                                extend_numeric_pair(&mut bounds, 1.0);
                            }
                            if negative < 0.0 {
                                extend_numeric_pair(&mut bounds, -1.0);
                            }
                        }
                    }
                }
            } else if let Some(dataset) = self.general_dataset(series.dataset) {
                for (row, &value) in dataset.y().iter().enumerate() {
                    if dataset.y_is_valid(row) {
                        extend_numeric_pair(&mut bounds, value);
                    }
                }
            }
        }
        if bounds.is_some() {
            extend_numeric_pair(&mut bounds, 0.0);
        }
        bounds
    }

    pub(crate) fn general_area_stack_axis_bounds(
        &self,
        pane_id: PaneId,
        y_axis_id: &str,
    ) -> Option<(f64, f64)> {
        let registry = self.general_series.as_ref()?;
        let mut bounds: Option<(f64, f64)> = None;
        let mut processed_stacks = HashSet::new();

        for series in registry.series.iter().filter(|series| {
            series.visible
                && series.kind == GeneralSeriesKind::XyArea
                && series.pane_id == pane_id
                && series.y_axis_id == y_axis_id
                && series.stack_id.is_some()
        }) {
            let key = (
                series.x_axis_id.clone(),
                series.stack_id.clone().expect("stacked area stack ID"),
                series.stack_mode,
            );
            if !processed_stacks.insert(key) {
                continue;
            }
            let mut totals: HashMap<GeneralStackXKey, (f64, f64)> = HashMap::new();
            for sibling in registry
                .series
                .iter()
                .filter(|candidate| candidate.visible && area_stack_matches(series, candidate))
            {
                accumulate_area_values(self, sibling, |x, value| {
                    let total = totals.entry(x).or_default();
                    if value >= 0.0 {
                        total.0 += value;
                    } else {
                        total.1 += -value;
                    }
                });
            }
            for (positive, negative) in totals.into_values() {
                match series.stack_mode {
                    GeneralStackMode::Normal => {
                        if positive > 0.0 {
                            extend_numeric_pair(&mut bounds, positive);
                        }
                        if negative > 0.0 {
                            extend_numeric_pair(&mut bounds, -negative);
                        }
                    }
                    GeneralStackMode::Percent => {
                        if positive > 0.0 {
                            extend_numeric_pair(&mut bounds, 1.0);
                        }
                        if negative > 0.0 {
                            extend_numeric_pair(&mut bounds, -1.0);
                        }
                    }
                }
            }
        }
        bounds
    }

    pub(crate) fn visit_general_stacked_area_points<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralRangePointGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::XyArea || series.stack_id.is_none()
        {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let Some(y_axis) = self.general_axis(&series.y_axis_id) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        let Some(registry) = self.general_series.as_ref() else {
            return;
        };

        let mut totals = (series.stack_mode == GeneralStackMode::Percent)
            .then(HashMap::<GeneralStackXKey, (f64, f64)>::new);
        if let Some(totals) = totals.as_mut() {
            for sibling in registry
                .series
                .iter()
                .filter(|candidate| candidate.visible && area_stack_matches(series, candidate))
            {
                accumulate_area_values(self, sibling, |x, value| {
                    let total = totals.entry(x).or_default();
                    if value >= 0.0 {
                        total.0 += value;
                    } else {
                        total.1 += -value;
                    }
                });
            }
        }

        let mut base: HashMap<GeneralStackXKey, (f64, f64)> = HashMap::new();
        for sibling in registry
            .series
            .iter()
            .filter(|candidate| candidate.visible && area_stack_matches(series, candidate))
        {
            if sibling.id == series.id {
                break;
            }
            accumulate_area_values(self, sibling, |x, value| {
                let entry = base.entry(x).or_default();
                if value >= 0.0 {
                    entry.0 += value;
                } else {
                    entry.1 += value;
                }
            });
        }
        if let Some(totals) = totals.as_ref() {
            for (x, base) in &mut base {
                let total = totals.get(x).copied().unwrap_or_default();
                if total.0 > 0.0 {
                    base.0 /= total.0;
                }
                if total.1 > 0.0 {
                    base.1 /= total.1;
                }
            }
        }

        self.visit_general_path_points(series, |geometry| {
            let Some(x_key) = general_stack_x_key(dataset, geometry.row) else {
                return;
            };
            let raw_value = dataset.y()[geometry.row];
            let normalized_value = match totals.as_ref().and_then(|values| values.get(&x_key)) {
                Some(total) if raw_value >= 0.0 && total.0 > 0.0 => raw_value / total.0,
                Some(total) if raw_value < 0.0 && total.1 > 0.0 => raw_value / total.1,
                Some(_) => 0.0,
                None => raw_value,
            };
            let stack_base = base.get(&x_key).copied().unwrap_or_default();
            let from_value = if raw_value >= 0.0 {
                stack_base.0
            } else {
                stack_base.1
            };
            let to_value = from_value + normalized_value;
            let low_y = y_scale
                .coordinate(from_value)
                .or_else(|| (from_value == 0.0).then_some(y_range.0));
            let high_y = y_scale.coordinate(to_value);
            let (Some(low_y), Some(high_y)) = (low_y, high_y) else {
                return;
            };
            if !low_y.is_finite() || !high_y.is_finite() {
                return;
            }
            visit(GeneralRangePointGeometry {
                row: geometry.row,
                x: geometry.x,
                low_y,
                high_y,
                starts_new_run: geometry.starts_new_run,
            });
        });
    }

    pub(crate) fn visit_general_path_points<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralLinePointGeometry),
    {
        if !series.visible
            || !matches!(
                series.kind,
                GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea
            )
        {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(x_domain) = self.effective_general_axis_domain(x_axis) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };

        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        let numeric_x_scale = match &x_domain {
            GeneralAxisDomain::Numeric(domain) => {
                NumericAxisScale::new(x_axis.scale(), *domain, x_range.0, x_range.1)
            }
            _ => None,
        };
        let temporal_x_scale = match &x_domain {
            GeneralAxisDomain::Temporal([from, to]) => {
                LinearScale::new(*from as f64, *to as f64, x_range.0, x_range.1).ok()
            }
            _ => None,
        };
        let category_lookup: Option<HashMap<&str, usize>> = match &x_domain {
            GeneralAxisDomain::Category(values) => Some(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (value.as_str(), index))
                    .collect(),
            ),
            _ => None,
        };
        let category_band_scale = match &x_domain {
            GeneralAxisDomain::Category(values) if x_axis.scale() == GeneralScaleType::Band => {
                BandScale::new(
                    values.len(),
                    x_range.0,
                    x_range.1,
                    x_axis.band_padding_inner(),
                    x_axis.band_padding_outer(),
                    0.5,
                )
                .ok()
            }
            _ => None,
        };
        let category_point_scale = match &x_domain {
            GeneralAxisDomain::Category(values) if x_axis.scale() == GeneralScaleType::Point => {
                PointScale::new(
                    values.len(),
                    x_range.0,
                    x_range.1,
                    x_axis.band_padding_outer(),
                    0.5,
                )
                .ok()
            }
            _ => None,
        };

        let mut starts_new_run = true;
        for row in 0..dataset.len() {
            if !dataset.y_is_valid(row) {
                if !series.connect_missing {
                    starts_new_run = true;
                }
                continue;
            }
            let Some(y) = y_scale.coordinate(dataset.y()[row]) else {
                starts_new_run = true;
                continue;
            };
            let x = match dataset.x_kind() {
                GeneralXKind::Numeric => dataset
                    .numeric_x()
                    .and_then(|values| values.get(row))
                    .and_then(|value| numeric_x_scale.and_then(|scale| scale.coordinate(*value))),
                GeneralXKind::Temporal => dataset
                    .temporal_x_epoch_ms()
                    .and_then(|values| values.get(row))
                    .and_then(|value| {
                        temporal_x_scale
                            .as_ref()
                            .and_then(|scale| scale.coordinate(*value as f64))
                    }),
                GeneralXKind::Category => {
                    let axis_index = dataset
                        .category_indices()
                        .and_then(|values| values.get(row))
                        .and_then(|value| usize::try_from(*value).ok())
                        .and_then(|index| dataset.categories().and_then(|values| values.get(index)))
                        .and_then(|category| {
                            category_lookup
                                .as_ref()
                                .and_then(|lookup| lookup.get(category.as_str()))
                                .copied()
                        });
                    axis_index.and_then(|index| match x_axis.scale() {
                        GeneralScaleType::Band => category_band_scale
                            .as_ref()
                            .and_then(|scale| scale.center(index)),
                        GeneralScaleType::Point => category_point_scale
                            .as_ref()
                            .and_then(|scale| scale.coordinate(index)),
                        _ => None,
                    })
                }
            };
            let Some(x) = x else {
                starts_new_run = true;
                continue;
            };
            if !x.is_finite() || !y.is_finite() {
                starts_new_run = true;
                continue;
            }
            visit(GeneralLinePointGeometry {
                row,
                x,
                y,
                starts_new_run,
            });
            starts_new_run = false;
        }
    }

    pub(crate) fn visit_general_range_points<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralRangePointGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::RangeArea {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let Some(low_values) = dataset.low() else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(x_domain) = self.effective_general_axis_domain(x_axis) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        let numeric_x_scale = match &x_domain {
            GeneralAxisDomain::Numeric(domain) => {
                NumericAxisScale::new(x_axis.scale(), *domain, x_range.0, x_range.1)
            }
            _ => None,
        };
        let temporal_x_scale = match &x_domain {
            GeneralAxisDomain::Temporal([from, to]) => {
                LinearScale::new(*from as f64, *to as f64, x_range.0, x_range.1).ok()
            }
            _ => None,
        };
        let category_lookup: Option<HashMap<&str, usize>> = match &x_domain {
            GeneralAxisDomain::Category(values) => Some(
                values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (value.as_str(), index))
                    .collect(),
            ),
            _ => None,
        };
        let category_band_scale = match &x_domain {
            GeneralAxisDomain::Category(values) if x_axis.scale() == GeneralScaleType::Band => {
                BandScale::new(
                    values.len(),
                    x_range.0,
                    x_range.1,
                    x_axis.band_padding_inner(),
                    x_axis.band_padding_outer(),
                    0.5,
                )
                .ok()
            }
            _ => None,
        };
        let category_point_scale = match &x_domain {
            GeneralAxisDomain::Category(values) if x_axis.scale() == GeneralScaleType::Point => {
                PointScale::new(
                    values.len(),
                    x_range.0,
                    x_range.1,
                    x_axis.band_padding_outer(),
                    0.5,
                )
                .ok()
            }
            _ => None,
        };

        let mut starts_new_run = true;
        for (row, &low) in low_values.iter().enumerate().take(dataset.len()) {
            if !dataset.y_is_valid(row) || !dataset.low_is_valid(row) {
                if !series.connect_missing {
                    starts_new_run = true;
                }
                continue;
            }
            let Some(high_y) = y_scale.coordinate(dataset.y()[row]) else {
                starts_new_run = true;
                continue;
            };
            let Some(low_y) = y_scale.coordinate(low) else {
                starts_new_run = true;
                continue;
            };
            let x = match dataset.x_kind() {
                GeneralXKind::Numeric => dataset
                    .numeric_x()
                    .and_then(|values| values.get(row))
                    .and_then(|value| numeric_x_scale.and_then(|scale| scale.coordinate(*value))),
                GeneralXKind::Temporal => dataset
                    .temporal_x_epoch_ms()
                    .and_then(|values| values.get(row))
                    .and_then(|value| {
                        temporal_x_scale
                            .as_ref()
                            .and_then(|scale| scale.coordinate(*value as f64))
                    }),
                GeneralXKind::Category => {
                    let axis_index = dataset
                        .category_indices()
                        .and_then(|values| values.get(row))
                        .and_then(|value| usize::try_from(*value).ok())
                        .and_then(|index| dataset.categories().and_then(|values| values.get(index)))
                        .and_then(|category| {
                            category_lookup
                                .as_ref()
                                .and_then(|lookup| lookup.get(category.as_str()))
                                .copied()
                        });
                    axis_index.and_then(|index| match x_axis.scale() {
                        GeneralScaleType::Band => category_band_scale
                            .as_ref()
                            .and_then(|scale| scale.center(index)),
                        GeneralScaleType::Point => category_point_scale
                            .as_ref()
                            .and_then(|scale| scale.coordinate(index)),
                        _ => None,
                    })
                }
            };
            let Some(x) = x else {
                starts_new_run = true;
                continue;
            };
            if !x.is_finite() || !low_y.is_finite() || !high_y.is_finite() {
                starts_new_run = true;
                continue;
            }
            visit(GeneralRangePointGeometry {
                row,
                x,
                low_y,
                high_y,
                starts_new_run,
            });
            starts_new_run = false;
        }
    }

    pub(crate) fn general_path_baseline_y(&self, series: &GeneralSeries) -> Option<f64> {
        if !series.visible
            || !matches!(
                series.kind,
                GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea
            )
        {
            return None;
        }
        let pane_index = self.pane_index_for_id(series.pane_id)?;
        let plot = self.general_plot_rect(pane_index)?;
        let y_axis = self.general_axis(&series.y_axis_id)?;
        let GeneralAxisDomain::Numeric(y_domain) = self.effective_general_axis_domain(y_axis)?
        else {
            return None;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let scale = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)?;
        Some(
            scale
                .coordinate(series.baseline_value.unwrap_or(0.0))
                .unwrap_or(y_range.0)
                .clamp(plot.y, plot_bottom),
        )
    }

    pub(crate) fn visit_general_error_bars<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralErrorBarGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::ErrorBar {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (Some(y_low_values), Some(y_high_values)) = (dataset.low(), dataset.high()) else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(x_domain) = self.effective_general_axis_domain(x_axis) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        match x_domain {
            GeneralAxisDomain::Numeric(domain) => {
                let (Some(x_values), Some(x_low_values), Some(x_high_values), Some(x_scale)) = (
                    dataset.numeric_x(),
                    dataset.x_low(),
                    dataset.x_high(),
                    NumericAxisScale::new(x_axis.scale(), domain, x_range.0, x_range.1),
                ) else {
                    return;
                };
                for row in 0..dataset.len() {
                    if !dataset.y_is_valid(row) {
                        continue;
                    }
                    let (Some(x), Some(y)) = (
                        x_scale.coordinate(x_values[row]),
                        y_scale.coordinate(dataset.y()[row]),
                    ) else {
                        continue;
                    };
                    let x_low = dataset
                        .x_low_is_valid(row)
                        .then(|| x_scale.coordinate(x_low_values[row]))
                        .flatten();
                    let x_high = dataset
                        .x_high_is_valid(row)
                        .then(|| x_scale.coordinate(x_high_values[row]))
                        .flatten();
                    let y_low = dataset
                        .low_is_valid(row)
                        .then(|| y_scale.coordinate(y_low_values[row]))
                        .flatten();
                    let y_high = dataset
                        .high_is_valid(row)
                        .then(|| y_scale.coordinate(y_high_values[row]))
                        .flatten();
                    visit(GeneralErrorBarGeometry {
                        row,
                        x,
                        y,
                        x_low,
                        x_high,
                        y_low,
                        y_high,
                        cap_half_size: series.point_radius,
                    });
                }
            }
            GeneralAxisDomain::Category(axis_categories) => {
                let (Some(categories), Some(indices)) =
                    (dataset.categories(), dataset.category_indices())
                else {
                    return;
                };
                let positions: Option<Vec<f64>> = match x_axis.scale() {
                    GeneralScaleType::Band => BandScale::new(
                        axis_categories.len(),
                        x_range.0,
                        x_range.1,
                        x_axis.band_padding_inner(),
                        x_axis.band_padding_outer(),
                        0.5,
                    )
                    .ok()
                    .and_then(|scale| {
                        (0..axis_categories.len())
                            .map(|index| scale.center(index))
                            .collect()
                    }),
                    GeneralScaleType::Point => PointScale::new(
                        axis_categories.len(),
                        x_range.0,
                        x_range.1,
                        x_axis.band_padding_outer(),
                        0.5,
                    )
                    .ok()
                    .and_then(|scale| {
                        (0..axis_categories.len())
                            .map(|index| scale.coordinate(index))
                            .collect()
                    }),
                    _ => None,
                };
                let Some(positions) = positions else {
                    return;
                };
                let category_x: HashMap<&str, f64> = axis_categories
                    .iter()
                    .map(String::as_str)
                    .zip(positions)
                    .collect();
                for row in 0..dataset.len() {
                    if !dataset.y_is_valid(row) {
                        continue;
                    }
                    let x = categories
                        .get(indices[row] as usize)
                        .and_then(|category| category_x.get(category.as_str()))
                        .copied();
                    let (Some(x), Some(y)) = (x, y_scale.coordinate(dataset.y()[row])) else {
                        continue;
                    };
                    let y_low = dataset
                        .low_is_valid(row)
                        .then(|| y_scale.coordinate(y_low_values[row]))
                        .flatten();
                    let y_high = dataset
                        .high_is_valid(row)
                        .then(|| y_scale.coordinate(y_high_values[row]))
                        .flatten();
                    visit(GeneralErrorBarGeometry {
                        row,
                        x,
                        y,
                        x_low: None,
                        x_high: None,
                        y_low,
                        y_high,
                        cap_half_size: series.point_radius,
                    });
                }
            }
            GeneralAxisDomain::Temporal([from, to]) => {
                let (Some(x_values), Some(x_low_values), Some(x_high_values)) = (
                    dataset.temporal_x_epoch_ms(),
                    dataset.x_low(),
                    dataset.x_high(),
                ) else {
                    return;
                };
                let Ok(x_scale) = LinearScale::new(from as f64, to as f64, x_range.0, x_range.1)
                else {
                    return;
                };
                for row in 0..dataset.len() {
                    if !dataset.y_is_valid(row) {
                        continue;
                    }
                    let (Some(x), Some(y)) = (
                        x_scale.coordinate(x_values[row] as f64),
                        y_scale.coordinate(dataset.y()[row]),
                    ) else {
                        continue;
                    };
                    let x_low = dataset
                        .x_low_is_valid(row)
                        .then(|| x_scale.coordinate(x_low_values[row]))
                        .flatten();
                    let x_high = dataset
                        .x_high_is_valid(row)
                        .then(|| x_scale.coordinate(x_high_values[row]))
                        .flatten();
                    let y_low = dataset
                        .low_is_valid(row)
                        .then(|| y_scale.coordinate(y_low_values[row]))
                        .flatten();
                    let y_high = dataset
                        .high_is_valid(row)
                        .then(|| y_scale.coordinate(y_high_values[row]))
                        .flatten();
                    visit(GeneralErrorBarGeometry {
                        row,
                        x,
                        y,
                        x_low,
                        x_high,
                        y_low,
                        y_high,
                        cap_half_size: series.point_radius,
                    });
                }
            }
            GeneralAxisDomain::Auto => {}
        }
    }

    pub(crate) fn visit_general_box_plots<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralBoxPlotGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::BoxPlot {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        let (
            Some(categories),
            Some(category_indices),
            Some(min_values),
            Some(max_values),
            Some(q1_values),
            Some(q3_values),
        ) = (
            dataset.categories(),
            dataset.category_indices(),
            dataset.low(),
            dataset.high(),
            dataset.x_low(),
            dataset.x_high(),
        )
        else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(GeneralAxisDomain::Category(axis_categories)) =
            self.effective_general_axis_domain(x_axis)
        else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let Ok(x_scale) = BandScale::new(
            axis_categories.len(),
            x_range.0,
            x_range.1,
            x_axis.band_padding_inner(),
            x_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        let axis_lookup: HashMap<&str, usize> = axis_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();

        for (row, &category_index) in category_indices.iter().enumerate() {
            if !dataset.y_is_valid(row)
                || !dataset.low_is_valid(row)
                || !dataset.high_is_valid(row)
                || !dataset.x_low_is_valid(row)
                || !dataset.x_high_is_valid(row)
            {
                continue;
            }
            let Some(category) = usize::try_from(category_index)
                .ok()
                .and_then(|index| categories.get(index))
            else {
                continue;
            };
            let Some(&axis_index) = axis_lookup.get(category.as_str()) else {
                continue;
            };
            let Some((band_from, band_to)) = x_scale.bounds(axis_index) else {
                continue;
            };
            let full_left = band_from.min(band_to).clamp(0.0, plot.width);
            let full_right = band_from.max(band_to).clamp(0.0, plot.width);
            let inset = (full_right - full_left) * 0.2;
            let left = full_left + inset;
            let right = full_right - inset;
            if right <= left {
                continue;
            }
            let (Some(min_y), Some(q1_y), Some(median_y), Some(q3_y), Some(max_y)) = (
                y_scale.coordinate(min_values[row]),
                y_scale.coordinate(q1_values[row]),
                y_scale.coordinate(dataset.y()[row]),
                y_scale.coordinate(q3_values[row]),
                y_scale.coordinate(max_values[row]),
            ) else {
                continue;
            };
            if [min_y, q1_y, median_y, q3_y, max_y]
                .iter()
                .any(|value| !value.is_finite())
            {
                continue;
            }
            visit(GeneralBoxPlotGeometry {
                row,
                center_x: (left + right) * 0.5,
                left,
                right,
                min_y,
                q1_y,
                median_y,
                q3_y,
                max_y,
            });
        }
    }

    pub(crate) fn visit_general_heatmap_cells<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralHeatmapGeometry),
    {
        if !series.visible || series.kind != GeneralSeriesKind::HeatmapGrid {
            return;
        }
        let Some(pane_index) = self.pane_index_for_id(series.pane_id) else {
            return;
        };
        let Some(plot) = self.general_plot_rect(pane_index) else {
            return;
        };
        let Some(dataset) = self.general_dataset(series.dataset) else {
            return;
        };
        if dataset.heatmap_y_numeric().is_some() {
            self.visit_general_numeric_heatmap_cells(series, pane_index, plot, dataset, &mut visit);
            return;
        }
        let (
            Some(x_categories),
            Some(x_category_indices),
            Some(y_categories),
            Some(y_category_indices),
        ) = (
            dataset.categories(),
            dataset.category_indices(),
            dataset.heatmap_y_categories(),
            dataset.heatmap_y_category_indices(),
        )
        else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let (
            Some(GeneralAxisDomain::Category(axis_x_categories)),
            Some(GeneralAxisDomain::Category(axis_y_categories)),
        ) = (
            self.effective_general_axis_domain(x_axis),
            self.effective_general_axis_domain(y_axis),
        )
        else {
            return;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let Ok(x_scale) = BandScale::new(
            axis_x_categories.len(),
            x_range.0,
            x_range.1,
            x_axis.band_padding_inner(),
            x_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Ok(y_scale) = BandScale::new(
            axis_y_categories.len(),
            y_range.0,
            y_range.1,
            y_axis.band_padding_inner(),
            y_axis.band_padding_outer(),
            0.5,
        ) else {
            return;
        };
        let x_lookup: HashMap<&str, usize> = axis_x_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();
        let y_lookup: HashMap<&str, usize> = axis_y_categories
            .iter()
            .enumerate()
            .map(|(index, category)| (category.as_str(), index))
            .collect();

        let mut value_bounds: Option<(f64, f64)> = None;
        for row in 0..dataset.len() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let mapped = x_category_indices
                .get(row)
                .and_then(|index| usize::try_from(*index).ok())
                .and_then(|index| x_categories.get(index))
                .and_then(|category| x_lookup.get(category.as_str()))
                .is_some()
                && y_category_indices
                    .get(row)
                    .and_then(|index| usize::try_from(*index).ok())
                    .and_then(|index| y_categories.get(index))
                    .and_then(|category| y_lookup.get(category.as_str()))
                    .is_some();
            if !mapped {
                continue;
            }
            let value = dataset.y()[row];
            value_bounds = Some(match value_bounds {
                Some((low, high)) => (low.min(value), high.max(value)),
                None => (value, value),
            });
        }
        let Some((value_low, value_high)) = value_bounds else {
            return;
        };

        for row in 0..dataset.len() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let Some(x_axis_index) = x_category_indices
                .get(row)
                .and_then(|index| usize::try_from(*index).ok())
                .and_then(|index| x_categories.get(index))
                .and_then(|category| x_lookup.get(category.as_str()))
                .copied()
            else {
                continue;
            };
            let Some(y_axis_index) = y_category_indices
                .get(row)
                .and_then(|index| usize::try_from(*index).ok())
                .and_then(|index| y_categories.get(index))
                .and_then(|category| y_lookup.get(category.as_str()))
                .copied()
            else {
                continue;
            };
            let (Some((x0, x1)), Some((y0, y1))) =
                (x_scale.bounds(x_axis_index), y_scale.bounds(y_axis_index))
            else {
                continue;
            };
            let left = x0.min(x1).clamp(0.0, plot.width);
            let right = x0.max(x1).clamp(0.0, plot.width);
            let top = y0.min(y1).clamp(plot.y, plot_bottom);
            let bottom = y0.max(y1).clamp(plot.y, plot_bottom);
            if right <= left || bottom <= top {
                continue;
            }
            let value = dataset.y()[row];
            let intensity = if value_high > value_low {
                ((value - value_low) / (value_high - value_low)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            visit(GeneralHeatmapGeometry {
                row,
                left,
                right,
                top,
                bottom,
                value,
                intensity,
            });
        }
    }

    fn visit_general_numeric_heatmap_cells<F>(
        &self,
        series: &GeneralSeries,
        _pane_index: usize,
        plot: crate::general_axes::GeneralPlotRect,
        dataset: &crate::GeneralDataset,
        visit: &mut F,
    ) where
        F: FnMut(GeneralHeatmapGeometry),
    {
        let Some(y_coordinates) = dataset.heatmap_y_numeric() else {
            return;
        };
        let (Some(x_axis), Some(y_axis)) = (
            self.general_axis(&series.x_axis_id),
            self.general_axis(&series.y_axis_id),
        ) else {
            return;
        };
        let Some(GeneralAxisDomain::Numeric(y_domain)) = self.effective_general_axis_domain(y_axis)
        else {
            return;
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let Some(y_scale) = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)
        else {
            return;
        };
        let x_domain = self.effective_general_axis_domain(x_axis);
        let numeric_x_scale = match x_domain.as_ref() {
            Some(GeneralAxisDomain::Numeric(domain)) => {
                let range = if x_axis.reverse() {
                    (plot.width, 0.0)
                } else {
                    (0.0, plot.width)
                };
                NumericAxisScale::new(x_axis.scale(), *domain, range.0, range.1)
            }
            _ => None,
        };
        let temporal_x_scale = match x_domain.as_ref() {
            Some(GeneralAxisDomain::Temporal([from, to])) => {
                let range = if x_axis.reverse() {
                    (plot.width, 0.0)
                } else {
                    (0.0, plot.width)
                };
                LinearScale::new(*from as f64, *to as f64, range.0, range.1).ok()
            }
            _ => None,
        };

        let mut mapped = Vec::with_capacity(dataset.len());
        let mut x_centers = Vec::with_capacity(dataset.len());
        let mut y_centers = Vec::with_capacity(dataset.len());
        for row in 0..dataset.len() {
            let x = match dataset.x_kind() {
                GeneralXKind::Numeric => dataset
                    .numeric_x()
                    .and_then(|values| values.get(row))
                    .and_then(|value| numeric_x_scale.and_then(|scale| scale.coordinate(*value))),
                GeneralXKind::Temporal => dataset
                    .temporal_x_epoch_ms()
                    .and_then(|values| values.get(row))
                    .and_then(|value| {
                        temporal_x_scale
                            .as_ref()
                            .and_then(|scale| scale.coordinate(*value as f64))
                    }),
                GeneralXKind::Category => None,
            };
            let y = y_coordinates
                .get(row)
                .and_then(|value| y_scale.coordinate(*value));
            let point = x.zip(y).filter(|(x, y)| x.is_finite() && y.is_finite());
            if let Some((x, y)) = point {
                x_centers.push(x);
                y_centers.push(y);
            }
            mapped.push(point);
        }
        x_centers.sort_by(f64::total_cmp);
        x_centers.dedup_by(|left, right| left.to_bits() == right.to_bits());
        y_centers.sort_by(f64::total_cmp);
        y_centers.dedup_by(|left, right| left.to_bits() == right.to_bits());
        if x_centers.is_empty() || y_centers.is_empty() {
            return;
        }

        let mut value_bounds: Option<(f64, f64)> = None;
        for (row, point) in mapped.iter().enumerate() {
            if point.is_none() || !dataset.y_is_valid(row) {
                continue;
            }
            let value = dataset.y()[row];
            value_bounds = Some(match value_bounds {
                Some((low, high)) => (low.min(value), high.max(value)),
                None => (value, value),
            });
        }
        let Some((value_low, value_high)) = value_bounds else {
            return;
        };

        for (row, point) in mapped.into_iter().enumerate() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let Some((x, y)) = point else {
                continue;
            };
            let Some((left, right)) = heatmap_center_bounds(x, &x_centers, 0.0, plot.width) else {
                continue;
            };
            let Some((top, bottom)) = heatmap_center_bounds(y, &y_centers, plot.y, plot_bottom)
            else {
                continue;
            };
            let value = dataset.y()[row];
            let intensity = if value_high > value_low {
                ((value - value_low) / (value_high - value_low)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            visit(GeneralHeatmapGeometry {
                row,
                left,
                right,
                top,
                bottom,
                value,
                intensity,
            });
        }
    }

    fn scatter_geometry_context(&self, series: &GeneralSeries) -> Option<ScatterGeometryContext> {
        if !series.visible
            || !matches!(
                series.kind,
                GeneralSeriesKind::Scatter | GeneralSeriesKind::Bubble
            )
        {
            return None;
        }
        let pane_index = self.pane_index_for_id(series.pane_id)?;
        let plot = self.general_plot_rect(pane_index)?;
        let dataset = self.general_dataset(series.dataset)?;
        dataset.numeric_x()?;
        let x_axis = self.general_axis(&series.x_axis_id)?;
        let y_axis = self.general_axis(&series.y_axis_id)?;
        let GeneralAxisDomain::Numeric(x_domain) = self.effective_general_axis_domain(x_axis)?
        else {
            return None;
        };
        let GeneralAxisDomain::Numeric(y_domain) = self.effective_general_axis_domain(y_axis)?
        else {
            return None;
        };
        let x_range = if x_axis.reverse() {
            (plot.width, 0.0)
        } else {
            (0.0, plot.width)
        };
        let plot_bottom = plot.y + plot.height;
        let y_range = if y_axis.reverse() {
            (plot.y, plot_bottom)
        } else {
            (plot_bottom, plot.y)
        };
        let x_scale = NumericAxisScale::new(x_axis.scale(), x_domain, x_range.0, x_range.1)?;
        let y_scale = NumericAxisScale::new(y_axis.scale(), y_domain, y_range.0, y_range.1)?;
        let radius = if series.kind == GeneralSeriesKind::Bubble {
            MAX_GENERAL_POINT_RADIUS
        } else {
            series.point_radius
        };
        Some(ScatterGeometryContext {
            key: ScatterGeometryKey {
                dataset_generation: dataset.generation(),
                plot_width: plot.width.to_bits(),
                plot_y: plot.y.to_bits(),
                plot_height: plot.height.to_bits(),
                x_domain: x_domain.map(f64::to_bits),
                y_domain: y_domain.map(f64::to_bits),
                x_scale: x_axis.scale(),
                y_scale: y_axis.scale(),
                x_reverse: x_axis.reverse(),
                y_reverse: y_axis.reverse(),
                radius: radius.to_bits(),
            },
            x_scale,
            y_scale,
            plot_width: plot.width,
            plot_y: plot.y,
            plot_bottom,
        })
    }

    fn build_scatter_spatial_index(
        &self,
        series: &GeneralSeries,
        context: ScatterGeometryContext,
    ) -> Option<ScatterSpatialIndex> {
        let dataset = self.general_dataset(series.dataset)?;
        let x_values = dataset.numeric_x()?;
        let mut points = Vec::with_capacity(dataset.len());
        for (row, (&x_value, &y_value)) in x_values.iter().zip(dataset.y()).enumerate() {
            if !dataset.y_is_valid(row) {
                continue;
            }
            let radius = match series.kind {
                GeneralSeriesKind::Scatter => series.point_radius,
                GeneralSeriesKind::Bubble => {
                    let size = dataset.size()?.get(row).copied()?;
                    if !dataset.size_is_valid(row) || size <= 0.0 {
                        continue;
                    }
                    size.sqrt()
                        .clamp(MIN_GENERAL_POINT_RADIUS, MAX_GENERAL_POINT_RADIUS)
                }
                _ => return None,
            };
            let (Some(x), Some(y)) = (
                context.x_scale.coordinate(x_value),
                context.y_scale.coordinate(y_value),
            ) else {
                continue;
            };
            if x < -radius
                || x > context.plot_width + radius
                || y < context.plot_y - radius
                || y > context.plot_bottom + radius
            {
                continue;
            }
            points.push(GeneralScatterGeometry { row, x, y, radius });
        }
        Some(ScatterSpatialIndex::new(
            context.key,
            context.plot_y,
            context.plot_width,
            context.plot_bottom - context.plot_y,
            points,
        ))
    }

    fn with_scatter_spatial_index<R, F>(&self, series: &GeneralSeries, use_index: F) -> Option<R>
    where
        F: FnOnce(&ScatterSpatialIndex) -> R,
    {
        let context = self.scatter_geometry_context(series)?;
        let registry = self.general_series.as_ref()?;
        let current = registry
            .scatter_spatial
            .borrow()
            .get(&series.id)
            .is_some_and(|index| index.key == context.key);
        if !current {
            let index = self.build_scatter_spatial_index(series, context)?;
            registry
                .scatter_spatial
                .borrow_mut()
                .insert(series.id, index);
        }
        let cache = registry.scatter_spatial.borrow();
        let index = cache.get(&series.id)?;
        Some(use_index(index))
    }

    /// Resolve one series' auto-domain contribution for an axis, rescanning its rows only when
    /// the dataset generation or the series binding that shapes the scan changed.
    pub(crate) fn cached_general_series_axis_scan<F>(
        &self,
        series: &GeneralSeries,
        dataset: &crate::general_data::GeneralDataset,
        dimension: AxisDimension,
        scale: GeneralScaleType,
        scan: F,
    ) -> (Option<(f64, f64)>, bool)
    where
        F: FnOnce() -> (Option<(f64, f64)>, bool),
    {
        let Some(registry) = self.general_series.as_ref() else {
            return scan();
        };
        let matches = |entry: &AxisScanEntry| {
            entry.dimension == dimension
                && entry.scale == scale
                && entry.dataset == series.dataset
                && entry.generation == dataset.generation()
                && entry.kind == series.kind
                && entry.stacked == series.stack_id.is_some()
        };
        if let Some(entry) = registry
            .axis_scans
            .borrow()
            .get(&series.id)
            .and_then(|entries| entries.iter().find(|entry| matches(entry)))
        {
            return entry.value;
        }
        let value = scan();
        let mut cache = registry.axis_scans.borrow_mut();
        let entries = cache.entry(series.id).or_default();
        entries.retain(|entry| !(entry.dimension == dimension && entry.scale == scale));
        if entries.len() >= MAX_AXIS_SCANS_PER_SERIES {
            entries.remove(0);
        }
        entries.push(AxisScanEntry {
            dimension,
            scale,
            dataset: series.dataset,
            generation: dataset.generation(),
            kind: series.kind,
            stacked: series.stack_id.is_some(),
            value,
        });
        value
    }

    pub(crate) fn visit_general_scatter_points<F>(&self, series: &GeneralSeries, mut visit: F)
    where
        F: FnMut(GeneralScatterGeometry),
    {
        let _ = self.with_scatter_spatial_index(series, |index| {
            for &point in &index.points {
                visit(point);
            }
        });
    }

    #[doc(hidden)]
    pub fn general_hit_test(
        &self,
        pane_index: usize,
        x_css: f64,
        y_css: f64,
        mode: GeneralHitMode,
    ) -> Option<GeneralSeriesHit> {
        if !x_css.is_finite() || !y_css.is_finite() {
            return None;
        }
        let max_distance = match mode {
            GeneralHitMode::Exact => 0.0,
            GeneralHitMode::Nearest { max_distance }
                if max_distance.is_finite() && max_distance >= 0.0 =>
            {
                max_distance
            }
            GeneralHitMode::Nearest { .. } => return None,
        };
        let pane_id = self.pane_stable_id(pane_index)?;
        let registry = self.general_series.as_ref()?;
        let mut best: Option<GeneralSeriesHit> = None;

        for series in registry
            .series
            .iter()
            .rev()
            .filter(|series| series.visible && series.pane_id == pane_id)
        {
            let Some(dataset) = self.general_dataset(series.dataset) else {
                continue;
            };
            let mut consider = |row: usize, distance: f64| {
                if distance > max_distance {
                    return;
                }
                if best
                    .as_ref()
                    .is_some_and(|current| distance >= current.distance)
                {
                    return;
                }
                let Some(row_id) = dataset.row_identity(row).cloned() else {
                    return;
                };
                best = Some(GeneralSeriesHit {
                    series: series.id,
                    row,
                    row_id,
                    distance,
                });
            };
            match series.kind {
                GeneralSeriesKind::XyLine => {
                    if series.interpolation != GeneralInterpolation::Linear {
                        let mut run = Vec::new();
                        self.visit_general_path_points(series, |geometry| {
                            if geometry.starts_new_run {
                                if let Some((row, distance)) = distance_to_interpolated_line(
                                    x_css,
                                    y_css,
                                    &run,
                                    series.interpolation.render_type(),
                                ) {
                                    consider(row, (distance - 3.0).max(0.0));
                                }
                                run.clear();
                            }
                            if series.point_markers {
                                consider(
                                    geometry.row,
                                    distance_to_point_symbol(
                                        x_css,
                                        y_css,
                                        geometry.x,
                                        geometry.y,
                                        series.point_radius,
                                        series.point_symbol,
                                    ),
                                );
                            }
                            run.push(geometry);
                        });
                        if let Some((row, distance)) = distance_to_interpolated_line(
                            x_css,
                            y_css,
                            &run,
                            series.interpolation.render_type(),
                        ) {
                            consider(row, (distance - 3.0).max(0.0));
                        }
                        continue;
                    }
                    let mut previous: Option<GeneralLinePointGeometry> = None;
                    self.visit_general_path_points(series, |geometry| {
                        if series.point_markers {
                            consider(
                                geometry.row,
                                distance_to_point_symbol(
                                    x_css,
                                    y_css,
                                    geometry.x,
                                    geometry.y,
                                    series.point_radius,
                                    series.point_symbol,
                                ),
                            );
                        }
                        if geometry.starts_new_run {
                            previous = Some(geometry);
                            return;
                        }
                        let Some(from) = previous else {
                            previous = Some(geometry);
                            return;
                        };
                        let (distance, position) = distance_to_segment(
                            x_css, y_css, from.x, from.y, geometry.x, geometry.y,
                        );
                        let row = if position <= 0.5 {
                            from.row
                        } else {
                            geometry.row
                        };
                        consider(row, (distance - 3.0).max(0.0));
                        previous = Some(geometry);
                    });
                }
                GeneralSeriesKind::XyArea => {
                    if series.stack_id.is_some() {
                        if series.interpolation != GeneralInterpolation::Linear {
                            let mut run = Vec::new();
                            self.visit_general_stacked_area_points(series, |geometry| {
                                if geometry.starts_new_run {
                                    if let Some((row, distance)) = distance_to_interpolated_band(
                                        x_css,
                                        y_css,
                                        &run,
                                        series.interpolation.render_type(),
                                    ) {
                                        consider(row, distance);
                                    }
                                    run.clear();
                                }
                                if series.point_markers {
                                    consider(
                                        geometry.row,
                                        distance_to_point_symbol(
                                            x_css,
                                            y_css,
                                            geometry.x,
                                            geometry.high_y,
                                            series.point_radius,
                                            series.point_symbol,
                                        ),
                                    );
                                }
                                run.push(geometry);
                            });
                            if let Some((row, distance)) = distance_to_interpolated_band(
                                x_css,
                                y_css,
                                &run,
                                series.interpolation.render_type(),
                            ) {
                                consider(row, distance);
                            }
                            continue;
                        }
                        let mut previous: Option<GeneralRangePointGeometry> = None;
                        self.visit_general_stacked_area_points(series, |geometry| {
                            if series.point_markers {
                                consider(
                                    geometry.row,
                                    distance_to_point_symbol(
                                        x_css,
                                        y_css,
                                        geometry.x,
                                        geometry.high_y,
                                        series.point_radius,
                                        series.point_symbol,
                                    ),
                                );
                            }
                            if geometry.starts_new_run {
                                previous = Some(geometry);
                                return;
                            }
                            let Some(from) = previous else {
                                previous = Some(geometry);
                                return;
                            };
                            let (distance, position) = distance_to_band_segment(
                                x_css,
                                y_css,
                                from.x,
                                from.low_y,
                                from.high_y,
                                geometry.x,
                                geometry.low_y,
                                geometry.high_y,
                            );
                            let row = if position <= 0.5 {
                                from.row
                            } else {
                                geometry.row
                            };
                            consider(row, distance);
                            previous = Some(geometry);
                        });
                        continue;
                    }
                    let Some(baseline_y) = self.general_path_baseline_y(series) else {
                        continue;
                    };
                    if series.interpolation != GeneralInterpolation::Linear {
                        let mut run = Vec::new();
                        self.visit_general_path_points(series, |geometry| {
                            if geometry.starts_new_run {
                                if let Some((row, distance)) = distance_to_interpolated_area(
                                    x_css,
                                    y_css,
                                    &run,
                                    baseline_y,
                                    series.interpolation.render_type(),
                                ) {
                                    consider(row, distance);
                                }
                                run.clear();
                            }
                            if series.point_markers {
                                consider(
                                    geometry.row,
                                    distance_to_point_symbol(
                                        x_css,
                                        y_css,
                                        geometry.x,
                                        geometry.y,
                                        series.point_radius,
                                        series.point_symbol,
                                    ),
                                );
                            }
                            run.push(geometry);
                        });
                        if let Some((row, distance)) = distance_to_interpolated_area(
                            x_css,
                            y_css,
                            &run,
                            baseline_y,
                            series.interpolation.render_type(),
                        ) {
                            consider(row, distance);
                        }
                        continue;
                    }
                    let mut previous: Option<GeneralLinePointGeometry> = None;
                    self.visit_general_path_points(series, |geometry| {
                        if series.point_markers {
                            consider(
                                geometry.row,
                                distance_to_point_symbol(
                                    x_css,
                                    y_css,
                                    geometry.x,
                                    geometry.y,
                                    series.point_radius,
                                    series.point_symbol,
                                ),
                            );
                        }
                        if geometry.starts_new_run {
                            previous = Some(geometry);
                            return;
                        }
                        let Some(from) = previous else {
                            previous = Some(geometry);
                            return;
                        };
                        let (distance, position) = distance_to_area_segment(
                            x_css, y_css, from.x, from.y, geometry.x, geometry.y, baseline_y,
                        );
                        let row = if position <= 0.5 {
                            from.row
                        } else {
                            geometry.row
                        };
                        consider(row, distance);
                        previous = Some(geometry);
                    });
                }
                GeneralSeriesKind::RangeArea => {
                    if series.interpolation != GeneralInterpolation::Linear {
                        let mut run = Vec::new();
                        self.visit_general_range_points(series, |geometry| {
                            if geometry.starts_new_run {
                                if let Some((row, distance)) = distance_to_interpolated_band(
                                    x_css,
                                    y_css,
                                    &run,
                                    series.interpolation.render_type(),
                                ) {
                                    consider(row, distance);
                                }
                                run.clear();
                            }
                            if series.point_markers {
                                consider(
                                    geometry.row,
                                    distance_to_point_symbol(
                                        x_css,
                                        y_css,
                                        geometry.x,
                                        geometry.low_y,
                                        series.point_radius,
                                        series.point_symbol,
                                    )
                                    .min(
                                        distance_to_point_symbol(
                                            x_css,
                                            y_css,
                                            geometry.x,
                                            geometry.high_y,
                                            series.point_radius,
                                            series.point_symbol,
                                        ),
                                    ),
                                );
                            }
                            run.push(geometry);
                        });
                        if let Some((row, distance)) = distance_to_interpolated_band(
                            x_css,
                            y_css,
                            &run,
                            series.interpolation.render_type(),
                        ) {
                            consider(row, distance);
                        }
                        continue;
                    }
                    let mut previous: Option<GeneralRangePointGeometry> = None;
                    self.visit_general_range_points(series, |geometry| {
                        if series.point_markers {
                            consider(
                                geometry.row,
                                distance_to_point_symbol(
                                    x_css,
                                    y_css,
                                    geometry.x,
                                    geometry.low_y,
                                    series.point_radius,
                                    series.point_symbol,
                                )
                                .min(distance_to_point_symbol(
                                    x_css,
                                    y_css,
                                    geometry.x,
                                    geometry.high_y,
                                    series.point_radius,
                                    series.point_symbol,
                                )),
                            );
                        }
                        if geometry.starts_new_run {
                            previous = Some(geometry);
                            return;
                        }
                        let Some(from) = previous else {
                            previous = Some(geometry);
                            return;
                        };
                        let (distance, position) = distance_to_band_segment(
                            x_css,
                            y_css,
                            from.x,
                            from.low_y,
                            from.high_y,
                            geometry.x,
                            geometry.low_y,
                            geometry.high_y,
                        );
                        let row = if position <= 0.5 {
                            from.row
                        } else {
                            geometry.row
                        };
                        consider(row, distance);
                        previous = Some(geometry);
                    });
                }
                GeneralSeriesKind::ErrorBar => {
                    // Every error-bar segment lies inside the bar's bounding box, so a pointer
                    // farther than the hit reach from that box can skip the segment distances.
                    let reach = max_distance + 3.0;
                    self.visit_general_error_bars(series, |geometry| {
                        let cap = geometry.cap_half_size;
                        let [left, right] = error_bar_extent(
                            geometry.x,
                            geometry.x_low,
                            geometry.x_high,
                            geometry.y_low.is_some() || geometry.y_high.is_some(),
                            cap,
                        );
                        let [top, bottom] = error_bar_extent(
                            geometry.y,
                            geometry.y_low,
                            geometry.y_high,
                            geometry.x_low.is_some() || geometry.x_high.is_some(),
                            cap,
                        );
                        if x_css < left - reach
                            || x_css > right + reach
                            || y_css < top - reach
                            || y_css > bottom + reach
                        {
                            return;
                        }
                        consider(
                            geometry.row,
                            (distance_to_error_bar(x_css, y_css, geometry) - 3.0).max(0.0),
                        );
                    });
                }
                GeneralSeriesKind::Column => self.visit_general_columns(series, |geometry| {
                    consider(geometry.row, distance_to_rect(x_css, y_css, geometry));
                }),
                GeneralSeriesKind::RangeBar => self.visit_general_range_bars(series, |geometry| {
                    consider(geometry.row, distance_to_rect(x_css, y_css, geometry));
                }),
                GeneralSeriesKind::HorizontalBar => {
                    self.visit_general_horizontal_bars(series, |geometry| {
                        consider(geometry.row, distance_to_rect(x_css, y_css, geometry));
                    })
                }
                GeneralSeriesKind::BoxPlot => self.visit_general_box_plots(series, |geometry| {
                    consider(
                        geometry.row,
                        (distance_to_box_plot(x_css, y_css, geometry) - 3.0).max(0.0),
                    );
                }),
                GeneralSeriesKind::HeatmapGrid => {
                    self.visit_general_heatmap_cells(series, |geometry| {
                        consider(
                            geometry.row,
                            distance_to_rect(
                                x_css,
                                y_css,
                                GeneralColumnGeometry {
                                    row: geometry.row,
                                    left: geometry.left,
                                    right: geometry.right,
                                    top: geometry.top,
                                    bottom: geometry.bottom,
                                },
                            ),
                        );
                    })
                }
                GeneralSeriesKind::Scatter | GeneralSeriesKind::Bubble => {
                    let expansion = if series.kind == GeneralSeriesKind::Bubble {
                        MAX_GENERAL_POINT_RADIUS
                    } else {
                        series.point_radius
                    } + max_distance;
                    let _ = self.with_scatter_spatial_index(series, |index| {
                        index.visit_candidates(x_css, y_css, expansion, |geometry| {
                            consider(
                                geometry.row,
                                distance_to_point_symbol(
                                    x_css,
                                    y_css,
                                    geometry.x,
                                    geometry.y,
                                    geometry.radius,
                                    if series.kind == GeneralSeriesKind::Bubble {
                                        GeneralPointSymbol::Circle
                                    } else {
                                        series.point_symbol
                                    },
                                ),
                            );
                        });
                    });
                }
            }
            if matches!(mode, GeneralHitMode::Exact)
                && best.as_ref().is_some_and(|hit| hit.distance == 0.0)
            {
                break;
            }
        }
        best
    }

    #[doc(hidden)]
    pub fn update_general_hover(
        &mut self,
        pane_index: usize,
        x_css: f64,
        y_css: f64,
    ) -> Option<GeneralSeriesHit> {
        let hit = self.general_hit_test(pane_index, x_css, y_css, GeneralHitMode::Exact);
        let next = hit.as_ref().map(GeneralInteractionTarget::from);
        let changed = self
            .general_series
            .as_ref()
            .is_some_and(|registry| registry.hovered != next);
        if let Some(registry) = self.general_series.as_mut() {
            registry.hovered = next;
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        hit
    }

    #[doc(hidden)]
    pub fn clear_general_hover(&mut self) {
        let changed = self
            .general_series
            .as_mut()
            .is_some_and(|registry| registry.hovered.take().is_some());
        if changed {
            self.invalidate_frame_overlay();
        }
    }

    #[doc(hidden)]
    pub fn general_hovered_hit(&self) -> Option<GeneralSeriesHit> {
        self.general_series
            .as_ref()?
            .hovered
            .as_ref()
            .map(GeneralInteractionTarget::hit)
    }

    #[doc(hidden)]
    pub fn select_general_hovered(&mut self) -> bool {
        let Some(registry) = self.general_series.as_mut() else {
            return false;
        };
        let next = registry.hovered.clone();
        let hit = next.is_some();
        if registry.selected != next {
            registry.selected = next;
            self.invalidate_frame_overlay();
        }
        hit
    }

    #[doc(hidden)]
    pub fn clear_general_selection(&mut self) {
        let changed = self
            .general_series
            .as_mut()
            .is_some_and(|registry| registry.selected.take().is_some());
        if changed {
            self.invalidate_frame_overlay();
        }
    }

    #[doc(hidden)]
    pub fn general_selected_hit(&self) -> Option<GeneralSeriesHit> {
        self.general_series
            .as_ref()?
            .selected
            .as_ref()
            .map(GeneralInteractionTarget::hit)
    }

    #[doc(hidden)]
    pub fn set_general_brush_from_pixels(
        &mut self,
        axis_id: &str,
        from_css: f64,
        to_css: f64,
    ) -> Result<GeneralBrushSnapshot, ChartError> {
        if !from_css.is_finite() || !to_css.is_finite() {
            return Err(invalid(
                "general brush coordinates must be finite CSS-pixel values",
            ));
        }
        let (pane_id, dimension, scale_type, reverse, padding_inner, padding_outer, domain, plot) = {
            let axis = self.general_axis(axis_id).ok_or_else(|| {
                ChartError::new(ErrorCode::InvalidHandle, "general axis is stale")
            })?;
            if !matches!(axis.dimension(), AxisDimension::X | AxisDimension::Y) {
                return Err(invalid("general brushes require a Cartesian X or Y axis"));
            }
            let pane_id = axis.pane_id();
            let pane = self
                .pane_index_for_id(pane_id)
                .ok_or_else(|| invalid("general brush axis references a stale pane"))?;
            let plot = self
                .general_plot_rect(pane)
                .ok_or_else(|| invalid("general brush pane has no plot rectangle"))?;
            let domain = self
                .effective_general_axis_domain(axis)
                .ok_or_else(|| invalid("general brush axis has no effective domain"))?;
            (
                pane_id,
                axis.dimension(),
                axis.scale(),
                axis.reverse(),
                axis.band_padding_inner(),
                axis.band_padding_outer(),
                domain,
                plot,
            )
        };
        let (range_from, range_to, coordinate_min, coordinate_max) = match dimension {
            AxisDimension::X => {
                let (from, to) = if reverse {
                    (plot.width, 0.0)
                } else {
                    (0.0, plot.width)
                };
                (from, to, 0.0, plot.width)
            }
            AxisDimension::Y => {
                let bottom = plot.y + plot.height;
                let (from, to) = if reverse {
                    (plot.y, bottom)
                } else {
                    (bottom, plot.y)
                };
                (from, to, plot.y, bottom)
            }
            AxisDimension::Angle | AxisDimension::Radius => unreachable!(),
        };
        let from_css = from_css.clamp(coordinate_min, coordinate_max);
        let to_css = to_css.clamp(coordinate_min, coordinate_max);
        let range = match domain {
            GeneralAxisDomain::Numeric(domain) => {
                let scale = NumericAxisScale::new(scale_type, domain, range_from, range_to)
                    .ok_or_else(|| invalid("general brush numeric transform is invalid"))?;
                let from = scale.invert(from_css).ok_or_else(|| {
                    invalid("general brush start is outside the numeric transform")
                })?;
                let to = scale
                    .invert(to_css)
                    .ok_or_else(|| invalid("general brush end is outside the numeric transform"))?;
                GeneralBrushRange::Numeric([from.min(to), from.max(to)])
            }
            GeneralAxisDomain::Temporal(domain) => {
                let scale =
                    LinearScale::new(domain[0] as f64, domain[1] as f64, range_from, range_to)
                        .map_err(|_| invalid("general brush temporal transform is invalid"))?;
                let from = scale
                    .invert(from_css)
                    .ok_or_else(|| {
                        invalid("general brush start is outside the temporal transform")
                    })?
                    .round() as i64;
                let to = scale
                    .invert(to_css)
                    .ok_or_else(|| invalid("general brush end is outside the temporal transform"))?
                    .round() as i64;
                GeneralBrushRange::Temporal([from.min(to), from.max(to)])
            }
            GeneralAxisDomain::Category(categories) => {
                if categories.is_empty() {
                    return Err(invalid("general brush category axis has no categories"));
                }
                let coordinate_for = |index: usize| -> Option<f64> {
                    match scale_type {
                        GeneralScaleType::Band => BandScale::new(
                            categories.len(),
                            range_from,
                            range_to,
                            padding_inner,
                            padding_outer,
                            0.5,
                        )
                        .ok()?
                        .center(index),
                        GeneralScaleType::Point => PointScale::new(
                            categories.len(),
                            range_from,
                            range_to,
                            padding_outer,
                            0.5,
                        )
                        .ok()?
                        .coordinate(index),
                        _ => None,
                    }
                };
                let nearest = |coordinate: f64| -> Option<usize> {
                    (0..categories.len())
                        .filter_map(|index| {
                            coordinate_for(index).map(|mapped| (index, (mapped - coordinate).abs()))
                        })
                        .min_by(|left, right| left.1.total_cmp(&right.1))
                        .map(|(index, _)| index)
                };
                let from = nearest(from_css)
                    .ok_or_else(|| invalid("general brush category transform is invalid"))?;
                let to = nearest(to_css)
                    .ok_or_else(|| invalid("general brush category transform is invalid"))?;
                let (from, to) = if from <= to { (from, to) } else { (to, from) };
                GeneralBrushRange::Category([categories[from].clone(), categories[to].clone()])
            }
            GeneralAxisDomain::Auto => {
                return Err(invalid("general brush axis has no resolved domain"));
            }
        };

        let registry = self
            .general_series
            .as_mut()
            .ok_or_else(|| invalid("general brush requires at least one general series"))?;
        registry.brush = Some(GeneralBrushSelection {
            pane_id,
            axis_id: axis_id.to_owned(),
            dimension,
            range,
        });
        self.invalidate_frame_overlay();
        self.general_brush_snapshot()
            .ok_or_else(|| invalid("general brush could not resolve its snapshot"))
    }

    #[doc(hidden)]
    pub fn clear_general_brush(&mut self) {
        let changed = self
            .general_series
            .as_mut()
            .is_some_and(|registry| registry.brush.take().is_some());
        if changed {
            self.invalidate_frame_overlay();
        }
    }

    #[doc(hidden)]
    pub fn general_brush_snapshot(&self) -> Option<GeneralBrushSnapshot> {
        let registry = self.general_series.as_ref()?;
        let brush = registry.brush.as_ref()?;
        let pane = self.pane_index_for_id(brush.pane_id)?;
        let axis = self.general_axis(&brush.axis_id)?;
        let domain = self.effective_general_axis_domain(axis)?;
        let mut items = Vec::new();
        for series in registry.series.iter().filter(|series| {
            series.visible
                && series.pane_id == brush.pane_id
                && match brush.dimension {
                    AxisDimension::X => series.x_axis_id == brush.axis_id,
                    AxisDimension::Y => series.y_axis_id == brush.axis_id,
                    AxisDimension::Angle | AxisDimension::Radius => false,
                }
        }) {
            let Some(dataset) = self.general_dataset(series.dataset) else {
                continue;
            };
            for row in 0..dataset.len() {
                if !general_row_matches_brush(series, dataset, row, brush, &domain) {
                    continue;
                }
                let Some(row_id) = dataset.row_identity(row).cloned() else {
                    continue;
                };
                items.push(GeneralSeriesHit {
                    series: series.id,
                    row,
                    row_id,
                    distance: 0.0,
                });
                if items.len() >= MAX_GENERAL_BRUSH_ITEMS {
                    return Some(GeneralBrushSnapshot {
                        pane,
                        axis_id: brush.axis_id.clone(),
                        dimension: brush.dimension,
                        range: brush.range.clone(),
                        items,
                    });
                }
            }
        }
        Some(GeneralBrushSnapshot {
            pane,
            axis_id: brush.axis_id.clone(),
            dimension: brush.dimension,
            range: brush.range.clone(),
            items,
        })
    }

    pub(crate) fn general_brush_axis_bounds(
        &self,
        pane_index: usize,
    ) -> Option<(AxisDimension, f64, f64)> {
        let registry = self.general_series.as_ref()?;
        let brush = registry.brush.as_ref()?;
        if self.pane_index_for_id(brush.pane_id)? != pane_index {
            return None;
        }
        let axis = self.general_axis(&brush.axis_id)?;
        let domain = self.effective_general_axis_domain(axis)?;
        let plot = self.general_plot_rect(pane_index)?;
        let (range_from, range_to) = match brush.dimension {
            AxisDimension::X => {
                if axis.reverse() {
                    (plot.width, 0.0)
                } else {
                    (0.0, plot.width)
                }
            }
            AxisDimension::Y => {
                let bottom = plot.y + plot.height;
                if axis.reverse() {
                    (plot.y, bottom)
                } else {
                    (bottom, plot.y)
                }
            }
            AxisDimension::Angle | AxisDimension::Radius => return None,
        };
        let bounds = match (&brush.range, domain) {
            (GeneralBrushRange::Numeric(values), GeneralAxisDomain::Numeric(domain)) => {
                let scale = NumericAxisScale::new(axis.scale(), domain, range_from, range_to)?;
                (scale.coordinate(values[0])?, scale.coordinate(values[1])?)
            }
            (GeneralBrushRange::Temporal(values), GeneralAxisDomain::Temporal(domain)) => {
                let scale =
                    LinearScale::new(domain[0] as f64, domain[1] as f64, range_from, range_to)
                        .ok()?;
                (
                    scale.coordinate(values[0] as f64)?,
                    scale.coordinate(values[1] as f64)?,
                )
            }
            (GeneralBrushRange::Category(values), GeneralAxisDomain::Category(categories)) => {
                let from = categories.iter().position(|value| value == &values[0])?;
                let to = categories.iter().position(|value| value == &values[1])?;
                match axis.scale() {
                    GeneralScaleType::Band => {
                        let scale = BandScale::new(
                            categories.len(),
                            range_from,
                            range_to,
                            axis.band_padding_inner(),
                            axis.band_padding_outer(),
                            0.5,
                        )
                        .ok()?;
                        let from = scale.bounds(from)?;
                        let to = scale.bounds(to)?;
                        (
                            from.0.min(from.1).min(to.0.min(to.1)),
                            from.0.max(from.1).max(to.0.max(to.1)),
                        )
                    }
                    GeneralScaleType::Point => {
                        let scale = PointScale::new(
                            categories.len(),
                            range_from,
                            range_to,
                            axis.band_padding_outer(),
                            0.5,
                        )
                        .ok()?;
                        (scale.coordinate(from)?, scale.coordinate(to)?)
                    }
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some((
            brush.dimension,
            bounds.0.min(bounds.1),
            bounds.0.max(bounds.1),
        ))
    }

    #[doc(hidden)]
    pub fn set_general_accessibility_focus(
        &mut self,
        series_id: GeneralSeriesId,
        row: usize,
    ) -> bool {
        let Some(series) = self.general_series(series_id) else {
            return false;
        };
        let dataset_id = series.dataset;
        let Some(row_id) = self
            .general_dataset(dataset_id)
            .and_then(|dataset| dataset.row_identity(row))
            .cloned()
        else {
            return false;
        };
        let next = Some(GeneralInteractionTarget {
            series: series_id,
            row,
            row_id,
            distance: 0.0,
        });
        let changed = self
            .general_series
            .as_ref()
            .is_some_and(|registry| registry.accessibility_focused != next);
        if let Some(registry) = self.general_series.as_mut() {
            registry.accessibility_focused = next;
        }
        if changed {
            self.invalidate_frame_overlay();
        }
        true
    }

    #[doc(hidden)]
    pub fn clear_general_accessibility_focus(&mut self) {
        let changed = self
            .general_series
            .as_mut()
            .is_some_and(|registry| registry.accessibility_focused.take().is_some());
        if changed {
            self.invalidate_frame_overlay();
        }
    }

    #[doc(hidden)]
    pub fn general_accessibility_focused_hit(&self) -> Option<GeneralSeriesHit> {
        self.general_series
            .as_ref()?
            .accessibility_focused
            .as_ref()
            .map(GeneralInteractionTarget::hit)
    }

    pub(crate) fn general_row_interaction(
        &self,
        series: GeneralSeriesId,
        row: usize,
    ) -> (bool, bool) {
        let Some(registry) = self.general_series.as_ref() else {
            return (false, false);
        };
        let matches =
            |target: &GeneralInteractionTarget| target.series == series && target.row == row;
        (
            registry.hovered.as_ref().is_some_and(matches)
                || registry.accessibility_focused.as_ref().is_some_and(matches),
            registry.selected.as_ref().is_some_and(matches),
        )
    }

    pub(crate) fn reconcile_general_interaction_for_dataset(
        &mut self,
        dataset_id: GeneralDatasetId,
        removed_front: usize,
    ) {
        let Some(registry) = self.general_series.as_ref() else {
            return;
        };
        let dataset_series: Vec<GeneralSeriesId> = registry
            .series
            .iter()
            .filter(|series| series.dataset == dataset_id)
            .map(GeneralSeries::id)
            .collect();
        let previous_hovered = registry.hovered.clone();
        let previous_selected = registry.selected.clone();
        let previous_accessibility_focused = registry.accessibility_focused.clone();
        let Some(dataset) = self.general_dataset(dataset_id) else {
            return;
        };
        let reconcile = |target: Option<GeneralInteractionTarget>| {
            let mut current = target?;
            if dataset_series.contains(&current.series) {
                let shifted = current.row.checked_sub(removed_front);
                current.row = shifted
                    .filter(|&row| dataset.row_identity(row) == Some(&current.row_id))
                    .or_else(|| {
                        (0..dataset.len())
                            .find(|&row| dataset.row_identity(row) == Some(&current.row_id))
                    })?;
            }
            Some(current)
        };
        let next_hovered = reconcile(previous_hovered.clone());
        let next_selected = reconcile(previous_selected.clone());
        let next_accessibility_focused = reconcile(previous_accessibility_focused.clone());
        if previous_hovered != next_hovered
            || previous_selected != next_selected
            || previous_accessibility_focused != next_accessibility_focused
        {
            if let Some(registry) = self.general_series.as_mut() {
                registry.hovered = next_hovered;
                registry.selected = next_selected;
                registry.accessibility_focused = next_accessibility_focused;
            }
            self.invalidate_frame_overlay();
        }
    }

    #[cfg(test)]
    pub(crate) fn general_scatter_hit_candidate_count(
        &self,
        series_id: GeneralSeriesId,
        x_css: f64,
        y_css: f64,
        max_distance: f64,
    ) -> Option<usize> {
        let series = self.general_series(series_id)?;
        if !matches!(
            series.kind,
            GeneralSeriesKind::Scatter | GeneralSeriesKind::Bubble
        ) || !x_css.is_finite()
            || !y_css.is_finite()
            || !max_distance.is_finite()
            || max_distance < 0.0
        {
            return None;
        }
        self.with_scatter_spatial_index(series, |index| {
            let mut count = 0usize;
            let expansion = if series.kind == GeneralSeriesKind::Bubble {
                MAX_GENERAL_POINT_RADIUS
            } else {
                series.point_radius
            } + max_distance;
            index.visit_candidates(x_css, y_css, expansion, |_| count += 1);
            count
        })
    }

    #[doc(hidden)]
    pub fn general_tooltip_snapshot(
        &self,
        series_id: GeneralSeriesId,
        row: usize,
    ) -> Option<GeneralTooltipSnapshot> {
        let series = self.general_series(series_id)?;
        let dataset = self.general_dataset(series.dataset)?;
        let row_id = dataset.row_identity(row)?.clone();
        let x_label = general_x_label(dataset, row)?;
        let y_label = (series.kind == GeneralSeriesKind::HeatmapGrid)
            .then(|| general_heatmap_y_label(dataset, row))
            .flatten();
        let is_box_plot = series.kind == GeneralSeriesKind::BoxPlot;
        Some(GeneralTooltipSnapshot {
            series: series_id,
            row,
            row_id,
            x_label,
            y_label,
            label: dataset.row_label(row).map(str::to_owned),
            value: dataset.y_is_valid(row).then(|| dataset.y()[row]),
            low: dataset
                .low()
                .and_then(|values| dataset.low_is_valid(row).then(|| values[row])),
            high: dataset.high().map_or_else(
                || {
                    dataset
                        .low()
                        .and_then(|_| dataset.y_is_valid(row).then(|| dataset.y()[row]))
                },
                |values| dataset.high_is_valid(row).then(|| values[row]),
            ),
            x_low: (!is_box_plot)
                .then(|| {
                    dataset
                        .x_low()
                        .and_then(|values| dataset.x_low_is_valid(row).then(|| values[row]))
                })
                .flatten(),
            x_high: (!is_box_plot)
                .then(|| {
                    dataset
                        .x_high()
                        .and_then(|values| dataset.x_high_is_valid(row).then(|| values[row]))
                })
                .flatten(),
            q1: is_box_plot
                .then(|| {
                    dataset
                        .x_low()
                        .and_then(|values| dataset.x_low_is_valid(row).then(|| values[row]))
                })
                .flatten(),
            q3: is_box_plot
                .then(|| {
                    dataset
                        .x_high()
                        .and_then(|values| dataset.x_high_is_valid(row).then(|| values[row]))
                })
                .flatten(),
            size: dataset
                .size()
                .and_then(|values| dataset.size_is_valid(row).then(|| values[row])),
            title: series.title.clone(),
        })
    }

    #[doc(hidden)]
    pub fn general_shared_tooltip_snapshot(
        &self,
        series_id: GeneralSeriesId,
        row: usize,
    ) -> Option<GeneralSharedTooltipSnapshot> {
        let anchor_series = self.general_series(series_id)?;
        let anchor_dataset = self.general_dataset(anchor_series.dataset)?;
        anchor_dataset.row_identity(row)?;
        let pane = self.pane_index_for_id(anchor_series.pane_id)?;
        let mut items = Vec::new();

        for candidate in self
            .general_series_iter()
            .filter(|candidate| candidate.visible && candidate.pane_id == anchor_series.pane_id)
        {
            let Some(dataset) = self.general_dataset(candidate.dataset) else {
                continue;
            };
            for candidate_row in 0..dataset.len() {
                if !general_rows_share_horizontal_datum(anchor_dataset, row, dataset, candidate_row)
                {
                    continue;
                }
                if let Some(snapshot) = self.general_tooltip_snapshot(candidate.id, candidate_row) {
                    items.push(snapshot);
                    if items.len() >= MAX_GENERAL_SHARED_TOOLTIP_ITEMS {
                        return Some(GeneralSharedTooltipSnapshot {
                            pane,
                            anchor_series: series_id,
                            anchor_row: row,
                            items,
                        });
                    }
                }
            }
        }

        Some(GeneralSharedTooltipSnapshot {
            pane,
            anchor_series: series_id,
            anchor_row: row,
            items,
        })
    }

    #[doc(hidden)]
    pub fn general_legend_snapshot(&self, pane: Option<usize>) -> GeneralLegendSnapshot {
        let pane_id = pane.and_then(|index| self.pane_stable_id(index));
        if pane.is_some() && pane_id.is_none() {
            return GeneralLegendSnapshot { items: Vec::new() };
        }
        let mut items = Vec::with_capacity(self.general_series_count().min(MAX_GENERAL_SERIES));
        for series in self.general_series_iter() {
            if pane_id.is_some_and(|id| series.pane_id != id) {
                continue;
            }
            let Some(pane) = self.pane_index_for_id(series.pane_id) else {
                continue;
            };
            items.push(GeneralLegendItem {
                series: series.id,
                pane,
                kind: series.kind,
                title: series.title.clone(),
                color: series.color.clone(),
                visible: series.visible,
            });
        }
        GeneralLegendSnapshot { items }
    }

    #[doc(hidden)]
    pub fn general_accessibility_snapshot(
        &self,
        series_id: GeneralSeriesId,
        offset: usize,
        limit: usize,
    ) -> Option<GeneralAccessibilitySnapshot> {
        let series = self.general_series(series_id)?;
        let dataset = self.general_dataset(series.dataset)?;
        let total_rows = dataset.len();
        let offset = offset.min(total_rows);
        let end = offset
            .saturating_add(limit.min(MAX_GENERAL_ACCESSIBILITY_ITEMS))
            .min(total_rows);
        let mut items = Vec::with_capacity(end - offset);
        for row in offset..end {
            let row_id = dataset.row_identity(row)?.clone();
            let x_label = general_x_label(dataset, row)?;
            let y_label = (series.kind == GeneralSeriesKind::HeatmapGrid)
                .then(|| general_heatmap_y_label(dataset, row))
                .flatten();
            let is_box_plot = series.kind == GeneralSeriesKind::BoxPlot;
            items.push(GeneralAccessibilityItem {
                row,
                row_id,
                x_label,
                y_label,
                label: dataset.row_label(row).map(str::to_owned),
                value: dataset.y_is_valid(row).then(|| dataset.y()[row]),
                low: dataset
                    .low()
                    .and_then(|values| dataset.low_is_valid(row).then(|| values[row])),
                high: dataset.high().map_or_else(
                    || {
                        dataset
                            .low()
                            .and_then(|_| dataset.y_is_valid(row).then(|| dataset.y()[row]))
                    },
                    |values| dataset.high_is_valid(row).then(|| values[row]),
                ),
                x_low: (!is_box_plot)
                    .then(|| {
                        dataset
                            .x_low()
                            .and_then(|values| dataset.x_low_is_valid(row).then(|| values[row]))
                    })
                    .flatten(),
                x_high: (!is_box_plot)
                    .then(|| {
                        dataset
                            .x_high()
                            .and_then(|values| dataset.x_high_is_valid(row).then(|| values[row]))
                    })
                    .flatten(),
                q1: is_box_plot
                    .then(|| {
                        dataset
                            .x_low()
                            .and_then(|values| dataset.x_low_is_valid(row).then(|| values[row]))
                    })
                    .flatten(),
                q3: is_box_plot
                    .then(|| {
                        dataset
                            .x_high()
                            .and_then(|values| dataset.x_high_is_valid(row).then(|| values[row]))
                    })
                    .flatten(),
                size: dataset
                    .size()
                    .and_then(|values| dataset.size_is_valid(row).then(|| values[row])),
            });
        }
        Some(GeneralAccessibilitySnapshot {
            series: series_id,
            title: series.title.clone(),
            total_rows,
            offset,
            items,
        })
    }
}

fn distance_to_rect(x: f64, y: f64, geometry: GeneralColumnGeometry) -> f64 {
    let dx = if x < geometry.left {
        geometry.left - x
    } else if x > geometry.right {
        x - geometry.right
    } else {
        0.0
    };
    let dy = if y < geometry.top {
        geometry.top - y
    } else if y > geometry.bottom {
        y - geometry.bottom
    } else {
        0.0
    };
    dx.hypot(dy)
}

fn column_stack_matches(reference: &GeneralSeries, candidate: &GeneralSeries) -> bool {
    reference.kind == GeneralSeriesKind::Column
        && candidate.kind == GeneralSeriesKind::Column
        && reference.pane_id == candidate.pane_id
        && reference.x_axis_id == candidate.x_axis_id
        && reference.y_axis_id == candidate.y_axis_id
        && reference.group_id == candidate.group_id
        && reference.stack_id.is_some()
        && reference.stack_id == candidate.stack_id
        && reference.stack_mode == candidate.stack_mode
}

fn horizontal_bar_stack_matches(reference: &GeneralSeries, candidate: &GeneralSeries) -> bool {
    reference.kind == GeneralSeriesKind::HorizontalBar
        && candidate.kind == GeneralSeriesKind::HorizontalBar
        && reference.pane_id == candidate.pane_id
        && reference.x_axis_id == candidate.x_axis_id
        && reference.y_axis_id == candidate.y_axis_id
        && reference.group_id == candidate.group_id
        && reference.stack_id.is_some()
        && reference.stack_id == candidate.stack_id
        && reference.stack_mode == candidate.stack_mode
}

fn area_stack_matches(reference: &GeneralSeries, candidate: &GeneralSeries) -> bool {
    reference.kind == GeneralSeriesKind::XyArea
        && candidate.kind == GeneralSeriesKind::XyArea
        && reference.pane_id == candidate.pane_id
        && reference.x_axis_id == candidate.x_axis_id
        && reference.y_axis_id == candidate.y_axis_id
        && reference.stack_id.is_some()
        && reference.stack_id == candidate.stack_id
        && reference.stack_mode == candidate.stack_mode
}

fn general_stack_x_key(dataset: &crate::GeneralDataset, row: usize) -> Option<GeneralStackXKey> {
    match dataset.x_kind() {
        GeneralXKind::Numeric => {
            dataset
                .numeric_x()
                .and_then(|values| values.get(row))
                .map(|value| {
                    GeneralStackXKey::Numeric(if *value == 0.0 {
                        0.0f64.to_bits()
                    } else {
                        value.to_bits()
                    })
                })
        }
        GeneralXKind::Temporal => dataset
            .temporal_x_epoch_ms()
            .and_then(|values| values.get(row))
            .copied()
            .map(GeneralStackXKey::Temporal),
        GeneralXKind::Category => {
            let category_index = dataset
                .category_indices()
                .and_then(|values| values.get(row))
                .and_then(|value| usize::try_from(*value).ok())?;
            dataset
                .categories()
                .and_then(|values| values.get(category_index))
                .cloned()
                .map(GeneralStackXKey::Category)
        }
    }
}

fn accumulate_area_values<F>(engine: &ChartEngine, series: &GeneralSeries, mut visit: F)
where
    F: FnMut(GeneralStackXKey, f64),
{
    let Some(dataset) = engine.general_dataset(series.dataset) else {
        return;
    };
    for (row, &value) in dataset.y().iter().enumerate() {
        if !dataset.y_is_valid(row) {
            continue;
        }
        let Some(x) = general_stack_x_key(dataset, row) else {
            continue;
        };
        visit(x, value);
    }
}

fn accumulate_column_values<F>(
    engine: &ChartEngine,
    series: &GeneralSeries,
    axis_lookup: &HashMap<&str, usize>,
    mut visit: F,
) where
    F: FnMut(usize, f64),
{
    let Some(dataset) = engine.general_dataset(series.dataset) else {
        return;
    };
    let (Some(categories), Some(indices)) = (dataset.categories(), dataset.category_indices())
    else {
        return;
    };
    for (row, &category_index) in indices.iter().enumerate() {
        if !dataset.y_is_valid(row) {
            continue;
        }
        let Some(category) = usize::try_from(category_index)
            .ok()
            .and_then(|index| categories.get(index))
        else {
            continue;
        };
        let Some(&axis_index) = axis_lookup.get(category.as_str()) else {
            continue;
        };
        visit(axis_index, dataset.y()[row]);
    }
}

fn extend_numeric_pair(bounds: &mut Option<(f64, f64)>, value: f64) {
    *bounds = Some(match *bounds {
        Some((low, high)) => (low.min(value), high.max(value)),
        None => (value, value),
    });
}

fn distance_to_point_symbol(
    x: f64,
    y: f64,
    center_x: f64,
    center_y: f64,
    radius: f64,
    symbol: GeneralPointSymbol,
) -> f64 {
    let dx = x - center_x;
    let dy = y - center_y;
    match symbol {
        GeneralPointSymbol::Circle => (dx.hypot(dy) - radius).max(0.0),
        GeneralPointSymbol::Square => (dx.abs() - radius)
            .max(0.0)
            .hypot((dy.abs() - radius).max(0.0)),
        GeneralPointSymbol::Diamond => {
            if dx.abs() + dy.abs() <= radius {
                return 0.0;
            }
            let vertices = [
                (center_x, center_y - radius),
                (center_x + radius, center_y),
                (center_x, center_y + radius),
                (center_x - radius, center_y),
            ];
            vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .take(vertices.len())
                .map(|(&(x0, y0), &(x1, y1))| distance_to_segment(x, y, x0, y0, x1, y1).0)
                .fold(f64::INFINITY, f64::min)
        }
        GeneralPointSymbol::Triangle => {
            let vertices = [
                (center_x, center_y - radius),
                (center_x + radius, center_y + radius),
                (center_x - radius, center_y + radius),
            ];
            let cross =
                |a: (f64, f64), b: (f64, f64)| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
            let signs = [
                cross(vertices[0], vertices[1]),
                cross(vertices[1], vertices[2]),
                cross(vertices[2], vertices[0]),
            ];
            if signs.iter().all(|value| *value >= 0.0) || signs.iter().all(|value| *value <= 0.0) {
                return 0.0;
            }
            vertices
                .iter()
                .zip(vertices.iter().cycle().skip(1))
                .take(vertices.len())
                .map(|(&(x0, y0), &(x1, y1))| distance_to_segment(x, y, x0, y0, x1, y1).0)
                .fold(f64::INFINITY, f64::min)
        }
    }
}

/// The span of an error bar along one axis: the center, its own error bounds, and the caps drawn
/// across it by the other axis's error bounds.
fn error_bar_extent(
    center: f64,
    low: Option<f64>,
    high: Option<f64>,
    crossed_by_caps: bool,
    cap_half_size: f64,
) -> [f64; 2] {
    let mut from = center;
    let mut to = center;
    for bound in [low, high].into_iter().flatten() {
        from = from.min(bound);
        to = to.max(bound);
    }
    if crossed_by_caps {
        from = from.min(center - cap_half_size);
        to = to.max(center + cap_half_size);
    }
    [from, to]
}

fn distance_to_error_bar(x: f64, y: f64, geometry: GeneralErrorBarGeometry) -> f64 {
    let mut distance = (x - geometry.x).hypot(y - geometry.y);
    if geometry.x_low.is_some() || geometry.x_high.is_some() {
        let from = geometry.x_low.unwrap_or(geometry.x);
        let to = geometry.x_high.unwrap_or(geometry.x);
        distance = distance.min(distance_to_segment(x, y, from, geometry.y, to, geometry.y).0);
        for bound in [geometry.x_low, geometry.x_high].into_iter().flatten() {
            distance = distance.min(
                distance_to_segment(
                    x,
                    y,
                    bound,
                    geometry.y - geometry.cap_half_size,
                    bound,
                    geometry.y + geometry.cap_half_size,
                )
                .0,
            );
        }
    }
    if geometry.y_low.is_some() || geometry.y_high.is_some() {
        let from = geometry.y_low.unwrap_or(geometry.y);
        let to = geometry.y_high.unwrap_or(geometry.y);
        distance = distance.min(distance_to_segment(x, y, geometry.x, from, geometry.x, to).0);
        for bound in [geometry.y_low, geometry.y_high].into_iter().flatten() {
            distance = distance.min(
                distance_to_segment(
                    x,
                    y,
                    geometry.x - geometry.cap_half_size,
                    bound,
                    geometry.x + geometry.cap_half_size,
                    bound,
                )
                .0,
            );
        }
    }
    distance
}

fn distance_to_box_plot(x: f64, y: f64, geometry: GeneralBoxPlotGeometry) -> f64 {
    let box_top = geometry.q1_y.min(geometry.q3_y);
    let box_bottom = geometry.q1_y.max(geometry.q3_y);
    if x >= geometry.left && x <= geometry.right && y >= box_top && y <= box_bottom {
        return 0.0;
    }
    [
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.q1_y,
            geometry.right,
            geometry.q1_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.q3_y,
            geometry.right,
            geometry.q3_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.median_y,
            geometry.right,
            geometry.median_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.q1_y,
            geometry.left,
            geometry.q3_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.right,
            geometry.q1_y,
            geometry.right,
            geometry.q3_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.center_x,
            geometry.min_y,
            geometry.center_x,
            geometry.q1_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.center_x,
            geometry.q3_y,
            geometry.center_x,
            geometry.max_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.min_y,
            geometry.right,
            geometry.min_y,
        )
        .0,
        distance_to_segment(
            x,
            y,
            geometry.left,
            geometry.max_y,
            geometry.right,
            geometry.max_y,
        )
        .0,
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

fn distance_to_segment(x: f64, y: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length_squared = dx * dx + dy * dy;
    if !length_squared.is_finite() || length_squared <= f64::EPSILON {
        return ((x - x0).hypot(y - y0), 0.0);
    }
    let position = (((x - x0) * dx + (y - y0) * dy) / length_squared).clamp(0.0, 1.0);
    let nearest_x = x0 + dx * position;
    let nearest_y = y0 + dy * position;
    ((x - nearest_x).hypot(y - nearest_y), position)
}

fn nearest_line_row(points: &[GeneralLinePointGeometry], x: f64, y: f64) -> Option<usize> {
    points
        .iter()
        .min_by(|left, right| {
            (left.x - x)
                .hypot(left.y - y)
                .total_cmp(&(right.x - x).hypot(right.y - y))
        })
        .map(|point| point.row)
}

fn nearest_band_row(points: &[GeneralRangePointGeometry], x: f64, y: f64) -> Option<usize> {
    points
        .iter()
        .min_by(|left, right| {
            let left_y = (left.low_y + left.high_y) * 0.5;
            let right_y = (right.low_y + right.high_y) * 0.5;
            (left.x - x)
                .hypot(left_y - y)
                .total_cmp(&(right.x - x).hypot(right_y - y))
        })
        .map(|point| point.row)
}

fn distance_to_interpolated_line(
    x: f64,
    y: f64,
    points: &[GeneralLinePointGeometry],
    line_type: LineType,
) -> Option<(usize, f64)> {
    if points.len() < 2 {
        return None;
    }
    let source = points
        .iter()
        .map(|point| LinePoint {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    let mut expanded = Vec::new();
    expand_line_into(&source, line_type, 1.0, 1.0, &mut expanded);
    let mut best = (f64::INFINITY, 0.0, 0.0);
    for pair in expanded.windows(2) {
        let (distance, position) =
            distance_to_segment(x, y, pair[0].x, pair[0].y, pair[1].x, pair[1].y);
        if distance < best.0 {
            best = (
                distance,
                pair[0].x + (pair[1].x - pair[0].x) * position,
                pair[0].y + (pair[1].y - pair[0].y) * position,
            );
        }
    }
    nearest_line_row(points, best.1, best.2).map(|row| (row, best.0))
}

fn distance_to_interpolated_area(
    x: f64,
    y: f64,
    points: &[GeneralLinePointGeometry],
    baseline_y: f64,
    line_type: LineType,
) -> Option<(usize, f64)> {
    if points.len() < 2 {
        return None;
    }
    let source = points
        .iter()
        .map(|point| LinePoint {
            x: point.x,
            y: point.y,
        })
        .collect::<Vec<_>>();
    let mut expanded = Vec::new();
    expand_line_into(&source, line_type, 1.0, 1.0, &mut expanded);
    let mut best = (f64::INFINITY, 0.0, 0.0);
    for pair in expanded.windows(2) {
        let (distance, position) =
            distance_to_area_segment(x, y, pair[0].x, pair[0].y, pair[1].x, pair[1].y, baseline_y);
        if distance < best.0 {
            best = (
                distance,
                pair[0].x + (pair[1].x - pair[0].x) * position,
                pair[0].y + (pair[1].y - pair[0].y) * position,
            );
        }
    }
    nearest_line_row(points, best.1, best.2).map(|row| (row, best.0))
}

fn distance_to_interpolated_band(
    x: f64,
    y: f64,
    points: &[GeneralRangePointGeometry],
    line_type: LineType,
) -> Option<(usize, f64)> {
    if points.len() < 2 {
        return None;
    }
    let upper = points
        .iter()
        .map(|point| LinePoint {
            x: point.x,
            y: point.high_y,
        })
        .collect::<Vec<_>>();
    let lower = points
        .iter()
        .map(|point| LinePoint {
            x: point.x,
            y: point.low_y,
        })
        .collect::<Vec<_>>();
    let mut expanded_upper = Vec::new();
    let mut expanded_lower = Vec::new();
    expand_band_into(
        &upper,
        &lower,
        line_type,
        1.0,
        1.0,
        &mut expanded_upper,
        &mut expanded_lower,
    );
    let mut best = (f64::INFINITY, 0.0, 0.0);
    for index in 0..expanded_upper
        .len()
        .min(expanded_lower.len())
        .saturating_sub(1)
    {
        let upper_from = expanded_upper[index];
        let upper_to = expanded_upper[index + 1];
        let lower_from = expanded_lower[index];
        let lower_to = expanded_lower[index + 1];
        let (distance, position) = distance_to_band_segment(
            x,
            y,
            upper_from.x,
            lower_from.y,
            upper_from.y,
            upper_to.x,
            lower_to.y,
            upper_to.y,
        );
        if distance < best.0 {
            let low_y = lower_from.y + (lower_to.y - lower_from.y) * position;
            let high_y = upper_from.y + (upper_to.y - upper_from.y) * position;
            best = (
                distance,
                upper_from.x + (upper_to.x - upper_from.x) * position,
                (low_y + high_y) * 0.5,
            );
        }
    }
    nearest_band_row(points, best.1, best.2).map(|row| (row, best.0))
}

fn heatmap_center_bounds(
    center: f64,
    centers: &[f64],
    range_min: f64,
    range_max: f64,
) -> Option<(f64, f64)> {
    if !center.is_finite() || centers.is_empty() {
        return None;
    }
    let index = centers
        .binary_search_by(|candidate| candidate.total_cmp(&center))
        .ok()?;
    let low = if centers.len() == 1 {
        range_min
    } else if index == 0 {
        center - (centers[1] - center) * 0.5
    } else {
        (centers[index - 1] + center) * 0.5
    };
    let high = if centers.len() == 1 {
        range_max
    } else if index + 1 == centers.len() {
        center + (center - centers[index - 1]) * 0.5
    } else {
        (center + centers[index + 1]) * 0.5
    };
    let low = low.clamp(range_min, range_max);
    let high = high.clamp(range_min, range_max);
    (high > low).then_some((low, high))
}

fn distance_to_area_segment(
    x: f64,
    y: f64,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    baseline_y: f64,
) -> (f64, f64) {
    let (_, projected) = distance_to_segment(x, y, x0, y0, x1, y1);
    let position = if (x1 - x0).abs() > f64::EPSILON {
        ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
    } else {
        projected
    };
    let top_y = y0 + (y1 - y0) * position;
    if x >= x0.min(x1)
        && x <= x0.max(x1)
        && y >= top_y.min(baseline_y)
        && y <= top_y.max(baseline_y)
    {
        return (0.0, position);
    }
    let distance = [
        distance_to_segment(x, y, x0, y0, x1, y1).0,
        distance_to_segment(x, y, x0, baseline_y, x1, baseline_y).0,
        distance_to_segment(x, y, x0, y0, x0, baseline_y).0,
        distance_to_segment(x, y, x1, y1, x1, baseline_y).0,
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    (distance, position)
}

#[allow(clippy::too_many_arguments)]
fn distance_to_band_segment(
    x: f64,
    y: f64,
    x0: f64,
    low0: f64,
    high0: f64,
    x1: f64,
    low1: f64,
    high1: f64,
) -> (f64, f64) {
    let (_, projected) = distance_to_segment(x, y, x0, high0, x1, high1);
    let position = if (x1 - x0).abs() > f64::EPSILON {
        ((x - x0) / (x1 - x0)).clamp(0.0, 1.0)
    } else {
        projected
    };
    let low_y = low0 + (low1 - low0) * position;
    let high_y = high0 + (high1 - high0) * position;
    if x >= x0.min(x1) && x <= x0.max(x1) && y >= low_y.min(high_y) && y <= low_y.max(high_y) {
        return (0.0, position);
    }
    let distance = [
        distance_to_segment(x, y, x0, low0, x1, low1).0,
        distance_to_segment(x, y, x0, high0, x1, high1).0,
        distance_to_segment(x, y, x0, low0, x0, high0).0,
        distance_to_segment(x, y, x1, low1, x1, high1).0,
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    (distance, position)
}

fn general_x_label(dataset: &crate::GeneralDataset, row: usize) -> Option<String> {
    if let Some(values) = dataset.numeric_x() {
        return values.get(row).map(ToString::to_string);
    }
    if let Some(values) = dataset.temporal_x_epoch_ms() {
        return values.get(row).map(ToString::to_string);
    }
    let category_index = usize::try_from(*dataset.category_indices()?.get(row)?).ok()?;
    dataset.categories()?.get(category_index).cloned()
}

fn general_rows_share_horizontal_datum(
    left: &crate::GeneralDataset,
    left_row: usize,
    right: &crate::GeneralDataset,
    right_row: usize,
) -> bool {
    if left.x_kind() != right.x_kind() {
        return false;
    }
    match left.x_kind() {
        GeneralXKind::Numeric => left
            .numeric_x()
            .and_then(|values| values.get(left_row))
            .zip(right.numeric_x().and_then(|values| values.get(right_row)))
            .is_some_and(|(left, right)| left.to_bits() == right.to_bits()),
        GeneralXKind::Temporal => left
            .temporal_x_epoch_ms()
            .and_then(|values| values.get(left_row))
            .zip(
                right
                    .temporal_x_epoch_ms()
                    .and_then(|values| values.get(right_row)),
            )
            .is_some_and(|(left, right)| left == right),
        GeneralXKind::Category => general_x_label(left, left_row)
            .zip(general_x_label(right, right_row))
            .is_some_and(|(left, right)| left == right),
    }
}

fn general_row_matches_brush(
    series: &GeneralSeries,
    dataset: &crate::GeneralDataset,
    row: usize,
    brush: &GeneralBrushSelection,
    domain: &GeneralAxisDomain,
) -> bool {
    match (&brush.range, domain) {
        (GeneralBrushRange::Numeric([from, to]), GeneralAxisDomain::Numeric(_)) => {
            let value = match brush.dimension {
                AxisDimension::X => {
                    if series.kind == GeneralSeriesKind::HorizontalBar {
                        dataset.y_is_valid(row).then(|| dataset.y()[row])
                    } else {
                        dataset
                            .numeric_x()
                            .and_then(|values| values.get(row))
                            .copied()
                    }
                }
                AxisDimension::Y => {
                    if series.kind == GeneralSeriesKind::HorizontalBar {
                        None
                    } else if series.kind == GeneralSeriesKind::HeatmapGrid {
                        dataset
                            .heatmap_y_numeric()
                            .and_then(|values| values.get(row))
                            .copied()
                    } else {
                        dataset.y_is_valid(row).then(|| dataset.y()[row])
                    }
                }
                AxisDimension::Angle | AxisDimension::Radius => None,
            };
            value.is_some_and(|value| value >= *from && value <= *to)
        }
        (GeneralBrushRange::Temporal([from, to]), GeneralAxisDomain::Temporal(_)) => {
            if brush.dimension != AxisDimension::X {
                return false;
            }
            dataset
                .temporal_x_epoch_ms()
                .and_then(|values| values.get(row))
                .is_some_and(|value| value >= from && value <= to)
        }
        (GeneralBrushRange::Category([from, to]), GeneralAxisDomain::Category(categories)) => {
            let Some(from_index) = categories.iter().position(|value| value == from) else {
                return false;
            };
            let Some(to_index) = categories.iter().position(|value| value == to) else {
                return false;
            };
            let label = match brush.dimension {
                AxisDimension::X => {
                    if series.kind == GeneralSeriesKind::HorizontalBar {
                        None
                    } else {
                        general_x_label(dataset, row)
                    }
                }
                AxisDimension::Y => match series.kind {
                    GeneralSeriesKind::HorizontalBar => general_x_label(dataset, row),
                    GeneralSeriesKind::HeatmapGrid => general_heatmap_y_label(dataset, row),
                    _ => None,
                },
                AxisDimension::Angle | AxisDimension::Radius => None,
            };
            label
                .and_then(|label| categories.iter().position(|value| value == &label))
                .is_some_and(|index| {
                    let low = from_index.min(to_index);
                    let high = from_index.max(to_index);
                    index >= low && index <= high
                })
        }
        _ => false,
    }
}

fn general_heatmap_y_label(dataset: &crate::GeneralDataset, row: usize) -> Option<String> {
    if let Some(value) = dataset
        .heatmap_y_numeric()
        .and_then(|values| values.get(row))
    {
        return Some(value.to_string());
    }
    let category_index = usize::try_from(*dataset.heatmap_y_category_indices()?.get(row)?).ok()?;
    dataset.heatmap_y_categories()?.get(category_index).cloned()
}

fn validate_dataset_for_series(
    kind: GeneralSeriesKind,
    dataset: &crate::GeneralDataset,
    x_axis: &crate::GeneralAxis,
    y_axis: &crate::GeneralAxis,
) -> Result<(), ChartError> {
    match kind {
        GeneralSeriesKind::Column | GeneralSeriesKind::HorizontalBar => {
            if dataset.x_kind() != GeneralXKind::Category {
                return Err(invalid("bar series require category/value data"));
            }
        }
        GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea => {
            let expected_x = match x_axis.scale() {
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog => GeneralXKind::Numeric,
                GeneralScaleType::Temporal => GeneralXKind::Temporal,
                GeneralScaleType::Band | GeneralScaleType::Point => GeneralXKind::Category,
                GeneralScaleType::RadialLinear | GeneralScaleType::AngularCategory => {
                    return Err(invalid("general path X axis scale is incompatible"));
                }
            };
            if dataset.x_kind() != expected_x {
                return Err(invalid(
                    "general path dataset X kind must match its bound X axis",
                ));
            }
        }
        GeneralSeriesKind::RangeArea | GeneralSeriesKind::RangeBar => {
            let expected_x = match x_axis.scale() {
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog => GeneralXKind::Numeric,
                GeneralScaleType::Temporal => GeneralXKind::Temporal,
                GeneralScaleType::Band | GeneralScaleType::Point => GeneralXKind::Category,
                GeneralScaleType::RadialLinear | GeneralScaleType::AngularCategory => {
                    return Err(invalid("range-area X axis scale is incompatible"));
                }
            };
            if dataset.x_kind() != expected_x {
                return Err(invalid(
                    "range-area dataset X kind must match its bound X axis",
                ));
            }
            dataset
                .low()
                .ok_or_else(|| invalid("range-area series require a low-value channel"))?;
        }
        GeneralSeriesKind::ErrorBar => {
            match dataset.x_kind() {
                GeneralXKind::Numeric => {
                    dataset
                        .x_low()
                        .ok_or_else(|| invalid("numeric error bars require an X-low channel"))?;
                    dataset
                        .x_high()
                        .ok_or_else(|| invalid("numeric error bars require an X-high channel"))?;
                }
                GeneralXKind::Category => {
                    if dataset.x_low().is_some() || dataset.x_high().is_some() {
                        return Err(invalid(
                            "category error bars cannot have numeric X-bound channels",
                        ));
                    }
                }
                GeneralXKind::Temporal => {
                    dataset
                        .x_low()
                        .ok_or_else(|| invalid("temporal error bars require an X-low channel"))?;
                    dataset
                        .x_high()
                        .ok_or_else(|| invalid("temporal error bars require an X-high channel"))?;
                }
            }
            dataset
                .low()
                .ok_or_else(|| invalid("error-bar series require a Y-low channel"))?;
            dataset
                .high()
                .ok_or_else(|| invalid("error-bar series require a Y-high channel"))?;
        }
        GeneralSeriesKind::BoxPlot => {
            if dataset.x_kind() != GeneralXKind::Category {
                return Err(invalid("box-plot series require category X data"));
            }
            let min = dataset
                .low()
                .ok_or_else(|| invalid("box-plot series require a min channel"))?;
            let max = dataset
                .high()
                .ok_or_else(|| invalid("box-plot series require a max channel"))?;
            let q1 = dataset
                .x_low()
                .ok_or_else(|| invalid("box-plot series require a q1 channel"))?;
            let q3 = dataset
                .x_high()
                .ok_or_else(|| invalid("box-plot series require a q3 channel"))?;
            if y_axis.scale() == GeneralScaleType::Logarithmic {
                for row in 0..dataset.len() {
                    for (valid, value) in [
                        (dataset.low_is_valid(row), min[row]),
                        (dataset.x_low_is_valid(row), q1[row]),
                        (dataset.y_is_valid(row), dataset.y()[row]),
                        (dataset.x_high_is_valid(row), q3[row]),
                        (dataset.high_is_valid(row), max[row]),
                    ] {
                        if valid && value <= 0.0 {
                            return Err(invalid(
                                "logarithmic box-plot statistics must be positive",
                            ));
                        }
                    }
                }
            }
        }
        GeneralSeriesKind::HeatmapGrid => match dataset.x_kind() {
            GeneralXKind::Category => {
                let y_categories = dataset
                    .heatmap_y_categories()
                    .ok_or_else(|| invalid("category heatmaps require Y-category labels"))?;
                let y_indices = dataset
                    .heatmap_y_category_indices()
                    .ok_or_else(|| invalid("category heatmaps require Y-category indices"))?;
                if y_indices.len() != dataset.len()
                    || (y_categories.is_empty() && !dataset.is_empty())
                {
                    return Err(invalid(
                        "heatmap-grid Y-category columns must stay aligned with the dataset",
                    ));
                }
            }
            GeneralXKind::Numeric | GeneralXKind::Temporal => {
                let y_coordinates = dataset.heatmap_y_numeric().ok_or_else(|| {
                    invalid("numeric/temporal heatmaps require numeric Y coordinates")
                })?;
                if y_coordinates.len() != dataset.len() {
                    return Err(invalid(
                        "heatmap-grid numeric Y coordinates must stay aligned with the dataset",
                    ));
                }
                if y_axis.scale() == GeneralScaleType::Logarithmic
                    && y_coordinates.iter().any(|value| *value <= 0.0)
                {
                    return Err(invalid(
                        "logarithmic heatmap Y coordinates must be positive",
                    ));
                }
                if dataset.x_kind() == GeneralXKind::Numeric
                    && x_axis.scale() == GeneralScaleType::Logarithmic
                    && dataset
                        .numeric_x()
                        .is_some_and(|values| values.iter().any(|value| *value <= 0.0))
                {
                    return Err(invalid(
                        "logarithmic heatmap X coordinates must be positive",
                    ));
                }
            }
        },
        GeneralSeriesKind::Scatter => {
            let values = dataset
                .numeric_x()
                .ok_or_else(|| invalid("scatter series require numeric X data"))?;
            if x_axis.scale() == GeneralScaleType::Logarithmic
                && values.iter().any(|value| *value <= 0.0)
            {
                return Err(invalid("logarithmic scatter X values must be positive"));
            }
            validate_logarithmic_general_y(dataset, y_axis)?;
        }
        GeneralSeriesKind::Bubble => {
            dataset
                .numeric_x()
                .ok_or_else(|| invalid("bubble series require numeric X data"))?;
            dataset
                .size()
                .ok_or_else(|| invalid("bubble series require a size channel"))?;
        }
    }
    Ok(())
}

fn validate_logarithmic_general_y(
    dataset: &crate::GeneralDataset,
    y_axis: &crate::GeneralAxis,
) -> Result<(), ChartError> {
    if y_axis.scale() == GeneralScaleType::Logarithmic
        && dataset
            .y()
            .iter()
            .enumerate()
            .any(|(index, value)| dataset.y_is_valid(index) && *value <= 0.0)
    {
        return Err(invalid(
            "logarithmic general-series Y values must be positive",
        ));
    }
    Ok(())
}

fn validate_general_reference_options(
    engine: &ChartEngine,
    pane_id: PaneId,
    options: &GeneralReferenceOptions,
) -> Result<(), ChartError> {
    let validate_axis = |id: &str,
                         dimension: AxisDimension|
     -> Result<&crate::GeneralAxis, ChartError> {
        let axis = engine
            .general_axis(id)
            .ok_or_else(|| invalid(format!("general reference axis {id:?} is stale")))?;
        if axis.pane_id() != pane_id || axis.dimension() != dimension {
            return Err(invalid(format!(
                "general reference axis {id:?} must belong to its pane and have {dimension:?} dimension"
            )));
        }
        Ok(axis)
    };
    let validate_color = |color: &Option<String>| -> Result<(), ChartError> {
        if let Some(color) = color {
            if color.len() > MAX_GENERAL_SERIES_COLOR_BYTES {
                return Err(resource(format!(
                    "general reference color exceeds {MAX_GENERAL_SERIES_COLOR_BYTES} bytes"
                )));
            }
            if Color::parse_css(color).is_none() {
                return Err(invalid(
                    "general reference color is not a supported CSS color",
                ));
            }
        }
        Ok(())
    };

    match options {
        GeneralReferenceOptions::Line {
            axis_id,
            value,
            color,
            line_width,
            ..
        } => {
            let axis = engine
                .general_axis(axis_id)
                .ok_or_else(|| invalid(format!("general reference axis {axis_id:?} is stale")))?;
            if axis.pane_id() != pane_id
                || !matches!(axis.dimension(), AxisDimension::X | AxisDimension::Y)
            {
                return Err(invalid(
                    "general reference line axis must be an X or Y axis on its pane",
                ));
            }
            validate_general_reference_value(value, axis)?;
            validate_color(color)?;
            if !line_width.is_finite() || *line_width <= 0.0 || *line_width > 64.0 {
                return Err(invalid(
                    "general reference line_width must be finite and in (0, 64]",
                ));
            }
        }
        GeneralReferenceOptions::Dot {
            x_axis_id,
            y_axis_id,
            x,
            y,
            color,
            radius,
            ..
        } => {
            let x_axis = validate_axis(x_axis_id, AxisDimension::X)?;
            let y_axis = validate_axis(y_axis_id, AxisDimension::Y)?;
            validate_general_reference_value(x, x_axis)?;
            validate_general_reference_value(y, y_axis)?;
            validate_color(color)?;
            if !radius.is_finite()
                || *radius < MIN_GENERAL_POINT_RADIUS
                || *radius > MAX_GENERAL_POINT_RADIUS
            {
                return Err(invalid(format!(
                    "general reference radius must be finite and in [{MIN_GENERAL_POINT_RADIUS}, {MAX_GENERAL_POINT_RADIUS}]"
                )));
            }
        }
        GeneralReferenceOptions::Region {
            x_axis_id,
            y_axis_id,
            x_from,
            x_to,
            y_from,
            y_to,
            fill_color,
            ..
        } => {
            let x_axis = validate_axis(x_axis_id, AxisDimension::X)?;
            let y_axis = validate_axis(y_axis_id, AxisDimension::Y)?;
            for value in [x_from, x_to] {
                validate_general_reference_value(value, x_axis)?;
            }
            for value in [y_from, y_to] {
                validate_general_reference_value(value, y_axis)?;
            }
            validate_color(fill_color)?;
        }
    }
    Ok(())
}

fn validate_general_reference_value(
    value: &GeneralReferenceValue,
    axis: &crate::GeneralAxis,
) -> Result<(), ChartError> {
    match (value, axis.scale()) {
        (
            GeneralReferenceValue::Numeric(value),
            GeneralScaleType::Linear
            | GeneralScaleType::Logarithmic
            | GeneralScaleType::SymmetricLog,
        ) => {
            if !value.is_finite() {
                return Err(invalid("general reference numeric values must be finite"));
            }
            if axis.scale() == GeneralScaleType::Logarithmic && *value <= 0.0 {
                return Err(invalid(
                    "general reference values on logarithmic axes must be positive",
                ));
            }
            Ok(())
        }
        (GeneralReferenceValue::Temporal(value), GeneralScaleType::Temporal) => {
            if value.unsigned_abs() > crate::MAX_GENERAL_TEMPORAL_MILLISECONDS as u64 {
                return Err(invalid(format!(
                    "general reference temporal values must stay within +/-{} epoch milliseconds",
                    crate::MAX_GENERAL_TEMPORAL_MILLISECONDS
                )));
            }
            Ok(())
        }
        (
            GeneralReferenceValue::Category(value),
            GeneralScaleType::Band | GeneralScaleType::Point,
        ) => {
            if value.is_empty() {
                return Err(invalid(
                    "general reference category values must not be empty",
                ));
            }
            if value.len() > crate::MAX_GENERAL_AXIS_CATEGORY_BYTES {
                return Err(resource(format!(
                    "general reference category exceeds {} bytes",
                    crate::MAX_GENERAL_AXIS_CATEGORY_BYTES
                )));
            }
            Ok(())
        }
        _ => Err(invalid(
            "general reference value type must match the bound axis scale",
        )),
    }
}

fn validate_input_for_series(
    kind: GeneralSeriesKind,
    input: &GeneralXyInput,
    x_axis: &crate::GeneralAxis,
    y_axis: &crate::GeneralAxis,
) -> Result<(), ChartError> {
    match kind {
        GeneralSeriesKind::Column | GeneralSeriesKind::HorizontalBar => {
            if input.x_kind() != GeneralXKind::Category {
                return Err(invalid(
                    "a dataset bound to a bar series must remain category/value data",
                ));
            }
        }
        GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea => {
            let expected_x = match x_axis.scale() {
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog => GeneralXKind::Numeric,
                GeneralScaleType::Temporal => GeneralXKind::Temporal,
                GeneralScaleType::Band | GeneralScaleType::Point => GeneralXKind::Category,
                GeneralScaleType::RadialLinear | GeneralScaleType::AngularCategory => {
                    return Err(invalid("general path X axis scale is incompatible"));
                }
            };
            if input.x_kind() != expected_x {
                return Err(invalid(
                    "a dataset bound to a general path series must keep the X kind required by its X axis",
                ));
            }
        }
        GeneralSeriesKind::RangeArea | GeneralSeriesKind::RangeBar => {
            let expected_x = match x_axis.scale() {
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog => GeneralXKind::Numeric,
                GeneralScaleType::Temporal => GeneralXKind::Temporal,
                GeneralScaleType::Band | GeneralScaleType::Point => GeneralXKind::Category,
                GeneralScaleType::RadialLinear | GeneralScaleType::AngularCategory => {
                    return Err(invalid("range-area X axis scale is incompatible"));
                }
            };
            if input.x_kind() != expected_x {
                return Err(invalid(
                    "a dataset bound to a range-area series must keep the X kind required by its X axis",
                ));
            }
            input.low_values().ok_or_else(|| {
                invalid("a dataset bound to a range-area series must retain its low-value channel")
            })?;
        }
        GeneralSeriesKind::ErrorBar => {
            let expected_x = match x_axis.scale() {
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog => GeneralXKind::Numeric,
                GeneralScaleType::Temporal => GeneralXKind::Temporal,
                GeneralScaleType::Band | GeneralScaleType::Point => GeneralXKind::Category,
                GeneralScaleType::RadialLinear | GeneralScaleType::AngularCategory => {
                    return Err(invalid("error-bar X axis scale is incompatible"))
                }
            };
            if input.x_kind() != expected_x {
                return Err(invalid("a dataset bound to an error-bar series must keep the X kind required by its X axis"));
            }
            input.low_values().ok_or_else(|| {
                invalid("a dataset bound to an error-bar series must retain its Y-low channel")
            })?;
            input.high_values().ok_or_else(|| {
                invalid("a dataset bound to an error-bar series must retain its Y-high channel")
            })?;
            if matches!(expected_x, GeneralXKind::Numeric | GeneralXKind::Temporal) {
                input.x_low_values().ok_or_else(|| {
                    invalid("numeric/temporal error bars must retain an X-low channel")
                })?;
                input.x_high_values().ok_or_else(|| {
                    invalid("numeric/temporal error bars must retain an X-high channel")
                })?;
            } else if input.x_low_values().is_some() || input.x_high_values().is_some() {
                return Err(invalid(
                    "category error bars cannot have numeric X-bound channels",
                ));
            }
        }
        GeneralSeriesKind::BoxPlot => {
            let GeneralXyInput::BoxCategory {
                min,
                min_valid,
                q1,
                q1_valid,
                median,
                median_valid,
                q3,
                q3_valid,
                max,
                max_valid,
                ..
            } = input
            else {
                return Err(invalid(
                    "a dataset bound to a box-plot series must remain category box-plot data",
                ));
            };
            if y_axis.scale() == GeneralScaleType::Logarithmic {
                for row in 0..median.len() {
                    for (validity, value) in [
                        (min_valid.as_deref(), min[row]),
                        (q1_valid.as_deref(), q1[row]),
                        (median_valid.as_deref(), median[row]),
                        (q3_valid.as_deref(), q3[row]),
                        (max_valid.as_deref(), max[row]),
                    ] {
                        if validity.is_none_or(|values| values[row] != 0) && value <= 0.0 {
                            return Err(invalid(
                                "logarithmic box-plot statistics must be positive",
                            ));
                        }
                    }
                }
            }
        }
        GeneralSeriesKind::HeatmapGrid => match (input, x_axis.scale(), y_axis.scale()) {
            (
                GeneralXyInput::HeatmapCategoryCategory {
                    y_categories,
                    y_category_indices,
                    value,
                    ..
                },
                GeneralScaleType::Band,
                GeneralScaleType::Band,
            ) => {
                if y_category_indices.len() != value.len()
                    || (y_categories.is_empty() && !value.is_empty())
                {
                    return Err(invalid_data(
                        "heatmap-grid Y-category columns must stay aligned with the value channel",
                    ));
                }
            }
            (
                GeneralXyInput::HeatmapNumericNumeric {
                    x,
                    y_coordinate,
                    value,
                    ..
                },
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog,
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog,
            ) => {
                if x.len() != value.len() || y_coordinate.len() != value.len() {
                    return Err(invalid_data(
                        "numeric heatmap coordinates must stay aligned with the value channel",
                    ));
                }
                if x_axis.scale() == GeneralScaleType::Logarithmic
                    && x.iter().any(|value| *value <= 0.0)
                {
                    return Err(invalid(
                        "logarithmic heatmap X coordinates must be positive",
                    ));
                }
                if y_axis.scale() == GeneralScaleType::Logarithmic
                    && y_coordinate.iter().any(|value| *value <= 0.0)
                {
                    return Err(invalid(
                        "logarithmic heatmap Y coordinates must be positive",
                    ));
                }
            }
            (
                GeneralXyInput::HeatmapTemporalNumeric {
                    x_epoch_ms,
                    y_coordinate,
                    value,
                    ..
                },
                GeneralScaleType::Temporal,
                GeneralScaleType::Linear
                | GeneralScaleType::Logarithmic
                | GeneralScaleType::SymmetricLog,
            ) => {
                if x_epoch_ms.len() != value.len() || y_coordinate.len() != value.len() {
                    return Err(invalid_data(
                        "temporal heatmap coordinates must stay aligned with the value channel",
                    ));
                }
                if y_axis.scale() == GeneralScaleType::Logarithmic
                    && y_coordinate.iter().any(|value| *value <= 0.0)
                {
                    return Err(invalid(
                        "logarithmic heatmap Y coordinates must be positive",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                        "a dataset bound to a heatmap-grid series must keep the coordinate shape required by its axes",
                    ));
            }
        },
        GeneralSeriesKind::Scatter => {
            let values = input.numeric_x_values().ok_or_else(|| {
                invalid("a dataset bound to a scatter series must remain numeric X data")
            })?;
            if x_axis.scale() == GeneralScaleType::Logarithmic
                && values.iter().any(|value| *value <= 0.0)
            {
                return Err(invalid("logarithmic scatter X values must be positive"));
            }
            validate_logarithmic_input_y(input, y_axis)?;
        }
        GeneralSeriesKind::Bubble => {
            input.numeric_x_values().ok_or_else(|| {
                invalid("a dataset bound to a bubble series must remain numeric X data")
            })?;
            input.size_values().ok_or_else(|| {
                invalid("a dataset bound to a bubble series must retain its size channel")
            })?;
        }
    }
    Ok(())
}

fn validate_logarithmic_input_y(
    input: &GeneralXyInput,
    y_axis: &crate::GeneralAxis,
) -> Result<(), ChartError> {
    if y_axis.scale() == GeneralScaleType::Logarithmic {
        let validity = input.y_valid_values();
        if input.y_values().iter().enumerate().any(|(index, value)| {
            validity
                .and_then(|values| values.get(index))
                .is_none_or(|valid| *valid != 0)
                && *value <= 0.0
        }) {
            return Err(invalid(
                "logarithmic general-series Y values must be positive",
            ));
        }
    }
    Ok(())
}

fn validate_presentation(options: &GeneralSeriesOptions) -> Result<(), ChartError> {
    if !options.fill_opacity.is_finite() || !(0.0..=1.0).contains(&options.fill_opacity) {
        return Err(invalid(
            "general series fill_opacity must be finite and in [0, 1]",
        ));
    }
    if options.fill_opacity != DEFAULT_GENERAL_FILL_OPACITY
        && !matches!(
            options.kind,
            GeneralSeriesKind::XyArea | GeneralSeriesKind::RangeArea
        )
    {
        return Err(invalid(
            "general series fill_opacity is supported only by xy_area and range_area",
        ));
    }
    if options.point_symbol != GeneralPointSymbol::Circle
        && !matches!(
            options.kind,
            GeneralSeriesKind::XyLine
                | GeneralSeriesKind::XyArea
                | GeneralSeriesKind::RangeArea
                | GeneralSeriesKind::Scatter
        )
    {
        return Err(invalid(
            "general series point_symbol is supported only by xy_line, xy_area, range_area, and scatter",
        ));
    }
    if options.connect_missing
        && !matches!(
            options.kind,
            GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea | GeneralSeriesKind::RangeArea
        )
    {
        return Err(invalid(
            "general series connect_missing is supported only by xy_line, xy_area, and range_area",
        ));
    }
    if options.interpolation != GeneralInterpolation::Linear
        && !matches!(
            options.kind,
            GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea | GeneralSeriesKind::RangeArea
        )
    {
        return Err(invalid(
            "general series interpolation is supported only by xy_line, xy_area, and range_area",
        ));
    }
    if options.point_markers
        && !matches!(
            options.kind,
            GeneralSeriesKind::XyLine | GeneralSeriesKind::XyArea | GeneralSeriesKind::RangeArea
        )
    {
        return Err(invalid(
            "general series point_markers is supported only by xy_line, xy_area, and range_area",
        ));
    }
    if !options.line_width.is_finite() || !(0.5..=32.0).contains(&options.line_width) {
        return Err(invalid(
            "general series line_width must be finite and in [0.5, 32]",
        ));
    }
    if let Some(value) = options.baseline_value {
        if !value.is_finite() {
            return Err(invalid("general series baseline_value must be finite"));
        }
        if options.kind != GeneralSeriesKind::XyArea {
            return Err(invalid(
                "general series baseline_value is supported only by xy_area",
            ));
        }
    }
    if options.title.len() > MAX_GENERAL_SERIES_TITLE_BYTES {
        return Err(resource(format!(
            "general series title exceeds {MAX_GENERAL_SERIES_TITLE_BYTES} UTF-8 bytes"
        )));
    }
    if let Some(color) = options.color.as_deref() {
        if color.len() > MAX_GENERAL_SERIES_COLOR_BYTES {
            return Err(resource(format!(
                "general series color exceeds {MAX_GENERAL_SERIES_COLOR_BYTES} UTF-8 bytes"
            )));
        }
        if Color::parse_css(color).is_none() {
            return Err(invalid("general series color must be a valid CSS color"));
        }
    }
    if options
        .group_id
        .as_ref()
        .is_some_and(|value| value.len() > MAX_GENERAL_SERIES_GROUP_ID_BYTES)
    {
        return Err(resource(format!(
            "general series group ID exceeds {MAX_GENERAL_SERIES_GROUP_ID_BYTES} UTF-8 bytes"
        )));
    }
    if options
        .stack_id
        .as_ref()
        .is_some_and(|value| value.len() > MAX_GENERAL_SERIES_STACK_ID_BYTES)
    {
        return Err(resource(format!(
            "general series stack ID exceeds {MAX_GENERAL_SERIES_STACK_ID_BYTES} UTF-8 bytes"
        )));
    }
    if !matches!(
        options.kind,
        GeneralSeriesKind::Column | GeneralSeriesKind::HorizontalBar
    ) && options.group_id.is_some()
    {
        return Err(invalid(
            "grouping is supported only by column and horizontal_bar series",
        ));
    }
    if !matches!(
        options.kind,
        GeneralSeriesKind::Column | GeneralSeriesKind::HorizontalBar | GeneralSeriesKind::XyArea
    ) && (options.stack_id.is_some() || options.stack_mode != GeneralStackMode::Normal)
    {
        return Err(invalid(
            "stacking is supported only by column, horizontal_bar, and xy_area series",
        ));
    }
    if options.stack_id.is_none() && options.stack_mode != GeneralStackMode::Normal {
        return Err(invalid("percent stack mode requires a stack ID"));
    }
    if (options.kind == GeneralSeriesKind::Scatter || options.point_markers)
        && (!options.point_radius.is_finite()
            || !(MIN_GENERAL_POINT_RADIUS..=MAX_GENERAL_POINT_RADIUS)
                .contains(&options.point_radius))
    {
        return Err(invalid(format!(
            "general series point radius must be finite and in {MIN_GENERAL_POINT_RADIUS}..={MAX_GENERAL_POINT_RADIUS} CSS px"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidOptions, message)
}

fn invalid_data(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::InvalidData, message)
}

fn resource(message: impl Into<String>) -> ChartError {
    ChartError::new(ErrorCode::ResourceLimit, message)
}
