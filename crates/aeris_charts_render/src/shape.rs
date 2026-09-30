//! Backend-neutral 2-D shape geometry shared by drawing tools.
//!
//! Segment extension and clipping, polyline and polygon clipping, parallel offsets, uniform
//! arc/ellipse tessellation (at most 256 chords) and clip-aware curve flattening (a 1,024-point
//! budget, refined only where the curve can be visible), nonzero-winding ribbon fills, tube
//! outlines, polyline simplification, and the hit-test predicates every drawing family uses.
//! All functions are plain `f64`
//! math in the caller's coordinate space (media px for hit testing, bitmap px for frame emission,
//! screen y growing downward); nothing here snaps to device pixels. Curves become point lists that
//! the caller emits through the existing `Prim::Polyline` / `Prim::BandFill` contracts, so every
//! executor paints the same tessellation.

/// A point `(x, y)` in the caller's coordinate space.
pub type Point = (f64, f64);

/// Axis-aligned rectangle with `left <= right` and `top <= bottom` (screen y grows downward).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    pub fn contains(&self, point: Point) -> bool {
        point.0 >= self.left
            && point.0 <= self.right
            && point.1 >= self.top
            && point.1 <= self.bottom
    }

    /// The rectangle grown by `by` on every side.
    pub fn inflate(&self, by: f64) -> Self {
        Self {
            left: self.left - by,
            top: self.top - by,
            right: self.right + by,
            bottom: self.bottom + by,
        }
    }

    /// Whether the two closed rectangles share at least one point.
    pub fn intersects(&self, other: &Self) -> bool {
        self.left <= other.right
            && self.right >= other.left
            && self.top <= other.bottom
            && self.bottom >= other.top
    }

    /// Bounds of `points`; `None` when empty or any coordinate is not finite.
    pub fn bounding(points: &[Point]) -> Option<Self> {
        let (&first, rest) = points.split_first()?;
        let mut bounds = Self {
            left: first.0,
            top: first.1,
            right: first.0,
            bottom: first.1,
        };
        for &(x, y) in rest {
            bounds.left = bounds.left.min(x);
            bounds.right = bounds.right.max(x);
            bounds.top = bounds.top.min(y);
            bounds.bottom = bounds.bottom.max(y);
        }
        // `min`/`max` skip NaN, so finiteness is checked per point.
        points
            .iter()
            .all(|point| point.0.is_finite() && point.1.is_finite())
            .then_some(bounds)
    }
}

/// Parametric interval `[t0, t1]` of the infinite line `a + t·(b − a)` that lies inside `rect`
/// (Liang–Barsky). `None` when the line misses the rectangle or `a == b`.
pub fn line_rect_interval(a: Point, b: Point, rect: Rect) -> Option<(f64, f64)> {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    if dx.abs() <= f64::EPSILON && dy.abs() <= f64::EPSILON {
        return None;
    }
    let mut t0 = f64::NEG_INFINITY;
    let mut t1 = f64::INFINITY;
    for (delta, low, high, start) in [
        (dx, rect.left, rect.right, a.0),
        (dy, rect.top, rect.bottom, a.1),
    ] {
        if delta.abs() <= f64::EPSILON {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let first = (low - start) / delta;
        let second = (high - start) / delta;
        t0 = t0.max(first.min(second));
        t1 = t1.min(first.max(second));
    }
    (t0 <= t1).then_some((t0, t1))
}

/// The segment `a → b` extended beyond `a` and/or beyond `b` to the rectangle boundary. An
/// extension direction that never reaches the rectangle keeps its anchor, and a degenerate
/// segment is returned unchanged, so the result always contains the original segment.
pub fn extend_segment(
    a: Point,
    b: Point,
    rect: Rect,
    beyond_a: bool,
    beyond_b: bool,
) -> (Point, Point) {
    if !beyond_a && !beyond_b {
        return (a, b);
    }
    let Some((t0, t1)) = line_rect_interval(a, b, rect) else {
        return (a, b);
    };
    let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let start = if beyond_a && t0 < 0.0 { at(t0) } else { a };
    let end = if beyond_b && t1 > 1.0 { at(t1) } else { b };
    (start, end)
}

/// The midpoint of `a` and `b`.
pub fn midpoint(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// Unit normal of `a → b` pointing to the left of the direction on screen (`(dy, −dx)` rotated
/// into y-down space: "above" a left-to-right segment). `None` for a degenerate segment.
pub fn segment_normal(a: Point, b: Point) -> Option<Point> {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let length = dx.hypot(dy);
    (length > f64::EPSILON).then(|| (dy / length, -dx / length))
}

/// The segment `a → b` translated perpendicular to itself by `distance` along
/// [`segment_normal`]. A degenerate segment is returned unchanged.
pub fn offset_segment(a: Point, b: Point, distance: f64) -> (Point, Point) {
    let Some((nx, ny)) = segment_normal(a, b) else {
        return (a, b);
    };
    let (ox, oy) = (nx * distance, ny * distance);
    ((a.0 + ox, a.1 + oy), (b.0 + ox, b.1 + oy))
}

/// The segment with the direction and length of `a → b` that starts at `through`.
pub fn parallel_through(a: Point, b: Point, through: Point) -> (Point, Point) {
    (through, (through.0 + b.0 - a.0, through.1 + b.1 - a.1))
}

/// Upper bound on the chords of one tessellated arc, keeping frame work bounded for huge radii.
pub const MAX_ARC_SEGMENTS: usize = 256;

/// Chords needed so a circular arc of `radius` and `sweep` radians deviates from the true curve
/// by at most `tolerance` (same units as the radius), between 1 and [`MAX_ARC_SEGMENTS`].
pub fn arc_segment_count(radius: f64, sweep: f64, tolerance: f64) -> usize {
    if !radius.is_finite() || !sweep.is_finite() || radius <= 0.0 || sweep == 0.0 {
        return 1;
    }
    let tolerance = tolerance.max(1e-3).min(radius);
    let step = 2.0 * (1.0 - tolerance / radius).clamp(-1.0, 1.0).acos();
    if step <= f64::EPSILON {
        return MAX_ARC_SEGMENTS;
    }
    ((sweep.abs() / step).ceil() as usize).clamp(1, MAX_ARC_SEGMENTS)
}

/// An elliptical arc: `center`, radii `rx`/`ry`, the ellipse `rotation`, and a `start` angle plus a
/// signed `sweep` in radians (screen space, so a positive sweep turns clockwise on screen). A full
/// ellipse or circle is `sweep = ±2π`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipseArc {
    pub center: Point,
    pub rx: f64,
    pub ry: f64,
    pub rotation: f64,
    pub start: f64,
    pub sweep: f64,
}

impl EllipseArc {
    /// A circular arc of `radius` around `center`.
    pub fn circle(center: Point, radius: f64, start: f64, sweep: f64) -> Self {
        Self {
            center,
            rx: radius,
            ry: radius,
            rotation: 0.0,
            start,
            sweep,
        }
    }

    /// Append the arc's points to `out`, both end points included, with the chord count of
    /// [`arc_segment_count`] on the larger radius (a full turn repeats its first point last).
    /// Non-finite input appends nothing.
    pub fn append_points(&self, tolerance: f64, out: &mut Vec<Point>) {
        let values = [
            self.center.0,
            self.center.1,
            self.rx,
            self.ry,
            self.rotation,
            self.start,
            self.sweep,
        ];
        if !values.iter().all(|value| value.is_finite()) {
            return;
        }
        let segments = arc_segment_count(self.rx.abs().max(self.ry.abs()), self.sweep, tolerance);
        let (sin_r, cos_r) = self.rotation.sin_cos();
        out.reserve(segments + 1);
        for step in 0..=segments {
            let angle = self.start + self.sweep * step as f64 / segments as f64;
            let (sin_a, cos_a) = angle.sin_cos();
            let (x, y) = (self.rx * cos_a, self.ry * sin_a);
            out.push((
                self.center.0 + x * cos_r - y * sin_r,
                self.center.1 + x * sin_r + y * cos_r,
            ));
        }
    }
}

/// Shortest distance from `point` to the segment `a → b` (the reference `distanceToSegment`).
pub fn distance_to_segment(point: Point, a: Point, b: Point) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    if dx == 0.0 && dy == 0.0 {
        return (point.0 - a.0).hypot(point.1 - a.1);
    }
    let t = (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (point.0 - (a.0 + dx * t)).hypot(point.1 - (a.1 + dy * t))
}

/// Shortest distance from `point` to an open polyline (`INFINITY` when empty; the point
/// distance for a single vertex).
pub fn distance_to_polyline(point: Point, points: &[Point]) -> f64 {
    match points {
        [] => f64::INFINITY,
        [only] => (point.0 - only.0).hypot(point.1 - only.1),
        _ => points
            .windows(2)
            .map(|pair| distance_to_segment(point, pair[0], pair[1]))
            .fold(f64::INFINITY, f64::min),
    }
}

/// Even-odd containment of `point` in a closed polygon (the last vertex joins the first).
pub fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = polygon[polygon.len() - 1];
    for &current in polygon {
        if (current.1 > point.1) != (previous.1 > point.1) {
            let x = current.0
                + (point.1 - current.1) * (previous.0 - current.0) / (previous.1 - current.1);
            if point.0 < x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Containment in the ribbon between two paired chains — the region `Prim::BandFill` paints:
/// the union of the quads `(upper[i], lower[i], lower[i+1], upper[i+1])`.
pub fn point_in_ribbon(point: Point, upper: &[Point], lower: &[Point]) -> bool {
    let count = upper.len().min(lower.len());
    (1..count).any(|index| {
        point_in_polygon(
            point,
            &[
                upper[index - 1],
                lower[index - 1],
                lower[index],
                upper[index],
            ],
        )
    })
}

/// Split a convex polygon into the two paired chains of a ribbon that covers exactly the
/// polygon: both chains start at vertex 0 and walk opposite ways around the outline, the shorter
/// one repeating its last vertex. Appends `upper` then `lower` chains of equal length to `out`
/// and returns that length (0 for fewer than three vertices).
pub fn convex_ribbon(polygon: &[Point], out: &mut Vec<Point>) -> usize {
    let count = polygon.len();
    if count < 3 {
        return 0;
    }
    // Forward chain 0..=m and backward chain 0, n-1, .., m (m = n / 2) meet at vertex m.
    let middle = count / 2;
    let upper_len = middle + 1;
    let lower_len = count - middle + 1;
    let length = upper_len.max(lower_len);
    out.reserve(length * 2);
    for index in 0..length {
        out.push(polygon[index.min(middle)]);
    }
    for index in 0..length {
        let step = index.min(lower_len - 1);
        out.push(polygon[(count - step) % count]);
    }
    length
}

/// Clip a polygon to an axis-aligned rectangle (Sutherland–Hodgman), replacing `out` with the
/// clipped outline. A convex polygon stays convex, crossings land exactly on the rectangle's
/// edges, and nothing remains when the polygon misses the rectangle (or holds non-finite points).
pub fn clip_polygon_to_rect(polygon: &[Point], rect: Rect, out: &mut Vec<Point>) {
    out.clear();
    if !polygon
        .iter()
        .all(|point| point.0.is_finite() && point.1.is_finite())
    {
        return;
    }
    out.extend_from_slice(polygon);
    let mut input = Vec::with_capacity(polygon.len() + 4);
    for edge in 0..4 {
        std::mem::swap(out, &mut input);
        out.clear();
        let Some(&last) = input.last() else {
            return;
        };
        let inside = |p: Point| match edge {
            0 => p.0 >= rect.left,
            1 => p.0 <= rect.right,
            2 => p.1 >= rect.top,
            _ => p.1 <= rect.bottom,
        };
        // Only called across the edge, so the divisor is never zero.
        let cut = |a: Point, b: Point| match edge {
            0 | 1 => {
                let x = if edge == 0 { rect.left } else { rect.right };
                (x, a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0))
            }
            _ => {
                let y = if edge == 2 { rect.top } else { rect.bottom };
                (a.0 + (b.0 - a.0) * (y - a.1) / (b.1 - a.1), y)
            }
        };
        let mut previous = last;
        for &current in &input {
            match (inside(previous), inside(current)) {
                (true, true) => out.push(current),
                (true, false) => out.push(cut(previous, current)),
                (false, true) => {
                    out.push(cut(previous, current));
                    out.push(current);
                }
                (false, false) => {}
            }
            previous = current;
        }
    }
}

/// Clip a convex polygon to the closed half-plane on the side of the directed line `a → b`
/// where `(b − a) × (p − a) >= 0` (the right-hand side on a y-down screen), replacing `out` with
/// the clipped convex polygon (Sutherland–Hodgman). Intersecting a pane rectangle with two
/// half-planes through a ray fan's apex yields the convex region between two rays.
pub fn clip_to_half_plane(polygon: &[Point], a: Point, b: Point, out: &mut Vec<Point>) {
    out.clear();
    let side = |p: Point| (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
    let Some(&last) = polygon.last() else {
        return;
    };
    let mut previous = last;
    let mut previous_side = side(previous);
    for &current in polygon {
        let current_side = side(current);
        if (current_side >= 0.0) != (previous_side >= 0.0) {
            let t = previous_side / (previous_side - current_side);
            out.push((
                previous.0 + (current.0 - previous.0) * t,
                previous.1 + (current.1 - previous.1) * t,
            ));
        }
        if current_side >= 0.0 {
            out.push(current);
        }
        previous = current;
        previous_side = current_side;
    }
}

/// Clip an open polyline to `rect`, calling `run` with each maximal part inside it, in order.
/// With a positive `period` (a dash pattern's length), every part after the start begins a
/// little outside `rect`, on the line of its first segment, at a whole multiple of `period` of
/// arc length from the polyline's start, so a dash pattern restarted at each part keeps the
/// unclipped polyline's phase inside `rect`. Segments with non-finite points are dropped. The
/// work and the parts' extent stay bounded by the rectangle however far the polyline reaches,
/// and a polyline entirely inside `rect` is passed through unchanged without touching `scratch`.
pub fn clip_polyline_to_rect(
    points: &[Point],
    rect: Rect,
    period: f64,
    scratch: &mut Vec<Point>,
    mut run: impl FnMut(&[Point]),
) {
    if points.len() < 2 {
        return;
    }
    if points.iter().all(|&point| rect.contains(point)) {
        run(points);
        return;
    }
    scratch.clear();
    let mut flush = |scratch: &mut Vec<Point>| {
        if scratch.len() >= 2 {
            run(scratch);
        }
        scratch.clear();
    };
    let mut walked = 0.0_f64;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = dx.hypot(dy);
        if !length.is_finite() || !a.0.is_finite() || !a.1.is_finite() {
            flush(scratch);
            walked = f64::NAN;
            continue;
        }
        let visible = if length <= f64::EPSILON {
            rect.contains(a).then_some((0.0, 1.0))
        } else {
            line_rect_interval(a, b, rect)
                .map(|(t0, t1)| (t0.max(0.0), t1.min(1.0)))
                .filter(|(t0, t1)| t0 <= t1)
        };
        match visible {
            Some((t0, t1)) => {
                // Clamped so float cancellation on astronomically long segments cannot carry a
                // point away from the rectangle.
                let at = |t: f64, pad: f64| {
                    (
                        (a.0 + dx * t).clamp(rect.left - pad, rect.right + pad),
                        (a.1 + dy * t).clamp(rect.top - pad, rect.bottom + pad),
                    )
                };
                if scratch.is_empty() {
                    let distance = walked + t0 * length;
                    if period > 0.0 && distance.is_finite() && length > f64::EPSILON {
                        let back = distance.rem_euclid(period);
                        if back > 1e-9 {
                            scratch.push(at(t0 - back / length, period));
                        }
                    }
                    scratch.push(at(t0, 0.0));
                }
                if t1 < 1.0 {
                    scratch.push(at(t1, 0.0));
                    flush(scratch);
                } else {
                    scratch.push(b);
                }
            }
            None => flush(scratch),
        }
        walked += length;
    }
    flush(scratch);
}

/// Where the infinite lines `a → b` and `c → d` cross, as the parameters `(t, u)` of
/// `a + t·(b − a)` and `c + u·(d − c)`. `None` when either line is degenerate or they are
/// parallel.
pub fn line_intersection(a: Point, b: Point, c: Point, d: Point) -> Option<(f64, f64)> {
    let (rx, ry) = (b.0 - a.0, b.1 - a.1);
    let (sx, sy) = (d.0 - c.0, d.1 - c.1);
    let denominator = rx * sy - ry * sx;
    let scale = rx.hypot(ry) * sx.hypot(sy);
    if scale <= f64::EPSILON || denominator.abs() <= f64::EPSILON * scale {
        return None;
    }
    let (qx, qy) = (c.0 - a.0, c.1 - a.1);
    let t = (qx * sy - qy * sx) / denominator;
    let u = (qx * ry - qy * rx) / denominator;
    (t.is_finite() && u.is_finite()).then_some((t, u))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 100.0,
        bottom: 50.0,
    };

    fn close(a: Point, b: Point) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn extension_reaches_the_edge_in_each_requested_direction() {
        let (a, b) = ((20.0, 25.0), (40.0, 15.0));
        assert_eq!(extend_segment(a, b, PANE, false, false), (a, b));
        let (start, end) = extend_segment(a, b, PANE, false, true);
        assert!(close(start, a));
        assert!(
            close(end, (70.0, 0.0)),
            "ray exits through the top edge: {end:?}"
        );
        let (start, end) = extend_segment(a, b, PANE, true, true);
        assert!(close(start, (0.0, 35.0)));
        assert!(close(end, (70.0, 0.0)));
        // A right-to-left ray follows its own direction.
        let (_, end) = extend_segment(b, a, PANE, false, true);
        assert!(close(end, (0.0, 35.0)));
        // Vertical and degenerate segments.
        let (start, end) = extend_segment((10.0, 20.0), (10.0, 30.0), PANE, true, true);
        assert!(close(start, (10.0, 0.0)) && close(end, (10.0, 50.0)));
        assert_eq!(extend_segment(a, a, PANE, true, true), (a, a));
        // A ray pointing away from the pane keeps its anchor instead of reversing.
        let (start, end) = extend_segment((150.0, 20.0), (160.0, 20.0), PANE, true, true);
        assert!(close(start, (0.0, 20.0)));
        assert!(close(end, (160.0, 20.0)));
    }

    #[test]
    fn line_rect_interval_rejects_misses() {
        assert_eq!(line_rect_interval((0.0, 60.0), (10.0, 60.0), PANE), None);
        assert_eq!(line_rect_interval((0.0, 0.0), (0.0, 0.0), PANE), None);
        let (t0, t1) = line_rect_interval((50.0, 25.0), (60.0, 25.0), PANE).unwrap();
        assert!((t0 + 5.0).abs() < 1e-9 && (t1 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn offsets_are_parallel_and_perpendicular() {
        let (a, b) = offset_segment((0.0, 10.0), (10.0, 10.0), 4.0);
        assert!(
            close(a, (0.0, 6.0)) && close(b, (10.0, 6.0)),
            "positive offsets go above"
        );
        let (c, d) = parallel_through((0.0, 0.0), (3.0, 4.0), (10.0, 10.0));
        assert!(close(c, (10.0, 10.0)) && close(d, (13.0, 14.0)));
        assert_eq!(segment_normal((1.0, 1.0), (1.0, 1.0)), None);
    }

    #[test]
    fn arcs_respect_tolerance_and_stay_bounded() {
        let mut out = Vec::new();
        EllipseArc::circle((0.0, 0.0), 40.0, 0.0, std::f64::consts::FRAC_PI_2)
            .append_points(0.25, &mut out);
        assert!(close(out[0], (40.0, 0.0)));
        assert!(close(*out.last().unwrap(), (0.0, 40.0)));
        for pair in out.windows(2) {
            let mid = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
            assert!(
                40.0 - mid.0.hypot(mid.1) <= 0.25 + 1e-9,
                "chord error within tolerance"
            );
        }
        assert_eq!(
            arc_segment_count(1e12, std::f64::consts::TAU, 0.25),
            MAX_ARC_SEGMENTS
        );
        assert_eq!(arc_segment_count(0.0, 1.0, 0.25), 1);
        // Rotated ellipse: the major axis follows the rotation.
        out.clear();
        let ellipse = EllipseArc {
            center: (0.0, 0.0),
            rx: 10.0,
            ry: 5.0,
            rotation: std::f64::consts::FRAC_PI_2,
            start: 0.0,
            sweep: 0.0,
        };
        ellipse.append_points(0.25, &mut out);
        assert!(close(out[0], (0.0, 10.0)));
        out.clear();
        EllipseArc {
            center: (0.0, f64::NAN),
            ..ellipse
        }
        .append_points(0.25, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn hit_predicates_cover_polylines_polygons_and_ribbons() {
        let line = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert!((distance_to_polyline((5.0, 3.0), &line) - 3.0).abs() < 1e-9);
        assert!((distance_to_polyline((13.0, 5.0), &line) - 3.0).abs() < 1e-9);
        assert_eq!(distance_to_polyline((0.0, 0.0), &[]), f64::INFINITY);
        assert!(
            (distance_to_segment((5.0, 5.0), (0.0, 0.0), (0.0, 0.0)) - 50f64.sqrt()).abs() < 1e-9
        );

        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        assert!(point_in_polygon((5.0, 5.0), &square));
        assert!(!point_in_polygon((15.0, 5.0), &square));
        assert!(!point_in_polygon((5.0, 5.0), &square[..2]));

        let upper = [(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let lower = [(0.0, 10.0), (10.0, 10.0), (20.0, 10.0)];
        assert!(point_in_ribbon((15.0, 5.0), &upper, &lower));
        assert!(!point_in_ribbon((25.0, 5.0), &upper, &lower));
    }

    #[test]
    fn convex_ribbons_cover_exactly_their_polygon() {
        for polygon in [
            vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)],
            vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
            vec![
                (0.0, 0.0),
                (8.0, -2.0),
                (12.0, 4.0),
                (6.0, 10.0),
                (-2.0, 6.0),
            ],
        ] {
            let mut chains = Vec::new();
            let length = convex_ribbon(&polygon, &mut chains);
            assert_eq!(chains.len(), length * 2);
            let (upper, lower) = chains.split_at(length);
            for y in 0..12 {
                for x in -3..14 {
                    let point = (x as f64 + 0.37, y as f64 - 2.0 + 0.41);
                    assert_eq!(
                        point_in_ribbon(point, upper, lower),
                        point_in_polygon(point, &polygon),
                        "{point:?} in {polygon:?}",
                    );
                }
            }
        }
        let mut chains = Vec::new();
        assert_eq!(convex_ribbon(&[(0.0, 0.0), (1.0, 1.0)], &mut chains), 0);
        assert!(chains.is_empty());
    }

    #[test]
    fn polygon_clipping_keeps_exactly_the_part_inside_the_rectangle() {
        let mut out = Vec::new();
        // Fully inside: unchanged.
        let inside = [(10.0, 10.0), (40.0, 10.0), (25.0, 30.0)];
        clip_polygon_to_rect(&inside, PANE, &mut out);
        assert_eq!(out, inside);
        // Fully outside or non-finite: nothing.
        clip_polygon_to_rect(&[(200.0, 0.0), (300.0, 0.0), (250.0, 40.0)], PANE, &mut out);
        assert!(out.is_empty());
        clip_polygon_to_rect(&[(f64::NAN, 0.0), (30.0, 0.0), (20.0, 9.0)], PANE, &mut out);
        assert!(out.is_empty());
        // A steep parallelogram crossing every edge: containment matches point-by-point, and the
        // clipped outline stays on or inside the rectangle.
        let polygon = [
            (-40.0, -500.0),
            (150.0, 700.0),
            (150.0, 740.0),
            (-40.0, -460.0),
        ];
        clip_polygon_to_rect(&polygon, PANE, &mut out);
        assert!(out.len() >= 3);
        assert!(out.iter().all(|&point| {
            point.0 >= PANE.left - 1e-9
                && point.0 <= PANE.right + 1e-9
                && point.1 >= PANE.top - 1e-9
                && point.1 <= PANE.bottom + 1e-9
        }));
        let mut chains = Vec::new();
        let length = convex_ribbon(&out, &mut chains);
        let (upper, lower) = chains.split_at(length);
        for y in -2..54 {
            for x in -3..104 {
                let point = (x as f64 + 0.37, y as f64 + 0.41);
                assert_eq!(
                    point_in_polygon(point, &out),
                    PANE.contains(point) && point_in_polygon(point, &polygon),
                    "{point:?}"
                );
                assert_eq!(
                    point_in_ribbon(point, upper, lower),
                    point_in_polygon(point, &out)
                );
            }
        }
    }

    #[test]
    fn half_plane_clips_keep_the_right_hand_side_and_compose_into_wedges() {
        let pane = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)];
        let mut out = Vec::new();
        // Walking right along y = 20, the right-hand side on a y-down screen is below.
        clip_to_half_plane(&pane, (0.0, 20.0), (1.0, 20.0), &mut out);
        assert!(out.iter().all(|point| point.1 >= 20.0 - 1e-9));
        assert!(point_in_polygon((50.0, 40.0), &out) && !point_in_polygon((50.0, 10.0), &out));
        // The wedge between two rays from (10, 25): toward (100, 0) and toward (100, 50).
        let apex = (10.0, 25.0);
        let mut first = Vec::new();
        clip_to_half_plane(&pane, apex, (100.0, 0.0), &mut first);
        clip_to_half_plane(&first, apex, (-80.0, 0.0), &mut out);
        for (point, inside) in [
            ((90.0, 25.0), true),
            ((50.0, 20.0), true),
            ((5.0, 25.0), false),
            ((50.0, 2.0), false),
        ] {
            assert_eq!(point_in_polygon(point, &out), inside, "{point:?}");
        }
        clip_to_half_plane(&[], apex, (1.0, 1.0), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn convex_clipping_keeps_exactly_the_part_inside_the_rect() {
        let mut out = Vec::new();
        // A strip crossing the whole pane diagonally and far beyond it.
        let strip = [(-50.0, -10.0), (150.0, 30.0), (150.0, 60.0), (-50.0, 20.0)];
        clip_polygon_to_rect(&strip, PANE, &mut out);
        assert!(out
            .iter()
            .all(|&(x, y)| (0.0..=100.0).contains(&x) && (0.0..=50.0).contains(&y)));
        for y in 0..50 {
            for x in 0..100 {
                let point = (x as f64 + 0.37, y as f64 + 0.41);
                assert_eq!(
                    point_in_polygon(point, &out),
                    point_in_polygon(point, &strip),
                    "{point:?}"
                );
            }
        }
        // Fully inside is unchanged; fully outside is empty; the pane inside a polygon is the pane.
        let inner = [(10.0, 10.0), (20.0, 10.0), (15.0, 20.0)];
        clip_polygon_to_rect(&inner, PANE, &mut out);
        assert_eq!(out, inner);
        clip_polygon_to_rect(&[(200.0, 0.0), (300.0, 0.0), (250.0, 40.0)], PANE, &mut out);
        assert!(out.is_empty());
        let cover = [(-10.0, -10.0), (110.0, -10.0), (110.0, 60.0), (-10.0, 60.0)];
        clip_polygon_to_rect(&cover, PANE, &mut out);
        let mut chains = Vec::new();
        let length = convex_ribbon(&out, &mut chains);
        let (upper, lower) = chains.split_at(length);
        assert!(point_in_ribbon((1.0, 1.0), upper, lower));
        assert!(point_in_ribbon((99.0, 49.0), upper, lower));
    }

    fn clipped_parts(points: &[Point], period: f64) -> Vec<Vec<Point>> {
        let mut parts = Vec::new();
        clip_polyline_to_rect(points, PANE, period, &mut Vec::new(), |run| {
            parts.push(run.to_vec());
        });
        parts
    }

    #[test]
    fn polyline_clipping_bounds_far_reaching_lines_and_skips_non_finite_segments() {
        let inside = [(10.0, 10.0), (20.0, 30.0), (90.0, 40.0)];
        assert_eq!(clipped_parts(&inside, 12.0), vec![inside.to_vec()]);
        assert!(clipped_parts(&[(200.0, 0.0), (300.0, 40.0)], 12.0).is_empty());
        // Lines reaching 10^12 or 10^300 px either way keep one part within a period of the
        // rectangle (float cancellation at 10^300 may shorten it, never carry it away).
        let parts = clipped_parts(&[(-1e12, 25.0), (1e12, 25.0)], 0.0);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].len(), 2);
        assert!((parts[0][0].0 - 0.0).abs() < 1e-3 && (parts[0][1].0 - 100.0).abs() < 1e-3);
        for part in clipped_parts(&[(-1e300, 25.0), (1e300, -25.0)], 12.0) {
            assert!(part
                .iter()
                .all(|&(x, y)| (-12.0..=112.0).contains(&x) && (-12.0..=62.0).contains(&y)));
        }
        let parts = clipped_parts(&[(-1e7 - 0.5, 25.0), (1e7, 25.0)], 12.0);
        assert_eq!(parts.len(), 1);
        let (start, end) = (parts[0][0], parts[0][parts[0].len() - 1]);
        assert!(start.0 <= 0.0 && start.0 > -12.0, "{start:?}");
        assert!(((start.0 + 1e7 + 0.5) / 12.0).fract().abs() < 1e-6);
        assert_eq!(end, (100.0, 25.0));
        // Non-finite segments drop out; the finite rest has no phase to keep.
        let parts = clipped_parts(
            &[
                (10.0, 10.0),
                (f64::INFINITY, 10.0),
                (20.0, 20.0),
                (30.0, 20.0),
            ],
            12.0,
        );
        assert_eq!(parts, vec![vec![(20.0, 20.0), (30.0, 20.0)]]);
    }

    #[test]
    fn polyline_clipping_keeps_the_dash_phase_inside_the_rect() {
        use crate::line::{dash_split, LinePoint};
        let pattern = [6.0, 6.0];
        // In, out below the rectangle, back in, and out through the right edge.
        let polyline = [
            (-37.3, 10.0),
            (50.0, 10.0),
            (50.0, 90.0),
            (70.0, 90.0),
            (70.0, 20.0),
            (180.0, 20.0),
        ];
        let to_line = |points: &[Point]| {
            points
                .iter()
                .map(|&(x, y)| LinePoint { x, y })
                .collect::<Vec<_>>()
        };
        let dashes = |runs: Vec<Vec<LinePoint>>| {
            runs.into_iter()
                .map(|run| run.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        let reference = dashes(dash_split(&to_line(&polyline), &pattern));
        let parts = clipped_parts(&polyline, 12.0);
        assert_eq!(parts.len(), 2, "{parts:?}");
        let clipped = parts
            .iter()
            .flat_map(|part| dashes(dash_split(&to_line(part), &pattern)))
            .collect::<Vec<_>>();
        let on = |runs: &[Vec<Point>], point: Point| {
            runs.iter()
                .any(|run| distance_to_polyline(point, run) < 1e-6)
        };
        let mut walked = 0.0;
        let mut samples = 0;
        for pair in polyline.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let mut s = 0.25;
            while s < length {
                let point = (
                    a.0 + (b.0 - a.0) * s / length,
                    a.1 + (b.1 - a.1) * s / length,
                );
                // Away from dash ends, where both sides agree to float precision.
                let phase = (walked + s) % 12.0;
                if PANE.contains(point) && (phase - 6.0).abs() > 0.1 && phase > 0.1 {
                    assert_eq!(on(&clipped, point), on(&reference, point), "{point:?}");
                    samples += 1;
                }
                s += 0.5;
            }
            walked += length;
        }
        assert!(samples > 200);
    }

    #[test]
    fn line_intersections_report_both_parameters() {
        let (t, u) = line_intersection((0.0, 0.0), (10.0, 10.0), (0.0, 10.0), (10.0, 0.0)).unwrap();
        assert!((t - 0.5).abs() < 1e-12 && (u - 0.5).abs() < 1e-12);
        // Beyond both segments: the lines still cross.
        let (t, u) = line_intersection((0.0, 0.0), (1.0, 0.0), (5.0, 3.0), (5.0, 2.0)).unwrap();
        assert!((t - 5.0).abs() < 1e-12 && (u - 3.0).abs() < 1e-12);
        assert_eq!(
            line_intersection((0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (2.0, 3.0)),
            None,
            "parallel"
        );
        assert_eq!(
            line_intersection((0.0, 0.0), (0.0, 0.0), (0.0, 1.0), (2.0, 3.0)),
            None,
            "degenerate"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Clipped curve flattening, circles through three points, and nonzero polygon fills.

/// Upper bound on the points one clipped flattening appends, keeping frame work bounded at any
/// zoom. Pieces past the budget become single chords.
pub const MAX_FLATTEN_POINTS: usize = 1024;
/// Deepest bisection of one curve piece (far below the budget for any on-screen curve).
const MAX_FLATTEN_DEPTH: u32 = 24;

/// Append the quadratic Bézier `p0 → p2` with control `p1` (see [`flatten_cubic`]).
pub fn flatten_quadratic(
    p0: Point,
    p1: Point,
    p2: Point,
    tolerance: f64,
    clip: Rect,
    out: &mut Vec<Point>,
) {
    // Exact degree elevation: one flattener serves both curve degrees.
    let toward = |from: Point| {
        (
            from.0 + (p1.0 - from.0) * 2.0 / 3.0,
            from.1 + (p1.1 - from.1) * 2.0 / 3.0,
        )
    };
    flatten_cubic(p0, toward(p0), toward(p2), p2, tolerance, clip, out);
}

/// Append the cubic Bézier `p0 → p3` with controls `p1`, `p2` as a polyline, `p0` included.
/// Pieces are bisected until their control points lie within `tolerance` of the chord, so the
/// polyline stays within `tolerance` of the curve wherever the curve can be inside `clip`. A piece
/// whose control polygon lies outside `clip` becomes one chord: the curve and that chord both lie
/// inside the control polygon, so nothing within `clip` changes, and off-screen geometry costs
/// almost nothing at any zoom. Non-finite input appends nothing.
pub fn flatten_cubic(
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
    tolerance: f64,
    clip: Rect,
    out: &mut Vec<Point>,
) {
    let points = [p0, p1, p2, p3];
    if Rect::bounding(&points).is_none() {
        return;
    }
    out.push(p0);
    let limit = out.len() + MAX_FLATTEN_POINTS;
    cubic_piece(points, tolerance.max(1e-3), clip, 0, limit, out);
}

fn cubic_piece(
    p: [Point; 4],
    tolerance: f64,
    clip: Rect,
    depth: u32,
    limit: usize,
    out: &mut Vec<Point>,
) {
    // The curve lies in its control polygon, so control points within `tolerance` of the chord
    // bound the curve's distance to it (distance to a segment is convex).
    let flat = distance_to_segment(p[1], p[0], p[3]).max(distance_to_segment(p[2], p[0], p[3]))
        <= tolerance;
    let hidden = Rect::bounding(&p).is_none_or(|bounds| !bounds.intersects(&clip));
    if flat || hidden || depth >= MAX_FLATTEN_DEPTH || out.len() + 1 >= limit {
        out.push(p[3]);
        return;
    }
    let (a, b, c) = (
        midpoint(p[0], p[1]),
        midpoint(p[1], p[2]),
        midpoint(p[2], p[3]),
    );
    let (d, e) = (midpoint(a, b), midpoint(b, c));
    let m = midpoint(d, e);
    cubic_piece([p[0], a, d, m], tolerance, clip, depth + 1, limit, out);
    cubic_piece([m, e, c, p[3]], tolerance, clip, depth + 1, limit, out);
}

impl EllipseArc {
    /// The point at `angle` on the ellipse scaled by `scale` about its center.
    fn point_at(&self, angle: f64, scale: f64) -> Point {
        let (sin_r, cos_r) = self.rotation.sin_cos();
        let (sin_a, cos_a) = angle.sin_cos();
        let (x, y) = (self.rx * cos_a * scale, self.ry * sin_a * scale);
        (
            self.center.0 + x * cos_r - y * sin_r,
            self.center.1 + x * sin_r + y * cos_r,
        )
    }

    /// Append the arc's points like [`EllipseArc::append_points`], both end points included, but
    /// spend them only where the arc can be inside `clip`: quarter-turn pieces are bisected until
    /// the mid point lies within `tolerance` of the chord (the exact deviation of an elliptical
    /// arc, whose tangent there is parallel to the chord), and a piece whose tangent triangle lies
    /// outside `clip` becomes one chord. A huge zoomed-in circle therefore stays within `tolerance`
    /// on screen with at most [`MAX_FLATTEN_POINTS`] points; an arc wholly inside `clip` takes the
    /// uniform chords of [`EllipseArc::append_points`]. Non-finite input appends nothing.
    pub fn append_clipped_points(&self, tolerance: f64, clip: Rect, out: &mut Vec<Point>) {
        let values = [
            self.center.0,
            self.center.1,
            self.rx,
            self.ry,
            self.rotation,
            self.start,
            self.sweep,
        ];
        if !values.iter().all(|value| value.is_finite()) {
            return;
        }
        // Wholly inside the clip, nothing can be skipped: the uniform chords are cheaper, and
        // within tolerance while they stay under their cap.
        let reach = self.rx.abs().max(self.ry.abs());
        let inside = Rect {
            left: self.center.0 - reach,
            top: self.center.1 - reach,
            right: self.center.0 + reach,
            bottom: self.center.1 + reach,
        };
        if clip.contains((inside.left, inside.top))
            && clip.contains((inside.right, inside.bottom))
            && arc_segment_count(reach, self.sweep, tolerance) < MAX_ARC_SEGMENTS
        {
            self.append_points(tolerance, out);
            return;
        }
        let pieces = ((self.sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
        let limit = out.len() + 1 + MAX_FLATTEN_POINTS;
        let mut from_angle = self.start;
        let mut from = self.point_at(from_angle, 1.0);
        out.push(from);
        for piece in 1..=pieces {
            let to_angle = self.start + self.sweep * piece as f64 / pieces as f64;
            let to = self.point_at(to_angle, 1.0);
            self.arc_piece(
                (from_angle, to_angle),
                (from, to),
                tolerance.max(1e-3),
                clip,
                0,
                limit,
                out,
            );
            (from_angle, from) = (to_angle, to);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn arc_piece(
        &self,
        (t0, t1): (f64, f64),
        (p0, p1): (Point, Point),
        tolerance: f64,
        clip: Rect,
        depth: u32,
        limit: usize,
        out: &mut Vec<Point>,
    ) {
        let half = (t1 - t0) / 2.0;
        let mid = t0 + half;
        let middle = self.point_at(mid, 1.0);
        // Pieces span at most a quarter turn, so the tangents at both ends meet at this apex and
        // the piece lies inside the triangle (p0, apex, p1).
        let apex = self.point_at(mid, 1.0 / half.cos());
        let flat = distance_to_segment(middle, p0, p1) <= tolerance;
        let hidden = Rect::bounding(&[p0, apex, p1]).is_none_or(|bounds| !bounds.intersects(&clip));
        if flat || hidden || depth >= MAX_FLATTEN_DEPTH || out.len() + 1 >= limit {
            out.push(p1);
            return;
        }
        self.arc_piece(
            (t0, mid),
            (p0, middle),
            tolerance,
            clip,
            depth + 1,
            limit,
            out,
        );
        self.arc_piece(
            (mid, t1),
            (middle, p1),
            tolerance,
            clip,
            depth + 1,
            limit,
            out,
        );
    }
}

/// Center and radius of the circle through `a`, `b`, and `c`; `None` when the points are not
/// finite or (nearly) collinear, where the circle degenerates into their line.
pub fn circle_through(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let (cx, cy) = (c.0 - a.0, c.1 - a.1);
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let d = 2.0 * (bx * cy - by * cx);
    // |d| = 2·|ab|·|ac|·sin(angle at a): reject angles below 1e-9 rad.
    if !d.is_finite() || d == 0.0 || d.abs() <= 2e-9 * (b2 * c2).sqrt() {
        return None;
    }
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    let radius = ux.hypot(uy);
    radius.is_finite().then_some(((a.0 + ux, a.1 + uy), radius))
}

/// Largest polygon [`nonzero_ribbon`] tessellates (vertices over all contours); with its bounds on
/// edge crossings and output rungs, beyond which the polygon gets no fill and no coarser fallback
/// (unlike [`tube_ribbon`]). Real drawings sit far below all three. The vertex bound also caps the
/// pairwise edge scan, which the crossing bound does not limit for a polygon without crossings; the
/// values are safety limits, not tuned ones.
pub const MAX_FILL_VERTICES: usize = 2048;
const MAX_FILL_CROSSINGS: usize = 4096;
const MAX_FILL_RUNGS: usize = 8192;

struct FillEdge {
    top: Point,
    bottom: Point,
    /// x change per unit of y.
    slope: f64,
    winding: i32,
}

impl FillEdge {
    fn new(top: Point, bottom: Point, winding: i32) -> Self {
        Self {
            top,
            bottom,
            slope: (bottom.0 - top.0) / (bottom.1 - top.1),
            winding,
        }
    }

    /// The edge's x at `y`, exact at its end points so strips continue across shared vertices.
    fn x_at(&self, y: f64) -> f64 {
        if y <= self.top.1 {
            self.top.0
        } else if y >= self.bottom.1 {
            self.bottom.0
        } else {
            self.top.0 + (y - self.top.1) * self.slope
        }
    }
}

/// The y of the proper crossing of two edges (strictly inside both y ranges), if any.
fn crossing_y(a: &FillEdge, b: &FillEdge) -> Option<f64> {
    if a.top.0.max(a.bottom.0) < b.top.0.min(b.bottom.0)
        || b.top.0.max(b.bottom.0) < a.top.0.min(a.bottom.0)
    {
        return None;
    }
    let (rx, ry) = (a.bottom.0 - a.top.0, a.bottom.1 - a.top.1);
    let (sx, sy) = (b.bottom.0 - b.top.0, b.bottom.1 - b.top.1);
    let denominator = rx * sy - ry * sx;
    if denominator * denominator
        <= f64::EPSILON * f64::EPSILON * ((rx * rx + ry * ry) * (sx * sx + sy * sy))
    {
        return None;
    }
    let (qx, qy) = (b.top.0 - a.top.0, b.top.1 - a.top.1);
    let t = (qx * sy - qy * sx) / denominator;
    let u = (qx * ry - qy * rx) / denominator;
    let y = a.top.1 + ry * t;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then_some(y)
        .filter(|y| *y > a.top.1 && *y < a.bottom.1 && *y > b.top.1 && *y < b.bottom.1)
}

/// Tessellate the nonzero-rule region of a closed polygon — concave or self-intersecting, the
/// last vertex joining the first — into one ribbon (see [`convex_ribbon`]). See
/// [`nonzero_ribbon_contours`].
pub fn nonzero_ribbon(polygon: &[Point], out: &mut Vec<Point>) -> usize {
    nonzero_ribbon_contours(polygon, &[polygon.len()], out)
}

/// Tessellate the nonzero-rule region of several closed contours — `ends` holds each contour's
/// exclusive end index into `points` in ascending order, and each contour's last vertex joins its
/// own first — into one ribbon: horizontal slabs between every vertex and crossing y become
/// trapezoids between the edges where the winding number leaves and returns to zero. A trapezoid
/// whose top side is the bottom side of one in the slab above continues that trapezoid's strip
/// (and extends its last quad while the same two edges bound it), so a simple region is one strip
/// of rungs `(left, right)` rather than one quad per slab. Strips never overlap, share one
/// orientation, and are joined by zero-area quads, so every executor's `Prim::BandFill` (the union
/// of its quads, or its path upper-forward + lower-backward under the nonzero rule) and
/// [`point_in_ribbon`] cover exactly the region. Appends the `upper` (left) then `lower` (right)
/// chains of equal length to `out` and returns that length; 0 when the region is empty or the
/// contours exceed [`MAX_FILL_VERTICES`] or the crossing and rung bounds.
pub fn nonzero_ribbon_contours(points: &[Point], ends: &[usize], out: &mut Vec<Point>) -> usize {
    let count = points.len();
    if !(3..=MAX_FILL_VERTICES).contains(&count)
        || ends.last() != Some(&count)
        || ends.windows(2).any(|pair| pair[0] > pair[1])
        || Rect::bounding(points).is_none()
    {
        return 0;
    }
    let mut edges = Vec::with_capacity(count);
    let mut start = 0;
    for &end in ends {
        for index in start..end {
            let (p, q) = (
                points[index],
                points[if index + 1 == end { start } else { index + 1 }],
            );
            // Horizontal edges never change the winding of a horizontal scan line.
            if p.1 == q.1 {
                continue;
            }
            let (top, bottom, winding) = if p.1 < q.1 { (p, q, 1) } else { (q, p, -1) };
            edges.push(FillEdge::new(top, bottom, winding));
        }
        start = end;
    }
    edges.sort_unstable_by(|a, b| a.top.1.total_cmp(&b.top.1));
    let mut ys = Vec::with_capacity(edges.len() * 2);
    ys.extend(edges.iter().flat_map(|edge| [edge.top.1, edge.bottom.1]));
    let endpoint_ys = ys.len();
    for (index, edge) in edges.iter().enumerate() {
        for other in &edges[index + 1..] {
            if other.top.1 >= edge.bottom.1 {
                break;
            }
            if let Some(y) = crossing_y(edge, other) {
                if ys.len() - endpoint_ys == MAX_FILL_CROSSINGS {
                    return 0;
                }
                ys.push(y);
            }
        }
    }
    ys.sort_unstable_by(f64::total_cmp);
    ys.dedup();

    // Rungs `(left, right)` at slab boundaries, linked per strip through `next`; strips as their
    // (first, last) rung.
    let mut rungs: Vec<(Point, Point, usize)> = Vec::new();
    let mut strips: Vec<(usize, usize)> = Vec::new();
    // Strips reaching the previous slab's bottom: (left edge, right edge, strip).
    let mut open: Vec<(usize, usize, usize)> = Vec::new();
    let mut reached: Vec<(usize, usize, usize)> = Vec::new();
    // Edges crossing the slab with their x at its middle (the sort key; nearly sorted already).
    let mut active: Vec<(f64, usize)> = Vec::new();
    let mut next = 0;
    for slab in ys.windows(2) {
        let (top, bottom) = (slab[0], slab[1]);
        active.retain(|&(_, edge)| edges[edge].bottom.1 > top);
        while next < edges.len() && edges[next].top.1 <= top {
            if edges[next].bottom.1 > top {
                active.push((0.0, next));
            }
            next += 1;
        }
        let middle = (top + bottom) / 2.0;
        for (key, edge) in &mut active {
            *key = edges[*edge].x_at(middle);
        }
        active.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        reached.clear();
        let mut winding = 0;
        let mut left = 0;
        for &(_, edge) in &active {
            let before = winding;
            winding += edges[edge].winding;
            if before == 0 && winding != 0 {
                left = edge;
            } else if before != 0 && winding == 0 {
                let right = edge;
                let rung = |y: f64| ((edges[left].x_at(y), y), (edges[right].x_at(y), y));
                let (top_rung, bottom_rung) = (rung(top), rung(bottom));
                if top_rung.1 .0 <= top_rung.0 .0 && bottom_rung.1 .0 <= bottom_rung.0 .0 {
                    // Coincident edges (collinear spikes) enclose no area.
                    continue;
                }
                let continued = open.iter().position(|&(open_left, open_right, strip)| {
                    let (rung_left, rung_right, _) = rungs[strips[strip].1];
                    (open_left == left && open_right == right)
                        || (rung_left == top_rung.0 && rung_right == top_rung.1)
                });
                let strip = match continued {
                    Some(position) => {
                        let (open_left, open_right, strip) = open.swap_remove(position);
                        let last = strips[strip].1;
                        if open_left == left && open_right == right {
                            // Both edges are straight: the strip's last quad simply grows.
                            rungs[last].0 = bottom_rung.0;
                            rungs[last].1 = bottom_rung.1;
                        } else {
                            if rungs.len() == MAX_FILL_RUNGS {
                                return 0;
                            }
                            rungs.push((bottom_rung.0, bottom_rung.1, usize::MAX));
                            rungs[last].2 = rungs.len() - 1;
                            strips[strip].1 = rungs.len() - 1;
                        }
                        strip
                    }
                    None => {
                        if rungs.len() + 2 > MAX_FILL_RUNGS {
                            return 0;
                        }
                        let first = rungs.len();
                        rungs.push((top_rung.0, top_rung.1, first + 1));
                        rungs.push((bottom_rung.0, bottom_rung.1, usize::MAX));
                        strips.push((first, first + 1));
                        strips.len() - 1
                    }
                };
                reached.push((left, right, strip));
            }
        }
        std::mem::swap(&mut open, &mut reached);
    }
    if strips.is_empty() {
        return 0;
    }
    // Entries (upper, lower) are each strip's rungs in order; consecutive strips join through
    // (R, R) → (L', L'), whose three connecting quads have zero area.
    let length = rungs.len() + 2 * (strips.len() - 1);
    out.reserve(length * 2);
    let last_strip = strips.len() - 1;
    for side in [0, 1] {
        for (index, &(first, last)) in strips.iter().enumerate() {
            if index > 0 {
                out.push(rungs[first].0);
            }
            let mut rung = first;
            while rung != usize::MAX {
                out.push(if side == 0 {
                    rungs[rung].0
                } else {
                    rungs[rung].1
                });
                rung = rungs[rung].2;
            }
            if index < last_strip {
                out.push(rungs[last].1);
            }
        }
    }
    length
}

// ---------------------------------------------------------------------------------------------
// Wide strokes painted once per pixel (the highlighter's `Tube` part).

/// Drop vertices of an open polyline while every dropped vertex stays within `tolerance` of the
/// kept polyline, in one linear pass (sleeve fitting: each run keeps the directions from its
/// start that pass within `tolerance` of every vertex so far). The first and last vertices are
/// always kept; appends the result to `out`.
pub fn simplify_polyline(points: &[Point], tolerance: f64, out: &mut Vec<Point>) {
    let Some((&first, rest)) = points.split_first() else {
        return;
    };
    out.push(first);
    let Some(&last) = rest.last() else {
        return;
    };
    let tolerance = tolerance.max(0.0);
    let mut anchor = first;
    // (reference angle, lowest and highest allowed offset from it) of chords from `anchor`.
    let mut sleeve: Option<(f64, f64, f64)> = None;
    let mut farthest = 0.0;
    let mut candidate: Option<Point> = None;
    for &point in rest {
        // At most twice: a point leaving the sleeve is retried from the vertex it forces.
        loop {
            let (dx, dy) = (point.0 - anchor.0, point.1 - anchor.1);
            let distance = dx.hypot(dy);
            if distance <= tolerance {
                break;
            }
            let angle = dy.atan2(dx);
            let (reference, low, high) =
                *sleeve.get_or_insert((angle, -std::f64::consts::PI, std::f64::consts::PI));
            let offset = (angle - reference + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            if (low..=high).contains(&offset) && distance >= farthest {
                let half = (tolerance / distance).asin();
                sleeve = Some((reference, low.max(offset - half), high.min(offset + half)));
                farthest = distance;
                candidate = Some(point);
                break;
            }
            // Only an accepted candidate narrows the sleeve, so one exists here.
            let Some(vertex) = candidate.take() else {
                break;
            };
            out.push(vertex);
            anchor = vertex;
            sleeve = None;
            farthest = 0.0;
        }
    }
    if let Some(vertex) = candidate.filter(|&vertex| vertex != last) {
        out.push(vertex);
    }
    if out.last() != Some(&last) {
        out.push(last);
    }
}

/// Append the closed outline of the tube of `radius` around an open polyline — every point within
/// `radius` of it, so round joins and round caps — with arcs as chords within `tolerance`. The
/// outline follows one side forward, the end cap, the other side backward, and the start cap; the
/// outer side of every turn gets its round join and the inner side passes through the vertex, so
/// the outline is the boundary sum of the segments' rectangles, the outer join wedges, and the
/// caps, all of one orientation. Its nonzero-rule region is therefore exactly their union, the
/// tube, wherever the polyline overlaps or crosses itself. An inner turn whose miter kite lies in
/// both adjacent rectangles takes the miter point instead, which only lowers a winding of at
/// least two there. A single point (or coincident points) is a disc; empty input appends nothing.
pub fn tube_outline(points: &[Point], radius: f64, tolerance: f64, out: &mut Vec<Point>) {
    let mut distinct: Vec<Point> = Vec::with_capacity(points.len());
    for &point in points {
        if distinct.last() != Some(&point) {
            distinct.push(point);
        }
    }
    let Some(&first) = distinct.first() else {
        return;
    };
    let at = |center: Point, angle: f64| {
        let (sin, cos) = angle.sin_cos();
        (center.0 + radius * cos, center.1 + radius * sin)
    };
    if distinct.len() == 1 {
        let chords = arc_segment_count(radius, std::f64::consts::TAU, tolerance).max(3);
        out.extend(
            (0..chords).map(|step| at(first, std::f64::consts::TAU * step as f64 / chords as f64)),
        );
        return;
    }
    let reversed = distinct.iter().rev().copied().collect::<Vec<_>>();
    tube_side(&distinct, radius, tolerance, true, out);
    tube_side(&reversed, radius, tolerance, false, out);
}

/// One side of [`tube_outline`] along `points` (distinct consecutive vertices) on the left of the
/// direction of travel, then its end cap. `first_side` breaks the tie of an exact U-turn, whose
/// two traversals are locally identical: the first gets the round join, the second the vertex.
fn tube_side(
    points: &[Point],
    radius: f64,
    tolerance: f64,
    first_side: bool,
    out: &mut Vec<Point>,
) {
    let direction = |a: Point, b: Point| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = dx.hypot(dy);
        (dx / length, dy / length)
    };
    // The side's normal: [`segment_normal`] of the direction.
    let normal = |d: Point| (d.1, -d.0);
    let offset = |p: Point, n: Point| (p.0 + n.0 * radius, p.1 + n.1 * radius);
    // Chords of the arc of `sweep` radians turning from normal `n` around `center`, without its
    // ends (a positive sweep turns from the normal toward the direction of travel).
    let arc = |center: Point, n: Point, sweep: f64, out: &mut Vec<Point>| {
        let chords = arc_segment_count(radius, sweep, tolerance);
        let start = n.1.atan2(n.0);
        out.extend((1..chords).map(|step| {
            let (sin, cos) = (start + sweep * step as f64 / chords as f64).sin_cos();
            (center.0 + radius * cos, center.1 + radius * sin)
        }));
    };
    let segments = points.len() - 1;
    let mut d = direction(points[0], points[1]);
    out.push(offset(points[0], normal(d)));
    for index in 0..segments {
        let end = points[index + 1];
        if index + 1 == segments {
            out.push(offset(end, normal(d)));
            break;
        }
        let next = direction(end, points[index + 2]);
        let cross = d.0 * next.1 - d.1 * next.0;
        let dot = d.0 * next.0 + d.1 * next.1;
        let (n, next_n) = (normal(d), normal(next));
        if cross == 0.0 && dot > 0.0 {
            // Straight on: the next segment starts where this one ends.
            out.push(offset(end, n));
        } else if cross > 0.0 || (cross == 0.0 && first_side) {
            // This side is outside the turn: the round join (a U-turn's through the direction of
            // travel).
            out.push(offset(end, n));
            arc(end, n, cross.abs().atan2(dot), out);
            out.push(offset(end, next_n));
        } else {
            // Inside the turn: through the vertex. Where the kite between the offset lines'
            // crossing (the miter point), both offset ends, and the vertex lies in both segments'
            // rectangles, it is covered twice, so the miter point alone keeps the region while
            // sparing the scan the vertex spokes.
            let shortest = (end.0 - points[index].0)
                .hypot(end.1 - points[index].1)
                .min((points[index + 2].0 - end.0).hypot(points[index + 2].1 - end.1));
            let sin = cross.abs();
            if dot > -1.0 && radius * sin.max(sin / (1.0 + dot)) <= shortest {
                let scale = radius / (1.0 + dot);
                out.push((
                    end.0 + (n.0 + next_n.0) * scale,
                    end.1 + (n.1 + next_n.1) * scale,
                ));
            } else {
                out.push(offset(end, n));
                out.push(end);
                out.push(offset(end, next_n));
            }
        }
        d = next;
    }
    // The end cap: half a turn from this side's normal through the direction of travel.
    arc(
        *points.last().expect("two vertices"),
        normal(d),
        std::f64::consts::PI,
        out,
    );
}

/// Attempts [`tube_ribbon`] makes, doubling its tolerance after each, before giving up.
const TUBE_ATTEMPTS: u32 = 5;

/// The nonzero ribbon (see [`nonzero_ribbon_contours`]) of the tube of `radius` around an open
/// polyline (see [`tube_outline`]): a wide stroke with round joins and caps as a region that every
/// executor paints once per pixel, so a translucent color keeps one opacity where the stroke
/// overlaps or crosses itself (overlapping stroke triangles would blend twice on the GPU
/// executors). Only the runs of segments whose tube can reach `clip` are outlined, each vertex
/// run is first simplified within `tolerance` (see [`simplify_polyline`]), and the scan runs
/// across the region's longer side. When the outline exceeds the fill bounds the tolerance
/// doubles, up to [`TUBE_ATTEMPTS`] times, so work stays bounded at any length and zoom. Appends
/// `upper` then `lower` chains and returns their length; 0 when nothing reaches `clip`, the
/// input is not finite, or every attempt exceeds the bounds.
pub fn tube_ribbon(
    points: &[Point],
    radius: f64,
    tolerance: f64,
    clip: Rect,
    out: &mut Vec<Point>,
) -> usize {
    if !(radius.is_finite() && radius > 0.0) {
        return 0;
    }
    let tolerance = tolerance.max(1e-3);
    // Vertex ranges of the runs of segments whose tube can reach the clip.
    let reaches = |segment: &[Point], tolerance: f64| {
        Rect::bounding(segment)
            .is_some_and(|bounds| bounds.inflate(radius + tolerance).intersects(&clip))
    };
    let mut runs: Vec<(usize, usize)> = Vec::new();
    if let [only] = points {
        if reaches(&[*only], tolerance) {
            runs.push((0, 1));
        }
    }
    for (index, segment) in points.windows(2).enumerate() {
        if !reaches(segment, tolerance) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if run.1 == index + 1 => run.1 = index + 2,
            _ => runs.push((index, index + 2)),
        }
    }
    if runs.is_empty() {
        return 0;
    }
    let mut centerline = Vec::new();
    let mut outline = Vec::new();
    let mut ends = Vec::with_capacity(runs.len());
    for attempt in 0..TUBE_ATTEMPTS {
        let tolerance = tolerance * f64::from(1_u32 << attempt);
        outline.clear();
        ends.clear();
        for &(start, end) in &runs {
            centerline.clear();
            simplify_polyline(&points[start..end], tolerance, &mut centerline);
            tube_outline(&centerline, radius, tolerance, &mut outline);
            ends.push(outline.len());
            if outline.len() > MAX_FILL_VERTICES {
                break;
            }
        }
        if outline.len() > MAX_FILL_VERTICES {
            continue;
        }
        // Scan across the longer side: a long stroke then meets each scan line only a few times.
        let transpose = Rect::bounding(&outline)
            .is_some_and(|bounds| bounds.right - bounds.left > bounds.bottom - bounds.top);
        if transpose {
            outline
                .iter_mut()
                .for_each(|point| *point = (point.1, point.0));
        }
        let first = out.len();
        let length = nonzero_ribbon_contours(&outline, &ends, out);
        if length > 0 {
            if transpose {
                out[first..]
                    .iter_mut()
                    .for_each(|point| *point = (point.1, point.0));
            }
            return length;
        }
    }
    0
}

#[cfg(test)]
mod flatten_and_fill_tests {
    use super::*;

    const WIDE: Rect = Rect {
        left: -1e9,
        top: -1e9,
        right: 1e9,
        bottom: 1e9,
    };

    fn cubic_at(p: [Point; 4], t: f64) -> Point {
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        (
            p.iter().zip(w).map(|(point, w)| point.0 * w).sum(),
            p.iter().zip(w).map(|(point, w)| point.1 * w).sum(),
        )
    }

    /// How many of a ribbon's quads contain `point`: executors blend each quad, so more than one
    /// would paint a translucent fill twice.
    pub(super) fn quad_coverage(point: Point, upper: &[Point], lower: &[Point]) -> usize {
        (1..upper.len().min(lower.len()))
            .filter(|&index| {
                point_in_polygon(
                    point,
                    &[
                        upper[index - 1],
                        lower[index - 1],
                        lower[index],
                        upper[index],
                    ],
                )
            })
            .count()
    }

    /// Nonzero winding number reference (the Canvas2D fill rule).
    pub(super) fn winding(point: Point, polygon: &[Point]) -> i32 {
        let mut winding = 0;
        for index in 0..polygon.len() {
            let (a, b) = (polygon[index], polygon[(index + 1) % polygon.len()]);
            let side = (b.0 - a.0) * (point.1 - a.1) - (point.0 - a.0) * (b.1 - a.1);
            if a.1 <= point.1 && b.1 > point.1 && side > 0.0 {
                winding += 1;
            } else if a.1 > point.1 && b.1 <= point.1 && side < 0.0 {
                winding -= 1;
            }
        }
        winding
    }

    #[test]
    fn rect_helpers_bound_inflate_and_intersect() {
        let bounds = Rect::bounding(&[(3.0, 4.0), (-1.0, 9.0), (2.0, -2.0)]).unwrap();
        assert_eq!(
            bounds,
            Rect {
                left: -1.0,
                top: -2.0,
                right: 3.0,
                bottom: 9.0
            }
        );
        assert_eq!(Rect::bounding(&[]), None);
        assert_eq!(Rect::bounding(&[(f64::NAN, 0.0)]), None);
        let grown = bounds.inflate(1.0);
        assert_eq!((grown.left, grown.bottom), (-2.0, 10.0));
        assert!(bounds.intersects(&Rect {
            left: 3.0,
            top: 9.0,
            right: 4.0,
            bottom: 10.0
        }));
        assert!(!bounds.intersects(&Rect {
            left: 3.5,
            top: 0.0,
            right: 4.0,
            bottom: 1.0
        }));
    }

    #[test]
    fn cubic_flattening_stays_within_tolerance_and_keeps_its_ends() {
        let controls = [(0.0, 0.0), (40.0, 120.0), (160.0, -80.0), (200.0, 30.0)];
        let mut out = Vec::new();
        flatten_cubic(
            controls[0],
            controls[1],
            controls[2],
            controls[3],
            0.25,
            WIDE,
            &mut out,
        );
        assert_eq!(out[0], controls[0]);
        assert_eq!(*out.last().unwrap(), controls[3]);
        assert!(out.len() > 8 && out.len() < 200, "{} points", out.len());
        for step in 0..=200 {
            let point = cubic_at(controls, f64::from(step) / 200.0);
            assert!(distance_to_polyline(point, &out) <= 0.25 + 1e-9);
        }
        // A quadratic is the elevated cubic: through the midpoint (a + 2c + b) / 4.
        out.clear();
        flatten_quadratic(
            (0.0, 0.0),
            (50.0, 100.0),
            (100.0, 0.0),
            0.25,
            WIDE,
            &mut out,
        );
        assert!(distance_to_polyline((50.0, 50.0), &out) <= 0.25);
        out.clear();
        flatten_cubic(
            (f64::NAN, 0.0),
            (1.0, 1.0),
            (2.0, 2.0),
            (3.0, 3.0),
            0.25,
            WIDE,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn clipped_flattening_spends_points_only_where_visible() {
        // A zoomed-in circle of radius 1e6 around a 1000 × 600 window: the visible run stays
        // within tolerance while the whole turn stays inside the point budget.
        let clip = Rect {
            left: 0.0,
            top: 0.0,
            right: 1000.0,
            bottom: 600.0,
        };
        let circle = EllipseArc::circle((500.0, 300.0 + 1e6), 1e6, 0.0, std::f64::consts::TAU);
        let mut out = Vec::new();
        circle.append_clipped_points(0.25, clip, &mut out);
        assert!(out.len() <= MAX_FLATTEN_POINTS + 1, "{} points", out.len());
        assert!(out.len() > 20, "the visible run is refined");
        for step in 0..=100 {
            let x = f64::from(step) * 10.0;
            let y = 300.0 + 1e6 - (1e12 - (x - 500.0).powi(2)).sqrt();
            assert!(
                distance_to_polyline((x, y), &out) <= 0.25 + 1e-6,
                "x {x}: {}",
                distance_to_polyline((x, y), &out)
            );
        }
        // A fully visible ellipse matches the unclipped tessellation's accuracy.
        let ellipse = EllipseArc {
            center: (200.0, 150.0),
            rx: 120.0,
            ry: 40.0,
            rotation: 0.3,
            start: 0.0,
            sweep: std::f64::consts::TAU,
        };
        out.clear();
        ellipse.append_clipped_points(0.25, clip, &mut out);
        assert!(
            (out[0].0 - out.last().unwrap().0).abs() < 1e-9,
            "closed turn"
        );
        let (sin_r, cos_r) = 0.3_f64.sin_cos();
        for step in 0..360 {
            let angle = f64::from(step).to_radians();
            let (x, y) = (120.0 * angle.cos(), 40.0 * angle.sin());
            let point = (200.0 + x * cos_r - y * sin_r, 150.0 + x * sin_r + y * cos_r);
            assert!(distance_to_polyline(point, &out) <= 0.25 + 1e-9);
        }
        // An arc entirely outside the window costs a handful of chords.
        out.clear();
        EllipseArc::circle((5000.0, 5000.0), 100.0, 0.0, std::f64::consts::TAU)
            .append_clipped_points(0.25, clip, &mut out);
        assert!(out.len() <= 5);
    }

    #[test]
    fn circles_through_three_points() {
        let (center, radius) = circle_through((0.0, 10.0), (10.0, 0.0), (-10.0, 0.0)).unwrap();
        assert!(center.0.abs() < 1e-9 && center.1.abs() < 1e-9);
        assert!((radius - 10.0).abs() < 1e-9);
        assert_eq!(circle_through((0.0, 0.0), (1.0, 1.0), (2.0, 2.0)), None);
        assert_eq!(circle_through((0.0, 0.0), (0.0, 0.0), (2.0, 5.0)), None);
        assert_eq!(
            circle_through((f64::NAN, 0.0), (1.0, 0.0), (0.0, 1.0)),
            None
        );
    }

    #[test]
    fn nonzero_ribbons_cover_exactly_the_nonzero_region() {
        let polygons: Vec<Vec<Point>> = vec![
            // Convex, concave (an L), a notched comb, a self-crossing bow tie, a pentagram
            // (its center winds twice), duplicate and collinear vertices, and a square with a
            // same-direction inner loop.
            vec![(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)],
            vec![
                (0.0, 0.0),
                (20.0, 0.0),
                (20.0, 20.0),
                (40.0, 20.0),
                (40.0, 40.0),
                (0.0, 40.0),
            ],
            vec![
                (0.0, 0.0),
                (10.0, 30.0),
                (20.0, 5.0),
                (30.0, 30.0),
                (40.0, 0.0),
                (40.0, 40.0),
                (0.0, 40.0),
            ],
            vec![(0.0, 0.0), (40.0, 40.0), (40.0, 0.0), (0.0, 40.0)],
            (0..5)
                .map(|index| {
                    let angle = f64::from(index * 2) * std::f64::consts::TAU / 5.0;
                    (20.0 + 20.0 * angle.sin(), 20.0 - 20.0 * angle.cos())
                })
                .collect(),
            vec![
                (0.0, 0.0),
                (0.0, 0.0),
                (20.0, 0.0),
                (40.0, 0.0),
                (40.0, 40.0),
                (0.0, 40.0),
            ],
            vec![
                (0.0, 0.0),
                (40.0, 0.0),
                (40.0, 40.0),
                (0.0, 40.0),
                (0.0, 10.0),
                (30.0, 10.0),
                (30.0, 30.0),
                (10.0, 30.0),
                (10.0, 0.0),
            ],
        ];
        for polygon in &polygons {
            let mut chains = Vec::new();
            let length = nonzero_ribbon(polygon, &mut chains);
            assert!(length > 0, "{polygon:?}");
            assert_eq!(chains.len(), length * 2);
            let (upper, lower) = chains.split_at(length);
            for y in 0..45 {
                for x in -2..45 {
                    let point = (f64::from(x) + 0.37, f64::from(y) - 2.0 + 0.41);
                    assert_eq!(
                        point_in_ribbon(point, upper, lower),
                        winding(point, polygon) != 0,
                        "{point:?} in {polygon:?}"
                    );
                    assert!(
                        quad_coverage(point, upper, lower) <= 1,
                        "{point:?} blends once"
                    );
                }
            }
            // Every quad keeps one orientation or has zero area, so the executors' path fill
            // (the sum of quad windings) agrees with the union of quads.
            let mut signs = (0, 0);
            for index in 1..length {
                let quad = [
                    upper[index - 1],
                    lower[index - 1],
                    lower[index],
                    upper[index],
                ];
                let area: f64 = (0..4)
                    .map(|corner| {
                        let (a, b) = (quad[corner], quad[(corner + 1) % 4]);
                        a.0 * b.1 - b.0 * a.1
                    })
                    .sum();
                if area > 1e-9 {
                    signs.0 += 1;
                } else if area < -1e-9 {
                    signs.1 += 1;
                }
            }
            assert!(signs.0 == 0 || signs.1 == 0, "{signs:?} for {polygon:?}");
        }
        let mut chains = Vec::new();
        assert_eq!(nonzero_ribbon(&[(0.0, 0.0), (1.0, 1.0)], &mut chains), 0);
        assert_eq!(
            nonzero_ribbon(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)], &mut chains),
            0,
            "a collinear polygon encloses nothing"
        );
        assert!(chains.is_empty(), "degenerate polygons append nothing");
    }

    /// A regular polygon of `count` vertices and radius 100: simple, convex, and never degenerate.
    fn regular_polygon(count: usize) -> Vec<Point> {
        (0..count)
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / count as f64;
                (100.0 * angle.cos(), 100.0 * angle.sin())
            })
            .collect()
    }

    #[test]
    fn polygons_beyond_the_vertex_bound_get_no_fill() {
        // The bound is exact: the largest polygon still fills, and one vertex more gets nothing
        // (a simple convex polygon, so neither the crossing nor the rung bound can be the cause).
        let mut chains = Vec::new();
        let length = nonzero_ribbon(&regular_polygon(MAX_FILL_VERTICES), &mut chains);
        assert!(length > 0, "{length}");
        assert_eq!(chains.len(), length * 2);
        chains.clear();
        assert_eq!(
            nonzero_ribbon(&regular_polygon(MAX_FILL_VERTICES + 1), &mut chains),
            0
        );
        assert!(chains.is_empty());
    }

    #[test]
    fn polygons_beyond_the_crossing_bound_get_no_fill() {
        // The star {200/99} joins every vertex to the one 99 steps on: 200 vertices, far below
        // the vertex bound, but 200 * 98 = 19,600 proper edge crossings.
        let star: Vec<Point> = (0..200)
            .map(|index| {
                let angle = (index * 99) as f64 * std::f64::consts::TAU / 200.0;
                (100.0 * angle.cos(), 100.0 * angle.sin())
            })
            .collect();
        let mut chains = Vec::new();
        assert_eq!(nonzero_ribbon(&star, &mut chains), 0);
        assert!(chains.is_empty());
    }

    #[test]
    fn simple_polygons_merge_into_few_trapezoids() {
        // A 64-gon: its two sides bound one merged trapezoid per vertex slab at most.
        let polygon: Vec<Point> = (0..64)
            .map(|index| {
                let angle = f64::from(index) * std::f64::consts::TAU / 64.0;
                (100.0 * angle.cos(), 100.0 * angle.sin())
            })
            .collect();
        let mut chains = Vec::new();
        let length = nonzero_ribbon(&polygon, &mut chains);
        assert!(length > 0 && length <= 4 * 64, "{length}");
    }
}

#[cfg(test)]
mod tube_tests {
    use super::flatten_and_fill_tests::{quad_coverage, winding};
    use super::*;

    const WIDE: Rect = Rect {
        left: -1e9,
        top: -1e9,
        right: 1e9,
        bottom: 1e9,
    };

    /// Nonzero coverage of several closed contours (the Canvas2D fill rule).
    fn contours_winding(point: Point, points: &[Point], ends: &[usize]) -> i32 {
        let mut start = 0;
        let mut total = 0;
        for &end in ends {
            total += winding(point, &points[start..end]);
            start = end;
        }
        total
    }

    fn polylines() -> Vec<Vec<Point>> {
        vec![
            // Straight, a zigzag with sharp turns, an exact U-turn, a loop crossing itself,
            // segments far shorter than the radius (pointer jitter), and a spike back and forth.
            vec![(0.0, 20.0), (60.0, 24.0)],
            vec![
                (0.0, 0.0),
                (15.0, 40.0),
                (25.0, 2.0),
                (35.0, 38.0),
                (60.0, 10.0),
            ],
            vec![(5.0, 20.0), (50.0, 20.0), (10.0, 20.0)],
            vec![
                (0.0, 30.0),
                (50.0, 30.0),
                (40.0, 5.0),
                (20.0, 45.0),
                (10.0, 10.0),
            ],
            (0..40)
                .map(|index| {
                    let t = f64::from(index);
                    (
                        5.0 + t * 1.3,
                        25.0 + (t / 4.0).sin() * 9.0 + f64::from(index % 3) * 0.7,
                    )
                })
                .collect(),
            vec![(10.0, 10.0), (40.0, 30.0), (10.0, 10.0), (40.0, 30.0)],
        ]
    }

    #[test]
    fn tube_outlines_fill_exactly_the_points_within_the_radius() {
        let radius = 6.0;
        for line in polylines() {
            let mut outline = Vec::new();
            tube_outline(&line, radius, 0.05, &mut outline);
            for y in -10..60 {
                for x in -10..75 {
                    let point = (f64::from(x) + 0.37, f64::from(y) + 0.41);
                    let distance = distance_to_polyline(point, &line);
                    // Chords sit within the tolerance inside the round parts.
                    if (distance - radius).abs() < 0.1 {
                        continue;
                    }
                    let inside = winding(point, &outline) != 0;
                    assert_eq!(inside, distance < radius, "{point:?} around {line:?}");
                    assert!(winding(point, &outline) >= 0, "one orientation");
                }
            }
        }
        let mut disc = Vec::new();
        tube_outline(&[(5.0, 5.0), (5.0, 5.0)], 4.0, 0.05, &mut disc);
        assert!(disc.len() > 8);
        assert!(disc
            .iter()
            .all(|point| ((point.0 - 5.0).hypot(point.1 - 5.0) - 4.0).abs() < 1e-9));
        let mut empty = Vec::new();
        tube_outline(&[], 4.0, 0.05, &mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn tube_ribbons_cover_the_tube_once_per_pixel() {
        let radius = 6.0;
        for line in polylines() {
            let mut chains = Vec::new();
            let length = tube_ribbon(&line, radius, 0.05, WIDE, &mut chains);
            assert!(length > 0, "{line:?}");
            let (upper, lower) = chains.split_at(length);
            for y in -10..60 {
                for x in -10..75 {
                    let point = (f64::from(x) + 0.37, f64::from(y) + 0.41);
                    let coverage = quad_coverage(point, upper, lower);
                    assert!(
                        coverage <= 1,
                        "{point:?} blends {coverage} times on {line:?}"
                    );
                    // Simplification and chords move the edge by at most a few tolerances.
                    let distance = distance_to_polyline(point, &line);
                    if (distance - radius).abs() > 0.3 {
                        assert_eq!(
                            coverage == 1,
                            distance < radius,
                            "{point:?} around {line:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn tube_ribbons_skip_what_cannot_reach_the_clip_and_stay_bounded() {
        let clip = Rect {
            left: 0.0,
            top: 0.0,
            right: 100.0,
            bottom: 100.0,
        };
        // A stroke leaving and re-entering the clip keeps both visible runs as one region.
        let line = [(10.0, 50.0), (500.0, 50.0), (500.0, 80.0), (10.0, 80.0)];
        let mut chains = Vec::new();
        let length = tube_ribbon(&line, 5.0, 0.25, clip, &mut chains);
        let (upper, lower) = chains.split_at(length);
        assert_eq!(quad_coverage((50.3, 50.2), upper, lower), 1);
        assert_eq!(quad_coverage((50.3, 80.2), upper, lower), 1);
        assert_eq!(quad_coverage((50.3, 65.2), upper, lower), 0);
        assert_eq!(
            quad_coverage((500.3, 65.2), upper, lower),
            0,
            "the segment that cannot reach the clip is dropped"
        );
        chains.clear();
        assert_eq!(
            tube_ribbon(
                &[(300.0, 300.0), (400.0, 300.0)],
                5.0,
                0.25,
                clip,
                &mut chains
            ),
            0
        );

        // A 100,000-sample jittery stroke 20,000 px long: only the part that can reach a
        // 2,000 px clip is outlined, simplified from its 10,000 samples there.
        let pane = Rect {
            left: 0.0,
            top: 0.0,
            right: 2_000.0,
            bottom: 600.0,
        };
        let huge = (0..100_000)
            .map(|index| {
                let t = f64::from(index);
                (
                    t * 0.2,
                    300.0 + (t / 900.0).sin() * 200.0 + f64::from(index % 5) * 0.3,
                )
            })
            .collect::<Vec<_>>();
        chains.clear();
        let length = tube_ribbon(&huge, 10.0, 0.25, pane, &mut chains);
        assert!(length > 0 && length < 4_000, "{length}");
        // Steep 400 px swings every 63 px across the whole clip stay beyond the bounds even at the
        // coarsest tolerance: no ribbon (the caller strokes it instead), after bounded work.
        let wide = Rect {
            right: 20_000.0,
            ..pane
        };
        let steep = huge
            .iter()
            .enumerate()
            .map(|(index, point)| (point.0, 300.0 + (index as f64 / 50.0).sin() * 200.0))
            .collect::<Vec<_>>();
        chains.clear();
        assert_eq!(tube_ribbon(&steep, 10.0, 0.25, wide, &mut chains), 0);
        assert!(chains.is_empty());
        assert_eq!(
            tube_ribbon(&huge[..2], f64::NAN, 0.25, wide, &mut chains),
            0
        );
    }

    #[test]
    fn simplified_polylines_stay_within_tolerance() {
        let jitter = (0..400)
            .map(|index| {
                let t = f64::from(index);
                (
                    t * 1.5,
                    100.0 + (t / 30.0).sin() * 40.0 + f64::from(index % 2) * 0.2,
                )
            })
            .collect::<Vec<_>>();
        for tolerance in [0.25, 1.0, 4.0] {
            let mut kept = Vec::new();
            simplify_polyline(&jitter, tolerance, &mut kept);
            assert_eq!(kept[0], jitter[0]);
            assert_eq!(kept.last(), jitter.last());
            assert!(
                kept.len() < jitter.len() / 3,
                "{} kept at {tolerance}",
                kept.len()
            );
            for point in &jitter {
                assert!(distance_to_polyline(*point, &kept) <= tolerance + 1e-9);
            }
        }
        // Backtracking keeps the turning point.
        let mut kept = Vec::new();
        simplify_polyline(
            &[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (5.0, 0.0)],
            0.5,
            &mut kept,
        );
        assert_eq!(kept, vec![(0.0, 0.0), (20.0, 0.0), (5.0, 0.0)]);
        kept.clear();
        simplify_polyline(&[(1.0, 1.0)], 0.5, &mut kept);
        assert_eq!(kept, vec![(1.0, 1.0)]);
    }

    #[test]
    fn contours_combine_under_the_nonzero_rule() {
        // Two overlapping squares of one orientation fill their union once; a reversed inner
        // square cuts a hole.
        let points = [
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 20.0),
            (0.0, 20.0),
            (10.0, 10.0),
            (30.0, 10.0),
            (30.0, 30.0),
            (10.0, 30.0),
            (22.0, 22.0),
            (22.0, 26.0),
            (26.0, 26.0),
            (26.0, 22.0),
        ];
        let ends = [4, 8, 12];
        let mut chains = Vec::new();
        let length = nonzero_ribbon_contours(&points, &ends, &mut chains);
        let (upper, lower) = chains.split_at(length);
        for y in -2..34 {
            for x in -2..34 {
                let point = (f64::from(x) + 0.37, f64::from(y) + 0.41);
                assert_eq!(
                    quad_coverage(point, upper, lower),
                    usize::from(contours_winding(point, &points, &ends) != 0),
                    "{point:?}"
                );
            }
        }
        assert_eq!(
            nonzero_ribbon_contours(&points, &[4, 8], &mut chains),
            0,
            "ends cover all"
        );
    }
}
