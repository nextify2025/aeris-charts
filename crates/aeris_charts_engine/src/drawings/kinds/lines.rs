//! B8 Lines family (wire ids 32..=47): ray, extended line, info line, trend angle, cross line,
//! and arrow line. The trend, horizontal, and vertical line tools predate the family and stay
//! core tools.
//!
//! The five segment tools share one geometry: the first two anchors, extended beyond the first
//! anchor by `extend_left` and beyond the second by `extend_right` to the pane edge (the ray and
//! extended line are these flags' defaults), end caps on the ends that are not extended, and the
//! segment-layout text label. Any visible `labels` render as one stats box (see
//! [`ChartEngine::drawing_stat_lines`]); the info line enables five stats by default. The trend
//! angle adds a dashed horizontal reference, the arc between it and the segment, and the screen
//! angle. The cross line is crisp full-span horizontal and vertical lines through its anchor with
//! the horizontal line's axis price tag.

use aeris_charts_render::color::Color;
use aeris_charts_render::draw_list::LineStyle;
use aeris_charts_render::shape::{self, Point};

use super::super::parts::{
    text_on, DrawingParts, PartContext, PartLabel, PartStroke, CURVE_TOLERANCE, STATS_ALPHA,
    STATS_GAP, STATS_PADDING,
};
use super::super::tools::{
    DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis, DrawingPlacement,
    DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout, DrawingToolSpec,
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

/// Shared two-anchor segment behavior; every spec below overrides its identity.
const SEGMENT_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Ray,
    wire_id: 32,
    name: "ray",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    grid_snap: false,
};

// A ray's and an extended line's reach comes from their `extend_left`/`extend_right` defaults:
// an extended drawing's semantic bounds are unbounded (`DrawingBounds::for_drawing`), and one whose
// extensions are switched off culls like any finite segment.
pub(crate) const RAY: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Ray,
    wire_id: 32,
    name: "ray",
    ..SEGMENT_TOOL
};

pub(crate) const EXTENDED_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ExtendedLine,
    wire_id: 33,
    name: "extended_line",
    ..SEGMENT_TOOL
};

pub(crate) const INFO_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::InfoLine,
    wire_id: 34,
    name: "info_line",
    ..SEGMENT_TOOL
};

pub(crate) const TREND_ANGLE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::TrendAngle,
    wire_id: 35,
    name: "trend_angle",
    ..SEGMENT_TOOL
};

pub(crate) const CROSS_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::CrossLine,
    wire_id: 36,
    name: "cross_line",
    placement: DrawingPlacement::ClickAnchors { count: 1 },
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    text_layout: DrawingTextLayout::Box,
    axis_price_label: true,
    ..SEGMENT_TOOL
};

pub(crate) const ARROW_LINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::ArrowLine,
    wire_id: 37,
    name: "arrow_line",
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

/// The info line's default stats: price change, percent change, bar count, duration, angle.
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

fn options(drawing: &Drawing) -> LineToolOptions {
    drawing.tool_options.line.unwrap_or_default()
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let drawing = ctx.drawing;
    let Some(&a) = ctx.px.first() else {
        return;
    };
    if drawing.kind == DrawingKind::CrossLine {
        parts.hline(a.1, ctx.pane.left, ctx.pane.right, PartStroke::default());
        parts.vline(a.0, ctx.pane.top, ctx.pane.bottom, PartStroke::default());
    } else {
        let Some(&b) = ctx.px.get(1) else {
            return;
        };
        let (extend_start, extend_end) = (drawing.extend_left, drawing.extend_right);
        let (start, end) = shape::extend_segment(a, b, ctx.pane, extend_start, extend_end);
        parts.capped_segment(drawing, start, end, (!extend_start, !extend_end), ctx.scale);
        if drawing.kind == DrawingKind::TrendAngle {
            angle_decoration(ctx, a, b, parts);
        }
    }
    stats_box(ctx, parts);
}

/// The dashed horizontal reference toward the second anchor's side, the arc from it to the
/// segment, and the screen angle beside the arc.
fn angle_decoration(ctx: &PartContext<'_>, a: Point, b: Point, parts: &mut DrawingParts) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dy);
    if length <= f64::EPSILON {
        return;
    }
    let direction = if dx < 0.0 { -1.0 } else { 1.0 };
    // Crisp like the horizontal-line tool, so the dash pattern is the executors' shared
    // full-pixel dash rather than a path dash.
    let decoration = PartStroke::decoration(ANGLE_DECORATION_WIDTH, LineStyle::Dashed);
    parts.hline(a.1, a.0, a.0 + direction * length, decoration);

    let radius = (length / 3.0)
        .clamp(ANGLE_ARC_MIN * ctx.scale, ANGLE_ARC_MAX * ctx.scale)
        .min(length);
    let start = if direction > 0.0 {
        0.0
    } else {
        std::f64::consts::PI
    };
    let sweep = {
        let raw = dy.atan2(dx) - start;
        (raw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
    };
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
    let size = ctx.engine.drawing_text_size(ctx.drawing) * ctx.scale;
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
        size,
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
/// second anchor, rising positive, in [-90°, 90°].
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

fn stats_box(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
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
    let base = drawing.stroke_color();
    let background = Color::rgba(base.r(), base.g(), base.b(), STATS_ALPHA);
    parts.label(ctx.stats_label(
        anchor,
        (h_align, v_align),
        lines,
        background,
        text_on(background),
    ));
}

/// Conservative reach of the stats box and the trend-angle label beyond the anchors, in CSS px.
/// Text that follows the viewport (angle, distance) or data (bars, duration) gets four ems of
/// slack so the cached pad stays valid between text-key refreshes.
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
    if drawing.kind == DrawingKind::TrendAngle {
        let size = engine.drawing_text_size(drawing);
        let width = engine.measure_text_run(
            "-90.00°",
            size,
            family,
            drawing.text_weight.unwrap_or(400),
            drawing.text_italic,
        );
        extent = extent.max(ANGLE_ARC_MAX + ANGLE_LABEL_GAP + width + size);
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
