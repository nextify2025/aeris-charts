//! B8 Projection & Annotations family, own-line part: the measuring ranges `price_range`,
//! `date_range`, and `date_price_range` (wire ids 13..=15, range fills with arrowed measures and
//! engine-formatted stats, grid-snapped anchors), and the KLineChart `simple_tag` and
//! `simple_annotation` (wire ids 245 and 246).
//!
//! Upstream renders the catalog's projection and annotation tools (forecast, bars pattern,
//! projection, anchored text, note, price note, callout, comment, price label, signpost, flag mark,
//! arrow markers, icon stamp). This module keeps the data readers re-applied on upstream's
//! forecast ([`forecast_status`], memoized in [`ForecastMemo`]), the built-in vector icon glyphs
//! upstream's icon stamp falls back to ([`built_in_icon_parts`]), the fork's public option block
//! ([`ProjectionAnnotationToolOptions`], [`BarsPatternMode`], [`DrawingIcon`]), and, for documents
//! the fork wrote, its pre-merge kind defaults ([`legacy_defaults`]).
//!
//! Every shape resolves into shared [`DrawingParts`], so frames and hit testing read the same
//! geometry. The family renders the common `text` and `labels` itself: range fills at 20% of the
//! stroke color, range arrows on the measured end, and box text that contrasts with its
//! background unless `text_color` overrides it.

use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;
use aeris_charts_core::model::plot_list::PlotValueIndex;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::Point;

use super::super::parts::{
    text_lines, DrawingParts, PartContext, PartLabel, PartStroke, STATS_ALPHA, STATS_PADDING,
};
use super::super::tools::{
    DrawingAnchorLink, DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis,
    DrawingPlacement, DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout,
    DrawingToolSpec,
};
use super::super::{Drawing, DrawingTextHAlign, DrawingTextVAlign};
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingKind, DrawingKindOptions, DrawingLabelMetric, DrawingLabelOptions,
    DrawingLabelPosition, DrawingLineCap,
};

/// How a bars pattern paints its copied bars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarsPatternMode {
    /// One high–low stick per bar.
    #[default]
    HlBars,
    /// One open–close stick per bar.
    OcBars,
    /// A line through the opens.
    LineOpen,
    /// A line through the highs.
    LineHigh,
    /// A line through the lows.
    LineLow,
    /// A line through the closes.
    LineClose,
}

/// The bounded built-in icon set: the fork's `icon` tool stamps, and the vector glyphs upstream's
/// icon stamp paints for these names when no raster of that name is registered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingIcon {
    #[default]
    Star,
    Heart,
    Check,
    Cross,
    Circle,
    Square,
    Diamond,
    TriangleUp,
    TriangleDown,
}

impl DrawingIcon {
    const ALL: [Self; 9] = [
        Self::Star,
        Self::Heart,
        Self::Check,
        Self::Cross,
        Self::Circle,
        Self::Square,
        Self::Diamond,
        Self::TriangleUp,
        Self::TriangleDown,
    ];

    /// The built-in icon `name` names (its snake_case name), `None` for any other name.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|icon| icon.name() == name)
    }

    /// The icon's snake_case name, as `icon_name` carries it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Star => "star",
            Self::Heart => "heart",
            Self::Check => "check",
            Self::Cross => "cross",
            Self::Circle => "circle",
            Self::Square => "square",
            Self::Diamond => "diamond",
            Self::TriangleUp => "triangle_up",
            Self::TriangleDown => "triangle_down",
        }
    }
}

/// Icon size bounds and default, in CSS px.
const ICON_SIZE_MIN: f64 = 8.0;
const ICON_SIZE_MAX: f64 = 128.0;
const DEFAULT_ICON_SIZE: f64 = 24.0;

/// Projection & Annotations options (`tool_options.projection_annotation`); absent fields keep
/// their defaults. For upstream's bars pattern and icon stamp, `bars_mode`, `mirrored`,
/// `flipped`, `bars`, `icon`, and `icon_size` are input aliases of the flat `bars_pattern_*`,
/// `icon_name`, and `icon_size` fields (see `drawing_contract::take_legacy_flat_options`);
/// `always_show_text` is stored but not rendered. Every field is serialized only when it differs
/// from its default, so a block at its defaults is written as `{}`: the marker documents the fork
/// wrote carry on their annotations (see `kinds::legacy_fork_tool_options`), with no alias key
/// written back.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectionAnnotationToolOptions {
    /// Bars pattern: how the copied bars paint.
    #[serde(skip_serializing_if = "is_default")]
    pub bars_mode: BarsPatternMode,
    /// Bars pattern: reverse the copied bars in time.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mirrored: bool,
    /// Bars pattern: turn the copied bars upside down within the box between the two anchors.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub flipped: bool,
    /// Bars pattern: the fork's copied `[open, high, low, close]` bars, oldest first (an input
    /// alias of upstream's `bars_pattern` snapshot). Named templates keep only the style. At most
    /// [`MAX_BARS_PATTERN_BARS`](crate::MAX_BARS_PATTERN_BARS).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bars: Vec<[f64; 4]>,
    /// Icon: the stamped icon.
    #[serde(skip_serializing_if = "is_default")]
    pub icon: DrawingIcon,
    /// Icon: the icon's size in CSS px (8..=128).
    #[serde(skip_serializing_if = "is_default_icon_size")]
    pub icon_size: f64,
    /// Note: paint the text box while the note is neither hovered, selected, nor edited (by
    /// default only the pin shows then, like the reference platform's note). Serialized only
    /// when set, so documents without it keep their exact form.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub always_show_text: bool,
}

static DEFAULT_OPTIONS: ProjectionAnnotationToolOptions = ProjectionAnnotationToolOptions {
    bars_mode: BarsPatternMode::HlBars,
    mirrored: false,
    flipped: false,
    bars: Vec::new(),
    icon: DrawingIcon::Star,
    icon_size: DEFAULT_ICON_SIZE,
    always_show_text: false,
};

impl Default for ProjectionAnnotationToolOptions {
    fn default() -> Self {
        DEFAULT_OPTIONS.clone()
    }
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

fn is_default_icon_size(size: &f64) -> bool {
    *size == DEFAULT_ICON_SIZE
}

impl ProjectionAnnotationToolOptions {
    /// Bounded pattern length, finite in-range prices, and an icon size within 8..=128 CSS px.
    pub fn validate(&self) -> bool {
        self.bars.len() <= crate::drawings::MAX_BARS_PATTERN_BARS
            && self
                .bars
                .iter()
                .flatten()
                .all(|value| value.is_finite() && value.abs() <= MAX_SAFE_VALUE)
            && self.icon_size.is_finite()
            && (ICON_SIZE_MIN..=ICON_SIZE_MAX).contains(&self.icon_size)
    }
}

/// Gap between an anchor or edge and a label box, in CSS px.
const LABEL_GAP: f64 = 8.0;
/// Annotation box padding (horizontal, vertical) in CSS px.
const BOX_PADDING: (f64, f64) = (8.0, 4.0);
/// Range fill alpha over the drawing color when `fill_color` is unset (the rectangle's 20%).
const FILL_ALPHA: u8 = 51;
/// Simple annotation: the gap under the stem, the stem length, and the head's height and half
/// width (CSS px), as in KLineChart's overlay.
const ANNOTATION_GAP: f64 = 6.0;
const ANNOTATION_STEM: f64 = 50.0;
const ANNOTATION_HEAD: f64 = 5.0;
const ANNOTATION_HEAD_HALF: f64 = 4.0;
/// Icon stroke width as a fraction of the icon size (check and cross).
const ICON_STROKE: f64 = 0.14;
/// Heart outline samples.
const HEART_SAMPLES: usize = 48;

/// Shared placement and editing behavior; every spec below overrides its identity.
const TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceRange,
    wire_id: 13,
    name: "price_range",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    // Placement opens the editor only for the simple annotation, whose text the user supplies;
    // double-click, Enter, or F2 on a selected text box edits it in place through the label its
    // parts mark with `DrawingParts::text_label`.
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

/// One-anchor annotation behavior.
const MARK: DrawingToolSpec = DrawingToolSpec {
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    ..TOOL
};

pub(crate) const PRICE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceRange,
    wire_id: 13,
    name: "price_range",
    // Anchors snap to whole bars and price ticks so the statistics read integral bars and ticks.
    grid_snap: true,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
    ..TOOL
};

pub(crate) const DATE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DateRange,
    wire_id: 14,
    name: "date_range",
    // Anchors snap to whole bars and price ticks so the statistics read integral bars and ticks.
    grid_snap: true,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
    ..TOOL
};

pub(crate) const DATE_PRICE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DatePriceRange,
    wire_id: 15,
    name: "date_price_range",
    // Anchors snap to whole bars and price ticks so the statistics read integral bars and ticks.
    grid_snap: true,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
    ..TOOL
};

// KLineChart's simple tag: a line across the pane whose text, or price, is the price-axis tag. The
// text is never painted on the chart, so there is nothing to edit in place.
pub(crate) const SIMPLE_TAG: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SimpleTag,
    wire_id: 245,
    name: "simple_tag",
    logical_extent: DrawingLogicalExtent::Full,
    axis_price_label: true,
    axis_tag_text: true,
    ..MARK
};

// KLineChart's simple annotation: a stem with a head under a boxed text. Placing it opens the
// editor, like the other tools whose text the user supplies.
pub(crate) const SIMPLE_ANNOTATION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::SimpleAnnotation,
    wire_id: 246,
    name: "simple_annotation",
    requests_text_editor: true,
    ..MARK
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.owns_labels = true;
    family.owns_text = true;
    family
};

/// Visible stats labels for `metrics`, in order.
fn stats(metrics: &[DrawingLabelMetric]) -> Vec<DrawingLabelOptions> {
    metrics
        .iter()
        .map(|&metric| DrawingLabelOptions {
            metric,
            visible: true,
            position: DrawingLabelPosition::On,
            text: None,
        })
        .collect()
}

fn apply_defaults(drawing: &mut Drawing) {
    use DrawingLabelMetric::{BarCount, Duration, PercentChange, PriceChange, Ticks};
    match drawing.kind {
        DrawingKind::PriceRange => {
            drawing.fill_enabled = true;
            drawing.stroke_end = DrawingLineCap::Arrow;
            drawing.labels = stats(&[PriceChange, PercentChange, Ticks]);
        }
        DrawingKind::DateRange => {
            drawing.fill_enabled = true;
            drawing.stroke_end = DrawingLineCap::Arrow;
            drawing.labels = stats(&[BarCount, Duration]);
        }
        DrawingKind::DatePriceRange => {
            drawing.fill_enabled = true;
            drawing.stroke_end = DrawingLineCap::Arrow;
            drawing.labels = stats(&[PriceChange, PercentChange, Ticks, BarCount, Duration]);
        }
        // KLineChart draws both dashed.
        DrawingKind::SimpleTag | DrawingKind::SimpleAnnotation => drawing.style = LineStyle::Dashed,
        _ => {}
    }
}

/// The fork's pre-merge defaults of the upstream projection and annotation tools it rendered (see
/// [`super::apply_legacy_fork_defaults`]): the projection's sector fill, the annotations' starter
/// text, the anchored text's top-left alignment, and the market colors of the up and down arrow
/// marks.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    match drawing.kind {
        DrawingKind::Projection => drawing.fill_enabled = true,
        DrawingKind::AnchoredText => {
            drawing.text = "Text".to_string();
            drawing.text_h_align = DrawingTextHAlign::Left;
            drawing.text_v_align = DrawingTextVAlign::Top;
        }
        DrawingKind::Note => drawing.text = "Note".to_string(),
        DrawingKind::Callout => drawing.text = "Callout".to_string(),
        DrawingKind::Comment => drawing.text = "Comment".to_string(),
        DrawingKind::Signpost => drawing.text = "Signpost".to_string(),
        DrawingKind::ArrowMarkerUp => {
            drawing.color = aeris_charts_core::style::MARKET_UP_CSS.to_string();
        }
        DrawingKind::ArrowMarkerDown => {
            drawing.color = aeris_charts_core::style::MARKET_DOWN_CSS.to_string();
        }
        _ => {}
    }
}

/// The fork-form marker (see `kinds::legacy_fork_tool_options`): an empty
/// `tool_options.projection_annotation` block on the annotations whose fork look (projection
/// sector, note pin, speech bubbles, boxed price note, signpost plate and pole editor, arrow-mark
/// text, forecast boxes) upstream does not draw. The block's presence selects that look on
/// upstream's lowering, so documents the fork wrote keep it and new drawings keep upstream's.
pub(super) fn legacy_tool_options(kind: DrawingKind) -> Option<(&'static str, serde_json::Value)> {
    matches!(
        kind,
        DrawingKind::Projection
            | DrawingKind::Note
            | DrawingKind::Comment
            | DrawingKind::PriceNote
            | DrawingKind::PriceLabel
            | DrawingKind::Signpost
            | DrawingKind::ArrowMarkerUp
            | DrawingKind::ArrowMarkerDown
            | DrawingKind::ArrowMarkerLeft
            | DrawingKind::ArrowMarkerRight
            | DrawingKind::Forecast
    )
    .then(|| ("projection_annotation", serde_json::json!({})))
}

fn options(drawing: &Drawing) -> &ProjectionAnnotationToolOptions {
    drawing
        .tool_options
        .projection_annotation
        .as_ref()
        .unwrap_or(&DEFAULT_OPTIONS)
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    let options = options(drawing);
    DrawingKindOptions::ProjectionAnnotation {
        bars_mode: options.bars_mode,
        mirrored: options.mirrored,
        flipped: options.flipped,
        pattern_bars: options.bars.len(),
        icon: options.icon,
        icon_size: options.icon_size,
        always_show_text: options.always_show_text,
    }
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    match ctx.drawing.kind {
        DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DatePriceRange => {
            range(ctx, parts)
        }
        DrawingKind::SimpleTag => simple_tag(ctx, parts),
        DrawingKind::SimpleAnnotation => simple_annotation(ctx, parts),
        _ => {}
    }
}

// --- shared styling ---------------------------------------------------------------------------

fn with_alpha(color: Color, alpha: u8) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), alpha)
}

/// Text on a box: `text_color`, else black or white against the box.
fn box_text_color(drawing: &Drawing, background: Color) -> Color {
    drawing
        .text_color
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or_else(|| background.solid().contrast_text())
}

/// The drawing's text lines for culling: one empty line when the text is empty, the caret line
/// its editor keeps, so the pad covers the box while it is being typed into.
fn culling_text_lines(drawing: &Drawing) -> Vec<String> {
    let mut lines = text_lines(&drawing.text);
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// A text box in the drawing's own font on `background` (`None` = no box), with the optional
/// `box_border_color` frame.
fn text_box(
    ctx: &PartContext<'_>,
    anchor: Point,
    (h_align, v_align): (DrawingTextHAlign, DrawingTextVAlign),
    lines: Vec<String>,
    background: Option<Color>,
) -> PartLabel {
    let drawing = ctx.drawing;
    PartLabel {
        anchor,
        h_align,
        v_align,
        lines,
        size: ctx.engine.drawing_text_size(drawing) * ctx.scale,
        weight: drawing.text_weight.unwrap_or(400),
        italic: drawing.text_italic,
        color: background.map(|background| box_text_color(drawing, background)),
        background,
        border: drawing
            .box_border_color
            .as_deref()
            .and_then(Color::parse_css),
        padding: (BOX_PADDING.0 * ctx.scale, BOX_PADDING.1 * ctx.scale),
        hit: true,
    }
}

/// A measurement box (the Lines stats-box style) on `background`.
fn stats_box(
    ctx: &PartContext<'_>,
    anchor: Point,
    (h_align, v_align): (DrawingTextHAlign, DrawingTextVAlign),
    lines: Vec<String>,
    background: Color,
) -> PartLabel {
    let text = box_text_color(ctx.drawing, background);
    ctx.stats_label(anchor, (h_align, v_align), lines, background, text)
}

/// Widest of `lines` in CSS px at `size` in the drawing's font.
fn widest(engine: &ChartEngine, drawing: &Drawing, lines: &[String], size: f64) -> f64 {
    let family = &engine.options.get().layout.font_family;
    lines
        .iter()
        .map(|line| {
            engine.measure_text_run(
                line,
                size,
                family,
                drawing.text_weight.unwrap_or(400),
                drawing.text_italic,
            )
        })
        .fold(0.0, f64::max)
}

/// Reach of a box of `lines` placed `gap` beyond an anchor, in CSS px: the larger of its padded
/// width and height plus the gap.
fn box_reach(
    engine: &ChartEngine,
    drawing: &Drawing,
    lines: &[String],
    size: f64,
    padding: (f64, f64),
    gap: f64,
) -> f64 {
    if lines.is_empty() {
        return 0.0;
    }
    let width = widest(engine, drawing, lines, size) + 2.0 * padding.0;
    let height = lines.len() as f64 * size * 1.25 + 2.0 * padding.1;
    gap + width.max(height)
}

/// A logical position as a bar index, `None` beyond the representable range.
fn bar_index(logical: f64) -> Option<i64> {
    (logical.is_finite() && logical.abs() < 9.0e15).then_some(logical as i64)
}

/// Everything an as-of forecast outcome reads: its source's data, the axis points, the replay
/// boundary, and the drawing's window and target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ForecastKey {
    series: crate::SeriesId,
    generation: u64,
    time_points: u64,
    replay: Option<i64>,
    first_index: i64,
    last_index: i64,
    target: u64,
    rising: bool,
}

/// The latest as-of forecast outcome of each drawing (bounded like the regression memo).
pub(crate) type ForecastMemo =
    std::collections::HashMap<crate::DrawingId, (ForecastKey, Option<bool>)>;

/// Forecast outcome from the source series: `Some(true)` once a bar after the source bar reaches
/// the target price (a high at or above a rising target, a low at or below a falling one) by the
/// target bar, `Some(false)` once a traded bar after the target bar exists without that (the
/// target bar itself may still be forming, and whitespace rows such as host-installed future
/// session slots are not bars), `None` while pending or without a source series. The bars are
/// the source's own (canonical) rows: an as-of source's rows collapsed between two axis points
/// count, and a point repeating the source bar is not a later bar. On a union source the extrema
/// and latest-bar queries walk the LOD pyramid, so a long forecast costs a logarithmic number of
/// summaries, not a scan of its range; an as-of source has no canonical-row pyramid, so its
/// outcome scans the window's rows once per data, axis, clock, or anchor change (memoized).
pub(crate) fn forecast_status(engine: &ChartEngine, drawing: &Drawing) -> Option<bool> {
    let (&source, &target) = (drawing.points.first()?, drawing.points.get(1)?);
    if target.logical <= source.logical {
        return None;
    }
    let rows = engine.drawing_source_window(drawing)?;
    let first_index = bar_index(source.logical.floor())?.checked_add(1)?;
    let last_index = bar_index(target.logical.floor())?;
    let rising = target.price >= source.price;
    let reached = |row: usize| {
        if rows.is_whitespace(row) {
            return false;
        }
        if rising {
            rows.value(row, PlotValueIndex::High) >= target.price
        } else {
            rows.value(row, PlotValueIndex::Low) <= target.price
        }
    };
    if rows.is_as_of() {
        let key = ForecastKey {
            series: rows.series,
            generation: engine.data.series_generation(rows.series)?,
            time_points: engine.data.time_points_generation(),
            replay: engine.replay_clock_micros,
            first_index,
            last_index,
            target: target.price.to_bits(),
            rising,
        };
        let mut memo = engine.drawing_settings.forecast_memo.borrow_mut();
        if let Some(&(_, status)) = memo
            .get(&drawing.id)
            .filter(|(memo_key, _)| *memo_key == key)
        {
            return status;
        }
        let status = if rows.rows_between(first_index, last_index).any(reached) {
            Some(true)
        } else {
            (0..rows.placed_rows())
                .rev()
                .find(|&row| !rows.is_whitespace(row))
                .and_then(|row| rows.index_of(row))
                .filter(|&index| index > last_index)
                .map(|_| false)
        };
        super::super::insert_drawing_memo(&mut memo, &engine.drawings, drawing.id, (key, status));
        return status;
    }
    let plot = rows.plot;
    let data_last = plot.index_at(plot.last_non_whitespace_row_before(plot.size())?)?;
    if first_index <= last_index.min(data_last) {
        if let (Some(first_row), Some(last_row)) = (
            plot.first_non_whitespace_row(first_index),
            plot.last_non_whitespace_row(last_index.min(data_last)),
        ) {
            if first_row <= last_row {
                let hit = match plot.lod() {
                    Some(lod) => lod
                        .rows_on_range(first_row..last_row + 1, usize::MAX)
                        .0
                        .iter()
                        .any(reached),
                    None => (first_row..=last_row).any(reached),
                };
                if hit {
                    return Some(true);
                }
            }
        }
    }
    (data_last > last_index).then_some(false)
}

/// Price, date, and date-price ranges: the fill between the anchors, the edge lines of the
/// measured axis, the arrowed measure through the middle toward the second anchor, and the
/// stats box beyond the measured end.
fn range(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let (left, right) = (a.0.min(b.0), a.0.max(b.0));
    let (top, bottom) = (a.1.min(b.1), a.1.max(b.1));
    let (middle_x, middle_y) = ((left + right) / 2.0, (top + bottom) / 2.0);
    if drawing.fill_enabled {
        parts.fill_convex(
            &[(left, top), (right, top), (right, bottom), (left, bottom)],
            Some(drawing.fill_or_wash(FILL_ALPHA)),
            true,
        );
    }
    let (price, time) = crate::drawings::MeasureAxes::for_kind(drawing.kind)
        .map_or((false, false), |axes| (axes.price(), axes.date()));
    if drawing.kind == DrawingKind::PriceRange {
        parts.hline(a.1, left, right, PartStroke::default());
        parts.hline(b.1, left, right, PartStroke::default());
    }
    if drawing.kind == DrawingKind::DateRange {
        parts.vline(a.0, top, bottom, PartStroke::default());
        parts.vline(b.0, top, bottom, PartStroke::default());
    }
    if price {
        range_arrow(ctx, parts, true, middle_x, a.1, b.1);
    }
    if time {
        range_arrow(ctx, parts, false, middle_y, a.0, b.0);
    }
    if !drawing.labels.iter().any(|label| label.visible) {
        return;
    }
    let lines = ctx.engine.drawing_stat_lines(drawing, 0, 1);
    if lines.is_empty() {
        return;
    }
    let gap = LABEL_GAP * ctx.scale;
    // Beyond the measured end: the second anchor's price edge, or below a date range.
    let below = !price || b.1 >= a.1;
    let anchor = if below {
        (middle_x, bottom + gap)
    } else {
        (middle_x, top - gap)
    };
    let v_align = if below {
        DrawingTextVAlign::Top
    } else {
        DrawingTextVAlign::Bottom
    };
    parts.label(stats_box(
        ctx,
        anchor,
        (DrawingTextHAlign::Center, v_align),
        lines,
        with_alpha(drawing.stroke_color(), STATS_ALPHA),
    ));
}

/// One measured axis' arrow from `from` to `to` (coordinates along the axis) through `middle` (the
/// area's center line across it): a crisp device-pixel shaft like the rules, ended by the drawing's
/// own caps. Shaft and rules are whole device pixels on every executor, so the lines of a measured
/// area paint identically on WebGPU and Canvas2D; the translucent fill is not snapped, so its own
/// edges may anti-alias by up to half a device pixel off the rules. The caps' apexes sit on the
/// shaft's pixel center. An arrow-capped end trims the shaft back by one stroke width (as
/// [`DrawingParts::capped_polyline`] does), and an arrow whose ends share a pixel paints nothing.
fn range_arrow(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    vertical: bool,
    middle: f64,
    from: f64,
    to: f64,
) {
    let drawing = ctx.drawing;
    let width = (drawing.width * ctx.scale).round().max(1.0);
    // A crisp line at integer coordinate `v` covers `[v - width / 2, v - width / 2 + width)`.
    let center = |v: f64| v - (width / 2.0).floor() + width / 2.0;
    let (from_px, to_px) = (from.round(), to.round());
    if from_px == to_px {
        return;
    }
    let lane = middle.round();
    let trims = |cap: crate::DrawingLineCap| {
        if cap == crate::DrawingLineCap::Arrow && (to_px - from_px).abs() > width * 2.0 {
            width
        } else {
            0.0
        }
    };
    let (start_trim, end_trim) = (trims(drawing.stroke_start), trims(drawing.stroke_end));
    // The shaft covers both end pixels.
    let (low, high) = if to_px > from_px {
        (from_px + start_trim, to_px - end_trim + 1.0)
    } else {
        (to_px + end_trim, from_px - start_trim + 1.0)
    };
    if vertical {
        parts.vline(lane, low, high, PartStroke::default());
    } else {
        parts.hline(lane, low, high, PartStroke::default());
    }
    let tip = |along: f64| {
        if vertical {
            (center(lane), center(along))
        } else {
            (center(along), center(lane))
        }
    };
    let (start, end) = (tip(from_px), tip(to_px));
    let cap_width = drawing.width * ctx.scale;
    parts.line_cap(drawing.stroke_end, end, start, cap_width);
    parts.line_cap(drawing.stroke_start, start, end, cap_width);
}

/// Simple tag: a line across the pane at the anchor's price. Its text, or its price, is the tag on
/// the price axis; nothing is painted on the chart.
fn simple_tag(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let Some(&a) = ctx.px.first() else {
        return;
    };
    parts.hline(a.1, ctx.pane.left, ctx.pane.right, PartStroke::default());
}

/// Simple annotation: a stem rising from just above the anchor, a head pointing down at the stem's
/// top, and the boxed text above the head.
fn simple_annotation(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let Some(&foot) = ctx.px.first() else {
        return;
    };
    let s = ctx.scale;
    let stem_bottom = foot.1 - ANNOTATION_GAP * s;
    let stem_top = stem_bottom - ANNOTATION_STEM * s;
    let head_top = stem_top - ANNOTATION_HEAD * s;
    parts.vline(foot.0, stem_top, stem_bottom, PartStroke::default());
    parts.fill_convex(
        &[
            (foot.0, stem_top),
            (foot.0 - ANNOTATION_HEAD_HALF * s, head_top),
            (foot.0 + ANNOTATION_HEAD_HALF * s, head_top),
        ],
        None,
        true,
    );
    parts.text_label(
        text_box(
            ctx,
            (foot.0, head_top),
            (DrawingTextHAlign::Center, DrawingTextVAlign::Bottom),
            ctx.text_lines(),
            Some(ctx.drawing.stroke_color()),
        ),
        0,
    );
}

/// A built-in icon's vector glyph centered on `center`, `size` caller px across, in the drawing
/// color: upstream's icon stamp paints it when no raster is registered under a built-in name.
/// Filled icons are one convex polygon, a disc, or a fan around their center (star, heart:
/// star-shaped outlines), so paint and hit test cover exactly the icon.
pub(crate) fn built_in_icon_parts(
    ctx: &PartContext<'_>,
    icon: DrawingIcon,
    center: Point,
    size: f64,
    parts: &mut DrawingParts,
) {
    let at = |x: f64, y: f64| (center.0 + x * size, center.1 + y * size);
    let stroke = PartStroke::decoration(
        size / ctx.scale.max(f64::EPSILON) * ICON_STROKE,
        LineStyle::Solid,
    );
    match icon {
        DrawingIcon::Circle => parts.disc(center, size * 0.5, None),
        DrawingIcon::Square => parts.fill_convex(
            &[at(-0.4, -0.4), at(0.4, -0.4), at(0.4, 0.4), at(-0.4, 0.4)],
            None,
            true,
        ),
        DrawingIcon::Diamond => parts.fill_convex(
            &[at(0.0, -0.5), at(0.5, 0.0), at(0.0, 0.5), at(-0.5, 0.0)],
            None,
            true,
        ),
        DrawingIcon::TriangleUp => {
            parts.fill_convex(&[at(0.0, -0.45), at(0.5, 0.4), at(-0.5, 0.4)], None, true)
        }
        DrawingIcon::TriangleDown => {
            parts.fill_convex(&[at(0.0, 0.45), at(-0.5, -0.4), at(0.5, -0.4)], None, true)
        }
        DrawingIcon::Check => parts.stroke(
            &[at(-0.36, 0.02), at(-0.1, 0.28), at(0.4, -0.3)],
            stroke,
            false,
        ),
        DrawingIcon::Cross => {
            parts.stroke(&[at(-0.32, -0.32), at(0.32, 0.32)], stroke, false);
            parts.stroke(&[at(-0.32, 0.32), at(0.32, -0.32)], stroke, false);
        }
        DrawingIcon::Star => {
            let outline = (0..=10)
                .map(|step| {
                    let angle =
                        -std::f64::consts::FRAC_PI_2 + step as f64 * std::f64::consts::PI / 5.0;
                    let radius = if step % 2 == 0 { 0.5 } else { 0.2 };
                    at(radius * angle.cos(), radius * angle.sin())
                })
                .collect::<Vec<_>>();
            fan(parts, center, &outline);
        }
        DrawingIcon::Heart => {
            // The parametric heart (x = 16 sin³t, y = 13 cos t − 5 cos 2t − 2 cos 3t − cos 4t, y
            // up) spans 34 × 29 units around a point it is star-shaped from.
            let kernel = at(0.0, 0.05);
            let outline = (0..=HEART_SAMPLES)
                .map(|step| {
                    let t = std::f64::consts::TAU * step as f64 / HEART_SAMPLES as f64;
                    let x = 16.0 * t.sin().powi(3);
                    let y = 13.0 * t.cos()
                        - 5.0 * (2.0 * t).cos()
                        - 2.0 * (3.0 * t).cos()
                        - (4.0 * t).cos();
                    at(x / 34.0, -(y + 2.5) / 34.0)
                })
                .collect::<Vec<_>>();
            fan(parts, kernel, &outline);
        }
    }
}

/// Fill a closed outline that is star-shaped from `kernel` as one region: the ribbon between the
/// kernel (repeated) and the outline is a triangle fan, so the executors paint the outline and the
/// hit test covers exactly its triangles.
fn fan(parts: &mut DrawingParts, kernel: Point, outline: &[Point]) {
    let hub = vec![kernel; outline.len()];
    parts.fill(&hub, outline, None, true);
}

// --- culling ------------------------------------------------------------------------------------

/// Conservative reach of the family's boxes beyond the anchors, in CSS px. Text that follows data
/// (bar counts, durations) gets four ems of slack so the cached pad stays valid between text-key
/// refreshes.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let size = engine.drawing_text_size(drawing);
    let stats = engine.drawing_stats_size();
    match drawing.kind {
        DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DatePriceRange => {
            let lines = engine.drawing_stat_lines(drawing, 0, 1);
            box_reach(engine, drawing, &lines, stats, STATS_PADDING, LABEL_GAP) + 4.0 * stats
        }
        DrawingKind::SimpleAnnotation => {
            let lines = culling_text_lines(drawing);
            ANNOTATION_GAP
                + ANNOTATION_STEM
                + ANNOTATION_HEAD
                + box_reach(engine, drawing, &lines, size, BOX_PADDING, 0.0)
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests;
