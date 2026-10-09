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
//! - rotated rectangle: a width handle on the near side's midpoint (`Handle(0)`, beside
//!   upstream's third handle on the far side's); dragging an edge corner keeps the on-screen
//!   width ([`derived_handles`], [`drag_handle`], [`follow_anchor_drag`]).
//! - arc, curve and double curve: placed by their ends first, then the points they pass through
//!   ([`placement_anchors`] reorders the clicks into upstream's stored order; upstream's curve
//!   anchors are points on the curve).

use std::borrow::Cow;

use aeris_charts_render::shape::{self, Point, Rect};

use super::super::Drawing;
use super::super::geometry::{DrawingBodyGeometry, rotated_rectangle_corners};
use super::super::handles::{DrawingHandle, HandleDrag};
use super::super::parts::{DrawingParts, cap_radius};
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

/// The derived handles of the shapes (see `kinds::upstream_derived_handles`; `px` are the
/// anchors' media px): a rotated rectangle gains a width handle `Handle(0)` at its near side's
/// midpoint (upstream's projection already put the depth anchor's handle on the far side's); a
/// zero-length edge keeps the plain anchors.
pub(crate) fn derived_handles(drawing: &Drawing, px: &[Point], handles: &mut Vec<DrawingHandle>) {
    if drawing.kind != DrawingKind::RotatedRectangle {
        return;
    }
    let Some(corners) = rotated_rectangle_corners(px) else {
        return;
    };
    let Some(&far) = handles
        .iter()
        .find(|handle| handle.part == DrawingDragPart::Anchor(2))
    else {
        return;
    };
    handles.push(DrawingHandle {
        point: shape::midpoint(corners[0], corners[1]),
        part: DrawingDragPart::Handle(0),
        ..far
    });
}

/// One drag sample of a shape's derived handle (see `kinds::drag_derived_handle`): the rotated
/// rectangle's width handle moves its edge (anchors 0 and 1) perpendicular to itself by the
/// target's move across it, so the far side stays. `None` rejects a sample whose anchors cannot
/// be placed.
pub(crate) fn drag_handle(
    engine: &ChartEngine,
    drawing: &Drawing,
    sample: &HandleDrag<'_>,
    points: &mut [DrawingPoint],
) -> Option<Option<DrawingToolOptions>> {
    if drawing.kind != DrawingKind::RotatedRectangle
        || sample.part != DrawingDragPart::Handle(0)
        || points.len() != 3
    {
        return Some(None);
    }
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
    Some(None)
}

/// After an anchor drag sample moved `points[index]` of a shape (from the baseline anchors at
/// media px `start_px`), re-derive what keeps the shape's on-screen construction: dragging an
/// edge corner of a rotated rectangle re-places its depth point on the new edge's far side at
/// the baseline width (so turning the edge, even through a zero-length one, keeps the width).
/// Leaves `points` unchanged when the new geometry cannot be placed.
pub(crate) fn follow_anchor_drag(
    engine: &ChartEngine,
    drawing: &Drawing,
    index: usize,
    start_px: &[Point],
    points: &mut [DrawingPoint],
) {
    if drawing.kind != DrawingKind::RotatedRectangle || index >= 2 || points.len() != 3 {
        return;
    }
    let (Some(&a), Some(&b), Some(&c)) = (start_px.first(), start_px.get(1), start_px.get(2))
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

/// Whether a `kind` is placed by its ends first, in an order other than the one its anchors are
/// stored in (see [`placement_anchors`]).
pub(crate) fn places_through(kind: DrawingKind) -> bool {
    matches!(
        kind,
        DrawingKind::Arc | DrawingKind::Curve | DrawingKind::DoubleCurve
    )
}

/// The anchors a completed click placement of a [`places_through`] tool stores, in upstream's
/// order (start, the points the curve passes through, end): an arc's clicks (start, end, a point
/// it passes through) and a curve's (start, end, its point at t = 1/2) become start, through
/// point, end; a double curve's (start, end, its points at t = 1/3 and 2/3) become start, the two
/// points, end. The commit and the placement preview both reorder here. `None` for another kind
/// or a placement that is not complete.
pub(crate) fn placement_anchors(
    kind: DrawingKind,
    clicks: &[DrawingPoint],
) -> Option<Vec<DrawingPoint>> {
    match (kind, clicks) {
        (DrawingKind::Arc | DrawingKind::Curve, &[a, b, through]) => Some(vec![a, through, b]),
        (DrawingKind::DoubleCurve, &[a, b, p, q]) => Some(vec![a, p, q, b]),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
