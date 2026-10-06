//! Backend-neutral resolved drawing geometry.
//!
//! A tool's semantic anchors are converted once into this small geometry vocabulary.  Frame
//! lowering and precise hit-testing both consume the same result, preventing the rendered shape
//! and the interactive shape from drifting as more drawing kinds are added.

use aeris_charts_render::draw_list::LineType;
use aeris_charts_render::shape::{self, EllipseArc, Point, Rect};

use super::{path_arrow_points, Drawing, DrawingKind, TextBox};
use crate::DrawingLevel;

/// Chord tolerance of a flattened curve (ellipse, circle, arc, Bézier): device px in the frame,
/// media px in hit testing, so paint and hit stay within a quarter pixel of the true curve.
pub(crate) const CURVE_TOLERANCE: f64 = 0.25;
/// Reach in CSS px beyond a stroke's half width past which a curve piece can neither paint nor
/// hit inside the pane (it covers the touch hit tolerance).
const CURVE_CLIP_MARGIN: f64 = 16.0;

/// The clip a curve flattens against: `pane` (in the caller's px) grown by the stroke's half
/// width plus [`CURVE_CLIP_MARGIN`], `scale` caller px per CSS px. Pieces outside it become single
/// chords, so a huge zoomed-in curve costs bounded work.
pub(crate) fn curve_clip(pane: Rect, line_width: f64, scale: f64) -> Rect {
    pane.inflate((line_width / 2.0 + CURVE_CLIP_MARGIN) * scale)
}

/// The outline of the axis-aligned ellipse around `center` with radii `rx`/`ry`, flattened against
/// `clip` (see [`EllipseArc::append_clipped_points`]): starts at angle 0 and repeats its first
/// point last.
pub(crate) fn ellipse_outline(center: Point, rx: f64, ry: f64, clip: Rect, out: &mut Vec<Point>) {
    EllipseArc {
        center,
        rx,
        ry,
        rotation: 0.0,
        start: 0.0,
        sweep: std::f64::consts::TAU,
    }
    .append_clipped_points(CURVE_TOLERANCE, clip, out);
}

/// The outline of the closed polygon through `vertices` as one run (a rotated rectangle, a
/// triangle, a closed polyline): it starts and ends at the midpoint of the first edge of nonzero
/// length, so the stroke's two butt ends meet collinearly mid-edge instead of notching a corner,
/// and every corner is a join. Vertices repeating their predecessor are skipped. Nothing when
/// every vertex coincides.
pub(crate) fn closed_outline(vertices: &[Point], out: &mut Vec<Point>) {
    let count = vertices.len();
    let Some(first) = (0..count).find(|&index| vertices[index] != vertices[(index + 1) % count])
    else {
        return;
    };
    let start = shape::midpoint(vertices[first], vertices[(first + 1) % count]);
    out.reserve(count + 2);
    out.push(start);
    for step in 1..=count {
        let vertex = vertices[(first + step) % count];
        if out.last() != Some(&vertex) {
            out.push(vertex);
        }
    }
    out.push(start);
}

/// The bands `drawing` fills between its `levels` (its own or another level list it owns), in
/// list order, as `(previous level's raw value, level)`: visible levels chain in list order, a
/// hidden level (or, with `positive_only`, one whose effective value is not positive) breaks the
/// chain, and a level fills toward its predecessor while the drawing's fill is on and the level's
/// `fill_between` is set. The Fibonacci, time-level, Fibonacci-arc and pitchfork band loops follow
/// this rule; the Gann box's grid, fan and arc band loops still chain across a hidden level.
pub(crate) fn level_band_pairs<'a>(
    drawing: &'a Drawing,
    levels: &'a [DrawingLevel],
    positive_only: bool,
) -> impl Iterator<Item = (f64, &'a DrawingLevel)> {
    let levels = if drawing.fill_enabled { levels } else { &[] };
    let mut previous = None;
    levels.iter().filter_map(move |level| {
        if !level.visible || (positive_only && drawing.level_value(level.value) <= 0.0) {
            previous = None;
            return None;
        }
        let prior = previous.replace(level.value)?;
        level.fill_between.then_some((prior, level))
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DrawingBodyGeometry<'a> {
    Empty,
    Segment {
        a: (f64, f64),
        b: (f64, f64),
    },
    Horizontal {
        y: f64,
        x0: f64,
        x1: f64,
    },
    Vertical {
        x: f64,
        y0: f64,
        y1: f64,
    },
    Cross {
        x: f64,
        y: f64,
        pane_w: f64,
        pane_top: f64,
        pane_bottom: f64,
    },
    Channel {
        first: [(f64, f64); 2],
        second: [(f64, f64); 2],
    },
    Regression {
        center: [(f64, f64); 2],
        upper: [(f64, f64); 2],
        lower: [(f64, f64); 2],
    },
    /// A regression trend without a fit (fewer than two source closes in its window: the future
    /// area, data not loaded yet, or replay rewound before it): the dashed segment between its
    /// anchors, which keeps the drawing visible and selectable until data arrives (the fork's
    /// placeholder, kept on upstream's resolver).
    RegressionWindow {
        a: (f64, f64),
        b: (f64, f64),
    },
    Fibonacci(FibonacciGeometry),
    TimeLevels(TimeLevelGeometry),
    FibonacciArcs(FibonacciArcGeometry),
    Pitchfork(PitchforkGeometry),
    Cycles(CycleGeometry),
    Sine(SineGeometry),
    Marker(MarkerGeometry),
    PriceLabel {
        x: f64,
        y: f64,
    },
    IconStamp {
        center: (f64, f64),
        size: f64,
    },
    GannGrid(GannGridGeometry),
    Quad {
        corners: [(f64, f64); 4],
    },
    Ellipse {
        center: (f64, f64),
        rx: f64,
        ry: f64,
    },
    Circle {
        center: (f64, f64),
        radius: f64,
    },
    Triangle {
        corners: [(f64, f64); 3],
    },
    Arc(ArcGeometry),
    Curve(CurveGeometry),
    /// A closed polyline (`tool_options.shape.closed`, three vertices or more): the last vertex
    /// joins the first, and the enclosed region fills by the nonzero rule.
    Polygon {
        points: &'a [(f64, f64)],
    },
    Rectangle {
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
    },
    Position(PositionGeometry),
    Polyline {
        points: &'a [(f64, f64)],
        line_type: LineType,
        terminal: Option<[(f64, f64); 3]>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionGeometry {
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) entry_y: f64,
    pub(crate) target_y: f64,
    pub(crate) stop_y: f64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FibonacciGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) start: (f64, f64),
    pub(crate) end: (f64, f64),
    pub(crate) pivot: Option<(f64, f64)>,
    pub(crate) x0: f64,
    pub(crate) x1: f64,
}

impl FibonacciGeometry {
    pub(crate) fn segment(self, value: f64) -> ((f64, f64), (f64, f64)) {
        match self.kind {
            DrawingKind::FibonacciSpeedFan => (
                self.start,
                (
                    self.end.0,
                    self.start.1 + (self.end.1 - self.start.1) * value,
                ),
            ),
            DrawingKind::GannFan => {
                let dx = self.end.0 - self.start.0;
                if dx.abs() < 1e-9 {
                    (self.start, self.end)
                } else {
                    let edge_x = if dx >= 0.0 { self.x1 } else { 0.0 };
                    (
                        self.start,
                        (
                            edge_x,
                            self.start.1
                                + (edge_x - self.start.0) * (self.end.1 - self.start.1) * value
                                    / dx,
                        ),
                    )
                }
            }
            DrawingKind::FibonacciExtension => {
                let pivot = self.pivot.unwrap_or(self.end);
                let y = pivot.1 + (self.end.1 - self.start.1) * value;
                ((self.x0, y), (self.x1, y))
            }
            DrawingKind::FibonacciChannel => {
                let pivot = self.pivot.unwrap_or(self.end);
                let dx = self.end.0 - self.start.0;
                let base_y = if dx.abs() > f64::EPSILON {
                    self.start.1 + (pivot.0 - self.start.0) * (self.end.1 - self.start.1) / dx
                } else {
                    self.start.1
                };
                let offset = (pivot.1 - base_y) * value;
                let mut ends = [
                    (self.start.0, self.start.1 + offset),
                    (self.end.0, self.end.1 + offset),
                ];
                // `extend_left`/`extend_right` moved `x0`/`x1` to the pane's edges: each level
                // runs along itself to the edge on that side of the screen (unextended, `x0` and
                // `x1` are the anchors' own span and nothing moves).
                if dx.abs() > f64::EPSILON {
                    let slope = (self.end.1 - self.start.1) / dx;
                    let (left, right) = if dx > 0.0 { (0, 1) } else { (1, 0) };
                    for (index, x) in [(left, self.x0), (right, self.x1)] {
                        if x != ends[index].0 {
                            ends[index] = (x, ends[index].1 + (x - ends[index].0) * slope);
                        }
                    }
                }
                (ends[0], ends[1])
            }
            _ => {
                let y = self.start.1 + (self.end.1 - self.start.1) * value;
                ((self.x0, y), (self.x1, y))
            }
        }
    }
}

impl FibonacciGeometry {
    /// The speed resistance fan's grid lines at the effective level `value` inside the anchors'
    /// box: the horizontal line through the level's price ray end, from the first anchor's time
    /// to the second's, and the vertical line at the same ratio of the anchors' time span, from
    /// the first anchor's price to the second's.
    pub(crate) fn grid_lines(self, value: f64) -> [((f64, f64), (f64, f64)); 2] {
        let y = self.start.1 + (self.end.1 - self.start.1) * value;
        let x = self.start.0 + (self.end.0 - self.start.0) * value;
        [
            ((self.start.0, y), (self.end.0, y)),
            ((x, self.start.1), (x, self.end.1)),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimeLevelGeometry {
    pub(crate) origin_x: f64,
    pub(crate) step_x: f64,
    pub(crate) pane_top: f64,
    pub(crate) pane_bottom: f64,
}

impl TimeLevelGeometry {
    pub(crate) fn x(self, value: f64) -> f64 {
        self.origin_x + self.step_x * value
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FibonacciArcGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) center: (f64, f64),
    pub(crate) radius: f64,
    pub(crate) start_angle: f64,
    pub(crate) sweep: f64,
}

impl FibonacciArcGeometry {
    pub(crate) fn point(self, value: f64, t: f64) -> (f64, f64) {
        let angle = if self.kind == DrawingKind::FibonacciWedge {
            self.start_angle + self.sweep * t
        } else {
            self.start_angle - self.sweep / 2.0 + self.sweep * t
        };
        let radius = if self.kind == DrawingKind::FibonacciSpiral {
            // A bounded logarithmic spiral reaches the second anchor at t=1.
            let growth = (std::f64::consts::PI * t).exp_m1() / std::f64::consts::PI.exp_m1();
            self.radius * value * growth
        } else {
            self.radius * value
        };
        (
            self.center.0 + radius * angle.cos(),
            self.center.1 + radius * angle.sin(),
        )
    }

    pub(crate) fn segments(self) -> u32 {
        if self.kind == DrawingKind::FibonacciSpiral {
            96
        } else {
            32
        }
    }

    /// The whole arc every level of a ring tool (speed arcs, circles, wedge) follows, as its
    /// start angle and signed sweep: the parametrisation of [`Self::point`].
    fn arc(self) -> (f64, f64) {
        if self.kind == DrawingKind::FibonacciWedge {
            (self.start_angle, self.sweep)
        } else {
            (self.start_angle - self.sweep / 2.0, self.sweep)
        }
    }

    /// The precise rings of a ring tool (the fork's tessellation, `kinds::fibonacci::precise_rings`)
    /// against `pane` (caller px), for strokes of half width `half` and levels up to the radius
    /// `largest`: only the part of the arc the pane shows is tessellated, within
    /// [`CURVE_TOLERANCE`] at the largest radius that can show (so at every smaller one), and a
    /// radius beyond the farthest pane point closes at it. `None` when the arc misses the pane.
    pub(crate) fn rings(self, pane: Rect, half: f64, largest: f64) -> Option<Rings> {
        if self.kind == DrawingKind::FibonacciSpiral
            || self.radius.is_nan()
            || self.radius <= f64::EPSILON
        {
            return None;
        }
        let (near, far) = pane_distances(pane, self.center);
        let cap = far + half + 1.0;
        let window = visible_arc(pane, self.center, self.arc())?;
        let segments = shape::arc_segment_count(largest.min(cap), window.1, CURVE_TOLERANCE);
        Some(Rings {
            center: self.center,
            arc: self.arc(),
            window,
            segments,
            near,
            far,
            cap,
            half,
        })
    }

    /// The golden spiral (`kinds::fibonacci::phi_spiral`) around the center through the second
    /// anchor, growing by φ every quarter turn, clockwise on screen (counterclockwise with
    /// `reverse`), from [`SPIRAL_MIN_RADIUS`] CSS px (`scale` caller px each) until it leaves
    /// `pane` (caller px) for good, as runs handed to `emit`. Only the part of each quarter turn
    /// inside the pane's angular window is tessellated (within [`CURVE_TOLERANCE`]), quarter turns
    /// that cannot reach the pane are skipped, and at most [`MAX_SPIRAL_QUARTERS`] are walked, so
    /// a spiral centered far off the pane costs bounded work. With a dash `period` (caller px; 0
    /// when solid) a run starts at the last dash-period boundary before it, in arc length from the
    /// inner end, so its dashes stay put while the pane scrolls.
    pub(crate) fn phi_spiral(
        self,
        reverse: bool,
        pane: Rect,
        (half, scale, period): (f64, f64, f64),
        mut emit: impl FnMut(&[Point]),
    ) {
        use std::f64::consts::{FRAC_PI_2, PI, TAU};
        let (a, r0, theta0) = (self.center, self.radius, self.start_angle);
        if r0.is_nan() || r0 <= f64::EPSILON || !theta0.is_finite() {
            return;
        }
        let turn = if reverse { -1.0 } else { 1.0 };
        // r = r0·e^(k·t) after turning `t` radians past the second anchor; k = ln φ / (π/2).
        let golden = (1.0 + 5.0_f64.sqrt()) / 2.0;
        let growth = golden.ln() / FRAC_PI_2;
        let radius = |t: f64| r0 * (growth * t).exp();
        let turned = |r: f64| (r / r0).ln() / growth;
        // A logarithmic spiral's arc length grows linearly with its radius.
        let length_per_radius = (1.0 + growth * growth).sqrt() / growth;
        let (near, far) = pane_distances(pane, a);
        let inner = (SPIRAL_MIN_RADIUS * scale).min(r0);
        let start = turned(inner);
        // Past the farthest pane point the radius only grows, so the spiral never returns.
        let end = turned(far + half);
        let quarters = ((end - start) / FRAC_PI_2)
            .ceil()
            .clamp(1.0, MAX_SPIRAL_QUARTERS as f64) as usize;
        let mut run: Vec<Point> = Vec::new();
        let mut run_end = f64::NAN;
        for index in 0..quarters {
            let t0 = start + FRAC_PI_2 * index as f64;
            if radius(t0 + FRAC_PI_2) + half < near {
                continue;
            }
            let angle0 = (theta0 + turn * t0 + PI).rem_euclid(TAU) - PI;
            let Some((from, sweep)) = visible_arc(pane, a, (angle0, turn * FRAC_PI_2)) else {
                continue;
            };
            let (t_from, t_to) = (
                t0 + (from - angle0) * turn,
                t0 + (from + sweep - angle0) * turn,
            );
            let t_start = if (t_from - run_end).abs() <= 1e-9 {
                // This piece continues the previous one.
                t_from
            } else {
                if run.len() >= 2 {
                    emit(&run);
                }
                run.clear();
                if period > 0.0 {
                    let travelled = (radius(t_from) - inner) * length_per_radius;
                    turned(radius(t_from) - travelled.rem_euclid(period) / length_per_radius)
                } else {
                    t_from
                }
            };
            let segments = shape::arc_segment_count(radius(t_to), t_to - t_start, CURVE_TOLERANCE);
            let first = usize::from(!run.is_empty());
            for step in first..=segments {
                let t = t_start + (t_to - t_start) * step as f64 / segments as f64;
                let (r, angle) = (radius(t), theta0 + turn * t);
                run.push((a.0 + r * angle.cos(), a.1 + r * angle.sin()));
            }
            run_end = t_to;
        }
        if run.len() >= 2 {
            emit(&run);
        }
    }
}

/// Smallest radius of the golden spiral, CSS px: it starts below a pixel, at its center.
const SPIRAL_MIN_RADIUS: f64 = 0.5;
/// Most quarter turns the golden spiral walks (φ^128 covers any pane from a sub-pixel start).
const MAX_SPIRAL_QUARTERS: usize = 128;

/// A ring tool's concentric arcs over the part of the pane they show
/// ([`FibonacciArcGeometry::rings`]): every ring and band chain shares the window's angles, so
/// paired band chains match point for point.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rings {
    center: Point,
    /// The whole arc: start angle and signed sweep.
    arc: (f64, f64),
    /// The part of `arc` the pane shows.
    window: (f64, f64),
    segments: usize,
    /// Nearest and farthest pane point from the center.
    near: f64,
    far: f64,
    /// Radius a ring beyond `far` closes at: the same pane pixels, with finite points.
    cap: f64,
    half: f64,
}

impl Rings {
    /// Whether the band between radii `inner` and `outer` (a ring when equal) can reach the pane.
    pub(crate) fn reaches(&self, inner: f64, outer: f64) -> bool {
        outer + self.half >= self.near && inner - self.half <= self.far
    }

    /// The window's chain at `radius` (closed at the cap), replacing `out`.
    pub(crate) fn chain(&self, radius: f64, out: &mut Vec<Point>) {
        out.clear();
        let radius = radius.min(self.cap);
        let (start, sweep) = self.window;
        out.extend((0..=self.segments).map(|step| {
            let angle = start + sweep * step as f64 / self.segments as f64;
            (
                self.center.0 + radius * angle.cos(),
                self.center.1 + radius * angle.sin(),
            )
        }));
    }

    /// Stroke the ring of `radius` over the window, as runs handed to `emit`. With a dash
    /// `period` (caller px; 0 when solid) a ring the pane shows only in part starts each piece at
    /// the last dash-period boundary before the window, in arc length from the arc's start, so
    /// dashes stay put while the pane scrolls instead of following the window's edge.
    pub(crate) fn stroke(
        &self,
        radius: f64,
        period: f64,
        out: &mut Vec<Point>,
        mut emit: impl FnMut(&[Point]),
    ) {
        if period <= 0.0 || self.window == self.arc {
            self.chain(radius, out);
            emit(out);
            return;
        }
        let (arc_start, sweep) = self.arc;
        let direction = sweep.signum();
        let length = self.window.1.abs();
        // How far along the arc the window starts, within one turn.
        let from = ((self.window.0 - arc_start) * direction).rem_euclid(std::f64::consts::TAU);
        let wrapped = (from + length - std::f64::consts::TAU).max(0.0);
        for (start, span) in [(from, length - wrapped), (0.0, wrapped)] {
            if span <= 0.0 {
                continue;
            }
            let back = (start * radius).rem_euclid(period) / radius;
            let piece = shape::EllipseArc::circle(
                self.center,
                radius,
                arc_start + direction * (start - back),
                direction * (span + back),
            );
            out.clear();
            piece.append_points(CURVE_TOLERANCE, out);
            emit(out);
        }
    }
}

/// Distances from `point` to the nearest and the farthest point of `pane`.
fn pane_distances(pane: Rect, point: Point) -> (f64, f64) {
    let dx = (pane.left - point.0).max(point.0 - pane.right).max(0.0);
    let dy = (pane.top - point.1).max(point.1 - pane.bottom).max(0.0);
    let far = pane_corners(pane)
        .iter()
        .map(|corner| (corner.0 - point.0).hypot(corner.1 - point.1))
        .fold(0.0_f64, f64::max);
    (dx.hypot(dy), far)
}

fn pane_corners(pane: Rect) -> [Point; 4] {
    [
        (pane.left, pane.top),
        (pane.right, pane.top),
        (pane.right, pane.bottom),
        (pane.left, pane.bottom),
    ]
}

/// The part of the arc `(start, sweep)` around `center` that can reach `pane`: the whole arc
/// when the center lies in the pane, else its overlap with the angular window the pane subtends
/// from the center (under π wide; a full turn becomes the window, and an arc of at most π meets
/// it in one piece), keeping the arc's direction. `None` when they miss.
fn visible_arc(pane: Rect, center: Point, (start, sweep): (f64, f64)) -> Option<(f64, f64)> {
    use std::f64::consts::{PI, TAU};
    if pane.contains(center) {
        return Some((start, sweep));
    }
    let reference = ((pane.top + pane.bottom) / 2.0 - center.1)
        .atan2((pane.left + pane.right) / 2.0 - center.0);
    let (mut low, mut high) = (0.0_f64, 0.0_f64);
    for corner in pane_corners(pane) {
        let angle = (corner.1 - center.1).atan2(corner.0 - center.0);
        let delta = (angle - reference + PI).rem_euclid(TAU) - PI;
        low = low.min(delta);
        high = high.max(delta);
    }
    let window = (reference + low, reference + high);
    if sweep.abs() >= TAU - 1e-9 {
        return Some((window.0, window.1 - window.0));
    }
    let arc = if sweep >= 0.0 {
        (start, start + sweep)
    } else {
        (start + sweep, start)
    };
    // Both ranges lie within (-3π, 3π], so shifts of up to two turns align them.
    (-2..=2).find_map(|turns| {
        let shift = TAU * f64::from(turns);
        let (from, to) = (arc.0.max(window.0 + shift), arc.1.min(window.1 + shift));
        (to > from).then_some(if sweep >= 0.0 {
            (from, to - from)
        } else {
            (to, from - to)
        })
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PitchforkGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) pivot: (f64, f64),
    pub(crate) b: (f64, f64),
    pub(crate) c: (f64, f64),
    pub(crate) pane_w: f64,
    pub(crate) pane_top: f64,
    pub(crate) pane_bottom: f64,
}

impl PitchforkGeometry {
    pub(crate) fn anchor(self, value: f64) -> (f64, f64) {
        let value = if self.kind == DrawingKind::InsidePitchfork {
            value * 2.0
        } else {
            value
        };
        (
            self.b.0 + (self.c.0 - self.b.0) * value,
            self.b.1 + (self.c.1 - self.b.1) * value,
        )
    }

    pub(crate) fn segment(self, value: f64) -> ((f64, f64), (f64, f64)) {
        let anchor = self.anchor(value);
        let midpoint = self.anchor(0.5);
        let (start, dx, dy) = if self.kind == DrawingKind::Pitchfan {
            (self.pivot, anchor.0 - self.pivot.0, anchor.1 - self.pivot.1)
        } else {
            let start = if (value - 0.5).abs() < f64::EPSILON {
                self.pivot
            } else {
                anchor
            };
            (start, midpoint.0 - self.pivot.0, midpoint.1 - self.pivot.1)
        };
        let end = if dx.abs() > f64::EPSILON {
            let edge = if dx > 0.0 { self.pane_w } else { 0.0 };
            (edge, start.1 + (edge - start.0) * dy / dx)
        } else {
            (
                start.0,
                if dy >= 0.0 {
                    self.pane_bottom
                } else {
                    self.pane_top
                },
            )
        };
        (start, end)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CycleGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) origin_x: f64,
    pub(crate) step_x: f64,
    pub(crate) pane_w: f64,
    pub(crate) pane_top: f64,
    pub(crate) pane_bottom: f64,
}

impl CycleGeometry {
    pub(crate) fn for_each_visible_line(self, mut emit: impl FnMut(i64, f64)) {
        if !self.step_x.is_finite() || self.step_x.abs() < 1e-6 {
            return;
        }
        let at_left = -self.origin_x / self.step_x;
        let at_right = (self.pane_w - self.origin_x) / self.step_x;
        let first = at_left.min(at_right).ceil().max(-1_000_000.0);
        let last = at_left.max(at_right).floor().min(1_000_000.0);
        let first = if self.kind == DrawingKind::TimeCycles {
            first.max(0.0)
        } else {
            first
        } as i64;
        let last = last as i64;
        if last < first {
            return;
        }
        let stride = ((last - first + 256) / 256).max(1);
        let mut index = first;
        while index <= last {
            emit(index, self.origin_x + index as f64 * self.step_x);
            index += stride;
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SineGeometry {
    pub(crate) pivot: (f64, f64),
    pub(crate) quarter_width: f64,
    pub(crate) amplitude: f64,
    pub(crate) pane_w: f64,
}

impl SineGeometry {
    pub(crate) fn visible_x(self) -> Option<(f64, f64)> {
        let (left, right) = if self.quarter_width > 0.0 {
            (self.pivot.0.max(0.0), self.pane_w)
        } else {
            (0.0, self.pivot.0.min(self.pane_w))
        };
        (right > left).then_some((left, right))
    }

    pub(crate) fn y(self, x: f64) -> f64 {
        self.pivot.1
            + self.amplitude
                * (std::f64::consts::FRAC_PI_2 * (x - self.pivot.0) / self.quarter_width).sin()
    }

    pub(crate) fn sample_count(self) -> u32 {
        let span = self.visible_x().map_or(0.0, |(left, right)| right - left);
        ((span / 2.0).ceil() as u32).clamp(2, 512)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MarkerGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) anchor: (f64, f64),
    pub(crate) base: Option<(f64, f64)>,
    pub(crate) radius: f64,
}

impl MarkerGeometry {
    pub(crate) fn triangle(self) -> [(f64, f64); 3] {
        let (x, y) = self.anchor;
        let r = self.radius;
        match self.kind {
            DrawingKind::ArrowMarkerUp => [(x, y), (x - r, y + 2.0 * r), (x + r, y + 2.0 * r)],
            DrawingKind::ArrowMarkerDown => [(x, y), (x - r, y - 2.0 * r), (x + r, y - 2.0 * r)],
            DrawingKind::ArrowMarkerLeft => [(x, y), (x + 2.0 * r, y - r), (x + 2.0 * r, y + r)],
            DrawingKind::ArrowMarkerRight => [(x, y), (x - 2.0 * r, y - r), (x - 2.0 * r, y + r)],
            _ => [(x, y - 2.0 * r), (x + 2.0 * r, y - 1.5 * r), (x, y - r)],
        }
    }

    pub(crate) fn stem(self) -> Option<((f64, f64), (f64, f64))> {
        match self.kind {
            DrawingKind::FlagMark => Some((
                self.anchor,
                (self.anchor.0, self.anchor.1 - 2.0 * self.radius),
            )),
            DrawingKind::Signpost => Some((self.base?, self.anchor)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ArcGeometry {
    pub(crate) center: (f64, f64),
    pub(crate) radius: f64,
    pub(crate) start: f64,
    pub(crate) sweep: f64,
}

impl ArcGeometry {
    /// The exact arc point at `t` in `[0, 1]` (the reference its flattening approximates).
    #[cfg(test)]
    pub(crate) fn point(self, t: f64) -> (f64, f64) {
        let angle = self.start + self.sweep * t;
        (
            self.center.0 + self.radius * angle.cos(),
            self.center.1 + self.radius * angle.sin(),
        )
    }

    /// The arc flattened against `clip` within [`CURVE_TOLERANCE`], both ends included, at most
    /// [`shape::MAX_FLATTEN_POINTS`] + 1 points (the same parametrisation as `point`).
    pub(crate) fn flatten(self, clip: Rect, out: &mut Vec<Point>) {
        EllipseArc::circle(self.center, self.radius, self.start, self.sweep).append_clipped_points(
            CURVE_TOLERANCE,
            clip,
            out,
        );
    }

    /// For each end, the point one caller px from it along the arc's exact tangent, into the arc
    /// (the direction an end cap points away from).
    pub(crate) fn end_towards(self) -> [Point; 2] {
        let travel = self.sweep.signum();
        let along = |angle: f64, sign: f64| {
            let (sin, cos) = angle.sin_cos();
            (
                self.center.0 + self.radius * cos - sin * travel * sign,
                self.center.1 + self.radius * sin + cos * travel * sign,
            )
        };
        [along(self.start, 1.0), along(self.start + self.sweep, -1.0)]
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CurveGeometry {
    pub(crate) points: [(f64, f64); 4],
    pub(crate) cubic: bool,
    /// Where `extend_left` / `extend_right` continue the start's and the end's tangent to the
    /// pane edge (`None`: not extended, or the tangent never reaches the pane).
    pub(crate) extend: [Option<(f64, f64)>; 2],
}

impl CurveGeometry {
    /// The curve's defining points: start, control(s), end.
    fn defining(&self) -> &[(f64, f64)] {
        &self.points[..if self.cubic { 4 } else { 3 }]
    }

    /// The start and the end point.
    pub(crate) fn ends(self) -> [Point; 2] {
        let points = self.defining();
        [points[0], points[points.len() - 1]]
    }

    /// For each end, the point its tangent runs toward: the nearest control point distinct from
    /// it, else the other end (`None` when every point coincides with that end). Tangent
    /// extensions and end caps follow it.
    pub(crate) fn end_towards(self) -> [Option<Point>; 2] {
        let points = self.defining();
        let [first, last] = self.ends();
        [
            points[1..].iter().copied().find(|&point| point != first),
            points[..points.len() - 1]
                .iter()
                .rev()
                .copied()
                .find(|&point| point != last),
        ]
    }

    /// The exact Bézier point at `t` in `[0, 1]` (the reference its flattening approximates, and
    /// where the on-curve handles sit).
    pub(crate) fn point(self, t: f64) -> (f64, f64) {
        let u = 1.0 - t;
        let weights = if self.cubic {
            [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t]
        } else {
            [u * u, 2.0 * u * t, t * t, 0.0]
        };
        let mut x = 0.0;
        let mut y = 0.0;
        for (point, weight) in self.points.into_iter().zip(weights) {
            x += point.0 * weight;
            y += point.1 * weight;
        }
        (x, y)
    }

    /// The curve flattened against `clip` within [`CURVE_TOLERANCE`], both ends included, at most
    /// [`shape::MAX_FLATTEN_POINTS`] + 1 points.
    pub(crate) fn flatten(self, clip: Rect, out: &mut Vec<Point>) {
        let [p0, p1, p2, p3] = self.points;
        if self.cubic {
            shape::flatten_cubic(p0, p1, p2, p3, CURVE_TOLERANCE, clip, out);
        } else {
            shape::flatten_quadratic(p0, p1, p2, CURVE_TOLERANCE, clip, out);
        }
    }
}

fn arc_through(points: [(f64, f64); 3]) -> Option<ArcGeometry> {
    let [(x0, y0), (x1, y1), (x2, y2)] = points;
    let d = 2.0 * (x0 * (y1 - y2) + x1 * (y2 - y0) + x2 * (y0 - y1));
    if d.abs() <= 1e-9 {
        return None;
    }
    let q0 = x0 * x0 + y0 * y0;
    let q1 = x1 * x1 + y1 * y1;
    let q2 = x2 * x2 + y2 * y2;
    let cx = (q0 * (y1 - y2) + q1 * (y2 - y0) + q2 * (y0 - y1)) / d;
    let cy = (q0 * (x2 - x1) + q1 * (x0 - x2) + q2 * (x1 - x0)) / d;
    let radius = (x0 - cx).hypot(y0 - cy);
    if !cx.is_finite() || !cy.is_finite() || !radius.is_finite() || radius > 1e6 {
        return None;
    }
    let start = (y0 - cy).atan2(x0 - cx);
    let middle = (y1 - cy).atan2(x1 - cx);
    let end = (y2 - cy).atan2(x2 - cx);
    let forward = (end - start).rem_euclid(std::f64::consts::TAU);
    let through = (middle - start).rem_euclid(std::f64::consts::TAU);
    let sweep = if through <= forward {
        forward
    } else {
        forward - std::f64::consts::TAU
    };
    Some(ArcGeometry {
        center: (cx, cy),
        radius,
        start,
        sweep,
    })
}

/// Which dimensions a measuring tool reports. Price arrows run vertically, time arrows
/// horizontally; the combined tool draws both through the box center.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeasureAxes {
    Price,
    Date,
    DatePrice,
}

impl MeasureAxes {
    pub(crate) const fn for_kind(kind: DrawingKind) -> Option<Self> {
        match kind {
            DrawingKind::PriceRange => Some(Self::Price),
            DrawingKind::DateRange => Some(Self::Date),
            DrawingKind::DatePriceRange => Some(Self::DatePrice),
            _ => None,
        }
    }

    pub(crate) const fn price(self) -> bool {
        matches!(self, Self::Price | Self::DatePrice)
    }

    pub(crate) const fn date(self) -> bool {
        matches!(self, Self::Date | Self::DatePrice)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositionZone {
    pub(crate) left: f64,
    pub(crate) right: f64,
    pub(crate) y0: f64,
    pub(crate) y1: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DrawingGeometryOptions {
    pub(crate) line_width: f64,
    pub(crate) device_scale: f64,
    pub(crate) icon_size: f64,
    pub(crate) extend_left: bool,
    pub(crate) extend_right: bool,
    /// Speed resistance arcs sweep full circles (`tool_options.fibonacci.full_circles`).
    pub(crate) full_circles: bool,
    /// A polyline joins its last vertex to its first (`tool_options.shape.closed`).
    pub(crate) closed: bool,
}

impl DrawingGeometryOptions {
    /// The resolver options of `drawing`'s stored style, `device_scale` caller px per CSS px (the
    /// vertical pixel ratio in the frame, 1 in media px). Every site that resolves a stored
    /// drawing builds its options here, so a new stored option reaches paint, hit testing,
    /// culling and text placement together.
    pub(crate) fn for_drawing(drawing: &Drawing, device_scale: f64) -> Self {
        Self {
            line_width: drawing.width,
            device_scale,
            icon_size: drawing.icon_size,
            extend_left: drawing.extend_left,
            extend_right: drawing.extend_right,
            full_circles: super::kinds::fibonacci::options(drawing).full_circles,
            closed: super::kinds::shapes::closed(drawing),
        }
    }
}

/// The far corner of a fixed Gann square from its pivot `start` (caller px): `ratio_corner`, the
/// corner a `tool_options.gann.scale_ratio` places (see `kinds::pitchforks_gann::ratio_point`),
/// or else a square on screen toward `end` whose side is the anchors' smaller one.
pub(crate) fn gann_fixed_end(
    start: (f64, f64),
    end: (f64, f64),
    ratio_corner: Option<(f64, f64)>,
) -> (f64, f64) {
    if let Some(corner) = ratio_corner {
        return corner;
    }
    let side = (end.0 - start.0).abs().min((end.1 - start.1).abs());
    (
        start.0 + side.copysign(end.0 - start.0),
        start.1 + side.copysign(end.1 - start.1),
    )
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GannGridGeometry {
    pub(crate) kind: DrawingKind,
    pub(crate) start: (f64, f64),
    pub(crate) end: (f64, f64),
}

impl GannGridGeometry {
    pub(crate) fn fan_segment(self, ratio: f64, reverse: bool) -> ((f64, f64), (f64, f64)) {
        let (start, end) = if reverse {
            (self.end, self.start)
        } else {
            (self.start, self.end)
        };
        let fraction = 1.0 / ratio.max(1.0);
        (
            start,
            (
                start.0 + (end.0 - start.0) * fraction,
                start.1 + (end.1 - start.1) * ratio * fraction,
            ),
        )
    }

    pub(crate) fn arc_point(self, radius: f64, progress: f64, reverse: bool) -> (f64, f64) {
        let (start, end) = if reverse {
            (self.end, self.start)
        } else {
            (self.start, self.end)
        };
        let angle = progress * std::f64::consts::FRAC_PI_2;
        (
            start.0 + (end.0 - start.0) * radius * angle.cos(),
            start.1 + (end.1 - start.1) * radius * angle.sin(),
        )
    }

    pub(crate) fn bounds(self) -> TextBox {
        TextBox {
            left: self.start.0.min(self.end.0),
            right: self.start.0.max(self.end.0),
            top: self.start.1.min(self.end.1),
            bottom: self.start.1.max(self.end.1),
        }
    }

    pub(crate) fn level_lines(self, value: f64) -> [((f64, f64), (f64, f64)); 2] {
        let x = self.start.0 + (self.end.0 - self.start.0) * value;
        let y = self.start.1 + (self.end.1 - self.start.1) * value;
        [
            ((x, self.start.1), (x, self.end.1)),
            ((self.start.0, y), (self.end.0, y)),
        ]
    }
}

impl PositionGeometry {
    fn from_points(entry: (f64, f64), target: (f64, f64), stop: (f64, f64)) -> Self {
        Self {
            left: entry.0.min(target.0),
            right: entry.0.max(target.0),
            entry_y: entry.1,
            target_y: target.1,
            stop_y: stop.1,
        }
    }

    pub(crate) fn reward_zone(self) -> PositionZone {
        PositionZone {
            left: self.left,
            right: self.right,
            y0: self.entry_y,
            y1: self.target_y,
        }
    }

    pub(crate) fn risk_zone(self) -> PositionZone {
        PositionZone {
            left: self.left,
            right: self.right,
            y0: self.entry_y,
            y1: self.stop_y,
        }
    }

    pub(crate) fn top(self) -> f64 {
        self.entry_y.min(self.target_y).min(self.stop_y)
    }

    pub(crate) fn bottom(self) -> f64 {
        self.entry_y.max(self.target_y).max(self.stop_y)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ResolvedDrawingGeometry<'a> {
    pub(crate) body: DrawingBodyGeometry<'a>,
    pub(crate) text_box: TextBox,
}

fn points_box(px: &[(f64, f64)]) -> Option<TextBox> {
    let &(first_x, first_y) = px.first()?;
    let (mut left, mut right, mut top, mut bottom) = (first_x, first_x, first_y, first_y);
    for &(x, y) in &px[1..] {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(TextBox {
        left,
        right,
        top,
        bottom,
    })
}

/// `line` extended along itself to the pane edge beyond its first point (`left`) and beyond its
/// second (`right`); a vertical line stays as it is.
fn extend_channel_line(
    line: [(f64, f64); 2],
    left: bool,
    right: bool,
    pane_w: f64,
) -> [(f64, f64); 2] {
    let [mut a, mut b] = line;
    let dx = b.0 - a.0;
    if dx.abs() <= f64::EPSILON {
        return line;
    }
    let slope = (b.1 - a.1) / dx;
    if left {
        let edge = if dx > 0.0 { 0.0 } else { pane_w };
        a.1 += (edge - a.0) * slope;
        a.0 = edge;
    }
    if right {
        let edge = if dx > 0.0 { pane_w } else { 0.0 };
        b.1 += (edge - b.0) * slope;
        b.0 = edge;
    }
    [a, b]
}

/// Which ends of a segment-body tool's anchor segment reach the pane edge: `(beyond the first
/// anchor, beyond the second)`, for anchors at `a` and `b`. The extended line always reaches both,
/// a ray always reaches past its second anchor and past its first by `extend_left`, and the other
/// segment tools follow `extend_left` and `extend_right`. A vertical segment extends to the pane's
/// top or bottom edge the same way, except the trend line and the forecast, which stay their anchor
/// segment. Coincident anchors have no direction: only upstream's ray (past its second anchor) and
/// extended line keep their vertical reach, and every other segment tool stays its empty segment.
/// Every reader of the resolved ends (the frame, hit testing, the line tools' end caps) takes them
/// from here.
pub(crate) fn segment_extension(
    kind: DrawingKind,
    options: DrawingGeometryOptions,
    a: Point,
    b: Point,
) -> (bool, bool) {
    let flags = (
        kind == DrawingKind::ExtendedLine || options.extend_left,
        matches!(kind, DrawingKind::Ray | DrawingKind::ExtendedLine) || options.extend_right,
    );
    if (b.0 - a.0).abs() > f64::EPSILON {
        return flags;
    }
    let coincident = (b.1 - a.1).abs() <= f64::EPSILON;
    match kind {
        DrawingKind::TrendLine | DrawingKind::Forecast => (false, false),
        DrawingKind::Ray | DrawingKind::ExtendedLine if coincident => {
            (kind == DrawingKind::ExtendedLine, true)
        }
        _ if coincident => (false, false),
        _ => flags,
    }
}

/// The corners of a rotated rectangle with anchors at `px` (its edge `a → b` and a depth point,
/// whose distance from the edge's line sets the depth), in order around it: `[a, b, b + o, a + o]`
/// with `o` the depth offset perpendicular to the edge. `None` for a zero-length edge or fewer
/// than three anchors.
pub(crate) fn rotated_rectangle_corners(px: &[Point]) -> Option<[Point; 4]> {
    let (&a, &b, &handle) = (px.first()?, px.get(1)?, px.get(2)?);
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let norm = dx * dx + dy * dy;
    if norm <= f64::EPSILON {
        return None;
    }
    let depth = ((handle.0 - a.0) * -dy + (handle.1 - a.1) * dx) / norm;
    let offset = (-dy * depth, dx * depth);
    Some([
        a,
        b,
        (b.0 + offset.0, b.1 + offset.1),
        (a.0 + offset.0, a.1 + offset.1),
    ])
}

pub(crate) fn resolve_drawing_geometry<'a>(
    kind: DrawingKind,
    px: &'a [(f64, f64)],
    pane_w: f64,
    pane_top: f64,
    pane_h: f64,
    options: DrawingGeometryOptions,
) -> Option<ResolvedDrawingGeometry<'a>> {
    if px.is_empty() {
        return None;
    }
    let body = match kind {
        DrawingKind::TrendLine
        | DrawingKind::Forecast
        | DrawingKind::Ray
        | DrawingKind::ExtendedLine
        | DrawingKind::InfoLine
        | DrawingKind::TrendAngle
        | DrawingKind::ArrowLine => {
            let mut a = *px.first()?;
            let mut b = *px.get(1)?;
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let (extend_a, extend_b) = segment_extension(kind, options, a, b);
            if dx.abs() > f64::EPSILON {
                let slope = dy / dx;
                if extend_a {
                    let edge = if dx > 0.0 { 0.0 } else { pane_w };
                    a.1 += (edge - a.0) * slope;
                    a.0 = edge;
                }
                if extend_b {
                    let edge = if dx > 0.0 { pane_w } else { 0.0 };
                    b.1 += (edge - b.0) * slope;
                    b.0 = edge;
                }
            } else {
                // A vertical segment extends to the pane's top or bottom edge on the ends
                // `segment_extension` selects.
                if extend_a {
                    a.1 = if dy > 0.0 {
                        pane_top
                    } else {
                        pane_top + pane_h
                    };
                }
                if extend_b {
                    b.1 = if dy > 0.0 {
                        pane_top + pane_h
                    } else {
                        pane_top
                    };
                }
            }
            DrawingBodyGeometry::Segment { a, b }
        }
        DrawingKind::HorizontalLine => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: 0.0,
            x1: pane_w,
        },
        DrawingKind::HorizontalRay => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: if options.extend_left { 0.0 } else { px[0].0 },
            x1: pane_w,
        },
        DrawingKind::VerticalLine => DrawingBodyGeometry::Vertical {
            x: px[0].0,
            y0: pane_top,
            y1: pane_top + pane_h,
        },
        DrawingKind::CrossLine => DrawingBodyGeometry::Cross {
            x: px[0].0,
            y: px[0].1,
            pane_w,
            pane_top,
            pane_bottom: pane_top + pane_h,
        },
        DrawingKind::ParallelChannel
        | DrawingKind::FlatTopChannel
        | DrawingKind::FlatBottomChannel
        | DrawingKind::DisjointChannel => {
            let first = [*px.first()?, *px.get(1)?];
            let second = match kind {
                DrawingKind::ParallelChannel => {
                    let control = *px.get(2)?;
                    let dx = first[1].0 - first[0].0;
                    let base_y = if dx.abs() > f64::EPSILON {
                        first[0].1 + (control.0 - first[0].0) * (first[1].1 - first[0].1) / dx
                    } else {
                        first[0].1
                    };
                    let offset = control.1 - base_y;
                    [
                        (first[0].0, first[0].1 + offset),
                        (first[1].0, first[1].1 + offset),
                    ]
                }
                DrawingKind::FlatTopChannel => {
                    let y = px.get(2)?.1.min(first[0].1).min(first[1].1);
                    [(first[0].0, y), (first[1].0, y)]
                }
                DrawingKind::FlatBottomChannel => {
                    let y = px.get(2)?.1.max(first[0].1).max(first[1].1);
                    [(first[0].0, y), (first[1].0, y)]
                }
                DrawingKind::DisjointChannel => [*px.get(2)?, *px.get(3)?],
                _ => unreachable!(),
            };
            // `extend_left`/`extend_right` run both lines (and so the fill) to the pane edges
            // beyond the first and second anchor (the own line's channel extensions).
            let extend =
                |line| extend_channel_line(line, options.extend_left, options.extend_right, pane_w);
            DrawingBodyGeometry::Channel {
                first: extend(first),
                second: extend(second),
            }
        }
        DrawingKind::RegressionTrend => {
            if px.len() < 8 {
                DrawingBodyGeometry::RegressionWindow {
                    a: *px.first()?,
                    b: *px.get(1)?,
                }
            } else {
                // `extend_left`/`extend_right` run the fitted lines (and so the zones) to the
                // pane edges beyond the first and second anchor's bar, like a channel's.
                let extend = |line| {
                    extend_channel_line(line, options.extend_left, options.extend_right, pane_w)
                };
                DrawingBodyGeometry::Regression {
                    center: extend([px[2], px[3]]),
                    upper: extend([px[4], px[5]]),
                    lower: extend([px[6], px[7]]),
                }
            }
        }
        DrawingKind::FibonacciRetracement
        | DrawingKind::FibonacciExtension
        | DrawingKind::FibonacciChannel
        | DrawingKind::FibonacciSpeedFan => {
            let start = *px.first()?;
            let end = *px.get(1)?;
            let pivot = if matches!(
                kind,
                DrawingKind::FibonacciRetracement | DrawingKind::FibonacciSpeedFan
            ) {
                None
            } else {
                Some(*px.get(2)?)
            };
            let (left, right) = if kind == DrawingKind::FibonacciExtension {
                let pivot = pivot?;
                let projected_x = pivot.0 + end.0 - start.0;
                (pivot.0.min(projected_x), pivot.0.max(projected_x))
            } else {
                (start.0.min(end.0), start.0.max(end.0))
            };
            DrawingBodyGeometry::Fibonacci(FibonacciGeometry {
                kind,
                start,
                end,
                pivot,
                x0: if options.extend_left { 0.0 } else { left },
                x1: if options.extend_right { pane_w } else { right },
            })
        }
        DrawingKind::FibonacciTimeZones | DrawingKind::FibonacciTrendTime => {
            let start = *px.first()?;
            let end = *px.get(1)?;
            let origin_x = if kind == DrawingKind::FibonacciTrendTime {
                px.get(2)?.0
            } else {
                start.0
            };
            DrawingBodyGeometry::TimeLevels(TimeLevelGeometry {
                origin_x,
                step_x: end.0 - start.0,
                pane_top,
                pane_bottom: pane_top + pane_h,
            })
        }
        DrawingKind::FibonacciSpeedArcs
        | DrawingKind::FibonacciCircles
        | DrawingKind::FibonacciSpiral
        | DrawingKind::FibonacciWedge => {
            let start = *px.first()?;
            let end = *px.get(1)?;
            let spiral = kind == DrawingKind::FibonacciSpiral;
            let wedge = kind == DrawingKind::FibonacciWedge;
            let center = if spiral || wedge { start } else { end };
            let outer = if spiral || wedge { end } else { start };
            let dx = outer.0 - center.0;
            let dy = outer.1 - center.1;
            let start_angle = dy.atan2(dx);
            let sweep = if wedge {
                let third = *px.get(2)?;
                let end_angle = (third.1 - center.1).atan2(third.0 - center.0);
                let mut delta = (end_angle - start_angle).rem_euclid(std::f64::consts::TAU);
                if delta > std::f64::consts::PI {
                    delta -= std::f64::consts::TAU;
                }
                delta
            } else if spiral {
                4.0 * std::f64::consts::PI
            } else if kind == DrawingKind::FibonacciCircles || options.full_circles {
                std::f64::consts::TAU
            } else {
                std::f64::consts::PI
            };
            DrawingBodyGeometry::FibonacciArcs(FibonacciArcGeometry {
                kind,
                center,
                radius: dx.hypot(dy),
                start_angle,
                sweep,
            })
        }
        DrawingKind::AndrewsPitchfork
        | DrawingKind::SchiffPitchfork
        | DrawingKind::ModifiedSchiffPitchfork
        | DrawingKind::InsidePitchfork
        | DrawingKind::Pitchfan => {
            let a = *px.first()?;
            let b = *px.get(1)?;
            let c = *px.get(2)?;
            let pivot = match kind {
                DrawingKind::SchiffPitchfork => (a.0, (a.1 + b.1) / 2.0),
                DrawingKind::ModifiedSchiffPitchfork => ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                DrawingKind::InsidePitchfork => ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                _ => a,
            };
            DrawingBodyGeometry::Pitchfork(PitchforkGeometry {
                kind,
                pivot,
                b,
                c,
                pane_w,
                pane_top,
                pane_bottom: pane_top + pane_h,
            })
        }
        DrawingKind::CyclicLines | DrawingKind::TimeCycles => {
            DrawingBodyGeometry::Cycles(CycleGeometry {
                kind,
                origin_x: px.first()?.0,
                step_x: px.get(1)?.0 - px.first()?.0,
                pane_w,
                pane_top,
                pane_bottom: pane_top + pane_h,
            })
        }
        DrawingKind::SineLine => {
            let start = *px.first()?;
            let peak = *px.get(1)?;
            if (peak.0 - start.0).abs() < 1e-6 {
                DrawingBodyGeometry::Empty
            } else {
                DrawingBodyGeometry::Sine(SineGeometry {
                    pivot: start,
                    quarter_width: peak.0 - start.0,
                    amplitude: peak.1 - start.1,
                    pane_w,
                })
            }
        }
        DrawingKind::ArrowMarkerUp
        | DrawingKind::ArrowMarkerDown
        | DrawingKind::ArrowMarkerLeft
        | DrawingKind::ArrowMarkerRight
        | DrawingKind::FlagMark
        | DrawingKind::Signpost => DrawingBodyGeometry::Marker(MarkerGeometry {
            kind,
            anchor: if kind == DrawingKind::Signpost {
                *px.get(1)?
            } else {
                *px.first()?
            },
            base: (kind == DrawingKind::Signpost).then(|| px[0]),
            radius: 7.0 * options.device_scale,
        }),
        DrawingKind::RotatedRectangle => {
            px.get(2)?;
            rotated_rectangle_corners(px).map_or(DrawingBodyGeometry::Empty, |corners| {
                DrawingBodyGeometry::Quad { corners }
            })
        }
        DrawingKind::Ellipse => {
            let a = *px.first()?;
            let b = *px.get(1)?;
            DrawingBodyGeometry::Ellipse {
                center: ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                rx: (a.0 - b.0).abs() / 2.0,
                ry: (a.1 - b.1).abs() / 2.0,
            }
        }
        DrawingKind::Circle => {
            let center = *px.first()?;
            let edge = *px.get(1)?;
            DrawingBodyGeometry::Circle {
                center,
                radius: (edge.0 - center.0).hypot(edge.1 - center.1),
            }
        }
        DrawingKind::Triangle => DrawingBodyGeometry::Triangle {
            corners: [*px.first()?, *px.get(1)?, *px.get(2)?],
        },
        DrawingKind::Arc => {
            let anchors = [*px.first()?, *px.get(1)?, *px.get(2)?];
            arc_through(anchors).map_or(
                DrawingBodyGeometry::Polyline {
                    points: px,
                    line_type: LineType::Simple,
                    terminal: None,
                },
                DrawingBodyGeometry::Arc,
            )
        }
        DrawingKind::Curve | DrawingKind::DoubleCurve => {
            let cubic = kind == DrawingKind::DoubleCurve;
            let mut curve = CurveGeometry {
                points: [
                    *px.first()?,
                    *px.get(1)?,
                    *px.get(2)?,
                    if cubic { *px.get(3)? } else { *px.get(2)? },
                ],
                cubic,
                extend: [None; 2],
            };
            // `extend_left`/`extend_right` continue the start's and the end's tangent to the
            // pane edge.
            let pane = Rect {
                left: 0.0,
                top: pane_top,
                right: pane_w,
                bottom: pane_top + pane_h,
            };
            let flags = [options.extend_left, options.extend_right];
            let (towards, ends) = (curve.end_towards(), curve.ends());
            for (((slot, flag), toward), end) in
                curve.extend.iter_mut().zip(flags).zip(towards).zip(ends)
            {
                if let (true, Some(toward)) = (flag, toward) {
                    let (_, edge) = shape::extend_segment(toward, end, pane, false, true);
                    *slot = (edge != end).then_some(edge);
                }
            }
            DrawingBodyGeometry::Curve(curve)
        }
        DrawingKind::Rectangle | DrawingKind::BarsPattern => {
            let (a, b) = (*px.first()?, *px.get(1)?);
            DrawingBodyGeometry::Rectangle {
                left: a.0.min(b.0),
                right: a.0.max(b.0),
                top: a.1.min(b.1),
                bottom: a.1.max(b.1),
            }
        }
        DrawingKind::Text
        | DrawingKind::Note
        | DrawingKind::Comment
        | DrawingKind::AnchoredText => DrawingBodyGeometry::Empty,
        DrawingKind::Callout => DrawingBodyGeometry::Segment {
            a: *px.first()?,
            b: *px.get(1)?,
        },
        DrawingKind::PriceNote => DrawingBodyGeometry::Horizontal {
            y: px[0].1,
            x0: 0.0,
            x1: pane_w,
        },
        DrawingKind::PriceLabel => DrawingBodyGeometry::PriceLabel {
            x: pane_w,
            y: px[0].1,
        },
        DrawingKind::GannBox | DrawingKind::GannSquare | DrawingKind::GannSquareFixed => {
            let start = px[0];
            let mut end = *px.get(1)?;
            if kind == DrawingKind::GannSquareFixed {
                // A scale ratio's corner is the third render point.
                end = gann_fixed_end(start, end, px.get(2).copied());
            }
            DrawingBodyGeometry::GannGrid(GannGridGeometry { kind, start, end })
        }
        DrawingKind::GannFan => {
            let start = px[0];
            // A scale ratio's 1×1 target is the third render point.
            let end = px.get(2).copied().unwrap_or(*px.get(1)?);
            DrawingBodyGeometry::Fibonacci(FibonacciGeometry {
                kind,
                start,
                end,
                pivot: None,
                x0: start.0,
                x1: pane_w,
            })
        }
        DrawingKind::Projection => {
            let pivot = px[0];
            let target = *px.get(1)?;
            DrawingBodyGeometry::Triangle {
                corners: [pivot, (target.0, pivot.1), target],
            }
        }
        DrawingKind::IconStamp => DrawingBodyGeometry::IconStamp {
            center: px[0],
            size: options.icon_size * options.device_scale,
        },
        DrawingKind::Brush | DrawingKind::Highlighter => DrawingBodyGeometry::Polyline {
            points: px,
            line_type: LineType::Curved,
            terminal: None,
        },
        DrawingKind::Path => DrawingBodyGeometry::Polyline {
            points: px,
            line_type: LineType::Simple,
            terminal: path_arrow_points(px, options.line_width, options.device_scale),
        },
        DrawingKind::Polyline if options.closed && px.len() >= 3 => {
            DrawingBodyGeometry::Polygon { points: px }
        }
        DrawingKind::Polyline
        | DrawingKind::PatternXabcd
        | DrawingKind::PatternCypher
        | DrawingKind::PatternAbcd
        | DrawingKind::PatternHeadShoulders
        | DrawingKind::PatternTriangle
        | DrawingKind::PatternThreeDrives
        | DrawingKind::ElliottImpulse
        | DrawingKind::ElliottCorrection
        | DrawingKind::ElliottTriangle
        | DrawingKind::ElliottDoubleCombination
        | DrawingKind::ElliottTripleCombination => DrawingBodyGeometry::Polyline {
            points: px,
            line_type: LineType::Simple,
            terminal: None,
        },
        DrawingKind::LongPosition | DrawingKind::ShortPosition => {
            let entry = *px.first()?;
            let target = *px.get(1)?;
            let stop = *px.get(2)?;
            DrawingBodyGeometry::Position(PositionGeometry::from_points(entry, target, stop))
        }
        DrawingKind::FixedRangeVolumeProfile => DrawingBodyGeometry::Segment {
            a: *px.first()?,
            b: *px.get(1)?,
        },
        DrawingKind::AnchoredVolumeProfile | DrawingKind::AnchoredVwap => {
            DrawingBodyGeometry::Vertical {
                x: px[0].0,
                y0: pane_top,
                y1: pane_top + pane_h,
            }
        }
        // The family kinds (own-line tools and the range tools) resolve their bodies through
        // `kinds::` parts; only their anchors' box reaches this resolver, as the reference box of
        // a box-layout text label.
        _ => {
            debug_assert!(kind.spec().family.is_some());
            return Some(ResolvedDrawingGeometry {
                body: DrawingBodyGeometry::Empty,
                text_box: points_box(px)?,
            });
        }
    };

    let text_box = if kind == DrawingKind::Callout {
        let (x, y) = *px.get(1)?;
        TextBox {
            left: x,
            right: x,
            top: y,
            bottom: y,
        }
    } else {
        match body {
            DrawingBodyGeometry::Empty => {
                let (x, y) = *px.first()?;
                TextBox {
                    left: x,
                    right: x,
                    top: y,
                    bottom: y,
                }
            }
            DrawingBodyGeometry::Segment { a, b } => TextBox {
                left: a.0.min(b.0),
                right: a.0.max(b.0),
                top: a.1.min(b.1),
                bottom: a.1.max(b.1),
            },
            DrawingBodyGeometry::Horizontal { y, x0, x1 } => TextBox {
                // Preserve the semantic direction of a ray. A right ray anchored beyond the pane may
                // intentionally have `left > right`; text placement historically uses that oriented
                // reference rather than normalizing it into a finite segment.
                left: x0,
                right: x1,
                top: y,
                bottom: y,
            },
            DrawingBodyGeometry::Vertical { x, y0, y1 } => TextBox {
                left: x,
                right: x,
                top: y0.min(y1),
                bottom: y0.max(y1),
            },
            DrawingBodyGeometry::Cross { x, y, .. } => TextBox {
                left: x,
                right: x,
                top: y,
                bottom: y,
            },
            DrawingBodyGeometry::Channel { first, second } => {
                points_box(&[first[0], first[1], second[0], second[1]])?
            }
            DrawingBodyGeometry::Regression {
                center,
                upper,
                lower,
            } => points_box(&[center[0], center[1], upper[0], upper[1], lower[0], lower[1]])?,
            DrawingBodyGeometry::RegressionWindow { a, b } => points_box(&[a, b])?,
            DrawingBodyGeometry::Fibonacci(fib) => TextBox {
                left: fib.x0,
                right: fib.x1,
                top: fib.start.1.min(fib.end.1),
                bottom: fib.start.1.max(fib.end.1),
            },
            DrawingBodyGeometry::TimeLevels(time) => TextBox {
                left: time.origin_x,
                right: time.origin_x + time.step_x,
                top: time.pane_top,
                bottom: time.pane_bottom,
            },
            DrawingBodyGeometry::FibonacciArcs(arcs) => TextBox {
                left: arcs.center.0 - arcs.radius,
                right: arcs.center.0 + arcs.radius,
                top: arcs.center.1 - arcs.radius,
                bottom: arcs.center.1 + arcs.radius,
            },
            DrawingBodyGeometry::Pitchfork(fork) => points_box(&[fork.pivot, fork.b, fork.c])?,
            DrawingBodyGeometry::Cycles(cycles) => TextBox {
                left: cycles.origin_x,
                right: cycles.origin_x + cycles.step_x,
                top: cycles.pane_top,
                bottom: cycles.pane_bottom,
            },
            DrawingBodyGeometry::Sine(sine) => TextBox {
                left: sine.pivot.0,
                right: sine.pivot.0 + sine.quarter_width,
                top: sine.pivot.1.min(sine.pivot.1 + sine.amplitude),
                bottom: sine.pivot.1.max(sine.pivot.1 + sine.amplitude),
            },
            DrawingBodyGeometry::Marker(marker) => {
                let triangle = marker.triangle();
                points_box(&[
                    triangle[0],
                    triangle[1],
                    triangle[2],
                    marker.base.unwrap_or(marker.anchor),
                ])?
            }
            DrawingBodyGeometry::PriceLabel { x, y } => TextBox {
                left: x,
                right: x,
                top: y,
                bottom: y,
            },
            DrawingBodyGeometry::IconStamp { center, size } => TextBox {
                left: center.0 - size / 2.0,
                right: center.0 + size / 2.0,
                top: center.1 - size / 2.0,
                bottom: center.1 + size / 2.0,
            },
            DrawingBodyGeometry::GannGrid(grid) => grid.bounds(),
            DrawingBodyGeometry::Quad { corners } => points_box(&corners)?,
            DrawingBodyGeometry::Ellipse { center, rx, ry } => TextBox {
                left: center.0 - rx,
                right: center.0 + rx,
                top: center.1 - ry,
                bottom: center.1 + ry,
            },
            DrawingBodyGeometry::Circle { center, radius } => TextBox {
                left: center.0 - radius,
                right: center.0 + radius,
                top: center.1 - radius,
                bottom: center.1 + radius,
            },
            DrawingBodyGeometry::Triangle { corners } => points_box(&corners)?,
            DrawingBodyGeometry::Arc(arc) => TextBox {
                left: arc.center.0 - arc.radius,
                right: arc.center.0 + arc.radius,
                top: arc.center.1 - arc.radius,
                bottom: arc.center.1 + arc.radius,
            },
            DrawingBodyGeometry::Curve(curve) => {
                points_box(&curve.points[..if curve.cubic { 4 } else { 3 }])?
            }
            DrawingBodyGeometry::Rectangle {
                left,
                right,
                top,
                bottom,
            } => TextBox {
                left,
                right,
                top,
                bottom,
            },
            DrawingBodyGeometry::Polyline { points, .. }
            | DrawingBodyGeometry::Polygon { points } => points_box(points)?,
            DrawingBodyGeometry::Position(position) => TextBox {
                left: position.left,
                right: position.right,
                top: position.top(),
                bottom: position.bottom(),
            },
        }
    };
    Some(ResolvedDrawingGeometry { body, text_box })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_bands_chain_visible_levels_in_list_order() {
        let mut drawing = Drawing::new(1, DrawingKind::FibonacciSpeedArcs, 0, Vec::new());
        drawing.fill_enabled = true;
        drawing.levels = [0.0, 0.5, 0.618, 1.0, 1.272, 1.618, -0.5, 2.0]
            .into_iter()
            .map(|value| DrawingLevel {
                visible: value != 0.618,
                fill_between: value != 1.272,
                ..DrawingLevel::at(value, "#123456")
            })
            .collect();
        let pairs = |drawing: &Drawing, positive_only| {
            level_band_pairs(drawing, &drawing.levels, positive_only)
                .map(|(prior, level)| (prior, level.value))
                .collect::<Vec<_>>()
        };
        // A hidden level breaks the chain; a level without `fill_between` still links it.
        assert_eq!(
            pairs(&drawing, false),
            [(0.0, 0.5), (1.272, 1.618), (1.618, -0.5), (-0.5, 2.0)]
        );
        // Radial levels also break on a non-positive effective value.
        assert_eq!(pairs(&drawing, true), [(1.272, 1.618)]);
        drawing.level_reverse = true;
        assert_eq!(pairs(&drawing, true), [(0.0, 0.5)]);
        drawing.fill_enabled = false;
        assert!(pairs(&drawing, false).is_empty());
    }

    #[test]
    fn gann_square_fans_and_arcs_share_bounded_geometry() {
        let grid = GannGridGeometry {
            kind: DrawingKind::GannSquareFixed,
            start: (10.0, 20.0),
            end: (110.0, 120.0),
        };
        assert_eq!(grid.fan_segment(1.0, false), ((10.0, 20.0), (110.0, 120.0)));
        assert_eq!(grid.fan_segment(2.0, false), ((10.0, 20.0), (60.0, 120.0)));
        assert_eq!(grid.fan_segment(2.0, true), ((110.0, 120.0), (60.0, 20.0)));
        assert_eq!(grid.arc_point(0.5, 0.0, false), (60.0, 20.0));
        assert!((grid.arc_point(0.5, 1.0, false).1 - 70.0).abs() < 1e-9);
    }

    #[test]
    fn projected_lines_resolve_in_both_directions() {
        let options = DrawingGeometryOptions::default();
        for (points, ray_end, extended_start) in [
            ([(30.0, 40.0), (50.0, 60.0)], (100.0, 110.0), (0.0, 10.0)),
            ([(70.0, 40.0), (50.0, 60.0)], (0.0, 110.0), (100.0, 10.0)),
        ] {
            let ray =
                resolve_drawing_geometry(DrawingKind::Ray, &points, 100.0, 0.0, 100.0, options)
                    .unwrap();
            let extended = resolve_drawing_geometry(
                DrawingKind::ExtendedLine,
                &points,
                100.0,
                0.0,
                100.0,
                options,
            )
            .unwrap();
            let DrawingBodyGeometry::Segment { a, b } = ray.body else {
                panic!("ray must be a segment");
            };
            assert_eq!(a, points[0]);
            assert_eq!(b, ray_end);
            let DrawingBodyGeometry::Segment { a, b } = extended.body else {
                panic!("extended line must be a segment");
            };
            assert_eq!(a, extended_start);
            assert_eq!(b, ray_end);
        }
    }

    #[test]
    fn cross_line_spans_the_pane_from_one_anchor() {
        let geometry = resolve_drawing_geometry(
            DrawingKind::CrossLine,
            &[(34.0, 56.0)],
            120.0,
            20.0,
            80.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        assert!(matches!(
            geometry.body,
            DrawingBodyGeometry::Cross {
                x: 34.0,
                y: 56.0,
                pane_w: 120.0,
                pane_top: 20.0,
                pane_bottom: 100.0,
            }
        ));
    }

    #[test]
    fn channel_boundaries_resolve_from_semantic_anchors() {
        let anchors = [(10.0, 20.0), (30.0, 40.0), (20.0, 55.0), (30.0, 65.0)];
        let cases = [
            (DrawingKind::ParallelChannel, [(10.0, 45.0), (30.0, 65.0)]),
            (DrawingKind::FlatTopChannel, [(10.0, 20.0), (30.0, 20.0)]),
            (DrawingKind::FlatBottomChannel, [(10.0, 55.0), (30.0, 55.0)]),
            (DrawingKind::DisjointChannel, [(20.0, 55.0), (30.0, 65.0)]),
        ];
        for (kind, expected) in cases {
            let geometry = resolve_drawing_geometry(
                kind,
                &anchors,
                100.0,
                0.0,
                100.0,
                DrawingGeometryOptions::default(),
            )
            .unwrap();
            let DrawingBodyGeometry::Channel { first, second } = geometry.body else {
                panic!("channel must resolve to shared channel geometry");
            };
            assert_eq!(first, [anchors[0], anchors[1]]);
            assert_eq!(second, expected);
        }
    }

    #[test]
    fn rotated_rectangle_arc_and_curves_keep_anchor_geometry() {
        let options = DrawingGeometryOptions::default();
        let rectangle = resolve_drawing_geometry(
            DrawingKind::RotatedRectangle,
            &[(10.0, 10.0), (30.0, 10.0), (20.0, 25.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        assert!(
            matches!(rectangle.body, DrawingBodyGeometry::Quad { corners } if corners == [(10.0, 10.0), (30.0, 10.0), (30.0, 25.0), (10.0, 25.0)])
        );

        let arc = resolve_drawing_geometry(
            DrawingKind::Arc,
            &[(10.0, 30.0), (20.0, 20.0), (30.0, 30.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Arc(arc) = arc.body else {
            panic!("noncollinear anchors should resolve to an arc");
        };
        assert!((arc.point(0.0).0 - 10.0).abs() < 1e-9);
        assert!((arc.point(1.0).0 - 30.0).abs() < 1e-9);
        assert!((arc.point(0.5).1 - 20.0).abs() < 1e-9);

        let quadratic = resolve_drawing_geometry(
            DrawingKind::Curve,
            &[(0.0, 0.0), (10.0, 20.0), (20.0, 0.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Curve(curve) = quadratic.body else {
            panic!("curve should resolve to shared sampled geometry");
        };
        assert_eq!(curve.point(0.0), (0.0, 0.0));
        assert_eq!(curve.point(0.5), (10.0, 10.0));
        assert_eq!(curve.point(1.0), (20.0, 0.0));
    }

    #[test]
    fn fibonacci_extension_and_channel_resolve_distinct_level_segments() {
        let options = DrawingGeometryOptions::default();
        let extension = resolve_drawing_geometry(
            DrawingKind::FibonacciExtension,
            &[(10.0, 20.0), (30.0, 40.0), (40.0, 50.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Fibonacci(extension) = extension.body else {
            panic!("extension must resolve to shared level geometry");
        };
        assert_eq!(extension.segment(0.0), ((40.0, 50.0), (60.0, 50.0)));
        assert_eq!(extension.segment(1.0), ((40.0, 70.0), (60.0, 70.0)));

        let channel = resolve_drawing_geometry(
            DrawingKind::FibonacciChannel,
            &[(10.0, 20.0), (30.0, 40.0), (20.0, 55.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Fibonacci(channel) = channel.body else {
            panic!("channel must resolve to shared level geometry");
        };
        assert_eq!(channel.segment(0.0), ((10.0, 20.0), (30.0, 40.0)));
        assert_eq!(channel.segment(1.0), ((10.0, 45.0), (30.0, 65.0)));
    }

    #[test]
    fn fibonacci_time_levels_use_the_interval_and_optional_projection_origin() {
        let options = DrawingGeometryOptions::default();
        for (kind, anchors, expected_x) in [
            (
                DrawingKind::FibonacciTimeZones,
                &[(10.0, 20.0), (30.0, 40.0)][..],
                110.0,
            ),
            (
                DrawingKind::FibonacciTrendTime,
                &[(10.0, 20.0), (30.0, 40.0), (40.0, 50.0)][..],
                140.0,
            ),
        ] {
            let geometry =
                resolve_drawing_geometry(kind, anchors, 200.0, 5.0, 95.0, options).unwrap();
            let DrawingBodyGeometry::TimeLevels(time) = geometry.body else {
                panic!("time tool must resolve to projected vertical levels");
            };
            assert_eq!(time.x(5.0), expected_x);
            assert_eq!((time.pane_top, time.pane_bottom), (5.0, 100.0));
        }
    }

    #[test]
    fn fibonacci_speed_fan_resolves_distinct_rays_from_one_origin() {
        let geometry = resolve_drawing_geometry(
            DrawingKind::FibonacciSpeedFan,
            &[(10.0, 20.0), (30.0, 60.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::Fibonacci(fan) = geometry.body else {
            panic!("speed fan must resolve to shared level geometry");
        };
        assert_eq!(fan.segment(0.0), ((10.0, 20.0), (30.0, 20.0)));
        assert_eq!(fan.segment(0.5), ((10.0, 20.0), (30.0, 40.0)));
        assert_eq!(fan.segment(1.0), ((10.0, 20.0), (30.0, 60.0)));
    }

    #[test]
    fn fibonacci_speed_arcs_use_second_anchor_as_center() {
        let geometry = resolve_drawing_geometry(
            DrawingKind::FibonacciSpeedArcs,
            &[(10.0, 50.0), (50.0, 50.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::FibonacciArcs(arcs) = geometry.body else {
            panic!("speed arcs must resolve to shared radial geometry");
        };
        assert_eq!(arcs.center, (50.0, 50.0));
        assert!((arcs.point(0.5, 0.5).0 - 30.0).abs() < 1e-9);
        assert!((arcs.point(0.5, 0.5).1 - 50.0).abs() < 1e-9);
        let circles = resolve_drawing_geometry(
            DrawingKind::FibonacciCircles,
            &[(10.0, 50.0), (50.0, 50.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::FibonacciArcs(circles) = circles.body else {
            panic!("Fibonacci circles must resolve to radial geometry");
        };
        assert!((circles.point(0.5, 0.0).0 - circles.point(0.5, 1.0).0).abs() < 1e-9);
        assert!((circles.point(0.5, 0.0).1 - circles.point(0.5, 1.0).1).abs() < 1e-9);
    }

    #[test]
    fn fibonacci_spiral_and_wedge_have_distinct_anchor_geometry() {
        let spiral = resolve_drawing_geometry(
            DrawingKind::FibonacciSpiral,
            &[(10.0, 20.0), (50.0, 20.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::FibonacciArcs(spiral) = spiral.body else {
            panic!("spiral must resolve to radial geometry");
        };
        assert_eq!(spiral.point(1.0, 0.0), (10.0, 20.0));
        assert!((spiral.point(1.0, 1.0).0 - 50.0).abs() < 1e-9);
        assert!(spiral.point(1.0, 0.5).0 > 10.0);

        let wedge = resolve_drawing_geometry(
            DrawingKind::FibonacciWedge,
            &[(10.0, 20.0), (30.0, 40.0), (40.0, 60.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::FibonacciArcs(wedge) = wedge.body else {
            panic!("wedge must resolve to concentric arcs");
        };
        let first = wedge.point(1.0, 0.0);
        let middle = wedge.point(1.0, 0.5);
        let last = wedge.point(1.0, 1.0);
        assert!((first.0 - 30.0).abs() < 1e-9 && (first.1 - 40.0).abs() < 1e-9);
        assert!(((middle.0 - 10.0).hypot(middle.1 - 20.0) - wedge.radius).abs() < 1e-9);
        assert!(((last.0 - 10.0).hypot(last.1 - 20.0) - wedge.radius).abs() < 1e-9);
        assert!(middle.0 < first.0 && middle.0 > last.0);
    }

    #[test]
    fn pitchfork_origins_and_fan_rays_follow_their_anchor_rules() {
        let anchors = [(10.0, 20.0), (30.0, 40.0), (30.0, 60.0)];
        for (kind, expected_origin) in [
            (DrawingKind::AndrewsPitchfork, (10.0, 20.0)),
            (DrawingKind::SchiffPitchfork, (10.0, 30.0)),
            (DrawingKind::ModifiedSchiffPitchfork, (20.0, 30.0)),
            (DrawingKind::InsidePitchfork, (20.0, 30.0)),
            (DrawingKind::Pitchfan, (10.0, 20.0)),
        ] {
            let geometry = resolve_drawing_geometry(
                kind,
                &anchors,
                100.0,
                0.0,
                100.0,
                DrawingGeometryOptions::default(),
            )
            .unwrap();
            let DrawingBodyGeometry::Pitchfork(fork) = geometry.body else {
                panic!("pitchfork must resolve to shared geometry");
            };
            assert_eq!(fork.pivot, expected_origin);
            assert_eq!(fork.segment(0.5).0, expected_origin);
            if kind == DrawingKind::Pitchfan {
                assert_eq!(fork.segment(0.0), ((10.0, 20.0), (100.0, 110.0)));
                assert_eq!(fork.segment(1.0), ((10.0, 20.0), (100.0, 200.0)));
            } else {
                assert_eq!(fork.segment(0.0).0, anchors[1]);
                if kind == DrawingKind::InsidePitchfork {
                    assert_eq!(fork.anchor(0.5), anchors[2]);
                    assert_eq!(fork.segment(1.0).0, (30.0, 80.0));
                } else {
                    assert_eq!(fork.segment(1.0).0, anchors[2]);
                }
            }
        }
    }

    #[test]
    fn cycles_repeat_with_bounded_visible_work_and_sine_reaches_first_peak() {
        let options = DrawingGeometryOptions::default();
        let cycles = resolve_drawing_geometry(
            DrawingKind::CyclicLines,
            &[(20.0, 10.0), (40.0, 30.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Cycles(cycles) = cycles.body else {
            panic!("cyclic lines need repeated vertical geometry");
        };
        let mut xs = Vec::new();
        cycles.for_each_visible_line(|_, x| xs.push(x));
        assert_eq!(xs, vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        let mut dense_count = 0;
        CycleGeometry {
            step_x: 0.01,
            ..cycles
        }
        .for_each_visible_line(|_, _| dense_count += 1);
        assert!(dense_count <= 256);

        let time = resolve_drawing_geometry(
            DrawingKind::TimeCycles,
            &[(20.0, 10.0), (40.0, 30.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Cycles(time) = time.body else {
            panic!("time cycles need repeated vertical geometry");
        };
        let mut xs = Vec::new();
        time.for_each_visible_line(|_, x| xs.push(x));
        assert_eq!(xs, vec![20.0, 40.0, 60.0, 80.0, 100.0]);

        let sine = resolve_drawing_geometry(
            DrawingKind::SineLine,
            &[(20.0, 50.0), (40.0, 70.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Sine(sine) = sine.body else {
            panic!("sine line needs sampled wave geometry");
        };
        assert!((sine.y(20.0) - 50.0).abs() < 1e-9);
        assert!((sine.y(40.0) - 70.0).abs() < 1e-9);
        assert!((sine.y(60.0) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn marker_arrows_keep_the_anchor_at_the_tip_and_signpost_keeps_its_stem() {
        let options = DrawingGeometryOptions::default();
        for kind in [
            DrawingKind::ArrowMarkerUp,
            DrawingKind::ArrowMarkerDown,
            DrawingKind::ArrowMarkerLeft,
            DrawingKind::ArrowMarkerRight,
        ] {
            let geometry =
                resolve_drawing_geometry(kind, &[(40.0, 50.0)], 100.0, 0.0, 100.0, options)
                    .unwrap();
            let DrawingBodyGeometry::Marker(marker) = geometry.body else {
                panic!("arrow marker needs shared marker geometry");
            };
            assert_eq!(marker.triangle()[0], (40.0, 50.0));
            assert!(marker.stem().is_none());
        }
        let signpost = resolve_drawing_geometry(
            DrawingKind::Signpost,
            &[(20.0, 80.0), (40.0, 50.0)],
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Marker(signpost) = signpost.body else {
            panic!("signpost needs marker geometry");
        };
        assert_eq!(signpost.stem(), Some(((20.0, 80.0), (40.0, 50.0))));
    }

    #[test]
    fn flat_channels_keep_the_named_horizontal_boundary() {
        let points = [(10.0, 30.0), (90.0, 70.0), (50.0, 50.0)];
        let options = DrawingGeometryOptions::default();
        let top = resolve_drawing_geometry(
            DrawingKind::FlatTopChannel,
            &points,
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let bottom = resolve_drawing_geometry(
            DrawingKind::FlatBottomChannel,
            &points,
            100.0,
            0.0,
            100.0,
            options,
        )
        .unwrap();
        let DrawingBodyGeometry::Channel {
            second: top_line, ..
        } = top.body
        else {
            panic!("top channel")
        };
        let DrawingBodyGeometry::Channel {
            second: bottom_line,
            ..
        } = bottom.body
        else {
            panic!("bottom channel")
        };
        assert_eq!(top_line[0].1, 30.0);
        assert_eq!(bottom_line[0].1, 70.0);
    }

    #[test]
    fn projection_sector_uses_the_horizon_and_target_price() {
        let geometry = resolve_drawing_geometry(
            DrawingKind::Projection,
            &[(10.0, 50.0), (80.0, 10.0)],
            100.0,
            0.0,
            100.0,
            DrawingGeometryOptions::default(),
        )
        .unwrap();
        let DrawingBodyGeometry::Triangle { corners } = geometry.body else {
            panic!("projection triangle")
        };
        assert_eq!(corners, [(10.0, 50.0), (80.0, 50.0), (80.0, 10.0)]);
    }
}
