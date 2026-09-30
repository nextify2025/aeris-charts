//! B8 Projection & Annotations family (wire ids 128..=159).
//!
//! Projection and measuring: `forecast` (a source → target projection whose target label turns
//! into an engine-evaluated success or failure), `bars_pattern` (a ghost copy of the bars between
//! its anchors, captured once at creation), `price_range`, `date_range`, `date_and_price_range`
//! (range fills with arrowed measures and engine-formatted stats), and `projection` (a circular
//! sector from an apex through a radius point to a price point).
//!
//! Annotations: `anchored_text` (text pinned to a pane position — its anchor is a pane fraction,
//! see [`DrawingFamily::pane_anchored`]), `note` (pin marker with text), `price_note` (leader to
//! a label with the priced point's price), `callout` (text box with a pointer), `comment` and
//! `price_label` (speech bubbles whose tail touches the anchor), `signpost` (pole with a text
//! plate), `flag_mark`, four arrow marks, and `icon` stamps from a bounded built-in set drawn
//! with the shared parts (no external assets).
//!
//! Every shape resolves into shared [`DrawingParts`], so frames and hit testing read the same
//! geometry. The family renders the common `text` and `labels` itself. Default styles follow the
//! conventional professional-platform look: the canonical primary color for markers and boxes,
//! the market up/down colors for the up/down arrow marks, range fills at 20% of the stroke color,
//! range arrows on the measured end, and box text that contrasts with its background unless
//! `text_color` overrides it.

use aeris_charts_core::model::data_validation::MAX_SAFE_VALUE;
use aeris_charts_core::model::plot_list::PlotValueIndex;
use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::parts::{
    cap_radius, text_lines, DrawingParts, PartContext, PartLabel, PartStroke, CURVE_TOLERANCE,
    STATS_ALPHA, STATS_PADDING,
};
use super::super::tools::{
    DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis, DrawingPlacement,
    DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout, DrawingToolSpec,
};
use super::super::{Drawing, DrawingPoint, DrawingTextHAlign, DrawingTextVAlign};
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingKind, DrawingKindOptions, DrawingLabelMetric, DrawingLabelOptions,
    DrawingLabelPosition, DrawingLineCap, DrawingPropertyDescriptor, DrawingPropertyType,
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

impl BarsPatternMode {
    const ALL: [Self; 6] = [
        Self::HlBars,
        Self::OcBars,
        Self::LineOpen,
        Self::LineHigh,
        Self::LineLow,
        Self::LineClose,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::HlBars => "hl_bars",
            Self::OcBars => "oc_bars",
            Self::LineOpen => "line_open",
            Self::LineHigh => "line_high",
            Self::LineLow => "line_low",
            Self::LineClose => "line_close",
        }
    }

    /// The OHLC slot a line mode follows.
    fn line_field(self) -> Option<usize> {
        match self {
            Self::HlBars | Self::OcBars => None,
            Self::LineOpen => Some(0),
            Self::LineHigh => Some(1),
            Self::LineLow => Some(2),
            Self::LineClose => Some(3),
        }
    }
}

/// The bounded built-in icon set of the `icon` tool.
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

    fn name(self) -> &'static str {
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

/// Upper bound on a bars pattern's copied bars. A wider source range aggregates into this many
/// OHLC buckets, so the serialized pattern stays far inside the `tool_options` size bound.
pub const MAX_BARS_PATTERN_BARS: usize = 128;
/// Icon size bounds and default, in CSS px.
const ICON_SIZE_MIN: f64 = 8.0;
const ICON_SIZE_MAX: f64 = 128.0;
const DEFAULT_ICON_SIZE: f64 = 24.0;

/// Projection & Annotations options (`tool_options.projection_annotation`); absent fields keep
/// their defaults.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProjectionAnnotationToolOptions {
    /// Bars pattern: how the copied bars paint.
    pub bars_mode: BarsPatternMode,
    /// Bars pattern: reverse the copied bars in time.
    pub mirrored: bool,
    /// Bars pattern: turn the copied bars upside down within the box between the two anchors.
    pub flipped: bool,
    /// Bars pattern: the copied `[open, high, low, close]` bars, oldest first. The engine
    /// captures them when the drawing is created; paste, sync, and persistence carry them, while
    /// named templates keep only the style. At most [`MAX_BARS_PATTERN_BARS`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bars: Vec<[f64; 4]>,
    /// Icon: the stamped icon.
    pub icon: DrawingIcon,
    /// Icon: the icon's size in CSS px (8..=128).
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

impl ProjectionAnnotationToolOptions {
    /// Bounded pattern length, finite in-range prices, and an icon size within 8..=128 CSS px.
    pub fn validate(&self) -> bool {
        self.bars.len() <= MAX_BARS_PATTERN_BARS
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
/// Note pin: head radius, head center height above the tip, and inner dot radius (CSS px).
const NOTE_HEAD_RADIUS: f64 = 7.0;
const NOTE_RISE: f64 = 17.0;
const NOTE_DOT_RADIUS: f64 = 2.5;
/// Speech-bubble tail height and width (CSS px).
const TAIL_HEIGHT: f64 = 10.0;
const TAIL_WIDTH: f64 = 10.0;
/// Callout pointer half width at the box (CSS px).
const POINTER_HALF_WIDTH: f64 = 7.0;
/// Signpost pole height (CSS px).
const SIGNPOST_POLE: f64 = 40.0;
/// Flag mark: pole height and width, flag width and height (CSS px).
const FLAG_POLE: f64 = 24.0;
const FLAG_POLE_WIDTH: f64 = 2.0;
const FLAG_WIDTH: f64 = 16.0;
const FLAG_HEIGHT: f64 = 11.0;
/// Arrow marks: head length and half width, shaft half width, and total length (CSS px).
const ARROW_HEAD_LENGTH: f64 = 10.0;
const ARROW_HEAD_HALF: f64 = 9.0;
const ARROW_SHAFT_HALF: f64 = 3.5;
const ARROW_LENGTH: f64 = 22.0;
/// Icon stroke width as a fraction of the icon size (check and cross).
const ICON_STROKE: f64 = 0.14;
/// Heart outline samples.
const HEART_SAMPLES: usize = 48;

/// Shared placement and editing behavior; every spec below overrides its identity.
const TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Forecast,
    wire_id: 128,
    name: "forecast",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    // Placement opens the editor only for the tools that start from a default text the user
    // will replace or extend (see the overrides below); double-click, Enter, or F2 on a selected
    // text box edits it in place through the label its parts mark with
    // `DrawingParts::text_label`.
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
};

/// One-anchor annotation behavior.
const MARK: DrawingToolSpec = DrawingToolSpec {
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    ..TOOL
};

pub(crate) const FORECAST: DrawingToolSpec = DrawingToolSpec {
    straighten: DrawingStraightenMode::Segment45,
    default_width: 2.0,
    ..TOOL
};

// The ghost fills the box between its anchors (see `Ghost`), so it culls like any finite
// two-anchor drawing.
pub(crate) const BARS_PATTERN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::BarsPattern,
    wire_id: 129,
    name: "bars_pattern",
    ..TOOL
};

pub(crate) const PRICE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceRange,
    wire_id: 130,
    name: "price_range",
    ..TOOL
};

pub(crate) const DATE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DateRange,
    wire_id: 131,
    name: "date_range",
    ..TOOL
};

pub(crate) const DATE_AND_PRICE_RANGE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DateAndPriceRange,
    wire_id: 132,
    name: "date_and_price_range",
    ..TOOL
};

// The sector is a circle in screen px around the apex, which no anchor box bounds; it culls by
// its paint box (see `paint_bounds`).
pub(crate) const PROJECTION: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Projection,
    wire_id: 133,
    name: "projection",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..TOOL
};

// Pane-anchored: its anchor is a pane fraction, so time/price bounds never apply.
pub(crate) const ANCHORED_TEXT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::AnchoredText,
    wire_id: 134,
    name: "anchored_text",
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    requests_text_editor: true,
    ..MARK
};

pub(crate) const NOTE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Note,
    wire_id: 135,
    name: "note",
    requests_text_editor: true,
    ..MARK
};

pub(crate) const PRICE_NOTE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceNote,
    wire_id: 136,
    name: "price_note",
    straighten: DrawingStraightenMode::Segment45,
    ..TOOL
};

pub(crate) const CALLOUT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Callout,
    wire_id: 137,
    name: "callout",
    requests_text_editor: true,
    ..TOOL
};

pub(crate) const COMMENT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Comment,
    wire_id: 138,
    name: "comment",
    requests_text_editor: true,
    ..MARK
};

pub(crate) const PRICE_LABEL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceLabel,
    wire_id: 139,
    name: "price_label",
    ..MARK
};

pub(crate) const SIGNPOST: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Signpost,
    wire_id: 140,
    name: "signpost",
    requests_text_editor: true,
    ..MARK
};

pub(crate) const FLAG_MARK: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::FlagMark,
    wire_id: 141,
    name: "flag_mark",
    ..MARK
};

pub(crate) const ARROW_MARK_UP: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ArrowMarkUp,
    wire_id: 142,
    name: "arrow_mark_up",
    ..MARK
};

pub(crate) const ARROW_MARK_DOWN: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ArrowMarkDown,
    wire_id: 143,
    name: "arrow_mark_down",
    ..MARK
};

pub(crate) const ARROW_MARK_LEFT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ArrowMarkLeft,
    wire_id: 144,
    name: "arrow_mark_left",
    ..MARK
};

pub(crate) const ARROW_MARK_RIGHT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ArrowMarkRight,
    wire_id: 145,
    name: "arrow_mark_right",
    ..MARK
};

pub(crate) const ICON: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Icon,
    wire_id: 146,
    name: "icon",
    ..MARK
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.owns_labels = true;
    family.paint_bounds = paint_bounds;
    family.on_create = on_create;
    family.owns_text = true;
    // A forecast's outcome follows its source series (a bars pattern keeps its own copy).
    family.reads_series_data = |drawing| drawing.kind == DrawingKind::Forecast;
    family.pane_anchored = |kind| kind == DrawingKind::AnchoredText;
    // A note shows its text only while focused, unless it always shows it.
    family.reveals_on_focus =
        |drawing| drawing.kind == DrawingKind::Note && !options(drawing).always_show_text;
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
        DrawingKind::DateAndPriceRange => {
            drawing.fill_enabled = true;
            drawing.stroke_end = DrawingLineCap::Arrow;
            drawing.labels = stats(&[PriceChange, PercentChange, Ticks, BarCount, Duration]);
        }
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
        DrawingKind::ArrowMarkUp => {
            drawing.color = aeris_charts_core::style::MARKET_UP_CSS.to_string();
        }
        DrawingKind::ArrowMarkDown => {
            drawing.color = aeris_charts_core::style::MARKET_DOWN_CSS.to_string();
        }
        _ => {}
    }
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
        DrawingKind::Forecast => forecast(ctx, parts),
        DrawingKind::BarsPattern => bars_pattern(ctx, parts),
        DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DateAndPriceRange => {
            range(ctx, parts)
        }
        DrawingKind::Projection => projection(ctx, parts),
        DrawingKind::AnchoredText => anchored_text(ctx, parts),
        DrawingKind::Note => note(ctx, parts),
        DrawingKind::PriceNote => price_note(ctx, parts),
        DrawingKind::Callout => callout(ctx, parts),
        DrawingKind::Comment => bubble(ctx, parts, Vec::new()),
        DrawingKind::PriceLabel => {
            let Some(point) = ctx.drawing.points.first() else {
                return;
            };
            let price = ctx.engine.drawing_price_text(ctx.drawing, point.price);
            bubble(ctx, parts, vec![price]);
        }
        DrawingKind::Signpost => signpost(ctx, parts),
        DrawingKind::FlagMark => flag_mark(ctx, parts),
        DrawingKind::ArrowMarkUp => arrow_mark(ctx, parts, (0.0, -1.0)),
        DrawingKind::ArrowMarkDown => arrow_mark(ctx, parts, (0.0, 1.0)),
        DrawingKind::ArrowMarkLeft => arrow_mark(ctx, parts, (-1.0, 0.0)),
        DrawingKind::ArrowMarkRight => arrow_mark(ctx, parts, (1.0, 0.0)),
        DrawingKind::Icon => icon(ctx, parts),
        _ => {}
    }
}

// --- shared styling ---------------------------------------------------------------------------

fn with_alpha(color: Color, alpha: u8) -> Color {
    Color::rgba(color.r(), color.g(), color.b(), alpha)
}

fn market_color(css: &str, fallback: (u8, u8, u8)) -> Color {
    Color::parse_css(css).unwrap_or(Color::rgb(fallback.0, fallback.1, fallback.2))
}

/// Text on a box: `text_color`, else black or white against the box.
fn box_text_color(drawing: &Drawing, background: Color) -> Color {
    drawing
        .text_color
        .as_deref()
        .and_then(Color::parse_css)
        .unwrap_or_else(|| background.solid().contrast_text())
}

/// `prefix` lines of engine text (a formatted price) followed by the drawing's own text lines
/// ([`PartContext::text_lines`]), and the index of the first text line.
fn with_text(ctx: &PartContext<'_>, mut prefix: Vec<String>) -> (Vec<String>, usize) {
    let first_line = prefix.len();
    prefix.extend(ctx.text_lines());
    (prefix, first_line)
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

/// Resolved box of `label` in caller px.
fn label_rect(ctx: &PartContext<'_>, label: &PartLabel) -> shape::Rect {
    let family = &ctx.engine.options.get().layout.font_family;
    label
        .layout(|line| {
            ctx.engine
                .measure_text_run(line, label.size, family, label.weight, label.italic)
        })
        .rect
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

// --- projection and measuring ----------------------------------------------------------------

/// Forecast: the source → target segment, a source dot and price box on the far side from the
/// target, and a target box with the change, the target time, and the evaluated outcome.
fn forecast(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b), Some(source)) =
        (ctx.px.first(), ctx.px.get(1), drawing.points.first())
    else {
        return;
    };
    parts.capped_segment(drawing, a, b, (true, true), ctx.scale);
    parts.disc(a, cap_radius(drawing.width) * ctx.scale, None);
    let gap = LABEL_GAP * ctx.scale;
    let forward = b.0 >= a.0;
    let base = drawing.stroke_color();
    let source_text = ctx.engine.drawing_price_text(drawing, source.price);
    parts.label(stats_box(
        ctx,
        (if forward { a.0 - gap } else { a.0 + gap }, a.1),
        (
            if forward {
                DrawingTextHAlign::Right
            } else {
                DrawingTextHAlign::Left
            },
            DrawingTextVAlign::Middle,
        ),
        vec![source_text],
        with_alpha(base, STATS_ALPHA),
    ));
    let status = forecast_status(ctx.engine, drawing);
    let background = match status {
        Some(true) => market_color(aeris_charts_core::style::MARKET_UP_CSS, (8, 153, 129)),
        Some(false) => market_color(aeris_charts_core::style::MARKET_DOWN_CSS, (247, 82, 95)),
        None => base,
    };
    parts.label(stats_box(
        ctx,
        (if forward { b.0 + gap } else { b.0 - gap }, b.1),
        (
            if forward {
                DrawingTextHAlign::Left
            } else {
                DrawingTextHAlign::Right
            },
            DrawingTextVAlign::Middle,
        ),
        forecast_target_lines(ctx.engine, drawing, status),
        with_alpha(background, STATS_ALPHA),
    ));
}

/// Target box lines: the change and percent from the source, the target time (when the axis has
/// time identity), and `Success`/`Failure` once evaluated.
fn forecast_target_lines(
    engine: &ChartEngine,
    drawing: &Drawing,
    status: Option<bool>,
) -> Vec<String> {
    let (Some(source), Some(target)) = (drawing.points.first(), drawing.points.get(1)) else {
        return Vec::new();
    };
    let change = target.price - source.price;
    let sign = if change > 0.0 { "+" } else { "" };
    let mut first = format!("{sign}{}", engine.drawing_price_text(drawing, change));
    if source.price.abs() > f64::EPSILON {
        first.push_str(&format!(" ({:+.2}%)", change / source.price.abs() * 100.0));
    }
    let mut lines = vec![first];
    if let Some(time) = engine.drawing_anchor_time_of(drawing, 1) {
        lines.push(engine.format_crosshair_ts(time.round() as i64));
    }
    match status {
        Some(true) => lines.push("Success".to_string()),
        Some(false) => lines.push("Failure".to_string()),
        None => {}
    }
    lines
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
fn forecast_status(engine: &ChartEngine, drawing: &Drawing) -> Option<bool> {
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

/// The bars between the anchors' rounded indexes on the source series as `[open, high, low,
/// close]`, oldest first, aggregated into at most [`MAX_BARS_PATTERN_BARS`] OHLC buckets, plus
/// the first and last copied bar indexes. `None` without source bars in the range. The bars are
/// the source's own (canonical) rows, each once. On a union source each bucket reads only its LOD
/// summary rows (first, last, and extrema), so a capture — and the placement preview that
/// repeats it every frame — costs a logarithmic number of summaries per bucket, not a scan of the
/// range; an as-of source has no canonical-row pyramid, so its buckets scan their rows, bounded by
/// the capture window.
fn capture_pattern(engine: &ChartEngine, drawing: &Drawing) -> Option<(Vec<[f64; 4]>, i64, i64)> {
    let (a, b) = (drawing.points.first()?, drawing.points.get(1)?);
    let from = bar_index(a.logical.min(b.logical).round())?;
    let to = bar_index(a.logical.max(b.logical).round())?;
    let rows = engine.drawing_source_window(drawing)?;
    let (first_row, last_row) = if rows.is_as_of() {
        let window = rows.rows_between(from, to);
        (
            window.clone().find(|&row| !rows.is_whitespace(row))?,
            window.rev().find(|&row| !rows.is_whitespace(row))?,
        )
    } else {
        (
            rows.plot.first_non_whitespace_row(from)?,
            rows.plot.last_non_whitespace_row(to)?,
        )
    };
    if first_row > last_row {
        return None;
    }
    let count = last_row - first_row + 1;
    let buckets = count.min(MAX_BARS_PATTERN_BARS);
    let lod = if rows.is_as_of() {
        None
    } else {
        rows.plot.lod()
    };
    let mut bars = Vec::with_capacity(buckets);
    for bucket in 0..buckets {
        let start = first_row + bucket * count / buckets;
        let end = first_row + (bucket + 1) * count / buckets;
        let mut merged: Option<[f64; 4]> = None;
        // Rows arrive oldest first: the first keeps its open, the last sets the close.
        let mut merge = |row: usize| {
            if rows.is_whitespace(row) {
                return;
            }
            let value = |field| rows.value(row, field);
            let bar = [
                value(PlotValueIndex::Open),
                value(PlotValueIndex::High),
                value(PlotValueIndex::Low),
                value(PlotValueIndex::Close),
            ];
            if !bar.iter().all(|value| value.is_finite()) {
                return;
            }
            merged = Some(match merged {
                None => bar,
                Some(first) => [first[0], first[1].max(bar[1]), first[2].min(bar[2]), bar[3]],
            });
        };
        match lod {
            Some(lod) => lod
                .rows_on_range(start..end, usize::MAX)
                .0
                .iter()
                .for_each(&mut merge),
            None => (start..end).for_each(&mut merge),
        }
        bars.extend(merged);
    }
    if bars.is_empty() {
        return None;
    }
    Some((bars, rows.index_of(first_row)?, rows.index_of(last_row)?))
}

/// Capture a new bars pattern's source bars once, pinning its anchors on the copy's box so the
/// ghost starts exactly over its source. A placement always copies its own range (a tool
/// template's bars are stale); `add_drawing` keeps bars its options carry (paste).
fn on_create(engine: &ChartEngine, drawing: &mut Drawing, placed: bool) {
    if drawing.kind != DrawingKind::BarsPattern
        || drawing.points.len() != 2
        || (!placed && !options(drawing).bars.is_empty())
    {
        return;
    }
    let Some((bars, first, last)) = capture_pattern(engine, drawing) else {
        return;
    };
    let mut block = options(drawing).clone();
    block.bars = bars;
    drawing.points = pinned_anchors(&block, first, last).to_vec();
    drawing.set_pending_times(Vec::new());
    drawing.tool_options.projection_annotation = Some(block);
}

/// The copy's price box: its highest and lowest value over every OHLC field.
fn copy_extent(options: &ProjectionAnnotationToolOptions) -> (f64, f64) {
    let values = options.bars.iter().flatten();
    let high = values.clone().copied().fold(f64::NEG_INFINITY, f64::max);
    let low = values.copied().fold(f64::INFINITY, f64::min);
    if high >= low {
        (high, low)
    } else {
        (0.0, 0.0)
    }
}

/// Anchors on the copy's box: the first copied bar's index at the copy's highest value and the
/// last copied bar's index at its lowest, where the ghost overlays its source exactly.
fn pinned_anchors(
    options: &ProjectionAnnotationToolOptions,
    first: i64,
    last: i64,
) -> [DrawingPoint; 2] {
    let (high, low) = copy_extent(options);
    [
        DrawingPoint {
            logical: first as f64,
            price: high,
        },
        DrawingPoint {
            logical: last as f64,
            price: low,
        },
    ]
}

/// The bars pattern's price and time mapping: bar `i` of `n` sits at the fraction `i / (n - 1)`
/// from the first anchor to the second, and the copy's box (highest to lowest value) fills the
/// anchors' price span, the highest value on the first anchor's price and the lowest on the
/// second's. The scale divides by the copy's full range, so a small anchor drag never blows the
/// ghost up; a flat copy keeps its offset from the first anchor. Mirroring reverses the bars in
/// time; flipping turns them upside down within the same box. The mapping is linear in the anchor
/// prices, so a price-basis rescale of the anchors rescales the ghost exactly, and the ghost never
/// leaves the anchors' price span.
pub(super) struct Ghost {
    a: DrawingPoint,
    b: DrawingPoint,
    count: usize,
    mirrored: bool,
    flipped: bool,
    high: f64,
    scale: f64,
}

impl Ghost {
    pub(super) fn new(
        options: &ProjectionAnnotationToolOptions,
        a: DrawingPoint,
        b: DrawingPoint,
    ) -> Self {
        let (high, low) = copy_extent(options);
        let range = high - low;
        let scale = if range > 1e-12 * high.abs().max(low.abs()).max(1.0) {
            (a.price - b.price) / range
        } else {
            1.0
        };
        Self {
            a,
            b,
            count: options.bars.len(),
            mirrored: options.mirrored,
            flipped: options.flipped,
            high,
            scale: if scale.is_finite() { scale } else { 1.0 },
        }
    }

    fn bar(&self, bars: &[[f64; 4]], index: usize) -> [f64; 4] {
        if self.mirrored {
            bars[self.count - 1 - index]
        } else {
            bars[index]
        }
    }

    fn fraction(&self, index: usize) -> f64 {
        if self.count > 1 {
            index as f64 / (self.count - 1) as f64
        } else {
            0.0
        }
    }

    fn logical(&self, index: usize) -> f64 {
        self.a.logical + self.fraction(index) * (self.b.logical - self.a.logical)
    }

    pub(super) fn price(&self, value: f64) -> f64 {
        let base = self.a.price + (value - self.high) * self.scale;
        if self.flipped {
            self.a.price + self.b.price - base
        } else {
            base
        }
    }
}

/// Bars pattern: the copied bars as crisp sticks or a line. A placement preview shows the bars the
/// commit will copy, over their source; a pattern that copied nothing (created before any data)
/// is a dashed box between its anchors.
fn bars_pattern(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b)) = (drawing.points.first(), drawing.points.get(1)) else {
        return;
    };
    let block = options(drawing);
    // The pending drawing (id 0) previews the copy its placement will capture, in place.
    if drawing.id == 0 {
        if let Some((bars, first, last)) = capture_pattern(ctx.engine, drawing) {
            let mut preview = block.clone();
            preview.bars = bars;
            let [a, b] = pinned_anchors(&preview, first, last);
            paint_pattern(ctx, parts, &preview, Ghost::new(&preview, a, b));
            return;
        }
    }
    if !block.bars.is_empty() {
        paint_pattern(ctx, parts, block, Ghost::new(block, a, b));
        return;
    }
    let (Some(&pa), Some(&pb)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    parts.stroke(
        &[pa, (pb.0, pa.1), pb, (pa.0, pb.1), pa],
        PartStroke::decoration(1.0, LineStyle::Dashed),
        false,
    );
}

fn paint_pattern(
    ctx: &PartContext<'_>,
    parts: &mut DrawingParts,
    block: &ProjectionAnnotationToolOptions,
    ghost: Ghost,
) {
    let bars = &block.bars;
    let (Some(start), Some(end)) = (ctx.point_px(ghost.a), ctx.point_px(ghost.b)) else {
        return;
    };
    let count = ghost.count;
    // x is linear in the logical index, so the copy's columns follow the anchors' px and only
    // the columns inside the pane convert prices.
    let x_at = |index: usize| start.0 + ghost.fraction(index) * (end.0 - start.0);
    let spacing = if count > 1 {
        (end.0 - start.0).abs() / (count - 1) as f64
    } else {
        0.0
    };
    let margin = spacing + ctx.drawing.width * ctx.scale;
    let visible = |index: usize| {
        let x = x_at(index);
        x >= ctx.pane.left - margin && x <= ctx.pane.right + margin
    };
    let y_at = |index: usize, value: f64| {
        ctx.point_px(DrawingPoint {
            logical: ghost.logical(index),
            price: ghost.price(value),
        })
        .map(|point| point.1)
    };
    match block.bars_mode.line_field() {
        None => {
            let oc = block.bars_mode == BarsPatternMode::OcBars;
            for index in (0..count).filter(|&index| visible(index)) {
                let bar = ghost.bar(bars, index);
                let (from, to) = if oc {
                    (bar[0], bar[3])
                } else {
                    (bar[1], bar[2])
                };
                let (Some(y0), Some(y1)) = (y_at(index, from), y_at(index, to)) else {
                    continue;
                };
                let (mut top, mut bottom) = (y0.min(y1), y0.max(y1));
                // A doji still paints one device pixel.
                if bottom - top < ctx.scale {
                    let middle = (top + bottom) / 2.0;
                    top = middle - ctx.scale / 2.0;
                    bottom = middle + ctx.scale / 2.0;
                }
                parts.vline(x_at(index), top, bottom, PartStroke::default());
            }
        }
        Some(field) => {
            let Some(first) = (0..count).position(visible) else {
                return;
            };
            let last = (0..count).rposition(visible).unwrap_or(first);
            let mut line = Vec::with_capacity(last - first + 1);
            for index in first..=last {
                let value = ghost.bar(bars, index)[field];
                if let Some(y) = y_at(index, value) {
                    line.push((x_at(index), y));
                }
            }
            parts.stroke(&line, PartStroke::default(), false);
        }
    }
}

/// Price, date, and date-and-price ranges: the fill between the anchors, the edge lines of the
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
    let price = matches!(
        drawing.kind,
        DrawingKind::PriceRange | DrawingKind::DateAndPriceRange
    );
    let time = matches!(
        drawing.kind,
        DrawingKind::DateRange | DrawingKind::DateAndPriceRange
    );
    if drawing.kind == DrawingKind::PriceRange {
        parts.hline(a.1, left, right, PartStroke::default());
        parts.hline(b.1, left, right, PartStroke::default());
    }
    if drawing.kind == DrawingKind::DateRange {
        parts.vline(a.0, top, bottom, PartStroke::default());
        parts.vline(b.0, top, bottom, PartStroke::default());
    }
    if price {
        parts.capped_segment(
            drawing,
            (middle_x, a.1),
            (middle_x, b.1),
            (true, true),
            ctx.scale,
        );
    }
    if time {
        parts.capped_segment(
            drawing,
            (a.0, middle_y),
            (b.0, middle_y),
            (true, true),
            ctx.scale,
        );
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

/// Projection: the circular sector around the apex from the ray through the radius point to
/// the ray through the price point (the shorter turn), filled and outlined, plus any visible
/// stats measured from the apex to the price point.
fn projection(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&apex), Some(&radius_point), Some(&price_point)) =
        (ctx.px.first(), ctx.px.get(1), ctx.px.get(2))
    else {
        return;
    };
    let radius = (radius_point.0 - apex.0).hypot(radius_point.1 - apex.1);
    if radius > f64::EPSILON {
        let start = (radius_point.1 - apex.1).atan2(radius_point.0 - apex.0);
        let end = (price_point.1 - apex.1).atan2(price_point.0 - apex.0);
        let sweep = (end - start + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        let mut outline = vec![apex];
        shape::EllipseArc::circle(apex, radius, start, sweep)
            .append_points(CURVE_TOLERANCE, &mut outline);
        if drawing.fill_enabled {
            // A sector of at most a half turn is convex.
            parts.fill_convex(&outline, Some(drawing.fill_or_wash(FILL_ALPHA)), true);
        }
        outline.push(apex);
        parts.stroke(&outline, PartStroke::default(), false);
    } else {
        parts.stroke(&[apex, price_point], PartStroke::default(), false);
    }
    if !drawing.labels.iter().any(|label| label.visible) {
        return;
    }
    let lines = ctx.engine.drawing_stat_lines(drawing, 0, 2);
    let gap = LABEL_GAP * ctx.scale;
    let right = price_point.0 >= apex.0;
    parts.label(stats_box(
        ctx,
        (
            if right {
                price_point.0 + gap
            } else {
                price_point.0 - gap
            },
            price_point.1,
        ),
        (
            if right {
                DrawingTextHAlign::Left
            } else {
                DrawingTextHAlign::Right
            },
            DrawingTextVAlign::Middle,
        ),
        lines,
        with_alpha(drawing.stroke_color(), STATS_ALPHA),
    ));
}

// --- annotations -------------------------------------------------------------------------------

/// Anchored text: the text with its aligned box edge on the pane-anchored point, boxed like the
/// text tool when `box_color`/`box_border_color` are set.
fn anchored_text(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(&anchor) = ctx.px.first() else {
        return;
    };
    let background = drawing.box_color.as_deref().and_then(Color::parse_css);
    // Without a box the text follows `text_color` or the chart text color, like the text tool.
    let mut label = text_box(
        ctx,
        anchor,
        (drawing.text_h_align, drawing.text_v_align),
        ctx.text_lines(),
        background,
    );
    let pad = super::super::TEXT_PAD * ctx.scale;
    label.padding = (pad, pad);
    parts.text_label(label, 0);
}

/// Note: a teardrop pin whose tip is the anchor, with a contrasting dot in its head and, while
/// the note is hovered, selected, or edited (or with `always_show_text`), the text in a box
/// beside the head.
fn note(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(&tip) = ctx.px.first() else {
        return;
    };
    let (radius, rise) = (NOTE_HEAD_RADIUS * ctx.scale, NOTE_RISE * ctx.scale);
    let head = (tip.0, tip.1 - rise);
    // The tangent points from the tip sit slightly below the head's center; the outline runs
    // from the right one over the top to the left one.
    let tangent_y = radius * radius / rise;
    let tangent_x = (radius * radius - tangent_y * tangent_y).max(0.0).sqrt();
    let start = tangent_y.atan2(tangent_x);
    let mut outline = vec![tip];
    shape::EllipseArc::circle(head, radius, start, -(std::f64::consts::PI + 2.0 * start))
        .append_points(CURVE_TOLERANCE, &mut outline);
    parts.fill_convex(&outline, None, true);
    let base = drawing.stroke_color();
    parts.disc(
        head,
        NOTE_DOT_RADIUS * ctx.scale,
        Some(base.solid().contrast_text()),
    );
    if !(ctx.text_editing || ctx.focused() || options(drawing).always_show_text) {
        return;
    }
    parts.text_label(
        text_box(
            ctx,
            (head.0 + radius + LABEL_GAP * ctx.scale / 2.0, head.1),
            (DrawingTextHAlign::Left, DrawingTextVAlign::Middle),
            ctx.text_lines(),
            Some(base),
        ),
        0,
    );
}

/// Price note: a leader from the priced point (with a dot) to a box holding its price and any
/// text, on the leader's far side.
fn price_note(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&a), Some(&b), Some(point)) = (ctx.px.first(), ctx.px.get(1), drawing.points.first())
    else {
        return;
    };
    parts.stroke(&[a, b], PartStroke::default(), false);
    parts.disc(a, cap_radius(drawing.width) * ctx.scale, None);
    let price = ctx.engine.drawing_price_text(drawing, point.price);
    let (lines, first_line) = with_text(ctx, vec![price]);
    parts.text_label(
        text_box(
            ctx,
            b,
            (
                if b.0 >= a.0 {
                    DrawingTextHAlign::Left
                } else {
                    DrawingTextHAlign::Right
                },
                DrawingTextVAlign::Middle,
            ),
            lines,
            Some(drawing.stroke_color()),
        ),
        first_line,
    );
}

/// Callout: the text box placed on the second anchor by the text alignment, with a pointer from
/// the box to the first anchor.
fn callout(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let (Some(&tip), Some(&place)) = (ctx.px.first(), ctx.px.get(1)) else {
        return;
    };
    let background = drawing.stroke_color();
    let label = text_box(
        ctx,
        place,
        (drawing.text_h_align, drawing.text_v_align),
        ctx.text_lines(),
        Some(background),
    );
    if !label.lines.is_empty() {
        let rect = label_rect(ctx, &label);
        if !rect.contains(tip) {
            let center = (
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0,
            );
            let (dx, dy) = (tip.0 - center.0, tip.1 - center.1);
            let length = dx.hypot(dy);
            let half = (POINTER_HALF_WIDTH * ctx.scale)
                .min((rect.right - rect.left).min(rect.bottom - rect.top) / 2.0);
            if length > f64::EPSILON {
                let (nx, ny) = (-dy / length * half, dx / length * half);
                // Based at the box center, so the box paints over the pointer's inner part.
                parts.fill_convex(
                    &[
                        tip,
                        (center.0 + nx, center.1 + ny),
                        (center.0 - nx, center.1 - ny),
                    ],
                    Some(background),
                    true,
                );
            }
        }
    }
    parts.text_label(label, 0);
}

/// Comment and price label: a speech bubble whose tail tip is the anchor, the box above and to
/// the right of it, holding `prefix` (the price label's price) and the text.
fn bubble(ctx: &PartContext<'_>, parts: &mut DrawingParts, prefix: Vec<String>) {
    let Some(&tip) = ctx.px.first() else {
        return;
    };
    let background = ctx.drawing.stroke_color();
    let base_y = tip.1 - TAIL_HEIGHT * ctx.scale;
    // The tail reaches one px into the box so the two never show a seam.
    let overlap = ctx.scale;
    parts.fill_convex(
        &[
            tip,
            (tip.0, base_y - overlap),
            (tip.0 + TAIL_WIDTH * ctx.scale, base_y - overlap),
        ],
        Some(background),
        true,
    );
    let (lines, first_line) = with_text(ctx, prefix);
    parts.text_label(
        text_box(
            ctx,
            (tip.0, base_y),
            (DrawingTextHAlign::Left, DrawingTextVAlign::Bottom),
            lines,
            Some(background),
        ),
        first_line,
    );
}

/// Signpost: a crisp pole from the anchor up to a text plate centered on its top.
fn signpost(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let Some(&foot) = ctx.px.first() else {
        return;
    };
    let top = foot.1 - SIGNPOST_POLE * ctx.scale;
    parts.vline(foot.0, top, foot.1, PartStroke::default());
    parts.text_label(
        text_box(
            ctx,
            (foot.0, top),
            (DrawingTextHAlign::Center, DrawingTextVAlign::Bottom),
            ctx.text_lines(),
            Some(ctx.drawing.stroke_color()),
        ),
        0,
    );
}

/// Flag mark: a pole standing on the anchor with a flag at its top right.
fn flag_mark(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let Some(&foot) = ctx.px.first() else {
        return;
    };
    let s = ctx.scale;
    let top = foot.1 - FLAG_POLE * s;
    parts.stroke(
        &[foot, (foot.0, top)],
        PartStroke::decoration(FLAG_POLE_WIDTH, LineStyle::Solid),
        false,
    );
    let left = foot.0 + FLAG_POLE_WIDTH * s / 2.0;
    parts.fill_convex(
        &[
            (left, top),
            (left + FLAG_WIDTH * s, top),
            (left + FLAG_WIDTH * s, top + FLAG_HEIGHT * s),
            (left, top + FLAG_HEIGHT * s),
        ],
        None,
        true,
    );
}

/// Arrow mark: a block arrow whose tip is the anchor, pointing along `direction`, painted as one
/// outline (paired chains down both sides) so no seam splits head and shaft; any text sits past
/// the tail in `text_color` or the arrow color.
fn arrow_mark(ctx: &PartContext<'_>, parts: &mut DrawingParts, direction: Point) {
    let drawing = ctx.drawing;
    let Some(&tip) = ctx.px.first() else {
        return;
    };
    let s = ctx.scale;
    let side = (-direction.1, direction.0);
    let at = |back: f64, across: f64| {
        (
            tip.0 - direction.0 * back * s + side.0 * across * s,
            tip.1 - direction.1 * back * s + side.1 * across * s,
        )
    };
    let chain = |sign: f64| {
        [
            at(0.0, 0.0),
            at(ARROW_HEAD_LENGTH, sign * ARROW_HEAD_HALF),
            at(ARROW_HEAD_LENGTH, sign * ARROW_SHAFT_HALF),
            at(ARROW_LENGTH, sign * ARROW_SHAFT_HALF),
        ]
    };
    parts.fill(&chain(-1.0), &chain(1.0), None, true);
    let lines = ctx.text_lines();
    if lines.is_empty() {
        return;
    }
    let anchor = at(ARROW_LENGTH + LABEL_GAP / 2.0, 0.0);
    let alignment = match direction {
        (_, y) if y < 0.0 => (DrawingTextHAlign::Center, DrawingTextVAlign::Top),
        (_, y) if y > 0.0 => (DrawingTextHAlign::Center, DrawingTextVAlign::Bottom),
        (x, _) if x < 0.0 => (DrawingTextHAlign::Left, DrawingTextVAlign::Middle),
        _ => (DrawingTextHAlign::Right, DrawingTextVAlign::Middle),
    };
    let mut label = text_box(ctx, anchor, alignment, lines, None);
    label.color = Some(
        drawing
            .text_color
            .as_deref()
            .and_then(Color::parse_css)
            .unwrap_or_else(|| drawing.stroke_color()),
    );
    label.padding = (0.0, 0.0);
    parts.text_label(label, 0);
}

/// Icon: the selected built-in icon centered on the anchor, `icon_size` CSS px across, in the
/// drawing color. Filled icons are one convex polygon, a disc, or a fan around their center
/// (star, heart: star-shaped outlines), so paint and hit test cover exactly the icon.
fn icon(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let Some(&center) = ctx.px.first() else {
        return;
    };
    let block = options(ctx.drawing);
    let size = block.icon_size * ctx.scale;
    let at = |x: f64, y: f64| (center.0 + x * size, center.1 + y * size);
    let stroke = PartStroke::decoration(block.icon_size * ICON_STROKE, LineStyle::Solid);
    match block.icon {
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

// --- culling and schema --------------------------------------------------------------------------

/// Media-px box of the projection's sector: its anchors and the square around the apex reaching
/// the radius point. Its stats box pads through `decoration_extent`. Every other tool keeps its
/// anchors' box (or the pane, for pane-anchored text).
fn paint_bounds(_: &ChartEngine, drawing: &Drawing, px: &[Point]) -> Option<shape::Rect> {
    if drawing.kind != DrawingKind::Projection {
        return None;
    }
    let (&apex, &rim) = (px.first()?, px.get(1)?);
    let radius = (rim.0 - apex.0).hypot(rim.1 - apex.1);
    if !radius.is_finite() {
        return None;
    }
    let mut points = px.to_vec();
    points.push((apex.0 - radius, apex.1 - radius));
    points.push((apex.0 + radius, apex.1 + radius));
    shape::Rect::bounding(&points)
}

/// Conservative reach of the family's boxes and markers beyond the anchors, in CSS px. Text that
/// follows data (bar counts, durations, forecast outcomes) gets four ems of slack so the cached
/// pad stays valid between text-key refreshes.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let size = engine.drawing_text_size(drawing);
    let stats = engine.drawing_stats_size();
    let lines = culling_text_lines(drawing);
    let price_line = |index: usize| {
        drawing
            .points
            .get(index)
            .map(|point| engine.drawing_price_text(drawing, point.price))
            .unwrap_or_default()
    };
    match drawing.kind {
        DrawingKind::Forecast => {
            let mut target = forecast_target_lines(engine, drawing, None);
            target.push("Failure".to_string());
            let source = box_reach(engine, drawing, &[price_line(0)], stats, STATS_PADDING, 0.0);
            let target = box_reach(engine, drawing, &target, stats, STATS_PADDING, 0.0);
            LABEL_GAP + source.max(target) + 4.0 * stats
        }
        DrawingKind::PriceRange | DrawingKind::DateRange | DrawingKind::DateAndPriceRange => {
            let lines = engine.drawing_stat_lines(drawing, 0, 1);
            box_reach(engine, drawing, &lines, stats, STATS_PADDING, LABEL_GAP) + 4.0 * stats
        }
        // The stats box beside the price point, beyond the sector's paint box.
        DrawingKind::Projection if drawing.labels.iter().any(|label| label.visible) => {
            let lines = engine.drawing_stat_lines(drawing, 0, 2);
            box_reach(engine, drawing, &lines, stats, STATS_PADDING, LABEL_GAP) + 4.0 * stats
        }
        // Pane-anchored (the pane bounds it), or the anchors' box (the ghost).
        DrawingKind::Projection | DrawingKind::AnchoredText | DrawingKind::BarsPattern => 0.0,
        DrawingKind::Note => {
            NOTE_RISE
                + NOTE_HEAD_RADIUS
                + box_reach(engine, drawing, &lines, size, BOX_PADDING, LABEL_GAP)
        }
        DrawingKind::PriceNote => {
            let mut all = vec![price_line(0)];
            all.extend(lines);
            box_reach(engine, drawing, &all, size, BOX_PADDING, 0.0) + 4.0 * size
        }
        DrawingKind::Callout => {
            // The box is placed on the second anchor by the 3×3 alignment.
            2.0 * box_reach(engine, drawing, &lines, size, BOX_PADDING, 0.0)
        }
        DrawingKind::Comment => {
            TAIL_HEIGHT + TAIL_WIDTH + box_reach(engine, drawing, &lines, size, BOX_PADDING, 0.0)
        }
        DrawingKind::PriceLabel => {
            let mut all = vec![price_line(0)];
            all.extend(lines);
            TAIL_HEIGHT
                + TAIL_WIDTH
                + box_reach(engine, drawing, &all, size, BOX_PADDING, 0.0)
                + 4.0 * size
        }
        DrawingKind::Signpost => {
            SIGNPOST_POLE + box_reach(engine, drawing, &lines, size, BOX_PADDING, 0.0)
        }
        DrawingKind::FlagMark => FLAG_POLE + FLAG_WIDTH,
        DrawingKind::ArrowMarkUp
        | DrawingKind::ArrowMarkDown
        | DrawingKind::ArrowMarkLeft
        | DrawingKind::ArrowMarkRight => {
            ARROW_LENGTH
                + ARROW_HEAD_HALF
                + box_reach(engine, drawing, &lines, size, (0.0, 0.0), LABEL_GAP)
        }
        DrawingKind::Icon => options(drawing).icon_size,
        _ => 0.0,
    }
}

fn descriptor(
    name: &str,
    property_type: DrawingPropertyType,
    default: serde_json::Value,
) -> DrawingPropertyDescriptor {
    crate::drawing_contract::descriptor(
        format!("tool_options.projection_annotation.{name}"),
        property_type,
        default,
    )
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let defaults = &DEFAULT_OPTIONS;
    match template.kind {
        DrawingKind::BarsPattern => {
            let mut mode = descriptor(
                "bars_mode",
                DrawingPropertyType::Enum,
                serde_json::json!(defaults.bars_mode.name()),
            );
            mode.enum_values = BarsPatternMode::ALL
                .iter()
                .map(|mode| mode.name().to_string())
                .collect();
            properties.push(mode);
            properties.push(descriptor(
                "mirrored",
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.mirrored),
            ));
            properties.push(descriptor(
                "flipped",
                DrawingPropertyType::Boolean,
                serde_json::json!(defaults.flipped),
            ));
        }
        DrawingKind::Icon => {
            let mut icon = descriptor(
                "icon",
                DrawingPropertyType::Enum,
                serde_json::json!(defaults.icon.name()),
            );
            icon.enum_values = DrawingIcon::ALL
                .iter()
                .map(|icon| icon.name().to_string())
                .collect();
            properties.push(icon);
            let mut size = descriptor(
                "icon_size",
                DrawingPropertyType::Number,
                serde_json::json!(defaults.icon_size),
            );
            size.min = Some(ICON_SIZE_MIN);
            size.max = Some(ICON_SIZE_MAX);
            properties.push(size);
        }
        DrawingKind::Note => properties.push(descriptor(
            "always_show_text",
            DrawingPropertyType::Boolean,
            serde_json::json!(defaults.always_show_text),
        )),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
