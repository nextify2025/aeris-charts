//! B8 Shapes family (wire ids 192..=223): rotated rectangle, ellipse, circle, triangle, arc,
//! curve, double curve, polyline, and highlighter. The rectangle, path, and brush predate the
//! family and stay core tools.
//!
//! Every shape resolves in the caller's px from its anchors, so it stays true on screen at any
//! zoom and scale (a circle stays round, a rotated rectangle keeps its right angles):
//!
//! - rotated rectangle: anchors 0 and 1 are the midpoints of the short sides and anchor 2 lies on
//!   a long side; the rectangle is symmetric about the 0→1 axis. Its handles are the two axis
//!   ends plus derived width handles at the long sides' midpoints (anchor 2 has no handle of its
//!   own); dragging an axis end keeps the screen width.
//! - ellipse: inscribed in the box of its two corners, edited with the rectangle's eight bounds
//!   handles (Shift makes it a circle).
//! - circle: its center (anchor 0) and a point on its rim (anchor 1).
//! - triangle: three vertices.
//! - arc: the circular arc from anchor 0 to anchor 1 through anchor 2 (a straight segment when
//!   they are collinear); its fill is the circular segment between the arc and its chord.
//! - curve: the quadratic Bézier from anchor 0 to anchor 1 through anchor 2 at its midpoint;
//!   double curve: the cubic from anchor 0 to anchor 1 through anchors 2 and 3 at one and two
//!   thirds. Every handle sits on the curve. `extend_left`/`extend_right` continue the end
//!   tangents to the pane edge, and the fill covers the region between the curve and its chord.
//! - polyline: multi-click vertices; `tool_options.shape.closed` joins the last vertex to the
//!   first and fills the enclosed region by the nonzero rule. Clicking the first vertex once three
//!   are placed sets it and finishes the placement. The fill is bounded work
//!   ([`shape::MAX_FILL_VERTICES`] vertices and the crossing and rung bounds of
//!   [`shape::nonzero_ribbon_contours`]): beyond them the outline is painted without a fill or an
//!   interior hit target.
//! - highlighter: a freehand wide translucent marker stroke with round joins and caps, painted as
//!   the region within half its width of the captured path (the shared `Tube` part), so every
//!   executor blends each pixel once and the stroke keeps one opacity where it overlaps itself.
//!   Its line style, caps, and fill do not apply.
//!
//! Closed shapes fill with `fill_color` (default: the stroke color at 20% alpha, the rectangle's
//! wash) while `fill_enabled`. Like the rectangle's, the fill is a body target only while the
//! drawing is selected, so an unselected shape's interior keeps panning the chart. Open strokes
//! (arc, curves, open polyline) carry the drawing's end caps. Curves and circles flatten through
//! the clip-aware helpers of `aeris_charts_render::shape`, so a huge zoomed-in shape stays within
//! tolerance on screen with bounded work, and off-screen pieces cost a few chords.

use aeris_charts_render::shape::{self, EllipseArc, Point, Rect};

use super::super::handles::{DrawingHandle, HandleDrag, HandleShape};
use super::super::parts::{cap_radius, DrawingParts, PartContext, PartStroke, CURVE_TOLERANCE};
use super::super::tools::{
    DrawingAnchorLink, DrawingHandleMode, DrawingLogicalExtent, DrawingMovementAxis,
    DrawingPlacement, DrawingPriceExtent, DrawingStraightenMode, DrawingTextLayout,
    DrawingToolSpec,
};
use super::super::Drawing;
use super::DrawingFamily;
use crate::{
    ChartEngine, DrawingDragPart, DrawingKind, DrawingKindOptions, DrawingLineCap, DrawingPoint,
    DrawingPropertyDescriptor, DrawingPropertyType, DrawingToolOptions,
};

/// Shapes-family options (`tool_options.shape`); absent fields keep their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShapeToolOptions {
    /// Polyline only: join the last vertex back to the first and, while `fill_enabled`, fill the
    /// enclosed region (nonzero rule; outline only beyond [`shape::MAX_FILL_VERTICES`] vertices or
    /// the fill's crossing and rung bounds). The other shapes ignore it.
    pub closed: bool,
}

/// Fill alpha over the stroke color when `fill_color` is unset (the rectangle's 20% wash).
const FILL_ALPHA: u8 = 51;
/// Highlighter opacity over the canonical market-warning amber, the conventional marker hue.
const HIGHLIGHTER_OPACITY: f64 = 0.4;
/// Screen reach, in CSS px beyond the stroke's half width, past which geometry can neither paint
/// nor hit inside the pane (covers the touch hit tolerance).
const CLIP_MARGIN: f64 = 16.0;

/// Shared closed-shape behavior; every spec below overrides its identity.
const SHAPE_TOOL: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Triangle,
    wire_id: 195,
    name: "triangle",
    placement: DrawingPlacement::ClickAnchors { count: 3 },
    handles: DrawingHandleMode::Anchors,
    movement_axis: DrawingMovementAxis::Both,
    straighten: DrawingStraightenMode::Segment45,
    logical_extent: DrawingLogicalExtent::Finite,
    price_extent: DrawingPriceExtent::Finite,
    bounds_padding_ratio: 0.0,
    default_width: 2.0,
    requests_text_editor: false,
    family: Some(&FAMILY),
    text_layout: DrawingTextLayout::Box,
    axis_price_label: false,
    grid_snap: false,
    anchor_link: DrawingAnchorLink::None,
};

// The rotated rectangle, circle, and arc derive their extent from screen-space perpendiculars
// and radii, which no scale-independent box of their anchors bounds, so they never cull
// semantically; their screen culling box is their exact shape box (the family's `paint_bounds`).
pub(crate) const ROTATED_RECTANGLE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::RotatedRectangle,
    wire_id: 192,
    name: "rotated_rectangle",
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..SHAPE_TOOL
};

pub(crate) const ELLIPSE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Ellipse,
    wire_id: 193,
    name: "ellipse",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    handles: DrawingHandleMode::RectangleBounds,
    straighten: DrawingStraightenMode::Square,
    ..SHAPE_TOOL
};

pub(crate) const CIRCLE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Circle,
    wire_id: 194,
    name: "circle",
    placement: DrawingPlacement::ClickAnchors { count: 2 },
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..SHAPE_TOOL
};

pub(crate) const TRIANGLE: DrawingToolSpec = SHAPE_TOOL;

pub(crate) const ARC: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Arc,
    wire_id: 196,
    name: "arc",
    straighten: DrawingStraightenMode::None,
    logical_extent: DrawingLogicalExtent::Full,
    price_extent: DrawingPriceExtent::Full,
    ..SHAPE_TOOL
};

// A curve through equally spaced parameters overshoots its anchors' span by at most
// (Lebesgue constant − 1) / 2 of it: 0.125 for three nodes, 0.316 for four. The logical axis is
// affine, so these pads bound the curve at any zoom; the price axis may be logarithmic, so it
// stays unbounded.
pub(crate) const CURVE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Curve,
    wire_id: 197,
    name: "curve",
    straighten: DrawingStraightenMode::None,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.25,
    ..SHAPE_TOOL
};

pub(crate) const DOUBLE_CURVE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::DoubleCurve,
    wire_id: 198,
    name: "double_curve",
    placement: DrawingPlacement::ClickAnchors { count: 4 },
    straighten: DrawingStraightenMode::None,
    price_extent: DrawingPriceExtent::Full,
    bounds_padding_ratio: 0.5,
    ..SHAPE_TOOL
};

pub(crate) const POLYLINE: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Polyline,
    wire_id: 199,
    name: "polyline",
    placement: DrawingPlacement::MultiClick { minimum: 2 },
    straighten: DrawingStraightenMode::None,
    ..SHAPE_TOOL
};

pub(crate) const HIGHLIGHTER: DrawingToolSpec = DrawingToolSpec {
    kind: DrawingKind::Highlighter,
    wire_id: 200,
    name: "highlighter",
    placement: DrawingPlacement::Freehand { minimum: 2 },
    handles: DrawingHandleMode::Endpoints,
    straighten: DrawingStraightenMode::None,
    default_width: 20.0,
    ..SHAPE_TOOL
};

pub(crate) static FAMILY: DrawingFamily = {
    let mut family = DrawingFamily::new(build_parts, kind_options);
    family.apply_defaults = apply_defaults;
    family.decoration_extent = decoration_extent;
    family.extend_schema = extend_schema;
    family.paint_bounds = |_, drawing, px| shape_box(drawing.kind, px);
    family.text_box = shape_box;
    family.handles = handles;
    family.drag = drag;
    family.close_placement = Some(close_placement);
    family
};

/// Clicking a polyline's first vertex closes it (the only multi-click shape).
fn close_placement(drawing: &mut Drawing) {
    let mut shape = options(drawing);
    shape.closed = true;
    drawing.tool_options.shape = Some(shape);
}

/// A rotated rectangle's width handles, at the midpoints of its long sides (`Handle(0)` on the
/// side the third anchor lies on, `Handle(1)` opposite), replace the third anchor's own handle.
/// A zero-length axis has no width to edit and keeps the anchor handle.
fn handles(
    _engine: &ChartEngine,
    drawing: &Drawing,
    px: &[Point],
    handles: &mut Vec<DrawingHandle>,
) {
    if drawing.kind != DrawingKind::RotatedRectangle {
        return;
    }
    let Some(corners) = rotated_rectangle(px) else {
        return;
    };
    handles.retain(|handle| handle.part != DrawingDragPart::Anchor(2));
    for (index, (a, b)) in [(corners[0], corners[1]), (corners[2], corners[3])]
        .into_iter()
        .enumerate()
    {
        handles.push(DrawingHandle {
            point: shape::midpoint(a, b),
            part: DrawingDragPart::Handle(index),
            cursor: "pointer",
            shape: HandleShape::Disc,
        });
    }
}

/// Rotated-rectangle drags keep it a rectangle of the intended width on screen: a width handle
/// sets the width to its target's distance from the axis, and an axis end drag keeps the
/// baseline width about the new axis (so rotating the axis never collapses it). The width point
/// is stored at the long side's midpoint, exactly on the construction, without time snapping.
fn drag(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [DrawingPoint],
) -> Option<DrawingToolOptions> {
    if drawing.kind != DrawingKind::RotatedRectangle || points.len() < 3 {
        return None;
    }
    let (&a, &b, &c) = (
        sample.start_px.first()?,
        sample.start_px.get(1)?,
        sample.start_px.get(2)?,
    );
    let normal = shape::segment_normal(a, b)?;
    let across = |point: Point, from: Point, normal: Point| {
        (point.0 - from.0) * normal.0 + (point.1 - from.1) * normal.1
    };
    let (axis, half) = match sample.part {
        DrawingDragPart::Handle(_) => ((a, b), across(sample.target_px, a, normal)),
        DrawingDragPart::Anchor(0 | 1) => (
            (
                engine.drawing_point_px(drawing, points[0])?,
                engine.drawing_point_px(drawing, points[1])?,
            ),
            across(c, a, normal),
        ),
        _ => return None,
    };
    let axis_normal = shape::segment_normal(axis.0, axis.1)?;
    let middle = shape::midpoint(axis.0, axis.1);
    points[2] = engine.drawing_anchor_from_px(
        drawing.kind,
        drawing.pane_index,
        drawing.price_scale,
        middle.0 + axis_normal.0 * half,
        middle.1 + axis_normal.1 * half,
    )?;
    None
}

fn apply_defaults(drawing: &mut Drawing) {
    match drawing.kind {
        DrawingKind::RotatedRectangle
        | DrawingKind::Ellipse
        | DrawingKind::Circle
        | DrawingKind::Triangle
        | DrawingKind::Arc
        | DrawingKind::Polyline => drawing.fill_enabled = true,
        DrawingKind::Highlighter => {
            let (r, g, b) = aeris_charts_core::style::MARKET_WARNING_RGB;
            drawing.color = format!("rgba({r}, {g}, {b}, {HIGHLIGHTER_OPACITY})");
        }
        _ => {}
    }
}

fn options(drawing: &Drawing) -> ShapeToolOptions {
    drawing.tool_options.shape.unwrap_or_default()
}

/// Corners of a rotated rectangle from its axis (the short sides' midpoints `a`, `b`) and a
/// point `c` on a long side, in order around the outline. `None` for a zero-length axis.
fn rotated_rectangle(px: &[Point]) -> Option<[Point; 4]> {
    let (&a, &b, &c) = (px.first()?, px.get(1)?, px.get(2)?);
    let normal = shape::segment_normal(a, b)?;
    let half = (c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1;
    let (ox, oy) = (normal.0 * half, normal.1 * half);
    Some([
        (a.0 + ox, a.1 + oy),
        (b.0 + ox, b.1 + oy),
        (b.0 - ox, b.1 - oy),
        (a.0 - ox, a.1 - oy),
    ])
}

/// A circle's center and screen radius from its center and rim anchors.
fn circle(px: &[Point]) -> Option<(Point, f64)> {
    let (&center, &rim) = (px.first()?, px.get(1)?);
    Some((center, (rim.0 - center.0).hypot(rim.1 - center.1)))
}

/// The full ellipse inscribed in the box of two corners.
fn ellipse(px: &[Point]) -> Option<EllipseArc> {
    let (&a, &b) = (px.first()?, px.get(1)?);
    Some(EllipseArc {
        center: shape::midpoint(a, b),
        rx: (b.0 - a.0).abs() / 2.0,
        ry: (b.1 - a.1).abs() / 2.0,
        rotation: 0.0,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    })
}

/// The circular arc from `a` to `b` through `through`; `None` when they are collinear.
fn arc_through(a: Point, b: Point, through: Point) -> Option<EllipseArc> {
    let (center, radius) = shape::circle_through(a, b, through)?;
    let angle = |point: Point| (point.1 - center.1).atan2(point.0 - center.0);
    let start = angle(a);
    let tau = std::f64::consts::TAU;
    let forward = (angle(b) - start).rem_euclid(tau);
    let sweep = if (angle(through) - start).rem_euclid(tau) <= forward {
        forward
    } else {
        forward - tau
    };
    Some(EllipseArc::circle(center, radius, start, sweep))
}

/// Whether `angle` lies on the arc's sweep.
fn on_sweep(arc: &EllipseArc, angle: f64) -> bool {
    let tau = std::f64::consts::TAU;
    if arc.sweep >= 0.0 {
        (angle - arc.start).rem_euclid(tau) <= arc.sweep
    } else {
        (arc.start - angle).rem_euclid(tau) <= -arc.sweep
    }
}

/// The quadratic control point of the curve through `m` at t = 1/2.
fn quadratic_control(a: Point, b: Point, m: Point) -> Point {
    (2.0 * m.0 - (a.0 + b.0) / 2.0, 2.0 * m.1 - (a.1 + b.1) / 2.0)
}

/// The cubic control points of the curve through `p` at t = 1/3 and `q` at t = 2/3.
fn cubic_controls(a: Point, b: Point, p: Point, q: Point) -> (Point, Point) {
    let control = |near: Point, far: Point, through_near: Point, through_far: Point| {
        let axis = |i: fn(Point) -> f64| {
            3.0 * i(through_near) - 1.5 * i(through_far) - 5.0 / 6.0 * i(near) + i(far) / 3.0
        };
        (axis(|point| point.0), axis(|point| point.1))
    };
    (control(a, b, p, q), control(b, a, q, p))
}

/// A fill part in the drawing's fill color, a body target only while the drawing is selected.
fn push_fill(ctx: &PartContext<'_>, polygon: &[Point], convex: bool, parts: &mut DrawingParts) {
    if !ctx.drawing.fill_enabled || polygon.len() < 3 {
        return;
    }
    let color = Some(ctx.drawing.fill_or_wash(FILL_ALPHA));
    let hit = ctx.fills_hit();
    if convex {
        parts.fill_convex(polygon, color, hit);
    } else {
        parts.fill_polygon(polygon, color, hit);
    }
}

fn misses(points: &[Point], clip: Rect) -> bool {
    Rect::bounding(points).is_none_or(|bounds| !bounds.intersects(&clip))
}

fn build_parts(ctx: &PartContext<'_>, parts: &mut DrawingParts) {
    let (drawing, px) = (ctx.drawing, ctx.px);
    let clip = ctx
        .pane
        .inflate((drawing.width / 2.0 + CLIP_MARGIN) * ctx.scale);
    match drawing.kind {
        DrawingKind::RotatedRectangle => {
            if let Some(corners) = rotated_rectangle(px) {
                closed_polygon(ctx, &corners, true, clip, parts);
            }
        }
        DrawingKind::Triangle if px.len() >= 3 => closed_polygon(ctx, &px[..3], true, clip, parts),
        DrawingKind::Ellipse => {
            if let Some(outline) = ellipse(px) {
                closed_curve(ctx, outline, clip, parts);
            }
        }
        DrawingKind::Circle => {
            if let Some((center, radius)) = circle(px) {
                let outline = EllipseArc::circle(center, radius, 0.0, std::f64::consts::TAU);
                closed_curve(ctx, outline, clip, parts);
            }
        }
        DrawingKind::Arc => arc(ctx, clip, parts),
        DrawingKind::Curve | DrawingKind::DoubleCurve => curve(ctx, clip, parts),
        DrawingKind::Polyline if options(drawing).closed => {
            closed_polygon(ctx, px, false, clip, parts);
        }
        DrawingKind::Polyline if !misses(px, clip) => {
            parts.capped_polyline(drawing, px, (true, true), (None, None), ctx.scale, false);
        }
        DrawingKind::Highlighter if !misses(px, clip) => {
            parts.tube(px, PartStroke::default());
        }
        _ => {}
    }
}

/// A closed outline through `vertices` over its fill. The stroke starts and ends mid-edge, so its
/// two butt ends meet collinearly instead of notching a corner. A non-convex fill beyond the
/// [`shape::nonzero_ribbon_contours`] bounds is skipped (the outline remains); the decision is made
/// on the whole unclipped polygon, not its visible part.
fn closed_polygon(
    ctx: &PartContext<'_>,
    vertices: &[Point],
    convex: bool,
    clip: Rect,
    parts: &mut DrawingParts,
) {
    let count = vertices.len();
    if misses(vertices, clip) {
        return;
    }
    push_fill(ctx, vertices, convex, parts);
    let Some(first) = (0..count).find(|&index| vertices[index] != vertices[(index + 1) % count])
    else {
        return;
    };
    let start = shape::midpoint(vertices[first], vertices[(first + 1) % count]);
    let mut outline = Vec::with_capacity(count + 2);
    outline.push(start);
    outline.extend((1..=count).map(|step| vertices[(first + step) % count]));
    outline.push(start);
    parts.stroke(&outline, PartStroke::default(), false);
}

/// A full ellipse or circle over its fill.
fn closed_curve(ctx: &PartContext<'_>, outline: EllipseArc, clip: Rect, parts: &mut DrawingParts) {
    let reach = outline.rx.max(outline.ry);
    let bounds = [
        (outline.center.0 - reach, outline.center.1 - reach),
        (outline.center.0 + reach, outline.center.1 + reach),
    ];
    if misses(&bounds, clip) {
        return;
    }
    let mut points = Vec::new();
    outline.append_clipped_points(CURVE_TOLERANCE, clip, &mut points);
    // The closing point repeats the first; the fill polygon needs it once.
    push_fill(ctx, &points[..points.len().saturating_sub(1)], true, parts);
    parts.stroke(&points, PartStroke::default(), false);
}

/// The arc (or its collinear straight segment) over its circular-segment fill, with end caps
/// along the exact end tangents.
fn arc(ctx: &PartContext<'_>, clip: Rect, parts: &mut DrawingParts) {
    let (Some(&a), Some(&b), Some(&through)) = (ctx.px.first(), ctx.px.get(1), ctx.px.get(2))
    else {
        return;
    };
    let mut points = Vec::new();
    let mut toward = (None, None);
    match arc_through(a, b, through) {
        Some(outline) => {
            outline.append_clipped_points(CURVE_TOLERANCE, clip, &mut points);
            // Unit tangents in the direction of travel: the caps point away from the stroke.
            let direction = outline.sweep.signum();
            let tangent = |point: Point| {
                let (rx, ry) = (point.0 - outline.center.0, point.1 - outline.center.1);
                let length = rx.hypot(ry).max(f64::MIN_POSITIVE);
                (-ry / length * direction, rx / length * direction)
            };
            let (start, end) = (tangent(a), tangent(b));
            toward = (
                Some((a.0 + start.0, a.1 + start.1)),
                Some((b.0 - end.0, b.1 - end.1)),
            );
        }
        None => points.extend([a, b]),
    }
    if misses(&points, clip) {
        return;
    }
    push_fill(ctx, &points, true, parts);
    parts.capped_polyline(ctx.drawing, &points, (true, true), toward, ctx.scale, false);
}

/// A quadratic or cubic curve through its anchors, its end-tangent extensions, and its chord
/// fill.
fn curve(ctx: &PartContext<'_>, clip: Rect, parts: &mut DrawingParts) {
    let (drawing, px) = (ctx.drawing, ctx.px);
    let (Some(&a), Some(&b)) = (px.first(), px.get(1)) else {
        return;
    };
    let mut points = Vec::new();
    let (first_control, last_control) = match (drawing.kind, px.get(2), px.get(3)) {
        (DrawingKind::Curve, Some(&m), _) => {
            let control = quadratic_control(a, b, m);
            shape::flatten_quadratic(a, control, b, CURVE_TOLERANCE, clip, &mut points);
            (control, control)
        }
        (DrawingKind::DoubleCurve, Some(&p), Some(&q)) => {
            let (first, last) = cubic_controls(a, b, p, q);
            shape::flatten_cubic(a, first, last, b, CURVE_TOLERANCE, clip, &mut points);
            (first, last)
        }
        _ => return,
    };
    // Each end's tangent follows its nearest distinct control point (the chord when the
    // controls collapse onto the end).
    let tangent_point =
        |end: Point, candidates: [Point; 3]| candidates.into_iter().find(|&point| point != end);
    let start_toward = tangent_point(a, [first_control, last_control, b]);
    let end_toward = tangent_point(b, [last_control, first_control, a]);
    let (extend_start, extend_end) = (drawing.extend_left, drawing.extend_right);
    let mut stroke = Vec::with_capacity(points.len() + 2);
    if let (true, Some(toward)) = (extend_start, start_toward) {
        let (_, edge) = shape::extend_segment(toward, a, ctx.pane, false, true);
        if edge != a {
            stroke.push(edge);
        }
    }
    stroke.extend_from_slice(&points);
    if let (true, Some(toward)) = (extend_end, end_toward) {
        let (_, edge) = shape::extend_segment(toward, b, ctx.pane, false, true);
        if edge != b {
            stroke.push(edge);
        }
    }
    if misses(&stroke, clip) {
        return;
    }
    // A quadratic and its chord bound a convex region; a cubic may cross its chord.
    push_fill(ctx, &points, drawing.kind == DrawingKind::Curve, parts);
    parts.capped_polyline(
        drawing,
        &stroke,
        (!extend_start, !extend_end),
        (start_toward, end_toward),
        ctx.scale,
        false,
    );
}

/// The box of the shapes that reach beyond or differ from their anchors' box, in the px of `px`:
/// the reference of their box-layout text and, in media px, the paint box of the full-extent
/// ones.
fn shape_box(kind: DrawingKind, px: &[Point]) -> Option<Rect> {
    match kind {
        DrawingKind::RotatedRectangle => Rect::bounding(&rotated_rectangle(px)?),
        DrawingKind::Circle => {
            let (center, radius) = circle(px)?;
            Some(Rect {
                left: center.0 - radius,
                top: center.1 - radius,
                right: center.0 + radius,
                bottom: center.1 + radius,
            })
        }
        DrawingKind::Arc => {
            let (&a, &b, &through) = (px.first()?, px.get(1)?, px.get(2)?);
            let Some(outline) = arc_through(a, b, through) else {
                return Rect::bounding(&[a, b]);
            };
            // The ends plus every axis extreme the sweep passes.
            let mut points = vec![a, b];
            for quarter in 0..4 {
                let angle = f64::from(quarter) * std::f64::consts::FRAC_PI_2;
                if on_sweep(&outline, angle) {
                    points.push((
                        outline.center.0 + outline.rx * angle.cos(),
                        outline.center.1 + outline.rx * angle.sin(),
                    ));
                }
            }
            Rect::bounding(&points)
        }
        _ => None,
    }
}

/// Reach of end caps beyond the anchors, in CSS px: an arrowhead is twice as long as its
/// half-width.
fn decoration_extent(_engine: &ChartEngine, drawing: &Drawing) -> f64 {
    let open = match drawing.kind {
        DrawingKind::Arc | DrawingKind::Curve | DrawingKind::DoubleCurve => true,
        DrawingKind::Polyline => !options(drawing).closed,
        _ => false,
    };
    let capped =
        drawing.stroke_start != DrawingLineCap::None || drawing.stroke_end != DrawingLineCap::None;
    if open && capped {
        2.0 * cap_radius(drawing.width)
    } else {
        0.0
    }
}

fn extend_schema(template: &Drawing, properties: &mut Vec<DrawingPropertyDescriptor>) {
    if template.kind == DrawingKind::Polyline {
        properties.push(DrawingPropertyDescriptor {
            name: "tool_options.shape.closed".to_string(),
            property_type: DrawingPropertyType::Boolean,
            default: serde_json::json!(ShapeToolOptions::default().closed),
            min: None,
            max: None,
            enum_values: Vec::new(),
        });
    }
}

fn kind_options(drawing: &Drawing) -> DrawingKindOptions {
    DrawingKindOptions::Shape {
        closed: options(drawing).closed,
    }
}

#[cfg(test)]
mod tests;
