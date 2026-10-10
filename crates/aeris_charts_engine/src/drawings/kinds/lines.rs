//! B8 Lines family, own-line tools (wire ids 240..=243): the KLineChart horizontal segment,
//! vertical ray, and vertical segment (axis-locked segments) and the price line. The upstream
//! catalog's line tools (ray, extended line, info line, trend angle, cross line, arrow line) are
//! upstream-rendered and carry no family hooks; [`legacy_defaults`] keeps the fork's pre-merge
//! defaults of those tools for documents the fork wrote, and the stored `tool_options.line` block
//! layers the fork's presentation on their upstream arms ([`fork_presentation`]): the stats box,
//! the trend angle's reference, arc and angle, and the segment tools' caps on their unextended
//! ends, trimmed under arrowheads.
//!
//! The axis-locked segments share one geometry: the first two anchors, extended beyond the first
//! anchor by `extend_left` and beyond the second by `extend_right` to the pane edge (the vertical
//! ray extends to the right by default), end caps on the ends that are not extended, and the
//! segment-layout text label. The price line is a ray to the right from its anchor with its price
//! printed above the line and tagged on the price axis. Any visible `labels` render as one stats
//! box (see [`ChartEngine::drawing_stat_lines`]).

use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::geometry::{CURVE_TOLERANCE, DrawingGeometryOptions, segment_extension};
use super::super::parts::{
    DrawingParts, PartContext, PartLabel, PartStroke, STATS_GAP, STATS_PADDING, cap_radius, text_on,
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
    DrawingLabelPosition, DrawingLineCap, DrawingPropertyDescriptor, DrawingPropertyType,
};

/// Where the stats box sits along the anchor segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingStatsPosition {
    /// Beyond the first anchor, away from the second.
    Start,
    /// Below the segment's midpoint.
    Middle,
    /// Beyond the second anchor, away from the first.
    #[default]
    End,
}

impl DrawingStatsPosition {
    fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// Lines-family options (`tool_options.line`); absent fields keep their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LineToolOptions {
    pub stats_position: DrawingStatsPosition,
}

/// Trend-angle arc radius bounds in CSS px (a third of the segment between them).
const ANGLE_ARC_MIN: f64 = 16.0;
const ANGLE_ARC_MAX: f64 = 48.0;
/// Gap between the arc and the angle text in CSS px.
const ANGLE_LABEL_GAP: f64 = 6.0;
/// Width of the trend angle's reference line and arc in CSS px.
const ANGLE_DECORATION_WIDTH: f64 = 1.0;
/// Upstream's trend angle (no `line` block): its dotted reference's and arc's radius and the gap
/// before its angle label, in CSS px (`ChartEngine::build_trend_angle_prims`).
pub(crate) const TREND_ANGLE_RADIUS_CSS: f64 = 60.0;
pub(crate) const TREND_ANGLE_LABEL_GAP_CSS: f64 = 10.0;

/// Shared two-anchor segment behavior; every spec below overrides its identity.
const SEGMENT_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::HorizontalSegment,
    wire_id: 240,
    name: "horizontal_segment",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 1.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    anchor_link: DrawingAnchorLink::None,
    axis_tag_text: false,
};

// KLineChart's axis-locked segments: the anchors cannot leave their axis, so there is nothing for
// Shift to straighten. A vertical ray extends beyond its second anchor by default; an extended
// drawing's semantic bounds are unbounded (`DrawingBounds::for_drawing`).
pub(crate) const HORIZONTAL_SEGMENT: DrawingToolSpec = DrawingToolSpec {
    anchor_link: DrawingAnchorLink::SamePrice,
    ..SEGMENT_TOOL
};

pub(crate) const VERTICAL_RAY: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::VerticalRay,
    wire_id: 241,
    name: "vertical_ray",
    anchor_link: DrawingAnchorLink::SameLogical,
    ..SEGMENT_TOOL
};

pub(crate) const VERTICAL_SEGMENT: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::VerticalSegment,
    wire_id: 242,
    name: "vertical_segment",
    anchor_link: DrawingAnchorLink::SameLogical,
    ..SEGMENT_TOOL
};

// KLineChart's price line: a ray to the right from one anchor, its price printed above the line
// and tagged on the price axis.
pub(crate) const PRICE_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::PriceLine,
    wire_id: 243,
    name: "price_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    logical_extent: DrawingLogicalExtent::FromFirst,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: true,
    ..SEGMENT_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.owns_labels = true;
    family
};

/// The fork's info-line stats: price change, percent change, bar count, duration, angle.
fn default_info_stats() -> Vec<DrawingLabelOptions> {
    [
        DrawingLabelMetric::PriceChange,
        DrawingLabelMetric::PercentChange,
        DrawingLabelMetric::BarCount,
        DrawingLabelMetric::Duration,
        DrawingLabelMetric::Angle,
    ]
    .into_iter()
    .map(|metric| DrawingLabelOptions {
        metric,
        visible: true,
        position: DrawingLabelPosition::On,
        text: None,
    })
    .collect()
}

fn apply_defaults(drawing: &mut Drawing) {
    if drawing.kind == DrawingKind::VerticalRay {
        drawing.extend_right = true;
    }
}

/// The fork's pre-merge defaults of the upstream line tools it rendered (see
/// [`super::apply_legacy_fork_defaults`]).
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
    match drawing.kind {
        DrawingKind::Ray => drawing.extend_right = true,
        DrawingKind::ExtendedLine => {
            drawing.extend_left = true;
            drawing.extend_right = true;
        }
        DrawingKind::InfoLine => drawing.labels = default_info_stats(),
        DrawingKind::ArrowLine => drawing.stroke_end = DrawingLineCap::Arrow,
        _ => {}
    }
}

/// The fork's unstored `tool_options` default of the upstream line tools it rendered (see
/// [`super::legacy_fork_tool_options`]): every one drew its visible `labels` as one stats box,
/// which the presence of the `line` block selects on upstream's lowering.
pub(super) fn legacy_tool_options(kind: DrawingKind) -> Option<(&'static str, serde_json::Value)> {
    upstream_line(kind).then(|| ("line", serde_json::json!({})))
}

/// The upstream catalog's line tools, which the fork rendered as this family.
const fn upstream_line(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::Ray
            | DrawingKind::ExtendedLine
            | DrawingKind::InfoLine
            | DrawingKind::TrendAngle
            | DrawingKind::CrossLine
            | DrawingKind::ArrowLine
    )
}

/// Whether the stored `tool_options.line` block selects the fork's presentation of an upstream
/// line tool ([`upstream_line_parts`] layered on its upstream arm): its visible `labels` as one
/// stats box instead of upstream's per-label text, the trend angle's reference line, arc and
/// angle, and the segment tools' caps through the parts layer. Without the block the tool renders
/// exactly as upstream does; `tool_options.line: null` removes it.
pub(crate) fn fork_presentation(drawing: &Drawing) -> bool {
    upstream_line(drawing.kind) && drawing.tool_options.line.is_some()
}

/// Whether `drawing` paints the trend angle's dashed reference, arc and angle.
pub(crate) fn draws_angle_reference(drawing: &Drawing) -> bool {
    drawing.kind == DrawingKind::TrendAngle
        && drawing.points.len() >= 2
        && fork_presentation(drawing)
}

/// The x the trend angle's dashed reference reaches from its first anchor `a` (in any px space,
/// with its second anchor `b`): as long as the segment, toward the second anchor's side. Paint and
/// the screen culling box share it, so the reference stays inside the box at any zoom.
pub(crate) fn angle_reference_end(a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let direction = if dx < 0.0 { -1.0 } else { 1.0 };
    a.0 + direction * dx.hypot(dy)
}

/// The parts the `line` block layers on an upstream line tool's arm ([`fork_presentation`]). For
/// the five segment tools, `segment` is the upstream body's resolved ends: the stroke between them
/// with the drawing's caps on the ends that do not reach the pane edge (an arrow end trims the
/// stroke under its head; see [`DrawingParts::capped_polyline`]) replaces upstream's stroke and
/// caps, and a trend angle adds its reference, arc and angle. The cross line (`None`) keeps its
/// crisp upstream lines. The stats box of the visible `labels` comes last.
pub(crate) fn upstream_line_parts(
    ctx: &PartContext<'_>,
    segment: Option<(Point, Point)>,
    parts: &mut DrawingParts,
) {
    let drawing = ctx.drawing;
    if let Some((a, b)) = segment {
        let options = DrawingGeometryOptions::for_drawing(drawing, ctx.scale);
        // The anchors decide the direction (a vertical or empty segment), as in the resolver.
        let (p, q) = match (ctx.px.first(), ctx.px.get(1)) {
            (Some(&p), Some(&q)) => (p, q),
            _ => (a, b),
        };
        let (extend_a, extend_b) = segment_extension(drawing.kind, options, p, q);
        parts.capped_segment(drawing, a, b, (!extend_a, !extend_b), ctx.scale);
        if let (true, Some(&a), Some(&b)) = (
            draws_angle_reference(drawing),
            ctx.px.first(),
            ctx.px.get(1),
        ) {
            angle_decoration(ctx, a, b, parts);
        }
    }
    line_stats(ctx, parts);
}

/// [`upstream_line_parts`]' reach beyond the anchors in CSS px for the culling pad: the stats box,
/// the angle's label beside its arc, and the end caps. Without the `line` block, upstream's trend
/// angle's reference, arc, and label; 0 for the other tools.
pub(crate) fn upstream_decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if !fork_presentation(drawing) {
        // Upstream's trend angle reaches its reference radius plus the widest signed angle
        // label beyond the first anchor.
        if drawing.kind != DrawingKind::TrendAngle {
            return 0.0;
        }
        let size = engine.drawing_stats_size();
        let width = engine.measure_text_run(
            "-179.99°",
            size,
            &engine.options.get().layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        );
        return TREND_ANGLE_RADIUS_CSS + TREND_ANGLE_LABEL_GAP_CSS + width + size;
    }
    let mut extent = decoration_extent(engine, drawing);
    if drawing.kind == DrawingKind::TrendAngle {
        let size = engine.drawing_text_size(drawing);
        let width = engine.measure_text_run(
            "-90.00°",
            size,
            &engine.options.get().layout.font_family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        );
        extent = extent.max(ANGLE_ARC_MAX + ANGLE_LABEL_GAP + width + size);
    }
    if drawing.stroke_start != DrawingLineCap::None || drawing.stroke_end != DrawingLineCap::None {
        extent = extent.max(cap_radius(drawing.width));
    }
    extent
}

/// The `tool_options.line.stats_position` descriptor of an upstream line tool. Its default is the
/// position a present block takes; the box itself exists only while the block does.
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    template: &Drawing,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    if upstream_line(kind) {
        extend_schema(template, properties);
    }
}

/// Whether `labels` (a clipboard or sync item's) are the fork's info-line default, which no
/// upstream drawing carries: upstream's info line starts with seven `above` stats (four before
/// upstream 664d347).
pub(crate) fn is_legacy_info_stats(labels: &[DrawingLabelOptions]) -> bool {
    labels == default_info_stats()
}

fn options(drawing: &Drawing) -> LineToolOptions {
    drawing.tool_options.line.unwrap_or_default()
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(&a) = ctx.px.first() else {
        return;
    };
    if drawing.kind == DrawingKind::PriceLine {
        price_line(ctx, a, parts);
    } else {
        let Some(&b) = ctx.px.get(1) else {
            return;
        };
        let (extend_start, extend_end) = (drawing.extend_left, drawing.extend_right);
        let (start, end) = shape::extend_segment(a, b, ctx.pane, extend_start, extend_end);
        parts.capped_segment(drawing, start, end, (!extend_start, !extend_end), ctx.scale);
    }
    line_stats(ctx, parts);
}

/// Gap between a price line and the price printed above its start (CSS px).
const PRICE_LINE_GAP: f64 = 2.0;

/// The line from the anchor to the pane's right edge, with the anchor's price above its start.
fn price_line(ctx: &PartContext<'_>, a: Point, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    parts.hline(a.1, a.0, ctx.pane.right, PartStroke::default());
    let Some(point) = drawing.points.first() else {
        return;
    };
    parts.label(PartLabel {
        anchor: (a.0, a.1 - PRICE_LINE_GAP * ctx.scale),
        h_align: DrawingTextHAlign::Left,
        v_align: DrawingTextVAlign::Bottom,
        lines: vec![ctx.engine.format_drawing_price(drawing, point.price)],
        size: ctx.engine.drawing_text_size(drawing) * ctx.scale,
        weight: drawing.text_weight.unwrap_or(400),
        italic: drawing.text_italic,
        color: None,
        background: None,
        border: None,
        padding: (0.0, 0.0),
        hit: false,
    });
}

/// The dashed horizontal reference toward the second anchor's side, the arc from it to the
/// segment, and the screen angle beside the arc.
fn angle_decoration(ctx: &PartContext<'_>, a: Point, b: Point, parts: &mut DrawingParts) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dy);
    if length <= f64::EPSILON {
        return;
    }
    // Crisp like the horizontal-line tool, so the dash pattern is the executors' shared
    // full-pixel dash rather than a path dash.
    let decoration = PartStroke::decoration(ANGLE_DECORATION_WIDTH, LineStyle::Dashed);
    parts.hline(a.1, a.0, angle_reference_end(a, b), decoration);

    let radius = (length / 3.0)
        .clamp(ANGLE_ARC_MIN * ctx.scale, ANGLE_ARC_MAX * ctx.scale)
        .min(length);
    let start = if dx < 0.0 { std::f64::consts::PI } else { 0.0 };
    let sweep = {
        let raw = dy.atan2(dx) - start;
        (raw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
    // At most 48 CSS px of radius: the uniform chords stay small and within the tolerance.
    let mut arc = Vec::new();
    shape::EllipseArc::circle(a, radius, start, sweep).append_points(CURVE_TOLERANCE, &mut arc);
    parts.stroke(
        &arc,
        PartStroke::decoration(ANGLE_DECORATION_WIDTH, LineStyle::Solid),
        false,
    );

    let Some(degrees) = trend_angle_degrees(ctx.engine, ctx.drawing) else {
        return;
    };
    let bisector = start + sweep / 2.0;
    let distance = radius + ANGLE_LABEL_GAP * ctx.scale;
    parts.label(PartLabel {
        anchor: (
            a.0 + distance * bisector.cos(),
            a.1 + distance * bisector.sin(),
        ),
        h_align: if bisector.cos() >= 0.0 {
            DrawingTextHAlign::Left
        } else {
            DrawingTextHAlign::Right
        },
        v_align: DrawingTextVAlign::Middle,
        lines: vec![format!("{degrees:.2}°")],
        size: ctx.engine.drawing_text_size(ctx.drawing) * ctx.scale,
        weight: ctx.drawing.text_weight.unwrap_or(400),
        italic: ctx.drawing.text_italic,
        color: None,
        background: None,
        border: None,
        padding: (0.0, 0.0),
        hit: false,
    });
}

/// The trend angle's value: the screen angle of the segment from the horizontal toward its
/// second anchor, rising positive, folded into [-90°, 90°].
fn trend_angle_degrees(engine: &ChartEngine, drawing: &Drawing) -> Option<f64> {
    let (angle, _) = engine.drawing_screen_vector(drawing, 0, 1)?;
    Some(if angle > 90.0 {
        180.0 - angle
    } else if angle < -90.0 {
        -180.0 - angle
    } else {
        angle
    })
}

/// The stats box of the visible `labels`, placed by `tool_options.line.stats_position`.
fn line_stats(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    if !drawing.labels.iter().any(|label| label.visible) {
        return;
    }
    let Some(&a) = ctx.px.first() else {
        return;
    };
    let to = usize::from(ctx.px.len() > 1);
    let lines = ctx.engine.drawing_stat_lines(drawing, 0, to);
    if lines.is_empty() {
        return;
    }
    let b = ctx.px.get(to).copied().unwrap_or(a);
    let gap = STATS_GAP * ctx.scale;
    let forward = b.0 >= a.0;
    let (anchor, h_align, v_align) = match options(drawing).stats_position {
        DrawingStatsPosition::End => (
            (if forward { b.0 + gap } else { b.0 - gap }, b.1),
            if forward {
                DrawingTextHAlign::Left
            } else {
                DrawingTextHAlign::Right
            },
            DrawingTextVAlign::Middle,
        ),
        DrawingStatsPosition::Start => (
            (if forward { a.0 - gap } else { a.0 + gap }, a.1),
            if forward {
                DrawingTextHAlign::Right
            } else {
                DrawingTextHAlign::Left
            },
            DrawingTextVAlign::Middle,
        ),
        DrawingStatsPosition::Middle => (
            ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0 + gap),
            DrawingTextHAlign::Center,
            DrawingTextVAlign::Top,
        ),
    };
    parts.label(ctx.stats_box(anchor, (h_align, v_align), lines, None, text_on));
}

/// Conservative reach of the stats box beyond the anchors, in CSS px. Text that follows the
/// viewport (angle, distance) or data (bars, duration) gets four ems of slack so the cached pad
/// stays valid between text-key refreshes.
fn decoration_extent(engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let family = &engine.options.get().layout.font_family;
    let mut extent: f64 = 0.0;
    if drawing.labels.iter().any(|label| label.visible) {
        let size = engine.drawing_stats_size();
        let to = usize::from(drawing.points.len() > 1);
        let lines = engine.drawing_stat_lines(drawing, 0, to);
        let width = lines
            .iter()
            .map(|line| engine.measure_text_run(line, size, family, 400, false))
            .fold(0.0_f64, f64::max);
        let height = lines.len() as f64 * size * 1.25;
        extent = extent.max(STATS_GAP + width + 2.0 * STATS_PADDING.0 + 4.0 * size);
        extent = extent.max(STATS_GAP + height + 2.0 * STATS_PADDING.1);
    }
    extent
}

fn extend_schema(_template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    let mut stats_position = DrawingPropertyDescriptor {
        name: "tool_options.line.stats_position".to_string(),
        property_type: DrawingPropertyType::Enum,
        default: serde_json::json!(DrawingStatsPosition::default().name()),
        min: None,
        max: None,
        enum_values: Vec::new(),
    };
    stats_position.enum_values = [
        DrawingStatsPosition::Start,
        DrawingStatsPosition::Middle,
        DrawingStatsPosition::End,
    ]
    .into_iter()
    .map(|position| position.name().to_string())
    .collect();
    properties.push(stats_position);
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    DrawingKindOptions::Line {
        stats_position: options(drawing).stats_position,
    }
}

#[cfg(test)]
mod tests;
