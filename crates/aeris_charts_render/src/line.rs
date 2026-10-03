//! Line / area / baseline geometry builder. Port of `walk-line.ts` + `line-renderer.ts` +
//! `area-renderer-base.ts`.
//!
//! Unlike the integer-rect series, lines are anti-aliased: points stay as floats in bitmap
//! space and are emitted as CPU-tessellated triangles. The stroke is a series of quads (one
//! per segment) plus round-join fans at interior vertices; the area fill is a triangle strip
//! between the polyline and a base level. The backend feathers edges for AA.
//!
//! Simple, stepped, and curved line types share the same bounded expansion math across executors.

use crate::color::Color;
use crate::draw_list::{LineStyle, LineType, Prim};
use crate::shape::{clip_polyline_to_rect, Rect};

/// A vertex the backend will render: bitmap-space position + straight RGBA color.
/// The stroke pipeline extrudes these with AA; the fill pipeline draws them opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineVertex {
    pub x: f32,
    pub y: f32,
    pub color: [f32; 4],
}

/// One data point in media coordinates (already converted by the views layer).
#[derive(Clone, Copy, Debug)]
pub struct LinePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct LineParams {
    pub horizontal_pixel_ratio: f64,
    pub vertical_pixel_ratio: f64,
    pub line_width: f64,
    pub line_type: LineType,
}

fn color_to_rgba(c: Color) -> [f32; 4] {
    [
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    ]
}

/// Upper bound on dash-walk steps for one polyline (each step closes one pattern element or
/// reaches a vertex).
pub const MAX_DASH_STEPS: usize = 1 << 16;

/// Split a polyline into the solid sub-segments a dash pattern produces (port of the Canvas2D
/// `setLineDash` walk: the pattern starts "on" at the first point and alternates on/off along
/// the path, in the same units as `points`). Each returned run is a maximal "on" sub-polyline of
/// two or more points; gap crossings close the current run. Frame builders and both GPU
/// executors stroke each run solid, so dash geometry matches the Canvas2D path by construction.
/// `points` are expected already expanded ([`expand_line`]) so dashes follow the rendered path
/// for stepped/curved lines.
///
/// Work is bounded by [`MAX_DASH_STEPS`]: a pattern too fine for the path length (including
/// elements too small to advance in `f64`) strokes the whole path solid, which is what a
/// sub-pixel dash converges to visually. A path with a non-finite length strokes nothing.
pub fn dash_split(points: &[LinePoint], pattern: &[f64]) -> Vec<Vec<LinePoint>> {
    let mut runs: Vec<Vec<LinePoint>> = Vec::new();
    if points.len() < 2
        || pattern.is_empty()
        || pattern.iter().any(|&len| !len.is_finite() || len <= 0.0)
    {
        return runs;
    }
    let total_len: f64 = points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y))
        .sum();
    if !total_len.is_finite() {
        return runs;
    }
    let period: f64 = pattern.iter().sum();
    let solid = || vec![points.to_vec()];
    if total_len / period * pattern.len() as f64 > MAX_DASH_STEPS as f64 {
        return solid();
    }
    let mut steps = 0usize;
    let interp = |a: LinePoint, b: LinePoint, t: f64| LinePoint {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    };
    let near = |a: LinePoint, b: LinePoint| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    let mut element = 0usize;
    let mut element_left = pattern[0];
    let mut on = true;
    let mut run: Vec<LinePoint> = vec![points[0]];
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let seg_len = (b.x - a.x).hypot(b.y - a.y);
        if seg_len < 1e-9 {
            continue;
        }
        let mut t0 = 0.0f64;
        while t0 < seg_len - 1e-9 {
            steps += 1;
            if steps > MAX_DASH_STEPS + points.len() + pattern.len() {
                return solid();
            }
            let step = element_left.min(seg_len - t0);
            let t1 = t0 + step;
            if on {
                let p0 = interp(a, b, t0 / seg_len);
                let p1 = interp(a, b, t1 / seg_len);
                if run.last().is_none_or(|&last| !near(last, p0)) {
                    // A gap ended since the last "on" point: close the run and start a new one.
                    if run.len() >= 2 {
                        runs.push(std::mem::take(&mut run));
                    } else {
                        run.clear();
                    }
                    run.push(p0);
                }
                run.push(p1);
            }
            t0 = t1;
            element_left -= step;
            if element_left <= 1e-9 {
                on = !on;
                element = (element + 1) % pattern.len();
                element_left = pattern[element];
            }
        }
    }
    if run.len() >= 2 {
        runs.push(run);
    }
    runs
}

/// Emit a polyline stroke. A solid style emits a single `Polyline` prim (the backends expand
/// `line_type` themselves, as before). Any dashed style is expanded with `line_type` and split
/// into solid dash sub-segments here in the frame producer — reference `setLineDash` semantics on
/// the device-px path (draw-line.ts `getDashPattern`) — so the gap geometry is generated once for
/// the whole frame, over the stroke's visible reach rather than the full path, and every backend
/// paints the same runs.
#[allow(clippy::too_many_arguments)]
pub fn push_line_stroke(
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
    window: &[[f32; 2]],
    width: f32,
    style: LineStyle,
    line_type: LineType,
    color: Color,
) {
    let pattern = style.dash_pattern(width);
    if pattern.is_empty() {
        let first = points.len() as u32;
        points.extend_from_slice(window);
        out.push(Prim::Polyline {
            first_point: first,
            point_count: window.len() as u32,
            width,
            style: LineStyle::Solid,
            line_type,
            color,
        });
        return;
    }
    for run in dash_runs(window, &pattern, line_type) {
        let first = points.len() as u32;
        points.extend(run.iter().map(|p| [p.x as f32, p.y as f32]));
        out.push(Prim::Polyline {
            first_point: first,
            point_count: run.len() as u32,
            width,
            style: LineStyle::Solid,
            line_type: LineType::Simple,
            color,
        });
    }
}

/// The solid "on" runs of `window` under a dash `pattern` (device px): the window is expanded with
/// `line_type` first, so dashes follow the rendered path, then split with the pattern starting
/// "on" at the first point.
pub fn dash_runs(window: &[[f32; 2]], pattern: &[f32], line_type: LineType) -> Vec<Vec<LinePoint>> {
    let device: Vec<LinePoint> = window
        .iter()
        .map(|p| LinePoint {
            x: p[0] as f64,
            y: p[1] as f64,
        })
        .collect();
    let expanded = expand_line(&device, line_type);
    let pattern: Vec<f64> = pattern.iter().map(|&len| len as f64).collect();
    dash_split(&expanded, &pattern)
}

/// The rect a stroke of `width` is clipped to (`pane` grown by the stroke's reach) and the dash
/// pattern's length in px (0 for a solid style), shared by [`push_clipped_stroke`] and
/// [`dash_run_bound`] so the bound and the lowering can never disagree about either.
fn stroke_clip(pane: Rect, width: f32, style: LineStyle) -> (Rect, f64) {
    let period = style
        .dash_pattern(width)
        .iter()
        .copied()
        .map(f64::from)
        .sum();
    (pane.inflate(f64::from(width) + 2.0), period)
}

/// Lower one stroke run that may reach far past `pane` (family and placement-guide strokes, and
/// the dashed strokes of [`push_styled_stroke`]) for every executor. The run is clipped to `pane`
/// grown by the stroke's reach, so frame work and coordinates past the pane stay bounded however
/// far the geometry reaches (an extreme level or zoom), and a dashed or dotted run is split into
/// solid dash runs through [`push_line_stroke`]; clipped parts keep the unclipped run's dash
/// phase, so dashes never shift while panning. The dash count inside the pane still grows with
/// the run's visible path length ([`dash_run_bound`]): a caller fed by untrusted geometry checks
/// it first.
pub fn push_clipped_stroke(
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
    run: &[(f64, f64)],
    pane: Rect,
    (width, style, color): (f32, LineStyle, Color),
    scratch: &mut Vec<(f64, f64)>,
) {
    let (clip, period) = stroke_clip(pane, width, style);
    clip_polyline_to_rect(run, clip, period, scratch, |part| {
        if style == LineStyle::Solid {
            let first_point = points.len() as u32;
            points.extend(part.iter().map(|&(x, y)| [x as f32, y as f32]));
            out.push(Prim::Polyline {
                first_point,
                point_count: part.len() as u32,
                width,
                style,
                line_type: LineType::Simple,
                color,
            });
        } else {
            let path: Vec<[f32; 2]> = part.iter().map(|&(x, y)| [x as f32, y as f32]).collect();
            push_line_stroke(out, points, &path, width, style, LineType::Simple, color);
        }
    });
}

/// Lower a styled stroke through `run` (bitmap px) whose geometry is not bounded by the viewport
/// (core drawings, general series). A solid run stays one polyline in `line_type`, which every
/// executor strokes alike. A dashed or dotted run is expanded with `line_type` first (a curve
/// clipped before expansion would bend differently inside the pane) and then lowered through
/// [`push_clipped_stroke`], so executors receive only solid dash runs, whatever their dash
/// support, and the work past the pane stays bounded; inside it the dash count follows the run's
/// visible path length ([`dash_run_bound`]).
pub fn push_styled_stroke(
    out: &mut Vec<Prim>,
    points: &mut Vec<[f32; 2]>,
    run: &[(f64, f64)],
    line_type: LineType,
    (width, style, color): (f32, LineStyle, Color),
    pane: Rect,
) {
    if style == LineStyle::Solid {
        let first_point = points.len() as u32;
        points.extend(run.iter().map(|&(x, y)| [x as f32, y as f32]));
        out.push(Prim::Polyline {
            first_point,
            point_count: run.len() as u32,
            width,
            style,
            line_type,
            color,
        });
        return;
    }
    let expanded;
    let run = if line_type == LineType::Simple {
        run
    } else {
        let line: Vec<LinePoint> = run.iter().map(|&(x, y)| LinePoint { x, y }).collect();
        expanded = expand_line(&line, line_type)
            .into_iter()
            .map(|point| (point.x, point.y))
            .collect::<Vec<_>>();
        &expanded
    };
    push_clipped_stroke(
        out,
        points,
        run,
        pane,
        (width, style, color),
        &mut Vec::new(),
    );
}

/// An upper bound on the solid dash runs [`push_styled_stroke`] emits for a dashed or dotted
/// `run` of [`LineType::Simple`] (expand other line types first, as it does), computed without
/// lowering it: the run is clipped exactly as the lowering clips it and each clipped part of arc
/// length `L` can hold at most `L / period + 1` on-stretches of the dash pattern. The count grows
/// with the path's visible length, not with the pane, so a caller fed by untrusted geometry (the
/// browser's plugin command buffers) checks it before lowering. A solid style, which is never
/// split, reports 0.
pub fn dash_run_bound(run: &[(f64, f64)], pane: Rect, width: f32, style: LineStyle) -> f64 {
    let (clip, period) = stroke_clip(pane, width, style);
    if period <= 0.0 {
        return 0.0;
    }
    let mut bound = 0.0;
    clip_polyline_to_rect(run, clip, period, &mut Vec::new(), |part| {
        let length: f64 = part
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            .sum();
        bound += length / period + 1.0;
    });
    bound
}

/// A crisp line's `[from, to]` span (either order) clamped to `pane` in whole pixels, `None` when
/// it misses the pane. Executors dash a crisp line from its start, so a start clamped into the
/// pane moves back to a whole dash period from the unclamped start and keeps the pattern's phase;
/// the executors' dash loops stay bounded by the pane.
pub fn crisp_span(
    from: f64,
    to: f64,
    (low, high): (f64, f64),
    width: i32,
    style: LineStyle,
) -> Option<(i32, i32)> {
    let (start, end) = (from.min(to).round(), from.max(to).round());
    if !(start <= high && end >= low) {
        return None;
    }
    let period: f64 = style
        .dash_pattern(width as f32)
        .iter()
        .copied()
        .map(f64::from)
        .sum();
    let clamped = if start < low && period > 0.0 {
        low - (low - start).rem_euclid(period)
    } else {
        start.max(low)
    };
    Some((clamped.round() as i32, end.min(high) as i32))
}

/// A tessellated stroke: triangle list of extruded segment quads + round joins.
/// The backend applies 1px edge feathering for AA.
#[derive(Default)]
pub struct StrokeMesh {
    pub vertices: Vec<LineVertex>,
}

impl StrokeMesh {
    fn push_tri(&mut self, a: LineVertex, b: LineVertex, c: LineVertex) {
        self.vertices.push(a);
        self.vertices.push(b);
        self.vertices.push(c);
    }

    fn push_quad(
        &mut self,
        p0: [f32; 2],
        p1: [f32; 2],
        p2: [f32; 2],
        p3: [f32; 2],
        color: [f32; 4],
    ) {
        let v = |p: [f32; 2]| LineVertex {
            x: p[0],
            y: p[1],
            color,
        };
        // p0-p1-p2, p0-p2-p3 (winding-agnostic; no culling in the pipeline)
        self.push_tri(v(p0), v(p1), v(p2));
        self.push_tri(v(p0), v(p2), v(p3));
    }

    fn push_round_join(&mut self, center: [f32; 2], radius: f32, color: [f32; 4]) {
        let segments = join_segments(radius);
        let v = |p: [f32; 2]| LineVertex {
            x: p[0],
            y: p[1],
            color,
        };
        let c = v(center);
        for i in 0..segments {
            let a0 = (i as f32) / segments as f32 * std::f32::consts::TAU;
            let a1 = ((i + 1) as f32) / segments as f32 * std::f32::consts::TAU;
            let p0 = [center[0] + radius * a0.cos(), center[1] + radius * a0.sin()];
            let p1 = [center[0] + radius * a1.cos(), center[1] + radius * a1.sin()];
            self.push_tri(c, v(p0), v(p1));
        }
    }
}

/// Fan segments for a round join of `radius`: enough to keep the chord error under a quarter
/// device pixel (`r * (1 - cos(pi / n)) < 0.25`), bounded so thin series lines stay cheap and
/// heavy brush strokes stay visibly round instead of octagonal.
pub fn join_segments(radius: f32) -> usize {
    ((std::f32::consts::PI * (radius * 2.0).sqrt()).ceil() as usize).clamp(8, 32)
}

/// Number of straight segments used to tessellate a curved interval.
const CURVE_SEGMENTS: usize = 16;

/// Target device-px length of one curved segment. Intervals already shorter than this render as
/// their chord: at a few device pixels the curve and its chord cover the same pixels, and
/// densifying them would multiply tessellation work (a freehand brush samples every ~1.5 px)
/// without changing the output.
const CURVE_SEGMENT_PX: f64 = 4.0;

/// Segments for one curved interval of device-px length `len_px`, capped at [`CURVE_SEGMENTS`]
/// and never denser than one segment (the chord).
fn curve_segments_for(len_px: f64) -> usize {
    (len_px / CURVE_SEGMENT_PX)
        .ceil()
        .clamp(1.0, CURVE_SEGMENTS as f64) as usize
}

/// Catmull-Rom interpolation of one scalar channel at parameter `t` (0..1).
fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Expand a polyline according to its [`LineType`] into `out` (cleared first, allocation reused):
/// `Simple` is unchanged; `WithSteps` inserts a horizontal-then-vertical corner at each interval
/// (the value holds until the next point); `Curved`
/// tessellates a Catmull-Rom spline through the points with a per-interval segment count adapted
/// to the interval's device-px length (`hpr`/`vpr` convert media to device px).
pub fn expand_line_into(
    points: &[LinePoint],
    line_type: LineType,
    hpr: f64,
    vpr: f64,
    out: &mut Vec<LinePoint>,
) {
    out.clear();
    match line_type {
        LineType::Simple => out.extend_from_slice(points),
        LineType::WithSteps => {
            out.reserve(points.len() * 2);
            for (i, p) in points.iter().enumerate() {
                if i > 0 {
                    // step corner: horizontal to this x at the previous y, then drop to this point
                    out.push(LinePoint {
                        x: p.x,
                        y: points[i - 1].y,
                    });
                }
                out.push(*p);
            }
        }
        LineType::Curved => {
            if points.len() < 3 {
                out.extend_from_slice(points);
                return;
            }
            let n = points.len();
            out.push(points[0]);
            for i in 0..n - 1 {
                let p0 = points[i.saturating_sub(1)];
                let p1 = points[i];
                let p2 = points[i + 1];
                let p3 = points[(i + 2).min(n - 1)];
                let len_px = ((p2.x - p1.x) * hpr).hypot((p2.y - p1.y) * vpr);
                let segments = curve_segments_for(len_px);
                for s in 1..=segments {
                    let t = s as f64 / segments as f64;
                    out.push(LinePoint {
                        x: catmull_rom(p0.x, p1.x, p2.x, p3.x, t),
                        y: catmull_rom(p0.y, p1.y, p2.y, p3.y, t),
                    });
                }
            }
        }
    }
}

/// Expand a polyline according to its [`LineType`]: `Simple` is unchanged; `WithSteps` inserts a
/// horizontal-then-vertical corner at each interval (the value holds until the next point);
/// `Curved` tessellates a Catmull-Rom spline through the points.
///
/// Allocating convenience wrapper over [`expand_line_into`] for callers whose points are already
/// in device px.
pub fn expand_line(points: &[LinePoint], line_type: LineType) -> Vec<LinePoint> {
    let mut out = Vec::new();
    expand_line_into(points, line_type, 1.0, 1.0, &mut out);
    out
}

/// Expand two aligned band boundaries with identical sample positions so the resulting points
/// remain pairwise suitable for a triangle strip. Curved sampling uses the longer of the two
/// boundary intervals to choose one shared subdivision count.
pub fn expand_band_into(
    upper: &[LinePoint],
    lower: &[LinePoint],
    line_type: LineType,
    hpr: f64,
    vpr: f64,
    out_upper: &mut Vec<LinePoint>,
    out_lower: &mut Vec<LinePoint>,
) {
    out_upper.clear();
    out_lower.clear();
    let count = upper.len().min(lower.len());
    let upper = &upper[..count];
    let lower = &lower[..count];
    match line_type {
        LineType::Simple => {
            out_upper.extend_from_slice(upper);
            out_lower.extend_from_slice(lower);
        }
        LineType::WithSteps => {
            out_upper.reserve(count.saturating_mul(2));
            out_lower.reserve(count.saturating_mul(2));
            for index in 0..count {
                if index > 0 {
                    out_upper.push(LinePoint {
                        x: upper[index].x,
                        y: upper[index - 1].y,
                    });
                    out_lower.push(LinePoint {
                        x: lower[index].x,
                        y: lower[index - 1].y,
                    });
                }
                out_upper.push(upper[index]);
                out_lower.push(lower[index]);
            }
        }
        LineType::Curved => {
            if count < 3 {
                out_upper.extend_from_slice(upper);
                out_lower.extend_from_slice(lower);
                return;
            }
            out_upper.push(upper[0]);
            out_lower.push(lower[0]);
            for index in 0..count - 1 {
                let upper_len = ((upper[index + 1].x - upper[index].x) * hpr)
                    .hypot((upper[index + 1].y - upper[index].y) * vpr);
                let lower_len = ((lower[index + 1].x - lower[index].x) * hpr)
                    .hypot((lower[index + 1].y - lower[index].y) * vpr);
                let segments = curve_segments_for(upper_len.max(lower_len));
                for sample in 1..=segments {
                    let t = sample as f64 / segments as f64;
                    let interpolate = |points: &[LinePoint]| LinePoint {
                        x: catmull_rom(
                            points[index.saturating_sub(1)].x,
                            points[index].x,
                            points[index + 1].x,
                            points[(index + 2).min(count - 1)].x,
                            t,
                        ),
                        y: catmull_rom(
                            points[index.saturating_sub(1)].y,
                            points[index].y,
                            points[index + 1].y,
                            points[(index + 2).min(count - 1)].y,
                            t,
                        ),
                    };
                    out_upper.push(interpolate(upper));
                    out_lower.push(interpolate(lower));
                }
            }
        }
    }
}

/// Allocating convenience wrapper over [`expand_band_into`] for device-space points.
pub fn expand_band(
    upper: &[LinePoint],
    lower: &[LinePoint],
    line_type: LineType,
) -> (Vec<LinePoint>, Vec<LinePoint>) {
    let mut out_upper = Vec::new();
    let mut out_lower = Vec::new();
    expand_band_into(
        upper,
        lower,
        line_type,
        1.0,
        1.0,
        &mut out_upper,
        &mut out_lower,
    );
    (out_upper, out_lower)
}

/// Builds a stroke mesh over `points` (single color). `visible_range` is `[from, to)` row
/// offsets. Returns triangles in bitmap space.
pub fn build_line_stroke(
    points: &[LinePoint],
    color: Color,
    params: &LineParams,
    out: &mut StrokeMesh,
) {
    let mut expanded = Vec::new();
    expand_line_into(
        points,
        params.line_type,
        params.horizontal_pixel_ratio,
        params.vertical_pixel_ratio,
        &mut expanded,
    );
    let points = &expanded[..];
    if points.len() < 2 {
        // single point: reference draws a short horizontal segment of barWidth; skip until we
        // carry barWidth here (area/line with 1 visible point is a rare edge).
        return;
    }

    let hpr = params.horizontal_pixel_ratio;
    let vpr = params.vertical_pixel_ratio;
    let half = (params.line_width * vpr / 2.0) as f32;
    let rgba = color_to_rgba(color);

    let bp = |p: &LinePoint| [(p.x * hpr) as f32, (p.y * vpr) as f32];

    let mut prev_dir: Option<[f32; 2]> = None;
    for i in 0..points.len() - 1 {
        let a = bp(&points[i]);
        let b = bp(&points[i + 1]);

        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            continue;
        }
        let dir = [dx / len, dy / len];
        // normal
        let nx = -dir[1] * half;
        let ny = dir[0] * half;

        self_push_segment(out, a, b, nx, ny, rgba);

        // Round join at the shared interior vertex, but only where the turn opens a visible
        // wedge: nearly-collinear segments (dense brush samples) leave a sub-pixel gap no
        // backend can resolve, and a fan there is pure tessellation overhead.
        if let Some(prev) = prev_dir {
            let cos = (prev[0] * dir[0] + prev[1] * dir[1]).clamp(-1.0, 1.0);
            let sin = (prev[0] * dir[1] - prev[1] * dir[0]).abs();
            let gap = (half + 0.5) * sin / (1.0 + cos).max(1e-6);
            if gap >= 0.25 {
                out.push_round_join(a, half, rgba);
            }
        }
        prev_dir = Some(dir);
    }
}

/// Half-width of the coverage transition centered on every anti-aliased stroke edge (device px).
pub const STROKE_AA_HALF_PX: f32 = 0.5;
/// Signed edge distance of a fully covered interior vertex.
pub const STROKE_AA_SOLID: f32 = -1.0;

/// One anti-aliased stroke vertex: device-space position plus signed distance (device px,
/// positive outside) from the nominal stroke edge. Coverage is `clamp(0.5 - distance, 0, 1)`,
/// interpolated linearly across each triangle; [`STROKE_AA_SOLID`] marks the solid core.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeAaVertex {
    pub position: [f32; 2],
    pub distance: f32,
}

impl StrokeAaVertex {
    /// Linear pixel coverage for backends that encode anti-aliasing in vertex alpha.
    pub fn coverage(self) -> f32 {
        (STROKE_AA_HALF_PX - self.distance).clamp(0.0, 1.0)
    }
}

/// Anti-aliased polyline stroke over device-space `points` (already expanded for the line type).
///
/// Every segment gets a solid core ending half a device pixel inside the nominal edge plus a
/// centered 1 px coverage transition, so integrated coverage equals `width` and edges resolve to
/// continuous coverage instead of MSAA's few sample levels. Interior vertices get an anti-aliased
/// round join where the turn opens a visible wedge, and both ends get a fading butt-cap strip.
/// Triangles are emitted through `tri`, so each backend chooses its coverage encoding (vertex
/// alpha for WebGPU, the path-shader `st` channel for GPUI) over identical geometry.
pub fn stroke_aa(points: &[LinePoint], width: f32, mut tri: impl FnMut([StrokeAaVertex; 3])) {
    let half = (width / 2.0).max(0.0);
    if points.len() < 2 || half <= 0.0 {
        return;
    }
    let mut emit = |a: [f32; 2], da: f32, b: [f32; 2], db: f32, c: [f32; 2], dc: f32| {
        tri([
            StrokeAaVertex {
                position: a,
                distance: da,
            },
            StrokeAaVertex {
                position: b,
                distance: db,
            },
            StrokeAaVertex {
                position: c,
                distance: dc,
            },
        ]);
    };
    let core_half = (half - STROKE_AA_HALF_PX).max(0.0);
    let outer_half = half + STROKE_AA_HALF_PX;
    let fade_in = core_half - half;
    let fade_out = STROKE_AA_HALF_PX;
    let solid = STROKE_AA_SOLID;
    let mut prev_dir: Option<[f32; 2]> = None;
    let mut prev_b = [0.0f32; 2];
    let mut segments = points.windows(2).filter_map(|pair| {
        let a = [pair[0].x as f32, pair[0].y as f32];
        let b = [pair[1].x as f32, pair[1].y as f32];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            return None;
        }
        Some(StrokeSegment {
            a,
            b,
            dir: [dx / len, dy / len],
            len,
        })
    });
    // A four-segment window decides each joint's clipping without allocating: a joint's
    // validity depends on both neighbors and on how far their other ends are clipped.
    let mut before: Option<StrokeSegment> = None;
    let mut current = segments.next();
    let mut next = segments.next();
    let mut after = segments.next();
    let mut start_clipped = false;
    while let Some(segment) = current {
        let StrokeSegment { a, b, dir, .. } = segment;
        let end_clipped = next
            .is_some_and(|next| stroke_aa_joint_clips(before, segment, next, after, outer_half));
        let n = [-dir[1], dir[0]];
        if prev_dir.is_none() {
            stroke_aa_cap(&mut emit, a, [-dir[0], -dir[1]], n, half);
        }
        // The bisectors partition the inner side of a turn between neighboring segments.
        // Without them, a translucent stroke blends the same pixel more than once.
        let start = prev_dir
            .filter(|_| start_clipped)
            .map(|prev| (a, [prev[0] + dir[0], prev[1] + dir[1]]));
        let end = next
            .filter(|_| end_clipped)
            .map(|next| (b, [-dir[0] - next.dir[0], -dir[1] - next.dir[1]]));
        let mut segment_emit = |a, da, b, db, c, dc| {
            stroke_aa_clip(
                [
                    StrokeAaVertex {
                        position: a,
                        distance: da,
                    },
                    StrokeAaVertex {
                        position: b,
                        distance: db,
                    },
                    StrokeAaVertex {
                        position: c,
                        distance: dc,
                    },
                ],
                start,
                end,
                &mut |vertices| {
                    emit(
                        vertices[0].position,
                        vertices[0].distance,
                        vertices[1].position,
                        vertices[1].distance,
                        vertices[2].position,
                        vertices[2].distance,
                    );
                },
            );
        };
        let offset = |p: [f32; 2], distance: f32| [p[0] + n[0] * distance, p[1] + n[1] * distance];
        if core_half > 0.0 {
            let (la, lb) = (offset(a, core_half), offset(b, core_half));
            let (ra, rb) = (offset(a, -core_half), offset(b, -core_half));
            segment_emit(la, solid, lb, solid, rb, solid);
            segment_emit(la, solid, rb, solid, ra, solid);
        }
        for side in [1.0f32, -1.0] {
            let (in_a, in_b) = (offset(a, core_half * side), offset(b, core_half * side));
            let (out_a, out_b) = (offset(a, outer_half * side), offset(b, outer_half * side));
            segment_emit(out_a, fade_out, out_b, fade_out, in_b, fade_in);
            segment_emit(out_a, fade_out, in_b, fade_in, in_a, fade_in);
        }
        drop(segment_emit);
        if let Some(prev) = prev_dir {
            let cos = (prev[0] * dir[0] + prev[1] * dir[1]).clamp(-1.0, 1.0);
            let cross = prev[0] * dir[1] - prev[1] * dir[0];
            let gap = (half + 2.0 * STROKE_AA_HALF_PX) * cross.abs() / (1.0 + cos).max(1e-6);
            if gap >= 0.25 {
                // Only the outer side of a turn opens a wedge; the segments already cover the
                // inner side. Filling just that wedge keeps joins a few triangles instead of a
                // full disc per vertex.
                let side = if cross > 0.0 { -1.0 } else { 1.0 };
                let from = [-prev[1] * side, prev[0] * side];
                let to = [n[0] * side, n[1] * side];
                stroke_aa_join(&mut emit, a, half, from, to, cos);
            }
        }
        prev_dir = Some(dir);
        prev_b = b;
        start_clipped = end_clipped;
        before = current;
        current = next;
        next = after;
        after = segments.next();
    }
    if let Some(dir) = prev_dir {
        stroke_aa_cap(&mut emit, prev_b, dir, [-dir[1], dir[0]], half);
    }
}

#[derive(Clone, Copy)]
struct StrokeSegment {
    a: [f32; 2],
    b: [f32; 2],
    dir: [f32; 2],
    len: f32,
}

/// Whether the bisector at the joint between `incoming` and `outgoing` may clip both segments.
///
/// Each segment hands the other the inner geometry on its far side of the bisector. That trade
/// only covers the stroke when each neighbor extends past the region it must cover — at most
/// `radius·sin φ / min(1, 1 + cos φ)` back from the joint — without overlapping the region its
/// other end already gives away. Sharp turns between short segments (dense zig-zags, wide
/// strokes) fail that test and keep both segments unclipped: overlapping coverage there blends a
/// translucent stroke twice, which is far less visible than the hole clipping would cut.
fn stroke_aa_joint_clips(
    before: Option<StrokeSegment>,
    incoming: StrokeSegment,
    outgoing: StrokeSegment,
    after: Option<StrokeSegment>,
    radius: f32,
) -> bool {
    let turn = |u: [f32; 2], w: [f32; 2]| {
        (
            (u[0] * w[0] + u[1] * w[1]).clamp(-1.0, 1.0),
            (u[0] * w[1] - u[1] * w[0]).abs(),
        )
    };
    let reach = |sin: f32, denominator: f32| {
        if denominator <= 1e-6 {
            f32::INFINITY
        } else {
            radius * sin / denominator
        }
    };
    let (cos, sin) = turn(incoming.dir, outgoing.dir);
    let neighbor_reach = reach(sin, (1.0 + cos).min(1.0));
    // A far joint's bisector removes at most `radius·tan(φ/2)` of its own segment. Assuming the
    // far joints clip keeps each decision local to a four-segment window.
    let own_reach = |u: [f32; 2], w: [f32; 2]| {
        let (cos, sin) = turn(u, w);
        reach(sin, 1.0 + cos)
    };
    let incoming_start = before.map_or(0.0, |before| own_reach(before.dir, incoming.dir));
    let outgoing_end = after.map_or(0.0, |after| own_reach(outgoing.dir, after.dir));
    incoming.len >= neighbor_reach + incoming_start && outgoing.len >= neighbor_reach + outgoing_end
}

/// Clips a segment triangle to its neighboring joint bisectors without allocating. The round
/// outer wedge is emitted separately; these planes remove only the overlapping inner geometry.
fn stroke_aa_clip(
    triangle: [StrokeAaVertex; 3],
    start: Option<([f32; 2], [f32; 2])>,
    end: Option<([f32; 2], [f32; 2])>,
    emit: &mut impl FnMut([StrokeAaVertex; 3]),
) {
    if start.is_none() && end.is_none() {
        emit(triangle);
        return;
    }
    let mut input = [triangle[0]; 6];
    input[..3].copy_from_slice(&triangle);
    let mut output = [triangle[0]; 6];
    let mut len = 3;
    for plane in [start, end].into_iter().flatten() {
        let (edge_point, normal) = plane;
        if normal[0] * normal[0] + normal[1] * normal[1] < 1e-8 {
            continue;
        }
        let signed = |vertex: StrokeAaVertex| {
            (vertex.position[0] as f64 - edge_point[0] as f64) * normal[0] as f64
                + (vertex.position[1] as f64 - edge_point[1] as f64) * normal[1] as f64
        };
        let mut output_len = 0;
        for index in 0..len {
            let current = input[index];
            let next = input[(index + 1) % len];
            let d0 = signed(current);
            let d1 = signed(next);
            if (d0 >= 0.0) != (d1 >= 0.0) {
                // Adjacent input triangles share an edge in opposite directions. Canonicalizing
                // that edge makes both intersection vertices bit-identical and avoids hairline
                // cracks after independently clipping their triangles.
                let (first, second, first_distance, second_distance) =
                    if current.position < next.position {
                        (current, next, d0, d1)
                    } else {
                        (next, current, d1, d0)
                    };
                let t = first_distance / (first_distance - second_distance);
                output[output_len] = StrokeAaVertex {
                    position: [
                        (first.position[0] as f64
                            + (second.position[0] as f64 - first.position[0] as f64) * t)
                            as f32,
                        (first.position[1] as f64
                            + (second.position[1] as f64 - first.position[1] as f64) * t)
                            as f32,
                    ],
                    distance: (first.distance as f64
                        + (second.distance as f64 - first.distance as f64) * t)
                        as f32,
                };
                output_len += 1;
            }
            if d1 >= 0.0 {
                output[output_len] = next;
                output_len += 1;
            }
        }
        len = output_len;
        input[..len].copy_from_slice(&output[..len]);
        if len < 3 {
            return;
        }
    }
    for index in 1..len - 1 {
        emit([input[0], input[index], input[index + 1]]);
    }
}

/// The exterior half of the centered coverage transition across a butt cap; it meets the side
/// transitions at the corners.
fn stroke_aa_cap(
    emit: &mut impl FnMut([f32; 2], f32, [f32; 2], f32, [f32; 2], f32),
    at: [f32; 2],
    out: [f32; 2],
    n: [f32; 2],
    half: f32,
) {
    let r = half + STROKE_AA_HALF_PX;
    let edge = 0.0;
    let fade_out = STROKE_AA_HALF_PX;
    let in_l = [at[0] + n[0] * r, at[1] + n[1] * r];
    let in_r = [at[0] - n[0] * r, at[1] - n[1] * r];
    let out_l = [
        at[0] + out[0] * STROKE_AA_HALF_PX + n[0] * r,
        at[1] + out[1] * STROKE_AA_HALF_PX + n[1] * r,
    ];
    let out_r = [
        at[0] + out[0] * STROKE_AA_HALF_PX - n[0] * r,
        at[1] + out[1] * STROKE_AA_HALF_PX - n[1] * r,
    ];
    emit(in_l, edge, in_r, edge, out_r, fade_out);
    emit(in_l, edge, out_r, fade_out, out_l, fade_out);
}

/// A coverage-exact round join over the outer wedge of a turn, from unit normal `from` to unit
/// normal `to` (the offset directions of the two segments on the outer side). A solid fan stops
/// half a device pixel inside the nominal radius, then a 1 px transition straddles it so the
/// requested stroke width is preserved. The first and last spokes reuse the exact segment normals,
/// so the wedge meets both segments' edges without a seam.
fn stroke_aa_join(
    emit: &mut impl FnMut([f32; 2], f32, [f32; 2], f32, [f32; 2], f32),
    center: [f32; 2],
    radius: f32,
    from: [f32; 2],
    to: [f32; 2],
    cos_turn: f32,
) {
    let core = (radius - STROKE_AA_HALF_PX).max(0.0);
    let rim = radius + STROKE_AA_HALF_PX;
    let fade_in = core - radius;
    let fade_out = STROKE_AA_HALF_PX;
    // Keep the full-circle chord density over the wedge's share of the circle.
    let turn = cos_turn.clamp(-1.0, 1.0).acos();
    let steps =
        ((join_segments(radius) as f32 * turn / std::f32::consts::TAU).ceil() as usize).max(1);
    // Rotate `from` toward `to` by equal angles; the end spoke is `to` exactly.
    let signed = (from[0] * to[1] - from[1] * to[0]).signum();
    let step = turn / steps as f32;
    let (step_cos, step_sin) = (step.cos(), step.sin() * signed);
    let mut previous = from;
    for index in 1..=steps {
        let spoke = if index == steps {
            to
        } else {
            [
                previous[0] * step_cos - previous[1] * step_sin,
                previous[0] * step_sin + previous[1] * step_cos,
            ]
        };
        let at = |normal: [f32; 2], distance: f32| {
            [
                center[0] + normal[0] * distance,
                center[1] + normal[1] * distance,
            ]
        };
        let (p0, p1) = (at(previous, core), at(spoke, core));
        if core > 0.0 {
            emit(
                center,
                STROKE_AA_SOLID,
                p0,
                STROKE_AA_SOLID,
                p1,
                STROKE_AA_SOLID,
            );
        }
        let (o0, o1) = (at(previous, rim), at(spoke, rim));
        emit(p0, fade_in, o0, fade_out, o1, fade_out);
        emit(p0, fade_in, o1, fade_out, p1, fade_in);
        previous = spoke;
    }
}

fn self_push_segment(
    out: &mut StrokeMesh,
    a: [f32; 2],
    b: [f32; 2],
    nx: f32,
    ny: f32,
    rgba: [f32; 4],
) {
    out.push_quad(
        [a[0] + nx, a[1] + ny],
        [b[0] + nx, b[1] + ny],
        [b[0] - nx, b[1] - ny],
        [a[0] - nx, a[1] - ny],
        rgba,
    );
}

/// Area fill mesh: triangle list between the polyline and `base_y` (media px). The backend
/// tints each vertex with a vertical gradient (top color at the line, bottom at base) — here
/// we only emit positions and per-vertex gradient factor via color alpha lerp done by caller.
#[derive(Default)]
pub struct AreaMesh {
    /// Each vertex carries its media-y so the backend can look up the gradient; color is the
    /// resolved top/bottom mix computed here for simplicity (single draw, no gradient uniform).
    pub vertices: Vec<LineVertex>,
    /// Vertex offsets of the six-vertex segments whose line crosses the base, ascending. Such a
    /// segment is `[a, crossing, a_base, crossing, b, b_base]`: one simple lobe ends at
    /// `crossing` and the next begins there, so contour tracers split lobes from this list
    /// instead of comparing coordinates.
    pub crossings: Vec<usize>,
}

/// Builds an area fill under `points` down to `base_y` (media px), vertically gradient-shaded
/// from `top_color` (at each point) to `bottom_color` (at `base_y`). Colors are premultiplied
/// per-vertex here so the existing solid triangle path can draw it.
pub fn build_area_fill(
    points: &[LinePoint],
    base_y: f64,
    top_color: Color,
    bottom_color: Color,
    params: &LineParams,
    out: &mut AreaMesh,
) {
    let mut expanded = Vec::new();
    expand_line_into(
        points,
        params.line_type,
        params.horizontal_pixel_ratio,
        params.vertical_pixel_ratio,
        &mut expanded,
    );
    let points = &expanded[..];
    if points.len() < 2 {
        return;
    }
    let hpr = params.horizontal_pixel_ratio;
    let vpr = params.vertical_pixel_ratio;
    let base = (base_y * vpr) as f32;

    let top = color_to_rgba(top_color);
    let bottom = color_to_rgba(bottom_color);

    // Gradient factor 0 at the fill's geometric top (top color), 1 at its geometric base
    // (bottom color). A normal fill spans [topmost point, base_y] below the line; an inverted
    // one (reference `invertFilledArea`, or a baseline segment under the base level) spans
    // [base_y, lowest point] above the line. Both directions keep the top stop at the
    // geometrically higher edge so the two backends shade identically.
    let min_y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min) * vpr;
    let max_y = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max) * vpr;
    let top_coord = min_y.min(base as f64);
    let span = (max_y.max(base as f64) - top_coord).max(1.0);
    let shade = |y: f32| -> [f32; 4] {
        let t = ((y as f64 - top_coord) / span).clamp(0.0, 1.0) as f32;
        [
            top[0] + (bottom[0] - top[0]) * t,
            top[1] + (bottom[1] - top[1]) * t,
            top[2] + (bottom[2] - top[2]) * t,
            top[3] + (bottom[3] - top[3]) * t,
        ]
    };
    let vert = |x: f32, y: f32| LineVertex {
        x,
        y,
        color: shade(y),
    };

    let bp = |p: &LinePoint| [(p.x * hpr) as f32, (p.y * vpr) as f32];

    for i in 0..points.len() - 1 {
        let a = bp(&points[i]);
        let b = bp(&points[i + 1]);
        if (a[1] - base) * (b[1] - base) < 0.0 {
            let t = (base - a[1]) / (b[1] - a[1]);
            let crossing = vert(a[0] + (b[0] - a[0]) * t, base);
            out.crossings.push(out.vertices.len());
            out.vertices.extend([
                vert(a[0], a[1]),
                crossing,
                vert(a[0], base),
                crossing,
                vert(b[0], b[1]),
                vert(b[0], base),
            ]);
            continue;
        }
        // quad a -> b -> (b.x, base) -> (a.x, base)
        let a_top = vert(a[0], a[1]);
        let b_top = vert(b[0], b[1]);
        let b_base = vert(b[0], base);
        let a_base = vert(a[0], base);
        out.vertices.push(a_top);
        out.vertices.push(b_top);
        out.vertices.push(b_base);
        out.vertices.push(a_top);
        out.vertices.push(b_base);
        out.vertices.push(a_base);
    }
}

/// Two triangles for one band segment. At a boundary crossing, split at the intersection so
/// the two filled lobes meet at one point instead of making an overlapping bow-tie quad.
pub fn band_segment_triangles(
    upper0: [f32; 2],
    upper1: [f32; 2],
    lower0: [f32; 2],
    lower1: [f32; 2],
) -> [[f32; 2]; 6] {
    let d0 = upper0[1] - lower0[1];
    let d1 = upper1[1] - lower1[1];
    if d0 * d1 < 0.0 {
        let t = d0 / (d0 - d1);
        let upper = [
            upper0[0] + (upper1[0] - upper0[0]) * t,
            upper0[1] + (upper1[1] - upper0[1]) * t,
        ];
        let lower = [
            lower0[0] + (lower1[0] - lower0[0]) * t,
            lower0[1] + (lower1[1] - lower0[1]) * t,
        ];
        let crossing = [(upper[0] + lower[0]) * 0.5, (upper[1] + lower[1]) * 0.5];
        [upper0, lower0, crossing, crossing, lower1, upper1]
    } else {
        [upper0, lower0, lower1, upper0, lower1, upper1]
    }
}

/// Builds a **baseline** series: a line whose portions above `baseline_y` (media px) use
/// `top_line`/`top_fill` and portions below use `bottom_line`/`bottom_fill`, with an area fill to
/// the baseline. Segments crossing the baseline are split at the crossing so the color flips
/// exactly there (port of `baseline-renderer-*.ts`). Smaller y = higher
/// price = "above".
#[allow(clippy::too_many_arguments)]
pub fn build_baseline(
    points: &[LinePoint],
    baseline_y: f64,
    top_line: Color,
    bottom_line: Color,
    top_fill: Color,
    bottom_fill: Color,
    params: &LineParams,
    stroke: &mut StrokeMesh,
    fill: &mut AreaMesh,
) {
    let mut expanded = Vec::new();
    expand_line_into(
        points,
        params.line_type,
        params.horizontal_pixel_ratio,
        params.vertical_pixel_ratio,
        &mut expanded,
    );
    let pts = &expanded[..];
    if pts.len() < 2 {
        return;
    }
    let hpr = params.horizontal_pixel_ratio;
    let vpr = params.vertical_pixel_ratio;
    let half = (params.line_width * vpr / 2.0) as f32;
    let base_b = (baseline_y * vpr) as f32;

    // split segment (a,b) at the baseline crossing into 1 or 2 sub-segments in media coords
    let split = |a: LinePoint, b: LinePoint| -> Vec<(LinePoint, LinePoint)> {
        let above_a = a.y < baseline_y;
        let above_b = b.y < baseline_y;
        if above_a == above_b || (b.y - a.y).abs() < 1e-9 {
            vec![(a, b)]
        } else {
            let t = (baseline_y - a.y) / (b.y - a.y);
            let c = LinePoint {
                x: a.x + (b.x - a.x) * t,
                y: baseline_y,
            };
            vec![(a, c), (c, b)]
        }
    };

    for i in 0..pts.len() - 1 {
        for (s0, s1) in split(pts[i], pts[i + 1]) {
            let above = ((s0.y + s1.y) / 2.0) < baseline_y;
            let lc = color_to_rgba(if above { top_line } else { bottom_line });
            let fc = color_to_rgba(if above { top_fill } else { bottom_fill });
            let a = [(s0.x * hpr) as f32, (s0.y * vpr) as f32];
            let b = [(s1.x * hpr) as f32, (s1.y * vpr) as f32];

            // fill between the sub-segment and the baseline
            let av = LineVertex {
                x: a[0],
                y: a[1],
                color: fc,
            };
            let bv = LineVertex {
                x: b[0],
                y: b[1],
                color: fc,
            };
            let ab = LineVertex {
                x: a[0],
                y: base_b,
                color: fc,
            };
            let bb = LineVertex {
                x: b[0],
                y: base_b,
                color: fc,
            };
            fill.vertices.extend([av, bv, bb, av, bb, ab]);

            // stroke the sub-segment
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let len = (dx * dx + dy * dy).sqrt();
            if len >= 1e-6 {
                let nx = -dy / len * half;
                let ny = dx / len * half;
                stroke.push_quad(
                    [a[0] + nx, a[1] + ny],
                    [b[0] + nx, b[1] + ny],
                    [b[0] - nx, b[1] - ny],
                    [a[0] - nx, a[1] - ny],
                    lc,
                );
            }
        }
    }
}

/// Closed polygon outline of a rounded rectangle (CSS proportional radius scaling), shared by
/// the WebGPU and GPUI executors so both tessellate
/// `Prim::RoundRect` identically. Corner arcs scale their chord count with the device radius.
pub fn round_rect_polygon(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Vec<[f32; 2]> {
    use std::f32::consts::PI;
    let [lt, rt, rb, lb] = normalized_round_rect_radii(w, h, radii);
    let mut out = Vec::with_capacity(24);
    out.push([x + lt, y]);
    out.push([x + w - rt, y]);
    append_arc(&mut out, x + w - rt, y + rt, rt, -PI / 2.0, 0.0);
    out.push([x + w, y + h - rb]);
    append_arc(&mut out, x + w - rb, y + h - rb, rb, 0.0, PI / 2.0);
    out.push([x + lb, y + h]);
    append_arc(&mut out, x + lb, y + h - lb, lb, PI / 2.0, PI);
    out.push([x, y + lt]);
    append_arc(&mut out, x + lt, y + lt, lt, PI, 3.0 * PI / 2.0);
    out
}

/// CSS border-radius overlap rule: all corners share one scale factor, preserving their ratios.
pub fn normalized_round_rect_radii(w: f32, h: f32, radii: [f32; 4]) -> [f32; 4] {
    let radii = radii.map(|r| if r.is_finite() { r.max(0.0) } else { 0.0 });
    let [lt, rt, rb, lb] = radii;
    let mut factor = 1.0_f32;
    for (side, sum) in [
        (w.max(0.0), lt + rt),
        (h.max(0.0), rt + rb),
        (w.max(0.0), rb + lb),
        (h.max(0.0), lb + lt),
    ] {
        if sum > 0.0 {
            factor = factor.min(side / sum);
        }
    }
    radii.map(|r| r * factor)
}

/// Geometry of a rounded rectangle with an inside border, shared by the WebGPU and GPUI
/// executors so the fill and the border tessellate identically on both.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoundRectBorder {
    /// Closed outer contour (the last vertex repeats the first).
    pub outer: Vec<[f32; 2]>,
    /// Closed contour bounding the inner fill; empty when the border covers the whole rect.
    /// The ring's inner edge is built from exactly these vertices, so a fill fanned over them
    /// meets the ring without a crescent gap or an overlap at rounded corners.
    pub inner: Vec<[f32; 2]>,
    /// Triangle list covering only the border, never the inner fill area.
    pub ring: Vec<[f32; 2]>,
}

/// Inside rounded-rectangle border plus its inner fill contour. The paired outer and inner
/// contours share each corner's angular steps (chosen from the outer radius), so the ring strip
/// never paints behind a translucent or transparent inner fill.
pub fn round_rect_border(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: [f32; 4],
    border_width: f32,
) -> RoundRectBorder {
    use std::f32::consts::PI;
    if w <= 0.0 || h <= 0.0 || border_width <= 0.0 {
        return RoundRectBorder::default();
    }
    let inset = border_width.min(w / 2.0).min(h / 2.0);
    let [lt, rt, rb, lb] = normalized_round_rect_radii(w, h, radii);
    if inset * 2.0 >= w || inset * 2.0 >= h {
        let outer = round_rect_polygon(x, y, w, h, radii);
        let center = [x + w / 2.0, y + h / 2.0];
        let ring = outer
            .windows(2)
            .flat_map(|edge| [center, edge[0], edge[1]])
            .collect();
        return RoundRectBorder {
            outer,
            inner: Vec::new(),
            ring,
        };
    }
    let (ix, iy, iw, ih) = (x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset);
    let [ilt, irt, irb, ilb] = [lt, rt, rb, lb].map(|radius| (radius - inset).max(0.0));
    let mut contours = Vec::<([f32; 2], [f32; 2])>::with_capacity(48);
    let corner = |contours: &mut Vec<([f32; 2], [f32; 2])>,
                  outer_center: [f32; 2],
                  inner_center: [f32; 2],
                  outer_radius: f32,
                  inner_radius: f32,
                  start: f32,
                  end: f32| {
        let steps = round_rect_arc_steps(outer_radius);
        for step in 1..=steps {
            let angle = start + (end - start) * step as f32 / steps as f32;
            let (sin, cos) = angle.sin_cos();
            contours.push((
                [
                    outer_center[0] + outer_radius * cos,
                    outer_center[1] + outer_radius * sin,
                ],
                [
                    inner_center[0] + inner_radius * cos,
                    inner_center[1] + inner_radius * sin,
                ],
            ));
        }
    };
    contours.push(([x + lt, y], [ix + ilt, iy]));
    contours.push(([x + w - rt, y], [ix + iw - irt, iy]));
    corner(
        &mut contours,
        [x + w - rt, y + rt],
        [ix + iw - irt, iy + irt],
        rt,
        irt,
        -PI / 2.0,
        0.0,
    );
    contours.push(([x + w, y + h - rb], [ix + iw, iy + ih - irb]));
    corner(
        &mut contours,
        [x + w - rb, y + h - rb],
        [ix + iw - irb, iy + ih - irb],
        rb,
        irb,
        0.0,
        PI / 2.0,
    );
    contours.push(([x + lb, y + h], [ix + ilb, iy + ih]));
    corner(
        &mut contours,
        [x + lb, y + h - lb],
        [ix + ilb, iy + ih - ilb],
        lb,
        ilb,
        PI / 2.0,
        PI,
    );
    contours.push(([x, y + lt], [ix, iy + ilt]));
    corner(
        &mut contours,
        [x + lt, y + lt],
        [ix + ilt, iy + ilt],
        lt,
        ilt,
        PI,
        3.0 * PI / 2.0,
    );
    let mut ring = Vec::with_capacity((contours.len() - 1) * 6);
    for edge in contours.windows(2) {
        let (outer0, inner0) = edge[0];
        let (outer1, inner1) = edge[1];
        ring.extend([outer0, outer1, inner1, outer0, inner1, inner0]);
    }
    let (outer, inner) = contours.into_iter().unzip();
    RoundRectBorder { outer, inner, ring }
}

/// Quarter-arc chords scale with the device radius so pill ends stay round: the chord error is
/// about `r·(π/2)²/(8·steps²)`, held near 0.05 device px, with small corners keeping 4 chords.
fn append_arc(out: &mut Vec<[f32; 2]>, cx: f32, cy: f32, radius: f32, start: f32, end: f32) {
    let steps = round_rect_arc_steps(radius);
    for step in 1..=steps {
        let t = start + (end - start) * (step as f32 / steps as f32);
        out.push([cx + radius * t.cos(), cy + radius * t.sin()]);
    }
}

fn round_rect_arc_steps(radius: f32) -> usize {
    (radius.max(0.0).sqrt() * 2.5).ceil().clamp(4.0, 24.0) as usize
}

/// Full-circle tessellation from a 0.1 device-pixel maximum chord error through radius 200.
/// The cap bounds frame work for arbitrarily large offscreen circles.
pub fn circle_segments(radius: f32) -> usize {
    if !radius.is_finite() || radius <= 0.0 {
        return 0;
    }
    let radius = f64::from(radius);
    let angle = (1.0 - 0.1 / radius).clamp(-1.0, 1.0).acos();
    (std::f64::consts::PI / angle).ceil().clamp(24.0, 256.0) as usize
}

/// Tessellates a filled disc (triangle fan) at `center` with `radius`, all in bitmap px.
/// Used for the crosshair marker on line and area series.
pub fn build_disc(center: [f32; 2], radius: f32, color: Color, out: &mut Vec<LineVertex>) {
    let segments = circle_segments(radius);
    let rgba = color_to_rgba(color);
    let v = |x: f32, y: f32| LineVertex { x, y, color: rgba };
    let c = v(center[0], center[1]);
    for i in 0..segments {
        let a0 = (i as f32) / segments as f32 * std::f32::consts::TAU;
        let a1 = ((i + 1) as f32) / segments as f32 * std::f32::consts::TAU;
        out.push(c);
        out.push(v(
            center[0] + radius * a0.cos(),
            center[1] + radius * a0.sin(),
        ));
        out.push(v(
            center[0] + radius * a1.cos(),
            center[1] + radius * a1.sin(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLUE: Color = Color::rgb(0x21, 0x96, 0xf3);

    fn triangle_covers(a: [f32; 2], b: [f32; 2], c: [f32; 2], p: [f32; 2]) -> bool {
        let side = |a: [f32; 2], b: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
        };
        let sides = [side(a, b), side(b, c), side(c, a)];
        sides.iter().all(|v| *v > 1e-5) || sides.iter().all(|v| *v < -1e-5)
    }

    #[test]
    fn area_crossing_base_has_exact_nonoverlapping_lobes() {
        let points = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 10.0, y: 10.0 }];
        let mut mesh = AreaMesh::default();
        build_area_fill(&points, 5.0, BLUE, BLUE, &params(1.0, 1.0), &mut mesh);
        assert_eq!(
            mesh.crossings,
            vec![0],
            "the only segment splits at the base"
        );
        let coverage = |p: [f32; 2]| {
            mesh.vertices
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|tri| {
                    triangle_covers(
                        [tri[0].x, tri[0].y],
                        [tri[1].x, tri[1].y],
                        [tri[2].x, tri[2].y],
                        p,
                    )
                })
                .count()
        };
        for (point, expected) in [
            ([3.0, 4.0], 1),
            ([3.0, 6.0], 0),
            ([7.0, 6.0], 1),
            ([7.0, 4.0], 0),
        ] {
            assert_eq!(coverage(point), expected, "at {point:?}");
        }
        for ix in 1..40 {
            for iy in 1..40 {
                let point = [ix as f32 * 0.25, iy as f32 * 0.25];
                if (point[1] - point[0]).abs() < 1e-4 || (point[1] - 5.0).abs() < 1e-4 {
                    continue;
                }
                let expected =
                    usize::from(point[1] > point[0].min(5.0) && point[1] < point[0].max(5.0));
                assert_eq!(coverage(point), expected, "at {point:?}");
            }
        }
    }

    #[test]
    fn translucent_zigzag_stroke_never_blends_two_triangles_at_one_sample() {
        let points = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 30.0, y: 30.0 },
            LinePoint { x: 60.0, y: 0.0 },
        ];
        let mut triangles = Vec::new();
        stroke_aa(&points, 8.0, |triangle| triangles.push(triangle));
        for ix in 23..37 {
            for iy in 22..39 {
                let sample = [ix as f32 + 0.37, iy as f32 + 0.23];
                let total = triangles
                    .iter()
                    .filter_map(|triangle| coverage_at(std::slice::from_ref(triangle), sample))
                    .sum::<f32>();
                assert!(total <= 1.01, "overlapping coverage {total} at {sample:?}");
            }
        }
    }

    /// Sum of interpolated coverage of every triangle containing `p`.
    fn total_coverage(tris: &[[StrokeAaVertex; 3]], p: [f32; 2]) -> f32 {
        tris.iter()
            .filter_map(|triangle| coverage_at(std::slice::from_ref(triangle), p))
            .sum()
    }

    /// Whether `p` lies at least `inset` inside the ideal stroke: the union of every segment's
    /// butt-ended rectangle and a disc at each interior vertex.
    fn inside_ideal_stroke(points: &[[f32; 2]], half: f32, inset: f32, p: [f32; 2]) -> bool {
        let r = half - inset;
        let last = points.len() - 2;
        let in_segment = points.windows(2).enumerate().any(|(index, pair)| {
            let (a, b) = (pair[0], pair[1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = d[0].hypot(d[1]);
            if len < 1e-6 {
                return false;
            }
            let rel = [p[0] - a[0], p[1] - a[1]];
            let along = (rel[0] * d[0] + rel[1] * d[1]) / len;
            let across = (rel[0] * d[1] - rel[1] * d[0]).abs() / len;
            // The butt ends meet a half-coverage cap exactly at the endpoint.
            let lo = if index == 0 { inset } else { 0.0 };
            let hi = if index == last { len - inset } else { len };
            along > lo && along < hi && across < r
        });
        // Join arcs are chords at `join_segments` density, so only their inscribed core counts.
        let disc = (half - STROKE_AA_HALF_PX)
            * (std::f32::consts::PI / join_segments(half) as f32).cos()
            - (inset - STROKE_AA_HALF_PX);
        in_segment
            || points[1..points.len() - 1]
                .iter()
                .any(|v| (p[0] - v[0]).hypot(p[1] - v[1]) < disc)
    }

    fn sharp_stroke_cases() -> Vec<(Vec<[f32; 2]>, f32)> {
        let zigzag = |dx: f32, dy: f32, count: usize| -> Vec<[f32; 2]> {
            (0..count)
                .map(|i| {
                    [
                        20.0 + i as f32 * dx,
                        20.0 + if i % 2 == 0 { 0.0 } else { dy },
                    ]
                })
                .collect()
        };
        let mut cases = vec![
            (zigzag(1.0, 10.0, 12), 8.0),
            (zigzag(10.0, 1.0, 12), 8.0),
            (zigzag(2.0, 10.0, 12), 8.0),
            (zigzag(1.0, 10.0, 12), 16.0),
            (zigzag(3.0, 4.0, 12), 12.0),
            // Hairpins whose middle segment is far shorter than the stroke width.
            (
                vec![[10.0, 20.0], [60.0, 20.0], [60.0, 22.0], [10.0, 22.0]],
                8.0,
            ),
            (
                vec![[10.0, 20.0], [60.0, 20.0], [61.0, 23.0], [10.0, 25.0]],
                10.0,
            ),
            (
                vec![
                    [10.0, 20.0],
                    [60.0, 20.0],
                    [58.0, 21.0],
                    [61.0, 22.0],
                    [10.0, 30.0],
                ],
                8.0,
            ),
        ];
        // Deterministic irregular sharp scribbles with short and long neighbors.
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed % 10_000) as f32 / 10_000.0
        };
        for _ in 0..12 {
            let mut points = vec![[40.0f32, 40.0]];
            let mut heading = next() * std::f32::consts::TAU;
            for _ in 0..8 {
                let length = 1.0 + next() * 14.0;
                heading += std::f32::consts::PI
                    * (0.55 + next() * 0.4)
                    * if next() < 0.5 { 1.0 } else { -1.0 };
                let last = *points.last().unwrap();
                points.push([
                    last[0] + heading.cos() * length,
                    last[1] + heading.sin() * length,
                ]);
            }
            cases.push((points, 6.0 + next() * 10.0));
        }
        cases
    }

    #[test]
    fn sharp_strokes_with_short_neighbors_cover_the_whole_core() {
        let mut failures = Vec::new();
        for (points, width) in sharp_stroke_cases() {
            let line: Vec<LinePoint> = points
                .iter()
                .map(|p| LinePoint {
                    x: p[0] as f64,
                    y: p[1] as f64,
                })
                .collect();
            let mut tris = Vec::new();
            stroke_aa(&line, width, |triangle| tris.push(triangle));
            let (min, max) = points.iter().fold(
                ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
                |(lo, hi), p| {
                    (
                        [lo[0].min(p[0]), lo[1].min(p[1])],
                        [hi[0].max(p[0]), hi[1].max(p[1])],
                    )
                },
            );
            let pad = width;
            let mut holes = 0;
            let mut first_hole = None;
            let mut y = min[1] - pad + 0.231;
            while y < max[1] + pad {
                let mut x = min[0] - pad + 0.137;
                while x < max[0] + pad {
                    let p = [x, y];
                    if inside_ideal_stroke(&points, width / 2.0, 0.55, p) {
                        let total = total_coverage(&tris, p);
                        if total < 0.99 {
                            holes += 1;
                            first_hole.get_or_insert((p, total));
                        }
                    }
                    x += 0.25;
                }
                y += 0.25;
            }
            if holes > 0 {
                failures.push(format!(
                    "width {width} {points:?}: {holes} uncovered core samples, first {first_hole:?}"
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn rounded_border_ring_and_inner_fill_cover_each_sample_exactly_once() {
        // The executors fan the inner fill around its centroid.
        let fan = |contour: &[[f32; 2]]| -> Vec<[[f32; 2]; 3]> {
            let n = contour.len() as f32;
            let center = [
                contour.iter().map(|p| p[0]).sum::<f32>() / n,
                contour.iter().map(|p| p[1]).sum::<f32>() / n,
            ];
            let mut tris: Vec<_> = contour
                .windows(2)
                .map(|edge| [center, edge[0], edge[1]])
                .collect();
            tris.push([center, contour[contour.len() - 1], contour[0]]);
            tris
        };
        let inside_polygon = |polygon: &[[f32; 2]], p: [f32; 2]| {
            let mut inside = false;
            for edge in polygon.windows(2) {
                let (a, b) = (edge[0], edge[1]);
                if (a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0])
                {
                    inside = !inside;
                }
            }
            inside
        };
        for (rect, radii, border) in [
            (
                [10.0f32, 10.0, 40.0, 30.0],
                [8.0f32, 3.0, 0.0, 12.0],
                2.0f32,
            ),
            ([10.0, 10.0, 60.0, 20.0], [20.0; 4], 3.0),
            ([10.0, 10.0, 30.0, 30.0], [1.0, 6.0, 1.0, 6.0], 3.0),
            ([5.0, 5.0, 80.0, 50.0], [24.0, 24.0, 4.0, 4.0], 1.5),
        ] {
            let [x, y, w, h] = rect;
            let geometry = round_rect_border(x, y, w, h, radii, border);
            assert!(!geometry.inner.is_empty());
            let mut tris = fan(&geometry.inner);
            tris.extend(geometry.ring.as_chunks::<3>().0.iter().copied());
            let mut sy = y - 1.0 + 0.0137;
            while sy < y + h + 1.0 {
                let mut sx = x - 1.0 + 0.0291;
                while sx < x + w + 1.0 {
                    let p = [sx, sy];
                    let count = tris
                        .iter()
                        .filter(|t| triangle_covers(t[0], t[1], t[2], p))
                        .count();
                    let expected = usize::from(inside_polygon(&geometry.outer, p));
                    assert_eq!(
                        count, expected,
                        "rect {rect:?} radii {radii:?} border {border}: sample {p:?}"
                    );
                    sx += 0.173;
                }
                sy += 0.181;
            }
        }
    }

    #[test]
    fn clipped_turns_with_long_neighbors_cover_each_sample_once() {
        for turn_degrees in [30.0f32, 90.0, 135.0, 160.0] {
            for side in [1.0f32, -1.0] {
                let heading = f64::from(side * turn_degrees.to_radians());
                let points = [
                    LinePoint { x: 0.0, y: 60.0 },
                    LinePoint { x: 60.0, y: 60.0 },
                    LinePoint {
                        x: 60.0 + 60.0 * heading.cos(),
                        y: 60.0 + 60.0 * heading.sin(),
                    },
                ];
                let mut tris = Vec::new();
                stroke_aa(&points, 8.0, |triangle| tris.push(triangle));
                for ix in 0..120 {
                    for iy in 0..120 {
                        // Offsets keep samples off the shared spokes and diagonals.
                        let sample = [30.013 + ix as f32 * 0.3719, 30.029 + iy as f32 * 0.4127];
                        let total = total_coverage(&tris, sample);
                        assert!(
                            total <= 1.01,
                            "turn {turn_degrees} side {side}: coverage {total} at {sample:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clipped_curved_stroke_keeps_inner_band() {
        let points = [
            LinePoint {
                x: 159.459716796875,
                y: 57.853759765625,
            },
            LinePoint {
                x: 164.939453125,
                y: 59.958984375,
            },
            LinePoint {
                x: 170.453369140625,
                y: 61.377685546875,
            },
            LinePoint { x: 176.0, y: 62.0 },
        ];
        let mut tris = Vec::new();
        stroke_aa(&points, 6.0, |triangle| tris.push(triangle));
        assert!(coverage_at(&tris, [169.76013, 58.359753]).is_some());
    }

    #[test]
    fn circle_chord_error_stays_below_tenth_device_pixel() {
        for radius in 1..=200 {
            let mut vertices = Vec::new();
            build_disc([0.0, 0.0], radius as f32, BLUE, &mut vertices);
            let segments = vertices.len() / 3;
            let error = radius as f32 * (1.0 - (std::f32::consts::PI / segments as f32).cos());
            assert!(
                error <= 0.10001,
                "radius {radius}, {segments} segments: {error} px"
            );
            assert!(segments <= 256, "tessellation must stay bounded");
        }
    }

    /// Interpolated coverage at `p`, or `None` outside every triangle.
    fn coverage_at(tris: &[[StrokeAaVertex; 3]], p: [f32; 2]) -> Option<f32> {
        tris.iter().find_map(|[a, b, c]| {
            let (a, b, c) = (a, b, c);
            let det = (b.position[1] - c.position[1]) * (a.position[0] - c.position[0])
                + (c.position[0] - b.position[0]) * (a.position[1] - c.position[1]);
            if det.abs() < 1e-9 {
                return None;
            }
            let l1 = ((b.position[1] - c.position[1]) * (p[0] - c.position[0])
                + (c.position[0] - b.position[0]) * (p[1] - c.position[1]))
                / det;
            let l2 = ((c.position[1] - a.position[1]) * (p[0] - c.position[0])
                + (a.position[0] - c.position[0]) * (p[1] - c.position[1]))
                / det;
            let l3 = 1.0 - l1 - l2;
            (l1 >= -1e-6 && l2 >= -1e-6 && l3 >= -1e-6)
                .then(|| l1 * a.coverage() + l2 * b.coverage() + l3 * c.coverage())
        })
    }

    #[test]
    fn aa_round_joins_cover_the_whole_corner_for_both_turn_directions() {
        let width = 6.0f32;
        let half = width / 2.0;
        // Right-angle and acute turns, both turning directions, four orientations each.
        let turns = [
            [[0.0, 50.0], [50.0, 50.0], [50.0, 0.0]],
            [[0.0, 50.0], [50.0, 50.0], [50.0, 100.0]],
            [[50.0, 0.0], [50.0, 50.0], [0.0, 50.0]],
            [[50.0, 0.0], [50.0, 50.0], [100.0, 50.0]],
            [[0.0, 50.0], [50.0, 50.0], [10.0, 40.0]],
            [[0.0, 50.0], [50.0, 50.0], [10.0, 60.0]],
        ];
        for turn in turns {
            let points = turn.map(|[x, y]| LinePoint { x, y });
            let mut tris = Vec::new();
            stroke_aa(&points, width, |tri| tris.push(tri));
            // Every point strictly inside the join's solid core must be fully covered: no wedge on
            // the wrong side, no seam between the wedge and the segments.
            for step in 0..72 {
                let angle = step as f32 / 72.0 * std::f32::consts::TAU;
                // Probe inside the core polygon's inscribed radius: arcs are chords (the same
                // `join_segments` density as the full-disc join), so the rim itself is approximate.
                let inscribed = (half - STROKE_AA_HALF_PX)
                    * (std::f32::consts::PI / join_segments(half) as f32).cos();
                for distance in [0.3, 1.0, inscribed - 0.05] {
                    let p = [50.0 + angle.cos() * distance, 50.0 + angle.sin() * distance];
                    let coverage = tris
                        .iter()
                        .filter_map(|tri| coverage_at(std::slice::from_ref(tri), p))
                        .fold(0.0f32, f32::max);
                    assert!(
                        coverage > 0.99,
                        "turn {turn:?}: hole at angle {angle:.2} distance {distance} ({coverage})"
                    );
                }
            }
        }
    }

    #[test]
    fn aa_stroke_conserves_width_and_ramps_edges_continuously() {
        // Sub-device-pixel hairlines approximate their weight (a 0.5 px stroke integrates to
        // ~0.56); every width a chart line can take (≥ 1 device px) is exact.
        for width in [1.0f32, 2.0, 3.0, 4.5] {
            let points = [
                LinePoint { x: 0.0, y: 50.0 },
                LinePoint { x: 100.0, y: 50.0 },
            ];
            let mut tris = Vec::new();
            stroke_aa(&points, width, |tri| tris.push(tri));
            // Integrate the vertical coverage profile through the segment's middle.
            let step = 0.01f32;
            let mut integral = 0.0f32;
            let mut levels = std::collections::BTreeSet::new();
            let mut y = 45.0f32;
            while y < 55.0 {
                if let Some(coverage) = coverage_at(&tris, [50.0, y]) {
                    integral += coverage * step;
                    levels.insert((coverage * 100.0).round() as i32);
                }
                y += step;
            }
            assert!(
                (integral - width).abs() < 0.05,
                "width {width}: integrated coverage {integral}"
            );
            // A continuous ramp, not a handful of MSAA-like steps.
            assert!(levels.len() > 20, "width {width}: {} levels", levels.len());
        }
    }

    fn params(dpr: f64, w: f64) -> LineParams {
        LineParams {
            horizontal_pixel_ratio: dpr,
            vertical_pixel_ratio: dpr,
            line_width: w,
            line_type: LineType::Simple,
        }
    }

    #[test]
    fn expand_simple_is_identity() {
        let pts = [LinePoint { x: 0.0, y: 1.0 }, LinePoint { x: 1.0, y: 2.0 }];
        let out = expand_line(&pts, LineType::Simple);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].x, 1.0);
        assert_eq!(out[1].y, 2.0);
    }

    #[test]
    fn expand_steps_inserts_corner_at_previous_y() {
        // two points -> point, step-corner, point = 3 vertices; corner at (x1, y0)
        let pts = [LinePoint { x: 0.0, y: 10.0 }, LinePoint { x: 5.0, y: 20.0 }];
        let out = expand_line(&pts, LineType::WithSteps);
        assert_eq!(out.len(), 3);
        assert_eq!((out[1].x, out[1].y), (5.0, 10.0)); // horizontal then vertical
        assert_eq!((out[2].x, out[2].y), (5.0, 20.0));
    }

    #[test]
    fn expand_curved_densifies_and_passes_through_points() {
        let pts = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 10.0, y: 10.0 },
            LinePoint { x: 20.0, y: 0.0 },
        ];
        let out = expand_line(&pts, LineType::Curved);
        // adaptive: each 14.1px interval gets ceil(14.1 / 4) = 4 segments
        assert_eq!(out.len(), 2 * 4 + 1);
        // curve interpolates through the source knots
        assert_eq!((out[0].x, out[0].y), (0.0, 0.0));
        assert_eq!((out[4].x, out[4].y), (10.0, 10.0));
        assert_eq!((out[8].x, out[8].y), (20.0, 0.0));
    }

    #[test]
    fn expand_curved_keeps_short_intervals_as_chords() {
        // Brush-density samples: 1.5px intervals are already below the visible faceting
        // threshold, so each renders as its chord instead of 16 curve segments.
        let pts: Vec<LinePoint> = (0..20)
            .map(|i| LinePoint {
                x: i as f64 * 1.5,
                y: (i as f64 * 0.7).sin(),
            })
            .collect();
        let out = expand_line(&pts, LineType::Curved);
        assert_eq!(out.len(), pts.len(), "one segment per short interval");
    }

    #[test]
    fn expand_curved_caps_long_intervals_at_sixteen_segments() {
        let pts = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 500.0, y: 100.0 },
            LinePoint { x: 1000.0, y: 0.0 },
        ];
        let out = expand_line(&pts, LineType::Curved);
        assert_eq!(out.len(), 2 * CURVE_SEGMENTS + 1);
    }

    #[test]
    fn expand_curved_scales_segment_count_by_pixel_ratio() {
        let pts = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 10.0, y: 10.0 },
            LinePoint { x: 20.0, y: 0.0 },
        ];
        let mut out = Vec::new();
        expand_line_into(&pts, LineType::Curved, 2.0, 2.0, &mut out);
        // 14.1 media px = 28.3 device px per interval -> ceil(28.3 / 4) = 8 segments
        assert_eq!(out.len(), 2 * 8 + 1);
    }

    #[test]
    fn band_expansion_keeps_step_and_curve_boundaries_pairwise_aligned() {
        let upper = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 10.0, y: 20.0 },
            LinePoint { x: 20.0, y: 0.0 },
        ];
        let lower = [
            LinePoint { x: 0.0, y: 30.0 },
            LinePoint { x: 10.0, y: 35.0 },
            LinePoint { x: 20.0, y: 25.0 },
        ];
        for line_type in [LineType::WithSteps, LineType::Curved] {
            let (expanded_upper, expanded_lower) = expand_band(&upper, &lower, line_type);
            assert_eq!(expanded_upper.len(), expanded_lower.len());
            assert!(expanded_upper.len() > upper.len());
            assert!(expanded_upper
                .iter()
                .zip(&expanded_lower)
                .all(|(upper, lower)| (upper.x - lower.x).abs() < f64::EPSILON));
            assert_eq!(expanded_upper.first().unwrap().x, 0.0);
            assert_eq!(expanded_upper.last().unwrap().x, 20.0);
        }
    }

    #[test]
    fn nearly_collinear_strokes_skip_round_joins() {
        // A dense, almost straight brush stroke: no turn opens a visible wedge, so the mesh is
        // exactly two triangles per segment with no join fans.
        let pts: Vec<LinePoint> = (0..50)
            .map(|i| LinePoint {
                x: i as f64 * 1.5,
                y: 10.0 + (i as f64 * 0.02).sin() * 0.05,
            })
            .collect();
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&pts, BLUE, &params(1.0, 6.0), &mut mesh);
        assert_eq!(mesh.vertices.len(), 49 * 6, "segments only, no joins");
    }

    #[test]
    fn baseline_splits_at_crossing() {
        // a below-baseline point to an above-baseline point crosses baseline_y=10 once.
        // Same-side pair => 1 sub-segment (2 tris fill); crossing pair => 2 sub-segments.
        let crossing = [LinePoint { x: 0.0, y: 20.0 }, LinePoint { x: 10.0, y: 0.0 }];
        let same = [LinePoint { x: 0.0, y: 5.0 }, LinePoint { x: 10.0, y: 2.0 }];
        let (tl, bl) = (Color::rgb(0, 200, 0), Color::rgb(200, 0, 0));
        let (tf, bf) = (Color::rgba(0, 200, 0, 40), Color::rgba(200, 0, 0, 40));

        let mut s1 = StrokeMesh::default();
        let mut f1 = AreaMesh::default();
        build_baseline(
            &crossing,
            10.0,
            tl,
            bl,
            tf,
            bf,
            &params(1.0, 2.0),
            &mut s1,
            &mut f1,
        );
        // two sub-segments => 2 stroke quads (12 verts) and 2 fill quads (12 verts)
        assert_eq!(s1.vertices.len(), 12);
        assert_eq!(f1.vertices.len(), 12);

        let mut s2 = StrokeMesh::default();
        let mut f2 = AreaMesh::default();
        build_baseline(
            &same,
            10.0,
            tl,
            bl,
            tf,
            bf,
            &params(1.0, 2.0),
            &mut s2,
            &mut f2,
        );
        // one sub-segment => 1 quad each
        assert_eq!(s2.vertices.len(), 6);
        assert_eq!(f2.vertices.len(), 6);
    }

    #[test]
    fn stroke_emits_two_triangles_per_segment() {
        let pts = [
            LinePoint { x: 0.0, y: 10.0 },
            LinePoint { x: 10.0, y: 10.0 },
        ];
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&pts, BLUE, &params(1.0, 2.0), &mut mesh);
        // one segment, no interior joins -> 6 vertices (2 tris)
        assert_eq!(mesh.vertices.len(), 6);
    }

    #[test]
    fn horizontal_segment_extrudes_vertically() {
        let pts = [
            LinePoint { x: 0.0, y: 10.0 },
            LinePoint { x: 10.0, y: 10.0 },
        ];
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&pts, BLUE, &params(1.0, 4.0), &mut mesh);
        // half width 2; the extruded quad should span y in [8, 12]
        let ys: Vec<f32> = mesh.vertices.iter().map(|v| v.y).collect();
        assert!(ys.iter().cloned().fold(f32::INFINITY, f32::min) == 8.0);
        assert!(ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max) == 12.0);
    }

    #[test]
    fn interior_vertices_get_round_joins() {
        let pts = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 10.0, y: 10.0 },
            LinePoint { x: 20.0, y: 0.0 },
        ];
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&pts, BLUE, &params(1.0, 3.0), &mut mesh);
        // 2 segments (2*6=12) + 1 round join (8 tris = 24 verts) = 36
        assert_eq!(mesh.vertices.len(), 12 + 24);
    }

    #[test]
    fn dpr_scales_positions() {
        let pts = [LinePoint { x: 5.0, y: 5.0 }, LinePoint { x: 15.0, y: 5.0 }];
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&pts, BLUE, &params(2.0, 2.0), &mut mesh);
        // x coords should be scaled by dpr: 10 and 30
        let xs: Vec<f32> = mesh.vertices.iter().map(|v| v.x).collect();
        assert!(xs.contains(&10.0));
        assert!(xs.contains(&30.0));
    }

    #[test]
    fn area_fill_reaches_base() {
        let pts = [
            LinePoint { x: 0.0, y: 10.0 },
            LinePoint { x: 10.0, y: 20.0 },
        ];
        let mut mesh = AreaMesh::default();
        build_area_fill(
            &pts,
            100.0,
            BLUE,
            Color::rgba(0x21, 0x96, 0xf3, 0),
            &params(1.0, 2.0),
            &mut mesh,
        );
        // 6 verts per segment
        assert_eq!(mesh.vertices.len(), 6);
        // some vertex sits at base y = 100
        assert!(mesh.vertices.iter().any(|v| v.y == 100.0));
        // top color opaque, base color transparent (gradient endpoints)
        let at_line = mesh.vertices.iter().find(|v| v.y == 10.0).unwrap();
        assert!(at_line.color[3] > 0.9);
        let at_base = mesh.vertices.iter().find(|v| v.y == 100.0).unwrap();
        assert!(at_base.color[3] < 0.1);
    }

    #[test]
    fn empty_and_single_point_no_geometry() {
        let mut mesh = StrokeMesh::default();
        build_line_stroke(&[], BLUE, &params(1.0, 2.0), &mut mesh);
        build_line_stroke(
            &[LinePoint { x: 0.0, y: 0.0 }],
            BLUE,
            &params(1.0, 2.0),
            &mut mesh,
        );
        assert!(mesh.vertices.is_empty());
    }

    #[test]
    fn disc_is_a_fan_around_center() {
        let mut v = Vec::new();
        build_disc([10.0, 20.0], 4.0, BLUE, &mut v);
        assert_eq!(v.len(), 24 * 3); // 24 fan triangles
                                     // every triangle's first vertex is the center
        for tri in v.chunks(3) {
            assert_eq!([tri[0].x, tri[0].y], [10.0, 20.0]);
        }
        // rim vertices lie ~radius from center
        let rim = &v[1];
        let d = ((rim.x - 10.0).powi(2) + (rim.y - 20.0).powi(2)).sqrt();
        assert!((d - 4.0).abs() < 1e-4);
    }

    #[test]
    fn dash_split_cuts_exact_on_segments() {
        // [2 on, 2 off] over a 10px horizontal line: on-runs [0,2], [4,6], [8,10].
        let pts = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 10.0, y: 0.0 }];
        let runs = dash_split(&pts, &[2.0, 2.0]);
        assert_eq!(runs.len(), 3);
        let spans: Vec<(f64, f64)> = runs
            .iter()
            .map(|run| (run.first().unwrap().x, run.last().unwrap().x))
            .collect();
        assert_eq!(spans, vec![(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)]);
        // every run is a proper sub-polyline
        assert!(runs.iter().all(|run| run.len() >= 2));
    }

    #[test]
    fn dash_split_continues_the_pattern_across_vertices() {
        // L-shaped path: 5px right then 5px down. Pattern [4 on, 4 off]: the first dash
        // covers (0,0)->(4,0); the 4px gap wraps the corner (1px horizontal + 3px vertical),
        // so the second run resumes at (5,3) and continues to (5,5) — the pattern never
        // restarts at a vertex (reference setLineDash semantics).
        let pts = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 5.0, y: 0.0 },
            LinePoint { x: 5.0, y: 5.0 },
        ];
        let runs = dash_split(&pts, &[4.0, 4.0]);
        assert_eq!(runs.len(), 2);
        let r0 = &runs[0];
        assert_eq!(r0.first().unwrap().x, 0.0);
        assert_eq!(r0.first().unwrap().y, 0.0);
        assert!((r0.last().unwrap().x - 4.0).abs() < 1e-9);
        assert_eq!(r0.last().unwrap().y, 0.0);
        let r1 = &runs[1];
        assert!((r1.first().unwrap().x - 5.0).abs() < 1e-9);
        assert!((r1.first().unwrap().y - 3.0).abs() < 1e-9);
        assert!((r1.last().unwrap().y - 5.0).abs() < 1e-9);
    }

    #[test]
    fn dash_split_ends_mid_dash_without_trailing_run() {
        // Path ends inside an "off" element: only the first dash is drawn.
        let pts = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 3.0, y: 0.0 }];
        let runs = dash_split(&pts, &[2.0, 2.0]);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].first().unwrap().x, 0.0);
        assert_eq!(runs[0].last().unwrap().x, 2.0);
    }

    #[test]
    fn dash_split_rejects_degenerate_input() {
        let pts = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 3.0, y: 0.0 }];
        assert!(dash_split(&pts, &[]).is_empty());
        assert!(dash_split(&pts, &[2.0, 0.0]).is_empty());
        assert!(dash_split(&pts[..1], &[2.0, 2.0]).is_empty());
        assert!(dash_split(&pts, &[f64::NAN, 2.0]).is_empty());
        assert!(dash_split(&pts, &[f64::INFINITY, 2.0]).is_empty());
    }

    #[test]
    fn dash_split_work_is_bounded_for_hostile_patterns_and_lengths() {
        let solid = |pts: &[LinePoint], pattern: &[f64]| {
            let runs = dash_split(pts, pattern);
            assert_eq!(
                runs.len(),
                1,
                "pattern {pattern:?} must fall back to one solid run"
            );
            assert_eq!(runs[0].len(), pts.len());
        };
        let short = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 100.0, y: 0.0 }];
        // A plugin polyline width of 1e-12 yields this dot pattern.
        solid(&short, &[1e-12, 4e-12]);
        let huge = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 1e30, y: 0.0 }];
        solid(&huge, &[1.0, 4.0]);
        // One element too small to advance f64 at this offset must not stall the walk.
        let long = [LinePoint { x: 0.0, y: 0.0 }, LinePoint { x: 1e7, y: 0.0 }];
        assert!(dash_split(&long, &[1e6, 1e-12]).len() <= MAX_DASH_STEPS);
        let infinite = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint {
                x: f64::INFINITY,
                y: 0.0,
            },
        ];
        assert!(dash_split(&infinite, &[1.0, 4.0]).is_empty());
        let nan = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint {
                x: f64::NAN,
                y: 0.0,
            },
        ];
        assert!(dash_split(&nan, &[1.0, 4.0]).is_empty());
        // The ceiling is generous for real charts: a 4k-wide dotted line still dashes.
        let wide = [
            LinePoint { x: 0.0, y: 0.0 },
            LinePoint { x: 8192.0, y: 0.0 },
        ];
        assert_eq!(dash_split(&wide, &[1.0, 1.0]).len(), 4096);
    }

    /// Each polyline prim's point window in the shared pool, asserting the lowering contract
    /// (executors receive only solid, simple runs from a dashed producer).
    fn solid_runs(prims: &[Prim], pool: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
        prims
            .iter()
            .map(|prim| {
                let Prim::Polyline {
                    first_point,
                    point_count,
                    style,
                    line_type,
                    ..
                } = *prim
                else {
                    panic!("unexpected prim {prim:?}");
                };
                assert_eq!(style, LineStyle::Solid, "dashed runs reach executors solid");
                assert_eq!(line_type, LineType::Simple, "dashed runs are pre-expanded");
                pool[first_point as usize..(first_point + point_count) as usize].to_vec()
            })
            .collect()
    }

    const PANE: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 200.0,
        bottom: 100.0,
    };

    #[test]
    fn solid_styled_stroke_is_one_polyline_in_the_requested_line_type() {
        let run = [(0.0, 0.0), (50.0, 40.0), (100.0, 10.0)];
        for line_type in [LineType::Simple, LineType::WithSteps, LineType::Curved] {
            let (mut prims, mut pool) = (Vec::new(), vec![[9.0, 9.0]]);
            push_styled_stroke(
                &mut prims,
                &mut pool,
                &run,
                line_type,
                (2.0, LineStyle::Solid, BLUE),
                PANE,
            );
            assert_eq!(
                prims,
                vec![Prim::Polyline {
                    first_point: 1,
                    point_count: 3,
                    width: 2.0,
                    style: LineStyle::Solid,
                    line_type,
                    color: BLUE,
                }]
            );
            assert_eq!(pool[1..], [[0.0, 0.0], [50.0, 40.0], [100.0, 10.0]]);
        }
    }

    #[test]
    fn dashed_line_stroke_emits_only_solid_dash_runs() {
        let window = [[0.0, 50.0], [200.0, 50.0]];
        let (mut prims, mut pool) = (Vec::new(), Vec::new());
        push_line_stroke(
            &mut prims,
            &mut pool,
            &window,
            2.0,
            LineStyle::Dashed,
            LineType::Simple,
            BLUE,
        );
        let spans: Vec<(f32, f32)> = solid_runs(&prims, &pool)
            .iter()
            .map(|run| (run[0][0], run[run.len() - 1][0]))
            .collect();
        // Dashed at width 2 is [12, 12]: dashes start every 24 px and the last is cut at 200.
        let expected: Vec<(f32, f32)> = (0..9)
            .map(|k| (k as f32 * 24.0, (k as f32 * 24.0 + 12.0).min(200.0)))
            .collect();
        assert_eq!(spans, expected);
    }

    #[test]
    fn clipped_dashed_stroke_keeps_the_unclipped_phase_and_stays_bounded() {
        // Dash starts sit at -1000 + 24k, so inside the pane they must land on x = 8 + 24k.
        let run = [(-1000.0, 50.0), (1.0e9, 50.0)];
        let (mut prims, mut pool) = (Vec::new(), Vec::new());
        push_styled_stroke(
            &mut prims,
            &mut pool,
            &run,
            LineType::Simple,
            (2.0, LineStyle::Dashed, BLUE),
            PANE,
        );
        let runs = solid_runs(&prims, &pool);
        assert!(
            runs.len() <= 12,
            "work is bounded by the pane: {}",
            runs.len()
        );
        assert!(pool.len() <= 32);
        let clip = PANE.inflate(2.0 + 2.0);
        assert!(pool
            .iter()
            .all(|p| f64::from(p[0]) >= clip.left - 24.0 && f64::from(p[0]) <= clip.right + 24.0));
        let inside: Vec<f32> = runs
            .iter()
            .map(|run| run[0][0])
            .filter(|&x| x >= 0.0)
            .collect();
        assert_eq!(inside.first(), Some(&8.0));
        assert!(inside.iter().all(|x| (x - 8.0).rem_euclid(24.0) == 0.0));
    }

    #[test]
    fn dash_run_bound_covers_the_lowered_run_count_of_any_path() {
        // Straight, dense zigzag, re-entering spiral, and mostly-outside paths, at every dashed
        // style and several widths: the bound never undercounts what `push_styled_stroke` emits
        // and stays within a small factor of it (it only over-counts partial dashes per part).
        let zigzag: Vec<(f64, f64)> = (0..500)
            .map(|i| (f64::from(i) * 0.3, if i % 2 == 0 { 0.0 } else { 100.0 }))
            .collect();
        let spiral: Vec<(f64, f64)> = (0..800)
            .map(|i| {
                let t = f64::from(i) * 0.05;
                (100.0 + 700.0 * t.cos(), 50.0 + 400.0 * t.sin())
            })
            .collect();
        let paths: [(&str, Vec<(f64, f64)>); 5] = [
            ("straight", vec![(0.0, 50.0), (200.0, 50.0)]),
            ("far", vec![(-1000.0, 50.0), (1.0e9, 50.0)]),
            ("zigzag", zigzag),
            ("spiral", spiral),
            ("outside", vec![(0.0, 500.0), (300.0, 500.0)]),
        ];
        for (name, run) in &paths {
            for style in [LineStyle::Dotted, LineStyle::Dashed] {
                for width in [0.5_f32, 1.0, 2.0, 5.0] {
                    let (mut prims, mut pool) = (Vec::new(), Vec::new());
                    push_styled_stroke(
                        &mut prims,
                        &mut pool,
                        run,
                        LineType::Simple,
                        (width, style, BLUE),
                        PANE,
                    );
                    let bound = dash_run_bound(run, PANE, width, style);
                    let runs = prims.len() as f64;
                    assert!(
                        bound >= runs,
                        "{name} {style:?} width {width}: {runs} runs over bound {bound}"
                    );
                    assert!(
                        bound <= 2.0 * runs + 4.0 * (run.len() as f64),
                        "{name} {style:?} width {width}: bound {bound} is loose for {runs} runs"
                    );
                }
            }
        }
        // A path that misses the pane costs nothing; a long in-pane path costs its length.
        assert_eq!(
            dash_run_bound(&paths[4].1, PANE, 2.0, LineStyle::Dashed),
            0.0
        );
        assert!(dash_run_bound(&paths[2].1, PANE, 1.0, LineStyle::Dotted) > 5_000.0);
    }

    #[test]
    fn dash_run_bound_is_zero_when_nothing_is_split() {
        let run = [(0.0, 50.0), (200.0, 50.0)];
        assert_eq!(dash_run_bound(&run, PANE, 2.0, LineStyle::Solid), 0.0);
        // A non-positive width has a non-positive pattern, which `dash_split` refuses to walk.
        assert_eq!(dash_run_bound(&run, PANE, 0.0, LineStyle::Dashed), 0.0);
        assert_eq!(dash_run_bound(&run, PANE, -3.0, LineStyle::Dotted), 0.0);
        assert_eq!(dash_run_bound(&run[..1], PANE, 2.0, LineStyle::Dashed), 0.0);
    }

    #[test]
    fn styled_dashes_expand_curved_and_stepped_lines_before_clipping() {
        let run = [(0.0, 80.0), (60.0, 10.0), (120.0, 70.0), (180.0, 20.0)];
        let style = (2.0, LineStyle::Dotted, BLUE);
        for line_type in [LineType::Curved, LineType::WithSteps] {
            let expanded: Vec<(f64, f64)> =
                expand_line(&run.map(|(x, y)| LinePoint { x, y }), line_type)
                    .into_iter()
                    .map(|point| (point.x, point.y))
                    .collect();
            assert!(expanded.len() > run.len(), "{line_type:?} expands");
            let (mut lowered, mut lowered_pool) = (Vec::new(), Vec::new());
            push_styled_stroke(
                &mut lowered,
                &mut lowered_pool,
                &run,
                line_type,
                style,
                PANE,
            );
            let (mut reference, mut reference_pool) = (Vec::new(), Vec::new());
            push_styled_stroke(
                &mut reference,
                &mut reference_pool,
                &expanded,
                LineType::Simple,
                style,
                PANE,
            );
            assert!(!lowered.is_empty());
            assert_eq!(lowered, reference, "{line_type:?}");
            assert_eq!(lowered_pool, reference_pool, "{line_type:?}");
            solid_runs(&lowered, &lowered_pool);
        }
    }

    #[test]
    fn crisp_span_clamps_to_the_pane_and_keeps_the_dash_phase() {
        let bounds = (0.0, 400.0);
        // Misses the pane on either side, in either order.
        assert_eq!(crisp_span(-50.0, -1.0, bounds, 2, LineStyle::Dashed), None);
        assert_eq!(crisp_span(500.0, 401.0, bounds, 2, LineStyle::Solid), None);
        // Inside: unchanged, either order.
        assert_eq!(
            crisp_span(400.0, 10.4, bounds, 2, LineStyle::Solid),
            Some((10, 400))
        );
        // A solid start clamps to the pane edge exactly.
        assert_eq!(
            crisp_span(-1.0e9, 1.0e9, bounds, 2, LineStyle::Solid),
            Some((0, 400))
        );
        // A dashed start moves back a whole period (24 at width 2) from the unclamped start.
        let (start, end) = crisp_span(-1000.0, 1.0e9, bounds, 2, LineStyle::Dashed).unwrap();
        assert_eq!(end, 400);
        assert!((-24..=0).contains(&start), "{start}");
        assert_eq!((start + 1000).rem_euclid(24), 0);
        let (start, _) = crisp_span(-1.0e9, 1.0e9, bounds, 2, LineStyle::Dotted).unwrap();
        assert!((-10..=0).contains(&start), "{start}");
        assert_eq!((start + 1_000_000_000).rem_euclid(10), 0);
    }

    #[test]
    fn area_fill_shades_inverted_fill_from_base_to_line() {
        // Inverted fill (base above the points): the top color sits at the base edge and the
        // bottom color at the lowest line point — the reverse of the normal direction.
        let pts = [
            LinePoint { x: 0.0, y: 20.0 },
            LinePoint { x: 10.0, y: 30.0 },
        ];
        let mut mesh = AreaMesh::default();
        build_area_fill(
            &pts,
            5.0,
            BLUE,
            Color::rgba(0x21, 0x96, 0xf3, 0),
            &params(1.0, 2.0),
            &mut mesh,
        );
        assert_eq!(mesh.vertices.len(), 6);
        let at_base = mesh.vertices.iter().find(|v| v.y == 5.0).unwrap();
        assert!(at_base.color[3] > 0.9, "base edge keeps the top color");
        let at_lowest = mesh.vertices.iter().find(|v| v.y == 30.0).unwrap();
        assert!(
            at_lowest.color[3] < 0.1,
            "lowest line point fades to the bottom color"
        );
    }
}
