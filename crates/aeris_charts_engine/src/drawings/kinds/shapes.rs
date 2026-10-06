//! B8 Shapes options, legacy defaults, and the fork features re-applied on upstream's shape
//! renderers. Upstream renders the rotated rectangle, ellipse, circle, triangle, arc, curve,
//! double curve, polyline, and highlighter from its catalog spec (`geometry.rs` body resolver,
//! frame arm, hit code); this module keeps the fork's public option block ([`ShapeToolOptions`]),
//! for documents the fork wrote its pre-merge kind defaults ([`legacy_defaults`]), and what
//! upstream's arms read from it:
//!
//! - polyline: `tool_options.shape.closed` joins the last vertex to the first ([`closed`]); the
//!   arm fills the enclosed region by the nonzero rule while `fill_enabled` and paints no caps.
//!   Clicking the first vertex once three are placed closes it (the engine's placement).
//! - arc, curve, double curve: the region between the curve and its chord fills while
//!   `fill_enabled`, the curves continue their end tangents by `extend_*`, and
//!   `stroke_start`/`stroke_end` cap the unextended ends ([`CurveStroke`], [`capped`]).
//! - rotated rectangle: its third handle sits on the far side's midpoint and a width handle on
//!   the near side's (`Handle(0)`); dragging an edge corner keeps the on-screen width
//!   ([`derived_handles`], [`drag_handle`], [`follow_anchor_drag`]).
//! - curve and double curve: placed and edited through points on the curve (the quadratic's at
//!   t = 1/2, the cubic's at t = 1/3 and 2/3), stored as upstream's control points
//!   ([`placement_anchors`], the derived `Handle` parts); an arc is placed by its ends first.

use std::borrow::Cow;

use aeris_charts_render::shape::{self, Point, Rect};

use super::super::geometry::{rotated_rectangle_corners, CurveGeometry, DrawingBodyGeometry};
use super::super::handles::{DrawingHandle, HandleDrag};
use super::super::parts::{cap_radius, DrawingParts};
use super::super::Drawing;
use crate::{
    ChartEngine, DrawingDragPart, DrawingKind, DrawingLineCap, DrawingPoint,
    DrawingPropertyDescriptor, DrawingPropertyType, DrawingToolOptions,
};

/// The fork's shapes options (`tool_options.shape`); absent fields keep their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShapeToolOptions {
    /// Polyline only: join the last vertex back to the first and, while `fill_enabled`, fill the
    /// enclosed region (nonzero rule; outline only beyond
    /// [`aeris_charts_render::shape::MAX_FILL_VERTICES`] vertices or the fill's crossing and rung
    /// bounds). The other shapes ignore it.
    pub closed: bool,
}

/// The fork's highlighter opacity over the canonical market-warning amber.
const HIGHLIGHTER_OPACITY: f64 = 0.4;

/// The fork's pre-merge defaults of the shape tools (see [`super::apply_legacy_fork_defaults`]):
/// closed shapes filled, and the highlighter in translucent amber.
pub(super) fn legacy_defaults(drawing: &mut Drawing) {
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

/// Whether `drawing` is a closed polyline (`tool_options.shape.closed`).
pub(crate) fn closed(drawing: &Drawing) -> bool {
    drawing.kind == DrawingKind::Polyline
        && drawing.tool_options.shape.is_some_and(|shape| shape.closed)
}

/// Whether `drawing` is an arc or curve with an end cap set: its stroke is then lowered and hit
/// through the shared capped stroke (`DrawingParts::capped_polyline`).
pub(crate) fn capped(drawing: &Drawing) -> bool {
    matches!(
        drawing.kind,
        DrawingKind::Arc | DrawingKind::Curve | DrawingKind::DoubleCurve
    ) && (drawing.stroke_start != DrawingLineCap::None
        || drawing.stroke_end != DrawingLineCap::None)
}

/// An arc's or curve's stroke resolved from its body in the caller's px (bitmap px in the frame,
/// media px in hit testing), so the frame and the hit test read one construction: the curve
/// flattened against a clip, the tangent extensions the stroke adds, the chord-region fill and
/// the end caps.
pub(crate) struct CurveStroke {
    /// The arc or curve flattened against `clip` (`ArcGeometry::flatten`,
    /// `CurveGeometry::flatten`).
    line: Vec<Point>,
    /// Where `extend_left`/`extend_right` continue the end tangents (curves only).
    extend: [Option<Point>; 2],
    /// Whether the region between the curve and its chord is convex (an arc, a quadratic).
    convex: bool,
    /// The points the caps point away from: the exact end tangents.
    towards: [Option<Point>; 2],
    clip: Rect,
}

impl CurveStroke {
    /// The stroke of an `Arc` or `Curve` body flattened against `clip`; `None` for any other
    /// body.
    pub(crate) fn resolve(body: DrawingBodyGeometry<'_>, clip: Rect) -> Option<Self> {
        let mut line = Vec::new();
        let (extend, convex, towards) = match body {
            DrawingBodyGeometry::Arc(arc) => {
                arc.flatten(clip, &mut line);
                let [start, end] = arc.end_towards();
                ([None; 2], true, [Some(start), Some(end)])
            }
            DrawingBodyGeometry::Curve(curve) => {
                curve.flatten(clip, &mut line);
                (curve.extend, !curve.cubic, curve.end_towards())
            }
            _ => return None,
        };
        Some(Self {
            line,
            extend,
            convex,
            towards,
            clip,
        })
    }

    /// The stroked run: the curve between its tangent extensions.
    pub(crate) fn run(&self) -> Cow<'_, [Point]> {
        match self.extend {
            [None, None] => Cow::Borrowed(&self.line),
            [start, end] => Cow::Owned(
                start
                    .into_iter()
                    .chain(self.line.iter().copied())
                    .chain(end)
                    .collect(),
            ),
        }
    }

    /// Whether the stroked run `run` (see [`Self::run`]) meets the clip: an off-screen arc or
    /// curve paints nothing.
    pub(crate) fn meets_clip(&self, run: &[Point]) -> bool {
        meets(run, self.clip)
    }

    /// The chord-region fill, never covering a tangent extension: the ribbon chains in `out`,
    /// each the returned count long (0: nothing to fill, also when the curve misses the clip).
    /// An arc's or a quadratic's region against its chord is convex; a cubic may cross its chord,
    /// so it fills by the nonzero rule (0 beyond that fill's bounds). The frame paints this ribbon
    /// and the selected-fill hit tests it, so both agree.
    pub(crate) fn chord_fill(&self, out: &mut Vec<Point>) -> usize {
        if !meets(&self.line, self.clip) {
            0
        } else if self.convex {
            shape::convex_ribbon(&self.line, out)
        } else {
            shape::nonzero_ribbon(&self.line, out)
        }
    }

    /// The stroked run `run` with `drawing`'s end caps on its unextended ends along the exact
    /// end tangents, at `scale` caller px per CSS px (the shared capped stroke: the arrow trims
    /// the stroke under it, and the caps are body targets).
    pub(crate) fn capped_parts(
        &self,
        run: &[Point],
        drawing: &Drawing,
        scale: f64,
        parts: &mut DrawingParts,
    ) {
        let [start, end] = self.towards;
        parts.capped_polyline(
            drawing,
            run,
            (self.extend[0].is_none(), self.extend[1].is_none()),
            (start, end),
            scale,
            false,
        );
    }
}

/// Whether `points`' bounds meet `clip`.
fn meets(points: &[Point], clip: Rect) -> bool {
    Rect::bounding(points).is_some_and(|bounds| bounds.intersects(&clip))
}

/// The culling pad an arc's or curve's end caps reach beyond its anchors (CSS px; see
/// `kinds::upstream_decoration_extent`): an arrowhead is twice as long as its half-width.
pub(crate) fn upstream_decoration_extent(_engine: &ChartEngine, drawing: &Drawing) -> f64 {
    if capped(drawing) {
        2.0 * cap_radius(drawing.width)
    } else {
        0.0
    }
}

/// The `tool_options.shape` descriptor the polyline reads on upstream's arm (see
/// `kinds::extend_upstream_schema`).
pub(crate) fn extend_upstream_schema(
    kind: DrawingKind,
    properties: &mut Vec<DrawingPropertyDescriptor>,
) {
    if kind == DrawingKind::Polyline {
        properties.push(crate::drawing_contract::descriptor(
            "tool_options.shape.closed".to_string(),
            DrawingPropertyType::Boolean,
            serde_json::json!(ShapeToolOptions::default().closed),
        ));
    }
}

/// The quadratic control point of the curve from `a` to `b` through `m` at t = 1/2.
pub(crate) fn quadratic_through(a: Point, b: Point, m: Point) -> Point {
    (2.0 * m.0 - (a.0 + b.0) / 2.0, 2.0 * m.1 - (a.1 + b.1) / 2.0)
}

/// The cubic control points of the curve from `a` to `b` through `p` at t = 1/3 and `q` at
/// t = 2/3.
pub(crate) fn cubic_through(a: Point, b: Point, p: Point, q: Point) -> (Point, Point) {
    let control = |near: Point, far: Point, through_near: Point, through_far: Point| {
        let axis = |i: fn(Point) -> f64| {
            3.0 * i(through_near) - 1.5 * i(through_far) - 5.0 / 6.0 * i(near) + i(far) / 3.0
        };
        (axis(|point| point.0), axis(|point| point.1))
    };
    (control(a, b, p, q), control(b, a, q, p))
}

/// The points on a curve with anchors at `px` (start, control(s), end) that its on-curve handles
/// sit at: the quadratic's at t = 1/2, the cubic's at t = 1/3 and 2/3.
fn on_curve_points(kind: DrawingKind, px: &[Point]) -> Option<Vec<Point>> {
    let (cubic, count) = match kind {
        DrawingKind::Curve => (false, 3),
        DrawingKind::DoubleCurve => (true, 4),
        _ => return None,
    };
    let px = px.get(..count)?;
    let curve = CurveGeometry {
        points: [px[0], px[1], px[2], px[count - 1]],
        cubic,
        extend: [None; 2],
    };
    Some(if cubic {
        vec![curve.point(1.0 / 3.0), curve.point(2.0 / 3.0)]
    } else {
        vec![curve.point(0.5)]
    })
}

/// The control points of the curve from `a` to `b` through `through` (one point for the
/// quadratic, two for the cubic, see [`on_curve_points`]).
fn controls_through(a: Point, b: Point, through: &[Point]) -> Vec<Point> {
    match *through {
        [m] => vec![quadratic_through(a, b, m)],
        [p, q] => {
            let (first, second) = cubic_through(a, b, p, q);
            vec![first, second]
        }
        _ => Vec::new(),
    }
}

/// The derived handles of the shapes (see `kinds::upstream_derived_handles`; `px` are the
/// anchors' media px). A rotated rectangle's third handle moves to the midpoint of its far side
/// (the depth point itself may lie anywhere on that side's line) and a width handle `Handle(0)`
/// is appended at the near side's midpoint; a zero-length edge keeps the plain anchors. A curve's
/// control handles become `Handle(k)` at its on-curve points, in their keyboard places.
pub(crate) fn derived_handles(drawing: &Drawing, px: &[Point], handles: &mut Vec<DrawingHandle>) {
    match drawing.kind {
        DrawingKind::RotatedRectangle => {
            let Some(corners) = rotated_rectangle_corners(px) else {
                return;
            };
            let Some(index) = handles
                .iter()
                .position(|handle| handle.part == DrawingDragPart::Anchor(2))
            else {
                return;
            };
            handles[index].point = shape::midpoint(corners[2], corners[3]);
            handles.push(DrawingHandle {
                point: shape::midpoint(corners[0], corners[1]),
                part: DrawingDragPart::Handle(0),
                ..handles[index]
            });
        }
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let Some(through) = on_curve_points(drawing.kind, px) else {
                return;
            };
            for (index, point) in through.into_iter().enumerate() {
                if let Some(handle) = handles
                    .iter_mut()
                    .find(|handle| handle.part == DrawingDragPart::Anchor(index + 1))
                {
                    handle.point = point;
                    handle.part = DrawingDragPart::Handle(index);
                }
            }
        }
        _ => {}
    }
}

/// One drag sample of a shape's derived handle (see `kinds::drag_derived_handle`): the rotated
/// rectangle's width handle moves its edge (anchors 0 and 1) perpendicular to itself by the
/// target's move across it, so the far side stays; a curve's on-curve handle makes the curve pass
/// through the target with its ends fixed, re-solving the control points from the baseline's
/// other on-curve point (control points are not time-snapped). `None` rejects a sample whose
/// anchors cannot be placed.
pub(crate) fn drag_handle(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [DrawingPoint],
) -> Option<Option<DrawingToolOptions>> {
    let DrawingDragPart::Handle(handle) = sample.part else {
        return Some(None);
    };
    match drawing.kind {
        DrawingKind::RotatedRectangle if handle == 0 && points.len() == 3 => {
            let (&a, &b) = (sample.start_px.first()?, sample.start_px.get(1)?);
            let normal = shape::segment_normal(a, b)?;
            let delta = (sample.target_px.0 - sample.handle_px.0) * normal.0
                + (sample.target_px.1 - sample.handle_px.1) * normal.1;
            let shift = |point: Point| (point.0 + normal.0 * delta, point.1 + normal.1 * delta);
            let (moved_a, moved_b) = (
                engine.drawing_anchor_at(drawing, shift(a))?,
                engine.drawing_anchor_at(drawing, shift(b))?,
            );
            (points[0], points[1]) = (moved_a, moved_b);
        }
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let mut through = on_curve_points(drawing.kind, sample.start_px)?;
            let last = through.len() + 1;
            if handle >= through.len() || points.len() != last + 1 {
                return Some(None);
            }
            through[handle] = sample.target_px;
            let controls = controls_through(sample.start_px[0], sample.start_px[last], &through);
            set_controls(engine, drawing, &controls, points)?;
        }
        _ => {}
    }
    Some(None)
}

/// Store the curve control points `controls` (media px) as `points[1..]`, without time snapping.
fn set_controls(
    engine: &ChartEngine,
    drawing: &Drawing,
    controls: &[Point],
    points: &mut [DrawingPoint],
) -> Option<()> {
    let converted = controls
        .iter()
        .map(|&(x, y)| engine.drawing_from_px_for(drawing.pane_index, drawing.price_scale, x, y))
        .collect::<Option<Vec<_>>>()?;
    points
        .get_mut(1..=converted.len())?
        .copy_from_slice(&converted);
    Some(())
}

/// After an anchor drag sample moved `points[index]` of a shape (from the baseline anchors at
/// media px `start_px`), re-derive what keeps the shape's on-screen construction: dragging an
/// edge corner of a rotated rectangle re-places its depth point on the new edge's far side at
/// the baseline width (so turning the edge, even through a zero-length one, keeps the width);
/// dragging a curve's end re-solves its control points so the baseline on-curve points stay.
/// Leaves `points` unchanged when the new geometry cannot be placed.
pub(crate) fn follow_anchor_drag(
    engine: &ChartEngine,
    drawing: &Drawing,
    index: usize,
    start_px: &[Point],
    points: &mut [DrawingPoint],
) {
    match drawing.kind {
        DrawingKind::RotatedRectangle if index < 2 && points.len() == 3 => {
            let (Some(&a), Some(&b), Some(&c)) =
                (start_px.first(), start_px.get(1), start_px.get(2))
            else {
                return;
            };
            let Some(normal) = shape::segment_normal(a, b) else {
                return;
            };
            let depth = (c.0 - a.0) * normal.0 + (c.1 - a.1) * normal.1;
            let (Some(a), Some(b)) = (
                engine.drawing_point_px(drawing, points[0]),
                engine.drawing_point_px(drawing, points[1]),
            ) else {
                return;
            };
            let Some(normal) = shape::segment_normal(a, b) else {
                return;
            };
            let middle = shape::midpoint(a, b);
            if let Some(point) = engine.drawing_from_px_for(
                drawing.pane_index,
                drawing.price_scale,
                middle.0 + normal.0 * depth,
                middle.1 + normal.1 * depth,
            ) {
                points[2] = point;
            }
        }
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let Some(through) = on_curve_points(drawing.kind, start_px) else {
                return;
            };
            let last = through.len() + 1;
            if (index != 0 && index != last) || points.len() != last + 1 {
                return;
            }
            let Some(moved) = engine.drawing_point_px(drawing, points[index]) else {
                return;
            };
            let (a, b) = if index == 0 {
                (moved, start_px[last])
            } else {
                (start_px[0], moved)
            };
            let mut edited = points.to_vec();
            if set_controls(
                engine,
                drawing,
                &controls_through(a, b, &through),
                &mut edited,
            )
            .is_some()
            {
                points.copy_from_slice(&edited);
            }
        }
        _ => {}
    }
}

/// Whether the clicks placing a `kind` are points on its geometry rather than its stored anchors
/// (see [`placement_anchors`]).
pub(crate) fn places_through(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::Arc | DrawingKind::Curve | DrawingKind::DoubleCurve
    )
}

/// The anchors a completed click placement of a [`places_through`] tool stores: an arc's clicks
/// (start, end, a point it passes through) reorder into upstream's start, through point, end; a
/// curve's (start, end, its point at t = 1/2) and a double curve's (start, end, its points at
/// t = 1/3 and 2/3) solve upstream's control points in the drawing's px, without time snapping.
/// The commit and the placement preview both convert here. `None` when a click has no px.
pub(crate) fn placement_anchors(
    engine: &ChartEngine,
    drawing: &Drawing,
    clicks: &[DrawingPoint],
) -> Option<Vec<DrawingPoint>> {
    match (drawing.kind, clicks) {
        (DrawingKind::Arc, &[a, b, through]) => Some(vec![a, through, b]),
        (DrawingKind::Curve, &[a, b, _]) | (DrawingKind::DoubleCurve, &[a, b, _, _]) => {
            let px = clicks
                .iter()
                .map(|&point| engine.drawing_point_px(drawing, point))
                .collect::<Option<Vec<_>>>()?;
            let mut points = vec![a; clicks.len()];
            set_controls(
                engine,
                drawing,
                &controls_through(px[0], px[1], &px[2..]),
                &mut points,
            )?;
            *points.last_mut()? = b;
            Some(points)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
