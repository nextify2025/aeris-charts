//! B8 Lines family, own-line tools (wire ids 240..=243): the KLineChart horizontal segment,
//! vertical ray, and vertical segment (axis-locked segments) and the price line. The upstream
//! catalog's line tools (ray, extended line, info line, trend angle, cross line, arrow line) are
//! upstream-rendered and carry no family hooks; [`legacy_defaults`] keeps the fork's pre-merge
//! defaults of those tools for documents the fork wrote.
//!
//! The axis-locked segments share one geometry: the first two anchors, extended beyond the first
//! anchor by `extend_left` and beyond the second by `extend_right` to the pane edge (the vertical
//! ray extends to the right by default), end caps on the ends that are not extended, and the
//! segment-layout text label. The price line is a ray to the right from its anchor with its price
//! printed above the line and tagged on the price axis. Any visible `labels` render as one stats
//! box (see [`ChartEngine::drawing_stat_lines`]).

use aeris_charts_render::color::Color;
use aeris_charts_render::shape::{self, Point};

use super::super::parts::{
    text_on, DrawingParts, PartContext, PartLabel, PartStroke, STATS_ALPHA, STATS_GAP,
    STATS_PADDING,
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
    default_width: 2.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Segment,
    axis_price_label: false,
    grid_snap: false,
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
    stats_box(ctx, parts);
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
        lines: vec![ctx.engine.drawing_price_text(drawing, point.price)],
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
