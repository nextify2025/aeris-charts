//! Backend-neutral frame production for the headless chart model.
//!
//! This is intentionally independent of WebGPU, Canvas2D, and DOM types. Hosts may convert the
//! returned primitives into any raster backend, or inspect them in tests.

use crate::drawings::DrawingKind;
use crate::SeriesThresholdRegion;
use crate::{
    ChartEngine, PriceFormatKind, PriceScaleSide, PriceScaleTarget, SeriesKind, SeriesPriceFormat,
    PANE_SEPARATOR,
};
use aeris_charts_core::format::percentage_formatter::PercentageFormatter;
use aeris_charts_core::format::price_formatter::PriceFormatter;
use aeris_charts_core::format::time_formatter::{
    format_crosshair_time_in, format_date_pattern, format_tick_label_in, weight_to_tick_mark_type,
    TickMarkType,
};
use aeris_charts_core::format::volume_formatter::VolumeFormatter;
use aeris_charts_core::model::data_layer::{PointColorChannel, SeriesId};
use aeris_charts_core::model::magnet::{magnet_snap_coordinate, CrosshairMode};
use aeris_charts_core::model::plot_list::{
    MinMax, MismatchDirection, PlotListView, PlotValueIndex,
};
use aeris_charts_core::model::price_range::PriceRange;
use aeris_charts_core::scale::price_scale_core::{PriceScaleCore, PriceScaleMode};
use aeris_charts_core::style::{
    AREA_FILL_FAINT_ALPHA, AREA_FILL_STRONG_ALPHA, DEFAULT_BORDER_RGB, DEFAULT_CROSSHAIR_LABEL_RGB,
    DEFAULT_CROSSHAIR_LINE_RGB, DEFAULT_PRIMARY_RGB, MARKET_DOWN_RGB, MARKET_UP_RGB,
    MARKET_VOLUME_ALPHA,
};
use aeris_charts_render::bars::{build_bars, BarItem, BarsParams};
use aeris_charts_render::candles::{build_candles, CandleItem, CandlesParams};
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::{
    Gradient, IRect, LineStyle, LineType, Prim, RasterImage, TextAlign,
};
use aeris_charts_render::histogram::{build_histogram, HistogramItem, HistogramParams};

const THRESHOLD_REGION_LINE_COLOR: Color = Color::rgb(0x78, 0x7B, 0x86);
const THRESHOLD_REGION_FILL_COLOR: Color = Color::rgba(0x78, 0x7B, 0x86, 0x33);

pub(crate) mod alert_geometry;
mod axis;
pub(crate) mod conflation;
mod crosshair;
mod depth_geometry;
mod drawings;
mod feature_geometry;
mod footprint_geometry;
mod general_series_geometry;
mod native_primitive_geometry;
#[cfg(test)]
mod run_break_tests;
mod series_geometry;
#[cfg(test)]
mod tests;
mod trading_geometry;

#[cfg(test)]
use conflation::{visible_histogram_rows, visible_ohlc};
use conflation::{
    visible_histogram_rows_with_work, visible_line_rows, visible_line_rows_with_work,
    visible_ohlc_with_values, visible_ohlc_with_work,
};

const UP: Color = Color::rgb(MARKET_UP_RGB.0, MARKET_UP_RGB.1, MARKET_UP_RGB.2);
const DOWN: Color = Color::rgb(MARKET_DOWN_RGB.0, MARKET_DOWN_RGB.1, MARKET_DOWN_RGB.2);
const PRIMARY: Color = Color::rgb(
    DEFAULT_PRIMARY_RGB.0,
    DEFAULT_PRIMARY_RGB.1,
    DEFAULT_PRIMARY_RGB.2,
);
/// Neutral position-entry chrome. Position risk/reward colors stay semantic red/green instead of
/// inheriting the drawing template's primary/accent color.
const POSITION_ENTRY: Color = Color::rgb(0x78, 0x7b, 0x86);
const GRID: Color = Color::rgb(
    DEFAULT_BORDER_RGB.0,
    DEFAULT_BORDER_RGB.1,
    DEFAULT_BORDER_RGB.2,
);
const LINE: Color = Color::rgb(0x21, 0x96, 0xf3);
const AREA_LINE: Color = UP;
/// The canonical area-fill gradient for a stroke color: `AREA_FILL_STRONG_ALPHA` at the series
/// extreme fading to `AREA_FILL_FAINT_ALPHA` at its base, scaled by the stroke's own alpha. Area,
/// both baseline halves, and brush ranges all derive their default fill here, so every area-like
/// surface has the same strength and follows its own line color.
pub(crate) fn area_fill_gradient(stroke: Color) -> (Color, Color) {
    let with = |alpha: u8| {
        let scaled = (u16::from(alpha) * u16::from(stroke.a()) + 127) / 255;
        Color::rgba(stroke.r(), stroke.g(), stroke.b(), scaled as u8)
    };
    (with(AREA_FILL_STRONG_ALPHA), with(AREA_FILL_FAINT_ALPHA))
}
const HISTOGRAM: Color = Color::rgba(
    MARKET_UP_RGB.0,
    MARKET_UP_RGB.1,
    MARKET_UP_RGB.2,
    MARKET_VOLUME_ALPHA,
);
const VOLUME_UP: Color = Color::rgba(
    MARKET_UP_RGB.0,
    MARKET_UP_RGB.1,
    MARKET_UP_RGB.2,
    MARKET_VOLUME_ALPHA,
);
const VOLUME_DOWN: Color = Color::rgba(
    MARKET_DOWN_RGB.0,
    MARKET_DOWN_RGB.1,
    MARKET_DOWN_RGB.2,
    MARKET_VOLUME_ALPHA,
);
const BASELINE_TOP_LINE: Color = UP;
const BASELINE_BOTTOM_LINE: Color = DOWN;
/// Aeris default stroke for line, area, and baseline series (CSS px), matching indicator lines.
pub(crate) const LINE_WIDTH: f64 = 2.0;
const CROSSHAIR_COLOR: Color = Color::rgb(
    DEFAULT_CROSSHAIR_LINE_RGB.0,
    DEFAULT_CROSSHAIR_LINE_RGB.1,
    DEFAULT_CROSSHAIR_LINE_RGB.2,
);
const CROSSHAIR_LABEL_BG: Color = Color::rgb(
    DEFAULT_CROSSHAIR_LABEL_RGB.0,
    DEFAULT_CROSSHAIR_LABEL_RGB.1,
    DEFAULT_CROSSHAIR_LABEL_RGB.2,
);

/// A bordered chrome box (tooltip, chip) in device px with every edge on a whole device pixel.
/// `RoundRect` borders paint inside the rect, so an edge at a fractional coordinate spreads a
/// 1 px border across two pixel rows or columns: the same border reads crisp on one side and
/// blurred on another, and the blur changes as the box moves. Snapping each edge independently
/// (rather than position and size separately) keeps all four borders one device pixel wide.
pub(crate) struct DeviceBox {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

impl DeviceBox {
    /// Snap a CSS-px box at `(left, top)` of `width × height` with the frame's ratios.
    pub(crate) fn snap(left: f64, top: f64, width: f64, height: f64, hpr: f64, vpr: f64) -> Self {
        let x0 = (left * hpr).round();
        let y0 = (top * vpr).round();
        let x1 = ((left + width) * hpr).round().max(x0 + 1.0);
        let y1 = ((top + height) * vpr).round().max(y0 + 1.0);
        Self {
            x: x0 as f32,
            y: y0 as f32,
            w: (x1 - x0) as f32,
            h: (y1 - y0) as f32,
        }
    }

    pub(crate) fn center_x(&self) -> f32 {
        self.x + self.w / 2.0
    }
}

fn ceiled_odd(value: f64) -> f64 {
    let ceiled = value.ceil() as i64;
    if ceiled % 2 == 0 {
        (ceiled - 1) as f64
    } else {
        ceiled as f64
    }
}

fn ceiled_even(value: f64) -> f64 {
    let ceiled = value.ceil() as i64;
    if ceiled % 2 != 0 {
        (ceiled - 1) as f64
    } else {
        ceiled as f64
    }
}

fn marker_envelope_size(bar_spacing: f64) -> f64 {
    ceiled_even(ceiled_odd(bar_spacing.clamp(12.0, 30.0)))
}

fn marker_shape_size(envelope: f64, coefficient: f64) -> f64 {
    ceiled_odd(envelope.max(12.0) * coefficient)
}

fn marker_margin(bar_spacing: f64) -> f64 {
    ceiled_odd(bar_spacing.clamp(12.0, 30.0) * 0.1).max(3.0)
}

fn marker_auto_scale_margins(markers: &[crate::Marker], bar_spacing: f64) -> (f64, f64) {
    if markers.is_empty() {
        return (0.0, 0.0);
    }
    let margin_value = marker_envelope_size(bar_spacing) * 1.5 + marker_margin(bar_spacing) * 2.0;
    let has_above = markers
        .iter()
        .any(|marker| marker.position == crate::marker_pos::ABOVE);
    let has_below = markers
        .iter()
        .any(|marker| marker.position == crate::marker_pos::BELOW);
    let has_in_bar = markers
        .iter()
        .any(|marker| marker.position == crate::marker_pos::IN_BAR);
    let adjusted = || (margin_value / 2.0).ceil();
    (
        if has_above {
            margin_value
        } else if has_in_bar {
            adjusted()
        } else {
            0.0
        },
        if has_below {
            margin_value
        } else if has_in_bar {
            adjusted()
        } else {
            0.0
        },
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FramePane {
    pub top: f64,
    pub height: f64,
    pub scissor: [u32; 4],
    pub under: Vec<Prim>,
    pub main: Vec<Prim>,
    /// Primitive z-order `top` layer (reference `PrimitivePaneViewZOrder` "top" — above everything,
    /// crosshair included). The engine emits nothing here today; hosts append plugin prims
    /// after frame construction. Kept beside `under`/`main` so both backends execute it with
    /// the pane's scissor and point pool. (`FramePane.top` is already the pane's CSS y offset,
    /// hence the `_prims` suffix.)
    pub top_prims: Vec<Prim>,
    /// Series paint-order marks (Phase C-c): `(series_id, main.len())` recorded right after
    /// each visible series' slot in the pane's paint loop. A custom series paints nothing
    /// here, so its mark equals the previous series'; hosts splice custom-series prims into
    /// `main` at the mark, preserving the chart z-order instead of always painting on top.
    pub series_paint_marks: Vec<(SeriesId, usize)>,
    pub points: Vec<[f32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartFrame {
    pub width: f64,
    pub height: f64,
    pub pixel_ratio: f64,
    pub panes: Vec<FramePane>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameBuildStats {
    pub layout_rebuilds: u64,
    pub autoscale_runs: u64,
    pub grid_rebuilds: u64,
    pub series_rebuilds: u64,
    pub drawing_rebuilds: u64,
    pub overlay_rebuilds: u64,
    pub trading_rebuilds: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePaneSegments {
    pub under_end: usize,
    pub series_end: usize,
    pub trading_regions_end: usize,
    pub drawings_end: usize,
    pub trading_end: usize,
    pub overlay_end: usize,
    pub under_revision: u64,
    pub drawings_revision: u64,
    pub trading_revision: u64,
    pub overlay_revision: u64,
    pub top_revision: u64,
    /// Canonical coordinate revision shared by every coordinate-dependent segment in this pane.
    pub coordinate_revision: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSeriesSegment {
    pub series_id: Option<SeriesId>,
    pub start: usize,
    pub end: usize,
    pub revision: u64,
    pub coordinate_revision: u64,
}

/// One drawing's `out.main` range for the current frame (ordering.rs assembly order:
/// idle drawings below price series, active drawings above ordinary series with the active
/// series, previews trailing as `None`). Backends retain per-drawing groups like series
/// segments; ordering changes swap group order (key mismatch re-uploads moved drawings)
/// without rebuilding retained geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameDrawingSegment {
    pub drawing_id: Option<crate::drawings::DrawingId>,
    pub start: usize,
    pub end: usize,
    pub revision: u64,
    pub coordinate_revision: u64,
}

#[derive(Default)]
pub(crate) struct FrameInvalidation {
    clock: u64,
    layout: u64,
    coordinate: u64,
    scene: u64,
    drawings: u64,
    trading: u64,
    overlay: u64,
    axis: u64,
    autoscale: u64,
    chrome: u64,
    series: Vec<(SeriesId, u64)>,
}

impl FrameInvalidation {
    /// Advances on every invalidation of any layer.
    pub(crate) const fn clock(&self) -> u64 {
        self.clock
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1).max(1);
        self.clock
    }

    fn all(&mut self) {
        let generation = self.tick();
        self.layout = generation;
        self.coordinate = generation;
        self.scene = generation;
        self.chrome = generation;
        self.drawings = generation;
        self.trading = generation;
        self.overlay = generation;
        self.axis = generation;
        self.autoscale = generation;
    }

    fn scene(&mut self) {
        let generation = self.tick();
        self.scene = generation;
        self.chrome = generation;
        self.drawings = generation;
        self.trading = generation;
        self.overlay = generation;
        self.axis = generation;
        self.autoscale = generation;
    }

    fn coordinates(&mut self) {
        let generation = self.tick();
        self.coordinate = generation;
        self.scene = generation;
        self.chrome = generation;
        self.drawings = generation;
        self.trading = generation;
        self.overlay = generation;
        self.axis = generation;
    }

    fn time_coordinates(&mut self) {
        self.coordinates();
        self.autoscale = self.clock;
    }

    fn series(&mut self, id: SeriesId) {
        let generation = self.series_geometry(id);
        self.chrome = generation;
        self.overlay = generation;
        self.autoscale = generation;
        self.axis = generation;
    }

    /// Only this series' own geometry layer; autoscale, axes, chrome, and overlay stay retained.
    fn series_geometry(&mut self, id: SeriesId) -> u64 {
        let generation = self.tick();
        match self.series.iter_mut().find(|entry| entry.0 == id) {
            Some(entry) => entry.1 = generation,
            None => self.series.push((id, generation)),
        }
        generation
    }

    fn series_generation(&self, id: SeriesId) -> u64 {
        self.series
            .iter()
            .find_map(|entry| (entry.0 == id).then_some(entry.1))
            .unwrap_or(0)
    }

    fn drawings(&mut self) {
        let generation = self.tick();
        self.drawings = generation;
        self.chrome = generation;
        self.overlay = generation;
        self.axis = generation;
    }

    fn trading(&mut self) {
        let generation = self.tick();
        self.trading = generation;
        self.axis = generation;
    }

    fn overlay(&mut self) {
        let generation = self.tick();
        self.overlay = generation;
        self.axis = generation;
    }

    fn axis(&mut self) {
        self.axis = self.tick();
    }

    fn layout_and_axis(&mut self) {
        let generation = self.tick();
        self.layout = generation;
        self.axis = generation;
    }
}

#[derive(Clone, Default)]
struct RetainedLayer {
    prims: Vec<Prim>,
    points: Vec<[f32; 2]>,
    revision: u64,
    coordinate_revision: u64,
}

#[derive(Clone, Default)]
struct RetainedPane {
    top: f64,
    height: f64,
    scissor: [u32; 4],
    under: RetainedLayer,
    /// Cursor-driven primitives with `bottom` z-order. Kept separate so pointer movement does not
    /// rebuild static grids or any series geometry.
    cursor_under: RetainedLayer,
    series_layers: Vec<RetainedSeriesLayer>,
    chrome: RetainedLayer,
    trading_regions: RetainedLayer,
    drawings: RetainedLayer,
    /// Per-drawing prim/point ranges inside `drawings` for committed drawings, in stable
    /// z-order as built (ordering.rs reassembly copies them idle-below / active-above without
    /// rebuilding geometry). Preview brush/pending trailing block starts at
    /// `drawing_preview_start` (prim, point) to the end of `drawings`.
    drawing_parts: Vec<RetainedDrawingPart>,
    drawing_preview_prim_start: usize,
    drawing_preview_point_start: usize,
    trading: RetainedLayer,
    overlay: RetainedLayer,
    top_layer: RetainedLayer,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct RetainedDrawingPart {
    pub(crate) id: crate::drawings::DrawingId,
    pub(crate) prim_start: usize,
    pub(crate) prim_end: usize,
    pub(crate) point_start: usize,
    pub(crate) point_end: usize,
}

#[derive(Clone, Default)]
struct RetainedSeriesLayer {
    id: SeriesId,
    scene_generation: u64,
    source_generation: u64,
    layer: RetainedLayer,
}

#[derive(Default)]
pub(crate) struct RetainedFrame {
    initialized: bool,
    panes: Vec<RetainedPane>,
    segments: Vec<FramePaneSegments>,
    series_segments: Vec<Vec<FrameSeriesSegment>>,
    drawing_segments: Vec<Vec<FrameDrawingSegment>>,
    layout_generation: u64,
    scene_generation: u64,
    chrome_generation: u64,
    drawings_generation: u64,
    trading_generation: u64,
    overlay_generation: u64,
    autoscale_generation: u64,
    axis_generation: u64,
    coordinate_generation: u64,
    last_layout_key: Option<[u64; 9]>,
    last_overlay_key: Option<[u64; 7]>,
    /// The hovered and the selected drawing when their family paints parts only while focused
    /// (`DrawingFamily::reveals_on_focus`).
    last_focus_key: [Option<crate::DrawingId>; 2],
    last_options_generation: u64,
    last_series_revision: u64,
    last_time_scale_revision: u64,
    last_price_scale_revisions: Vec<Vec<u64>>,
    /// Invalidation clock observed by the last prepared host frame.
    prepared_clock: u64,
}

impl RetainedFrame {
    pub(crate) fn capacity_bytes(&self) -> usize {
        fn layer_bytes(layer: &RetainedLayer) -> usize {
            layer.prims.capacity() * std::mem::size_of::<Prim>()
                + layer.points.capacity() * std::mem::size_of::<[f32; 2]>()
        }

        self.panes
            .iter()
            .map(|pane| {
                layer_bytes(&pane.under)
                    + layer_bytes(&pane.cursor_under)
                    + layer_bytes(&pane.chrome)
                    + layer_bytes(&pane.trading_regions)
                    + layer_bytes(&pane.drawings)
                    + layer_bytes(&pane.trading)
                    + layer_bytes(&pane.overlay)
                    + layer_bytes(&pane.top_layer)
                    + pane.series_layers.capacity() * std::mem::size_of::<RetainedSeriesLayer>()
                    + pane
                        .series_layers
                        .iter()
                        .map(|series| layer_bytes(&series.layer))
                        .sum::<usize>()
                    + pane.drawing_parts.capacity() * std::mem::size_of::<RetainedDrawingPart>()
            })
            .sum::<usize>()
            + self.panes.capacity() * std::mem::size_of::<RetainedPane>()
            + self.segments.capacity() * std::mem::size_of::<FramePaneSegments>()
            + self.series_segments.capacity() * std::mem::size_of::<Vec<FrameSeriesSegment>>()
            + self
                .series_segments
                .iter()
                .map(|segments| segments.capacity() * std::mem::size_of::<FrameSeriesSegment>())
                .sum::<usize>()
            + self.drawing_segments.capacity() * std::mem::size_of::<Vec<FrameDrawingSegment>>()
            + self
                .drawing_segments
                .iter()
                .map(|segments| segments.capacity() * std::mem::size_of::<FrameDrawingSegment>())
                .sum::<usize>()
            + self.last_price_scale_revisions.capacity() * std::mem::size_of::<Vec<u64>>()
            + self
                .last_price_scale_revisions
                .iter()
                .map(|revisions| revisions.capacity() * std::mem::size_of::<u64>())
                .sum::<usize>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisTextAlign {
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisTextMidpoint {
    /// Canvas `middle` baseline without an actual-glyph correction (time ticks and markers).
    None,
    /// Correct using this label's glyph bounds (price-axis labels).
    Label,
    /// Correct using the reference's stable representative time-label sample (crosshair time label).
    StableTime,
}

/// Per-corner rounding selection for a boxed axis label's background (painted with a 2 CSS px
/// radius by the host). A boxed label rounds only its axis-facing side — the corners pointing
/// away from the pane — and keeps the chart-facing side sharp: right-strip labels round their
/// right corners, left-strip labels their left corners, time-strip labels their bottom corners.
/// The last-value cluster selects per row instead: the axis-facing corners of the cluster's
/// top and bottom edges only, with sharp internal boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AxisLabelCorners {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_left: bool,
    pub bottom_right: bool,
}

impl AxisLabelCorners {
    pub const NONE: Self = Self {
        top_left: false,
        top_right: false,
        bottom_left: false,
        bottom_right: false,
    };
    /// Right side (right price strip: the axis-facing side is the label's right edge).
    pub const RIGHT: Self = Self {
        top_left: false,
        top_right: true,
        bottom_left: false,
        bottom_right: true,
    };
    /// Left side (left price strip).
    pub const LEFT: Self = Self {
        top_left: true,
        top_right: false,
        bottom_left: true,
        bottom_right: false,
    };
    /// Bottom side (time strip below the pane).
    pub const BOTTOM: Self = Self {
        top_left: false,
        top_right: false,
        bottom_left: true,
        bottom_right: true,
    };
    /// Every corner (small floating icon boxes such as the alert create glyph).
    pub const ALL: Self = Self {
        top_left: true,
        top_right: true,
        bottom_left: true,
        bottom_right: true,
    };

    /// The whole-side selection for a single-row boxed label from its text alignment: `Left`
    /// (text starting at the right strip's left edge) rounds the right corners, `Right` the
    /// left corners, and `Center` (time-axis boxes below the pane) the bottom corners.
    pub fn for_align(align: AxisTextAlign) -> Self {
        match align {
            AxisTextAlign::Left => Self::RIGHT,
            AxisTextAlign::Right => Self::LEFT,
            AxisTextAlign::Center => Self::BOTTOM,
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::NONE
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AxisLabel {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub color: Color,
    pub align: AxisTextAlign,
    pub midpoint: AxisTextMidpoint,
    /// Scale relative to the chart layout font size. Axis labels use `1.0`; secondary rows such
    /// as the candle countdown may be smaller while retaining the same family and metrics.
    pub font_scale: f64,
    pub bold: bool,
    pub background: Option<(f64, f64, f64, f64, Color)>,
    /// Rounded-corner selection for `background` (see [`AxisLabelCorners`]); `NONE` paints the
    /// plain sharp rectangle.
    pub background_corners: AxisLabelCorners,
    /// Extra width (media px) this label contributes to the axis-width negotiation beyond its
    /// own text. The last-value cluster puts it on the price-area label so the negotiated
    /// strip covers the title chip + price row (each label is otherwise measured alone).
    pub measure_extra: f64,
    /// Attachment group: boxed labels sharing a group id are painted with SHARED edges — each
    /// box's top edge is the previous box's exact bottom (no per-box rounding gaps between
    /// attached rows like the price chip and its countdown chip).
    pub attach_group: Option<u32>,
    /// Optional inside border for an axis-label background, in media px.
    pub border: Option<(f64, Color)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AxisRotatedLabel {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub color: Color,
    pub align: AxisTextAlign,
    pub font_scale: f64,
    pub bold: bool,
    /// Clockwise radians around the aligned `(x, y)` anchor.
    pub angle: f64,
}

/// A backend-neutral rectangle painted beneath axis chrome and labels. Rectangle drawings use
/// this for the official plugin's 15 CSS px price/time-axis pane shading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisBand {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: Color,
}

/// One shared SVG image in chart-space CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisIcon {
    pub x: f64,
    pub y: f64,
    pub side: f64,
    pub image: RasterImage,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisFrame {
    pub bands: Vec<AxisBand>,
    pub labels: Vec<AxisLabel>,
    pub rotated_labels: Vec<AxisRotatedLabel>,
    /// Original SVG pixels, retained at the current DPR/font size.
    pub crosshair_action_icon: Option<AxisIcon>,
    pub separators: Vec<f64>,
    /// Price-axis tick stubs (reference `ticksVisible`): 5 css px horizontal marks painted from the
    /// pane edge into the axis strip at each tick coordinate, in the strip's border color.
    /// Emitted only for scales whose `ticksVisible` option is on.
    pub price_ticks: Vec<PriceAxisTick>,
    /// Time-axis tick x positions (media px, relative to the chart offset) for the
    /// `ticksVisible` stubs; empty while `timeScale.ticksVisible` is off.
    pub time_ticks: Vec<f64>,
    /// Index (into `separators`) of the hovered pane separator, if any — the host paints the
    /// `layout.panes.separatorHoverColor` band over it (reference pane-separator.ts).
    pub separator_hover: Option<usize>,
}

/// One price-axis tick stub (reference price-axis-widget.ts `_drawTickMarks`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceAxisTick {
    /// Media-y of the tick mark (same coordinate as its label).
    pub y: f64,
    /// Media-x of the tick's left edge inside its exact price-scale strip.
    pub x: f64,
    /// Which strip the tick belongs to (`true` = left axis, `false` = right).
    pub left: bool,
}

#[derive(Clone, Copy)]
struct ResolvedSeries {
    id: SeriesId,
    kind: SeriesKind,
    color: Color,
    up: Color,
    down: Color,
    wick_up: Color,
    wick_down: Color,
    border_up: Color,
    border_down: Color,
    wick_visible: bool,
    border_visible: bool,
    line_width: f64,
    line_style: LineStyle,
    line_visible: bool,
    area_top: Color,
    area_bottom: Color,
    threshold_region: Option<SeriesThresholdRegion>,
    invert_filled_area: bool,
    point_markers: bool,
    point_markers_radius: Option<f64>,
    visible: bool,
    line_type: LineType,
    open_visible: bool,
    close_visible: bool,
    thin_bars: bool,
    heikin_ashi: bool,
    base: f64,
    top_fill1: Color,
    top_fill2: Color,
    top_line: Color,
    top_line_width: f64,
    top_line_style: LineStyle,
    bottom_fill1: Color,
    bottom_fill2: Color,
    bottom_line: Color,
    bottom_line_width: f64,
    bottom_line_style: LineStyle,
    scale_target: PriceScaleTarget,
    /// The pane this series renders on; `None` when its pane was removed (reference `removePane`
    /// orphans the pane's series) — it draws and scales nowhere until re-assigned.
    pane: Option<usize>,
    base_value: f64,
}

/// A series' own autoscale union in raw prices, and whether its data had a visible range (marker
/// margins and native primitives only participate alongside visible data).
struct SeriesAutoscaleRange {
    range: Option<PriceRange>,
    has_data: bool,
}

pub(crate) fn series_scale_target(series: &crate::SeriesEntry) -> PriceScaleTarget {
    series.price_scale_target
}

pub(crate) fn pane_scale(pane: &crate::Pane, target: PriceScaleTarget) -> &PriceScaleCore {
    pane.scale(target)
        .expect("live series and primitives reference a live pane price scale")
}

fn css_color(value: &str, fallback: Color) -> Color {
    Color::parse_css(value).unwrap_or(fallback)
}

/// Resolve a verbatim CSS color slot at render time (the wave-1 pattern): the stored string
/// parses, an unset slot or an unparseable string falls back to `fallback` (the reference's default —
/// a user string the renderer cannot parse degrades to the default rather than vanishing).
/// The stroke color a line-like series actually renders: its explicit color, or for an Area with
/// none, the Area default hue. Fills, brush defaults, and `options().color` all resolve through it.
pub(crate) fn series_stroke_color(series: &crate::SeriesEntry) -> Color {
    let color = verbatim_color(&series.line_color, crate::DEFAULT_LINE_COLOR);
    if series.kind == SeriesKind::Area && series.line_color.is_none() {
        AREA_LINE
    } else {
        color
    }
}

/// Default brushable-area styles for one series, all on the canonical fill strength: selected
/// ranges in the market up/down hues, and everything outside the selection as the series' own
/// stroke faded to 20% with its fill derived by the same rule.
pub(crate) fn area_brush_defaults(series: &crate::SeriesEntry) -> crate::AreaBrushDefaults {
    let line_width = series.line_width.unwrap_or(LINE_WIDTH);
    let style = |line_color: Color| {
        let (top_color, bottom_color) = area_fill_gradient(line_color);
        crate::BrushStyle {
            line_color,
            top_color,
            bottom_color,
            line_width,
        }
    };
    let stroke = series_stroke_color(series);
    let faded_alpha = ((u16::from(stroke.a()) * 51 + 127) / 255) as u8;
    crate::AreaBrushDefaults {
        outside: style(Color::rgba(stroke.r(), stroke.g(), stroke.b(), faded_alpha)),
        positive: style(UP),
        negative: style(DOWN),
    }
}

pub(crate) fn verbatim_color(value: &Option<String>, fallback: Color) -> Color {
    value
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or(fallback)
}

impl ChartEngine {
    pub(crate) fn themed_candle_colors(&self) -> (Color, Color) {
        let layout = &self.options.get().layout;
        (
            Color::parse_css(&layout.bullish_color).unwrap_or(UP),
            Color::parse_css(&layout.bearish_color).unwrap_or(DOWN),
        )
    }
}

impl ChartEngine {
    /// The Baseline series' effective baseline price: the pinned `baseline_value` option, or
    /// the visible-range close midpoint (the engine's auto mode when the option is unset).
    /// Shared by the baseline geometry builder and the bar-color resolution so both agree on
    /// which side of the baseline a bar sits.
    pub(crate) fn resolved_baseline_price(&self, id: SeriesId, from: i64, to: i64) -> Option<f64> {
        let series = self.series_entry(id)?;
        if let Some(price) = series.baseline {
            return Some(price);
        }
        let plot = self.data.plot(id);
        let close = |row: usize| plot.value_at(row, PlotValueIndex::Close);
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut any = false;
        for row in plot.visible_rows(from, to) {
            let value = close(row);
            if value.is_finite() {
                min = min.min(value);
                max = max.max(value);
                any = true;
            }
        }
        any.then_some((min + max) / 2.0)
    }

    /// reference `SeriesBarColorer.barColor` (model/series-bar-colorer.ts) for the series' bar at
    /// `row`: the color the built-in last-price line, the last-value axis label, and the
    /// crosshair marker background all follow when their own color option is unset.
    /// `baseline_price` is the resolved baseline for Baseline series (`None` for other kinds).
    pub(crate) fn series_bar_color(
        &self,
        series: &crate::SeriesEntry,
        row: usize,
        baseline_price: Option<f64>,
    ) -> Color {
        let plot = self.data.plot(series.id);
        // A hollow candle has no body to take a color from, so chrome that stands for "this bar"
        // resolves through what is actually painted instead of going invisible.
        if series.kind == SeriesKind::Candlestick {
            return self.candlestick_chrome_color(series, row);
        }
        // reference data-item colors: a per-point `color` (area reads `lineColor`, mapped onto the
        // body channel here) wins over the series-level resolution for every kind that reads
        // it (bar/candlestick/line/area/histogram); Baseline's barColor ignores data-item
        // colors (series-bar-colorer.ts Baseline arm).
        if !matches!(series.kind, SeriesKind::Baseline) {
            if let Some(c) = self
                .data
                .point_colors(series.id)
                .and_then(|colors| colors.color(PointColorChannel::Body, row))
            {
                return Color(c);
            }
        }
        match series.kind {
            // A line_color still holding the default placeholder resolves to the kind default,
            // exactly like the geometry builders.
            SeriesKind::Line => verbatim_color(&series.line_color, crate::DEFAULT_LINE_COLOR),
            SeriesKind::Area => series_stroke_color(series),
            SeriesKind::Histogram => {
                let color = verbatim_color(&series.line_color, crate::DEFAULT_LINE_COLOR);
                if color != LINE {
                    color
                } else {
                    HISTOGRAM
                }
            }
            // reference baseline colorer: top line color at/above the baseline, bottom below it.
            SeriesKind::Baseline => {
                let close = plot.value_at(row, PlotValueIndex::Close);
                match baseline_price {
                    Some(base) if close < base => series
                        .bottom_line_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(BASELINE_BOTTOM_LINE),
                    _ => series
                        .top_line_color
                        .as_deref()
                        .and_then(Color::parse_css)
                        .unwrap_or(BASELINE_TOP_LINE),
                }
            }
            // reference bar/candlestick colorer: up when open <= close.
            SeriesKind::Candlestick | SeriesKind::Bar => {
                let (up, down) = self.themed_candle_colors();
                let open = plot.value_at(row, PlotValueIndex::Open);
                let close = plot.value_at(row, PlotValueIndex::Close);
                if open <= close {
                    verbatim_color(&series.up_color, up)
                } else {
                    verbatim_color(&series.down_color, down)
                }
            }
            // reference custom-series colorer (series-bar-colorer.ts Custom arm): the series `color`
            // option (the data-item color wins in reference; the host folds those into the custom
            // frame values, so this arm is only the exhaustiveness fallback).
            SeriesKind::Feature => self
                .feature_bar_color(series.id, row)
                .unwrap_or_else(|| verbatim_color(&series.line_color, crate::DEFAULT_LINE_COLOR)),
            SeriesKind::Footprint => series
                .footprint
                .as_ref()
                .and_then(|state| {
                    self.trade_stream(state.trade_stream_id)
                        .and_then(|stream| stream.bars().get(row).map(|bar| (state, bar)))
                })
                .map_or(UP, |(state, bar)| {
                    if bar.delta >= 0.0 {
                        state.visual.positive_delta_color
                    } else {
                        state.visual.negative_delta_color
                    }
                }),
            SeriesKind::Custom => verbatim_color(&series.line_color, crate::DEFAULT_LINE_COLOR),
        }
    }

    /// The visible color of one candlestick, for chrome that represents the bar itself — the
    /// live price line, its last-value axis chip, and the crosshair marker.
    ///
    /// Normally that is the body color. A hollow candle (industry-standard: a transparent body,
    /// leaving the border frame and wick) has no body color to show, and following it anyway
    /// would paint the chip with a fully transparent fill — it would read as the bare chart
    /// surface rather than as the bar's bullish/bearish color. So a transparent body falls
    /// through to the parts that are actually painted, in the order they are drawn over:
    /// border, then wick. Each part follows the body color until pinned (reference parity), so
    /// an unpinned part inherits the same transparency and is skipped in turn; if nothing is
    /// visible at all, the body color stands.
    fn candlestick_chrome_color(&self, series: &crate::SeriesEntry, row: usize) -> Color {
        let (up, down) = self.themed_candle_colors();
        let plot = self.data.plot(series.id);
        let (open, close) = self
            .heikin_ashi_row(series.id, row)
            .map(|values| (values[0], values[3]))
            .unwrap_or_else(|| {
                (
                    plot.value_at(row, PlotValueIndex::Open),
                    plot.value_at(row, PlotValueIndex::Close),
                )
            });
        let rising = open <= close;
        let colors = self.data.point_colors(series.id);
        let point = |channel| {
            colors
                .and_then(|colors| colors.color(channel, row))
                .map(Color)
        };
        let pick = |up: &Option<String>, down: &Option<String>, fallback: Color| {
            if rising {
                verbatim_color(up, fallback)
            } else {
                verbatim_color(down, fallback)
            }
        };
        let body = point(PointColorChannel::Body).unwrap_or_else(|| {
            pick(
                &series.up_color,
                &series.down_color,
                if rising { up } else { down },
            )
        });
        if body.a() != 0 {
            return body;
        }
        if series.border_visible.unwrap_or(true) {
            let border = point(PointColorChannel::Border)
                .unwrap_or_else(|| pick(&series.border_up_color, &series.border_down_color, body));
            if border.a() != 0 {
                return border;
            }
        }
        if series.wick_visible.unwrap_or(true) {
            let wick = point(PointColorChannel::Wick)
                .unwrap_or_else(|| pick(&series.wick_up_color, &series.wick_down_color, body));
            if wick.a() != 0 {
                return wick;
            }
        }
        body
    }

    /// One effective color for a series' built-in live line and complete last-value cluster.
    /// A valid explicit `price_line_color` wins; otherwise callers supply the resolved color for
    /// the relevant row or custom-series frame value, preserving `price_line_source` semantics.
    pub(crate) fn effective_series_live_color(
        &self,
        series: &crate::SeriesEntry,
        resolved: Color,
    ) -> Color {
        series
            .price_line_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or(resolved)
    }
}

fn translate_prims_x(prims: &mut [Prim], dx: i32) {
    let dxf = dx as f32;
    for prim in prims {
        match prim {
            Prim::Rect { rect, .. } | Prim::RectFrame { rect, .. } => rect.x += dx,
            Prim::HLine { x0, x1, .. } => {
                *x0 += dx;
                *x1 += dx;
            }
            Prim::VLine { x, .. } => *x += dx,
            Prim::RoundRect { x, .. } => *x += dxf,
            Prim::Circle { cx, .. } => *cx += dxf,
            Prim::Triangle { a, b, c, .. } => {
                a[0] += dxf;
                b[0] += dxf;
                c[0] += dxf;
            }
            Prim::Text { x, .. } | Prim::RotatedText { x, .. } => *x += dxf,
            Prim::Image { rect, .. } => rect[0] += dxf,
            Prim::Polyline { .. }
            | Prim::Segments { .. }
            | Prim::AreaFill { .. }
            | Prim::BandFill { .. }
            | Prim::Background { .. } => {}
        }
    }
}

/// Shift every point-pool index of `prim` by `offset` (wrapping, so a negative shift is passed as
/// its two's complement). Retained layers and drawing parts keep pool-relative indices; assembling
/// them into a frame pane moves them to their place in the pane's pool. The match is exhaustive so
/// a future prim that references the pool cannot be forgotten here and read the wrong points.
fn shift_point_indices(prim: &mut Prim, offset: u32) {
    match prim {
        Prim::Polyline { first_point, .. }
        | Prim::Segments { first_point, .. }
        | Prim::AreaFill { first_point, .. } => {
            *first_point = first_point.wrapping_add(offset);
        }
        Prim::BandFill {
            upper_first,
            lower_first,
            ..
        } => {
            *upper_first = upper_first.wrapping_add(offset);
            *lower_first = lower_first.wrapping_add(offset);
        }
        Prim::Rect { .. }
        | Prim::RectFrame { .. }
        | Prim::HLine { .. }
        | Prim::VLine { .. }
        | Prim::RoundRect { .. }
        | Prim::Circle { .. }
        | Prim::Triangle { .. }
        | Prim::Background { .. }
        | Prim::Text { .. }
        | Prim::RotatedText { .. }
        | Prim::Image { .. } => {}
    }
}

/// Copy one retained drawing part (committed drawing or preview trailing block) into the
/// frame, remapping its point indices from retained space to the frame's point pool.
/// Reuses retained geometry; ordering changes reassemble without rebuilding.
#[allow(clippy::too_many_arguments)]
fn append_drawing_part(
    retained_prims: &[Prim],
    retained_points: &[[f32; 2]],
    prim_start: usize,
    prim_end: usize,
    point_start: usize,
    point_end: usize,
    prims: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
) {
    let prim_start = prim_start.min(retained_prims.len());
    let prim_end = prim_end.min(retained_prims.len()).max(prim_start);
    let point_start = point_start.min(retained_points.len());
    let point_end = point_end.min(retained_points.len()).max(point_start);
    if prim_start == prim_end {
        return;
    }
    let out_base = points.len() as u32;
    // Retained point indices in this part run `point_start..point_end`; they land at
    // `out_base..out_base + (point_end - point_start)`.
    let adjust = out_base.wrapping_sub(point_start as u32);
    points.extend_from_slice(&retained_points[point_start..point_end]);
    prims.reserve(prim_end - prim_start);
    for prim in &retained_prims[prim_start..prim_end] {
        let mut prim = prim.clone();
        shift_point_indices(&mut prim, adjust);
        prims.push(prim);
    }
}

fn append_retained_layer(layer: &RetainedLayer, prims: &mut Vec<Prim>, points: &mut Vec<[f32; 2]>) {
    let point_base = points.len() as u32;
    points.extend_from_slice(&layer.points);
    prims.reserve(layer.prims.len());
    for prim in &layer.prims {
        let mut prim = prim.clone();
        shift_point_indices(&mut prim, point_base);
        prims.push(prim);
    }
}

impl ChartEngine {
    pub(crate) fn invalidate_frame_all(&mut self) {
        self.frame_invalidation.all();
    }

    pub(crate) fn invalidate_frame_scene(&mut self) {
        self.frame_invalidation.scene();
    }

    pub(crate) fn invalidate_frame_series(&mut self, id: SeriesId) {
        self.frame_invalidation.series(id);
        // Family drawings whose geometry reads series data (regression statistics, a forecast's
        // outcome) follow their own source series; a change of any other series leaves them.
        // Structural source changes (series add, removal, pane or scale moves) invalidate the
        // whole scene instead.
        if self.drawings.iter().any(|drawing| {
            drawing
                .kind
                .spec()
                .family
                .is_some_and(|family| (family.reads_series_data)(drawing))
                && self.drawing_source_series(drawing) == Some(id)
        }) {
            self.frame_invalidation.drawings();
        }
    }

    /// The generation a retained series layer is built from. A `histogram_updown` histogram
    /// takes its column colors from the primary price series, so a price-only update (or a
    /// correction of an earlier close) rebuilds it too. Generations are ticks of one clock, so
    /// the newer of the two changes whenever either input changes.
    fn series_layer_source_generation(&self, rs: &ResolvedSeries) -> u64 {
        let own = self.frame_invalidation.series_generation(rs.id);
        if rs.kind != SeriesKind::Histogram
            || !self
                .series_entry(rs.id)
                .is_some_and(|series| series.histogram_updown)
        {
            return own;
        }
        self.primary_series().map_or(own, |primary| {
            own.max(self.frame_invalidation.series_generation(primary.id))
        })
    }

    /// A presentation change that only restyles this series' own primitives.
    pub(crate) fn invalidate_frame_series_geometry(&mut self, id: SeriesId) {
        self.frame_invalidation.series_geometry(id);
    }

    pub(crate) fn invalidate_frame_drawings(&mut self) {
        self.frame_invalidation.drawings();
    }

    pub(crate) fn invalidate_frame_trading(&mut self) {
        self.frame_invalidation.trading();
    }

    pub(crate) fn invalidate_frame_overlay(&mut self) {
        self.frame_invalidation.overlay();
    }

    /// Paint order changed while every retained layer stays valid: the next prepared frame must
    /// reassemble, but no geometry is rebuilt.
    pub(crate) fn invalidate_frame_assembly(&mut self) {
        self.frame_invalidation.tick();
    }

    pub(crate) fn invalidate_frame_axis(&mut self) {
        self.frame_invalidation.axis();
    }

    pub(crate) fn invalidate_frame_layout_and_axis(&mut self) {
        self.frame_invalidation.layout_and_axis();
    }

    pub fn frame_build_stats(&self) -> FrameBuildStats {
        self.frame_build_stats
    }

    pub fn frame_pane_segments(&self, pane: usize) -> Option<FramePaneSegments> {
        self.retained_frame.segments.get(pane).copied()
    }

    pub fn frame_series_segments(&self, pane: usize) -> &[FrameSeriesSegment] {
        self.retained_frame
            .series_segments
            .get(pane)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn frame_drawing_segments(&self, pane: usize) -> &[FrameDrawingSegment] {
        self.retained_frame
            .drawing_segments
            .get(pane)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn frame_coordinate_revision(&self) -> u64 {
        self.retained_frame.coordinate_generation
    }

    pub fn frame_requires_layout(&self) -> bool {
        self.retained_frame.layout_generation != self.frame_invalidation.layout
            || self.retained_frame.autoscale_generation != self.frame_invalidation.autoscale
            || self.retained_frame.last_series_revision != self.series.revision()
    }

    pub(crate) fn frame_layout_prepared(&mut self) {
        self.retained_frame.layout_generation = self.frame_invalidation.layout;
    }

    pub fn frame_requires_axis(&self) -> bool {
        self.retained_frame.axis_generation != self.frame_invalidation.axis
    }

    /// Whether any layer was invalidated since the last prepared host frame.
    pub(crate) fn frame_invalidated_since_prepare(&self) -> bool {
        self.retained_frame.prepared_clock != self.frame_invalidation.clock()
    }

    pub(crate) fn frame_prepared(&mut self) {
        self.retained_frame.prepared_clock = self.frame_invalidation.clock();
    }

    /// Force the next axis build to start from engine-owned labels. Browser extensions use this
    /// after mutating their transient axis views so detached plugin labels cannot be retained.
    pub fn invalidate_axis_frame(&mut self) {
        self.invalidate_frame_axis();
    }

    pub fn set_crosshair_at(&mut self, x: f64, y: f64) {
        let next = Some((x, y));
        if self.crosshair != next {
            self.crosshair = next;
            self.invalidate_frame_overlay();
        }
    }

    pub fn clear_crosshair_at(&mut self) {
        if self.crosshair.take().is_some() {
            self.invalidate_frame_overlay();
        }
    }

    /// Recompute pane price ranges for the current visible time window.
    /// Hosts that need scale-dependent layout measurements may call this before building a frame;
    /// `build_frame` calls it as well so standalone backends remain correct.
    pub fn autoscale_visible(&mut self) {
        self.frame_build_stats.autoscale_runs += 1;
        let mut before = std::mem::take(&mut self.retained_frame.last_price_scale_revisions);
        before.clear();
        before.extend(self.panes.iter().map(crate::Pane::scale_revisions));
        if let Some((from, to)) = self.visible_range_for_frame() {
            self.autoscale_for_frame(from, to);
        }
        if self
            .panes
            .iter()
            .zip(&before)
            .any(|(pane, before)| pane.scale_revisions() != *before)
        {
            self.frame_invalidation.coordinates();
        }
        before.clear();
        before.extend(self.panes.iter().map(crate::Pane::scale_revisions));
        self.retained_frame.last_price_scale_revisions = before;
        self.retained_frame.autoscale_generation = self.frame_invalidation.autoscale;
    }

    /// Build the visible chart geometry as backend-neutral primitives.
    ///
    /// The frame owns no GPU buffers and performs no browser calls. It is suitable for WebGPU,
    /// Canvas2D, tiny-skia, screenshots, and golden tests alike.
    pub fn build_frame(&mut self) -> ChartFrame {
        let mut frame = ChartFrame::default();
        self.build_frame_into(&mut frame);
        frame
    }

    /// Rebuild a frame while retaining its pane, primitive, and point allocations. Hosts that
    /// repaint repeatedly should keep one `ChartFrame` and call this method instead of allocating
    /// a fresh tree for every cursor/animation frame.
    pub fn build_frame_into(&mut self, output: &mut ChartFrame) {
        self.frame_build_stats = FrameBuildStats::default();
        self.reset_lod_work();
        self.build_frame_into_accumulating(output);
    }

    /// Start one host-coordinated frame whose layout/axis preparation occurs before pane-frame
    /// construction. The browser uses this to keep diagnostics for the complete operation.
    pub fn begin_frame_build(&mut self) {
        self.frame_build_stats = FrameBuildStats::default();
        self.reset_lod_work();
    }

    fn sync_frame_input_invalidation(&mut self) {
        // Layout font owns price-tick density: keep every owned scale's internal tick sizing
        // on the resolved axis metrics before any tick build or revision comparison below.
        // The write is a no-op once synced; a real change advances scale revisions exactly
        // like any other scale mutation.
        self.sync_axis_tick_fonts();
        let layout_key = [
            self.css_width.to_bits(),
            self.css_height.to_bits(),
            self.dpr.to_bits(),
            self.pane_w.to_bits(),
            self.pane_h.to_bits(),
            self.pane_left.to_bits(),
            self.left_axis_w.to_bits(),
            self.axis_w.to_bits(),
            self.panes.len() as u64,
        ];
        let crosshair = self.crosshair.unwrap_or((f64::NAN, f64::NAN));
        let overlay_key = [
            crosshair.0.to_bits(),
            crosshair.1.to_bits(),
            self.crosshair_mode as u64,
            u64::from(self.crosshair_ohlc_magnet),
            self.animation_time.to_bits(),
            self.separator_hover.map_or(u64::MAX, |index| index as u64),
            // Reduced motion removes the last-price pulse from the overlay layer.
            u64::from(self.interaction_options().reduced_motion),
        ];
        let options_generation = self.options.generation();
        let series_revision = self.series.revision();
        if self.retained_frame.last_layout_key != Some(layout_key)
            || self.retained_frame.last_options_generation != options_generation
        {
            self.frame_invalidation.all();
        } else if self.retained_frame.last_series_revision != series_revision {
            self.frame_invalidation.scene();
        } else if self.retained_frame.last_overlay_key != Some(overlay_key) {
            self.frame_invalidation.overlay();
        }
        let time_scale_revision = self.time_scale.revision();
        if self.retained_frame.last_time_scale_revision != time_scale_revision {
            self.frame_invalidation.time_coordinates();
        }
        // Hover and selection normally reassemble retained drawing geometry without rebuilding
        // it; a drawing that paints parts only while focused rebuilds the layer when it gains
        // or loses focus.
        let focus_key = [self.hovered_drawing(), self.selected_drawing()]
            .map(|id| id.filter(|&id| self.drawing_reveals_on_focus(id)));
        if self.retained_frame.last_focus_key != focus_key {
            self.frame_invalidation.drawings();
            self.retained_frame.last_focus_key = focus_key;
        }
        let price_scales_changed = self.retained_frame.last_price_scale_revisions.len()
            != self.panes.len()
            || self
                .panes
                .iter()
                .zip(&self.retained_frame.last_price_scale_revisions)
                .any(|(pane, revisions)| pane.scale_revisions() != *revisions);
        if price_scales_changed {
            self.frame_invalidation.coordinates();
        }
        self.retained_frame.last_layout_key = Some(layout_key);
        self.retained_frame.last_overlay_key = Some(overlay_key);
        self.retained_frame.last_options_generation = options_generation;
        self.retained_frame.last_series_revision = series_revision;
        self.retained_frame.last_time_scale_revision = time_scale_revision;
        self.retained_frame.last_price_scale_revisions.clear();
        self.retained_frame
            .last_price_scale_revisions
            .extend(self.panes.iter().map(crate::Pane::scale_revisions));
    }

    /// Build pane geometry without resetting work already recorded by host layout preparation.
    pub fn build_frame_into_accumulating(&mut self, output: &mut ChartFrame) {
        self.sync_frame_input_invalidation();

        let layout_dirty = self.retained_frame.layout_generation != self.frame_invalidation.layout;
        let autoscale_dirty =
            self.retained_frame.autoscale_generation != self.frame_invalidation.autoscale;

        if layout_dirty {
            self.layout_for_frame();
            self.frame_build_stats.layout_rebuilds += 1;
        }
        self.refresh_volume_profile_indicators();
        let visible = self.visible_range_for_frame();
        if autoscale_dirty {
            self.autoscale_visible();
        }
        if self.drawing_baselines_need_frame_refresh {
            self.refresh_drawing_pixel_baselines();
            self.drawing_baselines_need_frame_refresh = false;
        }
        let scene_dirty = self.retained_frame.scene_generation != self.frame_invalidation.scene;
        let drawings_dirty =
            self.retained_frame.drawings_generation != self.frame_invalidation.drawings;
        let trading_dirty = !self.retained_frame.initialized
            || self.retained_frame.trading_generation != self.frame_invalidation.trading;
        let overlay_dirty =
            self.retained_frame.overlay_generation != self.frame_invalidation.overlay;
        let chrome_dirty = self.retained_frame.chrome_generation != self.frame_invalidation.chrome;

        // reference/fancy-canvas renders each pane with its actual bitmap/media ratio, which can differ
        // slightly from devicePixelRatio when a fractional-DPR pane dimension rounds. Using DPR
        // directly shifts bars and grid lines relative to the independently rounded pane bitmap.
        let nominal_dpr = self.dpr.max(0.01);
        let hpr = (self.pane_w * nominal_dpr).round().max(1.0) / self.pane_w.max(1.0);
        let vpr = (self.pane_h * nominal_dpr).round().max(1.0) / self.pane_h.max(1.0);
        let pane_count = self.panes.len().max(1);
        let pane_w_px = (self.pane_w * hpr).round().max(1.0) as u32;
        let pane_left_px = (self.pane_left * nominal_dpr).round().max(0.0) as u32;
        let mut resolved = Vec::with_capacity(self.series.len());
        // Centralized pane-local paint order (ordering.rs): grid/background → idle indicators
        // → idle drawings → ordinary price series → active objects (dragging/editing →
        // hovered → selected → idle). Indicator outputs move as one visual group with internal
        // ordering preserved; explicit `set_series_order` overrides default idle grouping while
        // idle drawings stay below price series. The stable `series_order` and hit-test
        // arbitration are untouched so promotion cannot oscillate hover. The bump is global
        // across panes (filtering per pane preserves each pane's relative order).
        let order = self.effective_series_order();
        let (theme_up, theme_down) = self.themed_candle_colors();
        for &id in &order {
            let Some(s) = self.series_entry(id) else {
                continue;
            };
            let base_value = visible
                .and_then(|(from, _)| self.series_base_value(s.id, from))
                .unwrap_or(0.0);
            let (fallback_up, fallback_down) =
                if matches!(s.kind, SeriesKind::Candlestick | SeriesKind::Bar) {
                    (theme_up, theme_down)
                } else {
                    (UP, DOWN)
                };
            let up = verbatim_color(&s.up_color, fallback_up);
            let down = verbatim_color(&s.down_color, fallback_down);
            let color = series_stroke_color(s);
            // Area-like fills default to the canonical gradient of their own stroke color.
            let (area_strong, area_faint) = area_fill_gradient(color);
            let css = |value: &Option<String>| value.as_deref().and_then(Color::parse_css);
            let top_line = css(&s.top_line_color).unwrap_or(BASELINE_TOP_LINE);
            let bottom_line = css(&s.bottom_line_color).unwrap_or(BASELINE_BOTTOM_LINE);
            let (top_strong, top_faint) = area_fill_gradient(top_line);
            let (bottom_strong, bottom_faint) = area_fill_gradient(bottom_line);
            resolved.push(ResolvedSeries {
                id: s.id,
                kind: s.kind,
                color,
                up,
                down,
                // reference parity: an unset wick/border color follows the body color of its direction.
                wick_up: verbatim_color(&s.wick_up_color, up),
                wick_down: verbatim_color(&s.wick_down_color, down),
                border_up: verbatim_color(&s.border_up_color, up),
                border_down: verbatim_color(&s.border_down_color, down),
                wick_visible: s.wick_visible.unwrap_or(true),
                border_visible: s.border_visible.unwrap_or(true),
                line_width: s.line_width.unwrap_or(LINE_WIDTH),
                line_style: crate::line_style_from_u8(s.line_style),
                line_visible: s.line_visible,
                area_top: verbatim_color(&s.area_top_color, area_strong),
                area_bottom: verbatim_color(&s.area_bottom_color, area_faint),
                threshold_region: s.threshold_region,
                invert_filled_area: s.invert_filled_area,
                point_markers: s.point_markers,
                point_markers_radius: s.point_markers_radius,
                visible: s.visible,
                line_type: s.line_type,
                open_visible: s.open_visible,
                close_visible: s.close_visible,
                thin_bars: s.thin_bars,
                heikin_ashi: s.heikin_ashi,
                base: s.base,
                // An unset quadrant line width follows the series' line width (the reference's single
                // baseline lineWidth). Unset quadrant fills take the canonical area gradient of their
                // own quadrant line: strong at the extreme, faint at the baseline. Each gradient runs
                // top-to-bottom, so the top half is (strong, faint) and the bottom half (faint, strong).
                top_fill1: css(&s.top_fill_color1).unwrap_or(top_strong),
                top_fill2: css(&s.top_fill_color2).unwrap_or(top_faint),
                top_line,
                top_line_width: s.top_line_width.or(s.line_width).unwrap_or(LINE_WIDTH),
                top_line_style: crate::line_style_from_u8(s.top_line_style),
                bottom_fill1: css(&s.bottom_fill_color1).unwrap_or(bottom_faint),
                bottom_fill2: css(&s.bottom_fill_color2).unwrap_or(bottom_strong),
                bottom_line,
                bottom_line_width: s.bottom_line_width.or(s.line_width).unwrap_or(LINE_WIDTH),
                bottom_line_style: crate::line_style_from_u8(s.bottom_line_style),
                scale_target: series_scale_target(s),
                pane: (s.pane_index < pane_count).then_some(s.pane_index),
                base_value,
            });
        }

        output.width = self.pane_left + self.pane_w;
        output.height = self.pane_h;
        output.pixel_ratio = self.dpr;
        output.panes.resize_with(pane_count, FramePane::default);
        output.panes.truncate(pane_count);
        let time_marks = if layout_dirty || scene_dirty {
            self.time_marks_for_frame()
        } else {
            Vec::new()
        };
        let mut retained = std::mem::take(&mut self.retained_frame);
        retained
            .panes
            .resize_with(pane_count, RetainedPane::default);
        retained.panes.truncate(pane_count);
        retained
            .segments
            .resize(pane_count, FramePaneSegments::default());
        retained.segments.truncate(pane_count);
        retained.series_segments.resize_with(pane_count, Vec::new);
        retained.series_segments.truncate(pane_count);
        retained.drawing_segments.resize_with(pane_count, Vec::new);
        retained.drawing_segments.truncate(pane_count);
        let initial_build = !retained.initialized;
        for (pi, pane) in self.panes.iter().enumerate() {
            let top_px = (pane.top * vpr).round().max(0.0) as u32;
            let height_px = (pane.height * vpr).round().max(0.0) as u32;
            let cache = &mut retained.panes[pi];
            cache.top = pane.top;
            cache.height = pane.height;
            cache.scissor = [pane_left_px, top_px, pane_w_px, height_px];

            if layout_dirty || scene_dirty {
                cache.under.prims.clear();
                cache.under.points.clear();
                if let Some(background) =
                    self.background_gradient_prim(pane_left_px, top_px, pane_w_px, height_px)
                {
                    cache.under.prims.push(background);
                }
                if let Some((from, to)) = visible {
                    let mut grid_targets: Vec<_> = pane
                        .scale_targets()
                        .filter(|target| {
                            *target != PriceScaleTarget::Overlay
                                && self.price_scale_visible_for(pi, *target)
                                && self.scale_formatter_source(pi, *target).is_some()
                        })
                        .collect();
                    grid_targets.sort_by_key(|target| {
                        let order = pane.scale_order(*target).unwrap_or(usize::MAX);
                        let side = match pane.scale_side(*target) {
                            Some(PriceScaleSide::Right) => 0,
                            Some(PriceScaleSide::Left) => 1,
                            None => 2,
                        };
                        (order, side)
                    });
                    if let Some(grid_target) = grid_targets.first().copied() {
                        self.build_grid_frame(
                            &mut cache.under.prims,
                            &time_marks,
                            from,
                            to,
                            pane_w_px as i32,
                            top_px as i32,
                            height_px as i32,
                            hpr,
                            vpr,
                            &self.scale_tick_marks(pi, grid_target, 0.0),
                        );
                    }
                    self.build_native_session_highlighting_frame(
                        pi,
                        from,
                        to,
                        hpr,
                        vpr,
                        &mut cache.under.prims,
                    );
                    self.build_depth_heatmap_frame(pi, hpr, vpr, &mut cache.under.prims);
                }
                self.append_general_grid_frame(pi, hpr, vpr, &mut cache.under.prims);
                self.build_native_image_watermark_frame(pi, hpr, vpr, &mut cache.under.prims);
                cache.under.revision = self.frame_invalidation.scene;
                cache.under.coordinate_revision = self.frame_invalidation.coordinate;
                self.frame_build_stats.grid_rebuilds += 1;
            }

            let series_layers_dirty = scene_dirty
                || resolved
                    .iter()
                    .filter(|rs| rs.pane == Some(pi) && rs.visible)
                    .any(|rs| {
                        let source_generation = self.series_layer_source_generation(rs);
                        cache
                            .series_layers
                            .iter()
                            .find(|layer| layer.id == rs.id)
                            .is_none_or(|layer| {
                                layer.scene_generation != self.frame_invalidation.scene
                                    || layer.source_generation != source_generation
                            })
                    });
            if series_layers_dirty || chrome_dirty {
                cache
                    .series_layers
                    .retain(|layer| resolved.iter().any(|rs| rs.id == layer.id));
                cache.chrome.prims.clear();
                cache.chrome.points.clear();
                cache.top_layer.prims.clear();
                cache.top_layer.points.clear();
                if let Some((from, to)) = visible {
                    for rs in &resolved {
                        if rs.pane != Some(pi) || !rs.visible {
                            continue;
                        }
                        let source_generation = self.series_layer_source_generation(rs);
                        let layer_index = match cache
                            .series_layers
                            .iter()
                            .position(|layer| layer.id == rs.id)
                        {
                            Some(index) => index,
                            None => {
                                cache.series_layers.push(RetainedSeriesLayer {
                                    id: rs.id,
                                    ..RetainedSeriesLayer::default()
                                });
                                cache.series_layers.len() - 1
                            }
                        };
                        let series_layer = &mut cache.series_layers[layer_index];
                        let rebuild_series = scene_dirty
                            || series_layer.scene_generation != self.frame_invalidation.scene
                            || series_layer.source_generation != source_generation;
                        if !rebuild_series {
                            continue;
                        }
                        series_layer.layer.prims.clear();
                        series_layer.layer.points.clear();
                        let scale = pane_scale(pane, rs.scale_target);
                        let to = self.series_render_end(rs.id, to);
                        self.build_native_series_background_primitives_frame(
                            *rs,
                            from,
                            to,
                            hpr,
                            vpr,
                            &mut series_layer.layer.prims,
                            &mut series_layer.layer.points,
                            scale,
                        );
                        match rs.kind {
                            SeriesKind::Candlestick => self.build_candles_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                                scale,
                            ),
                            SeriesKind::Bar => self.build_bars_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                                scale,
                            ),
                            SeriesKind::Histogram => self.build_histogram_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                                scale,
                            ),
                            SeriesKind::Line | SeriesKind::Area => {
                                if let Some(region) = rs.threshold_region {
                                    let y_upper =
                                        (scale.price_to_coordinate(region.upper, rs.base_value)
                                            * vpr) as f32;
                                    let y_lower =
                                        (scale.price_to_coordinate(region.lower, rs.base_value)
                                            * vpr) as f32;
                                    let top = y_upper.min(y_lower);
                                    let height = (y_lower - y_upper).abs();
                                    if height >= 1.0 {
                                        series_layer.layer.prims.push(Prim::Rect {
                                            rect: IRect {
                                                x: 0,
                                                y: top.round() as i32,
                                                w: pane_w_px as i32,
                                                h: height.round() as i32,
                                            },
                                            color: THRESHOLD_REGION_FILL_COLOR,
                                        });
                                    }
                                }
                                self.build_line_frame(
                                    *rs,
                                    from,
                                    to,
                                    hpr,
                                    vpr,
                                    pane.top,
                                    pane.top + pane.height,
                                    &mut series_layer.layer.prims,
                                    &mut series_layer.layer.points,
                                    scale,
                                )
                            }
                            SeriesKind::Baseline => self.build_baseline_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                                &mut series_layer.layer.points,
                                scale,
                            ),
                            SeriesKind::Feature => self.build_feature_series_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                pane.top,
                                pane.height,
                                &mut series_layer.layer.prims,
                                &mut series_layer.layer.points,
                                scale,
                            ),
                            SeriesKind::Footprint => self.build_footprint_series_frame(
                                *rs,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                                scale,
                            ),
                            SeriesKind::Custom => {}
                        }
                        self.build_native_series_primitives_frame(
                            *rs,
                            from,
                            to,
                            hpr,
                            vpr,
                            &mut series_layer.layer.prims,
                            &mut series_layer.layer.points,
                            scale,
                        );
                        if self
                            .series
                            .iter()
                            .find(|series| series.id == rs.id)
                            .is_some_and(|series| {
                                series.markers_z_order == crate::marker_z_order::NORMAL
                            })
                        {
                            self.build_series_markers_frame(
                                rs.id,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut series_layer.layer.prims,
                            );
                        }
                        series_layer.scene_generation = self.frame_invalidation.scene;
                        series_layer.source_generation = source_generation;
                        series_layer.layer.revision =
                            self.frame_invalidation.scene.max(source_generation);
                        series_layer.layer.coordinate_revision = self.frame_invalidation.coordinate;
                        self.frame_build_stats.series_rebuilds += 1;
                    }
                    for rs in &resolved {
                        if rs.pane != Some(pi) || !rs.visible {
                            continue;
                        }
                        let Some(series) = self.series.iter().find(|series| series.id == rs.id)
                        else {
                            continue;
                        };
                        match series.markers_z_order {
                            crate::marker_z_order::ABOVE_SERIES => self.build_series_markers_frame(
                                rs.id,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut cache.chrome.prims,
                            ),
                            crate::marker_z_order::TOP => self.build_series_markers_frame(
                                rs.id,
                                from,
                                to,
                                hpr,
                                vpr,
                                &mut cache.top_layer.prims,
                            ),
                            _ => {}
                        }
                    }
                    self.build_depth_event_frame(
                        pi,
                        from,
                        to,
                        hpr,
                        vpr,
                        &mut cache.top_layer.prims,
                    );
                    self.build_price_lines_frame(
                        pi,
                        &mut cache.chrome.prims,
                        pane_w_px as i32,
                        vpr,
                    );
                    for rs in &resolved {
                        if rs.pane != Some(pi) || !rs.visible {
                            continue;
                        }
                        let Some(region) = rs.threshold_region else {
                            continue;
                        };
                        let scale = pane_scale(pane, rs.scale_target);
                        for price in [region.upper, region.lower] {
                            let y = (scale.price_to_coordinate(price, rs.base_value) * vpr) as f32;
                            cache.chrome.prims.push(Prim::HLine {
                                y: y.round() as i32,
                                x0: 0,
                                x1: pane_w_px as i32,
                                width: vpr.floor().max(1.0) as i32,
                                style: LineStyle::Dotted,
                                color: THRESHOLD_REGION_LINE_COLOR,
                            });
                        }
                    }
                    self.build_last_value_line_frame(
                        pi,
                        from,
                        to,
                        &mut cache.chrome.prims,
                        pane_w_px as i32,
                        hpr,
                        vpr,
                    );
                    self.build_position_progress_frame(
                        pi,
                        &mut cache.chrome.prims,
                        &mut cache.chrome.points,
                        hpr,
                        vpr,
                    );
                    self.build_bid_ask_lines_frame(
                        pi,
                        from,
                        &mut cache.chrome.prims,
                        pane_w_px as i32,
                        hpr,
                        vpr,
                    );
                }
                self.build_native_anchored_text_frame(pi, hpr, vpr, &mut cache.chrome.prims);
                self.build_native_text_watermark_frame(pi, hpr, vpr, &mut cache.chrome.prims);
                // Keep position information readable over its dynamic run overlay and all other
                // pane chrome. Trading/interaction layers still paint above these labels.
                self.build_position_labels_frame(pi, &mut cache.chrome.prims, hpr, vpr);
                cache.chrome.revision = self.frame_invalidation.chrome;
                cache.chrome.coordinate_revision = self.frame_invalidation.coordinate;
                cache.top_layer.revision = self.frame_invalidation.chrome;
                cache.top_layer.coordinate_revision = self.frame_invalidation.coordinate;
            }

            if drawings_dirty {
                cache.drawings.prims.clear();
                cache.drawings.points.clear();
                cache.drawing_parts.clear();
                let mut preview_start = (0usize, 0usize);
                self.build_drawings_frame_segmented(
                    pi,
                    pane_w_px as i32,
                    hpr,
                    vpr,
                    &mut cache.drawings.prims,
                    &mut cache.drawings.points,
                    &mut cache.drawing_parts,
                    &mut preview_start,
                );
                cache.drawing_preview_prim_start = preview_start.0;
                cache.drawing_preview_point_start = preview_start.1;
                cache.drawings.revision = self.frame_invalidation.drawings;
                cache.drawings.coordinate_revision = self.frame_invalidation.coordinate;
                self.frame_build_stats.drawing_rebuilds += 1;
            }

            if trading_dirty {
                cache.trading_regions.prims.clear();
                cache.trading_regions.points.clear();
                cache.trading.prims.clear();
                cache.trading.points.clear();
                self.build_alert_lines_frame(
                    pi,
                    pane_w_px as i32,
                    hpr,
                    vpr,
                    &mut cache.trading.prims,
                );
                self.build_trading_frame(
                    pi,
                    hpr,
                    vpr,
                    &mut cache.trading_regions.prims,
                    &mut cache.trading.prims,
                    &mut cache.trading.points,
                );
                cache.trading_regions.revision = self.frame_invalidation.trading;
                cache.trading_regions.coordinate_revision = self.frame_invalidation.coordinate;
                cache.trading.revision = self.frame_invalidation.trading;
                cache.trading.coordinate_revision = self.frame_invalidation.coordinate;
                self.frame_build_stats.trading_rebuilds += 1;
            }

            if overlay_dirty {
                cache.cursor_under.prims.clear();
                cache.cursor_under.points.clear();
                self.build_native_crosshair_highlight_frame(
                    pi,
                    hpr,
                    vpr,
                    &mut cache.cursor_under.prims,
                );
                self.build_native_tooltip_crosshair_frame(
                    pi,
                    hpr,
                    vpr,
                    &mut cache.cursor_under.prims,
                );
                cache.cursor_under.revision = self.frame_invalidation.overlay;
                cache.cursor_under.coordinate_revision = self.frame_invalidation.coordinate;
                cache.overlay.prims.clear();
                cache.overlay.points.clear();
                // The last-price pulse is a top pane view (reference `topPaneViews`): it lives in
                // the overlay layer because the animation clock invalidates only the overlay, so
                // it advances every tick without rebuilding series or chrome.
                if pi == 0 {
                    self.build_last_pulse_frame(&mut cache.overlay.prims, hpr, vpr);
                }
                self.build_native_accessibility_focus_frame(pi, hpr, vpr, &mut cache.overlay.prims);
                self.build_hovered_text_frame(
                    pi,
                    pane_w_px as i32,
                    hpr,
                    vpr,
                    &mut cache.overlay.prims,
                );
                self.build_selected_drawing_handles_frame(
                    pi,
                    pane_w_px as i32,
                    hpr,
                    vpr,
                    &mut cache.overlay.prims,
                );
                self.build_crosshair_frame(
                    pi,
                    pane_w_px as i32,
                    hpr,
                    vpr,
                    &mut cache.overlay.prims,
                );
                self.build_native_delta_tooltip_frame(pi, hpr, vpr, &mut cache.overlay.prims);
                if let Some((from, _)) = visible {
                    self.build_selection_anchors_frame(
                        pi,
                        from,
                        hpr,
                        vpr,
                        &mut cache.overlay.prims,
                    );
                }
                cache.overlay.revision = self.frame_invalidation.overlay;
                cache.overlay.coordinate_revision = self.frame_invalidation.coordinate;
                self.frame_build_stats.overlay_rebuilds += 1;
            }

            let out = &mut output.panes[pi];
            debug_assert_eq!(
                cache.under.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.cursor_under.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.chrome.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.drawings.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.trading_regions.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.trading.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.overlay.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert_eq!(
                cache.top_layer.coordinate_revision,
                self.frame_invalidation.coordinate
            );
            debug_assert!(cache
                .series_layers
                .iter()
                .filter(|layer| resolved.iter().any(|series| {
                    series.id == layer.id && series.pane == Some(pi) && series.visible
                }))
                .all(|layer| {
                    layer.layer.coordinate_revision == self.frame_invalidation.coordinate
                }));
            out.top = cache.top;
            out.height = cache.height;
            out.scissor = cache.scissor;
            out.under.clear();
            out.main.clear();
            out.top_prims.clear();
            out.series_paint_marks.clear();
            out.points.clear();
            // The first retained build knows its exact assembled size. Reserve once so initial
            // historical installs pay one retained-to-contract copy, not repeated Vec growth.
            if initial_build {
                let mut main_prims = cache.chrome.prims.len()
                    + cache.trading_regions.prims.len()
                    + cache.drawings.prims.len()
                    + cache.trading.prims.len()
                    + cache.overlay.prims.len();
                let mut point_count = cache.under.points.len()
                    + cache.cursor_under.points.len()
                    + cache.chrome.points.len()
                    + cache.trading_regions.points.len()
                    + cache.drawings.points.len()
                    + cache.trading.points.len()
                    + cache.overlay.points.len();
                point_count += cache.top_layer.points.len();
                for rs in &resolved {
                    if rs.pane == Some(pi) && rs.visible {
                        if let Some(layer) =
                            cache.series_layers.iter().find(|layer| layer.id == rs.id)
                        {
                            main_prims += layer.layer.prims.len();
                            point_count += layer.layer.points.len();
                        }
                    }
                }
                out.under
                    .reserve(cache.under.prims.len() + cache.cursor_under.prims.len());
                out.main.reserve(main_prims);
                out.points.reserve(point_count);
            }
            append_retained_layer(&cache.under, &mut out.under, &mut out.points);
            append_retained_layer(&cache.cursor_under, &mut out.under, &mut out.points);
            retained.series_segments[pi].clear();
            retained.drawing_segments[pi].clear();
            // Pane-local tiers from centralized ordering (ordering.rs). `resolved` is the global
            // effective series order (bottom→top); filtering preserves each pane's grouping.
            let explicit = self.series_order_is_explicit();
            let mut idle_indicators: Vec<usize> = Vec::new();
            let mut ordinary_idle: Vec<usize> = Vec::new();
            let mut idle_series: Vec<usize> = Vec::new();
            let mut active_series_idx: Vec<usize> = Vec::new();
            for (ri, rs) in resolved.iter().enumerate() {
                if rs.pane != Some(pi) || !rs.visible {
                    continue;
                }
                // Group promotion: hovering/selecting any output promotes the whole indicator
                // (ordering.rs); use the binding-global max so idle mates move with the active
                // member instead of splitting the group across tiers.
                let (group_priority, is_indicator) =
                    if let Some(binding) = self.indicator_binding_id(rs.id) {
                        let outputs = self.indicator_group_outputs(binding);
                        (self.series_group_priority(&outputs), true)
                    } else {
                        (self.series_active_priority(rs.id), false)
                    };
                if group_priority != crate::ordering::PRIORITY_IDLE {
                    active_series_idx.push(ri);
                    continue;
                }
                if explicit {
                    idle_series.push(ri);
                } else if is_indicator {
                    idle_indicators.push(ri);
                } else {
                    ordinary_idle.push(ri);
                }
            }
            let (idle_drawings, active_drawings) = self.pane_drawing_tiers(pi);
            // Emit one series (retained geometry) and record its segment + paint mark.
            // Note: `resolved`, `retained.series_segments`, and `out` borrow disjointly via
            // indices; the closure form avoids holding `cache` across `self` calls above.
            let emit_series =
                |ri: usize,
                 resolved: &[ResolvedSeries],
                 cache: &RetainedPane,
                 out: &mut FramePane,
                 segments: &mut Vec<FrameSeriesSegment>| {
                    let rs = &resolved[ri];
                    let start = out.main.len();
                    if let Some(layer) = cache.series_layers.iter().find(|layer| layer.id == rs.id)
                    {
                        append_retained_layer(&layer.layer, &mut out.main, &mut out.points);
                        segments.push(FrameSeriesSegment {
                            series_id: Some(rs.id),
                            start,
                            end: out.main.len(),
                            revision: layer.layer.revision,
                            coordinate_revision: layer.layer.coordinate_revision,
                        });
                    }
                    out.series_paint_marks.push((rs.id, out.main.len()));
                };
            // Emit one drawing part by id (retained geometry reuse, no rebuild on reorder).
            let emit_drawing =
                |id: crate::drawings::DrawingId,
                 cache: &RetainedPane,
                 out: &mut FramePane,
                 segments: &mut Vec<FrameDrawingSegment>| {
                    let Some(part) = cache.drawing_parts.iter().find(|part| part.id == id) else {
                        return;
                    };
                    let start = out.main.len();
                    append_drawing_part(
                        &cache.drawings.prims,
                        &cache.drawings.points,
                        part.prim_start,
                        part.prim_end,
                        part.point_start,
                        part.point_end,
                        &mut out.main,
                        &mut out.points,
                    );
                    // Empty drawings (e.g. empty text) emit no prims — no segment needed.
                    if out.main.len() != start {
                        segments.push(FrameDrawingSegment {
                            drawing_id: Some(id),
                            start,
                            end: out.main.len(),
                            revision: cache.drawings.revision,
                            coordinate_revision: cache.drawings.coordinate_revision,
                        });
                    }
                };
            if explicit {
                for id in &idle_drawings {
                    emit_drawing(*id, cache, out, &mut retained.drawing_segments[pi]);
                }
                for ri in idle_series {
                    emit_series(ri, &resolved, cache, out, &mut retained.series_segments[pi]);
                }
            } else {
                for ri in &idle_indicators {
                    emit_series(
                        *ri,
                        &resolved,
                        cache,
                        out,
                        &mut retained.series_segments[pi],
                    );
                }
                for id in &idle_drawings {
                    emit_drawing(*id, cache, out, &mut retained.drawing_segments[pi]);
                }
                for ri in ordinary_idle {
                    emit_series(ri, &resolved, cache, out, &mut retained.series_segments[pi]);
                }
            }
            // Active interleaved by priority (selected below hovered below drag/edit), series
            // before drawings within a tier so annotations stay above promoted series.
            // `active_series_idx` is already priority-ascending (effective order); split tiers.
            let mut active_p1_series: Vec<usize> = Vec::new();
            let mut active_p2_series: Vec<usize> = Vec::new();
            for ri in active_series_idx {
                let id = resolved[ri].id;
                // Group priority (indicator groups move together) decides the tier.
                let group_priority = if let Some(binding) = self.indicator_binding_id(id) {
                    let outputs = self.indicator_group_outputs(binding);
                    self.series_group_priority(&outputs)
                } else {
                    self.series_active_priority(id)
                };
                if group_priority <= crate::ordering::PRIORITY_SELECTED {
                    active_p1_series.push(ri);
                } else {
                    active_p2_series.push(ri);
                }
            }
            let mut active_p1_drawings: Vec<crate::drawings::DrawingId> = Vec::new();
            let mut active_p2_drawings: Vec<crate::drawings::DrawingId> = Vec::new();
            let mut active_p3_drawings: Vec<crate::drawings::DrawingId> = Vec::new();
            for id in active_drawings {
                match self.drawing_active_priority(id) {
                    crate::ordering::PRIORITY_SELECTED => active_p1_drawings.push(id),
                    crate::ordering::PRIORITY_HOVERED => active_p2_drawings.push(id),
                    _ => active_p3_drawings.push(id),
                }
            }
            for ri in active_p1_series {
                emit_series(ri, &resolved, cache, out, &mut retained.series_segments[pi]);
            }
            for id in active_p1_drawings {
                emit_drawing(id, cache, out, &mut retained.drawing_segments[pi]);
            }
            for ri in active_p2_series {
                emit_series(ri, &resolved, cache, out, &mut retained.series_segments[pi]);
            }
            for id in active_p2_drawings {
                emit_drawing(id, cache, out, &mut retained.drawing_segments[pi]);
            }
            for id in active_p3_drawings {
                emit_drawing(id, cache, out, &mut retained.drawing_segments[pi]);
            }
            // General-domain panes cannot contain financial series. Their engine-owned geometry
            // still enters the same ordered `main` primitive stream and receives a retained-group
            // segment so the WebGPU path consumes exactly what Canvas2D/GPUI/native consume.
            let general_start = out.main.len();
            let general_interaction =
                self.build_general_series_frame(pi, hpr, vpr, &mut out.main, &mut out.points);
            if out.main.len() != general_start {
                retained.series_segments[pi].push(FrameSeriesSegment {
                    series_id: None,
                    start: general_start,
                    end: out.main.len(),
                    revision: self
                        .frame_invalidation
                        .scene
                        .max(self.frame_invalidation.coordinate),
                    coordinate_revision: self.frame_invalidation.coordinate,
                });
            }
            let start = out.main.len();
            out.main.extend(general_interaction.into_iter().flatten());
            if out.main.len() != start {
                retained.series_segments[pi].push(FrameSeriesSegment {
                    series_id: None,
                    start,
                    end: out.main.len(),
                    revision: self.frame_invalidation.overlay,
                    coordinate_revision: self.frame_invalidation.coordinate,
                });
            }
            // Trailing creation previews (brush + pending) from retained, topmost among chart
            // content but still below chrome/trading (protected layers) and clipped to the pane.
            {
                let prim_start = cache
                    .drawing_preview_prim_start
                    .min(cache.drawings.prims.len());
                let point_start = cache
                    .drawing_preview_point_start
                    .min(cache.drawings.points.len());
                if prim_start < cache.drawings.prims.len() {
                    let start = out.main.len();
                    append_drawing_part(
                        &cache.drawings.prims,
                        &cache.drawings.points,
                        prim_start,
                        cache.drawings.prims.len(),
                        point_start,
                        cache.drawings.points.len(),
                        &mut out.main,
                        &mut out.points,
                    );
                    retained.drawing_segments[pi].push(FrameDrawingSegment {
                        drawing_id: None,
                        start,
                        end: out.main.len(),
                        revision: cache.drawings.revision,
                        coordinate_revision: cache.drawings.coordinate_revision,
                    });
                }
            }
            let chrome_start = out.main.len();
            append_retained_layer(&cache.chrome, &mut out.main, &mut out.points);
            retained.series_segments[pi].push(FrameSeriesSegment {
                series_id: None,
                start: chrome_start,
                end: out.main.len(),
                revision: cache.chrome.revision,
                coordinate_revision: cache.chrome.coordinate_revision,
            });
            let series_end = out.main.len();
            append_retained_layer(&cache.trading_regions, &mut out.main, &mut out.points);
            let trading_regions_end = out.main.len();
            // Legacy drawings slot kept empty (early drawings already emitted above in pane-local
            // order); `drawings_end == trading_regions_end` preserves the trading slice
            // `drawings_end..trading_end` for existing consumers while new per-drawing segments
            // carry the retained groups.
            let drawings_end = out.main.len();
            append_retained_layer(&cache.trading, &mut out.main, &mut out.points);
            let trading_end = out.main.len();
            append_retained_layer(&cache.overlay, &mut out.main, &mut out.points);
            let overlay_end = out.main.len();
            append_retained_layer(&cache.top_layer, &mut out.top_prims, &mut out.points);
            if pane_left_px != 0 {
                translate_prims_x(&mut out.under, pane_left_px as i32);
                translate_prims_x(&mut out.main, pane_left_px as i32);
                translate_prims_x(&mut out.top_prims, pane_left_px as i32);
                for point in &mut out.points {
                    point[0] += pane_left_px as f32;
                }
            }
            retained.segments[pi] = FramePaneSegments {
                under_end: out.under.len(),
                series_end,
                trading_regions_end,
                drawings_end,
                trading_end,
                overlay_end,
                under_revision: cache.under.revision.max(cache.cursor_under.revision),
                drawings_revision: cache.drawings.revision,
                trading_revision: cache.trading.revision,
                overlay_revision: cache.overlay.revision,
                top_revision: cache.top_layer.revision,
                coordinate_revision: self.frame_invalidation.coordinate,
            };
        }
        retained.layout_generation = self.frame_invalidation.layout;
        retained.scene_generation = self.frame_invalidation.scene;
        retained.chrome_generation = self.frame_invalidation.chrome;
        retained.drawings_generation = self.frame_invalidation.drawings;
        retained.trading_generation = self.frame_invalidation.trading;
        retained.overlay_generation = self.frame_invalidation.overlay;
        retained.autoscale_generation = self.frame_invalidation.autoscale;
        retained.coordinate_generation = self.frame_invalidation.coordinate;
        retained.last_price_scale_revisions.clear();
        retained
            .last_price_scale_revisions
            .extend(self.panes.iter().map(crate::Pane::scale_revisions));
        retained.initialized = true;
        self.retained_frame = retained;
    }

    fn layout_for_frame(&mut self) {
        // Hosts may negotiate an inner content width (for example after measuring the price axis).
        // Preserve that negotiated viewport; standalone/native callers start with pane_w/pane_h
        // equal to the CSS size.
        self.pane_w = if self.pane_w > 0.0 {
            self.pane_w
        } else {
            self.css_width.max(1.0)
        };
        self.pane_h = if self.pane_h > 0.0 {
            self.pane_h
        } else {
            self.css_height.max(1.0)
        };
        self.layout_panes(self.pane_h);
    }

    /// The pane's `layout.background` gradient prim (reference VerticalGradient,
    /// pane-widget.ts `_drawBackground`): a two-stop vertical gradient covering the pane's
    /// bitmap rect. `None` for the solid variant — the backends' clear color paints that.
    /// Accepts the reference's `"gradient"` wire value (and the `"vertical_gradient"` alias).
    fn background_gradient_prim(&self, x: u32, y: u32, w: u32, h: u32) -> Option<Prim> {
        let background = &self.options.get().layout.background;
        if background.kind != "gradient" && background.kind != "vertical_gradient" {
            return None;
        }
        let fallback = Color::rgb(
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.0,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.1,
            aeris_charts_core::style::DEFAULT_SURFACE_RGB.2,
        );
        Some(Prim::Background {
            rect: [x as f32, y as f32, w as f32, h as f32],
            gradient: Gradient {
                top: Color::parse_css(&background.top_color).unwrap_or(fallback),
                bottom: Color::parse_css(&background.bottom_color).unwrap_or(fallback),
            },
        })
    }

    pub(crate) fn visible_range_for_frame(&self) -> Option<(i64, i64)> {
        let n = self.data.merged_times().len() as i64;
        let r = self.time_scale.visible_strict_range()?;
        if n == 0 {
            return None;
        }
        let from = r.left().max(0);
        let to = r.right().min(n - 1);
        (from <= to).then_some((from, to))
    }

    /// Visible merged-time indices for host-side axis labels and hit-testing.
    pub fn visible_range(&self) -> Option<(i64, i64)> {
        self.visible_range_for_frame()
    }

    fn autoscale_for_frame(&mut self, from: i64, to: i64) {
        /// One pane-local scale's autoscale inputs for this pass, in its logical domain.
        struct ScaleAutoscale {
            pane: usize,
            target: PriceScaleTarget,
            exact: Option<PriceRange>,
            /// The same union one bar beyond both visible edges (stable scales only).
            extended: Option<PriceRange>,
            margins: (f64, f64),
            min_move: f64,
            stable: bool,
            center: Option<f64>,
        }
        fn merge(slot: &mut Option<PriceRange>, range: PriceRange) {
            *slot = Some(match slot.take() {
                Some(old) => old.merge(Some(&range)),
                None => range,
            });
        }

        let last_index = self.data.merged_times().len() as i64 - 1;
        let wide_window = ((from - 1).max(0), (to + 1).min(last_index.max(to)));
        let bar_spacing = self.time_scale.bar_spacing();
        let mut scales = Vec::new();
        for (pane_index, pane) in self.panes.iter().enumerate() {
            for target in pane.scale_targets() {
                let Some(scale) = pane.scale(target) else {
                    continue;
                };
                let options = scale.options();
                let center = options.autoscale_center.and_then(|center| {
                    let base = match scale.mode() {
                        PriceScaleMode::Percentage | PriceScaleMode::IndexedTo100 => {
                            options.base_value.or_else(|| {
                                self.scale_formatter_source(pane_index, target)
                                    .and_then(|series| self.series_base_value(series.id, from))
                            })?
                        }
                        _ => 0.0,
                    };
                    let logical = scale.price_to_logical_value(center, base);
                    logical.is_finite().then_some(logical)
                });
                scales.push(ScaleAutoscale {
                    pane: pane_index,
                    target,
                    exact: None,
                    extended: None,
                    margins: (0.0, 0.0),
                    min_move: self.scale_autoscale_min_move(pane_index, target),
                    stable: options.stable_auto_scale && scale.is_auto_scale(),
                    center,
                });
            }
        }
        // Data min/max queries go through the plot cache, which needs the data layer mutably:
        // resolve them first (strict window, plus the one-bar-wider window for stable scales).
        const LOW_HIGH: [PlotValueIndex; 2] = [PlotValueIndex::Low, PlotValueIndex::High];
        let data_min_max: Vec<_> = self
            .series
            .iter()
            .map(|s| {
                if !s.visible {
                    return (None, None);
                }
                let stable = scales.iter().any(|acc| {
                    acc.stable && acc.pane == s.pane_index && acc.target == series_scale_target(s)
                });
                let exact = self.data.min_max_on_range_cached(s.id, from, to, &LOW_HIGH);
                let wide = if stable {
                    self.data
                        .min_max_on_range_cached(s.id, wide_window.0, wide_window.1, &LOW_HIGH)
                } else {
                    None
                };
                (exact, wide)
            })
            .collect();
        for (s, (exact_min_max, wide_min_max)) in self.series.iter().zip(data_min_max) {
            // Hidden series remain engine-owned so they can be toggled back on, but—matching
            // reference—they must not contribute to the active price-scale autoscale range. A
            // pane-less series (its pane was removed) scales nowhere either.
            if !s.visible {
                continue;
            }
            let target = series_scale_target(s);
            let Some(slot) = scales
                .iter()
                .position(|acc| acc.pane == s.pane_index && acc.target == target)
            else {
                continue;
            };
            let Some(base_value) = self.series_base_value(s.id, from) else {
                continue;
            };
            let marker_margins = if s.markers_auto_scale {
                marker_auto_scale_margins(&s.markers, bar_spacing)
            } else {
                (0.0, 0.0)
            };
            let (exact, extended, margins) = match &s.autoscale_info_provider {
                // reference `autoscaleInfoProvider`: the host sees the series' own info and its
                // answer replaces it outright (range and margins).
                Some(provider) => {
                    let own = self.series_autoscale_range(s, exact_min_max, from, to);
                    let base = (!self.data.plot(s.id).is_empty()).then(|| crate::AutoscaleInfo {
                        price_range: own
                            .range
                            .map(|range| (range.min_value(), range.max_value())),
                        margins: (own.has_data && marker_margins != (0.0, 0.0))
                            .then_some(marker_margins),
                    });
                    let info = provider(base);
                    let range = info
                        .and_then(|info| info.price_range)
                        .filter(|(min, max)| min.is_finite() && max.is_finite() && min <= max)
                        .map(|(min, max)| PriceRange::new(min, max));
                    let margins = info
                        .and_then(|info| info.margins)
                        .filter(|(above, below)| above.is_finite() && below.is_finite())
                        .map_or((0.0, 0.0), |(above, below)| {
                            (above.max(0.0), below.max(0.0))
                        });
                    (range, range, margins)
                }
                None => {
                    let own = self.series_autoscale_range(s, exact_min_max, from, to);
                    let wide = if scales[slot].stable {
                        self.series_autoscale_range(s, wide_min_max, wide_window.0, wide_window.1)
                            .range
                    } else {
                        None
                    };
                    let margins = if own.has_data {
                        marker_margins
                    } else {
                        (0.0, 0.0)
                    };
                    (own.range, wide, margins)
                }
            };
            let Some(exact) = exact else {
                continue;
            };
            let scale = pane_scale(&self.panes[s.pane_index], target);
            let Some(exact) = scale.price_range_to_logical(&exact, base_value) else {
                continue;
            };
            let extended =
                extended.and_then(|range| scale.price_range_to_logical(&range, base_value));
            let acc = &mut scales[slot];
            merge(&mut acc.exact, exact);
            if acc.stable {
                merge(&mut acc.extended, extended.unwrap_or(exact));
            }
            acc.margins.0 = acc.margins.0.max(margins.0);
            acc.margins.1 = acc.margins.1.max(margins.1);
        }
        for acc in &scales {
            let pane = &mut self.panes[acc.pane];
            let auto = pane
                .scale(acc.target)
                .is_some_and(PriceScaleCore::is_auto_scale);
            let (above, below) = if auto { acc.margins } else { (0.0, 0.0) };
            match acc.target {
                PriceScaleTarget::Right => {
                    pane.marker_margin_above = above;
                    pane.marker_margin_below = below;
                }
                PriceScaleTarget::Left => {
                    pane.left_marker_margin_above = above;
                    pane.left_marker_margin_below = below;
                }
                PriceScaleTarget::Overlay => {
                    pane.overlay_marker_margin_above = above;
                    pane.overlay_marker_margin_below = below;
                }
                PriceScaleTarget::Named(id) => {
                    if let Some(entry) = pane.named_scale_mut(id) {
                        entry.marker_margin_above = above;
                        entry.marker_margin_below = below;
                    }
                }
            }
        }
        for pane in &mut self.panes {
            pane.refresh_internal_margins();
        }
        for acc in scales {
            let Some(scale) = self.panes[acc.pane].scale_mut(acc.target) else {
                continue;
            };
            if !scale.is_auto_scale() {
                continue;
            }
            let Some(mut exact) = acc.exact else {
                continue;
            };
            let mut extended = acc.extended;
            if acc.target == PriceScaleTarget::Overlay {
                // The overlay (volume) scale always keeps zero in range.
                let zero = PriceRange::new(0.0, 0.0);
                exact = exact.merge(Some(&zero));
                extended = extended.map(|range| range.merge(Some(&zero)));
            }
            scale.apply_autoscale_ranges(Some(exact), extended, acc.center, acc.min_move);
        }
    }

    /// One series' own autoscale range in raw prices over merged bars `[from, to]` (reference
    /// series.ts `_autoscaleInfoImpl`): its data (Heikin-Ashi and footprint-cell aware), the
    /// engine-owned native primitives that participate while the data is visible, and the host
    /// series-primitive contributions recorded for this frame.
    fn series_autoscale_range(
        &self,
        s: &crate::SeriesEntry,
        data_min_max: Option<MinMax>,
        from: i64,
        to: i64,
    ) -> SeriesAutoscaleRange {
        let mut range = self.series_data_autoscale_range(s, data_min_max, from, to);
        let has_data = range.is_some();
        if has_data {
            // Engine-owned volume profiles participate only while their anchored bar span
            // overlaps the visible logical range, matching the official primitive's autoscaleInfo
            // gate.
            for primitive in &s.native_primitives {
                if let Some(primitive_range) =
                    self.native_primitive_autoscale_range(s, primitive, from, to)
                {
                    range = Some(match range {
                        Some(old) => old.merge(Some(&primitive_range)),
                        None => primitive_range,
                    });
                }
            }
        }
        // Series-primitive autoscale contributions (plugin platform Phase C-b): reference merges a
        // series primitive's `autoscaleInfo` into its owning series' autoscale info. The pane and
        // scale recorded with the contribution must still match the series.
        let target = series_scale_target(s);
        for contribution in &self.primitive_autoscale {
            if contribution.series != s.id
                || contribution.pane != s.pane_index
                || contribution.target != target
            {
                continue;
            }
            let contribution = PriceRange::new(contribution.min, contribution.max);
            range = Some(match range {
                Some(old) => old.merge(Some(&contribution)),
                None => contribution,
            });
        }
        SeriesAutoscaleRange { range, has_data }
    }

    /// `data_min_max` is the plot cache's low/high min/max over `[from, to]`.
    fn series_data_autoscale_range(
        &self,
        s: &crate::SeriesEntry,
        data_min_max: Option<MinMax>,
        from: i64,
        to: i64,
    ) -> Option<PriceRange> {
        let (minimum, maximum) = if s.kind == SeriesKind::Candlestick && s.heikin_ashi {
            let plot = self.data.plot(s.id);
            let mut minimum = f64::INFINITY;
            let mut maximum = f64::NEG_INFINITY;
            for row in plot.visible_rows(from, to) {
                if let Some(values) = self.heikin_ashi_row(s.id, row) {
                    minimum = minimum.min(values[2]);
                    maximum = maximum.max(values[1]);
                }
            }
            (minimum, maximum)
        } else {
            let mm = data_min_max?;
            (mm.min, mm.max)
        };
        if !minimum.is_finite() || !maximum.is_finite() {
            return None;
        }
        if s.kind == SeriesKind::Footprint {
            let state = s.footprint.as_ref()?;
            let stream = self.trade_stream(state.trade_stream_id)?;
            let (minimum, maximum) =
                crate::footprint::footprint_row_price_bounds(&stream.options(), minimum, maximum);
            return Some(PriceRange::new(minimum, maximum));
        }
        if s.kind == SeriesKind::Histogram && s.base.is_finite() {
            // reference series.ts `_autoscaleInfoImpl`: a histogram's range always includes its
            // `base`, so columns grow from a visible base and keep heights proportional (a
            // single volume column spans the pane instead of collapsing to a degenerate range).
            return Some(PriceRange::new(minimum.min(s.base), maximum.max(s.base)));
        }
        Some(PriceRange::new(minimum, maximum))
    }

    fn native_primitive_autoscale_range(
        &self,
        s: &crate::SeriesEntry,
        primitive: &crate::native_primitives::NativeSeriesPrimitive,
        from: i64,
        to: i64,
    ) -> Option<PriceRange> {
        match &primitive.kind {
            crate::native_primitives::NativeSeriesPrimitiveKind::BandsIndicator(_) => {
                let plot = self.data.plot(s.id);
                let mut minimum = f64::INFINITY;
                let mut maximum = f64::NEG_INFINITY;
                for row in plot.visible_rows(from, to) {
                    if plot.is_whitespace_row(row) {
                        continue;
                    }
                    let price = plot.value_at(row, PlotValueIndex::Close);
                    if !price.is_finite() {
                        continue;
                    }
                    let first = price * 0.9;
                    let second = price * 1.1;
                    minimum = minimum.min(first.min(second));
                    maximum = maximum.max(first.max(second));
                }
                (minimum.is_finite() && maximum.is_finite())
                    .then(|| PriceRange::new(minimum, maximum))
            }
            crate::native_primitives::NativeSeriesPrimitiveKind::VolumeProfile { data, .. } => {
                let logical = self.time_to_index(data.time as f64, false)?;
                if to < logical || from as f64 > logical as f64 + data.width {
                    return None;
                }
                let (minimum, maximum) = data.profile.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(minimum, maximum), point| {
                        (minimum.min(point.price), maximum.max(point.price))
                    },
                );
                (minimum.is_finite() && maximum.is_finite())
                    .then(|| PriceRange::new(minimum, maximum))
            }
            crate::native_primitives::NativeSeriesPrimitiveKind::TrendLine {
                first_time,
                first_price,
                second_time,
                second_price,
                ..
            } => {
                let first = self.time_to_index(*first_time as f64, false)?;
                let second = self.time_to_index(*second_time as f64, false)?;
                if to < first.min(second) || from > first.max(second) {
                    return None;
                }
                Some(PriceRange::new(
                    first_price.min(*second_price),
                    first_price.max(*second_price),
                ))
            }
            _ => None,
        }
    }

    fn time_marks_for_frame(&mut self) -> Vec<(i64, u8)> {
        // reference time-scale.ts:635 — `(fontSize + 4) * 5 / 8 * tickMarkMaxCharacterLength` with
        // the grid's fixed 12px estimate; the option widens/narrows the mark spacing.
        let max_width = (12.0 + 4.0) * 5.0 / 8.0 * f64::from(self.tick_mark_max_character_length);
        self.time_marks(max_width)
    }

    /// Build the time marks used by both the frame grid and host axis labels: the explicit
    /// host marks when set, otherwise the automatic weight-and-spacing selection.
    pub fn time_marks(&mut self, max_label_width: f64) -> Vec<(i64, u8)> {
        if let Some(marks) = self.resolved_time_tick_marks() {
            return marks
                .into_iter()
                .map(|mark| (mark.index, mark.weight))
                .collect();
        }
        self.tick_marks
            .build(self.time_scale.bar_spacing(), max_label_width)
            .iter()
            .map(|m| (m.index, m.weight))
            .collect()
    }
}
